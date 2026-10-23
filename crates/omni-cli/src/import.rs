//! `omni import penpot <file.penpot> --project <id> [--feature <name>]`:
//! imports a Penpot export straight into the project (US-105), with no change
//! set, as an agent's own edits are written. The export is read first, with the
//! reader and converter of the `omni-penpot` crate, so a file that is no export
//! fails before anything is sent. Then the project gets
//!
//! - the images of the export, uploaded as workspace attachments, and the font
//!   files beside it (`fonts/<family>/`) of the families its texts use, which
//!   the typography tokens list as their font files (US-057),
//! - the design system doc, at the app level: the design tokens (Penpot's
//!   themes are the modes) and the components, written to its head. The doc is
//!   started unless the project has one; when it has one, the import adds to
//!   it: its tokens and components stay with their IDs, and a token of the same
//!   path takes the export's value,
//! - the design doc: the pages and boards, with the copies of components as
//!   instances, in the UI pillar of the feature when there is `--feature`, else
//!   at the app level.
//!
//! Whatever cannot be imported is in the report, which `--json` prints whole.

use std::path::Path;

use omni_penpot::ReadError;
use omni_penpot::import::{Ids, Import, ImportEntry, PageTags, import_file};

use crate::args::Args;
use crate::attach;
use crate::changesets::{creation_feature, project_id};
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

/// How many times the components are sent: the first time without the
/// instances in them, which wait for the IDs of the components they place.
const MAX_PASSES: usize = 8;

/// How many bytes of tags one batch of boards holds at most. A request body is
/// limited, and every batch makes the API read and write the design's head.
const BATCH_BYTES: usize = 1_000_000;

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

/// `omni import penpot <file.penpot> --project <id> [--feature <name>]`
pub fn penpot(args: &Args, file: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni import penpot <file.penpot> --project <id> [--feature <name>]";
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
    let feature = creation_feature(args, &agent, &project)?;
    let title = Path::new(file).file_name().map_or_else(
        || file.to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
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
        design_system = Some(send_design_system(&agent, &base, &mut import, &mut ids)?);
    }
    let design = if import.pages.is_empty() {
        None
    } else {
        Some(send_design(
            &agent,
            &base,
            feature.as_deref(),
            &import,
            &ids,
            &title,
        )?)
    };

    if args.json {
        say(&json_output(
            design.as_deref(),
            design_system.as_deref(),
            &import.report,
        ));
        return Ok(());
    }
    say(&format!("imported {title}"));
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
    Ok(())
}

/// The project's design system doc, started when it has none, with the tokens
/// and the components of the import written to its head. When the project's
/// design system already holds tokens or components, `import` becomes the
/// import onto it, so that the head keeps what it holds. Returns the doc's ID.
fn send_design_system(
    agent: &Agent,
    base: &str,
    import: &mut Import,
    ids: &mut Ids,
) -> Result<String> {
    let known = agent.request("GET", &format!("{base}/tokens"), None)?;
    let known = known.text();
    let doc = match parsed(&known, "the tokens")?.str_at("doc_id") {
        Some(doc) => {
            let head = agent.request("GET", &format!("{base}/docs/{doc}/html"), None)?;
            *import = import
                .onto_design_system_head(&known, &head.text(), ids)
                .map_err(read_error)?;
            doc.to_string()
        }
        None => {
            let body = format!(
                "{{\"kind\":\"design-system\",\"pillar\":\"ui\",\"title\":{}}}",
                quote("Design system")
            );
            let response = agent.request("POST", &format!("{base}/docs"), Some(body.as_bytes()))?;
            id_of(
                &parsed(&response.text(), "the new doc")?,
                "doc",
                "the new doc",
            )?
        }
    };
    agent.request(
        "PUT",
        &format!("{base}/tokens"),
        Some(body_of("tokens", &import.token_tree_json(ids)).as_bytes()),
    )?;
    if import.component_count() == 0 {
        return Ok(doc);
    }
    for _ in 0..MAX_PASSES {
        let rendered = import.component_tags(ids).map_err(read_error)?;
        let response = agent.request(
            "PUT",
            &format!("{base}/components"),
            Some(body_of("tags", &quote(&rendered.tags)).as_bytes()),
        )?;
        import
            .learn_components(ids, &rendered, &response.text())
            .map_err(read_error)?;
        if rendered.complete {
            return Ok(doc);
        }
    }
    bail!(
        "the components nest too deep to be sent in {MAX_PASSES} steps; the tokens are in the design system, but its components are not complete"
    )
}

