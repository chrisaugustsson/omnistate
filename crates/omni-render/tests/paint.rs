//! US-005: every node type is drawn on the canvas from the layout boxes of
//! US-003, as the design specifies. The tests draw designs natively with
//! `omni_render::paint` on a CPU raster surface (the WebGL canvas draws through
//! the same functions) and look at the pixels.
//!
//! What the tests expect of drawing, where the design format leaves room:
//!
//! - One canvas pixel is one CSS pixel, with the page origin at the top-left
//!   corner. Boxes at whole pixels have crisp edges.
//! - A frame paints its shadows, then its fills, then its strokes, then its
//!   children, and fills, strokes and shadows each go in list order, with the
//!   last fill and stroke on top and the first shadow on top.
//! - A stroke is drawn inside the edge of a frame or an image, like a CSS
//!   `inset` border, so it takes no space and follows the corner radii. The
//!   stroke of a vector is centered on its path.
//! - A frame's stroke may set `align` (US-053): `inside` is the default above,
//!   `center` straddles the edge, half inside and half outside, and `outside`
//!   lies around the box like a CSS `outline` (or a `box-shadow` spread),
//!   whose corners follow the radii grown by the stroke width.
//! - A stroke may be dashed (US-053): `"dash": {"length": <px>, "gap": <px>}`
//!   is SVG `stroke-dasharray: <length> <gap>` with butt ends, along the line
//!   that runs through the middle of the stroke. Around a box that line starts
//!   at its top-left corner and runs clockwise, as an SVG `<rect>` does, and
//!   along a vector it follows the path.
//! - A frame's radii that are too large for its box are scaled down, as CSS
//!   does. `clip` cuts children at the rounded box, not the frame's own shadow.
//! - A shadow is a CSS `box-shadow`: a blur of `b` is a Gaussian of standard
//!   deviation `b / 2`, a drop shadow is not drawn beneath its own box, and an
//!   inner shadow is drawn inside the box edge, offset toward the far side.
//! - `opacity` fades a node and everything below it together, as one group.
//! - A gradient follows CSS `linear-gradient(<angle>deg, ...)`.
//! - Images fit like CSS `object-fit`, centered. The path of a vector is in
//!   pixels from the top-left corner of its box.
//! - Text is drawn black unless it has a `color`, in the lines and at the
//!   baselines that layout computed: the lines Chrome breaks, which Skia's own
//!   line breaking does not give.
//! - An image that the host did not supply draws nothing.

mod design;

use design::{
    at, board, box_of, fixed, flex, frame, lay_out, node, none, rect, text as hug_text, with,
};
use omni_core::design::AttachmentRef;
use omni_render::layout;
use omni_render::paint::{self, Assets};
use serde_json::{Value, json};
use skia_safe::{AlphaType, ColorType, EncodedImageFormat, ImageInfo, Paint, surfaces};

/// An attachment the images of these tests are supplied under.
const PHOTO: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd";
const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 160, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const YELLOW: [u8; 3] = [255, 255, 0];

/// Inter's ascent and descent, in ems.
const INTER_ASCENT: f32 = 0.96875;
const INTER_DESCENT: f32 = 3.859375 / 16.0;

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) is outside the {} x {} canvas",
            self.width,
            self.height
        );
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let [r, g, b, _] = self.at(x, y);
        [r, g, b]
    }

    /// How dark the pixel is: 0 for white, 255 for the darkest channel at 0.
    fn ink(&self, x: u32, y: u32) -> u8 {
        let [r, g, b] = self.rgb(x, y);
        255 - r.min(g).min(b)
    }

    /// Whether the pixel is more than half covered with ink.
    fn inked(&self, x: u32, y: u32) -> bool {
        self.ink(x, y) > 127
    }

    /// The bounding box `[x0, y0, x1, y1)` (end exclusive) of the inked pixels
    /// in the region `[x0, y0, x1, y1)`, or `None` if there are none.
    fn ink_box(&self, [x0, y0, x1, y1]: [u32; 4]) -> Option<[u32; 4]> {
        let mut found: Option<[u32; 4]> = None;
        for y in y0..y1.min(self.height) {
            for x in x0..x1.min(self.width) {
                if self.inked(x, y) {
                    found = Some(match found {
                        None => [x, y, x + 1, y + 1],
                        Some([a, b, c, d]) => [a.min(x), b.min(y), c.max(x + 1), d.max(y + 1)],
                    });
                }
            }
        }
        found
    }

    /// How many pixels of row `y` between `x0` and `x1` are inked.
    fn inked_in_row(&self, y: u32, x0: u32, x1: u32) -> u32 {
        (x0..x1).filter(|&x| self.inked(x, y)).count() as u32
    }

    /// The runs of consecutive rows, `[first, end)`, that have ink between
    /// `x0` and `x1`.
    fn ink_rows(&self, x0: u32, x1: u32) -> Vec<[u32; 2]> {
        let mut runs: Vec<[u32; 2]> = Vec::new();
        for y in 0..self.height {
            if self.inked_in_row(y, x0, x1) == 0 {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run[1] == y => run[1] = y + 1,
                _ => runs.push([y, y + 1]),
            }
        }
        runs
    }

    /// How many pixels differ from `other` by more than `tolerance` in a channel.
    fn differing(&self, other: &Pixels, tolerance: u8) -> usize {
        assert_eq!((self.width, self.height), (other.width, other.height));
        self.rgba
            .chunks_exact(4)
            .zip(other.rgba.chunks_exact(4))
            .filter(|(a, b)| {
                a.iter()
                    .zip(b.iter())
                    .any(|(&a, &b)| a.abs_diff(b) > tolerance)
            })
            .count()
    }
}

fn close(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= tolerance)
}

#[track_caller]
fn assert_rgb(pixels: &Pixels, x: u32, y: u32, expected: [u8; 3], tolerance: u8, what: &str) {
    let actual = pixels.rgb(x, y);
    assert!(
        close(actual, expected, tolerance),
        "{what}: pixel ({x}, {y}) must be {expected:?} (give or take {tolerance}), but it is {actual:?}"
    );
}

#[track_caller]
fn assert_all_rgb(pixels: &Pixels, spots: &[(u32, u32)], expected: [u8; 3], what: &str) {
    for &(x, y) in spots {
        assert_rgb(pixels, x, y, expected, 3, what);
    }
}

fn solid(color: &str) -> Value {
    json!({"type": "solid", "color": color})
}

/// A board `width` x `height` with a white fill and `children` placed with `x`
/// and `y`.
fn scene(width: u32, height: u32, children: Vec<Value>) -> Value {
    let root = board(
        "brd00001",
        fixed(f64::from(width)),
        fixed(f64::from(height)),
        none(),
        children,
    );
    with(&root, "fills", json!([solid("#ffffff")]))
}

/// `node` with every property of `props` set.
fn styled(node: Value, props: Value) -> Value {
    let mut node = node;
    let object = node.as_object_mut().expect("fixture nodes are objects");
    for (key, value) in props.as_object().expect("properties are an object") {
        object.insert(key.clone(), value.clone());
    }
    node
}

/// An empty frame of `width` x `height` at `x`, `y` with `props` set.
fn box_at(id: &str, x: f64, y: f64, width: f64, height: f64, props: Value) -> Value {
    styled(at(&rect(id, width, height), x, y), props)
}

/// A 200 x 200 scene with a single `width` x `height` frame at 80, 80.
fn one_box(width: f64, height: f64, props: Value) -> Value {
    scene(
        200,
        200,
        vec![box_at("bx000001", 80.0, 80.0, width, height, props)],
    )
}

fn render(design: &Value, width: u32, height: u32) -> Pixels {
    render_with(design, &Assets::new(), width, height)
}

fn render_with(design: &Value, assets: &Assets, width: u32, height: u32) -> Pixels {
    let rgba = paint::render_rgba(&node(design), assets, width, height);
    assert_eq!(
        rgba.len(),
        width as usize * height as usize * 4,
        "render_rgba must return one RGBA pixel per canvas pixel"
    );
    Pixels {
        width,
        height,
        rgba,
    }
}

/// Draws a 200 x 200 scene.
fn render_200(design: &Value) -> Pixels {
    render(design, 200, 200)
}

