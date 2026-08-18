//! Port of `hacks/blaster.c`.
//!
//! ```text
//! blaster, Copyright (c) 1999 Jonathan H. Lin <jonlin@tesuji.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//!  Robots that move randomly and shoot lasers at each other. If the
//!  mothership is active, it will fly back and forth horizontally,
//!  firing 8 lasers in the 8 cardinal directions. The explosions are
//!  a 20 frame animation. Robots regenerate after the explosion is finished
//!  and all of its lasers have left the screen.
//! ```
//!
//! Coloured circles drift in front of a scrolling star field and shoot each
//! other. Each robot keeps a target, aims a short segment at it, and that
//! segment then walks itself across the screen a step at a time until it hits
//! something or runs off the edge. A hit starts a twenty-frame explosion, and
//! the robot only comes back once every one of its own lasers has also left the
//! screen, so a kill leaves a gap in the fight.
//!
//! Nothing is ever cleared. Every moving thing is erased by redrawing it in the
//! background colour at the position it had last frame, which is why the robots
//! carry an old position as well as a new one, and why the explosion is a
//! hand-written list of twenty frames alternating between two flame colours and
//! the background.
//!
//! Upstream's own quirks are kept where they are visible. The random walk
//! negates the horizontal speed in the arm that should negate the vertical one,
//! so a robot that tries to change its vertical drift sometimes reverses
//! sideways instead. The aiming code computes one of its two slopes with integer
//! division, which quantises shots at steep angles. And the guard meant to
//! reject an over-long laser tests that both components exceed the whole step
//! length, which cannot happen, so it never rejects anything.
//!
//! The mothership is an upstream option that the XML leaves out, and with it off
//! a third of the file cannot run.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::Pixel;
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XArc, random};

/// How many frames an explosion lasts.
const DEATH_FRAMES: i32 = 20;

#[derive(Clone, Copy, Default)]
struct Laser {
    active: bool,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
}

#[derive(Clone, Default)]
struct Robot {
    alive: bool,
    death: i32,
    /// Nought is the straight-line walk, one the random one.
    move_style: i32,
    target: usize,
    old_x: i32,
    old_y: i32,
    new_x: i32,
    new_y: i32,
    radius: i32,
    robot_color: Pixel,
    laser_color: Pixel,
    lasers: Vec<Laser>,
}

#[derive(Default)]
struct MotherShip {
    /// Doubles as the hit counter: it takes this many lasers to bring down.
    active: i32,
    death: i32,
    old_x: i32,
    new_x: i32,
    y: i32,
    ship_color: Pixel,
    laser_color: Pixel,
    lasers: [Laser; 8],
}

/// Which colour a frame of the explosion animation is drawn in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flame {
    One,
    Two,
    Erase,
}

/// The twenty-frame explosion, as upstream spells it out: which frame draws
/// what colour at which of the three sizes. The frames it does not mention
/// leave the picture alone.
const EXPLOSION: &[(i32, Flame, usize)] = &[
    (20, Flame::One, 0),
    (18, Flame::Two, 0),
    (17, Flame::One, 0),
    (15, Flame::Erase, 0),
    (14, Flame::Two, 0),
    (13, Flame::Erase, 0),
    (12, Flame::One, 0),
    (11, Flame::Erase, 0),
    (10, Flame::Two, 0),
    (9, Flame::Erase, 0),
    (8, Flame::One, 0),
    (7, Flame::Erase, 0),
    (6, Flame::Two, 1),
    (4, Flame::Erase, 1),
    (3, Flame::One, 1),
    (2, Flame::Erase, 1),
];

struct Blaster {
    delay: u32,
    scale: i32,
    num_robots: usize,
    num_lasers: usize,

    mother_ship: bool,
    mother_ship_width: i32,
    mother_ship_height: i32,
    mother_ship_laser: i32,
    mother_ship_period: i32,
    mother_ship_hits: i32,

    explode_size: [i32; 3],
    explode_color: [Pixel; 2],

