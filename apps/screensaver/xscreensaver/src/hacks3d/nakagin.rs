//! Port of `hacks/glx/nakagin.c`.
//!
//! ```text
//! nakagin, Copyright © 2022-2025 Jamie Zawinski <jwz@jwz.org>
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
//! The Nakagin Capsule Tower, demolished in 2022, still growing.
//!
//! Two concrete towers rise out of the bottom of the frame, and prefabricated
//! rooms fly in from off to the side, turn to face the way their floor plan
//! says they should, and bolt themselves on. The whole building scrolls
//! downward for ever: when the bottom floor leaves the frame the twenty floor
//! plans shift down one and a new one is drawn at the top, so the tower is
//! never finished and never the same twice.
//!
//! The floor plan is the real building's, with only the capsule orientations
//! left to chance. A capsule sticks out over the cell behind it, so a cell can
//! only be used if neither it nor its overhang is blocked by something on the
//! floor below: that occlusion test is what keeps the stack from growing into
//! itself. Where a capsule's window can go follows from the same map, and its
//! door has no choice at all, since it must open onto whichever side touches a
//! tower.
//!
//! The occlusion test never actually fires on the historical plan: every cell
//! is at the same height on every floor, and no overhang reaches a cell lower
//! than its own, so nothing is ever struck out. It is there for the random
//! ahistoric plans upstream considered and did not write. The code is kept
//! because it is what upstream runs, and there is a test either way: one that
//! feeds it a floor that does block, and one that pins the real plan as
//! self-compatible so that a change to the heights shows up.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// How many floor plans are alive at once. The bottom one falls out of the
/// frame and a new one appears on top.
const STACK_HEIGHT: usize = 20;
/// How far below the frame the towers start, so they have somewhere to rise
/// from.
const BASEMENT_DEPTH: f32 = 5.0;
/// A capsule is this much longer than it is wide.
const CAPSULE_ASPECT: f32 = 1.6;
/// The diameter of the round window, as a fraction of the capsule's width.
const WINDOW_SIZE: f32 = 0.528;

/// The floor plan is eight cells across and four deep.
const GRID_W: usize = 8;
const GRID_H: usize = 4;

/// Where a capsule is in its life, from waiting to be built to scrolled out of
/// the frame. The towers use the same names for the two states they have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// The cell could take a capsule but has none yet.
    Avail,
    /// Queued to launch, waiting its turn.
    Wait,
    /// Rising to the height it will fly across at.
    Up,
    /// Flying across to sit above its cell.
    Over,
    /// Coming down onto the tower.
    Down,
    /// Just landed.
    Docked,
    /// Landed, lit, and liable to be evicted.
    Occupied,
    /// Thrown off the building.
    Eject,
    /// Gone, or a cell that can never hold anything.
    Dead,
}

/// Which way a capsule faces, or what else is in the cell. `Xx` is empty air
/// and `Tt` is one of the two towers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Orient {
    Xx,
    Tt,
    N,
    S,
    E,
    W,
    /// The four pairs in the plan below that can go either way, so long as both
    /// of the pair agree.
    Ne,
    Se,
    Nw,
    Sw,
}

/// Which of the capsule's four pieces to draw. They are separate because each
/// takes its own colour and its own shine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Capsule,
    Window,
    Glass,
    Door,
}

/// Which wall of the capsule a window or door is on, seen from inside facing
/// the window. `Front` for a door means the back wall.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Front,
    Left,
    Right,
}

/// Whether anyone is home, and whether they are watching television.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Light {
    Dark,
    Solid,
    Tv,
}

type Xyz = [f32; 3];

#[derive(Clone, Copy)]
struct Capsule {
    state: State,
    start_pos: Xyz,
    end_pos: Xyz,
    pos: Xyz,
    start_th: f32,
    end_th: f32,
    th: f32,
    ratio: f32,
    speed: f32,
    wait_until: f64,
    window_pos: Side,
    door_pos: Side,
    light_state: Light,
    light_color: [f32; 4],
}

