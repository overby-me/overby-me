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

pub mod abstractile;
pub mod anemone;
pub mod anemotaxis;
pub mod ant;
pub mod apollonian;
pub mod attraction;
pub mod barcode;
pub mod binaryhorizon;
pub mod binaryring;
pub mod blaster;
pub mod blitspin;
pub mod bouboule;
pub mod boxfit;
pub mod braid;
pub mod bumps;
pub mod ccurve;
pub mod celtic;
pub mod cloudlife;
pub mod compass;
pub mod coral;
pub mod critical;
pub mod crystal;
pub mod cwaves;
pub mod cynosure;
pub mod decayscreen;
pub mod deco;
pub mod deluxe;
pub mod demon;
pub mod discrete;
pub mod distort;
pub mod drift;
pub mod droste;
pub mod epicycle;
pub mod eruption;
pub mod euler2d;
pub mod fadeplot;
pub mod fiberlamp;
pub mod filmleader;
pub mod fireworkx;
pub mod flame;
pub mod flow;
pub mod fluidballs;
pub mod fontglide;
pub mod forest;
pub mod fuzzyflakes;
pub mod galaxy;
pub mod goop;
pub mod grav;
pub mod greynetic;
pub mod halftone;
pub mod halo;
pub mod helix;
pub mod hexadrop;
pub mod hopalong;
pub mod hyperball;
pub mod hypercube;
pub mod ifs;
pub mod imsmap;
pub mod interaggregate;
pub mod interference;
pub mod intermomentary;
pub mod juggle;
pub mod julia;
pub mod kaleidescope;
pub mod kumppa;
pub mod laser;
pub mod lcdscrub;
pub mod lightning;
pub mod lisa;
pub mod lissie;
pub mod lmorph;
pub mod r#loop;
pub mod marbling;
pub mod memscroller;
pub mod metaballs;
pub mod moire;
pub mod moire2;
pub mod mountain;
pub mod munch;
pub mod nerverot;
pub mod pedal;
pub mod penetrate;
pub mod penrose;
pub mod petri;
pub mod piecewise;
pub mod polyominoes;
pub mod pong;
pub mod popsquares;
pub mod pyro;
pub mod qix;
pub mod rdbomb;
pub mod ripples;
pub mod rocks;
pub mod rorschach;
pub mod rotor;
pub mod rotzoomer;
pub mod scooter;
pub mod shadebobs;
pub mod sierpinski;
pub mod slidescreen;
pub mod slip;
pub mod speedmine;
pub mod sphere;
pub mod spiral;
pub mod spotlight;
pub mod squiral;
pub mod starfish;
pub mod strange;
pub mod substrate;
pub mod swirl;
pub mod t3d;
pub mod tessellimage;
pub mod thornbird;
pub mod triangle;
pub mod truchet;
pub mod twang;
pub mod vermiculate;
pub mod vines;
pub mod wander;
pub mod whirlwindwarp;
pub mod whirlygig;
pub mod worm;
pub mod wormhole;
pub mod xflame;
pub mod xjack;
pub mod xlyap;
pub mod xrayswarm;
pub mod xspirograph;
pub mod zoom;

