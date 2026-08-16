//! Port of `hacks/glx/hextrail.c`.
//!
//! ```text
//! hextrail, Copyright (c) 2022 Jamie Zawinski <jwz@jwz.org>
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
//! A network of colourful lines grows over a hexagonal grid: from one cell in
//! the middle, out along the six directions, branching where it can, until it
//! has nowhere left to go. Then the whole thing fades out and it starts again
//! somewhere else.
//!
//! The growth is a relay rather than a search. Every cell has six arms, and an
//! arm is in one of five states. When a cell sends an arm out towards a
//! neighbour, that neighbour's opposite arm is set to `WAIT` at the same
//! moment, so the space is claimed before it is drawn into. The outgoing arm
//! grows from the middle of its cell to the shared edge, and on arriving hands
//! the baton over: it goes `DONE` and the waiting arm goes `IN` at the same
//! speed, growing from the edge to the middle of the next cell. Only when an
//! arm reaches a cell's middle does that cell look for exits of its own. So the
//! front advances one cell at a time and never doubles back, because a cell
//! with any arm at all is not empty and cannot be claimed again.
//!
//! `live_count` is how many arms are in flight, and when it reaches zero the
//! network is finished. That is the only thing that decides when to fade: no
//! timer and no size limit, just growth continuing until it is boxed in.
//!
//! Colour is inherited: a newly claimed cell takes the colour of the cell that
//! claimed it, except one time in five when it steps one along the colourmap.
//! So the network is banded rather than random, and the bands mark how far the
//! front had travelled when it passed. The line drawn between two cells is a
//! gradient from one's colour to the average of the two, which is what makes
//! the bands blend rather than switch.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random, random_below, screenhack_event_helper,
};

/// What an arm, a border or the whole picture is doing.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Empty,
    In,
    Wait,
    Out,
    Done,
}

#[derive(Clone, Copy, Default)]
struct Arm {
    state: State,
    ratio: f32,
    speed: f32,
}

#[derive(Clone, Copy)]
struct Hexagon {
    pos: [f32; 3],
    /// Index into the grid, or `None` at the edges.
    neighbors: [Option<usize>; 6],
    arms: [Arm; 6],
    ccolor: usize,
    border_state: State,
    border_ratio: f32,
}

impl Default for Hexagon {
    fn default() -> Self {
        Hexagon {
            pos: [0.0; 3],
            neighbors: [None; 6],
            arms: [Arm::default(); 6],
            ccolor: 0,
            border_state: State::Empty,
            border_ratio: 0.0,
        }
    }
}

/// Whether the picture is starting, growing or fading out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    First,
    Draw,
    Fade,
}

struct HexTrail {
    rot: Rotator,
    trackball: Trackball,

    grid_w: i32,
    grid_h: i32,
    hexagons: Vec<Hexagon>,
    live_count: i32,
    state: Phase,
    fade_ratio: f32,

    colors: Vec<XColor>,

    count: i32,
    speed: f32,
    thickness: f32,
    wireframe: bool,
}

/// `sqrt(3)/2`, which is how far the flat side of a unit hexagon is from its
/// middle, and so the whole of the grid's geometry.
const H: f32 = 0.866_025_4;

const CORNERS: [[f32; 3]; 6] = [
    [0.0, -1.0, 0.0], /*      0      */
    [H, -0.5, 0.0],   /*  5       1  */
    [H, 0.5, 0.0],    /*             */
    [0.0, 1.0, 0.0],  /*  4       2  */
    [-H, 0.5, 0.0],   /*      3      */
    [-H, -0.5, 0.0],
];

impl HexTrail {
    fn make_plane(&mut self) {
        self.grid_w = self.count * 2;
        self.grid_h = self.grid_w;
        let (gw, gh) = (self.grid_w, self.grid_h);
        self.hexagons = vec![Hexagon::default(); (gw * gh) as usize];
        self.colors = make_smooth_colormap(8);
        let n = self.colors.len();

        let size = 2.0 / gw as f32;
        let w = size;
        let h = size * 3.0f32.sqrt() / 2.0;

        for y in 0..gh {
            for x in 0..gw {
                let h0 = &mut self.hexagons[(y * gw + x) as usize];
                h0.pos[0] = (x - gw / 2) as f32 * w;
                h0.pos[1] = (y - gh / 2) as f32 * h;
                h0.border_state = State::Empty;
                h0.border_ratio = 0.0;

                // Every other row is offset by half a cell, which is what makes
                // it a hexagonal grid rather than a square one.
                if y & 1 != 0 {
                    h0.pos[0] += w / 2.0;
                }

                h0.ccolor = random() as usize % n;
            }
        }

        // The six directions, as (even-row dx, odd-row dx, dy). The two dx
        // columns are the same half-cell offset again, seen from the cell.
        const DIRS: [(i32, i32, i32); 6] = [
            (0, 1, -1),
            (1, 1, 0),
            (0, 1, 1),
            (-1, 0, 1),
            (-1, -1, 0),
            (-1, 0, -1),
        ];
        for y in 0..gh {
            for x in 0..gw {
                let mut neighbors = [None; 6];
                for (i, (xe, xo, dy)) in DIRS.iter().enumerate() {
                    let x1 = x + if y & 1 != 0 { *xo } else { *xe };
                    let y1 = y + dy;
                    if x1 >= 0 && x1 < gw && y1 >= 0 && y1 < gh {
                        neighbors[i] = Some((y1 * gw + x1) as usize);
                    }
                }
                self.hexagons[(y * gw + x) as usize].neighbors = neighbors;
            }
        }
    }

