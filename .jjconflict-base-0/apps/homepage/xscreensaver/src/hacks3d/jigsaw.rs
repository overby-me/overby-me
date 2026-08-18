//! Port of `hacks/glx/jigsaw.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997-2019 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Written as an Xlib program some time in 1997.
//! Rewritten as an OpenGL program 24-Aug-2008.
//! ```
//!
//! Carves an image up into a jigsaw puzzle, shuffles it, and solves it.
//!
//! The pieces fly in from off screen and land in a shuffled grid, rotated as
//! well as moved. Then pairs of them are swapped, one pair at a time and both
//! arcing through the air on different heights so they do not pass through each
//! other, until every piece is home. Then the whole thing is thrown back off
//! screen and the next picture is loaded.
//!
//! A piece's shape is one spline drawn four times: the tab is the same curve
//! every time, pointing in or out or flattened away at the border, and the
//! piece to the right gets the mirror of whatever this one's right edge did.
//! Filling that outline is the interesting part, since it is not convex.
//! Upstream tessellates it with GLU where it can, and where it cannot, which is
//! its own OpenGL ES build, it cuts the piece into eighths and triangulates
//! each by hand. This runtime has no tessellator either, so the port takes the
//! second path.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::shapes::calc_normal;
use crate::runtime::spline::Spline;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random, screenhack_event_helper,
};

const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// Which way a tab points: into the piece, out of it, or nowhere because this
/// edge is on the border of the puzzle.
const IN: i32 = -1;
const FLAT: i32 = 0;
const OUT: i32 = 1;

/// Three samples averaged, so the middle of the range comes up most often.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

/// A position and a rotation about z, in degrees.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Xyzr {
    x: f64,
    y: f64,
    z: f64,
    r: f64,
}

struct Piece {
    edge: [i32; 4],
    /// The triangles of the face, in piece coordinates.
    tris: Vec<[f32; 3]>,
    /// The outline, which the sides and the two drawn edges follow.
    outline: Vec<[f32; 3]>,

    /// Where it belongs in the finished puzzle.
    home: Xyzr,
    /// Where it is right now.
    current: Xyzr,
    /// The move it is making, and how far through it it is.
    from: Xyzr,
    to: Xyzr,
    tick: f64,
    /// How high it arcs on the way.
    arc_height: f64,
    tilt: f64,
    max_tilt: f64,
}

/// What the puzzle is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Waiting for the picture, with a spinning piece to look at.
    LoadingMsg,
    /// The picture is here; cut it up.
    Loading,
    /// The pieces are flying in from off screen.
    Unscatter,
    /// Swapping pairs until it comes out right.
    Solve,
    /// Throwing them all back off screen again.
    Scatter,
}

struct Jigsaw {
    trackball: Trackball,
    rot: Rotator,
    font: TexFont,

    puzzle_width: usize,
    puzzle_height: usize,
    puzzle: Vec<Piece>,

    state: State,
    pausing: f64,
    tick_speed: f64,

    texid: Option<u32>,
    aspect: f32,

    speed: f64,
    complexity: f64,
    resolution: i32,
    thickness: f32,
    wobble: bool,
    wire: bool,
}

/// `make_puzzle_curve`: one edge of a piece, as a spline through thirteen
/// control points. The tab is the bulge in the middle; the two ends are flat
/// so that neighbouring edges meet cleanly.
fn make_puzzle_curve(pixels: i32) -> Spline {
    let pts: [(f64, f64); 7] = [
        (0.0000, 0.0000),
        (0.3333, 0.1000),
        (0.4333, 0.0333),
        (0.4666, -0.0666),
        (0.3333, -0.1666),
        (0.3666, -0.2900),
        (0.5000, -0.3333),
    ];

    let mut s = Spline::new(13);
    let p = f64::from(pixels);
    for (i, (x, y)) in pts.iter().enumerate() {
        s.control_x[i] = p * x;
        s.control_y[i] = p * y;
    }
    // The second half is the first mirrored about the middle.
    for (i, (x, y)) in pts.iter().take(6).rev().enumerate() {
        s.control_x[7 + i] = p * (1.0 - x);
        s.control_y[7 + i] = p * y;
    }
    s.compute();
    s
}