    stars: Vec<XArc>,
    num_stars: usize,
    move_stars: bool,
    move_stars_x: i32,
    move_stars_y: i32,
    move_stars_random: i32,

    mother: MotherShip,
    robots: Vec<Robot>,

    robot_colors: [Pixel; 6],
    laser_colors: [Pixel; 2],
    star_color: Pixel,
    background: Pixel,

    gc: Gc,
    width: i32,
    height: i32,
    initted: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());
    let scale = if width > 2560 || height > 2560 { 3 } else { 1 };

    let mut delay = d.res.int("delay").max(0) as u32;
    if delay == 0 {
        delay = 10000;
    }
    let mut num_robots = d.res.int("num_robots").max(0) as usize;
    if num_robots == 0 {
        num_robots = 5;
    }
    let num_lasers = d.res.int("num_lasers").max(1) as usize;

    let background = d.res.pixel("background");
    let mut gc = Gc::new(d.res.pixel("foreground"), background);
    gc.set_line_width(scale);

    let mother_ship = d.res.bool("mother_ship");
    let mut st = Blaster {
        delay,
        scale,
        num_robots,
        num_lasers,
        mother_ship,
        mother_ship_width: d.res.int("mother_ship_width") * scale,
        mother_ship_height: d.res.int("mother_ship_height") * scale,
        mother_ship_laser: d.res.int("mother_ship_laser") * scale,
        mother_ship_period: d.res.int("mother_ship_period").max(1),
        mother_ship_hits: d.res.int("mother_ship_hits").max(1),
        explode_size: [
            d.res.int("explode_size_1") * scale,
            d.res.int("explode_size_2") * scale,
            d.res.int("explode_size_3") * scale,
        ],
        explode_color: [
            d.res.pixel("explode_color_1"),
            d.res.pixel("explode_color_2"),
        ],
        stars: Vec::new(),
        num_stars: d.res.int("num_stars").max(0) as usize,
        move_stars: d.res.bool("move_stars"),
        move_stars_x: d.res.int("move_stars_x"),
        move_stars_y: d.res.int("move_stars_y"),
        move_stars_random: d.res.int("move_stars_random"),
        mother: MotherShip {
            ship_color: d.res.pixel("mother_ship_color0"),
            laser_color: d.res.pixel("mother_ship_color1"),
            ..MotherShip::default()
        },
        robots: Vec::new(),
        robot_colors: [
            d.res.pixel("r_color0"),
            d.res.pixel("r_color1"),
            d.res.pixel("r_color2"),
            d.res.pixel("r_color3"),
            d.res.pixel("r_color4"),
            d.res.pixel("r_color5"),
        ],
        laser_colors: [d.res.pixel("l_color0"), d.res.pixel("l_color1")],
        star_color: d.res.pixel("star_color"),
        background,
        gc,
        width,
        height,
        initted: false,
    };
    st.robots = vec![
        Robot {
            lasers: vec![Laser::default(); num_lasers],
            ..Robot::default()
        };
        num_robots
    ];
    st.init_stars();
    Box::new(st)
}

impl Blaster {
    fn init_stars(&mut self) {
        self.stars = (0..self.num_stars)
            .map(|_| {
                let w = ((random() % 4) as i32 + 1) * self.scale;
                XArc {
                    x: (random() % self.width.max(1) as u32) as i32,
                    y: (random() % self.height.max(1) as u32) as i32,
                    width: w,
                    height: w,
                    angle1: 0,
                    angle2: 360 * 64,
                }
            })
            .collect();
    }

    fn rnd(&self, n: i32) -> i32 {
        if n <= 0 {
            0
        } else {
            (random() % n as u32) as i32
        }
    }

