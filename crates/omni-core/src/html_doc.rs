//! HTML docs: PRDs, architecture and database docs as a tree of typed blocks
//! with stable node IDs, in a strict HTML form (docs/formats/html-docs.md).
//!
//! [`HtmlDoc::parse`] reads that HTML form, rejecting anything outside it with
//! the line and the problem, and cleans what it keeps: inline SVG is reduced to
//! an allow-list, links must use http, https or mailto, and images must be
//! attachments. A doc is stored as its block tree in JSON, never as raw HTML,
//! and [`HtmlDoc::to_html`] prints the tree back with a `data-node-id` on every
//! block.

use crate::design::{AttachmentRef, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// How deeply blocks and inline markup may nest, so that hostile input cannot
/// overflow the stack.
const MAX_DEPTH: usize = 64;

/// The type of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Section,
    Heading,
    Paragraph,
    Quote,
    Divider,
    List,
    ListItem,
    Table,
    TableRow,
    TableCell,
    Figure,
    Callout,
    CodeBlock,
    Image,
    Requirement,
    UserStory,
}

/// What a callout says: a note, a warning or a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalloutKind {
    Note,
    Warning,
    Decision,
}

impl CalloutKind {
    fn as_str(self) -> &'static str {
        match self {
            CalloutKind::Note => "note",
            CalloutKind::Warning => "warning",
            CalloutKind::Decision => "decision",
        }
    }
}

/// Inline markup inside a text block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Code(String),
    Link { href: String, children: Vec<Inline> },
    LineBreak,
}

/// What a figure shows: cleaned inline SVG markup, or Mermaid code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FigureSource {
    Svg(String),
    Mermaid(String),
}

/// Why HTML is not a valid HTML doc: the line it is on and the problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
    message: String,
}

impl ParseError {
    /// The line, counting from 1, of the tag with the problem.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The problem, such as "`<span>` is not allowed".
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

type Result<T> = std::result::Result<T, ParseError>;

fn error(line: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        line,
        message: message.into(),
    }
}

/// What a block is and holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Body {
    Section {
        children: Vec<Block>,
    },
    Heading {
        level: u8,
        inline: Vec<Inline>,
    },
    Paragraph {
        inline: Vec<Inline>,
    },
    Quote {
        inline: Vec<Inline>,
    },
    Divider,
    List {
        ordered: bool,
        children: Vec<Block>,
    },
    ListItem {
        inline: Vec<Inline>,
    },
    Table {
        children: Vec<Block>,
    },
    TableRow {
        children: Vec<Block>,
    },
    TableCell {
        header: bool,
        inline: Vec<Inline>,
    },
    Figure {
        source: FigureSource,
        caption: String,
    },
    Callout {
        kind: CalloutKind,
        inline: Vec<Inline>,
    },
    CodeBlock {
        code: String,
    },
    Image {
        attachment: AttachmentRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt: Option<String>,
    },
    Requirement {
        inline: Vec<Inline>,
    },
    UserStory {
        children: Vec<Block>,
    },
}

/// One block of a doc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<NodeId>,
    #[serde(flatten)]
    body: Body,
}

impl Block {
    /// The block's type.
    pub fn kind(&self) -> BlockKind {
        match &self.body {
            Body::Section { .. } => BlockKind::Section,
            Body::Heading { .. } => BlockKind::Heading,
            Body::Paragraph { .. } => BlockKind::Paragraph,
            Body::Quote { .. } => BlockKind::Quote,
            Body::Divider => BlockKind::Divider,
            Body::List { .. } => BlockKind::List,
            Body::ListItem { .. } => BlockKind::ListItem,
            Body::Table { .. } => BlockKind::Table,
            Body::TableRow { .. } => BlockKind::TableRow,
            Body::TableCell { .. } => BlockKind::TableCell,
            Body::Figure { .. } => BlockKind::Figure,
            Body::Callout { .. } => BlockKind::Callout,
            Body::CodeBlock { .. } => BlockKind::CodeBlock,
            Body::Image { .. } => BlockKind::Image,
            Body::Requirement { .. } => BlockKind::Requirement,
            Body::UserStory { .. } => BlockKind::UserStory,
        }
    }

    /// The block's `data-node-id`, or `None` for a block that is new.
    pub fn id(&self) -> Option<&NodeId> {
        self.id.as_ref()
    }

    /// Whether the block has no ID yet: the server assigns one when it is saved.
    pub fn is_new(&self) -> bool {
        self.id.is_none()
    }

    /// The blocks inside this one: the blocks of a section or user story, the
    /// items of a list, the rows of a table or the cells of a row.
    pub fn children(&self) -> &[Block] {
        match &self.body {
            Body::Section { children }
            | Body::List { children, .. }
            | Body::Table { children }
            | Body::TableRow { children }
            | Body::UserStory { children } => children,
            _ => &[],
        }
    }

    /// The inline content of a heading, paragraph, quote, list item, table
    /// cell, callout or requirement. Empty for the other blocks.
    pub fn inline(&self) -> &[Inline] {
        match &self.body {
            Body::Heading { inline, .. }
            | Body::Paragraph { inline }
            | Body::Quote { inline }
            | Body::ListItem { inline }
            | Body::TableCell { inline, .. }
            | Body::Callout { inline, .. }
            | Body::Requirement { inline } => inline,
            _ => &[],
        }
    }

    /// The level, 1 to 6, of a heading.
    pub fn heading_level(&self) -> Option<u8> {
        match &self.body {
            Body::Heading { level, .. } => Some(*level),
            _ => None,
        }
    }

    /// Whether a list is numbered (`<ol>`) rather than bulleted.
    pub fn is_ordered(&self) -> bool {
        matches!(&self.body, Body::List { ordered: true, .. })
    }