/// `make_piece_eighth`: triangles for one eighth of a piece's face.
///
/// A piece is cut into eight wedges, two per edge, each running from the
/// middle of the piece out to half of one edge. A flat edge is one triangle; a
/// tab is a fan that changes its hub where the curve turns back on itself,
/// which is what `inflected` is watching for.
fn make_piece_eighth(
    s: &Spline,
    resolution: i32,
    kind: i32,
    out: &mut Vec<[f32; 3]>,
    flip_x: bool,
    flip_y: bool,
    rotate: bool,
) {
    let start = out.len();
    let r = resolution as f32;
    let mut cx = (resolution / 2) as f32;
    let mut cy = (resolution / 2) as f32;

    if kind == FLAT {
        out.push([cx, 0.0, 0.0]);
        out.push([cx, cy, 0.0]);
        out.push([0.0, 0.0, 0.0]);
    } else {
        let np = s.points.len() / 2 + 1;
        let mut last: Option<(f32, f32)> = None;
        let mut inflected = false;

        // A tab that points in is drawn from the far end back, with its curve
        // turned upside down.
        let order: Vec<usize> = if kind == IN {
            (0..np).rev().collect()
        } else {
            (0..np).collect()
        };

        for i in order {
            let x = s.points[i].x as f32;
            let y = if kind == IN {
                -s.points[i].y as f32
            } else {
                s.points[i].y as f32
            };

            if let Some((lx, ly)) = last {
                if !inflected && (if kind == IN { x >= lx } else { x < lx }) {
                    inflected = true;
                    out.push([cx, cy, 0.0]);
                    out.push([lx, ly, 0.0]);
                    if kind == IN {
                        cx = 0.0;
                        cy = 0.0;
                    } else {
                        cy = y;
                    }
                    out.push([cx, cy, 0.0]);
                }
                out.push([cx, cy, 0.0]);
                out.push([lx, ly, 0.0]);
                out.push([x, y, 0.0]);
            }
            last = Some((x, y));
        }
    }

    let tris = &mut out[start..];
    if flip_x {
        for v in tris.iter_mut() {
            v[0] = r - v[0];
        }
    }
    if flip_y {
        for v in tris.iter_mut() {
            v[1] = r - v[1];
        }
    }

    // Flipping reverses the winding, so put it back by swapping two corners
    // of every triangle.
    let mut cw = kind == IN;
    if flip_x {
        cw = !cw;
    }
    if flip_y {
        cw = !cw;
    }
    if cw {
        for t in tris.chunks_exact_mut(3) {
            t.swap(0, 1);
        }
    }

    if rotate {
        for v in tris.iter_mut() {
            let (x, y) = (v[0], v[1]);
            v[0] = r - y;
            v[1] = x;
        }
    }
}

impl Piece {
    /// `draw_piece`, up to the point where it would start drawing: the outline
    /// of one piece and the triangles that fill it.
    ///
    /// Upstream compiles this into a display list. Here the geometry is kept
    /// and the drawing done at call time, since a list in this runtime would
    /// not replay the winding and the material the piece sets as it goes.
    fn build(&mut self, resolution: i32, thickness: f32) {
        let s = make_puzzle_curve(resolution);
        let r = resolution as f64;
        let z = resolution as f32 * thickness;
        let n = s.points.len();

        // The outline, anticlockwise from the top left. Each edge is either
        // the whole spline or the single corner point that replaces it.
        let mut o: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
        let t = self.edge[TOP];
        if t == 0 {
            o.push([0.0, 0.0, z]);
            o.push([resolution as f32, 0.0, z]);
        } else {
            for p in &s.points {
                o.push([p.x as f32, (p.y * t) as f32, z]);
            }
        }

        let t = self.edge[RIGHT];
        if t == 0 {
            o.push([resolution as f32, resolution as f32, z]);
        } else {
            for p in &s.points[1..] {
                o.push([(r + f64::from(p.y * -t)) as f32, p.x as f32, z]);
            }
        }

        let t = self.edge[BOTTOM];
        if t == 0 {
            o.push([0.0, resolution as f32, z]);
        } else {
            for i in 1..n {
                let p = s.points[n - i - 1];
                o.push([p.x as f32, (r + f64::from(p.y * -t)) as f32, z]);
            }
        }

        let t = self.edge[LEFT];
        if t == 0 {
            o.push([0.0, 0.0, z]);
        } else {
            for i in 1..n {
                let p = s.points[n - i - 1];
                o.push([(p.y * t) as f32, p.x as f32, z]);
            }
        }
        self.outline = o;

        // Eight wedges: two for each edge, mirrored into place.
        let mut tris = Vec::new();
        for (kind, fx, fy, rot) in [
            (self.edge[TOP], false, false, false),
            (self.edge[TOP], true, false, false),
            (self.edge[LEFT], false, true, true),
            (self.edge[LEFT], true, true, true),
            (self.edge[BOTTOM], false, true, false),
            (self.edge[BOTTOM], true, true, false),
            (self.edge[RIGHT], false, false, true),
            (self.edge[RIGHT], true, false, true),
        ] {
            make_piece_eighth(&s, resolution, kind, &mut tris, fx, fy, rot);
        }
        self.tris = tris;
    }

