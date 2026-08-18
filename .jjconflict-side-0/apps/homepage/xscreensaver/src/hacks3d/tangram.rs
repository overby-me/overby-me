//! Port of `hacks/glx/tangram.c` and `hacks/glx/tangram_shapes.c`.
//!
//! ```text
//! tangram, Copyright (c) 2005-2014 Jeremy English <jhe@jeremyenglish.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software
//! and its documentation for any purpose is hereby granted without
//! fee, provided that the above copyright notice appear in all copies
//! and that both that copyright notice and this permission notice
//! appear in supporting documentation.  No representations are made
//! about the suitability of this software for any purpose.  It is
//! provided "as is" without express or implied warranty.
//! ```
//!
//! The seven pieces of the Chinese puzzle, solving one figure after another.
//!
//! Nothing is solved at run time. Forty-five figures are stored as the
//! finished position of each piece, and the animation is the pieces travelling
//! from wherever they were to where the next figure wants them: each one eases
//! its position by a tenth of a unit and its two rotations by two degrees a
//! frame until it arrives, and a piece that has arrived stays put while the
//! rest catch up. When all seven have arrived the figure is named, held for a
//! few seconds, and the next one is dealt.
//!
//! While a piece is travelling it is also bouncing. It is thrown upward with a
//! velocity, pulled back down, and re-thrown from the floor when it lands, so
//! the pieces hop their way across the board rather than sliding. Half of them
//! bounce the other way, below the board instead of above it, which is what
//! the `up` flag in each stored position decides.
//!
//! The pieces are the real ones: two large triangles, one medium, two small,
//! a square and a rhomboid, all cut from the same 45-degree right triangle
//! scaled by one, the square root of two, and two. Each is given a thickness
//! so it can be lit as a solid rather than a flat shape.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_random_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random, screenhack_event_helper,
};

const NUM_SHAPES: usize = 7;
const SPEED: f32 = 0.03;
const BOTTOM: f32 = 0.0;
const INIT_DZ: f32 = 2.0;
/// Half the thickness of a piece.
const ALPHA: f32 = 0.05;

const SMALL_SCALE: f32 = 1.0;
const LARGE_SCALE: f32 = 2.0;
const MEDIUM_SCALE: f32 = std::f32::consts::SQRT_2;

