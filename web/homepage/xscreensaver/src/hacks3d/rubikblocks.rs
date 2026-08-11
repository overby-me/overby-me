//! Port of `hacks/glx/rubikblocks.c`.
//!
//! ```text
//! rubikblocks, Copyright (c) 2009 Vasek Potocek <vasek.potocek@post.cz>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! RubikBlocks - a Rubik's Mirror Blocks puzzle introduced in 2008.
//! No mirrors in this version, though, hence the altered name.
//! ```
//!
//! The Rubik's Mirror Blocks puzzle: a three by three cube whose twenty-seven
//! pieces are all different heights, so turning a layer leaves it a lopsided
//! heap rather than a cube again. Solving it means making it a cube.
//!
//! Every piece carries a quaternion of its own accumulated rotation and
//! nothing else. To turn a layer, the code asks each piece where its original
//! position has been carried to, by conjugating that position by the piece's
//! quaternion, and flags the ones that came out on the layer being turned.
//! Then a small quaternion is multiplied into every flagged piece once a frame
//! until the turn is done. So there is no board and no bookkeeping about which
//! piece is where: the state of the puzzle is twenty-seven quaternions.
//!
//! That accumulates error, which is why `settle_value` exists: at the end of a
//! turn every quaternion component is snapped to the nearest of zero, a half,
//! one over root two and one, which are the only values a component can hold
//! for a rotation by whole right angles.
//!
//! The pieces are not cubes. Each of the three coordinates is passed through a
//! function that pulls the outer faces in by a different amount, so the block
//! is subtly oblong, and that is the whole of why a shuffled one looks so
//! wrong.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent,
    random,
};

const SHUFFLE: usize = 100;
const TEX_WIDTH: i32 = 64;
const TEX_HEIGHT: i32 = 64;
/// How many pixels the dark edge of a face's outline runs over.
const BORDER: i32 = 5;
const BORDER2: i32 = BORDER * BORDER;

/// One over root two: the only value a quaternion component can hold for a
/// rotation by an odd number of right angles.
const SQRT1_2: f32 = std::f32::consts::FRAC_1_SQRT_2;

fn rnd01() -> i32 {
    (random() % 2) as i32
}

#[derive(Clone, Copy)]
struct Piece {
    /// Its _original_ position, which never changes.
    pos: [f32; 3],
    /// The quaternion of everything that has happened to it since.
    qr: [f32; 4],
    /// Whether it is part of the turn going on now.
    act: bool,
}

/// Multiplies two quaternions, `src * dest`, and stores the result in `dest`.
fn mult_quat(src: [f32; 4], dest: &mut [f32; 4]) {
    let r = src[0] * dest[0] - src[1] * dest[1] - src[2] * dest[2] - src[3] * dest[3];
    let i = src[0] * dest[1] + src[1] * dest[0] + src[2] * dest[3] - src[3] * dest[2];
    let j = src[0] * dest[2] + src[2] * dest[0] + src[3] * dest[1] - src[1] * dest[3];
    let k = src[0] * dest[3] + src[3] * dest[0] + src[1] * dest[2] - src[2] * dest[1];
    *dest = [r, i, j, k];
}

/// Sets the `act` flag for pieces which will undergo the rotation.
///
/// Where a piece is *now* is its original position conjugated by its own
/// quaternion, which is the one place the puzzle's state is ever read back.
fn flag_pieces(pieces: &mut [Piece; 27], axis: usize, side: f32) {
    for p in pieces.iter_mut() {
        let mut q = [0.0, p.pos[0], p.pos[1], p.pos[2]];
        mult_quat(p.qr, &mut q);
        for v in &mut q[1..4] {
            *v = -*v;
        }
        mult_quat(p.qr, &mut q);
        for v in &mut q[1..4] {
            *v = -*v;
        }
        p.act = (q[axis] - side).abs() < 0.1;
    }
}

/// "Rounds" the value to the nearest from the set `{0, +-1/2, +-1/sqrt(2),
/// +-1}`. It is guaranteed to be pretty close to one when this function is
/// called.
fn settle_value(v: f32) -> f32 {
    if v > 0.9 {
        1.0
    } else if v < -0.9 {
        -1.0
    } else if v > 0.6 {
        SQRT1_2
    } else if v < -0.6 {
        -SQRT1_2
    } else if v > 0.4 {
        0.5
    } else if v < -0.4 {
        -0.5
    } else {
        0.0
    }
}

