//! The `omni-penpot` command line: `omni-penpot convert <file.penpot> --out <dir>`,
//! `omni-penpot compare <file.penpot> --out <dir>` and
//! `omni-penpot report <folder> --out <dir>`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use omni_penpot::compare::{BoardComparison, Comparison, TOLERANCE, compare_file};
use omni_penpot::report::write_report;
use omni_penpot::{Conversion, convert_file};
use serde_json::{Value, json};

const USAGE: &str = "usage: omni-penpot convert <file.penpot> --out <dir>
       omni-penpot compare <file.penpot> --out <dir>
       omni-penpot report <folder> --out <dir>";

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
    let (file, out) = match command.as_str() {
        "convert" | "compare" | "report" => file_and_out(command, rest)?,
        _ => return Err(format!("unknown command `{command}`\n{USAGE}")),
    };
    if command == "report" {
        let written = write_report(&file, &out)?;
        println!(
            "{} boards of {} files compared, report written to {}",
            written.boards,
            written.files,
            out.join("index.html").display()
        );
        return Ok(());
    }
    if command == "compare" {
        let comparison = compare_file(&file).map_err(|error| error.to_string())?;
        write_comparison(&comparison, &out)?;
        println!(
            "{} boards compared, written to {}",
            comparison.boards.len(),
            out.display()
        );
        return Ok(());
    }
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

/// The file (the folder, for `report`) and the `--out` directory of a command's
/// arguments.
fn file_and_out(command: &str, args: &[String]) -> Result<(PathBuf, PathBuf), String> {
    let mut file = None;
    let mut out = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--out" {
            let dir = rest
                .next()
                .ok_or_else(|| format!("--out needs a directory\n{USAGE}"))?;
            out = Some(PathBuf::from(dir));
        } else if arg.starts_with("--") {
            return Err(format!("unknown option `{arg}`\n{USAGE}"));
        } else if file.replace(PathBuf::from(arg)).is_some() {
            return Err(format!(
                "only one input can be used at a time for `{command}`\n{USAGE}"
            ));
        }
    }
    let what = if command == "report" {
        "folder"
    } else {
        "file"
    };
    let file = file.ok_or_else(|| format!("no {what} for `{command}`\n{USAGE}"))?;
    let out = out.ok_or_else(|| format!("missing --out <dir>\n{USAGE}"))?;
    Ok((file, out))
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

/// Writes `compare.json` and the images it names below `out`.
fn write_comparison(comparison: &Comparison, out: &Path) -> Result<(), String> {
    let images = out.join("images");
    fs::create_dir_all(&images)
        .map_err(|error| format!("cannot create {}: {error}", images.display()))?;
    let mut used = HashSet::new();
    let mut boards = Vec::with_capacity(comparison.boards.len());
    for board in &comparison.boards {
        let name = unique_name(&mut used, &board.board, &board.board_id);
        boards.push(board_json(board, &name, &images)?);
    }
    let mut text = serde_json::to_string_pretty(&json!({
        "tolerance": TOLERANCE,
        "boards": boards,
    }))
    .map_err(|error| error.to_string())?;
    text.push('\n');
    put(&out.join("compare.json"), text.as_bytes())
}

/// The entry of a board in `compare.json`, after writing its images to `images`
/// as `<name>-render.png` and `<name>-diff.png`.
fn board_json(board: &BoardComparison, name: &str, images: &Path) -> Result<Value, String> {
    let pixel = match &board.pixel {
        None => Value::Null,
        Some(pixel) => {
            let mut paths = Vec::new();
            for (kind, bytes) in [("render", &pixel.render), ("diff", &pixel.diff)] {
                let file = format!("{name}-{kind}.png");
                put(&images.join(&file), bytes)?;
                paths.push(format!("images/{file}"));
            }
            json!({
                "width": pixel.width,
                "height": pixel.height,
                "compared": pixel.compared,
                "masked": pixel.masked,
                "differing": pixel.differing,
                "share": pixel.share(),
                "render": paths[0],
                "diff": paths[1],
            })
        }
    };
    let rect = |rect: &omni_penpot::Rect| json!({ "x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height });
    let mismatches: Vec<Value> = board
        .layout
        .mismatches
        .iter()
        .map(|mismatch| {
            json!({
                "shapeId": mismatch.shape_id,
                "name": mismatch.name,
                "penpot": rect(&mismatch.penpot),
                "omnistate": rect(&mismatch.omnistate),
                "offBy": mismatch.off_by,
            })
        })
        .collect();
    let unconverted: Vec<Value> = board
        .unconverted
        .iter()
        .map(|shape| {
            json!({ "shapeId": shape.shape_id, "name": shape.name, "reason": shape.reason })
        })
        .collect();
    Ok(json!({
        "board": board.board,
        "boardId": board.board_id,
        "page": board.page,
        "layoutOnly": board.pixel.is_none(),
        "note": board.note,
        "pixel": pixel,
        "layout": { "compared": board.layout.compared, "mismatches": mismatches },
        "unconverted": unconverted,
    }))
}
