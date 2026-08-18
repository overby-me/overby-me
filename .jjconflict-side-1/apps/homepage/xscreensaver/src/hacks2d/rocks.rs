//! Port of `hacks/rocks.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2013 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! 18-Sep-97: Johannes Keukelaar <johannes@nada.kth.se>: Added some color.
//! Using -mono gives the old behaviour.  (Modified by jwz.)
//!
//! Flying through an asteroid field.  Based on TI Explorer Lisp code by
//! John Nguyen <johnn@hx.lcs.mit.edu>
//! ```
//!
//! Each rock sits at a polar position and a depth, and every tick brings it a
//! little closer. Its size on screen is the arctangent of half a unit over the
//! depth, so a rock creeps in from nothing and then rushes past. The observer
//! rotates and drifts sideways at the same time, which is what makes it feel
//! like flying rather than falling.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_random_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, XRectangle, random,
    random_below,
};

const MIN_DEPTH: i32 = 2; // rocks disappear when they get this close
const MAX_DEPTH: i32 = 60; // this is where rocks appear
const MIN_SIZE: i32 = 3; // how small where pixmaps are not used
const MAX_SIZE: i32 = 400; // how big (in pixels) rocks are at depth 1
const DEPTH_SCALE: i32 = 100; // how many ticks there are between depths
const SIN_RESOLUTION: i32 = 1000;

const MAX_DEP: f32 = 0.3; // how far the displacement can be (percent)
const DIRECTION_CHANGE_RATE: i32 = 60;
const MAX_DEP_SPEED: i32 = 5; // maximum speed for movement
/// Only 0 and 1. Distinguishes the fact that these are the rocks that are
/// moving (1) or the rocks source (0).
const MOVE_STYLE: f64 = 0.0;

/// The rock, as seven points on a unit square. Upstream renders this once per
/// size into a stack of 400 depth-1 pixmaps and blits the right one; here it is
/// filled straight onto the screen, clipped to the same square. A polygon fill
/// is local and cheap, and the pixmap stack would be a hundred megabytes in a
/// framebuffer that stores a word per pixel rather than a bit.
const ROCK: [(f64, f64); 7] = [
    (0.15, 0.85),
    (0.00, 0.20),
    (0.30, 0.00),
    (0.40, 0.10),
    (0.90, 0.10),
    (1.00, 0.55),
    (0.45, 1.00),
];

#[derive(Clone, Copy, Default)]
struct Rock {
    real_size: i32,
    r: i32,
    theta: i32,
    depth: i32,
    size: i32,
    x: i32,
    y: i32,
    diff: i32,
    color: usize,
}

struct State {
    sins: Vec<f64>,
    coss: Vec<f64>,
    depths: Vec<f64>,

    width: i32,
    height: i32,
    midx: i32,
    midy: i32,
    dep_x: i32,
    dep_y: i32,
    ncolors: usize,
    max_dep: f32,
    erase_gc: Gc,
    draw_gcs: Vec<Gc>,
    rotate_p: bool,
    move_p: bool,
    speed: i32,
    three_d: bool,
    three_d_left_gc: Gc,
    three_d_right_gc: Gc,
    three_d_delta: f64,

    rocks: Vec<Rock>,
    delay: u32,

    move_current_dep: [i32; 2],
    move_speed: [i32; 2],
    move_direction: [i32; 2],
    move_limit: [i32; 2],

    /// Observer Z rotation.
    current_delta: i32,
    new_delta: i32,
    dchange_tick: i32,
}

impl State {
    fn getzdiff(&self, z: i32) -> f64 {
        self.three_d_delta
            * 40.0
            * (1.0
                - ((MAX_DEPTH * DEPTH_SCALE / 2) as f64 / (z as f64 + 20.0 * DEPTH_SCALE as f64)))
    }

