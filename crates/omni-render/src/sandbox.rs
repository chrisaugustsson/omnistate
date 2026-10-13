//! The scene drawn by the `render-sandbox/` test page: a solid background,
//! one filled rectangle and one line of text.

/// An opaque sRGB color: `[red, green, blue]`.
pub type Rgb = [u8; 3];

/// An axis-aligned rectangle in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Everything the sandbox page draws, in canvas pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxScene {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Fills the whole canvas.
    pub background: Rgb,
    /// The filled rectangle.
    pub rect: Rect,
    pub rect_color: Rgb,
    /// One line of text.
    pub text: String,
    /// Where the text starts on its baseline.
    pub text_x: f32,
    /// The y of the text baseline.
    pub text_baseline: f32,
    pub font_size: f32,
    pub text_color: Rgb,
}

impl SandboxScene {
    /// The scene the sandbox page shows.
    pub fn demo() -> Self {
        todo!("US-001: the render-sandbox scene")
    }
}

/// Draws `scene` with Skia on a CPU raster surface of `scene.width` x
/// `scene.height` pixels and returns them as unpremultiplied RGBA, row by row
/// from the top-left corner.
pub fn render_rgba(scene: &SandboxScene) -> Vec<u8> {
    todo!("US-001: rasterize {scene:?} with Skia")
}