/// These simple transforms make the actual shape of the pieces. The
/// parameters A, B and C affect the eccentricity of the pieces in each
/// direction.
fn fx(x: f32) -> f32 {
    const A: f32 = 0.5;
    if x > 1.4 {
        1.5 - A
    } else if x < -1.4 {
        -1.5 - A
    } else {
        x
    }
}

fn fy(y: f32) -> f32 {
    const B: f32 = 0.25;
    if y > 1.4 {
        1.5 - B
    } else if y < -1.4 {
        -1.5 - B
    } else {
        y
    }
}

fn fz(z: f32) -> f32 {
    const C: f32 = 0.0;
    if z > 1.4 {
        1.5 - C
    } else if z < -1.4 {
        -1.5 - C
    } else {
        z
    }
}

struct RubikBlocks {
    rot: Rotator,
    trackball: Trackball,
    /// Between two rotations rather than in one.
    pause: bool,
    /// The quaternion of one frame's worth of the turn under way.
    qfram: [f32; 4],
    /// The clock for the turn, and what it is counting to.
    t: f32,
    tmax: f32,
    pieces: [Piece; 27],
    /// One display list per piece, since a piece's shape never changes.
    lists: u32,
    /// Which axis the last turn was about, so the next one picks another.
    axis: usize,

    tspeed: f32,
    twait: f32,
    size: f32,
}

impl RubikBlocks {
    fn randomize(&mut self) {
        for _ in 0..SHUFFLE {
            let axis = (random() % 3 + 1) as usize;
            let side = (rnd01() * 2 - 1) as f32;
            flag_pieces(&mut self.pieces, axis, side);
            self.qfram = [SQRT1_2, 0.0, 0.0, 0.0];
            self.qfram[axis] = SQRT1_2;
            for p in &mut self.pieces {
                if p.act {
                    mult_quat(self.qfram, &mut p.qr);
                }
            }
        }
    }

    /// End the turn or the pause, and set up whichever comes next.
    fn finish(&mut self) {
        if self.pause {
            // Never the same axis twice running, which is what keeps it from
            // undoing itself.
            self.axis = match self.axis {
                1 => (rnd01() + 2) as usize,
                2 => (2 * rnd01() + 1) as usize,
                _ => (rnd01() + 1) as usize,
            };
            let side = (rnd01() * 2 - 1) as f32;
            let angle = rnd01() + 1;
            flag_pieces(&mut self.pieces, self.axis, side);
            self.pause = false;
            self.tmax = 90.0 * angle as f32;
            let pi = std::f32::consts::PI;
            self.qfram = [(self.tspeed * pi / 360.0).cos(), 0.0, 0.0, 0.0];
            self.qfram[self.axis] = ((rnd01() * 2 - 1) as f32 * self.tspeed * pi / 360.0).sin();
        } else {
            // Snap every component back onto the values a whole-right-angle
            // rotation can have, or the drift would show after a few hundred
            // turns.
            for p in &mut self.pieces {
                for v in &mut p.qr {
                    *v = settle_value(*v);
                }
            }
            self.pause = true;
            self.tmax = self.twait;
        }
        self.t = 0.0;
    }
}

/// The face texture: white with a dark border, which is what draws the seams
/// between the blocks.
fn make_texture(g: &mut Gl) -> u32 {
    let mut tex = vec![255u8; (TEX_WIDTH * TEX_HEIGHT) as usize];
    let at = |x: i32, y: i32| (y * TEX_WIDTH + x) as usize;

    let horz_line = |tex: &mut Vec<u8>, x1: i32, x2: i32, y0: i32| {
        let mut y = if y0 < BORDER { -y0 } else { -BORDER };
        while y < BORDER {
            if y0 + y >= TEX_HEIGHT {
                break;
            }
            let w = (y * y * 255 / BORDER2) as u8;
            for x in x1..=x2 {
                let i = at(x, y0 + y);
                if tex[i] > w {
                    tex[i] = w;
                }
            }
            y += 1;
        }
    };
    horz_line(&mut tex, 0, TEX_WIDTH - 1, 0);
    horz_line(&mut tex, 0, TEX_WIDTH - 1, TEX_HEIGHT - 1);

    let vert_line = |tex: &mut Vec<u8>, x0: i32, y1: i32, y2: i32| {
        let mut x = if x0 < BORDER { -x0 } else { -BORDER };
        while x < BORDER {
            if x0 + x >= TEX_WIDTH {
                break;
            }
            let w = (x * x * 255 / BORDER2) as u8;
            for y in y1..=y2 {
                let i = at(x0 + x, y);
                if tex[i] > w {
                    tex[i] = w;
                }
            }
            x += 1;
        }
    };
    vert_line(&mut tex, 0, 0, TEX_HEIGHT - 1);
    vert_line(&mut tex, TEX_WIDTH - 1, 0, TEX_HEIGHT - 1);

    // Upstream's is GL_LUMINANCE, one byte a pixel, which the fixed pipeline
    // reads as that value in all three colour channels and an opaque alpha.
    let data: Vec<u8> = tex.iter().flat_map(|&l| [l, l, l, 255]).collect();
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d_clamped(TEX_WIDTH, TEX_HEIGHT, data);
    id
}