/// The PNG of an image `colors.len()` pixels wide and 2 tall, each column one
/// of `colors`.
fn png(colors: &[[u8; 3]]) -> Vec<u8> {
    let mut surface = surfaces::raster_n32_premul((colors.len() as i32, 2)).expect("a surface");
    for (column, [r, g, b]) in colors.iter().enumerate() {
        let mut paint = Paint::default();
        paint.set_color(skia_safe::Color::from_rgb(*r, *g, *b));
        surface.canvas().draw_rect(
            skia_safe::Rect::from_xywh(column as f32, 0.0, 1.0, 2.0),
            &paint,
        );
    }
    let data = surface
        .image_snapshot()
        .encode(
            None::<&mut skia_safe::gpu::DirectContext>,
            EncodedImageFormat::PNG,
            None,
        )
        .expect("the image encodes as PNG");
    data.as_bytes().to_vec()
}

/// The four-column image of the image tests, supplied under [`PHOTO`]: red,
/// green, blue and yellow columns in a 2:1 picture.
fn stripes() -> Assets {
    let mut assets = Assets::new();
    assets
        .add_image(&attachment(PHOTO), &png(&[RED, GREEN, BLUE, YELLOW]))
        .expect("the host can supply a PNG");
    assets
}

fn attachment(reference: &str) -> AttachmentRef {
    reference.parse().expect("a valid attachment")
}

fn image_node(id: &str, x: f64, y: f64, width: f64, height: f64, fit: &str, props: Value) -> Value {
    styled(
        at(
            &json!({
                "id": id,
                "type": "image",
                "name": format!("Image {id}"),
                "width": fixed(width),
                "height": fixed(height),
                "image": PHOTO,
                "fit": fit
            }),
            x,
            y,
        ),
        props,
    )
}

fn vector_node(id: &str, x: f64, y: f64, path: &str, props: Value) -> Value {
    styled(
        at(
            &json!({
                "id": id,
                "type": "vector",
                "name": format!("Vector {id}"),
                "width": fixed(40.0),
                "height": fixed(40.0),
                "path": path
            }),
            x,
            y,
        ),
        props,
    )
}

/// A text node at `x`, `y` that hugs its content: `size` px Inter, lines
/// `line_height` times as tall, with `props` set.
fn text_at(
    id: &str,
    x: f64,
    y: f64,
    content: &str,
    size: f64,
    line_height: f64,
    props: Value,
) -> Value {
    styled(at(&hug_text(id, content, size, line_height), x, y), props)
}

// ---------------------------------------------------------------------------
// Frames: fills
// ---------------------------------------------------------------------------

#[test]
fn a_frame_is_filled_with_its_solid_color_inside_its_box_and_nowhere_else() {
    let pixels = render_200(&one_box(40.0, 30.0, json!({"fills": [solid("#ff0000")]})));
    assert_all_rgb(
        &pixels,
        &[(80, 80), (119, 109), (100, 95)],
        RED,
        "inside the box",
    );
    assert_all_rgb(
        &pixels,
        &[(79, 95), (120, 95), (100, 79), (100, 110), (10, 10)],
        WHITE,
        "outside the box, crisp at whole pixels",
    );
}

#[test]
fn fills_paint_in_list_order_with_the_last_on_top() {
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({"fills": [solid("#ff0000"), solid("#0000ff80")]}),
    ));
    // Half-transparent blue over red: (127, 0, 128).
    assert_rgb(
        &pixels,
        100,
        100,
        [127, 0, 128],
        3,
        "the second fill over the first",
    );

    let swapped = render_200(&one_box(
        40.0,
        40.0,
        json!({"fills": [solid("#0000ff80"), solid("#ff0000")]}),
    ));
    assert_rgb(
        &swapped,
        100,
        100,
        RED,
        3,
        "an opaque last fill covers the first",
    );
}

fn gradient_fill(angle: f64, stops: &[(f32, &str)]) -> Value {
    json!({
        "type": "linear-gradient",
        "angle": angle,
        "stops": stops
            .iter()
            .map(|(position, color)| json!({"position": position, "color": color}))
            .collect::<Vec<_>>()
    })
}

#[test]
fn a_linear_gradient_follows_the_css_gradient_line_at_any_angle() {
    // Angles are degrees clockwise from pointing up, so 0 runs bottom to top
    // and 90 left to right. The corners lie at the ends of the CSS gradient
    // line.
    for (angle, width, height, x, y, expected) in [
        (90.0, 100.0, 20.0, 90, 90, [228, 0, 27]),
        (90.0, 100.0, 20.0, 169, 90, [27, 0, 228]),
        (180.0, 20.0, 100.0, 90, 90, [228, 0, 27]),
        (180.0, 20.0, 100.0, 90, 169, [27, 0, 228]),
        (0.0, 20.0, 100.0, 90, 90, [27, 0, 228]),
        (270.0, 100.0, 20.0, 90, 90, [27, 0, 228]),
        (45.0, 100.0, 50.0, 82, 82, [170, 0, 85]),
        (45.0, 100.0, 50.0, 177, 82, [8, 0, 246]),
        (45.0, 100.0, 50.0, 82, 127, [246, 0, 8]),
        (135.0, 100.0, 50.0, 82, 82, [246, 0, 8]),
        (135.0, 100.0, 50.0, 177, 82, [85, 0, 170]),
        (135.0, 100.0, 50.0, 177, 127, [8, 0, 246]),
    ] {
        let pixels = render_200(&one_box(
            width,
            height,
            json!({"fills": [gradient_fill(angle, &[(0.0, "#ff0000"), (1.0, "#0000ff")])]}),
        ));
        assert_rgb(
            &pixels,
            x,
            y,
            expected,
            9,
            &format!("a {width} x {height} box with a gradient at {angle} degrees, at {x}, {y}"),
        );
    }
}

#[test]
fn a_gradient_holds_its_end_colors_beyond_its_first_and_last_stops() {
    let pixels = render_200(&one_box(
        100.0,
        20.0,
        json!({"fills": [gradient_fill(
            90.0,
            &[(0.25, "#ff0000"), (0.5, "#00a000"), (0.75, "#0000ff")],
        )]}),
    ));
    for (x, expected) in [
        (83, RED),
        (92, RED),
        (117, [128, 80, 0]),
        (142, [0, 80, 128]),
        (168, BLUE),
        (176, BLUE),
    ] {
        assert_rgb(
            &pixels,
            x,
            90,
            expected,
            9,
            &format!("a gradient with three stops, at x {x}"),
        );
    }
}

// ---------------------------------------------------------------------------
// Frames: strokes, radii
// ---------------------------------------------------------------------------

#[test]
fn a_stroke_is_drawn_inside_the_edge_of_the_box_and_takes_no_space() {
    let design = one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ff0000")],
            "strokes": [{"color": "#0000ff", "width": 4}]
        }),
    );
    let pixels = render_200(&design);
    assert_all_rgb(
        &pixels,
        &[
            (80, 100),
            (83, 100),
            (116, 100),
            (119, 100),
            (100, 80),
            (100, 83),
            (100, 116),
            (100, 119),
        ],
        BLUE,
        "the 4 px stroke along the edges",
    );
    assert_all_rgb(
        &pixels,
        &[(84, 100), (115, 100), (100, 84), (100, 115), (100, 100)],
        RED,
        "the fill inside the stroke",
    );
    assert_all_rgb(
        &pixels,
        &[(79, 100), (120, 100), (100, 79), (100, 120)],
        WHITE,
        "outside the box, where the stroke does not reach",
    );
    let boxes = lay_out(&design);
    let frame_box = box_of(&boxes, "bx000001");
    assert_eq!(
        [frame_box.width, frame_box.height],
        [40.0, 40.0],
        "a stroke takes no space"
    );
}

#[test]
fn strokes_paint_in_list_order_with_the_last_on_top() {
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({"strokes": [
            {"color": "#ff0000", "width": 8},
            {"color": "#0000ff", "width": 3}
        ]}),
    ));
    assert_all_rgb(
        &pixels,
        &[(80, 100), (82, 100)],
        BLUE,
        "the narrow stroke on top",
    );
    assert_all_rgb(
        &pixels,
        &[(83, 100), (87, 100)],
        RED,
        "the wide stroke beyond it",
    );
    assert_all_rgb(&pixels, &[(88, 100), (100, 100)], WHITE, "an empty middle");
}

