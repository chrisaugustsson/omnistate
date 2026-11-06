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
//!   It also keeps what the reviewer points at: the hovered and the selected
//!   node (see the README's "Selection"). A scene made with a design system
//!   draws the page with its instances expanded and its tokens resolved in the
//!   scene's mode (see the README's "Instances and tokens").

use crate::inspect::Inspection;
use crate::layout::{self, LayoutBox, ShapedTexts};
use crate::paint::{self, Assets, Painted};
use crate::resolve::{self, Details, Facts, ids_of};
use crate::select::{Event, Index, Layer, Modifiers, NodeAddress, Outline, OutlineKind};
use crate::system::{DesignSystem, RenderError};
use crate::text::{StandIn, stand_ins};
use omni_core::design::proto::Interaction;
use omni_core::design::tokens::ModeChoice;
use omni_core::design::{Node, NodeData, NodeId};
use serde_json::Value;
use skia_safe::{
    AlphaType, Canvas, ColorType, FilterMode, Image, ImageInfo, MipmapMode, Paint, Picture,
    PictureRecorder, Rect, SamplingOptions, png_encoder, surfaces,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// How long a stage of resolving a board goes on before it leaves the rest to
/// the next one.
const RESOLVE_STAGE: Duration = Duration::from_millis(2);

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
/// The smallest scale of a snapshot made for a viewer (`Scene::set_frame_budget`):
/// a view zoomed out further than this shows a snapshot of it all the same.
const SNAPSHOT_SCALE_MIN: f32 = 1.0 / 64.0;
/// The longest side of a snapshot in pixels; a board larger than this at
/// `SNAPSHOT_SCALE` is snapshotted at a smaller scale.
const SNAPSHOT_MAX_SIDE: f32 = 2048.0;

/// The scale of a board's preview (`board_preview`).
const PREVIEW_SCALE: f32 = 1.0 / 16.0;
/// The longest side of a preview in pixels; a board larger than this at
/// `PREVIEW_SCALE` is previewed at a smaller scale.
const PREVIEW_MAX_SIDE: f32 = 256.0;

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

    /// With space held, or with the middle button (`middle`), grabs the page
    /// where the pointer is. Otherwise the pointer is left to selection
    /// (US-007).
    pub fn pointer_down(&mut self, x: f32, y: f32, middle: bool) -> bool {
        self.grabbed = (self.space || middle).then(|| self.viewport.to_design((x, y)));
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
    /// Whether a board was drawn from an older or smaller picture than the
    /// view calls for, because the frame's budget (`Scene::set_frame_budget`)
    /// was spent: drawing again makes the sharper one.
    pub deferred: bool,
}

/// How the page's first board stands in a view: what shows that a viewer has
/// it on screen complete and can point at its nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FirstBoard {
    pub id: NodeId,
    /// Its box on the screen of the view, in CSS pixels from the top-left corner.
    pub screen: LayoutBox,
    /// Whether all of it is in the view.
    pub whole: bool,
    /// What its picture painted at full detail; nothing before that is
    /// recorded. A placeholder is one node.
    pub painted: Painted,
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
    /// Whether `picture` is the drawing of the board that this one replaced,
    /// kept until the new one is recorded.
    picture_stale: bool,
    /// What recording that drawing painted.
    painted: Painted,
    /// The board's drawing at reduced detail, once it has been made.
    snapshot: Option<Snapshot>,
    /// Whether `snapshot` is the preview of a board that has not been read
    /// (`Scene::use_preview`): nothing is drawn from the node, which is only
    /// the board's placeholder.
    previewed: bool,
}

/// A board drawn small, with where it goes on the page.
struct Snapshot {
    image: Image,
    rect: Rect,
    /// How much smaller than the board the image is.
    scale: f32,
    /// Whether it is the snapshot of the board that this one replaced, kept
    /// until the new one is made.
    stale: bool,
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
        texts: &ShapedTexts,
        assets: &Assets,
    ) -> usize {
        if self.picture.is_some() && !self.picture_stale {
            return 0;
        }
        let mut recorder = PictureRecorder::new();
        // The bounding box hierarchy lets a replay skip the parts of the
        // board that are out of view.
        let canvas = recorder.begin_recording(self.reach, true);
        self.painted = paint::draw_counted(canvas, node, boxes, texts, assets);
        self.picture = recorder.finish_recording_as_picture(None);
        self.picture_stale = false;
        self.painted.nodes
    }

    /// Draws the recorded board on `canvas`, in page coordinates.
    fn draw_full(&self, canvas: &Canvas) {
        if let Some(picture) = &self.picture {
            canvas.draw_picture(picture, None, None);
        }
    }

    /// The scale of the snapshot that a view whose device scale is `need`
    /// calls for. A scene that draws for a viewer (`adaptive`) makes snapshots
    /// no larger than the view can show, in steps of two, so that the many
    /// boards of a page zoomed far out cost little to snapshot.
    fn wanted_scale(&self, need: f32, adaptive: bool) -> f32 {
        let reach = self.reach;
        let longest = reach.width().max(reach.height());
        let scale = if adaptive {
            2f32.powf(need.log2().ceil())
                .clamp(SNAPSHOT_SCALE_MIN, SNAPSHOT_SCALE)
        } else {
            SNAPSHOT_SCALE
        };
        scale.min(SNAPSHOT_MAX_SIDE / longest)
    }

    /// Whether the snapshot is of this board and sharp enough for `wanted`.
    fn snapshot_holds(&self, wanted: f32) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.stale && snapshot.scale >= wanted * 0.99)
    }

    /// Makes the snapshot of `node`, the board, at `scale` unless the board
    /// has one that holds (`snapshot_holds`), and returns how many nodes making
    /// it painted. It is drawn at reduced detail (`paint::draw_reduced`), as it
    /// is only ever seen small, so it needs neither the board's picture nor the
    /// shaping of its text.
    fn snapshot(
        &mut self,
        node: &Node,
        boxes: &HashMap<NodeId, LayoutBox>,
        assets: &Assets,
        scale: f32,
    ) -> usize {
        if self.snapshot_holds(scale) {
            return 0;
        }
        let reach = self.reach;
        let size = (
            (reach.width() * scale).ceil().max(1.0) as i32,
            (reach.height() * scale).ceil().max(1.0) as i32,
        );
        let Some(mut surface) = surfaces::raster_n32_premul(size) else {
            return 0;
        };
        let canvas = surface.canvas();
        canvas.scale((scale, scale));
        canvas.translate((-reach.left, -reach.top));
        let painted = paint::draw_reduced(canvas, node, boxes, assets);
        self.snapshot = Some(Snapshot {
            image: surface.image_snapshot(),
            rect: Rect::from_xywh(
                reach.left,
                reach.top,
                size.0 as f32 / scale,
                size.1 as f32 / scale,
            ),
            scale,
            stale: false,
        });
        painted
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
    /// The page that is laid out and drawn: the page as given, or with a
    /// design system, the page with its instances expanded and its tokens
    /// resolved in the current mode.
    page: Node,
    /// The page as given, when a design system resolves it.
    source: Option<Node>,
    system: Option<Resolving>,
    errors: Vec<RenderError>,
    /// What resolving noted of the nodes, for the inspect panel.
    details: Details,
    /// The IDs of the page and those made up for its instances, and how many
    /// were made up, for the boards that `queue_boards` resolves.
    taken: HashSet<NodeId>,
    next_id: u64,
    /// The stages of boards waiting to be put in place.
    jobs: VecDeque<Job>,
    /// How many boards have been put in place.
    placed: u64,
    /// How long a frame spends making pictures and snapshots before it leaves
    /// the rest to the next frames, for a scene drawn for a viewer; `None`
    /// makes them all in the first frame that needs them.
    frame_budget: Option<Duration>,
    assets: Assets,
    boxes: HashMap<NodeId, LayoutBox>,
    /// The text nodes, shaped as layout measured them.
    texts: ShapedTexts,
    /// The visible boards of the page, in page order.
    boards: Vec<Board>,
    /// The boxes of every node, to find the node at a point.
    index: Index,
    hovered: Option<usize>,
    /// The selected node. The context is always its parent (the board is its
    /// own context), so it is not kept apart.
    selected: Option<usize>,
    events: Vec<Event>,
}

