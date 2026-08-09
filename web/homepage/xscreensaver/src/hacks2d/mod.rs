//! The 2D (Xlib) savers: upstream's `hacks/*.c`.
//!
//! Each module is one hack, ported against [`crate::runtime`] and keeping its
//! upstream copyright header. A hack exposes exactly one public item, its
//! [`SaverDef`], because that is all the host ever asks for: on the web each of
//! these becomes its own lazily-loaded wasm chunk, reached through a function
//! pointer rather than by name.
//!
//! [`SaverDef`]: crate::runtime::SaverDef

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::SaverDef;

pub mod greynetic;
pub mod munch;
pub mod rorschach;

/// Every 2D saver ported so far.
///
/// Native only: on wasm this table is exactly what must not exist, because
/// naming every saver in one place is what would keep them all in the main
/// module. See [`crate::all`].
#[cfg(not(target_arch = "wasm32"))]
pub static ALL: &[&SaverDef] = &[&greynetic::DEF, &munch::DEF, &rorschach::DEF];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Runner, XEvent, color::ALPHA};

    /// Frames to render per saver in the smoke tests. Enough for a hack with a
    /// slow start (rorschach spends its first calls on a single chunk of the
    /// walk) to have put something on screen.
    const FRAMES: usize = 120;

    fn run(def: &'static SaverDef, w: i32, h: i32, query: &str) -> Runner {
        let mut r = Runner::new(def, w, h, query, 20260809);
        for _ in 0..FRAMES {
            r.step();
        }
        r
    }

    #[test]
    fn every_saver_draws_something() {
        for def in ALL {
            let r = run(def, 320, 240, "");
            let lit = r
                .dpy
                .win_ref()
                .pixels()
                .iter()
                .filter(|p| **p != ALPHA)
                .count();
            assert!(lit > 100, "{} drew almost nothing ({lit} pixels)", def.slug);
        }
    }

    #[test]
    fn every_saver_keeps_changing() {
        for def in ALL {
            let mut r = run(def, 320, 240, "");
            let a = r.frame_hash();
            for _ in 0..FRAMES {
                r.step();
            }
            assert_ne!(r.frame_hash(), a, "{} froze", def.slug);
        }
    }

    #[test]
    fn every_saver_is_reproducible_from_its_seed() {
        for def in ALL {
            let a = run(def, 320, 240, "");
            let b = run(def, 320, 240, "");
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
        for def in ALL {
            for (w, h) in [(1, 1), (3, 200), (200, 3), (49, 49), (2560, 4)] {
                let mut r = Runner::new(def, w, h, "", 1);
                for _ in 0..30 {
                    r.step();
                }
                assert_eq!(r.dpy.width(), w.max(1), "{} at {w}x{h}", def.slug);
            }
        }
    }

    #[test]
    fn every_saver_survives_a_resize_mid_run() {
        for def in ALL {
            let mut r = run(def, 320, 240, "");
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
        for def in ALL {
            let mut r = run(def, 320, 240, "");
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
        for def in ALL {
            for opt in def.opts {
                let query = format!("{}={}", opt.key, opt.default);
                let r = Runner::new(def, 320, 240, &query, 1);
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
        for def in ALL {
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
                    let mut r = Runner::new(def, 200, 150, &query, 3);
                    for _ in 0..40 {
                        r.step();
                    }
                }
            }
        }
    }

    #[test]
    fn slugs_are_unique_and_url_safe() {
        let mut seen = std::collections::HashSet::new();
        for def in ALL {
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