    /// Whether a table cell is a header cell (`<th>`).
    pub fn is_header(&self) -> bool {
        matches!(&self.body, Body::TableCell { header: true, .. })
    }

    /// The kind of a callout.
    pub fn callout_kind(&self) -> Option<CalloutKind> {
        match &self.body {
            Body::Callout { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    /// The text of a code block, exactly as written.
    pub fn code(&self) -> Option<&str> {
        match &self.body {
            Body::CodeBlock { code } => Some(code),
            _ => None,
        }
    }

    /// What a figure shows.
    pub fn figure(&self) -> Option<&FigureSource> {
        match &self.body {
            Body::Figure { source, .. } => Some(source),
            _ => None,
        }
    }

    /// A figure's caption, as text.
    pub fn caption(&self) -> Option<&str> {
        match &self.body {
            Body::Figure { caption, .. } => Some(caption),
            _ => None,
        }
    }

    /// The attachment an image shows.
    pub fn attachment(&self) -> Option<&AttachmentRef> {
        match &self.body {
            Body::Image { attachment, .. } => Some(attachment),
            _ => None,
        }
    }

    /// An image's alternative text.
    pub fn alt(&self) -> Option<&str> {
        match &self.body {
            Body::Image { alt, .. } => alt.as_deref(),
            _ => None,
        }
    }
}

/// A doc: its blocks, in order. Stored as JSON, never as raw HTML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HtmlDoc {
    blocks: Vec<Block>,
}

impl HtmlDoc {
    /// Parses and cleans HTML. Anything outside the format is an error that
    /// names the line and the problem.
    pub fn parse(html: &str) -> std::result::Result<HtmlDoc, ParseError> {
        let mut parser = Parser {
            tokens: lex(html).into_iter(),
            last_line: 1,
            ids: HashSet::new(),
            depth: 0,
        };
        let blocks = parser.blocks(None)?;
        Ok(HtmlDoc { blocks })
    }

    /// The doc's top-level blocks.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The doc as HTML, with `data-node-id` on every block.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        print_blocks(&self.blocks, &mut out);
        out
    }
}

// --- Tokens ---

struct Attr {
    /// The name in lowercase.
    name: String,
    /// The name as written, for SVG attributes that are case-sensitive.
    raw_name: String,
    value: String,
}

struct StartTag {
    /// The name in lowercase.
    name: String,
    raw_name: String,
    attrs: Vec<Attr>,
    self_closing: bool,
    line: usize,
}

impl StartTag {
    fn display(&self) -> String {
        format!("`<{}>`", self.name)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.name == name)
            .map(|attr| attr.value.as_str())
    }

    /// Rejects the first attribute that is not in `allowed`.
    fn check_attrs(&self, allowed: &[&str]) -> Result<()> {
        for attr in &self.attrs {
            if allowed.contains(&attr.name.as_str()) {
                continue;
            }
            let name = &attr.name;
            let tag = self.display();
            let message = if name == "style" {
                format!(
                    "the `style` attribute is not allowed on {tag}: docs share one stylesheet and carry no inline styles"
                )
            } else if name.starts_with("on") {
                format!("the event handler attribute `{name}` is not allowed on {tag}")
            } else {
                format!("the attribute `{name}` is not allowed on {tag}")
            };
            return Err(error(self.line, message));
        }
        Ok(())
    }
}

enum Tok {
    Start(StartTag),
    End(String),
    Text(String),
    /// A comment, a doctype or a processing instruction.
    Skipped,
    Error(String),
    Eof,
}

struct Token {
    line: usize,
    tok: Tok,
}

// --- Lexer ---

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line_pos: usize,
    line: usize,
    /// After a `<script>` or `<style>` start tag, whose content is raw text.
    raw_until: Option<String>,
}