#[test]
fn each_corner_has_its_own_radius() {
    let pixels = render_200(&one_box(
        80.0,
        80.0,
        json!({
            "fills": [solid("#ff0000")],
            "radius": {"topLeft": 30, "topRight": 20, "bottomRight": 10, "bottomLeft": 0}
        }),
    ));
    // Each corner pixel of a rounded corner is cut away; a square corner is not.
    assert_rgb(&pixels, 81, 81, WHITE, 3, "the top-left corner, radius 30");
    assert_rgb(
        &pixels,
        158,
        81,
        WHITE,
        3,
        "the top-right corner, radius 20",
    );
    assert_rgb(
        &pixels,
        158,
        158,
        WHITE,
        3,
        "the bottom-right corner, radius 10",
    );
    assert_rgb(&pixels, 81, 158, RED, 3, "the bottom-left corner, radius 0");
    // A point inside each arc is filled.
    assert_rgb(&pixels, 90, 90, RED, 3, "inside the top-left arc");
    assert_rgb(&pixels, 150, 90, RED, 3, "inside the top-right arc");
    assert_rgb(&pixels, 152, 152, RED, 3, "inside the bottom-right arc");
    assert_rgb(&pixels, 120, 80, RED, 3, "the straight top edge");
}

#[test]
fn a_radius_larger_than_the_box_allows_is_scaled_down_as_css_does() {
    let radius = json!({"topLeft": 100, "topRight": 100, "bottomRight": 100, "bottomLeft": 100});
    // 40 x 40: a circle.
    let circle = render_200(&one_box(
        40.0,
        40.0,
        json!({"fills": [solid("#ff0000")], "radius": radius}),
    ));
    assert_rgb(&circle, 81, 81, WHITE, 3, "the corner of a circle");
    assert_all_rgb(
        &circle,
        &[(100, 81), (81, 100), (118, 100), (100, 118), (100, 100)],
        RED,
        "the circle's points",
    );
    // 60 x 20: a pill, whose radius is half its height.
    let pill = render_200(&one_box(
        60.0,
        20.0,
        json!({"fills": [solid("#ff0000")], "radius": radius}),
    ));
    assert_rgb(&pill, 81, 81, WHITE, 3, "the corner of a pill");
    assert_rgb(
        &pill,
        138,
        98,
        WHITE,
        3,
        "the bottom-right corner of a pill",
    );
    assert_all_rgb(
        &pill,
        &[(110, 81), (110, 98), (81, 90), (138, 90)],
        RED,
        "the pill's points",
    );
}

#[test]
fn a_stroke_follows_the_corner_radii() {
    let radius = json!({"topLeft": 30, "topRight": 30, "bottomRight": 30, "bottomLeft": 30});
    let pixels = render_200(&one_box(
        80.0,
        80.0,
        json!({
            "fills": [solid("#ff0000")],
            "strokes": [{"color": "#0000ff", "width": 4}],
            "radius": radius
        }),
    ));
    assert_rgb(&pixels, 81, 81, WHITE, 3, "outside the rounded corner");
    assert_rgb(
        &pixels,
        120,
        80,
        BLUE,
        3,
        "the stroke on the straight top edge",
    );
    assert_rgb(&pixels, 120, 85, RED, 3, "the fill inside the stroke");
    // On the arc, 27.6 px from its center at (110, 110): inside the 26..30 px
    // band that the 4 px stroke covers.
    assert_rgb(
        &pixels,
        90,
        90,
        BLUE,
        3,
        "the stroke on the arc at 45 degrees",
    );
    assert_rgb(
        &pixels,
        96,
        96,
        RED,
        3,
        "the fill inside the stroke on the arc",
    );
}

/// A 40 x 40 red frame at 80, 80 with one 4 px blue stroke that sets `align`.
fn aligned_stroke(align: &str) -> Value {
    one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ff0000")],
            "strokes": [{"color": "#0000ff", "width": 4, "align": align}]
        }),
    )
}

#[test]
fn a_frame_stroke_aligned_inside_lies_inside_the_edge() {
    let pixels = render_200(&aligned_stroke("inside"));
    assert_all_rgb(
        &pixels,
        &[
            (80, 100),
            (83, 100),
            (116, 100),
            (119, 100),
            (100, 80),
            (100, 119),
        ],
        BLUE,
        "the 4 px stroke inside the edge, from 80 to 84 and from 116 to 120",
    );
    assert_all_rgb(
        &pixels,
        &[(84, 100), (115, 100)],
        RED,
        "the fill inside the stroke",
    );
    assert_all_rgb(
        &pixels,
        &[(79, 100), (120, 100), (100, 79), (100, 120)],
        WHITE,
        "outside the box",
    );
}

#[test]
fn a_frame_stroke_aligned_to_the_center_straddles_the_edge() {
    let pixels = render_200(&aligned_stroke("center"));
    assert_all_rgb(
        &pixels,
        &[
            (78, 100),
            (81, 100),
            (118, 100),
            (121, 100),
            (100, 78),
            (100, 81),
            (100, 118),
            (100, 121),
        ],
        BLUE,
        "the 4 px stroke from 2 px outside the edge to 2 px inside it",
    );
    assert_all_rgb(
        &pixels,
        &[(82, 100), (117, 100), (100, 82), (100, 117)],
        RED,
        "the fill inside the stroke",
    );
    assert_all_rgb(
        &pixels,
        &[(77, 100), (122, 100), (100, 77), (100, 122)],
        WHITE,
        "beyond the half of the stroke outside the edge",
    );
    let frame_box = box_of(&lay_out(&aligned_stroke("center")), "bx000001");
    assert_eq!(
        [frame_box.x, frame_box.y, frame_box.width, frame_box.height],
        [80.0, 80.0, 40.0, 40.0],
        "a stroke takes no space wherever it sits"
    );
}

#[test]
fn a_frame_stroke_aligned_outside_lies_around_the_box_like_a_css_outline() {
    let pixels = render_200(&aligned_stroke("outside"));
    assert_all_rgb(
        &pixels,
        &[
            (76, 100),
            (79, 100),
            (120, 100),
            (123, 100),
            (100, 76),
            (100, 79),
            (100, 120),
            (100, 123),
        ],
        BLUE,
        "the 4 px stroke outside the edge, from 76 to 80 and from 120 to 124",
    );
    assert_all_rgb(
        &pixels,
        &[(80, 100), (119, 100), (100, 80), (100, 119)],
        RED,
        "the fill, which the stroke leaves alone up to the edge",
    );
    assert_all_rgb(
        &pixels,
        &[(75, 100), (124, 100), (100, 75), (100, 124)],
        WHITE,
        "beyond the stroke",
    );
    let frame_box = box_of(&lay_out(&aligned_stroke("outside")), "bx000001");
    assert_eq!(
        [frame_box.x, frame_box.y, frame_box.width, frame_box.height],
        [80.0, 80.0, 40.0, 40.0],
        "a stroke outside the box takes no space either"
    );
}

#[test]
fn a_frame_stroke_outside_the_box_follows_the_radii_grown_by_its_width() {
    let radius = json!({"topLeft": 30, "topRight": 30, "bottomRight": 30, "bottomLeft": 30});
    let pixels = render_200(&one_box(
        80.0,
        80.0,
        json!({
            "fills": [solid("#ff0000")],
            "strokes": [{"color": "#0000ff", "width": 4, "align": "outside"}],
            "radius": radius
        }),
    ));
    assert_rgb(
        &pixels,
        120,
        77,
        BLUE,
        3,
        "the stroke above the straight top edge",
    );
    // 46 px from the center of the arc at (110, 110), beyond the 34 px that
    // the 30 px radius grown by the 4 px stroke reaches, where a square
    // outline would be.
    assert_rgb(&pixels, 77, 77, WHITE, 3, "outside the rounded outline");
    // 33.2 px from the center of the arc: outside the rounded box, inside the
    // 30..34 px band of the stroke.
    assert_rgb(
        &pixels,
        86,
        86,
        BLUE,
        3,
        "the stroke around the arc at 45 degrees",
    );
}

/// A box of 80 x 40 at 80, 80 without a fill and with one 2 px blue stroke,
/// dashed 10 px on and 10 px off, that sets `align`.
fn dashed_box(align: &str) -> Value {
    one_box(
        80.0,
        40.0,
        json!({
            "strokes": [{
                "color": "#0000ff",
                "width": 2,
                "align": align,
                "dash": {"length": 10, "gap": 10}
            }]
        }),
    )
}

