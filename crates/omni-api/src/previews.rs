// The previews of a design's boards (US-119).
//
// A design viewer reads a board once the board is in view, which takes a moment,
// and a view can show hundreds of boards at once. So that each of them shows its
// content from the first frame, the viewer gets a small picture of every board
// of a page before it shows the page, and draws a board from it until the board
// is read. The picture is the board at reduced detail, as omni-render draws it
// when zoomed out (`view::board_preview`), with the bundled font's boxes and the
// board's images, in the first mode of the design system.
//
// A preview depends on nothing but the board's chunk and the design system
// version, so it is stored in RustFS under both
// (`ws/{workspace_id}/previews/{version}/{chunk}`). Writing a design makes the
// previews of the boards it changed in the background. A preview that is not
// stored (the design system changed since, or the write has not finished) is
// made when it is read.

use crate::AppState;
use crate::boards::{self, CONCURRENCY, each_concurrently, each_concurrently_up_to};
use crate::chunks::read_chunk;
use crate::design;
use crate::placeholders::{self, Sizing};
use crate::session::ApiError;
use base64::Engine;
use omni_core::design::{AttachmentRef, Node};
use omni_render::paint::{Assets, DecodedImage, decode_image};
use omni_render::view::{
    BOARD_PNG_MAX_SIDE, board_box, board_images, board_png, board_png_side, board_preview,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// How many boards are drawn at once when a write makes its previews in the
/// background. Each board holds its surface and the glyphs it shaped on its
/// own thread, and the memory a draw takes stays with the process, so a
/// design of many boards (a Penpot import) is drawn a few boards at a time. A
/// read that finds previews missing draws them `CONCURRENCY` at a time, as it
/// answers a waiting client.
const DRAWN_AT_ONCE: usize = 4;

/// The boards that background writes draw at once, across every write: the
/// previews of a write that comes while those of another are being drawn wait
/// for them instead of drawing beside them, so an import of many batches
/// never draws more than `DRAWN_AT_ONCE` boards at a time.
static DRAWING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(DRAWN_AT_ONCE);

/// The chunk that the last write gave each board, by workspace and board ID,
/// for the boards whose previews wait to be drawn in the background. A preview
/// that a later write has replaced is not drawn: the board's newer chunk is,
/// and a read draws any preview it misses.
static LATEST: Mutex<Option<HashMap<(String, String), String>>> = Mutex::new(None);

/// The ID of a board, from its JSON text.
fn board_id(text: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Id {
        id: String,
    }
    serde_json::from_str::<Id>(text).ok().map(|board| board.id)
}

/// Draws the preview of a board that a write changed, unless a later write has
/// changed the board again, and forgets the board once its latest chunk is
/// drawn.
async fn make_latest(
    state: &AppState,
    workspace_id: &str,
    sizing: &Sizing,
    board: (Option<String>, String, String),
    read: &ImagesRead,
    decoded: &ImagesDecoded,
) {
    let (id, chunk, text) = board;
    let key = id.map(|id| (workspace_id.to_string(), id));
    let latest = |key: &(String, String)| {
        let latest = LATEST.lock().unwrap_or_else(|e| e.into_inner());
        latest.as_ref().and_then(|map| map.get(key)).cloned()
    };
    if let Some(key) = &key
        && latest(key).is_some_and(|newer| newer != chunk)
    {
        return;
    }
    let Ok(_drawing) = DRAWING.acquire().await else {
        return;
    };
    // A write may have come while this one waited.
    if let Some(key) = &key
        && latest(key).is_some_and(|newer| newer != chunk)
    {
        return;
    }
    make(
        state,
        workspace_id,
        sizing,
        (chunk.clone(), text),
        read,
        decoded,
    )
    .await;
    if let Some(key) = key {
        let mut latest = LATEST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = latest.as_mut()
            && map.get(&key) == Some(&chunk)
        {
            map.remove(&key);
        }
    }
}

/// Where the preview of the board stored in `chunk` is kept, for the design
/// system version `version`.
fn stored_key(workspace_id: &str, version: &str, chunk: &str) -> String {
    let version = if version.is_empty() { "none" } else { version };
    format!("ws/{workspace_id}/previews/{version}/{chunk}")
}

/// The files of the images read so far, so that the boards of one write that
/// share an image read it once.
type ImagesRead = Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>;

/// The images decoded so far, by reference, so that the boards of one write
/// decode each image once; `None` for bytes that are not an image.
type ImagesDecoded = Arc<Mutex<HashMap<String, Option<DecodedImage>>>>;

/// Gives `assets` the images among `read`, decoding each reference once
/// across `decoded`. Bytes the renderer cannot read leave their image missing.
fn supply_images(assets: &mut Assets, read: Vec<(String, Arc<Vec<u8>>)>, decoded: &ImagesDecoded) {
    for (reference, bytes) in read {
        let Ok(attachment) = reference.parse::<AttachmentRef>() else {
            continue;
        };
        let known = decoded
            .lock()
            .ok()
            .and_then(|decoded| decoded.get(&reference).cloned());
        let image = match known {
            Some(image) => image,
            None => {
                let image = decode_image(&bytes);
                if let Ok(mut decoded) = decoded.lock() {
                    decoded.insert(reference, image.clone());
                }
                image
            }
        };
        if let Some(image) = image {
            assets.add_decoded(&attachment, image);
        }
    }
}

/// The bytes of the images in `references` that can be read, by reference. An
/// image that cannot be read is left out, and the preview draws without it.
async fn read_images(
    state: &AppState,
    workspace_id: &str,
    references: Vec<String>,
    read: &ImagesRead,
) -> Vec<(String, Arc<Vec<u8>>)> {
    let mut images = Vec::with_capacity(references.len());
    for reference in references {
        let known = read
            .lock()
            .ok()
            .and_then(|read| read.get(&reference).cloned());
        if let Some(bytes) = known {
            images.push((reference, bytes));
            continue;
        }
        let Some(id) = reference.strip_prefix("attachment:") else {
            continue;
        };
        let key = format!("ws/{workspace_id}/attachments/{id}");
        let Ok(object) = state.bucket.get_object(key).await else {
            continue;
        };
        if object.status_code() != 200 {
            continue;
        }
        let bytes = Arc::new(object.to_vec());
        if let Ok(mut read) = read.lock() {
            read.insert(reference.clone(), bytes.clone());
        }
        images.push((reference, bytes));
    }
    images
}

/// Makes the preview of the board in `text` and stores it for `chunk`. `None`
/// for a board that cannot be drawn.
async fn make(
    state: &AppState,
    workspace_id: &str,
    sizing: &Sizing,
    (chunk, text): (String, String),
    read: &ImagesRead,
    decoded: &ImagesDecoded,
) -> Option<Vec<u8>> {
    let system = sizing.system();
    let listed = tokio::task::spawn_blocking(move || {
        let board = omni_render::nesting::read(&text).ok()?;
        let images = board_images(&board, system.as_deref());
        Some((board, images))
    })
    .await
    .ok()
    .flatten();
    let (board, references) = listed?;
    let images = read_images(state, workspace_id, references, read).await;
    let system = sizing.system();
    let decoded = decoded.clone();
    let png = tokio::task::spawn_blocking(move || {
        let mut assets = placeholders::assets();
        supply_images(&mut assets, images, &decoded);
        board_preview(board, &assets, system.as_deref())
    })
    .await
    .ok()
    .flatten()?;
    let key = stored_key(workspace_id, sizing.key(), &chunk);
    if let Err(error) = state.bucket.put_object(key, &png).await {
        eprintln!("omni-api: cannot store the preview of a board: {error}");
    }
    Some(png)
}

/// Makes and stores the previews of `boards`, the chunk and text of each, by
/// chunk.
async fn make_all(
    state: &AppState,
    workspace_id: &str,
    sizing: &Sizing,
    boards: Vec<(String, String)>,
    at_once: usize,
) -> HashMap<String, Vec<u8>> {
    let (state, workspace_id) = (state.clone(), workspace_id.to_string());
    let sizing = sizing.clone();
    let read = ImagesRead::default();
    let decoded = ImagesDecoded::default();
    let made = each_concurrently_up_to(at_once, boards, move |board| {
        let (state, workspace_id, sizing) = (state.clone(), workspace_id.clone(), sizing.clone());
        let (read, decoded) = (read.clone(), decoded.clone());
        async move {
            let chunk = board.0.clone();
            let png = make(&state, &workspace_id, &sizing, board, &read, &decoded).await;
            Ok(png.map(|png| (chunk, png)))
        }
    })
    .await
    .unwrap_or_default();
    made.into_iter().flatten().collect()
}

/// Makes the previews of the boards that a write changed, in the background.
/// `boards` are the chunk and text of each. The drawing waits for the
/// previews of earlier writes (`DRAWING`), and a board that a later write
/// changes again is drawn once, as the later write left it (`LATEST`).
pub(crate) fn store(
    state: &AppState,
    workspace_id: &str,
    design_system_version_id: Option<String>,
    tokens: Option<Value>,
    components: Vec<Value>,
    boards: Vec<(String, String)>,
) {
    let boards: Vec<(Option<String>, String, String)> = boards
        .into_iter()
        .map(|(chunk, text)| (board_id(&text), chunk, text))
        .collect();
    {
        let mut latest = LATEST.lock().unwrap_or_else(|e| e.into_inner());
        let map = latest.get_or_insert_with(HashMap::new);
        for (id, chunk, _) in &boards {
            if let Some(id) = id {
                map.insert((workspace_id.to_string(), id.clone()), chunk.clone());
            }
        }
    }
    let (state, workspace_id) = (state.clone(), workspace_id.to_string());
    tokio::spawn(async move {
        let sizing = Sizing::without_stored(
            design_system_version_id.as_deref(),
            tokens.as_ref(),
            &components,
        )
        .await;
        let read = ImagesRead::default();
        let decoded = ImagesDecoded::default();
        let _ = each_concurrently_up_to(DRAWN_AT_ONCE, boards, move |board| {
            let (state, workspace_id, sizing) =
                (state.clone(), workspace_id.clone(), sizing.clone());
            let (read, decoded) = (read.clone(), decoded.clone());
            async move {
                make_latest(&state, &workspace_id, &sizing, board, &read, &decoded).await;
                Ok(())
            }
        })
        .await;
    });
}

/// The previews of the boards `wanted` (board IDs) of the head `head` of a
/// design in the project `project_id`, as Base64 PNGs by board ID. A board the
/// head does not have, or that cannot be drawn, is left out.
pub(crate) async fn of_boards(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    head: &str,
    wanted: &HashSet<&str>,
) -> Result<BTreeMap<String, String>, ApiError> {
    let boards: Vec<(String, String)> = boards::head_boards(state, workspace_id, head)
        .await?
        .into_iter()
        .filter(|board| wanted.contains(board.id.as_str()))
        .map(|board| (board.id, board.chunk))
        .collect();
    let version = design::system_version_of_project(state, workspace_id, project_id).await?;
    let (store, workspace) = (state.clone(), workspace_id.to_string());
    let found = version.clone();
    let stored = each_concurrently(boards, move |(id, chunk)| {
        let (store, workspace, version) = (store.clone(), workspace.clone(), found.clone());
        async move {
            let key = stored_key(&workspace, version.as_deref().unwrap_or_default(), &chunk);
            let png = match store.bucket.get_object(key).await {
                Ok(object) if object.status_code() == 200 => Some(object.to_vec()),
                _ => None,
            };
            Ok((id, chunk, png))
        }
    })
    .await?;

    let missing: Vec<&str> = stored
        .iter()
        .filter(|(_, _, png)| png.is_none())
        .map(|(_, chunk, _)| chunk.as_str())
        .collect();
    let mut made = HashMap::new();
    if !missing.is_empty() {
        let (store, workspace) = (state.clone(), workspace_id.to_string());
        let names = missing.iter().map(|chunk| chunk.to_string()).collect();
        let texts = each_concurrently(names, move |chunk: String| {
            let (store, workspace) = (store.clone(), workspace.clone());
            async move {
                // A head in the older form has no chunk for each board.
                let text = read_chunk(&store, &workspace, &chunk).await;
                Ok(text.ok().map(|text| (chunk, text)))
            }
        })
        .await?;
        let system = design::system_head_of_project(state, workspace_id, project_id).await?;
        let (tokens, components) = (system.tokens_json(), system.components_json());
        let sizing = Sizing::without_stored(version.as_deref(), tokens.as_ref(), &components).await;
        let texts = texts.into_iter().flatten().collect();
        made = make_all(state, workspace_id, &sizing, texts, CONCURRENCY).await;
    }

    let encoder = base64::engine::general_purpose::STANDARD;
    Ok(stored
        .into_iter()
        .filter_map(|(id, chunk, png)| {
            let png = png.as_ref().or_else(|| made.get(&chunk))?;
            Some((id, encoder.encode(png)))
        })
        .collect())
}

/// The picture of one board that an export holds.
pub(crate) enum BoardPng {
    Made(Vec<u8>),
    /// A side of the picture would pass the longest one.
    TooLarge,
    /// The board has no box to draw.
    Undrawable,
}

/// The board `board` of a design in the project `project_id` at `scale`, in
/// full detail against the project's design system, with its images.
pub(crate) async fn full_size(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    board: Node,
    scale: f32,
) -> Result<BoardPng, ApiError> {
    let system = design::system_head_of_project(state, workspace_id, project_id).await?;
    let version = design::system_version_of_project(state, workspace_id, project_id).await?;
    let (tokens, components) = (system.tokens_json(), system.components_json());
    let sizing = Sizing::without_stored(version.as_deref(), tokens.as_ref(), &components).await;
    let read = ImagesRead::default();
    let mut fonts = Vec::new();
    for (family, file) in fonts_of(tokens.as_ref()) {
        for (_, bytes) in read_images(state, workspace_id, vec![file], &read).await {
            fonts.push((family.clone(), bytes));
        }
    }
    let with_fonts = Arc::new(move || {
        let mut assets = placeholders::assets();
        for (family, bytes) in &fonts {
            // A font file the renderer cannot read leaves the bundled one to stand in.
            let _ = assets.add_font(family, bytes);
        }
        assets
    });
    let (listed, box_system, box_assets) = (board.clone(), sizing.system(), with_fonts.clone());
    let (references, sides) = tokio::task::spawn_blocking(move || {
        let references = board_images(&listed, box_system.as_deref());
        let sides = board_box(listed, &box_assets(), box_system.as_deref()).map(|b| {
            (
                board_png_side(b.width, scale),
                board_png_side(b.height, scale),
            )
        });
        (references, sides)
    })
    .await
    .map_err(|_| ApiError::Unavailable)?;
    let Some((width, height)) = sides else {
        return Ok(BoardPng::Undrawable);
    };
    if width > BOARD_PNG_MAX_SIDE || height > BOARD_PNG_MAX_SIDE {
        return Ok(BoardPng::TooLarge);
    }
    let images = read_images(state, workspace_id, references, &read).await;
    let system = sizing.system();
    let png = tokio::task::spawn_blocking(move || {
        let mut assets = with_fonts();
        for (reference, bytes) in images {
            if let Ok(attachment) = reference.parse::<AttachmentRef>() {
                // An image the renderer cannot read draws as a missing one.
                let _ = assets.add_image(&attachment, &bytes);
            }
        }
        board_png(board, &assets, system.as_deref(), scale)
    })
    .await
    .map_err(|_| ApiError::Unavailable)?;
    Ok(png.map_or(BoardPng::Undrawable, BoardPng::Made))
}

/// The font files of the typography tokens of `tokens`, each with the family of
/// the token that lists it, as the viewer gives them to the renderer.
fn fonts_of(tokens: Option<&Value>) -> Vec<(String, String)> {
    fn visit(node: &Value, found: &mut Vec<(String, String)>) {
        let Value::Object(fields) = node else {
            return;
        };
        if let Some(value) = fields.get("$value") {
            let family = value.get("fontFamily").and_then(Value::as_str);
            let files = fields
                .get("$extensions")
                .and_then(|extensions| extensions.get("omnistate"))
                .and_then(|omnistate| omnistate.get("fontFiles"))
                .and_then(Value::as_array);
            if let (Some(family), Some(files)) = (family, files) {
                for file in files.iter().filter_map(Value::as_str) {
                    let pair = (family.to_owned(), file.to_owned());
                    if file.starts_with("attachment:") && !found.contains(&pair) {
                        found.push(pair);
                    }
                }
            }
            return;
        }
        for (name, child) in fields {
            if !name.starts_with('$') {
                visit(child, found);
            }
        }
    }
    let mut found = Vec::new();
    if let Some(tokens) = tokens {
        visit(tokens, &mut found);
    }
    found
}