    /// The piece itself: both faces, the wall round the outside, and the two
    /// outlines drawn over the top of it.
    fn draw(&self, g: &mut Gl, jc: &Jigsaw, resolution: i32) {
        let wire = jc.wire;
        let r = resolution as f32;
        let ss = 1.0 / r;
        g.glx.scale(ss, ss, ss);

        let z = self.outline.first().map_or(0.0, |p| p[2]);
        let pw = jc.puzzle_width as f32;
        let ph = jc.puzzle_height as f32;

        // The front face carries the picture; the back is bare.
        for front in [true, false] {
            let zz = if front { z } else { -z };
            g.glx.front_face_cw(!front);
            g.glx.normal3f(0.0, 0.0, if front { 1.0 } else { -1.0 });

            if !wire {
                if front && let Some(id) = jc.texid {
                    g.glx.texturing(true);
                    g.glx.bind_texture(id);
                    g.glx.blend(Blend::Alpha);
                    g.glx.lighting(true);
                } else {
                    g.glx.texturing(false);
                }
            }

            g.glx.push_matrix();
            g.glx.translate(0.0, 0.0, zz);

            if wire {
                for t in self.tris.chunks_exact(3) {
                    g.glx.begin(Shape::LineLoop);
                    for v in t {
                        g.glx.vertex3f(v[0], v[1], v[2]);
                    }
                    g.glx.end();
                }
            } else {
                g.glx.begin(Shape::Triangles);
                for v in &self.tris {
                    // Where this vertex is in the whole picture, not just in
                    // the piece.
                    let xx = v[0] / r;
                    let yy = v[1] / r;
                    let tx = (self.home.x as f32 + xx) / pw;
                    // Upstream measures from the bottom because OpenGL's
                    // textures start there; ours start at the top.
                    let ty = (self.home.y as f32 + yy) / ph;
                    g.glx.tex_coord2f(tx, ty);
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            }

            g.glx.pop_matrix();
        }

        // The wall round the outside, joining the two faces.
        g.glx.texturing(false);
        g.glx.front_face_cw(false);
        let o = self.outline.len();
        g.glx
            .begin(if wire { Shape::Lines } else { Shape::QuadStrip });
        for i in 0..o {
            let p = self.outline[i];
            let pj = self.outline[(i + o - 1) % o];
            let pk = self.outline[(i + 1) % o];
            let n = calc_normal(
                [pj[0], pj[1], pj[2]],
                [pj[0], pj[1], -pj[2]],
                [pk[0], pk[1], pk[2]],
            );
            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx.vertex3f(p[0], p[1], p[2]);
            g.glx.vertex3f(p[0], p[1], -p[2]);
        }
        g.glx.end();

        // Both outlines, drawn flat in grey over the top.
        if !wire {
            g.glx.color4f(0.3, 0.3, 0.3, 1.0);
        }
        g.glx.lighting(false);
        g.glx.color_material(true);
        g.glx.line_width(jc.line_thickness());
        for sign in [1.0f32, -1.0] {
            g.glx.begin(Shape::LineLoop);
            for p in &self.outline {
                g.glx.vertex3f(p[0], p[1], sign * p[2]);
            }
            g.glx.end();
        }
        g.glx.color_material(false);
    }
}

/// Whether the two pieces are the same shape, with the second turned by the
/// given number of degrees.
fn same_shape(a: &Piece, b: &Piece, rotated_by: i32) -> bool {
    let k = match rotated_by {
        0 => 0,
        90 => 1,
        180 => 2,
        _ => 3,
    };
    (0..4).all(|i| a.edge[i] == b.edge[(i + k) % 4])
}

impl Jigsaw {
    fn line_thickness(&self) -> f32 {
        if self.wire { 1.0 } else { 2.0 }
    }