#[test]
fn a_dashed_stroke_inside_the_edge_starts_its_first_dash_at_the_top_left_corner() {
    // The middle of the stroke runs 1 px inside the edge, from (81, 81): dashes
    // from 81 to 91, 101 to 111 and 121 to 131 along the top, in the row of
    // pixels from y 80 to 82.
    let pixels = render_200(&dashed_box("inside"));
    assert_all_rgb(&pixels, &[(86, 81), (106, 81), (126, 81)], BLUE, "a dash");
    assert_all_rgb(&pixels, &[(96, 81), (116, 81)], WHITE, "a gap");
}

#[test]
fn a_dashed_stroke_on_the_edge_starts_its_first_dash_at_the_top_left_corner() {
    // The middle of the stroke is the edge, from (80, 80): dashes from 80 to
    // 90, 100 to 110 and 120 to 130, from y 79 to 81.
    let pixels = render_200(&dashed_box("center"));
    assert_all_rgb(&pixels, &[(85, 80), (105, 80), (125, 80)], BLUE, "a dash");
    assert_all_rgb(&pixels, &[(95, 80), (115, 80)], WHITE, "a gap");
}

#[test]
fn a_dashed_stroke_outside_the_edge_starts_its_first_dash_at_the_top_left_corner() {
    // The middle of the stroke runs 1 px outside the edge, from (79, 79):
    // dashes from 79 to 89, 99 to 109 and 119 to 129, from y 78 to 80.
    let pixels = render_200(&dashed_box("outside"));
    assert_all_rgb(&pixels, &[(84, 79), (104, 79), (124, 79)], BLUE, "a dash");
    assert_all_rgb(&pixels, &[(94, 79), (114, 79)], WHITE, "a gap");
}

#[test]
fn a_dashed_stroke_without_an_align_lies_inside_the_edge() {
    let design = one_box(
        80.0,
        40.0,
        json!({"strokes": [{"color": "#0000ff", "width": 2, "dash": {"length": 10, "gap": 10}}]}),
    );
    let pixels = render_200(&design);
    assert_all_rgb(&pixels, &[(86, 81), (106, 81)], BLUE, "a dash");
    assert_all_rgb(&pixels, &[(96, 81), (116, 81)], WHITE, "a gap");
}

#[test]
fn a_dashed_vector_stroke_follows_its_path_from_where_it_starts() {
    let line = vector_node(
        "vec00001",
        80.0,
        80.0,
        "M0 20 L40 20",
        json!({"strokes": [{"color": "#0000ff", "width": 6, "dash": {"length": 10, "gap": 10}}]}),
    );
    let pixels = render_200(&scene(200, 200, vec![line]));
    assert_all_rgb(
        &pixels,
        &[(85, 100), (105, 100), (85, 98), (105, 102)],
        BLUE,
        "the dashes from 80 to 90 and from 100 to 110",
    );
    assert_all_rgb(
        &pixels,
        &[(95, 100), (115, 100)],
        WHITE,
        "the gaps from 90 to 100 and from 110 to 120",
    );
}

// ---------------------------------------------------------------------------
// Frames: shadows
// ---------------------------------------------------------------------------

fn shadow(kind: &str, color: &str, x: f64, y: f64, blur: f64, spread: f64) -> Value {
    json!({
        "type": kind,
        "color": color,
        "offsetX": x,
        "offsetY": y,
        "blur": blur,
        "spread": spread
    })
}

#[test]
fn a_drop_shadow_falls_outside_the_box_by_its_offsets() {
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ff0000")],
            "shadows": [shadow("drop", "#000000", 8.0, 8.0, 0.0, 0.0)]
        }),
    ));
    // The shadow covers 88..128 on both axes, partly beneath the box.
    assert_all_rgb(
        &pixels,
        &[(124, 100), (100, 124), (126, 126)],
        BLACK,
        "the shadow beside the box",
    );
    assert_all_rgb(
        &pixels,
        &[(100, 100), (82, 82), (119, 119)],
        RED,
        "the box over its shadow",
    );
    assert_all_rgb(
        &pixels,
        &[(84, 124), (124, 84), (130, 130), (76, 100), (100, 76)],
        WHITE,
        "beyond the shadow",
    );
}

#[test]
fn a_drop_shadow_is_not_drawn_beneath_a_translucent_box() {
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ff000080")],
            "shadows": [shadow("drop", "#000000", 8.0, 8.0, 0.0, 0.0)]
        }),
    ));
    assert_rgb(
        &pixels,
        110,
        110,
        [255, 127, 127],
        3,
        "red over white, with no shadow beneath",
    );
    assert_rgb(&pixels, 124, 124, BLACK, 3, "the shadow outside the box");
}

#[test]
fn a_blurred_drop_shadow_fades_with_a_standard_deviation_of_half_the_blur() {
    // Blur 12: sigma 6. An edge is half dark (a normal curve at 0), so the
    // pixel centered 0.5 px outside it is 47% dark, 6.5 px out 14%, 20 px 0%.
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ffffff")],
            "shadows": [shadow("drop", "#000000", 0.0, 0.0, 12.0, 0.0)]
        }),
    ));
    let dark = |x: u32| f32::from(pixels.ink(x, 100)) / 255.0;
    assert!(
        (0.35..=0.58).contains(&dark(79)),
        "0.5 px outside the edge is about 47% dark, but it is {}",
        dark(79)
    );
    assert!(
        (0.07..=0.22).contains(&dark(73)),
        "6.5 px outside is about 14% dark, but it is {}",
        dark(73)
    );
    assert!(
        dark(60) < 0.03,
        "20 px outside is clear, but it is {}",
        dark(60)
    );
    assert!(
        (60..80)
            .map(dark)
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] <= pair[1] + 0.01),
        "the shadow grows darker toward the box"
    );
    let far = pixels.ink(10, 10);
    assert_eq!(far, 0, "no shadow far from the box");
}

#[test]
fn spread_grows_a_shadow_on_every_side() {
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ff0000")],
            "shadows": [shadow("drop", "#000000", 0.0, 0.0, 0.0, 5.0)]
        }),
    ));
    assert_all_rgb(
        &pixels,
        &[(76, 100), (123, 100), (100, 76), (100, 123), (76, 76)],
        BLACK,
        "inside the 5 px ring",
    );
    assert_all_rgb(
        &pixels,
        &[(74, 100), (125, 100), (100, 74), (100, 125)],
        WHITE,
        "beyond the ring",
    );
}

#[test]
fn an_inner_shadow_falls_inside_the_box_edge_on_the_side_it_is_offset_away_from() {
    // A CSS `inset` shadow offset 6 px to the right shows 6 px wide along the
    // left edge, and nothing at all is drawn outside the box.
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({
            "fills": [solid("#ff0000")],
            "shadows": [shadow("inner", "#000000", 6.0, 0.0, 0.0, 0.0)]
        }),
    ));
    assert_all_rgb(
        &pixels,
        &[(80, 100), (85, 100), (83, 82), (84, 117)],
        BLACK,
        "the shadow along the left edge",
    );
    assert_all_rgb(
        &pixels,
        &[(86, 100), (100, 100), (119, 100), (100, 80), (100, 119)],
        RED,
        "the fill the shadow leaves",
    );
    assert_all_rgb(
        &pixels,
        &[(79, 100), (120, 100), (100, 79), (100, 120)],
        WHITE,
        "outside the box",
    );
}

#[test]
fn the_first_shadow_is_on_top() {
    let pixels = render_200(&one_box(
        40.0,
        40.0,
        json!({
            "shadows": [
                shadow("drop", "#ff0000", 4.0, 0.0, 0.0, 0.0),
                shadow("drop", "#0000ff", 8.0, 0.0, 0.0, 0.0)
            ]
        }),
    ));
    assert_rgb(
        &pixels,
        122,
        100,
        RED,
        3,
        "where both shadows fall, the first",
    );
    assert_rgb(&pixels, 126, 100, BLUE, 3, "where only the second falls");
}

// ---------------------------------------------------------------------------
// Frames: opacity, clipping, visibility, order
// ---------------------------------------------------------------------------

