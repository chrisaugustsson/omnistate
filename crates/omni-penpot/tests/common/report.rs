//! A small reader for the HTML of the US-104 comparison report, so the tests can
//! ask what the page shows without a browser or an HTML library.
//!
//! The report marks what it shows with attributes, which is the contract these
//! tests hold it to (see report_synthetic.rs for the whole of it):
//!
//! - one element with `id="summary"`, before the first board;
//! - one element per board with `data-board-id` (no other element has it),
//!   everything up to the next board element belonging to that board;
//! - inside a board, elements with a `data-role`.

use std::fs;
use std::path::{Path, PathBuf};

use super::compare::Pixels;

#[derive(Debug)]
pub struct Tag {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub closing: bool,
    pub self_closing: bool,
    pub start: usize,
    pub end: usize,
}

impl Tag {
    /// The value of the attribute `key`, with the character references undone.
    pub fn attr(&self, key: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| unescape(value))
    }

    pub fn number(&self, key: &str) -> f64 {
        let value = self
            .attr(key)
            .unwrap_or_else(|| panic!("<{}> has no {key}: {self:?}", self.name));
        value
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{key}={value:?} of <{}> is not a number", self.name))
    }

    pub fn is_void(&self) -> bool {
        self.self_closing
            || matches!(
                self.name.as_str(),
                "img" | "br" | "hr" | "input" | "meta" | "link" | "source"
            )
    }
}

/// Undoes the character references an HTML writer uses.
pub fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Reads the tag that starts at `html[at]` (a `<`), or None where that is not a
/// tag. Comments and the doctype come back as a tag with an empty name.
fn read_tag(html: &str, at: usize) -> Option<Tag> {
    let bytes = html.as_bytes();
    let mut i = at + 1;
    if html[at..].starts_with("<!--") {
        let end = html[at..].find("-->").map_or(html.len(), |n| at + n + 3);
        return Some(empty_tag(at, end));
    }
    if bytes.get(i) == Some(&b'!') || bytes.get(i) == Some(&b'?') {
        let end = html[at..].find('>').map_or(html.len(), |n| at + n + 1);
        return Some(empty_tag(at, end));
    }
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        i += 1;
    }
    if i == name_start || !bytes[name_start].is_ascii_alphabetic() {
        return None;
    }
    let name = html[name_start..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => break,
            Some(b'>') => {
                i += 1;
                break;
            }
            Some(b'/') => {
                self_closing = true;
                i += 1;
            }
            Some(_) => {
                let key_start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                let key = html[key_start..i].to_ascii_lowercase();
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                let mut value = String::new();
                if bytes.get(i) == Some(&b'=') {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    match bytes.get(i) {
                        Some(&quote) if quote == b'"' || quote == b'\'' => {
                            let value_start = i + 1;
                            let close = html[value_start..]
                                .find(quote as char)
                                .map_or(html.len(), |n| value_start + n);
                            value = html[value_start..close].to_string();
                            i = (close + 1).min(html.len());
                        }
                        _ => {
                            let value_start = i;
                            while i < bytes.len()
                                && !bytes[i].is_ascii_whitespace()
                                && bytes[i] != b'>'
                            {
                                i += 1;
                            }
                            value = html[value_start..i].to_string();
                        }
                    }
                }
                self_closing = false;
                attrs.push((key, value));
            }
        }
    }
    Some(Tag {
        name,
        attrs,
        closing,
        self_closing,
        start: at,
        end: i,
    })
}

fn empty_tag(start: usize, end: usize) -> Tag {
    Tag {
        name: String::new(),
        attrs: Vec::new(),
        closing: false,
        self_closing: true,
        start,
        end,
    }
}

/// Every tag of `html`, in order, without the contents of scripts and styles.
fn scan(html: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    let mut at = 0;
    while let Some(found) = html[at..].find('<') {
        let start = at + found;
        let Some(tag) = read_tag(html, start) else {
            at = start + 1;
            continue;
        };
        at = tag.end.max(start + 1);
        let raw_text = !tag.closing && matches!(tag.name.as_str(), "script" | "style");
        let name = tag.name.clone();
        if !tag.name.is_empty() {
            tags.push(tag);
        }
        if raw_text {
            let close = format!("</{name}");
            at = html[at..]
                .to_ascii_lowercase()
                .find(&close)
                .map_or(html.len(), |n| at + n);
        }
    }
    tags
}

