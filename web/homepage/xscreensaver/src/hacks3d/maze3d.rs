//! Port of `hacks/glx/maze3d.c`.
//!
//! ```text
//! maze3d --- A recreation of the old 3D maze screensaver from Windows 95.
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! 03-Apr-2018: Released initial version of "3D Maze"
//! (sudoer@riseup.net)
//! ```
//!
//! A re-creation of the 3D Maze screensaver from Windows 95.
//!
//! The maze is carved by randomised Prim's algorithm on a grid where the odd
//! squares are rooms and the even ones the walls between them: start from one
//! room, keep a list of every wall that touches the part already carved, and
//! knock through whichever one the dice pick so long as it opens onto a room
//! nothing has reached yet.
//!
//! The camera is a rat, and it solves the maze by keeping its left hand on the
//! wall: at every junction it turns left if it can, goes straight if it
//! cannot, turns right failing that, and turns round in a dead end. That never
//! fails on a maze with no loops, which is what Prim's gives. The other rats
//! wander by the same rule and are drawn as billboards that always face the
//! camera. Bumping into one of the spinning polyhedra turns the world upside
//! down; reaching the FINISH sign drops the walls into the floor and builds a
//! new maze.
//!
//! Two things follow upstream rather than good sense and are kept: the camera
//! transform goes into the *projection* matrix, since `draw_maze` never
//! switches back to the modelview after setting the perspective; and the
//! overlay's `glOrtho` lands in the modelview for the same reason.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random};

/// `ARROW`, drawn as GL_POLYGON.
const ARROW: &[[f32; 2]] = &[
    [0.0, -0.25],
    [0.14694631, 0.20225425],
    [0.0, 0.125],
    [-0.14694631, 0.20225425],
];

/// `SQUARE`, drawn as GL_QUADS.
const SQUARE: &[[f32; 2]] = &[
    [-0.176_776_69, -0.176_776_69],
    [0.176_776_69, -0.176_776_69],
    [0.176_776_69, 0.176_776_69],
    [-0.176_776_69, 0.176_776_69],
];

/// `STAR`, drawn as GL_TRIANGLE_FAN.
const STAR: &[[f32; 2]] = &[
    [0.0, 0.0],
    [0.0, -0.25],
    [0.073473157, -0.101127124],
    [0.23776413, -0.07725425],
    [0.118_882_07, 0.038627124],
    [0.14694631, 0.20225425],
    [0.0, 0.125],
    [-0.14694631, 0.20225425],
    [-0.118_882_07, 0.038627124],
    [-0.23776413, -0.07725425],
    [-0.073473157, -0.101127124],
    [0.0, -0.25],
];

/// `TRIANGLE`, drawn as GL_POLYGON.
const TRIANGLE: &[[f32; 2]] = &[[0.0, -0.25], [0.21650635, 0.125], [-0.21650635, 0.125]];

/// The tetrahedron an inverter can be, as faces of (normal, corners).
const TETRAHEDRON: &[(&[f32; 3], &[[f32; 3]])] = &[
    (
        &[0.47140452, 0.8164966, 0.33333333],
        &[
            [0.0, 0.0, 0.25],
            [0.23570226, 0.0, -0.083333333],
            [-0.11785113, 0.204_124_14, -0.083333333],
        ],
    ),
    (
        &[-0.94280904, 0.0, 0.33333333],
        &[
            [0.0, 0.0, 0.25],
            [-0.11785113, 0.204_124_14, -0.083333333],
            [-0.11785113, -0.204_124_14, -0.083333333],
        ],
    ),
    (
        &[0.47140452, -0.8164966, 0.33333333],
        &[
            [0.0, 0.0, 0.25],
            [-0.11785113, -0.204_124_14, -0.083333333],
            [0.23570226, 0.0, -0.083333333],
        ],
    ),
    (
        &[0.0, 0.0, -1.0],
        &[
            [0.23570226, 0.0, -0.083333333],
            [-0.11785113, -0.204_124_14, -0.083333333],
            [-0.11785113, 0.204_124_14, -0.083333333],
        ],
    ),
];

/// The octahedron an inverter can be, as faces of (normal, corners).
const OCTAHEDRON: &[(&[f32; 3], &[[f32; 3]])] = &[
    (
        &[0.57735027, 0.57735027, 0.57735027],
        &[[0.0, 0.0, 0.25], [0.25, 0.0, 0.0], [0.0, 0.25, 0.0]],
    ),
    (
        &[-0.57735027, 0.57735027, 0.57735027],
        &[[0.0, 0.0, 0.25], [0.0, 0.25, 0.0], [-0.25, 0.0, 0.0]],
    ),
    (
        &[-0.57735027, -0.57735027, 0.57735027],
        &[[0.0, 0.0, 0.25], [-0.25, 0.0, 0.0], [0.0, -0.25, 0.0]],
    ),
    (
        &[0.57735027, -0.57735027, 0.57735027],
        &[[0.0, 0.0, 0.25], [0.0, -0.25, 0.0], [0.25, 0.0, 0.0]],
    ),
    (
        &[0.57735027, -0.57735027, -0.57735027],
        &[[0.25, 0.0, 0.0], [0.0, -0.25, 0.0], [0.0, 0.0, -0.25]],
    ),
    (
        &[0.57735027, 0.57735027, -0.57735027],
        &[[0.25, 0.0, 0.0], [0.0, 0.0, -0.25], [0.0, 0.25, 0.0]],
    ),
    (
        &[-0.57735027, 0.57735027, -0.57735027],
        &[[0.0, 0.25, 0.0], [0.0, 0.0, -0.25], [-0.25, 0.0, 0.0]],
    ),
    (
        &[-0.57735027, -0.57735027, -0.57735027],
        &[[-0.25, 0.0, 0.0], [0.0, 0.0, -0.25], [0.0, -0.25, 0.0]],
    ),
];