#[test]
fn opacity_fades_a_frame_and_everything_in_it_together_as_one_group() {
    let parent = box_at(
        "frm00001",
        20.0,
        20.0,
        100.0,
        100.0,
        json!({
            "fills": [solid("#ff0000")],
            "opacity": 0.5,
            "children": [box_at("frm00002", 20.0, 20.0, 40.0, 40.0, json!({"fills": [solid("#0000ff")]}))]
        }),
    );
    let pixels = render_200(&scene(200, 200, vec![parent]));
    assert_rgb(
        &pixels,
        30,
        30,
        [255, 127, 127],
        4,
        "half the red parent over white",
    );
    // The child fades with the group: half the blue over white, whatever is
    // beneath it in the group (not half the blue over half the red).
    assert_rgb(
        &pixels,
        50,
        50,
        [127, 127, 255],
        4,
        "half the blue child over white",
    );
}

#[test]
fn opacity_zero_draws_nothing_and_neither_does_a_hidden_node_or_what_is_in_it() {
    let faded = box_at(
        "frm00001",
        20.0,
        20.0,
        60.0,
        60.0,
        json!({
            "fills": [solid("#ff0000")],
            "opacity": 0,
            "shadows": [shadow("drop", "#000000", 70.0, 0.0, 0.0, 0.0)],
            "children": [box_at("frm00002", 10.0, 10.0, 20.0, 20.0, json!({"fills": [solid("#0000ff")]}))]
        }),
    );
    let hidden = box_at(
        "frm00003",
        20.0,
        110.0,
        60.0,
        60.0,
        json!({
            "fills": [solid("#ff0000")],
            "visible": false,
            "children": [box_at("frm00004", 10.0, 10.0, 20.0, 20.0, json!({"fills": [solid("#0000ff")]}))]
        }),
    );
    let pixels = render_200(&scene(200, 200, vec![faded, hidden]));
    assert!(
        pixels.ink_box([0, 0, 200, 200]).is_none(),
        "nothing is drawn: the whole canvas stays white"
    );
    assert_all_rgb(
        &pixels,
        &[(30, 30), (50, 50), (100, 50), (30, 120), (50, 140)],
        WHITE,
        "faded and hidden nodes",
    );
}

#[test]
fn clip_cuts_children_at_the_box_and_without_it_they_overflow() {
    let child = |id| {
        box_at(
            id,
            -20.0,
            -20.0,
            100.0,
            100.0,
            json!({"fills": [solid("#ff0000")]}),
        )
    };
    let clipped = box_at(
        "frm00001",
        40.0,
        40.0,
        60.0,
        60.0,
        json!({"clip": true, "children": [child("frm00002")]}),
    );
    let open = box_at(
        "frm00003",
        120.0,
        120.0,
        60.0,
        60.0,
        json!({"children": [child("frm00004")]}),
    );
    let loose = box_at(
        "frm00005",
        120.0,
        40.0,
        60.0,
        60.0,
        json!({"clip": false, "children": [child("frm00006")]}),
    );
    let pixels = render_200(&scene(200, 200, vec![clipped, open, loose]));
    assert_all_rgb(
        &pixels,
        &[(60, 60), (99, 99), (40, 99)],
        RED,
        "the child inside the clipping frame",
    );
    assert_all_rgb(
        &pixels,
        &[(30, 30), (39, 60), (60, 39)],
        WHITE,
        "the child's overflow is cut",
    );
    assert_all_rgb(
        &pixels,
        &[(110, 110), (130, 130), (140, 119)],
        RED,
        "a frame with no clip lets its child overflow",
    );
    assert_all_rgb(
        &pixels,
        &[(110, 30), (130, 50)],
        RED,
        "clip false lets its child overflow",
    );
}

#[test]
fn clip_follows_the_corner_radii_but_does_not_cut_the_frames_own_shadow() {
    let radius = json!({"topLeft": 30, "topRight": 30, "bottomRight": 30, "bottomLeft": 30});
    let frame = box_at(
        "frm00001",
        60.0,
        60.0,
        80.0,
        80.0,
        json!({
            "clip": true,
            "radius": radius,
            "shadows": [shadow("drop", "#0000ff", 0.0, 0.0, 0.0, 6.0)],
            "children": [box_at("frm00002", 0.0, 0.0, 80.0, 80.0, json!({"fills": [solid("#ff0000")]}))]
        }),
    );
    let pixels = render_200(&scene(200, 200, vec![frame]));
    assert_all_rgb(
        &pixels,
        &[(100, 100), (100, 61), (62, 100)],
        RED,
        "the child inside the rounded box",
    );
    assert_rgb(
        &pixels,
        61,
        61,
        WHITE,
        3,
        "the child is cut at the rounded corner, which the shadow does not reach",
    );
    assert_rgb(
        &pixels,
        100,
        57,
        BLUE,
        3,
        "the shadow outside the clipping frame",
    );
    assert_rgb(
        &pixels,
        57,
        100,
        BLUE,
        3,
        "the shadow beside the clipping frame",
    );
}

#[test]
fn children_are_drawn_in_order_so_the_last_is_on_top_and_in_their_layout_positions() {
    let overlapping = scene(
        200,
        200,
        vec![
            box_at(
                "frm00001",
                50.0,
                50.0,
                60.0,
                60.0,
                json!({"fills": [solid("#ff0000")]}),
            ),
            box_at(
                "frm00002",
                80.0,
                80.0,
                60.0,
                60.0,
                json!({"fills": [solid("#0000ff")]}),
            ),
        ],
    );
    let pixels = render_200(&overlapping);
    assert_rgb(&pixels, 90, 90, BLUE, 3, "the later child over the earlier");
    assert_rgb(
        &pixels,
        60,
        60,
        RED,
        3,
        "the earlier child where nothing covers it",
    );

    // Positions come from layout: a flex row places the second child after the
    // first and the gap, and a nested frame adds its ancestors' offsets.
    let row = board(
        "brd00001",
        fixed(200.0),
        fixed(200.0),
        flex(
            json!({"gap": {"row": 0, "column": 10}, "padding": {"top": 20, "right": 0, "bottom": 0, "left": 30}}),
        ),
        vec![
            styled(
                rect("frm00001", 40.0, 40.0),
                json!({"fills": [solid("#ff0000")]}),
            ),
            frame(
                "frm00002",
                fixed(40.0),
                fixed(40.0),
                none(),
                vec![box_at(
                    "frm00003",
                    10.0,
                    10.0,
                    20.0,
                    20.0,
                    json!({"fills": [solid("#0000ff")]}),
                )],
            ),
        ],
    );
    let design = with(&row, "fills", json!([solid("#ffffff")]));
    let boxes = lay_out(&design);
    let second = box_of(&boxes, "frm00002");
    let pixels = render_200(&design);
    assert_rgb(&pixels, 31, 21, RED, 3, "the first child at the padding");
    assert_rgb(
        &pixels,
        second.x as u32 + 20,
        second.y as u32 + 20,
        BLUE,
        3,
        "the grandchild at both offsets",
    );
    assert_rgb(
        &pixels,
        second.x as u32 + 5,
        second.y as u32 + 5,
        WHITE,
        3,
        "the second child has no fill",
    );
}