    fn empty_hexagon_p(&self, i: usize) -> bool {
        self.hexagons[i]
            .arms
            .iter()
            .all(|a| a.state == State::Empty)
    }

    /// Send arms out of this cell into as many free neighbours as it can find,
    /// up to a random target.
    fn add_arms(&mut self, h0: usize, out_p: bool) -> i32 {
        let mut added = 0;
        let mut target = 1 + random_below(4); /* Aim for 1-5 arms */

        let mut idx = [0usize; 6]; /* Traverse in random order */
        for (i, v) in idx.iter_mut().enumerate() {
            *v = i;
        }
        for i in 0..6 {
            let j = random_below(6) as usize;
            idx.swap(i, j);
        }

        if out_p {
            target -= 1;
        }

        for j in idx {
            let Some(h1) = self.hexagons[h0].neighbors[j] else {
                continue; /* No neighboring cell */
            };
            if !self.empty_hexagon_p(h1) {
                continue; /* Occupado */
            }
            if self.hexagons[h0].arms[j].state != State::Empty {
                continue; /* Arm already exists */
            }

            let speed = 0.05 * self.speed * (0.8 + frand(1.0) as f32);
            {
                let a0 = &mut self.hexagons[h0].arms[j];
                a0.state = if out_p { State::Out } else { State::In };
                a0.ratio = 0.0;
                a0.speed = speed;
            }
            {
                let a1 = &mut self.hexagons[h1].arms[(j + 3) % 6]; /* Opposite arm */
                a1.state = State::Wait;
                a1.ratio = 0.0;
                a1.speed = speed;
            }

            if self.hexagons[h1].border_state == State::Empty {
                self.hexagons[h1].border_state = State::In;

                /* Mostly keep the same color */
                let c0 = self.hexagons[h0].ccolor;
                let n = self.colors.len();
                self.hexagons[h1].ccolor = if random().is_multiple_of(5) {
                    (c0 + 1) % n
                } else {
                    c0
                };
            }

            self.live_count += 1;
            added += 1;
            if added >= target {
                break;
            }
        }
        added
    }

