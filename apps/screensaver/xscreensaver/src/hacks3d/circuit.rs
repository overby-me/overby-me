//! Port of `hacks/glx/circuit.c`.
//!
//! ```text
//! circuit - Random electronic components floating around
//!
//! version 1.4
//!
//! Since version 1.1: added to-220 transistor, added fuse
//! Since version 1.2: random display digits, LED improvements (flickering)
//! Since version 1.3: ICs look better, font textures, improved normals to
//!                    eliminate segmenting on curved surfaces, speedups
//! Since version 1.4: Added RCA connector, 3.5mm connector, slide switch,
//!                    surface mount, to-92 markings. Fixed ~5min crash.
//!                    Better LED illumination. Other minor changes.
//!
//! Copyright (C) 2001-2015 Ben Buxton (bb@cactii.net)
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Written over a few days in a (successful) bid to learn GL coding
//! ```
//!
//! Electronic components drift across a green grid and tumble as they go:
//! resistors with their colour codes, banded diodes, transistors and chips
//! with real part numbers printed on them, electrolytics, a glass fuse, an RCA
//! plug, a slide switch, and a seven-segment display counting to itself.
//!
//! One LED at a time gets to be a light source, and lights the rest of the
//! scene in its own colour until it drifts off the edge and another takes over.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};
use std::f64::consts::PI;

const MAX_COMPONENTS: usize = 400;
const MOVE_MULT: f32 = 0.02;

fn f_rand() -> f32 {
    frand(1.0) as f32
}

fn rand_range(min: f32, max: f32) -> f32 {
    min + (max - min) * f_rand()
}

/// The sine, cosine and tangent of a whole number of degrees. Upstream keeps
/// tables of these, since two hundred calls to `sin` in one frame "can be a
/// bit harsh"; that is no longer true, so they are computed.
fn sin_deg(a: i32) -> f32 {
    (f64::from(a) * PI / 180.0).sin() as f32
}

fn cos_deg(a: i32) -> f32 {
    (f64::from(a) * PI / 180.0).cos() as f32
}

fn tan_deg(a: i32) -> f32 {
    (f64::from(a) * PI / 180.0).tan() as f32
}

const TRANSISTOR_TYPES: [&str; 13] = [
    "TIP2955", "TIP32C", "LM 350T", "IRF730", "ULN2577", "7805T", "7912T", "TIP120", "2N6401",
    "BD239", "2SC1590", "MRF485", "SC141D",
];

const TO92_TYPES: [&str; 12] = [
    "C\n548", "C\n848", "74\nL05", "C\n858", "BC\n212L", "BC\n640", "BC\n337", "BC\n338", "S817",
    "78\nL12", "TL\n431", "LM\n35DZ",
];

const SMC_TYPES: [&str; 7] = ["1M-", "1K", "1F", "B10", "S14", "Q3", "4A"];

/// How many pins each chip has, and what is printed on it.
const IC_TYPES: [(usize, &str); 36] = [
    (8, "NE 555"),
    (8, "LM 386N"),
    (8, "ADC0831"),
    (8, "LM 383T"),
    (8, "TL071"),
    (8, "LM 311"),
    (8, "LM393"),
    (8, "LM 3909"),
    (14, "LM 380N"),
    (14, "NE 556"),
    (14, "TL074"),
    (14, "LM324"),
    (14, "LM339"),
    (14, "MC1488"),
    (14, "MC1489"),
    (14, "LM1877-9"),
    (14, "4011"),
    (14, "4017"),
    (14, "4013"),
    (14, "4024"),
    (14, "4066"),
    (16, "4076"),
    (16, "4049"),
    (16, "4094"),
    (16, "4043"),
    (16, "4510"),
    (16, "4511"),
    (16, "4035"),
    (16, "RS232"),
    (16, "MC1800"),
    (16, "ULN2081"),
    (16, "UDN2953"),
    (24, "ISD1416P"),
    (24, "4515"),
    (24, "TMS6264L"),
    (24, "MC146818"),
];

/// The standard resistor colour codes, black through silver.
const COLORCODES: [[f32; 3]; 12] = [
    [0.0, 0.0, 0.0],
    [0.49, 0.25, 0.08],
    [1.0, 0.0, 0.0],
    [1.0, 0.5, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.5, 1.0],
    [0.7, 0.2, 1.0],
    [0.5, 0.5, 0.5],
    [1.0, 1.0, 1.0],
    [0.66, 0.56, 0.2],
    [0.8, 0.8, 0.8],
];

/// Base values for components: the first two bands of a resistor.
const VALUES: [[usize; 2]; 9] = [
    [1, 0],
    [2, 2],
    [3, 3],
    [4, 7],
    [5, 6],
    [6, 8],
    [7, 5],
    [8, 2],
    [9, 1],
];

/// A band round a resistor or a diode: where it sits along the body, how wide
/// it is, and what colour.
#[derive(Clone, Copy)]
struct Band {
    pos: f32,
    len: f32,
    color: [f32; 3],
}

/// The eleven kinds of component, and whatever each one needs to know about
/// itself.
enum Part {
    Resistor {
        b: [usize; 4],
    },
    Diode {
        band: Band,
        color: [f32; 3],
    },
    /// Package zero is a TO-220, one a TO-92, two a surface mount.
    Transistor {
        kind: usize,
        text: &'static str,
    },
    Led {
        color: [f32; 3],
        light: bool,
    },
    /// Electrolytic when false, ceramic when true.
    Capacitor {
        ceramic: bool,
        width: f32,
        length: f32,
    },
    Ic {
        pins: usize,
        text: String,
    },
    Disp {
        value: usize,
    },
    Fuse,
    Rca {
        white: bool,
    },
    ThreeFive,
    Switch,
}

impl Part {
    /// Whether the part is drawn with blending, and so has to come after the
    /// opaque ones.
    fn alpha(&self) -> bool {
        matches!(self, Part::Led { .. } | Part::Fuse)
    }
}

struct Component {
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    rot: [f32; 3],
    /// Degrees per frame, and how far round it has got.
    drot: f32,
    rdeg: f32,
    angle: f32,
    part: Part,
}

struct Circuit {
    xmax: i32,
    ymax: i32,
    win_w: i32,
    win_h: i32,
    /// Whether some LED has claimed the light, and whether it has been set up.
    light: bool,
    lighton: bool,
    viewer: [f32; 3],
    lightpos: [f32; 4],
    components: Vec<Option<Component>>,
    grid_col: [f32; 3],
    grid_col2: [f32; 3],
    rotate_angle: f32,
    font: TexFont,
    /// The bright spot that runs along the grid: where it is, which way it
    /// goes, how fast, and whether it is running at all.
    draw_sx: f32,
    draw_sy: f32,
    draw_sdir: i32,
    draw_s: bool,
    draw_ds: f32,
    maxparts: usize,
    rotatespeed: i32,
    spin: bool,
    seven: bool,
    aspect: f32,
    scale: f32,
}

/* The shapes everything is built out of */

