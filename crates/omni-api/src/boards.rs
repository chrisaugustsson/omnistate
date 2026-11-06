//! Pages, boards and chunked storage (US-030, Q23). A design doc holds pages
//! and a page holds boards, and the head of a design stores each of them as a
//! chunk of its own in RustFS, at `ws/{workspace_id}/chunks/{hash}`, named by
//! the BLAKE3 hash of its bytes:
//!
//! - a board's chunk is the board and its subtree as JSON;
//! - a page's chunk is the page itself as JSON, without its children;
//! - the head's own chunk is its manifest, which lists the chunk of every
//!   page and, below it, of every board, in order.
//!
//! A chunk's name follows from its content, so a page or a board that a later
//! write does not change has the same chunk in the new head, and is stored,
//! indexed and listed in the node index once, however many heads use it. The
//! node index of a chunk (`node_index`) lists only the nodes of that chunk, and
//! a board's root has no parent in it: a board that moves to another page keeps
//! its chunk, and the manifest says where it sits.
//!
//! Reading one board of a head reads its manifest and that one chunk.

use crate::AppState;
use crate::activity::head_at;
use crate::chunks::{Part, Snapshot, read_chunk};
use crate::design::nodes_from;
use crate::docs::find_doc;
use crate::placeholders::{self, Sizing};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use omni_core::design::proto::PrototypeSettings;
use omni_core::design::tags;
use omni_core::design::{Node, NodeId};
use omni_core::hash::blake3_hex;
use omni_core::ops::IndexedNode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;

/// How many chunks are written or read at the same time.
const CONCURRENCY: usize = 16;

// The manifest ---------------------------------------------------------------------------

/// A board of a design's head and the chunk that holds it.
#[derive(Serialize, ToSchema)]
pub struct HeadBoard {
    /// The board's node ID.
    pub id: String,
    /// The BLAKE3 hash of the board's bytes; it changes when the board does.
    pub chunk: String,
}

/// A board of a page in a manifest.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ManifestBoard {
    /// The board's node ID.
    pub id: String,
    /// The name of the board's chunk: the BLAKE3 hash of its bytes, 64
    /// lowercase hexadecimal characters.
    pub chunk: String,
    /// The revision of each node of the board that is not at revision 1. A
    /// head keeps its revisions here, so that a node's chunk does not change
    /// with them; a version lists none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub revs: BTreeMap<String, u64>,
}

/// A page of a manifest, with its boards in order.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ManifestPage {
    /// The page's node ID.
    pub id: String,
    /// The name of the page's chunk, which holds the page without its boards.
    pub chunk: String,
    /// The page's own revision when it is not 1, as the boards list theirs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub revs: BTreeMap<String, u64>,
    pub boards: Vec<ManifestBoard>,
}

/// The chunks of a design version: its pages in order, each with its boards.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct Manifest {
    pub pages: Vec<ManifestPage>,
}

impl Manifest {
    /// Every chunk of the manifest, each once, pages before their boards.
    pub(crate) fn chunks(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        let mut chunks = Vec::new();
        for page in &self.pages {
            let boards = page.boards.iter().map(|board| board.chunk.as_str());
            for chunk in std::iter::once(page.chunk.as_str()).chain(boards) {
                if seen.insert(chunk) {
                    chunks.push(chunk);
                }
            }
        }
        chunks
    }

    /// The page that holds `board`, and the board's entry.
    fn locate(&self, board: &str) -> Option<(&ManifestPage, &ManifestBoard)> {
        self.pages.iter().find_map(|page| {
            page.boards
                .iter()
                .find(|entry| entry.id == board)
                .map(|entry| (page, entry))
        })
    }
}

// Splitting a design into chunks ---------------------------------------------------------

/// What `node` itself holds: its JSON without its children.
fn own_json(node: &Node) -> Value {
    let mut own = serde_json::to_value(node).expect("a node serializes to JSON");
    if let Some(object) = own.as_object_mut() {
        object.remove("children");
    }
    own
}

