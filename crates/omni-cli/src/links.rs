//! `~/.config/omni/links.toml`, which holds the project each folder is linked
//! to, and `omni link`, which writes it. The file has one `[<workspace id>]`
//! table per workspace and, under it, one `"<path>" = "<project id>"` line per
//! linked folder. The parts a folder holds are lines of a `[parts-<workspace
//! id>]` table: `"<path>\t<part name>" = "<path of the part under the folder>"`,
//! empty when the part is the folder itself. It holds no secret, so it keeps
//! the default file mode.

use crate::args::Args;
use crate::commands::{agent, say};
use crate::config;
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote as json_quote};
use crate::projects::{is_uuid, project_id, projects_path};
use crate::toml;
use crate::tracks::project_parts;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn links_path() -> Result<PathBuf> {
    let mut path = config::config_path()?;
    path.set_file_name("links.toml");
    Ok(path)
}

/// The links of every workspace, in the order of the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    /// `(workspace id, folder path, project id)`.
    entries: Vec<(String, String, String)>,
    /// `(workspace id, folder path, part name, path under the folder)`.
    parts: Vec<(String, String, String, String)>,
}

/// The prefix of the table that holds a workspace's parts.
const PARTS_TABLE: &str = "parts-";

impl Links {
    /// Whether no folder is linked at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The links in the file; none when the file does not exist.
    pub fn load() -> Result<Links> {
        let path = links_path()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Links::default()),
            Err(e) => bail!("cannot read {}: {e}", path.display()),
        };
        let invalid = |detail: &str| {
            CliError::new(format!(
                "{} is not valid ({detail}); fix it or remove it",
                path.display()
            ))
        };
        let pairs = toml::parse(&text).map_err(|e| invalid(&e))?;
        let mut entries = Vec::new();
        let mut parts = Vec::new();
        for (key, value) in pairs {
            let (workspace, folder) = key.split_once('.').ok_or_else(|| {
                invalid(&format!("the link {key:?} is outside a workspace table"))
            })?;
            if let Some(workspace) = workspace.strip_prefix(PARTS_TABLE) {
                let (folder, part) = folder
                    .rsplit_once('\t')
                    .ok_or_else(|| invalid(&format!("the part line {key:?} names no part")))?;
                parts.push((
                    workspace.to_string(),
                    folder.to_string(),
                    part.to_string(),
                    value,
                ));
            } else {
                entries.push((workspace.to_string(), folder.to_string(), value));
            }
        }
        Ok(Links { entries, parts })
    }

    /// Links `path` to `project_id` in a workspace. A link of the same path
    /// keeps its line; a new one goes at the end of its workspace's table,
    /// and a new table at the end of the file.
    pub fn set(&mut self, workspace_id: &str, path: &str, project_id: &str) {
        let entry = (
            workspace_id.to_string(),
            path.to_string(),
            project_id.to_string(),
        );
        if let Some(found) = self
            .entries
            .iter_mut()
            .find(|(w, p, _)| w == workspace_id && p == path)
        {
            if found.2 != project_id {
                self.parts
                    .retain(|(w, f, _, _)| !(w == workspace_id && f == path));
            }
            *found = entry;
        } else if let Some(last) = self.entries.iter().rposition(|(w, _, _)| w == workspace_id) {
            self.entries.insert(last + 1, entry);
        } else {
            self.entries.push(entry);
        }
    }

    /// Says that `part` lives in `path` under the linked folder `folder`
    /// (empty for the folder itself). The folder's other parts stay.
    pub fn set_part(&mut self, workspace_id: &str, folder: &str, part: &str, path: &str) {
        let entry = (
            workspace_id.to_string(),
            folder.to_string(),
            part.to_string(),
            path.to_string(),
        );
        if let Some(found) = self
            .parts
            .iter_mut()
            .find(|(w, f, n, _)| w == workspace_id && f == folder && n == part)
        {
            *found = entry;
        } else {
            self.parts.push(entry);
        }
    }

    /// The parts a linked folder holds, as `(name, path)`, in the file's order.
    pub fn parts_of(&self, workspace_id: &str, folder: &str) -> Vec<(String, String)> {
        self.parts
            .iter()
            .filter(|(w, f, _, _)| w == workspace_id && f == folder)
            .map(|(_, _, name, path)| (name.clone(), path.clone()))
            .collect()
    }

    /// Removes the line of `path` in a workspace's table; false when it has
    /// none. Other lines, and the table's header while it has lines, stay.
    /// The parts of the folder go with it.
    pub fn remove(&mut self, workspace_id: &str, path: &str) -> bool {
        self.parts
            .retain(|(w, f, _, _)| !(w == workspace_id && f == path));
        let before = self.entries.len();
        self.entries
            .retain(|(w, p, _)| !(w == workspace_id && p == path));
        self.entries.len() != before
    }

    /// Writes the whole file to a new file next to it and moves that into
    /// place, so a reader never sees half of it.
    pub fn save(&self) -> Result<()> {
        let path = links_path()?;
        let dir = path.parent().expect("the links path has a directory");
        std::fs::create_dir_all(dir)
            .map_err(|e| CliError::new(format!("cannot create {}: {e}", dir.display())))?;

        let mut workspaces: Vec<&str> = Vec::new();
        for (workspace, _, _) in &self.entries {
            if !workspaces.contains(&workspace.as_str()) {
                workspaces.push(workspace);
            }
        }
        let mut text = String::new();
        for workspace in workspaces {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("[{workspace}]\n"));
            for (_, folder, project) in self.entries.iter().filter(|(w, _, _)| w == workspace) {
                text.push_str(&format!(
                    "{} = {}\n",
                    toml::quote(folder),
                    toml::quote(project)
                ));
            }
            if self.parts.iter().any(|(w, _, _, _)| w == workspace) {
                text.push_str(&format!("\n[{PARTS_TABLE}{workspace}]\n"));
                for (_, folder, part, path) in
                    self.parts.iter().filter(|(w, _, _, _)| w == workspace)
                {
                    text.push_str(&format!(
                        "{} = {}\n",
                        toml::quote(&format!("{folder}\t{part}")),
                        toml::quote(path)
                    ));
                }
            }
        }

        let temp = dir.join("links.toml.tmp");
        let _ = std::fs::remove_file(&temp);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .and_then(|mut file| {
                file.write_all(text.as_bytes())
                    .and_then(|()| file.sync_all())
            })
            .and_then(|()| std::fs::rename(&temp, &path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            bail!("cannot write {}: {e}", path.display());
        }
        Ok(())
    }
}

