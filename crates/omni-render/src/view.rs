//! Looking at a design: zooming and panning across it, drawing only the
//! boards that are in view, and drawing them in less detail when they are
//! too small to see it.
//!
//! - A [`Viewport`] is the part of the page that the screen shows: a zoom and
//!   the page point at the screen's top-left corner. Screen points are CSS
//!   pixels from the canvas's top-left corner, and a page point `p` is at
//!   screen `(p - origin) * zoom`.
//! - [`Controls`] turns the wheel, pinch, pointer and space-key events of the
//!   page into changes of a viewport. It is plain Rust, so the tests drive it
//!   natively, and the WebAssembly page only forwards browser events.
//! - A [`Scene`] holds a page laid out once and draws it in any viewport. It
//!   draws the boards that reach into the view and no others. Each board is
//!   recorded once into a Skia picture, so that moving the view replays it
//!   without shaping its text again. Zoomed far out, a board is drawn from a
//!   small raster snapshot of it instead (see the README's "Level of detail").

use crate::layout::{self, LayoutBox};
use crate::paint::{self, Assets};
use omni_core::design::{Node, NodeData, NodeId};
use skia_safe::{
    AlphaType, Canvas, ColorType, FilterMode, Image, ImageInfo, MipmapMode, Paint, Picture,
    PictureRecorder, Rect, SamplingOptions, surfaces,
};
use std::collections::HashMap;

/// The smallest zoom, so that the whole stress file fits in view.
pub const MIN_ZOOM: f32 = 0.01;
/// The largest zoom.
pub const MAX_ZOOM: f32 = 64.0;

/// How fast the wheel zooms: a wheel event of `delta_y` multiplies the zoom by
/// `exp(-delta_y * WHEEL_ZOOM_RATE)`, so a notch of 100 zooms by 22%, and
/// events add up however they are split.
const WHEEL_ZOOM_RATE: f32 = 0.002;

/// Below this device scale (the zoom times the pixel ratio) a board is drawn
/// from its snapshot instead of node by node: 50%.
const REDUCED_BELOW: f32 = 0.5;
/// The scale of a snapshot. It is the device scale below which snapshots are
/// used, so a snapshot is only ever reduced, never magnified.
const SNAPSHOT_SCALE: f32 = REDUCED_BELOW;
/// The longest side of a snapshot in pixels; a board larger than this at
/// `SNAPSHOT_SCALE` is snapshotted at a smaller scale.
const SNAPSHOT_MAX_SIDE: f32 = 2048.0;

fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_nan() {
        1.0
    } else {
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    }
}

/// The part of the page that the screen shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    x: f32,
    y: f32,
    zoom: f32,
    width: f32,
    height: f32,
}

impl Viewport {
    /// A screen of `width` x `height` CSS pixels at 100%, showing the page
    /// from its origin.
    pub fn new(width: f32, height: f32) -> Self {
        Self::at(0.0, 0.0, 1.0, width, height)
    }

