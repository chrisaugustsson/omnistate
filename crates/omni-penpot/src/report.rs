//! Writes the page that shows the comparison of every board of every export in a
//! folder (docs/penpot-compare.md): one summary, then the boards from worst to
//! best, each with Penpot's PNG, OmniState's render and the diff side by side.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::compare::{BoardComparison, TOLERANCE, compare_file};

/// What a written report holds, for the command to print.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub files: usize,
    pub boards: usize,
}

/// One board of the page, with its images already written.
struct Entry {
    file: String,
    board: BoardComparison,
    /// The paths of Penpot's PNG, the render and the diff, relative to the
    /// report folder; None for a board with no PNG.
    images: Option<[String; 3]>,
}

impl Entry {
    /// The pixel score: the share of compared pixels that differ.
    fn share(&self) -> Option<f64> {
        self.board.pixel.as_ref().map(|pixel| pixel.share())
    }
}

/// Converts and compares every `*.penpot` file directly in `folder` and writes
/// `index.html` and its images below `out`.
pub fn write_report(folder: &Path, out: &Path) -> Result<Written, String> {
    let files = exports(folder)?;
    let images_dir = out.join("images");
    // The images of an earlier report would otherwise stay behind.
    if images_dir.is_dir() {
        fs::remove_dir_all(&images_dir)
            .map_err(|error| format!("cannot clear {}: {error}", images_dir.display()))?;
    }
    fs::create_dir_all(&images_dir)
        .map_err(|error| format!("cannot create {}: {error}", images_dir.display()))?;

    let mut used = HashSet::new();
    let mut entries = Vec::new();
    for path in &files {
        let file = file_name(path);
        let comparison = compare_file(path).map_err(|error| format!("{file}: {error}"))?;
        for mut board in comparison.boards {
            let mut images = None;
            if let Some(pixel) = &mut board.pixel {
                let base = unique_name(&mut used, &file, &board.board, &board.board_id);
                let mut paths = Vec::new();
                for (kind, bytes) in [
                    ("penpot", &mut pixel.penpot),
                    ("render", &mut pixel.render),
                    ("diff", &mut pixel.diff),
                ] {
                    let name = format!("images/{base}-{kind}.png");
                    put(&out.join(&name), bytes)?;
                    // The image is written; its bytes are not needed again.
                    *bytes = Vec::new();
                    paths.push(name);
                }
                images = paths.try_into().ok();
            }
            entries.push(Entry {
                file: file.clone(),
                board,
                images,
            });
        }
    }
    // The sort is stable, so ties stay in the order of the files and the boards.
    entries.sort_by(|a, b| match (a.share(), b.share()) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b
            .board
            .layout
            .mismatches
            .len()
            .cmp(&a.board.layout.mismatches.len()),
    });
    put(&out.join("index.html"), page(&entries).as_bytes())?;
    Ok(Written {
        files: files.len(),
        boards: entries.len(),
    })
}

/// The `*.penpot` files directly in `folder`, by name.
fn exports(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let listing = fs::read_dir(folder)
        .map_err(|error| format!("cannot read the folder {}: {error}", folder.display()))?;
    let mut files: Vec<PathBuf> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "penpot"))
        .collect();
    if files.is_empty() {
        return Err(format!(
            "there is no .penpot file in the folder {}",
            folder.display()
        ));
    }
    files.sort();
    Ok(files)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().to_string())
}

fn put(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// A file name stem for the images of a board that no other board has: the
/// export's and the board's names with anything but letters and digits turned
/// into `-`, then the board's Penpot id. Only lower-case letters, digits and
/// `-`, so that the path is safe in a `src` on any file system.
fn unique_name(used: &mut HashSet<String>, file: &str, board: &str, id: &str) -> String {
    let slug = |text: &str, limit: usize| -> String {
        let mut slug = String::new();
        for c in text.chars().take(limit) {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.ends_with('-') {
                slug.push('-');
            }
        }
        slug.trim_matches('-').to_string()
    };
    let file = slug(file.strip_suffix(".penpot").unwrap_or(file), 30);
    let base = [file, slug(board, 40), slug(id, 40)]
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("-");
    let mut candidate = base.clone();
    let mut count = 1;
    while !used.insert(candidate.clone()) {
        count += 1;
        candidate = format!("{base}-{count}");
    }
    candidate
}

/// The median of `values`, which must be sorted: the mean of the middle two
/// where there is an even number.
fn median(values: &[f64]) -> Option<f64> {
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.0),
    }
}

