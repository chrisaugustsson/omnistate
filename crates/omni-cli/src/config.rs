//! The two files the CLI reads: `~/.config/omni/config.toml`, which `omni login`
//! writes (mode 600, because it holds the API key), and `.omni.toml`, which a
//! project directory uses to set the default project and feature for commands
//! run in it.

use crate::error::{CliError, Result, bail};
use crate::toml;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

/// What `omni login` stores.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    pub server: Option<String>,
    pub workspace_id: Option<String>,
    pub api_key: Option<String>,
    pub email: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .ok_or_else(|| {
            CliError::new("HOME is not set, so there is nowhere to keep the config file")
        })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("omni")
        .join("config.toml"))
}

/// The stored login; `None` when the file does not exist.
pub fn load() -> Result<Option<Stored>> {
    let path = config_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => bail!("cannot read {}: {e}", path.display()),
    };
    let pairs = toml::parse(&text).map_err(|e| {
        CliError::new(format!(
            "{} is not valid ({e}); fix it or run `omni login` to write it again",
            path.display()
        ))
    })?;
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    Ok(Some(Stored {
        server: get("server"),
        workspace_id: get("workspace_id"),
        api_key: get("api_key"),
        email: get("email"),
    }))
}

/// Writes the config file with mode 600. It is written to a new file next to
/// it and moved into place, so a file with a looser mode is replaced and the
/// secret is never readable by others, not even for a moment.
pub fn save(stored: &Stored) -> Result<PathBuf> {
    let path = config_path()?;
    let dir = path.parent().expect("the config path has a directory");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| CliError::new(format!("cannot create {}: {e}", dir.display())))?;

    let mut text = String::new();
    for (key, value) in [
        ("server", &stored.server),
        ("workspace_id", &stored.workspace_id),
        ("api_key", &stored.api_key),
        ("email", &stored.email),
    ] {
        if let Some(value) = value {
            text.push_str(&format!("{key} = {}\n", toml::quote(value)));
        }
    }

    let temp = dir.join("config.toml.tmp");
    let _ = std::fs::remove_file(&temp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
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
    Ok(path)
}

/// The default project: `project` in the nearest `.omni.toml`, looking in the
/// current directory and then in each parent.
pub fn project() -> Result<Option<String>> {
    omni_toml("project", "project = \"my-project\"")
}

/// The default feature: `feature` in the nearest `.omni.toml`, found as the
/// project is.
pub fn feature() -> Result<Option<String>> {
    omni_toml("feature", "feature = \"my-feature\"")
}

/// The value of `key` in the nearest `.omni.toml`; `example` is the line the
/// error shows for a file that is not valid.
fn omni_toml(key: &str, example: &str) -> Result<Option<String>> {
    let Ok(mut dir) = std::env::current_dir() else {
        return Ok(None);
    };
    loop {
        let file = dir.join(".omni.toml");
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                let pairs = toml::parse(&text).map_err(|e| {
                    CliError::new(format!(
                        "{} is not valid ({e}); it should hold a line like {example}",
                        file.display()
                    ))
                })?;
                return Ok(pairs
                    .into_iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!("cannot read {}: {e}", file.display()),
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}