/// Which of the seven pieces a slot holds. The order is upstream's, and the
/// stored figures are indexed by it.
const PIECES: [Piece; NUM_SHAPES] = [
    Piece::SmallTriangle,
    Piece::SmallTriangle,
    Piece::MediumTriangle,
    Piece::LargeTriangle,
    Piece::LargeTriangle,
    Piece::Square,
    Piece::Rhomboid,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Piece {
    SmallTriangle,
    MediumTriangle,
    LargeTriangle,
    Square,
    Rhomboid,
}

/// Where a piece ends up in a finished figure.
struct Placed {
    x: f32,
    y: f32,
    r: i32,
    fr: i32,
    up: bool,
}

struct Solution {
    name: &'static str,
    shapes: [Placed; NUM_SHAPES],
}

/// One piece as it is right now: where it is, how it is turned, and how it is
/// bouncing.
#[derive(Clone, Copy)]
struct TangramShape {
    x: f32,
    y: f32,
    z: f32,
    /// Rotation about the vertical, in degrees.
    r: i32,
    /// The flip, which is how a piece gets turned over.
    fr: i32,
    /// Velocity and acceleration of the bounce.
    dz: f32,
    ddz: f32,
    solved: bool,
    up: bool,
}

impl TangramShape {
    fn from(p: &Placed) -> Self {
        TangramShape {
            x: p.x,
            y: p.y,
            z: 0.0,
            r: p.r,
            fr: p.fr,
            dz: INIT_DZ,
            ddz: -SPEED,
            solved: false,
            up: p.up,
        }
    }
}

struct Tangram {
    /// The pieces as they are, and where they are going.
    shapes: [TangramShape; NUM_SHAPES],
    goals: [TangramShape; NUM_SHAPES],
    lists: [u32; NUM_SHAPES],
    /// Which figure was drawn last, so the same one is never dealt twice.
    current: usize,
    puzzle_name: &'static str,
    /// The name shown on screen, which is empty while the pieces are moving.
    shown_name: &'static str,
    display_counter: i32,
    colors: Vec<XColor>,
    color: [f32; 4],

    theta: [f32; 3],
    going_down: [bool; 3],

    font_large: Option<TexFont>,
    font_medium: Option<TexFont>,
    font_small: Option<TexFont>,

    wireframe: bool,
    do_rotate: bool,
    do_labels: bool,
    viewing_time: i32,
    camera_rotate: [f32; 3],
}

/// `tri_45_90`: the right triangle every piece is cut from, given a thickness.
fn tri_45_90(g: &mut Gl, wire: bool) {
    let v = [
        [0.0, ALPHA, 0.0],
        [0.0, ALPHA, 1.0],
        [1.0, ALPHA, 0.0],
        [0.0, -ALPHA, 0.0],
        [0.0, -ALPHA, 1.0],
        [1.0, -ALPHA, 0.0],
    ];
    let face = |g: &mut Gl, n: [f32; 3], idx: &[usize]| {
        g.glx.normal3f(n[0], n[1], n[2]);
        for &i in idx {
            g.glx.vertex3f(v[i][0], v[i][1], v[i][2]);
        }
    };

    g.glx.begin(if wire {
        Shape::LineLoop
    } else {
        Shape::Triangles
    });
    face(g, [0.0, 1.0, 0.0], &[0, 2, 1]);
    face(g, [0.0, -1.0, 0.0], &[3, 4, 5]);
    g.glx.end();

    g.glx
        .begin(if wire { Shape::LineLoop } else { Shape::Quads });
    face(g, [1.0, 0.0, 1.0], &[2, 5, 4, 1]);
    face(g, [-1.0, 0.0, 0.0], &[0, 1, 4, 3]);
    face(g, [0.0, 0.0, -1.0], &[0, 3, 5, 2]);
    g.glx.end();
}

/// `unit_cube`: the square piece, a slab a unit on a side.
fn unit_cube(g: &mut Gl, wire: bool) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 1.0, 0.0],
            [
                [0.0, ALPHA, 0.0],
                [0.0, ALPHA, 1.0],
                [1.0, ALPHA, 1.0],
                [1.0, ALPHA, 0.0],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [0.0, -ALPHA, 1.0],
                [1.0, -ALPHA, 1.0],
                [1.0, ALPHA, 1.0],
                [0.0, ALPHA, 1.0],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [0.0, -ALPHA, 0.0],
                [0.0, ALPHA, 0.0],
                [1.0, ALPHA, 0.0],
                [1.0, -ALPHA, 0.0],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [1.0, -ALPHA, 0.0],
                [1.0, ALPHA, 0.0],
                [1.0, ALPHA, 1.0],
                [1.0, -ALPHA, 1.0],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [0.0, -ALPHA, 0.0],
                [0.0, -ALPHA, 1.0],
                [0.0, ALPHA, 1.0],
                [0.0, ALPHA, 0.0],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [0.0, -ALPHA, 0.0],
                [1.0, -ALPHA, 0.0],
                [1.0, -ALPHA, 1.0],
                [0.0, -ALPHA, 1.0],
            ],
        ),
    ];
    g.glx
        .begin(if wire { Shape::LineLoop } else { Shape::Quads });
    for (n, vs) in faces {
        g.glx.normal3f(n[0], n[1], n[2]);
        for p in vs {
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
    }
    g.glx.end();
}

