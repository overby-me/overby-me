//! Port of `hacks/bubbles.c`.
//!
//! ```text
//! bubbles.c - frying pan / soft drink in a glass simulation
//!
//!  Copyright (C) 1995-1996 James Macnicol
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! I got my original inspiration for this by looking at the bottom of a
//! frying pan while something was cooking and watching the little bubbles
//! coming off the bottom of the pan as the oil was boiling joining together
//! to form bigger bubbles and finally to *pop* and disappear.  I had some
//! time on my hands so I wrote this little xscreensaver module to imitate
//! it.  Now that it's done it reminds me more of the bubbles you get in
//! a glass of fizzy soft drink.....
//!
//! The problem seemed to be that the position/size etc. of all the bubbles
//! on the screen had to be remembered and searched through to find when
//! bubbles hit each other and combined.  To do this more efficiently, the
//! window/screen is divided up into a square mesh of side length mesh_length
//! and separate lists of bubbles contained in each cell of the mesh are
//! kept.  Only the cells in the immediate vicinity of the bubble in question
//! are searched.  This should make things more efficient although the whole
//! thing seems to use up too much CPU, but then I'm using an ancient PC so
//! perhaps it's not surprising .
//! (Six months after I wrote the above I now have a Pentium with PCI graphics
//! and things are _much_ nicer.)
//!
//! Author:           James Macnicol
//! Internet E-mail : j-macnicol@adfa.edu.au
//! ```
//!
//! Five new bubbles a frame, each dropped at a random point. A bubble that
//! lands touching another eats it, or is eaten: the survivor takes the
//! combined area and moves to the weighted mean of the two positions, and
//! that may put it against something else, so the merge repeats until it
//! settles. Grow past the largest size and it pops.
//!
//! The mesh is the whole reason the file is this long. Every bubble lives in a
//! cell of a square grid a little wider than the biggest bubble, in an
//! intrusive doubly-linked list, so finding what a bubble touches means
//! looking at nine cells rather than at every bubble on the screen. Here that
//! is an arena of indices, because a doubly-linked list of owned nodes is the
//! one shape Rust will not have.
//!
//! The bubbles themselves are ray-traced, eleven sizes of one of four liquids,
//! and drawn through their own outlines. Nothing is drawn round: the shape
//! comes from the picture.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Fb, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent,
    png, random_below,
};
use std::rc::Rc;

/// The most a bubble is shifted in one step when it is rising or falling.
const MAX_DROPPAGE: i32 = 20;

/// One bubble on the screen.
#[derive(Clone, Copy, Default)]
struct Bubble {
    radius: i32,
    /// Which of the rendered sizes this is, in fancy mode.
    step: usize,
    /// Tenths of a pixel, so the arithmetic stays in integers.
    area: i64,
    x: i32,
    y: i32,
    cell_index: usize,
    visible: bool,
    next: Option<usize>,
    prev: Option<usize>,
}

/// One of the rendered sizes: the picture, its outline, and how far a bubble
/// this big moves in one step.
struct Step {
    radius: i32,
    area: i64,
    droppage: i32,
    ball: Option<Pixmap>,
    shape_mask: Option<Rc<Fb>>,
}

struct Bubbles {
    /// The arena. A bubble is only ever reached through the mesh, so a hole
    /// here is a bubble that has popped or been eaten.
    bubbles: Vec<Option<Bubble>>,
    free: Vec<usize>,

    mesh: Vec<Option<usize>>,
    mesh_length: i32,
    mesh_width: i32,
    mesh_height: i32,
    mesh_cells: usize,
    /// The nine cells to search around each cell, itself included.
    adjacent_list: Vec<[i32; 9]>,

    screen_width: i32,
    screen_height: i32,

    /// Simple mode draws circles between these radii instead of pictures.
    bubble_min_radius: i32,
    bubble_max_radius: i32,
    bubble_areas: Vec<i64>,
    bubble_droppages: Vec<i32>,
    draw_gc: Gc,
    erase_gc: Gc,

    /// The rendered sizes, plus one extrapolated beyond the largest so the
    /// biggest bubble hangs about for a moment instead of popping at once.
    step_pixmaps: Vec<Step>,
    num_bubble_pixmaps: usize,

