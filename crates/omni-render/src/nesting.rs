//! How deep a design may nest. Reading, resolving, laying out, painting and
//! dropping a design all recurse into its nodes, so the call stack grows with
//! every level. In the WebAssembly build that stack is twofold: the module's
//! own stack in linear memory (`STACK_SIZE` in `build.rs`), at about 5 KB a
//! level in a release build, and the browser's JavaScript stack, through which
//! every call that may unwind passes (Chrome runs out of it at about 250
//! levels). A design nested deeper than [`MAX_NESTING`] levels is refused with
//! an error that says so, rather than overflowing either stack.
//!
//! - A page read from JSON is at most 63 levels deep: `serde_json` stops at
//!   128 nested arrays and objects, two for each level ([`read`] says what
//!   that error means).
//! - Expanding instances deepens a page, by the depth of each component it
//!   places. Resolving cuts a page off at [`MAX_NESTING`] levels: the node at
//!   the last level that holds more is drawn as an error placeholder without
//!   its children, and listed with `Missing::Nesting`.

use omni_core::design::Node;

/// The most levels of nodes below a page that the renderer draws: a board is
/// one level, a frame inside it two. The Penpot file tirebud-ordrar nests 12
/// levels deep with its instances expanded; this leaves room for ten times
/// that, and stays well inside both stacks.
pub const MAX_NESTING: usize = 128;

/// How many levels of nodes `node` holds: 1 for a node without children.
/// It does not recurse, so it is safe on a design of any depth.
pub fn depth(node: &Node) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(node, 1)];
    while let Some((node, level)) = pending.pop() {
        deepest = deepest.max(level);
        pending.extend(node.children().iter().map(|child| (child, level + 1)));
    }
    deepest
}

/// The reason a design that nests deeper than [`MAX_NESTING`] levels below
/// its page is not drawn; `at` names where, if that is known.
pub fn too_deep(at: Option<&str>) -> String {
    let place = at.map_or_else(String::new, |node| format!(" at node {node}"));
    format!(
        "the design nests more than {MAX_NESTING} levels deep{place}, more than the renderer \
         draws: flatten the frames or instances there"
    )
}

/// Reads `json` as a design node. A design too deeply nested to read gets the
/// reason [`too_deep`] gives in place of `serde_json`'s.
pub fn read(json: &str) -> Result<Node, String> {
    serde_json::from_str(json).map_err(|error| {
        let reason = error.to_string();
        if reason.starts_with("recursion limit exceeded") {
            too_deep(None)
        } else {
            reason
        }
    })
}
