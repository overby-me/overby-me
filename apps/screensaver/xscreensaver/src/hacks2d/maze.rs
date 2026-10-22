//! Port of `hacks/maze.c`.
//!
//! ```text
//! [ maze ] ...
//!
//! modified:  [ 13-08-08 ] Jamie Zawinski <jwz@jwz.org>
//!              Removed the bridge option: it didn't look good, and it made
//!              the code a lot harder to understand.
//!              Made the maze stay out of the area used for the -fps display.
//!              Cleaned up and commented.
//!
//! modified:  [ 1-04-00 ]  Johannes Keukelaar <johannes@nada.kth.se>
//!              Added -ignorant option (not the default) to remove knowlege
//!              of the direction in which the exit lies.
//!
//! modified:  [ 6-28-98 ]  Zack Weinberg <zack@rabi.phys.columbia.edu>
//!              Made the maze-solver somewhat more intelligent.
//!
//! modified:  [ 4-10-97 ]  Johannes Keukelaar <johannes@nada.kth.se>
//!              Added multiple maze creators. Robustified solver.
//!              Added bridge option.
//! modified:  [ 8-11-95 ] Ed James <james@mml.mmc.com>
//!              added fill of dead-end box to solve_maze while loop.
//! modified:  [ 3-7-93 ]  Jamie Zawinski <jwz@jwz.org>
//!              added the XRoger logo, cleaned up resources, made
//!              grid size a parameter.
//! modified:  [ 3-3-93 ]  Jim Randell <jmr@mddjmr.fc.hp.com>
//!              Added the colour stuff and integrated it with jwz's
//!              screenhack stuff.
//! modified:  [ 10-4-88 ]  Richard Hess    ...!uunet!cimshop!rhess
//! modified:  [ 1-29-88 ]  Dave Lemke      lemke@sun.com
//! original:  [ 6/21/85 ]  Martin Weiss    Sun Microsystems  [ SunView ]
//!
//! Copyright 1988 by Sun Microsystems, Inc. Mountain View, CA.
//!
//! All Rights Reserved
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation, and that the names of Sun or MIT not be
//! used in advertising or publicity pertaining to distribution of the
//! software without specific prior written permission. Sun and M.I.T.
//! make no representations about the suitability of this software for
//! any purpose. It is provided "as is" without any express or implied warranty.
//!
//! SUN DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE, INCLUDING
//! ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
//! PURPOSE. IN NO EVENT SHALL SUN BE LIABLE FOR ANY SPECIAL, INDIRECT
//! OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
//! OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE
//! OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE
//! OR PERFORMANCE OF THIS SOFTWARE.
//! ```
//!
//! Forty years of hands on the same file, and it shows: three maze generators
//! that share nothing but the grid, and a solver that has been made cleverer
//! three separate times.
//!
//! The generators are the three classics. Depth-first backtracking walks until
//! it is stuck and then unwinds; Prim's builds walls outward from random
//! corners until every corner is walled; Kruskal's throws every hedge in the
//! grid into a bag, shuffles it, and takes each one down if the squares either
//! side are not yet connected.
//!
//! Solving is a right-hand walk with three cheats bolted on, each visible in
//! its own colour: it will not enter a corridor it can see is a dead end
//! (drawn in the skip colour), it tries the direction the exit lies in first,
//! and the moment it has to backtrack it works out which whole regions can no
//! longer reach the exit and paints them out (the surround colour). Green is
//! where it is going, dark red where it has been and given up.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XEvent, png, random, random_below, screenhack_event_helper,
};
use std::rc::Rc;

const NOT_DEAD: u16 = 0x8000;
const SOLVER_VISIT: u16 = 0x4000;
const START_SQUARE: u16 = 0x2000;
const END_SQUARE: u16 = 0x1000;

const WALL_TOP: u16 = 0x8;
const WALL_RIGHT: u16 = 0x4;
const WALL_BOTTOM: u16 = 0x2;
const WALL_LEFT: u16 = 0x1;
const WALL_ANY: u16 = 0xF;

const DOOR_IN_TOP: u16 = 0x800;
const DOOR_IN_ANY: u16 = 0xF00;
const DOOR_OUT_TOP: u16 = 0x80;

/// One step of the depth-first generator, or of the solver's path.
#[derive(Clone, Copy, Default)]
struct Move {
    x: i32,
    y: i32,
    dir: u16,
    ways: u16,
}

/// Where the solver has got to. Upstream keeps this in its own allocation so
/// that solving can be suspended between frames, which is the whole point:
/// one square a frame is what makes it watchable.
#[derive(Default)]
struct SolveState {
    running: bool,
    i: usize,
    x: i32,
    y: i32,
    bt: bool,
}

struct Maze {
    /// Foreground (the walls), and the four colours the solver draws in.
    gc: Gc,
    /// `cgc`: where it has been and given up.
    dead_gc: Gc,
    /// `tgc`: where it is going now.
    live_gc: Gc,
    /// `sgc`: a corridor it could see was a dead end without entering.
    skip_gc: Gc,
    /// `ugc`: a region that can no longer reach the exit.
    unreachable_gc: Gc,
    logo_gc: Gc,

    logo: Option<Pixmap>,
    /// In grid cells, or `None` when the maze is too small for one.
    logo_cell: Option<(i32, i32)>,
    /// In pixels.
    logo_width: i32,
    logo_height: i32,

