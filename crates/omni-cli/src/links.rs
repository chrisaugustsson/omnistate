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
    /// The nearest folder linked in the logged-in workspace.
    Linked {
        matched: PathBuf,
        project_id: String,
    },
    /// No such folder, but the nearest linked one belongs to another workspace.
    OtherWorkspace {
        matched: PathBuf,
        workspace_id: String,
    },
    None,
}

/// The link in effect for `dir`: the folder itself, then each parent, the
/// first with a line in the workspace's table winning. Only when none has one
/// does a link of another workspace count, the nearest of them.
pub fn resolve(links: &Links, workspace_id: &str, dir: &Path) -> Resolution {
    let mut other = None;
    for folder in dir.ancestors() {
        let Some(key) = folder.to_str() else {
            continue;
        };
        let mut here = links.entries.iter().filter(|(_, path, _)| path == key);
        if let Some((_, _, project_id)) = here.clone().find(|(w, _, _)| w == workspace_id) {
            return Resolution::Linked {
                matched: folder.to_path_buf(),
                project_id: project_id.clone(),
            };
        }
        if other.is_none() {
            other = here.next().map(|(w, _, _)| (folder, w));
        }
    }
    match other {
        Some((folder, workspace_id)) => Resolution::OtherWorkspace {
            matched: folder.to_path_buf(),
            workspace_id: workspace_id.clone(),
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
