//! Pages, boards and chunked storage (US-030, Q23). A design doc holds pages
//! and a page holds boards, and a design version stores each of them as a
//! chunk of its own in RustFS, at `ws/{workspace_id}/chunks/{hash}`, named by
//! the BLAKE3 hash of its bytes:
//!
//! - a board's chunk is the board and its subtree as JSON;
//! - a page's chunk is the page itself as JSON, without its children;
//! - the version's own chunk is its manifest, which lists the chunk of every
//!   page and, below it, of every board, in order.
//!
//! A chunk's name follows from its content, so a page or a board that a later
//! version does not change has the same chunk in it, and is stored, indexed
//! and listed in the node index once, however many versions use it. The node
//! index of a chunk (`node_index`) lists only the nodes of that chunk, and a
//! board's root has no parent in it: a board that moves to another page keeps
//! its chunk, and the manifest says where it sits. The manifest is kept in the
//! `versions` row as well, so that a version's nodes and a comparison need no
//! read from RustFS.
//!
//! Reading one board of a version reads its manifest and that one chunk.

use crate::AppState;
use crate::design::nodes_from;
use crate::docs::{current_draft, ensure_loaded, find_doc, find_draft};
use crate::placeholders::{self, Sizing};
use crate::session::{ApiError, ErrorBody};
use crate::versions::{Part, Snapshot, chunk_of_version, read_chunk};
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use omni_core::design::tags;
use omni_core::design::{Node, NodeId};
use omni_core::ops::IndexedNode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;

/// How many chunks are written or read at the same time.
const CONCURRENCY: usize = 16;

// The manifest ---------------------------------------------------------------------------

/// A board of a page in a manifest.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ManifestBoard {
    /// The board's node ID.
    pub id: String,
    /// The name of the board's chunk: the BLAKE3 hash of its bytes, 64
    /// lowercase hexadecimal characters.
    pub chunk: String,
}