/// One piece, as a strip round its four sides and a quad for the top and the
/// bottom.
fn build_piece(g: &mut Gl, pos: [f32; 3]) {
    let [x, y, z] = pos;
    let v = |g: &mut Gl, u: f32, t: f32, px: f32, py: f32, pz: f32| {
        g.glx.tex_coord2f(u, t);
        g.glx.vertex3f(fx(px), fy(py), fz(pz));
    };

    g.glx.begin(Shape::QuadStrip);
    g.glx.normal3f(1.0, 0.0, 0.0);
    v(g, 0.0, 0.0, x + 0.5, y - 0.5, z - 0.5);
    v(g, 0.0, 1.0, x + 0.5, y + 0.5, z - 0.5);
    v(g, 1.0, 0.0, x + 0.5, y - 0.5, z + 0.5);
    v(g, 1.0, 1.0, x + 0.5, y + 0.5, z + 0.5);
    g.glx.normal3f(0.0, 0.0, 1.0);
    v(g, 0.0, 0.0, x - 0.5, y - 0.5, z + 0.5);
    v(g, 0.0, 1.0, x - 0.5, y + 0.5, z + 0.5);
    g.glx.normal3f(-1.0, 0.0, 0.0);
    v(g, 1.0, 0.0, x - 0.5, y - 0.5, z - 0.5);
    v(g, 1.0, 1.0, x - 0.5, y + 0.5, z - 0.5);
    g.glx.normal3f(0.0, 0.0, -1.0);
    v(g, 0.0, 0.0, x + 0.5, y - 0.5, z - 0.5);
    v(g, 0.0, 1.0, x + 0.5, y + 0.5, z - 0.5);
    g.glx.end();

    g.glx.begin(Shape::Quads);
    g.glx.normal3f(0.0, 1.0, 0.0);
    v(g, 0.0, 0.0, x + 0.5, y + 0.5, z + 0.5);
    v(g, 0.0, 1.0, x + 0.5, y + 0.5, z - 0.5);
    v(g, 1.0, 1.0, x - 0.5, y + 0.5, z - 0.5);
    v(g, 1.0, 0.0, x - 0.5, y + 0.5, z + 0.5);
    g.glx.normal3f(0.0, -1.0, 0.0);
    v(g, 0.0, 0.0, x + 0.5, y - 0.5, z - 0.5);
    v(g, 0.0, 1.0, x + 0.5, y - 0.5, z + 0.5);
    v(g, 1.0, 1.0, x - 0.5, y - 0.5, z + 0.5);
    v(g, 1.0, 0.0, x - 0.5, y - 0.5, z - 0.5);
    g.glx.end();
}

impl Hack3d for RubikBlocks {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.load_identity();

        let down = self.trackball.button_down();
        let (x, y, _) = self.rot.position(!down);
        g.glx
            .translate((x as f32 - 0.5) * 6.0, (y as f32 - 0.5) * 6.0, -20.0);

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        g.glx.scale(self.size, self.size, self.size);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        if !self.pause {
            let q = self.qfram;
            for p in &mut self.pieces {
                if p.act {
                    mult_quat(q, &mut p.qr);
                }
            }
        }