    /// A robot comes back on an edge somewhere, with no speed, once none of its
    /// own lasers are still in flight.
    fn make_new_robot(&mut self, index: usize) {
        if self.robots[index].lasers.iter().any(|l| l.active) {
            return;
        }
        self.robots[index].alive = true;
        self.robots[index].radius = (7 + self.rnd(7)) * self.scale;
        self.robots[index].move_style = self.rnd(2);

        let r = self.robots[index].radius;
        if self.rnd(2) == 0 {
            let x = self.rnd(self.width - r);
            self.robots[index].new_x = x;
            self.robots[index].old_x = x;
            let y = if self.rnd(2) == 0 { 0 } else { self.height - r };
            self.robots[index].new_y = y;
            self.robots[index].old_y = y;
        } else {
            let y = self.rnd(self.height - r);
            self.robots[index].new_y = y;
            self.robots[index].old_y = y;
            let x = if self.rnd(2) != 0 { 0 } else { self.width - r };
            self.robots[index].new_x = x;
            self.robots[index].old_x = x;
        }

        self.robots[index].robot_color = self.robot_colors[self.rnd(6) as usize];
        self.robots[index].laser_color = self.laser_colors[if self.rnd(2) == 0 { 0 } else { 1 }];

        if self.num_robots > 1 {
            let mut t = self.rnd(self.num_robots as i32) as usize;
            while t == index {
                t = self.rnd(self.num_robots as i32) as usize;
            }
            self.robots[index].target = t;
        }
    }

    /// Aim a laser from robot `x` at its target, as a short segment that will
    /// walk itself onward frame by frame.
    fn aim_laser(&mut self, x: usize, y: usize) {
        let step = 7 * self.scale;
        let (nx, ny) = (self.robots[x].new_x, self.robots[x].new_y);
        let radius = self.robots[x].radius;
        let target = self.robots[x].target;
        let (tx, ty) = (self.robots[target].new_x, self.robots[target].new_y);

        let l = &mut self.robots[x].lasers[y];
        if tx - nx != 0 {
            let slope = (ty - ny) as f64 / (tx - nx) as f64;
            if slope < 1.0 && slope > -1.0 {
                if tx > nx {
                    l.start_x = radius;
                    l.end_x = l.start_x + step;
                } else {
                    l.start_x = -radius;
                    l.end_x = l.start_x - step;
                }
                l.start_y = (l.start_x as f64 * slope) as i32;
                l.end_y = (l.end_x as f64 * slope) as i32;
            } else {
                // Upstream computes this second slope with integer division,
                // which quantises the steep shots.
                let slope = if ty - ny != 0 {
                    (tx - nx) / (ty - ny)
                } else {
                    0
                };
                if ty > ny {
                    l.start_y = radius;
                    l.end_y = l.start_y + step;
                } else {
                    l.start_y = -radius;
                    l.end_y = l.start_y - step;
                }
                l.start_x = l.start_y * slope;
                l.end_x = l.end_y * slope;
            }
            l.start_x += nx;
            l.start_y += ny;
            l.end_x += nx;
            l.end_y += ny;
        } else if ty > ny {
            l.start_x = nx;
            l.start_y = ny + radius;
            l.end_x = nx;
            l.end_y = l.start_y + step;
        } else {
            l.start_x = nx;
            l.start_y = ny - radius;
            l.end_x = nx;
            l.end_y = l.start_y - step;
        }

        // Upstream's over-long guard asks for both components to exceed the
        // whole step, which cannot happen, so the shot is always taken.
        l.active = true;
    }

    /// Fire a laser off one of the four diagonals, which is what a robot does
    /// on the frame it changes target.
    fn fire_wild(&mut self, x: usize, y: usize) {
        let step = 7 * self.scale;
        let (nx, ny, r) = (
            self.robots[x].new_x,
            self.robots[x].new_y,
            self.robots[x].radius,
        );
        let (sx, sy) = if self.rnd(2) == 0 {
            if self.rnd(2) == 0 { (1, 1) } else { (-1, 1) }
        } else if self.rnd(2) == 0 {
            (-1, -1)
        } else {
            (1, -1)
        };
        let l = &mut self.robots[x].lasers[y];
        l.active = true;
        l.start_x = nx + sx * r;
        l.start_y = ny + sy * r;
        l.end_x = l.start_x + sx * step;
        l.end_y = l.start_y + sy * step;
    }