    simple: bool,
    broken: bool,
    three_d: bool,
    drop: bool,
    trails: bool,
    drop_dir: i32,
    delay: u32,
}

impl Bubbles {
    /// The area of a bubble of radius r, in tenths of a pixel. In three
    /// dimensions it is a volume, which makes the big ones far greedier.
    fn calc_bubble_area(&self, r: i32) -> i64 {
        let r = f64::from(r);
        let v = if self.three_d {
            10.0 * std::f64::consts::PI * r * r * r
        } else {
            10.0 * std::f64::consts::PI * r * r
        };
        v as i64
    }

    // ---- The mesh --------------------------------------------------------

    fn cell_to_mesh(&self, x: i32, y: i32) -> usize {
        (self.mesh_width * y + x) as usize
    }

    fn pixel_to_mesh(&self, x: i32, y: i32) -> usize {
        let cx = (x / self.mesh_length).clamp(0, self.mesh_width - 1);
        let cy = (y / self.mesh_length).clamp(0, self.mesh_height - 1);
        self.cell_to_mesh(cx, cy)
    }

    fn verify_mesh_index(&self, x: i32, y: i32) -> i32 {
        if x < 0 || y < 0 || x >= self.mesh_width || y >= self.mesh_height {
            return -1;
        }
        self.cell_to_mesh(x, y) as i32
    }

    fn calculate_adjacent_list(&mut self) {
        self.adjacent_list = Vec::with_capacity(self.mesh_cells);
        for i in 0..self.mesh_cells {
            let (mut ix, mut iy) = (
                (i % self.mesh_width as usize) as i32,
                (i / self.mesh_width as usize) as i32,
            );
            let mut adj = [0i32; 9];
            ix -= 1;
            iy -= 1;
            adj[0] = self.verify_mesh_index(ix, iy);
            ix += 1;
            adj[1] = self.verify_mesh_index(ix, iy);
            ix += 1;
            adj[2] = self.verify_mesh_index(ix, iy);
            iy += 1;
            adj[3] = self.verify_mesh_index(ix, iy);
            iy += 1;
            adj[4] = self.verify_mesh_index(ix, iy);
            ix -= 1;
            adj[5] = self.verify_mesh_index(ix, iy);
            ix -= 1;
            adj[6] = self.verify_mesh_index(ix, iy);
            iy -= 1;
            adj[7] = self.verify_mesh_index(ix, iy);
            adj[8] = i as i32;
            self.adjacent_list.push(adj);
        }
    }

    fn bub(&self, i: usize) -> Bubble {
        self.bubbles[i].unwrap_or_default()
    }

    /// Put a bubble at the head of its cell's list.
    fn add_to_mesh(&mut self, i: usize) {
        let cell = self.bub(i).cell_index;
        let head = self.mesh[cell];
        if let Some(h) = head
            && let Some(b) = &mut self.bubbles[h]
        {
            b.prev = Some(i);
        }
        if let Some(b) = &mut self.bubbles[i] {
            b.next = head;
            b.prev = None;
        }
        self.mesh[cell] = Some(i);
    }

    /// Take a bubble out of its cell's list, and out of the world unless it is
    /// about to be put back in another cell.
    fn delete_bubble_in_mesh(&mut self, i: usize, keep: bool) {
        let b = self.bub(i);
        match (b.prev, b.next) {
            (Some(p), Some(n)) => {
                if let Some(x) = &mut self.bubbles[p] {
                    x.next = Some(n);
                }
                if let Some(x) = &mut self.bubbles[n] {
                    x.prev = Some(p);
                }
            }
            (Some(p), None) => {
                if let Some(x) = &mut self.bubbles[p] {
                    x.next = None;
                }
            }
            (None, Some(n)) => {
                if let Some(x) = &mut self.bubbles[n] {
                    x.prev = None;
                }
                self.mesh[b.cell_index] = Some(n);
            }
            (None, None) => {
                /* Only item on list */
                self.mesh[b.cell_index] = None;
            }
        }
        if !keep {
            self.bubbles[i] = None;
            self.free.push(i);
        }
    }

    // ---- Bubbles ---------------------------------------------------------

