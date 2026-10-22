//! Port of `hacks/glx/papercube.c`.
//!
//! ```text
//! papercube, Copyright © 2023 Ireneusz Szpilewski <irkostka@irkostka.pl>
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
//! A paper net folding itself into a cube, over and over.
//!
//! The net is a picture, not a data structure: a string of characters laid out
//! on a grid, where `o` is a square of paper, `^` a square with a tab on it,
//! and `+` between two of them is a crease. Everything the program knows about
//! the shape comes from reading that string.
//!
//! Drawing it is a walk rather than a loop. One square is the floor; it is
//! drawn, and then for each crease leading out of it the matrix is rotated
//! about that crease by however far it has been folded so far and the
//! neighbour is drawn inside that rotation, recursively. So a square four
//! creases along carries the four rotations before it, and the whole net hinges
//! correctly without anything ever computing a position.
//!
//! The folding is a script. Seventeen moves, each a start time, an end time and
//! a pair of angles, run in order with a pause between them, preceded by the
//! light coming up and followed by a spin while it fades. Two of the moves
//! belong to the last tab and overlap: it swings from thirty degrees and from a
//! hundred and twenty at the same time, which is what tucks it inside.
//!
//! The paper's texture is drawn at run time, a plain square of colour with a
//! black border and, on alternate rounds, a grid ruled across it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Shape, TexEnv};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

const MAP_ROWS: usize = 6;
const MAP_COLUMNS: usize = 5;
const BOTTOM_FIELD_ROW: i32 = 1;
const BOTTOM_FIELD_COLUMN: i32 = 1;
const PRESERVED_HEIGHT_TO_WIDTH: f64 = 1.0;
const SUN_DURATION: f64 = 2.0;
const FOLD_DURATION: f64 = 1.0;
const PAUSE_DURATION: f64 = 1.0;
const SPIN_DURATION: f64 = 3.0;
const SPIN_RPS: f64 = 1.0;

const PICTURE_SQUARE_COUNT: usize = 8;
const PICTURE_SQUARE_SIZE: usize = 16;
const PICTURE_LINE_WIDTH: usize = 2;
const PICTURE_BORDER_WIDTH: usize = 2;

/// The net, as upstream draws it: five columns of squares and the creases
/// between them, bottom row last.
const MAP: &str = concat!(
    "    ^    ",
    "    +    ",
    "    o    ",
    "    +    ",
    "    o    ",
    "    +    ",
    "o o o o  ",
    "+ + + +  ",
    "o+o+o+o+o",
    "+ +   +  ",
    "o o+o o  ",
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    LeftDown,
    RightUp,
}

impl Direction {
    fn opposite(self) -> Self {
        match self {
            Direction::LeftDown => Direction::RightUp,
            Direction::RightUp => Direction::LeftDown,
        }
    }

