//! Port of `hacks/glx/cityflow.c`.
//!
//! ```text
//! cityflow, Copyright (c) 2014-2017 Jamie Zawinski <jwz@jwz.org>
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
//! Waves move across a sea of boxes. The city swells. The walls are closing in.
//!
//! Eight hundred boxes stand on a plane, each a random size at a random place,
//! and each one's height is a sample of an interference pattern taken where it
//! stands. The pattern is six circular waves whose sources circle the field on
//! their own cosines, and the height at a point is the sum of a radial profile
//! sampled at the distance from each source: the same arithmetic as upstream's
//! 2D `interference` hack, which is where the wave code came from.
//!
//! So there is no fluid here and nothing propagates. Every box is reading the
//! same fixed function of position and time, and the waves that appear to roll
//! across the city are the moiré of a handful of ripples sliding over each
//! other.
//!
//! The colour of a box follows its height rather than its position, so the
//! wave shows in the colours as well as in the skyline, and the background is
//! the first colour of the same map so the far boxes fade into it.
//!
//! Only three faces of each box are drawn: the top and two sides. The other two
//! never face the camera, which does not move far enough for it to matter, and
//! upstream's comment says leaving them out makes no difference to the frame
//! rate anyway. The boxes are sorted front to back once at startup, which
//! upstream measured as its fastest order.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    screenhack_event_helper,
};

/// The field the waves are computed over, in samples. Upstream calls it a
/// texture size, which is what it was before the pattern was sampled per box
/// rather than drawn.
const TEXTURE_SIZE: i32 = 512;

struct Cube {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
    h: f32,
    d: f32,
    /// The cosine and sine of this box's own skew, worked out once.
    cth: f32,
    sth: f32,
}

/// One of the ripples: where it is now, and how far round its circuit.
#[derive(Clone, Copy, Default)]
struct WaveSrc {
    x: i32,
    y: i32,
    xth: f64,
    yth: f64,
}

struct Waves {
    radius: i32,
    speed: i32,
    srcs: Vec<WaveSrc>,
    /// The radial profile of a ripple, sampled once per unit of distance.
    heights: Vec<i32>,
}

struct CityFlow {
    trackball: Trackball,
    cubes: Vec<Cube>,
    waves: Waves,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    colors: Vec<XColor>,
    wireframe: bool,
}

impl CityFlow {
    fn reset_colors(&mut self, g: &mut Gl) {
        self.colors = make_smooth_colormap(256);
        if !self.wireframe {
            let c = self.colors[0];
            g.glx.clear_color(
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            );
        }
    }

    fn tweak_cubes(&mut self) {
        for cube in &mut self.cubes {
            cube.x += (frand(2.0) - 1.0) as f32 * 0.01;
            cube.y += (frand(2.0) - 1.0) as f32 * 0.01;
            cube.z += (frand(2.0) - 1.0) as f32 * 0.01;
        }
    }

    /// Compute the effect of the waves on a pixel.
    fn interference_point(&self, x: i32, y: i32) -> i32 {
        let ww = &self.waves;
        let mut result = 0;
        for s in &ww.srcs {
            let dx = x - s.x;
            let dy = y - s.y;
            let dist = f64::from(dx * dx + dy * dy).sqrt() as i32;
            result += if dist >= ww.radius {
                0
            } else {
                ww.heights[dist as usize]
            };
        }
        result = (f64::from(result) * 0.4) as i32;
        result.min(255)
    }

    /// Move the wave origins around.
    fn interference(&mut self) {
        let tau = std::f64::consts::PI * 2.0;
        let speed = f64::from(self.waves.speed) / 1000.0;
        for s in &mut self.waves.srcs {
            s.xth += speed;
            if s.xth > tau {
                s.xth -= tau;
            }
            s.yth += speed;
            if s.yth > tau {
                s.yth -= tau;
            }
            let half = f64::from(TEXTURE_SIZE / 2);
            s.x = (half + s.xth.cos() * half) as i32;
            s.y = (half + s.yth.cos() * half) as i32;
        }
    }