/// Starts the design doc, at the app level or in the feature, and writes the
/// pages into its head. Returns the doc's ID.
fn send_design(
    agent: &Agent,
    base: &str,
    feature: Option<&str>,
    import: &Import,
    ids: &Ids,
    title: &str,
) -> Result<String> {
    let parts = import.design_parts(ids).map_err(read_error)?;
    let body = format!(
        "{{\"kind\":\"design\",\"pillar\":\"ui\",\"title\":{}}}",
        quote(title)
    );
    let path = match feature {
        Some(feature) => format!("{base}/features/{feature}/docs"),
        None => format!("{base}/docs"),
    };
    let response = agent.request("POST", &path, Some(body.as_bytes()))?;
    let doc = id_of(
        &parsed(&response.text(), "the new doc")?,
        "doc",
        "the new doc",
    )?;

    // A new design starts with one empty page, which the import's pages replace.
    let head = agent.request("GET", &format!("{base}/docs/{doc}/html"), None)?;
    let head = parsed(&head.text(), "the new doc's head")?;
    let empty = head
        .get("pages")
        .and_then(Value::as_array)
        .and_then(|pages| pages.first())
        .and_then(|page| page.str_at("id"))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(
                "the server's answer about the new design has no page; is it an OmniState API?",
            )
        })?;
    let ops = format!("{base}/docs/{doc}/ops");
    let mut after = None;
    for part in &parts {
        let page = send_page(agent, &ops, part, after.as_deref())?;
        after = Some(page);
    }
    let delete = format!(
        "[{{\"op\":\"delete\",\"node\":{},\"rev\":1}}]",
        quote(&empty)
    );
    send_ops(agent, &ops, &delete)?;
    Ok(doc)
}

/// Inserts the page after `after` (the first page when it is none) and its
/// boards into it, in batches of about [`BATCH_BYTES`]. Returns the page's ID.
fn send_page(agent: &Agent, ops: &str, part: &PageTags, after: Option<&str>) -> Result<String> {
    let insert = |parent: Option<&str>, after: Option<&str>, tags: &str| {
        format!(
            "{{\"op\":\"insert\",\"parent\":{},\"position\":{{\"after\":{}}},\"content\":{}}}",
            parent.map_or_else(|| "null".to_string(), quote),
            after.map_or_else(|| "null".to_string(), quote),
            quote(tags)
        )
    };
    let created = send_ops(
        agent,
        ops,
        &format!("[{}]", insert(None, after, &part.page)),
    )?;
    let page = node_of(&created, 0)?;

    // Operations that go after the same board land in reverse order of
    // sending (docs/formats/operations.md, insert), so a batch lists its
    // boards backwards, all after the last board of the batch before.
    let mut last: Option<String> = None;
    let mut rest = part.boards.as_slice();
    while !rest.is_empty() {
        let mut size = 0;
        let mut count = 0;
        for tags in rest {
            if count > 0 && size + tags.len() > BATCH_BYTES {
                break;
            }
            size += tags.len();
            count += 1;
        }
        let (batch, later) = rest.split_at(count);
        let inserts: Vec<String> = batch
            .iter()
            .rev()
            .map(|tags| insert(Some(&page), last.as_deref(), tags))
            .collect();
        let created = send_ops(agent, ops, &format!("[{}]", inserts.join(",")))?;
        last = Some(node_of(&created, 0)?);
        rest = later;
    }
    Ok(page)
}

/// Sends a batch of operations to the doc's head; returns the answer.
fn send_ops(agent: &Agent, ops: &str, batch: &str) -> Result<Value> {
    let response = agent.request("POST", ops, Some(body_of("ops", batch).as_bytes()))?;
    parsed(&response.text(), "the operations")
}

/// The node the `index`th operation of an answer made or changed.
fn node_of(answer: &Value, index: usize) -> Result<String> {
    answer
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.get(index))
        .and_then(|result| result.str_at("node"))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(
                "the server's answer about the operations has no node; is it an OmniState API?",
            )
        })
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
    design: Option<&str>,
    design_system: Option<&str>,
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
        "{{\"design_doc_id\":{},\"design_system_doc_id\":{},\"report\":[{}]}}",
        optional(design),
        optional(design_system),
        entries.join(",")
    )
}
