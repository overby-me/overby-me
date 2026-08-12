/* -*- Mode: C; tab-width: 4 -*- */
/* pipes --- 3D selfbuilding pipe system */

/*-
 * Permission to use, copy, modify, and distribute this software and its
 * documentation for any purpose and without fee is hereby granted,
 * provided that the above copyright notice appear in all copies and that
 * both that copyright notice and this permission notice appear in
 * supporting documentation.
 *
 * This file is provided AS IS with no warranties of any kind.  The author
 * shall have no liability with respect to the infringement of copyrights,
 * trade secrets or any patents by this file or any part thereof.  In no
 * event will the author be liable for any lost revenue or profits or
 * other special, indirect and consequential damages.
 *
 * Copyright (c) 1997 by Marcelo F. Vianna (me@mfvianna.com.br)
 *
 * Revision History:
 * 31-May-97: Ported to xlockmore-4
 * 16-Apr-97: Initial version by Marcelo F. Vianna
 */

//! Port of `hacks/glx/pipes.c`.
//!
//! A growing plumbing system, with bolts and valves.
//!
//! A pipe is a self-avoiding walk on a lattice of thirty-three by twenty-five
//! by thirty-three cells. From wherever it starts it looks at its six
//! neighbours, keeps going the way it was going if it can, and now and then
//! picks a free neighbour instead; each step lays a length of pipe across the
//! face it just crossed, and each turn puts an elbow, a ball joint or a bend in
//! the corner where the two meet. When it runs out of free neighbours, or has
//! gone as far as it is allowed, it caps the end with a sphere and another pipe
//! begins somewhere else in a colour that has been used least. After the last
//! one the whole system spins away to nothing and it starts again.
//!
//! With the gadgetry knob up, one step in fifty is a fitting rather than a
//! plain pipe: a bolted collar with a pressure gauge on top of it, a valve with
//! a wheel turned to some angle nobody set, or, once in a very long while, a
//! teapot. Nine of those shapes are models Ed Mackey converted out of Lightwave
//! in 1997 and are read here by [`crate::runtime::lwo`].
//!
//! Upstream draws the system out of one display list per step and calls the
//! first `n` of them, one more each frame, which is how the pipe appears to
//! grow. A display list here is replayed rather than kept on the card, so the
//! system is instead grown once into a flat array of world-space triangles,
//! and a frame draws a prefix of it. That is the same picture with the same
//! arithmetic, done at the start rather than sixty times a second.
//!
//! What that costs is that the last frame of a run has to hand over the whole
//! system, and upstream's default of five pipes five hundred cells long comes
//! to 1.95 million vertices, which is nearly three times the heaviest thing
//! any of these savers draws. A bolted elbow alone is 3234 of them. So the
//! pipe-length default is a hundred and fifty rather than five hundred, which
//! peaks at 546 thousand for one frame and averages half of that; the knob
//! still goes to three thousand for anyone with the card for it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Frame, Glx, Mat4, Primitive, Shape};
use crate::runtime::lwo::Lwo;
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::teapot::unit_teapot;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

const SCALE4WINDOW: f32 = 0.1;
const ONE_THIRD: f32 = 0.333_333_34;

const DIR_NONE: i32 = -1;
const DIR_UP: i32 = 0;
const DIR_DOWN: i32 = 1;
const DIR_LEFT: i32 = 2;
const DIR_RIGHT: i32 = 3;
const DIR_NEAR: i32 = 4;
const DIR_FAR: i32 = 5;

const HCELLS: i32 = 33;
const VCELLS: i32 = 25;
const DEFINEDCOLORS: usize = 7;

const ELBOWRADIUS: f32 = 0.5;
const NOF_SYS_TYPES: i32 = 3;

const FRONT_SHININESS: f32 = 60.0;
const FRONT_SPECULAR: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
const AMBIENT0: [f32; 4] = [0.4, 0.4, 0.4, 1.0];
const DIFFUSE0: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const POSITION0: [f32; 4] = [1.0, 1.0, 1.0, 0.0];
const LMODEL_AMBIENT: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

const MATERIAL_RED: [f32; 4] = [0.7, 0.0, 0.0, 1.0];
const MATERIAL_GREEN: [f32; 4] = [0.1, 0.5, 0.2, 1.0];
const MATERIAL_BLUE: [f32; 4] = [0.0, 0.0, 0.7, 1.0];
const MATERIAL_CYAN: [f32; 4] = [0.2, 0.5, 0.7, 1.0];
const MATERIAL_YELLOW: [f32; 4] = [0.7, 0.7, 0.0, 1.0];
const MATERIAL_MAGENTA: [f32; 4] = [0.6, 0.2, 0.5, 1.0];
const MATERIAL_WHITE: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
const MATERIAL_GRAY: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

/// The seven colours a system may be, in the order `pinit` numbers them.
const SYSTEM_COLORS: [[f32; 4]; DEFINEDCOLORS] = [
    MATERIAL_RED,
    MATERIAL_GREEN,
    MATERIAL_BLUE,
    MATERIAL_CYAN,
    MATERIAL_YELLOW,
    MATERIAL_MAGENTA,
    MATERIAL_WHITE,
];