/// A page of a manifest, with its boards in order.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ManifestPage {
    /// The page's node ID.
    pub id: String,
    /// The name of the page's chunk, which holds the page without its boards.
    pub chunk: String,
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
            boards: Vec::new(),
        };
        parts.push(page_chunk);
        for board in page.children() {
            let part = board_part(board);
            entry.boards.push(ManifestBoard {
                id: board.id().to_string(),
                chunk: part.hash.clone(),
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

/// What a version of a design holds: its manifest as the version's own chunk
/// and a chunk for every page and board.
pub(crate) fn snapshot_of(pages: &[Node]) -> Snapshot {
    let (manifest, parts) = split(pages);
    Snapshot {
        html: manifest_text(&manifest),
        nodes: Vec::new(),
        parts,
        manifest: Some(manifest),
    }
}

/// The text of the version's own chunk for a design: its manifest.
pub(crate) fn content_of(pages: &[Node]) -> String {
    manifest_text(&split(pages).0)
}

// Revisions of the nodes of a draft ----------------------------------------------------

/// The revisions of a design that was just loaded from a version: every node
/// is at revision 1.
pub(crate) fn initial_revisions(pages: &[Node]) -> HashMap<NodeId, u64> {
    pages
        .iter()
        .flat_map(index_subtree)
        .map(|node| (node.id, 1))
        .collect()
}

/// The revisions after an import replaced the pages `before` with `after`: a
/// node whose own content changed is one revision further, a new node starts
/// at revision 1, and every other node keeps its revision. A child that is
/// added or moved does not change its parent's content, so it does not change
/// its revision either.
pub(crate) fn next_revisions(
    before: &[Node],
    revisions: &HashMap<NodeId, u64>,
    after: &[Node],
) -> HashMap<NodeId, u64> {
    let was: HashMap<NodeId, String> = before
        .iter()
        .flat_map(index_subtree)
        .map(|node| (node.id, node.content))
        .collect();
    after
        .iter()
        .flat_map(index_subtree)
        .map(|node| {
            let rev = match was.get(&node.id) {
                Some(content) => {
                    let rev = revisions.get(&node.id).copied().unwrap_or(1);
                    rev + u64::from(*content != node.content)
                }
                None => 1,
            };
            (node.id, rev)
        })
        .collect()
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
    let mut results = Vec::with_capacity(items.len());
    let mut items = items.into_iter();
    loop {
        let mut batch = tokio::task::JoinSet::new();
        for (index, item) in items.by_ref().take(CONCURRENCY).enumerate() {
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

/// A board of a saved version and the page that holds it: the manifest and the
/// board's one chunk are read, and no other.
async fn read_board(
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

// Reading one board ---------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct BoardBody {
    pub doc_id: String,
    /// The page that holds the board.
    pub page_id: String,
    pub board_id: String,
    /// The version that was read; `null` for a draft.
    pub version_id: Option<String>,
    /// The change set whose draft was read; left out for a version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changeset_id: Option<String>,
    /// The draft that was read; left out for a version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    /// The board and its subtree in the tag format (docs/formats/design-tags.md),
    /// without its page. For a draft, every tag carries `rev={n}`, the
    /// node's revision; a version's tags carry none.
    pub tags: String,
    /// The same board and subtree as JSON nodes, as the whole read of the
    /// version holds it under `pages`: the form the design viewer hands to the
    /// renderer.
    #[schema(value_type = Object)]
    pub node: Value,
}

fn unknown_board() -> ApiError {
    ApiError::Refused(StatusCode::NOT_FOUND, "unknown_board")
}

/// A board of a design doc must have a node ID to be one.
fn board_id(text: &str) -> Result<String, ApiError> {
    text.parse::<NodeId>()
        .map(|id| id.to_string())
        .map_err(|_| unknown_board())
}

/// Reads one board of the change set's draft of a design doc, unsaved changes
/// included, in the tag format with a `rev` on every tag.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/boards/{board_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The design doc's UUID"),
        ("board_id" = String, Path, description = "The board's node ID"),
    ),
    responses(
        (status = 200, description = "The board", body = BoardBody),
        (status = 400, description = "The doc is no design (`not_a_design`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace, or the ID names no board of the draft: an unknown node, a page, a frame, a text (`unknown_board`)", body = ErrorBody),
    )
)]
pub async fn read_draft_board(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, changeset_id, doc_id, board)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
) -> Result<Json<BoardBody>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    if doc.kind != "design" {
        return Err(ApiError::bad_request("not_a_design"));
    }
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let board = board_id(&board)?;

    let slot = state.drafts.slot(&request.workspace_id, &draft.id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = current_draft(&mut tx, &draft.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    ensure_loaded(&state, &request.workspace_id, &draft, &mut held).await?;
    let loaded = held.as_ref().expect("the draft is loaded");
    let (page, node) = loaded
        .design_doc
        .pages()
        .iter()
        .find_map(|page| {
            page.children()
                .iter()
                .find(|node| node.id().to_string() == board)
                .map(|node| (page, node))
        })
        .ok_or_else(unknown_board)?;
    let revision = |id: &NodeId| loaded.design_doc.revision(id);
    let tags = tags::print_with(node, Some(&revision));
    Ok(Json(BoardBody {
        doc_id,
        page_id: page.id().to_string(),
        board_id: board,
        version_id: None,
        changeset_id: Some(changeset_id),
        draft_id: Some(draft.id),
        tags,
        node: serde_json::to_value(node).expect("a node serializes to JSON"),
    }))
}

/// Reads one board of a design doc's mainline version, or of the version
/// named by `?version=`, in the tag format. Only the manifest and the board's
/// one chunk are read.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The design doc's UUID"),
        ("board_id" = String, Path, description = "The board's node ID"),
        ("version" = Option<String>, Query, description = "The version to read instead of the mainline version"),
    ),
    responses(
        (status = 200, description = "The board", body = BoardBody),
        (status = 400, description = "The doc is no design (`not_a_design`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, the doc has no such version (`not_found`), or the version has no such board: an unknown node, a page, a frame, a text (`unknown_board`)", body = ErrorBody),
        (status = 409, description = "The doc has no mainline version yet, so only its own change set has it (`no_mainline_version`)", body = ErrorBody),
    )
)]
pub async fn read_board_of_version(
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
    let version_id = match query.get("version") {
        Some(version) => parse_uuid(version).ok_or(ApiError::NotFound)?,
        None => doc
            .mainline_version_id
            .clone()
            .ok_or_else(|| ApiError::conflict("no_mainline_version"))?,
    };
    let chunk = chunk_of_version(&mut tx, &doc_id, &version_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let board = board_id(&board)?;
    // A version that removes the doc holds no boards.
    let chunk = chunk.ok_or_else(unknown_board)?;
    let (page_id, node) = read_board(&state, &request.workspace_id, &chunk, &board).await?;
    Ok(Json(BoardBody {
        doc_id,
        page_id,
        board_id: board,
        version_id: Some(version_id),
        changeset_id: None,
        draft_id: None,
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

    const DESIGN: &str = r#"<Page name="P">
  <Board name="A" x={0} y={0} width={10} height={10}>
    <Text name="T" content="one" fontFamily="Inter" fontSize={12}/>
  </Board>
  <Board name="B" x={20} y={0} width={10} height={10}/>
</Page>"#;

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

    #[test]
    fn a_design_splits_into_a_chunk_per_page_and_board_named_by_content() {
        let design = pages(DESIGN);
        let (manifest, parts) = split(&design);
        assert_eq!(manifest.pages.len(), 1);
        assert_eq!(manifest.pages[0].boards.len(), 2);
        assert_eq!(parts.len(), 3);
        for part in &parts {
            assert_eq!(part.hash, blake3_hex(part.text.as_bytes()));
        }
        // The page's chunk holds no board; a board's chunk holds its subtree.
        assert!(!parts[0].text.contains("one"));
        assert!(parts[1].text.contains("one"));
        // The same design splits the same way.
        let (again, _) = split(&design);
        assert_eq!(manifest_text(&again), manifest_text(&manifest));
        // A board's root has no parent in its chunk.
        assert!(parts[1].nodes[0].parent.is_none());
        assert_eq!(parts[1].nodes.len(), 2);
    }

    #[test]
    fn revisions_rise_for_the_nodes_an_import_changes_only() {
        let before = pages(DESIGN);
        let revisions = initial_revisions(&before);
        assert!(revisions.values().all(|rev| *rev == 1));

        // Importing the same design changes nothing.
        let same = next_revisions(&before, &revisions, &before);
        assert_eq!(same, revisions);

        // Another text changes the text and no one else.
        let changed = pages(&DESIGN.replace("one", "two"));
        let after = next_revisions(&before, &revisions, &changed);
        let text = *node_named(&changed, "T").id();
        assert_eq!(after[&text], 2);
        assert_eq!(after[node_named(&changed, "A").id()], 1);
        assert_eq!(after[node_named(&changed, "P").id()], 1);
    }
}
