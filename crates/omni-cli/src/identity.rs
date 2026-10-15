//! The agent identity that every request made with an API key carries: the run
//! and role (required) and the model and harness (optional), from `--as
//! <run>/<role>`, `--model` and `--harness`, falling back to `OMNI_RUN`,
//! `OMNI_ROLE`, `OMNI_MODEL` and `OMNI_HARNESS`. The server records what the
//! agent declares; it does not verify it.

use crate::args::Args;
use crate::error::{Result, bail};

#[derive(Debug, PartialEq, Eq)]
pub struct Identity {
    pub run: String,
    pub role: String,
    pub model: Option<String>,
    pub harness: Option<String>,
}

const HINT: &str =
    "pass --as <run>/<role>, such as --as run-7/explorer, or set OMNI_RUN and OMNI_ROLE";

impl Identity {
    pub fn resolve(args: &Args) -> Result<Identity> {
        Self::resolve_with(args, |name| std::env::var(name).ok())
    }

    fn resolve_with(args: &Args, env: impl Fn(&str) -> Option<String>) -> Result<Identity> {
        let env = |name: &str| {
            env(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let (run, role) = match &args.agent {
            Some(flag) => match flag.split_once('/') {
                Some((run, role))
                    if !run.trim().is_empty() && !role.trim().is_empty() && !role.contains('/') =>
                {
                    (Some(run.trim().to_string()), Some(role.trim().to_string()))
                }
                _ => bail!(
                    "--as {flag:?} is not an identity; it needs the form <run>/<role>, such as --as run-7/explorer"
                ),
            },
            None => (env("OMNI_RUN"), env("OMNI_ROLE")),
        };
        let (run, role) = match (run, role) {
            (Some(run), Some(role)) => (run, role),
            (None, None) => bail!("no agent identity: {HINT}"),
            (Some(_), None) => bail!("the agent identity has a run but no role: {HINT}"),
            (None, Some(_)) => bail!("the agent identity has a role but no run: {HINT}"),
        };
        let identity = Identity {
            run,
            role,
            model: args
                .model
                .clone()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| env("OMNI_MODEL")),
            harness: args
                .harness
                .clone()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| env("OMNI_HARNESS")),
        };
        for value in [
            Some(&identity.run),
            Some(&identity.role),
            identity.model.as_ref(),
            identity.harness.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !value.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                bail!("the agent identity {value:?} must be plain printable ASCII");
            }
        }
        Ok(identity)
    }

    /// The headers that carry the identity.
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![
            ("X-Omni-Run".to_string(), self.run.clone()),
            ("X-Omni-Role".to_string(), self.role.clone()),
        ];
        if let Some(model) = &self.model {
            headers.push(("X-Omni-Model".to_string(), model.clone()));
        }
        if let Some(harness) = &self.harness {
            headers.push(("X-Omni-Harness".to_string(), harness.clone()));
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(flags: &[&str], env: &[(&str, &str)]) -> Result<Identity> {
        let args = crate::args::parse(flags.iter().map(|f| f.to_string())).unwrap();
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        Identity::resolve_with(&args, |name| {
            env.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
        })
    }

    #[test]
    fn flags_win_and_the_environment_fills_in() {
        let env = [
            ("OMNI_RUN", "e-run"),
            ("OMNI_ROLE", "e-role"),
            ("OMNI_MODEL", "e-model"),
            ("OMNI_HARNESS", "e-h"),
        ];
        let id = resolve(&["--as", "f-run/f-role", "--model", "f-model"], &env).unwrap();
        assert_eq!((id.run.as_str(), id.role.as_str()), ("f-run", "f-role"));
        assert_eq!(
            (id.model.as_deref(), id.harness.as_deref()),
            (Some("f-model"), Some("e-h"))
        );
        let id = resolve(&[], &env).unwrap();
        assert_eq!(id.headers().len(), 4);
        assert_eq!(resolve(&["--as", "a/b"], &[]).unwrap().headers().len(), 2);
    }

    #[test]
    fn incomplete_identities_name_the_fix() {
        for (flags, env) in [
            (vec![], vec![]),
            (vec!["--as", "run-1"], vec![]),
            (vec!["--as", "run-1/"], vec![]),
            (vec!["--as", "/role"], vec![]),
            (vec!["--as", "a/b/c"], vec![]),
            (vec![], vec![("OMNI_RUN", "r")]),
            (vec![], vec![("OMNI_ROLE", "r")]),
            (vec!["--as", "a/b\u{1}"], vec![]),
        ] {
            let message = resolve(&flags, &env).unwrap_err().to_string();
            assert!(
                message.contains("<run>/<role>") || message.contains("printable"),
                "{message}"
            );
        }
    }
}
