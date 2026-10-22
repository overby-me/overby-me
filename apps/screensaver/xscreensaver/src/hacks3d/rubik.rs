//! Port of `hacks/glx/rubik.c`.
//!
//! ```text
//! rubik --- Shows an auto-solving Rubik's cube
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
//! Marcelo F. Vianna (Jul-31-1997)
//! ```
//!
//! A Rubik's cube that shuffles itself and then unshuffles itself, drifting
//! about the screen as it turns. There is no solver: the shuffle is a list of
//! random moves, and the solution is that list played backwards.
//!
//! It is not only 3×3×3. Any L×M×N is allowed, and a face that is not square
//! can only be turned half a turn at a time, which the move machinery works
//! out for itself.
//!
//! Upstream's own diagram of how the faces are numbered:
//!
//! ```text
//!             +-----------+
//!             |0-->       |
//!             ||          |
//!             |v  TOP(0)  |
//!             |          8|
//! +-----------+-----------+-----------+
//! |0-->       |0-->       |0-->       |
//! ||          ||          ||          |
//! |v  LEFT(1) |v FRONT(2) |v RIGHT(3) |
//! |          8|          8|          8|
//! +-----------+-----------+-----------+
//!             |0-->       |
//!             ||          |
//!             |v BOTTOM(4)|
//!             |          8|
//!             +-----------+             +---+---+---+
//!             |0-->       |             | 0 | 1 | 2 |
//!             ||          |             |--xxxxx(N)-+
//!             |v  BACK(5) |             | 3 | 4 | 5 |
//!             |          8|             +---+---+---+
//!             +-----------+             | 6 | 7 | 8 |
//!                                       +---+---+---+
//! ```

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
    random_below, screenhack_event_helper,
};

const ACTION_SOLVE: bool = true;
const ACTION_SHUFFLE: bool = false;

const DELAY_AFTER_SHUFFLING: f32 = 5.0;
const DELAY_AFTER_SOLVING: f32 = 20.0;

const MINSIZE: i32 = 2;

/// How many orientations a square has, and how many faces the cube has.
const MAXORIENT: i32 = 4;
const MAXFACES: usize = 6;

/// Directions relative to the face of a cubie.
const TOP: i32 = 0;
const RIGHT: i32 = 1;
const BOTTOM: i32 = 2;
const LEFT: i32 = 3;
const CW: i32 = MAXORIENT + 1;
const HALF: i32 = MAXORIENT + 2;
const CCW: i32 = 2 * MAXORIENT - 1;

const TOP_FACE: usize = 0;
const LEFT_FACE: usize = 1;
const FRONT_FACE: usize = 2;
const RIGHT_FACE: usize = 3;
const BOTTOM_FACE: usize = 4;
const BACK_FACE: usize = 5;
const NO_FACE: i32 = MAXFACES as i32;
const NO_ROTATION: i32 = 2 * MAXORIENT;

/// How big a cubie is, how far in from its corner the rounding starts, and
/// where the sticker sits.
const CUBELEN: f32 = 0.50;
const CUBEROUND: f32 = CUBELEN - 0.05;
const STICKERLONG: f32 = CUBEROUND - 0.05;
const STICKERSHORT: f32 = STICKERLONG - 0.05;
const STICKERDEPTH: f32 = CUBELEN + 0.01;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RubikLoc {
    face: i32,
    /// Which way up the sticker is. Upstream keeps it and never draws it.
    rotation: i32,
}

#[derive(Clone, Copy, Default)]
struct RubikMove {
    face: i32,
    direction: i32,
    position: i32,
}

/// A move, restated as "turn this plane of the cube, this deep in".
#[derive(Clone, Copy)]
struct RubikSlice {
    face: i32,
    rotation: i32,
    depth: i32,
}

/// Pick a face and a direction on it, and the next face round and its
/// orientation are then known.
const SLIDE_NEXT_ROW: [[RubikLoc; 4]; MAXFACES] = [
    [loc(5, TOP), loc(3, RIGHT), loc(2, TOP), loc(1, LEFT)],
    [loc(0, RIGHT), loc(2, TOP), loc(4, LEFT), loc(5, BOTTOM)],
    [loc(0, TOP), loc(3, TOP), loc(4, TOP), loc(1, TOP)],
    [loc(0, LEFT), loc(5, BOTTOM), loc(4, RIGHT), loc(2, TOP)],
    [loc(2, TOP), loc(3, LEFT), loc(5, TOP), loc(1, RIGHT)],
    [loc(4, TOP), loc(3, BOTTOM), loc(0, TOP), loc(1, BOTTOM)],
];

const fn loc(face: i32, rotation: i32) -> RubikLoc {
    RubikLoc { face, rotation }
}

/// Cubie zero on each face and its two distinct movements, translated into
/// slice movements. Clockwise is the deep depth and counter-clockwise the
/// shallow one, with reference to faces 0, 1 and 2.
const ROTATE_SLICE: [[RubikLoc; 2]; MAXFACES] = [
    [loc(1, CCW), loc(2, CW)],
    [loc(2, CW), loc(0, CCW)],
    [loc(1, CCW), loc(0, CCW)],
    [loc(2, CCW), loc(0, CCW)],
    [loc(1, CCW), loc(2, CCW)],
    [loc(1, CCW), loc(0, CW)],
];

/// Rotate a face clockwise by this many orients and its top then points at
/// this face.
const ROW_TO_ROTATE: [[usize; 4]; MAXFACES] = [
    [3, 2, 1, 5],
    [2, 4, 5, 0],
    [3, 4, 1, 0],
    [5, 4, 2, 0],
    [3, 5, 1, 2],
    [3, 0, 1, 4],
];