/// `createCylinder`: a tube along the x axis, optionally capped, optionally
/// only the top half of one.
fn create_cylinder(
    g: &mut Gl,
    win: (i32, i32),
    length: f32,
    radius: f32,
    endcaps: bool,
    half: bool,
) {
    let mut nsegs = (radius * win.0.max(win.1) as f32 / 20.0) as i32;
    nsegs = nsegs.max(4);
    if nsegs % 2 != 0 {
        nsegs += 1;
    }
    // Not 360: upstream runs a little past the turn so the seam closes.
    let angle = if half { 180 - 90 / nsegs } else { 374 };
    let step = (angle / nsegs).max(1);

    let (mut z1, mut y1) = (radius, 0.0);
    g.glx.begin(Shape::Quads);
    let mut a = 0;
    while a <= angle {
        let y2 = radius * sin_deg(a);
        let z2 = radius * cos_deg(a);
        g.glx.normal3f(0.0, y1, z1);
        g.glx.vertex3f(0.0, y1, z1);
        g.glx.vertex3f(length, y1, z1);
        g.glx.normal3f(0.0, y2, z2);
        g.glx.vertex3f(length, y2, z2);
        g.glx.vertex3f(0.0, y2, z2);
        z1 = z2;
        y1 = y2;
        a += step;
    }
    g.glx.end();

    if half {
        g.glx.begin(Shape::Polygon);
        g.glx.normal3f(0.0, 1.0, 0.0);
        g.glx.vertex3f(0.0, 0.0, radius);
        g.glx.vertex3f(length, 0.0, radius);
        g.glx.vertex3f(length, 0.0, -radius);
        g.glx.vertex3f(0.0, 0.0, -radius);
        g.glx.end();
    }
    if endcaps {
        for (ex, norm) in [(0.0, -1.0), (length, 1.0)] {
            let (mut z1, mut y1) = (radius, 0.0);
            g.glx.begin(Shape::Triangles);
            g.glx.normal3f(norm, 0.0, 0.0);
            let mut a = 0;
            while a <= angle {
                let y2 = radius * sin_deg(a);
                let z2 = radius * cos_deg(a);
                g.glx.vertex3f(ex, 0.0, 0.0);
                g.glx.vertex3f(ex, y1, z1);
                g.glx.vertex3f(ex, y2, z2);
                z1 = z2;
                y1 = y2;
                a += step;
            }
            g.glx.end();
        }
    }
}

/// A disc in the yz plane, or half of one.
fn circle(g: &mut Gl, radius: f32, half: bool) {
    let (s, t) = if half { (90, 270) } else { (0, 360) };
    let (mut x1, mut y1) = if half { (radius, 0.0) } else { (0.0, 0.0) };
    g.glx.begin(Shape::Triangles);
    g.glx.normal3f(1.0, 0.0, 0.0);
    let mut i = s;
    while i <= t {
        let x2 = radius * cos_deg(i);
        let y2 = radius * sin_deg(i);
        g.glx.vertex3f(0.0, 0.0, 0.0);
        g.glx.vertex3f(0.0, y1, x1);
        g.glx.vertex3f(0.0, y2, x2);
        x1 = x2;
        y1 = y2;
        i += 10;
    }
    g.glx.end();
}

/// A component's lead: a thin shiny cylinder.
fn wire(g: &mut Gl, win: (i32, i32), len: f32) {
    g.glx.material_ambient_diffuse([0.3, 0.3, 0.3, 1.0]);
    g.glx.material_specular([0.9, 0.9, 0.9, 1.0]);
    g.glx.material_shininess(30.0);
    create_cylinder(g, win, len, 0.05, true, false);
    g.glx.material_specular([0.4, 0.4, 0.4, 1.0]);
}

/// A band of a sphere, from `startstack` to `endstack` and `startslice` to
/// `endslice` of the way round.
fn sphere(g: &mut Gl, r: f32, stacks: f32, slices: f32, stack: (i32, i32), slice: (i32, i32)) {
    let step = (180.0 / stacks) as i32;
    let sstep = (360.0 / slices) as i32;
    let mut a1 = (stack.0 as f32 * (180.0 / stacks)) as i32;
    let b1 = (slice.0 as f32 * (360.0 / slices)) as i32;
    let (mut y1, mut z1, mut yy1, mut zz1) = (0.0, 0.0, 0.0, 0.0);
    let c0 = ((slice.1 as f32 / slices) * 360.0) as i32;
    let c1 = ((stack.1 as f32 / stacks) * 180.0) as i32;

    g.glx.begin(Shape::Quads);
    let mut a = a1;
    while a <= c1 {
        let (d, d1) = (sin_deg(a), sin_deg(a1));
        let (dd, dd1) = (cos_deg(a), cos_deg(a1));
        let (dr, dr1) = (d * r, d1 * r);
        let (big_dr, big_dr1) = (dd * r, dd1 * r);
        let mut b = b1;
        while b <= c0 {
            let y2 = dr * sin_deg(b);
            let z2 = dr * cos_deg(b);
            let yy2 = dr1 * sin_deg(b);
            let zz2 = dr1 * cos_deg(b);
            g.glx.normal3f(big_dr, y1, z1);
            g.glx.vertex3f(big_dr, y1, z1);
            g.glx.normal3f(big_dr, y2, z2);
            g.glx.vertex3f(big_dr, y2, z2);
            g.glx.normal3f(big_dr1, yy2, zz2);
            g.glx.vertex3f(big_dr1, yy2, zz2);
            g.glx.normal3f(big_dr1, yy1, zz1);
            g.glx.vertex3f(big_dr1, yy1, zz1);
            z1 = z2;
            y1 = y2;
            zz1 = zz2;
            yy1 = yy2;
            b += sstep;
        }
        a1 = a;
        a += step;
    }
    g.glx.end();
}