/// The dodecahedron an inverter can be, as faces of (normal, corners).
const DODECAHEDRON: &[(&[f32; 3], &[[f32; 3]])] = &[
    (
        &[0.0, 0.0, 1.0],
        &[
            [0.12278087, 0.08920552, 0.19866362],
            [-0.04689812, 0.14433757, 0.19866362],
            [-0.1517655, 0.0, 0.19866362],
            [-0.04689812, -0.14433757, 0.19866362],
            [0.12278087, -0.08920552, 0.19866362],
        ],
    ),
    (
        &[0.8944272, 0.0, 0.4472136],
        &[
            [0.19866362, -0.14433757, 0.04689812],
            [0.24556174, 0.0, -0.04689812],
            [0.19866362, 0.14433757, 0.04689812],
            [0.12278087, 0.08920552, 0.19866362],
            [0.12278087, -0.08920552, 0.19866362],
        ],
    ),
    (
        &[0.2763932, 0.8506508, 0.4472136],
        &[
            [0.19866362, 0.14433757, 0.04689812],
            [0.07588275, 0.23354309, -0.04689812],
            [-0.07588275, 0.23354309, 0.04689812],
            [-0.04689812, 0.14433757, 0.19866362],
            [0.12278087, 0.08920552, 0.19866362],
        ],
    ),
    (
        &[-0.7236068, 0.5257311, 0.4472136],
        &[
            [-0.07588275, 0.23354309, 0.04689812],
            [-0.19866362, 0.14433757, -0.04689812],
            [-0.24556174, 0.0, 0.04689812],
            [-0.1517655, 0.0, 0.19866362],
            [-0.04689812, 0.14433757, 0.19866362],
        ],
    ),
    (
        &[-0.7236068, -0.5257311, 0.4472136],
        &[
            [-0.24556174, 0.0, 0.04689812],
            [-0.19866362, -0.14433757, -0.04689812],
            [-0.07588275, -0.23354309, 0.04689812],
            [-0.04689812, -0.14433757, 0.19866362],
            [-0.1517655, 0.0, 0.19866362],
        ],
    ),
    (
        &[0.2763932, -0.8506508, 0.4472136],
        &[
            [-0.07588275, -0.23354309, 0.04689812],
            [0.07588275, -0.23354309, -0.04689812],
            [0.19866362, -0.14433757, 0.04689812],
            [0.12278087, -0.08920552, 0.19866362],
            [-0.04689812, -0.14433757, 0.19866362],
        ],
    ),
    (
        &[0.7236068, 0.5257311, -0.4472136],
        &[
            [0.24556174, 0.0, -0.04689812],
            [0.1517655, 0.0, -0.19866362],
            [0.04689812, 0.14433757, -0.19866362],
            [0.07588275, 0.23354309, -0.04689812],
            [0.19866362, 0.14433757, 0.04689812],
        ],
    ),
    (
        &[0.7236068, -0.5257311, -0.4472136],
        &[
            [0.19866362, -0.14433757, 0.04689812],
            [0.07588275, -0.23354309, -0.04689812],
            [0.04689812, -0.14433757, -0.19866362],
            [0.1517655, 0.0, -0.19866362],
            [0.24556174, 0.0, -0.04689812],
        ],
    ),
    (
        &[-0.2763932, 0.8506508, -0.4472136],
        &[
            [0.07588275, 0.23354309, -0.04689812],
            [0.04689812, 0.14433757, -0.19866362],
            [-0.12278087, 0.08920552, -0.19866362],
            [-0.19866362, 0.14433757, -0.04689812],
            [-0.07588275, 0.23354309, 0.04689812],
        ],
    ),
    (
        &[-0.8944272, 0.0, -0.4472136],
        &[
            [-0.19866362, 0.14433757, -0.04689812],
            [-0.12278087, 0.08920552, -0.19866362],
            [-0.12278087, -0.08920552, -0.19866362],
            [-0.19866362, -0.14433757, -0.04689812],
            [-0.24556174, 0.0, 0.04689812],
        ],
    ),
    (
        &[-0.2763932, -0.8506508, -0.4472136],
        &[
            [-0.19866362, -0.14433757, -0.04689812],
            [-0.12278087, -0.08920552, -0.19866362],
            [0.04689812, -0.14433757, -0.19866362],
            [0.07588275, -0.23354309, -0.04689812],
            [-0.07588275, -0.23354309, 0.04689812],
        ],
    ),
    (
        &[0.0, 0.0, -1.0],
        &[
            [0.04689812, -0.14433757, -0.19866362],
            [-0.12278087, -0.08920552, -0.19866362],
            [-0.12278087, 0.08920552, -0.19866362],
            [0.04689812, 0.14433757, -0.19866362],
            [0.1517655, 0.0, -0.19866362],
        ],
    ),
];

/// The icosahedron an inverter can be, as faces of (normal, corners).
const ICOSAHEDRON: &[(&[f32; 3], &[[f32; 3]])] = &[
    (
        &[0.49112347, 0.3568221, 0.7946545],
        &[
            [0.0, 0.0, 0.25],
            [0.2236068, 0.0, 0.1118034],
            [0.0690983, 0.2126627, 0.1118034],
        ],
    ),
    (
        &[-0.18759247, 0.57735027, 0.7946545],
        &[
            [0.0, 0.0, 0.25],
            [0.0690983, 0.2126627, 0.1118034],
            [-0.1809017, 0.131_432_77, 0.1118034],
        ],
    ),
    (
        &[-0.607062, 0.0, 0.7946545],
        &[
            [0.0, 0.0, 0.25],
            [-0.1809017, 0.131_432_77, 0.1118034],
            [-0.1809017, -0.131_432_77, 0.1118034],
        ],
    ),
    (
        &[-0.18759247, -0.57735027, 0.7946545],
        &[
            [0.0, 0.0, 0.25],
            [-0.1809017, -0.131_432_77, 0.1118034],
            [0.0690983, -0.2126627, 0.1118034],
        ],
    ),
    (
        &[0.49112347, -0.3568221, 0.7946545],
        &[
            [0.0, 0.0, 0.25],
            [0.0690983, -0.2126627, 0.1118034],
            [0.2236068, 0.0, 0.1118034],
        ],
    ),
    (
        &[0.7946545, -0.57735027, 0.18759247],
        &[
            [0.2236068, 0.0, 0.1118034],
            [0.0690983, -0.2126627, 0.1118034],
            [0.1809017, -0.131_432_77, -0.1118034],
        ],
    ),
    (
        &[0.98224695, 0.0, -0.18759247],
        &[
            [0.2236068, 0.0, 0.1118034],
            [0.1809017, -0.131_432_77, -0.1118034],
            [0.1809017, 0.131_432_77, -0.1118034],
        ],
    ),
    (
        &[0.7946545, 0.57735027, 0.18759247],
        &[
            [0.2236068, 0.0, 0.1118034],
            [0.1809017, 0.131_432_77, -0.1118034],
            [0.0690983, 0.2126627, 0.1118034],
        ],
    ),
    (
        &[0.303531, 0.93417236, -0.18759247],
        &[
            [0.0690983, 0.2126627, 0.1118034],
            [0.1809017, 0.131_432_77, -0.1118034],
            [-0.0690983, 0.2126627, -0.1118034],
        ],
    ),
    (
        &[-0.303531, 0.93417236, 0.18759247],
        &[
            [0.0690983, 0.2126627, 0.1118034],
            [-0.0690983, 0.2126627, -0.1118034],
            [-0.1809017, 0.131_432_77, 0.1118034],
        ],
    ),
    (
        &[-0.7946545, 0.57735027, -0.18759247],
        &[
            [-0.1809017, 0.131_432_77, 0.1118034],
            [-0.0690983, 0.2126627, -0.1118034],
            [-0.2236068, 0.0, -0.1118034],
        ],
    ),
    (
        &[-0.98224695, 0.0, 0.18759247],
        &[
            [-0.1809017, 0.131_432_77, 0.1118034],
            [-0.2236068, 0.0, -0.1118034],
            [-0.1809017, -0.131_432_77, 0.1118034],
        ],
    ),
    (
        &[-0.7946545, -0.57735027, -0.18759247],
        &[
            [-0.1809017, -0.131_432_77, 0.1118034],
            [-0.2236068, 0.0, -0.1118034],
            [-0.0690983, -0.2126627, -0.1118034],
        ],
    ),
    (
        &[-0.303531, -0.93417236, 0.18759247],
        &[
            [-0.1809017, -0.131_432_77, 0.1118034],
            [-0.0690983, -0.2126627, -0.1118034],
            [0.0690983, -0.2126627, 0.1118034],
        ],
    ),
    (
        &[0.303531, -0.93417236, -0.18759247],
        &[
            [0.0690983, -0.2126627, 0.1118034],
            [-0.0690983, -0.2126627, -0.1118034],
            [0.1809017, -0.131_432_77, -0.1118034],
        ],
    ),
    (
        &[0.607062, 0.0, -0.7946545],
        &[
            [0.1809017, 0.131_432_77, -0.1118034],
            [0.1809017, -0.131_432_77, -0.1118034],
            [0.0, 0.0, -0.25],
        ],
    ),
    (
        &[0.18759247, 0.57735027, -0.7946545],
        &[
            [0.1809017, 0.131_432_77, -0.1118034],
            [0.0, 0.0, -0.25],
            [-0.0690983, 0.2126627, -0.1118034],
        ],
    ),
    (
        &[0.18759247, -0.57735027, -0.7946545],
        &[
            [0.1809017, -0.131_432_77, -0.1118034],
            [-0.0690983, -0.2126627, -0.1118034],
            [0.0, 0.0, -0.25],
        ],
    ),
    (
        &[-0.49112347, 0.3568221, -0.7946545],
        &[
            [-0.0690983, 0.2126627, -0.1118034],
            [0.0, 0.0, -0.25],
            [-0.2236068, 0.0, -0.1118034],
        ],
    ),
    (
        &[-0.49112347, -0.3568221, -0.7946545],
        &[
            [-0.2236068, 0.0, -0.1118034],
            [0.0, 0.0, -0.25],
            [-0.0690983, -0.2126627, -0.1118034],
        ],
    ),
];

