//! Where `overlay::position` puts an overlay, from US-35: the presets against
//! the screen's edges, `manual` at `offset` from the opening layer (right
//! below it without one), and the presets measured inside the opening layer.

use omni_core::design::proto::{OverlayAnchor, OverlayPosition};
use omni_proto::Rect;
use omni_proto::overlay::position;

const SCREEN: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 400.0,
    height: 800.0,
};

const TRIGGER: Rect = Rect {
    x: 50.0,
    y: 60.0,
    width: 80.0,
    height: 30.0,
};

#[test]
fn the_screen_presets_sit_against_the_screens_edges() {
    let at = |preset| {
        position(
            preset,
            OverlayAnchor::Screen,
            None,
            TRIGGER,
            [200.0, 100.0],
            SCREEN,
        )
    };

    assert_eq!(at(OverlayPosition::TopCenter), [100.0, 0.0]);
    assert_eq!(at(OverlayPosition::BottomRight), [200.0, 700.0]);
    assert_eq!(at(OverlayPosition::BottomLeft), [0.0, 700.0]);
}

#[test]
fn manual_sits_at_the_offset_from_the_trigger_or_right_below_it() {
    let manual = |offset| {
        position(
            OverlayPosition::Manual,
            OverlayAnchor::Screen,
            offset,
            TRIGGER,
            [200.0, 100.0],
            SCREEN,
        )
    };

    assert_eq!(manual(Some([10.0, 20.0])), [60.0, 80.0]);
    assert_eq!(manual(None), [50.0, 90.0]);
}

#[test]
fn inside_the_trigger_the_presets_are_measured_on_its_box() {
    let placed = position(
        OverlayPosition::TopRight,
        OverlayAnchor::TriggerInside,
        None,
        TRIGGER,
        [20.0, 10.0],
        SCREEN,
    );

    assert_eq!(placed, [110.0, 60.0]);
}
