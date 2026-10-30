//! HTML docs: specs, architecture and database docs as free-form HTML
//! (docs/formats/html-docs.md). Any HTML, CSS and inline JavaScript is
//! accepted and kept as written.
//!
//! A doc is read with a general HTML parser (html5ever), the way a browser
//! reads it, into a [`Tree`]. Every block-level element of the tree is a node
//! with a stable ID, written in a `data-omni-id` attribute; the elements and
//! text inside a node are part of its content. [`crate::ops::EditDoc`] edits
//! the tree node by node, and the tree prints back as HTML that reads as the
//! same tree.

use crate::design::NodeId;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, ParseOpts, QualName, namespace_url, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// How deeply elements may nest, so that hostile input cannot overflow the
/// stack.
pub(crate) const MAX_DEPTH: usize = 128;

/// The attribute that holds a block-level element's node ID.
pub(crate) const ID_ATTRIBUTE: &str = "data-omni-id";
/// The attribute that holds a node's revision in a doc under edit.
pub(crate) const REV_ATTRIBUTE: &str = "data-rev";

/// An HTML doc that cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
    message: String,
}

impl ParseError {
    /// The line of the problem, counting from 1, or 0 when it is not in one
    /// place.
    pub fn line(&self) -> usize {
        self.line
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ParseError {}

pub(crate) fn error(message: impl Into<String>) -> ParseError {
    ParseError {
        line: 0,
        message: message.into(),
    }
}

// --- Block-level elements ---

/// The block-level elements, each with the type it is indexed as: the HTML
/// spec's flow content that is not phrasing content, plus `li`, `tr`, `td`,
/// `th`, `figure` and (as an SVG element) `svg`. Every one of them is a node
/// with a `data-omni-id`.
const BLOCK_LEVEL: &[(&str, &str)] = &[
    ("address", "address"),
    ("article", "article"),
    ("aside", "aside"),
    ("blockquote", "quote"),
    ("details", "details"),
    ("dialog", "dialog"),
    ("div", "div"),
    ("dl", "dl"),
    ("fieldset", "fieldset"),
    ("figure", "figure"),
    ("footer", "footer"),
    ("form", "form"),
    ("h1", "heading"),
    ("h2", "heading"),
    ("h3", "heading"),
    ("h4", "heading"),
    ("h5", "heading"),
    ("h6", "heading"),
    ("header", "header"),
    ("hgroup", "hgroup"),
    ("hr", "divider"),
    ("main", "main"),
    ("menu", "menu"),
    ("nav", "nav"),
    ("ol", "list"),
    ("p", "paragraph"),
    ("pre", "code_block"),
    ("search", "search"),
    ("section", "section"),
    ("table", "table"),
    ("ul", "list"),
    ("li", "list_item"),
    ("tr", "table_row"),
    ("td", "table_cell"),
    ("th", "table_cell"),
];

/// The namespace of an element: HTML, or the foreign content that the HTML
/// parser knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ns {
    Html,
    Svg,
    MathMl,
}

/// An element of the tree.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Element {
    /// The name as the parser has it: lowercase for HTML, and for SVG the
    /// case that SVG uses (`foreignObject`).
    pub name: String,
    pub ns: Ns,
    /// The attributes in the order written, without the ID and the revision.
    pub attrs: Vec<(String, String)>,
    /// The node ID of a block-level element once the tree is identified.
    pub id: Option<NodeId>,
    /// The `data-omni-id` as written, until the tree is identified.
    claimed: Option<String>,
    /// The `data-rev` as written, until the tree is identified.
    claimed_rev: Option<String>,
    /// The revision that a doc under edit stored for the node (set when a
    /// tree is identified strictly).
    pub rev: Option<u64>,
}

impl Element {
    pub fn is_void(&self) -> bool {
        self.ns == Ns::Html
            && matches!(
                self.name.as_str(),
                "area"
                    | "base"
                    | "basefont"
                    | "bgsound"
                    | "br"
                    | "col"
                    | "embed"
                    | "frame"
                    | "hr"
                    | "img"
                    | "input"
                    | "keygen"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            )
    }

    /// An element whose text is written as is: the parser reads it as raw
    /// text, so it is printed without escapes.
    fn holds_raw_text(&self) -> bool {
        self.ns == Ns::Html
            && matches!(
                self.name.as_str(),
                "style"
                    | "script"
                    | "xmp"
                    | "iframe"
                    | "noembed"
                    | "noframes"
                    | "plaintext"
                    | "noscript"
            )
    }

