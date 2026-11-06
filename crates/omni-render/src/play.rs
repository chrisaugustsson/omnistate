//! Playing a page's prototype on a canvas: the board the player shows, drawn
//! at its own size in the middle of the stage on the page's prototype
//! background, and the pointer events of the stage turned into the player's.
//!
//! The rules of the prototype are `omni_proto::Player`'s; this module gives it
//! the boxes of the layers (`Scene::board_boxes`), draws what it shows and
//! says what the page has to do next.

use crate::view::{Scene, Viewport};
use omni_core::design::proto::Transition;
use omni_core::design::{NodeData, NodeId, NodeKind};
use omni_proto::{Effect, HINT_MS, Player, PointerEvent, PointerKind, Rect};
use serde_json::{Value, json};
use skia_safe::{Canvas, Color};

/// FigCraft's default background, for a page whose settings do not say.
const DEFAULT_BACKGROUND: Color = Color::from_argb(255, 0x1e, 0x1e, 0x1e);

/// The width of the outline around a hotspot, in CSS pixels.
const HINT_WIDTH: f32 = 2.0;

/// What the page does next, after `Session::tick`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// Nothing changed since the last frame.
    Idle,
    /// What the player shows changed: draw it once.
    Changed,
    /// A transition is playing: draw, and tick again on the next frame.
    Animating,
}

/// Where the board sits on the stage, as the last frame drew it.
#[derive(Clone, Copy)]
struct Placement {
    scale: f32,
    x: f32,
    y: f32,
}

pub struct Session {
    player: Player,
    /// The boards of the page that the scene draws, in page order.
    boards: Vec<NodeId>,
    background: Color,
    placement: Placement,
    changed: bool,
    /// A link the player asked to open, until the page has read it.
    url: Option<(String, bool)>,
    /// Whether a click missed that the next tick has to time the outlines of.
    missed: bool,
    /// Whether a click missed that the page has not read yet.
    toast: bool,
    /// The time of the last tick, which events between ticks happen at.
    now: f64,
    /// The size of the last frame: width, height and pixel ratio.
    size: Option<(i32, i32, f32)>,
}

fn not_a_board(board: &str) -> String {
    format!("`{board}` is not a board of the shown page")
}

fn color(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#')?;
    let channel = |index: usize| u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok();
    let alpha = if hex.len() == 8 { channel(3)? } else { 255 };
    Some(Color::from_argb(
        alpha,
        channel(0)?,
        channel(1)?,
        channel(2)?,
    ))
}

impl Session {
    /// A session on `board` of the page that `scene` shows. Fails if the
    /// page has no such board.
    pub fn start(scene: &Scene, board: &str) -> Result<Session, String> {
        let drawn = scene.board_ids();
        let start = board
            .parse::<NodeId>()
            .ok()
            .filter(|start| drawn.contains(start))
            .ok_or_else(|| not_a_board(board))?;
        let NodeData::Page(page) = scene.source_page().data() else {
            return Err("the scene is not a page".to_string());
        };
        let boards: Vec<NodeId> = page
            .children
            .iter()
            .filter(|node| node.kind() == NodeKind::Board)
            .map(|node| *node.id())
            .filter(|id| drawn.contains(id))
            .collect();
        let background = color(page.prototype().background.as_str()).unwrap_or(DEFAULT_BACKGROUND);
        let mut player = Player::new(page, scene.components(), start);
        for id in &boards {
            let Some((rects, layers)) = scene.board_boxes(id) else {
                continue;
            };
            let rects = rects
                .into_iter()
                .map(|(id, rect)| {
                    let rect = Rect {
                        x: f64::from(rect.x),
                        y: f64::from(rect.y),
                        width: f64::from(rect.width),
                        height: f64::from(rect.height),
                    };
                    (id, rect)
                })
                .collect();
            player.set_boxes(*id, rects, layers);
        }
        Ok(Session {
            player,
            boards,
            background,
            placement: Placement {
                scale: 1.0,
                x: 0.0,
                y: 0.0,
            },
            changed: true,
            url: None,
            missed: false,
            toast: false,
            now: 0.0,
            size: None,
        })
    }

    /// A pointer event at the stage point `x`, `y` in CSS pixels: `kind` is 0
    /// for down, 1 for move, 2 for up and 3 for leave. Returns whether what
    /// the player shows changed.
    pub fn pointer(&mut self, kind: u32, x: f32, y: f32) -> bool {
        let kind = match kind {
            0 => PointerKind::Down,
            1 => PointerKind::Move,
            2 => PointerKind::Up,
            _ => PointerKind::Leave,
        };
        let Placement {
            scale,
            x: left,
            y: top,
        } = self.placement;
        let event = PointerEvent {
            kind,
            x: f64::from((x - left) / scale),
            y: f64::from((y - top) / scale),
            at: self.now,
        };
        let mut changed = false;
        for effect in self.player.pointer(event) {
            match effect {
                Effect::NeedsFrame => changed = true,
                Effect::Missed => {
                    self.missed = true;
                    self.toast = true;
                }
                Effect::OpenUrl { url, new_tab } => {
                    self.url = Some((url, new_tab));
                    changed = true;
                }
            }
        }
        self.changed |= changed;
        changed
    }

    /// Goes back to the board the last navigate left. Returns whether there
    /// was one.
    pub fn back(&mut self) -> bool {
        let before = self.player.frame();
        self.player.back();
        let moved = self.player.frame() != before;
        self.changed |= moved;
        moved
    }

    /// Goes to `board` as a navigate does, so that back returns to the board it
    /// leaves. Fails if the page has no such board.
    pub fn go(&mut self, board: &str) -> Result<bool, String> {
        let id = self.board(board)?;
        let moved = self.player.go(id);
        self.changed |= moved;
        Ok(moved)
    }