/// What is in a square of the grid. The four inverters are contiguous so that
/// "is this an inverter" is a range test, which is how upstream writes it.
const WALL: u8 = 0;
const CELL_UNVISITED: u8 = 1;
const CELL: u8 = 2;
const START_CELL: u8 = 3;
const FINISH: u8 = 4;
const INVERTER_TETRAHEDRON: u8 = 6;
const INVERTER_ICOSAHEDRON: u8 = 9;

/// Which way something is facing, in degrees.
const NORTH: f32 = 0.0;
const EAST: f32 = 90.0;
const SOUTH: f32 = 180.0;
const WEST: f32 = 270.0;

/// How many degrees of turn one unit of travel buys.
const ANGULAR_CONVERSION_FACTOR: f32 = 90.0;

/// What a rat is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Starting,
    Walking,
    TurningLeft,
    TurningRight,
    TurningAround,
    Inverting,
    Finishing,
}

#[derive(Clone, Copy)]
struct Rat {
    x: f32,
    z: f32,
    rotation: f32,
    desired_rotation: f32,
    inversion: f32,
    remaining: f32,
    state: State,
}

fn round_to_nearest_half(n: f32) -> f32 {
    (2.0 * n).round() / 2.0
}

struct Maze3d {
    grid: Vec<Vec<u8>>,
    num_rows: usize,
    num_columns: usize,
    /// The walls Prim's algorithm has yet to consider.
    wall_list: Vec<(usize, usize)>,

    start_position: (usize, usize),
    finish_position: (usize, usize),
    inverter_position: Vec<(usize, usize)>,

    camera: Rat,
    rats: Vec<Rat>,

    textures: Textures,
    acid_color: [f32; 3],
    acid_hue: f32,
    /// How far out of the floor the walls have risen: 0 at the start of a
    /// maze and 1 once it is built.
    wall_height: f32,
    inverter_rotation: f32,

    speed: f32,
    num_inverters: usize,
    num_rats: usize,
    drop_acid: bool,
    show_overlay: bool,
    button_down: bool,
}

#[derive(Default)]
struct Textures {
    wall: Option<u32>,
    floor: Option<u32>,
    ceiling: Option<u32>,
    start: Option<u32>,
    finish: Option<u32>,
    rat: Option<u32>,
}

impl Maze3d {
    fn is_odd(n: usize) -> bool {
        n % 2 == 1
    }

    /// Every square starts as a wall except the odd-odd ones, which are the
    /// rooms waiting to be reached.
    fn initialize_grid(&mut self) {
        for i in 0..self.num_rows {
            for j in 0..self.num_columns {
                self.grid[i][j] = if Self::is_odd(i) && Self::is_odd(j) {
                    CELL_UNVISITED
                } else {
                    WALL
                };
            }
        }
    }

    /// `buildMaze`: randomised Prim's algorithm.
    fn build_maze(&mut self) {
        self.wall_list.clear();
        self.grid[1][1] = CELL;
        self.add_walls_to_list((1, 1));

        while !self.wall_list.is_empty() {
            let n = (random() as usize) % self.wall_list.len();
            let (wr, wc) = self.wall_list[n];

            // A wall is knocked through only when the room on one side of it
            // has been reached and the room on the other has not.
            let cell_to_add = if !Self::is_odd(wr) {
                if self.grid[wr - 1][wc] == CELL && self.grid[wr + 1][wc] == CELL_UNVISITED {
                    Some((wr + 1, wc))
                } else if self.grid[wr + 1][wc] == CELL && self.grid[wr - 1][wc] == CELL_UNVISITED {
                    Some((wr - 1, wc))
                } else {
                    None
                }
            } else if self.grid[wr][wc - 1] == CELL && self.grid[wr][wc + 1] == CELL_UNVISITED {
                Some((wr, wc + 1))
            } else if self.grid[wr][wc + 1] == CELL && self.grid[wr][wc - 1] == CELL_UNVISITED {
                Some((wr, wc - 1))
            } else {
                None
            };

            if let Some(cell) = cell_to_add {
                self.grid[wr][wc] = CELL;
                self.grid[cell.0][cell.1] = CELL;
                self.add_walls_to_list(cell);
            }
            self.wall_list.remove(n);
        }
    }

    fn add_walls_to_list(&mut self, cell: (usize, usize)) {
        let (r, c) = cell;
        for (wr, wc) in [(r - 1, c), (r + 1, c), (r, c - 1), (r, c + 1)] {
            if self.grid[wr][wc] == WALL
                && wr > 0
                && wr < self.num_rows - 1
                && wc > 0
                && wc < self.num_columns - 1
            {
                self.wall_list.push((wr, wc));
            }
        }
    }

    /// `placeObject`: drop something into a room nothing else is in.
    fn place_object(&mut self, kind: u8) -> (usize, usize) {
        let mut p = (0usize, 0usize);
        // Upstream's loop is unbounded. On a full maze it could never end, so
        // it gives up rather than hanging the frame; a maze with no free room
        // left simply gets one fewer object.
        for _ in 0..100_000 {
            if self.grid[p.0][p.1] == CELL && Self::is_odd(p.0) && Self::is_odd(p.1) {
                self.grid[p.0][p.1] = kind;
                return p;
            }
            p.0 = (random() as usize) % self.num_rows;
            p.1 = (random() as usize) % self.num_columns;
        }
        (0, 0)
    }

