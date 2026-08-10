//! Port of `hacks/attraction.c`.
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
//! Simulation of a pair of quasi-gravitational fields, maybe sorta kinda
//! a little like the strong and weak electromagnetic forces.  Derived from
//! a Lispm screensaver by John Pezaris <pz@mit.edu>.  Viscosity added by
//! Philip Edward Cutone, III <pc2d+@andrew.cmu.edu>.
//!
//! John sez:
//!
//!     The simulation started out as a purely accurate gravitational
//!     simulation, but, with constant simulation step size, I quickly
//!     realized the field being simulated while grossly gravitational
//!     was, in fact, non-conservative.  It also had the rather annoying
//!     behavior of dealing very badly with colliding orbs.  Therefore,
//!     I implemented a negative-gravity region (with two thresholds; as
//!     I read your code, you only implemented one) to prevent orbs from
//!     every coming too close together, and added a viscosity factor if
//!     the speed of any orb got too fast.  This provides a nice stable
//!     system with interesting behavior.
//!
//! And (always the troublemaker) Joe Keane <jgk@jgk.org> sez:
//!
//!     Despite what John sez, the field being simulated is always
//!     conservative.  The real problem is that it uses a simple hack,
//!     computing acceleration *based only on the starting position*,
//!     instead of a real differential equation solver.  Thus you'll
//!     always have energy coming out of nowhere, although it's most
//!     blatant when balls get close together.
//! ```
//!
//! Every ball pulls on every other ball with an inverse-square law, except that
//! inside a threshold distance the sign flips and they shove each other apart
//! instead. That one change is what keeps them from collapsing into each other,
//! and it is the whole reason the thing has any behaviour at all: a plain
//! gravitational field would just clump.
//!
//! The balls need not be drawn as balls. They can be the corners of a polygon,
//! the control points of a closed spline, or the ends of a set of trails, and in
//! those modes what you watch is a shape being pulled around rather than a set
//! of objects. Orbital mode gives every ball the same mass and the tangential
//! speed that balances the field at its starting radius, so they circle instead
//! of falling together.
//!
//! Two knobs here are upstream options rather than XML ones. Glow colours each
//! ball by how hard it is being pulled rather than by identity, and the graph
//! modes draw a bar per ball of its speed along the axis you pick. Both are
//! whole pictures that the settings panel would otherwise have no way to reach.
//! The remaining unexposed options (`maxspeed`, `cbounce`, `vx`, `vy`,
//! `colorShift`) only tune the physics, and are left as resources at upstream's
//! defaults.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{
    Pixel, XColor, make_color_ramp, make_random_colormap, make_smooth_colormap,
};
use crate::runtime::spline::Spline;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent, XPoint,
    frand, random,
};

