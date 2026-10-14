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

use crate::layout;
use crate::paint::{self, Assets};
use crate::sandbox::{self, SandboxScene};
use crate::view::{Controls, Detail, FrameStats, Scene, Viewport};

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
