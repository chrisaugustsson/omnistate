//! Scrolling in play mode: where each scrolling container is scrolled to, and
//! where that puts every layer of a board on the screen.
//!
//! A container is the top-level board, when the prototype has a device screen
//! to clip it to, and every frame whose `overflow` scrolls. A layer with
//! `scrollPosition` `fixed` stays where it is in its nearest container; a
//! `sticky` one scrolls until it is its own offset from the top of that
//! container and then stays. Both draw above what scrolls under them.

use crate::player::{Entry, Rect};
use crate::transition::progress_of;
use omni_core::design::proto::{Easing, Overflow, ScrollPosition};
use omni_core::design::{Node, NodeData, NodeId};
use std::collections::{HashMap, HashSet};

/// The scroll state of each container of a board, by the container's ID (the
/// board's own for the board).
pub type Scrolls = HashMap<NodeId, ScrollState>;

/// Far enough out to leave a clip unbounded.
const BIG: f64 = 1.0e9;

/// A scroll that animates to an offset.
#[derive(Clone, Debug, PartialEq)]
struct Glide {
    from: [f64; 2],
    to: [f64; 2],
    started: f64,
    duration_ms: u32,
    easing: Easing,
    /// Whether the caller has yet to time it from its own clock.
    fresh: bool,
}

impl Glide {
    fn duration(&self) -> f64 {
        f64::from(self.easing.spring_duration_ms().unwrap_or(self.duration_ms))
    }
}

/// How far one scrolling container is scrolled: the offset of its content
/// from the top-left corner, and the animation that is taking it to a new one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollState {
    pub x: f64,
    pub y: f64,
    glide: Option<Glide>,
}

impl ScrollState {
    pub fn at(x: f64, y: f64) -> ScrollState {
        ScrollState { x, y, glide: None }
    }

    /// The offset at `now`, in milliseconds.
    pub fn offset(&self, now: f64) -> [f64; 2] {
        let Some(glide) = &self.glide else {
            return [self.x, self.y];
        };
        let t = progress_of(glide.easing, glide.duration(), glide.started, now).t;
        [
            glide.from[0] + (glide.to[0] - glide.from[0]) * t,
            glide.from[1] + (glide.to[1] - glide.from[1]) * t,
        ]
    }

    /// Whether it is animating.
    pub fn gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// Jumps to `offset`, cutting any animation short.
    pub(crate) fn set(&mut self, offset: [f64; 2]) {
        *self = ScrollState::at(offset[0], offset[1]);
    }

    /// Animates from where it is at `now` to `to`.
    pub(crate) fn glide_to(&mut self, to: [f64; 2], now: f64, duration_ms: u32, easing: Easing) {
        let from = self.offset(now);
        if duration_ms == 0 {
            self.set(to);
            return;
        }
        self.x = to[0];
        self.y = to[1];
        self.glide = Some(Glide {
            from,
            to,
            started: now,
            duration_ms,
            easing,
            fresh: true,
        });
    }

    /// Times an animation that began at an event from `now`, and ends one that
    /// has run its course.
    pub(crate) fn advance(&mut self, now: f64) {
        let Some(glide) = &mut self.glide else {
            return;
        };
        if std::mem::take(&mut glide.fresh) {
            glide.started = now;
        }
        if progress_of(glide.easing, glide.duration(), glide.started, now).done {
            self.glide = None;
        }
    }
}

/// A container, with where it is on the screen.
#[derive(Clone, Debug)]
pub(crate) struct Container {
    pub(crate) id: NodeId,
    /// The top-left corner of its viewport, as designed.
    pub(crate) origin: [f64; 2],
    /// Its viewport on the screen.
    pub(crate) view: Rect,
    /// The part of the screen its ancestors leave visible.
    pub(crate) clip: Rect,
    /// How far each axis can scroll.
    pub(crate) max: [f64; 2],
    pub(crate) axes: [bool; 2],
    /// Where it is scrolled to, within `max`.
    pub(crate) offset: [f64; 2],
    pub(crate) depth: usize,
}

impl Container {
    pub(crate) fn hit(&self, x: f64, y: f64) -> bool {
        self.view
            .intersect(&self.clip)
            .is_some_and(|visible| visible.contains(x, y))
    }
}

/// A layer as shown.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    /// Its box on the screen.
    pub(crate) rect: Rect,
    /// The part of the screen its ancestors leave visible.
    pub(crate) clip: Rect,
    /// How far it is moved from its design.
    pub(crate) shift: [f64; 2],
    pub(crate) parent: Option<NodeId>,
}