    /// Whether the element is block-level, and so a node. `inside_svg` says
    /// that an SVG element holds it, which makes a nested `<svg>` part of the
    /// outer one.
    pub fn is_block_level(&self, inside_svg: bool) -> bool {
        match self.ns {
            Ns::Html => BLOCK_LEVEL.iter().any(|(tag, _)| *tag == self.name),
            Ns::Svg => self.name == "svg" && !inside_svg,
            Ns::MathMl => false,
        }
    }

    /// The type that the node index lists the element as.
    pub fn kind_name(&self) -> &'static str {
        if self.ns == Ns::Svg {
            return "svg";
        }
        BLOCK_LEVEL
            .iter()
            .find(|(tag, _)| *tag == self.name)
            .map_or("element", |(_, kind)| kind)
    }
}

/// What an item of the tree is.
#[derive(Clone, Debug)]
pub(crate) enum Kind {
    /// The top of a tree: its children are the doc's top level.
    Root,
    Text(String),
    Comment(String),
    Element(Element),
}

#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub kind: Kind,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

/// A parsed doc: items in an arena, the root at index 0. An item that is
/// taken out of its parent stays in the arena, unreachable.
#[derive(Clone, Debug)]
pub(crate) struct Tree {
    pub items: Vec<Item>,
}

/// The element a fragment is parsed inside, which decides how the parser reads
/// it (`<li>` belongs in a `<ul>`, a `<td>` in a `<tr>`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context<'a> {
    pub name: &'a str,
    pub ns: Ns,
}

impl Context<'static> {
    pub const BODY: Context<'static> = Context {
        name: "body",
        ns: Ns::Html,
    };
}

impl Tree {
    pub fn new() -> Tree {
        Tree {
            items: vec![Item {
                kind: Kind::Root,
                parent: None,
                children: Vec::new(),
            }],
        }
    }

    pub fn push(&mut self, kind: Kind) -> usize {
        self.items.push(Item {
            kind,
            parent: None,
            children: Vec::new(),
        });
        self.items.len() - 1
    }

    pub fn element(&self, ix: usize) -> Option<&Element> {
        match &self.items[ix].kind {
            Kind::Element(element) => Some(element),
            _ => None,
        }
    }

    pub fn element_mut(&mut self, ix: usize) -> Option<&mut Element> {
        match &mut self.items[ix].kind {
            Kind::Element(element) => Some(element),
            _ => None,
        }
    }

    /// The ID of the item when it is a node.
    pub fn id_of(&self, ix: usize) -> Option<NodeId> {
        self.element(ix).and_then(|element| element.id)
    }

    /// Puts `child`, which has no parent, among the children of `parent`.
    pub fn attach(&mut self, parent: usize, at: usize, child: usize) {
        debug_assert!(self.items[child].parent.is_none());
        self.items[parent].children.insert(at, child);
        self.items[child].parent = Some(parent);
    }

    /// Takes an item out of its parent; it keeps its own children.
    pub fn detach(&mut self, ix: usize) {
        if let Some(parent) = self.items[ix].parent.take() {
            self.items[parent].children.retain(|child| *child != ix);
        }
    }

    /// The place of an item among the children of its parent.
    pub fn position(&self, ix: usize) -> usize {
        let parent = self.items[ix].parent.expect("the item has a parent");
        self.items[parent]
            .children
            .iter()
            .position(|child| *child == ix)
            .expect("a parent lists its children")
    }

    /// The elements above an item, from the top down to its parent.
    pub fn ancestors(&self, ix: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut at = self.items[ix].parent;
        while let Some(parent) = at {
            if matches!(self.items[parent].kind, Kind::Element(_)) {
                chain.push(parent);
            }
            at = self.items[parent].parent;
        }
        chain.reverse();
        chain
    }

    /// Whether `ix` or an element above it is an SVG element.
    pub fn within_svg(&self, ix: usize) -> bool {
        let mut at = Some(ix);
        while let Some(item) = at {
            if self
                .element(item)
                .is_some_and(|element| element.ns == Ns::Svg)
            {
                return true;
            }
            at = self.items[item].parent;
        }
        false
    }

