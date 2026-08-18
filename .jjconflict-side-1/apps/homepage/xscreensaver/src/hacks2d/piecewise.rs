//! Port of `hacks/piecewise.c`.
//!
//! ```text
//! piecewise, 21jan2003
//! Geoffrey Irving <irving@caltech.edu>
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
//! Circles drift about, and each one is drawn only in the arcs between the
//! points where other circles cross it, alternating on and off around its
//! circumference. The result reads as though the circles were cut out of each
//! other, except that which side is showing keeps flipping.
//!
//! Finding every crossing is done properly rather than by testing all pairs. A
//! horizontal line sweeps down the screen carrying an ordered list of the arcs
//! it currently cuts, kept in a splay tree so the neighbours of any arc are a
//! rotation away. Two arcs can only cross if they are adjacent in that order, so
//! the only pairs ever tested are the ones that just became neighbours, and each
//! crossing found is queued as an event for the sweep to reach later. That is
//! Bentley-Ottmann, on circles.
//!
//! The C is written in pointers, and casts its fringe and event records to a
//! shared `tree` header so that one splay routine serves both. Here the two live
//! in arenas and the tree links are a parallel array of index pairs, which is
//! the same trick with the same shape: [`Links`] is that header.
//!
//! Degeneracies are not handled. When two arcs start at exactly the same place
//! upstream nudges a circle and starts the sweep again, and when the tree comes
//! out inconsistent it nudges every circle and starts again. Both are here.
//! Upstream will retry forever; this gives up after a few dozen and draws the
//! circles whole for that frame, because a browser tab that stops responding is
//! worse than one wrong frame.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, make_color_loop};
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, frand, random};

/// Half a turn, in the sixty-fourths of a degree that X measures arcs in.
const X_PI: i32 = 180 * 64;

/// `fringe_x`, as a free function so a splay comparison can call it while the
/// tree links are borrowed mutably.
fn fringe_x_of(fringes: &[Fringe], circles: &[Circle], f: usize, y: f64) -> f64 {
    let c = &circles[fringes[f].c];
    let dy = c.y - y;
    let d = ((c.r * c.r) as f64 - dy * dy).sqrt();
    if fringes[f].side { c.x + d } else { c.x - d }
}

/// How many times to nudge and re-sweep before giving up on a frame.
const MAX_RESTARTS: usize = 32;

/// Splay tree links: upstream's `tree` header, which it reaches by casting a
/// fringe or an event to it.
#[derive(Clone, Copy, Default)]
struct Links {
    l: Option<usize>,
    r: Option<usize>,
}

/// Where the next node goes while splaying, which the C keeps as a pointer to a
/// pointer. The left-hand accumulator only ever writes right children and the
/// right-hand one only ever writes left children, so which field is implied.
#[derive(Clone, Copy)]
enum Slot {
    Root,
    Node(usize),
}

fn put_lr(links: &mut [Links], root: &mut Option<usize>, slot: Slot, v: Option<usize>) {
    match slot {
        Slot::Root => *root = v,
        Slot::Node(k) => links[k].r = v,
    }
}

fn put_rl(links: &mut [Links], root: &mut Option<usize>, slot: Slot, v: Option<usize>) {
    match slot {
        Slot::Root => *root = v,
        Slot::Node(k) => links[k].l = v,
    }
}