    /// `make_puzzle_grid`: how many pieces, which way each tab points, and the
    /// geometry of every one of them.
    fn make_puzzle_grid(&mut self, width: i32, height: i32) {
        let size = (8.0 + (random() % 8) as f64) * self.complexity;

        if self.wire {
            self.aspect = width as f32 / height.max(1) as f32;
        }

        if self.aspect >= 1.0 {
            self.puzzle_width = size as usize;
            self.puzzle_height = ((size + 0.5) / f64::from(self.aspect)) as usize;
        } else {
            self.puzzle_width = ((size + 0.5) * f64::from(self.aspect)) as usize;
            self.puzzle_height = size as usize;
        }
        self.puzzle_width = self.puzzle_width.max(1);
        self.puzzle_height = self.puzzle_height.max(1);

        let (w, h) = (self.puzzle_width, self.puzzle_height);
        // One row spare, so that the loop below can write the top edge of the
        // piece below the bottom row without running off the end.
        let mut puzzle: Vec<Piece> = (0..w * (h + 1))
            .map(|_| Piece {
                edge: [0; 4],
                tris: Vec::new(),
                outline: Vec::new(),
                home: Xyzr::default(),
                current: Xyzr::default(),
                from: Xyzr::default(),
                to: Xyzr::default(),
                tick: 0.0,
                arc_height: 0.0,
                tilt: 0.0,
                max_tilt: 0.0,
            })
            .collect();

        // Choose the right and bottom edge of each piece, and give the piece
        // on the other side of each the matching half.
        for y in 0..h {
            for x in 0..w {
                let right = if random() & 1 == 1 { IN } else { OUT };
                let bottom = if random() & 1 == 1 { IN } else { OUT };
                puzzle[y * w + x].edge[RIGHT] = right;
                puzzle[y * w + x].edge[BOTTOM] = bottom;
                if x + 1 < w {
                    puzzle[y * w + x + 1].edge[LEFT] = if right == IN { OUT } else { IN };
                }
                puzzle[(y + 1) * w + x].edge[TOP] = if bottom == IN { OUT } else { IN };
            }
        }

        puzzle.truncate(w * h);
        for y in 0..h {
            for x in 0..w {
                let p = &mut puzzle[y * w + x];
                p.home = Xyzr {
                    x: x as f64,
                    y: y as f64,
                    z: 0.0,
                    r: 0.0,
                };
                p.current = p.home;

                // The outside of the puzzle is a straight edge.
                if x == 0 {
                    p.edge[LEFT] = FLAT;
                }
                if y == 0 {
                    p.edge[TOP] = FLAT;
                }
                if x == w - 1 {
                    p.edge[RIGHT] = FLAT;
                }
                if y == h - 1 {
                    p.edge[BOTTOM] = FLAT;
                }

                p.build(self.resolution, self.thickness);
            }
        }
        self.puzzle = puzzle;
    }

    /// `proper_rotation`: how far this piece has to be turned to fit the hole
    /// at the given place.
    fn proper_rotation(&self, p: usize, x: f64, y: f64) -> f64 {
        let i = y as usize * self.puzzle_width + x as usize;
        for r in [0, 90, 180, 270] {
            if same_shape(&self.puzzle[p], &self.puzzle[i], r) {
                return f64::from(r);
            }
        }
        // Upstream aborts here; two pieces that fit nowhere cannot happen,
        // since a piece always fits its own hole.
        0.0
    }

    /// `piece_at`: which piece is sitting at the given place.
    fn piece_at(&self, x: f64, y: f64) -> Option<usize> {
        self.puzzle
            .iter()
            .position(|p| p.current.x as i32 == x as i32 && p.current.y as i32 == y as i32)
    }

    /// `shuffle_grid`: swap each piece with any other of the same shape, in
    /// whatever rotation makes it fit.
    fn shuffle_grid(&mut self) {
        let n = self.puzzle.len();
        for i in 0..n {
            let mut found = None;
            for _ in 0..n {
                let j = (random() as usize) % n;
                if [0, 90, 180, 270]
                    .iter()
                    .any(|&r| same_shape(&self.puzzle[i], &self.puzzle[j], r))
                {
                    found = Some(j);
                    break;
                }
            }
            if let Some(j) = found
                && i != j
            {
                let s = self.puzzle[i].current;
                self.puzzle[i].current = self.puzzle[j].current;
                self.puzzle[j].current = s;
                let (xi, yi) = (self.puzzle[i].current.x, self.puzzle[i].current.y);
                let (xj, yj) = (self.puzzle[j].current.x, self.puzzle[j].current.y);
                self.puzzle[i].current.r = self.proper_rotation(i, xi, yi);
                self.puzzle[j].current.r = self.proper_rotation(j, xj, yj);
            }
        }
    }

    /// `smooth_grid`: the arithmetic drifts, so put every value that ought to
    /// be a whole number back onto one.
    fn smooth_grid(&mut self) {
        for p in &mut self.puzzle {
            for v in [&mut p.home, &mut p.current, &mut p.from, &mut p.to] {
                v.x = (v.x + 0.5) as i32 as f64;
                v.y = (v.y + 0.5) as i32 as f64;
                v.z = (v.z + 0.5) as i32 as f64;
                v.r = (v.r + 0.5) as i32 as f64;
            }
            if p.tick <= 0.0001 {
                p.tick = 0.0;
            }
            if p.tick >= 0.9999 {
                p.tick = 1.0;
            }
        }
    }

