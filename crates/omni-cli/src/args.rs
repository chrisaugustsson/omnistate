//! The command line: the words of the command and the global flags, which work
//! before or after the command name.

use crate::error::{Result, bail};

#[derive(Debug, Default)]
pub struct Args {
    /// The command and its arguments, such as `["key", "revoke", "<id>"]`.
    pub words: Vec<String>,
    /// `--as <run>/<role>`
    pub agent: Option<String>,
    pub model: Option<String>,
    pub harness: Option<String>,
    pub server: Option<String>,
    pub body_file: Option<String>,
    /// `--project <name or id>`: the project a command works in, instead of
    /// the one in `.omni.toml`.
    pub project: Option<String>,
    /// `--title`, `--description` and `--run` describe a change set.
    pub title: Option<String>,
    pub description: Option<String>,
    pub run: Option<String>,
    pub json: bool,
    pub help: bool,
    pub version: bool,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args> {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            parsed.words.extend(args.by_ref());
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            parsed.words.push(arg);
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let slot = match name {
            "--help" | "-h" => {
                parsed.help = true;
                continue;
            }
            "--version" | "-V" => {
                parsed.version = true;
                continue;
            }
            "--json" => {
                parsed.json = true;
                continue;
            }
            "--as" => &mut parsed.agent,
            "--model" => &mut parsed.model,
            "--harness" => &mut parsed.harness,
            "--server" => &mut parsed.server,
            "--body-file" => &mut parsed.body_file,
            "--project" => &mut parsed.project,
            "--title" => &mut parsed.title,
            "--description" => &mut parsed.description,
            "--run" => &mut parsed.run,
            // Rejecting is the human's, whatever options it is tried with:
            // the command itself explains that.
            _ if parsed
                .words
                .starts_with(&["changeset".to_string(), "reject".to_string()]) =>
            {
                break;
            }
            _ => bail!("unknown option {arg}; see `omni --help`"),
        };
        match inline.or_else(|| args.next()) {
            Some(value) => *slot = Some(value),
            None => bail!("{arg} needs a value; see `omni --help`"),
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Args> {
        parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn global_flags_work_before_and_after_the_command() {
        for args in [
            ["--as", "r/x", "--json", "whoami"],
            ["whoami", "--as=r/x", "--json", "--"],
        ] {
            let parsed = parse_strs(&args).unwrap();
            assert_eq!(parsed.words, ["whoami"]);
            assert_eq!(parsed.agent.as_deref(), Some("r/x"));
            assert!(parsed.json);
        }
    }

    #[test]
    fn a_dash_is_a_value_and_double_dash_ends_the_flags() {
        let parsed = parse_strs(&["api", "POST", "/x", "--body-file", "-"]).unwrap();
        assert_eq!(parsed.body_file.as_deref(), Some("-"));
        assert_eq!(parsed.words, ["api", "POST", "/x"]);
        let parsed = parse_strs(&["api", "--", "--json"]).unwrap();
        assert_eq!(parsed.words, ["api", "--json"]);
    }

    #[test]
    fn change_set_options_take_values() {
        let parsed = parse_strs(&[
            "changeset",
            "create",
            "--title",
            "T",
            "--description=D",
            "--run",
            "r-1",
            "--project",
            "p",
        ])
        .unwrap();
        assert_eq!(parsed.words, ["changeset", "create"]);
        assert_eq!(parsed.title.as_deref(), Some("T"));
        assert_eq!(parsed.description.as_deref(), Some("D"));
        assert_eq!(parsed.run.as_deref(), Some("r-1"));
        assert_eq!(parsed.project.as_deref(), Some("p"));
    }

    #[test]
    fn rejecting_accepts_any_options_so_the_command_can_explain() {
        let parsed = parse_strs(&["changeset", "reject", "x", "--reason", "no"]).unwrap();
        assert_eq!(parsed.words, ["changeset", "reject", "x"]);
        assert!(parse_strs(&["changeset", "submit", "x", "--reason", "no"]).is_err());
    }

    #[test]
    fn bad_options_are_errors() {
        assert!(parse_strs(&["--bogus"]).is_err());
        assert!(parse_strs(&["whoami", "--as"]).is_err());
        assert!(parse_strs(&["-x"]).is_err());
        assert!(parse_strs(&["--version"]).unwrap().version);
        assert!(parse_strs(&["-h"]).unwrap().help);
    }
}
