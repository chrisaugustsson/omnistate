//! Drawing designs: the nodes of a laid-out design, painted with Skia from the
//! boxes that `layout::compute` gives them, with the images and fonts that the
//! host supplies.
//!
//! One canvas pixel is one CSS pixel, and the page origin is the canvas's
//! top-left corner. Where the design format leaves drawing open:
//!
//! - A frame paints its drop shadows, then its fills, its inner shadows, its
//!   strokes and its children. Fills, strokes and inner shadows go in list
//!   order, so the last of a kind is on top, but the first shadow is on top
//!   of the other shadows, as in CSS `box-shadow`.
//! - A stroke of a frame or an image lies inside the box edge, like a CSS
//!   `inset` border: it takes no space and follows the corner radii. A stroke
//!   of a vector is centered on its path.
//! - A shadow is a CSS `box-shadow`: a blur of `b` is a Gaussian of standard
//!   deviation `b / 2`, and a drop shadow is not drawn beneath its own shape.
//! - `opacity` fades a node and everything below it as one group.
//! - Images fit like CSS `object-fit`, centered. An image that is magnified
//!   keeps its pixels square, as CSS `image-rendering: pixelated` does, so a
//!   small picture stays crisp; one that is reduced is filtered.
//! - Values that are token references are not resolved here (US-032): a
//!   fill, stroke or shadow whose color is a token is not drawn, and a token
//!   for a length or an opacity is as if it were left out.

use crate::layout::{self, LayoutBox};
use crate::text::{Fonts, ShapedText, literal};
use omni_core::design::{
    self, AttachmentRef, Fill, GradientStop, ImageFit, Node, NodeData, NodeId, Radius, Shadow,
    ShadowKind, Stroke, TokenOr,
};
use skia_safe::gradient::{Colors, Gradient, Interpolation, interpolation::InPremul, shaders};
use skia_safe::{
    AlphaType, BlurStyle, Canvas, ClipOp, Color, Color4f, ColorType, Data, FilterMode, Image,
    ImageInfo, MaskFilter, MipmapMode, Paint, Path, PathBuilder, PathOp, RRect, Rect,
    SamplingOptions, Shader, TileMode, Vector, paint, path_utils, surfaces,
};
use std::cell::Cell;
use std::collections::HashMap;

/// What the host supplies for drawing: the images that `attachment:<id>`
/// references name, and font files. The bundled Inter is always there, and
/// stands in for every font family that the host did not supply.
pub struct Assets {
    fonts: Fonts,
    images: HashMap<AttachmentRef, Image>,
}

impl Assets {
    /// Assets with the bundled Inter only, and no images.
    pub fn new() -> Self {
        Self {
            fonts: Fonts::bundled(),
            images: HashMap::new(),
        }
    }

    /// Supplies the image that `attachment` names, from encoded PNG or JPEG
    /// bytes. Fails if the bytes are not an image.
    pub fn add_image(&mut self, attachment: &AttachmentRef, data: &[u8]) -> Result<(), String> {
        let image = Image::from_encoded(Data::new_copy(data))
            // Decode now, so bytes that are cut short fail here and drawing
            // never has to decode.
            .and_then(|image| image.make_raster_image(None, None))
            .ok_or_else(|| format!("the bytes for {} are not an image", attachment.as_str()))?;
        self.images.insert(attachment.clone(), image);
        Ok(())
    }

    /// Supplies a font file (TrueType or OpenType) for text in `family`. Fails
    /// if the bytes are not a font.
    pub fn add_font(&mut self, family: &str, data: &[u8]) -> Result<(), String> {
        self.fonts.add_font(family, data)
    }

    /// The fonts that layout measures text with and drawing draws it with.
    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}

/// Draws `root` and every node below it on `canvas`, in page coordinates (one
/// canvas pixel to a CSS pixel), each node in the box that `boxes` gives it.
pub fn draw(canvas: &Canvas, root: &Node, boxes: &HashMap<NodeId, LayoutBox>, assets: &Assets) {
    draw_counted(canvas, root, boxes, assets);
}

/// As [`draw`], and returns how many nodes it painted: the visible ones that
/// layout gave a box, below `root` and including it.
pub(crate) fn draw_counted(
    canvas: &Canvas,
    root: &Node,
    boxes: &HashMap<NodeId, LayoutBox>,
    assets: &Assets,
) -> usize {
    let painter = Painter {
        canvas,
        boxes,
        assets,
        painted: Cell::new(0),
    };
    painter.node(root);
    painter.painted.get()
}