    /// How many levels an item and what is inside it take up.
    pub fn height(&self, ix: usize) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(ix, 1)];
        while let Some((item, level)) = stack.pop() {
            deepest = deepest.max(level);
            stack.extend(self.items[item].children.iter().map(|c| (*c, level + 1)));
        }
        deepest
    }

    /// The items of the subtree of `ix`, in document order, `ix` first.
    pub fn subtree(&self, ix: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![ix];
        while let Some(item) = stack.pop() {
            out.push(item);
            stack.extend(self.items[item].children.iter().rev().copied());
        }
        out
    }

    /// Copies the subtree of `ix` in `from` into this tree, as an item with no
    /// parent.
    pub fn copy_from(&mut self, from: &Tree, ix: usize) -> usize {
        let mut stack = vec![(ix, None::<usize>)];
        let mut top = None;
        while let Some((source, parent)) = stack.pop() {
            let copy = self.push(from.items[source].kind.clone());
            match parent {
                Some(parent) => {
                    self.items[copy].parent = Some(parent);
                    self.items[parent].children.push(copy);
                }
                None => top = Some(copy),
            }
            // Pushed in reverse so that the children are copied in order.
            for child in from.items[source].children.iter().rev() {
                stack.push((*child, Some(copy)));
            }
        }
        top.expect("a subtree has a root")
    }

    // --- Reading ---

    /// Parses a fragment of HTML the way a browser parses the inside of
    /// `context`. The result's root holds the fragment's top level. Nothing is
    /// rejected, except nesting deeper than [`MAX_DEPTH`]: an element that the
    /// parser cannot place is dropped or moved, as a browser does.
    pub fn parse_fragment(html: &str, context: Context<'_>) -> Result<Tree, ParseError> {
        let namespace = match context.ns {
            Ns::Html => ns!(html),
            Ns::Svg => ns!(svg),
            Ns::MathMl => ns!(mathml),
        };
        let dom = parse_fragment(
            RcDom::default(),
            ParseOpts::default(),
            QualName::new(None, namespace, LocalName::from(context.name)),
            Vec::new(),
        )
        .one(html);
        let mut tree = Tree::new();
        // A fragment parse puts the fragment under one `html` element.
        let top: Vec<Handle> = dom.document.children.borrow().clone();
        let source = top
            .iter()
            .find(|node| matches!(node.data, NodeData::Element { .. }))
            .cloned();
        if let Some(source) = source {
            tree.convert(&source)?;
        }
        Ok(tree)
    }

    /// Copies the children of `source` into the root, iteratively.
    fn convert(&mut self, source: &Handle) -> Result<(), ParseError> {
        let mut stack: Vec<(Handle, usize, usize)> = vec![(source.clone(), 0, 1)];
        while let Some((handle, parent, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(error(format!(
                    "elements are nested more than {MAX_DEPTH} levels deep"
                )));
            }
            let children: Vec<Handle> = match &handle.data {
                NodeData::Element {
                    template_contents, ..
                } => match template_contents.borrow().as_ref() {
                    Some(contents) => contents.children.borrow().clone(),
                    None => handle.children.borrow().clone(),
                },
                _ => handle.children.borrow().clone(),
            };
            let mut nested = Vec::new();
            for child in children {
                let kind = match &child.data {
                    NodeData::Text { contents } => Kind::Text(contents.borrow().to_string()),
                    NodeData::Comment { contents } => Kind::Comment(contents.to_string()),
                    NodeData::Element { name, attrs, .. } => {
                        Kind::Element(read_element(name, &attrs.borrow()))
                    }
                    _ => continue,
                };
                let is_element = matches!(kind, Kind::Element(_));
                let ix = self.push(kind);
                self.items[ix].parent = Some(parent);
                self.items[parent].children.push(ix);
                if is_element {
                    nested.push((child, ix, depth + 1));
                }
            }
            // Reversed, so that the tree is walked in document order.
            stack.extend(nested.into_iter().rev());
        }
        Ok(())
    }

    // --- Identifying ---

    /// The block-level elements of the subtrees under `roots`, in document
    /// order, each with the items found. `inside_svg` says that the roots sit
    /// inside an SVG element.
    pub fn block_level(&self, roots: &[usize], inside_svg: bool) -> Vec<usize> {
        let mut found = Vec::new();
        let mut stack: Vec<(usize, bool)> = roots.iter().rev().map(|r| (*r, inside_svg)).collect();
        while let Some((ix, svg)) = stack.pop() {
            let mut svg_below = svg;
            if let Kind::Element(element) = &self.items[ix].kind {
                if element.is_block_level(svg) {
                    found.push(ix);
                }
                svg_below = svg || element.ns == Ns::Svg;
            }
            stack.extend(
                self.items[ix]
                    .children
                    .iter()
                    .rev()
                    .map(|c| (*c, svg_below)),
            );
        }
        found
    }

    /// Reads the IDs of a doc under edit: every block-level element must have
    /// a valid `data-omni-id` that no other element has, and may have a
    /// `data-rev`. Elements that are not block-level lose a `data-omni-id`
    /// they carry.
    pub fn identify_strict(&mut self) -> Result<(), ParseError> {
        let roots = self.items[0].children.clone();
        let blocks = self.block_level(&roots, false);
        let mut seen = HashSet::new();
        for ix in blocks {
            let element = self.element_mut(ix).expect("a block is an element");
            let tag = describe(element);
            let Some(claimed) = element.claimed.take() else {
                return Err(error(format!(
                    "{tag} has no `{ID_ATTRIBUTE}`: every block-level element of a doc under edit has its ID"
                )));
            };
            let id = NodeId::from_str(&claimed).map_err(|_| {
                error(format!(
                    "`{ID_ATTRIBUTE}` is `{claimed}` on {tag}, which is not a node ID: node IDs are 8 lowercase base32 characters (0-9 and a-z without i, l, o and u)"
                ))
            })?;
            if !seen.insert(id) {
                return Err(error(format!(
                    "`{ID_ATTRIBUTE}` `{id}` is used twice: every block-level element has its own ID"
                )));
            }
            element.id = Some(id);
            if let Some(text) = element.claimed_rev.take() {
                let rev = text.parse::<u64>().ok().filter(|rev| *rev >= 1);
                element.rev = Some(rev.ok_or_else(|| {
                    error(format!(
                        "`{REV_ATTRIBUTE}` is `{text}` on {tag}, which is not a revision: revisions are whole numbers from 1"
                    ))
                })?);
            }
        }
        for item in &mut self.items {
            if let Kind::Element(element) = &mut item.kind {
                element.claimed = None;
                element.claimed_rev = None;
            }
        }
        Ok(())
    }

    /// Gives IDs to the block-level elements under `roots`, in document
    /// order, and returns their items. An element keeps the `data-omni-id` it
    /// carries when that is a node ID that `allowed` accepts and no earlier
    /// element has taken; otherwise `fresh` supplies one. `reserved` lists IDs
    /// that nothing may take, and `first` the ID of the first block-level
    /// element, whatever it carries.
    pub fn identify(
        &mut self,
        roots: &[usize],
        inside_svg: bool,
        reserved: &[NodeId],
        first: Option<NodeId>,
        allowed: &dyn Fn(NodeId) -> bool,
        fresh: &mut dyn FnMut() -> NodeId,
    ) -> Vec<usize> {
        let blocks = self.block_level(roots, inside_svg);
        let mut seen: HashSet<NodeId> = reserved.iter().copied().collect();
        seen.extend(first);
        for (n, ix) in blocks.iter().enumerate() {
            let element = self.element_mut(*ix).expect("a block is an element");
            element.claimed_rev = None;
            let claimed = element
                .claimed
                .take()
                .and_then(|text| NodeId::from_str(&text).ok())
                .filter(|id| !seen.contains(id) && allowed(*id));
            let id = match (n, first, claimed) {
                (0, Some(first), _) => first,
                (_, _, Some(claimed)) => claimed,
                _ => loop {
                    let id = fresh();
                    if !seen.contains(&id) {
                        break id;
                    }
                },
            };
            seen.insert(id);
            element.id = Some(id);
        }
        // What is left of a claim on an element that is not a node is dropped.
        for ix in self.subtrees(roots) {
            if let Some(element) = self.element_mut(ix) {
                element.claimed = None;
                element.claimed_rev = None;
            }
        }
        blocks
    }

    fn subtrees(&self, roots: &[usize]) -> Vec<usize> {
        roots.iter().flat_map(|root| self.subtree(*root)).collect()
    }

    // --- Printing ---

    /// Prints an item and what is inside it.
    pub fn print(&self, ix: usize, how: &Print<'_>, out: &mut String) {
        self.print_item(ix, how, false, out);
    }

    fn print_item(&self, ix: usize, how: &Print<'_>, raw: bool, out: &mut String) {
        match &self.items[ix].kind {
            Kind::Root => self.print_children(ix, how, false, out),
            Kind::Text(text) => {
                if raw {
                    out.push_str(text);
                } else {
                    escape_text(text, out);
                }
            }
            Kind::Comment(text) => {
                out.push_str("<!--");
                out.push_str(text);
                out.push_str("-->");
            }
            Kind::Element(element) => {
                // The content of a node without the nodes inside it.
                if how.own.is_some_and(|top| top != ix) && element.id.is_some() {
                    return;
                }
                self.print_open(element, ix, how, out);
                if element.is_void() {
                    return;
                }
                // The parser drops a newline right after these start tags, so
                // a text that starts with one gets another to keep it.
                if element.ns == Ns::Html
                    && matches!(element.name.as_str(), "pre" | "textarea" | "listing")
                    && matches!(
                        self.items[ix].children.first().map(|c| &self.items[*c].kind),
                        Some(Kind::Text(text)) if text.starts_with('\n')
                    )
                {
                    out.push('\n');
                }
                self.print_children(ix, how, element.holds_raw_text(), out);
                out.push_str("</");
                out.push_str(&element.name);
                out.push('>');
            }
        }
    }

    fn print_children(&self, ix: usize, how: &Print<'_>, raw: bool, out: &mut String) {
        for child in &self.items[ix].children {
            self.print_item(*child, how, raw, out);
        }
    }

    /// The start tag of an element.
    pub fn print_open(&self, element: &Element, ix: usize, how: &Print<'_>, out: &mut String) {
        out.push('<');
        out.push_str(&element.name);
        if let (Some(id), true) = (element.id, how.ids && how.own != Some(ix)) {
            write_attr(ID_ATTRIBUTE, id.as_str(), out);
            if let Some(rev) = how.revisions.and_then(|revision| revision(id)) {
                write_attr(REV_ATTRIBUTE, &rev.to_string(), out);
            }
        }
        for (name, value) in &element.attrs {
            write_attr(name, value, out);
        }
        out.push('>');
    }

    /// An element and what is inside it as HTML.
    pub fn html(&self, ix: usize, how: &Print<'_>) -> String {
        let mut out = String::new();
        self.print(ix, how, &mut out);
        out
    }
}

