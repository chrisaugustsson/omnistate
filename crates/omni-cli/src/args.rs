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
    /// `OMNI_PROJECT` or the one the folder is linked to.
    pub project: Option<String>,
    /// `--feature <name or id>`: the feature a command works in, instead of
    /// `OMNI_FEATURE`; `--global` works at the app level whatever
    /// `OMNI_FEATURE` says.
    pub feature: Option<String>,
    pub global: bool,
    /// `--title` and `--description` describe a doc or a feature.
    pub title: Option<String>,
    pub description: Option<String>,
    /// `--kind` and `--pillar` say what kind of doc `omni doc create` starts
    /// and where; `--pillar` also filters `omni doc list`.
    pub kind: Option<String>,
    pub pillar: Option<String>,
    /// `omni wait --timeout <seconds>`; checked by the command.
    pub timeout: Option<String>,
    /// `omni wait --thread <id> --posts <n> --concluded`: wait on a thread for
    /// this many posts or for its conclusion.
    pub thread: Option<String>,
    pub posts: Option<String>,
    pub concluded: bool,
    /// `omni design read --doc <id> --board <id>`: the design doc and the
    /// board in it to read.
    pub doc: Option<String>,
    pub board: Option<String>,
    /// `omni doc activity <doc> --limit <n> --before <seq>`: how many entries
    /// to list and where the page ends.
    pub limit: Option<String>,
    pub before: Option<String>,
    /// `omni tokens export --out <file or directory>`: where to write.
    pub out: Option<String>,
    /// `omni tokens export --per-mode`: one file per mode.
    pub per_mode: bool,
    /// `omni thread create <question> --sealed`.
    pub sealed: bool,
    /// `omni post <thread> --summary <text> [<body>] [--attach <file>]...
    /// [--link <target>]... [--confidence <n>] [--replaces <post>]`.
    pub summary: Option<String>,
    pub attach: Vec<String>,
    pub link: Vec<String>,
    pub confidence: Option<String>,
    pub replaces: Option<String>,
    /// `omni story add --priority <n> --number-from <n>`, `omni story list
    /// --status <s> --pickable`.
    pub priority: Option<String>,
    pub number_from: Option<String>,
    pub status: Option<String>,
    pub pickable: bool,
    /// The options of the `omni story` writes: `--run`, `--stage`, `--to`,
    /// `--criterion`, `--finding`, `--question`, `--choice`, `--by`, `--text`,
    /// `--blocked-by`, `--file` (repeatable), `--pending` and `--unticked`.
    pub run: Option<String>,
    pub stage: Option<String>,
    pub to: Option<String>,
    pub criterion: Option<String>,
    pub finding: Option<String>,
    pub question: Option<String>,
    pub choice: Option<String>,
    pub by: Option<String>,
    pub text: Option<String>,
    pub blocked_by: Option<String>,
    pub file: Vec<String>,
    pub pending: bool,
    pub unticked: bool,
    pub json: bool,
    pub help: bool,
    pub version: bool,
}

/// Whether the words so far name a command that US-108 removed.
fn is_gone(words: &[String]) -> bool {
    let command: Vec<&str> = words.iter().map(String::as_str).collect();
    matches!(command.as_slice(), ["thread", "phase", ..])
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
            "--per-mode" => {
                parsed.per_mode = true;
                continue;
            }
            "--sealed" => {
                parsed.sealed = true;
                continue;
            }
            "--pickable" => {
                parsed.pickable = true;
                continue;
            }
            "--pending" => {
                parsed.pending = true;
                continue;
            }
            "--unticked" => {
                parsed.unticked = true;
                continue;
            }
            "--concluded" => {
                parsed.concluded = true;
                continue;
            }
            "--global" => {
                parsed.global = true;
                continue;
            }
            "--remove-commented" => {
                bail!(
                    "--remove-commented is gone: a write that removes a commented node always goes through, and the comment stays, orphaned, until the human deletes it"
                )
            }
            "--file" => {
                let Some(value) = inline.or_else(|| args.next()) else {
                    bail!("{arg} needs a value; see `omni --help`");
                };
                parsed.file.push(value);
                continue;
            }
            "--attach" | "--link" => {
                let Some(value) = inline.or_else(|| args.next()) else {
                    bail!("{arg} needs a value; see `omni --help`");
                };
                if name == "--attach" {
                    parsed.attach.push(value);
                } else {
                    parsed.link.push(value);
                }
                continue;
            }
            "--priority" => &mut parsed.priority,
            "--number-from" => &mut parsed.number_from,
            "--status" => &mut parsed.status,
            "--run" => &mut parsed.run,
            "--stage" => &mut parsed.stage,
            "--to" => &mut parsed.to,
            "--criterion" => &mut parsed.criterion,
            "--finding" => &mut parsed.finding,
            "--question" => &mut parsed.question,
            "--choice" => &mut parsed.choice,
            "--by" => &mut parsed.by,
            "--text" => &mut parsed.text,
            "--blocked-by" => &mut parsed.blocked_by,
            "--summary" => &mut parsed.summary,
            "--confidence" => &mut parsed.confidence,
            "--replaces" => &mut parsed.replaces,
            "--out" => &mut parsed.out,
            "--as" => &mut parsed.agent,
            "--model" => &mut parsed.model,
            "--harness" => &mut parsed.harness,
            "--server" => &mut parsed.server,
            "--body-file" => &mut parsed.body_file,
            "--project" => &mut parsed.project,
            "--feature" => &mut parsed.feature,
            "--title" => &mut parsed.title,
            "--description" => &mut parsed.description,
            "--doc" => &mut parsed.doc,
            "--board" => &mut parsed.board,
            "--limit" => &mut parsed.limit,
            "--before" => &mut parsed.before,
            "--kind" => &mut parsed.kind,
            "--pillar" => &mut parsed.pillar,
            "--module" => {
                bail!("--module is now --pillar; use --pillar spec|ui|architecture|db|research")
            }
            "--timeout" => &mut parsed.timeout,
            "--thread" => &mut parsed.thread,
            "--posts" => &mut parsed.posts,
            // `omni thread phase` is gone, whatever options it is tried with:
            // the command itself says so.
            _ if is_gone(&parsed.words) => break,
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
    fn doc_options_take_values_and_version_shows_omnis_version() {
        let parsed = parse_strs(&["doc", "create", "--kind=html", "--pillar", "spec"]).unwrap();
        assert_eq!(parsed.words, ["doc", "create"]);
        assert_eq!(parsed.kind.as_deref(), Some("html"));
        assert_eq!(parsed.pillar.as_deref(), Some("spec"));

        // Anywhere on the line it is the flag that shows omni's version.
        assert!(parse_strs(&["--version"]).unwrap().version);
        assert!(
            parse_strs(&["doc", "show", "d", "--version"])
                .unwrap()
                .version
        );
    }

    #[test]
    fn bad_options_are_errors() {
        assert!(parse_strs(&["--bogus"]).is_err());
        assert!(parse_strs(&["whoami", "--as"]).is_err());
        assert!(parse_strs(&["-x"]).is_err());
    }
}