/// What the links say about a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The nearest folder linked in the logged-in workspace, found through the
    /// folder's own parents or, when `via_main_worktree`, through the main
    /// worktree of the repository the folder is a worktree of.
    Linked {
        matched: PathBuf,
        project_id: String,
        via_main_worktree: bool,
    },
    /// No such folder, but the nearest linked one belongs to another workspace.
    OtherWorkspace {
        matched: PathBuf,
        workspace_id: String,
    },
    None,
}

/// The main worktree of the repository `dir` is in: the parent of git's common
/// directory. None when git is missing or fails, or `dir` is in no repository.
pub fn main_worktree(dir: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let common = PathBuf::from(
        String::from_utf8(output.stdout)
            .ok()?
            .trim_end_matches('\n'),
    );
    let main = common.parent()?;
    Some(std::fs::canonicalize(main).unwrap_or_else(|_| main.to_path_buf()))
}

/// A folder and the project id or workspace id its link names.
type Match = (PathBuf, String);

/// The nearest folder at or above `dir` with a line in the workspace's table,
/// else the nearest one with a line in another workspace's table.
fn nearest(links: &Links, workspace_id: &str, dir: &Path) -> (Option<Match>, Option<Match>) {
    let mut other = None;
    for folder in dir.ancestors() {
        let Some(key) = folder.to_str() else {
            continue;
        };
        let mut here = links.entries.iter().filter(|(_, path, _)| path == key);
        if let Some((_, _, project_id)) = here.clone().find(|(w, _, _)| w == workspace_id) {
            return (Some((folder.to_path_buf(), project_id.clone())), other);
        }
        if other.is_none() {
            other = here
                .next()
                .map(|(w, _, _)| (folder.to_path_buf(), w.clone()));
        }
    }
    (None, other)
}

/// The link in effect for `dir`: the folder itself, then each parent, the
/// first with a line in the workspace's table winning. When none has one and
/// `dir` is in a git worktree, the same lookup for the main worktree. Only when
/// neither finds one does a link of another workspace count, the folder's own
/// nearest first.
pub fn resolve(links: &Links, workspace_id: &str, dir: &Path) -> Resolution {
    let (linked, other) = nearest(links, workspace_id, dir);
    if let Some((matched, project_id)) = linked {
        return Resolution::Linked {
            matched,
            project_id,
            via_main_worktree: false,
        };
    }
    let main = main_worktree(dir).map(|main| nearest(links, workspace_id, &main));
    let (main_linked, main_other) = main.unwrap_or_default();
    if let Some((matched, project_id)) = main_linked {
        return Resolution::Linked {
            matched,
            project_id,
            via_main_worktree: true,
        };
    }
    match other.or(main_other) {
        Some((matched, workspace_id)) => Resolution::OtherWorkspace {
            matched,
            workspace_id,
        },
        None => Resolution::None,
    }
}

