//! Port of `hacks/braid.c`.
//!
//! ```text
//! braid --- random braids around a circle and then changes the color in
//!           a rotational pattern
//!
//! Copyright (c) 1995 by John Neil.
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
//! 10-May-1997: Jamie Zawinski <jwz@jwz.org> compatible with xscreensaver
//! 01-Sep-1995: color knotted components differently, J. Neil.
//! 29-Aug-1995: Written.
//! ```
//!
//! Concentric rings that swap places with their neighbours as they go round,
//! which is to say a braid closed into a knot. A braid word is rolled at
//! random, the strands are grouped into linked components, and each component
//! is given its own colour so you can see which strands belong to the same
//! loop. Upstream can also colour by angle rather than by component; the
//! by-component variant is the one it ships with.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs};

/// The longest and shortest a braid word may be.
const MAX_LENGTH: usize = 50;
const MIN_LENGTH: i32 = 8;
/// How many strands the braid may have.
const MAX_STRANDS: usize = 15;
const MIN_STRANDS: i32 = 3;
/// The rate at which the colours spin.
const SPINRATE: f32 = 12.0;

/// `INTRAND(min, max)`, inclusive at both ends.
fn intrand(min: i32, max: i32) -> i32 {
    nrand(max + 1 - min) + min
}

struct Braid {
    mi: ModeInfo,
    linewidth: i32,
    braidword: [i32; MAX_LENGTH],
    components: [i32; MAX_STRANDS],
    nstrands: usize,
    braidlength: usize,
    startcolor: f32,
    center_x: i32,
    center_y: i32,
    min_radius: f32,
    max_radius: f32,
    age: i32,
    color_direction: f32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // UNIFORM_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let mut st = Braid {
        mi,
        linewidth: 1,
        braidword: [0; MAX_LENGTH],
        components: [0; MAX_STRANDS],
        nstrands: MIN_STRANDS as usize,
        braidlength: MIN_LENGTH as usize,
        startcolor: 0.0,
        center_x: 0,
        center_y: 0,
        min_radius: 1.0,
        max_radius: 2.0,
        age: 0,
        color_direction: 1.0,
    };
    st.restart(d);
    Box::new(st)
}

impl Braid {
    /// Where strand `string` ends up after the whole word has been applied,
    /// starting from `position` and wrapping.
    fn applyword(&self, string: i32, position: usize) -> i32 {
        let mut c = string;
        for i in position..self.braidlength {
            if c == self.braidword[i].abs() {
                c -= 1;
            } else if c == self.braidword[i].abs() - 1 {
                c += 1;
            }
        }
        for i in 0..position {
            if c == self.braidword[i].abs() {
                c -= 1;
            } else if c == self.braidword[i].abs() - 1 {
                c += 1;
            }
        }
        c
    }

    /// Where in the colour map a strand's component sits, plus how far round
    /// the circle we have come.
    fn color_of(&self, strand: i32, i: usize, color: f32, psi: f32, t: f32, npixels: i32) -> f32 {
        if npixels <= 2 {
            return 0.0;
        }
        let pi = std::f32::consts::PI;
        let idx = self
            .applywordbackto(strand, i)
            .clamp(0, MAX_STRANDS as i32 - 1);
        let mut c = color
            + SPINRATE * self.components[idx as usize] as f32
            + (psi + t) / 2.0 / pi * npixels as f32;
        while c as i32 >= npixels {
            c -= npixels as f32;
        }
        while (c as i32) < 0 {
            c += npixels as f32;
        }
        c
    }