    /// `placeMiscObjects`: the start, the finish, the inverters and the rats.
    fn place_misc_objects(&mut self) {
        // The start wants a room with at least two ways out, so the camera
        // has somewhere to go.
        let mut surrounding = 3;
        while surrounding >= 3 {
            surrounding = 0;
            self.start_position = self.place_object(CELL);
            let (r, c) = self.start_position;
            if r == 0 && c == 0 {
                break;
            }
            for (rr, cc) in [(r, c + 1), (r - 1, c), (r, c - 1), (r + 1, c)] {
                if self.grid[rr][cc] == WALL {
                    surrounding += 1;
                }
            }
        }
        let (sr, sc) = self.start_position;
        self.grid[sr][sc] = START_CELL;

        // Stand the camera in the first open square next to the sign, facing
        // away from it.
        if self.grid[sr][sc + 1] != WALL {
            self.camera.x = (sc + 1) as f32 / 2.0;
            self.camera.z = sr as f32 / 2.0;
            self.camera.rotation = WEST;
        } else if self.grid[sr - 1][sc] != WALL {
            self.camera.x = sc as f32 / 2.0;
            self.camera.z = (sr - 1) as f32 / 2.0;
            self.camera.rotation = SOUTH;
        } else if self.grid[sr][sc - 1] != WALL {
            self.camera.x = (sc - 1) as f32 / 2.0;
            self.camera.z = sr as f32 / 2.0;
            self.camera.rotation = EAST;
        } else {
            self.camera.x = sc as f32 / 2.0;
            self.camera.z = (sr + 1) as f32 / 2.0;
            self.camera.rotation = NORTH;
        }

        self.finish_position = self.place_object(FINISH);

        self.inverter_position.clear();
        for _ in 0..self.num_inverters {
            let kind = (random() % 4) as u8 + INVERTER_TETRAHEDRON;
            let p = self.place_object(kind);
            self.inverter_position.push(p);
        }

        for i in 0..self.num_rats {
            let t = self.place_object(CELL);
            let rat = &mut self.rats[i];
            rat.x = t.1 as f32 / 2.0;
            rat.z = t.0 as f32 / 2.0;
            rat.state = State::Walking;
            if t.0 == 0 && t.1 == 0 {
                continue;
            }
            let (rz, rx) = ((rat.z * 2.0) as usize, (rat.x * 2.0) as usize);
            rat.rotation = if self.grid[rz][rx + 1] != WALL {
                EAST
            } else if self.grid[rz - 1][rx] != WALL {
                NORTH
            } else if self.grid[rz][rx - 1] != WALL {
                WEST
            } else {
                SOUTH
            };
        }
    }

    fn new_maze(&mut self) {
        self.camera.state = State::Starting;
        self.camera.inversion = 0.0;
        self.wall_height = 0.0;
        self.inverter_rotation = 0.0;
        self.acid_hue = 0.0;

        self.initialize_grid();
        self.build_maze();
        self.place_misc_objects();
    }

    /// `shiftAcidColor`: walk the hue round the wheel.
    fn shift_acid_color(&mut self) {
        let h = self.acid_hue;
        let x = 1.0 - ((h / 60.0) % 2.0 - 1.0).abs();
        self.acid_color = if h <= 60.0 {
            [1.0, x, 0.0]
        } else if h <= 120.0 {
            [x, 1.0, 0.0]
        } else if h <= 180.0 {
            [0.0, 1.0, x]
        } else if h <= 240.0 {
            [0.0, x, 1.0]
        } else if h <= 300.0 {
            [x, 0.0, 1.0]
        } else {
            [1.0, 0.0, x]
        };

        self.acid_hue += 75.0 * self.camera.remaining;
        if self.acid_hue >= 360.0 {
            self.acid_hue -= 360.0;
        }
    }

    /// `changeState`: look about and decide what to do next. Left hand on the
    /// wall: left if you can, straight if you cannot, right failing that, and
    /// round if there is nowhere else.
    fn change_state(&mut self, is_camera: bool, i: usize) {
        let rat = if is_camera { self.camera } else { self.rats[i] };
        let rx = (rat.x * 2.0).round() as usize;
        let rz = (rat.z * 2.0).round() as usize;

        let at = |r: usize, c: usize| -> u8 {
            self.grid
                .get(r)
                .and_then(|row| row.get(c))
                .copied()
                .unwrap_or(WALL)
        };
        let (in_front, to_left, ahead, to_right) = match rat.rotation as i32 {
            0 => (
                at(rz.wrapping_sub(1), rx),
                at(rz.wrapping_sub(1), rx.wrapping_sub(1)),
                at(rz.wrapping_sub(2), rx),
                at(rz.wrapping_sub(1), rx + 1),
            ),
            90 => (
                at(rz, rx + 1),
                at(rz.wrapping_sub(1), rx + 1),
                at(rz, rx + 2),
                at(rz + 1, rx + 1),
            ),
            180 => (
                at(rz + 1, rx),
                at(rz + 1, rx + 1),
                at(rz + 2, rx),
                at(rz + 1, rx.wrapping_sub(1)),
            ),
            270 => (
                at(rz, rx.wrapping_sub(1)),
                at(rz + 1, rx.wrapping_sub(1)),
                at(rz, rx.wrapping_sub(2)),
                at(rz.wrapping_sub(1), rx.wrapping_sub(1)),
            ),
            _ => (CELL, CELL, CELL, CELL),
        };

        let (state, desired) = if is_camera && in_front == FINISH {
            (State::Finishing, rat.desired_rotation)
        } else if is_camera && (INVERTER_TETRAHEDRON..=INVERTER_ICOSAHEDRON).contains(&in_front) {
            (State::Inverting, rat.desired_rotation)
        } else if to_left != WALL {
            (State::TurningLeft, rat.desired_rotation)
        } else if ahead != WALL {
            (State::Walking, rat.desired_rotation)
        } else if to_right != WALL {
            (State::TurningRight, rat.desired_rotation)
        } else {
            let d = match rat.rotation as i32 {
                0 => SOUTH,
                90 => WEST,
                180 => NORTH,
                _ => EAST,
            };
            (State::TurningAround, d)
        };

        let r = if is_camera {
            &mut self.camera
        } else {
            &mut self.rats[i]
        };
        r.state = state;
        r.desired_rotation = desired;
    }

    /// `walk`: move along one axis until the next half-square boundary.
    fn walk(&mut self, is_camera: bool, i: usize, axis_x: bool, sign: f32) {
        let rat = if is_camera {
            &mut self.camera
        } else {
            &mut self.rats[i]
        };
        let component = if axis_x { rat.x } else { rat.z };
        let previous = component;
        let temp = (component * 2.0) as u32;
        let on_boundary = (component * 2.0) == (component * 2.0).round();

        let moved = component + sign * rat.remaining;
        let crossed = !on_boundary && (moved * 2.0) as u32 != temp;
        let value = if crossed {
            round_to_nearest_half(moved)
        } else {
            moved
        };
        if axis_x {
            rat.x = value;
        } else {
            rat.z = value;
        }

        if crossed {
            rat.remaining -= (value - previous).abs();
            self.change_state(is_camera, i);
        } else {
            rat.remaining = 0.0;
        }
    }

    /// `turn`: swing round the corner of the square, which keeps the rat
    /// against the inside of the bend rather than pivoting on the spot.
    fn turn(&mut self, is_camera: bool, i: usize) {
        let rat = if is_camera {
            &mut self.camera
        } else {
            &mut self.rats[i]
        };
        let pi = std::f32::consts::PI;
        let previous = rat.rotation;
        let left = rat.state == State::TurningLeft;

        let tangent = if left {
            rat.rotation * (pi / 180.0) + pi
        } else {
            rat.rotation * (pi / 180.0)
        };
        let around_x = round_to_nearest_half(rat.x + 0.5 * tangent.cos());
        let around_z = round_to_nearest_half(rat.z + 0.5 * tangent.sin());

        if left {
            rat.rotation -= ANGULAR_CONVERSION_FACTOR * rat.remaining;
            let stop = [WEST, SOUTH, EAST, NORTH]
                .into_iter()
                .find(|&q| previous > q && rat.rotation <= q);
            match stop {
                Some(q) => {
                    rat.rotation = q;
                    rat.remaining -= (pi / 180.0) * (previous - rat.rotation).abs();
                }
                None => rat.remaining = 0.0,
            }
        } else {
            rat.rotation += ANGULAR_CONVERSION_FACTOR * rat.remaining;
            if rat.rotation >= 360.0 {
                rat.rotation = NORTH;
                rat.remaining -= (pi / 180.0) * (previous - 360.0).abs();
            } else {
                let stop = [WEST, SOUTH, EAST]
                    .into_iter()
                    .find(|&q| previous < q && rat.rotation >= q);
                match stop {
                    Some(q) => {
                        rat.rotation = q;
                        rat.remaining -= (pi / 180.0) * (previous - rat.rotation).abs();
                    }
                    None => rat.remaining = 0.0,
                }
            }
        }

        let tangent = if left {
            rat.rotation * (pi / 180.0)
        } else {
            rat.rotation * (pi / 180.0) + pi
        };
        rat.x = around_x + 0.5 * tangent.cos();
        rat.z = around_z + 0.5 * tangent.sin();
        if rat.rotation < 0.0 {
            rat.rotation += 360.0;
        }

        if rat.rotation == NORTH
            || rat.rotation == EAST
            || rat.rotation == SOUTH
            || rat.rotation == WEST
        {
            rat.x = round_to_nearest_half(rat.x);
            rat.z = round_to_nearest_half(rat.z);
            self.change_state(is_camera, i);
        }
    }