fn lex(src: &str) -> Vec<Token> {
    let mut lexer = Lexer {
        src,
        pos: 0,
        line_pos: 0,
        line: 1,
        raw_until: None,
    };
    let mut tokens = Vec::new();
    loop {
        match lexer.next() {
            Ok(Some(token)) => tokens.push(token),
            Ok(None) => {
                let line = lexer.line_at(src.len());
                tokens.push(Token {
                    line,
                    tok: Tok::Eof,
                });
                return tokens;
            }
            Err(token) => {
                tokens.push(token);
                return tokens;
            }
        }
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_')
}

impl Lexer<'_> {
    /// The line, counting from 1, of byte `pos`.
    fn line_at(&mut self, pos: usize) -> usize {
        if pos < self.line_pos {
            self.line_pos = 0;
            self.line = 1;
        }
        let newlines = self.src.as_bytes()[self.line_pos..pos]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count();
        self.line += newlines;
        self.line_pos = pos;
        self.line
    }

    fn fail(&mut self, pos: usize, message: impl Into<String>) -> Token {
        Token {
            line: self.line_at(pos),
            tok: Tok::Error(message.into()),
        }
    }

    fn next(&mut self) -> std::result::Result<Option<Token>, Token> {
        if let Some(name) = self.raw_until.take() {
            let rest = &self.src[self.pos..];
            let end = rest
                .to_ascii_lowercase()
                .find(&format!("</{name}"))
                .unwrap_or(rest.len());
            if end > 0 {
                let line = self.line_at(self.pos);
                let text = rest[..end].to_string();
                self.pos += end;
                return Ok(Some(Token {
                    line,
                    tok: Tok::Text(text),
                }));
            }
        }
        let bytes = self.src.as_bytes();
        if self.pos >= bytes.len() {
            return Ok(None);
        }
        if bytes[self.pos] != b'<' {
            let end = self.src[self.pos..]
                .find('<')
                .map_or(self.src.len(), |offset| self.pos + offset);
            let raw = &self.src[self.pos..end];
            let first = raw
                .find(|c: char| !c.is_ascii_whitespace())
                .unwrap_or_default();
            let line = self.line_at(self.pos + first);
            self.pos = end;
            return Ok(Some(Token {
                line,
                tok: Tok::Text(decode(raw)),
            }));
        }
        let start = self.pos;
        let rest = &self.src[start + 1..];
        if rest.starts_with("!--") {
            return match rest.find("-->") {
                Some(end) => self.skipped(start, start + 1 + end + 3),
                None => Err(self.fail(start, "this comment is never closed: it needs `-->`")),
            };
        }
        if rest.starts_with(['!', '?']) {
            return match rest.find('>') {
                Some(end) => self.skipped(start, start + 1 + end + 1),
                None => Err(self.fail(start, "this declaration is never closed: it needs `>`")),
            };
        }
        if let Some(after) = rest.strip_prefix('/') {
            return self.end_tag(start, after);
        }
        if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return self.start_tag(start).map(Some);
        }
        Err(self.fail(
            start,
            "a `<` that does not start a tag must be written `&lt;`",
        ))
    }

    fn skipped(&mut self, start: usize, end: usize) -> std::result::Result<Option<Token>, Token> {
        let line = self.line_at(start);
        self.pos = end;
        Ok(Some(Token {
            line,
            tok: Tok::Skipped,
        }))
    }

    fn end_tag(&mut self, start: usize, after: &str) -> std::result::Result<Option<Token>, Token> {
        let name_len = after.bytes().take_while(|&byte| is_name_byte(byte)).count();
        let name = after[..name_len].to_ascii_lowercase();
        let tail = after[name_len..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        if name.is_empty() || !tail.starts_with('>') {
            return Err(self.fail(start, "this end tag is malformed: write `</name>`"));
        }
        let line = self.line_at(start);
        self.pos = self.src.len() - tail.len() + 1;
        Ok(Some(Token {
            line,
            tok: Tok::End(name),
        }))
    }

    fn start_tag(&mut self, start: usize) -> std::result::Result<Token, Token> {
        let src = self.src;
        let bytes = src.as_bytes();
        let line = self.line_at(start);
        let name_start = start + 1;
        let mut i = name_start;
        while i < bytes.len() && is_name_byte(bytes[i]) {
            i += 1;
        }
        let raw_name = src[name_start..i].to_string();
        let name = raw_name.to_ascii_lowercase();
        if i < bytes.len() && !(bytes[i].is_ascii_whitespace() || matches!(bytes[i], b'/' | b'>')) {
            return Err(self.fail(start, format!("`<{raw_name}` is malformed")));
        }
        let mut attrs: Vec<Attr> = Vec::new();
        let mut self_closing = false;
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                return Err(self.fail(start, format!("`<{name}>` is never closed: it needs `>`")));
            }
            match bytes[i] {
                b'>' => {
                    i += 1;
                    break;
                }
                b'/' => {
                    if bytes.get(i + 1) == Some(&b'>') {
                        self_closing = true;
                        i += 2;
                        break;
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let attr_start = i;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !matches!(bytes[i], b'=' | b'>' | b'/')
            {
                i += 1;
            }
            if i == attr_start {
                return Err(self.fail(start, format!("`<{name}>` has a malformed attribute")));
            }
            let raw_attr = src[attr_start..i].to_string();
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
                    Some(&quote @ (b'"' | b'\'')) => {
                        let value_start = i + 1;
                        let Some(length) = src[value_start..].find(quote as char) else {
                            return Err(self.fail(
                                start,
                                format!("the value of `{raw_attr}` is never closed: it needs a closing quote"),
                            ));
                        };
                        value = decode(&src[value_start..value_start + length]);
                        i = value_start + length + 1;
                    }
                    _ => {
                        let value_start = i;
                        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>'
                        {
                            i += 1;
                        }
                        value = decode(&src[value_start..i]);
                    }
                }
            }
            let attr_name = raw_attr.to_ascii_lowercase();
            if attrs.iter().any(|attr| attr.name == attr_name) {
                return Err(self.fail(
                    start,
                    format!("the attribute `{attr_name}` is written twice on `<{name}>`"),
                ));
            }
            attrs.push(Attr {
                name: attr_name,
                raw_name: raw_attr,
                value,
            });
        }
        self.pos = i;
        if !self_closing && matches!(name.as_str(), "script" | "style") {
            self.raw_until = Some(name.clone());
        }
        Ok(Token {
            line,
            tok: Tok::Start(StartTag {
                name,
                raw_name,
                attrs,
                self_closing,
                line,
            }),
        })
    }
}

/// Decodes character references. A `&` that does not start a known one stays
/// as it is, which is what a browser shows.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let decoded = after
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| reference(&after[..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character that the reference `&name;` stands for.
fn reference(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code).filter(|&c| c != '\0')
        }
    }
}

// --- Parser ---

struct Parser {
    tokens: std::vec::IntoIter<Token>,
    last_line: usize,
    ids: HashSet<NodeId>,
    depth: usize,
}

fn is_blank(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_whitespace())
}

impl Parser {
    /// The next token, including comments. A lexing error is returned when the
    /// parser reaches it, so the first problem in the document is the one named.
    fn raw_next(&mut self) -> Result<Token> {
        match self.tokens.next() {
            Some(Token {
                line,
                tok: Tok::Error(message),
            }) => Err(error(line, message)),
            Some(token) => {
                self.last_line = token.line;
                Ok(token)
            }
            None => Ok(Token {
                line: self.last_line,
                tok: Tok::Eof,
            }),
        }
    }