/// Top-down splay.
///
/// Reference: "Self-adjusting Binary Search Trees", Sleator and Tarjan, JACM
/// Volume 32, No 3, July 1985, pp 652-686. See page 668 for the specific
/// transformations.
///
/// `cut` answers whether the thing being looked for is less than, equal to or
/// greater than the node it is handed.
fn splay(
    links: &mut [Links],
    t: Option<usize>,
    cut: &mut dyn FnMut(usize) -> i32,
) -> Option<usize> {
    let mut x = t?;
    let (mut l, mut r) = (None, None);
    let (mut lr, mut rl) = (Slot::Root, Slot::Root);

    loop {
        let v = cut(x);
        if v == 0 {
            break; // Success.
        } else if v < 0 {
            let Some(y) = links[x].l else { break }; // Trivial.
            let vv = cut(y);
            if vv == 0 {
                put_rl(links, &mut r, rl, Some(x)); // Zig.
                rl = Slot::Node(x);
                x = y;
                break;
            } else if vv < 0 {
                let Some(z) = links[y].l else {
                    put_rl(links, &mut r, rl, Some(x)); // Zig.
                    rl = Slot::Node(x);
                    x = y;
                    break;
                };
                links[x].l = links[y].r; // Zig-zig.
                links[y].r = Some(x);
                put_rl(links, &mut r, rl, Some(y));
                rl = Slot::Node(y);
                x = z;
            } else {
                let Some(z) = links[y].r else {
                    put_rl(links, &mut r, rl, Some(x)); // Zig.
                    rl = Slot::Node(x);
                    x = y;
                    break;
                };
                put_lr(links, &mut l, lr, Some(y)); // Zig-zag.
                lr = Slot::Node(y);
                put_rl(links, &mut r, rl, Some(x));
                rl = Slot::Node(x);
                x = z;
            }
        } else {
            let Some(y) = links[x].r else { break }; // Trivial.
            let vv = cut(y);
            if vv == 0 {
                put_lr(links, &mut l, lr, Some(x)); // Zig.
                lr = Slot::Node(x);
                x = y;
                break;
            } else if vv > 0 {
                let Some(z) = links[y].r else {
                    put_lr(links, &mut l, lr, Some(x)); // Zig.
                    lr = Slot::Node(x);
                    x = y;
                    break;
                };
                links[x].r = links[y].l; // Zig-zig.
                links[y].l = Some(x);
                put_lr(links, &mut l, lr, Some(y));
                lr = Slot::Node(y);
                x = z;
            } else {
                let Some(z) = links[y].l else {
                    put_lr(links, &mut l, lr, Some(x)); // Zig.
                    lr = Slot::Node(x);
                    x = y;
                    break;
                };
                put_rl(links, &mut r, rl, Some(y)); // Zig-zag.
                rl = Slot::Node(y);
                put_lr(links, &mut l, lr, Some(x));
                lr = Slot::Node(x);
                x = z;
            }
        }
    }

    put_lr(links, &mut l, lr, links[x].l);
    links[x].l = l;
    put_rl(links, &mut r, rl, links[x].r);
    links[x].r = r;
    Some(x)
}

/// Rotate the smallest node to the root.
fn splay_min(links: &mut [Links], t: Option<usize>) -> Option<usize> {
    let mut x = t?;
    let mut r = None;
    let mut rl = Slot::Root;

    // Trivial when there is no left child at all.
    while let Some(y) = links[x].l {
        let Some(z) = links[y].l else {
            put_rl(links, &mut r, rl, Some(x)); // Zig.
            rl = Slot::Node(x);
            x = y;
            break;
        };
        links[x].l = links[y].r; // Zig-zig.
        links[y].r = Some(x);
        put_rl(links, &mut r, rl, Some(y));
        rl = Slot::Node(y);
        x = z;
    }

    links[x].l = None;
    put_rl(links, &mut r, rl, links[x].r);
    links[x].r = r;
    Some(x)
}

/// Rotate the largest node to the root.
fn splay_max(links: &mut [Links], t: Option<usize>) -> Option<usize> {
    let mut x = t?;
    let mut l = None;
    let mut lr = Slot::Root;

    // Trivial when there is no right child at all.
    while let Some(y) = links[x].r {
        let Some(z) = links[y].r else {
            put_lr(links, &mut l, lr, Some(x)); // Zig.
            lr = Slot::Node(x);
            x = y;
            break;
        };
        links[x].r = links[y].l; // Zig-zig.
        links[y].l = Some(x);
        put_lr(links, &mut l, lr, Some(y));
        lr = Slot::Node(y);
        x = z;
    }

    put_lr(links, &mut l, lr, links[x].l);
    links[x].l = l;
    links[x].r = None;
    Some(x)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Start,
    Cross,
    Finish,
}

