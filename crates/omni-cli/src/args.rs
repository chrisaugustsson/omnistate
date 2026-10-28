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
    /// `--feature <name or id>`: the feature a change set command works in,
    /// instead of the one in `.omni.toml`; `--global` works at project level
    /// whatever `.omni.toml` says.
    pub feature: Option<String>,
    pub global: bool,
    /// `--title`, `--description` and `--run` describe a change set.
    pub title: Option<String>,
    pub description: Option<String>,
    pub run: Option<String>,
    /// `omni changeset resolve <conflict-id> --note <text>`: how the conflict
    /// was resolved.
    pub note: Option<String>,
    /// `--changeset <id>`, `--kind` and `--pillar` say which change set a doc
    /// command works in and what kind of doc `omni doc create` starts; `--pillar`
    /// also filters `omni doc list`.
    pub changeset: Option<String>,
    pub kind: Option<String>,
    pub pillar: Option<String>,
    /// `omni wait --until <status> --timeout <seconds>`; both are checked by
    /// the command.
    pub until: Option<String>,
    pub timeout: Option<String>,
    /// `omni wait --thread <id> --posts <n> --concluded`: wait on a thread
    /// instead of a change set, for this many posts or for its conclusion.
    pub thread: Option<String>,
    pub posts: Option<String>,
    pub concluded: bool,
    /// `omni doc read --version <v>`, `omni design read --version <v>` and
    /// `omni tokens export --version <v>`: the version to read; for `omni
    /// comment addressed --version <v>`, the version that fixes the comment.
    /// Everywhere else `--version` is the bare flag that shows the version of
    /// omni.
    pub doc_version: Option<String>,
    /// `omni design read --doc <id> --board <id>`: the design doc and the
    /// board in it to read.
    pub doc: Option<String>,
    pub board: Option<String>,
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
    pub json: bool,
    pub help: bool,
    pub version: bool,
}

/// Whether `--version` names a version to read for this command, instead of
/// being the flag that shows the version of omni.
fn takes_a_version(words: &[String]) -> bool {
    let command: Vec<&str> = words.iter().map(String::as_str).collect();
    matches!(
        command.as_slice(),
        ["doc", "read", ..]
            | ["design", "read", ..]
            | ["tokens", "export", ..]
            | ["comment", "addressed", ..]
    )
}

/// Whether the words so far name a command that US-108 removed.
fn is_gone(words: &[String]) -> bool {
    let command: Vec<&str> = words.iter().map(String::as_str).collect();
    matches!(
        command.as_slice(),
        ["decision", ..] | ["thread", "phase", ..]
    )
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
            "--version" if takes_a_version(&parsed.words) => &mut parsed.doc_version,
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
            "--run" => &mut parsed.run,
            "--note" => &mut parsed.note,
            "--changeset" => &mut parsed.changeset,
            "--doc" => &mut parsed.doc,
            "--board" => &mut parsed.board,
            "--kind" => &mut parsed.kind,
            "--pillar" => &mut parsed.pillar,
            "--module" => {
                bail!("--module is now --pillar; use --pillar spec|ui|architecture|db|research")
            }
            "--until" => &mut parsed.until,
            "--timeout" => &mut parsed.timeout,
            "--thread" => &mut parsed.thread,
            "--posts" => &mut parsed.posts,
            // `omni thread phase` and `omni decision` are gone, whatever options
            // they are tried with: the command itself says so.
            _ if is_gone(&parsed.words) => break,
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
    fn feature_takes_a_value_and_global_is_a_flag() {
        let parsed = parse_strs(&["changeset", "list", "--feature=Checkout"]).unwrap();
        assert_eq!(parsed.feature.as_deref(), Some("Checkout"));
        assert!(!parsed.global);
        let parsed = parse_strs(&["changeset", "create", "--global"]).unwrap();
        assert!(parsed.global);
        assert!(parse_strs(&["changeset", "list", "--feature"]).is_err());
    }

    #[test]
    fn resolve_takes_a_note() {
        let parsed = parse_strs(&["changeset", "resolve", "c", "--note", "Kept both"]).unwrap();
        assert_eq!(parsed.words, ["changeset", "resolve", "c"]);
        assert_eq!(parsed.note.as_deref(), Some("Kept both"));
        assert_eq!(
            parse_strs(&["changeset", "resolve", "--note=N", "c"])
                .unwrap()
                .note
                .as_deref(),
            Some("N")
        );
    }

    #[test]
    fn doc_options_take_values_and_doc_read_takes_a_version() {
        let parsed = parse_strs(&[
            "doc",
            "create",
            "--changeset",
            "c",
            "--kind=html",
            "--pillar",
            "spec",
        ])
        .unwrap();
        assert_eq!(parsed.words, ["doc", "create"]);
        assert_eq!(parsed.changeset.as_deref(), Some("c"));
        assert_eq!(parsed.kind.as_deref(), Some("html"));
        assert_eq!(parsed.pillar.as_deref(), Some("spec"));

        let parsed = parse_strs(&["doc", "read", "d", "--version", "v"]).unwrap();
        assert_eq!(parsed.words, ["doc", "read", "d"]);
        assert_eq!(parsed.doc_version.as_deref(), Some("v"));
        assert!(!parsed.version);
        // Anywhere else it is still the flag that shows omni's version.
        assert!(parse_strs(&["--version"]).unwrap().version);
        assert!(
            parse_strs(&["doc", "show", "d", "--version"])
                .unwrap()
                .version
        );
    }

    #[test]
    fn post_options_take_values_and_attach_and_link_repeat() {
        let parsed = parse_strs(&[
            "post",
            "t",
            "--summary=S",
            "--attach",
            "a.png",
            "--link",
            "post:p",
            "--attach=b.txt",
            "--link",
            "https://example.com",
            "--confidence",
            "0.5",
            "--replaces",
            "r",
        ])
        .unwrap();
        assert_eq!(parsed.words, ["post", "t"]);
        assert_eq!(parsed.summary.as_deref(), Some("S"));
        assert_eq!(parsed.attach, ["a.png", "b.txt"]);
        assert_eq!(parsed.link, ["post:p", "https://example.com"]);
        assert_eq!(parsed.confidence.as_deref(), Some("0.5"));
        assert_eq!(parsed.replaces.as_deref(), Some("r"));
        assert!(parse_strs(&["post", "--attach"]).is_err());
        assert!(parse_strs(&["post", "t", "--type", "note"]).is_err());
    }

    #[test]
    fn removed_commands_accept_any_options_so_the_command_can_explain() {
        let parsed = parse_strs(&["decision", "propose", "t", "--file", "d.json"]).unwrap();
        assert_eq!(parsed.words, ["decision", "propose", "t"]);
        let parsed = parse_strs(&["thread", "phase", "t", "--open"]).unwrap();
        assert_eq!(parsed.words, ["thread", "phase", "t"]);
        assert!(parse_strs(&["thread", "read", "t", "--open"]).is_err());
    }

    #[test]
    fn bad_options_are_errors() {
        assert!(parse_strs(&["--bogus"]).is_err());
        assert!(parse_strs(&["whoami", "--as"]).is_err());
        assert!(parse_strs(&["-x"]).is_err());
    }
}