    /// The next token. Comments, doctypes and processing instructions are not
    /// part of the format.
    fn next(&mut self) -> Result<Token> {
        let token = self.raw_next()?;
        if matches!(token.tok, Tok::Skipped) {
            return Err(error(
                token.line,
                "comments, doctypes and processing instructions are not allowed",
            ));
        }
        Ok(token)
    }

    fn descend<T>(&mut self, line: usize, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= MAX_DEPTH {
            return Err(error(
                line,
                format!("blocks are nested more than {MAX_DEPTH} levels deep"),
            ));
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn read_id(&mut self, tag: &StartTag, extra: &[&str]) -> Result<Option<NodeId>> {
        let mut allowed = vec!["data-node-id"];
        allowed.extend_from_slice(extra);
        tag.check_attrs(&allowed)?;
        let Some(value) = tag.attr("data-node-id") else {
            return Ok(None);
        };
        let id = NodeId::from_str(value).map_err(|_| {
            error(
                tag.line,
                format!("`data-node-id` is `{value}`, which is not a node ID: node IDs are 8 lowercase base32 characters (0-9 and a-z without i, l, o and u)"),
            )
        })?;
        if !self.ids.insert(id) {
            return Err(error(
                tag.line,
                format!("`data-node-id` `{id}` is used twice: every block has its own ID"),
            ));
        }
        Ok(Some(id))
    }

    /// The blocks up to the end tag of `parent`, or to the end of the document
    /// when there is none.
    fn blocks(&mut self, parent: Option<&StartTag>) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();
        loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) => {
                    if !is_blank(&text) {
                        return Err(error(line, TEXT_OUTSIDE));
                    }
                }
                Tok::Start(tag) => blocks.push(self.block(tag)?),
                Tok::End(name) => match parent {
                    Some(parent) if parent.name == name => return Ok(blocks),
                    Some(parent) => return Err(mismatch(line, &name, parent)),
                    None => {
                        return Err(error(
                            line,
                            format!("`</{name}>` closes a tag that is not open"),
                        ));
                    }
                },
                Tok::Eof => match parent {
                    Some(parent) => return Err(never_closed(parent)),
                    None => return Ok(blocks),
                },
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        }
    }

    fn block(&mut self, tag: StartTag) -> Result<Block> {
        let name = tag.name.as_str();
        let is_container = matches!(
            name,
            "section"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "p"
                | "blockquote"
                | "ul"
                | "ol"
                | "table"
                | "figure"
                | "aside"
                | "pre"
                | "article"
        );
        if is_container && tag.self_closing {
            return Err(error(
                tag.line,
                format!(
                    "{} cannot be self-closing: write `<{name}>` and `</{name}>`",
                    tag.display()
                ),
            ));
        }
        let (id, body) = match name {
            "section" => {
                let id = self.read_id(&tag, &[])?;
                let children = self.descend(tag.line, |p| p.blocks(Some(&tag)))?;
                (id, Body::Section { children })
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let id = self.read_id(&tag, &[])?;
                let level = name.as_bytes()[1] - b'0';
                let inline = self.inline_of(&tag)?;
                (id, Body::Heading { level, inline })
            }
            "p" => {
                let id = self.read_id(&tag, &["data-block"])?;
                let requirement = match tag.attr("data-block") {
                    None => false,
                    Some("requirement") => true,
                    Some(other) => return Err(unknown_block_type(&tag, other)),
                };
                let inline = self.inline_of(&tag)?;
                let body = if requirement {
                    Body::Requirement { inline }
                } else {
                    Body::Paragraph { inline }
                };
                (id, body)
            }
            "blockquote" => {
                let id = self.read_id(&tag, &[])?;
                let inline = self.inline_of(&tag)?;
                (id, Body::Quote { inline })
            }
            "hr" => {
                let id = self.read_id(&tag, &[])?;
                (id, Body::Divider)
            }
            "ul" | "ol" => {
                let id = self.read_id(&tag, &[])?;
                let children = self.descend(tag.line, |p| {
                    p.children(&tag, &["li"], "a list holds `<li>` items only")
                })?;
                (
                    id,
                    Body::List {
                        ordered: name == "ol",
                        children,
                    },
                )
            }
            "table" => {
                let id = self.read_id(&tag, &[])?;
                let children = self.descend(tag.line, |p| {
                    p.children(&tag, &["tr"], "a table holds `<tr>` rows only")
                })?;
                (id, Body::Table { children })
            }
            "figure" => {
                let id = self.read_id(&tag, &[])?;
                let (source, caption) = self.descend(tag.line, |p| p.figure(&tag))?;
                (id, Body::Figure { source, caption })
            }
            "aside" => {
                let id = self.read_id(&tag, &["data-callout"])?;
                let kind = match tag.attr("data-callout") {
                    Some("note") => CalloutKind::Note,
                    Some("warning") => CalloutKind::Warning,
                    Some("decision") => CalloutKind::Decision,
                    Some(other) => {
                        return Err(error(
                            tag.line,
                            format!(
                                "`data-callout=\"{other}\"` is not a callout kind: use note, warning or decision"
                            ),
                        ));
                    }
                    None => {
                        return Err(error(
                            tag.line,
                            "an `<aside>` is a callout and needs a `data-callout` attribute: note, warning or decision",
                        ));
                    }
                };
                let inline = self.inline_of(&tag)?;
                (id, Body::Callout { kind, inline })
            }
            "pre" => {
                if tag.attr("class").is_some() {
                    return Err(error(
                        tag.line,
                        "a `<pre>` with a class belongs inside a `<figure>` with a `<figcaption>`; a code block is `<pre><code>`",
                    ));
                }
                let id = self.read_id(&tag, &[])?;
                let code = self.code_block(&tag)?;
                (id, Body::CodeBlock { code })
            }
            "img" => {
                let id = self.read_id(&tag, &["src", "alt"])?;
                let Some(src) = tag.attr("src") else {
                    return Err(error(
                        tag.line,
                        "an `<img>` needs a `src` that is an attachment: `attachment:<id>`",
                    ));
                };
                let attachment = AttachmentRef::from_str(src).map_err(|_| {
                    error(
                        tag.line,
                        format!("the image `src` `{src}` is not an attachment: images must be written `attachment:<id>`"),
                    )
                })?;
                let alt = tag.attr("alt").map(str::to_string);
                (id, Body::Image { attachment, alt })
            }
            "article" => {
                let id = self.read_id(&tag, &["data-block"])?;
                if tag.attr("data-block") != Some("user-story") {
                    return Err(error(
                        tag.line,
                        "an `<article>` is only allowed as a user story: `<article data-block=\"user-story\">`",
                    ));
                }
                let children = self.descend(tag.line, |p| p.blocks(Some(&tag)))?;
                (id, Body::UserStory { children })
            }
            _ => return Err(not_a_block(&tag)),
        };
        Ok(Block { id, body })
    }

    /// The blocks that fill a list, a table or a row, which hold `allowed` tags
    /// only.
    fn children(&mut self, parent: &StartTag, allowed: &[&str], rule: &str) -> Result<Vec<Block>> {
        let mut children = Vec::new();
        loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) => {
                    if !is_blank(&text) {
                        return Err(error(
                            line,
                            format!(
                                "text is not allowed directly inside {}: {rule}",
                                parent.display()
                            ),
                        ));
                    }
                }
                Tok::Start(tag) if allowed.contains(&tag.name.as_str()) => {
                    children.push(self.child(tag)?)
                }
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside {}: {rule}",
                            tag.display(),
                            parent.display()
                        ),
                    ));
                }
                Tok::End(name) if name == parent.name => return Ok(children),
                Tok::End(name) => return Err(mismatch(line, &name, parent)),
                Tok::Eof => return Err(never_closed(parent)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        }
    }

    /// A list item, a table row or a table cell.
    fn child(&mut self, tag: StartTag) -> Result<Block> {
        if tag.self_closing {
            return Err(error(
                tag.line,
                format!(
                    "{} cannot be self-closing: write `<{}>` and `</{}>`",
                    tag.display(),
                    tag.name,
                    tag.name
                ),
            ));
        }
        let id = self.read_id(&tag, &[])?;
        let body = match tag.name.as_str() {
            "li" => Body::ListItem {
                inline: self.inline_of(&tag)?,
            },
            "tr" => Body::TableRow {
                children: self.descend(tag.line, |p| {
                    p.children(
                        &tag,
                        &["td", "th"],
                        "a row holds `<td>` and `<th>` cells only",
                    )
                })?,
            },
            _ => Body::TableCell {
                header: tag.name == "th",
                inline: self.inline_of(&tag)?,
            },
        };
        Ok(Block { id, body })
    }

    fn inline_of(&mut self, tag: &StartTag) -> Result<Vec<Inline>> {
        self.descend(tag.line, |p| p.inline(tag, false))
    }

    /// The inline content up to the end tag of `parent`.
    fn inline(&mut self, parent: &StartTag, in_link: bool) -> Result<Vec<Inline>> {
        let mut inline = Vec::new();
        loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) => inline.push(Inline::Text(text)),
                Tok::Start(tag) => {
                    let name = tag.name.as_str();
                    if matches!(name, "strong" | "em" | "code" | "a" | "br") {
                        tag.check_attrs(if name == "a" { &["href"] } else { &[] })?;
                    }
                    match name {
                        "br" => inline.push(Inline::LineBreak),
                        "strong" | "em" | "code" | "a" if tag.self_closing => {
                            return Err(error(
                                tag.line,
                                format!(
                                    "{} cannot be self-closing: write `<{name}>` and `</{name}>`",
                                    tag.display()
                                ),
                            ));
                        }
                        "strong" => {
                            let children = self.descend(tag.line, |p| p.inline(&tag, in_link))?;
                            inline.push(Inline::Strong(children));
                        }
                        "em" => {
                            let children = self.descend(tag.line, |p| p.inline(&tag, in_link))?;
                            inline.push(Inline::Emphasis(children));
                        }
                        "code" => inline.push(Inline::Code(self.text_only(&tag)?)),
                        "a" if in_link => {
                            return Err(error(tag.line, "a `<a>` link cannot hold another link"));
                        }
                        "a" => {
                            let href = tag.attr("href").unwrap_or_default().to_string();
                            if !is_safe_link(&href) {
                                return Err(error(
                                    tag.line,
                                    format!(
                                        "the link `href` `{href}` is not allowed: links must use http, https or mailto"
                                    ),
                                ));
                            }
                            let children = self.descend(tag.line, |p| p.inline(&tag, true))?;
                            inline.push(Inline::Link { href, children });
                        }
                        _ => {
                            return Err(error(
                                tag.line,
                                format!(
                                    "{} is not allowed inside {}: inline markup is `<strong>`, `<em>`, `<code>`, `<a>` and `<br>`",
                                    tag.display(),
                                    parent.display()
                                ),
                            ));
                        }
                    }
                }
                Tok::End(name) if name == parent.name => return Ok(inline),
                Tok::End(name) => return Err(mismatch(line, &name, parent)),
                Tok::Eof => return Err(never_closed(parent)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        }
    }

    /// The text up to the end tag of `parent`, which holds text only.
    fn text_only(&mut self, parent: &StartTag) -> Result<String> {
        if parent.self_closing {
            return Ok(String::new());
        }
        let mut text = String::new();
        loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(more) => text.push_str(&more),
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside {}: it holds text only",
                            tag.display(),
                            parent.display()
                        ),
                    ));
                }
                Tok::End(name) if name == parent.name => return Ok(text),
                Tok::End(name) => return Err(mismatch(line, &name, parent)),
                Tok::Eof => return Err(never_closed(parent)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        }
    }

    /// The text of the `<code>` inside a `<pre>`.
    fn code_block(&mut self, pre: &StartTag) -> Result<String> {
        let code = loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) if is_blank(&text) => {}
                Tok::Text(_) => return Err(error(line, NOT_A_CODE_BLOCK)),
                Tok::Start(tag) if tag.name == "code" => break tag,
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside {}: {NOT_A_CODE_BLOCK}",
                            tag.display(),
                            pre.display()
                        ),
                    ));
                }
                Tok::End(name) if name == pre.name => {
                    return Err(error(pre.line, NOT_A_CODE_BLOCK));
                }
                Tok::End(name) => return Err(mismatch(line, &name, pre)),
                Tok::Eof => return Err(never_closed(pre)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        };
        code.check_attrs(&[])?;
        let text = self.text_only(&code)?;
        loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) if is_blank(&text) => {}
                Tok::Text(_) => return Err(error(line, NOT_A_CODE_BLOCK)),
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside {}: {NOT_A_CODE_BLOCK}",
                            tag.display(),
                            pre.display()
                        ),
                    ));
                }
                Tok::End(name) if name == pre.name => return Ok(text),
                Tok::End(name) => return Err(mismatch(line, &name, pre)),
                Tok::Eof => return Err(never_closed(pre)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        }
    }

    /// A figure's picture and caption, up to its end tag.
    fn figure(&mut self, figure: &StartTag) -> Result<(FigureSource, String)> {
        let needs_picture = || {
            error(
                figure.line,
                "a `<figure>` needs an `<svg>` or a `<pre class=\"mermaid\">` before its `<figcaption>`",
            )
        };
        let source = loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) if is_blank(&text) => {}
                Tok::Text(_) => return Err(error(line, TEXT_OUTSIDE)),
                Tok::Start(tag) if tag.name == "svg" => {
                    break FigureSource::Svg(self.svg(&tag)?);
                }
                Tok::Start(tag) if tag.name == "pre" => {
                    if tag.attrs.len() != 1 || tag.attr("class") != Some("mermaid") {
                        return Err(error(
                            tag.line,
                            "the `<pre>` of a figure must be `<pre class=\"mermaid\">` and have no other attributes",
                        ));
                    }
                    let mut code = self.text_only(&tag)?;
                    if code.starts_with('\n') {
                        // A browser drops the line break right after `<pre>`.
                        code.remove(0);
                    }
                    break FigureSource::Mermaid(code);
                }
                Tok::Start(tag) if tag.name == "figcaption" => return Err(needs_picture()),
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside `<figure>`: a figure holds an `<svg>` or a Mermaid `<pre>` and a `<figcaption>`",
                            tag.display()
                        ),
                    ));
                }
                Tok::End(name) if name == figure.name => return Err(needs_picture()),
                Tok::End(name) => return Err(mismatch(line, &name, figure)),
                Tok::Eof => return Err(never_closed(figure)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        };
        let caption = loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) if is_blank(&text) => {}
                Tok::Text(_) => return Err(error(line, TEXT_OUTSIDE)),
                Tok::Start(tag) if tag.name == "figcaption" => {
                    tag.check_attrs(&[])?;
                    break self.text_only(&tag)?;
                }
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside `<figure>`: a figure holds one picture and then its `<figcaption>`",
                            tag.display()
                        ),
                    ));
                }
                Tok::End(name) if name == figure.name => {
                    return Err(error(
                        figure.line,
                        "a `<figure>` needs a `<figcaption>`: every figure has a caption",
                    ));
                }
                Tok::End(name) => return Err(mismatch(line, &name, figure)),
                Tok::Eof => return Err(never_closed(figure)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        };
        loop {
            let Token { line, tok } = self.next()?;
            match tok {
                Tok::Text(text) if is_blank(&text) => {}
                Tok::Text(_) => return Err(error(line, TEXT_OUTSIDE)),
                Tok::Start(tag) => {
                    return Err(error(
                        tag.line,
                        format!(
                            "{} is not allowed inside `<figure>`: a figure ends after its `<figcaption>`",
                            tag.display()
                        ),
                    ));
                }
                Tok::End(name) if name == figure.name => return Ok((source, caption)),
                Tok::End(name) => return Err(mismatch(line, &name, figure)),
                Tok::Eof => return Err(never_closed(figure)),
                Tok::Skipped | Tok::Error(_) => unreachable!("handled by next"),
            }
        }
    }

    /// The cleaned markup of the `<svg>` that `root` starts.
    fn svg(&mut self, root: &StartTag) -> Result<String> {
        let element = self
            .svg_element(root, true)?
            .expect("an <svg> is on the allow-list");
        let mut out = String::new();
        write_svg(&element, &mut out);
        Ok(out)
    }

    /// Reads one SVG element and everything inside it, and returns it cleaned,
    /// or `None` when it is dropped. An element that is not on the allow-list is
    /// dropped with all that it holds; `keep` is false inside one.
    fn svg_element(&mut self, tag: &StartTag, keep: bool) -> Result<Option<SvgElement>> {
        self.descend(tag.line, |parser| {
            let canonical = SVG_ELEMENTS
                .iter()
                .find(|name| name.eq_ignore_ascii_case(&tag.raw_name))
                .copied();
            let kept = keep && canonical.is_some();
            let keeps_text = kept && SVG_TEXT_ELEMENTS.contains(&canonical.unwrap_or_default());
            let mut children = Vec::new();
            let is_void = matches!(
                tag.name.as_str(),
                "area"
                    | "base"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "link"
                    | "meta"
                    | "source"
                    | "track"
                    | "wbr"
            );
            if !tag.self_closing && !is_void {
                loop {
                    let Token { line, tok } = parser.raw_next()?;
                    match tok {
                        Tok::Text(text) => {
                            if keeps_text {
                                children.push(SvgNode::Text(text));
                            }
                        }
                        Tok::Start(child) => {
                            if let Some(element) = parser.svg_element(&child, kept)? {
                                children.push(SvgNode::Element(element));
                            }
                        }
                        Tok::End(name) if name == tag.name => break,
                        Tok::End(name) => return Err(mismatch(line, &name, tag)),
                        Tok::Skipped => {}
                        Tok::Eof => return Err(never_closed(tag)),
                        Tok::Error(_) => unreachable!("handled by raw_next"),
                    }
                }
            }
            let Some(name) = canonical.filter(|_| kept) else {
                return Ok(None);
            };
            let attrs = tag
                .attrs
                .iter()
                .filter_map(|attr| {
                    let name = SVG_ATTRIBUTES
                        .iter()
                        .find(|name| name.eq_ignore_ascii_case(&attr.raw_name))?;
                    let safe = if *name == "xmlns" {
                        attr.value == SVG_NAMESPACE
                    } else {
                        is_safe_svg_value(&attr.value)
                    };
                    safe.then(|| (*name, attr.value.clone()))
                })
                .collect();
            Ok(Some(SvgElement {
                name,
                attrs,
                children,
            }))
        })
    }
}