struct Event {
    kind: Kind,
    y: f64,
    x: f64,
    lo: usize,
    hi: usize,
}

#[derive(Default)]
struct Circle {
    r: i32,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    visible: bool,
    /// Where other circles cross this one, as angles, sorted.
    i: Vec<i32>,
}

/// One side of a circle: the left half or the right half, which is what the
/// sweep line actually cuts.
#[derive(Default)]
struct Fringe {
    c: usize,
    /// False for the left side, true for the right.
    side: bool,
    i: Vec<i32>,
}

struct Piecewise {
    count: usize,
    delay: u32,
    colors: Vec<Pixel>,
    color_index: usize,
    color_iterations: i32,
    iterations: i32,
    circles: Vec<Circle>,
    fringes: Vec<Fringe>,
    flinks: Vec<Links>,
    events: Vec<Event>,
    elinks: Vec<Links>,
    erase_gc: Gc,
    draw_gc: Gc,
    width: i32,
    height: i32,
    speed: i32,
    minradius: f64,
    maxradius: f64,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let count = d.res.int("count").max(1) as usize;
    let ncolors = d.res.int("ncolors").max(1) as usize;
    let colorspeed = d.res.int("colorspeed");
    let foreground = d.res.pixel("foreground");
    let background = d.res.pixel("background");

    let mut color_iterations = if colorspeed != 0 {
        100 / colorspeed
    } else {
        100_000
    };
    if color_iterations == 0 {
        color_iterations = 1;
    }

    let mut colors: Vec<Pixel> = if d.res.bool("mono") {
        vec![foreground]
    } else {
        make_color_loop(0, 1.0, 1.0, 120, 1.0, 1.0, 240, 1.0, 1.0, ncolors)
            .iter()
            .map(|c| c.pixel)
            .collect()
    };
    if colors.len() < 2 {
        colors = vec![foreground];
    }

    let mut line_width = 1;
    if d.width() > 2560 || d.height() > 2560 {
        line_width *= 3; // Retina displays.
    }
    let mut erase_gc = Gc::new(background, background);
    erase_gc.set_line_width(line_width);
    let color_index = (random() as usize) % colors.len();
    let mut draw_gc = Gc::new(colors[color_index], background);
    draw_gc.set_line_width(line_width);

    let mut st = Piecewise {
        count,
        delay: d.res.int("delay").max(0) as u32,
        colors,
        color_index,
        color_iterations,
        iterations: 0,
        circles: Vec::new(),
        fringes: Vec::new(),
        flinks: Vec::new(),
        events: Vec::new(),
        elinks: Vec::new(),
        erase_gc,
        draw_gc,
        width: d.width(),
        height: d.height(),
        speed: d.res.int("speed"),
        minradius: d.res.float("minradius"),
        maxradius: d.res.float("maxradius"),
    };
    st.init_circles();
    Box::new(st)
}