    /// `begin_scatter`: throw every piece straight out from the middle, far
    /// enough to leave the frame. Reversed, this is how they arrive.
    fn begin_scatter(&mut self, unscatter: bool) {
        let ctr = (
            self.puzzle_width as f64 / 2.0,
            self.puzzle_height as f64 / 2.0,
        );
        let d = self.puzzle_width.max(self.puzzle_height) as f64 * 2.0;

        for p in &mut self.puzzle {
            p.tick = -frand(1.0);
            p.from = p.current;

            let ax = p.from.x - ctr.0;
            let ay = p.from.y - ctr.1;
            let r = (ax * ax + ay * ay).sqrt();
            let th = ax.atan2(ay);
            let r = r * r + d;

            p.to.x = ctr.0 + r * th.sin();
            p.to.y = ctr.1 + r * th.cos();
            p.to.z = p.from.z;
            p.to.r = f64::from((p.from.r as i32 + (random() % 180) as i32) % 360);
            p.arc_height = frand(10.0);

            if unscatter {
                std::mem::swap(&mut p.to, &mut p.from);
                p.current = p.from;
            }
        }
    }

    fn solved(&self) -> bool {
        self.puzzle
            .iter()
            .all(|p| p.current.x == p.home.x && p.current.y == p.home.y && p.current.z == p.home.z)
    }

    /// `move_one_piece`: pick a piece that is not home and swap it with
    /// whatever is sitting in its place.
    fn move_one_piece(&mut self) {
        let n = self.puzzle.len();
        for _ in 0..n * 100 {
            let i = (random() as usize) % n;
            let p0 = &self.puzzle[i];
            if p0.current.x == p0.home.x && p0.current.y == p0.home.y && p0.current.z == p0.home.z {
                continue; /* piece already solved - try again */
            }

            let (hx, hy) = (p0.home.x, p0.home.y);
            let Some(j) = self.piece_at(hx, hy) else {
                continue;
            };
            if i == j {
                continue;
            }

            let (c0, c1) = (self.puzzle[i].current, self.puzzle[j].current);
            self.puzzle[i].tick = 0.0;
            self.puzzle[i].from = c0;
            self.puzzle[i].to = c1;
            self.puzzle[j].tick = 0.0;
            self.puzzle[j].from = c1;
            self.puzzle[j].to = c0;
            self.puzzle[i].to.r = self.proper_rotation(i, c1.x, c1.y);
            self.puzzle[j].to.r = self.proper_rotation(j, c0.x, c0.y);

            // Give the two different heights, so they do not fly through each
            // other on the way.
            let (mut a0, mut a1) = (0.0f64, 0.0f64);
            while (a0 - a1).abs() < 1.5 {
                a0 = 0.5 + frand(3.0);
                a1 = 1.0 + frand(3.0);
            }
            self.puzzle[i].arc_height = a0;
            self.puzzle[j].arc_height = a1;

            let rtilt = || {
                let mut v = 90.0 - bellrand(180.0);
                for _ in 0..3 {
                    if random().is_multiple_of(5) {
                        v *= 2.0;
                    }
                }
                v
            };
            self.puzzle[i].max_tilt = rtilt();
            self.puzzle[j].max_tilt = rtilt();
            return;
        }
    }

    /// `anim_tick`: move everything on one step. True when nothing is still
    /// moving.
    fn anim_tick(&mut self) -> bool {
        if self.pausing > 0.0 {
            self.pausing -= self.tick_speed * self.speed;
            return false;
        }

        let mut finished = true;
        for p in &mut self.puzzle {
            if p.tick >= 1.0 {
                continue; /* this piece is done */
            }
            finished = false;

            p.tick = (p.tick + self.tick_speed * self.speed).min(1.0);
            if p.tick < 0.0 {
                continue; /* not yet started */
            }

            let pi = std::f64::consts::PI;
            let tt = 1.0 - (pi / 2.0 - p.tick * pi / 2.0).sin();
            p.current.x = p.from.x + (p.to.x - p.from.x) * tt;
            p.current.y = p.from.y + (p.to.y - p.from.y) * tt;
            p.current.z = p.from.z + (p.to.z - p.from.z) * tt;
            p.current.r = p.from.r + (p.to.r - p.from.r) * tt;

            p.current.z += p.arc_height * (p.tick * pi).sin();
            p.tilt = p.max_tilt * (p.tick * pi).sin();
        }
        finished
    }