    solve_delay: u32,
    pre_solve_delay: u32,
    post_solve_delay: u32,

    maze: Vec<u16>,
    move_list: Vec<Move>,
    path: Vec<Move>,

    maze_size_x: i32,
    maze_size_y: i32,
    sqnum: i32,
    cur_sq_x: i32,
    cur_sq_y: i32,
    start_x: i32,
    start_y: i32,
    start_dir: i32,
    end_x: i32,
    end_y: i32,
    end_dir: i32,
    grid_width: i32,
    grid_height: i32,
    /// Half the wall thickness, which is what the filled squares are inset by.
    bw: i32,

    restart: bool,
    stop: bool,
    state: i32,
    max_length: i32,
    ignorant_p: bool,
    generator: i32,

    solve: SolveState,
    /// The sets, for Kruskal, and the shuffled list of hedges.
    sets: Vec<i32>,
    hedges: Vec<i32>,

    erase_window: bool,
    eraser: Option<Eraser>,

    /// Whether the grid size was left to chance, and whether this is the first
    /// maze since it was.
    ifrandom: bool,
    ifinit: bool,
}

impl Maze {
    /// Upstream's grid is a fixed 1000 by 1000 array and several places index
    /// one cell past the edge of the maze in use, landing on a cell that is
    /// nominally zero. Out of range reads as nothing here, and writes go
    /// nowhere, which is the same thing without the array.
    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.maze_size_x || y >= self.maze_size_y {
            return None;
        }
        Some((x * self.maze_size_y + y) as usize)
    }

    #[inline]
    fn at(&self, x: i32, y: i32) -> u16 {
        self.index(x, y).map_or(0, |i| self.maze[i])
    }

    #[inline]
    fn or(&mut self, x: i32, y: i32, v: u16) {
        if let Some(i) = self.index(x, y) {
            self.maze[i] |= v;
        }
    }

    #[inline]
    fn clear_bits(&mut self, x: i32, y: i32, v: u16) {
        if let Some(i) = self.index(x, y) {
            self.maze[i] &= !v;
        }
    }

    fn set_maze_sizes(&mut self, width: i32, height: i32) {
        self.maze_size_x = (width / self.grid_width).max(4);
        self.maze_size_y = (height / self.grid_height).max(4);
        self.maze = vec![0; (self.maze_size_x * self.maze_size_y) as usize];
        // Both walks are indexed by how far they have got, which cannot exceed
        // the number of squares.
        let cells = (self.maze_size_x * self.maze_size_y) as usize + 1;
        self.move_list = vec![Move::default(); cells];
        self.path = vec![Move::default(); cells];
    }

    fn logo_cells(&self) -> (i32, i32) {
        (
            1 + self.logo_width / self.grid_width,
            1 + self.logo_height / self.grid_height,
        )
    }

    /// Reset the grid, pick the way in and the way out, and find somewhere for
    /// the logo.
    fn initialize_maze(&mut self) {
        let (logow, logoh) = self.logo_cells();
        self.maze.fill(0);

        for i in 0..self.maze_size_x {
            self.or(i, 0, WALL_TOP);
            self.or(i, self.maze_size_y - 1, WALL_BOTTOM);
        }
        for j in 0..self.maze_size_y {
            self.or(self.maze_size_x - 1, j, WALL_RIGHT);
            self.or(0, j, WALL_LEFT);
        }

        // The way in is somewhere along a random edge, and the way out is on
        // the opposite one.
        let wall = random_below(4);
        let (i, j) = self.edge_cell(wall);
        self.or(i, j, START_SQUARE | (DOOR_IN_TOP >> wall));
        self.clear_bits(i, j, WALL_TOP >> wall);
        self.cur_sq_x = i;
        self.cur_sq_y = j;
        self.start_x = i;
        self.start_y = j;
        self.start_dir = wall;
        self.sqnum = 0;

        let wall = (wall + 2) % 4;
        let (i, j) = self.edge_cell(wall);
        self.or(i, j, END_SQUARE | (DOOR_OUT_TOP >> wall));
        self.clear_bits(i, j, WALL_TOP >> wall);
        self.end_x = i;
        self.end_y = j;
        self.end_dir = wall;

        // Not closer than three cells to a wall, so it cannot swallow the
        // entrance or the exit.
        if self.maze_size_x - logow >= 6 && self.maze_size_y - logoh >= 6 {
            let lx = random_below(self.maze_size_x - logow - 5) + 3;
            let ly = random_below(self.maze_size_y - logoh - 5) + 3;
            self.logo_cell = Some((lx, ly));
            for i in 0..logow {
                for j in 0..logoh {
                    // Marked as already having doors, so no generator uses it.
                    self.or(lx + i, ly + j, DOOR_IN_ANY);
                }
            }
        } else {
            self.logo_cell = None;
        }
    }

    /// A random cell along the given edge: 0 top, 1 right, 2 bottom, 3 left.
    fn edge_cell(&self, wall: i32) -> (i32, i32) {
        match wall {
            0 => (random_below(self.maze_size_x), 0),
            1 => (self.maze_size_x - 1, random_below(self.maze_size_y)),
            2 => (random_below(self.maze_size_x), self.maze_size_y - 1),
            _ => (0, random_below(self.maze_size_y)),
        }
    }

    // ---- Generator 0: depth-first recursive backtracker ------------------

    /// Step in a random direction until stuck, then unwind to the last square
    /// with a door left to try.
    fn create_maze(&mut self, d: &mut Dpy) {
        let mut newdoor = 0;
        loop {
            self.move_list[self.sqnum as usize] = Move {
                x: self.cur_sq_x,
                y: self.cur_sq_y,
                dir: newdoor as u16,
                ways: 0,
            };
            loop {
                match self.choose_door(d) {
                    Some(door) => {
                        newdoor = door;
                        break;
                    }
                    None => {
                        // No doors left: back up, and stop if we are home.
                        self.sqnum -= 1;
                        if self.sqnum < 0 {
                            return;
                        }
                        self.cur_sq_x = self.move_list[self.sqnum as usize].x;
                        self.cur_sq_y = self.move_list[self.sqnum as usize].y;
                    }
                }
            }

            self.or(self.cur_sq_x, self.cur_sq_y, DOOR_OUT_TOP >> newdoor);
            match newdoor {
                0 => self.cur_sq_y -= 1,
                1 => self.cur_sq_x += 1,
                2 => self.cur_sq_y += 1,
                _ => self.cur_sq_x -= 1,
            }
            self.sqnum += 1;
            self.or(
                self.cur_sq_x,
                self.cur_sq_y,
                DOOR_IN_TOP >> ((newdoor + 2) % 4),
            );
        }
    }

    /// Which way out of the current square is still open. Walls are built here
    /// as a side effect, against squares that already have a door.
    fn choose_door(&mut self, d: &mut Dpy) -> Option<i32> {
        let mut candidates = [0i32; 4];
        let mut n = 0;
        // Top, right, bottom, left; the neighbour is one step that way.
        const STEPS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        for dir in 0..4 {
            let here = self.at(self.cur_sq_x, self.cur_sq_y);
            let bit = WALL_TOP >> dir;
            if here & (bit | (bit << 8) | (bit << 4)) != 0 {
                // Wall, door in, or door out already on this side.
                continue;
            }
            let (dx, dy) = STEPS[dir as usize];
            if self.at(self.cur_sq_x + dx, self.cur_sq_y + dy) & DOOR_IN_ANY != 0 {
                self.or(self.cur_sq_x, self.cur_sq_y, bit);
                // The neighbour's facing wall is the same bit rotated two
                // places, which is what the shifts below do.
                let facing = ((bit << 2) | (bit >> 2)) & WALL_ANY;
                self.or(self.cur_sq_x + dx, self.cur_sq_y + dy, facing);
                let (x, y) = (self.cur_sq_x, self.cur_sq_y);
                self.draw_wall(d, x, y, dir);
                continue;
            }
            candidates[n] = dir;
            n += 1;
        }
        match n {
            0 => None,
            1 => Some(candidates[0]),
            _ => Some(candidates[random_below(n as i32) as usize]),
        }
    }

    // ---- Generator 1: wall building (Prim) -------------------------------

    /// Pick a random empty corner and a random direction, and draw a wall that
    /// way until it hits something, but only if that would be short.
    fn alt_create_maze(&mut self, d: &mut Dpy) {
        let height = self.maze_size_y + 1;
        let width = self.maze_size_x + 1;
        let mut corners = vec![false; (height * width) as usize];

        let mut c_idx: Vec<i32> = (0..height * width).collect();
        for i in 0..c_idx.len() {
            let k = (random() as usize) % c_idx.len();
            c_idx.swap(i, k);
        }

        for i in 0..width {
            corners[i as usize] = true;
            corners[(i + width * (height - 1)) as usize] = true;
        }
        for i in 0..height {
            corners[(i * width) as usize] = true;
            corners[(i * width + width - 1) as usize] = true;
        }

        if let Some((lx, ly)) = self.logo_cell {
            let (logow, logoh) = self.logo_cells();
            self.alt_mask_out_rect(d, &mut corners, lx, ly, logow, logoh);
        }

        let mut open_corners = corners.iter().filter(|c| !**c).count() as i32;

        while open_corners > 0 {
            for i in 0..(width * height) as usize {
                if corners[c_idx[i] as usize] {
                    continue;
                }
                let x0 = c_idx[i] % width;
                let y0 = c_idx[i] / width;
                let dir = random_below(4);

                // Measure the wall we would draw before committing to it.
                let (mut xx, mut yy) = (x0, y0);
                let mut k = 0;
                while !corners[(xx + width * yy) as usize] {
                    k += 1;
                    match dir {
                        0 => yy -= 1,
                        1 => xx += 1,
                        2 => yy += 1,
                        _ => xx -= 1,
                    }
                }
                if k > self.max_length {
                    continue;
                }

                let (mut xx, mut yy) = (x0, y0);
                while !corners[(xx + width * yy) as usize] {
                    open_corners -= 1;
                    corners[(xx + width * yy) as usize] = true;
                    match dir {
                        0 => {
                            self.build_wall(d, xx - 1, yy - 1, 1);
                            yy -= 1;
                        }
                        1 => {
                            self.build_wall(d, xx, yy, 0);
                            xx += 1;
                        }
                        2 => {
                            self.build_wall(d, xx, yy, 3);
                            yy += 1;
                        }
                        _ => {
                            self.build_wall(d, xx - 1, yy - 1, 2);
                            xx -= 1;
                        }
                    }
                }
            }
        }
    }

    fn alt_mask_out_rect(
        &mut self,
        d: &mut Dpy,
        corners: &mut [bool],
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) {
        let mazew = self.maze_size_x + 1;
        for i in x..=x + w {
            for j in y..=y + h {
                corners[(i + mazew * j) as usize] = true;
            }
        }
        for xx in x..x + w {
            self.build_wall(d, xx, y, 0);
            if y + h < self.maze_size_y {
                self.build_wall(d, xx, y + h, 0);
            }
        }
        for yy in y..y + h {
            if x > 0 {
                self.build_wall(d, x, yy, 3);
            }
            self.build_wall(d, x + w, yy, 3);
        }
    }

    // ---- Generator 2: set joining (Kruskal) ------------------------------

    /// Every square in its own set; every hedge in a bag, shuffled. Take a
    /// hedge down whenever the squares either side are not already connected.
    fn set_create_maze(&mut self, d: &mut Dpy) {
        self.init_sets(d);

        for i in 0..(2 * self.maze_size_x * self.maze_size_y) as usize {
            let h = self.hedges[i];
            if h == -1 {
                // In the logo, or on the outside border.
                continue;
            }
            let dir = if h % 2 != 0 { 1 } else { 2 };
            let xx = (h >> 1) % self.maze_size_x;
            let yy = (h >> 1) / self.maze_size_x;
            let (v, w) = if dir == 1 { (xx + 1, yy) } else { (xx, yy + 1) };

            let a = xx + yy * self.maze_size_x;
            let b = v + w * self.maze_size_x;
            if self.get_set(a) != self.get_set(b) {
                self.join_sets(a, b);
                /* Don't draw the wall. */
            } else {
                self.build_wall(d, xx, yy, dir);
            }
        }

        self.sets = Vec::new();
        self.hedges = Vec::new();
    }

    fn init_sets(&mut self, d: &mut Dpy) {
        let n = self.maze_size_x * self.maze_size_y;
        self.sets = (0..n).collect();
        self.hedges = (0..n * 2).collect();

        for i in 0..self.maze_size_y {
            self.hedges[(2 * (self.maze_size_x * i + self.maze_size_x - 1) + 1) as usize] = -1;
        }
        for i in 0..self.maze_size_x {
            self.hedges[(2 * ((self.maze_size_y - 1) * self.maze_size_x + i)) as usize] = -1;
        }

        if let Some((lx, ly)) = self.logo_cell {
            let (logow, logoh) = self.logo_cells();
            self.mask_out_set_rect(d, lx, ly, logow, logoh);
        }

        for i in 0..self.hedges.len() {
            let r = (random() as usize) % self.hedges.len();
            self.hedges.swap(i, r);
        }
    }

    /// The representative of a set, flattening the chain on the way out.
    fn get_set(&mut self, num: i32) -> i32 {
        if self.sets[num as usize] == num {
            return num;
        }
        let s = self.get_set(self.sets[num as usize]);
        self.sets[num as usize] = s;
        s
    }

    fn join_sets(&mut self, num1: i32, num2: i32) {
        let s1 = self.get_set(num1);
        let s2 = self.get_set(num2);
        if s1 < s2 {
            self.sets[s2 as usize] = s1;
        } else {
            self.sets[s1 as usize] = s2;
        }
    }

    fn mask_out_set_rect(&mut self, d: &mut Dpy, x: i32, y: i32, w: i32, h: i32) {
        for xx in x..x + w {
            for yy in y..y + h {
                let at = (2 * (xx + self.maze_size_x * yy)) as usize;
                self.hedges[at] = -1;
                self.hedges[at + 1] = -1;
            }
        }
        for xx in x..x + w {
            self.build_wall(d, xx, y, 0);
            self.build_wall(d, xx, y + h, 0);
            self.hedges[(2 * (xx + self.maze_size_x * (y - 1))) as usize] = -1;
        }
        for yy in y..y + h {
            self.build_wall(d, x, yy, 3);
            self.build_wall(d, x + w, yy, 3);
            self.hedges[(2 * (x - 1 + self.maze_size_x * yy) + 1) as usize] = -1;
        }
    }

    // ---- Drawing ---------------------------------------------------------

    /// Mark a wall in the grid and draw it.
    fn build_wall(&mut self, d: &mut Dpy, i: i32, j: i32, dir: i32) {
        self.draw_wall(d, i, j, dir);
        match dir {
            0 => {
                self.or(i, j, WALL_TOP);
                if j > 0 {
                    self.or(i, j - 1, WALL_BOTTOM);
                }
            }
            1 => {
                self.or(i, j, WALL_RIGHT);
                if i < self.maze_size_x - 1 {
                    self.or(i + 1, j, WALL_LEFT);
                }
            }
            2 => {
                self.or(i, j, WALL_BOTTOM);
                if j < self.maze_size_y - 1 {
                    self.or(i, j + 1, WALL_TOP);
                }
            }
            _ => {
                self.or(i, j, WALL_LEFT);
                if i > 0 {
                    self.or(i - 1, j, WALL_RIGHT);
                }
            }
        }
    }

    /// One wall, in the foreground colour. Upstream takes a GC here, but every
    /// caller passes the same one.
    fn draw_wall(&self, d: &mut Dpy, i: i32, j: i32, dir: i32) {
        let gc = &self.gc;
        let (gw, gh) = (self.grid_width, self.grid_height);
        let (x1, y1, x2, y2) = match dir {
            0 => (gw * i, gh * j, gw * (i + 1), gh * j),
            1 => (gw * (i + 1), gh * j, gw * (i + 1), gh * (j + 1)),
            2 => (gw * i, gh * (j + 1), gw * (i + 1), gh * (j + 1)),
            _ => (gw * i, gh * j, gw * i, gh * (j + 1)),
        };
        d.win().draw_line(gc, x1, y1, x2, y2);
    }

    /// Fill a square, leaving the wall on one side open so consecutive squares
    /// read as a corridor rather than as separate boxes.
    fn draw_solid_square(&mut self, d: &mut Dpy, i: i32, j: i32, dir: u16, gc: SolveColour) {
        let bw = self.bw;
        let pad = bw + i32::from(bw == 0);
        let (gw, gh) = (self.grid_width, self.grid_height);
        let (x, y, w, h) = match dir {
            WALL_TOP => (pad + gw * i, -pad + gh * j, gw - (bw + pad), gh),
            WALL_RIGHT => (pad + gw * i, pad + gh * j, gw, gh - (bw + pad)),
            WALL_BOTTOM => (pad + gw * i, pad + gh * j, gw - (bw + pad), gh),
            WALL_LEFT => (-pad + gw * i, pad + gh * j, gw, gh - (bw + pad)),
            _ => return,
        };
        let gc = self.solve_gc(gc).clone();
        d.win().fill_rectangle(&gc, x, y, w, h);
    }

    fn solve_gc(&self, which: SolveColour) -> &Gc {
        match which {
            SolveColour::Live => &self.live_gc,
            SolveColour::Dead => &self.dead_gc,
            SolveColour::Skip => &self.skip_gc,
            SolveColour::Unreachable => &self.unreachable_gc,
        }
    }

    /// The outline of the maze, the logo, and the two open squares.
    fn draw_maze_border(&mut self, d: &mut Dpy) {
        let (gw, gh) = (self.grid_width, self.grid_height);
        let gc = self.gc.clone();
        for i in 0..self.maze_size_x {
            if self.at(i, 0) & WALL_TOP != 0 {
                d.win().draw_line(&gc, gw * i, 0, gw * (i + 1) - 1, 0);
            }
            if self.at(i, self.maze_size_y - 1) & WALL_BOTTOM != 0 {
                let y = gh * self.maze_size_y - 1;
                d.win().draw_line(&gc, gw * i, y, gw * (i + 1) - 1, y);
            }
        }
        for j in 0..self.maze_size_y {
            if self.at(self.maze_size_x - 1, j) & WALL_RIGHT != 0 {
                let x = gw * self.maze_size_x - 1;
                d.win().draw_line(&gc, x, gh * j, x, gh * (j + 1) - 1);
            }
            if self.at(0, j) & WALL_LEFT != 0 {
                d.win().draw_line(&gc, 0, gh * j, 0, gh * (j + 1) - 1);
            }
        }

        if let (Some((lx, ly)), Some(logo)) = (self.logo_cell, &self.logo) {
            let (w, h) = (logo.width(), logo.height());
            // Round the hole up to whole cells. Upstream's kludge: if the hole
            // is about the size of the logo, do not centre it, because the
            // logo is a little off centre itself; do centre it when the hole is
            // much bigger.
            let mut ww = (self.logo_width / gw + 1) * gw;
            let mut hh = (self.logo_height / gh + 1) * gh;
            if ww < self.logo_width + 5 {
                ww = w;
            }
            if hh < self.logo_height + 5 {
                hh = h;
            }
            let lx_px = 3 + gw * lx + (ww - w) / 2;
            let ly_px = 3 + gh * ly + (hh - h) / 2;

            let ugc = self.unreachable_gc.clone();
            d.win()
                .fill_rectangle(&ugc, 3 + gw * lx, 3 + gh * ly, ww, hh);

            let mut gc = self.logo_gc.clone();
            gc.set_clip_origin(lx_px, ly_px);
            d.win().copy_area(&gc, logo, 0, 0, w, h, lx_px, ly_px);
        }

        let (sx, sy, sd) = (self.start_x, self.start_y, self.start_dir);
        self.draw_solid_square(d, sx, sy, WALL_TOP >> sd, SolveColour::Live);
        let (ex, ey, ed) = (self.end_x, self.end_y, self.end_dir);
        self.draw_solid_square(d, ex, ey, WALL_TOP >> ed, SolveColour::Live);
    }

    // ---- Solving ---------------------------------------------------------

    /// Is the corridor beyond this square a dead end all the way down? If so,
    /// paint it out without walking it.
    fn longdeadend_p(
        &mut self,
        d: &mut Dpy,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        endwall: u16,
    ) -> bool {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let (mut x2, mut y2) = (x2, y2);
        let sidewalls = !(endwall | (endwall >> 2 | endwall << 2)) & WALL_ANY;

        while self.at(x2, y2) & WALL_ANY == sidewalls {
            if x2 + dx < 0
                || x2 + dx >= self.maze_size_x
                || y2 + dy < 0
                || y2 + dy >= self.maze_size_y
            {
                break;
            }
            x2 += dx;
            y2 += dy;
        }

        if self.at(x2, y2) & WALL_ANY == (sidewalls | endwall) {
            let endwall = (endwall >> 2 | endwall << 2) & WALL_ANY;
            let (mut x1, mut y1) = (x1, y1);
            while x1 != x2 || y1 != y2 {
                x1 += dx;
                y1 += dy;
                self.draw_solid_square(d, x1, y1, endwall, SolveColour::Skip);
                self.or(x1, y1, SOLVER_VISIT);
            }
            true
        } else {
            false
        }
    }

    /// Everything that can no longer reach the exit without crossing ground
    /// already covered. Flood outward from the start, then paint out whatever
    /// the flood did not touch.
    fn find_dead_regions(&mut self, d: &mut Dpy) {
        self.or(self.start_x, self.start_y, NOT_DEAD);

        loop {
            let mut flipped = false;
            for xx in 0..self.maze_size_x {
                for yy in 0..self.maze_size_y {
                    if self.at(xx, yy) & (SOLVER_VISIT | NOT_DEAD) == 0
                        && ((xx != 0 && self.at(xx - 1, yy) & NOT_DEAD != 0)
                            || (yy != 0 && self.at(xx, yy - 1) & NOT_DEAD != 0))
                    {
                        flipped = true;
                        self.or(xx, yy, NOT_DEAD);
                    }
                }
            }
            for xx in (0..self.maze_size_x).rev() {
                for yy in (0..self.maze_size_y).rev() {
                    if self.at(xx, yy) & (SOLVER_VISIT | NOT_DEAD) == 0
                        && ((xx != self.maze_size_x - 1 && self.at(xx + 1, yy) & NOT_DEAD != 0)
                            || (yy != self.maze_size_y - 1 && self.at(xx, yy + 1) & NOT_DEAD != 0))
                    {
                        flipped = true;
                        self.or(xx, yy, NOT_DEAD);
                    }
                }
            }
            if !flipped {
                break;
            }
        }

        let (logo_x, logo_y) = self.logo_cell.unwrap_or((-1, -1));
        for yy in 0..self.maze_size_y {
            for xx in 0..self.maze_size_x {
                if self.at(xx, yy) & NOT_DEAD != 0 {
                    self.clear_bits(xx, yy, NOT_DEAD);
                } else if self.at(xx, yy) & SOLVER_VISIT == 0 {
                    self.or(xx, yy, SOLVER_VISIT);
                    let inside_logo = xx >= logo_x
                        && xx <= logo_x + self.logo_width / self.grid_width
                        && yy >= logo_y
                        && yy <= logo_y + self.logo_height / self.grid_height;
                    if inside_logo {
                        continue;
                    }
                    if self.at(xx, yy) & WALL_ANY == WALL_ANY {
                        // Boxed in on all four sides: just the inside.
                        let (bw, gw, gh) = (self.bw, self.grid_width, self.grid_height);
                        let gc = self.unreachable_gc.clone();
                        d.win().fill_rectangle(
                            &gc,
                            bw + gw * xx,
                            bw + gh * yy,
                            gw - bw - bw,
                            gh - bw - bw,
                        );
                    } else {
                        for side in [WALL_LEFT, WALL_RIGHT, WALL_TOP, WALL_BOTTOM] {
                            if self.at(xx, yy) & side == 0 {
                                self.draw_solid_square(d, xx, yy, side, SolveColour::Unreachable);
                            }
                        }
                    }
                }
            }
        }
    }

    /// One square of the search. Returns true when the way home has been found
    /// or the maze turns out to be unsolvable.
    fn solve_maze(&mut self, d: &mut Dpy) -> bool {
        if !self.solve.running {
            /* plug up the surrounding wall */
            self.or(self.end_x, self.end_y, WALL_TOP >> self.end_dir);
            self.solve.i = 0;
            self.path[0] = Move {
                x: self.end_x,
                y: self.end_y,
                dir: 0,
                ways: 0,
            };
            self.or(self.end_x, self.end_y, SOLVER_VISIT);
            self.solve.running = true;
        }

        let i = self.solve.i;
        if self.at(self.path[i].x, self.path[i].y) & START_SQUARE != 0 {
            self.solve.running = false;
            return true;
        }

        let mut ways;
        if self.path[i].dir == 0 {
            ways = 0;
            // First visit: which of the four sides are open, and of those,
            // which lead somewhere worth going?
            let mut dir = WALL_TOP;
            while dir & WALL_ANY != 0 {
                if self.at(self.path[i].x, self.path[i].y) & dir == 0 {
                    let y = self.path[i].y - i32::from(dir & WALL_TOP != 0)
                        + i32::from(dir & WALL_BOTTOM != 0);
                    let x = self.path[i].x + i32::from(dir & WALL_RIGHT != 0)
                        - i32::from(dir & WALL_LEFT != 0);
                    self.solve.x = x;
                    self.solve.y = y;

                    if self.at(x, y) & SOLVER_VISIT == 0 {
                        let from = (dir << 2 & WALL_ANY) | (dir >> 2 & WALL_ANY);
                        if (self.at(x, y) & WALL_ANY) | from != WALL_ANY {
                            let (px, py) = (self.path[i].x, self.path[i].y);
                            if !self.longdeadend_p(d, px, py, x, y, dir) {
                                ways |= dir;
                            }
                        } else {
                            self.draw_solid_square(d, x, y, from, SolveColour::Skip);
                            self.or(x, y, SOLVER_VISIT);
                        }
                    }
                }
                dir >>= 1;
            }
        } else {
            ways = self.path[i].ways;
        }

        let dir = if ways == 0 {
            None
        } else if !self.ignorant_p {
            // Try the direction the exit lies in first, then the other axis,
            // then the reverse, then anything at all.
            let dx = self.path[i].x - self.start_x;
            let dy = self.path[i].y - self.start_y;
            let mut dir = if dy.abs() <= dx.abs() {
                if dx > 0 { WALL_LEFT } else { WALL_RIGHT }
            } else if dy > 0 {
                WALL_TOP
            } else {
                WALL_BOTTOM
            };
            if dir & ways == 0 {
                dir = match dir {
                    WALL_LEFT | WALL_RIGHT => {
                        if dy > 0 {
                            WALL_TOP
                        } else {
                            WALL_BOTTOM
                        }
                    }
                    _ => {
                        if dx > 0 {
                            WALL_LEFT
                        } else {
                            WALL_RIGHT
                        }
                    }
                };
            }
            if dir & ways == 0 {
                dir = (dir << 2 & WALL_ANY) | (dir >> 2 & WALL_ANY);
            }
            if dir & ways == 0 {
                dir = ways;
            }
            Some(dir)
        } else if ways & WALL_TOP != 0 {
            Some(WALL_TOP)
        } else if ways & WALL_LEFT != 0 {
            Some(WALL_LEFT)
        } else if ways & WALL_BOTTOM != 0 {
            Some(WALL_BOTTOM)
        } else if ways & WALL_RIGHT != 0 {
            Some(WALL_RIGHT)
        } else {
            None
        };

        let Some(dir) = dir else {
            return self.backtrack(d);
        };

        self.solve.bt = false;
        ways &= !dir; /* tried this one */

        let y = self.path[i].y - i32::from(dir & WALL_TOP != 0) + i32::from(dir & WALL_BOTTOM != 0);
        let x = self.path[i].x + i32::from(dir & WALL_RIGHT != 0) - i32::from(dir & WALL_LEFT != 0);

        self.path[i].dir = dir;
        self.path[i].ways = ways;
        let (px, py) = (self.path[i].x, self.path[i].y);
        self.draw_solid_square(d, px, py, dir, SolveColour::Live);

        self.solve.i += 1;
        self.path[self.solve.i] = Move {
            x,
            y,
            dir: 0,
            ways: 0,
        };
        self.or(x, y, SOLVER_VISIT);
        false
    }

    /// Give up on this square and step back, painting where we have been.
    fn backtrack(&mut self, d: &mut Dpy) -> bool {
        let i = self.solve.i;
        if i == 0 {
            /* Unsolvable maze. */
            self.solve.running = false;
            return true;
        }
        if !self.solve.bt && !self.ignorant_p {
            self.find_dead_regions(d);
        }
        self.solve.bt = true;
        let from = self.path[i - 1].dir;
        let from = (from << 2 & WALL_ANY) | (from >> 2 & WALL_ANY);
        let (px, py) = (self.path[i].x, self.path[i].y);
        self.draw_solid_square(d, px, py, from, SolveColour::Dead);
        self.solve.i -= 1;
        false
    }
}