#[test]
fn draw_places_every_node_in_the_box_it_is_given() {
    let design = scene(
        200,
        200,
        vec![box_at(
            "frm00001",
            20.0,
            20.0,
            40.0,
            40.0,
            json!({"fills": [solid("#ff0000")]}),
        )],
    );
    let root = node(&design);
    let assets = Assets::new();
    let mut boxes = layout::compute(&root, assets.fonts());
    let moved = boxes
        .get_mut(&"frm00001".parse().expect("a node ID"))
        .expect("layout boxes the frame");
    moved.x = 120.0;
    moved.y = 130.0;

    let mut surface = surfaces::raster_n32_premul((200, 200)).expect("a surface");
    paint::draw(surface.canvas(), &root, &boxes, &assets);
    let info = ImageInfo::new((200, 200), ColorType::RGBA8888, AlphaType::Unpremul, None);
    let mut rgba = vec![0; info.compute_min_byte_size()];
    assert!(surface.read_pixels(&info, &mut rgba, info.min_row_bytes(), (0, 0)));
    let pixels = Pixels {
        width: 200,
        height: 200,
        rgba,
    };
    assert_rgb(
        &pixels,
        140,
        150,
        RED,
        3,
        "the frame where the boxes put it",
    );
    assert_rgb(&pixels, 40, 40, WHITE, 3, "and not where layout would have");
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn text_is_drawn_black_inside_its_layout_box_unless_it_has_a_color() {
    let design = scene(
        300,
        100,
        vec![text_at(
            "txt00001",
            20.0,
            20.0,
            "HHHH",
            32.0,
            1.5,
            json!({}),
        )],
    );
    let boxes = lay_out(&design);
    let text_box = box_of(&boxes, "txt00001");
    let pixels = render(&design, 300, 100);
    let ink = pixels.ink_box([0, 0, 300, 100]).expect("the text has ink");
    assert!(
        ink[0] as f32 >= text_box.x - 1.0
            && ink[2] as f32 <= text_box.x + text_box.width + 1.0
            && ink[1] as f32 >= text_box.y - 1.0
            && ink[3] as f32 <= text_box.y + text_box.height + 1.0,
        "the ink {ink:?} must stay inside the layout box {text_box:?}"
    );
    assert!(
        (ink[2] - ink[0]) as f32 >= text_box.width * 0.8,
        "four capital Hs fill most of their box: ink {ink:?}, box {text_box:?}"
    );
    let capital_height = (ink[3] - ink[1]) as f32;
    assert!(
        (capital_height - 0.727 * 32.0).abs() <= 2.0,
        "32 px Inter capitals are about 23 px tall, not {capital_height}"
    );
    let darkest = (0..300)
        .flat_map(|x| (0..100).map(move |y| (x, y)))
        .map(|(x, y)| pixels.rgb(x, y))
        .min_by_key(|[r, g, b]| u32::from(*r) + u32::from(*g) + u32::from(*b))
        .expect("pixels");
    assert!(
        close(darkest, BLACK, 12),
        "text without a color is black, but the darkest pixel is {darkest:?}"
    );

    let red = scene(
        300,
        100,
        vec![text_at(
            "txt00001",
            20.0,
            20.0,
            "HHHH",
            32.0,
            1.5,
            json!({"color": "#ff0000"}),
        )],
    );
    let pixels = render(&red, 300, 100);
    let ink_pixels = (0..300)
        .flat_map(|x| (0..100).map(move |y| (x, y)))
        .filter(|&(x, y)| pixels.inked(x, y))
        .collect::<Vec<_>>();
    assert!(!ink_pixels.is_empty(), "the red text has ink");
    for (x, y) in ink_pixels {
        assert!(
            pixels.rgb(x, y)[0] >= 253,
            "red text over white keeps a full red channel at ({x}, {y})"
        );
    }
    let reddest = (0..300)
        .flat_map(|x| (0..100).map(move |y| (x, y)))
        .map(|(x, y)| pixels.rgb(x, y))
        .min_by_key(|[_, g, b]| u32::from(*g) + u32::from(*b))
        .expect("pixels");
    assert!(
        close(reddest, RED, 12),
        "the text color is #ff0000, but the strongest pixel is {reddest:?}"
    );
}

#[test]
fn the_first_baseline_sits_where_css_puts_it_inside_the_line() {
    // A line's content is centered in the line height: the half-leading above
    // the ascent is (line height - (ascent + descent)) / 2. Capital Hs sit
    // on the baseline.
    for (line_height, top) in [(2.0, 20.0), (1.0, 40.0)] {
        let size = 32.0;
        let design = scene(
            200,
            160,
            vec![text_at(
                "txt00001",
                20.0,
                top,
                "HHHH",
                size,
                line_height,
                json!({}),
            )],
        );
        let pixels = render(&design, 200, 160);
        let ink = pixels.ink_box([0, 0, 200, 160]).expect("the text has ink");
        let content = (INTER_ASCENT + INTER_DESCENT) * size as f32;
        let baseline = top as f32
            + (line_height as f32 * size as f32 - content) / 2.0
            + INTER_ASCENT * size as f32;
        assert!(
            (ink[3] as f32 - baseline).abs() <= 1.5,
            "with line height {line_height} the baseline is at {baseline}, but the Hs end at {}",
            ink[3]
        );
    }
}

#[test]
fn lines_are_a_line_height_apart() {
    let design = scene(
        200,
        200,
        vec![text_at(
            "txt00001",
            20.0,
            10.0,
            "H\nH\nH",
            32.0,
            1.5,
            json!({}),
        )],
    );
    let pixels = render_200(&design);
    let lines = pixels.ink_rows(0, 200);
    assert_eq!(
        lines.len(),
        3,
        "three lines of text have three runs of ink rows: {lines:?}"
    );
    for pair in lines.windows(2) {
        let step = pair[1][1] as f32 - pair[0][1] as f32;
        assert!(
            (step - 48.0).abs() <= 1.5,
            "lines are 48 px apart, not {step}"
        );
    }
}

/// How many pixels of the two drawings differ by more than a few levels.
fn differing(a: &Value, b: &Value, width: u32, height: u32) -> usize {
    render(a, width, height).differing(&render(b, width, height), 24)
}

fn text_of(content: &str, width: Value, props: Value) -> Value {
    scene(
        260,
        140,
        vec![styled(
            text_at(
                "txt00001",
                20.0,
                20.0,
                content,
                16.0,
                1.5,
                json!({"fontFamily": "Inter"}),
            ),
            with(&props, "width", width),
        )],
    )
}

#[test]
fn text_is_drawn_in_the_lines_layout_computed_not_in_skias_own_wrapping() {
    // A word wider than its box is not broken, as in Chrome: the line
    // overflows the box. Skia's own wrapping would cut the word.
    let long = text_of("Supercalifragilistic", fixed(40.0), json!({}));
    let hugging = text_of("Supercalifragilistic", json!("hug"), json!({}));
    assert_eq!(
        differing(&long, &hugging, 260, 140),
        0,
        "an overlong word stays on one line"
    );
    let ink = render(&long, 260, 140)
        .ink_box([0, 0, 260, 140])
        .expect("ink");
    assert!(
        ink[2] > 20 + 40 + 20,
        "the word runs past its 40 px box: ink {ink:?}"
    );

    // Chrome does not break after a slash between letters; Skia does.
    let slashes = text_of("aaaa/bbbb/cccc dddd", fixed(60.0), json!({}));
    let by_hand = text_of("aaaa/bbbb/cccc\ndddd", json!("hug"), json!({}));
    assert_eq!(
        differing(&slashes, &by_hand, 260, 140),
        0,
        "a path with slashes breaks only at the space"
    );

    // Where each word has to go on a line of its own.
    let wrapped = text_of("alpha beta gamma", fixed(60.0), json!({}));
    let by_hand = text_of("alpha\nbeta\ngamma", json!("hug"), json!({}));
    assert_eq!(
        differing(&wrapped, &by_hand, 260, 140),
        0,
        "lines break at the spaces the box is too narrow for"
    );
    let rows = render(&wrapped, 260, 140).ink_rows(0, 260);
    assert_eq!(rows.len(), 3, "three lines of ink: {rows:?}");
}

#[test]
fn text_align_places_each_line_in_the_box() {
    let ink_of = |align: &str| {
        let design = scene(
            300,
            100,
            vec![text_at(
                "txt00001",
                50.0,
                20.0,
                "HH",
                32.0,
                1.5,
                json!({"width": fixed(200.0), "textAlign": align}),
            )],
        );
        render(&design, 300, 100)
            .ink_box([0, 0, 300, 100])
            .expect("ink")
    };
    let left = ink_of("left");
    let center = ink_of("center");
    let right = ink_of("right");
    assert!(
        left[0] <= 50 + 4,
        "left-aligned text starts at the box's left edge: {left:?}"
    );
    let middle = (center[0] + center[2]) as f32 / 2.0;
    assert!(
        (middle - 150.0).abs() <= 3.0,
        "centered text is centered in 50..250, not at {middle}"
    );
    assert!(
        right[2] >= 250 - 4 && right[2] <= 251,
        "right-aligned text ends at the box's right edge: {right:?}"
    );
}

#[test]
fn justified_text_spreads_every_line_but_the_last_to_the_boxs_width() {
    let content = "HH HH HH HH HH HH HH";
    let ink_rows_of = |align: &str| {
        let design = scene(
            260,
            140,
            vec![text_at(
                "txt00001",
                20.0,
                20.0,
                content,
                16.0,
                1.5,
                json!({"width": fixed(150.0), "textAlign": align}),
            )],
        );
        let pixels = render(&design, 260, 140);
        let first = pixels
            .ink_box([0, 20, 260, 44])
            .expect("the first line has ink");
        let last = pixels
            .ink_box([0, 44, 260, 68])
            .expect("the second line has ink");
        (first, last)
    };
    let (left_first, _) = ink_rows_of("left");
    let (justified_first, justified_last) = ink_rows_of("justify");
    assert!(
        left_first[2] <= 20 + 146,
        "left-aligned, the first line stops short of the box edge: {left_first:?}"
    );
    assert!(
        justified_first[2] >= 20 + 150 - 3,
        "justified, the first line reaches the box edge: {justified_first:?}"
    );
    assert!(
        justified_last[2] <= 20 + 90,
        "the last line is not spread: {justified_last:?}"
    );
}

#[test]
fn letter_spacing_spreads_the_glyphs() {
    let width_of = |spacing: f64| {
        let design = scene(
            200,
            100,
            vec![text_at(
                "txt00001",
                20.0,
                20.0,
                "HHHH",
                20.0,
                1.5,
                json!({"letterSpacing": spacing}),
            )],
        );
        let ink = render(&design, 200, 100)
            .ink_box([0, 0, 200, 100])
            .expect("ink");
        (ink[2] - ink[0]) as f32
    };
    let extra = width_of(8.0) - width_of(0.0);
    assert!(
        (extra - 24.0).abs() <= 2.5,
        "four glyphs 8 px apart are 3 x 8 = 24 px wider, but are {extra}"
    );
}

#[test]
fn underline_and_line_through_are_drawn_across_the_text() {
    let draw = |decoration: &str| {
        let design = scene(
            200,
            100,
            vec![text_at(
                "txt00001",
                20.0,
                20.0,
                "IIII",
                32.0,
                1.5,
                json!({"textDecoration": decoration}),
            )],
        );
        let boxes = lay_out(&design);
        (render(&design, 200, 100), box_of(&boxes, "txt00001"))
    };
    let (plain, text_box) = draw("none");
    let ink = plain.ink_box([0, 0, 200, 100]).expect("ink");
    let baseline = ink[3];
    let (x0, x1) = (text_box.x as u32, (text_box.x + text_box.width) as u32);
    let widest_row = |pixels: &Pixels, rows: std::ops::Range<u32>| {
        rows.map(|y| pixels.inked_in_row(y, x0, x1))
            .max()
            .unwrap_or(0)
    };
    let below = baseline + 1..baseline + 11;
    let through = baseline - 15..baseline - 5;

    assert_eq!(
        widest_row(&plain, below.clone()),
        0,
        "no decoration, nothing below the baseline"
    );
    assert!(
        widest_row(&plain, through.clone()) as f32 <= text_box.width * 0.6,
        "the stems of Is do not span the text"
    );

    let (underlined, _) = draw("underline");
    assert!(
        widest_row(&underlined, below) as f32 >= text_box.width * 0.9,
        "an underline spans the text below the baseline"
    );
    let (struck, _) = draw("line-through");
    assert!(
        widest_row(&struck, through) as f32 >= text_box.width * 0.9,
        "a line-through spans the text above the baseline"
    );
    assert_eq!(
        widest_row(&struck, baseline + 1..baseline + 11),
        0,
        "a line-through is not an underline"
    );
}

#[test]
fn text_fades_with_its_opacity() {
    let darkest = |opacity: f64| {
        let design = scene(
            200,
            100,
            vec![text_at(
                "txt00001",
                20.0,
                20.0,
                "HHHH",
                32.0,
                1.5,
                json!({"opacity": opacity}),
            )],
        );
        let pixels = render(&design, 200, 100);
        (0..200)
            .flat_map(|x| (0..100).map(move |y| (x, y)))
            .map(|(x, y)| pixels.ink(x, y))
            .max()
            .expect("pixels")
    };
    assert!(darkest(1.0) >= 245, "opaque text is fully dark");
    let half = darkest(0.5);
    assert!(
        (118..=138).contains(&half),
        "half-transparent black text is half dark, not {half}"
    );
}

#[test]
fn a_text_in_a_font_the_host_did_not_supply_is_set_in_the_bundled_inter() {
    let in_family = |family: &str| {
        scene(
            200,
            100,
            vec![text_at(
                "txt00001",
                20.0,
                20.0,
                "Hamburgefonts",
                24.0,
                1.5,
                json!({"fontFamily": family}),
            )],
        )
    };
    let missing = in_family("No Such Sans");
    let inter = in_family("Inter");
    assert_eq!(
        differing(&missing, &inter, 200, 100),
        0,
        "a missing font is drawn exactly as Inter"
    );
    let (a, b) = (lay_out(&missing), lay_out(&inter));
    assert_eq!(
        box_of(&a, "txt00001"),
        box_of(&b, "txt00001"),
        "and measured exactly as Inter"
    );
    assert!(
        render(&missing, 200, 100)
            .ink_box([0, 0, 200, 100])
            .is_some(),
        "it has ink"
    );
}

#[test]
fn a_font_file_supplied_by_the_host_is_used_for_layout_and_for_drawing() {
    // Monaco is monospaced, so "iiiiiiii" is as wide as "WWWWWWWW" in it, and
    // nowhere near that in Inter. Where the file is not installed (it ships
    // with macOS, the platform the renderer builds on), the test has nothing
    // to supply.
    let Ok(font) = std::fs::read("/System/Library/Fonts/Monaco.ttf") else {
        eprintln!("skipped: /System/Library/Fonts/Monaco.ttf is not installed");
        return;
    };
    let design_of = |content: &str, family: &str| {
        scene(
            300,
            100,
            vec![text_at(
                "txt00001",
                20.0,
                20.0,
                content,
                24.0,
                1.5,
                json!({"fontFamily": family}),
            )],
        )
    };
    let width_in = |assets: &Assets, content: &str, family: &str| {
        let design = design_of(content, family);
        let boxes = layout::compute(&node(&design), assets.fonts());
        box_of(&boxes, "txt00001").width
    };

    let mut assets = Assets::new();
    assets
        .add_font("Host Mono", &font)
        .expect("the host can supply a font file");
    let narrow = width_in(&assets, "iiiiiiii", "Host Mono");
    let wide = width_in(&assets, "WWWWWWWW", "Host Mono");
    assert!(
        (narrow - wide).abs() <= 0.05,
        "a monospaced font: {narrow} px and {wide} px"
    );
    let in_inter = width_in(&assets, "iiiiiiii", "Inter");
    assert!(
        (narrow - in_inter).abs() > 20.0,
        "not Inter's {in_inter} px, so layout measures with the supplied font"
    );

    let supplied = render_with(&design_of("iiiiiiii", "Host Mono"), &assets, 300, 100);
    let fallback = render_with(
        &design_of("iiiiiiii", "Host Mono"),
        &Assets::new(),
        300,
        100,
    );
    assert!(
        supplied.differing(&fallback, 24) > 100,
        "the supplied font is drawn, not Inter"
    );
    let ink = supplied
        .ink_box([0, 0, 300, 100])
        .expect("the text has ink");
    assert!(
        (ink[2] - ink[0]) as f32 >= narrow * 0.8,
        "the glyphs fill the measured width: {ink:?} for {narrow} px"
    );
}

#[test]
fn the_host_cannot_supply_a_font_or_an_image_that_is_not_one() {
    let mut assets = Assets::new();
    assert!(assets.add_font("Broken", b"this is not a font").is_err());
    assert!(
        assets
            .add_image(&attachment(PHOTO), b"this is not an image")
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

#[test]
fn an_image_node_covers_its_box_by_cropping_or_fits_inside_it() {
    let assets = stripes();
    // The picture is 2:1 with four columns. In a square box, `cover` scales it
    // up until it fills the box and crops the sides, leaving the middle two
    // columns; `contain` scales it to the box's width and centers it.
    let design = scene(
        200,
        200,
        vec![
            image_node("pnt00001", 10.0, 10.0, 40.0, 40.0, "cover", json!({})),
            image_node("pnt00002", 100.0, 10.0, 40.0, 40.0, "contain", json!({})),
        ],
    );
    let pixels = render_with(&design, &assets, 200, 200);
    assert_rgb(
        &pixels,
        15,
        30,
        GREEN,
        6,
        "cover: the left half is the second column",
    );
    assert_rgb(
        &pixels,
        45,
        30,
        BLUE,
        6,
        "cover: the right half is the third column",
    );
    assert_rgb(
        &pixels,
        15,
        13,
        GREEN,
        6,
        "cover: it fills the box to its top",
    );
    assert_rgb(
        &pixels,
        45,
        47,
        BLUE,
        6,
        "cover: it fills the box to its bottom",
    );
    assert_rgb(&pixels, 55, 30, WHITE, 3, "cover: nothing beyond the box");

    assert_rgb(&pixels, 105, 30, RED, 6, "contain: the first column");
    assert_rgb(&pixels, 115, 30, GREEN, 6, "contain: the second column");
    assert_rgb(&pixels, 125, 30, BLUE, 6, "contain: the third column");
    assert_rgb(&pixels, 135, 30, YELLOW, 6, "contain: the fourth column");
    assert_rgb(
        &pixels,
        105,
        14,
        WHITE,
        3,
        "contain: empty above the centered picture",
    );
    assert_rgb(
        &pixels,
        135,
        46,
        WHITE,
        3,
        "contain: empty below the centered picture",
    );
}

#[test]
fn an_image_fill_fits_a_frame_like_an_image_node() {
    let assets = stripes();
    let fill = |fit: &str| json!({"fills": [{"type": "image", "image": PHOTO, "fit": fit}]});
    let design = scene(
        200,
        200,
        vec![
            box_at("frm00001", 10.0, 10.0, 40.0, 40.0, fill("cover")),
            box_at("frm00002", 100.0, 10.0, 40.0, 40.0, fill("contain")),
        ],
    );
    let pixels = render_with(&design, &assets, 200, 200);
    assert_rgb(&pixels, 15, 30, GREEN, 6, "cover");
    assert_rgb(&pixels, 45, 30, BLUE, 6, "cover");
    assert_rgb(&pixels, 105, 30, RED, 6, "contain");
    assert_rgb(&pixels, 135, 30, YELLOW, 6, "contain");
    assert_rgb(
        &pixels,
        105,
        14,
        WHITE,
        3,
        "contain leaves the frame empty above the picture",
    );
}

#[test]
fn an_image_is_clipped_to_its_radii_and_takes_a_stroke_a_shadow_and_opacity() {
    let assets = stripes();
    let design = scene(
        200,
        200,
        vec![
            image_node(
                "pnt00001",
                20.0,
                20.0,
                60.0,
                60.0,
                "cover",
                json!({
                    "radius": {"topLeft": 30, "topRight": 30, "bottomRight": 30, "bottomLeft": 30},
                }),
            ),
            image_node(
                "pnt00002",
                110.0,
                20.0,
                60.0,
                60.0,
                "cover",
                json!({
                    "strokes": [{"color": "#ff0000", "width": 4}],
                    "shadows": [shadow("drop", "#000000", 10.0, 0.0, 0.0, 0.0)]
                }),
            ),
            image_node(
                "pnt00003",
                20.0,
                110.0,
                60.0,
                60.0,
                "cover",
                json!({"opacity": 0.5}),
            ),
        ],
    );
    let pixels = render_with(&design, &assets, 200, 200);
    assert_rgb(
        &pixels,
        21,
        21,
        WHITE,
        3,
        "the image is cut at its rounded corner",
    );
    assert_rgb(&pixels, 40, 50, GREEN, 6, "and shows inside it");
    assert_rgb(
        &pixels,
        110,
        50,
        RED,
        3,
        "the stroke is inside the image's edge",
    );
    assert_rgb(&pixels, 120, 50, GREEN, 6, "the image inside the stroke");
    assert_rgb(
        &pixels,
        175,
        50,
        BLACK,
        3,
        "the drop shadow of an image node",
    );
    assert_rgb(
        &pixels,
        30,
        140,
        [127, 207, 127],
        8,
        "half-transparent green over white",
    );
}

#[test]
fn an_image_the_host_did_not_supply_draws_nothing() {
    let design = scene(
        200,
        200,
        vec![
            image_node("pnt00001", 20.0, 20.0, 40.0, 40.0, "cover", json!({})),
            box_at(
                "frm00001",
                100.0,
                20.0,
                40.0,
                40.0,
                json!({"fills": [{"type": "image", "image": PHOTO, "fit": "cover"}]}),
            ),
        ],
    );
    let pixels = render(&design, 200, 200);
    assert!(
        pixels.ink_box([0, 0, 200, 200]).is_none(),
        "the whole canvas stays white"
    );
}

// ---------------------------------------------------------------------------
// Vectors
// ---------------------------------------------------------------------------

#[test]
fn a_vector_fills_its_path_in_pixels_from_the_corner_of_its_box() {
    let triangle = vector_node(
        "vec00001",
        80.0,
        80.0,
        "M0 40 L20 0 L40 40 Z",
        json!({"fills": [solid("#ff0000")]}),
    );
    let pixels = render_200(&scene(200, 200, vec![triangle]));
    assert_all_rgb(
        &pixels,
        &[(100, 110), (100, 85), (90, 115), (110, 115)],
        RED,
        "inside the triangle",
    );
    assert_all_rgb(
        &pixels,
        &[(83, 85), (110, 90), (90, 85), (79, 100), (125, 100)],
        WHITE,
        "outside it",
    );
}

#[test]
fn a_vector_without_a_fill_or_a_stroke_is_not_drawn() {
    let triangle = vector_node("vec00001", 80.0, 80.0, "M0 40 L20 0 L40 40 Z", json!({}));
    let pixels = render_200(&scene(200, 200, vec![triangle]));
    assert!(
        pixels.ink_box([0, 0, 200, 200]).is_none(),
        "the canvas stays white"
    );
}

#[test]
fn a_vector_stroke_is_centered_on_its_path_and_over_the_fill() {
    let line = vector_node(
        "vec00001",
        80.0,
        80.0,
        "M0 20 L40 20",
        json!({"strokes": [{"color": "#0000ff", "width": 6}]}),
    );
    let pixels = render_200(&scene(200, 200, vec![line]));
    assert_all_rgb(
        &pixels,
        &[
            (100, 97),
            (100, 99),
            (100, 100),
            (100, 102),
            (81, 100),
            (118, 100),
        ],
        BLUE,
        "the 6 px line",
    );
    assert_all_rgb(
        &pixels,
        &[(100, 96), (100, 103), (78, 100), (121, 100)],
        WHITE,
        "beyond the line and its square ends",
    );

    let square = vector_node(
        "vec00001",
        80.0,
        80.0,
        "M0 0 L40 0 L40 40 L0 40 Z",
        json!({"fills": [solid("#ff0000")], "strokes": [{"color": "#0000ff", "width": 4}]}),
    );
    let pixels = render_200(&scene(200, 200, vec![square]));
    assert_rgb(
        &pixels,
        81,
        100,
        BLUE,
        3,
        "the stroke over the fill, inside the path",
    );
    assert_rgb(
        &pixels,
        78,
        100,
        BLUE,
        3,
        "and outside it, centered on the path",
    );
    assert_rgb(&pixels, 83, 100, RED, 3, "the fill beyond the stroke");
}

#[test]
fn a_vector_has_a_shadow_in_the_shape_of_its_path_and_an_opacity() {
    let shadowed = vector_node(
        "vec00001",
        40.0,
        80.0,
        "M0 40 L20 0 L40 40 Z",
        json!({
            "fills": [solid("#ff0000")],
            "shadows": [shadow("drop", "#000000", 40.0, 0.0, 0.0, 0.0)]
        }),
    );
    let faded = vector_node(
        "vec00002",
        40.0,
        140.0,
        "M0 0 L40 0 L40 40 L0 40 Z",
        json!({"fills": [solid("#ff0000")], "opacity": 0.5}),
    );
    let pixels = render_200(&scene(200, 200, vec![shadowed, faded]));
    assert_rgb(
        &pixels,
        100,
        110,
        BLACK,
        3,
        "the shadow, 40 px to the right of the triangle",
    );
    assert_rgb(
        &pixels,
        80,
        85,
        WHITE,
        3,
        "where the shadowed triangle has no apex",
    );
    assert_rgb(
        &pixels,
        60,
        160,
        [255, 127, 127],
        4,
        "half-transparent red over white",
    );
}