/// What a clockwise move turns into, as something more manageable.
struct RubikRowNext {
    face: usize,
    direction: i32,
    side_face: i32,
}

const ROTATE_TO_ROW: [RubikRowNext; MAXFACES] = [
    RubikRowNext {
        face: 1,
        direction: LEFT,
        side_face: TOP,
    },
    RubikRowNext {
        face: 0,
        direction: BOTTOM,
        side_face: RIGHT,
    },
    RubikRowNext {
        face: 0,
        direction: RIGHT,
        side_face: BOTTOM,
    },
    RubikRowNext {
        face: 0,
        direction: TOP,
        side_face: LEFT,
    },
    RubikRowNext {
        face: 1,
        direction: RIGHT,
        side_face: BOTTOM,
    },
    RubikRowNext {
        face: 0,
        direction: LEFT,
        side_face: TOP,
    },
];

const MATERIAL_RED: [f32; 4] = [0.5, 0.0, 0.0, 1.0];
const MATERIAL_GREEN: [f32; 4] = [0.0, 0.5, 0.0, 1.0];
const MATERIAL_BLUE: [f32; 4] = [0.0, 0.0, 0.5, 1.0];
const MATERIAL_YELLOW: [f32; 4] = [0.7, 0.7, 0.0, 1.0];
const MATERIAL_ORANGE: [f32; 4] = [0.9, 0.45, 0.36, 1.0];
const MATERIAL_WHITE: [f32; 4] = [0.8, 0.8, 0.8, 1.0];
const MATERIAL_GRAY: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

/// Which colour a sticker on the given face started out.
fn pickcolor(c: i32) -> [f32; 4] {
    match c as usize {
        TOP_FACE => MATERIAL_RED,
        LEFT_FACE => MATERIAL_YELLOW,
        FRONT_FACE => MATERIAL_WHITE,
        RIGHT_FACE => MATERIAL_GREEN,
        BOTTOM_FACE => MATERIAL_ORANGE,
        _ => MATERIAL_BLUE,
    }
}

