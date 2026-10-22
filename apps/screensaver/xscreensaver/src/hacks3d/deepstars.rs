//! Port of `hacks/glx/deepstars.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2019 Jamie Zawinski <jwz@jwz.org>
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
//! A long exposure of the night sky, showing star paths as vapour trails.
//!
//! There are no trails in the geometry. The whole sky is drawn again and again
//! in one frame, each copy turned a little further back about the pole and
//! drawn a little fainter, and what the eye reads as a trail is the pile of
//! copies. How many copies there are drifts up and down over the run, so the
//! exposure appears to lengthen and shorten.
//!
//! Half the stars are scattered evenly over the sphere and half are squashed
//! into a band a twentieth as thick, which is the Milky Way; the band is then
//! tilted fifty degrees out of the sky's own axis. The horizon is one ragged
//! quad strip drawn in screen coordinates, so it stays put while everything
//! above it turns.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/// How many copies of the sky the longest exposure is made of.
const SMEAR_BASE: f64 = 400.0;
/// How far the sky turns between one copy and the next, in degrees.
const SPEED_BASE: f64 = 0.02;
/// One frame in this many starts the exposure lengthening or shortening.
const SMEAR_CHANGE: u32 = 800;
/// How far away the sky is.
const SKY_SCALE: f32 = 60.0;

/// Only a few distinct star colours are needed, since each is one batch whose
/// alpha the exposure varies.
const NCOLORS: usize = 16;
/// The biggest a star is drawn, and the step between sizes.
const MAX_SIZE: f64 = 3.0;
const SIZE_INC: f64 = 0.5;

/// Three samples averaged, so the middle of the range comes up most often.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

/// The most star-points one frame may draw.
///
/// This is the only thing about the saver that is not upstream's. Upstream
/// keeps the sky in display lists on the card, so each of the four hundred
/// copies is one draw call however many stars are in it; here a list is
/// replayed as geometry, so the copies multiply. At 1280x720 upstream's own
/// star count is 20448 points, which at the longest exposure is 8.2 million
/// points a frame.
///
/// What gives is the *exposure*, not the star count: the sky keeps as many
/// stars as upstream puts in it and the trails come out shorter. A sparser
/// sky with long trails was tried first and looked like a different picture
/// altogether, since the density of the sky is most of what it looks like.
///
/// The turn is folded into the vertices rather than the matrix, so the whole
/// exposure of one colour and size is a single draw call however many copies
/// it is made of. Measured at 1280x720: 97 batches whatever the settings, and
/// about a millisecond a frame at this budget.
const MAX_POINTS_PER_FRAME: f64 = 600_000.0;

/// One size class of one colour of star: the points, and how big they are.
struct StarGroup {
    point_size: f32,
    points: Vec<[f32; 3]>,
}

struct DeepStars {
    trackball: Trackball,
    colors: [[f32; 4]; NCOLORS],
    /// The sky, grouped by colour and then by point size.
    stars: Vec<Vec<StarGroup>>,
    /// The horizon, as heights along a strip.
    ground: Vec<f32>,

    /// How far the sky has turned, in degrees.
    z: f64,
    latitude: f64,
    facing: f64,
    /// How many copies the exposure is currently made of, and which way that
    /// is drifting.
    smear: f64,
    dsmear: f64,

    speed: f64,
    max_smear: f64,
}

impl DeepStars {
    /// One frame's worth of drift in the exposure length.
    fn tick_smear(&mut self) {
        if self.dsmear == 0.0 && random().is_multiple_of(SMEAR_CHANGE) {
            self.dsmear = 1.0;
        } else if self.smear == self.max_smear && random().is_multiple_of(SMEAR_CHANGE) {
            self.dsmear = -1.0;
        }
        if !self.trackball.button_down() {
            self.smear += self.dsmear;
        }
        self.smear = self.smear.clamp(1.0, self.max_smear);
    }
}