    fn rock_reset(&mut self, d: &mut Dpy, i: usize) {
        let rock = &mut self.rocks[i];
        rock.real_size = MAX_SIZE;
        rock.r = ((SIN_RESOLUTION as f64 * 0.7) as i32)
            + (random() % (30 * SIN_RESOLUTION) as u32) as i32;
        rock.theta = random_below(SIN_RESOLUTION);
        rock.depth = MAX_DEPTH * DEPTH_SCALE;
        rock.color = random_below(self.ncolors as i32) as usize;
        self.rock_compute(i);
        self.rock_draw(d, i, true);
    }

    fn rock_tick(&mut self, d: &mut Dpy, i: usize, delta: i32) {
        if self.rocks[i].depth > 0 {
            self.rock_draw(d, i, false);
            self.rocks[i].depth -= self.speed;
            if self.rotate_p {
                self.rocks[i].theta = (self.rocks[i].theta + delta) % SIN_RESOLUTION;
            }
            while self.rocks[i].theta < 0 {
                self.rocks[i].theta += SIN_RESOLUTION;
            }
            if self.rocks[i].depth < MIN_DEPTH * DEPTH_SCALE {
                self.rocks[i].depth = 0;
            } else {
                self.rock_compute(i);
                self.rock_draw(d, i, true);
            }
        } else if random_below(40) == 0 {
            self.rock_reset(d, i);
        }
    }

    fn rock_compute(&mut self, i: usize) {
        let depth = self.rocks[i].depth.clamp(0, self.depths.len() as i32 - 1);
        let factor = self.depths[depth as usize];
        let rsize = self.rocks[i].real_size as f64 * factor;
        let diff = self.getzdiff(self.rocks[i].depth) as i32;
        let theta = self.rocks[i].theta.rem_euclid(SIN_RESOLUTION) as usize;
        let r = self.rocks[i].r as f64;

        let rock = &mut self.rocks[i];
        rock.size = (rsize + 0.5) as i32;
        rock.diff = diff;
        rock.x = self.midx + (self.coss[theta] * r * factor) as i32;
        rock.y = self.midy + (self.sins[theta] * r * factor) as i32;

        if self.move_p {
            // move_factor is 0 when the rock is close, 1 when far.
            let move_factor =
                MOVE_STYLE - (rock.depth as f64 / ((MAX_DEPTH + 1) as f64 * DEPTH_SCALE as f64));
            rock.x += (self.dep_x as f64 * move_factor) as i32;
            rock.y += (self.dep_y as f64 * move_factor) as i32;
        }
    }

    /// Paint the rock shape, or the square that erases it. `gc` decides which:
    /// clipping to the rock's square and filling it with the background first
    /// is exactly what `XCopyPlane` of a one-bit rock did.
    fn blit_rock(&self, d: &mut Dpy, gc: &Gc, x: i32, y: i32, size: i32) {
        let mut gc = gc.clone();
        gc.set_clip_rect(XRectangle {
            x,
            y,
            width: size,
            height: size,
        });
        let fg = gc.foreground;
        gc.set_foreground(gc.background);
        d.win().fill_rectangle(&gc, x, y, size, size);
        gc.set_foreground(fg);
        let pts: Vec<XPoint> = ROCK
            .iter()
            .map(|(px, py)| XPoint {
                x: x + (size as f64 * px) as i32,
                y: y + (size as f64 * py) as i32,
            })
            .collect();
        d.win().fill_polygon(&gc, &pts);
    }