/// `unit_rhomboid`: the one piece that has to be turned over to fit some
/// figures, which is what the flip rotation is for.
fn unit_rhomboid(g: &mut Gl, wire: bool) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 1.0, 0.0],
            [
                [0.0, ALPHA, 0.0],
                [1.0, ALPHA, 1.0],
                [1.0, ALPHA, 2.0],
                [0.0, ALPHA, 1.0],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [0.0, -ALPHA, 0.0],
                [0.0, -ALPHA, 1.0],
                [1.0, -ALPHA, 2.0],
                [1.0, -ALPHA, 1.0],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [0.0, ALPHA, 0.0],
                [0.0, ALPHA, 1.0],
                [0.0, -ALPHA, 1.0],
                [0.0, -ALPHA, 0.0],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [0.0, ALPHA, 1.0],
                [1.0, ALPHA, 2.0],
                [1.0, -ALPHA, 2.0],
                [0.0, -ALPHA, 1.0],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [1.0, ALPHA, 1.0],
                [1.0, -ALPHA, 1.0],
                [1.0, -ALPHA, 2.0],
                [1.0, ALPHA, 2.0],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [0.0, ALPHA, 0.0],
                [0.0, -ALPHA, 0.0],
                [1.0, -ALPHA, 1.0],
                [1.0, ALPHA, 1.0],
            ],
        ),
    ];
    g.glx
        .begin(if wire { Shape::LineLoop } else { Shape::Quads });
    for (n, vs) in faces {
        g.glx.normal3f(n[0], n[1], n[2]);
        for p in vs {
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
    }
    g.glx.end();
}

/// `approach_number`: move an integer towards a goal by at most `step`.
fn approach_number(goal: i32, current: i32, step: i32) -> i32 {
    if goal > current {
        (current + step).min(goal)
    } else if goal < current {
        (current - step).max(goal)
    } else {
        current
    }
}

/// `approach_float`, and whether it moved. The tolerance is the step, so a
/// piece within one step of its goal is considered to have arrived.
fn approach_float(goal: f32, current: f32, per: f32) -> (f32, bool) {
    if current < goal && (goal - current).abs() > per {
        (current + per, true)
    } else if current > goal && (goal - current).abs() > per {
        (current - per, true)
    } else {
        (current, false)
    }
}

impl Tangram {
    /// `get_solved_puzzle`: deal a figure that is not the one just drawn.
    fn deal(&mut self) -> [TangramShape; NUM_SHAPES] {
        let n = SOLUTIONS.len();
        let mut r = self.current;
        while r == self.current {
            r = (random() as usize) % n;
        }
        self.current = r;
        self.puzzle_name = SOLUTIONS[r].name;
        std::array::from_fn(|i| TangramShape::from(&SOLUTIONS[r].shapes[i]))
    }

    /// `bounce`: throw the piece up, pull it back down, and re-throw it from
    /// the floor. Upstream negates z either side of the arithmetic so the same
    /// code serves the pieces that bounce downward.
    fn bounce(&self, s: &mut TangramShape) {
        s.z *= -1.0;
        s.dz += s.ddz;
        s.z += s.dz * SPEED;
        if s.z < BOTTOM {
            s.z = BOTTOM;
            s.dz = INIT_DZ + (random() % 10) as f32 / 10.0;
            s.ddz = -SPEED;
        }
        s.z *= -1.0;
    }

    /// `solve`: ease one piece towards its goal, and note whether it arrived.
    fn solve(goal: &TangramShape, s: &mut TangramShape) {
        s.fr = approach_number(goal.fr, s.fr, 2);
        let moved_fr = s.fr != goal.fr;
        s.r = approach_number(goal.r, s.r, 2);
        let moved_r = s.r != goal.r;

        let (x, moved_x) = approach_float(goal.x, s.x, 0.1);
        s.x = if moved_x { x } else { goal.x };
        let (y, moved_y) = approach_float(goal.y, s.y, 0.1);
        s.y = if moved_y { y } else { goal.y };

        let z_ok = -s.z <= BOTTOM;
        s.solved = !moved_x && !moved_y && !moved_r && !moved_fr && z_ok;
    }