impl Hack3d for DeepStars {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        g.glx.color_material(true);
        g.glx.clear();

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        // At the equator Polaris is on the horizon; in the Arctic, overhead.
        g.glx.rotate(180.0 - self.latitude as f32, 1.0, 0.0, 0.0);
        g.glx.rotate(self.facing as f32, 0.0, 1.0, 0.0);

        self.tick_smear();
        if !self.trackball.button_down() {
            self.z -= SPEED_BASE * self.speed;
        }

        g.glx.blend(Blend::Alpha);

        // Upstream turns the whole sky with the matrix stack and calls the
        // same display list once per copy. A matrix change starts a new batch
        // here, and there are ninety-six lists, so that would be ninety-six
        // draw calls per copy and up to two thousand copies. The turn and the
        // tilt are folded into the vertices instead, which leaves the alpha
        // as the only thing that varies between copies, and an alpha is
        // vertex data rather than batch state: the whole exposure of one
        // colour and size is then a single call. The points and the order
        // they go down in are unchanged.
        let smear = self.smear as i32;
        let tilt = 50.0f64.to_radians();
        let (st_, ct) = (tilt.sin(), tilt.cos());

        for (c, groups) in self.colors.iter().zip(&self.stars) {
            for grp in groups {
                g.glx.point_size(grp.point_size);
                g.glx.begin(Shape::Points);
                for i in 0..smear {
                    let alpha = (1.0 - f64::from(i) / f64::from(smear)) as f32;
                    let a = (self.z + f64::from(i) * SPEED_BASE * self.speed).to_radians();
                    let (sa, ca) = (a.sin() as f32, a.cos() as f32);
                    g.glx.color4f(c[0], c[1], c[2], alpha);
                    for p in &grp.points {
                        // Out to the sky, tilted, then turned about the pole.
                        let (x, y, z) = (p[0] * SKY_SCALE, p[1] * SKY_SCALE, p[2] * SKY_SCALE);
                        let ty = y * ct as f32 - z * st_ as f32;
                        let tz = y * st_ as f32 + z * ct as f32;
                        g.glx.vertex3f(x * ca - ty * sa, x * sa + ty * ca, tz);
                    }
                }
                g.glx.end();
            }
        }
        g.glx.pop_matrix();

        // The horizon, in screen coordinates so that it stays put.
        g.glx.blend(Blend::Off);
        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.translate(-1.0, -1.0, 0.0);
        g.glx.scale(2.0, 0.7, 1.0);

        g.glx.color4f(0.02, 0.02, 0.05, 1.0);
        g.glx.begin(Shape::QuadStrip);
        let n = (self.ground.len() - 1) as f32;
        for (i, h) in self.ground.iter().enumerate() {
            g.glx.vertex3f(i as f32 / n, 0.0, 0.0);
            g.glx.vertex3f(i as f32 / n, *h, 0.0);
        }
        g.glx.end();

        g.glx.pop_matrix();
        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width.max(1) as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -h, h, 5.0, 200.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.translate(0.0, 0.0, -40.0);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        // Upstream neutralises the horizontal drag and flips the vertical, so
        // the sky can only be tilted, not spun. Doing that to the event
        // rather than to the trackball is upstream's own trick.
        let (w, h) = (g.width(), g.height());
        let fixed = match event {
            XEvent::ButtonPress { y: _, button, x: _ } => Some(XEvent::ButtonPress {
                x: w / 2,
                y: h - h / 2,
                button: *button,
            }),
            XEvent::ButtonRelease { y, button, .. } => Some(XEvent::ButtonRelease {
                x: w / 2,
                y: h - y,
                button: *button,
            }),
            XEvent::MotionNotify { y, .. } => Some(XEvent::MotionNotify { x: w / 2, y: h - y }),
            _ => None,
        };
        match fixed {
            Some(e) => self.trackball.event(&e, w, h),
            None => false,
        }
    }
}