/// Every 2D saver ported so far.
///
/// Native only: on wasm this table is exactly what must not exist, because
/// naming every saver's entry point in one place is what would keep them all in
/// the main module. See [`crate::all`].
#[cfg(not(target_arch = "wasm32"))]
pub static ALL: &[&Saver] = &[
    &ant::SAVER,
    &abstractile::SAVER,
    &anemone::SAVER,
    &anemotaxis::SAVER,
    &apollonian::SAVER,
    &attraction::SAVER,
    &barcode::SAVER,
    &binaryhorizon::SAVER,
    &binaryring::SAVER,
    &blaster::SAVER,
    &blitspin::SAVER,
    &bouboule::SAVER,
    &boxfit::SAVER,
    &braid::SAVER,
    &bumps::SAVER,
    &ccurve::SAVER,
    &celtic::SAVER,
    &cloudlife::SAVER,
    &compass::SAVER,
    &coral::SAVER,
    &critical::SAVER,
    &crystal::SAVER,
    &cwaves::SAVER,
    &deco::SAVER,
    &cynosure::SAVER,
    &decayscreen::SAVER,
    &deluxe::SAVER,
    &demon::SAVER,
    &discrete::SAVER,
    &fuzzyflakes::SAVER,
    &galaxy::SAVER,
    &greynetic::SAVER,
    &distort::SAVER,
    &drift::SAVER,
    &droste::SAVER,
    &epicycle::SAVER,
    &euler2d::SAVER,
    &eruption::SAVER,
    &fadeplot::SAVER,
    &fiberlamp::SAVER,
    &filmleader::SAVER,
    &fireworkx::SAVER,
    &flame::SAVER,
    &flow::SAVER,
    &fluidballs::SAVER,
    &fontglide::SAVER,
    &forest::SAVER,
    &goop::SAVER,
    &grav::SAVER,
    &halo::SAVER,
    &halftone::SAVER,
    &helix::SAVER,
    &hexadrop::SAVER,
    &hopalong::SAVER,
    &hyperball::SAVER,
    &hypercube::SAVER,
    &ifs::SAVER,
    &imsmap::SAVER,
    &interaggregate::SAVER,
    &interference::SAVER,
    &intermomentary::SAVER,
    &juggle::SAVER,
    &julia::SAVER,
    &kaleidescope::SAVER,
    &laser::SAVER,
    &kumppa::SAVER,
    &lcdscrub::SAVER,
    &lightning::SAVER,
    &lisa::SAVER,
    &lissie::SAVER,
    &moire::SAVER,
    &lmorph::SAVER,
    &r#loop::SAVER,
    &marbling::SAVER,
    &memscroller::SAVER,
    &metaballs::SAVER,
    &moire2::SAVER,
    &mountain::SAVER,
    &munch::SAVER,
    &nerverot::SAVER,
    &pedal::SAVER,
    &penetrate::SAVER,
    &penrose::SAVER,
    &petri::SAVER,
    &piecewise::SAVER,
    &polyominoes::SAVER,
    &pong::SAVER,
    &popsquares::SAVER,
    &pyro::SAVER,
    &qix::SAVER,
    &rdbomb::SAVER,
    &ripples::SAVER,
    &rocks::SAVER,
    &rorschach::SAVER,
    &rotor::SAVER,
    &scooter::SAVER,
    &shadebobs::SAVER,
    &rotzoomer::SAVER,
    &sierpinski::SAVER,
    &slidescreen::SAVER,
    &slip::SAVER,
    &speedmine::SAVER,
    &sphere::SAVER,
    &spiral::SAVER,
    &spotlight::SAVER,
    &squiral::SAVER,
    &starfish::SAVER,
    &strange::SAVER,
    &substrate::SAVER,
    &swirl::SAVER,
    &t3d::SAVER,
    &tessellimage::SAVER,
    &thornbird::SAVER,
    &triangle::SAVER,
    &vines::SAVER,
    &truchet::SAVER,
    &twang::SAVER,
    &vermiculate::SAVER,
    &wander::SAVER,
    &whirlwindwarp::SAVER,
    &whirlygig::SAVER,
    &worm::SAVER,
    &wormhole::SAVER,
    &xflame::SAVER,
    &xjack::SAVER,
    &xlyap::SAVER,
    &xrayswarm::SAVER,
    &xspirograph::SAVER,
    &zoom::SAVER,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Runner, StartArgs, XEvent, color::ALPHA};

    /// Frames of warm-up before the robustness checks: enough that a hack with
    /// a slow start (rorschach spends its first calls on a single chunk of the
    /// walk) is properly under way. Whether a saver ever draws is checked
    /// separately and patiently, so this does not have to be long, and the
    /// expensive hacks (marbling computes Perlin noise per pixel) are paid for
    /// once per frame here across every test that warms up.
    const FRAMES: usize = 60;

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

    /// How long to keep looking for a saver's first picture. Lightning spends
    /// well over a second of its cycle waiting for the strike, and a hack that
    /// draws nothing at all for that long is still a bug, so the window is
    /// generous and the loop leaves as soon as it has seen something.
    const PATIENCE: usize = 400;

    /// Sampled as the run goes rather than measured at the end: pyro's shells
    /// fly off the top of the screen between launches, so its last frame can be
    /// nearly empty while the hack is drawing plenty.
    #[test]
    fn every_saver_draws_something() {
        for saver in ALL {
            let def = saver.def;
            let mut r = (saver.start)(StartArgs::new(320, 240, "", 20260809));
            let mut best = 0;
            for i in 0..PATIENCE {
                r.step();
                if i % 10 == 0 {
                    best = best.max(lit(&r));
                    if best > 100 {
                        break;
                    }
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
            // A resize either lands or it does not; twenty frames on the far
            // side of each is enough to catch a stale width or a bad index,
            // and the second size is four times the pixels of the first.
            r.resize(64, 480);
            for _ in 0..20 {
                r.step();
            }
            r.resize(800, 600);
            for _ in 0..20 {
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