    /// Upstream's `edge.direction` doubles as a zero-or-one offset.
    fn offset(self) -> i32 {
        match self {
            Direction::LeftDown => 0,
            Direction::RightUp => 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Edge {
    orientation: Orientation,
    direction: Direction,
}

#[derive(Clone, Copy)]
struct Field {
    row: i32,
    column: i32,
    arrow: bool,
}

#[derive(Clone, Copy, Default)]
struct Angle {
    angle: f64,
    /// Set while the last tab is tucking itself in, which shortens the square
    /// rather than turning it.
    inserting: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MoveStage {
    BeforeStart,
    Starting,
    DuringMove,
    Stopping,
    AfterStop,
}

/// One scripted change: a value easing from one number to another between two
/// times.
#[derive(Clone, Copy)]
struct Move {
    stage: MoveStage,
    start: f64,
    stop: f64,
    start_value: f64,
    stop_value: f64,
}

impl Move {
    fn new(start: f64, stop: f64, start_value: f64, stop_value: f64) -> Self {
        Move {
            stage: MoveStage::BeforeStart,
            start,
            stop,
            start_value,
            stop_value,
        }
    }

    /// `get_move_value`. Reports what the value is now and how far through the
    /// move that is, and latches once it has finished.
    fn value(&mut self, time: f64) -> (MoveStage, f64) {
        if self.stage == MoveStage::AfterStop {
            return (MoveStage::AfterStop, self.stop_value);
        }
        if self.stop <= time {
            self.stage = MoveStage::AfterStop;
            return (MoveStage::Stopping, self.stop_value);
        }
        if self.start <= time {
            let v = self.start_value
                + (self.stop_value - self.start_value) * (time - self.start)
                    / (self.stop - self.start);
            if self.stage == MoveStage::BeforeStart {
                self.stage = MoveStage::DuringMove;
                return (MoveStage::Starting, v);
            }
            return (MoveStage::DuringMove, v);
        }
        (MoveStage::BeforeStart, self.start_value)
    }
}

#[derive(Clone, Copy)]
struct FieldMove {
    mv: Move,
    row: i32,
    column: i32,
    inserting: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Sunrise,
    Fold,
    SpinAndSunset,
}

struct PaperCube {
    angles: [[Angle; MAP_COLUMNS]; MAP_ROWS],
    stage: Stage,
    sunrise: Move,
    moves: [FieldMove; 17],
    spin: Move,
    sunset: Move,
    /// Seconds since the fold began.
    time: f64,
    started_at: f64,

    eye_rotation: f64,
    spin_rotation: f64,
    spin_sign: f64,
    /// What the sun move sets: the whole net is drawn in this grey.
    brightness: f64,

    rot: Rotator,
    trackball: Trackball,
    show_grid: bool,
    texture: u32,
    fg: [u8; 4],
    bg: [u8; 4],
    /// The extra scale a tall window needs, worked out by `reshape`.
    scale: f64,
    speed: f64,
}

/// `get_field_from_map`.
fn field_from_map(row: i32, column: i32) -> u8 {
    let i = 2 * (MAP_ROWS as i32 - row - 1) * (MAP_COLUMNS as i32 * 2 - 1) + column * 2;
    MAP.as_bytes()[i as usize]
}

/// `get_edge_from_map`. A space means there is no crease, which is also what
/// the edges of the grid report.
fn edge_from_map(row: i32, column: i32, e: Edge) -> u8 {
    let outside = match (e.orientation, e.direction) {
        (Orientation::Horizontal, Direction::LeftDown) => column == 0,
        (Orientation::Horizontal, Direction::RightUp) => column == MAP_COLUMNS as i32 - 1,
        (Orientation::Vertical, Direction::LeftDown) => row == 0,
        (Orientation::Vertical, Direction::RightUp) => row == MAP_ROWS as i32 - 1,
    };
    if outside {
        return b' ';
    }
    let mut offset = match e.orientation {
        Orientation::Horizontal => 1,
        Orientation::Vertical => -(MAP_COLUMNS as i32 * 2 - 1),
    };
    if e.direction == Direction::LeftDown {
        offset = -offset;
    }
    let i = 2 * (MAP_ROWS as i32 - row - 1) * (MAP_COLUMNS as i32 * 2 - 1) + column * 2 + offset;
    MAP.as_bytes()[i as usize]
}

fn neighbour(f: &Field, e: Edge) -> Field {
    let mut n = *f;
    match (e.orientation, e.direction) {
        (Orientation::Horizontal, Direction::LeftDown) => n.column -= 1,
        (Orientation::Horizontal, Direction::RightUp) => n.column += 1,
        (Orientation::Vertical, Direction::LeftDown) => n.row -= 1,
        (Orientation::Vertical, Direction::RightUp) => n.row += 1,
    }
    n
}

impl PaperCube {
    /// `paint_field`. One square, and the tab on it if it has one and has
    /// grown far enough for the tab to show.
    fn paint_field(g: &mut Gl, field: &Field, angle: Option<&Angle>) {
        let arrow_height = 5.0 / 8.0;
        let mut arrow_width = 1.0 / 8.0;

        // A square being tucked in gets shorter rather than turning.
        let height = match angle {
            Some(a) if a.inserting => {
                2.0 * (std::f64::consts::PI - std::f64::consts::PI * a.angle / 180.0).cos()
            }
            _ => 1.0,
        };

        let (draw_arrow, rect_height) = if !field.arrow {
            (false, height)
        } else if height > arrow_height {
            (true, arrow_height)
        } else {
            (false, height)
        };

        let (r, c) = (field.row as f64, field.column as f64);
        g.glx.begin(Shape::Triangles);
        let vert = |g: &mut Gl, u: f64, v: f64, x: f64, z: f64| {
            g.glx.tex_coord2f(u as f32, v as f32);
            g.glx.vertex3f(x as f32, 0.0, z as f32);
        };

        vert(g, 0.0, 0.0, c, -r);
        vert(g, 0.0, rect_height, c, -(r + rect_height));
        vert(g, 1.0, 0.0, c + 1.0, -r);

        vert(g, 0.0, rect_height, c, -(r + rect_height));
        vert(g, 1.0, 0.0, c + 1.0, -r);
        vert(g, 1.0, rect_height, c + 1.0, -(r + rect_height));

        if draw_arrow {
            arrow_width *= (height - arrow_height) / (1.0 - arrow_height);
            vert(g, 0.0, arrow_height, c, -(r + arrow_height));
            vert(g, arrow_width, height, c + arrow_width, -(r + height));
            vert(g, 1.0, arrow_height, c + 1.0, -(r + arrow_height));

            vert(g, 1.0, arrow_height, c + 1.0, -(r + arrow_height));
            vert(
                g,
                1.0 - arrow_width,
                height,
                c + 1.0 - arrow_width,
                -(r + height),
            );
            vert(g, arrow_width, height, c + arrow_width, -(r + height));
        }
        g.glx.end();
    }

    /// `paint_field_and_neighbours`. The walk: draw this square, then for each
    /// crease out of it turn the matrix about that crease and recurse.
    fn paint_from(&self, g: &mut Gl, field: &mut Field, entry: Option<Edge>) {
        field.arrow = field_from_map(field.row, field.column) == b'^';

        let a = self.angles[field.row as usize][field.column as usize];
        Self::paint_field(g, field, if entry.is_some() { Some(&a) } else { None });

        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            for direction in [Direction::LeftDown, Direction::RightUp] {
                let e = Edge {
                    orientation,
                    direction,
                };
                if entry == Some(e) {
                    continue;
                }
                if edge_from_map(field.row, field.column, e) != b'+' {
                    continue;
                }

                g.glx.push_matrix();
                let mut n = neighbour(field, e);
                let back = Edge {
                    orientation,
                    direction: direction.opposite(),
                };
                let angle = self.angles[n.row as usize][n.column as usize].angle as f32;

                match orientation {
                    Orientation::Horizontal => {
                        let axis = (n.column + back.direction.offset()) as f32;
                        g.glx.translate(axis, 0.0, 0.0);
                        let sign = if axis <= BOTTOM_FIELD_COLUMN as f32 {
                            -1.0
                        } else {
                            1.0
                        };
                        g.glx.rotate(sign * angle, 0.0, 0.0, 1.0);
                        g.glx.translate(-axis, 0.0, 0.0);
                    }
                    Orientation::Vertical => {
                        let axis = (n.row + back.direction.offset()) as f32;
                        g.glx.translate(0.0, 0.0, -axis);
                        let sign = if axis <= BOTTOM_FIELD_ROW as f32 {
                            -1.0
                        } else {
                            1.0
                        };
                        g.glx.rotate(sign * angle, 1.0, 0.0, 0.0);
                        g.glx.translate(0.0, 0.0, axis);
                    }
                }

                self.paint_from(g, &mut n, Some(back));
                g.glx.pop_matrix();
            }
        }
    }

    /// `initialize_moves`. The whole script, in order.
    fn initialize_moves(&mut self) {
        const FIELDS: [(i32, i32); 15] = [
            (0, 0),
            (2, 0),
            (1, 0),
            (0, 2),
            (0, 1),
            (2, 1),
            (1, 2),
            (1, 3),
            (2, 3),
            (0, 3),
            (1, 4),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 2),
        ];

        let angle = 90.0;
        let fold = FOLD_DURATION / self.speed;
        let pause = PAUSE_DURATION / self.speed;
        let sun_d = SUN_DURATION / self.speed;
        let spin = SPIN_DURATION / self.speed;
        let brightness = 1.0;
        let mut time = 0.0;

        self.sunrise = Move::new(time, sun_d, 0.0, brightness);
        time = self.sunrise.stop + pause;

        for (i, (row, column)) in FIELDS.into_iter().enumerate() {
            // The last two squares of the strip fold by a third and by four
            // thirds of a right angle, which is what closes the box.
            let multi = match i {
                13 => 1.0 / 3.0,
                14 => 4.0 / 3.0,
                _ => 1.0,
            };
            self.moves[i] = FieldMove {
                mv: Move::new(time, time + fold * multi, 0.0, angle * multi),
                row,
                column,
                inserting: false,
            };
            time = self.moves[i].mv.stop + pause;
        }

        // The tab swings from two places at once, which tucks it inside.
        self.moves[15] = FieldMove {
            mv: Move::new(time, time + fold * 2.0 / 3.0, 30.0, angle),
            row: FIELDS[13].0,
            column: FIELDS[13].1,
            inserting: false,
        };
        self.moves[16] = FieldMove {
            mv: Move::new(time, time + fold * 2.0 / 3.0, 120.0, angle),
            row: FIELDS[14].0,
            column: FIELDS[14].1,
            inserting: true,
        };
        time = self.moves[16].mv.stop + pause;

        self.spin = Move::new(
            time,
            time + spin + sun_d,
            0.0,
            (spin + sun_d) * SPIN_RPS * 360.0,
        );
        time += spin;
        self.sunset = Move::new(time, time + sun_d, brightness, 0.0);

        self.stage = Stage::Sunrise;
    }

    /// `move_papercube`. Advance whichever part of the script is running.
    fn advance(&mut self) -> MoveStage {
        let t = self.time;
        let mut result = MoveStage::BeforeStart;

        match self.stage {
            Stage::Sunrise => {
                let (stage, v) = self.sunrise.value(t);
                if stage != MoveStage::BeforeStart {
                    self.brightness = v;
                }
                result = match stage {
                    MoveStage::BeforeStart => MoveStage::BeforeStart,
                    MoveStage::Starting => MoveStage::Starting,
                    _ => MoveStage::DuringMove,
                };
                if stage == MoveStage::Stopping {
                    self.stage = Stage::Fold;
                }
            }
            Stage::Fold => {
                let mut last = MoveStage::BeforeStart;
                for i in 0..self.moves.len() {
                    let (stage, v) = self.moves[i].mv.value(t);
                    last = stage;
                    match stage {
                        MoveStage::BeforeStart | MoveStage::AfterStop => {}
                        _ => {
                            let (r, c) =
                                (self.moves[i].row as usize, self.moves[i].column as usize);
                            if stage == MoveStage::Starting && self.moves[i].inserting {
                                self.angles[r][c].inserting = true;
                            }
                            self.angles[r][c].angle = v;
                        }
                    }
                }
                result = if last == MoveStage::Stopping {
                    MoveStage::Stopping
                } else {
                    MoveStage::DuringMove
                };
                if last == MoveStage::Stopping {
                    self.stage = Stage::SpinAndSunset;
                }
            }
            Stage::SpinAndSunset => {
                let (stage, v) = self.spin.value(t);
                if stage != MoveStage::BeforeStart {
                    self.spin_rotation = v;
                }
                let (stage, v) = self.sunset.value(t);
                if stage != MoveStage::BeforeStart {
                    self.brightness = v;
                }
                if stage == MoveStage::Stopping {
                    result = MoveStage::Stopping;
                }
            }
        }
        result
    }

    /// `initialize_texture`. The paper: a square of colour with a black
    /// border, ruled with a grid on alternate rounds.
    fn make_texture(&mut self, g: &mut Gl) {
        let w = PICTURE_SQUARE_COUNT * PICTURE_SQUARE_SIZE;
        let mut data = vec![0u8; w * w * 4];
        let rect = |data: &mut [u8], x: usize, y: usize, rw: usize, rh: usize, c: [u8; 4]| {
            for j in y..(y + rh).min(w) {
                for i in x..(x + rw).min(w) {
                    data[(j * w + i) * 4..(j * w + i) * 4 + 4].copy_from_slice(&c);
                }
            }
        };

        rect(&mut data, 0, 0, w, w, self.fg);
        if self.show_grid {
            let half = PICTURE_LINE_WIDTH / 2;
            for k in 1..PICTURE_SQUARE_COUNT {
                rect(
                    &mut data,
                    k * PICTURE_SQUARE_SIZE - half,
                    0,
                    PICTURE_LINE_WIDTH,
                    w,
                    self.bg,
                );
                rect(
                    &mut data,
                    0,
                    k * PICTURE_SQUARE_SIZE - half,
                    w,
                    PICTURE_LINE_WIDTH,
                    self.bg,
                );
            }
        }
        let black = [0, 0, 0, 255];
        rect(&mut data, 0, 0, w, PICTURE_BORDER_WIDTH, black);
        rect(&mut data, 0, 0, PICTURE_BORDER_WIDTH, w, black);
        rect(
            &mut data,
            0,
            w - PICTURE_BORDER_WIDTH,
            w,
            PICTURE_BORDER_WIDTH,
            black,
        );
        rect(
            &mut data,
            w - PICTURE_BORDER_WIDTH,
            0,
            PICTURE_BORDER_WIDTH,
            w,
            black,
        );

        if self.texture == 0 {
            self.texture = g.glx.gen_texture();
        }
        g.glx.bind_texture(self.texture);
        g.glx.tex_image_2d(w as i32, w as i32, data);
        g.glx.tex_env(TexEnv::Modulate);
    }

    /// `initialize_papercube`. A new round: a new colour, the grid toggled,
    /// and the script rewound.
    fn restart(&mut self, g: &mut Gl, first_time: bool) {
        if first_time {
            self.eye_rotation = 45.0;
            self.show_grid = false;
        } else {
            self.eye_rotation = (random() % 360) as f64;
            self.show_grid = !self.show_grid;
        }
        self.spin_sign = if random().is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        self.spin_rotation = 0.0;

        let chan = || (255.0 * (0.5 + frand(0.5))) as u8;
        self.fg = [chan(), chan(), chan(), 255];
        self.bg = [
            (0.7 * (255 - self.fg[0]) as f64) as u8,
            (0.7 * (255 - self.fg[1]) as f64) as u8,
            (0.7 * (255 - self.fg[2]) as f64) as u8,
            255,
        ];

        self.angles = [[Angle::default(); MAP_COLUMNS]; MAP_ROWS];
        self.initialize_moves();
        self.started_at = g.time;
        self.time = 0.0;
        self.make_texture(g);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.string("spin").to_string();
    let spinx = spin.contains(['x', 'X']);
    let spiny = spin.contains(['y', 'Y']);
    let spinz = spin.contains(['z', 'Z']);
    let do_wander = g.res.bool("wander");
    let speed = g.res.float("speed").max(0.001);

    let mut this = PaperCube {
        angles: [[Angle::default(); MAP_COLUMNS]; MAP_ROWS],
        stage: Stage::Sunrise,
        sunrise: Move::new(0.0, 1.0, 0.0, 1.0),
        moves: [FieldMove {
            mv: Move::new(0.0, 1.0, 0.0, 0.0),
            row: 0,
            column: 0,
            inserting: false,
        }; 17],
        spin: Move::new(0.0, 1.0, 0.0, 0.0),
        sunset: Move::new(0.0, 1.0, 1.0, 0.0),
        time: 0.0,
        started_at: 0.0,
        eye_rotation: 45.0,
        spin_rotation: 0.0,
        spin_sign: 1.0,
        brightness: 0.0,
        rot: Rotator::new(
            if spinx { 0.5 * speed } else { 0.0 },
            if spiny { 0.5 * speed } else { 0.0 },
            if spinz { 0.5 * speed } else { 0.0 },
            0.3,
            if do_wander { 0.01 * speed } else { 0.0 },
            false,
        ),
        trackball: Trackball::new(),
        show_grid: false,
        texture: 0,
        fg: [255; 4],
        bg: [0, 0, 0, 255],
        scale: 1.0,
        speed,
    };

    this.restart(g, true);
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for PaperCube {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();

        self.scale = if height as f64 / width as f64 > PRESERVED_HEIGHT_TO_WIDTH {
            width as f64 * PRESERVED_HEIGHT_TO_WIDTH / height as f64
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.texturing(true);
        g.glx.lighting(false);
        g.glx.bind_texture(self.texture);

        self.time = g.time - self.started_at;
        if self.advance() == MoveStage::Stopping {
            self.restart(g, false);
        }

        // The sun move sets the shade everything is drawn in.
        let b = self.brightness as f32;
        g.glx.color3f(b, b, b);

        // The camera, which `reshape` sets up in the modelview and the net is
        // then drawn under.
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 10.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = self.scale as f32;
        g.glx.scale(s, s, s);
        g.glx.translate(
            -BOTTOM_FIELD_COLUMN as f32 - 0.5,
            -0.5,
            BOTTOM_FIELD_ROW as f32 + 0.5,
        );

        let cx = BOTTOM_FIELD_COLUMN as f32 + 0.5;
        let cy = 0.5;
        let cz = -BOTTOM_FIELD_ROW as f32 - 0.5;

        g.glx.push_matrix();
        g.glx.translate(cx, cy, cz);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 3.0,
            (y as f32 - 0.5) * 3.0,
            (z as f32 - 0.5) * 3.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.rotate(
            (self.eye_rotation + self.spin_sign * self.spin_rotation) as f32,
            0.0,
            1.0,
            0.0,
        );
        g.glx.translate(-cx, -cy, -cz);

        let mut start = Field {
            row: BOTTOM_FIELD_ROW,
            column: BOTTOM_FIELD_COLUMN,
            arrow: false,
        };
        self.paint_from(g, &mut start, None);
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         Y",
    "*wander:       True",
    "*speed:        1.0",
];

const SPINS: &[SelectItem] = &[
    SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    SelectItem {
        value: "0",
        label: "Don't rotate",
    },
    SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    SelectItem {
        value: "XY",
        label: "Rotate around X and Y axes",
    },
    SelectItem {
        value: "XZ",
        label: "Rotate around X and Z axes",
    },
    SelectItem {
        value: "YZ",
        label: "Rotate around Y and Z axes",
    },
    SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.02, 5.0, 0.02, 2, "1.0"),
    Opt::select("spin", "Rotation", SPINS, "Y"),
    Opt::boolean("wander", "Wander", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "papercube",
    label: "Paper Cube",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ireneusz Szpilewski",
        year: "2023",
        video: Some("https://www.youtube.com/watch?v=gAIZs0Ar2Ig"),
        blurb: "A paper net folding itself into a cube.",
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
    fn the_net_is_fifteen_squares_and_a_tab() {
        // Everything the program knows about the shape is in the map string,
        // so it is worth checking what it says.
        let mut squares = 0;
        let mut tabs = 0;
        for row in 0..MAP_ROWS as i32 {
            for column in 0..MAP_COLUMNS as i32 {
                match field_from_map(row, column) {
                    b'o' => squares += 1,
                    b'^' => tabs += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(squares, 15, "got {squares} squares");
        assert_eq!(tabs, 1, "got {tabs} tabs");
    }

    #[test]
    fn a_crease_reads_the_same_from_either_side() {
        // The walk relies on this: it enters a square through the crease it
        // came in by, and has to recognise the same crease from the far side.
        for row in 0..MAP_ROWS as i32 {
            for column in 0..MAP_COLUMNS as i32 {
                for orientation in [Orientation::Horizontal, Orientation::Vertical] {
                    for direction in [Direction::LeftDown, Direction::RightUp] {
                        let e = Edge {
                            orientation,
                            direction,
                        };
                        if edge_from_map(row, column, e) != b'+' {
                            continue;
                        }
                        let n = neighbour(
                            &Field {
                                row,
                                column,
                                arrow: false,
                            },
                            e,
                        );
                        let back = Edge {
                            orientation,
                            direction: direction.opposite(),
                        };
                        assert_eq!(
                            edge_from_map(n.row, n.column, back),
                            b'+',
                            "the crease at {row},{column} is one-way"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_square_is_reachable_from_the_floor() {
        // If the net were not connected the walk would silently draw only part
        // of it, and the missing squares would look like holes.
        fn walk(row: i32, column: i32, entry: Option<Edge>, seen: &mut Vec<(i32, i32)>) {
            seen.push((row, column));
            for orientation in [Orientation::Horizontal, Orientation::Vertical] {
                for direction in [Direction::LeftDown, Direction::RightUp] {
                    let e = Edge {
                        orientation,
                        direction,
                    };
                    if entry == Some(e) || edge_from_map(row, column, e) != b'+' {
                        continue;
                    }
                    let n = neighbour(
                        &Field {
                            row,
                            column,
                            arrow: false,
                        },
                        e,
                    );
                    walk(
                        n.row,
                        n.column,
                        Some(Edge {
                            orientation,
                            direction: direction.opposite(),
                        }),
                        seen,
                    );
                }
            }
        }
        let mut seen = Vec::new();
        walk(BOTTOM_FIELD_ROW, BOTTOM_FIELD_COLUMN, None, &mut seen);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 16, "reached {} of the sixteen", seen.len());
    }

    #[test]
    fn the_net_folds_up_and_starts_again() {
        let mut r = start(StartArgs::new(640, 480, "speed=5", 20260811));
        // The fold lives in the matrices, not the vertices: every square is
        // drawn flat at y = 0 and hinged into place by the walk. So the
        // measure is how far a square's orientation has departed from the
        // floor square's, which is zero for a flat net.
        let bend = |r: &Runner3d| {
            let f = r.frame();
            let Some(first) = f.batches.first() else {
                return 0.0;
            };
            f.batches
                .iter()
                .map(|b| {
                    // The upper three by three of the modelview is the
                    // rotation; the translation in the last row is just where
                    // the square sits on the sheet.
                    (0..3)
                        .flat_map(|c| (0..3).map(move |k| c * 4 + k))
                        .map(|i| (b.modelview.0[i] - first.modelview.0[i]).abs())
                        .fold(0.0f32, f32::max)
                })
                .fold(0.0f32, f32::max)
        };

        let mut folded = 0.0f32;
        let mut restarted = false;
        let mut was_folded = false;
        for _ in 0..3000 {
            r.step();
            let b = bend(&r);
            folded = folded.max(b);
            if b > 0.5 {
                was_folded = true;
            } else if was_folded {
                restarted = true;
            }
        }
        assert!(folded > 0.5, "the net never folded, {folded}");
        assert!(restarted, "it never started over");
    }

    #[test]
    fn the_light_comes_up_and_goes_down_again() {
        let mut r = start(StartArgs::new(640, 480, "speed=5", 20260811));
        let (mut lo, mut hi) = (1.0f32, 0.0f32);
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            if let Some(v) = f.vertices.first() {
                lo = lo.min(v.color[0]);
                hi = hi.max(v.color[0]);
            }
        }
        assert!(lo < 0.1, "it never started dark, {lo}");
        assert!(hi > 0.9, "it never came up to full, {hi}");
    }
}