    fn rock_draw(&mut self, d: &mut Dpy, i: usize, draw_p: bool) {
        let rock = self.rocks[i];
        let gc = if draw_p {
            if self.three_d {
                self.erase_gc.clone()
            } else {
                self.draw_gcs[rock.color % self.draw_gcs.len()].clone()
            }
        } else {
            self.erase_gc.clone()
        };

        if rock.x <= 0 || rock.y <= 0 || rock.x >= self.width || rock.y >= self.height {
            // This means that if a rock were to go off the screen at 12:00, but
            // would have been visible at 3:00, it will not come back once the
            // observer rotates around so that the rock would have been visible
            // again. Oh well.
            if !self.move_p {
                self.rocks[i].depth = 0;
            }
            return;
        }

        if rock.size <= 1 {
            if self.three_d {
                let g = if draw_p { &self.three_d_left_gc } else { &gc };
                d.win().draw_point(g, rock.x - rock.diff, rock.y);
                let g = if draw_p { &self.three_d_right_gc } else { &gc };
                d.win().draw_point(g, rock.x + rock.diff, rock.y);
            } else {
                d.win().draw_point(&gc, rock.x, rock.y);
            }
        } else if rock.size <= MIN_SIZE || !draw_p {
            let (hw, s) = (rock.size / 2, rock.size);
            if self.three_d {
                let g = if draw_p { &self.three_d_left_gc } else { &gc };
                d.win()
                    .fill_rectangle(g, rock.x - hw - rock.diff, rock.y - hw, s, s);
                let g = if draw_p { &self.three_d_right_gc } else { &gc };
                d.win()
                    .fill_rectangle(g, rock.x - hw + rock.diff, rock.y - hw, s, s);
            } else {
                d.win().fill_rectangle(&gc, rock.x - hw, rock.y - hw, s, s);
            }
        } else if rock.size < MAX_SIZE {
            let (hw, s) = (rock.size / 2, rock.size);
            if self.three_d {
                let left = self.three_d_left_gc.clone();
                self.blit_rock(d, &left, rock.x - hw - rock.diff, rock.y - hw, s);
                let right = self.three_d_right_gc.clone();
                self.blit_rock(d, &right, rock.x - hw + rock.diff, rock.y - hw, s);
            } else {
                self.blit_rock(d, &gc, rock.x - hw, rock.y - hw, s);
            }
        }
    }

    /// 0 for x, 1 for y.
    fn compute_move(&mut self, axe: usize) -> i32 {
        self.move_limit[0] = self.midx;
        self.move_limit[1] = self.midy;

        // Adjust the displacement.
        self.move_current_dep[axe] += self.move_speed[axe];

        if self.move_current_dep[axe] > (self.move_limit[axe] as f32 * self.max_dep) as i32 {
            // This is when we reach the upper screen limit.
            if self.move_current_dep[axe] > self.move_limit[axe] {
                self.move_current_dep[axe] = self.move_limit[axe];
            }
            self.move_direction[axe] = -1;
        }
        if self.move_current_dep[axe] < (-self.move_limit[axe] as f32 * self.max_dep) as i32 {
            // This is when we reach the lower screen limit.
            if self.move_current_dep[axe] < -self.move_limit[axe] {
                self.move_current_dep[axe] = -self.move_limit[axe];
            }
            self.move_direction[axe] = 1;
        }

        // Adjust the speed.
        if self.move_direction[axe] == 1 {
            self.move_speed[axe] += 1;
        } else if self.move_direction[axe] == -1 {
            self.move_speed[axe] -= 1;
        }
        self.move_speed[axe] = self.move_speed[axe].clamp(-MAX_DEP_SPEED, MAX_DEP_SPEED);

        if self.move_p && random_below(DIRECTION_CHANGE_RATE) == 0 {
            // We change direction.
            let change = (random() & 1) as i32;
            if change != 1 {
                if self.move_direction[axe] == 0 {
                    // 0 becomes either 1 or -1.
                    self.move_direction[axe] = change - 1;
                } else {
                    // -1 or 1 become 0.
                    self.move_direction[axe] = 0;
                }
            }
        }
        self.move_current_dep[axe]
    }

