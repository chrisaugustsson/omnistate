//! The player: a prototype's play state and the rules that change it.
//!
//! The rules follow FigCraft's `proto_run.rs` (MIT OR Apache-2.0; see
//! `NOTICE`): a click is a Down and an Up within a few pixels, the deepest
//! layer under the point with an interaction for the trigger wins, and every
//! action of an interaction runs in list order. A hover or a press puts
//! back what it did when it ends; a drag that starts a directional
//! transition lets the pointer carry it.

use crate::history::{History, Play};
use crate::input::{DragTarget, Follow, Holder, Hover, Press, Revert, Site};
use crate::keys::{self, Mods};
use crate::overlay;
use crate::timers::{Target, Timer};
use crate::transition::{duration_ms, progress, reversed};
use crate::variables::{self, PlayVariables, TokenValue};
use omni_core::design::proto::{
    Action, Direction, Interaction, OverlayAnchor, OverlayPosition, Trigger,
};
use omni_core::design::tokens::{ModeChoice, TokenTree};
use omni_core::design::{
    Node, NodeAddress, NodeData, NodeId, NodeKind, Page, PropDef, PropDefs, TokenOr,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// How far, in screen pixels, the pointer may travel between Down and Up for
/// the two to be a click, and how far it goes before a drag starts.
const CLICK_SLOP: f64 = 4.0;

/// How deep the walk goes, so a component that holds itself ends.
const MAX_DEPTH: usize = 256;

/// How long the hotspot outlines of a miss fade for, in milliseconds.
pub const HINT_MS: f64 = 1000.0;

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
    /// A click landed on nothing that reacts: the hotspots are outlined.
    Missed,
    /// The frame changed or is changing: draw it.
    NeedsFrame,
    /// A key or controller button fired a trigger, so it is not the caller's
    /// to use for anything else.
    KeyHandled,
}

/// A key going down or up. `key` is the name the trigger writes it with
/// (`K`, `Enter`, `ArrowLeft`, `Space`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub down: bool,
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
    /// Which showing of an overlay this is, for the waits that end with it.
    pub visit: u64,
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
    /// Whether the pointer, which a drag started it with, sets how far it has
    /// come until the button comes up.
    pub follows_pointer: bool,
    /// The transition this one cut short: its frame, as it was when this one
    /// started, is what this one changes from.
    pub interrupted: Option<Box<Interrupted>>,
}

/// A transition that a new one cut short, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Interrupted {
    pub running: Running,
    pub at: f64,
}

/// What to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub board: NodeId,
    /// Open overlays, the top one last.
    pub overlays: Vec<OverlayState>,
    pub transition: Option<Running>,
    /// The component each swapped instance of a board shows now, by the
    /// instance's node ID.
    pub variant_swaps: BTreeMap<NodeId, NodeId>,
    /// The same for an instance inside another instance, by its address: the
    /// instance of the board and the node inside its component.
    pub nested_swaps: HashMap<NodeAddress, NodeId>,
    /// What the session has set: token values and the modes of boards.
    pub variables: PlayVariables,
}

/// The boxes of one board's layers, as the caller laid them out.
#[derive(Default)]
struct Boxes {
    rects: HashMap<NodeId, Rect>,
    /// The layers inside instances: the instance and component node each
    /// stands for.
    layers: HashMap<NodeId, NodeAddress>,
}

/// The layer under a point that has the interactions of a trigger.
struct Found {
    id: NodeId,
    depth: usize,
    interactions: Vec<Interaction>,
    site: Site,
    rect: Rect,
}

/// A layer that has interactions and is under the pointer, with its box on
/// the screen.
struct Under {
    entry: Entry,
    rect: Rect,
}

/// The deepest of `under` with an interaction whose trigger `matches`. A
/// later layer wins over an earlier one of the same depth, as it is drawn on
/// top.
fn deepest(under: &[Under], matches: impl Fn(&Trigger) -> bool) -> Option<Found> {
    let mut best: Option<Found> = None;
    for layer in under {
        let interactions: Vec<Interaction> = layer
            .entry
            .interactions
            .iter()
            .filter(|interaction| matches(&interaction.trigger))
            .cloned()
            .collect();
        if interactions.is_empty() || best.as_ref().is_some_and(|b| b.depth > layer.entry.depth) {
            continue;
        }
        best = Some(Found {
            id: layer.entry.id,
            depth: layer.entry.depth,
            interactions,
            site: layer.entry.site.clone(),
            rect: layer.rect,
        });
    }
    best
}

/// The swaps of one board's visit.
#[derive(Default)]
struct BoardSwaps {
    top: BTreeMap<NodeId, NodeId>,
    nested: HashMap<NodeAddress, NodeId>,
}

/// A layer that reacts to touch.
struct Entry {
    id: NodeId,
    depth: usize,
    interactions: Vec<Interaction>,
    /// Its board and the instances that hold the layer, or are the layer.
    site: Site,
}

