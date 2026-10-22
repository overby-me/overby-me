//! Port of `hacks/glx/glknots.c`.
//!
//! ```text
//! glknots, Copyright (c) 2003-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Generates some 3D knots (closed loops).
//! Inspired by Paul Bourke <pbourke@swin.edu.au> at
//! http://astronomy.swin.edu.au/~pbourke/curves/knot/
//! ```
//!
//! A closed loop of pipe, tied in a knot, spinning. After a few seconds it
//! shrinks to nothing, ties itself a different one, and grows back.
//!
//! There is no knot theory in here at all. The curve is a sum of sines and
//! cosines of nine random small integers, sampled eight hundred times, and the
//! solid is eight hundred short tubes laid end to end between consecutive
//! samples. Whether the result is a trefoil or a tangle or barely a knot is
//! left entirely to which integers came up.
//!
//! There are two families of curve. The first closes into a loop, because every
//! one of its terms has an integer period over the same turn. The second scales
//! one of its periods by a non-integer, so it does not quite close and the pipe
//! has two ends in it somewhere, which is upstream's arithmetic and not a slip
//! in the port.
//!
//! The joins are the interesting part. Consecutive tubes meet at an angle, and
//! two cut cylinders meeting at an angle leave a notch. Passing `dist/3` as the
//! cap size extends every tube by a third of its own length past both ends, so
//! the neighbours interpenetrate and there is no notch left to see. It costs
//! nothing and it is why the pipe reads as continuous.
//!
//! One in five knots is "blobby", which is not a different curve but a
//! different thickness rule: instead of a constant diameter, each tube's is the
//! cube of how far the curve moved, so the pipe swells where the curve is
//! moving fast and pinches to a thread where it is slow.
//!
//! One divergence: upstream skips the clear one time in fifteen, so that knot
//! smears across the screen as it turns. A WebGL canvas is cleared for us
//! between frames unless the context asks otherwise, and asking would cost
//! every other saver, so that one knot in fifteen simply does not smear.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::tube::tube;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent,
    random, screenhack_event_helper,
};

/// What the saver is doing: holding still, shrinking away, or growing back.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Out,
    In,
}

struct Knot {
    rot: Rotator,
    trackball: Trackball,

    knot_list: u32,

    colors: Vec<XColor>,
    ccolor: usize,

    mode: Mode,
    mode_tick: i32,
    clear_p: bool,

    /// When the current knot was tied, in seconds since the saver started.
    /// `None` until the first frame that looks at the clock.
    last_time: Option<f64>,
    draw_tick: i32,

    speed: f32,
    thickness: f32,
    segments: u32,
    duration: f64,
    wireframe: bool,
}

impl Knot {
    /// Tie one: nine random integers, eight hundred samples, and a tube
    /// between each pair of them.
    fn make_knot(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let diam = 4.0 * self.thickness;
        let faces = if wire { 3 } else { 6 };

        let mut p = [0.0f64; 9];
        let mut blobby_p = random().is_multiple_of(5);
        let curve = random() % 2;

        for v in &mut p {
            *v = f64::from(1 + random() % 4);
            if random().is_multiple_of(3) {
                *v += f64::from(random() % 5);
            }
        }

        if curve == 1 {
            p[0] += 4.0;
            p[1] *= (p[0] + p[0]) / 10.0;
            blobby_p = false;
        }

        let segments = self.segments;
        let (mut ox, mut oy, mut oz) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..=segments {
            let (x, y, z);
            if curve == 0 {
                let mu = f64::from(i) * (std::f64::consts::PI * 2.0) / f64::from(segments);
                x = 10.0 * ((mu).cos() + (p[0] * mu).cos()) + (p[1] * mu).cos() + (p[2] * mu).cos();
                y = 6.0 * mu.sin() + 10.0 * (p[3] * mu).sin();
                z = 16.0 * (p[4] * mu).sin() * (p[5] * mu / 2.0).sin() + p[6] * (p[7] * mu).sin()
                    - 2.0 * (p[8] * mu).sin();
            } else {
                let mu = f64::from(i) * (std::f64::consts::PI * 2.0) * p[0] / f64::from(segments);
                x = 10.0 * mu.cos() * (1.0 + (p[1] * mu / p[0]).cos() / 2.0);
                y = 25.0 * (p[1] * mu / p[0]).sin() / 2.0;
                z = 10.0 * mu.sin() * (1.0 + (p[1] * mu / p[0]).cos() / 2.0);
            }

            if i != 0 {
                let dist = ((x - ox).powi(2) + (y - oy).powi(2) + (z - oz).powi(2)).sqrt() as f32;
                let di = if blobby_p {
                    // Thickness follows speed along the curve, cubed, so the
                    // pipe swells and pinches instead of running true.
                    let di = dist * (segments as f32 / 500.0);
                    di * di * 3.0
                } else {
                    diam
                };
                tube(
                    &mut g.glx,
                    [ox as f32, oy as f32, oz as f32],
                    [x as f32, y as f32, z as f32],
                    di,
                    dist / 3.0,
                    faces,
                    true,
                    wire,
                    wire,
                );
            }

            ox = x;
            oy = y;
            oz = z;
        }
    }

