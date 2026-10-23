//! `omni import penpot <file.penpot> --project <id>`: imports a Penpot export
//! through a new change set (US-051). The export is read first, with the reader
//! and converter of the `omni-penpot` crate, so a file that is no export fails
//! before anything is sent. Then the change set gets
//!
//! - the images of the export, uploaded as workspace attachments, and the font
//!   files beside it (`fonts/<family>/`) of the families its texts use, which
//!   the typography tokens list as their font files (US-057),
//! - the design system doc: the design tokens (Penpot's themes are the modes)
//!   and the components, started in the change set unless the project has one.
//!   When it has one with a mainline version, the import adds to it: its
//!   tokens and components stay, and a token of the same path takes the
//!   export's value,
//! - the design doc: the pages and boards, with the copies of components as
//!   instances,
//!
//! and a saved version of each doc. The change set is left `open`: the agent
//! submits it, and the human approves it and merges it like any other change.
//! Whatever cannot be imported is in the report, which `--json` prints whole.

use std::path::Path;

use omni_penpot::ReadError;
use omni_penpot::import::{Ids, Import, ImportEntry, import_file};

use crate::args::Args;
use crate::attach;
use crate::changesets::{changesets_path, project_id};
use crate::commands::{Agent, agent, checked, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

/// How many times the components are sent: the first time without the
/// instances in them, which wait for the IDs of the components they place.
const MAX_PASSES: usize = 8;

/// What the report says when the project's design system cannot take the
/// import.
const WAITING: &str = "the project's design system doc has no mainline version yet, because the change set that started it is not merged, so this change set cannot hold the tokens, colors, typographies and components; they are not imported, the copies of components are plain frames and the token links plain colors. Merge that change set and import again to get them";

/// How many entries of the report the short output lists.
const SHOWN: usize = 10;

fn read_error(error: ReadError) -> CliError {
    CliError::new(error.to_string())
}

fn parsed(text: &str, what: &str) -> Result<Value> {
    Value::parse(text).map_err(|_| {
        CliError::new(format!(
            "the server's answer about {what} is not readable; is it an OmniState API?"
        ))
    })
}

fn id_of(answer: &Value, key: &str, what: &str) -> Result<String> {
    let found = answer.get(key).unwrap_or(answer);
    found.str_at("id").map(str::to_string).ok_or_else(|| {
        CliError::new(format!(
            "the server's answer about {what} has no ID; is it an OmniState API?"
        ))
    })
}

fn body_of(field: &str, value: &str) -> String {
    format!("{{{}:{value}}}", quote(field))
}

/// `omni import penpot <file.penpot> --project <id>`
pub fn penpot(args: &Args, file: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni import penpot <file.penpot> --project <id>";
    let Some(file) = file else {
        bail!("this command needs the Penpot export to import, a .penpot file: {USAGE}");
    };
    let mut import = import_file(Path::new(file)).map_err(read_error)?;
    for attachment in &import.attachments {
        if attachment.bytes.len() > attach::MAX_UPLOAD {
            bail!(
                "the image or font file {} of {file} is larger than 20 MB, the limit for an attachment; nothing was imported",
                attachment.id
            );
        }
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let title = Path::new(file).file_name().map_or_else(
        || file.to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut fields = vec![("title", format!("Import of {title}"))];
    fields.extend(args.run.clone().map(|tag| ("run_tag", tag)));
    let body = format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(name, value)| format!("{}:{}", quote(name), quote(value)))
            .collect::<Vec<_>>()
            .join(",")
    );
    let created = agent.request(
        "POST",
        &changesets_path(&agent, &project),
        Some(body.as_bytes()),
    )?;
    let changeset = id_of(
        &parsed(&created.text(), "the new change set")?,
        "changeset",
        "the new change set",
    )?;
    let base = format!("/api/v1/w/{}/projects/{project}", agent.workspace_id);

    let mut ids = Ids::default();
    for attachment in &import.attachments {
        let name = format!("{}.{}", attachment.id, attachment.extension);
        let response = attach::send(&agent, &name, &attachment.bytes)?;
        let uploaded = id_of(
            &parsed(&response.text(), "the attachment")?,
            "attachment",
            "the attachment",
        )?;
        ids.attachment(&attachment.id, &uploaded);
    }

    let components = import.component_count();
    let mut design_system = None;
    if import.has_tokens() || components > 0 {
        match send_design_system(&agent, &base, &changeset, &mut import, &mut ids)? {
            Sent::Doc(doc) => design_system = Some(doc),
            Sent::Waiting => {
                import = import.without_design_system(WAITING).map_err(read_error)?;
            }
        }
    }
    let design = if import.pages.is_empty() {
        None
    } else {
        Some(send_design(
            &agent, &base, &changeset, &import, &ids, &title,
        )?)
    };

    // The design system first, so that the design records its saved version.
    let mut versions = Vec::new();
    for doc in design_system.iter().chain(design.iter()) {
        let response = agent.request(
            "POST",
            &format!("{base}/changesets/{changeset}/docs/{doc}/versions"),
            Some(b"{}"),
        )?;
        versions.push(id_of(
            &parsed(&response.text(), "the saved version")?,
            "version",
            "the saved version",
        )?);
    }
    let mut versions = versions.into_iter();
    let design_system_version = design_system.as_ref().and_then(|_| versions.next());
    let design_version = design.as_ref().and_then(|_| versions.next());

    if args.json {
        say(&json_output(
            &changeset,
            design.as_deref(),
            design_system.as_deref(),
            design_version.as_deref(),
            design_system_version.as_deref(),
            &import.report,
        ));
        return Ok(());
    }
    say(&format!("imported {title} into change set {changeset}"));
    let boards: usize = import.pages.iter().map(|page| page.children().len()).sum();
    say(&format!(
        "{} pages, {boards} boards, {} components, {} attachments",
        import.pages.len(),
        components,
        import.attachments.len()
    ));
    if let Some(doc) = &design {
        say(&format!("design doc: {doc}"));
    }
    if let Some(doc) = &design_system {
        say(&format!("design system doc: {doc}"));
    }
    if import.report.is_empty() {
        say("everything was imported");
    } else {
        say(&format!(
            "{} things could not be imported (all of them with --json):",
            import.report.len()
        ));
        for entry in import.report.iter().take(SHOWN) {
            say(&format!("  {}", entry_line(entry)));
        }
        if import.report.len() > SHOWN {
            say(&format!("  and {} more", import.report.len() - SHOWN));
        }
    }
    say("the change set is open: `omni changeset submit` sends it to review");
    Ok(())
}

/// What came of the design system.
enum Sent {
    /// The design system doc, with the import in its draft.
    Doc(String),
    /// The project's design system doc has no mainline version, so only the
    /// change set that started it has a draft of it.
    Waiting,
}

/// The design system doc: the project's, or one started in the change set,
/// with the tokens and the components of the import in its draft. When the
/// project's design system has a mainline version, `import` becomes the
/// import onto it, so that the draft keeps what that version holds.
fn send_design_system(
    agent: &Agent,
    base: &str,
    changeset: &str,
    import: &mut Import,
    ids: &mut Ids,
) -> Result<Sent> {
    let known = agent.request(
        "GET",
        &format!("{base}/changesets/{changeset}/tokens"),
        None,
    )?;
    let doc = match parsed(&known.text(), "the tokens")?.str_at("doc_id") {
        Some(doc) => doc.to_string(),
        None => {
            let body = format!(
                "{{\"kind\":\"design-system\",\"pillar\":\"ui\",\"title\":{}}}",
                quote("Design system")
            );
            let response = agent.request(
                "POST",
                &format!("{base}/changesets/{changeset}/docs"),
                Some(body.as_bytes()),
            )?;
            id_of(
                &parsed(&response.text(), "the new doc")?,
                "doc",
                "the new doc",
            )?
        }
    };
    let mainline = agent.request("GET", &format!("{base}/tokens"), None)?;
    let mainline = mainline.text();
    if let Some(version) = parsed(&mainline, "the tokens")?.str_at("version_id") {
        let read = agent.request("GET", &format!("{base}/versions/{version}"), None)?;
        *import = import
            .onto_design_system(&mainline, &read.text(), ids)
            .map_err(read_error)?;
    }
    let tokens = agent.request_any(
        "PUT",
        &format!("{base}/changesets/{changeset}/tokens"),
        Some(body_of("tokens", &import.token_tree_json(ids)).as_bytes()),
    )?;
    let refused = parsed(&tokens.text(), "the tokens");
    if tokens.status == 409
        && refused
            .ok()
            .is_some_and(|answer| answer.str_at("error") == Some("no_mainline_version"))
    {
        return Ok(Sent::Waiting);
    }
    checked(tokens)?;
    if import.component_count() == 0 {
        return Ok(Sent::Doc(doc));
    }
    for _ in 0..MAX_PASSES {
        let rendered = import.component_tags(ids).map_err(read_error)?;
        let response = agent.request(
            "PUT",
            &format!("{base}/changesets/{changeset}/components"),
            Some(body_of("tags", &quote(&rendered.tags)).as_bytes()),
        )?;
        import
            .learn_components(ids, &rendered, &response.text())
            .map_err(read_error)?;
        if rendered.complete {
            return Ok(Sent::Doc(doc));
        }
    }
    bail!(
        "the components nest too deep to be sent in {MAX_PASSES} steps; the tokens are in the change set, but its components are not complete"
    )
}

/// Starts the design doc in the change set and puts the pages into its draft.
/// Returns the doc's ID.
fn send_design(
    agent: &Agent,
    base: &str,
    changeset: &str,
    import: &Import,
    ids: &Ids,
    title: &str,
) -> Result<String> {
    let body = format!(
        "{{\"kind\":\"design\",\"pillar\":\"ui\",\"title\":{}}}",
        quote(title)
    );
    let response = agent.request(
        "POST",
        &format!("{base}/changesets/{changeset}/docs"),
        Some(body.as_bytes()),
    )?;
    let doc = id_of(
        &parsed(&response.text(), "the new doc")?,
        "doc",
        "the new doc",
    )?;
    let tags = import.design_tags(ids).map_err(read_error)?;
    agent.request(
        "PUT",
        &format!("{base}/changesets/{changeset}/docs/{doc}/design"),
        Some(body_of("tags", &quote(&tags)).as_bytes()),
    )?;
    Ok(doc)
}

/// One line about an entry of the report.
fn entry_line(entry: &ImportEntry) -> String {
    let place = [
        entry.page.as_str(),
        entry.board.as_str(),
        entry.name.as_str(),
    ]
    .iter()
    .filter(|part| !part.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" / ");
    let property = entry
        .property
        .as_deref()
        .map_or(String::new(), |property| format!(" ({property})"));
    format!("{place}{property}: {}", entry.reason)
}

fn optional(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_string(), quote)
}

fn json_output(
    changeset: &str,
    design: Option<&str>,
    design_system: Option<&str>,
    design_version: Option<&str>,
    design_system_version: Option<&str>,
    report: &[ImportEntry],
) -> String {
    let entries: Vec<String> = report
        .iter()
        .map(|entry| {
            format!(
                "{{\"page\":{},\"board\":{},\"board_id\":{},\"shape_id\":{},\"name\":{},\"property\":{},\"reason\":{}}}",
                quote(&entry.page),
                quote(&entry.board),
                quote(&entry.board_id),
                quote(&entry.shape_id),
                quote(&entry.name),
                optional(entry.property.as_deref()),
                quote(&entry.reason)
            )
        })
        .collect();
    format!(
        "{{\"changeset_id\":{},\"design_doc_id\":{},\"design_system_doc_id\":{},\"design_version_id\":{},\"design_system_version_id\":{},\"report\":[{}]}}",
        quote(changeset),
        optional(design),
        optional(design_system),
        optional(design_version),
        optional(design_system_version),
        entries.join(",")
    )
}
