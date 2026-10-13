//! Drawing on the page's WebGL 2 canvas, in the WebAssembly build.

use skia_safe::{
    ColorType,
    gpu::{
        DirectContext, Protected, SurfaceOrigin, backend_render_targets, direct_contexts,
        gl::FramebufferInfo, interfaces, surfaces,
    },
};

use crate::sandbox::{self, SandboxScene};

/// `GL_RGBA8`, the format of a WebGL default framebuffer with an alpha channel.
const GL_RGBA8: u32 = 0x8058;

/// The stencil depth the page requests when it creates the WebGL context.
const STENCIL_BITS: usize = 8;

/// Draws the demo sandbox scene on the default framebuffer of the current
/// WebGL 2 context. The framebuffer is `width` x `height` device pixels, with
/// `pixel_ratio` device pixels per canvas pixel. Returns whether Skia drew it.
pub fn draw_sandbox(width: i32, height: i32, pixel_ratio: f32) -> bool {
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
    sandbox::draw(canvas, &SandboxScene::demo());
    context.flush_and_submit();
    true
}

/// A Skia GPU context on the WebGL context the page made current.
fn webgl_context() -> Option<DirectContext> {
    direct_contexts::make_gl(interfaces::make_web_gl()?, None)
}