impl Piecewise {
    fn init_circles(&mut self) {
        let (w, h) = (self.width, self.height);
        let maxradius = self.maxradius.max(self.minradius);
        let r0 = (self.minradius * h as f64).ceil() as i32;
        let dr = (maxradius * h as f64).floor() as i32 - r0 + 1;

        self.circles = Vec::with_capacity(self.count);
        self.fringes = Vec::with_capacity(self.count * 2);
        for _ in 0..self.count {
            let r = (r0
                + if dr > 0 {
                    (random() % dr as u32) as i32
                } else {
                    0
                })
            .max(1);
            let a = frand(std::f64::consts::TAU);
            let v = (1.0 + frand(0.5)) * self.speed as f64 / 10.0;
            let c = Circle {
                r,
                x: r as f64 + frand((w - 1 - 2 * r) as f64),
                y: r as f64 + frand((h - 1 - 2 * r) as f64),
                dx: v * a.cos(),
                dy: v * a.sin(),
                visible: random() & 1 != 0,
                i: Vec::new(),
            };
            let ci = self.circles.len();
            self.circles.push(c);
            self.fringes.push(Fringe {
                c: ci,
                side: false,
                i: Vec::new(),
            });
            self.fringes.push(Fringe {
                c: ci,
                side: true,
                i: Vec::new(),
            });
        }
        self.flinks = vec![Links::default(); self.fringes.len()];
    }

    /// Upstream's own description of this: "this is a hack, but I guess that's
    /// what I writing anyways". It breaks a tie by moving a circle slightly.
    fn tweak_circle(&mut self, ci: usize) {
        self.circles[ci].x += frand(2.0) - 1.0;
        self.circles[ci].y += frand(1.0) + 0.1;
    }

    fn move_circle(&mut self, ci: usize) {
        let (w, h) = (self.width as f64, self.height as f64);
        let c = &mut self.circles[ci];
        let r = c.r as f64;
        c.x += c.dx;
        if c.x < r {
            c.x = r;
            c.dx = -c.dx;
        } else if c.x >= w - r {
            c.x = w - 1.0 - r;
            c.dx = -c.dx;
        }
        c.y += c.dy;
        if c.y < r {
            c.y = r;
            c.dy = -c.dy;
        } else if c.y >= h - r {
            c.y = h - 1.0 - r;
            c.dy = -c.dy;
        }
    }

    /// Where the sweep line at `y` cuts this side of its circle.
    fn fringe_x(&self, f: usize, y: f64) -> f64 {
        fringe_x_of(&self.fringes, &self.circles, f, y)
    }

    fn fringe_add_intersection(&mut self, f: usize, x: f64, y: f64) {
        let c = &self.circles[self.fringes[f].c];
        let a = ((y - c.y).atan2(x - c.x) * X_PI as f64 / std::f64::consts::PI).round() as i32;
        self.fringes[f].i.push(a);
    }

    fn new_event(&mut self, kind: Kind, x: f64, y: f64, lo: usize, hi: usize) -> usize {
        self.events.push(Event { kind, y, x, lo, hi });
        self.elinks.push(Links::default());
        self.events.len() - 1
    }

    fn event_insert(&mut self, eq: &mut Option<usize>, e: usize) {
        let Some(root) = *eq else {
            self.elinks[e] = Links::default();
            *eq = Some(e);
            return;
        };

        let ey = self.events[e].y;
        let events = &self.events;
        // Splaying a non-empty tree always leaves a root; there is nothing
        // sensible to do if it somehow did not.
        let Some(root) = splay(&mut self.elinks, Some(root), &mut |i| {
            let node_y = events[i].y;
            if ey == node_y {
                0
            } else if ey < node_y {
                -1
            } else {
                1
            }
        }) else {
            return;
        };
        *eq = Some(root);

        if self.events[e].y == self.events[root].y {
            let same = (self.events[e].lo == self.events[root].lo
                && self.events[e].hi == self.events[root].hi)
                || (self.events[e].lo == self.events[root].hi
                    && self.events[e].hi == self.events[root].lo);
            if !same {
                // Upstream notes that doing this rather than dying might be
                // dangerous.
                self.elinks[e].l = self.elinks[root].l;
                self.elinks[e].r = None;
                self.elinks[root].l = Some(e);
            }
            // Otherwise the event is a duplicate and is simply dropped.
        } else if self.events[e].y < self.events[root].y {
            self.elinks[e].l = self.elinks[root].l;
            self.elinks[e].r = Some(root);
            self.elinks[root].l = None;
            *eq = Some(e);
        } else {
            self.elinks[e].l = Some(root);
            self.elinks[e].r = self.elinks[root].r;
            self.elinks[root].r = None;
            *eq = Some(e);
        }
    }