/// The node index of a subtree: every node with its ID, type, parent and own
/// content, in document order. The root has no parent.
pub(crate) fn index_subtree(root: &Node) -> Vec<IndexedNode> {
    fn walk(node: &Node, parent: Option<NodeId>, out: &mut Vec<IndexedNode>) {
        out.push(IndexedNode {
            id: *node.id(),
            kind: node.kind().as_str(),
            parent,
            content: own_json(node).to_string(),
        });
        for child in node.children() {
            walk(child, Some(*node.id()), out);
        }
    }
    let mut out = Vec::new();
    walk(root, None, &mut out);
    out
}

/// Gives a page's JSON the `flows` and the `prototype` the viewer reads: a
/// page written without them has no flows and the default settings, and the
/// settings carry every field, as `PrototypeSettings::outline` has them.
fn fill_prototype(page: &mut Value) {
    let Some(object) = page.as_object_mut() else {
        return;
    };
    object
        .entry("flows")
        .or_insert_with(|| Value::Array(Vec::new()));
    let settings = object
        .get("prototype")
        .and_then(|prototype| PrototypeSettings::from_value(prototype).ok())
        .unwrap_or_default();
    object.insert("prototype".to_string(), settings.outline());
}

/// The chunk of a page: the page without its boards, with the node index of
/// that one node.
fn page_part(page: &Node) -> Part {
    let own = own_json(page);
    Part::new(
        own.to_string(),
        vec![IndexedNode {
            id: *page.id(),
            kind: page.kind().as_str(),
            parent: None,
            content: own.to_string(),
        }],
    )
}

/// The chunk of a board: the board and its subtree, with their node index.
fn board_part(board: &Node) -> Part {
    let text = serde_json::to_string(board).expect("a node serializes to JSON");
    Part::new(text, index_subtree(board))
}

/// The manifest of a design and the chunks it lists.
fn split(pages: &[Node]) -> (Manifest, Vec<Part>) {
    let mut manifest = Manifest { pages: Vec::new() };
    let mut parts = Vec::new();
    for page in pages {
        let page_chunk = page_part(page);
        let mut entry = ManifestPage {
            id: page.id().to_string(),
            chunk: page_chunk.hash.clone(),
            revs: BTreeMap::new(),
            boards: Vec::new(),
        };
        parts.push(page_chunk);
        for board in page.children() {
            let part = board_part(board);
            entry.boards.push(ManifestBoard {
                id: board.id().to_string(),
                chunk: part.hash.clone(),
                revs: BTreeMap::new(),
            });
            parts.push(part);
        }
        manifest.pages.push(entry);
    }
    (manifest, parts)
}

fn manifest_text(manifest: &Manifest) -> String {
    serde_json::to_string(manifest).expect("a manifest serializes to JSON")
}

/// What the head of a design holds: its manifest as the head's own chunk and a
/// chunk for every page and board.
pub(crate) fn snapshot_of(pages: &[Node]) -> Snapshot {
    let (manifest, parts) = split(pages);
    Snapshot {
        html: manifest_text(&manifest),
        nodes: Vec::new(),
        parts,
    }
}

// Writing and reading chunks ------------------------------------------------------------

/// Runs `job` for every item, `CONCURRENCY` at a time, and fails with the
/// first error.
pub(crate) async fn each_concurrently<T, R, F, Fut>(
    items: Vec<T>,
    job: F,
) -> Result<Vec<R>, ApiError>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<R, ApiError>> + Send + 'static,
{
    each_concurrently_up_to(CONCURRENCY, items, job).await
}

/// Runs `job` for every item, at most `limit` at a time, and fails with the
/// first error. For jobs that hold more memory than a chunk read, such as
/// drawing a board, the caller picks a smaller limit than `CONCURRENCY`.
pub(crate) async fn each_concurrently_up_to<T, R, F, Fut>(
    limit: usize,
    items: Vec<T>,
    job: F,
) -> Result<Vec<R>, ApiError>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<R, ApiError>> + Send + 'static,
{
    let mut results = Vec::with_capacity(items.len());
    let mut items = items.into_iter();
    loop {
        let mut batch = tokio::task::JoinSet::new();
        for (index, item) in items.by_ref().take(limit.max(1)).enumerate() {
            let job = job.clone();
            batch.spawn(async move { (index, job(item).await) });
        }
        if batch.is_empty() {
            return Ok(results);
        }
        let mut done = Vec::new();
        while let Some(joined) = batch.join_next().await {
            let (index, result) = joined.map_err(|e| {
                eprintln!("omni-api: a storage task failed: {e}");
                ApiError::Unavailable
            })?;
            done.push((index, result?));
        }
        done.sort_by_key(|(index, _)| *index);
        results.extend(done.into_iter().map(|(_, result)| result));
    }
}