const TEXT_OUTSIDE: &str =
    "text is not allowed here: write it inside a paragraph or another text block";

const NOT_A_CODE_BLOCK: &str = "a code block is `<pre><code>` holding text only";

fn mismatch(line: usize, end: &str, open: &StartTag) -> ParseError {
    error(
        line,
        format!(
            "`</{end}>` does not match the open {}: close it with `</{}>` first",
            open.display(),
            open.name
        ),
    )
}

fn never_closed(tag: &StartTag) -> ParseError {
    error(
        tag.line,
        format!("{} is never closed: add `</{}>`", tag.display(), tag.name),
    )
}

fn unknown_block_type(tag: &StartTag, value: &str) -> ParseError {
    error(
        tag.line,
        format!(
            "`data-block=\"{value}\"` is not a block type: the only one on {} is `requirement`",
            tag.display()
        ),
    )
}

/// The error for a start tag that begins no block.
fn not_a_block(tag: &StartTag) -> ParseError {
    let name = tag.display();
    let message = match tag.name.as_str() {
        "li" => format!("{name} must be inside a `<ul>` or `<ol>`"),
        "tr" => format!("{name} must be inside a `<table>`"),
        "td" | "th" => format!("{name} must be inside a `<tr>` of a `<table>`"),
        "strong" | "em" | "code" | "a" | "br" => {
            format!("{name} is inline markup and must be inside a text block such as a paragraph")
        }
        "svg" => format!("{name} is only allowed inside a `<figure>` with a `<figcaption>`"),
        "figcaption" => format!("{name} is only allowed inside a `<figure>`"),
        _ => format!("{name} is not part of the format"),
    };
    error(tag.line, message)
}