/// A stage of putting a board in place of the board of the page that has its
/// ID (`Scene::queue_boards`), by the board's position among the page's children.
enum Job {
    /// Resolve the board.
    Resolve(usize, Node),
    /// Resolve the board a little more.
    Resolving(usize, resolve::BoardResolving),
    /// Lay out the resolved board and put it in place.
    Place(usize, resolve::ResolvedBoard),
}

/// The design system that a scene resolves its page against, and the mode it
/// resolves tokens in, a mode for each collection.
struct Resolving {
    system: DesignSystem,
    choice: ModeChoice,
}

/// What laying out a page gives a scene.
struct Laid {
    boxes: HashMap<NodeId, LayoutBox>,
    texts: ShapedTexts,
    boards: Vec<Board>,
    index: Index,
}

/// What the inspection shows as the scene's mode: the mode when every
/// collection has the same one, else each collection with its mode, such as
/// `color:dark,density:compact`.
fn mode_text(choice: &ModeChoice) -> String {
    let mut modes = choice.0.values();
    match modes.next() {
        Some(first) if modes.any(|mode| mode != first) => choice
            .0
            .iter()
            .map(|(name, mode)| format!("{name}:{mode}"))
            .collect::<Vec<_>>()
            .join(","),
        Some(first) => first.clone(),
        None => String::new(),
    }
}

/// The boards of `page`: its children, or the node itself if it is not a page.
fn board_nodes(page: &Node) -> &[Node] {
    match page.data() {
        NodeData::Page(page) => &page.children,
        _ => std::slice::from_ref(page),
    }
}

/// The `overflow` and `scrollPosition` the inspect panel shows for `node`:
/// both for a frame, only the overflow for a board, with the defaults where
/// the node leaves them out, and neither for any other node.
fn scrolling(node: &Node) -> (Option<String>, Option<String>) {
    match node.data() {
        NodeData::Board(frame) => (Some(frame.overflow.as_str().to_string()), None),
        NodeData::Frame(frame) => (
            Some(frame.overflow.as_str().to_string()),
            Some(node.scroll_position().as_str().to_string()),
        ),
        _ => (None, None),
    }
}

/// An interaction as the inspect panel shows it: its JSON form with the
/// defaults filled in and each action's target named by `target` (name and
/// kind of the node a `dest` is). `dangling` is set when an action points at
/// a node that `target` does not find.
fn interaction_json(
    interaction: &Interaction,
    target: &dyn Fn(&str) -> Option<(String, String)>,
) -> Value {
    let mut json = serde_json::to_value(interaction).unwrap_or(Value::Null);
    let mut dangling = false;
    if let Some(map) = json.as_object_mut() {
        let trigger = map
            .entry("trigger")
            .or_insert_with(|| serde_json::json!({"type": "onClick"}));
        if let Some(bare) = trigger.as_str() {
            *trigger = serde_json::json!({"type": bare});
        }
        let transition = map
            .entry("transition")
            .or_insert_with(|| serde_json::json!({"type": "instant"}));
        if let Some(bare) = transition.as_str() {
            *transition = serde_json::json!({"type": bare});
        }
        map.entry("durationMs").or_insert(Value::from(300));
        map.entry("easing").or_insert(Value::from("easeOut"));
        map.entry("delayMs").or_insert(Value::from(0));
        if let Some(Value::Array(actions)) = map.get_mut("actions") {
            for action in actions {
                dangling |= name_targets(action, target);
            }
        }
        map.insert("dangling".to_string(), Value::Bool(dangling));
    }
    json
}