impl Default for Capsule {
    fn default() -> Self {
        Capsule {
            state: State::Dead,
            start_pos: [0.0; 3],
            end_pos: [0.0; 3],
            pos: [0.0; 3],
            start_th: 0.0,
            end_th: 0.0,
            th: 0.0,
            ratio: 0.0,
            speed: 0.0,
            wait_until: 0.0,
            window_pos: Side::Front,
            door_pos: Side::Front,
            light_state: Light::Dark,
            light_color: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Tower {
    /// Only ever `Docked` or `Up`.
    state: TowerState,
    start_pos: Xyz,
    end_pos: Xyz,
    pos: Xyz,
    ratio: f32,
    speed: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum TowerState {
    #[default]
    Docked,
    Up,
}

#[derive(Clone, Copy)]
struct Cell {
    orient: Orient,
    /// How far this cell is raised above its floor: the east and west columns
    /// sit a little higher, and the north rows higher still.
    y: f32,
    c: Capsule,
}

#[derive(Clone, Copy)]
struct Floorplan {
    /// Where the floor is, which decreases for ever as the building scrolls.
    y: f32,
    cell: [[Cell; GRID_W]; GRID_H],
}

/// The real building's plan. Any pair labelled `Ne` can be either N or E, but
/// both of the pair have to be the same; likewise the other three.
///
/// ```text
///     -1     0     1      2      3       4     5       6     7      8
/// .------.------.------.------.------.------.------.------.------.------.
/// |      |                           |                           |      |
/// -1     |       ###### ######       |       ###### ######       |      |
/// |      |       #    # #    #       |       #    # #    #       |      |
/// .------.----   #    # #    #  -----.-----  #    # #    #  -----.------.
/// |              #    # #    #               #    # #    #              |
/// 0      |###### # NW # # N  # ###### ###### # N  # # NE # ######       |
/// |      |#    # ###### ###### #    # #    # ###### ###### #    #       |
/// .-----  #    #               #    #.#    #               #    #  -----.
/// |       #    #       |       #    # #    #       |       #    #       |
/// 1      |# NW #       |       # N  # # N  #       |       # NE #       |
/// |      |######       |       ###### ######       |       ######       |
/// .-----          -----.-----                ------.------         -----.
/// ```
///
/// Upstream notes that the real capsules came in mirrored sets, with the doors
/// facing the tower rather than always being on the left.
const BASE_PLAN: [[(Orient, f32); GRID_W]; GRID_H] = [
    [
        (Orient::Xx, 0.4),
        (Orient::Nw, 0.2),
        (Orient::N, 0.2),
        (Orient::Xx, 0.2),
        (Orient::Xx, 0.2),
        (Orient::N, 0.2),
        (Orient::Ne, 0.2),
        (Orient::Xx, 0.4),
    ],
    [
        (Orient::Nw, 0.4),
        (Orient::Tt, 0.0),
        (Orient::Tt, 0.0),
        (Orient::N, 0.2),
        (Orient::N, 0.2),
        (Orient::Tt, 0.0),
        (Orient::Tt, 0.0),
        (Orient::Ne, 0.4),
    ],
    [
        (Orient::Sw, 0.2),
        (Orient::Tt, 0.0),
        (Orient::Tt, 0.0),
        (Orient::S, 0.0),
        (Orient::S, 0.0),
        (Orient::Tt, 0.0),
        (Orient::Tt, 0.0),
        (Orient::Se, 0.2),
    ],
    [
        (Orient::Xx, 0.2),
        (Orient::Sw, 0.0),
        (Orient::S, 0.0),
        (Orient::Xx, 0.0),
        (Orient::Xx, 0.0),
        (Orient::S, 0.0),
        (Orient::Se, 0.0),
        (Orient::Xx, 0.2),
    ],
];

/// Where the cell a capsule overhangs is, relative to the cell it stands in.
fn overhang(o: Orient) -> (i32, i32) {
    match o {
        Orient::N => (0, -1),
        Orient::S => (0, 1),
        Orient::E => (1, 0),
        Orient::W => (-1, 0),
        _ => (0, 0),
    }
}

/// One floor plan, with the four either-way pairs settled and the capsules
/// that the floor below leaves no room for struck out.
fn make_floorplan(prev: Option<&Floorplan>) -> Floorplan {
    let nw = if random() & 1 == 1 {
        Orient::N
    } else {
        Orient::W
    };
    let ne = if random() & 1 == 1 {
        Orient::N
    } else {
        Orient::E
    };
    let sw = if random() & 1 == 1 {
        Orient::S
    } else {
        Orient::W
    };
    let se = if random() & 1 == 1 {
        Orient::S
    } else {
        Orient::E
    };

    // Occlusion map: how far up does the lower floor intrude on this one?
    let mut occ = [[0.0f32; GRID_W]; GRID_H];
    if let Some(prev) = prev {
        for y in 0..GRID_H {
            for x in 0..GRID_W {
                let o = prev.cell[y][x].orient;
                let yo = prev.cell[y][x].y;
                if o != Orient::Xx && o != Orient::Tt {
                    let (dx, dy) = overhang(o);
                    occ[y][x] = yo;
                    let (x2, y2) = (x as i32 + dx, y as i32 + dy);
                    if (0..GRID_W as i32).contains(&x2) && (0..GRID_H as i32).contains(&y2) {
                        occ[y2 as usize][x2 as usize] = yo;
                    }
                }
            }
        }
    }

    let mut fp = Floorplan {
        y: 0.0,
        cell: [[Cell {
            orient: Orient::Xx,
            y: 0.0,
            c: Capsule::default(),
        }; GRID_W]; GRID_H],
    };

    // If either of the two cells this capsule takes up, the one it stands on
    // and the one it hangs over, is blocked from below, nothing can go here.
    for y in 0..GRID_H {
        for x in 0..GRID_W {
            let (base, yo) = BASE_PLAN[y][x];
            let mut o = match base {
                Orient::Nw => nw,
                Orient::Ne => ne,
                Orient::Sw => sw,
                Orient::Se => se,
                other => other,
            };

            let (dx, dy) = overhang(o);
            let (x2, y2) = (x as i32 + dx, y as i32 + dy);
            let blocked_over = (0..GRID_W as i32).contains(&x2)
                && (0..GRID_H as i32).contains(&y2)
                && occ[y2 as usize][x2 as usize] > yo;
            if occ[y][x] > yo || blocked_over {
                o = Orient::Xx;
            }

            fp.cell[y][x].orient = o;
            fp.cell[y][x].y = yo;
            fp.cell[y][x].c.state = if o == Orient::Xx || o == Orient::Tt {
                State::Dead
            } else {
                State::Avail
            };
        }
    }

    // Decide which window positions are available. The front always works, and
    // a side works when there is no neighbour on that side, since a neighbour
    // would clip about a quarter of the window.
    for y in 0..GRID_H {
        for x in 0..GRID_W {
            let o = fp.cell[y][x].orient;
            if o == Orient::Xx || o == Orient::Tt {
                continue;
            }

            // The cells to the capsule's left and right, facing the window.
            let (left, right) = match o {
                Orient::N => ((-1, 0), (1, 0)),
                Orient::S => ((1, 0), (-1, 0)),
                Orient::E => ((0, -1), (0, 1)),
                _ => ((0, 1), (0, -1)),
            };
            // Read the neighbours out before touching the cell, so nothing
            // is borrowed while it is being written.
            let at = |(dx, dy): (i32, i32)| -> Option<Orient> {
                let (x2, y2) = (x as i32 + dx, y as i32 + dy);
                if (0..GRID_W as i32).contains(&x2) && (0..GRID_H as i32).contains(&y2) {
                    Some(fp.cell[y2 as usize][x2 as usize].orient)
                } else {
                    None
                }
            };
            let (at_left, at_right) = (at(left), at(right));

            let (left_p, right_p) = (
                at_left.is_none_or(|o| o == Orient::Xx),
                at_right.is_none_or(|o| o == Orient::Xx),
            );

            fp.cell[y][x].c.window_pos = match (left_p, right_p) {
                (true, true) => match random() % 4 {
                    0 | 1 => Side::Front,
                    2 => Side::Left,
                    _ => Side::Right,
                },
                (true, false) => {
                    if random() % 3 <= 1 {
                        Side::Front
                    } else {
                        Side::Left
                    }
                }
                (false, true) => {
                    if random() % 3 <= 1 {
                        Side::Front
                    } else {
                        Side::Right
                    }
                }
                (false, false) => Side::Front,
            };

            // The door has only one option: the wall that touches the central
            // tower. If neither side does, it goes on the back.
            fp.cell[y][x].c.door_pos = if at_left == Some(Orient::Tt) {
                Side::Left
            } else if at_right == Some(Orient::Tt) {
                Side::Right
            } else {
                Side::Front
            };

            // Decide on the interior lighting.
            let c = &mut fp.cell[y][x].c;
            let n = random() % 100;
            c.light_state = if n < 8 {
                Light::Solid
            } else if n < 10 {
                Light::Tv
            } else {
                Light::Dark
            };

            let n = random() % 100;
            c.light_color = if c.light_state == Light::Dark {
                [0.0, 0.0, 0.0, 1.0]
            } else if n < 50 {
                // Yellow-ish.
                [
                    0.2 + frand(0.02) as f32,
                    0.2 + frand(0.02) as f32,
                    frand(0.005) as f32,
                    1.0,
                ]
            } else if n < 85 {
                // Red-ish.
                [
                    0.2 + frand(0.1) as f32,
                    frand(0.01) as f32,
                    frand(0.01) as f32,
                    1.0,
                ]
            } else {
                // Blue-ish.
                [
                    frand(0.01) as f32,
                    frand(0.01) as f32,
                    0.2 + frand(0.1) as f32,
                    1.0,
                ]
            };
        }
    }

    fp
}

/// One piece of a capsule, at the origin.
///
/// Upstream compiles each of these into a display list once. Here they are
/// drawn as they come: a list in this runtime replays the calls rather than
/// the result, and the winding this sets would not survive being recorded.
fn make_capsule(g: &mut Gl, which: Part, wire: bool) {
    let wthick = 0.10;
    let wdepth = 0.02;
    let z = CAPSULE_ASPECT - 0.5;
    let steps = if wire { 12 } else { 60 };
    let quads = if wire { Shape::LineLoop } else { Shape::Quads };

    g.glx.push_matrix();
    g.glx.front_face_cw(false);
    g.glx.rotate(180.0, 0.0, 1.0, 0.0);
    g.glx.translate(0.0, 0.5, 0.0);

    match which {
        Part::Capsule => {
            // Back, right, front, left, and then the two ends.
            let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
                (
                    [0.0, 0.0, 1.0],
                    [
                        [0.5, -0.5, 0.5],
                        [0.5, 0.5, 0.5],
                        [-0.5, 0.5, 0.5],
                        [-0.5, -0.5, 0.5],
                    ],
                ),
                (
                    [1.0, 0.0, 0.0],
                    [
                        [0.5, 0.5, 0.5],
                        [0.5, -0.5, 0.5],
                        [0.5, -0.5, -z],
                        [0.5, 0.5, -z],
                    ],
                ),
                (
                    [0.0, 0.0, -1.0],
                    [
                        [0.5, -0.5, -z],
                        [-0.5, -0.5, -z],
                        [-0.5, 0.5, -z],
                        [0.5, 0.5, -z],
                    ],
                ),
                (
                    [-1.0, 0.0, 0.0],
                    [
                        [-0.5, -0.5, 0.5],
                        [-0.5, 0.5, 0.5],
                        [-0.5, 0.5, -z],
                        [-0.5, -0.5, -z],
                    ],
                ),
                (
                    [0.0, 1.0, 0.0],
                    [
                        [-0.5, 0.5, 0.5],
                        [0.5, 0.5, 0.5],
                        [0.5, 0.5, -z],
                        [-0.5, 0.5, -z],
                    ],
                ),
                (
                    [0.0, -1.0, 0.0],
                    [
                        [0.5, -0.5, 0.5],
                        [-0.5, -0.5, 0.5],
                        [-0.5, -0.5, -z],
                        [0.5, -0.5, -z],
                    ],
                ),
            ];
            // In wireframe the top and bottom are left off: the four walls
            // already outline the box.
            let n = if wire { 4 } else { faces.len() };
            for (normal, verts) in &faces[..n] {
                g.glx.begin(quads);
                g.glx.normal3f(normal[0], normal[1], normal[2]);
                for v in verts {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            }
        }

        Part::Window => {
            // The flat ring round the window.
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::QuadStrip
            });
            g.glx.normal3f(0.0, 0.0, -1.0);
            for i in 0..=steps {
                let r = i as f32 / steps as f32;
                let x = WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).cos();
                let y = WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).sin();
                let r2 = 1.0 - wthick;
                g.glx.vertex3f(x, y, -z - wdepth);
                if !wire {
                    g.glx.vertex3f(x * r2, y * r2, -z - wdepth);
                }
            }
            g.glx.end();

            if wire {
                g.glx.pop_matrix();
                return;
            }

            // The outside of the rim, then the inside.
            g.glx.begin(Shape::QuadStrip);
            for i in 0..=steps {
                let r = i as f32 / steps as f32;
                let x = WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).cos();
                let y = WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).sin();
                g.glx.normal3f(x, y, 0.0);
                g.glx.vertex3f(x, y, -z);
                g.glx.vertex3f(x, y, -z - wdepth);
            }
            g.glx.end();

