//! The player: a prototype's play state and the rules that change it.
//!
//! The rules follow FigCraft's `proto_run.rs` (MIT OR Apache-2.0; see
//! `NOTICE`): a click is a Down and an Up within a few pixels, the deepest
//! layer under the point with an interaction for the trigger wins, and every
//! action of an interaction runs in list order.

use crate::history::{History, Play};
use crate::overlay;
use omni_core::design::proto::{Action, Interaction, OverlayAnchor, OverlayPosition, Trigger};
use omni_core::design::{Node, NodeAddress, NodeData, NodeId, NodeKind, Page};
use std::collections::{BTreeMap, HashMap};

/// How far, in pixels, the pointer may travel between Down and Up for the two
/// to be a click.
const CLICK_SLOP: f64 = 4.0;

/// How deep the walk goes, so a component that holds itself ends.
const MAX_DEPTH: usize = 256;

/// A box in the same space as the pointer's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    fn offset(self, by: [f64; 2]) -> Rect {
        Rect {
            x: self.x + by[0],
            y: self.y + by[1],
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Down,
    Move,
    Up,
    Leave,
}

/// A pointer event; `at` is the time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub x: f64,
    pub y: f64,
    pub at: f64,
}

/// What the caller has to do after an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpenUrl {
        url: String,
        new_tab: bool,
    },
    /// The frame changed or is changing: draw it.
    NeedsFrame,
}

/// An overlay that is open.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayState {
    pub board: NodeId,
    /// Where the overlay sits, as an offset from the board's origin.
    pub origin: [f64; 2],
    /// A colour drawn over the screen under the overlay.
    pub background: Option<String>,
    pub close_on_outside: bool,
    /// When the overlay opened, in milliseconds: its delays count from here.
    pub opened_at: f64,
    pub position: OverlayPosition,
    pub anchor: OverlayAnchor,
    pub offset: Option<[f64; 2]>,
    /// The box of the layer that opened it, kept to place a swapped-in board.
    pub trigger_box: Rect,
}

/// A change of board that is playing.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub from: NodeId,
    pub to: NodeId,
    pub transition: omni_core::design::proto::Transition,
    /// Back plays the transition of the navigate it undoes, reversed.
    pub reversed: bool,
    pub started: f64,
    pub duration_ms: u32,
    pub easing: omni_core::design::proto::Easing,
}

/// What to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub board: NodeId,
    /// Open overlays, the top one last.
    pub overlays: Vec<OverlayState>,
    pub transition: Option<Running>,
    /// The component each swapped instance shows now, by instance.
    pub variant_swaps: BTreeMap<NodeId, NodeId>,
}

/// The boxes of one board's layers, as the caller laid them out.
#[derive(Default)]
struct Boxes {
    rects: HashMap<NodeId, Rect>,
    /// The layers inside instances: the instance and component node each
    /// stands for.
    layers: HashMap<NodeId, NodeAddress>,
}

struct Press {
    x: f64,
    y: f64,
    moved: bool,
}

/// A layer that reacts to touch.
struct Entry {
    id: NodeId,
    depth: usize,
    interactions: Vec<Interaction>,
    /// The nearest instance that holds the layer, or is the layer.
    holder: Option<NodeId>,
}

pub struct Player {
    boards: HashMap<NodeId, Node>,
    components: HashMap<NodeId, Node>,
    boxes: HashMap<NodeId, Boxes>,
    /// The component each instance shows, where the caller chose a variant.
    shown: HashMap<NodeId, NodeId>,
    frame: Frame,
    history: History,
    press: Option<Press>,
    /// The time of the last event the player saw.
    last_at: f64,
}

impl Player {
    /// A player on `start`. `components` are the nodes of the design system's
    /// components, by ID.
    pub fn new(page: &Page, components: HashMap<NodeId, Node>, start: NodeId) -> Player {
        let boards = page
            .children
            .iter()
            .filter(|node| node.kind() == NodeKind::Board)
            .map(|node| (*node.id(), node.clone()))
            .collect();
        Player {
            boards,
            components,
            boxes: HashMap::new(),
            shown: HashMap::new(),
            frame: Frame {
                board: start,
                overlays: Vec::new(),
                transition: None,
                variant_swaps: BTreeMap::new(),
            },
            history: History::default(),
            press: None,
            last_at: 0.0,
        }
    }

