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

/// `GL_RGBA8`, the format of a WebGL default framebuffer with an alpha channel.
const GL_RGBA8: u32 = 0x8058;

/// The stencil depth the page requests when it creates the WebGL context.
const STENCIL_BITS: usize = 8;

thread_local! {
    /// What the page supplied for drawing: images and fonts, which stay until
    /// the page is closed.
    static ASSETS: RefCell<Assets> = RefCell::new(Assets::new());
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

/// A Skia GPU context on the WebGL context the page made current.
fn webgl_context() -> Option<DirectContext> {
    direct_contexts::make_gl(interfaces::make_web_gl()?, None)
}
