//! Drawing on the page's WebGL 2 canvas, in the WebAssembly build.

use std::cell::RefCell;

use omni_core::design::AttachmentRef;
use skia_safe::{
    Canvas, Color, ColorType, PixelGeometry, SurfaceProps, SurfacePropsFlags,
    gpu::{
        ContextOptions, DirectContext, Protected, SurfaceOrigin, backend_render_targets,
        direct_contexts, gl::FramebufferInfo, interfaces, surfaces,
    },
};

use crate::inspect::{Inspection, Property};
use crate::layout::{self, LayoutBox};
use crate::nesting::{self, MAX_NESTING};
use crate::paint::{self, Assets};
use crate::sandbox::{self, SandboxScene};
use crate::select::{Event, Layer, Modifiers, NodeAddress, Outline, OutlineKind};
use crate::system::{DesignSystem, Missing};
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
    /// The design system version that scenes resolve against, as the page
    /// supplied it (`DesignSystem::from_json`).
    static SYSTEM: RefCell<Option<Value>> = const { RefCell::new(None) };
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

/// Loads `design_json`, a page, into the scene that `scene_draw` draws, with
/// the images and fonts that the page has supplied so far. The view is fitted
/// to its boards at the first frame. Fails if the page nests more than
/// `MAX_NESTING` levels deep once its instances are expanded (see
/// [`nesting`]); the scene is loaded all the same, without the deeper nodes.
pub fn scene_load(design_json: &str) -> Result<(), String> {
    let page = nesting::read(design_json)?;
    let assets = ASSETS.take();
    let system = SYSTEM
        .with_borrow(|system| system.as_ref().map(DesignSystem::from_json))
        .transpose()?;
    let scene = match system {
        Some(system) => Scene::with_system(page, assets, system),
        None => Scene::new(page, assets),
    };
    let stage = Stage {
        scene,
        controls: Controls::new(Viewport::new(1.0, 1.0)),
        fit_pending: true,
        stats: None,
    };
    let nested = nested_too_deep(&stage.scene);
    STAGE.set(Some(stage));
    nested
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
    let json: Value = serde_json::from_str(system_json).map_err(|error| error.to_string())?;
    let system = DesignSystem::from_json(&json)?;
    SYSTEM.set(Some(json));
    STAGE.with_borrow_mut(|stage| match stage {
        Some(stage) => {
            stage.scene.set_system(system);
            nested_too_deep(&stage.scene)
        }
        None => Ok(()),
    })
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
        let drawn = on_canvas(width, height, pixel_ratio, true, |canvas| {
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

/// The boards of the loaded scene with every node below them, as a JSON array:
/// `[{"address": "<address>", "name": "Cart", "type": "board", "children":
/// [...]}]`. Empty if no scene is loaded.
pub fn scene_layers() -> String {
    let layers = with_stage(|scene, _| scene.layers()).unwrap_or_default();
    Value::Array(layers.iter().map(layer_json).collect()).to_string()
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
/// JSON `{"x", "y", "width", "height"}` in CSS pixels from the canvas's
/// top-left corner: where the design viewer pins the node's comments.
/// `Ok(None)` if no scene is loaded or the address names no node.
pub fn node_box(address: &str) -> Result<Option<String>, String> {
    let address: NodeAddress = address.parse().map_err(|error| format!("{error}"))?;
    Ok(with_stage(|scene, viewport| {
        scene
            .screen_box_of(&address, viewport)
            .map(|rect| box_json(rect).to_string())
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
    context.flush_and_submit();
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
