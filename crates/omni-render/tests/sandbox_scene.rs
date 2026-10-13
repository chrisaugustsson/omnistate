//! US-001: the render-sandbox scene (one rectangle and one line of text),
//! rasterized natively with Skia so its pixels can be checked.

use omni_render::sandbox::{Rect, Rgb, SandboxScene, render_rgba};

/// Allowed per-channel difference for solid fills.
const FILL_TOLERANCE: u8 = 2;

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

fn render(scene: &SandboxScene) -> Pixels {
    let rgba = render_rgba(scene);
    assert_eq!(
        rgba.len(),
        scene.width as usize * scene.height as usize * 4,
        "render_rgba must return one RGBA pixel per canvas pixel"
    );
    Pixels {
        width: scene.width,
        height: scene.height,
        rgba,
    }
}

/// The half-open pixel box [x0, x1) x [y0, y1).
struct PixelBox {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl PixelBox {
    fn contains(&self, x: i64, y: i64) -> bool {
        (self.x0..self.x1).contains(&x) && (self.y0..self.y1).contains(&y)
    }

    fn area(&self) -> usize {
        ((self.x1 - self.x0).max(0) * (self.y1 - self.y0).max(0)) as usize
    }
}

/// Pixels the rectangle may touch, anti-aliased edges included.
fn rect_outer(rect: &Rect) -> PixelBox {
    PixelBox {
        x0: rect.x.floor() as i64 - 1,
        y0: rect.y.floor() as i64 - 1,
        x1: (rect.x + rect.width).ceil() as i64 + 1,
        y1: (rect.y + rect.height).ceil() as i64 + 1,
    }
}

/// Pixels fully inside the rectangle, one pixel in from its edges.
fn rect_inner(rect: &Rect) -> PixelBox {
    PixelBox {
        x0: rect.x.ceil() as i64 + 1,
        y0: rect.y.ceil() as i64 + 1,
        x1: (rect.x + rect.width).floor() as i64 - 1,
        y1: (rect.y + rect.height).floor() as i64 - 1,
    }
}

/// The only place one line of text may put ink: from 1.25 em above the
/// baseline to 0.5 em below it, starting half an em before the text origin.
fn text_line(scene: &SandboxScene) -> PixelBox {
    PixelBox {
        x0: (scene.text_x - 0.5 * scene.font_size).floor() as i64,
        y0: (scene.text_baseline - 1.25 * scene.font_size).floor() as i64,
        x1: i64::MAX,
        y1: (scene.text_baseline + 0.5 * scene.font_size).ceil() as i64,
    }
}

fn distance(a: Rgb, b: Rgb) -> u32 {
    a.iter()
        .zip(b)
        .map(|(&x, y)| u32::from(x.abs_diff(y)).pow(2))
        .sum()
}

fn close(a: Rgb, b: Rgb, tolerance: u8) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= tolerance)
}

/// What a rendered scene contains, measured against what the scene describes.
struct Drawn {
    /// Pixels outside the text line that are not the fill the scene puts there.
    stray: Vec<(i64, i64, [u8; 4])>,
    /// Pixels that are not fully opaque.
    translucent: usize,
    /// Pixels inside the rectangle, outside the text line, that have the rectangle color.
    rect_pixels: usize,
    /// Text-line pixels closer to the text color than to the fill beneath them.
    ink: Vec<(i64, i64, Rgb)>,
}

impl Drawn {
    fn ink_columns(&self) -> (i64, i64) {
        let xs = self.ink.iter().map(|&(x, _, _)| x);
        (
            xs.clone().min().expect("the text has ink"),
            xs.max().expect("the text has ink"),
        )
    }

    fn ink_rows(&self) -> (i64, i64) {
        let ys = self.ink.iter().map(|&(_, y, _)| y);
        (
            ys.clone().min().expect("the text has ink"),
            ys.max().expect("the text has ink"),
        )
    }
}

fn inspect(scene: &SandboxScene, pixels: &Pixels) -> Drawn {
    let outer = rect_outer(&scene.rect);
    let inner = rect_inner(&scene.rect);
    let line = text_line(scene);
    let has_text = !scene.text.is_empty();
    let mut drawn = Drawn {
        stray: Vec::new(),
        translucent: 0,
        rect_pixels: 0,
        ink: Vec::new(),
    };
    for y in 0..pixels.height {
        for x in 0..pixels.width {
            let pixel = pixels.at(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];
            if pixel[3] != 255 {
                drawn.translucent += 1;
            }
            let (x, y) = (i64::from(x), i64::from(y));
            let in_rect = inner.contains(x, y);
            let fill = if in_rect {
                scene.rect_color
            } else if outer.contains(x, y) {
                // An anti-aliased rectangle edge may blend both fills.
                continue;
            } else {
                scene.background
            };
            if has_text && line.contains(x, y) {
                if distance(rgb, scene.text_color) < distance(rgb, fill) {
                    drawn.ink.push((x, y, rgb));
                }
            } else if !close(rgb, fill, FILL_TOLERANCE) {
                drawn.stray.push((x, y, pixel));
            } else if in_rect {
                drawn.rect_pixels += 1;
            }
        }
    }
    drawn
}

fn assert_only_the_scene_is_drawn(scene: &SandboxScene, drawn: &Drawn) {
    assert_eq!(
        drawn.translucent, 0,
        "every canvas pixel must be opaque for {scene:?}"
    );
    assert!(
        drawn.stray.is_empty(),
        "{} pixels outside the rectangle and the text line differ from the scene (first: {:?}) for {scene:?}",
        drawn.stray.len(),
        &drawn.stray[..drawn.stray.len().min(5)]
    );
}

