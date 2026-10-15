//! HTML docs: PRDs, architecture and database docs as a tree of typed blocks
//! with stable node IDs, in a strict HTML form (docs/formats/html-docs.md).

use crate::design::{AttachmentRef, NodeId};
use serde::{Deserialize, Serialize};
use std::fmt;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalloutKind {
    Note,
    Warning,
    Decision,
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

/// One block of a doc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    id: Option<NodeId>,
}

impl Block {
    /// The block's type.
    pub fn kind(&self) -> BlockKind {
        todo!("the type of a block")
    }

    /// The block's `data-node-id`, or `None` for a block that is new.
    pub fn id(&self) -> Option<&NodeId> {
        todo!("the ID of a block")
    }

    /// Whether the block has no ID yet: the server assigns one when it is saved.
    pub fn is_new(&self) -> bool {
        todo!("whether a block is new")
    }

    /// The blocks inside this one: the blocks of a section or user story, the
    /// items of a list, the rows of a table or the cells of a row.
    pub fn children(&self) -> &[Block] {
        todo!("the children of a block")
    }

    /// The inline content of a heading, paragraph, quote, list item, table
    /// cell, callout or requirement. Empty for the other blocks.
    pub fn inline(&self) -> &[Inline] {
        todo!("the inline content of a block")
    }

    /// The level, 1 to 6, of a heading.
    pub fn heading_level(&self) -> Option<u8> {
        todo!("the level of a heading")
    }

    /// Whether a list is numbered (`<ol>`) rather than bulleted.
    pub fn is_ordered(&self) -> bool {
        todo!("whether a list is ordered")
    }

    /// Whether a table cell is a header cell (`<th>`).
    pub fn is_header(&self) -> bool {
        todo!("whether a cell is a header")
    }

    /// The kind of a callout.
    pub fn callout_kind(&self) -> Option<CalloutKind> {
        todo!("the kind of a callout")
    }

    /// The text of a code block, exactly as written.
    pub fn code(&self) -> Option<&str> {
        todo!("the text of a code block")
    }

    /// What a figure shows.
    pub fn figure(&self) -> Option<&FigureSource> {
        todo!("what a figure shows")
    }

    /// A figure's caption, as text.
    pub fn caption(&self) -> Option<&str> {
        todo!("the caption of a figure")
    }

    /// The attachment an image shows.
    pub fn attachment(&self) -> Option<&AttachmentRef> {
        todo!("the attachment of an image")
    }

    /// An image's alternative text.
    pub fn alt(&self) -> Option<&str> {
        todo!("the alternative text of an image")
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
    pub fn parse(_html: &str) -> Result<HtmlDoc, ParseError> {
        todo!("parse an HTML doc")
    }

    /// The doc's top-level blocks.
    pub fn blocks(&self) -> &[Block] {
        todo!("the blocks of a doc")
    }

    /// The doc as HTML, with `data-node-id` on every block.
    pub fn to_html(&self) -> String {
        todo!("print a doc as HTML")
    }
}
