//! The `omni-penpot` command line: `omni-penpot convert <file.penpot> --out <dir>`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use omni_penpot::{Conversion, convert_file};
use serde_json::{Value, json};

const USAGE: &str = "usage: omni-penpot convert <file.penpot> --out <dir>";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("omni-penpot: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    if command != "convert" {
        return Err(format!("unknown command `{command}`\n{USAGE}"));
    }
    let mut file = None;
    let mut out = None;
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        if arg == "--out" {
            let dir = rest
                .next()
                .ok_or_else(|| format!("--out needs a directory\n{USAGE}"))?;
            out = Some(PathBuf::from(dir));
        } else if arg.starts_with("--") {
            return Err(format!("unknown option `{arg}`\n{USAGE}"));
        } else if file.replace(PathBuf::from(arg)).is_some() {
            return Err(format!("only one file can be converted at a time\n{USAGE}"));
        }
    }
    let file = file.ok_or_else(|| format!("no file to convert\n{USAGE}"))?;
    let out = out.ok_or_else(|| format!("missing --out <dir>\n{USAGE}"))?;
    let conversion = convert_file(&file).map_err(|error| error.to_string())?;
    write(&conversion, &out)?;
    println!(
        "{} boards converted, {} report entries, written to {}",
        conversion.boards.len(),
        conversion.report.entries.len(),
        out.display()
    );
    Ok(())
}

/// Writes the designs, the report and the images below `out`.
fn write(conversion: &Conversion, out: &Path) -> Result<(), String> {
    let designs = out.join("designs");
    let attachments = out.join("attachments");
    for dir in [&designs, &attachments] {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    let mut used = HashSet::new();
    for board in &conversion.boards {
        let name = unique_name(&mut used, &board.name, &board.id);
        let text = serde_json::to_string(&board.design).map_err(|error| error.to_string())?;
        put(&designs.join(format!("{name}.json")), text.as_bytes())?;
    }
    for attachment in &conversion.attachments {
        let name = format!("{}.{}", attachment.id, attachment.extension);
        put(&attachments.join(name), &attachment.bytes)?;
    }
    let entries: Vec<Value> = conversion
        .report
        .entries
        .iter()
        .map(|entry| {
            let mut value = json!({
                "board": entry.board,
                "boardId": entry.board_id,
                "shapeId": entry.shape_id,
                "name": entry.name,
                "reason": entry.reason,
            });
            if let Some(property) = &entry.property {
                value["property"] = json!(property);
            }
            value
        })
        .collect();
    let mut report =
        serde_json::to_string_pretty(&json!({ "entries": entries })).map_err(|e| e.to_string())?;
    report.push('\n');
    put(&out.join("report.json"), report.as_bytes())
}

fn put(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// A file name for a board that no other board has: its name with anything
/// but letters, digits, `.` and `-` turned into `-`, then its Penpot id.
fn unique_name(used: &mut HashSet<String>, name: &str, id: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let base = if slug.is_empty() {
        id.to_string()
    } else {
        format!("{slug}-{id}")
    };
    let mut candidate = base.clone();
    let mut count = 1;
    while !used.insert(candidate.clone()) {
        count += 1;
        candidate = format!("{base}-{count}");
    }
    candidate
}