/// The chunks of a design: the manifest's, then every page and board. Each is
/// read from RustFS, `CONCURRENCY` at a time, by its name.
async fn read_chunks(
    state: &AppState,
    workspace_id: &str,
    names: &[&str],
) -> Result<HashMap<String, String>, ApiError> {
    let jobs: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    let state = state.clone();
    let workspace_id = workspace_id.to_string();
    let texts = each_concurrently(jobs.clone(), move |name| {
        let (state, workspace_id) = (state.clone(), workspace_id.clone());
        async move {
            let text = read_chunk(&state, &workspace_id, &name).await?;
            Ok((name, text))
        }
    })
    .await?;
    Ok(texts.into_iter().collect())
}

fn unavailable(what: &str, why: impl std::fmt::Display) -> ApiError {
    eprintln!("omni-api: {what} holds no design: {why}");
    ApiError::Unavailable
}

/// The pages that a saved design version holds, read from the chunk `chunk`
/// that is the version's manifest. A version saved before designs were split
/// into chunks holds all its pages in its one chunk, and reads the same way.
pub(crate) async fn read_pages(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
    what: &str,
) -> Result<Vec<Node>, ApiError> {
    let text = read_chunk(state, workspace_id, chunk).await?;
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        return read_whole_pages(&text, what);
    };
    assemble_pages(state, workspace_id, &manifest, what).await
}

/// The pages that a manifest lists, each with its boards, read from RustFS.
async fn assemble_pages(
    state: &AppState,
    workspace_id: &str,
    manifest: &Manifest,
    what: &str,
) -> Result<Vec<Node>, ApiError> {
    let texts = read_chunks(state, workspace_id, &manifest.chunks()).await?;
    let parse = |chunk: &str| -> Result<Value, ApiError> {
        let text = texts
            .get(chunk)
            .ok_or_else(|| unavailable(what, format!("the chunk {chunk} was not read")))?;
        serde_json::from_str(text).map_err(|e| unavailable(what, e))
    };
    let mut pages = Vec::with_capacity(manifest.pages.len());
    for entry in &manifest.pages {
        let mut page = parse(&entry.chunk)?;
        let boards = entry
            .boards
            .iter()
            .map(|board| parse(&board.chunk))
            .collect::<Result<Vec<_>, _>>()?;
        match page.as_object_mut() {
            Some(object) => object.insert("children".to_string(), Value::Array(boards)),
            None => return Err(unavailable(what, "a page chunk is no object")),
        };
        pages.push(page);
    }
    nodes_from(&Value::Array(pages)).map_err(|e| unavailable(what, e))
}

/// A design doc's head as an edit needs it: its manifest, its pages with their
/// boards, and the revision of every node.
pub(crate) struct HeadDesign {
    pub(crate) manifest: Manifest,
    pub(crate) pages: Vec<Node>,
    pub(crate) revisions: HashMap<NodeId, u64>,
}

/// The revisions that a manifest lists; a node it does not list is at
/// revision 1.
fn revisions_of(manifest: &Manifest) -> HashMap<NodeId, u64> {
    manifest
        .pages
        .iter()
        .flat_map(|page| {
            std::iter::once(&page.revs).chain(page.boards.iter().map(|board| &board.revs))
        })
        .flatten()
        .filter_map(|(id, rev)| id.parse().ok().map(|id| (id, *rev)))
        .collect()
}

/// A design doc's head, read from RustFS. A head in the older form, all its
/// pages in one chunk, reads with an empty manifest: every chunk of its next
/// head is new.
pub(crate) async fn read_head_design(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
    what: &str,
) -> Result<HeadDesign, ApiError> {
    let text = read_chunk(state, workspace_id, chunk).await?;
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        return Ok(HeadDesign {
            manifest: Manifest { pages: Vec::new() },
            pages: read_whole_pages(&text, what)?,
            revisions: HashMap::new(),
        });
    };
    let pages = assemble_pages(state, workspace_id, &manifest, what).await?;
    let revisions = revisions_of(&manifest);
    Ok(HeadDesign {
        manifest,
        pages,
        revisions,
    })
}

