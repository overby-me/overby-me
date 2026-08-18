//! Port of `hacks/lightning.c`.
//!
//! ```text
//! lightning --- fractal lightning bolds
//!
//! Copyright (c) 1996 by Keith Romberg <kromberg@saxe.com>
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
//! Revision History:
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: Compatible with xscreensaver
//! 14-Jul-1996: Cleaned up code.
//! 27-Jun-1996: Written and submitted by Keith Romberg <kromberg@saxe.com>.
//! ```
//!
//! A bolt is a line from the top of the screen to the bottom, subdivided by
//! displacing the midpoint of every segment four times over, with up to two
//! forks branching off it the same way. It is drawn three times at widening
//! offsets so the core stays white and the edges take the storm's colour, then
//! every vertex is jittered by a shrinking amount each frame until it settles,
//! which is the crackle.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

const BOLT_NUMBER: usize = 4;
const BOLT_ITERATION: i32 = 4;
const LONG_FORK_ITERATION: i32 = 3;
const MEDIUM_FORK_ITERATION: i32 = 2;
const SMALL_FORK_ITERATION: i32 = 1;

const WIDTH_VARIATION: i32 = 30;
const HEIGHT_VARIATION: i32 = 15;

const DELAY_TIME_AMOUNT: i32 = 15;
const MULTI_DELAY_TIME_BASE: i32 = 5;

const MAX_WIGGLES: i32 = 16;
const WIGGLE_BASE: i32 = 8;
const WIGGLE_AMOUNT: i32 = 14;

const RANDOM_FORK_PROBABILITY: i32 = 4;

const FIRST_LEVEL_STRIKE: i32 = 0;
const LEVEL_ONE_STRIKE: i32 = 1;
const LEVEL_TWO_STRIKE: i32 = 2;

/// How many vertices of the main bolt are drawn.
const BOLT_VERTICES: usize = (1 << BOLT_ITERATION) - 1;

/// How many `generate` actually writes: its recursion has `2^iter` leaves, one
/// more than the array upstream declares. The extra lands in the next struct
/// field, which is assigned immediately afterwards, so it never shows; here
/// there is simply room for it.
const BOLT_WRITTEN: usize = 1 << BOLT_ITERATION;

const NUMBER_FORK_VERTICES: usize = 9;

const FLASH_PROBABILITY: i32 = 20;
/// Half the total duration of the bolt.
const MAX_FLASH_AMOUNT: i32 = 2;

#[derive(Clone, Copy)]
struct Fork {
    fork_vertices: [XPoint; NUMBER_FORK_VERTICES],
    num_used: usize,
}