/// Where every layer of a board is.
#[derive(Debug, Default)]
pub(crate) struct Placement {
    pub(crate) nodes: HashMap<NodeId, Shown>,
    pub(crate) containers: Vec<Container>,
    /// The fixed layers and the sticky ones that are stuck, which draw above
    /// the content of their container, in drawing order.
    pub(crate) raised: Vec<NodeId>,
    /// The frames that clip their content to their box.
    pub(crate) clipped: HashSet<NodeId>,
}

impl Placement {
    /// The topmost raised layer that holds the point.
    pub(crate) fn raised_hit(&self, x: f64, y: f64) -> Option<NodeId> {
        self.raised.iter().rev().copied().find(|id| {
            self.nodes
                .get(id)
                .is_some_and(|shown| shown.rect.contains(x, y) && shown.clip.contains(x, y))
        })
    }

    /// The layer of the board that `entry` is on or inside, which has a place
    /// of its own: the layer itself, or the instance around it.
    pub(crate) fn anchor(&self, entry: &Entry) -> Option<NodeId> {
        if self.nodes.contains_key(&entry.id) {
            return Some(entry.id);
        }
        let holder = entry.site.holders.first()?;
        Some(*holder.address.node())
    }

    /// Whether `node` is `top`, is inside it, or holds it.
    pub(crate) fn related(&self, node: NodeId, top: NodeId) -> bool {
        self.inside(node, top) || self.inside(top, node)
    }

    fn inside(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(id) = at {
            if id == ancestor {
                return true;
            }
            at = self.nodes.get(&id).and_then(|shown| shown.parent);
        }
        false
    }

    /// What the renderer needs to draw the board.
    pub(crate) fn view(&self) -> ScrollView {
        ScrollView {
            shifts: self
                .nodes
                .iter()
                .map(|(id, shown)| (*id, shown.shift))
                .collect(),
            raised: self.raised.iter().copied().collect(),
            clipped: self.clipped.clone(),
        }
    }
}

/// How a board is drawn scrolled: how far each layer moves, which layers
/// draw above their siblings, and which frames clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollView {
    /// By layer. A layer inside an instance moves with the instance.
    pub shifts: HashMap<NodeId, [f64; 2]>,
    /// The fixed layers and the sticky ones that are stuck: they draw last
    /// among their siblings.
    pub raised: HashSet<NodeId>,
    /// The frames that clip their content to their box.
    pub clipped: HashSet<NodeId>,
}

impl ScrollView {
    /// Whether the board is drawn as designed.
    pub fn is_plain(&self) -> bool {
        self.raised.is_empty()
            && self.clipped.is_empty()
            && self.shifts.values().all(|shift| *shift == [0.0, 0.0])
    }
}

struct Ctx {
    /// How far the container is scrolled.
    offset: [f64; 2],
    /// Where its viewport's top edge is on the screen.
    top: f64,
}

struct Walk<'a> {
    rects: &'a HashMap<NodeId, Rect>,
    screen: Option<[f64; 2]>,
    scrolls: &'a Scrolls,
    now: f64,
    out: Placement,
}

/// Where the layers of `root`, a board, are with `scrolls` at `now`. `rects`
/// are the layers' designed boxes; `screen` is the size of the device screen
/// that clips the board, if there is one.
pub(crate) fn place(
    root: &Node,
    rects: &HashMap<NodeId, Rect>,
    screen: Option<[f64; 2]>,
    scrolls: &Scrolls,
    now: f64,
) -> Placement {
    let mut walk = Walk {
        rects,
        screen,
        scrolls,
        now,
        out: Placement::default(),
    };
    let everything = Rect {
        x: -BIG,
        y: -BIG,
        width: 2.0 * BIG,
        height: 2.0 * BIG,
    };
    walk.visit(root, None, [0.0, 0.0], everything, None, 0);
    walk.out
}

