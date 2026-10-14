//! Drawing on the page's WebGL 2 canvas, in the WebAssembly build.

use std::cell::RefCell;

use omni_core::design::{AttachmentRef, Node};
use skia_safe::{
    Canvas, Color, ColorType,
    gpu::{
        DirectContext, Protected, SurfaceOrigin, backend_render_targets, direct_contexts,
        gl::FramebufferInfo, interfaces, surfaces,
    },
};

use crate::layout::{self, LayoutBox};
use crate::paint::{self, Assets};
use crate::sandbox::{self, SandboxScene};
use crate::select::{Event, Modifiers, NodeAddress, Outline, OutlineKind};
use crate::view::{Controls, Detail, FrameStats, Scene, Viewport};
use serde_json::{Value, json};

/// `GL_RGBA8`, the format of a WebGL default framebuffer with an alpha channel.
const GL_RGBA8: u32 = 0x8058;

/// The stencil depth the page requests when it creates the WebGL context.
const STENCIL_BITS: usize = 8;

/// What the stress page looks at: a scene, the controls that move its view,
/// and what the last frame did.
struct Stage {
    scene: Scene,
    controls: Controls,
    /// The view is fitted to the boards at the first frame, when the size of
    /// the canvas is known.
    fit_pending: bool,
    stats: Option<FrameStats>,
}

thread_local! {
    /// What the page supplied for drawing: images and fonts, which stay until
    /// the page is closed.
    static ASSETS: RefCell<Assets> = RefCell::new(Assets::new());
    /// The scene of the stress page, once it has loaded one.
    static STAGE: RefCell<Option<Stage>> = const { RefCell::new(None) };
    /// The Skia context on the page's WebGL context. Frames reuse it, so that
    /// compiled shaders and the textures of snapshots stay on the GPU.
    static GPU: RefCell<Option<DirectContext>> = const { RefCell::new(None) };
}

/// Draws the demo sandbox scene on the default framebuffer of the current
/// WebGL 2 context. The framebuffer is `width` x `height` device pixels, with
/// `pixel_ratio` device pixels per canvas pixel. Returns whether Skia drew it.
pub fn draw_sandbox(width: i32, height: i32, pixel_ratio: f32) -> bool {
    on_canvas(width, height, pixel_ratio, |canvas| {
        sandbox::draw(canvas, &SandboxScene::demo());
    })
}

/// Supplies the image that the attachment `attachment` (`attachment:<id>`)
/// names, from PNG or JPEG bytes.
pub fn add_image(attachment: &str, data: &[u8]) -> Result<(), String> {
    let attachment: AttachmentRef = attachment.parse().map_err(|error| format!("{error}"))?;
    ASSETS.with_borrow_mut(|assets| assets.add_image(&attachment, data))
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
    let design: Node = serde_json::from_str(design_json).map_err(|error| error.to_string())?;
    let drawn = ASSETS.with_borrow(|assets| {
        let boxes = layout::compute(&design, assets.fonts());
        on_canvas(width, height, pixel_ratio, |canvas| {
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

/// Loads `design_json`, a page, into the scene that `scene_draw` draws, with
/// the images and fonts that the page has supplied so far. The view is fitted
/// to its boards at the first frame.
pub fn scene_load(design_json: &str) -> Result<(), String> {
    let page: Node = serde_json::from_str(design_json).map_err(|error| error.to_string())?;
    let assets = ASSETS.take();
    let stage = Stage {
        scene: Scene::new(page, assets),
        controls: Controls::new(Viewport::new(1.0, 1.0)),
        fit_pending: true,
        stats: None,
    };
    STAGE.set(Some(stage));
    Ok(())
}

/// Draws the scene in its current view on the default framebuffer of the
/// current WebGL 2 context, which is `width` x `height` device pixels with
/// `pixel_ratio` device pixels per CSS pixel, on a transparent background.
pub fn scene_draw(width: i32, height: i32, pixel_ratio: f32) -> Result<(), String> {
    STAGE.with_borrow_mut(|stage| {
        let stage = stage.as_mut().ok_or("no scene is loaded")?;
        let viewport = stage.controls.viewport_mut();
        viewport.resize(width as f32 / pixel_ratio, height as f32 / pixel_ratio);
        // A canvas that has no size yet (a hidden page) has nothing to fit.
        if stage.fit_pending && width >= 2 && height >= 2 {
            viewport.fit(stage.scene.bounds());
            stage.fit_pending = false;
        }
        let viewport = *stage.controls.viewport();
        let mut stats = None;
        let drawn = on_canvas(width, height, pixel_ratio, |canvas| {
            canvas.clear(Color::TRANSPARENT);
            stats = Some(stage.scene.render(canvas, &viewport));
        });
        stage.stats = stats;
        if drawn {
            Ok(())
        } else {
            Err("Skia could not draw on the WebGL context".to_string())
        }
    })
}

/// What the last `scene_draw` did, as JSON: `{"boardsDrawn": 3,
/// "nodesPainted": 640, "detail": "full", "zoom": 1}`, with `detail` either
/// `"full"` or `"reduced"`. `None` before the first frame.
pub fn scene_stats() -> Option<String> {
    STAGE.with_borrow(|stage| {
        let stage = stage.as_ref()?;
        let stats = stage.stats.as_ref()?;
        let detail = match stats.detail {
            Detail::Full => "full",
            Detail::Reduced => "reduced",
        };
        Some(format!(
            "{{\"boardsDrawn\": {}, \"nodesPainted\": {}, \"detail\": \"{detail}\", \"zoom\": {}}}",
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
        let (x, y) = viewport.to_screen((page.x, page.y));
        let zoom = viewport.zoom();
        let screen = LayoutBox {
            x,
            y,
            width: page.width * zoom,
            height: page.height * zoom,
        };
        Some(json!({"page": box_json(page), "screen": box_json(screen)}).to_string())
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
fn on_canvas(width: i32, height: i32, pixel_ratio: f32, draw: impl FnOnce(&Canvas)) -> bool {
    let Some(mut context) = webgl_context() else {
        return false;
    };
    let framebuffer = FramebufferInfo {
        fboid: 0,
        format: GL_RGBA8,
        protected: Protected::No,
    };
    let target = backend_render_targets::make_gl((width, height), None, STENCIL_BITS, framebuffer);
    let Some(mut surface) = surfaces::wrap_backend_render_target(
        &mut context,
        &target,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    ) else {
        return false;
    };
    let canvas = surface.canvas();
    canvas.scale((pixel_ratio, pixel_ratio));
    draw(canvas);
    context.flush_and_submit();
    true
}

/// The Skia GPU context on the WebGL context the page made current, made on
/// first use.
fn webgl_context() -> Option<DirectContext> {
    GPU.with_borrow_mut(|gpu| {
        if gpu.is_none() {
            *gpu = direct_contexts::make_gl(interfaces::make_web_gl()?, None);
        }
        let mut context = gpu.clone()?;
        // The canvas was resized or the page drew in between, so what Skia
        // remembers of the GL state is stale.
        context.reset(None);
        Some(context)
    })
}
