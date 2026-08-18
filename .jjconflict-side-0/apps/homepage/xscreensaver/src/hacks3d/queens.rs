//! Port of `hacks/glx/queens.c`.
//!
//! ```text
//! queens - solves n queens problem, displays
//! i make no claims that this is an optimal solution to the problem,
//! good enough for xss
//! hacked from glchess
//!
//! version 1.0 - May 10, 2002
//!
//! Copyright (C) 2002 Blair Tennessy (tennessy@cs.ubc.ca)
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
//! The eight queens puzzle, or rather the N queens puzzle, solved by plain
//! backtracking and then stood on a board: place a queen on every row so that
//! no two share a column or a diagonal. The board is between five and ten
//! squares a side, and every seventeen seconds or so it drops out of the
//! bottom of the screen and a new one of a new size drops in from the top.
//!
//! The solver is not clever and does not need to be. It walks the rows in
//! order, and at each row takes the first column that does not conflict; when
//! it runs out of columns it gives up on the whole board and starts again from
//! a different random column in the top row. For boards this small that is
//! instant, and the randomness is what stops it drawing the same solution
//! every time.
//!
//! The reflections in the board are what the stencil buffer is for. The tiles
//! are painted into the stencil first with the colour mask off, and then the
//! pieces are drawn again upside down with the test set so that only the
//! stencilled pixels take: the reflection appears on the board and stops dead
//! at its edge. The light is mirrored along with the pieces, or the reflection
//! would be lit from the wrong side.
//!
//! Upstream turns wireframe off wherever `glPolygonMode` is missing, which is
//! here, so there is no wireframe knob. Its light attenuation is left out too,
//! with upstream's own note that it does nothing: the fade in and out is done
//! by moving the board, not by dimming it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::chessmodels::Piece;
use crate::runtime::gl::{Blend, Shape, Stencil, StencilFunc, StencilOp};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random_below,
    screenhack_event_helper,
};

const MINBOARD: usize = 5;
const MAXBOARD: usize = 10;
const COLORSETS: usize = 5;

/// How many frames a board lasts.
const MAX_STEPS: i32 = 1024;

/// Definition of white/black colors.
const COLORS: [[[f32; 3]; 2]; COLORSETS] = [
    [[0.43, 0.54, 0.76], [0.8, 0.8, 0.8]],
    [[0.5, 0.7, 0.9], [0.2, 0.3, 0.6]],
    [[0.537_254_9, 0.360_784_31, 0.521_568_65], [0.6, 0.6, 0.6]],
    [[0.15, 0.77, 0.54], [0.5, 0.5, 0.5]],
    [[0.9, 0.45, 0.0], [0.5, 0.5, 0.5]],
];

/* lighting variables */
const FRONT_SHININESS: f32 = 60.0;
const FRONT_SPECULAR: [f32; 4] = [0.4, 0.4, 0.4, 1.0];
const AMBIENT: [f32; 4] = [0.3, 0.3, 0.3, 1.0];
const DIFFUSE: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

struct Queens {
    trackball: Trackball,
    position: [f32; 4],
    queen: Piece,

    board: [[bool; MAXBOARD]; MAXBOARD],
    steps: i32,
    colorset: usize,
    board_size: usize,
    theta: f64,
}

impl Queens {
    /// Returns true if placing a queen on column c causes a conflict.
    fn conflicts_cols(&self, c: usize) -> bool {
        (0..self.board_size).any(|i| self.board[i][c])
    }

    /// Returns true if placing a queen on (r,c) causes a diagonal conflict.
    fn conflicts_diag(&self, r: usize, c: usize) -> bool {
        let size = self.board_size as isize;
        let (r, c) = (r as isize, c as isize);

        /* positive slope */
        let n = r.min(c);
        for i in 0..size - (r - c).abs() {
            if self.board[(r - n + i) as usize][(c - n + i) as usize] {
                return true;
            }
        }

        /* negative slope */
        let n = r.min(size - (c + 1));
        for i in 0..size - (size - (1 + r + c)).abs() {
            if self.board[(r - n + i) as usize][(c + n - i) as usize] {
                return true;
            }
        }

        false
    }

    /// Returns true if placing a queen at (r,c) causes a conflict.
    fn conflicts(&self, r: usize, c: usize) -> bool {
        if self.conflicts_cols(c) {
            true
        } else {
            self.conflicts_diag(r, c)
        }
    }

    /// Clear board.
    fn blank(&mut self) {
        self.board = [[false; MAXBOARD]; MAXBOARD];
    }

