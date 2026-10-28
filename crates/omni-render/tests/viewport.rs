//! US-006: zooming and panning. A `Viewport` is the part of the page that the
//! screen shows (a zoom and the page point at the screen's top-left corner),
//! and `Controls` turns the page's pointer, wheel and key events into changes
//! of it. Both are plain Rust, so the tests drive them natively and the
//! WebAssembly page only forwards browser events (`view_sandbox.rs`).
//!
//! What the tests expect:
//!
//! - Screen points are CSS pixels from the canvas's top-left corner. A page
//!   point `p` is at screen `(p - origin) * zoom`.
//! - Zooming is centered on the pointer: the page point under it stays under
//!   it. Zoom stays between `MIN_ZOOM` and `MAX_ZOOM`, and the anchor holds
//!   even when the zoom is clamped.
//! - Ctrl or Cmd with the wheel zooms (a trackpad pinch reaches the page as
//!   exactly that): a negative `delta_y` zooms in, a positive one out, and
//!   zoom is exponential in the delta, so two small events zoom as much as
//!   one event of their sum, and opposite events cancel. A notch of a mouse
//!   wheel (`delta_y` of 100) changes the zoom by 5% to 100%.
//! - The wheel alone pans, by the delta in screen pixels: content moves
//!   against the scroll, so a positive `delta_y` shows what is further down.
//! - Dragging with the pointer pans only while the space key is held: the
//!   page point that was under the pointer stays under it.

use omni_render::layout::LayoutBox;
use omni_render::view::{Controls, MAX_ZOOM, MIN_ZOOM, Viewport, Wheel};

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;

/// How far a page point may drift, in page pixels.
const EPSILON: f32 = 0.02;

/// A viewport away from the origin and zoomed, where mistakes in the maths
/// do not cancel out.
fn somewhere() -> Viewport {
    Viewport::at(1200.0, 700.0, 0.5, WIDTH, HEIGHT)
}

#[track_caller]
fn assert_near(actual: f32, expected: f32, tolerance: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: expected {expected} (give or take {tolerance}), got {actual}"
    );
}

#[track_caller]
fn assert_point(actual: (f32, f32), expected: (f32, f32), tolerance: f32, what: &str) {
    assert_near(actual.0, expected.0, tolerance, &format!("{what} (x)"));
    assert_near(actual.1, expected.1, tolerance, &format!("{what} (y)"));
}

#[track_caller]
fn assert_ratio(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual / expected - 1.0).abs() < 1e-3,
        "{what}: expected {expected}, got {actual}"
    );
}

fn wheel(delta_y: f32, x: f32, y: f32) -> Wheel {
    Wheel {
        delta_y,
        x,
        y,
        ..Wheel::default()
    }
}

fn ctrl_wheel(delta_y: f32, x: f32, y: f32) -> Wheel {
    Wheel {
        ctrl: true,
        ..wheel(delta_y, x, y)
    }
}

fn controls() -> Controls {
    Controls::new(somewhere())
}

// ---------------------------------------------------------------------------
// The viewport
// ---------------------------------------------------------------------------

#[test]
fn screen_and_page_points_convert_by_the_zoom_and_the_origin() {
    let viewport = Viewport::at(100.0, 50.0, 2.0, WIDTH, HEIGHT);
    assert_eq!(viewport.zoom(), 2.0);
    assert_eq!(viewport.origin(), (100.0, 50.0));
    assert_point(
        viewport.to_screen((100.0, 50.0)),
        (0.0, 0.0),
        EPSILON,
        "the origin is at the screen's corner",
    );
    assert_point(
        viewport.to_screen((110.0, 60.0)),
        (20.0, 20.0),
        EPSILON,
        "10 page pixels are 20 screen pixels at 200%",
    );
    assert_point(
        viewport.to_design((20.0, 20.0)),
        (110.0, 60.0),
        EPSILON,
        "and back",
    );
    assert_eq!(
        viewport.visible_rect(),
        LayoutBox {
            x: 100.0,
            y: 50.0,
            width: WIDTH / 2.0,
            height: HEIGHT / 2.0
        },
        "at 200% the screen shows half as much of the page"
    );
}