/// Whether a link may be kept: http, https or mailto, with something after it.
fn is_safe_link(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    let rest = ["http://", "https://", "mailto:"]
        .iter()
        .find_map(|scheme| lower.strip_prefix(scheme));
    rest.is_some_and(|rest| !rest.is_empty())
        && !href.chars().any(|c| c.is_whitespace() || c.is_control())
}

// --- SVG cleaning ---

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// The SVG elements a figure may keep: shapes, text, groups and gradients. A
/// script, a style, a `foreignObject`, an image, a link or a `use` is dropped
/// with all that it holds.
const SVG_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "title",
    "desc",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "path",
    "text",
    "tspan",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "marker",
];

/// The elements whose text is kept.
const SVG_TEXT_ELEMENTS: &[&str] = &["text", "tspan", "title", "desc"];

/// The SVG attributes a figure may keep: geometry and presentation. No event
/// handler, `style`, `href` or `xlink:href` is on the list.
const SVG_ATTRIBUTES: &[&str] = &[
    "xmlns",
    "viewBox",
    "preserveAspectRatio",
    "width",
    "height",
    "x",
    "y",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x1",
    "y1",
    "x2",
    "y2",
    "fx",
    "fy",
    "dx",
    "dy",
    "points",
    "d",
    "pathLength",
    "transform",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "clip-path",
    "clip-rule",
    "clipPathUnits",
    "marker-start",
    "marker-mid",
    "marker-end",
    "markerWidth",
    "markerHeight",
    "markerUnits",
    "refX",
    "refY",
    "orient",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-anchor",
    "dominant-baseline",
    "letter-spacing",
    "text-decoration",
    "id",
    "class",
    "role",
    "aria-label",
    "aria-hidden",
    "aria-labelledby",
    "aria-describedby",
    "version",
];