/// Adds `destName` and `destKind` to `action` and to the actions of its
/// conditional branches, and says whether a target is missing.
fn name_targets(action: &mut Value, target: &dyn Fn(&str) -> Option<(String, String)>) -> bool {
    let mut dangling = false;
    let Some(map) = action.as_object_mut() else {
        return false;
    };
    if let Some(dest) = map.get("dest").and_then(Value::as_str) {
        match target(dest) {
            Some((name, kind)) => {
                map.insert("destName".to_string(), Value::from(name));
                map.insert("destKind".to_string(), Value::from(kind));
            }
            None => dangling = true,
        }
    }
    if let Some(Value::Array(blocks)) = map.get_mut("blocks") {
        for block in blocks {
            if let Some(Value::Array(actions)) = block.get_mut("actions") {
                for inner in actions {
                    dangling |= name_targets(inner, target);
                }
            }
        }
    }
    dangling
}

/// The node `id` in `root` or below it.
fn find_node<'a>(root: &'a Node, id: &NodeId) -> Option<&'a Node> {
    if root.id() == id {
        return Some(root);
    }
    root.children()
        .iter()
        .find_map(|child| find_node(child, id))
}

/// Lays `page` out. `addresses` names the nodes inside instances
/// (`resolve::Resolved::addresses`), and `instances` the instances that
/// were expanded into frames (`resolve::Resolved::instances`).
fn lay_out(
    page: &Node,
    assets: &Assets,
    addresses: &HashMap<NodeId, NodeAddress>,
    instances: &HashSet<NodeId>,
) -> Laid {
    let (boxes, texts) = layout::compute_shaped(page, assets.fonts());
    let boards = board_nodes(page)
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            Some(Board {
                index,
                id: *node.id(),
                bounds: *boxes.get(node.id())?,
                reach: paint::reach(node, &boxes)?,
                picture: None,
                picture_stale: false,
                painted: Painted::default(),
                snapshot: None,
                previewed: false,
            })
        })
        .collect();
    let index = Index::new(board_nodes(page), &boxes, addresses, instances);
    Laid {
        boxes,
        texts,
        boards,
        index,
    }
}

/// The box that `board`, a board of a page with all its content, is laid out
/// in on its page, its instances expanded against `system` when there is one
/// (in its first mode). A board that hugs its content gets the size of that
/// content here, which its own JSON does not say: the API gives the
/// placeholders of a design's outline this box, so that a board not yet read
/// takes its real room on the canvas (US-058). `None` when `board` has no box.
pub fn board_box(board: Node, assets: &Assets, system: Option<&DesignSystem>) -> Option<LayoutBox> {
    let id = *board.id();
    let scene = match system {
        Some(system) => Scene::with_system(board, assets.clone(), system.clone()),
        None => Scene::new(board, assets.clone()),
    };
    scene.boxes.get(&id).copied()
}

/// A small picture of `board`, a board of a page with all its content, as a
/// PNG: the board at reduced detail (`paint::draw_reduced`), its instances
/// expanded against `system` when there is one (in its first mode), and with
/// the images of `assets`. The API makes one for every board that is written,
/// so that a viewer can show a board's content before it has read the board
/// (`Scene::use_preview`). `None` when `board` has no box.
pub fn board_preview(
    board: Node,
    assets: &Assets,
    system: Option<&DesignSystem>,
) -> Option<Vec<u8>> {
    let scene = match system {
        Some(system) => Scene::with_system(board, assets.clone(), system.clone()),
        None => Scene::new(board, assets.clone()),
    };
    let held = scene.boards.first()?;
    let bounds = held.bounds;
    let scale = PREVIEW_SCALE.min(PREVIEW_MAX_SIDE / bounds.width.max(bounds.height));
    let size = (
        (bounds.width * scale).ceil().max(1.0) as i32,
        (bounds.height * scale).ceil().max(1.0) as i32,
    );
    let mut surface = surfaces::raster_n32_premul(size)?;
    let canvas = surface.canvas();
    canvas.scale((size.0 as f32 / bounds.width, size.1 as f32 / bounds.height));
    canvas.translate((-bounds.x, -bounds.y));
    let node = &board_nodes(&scene.page)[held.index];
    paint::draw_reduced(canvas, node, &scene.boxes, &scene.assets);
    let pixels = surface.peek_pixels()?;
    let mut png = Vec::new();
    png_encoder::encode(&pixels, &mut png, &png_encoder::Options::default()).then_some(png)
}

/// The longest side of a board's PNG (`board_png`) in pixels.
pub const BOARD_PNG_MAX_SIDE: u32 = 16384;

/// The pixels along a board's side of `length` at `scale`: rounded up, at
/// least one.
pub fn board_png_side(length: f32, scale: f32) -> u32 {
    (length * scale - 1e-6).ceil().max(1.0) as u32
}

/// `board`, a board of a page with all its content, as a PNG of the board's
/// own box at `scale` (2 for the pictures of an export): drawn as the viewer
/// draws a frame at that zoom, its instances expanded against `system` when
/// there is one (in its first mode), on a transparent background, and cut at
/// the board's box, so that a shadow does not reach beyond it. `None` when
/// `board` has no box or a side would exceed [`BOARD_PNG_MAX_SIDE`].
pub fn board_png(
    board: Node,
    assets: &Assets,
    system: Option<&DesignSystem>,
    scale: f32,
) -> Option<Vec<u8>> {
    let mut scene = match system {
        Some(system) => Scene::with_system(board, assets.clone(), system.clone()),
        None => Scene::new(board, assets.clone()),
    };
    let bounds = scene.boards.first()?.bounds;
    let size = (
        board_png_side(bounds.width, scale),
        board_png_side(bounds.height, scale),
    );
    if size.0 > BOARD_PNG_MAX_SIDE || size.1 > BOARD_PNG_MAX_SIDE {
        return None;
    }
    let mut surface = surfaces::raster_n32_premul((size.0 as i32, size.1 as i32))?;
    let viewport = Viewport::at(bounds.x, bounds.y, scale, size.0 as f32, size.1 as f32);
    scene.render(surface.canvas(), &viewport);
    let pixels = surface.peek_pixels()?;
    let mut png = Vec::new();
    png_encoder::encode(&pixels, &mut png, &png_encoder::Options::default()).then_some(png)
}