impl Walk<'_> {
    /// The designed box of a layer; a layer the caller gave no box has none.
    fn rect(&self, id: &NodeId) -> Rect {
        self.rects.get(id).copied().unwrap_or(NOTHING_AT_ORIGIN)
    }

    fn visit(
        &mut self,
        node: &Node,
        parent: Option<NodeId>,
        shift: [f64; 2],
        clip: Rect,
        ctx: Option<&Ctx>,
        depth: usize,
    ) {
        let id = *node.id();
        let rect = self.rect(&id);
        let shown = rect.offset(shift);
        self.out.nodes.insert(
            id,
            Shown {
                rect: shown,
                clip,
                shift,
                parent,
            },
        );
        let mut inner_clip = clip;
        let mut scrolled = shift;
        let mut own = None;
        if let Some((axes, content, view)) = self.container(node, rect, parent.is_none()) {
            let max = [
                if axes[0] {
                    (content[0] - view.width).max(0.0)
                } else {
                    0.0
                },
                if axes[1] {
                    (content[1] - view.height).max(0.0)
                } else {
                    0.0
                },
            ];
            let raw = self
                .scrolls
                .get(&id)
                .map_or([0.0, 0.0], |state| state.offset(self.now));
            let offset = [raw[0].clamp(0.0, max[0]), raw[1].clamp(0.0, max[1])];
            let displayed = view.offset(shift);
            self.out.containers.push(Container {
                id,
                origin: [view.x, view.y],
                view: displayed,
                clip,
                max,
                axes,
                offset,
                depth,
            });
            if parent.is_some() {
                self.out.clipped.insert(id);
            }
            inner_clip = clip.intersect(&displayed).unwrap_or(NOTHING);
            scrolled = [shift[0] - offset[0], shift[1] - offset[1]];
            own = Some(Ctx {
                offset,
                top: displayed.y,
            });
        } else if clips(node) {
            inner_clip = clip.intersect(&shown).unwrap_or(NOTHING);
        }
        let ctx = own.as_ref().or(ctx);
        let raised = self.inside_raised(id);
        for child in node.children() {
            let child_rect = self.rect(child.id());
            let (child_shift, lifted) = match child.scroll_position() {
                ScrollPosition::Scrolls => (scrolled, false),
                ScrollPosition::Fixed => {
                    let back = ctx.map_or([0.0, 0.0], |ctx| ctx.offset);
                    ([scrolled[0] + back[0], scrolled[1] + back[1]], true)
                }
                ScrollPosition::Sticky => {
                    let mut moved = scrolled;
                    let mut stuck = false;
                    if let Some(ctx) = ctx {
                        let stick = ctx.top + (child_rect.y - rect.y);
                        let at = child_rect.y + moved[1];
                        if at < stick {
                            moved[1] += stick - at;
                            stuck = true;
                        }
                    }
                    (moved, stuck)
                }
            };
            if lifted && !raised {
                self.out.raised.push(*child.id());
            }
            self.visit(child, Some(id), child_shift, inner_clip, ctx, depth + 1);
        }
    }

    /// Whether `id` is a raised layer or inside one.
    fn inside_raised(&self, id: NodeId) -> bool {
        let mut at = Some(id);
        while let Some(id) = at {
            if self.out.raised.contains(&id) {
                return true;
            }
            at = self.out.nodes.get(&id).and_then(|shown| shown.parent);
        }
        false
    }

    /// The axes a node scrolls on, the size of its content and its viewport,
    /// if it is a container.
    fn container(
        &self,
        node: &Node,
        rect: Rect,
        is_board: bool,
    ) -> Option<([bool; 2], [f64; 2], Rect)> {
        if is_board {
            let [width, height] = self.screen?;
            let view = Rect {
                x: rect.x,
                y: rect.y,
                width,
                height,
            };
            return Some(([true, true], [rect.width, rect.height], view));
        }
        let NodeData::Frame(frame) = node.data() else {
            return None;
        };
        let axes = match frame.overflow {
            Overflow::None => return None,
            Overflow::Horizontal => [true, false],
            Overflow::Vertical => [false, true],
            Overflow::Both => [true, true],
        };
        let mut content = [0.0f64, 0.0f64];
        for child in &frame.children {
            if child.scroll_position() == ScrollPosition::Fixed {
                continue;
            }
            if let Some(child) = self.rects.get(child.id()) {
                content[0] = content[0].max(child.x + child.width - rect.x);
                content[1] = content[1].max(child.y + child.height - rect.y);
            }
        }
        Some((axes, content, rect))
    }
}

const NOTHING_AT_ORIGIN: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 0.0,
    height: 0.0,
};

/// A clip that holds no point.
const NOTHING: Rect = Rect {
    x: BIG,
    y: BIG,
    width: 0.0,
    height: 0.0,
};

/// Whether a frame that does not scroll clips its content.
fn clips(node: &Node) -> bool {
    matches!(node.data(), NodeData::Frame(frame) if frame.clip == Some(true))
}