    fn tick_hexagons(&mut self) {
        /* Enlarge any still-growing arms. */
        for i in 0..self.hexagons.len() {
            for j in 0..6 {
                match self.hexagons[i].arms[j].state {
                    State::Out => {
                        self.hexagons[i].arms[j].ratio += self.hexagons[i].arms[j].speed;
                        if self.hexagons[i].arms[j].ratio > 1.0 {
                            /* Just finished growing from center to edge.
                            Pass the baton to this waiting neighbor. */
                            let speed = self.hexagons[i].arms[j].speed;
                            self.hexagons[i].arms[j].state = State::Done;
                            self.hexagons[i].arms[j].ratio = 1.0;
                            if let Some(h1) = self.hexagons[i].neighbors[j] {
                                let a1 = &mut self.hexagons[h1].arms[(j + 3) % 6];
                                a1.state = State::In;
                                a1.ratio = 0.0;
                                a1.speed = speed;
                            }
                            /* live_count unchanged */
                        }
                    }
                    State::In => {
                        self.hexagons[i].arms[j].ratio += self.hexagons[i].arms[j].speed;
                        if self.hexagons[i].arms[j].ratio > 1.0 {
                            /* Just finished growing from edge to center.
                            Look for any available exits. */
                            self.hexagons[i].arms[j].state = State::Done;
                            self.hexagons[i].arms[j].ratio = 1.0;
                            self.live_count -= 1;
                            self.add_arms(i, true);
                        }
                    }
                    State::Empty | State::Wait | State::Done => {}
                }
            }

            let step = 0.05 * self.speed;
            match self.hexagons[i].border_state {
                State::In => {
                    self.hexagons[i].border_ratio += step;
                    if self.hexagons[i].border_ratio >= 1.0 {
                        self.hexagons[i].border_ratio = 1.0;
                        self.hexagons[i].border_state = State::Wait;
                    }
                }
                State::Out => {
                    self.hexagons[i].border_ratio -= step;
                    if self.hexagons[i].border_ratio <= 0.0 {
                        self.hexagons[i].border_ratio = 0.0;
                        self.hexagons[i].border_state = State::Empty;
                    }
                    // Upstream falls out of OUT into WAIT rather than
                    // breaking, so a border on its way out rolls the same die
                    // as a settled one. It changes nothing about the border,
                    // which is already going out, but it does draw a random
                    // number, and every other roll in the saver comes after it.
                    if random().is_multiple_of(50) {
                        self.hexagons[i].border_state = State::Out;
                    }
                }
                State::Wait => {
                    if random().is_multiple_of(50) {
                        self.hexagons[i].border_state = State::Out;
                    }
                }
                State::Empty | State::Done => {}
            }
        }

        /* Start a new cell growing. */
        if self.live_count <= 0 {
            for _ in 0..self.hexagons.len() / 3 {
                let (x, y) = if self.state == Phase::First {
                    self.state = Phase::Draw;
                    self.fade_ratio = 1.0;
                    (self.grid_w / 2, self.grid_h / 2)
                } else {
                    (random_below(self.grid_w), random_below(self.grid_h))
                };
                let h0 = (y * self.grid_w + x) as usize;
                if self.empty_hexagon_p(h0) && self.add_arms(h0, true) != 0 {
                    break;
                }
            }
        }

        if self.live_count <= 0 && self.state != Phase::Fade {
            self.state = Phase::Fade;
            self.fade_ratio = 1.0;

            for h in &mut self.hexagons {
                if h.border_state == State::In || h.border_state == State::Wait {
                    h.border_state = State::Out;
                }
            }
        } else if self.state == Phase::Fade {
            self.fade_ratio -= 0.01 * self.speed;
            if self.fade_ratio <= 0.0 {
                self.make_plane();
                self.state = Phase::First;
                self.fade_ratio = 1.0;
            }
        }
    }

    fn hexagon_color(&self, h: usize) -> [f32; 4] {
        let c = &self.colors[self.hexagons[h].ccolor.min(self.colors.len() - 1)];
        [
            f32::from(c.red) / 65535.0 * self.fade_ratio,
            f32::from(c.green) / 65535.0 * self.fade_ratio,
            f32::from(c.blue) / 65535.0 * self.fade_ratio,
            1.0,
        ]
    }