#[test]
fn zooming_is_centered_on_the_anchor_so_the_page_point_under_it_stays() {
    let anchors = [
        (0.0, 0.0),
        (WIDTH, HEIGHT),
        (WIDTH / 2.0, HEIGHT / 2.0),
        (317.0, 640.0),
    ];
    for factor in [2.0, 1.25, 0.8, 0.1, 7.0] {
        for anchor in anchors {
            let mut viewport = somewhere();
            let under_pointer = viewport.to_design(anchor);
            let before = viewport.zoom();
            viewport.zoom_at(factor, anchor);
            assert_ratio(
                viewport.zoom(),
                before * factor,
                &format!("zoom by {factor} at {anchor:?}"),
            );
            assert_point(
                viewport.to_design(anchor),
                under_pointer,
                EPSILON,
                &format!("the page point under {anchor:?} after zooming by {factor}"),
            );
            assert_point(
                viewport.to_screen(under_pointer),
                anchor,
                EPSILON,
                &format!("and it is still drawn at {anchor:?}"),
            );
        }
    }
}

#[test]
fn zoom_is_kept_within_its_limits_and_the_anchor_still_holds() {
    let anchor = (500.0, 250.0);
    for (factor, limit) in [(1.0e9, MAX_ZOOM), (1.0e-9, MIN_ZOOM)] {
        let mut viewport = somewhere();
        let under_pointer = viewport.to_design(anchor);
        viewport.zoom_at(factor, anchor);
        assert_ratio(viewport.zoom(), limit, &format!("zoom by {factor}"));
        assert_point(
            viewport.to_design(anchor),
            under_pointer,
            EPSILON,
            &format!("the page point under the pointer when zoom stops at {limit}"),
        );
    }
}

