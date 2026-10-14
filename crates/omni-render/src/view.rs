//! Looking at a design: zooming and panning across it, drawing only the
//! boards that are in view, and drawing them in less detail when they are
//! too small to see it.
//!
//! These are the signatures US-006's tests are written against. Every body is
//! `todo!()` until the green step.

use crate::layout::LayoutBox;
use crate::paint::Assets;
use omni_core::design::{Node, NodeId};
use skia_safe::Canvas;

/// The smallest zoom, so that the whole stress file fits in view.
pub const MIN_ZOOM: f32 = 0.01;
/// The largest zoom.
pub const MAX_ZOOM: f32 = 64.0;

/// The part of the page that the screen shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    _private: (),
}

impl Viewport {
    /// A screen of `width` x `height` CSS pixels at 100%, showing the page
    /// from its origin.
    pub fn new(width: f32, height: f32) -> Self {
        let _ = (width, height);
        todo!("a viewport at 100% from the page origin")
    }

    /// A screen of `width` x `height` whose top-left corner shows the page
    /// point (`x`, `y`) at `zoom`.
    pub fn at(x: f32, y: f32, zoom: f32, width: f32, height: f32) -> Self {
        let _ = (x, y, zoom, width, height);
        todo!("a viewport at a place")
    }

    /// Screen pixels per page pixel.
    pub fn zoom(&self) -> f32 {
        todo!("the zoom")
    }

    /// The page point at the top-left corner of the screen.
    pub fn origin(&self) -> (f32, f32) {
        todo!("the origin")
    }

    /// The size of the screen in CSS pixels.
    pub fn size(&self) -> (f32, f32) {
        todo!("the size")
    }

    /// Changes the size of the screen, keeping the zoom and the page point at
    /// its top-left corner.
    pub fn resize(&mut self, width: f32, height: f32) {
        let _ = (width, height);
        todo!("resize")
    }

    /// The page point under the screen point `screen`.
    pub fn to_design(&self, screen: (f32, f32)) -> (f32, f32) {
        let _ = screen;
        todo!("screen to page")
    }

    /// The screen point that shows the page point `design`.
    pub fn to_screen(&self, design: (f32, f32)) -> (f32, f32) {
        let _ = design;
        todo!("page to screen")
    }

    /// The rectangle of the page that the screen shows.
    pub fn visible_rect(&self) -> LayoutBox {
        todo!("the visible rectangle")
    }

    /// Multiplies the zoom by `factor` (within `MIN_ZOOM` and `MAX_ZOOM`),
    /// keeping the page point under the screen point `anchor` where it is.
    pub fn zoom_at(&mut self, factor: f32, anchor: (f32, f32)) {
        let _ = (factor, anchor);
        todo!("zoom around a point")
    }

    /// Scrolls the page by `dx`, `dy` screen pixels: a positive `dy` shows
    /// what is further down.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        let _ = (dx, dy);
        todo!("pan")
    }

    /// Zooms and pans so that `rect` is wholly in view, as large as it can be
    /// and centered.
    pub fn fit(&mut self, rect: LayoutBox) {
        let _ = rect;
        todo!("fit a rectangle")
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
    _private: (),
}

impl Controls {
    pub fn new(viewport: Viewport) -> Self {
        let _ = viewport;
        todo!("controls")
    }

    pub fn viewport(&self) -> &Viewport {
        todo!("the viewport")
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        todo!("the viewport")
    }

    /// Ctrl or Cmd with the wheel zooms around the pointer, which is also how
    /// browsers report a trackpad pinch; the wheel alone pans.
    pub fn wheel(&mut self, event: Wheel) -> bool {
        let _ = event;
        todo!("wheel")
    }

    /// A touch pinch: `scale` is the change in distance between the fingers
    /// since the last call, centered on the point `x`, `y` between them.
    pub fn pinch(&mut self, scale: f32, x: f32, y: f32) -> bool {
        let _ = (scale, x, y);
        todo!("pinch")
    }

    /// Whether the space key is held.
    pub fn set_space(&mut self, held: bool) {
        let _ = held;
        todo!("space")
    }

    pub fn pointer_down(&mut self, x: f32, y: f32) -> bool {
        let _ = (x, y);
        todo!("pointer down")
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        let _ = (x, y);
        todo!("pointer move")
    }

    pub fn pointer_up(&mut self) {
        todo!("pointer up")
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

/// A page laid out once, ready to be drawn in any viewport.
pub struct Scene {
    _private: (),
}

impl Scene {
    pub fn new(page: Node, assets: Assets) -> Self {
        let _ = (page, assets);
        todo!("a scene")
    }

    /// The smallest rectangle that holds every board.
    pub fn bounds(&self) -> LayoutBox {
        todo!("bounds")
    }

    /// The boards that `viewport` shows, in page order.
    pub fn visible_boards(&self, viewport: &Viewport) -> Vec<NodeId> {
        let _ = viewport;
        todo!("visible boards")
    }

    /// Draws what `viewport` shows on `canvas`, whose unit is one CSS pixel
    /// (the host has scaled it by the device pixel ratio).
    pub fn render(&mut self, canvas: &Canvas, viewport: &Viewport) -> FrameStats {
        let _ = (canvas, viewport);
        todo!("render")
    }

    /// Draws a frame on a transparent CPU surface of the viewport's size times
    /// `pixel_ratio` and returns it as unpremultiplied RGBA.
    pub fn render_rgba(&mut self, viewport: &Viewport, pixel_ratio: f32) -> (FrameStats, Vec<u8>) {
        let _ = (viewport, pixel_ratio);
        todo!("render to pixels")
    }
}