    /// `set_camera`. The pitch swings between level and overhead and back;
    /// the other two axes just turn.
    fn set_camera(&mut self, g: &mut Gl) {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        // Upstream passes an aspect of -1, which mirrors the scene. Kept,
        // because the stored figures are laid out for it.
        g.glx.perspective(60.0, -1.0, 0.1, 50.0);
        g.glx
            .look_at([0.0, 5.0, -5.0], [0.0, 0.0, 0.0], [0.0, -1.0, 0.0]);
        if self.do_rotate {
            g.glx.rotate(self.theta[0], 1.0, 0.0, 0.0);
            g.glx.rotate(self.theta[1], 0.0, 1.0, 0.0);
            g.glx.rotate(self.theta[2], 0.0, 0.0, 1.0);
        }
        g.glx.matrix_mode_modelview();

        if self.going_down[0] && self.theta[0] < 0.0 {
            self.going_down[0] = false;
        } else if !self.going_down[0] && self.theta[0] > 90.0 {
            self.going_down[0] = true;
        }
        if self.theta[1] > 360.0 {
            self.theta[1] -= 360.0;
        }
        if self.theta[2] > 360.0 {
            self.theta[2] -= 360.0;
        }
        if self.going_down[0] {
            self.theta[0] -= self.camera_rotate[0];
        } else {
            self.theta[0] += self.camera_rotate[0];
        }
        self.theta[1] += self.camera_rotate[1];
        self.theta[2] += self.camera_rotate[2];
    }

