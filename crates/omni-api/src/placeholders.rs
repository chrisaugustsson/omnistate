// The boxes of the placeholders of a design version's outline (US-058).
//
// The outline lists each board without its content, and the design viewer
// draws it as a placeholder until the board is read. A board whose width or
// height hugs its content (or fills, or is a token) says nothing of its size
// once its content is left out: the renderer would lay the placeholder out at
// its padding, 0 x 0 without one, so the page would jump as boards load, the
// view would be fitted to the wrong boxes, and a board of no size is never in
// view and never read. So the API lays each such board out with its content,
// once, as the renderer does (omni-render's `view::board_box`, with its
// bundled font), and gives its placeholder that box as a fixed width and
// height. Sizes that are already fixed are kept as they are.
//
// Nothing about the box is stored: the outline reads every board's chunk
// anyway, and a chunk never changes (it is named by the hash of its bytes),
// so the box is kept in memory by chunk and design system version, and a
// board is laid out once for the life of the API process.

use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::board_box;
use serde_json::{Number, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// How many boxes are kept before the memory is cleared.
const MOST_KEPT: usize = 200_000;

/// A board's chunk and the ID of the design system version it is laid out
/// against.
type BoxKey = (String, String);

/// The boxes laid out so far, their width and height.
static BOXES: LazyLock<Mutex<HashMap<BoxKey, (f32, f32)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

thread_local! {
    /// The renderer's bundled font, which stands in for every font family.
    static ASSETS: Assets = Assets::new();
}

/// What the placeholders of one outline are laid out against: the design
/// system version it recorded, as the viewer resolves its boards.
#[derive(Clone)]
pub(crate) struct Sizing {
    system: Option<Arc<DesignSystem>>,
    /// The design system version's ID, or empty without one.
    key: String,
}

impl Sizing {
    /// The sizing for an outline whose design system version is
    /// `design_system_version_id`, with its `tokens` (a token tree as JSON)
    /// and `components`, read the way the viewer reads them.
    pub(crate) async fn new(
        design_system_version_id: Option<&str>,
        tokens: Option<&Value>,
        components: &[Value],
    ) -> Self {
        let key = design_system_version_id.unwrap_or_default().to_string();
        if tokens.is_none() && components.is_empty() {
            return Self { system: None, key };
        }
        let mut json = tokens.cloned().unwrap_or_else(|| json!({}));
        if let Some(object) = json.as_object_mut() {
            object.insert("$components".to_string(), Value::Array(components.to_vec()));
        }
        let system = tokio::task::spawn_blocking(move || DesignSystem::from_json(&json).ok())
            .await
            .ok()
            .flatten()
            .map(Arc::new);
        Self { system, key }
    }
}

fn is_fixed(size: Option<&Value>) -> bool {
    size.and_then(|size| size.get("fixed"))
        .is_some_and(Value::is_number)
}

/// Whether the placeholder of `board` (a board's own JSON) needs the box that
/// its content gives it.
pub(crate) fn needs_box(board: &Value) -> bool {
    !is_fixed(board.get("width")) || !is_fixed(board.get("height"))
}

/// A length as JSON: whole lengths as integers, others to a hundredth.
fn length(value: f32) -> Value {
    let rounded = (f64::from(value) * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        json!(rounded as i64)
    } else {
        Number::from_f64(rounded).map_or(Value::Null, Value::Number)
    }
}

/// Gives `placeholder` (a board's own JSON) the size `(width, height)` in
/// place of each of its sizes that is not fixed.
fn sized(placeholder: &mut Value, (width, height): (f32, f32)) {
    let Some(object) = placeholder.as_object_mut() else {
        return;
    };
    for (key, value) in [("width", width), ("height", height)] {
        if !is_fixed(object.get(key)) {
            object.insert(key.to_string(), json!({"fixed": length(value)}));
        }
    }
}

/// Gives `placeholder`, the own JSON of the board stored in the chunk `chunk`
/// as `text` (the board with its content), the box of that content in place of
/// its sizes that are not fixed. A board that cannot be laid out keeps its
/// sizes.
pub(crate) async fn size_placeholder(
    sizing: &Sizing,
    chunk: &str,
    text: String,
    placeholder: &mut Value,
) {
    if !needs_box(placeholder) {
        return;
    }
    let key = (chunk.to_string(), sizing.key.clone());
    let known = BOXES.lock().ok().and_then(|boxes| boxes.get(&key).copied());
    let size = match known {
        Some(size) => Some(size),
        None => {
            let system = sizing.system.clone();
            let laid = tokio::task::spawn_blocking(move || lay_out(&text, system.as_deref()))
                .await
                .ok()
                .flatten();
            if let (Some(size), Ok(mut boxes)) = (laid, BOXES.lock()) {
                if boxes.len() >= MOST_KEPT {
                    boxes.clear();
                }
                boxes.insert(key, size);
            }
            laid
        }
    };
    if let Some(size) = size {
        sized(placeholder, size);
    }
}

/// The width and height of the board in `text` (a board with its content).
fn lay_out(text: &str, system: Option<&DesignSystem>) -> Option<(f32, f32)> {
    let board = omni_render::nesting::read(text).ok()?;
    let laid = ASSETS.with(|assets| board_box(board, assets, system))?;
    (laid.width.is_finite() && laid.height.is_finite()).then_some((laid.width, laid.height))
}

/// Gives the placeholders of `boards` (boards with their content, of a version
/// stored before designs were split into chunks) their boxes, in the order of
/// `boards`; `None` for a board that cannot be laid out.
pub(crate) async fn boxes_of(sizing: &Sizing, boards: Vec<String>) -> Vec<Option<(f32, f32)>> {
    let system = sizing.system.clone();
    tokio::task::spawn_blocking(move || {
        boards
            .iter()
            .map(|text| lay_out(text, system.as_deref()))
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Gives `placeholder` the size `size` (from [`boxes_of`]) in place of its
/// sizes that are not fixed.
pub(crate) fn apply(placeholder: &mut Value, size: Option<(f32, f32)>) {
    if let Some(size) = size {
        sized(placeholder, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_lengths_are_integers_and_others_are_kept_to_a_hundredth() {
        assert_eq!(length(140.0), json!(140));
        assert_eq!(length(2361.999), json!(2362));
        assert_eq!(length(97.5), json!(97.5));
    }

    #[test]
    fn only_the_sizes_that_are_not_fixed_are_replaced() {
        let mut board = json!({"id": "b0000001", "width": {"fixed": 1800}, "height": "hug"});
        assert!(needs_box(&board));
        sized(&mut board, (1799.5, 640.25));
        assert_eq!(board["width"], json!({"fixed": 1800}));
        assert_eq!(board["height"], json!({"fixed": 640.25}));
        assert!(!needs_box(&board));
        let mut no_size = json!({"id": "b0000002"});
        sized(&mut no_size, (12.0, 8.0));
        assert_eq!(no_size["width"], json!({"fixed": 12}));
        assert_eq!(no_size["height"], json!({"fixed": 8}));
    }

    #[test]
    fn a_board_that_hugs_its_content_gets_the_size_of_that_content() {
        let board = json!({
            "id": "b0000001", "type": "board", "name": "Status", "x": 0, "y": 0,
            "width": "hug", "height": {"fixed": 32},
            "layout": {"mode": "flex", "padding": {"left": 12, "right": 12, "top": 0, "bottom": 0}},
            "children": [{
                "id": "f0000001", "type": "frame", "name": "Dot",
                "width": {"fixed": 60}, "height": {"fixed": 20}, "layout": {"mode": "none"}
            }]
        });
        assert_eq!(lay_out(&board.to_string(), None), Some((84.0, 32.0)));
    }
}