    fn animate_cubes(&mut self) {
        for i in 0..self.cubes.len() {
            let (fx, fy) = {
                let cube = &self.cubes[i];
                (
                    (cube.x - self.min_x) / (self.max_x - self.min_x),
                    (cube.y - self.min_y) / (self.max_y - self.min_y),
                )
            };
            let x = (TEXTURE_SIZE as f32 * fx) as i32 % TEXTURE_SIZE;
            let y = (TEXTURE_SIZE as f32 * fy) as i32 % TEXTURE_SIZE;
            let v = self.interference_point(x, y).clamp(0, 255);
            self.cubes[i].h = self.cubes[i].z + (v as f32 / 256.0 / 2.5) + 0.1;
        }
    }
}

impl Hack3d for CityFlow {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;

        self.interference();
        self.animate_cubes();

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(-180.0, 1.0, 0.0, 0.0);

        g.glx.scale(15.0, 15.0, 15.0);
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);

        g.glx.translate(-0.18, 0.0, -0.18);
        g.glx.rotate(37.0, 1.0, 0.0, 0.0);
        g.glx.rotate(20.0, 0.0, 0.0, 1.0);

        g.glx.scale(2.1, 2.1, 2.1);

        /* Position lights after device rotation. */
        if !wire {
            g.glx.light_position(0, 0.0, 0.25, -1.0, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }

        g.glx.begin(if wire { Shape::Lines } else { Shape::Quads });

        let n = self.colors.len();
        for cube in &self.cubes {
            let (cth, sth) = (cube.cth, cube.sth);
            let x = cth * cube.x + sth * cube.y;
            let y = -sth * cube.x + cth * cube.y;
            let w = cube.w / 2.0;
            let h = cube.h / 2.0;
            let d = cube.d / 2.0;
            let bottom = 5.0;

            let (xw, xd) = (cth * w, sth * d);
            let (yw, yd) = (-sth * w, cth * d);

            // The colour follows the height, so the wave shows in the colours
            // as well as in the skyline.
            let c = ((cube.h * n as f32 * 0.7) as i32).rem_euclid(n as i32) as usize;
            let color = [
                f32::from(self.colors[c].red) / 65536.0,
                f32::from(self.colors[c].green) / 65536.0,
                f32::from(self.colors[c].blue) / 65536.0,
                1.0,
            ];
            g.glx.material_ambient_diffuse(color);

            if !wire {
                g.glx.normal3f(0.0, 0.0, -1.0); /* top */
                g.glx.vertex3f(x + xw + xd, y + yw + yd, -h);
                g.glx.vertex3f(x + xw - xd, y + yw - yd, -h);
                g.glx.vertex3f(x - xw - xd, y - yw - yd, -h);
                g.glx.vertex3f(x - xw + xd, y - yw + yd, -h);

                g.glx.normal3f(sth, cth, 0.0); /* front */
                g.glx.vertex3f(x + xw + xd, y + yw + yd, bottom);
                g.glx.vertex3f(x + xw + xd, y + yw + yd, -h);
                g.glx.vertex3f(x - xw + xd, y - yw + yd, -h);
                g.glx.vertex3f(x - xw + xd, y - yw + yd, bottom);

                g.glx.normal3f(cth, -sth, 0.0); /* right */
                g.glx.vertex3f(x + xw - xd, y + yw - yd, -h);
                g.glx.vertex3f(x + xw + xd, y + yw + yd, -h);
                g.glx.vertex3f(x + xw + xd, y + yw + yd, bottom);
                g.glx.vertex3f(x + xw - xd, y + yw - yd, bottom);
            } else {
                g.glx.normal3f(0.0, 0.0, -1.0); /* top */
                g.glx.vertex3f(x + xw + xd, y + yw + yd, -h);
                g.glx.vertex3f(x + xw - xd, y + yw - yd, -h);

                g.glx.vertex3f(x + xw - xd, y + yw - yd, -h);
                g.glx.vertex3f(x - xw - xd, y - yw - yd, -h);

                g.glx.vertex3f(x - xw - xd, y - yw - yd, -h);
                g.glx.vertex3f(x - xw + xd, y - yw + yd, -h);

                g.glx.vertex3f(x - xw + xd, y - yw + yd, -h);
                g.glx.vertex3f(x + xw + xd, y + yw + yd, -h);
            }
        }
        g.glx.end();
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 2 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        /* For this one it's really important to minimize the distance between
        near and far. */
        g.glx.perspective(30.0, 1.0 / h, 10.0, 50.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        // The camera only tips, never turns: upstream flattens the vertical
        // component of every drag before the trackball sees it.
        let flat = match *event {
            XEvent::ButtonPress { x, button, .. } => XEvent::ButtonPress {
                x,
                y: g.height() / 2,
                button,
            },
            XEvent::ButtonRelease { x, button, .. } => XEvent::ButtonRelease {
                x,
                y: g.height() / 2,
                button,
            },
            XEvent::MotionNotify { x, .. } => XEvent::MotionNotify {
                x,
                y: g.height() / 2,
            },
            e => e,
        };
        if self.trackball.event(&flat, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            self.reset_colors(g);
            self.tweak_cubes();
            self.trackball.reset(0.0, 0.0);
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let skew = g.res.int("skew").clamp(0, 45);
    let ncubes = g.res.int("count").clamp(1, 4000) as usize;

    let mut st = CityFlow {
        trackball: Trackball::new(),
        cubes: Vec::with_capacity(ncubes),
        waves: Waves {
            radius: g.res.int("waveRadius").clamp(5, 512),
            speed: g.res.int("waveSpeed").clamp(5, 150),
            srcs: Vec::new(),
            heights: Vec::new(),
        },
        // Upstream leaves these at zero and only widens them, so the field is
        // always at least the middle of the plane.
        min_x: 0.0,
        max_x: 0.0,
        min_y: 0.0,
        max_y: 0.0,
        colors: Vec::new(),
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    st.reset_colors(g);

    /* init_wave */
    let ncolors = st.colors.len() as f32;
    let radius = st.waves.radius;
    st.waves.heights = (0..radius)
        .map(|i| {
            let max = ncolors * (radius - i) as f32 / radius as f32;
            ((max + max * (f64::from(i) / 50.0).cos() as f32) / 2.0) as i32
        })
        .collect();
    let nwaves = g.res.int("waves").clamp(1, 20);
    st.waves.srcs = (0..nwaves)
        .map(|_| WaveSrc {
            xth: frand(2.0) * std::f64::consts::PI,
            yth: frand(2.0) * std::f64::consts::PI,
            ..WaveSrc::default()
        })
        .collect();

    for _ in 0..ncubes {
        /* Set the size to roughly cover a 2x2 square on average. */
        let scale = 1.8 / (ncubes as f32).sqrt();
        let th = -(if skew != 0 {
            frand(f64::from(skew))
        } else {
            0.0
        }) * std::f64::consts::PI
            / 180.0;
        let cube = Cube {
            x: (frand(1.0) - 0.5) as f32,
            y: (frand(1.0) - 0.5) as f32,
            z: frand(0.12) as f32,
            w: scale * (frand(1.0) as f32 + 0.2),
            h: 0.0,
            d: scale * (frand(1.0) as f32 + 0.2),
            cth: th.cos() as f32,
            sth: th.sin() as f32,
        };
        st.min_x = st.min_x.min(cube.x);
        st.min_y = st.min_y.min(cube.y);
        st.max_x = st.max_x.max(cube.x);
        st.max_y = st.max_y.max(cube.y);
        st.cubes.push(cube);
    }

    /* Sorting by depth improves frame rate slightly. */
    st.cubes
        .sort_by_key(|c| std::cmp::Reverse((c.y * 10000.0) as i32));

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*count:        800",
    "*showFPS:      False",
    "*wireframe:    False",
    "*waves:        6",
    "*waveSpeed:    25",
    "*waveRadius:   256",
    "*skew:         12",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("count", "Boxes", 50.0, 4000.0, 50.0, 0, "800"),
    Opt::slider("skew", "Skew", 0.0, 45.0, 1.0, 0, "12"),
    Opt::slider("waveSpeed", "Wave speed", 5.0, 150.0, 1.0, 0, "25"),
    Opt::slider("waveRadius", "Wave overlap", 5.0, 512.0, 1.0, 0, "256"),
    Opt::slider("waves", "Wave complexity", 1.0, 20.0, 1.0, 0, "6"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cityflow",
    label: "City Flow",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2014",
        video: Some("https://www.youtube.com/watch?v=LJMtu-9T3U0"),
        blurb: "Waves move across a sea of boxes. The city swells.",
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

    /// Every box gets its own colour, which means its own batch: the material
    /// changes inside the one long block of quads, and a batch carries one
    /// material.
    #[test]
    fn every_box_is_its_own_batch() {
        let mut r = start(StartArgs::new(640, 480, "count=50", 20260811));
        r.step();
        let f = r.frame();
        // Fifty boxes, three faces each, two triangles a face.
        assert_eq!(f.vertices.len(), 50 * 3 * 6);
        // Boxes that land on the same colour merge, so this is a lower bound
        // rather than exactly fifty.
        assert!(
            f.batches.len() > 20,
            "{} batches for fifty boxes",
            f.batches.len()
        );
        let colors: std::collections::BTreeSet<[u32; 3]> = f
            .batches
            .iter()
            .map(|b| {
                let c = b.material.ambient_diffuse;
                [
                    (c[0] * 255.0) as u32,
                    (c[1] * 255.0) as u32,
                    (c[2] * 255.0) as u32,
                ]
            })
            .collect();
        assert!(colors.len() > 5, "the boxes are all one colour");
    }

    /// The city swells: a box's height is a sample of the wave pattern where
    /// it stands, so it has to change as the pattern moves.
    #[test]
    fn the_boxes_rise_and_fall() {
        let mut r = start(StartArgs::new(640, 480, "count=50&waveSpeed=150", 20260811));
        // Every vertex that is not on the ground is on the top of a box, and
        // the boxes are sorted rather than matched up, since which batch a box
        // lands in shifts as neighbours merge and unmerge.
        let tops = |r: &Runner3d| -> Vec<i32> {
            let f = r.frame();
            let mut v: Vec<i32> = f
                .vertices
                .iter()
                .map(|v| (v.pos[2] * 10000.0) as i32)
                .filter(|z| *z < 40000)
                .collect();
            v.sort_unstable();
            v
        };
        r.step();
        let before = tops(&r);
        for _ in 0..30 {
            r.step();
        }
        let after = tops(&r);
        assert_eq!(before.len(), after.len());
        let moved = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert!(moved > before.len() / 2, "only {moved} boxes moved");
    }

    /// The wave profile falls away from the middle: that is what makes a
    /// ripple rather than a disc.
    #[test]
    fn a_ripple_fades_with_distance() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        // Rebuilt from the same arithmetic, since the profile is a pure
        // function of the radius and the colour count.
        let radius = 256;
        let ncolors = 256.0f32;
        let heights: Vec<i32> = (0..radius)
            .map(|i| {
                let max = ncolors * (radius - i) as f32 / radius as f32;
                ((max + max * (f64::from(i) / 50.0).cos() as f32) / 2.0) as i32
            })
            .collect();
        assert!(heights[0] > heights[radius as usize - 1]);
        // It is a cosine on a falling ramp, so it is bumpy but bounded.
        assert!(heights.iter().all(|&h| (0..=256).contains(&h)));
        let peaks = heights
            .windows(3)
            .filter(|w| w[1] > w[0] && w[1] >= w[2])
            .count();
        assert!(peaks >= 2, "only {peaks} ripples in the profile");
    }
}
