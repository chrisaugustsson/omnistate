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
// A chunk never changes (it is named by the hash of its bytes), so a box is
// kept by chunk and design system version. Writing a design lays out the
// boards it changed in the background and stores their boxes in RustFS, one
// object per design system version (`ws/{workspace_id}/boxes/{version}`), so
// that the first outline after a restart reads them instead of laying every
// board out. A box that is not stored (the design system changed since, or the
// write has not finished) is laid out when the outline is read, once for the
// life of the API process.

use crate::AppState;
use crate::boards::each_concurrently;
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
    /// The boxes stored for this version, by chunk.
    stored: Arc<HashMap<String, (f32, f32)>>,
}

impl Sizing {
    /// The sizing for an outline whose design system version is
    /// `design_system_version_id`, with its `tokens` (a token tree as JSON)
    /// and `components`, read the way the viewer reads them, and the boxes
    /// stored for that version.
    pub(crate) async fn new(
        state: &AppState,
        workspace_id: &str,
        design_system_version_id: Option<&str>,
        tokens: Option<&Value>,
        components: &[Value],
    ) -> Self {
        let mut sizing = Self::without_stored(design_system_version_id, tokens, components).await;
        sizing.stored = Arc::new(read_stored(state, workspace_id, &sizing.key).await);
        sizing
    }

    /// The sizing for an outline, without the boxes stored for its version.
    pub(crate) async fn without_stored(
        design_system_version_id: Option<&str>,
        tokens: Option<&Value>,
        components: &[Value],
    ) -> Self {
        let key = design_system_version_id.unwrap_or_default().to_string();
        let stored = Arc::new(HashMap::new());
        if tokens.is_none() && components.is_empty() {
            return Self {
                system: None,
                key,
                stored,
            };
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
        Self {
            system,
            key,
            stored,
        }
    }

    /// The design system version the sizing is for.
    pub(crate) fn system(&self) -> Option<Arc<DesignSystem>> {
        self.system.clone()
    }

    /// The design system version's ID, or empty without one.
    pub(crate) fn key(&self) -> &str {
        &self.key
    }
}

/// What the renderer draws with on this thread: its bundled font, and no images.
pub(crate) fn assets() -> Assets {
    ASSETS.with(Assets::clone)
}

/// Where the boxes laid out against the design system version `key` are stored.
fn stored_key(workspace_id: &str, key: &str) -> String {
    let version = if key.is_empty() { "none" } else { key };
    format!("ws/{workspace_id}/boxes/{version}")
}

/// The boxes stored for the design system version `key`, by chunk; none when
/// there are none or they cannot be read, as they are only a shortcut.
async fn read_stored(
    state: &AppState,
    workspace_id: &str,
    key: &str,
) -> HashMap<String, (f32, f32)> {
    let Ok(object) = state.bucket.get_object(stored_key(workspace_id, key)).await else {
        return HashMap::new();
    };
    if object.status_code() != 200 {
        return HashMap::new();
    }
    serde_json::from_slice(object.as_slice()).unwrap_or_default()
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
    let known = known.or_else(|| sizing.stored.get(chunk).copied());
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

/// Serialises the writes of stored boxes, which add to one object.
static STORING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Lays out the boards that a write changed, in the background, and adds their
/// boxes to those stored for the design system version `design_system_version_id`.
/// `boards` are the chunk and text of each changed board. A board whose size is
/// fixed, or whose box is stored already, is left out; nothing is stored when
/// that leaves no board.
pub(crate) fn store_boxes(
    state: &AppState,
    workspace_id: &str,
    design_system_version_id: Option<String>,
    tokens: Option<Value>,
    components: Vec<Value>,
    boards: Vec<(String, String)>,
) {
    let (state, workspace_id) = (state.clone(), workspace_id.to_string());
    tokio::spawn(async move {
        let wanted = tokio::task::spawn_blocking(move || {
            boards
                .into_iter()
                .filter(|(_, text)| {
                    serde_json::from_str::<Value>(text).is_ok_and(|board| needs_box(&board))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        if wanted.is_empty() {
            return;
        }
        let _storing = STORING.lock().await;
        let sizing = Sizing::new(
            &state,
            &workspace_id,
            design_system_version_id.as_deref(),
            tokens.as_ref(),
            &components,
        )
        .await;
        let mut stored = (*sizing.stored).clone();
        let before = stored.len();
        let missing: Vec<(String, String)> = wanted
            .into_iter()
            .filter(|(chunk, _)| !stored.contains_key(chunk))
            .collect();
        let system = sizing.system.clone();
        let laid = each_concurrently(missing, move |(chunk, text)| {
            let system = system.clone();
            async move {
                let size = tokio::task::spawn_blocking(move || lay_out(&text, system.as_deref()))
                    .await
                    .ok()
                    .flatten();
                Ok((chunk, size))
            }
        })
        .await
        .unwrap_or_default();
        for (chunk, size) in laid {
            if let Some(size) = size {
                stored.insert(chunk, size);
            }
        }
        if stored.len() == before {
            return;
        }
        let Ok(body) = serde_json::to_vec(&stored) else {
            return;
        };
        if let Err(error) = state
            .bucket
            .put_object(stored_key(&workspace_id, &sizing.key), &body)
            .await
        {
            eprintln!("omni-api: cannot store the boxes of the boards: {error}");
        }
    });
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
}
