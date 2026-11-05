//! `~/.config/omni/links.toml`, which holds the project each folder is linked
//! to, and `omni link`, which writes it. The file has one `[<workspace id>]`
//! table per workspace and, under it, one `"<path>" = "<project id>"` line per
//! linked folder. It holds no secret, so it keeps the default file mode.

use crate::args::Args;
use crate::commands::{agent, say};
use crate::config;
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote as json_quote};
use crate::projects::{is_uuid, projects_path};
use crate::toml;
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
}

impl Links {
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
        for (key, project) in pairs {
            let (workspace, folder) = key.split_once('.').ok_or_else(|| {
                invalid(&format!("the link {key:?} is outside a workspace table"))
            })?;
            entries.push((workspace.to_string(), folder.to_string(), project));
        }
        Ok(Links { entries })
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
            *found = entry;
        } else if let Some(last) = self.entries.iter().rposition(|(w, _, _)| w == workspace_id) {
            self.entries.insert(last + 1, entry);
        } else {
            self.entries.push(entry);
        }
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

    let mut links = Links::load()?;
    links.set(&agent.workspace_id, &folder, id);
    links.save()?;

    if args.json {
        say(&format!(
            "{{\"path\": {}, \"workspace_id\": {}, \"project\": {{\"id\": {}, \"name\": {}}}}}",
            json_quote(&folder),
            json_quote(&agent.workspace_id),
            json_quote(id),
            json_quote(name)
        ));
    } else {
        say(&format!("linked {folder} to {name} ({id})"));
    }
    Ok(())
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
            "{} is linked to a project of another workspace ({workspace_id}); run omni link here, or omni login",
            matched.display()
        ),
        Resolution::None => bail!(
            "this folder is not linked to a project: run omni link <project name|id> here, or pass --project"
        ),
    };

    if args.json {
        say(&format!(
            "{{\"path\": {}, \"matched\": {}, \"workspace_id\": {}, \"project_id\": {}}}",
            json_quote(&folder.to_string_lossy()),
            json_quote(&matched.to_string_lossy()),
            json_quote(&agent.workspace_id),
            json_quote(&project_id)
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
    Ok(())
}