/// The normal (and max) width for a graph bar.
const BAR_SIZE: i32 = 11;
const MAX_SIZE: i32 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjectMode {
    Ball,
    Line,
    Polygon,
    Spline,
    SplineFilled,
    Tail,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphMode {
    None,
    X,
    Y,
    Both,
    Speed,
}

#[derive(Clone, Copy, Default)]
struct Ball {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    dx: f64,
    dy: f64,
    mass: f64,
    size: i32,
    pixel_index: i32,
}

struct Attraction {
    balls: Vec<Ball>,
    x_vels: Vec<f64>,
    y_vels: Vec<f64>,
    speeds: Vec<f64>,
    npoints: usize,
    threshold: i32,
    delay: u32,
    global_size: i32,
    segments: i32,
    glow_p: bool,
    walls_p: bool,
    maxspeed_p: bool,
    cbounce_p: bool,
    point_stack: Vec<XPoint>,
    point_stack_size: usize,
    point_stack_fp: usize,
    colors: Vec<XColor>,
    ncolors: usize,
    fg_index: usize,
    color_shift: i32,
    xlim: i32,
    ylim: i32,
    /// For the tail-mode fix: do not erase the trail before there is one.
    no_erase_yet: bool,
    viscosity: f64,
    mono: bool,

    mouse_ball: i32,
    mouse_pixel: Pixel,
    mouse_x: i32,
    mouse_y: i32,

    mode: ObjectMode,
    graph_mode: GraphMode,

    draw_gc: Gc,
    erase_gc: Gc,

    total_ticks: i32,
    color_tick: i32,
    spl: Option<Spline>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (xlim, ylim) = (d.width(), d.height());
    let (midx, midy) = (xlim / 2, ylim / 2);
    let walls_p = d.res.bool("walls");

    // Without walls there is no reason to limit the radius.
    let mut r = d.res.int("radius");
    if r <= 0 || (r > (xlim / 2).min(ylim / 2) && walls_p) {
        r = (xlim / 2).min(ylim / 2) - 50;
    }

    let vx = d.res.int("vx");
    let vy = d.res.int("vy");

    let mut npoints = d.res.int("points");
    if npoints < 1 {
        npoints = 3 + (random() % 5) as i32;
    }
    let npoints = npoints as usize;

    let segments = d.res.int("segments").max(0);
    let threshold = d.res.int("threshold").max(0);
    let delay = d.res.int("delay").max(0) as u32;
    let mut global_size = d.res.int("size").max(0);
    let mut glow_p = d.res.bool("glow");
    let mut orbit_p = d.res.bool("orbit");
    let maxspeed_p = d.res.bool("maxspeed");
    let cbounce_p = d.res.bool("cbounce");
    let mut color_shift = d.res.int("colorShift");
    if color_shift <= 0 {
        color_shift = 5;
    }
    let viscosity = d.res.float("viscosity");

    let mut mode = match d.res.string("mode") {
        "lines" => ObjectMode::Line,
        "polygons" => ObjectMode::Polygon,
        "tails" => ObjectMode::Tail,
        "splines" => ObjectMode::Spline,
        "filled-splines" => ObjectMode::SplineFilled,
        _ => ObjectMode::Ball,
    };
    let graph_mode = match d.res.string("graphmode") {
        "x" => GraphMode::X,
        "y" => GraphMode::Y,
        "both" => GraphMode::Both,
        "speed" => GraphMode::Speed,
        _ => GraphMode::None,
    };

    if mode != ObjectMode::Ball && mode != ObjectMode::Tail {
        glow_p = false;
    }
    if mode == ObjectMode::Polygon && npoints < 3 {
        mode = ObjectMode::Line;
    }

    let mut ncolors = d.res.int("colors").max(2) as usize;
    let mut mono = ncolors <= 2;
    let mut colors: Vec<XColor> = Vec::new();
    let mut fg_index = 0;

    if !mono {
        match mode {
            ObjectMode::Ball => {
                if glow_p {
                    let h = (random() % 360) as i32;
                    let v = frand(0.25) + 0.75;
                    colors = make_color_ramp(h, 0.25, v, h, 1.00, v, ncolors, false);
                } else {
                    ncolors = npoints;
                    colors = make_random_colormap(ncolors, true);
                }
            }
            _ => {
                colors = make_smooth_colormap(ncolors);
            }
        }
        ncolors = colors.len();
    }
    if !mono && ncolors <= 2 {
        colors.clear();
        mono = true;
    }
    if mono {
        glow_p = false;
    }

    let mut size_scale = 3.0;
    if xlim < 100 || ylim < 100 {
        size_scale = 0.75; // Tiny windows.
    }
    // Upstream's `rand_size`: let's make the balls bigger by default.
    let rand_size = move || (size_scale * (8 + random() % 7) as f64) as i32;

    if orbit_p && global_size == 0 {
        // To orbit, all objects must be the same mass, or the maths gets
        // really hairy.
        global_size = rand_size();
    }

    let mut balls = vec![Ball::default(); npoints];
    let mut th;
    loop {
        th = frand(std::f64::consts::TAU);
        for (i, b) in balls.iter_mut().enumerate() {
            let new_size = if global_size != 0 {
                global_size
            } else {
                rand_size()
            };
            b.dx = 0.0;
            b.dy = 0.0;
            b.size = new_size;
            b.mass = (new_size * new_size * 10) as f64;
            let a = i as f64 * (std::f64::consts::TAU / npoints as f64) + th;
            b.x = midx as f64 + r as f64 * a.cos();
            b.y = midy as f64 + r as f64 * a.sin();
            if !orbit_p {
                b.vx = if vx != 0 {
                    vx as f64
                } else {
                    (6.0 - (random() % 11) as f64) / 8.0
                };
                b.vy = if vy != 0 {
                    vy as f64
                } else {
                    (6.0 - (random() % 11) as f64) / 8.0
                };
            }
            b.pixel_index = if mono || mode != ObjectMode::Ball {
                -1
            } else if glow_p {
                0
            } else {
                (random() % ncolors as u32) as i32
            };
        }

        // The modes whose points have no size should get the whole window,
        // rather than bouncing early off a size they do not draw.
        if matches!(
            mode,
            ObjectMode::Line | ObjectMode::Spline | ObjectMode::SplineFilled | ObjectMode::Polygon
        ) {
            for b in balls.iter_mut().skip(1) {
                b.size = 0;
            }
        }

        if !orbit_p {
            break;
        }

        // Give every ball the tangential speed that balances the field at this
        // radius, so they circle rather than fall together.
        let mut a = 0.0;
        let mut v_mult = d.res.float("vMult");
        if v_mult == 0.0 {
            v_mult = 1.0;
        }
        for b in balls.iter().take(npoints).skip(1) {
            let i = balls.iter().position(|x| std::ptr::eq(x, b)).unwrap_or(0);
            let _2ipi_n = 2.0 * i as f64 * std::f64::consts::PI / npoints as f64;
            let x = r as f64 * _2ipi_n.cos();
            let y = r as f64 * _2ipi_n.sin();
            let distx = r as f64 - x;
            let dist2 = (distx * distx) + (y * y);
            let dist = dist2.sqrt();
            a += (b.mass / dist2)
                * (if dist < threshold as f64 { -1.0 } else { 1.0 })
                * (distx / dist);
        }
        if a < 0.0 {
            // "domain error: forces on balls too great" -- the window is too
            // small for these orbit settings, so fall back to a free-for-all.
            orbit_p = false;
            continue;
        }
        let v = (a * r as f64).sqrt() * v_mult;
        for (i, b) in balls.iter_mut().enumerate() {
            let k = (2.0 * i as f64 * std::f64::consts::PI / npoints as f64) + th;
            b.vx = -v * k.sin();
            b.vy = v * k.cos();
        }
        break;
    }

    let (point_stack_size, point_stack) = if mode != ObjectMode::Ball {
        let size = (if segments != 0 { segments } else { 1 }) as usize * (npoints + 1);
        (size, vec![XPoint::default(); size])
    } else {
        (0, Vec::new())
    };

    let line_width = if mode == ObjectMode::Tail {
        if global_size != 0 {
            global_size
        } else {
            MAX_SIZE * 2 / 3
        }
    } else {
        1
    };
    let background = d.res.pixel("background");
    let foreground = if mono {
        d.res.pixel("foreground")
    } else {
        colors[fg_index].pixel
    };
    let mut draw_gc = Gc::new(foreground, background);
    draw_gc.set_line_width(line_width);
    let mut erase_gc = Gc::new(background, background);
    erase_gc.set_line_width(line_width);
    fg_index = 0;

    let (x_vels, y_vels, speeds) = (
        if matches!(graph_mode, GraphMode::X | GraphMode::Both) {
            vec![0.0; npoints]
        } else {
            Vec::new()
        },
        if matches!(graph_mode, GraphMode::Y | GraphMode::Both) {
            vec![0.0; npoints]
        } else {
            Vec::new()
        },
        if graph_mode == GraphMode::Speed {
            vec![0.0; npoints]
        } else {
            Vec::new()
        },
    );

    d.clear_window();
    Box::new(Attraction {
        balls,
        x_vels,
        y_vels,
        speeds,
        npoints,
        threshold,
        delay,
        global_size,
        segments,
        glow_p,
        walls_p,
        maxspeed_p,
        cbounce_p,
        point_stack,
        point_stack_size,
        point_stack_fp: 0,
        colors,
        ncolors,
        fg_index,
        color_shift,
        xlim,
        ylim,
        no_erase_yet: true,
        viscosity,
        mono,
        mouse_ball: -1,
        mouse_pixel: d.res.pixel("mouseForeground"),
        mouse_x: -9999,
        mouse_y: -9999,
        mode,
        graph_mode,
        draw_gc,
        erase_gc,
        total_ticks: 0,
        color_tick: 0,
        spl: None,
    })
}

impl Attraction {
    /// The sum of every other ball's pull, which turns into a shove inside the
    /// threshold distance.
    fn compute_force(&self, i: usize) -> (f64, f64) {
        let (mut dx, mut dy) = (0.0, 0.0);
        for j in 0..self.npoints {
            if i == j {
                continue;
            }
            let x_dist = self.balls[j].x - self.balls[i].x;
            let y_dist = self.balls[j].y - self.balls[i].y;
            let dist2 = (x_dist * x_dist) + (y_dist * y_dist);
            let dist = dist2.sqrt();

            if dist > 0.1 {
                // The balls are not overlapping.
                let new_acc = (self.balls[j].mass / dist2)
                    * (if dist < self.threshold as f64 {
                        -1.0
                    } else {
                        1.0
                    });
                let new_acc_dist = new_acc / dist;
                dx += new_acc_dist * x_dist;
                dy += new_acc_dist * y_dist;
            } else {
                // The balls are overlapping; move randomly.
                dx += frand(10.0) - 5.0;
                dy += frand(10.0) - 5.0;
            }
        }
        (dx, dy)
    }

    /// A bar per ball of its speed along x, drawn down the diagonal or the
    /// middle depending on whether the y graph is sharing the screen.
    fn draw_meter_x(&mut self, d: &mut Dpy, i: usize, alone: bool) {
        let (mut y, mut h) = if self.ylim < BAR_SIZE * self.npoints as i32 {
            (
                i as i32 * (self.ylim / self.npoints as i32),
                (self.ylim / self.npoints as i32) - 2,
            )
        } else {
            (BAR_SIZE * i as i32, BAR_SIZE - 2)
        };

        let (mut x1, mut x2) = if alone {
            (self.xlim / 2, self.xlim / 2)
        } else {
            let x = (i as i32 * (h + 2)).max(i as i32);
            (x, x)
        };

        if y < 1 {
            y = i as i32;
        }
        if h < 1 {
            h = 1;
        }

        let mut w1 = (20.0 * self.x_vels[i]) as i32;
        let mut w2 = (20.0 * self.balls[i].vx) as i32;
        self.x_vels[i] = self.balls[i].vx;

        if w1 < 0 {
            w1 = -w1;
            x1 -= w1;
        }
        if w2 < 0 {
            w2 = -w2;
            x2 -= w2;
        }
        d.win()
            .draw_rectangle(&self.erase_gc, x1 + (h + 2) / 2, y, w1, h);
        d.win()
            .draw_rectangle(&self.draw_gc, x2 + (h + 2) / 2, y, w2, h);
    }

    /// The same for y. Upstream asks whether these could be one function
    /// without becoming unreadable, and leaves them as two.
    fn draw_meter_y(&mut self, d: &mut Dpy, i: usize, alone: bool) {
        // Still keyed off the height, as upstream notes.
        let (mut x, mut w) = if self.ylim < BAR_SIZE * self.npoints as i32 {
            (
                i as i32 * (self.ylim / self.npoints as i32),
                (self.ylim / self.npoints as i32) - 2,
            )
        } else {
            (BAR_SIZE * i as i32, BAR_SIZE - 2)
        };

        let (mut y1, mut y2) = if alone {
            (self.ylim / 2, self.ylim / 2)
        } else {
            let y = (i as i32 * (w + 2)).max(i as i32);
            (y, y)
        };

        if x < 1 {
            x = i as i32;
        }
        if w < 1 {
            w = 1;
        }

        let mut h1 = (20.0 * self.y_vels[i]) as i32;
        let mut h2 = (20.0 * self.balls[i].vy) as i32;
        self.y_vels[i] = self.balls[i].vy;

        if h1 < 0 {
            h1 = -h1;
            y1 -= h1;
        }
        if h2 < 0 {
            h2 = -h2;
            y2 -= h2;
        }
        d.win()
            .draw_rectangle(&self.erase_gc, x, y1 + (w + 2) / 2, w, h1);
        d.win()
            .draw_rectangle(&self.draw_gc, x, y2 + (w + 2) / 2, w, h2);
    }

    /// A bar per ball of its total speed, down the left side.
    fn draw_meter_speed(&mut self, d: &mut Dpy, i: usize) {
        let (mut y, mut h) = if self.ylim < BAR_SIZE * self.npoints as i32 {
            (
                i as i32 * (self.ylim / self.npoints as i32),
                (self.ylim / self.npoints as i32) - 2,
            )
        } else {
            (BAR_SIZE * i as i32, BAR_SIZE - 2)
        };
        if y < 1 {
            y = i as i32;
        }
        if h < 1 {
            h = 1;
        }

        let w1 = (5.0 * self.speeds[i]) as i32;
        let sq = self.balls[i].vy * self.balls[i].vy + self.balls[i].vx * self.balls[i].vx;
        let w2 = (5.0 * sq) as i32;
        self.speeds[i] = sq;

        d.win().draw_rectangle(&self.erase_gc, 0, y, w1, h);
        d.win().draw_rectangle(&self.draw_gc, 0, y, w2, h);
    }

    fn bounce(&mut self, i: usize) {
        let size = self.balls[i].size as f64;
        let (xlim, ylim) = (self.xlim as f64, self.ylim as f64);
        if self.cbounce_p {
            // Keep bouncing while it is out of range, up to four times.
            let mut bounce_allowed = 4;
            while bounce_allowed > 0
                && (self.balls[i].x >= xlim - size
                    || self.balls[i].y >= ylim - size
                    || self.balls[i].x <= 0.0
                    || self.balls[i].y <= 0.0)
            {
                bounce_allowed -= 1;
                if self.balls[i].x >= xlim - size {
                    self.balls[i].x = 2.0 * (xlim - size) - self.balls[i].x;
                    self.balls[i].vx = -self.balls[i].vx;
                }
                if self.balls[i].y >= ylim - size {
                    self.balls[i].y = 2.0 * (ylim - size) - self.balls[i].y;
                    self.balls[i].vy = -self.balls[i].vy;
                }
                if self.balls[i].x <= 0.0 {
                    self.balls[i].x = -self.balls[i].x;
                    self.balls[i].vx = -self.balls[i].vx;
                }
                if self.balls[i].y <= 0.0 {
                    self.balls[i].y = -self.balls[i].y;
                    self.balls[i].vy = -self.balls[i].vy;
                }
            }
        } else {
            // The old bouncing.
            if self.balls[i].x >= xlim - size {
                self.balls[i].x = xlim - size - 1.0;
                if self.balls[i].vx > 0.0 {
                    self.balls[i].vx = -self.balls[i].vx;
                }
            }
            if self.balls[i].y >= ylim - size {
                self.balls[i].y = ylim - size - 1.0;
                if self.balls[i].vy > 0.0 {
                    self.balls[i].vy = -self.balls[i].vy;
                }
            }
            if self.balls[i].x <= 0.0 {
                self.balls[i].x = 0.0;
                if self.balls[i].vx < 0.0 {
                    self.balls[i].vx = -self.balls[i].vx;
                }
            }
            if self.balls[i].y <= 0.0 {
                self.balls[i].y = 0.0;
                if self.balls[i].vy < 0.0 {
                    self.balls[i].vy = -self.balls[i].vy;
                }
            }
        }
    }
}

impl Screenhack for Attraction {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let last_point_stack_fp = self.point_stack_fp;
        let radius = if self.global_size == 0 {
            MAX_SIZE / 3
        } else {
            self.global_size / 2
        };

        self.total_ticks += 1;

        match self.graph_mode {
            GraphMode::Both => {
                for i in 0..self.npoints {
                    self.draw_meter_x(d, i, false);
                    self.draw_meter_y(d, i, false);
                }
            }
            GraphMode::X => {
                for i in 0..self.npoints {
                    self.draw_meter_x(d, i, true);
                }
            }
            GraphMode::Y => {
                for i in 0..self.npoints {
                    self.draw_meter_y(d, i, true);
                }
            }
            GraphMode::Speed => {
                for i in 0..self.npoints {
                    self.draw_meter_speed(d, i);
                }
            }
            GraphMode::None => {}
        }

        for i in 0..self.npoints {
            let (dx, dy) = self.compute_force(i);
            self.balls[i].dx = dx;
            self.balls[i].dy = dy;
        }

        for i in 0..self.npoints {
            let old_x = self.balls[i].x;
            let old_y = self.balls[i].y;
            let size = self.balls[i].size;

            self.balls[i].vx += self.balls[i].dx;
            self.balls[i].vy += self.balls[i].dy;

            // Give the medium a viscosity of nine tenths for balls over the
            // speed limit, which is what stops the energy the integrator
            // invents from running away.
            if self.balls[i].vx.abs() > 10.0 && self.maxspeed_p {
                self.balls[i].vx *= 0.9;
                self.balls[i].dx = 0.0;
            }
            if self.viscosity != 1.0 {
                self.balls[i].vx *= self.viscosity;
            }
            if self.balls[i].vy.abs() > 10.0 && self.maxspeed_p {
                self.balls[i].vy *= 0.9;
                self.balls[i].dy = 0.0;
            }
            if self.viscosity != 1.0 {
                self.balls[i].vy *= self.viscosity;
            }

            self.balls[i].x += self.balls[i].vx;
            self.balls[i].y += self.balls[i].vy;

            // A ball is actually its upper left corner.
            if self.walls_p {
                self.bounce(i);
            }

            if i as i32 == self.mouse_ball {
                let (mut x, mut y) = (self.mouse_x, self.mouse_y);
                if self.mode == ObjectMode::Ball {
                    x -= self.balls[i].size / 2;
                    y -= self.balls[i].size / 2;
                }
                self.balls[i].x = x as f64;
                self.balls[i].y = y as f64;
            }

            let new_x = self.balls[i].x;
            let new_y = self.balls[i].y;

            if !self.mono && self.mode == ObjectMode::Ball {
                if self.glow_p {
                    // Colour saturation follows the ball's acceleration.
                    let limit = 0.5;
                    let vx = self.balls[i].dx.abs();
                    let vy = self.balls[i].dy.abs();
                    let fraction = (vx + vy).min(limit);
                    let s = 1.0 - (fraction / limit);
                    self.balls[i].pixel_index = (self.ncolors as f64 * s) as i32;
                }
                let p = if i as i32 == self.mouse_ball {
                    self.mouse_pixel
                } else {
                    let k = (self.balls[i].pixel_index.max(0) as usize).min(self.colors.len() - 1);
                    self.colors[k].pixel
                };
                self.draw_gc.set_foreground(p);
            }

            if self.mode == ObjectMode::Ball {
                d.win().fill_arc(
                    &self.erase_gc,
                    old_x as i32,
                    old_y as i32,
                    size,
                    size,
                    0,
                    360 * 64,
                );
                d.win().fill_arc(
                    &self.draw_gc,
                    new_x as i32,
                    new_y as i32,
                    size,
                    size,
                    0,
                    360 * 64,
                );
            } else {
                self.point_stack[self.point_stack_fp] = XPoint {
                    x: new_x as i32,
                    y: new_y as i32,
                };
                self.point_stack_fp += 1;
            }
        }

        if self.mode == ObjectMode::Ball {
            return self.delay;
        }

        // Close the polygon.
        self.point_stack[self.point_stack_fp] = XPoint {
            x: self.balls[0].x as i32,
            y: self.balls[0].y as i32,
        };
        self.point_stack_fp += 1;
        if self.point_stack_fp == self.point_stack_size {
            self.point_stack_fp = 0;
        }
        if !self.mono {
            self.color_tick += 1;
            if self.color_tick - 1 == self.color_shift {
                self.color_tick = 0;
                self.fg_index = (self.fg_index + 1) % self.ncolors;
                let p = self.colors[self.fg_index].pixel;
                self.draw_gc.set_foreground(p);
            }
        }

        let n = self.npoints + 1;
        let old = &self.point_stack[self.point_stack_fp..self.point_stack_fp + n];
        let new = &self.point_stack[last_point_stack_fp..last_point_stack_fp + n];

        match self.mode {
            ObjectMode::Ball => {}
            ObjectMode::Line => {
                if self.segments > 0 {
                    d.win().draw_lines(&self.erase_gc, old);
                }
                d.win().draw_lines(&self.draw_gc, new);
            }
            ObjectMode::Polygon => {
                if self.segments > 0 {
                    let old = old.to_vec();
                    d.win().fill_polygon(&self.erase_gc, &old);
                }
                let new = new.to_vec();
                d.win().fill_polygon(&self.draw_gc, &new);
            }
            ObjectMode::Tail => {
                for i in 0..self.npoints {
                    let index = self.point_stack_fp + i;
                    let next_index = (index + n) % self.point_stack_size;
                    // Do not erase a trail that is not there yet, which is
                    // what used to draw a line in from the corner.
                    let erase = if self.no_erase_yet {
                        if self.total_ticks >= self.segments {
                            self.no_erase_yet = false;
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    };
                    if erase {
                        let (a, b) = (self.point_stack[index], self.point_stack[next_index]);
                        d.win().draw_line(
                            &self.erase_gc,
                            a.x + radius,
                            a.y + radius,
                            b.x + radius,
                            b.y + radius,
                        );
                    }

                    let index = last_point_stack_fp + i;
                    let next_index = (index + self.point_stack_size - n) % self.point_stack_size;
                    let b = self.point_stack[next_index];
                    if b.x == 0 && b.y == 0 {
                        continue;
                    }
                    let a = self.point_stack[index];
                    d.win().draw_line(
                        &self.draw_gc,
                        a.x + radius,
                        a.y + radius,
                        b.x + radius,
                        b.y + radius,
                    );
                }
            }
            ObjectMode::Spline | ObjectMode::SplineFilled => {
                let mut spl = self.spl.take().unwrap_or_else(|| Spline::new(self.npoints));
                if self.segments > 0 {
                    for i in 0..self.npoints {
                        spl.control_x[i] = self.point_stack[self.point_stack_fp + i].x as f64;
                        spl.control_y[i] = self.point_stack[self.point_stack_fp + i].y as f64;
                    }
                    spl.compute_closed();
                    if self.mode == ObjectMode::SplineFilled {
                        d.win().fill_polygon(&self.erase_gc, &spl.points);
                    } else {
                        d.win().draw_lines(&self.erase_gc, &spl.points);
                    }
                }
                for i in 0..self.npoints {
                    spl.control_x[i] = self.point_stack[last_point_stack_fp + i].x as f64;
                    spl.control_y[i] = self.point_stack[last_point_stack_fp + i].y as f64;
                }
                spl.compute_closed();
                if self.mode == ObjectMode::SplineFilled {
                    d.win().fill_polygon(&self.draw_gc, &spl.points);
                } else {
                    d.win().draw_lines(&self.draw_gc, &spl.points);
                }
                self.spl = Some(spl);
            }
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.xlim = width;
        self.ylim = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match *event {
            XEvent::ButtonPress { x, y, .. } => {
                self.mouse_x = x;
                self.mouse_y = y;
                if self.mouse_ball != -1 {
                    self.mouse_ball = -1; // A second click drops the ball.
                    return true;
                }
                // Look for a click inside a ball, then widen the search until
                // something nearby turns up.
                let max = 10.0
                    * (if self.global_size != 0 {
                        self.global_size
                    } else {
                        MAX_SIZE
                    }) as f64;
                let step = max / 100.0;
                let mut r2 = step;
                while r2 < max {
                    for i in 0..self.npoints {
                        let dx = self.balls[i].x - x as f64;
                        let dy = self.balls[i].y - y as f64;
                        let dist = dx * dx + dy * dy;
                        let r = (self.balls[i].size as f64).max(r2);
                        if dist < r * r {
                            self.mouse_ball = i as i32;
                            return true;
                        }
                    }
                    r2 += step;
                }
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.mouse_ball = -1; // Drop the ball.
                true
            }
            XEvent::MotionNotify { x, y } => {
                self.mouse_x = x;
                self.mouse_y = y;
                false
            }
            _ => false,
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*mode: balls",
    "*graphmode: none",
    "*points: 0",
    "*size: 0",
    "*colors: 200",
    "*threshold: 200",
    "*delay: 10000",
    "*glow: false",
    "*walls: true",
    "*maxspeed: true",
    "*cbounce: true",
    "*viscosity: 1.0",
    "*orbit: false",
    "*colorShift: 3",
    "*segments: 500",
    "*vMult: 0.9",
    "*radius: 0",
    "*vx: 0",
    "*vy: 0",
    "*mouseForeground: white",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "balls",
        label: "Balls",
    },
    SelectItem {
        value: "lines",
        label: "Lines",
    },
    SelectItem {
        value: "tails",
        label: "Tails",
    },
    SelectItem {
        value: "polygons",
        label: "Polygons",
    },
    SelectItem {
        value: "splines",
        label: "Splines",
    },
    SelectItem {
        value: "filled-splines",
        label: "Filled splines",
    },
];

const GRAPH_MODES: &[SelectItem] = &[
    SelectItem {
        value: "none",
        label: "No graph",
    },
    SelectItem {
        value: "x",
        label: "Graph speed along x",
    },
    SelectItem {
        value: "y",
        label: "Graph speed along y",
    },
    SelectItem {
        value: "both",
        label: "Graph both axes",
    },
    SelectItem {
        value: "speed",
        label: "Graph total speed",
    },
];

const OPTS: &[Opt] = &[
    Opt::select("mode", "Shape", MODES, "balls"),
    Opt::boolean("walls", "Bounce off walls", "true"),
    Opt::spin("points", "Ball count", 0.0, 200.0, "0"),
    Opt::slider(
        "viscosity",
        "Environmental viscosity",
        0.0,
        1.0,
        0.05,
        2,
        "1.0",
    )
    .inverted(),
    Opt::slider("segments", "Trail length", 2.0, 1000.0, 10.0, 0, "500"),
    Opt::slider("colors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
    Opt::slider("size", "Ball mass", 0.0, 100.0, 1.0, 0, "0"),
    Opt::slider(
        "threshold",
        "Repulsion threshold",
        0.0,
        600.0,
        10.0,
        0,
        "200",
    ),
    Opt::slider("delay", "Speed", 0.0, 40000.0, 500.0, 0, "10000").inverted(),
    Opt::boolean("orbit", "Orbital mode", "false"),
    Opt::spin("radius", "Radius", 0.0, 1000.0, "0"),
    Opt::slider("vMult", "Orbit speed", -5.0, 5.0, 0.1, 1, "0.9"),
    Opt::boolean("glow", "Colour by acceleration", "false"),
    Opt::select("graphmode", "Speed graph", GRAPH_MODES, "none"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "attraction",
    label: "Attraction",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and John Pezaris",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=KAT9nkXCdms"),
        blurb: "Points attract each other and then repel, similar to the strong and weak nuclear forces.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