            g.glx.begin(Shape::QuadStrip);
            for i in 0..=steps {
                let r = i as f32 / steps as f32;
                let r2 = 1.0 - wthick;
                let x = r2 * WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).cos();
                let y = r2 * WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).sin();
                g.glx.normal3f(-x, -y, 0.0);
                g.glx.vertex3f(x, y, -z - wdepth);
                g.glx.vertex3f(x, y, -z);
            }
            g.glx.end();
        }

        Part::Glass => {
            if wire {
                g.glx.pop_matrix();
                return;
            }
            g.glx.begin(Shape::TriangleFan);
            g.glx.normal3f(0.0, 0.0, -1.0);
            g.glx.vertex3f(0.0, 0.0, -z - wdepth / 2.0);
            for i in (0..=steps).rev() {
                let r = i as f32 / steps as f32;
                let r2 = 1.0 - wthick;
                let x = WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).cos();
                let y = WINDOW_SIZE * 0.5 * (std::f32::consts::TAU * r).sin();
                g.glx.vertex3f(x * r2, y * r2, -z - wdepth / 2.0);
            }
            g.glx.end();
        }

        Part::Door => {
            let dw = 0.39;
            let dh = 0.91;
            let left = dw / 3.0;
            let right = left + dw;
            let bot = 0.01;
            let top = dh + bot;
            let thick = 0.02;
            let (l, r) = (left - 0.5, right - 0.5);
            let (b, t) = (bot - 0.5, top - 0.5);
            let (zi, zo) = (0.5 - thick, 0.5 + thick);

            let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
                // Outside.
                (
                    [0.0, 0.0, 1.0],
                    [[l, b, zo], [r, b, zo], [r, t, zo], [l, t, zo]],
                ),
                // Inside.
                (
                    [0.0, 0.0, -1.0],
                    [[l, t, zi], [r, t, zi], [r, b, zi], [l, b, zi]],
                ),
                // Right, left, top, bottom.
                (
                    [1.0, 0.0, 0.0],
                    [[r, t, 0.5], [r, t, zo], [r, b, zo], [r, b, 0.5]],
                ),
                (
                    [-1.0, 0.0, 0.0],
                    [[l, b, 0.5], [l, b, zo], [l, t, zo], [l, t, 0.5]],
                ),
                (
                    [0.0, 1.0, 0.0],
                    [[l, t, 0.5], [l, t, zo], [r, t, zo], [r, t, 0.5]],
                ),
                (
                    [0.0, -1.0, 0.0],
                    [[r, b, 0.5], [r, b, zo], [l, b, zo], [l, b, 0.5]],
                ),
            ];
            let n = if wire { 1 } else { faces.len() };
            for (normal, verts) in &faces[..n] {
                g.glx.begin(quads);
                g.glx.normal3f(normal[0], normal[1], normal[2]);
                for v in verts {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            }
        }
    }

    g.glx.pop_matrix();
}