/// How an item is printed.
pub(crate) struct Print<'a> {
    /// Whether nodes carry their `data-omni-id`.
    pub ids: bool,
    /// The revision to write as `data-rev` on a node, if the doc has one.
    pub revisions: Option<&'a dyn Fn(NodeId) -> Option<u64>>,
    /// Print the content of this node only, leaving out the nodes inside it
    /// and its own ID.
    pub own: Option<usize>,
}

impl Print<'_> {
    pub const PLAIN: Print<'static> = Print {
        ids: true,
        revisions: None,
        own: None,
    };
    pub const WITHOUT_IDS: Print<'static> = Print {
        ids: false,
        revisions: None,
        own: None,
    };
}

fn describe(element: &Element) -> String {
    format!("<{}>", element.name)
}

fn write_attr(name: &str, value: &str, out: &mut String) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// An element as the parser read it.
fn read_element(name: &QualName, attributes: &[html5ever::Attribute]) -> Element {
    let ns = if name.ns == ns!(svg) {
        Ns::Svg
    } else if name.ns == ns!(mathml) {
        Ns::MathMl
    } else {
        Ns::Html
    };
    let mut attrs = Vec::new();
    let mut claimed = None;
    let mut claimed_rev = None;
    for attribute in attributes {
        let local = attribute.name.local.to_string();
        let printed = if attribute.name.ns == ns!() {
            local
        } else if attribute.name.ns == ns!(xlink) {
            format!("xlink:{local}")
        } else if attribute.name.ns == ns!(xml) {
            format!("xml:{local}")
        } else if attribute.name.ns == ns!(xmlns) {
            if local == "xmlns" {
                local
            } else {
                format!("xmlns:{local}")
            }
        } else {
            local
        };
        match printed.as_str() {
            ID_ATTRIBUTE => claimed = Some(attribute.value.to_string()),
            REV_ATTRIBUTE => claimed_rev = Some(attribute.value.to_string()),
            _ => attrs.push((printed, attribute.value.to_string())),
        }
    }
    Element {
        name: name.local.to_string(),
        ns,
        attrs,
        id: None,
        claimed,
        claimed_rev,
        rev: None,
    }
}