/// The area that painting `node` and everything below it can touch: its box,
/// grown by the drop shadows that reach beyond it, and by whatever of its
/// children sticks out of it unless it clips them. `None` if nothing of it is
/// painted. Culling by this area never changes the picture.
pub(crate) fn reach(node: &Node, boxes: &HashMap<NodeId, LayoutBox>) -> Option<Rect> {
    let layout_box = boxes.get(node.id())?;
    if !node.visible() {
        return None;
    }
    let rect = Rect::from_xywh(
        layout_box.x,
        layout_box.y,
        layout_box.width,
        layout_box.height,
    );
    let below = |own: Rect, children: &[Node]| {
        children
            .iter()
            .filter_map(|child| reach(child, boxes))
            .fold(own, Rect::join2)
    };
    match node.data() {
        NodeData::Page(page) => Some(below(Rect::new_empty(), &page.children)),
        NodeData::Board(frame) | NodeData::Frame(frame) => {
            let own = with_shadows(rect, frame.shadows.as_deref());
            Some(if frame.clip == Some(true) {
                own
            } else {
                below(own, &frame.children)
            })
        }
        NodeData::Text(_) => Some(rect),
        NodeData::Image(image) => Some(with_shadows(rect, image.shadows.as_deref())),
        NodeData::Vector(vector) => {
            let widest = vector
                .strokes
                .iter()
                .flatten()
                .filter_map(|stroke| literal(&stroke.width))
                .fold(0.0, f32::max);
            // The path is not clipped to the box, and a miter join reaches up
            // to four times half the stroke width from the path.
            let drawn = Path::from_svg(vector.path.as_str()).map_or(rect, |path| {
                let path = path.with_offset((rect.left, rect.top));
                let bounds = path
                    .compute_tight_bounds()
                    .with_outset((widest * 2.0, widest * 2.0));
                Rect::join2(rect, bounds)
            });
            Some(with_shadows(drawn, vector.shadows.as_deref()))
        }
        NodeData::Instance(_) => None,
    }
}

/// `rect` and the area of the drop shadows it casts: shifted by their offsets
/// and grown by their spread and by three standard deviations of their blur.
fn with_shadows(rect: Rect, casting: Option<&[TokenOr<Shadow>]>) -> Rect {
    shadows(casting)
        .iter()
        .filter(|shadow| shadow.kind == ShadowKind::Drop)
        .fold(rect, |reach, shadow| {
            let grow = shadow.spread.max(0.0) + shadow.blur * 1.5;
            let cast = rect
                .with_offset((shadow.x, shadow.y))
                .with_outset((grow, grow));
            Rect::join2(reach, cast)
        })
}

/// Lays `root` out with the fonts of `assets`, draws it with Skia on a
/// transparent CPU raster surface of `width` x `height` pixels, and returns
/// the pixels as unpremultiplied RGBA, row by row from the top-left corner.
pub fn render_rgba(root: &Node, assets: &Assets, width: u32, height: u32) -> Vec<u8> {
    let size = (pixels(width), pixels(height));
    let boxes = layout::compute(root, assets.fonts());
    let mut surface =
        surfaces::raster_n32_premul(size).expect("Skia creates a raster surface of the size");
    draw(surface.canvas(), root, &boxes, assets);

    let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Unpremul, None);
    let mut rgba = vec![0; info.compute_min_byte_size()];
    let read = surface.read_pixels(&info, &mut rgba, info.min_row_bytes(), (0, 0));
    assert!(read, "Skia reads the drawn pixels back as RGBA");
    rgba
}

fn pixels(length: u32) -> i32 {
    i32::try_from(length).expect("canvas sizes fit in an i32")
}

struct Painter<'a> {
    canvas: &'a Canvas,
    boxes: &'a HashMap<NodeId, LayoutBox>,
    assets: &'a Assets,
    /// The nodes painted so far.
    painted: Cell<usize>,
}

