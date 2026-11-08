//! Playing a page's prototype on a canvas: the board the player shows, drawn
//! at its own size in the middle of the stage on the page's prototype
//! background, and the pointer events of the stage turned into the player's.
//!
//! The rules of the prototype are `omni_proto::Player`'s; this module gives it
//! the boxes of the layers (`Scene::board_boxes`), draws what it shows and
//! says what the page has to do next.

use crate::layout::{LayoutBox, ShapedTexts};
use crate::view::{Scene, Viewport};
use omni_core::design::devices::{self, CUSTOM};
use omni_core::design::proto::{Direction, Rotation, Transition};
use omni_core::design::tokens::{ModeChoice, TokenValue};
use omni_core::design::{
    Color as DesignColor, Fill, Node, NodeAddress, NodeData, NodeId, NodeKind, Radius, Stroke,
    TokenOr,
};
use omni_proto::keys::gamepad_button;
use omni_proto::matching::match_layers;
use omni_proto::transition::{duration_ms, progress};
use omni_proto::{
    Effect, HINT_MS, KeyEvent, Player, PointerEvent, PointerKind, Rect, Running, ScrollView,
    VariantRun,
};
use serde_json::{Value, json};
use skia_safe::{Canvas, Color};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

/// FigCraft's default background, for a page whose settings do not say.
const DEFAULT_BACKGROUND: Color = Color::from_argb(255, 0x1e, 0x1e, 0x1e);

/// The width of the outline around a hotspot, in CSS pixels.
const HINT_WIDTH: f32 = 2.0;

/// The width of the band in the device's colour around its screen, in the
/// screen's own pixels.
const BAND_WIDTH: f32 = 12.0;

/// How the screen is scaled on the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scale {
    /// Down to the stage, never up.
    Fit,
    Full,
    Half,
}

impl Scale {
    fn name(self) -> &'static str {
        match self {
            Scale::Fit => "fit",
            Scale::Full => "100",
            Scale::Half => "50",
        }
    }

    /// The scale of a `width` x `height` screen on a stage of `stage`.
    fn of(self, stage: (f32, f32), (width, height): (f32, f32)) -> f32 {
        match self {
            Scale::Fit => (stage.0 / width.max(1.0))
                .min(stage.1 / height.max(1.0))
                .min(1.0),
            Scale::Full => 1.0,
            Scale::Half => 0.5,
        }
    }
}

/// What the page does next, after `Session::tick`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// Nothing changed since the last frame.
    Idle,
    /// What the player shows changed: draw it once.
    Changed,
    /// A transition is playing or a wait is pending: draw, and tick again on the
    /// next frame.
    Animating,
    /// Nothing to draw, but a wait is running that a tick has to see run out:
    /// tick again on the next frame.
    Waiting,
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
    /// Whether an event began a transition that the next tick has to time.
    fresh: bool,
    /// The time of the last tick, which events between ticks happen at.
    now: f64,
    /// The size of the last frame: width, height and pixel ratio.
    size: Option<(i32, i32, f32)>,
    /// What each board that the scene has resolved for play was resolved
    /// with, where that is not the design system's own.
    resolved: HashMap<NodeId, PlayResolve>,
    /// The instance each smart-animated change of variant starts from, as it
    /// was drawn before the change, with the run, by the run's ID.
    snapshots: HashMap<u64, (VariantRun, Layers)>,
    /// The time the last frame was drawn at.
    drawn: f64,
    /// The device the prototype is shown on, by its name in the device table.
    device: String,
    scale: Scale,
    /// What the page's `prototype` says of the device.
    page: PageDevice,
}

/// The device settings of the page.
struct PageDevice {
    device: String,
    color: Option<String>,
    rotation: Rotation,
    custom_size: Option<[f64; 2]>,
}

impl PageDevice {
    /// The size of the screen of the device `name` as it is shown: `None` for
    /// a device with no screen, rotated when the page says landscape and the
    /// device turns.
    fn screen(&self, name: &str) -> Option<(f32, f32)> {
        if name == CUSTOM {
            return self
                .custom_size
                .map(|[width, height]| (width as f32, height as f32));
        }
        let device = devices::device(name)?;
        if device.width <= 0.0 || device.height <= 0.0 {
            return None;
        }
        let (width, height) = (device.width as f32, device.height as f32);
        Some(if device.rotates && self.rotation == Rotation::Landscape {
            (height, width)
        } else {
            (width, height)
        })
    }

    /// The colour of the band around the screen of the device `name`: the
    /// page's, when it is the page's device, else the device's first.
    fn band(&self, name: &str) -> Option<Color> {
        let device = devices::device(name)?;
        let chosen = self
            .color
            .as_deref()
            .filter(|_| name == self.device)
            .and_then(|wanted| device.colors.iter().find(|color| color.name == wanted));
        let [r, g, b] = chosen.or(device.colors.first())?.rgb;
        Some(Color::from_rgb(r, g, b))
    }
}

/// The token values, the modes of its own and the variant swaps that a board
/// was resolved with.
type PlayResolve = (
    BTreeMap<String, TokenValue>,
    Option<ModeChoice>,
    HashMap<NodeAddress, NodeId>,
);