    /// Gives the boxes of a board's layers. `layers` says which instance and
    /// component node each layer inside an instance stands for.
    pub fn set_boxes(
        &mut self,
        board: NodeId,
        rects: HashMap<NodeId, Rect>,
        layers: HashMap<NodeId, NodeAddress>,
    ) {
        self.boxes.insert(board, Boxes { rects, layers });
    }

    /// Tells the player which component an instance shows, where its variant
    /// properties pick one other than the instance's own component.
    pub fn set_variant(&mut self, instance: NodeId, component: NodeId) {
        self.shown.insert(instance, component);
    }

    pub fn frame(&self) -> Frame {
        self.frame.clone()
    }

    /// The boards that navigate left, the newest last.
    pub fn history(&self) -> &[NodeId] {
        self.history.boards()
    }

    pub fn pointer(&mut self, event: PointerEvent) -> Vec<Effect> {
        self.last_at = event.at;
        let mut effects = Vec::new();
        let mut changed = false;
        match event.kind {
            PointerKind::Down => {
                self.press = Some(Press {
                    x: event.x,
                    y: event.y,
                    moved: false,
                });
            }
            PointerKind::Move => {
                if let Some(press) = &mut self.press {
                    if (event.x - press.x).hypot(event.y - press.y) > CLICK_SLOP {
                        press.moved = true;
                    }
                }
            }
            PointerKind::Leave => self.press = None,
            PointerKind::Up => {
                if let Some(press) = self.press.take() {
                    let near = (event.x - press.x).hypot(event.y - press.y) <= CLICK_SLOP;
                    if near && !press.moved {
                        changed = self.click(event.x, event.y, event.at, &mut effects);
                    }
                }
            }
        }
        if changed {
            effects.push(Effect::NeedsFrame);
        }
        effects
    }

    /// Moves time on: a finished transition ends. Returns `NeedsFrame` while a
    /// transition was running.
    pub fn tick(&mut self, now: f64) -> Vec<Effect> {
        self.last_at = now;
        let Some(running) = &self.frame.transition else {
            return Vec::new();
        };
        if now >= running.started + f64::from(running.duration_ms) {
            self.frame.transition = None;
        }
        vec![Effect::NeedsFrame]
    }

    /// Goes back to the board the last navigate left; nothing on the first
    /// board.
    pub fn back(&mut self) {
        self.go_back(self.last_at);
    }

    /// Goes to `board` as a navigate with the defaults does, pushing the board
    /// it leaves. Nothing, and `false`, if `board` is not a board of the page
    /// or is the board shown.
    pub fn go(&mut self, board: NodeId) -> bool {
        if board == self.frame.board || !self.boards.contains_key(&board) {
            return false;
        }
        let defaults = Interaction::default();
        let play = Play {
            transition: defaults.transition,
            duration_ms: defaults.duration_ms,
            easing: defaults.easing,
        };
        let from = self.frame.board;
        self.history.push(from, play);
        self.frame.board = board;
        self.frame.overlays.clear();
        self.frame.transition = Some(running(from, board, false, play, self.last_at));
        true
    }

    /// Starts over on `board`: no history, overlays or swaps.
    pub fn restart(&mut self, board: NodeId) {
        self.history.clear();
        self.frame = Frame {
            board,
            overlays: Vec::new(),
            transition: None,
            variant_swaps: BTreeMap::new(),
        };
        self.press = None;
    }

    /// The layers of the current board and of the open overlays that have an
    /// interaction, with the box the caller gave each.
    pub fn hotspots(&self, boxes: &HashMap<NodeId, Rect>) -> Vec<(NodeId, Rect)> {
        let mut spots = Vec::new();
        for (board, origin) in self.sources() {
            for entry in self.entries(&board) {
                if let Some(rect) = boxes.get(&entry.id) {
                    spots.push((entry.id, rect.offset(origin)));
                }
            }
        }
        spots
    }

    /// The boards a touch lands on, the top overlay first, with their origin.
    fn sources(&self) -> Vec<(NodeId, [f64; 2])> {
        match self.frame.overlays.last() {
            Some(top) => vec![(top.board, top.origin)],
            None => vec![(self.frame.board, [0.0, 0.0])],
        }
    }

