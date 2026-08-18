//! Port of `hacks/glx/engine.c`.
//!
//! ```text
//! engine.c - GL representation of a 4 stroke engine
//!
//! Copyright (C) 2001 Ben Buxton (bb@cactii.net)
//! modified by Ed Beroset (beroset@mindspring.com)
//!  - command line argument to specify number of cylinders
//!  - command line argument to specify included angle of engine
//!  - included crankshaft shapes and firing orders for real engines
//!    verified using the Bosch _Automotive Handbook_, 5th edition, pp 402,403
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
//! A four-stroke engine, cut away, turning over.
//!
//! It is one crankshaft angle and a table. Ten real engines are stored as their
//! cylinder count, the angle between their two banks, and the crank angle at
//! which each cylinder fires; everything on screen follows from those. A piston
//! is at `cos(a) + sqrt(25 - sin(a)^2)` up its bore, which is the exact
//! solution for a rod five times the crank throw, and its connecting rod is
//! drawn at the angle and length that same triangle gives. The three tables of
//! sine, cosine and rod geometry are computed once for all 720 crank degrees,
//! because a twelve-cylinder engine would otherwise want a couple of hundred
//! trigonometric calls a frame.
//!
//! Firing is not simulated either: a cylinder fires when the crank reaches the
//! angle its row of the table names. The flash is a translucent rod that grows
//! and fades over a few frames, with a second light source inside it, so the
//! whole block lights up from within as each cylinder goes.
//!
//! The block itself is drawn last, in translucent yellow with depth writes off,
//! so the machinery inside stays visible through it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

const HALFREV: i32 = 180;
const ONEREV: i32 = 360;
const TWOREV: i32 = 720;

const LIGHTPOS: [f32; 4] = [7.0, 7.0, 12.0, 1.0];
const LIGHT_SP: [f32; 4] = [0.8, 0.8, 0.8, 0.5];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const YELLOW_T: [f32; 4] = [1.0, 1.0, 0.0, 0.4];

/// One engine: how many cylinders, the angle between the banks, and the crank
/// angle at which each cylinder fires.
struct EngineType {
    cylinders: usize,
    included_angle: i32,
    piston_angle: [i32; 12],
    /// Step in crank degrees per frame.
    speed: i32,
    name: &'static str,
}

