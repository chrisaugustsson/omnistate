//! Drawing on the page's WebGL 2 canvas, in the WebAssembly build.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use omni_core::design::{AttachmentRef, NodeId};
use skia_safe::{
    Canvas, Color, ColorType, Image, PixelGeometry, SurfaceProps, SurfacePropsFlags,
    gpu::{
        ContextOptions, DirectContext, Protected, SurfaceOrigin, backend_render_targets,
        direct_contexts, gl::FramebufferInfo, interfaces, surfaces,
    },
};

use crate::inspect::{Inspection, Property};
use crate::layout::{self, LayoutBox};
use crate::nesting::{self, MAX_NESTING};
use crate::paint::{self, Assets};
use crate::play::{Session, Tick};
use crate::sandbox::{self, SandboxScene};
use crate::select::{Event, Layer, Modifiers, NodeAddress, Outline, OutlineKind};
use crate::snapshot;
use crate::system::{DesignSystem, Missing};
use crate::view::{Controls, Detail, FrameStats, Scene, Viewport};
use serde_json::{Value, json};

/// `GL_RGBA8`, the format of a WebGL default framebuffer with an alpha channel.
const GL_RGBA8: u32 = 0x8058;

/// The stencil depth the page requests when it creates the WebGL context.
const STENCIL_BITS: usize = 8;

/// How long a frame of the design viewer spends making the pictures and
/// snapshots of the boards it shows (`Scene::set_frame_budget`).
const FRAME_BUDGET: Duration = Duration::from_millis(3);

/// What the stress page looks at: a scene, the controls that move its view,
/// and what the last frame did.
struct Stage {
    scene: Scene,
    /// The ID of the page the scene shows, so that showing it again (with more
    /// of its boards loaded) keeps the view.
    page: NodeId,
    controls: Controls,
    /// The view is fitted to the boards at the first frame, when the size of
    /// the canvas is known.
    fit_pending: bool,
    stats: Option<FrameStats>,
}

thread_local! {
    /// Whether a page opens on what `Scene::opening` says instead of on all its
    /// boards.
    static OPEN_READABLE: Cell<bool> = const { Cell::new(false) };
    /// The board a link named, which `Scene::opening` opens the page on.
    static OPEN_ON: Cell<Option<NodeId>> = const { Cell::new(None) };
    /// What the page supplied for drawing: images and fonts, which stay until
    /// the page is closed.
    static ASSETS: RefCell<Assets> = RefCell::new(Assets::new());
    /// The previews of boards that the page supplied, by board.
    static PREVIEWS: RefCell<HashMap<NodeId, Image>> = RefCell::new(HashMap::new());
    /// The design system version that scenes resolve against, as the page
    /// supplied it (`DesignSystem::from_json`).
    static SYSTEM: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// The prototype being played on the scene's page, once `play_start` has
    /// started one.
    static PLAY: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The scene of the stress page, once it has loaded one.
    static STAGE: RefCell<Option<Stage>> = const { RefCell::new(None) };
    /// The Skia context on the page's WebGL context. Frames reuse it, so that
    /// compiled shaders and the textures of snapshots stay on the GPU.
    static GPU: RefCell<Option<DirectContext>> = const { RefCell::new(None) };
    /// Where the time of the last `scene_load` or `scene_set_system` and the
    /// last `scene_draw` went, in milliseconds: reading, laying out, painting
    /// and submitting to the GPU.
    static TIMINGS: Cell<Timings> = const { Cell::new(Timings { parse: 0.0, layout: 0.0, paint: 0.0, submit: 0.0 }) };
    /// How long the last `on_canvas` took to submit its commands to the GPU.
    static SUBMIT_MS: Cell<f64> = const { Cell::new(0.0) };
}

/// The phases of the last scene load or change of design system (`parse`,
/// `layout`) and the last frame (`paint`, `submit`), in milliseconds
/// (`scene_timings`).
#[derive(Clone, Copy)]
struct Timings {
    parse: f64,
    layout: f64,
    paint: f64,
    submit: f64,
}