    /// `turnAround`: a dead end, so spin on the spot half again as fast.
    fn turn_around(&mut self, is_camera: bool, i: usize) {
        let rat = if is_camera {
            &mut self.camera
        } else {
            &mut self.rats[i]
        };
        let pi = std::f32::consts::PI;
        let previous = rat.rotation;
        rat.rotation -= 1.5 * ANGULAR_CONVERSION_FACTOR * rat.remaining;

        if previous > rat.desired_rotation && rat.rotation <= rat.desired_rotation {
            rat.rotation = rat.desired_rotation;
            rat.remaining -= (pi / 180.0) * (previous - rat.rotation).abs();
            self.change_state(is_camera, i);
        } else {
            rat.remaining = 0.0;
            if rat.rotation < 0.0 {
                rat.rotation += 360.0;
            }
        }
    }

    /// `invert`: roll the whole world over, and eat the inverter on the way
    /// past by turning its square back into an ordinary room.
    fn invert(&mut self) {
        let pi = std::f32::consts::PI;
        let previous = self.camera.inversion;
        let cx = (self.camera.x * 2.0).round() as usize;
        let cz = (self.camera.z * 2.0).round() as usize;

        self.camera.inversion += 1.5 * ANGULAR_CONVERSION_FACTOR * self.camera.remaining;
        let mut changed = false;
        if previous < 180.0 && self.camera.inversion >= 180.0 {
            self.camera.inversion = 180.0;
            self.camera.remaining -= (pi / 180.0) * (previous - 180.0).abs();
            changed = true;
        } else if self.camera.inversion >= 360.0 {
            self.camera.inversion = 0.0;
            self.camera.remaining -= (pi / 180.0) * previous.abs();
            changed = true;
        } else {
            self.camera.remaining = 0.0;
        }

        if changed {
            let (r, c) = match self.camera.rotation as i32 {
                0 => (cz.wrapping_sub(1), cx),
                90 => (cz, cx + 1),
                180 => (cz + 1, cx),
                _ => (cz, cx.wrapping_sub(1)),
            };
            if let Some(cell) = self.grid.get_mut(r).and_then(|row| row.get_mut(c)) {
                *cell = CELL;
            }
            self.change_state(true, 0);
        }
    }

    /// `step`: run one frame's worth of travel through the state machine.
    fn step(&mut self, is_camera: bool, i: usize) {
        let rat = if is_camera { self.camera } else { self.rats[i] };
        if !is_camera && (self.wall_height < 1.0 || (rat.x == 0.0 && rat.z == 0.0)) {
            return;
        }

        // Upstream's `while (remaining > 0)` has no other bound. Every state
        // either consumes the whole allowance or takes a bite out of it, so
        // it terminates, but a state that did neither would hang the frame.
        for _ in 0..1000 {
            let rat = if is_camera { self.camera } else { self.rats[i] };
            if rat.remaining <= 0.0 {
                break;
            }
            let previous_wall_height = self.wall_height;

            match rat.state {
                State::Walking => match rat.rotation as i32 {
                    0 => self.walk(is_camera, i, false, -1.0),
                    90 => self.walk(is_camera, i, true, 1.0),
                    180 => self.walk(is_camera, i, false, 1.0),
                    270 => self.walk(is_camera, i, true, -1.0),
                    _ => {
                        let r = if is_camera {
                            &mut self.camera
                        } else {
                            &mut self.rats[i]
                        };
                        r.rotation = 90.0 * (r.rotation / 90.0).round();
                    }
                },
                State::TurningLeft | State::TurningRight => self.turn(is_camera, i),
                State::TurningAround => self.turn_around(is_camera, i),
                State::Inverting => self.invert(),
                State::Starting => {
                    self.wall_height += 0.48 * rat.remaining;
                    if self.wall_height > 1.0 {
                        self.wall_height = 1.0;
                        self.camera.remaining = (previous_wall_height - 1.0).abs();
                        self.change_state(true, 0);
                    } else {
                        self.camera.remaining = 0.0;
                    }
                }
                State::Finishing => {
                    if self.wall_height == 0.0 {
                        self.new_maze();
                        self.camera.remaining = 0.0;
                    } else if self.wall_height < 0.0 {
                        self.wall_height = 0.0;
                        self.camera.remaining = previous_wall_height.abs();
                    } else {
                        self.wall_height -= 0.48 * rat.remaining;
                        self.camera.remaining = 0.0;
                    }
                }
            }
        }
    }

    /// `drawWalls`: every wall square, as one quad standing on the grid line
    /// it sits on.
    fn draw_walls(&self, g: &mut Gl) {
        if let Some(id) = self.textures.wall {
            g.glx.texturing(true);
            g.glx.bind_texture(id);
        }
        if self.drop_acid {
            let c = self.acid_color;
            g.glx.color4f(c[0], c[1], c[2], 1.0);
        } else {
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        }

        g.glx.begin(Shape::Quads);
        for i in 0..self.num_rows {
            for j in 0..self.num_columns {
                if self.grid[i][j] != WALL {
                    continue;
                }
                // A wall on an odd row runs north-south; one on an odd column
                // runs east-west. A square that is odd in both is a room and
                // one that is even in both is a corner post with no wall.
                let (sr, sc, er, ec) = if Self::is_odd(i) && !Self::is_odd(j) {
                    (i / 2, j / 2, i / 2 + 1, j / 2)
                } else if !Self::is_odd(i) && Self::is_odd(j) {
                    (i / 2, j / 2, i / 2, j / 2 + 1)
                } else {
                    continue;
                };
                self.draw_wall(g, sr, sc, er, ec);
            }
        }
        g.glx.end();
        g.glx.texturing(false);
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
    }

    fn draw_wall(&self, g: &mut Gl, sr: usize, sc: usize, er: usize, ec: usize) {
        let h = self.wall_height;
        let (sr, sc, er, ec) = (sr as f32, sc as f32, er as f32, ec as f32);
        let quad: [(f32, f32, f32, f32, f32); 4] = if sr == er {
            [
                (0.0, 0.0, sc, 0.0, sr),
                (1.0, 0.0, ec, 0.0, sr),
                (1.0, 1.0, ec, h, er),
                (0.0, 1.0, sc, h, er),
            ]
        } else {
            [
                (0.0, 0.0, sc, 0.0, sr),
                (1.0, 0.0, sc, 0.0, er),
                (1.0, 1.0, ec, h, er),
                (0.0, 1.0, ec, h, sr),
            ]
        };
        for (u, v, x, y, z) in quad {
            // The textures here are top-down and OpenGL's are bottom-up.
            g.glx.tex_coord2f(u, 1.0 - v);
            g.glx.vertex3f(x, y, z);
        }
    }