fn percent(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

/// `text` as it is written in HTML text or in a quoted attribute.
fn esc(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

fn page(entries: &[Entry]) -> String {
    let mut shares: Vec<f64> = entries.iter().filter_map(Entry::share).collect();
    shares.sort_by(f64::total_cmp);
    let median = median(&shares);
    let mismatches: usize = entries
        .iter()
        .map(|entry| entry.board.layout.mismatches.len())
        .sum();
    let unconverted: usize = entries
        .iter()
        .map(|entry| entry.board.unconverted.len())
        .sum();

    let mut html = String::from(HEAD);
    html.push_str("<header>\n<h1>OmniState against Penpot</h1>\n");
    let _ = writeln!(
        html,
        "<p class=\"lede\">Each board is drawn by OmniState at 2x and set beside Penpot's own PNG. \
The difference image fades Penpot's PNG and paints every pixel that differs by more than {TOLERANCE} \
in a channel magenta. Boards are sorted worst first: by the share of pixels that differ and, \
for boards without a PNG, by the number of layout mismatches. Text differs a little everywhere, \
because Penpot draws text in the browser; look for differences in solid areas, edges, shadows \
and colors.</p>"
    );
    let _ = writeln!(
        html,
        "<dl id=\"summary\" data-boards=\"{}\" data-median-share=\"{}\" data-layout-mismatches=\"{mismatches}\" data-unconverted=\"{unconverted}\">",
        entries.len(),
        median.map_or_else(String::new, |median| median.to_string()),
    );
    let _ = writeln!(html, "<div><dt>Boards</dt><dd>{}</dd></div>", entries.len());
    let _ = writeln!(
        html,
        "<div><dt>Median pixel score</dt><dd>{}</dd></div>",
        median.map_or_else(|| "none".to_string(), percent)
    );
    let _ = writeln!(
        html,
        "<div><dt>Layout mismatches</dt><dd>{mismatches}</dd></div>"
    );
    let _ = writeln!(
        html,
        "<div><dt>Unconverted shapes</dt><dd>{unconverted}</dd></div>"
    );
    html.push_str("</dl>\n</header>\n<main>\n");
    for entry in entries {
        board(&mut html, entry);
    }
    html.push_str("</main>\n</body>\n</html>\n");
    html
}

fn board(html: &mut String, entry: &Entry) {
    let board = &entry.board;
    let share = entry.share();
    let _ = writeln!(
        html,
        "<article class=\"board\" data-file=\"{}\" data-board-id=\"{}\" data-board=\"{}\" data-pixel-share=\"{}\" data-layout-mismatches=\"{}\" data-unconverted=\"{}\">",
        esc(&entry.file),
        esc(&board.board_id),
        esc(&board.board),
        share.map_or_else(String::new, |share| share.to_string()),
        board.layout.mismatches.len(),
        board.unconverted.len()
    );
    let _ = writeln!(
        html,
        "<h2>{}</h2>\n<p class=\"meta\">{} · page {} · pixel score <strong>{}</strong> · {} layout mismatches of {} shapes · {} unconverted shapes</p>",
        esc(&board.board),
        esc(&entry.file),
        esc(&board.page),
        share.map_or_else(|| "none".to_string(), percent),
        board.layout.mismatches.len(),
        board.layout.compared,
        board.unconverted.len()
    );
    match (&entry.images, &board.pixel) {
        (Some(paths), Some(pixel)) => {
            html.push_str("<div class=\"images\">\n");
            for (path, role, label, alt) in [
                (&paths[0], "penpot", "Penpot", "Penpot's PNG"),
                (&paths[1], "omnistate", "OmniState", "OmniState's rendering"),
                (
                    &paths[2],
                    "diff",
                    "Difference",
                    "The difference between the two renderings",
                ),
            ] {
                let _ = writeln!(
                    html,
                    "<figure><a href=\"{path}\"><img data-role=\"{role}\" src=\"{path}\" width=\"{}\" height=\"{}\" loading=\"lazy\" alt=\"{alt} of the board {}\"></a><figcaption>{label}</figcaption></figure>",
                    pixel.width,
                    pixel.height,
                    esc(&board.board)
                );
            }
            html.push_str("</div>\n");
            if pixel.masked > 0 {
                let _ = writeln!(
                    html,
                    "<p class=\"note\">{} pixels under unconverted shapes are left out of the score and shown flat light blue in the difference.</p>",
                    pixel.masked
                );
            }
        }
        _ => {
            let note = board.note.as_deref().unwrap_or(
                "Layout comparison only: there is no Penpot PNG, so the pixels are not compared.",
            );
            let _ = writeln!(html, "<p class=\"note\">{}</p>", esc(note));
        }
    }

    let count = board.layout.mismatches.len();
    let _ = writeln!(
        html,
        "<details{}>\n<summary>Layout mismatches ({count})</summary>",
        open_if_short(count)
    );
    if count == 0 {
        html.push_str("<p class=\"none\">Every shape is within 2 px of where Penpot put it.</p>\n");
    } else {
        html.push_str("<ul>\n");
        for mismatch in &board.layout.mismatches {
            let (p, o) = (&mismatch.penpot, &mismatch.omnistate);
            let _ = writeln!(
                html,
                "<li data-role=\"mismatch\" data-shape-id=\"{}\" data-off-by=\"{}\"><span class=\"name\">{}</span> <span class=\"boxes\">Penpot {:.1}, {:.1}, {:.1} × {:.1}; OmniState {:.1}, {:.1}, {:.1} × {:.1}</span> <span class=\"off\">off by {:.1} px</span></li>",
                esc(&mismatch.shape_id),
                mismatch.off_by,
                esc(&mismatch.name),
                p.x,
                p.y,
                p.width,
                p.height,
                o.x,
                o.y,
                o.width,
                o.height,
                mismatch.off_by
            );
        }
        html.push_str("</ul>\n");
    }
    html.push_str("</details>\n");

    let count = board.unconverted.len();
    let _ = writeln!(
        html,
        "<details{}>\n<summary>Unconverted shapes ({count})</summary>",
        open_if_short(count)
    );
    if count == 0 {
        html.push_str("<p class=\"none\">Every shape was converted.</p>\n");
    } else {
        html.push_str("<ul>\n");
        for shape in &board.unconverted {
            let _ = writeln!(
                html,
                "<li data-role=\"unconverted\" data-shape-id=\"{}\"><span class=\"name\">{}</span> <span class=\"reason\">{}</span></li>",
                esc(&shape.shape_id),
                esc(&shape.name),
                esc(&shape.reason)
            );
        }
        html.push_str("</ul>\n");
    }
    html.push_str("</details>\n</article>\n");
}

/// A list of 1 to 20 items is open on the page; longer lists are folded.
fn open_if_short(count: usize) -> &'static str {
    if (1..=20).contains(&count) {
        " open"
    } else {
        ""
    }
}

const HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>OmniState against Penpot</title>
<style>
:root {
  --bg: #fafaf8; --panel: #ffffff; --text: #1d1d1b; --muted: #5d5d58;
  --line: #deded8; --accent: #b0007a; --soft: #f0f0ea;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #161615; --panel: #1f1f1d; --text: #ecece6; --muted: #a5a59c;
    --line: #35352f; --accent: #ff7ad1; --soft: #292926;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; padding: 24px 16px 64px; background: var(--bg); color: var(--text);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif;
}
header, main { max-width: 1600px; margin: 0 auto; }
h1 { margin: 0 0 8px; font-size: 1.75rem; }
h2 { margin: 0; font-size: 1.25rem; overflow-wrap: anywhere; }
.lede { max-width: 70ch; margin: 0 0 20px; color: var(--muted); }
#summary { display: flex; flex-wrap: wrap; gap: 12px; margin: 0 0 32px; }
#summary div {
  flex: 1 1 160px; padding: 14px 16px; background: var(--panel);
  border: 1px solid var(--line); border-radius: 8px;
}
#summary dt { color: var(--muted); font-size: 0.875rem; }
#summary dd { margin: 0; font-size: 2rem; font-weight: 650; font-variant-numeric: tabular-nums; }
.board {
  margin: 0 0 24px; padding: 16px; background: var(--panel);
  border: 1px solid var(--line); border-radius: 8px;
}
.meta, .note { margin: 4px 0 12px; color: var(--muted); font-size: 0.9375rem; }
.meta strong { color: var(--accent); font-size: 1.0625rem; }
.images { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
figure { margin: 0; min-width: 0; }
figure img {
  display: block; width: 100%; height: auto; background: var(--soft);
  border: 1px solid var(--line);
}
figcaption { margin-top: 4px; color: var(--muted); font-size: 0.875rem; text-align: center; }
@media (max-width: 720px) { .images { grid-template-columns: 1fr; } }
details { margin-top: 12px; }
summary { cursor: pointer; font-weight: 600; }
ul { margin: 8px 0 0; padding: 0; list-style: none; }
li {
  display: flex; flex-wrap: wrap; gap: 4px 12px; padding: 6px 0;
  border-top: 1px solid var(--line); font-size: 0.9375rem;
}
li .name { font-weight: 600; overflow-wrap: anywhere; }
li .boxes, li .reason { color: var(--muted); overflow-wrap: anywhere; }
li .off { margin-left: auto; color: var(--accent); font-variant-numeric: tabular-nums; }
.none { margin: 8px 0 0; color: var(--muted); }
</style>
</head>
<body>
"#;