    /// A new bubble of the smallest size, somewhere at random.
    fn new_bubble(&mut self) -> usize {
        let (radius, area, step) = if self.simple {
            (
                self.bubble_min_radius,
                self.bubble_areas[self.bubble_min_radius as usize],
                0,
            )
        } else {
            (
                self.step_pixmaps[0].radius,
                self.step_pixmaps[0].area,
                0usize,
            )
        };
        let x = random_below(self.screen_width);
        let y = random_below(self.screen_height);
        let b = Bubble {
            radius,
            step,
            area,
            x,
            y,
            cell_index: self.pixel_to_mesh(x, y),
            visible: false,
            next: None,
            prev: None,
        };
        match self.free.pop() {
            Some(i) => {
                self.bubbles[i] = Some(b);
                i
            }
            None => {
                self.bubbles.push(Some(b));
                self.bubbles.len() - 1
            }
        }
    }

    fn show_bubble(&mut self, d: &mut Dpy, i: usize) {
        let b = self.bub(i);
        if b.visible {
            return;
        }
        if let Some(x) = &mut self.bubbles[i] {
            x.visible = true;
        }

        if self.simple {
            let gc = self.draw_gc.clone();
            d.win().draw_arc(
                &gc,
                b.x - b.radius,
                b.y - b.radius,
                b.radius * 2,
                b.radius * 2,
                0,
                360 * 64,
            );
        } else {
            let step = &self.step_pixmaps[b.step];
            let Some(ball) = &step.ball else { return };
            let mut gc = self.draw_gc.clone();
            if let Some(m) = &step.shape_mask {
                gc.set_clip_mask(Rc::clone(m));
            }
            gc.set_clip_origin(b.x - b.radius, b.y - b.radius);
            d.win().copy_area(
                &gc,
                ball,
                0,
                0,
                b.radius * 2,
                b.radius * 2,
                b.x - b.radius,
                b.y - b.radius,
            );
        }
    }

    fn hide_bubble(&mut self, d: &mut Dpy, i: usize) {
        let b = self.bub(i);
        if !b.visible {
            return;
        }
        if let Some(x) = &mut self.bubbles[i] {
            x.visible = false;
        }

        if self.simple {
            let gc = self.erase_gc.clone();
            d.win().draw_arc(
                &gc,
                b.x - b.radius,
                b.y - b.radius,
                b.radius * 2,
                b.radius * 2,
                0,
                360 * 64,
            );
        } else if !self.broken {
            let mut gc = self.erase_gc.clone();
            if let Some(m) = &self.step_pixmaps[b.step].shape_mask {
                gc.set_clip_mask(Rc::clone(m));
            }
            gc.set_clip_origin(b.x - b.radius, b.y - b.radius);
            d.win().fill_rectangle(
                &gc,
                b.x - b.radius,
                b.y - b.radius,
                b.radius * 2,
                b.radius * 2,
            );
        }
    }

    /// The nearest bubble this one is touching, if any.
    fn get_closest_bubble(&self, i: usize) -> Option<usize> {
        let bb = self.bub(i);
        let mut rv = None;
        let mut closest2 = u64::MAX;
        for k in 0..9 {
            let cell = self.adjacent_list[bb.cell_index][k];
            if cell == -1 {
                continue;
            }
            let mut tmp = self.mesh[cell as usize];
            while let Some(t) = tmp {
                if t != i {
                    let o = self.bub(t);
                    let dx = i64::from(o.x - bb.x);
                    let dy = i64::from(o.y - bb.y);
                    let separation2 = (dx * dx + dy * dy) as u64;
                    // A little extra leeway so circles never overlap.
                    let touch = i64::from(o.radius + bb.radius + 2);
                    let touchdist2 = (touch * touch) as u64;
                    if separation2 <= touchdist2 && separation2 < closest2 {
                        rv = Some(t);
                        closest2 = separation2;
                    }
                }
                tmp = self.bub(t).next;
            }
        }
        rv
    }