/// A box with its near-top corner at `(x, y, z)`, `t` deep towards the viewer.
fn rect(g: &mut Gl, x: f32, y: f32, z: f32, w: f32, h: f32, t: f32) {
    let (yh, xw, zt) = (y + h, x + w, z - t);
    g.glx.begin(Shape::Quads);
    for (n, quad) in [
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
    ] {
        g.glx.normal3f(n[0], n[1], n[2]);
        for v in quad {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

/// One leg of a chip, bent down and out.
fn ic_leg(g: &mut Gl, x: f32, y: f32, z: f32, left: bool) {
    if left {
        rect(g, x - 0.1, y, z, 0.1, 0.1, 0.02);
        rect(g, x - 0.1, y, z, 0.02, 0.1, 0.1);
        rect(g, x - 0.1, y + 0.03, z - 0.1, 0.02, 0.05, 0.3);
    } else {
        rect(g, x, y, z, 0.1, 0.1, 0.02);
        rect(g, x + 0.8 * 0.1, y, z, 0.02, 0.1, 0.1);
        rect(g, x + 0.8 * 0.1, y + 0.03, z - 0.1, 0.02, 0.05, 0.3);
    }
}

/// A slab with a round hole through it, which is a switch's mounting lug and
/// a TO-220's tab.
fn holed_rectangle(g: &mut Gl, w: f32, h: f32, d: f32, radius: f32, p: i32) {
    let step = 360 / p;
    let (mut x1, mut y1) = (radius, 0.0);
    let (mut xr1, mut yr1) = (w / 2.0, 0.0);
    let (side, side1) = (w / 2.0, h / 2.0);

    g.glx.begin(Shape::Quads);
    let mut a = 0;
    while a <= 360 {
        let y2 = radius * sin_deg(a);
        let x2 = radius * cos_deg(a);

        let (xr, yr, nx, ny);
        if !(45..=315).contains(&a) {
            xr = side;
            yr = side1 * tan_deg(a);
            nx = 1.0;
            ny = 0.0;
        } else if a <= 135 || a >= 225 {
            if a >= 225 {
                yr = -side1;
                xr = -(side / tan_deg(a));
                nx = 0.0;
                ny = -1.0;
            } else {
                xr = side / tan_deg(a);
                yr = side1;
                nx = 0.0;
                ny = 1.0;
            }
        } else {
            xr = -side;
            yr = -side1 * tan_deg(a);
            nx = -1.0;
            ny = 0.0;
        }

        // The wall of the hole.
        g.glx.normal3f(-x1, -y1, 0.0);
        g.glx.vertex3f(x1, y1, 0.0);
        g.glx.vertex3f(x1, y1, -d);
        g.glx.vertex3f(x2, y2, -d);
        g.glx.vertex3f(x2, y2, 0.0);
        // The front face.
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.vertex3f(x1, y1, 0.0);
        g.glx.vertex3f(xr1, yr1, 0.0);
        g.glx.vertex3f(xr, yr, 0.0);
        g.glx.vertex3f(x2, y2, 0.0);
        // The outside.
        g.glx.normal3f(nx, ny, 0.0);
        g.glx.vertex3f(xr, yr, 0.0);
        g.glx.vertex3f(xr, yr, -d);
        g.glx.vertex3f(xr1, yr1, -d);
        g.glx.vertex3f(xr1, yr1, 0.0);
        // And the back.
        g.glx.normal3f(0.0, 0.0, -1.0);
        g.glx.vertex3f(xr, yr, -d);
        g.glx.vertex3f(x2, y2, -d);
        g.glx.vertex3f(x1, y1, -d);
        g.glx.vertex3f(xr1, yr1, -d);

        x1 = x2;
        y1 = y2;
        xr1 = xr;
        yr1 = yr;
        a += step;
    }
    g.glx.end();
}

/// The seven segments, as the offsets of a horizontal and a vertical bar.
const VDATA_H: [[f32; 2]; 6] = [
    [0.0, 0.0],
    [0.1, 0.1],
    [0.9, 0.1],
    [1.0, 0.0],
    [0.9, -0.1],
    [0.1, -0.1],
];
const VDATA_V: [[f32; 2]; 6] = [
    [0.27, 0.0],
    [0.35, -0.1],
    [0.2, -0.9],
    [0.1, -1.0],
    [0.0, -0.9],
    [0.15, -0.15],
];
const SEG_START: [[f32; 2]; 7] = [
    [0.55, 2.26],
    [1.35, 2.26],
    [1.2, 1.27],
    [0.25, 0.25],
    [0.06, 1.25],
    [0.25, 2.25],
    [0.39, 1.24],
];
const NUMS: [[bool; 7]; 10] = [
    [true, true, true, true, true, true, false],
    [false, true, true, false, false, false, false],
    [true, true, false, true, true, false, true],
    [true, true, true, true, false, false, true],
    [false, true, true, false, false, true, true],
    [true, false, true, true, false, true, true],
    [true, false, true, true, true, true, true],
    [true, true, true, false, false, false, false],
    [true, true, true, true, true, true, true],
    [true, true, true, false, false, true, true],
];

impl Circuit {
    fn win(&self) -> (i32, i32) {
        (self.win_w, self.win_h)
    }

    /// The text of a component, in the grey upstream prints it in. Upstream's
    /// `print_texture_string` turns the lights off for the duration.
    fn print(&self, g: &mut Gl, s: &str) {
        g.glx.lighting(false);
        g.glx.blend(Blend::Alpha);
        g.glx.color4f(0.7, 0.7, 0.7, 1.0);
        self.font.print_string(&mut g.glx, s);
        g.glx.blend(Blend::Off);
        g.glx.lighting(true);
    }

    fn draw_resistor(&self, g: &mut Gl, b: [usize; 4]) {
        g.glx.translate(-4.0, 0.0, 0.0);
        wire(g, self.win(), 3.0);
        g.glx.translate(3.0, 0.0, 0.0);
        g.glx.material_ambient_diffuse([0.74, 0.62, 0.46, 1.0]);
        g.glx.material_specular([0.8, 0.8, 0.8, 1.0]);
        g.glx.material_shininess(30.0);
        create_cylinder(g, self.win(), 1.8, 0.4, true, false);
        // `makebandlist`: upstream compiles the twelve colour-code bands into
        // display lists, with the material set inside each. A list here
        // replays geometry and not state, and this saver never turns on
        // `GL_COLOR_MATERIAL`, so a colour on the vertices would do nothing:
        // the bands are drawn where they are wanted instead.
        g.glx.push_matrix();
        for band in b {
            g.glx.translate(0.35, 0.0, 0.0);
            let c = COLORCODES[band];
            g.glx.material_ambient_diffuse([c[0], c[1], c[2], 0.0]);
            g.glx.material_specular([0.8, 0.8, 0.8, 0.0]);
            g.glx.material_shininess(40.0);
            create_cylinder(g, self.win(), 0.1, 0.42, false, false);
        }
        g.glx.pop_matrix();
        g.glx.translate(1.8, 0.0, 0.0);
        wire(g, self.win(), 3.0);
    }

    fn draw_rca(&self, g: &mut Gl, white: bool) {
        g.glx.push_matrix();
        g.glx.translate(0.3, 0.0, 0.0);
        g.glx.material_ambient_diffuse([0.6, 0.6, 0.6, 1.0]);
        g.glx.material_shininess(40.0);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        create_cylinder(g, self.win(), 0.7, 0.45, false, false);
        g.glx.translate(0.4, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.9, 0.15, true, false);
        g.glx.translate(-1.9, 0.0, 0.0);
        g.glx.material_shininess(20.0);
        g.glx.material_ambient_diffuse(if white {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        });
        create_cylinder(g, self.win(), 1.5, 0.6, true, false);
        g.glx.translate(-0.9, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.9, 0.25, false, false);
        g.glx.translate(0.1, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.2, 0.3, false, false);
        g.glx.translate(0.3, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.2, 0.3, true, false);
        g.glx.translate(0.3, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.2, 0.3, true, false);
        g.glx.pop_matrix();
    }

    fn draw_switch(&self, g: &mut Gl) {
        let dark = [0.1, 0.1, 0.1, 1.0];
        let spec = [0.9, 0.9, 0.9, 1.0];
        g.glx.push_matrix();
        g.glx.material_diffuse([0.6, 0.6, 0.6, 0.0]);
        g.glx.material_ambient(dark);
        g.glx.material_specular(spec);
        g.glx.material_shininess(90.0);
        rect(g, -0.25, 0.0, 0.0, 1.5, 0.5, 0.75);
        g.glx.push_matrix();
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.translate(-0.5, -0.4, -0.4);
        holed_rectangle(g, 0.5, 0.75, 0.1, 0.15, 8);
        g.glx.translate(2.0, 0.0, 0.0);
        holed_rectangle(g, 0.5, 0.75, 0.1, 0.15, 8);
        g.glx.pop_matrix();
        for z in [-0.25, -0.5] {
            for x in [0.1, 0.5, 0.9] {
                rect(g, x, -0.4, z, 0.1, 0.4, 0.05);
            }
        }
        g.glx.material_ambient_diffuse(dark);
        g.glx.material_specular(spec);
        rect(g, 0.0, 0.5, -0.1, 1.0, 0.05, 0.5);
        rect(g, 0.0, 0.6, -0.1, 0.5, 0.6, 0.5);
        g.glx.material_ambient_diffuse([0.69, 0.32, 0.0, 1.0]);
        rect(g, -0.2, -0.01, -0.1, 1.4, 0.1, 0.55);
        g.glx.pop_matrix();
    }

    fn draw_fuse(&self, g: &mut Gl) {
        let col = [0.5, 0.5, 0.5, 1.0];
        g.glx.push_matrix();
        g.glx.translate(-1.8, 0.0, 0.0);
        g.glx.material_ambient_diffuse(col);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(40.0);
        create_cylinder(g, self.win(), 0.8, 0.45, true, false);
        g.glx.translate(0.8, 0.0, 0.0);
        g.glx.blend(Blend::Alpha);
        g.glx.depth_mask(false);
        g.glx.material_ambient_diffuse([0.4, 0.4, 0.4, 0.3]);
        g.glx.material_shininess(40.0);
        create_cylinder(g, self.win(), 2.0, 0.4, false, false);
        create_cylinder(g, self.win(), 2.0, 0.3, false, false);
        g.glx.blend(Blend::Off);
        g.glx.depth_mask(true);
        g.glx.material_ambient_diffuse(col);
        g.glx.material_shininess(40.0);
        // The wire down the middle of it.
        g.glx.begin(Shape::Lines);
        g.glx.vertex3f(0.0, 0.0, 0.0);
        g.glx.vertex3f(2.0, 0.0, 0.0);
        g.glx.end();
        g.glx.translate(2.0, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.8, 0.45, true, false);
        g.glx.pop_matrix();
    }

    fn draw_capacitor(&self, g: &mut Gl, ceramic: bool, width: f32, length: f32) {
        g.glx.push_matrix();
        if ceramic {
            g.glx.material_ambient_diffuse([0.84, 0.5, 0.0, 1.0]);
            sphere(g, width, 15.0, 15.0, (0, 4), (0, 15));
            g.glx.translate(1.35 * width, 0.0, 0.0);
            sphere(g, width, 15.0, 15.0, (11, 15), (0, 15));
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            g.glx.translate(0.0, 0.7 * width, 0.3 * width);
            wire(g, self.win(), 3.0 * width);
            g.glx.translate(0.0, 0.0, -0.6 * width);
            wire(g, self.win(), 3.0 * width);
        } else {
            g.glx.translate(-length * 2.0, 0.0, 0.0);
            g.glx.material_ambient_diffuse([0.0, 0.0, 0.0, 0.0]);
            g.glx.material_specular([0.8, 0.8, 0.8, 0.0]);
            g.glx.material_shininess(40.0);
            // The stripe up the side.
            g.glx.begin(Shape::Polygon);
            g.glx.vertex3f(0.0, 0.82 * width, -0.1);
            g.glx.vertex3f(3.0 * length, 0.82 * width, -0.1);
            g.glx.vertex3f(3.0 * length, 0.82 * width, 0.1);
            g.glx.vertex3f(0.0, 0.82 * width, 0.1);
            g.glx.end();
            g.glx.material_ambient_diffuse([0.0, 0.2, 0.9, 0.0]);
            g.glx.polygon_offset(Some((1.0, 1.0)));
            create_cylinder(g, self.win(), 3.0 * length, 0.8 * width, true, false);
            g.glx.polygon_offset(None);
            g.glx.material_ambient_diffuse([0.7, 0.7, 0.7, 0.0]);
            circle(g, 0.6 * width, false);
            g.glx.material_ambient_diffuse([0.0, 0.0, 0.0, 0.0]);
            g.glx.translate(3.0 * length, 0.0, 0.0);
            circle(g, 0.6 * width, false);
            g.glx.translate(0.0, 0.4 * width, 0.0);
            wire(g, self.win(), 3.0 * length);
            g.glx.translate(0.0, -0.8 * width, 0.0);
            wire(g, self.win(), 3.3 * length);
        }
        g.glx.pop_matrix();
    }

    fn draw_led(&mut self, g: &mut Gl, color: [f32; 3], lit: bool) {
        let col = [color[0], color[1], color[2], 0.6];
        if lit && self.light && !self.lighton {
            // No cone: the runtime has no spotlight, so the LED lights the
            // scene from where it is rather than in the beam upstream aims
            // along its own axis.
            g.glx.light_specular(1, col);
            g.glx.light_ambient(1, [0.0, 0.0, 0.0, 0.6]);
            g.glx
                .light_diffuse(1, [col[0] / 1.5, col[1] / 1.5, col[2] / 1.5, col[3]]);
            self.lighton = true;
        }
        g.glx.material_ambient_diffuse(col);
        g.glx.material_specular(col);
        if !lit {
            // Upstream blends this one the other way round, source by one
            // minus alpha; the same picture comes of ordinary blending with
            // the alpha complemented.
            let c = [col[0], col[1], col[2], 1.0 - col[3]];
            g.glx.material_ambient_diffuse(c);
            g.glx.material_specular(c);
            g.glx.blend(Blend::Alpha);
            g.glx.depth_mask(false);
        }
        g.glx.translate(-0.9, 0.0, 0.0);
        create_cylinder(g, self.win(), 1.2, 0.3, false, false);
        if lit && self.light {
            g.glx.lighting(false);
            g.glx.color4f(col[0], col[1], col[2], 1.0);
        }
        sphere(g, 0.3, 7.0, 7.0, (3, 7), (0, 7));
        if lit && self.light {
            g.glx.lighting(true);
        } else {
            g.glx.depth_mask(true);
            g.glx.blend(Blend::Off);
        }

        g.glx.translate(1.2, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.1, 0.38, true, false);
        g.glx.translate(-0.3, 0.15, 0.0);
        wire(g, self.win(), 3.0);
        g.glx.translate(0.0, -0.3, 0.0);
        wire(g, self.win(), 3.3);
    }

    fn draw_three_five(&self, g: &mut Gl) {
        let light = [0.6, 0.6, 0.6, 0.0];
        g.glx.push_matrix();
        g.glx.material_shininess(40.0);
        g.glx.material_ambient_diffuse([0.8, 0.8, 0.6, 0.0]);
        g.glx.material_specular([0.7, 0.7, 0.7, 0.0]);
        g.glx.translate(-2.0, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.7, 0.2, false, false);
        g.glx.translate(0.7, 0.0, 0.0);
        create_cylinder(g, self.win(), 1.3, 0.4, true, false);
        g.glx.material_ambient_diffuse(light);
        g.glx.translate(1.3, 0.0, 0.0);
        create_cylinder(g, self.win(), 1.3, 0.2, false, false);
        g.glx.material_ambient_diffuse([0.3, 0.3, 0.3, 0.0]);
        g.glx.translate(0.65, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.15, 0.21, false, false);
        g.glx.translate(0.3, 0.0, 0.0);
        create_cylinder(g, self.win(), 0.15, 0.21, false, false);
        g.glx.material_ambient_diffuse(light);
        g.glx.translate(0.4, 0.0, 0.0);
        sphere(g, 0.23, 7.0, 7.0, (0, 5), (0, 7));
        g.glx.pop_matrix();
    }

    fn draw_diode(&self, g: &mut Gl, band: Band, color: [f32; 3]) {
        g.glx.push_matrix();
        g.glx.material_shininess(40.0);
        g.glx.material_ambient_diffuse([0.3, 0.3, 0.3, 0.0]);
        g.glx.material_specular([0.7, 0.7, 0.7, 0.0]);
        g.glx.translate(-4.0, 0.0, 0.0);
        wire(g, self.win(), 3.0);
        g.glx.translate(3.0, 0.0, 0.0);
        // `bandedCylinder`: the body, then one band round it.
        g.glx
            .material_ambient_diffuse([color[0], color[1], color[2], 0.0]);
        create_cylinder(g, self.win(), 1.5, 0.3, true, false);
        g.glx.push_matrix();
        g.glx.translate(band.pos * 1.5, 0.0, 0.0);
        g.glx
            .material_ambient_diffuse([band.color[0], band.color[1], band.color[2], 0.0]);
        create_cylinder(g, self.win(), band.len * 1.5, 0.3 * 1.05, false, false);
        g.glx.pop_matrix();
        g.glx.translate(1.5, 0.0, 0.0);
        wire(g, self.win(), 3.0);
        g.glx.pop_matrix();
    }

    fn draw_ic(&self, g: &mut Gl, pins: usize, text: &str) {
        g.glx.push_matrix();
        g.glx.material_ambient_diffuse([0.1, 0.1, 0.1, 0.0]);
        g.glx.material_specular([0.6, 0.6, 0.6, 0.0]);
        g.glx.material_shininess(40.0);
        let (w, h) = match pins {
            8 => (1.0, 1.5),
            14 | 16 => (1.0, 3.0),
            _ => (1.5, 3.5),
        };
        let (w, h) = (w / 2.0, h / 2.0);

        g.glx.polygon_offset(Some((1.0, 1.0)));
        g.glx.begin(Shape::Quads);
        for (n, quad) in [
            (
                [0.0, 0.0, 1.0],
                [[w, h, 0.1], [w, -h, 0.1], [-w, -h, 0.1], [-w, h, 0.1]],
            ),
            (
                [0.0, 0.0, -1.0],
                [[w, h, -0.1], [w, -h, -0.1], [-w, -h, -0.1], [-w, h, -0.1]],
            ),
            (
                [1.0, 0.0, 0.0],
                [[w, h, -0.1], [w, -h, -0.1], [w, -h, 0.1], [w, h, 0.1]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[w, -h, -0.1], [w, -h, 0.1], [-w, -h, 0.1], [-w, -h, -0.1]],
            ),
            (
                [-1.0, 0.0, 0.0],
                [[-w, h, -0.1], [-w, h, 0.1], [-w, -h, 0.1], [-w, -h, -0.1]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[-w, h, -0.1], [w, h, -0.1], [w, h, 0.1], [-w, h, 0.1]],
            ),
        ] {
            g.glx.normal3f(n[0], n[1], n[2]);
            for v in quad {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
        }
        g.glx.end();
        g.glx.polygon_offset(None);

        // The part number, along the length of the package. Upstream takes the
        // string's metrics here and then centres on half the package width
        // instead, which is what this does.
        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, 0.1);
        g.glx.rotate(90.0, 0.0, 0.0, 1.0);
        g.glx.scale(0.015, 0.015, 0.015);
        g.glx.translate(-w / 2.0, 0.0, 0.0);
        self.print(g, text);
        g.glx.pop_matrix();

        let d = ((h * 2.0 - 0.1) / pins as f32) * 2.0;
        g.glx.material_ambient_diffuse([0.4, 0.4, 0.4, 0.0]);
        g.glx.material_specular([0.6, 0.6, 0.6, 0.0]);
        g.glx.material_shininess(40.0);
        for z in 0..pins / 2 {
            ic_leg(g, w, -h + z as f32 * d + d / 2.0, 0.0, false);
        }
        for z in 0..pins / 2 {
            ic_leg(g, -w, -h + z as f32 * d + d / 2.0, 0.0, true);
        }
        // The dimple that marks pin one.
        g.glx.material_ambient_diffuse([0.2, 0.2, 0.2, 0.0]);
        g.glx.translate(-w + 0.3, h - 0.3, 0.1);
        g.glx.rotate(90.0, 0.0, 1.0, 0.0);
        circle(g, 0.1, false);
        g.glx.pop_matrix();
    }

    fn draw_disp(&self, g: &mut Gl, value: usize) {
        let on = [0.9, 0.0, 0.0];
        let off = [0.3, 0.0, 0.0];
        g.glx.translate(-0.9, -1.8, 0.0);
        g.glx.material_ambient_diffuse([0.8, 0.8, 0.8, 1.0]);
        rect(g, 0.0, 0.0, -0.01, 1.8, 2.6, 0.7);
        g.glx.material_ambient_diffuse([0.2, 0.2, 0.2, 1.0]);
        g.glx.begin(Shape::Quads);
        g.glx.vertex3f(-0.05, -0.05, 0.0);
        g.glx.vertex3f(-0.05, 2.65, 0.0);
        g.glx.vertex3f(1.85, 2.65, 0.0);
        g.glx.vertex3f(1.85, -0.05, 0.0);
        g.glx.end();

        // The lit segments need no light of their own.
        g.glx.lighting(false);
        for j in 0..7 {
            let c = if NUMS[value][j] { on } else { off };
            g.glx.color4f(c[0], c[1], c[2], 1.0);
            g.glx.begin(Shape::Polygon);
            for k in 0..6 {
                let v = if j == 0 || j == 3 || j == 6 {
                    VDATA_H[k]
                } else {
                    VDATA_V[k]
                };
                g.glx
                    .vertex3f(SEG_START[j][0] + v[0], SEG_START[j][1] + v[1], 0.01);
            }
            g.glx.end();
        }
        // And the decimal point.
        g.glx.color4f(on[0], on[1], on[2], 1.0);
        g.glx.point_size(4.0);
        g.glx.begin(Shape::Points);
        g.glx.vertex3f(1.5, 0.2, 0.01);
        g.glx.end();
        g.glx.lighting(true);

        g.glx.material_ambient_diffuse([0.4, 0.4, 0.4, 0.0]);
        g.glx.material_specular([0.6, 0.6, 0.6, 0.0]);
        g.glx.material_shininess(40.0);
        let mut x = 0.35;
        while x <= 1.5 {
            let mut y = 0.2;
            while y <= 2.4 {
                ic_leg(g, x, y, -0.7, true);
                y += 0.3;
            }
            x += 1.15;
        }
    }

    fn draw_transistor(&self, g: &mut Gl, kind: usize, text: &str) {
        let col = [0.3, 0.3, 0.3, 1.0];
        g.glx.push_matrix();
        g.glx.material_shininess(30.0);
        g.glx.material_diffuse(col);
        if kind == 1 {
            // TO-92: a half-round body with the type printed on the flat.
            g.glx.material_specular(col);
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            create_cylinder(g, self.win(), 1.0, 0.4, true, true);
            rect(g, 0.0, -0.2, 0.4, 1.0, 0.2, 0.8);
            let w = self.font.metrics(text).width as f32;
            g.glx.push_matrix();
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
            g.glx.translate(0.5, -0.05, 0.21);
            g.glx.scale(0.015, 0.015, 0.015);
            g.glx.translate(-w / 2.0, 0.0, 0.0);
            self.print(g, text);
            g.glx.pop_matrix();
            g.glx.translate(-2.0, 0.0, -0.2);
            for _ in 0..3 {
                wire(g, self.win(), 2.0);
                g.glx.translate(0.0, 0.0, 0.2);
            }
        } else if kind == 0 {
            // TO-220: a slab with a tab and a hole in it.
            rect(g, 0.0, 0.0, 0.0, 1.5, 1.5, 0.5);
            let w = self.font.metrics(text).width as f32;
            g.glx.push_matrix();
            g.glx.translate(0.75, 0.75, 0.01);
            g.glx.scale(0.015, 0.015, 0.015);
            g.glx.translate(-w / 2.0, 0.0, 0.0);
            self.print(g, text);
            g.glx.pop_matrix();
            g.glx.material_ambient_diffuse(col);
            g.glx.material_specular([0.9, 0.9, 0.9, 1.0]);
            g.glx.material_shininess(30.0);
            rect(g, 0.0, 0.0, -0.5, 1.5, 1.5, 0.30);
            g.glx.translate(0.75, 1.875, -0.55);
            holed_rectangle(g, 1.5, 0.75, 0.25, 0.2, 8);
            g.glx.material_specular([0.4, 0.4, 0.4, 1.0]);
            g.glx.translate(-0.375, -1.875, 0.0);
            g.glx.rotate(90.0, 0.0, 0.0, -1.0);
            for _ in 0..3 {
                wire(g, self.win(), 2.0);
                g.glx.translate(0.0, 0.375, 0.0);
            }
        } else {
            // Surface mount: a chip of plastic with three tabs.
            g.glx.material_specular(col);
            g.glx.translate(-0.5, -0.25, 0.1);
            rect(g, 0.0, 0.0, 0.0, 1.0, 0.5, 0.2);
            // Upstream draws whatever texture happens to be bound over the
            // face here, which is the font atlas, and never uses the part
            // number it picked. The number is printed instead.
            let w = self.font.metrics(text).width as f32;
            g.glx.push_matrix();
            g.glx.translate(0.5, 0.15, 0.01);
            g.glx.scale(0.008, 0.008, 0.008);
            g.glx.translate(-w / 2.0, 0.0, 0.0);
            self.print(g, text);
            g.glx.pop_matrix();
            g.glx.material_ambient_diffuse(col);
            g.glx.material_specular([0.9, 0.9, 0.9, 1.0]);
            g.glx.material_shininess(30.0);
            rect(g, 0.25, -0.1, -0.05, 0.1, 0.1, 0.2);
            rect(g, 0.75, -0.1, -0.05, 0.1, 0.1, 0.2);
            rect(g, 0.5, 0.5, -0.05, 0.1, 0.1, 0.2);
            rect(g, 0.25, -0.2, -0.2, 0.1, 0.15, 0.1);
            rect(g, 0.75, -0.2, -0.2, 0.1, 0.15, 0.1);
            rect(g, 0.5, 0.5, -0.2, 0.1, 0.15, 0.1);
        }
        g.glx.pop_matrix();
    }

    /// `drawgrid`: the green graph paper everything floats over, and the
    /// bright spot that now and then runs across it.
    fn drawgrid(&mut self, g: &mut Gl) {
        let (xmax, ymax) = (self.xmax as f32, self.ymax as f32);
        if !self.draw_s {
            if f_rand() < if self.rotatespeed > 0 { 0.05 } else { 0.01 } {
                self.draw_sdir = rand_range(0.0, 4.0) as i32;
                self.draw_ds = rand_range(0.4, 0.8);
                match self.draw_sdir {
                    0 => {
                        self.draw_sx = -xmax / 2.0;
                        self.draw_sy = (rand_range(0.0, ymax / 2.0) as i32 * 2) as f32 - ymax / 2.0;
                    }
                    1 => {
                        self.draw_sx = xmax / 2.0;
                        self.draw_sy = (rand_range(0.0, ymax / 2.0) as i32 * 2) as f32 - ymax / 2.0;
                    }
                    2 => {
                        self.draw_sy = ymax / 2.0;
                        self.draw_sx = (rand_range(0.0, xmax / 2.0) as i32 * 2) as f32 - xmax / 2.0;
                    }
                    _ => {
                        self.draw_sy = -ymax / 2.0;
                        self.draw_sx = (rand_range(0.0, xmax / 2.0) as i32 * 2) as f32 - xmax / 2.0;
                    }
                }
                self.draw_s = true;
            }
        } else if self.rotatespeed <= 0 && self.grid_col[1] < 0.25 {
            self.grid_col[1] += 0.025;
            self.grid_col[2] += 0.005;
            self.grid_col2[1] += 0.015;
            self.grid_col2[2] += 0.005;
        }

        g.glx.lighting(false);
        if self.draw_s {
            g.glx.color4f(0.0, 0.8, 0.0, 1.0);
            g.glx.push_matrix();
            g.glx.translate(self.draw_sx, self.draw_sy, -10.0);
            sphere(g, 0.1, 10.0, 10.0, (0, 10), (0, 10));
            match self.draw_sdir {
                0 => g.glx.translate(-self.draw_ds, 0.0, 0.0),
                1 => g.glx.translate(self.draw_ds, 0.0, 0.0),
                2 => g.glx.translate(0.0, self.draw_ds, 0.0),
                _ => g.glx.translate(0.0, -self.draw_ds, 0.0),
            }
            sphere(g, 0.05, 10.0, 10.0, (0, 10), (0, 10));
            g.glx.pop_matrix();
            match self.draw_sdir {
                0 => {
                    self.draw_sx += self.draw_ds;
                    if self.draw_sx > xmax / 2.0 {
                        self.draw_s = false;
                    }
                }
                1 => {
                    self.draw_sx -= self.draw_ds;
                    if self.draw_sx < -xmax / 2.0 {
                        self.draw_s = false;
                    }
                }
                2 => {
                    self.draw_sy -= self.draw_ds;
                    if self.draw_sy < ymax / 2.0 {
                        self.draw_s = false;
                    }
                }
                _ => {
                    self.draw_sy += self.draw_ds;
                    if self.draw_sy > ymax / 2.0 {
                        self.draw_s = false;
                    }
                }
            }
        } else if self.rotatespeed <= 0 && self.grid_col[1] > 0.0 {
            self.grid_col[1] -= 0.0025;
            self.grid_col[2] -= 0.0005;
            self.grid_col2[1] -= 0.0015;
            self.grid_col2[2] -= 0.0005;
        }

        // Each line is drawn three times: one bright and two dim beside it,
        // which is what gives the grid its glow.
        let (c1, c2) = (self.grid_col, self.grid_col2);
        let mut x = -xmax / 2.0;
        while x <= xmax / 2.0 {
            g.glx.begin(Shape::Lines);
            g.glx.color4f(c1[0], c1[1], c1[2], 1.0);
            g.glx.vertex3f(x, ymax / 2.0, -10.0);
            g.glx.vertex3f(x, -ymax / 2.0, -10.0);
            g.glx.color4f(c2[0], c2[1], c2[2], 1.0);
            for d in [-0.02, 0.02] {
                g.glx.vertex3f(x + d, ymax / 2.0, -10.0);
                g.glx.vertex3f(x + d, -ymax / 2.0, -10.0);
            }
            g.glx.end();
            x += 2.0;
        }
        let mut y = -ymax / 2.0;
        while y <= ymax / 2.0 {
            g.glx.begin(Shape::Lines);
            g.glx.color4f(c1[0], c1[1], c1[2], 1.0);
            g.glx.vertex3f(-xmax / 2.0, y, -10.0);
            g.glx.vertex3f(xmax / 2.0, y, -10.0);
            g.glx.color4f(c2[0], c2[1], c2[2], 1.0);
            for d in [-0.02, 0.02] {
                g.glx.vertex3f(-xmax / 2.0, y + d, -10.0);
                g.glx.vertex3f(xmax / 2.0, y + d, -10.0);
            }
            g.glx.end();
            y += 2.0;
        }
        g.glx.lighting(true);
    }

    /// `DrawComponent`: put one component where it is, draw it, and move it
    /// on. True if it has left the screen and should go.
    fn draw_component(&mut self, g: &mut Gl, i: usize) -> bool {
        let Some(c) = self.components[i].take() else {
            return false;
        };
        let mut c = c;

        g.glx.push_matrix();
        g.glx.translate(c.x, c.y, c.z);
        if c.angle > 0.0 {
            g.glx.rotate(c.angle, c.rot[0], c.rot[1], c.rot[2]);
        }
        if self.spin {
            g.glx.rotate(c.rdeg, c.rot[0], c.rot[1], c.rot[2]);
            c.rdeg += c.drot;
        }

        match &c.part {
            Part::Resistor { b } => self.draw_resistor(g, *b),
            Part::Diode { band, color } => self.draw_diode(g, *band, *color),
            Part::Transistor { kind, text } => self.draw_transistor(g, *kind, text),
            Part::Led { color, light } => {
                if *light && self.light {
                    g.glx.light_enable(1, true);
                    g.glx.light_position(1, 0.1, 0.0, 0.0, 1.0);
                }
                let (color, light) = (*color, *light);
                self.draw_led(g, color, light);
            }
            Part::Capacitor {
                ceramic,
                width,
                length,
            } => self.draw_capacitor(g, *ceramic, *width, *length),
            Part::Ic { pins, text } => self.draw_ic(g, *pins, text),
            Part::Disp { value } => self.draw_disp(g, *value),
            Part::Fuse => self.draw_fuse(g),
            Part::Rca { white } => self.draw_rca(g, *white),
            Part::ThreeFive => self.draw_three_five(g),
            Part::Switch => self.draw_switch(g),
        }

        // An LED flickers on and off, and takes the scene's light with it.
        if let Part::Led { light, .. } = &mut c.part
            && random() % 50 == 25
        {
            if *light {
                *light = false;
                self.light = false;
                self.lighton = false;
                g.glx.light_enable(1, false);
            } else if !self.light {
                *light = true;
                self.light = true;
            }
        }
        if let Part::Disp { value } = &mut c.part
            && !self.seven
            && random() % 30 == 19
        {
            *value = (random() % 10) as usize;
        }

        c.x += c.dx * MOVE_MULT;
        c.y += c.dy * MOVE_MULT;
        let gone = c.x > (self.xmax / 2) as f32
            || c.x < -(self.xmax / 2) as f32
            || c.y > (self.ymax / 2) as f32
            || c.y < -(self.ymax / 2) as f32;
        if gone
            && let Part::Led { light: true, .. } = c.part
            && self.light
        {
            g.glx.light_enable(1, false);
            self.light = false;
            self.lighton = false;
        }
        g.glx.pop_matrix();
        if !gone {
            self.components[i] = Some(c);
        }
        gone
    }

    /// `NewComponent`: one more part, coming in from an edge.
    fn new_component(&mut self) -> Component {
        let angle = rand_range(0.0, 360.0);
        let rnd = f_rand();
        let (x, y, dx, dy);
        let (xmax, ymax) = (self.xmax as f32, self.ymax as f32);
        if rnd < 0.25 {
            y = ymax / 2.0;
            x = rand_range(0.0, xmax) - xmax / 2.0;
            dx = if x > 0.0 {
                -rand_range(0.5, 2.0)
            } else {
                rand_range(0.5, 2.0)
            };
            dy = -rand_range(0.5, 2.0);
        } else if rnd < 0.5 {
            y = -ymax / 2.0;
            x = rand_range(0.0, xmax) - xmax / 2.0;
            dx = if x > 0.0 {
                -rand_range(0.5, 2.0)
            } else {
                rand_range(0.5, 2.0)
            };
            dy = rand_range(0.5, 2.0);
        } else if rnd < 0.75 {
            x = -xmax / 2.0;
            y = rand_range(0.0, ymax) - ymax / 2.0;
            dx = rand_range(0.5, 2.0);
            dy = if y > 0.0 {
                -rand_range(0.5, 2.0)
            } else {
                rand_range(0.5, 2.0)
            };
        } else {
            x = xmax / 2.0;
            y = rand_range(0.0, ymax) - ymax / 2.0;
            dx = -rand_range(0.5, 2.0);
            dy = if y > 0.0 {
                -rand_range(0.5, 2.0)
            } else {
                rand_range(0.5, 2.0)
            };
        }
        let z = rand_range(0.0, 7.0) - 9.0;
        let rot = [f_rand(), f_rand(), f_rand()];
        let drot = f_rand() * 3.0;

        let part = match random() % 11 {
            0 => Part::Resistor {
                b: self.new_resistor(),
            },
            1 => {
                let (band, color) = new_diode();
                Part::Diode { band, color }
            }
            2 => {
                let kind = (random() % 3) as usize;
                let text = match kind {
                    0 => TRANSISTOR_TYPES[random() as usize % TRANSISTOR_TYPES.len()],
                    2 => SMC_TYPES[random() as usize % SMC_TYPES.len()],
                    _ => TO92_TYPES[random() as usize % TO92_TYPES.len()],
                };
                Part::Transistor { kind, text }
            }
            3 => {
                let ceramic = f_rand() < 0.5;
                if ceramic {
                    Part::Capacitor {
                        ceramic,
                        width: rand_range(0.3, 1.0),
                        length: 0.0,
                    }
                } else {
                    Part::Capacitor {
                        ceramic,
                        length: rand_range(0.5, 1.0),
                        width: rand_range(0.5, 1.0),
                    }
                }
            }
            4 => new_ic(),
            5 => {
                let mut light = false;
                if !self.light && f_rand() < 0.4 {
                    self.light = true;
                    light = true;
                }
                let r = f_rand();
                let color = if r < 0.2 {
                    [0.9, 0.0, 0.0]
                } else if r < 0.4 {
                    [0.3, 0.9, 0.0]
                } else if r < 0.6 {
                    [0.8, 0.9, 0.0]
                } else if r < 0.8 {
                    [0.0, 0.2, 0.8]
                } else {
                    [0.9, 0.55, 0.0]
                };
                Part::Led { color, light }
            }
            6 => Part::Fuse,
            7 => Part::Rca {
                white: random() % 10 < 5,
            },
            8 => Part::ThreeFive,
            9 => Part::Switch,
            _ => Part::Disp {
                value: if self.seven {
                    7
                } else {
                    rand_range(0.0, 10.0) as usize
                },
            },
        };

        Component {
            x,
            y,
            z,
            dx,
            dy,
            rot,
            drot,
            rdeg: 0.0,
            angle,
            part,
        }
    }

    fn new_resistor(&self) -> [usize; 4] {
        let v = (random() % 9) as usize;
        let m = (random() % 5) as usize;
        let t = if random() % 10 < 5 { 10 } else { 11 };
        if self.seven {
            [7, 7, 7, t]
        } else {
            [VALUES[v][0], VALUES[v][1], m, t]
        }
    }
}

fn new_diode() -> (Band, [f32; 3]) {
    let band = Band {
        pos: 0.8,
        len: 0.1,
        color: [1.0, 1.0, 1.0],
    };
    let color = if f_rand() < 0.5 {
        [0.7, 0.1, 0.1]
    } else {
        [0.2, 0.2, 0.2]
    };
    (band, color)
}

fn new_ic() -> Part {
    let pins = match rand_range(0.0, 4.0) as i32 {
        0 => 8,
        1 => 14,
        2 => 16,
        _ => 24,
    };
    let types: Vec<usize> = (0..IC_TYPES.len())
        .filter(|&i| IC_TYPES[i].0 == pins)
        .collect();
    let val = IC_TYPES[types[random() as usize % types.len()]].1;
    // The part number, then a date code: a year in the eighties or nineties
    // and a week of it.
    let text = format!(
        "{val}\n{:02}{:02}",
        rand_range(80.0, 100.0) as i32,
        rand_range(1.0, 53.0) as i32
    );
    Part::Ic { pins, text }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let maxparts = (g.res.int("parts") as usize).min(MAX_COMPONENTS - 1);
    let uselight = g.res.bool("light");
    let font = TexFont::load(&mut g.glx, g.res.string("componentFont"));

    let mut this = Circuit {
        xmax: 50,
        ymax: 50,
        win_w: g.width(),
        win_h: g.height(),
        // With no lighting asked for, no LED ever gets to be the light.
        light: !uselight,
        lighton: false,
        viewer: [0.0, 0.0, 14.0],
        lightpos: [7.0, 7.0, 15.0, 1.0],
        components: (0..maxparts).map(|_| None).collect(),
        grid_col: [0.0, 0.25, 0.05],
        grid_col2: [0.0, 0.125, 0.05],
        rotate_angle: 0.0,
        font,
        draw_sx: 0.0,
        draw_sy: 0.0,
        draw_sdir: 0,
        draw_s: false,
        draw_ds: 0.0,
        maxparts,
        rotatespeed: g.res.int("rotatespeed"),
        spin: g.res.bool("spin"),
        seven: g.res.bool("seven"),
        aspect: 1.0,
        scale: 1.0,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Circuit {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        let mut h = height as f32 / width as f32;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = h;
        self.win_h = height;
        self.win_w = width;
        self.ymax = (self.xmax as f32 * h) as i32;
        self.scale = if g.width() < g.height() {
            g.height() as f32 / g.width() as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .frustum(-1.0, 1.0, -self.aspect, self.aspect, 1.5, 35.0);
        g.glx.matrix_mode_modelview();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        g.glx.color_material(false);
        g.glx.lighting(true);
        g.glx.load_identity();
        let v = self.viewer;
        g.glx
            .look_at([v[0], v[1], v[2]], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.push_matrix();
        g.glx.rotate(self.rotate_angle, 0.0, 0.0, 1.0);
        self.rotate_angle += 0.01 * self.rotatespeed as f32;
        if self.rotate_angle >= 360.0 {
            self.rotate_angle = 0.0;
        }

        g.glx.light_enable(0, true);
        let lp = self.lightpos;
        g.glx.light_position(0, lp[0], lp[1], lp[2], lp[3]);
        g.glx.light_specular(0, [0.8, 0.8, 0.8, 1.0]);
        g.glx.light_diffuse(0, [0.8, 0.8, 0.8, 1.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

        self.drawgrid(g);
        if f_rand() < 0.05 {
            for j in 0..self.maxparts {
                if self.components[j].is_none() {
                    let c = self.new_component();
                    self.components[j] = Some(c);
                    break;
                }
            }
            // `reorder`: the opaque parts first, then the transparent ones,
            // then the empty slots, so that blending has something to blend
            // with. The sort is stable, so parts otherwise keep their order.
            self.components.sort_by_key(|c| match c {
                Some(c) if !c.part.alpha() => 0,
                Some(_) => 1,
                None => 2,
            });
        }
        for j in 0..self.maxparts {
            g.glx.material_ambient_diffuse([0.0, 0.0, 0.0, 1.0]);
            g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
            self.draw_component(g, j);
        }
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:          20000",
    "*showFPS:        False",
    "*componentFont:  monospace bold 12",
    "*parts:          10",
    "*rotatespeed:    1",
    "*spin:           True",
    "*light:          True",
    "*seven:          False",
];

const RENDER: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "true",
        label: "Directional lighting",
    },
    crate::runtime::opts::SelectItem {
        value: "false",
        label: "Flat coloring",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("parts", "Parts", 1.0, 30.0, 1.0, 0, "10"),
    Opt::slider("rotatespeed", "Rotation speed", 0.0, 100.0, 1.0, 0, "1"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::select("light", "Rendering", RENDER, "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "circuit",
    label: "Circuit",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ben Buxton",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=tfqR1j1OQs8"),
        blurb: "Electronic components float around.",
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

    /// Every chip in the table has a part number and one of the four pin
    /// counts the package switch knows how to draw.
    #[test]
    fn every_chip_fits_a_package() {
        for (pins, val) in IC_TYPES {
            assert!(
                [8, 14, 16, 24].contains(&pins),
                "{val} has {pins} pins, which has no package"
            );
            assert!(!val.is_empty());
        }
        for pins in [8, 14, 16, 24] {
            assert!(
                IC_TYPES.iter().any(|t| t.0 == pins),
                "no chip has {pins} pins, so picking that package would loop"
            );
        }
    }

    /// A resistor's bands are colour-code indices, and there are only twelve
    /// colours.
    #[test]
    fn a_resistor_reads_a_real_value() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        for _ in 0..200 {
            r.step();
        }
        for v in VALUES {
            assert!(v[0] < COLORCODES.len() && v[1] < COLORCODES.len());
        }
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }

    /// A resistor's four bands are four separate colours, which is the whole
    /// point of them: they are the part's value.
    #[test]
    fn a_resistor_wears_its_value() {
        let mut r = start(StartArgs::new(640, 480, "parts=30", 20260812));
        for _ in 0..600 {
            r.step();
        }
        let f = r.frame();
        // The colour codes are materials rather than vertex colours, so look
        // for batches whose diffuse is one of them.
        let mut seen = 0;
        for code in COLORCODES {
            if f.batches.iter().any(|b| {
                let d = b.material.ambient_diffuse;
                (d[0] - code[0]).abs() < 1e-6
                    && (d[1] - code[1]).abs() < 1e-6
                    && (d[2] - code[2]).abs() < 1e-6
            }) {
                seen += 1;
            }
        }
        assert!(seen >= 3, "only {seen} colour codes were drawn");
    }

    /// The grid is behind everything and drawn unlit, so its colour is exactly
    /// the green it was given.
    #[test]
    fn the_grid_is_unlit_green() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        r.step();
        let f = r.frame();
        let grid: Vec<_> = f
            .batches
            .iter()
            .filter(|b| !b.lighting && b.primitive == crate::runtime::gl::Primitive::Lines)
            .collect();
        assert!(!grid.is_empty(), "the grid is missing");
        for b in &grid {
            for v in &f.vertices[b.first..b.first + b.count] {
                assert!(v.color[1] > 0.0, "a grid line is not green");
                assert!(v.color[0] == 0.0, "a grid line is not green");
                assert_eq!(v.pos[2], -10.0, "a grid line is not on the back plane");
            }
        }
    }

    /// Parts arrive at an edge and leave at one, so the scene fills up and
    /// then stays about the same size rather than growing without end.
    #[test]
    fn parts_come_and_go() {
        let mut r = start(StartArgs::new(640, 480, "parts=10", 20260812));
        for _ in 0..100 {
            r.step();
        }
        let early = r.frame().vertices.len();
        for _ in 0..2000 {
            r.step();
        }
        let late = r.frame().vertices.len();
        assert!(early > 0 && late > 0, "nothing was drawn");
        assert!(late < early * 8, "the scene grew from {early} to {late}");
    }
}