    fn circle_start_event(&mut self, eq: &mut Option<usize>, ci: usize) {
        let c = &self.circles[ci];
        let (x, y) = (c.x, c.y - c.r as f64);
        let e = self.new_event(Kind::Start, x, y, ci * 2, ci * 2 + 1);
        self.event_insert(eq, e);
    }

    fn circle_finish_event(&mut self, eq: &mut Option<usize>, ci: usize) {
        let c = &self.circles[ci];
        let (x, y) = (c.x, c.y + c.r as f64);
        let e = self.new_event(Kind::Finish, x, y, ci * 2, ci * 2 + 1);
        self.event_insert(eq, e);
    }

    fn event_next(&mut self, eq: &mut Option<usize>) -> Option<usize> {
        let root = (*eq)?;
        let e = splay_min(&mut self.elinks, Some(root))?;
        *eq = self.elinks[e].r;
        Some(e)
    }

    /// Queue the crossings of two adjacent arcs, if they cross below the sweep
    /// line and on the right sides of their circles.
    fn fringe_intersect(&mut self, eq: &mut Option<usize>, y: f64, lo: usize, hi: usize) {
        let (lc, hc) = (self.fringes[lo].c, self.fringes[hi].c);
        if lc == hc {
            return;
        }
        let (lo_c, hi_c) = (&self.circles[lc], &self.circles[hc]);

        let dx = hi_c.x - lo_c.x;
        let dy = hi_c.y - lo_c.y;
        let mut sd = dx * dx + dy * dy;
        if sd == 0.0 {
            return;
        }

        let rs = (hi_c.r + lo_c.r) as f64;
        let rd = (hi_c.r - lo_c.r) as f64;
        let d = (rd * rd - sd) * (sd - rs * rs);
        if d <= 0.0 {
            return;
        }

        sd = 0.5 / sd;
        let rp = rs * rd;
        let sqd = d.sqrt();
        let sx = (lo_c.x + hi_c.x) / 2.0;
        let sy = (lo_c.y + hi_c.y) / 2.0;
        let x1 = sx + sd * (dy * sqd - dx * rp);
        let y1 = sy - sd * (dx * sqd + dy * rp);
        let x2 = sx - sd * (dy * sqd + dx * rp);
        let y2 = sy + sd * (dx * sqd - dy * rp);

        let (lo_side, hi_side) = (self.fringes[lo].side, self.fringes[hi].side);
        let (lo_cx, hi_cx) = (lo_c.x, hi_c.x);
        // The crossing has to be below the sweep line, and on the half of each
        // circle that this fringe actually is.
        let check =
            |xi: f64, yi: f64| y <= yi && ((xi < lo_cx) != lo_side) && ((xi < hi_cx) != hi_side);

        let mut add = |st: &mut Self, xi, yi, ilo, ihi| {
            let e = st.new_event(Kind::Cross, xi, yi, ilo, ihi);
            st.event_insert(eq, e);
        };

        if check(x1, y1) {
            if check(x2, y2) {
                if y1 < y2 {
                    add(self, x1, y1, lo, hi);
                    add(self, x2, y2, hi, lo);
                } else {
                    add(self, x1, y1, hi, lo);
                    add(self, x2, y2, lo, hi);
                }
            } else {
                add(self, x1, y1, lo, hi);
            }
        } else if check(x2, y2) {
            add(self, x2, y2, lo, hi);
        }
    }

    fn check_lo(
        &mut self,
        eq: &mut Option<usize>,
        y: f64,
        f: Option<usize>,
        hi: usize,
    ) -> Option<usize> {
        let f = splay_max(&mut self.flinks, f)?;
        self.fringe_intersect(eq, y, f, hi);
        Some(f)
    }

