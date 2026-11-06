//! Where an overlay sits: the placement rules of FigCraft's
//! `OverlayAnchor::offset` (MIT OR Apache-2.0; see `NOTICE`), in the
//! player's space, where the base board's top-left corner is the origin.

use crate::player::Rect;
use omni_core::design::proto::{OverlayAnchor, OverlayPosition};

/// A point in the player's space.
pub type Point = [f64; 2];

#[derive(Clone, Copy)]
enum Side {
    Start,
    Middle,
    End,
}

/// Where the top-left corner of an overlay of `overlay_size` goes.
///
/// `screen` is the box of the base board and `trigger_box` that of the layer
/// that opened the overlay. `Manual` puts the overlay at `offset` from the
/// trigger's top-left corner, right below the trigger without one. The other
/// presets are measured on the screen, inside the trigger, or outside it
/// (above the trigger for the top presets, below it for the bottom ones, level
/// with it for `Center`), as `anchor` says.
pub fn position(
    preset: OverlayPosition,
    anchor: OverlayAnchor,
    offset: Option<[f64; 2]>,
    trigger_box: Rect,
    overlay_size: [f64; 2],
    screen: Rect,
) -> Point {
    let [width, height] = overlay_size;
    let (horizontal, vertical) = match preset {
        OverlayPosition::Manual => {
            return match offset {
                Some([x, y]) => [trigger_box.x + x, trigger_box.y + y],
                None => [trigger_box.x, trigger_box.y + trigger_box.height],
            };
        }
        OverlayPosition::Center => (Side::Middle, Side::Middle),
        OverlayPosition::TopLeft => (Side::Start, Side::Start),
        OverlayPosition::TopCenter => (Side::Middle, Side::Start),
        OverlayPosition::TopRight => (Side::End, Side::Start),
        OverlayPosition::BottomLeft => (Side::Start, Side::End),
        OverlayPosition::BottomCenter => (Side::Middle, Side::End),
        OverlayPosition::BottomRight => (Side::End, Side::End),
    };
    let along = |side: Side, start: f64, extent: f64, size: f64| match side {
        Side::Start => start,
        Side::Middle => start + (extent - size) / 2.0,
        Side::End => start + extent - size,
    };
    match anchor {
        OverlayAnchor::Screen | OverlayAnchor::TriggerInside => {
            let frame = if anchor == OverlayAnchor::Screen {
                screen
            } else {
                trigger_box
            };
            [
                along(horizontal, frame.x, frame.width, width),
                along(vertical, frame.y, frame.height, height),
            ]
        }
        OverlayAnchor::TriggerOutside => {
            let y = match vertical {
                Side::Start => trigger_box.y - height,
                Side::Middle => trigger_box.y + (trigger_box.height - height) / 2.0,
                Side::End => trigger_box.y + trigger_box.height,
            };
            [
                along(horizontal, trigger_box.x, trigger_box.width, width),
                y,
            ]
        }
    }
}