    /// The diner eats the food. Returns false when the diner grew past the
    /// largest size and popped.
    fn bubble_eat(&mut self, d: &mut Dpy, diner: usize, food: usize) -> bool {
        // Hide the diner even if it does not grow, so that a bit of the food
        // overlapping its edge is repainted.
        self.hide_bubble(d, diner);
        self.hide_bubble(d, food);

        let (dn, fd) = (self.bub(diner), self.bub(food));
        let x = weighted_mean(dn.x, fd.x, dn.area, fd.area);
        let y = weighted_mean(dn.y, fd.y, dn.area, fd.area);
        let newmi = self.pixel_to_mesh(x, y);
        let area = dn.area + fd.area;
        if let Some(b) = &mut self.bubbles[diner] {
            b.x = x;
            b.y = y;
            b.area = area;
        }
        self.delete_bubble_in_mesh(food, false);

        let biggest = if self.simple {
            self.bubble_areas[self.bubble_max_radius as usize]
        } else {
            self.step_pixmaps[self.num_bubble_pixmaps].area
        };
        if self.bub(diner).area > biggest {
            if self.drop {
                // Rising and falling bubbles do not pop, they just stop
                // growing.
                if let Some(b) = &mut self.bubbles[diner] {
                    b.area = biggest;
                }
            } else {
                self.delete_bubble_in_mesh(diner, false);
                return false;
            }
        }

        // Move up to whatever size the new area calls for.
        let b = self.bub(diner);
        if self.simple {
            if b.area > self.bubble_areas[b.radius as usize + 1] {
                let mut i = b.radius;
                while i < self.bubble_max_radius - 1 && b.area > self.bubble_areas[i as usize + 1] {
                    i += 1;
                }
                if let Some(x) = &mut self.bubbles[diner] {
                    x.radius = i;
                }
            }
        } else if b.area > self.step_pixmaps[b.step + 1].area {
            let mut i = b.step;
            while i < self.num_bubble_pixmaps - 1 && b.area > self.step_pixmaps[i + 1].area {
                i += 1;
            }
            let radius = self.step_pixmaps[i].radius;
            if let Some(x) = &mut self.bubbles[diner] {
                x.step = i;
                x.radius = radius;
            }
        }
        self.show_bubble(d, diner);

        if newmi != self.bub(diner).cell_index {
            self.delete_bubble_in_mesh(diner, true);
            if let Some(x) = &mut self.bubbles[diner] {
                x.cell_index = newmi;
            }
            self.add_to_mesh(diner);
        }

        true
    }

    /// Which of the two survives: the first, the second, or neither.
    fn merge_bubbles(&mut self, d: &mut Dpy, b1: usize, b2: usize) -> Option<usize> {
        if b1 == b2 {
            self.hide_bubble(d, b1);
            self.delete_bubble_in_mesh(b1, false);
            return None;
        }

        let (s1, s2) = (self.bub(b1).area, self.bub(b2).area);
        let (diner, food) = match s1.cmp(&s2) {
            std::cmp::Ordering::Greater => (b1, b2),
            std::cmp::Ordering::Less => (b2, b1),
            // Same size, so toss for it.
            std::cmp::Ordering::Equal if random_below(2) == 0 => (b1, b2),
            std::cmp::Ordering::Equal => (b2, b1),
        };
        if self.bubble_eat(d, diner, food) {
            Some(diner)
        } else {
            None
        }
    }

    /// Merge everything the new bubble lands on, then whatever the survivor
    /// lands on, until it settles or pops.
    fn insert_new_bubble(&mut self, d: &mut Dpy, tmp: usize) {
        let mut nextbub = Some(tmp);
        let mut touch = self.get_closest_bubble(tmp);
        if touch.is_none() {
            return;
        }

        loop {
            /* Merge all touching bubbles */
            while let (Some(n), Some(t)) = (nextbub, touch) {
                nextbub = self.merge_bubbles(d, n, t);
                let Some(n) = nextbub else { break };
                touch = self.get_closest_bubble(n);
            }

            let Some(n) = nextbub else { break };

            /* Shift bubble down. Break if we run off the screen. */
            if self.drop && !self.drop_bubble(d, n) {
                break;
            }

            touch = self.get_closest_bubble(n);
            if touch.is_none() {
                // Every so often keep going anyway, if this one is dropping
                // and is already as big as it gets.
                if self.drop {
                    let b = self.bub(n);
                    let full = if self.simple {
                        b.area >= self.bubble_areas[self.bubble_max_radius as usize - 1]
                    } else {
                        b.step >= self.num_bubble_pixmaps - 1
                    };
                    if full && random_below(2) == 0 {
                        continue;
                    }
                }
                break;
            }
        }
    }