impl Painter<'_> {
    /// Draws `node` and what is below it, unless it is hidden or layout gave
    /// it no box.
    fn node(&self, node: &Node) {
        let Some(&layout_box) = self.boxes.get(node.id()) else {
            return;
        };
        if !node.visible() {
            return;
        }
        if !matches!(node.data(), NodeData::Page(_) | NodeData::Instance(_)) {
            self.painted.set(self.painted.get() + 1);
        }
        let rect = Rect::from_xywh(
            layout_box.x,
            layout_box.y,
            layout_box.width,
            layout_box.height,
        );
        match node.data() {
            NodeData::Page(page) => self.children(&page.children),
            NodeData::Board(frame) | NodeData::Frame(frame) => {
                self.group(frame.opacity.as_ref(), || self.frame(frame, rect));
            }
            NodeData::Text(text) => self.group(text.opacity.as_ref(), || {
                let color = text.color.as_ref().and_then(color).unwrap_or(Color::BLACK);
                ShapedText::new(text, &self.assets.fonts).draw(
                    self.canvas,
                    &self.assets.fonts,
                    color,
                    (rect.left, rect.top),
                    rect.width(),
                );
            }),
            NodeData::Image(image) => self.group(image.opacity.as_ref(), || {
                self.image(image, rect);
            }),
            NodeData::Vector(vector) => self.group(vector.opacity.as_ref(), || {
                self.vector(vector, rect);
            }),
            // Instances wait for components and tokens (US-032).
            NodeData::Instance(_) => {}
        }
    }

    fn children(&self, children: &[Node]) {
        for child in children {
            self.node(child);
        }
    }

    /// Runs `draw` with the node's `opacity` applied to everything it draws
    /// together, or not at all when the node is transparent.
    fn group(&self, opacity: Option<&TokenOr<f64>>, draw: impl FnOnce()) {
        let alpha = opacity.and_then(literal).map_or(1.0, |o| o.clamp(0.0, 1.0));
        if alpha <= 0.0 {
            return;
        }
        if alpha < 1.0 {
            self.canvas.save_layer_alpha_f(None, alpha);
        } else {
            self.canvas.save();
        }
        draw();
        self.canvas.restore();
    }

    fn frame(&self, frame: &design::Frame, rect: Rect) {
        let rounded = Rounded::new(rect, frame.radius.as_ref());
        let shape = Shape::Rounded(rounded);
        let shadows = shadows(frame.shadows.as_deref());
        self.drop_shadows(&shape, &shadows);
        for fill in frame.fills.iter().flatten() {
            self.canvas.save();
            self.canvas.clip_rrect(rounded.rrect(), None, true);
            self.fill(fill, rect, &shape);
            self.canvas.restore();
        }
        self.inner_shadows(&shape, &shadows);
        self.inside_strokes(rounded, frame.strokes.as_deref());
        if frame.clip == Some(true) {
            self.canvas.save();
            self.canvas.clip_rrect(rounded.rrect(), None, true);
            self.children(&frame.children);
            self.canvas.restore();
        } else {
            self.children(&frame.children);
        }
    }

    fn image(&self, image: &design::Image, rect: Rect) {
        let rounded = Rounded::new(rect, image.radius.as_ref());
        let shape = Shape::Rounded(rounded);
        let shadows = shadows(image.shadows.as_deref());
        // An image that the host did not supply draws nothing, and its
        // shadow is not a shadow of anything.
        let Some(picture) = self.assets.images.get(&image.image) else {
            return;
        };
        self.drop_shadows(&shape, &shadows);
        self.canvas.save();
        self.canvas.clip_rrect(rounded.rrect(), None, true);
        self.draw_image(picture, image.fit, rect);
        self.canvas.restore();
        self.inner_shadows(&shape, &shadows);
        self.inside_strokes(rounded, image.strokes.as_deref());
    }

    fn vector(&self, vector: &design::Vector, rect: Rect) {
        let fills = vector.fills.as_deref().unwrap_or_default();
        let strokes: Vec<(Color, f32)> = vector
            .strokes
            .iter()
            .flatten()
            .filter_map(|stroke| Some((color(&stroke.color)?, literal(&stroke.width)?)))
            .filter(|&(_, width)| width > 0.0)
            .collect();
        // Path data is in pixels from the corner of the box.
        let Some(path) = Path::from_svg(vector.path.as_str()) else {
            return;
        };
        let path = path.with_offset((rect.left, rect.top));
        let widest = strokes.iter().map(|&(_, width)| width).fold(0.0, f32::max);
        // The shape of a vector's shadow is what the vector paints.
        let shape = match (fills.is_empty(), strokes.is_empty()) {
            (true, true) => return,
            (false, true) => path.clone(),
            (true, false) => stroke_outline(&path, widest),
            (false, false) => path
                .op(&stroke_outline(&path, widest), PathOp::Union)
                .unwrap_or_else(|| path.clone()),
        };
        let shape = Shape::Path(shape);
        let shadows = shadows(vector.shadows.as_deref());
        self.drop_shadows(&shape, &shadows);
        for fill in fills {
            self.canvas.save();
            self.canvas.clip_path(&path, None, true);
            self.fill(fill, rect, &Shape::Path(path.clone()));
            self.canvas.restore();
        }
        for (stroke_color, width) in strokes {
            let mut paint = Paint::default();
            paint
                .set_anti_alias(true)
                .set_style(paint::Style::Stroke)
                .set_stroke_width(width)
                .set_color(stroke_color);
            self.canvas.draw_path(&path, &paint);
        }
        self.inner_shadows(&shape, &shadows);
    }

    /// Paints `fill` over `shape`, whose box is `rect`.
    fn fill(&self, fill: &Fill, rect: Rect, shape: &Shape) {
        match fill {
            Fill::Solid { color: fill_color } => {
                if let Some(fill_color) = color(fill_color) {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true).set_color(fill_color);
                    shape.draw(self.canvas, &paint);
                }
            }
            Fill::LinearGradient { angle, stops } => {
                if let Some(shader) = gradient(*angle, stops, rect) {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true).set_shader(shader);
                    shape.draw(self.canvas, &paint);
                }
            }
            Fill::Image { image, fit } => {
                if let Some(picture) = self.assets.images.get(image) {
                    self.draw_image(picture, *fit, rect);
                }
            }
        }
    }

    /// Draws `picture` in `rect` as CSS `object-fit` does, centered. The caller
    /// clips what overflows.
    fn draw_image(&self, picture: &Image, fit: ImageFit, rect: Rect) {
        let (width, height) = (picture.width() as f32, picture.height() as f32);
        if width <= 0.0 || height <= 0.0 || rect.is_empty() {
            return;
        }
        let across = rect.width() / width;
        let down = rect.height() / height;
        let scale = match fit {
            ImageFit::Cover => across.max(down),
            ImageFit::Contain => across.min(down),
        };
        let size = (width * scale, height * scale);
        let target = Rect::from_xywh(
            rect.left + (rect.width() - size.0) / 2.0,
            rect.top + (rect.height() - size.1) / 2.0,
            size.0,
            size.1,
        );
        let sampling = if scale > 1.0 {
            SamplingOptions::default()
        } else {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)
        };
        self.canvas.draw_image_rect_with_sampling_options(
            picture,
            None,
            target,
            sampling,
            &Paint::default(),
        );
    }

    /// Draws the strokes of a box inside its edge, in list order.
    fn inside_strokes(&self, outer: Rounded, strokes: Option<&[Stroke]>) {
        for stroke in strokes.into_iter().flatten() {
            let (Some(stroke_color), Some(width)) = (color(&stroke.color), literal(&stroke.width))
            else {
                continue;
            };
            if width <= 0.0 {
                continue;
            }
            let mut paint = Paint::default();
            paint.set_anti_alias(true).set_color(stroke_color);
            match outer.grown(-width) {
                Some(inner) => {
                    self.canvas
                        .draw_drrect(outer.rrect(), inner.rrect(), &paint);
                }
                // A stroke wider than half the box fills it.
                None => {
                    self.canvas.draw_rrect(outer.rrect(), &paint);
                }
            }
        }
    }

    /// Draws the drop shadows of `shape`, the first on top, never beneath the
    /// shape itself.
    fn drop_shadows(&self, shape: &Shape, shadows: &[ResolvedShadow]) {
        for shadow in shadows
            .iter()
            .rev()
            .filter(|shadow| shadow.kind == ShadowKind::Drop)
        {
            self.canvas.save();
            shape.clip(self.canvas, ClipOp::Difference);
            let casting = shape
                .grown(shadow.spread)
                .map(|shape| shape.offset(shadow.x, shadow.y));
            if let Some(casting) = casting {
                casting.draw(self.canvas, &shadow.paint());
            }
            self.canvas.restore();
        }
    }

    /// Draws the inner shadows of `shape`, the first on top: the shape
    /// shrunk by the spread and moved by the offsets is a hole in the shadow
    /// that fills the rest of the shape.
    fn inner_shadows(&self, shape: &Shape, shadows: &[ResolvedShadow]) {
        for shadow in shadows
            .iter()
            .rev()
            .filter(|shadow| shadow.kind == ShadowKind::Inner)
        {
            self.canvas.save();
            shape.clip(self.canvas, ClipOp::Intersect);
            // The blur reaches this far past the edge of the hole.
            let reach = shadow.x.abs() + shadow.y.abs() + shadow.blur * 1.5 + shadow.spread.abs();
            let outer = Shape::Rounded(Rounded {
                rect: shape.bounds().with_outset((reach + 2.0, reach + 2.0)),
                radii: [0.0; 4],
            });
            let around = match shape.grown(-shadow.spread) {
                Some(hole) => outer
                    .path()
                    .op(&hole.offset(shadow.x, shadow.y).path(), PathOp::Difference),
                None => Some(outer.path()),
            };
            if let Some(around) = around {
                self.canvas.draw_path(&around, &shadow.paint());
            }
            self.canvas.restore();
        }
    }
}

