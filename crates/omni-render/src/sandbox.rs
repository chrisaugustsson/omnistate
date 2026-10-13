//! The scene drawn by the `render-sandbox/` test page: a solid background,
//! one filled rectangle and one line of text.

use skia_safe::{
    AlphaType, Canvas, Color, ColorType, Font, ImageInfo, Paint, font::Edging, surfaces,
};

use crate::text::inter;

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
        Self {
            width: 640,
            height: 360,
            background: [248, 250, 252],
            rect: Rect {
                x: 48.0,
                y: 48.0,
                width: 256.0,
                height: 160.0,
            },
            rect_color: [37, 99, 235],
            text: "Drawn by Skia from Rust, on WebGL".to_string(),
            text_x: 48.0,
            text_baseline: 272.0,
            font_size: 28.0,
            text_color: [15, 23, 42],
        }
    }
}

/// Draws `scene` on `canvas`, in the scene's canvas pixels.
pub fn draw(canvas: &Canvas, scene: &SandboxScene) {
    canvas.clear(color(scene.background));

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color(scene.rect_color));
    let Rect {
        x,
        y,
        width,
        height,
    } = scene.rect;
    canvas.draw_rect(skia_safe::Rect::from_xywh(x, y, width, height), &paint);

    let mut font = Font::from_typeface(inter(), scene.font_size);
    font.set_edging(Edging::AntiAlias);
    font.set_subpixel(true);
    paint.set_color(color(scene.text_color));
    canvas.draw_str(
        &scene.text,
        (scene.text_x, scene.text_baseline),
        &font,
        &paint,
    );
}

/// Draws `scene` with Skia on a CPU raster surface of `scene.width` x
/// `scene.height` pixels and returns them as unpremultiplied RGBA, row by row
/// from the top-left corner.
pub fn render_rgba(scene: &SandboxScene) -> Vec<u8> {
    let size = (pixels(scene.width), pixels(scene.height));
    let mut surface =
        surfaces::raster_n32_premul(size).expect("Skia creates a raster surface of the scene size");
    draw(surface.canvas(), scene);

    let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Unpremul, None);
    let mut rgba = vec![0; info.compute_min_byte_size()];
    let read = surface.read_pixels(&info, &mut rgba, info.min_row_bytes(), (0, 0));
    assert!(read, "Skia reads the rendered pixels back as RGBA");
    rgba
}

fn pixels(length: u32) -> i32 {
    i32::try_from(length).expect("canvas sizes fit in an i32")
}

fn color([red, green, blue]: Rgb) -> Color {
    Color::from_rgb(red, green, blue)
}