/// The revisions of the nodes of a subtree that are not 1.
fn revs_of(root: &Node, revisions: &HashMap<NodeId, u64>) -> BTreeMap<String, u64> {
    let mut revs = BTreeMap::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        match revisions.get(node.id()) {
            Some(rev) if *rev != 1 => {
                revs.insert(node.id().to_string(), *rev);
            }
            _ => {}
        }
        pending.extend(node.children());
    }
    revs
}

/// What writing a new head of a design takes: its manifest and the text of
/// it, the chunks that no chunk of the head before has the name of, and the
/// boards whose chunk changed.
pub(crate) struct HeadWrite {
    pub(crate) text: String,
    pub(crate) changed: Vec<Part>,
    /// `[{board, before, after}]`, as `doc_changes.boards` holds it.
    pub(crate) boards: Value,
}

impl HeadWrite {
    /// What the chunks of the write are listed by in the chunk index: the
    /// manifest, and the changed chunks as parts.
    pub(crate) fn into_snapshot(self) -> (Snapshot, Value) {
        let snapshot = Snapshot {
            html: self.text,
            nodes: Vec::new(),
            parts: self.changed,
        };
        (snapshot, self.boards)
    }
}

/// Plans the write of `pages` with their `revisions` as the head that follows
/// `before`: a chunk is written only when no chunk of `before` is named by its
/// hash, so a write stores the boards it changed and the manifest.
pub(crate) fn plan_head_write(
    before: &Manifest,
    pages: &[Node],
    revisions: &HashMap<NodeId, u64>,
) -> HeadWrite {
    let (mut manifest, parts) = split(pages);
    for (entry, page) in manifest.pages.iter_mut().zip(pages) {
        entry.revs = match revisions.get(page.id()) {
            Some(rev) if *rev != 1 => BTreeMap::from([(page.id().to_string(), *rev)]),
            _ => BTreeMap::new(),
        };
        for (board_entry, board) in entry.boards.iter_mut().zip(page.children()) {
            board_entry.revs = revs_of(board, revisions);
        }
    }
    let known: HashSet<&str> = before.chunks().into_iter().collect();
    let changed = parts
        .into_iter()
        .filter(|part| !known.contains(part.hash.as_str()))
        .collect();
    let boards = board_changes(before, &manifest);
    let text = manifest_text(&manifest);
    HeadWrite {
        text,
        changed,
        boards,
    }
}

/// The boards whose chunk differs between two manifests, as `{board, before,
/// after}`: the boards of `after` in order, then the ones it lacks.
fn board_changes(before: &Manifest, after: &Manifest) -> Value {
    let boards = |manifest: &Manifest| -> Vec<(String, String)> {
        manifest
            .pages
            .iter()
            .flat_map(|page| page.boards.iter())
            .map(|board| (board.id.clone(), board.chunk.clone()))
            .collect()
    };
    let (was, now) = (boards(before), boards(after));
    let was_by_id: HashMap<&str, &str> = was
        .iter()
        .map(|(id, chunk)| (id.as_str(), chunk.as_str()))
        .collect();
    let now_ids: HashSet<&str> = now.iter().map(|(id, _)| id.as_str()).collect();
    let mut changes = Vec::new();
    for (id, chunk) in &now {
        let before = was_by_id.get(id.as_str()).copied();
        if before != Some(chunk.as_str()) {
            changes.push(serde_json::json!({ "board": id, "before": before, "after": chunk }));
        }
    }
    for (id, chunk) in &was {
        if !now_ids.contains(id.as_str()) {
            changes.push(serde_json::json!({ "board": id, "before": chunk, "after": null }));
        }
    }
    Value::Array(changes)
}