/// Gives the player the boxes of the layers of `board` as the scene has them.
fn give_boxes(player: &mut Player, scene: &Scene, board: &NodeId) {
    let Some((rects, layers)) = scene.board_boxes(board) else {
        return;
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
    player.set_boxes(*board, rects, layers);
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

/// The way a transition moves a board: a unit step on the stage.
fn step(direction: Direction) -> (f32, f32) {
    match direction {
        Direction::Left => (-1.0, 0.0),
        Direction::Right => (1.0, 0.0),
        Direction::Top => (0.0, -1.0),
        Direction::Bottom => (0.0, 1.0),
    }
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
        let settings = page.prototype();
        let background = color(settings.background.as_str()).unwrap_or(DEFAULT_BACKGROUND);
        let page_device = PageDevice {
            device: settings.device.clone(),
            color: settings.color.clone(),
            rotation: settings.rotation,
            custom_size: settings.custom_size,
        };
        let mut player = Player::new(page, scene.components(), start);
        if let Some(tokens) = scene.tokens() {
            player.set_tokens(tokens.clone());
        }
        for id in &boards {
            give_boxes(&mut player, scene, id);
        }
        let device = page_device.device.clone();
        if let Some((width, height)) = page_device.screen(&device) {
            player.set_screen(f64::from(width), f64::from(height));
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
            fresh: false,
            now: 0.0,
            size: None,
            resolved: HashMap::new(),
            snapshots: HashMap::new(),
            drawn: 0.0,
            device,
            scale: Scale::Fit,
            page: page_device,
        })
    }

    /// Ends the session: the boards that play resolved with its own values
    /// and modes are resolved as the design has them again.
    pub fn end(self, scene: &mut Scene) {
        for board in self.resolved.into_keys() {
            scene.designed_board(&board);
        }
    }

    /// The boards the frame draws from: the one shown, the open overlays and
    /// those of the transition that is playing.
    fn drawn_boards(&self) -> Vec<NodeId> {
        let frame = self.player.frame();
        let mut boards = vec![frame.board];
        boards.extend(frame.overlays.iter().map(|overlay| overlay.board));
        let mut running = frame.transition.as_ref();
        while let Some(playing) = running {
            boards.extend([playing.from, playing.to]);
            running = playing.interrupted.as_ref().map(|cut| &cut.running);
        }
        boards
    }

    /// The instances of `board` that the visit has swapped, by address.
    fn swaps_of(&self, board: NodeId) -> HashMap<NodeAddress, NodeId> {
        let top = self.player.swaps_for(board);
        let nested = self.player.nested_swaps_for(board);
        top.iter()
            .map(|(instance, component)| (NodeAddress::new(None, *instance), *component))
            .chain(
                nested
                    .iter()
                    .map(|(at, component)| (at.clone(), *component)),
            )
            .collect()
    }

    /// Resolves the boards that the frame draws again where the session's
    /// variables or the visit's swaps changed them (or no longer do), and
    /// gives the player their new boxes. A board that a transition leaves is
    /// drawn as it was left until the transition ends.
    fn refresh(&mut self, scene: &mut Scene) {
        let shown = self.player.frame();
        let mut staying = vec![shown.board];
        staying.extend(shown.overlays.iter().map(|overlay| overlay.board));
        let drawing = self.drawn_boards();
        let mut boards = drawing.clone();
        boards.extend(self.resolved.keys().copied());
        let mut seen = Vec::new();
        for board in boards {
            if seen.contains(&board) {
                continue;
            }
            seen.push(board);
            let before = self.resolved.get(&board).cloned().unwrap_or_default();
            let variables = self.player.variables();
            let drawn = drawing.contains(&board);
            let now: PlayResolve = if drawn {
                (
                    variables.values.clone(),
                    variables.modes.get(&board).cloned(),
                    if staying.contains(&board) {
                        self.swaps_of(board)
                    } else {
                        before.2.clone()
                    },
                )
            } else if before.2.is_empty() {
                continue;
            } else {
                (before.0.clone(), before.1.clone(), HashMap::new())
            };
            if now == before {
                continue;
            }
            let done = if now == PlayResolve::default() {
                scene.designed_board(&board)
            } else {
                scene.play_board(&board, now.1.as_ref(), &now.0, &now.2)
            };
            if !done {
                continue;
            }
            give_boxes(&mut self.player, scene, &board);
            if now == PlayResolve::default() {
                self.resolved.remove(&board);
            } else {
                self.resolved.insert(board, now);
            }
        }
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
        let effects = self.player.pointer(event);
        let changed = self.take(effects).0;
        // A wait that began needs a frame to be ticked on.
        changed || !self.player.timers().is_empty()
    }

    /// Applies what the player asked for after an event. Returns whether what
    /// it shows changed and whether a key trigger fired.
    fn take(&mut self, effects: Vec<Effect>) -> (bool, bool) {
        let before = self.player.frame();
        let (changed, handled) = self.absorb(effects);
        self.changed |= changed;
        let after = self.player.frame();
        // A change of variant is timed by the player itself: only a new
        // transition restarts the clock of the one that plays.
        self.fresh |= after.transition != before.transition;
        (changed, handled)
    }

    /// Takes what the player asked for: whether the frame changed and whether
    /// a key trigger fired.
    fn absorb(&mut self, effects: Vec<Effect>) -> (bool, bool) {
        let mut changed = false;
        let mut handled = false;
        for effect in effects {
            match effect {
                Effect::NeedsFrame => changed = true,
                Effect::KeyHandled => handled = true,
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
        (changed, handled)
    }

    /// A key event, as JSON `{key, ctrl, alt, shift, cmd, down}`. Returns
    /// whether a key trigger fired.
    pub fn key(&mut self, json: &str) -> Result<bool, String> {
        let event: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
        let flag = |name: &str| event[name].as_bool().unwrap_or(false);
        let key = event["key"].as_str().ok_or("a key event needs a key")?;
        let effects = self.player.key(KeyEvent {
            key: key.to_string(),
            ctrl: flag("ctrl"),
            alt: flag("alt"),
            shift: flag("shift"),
            cmd: flag("cmd"),
            down: flag("down"),
        });
        Ok(self.take(effects).1)
    }

    /// The controller button at `index` of the standard layout going down or
    /// up. Returns whether a key trigger fired.
    pub fn gamepad(&mut self, index: u32, down: bool) -> bool {
        let Some(button) = gamepad_button(index as usize) else {
            return false;
        };
        let effects = self.player.gamepad(button, down);
        self.take(effects).1
    }

    /// Goes back to the board the last navigate left. Returns whether there
    /// was one.
    pub fn back(&mut self) -> bool {
        let before = self.player.frame();
        self.player.back();
        let moved = self.player.frame() != before;
        self.changed |= moved;
        self.fresh |= moved;
        moved
    }

    /// Goes to `board` as a navigate does, so that back returns to the board it
    /// leaves. Fails if the page has no such board.
    pub fn go(&mut self, board: &str) -> Result<bool, String> {
        let id = self.board(board)?;
        let moved = self.player.go(id);
        self.changed |= moved;
        self.fresh |= moved;
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

    /// Scrolls the content under the stage point `x`, `y` by a wheel's
    /// `dx`, `dy` in CSS pixels. Returns whether anything moved.
    pub fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        let Placement {
            scale,
            x: left,
            y: top,
        } = self.placement;
        let scale = f64::from(scale);
        let moved = self.player.scroll_at(
            f64::from(x - left) / scale,
            f64::from(y - top) / scale,
            f64::from(dx) / scale,
            f64::from(dy) / scale,
        );
        self.changed |= moved;
        moved
    }

    /// Shows the prototype on the device `name` of the device table (`Custom`
    /// only where the page's device is `Custom`). Fails for another name.
    pub fn set_device(&mut self, name: &str) -> Result<(), String> {
        let listed = self.devices().iter().any(|(listed, _)| listed == name);
        if !listed {
            return Err(format!("`{name}` is not a device of this prototype"));
        }
        self.device = name.to_string();
        let (width, height) = self.page.screen(name).unwrap_or((0.0, 0.0));
        self.player.set_screen(f64::from(width), f64::from(height));
        self.changed = true;
        Ok(())
    }

    /// Sets how the screen is scaled on the stage: `fit`, `100` or `50`.
    pub fn set_scale(&mut self, name: &str) -> Result<(), String> {
        self.scale = match name {
            "fit" => Scale::Fit,
            "100" => Scale::Full,
            "50" => Scale::Half,
            other => return Err(format!("`{other}` is not a scale: write fit, 100 or 50")),
        };
        self.changed = true;
        Ok(())
    }

    /// The devices to pick from with the size of the screen of each as it
    /// would be shown, `None` for the device that has no screen.
    fn devices(&self) -> Vec<(String, Option<(f32, f32)>)> {
        devices::DEVICES
            .iter()
            .filter(|device| device.name != CUSTOM || self.page.device == CUSTOM)
            .map(|device| (device.name.to_string(), self.page.screen(device.name)))
            .collect()
    }

    /// The size of the frame `bounds`, the board's own size, is shown in: the
    /// screen of the device, or the board at its own size.
    fn framed(&self, bounds: &crate::layout::LayoutBox) -> (f32, f32) {
        self.page
            .screen(&self.device)
            .unwrap_or((bounds.width, bounds.height))
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
        // The same goes for a transition the click began.
        if std::mem::take(&mut self.fresh) {
            self.player.restart_transition(now);
        }
        // Waits that began at an event count from the first frame after it.
        self.player.restart_timers(now);
        let hinting = self.player.hint_until().is_some();
        let playing = |player: &Player| {
            let frame = player.frame();
            frame.transition.is_some() || !frame.variant_runs.is_empty() || player.gliding()
        };
        let moving = playing(&self.player);
        let before = self.player.frame();
        let effects = self.player.tick(now);
        let linked = effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenUrl { .. }));
        self.absorb(effects);
        // A wait that ran out acted: the board, an overlay, a variant or a
        // new transition is not what it was.
        let after = self.player.frame();
        let acted = linked
            || before.board != after.board
            || before.overlays != after.overlays
            || before.variant_swaps != after.variant_swaps
            || before.nested_swaps != after.nested_swaps
            || (after.transition.is_some() && after.transition != before.transition);
        let hint_ended = hinting && self.player.hint_until().is_none();
        let settled = moving && !playing(&self.player);
        self.changed |= hint_ended || settled || acted;
        let animating = self.player.hint_until().is_some()
            || !self.player.frame().variant_runs.is_empty()
            || self.player.gliding()
            || self
                .player
                .frame()
                .transition
                .is_some_and(|running| running.transition != Transition::Instant);
        let waiting = !self.player.timers().is_empty();
        if animating || (self.changed && waiting) {
            Tick::Animating
        } else if self.changed {
            Tick::Changed
        } else if waiting {
            Tick::Waiting
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

    /// The screen the prototype is shown on: `{"device": "iPhone 16", "scale":
    /// "fit", "devices": [{"name": "iPhone 16", "width": 393, "height": 852}]}`.
    /// `scale` is `fit`, `100` or `50`; `devices` lists the devices to pick
    /// from with the size of each one's screen as it would be shown, `null`
    /// for the device that has none.
    pub fn screen(&self) -> Value {
        json!({
            "device": self.device,
            "scale": self.scale.name(),
            "devices": self.devices().into_iter().map(|(name, size)| {
                json!({"name": name, "width": size.map(|size| size.0), "height": size.map(|size| size.1)})
            }).collect::<Vec<_>>(),
        })
    }

    /// Draws `board` at its own size in the middle of the stage, scaled down
    /// to fit, moved by `offset` CSS pixels and clipped to its own box.
    fn draw_board(
        &self,
        scene: &mut Scene,
        canvas: &Canvas,
        size: (f32, f32),
        board: &NodeId,
        offset: (f32, f32),
    ) {
        let playing = self.board_layers(scene, board);
        self.draw_board_with(scene, canvas, size, board, offset, playing.as_ref());
    }

    /// As `draw_board`, drawing `layers`, in the page coordinates of `board`,
    /// in place of the board when there are any.
    fn draw_board_with(
        &self,
        scene: &mut Scene,
        canvas: &Canvas,
        (width, height): (f32, f32),
        board: &NodeId,
        offset: (f32, f32),
        layers: Option<&Layers>,
    ) {
        let Some(bounds) = scene.board_bounds(board) else {
            return;
        };
        let frame = self.framed(&bounds);
        let scale = self.scale.of((width, height), frame);
        let at = (
            (width - frame.0 * scale) / 2.0 + offset.0,
            (height - frame.1 * scale) / 2.0 + offset.1,
        );
        let viewport = Viewport::at(
            bounds.x - at.0 / scale,
            bounds.y - at.1 / scale,
            scale,
            width,
            height,
        );
        canvas.save();
        canvas.clip_rect(
            skia_safe::Rect::from_xywh(at.0, at.1, frame.0 * scale, frame.1 * scale),
            None,
            None,
        );
        // A board that scrolls, or has layers that stay or stick, is drawn
        // from its layers, moved; any other from the picture the scene keeps.
        let view = self.player.scroll_view(*board, self.now);
        let scrolled = (!view.is_plain())
            .then(|| {
                let mut whole = match layers {
                    Some(layers) => Some(layers.clone()),
                    None => scene.layers_of_board(board, None),
                }?;
                scroll(&mut whole, &view);
                Some(whole)
            })
            .flatten();
        match scrolled.as_ref().or(layers) {
            Some(layers) => scene.draw_layers(canvas, &viewport, layers),
            None => {
                scene.render_board(canvas, &viewport, board);
            }
        }
        canvas.restore();
    }

    /// `board` with the instances that smart animate a change of variant
    /// drawn as they are now, or `None` when none does.
    fn board_layers(&self, scene: &Scene, board: &NodeId) -> Option<Layers> {
        let runs: Vec<VariantRun> = self
            .player
            .frame()
            .variant_runs
            .into_iter()
            .filter(|run| run.board == *board && self.snapshots.contains_key(&run.id))
            .collect();
        if runs.is_empty() {
            return None;
        }
        let mut whole = scene.layers_of_board(board, None)?;
        for run in runs {
            let Some(instance) = instance_layer(scene, board, &run.instance) else {
                continue;
            };
            let Some(to) = scene.layers_of_board(board, Some(&instance)) else {
                continue;
            };
            let eased = run.progress(self.now).t;
            let switched = self.now - run.started >= duration_of(&run) / 2.0;
            let mixed = smart_animate(self.snapshots[&run.id].1.clone(), to, eased, switched);
            splice(&mut whole.node, &instance, &mut Some(mixed.node));
            whole.boxes.extend(mixed.boxes);
            whole.texts.extend(mixed.texts);
        }
        Some(whole)
    }

    /// Takes the instances that start a smart-animated change of variant as
    /// the last frame drew them, before the change is drawn: as the scene has
    /// them, or, where the change cut short another on the same instance, as
    /// that one had mixed them.
    fn capture(&mut self, scene: &Scene) {
        let runs = self.player.frame().variant_runs;
        let (kept, mut cut): (HashMap<_, _>, HashMap<_, _>) = std::mem::take(&mut self.snapshots)
            .into_iter()
            .partition(|(id, _)| runs.iter().any(|run| run.id == *id));
        self.snapshots = kept;
        for run in runs {
            // The run as the player times it now, for a later change to cut
            // it short at.
            if let Some(held) = self.snapshots.get_mut(&run.id) {
                held.0 = run;
                continue;
            }
            let Some(layers) = instance_layer(scene, &run.board, &run.instance)
                .and_then(|instance| scene.layers_of_board(&run.board, Some(&instance)))
            else {
                continue;
            };
            let before = cut
                .iter()
                .find(|(_, (other, _))| other.board == run.board && other.instance == run.instance)
                .map(|(id, _)| *id);
            let layers = match before.and_then(|id| cut.remove(&id)) {
                Some((other, from)) => {
                    let eased = other.progress(self.drawn).t;
                    let switched = self.drawn - other.started >= duration_of(&other) / 2.0;
                    smart_animate(from, layers, eased, switched)
                }
                None => layers,
            };
            self.snapshots.insert(run.id, (run, layers));
        }
    }

    /// Draws a change of board `now` moments in: the board it changes from, as
    /// it is (or as the transition it cut short had come), and the board it
    /// changes to moved or faded over it. The directions name the way a board
    /// moves; a board that does not move stays where it rests.
    fn draw_running(
        &self,
        scene: &mut Scene,
        canvas: &Canvas,
        size: (f32, f32),
        running: &Running,
        now: f64,
    ) {
        let eased = progress(running, now).t as f32;
        // How far a board has come in or gone out along a direction.
        let inbound = |direction| {
            let (x, y) = step(direction);
            (-x * size.0 * (1.0 - eased), -y * size.1 * (1.0 - eased))
        };
        let outbound = |direction| {
            let (x, y) = step(direction);
            (x * size.0 * eased, y * size.1 * eased)
        };
        let incoming = |this: &Session, scene: &mut Scene, canvas: &Canvas, offset| {
            this.draw_board(scene, canvas, size, &running.to, offset);
        };
        // A smart animate that cut another short has no layers of its own to
        // match the old board against: it dissolves.
        let transition = match running.transition {
            Transition::SmartAnimate if running.interrupted.is_some() => Transition::Dissolve,
            other => other,
        };
        match transition {
            Transition::Dissolve => {
                self.draw_outgoing(scene, canvas, size, running, (0.0, 0.0));
                let alpha = eased.clamp(0.0, 1.0);
                if alpha > 0.0 {
                    canvas.save_layer_alpha_f(None, alpha);
                    incoming(self, scene, canvas, (0.0, 0.0));
                    canvas.restore();
                }
            }
            Transition::MoveIn { direction } | Transition::SlideIn { direction } => {
                self.draw_outgoing(scene, canvas, size, running, (0.0, 0.0));
                incoming(self, scene, canvas, inbound(direction));
            }
            Transition::MoveOut { direction } | Transition::SlideOut { direction } => {
                // The board that leaves is on top of the one beneath it.
                incoming(self, scene, canvas, (0.0, 0.0));
                self.draw_outgoing(scene, canvas, size, running, outbound(direction));
            }
            Transition::Push { direction } => {
                self.draw_outgoing(scene, canvas, size, running, outbound(direction));
                incoming(self, scene, canvas, inbound(direction));
            }
            Transition::SmartAnimate => {
                if !self.draw_smart(scene, canvas, size, running, now) {
                    incoming(self, scene, canvas, (0.0, 0.0));
                }
            }
            Transition::Instant => {
                incoming(self, scene, canvas, (0.0, 0.0));
            }
        }
    }

    /// Draws the layers that the two boards of a smart animate share, moved
    /// and restyled between them, and the others faded. Returns whether it
    /// could.
    fn draw_smart(
        &self,
        scene: &mut Scene,
        canvas: &Canvas,
        size: (f32, f32),
        running: &Running,
        now: f64,
    ) -> bool {
        let (Some(from), Some(to)) = (
            scene.layers_of_board(&running.from, None),
            scene.layers_of_board(&running.to, None),
        ) else {
            return false;
        };
        let eased = progress(running, now).t;
        let switched = now - running.started >= duration_ms(running) / 2.0;
        let mixed = smart_animate(from, to, eased, switched);
        self.draw_board_with(scene, canvas, size, &running.to, (0.0, 0.0), Some(&mixed));
        true
    }

    /// Draws what a change of board changes from, moved by `offset`: the board
    /// it leaves, or the frame of the transition it cut short as that stood.
    fn draw_outgoing(
        &self,
        scene: &mut Scene,
        canvas: &Canvas,
        size: (f32, f32),
        running: &Running,
        offset: (f32, f32),
    ) {
        match &running.interrupted {
            Some(before) if before.running.transition != Transition::Instant => {
                canvas.save();
                canvas.translate(offset);
                self.draw_running(scene, canvas, size, &before.running, before.at);
                canvas.restore();
            }
            _ => self.draw_board(scene, canvas, size, &running.from, offset),
        }
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
        self.capture(scene);
        self.drawn = self.now;
        self.refresh(scene);
        self.changed = false;
        canvas.clear(self.background);
        let frame = self.player.frame();
        let Some(bounds) = scene.board_bounds(&frame.board) else {
            return;
        };
        let shown = self.framed(&bounds);
        let scale = self.scale.of((width, height), shown);
        let left = (width - shown.0 * scale) / 2.0;
        let top = (height - shown.1 * scale) / 2.0;
        self.player.set_scale(f64::from(scale));
        self.placement = Placement {
            scale,
            x: left,
            y: top,
        };
        // The device's colour shows as a band around its screen.
        if let Some(band) = self
            .page
            .screen(&self.device)
            .and(self.page.band(&self.device))
        {
            let around = BAND_WIDTH * scale;
            canvas.draw_rrect(
                skia_safe::RRect::new_rect_xy(
                    skia_safe::Rect::from_xywh(left, top, shown.0 * scale, shown.1 * scale)
                        .with_outset((around, around)),
                    around * 2.0,
                    around * 2.0,
                ),
                &skia_safe::Paint::new(skia_safe::Color4f::from(band), None),
            );
            canvas.draw_rect(
                skia_safe::Rect::from_xywh(left, top, shown.0 * scale, shown.1 * scale),
                &skia_safe::Paint::new(skia_safe::Color4f::from(self.background), None),
            );
        }
        match &frame.transition {
            Some(running) if running.transition != Transition::Instant => {
                self.draw_running(scene, canvas, (width, height), running, self.now);
            }
            _ => self.draw_board(scene, canvas, (width, height), &frame.board, (0.0, 0.0)),
        }
        let overlays = frame.overlays.iter().map(|overlay| {
            (
                overlay.board,
                overlay.origin,
                overlay.background.as_deref().and_then(color),
            )
        });
        for (board, origin, sheet) in overlays {
            if let Some(sheet) = sheet {
                canvas.draw_rect(
                    skia_safe::Rect::from_xywh(left, top, shown.0 * scale, shown.1 * scale),
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
            let playing = self.board_layers(scene, &board);
            // An overlay does not leave the screen.
            canvas.save();
            if self.page.screen(&self.device).is_some() {
                canvas.clip_rect(
                    skia_safe::Rect::from_xywh(left, top, shown.0 * scale, shown.1 * scale),
                    None,
                    None,
                );
            }
            match playing {
                Some(layers) => scene.draw_layers(canvas, &viewport, &layers),
                None => {
                    scene.render_board(canvas, &viewport, &board);
                }
            }
            canvas.restore();
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

/// Moves the layers of `layers` as `view` has them scrolled: each layer and
/// what is in it by its shift, the fixed and stuck layers to the end of their
/// container so that they draw above all that scrolls in it, and the
/// scrolling frames clipping.
fn scroll(layers: &mut Layers, view: &ScrollView) {
    fn visit(
        node: &mut Node,
        view: &ScrollView,
        inherited: [f64; 2],
        boxes: &mut HashMap<NodeId, LayoutBox>,
    ) {
        let shift = view.shifts.get(node.id()).copied().unwrap_or(inherited);
        if let (true, Some(layout)) = (shift != [0.0, 0.0], boxes.get_mut(node.id())) {
            layout.x += shift[0] as f32;
            layout.y += shift[1] as f32;
        }
        if view.clipped.contains(node.id()) {
            if let NodeData::Frame(frame) = node.data_mut() {
                frame.clip = Some(true);
            }
        }
        if let Some(children) = node.children_mut() {
            for child in children.iter_mut() {
                visit(child, view, shift, boxes);
            }
        }
    }
    /// Takes the layer `id` out of the children of `node` or below it, with
    /// whether a layer around it is hidden.
    fn take(node: &mut Node, id: &NodeId) -> Option<(Node, bool)> {
        let hidden = !node.visible();
        let children = node.children_mut()?;
        if let Some(at) = children.iter().position(|child| child.id() == id) {
            return Some((children.remove(at), hidden));
        }
        children
            .iter_mut()
            .find_map(|child| take(child, id))
            .map(|(layer, inner)| (layer, hidden || inner))
    }
    fn find<'a>(node: &'a mut Node, id: &NodeId) -> Option<&'a mut Node> {
        if node.id() == id {
            return Some(node);
        }
        node.children_mut()?
            .iter_mut()
            .find_map(|child| find(child, id))
    }
    visit(&mut layers.node, view, [0.0, 0.0], &mut layers.boxes);
    // The boxes are in page coordinates, so a layer keeps its place when it
    // moves in the tree.
    for (id, target) in &view.raised {
        let Some((layer, hidden)) = take(&mut layers.node, id) else {
            continue;
        };
        // A layer in a hidden one is not drawn where it goes either.
        if hidden {
            continue;
        }
        if let Some(children) = find(&mut layers.node, target).and_then(Node::children_mut) {
            children.push(layer);
        }
    }
}

/// The layer `root` of a board with everything below it, as the scene had it
/// drawn: its boxes in page coordinates and its shaped texts.
#[derive(Clone)]
pub(crate) struct Layers {
    pub(crate) node: Node,
    pub(crate) boxes: HashMap<NodeId, LayoutBox>,
    pub(crate) texts: ShapedTexts,
}

/// How long `run` takes, in milliseconds.
fn duration_of(run: &VariantRun) -> f64 {
    f64::from(run.easing.spring_duration_ms().unwrap_or(run.duration_ms))
}

/// The ID that the instance at `address` of `board` has in the board as the
/// scene resolved it.
fn instance_layer(scene: &Scene, board: &NodeId, address: &NodeAddress) -> Option<NodeId> {
    if address.instance().is_none() {
        return Some(*address.node());
    }
    scene
        .board_boxes(board)?
        .1
        .into_iter()
        .find_map(|(id, held)| (held == *address).then_some(id))
}

/// Puts `with` in place of the node `id` of `root` or below it.
fn splice(root: &mut Node, id: &NodeId, with: &mut Option<Node>) -> bool {
    if root.id() == id {
        if let Some(with) = with.take() {
            *root = with;
        }
        return true;
    }
    let Some(children) = root.children_mut() else {
        return false;
    };
    children.iter_mut().any(|child| splice(child, id, with))
}

/// What smart animate mixes of a layer besides its box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub opacity: Option<TokenOr<f64>>,
    pub radius: Option<Radius>,
    pub fills: Option<Vec<Fill>>,
    pub strokes: Option<Vec<Stroke>>,
    /// The colour of a Text.
    pub text_color: Option<TokenOr<DesignColor>>,
}

impl Style {
    /// The style of `node`: the properties its kind has.
    pub fn of(node: &Node) -> Style {
        match node.data() {
            NodeData::Board(frame) | NodeData::Frame(frame) => Style {
                opacity: frame.opacity.clone(),
                radius: frame.radius.clone(),
                fills: frame.fills.clone(),
                strokes: frame.strokes.clone(),
                text_color: None,
            },
            NodeData::Text(text) => Style {
                opacity: text.opacity.clone(),
                text_color: text.color.clone(),
                ..Style::default()
            },
            NodeData::Image(image) => Style {
                opacity: image.opacity.clone(),
                radius: image.radius.clone(),
                strokes: image.strokes.clone(),
                ..Style::default()
            },
            NodeData::Vector(vector) => Style {
                opacity: vector.opacity.clone(),
                fills: vector.fills.clone(),
                strokes: vector.strokes.clone(),
                ..Style::default()
            },
            _ => Style::default(),
        }
    }

    /// Gives `node` this style, for the properties its kind has.
    pub fn set(self, node: &mut Node) {
        match node.data_mut() {
            NodeData::Board(frame) | NodeData::Frame(frame) => {
                frame.opacity = self.opacity;
                frame.radius = self.radius;
                frame.fills = self.fills;
                frame.strokes = self.strokes;
            }
            NodeData::Text(text) => {
                text.opacity = self.opacity;
                text.color = self.text_color;
            }
            NodeData::Image(image) => {
                image.opacity = self.opacity;
                image.radius = self.radius;
                image.strokes = self.strokes;
            }
            NodeData::Vector(vector) => {
                vector.opacity = self.opacity;
                vector.fills = self.fills;
                vector.strokes = self.strokes;
            }
            _ => {}
        }
    }
}

fn lerp(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t
}

fn lerp_box(from: LayoutBox, to: LayoutBox, t: f64) -> LayoutBox {
    let mix = |from: f32, to: f32| lerp(f64::from(from), f64::from(to), t) as f32;
    LayoutBox {
        x: mix(from.x, to.x),
        y: mix(from.y, to.y),
        width: mix(from.width, to.width),
        height: mix(from.height, to.height),
    }
}

/// The straight (not premultiplied) red, green, blue and alpha of `color`,
/// from 0 to 255.
fn channels(color: &DesignColor) -> Option<[f64; 4]> {
    let hex = color.as_str().strip_prefix('#')?;
    let channel = |at: usize| {
        Some(f64::from(
            u8::from_str_radix(hex.get(at * 2..at * 2 + 2)?, 16).ok()?,
        ))
    };
    let alpha = if hex.len() == 8 { channel(3)? } else { 255.0 };
    Some([channel(0)?, channel(1)?, channel(2)?, alpha])
}

/// The colour `t` of the way from `from` to `to`, mixed channel by channel.
/// `None` when either is a token or not a colour.
fn mix_color(
    from: &TokenOr<DesignColor>,
    to: &TokenOr<DesignColor>,
    t: f64,
) -> Option<TokenOr<DesignColor>> {
    let (TokenOr::Literal(from), TokenOr::Literal(to)) = (from, to) else {
        return None;
    };
    let [fr, fg, fb, fa] = channels(from)?;
    let [tr, tg, tb, ta] = channels(to)?;
    let part = |from: f64, to: f64| lerp(from, to, t).round().clamp(0.0, 255.0) as u8;
    let (r, g, b, a) = (part(fr, tr), part(fg, tg), part(fb, tb), part(fa, ta));
    let opaque = from.as_str().len() == 7 && to.as_str().len() == 7;
    let text = if opaque {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    };
    DesignColor::from_str(&text).ok().map(TokenOr::Literal)
}

/// A number `t` of the way from `from` to `to`; `None` when either is a token.
fn mix_number(from: &TokenOr<f64>, to: &TokenOr<f64>, t: f64) -> Option<TokenOr<f64>> {
    match (from, to) {
        (TokenOr::Literal(from), TokenOr::Literal(to)) => {
            Some(TokenOr::Literal(lerp(*from, *to, t)))
        }
        _ => None,
    }
}

/// `from` or `to`, whichever the switch at the middle has come to.
fn pick<T: Clone>(from: &T, to: &T, switched: bool) -> T {
    if switched { to } else { from }.clone()
}

fn mix_fills(from: &[Fill], to: &[Fill], t: f64, switched: bool) -> Vec<Fill> {
    if from.len() != to.len() {
        return pick(&from.to_vec(), &to.to_vec(), switched);
    }
    from.iter()
        .zip(to)
        .map(|pair| match pair {
            (Fill::Solid { color: from }, Fill::Solid { color: to }) => Fill::Solid {
                color: mix_color(from, to, t).unwrap_or_else(|| pick(from, to, switched)),
            },
            (from, to) => pick(from, to, switched),
        })
        .collect()
}

fn mix_strokes(from: &[Stroke], to: &[Stroke], t: f64, switched: bool) -> Vec<Stroke> {
    if from.len() != to.len() {
        return pick(&from.to_vec(), &to.to_vec(), switched);
    }
    from.iter()
        .zip(to)
        .map(|(from, to)| {
            let mut stroke = pick(from, to, switched);
            stroke.color = mix_color(&from.color, &to.color, t).unwrap_or(stroke.color);
            stroke.width = mix_number(&from.width, &to.width, t).unwrap_or(stroke.width);
            stroke
        })
        .collect()
}

fn mix_radius(from: &Radius, to: &Radius, t: f64, switched: bool) -> Radius {
    let zero = TokenOr::Literal(0.0);
    let corner = |from: &Option<TokenOr<f64>>, to: &Option<TokenOr<f64>>| {
        let (from, to) = (from.as_ref().unwrap_or(&zero), to.as_ref().unwrap_or(&zero));
        Some(mix_number(from, to, t).unwrap_or_else(|| pick(from, to, switched)))
    };
    Radius {
        top_left: corner(&from.top_left, &to.top_left),
        top_right: corner(&from.top_right, &to.top_right),
        bottom_right: corner(&from.bottom_right, &to.bottom_right),
        bottom_left: corner(&from.bottom_left, &to.bottom_left),
    }
}

/// A layer `t` of the way from one board to the other, in the box and the
/// style that a smart animate has it in: the position, size, opacity, radius,
/// fills, strokes and text colour mixed by `t`, the eased progress. What
/// cannot be mixed (a fill against a fill of another kind, fills or strokes
/// of another number, a token that is not resolved) is the old layer's until
/// `t` passes one half and the new layer's after.
pub fn interpolate(
    from_box: LayoutBox,
    to_box: LayoutBox,
    from_style: &Style,
    to_style: &Style,
    t: f64,
) -> (LayoutBox, Style) {
    interpolate_at(from_box, to_box, from_style, to_style, t, t >= 0.5)
}

/// As [`interpolate`], with `switched` saying whether what cannot be mixed
/// has come to the new layer's.
pub fn interpolate_at(
    from_box: LayoutBox,
    to_box: LayoutBox,
    from_style: &Style,
    to_style: &Style,
    t: f64,
    switched: bool,
) -> (LayoutBox, Style) {
    let opaque = TokenOr::Literal(1.0);
    let opacity = match (&from_style.opacity, &to_style.opacity) {
        (None, None) => None,
        (from, to) => {
            let (from, to) = (
                from.as_ref().unwrap_or(&opaque),
                to.as_ref().unwrap_or(&opaque),
            );
            Some(mix_number(from, to, t).unwrap_or_else(|| pick(from, to, switched)))
        }
    };
    let radius = match (&from_style.radius, &to_style.radius) {
        (None, None) => None,
        (from, to) => {
            let none = Radius::default();
            Some(mix_radius(
                from.as_ref().unwrap_or(&none),
                to.as_ref().unwrap_or(&none),
                t,
                switched,
            ))
        }
    };
    let fills = match (&from_style.fills, &to_style.fills) {
        (None, None) => None,
        (Some(from), Some(to)) => Some(mix_fills(from, to, t, switched)),
        (from, to) => pick(from, to, switched),
    };
    let strokes = match (&from_style.strokes, &to_style.strokes) {
        (None, None) => None,
        (Some(from), Some(to)) => Some(mix_strokes(from, to, t, switched)),
        (from, to) => pick(from, to, switched),
    };
    let text_color = match (&from_style.text_color, &to_style.text_color) {
        (None, None) => None,
        (from, to) => {
            let black =
                TokenOr::Literal(DesignColor::from_str("#000000").expect("black is a colour"));
            let (from, to) = (
                from.as_ref().unwrap_or(&black),
                to.as_ref().unwrap_or(&black),
            );
            Some(mix_color(from, to, t).unwrap_or_else(|| pick(from, to, switched)))
        }
    };
    (
        lerp_box(from_box, to_box, t),
        Style {
            opacity,
            radius,
            fills,
            strokes,
            text_color,
        },
    )
}

/// Whether two layers of one name can be one layer that changes: boards and
/// frames can, and other kinds only with their own.
fn alike(a: &Node, b: &Node) -> bool {
    let group = |node: &Node| match node.kind() {
        NodeKind::Board | NodeKind::Frame | NodeKind::Component => NodeKind::Frame,
        other => other,
    };
    group(a) == group(b)
}

/// The boxes and shaped texts of one side of a smart animate.
struct Side {
    boxes: HashMap<NodeId, LayoutBox>,
    texts: ShapedTexts,
}

/// The two sides of a smart animate and what it has made so far.
struct Mixer {
    from: Side,
    to: Side,
    /// Where the roots are, which the boxes inside are measured from.
    from_origin: (f32, f32),
    to_origin: (f32, f32),
    /// The layers of the new side by the layer of the old side that they pair
    /// with.
    pairs: HashMap<NodeId, NodeId>,
    eased: f64,
    switched: bool,
    boxes: HashMap<NodeId, LayoutBox>,
    texts: ShapedTexts,
}

/// The layers that two boards of a smart animate (or the two variants of an
/// instance) make `eased` of the way from `from` to `to`, in the page
/// coordinates of `to`: a layer on both moves and restyles, one that is only
/// on `from` fades out where its parent carries it, and one that is only on
/// `to` fades in. The roots are drawn as one layer in the box of the new
/// root. `switched` says whether the halfway point has passed, from which what
/// cannot be mixed is the new layer's. The faded layers of the old side paint
/// beneath the layers of the new side, which keep its order.
pub(crate) fn smart_animate(from: Layers, to: Layers, eased: f64, switched: bool) -> Layers {
    if !alike(&from.node, &to.node) {
        return to;
    }
    let origin = |layers: &Layers| {
        layers
            .boxes
            .get(layers.node.id())
            .map_or((0.0, 0.0), |held| (held.x, held.y))
    };
    let (from_origin, to_origin) = (origin(&from), origin(&to));
    let pairs = match_layers(&from.node, &to.node).into_iter().collect();
    let (old, new) = (from.node, to.node);
    let mut mixer = Mixer {
        from: Side {
            boxes: from.boxes,
            texts: from.texts,
        },
        to: Side {
            boxes: to.boxes,
            texts: to.texts,
        },
        from_origin,
        to_origin,
        pairs,
        eased,
        switched,
        boxes: HashMap::new(),
        texts: ShapedTexts::new(),
    };
    let node = mixer.merge(old, new, true);
    Layers {
        node,
        boxes: mixer.boxes,
        texts: mixer.texts,
    }
}

impl Mixer {
    /// The one layer that `old` and `new` make, with the children of both.
    fn merge(&mut self, mut old: Node, mut new: Node, root: bool) -> Node {
        let old_box = self.from.boxes.get(old.id()).copied().unwrap_or_default();
        let new_box = self.to.boxes.get(new.id()).copied().unwrap_or_default();
        let inside = |laid: LayoutBox, origin: (f32, f32)| LayoutBox {
            x: laid.x - origin.0,
            y: laid.y - origin.1,
            ..laid
        };
        let (laid, style) = interpolate_at(
            inside(old_box, self.from_origin),
            inside(new_box, self.to_origin),
            &Style::of(&old),
            &Style::of(&new),
            self.eased,
            self.switched,
        );
        let placed = if root {
            new_box
        } else {
            LayoutBox {
                x: laid.x + self.to_origin.0,
                y: laid.y + self.to_origin.1,
                ..laid
            }
        };
        let old_children = old.take_children();
        let mut new_children: Vec<Option<Node>> =
            new.take_children().into_iter().map(Some).collect();
        // The layer keeps what is not mixed from the side the halfway point
        // has come to.
        let (mut node, texts) = if self.switched {
            (new, &self.to.texts)
        } else {
            (old, &self.from.texts)
        };
        if let Some(shaped) = texts.get(node.id()) {
            self.texts.insert(*node.id(), shaped.clone());
        }
        style.set(&mut node);
        self.boxes.insert(*node.id(), placed);

        let partner = |this: &Mixer, old: &Node, new: &[Option<Node>]| {
            let wanted = this.pairs.get(old.id())?;
            let at = new
                .iter()
                .position(|held| held.as_ref().is_some_and(|new| new.id() == wanted))?;
            let new = new[at].as_ref()?;
            (alike(old, new) && old.visible() && new.visible()).then_some(at)
        };
        let partners: Vec<Option<usize>> = old_children
            .iter()
            .map(|old| partner(self, old, &new_children))
            .collect();
        let mut children = Vec::new();
        let gone = (placed.x - old_box.x, placed.y - old_box.y);
        let mut mates: HashMap<usize, Node> = HashMap::new();
        for (old, at) in old_children.into_iter().zip(partners) {
            match at {
                Some(at) => {
                    mates.insert(at, old);
                }
                None => children.extend(self.fade(old, true, gone)),
            }
        }
        let come = (placed.x - new_box.x, placed.y - new_box.y);
        for (at, new) in new_children.iter_mut().enumerate() {
            let Some(new) = new.take() else { continue };
            match mates.remove(&at) {
                Some(old) => {
                    let merged = self.merge(old, new, false);
                    children.push(merged);
                }
                None => children.extend(self.fade(new, false, come)),
            }
        }
        node.set_children(children);
        node
    }

    /// `layer` and what is below it moved by `by`, faded out if `leaving` and
    /// in otherwise, as far as the progress has come. `None` for a layer that
    /// is hidden.
    fn fade(&mut self, mut layer: Node, leaving: bool, by: (f32, f32)) -> Option<Node> {
        if !layer.visible() {
            return None;
        }
        let alpha = if leaving {
            1.0 - self.eased
        } else {
            self.eased
        }
        .clamp(0.0, 1.0);
        let mut style = Style::of(&layer);
        let held = match &style.opacity {
            Some(TokenOr::Literal(held)) => *held,
            _ => 1.0,
        };
        style.opacity = Some(TokenOr::Literal(held * alpha));
        style.set(&mut layer);
        let side = if leaving { &self.from } else { &self.to };
        let mut pending = vec![&layer];
        while let Some(at) = pending.pop() {
            if let Some(laid) = side.boxes.get(at.id()) {
                self.boxes.insert(
                    *at.id(),
                    LayoutBox {
                        x: laid.x + by.0,
                        y: laid.y + by.1,
                        ..*laid
                    },
                );
            }
            if let Some(shaped) = side.texts.get(at.id()) {
                self.texts.insert(*at.id(), shaped.clone());
            }
            pending.extend(at.children());
        }
        Some(layer)
    }
}