struct SvgElement {
    name: &'static str,
    attrs: Vec<(&'static str, String)>,
    children: Vec<SvgNode>,
}

enum SvgNode {
    Element(SvgElement),
    Text(String),
}

/// Whether the value of an SVG attribute loads nothing from outside and runs
/// nothing: no script URL, no `@import`, no `url()` except to an ID in the same
/// picture, and no address.
fn is_safe_svg_value(value: &str) -> bool {
    let squashed: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    const FORBIDDEN: [&str; 9] = [
        "javascript:",
        "vbscript:",
        "data:",
        "@import",
        "expression(",
        "://",
        "<",
        ">",
        "\\",
    ];
    if squashed.starts_with("//") || FORBIDDEN.iter().any(|bad| squashed.contains(bad)) {
        return false;
    }
    let mut rest = squashed.as_str();
    while let Some(at) = rest.find("url(") {
        let after = rest[at + 4..].trim_start_matches(['\'', '"']);
        if !after.starts_with('#') {
            return false;
        }
        rest = after;
    }
    true
}

fn write_svg(element: &SvgElement, out: &mut String) {
    out.push('<');
    out.push_str(element.name);
    for (name, value) in &element.attrs {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&escape_attr(value));
        out.push('"');
    }
    if element.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in &element.children {
        match child {
            SvgNode::Element(child) => write_svg(child, out),
            SvgNode::Text(text) => out.push_str(&escape_text(text)),
        }
    }
    out.push_str("</");
    out.push_str(element.name);
    out.push('>');
}