/// Build the sky: half of it scattered evenly and half squashed into the
/// Milky Way's band, in `NCOLORS` colours and a few size classes.
fn make_stars(nstars: usize, scale: f64) -> Vec<Vec<StarGroup>> {
    let sizes = (MAX_SIZE / SIZE_INC) as usize;
    let per_color = nstars / NCOLORS;
    (0..NCOLORS)
        .map(|_| {
            (1..=sizes)
                .map(|j| StarGroup {
                    point_size: (SIZE_INC * j as f64 * scale) as f32,
                    points: (0..per_color / sizes)
                        .map(|_| {
                            let x = frand(1.0) - 0.5;
                            let y = frand(1.0) - 0.5;
                            // Half the stars are in a band a twentieth as
                            // thick as the sphere: the Milky Way.
                            let z = if random() & 1 == 1 {
                                frand(1.0) - 0.5
                            } else {
                                (bellrand(1.0) - 0.5) / 20.0
                            };
                            let d = (x * x + y * y + z * z).sqrt().max(1e-9);
                            [(x / d) as f32, (y / d) as f32, (z / d) as f32]
                        })
                        .collect(),
                })
                .collect()
        })
        .collect()
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let (w, h) = (g.width(), g.height());
    let size = w.max(h);
    let smear_arg = g.res.float("smear").clamp(0.1, 5.0);
    let max_smear = SMEAR_BASE * smear_arg;

    // Upstream's star count, kept; the exposure is what gets cut instead.
    let nstars = ((f64::from(size) * f64::from(size) / 80.0) as usize)
        .max(NCOLORS * (MAX_SIZE / SIZE_INC) as usize);
    let max_smear = max_smear.min(MAX_POINTS_PER_FRAME / nstars as f64).max(1.0);

    let mut colors = [[0.0f32; 4]; NCOLORS];
    for c in &mut colors {
        // Stars are nearly white, and each is nudged a little off it.
        let d = 0.1;
        let r = 0.15 + frand(0.3);
        *c = [
            r as f32,
            (r + frand(d) - d) as f32,
            (r + frand(d) - d) as f32,
            1.0,
        ];
    }

    let mut ground = Vec::with_capacity(51);
    let mut inc = 0.5f64;
    for _ in 0..=50 {
        ground.push(inc as f32);
        inc += 0.1 * (frand(1.0) - 0.5);
    }

    let mut st = DeepStars {
        trackball: Trackball::new(),
        colors,
        stars: make_stars(nstars, 1.0),
        ground,
        z: 0.0,
        latitude: 10.0 + frand(70.0),
        facing: 10.0 * (frand(1.0) - 0.5),
        smear: 0.0,
        dsmear: 0.0,
        speed: g.res.float("speed").clamp(0.01, 8.0),
        max_smear,
    };

    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:   30000",
    "*showFPS: False",
    "*speed: 1.0",
    "*smear: 1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::slider("smear", "Smear", 0.1, 5.0, 0.1, 1, "1.0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "deepstars",
    label: "Deep Stars",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2019",
        video: Some("https://www.youtube.com/watch?v=_FhYeKXGpxs"),
        blurb: "A long exposure of the night sky, showing star paths as vapor trails.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// Every star is on the unit sphere, so the sky is a sphere and not a
    /// cube of scattered points.
    #[test]
    fn every_star_is_on_the_sky() {
        // These reach past the runner, so the generator has to be started by
        // hand for the run to be repeatable.
        crate::runtime::ya_rand_init(20260812);
        let sky = make_stars(2000, 1.0);
        let mut n = 0;
        for groups in &sky {
            for grp in groups {
                assert!(grp.point_size > 0.0);
                for p in &grp.points {
                    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    assert!((r - 1.0).abs() < 1e-5, "a star is {r} from the middle");
                    n += 1;
                }
            }
        }
        assert!(n > 1500, "only {n} stars of the 2000 asked for");
    }

    /// Half the stars are squashed into a thin band, which is the Milky Way.
    /// Without it the sky would be an even scatter with no galaxy in it.
    #[test]
    fn half_the_stars_are_in_the_milky_way() {
        crate::runtime::ya_rand_init(20260812);
        let sky = make_stars(4000, 1.0);
        let all: Vec<[f32; 3]> = sky
            .iter()
            .flatten()
            .flat_map(|g| g.points.iter().copied())
            .collect();
        // The band is a twentieth as thick as the sphere, so a star in it has
        // a small z once normalised.
        let in_band = all.iter().filter(|p| p[2].abs() < 0.1).count();
        let frac = in_band as f64 / all.len() as f64;
        assert!(
            frac > 0.4 && frac < 0.75,
            "{frac} of the sky is in the band, which is not half"
        );
        // And the rest really is spread out.
        assert!(
            all.iter().any(|p| p[2].abs() > 0.8),
            "nothing is away from the band at all"
        );
    }

    /// The exposure lengthens and shortens over the run, and never leaves its
    /// bounds.
    #[test]
    fn the_exposure_drifts_between_its_bounds() {
        crate::runtime::ya_rand_init(20260812);
        let mut st = a_sky();
        st.max_smear = 40.0;
        let (mut lo, mut hi) = (f64::MAX, 0.0f64);
        // Push it up by hand, since the drift starts at random.
        st.dsmear = 1.0;
        for _ in 0..5000 {
            st.tick_smear();
            lo = lo.min(st.smear);
            hi = hi.max(st.smear);
            assert!(
                (1.0..=40.0).contains(&st.smear),
                "the exposure left its bounds at {}",
                st.smear
            );
        }
        assert_eq!(hi, 40.0, "it never reached the longest exposure");
        assert!(lo <= 1.0, "it never reached the shortest: {lo}");
    }

    /// However long an exposure is asked for, one frame never draws more
    /// points than the budget: the sky keeps its stars and the exposure is
    /// what gets cut.
    #[test]
    fn the_exposure_is_cut_to_fit_the_frame() {
        for query in ["smear=0.1", "smear=1.0", "smear=5.0"] {
            let mut r = start(StartArgs::new(1280, 720, query, 20260812));
            let mut worst = 0usize;
            // Long enough for the exposure to have drifted up to its cap.
            for _ in 0..2500 {
                r.step();
                worst = worst.max(r.frame().vertices.len());
            }
            assert!(worst > 0, "{query} drew nothing");
            assert!(
                worst as f64 <= MAX_POINTS_PER_FRAME * 1.05,
                "{query} drew {worst} points, over the budget"
            );
        }
        // And the sky itself is upstream's size whatever the exposure.
        let r = run("smear=5.0", 1);
        let stars: usize = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
            .map(|b| b.count)
            .sum();
        assert!(stars > 3000, "only {stars} stars in the sky");
    }

    /// It draws: a sky of points and a horizon under it.
    #[test]
    fn the_sky_and_the_horizon_are_drawn() {
        let r = run("", 4);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(
            f.batches
                .iter()
                .any(|b| b.primitive == crate::runtime::gl::Primitive::Points),
            "no stars"
        );
        // The horizon is the last thing drawn, as triangles.
        let last = f.batches.last().expect("no batches");
        assert_eq!(last.primitive, crate::runtime::gl::Primitive::Triangles);
    }

    /// A sky with no GL behind it.
    fn a_sky() -> DeepStars {
        DeepStars {
            trackball: Trackball::new(),
            colors: [[1.0; 4]; NCOLORS],
            stars: Vec::new(),
            ground: vec![0.5; 51],
            z: 0.0,
            latitude: 45.0,
            facing: 0.0,
            smear: 0.0,
            dsmear: 0.0,
            speed: 1.0,
            max_smear: SMEAR_BASE,
        }
    }
}