    fn leave_trail(&mut self, d: &mut Dpy, from: usize) {
        let b = self.bub(from);
        let i = self.new_bubble();
        let x = b.x;
        let y = b.y - (b.radius + 10) * self.drop_dir;
        let cell = self.pixel_to_mesh(x, y);
        if let Some(t) = &mut self.bubbles[i] {
            t.x = x;
            t.y = y;
            t.cell_index = cell;
        }
        self.add_to_mesh(i);
        self.insert_new_bubble(d, i);
        if self.bubbles[i].is_some() {
            self.show_bubble(d, i);
        }
    }

    /// Move a bubble one step up or down. Returns false when it has left the
    /// screen and been deleted.
    fn drop_bubble(&mut self, d: &mut Dpy, i: usize) -> bool {
        self.hide_bubble(d, i);

        let b = self.bub(i);
        let step = if self.simple {
            self.bubble_droppages[b.radius as usize]
        } else {
            self.step_pixmaps[b.step].droppage
        };
        let y = b.y + step * self.drop_dir;
        if let Some(x) = &mut self.bubbles[i] {
            x.y = y;
        }
        if y < 0 || y > self.screen_height {
            self.delete_bubble_in_mesh(i, false);
            return false;
        }

        self.show_bubble(d, i);

        let b = self.bub(i);
        let newmi = self.pixel_to_mesh(b.x, b.y);
        if newmi != b.cell_index {
            self.delete_bubble_in_mesh(i, true);
            if let Some(x) = &mut self.bubbles[i] {
                x.cell_index = newmi;
            }
            self.add_to_mesh(i);
        }

        if self.trails {
            let b = self.bub(i);
            let full = if self.simple {
                b.area >= self.bubble_areas[self.bubble_max_radius as usize - 1]
            } else {
                b.step >= self.num_bubble_pixmaps - 1
            };
            if full && random_below(2) == 0 {
                self.leave_trail(d, i);
            }
        }

        true
    }
}

/// Mean of two positions weighted by the two areas, rounding half up.
fn weighted_mean(n1: i32, n2: i32, w1: i64, w2: i64) -> i32 {
    let num = i64::from(n1) * w1 + i64::from(n2) * w2;
    let dem = w1 + w2;
    if dem == 0 {
        return n1;
    }
    let mut divvie = num / dem;
    if num % dem > dem / 2 {
        divvie += 1;
    }
    divvie as i32
}

impl Screenhack for Bubbles {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for _ in 0..5 {
            let i = self.new_bubble();
            self.add_to_mesh(i);
            self.insert_new_bubble(d, i);
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {}

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let simple = d.res.bool("simple");

    let mode = d.res.string("mode").to_string();
    let rise = mode.eq_ignore_ascii_case("rise");
    let fall = mode.eq_ignore_ascii_case("drop");
    let drop_dir = if fall { 1 } else { -1 };

    let mut st = Bubbles {
        bubbles: Vec::new(),
        free: Vec::new(),
        mesh: Vec::new(),
        mesh_length: 1,
        mesh_width: 1,
        mesh_height: 1,
        mesh_cells: 1,
        adjacent_list: Vec::new(),
        screen_width: d.width(),
        screen_height: d.height(),
        bubble_min_radius: 1,
        bubble_max_radius: 2,
        bubble_areas: Vec::new(),
        bubble_droppages: Vec::new(),
        draw_gc: Gc::new(fg, bg),
        erase_gc: Gc::new(bg, bg),
        step_pixmaps: Vec::new(),
        num_bubble_pixmaps: 0,
        simple,
        broken: d.res.bool("broken"),
        three_d: d.res.bool("3D"),
        drop: fall || rise,
        trails: d.res.bool("trails"),
        drop_dir,
        delay: d.res.int("delay").max(0) as u32,
    };

