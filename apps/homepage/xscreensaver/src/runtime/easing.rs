//! Port of `utils/easing.c`.
//!
//! ```text
//! Copyright © 2025 Jamie Zawinski <jwz@jwz.org>
//! Easing functions.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! Curves for a value going from 0 to 1: the difference between a thing that
//! moves at a constant rate and one that sets off, gets on with it and settles.
//! Upstream's comment is the important one, and it is kept below: *these are
//! the same semantics as CSS and jQuery*, so the names mean what they mean
//! everywhere else, and the constants are theirs.
//!
//! Nineteen of the savers use these.

use std::f64::consts::PI;

/// Which curve. The names are CSS's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    None,
    InSine,
    OutSine,
    InOutSine,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InQuart,
    OutQuart,
    InOutQuart,
    InQuint,
    OutQuint,
    InOutQuint,
    InExpo,
    OutExpo,
    InOutExpo,
    InCirc,
    OutCirc,
    InOutCirc,
    InBack,
    OutBack,
    InOutBack,
    InElastic,
    OutElastic,
    InOutElastic,
    InBounce,
    OutBounce,
    InOutBounce,
}

/// `ease`: put `x`, which runs 0 to 1, through the named curve.
pub fn ease(fn_: Ease, x: f64) -> f64 {
    match fn_ {
        Ease::None => x,
        Ease::InSine => 1.0 - ((x * PI) / 2.0).cos(),
        Ease::OutSine => ((x * PI) / 2.0).sin(),
        Ease::InOutSine => -((PI * x).cos() - 1.0) / 2.0,
        Ease::InQuad => x * x,
        Ease::OutQuad => 1.0 - (1.0 - x) * (1.0 - x),
        Ease::InOutQuad => {
            if x < 0.5 {
                2.0 * x * x
            } else {
                1.0 - (-2.0 * x + 2.0).powi(2) / 2.0
            }
        }
        Ease::InCubic => x * x * x,
        Ease::OutCubic => 1.0 - (1.0 - x).powi(3),
        Ease::InOutCubic => {
            if x < 0.5 {
                4.0 * x * x * x
            } else {
                1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
            }
        }
        Ease::InQuart => x * x * x * x,
        Ease::OutQuart => 1.0 - (1.0 - x).powi(4),
        Ease::InOutQuart => {
            if x < 0.5 {
                8.0 * x * x * x * x
            } else {
                1.0 - (-2.0 * x + 2.0).powi(4) / 2.0
            }
        }
        Ease::InQuint => x * x * x * x * x,
        Ease::OutQuint => 1.0 - (1.0 - x).powi(5),
        Ease::InOutQuint => {
            if x < 0.5 {
                16.0 * x * x * x * x * x
            } else {
                1.0 - (-2.0 * x + 2.0).powi(5) / 2.0
            }
        }
        Ease::InExpo => {
            if x == 0.0 {
                0.0
            } else {
                2.0f64.powf(10.0 * x - 10.0)
            }
        }
        Ease::OutExpo => {
            if x == 1.0 {
                1.0
            } else {
                1.0 - 2.0f64.powf(-10.0 * x)
            }
        }
        Ease::InOutExpo => {
            if x == 0.0 {
                0.0
            } else if x == 1.0 {
                1.0
            } else if x < 0.5 {
                2.0f64.powf(20.0 * x - 10.0) / 2.0
            } else {
                (2.0 - 2.0f64.powf(-20.0 * x + 10.0)) / 2.0
            }
        }
        Ease::InCirc => 1.0 - (1.0 - x.powi(2)).sqrt(),
        Ease::OutCirc => (1.0 - (x - 1.0).powi(2)).sqrt(),
        Ease::InOutCirc => {
            if x < 0.5 {
                (1.0 - (1.0 - (2.0 * x).powi(2)).sqrt()) / 2.0
            } else {
                ((1.0 - (-2.0 * x + 2.0).powi(2)).sqrt() + 1.0) / 2.0
            }
        }
        // The overshoot constants are CSS's, not anybody's choice here: an
        // "in back" curve goes the wrong way first and then springs forward.
        Ease::InBack => {
            let c1 = 1.70158;
            let c3 = c1 + 1.0;
            c3 * x * x * x - c1 * x * x
        }
        Ease::OutBack => {
            let c1 = 1.70158;
            let c3 = c1 + 1.0;
            1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
        }
        Ease::InOutBack => {
            let c1 = 1.70158;
            let c2 = c1 * 1.525;
            if x < 0.5 {
                ((2.0 * x).powi(2) * ((c2 + 1.0) * 2.0 * x - c2)) / 2.0
            } else {
                ((2.0 * x - 2.0).powi(2) * ((c2 + 1.0) * (x * 2.0 - 2.0) + c2) + 2.0) / 2.0
            }
        }
        Ease::InElastic => {
            let c4 = (2.0 * PI) / 3.0;
            if x == 0.0 {
                0.0
            } else if x == 1.0 {
                1.0
            } else {
                -(2.0f64.powf(10.0 * x - 10.0)) * ((x * 10.0 - 10.75) * c4).sin()
            }
        }
        Ease::OutElastic => {
            let c4 = (2.0 * PI) / 3.0;
            if x == 0.0 {
                0.0
            } else if x == 1.0 {
                1.0
            } else {
                2.0f64.powf(-10.0 * x) * ((x * 10.0 - 0.75) * c4).sin() + 1.0
            }
        }
        Ease::InOutElastic => {
            let c5 = (2.0 * PI) / 4.5;
            if x == 0.0 {
                0.0
            } else if x == 1.0 {
                1.0
            } else if x < 0.5 {
                -(2.0f64.powf(20.0 * x - 10.0) * ((20.0 * x - 11.125) * c5).sin()) / 2.0
            } else {
                (2.0f64.powf(-20.0 * x + 10.0) * ((20.0 * x - 11.125) * c5).sin()) / 2.0 + 1.0
            }
        }
        Ease::InBounce => 1.0 - out_bounce(1.0 - x),
        Ease::OutBounce => out_bounce(x),
        Ease::InOutBounce => {
            if x < 0.5 {
                (1.0 - out_bounce(1.0 - 2.0 * x)) / 2.0
            } else {
                (1.0 + out_bounce(2.0 * x - 1.0)) / 2.0
            }
        }
    }
}