    /// Where strand `string` came from, running the word backwards.
    fn applywordbackto(&self, string: i32, position: usize) -> i32 {
        let mut c = string;
        for i in (0..position).rev() {
            if c == self.braidword[i].abs() {
                c -= 1;
            } else if c == self.braidword[i].abs() - 1 {
                c += 1;
            }
        }
        c
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.center_x = self.mi.width / 2;
        self.center_y = self.mi.height / 2;
        self.age = 0;

        // jwz: go in the other direction sometimes.
        self.color_direction = if lrand() & 1 == 1 { 1.0 } else { -1.0 };

        d.clear_window();

        let min_length = self.center_x.min(self.center_y) as f32;
        self.min_radius = min_length * 0.30;
        self.max_radius = min_length * 0.90;

        self.nstrands = if self.mi.count < MIN_STRANDS {
            MIN_STRANDS as usize
        } else {
            intrand(
                MIN_STRANDS,
                (MAX_STRANDS as i32)
                    .min(self.mi.count)
                    .min(((self.max_radius - self.min_radius) / 5.0) as i32)
                    .max(MIN_STRANDS),
            ) as usize
        };
        self.braidlength = intrand(
            MIN_LENGTH,
            (MAX_LENGTH as i32 - 1).min(self.nstrands as i32 * 6),
        ) as usize;

        let roll = |n: usize| intrand(1, n as i32 - 1) * (intrand(1, 2) * 2 - 3);

        for i in 0..self.braidlength {
            self.braidword[i] = roll(self.nstrands);
            if i > 0 {
                while self.braidword[i] == -self.braidword[i - 1] {
                    self.braidword[i] = roll(self.nstrands);
                }
            }
        }
        while self.braidword[0] == -self.braidword[self.braidlength - 1] {
            self.braidword[self.braidlength - 1] = roll(self.nstrands);
        }

        // Keep adding letters until every strand takes part, so the braid is
        // one piece rather than a few loose rings.
        let mut count;
        loop {
            let mut used = [0i32; MAX_STRANDS + 1];
            count = 0;
            for i in 0..self.braidlength {
                used[self.braidword[i].unsigned_abs() as usize] += 1;
            }
            for u in used.iter().take(self.nstrands) {
                count += i32::from(*u > 0);
            }
            if count < self.nstrands as i32 - 1 && self.braidlength < MAX_LENGTH {
                let at = self.braidlength;
                self.braidword[at] = roll(self.nstrands);
                while self.braidword[at] == -self.braidword[at - 1]
                    || self.braidword[0] == -self.braidword[at]
                {
                    self.braidword[at] = roll(self.nstrands);
                }
                self.braidlength += 1;
            }
            if count >= self.nstrands as i32 - 1 || self.braidlength >= MAX_LENGTH {
                break;
            }
        }

        self.startcolor = if self.mi.npixels() > 2 {
            nrand(self.mi.npixels()) as f32
        } else {
            0.0
        };

        // Group the strands into linked components.
        self.components = [0; MAX_STRANDS];
        let mut c = 1;
        let mut comp = 0usize;
        self.components[0] = 1;
        loop {
            let mut i = comp as i32;
            loop {
                i = self.applyword(i, 0);
                if !(0..self.nstrands as i32).contains(&i) {
                    break;
                }
                self.components[i as usize] = self.components[comp];
                if i == comp as i32 {
                    break;
                }
            }
            let mut left = 0;
            for k in 0..self.nstrands {
                if self.components[k] == 0 {
                    left += 1;
                }
            }
            if left == 0 {
                break;
            }
            comp = 0;
            while comp < self.nstrands && self.components[comp] != 0 {
                comp += 1;
            }
            if comp >= self.nstrands {
                break;
            }
            c += 1;
            self.components[comp] = c;
        }

        self.linewidth = self.mi.size;
        if self.linewidth < 0 {
            self.linewidth = nrand(-self.linewidth) + 1;
        }
        if self.linewidth * self.linewidth * 8 > self.mi.width.min(self.mi.height) {
            // Upstream takes the smaller of one and the root here, so this is
            // always one or zero.
            self.linewidth = 1.min(((self.mi.width.min(self.mi.height) / 8) as f64).sqrt() as i32);
        }
        for i in 0..self.nstrands {
            if self.components[i] & 1 == 0 {
                self.components[i] *= -1;
            }
        }
    }
}

impl Screenhack for Braid {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let num_points = 500;
        let npixels = self.mi.npixels();
        self.mi.gc.set_line_width(self.linewidth);

        let theta = (2.0 * std::f32::consts::PI) / self.braidlength as f32;
        let t_inc = (2.0 * std::f32::consts::PI) / num_points as f32;
        let color_inc = npixels as f32 * self.color_direction / num_points as f32;
        self.startcolor += SPINRATE * color_inc;
        if self.startcolor as i32 >= npixels {
            self.startcolor = 0.0;
        }