    /// Tie a new one and give it new colours, brighter than the colormap makes
    /// them: a quarter of the colour plus half of white, so the pipe is always
    /// a pastel rather than a saturated one.
    fn new_knot(&mut self, g: &mut Gl) {
        self.clear_p = !random().is_multiple_of(15);

        self.colors = make_smooth_colormap(128);
        for c in &mut self.colors {
            /* make colors twice as bright */
            c.red = (c.red >> 2) + 0x7FFF;
            c.green = (c.green >> 2) + 0x7FFF;
            c.blue = (c.blue >> 2) + 0x7FFF;
        }

        g.glx.new_list(self.knot_list);
        self.make_knot(g);
        g.glx.end_list();
    }

    /// How many frames a shrink or a grow takes.
    fn mode_ticks(&self) -> i32 {
        (10.0 / self.speed.max(0.001)) as i32
    }
}

impl Hack3d for Knot {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let down = self.trackball.button_down();
        match self.mode {
            Mode::Normal => {
                self.draw_tick += 1;
                if self.draw_tick > 10 {
                    let now = g.time;
                    let last = *self.last_time.get_or_insert(now);
                    self.draw_tick = 0;
                    if !down && now - last >= self.duration {
                        self.mode = Mode::Out; /* go out */
                        self.mode_tick = self.mode_ticks();
                        self.last_time = Some(now);
                    }
                }
            }
            Mode::Out => {
                self.mode_tick -= 1;
                if self.mode_tick <= 0 {
                    self.new_knot(g);
                    self.mode_tick = self.mode_ticks();
                    self.mode = Mode::In; /* go in */
                }
            }
            Mode::In => {
                self.mode_tick -= 1;
                if self.mode_tick <= 0 {
                    self.mode = Mode::Normal; /* normal */
                }
            }
        }

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }

        if self.clear_p {
            g.glx.clear();
        }

        g.glx.push_matrix();

        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 15.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        let n = self.colors.len();
        let c = &self.colors[self.ccolor.min(n - 1)];
        let bcolor = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            1.0,
        ];
        self.ccolor = (self.ccolor + 1) % n;

        if self.wireframe {
            g.glx.color4f(bcolor[0], bcolor[1], bcolor[2], 1.0);
        } else {
            g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
            g.glx.material_shininess(128.0);
            g.glx.material_ambient_diffuse(bcolor);
        }

        g.glx.scale(0.25, 0.25, 0.25);

        if self.mode != Mode::Normal {
            let ticks = self.mode_ticks() as f32;
            let s = if self.mode == Mode::Out {
                self.mode_tick as f32 / ticks
            } else {
                (ticks - self.mode_tick as f32 + 1.0) / ticks
            };
            g.glx.scale(s, s, s);
        }

        g.glx.call_list(self.knot_list);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            // Upstream backdates the clock to a second after the epoch, which
            // is its way of saying "expired": the next tick ties a new knot.
            self.last_time = Some(f64::MIN);
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let spin = g.res.string("spin").to_string();
    let (mut spinx, mut spiny, mut spinz) = (false, false, false);
    for c in spin.chars() {
        match c {
            'x' | 'X' => spinx = true,
            'y' | 'Y' => spiny = true,
            'z' | 'Z' => spinz = true,
            // Upstream exits with a message. There is nowhere to exit to here,
            // and the panel only offers the eight it knows, so anything else
            // is simply no spin about that axis.
            _ => {}
        }
    }

    let spin_speed = 2.0;
    let wander_speed = 0.05;
    let spin_accel = 0.2;

    let mut st = Knot {
        rot: Rotator::new(
            if spinx { spin_speed } else { 0.0 },
            if spiny { spin_speed } else { 0.0 },
            if spinz { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            spinx && spiny && spinz,
        ),
        trackball: Trackball::new(),
        knot_list: 0,
        colors: Vec::new(),
        ccolor: 0,
        mode: Mode::Normal,
        mode_tick: 0,
        clear_p: true,
        last_time: None,
        draw_tick: 0,
        speed: g.res.float("speed") as f32,
        thickness: g.res.float("thickness").clamp(0.001, 1.0) as f32,
        segments: g.res.int("segments").max(10) as u32,
        duration: f64::from(g.res.int("duration")),
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        // After the reshape, so the light is fixed to the camera rather than
        // to the knot.
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    st.knot_list = g.glx.gen_lists(1);
    st.new_knot(g);
    g.glx.clear();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         XYZ",
    "*wander:       True",
    "*speed:        1.0",
    "*thickness:    0.3",
    "*segments:     800",
    "*duration:     8",
];

const SPINS: &[SelectItem] = &[
    SelectItem {
        value: "0",
        label: "Don't rotate",
    },
    SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    SelectItem {
        value: "XY",
        label: "Rotate around X and Y axes",
    },
    SelectItem {
        value: "XZ",
        label: "Rotate around X and Z axes",
    },
    SelectItem {
        value: "YZ",
        label: "Rotate around Y and Z axes",
    },
    SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 5.0, 0.01, 2, "1.0"),
    Opt::select("spin", "Rotation", SPINS, "XYZ"),
    Opt::slider("segments", "Resolution", 100.0, 2000.0, 10.0, 0, "800"),
    Opt::slider("thickness", "Thickness", 0.05, 1.0, 0.01, 2, "0.3"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glknots",
    label: "GL Knots",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=ILiYNkeEb_k"),
        blurb: "Generates some twisting 3d knot patterns, and spins them around.",
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

    /// The pipe is continuous: each tube starts where the one before it
    /// ended. A knot that jumps between samples is a dotted line.
    ///
    /// Not that it is a *closed* loop, though. The first family of curves does
    /// close, because every term has an integer period over the turn; the
    /// second scales one of its periods by a non-integer and so leaves the two
    /// ends apart, and that is upstream's own arithmetic rather than a slip
    /// here.
    #[test]
    fn the_tubes_are_laid_end_to_end() {
        let mut r = start(StartArgs::new(640, 480, "segments=200", 20260811));
        r.step();
        let f = r.frame();
        assert!(f.batches.len() > 100, "one batch per tube");
        // The cap size is a third of the tube's own length, so the geometry
        // runs from a third before the sample it starts at to a third past the
        // one it ends at. That puts the two samples at 0.2 and 0.8 of the way
        // along the drawn tube.
        let axis = |b: &crate::runtime::gl::Batch, t: f32| b.modelview.transform([0.0, t, 0.0]);
        let gap = |a: [f32; 3], b: [f32; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        let worst = f
            .batches
            .windows(2)
            .map(|w| gap(axis(&w[0], 0.8), axis(&w[1], 0.2)))
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1e-3,
            "a tube ends {worst} from where the next begins"
        );
    }

    /// Resolution is the number of tubes, so more of it is more geometry.
    #[test]
    fn resolution_does_something() {
        let count = |segments: &str| {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("segments={segments}"),
                20260811,
            ));
            r.step();
            r.frame().vertices.len()
        };
        assert!(count("1000") > count("200"));
    }

    /// It ties a new one after `duration` seconds, and the new one is not the
    /// old one.
    #[test]
    fn it_ties_a_new_knot_eventually() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "segments=200&duration=1",
            20260811,
        ));
        r.step();
        // How long the first few tubes are relative to each other. Not their
        // positions, which turn with the spin every frame, and not how many
        // there are, which is the same for every knot: the curve's own shape,
        // which only a new knot changes.
        let shape = |r: &Runner3d| -> Vec<i32> {
            let lens: Vec<f32> = r
                .frame()
                .batches
                .iter()
                .map(|b| {
                    let a = b.modelview.transform([0.0, 0.2, 0.0]);
                    let c = b.modelview.transform([0.0, 0.8, 0.0]);
                    ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2) + (a[2] - c[2]).powi(2)).sqrt()
                })
                .collect();
            // Relative to the first, so the shrinking and growing of a knot on
            // its way out and back does not read as a different curve.
            let unit = lens.first().copied().unwrap_or(1.0).max(1e-9);
            lens.iter()
                .take(8)
                .map(|l| (l / unit * 100.0) as i32)
                .collect()
        };
        let before = shape(&r);
        // Long enough to expire, shrink away, and grow back.
        for _ in 0..300 {
            r.step();
        }
        let after = shape(&r);
        assert_ne!(before, after, "the same knot came back");
    }
}