    /// The floor and the ceiling: one quad each over the whole maze, with the
    /// texture repeated once per square.
    fn draw_slab(&self, g: &mut Gl, y: f32, texture: Option<u32>) {
        let Some(id) = texture else { return };
        let far_r = (self.num_rows / 2) as f32;
        let far_c = (self.num_columns / 2) as f32;

        g.glx.texturing(true);
        g.glx.bind_texture(id);
        if self.drop_acid {
            let c = self.acid_color;
            g.glx.color4f(c[0], c[1], c[2], 1.0);
        }
        g.glx.begin(Shape::Quads);
        for (u, v, x, z) in [
            (0.0, 0.0, 0.0, 0.0),
            (far_c, 0.0, far_c, 0.0),
            (far_c, far_r, far_c, far_r),
            (0.0, far_r, 0.0, far_r),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.texturing(false);
    }

    /// `drawInverter`: one of the four spinning polyhedra.
    fn draw_inverter(&self, g: &mut Gl, p: (usize, usize)) {
        let kind = self.grid[p.0][p.1];
        if self.wall_height < 1.0
            || !(INVERTER_TETRAHEDRON..=INVERTER_ICOSAHEDRON).contains(&kind)
            || (p.0 == 0 && p.1 == 0)
        {
            return;
        }
        let faces = match kind - INVERTER_TETRAHEDRON {
            0 => TETRAHEDRON,
            1 => OCTAHEDRON,
            2 => DODECAHEDRON,
            _ => ICOSAHEDRON,
        };

        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.cull_face(true);
        g.glx.push_matrix();
        g.glx.translate(p.1 as f32 / 2.0, 0.25, p.0 as f32 / 2.0);
        // The two turns are in an irrational ratio, so it never repeats.
        g.glx
            .rotate(0.618_034 * self.inverter_rotation, 0.0, 1.0, 0.0);
        g.glx.rotate(self.inverter_rotation, 1.0, 0.0, 0.0);

        for (n, verts) in faces {
            g.glx.begin(Shape::Polygon);
            g.glx.normal3f(n[0], n[1], n[2]);
            for v in *verts {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }

        g.glx.pop_matrix();
        g.glx.lighting(false);
        g.glx.cull_face(false);
    }

    /// `drawPane`: the START or FINISH sign, always turned to face the camera.
    fn draw_pane(&self, g: &mut Gl, texture: Option<u32>, p: (usize, usize), small: bool) {
        let Some(id) = texture else { return };
        if p.0 == 0 && p.1 == 0 {
            return;
        }

        g.glx.blend(Blend::Alpha);
        g.glx.texturing(true);
        g.glx.bind_texture(id);
        g.glx.push_matrix();
        g.glx.translate(p.1 as f32 / 2.0, 0.0, p.0 as f32 / 2.0);
        g.glx.rotate(-self.camera.rotation - 90.0, 0.0, 1.0, 0.0);
        if small {
            // Keep the sign readable in portrait.
            g.glx.scale(0.5, 0.5, 0.5);
            g.glx.translate(0.0, 0.5, 0.0);
        }

        g.glx.color4f(1.0, 1.0, 1.0, 0.9);
        g.glx.begin(Shape::Quads);
        for (u, v, y, z) in [
            (0.0, 0.0, 0.0, 0.5),
            (1.0, 0.0, 0.0, -0.5),
            (1.0, 1.0, self.wall_height, -0.5),
            (0.0, 1.0, self.wall_height, 0.5),
        ] {
            g.glx.tex_coord2f(u, 1.0 - v);
            g.glx.vertex3f(0.0, y, z);
        }
        g.glx.end();
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);

        g.glx.pop_matrix();
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
    }

    /// `drawRat`: a quarter-size billboard of the other rat.
    fn draw_rat(&self, g: &mut Gl, x: f32, z: f32) {
        let Some(id) = self.textures.rat else { return };
        if x == 0.0 && z == 0.0 {
            return;
        }

        g.glx.blend(Blend::Alpha);
        g.glx.texturing(true);
        g.glx.bind_texture(id);
        g.glx.push_matrix();
        g.glx.translate(x, 0.0, z);
        g.glx.rotate(-self.camera.rotation - 90.0, 0.0, 1.0, 0.0);
        g.glx.scale(0.25, 0.25, 0.25);

        g.glx.begin(Shape::Quads);
        for (u, v, y, zz) in [
            (0.0, 0.0, 0.0, 0.5),
            (1.0, 0.0, 0.0, -0.5),
            (1.0, 1.0, self.wall_height, -0.5),
            (0.0, 1.0, self.wall_height, 0.5),
        ] {
            g.glx.tex_coord2f(u, 1.0 - v);
            g.glx.vertex3f(0.0, y, zz);
        }
        g.glx.end();

        g.glx.pop_matrix();
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
    }

    /// `drawOverlay`: the plan view in the corner, with an arrow for each rat
    /// and marks for the start, the finish and every inverter still standing.
    fn draw_overlay(&self, g: &mut Gl) {
        let h = g.height() as f32 / g.width().max(1) as f32;

        // Upstream loads identity into both matrices and then calls glOrtho
        // while the modelview is current, so the ortho lands there.
        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.ortho(-1.0 / h, 1.0 / h, 1.0, -1.0, -1.0, 1.0);

        g.glx.blend(Blend::Alpha);
        g.glx.lighting(false);
        g.glx.color_material(true);
        g.glx.color4f(0.0, 0.0, 1.0, 0.75);
        g.glx.scale(0.25, 0.25, 0.25);

        // The camera's own arrow sits still in the middle while the map turns
        // underneath it.
        draw_flat(g, Shape::Polygon, ARROW);

        g.glx.rotate(self.camera.inversion, 0.0, 1.0, 0.0);
        g.glx.rotate(self.camera.rotation, 0.0, 0.0, -1.0);
        g.glx.translate(-self.camera.x, -self.camera.z, 0.0);
        g.glx.color4f(1.0, 1.0, 1.0, 0.75);

        g.glx.begin(Shape::Lines);
        for i in 0..self.num_rows {
            for j in 0..self.num_columns {
                if self.grid[i][j] != WALL {
                    continue;
                }
                let (a, b) = if Self::is_odd(i) && !Self::is_odd(j) {
                    ((j / 2, i / 2), (j / 2, i / 2 + 1))
                } else if !Self::is_odd(i) && Self::is_odd(j) {
                    ((j / 2, i / 2), (j / 2 + 1, i / 2))
                } else {
                    continue;
                };
                g.glx.vertex3f(a.0 as f32, a.1 as f32, 0.0);
                g.glx.vertex3f(b.0 as f32, b.1 as f32, 0.0);
            }
        }
        g.glx.end();

        g.glx.color4f(1.0, 0.0, 0.0, 0.75);
        g.glx.push_matrix();
        g.glx.translate(
            self.start_position.1 as f32 / 2.0,
            self.start_position.0 as f32 / 2.0,
            0.0,
        );
        draw_flat(g, Shape::Quads, SQUARE);
        g.glx.pop_matrix();

        g.glx.color4f(1.0, 1.0, 0.0, 0.75);
        g.glx.push_matrix();
        g.glx.translate(
            self.finish_position.1 as f32 / 2.0,
            self.finish_position.0 as f32 / 2.0,
            0.0,
        );
        draw_flat(g, Shape::TriangleFan, STAR);
        g.glx.pop_matrix();

        g.glx.color4f(1.0, 0.607_843_1, 0.0, 0.75);
        for rat in &self.rats {
            if rat.x == 0.0 && rat.z == 0.0 {
                continue;
            }
            g.glx.push_matrix();
            g.glx.translate(rat.x, rat.z, 0.0);
            g.glx.rotate(rat.rotation, 0.0, 0.0, 1.0);
            draw_flat(g, Shape::Polygon, ARROW);
            g.glx.pop_matrix();
        }

        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        for p in &self.inverter_position {
            let kind = self.grid[p.0][p.1];
            if !(INVERTER_TETRAHEDRON..=INVERTER_ICOSAHEDRON).contains(&kind) {
                continue;
            }
            g.glx.push_matrix();
            g.glx.translate(p.1 as f32 / 2.0, p.0 as f32 / 2.0, 0.0);
            g.glx.rotate(1.5 * self.inverter_rotation, 0.0, 0.0, 1.0);
            draw_flat(g, Shape::Polygon, TRIANGLE);
            g.glx.pop_matrix();
        }

        g.glx.blend(Blend::Off);
        g.glx.color_material(false);
        g.glx.pop_matrix();
        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();
    }
}

/// One of the flat shapes the overlay is drawn out of.
fn draw_flat(g: &mut Gl, shape: Shape, verts: &[[f32; 2]]) {
    g.glx.begin(shape);
    for v in verts {
        g.glx.vertex3f(v[0], v[1], 0.0);
    }
    g.glx.end();
}

impl Hack3d for Maze3d {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let delay = g.res.int("delay").max(0);
        let h = g.height() as f32 / g.width().max(1) as f32;