/// The pages of a design version as the viewer reads them first: each page
/// as its own JSON with its boards under `children`, and each board as its own
/// JSON without its children, the placeholder that the viewer draws until it
/// reads the board. A placeholder whose width or height is not fixed has the
/// box that the board's content gives it, laid out against `sizing`
/// (`placeholders.rs`). The boards' chunks are read `CONCURRENCY` at a time
/// and each is cut down as soon as it is read, so the version's whole body is
/// never held at once.
pub(crate) async fn read_outline(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
    what: &str,
    sizing: &Sizing,
) -> Result<Vec<Value>, ApiError> {
    let text = read_chunk(state, workspace_id, chunk).await?;
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        let pages = read_whole_pages(&text, what)?;
        let mut outline = Vec::with_capacity(pages.len());
        for page in &pages {
            let boards = page.children();
            let texts = boards
                .iter()
                .map(|board| serde_json::to_string(board).expect("a node serializes to JSON"))
                .collect();
            let sizes = placeholders::boxes_of(sizing, texts).await;
            let mut json = own_json(page);
            fill_prototype(&mut json);
            let placeholders = boards
                .iter()
                .zip(sizes)
                .map(|(board, size)| {
                    let mut placeholder = own_json(board);
                    placeholders::apply(&mut placeholder, size);
                    placeholder
                })
                .collect();
            json["children"] = Value::Array(placeholders);
            outline.push(json);
        }
        return Ok(outline);
    };
    let boards: HashSet<String> = manifest
        .pages
        .iter()
        .flat_map(|page| page.boards.iter().map(|board| board.chunk.clone()))
        .collect();
    let names: Vec<String> = manifest
        .pages
        .iter()
        .flat_map(|page| {
            std::iter::once(page.chunk.clone()).chain(page.boards.iter().map(|b| b.chunk.clone()))
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let (state, workspace) = (state.clone(), workspace_id.to_string());
    let what = what.to_string();
    let (boards, sizing) = (Arc::new(boards), sizing.clone());
    let own = each_concurrently(names.clone(), move |name| {
        let (state, workspace, what) = (state.clone(), workspace.clone(), what.clone());
        let (boards, sizing) = (boards.clone(), sizing.clone());
        async move {
            let text = read_chunk(&state, &workspace, &name).await?;
            let mut json: Value = serde_json::from_str(&text).map_err(|e| unavailable(&what, e))?;
            if let Some(object) = json.as_object_mut() {
                object.remove("children");
            }
            if boards.contains(&name) {
                placeholders::size_placeholder(&sizing, &name, text, &mut json).await;
            }
            Ok(json)
        }
    })
    .await?;
    let own: HashMap<String, Value> = names.into_iter().zip(own).collect();
    let find = |chunk: &str| -> Result<Value, ApiError> {
        own.get(chunk)
            .cloned()
            .ok_or_else(|| unavailable("a version", format!("the chunk {chunk} was not read")))
    };
    let mut pages = Vec::with_capacity(manifest.pages.len());
    for entry in &manifest.pages {
        let mut page = find(&entry.chunk)?;
        fill_prototype(&mut page);
        let boards = entry
            .boards
            .iter()
            .map(|board| find(&board.chunk))
            .collect::<Result<Vec<_>, _>>()?;
        match page.as_object_mut() {
            Some(object) => object.insert("children".to_string(), Value::Array(boards)),
            None => return Err(unavailable("a version", "a page chunk is no object")),
        };
        pages.push(page);
    }
    Ok(pages)
}

/// The pages of a version in the older form, `{"pages": [...]}` in one chunk.
fn read_whole_pages(text: &str, what: &str) -> Result<Vec<Node>, ApiError> {
    let json: Value = serde_json::from_str(text).map_err(|e| unavailable(what, e))?;
    let pages = json
        .get("pages")
        .ok_or_else(|| unavailable(what, "it has no pages"))?;
    nodes_from(pages).map_err(|e| unavailable(what, e))
}

/// The boards of the design whose head is `chunk`, in page order, each with its
/// chunk: only the manifest is read. A head in the older form, every page in
/// the one chunk, has the hash of each board's JSON.
pub(crate) async fn head_boards(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
) -> Result<Vec<HeadBoard>, ApiError> {
    let what = format!("the chunk {chunk}");
    let text = read_chunk(state, workspace_id, chunk).await?;
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        let pages = read_whole_pages(&text, &what)?;
        return Ok(pages
            .iter()
            .flat_map(|page| page.children())
            .map(|board| HeadBoard {
                id: board.id().to_string(),
                chunk: blake3_hex(
                    serde_json::to_string(board)
                        .expect("a node serializes to JSON")
                        .as_bytes(),
                ),
            })
            .collect());
    };
    Ok(manifest
        .pages
        .iter()
        .flat_map(|page| &page.boards)
        .map(|board| HeadBoard {
            id: board.id.clone(),
            chunk: board.chunk.clone(),
        })
        .collect())
}