    /// The box of a board's root in its own space.
    fn root_rect(&self, board: &NodeId) -> Option<Rect> {
        self.boxes.get(board)?.rects.get(board).copied()
    }

    /// Where an overlay of `board` goes, opened from `trigger`.
    fn place(
        &self,
        board: &NodeId,
        position: OverlayPosition,
        anchor: OverlayAnchor,
        offset: Option<[f64; 2]>,
        trigger: Rect,
    ) -> [f64; 2] {
        let screen = self.root_rect(&self.frame.board).unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        let size = self
            .root_rect(board)
            .map_or([0.0, 0.0], |rect| [rect.width, rect.height]);
        overlay::position(
            position,
            anchor,
            offset,
            trigger,
            size,
            Rect {
                x: 0.0,
                y: 0.0,
                ..screen
            },
        )
    }

    fn click(&mut self, x: f64, y: f64, at: f64, effects: &mut Vec<Effect>) -> bool {
        if let Some(top) = self.frame.overlays.last() {
            let inside = self
                .root_rect(&top.board)
                .is_some_and(|rect| rect.offset(top.origin).contains(x, y));
            if !inside {
                if top.close_on_outside {
                    self.frame.overlays.pop();
                    return true;
                }
                return false;
            }
        }
        let Some((interactions, holder, trigger)) = self.hit(&Trigger::OnClick, x, y) else {
            return false;
        };
        let mut changed = false;
        for interaction in &interactions {
            let play = Play {
                transition: interaction.transition,
                duration_ms: interaction.duration_ms,
                easing: interaction.easing,
            };
            for action in &interaction.actions {
                changed |= self.apply(action, play, holder, trigger, at, effects);
            }
        }
        changed
    }

    /// The interactions for `trigger` of the deepest layer under the point
    /// that has any, and the instance that holds the layer. A later layer
    /// wins over an earlier one of the same depth, as it is drawn on top.
    fn hit(
        &self,
        trigger: &Trigger,
        x: f64,
        y: f64,
    ) -> Option<(Vec<Interaction>, Option<NodeId>, Rect)> {
        for (board, origin) in self.sources() {
            let Some(boxes) = self.boxes.get(&board) else {
                continue;
            };
            let mut best: Option<(usize, Vec<Interaction>, Option<NodeId>, Rect)> = None;
            for entry in self.entries(&board) {
                let Some(rect) = boxes.rects.get(&entry.id) else {
                    continue;
                };
                if !rect.offset(origin).contains(x, y) {
                    continue;
                }
                let matching: Vec<Interaction> = entry
                    .interactions
                    .into_iter()
                    .filter(|interaction| interaction.trigger == *trigger)
                    .collect();
                if matching.is_empty() || best.as_ref().is_some_and(|b| b.0 > entry.depth) {
                    continue;
                }
                best = Some((entry.depth, matching, entry.holder, rect.offset(origin)));
            }
            if let Some((_, interactions, holder, rect)) = best {
                return Some((interactions, holder, rect));
            }
        }
        None
    }

    fn apply(
        &mut self,
        action: &Action,
        play: Play,
        holder: Option<NodeId>,
        trigger: Rect,
        at: f64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        match action {
            Action::Navigate { dest, .. } => {
                let Some(to) = self.board_id(dest) else {
                    return false;
                };
                let from = self.frame.board;
                self.history.push(from, play);
                self.frame.board = to;
                self.frame.overlays.clear();
                self.frame.transition = Some(running(from, to, false, play, at));
                true
            }
            Action::Back => self.go_back(at),
            Action::OpenOverlay {
                dest,
                position,
                offset,
                anchor,
                close_on_outside,
                background,
            } => {
                let Some(board) = self.board_id(dest) else {
                    return false;
                };
                let origin = self.place(&board, *position, *anchor, *offset, trigger);
                self.frame.overlays.push(OverlayState {
                    board,
                    origin,
                    background: background.clone(),
                    close_on_outside: *close_on_outside,
                    opened_at: at,
                    position: *position,
                    anchor: *anchor,
                    offset: *offset,
                    trigger_box: trigger,
                });
                true
            }
            Action::SwapOverlay { dest } => {
                let Some(board) = self.board_id(dest) else {
                    return false;
                };
                let Some(top) = self.frame.overlays.last() else {
                    return false;
                };
                let origin = self.place(
                    &board,
                    top.position,
                    top.anchor,
                    top.offset,
                    top.trigger_box,
                );
                let top = self.frame.overlays.last_mut().expect("checked above");
                top.board = board;
                top.origin = origin;
                top.opened_at = at;
                true
            }
            Action::CloseOverlay => self.frame.overlays.pop().is_some(),
            Action::OpenUrl { url, new_tab } => {
                effects.push(Effect::OpenUrl {
                    url: url.clone(),
                    new_tab: *new_tab,
                });
                false
            }
            Action::ChangeTo { dest } => {
                let (Some(instance), Ok(component)) = (holder, dest.parse::<NodeId>()) else {
                    return false;
                };
                if !self.components.contains_key(&component) {
                    return false;
                }
                self.frame.variant_swaps.insert(instance, component);
                true
            }
            // Scrolling, variables and conditions come in later stories.
            _ => false,
        }
    }