    fn draw_hexagons(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let length = 3.0f32.sqrt() / 3.0;
        let size = length / self.count as f32;
        let thick2 = self.thickness * self.fade_ratio;

        g.glx.front_face_cw(false);
        g.glx
            .begin(if wire { Shape::Lines } else { Shape::Triangles });
        g.glx.normal3f(0.0, 0.0, 1.0);

        // The whole picture is one `glBegin` block, so the colour has to be
        // per vertex, which it is. Upstream sets the material alongside it and
        // that does nothing: hextrail never turns lighting on.
        let set = |g: &mut Gl, c: [f32; 4]| g.glx.color4f(c[0], c[1], c[2], c[3]);

        for i in 0..self.hexagons.len() {
            let h = &self.hexagons[i];
            let total_arms = h
                .arms
                .iter()
                .filter(|a| a.state == State::Out || a.state == State::Done)
                .count();
            let color = self.hexagon_color(i);

            for (j, cj) in CORNERS.iter().enumerate() {
                let a = &h.arms[j];
                let margin = self.thickness * 0.4;
                let size1 = size * (1.0 - margin * 2.0);
                let size2 = size * (1.0 - margin * 3.0);
                let ck = &CORNERS[(j + 1) % 6];
                let at = |c: [f32; 3], s: f32| [h.pos[0] + c[0] * s, h.pos[1] + c[1] * s, h.pos[2]];

                if h.border_state != State::Empty {
                    // The cell's outline, a ring two sizes wide, brought up
                    // and taken away again by its own ratio.
                    let r = h.border_ratio;
                    set(g, [color[0] * r, color[1] * r, color[2] * r, color[3]]);

                    /* Outer edge of hexagon border */
                    let p0 = at(*cj, size1);
                    let p1 = at(*ck, size1);
                    /* Inner edge of hexagon border */
                    let p2 = at(*ck, size2);
                    let p3 = at(*cj, size2);

                    g.glx.vertex3f(p0[0], p0[1], p0[2]);
                    g.glx.vertex3f(p1[0], p1[1], p1[2]);
                    if !wire {
                        g.glx.vertex3f(p2[0], p2[1], p2[2]);
                    }
                    g.glx.vertex3f(p2[0], p2[1], p2[2]);
                    g.glx.vertex3f(p3[0], p3[1], p3[2]);
                    if !wire {
                        g.glx.vertex3f(p0[0], p0[1], p0[2]);
                    }
                }

                /* Line from center to edge, or edge to center. */
                if a.state == State::In || a.state == State::Out || a.state == State::Done {
                    let x = (cj[0] + ck[0]) / 2.0;
                    let y = (cj[1] + ck[1]) / 2.0;
                    let xoff = ck[0] - cj[0];
                    let yoff = ck[1] - cj[1];
                    let line_length = a.ratio;

                    /* Color of the outer point of the line is average color of
                    this and the neighbor. */
                    let mut ncolor = match h.neighbors[j] {
                        Some(n) => self.hexagon_color(n),
                        None => color,
                    };
                    for c in 0..4 {
                        ncolor[c] = (ncolor[c] + color[c]) / 2.0;
                    }

                    let (start, end, color1, color2) = if a.state == State::Out {
                        (0.0, size * line_length, color, ncolor)
                    } else {
                        (size, size * (1.0 - line_length), ncolor, color)
                    };

                    let along = |off: f32, t: f32| {
                        [
                            h.pos[0] + xoff * size2 * thick2 * off + x * t,
                            h.pos[1] + yoff * size2 * thick2 * off + y * t,
                            h.pos[2],
                        ]
                    };
                    /* Center */
                    let p0 = along(1.0, start);
                    let p1 = along(-1.0, start);
                    /* Edge */
                    let p2 = along(-1.0, end);
                    let p3 = along(1.0, end);

                    set(g, color2);
                    g.glx.vertex3f(p3[0], p3[1], p3[2]);
                    set(g, color1);
                    g.glx.vertex3f(p0[0], p0[1], p0[2]);
                    if !wire {
                        g.glx.vertex3f(p1[0], p1[1], p1[2]);
                    }
                    g.glx.vertex3f(p1[0], p1[1], p1[2]);
                    set(g, color2);
                    g.glx.vertex3f(p2[0], p2[1], p2[2]);
                    if !wire {
                        g.glx.vertex3f(p3[0], p3[1], p3[2]);
                    }
                }

                /* Hexagon (one triangle of) in center to hide line
                miter/bevels. */
                if total_arms != 0 {
                    let mut size3 = size * thick2 * 0.8;
                    if total_arms == 1 {
                        size3 *= 2.0;
                    }

                    let p1 = at(*cj, size3);
                    let p2 = at(*ck, size3);

                    set(g, color);
                    if !wire {
                        g.glx.vertex3f(h.pos[0], h.pos[1], h.pos[2]);
                    }
                    g.glx.vertex3f(p1[0], p1[1], p1[2]);
                    g.glx.vertex3f(p2[0], p2[1], p2[2]);
                }
            }
        }
        g.glx.end();
    }

    /// Start over, which is what any key does.
    fn reset(&mut self) {
        self.count = self.count.max(1);
        self.state = Phase::First;
        self.fade_ratio = 1.0;
        self.live_count = 0;
        self.make_plane();
    }
}

impl Hack3d for HexTrail {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        // Flat, and drawn back to front by construction, so neither the depth
        // test nor culling has anything to decide.
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.clear();

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 6.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 12.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        // Only the z rotation is used: the grid is a plane, and turning it out
        // of its own plane would only foreshorten it.
        let (_, _, z) = self.rot.rotation(!down);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(18.0, 18.0, 18.0);

