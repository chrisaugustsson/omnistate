//! The prototype player's state machine: which board a prototype shows, what
//! it can go back to, which overlays are open and which variant each instance
//! shows, changed by pointer events. It draws nothing and lays nothing out;
//! the caller gives it the boxes of the layers (`Player::set_boxes`).
//!
//! The behaviour is ported from FigCraft's `crates/engine/src/proto_run.rs`
//! (MIT OR Apache-2.0; see `NOTICE`).

mod history;
mod player;

pub use history::Play;
pub use player::{Effect, Frame, OverlayState, Player, PointerEvent, PointerKind, Rect, Running};