/// Which of the solver's four colours to fill a square with.
#[derive(Clone, Copy)]
enum SolveColour {
    Live,
    Dead,
    Skip,
    Unreachable,
}

impl Screenhack for Maze {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = self.solve_delay;

        if self.eraser.is_some() || self.erase_window {
            self.erase_window = false;
            self.eraser = erase_window(d, self.eraser.take());
            if self.eraser.is_some() {
                return 10_000;
            }
            return 1_000_000.min(self.pre_solve_delay);
        }

        if !(self.restart || self.stop) {
            match self.state {
                1 => {
                    self.initialize_maze();
                    if self.ifrandom && self.ifinit {
                        let size = 7 + random_below(30);
                        self.grid_width = size;
                        self.grid_height = size;
                        self.bw = if size > 6 { 3 } else { (size - 1) / 2 };
                        self.ifinit = false;
                        self.restart = true;
                    }
                }
                2 => {
                    d.clear_window();
                    self.draw_maze_border(d);
                }
                3 => {
                    let mut which = self.generator;
                    if !(0..=2).contains(&which) {
                        which = random_below(3);
                    }
                    match which {
                        0 => self.create_maze(d),
                        1 => self.alt_create_maze(d),
                        _ => self.set_create_maze(d),
                    }
                }
                4 => this_delay = self.pre_solve_delay,
                5 => {
                    if !self.solve_maze(d) {
                        self.state -= 1; /* stay in state 5 */
                    }
                }
                _ => {
                    self.erase_window = true;
                    this_delay = self.post_solve_delay;
                    self.state = 0;
                    self.ifinit = true;
                }
            }
            self.state += 1;
        }

