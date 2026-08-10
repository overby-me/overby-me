//! The 2D (Xlib) savers: upstream's `hacks/*.c`.
//!
//! Each module is one hack, ported against [`crate::runtime`] and keeping its
//! upstream copyright header. A hack exposes its [`SaverDef`] (identity and
//! knobs) and a `start` function that builds a [`Runner`] for it. `start` is the
//! only way in, and on the web it is the single function its wasm chunk
//! exports, which is what lets the splitter attribute the hack's code to that
//! chunk instead of the main module.
//!
//! [`SaverDef`]: crate::runtime::SaverDef
//! [`Runner`]: crate::runtime::Runner

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;

pub mod anemone;
pub mod binaryhorizon;
pub mod binaryring;
pub mod blitspin;
pub mod boxfit;
pub mod braid;
pub mod cloudlife;
pub mod coral;
pub mod critical;
pub mod cwaves;
pub mod cynosure;
pub mod decayscreen;
pub mod deco;
pub mod deluxe;
pub mod discrete;
pub mod fadeplot;
pub mod fiberlamp;
pub mod flame;
pub mod forest;
pub mod fuzzyflakes;
pub mod galaxy;
pub mod grav;
pub mod greynetic;
pub mod halftone;
pub mod halo;
pub mod helix;
pub mod hexadrop;
pub mod hopalong;
pub mod hypercube;
pub mod ifs;
pub mod imsmap;
pub mod julia;
pub mod kaleidescope;
pub mod kumppa;
pub mod laser;
pub mod lcdscrub;
pub mod lissie;
pub mod lmorph;
pub mod metaballs;
pub mod moire;
pub mod moire2;
pub mod mountain;
pub mod munch;
pub mod pedal;
pub mod popsquares;
pub mod pyro;
pub mod rocks;
pub mod rorschach;
pub mod rotor;
pub mod rotzoomer;
pub mod shadebobs;
pub mod sierpinski;
pub mod slidescreen;
pub mod slip;
pub mod sphere;
pub mod spiral;
pub mod spotlight;
pub mod squiral;
pub mod starfish;
pub mod thornbird;
pub mod triangle;
pub mod truchet;
pub mod vines;
pub mod wander;
pub mod whirlwindwarp;
pub mod worm;
pub mod xspirograph;
pub mod zoom;