    fn tick_rocks(&mut self, d: &mut Dpy, delta: i32) {
        if self.move_p {
            self.dep_x = self.compute_move(0);
            self.dep_y = self.compute_move(1);
        }
        for i in 0..self.rocks.len() {
            self.rock_tick(d, i, delta);
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let bg = d.res.pixel("background");
    let fg = d.res.pixel("foreground");

    let mut ncolors = d.res.int("colors").max(2) as usize;
    // colors[0] is the background, so a rock that draws in it is invisible.
    // That is upstream's arrangement, and part of how sparse the field looks.
    let mut colors = vec![bg];
    colors.extend(
        make_random_colormap(ncolors - 1, true)
            .iter()
            .map(|c| c.pixel),
    );
    if colors.len() < 2 {
        colors = vec![bg, fg];
    }
    ncolors = colors.len();

    let mut st = State {
        sins: (0..SIN_RESOLUTION)
            .map(|i| ((i as f64) / (SIN_RESOLUTION as f64 / 2.0) * std::f64::consts::PI).sin())
            .collect(),
        coss: (0..SIN_RESOLUTION)
            .map(|i| ((i as f64) / (SIN_RESOLUTION as f64 / 2.0) * std::f64::consts::PI).cos())
            .collect(),
        // We actually only need i/speed of these, but why not.
        depths: (0..(MAX_DEPTH + 1) * DEPTH_SCALE)
            .map(|i| {
                if i == 0 {
                    // Avoid division by 0.
                    std::f64::consts::FRAC_PI_2
                } else {
                    (0.5 / (i as f64 / DEPTH_SCALE as f64)).atan()
                }
            })
            .collect(),
        width: d.width(),
        height: d.height(),
        midx: d.width() / 2,
        midy: d.height() / 2,
        dep_x: 0,
        dep_y: 0,
        ncolors,
        draw_gcs: colors.iter().map(|c| Gc::new(*c, bg)).collect(),
        max_dep: 0.0,
        erase_gc: Gc::new(bg, bg),
        rotate_p: d.res.bool("rotate"),
        move_p: d.res.bool("move"),
        speed: d.res.int("speed").clamp(1, 100),
        three_d: d.res.bool("use3d"),
        three_d_left_gc: Gc::new(d.res.pixel("left3d"), bg),
        three_d_right_gc: Gc::new(d.res.pixel("right3d"), bg),
        three_d_delta: d.res.float("delta3d"),
        rocks: vec![Rock::default(); d.res.int("count").max(1) as usize],
        delay: d.res.int("delay").max(0) as u32,
        move_current_dep: [0; 2],
        move_speed: [0; 2],
        move_direction: [0; 2],
        move_limit: [0; 2],
        current_delta: 0,
        new_delta: 0,
        dchange_tick: 0,
    };
    st.max_dep = if st.move_p { MAX_DEP } else { 0.0 };
    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.current_delta != self.new_delta {
            self.dchange_tick += 1;
            if self.dchange_tick == 6 {
                self.dchange_tick = 0;
                if self.current_delta < self.new_delta {
                    self.current_delta += 1;
                } else {
                    self.current_delta -= 1;
                }
            }
        } else if random_below(50) == 0 {
            self.new_delta = random_below(11) - 5;
            if random_below(10) == 0 {
                self.new_delta *= 5;
            }
        }
        let delta = self.current_delta;
        self.tick_rocks(d, delta);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.midx = width / 2;
        self.midy = height / 2;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: Black",
    ".foreground: #E9967A",
    "*fpsSolid: true",
    "*colors: 5",
    "*count: 100",
    "*delay: 50000",
    "*speed: 100",
    "*rotate: true",
    "*move: true",
    "*use3d: False",
    "*left3d: Blue",
    "*right3d: Red",
    "*delta3d: 1.5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("count", "Count", 0.0, 200.0, 1.0, 0, "100"),
    Opt::slider("speed", "Velocity", 1.0, 100.0, 1.0, 0, "100"),
    Opt::boolean("rotate", "Rotation", "true"),
    Opt::boolean("move", "Steering", "true"),
    Opt::boolean("use3d", "Do Red/Blue 3D separation", "False"),
    Opt::slider("colors", "Number of colors", 1.0, 255.0, 1.0, 0, "5"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rocks",
    label: "Rocks",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=7x7PMI7LFK0"),
        blurb: "An asteroid field zooms by.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