        let deg = 360.0 / std::f32::consts::PI;
        for (i, p) in self.pieces.iter().enumerate() {
            g.glx.push_matrix();
            if p.qr[0].abs() < 1.0 {
                g.glx
                    .rotate(deg * p.qr[0].acos(), p.qr[1], p.qr[2], p.qr[3]);
            }
            g.glx.call_list(self.lists + i as u32);
            g.glx.pop_matrix();
        }

        self.t += self.tspeed;
        if self.t > self.tmax {
            self.finish();
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
        }
        let height = height.max(1);
        let ratio = width as f32 / height as f32;

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, ratio, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // Upstream turns wireframe off under GLES, where a line polygon mode does
    // not exist; the same applies here, so there is no wireframe knob.
    let tex = g.res.bool("texture");
    let texid = if tex { make_texture(g) } else { 0 };

    let spin = g.res.bool("spin");
    let spinspeed = g.res.float("spinspeed");
    let wspeed = g.res.float("wanderspeed");

    let mut pieces = [Piece {
        pos: [0.0; 3],
        qr: [1.0, 0.0, 0.0, 0.0],
        act: false,
    }; 27];
    let mut m = 0;
    for i in -1..=1 {
        for j in -1..=1 {
            for k in -1..=1 {
                pieces[m].pos = [k as f32, j as f32, i as f32];
                m += 1;
            }
        }
    }