fn millis(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Draws the demo sandbox scene on the default framebuffer of the current
/// WebGL 2 context. The framebuffer is `width` x `height` device pixels, with
/// `pixel_ratio` device pixels per canvas pixel. Returns whether Skia drew it.
pub fn draw_sandbox(width: i32, height: i32, pixel_ratio: f32) -> bool {
    on_canvas(width, height, pixel_ratio, false, |canvas| {
        sandbox::draw(canvas, &SandboxScene::demo());
    })
}

/// Supplies the image that the attachment `attachment` (`attachment:<id>`)
/// names, from PNG or JPEG bytes.
pub fn add_image(attachment: &str, data: &[u8]) -> Result<(), String> {
    let attachment: AttachmentRef = attachment.parse().map_err(|error| format!("{error}"))?;
    ASSETS.with_borrow_mut(|assets| assets.add_image(&attachment, data))
}

/// Supplies the preview of the board `board` (`view::board_preview`), from PNG
/// or JPEG bytes, in place of any it had.
pub fn add_preview(board: &str, data: &[u8]) -> Result<(), String> {
    let board: NodeId = board.parse().map_err(|error| format!("{error}"))?;
    let image = paint::decode_image(data)
        .ok_or_else(|| format!("the bytes for the preview of {board} are not an image"))?;
    PREVIEWS.with_borrow_mut(|previews| previews.insert(board, image));
    Ok(())
}

/// Supplies a font file for text in `family`.
pub fn add_font(family: &str, data: &[u8]) -> Result<(), String> {
    ASSETS.with_borrow_mut(|assets| assets.add_font(family, data))
}

/// Draws the design in `design_json` on the default framebuffer of the current
/// WebGL 2 context, which is `width` x `height` device pixels with
/// `pixel_ratio` device pixels per CSS pixel, on a transparent background.
pub fn draw_design(
    design_json: &str,
    width: i32,
    height: i32,
    pixel_ratio: f32,
) -> Result<(), String> {
    let design = nesting::read(design_json)?;
    let drawn = ASSETS.with_borrow(|assets| {
        let boxes = layout::compute(&design, assets.fonts());
        on_canvas(width, height, pixel_ratio, false, |canvas| {
            canvas.clear(Color::TRANSPARENT);
            paint::draw(canvas, &design, &boxes, assets);
        })
    });
    if drawn {
        Ok(())
    } else {
        Err("Skia could not draw on the WebGL context".to_string())
    }
}

/// The boxes that the layout of the design in `design_json` gives its nodes,
/// as JSON (see [`sandbox::layout_json`]).
pub fn layout_design(design_json: &str) -> Result<String, String> {
    ASSETS.with_borrow(|assets| sandbox::layout_json(design_json, assets.fonts()))
}

/// Makes every page open on what can be read (`Scene::opening`) from now on,
/// as the design viewer wants, instead of on all its boards.
pub fn view_open_readable() {
    OPEN_READABLE.set(true);
}

/// Makes the pages opened from now on open on `board`, the ID of a board a link
/// named, as far as `Scene::opening` says so. Anything that is not a node ID
/// (the empty text, for no board) asks for no board.
pub fn view_open_on(board: &str) {
    OPEN_ON.set(board.parse().ok());
}

/// Loads the node in `tags` (one tag and its subtree, the form the API gives
/// the snapshot of a comment) as a scene of its own, as [`scene_load`] loads a
/// page; see [`snapshot::page_of_tags`].
pub fn scene_load_tags(tags: &str) -> Result<(), String> {
    let page = snapshot::page_of_tags(tags)?;
    // Every snapshot is on the same page, whose view is fitted afresh.
    STAGE.set(None);
    scene_load(&page)
}

/// Loads `design_json`, a page, into the scene that `scene_draw` draws, with
/// the images and fonts that the page has supplied so far, which stay for
/// every scene loaded after them. The view is fitted to its boards, or to what
/// `view_open_readable` says, at the first frame, unless the scene replaces one
/// of the same page (a page shown again with more of its boards loaded), which
/// keeps its view and its selection. Fails if the page nests more than
/// `MAX_NESTING` levels deep once its instances are expanded (see
/// [`nesting`]); the scene is loaded all the same, without the deeper nodes.
pub fn scene_load(design_json: &str) -> Result<(), String> {
    let started = Instant::now();
    let page = nesting::read(design_json)?;
    let page_id = *page.id();
    let assets = ASSETS.with_borrow(Assets::clone);
    let system = SYSTEM
        .with_borrow(|system| system.as_ref().map(DesignSystem::from_json))
        .transpose()?;
    let parse = millis(started);
    let mut scene = match system {
        Some(system) => Scene::with_system(page, assets, system),
        None => Scene::new(page, assets),
    };
    if OPEN_READABLE.get() {
        scene.set_frame_budget(FRAME_BUDGET);
    }
    let layout = millis(started) - parse;
    TIMINGS.set(Timings {
        parse,
        layout,
        ..TIMINGS.get()
    });
    let mut stage = Stage {
        scene,
        page: page_id,
        controls: Controls::new(Viewport::new(1.0, 1.0)),
        fit_pending: true,
        stats: None,
    };
    if let Some(shown) = STAGE.take()
        && shown.page == page_id
    {
        let selected = shown.scene.selected_address();
        stage.controls = shown.controls;
        stage.fit_pending = shown.fit_pending;
        stage.scene.select_address(selected.as_ref());
        // The page that asked for the scene knows this selection.
        stage.scene.take_events();
    }
    let nested = nested_too_deep(&stage.scene);
    STAGE.set(Some(stage));
    nested
}

/// Draws the boards in `boards_json`, a JSON array of board IDs, of the loaded
/// scene from their previews (`add_preview`, `Scene::use_preview`) until they
/// are put in place. A board without a preview stays as it is.
pub fn scene_use_previews(boards_json: &str) -> Result<(), String> {
    let boards: Vec<NodeId> =
        serde_json::from_str(boards_json).map_err(|error| error.to_string())?;
    STAGE.with_borrow_mut(|stage| {
        let Some(stage) = stage else {
            return;
        };
        PREVIEWS.with_borrow(|previews| {
            for (board, image) in boards
                .iter()
                .filter_map(|board| Some((board, previews.get(board)?)))
            {
                stage.scene.use_preview(board, image.clone());
            }
        });
    });
    Ok(())
}

/// Queues the boards in `boards_json`, a JSON array of boards, to take the
/// place of the boards of the loaded page that have their IDs, with the images
/// and fonts supplied so far (`Scene::queue_boards`): the way to show a board
/// that was read after the page was shown, without loading the page again.
/// `scene_update_step` puts them in place. `Ok(false)`, with nothing queued,
/// if no page is loaded or it has no such board.
pub fn scene_update_boards(boards_json: &str) -> Result<bool, String> {
    let boards = nesting::read_all(boards_json)?;
    let assets = ASSETS.with_borrow(Assets::clone);
    Ok(STAGE.with_borrow_mut(|stage| {
        stage
            .as_mut()
            .is_some_and(|stage| stage.scene.queue_boards(boards, assets))
    }))
}

/// Does the next stage of the boards queued by `scene_update_boards` (a stage
/// takes a few milliseconds even for a big board, so that a frame can do one
/// between draws) and returns whether there are more, and whether the stage put
/// a board in place, which the next frame has to draw. The view, the other
/// boards' pictures and the selection stay. Fails, as `scene_load` does, if the
/// page then nests too deeply; the boards are in place all the same.
pub fn scene_update_step() -> Result<(bool, bool), String> {
    let started = Instant::now();
    let stepped = STAGE.with_borrow_mut(|stage| {
        let Some(stage) = stage else {
            return Ok((false, false));
        };
        let placed = stage.scene.placed();
        let more = stage.scene.work();
        let placed = stage.scene.placed() != placed;
        nested_too_deep(&stage.scene).map(|()| (more, placed))
    });
    TIMINGS.set(Timings {
        layout: millis(started),
        ..TIMINGS.get()
    });
    stepped
}

/// Fails if `scene` holds nodes deeper than `MAX_NESTING` levels below its
/// page once its instances are expanded. Resolving left them out, so the scene
/// is safe to draw, but it is not the whole design.
fn nested_too_deep(scene: &Scene) -> Result<(), String> {
    match scene
        .errors()
        .iter()
        .find(|error| error.missing == Missing::Nesting)
    {
        Some(error) => Err(nesting::too_deep(Some(&error.node.to_string()))),
        None => Ok(()),
    }
}

/// Reads `system_json`, a design system version (`DesignSystem::from_json`: a
/// DTCG token tree with the components under `$components`), and makes the
/// loaded scene and every scene loaded after it resolve their instances and
/// token references against it, in its first mode. Fails, as `scene_load`
/// does, if the loaded scene then nests too deeply.
pub fn scene_set_system(system_json: &str) -> Result<(), String> {
    let started = Instant::now();
    let json: Value = serde_json::from_str(system_json).map_err(|error| error.to_string())?;
    let system = DesignSystem::from_json(&json)?;
    SYSTEM.set(Some(json));
    let parse = millis(started);
    let result = STAGE.with_borrow_mut(|stage| match stage {
        Some(stage) => {
            stage.scene.set_system(system);
            nested_too_deep(&stage.scene)
        }
        None => Ok(()),
    });
    TIMINGS.set(Timings {
        parse,
        layout: millis(started) - parse,
        ..TIMINGS.get()
    });
    result
}

/// The IDs of the boards of the loaded scene that its view shows, in page order,
/// as a JSON array. The view is the one of the last frame, so a page that has
/// not been drawn yet still has its first view. Empty if no scene is loaded.
pub fn visible_boards() -> String {
    let boards = with_stage(|scene, viewport| scene.visible_boards(viewport)).unwrap_or_default();
    Value::Array(
        boards
            .iter()
            .map(|id| Value::String(id.to_string()))
            .collect(),
    )
    .to_string()
}

/// Resolves the tokens of the loaded scene in `mode` from now on. Fails if no
/// scene is loaded, if the scene's design system has no such mode, or, as
/// `scene_load` does, if the scene then nests too deeply.
pub fn scene_set_mode(mode: &str) -> Result<(), String> {
    STAGE.with_borrow_mut(|stage| {
        let scene = &mut stage.as_mut().ok_or("no scene is loaded")?.scene;
        scene.set_mode(mode)?;
        nested_too_deep(scene)
    })
}

/// The nodes of the loaded scene that are drawn as error placeholders, as a
/// JSON array: `[{"node": "<address>", "component": "<id>"}, {"node":
/// "<address>", "token": "$color.surface"}]`. A token whose value does not
/// fit where it is used also names the node's property: `{"node":
/// "<address>", "token": "$color.brand", "property": "layout"}`, and a node
/// whose children lie deeper than `MAX_NESTING` levels names the limit:
/// `{"node": "<address>", "nesting": 128}`.
pub fn scene_errors() -> String {
    let errors = with_stage(|scene, _| {
        scene
            .errors()
            .iter()
            .map(|error| match &error.missing {
                Missing::Component(id) => {
                    json!({"node": error.node.to_string(), "component": id.to_string()})
                }
                Missing::Token(token) => {
                    json!({"node": error.node.to_string(), "token": token.as_str()})
                }
                Missing::Unfit { token, property } => json!({
                    "node": error.node.to_string(),
                    "token": token.as_str(),
                    "property": property,
                }),
                Missing::Nesting => {
                    json!({"node": error.node.to_string(), "nesting": MAX_NESTING})
                }
            })
            .collect::<Vec<Value>>()
    })
    .unwrap_or_default();
    Value::Array(errors).to_string()
}

/// Draws the scene in its current view on the default framebuffer of the
/// current WebGL 2 context, which is `width` x `height` device pixels with
/// `pixel_ratio` device pixels per CSS pixel, on a transparent background.
/// Returns whether a board was drawn from an older or smaller picture than
/// the view calls for, so that drawing again is worth it
/// (`FrameStats::deferred`).
pub fn scene_draw(width: i32, height: i32, pixel_ratio: f32) -> Result<bool, String> {
    STAGE.with_borrow_mut(|stage| {
        let stage = stage.as_mut().ok_or("no scene is loaded")?;
        let viewport = stage.controls.viewport_mut();
        viewport.resize(width as f32 / pixel_ratio, height as f32 / pixel_ratio);
        // A canvas that has no size yet (a hidden page) has nothing to fit.
        if stage.fit_pending && width >= 2 && height >= 2 {
            let opening = if OPEN_READABLE.get() {
                stage.scene.opening(viewport, pixel_ratio, OPEN_ON.get())
            } else {
                stage.scene.bounds()
            };
            viewport.fit(opening);
            stage.fit_pending = false;
        }
        let viewport = *stage.controls.viewport();
        let mut stats = None;
        let mut paint = 0.0;
        let drawn = on_canvas(width, height, pixel_ratio, true, |canvas| {
            let started = Instant::now();
            canvas.clear(Color::TRANSPARENT);
            stats = Some(stage.scene.render(canvas, &viewport));
            paint = millis(started);
        });
        TIMINGS.set(Timings {
            paint,
            submit: SUBMIT_MS.get(),
            ..TIMINGS.get()
        });
        let deferred = stats.as_ref().is_some_and(|stats| stats.deferred);
        stage.stats = stats;
        if drawn {
            Ok(deferred)
        } else {
            Err("Skia could not draw on the WebGL context".to_string())
        }
    })
}

/// Where the time of the last `scene_load` or `scene_set_system` and the last
/// `scene_draw` went, as JSON in milliseconds: `{"parseMs": 12.5, "layoutMs":
/// 80.1, "paintMs": 3.2, "submitMs": 1.4}`. Parsing the page and the design
/// system and laying the scene out (resolving its instances and tokens
/// included) belong to the load or the change of design system;
/// painting (Skia recording and rasterising the boards) and submitting the
/// commands to the GPU belong to the frame. The GPU finishing them is not
/// counted: the page waits for it itself (`gl.finish()`).
pub fn scene_timings() -> String {
    let t = TIMINGS.get();
    format!(
        "{{\"parseMs\": {}, \"layoutMs\": {}, \"paintMs\": {}, \"submitMs\": {}}}",
        t.parse, t.layout, t.paint, t.submit
    )
}

/// What the last `scene_draw` did, as JSON: `{"boardsDrawn": 3,
/// "nodesPainted": 640, "detail": "full", "zoom": 1}`, with `detail` either
/// `"full"` or `"reduced"`. `"firstBoard"` is how the page's first board stands
/// in the view, or `null` for a page without boards: `{"id": "<id>", "x": 0,
/// "y": 0, "width": 1200, "height": 536, "whole": true, "nodes": 57,
/// "missingImages": 0}`, with the box in canvas pixels, and `nodes` and
/// `missingImages` as far as its full detail was painted (`FirstBoard`).
/// `None` before the first frame.
pub fn scene_stats() -> Option<String> {
    STAGE.with_borrow(|stage| {
        let stage = stage.as_ref()?;
        let stats = stage.stats.as_ref()?;
        let detail = match stats.detail {
            Detail::Full => "full",
            Detail::Reduced => "reduced",
        };
        let first_board = stage
            .scene
            .first_board(stage.controls.viewport())
            .map_or(Value::Null, |first| {
                json!({
                    "id": first.id.to_string(),
                    "x": first.screen.x,
                    "y": first.screen.y,
                    "width": first.screen.width,
                    "height": first.screen.height,
                    "whole": first.whole,
                    "nodes": first.painted.nodes,
                    "missingImages": first.painted.missing_images,
                })
            });
        Some(format!(
            "{{\"boardsDrawn\": {}, \"nodesPainted\": {}, \"detail\": \"{detail}\", \"zoom\": {}, \"firstBoard\": {first_board}}}",
            stats.boards_drawn.len(),
            stats.nodes_painted,
            stage.controls.viewport().zoom()
        ))
    })
}

/// Runs `handle` on the controls of the scene. `false` if no scene is loaded.
pub fn with_controls(handle: impl FnOnce(&mut Controls) -> bool) -> bool {
    STAGE.with_borrow_mut(|stage| stage.as_mut().is_some_and(|s| handle(&mut s.controls)))
}

/// Runs `handle` on the scene and its current view. `None` if no scene is
/// loaded.
fn with_stage<T>(handle: impl FnOnce(&mut Scene, &Viewport) -> T) -> Option<T> {
    STAGE.with_borrow_mut(|stage| {
        let stage = stage.as_mut()?;
        let viewport = *stage.controls.viewport();
        Some(handle(&mut stage.scene, &viewport))
    })
}

/// Brings the selected node into view (`Scene::reveal_selected`).
pub fn view_reveal_selected() {
    STAGE.with_borrow_mut(|stage| {
        if let Some(stage) = stage {
            let viewport = stage.controls.viewport_mut();
            stage.scene.reveal_selected(viewport);
        }
    });
}

/// The pointer moved to the canvas point `x`, `y` (CSS pixels): hover.
pub fn select_pointer_move(x: f32, y: f32, ctrl: bool, meta: bool) {
    with_stage(|scene, viewport| scene.pointer_move(viewport, (x, y), Modifiers { ctrl, meta }));
}

/// A click at the canvas point `x`, `y`.
pub fn select_click(x: f32, y: f32, ctrl: bool, meta: bool) {
    with_stage(|scene, viewport| scene.click(viewport, (x, y), Modifiers { ctrl, meta }));
}

/// A double-click at the canvas point `x`, `y`.
pub fn select_double_click(x: f32, y: f32) {
    with_stage(|scene, viewport| scene.double_click(viewport, (x, y)));
}

/// The Escape key.
pub fn select_escape() {
    with_stage(|scene, _| scene.escape());
}

/// Selects the node at `address` (`<id>` or `<instanceId>/<componentNodeId>`),
/// as the design viewer's layers panel does; empty text clears the selection.
/// `Ok(false)` if no scene is loaded or the address names no node.
pub fn select_node(address: &str) -> Result<bool, String> {
    let address: Option<NodeAddress> = if address.is_empty() {
        None
    } else {
        Some(address.parse().map_err(|error| format!("{error}"))?)
    };
    Ok(with_stage(|scene, _| scene.select_address(address.as_ref())).unwrap_or(false))
}

fn layer_json(layer: &Layer) -> Value {
    json!({
        "address": layer.address.to_string(),
        "name": layer.name,
        "type": layer.kind,
        "children": layer.children.iter().map(layer_json).collect::<Vec<Value>>(),
    })
}

/// The boards of the loaded scene without their children, as a JSON array:
/// `[{"address": "<address>", "name": "Cart", "type": "board", "children":
/// []}]`. Empty if no scene is loaded.
pub fn scene_layers() -> String {
    let layers = with_stage(|scene, _| scene.layers()).unwrap_or_default();
    Value::Array(layers.iter().map(layer_json).collect()).to_string()
}

/// The texts of the loaded scene that are drawn in another font than their
/// own, as a JSON array: `[{"family": "Roboto", "weight": 700, "style":
/// "normal", "drawnAs": {"family": "Inter", "weight": 400, "style": "normal"},
/// "texts": 12}]`. Empty if no scene is loaded.
pub fn scene_fonts() -> String {
    let stood_in = with_stage(|scene, _| scene.fonts_stood_in()).unwrap_or_default();
    Value::Array(
        stood_in
            .iter()
            .map(|entry| {
                json!({
                    "family": entry.family,
                    "weight": entry.weight,
                    "style": entry.style.as_str(),
                    "drawnAs": {
                        "family": entry.drawn_family,
                        "weight": entry.drawn_weight,
                        "style": entry.drawn_style.as_str(),
                    },
                    "texts": entry.texts,
                })
            })
            .collect(),
    )
    .to_string()
}

/// The node at `address` (`<id>` or `<instanceId>/<componentNodeId>`) with
/// every node below it, as the JSON object `scene_layers` lists boards with.
/// `Ok(None)` if the address names no node.
pub fn scene_layers_of(address: &str) -> Result<Option<String>, String> {
    let address: NodeAddress = address.parse().map_err(|error| format!("{error}"))?;
    Ok(with_stage(|scene, _| scene.layers_of(&address))
        .flatten()
        .map(|layer| layer_json(&layer).to_string()))
}

/// The ID of the board that holds the node at `address` (`<id>` or
/// `<instanceId>/<componentNodeId>`). `Ok(None)` if the address names no node.
pub fn scene_board_of(address: &str) -> Result<Option<String>, String> {
    let address: NodeAddress = address.parse().map_err(|error| format!("{error}"))?;
    Ok(with_stage(|scene, _| scene.board_of(&address))
        .flatten()
        .map(|board| board.to_string()))
}

fn box_json(rect: LayoutBox) -> Value {
    json!({"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height})
}

/// The bounds of the node at `address` (`<id>` or `<instanceId>/<componentNodeId>`)
/// as JSON, in page coordinates and in canvas pixels at the current view:
/// `{"page": {"x": 20, "y": 20, "width": 200, "height": 200}, "screen": {...}}`.
/// `Ok(None)` if the address names no node.
pub fn bounds_of(address: &str) -> Result<Option<String>, String> {
    let address: NodeAddress = address.parse().map_err(|error| format!("{error}"))?;
    Ok(with_stage(|scene, viewport| {
        let page = scene.bounds_of(&address)?;
        let screen = scene.screen_box_of(&address, viewport)?;
        Some(json!({"page": box_json(page), "screen": box_json(screen)}).to_string())
    })
    .flatten())
}

/// The box of the node at `address` on the canvas at the current view, as
/// JSON `{"x", "y", "width", "height", "name"}` in CSS pixels from the canvas's
/// top-left corner, with the node's name: where the design viewer pins the
/// node's comments.
/// `Ok(None)` if no scene is loaded or the address names no node.
pub fn node_box(address: &str) -> Result<Option<String>, String> {
    let address: NodeAddress = address.parse().map_err(|error| format!("{error}"))?;
    Ok(with_stage(|scene, viewport| {
        let rect = scene.screen_box_of(&address, viewport)?;
        let mut json = box_json(rect);
        json["name"] = scene.name_of(&address)?.into();
        Some(json.to_string())
    })
    .flatten())
}

fn property_json(property: &Property) -> Value {
    json!({
        "name": property.name,
        "value": property.value,
        "tokens": property
            .tokens
            .iter()
            .map(|used| json!({"token": used.token, "value": used.value}))
            .collect::<Vec<Value>>(),
    })
}

fn inspection_json(inspection: &Inspection) -> Value {
    let group =
        |properties: &[Property]| Value::Array(properties.iter().map(property_json).collect());
    let instance = inspection
        .instance
        .as_ref()
        .map_or(Value::Null, |instance| {
            json!({
                "component": instance.component.to_string(),
                "componentName": instance.component_name,
                "props": instance
                    .props
                    .iter()
                    .map(|prop| json!({
                        "name": prop.name,
                        "kind": prop.kind,
                        "value": prop.value,
                        "set": prop.set,
                    }))
                    .collect::<Vec<Value>>(),
                "overrides": instance
                    .overrides
                    .iter()
                    .map(|over| json!({
                        "node": over.node.to_string(),
                        "name": over.name,
                        "fields": over.fields,
                    }))
                    .collect::<Vec<Value>>(),
            })
        });
    json!({
        "address": inspection.address.to_string(),
        "name": inspection.name,
        "type": inspection.kind,
        "mode": inspection.mode,
        "box": box_json(inspection.bounds),
        "layout": group(&inspection.layout),
        "size": group(&inspection.size),
        "style": group(&inspection.style),
        "text": group(&inspection.text),
        "instance": instance,
        "interactions": inspection.interactions,
        "overflow": inspection.overflow,
        "scrollPosition": inspection.scroll_position,
    })
}

/// The node at `address` (`<id>` or `<instanceId>/<componentNodeId>`) as the
/// inspect panel shows it, as JSON (see `omni_render_inspect`). `Ok(None)` if
/// no scene is loaded or the address names no node.
pub fn inspect(address: &str) -> Result<Option<String>, String> {
    let address: NodeAddress = address.parse().map_err(|error| format!("{error}"))?;
    Ok(with_stage(|scene, _| {
        scene
            .inspect(&address)
            .map(|inspection| inspection_json(&inspection).to_string())
    })
    .flatten())
}

fn address_json(address: Option<NodeAddress>) -> Value {
    address.map_or(Value::Null, |address| Value::String(address.to_string()))
}

/// The hover and selection changes since the last call as a JSON array,
/// oldest first: `[{"type": "hover", "node": "<address>" | null}, {"type":
/// "select", "node": "<address>" | null}]`.
pub fn take_events() -> String {
    let events = with_stage(|scene, _| scene.take_events()).unwrap_or_default();
    let events: Vec<Value> = events
        .into_iter()
        .map(|event| match event {
            Event::Hover(address) => json!({"type": "hover", "node": address_json(address)}),
            Event::Select(address) => json!({"type": "select", "node": address_json(address)}),
        })
        .collect();
    Value::Array(events).to_string()
}

/// The outlines of the hovered and the selected node in the current view as a
/// JSON array, in canvas pixels: `[{"kind": "hover" | "selection", "node":
/// "<address>", "label": "Card", "x": 240, "y": 140, "width": 400, "height":
/// 400}]`.
pub fn overlay() -> String {
    let outlines = with_stage(|scene, viewport| scene.overlay(viewport)).unwrap_or_default();
    let outlines: Vec<Value> = outlines.into_iter().map(outline_json).collect();
    Value::Array(outlines).to_string()
}

fn outline_json(outline: Outline) -> Value {
    let kind = match outline.kind {
        OutlineKind::Hover => "hover",
        OutlineKind::Selection => "selection",
    };
    json!({
        "kind": kind,
        "node": outline.node.to_string(),
        "label": outline.label,
        "x": outline.rect.x,
        "y": outline.rect.y,
        "width": outline.rect.width,
        "height": outline.rect.height,
    })
}

/// Runs `draw` on a canvas over the default framebuffer of the current WebGL 2
/// context, scaled by `pixel_ratio`, and flushes it. Returns whether Skia
/// could draw.
///
/// The design pages draw text from distance fields at every size (see
/// [`webgl_context`]), which needs the surface to say that its fonts are
/// device independent. The demo scene keeps Skia's usual hinted glyphs.
fn on_canvas(
    width: i32,
    height: i32,
    pixel_ratio: f32,
    device_independent_fonts: bool,
    draw: impl FnOnce(&Canvas),
) -> bool {
    let Some(mut context) = webgl_context() else {
        return false;
    };
    let framebuffer = FramebufferInfo {
        fboid: 0,
        format: GL_RGBA8,
        protected: Protected::No,
    };
    let target = backend_render_targets::make_gl((width, height), None, STENCIL_BITS, framebuffer);
    let flags = if device_independent_fonts {
        SurfacePropsFlags::USE_DEVICE_INDEPENDENT_FONTS
    } else {
        SurfacePropsFlags::empty()
    };
    let props = SurfaceProps::new(flags, PixelGeometry::Unknown);
    let Some(mut surface) = surfaces::wrap_backend_render_target(
        &mut context,
        &target,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        Some(&props),
    ) else {
        return false;
    };
    let canvas = surface.canvas();
    canvas.scale((pixel_ratio, pixel_ratio));
    draw(canvas);
    let started = Instant::now();
    context.flush_and_submit();
    SUBMIT_MS.set(millis(started));
    true
}

/// The Skia GPU context on the WebGL context the page made current, made on
/// first use.
///
/// Text of a zooming view is drawn from signed distance fields, which Skia
/// makes once per glyph and then scales on the GPU, from 1 px up. Glyph
/// bitmaps are made for one scale, so a view that zooms makes them again at
/// every step, which is most of the cost of a frame (US-008). The glyph atlas
/// may grow to 64 MB, as a board of text fills a good deal of it.
fn webgl_context() -> Option<DirectContext> {
    GPU.with_borrow_mut(|gpu| {
        if gpu.is_none() {
            let mut options = ContextOptions::new();
            options.min_distance_field_font_size = 1.0;
            options.glyph_cache_texture_maximum_bytes = 64 << 20;
            *gpu = direct_contexts::make_gl(interfaces::make_web_gl()?, &options);
        }
        let mut context = gpu.clone()?;
        // The canvas was resized or the page drew in between, so what Skia
        // remembers of the GL state is stale.
        context.reset(None);
        Some(context)
    })
}

/// Starts playing the loaded page on `board`, the ID of one of its boards, in
/// place of any session (`Session::start`). Fails if no scene is loaded or the
/// page has no such board.
pub fn play_start(board: &str) -> Result<(), String> {
    let session = STAGE.with_borrow(|stage| {
        let stage = stage.as_ref().ok_or("no scene is loaded")?;
        Session::start(&stage.scene, board)
    })?;
    PLAY.set(Some(session));
    Ok(())
}

/// Ends the session, if there is one.
pub fn play_stop() {
    PLAY.set(None);
}

/// Runs `handle` on the session. `Err` if none has started.
fn with_play<T>(handle: impl FnOnce(&mut Session) -> T) -> Result<T, String> {
    PLAY.with_borrow_mut(|play| {
        play.as_mut()
            .map(handle)
            .ok_or_else(|| "no prototype is playing".to_string())
    })
}

/// A pointer event on the stage (`Session::pointer`). Returns whether what the
/// player shows changed.
pub fn play_pointer(kind: u32, x: f32, y: f32) -> Result<bool, String> {
    with_play(|play| play.pointer(kind, x, y))
}

/// A key event as JSON (`Session::key`). Returns whether a key trigger fired.
pub fn play_key(json: &str) -> Result<bool, String> {
    with_play(|play| play.key(json))?
}

/// A controller button of the standard layout going down or up
/// (`Session::gamepad`). Returns whether a key trigger fired.
pub fn play_gamepad(button: u32, down: bool) -> Result<bool, String> {
    with_play(|play| play.gamepad(button, down))
}

/// Goes back (`Session::back`). Returns whether there was a board to go back to.
pub fn play_back() -> Result<bool, String> {
    with_play(Session::back)
}

/// Goes to `board`, leaving the board shown in the history (`Session::go`).
pub fn play_go(board: &str) -> Result<bool, String> {
    with_play(|play| play.go(board))?
}

/// Starts over on `board`, with no history (`Session::restart`).
pub fn play_restart(board: &str) -> Result<(), String> {
    with_play(|play| play.restart(board))?
}

/// Where the player is, as JSON (`Session::state`).
pub fn play_state() -> Result<String, String> {
    with_play(|play| play.state().to_string())
}

/// Moves the session on to `now` (milliseconds) and draws it, at the size of
/// the last `play_draw`, if it changed or is animating. Returns 0 when
/// nothing changed, 1 when a frame was drawn and 2 when one was drawn and
/// another is needed, as a transition is playing.
pub fn play_tick(now: f64) -> Result<u8, String> {
    let (tick, size) = with_play(|play| (play.tick(now), play.size()))?;
    match (tick, size) {
        (Tick::Idle, _) | (_, None) => Ok(0),
        (tick, Some((width, height, pixel_ratio))) => {
            play_draw(width, height, pixel_ratio)?;
            Ok(if tick == Tick::Animating { 2 } else { 1 })
        }
    }
}

/// Draws the session on the default framebuffer of the current WebGL 2
/// context, which is `width` x `height` device pixels with `pixel_ratio` device
/// pixels per CSS pixel (`Session::draw`).
pub fn play_draw(width: i32, height: i32, pixel_ratio: f32) -> Result<(), String> {
    let css = (width as f32 / pixel_ratio, height as f32 / pixel_ratio);
    let drawn = STAGE.with_borrow_mut(|stage| {
        PLAY.with_borrow_mut(|play| {
            let stage = stage.as_mut().ok_or("no scene is loaded")?;
            let play = play.as_mut().ok_or("no prototype is playing")?;
            Ok::<_, String>(on_canvas(width, height, pixel_ratio, true, |canvas| {
                play.draw(&mut stage.scene, canvas, css, pixel_ratio);
            }))
        })
    })?;
    if drawn {
        Ok(())
    } else {
        Err("Skia could not draw on the WebGL context".to_string())
    }
}