/// The box of a node with rounded corners: the radius of each corner, top
/// left, top right, bottom right, bottom left, after CSS has scaled them down
/// to fit.
#[derive(Clone, Copy)]
struct Rounded {
    rect: Rect,
    radii: [f32; 4],
}

impl Rounded {
    fn new(rect: Rect, radius: Option<&Radius>) -> Self {
        let corner = |corner: &Option<design::Length>| {
            corner.as_ref().and_then(literal).unwrap_or(0.0).max(0.0)
        };
        let mut radii = radius.map_or([0.0; 4], |radius| {
            [
                corner(&radius.top_left),
                corner(&radius.top_right),
                corner(&radius.bottom_right),
                corner(&radius.bottom_left),
            ]
        });
        // As in CSS, radii that add up to more than a side of the box all
        // shrink by the same factor, until the most crowded side fits.
        let [tl, tr, br, bl] = radii;
        let factor = [
            (rect.width(), tl + tr),
            (rect.height(), tr + br),
            (rect.width(), br + bl),
            (rect.height(), bl + tl),
        ]
        .into_iter()
        .filter(|&(_, sum)| sum > 0.0)
        .map(|(side, sum)| side.max(0.0) / sum)
        .fold(1.0, f32::min);
        if factor < 1.0 {
            radii = radii.map(|radius| radius * factor);
        }
        Rounded { rect, radii }
    }