    fn go_back(&mut self, at: f64) -> bool {
        let Some((to, play)) = self.history.pop() else {
            return false;
        };
        let from = self.frame.board;
        self.frame.board = to;
        self.frame.overlays.clear();
        self.frame.transition = Some(running(from, to, true, play, at));
        true
    }

    fn board_id(&self, dest: &str) -> Option<NodeId> {
        let id = dest.parse::<NodeId>().ok()?;
        self.boards.contains_key(&id).then_some(id)
    }

    /// The layers of a board that have interactions, in drawing order.
    fn entries(&self, board: &NodeId) -> Vec<Entry> {
        let Some(root) = self.boards.get(board) else {
            return Vec::new();
        };
        let inverse = self
            .boxes
            .get(board)
            .map(|boxes| {
                boxes
                    .layers
                    .iter()
                    .filter_map(|(id, address)| {
                        Some(((*address.instance()?, *address.node()), *id))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut entries = Vec::new();
        self.walk(root, 0, None, None, &inverse, &mut entries);
        entries
    }

    /// `outer` is the instance of the page that the node is inside, which
    /// addresses it; `holder` is the nearest instance around it.
    fn walk(
        &self,
        node: &Node,
        depth: usize,
        outer: Option<NodeId>,
        holder: Option<NodeId>,
        inverse: &HashMap<(NodeId, NodeId), NodeId>,
        out: &mut Vec<Entry>,
    ) {
        if !node.visible() || depth > MAX_DEPTH {
            return;
        }
        let id = match outer {
            None => Some(*node.id()),
            Some(outer) => inverse.get(&(outer, *node.id())).copied(),
        };
        if let NodeData::Instance(instance) = node.data() {
            let shown = self
                .frame
                .variant_swaps
                .get(node.id())
                .or_else(|| self.shown.get(node.id()))
                .unwrap_or(&instance.component);
            let component = self.components.get(shown);
            // An instance's own interaction wins over its variant's for the
            // same trigger.
            let mut interactions = node.interactions().to_vec();
            if let Some(component) = component {
                interactions.extend(
                    component
                        .interactions()
                        .iter()
                        .filter(|inherited| {
                            !node
                                .interactions()
                                .iter()
                                .any(|own| own.trigger == inherited.trigger)
                        })
                        .cloned(),
                );
            }
            if let (Some(id), false) = (id, interactions.is_empty()) {
                out.push(Entry {
                    id,
                    depth,
                    interactions,
                    holder: Some(*node.id()),
                });
            }
            if let Some(NodeData::Component(component)) = component.map(Node::data) {
                let outer = outer.unwrap_or(*node.id());
                for child in &component.frame.children {
                    self.walk(
                        child,
                        depth + 1,
                        Some(outer),
                        Some(*node.id()),
                        inverse,
                        out,
                    );
                }
            }
            return;
        }
        if let (Some(id), false) = (id, node.interactions().is_empty()) {
            out.push(Entry {
                id,
                depth,
                interactions: node.interactions().to_vec(),
                holder,
            });
        }
        for child in node.children() {
            self.walk(child, depth + 1, outer, holder, inverse, out);
        }
    }
}

fn running(from: NodeId, to: NodeId, reversed: bool, play: Play, started: f64) -> Running {
    Running {
        from,
        to,
        transition: play.transition,
        reversed,
        started,
        duration_ms: play.duration_ms,
        easing: play.easing,
    }
}