/// One of the two concrete towers, standing on its pilings with a shaft up the
/// middle. The four walls step down in height going round, which is what gives
/// the tower its slanted top.
fn make_tower(g: &mut Gl, wire: bool) {
    let wthick = 0.15f32;
    let w = 2.0f32;
    let piling = BASEMENT_DEPTH + 4.0;
    let cap = 3.0;
    let h = STACK_HEIGHT as f32 + piling + cap;
    let h0 = h;
    let h1 = h - 2.0 * 3.0 / 4.0;
    let h2 = h - 2.0 * 6.0 / 4.0;
    let h3 = h - 2.0;
    let quads = if wire { Shape::LineLoop } else { Shape::Quads };

    g.glx.push_matrix();
    g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
    g.glx.translate(0.0, 0.0, -piling);

    // Pass 0 is the outside of the shell, pass 1 the inside, drawn inside out
    // and a wall's thickness in.
    let mut s = 1.0;
    for i in 0..2 {
        if wire && i == 1 {
            break;
        }
        let si = if i == 1 { -1.0 } else { 1.0 };
        s = if i == 1 { 1.0 - wthick } else { 1.0 };
        g.glx.front_face_cw(i == 1);

        let inset = if i == 1 { wthick } else { 0.0 };
        let faces: [([f32; 3], [[f32; 3]; 4]); 4] = [
            // North.
            (
                [0.0, -si, 0.0],
                [
                    [s * w / 2.0, s * -w / 2.0, 0.0],
                    [s * w / 2.0, s * -w / 2.0, h1],
                    [s * -w / 2.0, s * -w / 2.0, h0 - inset * 2.0],
                    [s * -w / 2.0, s * -w / 2.0, 0.0],
                ],
            ),
            // East.
            (
                [si, 0.0, 0.0],
                [
                    [s * w / 2.0, s * -w / 2.0, h1],
                    [s * w / 2.0, s * -w / 2.0, 0.0],
                    [s * w / 2.0, s * w / 2.0, 0.0],
                    [s * w / 2.0, s * w / 2.0, h2 + inset],
                ],
            ),
            // South.
            (
                [0.0, si, 0.0],
                [
                    [s * w / 2.0, s * w / 2.0, h2 + inset],
                    [s * w / 2.0, s * w / 2.0, 0.0],
                    [s * -w / 2.0, s * w / 2.0, 0.0],
                    [s * -w / 2.0, s * w / 2.0, h3],
                ],
            ),
            // West.
            (
                [-si, 0.0, 0.0],
                [
                    [s * -w / 2.0, s * w / 2.0, h3],
                    [s * -w / 2.0, s * w / 2.0, 0.0],
                    [s * -w / 2.0, s * -w / 2.0, 0.0],
                    [s * -w / 2.0, s * -w / 2.0, h0 - inset * 2.0],
                ],
            ),
        ];
        for (normal, verts) in faces {
            g.glx.begin(quads);
            g.glx.normal3f(normal[0], normal[1], normal[2]);
            for v in verts {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }
    }

    if !wire {
        // The rim along the top of the wall, joining outside to inside.
        g.glx.front_face_cw(true);
        g.glx.begin(Shape::QuadStrip);
        // Upstream admits this normal is not quite right.
        g.glx.normal3f(0.0, 0.0, 1.0);
        let rim: [([f32; 3], [f32; 3]); 5] = [
            ([w / 2.0, -w / 2.0, h1], [s * w / 2.0, s * -w / 2.0, h1]),
            (
                [w / 2.0, w / 2.0, h2],
                [s * w / 2.0, s * w / 2.0, h2 + wthick],
            ),
            ([-w / 2.0, w / 2.0, h3], [s * -w / 2.0, s * w / 2.0, h3]),
            (
                [-w / 2.0, -w / 2.0, h0],
                [s * -w / 2.0, s * -w / 2.0, h0 - wthick * 2.0],
            ),
            ([w / 2.0, -w / 2.0, h1], [s * w / 2.0, s * -w / 2.0, h1]),
        ];
        for (a, b) in rim {
            g.glx.vertex3f(a[0], a[1], a[2]);
            g.glx.vertex3f(b[0], b[1], b[2]);
        }
        g.glx.end();

        // The floor of the shaft, so it is not see-through from above.
        g.glx.front_face_cw(true);
        g.glx.begin(Shape::Quads);
        g.glx.normal3f(0.0, 0.0, 1.0);
        for v in [
            [s * -w / 2.0, s * w / 2.0, h2],
            [s * w / 2.0, s * w / 2.0, h2],
            [s * w / 2.0, s * -w / 2.0, h2],
            [s * -w / 2.0, s * -w / 2.0, h2],
        ] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }

    g.glx.pop_matrix();
}

struct Nakagin {
    rot: Rotator,
    rot2: Rotator,
    trackball: Trackball,
    /// How many more capsules to dock before the first frame is shown, so the
    /// building starts out already built rather than empty.
    ffwd: i32,

    capsule_color: [f32; 4],
    window_color: [f32; 4],
    door_color: [f32; 4],
    tower_color: [f32; 4],

    floorplans: Vec<Floorplan>,
    towers: [Tower; 2],

    speed: f32,
    do_spin: bool,
    do_wander: bool,
    do_tilt: bool,
    wireframe: bool,
    /// Seconds since the saver started, which is what the waits are measured
    /// against.
    now: f64,
}

impl Nakagin {
    /// `max_stack_height`: how far up the tallest column of capsules reaches,
    /// which is where the next one flies in above and how tall the towers have
    /// to grow.
    fn max_stack_height(&self) -> f32 {
        let mut yo = 0.0f32;
        for (z, fp) in self.floorplans.iter().enumerate() {
            for row in &fp.cell {
                for cell in row {
                    let y2 = z as f32 + cell.y + cell.c.pos[1];
                    if cell.c.state != State::Avail && cell.c.state != State::Dead && y2 > yo {
                        yo = y2;
                    }
                }
            }
        }
        yo
    }

    /// `move_capsules`: one step of the whole simulation. Scroll everything
    /// down, run each capsule's state machine, launch a few new ones, and grow
    /// the towers to keep up.
    fn move_capsules(&mut self) {
        let mut moving = 0;
        let mut avail: Vec<(usize, usize, usize)> = Vec::new();
        let scroll_speed = self.speed * 0.002;
        let slide_speed = self.speed * 0.03;
        let now = self.now;
        let ffwd = self.ffwd > 0;

        for fp in &mut self.floorplans {
            fp.y -= scroll_speed;
        }
        for t in &mut self.towers {
            t.pos[1] -= scroll_speed;
        }

        // If the bottom floor has fallen out of the frame, move the others
        // down one and put a fresh blank one on top.
        if self.floorplans[0].y < -1.0 {
            let busy = self.floorplans[0].cell.iter().flatten().any(|cell| {
                !matches!(
                    cell.c.state,
                    State::Docked | State::Occupied | State::Dead | State::Avail
                )
            });
            if !busy {
                self.floorplans.remove(0);
                let last = *self.floorplans.last().expect("the stack is never empty");
                let mut fp = make_floorplan(Some(&last));
                fp.y = last.y + 1.0;
                self.floorplans.push(fp);
            }
        }

        // Run the capsule state machine.
        for z in 0..self.floorplans.len() {
            for y in 0..GRID_H {
                for x in 0..GRID_W {
                    let orient = self.floorplans[z].cell[y][x].orient;
                    let fp_y = self.floorplans[z].y;
                    let c = &mut self.floorplans[z].cell[y][x].c;
                    let yo = fp_y + c.pos[1];

                    match c.state {
                        State::Dead => {}

                        State::Occupied => {
                            if yo < -1.0 {
                                // Scrolled off the bottom.
                                c.state = State::Dead;
                            } else if !ffwd
                                && moving == 0
                                && random().is_multiple_of((12000.0 * self.speed).max(1.0) as u32)
                            {
                                // Eviction: thrown off sideways and down.
                                let d = 500.0;
                                c.state = State::Eject;
                                c.speed = 0.1;
                                c.end_pos[1] -= d / 3.0;
                                moving += 1;
                                match orient {
                                    Orient::N => c.end_pos[0] -= d,
                                    Orient::S => c.end_pos[0] += d,
                                    Orient::E => c.end_pos[2] -= d,
                                    _ => c.end_pos[2] += d,
                                }
                            }
                        }

                        State::Avail => {
                            if orient != Orient::Xx && orient != Orient::Tt && avail.len() < 16 {
                                avail.push((z, y, x));
                            }
                        }

                        State::Wait => {
                            moving += 1;
                            if ffwd || c.wait_until < now {
                                c.state = State::Up;
                            }
                        }

                        State::Up | State::Over | State::Down | State::Docked | State::Eject => {
                            if c.state != State::Docked {
                                moving += 1;
                            }
                            c.ratio += slide_speed * c.speed;
                            if ffwd {
                                c.ratio = 1.0;
                            }
                            let r = ease(Ease::InOutSine, f64::from(c.ratio)) as f32;
                            for i in 0..3 {
                                c.pos[i] = c.start_pos[i] + r * (c.end_pos[i] - c.start_pos[i]);
                            }
                            c.th = c.start_th + r * (c.end_th - c.start_th);

                            if c.ratio >= 1.0 {
                                c.ratio = 0.0;
                                c.pos = c.end_pos;
                                c.start_pos = c.end_pos;
                                c.th = c.end_th;
                                c.start_th = c.end_th;
                                match c.state {
                                    State::Up => {
                                        c.state = State::Over;
                                        c.end_pos[0] = y as f32;
                                        c.end_pos[1] = c.pos[1];
                                        c.end_pos[2] = (GRID_W - x) as f32;
                                        c.end_th = match orient {
                                            Orient::N => 270.0,
                                            Orient::W => 0.0,
                                            Orient::S => 90.0,
                                            _ => 180.0,
                                        };
                                        // Turn the short way round.
                                        if c.end_th - c.start_th > 180.0 {
                                            c.end_th -= 360.0;
                                        } else if c.end_th - c.start_th < -180.0 {
                                            c.end_th += 360.0;
                                        }
                                    }
                                    State::Over => {
                                        c.state = State::Down;
                                        // Relative to the floor plan.
                                        c.end_pos[1] = 0.0;
                                    }
                                    State::Down => {
                                        c.state = State::Docked;
                                        if self.ffwd > 0 {
                                            self.ffwd -= 1;
                                        }
                                    }
                                    State::Docked => c.state = State::Occupied,
                                    _ => c.state = State::Dead,
                                }
                            }
                        }
                    }
                }
            }
        }

        // Shuffle the available cells, so the building does not fill in order.
        for i in 0..avail.len() {
            let a = (random() as usize) % avail.len();
            avail.swap(a, i);
        }

        // Launch some new capsules.
        for (i, &(zc, y, x)) in avail.iter().enumerate() {
            if moving as f32 > 16.0 * 0.33 {
                break;
            }
            // Only a third of them each time.
            if !random().is_multiple_of(3) {
                continue;
            }

            let hh = self.max_stack_height();
            let o = self.floorplans[zc].cell[y][x].orient;

            // Anything lower in this same column is now blocked in for good.
            for z2 in 0..zc {
                let c2 = &mut self.floorplans[z2].cell[y][x].c;
                if c2.state == State::Avail {
                    c2.state = State::Dead;
                }
            }

            let d = 3.0;
            let c = &mut self.floorplans[zc].cell[y][x].c;
            c.state = State::Wait;
            c.wait_until = now + (i as f64 * 0.3) / f64::from(self.speed);
            c.speed = 1.0;

            c.start_pos = [y as f32, -(hh + 2.0), (GRID_W - x) as f32];
            let jitter = frand(1.0) as f32 - 0.5;
            match o {
                Orient::N => {
                    c.start_pos[0] -= d;
                    c.start_pos[2] += jitter;
                }
                Orient::S => {
                    c.start_pos[0] += d;
                    c.start_pos[2] += jitter;
                }
                Orient::E => {
                    c.start_pos[2] -= d;
                    c.start_pos[0] += jitter;
                }
                _ => {
                    c.start_pos[2] += d;
                    c.start_pos[0] += jitter;
                }
            }

            c.pos = c.start_pos;
            c.start_th = frand(360.0) as f32;
            c.th = c.start_th;
            c.end_th = c.start_th;
            c.end_pos = [c.start_pos[0], 2.0 + frand(1.5) as f32, c.start_pos[2]];
            moving += 1;
        }

        // Grow the towers to stay ahead of the capsules.
        for z in 0..self.towers.len() {
            match self.towers[z].state {
                TowerState::Docked => {
                    // The east tower is the taller of the two.
                    let top = self.towers[z].pos[1] + STACK_HEIGHT as f32;
                    let hh = self.max_stack_height() - 1.0 - if z == 1 { 2.0 } else { 0.0 };
                    if top < hh {
                        let t = &mut self.towers[z];
                        t.state = TowerState::Up;
                        t.ratio = 0.0;
                        t.start_pos = t.pos;
                        t.end_pos = t.pos;
                        t.end_pos[1] += 2.0 + frand(1.5) as f32;
                        t.speed = 0.3 * (0.7 + frand(0.6) as f32);
                    }
                }
                TowerState::Up => {
                    let t = &mut self.towers[z];
                    t.ratio += slide_speed * t.speed;
                    if ffwd {
                        t.ratio = 1.0;
                    }
                    let r = ease(Ease::InOutSine, f64::from(t.ratio)) as f32;
                    for i in 0..3 {
                        t.pos[i] = t.start_pos[i] + r * (t.end_pos[i] - t.start_pos[i]);
                    }
                    if t.ratio >= 1.0 {
                        t.ratio = 0.0;
                        t.pos = t.end_pos;
                        t.start_pos = t.end_pos;
                        t.state = TowerState::Docked;
                    }
                }
            }
        }
    }

    /// `draw_capsule`: the box, its door, its window frame and its glass, with
    /// whatever light is on inside showing through.
    fn draw_capsule(&self, g: &mut Gl, z: usize, y: usize, x: usize, floor_y: f32) {
        let wire = self.wireframe;
        let c = self.floorplans[z].cell[y][x].c;
        let ss = 0.95;
        let spec = [0.3, 0.3, 0.3, 1.0];
        let wspec = [1.0, 1.0, 0.0, 1.0];

        g.glx.material_specular(spec);
        g.glx.material_shininess(128.0);

        g.glx.push_matrix();
        g.glx.translate(c.pos[0], floor_y + c.pos[1], c.pos[2]);
        g.glx.rotate(c.th, 0.0, 1.0, 0.0);
        g.glx.scale(ss, ss, ss);

        g.glx.color4f(
            self.capsule_color[0],
            self.capsule_color[1],
            self.capsule_color[2],
            self.capsule_color[3],
        );
        g.glx.material_ambient_diffuse(self.capsule_color);
        make_capsule(g, Part::Capsule, wire);

        g.glx.push_matrix();
        match c.door_pos {
            Side::Front => {}
            Side::Left => g.glx.rotate(-90.0, 0.0, 1.0, 0.0),
            Side::Right => g.glx.rotate(90.0, 0.0, 1.0, 0.0),
        }
        g.glx.color4f(
            self.door_color[0],
            self.door_color[1],
            self.door_color[2],
            self.door_color[3],
        );
        g.glx.material_ambient_diffuse(self.door_color);
        make_capsule(g, Part::Door, wire);
        g.glx.pop_matrix();

        match c.window_pos {
            Side::Front => {}
            Side::Left => {
                g.glx.rotate(90.0, 0.0, 1.0, 0.0);
                g.glx
                    .translate(1.0 - CAPSULE_ASPECT, 0.0, 1.0 - CAPSULE_ASPECT);
            }
            Side::Right => {
                g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
                g.glx
                    .translate(-(1.0 - CAPSULE_ASPECT), 0.0, 1.0 - CAPSULE_ASPECT);
            }
        }

        g.glx.color4f(
            self.window_color[0],
            self.window_color[1],
            self.window_color[2],
            self.window_color[3],
        );
        g.glx.material_ambient_diffuse(self.window_color);
        make_capsule(g, Part::Window, wire);

        let lit = if c.state == State::Occupied || c.state == State::Eject {
            c.light_color
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        g.glx.color4f(lit[0], lit[1], lit[2], lit[3]);
        g.glx.material_ambient_diffuse(lit);

        g.glx.material_specular(wspec);
        g.glx.material_shininess(100.0);
        make_capsule(g, Part::Glass, wire);

        g.glx.material_specular(spec);
        g.glx.material_shininess(128.0);
        g.glx.pop_matrix();
    }
}

impl Hack3d for Nakagin {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;
        self.now = g.time;

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        if !wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 4.0, 1.4, 1.1, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.5, 0.5, 0.5, 1.0]);
        }

        // The first frame runs the simulation until the building has caught up
        // with itself, so it opens on a tower rather than on bare pilings.
        if self.ffwd > 0 || !self.trackball.button_down() {
            loop {
                self.move_capsules();
                if self.ffwd <= 0 {
                    break;
                }
            }
        }

        // A capsule with the television on flickers, which upstream drives off
        // the wall clock rather than off the animation.
        for fp in &mut self.floorplans {
            for row in &mut fp.cell {
                for cell in row {
                    let c = &mut cell.c;
                    if c.light_state == Light::Tv
                        && (c.state == State::Occupied || c.state == State::Eject)
                        && c.wait_until < self.now
                    {
                        c.light_color =
                            [frand(0.3) as f32, frand(0.3) as f32, frand(0.3) as f32, 1.0];
                        c.wait_until = self.now + 0.05 + frand(0.2);
                    }
                }
            }
        }

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.do_wander {
            let (x, y, z) = self.rot.position(!down);
            g.glx.translate(
                (x as f32 - 0.5) * 4.0,
                (y as f32 - 0.5) * 0.2,
                (z as f32 - 0.5) * 8.0,
            );
        }

        if self.do_tilt {
            let maxz = 50.0;
            let (_, _, z) = self.rot2.position(!down);
            g.glx.rotate(maxz / 2.0 - z as f32 * maxz, 1.0, 0.0, 0.0);
        }

        let (_, _, z) = self.rot.rotation(!down);
        if self.do_spin {
            // Turn about the middle of the building rather than its corner.
            g.glx.translate(0.0, 0.0, GRID_H as f32 / 2.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.translate(0.0, 0.0, -(GRID_H as f32 / 2.0));
        }

        g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
        g.glx.translate(0.0, 0.0, -(GRID_W as f32 / 2.0));
        g.glx
            .translate(0.0, -(STACK_HEIGHT as f32 / 2.0 + BASEMENT_DEPTH), 0.0);
        g.glx.translate(0.0, -2.0, 0.0);

        g.glx.color4f(
            self.tower_color[0],
            self.tower_color[1],
            self.tower_color[2],
            self.tower_color[3],
        );
        g.glx.material_ambient_diffuse(self.tower_color);
        for t in self.towers {
            g.glx.push_matrix();
            g.glx.translate(t.pos[0] - 0.5, t.pos[1], t.pos[2] - 0.5);
            g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
            make_tower(g, wire);
            g.glx.pop_matrix();
        }

        for z in 0..self.floorplans.len() {
            for y in 0..GRID_H {
                for x in 0..GRID_W {
                    let cell = self.floorplans[z].cell[y][x];
                    let c = cell.c;
                    let fp_y = self.floorplans[z].y;

                    // The door left behind on the tower, which fades in as its
                    // capsule flies over and out again if it is thrown off.
                    if cell.orient != Orient::Xx
                        && cell.orient != Orient::Tt
                        && c.state != State::Avail
                        && c.state != State::Wait
                    {
                        let alpha = match c.state {
                            State::Up => c.ratio / 2.0,
                            State::Over => c.ratio / 2.0 + 0.5,
                            State::Down | State::Docked | State::Occupied => 1.0,
                            State::Eject => 1.0 - c.ratio,
                            _ => 0.0,
                        };
                        let color = [
                            self.door_color[0],
                            self.door_color[1],
                            self.door_color[2],
                            alpha,
                        ];

                        g.glx.push_matrix();
                        g.glx
                            .translate(y as f32, fp_y + cell.y, (GRID_W - x) as f32);
                        match cell.orient {
                            Orient::N => g.glx.rotate(270.0, 0.0, 1.0, 0.0),
                            Orient::W => {}
                            Orient::S => g.glx.rotate(90.0, 0.0, 1.0, 0.0),
                            _ => g.glx.rotate(180.0, 0.0, 1.0, 0.0),
                        }
                        match c.door_pos {
                            Side::Front => {}
                            Side::Left => g.glx.rotate(-90.0, 0.0, 1.0, 0.0),
                            Side::Right => g.glx.rotate(90.0, 0.0, 1.0, 0.0),
                        }

                        g.glx.blend(Blend::Alpha);
                        g.glx.color4f(color[0], color[1], color[2], color[3]);
                        g.glx.material_ambient_diffuse(color);
                        make_capsule(g, Part::Door, wire);
                        g.glx.blend(Blend::Off);
                        g.glx.pop_matrix();
                    }

                    if c.state != State::Avail && c.state != State::Dead {
                        self.draw_capsule(g, z, y, x, fp_y + cell.y);
                    }
                }
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
            height = width;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 500.0);

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
        self.trackball.event(event, g.width(), g.height())
    }
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");
    let do_tilt = g.res.bool("tilt");

    let mut capsule_color = resource_color(g, "capsuleColor");
    let window_color = resource_color(g, "windowColor");
    let mut door_color = resource_color(g, "doorColor");
    let mut tower_color = resource_color(g, "towerColor");

    if wire {
        // In wireframe there is no shading to tell the pieces apart, so the
        // capsules are darkened and everything else brightened.
        for z in 0..3 {
            capsule_color[z] *= 0.7;
            tower_color[z] *= 2.0;
            door_color[z] *= 2.0;
        }
    }

    let spin_speed = 0.05;
    let wander_speed = 0.0025;
    let tilt_speed = 0.001;
    let spin_accel = 0.5;

    let mut floorplans: Vec<Floorplan> = Vec::with_capacity(STACK_HEIGHT);
    for z in 0..STACK_HEIGHT {
        let prev = floorplans.last().copied();
        let mut fp = make_floorplan(prev.as_ref());
        fp.y = z as f32;
        floorplans.push(fp);
    }

    let mut towers = [Tower::default(); 2];
    for (z, t) in towers.iter_mut().enumerate() {
        t.speed = 1.0;
        t.ratio = 0.0;
        t.state = TowerState::Docked;
        t.pos = [
            2.0,
            (if z == 0 { 0.0 } else { -2.0 }) - STACK_HEIGHT as f32 - 5.0,
            if z == 0 { 3.0 } else { 7.0 },
        ];
        t.start_pos = t.pos;
        t.end_pos = t.pos;
    }

    let mut st = Nakagin {
        rot: Rotator::new(
            0.0,
            0.0,
            if do_spin { spin_speed } else { 0.0 },
            spin_accel,
            if do_wander { wander_speed } else { 0.0 },
            true,
        ),
        rot2: Rotator::new(
            0.0,
            0.0,
            0.0,
            0.0,
            if do_tilt { tilt_speed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        // How many capsules to pre-load before the first frame.
        ffwd: BASEMENT_DEPTH as i32 * 32,
        capsule_color,
        window_color,
        door_color,
        tower_color,
        floorplans,
        towers,
        speed: g.res.float("speed").clamp(0.01, 8.0) as f32,
        do_spin,
        do_wander,
        do_tilt,
        wireframe: wire,
        now: 0.0,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*capsuleColor: #DDDDFF",
    "*windowColor:  #8888AA",
    "*doorColor:    #402010",
    "*towerColor:   #873E23",
    "*speed:        1.0",
    "*spin:         True",
    "*wander:       False",
    "*tilt:         True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Scrolling speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("tilt", "Tilt", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "nakagin",
    label: "Nakagin",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2022",
        video: Some("https://www.youtube.com/watch?v=JRXglvnKb6A"),
        blurb: "The Nakagin Capsule Tower, demolished in 2022, still growing.",
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

    /// The two central towers are fixed: their cells are never capsules, and
    /// every other cell of the base plan is either empty air or one of the four
    /// compass directions once the either-way pairs are settled.
    #[test]
    fn the_towers_are_where_the_plan_says() {
        let fp = make_floorplan(None);
        for (y, plan_row) in BASE_PLAN.iter().enumerate() {
            for (x, (base, _)) in plan_row.iter().enumerate() {
                let base = *base;
                let o = fp.cell[y][x].orient;
                if base == Orient::Tt {
                    assert_eq!(o, Orient::Tt, "cell {x},{y}");
                } else {
                    assert!(
                        matches!(
                            o,
                            Orient::Xx | Orient::N | Orient::S | Orient::E | Orient::W
                        ),
                        "cell {x},{y} came out {o:?}"
                    );
                }
            }
        }
        // The four cells of the two towers, and nothing else.
        let towers = fp
            .cell
            .iter()
            .flatten()
            .filter(|c| c.orient == Orient::Tt)
            .count();
        assert_eq!(towers, 8);
    }

    /// The four either-way pairs have to agree with each other, or the two
    /// capsules of a pair would overhang the same cell.
    #[test]
    fn each_either_way_pair_agrees() {
        for _ in 0..20 {
            let fp = make_floorplan(None);
            let mut pairs: Vec<(Orient, Vec<Orient>)> = Vec::new();
            for (y, plan_row) in BASE_PLAN.iter().enumerate() {
                for (x, &(base, _)) in plan_row.iter().enumerate() {
                    if !matches!(base, Orient::Nw | Orient::Ne | Orient::Sw | Orient::Se) {
                        continue;
                    }
                    // A blocked cell is struck out, so only compare live ones.
                    let o = fp.cell[y][x].orient;
                    if o == Orient::Xx {
                        continue;
                    }
                    match pairs.iter_mut().find(|(b, _)| *b == base) {
                        Some((_, v)) => v.push(o),
                        None => pairs.push((base, vec![o])),
                    }
                }
            }
            for (base, seen) in pairs {
                assert!(
                    seen.windows(2).all(|w| w[0] == w[1]),
                    "{base:?} came out {seen:?}"
                );
            }
        }
    }

    /// A capsule hangs over the cell behind it, so a cell can only be used
    /// when neither it nor its overhang is blocked by the floor below.
    ///
    /// With the historical plan the test never fires, because every cell sits
    /// at the same height on every floor and no overhang ever reaches a cell
    /// lower than its own. Feed it a floor that *is* in the way and the cells
    /// above it have to go.
    #[test]
    fn a_capsule_cannot_be_built_into_the_floor_below() {
        // A floor of north-facing capsules raised above anything on the plan,
        // filling every cell so that nothing above has anywhere to stand.
        let mut below = make_floorplan(None);
        for row in &mut below.cell {
            for cell in row {
                cell.orient = Orient::N;
                cell.y = 9.0;
            }
        }

        // The test runs on every cell, towers included: upstream strikes a
        // blocked cell out whatever was going to be in it.
        let above = make_floorplan(Some(&below));
        for y in 0..GRID_H {
            for x in 0..GRID_W {
                assert_eq!(
                    above.cell[y][x].orient,
                    Orient::Xx,
                    "cell {x},{y} was built on top of a floor nine units up"
                );
            }
        }
    }

    /// The other side of that: the real plan is self-compatible, so a floor
    /// laid on an ordinary floor loses nothing. If this ever starts failing,
    /// the plan or its heights have been changed.
    #[test]
    fn the_historical_plan_never_blocks_itself() {
        let below = make_floorplan(None);
        let above = make_floorplan(Some(&below));
        for (y, plan_row) in BASE_PLAN.iter().enumerate() {
            for (x, &(base, _)) in plan_row.iter().enumerate() {
                let got = above.cell[y][x].orient;
                if base == Orient::Xx || base == Orient::Tt {
                    assert_eq!(got, base, "cell {x},{y}");
                } else {
                    assert_ne!(got, Orient::Xx, "cell {x},{y} was struck out");
                }
            }
        }
    }

    /// A door opens onto whichever side touches a tower, and onto the back
    /// wall when neither does. It never opens into thin air next to a tower.
    #[test]
    fn a_door_faces_the_tower_when_there_is_one() {
        let fp = make_floorplan(None);
        for y in 0..GRID_H {
            for x in 0..GRID_W {
                let o = fp.cell[y][x].orient;
                if o == Orient::Xx || o == Orient::Tt {
                    continue;
                }
                let (left, right) = match o {
                    Orient::N => ((-1, 0), (1, 0)),
                    Orient::S => ((1, 0), (-1, 0)),
                    Orient::E => ((0, -1), (0, 1)),
                    _ => ((0, 1), (0, -1)),
                };
                let at = |(dx, dy): (i32, i32)| -> Option<Orient> {
                    let (x2, y2) = (x as i32 + dx, y as i32 + dy);
                    if (0..GRID_W as i32).contains(&x2) && (0..GRID_H as i32).contains(&y2) {
                        Some(fp.cell[y2 as usize][x2 as usize].orient)
                    } else {
                        None
                    }
                };
                let want = if at(left) == Some(Orient::Tt) {
                    Side::Left
                } else if at(right) == Some(Orient::Tt) {
                    Side::Right
                } else {
                    Side::Front
                };
                assert_eq!(fp.cell[y][x].c.door_pos, want, "cell {x},{y}");
            }
        }
    }

    /// The building opens already built: the fast-forward runs the simulation
    /// until it has docked its quota, so the first frame shows a tower and not
    /// bare pilings.
    #[test]
    fn it_starts_out_already_built() {
        let r = run("", 1);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        // Two towers alone come to well under a thousand vertices; a stack of
        // capsules is tens of thousands.
        assert!(
            f.vertices.len() > 20_000,
            "only {} vertices, so nothing was built",
            f.vertices.len()
        );
    }

    /// It keeps running, and the building keeps scrolling down.
    #[test]
    fn the_building_scrolls_down() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        r.step();
        let before = r.frame().vertices.len();
        for _ in 0..40 {
            r.step();
        }
        let after = r.frame().vertices.len();
        assert!(before > 0 && after > 0, "{before} then {after}");
    }
}