pub struct Player {
    boards: HashMap<NodeId, Node>,
    components: HashMap<NodeId, Node>,
    /// The design system's tokens, which expressions read and actions set.
    tokens: Option<TokenTree>,
    boxes: HashMap<NodeId, Boxes>,
    /// The component each instance shows, where the caller chose a variant.
    shown: HashMap<NodeId, NodeId>,
    frame: Frame,
    /// The swaps made while each board was shown or open, by board: they
    /// last as long as the visit. `frame.variant_swaps` holds them all.
    swaps: HashMap<NodeId, BoardSwaps>,
    history: History,
    press: Option<Press>,
    /// The layers the pointer is over.
    hovered: Vec<Hover>,
    timers: Vec<Timer>,
    /// How many boards and overlays have been shown, to tell the showings
    /// apart.
    visits: u64,
    /// Which showing the board shown is.
    board_visit: u64,
    /// The CSS pixels of the screen per unit of the pointer's coordinates.
    scale: f64,
    /// What a transition let go by a drag has already come, until the page
    /// times it from its own clock.
    resume_ms: Option<f64>,
    /// What the actions run now would put back, where a hover or press
    /// collects it.
    recording: Option<Vec<Revert>>,
    /// The time of the last event the player saw.
    last_at: f64,
    /// Until when the hotspots are outlined after a click that missed.
    hint_until: Option<f64>,
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
        let mut player = Player {
            boards,
            components,
            tokens: None,
            boxes: HashMap::new(),
            shown: HashMap::new(),
            frame: Frame {
                board: start,
                overlays: Vec::new(),
                transition: None,
                variant_swaps: BTreeMap::new(),
                nested_swaps: HashMap::new(),
                variables: PlayVariables::default(),
            },
            swaps: HashMap::new(),
            history: History::default(),
            press: None,
            hovered: Vec::new(),
            timers: Vec::new(),
            visits: 1,
            board_visit: 1,
            scale: 1.0,
            resume_ms: None,
            recording: None,
            last_at: 0.0,
            hint_until: None,
        };
        player.schedule(start, 1, [0.0, 0.0], 0.0);
        player
    }

    /// Tells the player how many CSS pixels of the screen one unit of the
    /// pointer's coordinates is, which the click and drag distances are
    /// measured in.
    pub fn set_scale(&mut self, scale: f64) {
        if scale > 0.0 {
            self.scale = scale;
        }
    }

    /// The waits that are running: an `afterDelay` of the board or overlay
    /// shown, and the `delayMs` of a gesture. Each runs out at `Timer::due`
    /// and acts in `tick`.
    pub fn timers(&self) -> Vec<&Timer> {
        self.timers
            .iter()
            .filter(|timer| self.current(timer.visit))
            .collect()
    }

    /// Counts the waits that began at an event again from `at`, for a page
    /// whose clock was stale when they began.
    pub fn restart_timers(&mut self, at: f64) {
        for timer in self.timers.iter_mut().filter(|timer| timer.fresh) {
            timer.restart(at);
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

    /// Gives the design system's tokens: without them no variable is set and
    /// no expression is read.
    pub fn set_tokens(&mut self, tokens: TokenTree) {
        self.tokens = Some(tokens);
    }

    /// What the session has set so far.
    pub fn variables(&self) -> &PlayVariables {
        &self.frame.variables
    }

    pub fn frame(&self) -> Frame {
        self.frame.clone()
    }

    /// The swaps of `board`'s visit, by instance: a board that is not shown
    /// or open has none. An instance inside another instance is in
    /// `nested_swaps_for`.
    pub fn swaps_for(&self, board: NodeId) -> &BTreeMap<NodeId, NodeId> {
        static NONE: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        self.swaps.get(&board).map_or(&NONE, |swaps| &swaps.top)
    }

    /// The swaps of `board`'s visit of instances inside instances, by the
    /// instance of the board and the node inside its component.
    pub fn nested_swaps_for(&self, board: NodeId) -> &HashMap<NodeAddress, NodeId> {
        static NONE: std::sync::LazyLock<HashMap<NodeAddress, NodeId>> =
            std::sync::LazyLock::new(HashMap::new);
        self.swaps.get(&board).map_or(&NONE, |swaps| &swaps.nested)
    }

    /// The boards that navigate left, the newest last.
    pub fn history(&self) -> &[NodeId] {
        self.history.boards()
    }

    pub fn pointer(&mut self, event: PointerEvent) -> Vec<Effect> {
        self.last_at = event.at;
        let mut effects = Vec::new();
        let changed = match event.kind {
            PointerKind::Down => self.down(event, &mut effects),
            PointerKind::Move => self.moved(event, &mut effects),
            PointerKind::Up => self.up(event, &mut effects),
            PointerKind::Leave => self.leave(event.at, &mut effects),
        };
        self.prune();
        if changed {
            effects.push(Effect::NeedsFrame);
        }
        effects
    }

    fn slop(&self) -> f64 {
        CLICK_SLOP / self.scale
    }

    fn down(&mut self, event: PointerEvent, effects: &mut Vec<Effect>) -> bool {
        let (x, y, at) = (event.x, event.y, event.at);
        // A button that never came up ends here.
        let mut changed = self.release(at, effects);
        changed |= self.hover_to(x, y, at, effects);
        let under = self.under(x, y);
        self.press = Some(Press {
            x,
            y,
            moved: false,
            layer: deepest(&under, |trigger| *trigger == Trigger::OnClick).map(|found| found.id),
            visit: self.source_visit(),
            drag: deepest(&under, |trigger| *trigger == Trigger::OnDrag).map(|found| DragTarget {
                interactions: found.interactions,
                site: found.site,
                rect: found.rect,
            }),
            dragged: false,
            follow: None,
            reverts: Vec::new(),
        });
        if let Some(found) = deepest(&under, |trigger| *trigger == Trigger::MouseDown) {
            changed |= self.fire(
                &found.interactions,
                &found.site,
                found.rect,
                true,
                at,
                effects,
            );
        }
        if let Some(found) = deepest(&under, |trigger| *trigger == Trigger::OnPress) {
            let (pressed, reverts) = self.recording(|player| {
                player.fire(
                    &found.interactions,
                    &found.site,
                    found.rect,
                    false,
                    at,
                    effects,
                )
            });
            changed |= pressed;
            if let Some(press) = &mut self.press {
                press.reverts = reverts;
            }
        }
        changed
    }

    fn moved(&mut self, event: PointerEvent, effects: &mut Vec<Effect>) -> bool {
        let (x, y, at) = (event.x, event.y, event.at);
        let mut changed = self.hover_to(x, y, at, effects);
        changed |= self.drag_to(x, y, at, effects);
        changed
    }

    fn up(&mut self, event: PointerEvent, effects: &mut Vec<Effect>) -> bool {
        let (x, y, at) = (event.x, event.y, event.at);
        let Some(press) = self.press.take() else {
            return false;
        };
        let near = (x - press.x).hypot(y - press.y) <= self.slop() && !press.moved;
        let mut changed = false;
        // Mouse up acts for the layer under the pointer at release.
        let under = self.under(x, y);
        if let Some(found) = deepest(&under, |trigger| *trigger == Trigger::MouseUp) {
            changed |= self.fire(
                &found.interactions,
                &found.site,
                found.rect,
                true,
                at,
                effects,
            );
        }
        changed |= self.finish(press.follow, press.reverts, at, effects);
        // A click is a press and a release on the same layer, of the same
        // showing, that went nowhere between.
        if near && self.source_visit() == press.visit {
            changed |= self.click(x, y, at, press.layer, effects);
        }
        // What the pointer is over on the board the press left behind.
        changed |= self.hover_to(x, y, at, effects);
        changed
    }

    /// The pointer left the play area: nothing is hovered, and a button that
    /// is down comes up.
    fn leave(&mut self, at: f64, effects: &mut Vec<Effect>) -> bool {
        let mut changed = self.hover_to(f64::NAN, f64::NAN, at, effects);
        changed |= self.release(at, effects);
        changed
    }

    /// Ends a button that is down without a click.
    fn release(&mut self, at: f64, effects: &mut Vec<Effect>) -> bool {
        match self.press.take() {
            Some(press) => self.finish(press.follow, press.reverts, at, effects),
            None => false,
        }
    }

    /// What the end of a press does: a transition the pointer carried plays
    /// on from where it was left, and what the press did is put back.
    fn finish(
        &mut self,
        follow: Option<Follow>,
        reverts: Vec<Revert>,
        at: f64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let mut changed = false;
        if let (Some(follow), Some(running)) = (follow, self.frame.transition.as_mut()) {
            if running.follows_pointer {
                running.follows_pointer = false;
                let done = follow.fraction * duration_ms(running);
                running.started = at - done;
                self.resume_ms = Some(done);
                changed = true;
            }
        }
        let before = self.board_visit;
        changed |= self.revert(reverts, at);
        if self.board_visit != before {
            changed |= self.drop_stale(vec![before], at, effects);
        }
        changed
    }

    /// Updates what the pointer is over for a pointer at `x`, `y`, which is
    /// nowhere when it is NaN. The layers it left revert and run their mouse
    /// leave, then the layers it entered run their mouse enter and hover,
    /// each innermost first.
    fn hover_to(&mut self, x: f64, y: f64, at: f64, effects: &mut Vec<Effect>) -> bool {
        let (kept, mut left): (Vec<Hover>, Vec<Hover>) = std::mem::take(&mut self.hovered)
            .into_iter()
            .partition(|hover| hover.rect.contains(x, y));
        self.hovered = kept;
        left.sort_by(|a, b| b.depth.cmp(&a.depth));
        let mut changed = false;
        let mut gone = Vec::new();
        for hover in left {
            let before = self.board_visit;
            changed |= self.revert(hover.reverts, at);
            changed |= self.fire_on(
                hover.source_visit,
                &hover.leave,
                &hover.site,
                hover.rect,
                true,
                at,
                effects,
            );
            if self.board_visit != before {
                gone.push(before);
            }
        }
        changed |= self.drop_stale(gone, at, effects);
        let under = if x.is_nan() {
            Vec::new()
        } else {
            self.under(x, y)
        };
        let mut entered: Vec<&Under> = under
            .iter()
            .filter(|layer| !self.hovered.iter().any(|hover| hover.id == layer.entry.id))
            .collect();
        entered.sort_by(|a, b| b.entry.depth.cmp(&a.entry.depth));
        // The layers were found on this showing, whatever their actions show.
        let visit = self.board_visit;
        // `under` hit the top overlay when one is open, else the board.
        let source = self.source_visit();
        for layer in entered {
            let with = |wanted: Trigger| -> Vec<Interaction> {
                layer
                    .entry
                    .interactions
                    .iter()
                    .filter(|interaction| interaction.trigger == wanted)
                    .cloned()
                    .collect()
            };
            let (enter, hover, leave) = (
                with(Trigger::MouseEnter),
                with(Trigger::OnHover),
                with(Trigger::MouseLeave),
            );
            if enter.is_empty() && hover.is_empty() && leave.is_empty() {
                continue;
            }
            let (site, rect) = (layer.entry.site.clone(), layer.rect);
            changed |= self.fire_on(source, &enter, &site, rect, true, at, effects);
            let (hovered, reverts) =
                self.recording(|player| player.fire(&hover, &site, rect, false, at, effects));
            changed |= hovered;
            self.hovered.push(Hover {
                id: layer.entry.id,
                depth: layer.entry.depth,
                rect,
                site,
                leave,
                reverts,
                board_visit: visit,
                source_visit: source,
            });
        }
        changed
    }

    /// Reverts and drops the hovers entered on boards a revert left, as if
    /// the pointer had left them.
    fn drop_stale(&mut self, mut gone: Vec<u64>, at: f64, effects: &mut Vec<Effect>) -> bool {
        let mut changed = false;
        while let Some(visit) = gone.pop() {
            let (stale, kept): (Vec<Hover>, Vec<Hover>) = std::mem::take(&mut self.hovered)
                .into_iter()
                .partition(|hover| hover.board_visit == visit);
            self.hovered = kept;
            for hover in stale {
                let before = self.board_visit;
                changed |= self.revert(hover.reverts, at);
                changed |= self.fire_on(
                    hover.source_visit,
                    &hover.leave,
                    &hover.site,
                    hover.rect,
                    true,
                    at,
                    effects,
                );
                if self.board_visit != before {
                    gone.push(before);
                }
            }
        }
        changed
    }

    /// Starts the drag of the layer the button went down on once the pointer
    /// has gone far enough in a direction its transition takes, and lets a
    /// transition it started follow the pointer.
    fn drag_to(&mut self, x: f64, y: f64, at: f64, effects: &mut Vec<Effect>) -> bool {
        let slop = self.slop();
        let Some(press) = self.press.as_mut() else {
            return false;
        };
        let (dx, dy) = (x - press.x, y - press.y);
        if dx.hypot(dy) > slop {
            press.moved = true;
        }
        if press.follow.is_some() {
            return self.follow_to(dx, dy, at);
        }
        if press.dragged || !press.moved {
            return false;
        }
        let Some(target) = press.drag.clone() else {
            return false;
        };
        let Some(interaction) = target
            .interactions
            .iter()
            .find(|interaction| interaction.transition.accepts_drag(dx, dy))
            .cloned()
        else {
            return false;
        };
        press.dragged = true;
        let before = self.frame.transition.clone();
        let mut changed = self.run(&interaction, &target.site, target.rect, at, effects);
        let direction = interaction.transition.direction();
        if let (Some(direction), true) = (direction, self.frame.transition != before) {
            if let Some(running) = &mut self.frame.transition {
                running.follows_pointer = true;
                if let Some(press) = &mut self.press {
                    press.follow = Some(Follow {
                        direction,
                        fraction: 0.0,
                    });
                }
                changed |= self.follow_to(dx, dy, at);
            }
        }
        changed
    }

    /// Sets how far the transition the pointer carries has come, from how far
    /// along its direction the pointer has gone, as a part of the board's size
    /// on that axis.
    fn follow_to(&mut self, dx: f64, dy: f64, at: f64) -> bool {
        let Some(follow) = self.press.as_ref().and_then(|press| press.follow) else {
            return false;
        };
        let Some(running) = self.frame.transition.as_ref().filter(|r| r.follows_pointer) else {
            if let Some(press) = &mut self.press {
                press.follow = None;
            }
            return false;
        };
        let size = self
            .root_rect(&running.to)
            .map_or([1.0, 1.0], |rect| [rect.width, rect.height]);
        let (along, extent) = match follow.direction {
            Direction::Left => (-dx, size[0]),
            Direction::Right => (dx, size[0]),
            Direction::Top => (-dy, size[1]),
            Direction::Bottom => (dy, size[1]),
        };
        let fraction = (along / extent.max(1.0)).clamp(0.0, 1.0);
        if let Some(press) = &mut self.press {
            press.follow = Some(Follow { fraction, ..follow });
        }
        self.pin(at);
        true
    }

    /// Times the transition the pointer carries so that it stands where the
    /// pointer has put it at `now`.
    fn pin(&mut self, now: f64) {
        let fraction = self
            .press
            .as_ref()
            .and_then(|press| press.follow)
            .map(|follow| follow.fraction);
        if let Some(running) = &mut self.frame.transition {
            if running.follows_pointer {
                match fraction {
                    Some(fraction) => running.started = now - fraction * duration_ms(running),
                    None => running.follows_pointer = false,
                }
            }
        }
    }

    /// Runs `body`, and gives what its actions would put back.
    fn recording<R>(&mut self, body: impl FnOnce(&mut Self) -> R) -> (R, Vec<Revert>) {
        let outer = self.recording.replace(Vec::new());
        let result = body(self);
        let reverts = std::mem::replace(&mut self.recording, outer).unwrap_or_default();
        (result, reverts)
    }

    fn note(&mut self, revert: Revert) {
        if let Some(reverts) = &mut self.recording {
            reverts.push(revert);
        }
    }

    /// Puts back what a hover or press did, the last thing first. A navigate
    /// is only undone while its board is shown, and is not a history step.
    fn revert(&mut self, reverts: Vec<Revert>, at: f64) -> bool {
        let mut changed = false;
        for revert in reverts.into_iter().rev() {
            match revert {
                Revert::Board {
                    to,
                    from,
                    visit,
                    history_len,
                    play,
                } => {
                    if self.frame.board != to {
                        continue;
                    }
                    self.history.truncate(history_len);
                    let running = self.running(to, from, true, play, at);
                    self.end_visit();
                    self.frame.board = from;
                    self.board_visit = visit;
                    self.frame.transition = Some(running);
                    changed = true;
                }
                Revert::Variant {
                    board,
                    instance,
                    previous,
                } => {
                    // The visit it was made in is over otherwise.
                    if self.frame.board == board
                        || self.frame.overlays.iter().any(|o| o.board == board)
                    {
                        self.set_swap(board, instance, previous);
                        changed = true;
                    }
                }
                Revert::Overlay { visit } => {
                    if let Some(open) = self.frame.overlays.iter().position(|o| o.visit == visit) {
                        let closed = self.frame.overlays.remove(open);
                        self.forget(closed.board);
                        changed = true;
                    }
                }
                Revert::Swap { visit, before } => {
                    if let Some(top) = self.frame.overlays.iter_mut().find(|o| o.visit == visit) {
                        let swapped_in = top.board;
                        *top = before;
                        self.forget(swapped_in);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Runs `interactions` for a gesture. A mouse trigger (`delayable`) with a
    /// `delayMs` waits that long first, once.
    fn fire(
        &mut self,
        interactions: &[Interaction],
        site: &Site,
        rect: Rect,
        delayable: bool,
        at: f64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let visit = self.source_visit();
        self.fire_on(visit, interactions, site, rect, delayable, at, effects)
    }

    /// `fire` for waits that belong to the showing `visit`.
    #[allow(clippy::too_many_arguments)]
    fn fire_on(
        &mut self,
        visit: u64,
        interactions: &[Interaction],
        site: &Site,
        rect: Rect,
        delayable: bool,
        at: f64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let mut changed = false;
        for interaction in interactions {
            if delayable && interaction.delay_ms > 0 {
                self.timers.push(Timer::new(
                    visit,
                    interaction.clone(),
                    site.clone(),
                    Target::Rect(rect),
                    at,
                    interaction.delay_ms,
                ));
            } else {
                changed |= self.run(interaction, site, rect, at, effects);
            }
        }
        changed
    }

    /// Runs the actions of an interaction in list order, with its transition.
    fn run(
        &mut self,
        interaction: &Interaction,
        site: &Site,
        trigger: Rect,
        at: f64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let play = Play {
            transition: interaction.transition,
            duration_ms: interaction.duration_ms,
            easing: interaction.easing,
        };
        let mut changed = false;
        for action in &interaction.actions {
            changed |= self.apply(action, play, site, trigger, at, effects);
        }
        changed
    }

    /// Which showing the board or overlay a touch lands on is.
    fn source_visit(&self) -> u64 {
        self.frame
            .overlays
            .last()
            .map_or(self.board_visit, |top| top.visit)
    }

    /// Whether `visit` is a showing that is on the screen.
    fn current(&self, visit: u64) -> bool {
        visit == self.board_visit || self.frame.overlays.iter().any(|o| o.visit == visit)
    }

    fn next_visit(&mut self) -> u64 {
        self.visits += 1;
        self.visits
    }

    /// Drops the waits of showings that are gone, except those a hover or a
    /// press would bring back.
    fn prune(&mut self) {
        let mut held: Vec<u64> = Vec::new();
        let reverts = self
            .hovered
            .iter()
            .flat_map(|hover| hover.reverts.iter())
            .chain(self.press.iter().flat_map(|press| press.reverts.iter()));
        for revert in reverts {
            match revert {
                Revert::Board { visit, .. } => held.push(*visit),
                Revert::Swap { before, .. } => held.push(before.visit),
                _ => {}
            }
        }
        let keep: Vec<bool> = self
            .timers
            .iter()
            .map(|timer| self.current(timer.visit) || held.contains(&timer.visit))
            .collect();
        let mut keep = keep.into_iter();
        self.timers.retain(|_| keep.next().unwrap_or(false));
    }

    /// Starts the `afterDelay` waits of the layers of `board`, shown at `at`
    /// as showing `visit` at `origin` on the screen.
    fn schedule(&mut self, board: NodeId, visit: u64, origin: [f64; 2], at: f64) {
        for entry in self.entries(&board) {
            for interaction in &entry.interactions {
                if let Trigger::AfterDelay { ms } = interaction.trigger {
                    self.timers.push(Timer::new(
                        visit,
                        interaction.clone(),
                        entry.site.clone(),
                        Target::Layer {
                            board,
                            id: entry.id,
                            origin,
                        },
                        at,
                        ms,
                    ));
                }
            }
        }
    }

    /// Shows `board` as a new showing from `at`, as a navigate, back or a
    /// start over does.
    fn show(&mut self, board: NodeId, at: f64) {
        self.end_visit();
        self.frame.board = board;
        self.board_visit = self.next_visit();
        // Layers entered on the board left are not on this one.
        self.hovered.retain(|hover| !hover.reverts.is_empty());
        self.schedule(board, self.board_visit, [0.0, 0.0], at);
    }

    /// Runs the waits that have run out, the earliest first. A wait whose
    /// board or overlay went away while an earlier one ran is dropped.
    fn run_due(&mut self, now: f64, effects: &mut Vec<Effect>) -> bool {
        self.prune();
        let mut due: Vec<Timer> = Vec::new();
        let mut rest = Vec::new();
        for timer in std::mem::take(&mut self.timers) {
            if timer.due() <= now && self.current(timer.visit) {
                due.push(timer);
            } else {
                rest.push(timer);
            }
        }
        self.timers = rest;
        due.sort_by(|a, b| a.due().total_cmp(&b.due()));
        let mut changed = false;
        for timer in due {
            if !self.current(timer.visit) {
                continue;
            }
            let rect = match &timer.target {
                Target::Rect(rect) => *rect,
                Target::Layer { board, id, origin } => self
                    .boxes
                    .get(board)
                    .and_then(|boxes| boxes.rects.get(id))
                    .map_or(
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 0.0,
                            height: 0.0,
                        },
                        |rect| rect.offset(*origin),
                    ),
            };
            changed |= self.run(&timer.interaction, &timer.site, rect, now, effects);
        }
        self.prune();
        changed
    }

    /// Moves time on: waits that ran out act, and a finished transition
    /// ends. Returns `NeedsFrame` while a transition was running or an
    /// action ran.
    pub fn tick(&mut self, now: f64) -> Vec<Effect> {
        self.last_at = now;
        if self.hint_until.is_some_and(|until| now >= until) {
            self.hint_until = None;
        }
        self.pin(now);
        let moving = self.frame.transition.is_some();
        let mut effects = Vec::new();
        let changed = self.run_due(now, &mut effects);
        if let Some(running) = &self.frame.transition {
            if !running.follows_pointer && progress(running, now).done {
                self.frame.transition = None;
            }
        }
        if moving || changed {
            effects.push(Effect::NeedsFrame);
        }
        effects
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
        self.show(board, self.last_at);
        self.frame.transition = Some(self.running(from, board, false, play, self.last_at));
        true
    }

    /// Starts over on `board`: no history, overlays or swaps.
    pub fn restart(&mut self, board: NodeId) {
        self.history.clear();
        self.swaps.clear();
        self.frame = Frame {
            board,
            overlays: Vec::new(),
            transition: None,
            variant_swaps: BTreeMap::new(),
            nested_swaps: HashMap::new(),
            variables: PlayVariables::default(),
        };
        self.press = None;
        self.hovered.clear();
        self.timers.clear();
        self.resume_ms = None;
        self.show(board, self.last_at);
        self.hint_until = None;
    }

    /// Until when the hotspots are outlined, in milliseconds, if they are.
    pub fn hint_until(&self) -> Option<f64> {
        self.hint_until
    }

    /// Starts the transition that is playing over again at `at`, with the one
    /// it cut short frozen at `at` too, for a page whose clock was stale when
    /// the transition began.
    pub fn restart_transition(&mut self, at: f64) {
        let done = self.resume_ms.take().unwrap_or(0.0);
        if let Some(running) = &mut self.frame.transition {
            running.started = at - done;
            if let Some(cut) = &mut running.interrupted {
                cut.at = at;
            }
        }
    }

    /// Outlines the hotspots from `at` for `HINT_MS`, if they are outlined.
    pub fn restart_hint(&mut self, at: f64) {
        if self.hint_until.is_some() {
            self.hint_until = Some(at + HINT_MS);
        }
    }

    /// The boxes of the layers a pointer can trigger on the board shown and on
    /// its open overlays, on the screen: the layers with an interaction that
    /// a click, hover or press runs and whose target is there.
    pub fn hotspot_rects(&self) -> Vec<Rect> {
        let mut shown = vec![(self.frame.board, [0.0, 0.0])];
        shown.extend(self.frame.overlays.iter().map(|o| (o.board, o.origin)));
        let mut rects = Vec::new();
        for (board, origin) in shown {
            let Some(boxes) = self.boxes.get(&board) else {
                continue;
            };
            for entry in self.entries(&board) {
                let reacts = entry.interactions.iter().any(|interaction| {
                    !matches!(
                        interaction.trigger,
                        Trigger::AfterDelay { .. } | Trigger::KeyPress { .. }
                    ) && interaction
                        .actions
                        .iter()
                        .any(|action| self.resolves(action))
                });
                if let (true, Some(rect)) = (reacts, boxes.rects.get(&entry.id)) {
                    rects.push(rect.offset(origin));
                }
            }
        }
        rects
    }

    /// Whether the player would act on `action`: one that points at a node
    /// needs it to be there.
    fn resolves(&self, action: &Action) -> bool {
        match action {
            Action::Navigate { dest, .. }
            | Action::OpenOverlay { dest, .. }
            | Action::SwapOverlay { dest } => self.board_id(dest).is_some(),
            Action::ChangeTo { dest } => dest
                .parse::<NodeId>()
                .is_ok_and(|id| self.components.contains_key(&id)),
            _ => true,
        }
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

    /// A click at `x`, `y` on the layer the button went down on, `layer`.
    fn click(
        &mut self,
        x: f64,
        y: f64,
        at: f64,
        layer: Option<NodeId>,
        effects: &mut Vec<Effect>,
    ) -> bool {
        if let Some(top) = self.frame.overlays.last() {
            let inside = self
                .root_rect(&top.board)
                .is_some_and(|rect| rect.offset(top.origin).contains(x, y));
            if !inside {
                if top.close_on_outside {
                    self.close_overlay();
                    return true;
                }
                return false;
            }
        }
        let under = self.under(x, y);
        let Some(found) = deepest(&under, |trigger| *trigger == Trigger::OnClick) else {
            return layer.is_none() && self.miss(at, effects);
        };
        if layer != Some(found.id) {
            return false;
        }
        let mut changed = false;
        for interaction in &found.interactions {
            changed |= self.run(interaction, &found.site, found.rect, at, effects);
        }
        changed
    }

    /// A key going down. A key trigger fires on the topmost layer that has
    /// one for the key and the modifiers held, looking through the open
    /// overlays from the top and then the board shown; nothing else fires.
    /// Releasing a key does nothing.
    pub fn key(&mut self, event: KeyEvent) -> Vec<Effect> {
        let mods = Mods {
            ctrl: event.ctrl,
            alt: event.alt,
            shift: event.shift,
            cmd: event.cmd,
        };
        self.press_key(mods, keys::key_alias(&event.key), event.down)
    }

    /// A controller button going down: `button` is one of
    /// `keys::GAMEPAD_BUTTONS`, which a trigger writes as `Gamepad A`.
    pub fn gamepad(&mut self, button: &str, down: bool) -> Vec<Effect> {
        self.press_key(
            Mods::default(),
            keys::key_alias(&format!("Gamepad {button}")),
            down,
        )
    }

    fn press_key(&mut self, mods: Mods, key: String, down: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        if !down {
            return effects;
        }
        let at = self.last_at;
        let mut sources: Vec<_> = self
            .frame
            .overlays
            .iter()
            .rev()
            .map(|overlay| (overlay.board, overlay.origin))
            .collect();
        sources.push((self.frame.board, [0.0, 0.0]));
        for (board, origin) in sources {
            let found = self.entries(&board).into_iter().rev().find_map(|entry| {
                let matching: Vec<Interaction> = entry
                    .interactions
                    .into_iter()
                    .filter(|interaction| {
                        matches!(&interaction.trigger, Trigger::KeyPress { key: text }
                            if keys::split(text) == (mods, key.clone()))
                    })
                    .collect();
                (!matching.is_empty()).then_some((matching, entry.site, entry.id))
            });
            let Some((interactions, site, id)) = found else {
                continue;
            };
            let rect = self
                .boxes
                .get(&board)
                .and_then(|boxes| boxes.rects.get(&id))
                .map_or(
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    },
                    |rect| rect.offset(origin),
                );
            effects.push(Effect::KeyHandled);
            let mut changed = false;
            for interaction in &interactions {
                changed |= self.run(interaction, &site, rect, at, &mut effects);
            }
            if changed {
                effects.push(Effect::NeedsFrame);
            }
            break;
        }
        effects
    }

    /// A click that did nothing: outlines the hotspots.
    fn miss(&mut self, at: f64, effects: &mut Vec<Effect>) -> bool {
        self.hint_until = Some(at + HINT_MS);
        effects.push(Effect::Missed);
        true
    }

    /// The layers with interactions that are under the point, in drawing
    /// order, on the board or overlay a touch lands on.
    fn under(&self, x: f64, y: f64) -> Vec<Under> {
        let Some((board, origin)) = self.sources().into_iter().next() else {
            return Vec::new();
        };
        let Some(boxes) = self.boxes.get(&board) else {
            return Vec::new();
        };
        self.entries(&board)
            .into_iter()
            .filter_map(|entry| {
                let rect = boxes.rects.get(&entry.id)?.offset(origin);
                rect.contains(x, y).then_some(Under { entry, rect })
            })
            .collect()
    }

    fn apply(
        &mut self,
        action: &Action,
        play: Play,
        site: &Site,
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
                let history_len = self.history.boards().len();
                let visit = self.board_visit;
                self.history.push(from, play);
                // What a hover or press puts back returns to the board it
                // left, and the layers the pointer never left stay hovered.
                let kept = if self.recording.is_some() {
                    self.hovered
                        .iter()
                        .filter(|hover| hover.reverts.is_empty() && hover.board_visit == visit)
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                self.show(to, at);
                self.hovered.extend(kept);
                self.frame.transition = Some(self.running(from, to, false, play, at));
                self.note(Revert::Board {
                    to,
                    from,
                    visit,
                    history_len,
                    play,
                });
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
                let visit = self.next_visit();
                self.frame.overlays.push(OverlayState {
                    board,
                    origin,
                    background: background.clone(),
                    close_on_outside: *close_on_outside,
                    opened_at: at,
                    visit,
                    position: *position,
                    anchor: *anchor,
                    offset: *offset,
                    trigger_box: trigger,
                });
                self.note(Revert::Overlay { visit });
                self.schedule(board, visit, origin, at);
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
                let visit = self.next_visit();
                let top = self.frame.overlays.last_mut().expect("checked above");
                let before = top.clone();
                top.board = board;
                self.forget(before.board);
                let top = self.frame.overlays.last_mut().expect("checked above");
                top.origin = origin;
                top.opened_at = at;
                top.visit = visit;
                self.note(Revert::Swap { visit, before });
                self.schedule(board, visit, origin, at);
                true
            }
            Action::CloseOverlay => self.close_overlay(),
            Action::OpenUrl { url, new_tab } => {
                effects.push(Effect::OpenUrl {
                    url: url.clone(),
                    new_tab: *new_tab,
                });
                false
            }
            Action::ChangeTo { dest } => {
                let Ok(component) = dest.parse::<NodeId>() else {
                    return false;
                };
                // The innermost instance whose variants include the
                // destination changes; the layer's own instance when it is
                // the one that fires.
                let Some(destination) = self.components.get(&component) else {
                    return false;
                };
                let Some(instance) = site.holders.iter().rev().find(|holder| {
                    self.components
                        .get(&holder.shown)
                        .is_some_and(|shown| same_set(shown, destination))
                }) else {
                    return false;
                };
                let address = instance.address.clone();
                let previous = self.set_swap(site.board, address.clone(), Some(component));
                self.note(Revert::Variant {
                    board: site.board,
                    instance: address,
                    previous,
                });
                true
            }
            Action::SetVariable {
                variable,
                value,
                expr,
            } => self.set_variable(variable, value.as_ref(), expr.as_ref()),
            Action::SetVariableMode { collection, mode } => {
                self.set_variable_mode(collection, mode)
            }
            Action::Conditional { blocks } => {
                for block in blocks {
                    let holds = match &block.condition {
                        None => true,
                        Some(condition) => {
                            matches!(self.evaluate(condition), Some(TokenValue::Boolean(true)))
                        }
                    };
                    if holds {
                        let mut changed = false;
                        for action in &block.actions {
                            changed |= self.apply(action, play, site, trigger, at, effects);
                        }
                        return changed;
                    }
                }
                false
            }
            // Scrolling comes in a later story.
            _ => false,
        }
    }

    /// Ends the visit of the board shown and of its overlays: their swaps go.
    fn end_visit(&mut self) {
        self.forget(self.frame.board);
        for overlay in std::mem::take(&mut self.frame.overlays) {
            self.forget(overlay.board);
        }
    }

    /// Closes the top overlay, with the swaps made inside it.
    fn close_overlay(&mut self) -> bool {
        let Some(top) = self.frame.overlays.pop() else {
            return false;
        };
        self.forget(top.board);
        true
    }

    /// Drops the swaps of `board`'s visit.
    fn forget(&mut self, board: NodeId) {
        let Some(swaps) = self.swaps.remove(&board) else {
            return;
        };
        for instance in swaps.top.keys() {
            self.frame.variant_swaps.remove(instance);
        }
        for address in swaps.nested.keys() {
            self.frame.nested_swaps.remove(address);
        }
    }

    /// Sets, or with `None` clears, the swap of the instance at `address` in
    /// `board`'s visit, and gives the one it had.
    fn set_swap(
        &mut self,
        board: NodeId,
        address: NodeAddress,
        to: Option<NodeId>,
    ) -> Option<NodeId> {
        let swaps = self.swaps.entry(board).or_default();
        match address.instance() {
            None => {
                let id = *address.node();
                match to {
                    Some(component) => {
                        self.frame.variant_swaps.insert(id, component);
                        swaps.top.insert(id, component)
                    }
                    None => {
                        self.frame.variant_swaps.remove(&id);
                        swaps.top.remove(&id)
                    }
                }
            }
            Some(_) => match to {
                Some(component) => {
                    self.frame.nested_swaps.insert(address.clone(), component);
                    swaps.nested.insert(address, component)
                }
                None => {
                    self.frame.nested_swaps.remove(&address);
                    swaps.nested.remove(&address)
                }
            },
        }
    }

    /// The board that variables are read and set for: the one under the
    /// overlays.
    fn current_board(&self) -> Option<&Node> {
        self.boards.get(&self.frame.board)
    }

    /// The value of the expression `json` on the current board; `None` when
    /// it cannot be evaluated.
    fn evaluate(&self, json: &Value) -> Option<TokenValue> {
        let (tree, board) = (self.tokens.as_ref()?, self.current_board()?);
        variables::evaluate(json, &self.frame.variables, tree, board).ok()
    }

    /// Sets the token `variable` to `value` or to what `expr` evaluates to,
    /// for the session. Nothing happens if the token is not there, the value
    /// does not fit its type, or the expression cannot be evaluated.
    fn set_variable(
        &mut self,
        variable: &str,
        value: Option<&Value>,
        expr: Option<&Value>,
    ) -> bool {
        let new = match (value, expr) {
            (Some(value), None) => TokenValue::from_json(value),
            (None, Some(expr)) => self.evaluate(expr),
            _ => None,
        };
        let fits = new.as_ref().is_some_and(|new| {
            self.tokens
                .as_ref()
                .and_then(|tree| tree.token(variable))
                .is_some_and(|token| new.fits(token.ty()))
        });
        let Some(new) = new.filter(|_| fits) else {
            return false;
        };
        self.frame
            .variables
            .values
            .insert(variable.to_string(), new);
        true
    }

    /// Sets the mode of `collection` on the current board for the session,
    /// from the modes the board has. Nothing happens if the design system has
    /// no such collection or mode.
    fn set_variable_mode(&mut self, collection: &str, mode: &str) -> bool {
        let known = self.tokens.as_ref().is_some_and(|tree| {
            tree.collections()
                .iter()
                .any(|c| c.name == collection && c.modes.iter().any(|listed| listed == mode))
        });
        let Some(board) = self.current_board().filter(|_| known) else {
            return false;
        };
        let id = *board.id();
        let start = board.modes().cloned().unwrap_or_default();
        self.frame
            .variables
            .modes
            .entry(id)
            .or_insert(ModeChoice(start))
            .0
            .insert(collection.to_string(), mode.to_string());
        true
    }

    /// Whether the layer `node` of `board` is drawn: its `visible`, with a
    /// token read as the session has it.
    fn shown(&self, node: &Node, board: &NodeId) -> bool {
        match node.visible_setting() {
            TokenOr::Literal(visible) => *visible,
            TokenOr::Token(token) => {
                let json = serde_json::json!({"variable": token.path()});
                let hidden =
                    self.boards
                        .get(board)
                        .zip(self.tokens.as_ref())
                        .and_then(|(board, tree)| {
                            variables::evaluate(&json, &self.frame.variables, tree, board).ok()
                        })
                        == Some(TokenValue::Boolean(false));
                !hidden
            }
        }
    }

    fn go_back(&mut self, at: f64) -> bool {
        let Some((to, play)) = self.history.pop() else {
            return false;
        };
        let from = self.frame.board;
        self.show(to, at);
        self.frame.transition = Some(self.running(from, to, true, play, at));
        true
    }

    /// The transition from `from` to `to` that starts at `started`. Back plays
    /// the transition of the navigate it undoes, reversed. A transition still
    /// playing is frozen as it is and becomes what this one changes from.
    fn running(&self, from: NodeId, to: NodeId, back: bool, play: Play, started: f64) -> Running {
        let interrupted = self
            .frame
            .transition
            .clone()
            .filter(|before| !progress(before, started).done)
            .map(|running| {
                Box::new(Interrupted {
                    running,
                    at: started,
                })
            });
        Running {
            from,
            to,
            transition: if back {
                reversed(play.transition)
            } else {
                play.transition
            },
            reversed: back,
            started,
            duration_ms: play.duration_ms,
            easing: play.easing,
            follows_pointer: false,
            interrupted,
        }
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
        let site = Site {
            board: *board,
            holders: Vec::new(),
        };
        self.walk(root, board, 0, None, &site, &inverse, &mut entries);
        entries
    }

    /// `outer` is the instance of the page that the node is inside, which
    /// addresses it; `site` is the board and the instances around it.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        node: &Node,
        board: &NodeId,
        depth: usize,
        outer: Option<NodeId>,
        site: &Site,
        inverse: &HashMap<(NodeId, NodeId), NodeId>,
        out: &mut Vec<Entry>,
    ) {
        if !self.shown(node, board) || depth > MAX_DEPTH {
            return;
        }
        let id = match outer {
            None => Some(*node.id()),
            Some(outer) => inverse.get(&(outer, *node.id())).copied(),
        };
        if let NodeData::Instance(instance) = node.data() {
            let address = NodeAddress::new(outer, *node.id());
            let swapped = match outer {
                None => self.frame.variant_swaps.get(node.id()),
                Some(_) => self.frame.nested_swaps.get(&address),
            };
            let shown = swapped
                .or_else(|| self.shown.get(node.id()))
                .unwrap_or(&instance.component);
            let component = self.components.get(shown);
            let mut site = site.clone();
            site.holders.push(Holder {
                address,
                shown: *shown,
            });
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
                    site: site.clone(),
                });
            }
            if let Some(NodeData::Component(component)) = component.map(Node::data) {
                let outer = outer.unwrap_or(*node.id());
                for child in &component.frame.children {
                    self.walk(child, board, depth + 1, Some(outer), &site, inverse, out);
                }
            }
            return;
        }
        if let (Some(id), false) = (id, node.interactions().is_empty()) {
            out.push(Entry {
                id,
                depth,
                interactions: node.interactions().to_vec(),
                site: site.clone(),
            });
        }
        for child in node.children() {
            self.walk(child, board, depth + 1, outer, site, inverse, out);
        }
    }
}

/// Whether `a` and `b` are variants of one component: the same name and the
/// same variant properties with the same values.
fn same_set(a: &Node, b: &Node) -> bool {
    fn variants(node: &Node) -> Vec<(&String, &Vec<String>)> {
        let NodeData::Component(component) = node.data() else {
            return Vec::new();
        };
        component
            .props
            .iter()
            .flat_map(PropDefs::iter)
            .filter_map(|(name, def)| match def {
                PropDef::Variant { values, .. } => Some((name, values)),
                _ => None,
            })
            .collect()
    }
    a.name() == b.name() && variants(a) == variants(b)
}