/// Four parabolic hops of decreasing height, which is what a dropped thing
/// does. The other two bounce curves are this one run backwards or halved.
fn out_bounce(x: f64) -> f64 {
    let n1 = 7.5625;
    let d1 = 2.75;
    if x < 1.0 / d1 {
        n1 * x * x
    } else if x < 2.0 / d1 {
        let x = x - (1.5 / d1);
        n1 * x * x + 0.75
    } else if x < 2.5 / d1 {
        let x = x - (2.25 / d1);
        n1 * x * x + 0.9375
    } else {
        let x = x - (2.625 / d1);
        n1 * x * x + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Ease; 31] = [
        Ease::None,
        Ease::InSine,
        Ease::OutSine,
        Ease::InOutSine,
        Ease::InQuad,
        Ease::OutQuad,
        Ease::InOutQuad,
        Ease::InCubic,
        Ease::OutCubic,
        Ease::InOutCubic,
        Ease::InQuart,
        Ease::OutQuart,
        Ease::InOutQuart,
        Ease::InQuint,
        Ease::OutQuint,
        Ease::InOutQuint,
        Ease::InExpo,
        Ease::OutExpo,
        Ease::InOutExpo,
        Ease::InCirc,
        Ease::OutCirc,
        Ease::InOutCirc,
        Ease::InBack,
        Ease::OutBack,
        Ease::InOutBack,
        Ease::InElastic,
        Ease::OutElastic,
        Ease::InOutElastic,
        Ease::InBounce,
        Ease::OutBounce,
        Ease::InOutBounce,
    ];

    /// The one property they all share, and the one a caller depends on: an
    /// eased value still starts where it started and ends where it ended.
    #[test]
    fn every_curve_runs_from_zero_to_one() {
        for f in ALL {
            assert!(ease(f, 0.0).abs() < 1e-9, "{f:?} does not start at 0");
            assert!((ease(f, 1.0) - 1.0).abs() < 1e-9, "{f:?} does not end at 1");
        }
    }

    /// And none of them wanders off. The springy ones do overshoot, and are
    /// meant to: "back" pulls away before it sets off and "elastic" rings
    /// around the end for a while. Half again either way covers both, and
    /// nothing may return a NaN.
    #[test]
    fn no_curve_runs_away() {
        for f in ALL {
            for i in 0..=1000 {
                let y = ease(f, f64::from(i) / 1000.0);
                assert!(y.is_finite() && (-0.5..=1.5).contains(&y), "{f:?} gave {y}");
            }
        }
    }

    /// The springy ones have to actually spring, or they are just slow curves.
    #[test]
    fn back_and_elastic_overshoot() {
        for f in [Ease::OutBack, Ease::OutElastic] {
            let over = (0..=1000)
                .map(|i| ease(f, f64::from(i) / 1000.0))
                .any(|y| y > 1.0);
            assert!(over, "{f:?} never overshot");
        }
        for f in [Ease::InBack, Ease::InElastic] {
            let under = (0..=1000)
                .map(|i| ease(f, f64::from(i) / 1000.0))
                .any(|y| y < 0.0);
            assert!(under, "{f:?} never pulled back");
        }
    }

    /// The "in" curves start slowly and the "out" curves finish slowly, which
    /// is the whole distinction between them.
    #[test]
    fn in_starts_slow_and_out_starts_fast() {
        assert!(ease(Ease::InQuad, 0.1) < 0.1);
        assert!(ease(Ease::OutQuad, 0.1) > 0.1);
        assert!(ease(Ease::InCubic, 0.5) < ease(Ease::InQuad, 0.5));
    }

    /// The symmetric ones pass through the middle at the middle.
    #[test]
    fn the_in_out_curves_are_halfway_at_halfway() {
        for f in [
            Ease::InOutSine,
            Ease::InOutQuad,
            Ease::InOutCubic,
            Ease::InOutQuart,
            Ease::InOutQuint,
            Ease::InOutCirc,
        ] {
            assert!((ease(f, 0.5) - 0.5).abs() < 1e-9, "{f:?}");
        }
    }
}