/// A board of a saved version and the page that holds it: the manifest and the
/// board's one chunk are read, and no other.
pub(crate) async fn read_board(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
    board: &str,
) -> Result<(String, Node), ApiError> {
    let what = format!("the chunk {chunk}");
    let text = read_chunk(state, workspace_id, chunk).await?;
    let unknown = || ApiError::Refused(StatusCode::NOT_FOUND, "unknown_board");
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        // The older form: every page is in the one chunk.
        let pages = read_whole_pages(&text, &what)?;
        return pages
            .iter()
            .find_map(|page| {
                page.children()
                    .iter()
                    .find(|node| node.id().to_string() == board)
                    .map(|node| (page.id().to_string(), node.clone()))
            })
            .ok_or_else(unknown);
    };
    let (page, entry) = manifest.locate(board).ok_or_else(unknown)?;
    let text = read_chunk(state, workspace_id, &entry.chunk).await?;
    let node = serde_json::from_str(&text).map_err(|e| unavailable(&what, e))?;
    Ok((page.id.clone(), node))
}

/// The IDs of the boards that a head of a design lists, in order: its manifest
/// is read, and no board.
pub(crate) async fn board_ids_of(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
) -> Result<Vec<String>, ApiError> {
    let what = format!("the chunk {chunk}");
    let text = read_chunk(state, workspace_id, chunk).await?;
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        // The older form: every page is in the one chunk.
        return Ok(read_whole_pages(&text, &what)?
            .iter()
            .flat_map(|page| page.children().iter().map(|node| node.id().to_string()))
            .collect());
    };
    Ok(manifest
        .pages
        .iter()
        .flat_map(|page| page.boards.iter().map(|board| board.id.clone()))
        .collect())
}

// Reading one board ---------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct BoardBody {
    pub doc_id: String,
    /// The page that holds the board.
    pub page_id: String,
    pub board_id: String,
    /// The board and its subtree in the tag format (docs/formats/design-tags.md),
    /// without its page.
    pub tags: String,
    /// The same board and subtree as JSON nodes, as the whole read of the
    /// version holds it under `pages`: the form the design viewer hands to the
    /// renderer.
    #[schema(value_type = Object)]
    pub node: Value,
}

pub(crate) fn unknown_board() -> ApiError {
    ApiError::Refused(StatusCode::NOT_FOUND, "unknown_board")
}

/// A board of a design doc must have a node ID to be one.
pub(crate) fn board_id(text: &str) -> Result<String, ApiError> {
    text.parse::<NodeId>()
        .map(|id| id.to_string())
        .map_err(|_| unknown_board())
}

