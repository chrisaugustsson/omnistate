// Where the comment pins of a design's boards go (US-119).
//
// A board that the viewer has not read is drawn from its preview, which has no
// nodes, so the viewer cannot ask the renderer where a commented node is. A
// reviewer who sees the board's content still has to see its pins, so the viewer
// asks for the boxes of the commented nodes here: each laid out as the board's
// preview is, from the board's top-left corner, so that the viewer can put a pin
// on the node wherever the board is on the canvas and whatever the zoom.

use crate::AppState;
use crate::boards::{self, each_concurrently};
use crate::chunks::read_chunk;
use crate::design;
use crate::placeholders::{self, Sizing};
use crate::session::ApiError;
use omni_core::design::NodeAddress;
use omni_render::view::nodes_on_board;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

/// Where a node sits on its board.
#[derive(Serialize, ToSchema)]
pub struct NodeBox {
    /// The ID of the board that holds the node.
    pub board: String,
    /// The width of the board, which says how the box scales with the board.
    pub board_width: f32,
    /// The left of the node's box, from the board's left.
    pub x: f32,
    /// The top of the node's box, from the board's top.
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// The node's layer name.
    pub name: String,
}

/// The node of the board that `address` is on: the node itself, or its instance.
fn placed_node(address: &str) -> &str {
    address.split('/').next().unwrap_or_default()
}

/// The boxes of the nodes `wanted` (node addresses) of the head `head` of a
/// design in the project `project_id`, by address. An address the head does not
/// have, or on a board that cannot be laid out, is left out.
pub(crate) async fn of_nodes(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    head: &str,
    wanted: &[&str],
) -> Result<BTreeMap<String, NodeBox>, ApiError> {
    let boards: HashMap<String, String> = boards::head_boards(state, workspace_id, head)
        .await?
        .into_iter()
        .map(|board| (board.chunk, board.id))
        .collect();
    let chunks: Vec<&str> = boards.keys().map(String::as_str).collect();
    let nodes: Vec<&str> = wanted.iter().map(|address| placed_node(address)).collect();
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found: Vec<(String, String)> = sqlx::query_as(
        "SELECT node, chunk FROM node_index \
         WHERE chunk = ANY($1::text[]) AND node = ANY($2::text[])",
    )
    .bind(&chunks)
    .bind(&nodes)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let holders: HashMap<String, String> = found.into_iter().collect();

    let mut by_chunk: HashMap<&str, Vec<String>> = HashMap::new();
    for address in wanted {
        if let Some(chunk) = holders.get(placed_node(address)) {
            by_chunk.entry(chunk).or_default().push(address.to_string());
        }
    }
    if by_chunk.is_empty() {
        return Ok(BTreeMap::new());
    }

    let version = design::system_version_of_project(state, workspace_id, project_id).await?;
    let system = design::system_head_of_project(state, workspace_id, project_id).await?;
    let (tokens, components) = (system.tokens_json(), system.components_json());
    let sizing = Sizing::without_stored(version.as_deref(), tokens.as_ref(), &components).await;
    let groups: Vec<(String, String, Vec<String>)> = by_chunk
        .into_iter()
        .filter_map(|(chunk, addresses)| {
            let board = boards.get(chunk)?.clone();
            Some((chunk.to_string(), board, addresses))
        })
        .collect();
    let (state, workspace_id) = (state.clone(), workspace_id.to_string());
    let laid = each_concurrently(groups, move |(chunk, board, addresses)| {
        let (state, workspace_id, sizing) = (state.clone(), workspace_id.clone(), sizing.clone());
        async move {
            // A head in the older form has no chunk for each board.
            let Ok(text) = read_chunk(&state, &workspace_id, &chunk).await else {
                return Ok(Vec::new());
            };
            let system = sizing.system();
            let boxes = tokio::task::spawn_blocking(move || {
                lay_out(&text, &board, addresses, system.as_deref())
            })
            .await;
            Ok(boxes.unwrap_or_default())
        }
    })
    .await?;
    Ok(laid.into_iter().flatten().collect())
}

/// The boxes of the nodes at `addresses` on `board`, the board in `text`.
fn lay_out(
    text: &str,
    board: &str,
    addresses: Vec<String>,
    system: Option<&omni_render::system::DesignSystem>,
) -> Vec<(String, NodeBox)> {
    let Ok(node) = omni_render::nesting::read(text) else {
        return Vec::new();
    };
    let (addresses, parsed): (Vec<String>, Vec<NodeAddress>) = addresses
        .into_iter()
        .filter_map(|address| {
            let parsed = address.parse().ok()?;
            Some((address, parsed))
        })
        .unzip();
    let Some((board_width, nodes)) = nodes_on_board(node, &placeholders::assets(), system, &parsed)
    else {
        return Vec::new();
    };
    addresses
        .into_iter()
        .zip(nodes)
        .filter_map(|(address, found)| {
            let (rect, name) = found?;
            let node_box = NodeBox {
                board: board.to_string(),
                board_width,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                name,
            };
            Some((address, node_box))
        })
        .collect()
}