/// A board's width and, for each address asked for, the node's box from the
/// board's top-left corner with its name, or `None` when the board has no node
/// there.
pub type NodesOnBoard = (f32, Vec<Option<(LayoutBox, String)>>);

/// Where the nodes at `addresses` sit on `board`, a board of a page with all its
/// content, laid out as `board_preview` lays it out, so that a viewer can pin a
/// node on a board it has not read. `None` when `board` has no box.
pub fn nodes_on_board(
    board: Node,
    assets: &Assets,
    system: Option<&DesignSystem>,
    addresses: &[NodeAddress],
) -> Option<NodesOnBoard> {
    let scene = match system {
        Some(system) => Scene::with_system(board, assets.clone(), system.clone()),
        None => Scene::new(board, assets.clone()),
    };
    let bounds = scene.boards.first()?.bounds;
    let nodes = addresses
        .iter()
        .map(|address| {
            let rect = scene.bounds_of(address)?;
            let placed = LayoutBox {
                x: rect.x - bounds.x,
                y: rect.y - bounds.y,
                ..rect
            };
            Some((placed, scene.name_of(address)?.to_string()))
        })
        .collect();
    Some((bounds.width, nodes))
}

/// The images that drawing `board` calls for, as `attachment:<id>`: its own and
/// those of the components it places, with its instances expanded against
/// `system` when there is one.
pub fn board_images(board: &Node, system: Option<&DesignSystem>) -> Vec<String> {
    fn visit(value: &Value, found: &mut HashSet<String>) {
        match value {
            Value::Array(values) => values.iter().for_each(|value| visit(value, found)),
            Value::Object(fields) => {
                for (key, value) in fields {
                    match value.as_str() {
                        Some(text) if key == "image" => {
                            found.insert(text.to_string());
                        }
                        _ => visit(value, found),
                    }
                }
            }
            _ => {}
        }
    }
    let json = match system {
        Some(system) => {
            let choice = system.first_modes();
            serde_json::to_value(&resolve::resolve(board, system, &choice).page)
        }
        None => serde_json::to_value(board),
    };
    let mut found = HashSet::new();
    if let Ok(json) = json {
        visit(&json, &mut found);
    }
    found.into_iter().collect()
}

impl Scene {
    pub fn new(page: Node, assets: Assets) -> Self {
        let laid = lay_out(&page, &assets, &HashMap::new(), &HashSet::new());
        Self {
            page,
            source: None,
            system: None,
            errors: Vec::new(),
            details: Details::default(),
            taken: HashSet::new(),
            next_id: 0,
            jobs: VecDeque::new(),
            placed: 0,
            frame_budget: None,
            assets,
            boxes: laid.boxes,
            texts: laid.texts,
            boards: laid.boards,
            index: laid.index,
            hovered: None,
            selected: None,
            events: Vec::new(),
        }
    }

    /// A scene whose instances and token references resolve against
    /// `system`, in its first mode.
    pub fn with_system(page: Node, assets: Assets, system: DesignSystem) -> Self {
        let choice = system.first_modes();
        let resolved = resolve::resolve(&page, &system, &choice);
        let laid = lay_out(
            &resolved.page,
            &assets,
            &resolved.addresses,
            &resolved.instances,
        );
        Self {
            page: resolved.page,
            source: Some(page),
            system: Some(Resolving { system, choice }),
            errors: resolved.errors,
            details: resolved.details,
            taken: resolved.taken,
            next_id: resolved.next,
            jobs: VecDeque::new(),
            placed: 0,
            frame_budget: None,
            assets,
            boxes: laid.boxes,
            texts: laid.texts,
            boards: laid.boards,
            index: laid.index,
            hovered: None,
            selected: None,
            events: Vec::new(),
        }
    }

    /// Resolves the scene's instances and token references against `system`
    /// from now on, in its first mode, in place of any design system it had.
    pub fn set_system(&mut self, system: DesignSystem) {
        while self.work() {}
        let page = match self.source.take() {
            Some(page) => page,
            None => self.page.clone(),
        };
        let choice = system.first_modes();
        let resolved = resolve::resolve(&page, &system, &choice);
        self.source = Some(page);
        self.system = Some(Resolving { system, choice });
        self.adopt(resolved);
    }

    /// Resolves token references in the modes that `text` chooses from now
    /// on: a bare mode name, or a JSON object of collection to mode (see
    /// `DesignSystem::choose`). Fails, and keeps the current modes, if the
    /// design system has no such mode or collection. A board that names a
    /// mode for a collection (`modes`) keeps it; the choice changes the rest.
    pub fn set_mode(&mut self, text: &str) -> Result<(), String> {
        while self.work() {}
        let (Some(resolving), Some(source)) = (&mut self.system, &self.source) else {
            return Err("the scene has no design system, so it has no modes".to_string());
        };
        let choice = resolving.system.choose(&resolving.choice, text)?;
        let resolved = resolve::resolve(source, &resolving.system, &choice);
        resolving.choice = choice;
        self.adopt(resolved);
        Ok(())
    }

    /// Lays out and draws `resolved` from now on. The same nodes are there
    /// whenever the design system or its mode changes, so the pointer and the
    /// selection stay on them.
    fn adopt(&mut self, resolved: resolve::Resolved) {
        let laid = lay_out(
            &resolved.page,
            &self.assets,
            &resolved.addresses,
            &resolved.instances,
        );
        let hovered = self.hovered.map(|at| self.index.address(at));
        let selected = self.selected.map(|at| self.index.address(at));
        let mut previews: HashMap<NodeId, Snapshot> = self
            .boards
            .iter_mut()
            .filter(|board| board.previewed)
            .filter_map(|board| Some((board.id, board.snapshot.take()?)))
            .collect();
        self.page = resolved.page;
        self.errors = resolved.errors;
        self.details = resolved.details;
        self.taken = resolved.taken;
        self.next_id = resolved.next;
        self.boxes = laid.boxes;
        self.texts = laid.texts;
        self.boards = laid.boards;
        for board in &mut self.boards {
            if let Some(preview) = previews.remove(&board.id) {
                board.snapshot = Some(preview);
                board.previewed = true;
            }
        }
        self.index = laid.index;
        self.hovered = hovered.and_then(|address| self.index.lookup(&address));
        self.selected = selected.and_then(|address| self.index.lookup(&address));
    }