    /// A screen of `width` x `height` whose top-left corner shows the page
    /// point (`x`, `y`) at `zoom`.
    pub fn at(x: f32, y: f32, zoom: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            zoom: clamp_zoom(zoom),
            width,
            height,
        }
    }

    /// Screen pixels per page pixel.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// The page point at the top-left corner of the screen.
    pub fn origin(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    /// The size of the screen in CSS pixels.
    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    /// Changes the size of the screen, keeping the zoom and the page point at
    /// its top-left corner.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.width = width;
        self.height = height;
    }

    /// The page point under the screen point `screen`.
    pub fn to_design(&self, screen: (f32, f32)) -> (f32, f32) {
        (self.x + screen.0 / self.zoom, self.y + screen.1 / self.zoom)
    }

    /// The screen point that shows the page point `design`.
    pub fn to_screen(&self, design: (f32, f32)) -> (f32, f32) {
        (
            (design.0 - self.x) * self.zoom,
            (design.1 - self.y) * self.zoom,
        )
    }

    /// The rectangle of the page that the screen shows.
    pub fn visible_rect(&self) -> LayoutBox {
        LayoutBox {
            x: self.x,
            y: self.y,
            width: self.width / self.zoom,
            height: self.height / self.zoom,
        }
    }

    /// Multiplies the zoom by `factor` (within `MIN_ZOOM` and `MAX_ZOOM`),
    /// keeping the page point under the screen point `anchor` where it is.
    pub fn zoom_at(&mut self, factor: f32, anchor: (f32, f32)) {
        let kept = self.to_design(anchor);
        self.zoom = clamp_zoom(self.zoom * factor);
        self.x = kept.0 - anchor.0 / self.zoom;
        self.y = kept.1 - anchor.1 / self.zoom;
    }

    /// Scrolls the page by `dx`, `dy` screen pixels: a positive `dy` shows
    /// what is further down.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.x += dx / self.zoom;
        self.y += dy / self.zoom;
    }

    /// Zooms and pans so that `rect` is wholly in view, as large as it can be
    /// and centered.
    pub fn fit(&mut self, rect: LayoutBox) {
        let across = self.width / rect.width;
        let down = self.height / rect.height;
        let fitting = across.min(down);
        // A rectangle with no size or a screen with none has no zoom to fit.
        if fitting.is_finite() && fitting > 0.0 {
            self.zoom = clamp_zoom(fitting);
        }
        self.x = rect.x + rect.width / 2.0 - self.width / self.zoom / 2.0;
        self.y = rect.y + rect.height / 2.0 - self.height / self.zoom / 2.0;
    }
}

/// A mouse wheel or trackpad scroll event, in screen CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheel {
    pub delta_x: f32,
    pub delta_y: f32,
    /// The pointer position.
    pub x: f32,
    pub y: f32,
    pub ctrl: bool,
    pub meta: bool,
}

/// Turns the pointer, wheel and key events of the page into zooming and
/// panning of a viewport. Each handler returns whether it used the event, so
/// that the page knows to stop the browser from also acting on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Controls {
    viewport: Viewport,
    space: bool,
    /// The page point that a pan with the pointer holds under it.
    grabbed: Option<(f32, f32)>,
}

impl Controls {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            space: false,
            grabbed: None,
        }
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    /// Ctrl or Cmd with the wheel zooms around the pointer, which is also how
    /// browsers report a trackpad pinch; the wheel alone pans.
    pub fn wheel(&mut self, event: Wheel) -> bool {
        if event.ctrl || event.meta {
            let factor = (-event.delta_y * WHEEL_ZOOM_RATE).exp();
            if factor.is_finite() {
                self.viewport.zoom_at(factor, (event.x, event.y));
            }
        } else {
            self.viewport.pan_by(event.delta_x, event.delta_y);
        }
        true
    }

    /// A touch pinch: `scale` is the change in distance between the fingers
    /// since the last call, centered on the point `x`, `y` between them.
    pub fn pinch(&mut self, scale: f32, x: f32, y: f32) -> bool {
        if !(scale.is_finite() && scale > 0.0) {
            return false;
        }
        self.viewport.zoom_at(scale, (x, y));
        true
    }

    /// Whether the space key is held. Letting go of it ends a pan.
    pub fn set_space(&mut self, held: bool) {
        self.space = held;
        if !held {
            self.grabbed = None;
        }
    }

    /// With space held, grabs the page where the pointer is. Without it, the
    /// pointer is left to selection (US-007).
    pub fn pointer_down(&mut self, x: f32, y: f32) -> bool {
        self.grabbed = self.space.then(|| self.viewport.to_design((x, y)));
        self.grabbed.is_some()
    }

    /// Pans so that the grabbed page point is under the pointer.
    pub fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        let Some((grabbed_x, grabbed_y)) = self.grabbed else {
            return false;
        };
        self.viewport.x = grabbed_x - x / self.viewport.zoom;
        self.viewport.y = grabbed_y - y / self.viewport.zoom;
        true
    }

    pub fn pointer_up(&mut self) {
        self.grabbed = None;
    }
}

/// How much of each board a frame drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Every node.
    Full,
    /// The cheaper level of detail for views zoomed far out.
    Reduced,
}