    fn check_hi(
        &mut self,
        eq: &mut Option<usize>,
        y: f64,
        lo: usize,
        f: Option<usize>,
    ) -> Option<usize> {
        let f = splay_min(&mut self.flinks, f)?;
        self.fringe_intersect(eq, y, lo, f);
        Some(f)
    }

    /// Splay the fringe tree by where each arc cuts the sweep line.
    fn splay_fringe_by_x(&mut self, f: Option<usize>, x: f64, y: f64) -> Option<usize> {
        let (fringes, circles) = (&self.fringes, &self.circles);
        splay(&mut self.flinks, f, &mut |i| {
            let sx = fringe_x_of(fringes, circles, i, y);
            if x == sx {
                0
            } else if x < sx {
                -1
            } else {
                1
            }
        })
    }

    /// A circle's topmost point: put both of its arcs into the sweep order.
    fn fringe_start(
        &mut self,
        eq: &mut Option<usize>,
        f: Option<usize>,
        x: f64,
        y: f64,
        lo: usize,
        hi: usize,
    ) -> Option<usize> {
        let Some(f) = f else {
            self.circle_finish_event(eq, self.fringes[lo].c);
            self.flinks[lo].l = None;
            self.flinks[lo].r = Some(hi);
            self.flinks[hi] = Links::default();
            return Some(lo);
        };

        let f = self.splay_fringe_by_x(Some(f), x, y)?;
        let sx = self.fringe_x(f, y);

        if x == sx {
            // Time to cheat my way out of handling degeneracies.
            let ci = self.fringes[lo].c;
            self.tweak_circle(ci);
            self.circle_start_event(eq, ci);
            Some(f)
        } else if x < sx {
            self.circle_finish_event(eq, self.fringes[lo].c);
            let l = self.flinks[f].l;
            self.flinks[f].l = self.check_lo(eq, y, l, lo);
            self.fringe_intersect(eq, y, hi, f);
            self.flinks[lo].l = self.flinks[f].l;
            self.flinks[lo].r = Some(f);
            self.flinks[f].l = Some(hi);
            self.flinks[hi] = Links::default();
            Some(lo)
        } else {
            self.circle_finish_event(eq, self.fringes[lo].c);
            self.fringe_intersect(eq, y, f, lo);
            let r = self.flinks[f].r;
            self.flinks[f].r = self.check_hi(eq, y, hi, r);
            self.flinks[hi].r = self.flinks[f].r;
            self.flinks[hi].l = Some(f);
            self.flinks[f].r = Some(lo);
            self.flinks[lo] = Links::default();
            Some(hi)
        }
    }

    /// Bring two arcs that should be neighbours to the root and its child. A
    /// false answer means the tree is not in the state it should be, which is
    /// what upstream calls a panic.
    fn fringe_double_splay(
        &mut self,
        f: Option<usize>,
        x: f64,
        y: f64,
        lo: usize,
        hi: usize,
    ) -> (Option<usize>, bool) {
        let (fringes, circles) = (&self.fringes, &self.circles);
        let f = splay(&mut self.flinks, f, &mut |i| {
            if i == lo || i == hi {
                return 0;
            }
            let sx = fringe_x_of(fringes, circles, i, y);
            if x == sx {
                0
            } else if x < sx {
                -1
            } else {
                1
            }
        });
        let Some(root) = f else {
            return (None, false);
        };
        if root == lo {
            let rr = self.flinks[root].r;
            let r = splay_min(&mut self.flinks, rr);
            self.flinks[root].r = r;
            (f, r == Some(hi))
        } else if root == hi {
            let ll = self.flinks[root].l;
            let l = splay_max(&mut self.flinks, ll);
            self.flinks[root].l = l;
            (f, l == Some(lo))
        } else {
            (f, false)
        }
    }