/// The firing order and included angle of each engine, renumbered from the
/// flywheel back so that cylinder zero always fires first. Upstream's table,
/// checked against the Bosch handbook.
const ENGINES: &[EngineType] = &[
    EngineType {
        cylinders: 3,
        included_angle: 0,
        piston_angle: [0, 240, 480, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "Honda Insight",
    },
    EngineType {
        cylinders: 4,
        included_angle: 0,
        piston_angle: [0, 180, 540, 360, 0, 0, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "BMW M3",
    },
    EngineType {
        cylinders: 4,
        included_angle: 180,
        piston_angle: [0, 360, 180, 540, 0, 0, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "VW Beetle",
    },
    EngineType {
        cylinders: 5,
        included_angle: 0,
        piston_angle: [0, 576, 144, 432, 288, 0, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "Audi Quattro",
    },
    EngineType {
        cylinders: 6,
        included_angle: 0,
        piston_angle: [0, 240, 480, 120, 600, 360, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "BMW M5",
    },
    EngineType {
        cylinders: 6,
        included_angle: 90,
        piston_angle: [0, 360, 480, 120, 240, 600, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "Subaru XT",
    },
    EngineType {
        cylinders: 6,
        included_angle: 180,
        piston_angle: [0, 360, 240, 600, 480, 120, 0, 0, 0, 0, 0, 0],
        speed: 12,
        name: "Porsche 911",
    },
    EngineType {
        cylinders: 8,
        included_angle: 90,
        piston_angle: [0, 450, 90, 180, 270, 360, 540, 630, 0, 0, 0, 0],
        speed: 15,
        name: "Corvette Z06",
    },
    EngineType {
        cylinders: 10,
        included_angle: 90,
        piston_angle: [0, 72, 432, 504, 288, 360, 144, 216, 576, 648, 0, 0],
        speed: 12,
        name: "Dodge Viper",
    },
    EngineType {
        cylinders: 12,
        included_angle: 60,
        piston_angle: [0, 300, 240, 540, 480, 60, 120, 420, 600, 180, 360, 660],
        speed: 12,
        name: "Jaguar XKE",
    },
];

struct Engine {
    rot: Rotator,
    trackball: Trackball,
    font: Option<TexFont>,
    engine_name: String,
    engine_type: usize,

    crank_offset: f32,
    crank_width: f32,

    /// Sine and cosine of every whole degree of two revolutions, so a twelve
    /// cylinder engine does not want two hundred trig calls a frame.
    sin_table: Vec<f32>,
    cos_table: Vec<f32>,
    /// How far up its bore a piston is at each crank angle, how long the
    /// connecting rod appears, and at what angle it lies.
    yp: Vec<f32>,
    ln: Vec<f32>,
    ang: Vec<f32>,

    boom_red: [f32; 4],
    boom_lpos: [f32; 4],
    boom_d: f32,
    boom_wd: f32,
    boom_time: i32,
    last_plug: usize,

    viewer: [f32; 3],
    lookat: [f32; 3],
    display_a: i32,

    move_p: bool,
    spin_p: bool,
    do_titles: bool,
    /// The extra scale a narrow window needs.
    scale: f32,
}

impl Engine {
    fn eng(&self) -> &'static EngineType {
        &ENGINES[self.engine_type]
    }

    /// `cylinder`. A tube or a solid rod along the x axis, in absolute
    /// coordinates rather than under a matrix, which is why a run of them
    /// folds into one batch.
    ///
    /// `endcaps` is 0 for none, 1 for the left, 2 for the right and 3 for
    /// both; the angles say how far round the axis to go.
    #[allow(clippy::too_many_arguments)]
    fn cylinder(
        &self,
        g: &mut Gl,
        x: f32,
        y: f32,
        z: f32,
        length: f32,
        outer: f32,
        inner: f32,
        endcaps: i32,
        sang: i32,
        eang: i32,
    ) {
        // Upstream computes a segment count from the window size and then
        // clamps it up to forty, which makes the first two lines dead.
        let nsegs = 40;
        let step = ONEREV / nsegs;
        let tube = inner < outer && endcaps < 3;

        let mut y2c = vec![0.0f32; TWOREV as usize + 1];
        let mut z2c = vec![0.0f32; TWOREV as usize + 1];

        let mut z1 = self.cos_table[sang as usize] * outer + z;
        let mut y1 = self.sin_table[sang as usize] * outer + y;
        let mut big_z1 = self.cos_table[sang as usize] * inner + z;
        let mut big_y1 = self.sin_table[sang as usize] * inner + y;
        let mut big_z2 = z;
        let mut big_y2 = y;
        let xl = x + length;

        let (mut y2, mut z2) = (0.0f32, 0.0f32);
        g.glx.begin(Shape::Quads);
        let mut a = sang;
        let mut b = 0;
        while a <= eang || b <= eang {
            y2 = outer * self.sin_table[a as usize] + y;
            z2 = outer * self.cos_table[a as usize] + z;
            if endcaps != 0 {
                y2c[a as usize] = y2;
                z2c[a as usize] = z2;
            }
            if tube {
                big_y2 = inner * self.sin_table[a as usize] + y;
                big_z2 = inner * self.cos_table[a as usize] + z;
            }

            g.glx.normal3f(0.0, y1, z1);
            g.glx.vertex3f(x, y1, z1);
            g.glx.vertex3f(xl, y1, z1);
            g.glx.normal3f(0.0, y2, z2);
            g.glx.vertex3f(xl, y2, z2);
            g.glx.vertex3f(x, y2, z2);

            if a == sang && eang - sang < ONEREV {
                if tube {
                    g.glx.vertex3f(x, big_y1, big_z1);
                } else {
                    g.glx.vertex3f(x, y, z);
                }
                g.glx.vertex3f(x, y1, z1);
                g.glx.vertex3f(xl, y1, z1);
                if tube {
                    // Upstream writes Z1 twice here, which puts this corner
                    // somewhere it did not mean; kept, because the face it
                    // makes is part of what the engine looks like.
                    g.glx.vertex3f(xl, big_z1, big_z1);
                } else {
                    g.glx.vertex3f(xl, y, z);
                }
            }

            if tube {
                if endcaps != 1 {
                    g.glx.normal3f(-1.0, 0.0, 0.0); // left end
                    g.glx.vertex3f(x, y1, z1);
                    g.glx.vertex3f(x, y2, z2);
                    g.glx.vertex3f(x, big_y2, big_z2);
                    g.glx.vertex3f(x, big_y1, big_z1);
                }
                g.glx.normal3f(0.0, -big_y1, -big_z1); // inner surface
                g.glx.vertex3f(x, big_y1, big_z1);
                g.glx.vertex3f(xl, big_y1, big_z1);
                g.glx.normal3f(0.0, -big_y2, -big_z2);
                g.glx.vertex3f(xl, big_y2, big_z2);
                g.glx.vertex3f(x, big_y2, big_z2);
                if endcaps != 2 {
                    g.glx.normal3f(1.0, 0.0, 0.0); // right end
                    g.glx.vertex3f(xl, y1, z1);
                    g.glx.vertex3f(xl, y2, z2);
                    g.glx.vertex3f(xl, big_y2, big_z2);
                    g.glx.vertex3f(xl, big_y1, big_z1);
                }
            }

            z1 = z2;
            y1 = y2;
            big_z1 = big_z2;
            big_y1 = big_y2;
            b = a;
            a += step;
        }
        g.glx.end();

        // The flat face that closes a partial sweep.
        if eang - sang < ONEREV {
            let n = face_normal([x, y1, z1], [x, y, z], [xl, y1, z1]);
            g.glx.begin(Shape::Quads);
            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx.vertex3f(x, y, z);
            g.glx.vertex3f(x, y1, z1);
            g.glx.vertex3f(xl, y1, z1);
            g.glx.vertex3f(xl, y, z);
            g.glx.end();
        }

        if endcaps == 0 {
            return;
        }

        let (start, end, mut norm) = if tube {
            match endcaps {
                1 => (0.0, 0.0, 1.0),
                2 => (length + 0.01, length + 0.01, 1.0),
                _ => (-0.01, length + 0.02, 1.0),
            }
        } else {
            (0.0, length, -1.0)
        };

        let mut ex = start;
        while ex <= end {
            let mut z1 = outer * self.cos_table[sang as usize] + z;
            let mut y1 = y + self.sin_table[sang as usize] * outer;
            g.glx.begin(Shape::Triangles);
            let mut a = sang;
            let mut b = 0;
            while a <= eang || b <= eang {
                g.glx.normal3f(norm, 0.0, 0.0);
                g.glx.vertex3f(x + ex, y, z);
                g.glx.vertex3f(x + ex, y1, z1);
                g.glx.vertex3f(x + ex, y2c[a as usize], z2c[a as usize]);
                y1 = y2c[a as usize];
                z1 = z2c[a as usize];
                b = a;
                a += step;
            }
            if !tube {
                norm = 1.0;
            }
            g.glx.end();
            let _ = (y2, z2, b);
            ex += length;
            if length == 0.0 {
                break;
            }
        }
    }

    /// `rod`: a solid cylinder.
    fn rod(&self, g: &mut Gl, x: f32, y: f32, z: f32, length: f32, diameter: f32) {
        self.cylinder(g, x, y, z, length, diameter, diameter, 3, 0, ONEREV);
    }

    /// `CrankBit`: one cheek of the crankshaft.
    fn crank_bit(&self, g: &mut Gl, x: f32) {
        rect(g, x, -1.4, 0.5, 0.2, 1.8, 1.0);
        self.cylinder(g, x, -0.5, 0.0, 0.2, 2.0, 2.0, 1, 60, 120);
    }

    /// `makeshaft`: the flywheel, the shaft between the cranks, and a wrist pin
    /// and pair of cheeks per cylinder.
    fn make_shaft(&self, g: &mut Gl) {
        let crank_thick = 0.2;
        let crank_diam = 0.3;
        let eng = self.eng();

        g.glx.material_ambient_diffuse(BLUE);
        self.cylinder(g, -2.5, 0.0, 0.0, 1.0, 3.0, 2.5, 0, 0, ONEREV);
        rect(g, -2.0, -0.3, 2.8, 0.5, 0.6, 5.6);
        rect(g, -2.0, -2.8, 0.3, 0.5, 5.6, 0.6);

        // The first crankshaft bit is always two units long, from the flywheel.
        self.rod(g, -2.0, 0.0, 0.0, 2.0, crank_diam);

        let mut j = 0;
        while j + 1 < eng.cylinders {
            self.rod(
                g,
                self.crank_width - crank_thick + self.crank_offset * j as f32,
                0.0,
                0.0,
                self.crank_offset - self.crank_width + 2.0 * crank_thick,
                crank_diam,
            );
            j += 1;
        }
        // The last bit connects to the engine wall on the non-flywheel end.
        self.rod(
            g,
            self.crank_width - crank_thick + self.crank_offset * j as f32,
            0.0,
            0.0,
            0.9,
            crank_diam,
        );

        for j in 0..eng.cylinders {
            g.glx.push_matrix();
            let extra = if j & 1 != 0 { eng.included_angle } else { 0 };
            g.glx.rotate(
                (HALFREV + eng.piston_angle[j] + extra) as f32,
                1.0,
                0.0,
                0.0,
            );
            g.glx.material_ambient_diffuse(BLUE);
            self.rod(
                g,
                self.crank_offset * j as f32,
                -1.0,
                0.0,
                self.crank_width,
                crank_diam,
            );
            g.glx.material_ambient_diffuse(GREEN);
            self.crank_bit(g, self.crank_offset * j as f32);
            self.crank_bit(
                g,
                self.crank_width - crank_thick + self.crank_offset * j as f32,
            );
            g.glx.pop_matrix();
        }
    }

    /// `makepiston`: the body and its two rings.
    fn make_piston(&self, g: &mut Gl) {
        g.glx.rotate(90.0, 0.0, 0.0, 1.0);
        let colour = [0.6, 0.6, 0.6, 1.0];
        g.glx.material_ambient_diffuse(colour);
        g.glx.material_specular(colour);
        g.glx.material_shininess(20.0);
        self.cylinder(g, 0.0, 0.0, 0.0, 2.0, 1.0, 0.7, 2, 0, ONEREV);
        g.glx.material_ambient_diffuse([0.2, 0.2, 0.2, 1.0]);
        self.cylinder(g, 1.6, 0.0, 0.0, 0.1, 1.05, 1.05, 0, 0, ONEREV);
        self.cylinder(g, 1.8, 0.0, 0.0, 0.1, 1.05, 1.05, 0, 0, ONEREV);
    }

    /// `boom`: the flash inside a cylinder that has just fired, with a light
    /// source in it so the block glows from within.
    fn boom(&mut self, g: &mut Gl, x: f32, y: f32, s: bool) {
        let eng = self.eng();
        let flame_out = TWOREV / eng.speed / eng.cylinders as i32;

        if self.boom_time == 0 && s {
            self.boom_red[0] = 0.0;
            self.boom_red[1] = 0.0;
            self.boom_d = 0.05;
            self.boom_time += 1;
            g.glx.light_enable(1, true);
        } else if self.boom_time == 0 && !s {
            return;
        } else if self.boom_time >= 8 && self.boom_time < flame_out && !s {
            self.boom_time += 1;
            self.boom_red[0] -= 0.2;
            self.boom_red[1] -= 0.1;
            self.boom_d -= 0.04;
        } else if self.boom_time >= flame_out {
            self.boom_time = 0;
            g.glx.light_enable(1, false);
            return;
        } else {
            self.boom_red[0] += 0.2;
            self.boom_red[1] += 0.1;
            self.boom_d += 0.04;
            self.boom_time += 1;
        }

        self.boom_lpos[0] = x - self.boom_d;
        self.boom_lpos[1] = y;
        g.glx.light_position(
            1,
            self.boom_lpos[0],
            self.boom_lpos[1],
            self.boom_lpos[2],
            self.boom_lpos[3],
        );
        g.glx.light_diffuse(1, self.boom_red);
        g.glx.light_specular(1, self.boom_red);

        g.glx.material_ambient_diffuse(self.boom_red);
        self.boom_wd = (self.boom_d * 3.0).min(0.7);
        g.glx.blend(Blend::Alpha);
        g.glx.depth_mask(false);
        self.rod(g, x, y, 0.0, self.boom_d, self.boom_wd);
        g.glx.depth_mask(true);
        g.glx.blend(Blend::Off);
    }
}

/// `Rect`: a box, given a corner and three sizes.
fn rect(g: &mut Gl, x: f32, y: f32, z: f32, w: f32, h: f32, t: f32) {
    let (yh, xw, zt) = (y + h, x + w, z - t);
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [[x, y, z], [x, yh, z], [xw, yh, z], [xw, y, z]],
        ),
        (
            [0.0, 0.0, -1.0],
            [[x, y, zt], [x, yh, zt], [xw, yh, zt], [xw, y, zt]],
        ),
        (
            [0.0, 1.0, 0.0],
            [[x, yh, z], [x, yh, zt], [xw, yh, zt], [xw, yh, z]],
        ),
        (
            [0.0, -1.0, 0.0],
            [[x, y, z], [x, y, zt], [xw, y, zt], [xw, y, z]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [[x, y, z], [x, y, zt], [x, yh, zt], [x, yh, z]],
        ),
        (
            [1.0, 0.0, 0.0],
            [[xw, y, z], [xw, y, zt], [xw, yh, zt], [xw, yh, z]],
        ),
    ];
    g.glx.begin(Shape::Quads);
    for (n, vs) in faces {
        g.glx.normal3f(n[0], n[1], n[2]);
        for v in vs {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

/// `normal`: the cross product of two edges, unnormalised as upstream leaves
/// it.
fn face_normal(v1: [f32; 3], v2: [f32; 3], v3: [f32; 3]) -> [f32; 3] {
    let (x, y, z) = (v2[0] - v1[0], v2[1] - v1[1], v2[2] - v1[2]);
    let (big_x, big_y, big_z) = (v3[0] - v1[0], v3[1] - v1[1], v3[2] - v1[2]);
    [
        big_y * z - big_z * y,
        big_z * x - big_x * z,
        big_x * y - big_y * x,
    ]
}

/// `find_engine`. The names in the panel carry underscores, which upstream
/// turns into spaces before matching.
fn find_engine(name: &str) -> usize {
    if name.is_empty() || name.eq_ignore_ascii_case("(none)") {
        return (random() as usize) % ENGINES.len();
    }
    let name = name.replace(['-', '_'], " ");
    ENGINES
        .iter()
        .position(|e| e.name.eq_ignore_ascii_case(&name))
        .unwrap_or_else(|| (random() as usize) % ENGINES.len())
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let move_p = g.res.bool("move");
    let spin_p = g.res.bool("spin");

    let n = TWOREV as usize;
    let f = ONEREV as f32 / (std::f32::consts::PI * 2.0);
    let sin_table: Vec<f32> = (0..n).map(|i| (i as f32 / f).sin()).collect();
    let cos_table: Vec<f32> = (0..n).map(|i| (i as f32 / f).cos()).collect();

    // The piston is where the rod, five times the crank throw, puts it.
    let mut yp = vec![0.0f32; n];
    let mut ln = vec![0.0f32; n];
    let mut ang = vec![0.0f32; n];
    for i in 0..n {
        let zb = sin_table[i];
        let yb = cos_table[i];
        yp[i] = yb + (25.0 - zb * zb).sqrt();
        ln[i] = (zb * zb + (yb - yp[i]) * (yb - yp[i])).sqrt();
        ang[i] = -((zb / 5.0).asin() * 57.0);
    }

    let engine_type = find_engine(g.res.string("engine"));
    let eng = &ENGINES[engine_type];
    let engine_name = format!(
        "{}\n{}{}{}",
        eng.name,
        match eng.included_angle {
            0 => "",
            180 => "Flat ",
            _ => "V",
        },
        eng.cylinders,
        if eng.included_angle == 0 {
            " Cylinder"
        } else {
            ""
        }
    );

    let mut crank_offset = 3.3;
    if eng.included_angle != 0 {
        crank_offset /= 2.0;
    }

    let mut this = Engine {
        rot: Rotator::new(
            if spin_p { 0.5 } else { 0.0 },
            if spin_p { 0.5 } else { 0.0 },
            if spin_p { 0.5 } else { 0.0 },
            1.0,
            if move_p { 0.01 } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        font: Some(TexFont::load(&mut g.glx, "sans-serif 18")),
        engine_name,
        engine_type,
        crank_offset,
        crank_width: 1.5,
        sin_table,
        cos_table,
        yp,
        ln,
        ang,
        boom_red: [0.0, 0.0, 0.0, 0.9],
        boom_lpos: [0.0, 0.0, 0.0, 1.0],
        boom_d: 0.0,
        boom_wd: 0.0,
        boom_time: 0,
        last_plug: usize::MAX,
        viewer: [0.0, 2.0, 30.0],
        lookat: [0.0, 0.0, 0.0],
        display_a: 0,
        move_p,
        spin_p,
        do_titles: g.res.bool("titles"),
        scale: 1.0,
    };
    if !move_p {
        this.viewer = [0.0, 2.0, 30.0];
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Engine {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            // Tiny window: show the middle.
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(40.0, 1.0 / h, 1.5, 70.0);
        g.glx.matrix_mode_modelview();
        self.scale = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            // Upstream restarts itself with a random engine.
            self.engine_type = (random() as usize) % ENGINES.len();
            let eng = self.eng();
            self.crank_offset = if eng.included_angle != 0 { 1.65 } else { 3.3 };
            self.engine_name = format!(
                "{}\n{}{}{}",
                eng.name,
                match eng.included_angle {
                    0 => "",
                    180 => "Flat ",
                    _ => "V",
                },
                eng.cylinders,
                if eng.included_angle == 0 {
                    " Cylinder"
                } else {
                    ""
                }
            );
            self.display_a = 0;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);

        g.glx.load_identity();
        g.glx.look_at(self.viewer, self.lookat, [0.0, 1.0, 0.0]);
        g.glx.push_matrix();
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx
            .light_position(0, LIGHTPOS[0], LIGHTPOS[1], LIGHTPOS[2], LIGHTPOS[3]);
        g.glx.light_specular(0, LIGHT_SP);
        g.glx.light_diffuse(0, LIGHT_SP);

        let down = self.trackball.button_down();
        if self.move_p {
            let (x, y, z) = self.rot.position(!down);
            g.glx.translate(
                x as f32 * 16.0 - 9.0,
                y as f32 * 14.0 - 7.0,
                z as f32 * 16.0 - 10.0,
            );
        }
        if self.spin_p {
            let m = self.trackball.matrix();
            g.glx.mult_matrix(m);
            let (x, y, _z) = self.rot.rotation(!down);
            g.glx.rotate(x as f32 * ONEREV as f32, 1.0, 0.0, 0.0);
            g.glx.rotate(y as f32 * ONEREV as f32, 0.0, 1.0, 0.0);
            // Upstream passes x again here rather than z.
            g.glx.rotate(x as f32 * ONEREV as f32, 0.0, 0.0, 1.0);
        }

        // So the rotation appears around the centre of the engine.
        g.glx.translate(-5.0, 0.0, 0.0);

        g.glx.push_matrix();
        g.glx.rotate(self.display_a as f32, 1.0, 0.0, 0.0);
        self.make_shaft(g);
        g.glx.pop_matrix();

        let eng = self.eng();
        let sides = if eng.included_angle == 0 { 1 } else { 2 };
        let (cw, co) = (self.crank_width, self.crank_offset);

        g.glx.push_matrix();
        for half in 0..sides {
            if half > 0 {
                g.glx.rotate(eng.included_angle as f32, 1.0, 0.0, 0.0);
            }

            // Pistons.
            let mut j = half;
            while j < eng.cylinders {
                let b = ((self.display_a + eng.piston_angle[j]) % ONEREV) as usize;
                g.glx.push_matrix();
                g.glx
                    .translate(cw / 2.0 + co * j as f32, self.yp[b] - 0.3, 0.0);
                self.make_piston(g);
                g.glx.pop_matrix();
                j += sides;
            }

            // Spark plugs.
            g.glx.push_matrix();
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            g.glx.material_ambient_diffuse(RED);
            let mut j = half;
            while j < eng.cylinders {
                self.cylinder(
                    g,
                    8.5,
                    -cw / 2.0 - co * j as f32,
                    0.0,
                    0.5,
                    0.4,
                    0.3,
                    1,
                    0,
                    ONEREV,
                );
                j += sides;
            }
            g.glx.material_ambient_diffuse(WHITE);
            let mut j = half;
            while j < eng.cylinders {
                self.rod(g, 8.0, -cw / 2.0 - co * j as f32, 0.0, 0.5, 0.2);
                self.rod(g, 9.0, -cw / 2.0 - co * j as f32, 0.0, 1.0, 0.15);
                j += sides;
            }

            // Connecting rods.
            g.glx.material_ambient_diffuse(BLUE);
            let mut j = half;
            while j < eng.cylinders {
                let b = ((self.display_a + HALFREV + eng.piston_angle[j]) % TWOREV) as usize;
                g.glx.push_matrix();
                g.glx.rotate(self.ang[b], 0.0, 1.0, 0.0);
                self.rod(
                    g,
                    -self.cos_table[b],
                    -cw / 2.0 - co * j as f32,
                    -self.sin_table[b],
                    self.ln[b],
                    0.2,
                );
                g.glx.pop_matrix();
                j += sides;
            }
            g.glx.pop_matrix();

            // The block, translucent so the machinery stays visible inside it.
            g.glx.material_ambient_diffuse(YELLOW_T);
            g.glx.blend(Blend::Alpha);
            g.glx.depth_mask(false);
            let right_side = if sides > 1 { 0.0 } else { 1.6 };
            let span = cw / 2.0 + 0.1 + co * eng.cylinders as f32 - right_side;
            rect(g, -cw / 2.0, -0.5, 1.0, 0.2, 9.0, 2.0); // left plate
            rect(
                g,
                0.3 + co * eng.cylinders as f32 - right_side,
                -0.5,
                1.0,
                0.2,
                9.0,
                2.0,
            ); // right plate
            rect(g, -cw / 2.0 + 0.2, 8.3, 1.0, span, 0.2, 2.0); // head plate
            rect(g, -cw / 2.0 + 0.2, 3.0, 1.0, span, 0.2, 0.2); // front rail
            rect(g, -cw / 2.0 + 0.2, 3.0, -1.0 + 0.2, span, 0.2, 0.2); // back rail
            let last = eng.cylinders - usize::from(sides == 1);
            let mut j = 0;
            while j < last {
                rect(
                    g,
                    0.4 + cw + co * (j as f32 - half as f32),
                    3.0,
                    1.0,
                    1.0,
                    5.3,
                    2.0,
                );
                j += sides;
            }
            g.glx.depth_mask(true);
        }
        g.glx.pop_matrix();

        // Which plug fires now, if any.
        let cylinders = eng.cylinders;
        let included = eng.included_angle;
        let mut fired = None;
        for j in 0..cylinders {
            if (self.display_a + eng.piston_angle[j]) % TWOREV == 0 {
                fired = Some(j);
            }
        }
        if let Some(j) = fired {
            g.glx.push_matrix();
            if j & 1 != 0 {
                g.glx.rotate(included as f32, 1.0, 0.0, 0.0);
            }
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            self.boom(g, 8.0, -cw / 2.0 - co * j as f32, true);
            self.last_plug = j;
            g.glx.pop_matrix();
        } else if self.last_plug != usize::MAX {
            // The last explosion dims gradually.
            if self.last_plug & 1 != 0 {
                g.glx.rotate(included as f32, 1.0, 0.0, 0.0);
            }
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            let plug = self.last_plug;
            self.boom(g, 8.0, -cw / 2.0 - co * plug as f32, false);
        }
        g.glx.blend(Blend::Off);

        self.display_a += self.eng().speed;
        if self.display_a >= TWOREV {
            self.display_a = 0;
        }
        g.glx.pop_matrix();

        if self.do_titles
            && let Some(font) = &self.font
        {
            let (w, h) = (g.width(), g.height());
            font.print_label(&mut g.glx, &self.engine_name, w, h, 1, [1.0, 1.0, 0.0, 1.0]);
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*suppressRotationAnimation: True",
    "*titleFont:    sans-serif 18",
    "*engine:       (none)",
    "*titles:       False",
    "*spin:         True",
    "*move:         True",
];

const MODELS: &[SelectItem] = &[
    SelectItem {
        value: "(none)",
        label: "Random engine",
    },
    SelectItem {
        value: "honda_insight",
        label: "Honda Insight (3 cylinders)",
    },
    SelectItem {
        value: "bmw_m3",
        label: "BMW M3 (4 cylinders)",
    },
    SelectItem {
        value: "vw_beetle",
        label: "VW Beetle (4 cylinders, flat)",
    },
    SelectItem {
        value: "audi_quattro",
        label: "Audi Quattro (5 cylinders)",
    },
    SelectItem {
        value: "bmw_m5",
        label: "BMW M5 (6 cylinders)",
    },
    SelectItem {
        value: "subaru_xt",
        label: "Subaru XT (6 cylinders, V)",
    },
    SelectItem {
        value: "porsche_911",
        label: "Porsche 911 (6 cylinders, flat)",
    },
    SelectItem {
        value: "corvette_z06",
        label: "Corvette Z06 (8 cylinders, V)",
    },
    SelectItem {
        value: "dodge_viper",
        label: "Dodge Viper (10 cylinders, V)",
    },
    SelectItem {
        value: "jaguar_xke",
        label: "Jaguar XKE (12 cylinders, V)",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::select("engine", "Engine", MODELS, "(none)"),
    Opt::boolean("titles", "Show engine name", "false"),
    Opt::boolean("move", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "engine",
    label: "Engine",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ben Buxton and Ed Beroset",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=8BL2o8QJmiA"),
        blurb: "A four-stroke engine, cut away, with the firing orders of ten \
                real engines.",
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

    #[test]
    fn every_engine_fires_each_cylinder_once_in_two_revolutions() {
        // The table is the whole of the simulation, so it is worth checking
        // that it says something coherent: a four-stroke engine fires every
        // cylinder exactly once per two turns, evenly spaced.
        for e in ENGINES {
            let mut angles: Vec<i32> = e.piston_angle[..e.cylinders].to_vec();
            assert!(
                angles.iter().all(|a| (0..TWOREV).contains(a)),
                "{} fires outside two revolutions",
                e.name
            );
            angles.sort_unstable();
            angles.dedup();
            assert_eq!(
                angles.len(),
                e.cylinders,
                "{} fires two cylinders at once",
                e.name
            );
            // Evenly spaced: two revolutions divided by the cylinder count.
            let step = TWOREV / e.cylinders as i32;
            for (i, a) in angles.iter().enumerate() {
                assert_eq!(*a, i as i32 * step, "{} is not evenly spaced", e.name);
            }
        }
    }

    #[test]
    fn a_piston_travels_twice_the_crank_throw() {
        // The piston is at cos(a) + sqrt(25 - sin(a)^2), which is the exact
        // solution for a rod five times the throw. Top and bottom differ by
        // two, whatever the rod does in between.
        let mut r = start(StartArgs::new(640, 480, "engine=bmw_m3", 20260811));
        r.step();
        let f = std::f32::consts::PI * 2.0 / ONEREV as f32;
        let yp = |a: i32| {
            let zb = (a as f32 * f).sin();
            let yb = (a as f32 * f).cos();
            yb + (25.0 - zb * zb).sqrt()
        };
        assert!((yp(0) - yp(180) - 2.0).abs() < 1e-4);
        // And it is never outside that range in between.
        for a in 0..ONEREV {
            assert!(yp(a) <= yp(0) + 1e-4 && yp(a) >= yp(180) - 1e-4);
        }
    }

    #[test]
    fn a_v_engine_has_two_banks_and_an_inline_one_has_one() {
        let banks = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            let f = r.frame();
            // Each bank is drawn under its own rotation about x, so the
            // distinct modelviews count them.
            let mut seen: Vec<[u32; 16]> = f
                .batches
                .iter()
                .map(|b| std::array::from_fn(|i| b.modelview.0[i].to_bits()))
                .collect();
            seen.sort_unstable();
            seen.dedup();
            seen.len()
        };
        // A flat six has both banks and more distinct positions than a
        // straight four with the same number of parts per bank.
        assert!(
            banks("engine=porsche_911") > banks("engine=bmw_m3"),
            "the flat six is not drawn in two banks"
        );
    }

    #[test]
    fn the_block_is_translucent_and_does_not_write_depth() {
        let mut r = start(StartArgs::new(640, 480, "engine=bmw_m3", 20260811));
        r.step();
        let f = r.frame();
        let block: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.material.ambient_diffuse == YELLOW_T)
            .collect();
        assert!(!block.is_empty(), "the block was never drawn");
        for b in &block {
            assert!(!b.depth_mask, "the block wrote depth");
            assert_eq!(b.blend, Blend::Alpha);
        }
    }

    #[test]
    fn each_cylinder_fires_as_the_crank_reaches_its_angle() {
        let mut r = start(StartArgs::new(640, 480, "engine=bmw_m3", 20260811));
        let mut booms = 0;
        // Two revolutions at twelve degrees a frame is sixty frames, in which
        // a four cylinder fires four times.
        for _ in 0..60 {
            r.step();
            let f = r.frame();
            // The flash is the only thing drawn with a red-ish material that
            // is not the spark plugs' flat red.
            if f.batches.iter().any(|b| {
                let m = b.material.ambient_diffuse;
                m[3] > 0.8 && m[3] < 0.95 && m[0] > 0.0
            }) {
                booms += 1;
            }
        }
        assert!(booms > 0, "nothing ever fired");
    }
}
