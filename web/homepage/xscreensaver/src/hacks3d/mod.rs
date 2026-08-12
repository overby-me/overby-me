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

pub mod antinspect;
pub mod atunnel;
pub mod beats;
pub mod blinkbox;
pub mod blocktube;
pub mod boing;
pub mod bouncingcow;
pub mod bubble3d;
pub mod cage;
pub mod carousel;
pub mod chompytower;
pub mod circuit;
pub mod cityflow;
pub mod crackberg;
pub mod cube21;
pub mod cubenetic;
pub mod cubestack;
pub mod cubestorm;
pub mod cubetwist;
pub mod cubicgrid;
pub mod dangerball;
pub mod discoball;
pub mod dumpsterfire;
pub mod energystream;
pub mod engine;
pub mod etruscanvenus;
pub mod flipflop;
pub mod flipscreen3d;
pub mod fliptext;
pub mod flyingtoasters;
pub mod gears;
pub mod geodesic;
pub mod gflux;
pub mod gibson;
pub mod glblur;
pub mod glhanoi;
pub mod glknots;
pub mod glmatrix;
pub mod glschool;
pub mod glsnake;
mod glsnake_models;
pub mod gltext;
pub mod gravitywell;
pub mod handsy;
// Public because it is converted upstream data rather than a private helper:
// the header defines three animations that its own table never lists, a whole
// round of rock-paper-scissors among them, and they are kept because this is
// that header converted and not a selection from it.
pub mod handsy_anim;
pub mod headroom;
pub mod hexstrut;
pub mod hextrail;
pub mod highvoltage;
pub mod hydrostat;
pub mod hypertorus;
pub mod hypnowheel;
pub mod kaleidocycle;
pub mod kallisti;
pub mod klein;
pub mod lavalite;
pub mod lockward;
pub mod menger;
pub mod moebius;
pub mod moebiusgears;
pub mod molecule;
pub mod morph3d;
pub mod noof;
pub mod papercube;
pub mod photopile;
pub mod pinion;
pub mod projectiveplane;
pub mod providence;
pub mod pulsar;
pub mod quasicrystal;
pub mod raverhoop;
pub mod razzledazzle;
pub mod romanboy;
pub mod rubik;
pub mod rubikblocks;
pub mod sballs;
pub mod sierpinski3d;
pub mod skulloop;
pub mod skytentacles;
pub mod spheremonics;
pub mod splodesic;
pub mod sproingies;
pub mod stairs;
pub mod starwars;
pub mod stonerview;
pub mod superquadrics;
pub mod surfaces;
pub mod tangram;
pub mod topblock;
pub mod tronbit;
pub mod unknownpleasures;
pub mod vigilance;
pub mod voronoi;
pub mod winduprobot;
mod winduprobot_wobble;