    /// Two arcs cross: they swap places in the sweep order, and each gains a new
    /// neighbour to be tested against.
    fn fringe_cross(
        &mut self,
        eq: &mut Option<usize>,
        f: Option<usize>,
        x: f64,
        y: f64,
        lo: usize,
        hi: usize,
    ) -> Option<Option<usize>> {
        let (f, ok) = self.fringe_double_splay(f, x, y, lo, hi);
        if !ok {
            let _ = f;
            return None; // Panic.
        }
        let ll = self.flinks[lo].l;
        let l = self.check_lo(eq, y, ll, hi);
        let hr = self.flinks[hi].r;
        let r = self.check_hi(eq, y, lo, hr);
        self.flinks[lo].l = Some(hi);
        self.flinks[lo].r = r;
        self.flinks[hi].l = l;
        self.flinks[hi].r = None;
        Some(Some(lo))
    }

    /// A circle's bottommost point: both its arcs leave, and whatever was either
    /// side of them becomes adjacent.
    fn fringe_finish(
        &mut self,
        eq: &mut Option<usize>,
        f: Option<usize>,
        x: f64,
        y: f64,
        lo: usize,
        hi: usize,
    ) -> Option<Option<usize>> {
        let (_f, ok) = self.fringe_double_splay(f, x, y, lo, hi);
        if !ok {
            return None; // Panic.
        }
        if self.flinks[lo].l.is_none() {
            Some(self.flinks[hi].r)
        } else if self.flinks[hi].r.is_none() {
            Some(self.flinks[lo].l)
        } else {
            let ll = self.flinks[lo].l;
            let l = splay_max(&mut self.flinks, ll);
            self.flinks[lo].l = l;
            let rr = self.flinks[hi].r;
            let r = splay_min(&mut self.flinks, rr);
            self.flinks[hi].r = r;
            // Both sides were just checked to be occupied, so splaying them
            // leaves a root; anything else means the tree is inconsistent,
            // which is what upstream calls a panic.
            let (Some(l), Some(r)) = (l, r) else {
                return None;
            };
            self.fringe_intersect(eq, y, l, r);
            self.flinks[l].r = Some(r);
            self.flinks[r].l = None;
            Some(Some(l))
        }
    }

    /// Run the sweep line down the screen, collecting every crossing.
    fn sweep(&mut self) {
        for restart in 0..=MAX_RESTARTS {
            self.events.clear();
            self.elinks.clear();
            for f in &mut self.fringes {
                f.i.clear();
            }

            let mut eq: Option<usize> = None;
            for ci in 0..self.count {
                self.circle_start_event(&mut eq, ci);
            }
            let mut f: Option<usize> = None;
            let mut panicked = false;

            while let Some(e) = self.event_next(&mut eq) {
                let (kind, ex, ey, elo, ehi) = {
                    let ev = &self.events[e];
                    (ev.kind, ev.x, ev.y, ev.lo, ev.hi)
                };
                match kind {
                    Kind::Start => {
                        f = self.fringe_start(&mut eq, f, ex, ey, elo, ehi);
                    }
                    Kind::Cross => {
                        match self.fringe_cross(&mut eq, f, ex, ey, elo, ehi) {
                            Some(nf) => f = nf,
                            None => {
                                panicked = true;
                                break;
                            }
                        }
                        self.fringe_add_intersection(elo, ex, ey);
                        self.fringe_add_intersection(ehi, ex, ey);
                    }
                    Kind::Finish => match self.fringe_finish(&mut eq, f, ex, ey, elo, ehi) {
                        Some(nf) => f = nf,
                        None => {
                            panicked = true;
                            break;
                        }
                    },
                }
            }

            if !panicked {
                return;
            }
            if restart == MAX_RESTARTS {
                // Give up on this frame rather than spin: leave every circle
                // uncut, which is what a sweep that found nothing would give.
                for f in &mut self.fringes {
                    f.i.clear();
                }
                return;
            }
            for ci in 0..self.count {
                self.tweak_circle(ci);
            }
        }
    }