    /// Starts over on `board`, with no history. Fails if the page has no such
    /// board.
    pub fn restart(&mut self, board: &str) -> Result<(), String> {
        let id = self.board(board)?;
        self.player.restart(id);
        self.changed = true;
        Ok(())
    }

    fn board(&self, board: &str) -> Result<NodeId, String> {
        board
            .parse::<NodeId>()
            .ok()
            .filter(|id| self.boards.contains(id))
            .ok_or_else(|| not_a_board(board))
    }

    /// Moves time on to `now`, in milliseconds.
    pub fn tick(&mut self, now: f64) -> Tick {
        self.now = now;
        // The outlines count from the first frame after the click, whose own
        // time is only as fresh as the last tick.
        if std::mem::take(&mut self.missed) {
            self.player.restart_hint(now);
        }
        let hinting = self.player.hint_until().is_some();
        self.player.tick(now);
        let hint_ended = hinting && self.player.hint_until().is_none();
        self.changed |= hint_ended;
        let animating = self.player.hint_until().is_some()
            || self
                .player
                .frame()
                .transition
                .is_some_and(|running| running.transition != Transition::Instant);
        if animating {
            Tick::Animating
        } else if self.changed {
            Tick::Changed
        } else {
            Tick::Idle
        }
    }

    /// The size of the last frame, to draw again at.
    pub fn size(&self) -> Option<(i32, i32, f32)> {
        self.size
    }

    /// Where the player is: `{"board": "<id>", "history": ["<id>"],
    /// "overlays": ["<id>"], "url": null}`, the history oldest first and the
    /// overlays with the top one last. `url` is `{"url": "...", "newTab":
    /// true}` once after a link was asked for, and `null` otherwise; `missed`
    /// is true once after a click that landed on nothing.
    pub fn state(&mut self) -> Value {
        let frame = self.player.frame();
        json!({
            "board": frame.board.to_string(),
            "history": self.player.history().iter().map(NodeId::to_string).collect::<Vec<_>>(),
            "overlays": frame.overlays.iter().map(|overlay| overlay.board.to_string()).collect::<Vec<_>>(),
            "missed": std::mem::take(&mut self.toast),
            "url": self.url.take().map(|(url, new_tab)| json!({"url": url, "newTab": new_tab})),
        })
    }

    /// Draws the board the player shows, and the overlays that are open, on
    /// `canvas` (`width` x `height` CSS pixels, `pixel_ratio` device pixels
    /// each): at its own size in the middle of the stage, scaled down to fit
    /// when it is bigger.
    pub fn draw(
        &mut self,
        scene: &mut Scene,
        canvas: &Canvas,
        (width, height): (f32, f32),
        pixel_ratio: f32,
    ) {
        self.size = Some((
            (width * pixel_ratio).round() as i32,
            (height * pixel_ratio).round() as i32,
            pixel_ratio,
        ));
        self.changed = false;
        canvas.clear(self.background);
        let frame = self.player.frame();
        let Some(bounds) = scene.board_bounds(&frame.board) else {
            return;
        };
        let scale = (width / bounds.width.max(1.0))
            .min(height / bounds.height.max(1.0))
            .min(1.0);
        let left = (width - bounds.width * scale) / 2.0;
        let top = (height - bounds.height * scale) / 2.0;
        self.placement = Placement {
            scale,
            x: left,
            y: top,
        };
        let mut shown = vec![(frame.board, [0.0, 0.0], None)];
        shown.extend(frame.overlays.iter().map(|overlay| {
            (
                overlay.board,
                overlay.origin,
                overlay.background.as_deref().and_then(color),
            )
        }));
        for (board, origin, sheet) in shown {
            if let Some(sheet) = sheet {
                canvas.draw_rect(
                    skia_safe::Rect::from_xywh(
                        left,
                        top,
                        bounds.width * scale,
                        bounds.height * scale,
                    ),
                    &skia_safe::Paint::new(skia_safe::Color4f::from(sheet), None),
                );
            }
            let Some(bounds) = scene.board_bounds(&board) else {
                continue;
            };
            let at = (
                left + origin[0] as f32 * scale,
                top + origin[1] as f32 * scale,
            );
            let viewport = Viewport::at(
                bounds.x - at.0 / scale,
                bounds.y - at.1 / scale,
                scale,
                width,
                height,
            );
            scene.render_board(canvas, &viewport, &board);
        }
        if let Some(until) = self.player.hint_until().filter(|until| *until > self.now) {
            let alpha = ((until - self.now) / HINT_MS).clamp(0.0, 1.0) as f32;
            let mut paint = skia_safe::Paint::new(
                skia_safe::Color4f::new(
                    0x3b as f32 / 255.0,
                    0x82 as f32 / 255.0,
                    0xf6 as f32 / 255.0,
                    alpha,
                ),
                None,
            );
            paint.set_style(skia_safe::PaintStyle::Stroke);
            paint.set_stroke_width(HINT_WIDTH);
            paint.set_anti_alias(true);
            for rect in self.player.hotspot_rects() {
                // Outside the box: the stroke's middle lies half its width out.
                canvas.draw_rect(
                    skia_safe::Rect::from_xywh(
                        left + rect.x as f32 * scale,
                        top + rect.y as f32 * scale,
                        rect.width as f32 * scale,
                        rect.height as f32 * scale,
                    )
                    .with_outset((HINT_WIDTH / 2.0, HINT_WIDTH / 2.0)),
                    &paint,
                );
            }
        }
    }
}
