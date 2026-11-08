//! The prototype player's state machine: which board a prototype shows, what
//! it can go back to, which overlays are open and which variant each instance
//! shows, changed by pointer and key events. It draws nothing and lays nothing out;
//! the caller gives it the boxes of the layers (`Player::set_boxes`).
//!
//! The behaviour is ported from FigCraft's `crates/engine/src/proto_run.rs`
//! (MIT OR Apache-2.0; see `NOTICE`).

pub mod easing;
mod history;
mod input;
pub mod keys;
pub mod matching;
pub mod overlay;
mod player;
pub mod scroll;
pub mod timers;
pub mod transition;
pub mod variables;

pub use history::Play;
pub use player::{
    Effect, Frame, HINT_MS, Interrupted, KeyEvent, OverlayState, Player, PointerEvent, PointerKind,
    Rect, Running, VariantRun,
};
pub use scroll::{ScrollState, ScrollView};
pub use timers::Timer;