        if self.restart {
            self.restart = false;
            self.stop = false;
            self.state = 1;
            self.solve.running = false;

            let mut size = d.res.int("gridSize");
            if size < 2 {
                size = 7 + random_below(30);
            }
            self.grid_width = size;
            self.grid_height = size;
            self.bw = if size > 6 { 3 } else { (size - 1) / 2 };
            self.set_maze_sizes(d.width(), d.height());
        }

        this_delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {
        self.restart = true;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match *event {
            XEvent::ButtonPress { button: 2, .. } => {
                self.stop = !self.stop;
                if self.state == 5 {
                    self.state = 4;
                } else {
                    self.restart = true;
                    self.stop = false;
                }
                true
            }
            XEvent::ButtonPress { .. } => {
                self.restart = true;
                self.stop = false;
                true
            }
            _ if screenhack_event_helper(event) => {
                self.restart = true;
                self.stop = false;
                true
            }
            _ => false,
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut size = d.res.int("gridSize");
    let ifrandom = size == 0;
    if size < 2 {
        size = 7 + random_below(30);
    }

    // Upstream picks the logo by screen size, so the maze keeps roughly the
    // same proportion of it whatever the display is.
    let bytes = if d.width() > 2500 || d.height() > 2500 {
        crate::images::LOGO_360
    } else if d.width() > 900 || d.height() > 900 {
        crate::images::LOGO_180
    } else {
        crate::images::LOGO_50
    };
    let decoded = png::decode(bytes);
    let (logo_width, logo_height) = match &decoded {
        Some((img, _)) => (img.width(), img.height()),
        None => (0, 0),
    };

    let fg: Pixel = d.res.pixel("foreground");
    let bg: Pixel = d.res.pixel("background");
    let mut logo_gc = Gc::new(fg, bg);
    if let Some((_, Some(mask))) = &decoded {
        logo_gc.set_clip_mask(Rc::new(mask.clone()));
    }

    let mut st = Maze {
        gc: Gc::new(fg, bg),
        dead_gc: Gc::new(d.res.pixel("deadColor"), bg),
        live_gc: Gc::new(d.res.pixel("liveColor"), bg),
        skip_gc: Gc::new(d.res.pixel("skipColor"), bg),
        unreachable_gc: Gc::new(d.res.pixel("surroundColor"), bg),
        logo_gc,
        logo: decoded.map(|(img, _)| img),
        logo_cell: None,
        logo_width,
        logo_height,
        solve_delay: d.res.int("solveDelay").max(0) as u32,
        pre_solve_delay: d.res.int("preDelay").max(0) as u32,
        post_solve_delay: d.res.int("postDelay").max(0) as u32,
        maze: Vec::new(),
        move_list: Vec::new(),
        path: Vec::new(),
        maze_size_x: 0,
        maze_size_y: 0,
        sqnum: 0,
        cur_sq_x: 0,
        cur_sq_y: 0,
        start_x: 0,
        start_y: 0,
        start_dir: 0,
        end_x: 0,
        end_y: 0,
        end_dir: 0,
        grid_width: size,
        grid_height: size,
        bw: if size > 6 { 3 } else { (size - 1) / 2 },
        restart: false,
        stop: false,
        state: 1,
        max_length: d.res.int("maxLength"),
        ignorant_p: d.res.bool("ignorant"),
        generator: d.res.int("generator"),
        solve: SolveState::default(),
        sets: Vec::new(),
        hedges: Vec::new(),
        erase_window: false,
        eraser: None,
        ifrandom,
        ifinit: true,
    };
    st.set_maze_sizes(d.width(), d.height());
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    ".background:	   black",
    ".foreground:	   white",
    "*fpsSolid:	   true",
    "*gridSize:	   0",
    "*generator:     -1",
    "*maxLength:     5",
    "*ignorant:      False",
    "*solveDelay:	   10000",
    "*preDelay:	   2000000",
    "*postDelay:	   4000000",
    "*liveColor:	   #00FF00",
    "*deadColor:	   #880000",
    "*skipColor:     #8B5A00",
    "*surroundColor: #220055",
];

const GENERATORS: &[SelectItem] = &[
    SelectItem {
        value: "-1",
        label: "Random maze generator",
    },
    SelectItem {
        value: "0",
        label: "Depth-first backtracking maze generator",
    },
    SelectItem {
        value: "1",
        label: "Wall-building maze generator (Prim)",
    },
    SelectItem {
        value: "2",
        label: "Set-joining maze generator (Kruskal)",
    },
];

const IGNORANCE: &[SelectItem] = &[
    SelectItem {
        value: "False",
        label: "Head toward exit",
    },
    SelectItem {
        value: "True",
        label: "Ignorant of exit direction",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "solveDelay",
        "Frame rate",
        0.0,
        100_000.0,
        1000.0,
        0,
        "10000",
    )
    .inverted(),
    Opt::select("generator", "Generator", GENERATORS, "-1"),
    Opt::select("ignorant", "Solver", IGNORANCE, "False"),
    Opt::spin("gridSize", "Grid size", 0.0, 100.0, "0"),
    Opt::slider(
        "preDelay",
        "Linger before solving",
        0.0,
        10_000_000.0,
        100_000.0,
        0,
        "2000000",
    ),
    Opt::slider(
        "postDelay",
        "Linger after solving",
        0.0,
        10_000_000.0,
        100_000.0,
        0,
        "4000000",
    ),
];

pub static DEF: SaverDef = SaverDef {
    slug: "maze",
    label: "Maze",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Martin Weiss, Dave Lemke, Jim Randell, Jamie Zawinski, Johannes Keukelaar, and Zack Weinberg",
        year: "1985",
        video: Some("https://www.youtube.com/watch?v=-u4neMXIRA8"),
        blurb: "Random mazes, built three different ways and then solved.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