impl Default for Fork {
    fn default() -> Self {
        Self {
            fork_vertices: [XPoint::default(); NUMBER_FORK_VERTICES],
            num_used: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Bolt {
    end1: XPoint,
    end2: XPoint,
    middle: [XPoint; BOLT_WRITTEN],
    fork_number: usize,
    forks_start: [usize; 2],
    branch: [Fork; 2],
    wiggle_number: i32,
    wiggle_amount: i32,
    delay_time: i32,
    flash_begin: i32,
    flash_stop: i32,
    visible: bool,
    strike_level: i32,
}

impl Default for Bolt {
    fn default() -> Self {
        Self {
            end1: XPoint::default(),
            end2: XPoint::default(),
            middle: [XPoint::default(); BOLT_WRITTEN],
            fork_number: 0,
            forks_start: [0; 2],
            branch: [Fork::default(); 2],
            wiggle_number: 0,
            wiggle_amount: 0,
            delay_time: 0,
            flash_begin: 0,
            flash_stop: 0,
            visible: false,
            strike_level: 0,
        }
    }
}

struct Storm {
    mi: ModeInfo,
    bolts: [Bolt; BOLT_NUMBER],
    scr_width: i32,
    scr_height: i32,
    multi_strike: usize,
    draw_time: i32,
    stage: i32,
    busy_loop: i32,
    color: usize,
}

fn distance(a: XPoint, b: XPoint) -> i32 {
    (((a.x - b.x) as f64 * (a.x - b.x) as f64 + (a.y - b.y) as f64 * (a.y - b.y) as f64).sqrt())
        as i32
}

fn setup_multi_strike() -> usize {
    let multi_prob = nrand(100);
    if multi_prob < 50 {
        1
    } else if (51..75).contains(&multi_prob) {
        2
    } else if (76..92).contains(&multi_prob) {
        3
    } else {
        BOLT_NUMBER
    }
}

/// Upstream compares a value below `FLASH_PROBABILITY` against `<=` it, so this
/// is always true and every bolt flashes.
fn flashing_strike() -> bool {
    nrand(FLASH_PROBABILITY) <= FLASH_PROBABILITY
}

fn flash_duration(total_duration: i32) -> (i32, i32) {
    let mid = total_duration / MAX_FLASH_AMOUNT;
    let d = nrand(total_duration / MAX_FLASH_AMOUNT) / 2;
    (mid - d, mid + d)
}

/// Midpoint displacement: halve the segment, kick the middle, recurse.
fn generate(a: XPoint, b: XPoint, iter: i32, verts: &mut [XPoint], idx: &mut usize) {
    let mid = XPoint {
        x: (a.x + b.x) / 2 + nrand(WIDTH_VARIATION) - WIDTH_VARIATION / 2,
        y: (a.y + b.y) / 2 + nrand(HEIGHT_VARIATION) - HEIGHT_VARIATION / 2,
    };
    if iter == 0 {
        if *idx < verts.len() {
            verts[*idx] = mid;
        }
        *idx += 1;
        return;
    }
    generate(a, mid, iter - 1, verts, idx);
    generate(mid, b, iter - 1, verts, idx);
}

fn create_fork(f: &mut Fork, start: XPoint, end: XPoint, level: usize) {
    let mut tmp = 1usize;
    f.fork_vertices[0] = start;

    if level <= 6 {
        generate(
            start,
            end,
            LONG_FORK_ITERATION,
            &mut f.fork_vertices,
            &mut tmp,
        );
        f.num_used = 9;
    } else if (7..=11).contains(&level) || distance(start, end) > 100 {
        // Upstream writes these as two branches: one for the middle levels,
        // one for a fork low down that still has a long way to fall.
        generate(
            start,
            end,
            MEDIUM_FORK_ITERATION,
            &mut f.fork_vertices,
            &mut tmp,
        );
        f.num_used = 5;
    } else {
        generate(
            start,
            end,
            SMALL_FORK_ITERATION,
            &mut f.fork_vertices,
            &mut tmp,
        );
        f.num_used = 3;
    }
    f.fork_vertices[f.num_used - 1] = end;
}

fn wiggle_line(p: &mut [XPoint], number: usize, amount: i32) {
    for item in p.iter_mut().take(number) {
        item.x += nrand(amount) - amount / 2;
        item.y += nrand(amount) - amount / 2;
    }
}

fn wiggle_bolt(bolt: &mut Bolt) {
    let amount = bolt.wiggle_amount;
    wiggle_line(&mut bolt.middle, BOLT_VERTICES, amount);
    bolt.end2.x += nrand(amount) - amount / 2;
    bolt.end2.y += nrand(amount) - amount / 2;

    for i in 0..bolt.fork_number {
        let n = bolt.branch[i].num_used;
        wiggle_line(&mut bolt.branch[i].fork_vertices, n, amount);
        bolt.branch[i].fork_vertices[0] = bolt.middle[bolt.forks_start[i]];
    }

    if bolt.wiggle_amount > 1 {
        bolt.wiggle_amount -= 1;
    } else {
        bolt.wiggle_amount = 0;
    }
}

fn update_bolt(bolt: &mut Bolt, time_now: i32) {
    wiggle_bolt(bolt);
    if bolt.wiggle_amount == 0 && bolt.wiggle_number > 2 {
        bolt.wiggle_number = 0;
    }
    if time_now % 3 == 0 {
        bolt.wiggle_amount += 1;
    }

    bolt.visible =
        (time_now >= bolt.delay_time && time_now < bolt.flash_begin) || time_now > bolt.flash_stop;

    bolt.strike_level = if time_now == bolt.delay_time {
        FIRST_LEVEL_STRIKE
    } else if time_now == bolt.delay_time + 1 {
        LEVEL_ONE_STRIKE
    } else if time_now > bolt.delay_time + 1 && time_now <= bolt.delay_time + bolt.flash_begin - 2 {
        LEVEL_TWO_STRIKE
    } else if time_now == bolt.delay_time + bolt.flash_begin - 1
        || time_now == bolt.delay_time + bolt.flash_stop + 1
    {
        // The frame on either side of the flash, which upstream spells as two
        // branches with the same answer.
        LEVEL_ONE_STRIKE
    } else {
        LEVEL_TWO_STRIKE
    };
}

impl Storm {
    fn random_storm(&mut self) {
        let (w, h) = (self.scr_width, self.scr_height);
        for i in 0..self.multi_strike {
            let mut b = Bolt {
                end1: XPoint { x: nrand(w), y: 0 },
                end2: XPoint { x: nrand(w), y: h },
                wiggle_number: WIGGLE_BASE + nrand(MAX_WIGGLES),
                ..Bolt::default()
            };
            if flashing_strike() {
                let (a, z) = flash_duration(b.wiggle_number);
                b.flash_begin = a;
                b.flash_stop = z;
            }
            b.wiggle_amount = WIGGLE_AMOUNT;
            b.delay_time = if i == 0 {
                nrand(DELAY_TIME_AMOUNT)
            } else {
                nrand(DELAY_TIME_AMOUNT) + MULTI_DELAY_TIME_BASE * i as i32
            };
            b.strike_level = FIRST_LEVEL_STRIKE;

            let mut tmp = 0usize;
            generate(b.end1, b.end2, BOLT_ITERATION, &mut b.middle, &mut tmp);
            b.fork_number = 0;
            b.visible = false;

            for j in 0..BOLT_VERTICES {
                if b.fork_number >= 2 {
                    break;
                }
                if nrand(100) < RANDOM_FORK_PROBABILITY {
                    let p = XPoint { x: nrand(w), y: h };
                    b.forks_start[b.fork_number] = j;
                    let start = b.middle[j];
                    let n = b.fork_number;
                    create_fork(&mut b.branch[n], start, p, j);
                    b.fork_number += 1;
                }
            }
            self.bolts[i] = b;
        }
    }

    fn storm_active(&self) -> bool {
        (0..self.multi_strike).any(|i| self.bolts[i].wiggle_number > 0)
    }

    /// Segments are drawn shifted by `offset` sideways, and also downwards
    /// when the segment climbs, which is what gives the three passes their
    /// slightly ragged edge rather than a clean parallel outline.
    fn draw_line(&mut self, d: &mut Dpy, points: &[XPoint], number: usize, offset: i32) {
        for i in 0..number.saturating_sub(1) {
            let (a, b) = (points[i], points[i + 1]);
            if a.y <= b.y {
                d.win()
                    .draw_line(&self.mi.gc, a.x + offset, a.y, b.x + offset, b.y);
            } else if a.x < b.x {
                d.win().draw_line(
                    &self.mi.gc,
                    a.x + offset,
                    a.y + offset,
                    b.x + offset,
                    b.y + offset,
                );
            } else {
                d.win().draw_line(
                    &self.mi.gc,
                    a.x - offset,
                    a.y + offset,
                    b.x - offset,
                    b.y + offset,
                );
            }
        }
    }

    /// One pass of the bolt at a given sideways offset.
    fn strike_pass(&mut self, d: &mut Dpy, bolt: &Bolt, offset: i32) {
        let last = bolt.middle[BOLT_VERTICES - 1];
        let m0 = bolt.middle[0];
        d.win().draw_line(
            &self.mi.gc,
            bolt.end1.x + offset,
            bolt.end1.y,
            m0.x + offset,
            m0.y,
        );
        let middle = bolt.middle;
        self.draw_line(d, &middle, BOLT_VERTICES, offset);
        d.win().draw_line(
            &self.mi.gc,
            last.x + offset,
            last.y,
            bolt.end2.x + offset,
            bolt.end2.y,
        );
        for i in 0..bolt.fork_number {
            let f = bolt.branch[i];
            self.draw_line(d, &f.fork_vertices, f.num_used, offset);
        }
    }

    fn first_strike(&mut self, d: &mut Dpy, bolt: &Bolt) {
        let white = self.mi.white;
        self.mi.gc.set_foreground(white);
        self.strike_pass(d, bolt, 0);
    }

    fn level1_strike(&mut self, d: &mut Dpy, bolt: &Bolt) {
        let c = if self.mi.npixels() > 2 {
            self.mi.pixel(self.color)
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(c);
        self.strike_pass(d, bolt, -1);
        self.strike_pass(d, bolt, 1);
        self.first_strike(d, bolt);
    }

    /// Originally meant to be a little darker than the level one strike;
    /// changed to work on multiple screens and add colour variety.
    fn level2_strike(&mut self, d: &mut Dpy, bolt: &Bolt) {
        let c = if self.mi.npixels() > 2 {
            self.mi.pixel(self.color)
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(c);
        self.strike_pass(d, bolt, -2);
        self.strike_pass(d, bolt, 2);
        self.level1_strike(d, bolt);
    }

    fn draw_bolt(&mut self, d: &mut Dpy, i: usize) {
        let bolt = self.bolts[i];
        if !bolt.visible {
            return;
        }
        if bolt.strike_level == FIRST_LEVEL_STRIKE {
            self.first_strike(d, &bolt);
        } else if bolt.strike_level == LEVEL_ONE_STRIKE {
            self.level1_strike(d, &bolt);
        } else {
            self.level2_strike(d, &bolt);
        }
    }

    fn restart(&mut self) {
        self.multi_strike = setup_multi_strike();
        self.random_storm();
        self.stage = 0;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Bright);
    let (scr_width, scr_height) = (mi.width, mi.height);
    let mut st = Storm {
        mi,
        bolts: [Bolt::default(); BOLT_NUMBER],
        scr_width,
        scr_height,
        multi_strike: 1,
        draw_time: 0,
        stage: 0,
        busy_loop: 0,
        color: 0,
    };
    st.restart();
    Box::new(st)
}

impl Screenhack for Storm {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        match self.stage {
            0 => {
                d.clear_window();
                self.color = nrand(self.mi.npixels().max(1)) as usize;
                self.draw_time = 0;
                self.stage = if self.storm_active() { 1 } else { 4 };
            }
            1 => {
                for i in 0..self.multi_strike {
                    if self.bolts[i].visible {
                        self.draw_bolt(d, i);
                    }
                    let t = self.draw_time;
                    update_bolt(&mut self.bolts[i], t);
                }
                self.draw_time += 1;
                self.stage += 1;
                self.busy_loop = 0;
            }
            2 => {
                self.busy_loop += 1;
                if self.busy_loop > 6 {
                    self.stage += 1;
                    self.busy_loop = 0;
                }
            }
            3 => {
                d.clear_window();
                self.stage = if self.storm_active() { 1 } else { 4 };
            }
            _ => {
                self.busy_loop += 1;
                if self.busy_loop > 100 {
                    self.busy_loop = 0;
                }
                self.restart();
            }
        }
        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.scr_width = width;
        self.scr_height = height;
        self.restart();
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*ncolors: 64",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lightning",
    label: "Lightning",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Keith Romberg",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=lUUdHtPvp5Y"),
        blurb: "Crackling fractal lightning bolts.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