    fn rrect(&self) -> RRect {
        RRect::new_rect_radii(
            self.rect,
            &self.radii.map(|radius| Vector::new(radius, radius)),
        )
    }

    /// The box `by` px further out on every side, or in, for a negative `by`,
    /// or `None` once nothing is left. Round corners grow with it, and square
    /// corners stay square, as in CSS shadows and borders.
    fn grown(&self, by: f32) -> Option<Rounded> {
        let rect = self.rect.with_outset((by, by));
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        Some(Rounded {
            rect,
            radii: self.radii.map(|radius| {
                if radius > 0.0 {
                    (radius + by).max(0.0)
                } else {
                    0.0
                }
            }),
        })
    }
}

/// What casts a shadow and holds it: a box with rounded corners, or the
/// outline of a vector.
enum Shape {
    Rounded(Rounded),
    Path(Path),
}

impl Shape {
    fn path(&self) -> Path {
        match self {
            Shape::Rounded(rounded) => PathBuilder::new()
                .add_rrect(rounded.rrect(), None, None)
                .detach(),
            Shape::Path(path) => path.clone(),
        }
    }

    fn bounds(&self) -> Rect {
        match self {
            Shape::Rounded(rounded) => rounded.rect,
            Shape::Path(path) => path.compute_tight_bounds(),
        }
    }

    fn draw(&self, canvas: &Canvas, paint: &Paint) {
        match self {
            Shape::Rounded(rounded) => canvas.draw_rrect(rounded.rrect(), paint),
            Shape::Path(path) => canvas.draw_path(path, paint),
        };
    }

    fn clip(&self, canvas: &Canvas, op: ClipOp) {
        match self {
            Shape::Rounded(rounded) => canvas.clip_rrect(rounded.rrect(), op, true),
            Shape::Path(path) => canvas.clip_path(path, op, true),
        };
    }

    /// The shape `by` px further out, or in for a negative `by`.
    fn grown(&self, by: f32) -> Option<Shape> {
        match self {
            Shape::Rounded(rounded) => rounded.grown(by).map(Shape::Rounded),
            Shape::Path(path) if by == 0.0 => Some(Shape::Path(path.clone())),
            Shape::Path(path) => {
                let outline = stroke_outline(path, by.abs() * 2.0);
                let op = if by > 0.0 {
                    PathOp::Union
                } else {
                    PathOp::Difference
                };
                path.op(&outline, op)
                    .filter(|grown| !grown.is_empty())
                    .map(Shape::Path)
            }
        }
    }