    /// Merge this frame's crossings into the circle's list, and work out which
    /// way round the alternation should start.
    fn adjust_circle_visibility(&mut self, ci: usize) {
        let (lo, hi) = (ci * 2, ci * 2 + 1);
        let (lo_ni, hi_ni) = (self.fringes[lo].i.len(), self.fringes[hi].i.len());
        let n = lo_ni + hi_ni;

        let mut inv = vec![0i32; n];
        inv[..hi_ni].copy_from_slice(&self.fringes[hi].i);
        // The low side's angles run the other way round the circle.
        for k in 0..lo_ni {
            let v = self.fringes[lo].i[lo_ni - 1 - k];
            inv[hi_ni + k] = if v > 0 { v } else { v + 2 * X_PI };
        }
        self.fringes[lo].i.clear();
        self.fringes[hi].i.clear();

        // The alternating sum of the merged angle lists: more than half a turn
        // of it means the circle starts on the other side.
        let old = &self.circles[ci].i;
        let (mut i, mut j) = (0usize, 0usize);
        let mut a = 0i32;
        while i < n && j < old.len() {
            a = if inv[i] < old[j] {
                i += 1;
                inv[i - 1]
            } else {
                j += 1;
                old[j - 1]
            } - a;
        }
        while i < n {
            i += 1;
            a = inv[i - 1] - a;
        }
        while j < old.len() {
            j += 1;
            a = old[j - 1] - a;
        }

        if a > X_PI {
            self.circles[ci].visible = !self.circles[ci].visible;
        }
        self.circles[ci].i = inv;
    }

    fn draw_circle(&mut self, d: &mut Dpy, ci: usize) {
        self.adjust_circle_visibility(ci);
        let c = &self.circles[ci];
        let xi = c.x.round() as i32 - c.r;
        let yi = c.y.round() as i32 - c.r;
        let di = c.r << 1;
        let visible = c.visible;
        let angles = c.i.clone();

        let mut arc = |st: &Self, p: usize, a1: i32, a2: i32| {
            let _ = st;
            if (p & 1 != 0) != visible {
                d.win()
                    .draw_arc(&self.draw_gc, xi, yi, di, di, -a1, a1 - a2);
            }
        };

        if angles.is_empty() {
            arc(self, 0, 0, 2 * X_PI);
        } else {
            arc(self, 0, angles[angles.len() - 1], angles[0] + 2 * X_PI);
        }
        for i in 1..angles.len() {
            arc(self, i, angles[i - 1], angles[i]);
        }
    }
}

impl Screenhack for Piecewise {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.erase_gc, 0, 0, w, h);

        self.sweep();
        for ci in 0..self.count {
            self.draw_circle(d, ci);
            self.move_circle(ci);
        }

        self.iterations += 1;
        if self.iterations % self.color_iterations == 0 {
            self.color_index = (self.color_index + 1) % self.colors.len();
            let p = self.colors[self.color_index];
            self.draw_gc.set_foreground(p);
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*speed: 15",
    "*ncolors: 256",
    ".colorspeed: 10",
    ".count: 32",
    ".minradius: 0.05",
    ".maxradius: 0.2",
    "*mono: false",
    "*doubleBuffer: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Count", 4.0, 100.0, 1.0, 0, "32"),
    Opt::slider("colorspeed", "Color shift", 0.0, 100.0, 1.0, 0, "10"),
    Opt::slider("minradius", "Minimum radius", 0.01, 0.5, 0.01, 2, "0.05"),
    Opt::slider("maxradius", "Maximum radius", 0.01, 0.5, 0.01, 2, "0.2"),
    Opt::boolean("mono", "One colour only", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "piecewise",
    label: "Piecewise",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Geoffrey Irving",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=3gQr1FxFSe0"),
        blurb: "Moving circles switch from visibility to invisibility at intersection points.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