/// What a frame did.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameStats {
    /// The boards that the frame drew or composited, in page order.
    pub boards_drawn: Vec<NodeId>,
    /// The design nodes the frame painted. Cached pictures that the frame
    /// only composited do not count.
    pub nodes_painted: usize,
    pub detail: Detail,
}

/// A board of the page with what the scene keeps of it.
struct Board {
    /// Where the board is among the page's children.
    index: usize,
    id: NodeId,
    bounds: LayoutBox,
    /// Everything that painting the board can touch (`paint::reach`).
    reach: Rect,
    /// The board's drawing at full detail, once it has been recorded.
    picture: Option<Picture>,
    /// The board's drawing at reduced detail, once it has been made.
    snapshot: Option<Snapshot>,
}

/// A board drawn small, with where it goes on the page.
struct Snapshot {
    image: Image,
    rect: Rect,
}

impl Board {
    /// Whether painting the board touches the `visible` part of the page.
    fn is_in(&self, visible: &LayoutBox) -> bool {
        !self.reach.is_empty()
            && self.reach.left < visible.x + visible.width
            && self.reach.right > visible.x
            && self.reach.top < visible.y + visible.height
            && self.reach.bottom > visible.y
    }

    /// Records the board's drawing unless that is done, and returns how many
    /// nodes recording painted.
    fn record(
        &mut self,
        node: &Node,
        boxes: &HashMap<NodeId, LayoutBox>,
        assets: &Assets,
    ) -> usize {
        if self.picture.is_some() {
            return 0;
        }
        let mut recorder = PictureRecorder::new();
        // The bounding box hierarchy lets a replay skip the parts of the
        // board that are out of view.
        let canvas = recorder.begin_recording(self.reach, true);
        let painted = paint::draw_counted(canvas, node, boxes, assets);
        self.picture = recorder.finish_recording_as_picture(None);
        painted
    }

    /// Draws the recorded board on `canvas`, in page coordinates.
    fn draw_full(&self, canvas: &Canvas) {
        if let Some(picture) = &self.picture {
            canvas.draw_picture(picture, None, None);
        }
    }

    /// Makes the snapshot from the recorded drawing unless that is done.
    fn snapshot(&mut self) {
        if self.snapshot.is_some() {
            return;
        }
        let Some(picture) = &self.picture else {
            return;
        };
        let reach = self.reach;
        let longest = reach.width().max(reach.height());
        let scale = SNAPSHOT_SCALE.min(SNAPSHOT_MAX_SIDE / longest);
        let size = (
            (reach.width() * scale).ceil().max(1.0) as i32,
            (reach.height() * scale).ceil().max(1.0) as i32,
        );
        let Some(mut surface) = surfaces::raster_n32_premul(size) else {
            return;
        };
        let canvas = surface.canvas();
        canvas.scale((scale, scale));
        canvas.translate((-reach.left, -reach.top));
        canvas.draw_picture(picture, None, None);
        self.snapshot = Some(Snapshot {
            image: surface.image_snapshot(),
            rect: Rect::from_xywh(
                reach.left,
                reach.top,
                size.0 as f32 / scale,
                size.1 as f32 / scale,
            ),
        });
    }

    /// Draws the snapshot on `canvas`, in page coordinates.
    fn draw_reduced(&self, canvas: &Canvas) {
        if let Some(snapshot) = &self.snapshot {
            // The snapshot is only reduced, so mipmaps keep it smooth.
            canvas.draw_image_rect_with_sampling_options(
                &snapshot.image,
                None,
                snapshot.rect,
                SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
                &Paint::default(),
            );
        }
    }
}

/// A page laid out once, ready to be drawn in any viewport.
pub struct Scene {
    page: Node,
    assets: Assets,
    boxes: HashMap<NodeId, LayoutBox>,
    /// The visible boards of the page, in page order.
    boards: Vec<Board>,
}

/// The boards of `page`: its children, or the node itself if it is not a page.
fn board_nodes(page: &Node) -> &[Node] {
    match page.data() {
        NodeData::Page(page) => &page.children,
        _ => std::slice::from_ref(page),
    }
}