/// The text of `html[from..to]`: tags dropped, character references undone,
/// white space collapsed.
fn text_between(html: &str, from: usize, to: usize) -> String {
    let mut text = String::new();
    let mut at = from;
    while at < to {
        let Some(found) = html[at..to].find('<') else {
            text.push_str(&html[at..to]);
            break;
        };
        let start = at + found;
        text.push_str(&html[at..start]);
        match read_tag(html, start) {
            Some(tag) => {
                text.push(' ');
                at = tag.end.max(start + 1);
            }
            None => {
                text.push('<');
                at = start + 1;
            }
        }
    }
    unescape(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct Report {
    pub html: String,
    tags: Vec<Tag>,
}

/// A board of the report and everything the page shows for it.
pub struct BoardView<'a> {
    report: &'a Report,
    index: usize,
    end: usize,
}

impl Report {
    pub fn read(out: &Path) -> Report {
        let path = out.join("index.html");
        let html = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{path:?} is not written: {error}"));
        Report::parse(html)
    }

    pub fn parse(html: String) -> Report {
        let tags = scan(&html);
        Report { html, tags }
    }

    /// The boards in the order the page shows them.
    pub fn boards(&self) -> Vec<BoardView<'_>> {
        let starts: Vec<usize> = (0..self.tags.len())
            .filter(|&i| !self.tags[i].closing && self.tags[i].attr("data-board-id").is_some())
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, &index)| BoardView {
                report: self,
                index,
                end: starts.get(n + 1).copied().unwrap_or(self.tags.len()),
            })
            .collect()
    }

    pub fn board(&self, board_id: &str) -> BoardView<'_> {
        self.boards()
            .into_iter()
            .find(|board| board.tag().attr("data-board-id").as_deref() == Some(board_id))
            .unwrap_or_else(|| panic!("the page has no board {board_id}"))
    }

    /// The summary: the element with `id="summary"`, which must come before
    /// the first board, and the text up to the first board.
    pub fn summary(&self) -> (&Tag, String) {
        let tag = self
            .tags
            .iter()
            .find(|tag| !tag.closing && tag.attr("id").as_deref() == Some("summary"))
            .expect("the page has an element with id=\"summary\"");
        let first_board = self
            .boards()
            .first()
            .map_or(self.html.len(), |board| board.tag().start);
        assert!(
            tag.start < first_board,
            "the summary comes before the first board"
        );
        (tag, text_between(&self.html, tag.start, first_board))
    }
}

impl<'a> BoardView<'a> {
    pub fn tag(&self) -> &'a Tag {
        &self.report.tags[self.index]
    }

    pub fn attr(&self, key: &str) -> Option<String> {
        self.tag().attr(key)
    }

    pub fn id(&self) -> String {
        self.attr("data-board-id").expect("a board id")
    }

    /// Everything the page says about the board, as text.
    pub fn text(&self) -> String {
        let to = self
            .report
            .tags
            .get(self.end)
            .map_or(self.report.html.len(), |tag| tag.start);
        text_between(&self.report.html, self.tag().start, to)
    }

    fn inside(&self) -> impl Iterator<Item = (usize, &'a Tag)> + '_ {
        (self.index + 1..self.end).map(|i| (i, &self.report.tags[i]))
    }

    /// The `<img>` elements of the board, with their `data-role`s.
    pub fn images(&self) -> Vec<&'a Tag> {
        self.inside()
            .map(|(_, tag)| tag)
            .filter(|tag| tag.name == "img")
            .collect()
    }

    pub fn image(&self, role: &str) -> Option<&'a Tag> {
        self.images()
            .into_iter()
            .find(|tag| tag.attr("data-role").as_deref() == Some(role))
    }

    /// The elements of the board with `data-role` `role`, with their text.
    pub fn items(&self, role: &str) -> Vec<(&'a Tag, String)> {
        self.inside()
            .filter(|(_, tag)| !tag.closing && tag.attr("data-role").as_deref() == Some(role))
            .map(|(i, tag)| (tag, self.element_text(i)))
            .collect()
    }

    fn element_text(&self, i: usize) -> String {
        let tags = &self.report.tags;
        let open = &tags[i];
        if open.is_void() {
            return String::new();
        }
        let mut depth = 1;
        for tag in &tags[i + 1..] {
            if tag.name != open.name {
                continue;
            }
            if tag.closing {
                depth -= 1;
            } else if !tag.is_void() {
                depth += 1;
            }
            if depth == 0 {
                return text_between(&self.report.html, open.end, tag.start);
            }
        }
        text_between(&self.report.html, open.end, self.report.html.len())
    }
}

/// The file an `<img>` of the page shows, below `out`. The page must stay
/// usable when `out` is moved or opened from disk, so the path is relative and
/// stays inside `out`, and the file must be there.
pub fn image_file(out: &Path, image: &Tag) -> PathBuf {
    let src = image.attr("src").expect("an <img> has a src");
    assert!(
        !src.is_empty()
            && !src.starts_with('/')
            && !src.contains("..")
            && !src.contains(':')
            && !src.contains('\\'),
        "the image path {src:?} is relative and inside the report folder"
    );
    assert!(
        image.attr("alt").is_some_and(|alt| !alt.trim().is_empty()),
        "the image {src} has alt text"
    );
    let path = out.join(&src);
    assert!(path.is_file(), "{path:?} (src {src:?}) is not written");
    path
}

pub fn image_pixels(out: &Path, image: &Tag) -> Pixels {
    let bytes = fs::read(image_file(out, image)).expect("a readable image");
    Pixels::from_png(&bytes)
}

/// The numbers written in `text`, as the digit runs of whole numbers.
pub fn number_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}