        if !down {
            self.tick_hexagons();
        }
        self.draw_hexagons(g);

        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 3 {
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
        if let XEvent::KeyPress { key } = event {
            match key {
                ' ' | '\t' | '\r' | '\n' => {}
                '>' | '.' | '+' | '=' => self.count += 1,
                '<' | ',' | '-' | '_' => self.count -= 1,
                _ if screenhack_event_helper(event) => {}
                _ => return false,
            }
            self.reset();
            return true;
        }
        if screenhack_event_helper(event) {
            self.reset();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let spin_speed = 0.002;
    let wander_speed = 0.003;
    let spin_accel = 1.0;

    let mut st = HexTrail {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        trackball: Trackball::new(),
        grid_w: 0,
        grid_h: 0,
        hexagons: Vec::new(),
        live_count: 0,
        state: Phase::First,
        fade_ratio: 1.0,
        colors: Vec::new(),
        count: g.res.int("count").clamp(1, 80),
        speed: g.res.float("speed") as f32,
        thickness: g.res.float("thickness").clamp(0.05, 0.5) as f32,
        wireframe: g.res.bool("wireframe"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    /* Let's tilt the scene a little. */
    st.trackball.reset(-0.4 + frand(0.8), -0.4 + frand(0.8));

    st.make_plane();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*count:        20",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*thickness:    0.15",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 20.0, 0.1, 1, "1.0"),
    Opt::slider("count", "Hexagon size", 2.0, 80.0, 1.0, 0, "20").inverted(),
    Opt::slider("thickness", "Line thickness", 0.01, 0.5, 0.01, 2, "0.15"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hextrail",
    label: "Hex Trail",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2022",
        video: Some("https://www.youtube.com/watch?v=gXcEitEmLbw"),
        blurb: "A network of colorful lines grows upon a hexagonal substrate.",
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

    /// Neighbouring is mutual, and going out of a cell and back in along the
    /// opposite arm returns to where you started. The relay depends on it:
    /// arm `j` of one cell and arm `j+3` of its neighbour are the two ends of
    /// the same edge.
    #[test]
    fn the_grid_agrees_with_itself() {
        let mut r = start(StartArgs::new(640, 480, "count=6", 20260811));
        r.step();
        // Rebuilt here rather than reached through the runner, since the grid
        // is a pure function of the count.
        let mut st = HexTrail {
            rot: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.0, false),
            trackball: Trackball::new(),
            grid_w: 0,
            grid_h: 0,
            hexagons: Vec::new(),
            live_count: 0,
            state: Phase::First,
            fade_ratio: 1.0,
            colors: Vec::new(),
            count: 6,
            speed: 1.0,
            thickness: 0.15,
            wireframe: false,
        };
        st.make_plane();
        assert_eq!(st.hexagons.len(), 12 * 12);
        for (i, h) in st.hexagons.iter().enumerate() {
            for (j, n) in h.neighbors.iter().enumerate() {
                let Some(n) = *n else { continue };
                assert_eq!(
                    st.hexagons[n].neighbors[(j + 3) % 6],
                    Some(i),
                    "cell {i} arm {j} points at {n}, which does not point back"
                );
            }
        }
    }

    /// Neighbours are adjacent: one cell away, not two. A grid whose
    /// directions are wrong still links up mutually but draws lines across
    /// gaps.
    #[test]
    fn neighbours_touch() {
        let mut st = HexTrail {
            rot: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.0, false),
            trackball: Trackball::new(),
            grid_w: 0,
            grid_h: 0,
            hexagons: Vec::new(),
            live_count: 0,
            state: Phase::First,
            fade_ratio: 1.0,
            colors: Vec::new(),
            count: 6,
            speed: 1.0,
            thickness: 0.15,
            wireframe: false,
        };
        st.make_plane();
        // One cell across is 2/grid_w; the six neighbours are all that far.
        let step = 2.0 / st.grid_w as f32;
        for h in &st.hexagons {
            for n in h.neighbors.iter().flatten() {
                let p = st.hexagons[*n].pos;
                let d = ((p[0] - h.pos[0]).powi(2) + (p[1] - h.pos[1]).powi(2)).sqrt();
                assert!(
                    (d - step).abs() < step * 0.15,
                    "a neighbour {d} away when a cell is {step} across"
                );
            }
        }
    }

    /// It grows, it fills up, and then it starts again. All three have to
    /// happen: growth that never stops never fades, and a fade that never
    /// restarts is a blank screen.
    #[test]
    fn it_grows_and_starts_over() {
        let mut r = start(StartArgs::new(640, 480, "count=4&speed=20", 20260811));
        let size = |r: &Runner3d| r.frame().vertices.len();
        r.step();
        // The first tick starts one cell in the middle growing, so the first
        // frame is that cell's neighbours' borders and nothing else.
        let first = size(&r);
        assert!(first > 0 && first < 200, "it started with {first} vertices");
        let mut most = 0;
        let mut least_after = usize::MAX;
        for _ in 0..400 {
            r.step();
            most = most.max(size(&r));
        }
        assert!(most > 500, "it only ever drew {most} vertices");
        // Somewhere in the next stretch it has to fade away and come back.
        for _ in 0..800 {
            r.step();
            least_after = least_after.min(size(&r));
        }
        assert!(least_after < most / 2, "it never faded out");
    }
}