impl Scene {
    pub fn new(page: Node, assets: Assets) -> Self {
        let boxes = layout::compute(&page, assets.fonts());
        let boards = board_nodes(&page)
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                Some(Board {
                    index,
                    id: *node.id(),
                    bounds: *boxes.get(node.id())?,
                    reach: paint::reach(node, &boxes)?,
                    picture: None,
                    snapshot: None,
                })
            })
            .collect();
        Self {
            page,
            assets,
            boxes,
            boards,
        }
    }

    /// The smallest rectangle that holds every board.
    pub fn bounds(&self) -> LayoutBox {
        let mut boards = self.boards.iter().map(|board| board.bounds);
        let Some(first) = boards.next() else {
            return LayoutBox::default();
        };
        let (left, top, right, bottom) = boards.fold(
            (
                first.x,
                first.y,
                first.x + first.width,
                first.y + first.height,
            ),
            |(left, top, right, bottom), board| {
                (
                    left.min(board.x),
                    top.min(board.y),
                    right.max(board.x + board.width),
                    bottom.max(board.y + board.height),
                )
            },
        );
        LayoutBox {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    /// The boards that `viewport` shows, in page order. A board whose shadow
    /// reaches into the view is shown.
    pub fn visible_boards(&self, viewport: &Viewport) -> Vec<NodeId> {
        let visible = viewport.visible_rect();
        self.boards
            .iter()
            .filter(|board| board.is_in(&visible))
            .map(|board| board.id)
            .collect()
    }

    /// Draws what `viewport` shows on `canvas`, whose unit is one CSS pixel
    /// (the host has scaled it by the device pixel ratio).
    ///
    /// The detail follows the device scale, the zoom times that ratio: below
    /// 50% a board is a snapshot, and otherwise it is drawn in full.
    pub fn render(&mut self, canvas: &Canvas, viewport: &Viewport) -> FrameStats {
        let detail = if viewport.zoom() * pixel_ratio(canvas) < REDUCED_BELOW {
            Detail::Reduced
        } else {
            Detail::Full
        };
        let visible = viewport.visible_rect();
        let Scene {
            page,
            assets,
            boxes,
            boards,
        } = self;
        let nodes = board_nodes(page);
        let mut stats = FrameStats {
            boards_drawn: Vec::new(),
            nodes_painted: 0,
            detail,
        };
        let (x, y) = viewport.origin();
        canvas.save();
        canvas.scale((viewport.zoom(), viewport.zoom()));
        canvas.translate((-x, -y));
        for board in boards.iter_mut().filter(|board| board.is_in(&visible)) {
            stats.boards_drawn.push(board.id);
            stats.nodes_painted += board.record(&nodes[board.index], boxes, assets);
            match detail {
                Detail::Full => board.draw_full(canvas),
                Detail::Reduced => {
                    board.snapshot();
                    board.draw_reduced(canvas);
                }
            }
        }
        canvas.restore();
        stats
    }

    /// Draws a frame on a transparent CPU surface of the viewport's size times
    /// `pixel_ratio` and returns it as unpremultiplied RGBA.
    pub fn render_rgba(&mut self, viewport: &Viewport, pixel_ratio: f32) -> (FrameStats, Vec<u8>) {
        let (width, height) = viewport.size();
        let size = (
            (width * pixel_ratio).round().max(1.0) as i32,
            (height * pixel_ratio).round().max(1.0) as i32,
        );
        let mut surface =
            surfaces::raster_n32_premul(size).expect("Skia creates a raster surface of the size");
        let canvas = surface.canvas();
        canvas.scale((pixel_ratio, pixel_ratio));
        let stats = self.render(canvas, viewport);

        let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Unpremul, None);
        let mut rgba = vec![0; info.compute_min_byte_size()];
        let read = surface.read_pixels(&info, &mut rgba, info.min_row_bytes(), (0, 0));
        assert!(read, "Skia reads the drawn pixels back as RGBA");
        (stats, rgba)
    }
}

/// Device pixels per canvas pixel: how much the host has scaled `canvas`.
fn pixel_ratio(canvas: &Canvas) -> f32 {
    let along_x = canvas.local_to_device_as_3x3().map_vector((1.0, 0.0));
    let ratio = along_x.x.hypot(along_x.y);
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    }
}