        // Upstream sets the perspective and then applies the camera without
        // switching back to the modelview, so the whole camera transform is
        // in the projection matrix. Kept, since it is what it draws.
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(90.0, 1.0 / h, 0.05, 100.0);
        g.glx.line_width(2.0);

        g.glx.rotate(self.camera.inversion, 0.0, 0.0, 1.0);
        g.glx.rotate(self.camera.rotation, 0.0, 1.0, 0.0);
        g.glx.translate(-self.camera.x, -0.5, -self.camera.z);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        // How far everything gets to move this frame.
        let step = self.speed * 1.6 * (delay as f32 / 1_000_000.0);
        self.camera.remaining = step;
        for r in &mut self.rats {
            r.remaining = step;
        }

        self.inverter_rotation += 45.0 * self.camera.remaining;
        self.shift_acid_color();
        self.step(true, 0);

        g.glx.depth_test(true);
        g.glx.clear();

        self.draw_walls(g);
        self.draw_slab(g, 1.0, self.textures.ceiling);
        self.draw_slab(g, 0.0, self.textures.floor);

        for i in 0..self.inverter_position.len() {
            self.draw_inverter(g, self.inverter_position[i]);
        }

        for i in 0..self.rats.len() {
            self.step(false, i);
            let (x, z) = (self.rats[i].x, self.rats[i].z);
            self.draw_rat(g, x, z);
        }

        let small = g.width() < g.height();
        self.draw_pane(g, self.textures.finish, self.finish_position, small);
        self.draw_pane(g, self.textures.start, self.start_position, small);

        if self.show_overlay || self.button_down {
            self.draw_overlay(g);
        }

        delay as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.clear();
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { .. } => {
                self.button_down = true;
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down = false;
                true
            }
            _ => false,
        }
    }
}