#[test]
fn fit_puts_a_rectangle_wholly_in_view_as_large_as_it_goes_and_centered() {
    let rect = LayoutBox {
        x: 100.0,
        y: 200.0,
        width: 9600.0,
        height: 15400.0,
    };
    let mut viewport = somewhere();
    viewport.fit(rect);
    let visible = viewport.visible_rect();
    assert!(
        visible.x <= rect.x + EPSILON
            && visible.y <= rect.y + EPSILON
            && visible.x + visible.width >= rect.x + rect.width - EPSILON
            && visible.y + visible.height >= rect.y + rect.height - EPSILON,
        "the rectangle {rect:?} must be wholly in view, but the screen shows {visible:?}"
    );
    // 900 / 15400 is the tighter axis.
    assert_ratio(viewport.zoom(), HEIGHT / rect.height, "the zoom that fits");
    assert_point(
        (
            visible.x + visible.width / 2.0,
            visible.y + visible.height / 2.0,
        ),
        (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
        0.5,
        "the rectangle is centered",
    );
}

// ---------------------------------------------------------------------------
// Wheel, pinch and space-drag
// ---------------------------------------------------------------------------

#[test]
fn ctrl_or_cmd_with_the_wheel_zooms_around_the_pointer() {
    for (name, event) in [
        ("ctrl", ctrl_wheel(-100.0, 400.0, 300.0)),
        (
            "cmd",
            Wheel {
                meta: true,
                ..wheel(-100.0, 400.0, 300.0)
            },
        ),
    ] {
        let mut controls = controls();
        let under_pointer = controls.viewport().to_design((400.0, 300.0));
        let before = controls.viewport().zoom();
        assert!(
            controls.wheel(event),
            "{name} + wheel is used, so the browser must not zoom the page"
        );
        assert!(
            controls.viewport().zoom() > before,
            "{name} + wheel up (negative delta) must zoom in"
        );
        assert_point(
            controls.viewport().to_design((400.0, 300.0)),
            under_pointer,
            EPSILON,
            &format!("{name} + wheel keeps the page point under the pointer"),
        );

        let mut out = Controls::new(somewhere());
        out.wheel(Wheel {
            delta_y: 100.0,
            ..event
        });
        assert!(
            out.viewport().zoom() < before,
            "{name} + wheel down (positive delta) must zoom out"
        );
    }
}

#[test]
fn a_wheel_notch_zooms_smoothly_and_wheel_events_add_up() {
    let mut notch = controls();
    notch.wheel(ctrl_wheel(-100.0, 700.0, 450.0));
    let step = notch.viewport().zoom() / 0.5;
    assert!(
        (1.05..=2.0).contains(&step),
        "a notch of 100 must change the zoom by 5% to 100%, but it zoomed by {step}x"
    );

    let mut halves = controls();
    halves.wheel(ctrl_wheel(-50.0, 700.0, 450.0));
    halves.wheel(ctrl_wheel(-50.0, 700.0, 450.0));
    assert_ratio(
        halves.viewport().zoom(),
        notch.viewport().zoom(),
        "two events of -50 must zoom as much as one of -100",
    );

    let mut back = controls();
    back.wheel(ctrl_wheel(-70.0, 123.0, 456.0));
    back.wheel(ctrl_wheel(70.0, 123.0, 456.0));
    assert_ratio(back.viewport().zoom(), 0.5, "opposite events cancel");
    assert_point(
        back.viewport().origin(),
        (1200.0, 700.0),
        EPSILON,
        "and restore the view",
    );

    let mut nothing = controls();
    nothing.wheel(ctrl_wheel(0.0, 123.0, 456.0));
    assert_eq!(nothing.viewport().zoom(), 0.5, "no delta, no zoom");
}

#[test]
fn the_wheel_alone_pans_by_its_delta_and_never_zooms() {
    let mut controls = controls();
    assert!(
        controls.wheel(Wheel {
            delta_x: 20.0,
            delta_y: 40.0,
            x: 700.0,
            y: 450.0,
            ..Wheel::default()
        }),
        "a scroll is used, so the browser must not scroll the page"
    );
    assert_eq!(controls.viewport().zoom(), 0.5);
    // 20, 40 screen pixels at 50% are 40, 80 page pixels.
    assert_point(
        controls.viewport().origin(),
        (1200.0 + 40.0, 700.0 + 80.0),
        EPSILON,
        "scrolling down and right shows what is further down and right",
    );
}

#[test]
fn a_touch_pinch_zooms_by_its_scale_around_the_fingers() {
    let mut controls = controls();
    let under_fingers = controls.viewport().to_design((600.0, 400.0));
    assert!(controls.pinch(1.25, 600.0, 400.0));
    assert_ratio(controls.viewport().zoom(), 0.5 * 1.25, "spreading zooms in");
    assert_point(
        controls.viewport().to_design((600.0, 400.0)),
        under_fingers,
        EPSILON,
        "the page point between the fingers stays between them",
    );
    controls.pinch(0.8, 600.0, 400.0);
    assert_ratio(controls.viewport().zoom(), 0.5, "pinching in zooms out");
}

#[test]
fn dragging_with_space_held_pans_so_the_page_follows_the_pointer() {
    let mut controls = controls();
    let grabbed = controls.viewport().to_design((100.0, 100.0));
    controls.set_space(true);
    assert!(controls.pointer_down(100.0, 100.0));
    assert!(controls.pointer_move(130.0, 90.0), "a drag move is used");
    assert_point(
        controls.viewport().to_design((130.0, 90.0)),
        grabbed,
        EPSILON,
        "the page point that was grabbed is under the pointer",
    );
    assert!(controls.pointer_move(40.0, 200.0));
    assert_point(
        controls.viewport().to_design((40.0, 200.0)),
        grabbed,
        EPSILON,
        "and it keeps following",
    );
    assert_eq!(controls.viewport().zoom(), 0.5, "dragging does not zoom");

    controls.pointer_up();
    let after = *controls.viewport();
    assert!(
        !controls.pointer_move(300.0, 300.0),
        "moving the pointer after the drag is not used"
    );
    assert_eq!(*controls.viewport(), after, "and pans nothing");
}

#[test]
fn dragging_without_space_does_not_pan() {
    let mut controls = controls();
    let before = *controls.viewport();
    controls.pointer_down(100.0, 100.0);
    assert!(
        !controls.pointer_move(180.0, 160.0),
        "a drag without space is left to selection (US-007)"
    );
    controls.pointer_up();
    assert_eq!(*controls.viewport(), before);
}

#[test]
fn letting_go_of_space_ends_the_pan() {
    let mut controls = controls();
    controls.set_space(true);
    controls.pointer_down(100.0, 100.0);
    controls.pointer_move(120.0, 120.0);
    controls.set_space(false);
    let after = *controls.viewport();
    assert!(!controls.pointer_move(200.0, 200.0));
    assert_eq!(
        *controls.viewport(),
        after,
        "without space the pointer no longer pans"
    );
}