/// Reads one board of a design doc's head, or as it was right after the write
/// named by `?at=`, in the tag format. Only the manifest and the board's one
/// chunk are read.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The design doc's UUID"),
        ("board_id" = String, Path, description = "The board's node ID"),
        ("at" = Option<i64>, Query, description = "Read the board as it was right after this write of the doc's activity (`seq`, from 1), instead of the head"),
    ),
    responses(
        (status = 200, description = "The board", body = BoardBody),
        (status = 400, description = "The doc is no design (`not_a_design`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, the doc has no write `at` (`not_found`), or the head or write has no such board: an unknown node, a page, a frame, a text (`unknown_board`)", body = ErrorBody),
    )
)]
pub async fn read_head_board(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id, board)): Path<(String, String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<BoardBody>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    if doc.kind != "design" {
        return Err(ApiError::bad_request("not_a_design"));
    }
    let chunk = match query.get("at") {
        Some(at) => Some(head_at(&mut tx, &doc_id, at).await?),
        None => doc.head.clone(),
    };
    tx.commit().await.map_err(ApiError::database)?;
    let board = board_id(&board)?;
    // A doc with no head holds no boards.
    let chunk = chunk.ok_or_else(unknown_board)?;
    let (page_id, node) = read_board(&state, &request.workspace_id, &chunk, &board).await?;
    Ok(Json(BoardBody {
        doc_id,
        page_id,
        board_id: board,
        tags: tags::print(&node),
        node: serde_json::to_value(&node).expect("a node serializes to JSON"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omni_core::design::tags;
    use omni_core::hash::blake3_hex;

    fn pages(source: &str) -> Vec<Node> {
        let mut next = 0u32;
        tags::parse_all(source)
            .expect("the tags parse")
            .into_iter()
            .map(|page| {
                page.assign_ids(|| {
                    next += 1;
                    format!("{next:08x}").parse().expect("a node ID")
                })
            })
            .collect()
    }

    fn node_named<'a>(pages: &'a [Node], name: &str) -> &'a Node {
        let mut pending: Vec<&Node> = pages.iter().collect();
        while let Some(node) = pending.pop() {
            if node.name() == name {
                return node;
            }
            pending.extend(node.children());
        }
        panic!("no node named {name}");
    }

    /// Two pages: P with the boards A (holding the text T) and B, Q with the
    /// board C.
    const TWO_PAGES: &str = r#"<Page name="P">
  <Board name="A" x={0} y={0} width={10} height={10}>
    <Text name="T" content="quokka" fontFamily="Inter" fontSize={12}/>
  </Board>
  <Board name="B" x={20} y={0} width={10} height={10}/>
</Page>
<Page name="Q">
  <Board name="C" x={0} y={0} width={10} height={10}/>
</Page>"#;

    /// The part named by `chunk`.
    fn part<'a>(parts: &'a [Part], chunk: &str) -> &'a Part {
        parts
            .iter()
            .find(|part| part.hash == chunk)
            .unwrap_or_else(|| panic!("no part is named {chunk}"))
    }

    #[test]
    fn a_design_splits_into_a_chunk_per_page_and_board_named_by_content() {
        let design = pages(TWO_PAGES);
        let id = |name: &str| node_named(&design, name).id().to_string();
        let (manifest, parts) = split(&design);

        let page_ids: Vec<&str> = manifest.pages.iter().map(|page| page.id.as_str()).collect();
        assert_eq!(page_ids, [id("P"), id("Q")], "the pages in order");
        let boards = |page: &ManifestPage| -> Vec<String> {
            page.boards.iter().map(|board| board.id.clone()).collect()
        };
        assert_eq!(boards(&manifest.pages[0]), [id("A"), id("B")]);
        assert_eq!(boards(&manifest.pages[1]), [id("C")]);

        assert_eq!(parts.len(), 5, "a chunk per page and per board");
        for part in &parts {
            assert_eq!(part.hash, blake3_hex(part.text.as_bytes()));
        }

        // The page's chunk holds no board; a board's chunk holds its subtree
        // and nothing of another board.
        let page = &part(&parts, &manifest.pages[0].chunk).text;
        assert!(
            !page.contains(&id("A")) && !page.contains("quokka"),
            "{page}"
        );
        let a = part(&parts, &manifest.pages[0].boards[0].chunk);
        assert!(
            a.text.contains(&id("T")) && a.text.contains("quokka"),
            "{}",
            a.text
        );
        let b = &part(&parts, &manifest.pages[0].boards[1].chunk).text;
        for needle in [id("A"), id("T"), "quokka".to_string()] {
            assert!(!b.contains(&needle), "B's chunk holds nothing of A: {b}");
        }

        // A board's node index is the board, its root without a parent, and
        // its subtree.
        let index: Vec<(String, Option<String>)> = a
            .nodes
            .iter()
            .map(|node| (node.id.to_string(), node.parent.map(|p| p.to_string())))
            .collect();
        assert_eq!(index, [(id("A"), None), (id("T"), Some(id("A")))]);

        // The same design splits the same way.
        let (again, _) = split(&design);
        assert_eq!(manifest_text(&again), manifest_text(&manifest));
    }

    /// A write draws a design's previews a few at a time (previews.rs), so
    /// the memory of the boards being drawn stays bounded.
    #[tokio::test]
    async fn each_concurrently_up_to_runs_at_most_the_limit_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let (seen, peak) = (running.clone(), most.clone());
        let results = each_concurrently_up_to(3, (0..10).collect(), move |n: usize| {
            let (seen, peak) = (seen.clone(), peak.clone());
            async move {
                let now = seen.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                seen.fetch_sub(1, Ordering::SeqCst);
                Ok(n * 2)
            }
        })
        .await
        .ok()
        .expect("every job succeeds");
        assert_eq!(results, (0..10).map(|n| n * 2).collect::<Vec<_>>());
        assert_eq!(most.load(Ordering::SeqCst), 3);
    }
}