/// Every 2D saver ported so far.
///
/// Native only: on wasm this table is exactly what must not exist, because
/// naming every saver's entry point in one place is what would keep them all in
/// the main module. See [`crate::all`].
#[cfg(not(target_arch = "wasm32"))]
pub static ALL: &[&Saver] = &[
    &anemone::SAVER,
    &binaryhorizon::SAVER,
    &binaryring::SAVER,
    &blitspin::SAVER,
    &boxfit::SAVER,
    &braid::SAVER,
    &cloudlife::SAVER,
    &coral::SAVER,
    &critical::SAVER,
    &cwaves::SAVER,
    &deco::SAVER,
    &cynosure::SAVER,
    &decayscreen::SAVER,
    &deluxe::SAVER,
    &discrete::SAVER,
    &fuzzyflakes::SAVER,
    &galaxy::SAVER,
    &greynetic::SAVER,
    &fadeplot::SAVER,
    &fiberlamp::SAVER,
    &flame::SAVER,
    &forest::SAVER,
    &grav::SAVER,
    &halo::SAVER,
    &halftone::SAVER,
    &helix::SAVER,
    &hexadrop::SAVER,
    &hopalong::SAVER,
    &hypercube::SAVER,
    &ifs::SAVER,
    &imsmap::SAVER,
    &julia::SAVER,
    &kaleidescope::SAVER,
    &laser::SAVER,
    &kumppa::SAVER,
    &lcdscrub::SAVER,
    &lissie::SAVER,
    &moire::SAVER,
    &lmorph::SAVER,
    &metaballs::SAVER,
    &moire2::SAVER,
    &mountain::SAVER,
    &munch::SAVER,
    &pedal::SAVER,
    &popsquares::SAVER,
    &pyro::SAVER,
    &rocks::SAVER,
    &rorschach::SAVER,
    &rotor::SAVER,
    &shadebobs::SAVER,
    &rotzoomer::SAVER,
    &sierpinski::SAVER,
    &slidescreen::SAVER,
    &slip::SAVER,
    &sphere::SAVER,
    &spiral::SAVER,
    &spotlight::SAVER,
    &squiral::SAVER,
    &starfish::SAVER,
    &thornbird::SAVER,
    &triangle::SAVER,
    &vines::SAVER,
    &truchet::SAVER,
    &wander::SAVER,
    &whirlwindwarp::SAVER,
    &worm::SAVER,
    &xspirograph::SAVER,
    &zoom::SAVER,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Runner, StartArgs, XEvent, color::ALPHA};

    /// Frames to render per saver in the smoke tests. Enough for a hack with a
    /// slow start (rorschach spends its first calls on a single chunk of the
    /// walk) to have put something on screen.
    const FRAMES: usize = 120;

    fn run(saver: &'static Saver, w: i32, h: i32, query: &str) -> Runner {
        let mut r = (saver.start)(StartArgs::new(w, h, query, 20260809));
        for _ in 0..FRAMES {
            r.step();
        }
        r
    }

    fn lit(r: &Runner) -> usize {
        r.dpy
            .win_ref()
            .pixels()
            .iter()
            .filter(|p| **p != ALPHA)
            .count()
    }

    /// Sampled as the run goes rather than measured at the end: pyro's shells
    /// fly off the top of the screen between launches, so its last frame can be
    /// nearly empty while the hack is drawing plenty.
    #[test]
    fn every_saver_draws_something() {
        for saver in ALL {
            let def = saver.def;
            let mut r = (saver.start)(StartArgs::new(320, 240, "", 20260809));
            let mut best = 0;
            for i in 0..FRAMES {
                r.step();
                if i % 10 == 0 {
                    best = best.max(lit(&r));
                }
            }
            best = best.max(lit(&r));
            assert!(
                best > 100,
                "{} drew almost nothing ({best} pixels)",
                def.slug
            );
        }
    }

    /// Looking once is not enough: lcdscrub slides its pattern one pixel a
    /// frame and repeats every eight, so a fixed stride can land on the same
    /// picture twice and read as frozen.
    #[test]
    fn every_saver_keeps_changing() {
        for saver in ALL {
            let def = saver.def;
            let mut r = run(saver, 320, 240, "");
            let a = r.frame_hash();
            let mut changed = false;
            for _ in 0..5 {
                for _ in 0..37 {
                    r.step();
                }
                if r.frame_hash() != a {
                    changed = true;
                    break;
                }
            }
            assert!(changed, "{} froze", def.slug);
        }
    }

    #[test]
    fn every_saver_is_reproducible_from_its_seed() {
        for saver in ALL {
            let def = saver.def;
            let a = run(saver, 320, 240, "");
            let b = run(saver, 320, 240, "");
            assert_eq!(
                a.frame_hash(),
                b.frame_hash(),
                "{} is not deterministic",
                def.slug
            );
        }
    }

    /// Awkward geometries are where hacks divide by zero or index off the end:
    /// a window narrower than the minimum rectangle greynetic wants, one pixel
    /// tall, or square.
    #[test]
    fn every_saver_survives_a_degenerate_window() {
        for saver in ALL {
            let def = saver.def;
            for (w, h) in [(1, 1), (3, 200), (200, 3), (49, 49), (2560, 4)] {
                let mut r = (saver.start)(StartArgs::new(w, h, "", 1));
                for _ in 0..30 {
                    r.step();
                }
                assert_eq!(r.dpy.width(), w.max(1), "{} at {w}x{h}", def.slug);
            }
        }
    }

    #[test]
    fn every_saver_survives_a_resize_mid_run() {
        for saver in ALL {
            let def = saver.def;
            let mut r = run(saver, 320, 240, "");
            r.resize(64, 480);
            for _ in 0..60 {
                r.step();
            }
            r.resize(800, 600);
            for _ in 0..60 {
                r.step();
            }
            assert_eq!(r.dpy.width(), 800, "{}", def.slug);
        }
    }

    #[test]
    fn every_saver_survives_being_poked() {
        for saver in ALL {
            let mut r = run(saver, 320, 240, "");
            r.event(XEvent::ButtonPress {
                x: 10,
                y: 10,
                button: 1,
            });
            r.event(XEvent::KeyPress { key: ' ' });
            r.event(XEvent::MotionNotify { x: 5, y: 5 });
            for _ in 0..60 {
                r.step();
            }
        }
    }

    /// Every declared option has to be readable back out of the resource
    /// database, or the panel would show a control that changes nothing.
    #[test]
    fn declared_options_reach_the_hack() {
        for saver in ALL {
            let def = saver.def;
            for opt in def.opts {
                let query = format!("{}={}", opt.key, opt.default);
                let r = (saver.start)(StartArgs::new(320, 240, &query, 1));
                assert_eq!(
                    r.dpy.res.value_of(opt),
                    opt.default,
                    "{}: option {} did not survive the round trip",
                    def.slug,
                    opt.key
                );
            }
        }
    }

    /// Extreme but legal option values must not hang or panic. The sliders'
    /// own bounds are the contract, so both ends of each are fair game.
    #[test]
    fn extreme_option_values_are_survivable() {
        use crate::runtime::OptKind;
        for saver in ALL {
            let def = saver.def;
            for opt in def.opts {
                let values: Vec<String> = match opt.kind {
                    OptKind::Slider { low, high, .. } | OptKind::Spin { low, high } => {
                        vec![format!("{low}"), format!("{high}")]
                    }
                    OptKind::Bool => vec!["true".into(), "false".into()],
                    OptKind::Select(items) => items.iter().map(|i| i.value.to_string()).collect(),
                };
                for v in values {
                    let query = format!("{}={}", opt.key, v);
                    let mut r = (saver.start)(StartArgs::new(200, 150, &query, 3));
                    // A setting that cannot be survived fails in the first few
                    // frames: a division by the extreme value, an allocation
                    // sized from it, an index off the end of a table. This runs
                    // twice per option per saver, so keep the count low enough
                    // that a full tier stays quick to check.
                    for _ in 0..12 {
                        r.step();
                    }
                }
            }
        }
    }

    #[test]
    fn slugs_are_unique_and_url_safe() {
        let mut seen = std::collections::HashSet::new();
        for saver in ALL {
            let def = saver.def;
            assert!(seen.insert(def.slug), "duplicate slug {}", def.slug);
            assert!(
                def.slug
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "slug {} is not url safe",
                def.slug
            );
            assert!(!def.label.is_empty());
        }
    }
}