    /// `animate`: the state machine that runs the whole thing.
    fn animate(&mut self, g: &mut Gl) {
        let slow = 0.01;
        let fast = 0.04;

        if self.trackball.button_down() && self.state != State::LoadingMsg {
            return;
        }

        match self.state {
            State::LoadingMsg | State::Loading => {
                if self.puzzle.is_empty() {
                    return; /* still loading */
                }
                self.tick_speed = slow;
                self.shuffle_grid();
                self.smooth_grid();
                self.begin_scatter(true);
                self.pausing = 0.0;
                self.state = State::Unscatter;
            }
            State::Unscatter => {
                self.tick_speed = slow;
                if self.anim_tick() {
                    self.smooth_grid();
                    self.pausing = 1.0;
                    self.state = State::Solve;
                }
            }
            State::Solve => {
                self.tick_speed = fast;
                if self.anim_tick() {
                    self.smooth_grid();
                    if self.solved() {
                        self.begin_scatter(false);
                        self.state = State::Scatter;
                        self.pausing = 3.0;
                    } else {
                        self.move_one_piece();
                        self.pausing = 0.3;
                    }
                }
            }
            State::Scatter => {
                self.tick_speed = slow;
                if self.anim_tick() {
                    self.puzzle.clear();
                    self.puzzle_width = 0;
                    self.puzzle_height = 0;
                    self.texid = None;
                    self.state = State::Loading;
                    self.pausing = 1.0;
                    let _ = g;
                }
            }
        }
    }

    /// `loading_msg`: one piece turning over on its own, with the word under
    /// it, for as long as there is no picture yet.
    fn loading_msg(&mut self, g: &mut Gl) {
        if self.wire {
            return;
        }
        let text = "Loading...";

        let mut p = Piece {
            edge: [OUT, OUT, IN, OUT],
            tris: Vec::new(),
            outline: Vec::new(),
            home: Xyzr::default(),
            current: Xyzr::default(),
            from: Xyzr::default(),
            to: Xyzr::default(),
            tick: 0.0,
            arc_height: 0.0,
            tilt: 0.0,
            max_tilt: 0.0,
        };
        p.build(self.resolution, self.thickness);

        g.glx.color4f(0.2, 0.2, 0.4, 1.0);
        g.glx.push_matrix();
        let (x, y, z) = self.rot.position(true);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        g.glx.scale(5.0, 5.0, 5.0);
        g.glx.translate(-0.5, -0.5, 0.0);
        let res = self.resolution;
        p.draw(g, self, res);
        g.glx.pop_matrix();

        let (w, h) = (g.width(), g.height());
        g.glx.color4f(0.7, 0.7, 1.0, 1.0);
        g.glx.lighting(false);
        g.glx.blend(Blend::Alpha);
        self.font
            .print_label(&mut g.glx, text, w, h, 0, [0.7, 0.7, 1.0, 1.0]);
    }
}

impl Hack3d for Jigsaw {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        // Ask for the picture until it turns up, then cut it into pieces.
        if self.texid.is_none() && !self.wire {
            let (w, h) = (g.width(), g.height());
            if let Some(img) = g.load_image(w.max(h), w.max(h)) {
                let id = self.texid.unwrap_or_else(|| g.glx.gen_texture());
                g.glx.bind_texture(id);
                g.glx.tex_image_2d(img.width, img.height, img.pixels);
                g.glx.tex_clamp(true);
                self.aspect = img.geometry.width as f32 / img.geometry.height.max(1) as f32;
                self.texid = Some(id);
                self.make_puzzle_grid(w, h);
            }
        }

        g.glx.depth_test(true);
        g.glx.clear();

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.puzzle.is_empty() {
            self.loading_msg(g);
        }
        self.animate(g);

        if self.wobble && !self.puzzle.is_empty() {
            let max = 60.0;
            let (_, _, z) = self.rot.position(!self.trackball.button_down());
            // Always lean back.
            g.glx.rotate(max / 2.0 - max, 1.0, 0.0, 0.0);
            g.glx.rotate(max / 2.0 - z as f32 * max, 0.0, 1.0, 0.0);
        }