        let r_diff = (self.max_radius - self.min_radius) / self.nstrands as f32;
        let color = self.startcolor;
        let (cx, cy) = (self.center_x as f32, self.center_y as f32);
        let half_pi = std::f32::consts::FRAC_PI_2;
        let pi = std::f32::consts::PI;

        let mut psi = 0.0f32;
        for i in 0..self.braidlength {
            psi += theta;
            let mut t = 0.0f32;
            while t < theta {
                for s in 0..self.nstrands as i32 {
                    if self.braidword[i].abs() == s {
                        continue;
                    }

                    if self.braidword[i].abs() - 1 == s {
                        // Crossing: the two strands trade places over this
                        // stretch, one passing in front of the other.
                        let r1 = self.min_radius + r_diff * s as f32;
                        let r2 = self.min_radius + r_diff * (s + 1) as f32;

                        let blend = |a: f32, b: f32, tt: f32| {
                            0.5 * (1.0 + (tt / theta * pi - half_pi).sin()) * b
                                + 0.5 * (1.0 + ((theta - tt) / theta * pi - half_pi).sin()) * a
                        };

                        if self.braidword[i] > 0 || (t - theta / 2.0).abs() > theta / 7.0 {
                            let c = self.color_of(s, i, color, psi, t, npixels);
                            let x1 = blend(r1, r2, t) * (t + psi).cos() + cx;
                            let y1 = blend(r1, r2, t) * (t + psi).sin() + cy;
                            let x2 = blend(r1, r2, t + t_inc) * (t + t_inc + psi).cos() + cx;
                            let y2 = blend(r1, r2, t + t_inc) * (t + t_inc + psi).sin() + cy;
                            let p = if npixels > 2 {
                                self.mi.pixel(c as usize)
                            } else {
                                self.mi.white
                            };
                            self.mi.gc.set_foreground(p);
                            d.win().draw_line(
                                &self.mi.gc,
                                x1 as i32,
                                y1 as i32,
                                x2 as i32,
                                y2 as i32,
                            );
                        }

                        if self.braidword[i] < 0 || (t - theta / 2.0).abs() > theta / 7.0 {
                            let c = self.color_of(s + 1, i, color, psi, t, npixels);
                            let x1 = blend(r2, r1, t) * (t + psi).cos() + cx;
                            let y1 = blend(r2, r1, t) * (t + psi).sin() + cy;
                            let x2 = blend(r2, r1, t + t_inc) * (t + t_inc + psi).cos() + cx;
                            let y2 = blend(r2, r1, t + t_inc) * (t + t_inc + psi).sin() + cy;
                            let p = if npixels > 2 {
                                self.mi.pixel(c as usize)
                            } else {
                                self.mi.white
                            };
                            self.mi.gc.set_foreground(p);
                            d.win().draw_line(
                                &self.mi.gc,
                                x1 as i32,
                                y1 as i32,
                                x2 as i32,
                                y2 as i32,
                            );
                        }
                    } else {
                        // No crossing: this strand just follows its own ring.
                        let c = self.color_of(s, i, color, psi, t, npixels);
                        let r1 = self.min_radius + r_diff * s as f32;
                        let x1 = r1 * (t + psi).cos() + cx;
                        let y1 = r1 * (t + psi).sin() + cy;
                        let x2 = r1 * (t + t_inc + psi).cos() + cx;
                        let y2 = r1 * (t + t_inc + psi).sin() + cy;
                        let p = if npixels > 2 {
                            self.mi.pixel(c as usize)
                        } else {
                            self.mi.white
                        };
                        self.mi.gc.set_foreground(p);
                        d.win()
                            .draw_line(&self.mi.gc, x1 as i32, y1 as i32, x2 as i32, y2 as i32);
                    }
                }
                t += t_inc;
            }
        }

        self.mi.gc.set_line_width(1);

        self.age += 1;
        if self.age > self.mi.cycles {
            self.restart(d);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 1000",
    "*count: 15",
    "*cycles: 100",
    "*size: -7",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "1000").inverted(),
    Opt::slider("cycles", "Duration", 0.0, 500.0, 10.0, 0, "100"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::spin("count", "Number of rings", 3.0, 15.0, "15"),
    Opt::spin("size", "Line thickness", -20.0, 20.0, "-7"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "braid",
    label: "Braid",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "John Neil",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=PUhJq56ViGM"),
        blurb: "Inter-braided concentric circles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