/// Where an elbow goes and which way it faces, for each of the twenty-four
/// ways a pipe can turn.
///
/// Upstream spells this out as a switch inside a switch, a hundred and fifty
/// lines of `glTranslatef` and `glRotatef`. Every one of those translations is
/// the cell's centre plus or minus a third along two axes, so what is kept here
/// is the two signs and the turns that follow: `(nowdir, newdir, thirds,
/// turns)`.
type Turn = (f32, f32, f32, f32);
const ELBOWS: &[(i32, i32, [f32; 3], &[Turn])] = &[
    (
        DIR_UP,
        DIR_LEFT,
        [-1.0, -1.0, 0.0],
        &[(180.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_UP,
        DIR_RIGHT,
        [1.0, -1.0, 0.0],
        &[(180.0, 1.0, 0.0, 0.0), (180.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_UP,
        DIR_FAR,
        [0.0, -1.0, -1.0],
        &[(90.0, 0.0, 1.0, 0.0), (180.0, 0.0, 0.0, 1.0)],
    ),
    (
        DIR_UP,
        DIR_NEAR,
        [0.0, -1.0, 1.0],
        &[(90.0, 0.0, 1.0, 0.0), (180.0, 1.0, 0.0, 0.0)],
    ),
    (DIR_DOWN, DIR_LEFT, [-1.0, 1.0, 0.0], &[]),
    (
        DIR_DOWN,
        DIR_RIGHT,
        [1.0, 1.0, 0.0],
        &[(180.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_DOWN,
        DIR_FAR,
        [0.0, 1.0, -1.0],
        &[(270.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_DOWN,
        DIR_NEAR,
        [0.0, 1.0, 1.0],
        &[(90.0, 0.0, 1.0, 0.0)],
    ),
    (DIR_LEFT, DIR_UP, [1.0, 1.0, 0.0], &[(180.0, 0.0, 1.0, 0.0)]),
    (
        DIR_LEFT,
        DIR_DOWN,
        [1.0, -1.0, 0.0],
        &[(180.0, 1.0, 0.0, 0.0), (180.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_LEFT,
        DIR_FAR,
        [1.0, 0.0, -1.0],
        &[(270.0, 1.0, 0.0, 0.0), (180.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_LEFT,
        DIR_NEAR,
        [1.0, 0.0, 1.0],
        &[(270.0, 1.0, 0.0, 0.0), (180.0, 0.0, 0.0, 1.0)],
    ),
    (DIR_RIGHT, DIR_UP, [-1.0, 1.0, 0.0], &[]),
    (
        DIR_RIGHT,
        DIR_DOWN,
        [-1.0, -1.0, 0.0],
        &[(180.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_RIGHT,
        DIR_FAR,
        [-1.0, 0.0, -1.0],
        &[(270.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_RIGHT,
        DIR_NEAR,
        [-1.0, 0.0, 1.0],
        &[(90.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_NEAR,
        DIR_LEFT,
        [-1.0, 0.0, -1.0],
        &[(270.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_NEAR,
        DIR_RIGHT,
        [1.0, 0.0, -1.0],
        &[(270.0, 1.0, 0.0, 0.0), (180.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_NEAR,
        DIR_UP,
        [0.0, 1.0, -1.0],
        &[(270.0, 0.0, 1.0, 0.0)],
    ),
    (
        DIR_NEAR,
        DIR_DOWN,
        [0.0, -1.0, -1.0],
        &[(90.0, 0.0, 1.0, 0.0), (180.0, 0.0, 0.0, 1.0)],
    ),
    (DIR_FAR, DIR_UP, [0.0, 1.0, 1.0], &[(90.0, 0.0, 1.0, 0.0)]),
    (
        DIR_FAR,
        DIR_DOWN,
        [0.0, -1.0, 1.0],
        &[(90.0, 0.0, 1.0, 0.0), (180.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_FAR,
        DIR_LEFT,
        [-1.0, 0.0, 1.0],
        &[(90.0, 1.0, 0.0, 0.0)],
    ),
    (
        DIR_FAR,
        DIR_RIGHT,
        [1.0, 0.0, 1.0],
        &[(270.0, 1.0, 0.0, 0.0), (180.0, 0.0, 0.0, 1.0)],
    ),
];

/// `NRAND`.
fn nrand(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    (random() % n as u32) as i32
}

/* -------------------------------------------------------------------------
 * The shapes
 * ---------------------------------------------------------------------- */

/// The nine Lightwave models, read once.
struct Models {
    valve: Lwo,
    bolts: Lwo,
    betweenbolts: Lwo,
    elbowbolts: Lwo,
    elbowcoins: Lwo,
    guagehead: Lwo,
    guageface: Lwo,
    guagedial: Lwo,
    guageconnector: Lwo,
}

impl Models {
    fn load() -> Self {
        Models {
            valve: Lwo::parse(crate::models::PIPES_BIGVALVE),
            bolts: Lwo::parse(crate::models::PIPES_BOLTS3D),
            betweenbolts: Lwo::parse(crate::models::PIPES_PIPEBETWEENBOLTS),
            elbowbolts: Lwo::parse(crate::models::PIPES_ELBOWBOLTS),
            elbowcoins: Lwo::parse(crate::models::PIPES_ELBOWCOINS),
            guagehead: Lwo::parse(crate::models::PIPES_GUAGEHEAD),
            guageface: Lwo::parse(crate::models::PIPES_GUAGEFACE),
            guagedial: Lwo::parse(crate::models::PIPES_GUAGEDIAL),
            guageconnector: Lwo::parse(crate::models::PIPES_GUAGECONNECTOR),
        }
    }
}

/// `MakeTube`: one cell's length of pipe, lying along `direction`.
fn make_tube(gl: &mut Glx, wire: bool, direction: i32) {
    let facets = if wire { 5 } else { 24 };

    /* dirUP    = 00000000 */
    /* dirDOWN  = 00000001 */
    /* dirLEFT  = 00000010 */
    /* dirRIGHT = 00000011 */
    /* dirNEAR  = 00000100 */
    /* dirFAR   = 00000101 */
    if direction & 4 == 0 {
        let sideways = direction & 2 != 0;
        gl.rotate(
            90.0,
            if sideways { 0.0 } else { 1.0 },
            if sideways { 1.0 } else { 0.0 },
            0.0,
        );
    }
    gl.begin(if wire {
        Shape::LineStrip
    } else {
        Shape::QuadStrip
    });
    let step = std::f32::consts::TAU / facets as f32;
    let mut an = 0.0;
    while an <= std::f32::consts::TAU {
        let (sin_3, cos_3) = (an.sin() / 3.0, an.cos() / 3.0);
        gl.normal3f(cos_3, sin_3, 0.0);
        gl.vertex3f(cos_3, sin_3, ONE_THIRD);
        gl.vertex3f(cos_3, sin_3, -ONE_THIRD);
        an += step;
    }
    gl.end();
}

/// `mySphere`: upstream's stand-in for `gluSphere`.
fn my_sphere(gl: &mut Glx, radius: f32, wire: bool) {
    gl.push_matrix();
    gl.scale(radius, radius, radius);
    gl.rotate(90.0, 1.0, 0.0, 0.0);
    unit_sphere(gl, 16, 16, wire);
    gl.pop_matrix();
}

/// `myElbow`: a quarter of a torus, and the ring of coins and bolts that holds
/// it on when the system is the bolted kind.
fn my_elbow(
    gl: &mut Glx,
    wire: bool,
    models: &Models,
    factory: i32,
    color: [f32; 4],
    bolted: bool,
) {
    let nsides: i32 = if wire { 6 } else { 25 };
    let rings = nsides;
    const R: f32 = ONE_THIRD;

    for i in 0..=rings / 4 {
        let theta = i as f32 * std::f32::consts::TAU / rings as f32;
        let theta1 = (i + 1) as f32 * std::f32::consts::TAU / rings as f32;
        for j in 0..nsides {
            let phi = j as f32 * std::f32::consts::TAU / nsides as f32;
            let phi1 = (j + 1) as f32 * std::f32::consts::TAU / nsides as f32;

            let (cos_theta, cos_theta1) = (theta.cos(), theta1.cos());
            let (sin_theta, sin_theta1) = (-theta.sin(), -theta1.sin());
            let (cos_phi, cos_phi1) = (phi.cos(), phi1.cos());

            let n0 = [cos_theta * cos_phi, sin_theta * cos_phi, phi.sin()];
            let n1 = [cos_theta1 * cos_phi, sin_theta1 * cos_phi, phi.sin()];
            let n2 = [cos_theta1 * cos_phi1, sin_theta1 * cos_phi1, phi1.sin()];
            let n3 = [cos_theta * cos_phi1, sin_theta * cos_phi1, phi1.sin()];

            let p0 = [
                cos_theta * (R + R * cos_phi),
                sin_theta * (R + R * cos_phi),
                R * n0[2],
            ];
            let p1 = [
                cos_theta1 * (R + R * cos_phi),
                sin_theta1 * (R + R * cos_phi),
                R * n1[2],
            ];
            let p2 = [
                cos_theta1 * (R + R * cos_phi1),
                sin_theta1 * (R + R * cos_phi1),
                R * n2[2],
            ];
            let p3 = [
                cos_theta * (R + R * cos_phi1),
                sin_theta * (R + R * cos_phi1),
                R * n3[2],
            ];

            gl.begin(if wire { Shape::LineLoop } else { Shape::Quads });
            for (n, p) in [(n3, p3), (n2, p2), (n1, p1), (n0, p0)] {
                gl.normal3f(n[0], n[1], n[2]);
                gl.vertex3f(p[0], p[1], p[2]);
            }
            gl.end();
        }
    }

    if factory > 0 && bolted {
        /* Bolt the elbow onto the pipe system */
        gl.front_face_cw(true);
        gl.push_matrix();
        gl.rotate(90.0, 0.0, 0.0, -1.0);
        gl.rotate(90.0, 0.0, 1.0, 0.0);
        gl.translate(0.0, ONE_THIRD, ONE_THIRD);
        models.elbowcoins.render(gl, wire);
        gl.material_diffuse(MATERIAL_GRAY);
        models.elbowbolts.render(gl, wire);
        gl.material_diffuse(color);
        gl.pop_matrix();
        gl.front_face_cw(false);
    }
}

/// The turns that stand a fitting up the way the pipe runs. Upstream repeats
/// this switch in `MakeValve` and again in `MakeTeapot`.
fn orient_fitting(gl: &mut Glx, newdir: i32) {
    match newdir {
        DIR_UP | DIR_DOWN => {
            gl.rotate(90.0, 1.0, 0.0, 0.0);
            gl.rotate(nrand(3) as f32 * 90.0, 0.0, 0.0, 1.0);
        }
        DIR_LEFT | DIR_RIGHT => {
            gl.rotate(90.0, 0.0, -1.0, 0.0);
            gl.rotate(nrand(3) as f32 * 90.0 - 90.0, 0.0, 0.0, 1.0);
        }
        _ => gl.rotate(nrand(4) as f32 * 90.0, 0.0, 0.0, 1.0),
    }
}

/// `MakeValve`: a bolted collar with a wheel on it, turned to some angle
/// between nought and ninety degrees that nobody chose.
fn make_valve(gl: &mut Glx, wire: bool, models: &Models, color: [f32; 4], newdir: i32) {
    orient_fitting(gl, newdir);

    gl.front_face_cw(true);
    models.betweenbolts.render(gl, wire);
    gl.material_diffuse(MATERIAL_GRAY);
    models.bolts.render(gl, wire);

    // The wheel is never the colour of the pipe it is bolted to.
    let wheel = if color == MATERIAL_RED {
        if nrand(2) != 0 {
            MATERIAL_YELLOW
        } else {
            MATERIAL_BLUE
        }
    } else if color == MATERIAL_BLUE {
        if nrand(2) != 0 {
            MATERIAL_RED
        } else {
            MATERIAL_YELLOW
        }
    } else if color == MATERIAL_YELLOW {
        if nrand(2) != 0 {
            MATERIAL_BLUE
        } else {
            MATERIAL_RED
        }
    } else {
        match nrand(3) {
            0 => MATERIAL_RED,
            1 => MATERIAL_BLUE,
            _ => MATERIAL_YELLOW,
        }
    };
    gl.material_diffuse(wheel);

    gl.rotate(nrand(90) as f32, 1.0, 0.0, 0.0);
    models.valve.render(gl, wire);
    gl.material_diffuse(color);
    gl.front_face_cw(false);
}

/// `MakeTeapot`: once in every five thousand steps, for no reason at all.
fn make_teapot(gl: &mut Glx, wire: bool, newdir: i32) {
    orient_fitting(gl, newdir);
    unit_teapot(gl, 12, wire);
    gl.front_face_cw(false);
}

/* -------------------------------------------------------------------------
 * Baking
 * ---------------------------------------------------------------------- */

/// One baked vertex: where it is in the system, and which way it faces.
#[derive(Clone, Copy)]
struct Vert {
    pos: [f32; 3],
    normal: [f32; 3],
}

/// A stretch of the baked system that is all one colour.
struct Run {
    color: [f32; 4],
    /// Where this run ends in each of the two streams.
    tris: usize,
    lines: usize,
}

/// The whole grown system, flat.
#[derive(Default)]
struct System {
    tris: Vec<Vert>,
    lines: Vec<Vert>,
    runs: Vec<Run>,
    /// How much of each stream is complete after each step, which is what
    /// makes the pipe appear to grow: a frame draws the first `n` steps.
    steps: Vec<(usize, usize)>,
}

/// The rotation half of a modelview, applied to a normal and made unit again.
/// Every matrix in this saver is turns, moves and one uniform scale, so the
/// upper three by three is all a normal needs and `GL_NORMALIZE` does the rest.
fn turn_normal(m: &Mat4, n: [f32; 3]) -> [f32; 3] {
    let a = &m.0;
    let out = [
        a[0] * n[0] + a[4] * n[1] + a[8] * n[2],
        a[1] * n[0] + a[5] * n[1] + a[9] * n[2],
        a[2] * n[0] + a[6] * n[1] + a[10] * n[2],
    ];
    let d = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt();
    if d == 0.0 {
        out
    } else {
        [out[0] / d, out[1] / d, out[2] / d]
    }
}

/// Bake one step of the oven's output into world space.
///
/// What comes out is triangles and lines and nothing else, because those are
/// the two primitives the runtime can merge: a system left as quad strips and
/// polygons would be twenty-five hundred draw calls. A batch wound the other
/// way round is baked the other way round rather than carrying a flag out with
/// it, so the whole system can be drawn with one winding.
fn harvest(frame: &Frame, wire: bool, color: [f32; 4], out: &mut System) {
    for b in &frame.batches {
        if b.count == 0 {
            continue;
        }
        let m = b.modelview;
        let vs = &frame.vertices[b.first..b.first + b.count];
        let at = |i: usize| Vert {
            pos: m.transform(vs[i].pos),
            normal: turn_normal(&m, vs[i].normal),
        };
        let (a, c) = if b.front_face_cw { (2, 1) } else { (1, 2) };

        match b.primitive {
            Primitive::Triangles => {
                for k in (0..vs.len().saturating_sub(2)).step_by(3) {
                    out.tris.extend([at(k), at(k + a), at(k + c)]);
                }
            }
            Primitive::TriangleStrip => {
                for k in 0..vs.len().saturating_sub(2) {
                    // Every other triangle of a strip is wound backwards.
                    let (a, c) = if k % 2 == 0 { (a, c) } else { (c, a) };
                    out.tris.extend([at(k), at(k + a), at(k + c)]);
                }
            }
            Primitive::TriangleFan => {
                for k in 1..vs.len().saturating_sub(1) {
                    out.tris.extend([at(0), at(k + a - 1), at(k + c - 1)]);
                }
            }
            Primitive::Lines => {
                for k in (0..vs.len().saturating_sub(1)).step_by(2) {
                    out.lines.extend([at(k), at(k + 1)]);
                }
            }
            Primitive::LineStrip => {
                for k in 0..vs.len().saturating_sub(1) {
                    out.lines.extend([at(k), at(k + 1)]);
                }
            }
            Primitive::LineLoop => {
                for k in 0..vs.len() {
                    out.lines.extend([at(k), at((k + 1) % vs.len())]);
                }
            }
            Primitive::Points => {
                for k in 0..vs.len() {
                    out.lines.extend([at(k), at(k)]);
                }
            }
        }

        // In wireframe upstream draws the whole system in the system's colour:
        // the materials the fittings set are never looked at, because the
        // lighting they would have fed is switched off.
        let color = if wire {
            color
        } else {
            b.material.ambient_diffuse
        };
        match out.runs.last_mut() {
            Some(r) if r.color == color => {
                r.tris = out.tris.len();
                r.lines = out.lines.len();
            }
            _ => out.runs.push(Run {
                color,
                tris: out.tris.len(),
                lines: out.lines.len(),
            }),
        }
    }
}

/* -------------------------------------------------------------------------
 * The walk
 * ---------------------------------------------------------------------- */

/// The knobs, resolved once.
struct Config {
    factory: i32,
    fisheye: bool,
    tightturns: bool,
    rotatepipes: bool,
    system_type: i32,
    number_of_systems: i32,
    system_length: i32,
    wire: bool,
}

/// Upstream's `pipesstruct`, less everything to do with drawing.
struct Grower {
    cells: Vec<u8>,
    usedcolors: [i32; DEFINEDCOLORS],
    directions: [bool; 6],
    ndirections: i32,
    nowdir: i32,
    olddir: i32,
    system_number: i32,
    counter: i32,
    px: i32,
    py: i32,
    pz: i32,
    turncounter: i32,
    system_color: [f32; 4],
}

impl Grower {
    fn new() -> Self {
        Grower {
            cells: vec![0; (HCELLS * VCELLS * HCELLS) as usize],
            usedcolors: [0; DEFINEDCOLORS],
            directions: [false; 6],
            ndirections: 0,
            nowdir: DIR_NONE,
            olddir: DIR_NONE,
            system_number: 1,
            counter: 0,
            px: 0,
            py: 0,
            pz: 0,
            turncounter: 0,
            system_color: MATERIAL_GRAY,
        }
    }

    fn cell(&self, x: i32, y: i32, z: i32) -> u8 {
        self.cells[((x * VCELLS + y) * HCELLS + z) as usize]
    }

    fn set_cell(&mut self, x: i32, y: i32, z: i32) {
        self.cells[((x * VCELLS + y) * HCELLS + z) as usize] = 1;
    }

    /// `FindNeighbors`: which of the six ways out of this cell are free.
    fn find_neighbors(&mut self) {
        let (x, y, z) = (self.px, self.py, self.pz);
        let free = [
            self.cell(x, y + 1, z) == 0,
            self.cell(x, y - 1, z) == 0,
            self.cell(x - 1, y, z) == 0,
            self.cell(x + 1, y, z) == 0,
            self.cell(x, y, z + 1) == 0,
            self.cell(x, y, z - 1) == 0,
        ];
        // Upstream fills `directions` in dir order, which puts NEAR before FAR
        // and so is not the order the tests above read best in.
        self.directions[DIR_UP as usize] = free[0];
        self.directions[DIR_DOWN as usize] = free[1];
        self.directions[DIR_LEFT as usize] = free[2];
        self.directions[DIR_RIGHT as usize] = free[3];
        self.directions[DIR_NEAR as usize] = free[4];
        self.directions[DIR_FAR as usize] = free[5];
        self.ndirections = self.directions.iter().filter(|d| **d).count() as i32;
    }

    /// `SelectNeighbor`: one of the free ways out, at random.
    fn select_neighbor(&self) -> i32 {
        let dirlist: Vec<i32> = (0..6).filter(|i| self.directions[*i as usize]).collect();
        dirlist[nrand(self.ndirections) as usize]
    }

    /// `pinit`: start a system. With `zera` set, start the whole thing over.
    fn pinit(&mut self, zera: bool) {
        if zera {
            self.system_number = 1;
            self.cells.fill(0);
            // The outermost shell of cells is filled in, so the walk can never
            // step off the lattice and never has to check that it might.
            for x in 0..HCELLS {
                for y in 0..VCELLS {
                    self.set_cell(x, y, 0);
                    self.set_cell(x, y, HCELLS - 1);
                    self.set_cell(0, y, x);
                    self.set_cell(HCELLS - 1, y, x);
                }
            }
            for x in 0..HCELLS {
                for z in 0..HCELLS {
                    self.set_cell(x, 0, z);
                    self.set_cell(x, VCELLS - 1, z);
                }
            }
            self.usedcolors = [0; DEFINEDCOLORS];
        }
        self.counter = 0;
        self.turncounter = 0;

        /* Avoid repeating colors on the same screen unless necessary */
        let lower = self.usedcolors.iter().copied().min().unwrap_or(0);
        let collist: Vec<usize> = (0..DEFINEDCOLORS)
            .filter(|i| self.usedcolors[*i] == lower)
            .collect();
        let i = collist[nrand(collist.len() as i32) as usize];
        self.usedcolors[i] += 1;
        self.system_color = SYSTEM_COLORS[i];

        loop {
            self.px = nrand(HCELLS - 1) + 1;
            self.py = nrand(VCELLS - 1) + 1;
            self.pz = nrand(HCELLS - 1) + 1;
            let (x, y, z) = (self.px, self.py, self.pz);
            // The second test is only ever reached for a free cell, which by
            // the shell above is never on the edge, so the neighbours it looks
            // at are always there.
            if self.cell(x, y, z) == 0
                && !(self.cell(x + 1, y, z) != 0
                    && self.cell(x - 1, y, z) != 0
                    && self.cell(x, y + 1, z) != 0
                    && self.cell(x, y - 1, z) != 0
                    && self.cell(x, y, z + 1) != 0
                    && self.cell(x, y, z - 1) != 0)
            {
                break;
            }
        }
        self.set_cell(self.px, self.py, self.pz);
        self.olddir = DIR_NONE;
        self.find_neighbors();
        self.nowdir = self.select_neighbor();
    }
}

/// The centre of a cell, in the space the system is drawn in.
fn cell_centre(x: f32, y: f32, z: f32) -> [f32; 3] {
    [
        (x - 16.0) / 3.0 * 4.0,
        (y - 12.0) / 3.0 * 4.0,
        (z - 16.0) / 3.0 * 4.0,
    ]
}

/// `generate_system`: grow the whole thing, once.
fn generate_system(cfg: &Config, models: &Models) -> System {
    let wire = cfg.wire;
    let mut g = Grower::new();
    let mut out = System::default();

    // The oven. Nothing is ever shown from it: it is run one step at a time so
    // that the vertices it collects can be read back out in world space.
    let mut oven = Glx::new();

    g.pinit(true);

    loop {
        oven.start_frame(1, 1);
        oven.matrix_mode_modelview();
        oven.load_identity();
        oven.front_face_cw(false);
        oven.material_diffuse(g.system_color);
        oven.push_matrix();

        g.find_neighbors();

        /* If it's the begining of a system, draw a sphere */
        if g.olddir == DIR_NONE {
            oven.push_matrix();
            let c = cell_centre(g.px as f32, g.py as f32, g.pz as f32);
            oven.translate(c[0], c[1], c[2]);
            my_sphere(&mut oven, 0.6, wire);
            oven.pop_matrix();
        }

        /* Check for stop conditions */
        if g.ndirections == 0 || g.counter > cfg.system_length {
            oven.push_matrix();
            let c = cell_centre(g.px as f32, g.py as f32, g.pz as f32);
            oven.translate(c[0], c[1], c[2]);
            /* Finish the system with another sphere */
            my_sphere(&mut oven, 0.6, wire);
            oven.pop_matrix();

            oven.pop_matrix();
            harvest(oven.frame(), wire, g.system_color, &mut out);
            out.steps.push((out.tris.len(), out.lines.len()));

            g.system_number += 1;
            if g.system_number > cfg.number_of_systems {
                break;
            }
            g.pinit(false);
            continue;
        }

        g.counter += 1;
        g.turncounter += 1;

        /* Do will the direction change? if so, determine the new one */
        let mut newdir = g.nowdir;
        if !g.directions[newdir as usize] {
            /* cannot proceed in the current direction */
            newdir = g.select_neighbor();
        } else if cfg.tightturns {
            /* random change (20% chance) */
            if g.counter > 1 && nrand(100) < 20 {
                newdir = g.select_neighbor();
            }
        } else {
            /* Chance to turn increases after each length of pipe drawn */
            if g.counter > 1 && nrand(50) < nrand(g.turncounter + 1) {
                newdir = g.select_neighbor();
                g.turncounter = 0;
            }
        }

        if newdir == g.nowdir {
            /* If not, draw the cell's center pipe */
            oven.push_matrix();
            let c = cell_centre(g.px as f32, g.py as f32, g.pz as f32);
            oven.translate(c[0], c[1], c[2]);
            /* Chance of factory shape here, if enabled. */
            if g.counter > 1 && nrand(100) < cfg.factory {
                make_shape(&mut g, &mut oven, cfg, models, newdir);
            } else {
                make_tube(&mut oven, wire, newdir);
            }
            oven.pop_matrix();
        } else {
            /* If so, draw the cell's center elbow/sphere */
            let mut sys_t = cfg.system_type;
            if sys_t == NOF_SYS_TYPES + 1 {
                sys_t = (g.system_number - 1) % NOF_SYS_TYPES + 1;
            }
            oven.push_matrix();
            let c = cell_centre(g.px as f32, g.py as f32, g.pz as f32);
            if sys_t == 1 {
                oven.translate(c[0], c[1], c[2]);
                my_sphere(&mut oven, ELBOWRADIUS, wire);
            } else {
                let elbow = ELBOWS
                    .iter()
                    .find(|(now, new, _, _)| *now == g.nowdir && *new == newdir);
                if let Some((_, _, thirds, turns)) = elbow {
                    oven.translate(
                        c[0] + thirds[0] * ONE_THIRD,
                        c[1] + thirds[1] * ONE_THIRD,
                        c[2] + thirds[2] * ONE_THIRD,
                    );
                    for (a, x, y, z) in *turns {
                        oven.rotate(*a, *x, *y, *z);
                    }
                }
                my_elbow(
                    &mut oven,
                    wire,
                    models,
                    cfg.factory,
                    g.system_color,
                    sys_t == 2,
                );
            }
            oven.pop_matrix();
        }

        let (opx, opy, opz) = (g.px, g.py, g.pz);
        g.olddir = g.nowdir;
        g.nowdir = newdir;
        match g.nowdir {
            DIR_UP => g.py += 1,
            DIR_DOWN => g.py -= 1,
            DIR_LEFT => g.px -= 1,
            DIR_RIGHT => g.px += 1,
            DIR_NEAR => g.pz += 1,
            _ => g.pz -= 1,
        }
        g.set_cell(g.px, g.py, g.pz);

        /* Cells'face pipe */
        let c = cell_centre(
            (g.px + opx) as f32 / 2.0,
            (g.py + opy) as f32 / 2.0,
            (g.pz + opz) as f32 / 2.0,
        );
        oven.translate(c[0], c[1], c[2]);
        make_tube(&mut oven, wire, newdir);

        oven.pop_matrix();
        harvest(oven.frame(), wire, g.system_color, &mut out);
        out.steps.push((out.tris.len(), out.lines.len()));
    }

    out
}

/// `MakeGuage`: a pressure gauge standing on the pipe, if there is room above
/// it for one. Returns false when there is not, and the caller lays plain pipe.
fn make_guage(
    g: &mut Grower,
    gl: &mut Glx,
    wire: bool,
    models: &Models,
    color: [f32; 4],
    newdir: i32,
) -> bool {
    /* Can't have a guage on a vertical pipe. */
    if newdir == DIR_UP || newdir == DIR_DOWN {
        return false;
    }
    /* Is there space above this pipe for a guage? */
    if !g.directions[DIR_UP as usize] {
        return false;
    }
    /* Yes!  Mark the space as used. */
    g.set_cell(g.px, g.py + 1, g.pz);

    gl.front_face_cw(true);
    gl.push_matrix();
    if newdir == DIR_LEFT || newdir == DIR_RIGHT {
        gl.rotate(90.0, 0.0, 1.0, 0.0);
    }
    models.betweenbolts.render(gl, wire);
    gl.material_diffuse(MATERIAL_GRAY);
    models.bolts.render(gl, wire);
    gl.pop_matrix();

    models.guageconnector.render(gl, wire);
    gl.push_matrix();
    gl.translate(0.0, 1.33333, 0.0);
    /* Do not change the above to 1 + ONE_THIRD, because */
    /* the object really is centered on 1.3333300000. */
    gl.rotate(nrand(270) as f32 + 45.0, 0.0, 0.0, -1.0);
    /* Random rotation for the dial.  I love it. */
    models.guagedial.render(gl, wire);
    gl.pop_matrix();

    gl.material_diffuse(color);
    models.guagehead.render(gl, wire);
    /* GuageFace is drawn last, in case of low-res depth buffers. */
    gl.material_diffuse(MATERIAL_WHITE);
    models.guageface.render(gl, wire);
    gl.material_diffuse(color);
    gl.front_face_cw(false);
    true
}

/// `MakeShape`: which piece of gadgetry this step gets.
fn make_shape(g: &mut Grower, gl: &mut Glx, cfg: &Config, models: &Models, newdir: i32) {
    let n = nrand(100);
    let color = g.system_color;
    if n < 50 {
        if !make_guage(g, gl, cfg.wire, models, color, newdir) {
            make_tube(gl, cfg.wire, newdir);
        }
    } else if n < 98 {
        make_valve(gl, cfg.wire, models, color, newdir);
    } else {
        make_teapot(gl, cfg.wire, newdir);
    }
}

/* -------------------------------------------------------------------------
 * The saver
 * ---------------------------------------------------------------------- */

struct PipesState {
    cfg: Config,
    models: Models,
    system: System,
    /// How many steps of the system are on screen. One more every frame.
    system_index: usize,
    /// A hundred down to nought while the finished system spins away.
    fadeout: i32,
    initial_rotation: f32,
    trackball: Trackball,
}

impl PipesState {
    fn regenerate(&mut self) {
        self.system = generate_system(&self.cfg, &self.models);
        self.system_index = 0;
    }
}

impl Hack3d for PipesState {
    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = f64::from(height) / f64::from(width.max(1));
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = f64::from(height) / f64::from(width);
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(65.0, (1.0 / h) as f32, 0.1, 20.0);
        g.glx.matrix_mode_modelview();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            self.fadeout = 100;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.cfg.wire;
        g.glx.clear();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.light_ambient(0, AMBIENT0);
        g.glx.light_diffuse(0, DIFFUSE0);
        g.glx
            .light_position(0, POSITION0[0], POSITION0[1], POSITION0[2], POSITION0[3]);
        g.glx.light_model_ambient(LMODEL_AMBIENT);

        if wire {
            g.glx.lighting(false);
        } else {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            /* This looks crappy, so light 1 stays off. */
            g.glx.depth_test(true);
            g.glx.cull_face(true);
        }
        g.glx.front_face_cw(false);
        g.glx.material_shininess(FRONT_SHININESS);
        g.glx.material_specular(FRONT_SPECULAR);

        g.glx.push_matrix();
        self.initial_rotation += 0.02;
        g.glx
            .translate(0.0, 0.0, if self.cfg.fisheye { -3.8 } else { -4.8 });
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        if self.cfg.rotatepipes {
            g.glx.rotate(self.initial_rotation, 0.0, 1.0, 0.0);
        }
        g.glx.scale(SCALE4WINDOW, SCALE4WINDOW, SCALE4WINDOW);

        if self.fadeout > 0 {
            let s = (self.fadeout * self.fadeout) as f32 / 10000.0;
            g.glx.scale(s, s, s);
            g.glx
                .rotate(90.0 * (1.0 - self.fadeout as f32 / 100.0), 1.0, 0.0, 0.1);
            self.fadeout -= 4;
            if self.fadeout <= 0 {
                self.fadeout = 0;
                self.regenerate();
            }
        } else if self.system_index < self.system.steps.len() {
            self.system_index += 1;
        } else {
            self.fadeout = 100;
        }

        self.draw_system(g);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }
}

impl PipesState {
    /// Draw the first `system_index` steps of the system.
    ///
    /// The vertices are already where they belong, so all this does is walk the
    /// colour runs and stop each one at the step the growth has reached.
    fn draw_system(&mut self, g: &mut Gl) {
        let Some(&(tri_limit, line_limit)) = self
            .system_index
            .checked_sub(1)
            .and_then(|i| self.system.steps.get(i))
        else {
            return;
        };
        let wire = self.cfg.wire;
        let mut prev = (0usize, 0usize);
        for r in &self.system.runs {
            let end = (r.tris.min(tri_limit), r.lines.min(line_limit));
            if end.0 > prev.0 || end.1 > prev.1 {
                if wire {
                    g.glx
                        .color4f(r.color[0], r.color[1], r.color[2], r.color[3]);
                } else {
                    g.glx.material_diffuse(r.color);
                }
            }
            if end.0 > prev.0 {
                g.glx.begin(Shape::Triangles);
                for v in &self.system.tris[prev.0..end.0] {
                    g.glx.normal3f(v.normal[0], v.normal[1], v.normal[2]);
                    g.glx.vertex3f(v.pos[0], v.pos[1], v.pos[2]);
                }
                g.glx.end();
            }
            if end.1 > prev.1 {
                g.glx.begin(Shape::Lines);
                for v in &self.system.lines[prev.1..end.1] {
                    g.glx.normal3f(v.normal[0], v.normal[1], v.normal[2]);
                    g.glx.vertex3f(v.pos[0], v.pos[1], v.pos[2]);
                }
                g.glx.end();
            }
            prev = end;
            if r.tris >= tri_limit && r.lines >= line_limit {
                break;
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let count = g.res.int("count");
    let cycles = g.res.int("cycles");
    let size = g.res.int("size");

    let cfg = Config {
        factory: g.res.int("factory"),
        fisheye: g.res.bool("fisheye"),
        tightturns: g.res.bool("tightturns"),
        rotatepipes: g.res.bool("rotatepipes"),
        system_type: if !(1..=NOF_SYS_TYPES + 1).contains(&count) {
            nrand(NOF_SYS_TYPES) + 1
        } else {
            count
        },
        number_of_systems: if cycles > 0 && cycles < 11 { cycles } else { 5 },
        system_length: size.clamp(10, 1000),
        wire: g.res.bool("wireframe"),
    };

    let models = Models::load();
    let system = generate_system(&cfg, &models);

    let mut st = PipesState {
        cfg,
        models,
        system,
        system_index: 0,
        fadeout: 0,
        initial_rotation: if g.res.bool("rotatepipes") {
            nrand(180) as f32
        } else {
            -10.0
        },
        trackball: Trackball::new(),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

/// Upstream's, save for `size`: see the frame budget at the top of the file.
const DEFAULTS: &[&str] = &[
    "*delay:      10000",
    "*count:      2",
    "*cycles:     5",
    "*size:       150",
    "*showFPS:    False",
    "*wireframe:  False",
    "*suppressRotationAnimation: True",
    "*factory:    2",
    "*fisheye:    True",
    "*tightturns: False",
    "*rotatepipes: True",
];

/// Which corner piece a system is plumbed with. Upstream's `count`, and its
/// fourth value, which cycles through the other three, is not offered here
/// because upstream's own settings panel does not offer it either.
const STYLES: &[SelectItem] = &[
    SelectItem {
        value: "2",
        label: "Bolted fittings",
    },
    SelectItem {
        value: "3",
        label: "Curved pipes",
    },
    SelectItem {
        value: "1",
        label: "Ball joints",
    },
    SelectItem {
        value: "0",
        label: "Random style",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("cycles", "Number of pipes", 1.0, 100.0, 1.0, 0, "5"),
    Opt::slider("size", "Pipe length", 0.0, 3000.0, 10.0, 0, "150"),
    Opt::slider("factory", "Gadgetry", 0.0, 10.0, 1.0, 0, "2"),
    Opt::select("count", "Style", STYLES, "2"),
    Opt::boolean("fisheye", "Fisheye lens", "true"),
    Opt::boolean("tightturns", "Allow tight turns", "false"),
    Opt::boolean("rotatepipes", "Rotate", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pipes",
    label: "Pipes",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Marcelo Vianna and Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=UsUGENa7jvE"),
        blurb: "A growing plumbing system, with bolts and valves.",
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
    use crate::runtime::ya_rand_init;

    /// The knobs a default system is grown with.
    fn default_config() -> Config {
        Config {
            factory: 2,
            fisheye: true,
            tightturns: false,
            rotatepipes: true,
            system_type: 2,
            number_of_systems: 5,
            system_length: 150,
            wire: false,
        }
    }

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// A grower to drive by hand. The seed matters: `pinit` picks a starting
    /// cell by rejection, so it needs a generator that is really generating.
    fn grower(seed: u32) -> Grower {
        ya_rand_init(seed);
        let mut g = Grower::new();
        g.pinit(true);
        g
    }

    /// A system is a self-avoiding walk: no cell is ever entered twice.
    #[test]
    fn the_walk_never_crosses_itself() {
        let cfg = default_config();
        let mut g = grower(20260812);
        let mut visited = std::collections::HashSet::new();
        visited.insert((g.px, g.py, g.pz));
        let mut systems = 1;
        for _ in 0..20_000 {
            g.find_neighbors();
            if g.ndirections == 0 || g.counter > cfg.system_length {
                g.system_number += 1;
                if g.system_number > cfg.number_of_systems {
                    break;
                }
                g.pinit(false);
                systems += 1;
                assert!(
                    visited.insert((g.px, g.py, g.pz)),
                    "a system began inside another"
                );
                continue;
            }
            g.counter += 1;
            let newdir = if g.directions[g.nowdir as usize] {
                g.nowdir
            } else {
                g.select_neighbor()
            };
            g.nowdir = newdir;
            match newdir {
                DIR_UP => g.py += 1,
                DIR_DOWN => g.py -= 1,
                DIR_LEFT => g.px -= 1,
                DIR_RIGHT => g.px += 1,
                DIR_NEAR => g.pz += 1,
                _ => g.pz -= 1,
            }
            g.set_cell(g.px, g.py, g.pz);
            assert!(
                visited.insert((g.px, g.py, g.pz)),
                "the pipe walked into itself at {:?}",
                (g.px, g.py, g.pz)
            );
        }
        assert_eq!(systems, 5);
    }

    /// No system ever leaves the lattice, because the shell of filled cells
    /// around it is never free to walk into.
    #[test]
    fn the_walk_stays_inside_the_lattice() {
        for seed in [1, 20260812, 999_999] {
            let mut g = grower(seed);
            for _ in 0..3000 {
                g.find_neighbors();
                if g.ndirections == 0 {
                    break;
                }
                match g.select_neighbor() {
                    DIR_UP => g.py += 1,
                    DIR_DOWN => g.py -= 1,
                    DIR_LEFT => g.px -= 1,
                    DIR_RIGHT => g.px += 1,
                    DIR_NEAR => g.pz += 1,
                    _ => g.pz -= 1,
                }
                g.set_cell(g.px, g.py, g.pz);
                assert!((1..HCELLS - 1).contains(&g.px), "x left the lattice");
                assert!((1..VCELLS - 1).contains(&g.py), "y left the lattice");
                assert!((1..HCELLS - 1).contains(&g.pz), "z left the lattice");
            }
        }
    }

    /// Colours are shared out before any of them comes round again, so no two
    /// pipes on the screen are the same colour until all seven have been used.
    #[test]
    fn colours_are_shared_out_before_they_repeat() {
        let mut g = grower(20260812);
        let mut seen = vec![g.system_color];
        for _ in 0..6 {
            g.pinit(false);
            seen.push(g.system_color);
        }
        seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
        seen.dedup();
        assert_eq!(seen.len(), DEFINEDCOLORS, "a colour came round early");
    }

    /// The pipe grows: one more step of it is on screen every frame.
    #[test]
    fn the_system_grows_a_step_a_frame() {
        let mut r = start(StartArgs::new(640, 480, "size=40&cycles=1", 20260812));
        let mut last = 0;
        let mut grew = 0;
        for _ in 0..40 {
            r.step();
            let n = r.frame().vertices.len();
            if n > last {
                grew += 1;
            }
            last = n;
        }
        assert!(grew > 30, "the pipe only grew on {grew} of forty frames");
    }

    /// Everything the saver draws merges into a handful of batches. The whole
    /// point of baking the system flat is that a frame is a few draw calls and
    /// not one per step: what is left is one batch per colour change, which is
    /// what upstream has too, so turning the gadgetry up raises it.
    #[test]
    fn the_whole_system_draws_in_a_few_batches() {
        let r = run("size=200&cycles=3", 400);
        let f = r.frame();
        assert!(!f.vertices.is_empty());
        assert!(
            f.batches.len() < 200,
            "{} batches for one frame of pipes",
            f.batches.len()
        );
    }

    /// A finished system spins away to nothing and a new one starts, so the
    /// saver never simply stops.
    #[test]
    fn a_finished_system_is_replaced() {
        let mut r = start(StartArgs::new(640, 480, "size=10&cycles=1", 20260812));
        let mut thrown_away = 0;
        let mut last = 0;
        for _ in 0..300 {
            r.step();
            let n = r.frame().vertices.len();
            if last > 0 && n == 0 {
                thrown_away += 1;
            }
            last = n;
        }
        assert!(thrown_away >= 1, "the system was never thrown away");
    }

    /// Wireframe draws lines and no triangles, and the fill draws triangles
    /// and no lines.
    #[test]
    fn wireframe_draws_lines_only() {
        let r = run("wireframe=true&size=60&cycles=1", 60);
        let f = r.frame();
        assert!(!f.batches.is_empty());
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == Primitive::Lines || b.count == 0),
            "wireframe drew something solid"
        );

        let r = run("size=60&cycles=1", 60);
        let f = r.frame();
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == Primitive::Triangles || b.count == 0),
            "the solid pipe drew something flat"
        );
    }

    /// The gadgetry knob is the chance in a hundred of a fitting rather than a
    /// pipe, so turning it up puts more geometry on the screen.
    #[test]
    fn gadgetry_adds_fittings() {
        let plain = run("factory=0&size=300&cycles=1", 320)
            .frame()
            .vertices
            .len();
        let busy = run("factory=10&size=300&cycles=1", 320)
            .frame()
            .vertices
            .len();
        assert!(
            busy > plain * 5 / 4,
            "gadgetry at ten drew {busy} against {plain} at nought"
        );
    }

    /// Ball joints and curved pipes really are different shapes, so the corner
    /// style is not just a name.
    #[test]
    fn the_style_knob_changes_the_corners() {
        let balls = run("count=1&size=200&cycles=1&factory=0", 210)
            .frame()
            .vertices
            .len();
        let curves = run("count=3&size=200&cycles=1&factory=0", 210)
            .frame()
            .vertices
            .len();
        assert_ne!(balls, curves);
    }

    /// How much geometry a whole default system comes to, which is what decides
    /// whether the saver can be drawn this way at all. It is drawn a step at a
    /// time, so this is the peak, reached for one frame before the fadeout.
    #[test]
    fn a_default_system_fits_in_the_frame_budget() {
        ya_rand_init(20260812);
        let models = Models::load();
        let mut worst = 0;
        for _ in 0..3 {
            let s = generate_system(&default_config(), &models);
            assert!(s.lines.is_empty(), "the solid pipe baked lines");
            worst = worst.max(s.tris.len());
        }
        // Upstream's own five hundred comes to 1.95 million, which is why the
        // default here is a hundred and fifty; see `DEFAULTS`.
        assert!(worst < 600_000, "a default system came to {worst} vertices");
    }
}
