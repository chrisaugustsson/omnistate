//! The maths of an easing: its curve for a progress between 0 and 1, and the
//! length of a spring.
//!
//! Ported from FigCraft's `Easing::eval` and `Spring`, crates/doc/src/proto.rs
//! (MIT OR Apache-2.0; see `NOTICE`).

use super::Easing;

/// Figma's spring presets: mass, stiffness and damping.
const GENTLE: (f64, f64, f64) = (1.0, 100.0, 15.0);
const QUICK: (f64, f64, f64) = (1.0, 300.0, 20.0);
const BOUNCY: (f64, f64, f64) = (1.0, 600.0, 15.0);
const SLOW: (f64, f64, f64) = (1.0, 80.0, 20.0);

/// How close to 1 a spring has to stay for good to have settled.
const SETTLED: f64 = 0.001;

/// The longest a spring runs, in milliseconds, however weakly it is damped.
const MAX_SPRING_MS: u32 = 10_000;

impl Easing {
    /// The eased progress for the linear progress `t` in 0..=1: 0 at 0 and
    /// exactly 1 at 1 for every easing. Back easings and springs leave 0..=1
    /// between the two.
    pub fn eval(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        match *self {
            Easing::Linear => t,
            Easing::EaseIn => bezier(0.42, 0.0, 1.0, 1.0, t),
            Easing::EaseOut => bezier(0.0, 0.0, 0.58, 1.0, t),
            Easing::EaseInOut => bezier(0.42, 0.0, 0.58, 1.0, t),
            Easing::EaseInBack => bezier(0.36, 0.0, 0.66, -0.56, t),
            Easing::EaseOutBack => bezier(0.34, 1.56, 0.64, 1.0, t),
            Easing::EaseInOutBack => bezier(0.68, -0.6, 0.32, 1.6, t),
            Easing::CustomBezier { x1, y1, x2, y2 } => bezier(x1, y1, x2, y2, t),
            _ => match self.spring() {
                Some(spring) => {
                    let ms = f64::from(self.spring_duration_ms().unwrap_or(0));
                    spring.at(t * ms / 1000.0)
                }
                None => t,
            },
        }
    }

    /// How long a spring plays, in milliseconds, which replaces a transition's
    /// `durationMs`: the time after which it stays within 0.1% of 1. `None`
    /// for a curve.
    pub fn spring_duration_ms(&self) -> Option<u32> {
        let spring = self.spring()?;
        let mut last_outside = 0u32;
        for ms in 0..=spring.scan_ms() {
            if (spring.at(f64::from(ms) / 1000.0) - 1.0).abs() > SETTLED {
                last_outside = ms;
            }
        }
        Some((last_outside + 1).min(MAX_SPRING_MS))
    }

    fn spring(&self) -> Option<Spring> {
        let (mass, stiffness, damping) = match *self {
            Easing::Gentle => GENTLE,
            Easing::Quick => QUICK,
            Easing::Bouncy => BOUNCY,
            Easing::Slow => SLOW,
            Easing::CustomSpring {
                mass,
                stiffness,
                damping,
            } => (mass, stiffness, damping),
            _ => return None,
        };
        Some(Spring {
            mass,
            stiffness,
            damping,
        })
    }
}

/// A damped spring released at rest from 0 towards 1.
struct Spring {
    mass: f64,
    stiffness: f64,
    damping: f64,
}

impl Spring {
    /// How many milliseconds to look at: past where an underdamped spring's
    /// envelope is inside `SETTLED`, nothing leaves it again.
    fn scan_ms(&self) -> u32 {
        let mass = self.mass.max(f64::EPSILON);
        let stiffness = self.stiffness.max(f64::EPSILON);
        let omega = (stiffness / mass).sqrt();
        let zeta = self.damping.max(0.0) / (2.0 * (stiffness * mass).sqrt());
        if zeta <= 0.0 || zeta >= 1.0 {
            return MAX_SPRING_MS;
        }
        let decay = zeta * omega;
        let ringing = omega * (1.0 - zeta * zeta).sqrt();
        let reach = (1.0 + (decay / ringing).powi(2)).sqrt();
        let seconds = (reach / SETTLED).ln() / decay;
        ((seconds * 1000.0).ceil() as u32 + 1).min(MAX_SPRING_MS)
    }

    /// The position after `seconds`.
    fn at(&self, seconds: f64) -> f64 {
        let mass = self.mass.max(f64::EPSILON);
        let omega = (self.stiffness.max(f64::EPSILON) / mass).sqrt();
        let zeta = self.damping.max(0.0) / (2.0 * (self.stiffness.max(f64::EPSILON) * mass).sqrt());
        if zeta < 1.0 {
            let decay = zeta * omega;
            let ringing = omega * (1.0 - zeta * zeta).sqrt();
            1.0 - (-decay * seconds).exp()
                * ((ringing * seconds).cos() + decay / ringing * (ringing * seconds).sin())
        } else if zeta == 1.0 {
            1.0 - (-omega * seconds).exp() * (1.0 + omega * seconds)
        } else {
            let root = omega * (zeta * zeta - 1.0).sqrt();
            let slow = -omega * zeta + root;
            let fast = -omega * zeta - root;
            1.0 - (fast * (slow * seconds).exp() - slow * (fast * seconds).exp()) / (fast - slow)
        }
    }
}

/// The CSS cubic Bézier through (0, 0), (x1, y1), (x2, y2) and (1, 1), read
/// at the time `t`: y at the parameter where x is `t`.
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let axis = |a: f64, b: f64, s: f64| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * s * a + 3.0 * inverse * s * s * b + s * s * s
    };
    let (mut low, mut high) = (0.0, 1.0);
    let mut s = t;
    for _ in 0..60 {
        let x = axis(x1, x2, s);
        if (x - t).abs() < 1e-9 {
            break;
        }
        if x < t {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    axis(y1, y2, s)
}