// --- Printing ---

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}

fn print_blocks(blocks: &[Block], out: &mut String) {
    for block in blocks {
        print_block(block, out);
        out.push('\n');
    }
}

fn print_inline(inline: &[Inline], out: &mut String) {
    for part in inline {
        match part {
            Inline::Text(text) => out.push_str(&escape_text(text)),
            Inline::Strong(children) => {
                out.push_str("<strong>");
                print_inline(children, out);
                out.push_str("</strong>");
            }
            Inline::Emphasis(children) => {
                out.push_str("<em>");
                print_inline(children, out);
                out.push_str("</em>");
            }
            Inline::Code(code) => {
                out.push_str("<code>");
                out.push_str(&escape_text(code));
                out.push_str("</code>");
            }
            Inline::Link { href, children } => {
                out.push_str("<a href=\"");
                out.push_str(&escape_attr(href));
                out.push_str("\">");
                print_inline(children, out);
                out.push_str("</a>");
            }
            Inline::LineBreak => out.push_str("<br>"),
        }
    }
}

/// Prints `<tag data-node-id="..." extra>`, the ID first.
fn open_tag(tag: &str, id: Option<&NodeId>, extra: &str, out: &mut String) {
    out.push('<');
    out.push_str(tag);
    if let Some(id) = id {
        out.push_str(" data-node-id=\"");
        out.push_str(id.as_str());
        out.push('"');
    }
    out.push_str(extra);
    out.push('>');
}

fn print_text_block(tag: &str, block: &Block, extra: &str, inline: &[Inline], out: &mut String) {
    open_tag(tag, block.id(), extra, out);
    print_inline(inline, out);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn print_container(tag: &str, block: &Block, extra: &str, children: &[Block], out: &mut String) {
    open_tag(tag, block.id(), extra, out);
    out.push('\n');
    print_blocks(children, out);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn print_block(block: &Block, out: &mut String) {
    match &block.body {
        Body::Section { children } => print_container("section", block, "", children, out),
        Body::Heading { level, inline } => {
            print_text_block(&format!("h{level}"), block, "", inline, out)
        }
        Body::Paragraph { inline } => print_text_block("p", block, "", inline, out),
        Body::Quote { inline } => print_text_block("blockquote", block, "", inline, out),
        Body::Divider => open_tag("hr", block.id(), "", out),
        Body::List { ordered, children } => {
            print_container(if *ordered { "ol" } else { "ul" }, block, "", children, out)
        }
        Body::ListItem { inline } => print_text_block("li", block, "", inline, out),
        Body::Table { children } => print_container("table", block, "", children, out),
        Body::TableRow { children } => print_container("tr", block, "", children, out),
        Body::TableCell { header, inline } => {
            print_text_block(if *header { "th" } else { "td" }, block, "", inline, out)
        }
        Body::Figure { source, caption } => {
            open_tag("figure", block.id(), "", out);
            match source {
                FigureSource::Svg(markup) => out.push_str(markup),
                FigureSource::Mermaid(code) => {
                    out.push_str("<pre class=\"mermaid\">");
                    if code.starts_with('\n') {
                        // The line break that a browser drops after `<pre>`.
                        out.push('\n');
                    }
                    out.push_str(&escape_text(code));
                    out.push_str("</pre>");
                }
            }
            out.push_str("<figcaption>");
            out.push_str(&escape_text(caption));
            out.push_str("</figcaption></figure>");
        }
        Body::Callout { kind, inline } => print_text_block(
            "aside",
            block,
            &format!(" data-callout=\"{}\"", kind.as_str()),
            inline,
            out,
        ),
        Body::CodeBlock { code } => {
            open_tag("pre", block.id(), "", out);
            out.push_str("<code>");
            out.push_str(&escape_text(code));
            out.push_str("</code></pre>");
        }
        Body::Image { attachment, alt } => {
            let mut extra = format!(" src=\"{}\"", escape_attr(attachment.as_str()));
            if let Some(alt) = alt {
                extra.push_str(&format!(" alt=\"{}\"", escape_attr(alt)));
            }
            open_tag("img", block.id(), &extra, out);
        }
        Body::Requirement { inline } => {
            print_text_block("p", block, " data-block=\"requirement\"", inline, out)
        }
        Body::UserStory { children } => print_container(
            "article",
            block,
            " data-block=\"user-story\"",
            children,
            out,
        ),
    }
}