/// `omni link <project name|id>`: links the current folder to a project of the
/// logged-in workspace.
pub fn link(args: &Args, project: Option<&str>) -> Result<()> {
    let Some(given) = project else {
        bail!("a link needs the project: omni link <project name|id>");
    };
    let agent = agent(args)?;
    let folder = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| CliError::new(format!("cannot find the current folder: {e}")))?;
    let folder = folder.to_string_lossy().into_owned();

    let response = agent.request("GET", &projects_path(&agent), None)?;
    let body = Value::parse(&response.text()).map_err(|_| {
        CliError::new(
            "the server's answer about the projects is not readable; is it an OmniState API?",
        )
    })?;
    let projects = body
        .get("projects")
        .and_then(Value::as_array)
        .unwrap_or_default();
    let found = projects.iter().find(|candidate| {
        if is_uuid(given) {
            candidate
                .str_at("id")
                .is_some_and(|id| id.eq_ignore_ascii_case(given))
        } else {
            candidate.str_at("name") == Some(given)
        }
    });
    let (Some(id), Some(name)) = (
        found.and_then(|p| p.str_at("id")),
        found.and_then(|p| p.str_at("name")),
    ) else {
        bail!("no project named {given:?}; `omni project list` shows the projects");
    };

    let mut wanted = Vec::new();
    if !args.parts.is_empty() {
        let known = project_parts(&agent, id)?;
        for given in &args.parts {
            let (part, path) = given.split_once('=').unwrap_or((given, ""));
            let part = part.trim();
            let canonical = known
                .iter()
                .find(|candidate| {
                    candidate
                        .str_at("name")
                        .is_some_and(|n| n.eq_ignore_ascii_case(part))
                })
                .and_then(|candidate| candidate.str_at("name"))
                .ok_or_else(|| {
                    CliError::new(format!(
                        "unknown_part: {name} has no part named {part:?}; `omni part list` shows the parts"
                    ))
                })?;
            wanted.push((canonical.to_string(), path.to_string()));
        }
    }

    let mut links = Links::load()?;
    links.set(&agent.workspace_id, &folder, id);
    for (part, path) in &wanted {
        links.set_part(&agent.workspace_id, &folder, part, path);
    }
    links.save()?;
    let parts = links.parts_of(&agent.workspace_id, &folder);

    if args.json {
        say(&format!(
            "{{\"path\": {}, \"workspace_id\": {}, \"project\": {{\"id\": {}, \"name\": {}}}{}}}",
            json_quote(&folder),
            json_quote(&agent.workspace_id),
            json_quote(id),
            json_quote(name),
            parts_json(&parts)
        ));
    } else {
        say(&format!("linked {folder} to {name} ({id})"));
        for line in part_lines(&folder, &parts) {
            say(&line);
        }
    }
    Ok(())
}

/// `, "parts": [...]` for a folder's parts; nothing when it has none.
fn parts_json(parts: &[(String, String)]) -> String {
    if parts.is_empty() {
        return String::new();
    }
    let items: Vec<String> = parts
        .iter()
        .map(|(name, path)| {
            format!(
                "{{\"name\": {}, \"path\": {}}}",
                json_quote(name),
                json_quote(path)
            )
        })
        .collect();
    format!(", \"parts\": [{}]", items.join(", "))
}

/// One line per part: its name and its path, the folder itself when it has none.
fn part_lines(folder: &str, parts: &[(String, String)]) -> Vec<String> {
    parts
        .iter()
        .map(|(name, path)| {
            let shown = if path.is_empty() { folder } else { path };
            format!("  {name}  {shown}")
        })
        .collect()
}