    fn move_robots(&mut self) {
        for x in 0..self.num_robots {
            if !self.robots[x].alive {
                if self.robots[x].death == 0 {
                    self.make_new_robot(x);
                }
                continue;
            }

            // A robot that has not moved yet is given a shove off the edge.
            if self.robots[x].new_x == self.robots[x].old_x
                && self.robots[x].new_y == self.robots[x].old_y
            {
                self.robots[x].old_x = if self.robots[x].new_x == 0 {
                    -(self.rnd(3) + 1) * self.scale
                } else {
                    self.robots[x].old_x + (self.rnd(3) + 1) * self.scale
                };
                self.robots[x].old_y = if self.robots[x].new_y == 0 {
                    -(self.rnd(3) + 1) * self.scale
                } else {
                    self.robots[x].old_y + (self.rnd(3) + 1) * self.scale
                };
            }

            let mut dx = self.robots[x].new_x - self.robots[x].old_x;
            let mut dy = self.robots[x].new_y - self.robots[x].old_y;
            if self.robots[x].move_style == 0 {
                dx = dx.clamp(-3, 3);
                dy = dy.clamp(-3, 3);
            } else {
                match self.rnd(3) {
                    0 => dx -= self.rnd(7) + 1,
                    1 => dx += self.rnd(7) + 1,
                    _ => dx = -dx,
                }
                dx = dx.clamp(-3, 3);
                match self.rnd(3) {
                    0 => dy -= (self.rnd(7) + 1) * self.scale,
                    1 => dy += (self.rnd(7) + 1) * self.scale,
                    // Upstream negates dx here rather than dy.
                    _ => dx = -dx,
                }
                dy = dy.clamp(-3, 3);
            }
            self.robots[x].old_x = self.robots[x].new_x;
            self.robots[x].old_y = self.robots[x].new_y;
            self.robots[x].new_x += dx * self.scale;
            self.robots[x].new_y += dy * self.scale;

            // Bounds corrections.
            let r = self.robots[x].radius;
            if self.robots[x].new_x >= self.width - r {
                self.robots[x].new_x = self.width - r;
            } else if self.robots[x].new_x < 0 {
                self.robots[x].new_x = 0;
            }
            if self.robots[x].new_y >= self.height - r {
                self.robots[x].new_y = self.height - r;
            } else if self.robots[x].new_y < 0 {
                self.robots[x].new_y = 0;
            }

            self.robots[x].move_style = i32::from(self.rnd(10) == 0);

            if self.num_robots > 1 && self.rnd(2) == 0 {
                if self.rnd(200) == 0 {
                    let mut t = self.rnd(self.num_robots as i32) as usize;
                    while t == x {
                        t = self.rnd(self.num_robots as i32) as usize;
                    }
                    self.robots[x].target = t;
                    for y in 0..self.num_lasers {
                        if !self.robots[x].lasers[y].active {
                            self.fire_wild(x, y);
                            break;
                        }
                    }
                } else {
                    for y in 0..self.num_lasers {
                        if !self.robots[x].lasers[y].active {
                            self.aim_laser(x, y);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Step one laser forward, and see what it runs into on the way.
    fn move_laser(&mut self, d: &mut Dpy, rindex: Option<usize>, index: usize) {
        let laser = |st: &Self| match rindex {
            Some(r) => st.robots[r].lasers[index],
            None => st.mother.lasers[index],
        };
        if !laser(self).active {
            return;
        }

        // Collisions with the other robots.
        for x in 0..self.num_robots {
            if Some(x) == rindex || !self.robots[x].alive {
                continue;
            }
            let l = laser(self);
            let r = self.robots[x].radius;
            let hits = |px: i32, py: i32| {
                (px - self.robots[x].new_x).abs() < r - 1
                    && (py - self.robots[x].new_y).abs() < r - 1
            };
            if hits(l.start_x, l.start_y) || hits(l.end_x, l.end_y) {
                self.robots[x].alive = false;
                self.robots[x].death = DEATH_FRAMES;
                let (ox, oy, nx, ny) = (
                    self.robots[x].old_x,
                    self.robots[x].old_y,
                    self.robots[x].new_x,
                    self.robots[x].new_y,
                );
                self.gc.set_foreground(self.background);
                d.win().fill_arc(&self.gc, ox, oy, r, r, 0, 360 * 64);
                d.win().fill_arc(&self.gc, nx, ny, r, r, 0, 360 * 64);
                match rindex {
                    Some(ri) => self.robots[ri].lasers[index].active = false,
                    None => self.mother.lasers[index].active = false,
                }
                break;
            }
        }

        // A robot's laser can also hit the mothership.
        if self.mother_ship && rindex.is_some() && laser(self).active && self.mother.active > 0 {
            let l = laser(self);
            let hits = |px: i32, py: i32| {
                (py - self.mother.y).abs() < self.mother_ship_height - 1
                    && (px - self.mother.new_x).abs() < self.mother_ship_width - 1
            };
            if hits(l.start_x, l.start_y) || hits(l.end_x, l.end_y) {
                if let Some(ri) = rindex {
                    self.robots[ri].lasers[index].active = false;
                }
                self.mother.active -= 1;
            }
            if self.mother.active == 0 {
                self.mother.death = DEATH_FRAMES;
            }
        }

        if !laser(self).active {
            return;
        }
        // Walk the segment on by its own length.
        let l = laser(self);
        let dx = l.start_x - l.end_x;
        let dy = l.start_y - l.end_y;
        let next = Laser {
            active: !(l.end_x - dx < 0
                || l.end_x - dx >= self.width
                || l.end_y - dy < 0
                || l.end_y - dy >= self.height),
            start_x: l.end_x,
            start_y: l.end_y,
            end_x: l.end_x - dx,
            end_y: l.end_y - dy,
        };
        match rindex {
            Some(ri) => self.robots[ri].lasers[index] = next,
            None => self.mother.lasers[index] = next,
        }
    }

    /// One frame of the twenty-frame explosion.
    fn draw_explosion(&mut self, d: &mut Dpy, death: i32, x: i32, y: i32, big_x: i32, big_y: i32) {
        for &(at, flame, size) in EXPLOSION {
            if at == death {
                let p = match flame {
                    Flame::One => self.explode_color[0],
                    Flame::Two => self.explode_color[1],
                    Flame::Erase => self.background,
                };
                let s = self.explode_size[size];
                self.gc.set_foreground(p);
                d.win().fill_arc(&self.gc, x, y, s, s, 0, 360 * 64);
            }
        }
        // The last flash is a small one, thrown further out.
        if death == 2 {
            let s = self.explode_size[2];
            self.gc.set_foreground(self.explode_color[1]);
            d.win().fill_arc(&self.gc, big_x, big_y, s, s, 0, 360 * 64);
        } else if death == 1 {
            let s = self.explode_size[2];
            self.gc.set_foreground(self.background);
            d.win().fill_arc(&self.gc, big_x, big_y, s, s, 0, 360 * 64);
        }
    }

    fn draw_robots(&mut self, d: &mut Dpy) {
        for x in 0..self.num_robots {
            let r = self.robots[x].radius;
            let (ox, oy, nx, ny) = (
                self.robots[x].old_x,
                self.robots[x].old_y,
                self.robots[x].new_x,
                self.robots[x].new_y,
            );
            self.gc.set_foreground(self.background);
            d.win().fill_arc(&self.gc, ox, oy, r, r, 0, 360 * 64);

            if self.robots[x].alive {
                let p = self.robots[x].robot_color;
                self.gc.set_foreground(p);
                d.win().fill_arc(&self.gc, nx, ny, r, r, 0, 360 * 64);
            } else if self.robots[x].death > 0 {
                let death = self.robots[x].death;
                self.draw_explosion(
                    d,
                    death,
                    nx + r / 3,
                    ny + r / 3,
                    nx + (1.7 * r as f64 / 2.0) as i32,
                    ny + (1.7 * r as f64 / 2.0) as i32,
                );
                self.robots[x].death -= 1;
            }
        }

        for x in 0..self.num_robots {
            for y in 0..self.num_lasers {
                if !self.robots[x].lasers[y].active {
                    continue;
                }
                let l = self.robots[x].lasers[y];
                self.gc.set_foreground(self.background);
                d.win()
                    .draw_line(&self.gc, l.start_x, l.start_y, l.end_x, l.end_y);
                self.move_laser(d, Some(x), y);
                let l = self.robots[x].lasers[y];
                let p = if l.active {
                    self.robots[x].laser_color
                } else {
                    self.background
                };
                self.gc.set_foreground(p);
                d.win()
                    .draw_line(&self.gc, l.start_x, l.start_y, l.end_x, l.end_y);
            }
        }

        if !self.mother_ship {
            return;
        }
        let (mw, mh) = (self.mother_ship_width, self.mother_ship_height);
        self.gc.set_foreground(self.background);
        let (ox, my) = (self.mother.old_x, self.mother.y);
        d.win().fill_arc(&self.gc, ox, my, mw, mh, 0, 360 * 64);
        if self.mother.active > 0 {
            let p = self.mother.ship_color;
            self.gc.set_foreground(p);
            let nx = self.mother.new_x;
            d.win().fill_arc(&self.gc, nx, my, mw, mh, 0, 360 * 64);
        } else if self.mother.death > 0 {
            let (death, nx) = (self.mother.death, self.mother.new_x);
            self.draw_explosion(
                d,
                death,
                nx + 1,
                my + 1,
                nx + (1.7 * mw as f64 / 2.0) as i32,
                my + (1.7 * mh as f64 / 2.0) as i32,
            );
            self.mother.death -= 1;
        }

        for y in 0..8 {
            if !self.mother.lasers[y].active {
                continue;
            }
            let l = self.mother.lasers[y];
            self.gc.set_foreground(self.background);
            d.win()
                .draw_line(&self.gc, l.start_x, l.start_y, l.end_x, l.end_y);
            self.move_laser(d, None, y);
            let l = self.mother.lasers[y];
            let p = if l.active {
                self.mother.laser_color
            } else {
                self.background
            };
            self.gc.set_foreground(p);
            d.win()
                .draw_line(&self.gc, l.start_x, l.start_y, l.end_x, l.end_y);
        }
    }

    fn step_stars(&mut self, d: &mut Dpy) {
        if self.num_stars == 0 {
            return;
        }
        if self.move_stars {
            self.gc.set_foreground(self.background);
            let stars = std::mem::take(&mut self.stars);
            d.win().fill_arcs(&self.gc, &stars);
            self.stars = stars;

            let (mut mx, mut my) = (self.move_stars_x, self.move_stars_y);
            if self.move_stars_random != 0 {
                if self.rnd(167) == 0 {
                    mx = -mx;
                }
                if self.rnd(173) == 0 {
                    my = -my;
                }
                let jitter = |v: &mut i32, limit: i32, up: bool| {
                    if up {
                        *v += 1;
                        *v = (*v).min(limit);
                    } else {
                        *v -= 1;
                        *v = (*v).max(-limit);
                    }
                };
                if self.rnd(50) == 0 {
                    jitter(&mut mx, self.move_stars_random, self.rnd(2) != 0);
                }
                if self.rnd(50) == 0 {
                    jitter(&mut my, self.move_stars_random, self.rnd(2) != 0);
                }
                self.move_stars_x = mx;
                self.move_stars_y = my;
            }
            for s in &mut self.stars {
                s.x += mx;
                s.y += my;
                if s.x < 0 {
                    s.x += self.width;
                } else if s.x > self.width {
                    s.x -= self.width;
                }
                if s.y < 0 {
                    s.y += self.height;
                } else if s.y > self.height {
                    s.y -= self.height;
                }
            }
        }
        self.gc.set_foreground(self.star_color);
        let stars = std::mem::take(&mut self.stars);
        d.win().fill_arcs(&self.gc, &stars);
        self.stars = stars;
    }
}

impl Screenhack for Blaster {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if !self.initted {
            self.initted = true;
            self.init_stars();
        }

        self.step_stars(d);

        if self.mother_ship
            && self.rnd(self.mother_ship_period) == 0
            && self.mother.active == 0
            && self.mother.death == 0
        {
            self.mother.active = self.mother_ship_hits;
            self.mother.y = self.rnd(self.height - 7);
            let x = if self.rnd(2) == 0 { 0 } else { self.width - 25 };
            self.mother.old_x = x;
            self.mother.new_x = x;
        }

        self.move_robots();

        if self.mother_ship && self.mother.active > 0 {
            if self.mother.old_x == self.mother.new_x {
                self.mother.new_x = if self.mother.old_x == 0 {
                    3
                } else {
                    self.mother.new_x - 3
                };
            } else {
                let backwards = self.mother.old_x > self.mother.new_x;
                self.mother.old_x = self.mother.new_x;
                self.mother.new_x += if backwards { -3 } else { 3 };
                if (backwards && self.mother.new_x < 0)
                    || (!backwards && self.mother.new_x > self.width)
                {
                    self.mother.active = 0;
                    let (mw, mh) = (self.mother_ship_width, self.mother_ship_height);
                    self.gc.set_foreground(self.background);
                    let (ox, nx, my) = (self.mother.old_x, self.mother.new_x, self.mother.y);
                    d.win().fill_arc(&self.gc, ox, my, mw, mh, 0, 360 * 64);
                    d.win().fill_arc(&self.gc, nx, my, mw, mh, 0, 360 * 64);
                }
            }

            // Eight lasers at once, in the eight cardinal directions, but only
            // once the last volley has gone.
            if !self.mother.lasers.iter().any(|l| l.active) {
                let sx = self.mother.new_x + self.mother_ship_width / 2;
                let sy = self.mother.y + self.mother_ship_height / 2;
                let big = self.mother_ship_laser;
                let small = (big as f64 / 1.5) as i32;
                let dirs = [
                    (-big, 0),
                    (-small, -small),
                    (0, -big),
                    (small, -small),
                    (big, 0),
                    (small, small),
                    (0, big),
                    (-small, small),
                ];
                for (l, (dx, dy)) in self.mother.lasers.iter_mut().zip(dirs) {
                    l.active = true;
                    l.start_x = sx;
                    l.start_y = sy;
                    l.end_x = sx + dx;
                    l.end_y = sy + dy;
                }
            }
        }

        self.draw_robots(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        d.clear_window();
        self.init_stars();
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*r_color0: #FF00FF",
    "*r_color1: #FFA500",
    "*r_color2: #FFFF00",
    "*r_color3: #FFFFFF",
    "*r_color4: #0000FF",
    "*r_color5: #00FFFF",
    "*l_color0: #00FF00",
    "*l_color1: #FF0000",
    "*mother_ship_color0: #00008B",
    "*mother_ship_color1: #FFFFFF",
    "*explode_color_1: #FFFF00",
    "*explode_color_2: #FFA500",
    "*delay: 10000",
    "*num_robots: 5",
    "*num_lasers: 3",
    "*mother_ship: false",
    "*mother_ship_width: 25",
    "*mother_ship_height: 7",
    "*mother_ship_laser: 15",
    "*mother_ship_period: 150",
    "*mother_ship_hits: 10",
    "*explode_size_1: 27",
    "*explode_size_2: 19",
    "*explode_size_3: 7",
    "*num_stars: 50",
    "*star_color: white",
    "*move_stars: true",
    "*move_stars_x: 2",
    "*move_stars_y: 1",
    "*move_stars_random: 0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::spin("num_robots", "Robots", 2.0, 50.0, "5"),
    Opt::spin("num_lasers", "Lasers", 1.0, 100.0, "3"),
    Opt::slider("num_stars", "Stars", 5.0, 200.0, 5.0, 0, "50"),
    Opt::boolean("mother_ship", "Mothership", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "blaster",
    label: "Blaster",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jonathan Lin",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=bp3J3si2Hr0"),
        blurb: "Flying space-combat robots (cleverly disguised as colored circles) do battle in front of a moving star field.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