/// Every ported OpenGL saver, in the order they were added. Native only, for
/// the reason [`crate::all`] gives.
#[cfg(not(target_arch = "wasm32"))]
pub static ALL: &[&Saver3d] = &[
    &antinspect::SAVER,
    &atunnel::SAVER,
    &blinkbox::SAVER,
    &blocktube::SAVER,
    &boing::SAVER,
    &cubestorm::SAVER,
    &cubetwist::SAVER,
    &cubicgrid::SAVER,
    &beats::SAVER,
    &bouncingcow::SAVER,
    &bubble3d::SAVER,
    &cage::SAVER,
    &cityflow::SAVER,
    &circuit::SAVER,
    &cube21::SAVER,
    &crackberg::SAVER,
    &cubenetic::SAVER,
    &cubestack::SAVER,
    &dangerball::SAVER,
    &discoball::SAVER,
    &dumpsterfire::SAVER,
    &energystream::SAVER,
    &engine::SAVER,
    &etruscanvenus::SAVER,
    &flyingtoasters::SAVER,
    &gears::SAVER,
    &geodesic::SAVER,
    &gibson::SAVER,
    &glblur::SAVER,
    &glknots::SAVER,
    &glhanoi::SAVER,
    &glschool::SAVER,
    &headroom::SAVER,
    &handsy::SAVER,
    &highvoltage::SAVER,
    &carousel::SAVER,
    &chompytower::SAVER,
    &glmatrix::SAVER,
    &gltext::SAVER,
    &fliptext::SAVER,
    &flipflop::SAVER,
    &flipscreen3d::SAVER,
    &gflux::SAVER,
    &photopile::SAVER,
    &hexstrut::SAVER,
    &starwars::SAVER,
    &skytentacles::SAVER,
    &sproingies::SAVER,
    &winduprobot::SAVER,
    &glsnake::SAVER,
    &gravitywell::SAVER,
    &hextrail::SAVER,
    &hydrostat::SAVER,
    &hypertorus::SAVER,
    &kaleidocycle::SAVER,
    &kallisti::SAVER,
    &klein::SAVER,
    &lockward::SAVER,
    &lavalite::SAVER,
    &menger::SAVER,
    &moebius::SAVER,
    &moebiusgears::SAVER,
    &molecule::SAVER,
    &morph3d::SAVER,
    &noof::SAVER,
    &papercube::SAVER,
    &pinion::SAVER,
    &projectiveplane::SAVER,
    &providence::SAVER,
    &pulsar::SAVER,
    &quasicrystal::SAVER,
    &raverhoop::SAVER,
    &romanboy::SAVER,
    &razzledazzle::SAVER,
    &rubikblocks::SAVER,
    &rubik::SAVER,
    &sballs::SAVER,
    &sierpinski3d::SAVER,
    &skulloop::SAVER,
    &spheremonics::SAVER,
    &splodesic::SAVER,
    &stairs::SAVER,
    &stonerview::SAVER,
    &superquadrics::SAVER,
    &surfaces::SAVER,
    &tangram::SAVER,
    &topblock::SAVER,
    &tronbit::SAVER,
    &unknownpleasures::SAVER,
    &vigilance::SAVER,
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

    /// Whether anything a saver drew can be seen: geometry inside the clip
    /// volume, or geometry drawn bigger than the screen, which has every
    /// corner off it and covers it anyway. `quasicrystal`'s planes are three
    /// times the size of the view, so the second case is checked by looking
    /// for a batch whose bounding box has the middle of the screen inside it.
    fn anything_visible(f: &crate::runtime::gl::Frame) -> bool {
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
        if seen > 10 {
            return true;
        }
        f.batches.iter().any(|b| {
            let ps: Vec<[f32; 3]> = f.vertices[b.first..b.first + b.count]
                .iter()
                .map(|v| b.mvp.transform(v.pos))
                .collect();
            (0..3).all(|k| {
                ps.iter().map(|p| p[k]).fold(f32::MAX, f32::min) <= 0.0
                    && ps.iter().map(|p| p[k]).fold(f32::MIN, f32::max) >= 0.0
            })
        })
    }

    /// And its geometry has to land somewhere a camera can see: all of it
    /// behind the eye, or all of it off the side, is the same black screen.
    #[test]
    fn every_saver_puts_something_on_screen() {
        for saver in ALL {
            if anything_visible(run(saver, 640, 480, "").frame()) {
                continue;
            }
            // A saver is allowed to open on an empty view and swing round to
            // its subject: `flyingtoasters` starts with the camera pointed
            // away from the flock and takes a hundred frames to come about.
            let mut r = (saver.start)(StartArgs::new(640, 480, "", 20260811));
            for _ in 0..300 {
                r.step();
            }
            assert!(
                anything_visible(r.frame()),
                "{} never put anything on screen",
                saver.def.slug
            );
        }
    }

    /// A digest of everything a frame would draw: where its vertices are and
    /// under which matrices. Compared rather than the frames themselves so that
    /// a failure prints a number instead of a megabyte of floats.
    fn frame_fingerprint(r: &Runner3d) -> u64 {
        let f = r.frame();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for x in f
            .batches
            .iter()
            .flat_map(|b| b.mvp.0)
            .chain(f.vertices.iter().flat_map(|v| v.pos))
        {
            h ^= u64::from(x.to_bits());
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    /// Looking once is not enough: these all turn over, so two frames a moment
    /// apart must not be identical.
    ///
    /// What moves is usually the matrices, but it need not be: `stonerview`
    /// draws all forty of its squares under one matrix and animates the corners
    /// instead, so the vertices count too.
    #[test]
    fn every_saver_keeps_moving() {
        for saver in ALL {
            let mut r = (saver.start)(StartArgs::new(640, 480, "", 20260811));
            r.step();
            let before = frame_fingerprint(&r);
            for _ in 0..60 {
                r.step();
            }
            assert_ne!(before, frame_fingerprint(&r), "{} froze", saver.def.slug);
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