/// `draw_stickerless_cubit`: the rounded grey body every cubie has, six faces
/// with their corners and edges cut off.
fn draw_stickerless_cubit(g: &mut Gl) {
    const L: f32 = CUBELEN;
    const R: f32 = CUBEROUND;
    // The six faces, then the twelve bevels along the edges.
    const QUADS: [([f32; 3], [[f32; 3]; 4]); 18] = [
        (
            [0.0, 0.0, 1.0],
            [[-R, -R, L], [R, -R, L], [R, R, L], [-R, R, L]],
        ),
        (
            [0.0, 0.0, -1.0],
            [[-R, R, -L], [R, R, -L], [R, -R, -L], [-R, -R, -L]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [[-L, -R, R], [-L, R, R], [-L, R, -R], [-L, -R, -R]],
        ),
        (
            [1.0, 0.0, 0.0],
            [[L, -R, -R], [L, R, -R], [L, R, R], [L, -R, R]],
        ),
        (
            [0.0, -1.0, 0.0],
            [[R, -L, -R], [R, -L, R], [-R, -L, R], [-R, -L, -R]],
        ),
        (
            [0.0, 1.0, 0.0],
            [[-R, L, -R], [-R, L, R], [R, L, R], [R, L, -R]],
        ),
        (
            [-1.0, -1.0, 0.0],
            [[-R, -L, -R], [-R, -L, R], [-L, -R, R], [-L, -R, -R]],
        ),
        (
            [1.0, 1.0, 0.0],
            [[R, L, -R], [R, L, R], [L, R, R], [L, R, -R]],
        ),
        (
            [-1.0, 1.0, 0.0],
            [[-L, R, -R], [-L, R, R], [-R, L, R], [-R, L, -R]],
        ),
        (
            [1.0, -1.0, 0.0],
            [[L, -R, -R], [L, -R, R], [R, -L, R], [R, -L, -R]],
        ),
        (
            [0.0, -1.0, -1.0],
            [[-R, -R, -L], [R, -R, -L], [R, -L, -R], [-R, -L, -R]],
        ),
        (
            [0.0, 1.0, 1.0],
            [[-R, R, L], [R, R, L], [R, L, R], [-R, L, R]],
        ),
        (
            [0.0, -1.0, 1.0],
            [[-R, -L, R], [R, -L, R], [R, -R, L], [-R, -R, L]],
        ),
        (
            [0.0, 1.0, -1.0],
            [[-R, L, -R], [R, L, -R], [R, R, -L], [-R, R, -L]],
        ),
        (
            [-1.0, 0.0, -1.0],
            [[-L, -R, -R], [-L, R, -R], [-R, R, -L], [-R, -R, -L]],
        ),
        (
            [1.0, 0.0, 1.0],
            [[L, -R, R], [L, R, R], [R, R, L], [R, -R, L]],
        ),
        (
            [1.0, 0.0, -1.0],
            [[R, -R, -L], [R, R, -L], [L, R, -R], [L, -R, -R]],
        ),
        (
            [-1.0, 0.0, 1.0],
            [[-R, -R, L], [-R, R, L], [-L, R, R], [-L, -R, R]],
        ),
    ];
    // And the eight corners.
    const TRIS: [([f32; 3], [[f32; 3]; 3]); 8] = [
        ([1.0, 1.0, 1.0], [[R, R, L], [L, R, R], [R, L, R]]),
        (
            [-1.0, -1.0, -1.0],
            [[-R, -L, -R], [-L, -R, -R], [-R, -R, -L]],
        ),
        // Upstream's third corner has its vertices split across the polygon
        // count, which puts one of them after the next normal; kept as it is
        // written, since that is the shape it draws.
        ([-1.0, 1.0, 1.0], [[-R, R, L], [-R, L, R], [-L, R, R]]),
        ([1.0, -1.0, -1.0], [[L, -R, -R], [R, -L, -R], [R, -R, -L]]),
        ([1.0, -1.0, 1.0], [[R, -R, L], [R, -L, R], [L, -R, R]]),
        ([-1.0, 1.0, -1.0], [[-L, R, -R], [-R, L, -R], [-R, R, -L]]),
        ([-1.0, -1.0, 1.0], [[-R, -R, L], [-L, -R, R], [-R, -L, R]]),
        ([1.0, 1.0, -1.0], [[L, R, -R], [R, R, -L], [R, L, -R]]),
    ];

    g.glx.material_diffuse(MATERIAL_GRAY);
    g.glx.begin(Shape::Quads);
    for (n, quad) in QUADS {
        g.glx.normal3f(n[0], n[1], n[2]);
        for v in quad {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
    // Upstream sets the same grey again here; saying so twice would only cut
    // the batch in half.
    g.glx.begin(Shape::Triangles);
    for (n, tri) in TRIS {
        g.glx.normal3f(n[0], n[1], n[2]);
        for v in tri {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

/// The eight corners of a sticker, which is a square with its corners cut off.
const STICKER: [[f32; 2]; 8] = [
    [-STICKERSHORT, STICKERLONG],
    [STICKERSHORT, STICKERLONG],
    [STICKERLONG, STICKERSHORT],
    [STICKERLONG, -STICKERSHORT],
    [STICKERSHORT, -STICKERLONG],
    [-STICKERSHORT, -STICKERLONG],
    [-STICKERLONG, -STICKERSHORT],
    [-STICKERLONG, STICKERSHORT],
];

/// `draw_cubit`: the body, then whichever of its six sides carries a sticker.
fn draw_cubit(g: &mut Gl, faces: [i32; 6]) {
    draw_stickerless_cubit(g);
    let [back, front, left, right, bottom, top] = faces;

    // Each side takes the octagon above, laid into the right plane and wound
    // so that it faces outwards.
    /// Which sticker, which way it faces, and how the octagon above is laid
    /// into that plane.
    type Side = (i32, [f32; 3], fn([f32; 2]) -> [f32; 3]);
    let sides: [Side; 6] = [
        (back, [0.0, 0.0, -1.0], |p| [p[0], p[1], -STICKERDEPTH]),
        (front, [0.0, 0.0, 1.0], |p| [p[0], -p[1], STICKERDEPTH]),
        (left, [-1.0, 0.0, 0.0], |p| [-STICKERDEPTH, p[0], p[1]]),
        (right, [1.0, 0.0, 0.0], |p| [STICKERDEPTH, p[0], -p[1]]),
        (bottom, [0.0, -1.0, 0.0], |p| [p[1], -STICKERDEPTH, p[0]]),
        (top, [0.0, 1.0, 0.0], |p| [-p[1], STICKERDEPTH, p[0]]),
    ];
    for (face, normal, place) in sides {
        if face == NO_FACE {
            continue;
        }
        g.glx.material_diffuse(pickcolor(face));
        g.glx.begin(Shape::Polygon);
        g.glx.normal3f(normal[0], normal[1], normal[2]);
        for p in STICKER {
            let v = place(p);
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }
}

struct Rubik {
    step: f32,
    moves: Vec<RubikMove>,
    storedmoves: i32,
    degree_turn: i32,
    shufflingmoves: i32,
    sizex: i32,
    sizey: i32,
    sizez: i32,
    avsize: f32,
    /// Whether the cube is being unshuffled rather than shuffled.
    action: bool,
    done: bool,
    anglestep: f32,
    cube_loc: [Vec<RubikLoc>; MAXFACES],
    row_loc: [Vec<RubikLoc>; 4],
    movement: RubikMove,
    /// Doubles as the angle of the turn in progress and as the count of frames
    /// to wait once there is nothing left to do.
    rotatestep: f32,
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
    trackball: Trackball,
    hideshuffling: bool,
    cycles: i32,
    count: i32,
    size: i32,
    fixed: [i32; 3],
    aspect: f32,
    scale: f32,
}

impl Rubik {
    fn face_sizes(&self, face: usize) -> (i32, i32) {
        match face {
            0 | 4 => (self.sizex, self.sizez),
            1 | 3 => (self.sizez, self.sizey),
            _ => (self.sizex, self.sizey),
        }
    }

    /// A face that is not square can only be turned half a turn at a time.
    fn check_face_square(&self, face: usize) -> bool {
        let (r, c) = self.face_sizes(face);
        r == c
    }

    fn size_face(&self, face: usize) -> i32 {
        let (r, c) = self.face_sizes(face);
        r * c
    }

    fn size_row(&self, face: usize) -> i32 {
        self.face_sizes(face).0
    }

    /// `convertMove`: turn "this cubie, this way" into "this plane, this deep".
    fn convert_move(&self, mv: RubikMove) -> RubikSlice {
        let plane = ROTATE_SLICE[mv.face as usize][(mv.direction % 2) as usize];
        let mut slice = RubikSlice {
            face: plane.face,
            rotation: plane.rotation,
            depth: 0,
        };
        let (size_of_row, size_of_column) = self.face_sizes(mv.face as usize);
        // Vertical planes, and the front plane seen from the sides.
        if plane.face == 1 || (plane.face == 2 && (mv.face == 1 || mv.face == 3)) {
            slice.depth = if slice.rotation == CW {
                size_of_row - 1 - mv.position % size_of_row
            } else {
                mv.position % size_of_row
            };
        } else {
            slice.depth = if slice.rotation == CW {
                size_of_column - 1 - mv.position / size_of_row
            } else {
                mv.position / size_of_row
            };
        }
        if mv.direction / 2 != 0 {
            slice.rotation = if slice.rotation == CW { CCW } else { CW };
        }
        slice
    }

    /* The cube's own bookkeeping */

    fn read_rc(&mut self, face: usize, dir: i32, h: i32, orient: usize, size: i32) {
        let size_of_row = self.size_row(face);
        for gg in 0..size {
            let idx = if dir == TOP || dir == BOTTOM {
                gg * size_of_row + h
            } else {
                h * size_of_row + gg
            };
            self.row_loc[orient][gg as usize] = self.cube_loc[face][idx as usize];
        }
    }

    fn write_rc(&mut self, face: usize, dir: i32, h: i32, orient: usize, size: i32) {
        let size_of_row = self.size_row(face);
        for gg in 0..size {
            let idx = if dir == TOP || dir == BOTTOM {
                gg * size_of_row + h
            } else {
                h * size_of_row + gg
            };
            self.cube_loc[face][idx as usize] = self.row_loc[orient][gg as usize];
        }
    }

    fn rotate_rc(&mut self, rotate: i32, orient: usize, size: i32) {
        for gg in 0..size as usize {
            let r = &mut self.row_loc[orient][gg].rotation;
            *r = (*r + rotate) % MAXORIENT;
        }
    }

    fn reverse_rc(&mut self, orient: usize, size: i32) {
        self.row_loc[orient][..size as usize].reverse();
    }

    /// Turn one whole face, which is what happens to the end of a slice.
    fn rotate_face(&mut self, face: usize, direction: i32) {
        let (size_of_row, size_of_column) = self.face_sizes(face);
        let size_on_plane = (size_of_row * size_of_column) as usize;
        let face_loc = self.cube_loc[face][..size_on_plane].to_vec();
        for position in 0..size_on_plane {
            let i = position as i32 % size_of_row;
            let j = position as i32 / size_of_row;
            let from = if direction == CW {
                (size_of_row - i - 1) * size_of_row + j
            } else if direction == CCW {
                i * size_of_row + size_of_column - j - 1
            } else {
                size_of_row - i - 1 + (size_of_column - j - 1) * size_of_row
            };
            let mut v = face_loc[from as usize];
            v.rotation = (v.rotation + direction - MAXORIENT) % MAXORIENT;
            self.cube_loc[face][position] = v;
        }
    }

    /// `slideRC`: where a row goes when it slides off the edge of a face.
    /// "Yeah this is big and ugly."
    fn slide_rc(
        face: usize,
        direction: i32,
        h: i32,
        size_on_opp_axis: i32,
    ) -> (usize, i32, i32, i32, bool) {
        let next = SLIDE_NEXT_ROW[face][direction as usize];
        let new_face = next.face as usize;
        let rotate = next.rotation;
        let new_direction = (rotate + direction) % MAXORIENT;
        let (new_h, reverse) = match rotate {
            TOP => (h, false),
            RIGHT => {
                if new_direction == TOP || new_direction == BOTTOM {
                    (size_on_opp_axis - 1 - h, false)
                } else {
                    (h, true)
                }
            }
            BOTTOM => (size_on_opp_axis - 1 - h, true),
            LEFT => {
                if new_direction == TOP || new_direction == BOTTOM {
                    (h, true)
                } else {
                    (size_on_opp_axis - 1 - h, false)
                }
            }
            _ => (0, false),
        };
        (new_face, new_direction, new_h, rotate, reverse)
    }

    /// `moveRubik`: actually apply a move to the cube's state.
    fn move_rubik(&mut self, face: i32, direction: i32, position: i32) {
        let mut face = face as usize;
        let mut direction = direction;
        let mut position = position;
        let (mut size_of_row, mut size_of_column) = self.face_sizes(face);

        if direction == CW || direction == CCW {
            direction = if direction == CCW {
                (ROTATE_TO_ROW[face].direction + 2) % MAXORIENT
            } else {
                ROTATE_TO_ROW[face].direction
            };
            let ij = if ROTATE_TO_ROW[face].side_face == RIGHT {
                size_of_column - 1
            } else if ROTATE_TO_ROW[face].side_face == BOTTOM {
                size_of_row - 1
            } else {
                0
            };
            face = ROTATE_TO_ROW[face].face;
            position = ij * size_of_row + ij;
        }
        (size_of_row, size_of_column) = self.face_sizes(face);
        let i = position % size_of_row;
        let j = position / size_of_row;
        let mut h = if direction == TOP || direction == BOTTOM {
            i
        } else {
            j
        };
        let (size_on_axis, size_on_opp_axis) = if direction == TOP || direction == BOTTOM {
            (size_of_column, size_of_row)
        } else {
            (size_of_row, size_of_column)
        };

        // A slice at either end of the cube takes a whole face round with it.
        if h == size_on_opp_axis - 1 {
            let nd = if direction == TOP || direction == BOTTOM {
                TOP
            } else {
                RIGHT
            };
            let turn = if self.degree_turn == 180 {
                HALF
            } else if direction == TOP || direction == RIGHT {
                CW
            } else {
                CCW
            };
            self.rotate_face(ROW_TO_ROTATE[face][nd as usize], turn);
        }
        if h == 0 {
            let nd = if direction == TOP || direction == BOTTOM {
                BOTTOM
            } else {
                LEFT
            };
            let turn = if self.degree_turn == 180 {
                HALF
            } else if direction == TOP || direction == RIGHT {
                CCW
            } else {
                CW
            };
            self.rotate_face(ROW_TO_ROTATE[face][nd as usize], turn);
        }

        // Slide the rows or columns round the four faces they cross.
        self.read_rc(face, direction, h, 0, size_on_axis);
        if self.degree_turn == 180 {
            let (nf, nd, nh, rotate, reverse) =
                Self::slide_rc(face, direction, h, size_on_opp_axis);
            let size_on_depth_axis = self.size_face(nf) / size_on_opp_axis;
            self.read_rc(nf, nd, nh, 1, size_on_depth_axis);
            self.rotate_rc(rotate, 0, size_on_axis);
            if reverse {
                self.reverse_rc(0, size_on_axis);
            }
            face = nf;
            direction = nd;
            h = nh;
            for k in 2..=MAXORIENT + 1 {
                let (nf, nd, nh, rotate, reverse) =
                    Self::slide_rc(face, direction, h, size_on_opp_axis);
                let odd = k % 2 != 0;
                let a = if odd {
                    size_on_depth_axis
                } else {
                    size_on_axis
                };
                let b = if odd {
                    size_on_axis
                } else {
                    size_on_depth_axis
                };
                if k != MAXORIENT && k != MAXORIENT + 1 {
                    self.read_rc(nf, nd, nh, k as usize, a);
                }
                self.rotate_rc(rotate, (k - 2) as usize, a);
                if k != MAXORIENT + 1 {
                    self.rotate_rc(rotate, (k - 1) as usize, b);
                }
                if reverse {
                    self.reverse_rc((k - 2) as usize, a);
                    if k != MAXORIENT + 1 {
                        self.reverse_rc((k - 1) as usize, b);
                    }
                }
                self.write_rc(nf, nd, nh, (k - 2) as usize, a);
                face = nf;
                direction = nd;
                h = nh;
            }
        } else {
            for k in 1..=MAXORIENT {
                let (nf, nd, nh, rotate, reverse) =
                    Self::slide_rc(face, direction, h, size_on_opp_axis);
                if k != MAXORIENT {
                    self.read_rc(nf, nd, nh, k as usize, size_on_axis);
                }
                self.rotate_rc(rotate, (k - 1) as usize, size_on_axis);
                if reverse {
                    self.reverse_rc((k - 1) as usize, size_on_axis);
                }
                self.write_rc(nf, nd, nh, (k - 1) as usize, size_on_axis);
                face = nf;
                direction = nd;
                h = nh;
            }
        }
    }

    fn evalmovement(&mut self, movement: RubikMove) {
        if movement.face < 0 || movement.face >= MAXFACES as i32 {
            return;
        }
        self.move_rubik(movement.face, movement.direction, movement.position);
    }

    /// Whether two moves turn the same slice, the same way round or the
    /// opposite way.
    fn compare_moves(&self, m1: RubikMove, m2: RubikMove, opp: bool) -> bool {
        let s1 = self.convert_move(m1);
        let s2 = self.convert_move(m2);
        if s1.face == s2.face && s1.depth == s2.depth {
            return (s1.rotation == s2.rotation) != opp;
        }
        false
    }

    /// `shuffle`: pick a size, put the cube back together, and make up the
    /// list of moves that will scramble it.
    fn shuffle(&mut self) {
        let pick = |fixed: i32, size: i32| {
            let i = if fixed != 0 { fixed } else { size };
            if i < -MINSIZE {
                random_below(-i - MINSIZE + 1) + MINSIZE
            } else if i < MINSIZE {
                MINSIZE
            } else {
                i
            }
        };
        let i = pick(self.fixed[0], self.size);
        // Cubes are more likely than boxes, and boxes with a square face more
        // likely than boxes without.
        if random() % 2 == 1 && self.fixed[1] == 0 && self.fixed[2] == 0 {
            self.sizex = i;
            self.sizey = i;
            self.sizez = i;
        } else {
            self.sizex = i;
            let i = pick(self.fixed[1], self.size);
            if random() % 2 == 1 && self.fixed[2] == 0 {
                self.sizey = i;
                self.sizez = i;
            } else {
                self.sizey = i;
                self.sizez = pick(self.fixed[2], self.size);
            }
        }
        self.avsize = (self.sizex + self.sizey + self.sizez) as f32 / 3.0;

        for face in 0..MAXFACES {
            let n = self.size_face(face) as usize;
            self.cube_loc[face] = vec![
                RubikLoc {
                    face: face as i32,
                    rotation: TOP,
                };
                n
            ];
        }
        let maxmax = self.sizex.max(self.sizey).max(self.sizez) as usize;
        for row in &mut self.row_loc {
            *row = vec![RubikLoc::default(); maxmax];
        }

        self.storedmoves = self.count;
        if self.storedmoves < 0 {
            self.storedmoves = random_below(-self.storedmoves) + 1;
        }
        self.moves = vec![RubikMove::default(); self.storedmoves.max(0) as usize + 1];
        self.anglestep = if self.cycles <= 1 {
            90.0
        } else {
            90.0 / self.cycles as f32
        };

        for i in 0..self.storedmoves as usize {
            let mut mv = RubikMove::default();
            // Upstream spins here until it finds a move it likes; bounded, so
            // that a cube with very few distinct moves cannot hang.
            for _ in 0..200 {
                mv.face = random_below(MAXFACES as i32);
                // Excluding CW and CCW is fine.
                mv.direction = random_below(MAXORIENT);
                mv.position = random_below(self.size_face(mv.face as usize));
                self.degree_turn = if self
                    .check_face_square(ROW_TO_ROTATE[mv.face as usize][mv.direction as usize])
                {
                    90
                } else {
                    180
                };
                let mut ok = true;
                if i > 0 {
                    // Do not immediately undo the move just made.
                    if self.compare_moves(mv, self.moves[i - 1], true) {
                        ok = false;
                    }
                    if self.degree_turn == 180 && self.compare_moves(mv, self.moves[i - 1], false) {
                        ok = false;
                    }
                }
                if i > 1
                    && self.compare_moves(mv, self.moves[i - 1], false)
                    && self.compare_moves(mv, self.moves[i - 2], false)
                {
                    // And do not make three identical moves in a row.
                    ok = false;
                }
                if ok {
                    break;
                }
            }
            if self.hideshuffling {
                self.evalmovement(mv);
            }
            self.moves[i] = mv;
        }

        self.vx = if random() % 100 < 50 { -0.005 } else { 0.005 };
        self.vy = if random() % 100 < 50 { -0.005 } else { 0.005 };
        self.movement.face = NO_FACE;
        self.rotatestep = 0.0;
        self.action = if self.hideshuffling {
            ACTION_SOLVE
        } else {
            ACTION_SHUFFLE
        };
        self.shufflingmoves = 0;
        self.done = false;
    }

    /// `draw_cube`.
    ///
    /// Upstream unrolls this into three cases, one per axis a slice can turn
    /// about, each a dozen nested loops long, so that the turn can be applied
    /// a layer at a time between draws. The transform each cubie ends up under
    /// is the same either way, so this walks the shell once and applies the
    /// turn to whichever cubies are in the slice. Only the shell is drawn:
    /// upstream skips the inside too, since none of it can be seen.
    fn draw_cube(&self, g: &mut Gl) {
        let slice = if self.movement.face == NO_FACE {
            RubikSlice {
                face: NO_FACE,
                rotation: NO_ROTATION,
                depth: self.sizex.max(self.sizey).max(self.sizez),
            }
        } else {
            self.convert_move(self.movement)
        };
        let rotatestep = if slice.rotation == CCW {
            self.rotatestep
        } else {
            -self.rotatestep
        };

        let (sx, sy, sz) = (self.sizex, self.sizey, self.sizez);
        let half = [
            (sx - 1) as f32 / 2.0,
            (sy - 1) as f32 / 2.0,
            (sz - 1) as f32 / 2.0,
        ];
        let revy = |j: i32| sy - j - 1;
        let revz = |k: i32| sz - k - 1;

        for j in 0..sy {
            for i in 0..sx {
                for k in 0..sz {
                    let shell =
                        i == 0 || i == sx - 1 || j == 0 || j == sy - 1 || k == 0 || k == sz - 1;
                    if !shell {
                        continue;
                    }

                    let faces = [
                        if k == 0 {
                            self.cube_loc[BACK_FACE][(i + sx * j) as usize].face
                        } else {
                            NO_FACE
                        },
                        if k == sz - 1 {
                            self.cube_loc[FRONT_FACE][(i + sx * revy(j)) as usize].face
                        } else {
                            NO_FACE
                        },
                        if i == 0 {
                            self.cube_loc[LEFT_FACE][(k + sz * revy(j)) as usize].face
                        } else {
                            NO_FACE
                        },
                        if i == sx - 1 {
                            self.cube_loc[RIGHT_FACE][(revz(k) + sz * revy(j)) as usize].face
                        } else {
                            NO_FACE
                        },
                        if j == 0 {
                            self.cube_loc[BOTTOM_FACE][(i + sx * revz(k)) as usize].face
                        } else {
                            NO_FACE
                        },
                        if j == sy - 1 {
                            self.cube_loc[TOP_FACE][(i + sx * k) as usize].face
                        } else {
                            NO_FACE
                        },
                    ];

                    g.glx.push_matrix();
                    match slice.face as usize {
                        TOP_FACE if slice.depth == revy(j) => {
                            g.glx.rotate(rotatestep, 0.0, half[1], 0.0);
                        }
                        LEFT_FACE if slice.depth == i => {
                            g.glx.rotate(-rotatestep, half[0], 0.0, 0.0);
                        }
                        FRONT_FACE if slice.depth == revz(k) => {
                            g.glx.rotate(rotatestep, 0.0, 0.0, half[2]);
                        }
                        _ => {}
                    }
                    g.glx.translate(
                        (2 * i - sx + 1) as f32 / 2.0,
                        (2 * j - sy + 1) as f32 / 2.0,
                        (2 * k - sz + 1) as f32 / 2.0,
                    );
                    draw_cubit(g, faces);
                    g.glx.pop_matrix();
                }
            }
        }
    }

    /// How far the turn in progress has to go before the move is done.
    fn set_degree_turn(&mut self) {
        let square = if self.movement.direction == CW || self.movement.direction == CCW {
            self.check_face_square(self.movement.face as usize)
        } else {
            self.check_face_square(
                ROW_TO_ROTATE[self.movement.face as usize][self.movement.direction as usize],
            )
        };
        self.degree_turn = if square { 90 } else { 180 };
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut this = Rubik {
        step: random_below(90) as f32,
        moves: Vec::new(),
        storedmoves: 0,
        degree_turn: 90,
        shufflingmoves: 0,
        sizex: 3,
        sizey: 3,
        sizez: 3,
        avsize: 3.0,
        action: ACTION_SHUFFLE,
        done: false,
        anglestep: 4.5,
        cube_loc: Default::default(),
        row_loc: Default::default(),
        movement: RubikMove {
            face: NO_FACE,
            direction: 0,
            position: 0,
        },
        rotatestep: 0.0,
        px: frand(2.0) as f32 - 1.0,
        py: frand(2.0) as f32 - 1.0,
        vx: 0.005,
        vy: 0.005,
        trackball: Trackball::new(),
        hideshuffling: g.res.bool("hideshuffling"),
        cycles: g.res.int("cycles"),
        count: g.res.int("count"),
        size: g.res.int("size"),
        fixed: [g.res.int("sizex"), g.res.int("sizey"), g.res.int("sizez")],
        aspect: 1.0,
        scale: 1.0,
    };
    this.shuffle();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Rubik {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        // `Scale4Window * WindH / WindW`: the cube keeps its shape however
        // wide the window is.
        self.aspect = height as f32 / width as f32;
        self.scale = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            self.done = true;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 5.0, 15.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.color_material(false);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_enable(1, true);
        g.glx.light_ambient(1, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(1, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(1, -1.0, -1.0, 1.0, 0.0);
        g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
        g.glx.material_shininess(60.0);
        g.glx.material_specular([0.7, 0.7, 0.7, 1.0]);

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -10.0);

        // The cube drifts about and bounces off the edges, gaining a little
        // randomness each time it does.
        self.px += self.vx;
        self.py += self.vy;
        let mut bounced = false;
        for (p, v) in [(&mut self.px, &mut self.vx), (&mut self.py, &mut self.vy)] {
            // Upstream writes these as `p += -1 - p` and `p -= p - 1`,
            // which come to the same thing: back onto the edge exactly.
            if *p < -1.0 {
                *p = -1.0;
                *v = -*v;
                bounced = true;
            }
            if *p > 1.0 {
                *p = 1.0;
                *v = -*v;
                bounced = true;
            }
        }
        if bounced {
            self.vx += frand(0.002) as f32 - 0.001;
            self.vy += frand(0.002) as f32 - 0.001;
            self.vx = self.vx.clamp(-0.006, 0.006);
            self.vy = self.vy.clamp(-0.006, 0.006);
        }

        g.glx.translate(self.px, self.py, 0.0);
        let s = 0.9 / self.avsize;
        g.glx.scale(s * self.aspect, s, s);
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.mult_matrix(self.trackball.matrix());
        g.glx.rotate(self.step * 100.0, 1.0, 0.0, 0.0);
        g.glx.rotate(self.step * 95.0, 0.0, 1.0, 0.0);
        g.glx.rotate(self.step * 90.0, 0.0, 0.0, 1.0);

        self.draw_cube(g);
        g.glx.pop_matrix();

        // And then step the shuffle or the solution along by one frame.
        if self.action == ACTION_SHUFFLE {
            if self.done {
                self.rotatestep += 1.0;
                if self.rotatestep > DELAY_AFTER_SHUFFLING {
                    self.movement.face = NO_FACE;
                    self.rotatestep = 0.0;
                    self.action = ACTION_SOLVE;
                    self.done = false;
                }
            } else if self.movement.face == NO_FACE {
                self.rotatestep = 0.0;
                if self.shufflingmoves < self.storedmoves {
                    self.movement = self.moves[self.shufflingmoves as usize];
                } else {
                    self.done = true;
                }
            } else {
                if self.rotatestep == 0.0 {
                    self.set_degree_turn();
                }
                self.rotatestep += self.anglestep;
                if self.rotatestep > self.degree_turn as f32 {
                    let m = self.movement;
                    self.evalmovement(m);
                    self.shufflingmoves += 1;
                    self.movement.face = NO_FACE;
                }
            }
        } else if self.done {
            self.rotatestep += 1.0;
            if self.rotatestep > DELAY_AFTER_SOLVING {
                self.shuffle();
            }
        } else if self.movement.face == NO_FACE {
            self.rotatestep = 0.0;
            if self.storedmoves > 0 {
                self.movement = self.moves[self.storedmoves as usize - 1];
                // Backwards: the solution is the shuffle undone.
                self.movement.direction = (self.movement.direction + MAXORIENT / 2) % MAXORIENT;
            } else {
                self.done = true;
            }
        } else {
            if self.rotatestep == 0.0 {
                self.set_degree_turn();
            }
            self.rotatestep += self.anglestep;
            if self.rotatestep > self.degree_turn as f32 {
                let m = self.movement;
                self.evalmovement(m);
                self.storedmoves -= 1;
                self.movement.face = NO_FACE;
            }
        }

        self.step += 0.002;
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:         20000",
    "*count:         -30",
    "*showFPS:       False",
    "*cycles:        20",
    "*size:          -6",
    "*sizex:         0",
    "*sizey:         0",
    "*sizez:         0",
    "*hideshuffling: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::spin("count", "Count", -100.0, 100.0, "-30"),
    Opt::slider("cycles", "Rotation", 3.0, 200.0, 1.0, 0, "20").inverted(),
    // Upstream's spinner runs to twenty a side. Every cubie of the shell is
    // its own draw, so a twenty-cube is two thousand of them and well past
    // what a frame here can hold; ten is about four hundred, which is fine.
    Opt::spin("size", "Size", -10.0, 10.0, "-6"),
    Opt::boolean("hideshuffling", "Hide shuffling", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rubik",
    label: "Rubik",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Marcelo Vianna",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=AQdJgvyVkXU"),
        blurb: "A Rubik's Cube that repeatedly shuffles and solves itself.",
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

    fn solved(r: &Rubik) -> bool {
        (0..MAXFACES).all(|f| r.cube_loc[f].iter().all(|l| l.face == f as i32))
    }

    fn cube() -> Rubik {
        let mut r = Rubik {
            step: 0.0,
            moves: Vec::new(),
            storedmoves: 0,
            degree_turn: 90,
            shufflingmoves: 0,
            sizex: 3,
            sizey: 3,
            sizez: 3,
            avsize: 3.0,
            action: ACTION_SHUFFLE,
            done: false,
            anglestep: 4.5,
            cube_loc: Default::default(),
            row_loc: Default::default(),
            movement: RubikMove {
                face: NO_FACE,
                direction: 0,
                position: 0,
            },
            rotatestep: 0.0,
            px: 0.0,
            py: 0.0,
            vx: 0.005,
            vy: 0.005,
            trackball: Trackball::new(),
            hideshuffling: false,
            cycles: 20,
            count: -30,
            size: 3,
            fixed: [3, 3, 3],
            aspect: 1.0,
            scale: 1.0,
        };
        crate::runtime::ya_rand_init(20260812);
        r.shuffle();
        r
    }

    /// The whole conceit: the solution is the shuffle played backwards, so
    /// running every move and then undoing it has to put every sticker back.
    #[test]
    fn undoing_the_shuffle_solves_it() {
        let mut r = cube();
        assert!(solved(&r), "the cube did not start solved");
        let moves = r.moves.clone();
        let n = r.storedmoves as usize;
        for m in &moves[..n] {
            r.movement = *m;
            r.set_degree_turn();
            r.evalmovement(*m);
        }
        assert!(!solved(&r), "shuffling {n} moves changed nothing");
        for m in moves[..n].iter().rev() {
            let mut back = *m;
            r.movement = back;
            r.set_degree_turn();
            back.direction = (back.direction + MAXORIENT / 2) % MAXORIENT;
            r.evalmovement(back);
        }
        assert!(solved(&r), "undoing {n} moves did not solve it");
    }

    /// A move never invents or loses a sticker: each colour still covers
    /// exactly one face's worth.
    #[test]
    fn a_move_only_moves_stickers() {
        let mut r = cube();
        let want = r.size_face(0) as usize;
        let moves = r.moves.clone();
        for m in &moves[..r.storedmoves as usize] {
            r.movement = *m;
            r.set_degree_turn();
            r.evalmovement(*m);
            for f in 0..MAXFACES {
                let n = r
                    .cube_loc
                    .iter()
                    .flatten()
                    .filter(|l| l.face == f as i32)
                    .count();
                assert_eq!(n, want, "colour {f} covers {n} squares");
            }
        }
    }

    /// A face that is not square can only turn half a turn, which is what the
    /// move machinery has to work out for a box rather than a cube.
    #[test]
    fn an_oblong_face_turns_half_way() {
        let mut r = cube();
        r.sizex = 2;
        r.sizey = 3;
        r.sizez = 2;
        // The top is 2x2 and square; the front is 2x3 and is not.
        assert!(r.check_face_square(TOP_FACE));
        assert!(!r.check_face_square(FRONT_FACE));
        r.movement = RubikMove {
            face: 0,
            direction: RIGHT,
            position: 0,
        };
        r.set_degree_turn();
        assert_eq!(r.degree_turn, 180, "a turn about an oblong face was 90");
    }

    /// Every sticker has to be wound so that its face points the way its
    /// normal says, or the one that is wound backwards gets culled and that
    /// side of the cube comes out bare.
    #[test]
    fn every_sticker_faces_outwards() {
        let mut r = start(StartArgs::new(640, 480, "size=3", 20260812));
        r.step();
        let f = r.frame();
        let mut checked = 0;
        for b in &f.batches {
            let vs = &f.vertices[b.first..b.first + b.count];
            // A sticker is an octagon, kept as a triangle fan of eight.
            if b.primitive != crate::runtime::gl::Primitive::TriangleFan || b.count != 8 {
                continue;
            }
            let n = vs[0].normal;
            let (a, bb, c) = (vs[0].pos, vs[1].pos, vs[2].pos);
            let e1 = [bb[0] - a[0], bb[1] - a[1], bb[2] - a[2]];
            let e2 = [c[0] - bb[0], c[1] - bb[1], c[2] - bb[2]];
            let cross = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let dot = cross[0] * n[0] + cross[1] * n[1] + cross[2] * n[2];
            assert!(
                dot > 0.0,
                "a sticker with normal {n:?} is wound the other way ({dot})"
            );
            checked += 1;
        }
        assert!(checked > 20, "only {checked} stickers were checked");
    }

    /// Only the shell is drawn, so a bigger cube costs its surface rather than
    /// its volume.
    #[test]
    fn only_the_shell_is_drawn() {
        let mut r = start(StartArgs::new(640, 480, "size=4", 20260812));
        r.step();
        let f = r.frame();
        // Four a side is 64 cubies, of which 56 are on the shell, and each is
        // a body plus up to three stickers.
        let bodies = f
            .batches
            .iter()
            .filter(|b| b.count == 18 * 6 + 8 * 3)
            .count();
        assert_eq!(bodies, 56, "{bodies} cubies were drawn");
    }
}