/// `omni link` alone: the folder that matched and its project, found without
/// `--project` and `OMNI_PROJECT`. A project the workspace's list does not
/// hold, or a list that cannot be fetched, shows as not found.
pub fn show(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let folder = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| CliError::new(format!("cannot find the current folder: {e}")))?;
    let (matched, project_id, via_main_worktree) = match resolve(
        &Links::load()?,
        &agent.workspace_id,
        &folder,
    ) {
        Resolution::Linked {
            matched,
            project_id,
            via_main_worktree,
        } => (matched, project_id, via_main_worktree),
        Resolution::OtherWorkspace {
            matched,
            workspace_id,
        } => bail!(
            "{} is linked to a project of another workspace ({workspace_id}); run omni link here, pass --project, or leave out --workspace and OMNI_WORKSPACE",
            matched.display()
        ),
        Resolution::None => bail!(
            "this folder is not linked to a project: run omni link <project name|id> here, or pass --project"
        ),
    };

    let matched_key = matched.to_string_lossy().into_owned();
    let parts = Links::load()?.parts_of(&agent.workspace_id, &matched_key);
    if args.json {
        say(&format!(
            "{{\"path\": {}, \"matched\": {}, \"workspace_id\": {}, \"project_id\": {}{}}}",
            json_quote(&folder.to_string_lossy()),
            json_quote(&matched.to_string_lossy()),
            json_quote(&agent.workspace_id),
            json_quote(&project_id),
            parts_json(&parts)
        ));
        return Ok(());
    }

    let name = agent
        .request("GET", &projects_path(&agent), None)
        .ok()
        .and_then(|response| Value::parse(&response.text()).ok())
        .and_then(|body| {
            body.get("projects")
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .find(|p| {
                    p.str_at("id")
                        .is_some_and(|id| id.eq_ignore_ascii_case(&project_id))
                })
                .and_then(|p| p.str_at("name").map(str::to_string))
        })
        .unwrap_or_else(|| "(project not found in this workspace)".to_string());
    let matched = matched.display();
    let origin = if via_main_worktree {
        format!("{matched} (main worktree of {})", folder.display())
    } else {
        matched.to_string()
    };
    say(&format!("{origin} is linked to {name} ({project_id})"));
    for line in part_lines(&matched_key, &parts) {
        say(&line);
    }
    Ok(())
}

/// `omni part where <part>`: the absolute folder of a part, found among the
/// folders linked to the project of the current folder; the first in the
/// file's order that lists it.
pub fn part_where(args: &Args, part: Option<&str>) -> Result<()> {
    let Some(given) = part else {
        bail!("a part needs its name: omni part where <name|id>");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let known = project_parts(&agent, &project)?;
    let given = given.trim();
    let name = known
        .iter()
        .find(|candidate| {
            candidate
                .str_at("name")
                .is_some_and(|n| n.eq_ignore_ascii_case(given))
                || (is_uuid(given)
                    && candidate
                        .str_at("id")
                        .is_some_and(|id| id.eq_ignore_ascii_case(given)))
        })
        .and_then(|candidate| candidate.str_at("name"))
        .ok_or_else(|| {
            CliError::new(format!(
                "unknown_part: no part named {given:?}; `omni part list` shows the parts"
            ))
        })?;

    let links = Links::load()?;
    for (workspace, folder, linked) in &links.entries {
        if *workspace != agent.workspace_id || !linked.eq_ignore_ascii_case(&project) {
            continue;
        }
        let found = links
            .parts_of(workspace, folder)
            .into_iter()
            .find(|(part, _)| part.eq_ignore_ascii_case(name));
        if let Some((_, path)) = found {
            let full = Path::new(folder).join(path);
            if args.json {
                say(&format!(
                    "{{\"part\": {}, \"path\": {}}}",
                    json_quote(name),
                    json_quote(&full.to_string_lossy())
                ));
            } else {
                say(&full.to_string_lossy());
            }
            return Ok(());
        }
    }
    bail!(
        "not_linked: no folder linked to this project lists the part {name:?}; run omni link <project> --part {name}=<path> in its folder"
    )
}

/// `omni unlink`: removes the current folder's own line, never the link of an
/// ancestor or of the main worktree.
pub fn unlink(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let folder = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| CliError::new(format!("cannot find the current folder: {e}")))?;
    let mut links = Links::load()?;
    if links.remove(&agent.workspace_id, &folder.to_string_lossy()) {
        links.save()?;
        say(&format!("unlinked {}", folder.display()));
        return Ok(());
    }
    match resolve(&links, &agent.workspace_id, &folder) {
        Resolution::Linked { matched, .. } => bail!(
            "this folder has no link of its own; the link in effect is {}: run omni unlink there",
            matched.display()
        ),
        Resolution::OtherWorkspace { .. } | Resolution::None => bail!("this folder is not linked"),
    }
}
