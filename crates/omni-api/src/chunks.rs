//! Chunks: the content of a doc, stored once in RustFS as a chunk named by the
//! BLAKE3 hash of its bytes (`ws/{workspace_id}/chunks/{chunk}`). A doc's head
//! is the name of a chunk (src/docs.rs). Every chunk has a row in the chunk
//! index, the tenant table `chunks`, keyed by the workspace and the hash, and
//! its nodes are listed in the tenant table `node_index`: the ID, type, parent
//! and own-content fingerprint of each, in document order. Both tables are
//! append-only: the API role has no UPDATE privilege on them. Comments read the
//! node index to find out whether the node they point to is still in the head.
//!
//! A design head is a manifest (src/boards.rs): its own chunk lists the chunks
//! of its pages and boards, each stored and indexed like any other chunk, so
//! that a page or a board that a write does not change is shared with the head
//! before it.

use crate::AppState;
use crate::boards::{self, Manifest};
use crate::docs::Tx;
use crate::session::ApiError;
use crate::workspace::sha256_hex;
use omni_core::hash::blake3_hex;
use omni_core::ops::IndexedNode;
use std::collections::HashSet;

// Chunks in RustFS ------------------------------------------------------------------

fn chunk_key(workspace_id: &str, chunk: &str) -> String {
    format!("ws/{workspace_id}/chunks/{chunk}")
}

/// Stores `text` as the chunk `chunk`, which is the BLAKE3 hash of the text.
async fn put_chunk(
    bucket: &s3::Bucket,
    workspace_id: &str,
    chunk: &str,
    text: &str,
) -> Result<(), ApiError> {
    let stored = bucket
        .put_object(chunk_key(workspace_id, chunk), text.as_bytes())
        .await
        .map_err(|e| {
            eprintln!("omni-api: cannot store chunk {chunk}: {e}");
            ApiError::Unavailable
        })?;
    if stored.status_code() != 200 {
        eprintln!(
            "omni-api: cannot store chunk {chunk}: storage answered {}",
            stored.status_code()
        );
        return Err(ApiError::Unavailable);
    }
    Ok(())
}

/// A chunk that a snapshot lists besides its own: the page or board of a
/// design, with the node index of the nodes it holds.
pub(crate) struct Part {
    /// The name of the chunk: the BLAKE3 hash of `text`.
    pub(crate) hash: String,
    pub(crate) text: String,
    pub(crate) nodes: Vec<IndexedNode>,
}

impl Part {
    pub(crate) fn new(text: String, nodes: Vec<IndexedNode>) -> Self {
        Self {
            hash: blake3_hex(text.as_bytes()),
            text,
            nodes,
        }
    }
}

/// Stores a snapshot's chunks: its own, named by the hash of its text, and the
/// parts it lists. A chunk is named by its content, so one that the chunk
/// index lists is stored already and is not written again, and a chunk without
/// a head is harmless. Returns the name of the snapshot's own chunk.
pub(crate) async fn store_snapshot(
    state: &AppState,
    workspace_id: &str,
    snapshot: &Snapshot,
) -> Result<String, ApiError> {
    let chunk = blake3_hex(snapshot.html.as_bytes());
    let mut texts: Vec<(&str, &str)> = vec![(chunk.as_str(), snapshot.html.as_str())];
    texts.extend(
        snapshot
            .parts
            .iter()
            .map(|part| (part.hash.as_str(), part.text.as_str())),
    );
    let mut seen = HashSet::new();
    texts.retain(|(hash, _)| seen.insert(*hash));
    let names: Vec<&str> = texts.iter().map(|(hash, _)| *hash).collect();

    let mut tx = state.workspace_tx(workspace_id).await?;
    let known: HashSet<String> =
        sqlx::query_scalar("SELECT hash FROM chunks WHERE hash = ANY($1::text[])")
            .bind(&names)
            .fetch_all(&mut *tx)
            .await
            .map_err(ApiError::database)?
            .into_iter()
            .collect();
    tx.commit().await.map_err(ApiError::database)?;

    let missing: Vec<(String, String)> = texts
        .into_iter()
        .filter(|(hash, _)| !known.contains(*hash))
        .map(|(hash, text)| (hash.to_string(), text.to_string()))
        .collect();
    let bucket = state.bucket.clone();
    let workspace = workspace_id.to_string();
    boards::each_concurrently(missing, move |(hash, text)| {
        let (bucket, workspace) = (bucket.clone(), workspace.clone());
        async move { put_chunk(&bucket, &workspace, &hash, &text).await }
    })
    .await?;
    Ok(chunk)
}

/// Stores `text` as a chunk of its own and returns its name, the BLAKE3 hash of
/// the text. It does not consult the chunk index, so it needs no database
/// connection: a caller that holds a row lock stores its chunk with it.
pub(crate) async fn store_text(
    state: &AppState,
    workspace_id: &str,
    text: &str,
) -> Result<String, ApiError> {
    let chunk = blake3_hex(text.as_bytes());
    put_chunk(&state.bucket, workspace_id, &chunk, text).await?;
    Ok(chunk)
}