/// A 320x160 scene: a rectangle in the top-left corner and `text` at 32 px,
/// starting at x = 120 on a baseline at y = 112.
fn test_scene(text: &str) -> SandboxScene {
    let mut scene = SandboxScene::demo();
    scene.width = 320;
    scene.height = 160;
    scene.background = [255, 255, 255];
    scene.rect = Rect {
        x: 16.0,
        y: 16.0,
        width: 64.0,
        height: 48.0,
    };
    scene.rect_color = [37, 99, 235];
    scene.text = text.to_string();
    scene.text_x = 120.0;
    scene.text_baseline = 112.0;
    scene.font_size = 32.0;
    scene.text_color = [17, 24, 39];
    scene
}

#[test]
fn demo_scene_is_one_visible_rectangle_and_one_line_of_text_on_the_canvas() {
    let scene = SandboxScene::demo();
    let (width, height) = (scene.width as f32, scene.height as f32);
    assert!(
        scene.width > 0 && scene.height > 0,
        "the canvas has a size: {scene:?}"
    );

    let rect = scene.rect;
    assert!(
        rect.width >= 8.0 && rect.height >= 8.0,
        "the rectangle must be big enough to see: {rect:?}"
    );
    assert!(
        rect.x >= 0.0
            && rect.y >= 0.0
            && rect.x + rect.width <= width
            && rect.y + rect.height <= height,
        "the rectangle must lie on the {width}x{height} canvas: {rect:?}"
    );
    assert_ne!(
        scene.rect_color, scene.background,
        "the rectangle must stand out from the background"
    );

    assert!(
        !scene.text.trim().is_empty(),
        "the scene must show some text"
    );
    assert!(
        !scene.text.contains(['\n', '\r']),
        "the text must be one line: {:?}",
        scene.text
    );
    assert!(
        scene.font_size >= 10.0,
        "the text must be readable, got font size {}",
        scene.font_size
    );
    assert!(
        scene.text_x >= 0.0 && scene.text_x < width,
        "the text must start on the canvas: {scene:?}"
    );
    assert!(
        scene.text_baseline - scene.font_size >= 0.0
            && scene.text_baseline + 0.25 * scene.font_size <= height,
        "the text line must fit on the canvas: {scene:?}"
    );
    assert_ne!(
        scene.text_color, scene.background,
        "the text must stand out from the background"
    );
}

#[test]
fn demo_scene_renders_a_rectangle_and_one_line_of_text_and_nothing_else() {
    let scene = SandboxScene::demo();
    let drawn = inspect(&scene, &render(&scene));
    assert_only_the_scene_is_drawn(&scene, &drawn);
    assert!(
        drawn.rect_pixels >= 16,
        "the rectangle must be drawn in its color, found {} matching pixels for {scene:?}",
        drawn.rect_pixels
    );
    assert!(
        drawn.ink.len() >= 20,
        "the text must be drawn in its color, found {} ink pixels for {scene:?}",
        drawn.ink.len()
    );
    let (left, right) = drawn.ink_columns();
    assert!(
        right - left >= scene.font_size as i64,
        "the text ink spans only columns {left}..={right} for {scene:?}"
    );
}

#[test]
fn render_rgba_fills_the_rectangle_and_background_the_scene_describes() {
    let first = test_scene("");
    let mut second = test_scene("");
    second.background = [241, 245, 249];
    second.rect = Rect {
        x: 180.5,
        y: 70.25,
        width: 120.0,
        height: 72.0,
    };
    second.rect_color = [220, 38, 38];

    for scene in [&first, &second] {
        let drawn = inspect(scene, &render(scene));
        assert_only_the_scene_is_drawn(scene, &drawn);
        assert!(
            drawn.ink.is_empty(),
            "an empty text must leave no ink for {scene:?}"
        );
        assert_eq!(
            drawn.rect_pixels,
            rect_inner(&scene.rect).area(),
            "every pixel inside the rectangle must have its color for {scene:?}"
        );
    }
}

#[test]
fn render_rgba_draws_the_text_as_one_line_on_its_baseline_in_its_color() {
    let scene = test_scene("Skia");
    let drawn = inspect(&scene, &render(&scene));
    assert_only_the_scene_is_drawn(&scene, &drawn);
    assert!(
        drawn.ink.len() >= 40,
        "\"Skia\" at 32 px must leave ink, found {} pixels",
        drawn.ink.len()
    );

    let (left, _) = drawn.ink_columns();
    assert!(
        left <= (scene.text_x + scene.font_size) as i64,
        "the text must start at x = {}, its ink starts at {left}",
        scene.text_x
    );
    let (top, _) = drawn.ink_rows();
    assert!(
        top <= (scene.text_baseline - 0.5 * scene.font_size) as i64,
        "the glyphs must stand on the baseline at y = {}, their ink starts at {top}",
        scene.text_baseline
    );
    assert!(
        drawn
            .ink
            .iter()
            .any(|&(_, _, rgb)| close(rgb, scene.text_color, 24)),
        "solid glyph pixels must have the text color {:?}",
        scene.text_color
    );
}

#[test]
fn render_rgba_draws_the_whole_string() {
    let short = test_scene("Skia");
    let long = test_scene("Skia text");
    let short_drawn = inspect(&short, &render(&short));
    let long_drawn = inspect(&long, &render(&long));
    assert_only_the_scene_is_drawn(&short, &short_drawn);
    assert_only_the_scene_is_drawn(&long, &long_drawn);

    let (_, short_right) = short_drawn.ink_columns();
    let (_, long_right) = long_drawn.ink_columns();
    assert!(
        long_right >= short_right + long.font_size as i64,
        "\"Skia text\" must reach further right than \"Skia\": {long_right} vs {short_right}"
    );
}