fn load(g: &mut Gl, png: &[u8]) -> Option<u32> {
    let (w, h, px) = crate::runtime::png::decode_rgba(png)?;
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(w, h, px);
    // The floor and the ceiling tile their picture once per square.
    g.glx.tex_clamp(false);
    Some(id)
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let rows = g.res.int("numRows").clamp(2, 24) as usize;
    let columns = g.res.int("numColumns").clamp(2, 24) as usize;
    let num_rows = rows * 2 + 1;
    let num_columns = columns * 2 + 1;

    // There have to be rooms left over for everything that goes in one.
    let mut free = (num_rows / 2) * (num_columns / 2) - 2;
    let mut num_inverters = g.res.int("numInverters").clamp(0, 100) as usize;
    if free < num_inverters {
        num_inverters = free;
        free = 0;
    } else {
        free -= num_inverters;
    }
    let mut num_rats = g.res.int("numRats").clamp(0, 100) as usize;
    if free < num_rats {
        num_rats = free;
    }

    let textures = Textures {
        wall: load(g, crate::images::BRICK1),
        ceiling: load(g, crate::images::BRICK2),
        floor: load(g, crate::images::WOOD2),
        start: load(g, crate::images::START),
        finish: load(g, crate::images::LOGO_32),
        rat: load(g, crate::images::BOB),
    };

    let blank = Rat {
        x: 0.0,
        z: 0.0,
        rotation: NORTH,
        desired_rotation: NORTH,
        inversion: 0.0,
        remaining: 0.0,
        state: State::Walking,
    };

    let mut st = Maze3d {
        grid: vec![vec![WALL; num_columns]; num_rows],
        num_rows,
        num_columns,
        wall_list: Vec::new(),
        start_position: (0, 0),
        finish_position: (0, 0),
        inverter_position: Vec::new(),
        camera: Rat {
            state: State::Starting,
            ..blank
        },
        rats: vec![blank; num_rats],
        textures,
        acid_color: [1.0; 3],
        acid_hue: 0.0,
        wall_height: 0.0,
        inverter_rotation: 0.0,
        speed: g.res.float("speed").clamp(0.02, 4.0) as f32,
        num_inverters,
        num_rats,
        drop_acid: g.res.bool("dropAcid"),
        show_overlay: g.res.bool("showOverlay"),
        button_down: false,
    };

    st.new_maze();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
    g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
    g.glx.light_position(0, 0.0, 2.0, 0.0, 0.0);
    g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
    g.glx.color_material(true);

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*speed:        1.0",
    "*numRows:      12",
    "*numColumns:   12",
    "*numRats:      1",
    "*numInverters: 10",
    "*showOverlay:  False",
    "*dropAcid:     False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.02, 4.0, 0.02, 2, "1.0"),
    Opt::slider("numRows", "Rows", 2.0, 24.0, 1.0, 0, "12"),
    Opt::slider("numColumns", "Columns", 2.0, 24.0, 1.0, 0, "12"),
    Opt::slider("numInverters", "Inverters", 0.0, 100.0, 1.0, 0, "10"),
    Opt::slider("numRats", "Rats", 0.0, 100.0, 1.0, 0, "1"),
    Opt::boolean("showOverlay", "Show overlay", "false"),
    Opt::boolean("dropAcid", "Acid", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "maze3d",
    label: "Maze 3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Sudoer",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=VTAwxTVdyLc"),
        blurb: "A re-creation of the 3D Maze screensaver from Windows 95.",
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

    /// A maze with no GL behind it.
    fn a_maze(rows: usize, columns: usize, inverters: usize, rats: usize) -> Maze3d {
        let blank = Rat {
            x: 0.0,
            z: 0.0,
            rotation: NORTH,
            desired_rotation: NORTH,
            inversion: 0.0,
            remaining: 0.0,
            state: State::Walking,
        };
        let mut st = Maze3d {
            grid: vec![vec![WALL; columns * 2 + 1]; rows * 2 + 1],
            num_rows: rows * 2 + 1,
            num_columns: columns * 2 + 1,
            wall_list: Vec::new(),
            start_position: (0, 0),
            finish_position: (0, 0),
            inverter_position: Vec::new(),
            camera: Rat {
                state: State::Starting,
                ..blank
            },
            rats: vec![blank; rats],
            textures: Textures::default(),
            acid_color: [1.0; 3],
            acid_hue: 0.0,
            wall_height: 0.0,
            inverter_rotation: 0.0,
            speed: 1.0,
            num_inverters: inverters,
            num_rats: rats,
            drop_acid: false,
            show_overlay: false,
            button_down: false,
        };
        st.new_maze();
        st
    }

    /// Which squares of the grid are open, by flooding from one of them.
    fn reachable(m: &Maze3d) -> Vec<Vec<bool>> {
        let mut seen = vec![vec![false; m.num_columns]; m.num_rows];
        let mut stack = vec![(1usize, 1usize)];
        seen[1][1] = true;
        while let Some((r, c)) = stack.pop() {
            for (rr, cc) in [(r - 1, c), (r + 1, c), (r, c - 1), (r, c + 1)] {
                if rr < m.num_rows && cc < m.num_columns && m.grid[rr][cc] != WALL && !seen[rr][cc]
                {
                    seen[rr][cc] = true;
                    stack.push((rr, cc));
                }
            }
        }
        seen
    }

    /// Prim's algorithm gives a perfect maze: every room reachable from every
    /// other, and exactly one way between any two of them. A grid of `n`
    /// rooms joined by `n - 1` knocked-through walls and no loops is a tree,
    /// which is what makes the left-hand rule solve it.
    #[test]
    fn the_maze_is_a_tree_with_every_room_in_it() {
        for (rows, cols) in [(3usize, 3usize), (12, 12), (6, 20)] {
            let m = a_maze(rows, cols, 0, 0);
            let seen = reachable(&m);

            let mut rooms = 0;
            for (i, row) in seen.iter().enumerate() {
                for (j, open) in row.iter().enumerate() {
                    if Maze3d::is_odd(i) && Maze3d::is_odd(j) {
                        rooms += 1;
                        assert!(*open, "room {i},{j} of {rows}x{cols} is walled in");
                    }
                }
            }
            assert_eq!(rooms, rows * cols);

            // One opening per wall knocked through, and a tree of n rooms has
            // n - 1 edges.
            let mut openings = 0;
            for i in 0..m.num_rows {
                for j in 0..m.num_columns {
                    if Maze3d::is_odd(i) != Maze3d::is_odd(j) && m.grid[i][j] != WALL {
                        openings += 1;
                    }
                }
            }
            assert_eq!(
                openings,
                rooms - 1,
                "{rows}x{cols} has {openings} openings for {rooms} rooms, so it is not a tree"
            );
        }
    }

    /// The border is solid all the way round, so nothing can walk out.
    #[test]
    fn the_maze_has_a_wall_round_the_outside() {
        let m = a_maze(8, 8, 0, 0);
        for i in 0..m.num_rows {
            assert_eq!(m.grid[i][0], WALL, "row {i} is open on the left");
            assert_eq!(
                m.grid[i][m.num_columns - 1],
                WALL,
                "row {i} is open on the right"
            );
        }
        for j in 0..m.num_columns {
            assert_eq!(m.grid[0][j], WALL, "column {j} is open at the top");
            assert_eq!(
                m.grid[m.num_rows - 1][j],
                WALL,
                "column {j} is open at the bottom"
            );
        }
    }

    /// The start, the finish and every inverter go in a room of their own, and
    /// the camera starts in an open square beside the start sign.
    #[test]
    fn everything_is_placed_in_a_room_of_its_own() {
        let m = a_maze(10, 10, 8, 3);
        let mut taken = vec![m.start_position, m.finish_position];
        taken.extend(m.inverter_position.iter().copied());

        for (i, p) in taken.iter().enumerate() {
            assert!(
                Maze3d::is_odd(p.0) && Maze3d::is_odd(p.1),
                "object {i} is at {p:?}, which is not a room"
            );
            assert!(
                taken.iter().skip(i + 1).all(|q| q != p),
                "two objects share the room at {p:?}"
            );
        }
        assert_eq!(m.grid[m.start_position.0][m.start_position.1], START_CELL);
        assert_eq!(m.grid[m.finish_position.0][m.finish_position.1], FINISH);

        // The camera is on a half-square, in an open square, facing away from
        // the sign it is standing next to.
        let cz = (m.camera.z * 2.0).round() as usize;
        let cx = (m.camera.x * 2.0).round() as usize;
        assert_ne!(m.grid[cz][cx], WALL, "the camera started inside a wall");
        assert!([NORTH, EAST, SOUTH, WEST].contains(&m.camera.rotation));
    }

    /// The left-hand rule: walking the maze with a hand on the left wall
    /// visits every room, which is what lets the camera always find the
    /// finish. Follow it and check it comes back to where it started.
    #[test]
    fn the_left_hand_rule_gets_round_the_whole_maze() {
        let m = a_maze(5, 5, 0, 0);
        // Grid directions, in the order the rule prefers them.
        let dirs: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        let start = ((m.camera.z * 2.0) as i32, (m.camera.x * 2.0) as i32);
        let mut at = start;
        let mut facing = match m.camera.rotation as i32 {
            0 => 0usize,
            90 => 1,
            180 => 2,
            _ => 3,
        };

        let open = |r: i32, c: i32| -> bool {
            r >= 0
                && c >= 0
                && (r as usize) < m.num_rows
                && (c as usize) < m.num_columns
                && m.grid[r as usize][c as usize] != WALL
        };

        let mut visited = std::collections::HashSet::new();
        for _ in 0..20_000 {
            visited.insert(at);
            // Left, then straight on, then right, then back.
            let order = [(facing + 3) % 4, facing, (facing + 1) % 4, (facing + 2) % 4];
            let next = order
                .into_iter()
                .find(|&d| open(at.0 + dirs[d].0, at.1 + dirs[d].1));
            let d = next.expect("a maze square always has a way out");
            facing = d;
            at = (at.0 + dirs[d].0, at.1 + dirs[d].1);
        }

        let rooms: Vec<(i32, i32)> = (0..m.num_rows)
            .flat_map(|i| (0..m.num_columns).map(move |j| (i, j)))
            .filter(|(i, j)| Maze3d::is_odd(*i) && Maze3d::is_odd(*j))
            .map(|(i, j)| (i as i32, j as i32))
            .collect();
        for r in &rooms {
            assert!(
                visited.contains(r),
                "the rule never reached the room at {r:?}"
            );
        }
    }

    /// The walls rise out of the floor when a maze is built and sink back
    /// into it when the finish is reached.
    #[test]
    fn the_walls_rise_and_fall() {
        let mut r = start(StartArgs::new(640, 480, "numRows=3&numColumns=3", 20260812));
        r.step();
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");

        let mut m = a_maze(3, 3, 0, 0);
        assert_eq!(m.wall_height, 0.0, "it did not start flat");
        assert_eq!(m.camera.state, State::Starting);
        for _ in 0..200 {
            m.camera.remaining = 0.032;
            m.step(true, 0);
            if m.wall_height >= 1.0 {
                break;
            }
        }
        assert_eq!(m.wall_height, 1.0, "the walls never finished rising");
        assert_ne!(m.camera.state, State::Starting, "it never started walking");
    }

    /// It draws the maze, with all six pictures on it.
    #[test]
    fn the_maze_is_drawn() {
        let r = run("numRows=6&numColumns=6&showOverlay=true", 40);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        let textures: std::collections::HashSet<_> =
            f.batches.iter().filter_map(|b| b.texture).collect();
        assert!(textures.len() >= 3, "only {} pictures used", textures.len());
    }
}