    /// Recursively determine solution.
    fn find_solution(&mut self, row: usize, mut col: usize) -> bool {
        if row == self.board_size {
            return true;
        }

        while col < self.board_size {
            if !self.conflicts(row, col) {
                self.board[row][col] = true;

                if self.find_solution(row + 1, 0) {
                    return true;
                }

                self.board[row][col] = false;
            }

            col += 1;
        }

        false
    }

    /// Driver for finding solution. A run that fails leaves the board as it
    /// found it, so retrying from a different column is all that is needed.
    fn go(&mut self) {
        while !self.find_solution(0, random_below(self.board_size as i32) as usize) {}
    }

    /// Configure lighting.
    fn setup_lights(&self, g: &mut Gl) {
        /* setup twoside lighting */
        g.glx.light_ambient(0, AMBIENT);
        g.glx.light_diffuse(0, DIFFUSE);
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);

        /* setup material properties */
        g.glx.material_shininess(FRONT_SHININESS);
        g.glx.material_specular(FRONT_SPECULAR);
    }

    /// Draw pieces.
    ///
    /// Upstream leaves the colour to `glColorMaterial (GL_FRONT, GL_DIFFUSE)`,
    /// which is the diffuse alone and not the ambient. Here the two rows'
    /// colours go into the material directly, which comes to the same thing
    /// and leaves the ambient at OpenGL's own dim grey.
    fn draw_pieces(&self, g: &mut Gl) {
        for i in 0..self.board_size {
            for j in 0..self.board_size {
                if self.board[i][j] {
                    let c = COLORS[self.colorset][i % 2];
                    g.glx.material_diffuse([c[0], c[1], c[2], 1.0]);
                    self.queen.draw(&mut g.glx);
                }

                g.glx.translate(1.0, 0.0, 0.0);
            }

            g.glx.translate(-(self.board_size as f32), 0.0, 1.0);
        }
    }

    /// Reflectionboard.
    fn draw_reflections(&self, g: &mut Gl) {
        g.glx.stencil(Some(Stencil {
            func: StencilFunc::Always,
            reference: 1,
            pass: StencilOp::Replace,
        }));
        g.glx.color_mask(false);
        g.glx.cull_face(false);

        g.glx.depth_test(false);
        g.glx.begin(Shape::Quads);

        /* only draw white squares */
        for i in 0..self.board_size {
            for j in ((self.board_size + i) % 2..self.board_size).step_by(2) {
                let (i, j) = (i as f32, j as f32);
                g.glx.vertex3f(i, 0.0, j + 1.0);
                g.glx.vertex3f(i + 1.0, 0.0, j + 1.0);
                g.glx.vertex3f(i + 1.0, 0.0, j);
                g.glx.vertex3f(i, 0.0, j);
            }
        }
        g.glx.end();
        g.glx.depth_test(true);

        g.glx.color_mask(true);
        g.glx.stencil(Some(Stencil {
            func: StencilFunc::Equal,
            reference: 1,
            pass: StencilOp::Keep,
        }));

        g.glx.push_matrix();
        g.glx.scale(1.0, -1.0, 1.0);
        g.glx.translate(0.5, 0.001, 0.5);
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);
        self.draw_pieces(g);
        g.glx.pop_matrix();
        g.glx.stencil(None);

        /* replace lights */
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);

        g.glx.cull_face(true);
    }

    /// Draw board.
    fn draw_board(&self, g: &mut Gl) {
        // Upstream colours each square inside one block and lets
        // GL_COLOR_MATERIAL carry it. There are only ever two colours, so the
        // squares are drawn a colour at a time instead, which says the same
        // thing to the material and keeps the ambient out of it.
        for (par, c) in COLORS[self.colorset].iter().enumerate() {
            g.glx.material_diffuse([c[0], c[1], c[2], 0.70]);
            g.glx.begin(Shape::Quads);
            for i in 0..self.board_size {
                for j in 0..self.board_size {
                    if (i + self.board_size - j) % 2 != par {
                        continue;
                    }
                    let (i, j) = (i as f32, j as f32);
                    g.glx.normal3f(0.0, 1.0, 0.0);
                    g.glx.vertex3f(i, 0.0, j + 1.0);
                    g.glx.vertex3f(i + 1.0, 0.0, j + 1.0);
                    g.glx.vertex3f(i + 1.0, 0.0, j);
                    g.glx.vertex3f(i, 0.0, j);
                }
            }
            g.glx.end();
        }

        {
            let off = 0.01;
            let w = self.board_size as f32;
            let h = 0.1;

            /* Give the board a slight lip. */
            /* #### oops, normals are wrong here, but you can't tell */

            g.glx.material_diffuse([0.3, 0.3, 0.3, 1.0]);
            box_sides(g, w, h);

            /* Fill in the underside of the board with an invisible black box
               to hide the reflections that are not on tiles.  Probably there's
               a way to do this with stencils instead.
            */
            let w = w - off * 2.0;
            let h = 5.0;

            g.glx.push_matrix();
            g.glx.translate(off, 0.0, off);
            g.glx.lighting(false);
            g.glx.color3f(0.0, 0.0, 0.0);
            box_sides(g, w, h);
            g.glx.pop_matrix();
            g.glx.lighting(true);
        }
    }

    fn display(&mut self, g: &mut Gl) {
        g.glx.clear();

        g.glx.load_identity();

        /* setup perspective */
        g.glx.translate(0.0, 0.0, -1.5 * self.board_size as f32);
        g.glx.rotate(30.0, 1.0, 0.0, 0.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(self.theta as f32 * 100.0, 0.0, 1.0, 0.0);
        g.glx.translate(
            -0.5 * self.board_size as f32,
            0.0,
            -0.5 * self.board_size as f32,
        );

        /* find light positions */
        let b = self.board_size as f64;
        let a = self.theta * 100.0 * std::f64::consts::PI / 180.0;
        self.position[0] = (b / 2.0 + b / 1.4 * -a.sin()) as f32;
        self.position[2] = (b / 2.0 + b / 1.4 * a.cos()) as f32;
        self.position[1] = 6.0;

        g.glx.lighting(true);
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);
        g.glx.light_enable(0, true);

        /* Since the lighting attenuation trick up there doesn't seem to be working,
        let's drop the old board down and drop the new board in. */
        let eighth = MAX_STEPS as f32 / 8.0;
        if (self.steps as f32) < eighth {
            let y = self.steps as f32 / eighth;
            let y = (std::f32::consts::FRAC_PI_2 * y).sin();
            g.glx.translate(0.0, 10.0 - (y * 10.0), 0.0);
        } else if self.steps as f32 > MAX_STEPS as f32 - eighth {
            let y = (self.steps as f32 - (MAX_STEPS as f32 - eighth)) / eighth;
            let y = 1.0 - (std::f32::consts::FRAC_PI_2 * (1.0 - y)).sin();
            g.glx.translate(0.0, -y * 15.0, 0.0);
        }

        /* draw reflections */
        self.draw_reflections(g);
        g.glx.blend(Blend::Alpha);
        self.draw_board(g);
        g.glx.blend(Blend::Off);

        g.glx.translate(0.5, 0.0, 0.5);
        self.draw_pieces(g);

        /* rotate camera */
        if !self.trackball.button_down() {
            self.theta += 0.002;
        }

        /* zero out board, find new solution of size MINBOARD <= i <= MAXBOARD */
        self.steps += 1;
        if self.steps == MAX_STEPS {
            self.steps = 0;
            self.blank();
            self.board_size = MINBOARD + random_below((MAXBOARD - MINBOARD + 1) as i32) as usize;
            self.colorset = (self.colorset + 1) % COLORSETS;
            self.go();
        }
    }
}