    fn offset(self, x: f32, y: f32) -> Shape {
        match self {
            Shape::Rounded(mut rounded) => {
                rounded.rect = rounded.rect.with_offset((x, y));
                Shape::Rounded(rounded)
            }
            Shape::Path(path) => Shape::Path(path.with_offset((x, y))),
        }
    }
}

/// The area that a stroke of `width` covers along `path`.
fn stroke_outline(path: &Path, width: f32) -> Path {
    let mut paint = Paint::default();
    paint
        .set_style(paint::Style::Stroke)
        .set_stroke_width(width);
    let mut outline = PathBuilder::new();
    path_utils::fill_path_with_paint(path, &paint, &mut outline, None, None);
    outline.detach()
}

/// A shadow whose values are all literals.
struct ResolvedShadow {
    kind: ShadowKind,
    color: Color,
    x: f32,
    y: f32,
    blur: f32,
    spread: f32,
}

impl ResolvedShadow {
    /// The paint that draws the shadow's shape: its color, blurred.
    fn paint(&self) -> Paint {
        let mut paint = Paint::default();
        paint.set_anti_alias(true).set_color(self.color);
        if self.blur > 0.0 {
            // CSS blurs a shadow by a Gaussian with a standard deviation of
            // half the blur radius.
            paint.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, self.blur / 2.0, true));
        }
        paint
    }
}

fn shadows(shadows: Option<&[TokenOr<Shadow>]>) -> Vec<ResolvedShadow> {
    shadows
        .into_iter()
        .flatten()
        .filter_map(|shadow| {
            let TokenOr::Literal(shadow) = shadow else {
                return None;
            };
            let length = |length: Option<&design::Length>| length.and_then(literal).unwrap_or(0.0);
            Some(ResolvedShadow {
                kind: shadow.kind,
                color: color(&shadow.color)?,
                x: literal(&shadow.offset_x).unwrap_or(0.0),
                y: literal(&shadow.offset_y).unwrap_or(0.0),
                blur: length(shadow.blur.as_ref()).max(0.0),
                spread: length(shadow.spread.as_ref()),
            })
        })
        .collect()
}

/// The shader of a CSS `linear-gradient(<angle>deg, ...)` over `rect`: the
/// gradient line runs through the center of the box at `angle`, clockwise from
/// pointing up, and is as long as it takes for the corners to lie at its ends.
/// Stops that are not at 0 and 1 stretch it, and the colors beyond the first
/// and last stop hold. `None` if a stop is a token reference.
fn gradient(angle: f64, stops: &[GradientStop], rect: Rect) -> Option<Shader> {
    let mut colors = Vec::with_capacity(stops.len());
    let mut positions = Vec::with_capacity(stops.len());
    let mut last = f32::NEG_INFINITY;
    for stop in stops {
        let position = (stop.position as f32).max(last);
        last = position;
        positions.push(position);
        colors.push(Color4f::from(color(&stop.color)?));
    }
    let (first, end) = (positions[0], positions[positions.len() - 1]);
    let span = (end - first).max(f32::EPSILON);
    let positions: Vec<f32> = positions.iter().map(|p| (p - first) / span).collect();

    let (sin, cos) = (angle as f32).to_radians().sin_cos();
    let length = (rect.width() * sin).abs() + (rect.height() * cos).abs();
    let center = rect.center();
    let along = |at: f32| {
        (
            center.x + sin * length * (at - 0.5),
            center.y - cos * length * (at - 0.5),
        )
    };
    let gradient = Gradient::new(
        Colors::new(&colors, Some(&positions), TileMode::Clamp, None),
        // CSS interpolates in premultiplied alpha.
        Interpolation {
            in_premul: InPremul::Yes,
            ..Interpolation::default()
        },
    );
    shaders::linear_gradient((along(first), along(end)), &gradient, None)
}

/// The color `#rrggbb` or `#rrggbbaa`, or `None` for a token reference.
fn color(value: &TokenOr<design::Color>) -> Option<Color> {
    let TokenOr::Literal(value) = value else {
        return None;
    };
    let hex = value.as_str().strip_prefix('#')?;
    let channel = |index: usize| u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok();
    let alpha = if hex.len() == 8 { channel(3)? } else { 255 };
    Some(Color::from_argb(
        alpha,
        channel(0)?,
        channel(1)?,
        channel(2)?,
    ))
}