    let mut st = RubikBlocks {
        rot: Rotator::new(
            if spin { spinspeed } else { 0.0 },
            if spin { spinspeed } else { 0.0 },
            if spin { spinspeed } else { 0.0 },
            0.1,
            if g.res.bool("wander") { wspeed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        pause: true,
        qfram: [1.0, 0.0, 0.0, 0.0],
        t: 0.0,
        tmax: g.res.float("wait") as f32,
        pieces,
        lists: 0,
        // Upstream's is a function-local static starting at one.
        axis: 1,
        tspeed: g.res.float("rotspeed") as f32,
        twait: g.res.float("wait") as f32,
        size: g.res.float("cubesize") as f32,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if tex {
        g.glx.texturing(true);
        g.glx.bind_texture(texid);
    }
    // Two lights from opposite corners, an ambient of a tenth rather than
    // OpenGL's fifth, and a material that tracks the current colour, which is
    // left white throughout.
    g.glx.lighting(true);
    g.glx.light_model_ambient([0.1, 0.1, 0.1, 1.0]);
    for (i, pos) in [[1.0, 1.0, 1.0, 0.0], [-1.0, -1.0, 1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        g.glx.light_enable(i, true);
        g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
        g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
        // Upstream sets no specular, so each light keeps OpenGL's default:
        // white for the first and black for every other.
        let spec = if i == 0 { 1.0 } else { 0.0 };
        g.glx.light_specular(i, [spec, spec, spec, 1.0]);
    }
    g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
    g.glx.material_specular([0.2, 0.2, 0.2, 1.0]);
    g.glx.material_shininess(20.0);

    if g.res.bool("randomize") {
        st.randomize();
    }

    st.lists = g.glx.gen_lists(27);
    for i in 0..27 {
        g.glx.new_list(st.lists + i as u32);
        build_piece(g, st.pieces[i].pos);
        g.glx.end_list();
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*texture:      True",
    "*randomize:    False",
    "*spinspeed:    0.1",
    "*rotspeed:     3.0",
    "*wanderspeed:  0.005",
    "*wait:         40.0",
    "*cubesize:     1.0",
];

const STARTS: &[SelectItem] = &[
    SelectItem {
        value: "false",
        label: "Start as cube",
    },
    SelectItem {
        value: "true",
        label: "Start as random shape",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("cubesize", "Cube size", 0.4, 2.0, 0.05, 2, "1.0"),
    Opt::slider("rotspeed", "Rotation", 1.0, 10.0, 0.1, 1, "3.0"),
    Opt::slider("spinspeed", "Spin", 0.01, 4.0, 0.01, 2, "0.1"),
    Opt::slider("wanderspeed", "Wander", 0.001, 0.1, 0.001, 3, "0.005"),
    Opt::slider("wait", "Linger", 10.0, 100.0, 1.0, 0, "40.0"),
    Opt::select("randomize", "Start", STARTS, "false"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("texture", "Outlines", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rubikblocks",
    label: "Rubik Blocks",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Vasek Potocek",
        year: "2009",
        video: Some("https://www.youtube.com/watch?v=B2sGaRLWz-A"),
        blurb: "The Rubik's Mirror Blocks puzzle, shuffling itself.",
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

    /// Quaternion multiplication is the whole of the puzzle's state, so it had
    /// better be quaternion multiplication.
    #[test]
    fn quaternions_multiply() {
        // A quarter turn about x, twice, is a half turn about x.
        let q = [SQRT1_2, SQRT1_2, 0.0, 0.0];
        let mut d = q;
        mult_quat(q, &mut d);
        for (a, b) in d.iter().zip(&[0.0, 1.0, 0.0, 0.0]) {
            assert!((a - b).abs() < 1e-6, "{d:?}");
        }
        // And the identity leaves anything alone.
        let mut d = [0.3, 0.4, 0.5, 0.7];
        mult_quat([1.0, 0.0, 0.0, 0.0], &mut d);
        assert_eq!(d, [0.3, 0.4, 0.5, 0.7]);
    }

    /// A turn moves exactly the nine pieces of one layer, which is what makes
    /// it a Rubik's cube rather than a pile of blocks.
    #[test]
    fn a_turn_takes_one_layer() {
        let mut pieces = [Piece {
            pos: [0.0; 3],
            qr: [1.0, 0.0, 0.0, 0.0],
            act: false,
        }; 27];
        let mut m = 0;
        for i in -1..=1 {
            for j in -1..=1 {
                for k in -1..=1 {
                    pieces[m].pos = [k as f32, j as f32, i as f32];
                    m += 1;
                }
            }
        }
        for axis in 1..4 {
            for side in [-1.0, 1.0] {
                flag_pieces(&mut pieces, axis, side);
                let n = pieces.iter().filter(|p| p.act).count();
                assert_eq!(n, 9, "axis {axis} side {side} took {n} pieces");
            }
        }
    }

    /// Settling rounds a drifted quaternion back onto the values a rotation by
    /// whole right angles can hold, which is what stops a few hundred turns
    /// from smearing the puzzle.
    #[test]
    fn settling_snaps_to_the_right_angles() {
        assert_eq!(settle_value(0.9999), 1.0);
        assert_eq!(settle_value(-0.9999), -1.0);
        assert_eq!(settle_value(0.7072), SQRT1_2);
        assert_eq!(settle_value(0.4999), 0.5);
        assert_eq!(settle_value(0.0001), 0.0);
        assert_eq!(settle_value(-0.7072), -SQRT1_2);
    }

    /// The blocks are not cubes: each axis pulls its outer face in by its own
    /// amount, which is the whole reason a shuffled one looks wrong.
    #[test]
    fn the_blocks_are_lopsided() {
        // The outer faces at +1.5 come in by 0.5, 0.25 and nothing.
        assert_eq!(fx(1.5), 1.0);
        assert_eq!(fy(1.5), 1.25);
        assert_eq!(fz(1.5), 1.5);
        // And the same shift is applied at the far side, which is what makes
        // the piece lopsided rather than merely smaller.
        assert_eq!(fx(-1.5), -2.0);
        assert_eq!(fy(-1.5), -1.75);
        assert_eq!(fz(-1.5), -1.5);
        // The middle is untouched.
        assert_eq!(fx(0.5), 0.5);
    }

    /// It turns, it stops, it turns again, and every piece stays somewhere a
    /// piece of a three by three cube could be.
    #[test]
    fn it_shuffles_itself() {
        let mut r = start(StartArgs::new(640, 480, "wait=10&rotspeed=10", 20260811));
        let mut angles = std::collections::BTreeSet::new();
        for _ in 0..400 {
            r.step();
            let f = r.frame();
            // Pieces that share a rotation share a batch, and at rest they all
            // do, so the count says nothing; that everything is drawn does.
            // Four quads round the sides and two on the ends, cut into
            // triangles: thirty-six vertices a piece.
            assert_eq!(f.vertices.len(), 27 * 36);
            for v in &f.vertices {
                let r = (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt();
                assert!(r < 4.0, "a corner {r} from the middle");
            }
            // The first piece's matrix, rounded, as a fingerprint of the turn.
            let m = f.batches[0].modelview.0;
            angles.insert((m[0] * 100.0) as i32);
        }
        assert!(angles.len() > 20, "it never turned");
    }
}