/// The four walls and the floor of a box `w` square and `h` deep hanging below
/// the origin, which is both the board's lip and the black box under it.
fn box_sides(g: &mut Gl, w: f32, h: f32) {
    g.glx.begin(Shape::Quads);
    for [a, b, c, d] in [
        [[0.0, 0.0, 0.0], [0.0, -h, 0.0], [0.0, -h, w], [0.0, 0.0, w]],
        [[0.0, 0.0, w], [0.0, -h, w], [w, -h, w], [w, 0.0, w]],
        [[w, 0.0, w], [w, -h, w], [w, -h, 0.0], [w, 0.0, 0.0]],
        [[w, 0.0, 0.0], [w, -h, 0.0], [0.0, -h, 0.0], [0.0, 0.0, 0.0]],
        [[0.0, -h, 0.0], [w, -h, 0.0], [w, -h, w], [0.0, -h, w]],
    ] {
        for v in [a, b, c, d] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

impl Hack3d for Queens {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.setup_lights(g);
        g.glx.cull_face(true);
        self.display(g);
        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height.max(1);
        let mut h = height as f32 / width as f32;
        let mut y = 0;

        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0 / h, 2.0, 30.0);
        g.glx.matrix_mode_modelview();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            true
        } else if screenhack_event_helper(event) {
            self.steps = MAX_STEPS - 1;
            true
        } else {
            false
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut st = Queens {
        trackball: Trackball::new(),
        position: [0.0, 0.0, 0.0, 1.0],
        queen: Piece::queen(),
        board: [[false; MAXBOARD]; MAXBOARD],
        steps: 0,
        colorset: 0,
        board_size: 8, /* 8 cuz its classic */
        theta: 0.0,
    };

    /* find a solution */
    st.go();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:       20000",
    "*showFPS:     False",
    "*wireframe:   False",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "queens",
    label: "Queens",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Blair Tennessy and Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=Ssy0ldFDeAs"),
        blurb: "The N-Queens problem: how to place N queens on an NxN chessboard \
                such that no queen can attack a sister?",
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

    fn solved(size: usize) -> Queens {
        let mut st = Queens {
            trackball: Trackball::new(),
            position: [0.0, 0.0, 0.0, 1.0],
            queen: Piece::queen(),
            board: [[false; MAXBOARD]; MAXBOARD],
            steps: 0,
            colorset: 0,
            board_size: size,
            theta: 0.0,
        };
        st.go();
        st
    }

    #[test]
    fn no_queen_can_attack_a_sister() {
        crate::runtime::ya_rand_init(20260812);
        for size in MINBOARD..=MAXBOARD {
            let st = solved(size);
            let at: Vec<(usize, usize)> = (0..size)
                .flat_map(|i| (0..size).map(move |j| (i, j)))
                .filter(|&(i, j)| st.board[i][j])
                .collect();
            assert_eq!(at.len(), size, "{size}: {at:?}");
            for (n, &(r1, c1)) in at.iter().enumerate() {
                for &(r2, c2) in &at[n + 1..] {
                    assert_ne!(r1, r2, "{size}: two on row {r1}");
                    assert_ne!(c1, c2, "{size}: two on column {c1}");
                    assert_ne!(
                        r1.abs_diff(r2),
                        c1.abs_diff(c2),
                        "{size}: {:?} attacks {:?} diagonally",
                        (r1, c1),
                        (r2, c2)
                    );
                }
            }
        }
    }

    #[test]
    fn the_reflection_is_masked_to_the_board() {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        let mut st = solved(8);
        st.reshape(&mut g, 640, 480);
        g.glx.start_frame(640, 480);
        st.draw(&mut g);

        let batches = &g.glx.frame().batches;
        // The stencil is written by one pass with the colour mask off, and
        // read by the mirrored pieces.
        let writes = batches
            .iter()
            .filter(|b| b.stencil.is_some_and(|s| s.pass == StencilOp::Replace))
            .count();
        assert_eq!(writes, 1, "{writes}");
        assert!(
            batches
                .iter()
                .any(|b| b.stencil.is_some_and(|s| s.pass == StencilOp::Keep))
        );
        assert!(batches.iter().all(|b| b.color_mask == [true; 4]
            || b.stencil.is_some_and(|s| s.pass == StencilOp::Replace)));
        // Eight queens, upright and reflected, plus a board.
        let pieces = batches.iter().filter(|b| b.count > 100).count();
        assert!(pieces >= 16, "{pieces} of {}", batches.len());
        assert!(batches.len() < 200, "{}", batches.len());
    }

    #[test]
    fn the_reflection_falls_on_the_squares_of_one_colour() {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        let mut st = solved(8);
        st.reshape(&mut g, 640, 480);
        g.glx.start_frame(640, 480);
        st.draw(&mut g);

        // Both the stencil pass and the board are quads at whole numbers in
        // the x/z plane, so which squares a batch covers can be read straight
        // off its vertices.
        let squares = |b: &crate::runtime::gl::Batch| {
            let v = &g.glx.frame().vertices;
            let mut s: Vec<(i32, i32)> = v[b.first..b.first + b.count]
                .chunks(4)
                .map(|q| {
                    let least = |i: usize| q.iter().fold(f32::MAX, |a, v| a.min(v.pos[i]));
                    (least(0) as i32, least(2) as i32)
                })
                .collect();
            s.sort_unstable();
            s.dedup();
            s
        };
        let batches = &g.glx.frame().batches;
        let stencilled = squares(
            batches
                .iter()
                .find(|b| b.stencil.is_some_and(|s| s.pass == StencilOp::Replace))
                .expect("a stencil pass"),
        );
        // The first board batch is the one drawn in the first of the two
        // colours, which is the one upstream calls white.
        let white = squares(
            batches
                .iter()
                .find(|b| b.stencil.is_none() && b.material.ambient_diffuse[3] < 1.0)
                .expect("a board"),
        );
        assert_eq!(stencilled.len(), 32, "{stencilled:?}");
        assert_eq!(stencilled, white);
    }
}
