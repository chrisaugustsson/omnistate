//! A node in tag form as a page to draw: what the comments of a design show
//! of a node that the design no longer has (US-113).

use omni_core::design::{Node, NodeId, tags};
use serde_json::{Value, json};

/// The ID of the page that holds the snapshot, and of the board that holds
/// a node that is not a board.
const PAGE_ID: &str = "sn4p0ag3";
const BOARD_ID: &str = "sn4p5h0t";

/// The page, as JSON in the form `scene_load` reads, that shows the node in
/// `tags` (one tag and its subtree, as the API gives the snapshot of a comment):
/// the page holds the node when it is a board, else a board of its own
/// size that holds the node. A page in `tags` is shown as it is.
pub fn page_of_tags(source: &str) -> Result<String, String> {
    let mut spare = 0u32;
    let node: Node = tags::parse(source)
        .map_err(|error| error.to_string())?
        .try_assign_ids(|| {
            // The tags of a snapshot carry the IDs of their nodes; one that has
            // none gets an ID no other node has.
            spare += 1;
            format!("sn{spare:06}")
                .parse::<NodeId>()
                .expect("a spare ID is 8 base32 characters")
        })?;
    let node = serde_json::to_value(&node).map_err(|error| error.to_string())?;
    let page = match node["type"].as_str() {
        Some("page") => node,
        Some("board") => page_with(node),
        _ => page_with(json!({
            "id": BOARD_ID,
            "type": "board",
            "name": "Snapshot",
            "x": 0,
            "y": 0,
            "width": "hug",
            "height": "hug",
            "layout": {"mode": "flex", "direction": "column"},
            "children": [node],
        })),
    };
    Ok(page.to_string())
}

fn page_with(board: Value) -> Value {
    json!({"id": PAGE_ID, "type": "page", "name": "Snapshot", "children": [board]})
}
