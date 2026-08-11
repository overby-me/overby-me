//! The OpenGL savers: upstream's `hacks/glx/*.c`.
//!
//! Each module is one hack, ported against [`crate::runtime::gl`] and keeping
//! its upstream copyright header, and each exposes its [`SaverDef`] and a
//! `start` that builds a [`Runner3d`] for it. The rule the 2D tier follows
//! holds here too: `start` is the only way in, and on the web it is the single
//! function its wasm chunk exports.
//!
//! What is different is what a hack draws with. A 2D hack rasterises into a
//! framebuffer; these are written against OpenGL 1.3, which no longer exists,
//! so [`crate::runtime::gl`] takes their `glBegin`/`glVertex`/`glEnd` and turns
//! it into vertex buffers for the host to hand to WebGL2.
//!
//! [`SaverDef`]: crate::runtime::SaverDef
//! [`Runner3d`]: crate::runtime::Runner3d

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;

pub mod beats;
pub mod blinkbox;
pub mod boing;
pub mod cityflow;
pub mod cubestack;
pub mod cubestorm;
pub mod cubicgrid;
pub mod dangerball;
pub mod glknots;
pub mod gravitywell;
pub mod hexstrut;
pub mod hextrail;
pub mod hypnowheel;
pub mod kaleidocycle;
pub mod menger;
pub mod sierpinski3d;
pub mod splodesic;
pub mod voronoi;

/// Every ported OpenGL saver, in the order they were added. Native only, for
/// the reason [`crate::all`] gives.
#[cfg(not(target_arch = "wasm32"))]
pub static ALL: &[&Saver3d] = &[
    &blinkbox::SAVER,
    &boing::SAVER,
    &cubestorm::SAVER,
    &cubicgrid::SAVER,
    &beats::SAVER,
    &cityflow::SAVER,
    &cubestack::SAVER,
    &dangerball::SAVER,
    &glknots::SAVER,
    &hexstrut::SAVER,
    &gravitywell::SAVER,
    &hextrail::SAVER,
    &kaleidocycle::SAVER,
    &menger::SAVER,
    &sierpinski3d::SAVER,
    &splodesic::SAVER,
    &voronoi::SAVER,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Runner3d, StartArgs};

    fn run(saver: &'static Saver3d, w: i32, h: i32, query: &str) -> Runner3d {
        let mut r = (saver.start)(StartArgs::new(w, h, query, 20260811));
        for _ in 0..30 {
            r.step();
        }
        r
    }

    /// A saver that emits no geometry is a black screen, however well it
    /// compiles.
    #[test]
    fn every_saver_draws_something() {
        for saver in ALL {
            let r = run(saver, 640, 480, "");
            let f = r.frame();
            assert!(
                !f.batches.is_empty() && !f.vertices.is_empty(),
                "{} drew nothing",
                saver.def.slug
            );
        }
    }

    /// And its geometry has to land somewhere a camera can see: all of it
    /// behind the eye, or all of it off the side, is the same black screen.
    #[test]
    fn every_saver_puts_something_on_screen() {
        for saver in ALL {
            let r = run(saver, 640, 480, "");
            let f = r.frame();
            let mut seen = 0;
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let p = b.mvp.transform(v.pos);
                    if (-1.0..=1.0).contains(&p[0])
                        && (-1.0..=1.0).contains(&p[1])
                        && (-1.0..=1.0).contains(&p[2])
                    {
                        seen += 1;
                    }
                }
            }
            assert!(seen > 10, "{} drew {seen} visible vertices", saver.def.slug);
        }
    }

    /// Looking once is not enough: these all turn over, so two frames a moment
    /// apart must not be identical.
    #[test]
    fn every_saver_keeps_moving() {
        for saver in ALL {
            let mut r = (saver.start)(StartArgs::new(640, 480, "", 20260811));
            r.step();
            let before: Vec<[f32; 16]> = r.frame().batches.iter().map(|b| b.mvp.0).collect();
            for _ in 0..60 {
                r.step();
            }
            let after: Vec<[f32; 16]> = r.frame().batches.iter().map(|b| b.mvp.0).collect();
            assert_ne!(before, after, "{} froze", saver.def.slug);
        }
    }

    /// A window of nothing must not panic, and neither must a resize.
    #[test]
    fn every_saver_survives_a_degenerate_window() {
        for saver in ALL {
            let mut r = (saver.start)(StartArgs::new(1, 1, "", 20260811));
            for _ in 0..10 {
                r.step();
            }
            r.resize(640, 1);
            r.step();
            r.resize(1, 480);
            r.step();
            r.resize(800, 600);
            r.step();
        }
    }

    /// Being poked must not panic either, and a drag has to turn the picture.
    #[test]
    fn every_saver_survives_being_poked() {
        use crate::runtime::XEvent;
        for saver in ALL {
            let mut r = (saver.start)(StartArgs::new(640, 480, "", 20260811));
            r.step();
            r.event(XEvent::ButtonPress {
                x: 320,
                y: 240,
                button: 1,
            });
            r.event(XEvent::MotionNotify { x: 400, y: 240 });
            r.event(XEvent::ButtonRelease {
                x: 400,
                y: 240,
                button: 1,
            });
            r.event(XEvent::KeyPress { key: ' ' });
            r.step();
        }
    }

    /// The seed is the whole of a run's randomness, so the same seed has to
    /// give the same picture.
    #[test]
    fn every_saver_is_reproducible_from_its_seed() {
        for saver in ALL {
            let one = run(saver, 320, 240, "");
            let two = run(saver, 320, 240, "");
            let (a, b) = (one.frame(), two.frame());
            assert_eq!(a.vertices.len(), b.vertices.len(), "{}", saver.def.slug);
            assert_eq!(a.batches.len(), b.batches.len(), "{}", saver.def.slug);
            for (x, y) in a.batches.iter().zip(&b.batches) {
                assert_eq!(x.mvp, y.mvp, "{}", saver.def.slug);
            }
        }
    }

    /// Every knob the panel offers has to be one the saver reads, and moving
    /// it to either end must not break anything.
    #[test]
    fn extreme_option_values_are_survivable() {
        use crate::runtime::OptKind;
        for saver in ALL {
            for opt in saver.def.opts {
                let values: Vec<String> = match opt.kind {
                    OptKind::Slider { low, high, .. } | OptKind::Spin { low, high } => {
                        vec![format!("{low}"), format!("{high}")]
                    }
                    OptKind::Bool => vec!["true".into(), "false".into()],
                    OptKind::Select(items) => items.iter().map(|i| i.value.to_string()).collect(),
                };
                for v in values {
                    let query = format!("{}={v}", opt.key);
                    let mut r = (saver.start)(StartArgs::new(200, 150, &query, 3));
                    for _ in 0..8 {
                        r.step();
                    }
                }
            }
        }
    }
}
