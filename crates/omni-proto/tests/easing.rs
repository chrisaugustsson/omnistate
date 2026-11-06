//! US-32: the easing curves and springs a transition plays with
//! (`omni_proto::easing::Easing`), and how far a running transition has come
//! (`omni_proto::transition::progress`).
//!
//! What the tests expect, from the story:
//!
//! - Every named easing is 0 at 0 and exactly 1 at 1, springs too, so a board
//!   lands exactly in place.
//! - easeIn, easeOut and easeInOut are the CSS curves: easeIn(0.5) < 0.5 <
//!   easeOut(0.5), easeInOut(0.5) = 0.5, and easeIn is the cubic Bézier
//!   (0.42, 0, 1, 1). easeOutBack overshoots 1.
//! - gentle, quick, bouncy and slow are springs, as customSpring is: their
//!   length, the last millisecond more than 0.1% away from 1 plus 1 and at most
//!   10 s, replaces the transition's `durationMs`. A curve has no such length.

use omni_core::design::NodeId;
use omni_core::design::proto::Transition;
use omni_proto::Running;
use omni_proto::easing::Easing;
use omni_proto::transition::{Progress, progress};

const NAMED: [Easing; 11] = [
    Easing::Linear,
    Easing::EaseIn,
    Easing::EaseOut,
    Easing::EaseInOut,
    Easing::EaseInBack,
    Easing::EaseOutBack,
    Easing::EaseInOutBack,
    Easing::Gentle,
    Easing::Quick,
    Easing::Bouncy,
    Easing::Slow,
];

const SPRING: Easing = Easing::CustomSpring {
    mass: 1.0,
    stiffness: 100.0,
    damping: 15.0,
};

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

fn running(easing: Easing, duration_ms: u32) -> Running {
    Running {
        from: id("b0000001"),
        to: id("b0000002"),
        transition: Transition::Dissolve,
        reversed: false,
        started: 1000.0,
        duration_ms,
        easing,
        interrupted: None,
    }
}

#[test]
fn every_named_easing_starts_at_0_and_ends_at_exactly_1() {
    let at_start: Vec<f64> = NAMED.iter().map(|easing| easing.eval(0.0)).collect();
    let at_end: Vec<f64> = NAMED.iter().map(|easing| easing.eval(1.0)).collect();

    assert_eq!(at_start, vec![0.0; NAMED.len()]);
    assert_eq!(at_end, vec![1.0; NAMED.len()]);
}

#[test]
fn ease_in_lags_and_ease_out_leads_and_ease_in_out_is_half_way_at_the_middle() {
    assert!(
        Easing::EaseIn.eval(0.5) < 0.5,
        "{}",
        Easing::EaseIn.eval(0.5)
    );
    assert!(
        Easing::EaseOut.eval(0.5) > 0.5,
        "{}",
        Easing::EaseOut.eval(0.5)
    );
    assert!((Easing::EaseInOut.eval(0.5) - 0.5).abs() <= 0.001);
}

#[test]
fn a_custom_bezier_of_0_42_0_1_1_is_ease_in() {
    let bezier = Easing::CustomBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };

    assert!((bezier.eval(0.25) - Easing::EaseIn.eval(0.25)).abs() <= 0.001);
    assert!((bezier.eval(0.5) - Easing::EaseIn.eval(0.5)).abs() <= 0.001);
    assert!((bezier.eval(0.75) - Easing::EaseIn.eval(0.75)).abs() <= 0.001);
}

#[test]
fn ease_out_back_overshoots_1_before_it_ends() {
    let highest = (1..100)
        .map(|step| Easing::EaseOutBack.eval(f64::from(step) / 100.0))
        .fold(f64::MIN, f64::max);

    assert!(highest > 1.0, "{highest}");
}

#[test]
fn the_presets_and_a_custom_spring_are_springs_and_the_curves_are_not() {
    let springs = [
        Easing::Gentle,
        Easing::Quick,
        Easing::Bouncy,
        Easing::Slow,
        SPRING,
    ];
    let curves = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::EaseInBack,
        Easing::EaseOutBack,
        Easing::EaseInOutBack,
    ];

    for spring in springs {
        assert!(spring.spring_duration_ms().is_some(), "{spring:?}");
    }
    for curve in curves {
        assert_eq!(curve.spring_duration_ms(), None, "{curve:?}");
    }
}

#[test]
fn a_spring_lasts_until_it_stays_within_a_tenth_of_a_percent_of_1() {
    // mass 1, stiffness 100, damping 15: ω = 10, ζ = 0.75, so it rings with an
    // envelope of 1.512 e^(−7.5 s), inside 1% after 0.67 s and inside 0.1%
    // after 0.977 s.
    let ms = SPRING.spring_duration_ms().expect("a spring has a length");
    let at = |ms_in: u32| SPRING.eval(f64::from(ms_in) / f64::from(ms));

    assert!(ms <= 977, "{ms}");
    let last_outside = (at(ms - 1) - 1.0).abs();
    assert!(
        last_outside > 0.001 && last_outside < 0.0015,
        "{last_outside}"
    );
    assert!(
        (670..ms).all(|ms_in| (at(ms_in) - 1.0).abs() < 0.01),
        "settled within 1% from 670 ms on"
    );
}

#[test]
fn an_undamped_spring_plays_for_10_s() {
    let undamped = Easing::CustomSpring {
        mass: 1.0,
        stiffness: 100.0,
        damping: 0.0,
    };

    assert_eq!(undamped.spring_duration_ms(), Some(10_000));
}

#[test]
fn a_springs_length_replaces_the_transitions_duration() {
    let ms = SPRING.spring_duration_ms().expect("a spring has a length");
    let spring = running(SPRING, 100);

    assert!(
        !progress(&spring, 1000.0 + 100.0).done,
        "durationMs is ignored"
    );
    assert!(!progress(&spring, 1000.0 + f64::from(ms) - 1.0).done);
    assert_eq!(
        progress(&spring, 1000.0 + f64::from(ms)),
        Progress { t: 1.0, done: true }
    );
}