    /// Makes the scene draw for a viewer: a frame spends about `budget` making
    /// the pictures and snapshots of the boards it shows, and draws a board
    /// that has an older or smaller one from that, so that the view keeps
    /// moving while the sharper ones are made over the next frames
    /// (`FrameStats::deferred`). Snapshots are made no larger than the view
    /// can show. A board put in place of another (`queue_boards`) is drawn from
    /// the old one's pictures until its own are made.
    pub fn set_frame_budget(&mut self, budget: Duration) {
        self.frame_budget = Some(budget);
    }

    /// Draws the board `board`, whose placeholder is in the page because it has
    /// not been read, from `image` (a [`board_preview`] of it) until `queue_boards`
    /// puts the board in place, so that it shows its content in every frame.
    /// The image is stretched over the board's box. Returns `false`, and
    /// changes nothing, if the page has no such board.
    pub fn use_preview(&mut self, board: &NodeId, image: Image) -> bool {
        let Some(held) = self.boards.iter_mut().find(|held| held.id == *board) else {
            return false;
        };
        let bounds = held.bounds;
        held.snapshot = Some(Snapshot {
            scale: image.width() as f32 / bounds.width,
            image,
            rect: Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height),
            stale: true,
        });
        held.previewed = true;
        true
    }

    /// Queues `boards` to take the place of the boards of the page that have
    /// their IDs (a board read after the page was shown as a placeholder, or
    /// read again after it changed), with `assets`. Nothing changes until
    /// `work` has done the jobs, a stage of one board each, so that a caller
    /// can spread a board's laying out over several frames. Returns `false`,
    /// and queues nothing, if the scene is not a page or the page has no board
    /// with the ID of one of `boards`.
    pub fn queue_boards(&mut self, boards: Vec<Node>, assets: Assets) -> bool {
        if !matches!(self.page.data(), NodeData::Page(_)) {
            return false;
        }
        let positions: Option<Vec<usize>> = boards
            .iter()
            .map(|board| {
                board_nodes(&self.page)
                    .iter()
                    .position(|held| held.id() == board.id())
            })
            .collect();
        let Some(positions) = positions else {
            return false;
        };
        self.assets = assets;
        self.jobs.extend(
            positions
                .into_iter()
                .zip(boards)
                .map(|(position, board)| Job::Resolve(position, board)),
        );
        true
    }

    /// How many boards `work` has put in place so far.
    pub fn placed(&self) -> u64 {
        self.placed
    }

    /// Whether `work` has jobs to do.
    pub fn has_work(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// Does the next stage of the queued boards (`queue_boards`): resolving a
    /// board a little (about `RESOLVE_STAGE` of its nodes' time), or laying it
    /// out and putting it in place. Everything else stays as it is: the view,
    /// the pictures of the other boards, and the hover and the selection while
    /// their nodes are still there. Returns whether there is more to do.
    pub fn work(&mut self) -> bool {
        match self.jobs.pop_front() {
            Some(Job::Resolve(position, board)) => self.begin_board(position, board),
            Some(Job::Resolving(position, mut resolving)) => {
                let finished = match &self.system {
                    Some(resolving_with) => resolving.step(
                        &resolving_with.system,
                        &resolving_with.choice,
                        RESOLVE_STAGE,
                        &mut self.taken,
                        &mut self.next_id,
                    ),
                    None => None,
                };
                self.jobs.push_front(match finished {
                    Some(resolved) => Job::Place(position, resolved),
                    None => Job::Resolving(position, resolving),
                });
            }
            Some(Job::Place(position, resolved)) => {
                let hovered = self.hovered.map(|at| self.index.address(at));
                let selected = self.selected.map(|at| self.index.address(at));
                self.place_board(position, resolved);
                self.hovered = hovered.and_then(|address| self.index.lookup(&address));
                self.selected = selected.and_then(|address| self.index.lookup(&address));
            }
            None => {}
        }
        self.has_work()
    }

    /// Queues `boards` and does all the jobs (`queue_boards`, `work`).
    pub fn replace_boards(&mut self, boards: Vec<Node>, assets: Assets) -> bool {
        let queued = self.queue_boards(boards, assets);
        while self.work() {}
        queued
    }

    /// Starts resolving `board`, which is to take the place of the board at
    /// `position` among the page's children: its instances are expanded and
    /// its token references resolved by the jobs that follow (`work`).
    fn begin_board(&mut self, position: usize, board: Node) {
        // The IDs of the board that is replaced are free again.
        for gone in &ids_of(&board_nodes(&self.page)[position]) {
            self.taken.remove(gone);
        }
        let job = match (&mut self.source, &self.system) {
            (Some(source), Some(_)) => {
                if let Some(slot) = source
                    .children_mut()
                    .and_then(|children| children.get_mut(position))
                {
                    *slot = board.clone();
                }
                Job::Resolving(
                    position,
                    resolve::begin_board(board, &mut self.taken, &mut self.next_id),
                )
            }
            _ => Job::Place(
                position,
                resolve::ResolvedBoard {
                    board,
                    addresses: HashMap::new(),
                    instances: HashSet::new(),
                    details: Details::default(),
                    errors: Vec::new(),
                },
            ),
        };
        self.jobs.push_front(job);
    }

    /// Lays out `resolved`, a resolved board, and puts it in place of the
    /// board at `position` among the page's children (`work`).
    fn place_board(&mut self, position: usize, resolved: resolve::ResolvedBoard) {
        let resolve::ResolvedBoard {
            board,
            addresses,
            instances,
            details,
            errors,
        } = resolved;
        let id = *board.id();
        for gone in &ids_of(&board_nodes(&self.page)[position]) {
            self.boxes.remove(gone);
            self.texts.remove(gone);
            self.details.tokens.remove(gone);
            self.details.instances.remove(gone);
        }
        self.details.tokens.extend(details.tokens);
        self.details.instances.extend(details.instances);
        let (boxes, texts) = layout::compute_shaped(&board, self.assets.fonts());
        self.boxes.extend(boxes);
        self.texts.extend(texts);
        let removed =
            self.index
                .replace_board(position, &board, &self.boxes, &addresses, &instances);
        if !self.errors.is_empty() {
            let removed: HashSet<NodeAddress> = removed.into_iter().collect();
            self.errors.retain(|error| !removed.contains(&error.node));
        }
        self.errors.extend(errors);
        let entry = self.boxes.get(&id).copied().and_then(|bounds| {
            Some(Board {
                index: position,
                id,
                bounds,
                reach: paint::reach(&board, &self.boxes)?,
                picture: None,
                picture_stale: false,
                painted: Painted::default(),
                snapshot: None,
                previewed: false,
            })
        });
        let held = self.boards.iter().position(|board| board.id == id);
        match (held, entry) {
            (Some(at), Some(mut entry)) => {
                if self.frame_budget.is_some() {
                    let old = &mut self.boards[at];
                    entry.picture_stale = old.picture.is_some();
                    entry.picture = old.picture.take();
                    entry.snapshot = old.snapshot.take().map(|snapshot| Snapshot {
                        stale: true,
                        ..snapshot
                    });
                }
                self.boards[at] = entry;
            }
            (Some(at), None) => {
                self.boards.remove(at);
            }
            (None, Some(entry)) => {
                let at = self.boards.partition_point(|board| board.index < position);
                self.boards.insert(at, entry);
            }
            (None, None) => {}
        }
        if let Some(children) = self.page.children_mut() {
            children[position] = board;
        }
        self.placed += 1;
    }

    /// The nodes drawn as error placeholders, with what each was missing.
    pub fn errors(&self) -> &[RenderError] {
        &self.errors
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

    /// What a viewer opens the page on, on a screen of `pixel_ratio` device
    /// pixels per CSS pixel: all of it where that draws its boards in full
    /// detail, and otherwise its first board, so that what is on screen first
    /// can be read and a page of hundreds of boards does not open as specks.
    /// A page opened by a link to the board `linked` opens on that board when
    /// the opening would not hold all of it; a board the page does not have
    /// changes nothing.
    pub fn opening(
        &self,
        viewport: &Viewport,
        pixel_ratio: f32,
        linked: Option<NodeId>,
    ) -> LayoutBox {
        let bounds = self.bounds();
        let mut all = *viewport;
        all.fit(bounds);
        let opening = match self.boards.first() {
            Some(first) if all.zoom() * pixel_ratio < REDUCED_BELOW => first.bounds,
            _ => bounds,
        };
        let holds = |board: &LayoutBox| {
            board.x >= opening.x
                && board.y >= opening.y
                && board.x + board.width <= opening.x + opening.width
                && board.y + board.height <= opening.y + opening.height
        };
        match linked.and_then(|id| self.boards.iter().find(|board| board.id == id)) {
            Some(board) if !holds(&board.bounds) => board.bounds,
            _ => opening,
        }
    }

    /// How the page's first board stands in `viewport`; `None` for a page
    /// without boards.
    pub fn first_board(&self, viewport: &Viewport) -> Option<FirstBoard> {
        // The fit of a view is exact only to a fraction of a page pixel.
        const MARGIN: f32 = 1.0;
        let first = self.boards.first()?;
        let visible = viewport.visible_rect();
        let bounds = first.bounds;
        let (x, y) = viewport.to_screen((bounds.x, bounds.y));
        Some(FirstBoard {
            id: first.id,
            screen: LayoutBox {
                x,
                y,
                width: bounds.width * viewport.zoom(),
                height: bounds.height * viewport.zoom(),
            },
            whole: bounds.x + MARGIN >= visible.x
                && bounds.y + MARGIN >= visible.y
                && bounds.x + bounds.width <= visible.x + visible.width + MARGIN
                && bounds.y + bounds.height <= visible.y + visible.height + MARGIN,
            painted: first.painted,
        })
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

    /// The page as it was given, which a prototype plays: its boards in page
    /// order, its flows and its prototype settings.
    pub fn source_page(&self) -> &Node {
        self.source.as_ref().unwrap_or(&self.page)
    }

    /// The components of the design system the scene resolves against, by ID.
    pub fn components(&self) -> HashMap<NodeId, Node> {
        self.system
            .as_ref()
            .map(|resolving| resolving.system.component_nodes())
            .unwrap_or_default()
    }

    /// The box of the board `board` on the page.
    pub fn board_bounds(&self, board: &NodeId) -> Option<LayoutBox> {
        self.boards
            .iter()
            .find(|held| held.id == *board)
            .map(|held| held.bounds)
    }

    /// The IDs of the boards of the page, in page order.
    pub fn board_ids(&self) -> Vec<NodeId> {
        self.boards.iter().map(|board| board.id).collect()
    }

    /// The boxes of the layers of `board` from its top-left corner, with
    /// the address of each layer inside an instance (`Index::board_boxes`).
    pub fn board_boxes(&self, board: &NodeId) -> Option<crate::select::BoardBoxes> {
        self.index.board_boxes(board)
    }

    /// The topmost visible node at the page point `point`: the deepest node
    /// whose box holds it, with later siblings above earlier ones. Hidden
    /// nodes and what lies outside a clipping ancestor cannot be hit.
    pub fn node_at(&self, point: (f32, f32)) -> Option<NodeAddress> {
        self.index.node_at(point).map(|at| self.index.address(at))
    }

    /// The box, in page coordinates, of the node at `address`: a node ID, or
    /// `<instanceId>/<componentNodeId>` for a node inside an instance, which
    /// the scene has when it was made with a design system.
    pub fn bounds_of(&self, address: &NodeAddress) -> Option<LayoutBox> {
        Some(self.index.rect(self.index.lookup(address)?))
    }

    /// The box of the node at `address` on the screen of `viewport`, in CSS
    /// pixels from its top-left corner: where the design viewer pins the
    /// node's comments (US-045). `None` when the address names no node.
    pub fn screen_box_of(&self, address: &NodeAddress, viewport: &Viewport) -> Option<LayoutBox> {
        let page = self.bounds_of(address)?;
        let (x, y) = viewport.to_screen((page.x, page.y));
        let zoom = viewport.zoom();
        Some(LayoutBox {
            x,
            y,
            width: page.width * zoom,
            height: page.height * zoom,
        })
    }

    /// The name of the node at `address`, as the layers panel lists it.
    pub fn name_of(&self, address: &NodeAddress) -> Option<&str> {
        Some(self.index.name(self.index.lookup(address)?))
    }

    /// What a click, or the pointer moving with Ctrl or Cmd held for `deep`,
    /// at the screen point `screen` of `viewport` selects: the deepest node
    /// there when `deep`, and otherwise the child of the context that holds
    /// it, or the board when no child of the context does.
    fn target(&self, viewport: &Viewport, screen: (f32, f32), deep: bool) -> Option<usize> {
        let hit = self.index.node_at(viewport.to_design(screen))?;
        if deep {
            return Some(hit);
        }
        let board = self.index.board(hit);
        // A context in another board is not one here: a click starts over.
        let context = self
            .selected
            .map(|selected| self.index.context(selected))
            .filter(|&context| self.index.board(context) == board)
            .unwrap_or(board);
        Some(self.index.child_of(hit, context).unwrap_or(board))
    }

    fn select(&mut self, selected: Option<usize>) {
        if self.selected != selected {
            self.selected = selected;
            let address = selected.map(|at| self.index.address(at));
            self.events.push(Event::Select(address));
        }
    }

    /// Fits `viewport` to the board of the selected node, for a link to a node
    /// that opened somewhere else. A node that is in view already stays where
    /// it is.
    pub fn reveal_selected(&self, viewport: &mut Viewport) {
        let Some(selected) = self.selected else {
            return;
        };
        let visible = viewport.visible_rect();
        let node = self.index.rect(selected);
        let in_view = node.x < visible.x + visible.width
            && node.x + node.width > visible.x
            && node.y < visible.y + visible.height
            && node.y + node.height > visible.y;
        if !in_view {
            viewport.fit(self.index.rect(self.index.board(selected)));
        }
    }

    /// The texts of the boards in place that are drawn in a font other than
    /// their own, once per distinct family, weight and style asked for: the
    /// family has no font file, or the file has no face of that weight or slant.
    pub fn fonts_stood_in(&self) -> Vec<StandIn> {
        stand_ins(self.texts.values().map(|text| &**text))
    }

    /// The boards of the page in order, each without its children, as the
    /// layers panel of the design viewer lists them (US-043).
    pub fn layers(&self) -> Vec<Layer> {
        self.index.layers()
    }

    /// The node at `address` with every node below it, as a layer; a board
    /// that is not read has no children. `None` when the address names no
    /// node of the page.
    pub fn layers_of(&self, address: &NodeAddress) -> Option<Layer> {
        Some(self.index.layer(self.index.lookup(address)?))
    }

    /// The board that holds the node at `address`, or the board itself.
    /// `None` when the address names no node of the page.
    pub fn board_of(&self, address: &NodeAddress) -> Option<NodeId> {
        let at = self.index.lookup(address)?;
        Some(self.index.page_id(self.index.board(at)))
    }

    /// Selects the node at `address`, as the layers panel does (US-043), or
    /// clears the selection for `None`. Returns `false` and changes nothing
    /// when the address names no node of the scene.
    pub fn select_address(&mut self, address: Option<&NodeAddress>) -> bool {
        let Some(address) = address else {
            self.select(None);
            return true;
        };
        let Some(at) = self.index.lookup(address) else {
            return false;
        };
        self.select(Some(at));
        true
    }

    /// The node at `address` as the inspect panel of the design viewer shows
    /// it (US-044): its properties as the scene draws them, with their tokens
    /// resolved in the current mode, and for an instance what it stores.
    /// `None` when the address names no node of the scene.
    pub fn inspect(&self, address: &NodeAddress) -> Option<Inspection> {
        let at = self.index.lookup(address)?;
        let id = self.index.page_id(at);
        let node = board_nodes(&self.page)
            .iter()
            .find_map(|board| find_node(board, &id))?;
        let page = self.source.as_ref().unwrap_or(&self.page);
        let own = board_nodes(page)
            .iter()
            .find_map(|board| find_node(board, &id))
            .unwrap_or(node);
        let target = |dest: &str| -> Option<(String, String)> {
            let found = dest.parse::<NodeId>().ok()?;
            if let Some(node) = board_nodes(page)
                .iter()
                .find_map(|board| find_node(board, &found))
            {
                return Some((node.name().to_string(), node.kind().as_str().to_string()));
            }
            let component = self.system.as_ref()?.system.component(&found)?;
            Some((component.name.clone(), "component".to_string()))
        };
        let (overflow, scroll_position) = scrolling(own);
        Some(
            self.details.inspect(
                node,
                Facts {
                    interactions: own
                        .interactions()
                        .iter()
                        .map(|interaction| interaction_json(interaction, &target))
                        .collect(),
                    overflow,
                    scroll_position,
                    address: address.clone(),
                    name: self.index.name(at).to_string(),
                    kind: self.index.kind(at).to_string(),
                    mode: self
                        .system
                        .as_ref()
                        .map(|resolving| mode_text(&resolving.choice)),
                    bounds: self.index.rect(at),
                },
            ),
        )
    }

    /// The pointer moved to the screen point `screen` of `viewport`.
    pub fn pointer_move(&mut self, viewport: &Viewport, screen: (f32, f32), keys: Modifiers) {
        let hovered = self.target(viewport, screen, keys.ctrl || keys.meta);
        if self.hovered != hovered {
            self.hovered = hovered;
            let address = hovered.map(|at| self.index.address(at));
            self.events.push(Event::Hover(address));
        }
    }

    /// A click at the screen point `screen` of `viewport`. With Ctrl or Cmd
    /// held it selects the deepest node there. A click on no board clears the
    /// selection.
    pub fn click(&mut self, viewport: &Viewport, screen: (f32, f32), keys: Modifiers) {
        let target = self.target(viewport, screen, keys.ctrl || keys.meta);
        self.select(target);
    }

    /// A double-click at the screen point `screen` of `viewport`: a click,
    /// then the selected node becomes the context and its child under the
    /// pointer is selected. With no child there, the click's selection stays.
    pub fn double_click(&mut self, viewport: &Viewport, screen: (f32, f32)) {
        self.click(viewport, screen, Modifiers::default());
        let Some(selected) = self.selected else {
            return;
        };
        let child = self
            .index
            .node_at(viewport.to_design(screen))
            .and_then(|hit| self.index.child_of(hit, selected));
        if child.is_some() {
            self.select(child);
        }
    }

    /// The Escape key: selects the parent of the selection, which moves the
    /// context up one level. On a board it clears the selection.
    pub fn escape(&mut self) {
        if let Some(selected) = self.selected {
            self.select(self.index.parent(selected));
        }
    }

    /// The node the pointer is over, as a click would select it.
    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered.map(|at| self.index.id(at))
    }

    /// The selected node.
    pub fn selected(&self) -> Option<NodeId> {
        self.selected.map(|at| self.index.id(at))
    }

    /// The address of the selected node.
    pub fn selected_address(&self) -> Option<NodeAddress> {
        self.selected.map(|at| self.index.address(at))
    }

    /// The node whose children a click selects among: the parent of the
    /// selection, or the board when a board is selected. `None` while nothing
    /// is selected.
    pub fn context(&self) -> Option<NodeId> {
        let selected = self.selected?;
        Some(self.index.id(self.index.context(selected)))
    }

    /// The hover and selection changes since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The outlines of the hovered and the selected node in `viewport`, the
    /// selection last so that it is drawn on top. A node that is both is
    /// outlined as the selection.
    pub fn overlay(&self, viewport: &Viewport) -> Vec<Outline> {
        let outline = |at: usize, kind: OutlineKind| {
            let rect = self.index.rect(at);
            let (x, y) = viewport.to_screen((rect.x, rect.y));
            Outline {
                kind,
                node: self.index.address(at),
                label: self.index.name(at).to_string(),
                rect: LayoutBox {
                    x,
                    y,
                    width: rect.width * viewport.zoom(),
                    height: rect.height * viewport.zoom(),
                },
            }
        };
        let hovered = self
            .hovered
            .filter(|&at| Some(at) != self.selected)
            .map(|at| outline(at, OutlineKind::Hover));
        let selected = self.selected.map(|at| outline(at, OutlineKind::Selection));
        hovered.into_iter().chain(selected).collect()
    }

    /// Draws what `viewport` shows on `canvas`, whose unit is one CSS pixel
    /// (the host has scaled it by the device pixel ratio).
    ///
    /// The detail follows the device scale, the zoom times that ratio: below
    /// 50% a board is a snapshot, and otherwise it is drawn in full.
    pub fn render(&mut self, canvas: &Canvas, viewport: &Viewport) -> FrameStats {
        self.render_boards(canvas, viewport, None)
    }

    /// Draws only the board `board` of what `viewport` shows, as `render`
    /// does, so that its neighbours on the page stay out of the picture.
    pub fn render_board(
        &mut self,
        canvas: &Canvas,
        viewport: &Viewport,
        board: &NodeId,
    ) -> FrameStats {
        self.render_boards(canvas, viewport, Some(board))
    }

    fn render_boards(
        &mut self,
        canvas: &Canvas,
        viewport: &Viewport,
        only: Option<&NodeId>,
    ) -> FrameStats {
        let need = viewport.zoom() * pixel_ratio(canvas);
        let detail = if need < REDUCED_BELOW {
            Detail::Reduced
        } else {
            Detail::Full
        };
        let started = Instant::now();
        let adaptive = self.frame_budget.is_some();
        let spent =
            |budget: Option<Duration>| budget.is_some_and(|budget| started.elapsed() >= budget);
        let visible = viewport.visible_rect();
        let Scene {
            page,
            assets,
            boxes,
            texts,
            boards,
            frame_budget,
            ..
        } = self;
        let nodes = board_nodes(page);
        let mut stats = FrameStats {
            boards_drawn: Vec::new(),
            nodes_painted: 0,
            detail,
            deferred: false,
        };
        let (x, y) = viewport.origin();
        canvas.save();
        canvas.scale((viewport.zoom(), viewport.zoom()));
        canvas.translate((-x, -y));
        for board in boards
            .iter_mut()
            .filter(|board| only.is_none_or(|only| *only == board.id) && board.is_in(&visible))
        {
            stats.boards_drawn.push(board.id);
            if board.previewed {
                board.draw_reduced(canvas);
                continue;
            }
            let node = &nodes[board.index];
            match detail {
                Detail::Full => {
                    let fresh = board.picture.is_some() && !board.picture_stale;
                    // A board that has some drawing is left with it when the frame has no time for the new one.
                    let has_other = board.picture.is_some() || board.snapshot.is_some();
                    if fresh || (has_other && spent(*frame_budget)) {
                        stats.deferred |= !fresh;
                    } else {
                        stats.nodes_painted += board.record(node, boxes, texts, assets);
                    }
                    if board.picture.is_some() {
                        board.draw_full(canvas);
                    } else {
                        board.draw_reduced(canvas);
                    }
                }
                Detail::Reduced => {
                    let wanted = board.wanted_scale(need, adaptive);
                    if !board.snapshot_holds(wanted) {
                        if board.snapshot.is_some() && spent(*frame_budget) {
                            stats.deferred = true;
                        } else {
                            stats.nodes_painted += board.snapshot(node, boxes, assets, wanted);
                        }
                    }
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