        if !self.puzzle.is_empty() {
            let s = 14.0 / self.puzzle_height as f32;
            g.glx.cull_face(true);
            g.glx.depth_test(true);
            g.glx.scale(s, s, s);
            g.glx.translate(
                -(self.puzzle_width as f32) / 2.0,
                -(self.puzzle_height as f32) / 2.0,
                0.0,
            );

            if !self.wire {
                g.glx.lighting(true);
                g.glx.light_enable(0, true);
                g.glx.blend(Blend::Alpha);
            }

            for i in 0..self.puzzle.len() {
                let p = &self.puzzle[i];
                let (cur, tilt) = (p.current, p.tilt);
                g.glx.push_matrix();
                g.glx.translate(cur.x as f32, cur.y as f32, cur.z as f32);
                g.glx.translate(0.5, 0.5, 0.0);
                g.glx.rotate(cur.r as f32, 0.0, 0.0, 1.0);
                g.glx.rotate(tilt as f32, 0.0, 1.0, 0.0);
                g.glx.translate(-0.5, -0.5, 0.0);

                // Drawn directly rather than replayed from a list: a list here
                // would not carry the winding or the material the piece sets
                // as it goes.
                let res = self.resolution;
                self.puzzle[i].draw(g, self, res);
                g.glx.pop_matrix();
            }
        }

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
        if screenhack_event_helper(event) && !self.puzzle.is_empty() {
            self.begin_scatter(false);
            self.state = State::Scatter;
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed").clamp(0.1, 8.0);
    let complexity = g.res.float("complexity").clamp(1.0, 4.0);

    let mut resolution = (g.res.float("resolution").clamp(50.0, 300.0) / complexity) as i32;
    // Cutting the piece into eighths only comes out right on an even number.
    if resolution & 1 == 1 {
        resolution += 1;
    }

    let font = TexFont::load(&mut g.glx, "sans-serif bold 24");

    let mut st = Jigsaw {
        trackball: Trackball::new(),
        rot: Rotator::new(0.0, 0.0, 0.0, 0.0, speed * 0.002, true),
        font,
        puzzle_width: 0,
        puzzle_height: 0,
        puzzle: Vec::new(),
        state: State::LoadingMsg,
        pausing: 0.0,
        tick_speed: 0.01,
        texid: None,
        aspect: 1.0,
        speed,
        complexity,
        resolution,
        thickness: g.res.float("thickness").clamp(0.005, 0.5) as f32,
        wobble: g.res.bool("wobble"),
        wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        g.glx.light_position(0, 0.05, 0.07, 1.00, 0.0);
        g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    } else {
        st.make_puzzle_grid(w, h);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      20000",
    "*showFPS:    False",
    "*font:       sans-serif bold 24",
    "*wireframe:  False",
    "*speed:      1.0",
    "*complexity: 1.0",
    "*resolution: 100",
    "*thickness:  0.06",
    "*wobble:     True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 8.0, 0.1, 2, "1.0"),
    // Upstream's slider goes to four. Measured at 1280x720: complexity 1 is
    // 845 batches and 253k vertices, 2 is 3380 and 1.05M, and 4 is 13520 and
    // 4.3M, which is twice the batch ceiling. Capped at two.
    Opt::slider("complexity", "Puzzle pieces", 1.0, 2.0, 0.1, 1, "1.0"),
    Opt::slider("resolution", "Resolution", 50.0, 300.0, 10.0, 0, "100"),
    Opt::boolean("wobble", "Tilt", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "jigsaw",
    label: "Jigsaw",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=T5_hiY2eEeo"),
        blurb: "Carves an image up into a jigsaw puzzle, shuffles it, and solves it.",
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

    /// A puzzle with no GL behind it, cut to the given shape.
    fn a_puzzle(aspect: f32) -> Jigsaw {
        let mut jc = Jigsaw {
            trackball: Trackball::new(),
            rot: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.002, true),
            font: TexFont::load(&mut crate::runtime::gl::Glx::new(), "sans-serif bold 24"),
            puzzle_width: 0,
            puzzle_height: 0,
            puzzle: Vec::new(),
            state: State::LoadingMsg,
            pausing: 0.0,
            tick_speed: 0.01,
            texid: None,
            aspect,
            speed: 1.0,
            complexity: 1.0,
            resolution: 100,
            thickness: 0.06,
            wobble: true,
            wire: false,
        };
        jc.make_puzzle_grid(640, 480);
        jc
    }

    /// Neighbouring pieces have to interlock: where one has a tab the next has
    /// a hole, and the outside of the puzzle is straight all the way round.
    #[test]
    fn every_piece_fits_its_neighbours() {
        for aspect in [1.78f32, 1.0, 0.6] {
            let jc = a_puzzle(aspect);
            let (w, h) = (jc.puzzle_width, jc.puzzle_height);
            assert!(w >= 1 && h >= 1, "a puzzle of {w}x{h} is not a puzzle");
            assert_eq!(jc.puzzle.len(), w * h);

            for y in 0..h {
                for x in 0..w {
                    let p = &jc.puzzle[y * w + x];
                    if x + 1 < w {
                        let r = &jc.puzzle[y * w + x + 1];
                        assert_eq!(
                            p.edge[RIGHT], -r.edge[LEFT],
                            "{x},{y} does not meet the piece to its right"
                        );
                    } else {
                        assert_eq!(p.edge[RIGHT], FLAT, "the right border is not straight");
                    }
                    if y + 1 < h {
                        let b = &jc.puzzle[(y + 1) * w + x];
                        assert_eq!(
                            p.edge[BOTTOM], -b.edge[TOP],
                            "{x},{y} does not meet the piece below it"
                        );
                    } else {
                        assert_eq!(p.edge[BOTTOM], FLAT, "the bottom border is not straight");
                    }
                    if x == 0 {
                        assert_eq!(p.edge[LEFT], FLAT);
                    }
                    if y == 0 {
                        assert_eq!(p.edge[TOP], FLAT);
                    }
                }
            }
        }
    }

    /// Shuffling only ever swaps pieces that are the same shape, so the grid
    /// stays a permutation of itself: one piece per cell, no cell empty, and
    /// every piece turned to fit where it landed.
    #[test]
    fn shuffling_leaves_one_piece_in_every_cell() {
        let mut jc = a_puzzle(1.78);
        let (w, h) = (jc.puzzle_width, jc.puzzle_height);
        jc.shuffle_grid();

        let mut seen = vec![false; w * h];
        for p in &jc.puzzle {
            let (x, y) = (p.current.x, p.current.y);
            assert!(
                (0.0..w as f64).contains(&x) && (0.0..h as f64).contains(&y),
                "a piece is at {x},{y}, off the grid"
            );
            let i = y as usize * w + x as usize;
            assert!(!seen[i], "two pieces at {x},{y}");
            seen[i] = true;
        }
        assert!(seen.iter().all(|&b| b), "a cell was left empty");

        for (i, p) in jc.puzzle.iter().enumerate() {
            let j = (p.current.y as usize) * w + p.current.x as usize;
            assert!(
                same_shape(p, &jc.puzzle[j], p.current.r as i32),
                "piece {i} does not fit where it was shuffled to"
            );
        }
    }

    /// Every piece fits its own hole with no turn at all, which is what makes
    /// the solved puzzle the one the picture was cut from.
    #[test]
    fn a_piece_fits_its_own_hole_unturned() {
        let jc = a_puzzle(1.78);
        for (i, p) in jc.puzzle.iter().enumerate() {
            assert_eq!(
                jc.proper_rotation(i, p.home.x, p.home.y),
                0.0,
                "a piece did not fit its own hole"
            );
        }
    }

    /// Scattering throws every piece away from the middle and out of the
    /// frame; unscattering is the same move run backwards.
    #[test]
    fn scattering_throws_every_piece_clear() {
        let mut jc = a_puzzle(1.78);
        let far = jc.puzzle_width.max(jc.puzzle_height) as f64;
        jc.begin_scatter(false);
        for p in &jc.puzzle {
            let d = ((p.to.x - jc.puzzle_width as f64 / 2.0).powi(2)
                + (p.to.y - jc.puzzle_height as f64 / 2.0).powi(2))
            .sqrt();
            assert!(d >= far, "a piece only went {d} out");
            assert!(p.tick <= 0.0, "a piece started before it was thrown");
        }

        let mut jc = a_puzzle(1.78);
        jc.begin_scatter(true);
        for p in &jc.puzzle {
            // Coming in, the piece starts off screen and ends on the grid.
            assert!(p.to.x >= 0.0 && p.to.x < jc.puzzle_width as f64);
            assert_eq!(p.current, p.from, "it did not start where it came from");
        }
    }

    /// The tab is a spline that comes back to the far corner, so an edge with
    /// one starts and ends where a flat edge would.
    #[test]
    fn a_tab_starts_and_ends_where_a_flat_edge_would() {
        let s = make_puzzle_curve(100);
        assert!(s.points.len() > 20, "only {} points", s.points.len());
        let first = s.points[0];
        let last = s.points[s.points.len() - 1];
        assert!(first.x <= 1 && first.y.abs() <= 1, "starts at {first:?}");
        assert!(
            (last.x - 100).abs() <= 1 && last.y.abs() <= 1,
            "ends at {last:?}"
        );
        // And it bulges: the curve leaves the straight line in between.
        let out = s.points.iter().map(|p| p.y).max().expect("points");
        let inn = s.points.iter().map(|p| p.y).min().expect("points");
        assert!(out > 5, "the tab has no bulge: {out}");
        assert!(inn < -20, "the tab has no neck: {inn}");
    }

    /// A flat edge is one triangle; a tab is a fan of them. Either way the
    /// wedge is a whole number of triangles.
    #[test]
    fn a_wedge_is_whole_triangles() {
        let s = make_puzzle_curve(100);
        for kind in [FLAT, IN, OUT] {
            let mut v = Vec::new();
            make_piece_eighth(&s, 100, kind, &mut v, false, false, false);
            assert!(!v.is_empty(), "{kind} drew nothing");
            assert_eq!(v.len() % 3, 0, "{kind} left a part triangle");
            if kind == FLAT {
                assert_eq!(v.len(), 3, "a flat edge is one triangle");
            } else {
                assert!(v.len() > 30, "{kind} is only {} vertices", v.len());
            }
        }
    }

    /// It draws, and the picture ends up on the pieces.
    #[test]
    fn the_puzzle_is_drawn_from_the_picture() {
        let r = run("", 6);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(
            f.batches.iter().any(|b| b.texture.is_some()),
            "the picture was never put on a piece"
        );
    }
}