    if st.simple {
        // Radii plucked out of the air upstream.
        let small = st.screen_width.min(st.screen_height);
        st.bubble_min_radius = ((0.006 * f64::from(small)) as i32).max(1);
        st.bubble_max_radius = ((0.045 * f64::from(small)) as i32).max(st.bubble_min_radius + 1);
        st.mesh_length = 2 * st.bubble_max_radius + 3;

        let n = st.bubble_max_radius as usize + 2;
        st.bubble_areas = (0..n as i32)
            .map(|i| {
                if i < st.bubble_min_radius {
                    0
                } else {
                    st.calc_bubble_area(i)
                }
            })
            .collect();
        let span = (st.bubble_max_radius - st.bubble_min_radius).max(1);
        st.bubble_droppages = (0..n as i32)
            .map(|i| {
                if i < st.bubble_min_radius {
                    0
                } else {
                    MAX_DROPPAGE * (i - st.bubble_min_radius) / span
                }
            })
            .collect();
    } else {
        default_to_pixmaps(&mut st);
        st.mesh_length = 2 * st.step_pixmaps[st.num_bubble_pixmaps - 1].radius + 3;
    }

    st.mesh_width = st.screen_width / st.mesh_length + 1;
    st.mesh_height = st.screen_height / st.mesh_length + 1;
    st.mesh_cells = (st.mesh_width * st.mesh_height) as usize;
    st.mesh = vec![None; st.mesh_cells];
    st.calculate_adjacent_list();

    // Upstream also scales every area down here if the products in
    // `weighted_mean` could overflow a long. They are computed in i64, where
    // the factor works out to one for any screen, so there is nothing to do.

    d.clear_window();
    Box::new(st)
}

/// Load one of the four liquids, at random, and sort its eleven sizes.
fn default_to_pixmaps(st: &mut Bubbles) {
    let set = crate::images::BUBBLES[random_below(crate::images::BUBBLES.len() as i32) as usize];
    let mut list: Vec<Step> = Vec::with_capacity(set.len() + 1);
    for bytes in set {
        let (ball, mask) = match png::decode(bytes) {
            Some((img, mask)) => (img, mask),
            None => continue,
        };
        let radius = ball.width().max(ball.height()) / 2;
        let area = st.calc_bubble_area(radius);
        list.push(Step {
            radius,
            area,
            droppage: 0,
            ball: Some(ball),
            shape_mask: mask.map(Rc::new),
        });
    }

    list.sort_by_key(|s| s.radius);
    st.num_bubble_pixmaps = list.len();

    // One more bubble past the ones with pictures, so the largest hangs about
    // for a moment rather than popping the instant it gets there. Its radius
    // is the last two extrapolated.
    let n = st.num_bubble_pixmaps;
    let radius = if n >= 2 {
        list[n - 1].radius + (list[n - 1].radius - list[n - 2].radius)
    } else {
        list.last().map_or(1, |s| s.radius * 2)
    };
    let area = st.calc_bubble_area(radius);
    list.push(Step {
        radius,
        area,
        droppage: 0,
        ball: None,
        shape_mask: None,
    });

    for (i, s) in list.iter_mut().take(n).enumerate() {
        s.droppage = MAX_DROPPAGE * i as i32 / n as i32;
    }
    st.step_pixmaps = list;
}

const DEFAULTS: &[&str] = &[
    ".background:		black",
    ".foreground:		white",
    "*fpsSolid:		true",
    "*simple:		false",
    "*broken:		false",
    "*delay:		10000",
    "*quiet:		false",
    "*mode:		float",
    "*trails:		false",
    "*3D:			false",
];

const GRAVITY: &[SelectItem] = &[
    SelectItem {
        value: "float",
        label: "Bubbles float",
    },
    SelectItem {
        value: "rise",
        label: "Bubbles rise",
    },
    SelectItem {
        value: "drop",
        label: "Bubbles fall",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::select("mode", "Gravity", GRAVITY, "float"),
    Opt::boolean("simple", "Draw circles instead of bubble images", "False"),
    Opt::boolean("broken", "Don't hide bubbles when they pop", "False"),
    Opt::boolean("trails", "Leave trails", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "bubbles",
    label: "Bubbles",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "James Macnicol",
        year: "1996",
        video: Some("https://www.youtube.com/watch?v=Mli1TjZY1YA"),
        blurb: "Small bubbles join into larger ones, which eventually pop.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