/// The HTML that a chunk holds.
pub(crate) async fn read_chunk(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
) -> Result<String, ApiError> {
    let unavailable = |why: String| {
        eprintln!("omni-api: cannot read chunk {chunk}: {why}");
        ApiError::Unavailable
    };
    let object = state
        .bucket
        .get_object(chunk_key(workspace_id, chunk))
        .await
        .map_err(|e| unavailable(e.to_string()))?;
    if object.status_code() != 200 {
        return Err(unavailable(format!(
            "storage answered {}",
            object.status_code()
        )));
    }
    String::from_utf8(object.to_vec()).map_err(|e| unavailable(e.to_string()))
}

/// The manifest that a design doc's head chunk holds, `None` for a head in the
/// older form, all the pages in one chunk, and for a doc that has no head.
pub(crate) async fn head_manifest(
    state: &AppState,
    workspace_id: &str,
    head: Option<&str>,
) -> Result<Option<Manifest>, ApiError> {
    let Some(head) = head else {
        return Ok(None);
    };
    let text = read_chunk(state, workspace_id, head).await?;
    Ok(serde_json::from_str(&text).ok())
}

// Indexing -----------------------------------------------------------------------------

/// What a head holds: the doc's HTML and its node index. A design head holds
/// its manifest as `html` and the chunks it
/// lists as `parts`, each with its own node index.
pub(crate) struct Snapshot {
    pub(crate) html: String,
    pub(crate) nodes: Vec<IndexedNode>,
    pub(crate) parts: Vec<Part>,
}

impl Snapshot {
    /// A head whose content is the one chunk `html`.
    pub(crate) fn new(html: String, nodes: Vec<IndexedNode>) -> Self {
        Self {
            html,
            nodes,
            parts: Vec::new(),
        }
    }
}

/// Lists the chunks in the chunk index, and the nodes of those that were not
/// in it, in the node index. A chunk that is listed has these very nodes
/// indexed already: its name is the hash of the content they come from.
async fn index_chunks(
    tx: &mut Tx,
    workspace_id: &str,
    chunks: &[(&str, &[IndexedNode])],
) -> Result<(), ApiError> {
    let names: Vec<&str> = chunks.iter().map(|(chunk, _)| *chunk).collect();
    let added: HashSet<String> = sqlx::query_scalar(
        "INSERT INTO chunks (workspace_id, hash) \
         SELECT $1::uuid, hash FROM unnest($2::text[]) AS c (hash) \
         ON CONFLICT (workspace_id, hash) DO NOTHING \
         RETURNING hash",
    )
    .bind(workspace_id)
    .bind(&names)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .into_iter()
    .collect();
    let mut indexed_chunks = Vec::new();
    let mut ids = Vec::new();
    let mut kinds = Vec::new();
    let mut parents = Vec::new();
    let mut fingerprints = Vec::new();
    let mut ordinals = Vec::new();
    let mut done = HashSet::new();
    for (chunk, nodes) in chunks {
        if !added.contains(*chunk) || !done.insert(*chunk) {
            continue;
        }
        for (ordinal, node) in nodes.iter().enumerate() {
            indexed_chunks.push(*chunk);
            ids.push(node.id.to_string());
            kinds.push(node.kind);
            parents.push(node.parent.map(|parent| parent.to_string()));
            fingerprints.push(sha256_hex(&node.content));
            ordinals.push(ordinal as i32);
        }
    }
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO node_index (workspace_id, chunk, node, type, parent, fingerprint, ordinal) \
         SELECT $1::uuid, chunk, node, type, parent, fingerprint, ordinal \
         FROM unnest($2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::int[]) \
              AS n (chunk, node, type, parent, fingerprint, ordinal) \
         ON CONFLICT (workspace_id, chunk, node) DO NOTHING",
    )
    .bind(workspace_id)
    .bind(&indexed_chunks)
    .bind(&ids)
    .bind(&kinds)
    .bind(&parents)
    .bind(&fingerprints)
    .bind(&ordinals)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Lists the chunks of a snapshot (its own, named `chunk`, and its parts) in
/// the chunk index, and the nodes of those that are new in the node index.
pub(crate) async fn index_snapshot(
    tx: &mut Tx,
    workspace_id: &str,
    chunk: &str,
    snapshot: &Snapshot,
) -> Result<(), ApiError> {
    let mut chunks: Vec<(&str, &[IndexedNode])> = vec![(chunk, &snapshot.nodes)];
    chunks.extend(
        snapshot
            .parts
            .iter()
            .map(|part| (part.hash.as_str(), part.nodes.as_slice())),
    );
    index_chunks(tx, workspace_id, &chunks).await
}