    fn draw_shape(&self, g: &mut Gl, i: usize) {
        let s = self.shapes[i];
        let up = if self.do_rotate { s.up } else { true };
        g.glx.push_matrix();
        g.glx.translate(s.x, s.y, if up { s.z } else { -s.z });
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(s.fr as f32, 1.0, 0.0, 0.0);
        g.glx.rotate(s.r as f32, 0.0, 1.0, 0.0);
        g.glx.call_list(self.lists[i]);
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let do_rotate = g.res.bool("rotate");

    let blank = TangramShape {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        r: 0,
        fr: 0,
        dz: INIT_DZ,
        ddz: -SPEED,
        solved: false,
        up: true,
    };

    let mut this = Tangram {
        shapes: [blank; NUM_SHAPES],
        goals: [blank; NUM_SHAPES],
        lists: [0; NUM_SHAPES],
        current: usize::MAX,
        puzzle_name: "",
        shown_name: "",
        display_counter: 0,
        colors: Vec::new(),
        color: [0.5, 0.5, 0.5, 1.0],
        theta: [1.0; 3],
        going_down: [false; 3],
        font_large: Some(TexFont::load(&mut g.glx, "sans-serif 18")),
        font_medium: Some(TexFont::load(&mut g.glx, "sans-serif 12")),
        font_small: Some(TexFont::load(&mut g.glx, "sans-serif 8")),
        wireframe,
        do_rotate,
        do_labels: g.res.bool("labels"),
        viewing_time: g.res.int("viewing_time").max(0),
        camera_rotate: [
            g.res.float("x_camera_rotate") as f32,
            g.res.float("y_camera_rotate") as f32,
            g.res.float("z_camera_rotate") as f32,
        ],
    };

    this.shapes = this.deal();
    this.goals = this.deal();

    // Each piece is the same right triangle at one of three scales, or the
    // square, or the rhomboid.
    for (i, piece) in PIECES.into_iter().enumerate() {
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        match piece {
            Piece::SmallTriangle => {
                g.glx.scale(SMALL_SCALE, SMALL_SCALE, SMALL_SCALE);
                tri_45_90(g, wireframe);
            }
            Piece::MediumTriangle => {
                g.glx.scale(MEDIUM_SCALE, SMALL_SCALE, MEDIUM_SCALE);
                tri_45_90(g, wireframe);
            }
            Piece::LargeTriangle => {
                g.glx.scale(LARGE_SCALE, SMALL_SCALE, LARGE_SCALE);
                tri_45_90(g, wireframe);
            }
            Piece::Square => {
                g.glx.scale(SMALL_SCALE, SMALL_SCALE, SMALL_SCALE);
                unit_cube(g, wireframe);
            }
            Piece::Rhomboid => {
                g.glx.scale(SMALL_SCALE, SMALL_SCALE, SMALL_SCALE);
                unit_rhomboid(g, wireframe);
            }
        }
        g.glx.end_list();
        this.lists[i] = list;
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Tangram {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        if width > height * 5 {
            // Tiny window: show the middle.
            height = width;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.display_counter = 0;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let max_display = self.viewing_time * 100;
        self.set_camera(g);

        if self.display_counter <= 0 {
            for i in 0..NUM_SHAPES {
                if self.shapes[i].solved {
                    if self.shapes.iter().all(|s| s.solved) {
                        // The figure is finished: name it, hold it, and deal
                        // the next one along with a new colour.
                        self.display_counter = max_display;
                        self.shown_name = self.puzzle_name;
                        self.goals = self.deal();
                        self.colors = make_random_colormap(128, true);
                        self.color = [
                            self.colors[0].red as f32 / 65536.0,
                            self.colors[1].green as f32 / 65536.0,
                            self.colors[2].blue as f32 / 65536.0,
                            1.0,
                        ];
                        for s in &mut self.shapes {
                            s.solved = false;
                        }
                        break;
                    }
                } else {
                    self.shown_name = "";
                    let mut s = self.shapes[i];
                    self.bounce(&mut s);
                    Self::solve(&self.goals[i], &mut s);
                    self.shapes[i] = s;
                }
            }
        } else {
            self.display_counter -= 1;
        }

        g.glx.clear();
        if !self.wireframe {
            g.glx.depth_test(true);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            // The lights move with the camera rather than the board.
            let (x, y) = if self.do_rotate {
                (5.0, -10.0)
            } else {
                (10.0, 3.0)
            };
            g.glx.light_position(0, -x, y, -5.0, 1.0);
            if self.do_rotate {
                g.glx.light_enable(1, true);
                g.glx.light_position(1, 0.0, y, 5.0, 1.0);
                g.glx.light_diffuse(1, [1.0, 1.0, 1.0, 1.0]);
            }
        }

        g.glx.push_matrix();
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse(self.color);

        for i in 0..NUM_SHAPES {
            self.draw_shape(g, i);
        }

        if self.do_labels {
            let (w, h) = (g.width(), g.height());
            let font = if w >= 500 && h >= 375 {
                &self.font_large
            } else if w >= 350 && h >= 260 {
                &self.font_medium
            } else {
                &self.font_small
            };
            if let Some(font) = font {
                font.print_label(&mut g.glx, self.shown_name, w, h, 1, [0.8, 0.8, 0.0, 1.0]);
            }
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        10000",
    "*wireframe:    False",
    "*titleFont:    sans-serif 18",
    "*titleFont2:   sans-serif 12",
    "*titleFont3:   sans-serif 8",
    "*viewing_time: 5",
    "*rotate:       True",
    "*labels:       True",
    "*x_camera_rotate: 0.2",
    "*y_camera_rotate: 0.5",
    "*z_camera_rotate: 0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000"),
    Opt::slider("viewing_time", "Linger", 0.0, 30.0, 1.0, 0, "5"),
    Opt::slider("x_camera_rotate", "X rotation", 0.0, 1.0, 0.05, 2, "0.2"),
    Opt::slider("y_camera_rotate", "Y rotation", 0.0, 1.0, 0.05, 2, "0.5"),
    Opt::slider("z_camera_rotate", "Z rotation", 0.0, 1.0, 0.05, 2, "0"),
    Opt::boolean("labels", "Draw labels", "true"),
    Opt::boolean("rotate", "Rotate", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "tangram",
    label: "Tangram",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jeremy English",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=JgJ-OsgCCJ4"),
        blurb: "Solves tangram puzzles, an ancient Chinese game of seven \
                pieces cut from a square.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

include!("tangram_solutions.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_figure_places_all_seven_pieces() {
        assert_eq!(SOLUTIONS.len(), 45);
        for s in SOLUTIONS {
            assert_eq!(s.shapes.len(), NUM_SHAPES, "{} is short", s.name);
            assert!(!s.name.is_empty());
            // The board is a few units across; nothing should be off it.
            for p in &s.shapes {
                assert!(
                    p.x.abs() < 10.0 && p.y.abs() < 10.0,
                    "{} is off the board",
                    s.name
                );
                assert!((0..=360).contains(&p.r), "{} turns to {}", s.name, p.r);
            }
        }
    }

    #[test]
    fn a_piece_eases_towards_its_goal_and_then_stops() {
        let goal = TangramShape {
            x: 1.0,
            y: -2.0,
            r: 90,
            fr: 180,
            ..TangramShape::from(&SOLUTIONS[0].shapes[0])
        };
        let mut s = TangramShape {
            x: -3.0,
            y: 3.0,
            r: 0,
            fr: 0,
            solved: false,
            ..goal
        };
        let mut steps = 0;
        while !s.solved && steps < 1000 {
            Tangram::solve(&goal, &mut s);
            steps += 1;
        }
        assert!(s.solved, "it never arrived");
        assert_eq!((s.x, s.y, s.r, s.fr), (goal.x, goal.y, goal.r, goal.fr));
        // A tenth of a unit and two degrees a frame: the rotation is the slow
        // part here, ninety steps of it.
        assert!((88..=95).contains(&steps), "took {steps} frames");
    }

    #[test]
    fn a_piece_bounces_rather_than_sliding() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut lo = f32::MAX;
        let mut hi = 0.0f32;
        for _ in 0..200 {
            r.step();
            let f = r.frame();
            for b in &f.batches {
                // The translation the piece was drawn under: its height is the
                // third component of the modelview's last row.
                hi = hi.max(b.modelview.0[14]);
                lo = lo.min(b.modelview.0[14]);
            }
        }
        assert!(hi - lo > 0.5, "the pieces never left the board, {lo}..{hi}");
    }

    #[test]
    fn a_finished_figure_is_named_and_held() {
        let mut r = start(StartArgs::new(640, 480, "viewing_time=1", 20260811));
        // Run long enough for at least one figure to come together.
        let mut labelled = 0;
        for _ in 0..3000 {
            r.step();
            // The label is drawn in its own colour, which nothing else uses.
            let f = r.frame();
            if f.vertices.iter().any(|v| {
                (v.color[0] - 0.8).abs() < 1e-5
                    && (v.color[1] - 0.8).abs() < 1e-5
                    && v.color[2] < 1e-5
            }) {
                labelled += 1;
            }
        }
        assert!(labelled > 10, "a figure was never named, {labelled} frames");
    }

    #[test]
    fn the_same_figure_is_never_dealt_twice_running() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        // Two figures are dealt at startup, one as the pieces' starting
        // positions and one as their goal, and they must differ.
        let mut seen = std::collections::HashSet::new();
        let mut t = Tangram {
            current: usize::MAX,
            ..bare()
        };
        let mut last = usize::MAX;
        for _ in 0..200 {
            t.deal();
            assert_ne!(t.current, last, "dealt the same figure twice running");
            last = t.current;
            seen.insert(t.current);
        }
        assert!(seen.len() > 20, "only {} of 45 figures came up", seen.len());
    }

    /// A saver with nothing in it, for exercising the dealing on its own.
    fn bare() -> Tangram {
        let blank = TangramShape {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            r: 0,
            fr: 0,
            dz: INIT_DZ,
            ddz: -SPEED,
            solved: false,
            up: true,
        };
        Tangram {
            shapes: [blank; NUM_SHAPES],
            goals: [blank; NUM_SHAPES],
            lists: [0; NUM_SHAPES],
            current: usize::MAX,
            puzzle_name: "",
            shown_name: "",
            display_counter: 0,
            colors: Vec::new(),
            color: [0.5, 0.5, 0.5, 1.0],
            theta: [1.0; 3],
            going_down: [false; 3],
            font_large: None,
            font_medium: None,
            font_small: None,
            wireframe: false,
            do_rotate: true,
            do_labels: true,
            viewing_time: 5,
            camera_rotate: [0.2, 0.5, 0.0],
        }
    }
}
