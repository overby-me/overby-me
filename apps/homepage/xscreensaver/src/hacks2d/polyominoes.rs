//! Port of `hacks/polyominoes.c`.
//!
//! ```text
//! polyominoes --- Shows attempts to place polyominoes into a rectangle
//!
//! Copyright (c) 2000 by Stephen Montgomery-Smith <stephen@math.missouri.edu>
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
//! ```
//!
//! A rectangle and a set of pieces that exactly cover it, filled in one piece
//! per frame by a backtracking search you can watch make its mistakes. When it
//! runs out of room it takes the last piece back off and tries the next way up.
//!
//! What makes it finish at all is that it gives up early. Before a placement is
//! kept, the board is checked for holes that cannot be filled: a region whose
//! area is not a multiple of the piece size, or (the chess-board argument) one
//! whose black and white squares are too far out of balance for the pieces left
//! to cover. And rather than filling in reading order it picks the square that
//! the fewest pieces can reach, which is usually the one that is about to prove
//! the position dead.
//!
//! There are nine puzzles built out of one repeated piece and five built out of
//! a full set of distinct ones, and two of the nine are stated to have only
//! solutions symmetric under a half turn, so the search places those pieces in
//! pairs.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Fb, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XRectangle, XSegment,
};

/// One of upstream's fixed shape tables: the squares, which of the eight
/// symmetries of the square give a distinct placement, and the most black
/// squares it can cover on a chess board.
struct Shape {
    points: &'static [(i32, i32)],
    transforms: &'static [u8],
    max_white: i32,
}

/// A piece of the puzzle in play: the shape with its squares and symmetries
/// shuffled, plus where it currently sits.
#[derive(Clone)]
struct Piece {
    point: Vec<(i32, i32)>,
    transform_list: Vec<u8>,
    max_white: i32,
    color: Pixel,
    attached: bool,
    attach_point: (i32, i32),
    point_no: usize,
    transform_index: usize,
}

impl Piece {
    /// `copy_polyomino`. With `new_rand` the squares and the symmetries are
    /// each taken in a random order, which is what makes two runs of the same
    /// puzzle find different solutions.
    fn from_shape(s: &Shape, new_rand: bool, fg: Pixel) -> Self {
        let point = if new_rand {
            let perm = random_permutation(s.points.len());
            perm.iter().map(|&i| s.points[i]).collect()
        } else {
            s.points.to_vec()
        };
        let transform_list = if new_rand {
            let perm = random_permutation(s.transforms.len());
            perm.iter().map(|&i| s.transforms[i]).collect()
        } else {
            s.transforms.to_vec()
        };
        Piece {
            point,
            transform_list,
            max_white: s.max_white,
            color: fg,
            attached: false,
            attach_point: (0, 0),
            point_no: 0,
            transform_index: 0,
        }
    }
}

/// `random_permutation`, kept as upstream writes it rather than as a shuffle,
/// because it draws from the generator in its own particular order.
fn random_permutation(n: usize) -> Vec<usize> {
    let mut a = vec![usize::MAX; n];
    for i in 0..n {
        let r = nrand((n - i) as i32) as usize;
        let mut k = 0;
        while a[k] != usize::MAX {
            k += 1;
        }
        for _ in 0..r {
            k += 1;
            while a[k] != usize::MAX {
                k += 1;
            }
        }
        a[k] = i;
    }
    a
}

/// The eight symmetries of the square, as upstream's `transform`.
fn transform(inp: (i32, i32), offset: (i32, i32), no: u8, at: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (inp.0 - offset.0, inp.1 - offset.1);
    match no {
        1 => (-dy + at.0, dx + at.1),
        2 => (-dx + at.0, -dy + at.1),
        3 => (dy + at.0, -dx + at.1),
        4 => (-dx + at.0, dy + at.1),
        5 => (dy + at.0, dx + at.1),
        6 => (dx + at.0, -dy + at.1),
        7 => (-dy + at.0, -dx + at.1),
        _ => (dx + at.0, dy + at.1),
    }
}

/// Which test a puzzle uses to reject a dead position. The number is the size
/// of the pieces; the pair is a puzzle mixing two sizes, where a region has to
/// be a non-negative combination of them.
#[derive(Clone, Copy)]
enum Check {
    MultipleOf(i32),
    CombinationOf(i32, i32),
}

/// A set bit says that side or corner of a square is an edge of its piece, so
/// the index is which of the 256 tiles to draw.
const LEFT: usize = 1 << 0;
const RIGHT: usize = 1 << 1;
const UP: usize = 1 << 2;
const DOWN: usize = 1 << 3;
const LEFT_UP: usize = 1 << 4;
const LEFT_DOWN: usize = 1 << 5;
const RIGHT_UP: usize = 1 << 6;
const RIGHT_DOWN: usize = 1 << 7;

/// A corner tile is the same picture as the tile without that corner whenever
/// one of the two sides meeting there is already an edge, so upstream builds
/// one bitmap and points the duplicates at it. This is that reduction, applied
/// until it settles, since the tile it lands on may itself be a duplicate.
fn canonical(n: usize) -> usize {
    let mut n = n;
    loop {
        let m = if n & LEFT_UP != 0 && (n & LEFT != 0 || n & UP != 0) {
            n & !LEFT_UP
        } else if n & LEFT_DOWN != 0 && (n & LEFT != 0 || n & DOWN != 0) {
            n & !LEFT_DOWN
        } else if n & RIGHT_UP != 0 && (n & RIGHT != 0 || n & UP != 0) {
            n & !RIGHT_UP
        } else if n & RIGHT_DOWN != 0 && (n & RIGHT != 0 || n & DOWN != 0) {
            n & !RIGHT_DOWN
        } else {
            return n;
        };
        n = m;
    }
}

struct Polyominoes {
    mi: ModeInfo,
    wait: i32,
    /// The board, in squares.
    width: i32,
    height: i32,
    border_color: Pixel,
    mono: bool,
    polyomino: Vec<Piece>,
    identical: bool,
    use3d: bool,
    attach_list: Vec<usize>,
    nr_attached: usize,
    /// Which piece covers each square, `-1` for blank. The search also parks
    /// negative marks here while it walks a region, and puts them back after.
    array: Vec<i32>,
    changed_array: Vec<bool>,
    /// Size of a square on screen, and where the board sits in the window.
    box_: i32,
    x_margin: i32,
    y_margin: i32,
    /// Which corner the search prefers to work from, rolled once per puzzle.
    left_right: bool,
    top_bottom: bool,
    use_bitmaps: bool,
    /// The 256 tiles, built once a square is big enough to be worth shading.
    /// Only the tiles that are not duplicates of another are present.
    bitmaps: Vec<Option<Fb>>,
    check: Check,
    /// Solutions are invariant under a half turn, so pieces go on in pairs.
    rot180: bool,
    /// For the puzzles built from one repeated piece: which already-placed
    /// pieces were in the way of the last attempt. Backtracking then goes
    /// straight back to one of those rather than undoing work that was fine.
    reason_to_not_attach: Vec<bool>,
    counter: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Polyominoes {
        mi,
        wait: 0,
        width: 1,
        height: 1,
        border_color: 0,
        mono: false,
        polyomino: Vec::new(),
        identical: false,
        use3d: true,
        attach_list: Vec::new(),
        nr_attached: 0,
        array: Vec::new(),
        changed_array: Vec::new(),
        box_: 0,
        x_margin: 0,
        y_margin: 0,
        left_right: false,
        top_bottom: false,
        use_bitmaps: false,
        bitmaps: Vec::new(),
        check: Check::MultipleOf(5),
        rot180: false,
        reason_to_not_attach: Vec::new(),
        counter: 0,
    };
    st.reset(d);
    Box::new(st)
}

impl Polyominoes {
    fn at(&self, x: i32, y: i32) -> i32 {
        self.array[(x * self.height + y) as usize]
    }

    fn set_at(&mut self, x: i32, y: i32, v: i32) {
        self.array[(x * self.height + y) as usize] = v;
    }

    fn changed(&self, x: i32, y: i32) -> bool {
        self.changed_array[(x * self.height + y) as usize]
    }

    /// `ARR`: like `at`, but off the board counts as covered.
    fn arr(&self, x: i32, y: i32) -> i32 {
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            -2
        } else {
            self.at(x, y)
        }
    }

    fn reason(&self, x: usize, y: usize) -> bool {
        self.reason_to_not_attach[x * self.polyomino.len() + y]
    }

    fn set_reason(&mut self, x: usize, y: usize, v: bool) {
        let n = self.polyomino.len();
        self.reason_to_not_attach[x * n + y] = v;
    }

    /// `count_adjacent_blanks`. Upstream recurses; this walks a stack, which
    /// marks the same squares and counts the same number without putting a
    /// board's worth of frames on the stack.
    fn count_adjacent_blanks(&mut self, x: i32, y: i32, blank_mark: i32) -> i32 {
        if self.at(x, y) != -1 {
            return 0;
        }
        let mut count = 0;
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            if self.at(x, y) != -1 {
                continue;
            }
            count += 1;
            self.set_at(x, y, blank_mark);
            if x >= 1 {
                stack.push((x - 1, y));
            }
            if x < self.width - 1 {
                stack.push((x + 1, y));
            }
            if y >= 1 {
                stack.push((x, y - 1));
            }
            if y < self.height - 1 {
                stack.push((x, y + 1));
            }
        }
        count
    }

    fn unmark(&mut self, mark: i32) {
        for v in self.array.iter_mut() {
            if *v == mark {
                *v = -1;
            }
        }
    }

    /// No region of blanks may have an area that the remaining pieces cannot
    /// tile, which for one piece size means a multiple of it.
    fn regions_multiple_of(&mut self, n: i32) -> bool {
        let mut good = true;
        'outer: for x in 0..self.width {
            for y in 0..self.height {
                let count = self.count_adjacent_blanks(x, y, -2);
                good = count % n == 0;
                if !good {
                    break 'outer;
                }
            }
        }
        self.unmark(-2);
        good
    }

    /// The same for a puzzle mixing two sizes: a region has to be some number
    /// of one plus some number of the other.
    fn regions_combination_of(&mut self, m: i32, n: i32) -> bool {
        let mut good = true;
        'outer: for x in 0..self.width {
            for y in 0..self.height {
                let mut count = self.count_adjacent_blanks(x, y, -2);
                good = false;
                while count >= 0 && !good {
                    good = count % n == 0;
                    count -= m;
                }
                if !good {
                    break 'outer;
                }
            }
        }
        self.unmark(-2);
        good
    }

    /// The chess-board argument: colour the board like a chess board and count
    /// what is left blank of each colour. Each piece covers a known range of
    /// blacks, so if the blanks fall outside what the remaining pieces can
    /// cover between them, the position is already lost.
    fn whites_ok(&self) -> bool {
        let (mut whites, mut blacks) = (0, 0);
        for x in 0..self.width {
            for y in 0..self.height {
                if self.at(x, y) == -1 {
                    if (x + y) % 2 != 0 {
                        whites += 1;
                    } else {
                        blacks += 1;
                    }
                }
            }
        }
        let (mut max_white, mut min_white) = (0, 0);
        for p in &self.polyomino {
            if !p.attached {
                max_white += p.max_white;
                min_white += p.point.len() as i32 - p.max_white;
            }
        }
        min_white <= blacks && min_white <= whites && blacks <= max_white && whites <= max_white
    }

    fn check_ok(&mut self) -> bool {
        match self.check {
            Check::MultipleOf(n) => self.regions_multiple_of(n) && self.whites_ok(),
            Check::CombinationOf(m, n) => self.regions_combination_of(m, n) && self.whites_ok(),
        }
    }

    fn first_poly_no(&self) -> usize {
        let mut n = 0;
        while n < self.polyomino.len() && self.polyomino[n].attached {
            n += 1;
        }
        n
    }

    /// With every piece the same there is no point trying the others, so this
    /// runs straight off the end.
    fn next_poly_no(&self, poly_no: &mut usize) {
        if self.identical {
            *poly_no = self.polyomino.len();
        } else {
            loop {
                *poly_no += 1;
                if *poly_no >= self.polyomino.len() || !self.polyomino[*poly_no].attached {
                    break;
                }
            }
        }
    }

    /// How many ways any remaining piece can be made to cover this square. The
    /// square with the fewest is the one to fill next.
    fn score_point(&self, x: i32, y: i32, min_score_so_far: i32) -> i32 {
        // A square with all eight neighbours free tells us nothing, and
        // checking it is the expensive case, so upstream scores it out of the
        // running instead.
        if x >= 1
            && x < self.width - 1
            && y >= 1
            && y < self.height - 1
            && self.at(x - 1, y - 1) < 0
            && self.at(x - 1, y) < 0
            && self.at(x - 1, y + 1) < 0
            && self.at(x + 1, y - 1) < 0
            && self.at(x + 1, y) < 0
            && self.at(x + 1, y + 1) < 0
            && self.at(x, y - 1) < 0
            && self.at(x, y + 1) < 0
        {
            return 10000;
        }

        let attach_point = (x, y);
        let mut score = 0;
        let mut poly_no = self.first_poly_no();
        while poly_no < self.polyomino.len() {
            let p = &self.polyomino[poly_no];
            if !p.attached {
                for point_no in 0..p.point.len() {
                    for &t in &p.transform_list {
                        let mut attachable = true;
                        for i in 0..p.point.len() {
                            let tp = transform(p.point[i], p.point[point_no], t, attach_point);
                            if !(tp.0 >= 0
                                && tp.0 < self.width
                                && tp.1 >= 0
                                && tp.1 < self.height
                                && self.at(tp.0, tp.1) < 0)
                            {
                                attachable = false;
                                break;
                            }
                        }
                        if attachable {
                            score += 1;
                            if score >= min_score_so_far {
                                return score;
                            }
                        }
                    }
                }
            }
            self.next_poly_no(&mut poly_no);
        }
        score
    }

    /// `find_smallest_blank_component`: the mark of the smallest region of
    /// blanks, which is where the search should work.
    fn find_smallest_blank_component(&mut self) -> i32 {
        let mut blank_mark = -10;
        let mut smallest_mark = -10;
        let mut smallest_size = 1_000_000_000;
        for x in 0..self.width {
            for y in 0..self.height {
                if self.at(x, y) == -1 {
                    let size = self.count_adjacent_blanks(x, y, blank_mark);
                    if size < smallest_size {
                        smallest_mark = blank_mark;
                        smallest_size = size;
                    }
                    blank_mark -= 1;
                }
            }
        }
        smallest_mark
    }

    fn find_blank(&mut self, point: &mut (i32, i32)) {
        let blank_mark = self.find_smallest_blank_component();

        let mut worst_score = 1_000_000;
        for x in 0..self.width {
            for y in 0..self.height {
                if self.at(x, y) != blank_mark {
                    continue;
                }
                let mut score = 100 * self.score_point(x, y, worst_score);
                if score > 0 {
                    score += if self.left_right {
                        10 * x
                    } else {
                        10 * (self.width - 1 - x)
                    };
                    score += if self.top_bottom {
                        y
                    } else {
                        self.height - 1 - y
                    };
                }
                if score < worst_score {
                    *point = (x, y);
                    worst_score = score;
                }
            }
        }

        for v in self.array.iter_mut() {
            if *v < 0 {
                *v = -1;
            }
        }
    }

    /// Take the most recently placed piece back off.
    fn detach(&mut self, cur: &mut Cursor, rot180: bool) {
        if self.nr_attached == 0 {
            return;
        }
        self.nr_attached -= 1;
        cur.poly_no = self.attach_list[self.nr_attached];
        let p = &self.polyomino[cur.poly_no];
        cur.point_no = p.point_no;
        cur.transform_index = p.transform_index;
        cur.attach_point = p.attach_point;
        let t = p.transform_list[cur.transform_index] ^ ((rot180 as u8) << 1);
        let (points, origin, at) = (p.point.clone(), p.point[cur.point_no], cur.attach_point);
        for pt in points {
            let tp = transform(pt, origin, t, at);
            self.set_at(tp.0, tp.1, -1);
            self.changed_array[(tp.0 * self.height + tp.1) as usize] = true;
        }
        self.polyomino[cur.poly_no].attached = false;
    }

    /// Try to place a piece so that its `point_no`-th square lands on
    /// `attach_point`, under one of its symmetries.
    fn attach(
        &mut self,
        poly_no: usize,
        point_no: usize,
        transform_index: usize,
        attach_point: (i32, i32),
        rot180: bool,
    ) -> bool {
        let mut attach_point = attach_point;
        if rot180 {
            attach_point = (
                self.width - 1 - attach_point.0,
                self.height - 1 - attach_point.1,
            );
        }
        if self.polyomino[poly_no].attached {
            return false;
        }

        let p = &self.polyomino[poly_no];
        if point_no >= p.point.len() || transform_index >= p.transform_list.len() {
            return false;
        }
        let t = p.transform_list[transform_index] ^ ((rot180 as u8) << 1);
        let origin = p.point[point_no];
        let points = p.point.clone();

        let mut attachable = true;
        let mut worst_reason_not_to_attach = 1_000_000_000;
        for &pt in &points {
            let tp = transform(pt, origin, t, attach_point);
            let free = tp.0 >= 0
                && tp.0 < self.width
                && tp.1 >= 0
                && tp.1 < self.height
                && self.at(tp.0, tp.1) == -1;
            if !free {
                if !self.identical {
                    return false;
                }
                attachable = false;
                if tp.0 >= 0 && tp.0 < self.width && tp.1 >= 0 && tp.1 < self.height {
                    let v = self.at(tp.0, tp.1);
                    if v >= 0 && v < worst_reason_not_to_attach {
                        worst_reason_not_to_attach = v;
                    }
                }
            }
        }

        if self.identical && !attachable {
            if worst_reason_not_to_attach < 1_000_000_000 {
                let row = self.nr_attached;
                self.set_reason(row, worst_reason_not_to_attach as usize, true);
            }
            return false;
        }

        for &pt in &points {
            let tp = transform(pt, origin, t, attach_point);
            self.set_at(tp.0, tp.1, poly_no as i32);
            self.changed_array[(tp.0 * self.height + tp.1) as usize] = true;
        }

        self.attach_list[self.nr_attached] = poly_no;
        self.nr_attached += 1;

        let p = &mut self.polyomino[poly_no];
        p.attached = true;
        p.point_no = point_no;
        p.attach_point = attach_point;
        p.transform_index = transform_index;

        if !self.check_ok() {
            let mut cur = Cursor {
                poly_no,
                point_no,
                transform_index,
                attach_point,
            };
            self.detach(&mut cur, rot180);
            return false;
        }
        true
    }

    /// Step to the next thing to try: the next symmetry, else the next square
    /// of the piece, else the next piece. False once every combination at this
    /// square has been tried.
    fn next_attach_try(&self, cur: &mut Cursor) -> bool {
        cur.transform_index += 1;
        if cur.transform_index >= self.polyomino[cur.poly_no].transform_list.len() {
            cur.transform_index = 0;
            cur.point_no += 1;
            if cur.point_no >= self.polyomino[cur.poly_no].point.len() {
                cur.point_no = 0;
                self.next_poly_no(&mut cur.poly_no);
                if cur.poly_no >= self.polyomino.len() {
                    cur.poly_no = self.first_poly_no();
                    return false;
                }
            }
        }
        true
    }
}

/// Where the search is: which piece, on which of its squares, under which
/// symmetry, at which square of the board.
struct Cursor {
    poly_no: usize,
    point_no: usize,
    transform_index: usize,
    attach_point: (i32, i32),
}

/*******************************************************
Display routines.
*******************************************************/

impl Polyominoes {
    fn draw_without_bitmaps(&mut self, d: &mut Dpy) {
        let box_ = self.box_;
        self.mi.gc.set_line_width(box_ / 10 + 1);

        let mut rects: Vec<XRectangle> = Vec::new();
        for poly_no in -1..self.polyomino.len() as i32 {
            rects.clear();
            for x in 0..self.width {
                for y in 0..self.height {
                    if self.changed(x, y) && self.at(x, y) == poly_no {
                        rects.push(XRectangle {
                            x: self.x_margin + box_ * x,
                            y: self.y_margin + box_ * y,
                            width: box_,
                            height: box_,
                        });
                    }
                }
            }
            let c = if poly_no == -1 {
                self.mi.black
            } else {
                self.polyomino[poly_no as usize].color
            };
            self.mi.gc.set_foreground(c);
            d.win().fill_rectangles(&self.mi.gc, &rects);
        }

        // The grid inside the blank area, in the background colour, so the
        // squares that are still empty read as squares.
        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        let mut segs: Vec<XSegment> = Vec::new();
        for x in 0..self.width - 1 {
            for y in 0..self.height {
                if self.at(x, y) == -1
                    && self.at(x + 1, y) == -1
                    && (self.changed(x, y) || self.changed(x + 1, y))
                {
                    segs.push(XSegment {
                        x1: self.x_margin + box_ * (x + 1),
                        y1: self.y_margin + box_ * y,
                        x2: self.x_margin + box_ * (x + 1),
                        y2: self.y_margin + box_ * (y + 1),
                    });
                }
            }
        }
        d.win().draw_segments(&self.mi.gc, &segs);

        segs.clear();
        for x in 0..self.width {
            for y in 0..self.height - 1 {
                if self.at(x, y) == -1
                    && self.at(x, y + 1) == -1
                    && (self.changed(x, y) || self.changed(x, y + 1))
                {
                    segs.push(XSegment {
                        x1: self.x_margin + box_ * x,
                        y1: self.y_margin + box_ * (y + 1),
                        x2: self.x_margin + box_ * (x + 1),
                        y2: self.y_margin + box_ * (y + 1),
                    });
                }
            }
        }
        d.win().draw_segments(&self.mi.gc, &segs);

        let white = self.mi.white;
        self.mi.gc.set_foreground(white);
        d.win().draw_rectangle(
            &self.mi.gc,
            self.x_margin,
            self.y_margin,
            box_ * self.width,
            box_ * self.height,
        );

        // The outlines between pieces, which is the only thing telling one
        // piece from its neighbour when they land on the same colour.
        segs.clear();
        for x in 0..self.width - 1 {
            for y in 0..self.height {
                if self.at(x + 1, y) != self.at(x, y) {
                    segs.push(XSegment {
                        x1: self.x_margin + box_ * (x + 1),
                        y1: self.y_margin + box_ * y,
                        x2: self.x_margin + box_ * (x + 1),
                        y2: self.y_margin + box_ * (y + 1),
                    });
                }
            }
        }
        d.win().draw_segments(&self.mi.gc, &segs);

        segs.clear();
        for x in 0..self.width {
            for y in 0..self.height - 1 {
                if self.at(x, y + 1) != self.at(x, y) {
                    segs.push(XSegment {
                        x1: self.x_margin + box_ * x,
                        y1: self.y_margin + box_ * (y + 1),
                        x2: self.x_margin + box_ * (x + 1),
                        y2: self.y_margin + box_ * (y + 1),
                    });
                }
            }
        }
        d.win().draw_segments(&self.mi.gc, &segs);
        self.mi.gc.set_line_width(1);
    }

    fn draw_with_bitmaps(&mut self, d: &mut Dpy) {
        let box_ = self.box_;
        for x in 0..self.width {
            for y in 0..self.height {
                if self.at(x, y) == -1 {
                    if self.changed(x, y) {
                        let black = self.mi.black;
                        self.mi.gc.set_foreground(black);
                        d.win().fill_rectangle(
                            &self.mi.gc,
                            self.x_margin + box_ * x,
                            self.y_margin + box_ * y,
                            box_,
                            box_,
                        );
                    }
                    continue;
                }
                let c = self.polyomino[self.at(x, y) as usize].color;
                self.mi.gc.set_foreground(c);
                let here = self.arr(x, y);
                let mut n = 0;
                if here != self.arr(x - 1, y) {
                    n |= LEFT;
                }
                if here != self.arr(x + 1, y) {
                    n |= RIGHT;
                }
                if here != self.arr(x, y - 1) {
                    n |= UP;
                }
                if here != self.arr(x, y + 1) {
                    n |= DOWN;
                }
                if here != self.arr(x - 1, y - 1) {
                    n |= LEFT_UP;
                }
                if here != self.arr(x - 1, y + 1) {
                    n |= LEFT_DOWN;
                }
                if here != self.arr(x + 1, y - 1) {
                    n |= RIGHT_UP;
                }
                if here != self.arr(x + 1, y + 1) {
                    n |= RIGHT_DOWN;
                }
                let Some(bm) = &self.bitmaps[canonical(n)] else {
                    continue;
                };
                d.win().copy_plane(
                    &self.mi.gc,
                    bm,
                    0,
                    0,
                    box_,
                    box_,
                    self.x_margin + box_ * x,
                    self.y_margin + box_ * y,
                );
            }
        }

        let border = self.border_color;
        self.mi.gc.set_foreground(border);
        let g = box_ / 45 + 1;
        let t = if box_ <= 12 { 1 } else { g * 2 };
        for k in g..g + t {
            d.win().draw_rectangle(
                &self.mi.gc,
                self.x_margin - k - 1,
                self.y_margin - k - 1,
                box_ * self.width + 1 + 2 * k,
                box_ * self.height + 1 + 2 * k,
            );
        }
    }

    /// Build the 256 tiles a piece can be drawn from: one for each combination
    /// of which sides and corners of a square are the edge of its piece. The
    /// shading is a dither pattern rather than a colour, because the tile is a
    /// one-bit bitmap stamped in the piece's own colour.
    fn create_bitmaps(&mut self) {
        let box_ = self.box_;
        let g = box_ / 45 + 1;
        let t = if box_ <= 12 { 1 } else { g * 2 };
        let r = if box_ <= 12 { 1 } else { g * 6 };
        // 3 approximates 2 sqrt(2).
        let rt = if box_ <= 12 { 1 } else { g * 3 };
        let rr = 0;

        self.bitmaps = (0..256).map(|_| None).collect();
        for n in 0..256usize {
            if canonical(n) != n {
                continue;
            }
            let mut bm = Fb::new_bitmap(box_, box_);
            let set = |bm: &mut Fb, x: i32, y: i32| bm.put_pixel(x, y, 1);
            let res = |bm: &mut Fb, x: i32, y: i32| bm.put_pixel(x, y, 0);
            let half =
                |bm: &mut Fb, x: i32, y: i32| bm.put_pixel(x, y, u32::from((x - y) % 2 != 0));
            let two_thirds =
                |bm: &mut Fb, x: i32, y: i32| bm.put_pixel(x, y, u32::from((x + y - 1) % 3 != 0));
            let third =
                |bm: &mut Fb, x: i32, y: i32| bm.put_pixel(x, y, u32::from((x - y - 1) % 3 == 0));
            let three_quarters = |bm: &mut Fb, x: i32, y: i32| {
                bm.put_pixel(x, y, u32::from(y % 2 != 0 || (x + 2 + y / 2 + 1) % 2 != 0))
            };

            let is = |bit: usize| n & bit != 0;

            // The body of the tile: flat is a checkerboard dither, and the
            // raised look is four trapezia meeting at the middle, each at a
            // different density, so a piece reads as a slab lit from one side.
            for y in 0..box_ {
                // The two diagonals of the square cut it into four triangles,
                // and which side of each a point falls on is what selects the
                // face it belongs to.
                let anti = box_ - y - 1;
                let half_way = box_ / 2;
                for x in 0..box_ {
                    if !self.use3d {
                        half(&mut bm, x, y);
                    } else if (x >= y && x <= anti && is(UP))
                        || (x <= y && x <= anti && y < half_way && !is(LEFT))
                        || (x >= y && x >= anti && y < half_way && !is(RIGHT))
                    {
                        set(&mut bm, x, y);
                    } else if (x <= y && x <= anti && is(LEFT))
                        || (x >= y && x <= anti && x < half_way && !is(UP))
                        || (x <= y && x >= anti && x < half_way && !is(DOWN))
                    {
                        two_thirds(&mut bm, x, y);
                    } else if (x >= y && x >= anti && is(RIGHT))
                        || (x >= y && x <= anti && x >= half_way && !is(UP))
                        || (x <= y && x >= anti && x >= half_way && !is(DOWN))
                    {
                        half(&mut bm, x, y);
                    } else if (x <= y && x >= anti && is(DOWN))
                        || (x <= y && x <= anti && y >= half_way && !is(LEFT))
                        || (x >= y && x >= anti && y >= half_way && !is(RIGHT))
                    {
                        third(&mut bm, x, y);
                    }
                }
            }

            // A solid wall along each side that is an edge of the piece, and a
            // gap outside it so neighbouring pieces do not touch.
            if is(LEFT) {
                for y in 0..box_ {
                    for x in g..g + t {
                        set(&mut bm, x, y);
                    }
                    for x in 0..g {
                        res(&mut bm, x, y);
                    }
                }
            }
            if is(RIGHT) {
                for y in 0..box_ {
                    for x in g..g + t {
                        set(&mut bm, box_ - 1 - x, y);
                    }
                    for x in 0..g {
                        res(&mut bm, box_ - 1 - x, y);
                    }
                }
            }
            if is(UP) {
                for x in 0..box_ {
                    for y in g..g + t {
                        set(&mut bm, x, y);
                    }
                    for y in 0..g {
                        res(&mut bm, x, y);
                    }
                }
            }
            if is(DOWN) {
                for x in 0..box_ {
                    for y in g..g + t {
                        set(&mut bm, x, box_ - 1 - y);
                    }
                    for y in 0..g {
                        res(&mut bm, x, box_ - 1 - y);
                    }
                }
            }

            // Round off a corner where two walls meet.
            for (lx, ly, wall_x, wall_y) in [
                (true, true, LEFT, UP),
                (true, false, LEFT, DOWN),
                (false, true, RIGHT, UP),
                (false, false, RIGHT, DOWN),
            ] {
                if !(is(wall_x) && is(wall_y)) {
                    continue;
                }
                for x in g..=g + r {
                    for y in g..=r + 2 * g - x {
                        let px = if lx { x } else { box_ - 1 - x };
                        let py = if ly { y } else { box_ - 1 - y };
                        if x + y > r + 2 * g - rt {
                            set(&mut bm, px, py);
                        } else {
                            res(&mut bm, px, py);
                        }
                    }
                }
            }

            // An inside corner, where the piece turns: the wall carries on
            // round the notch.
            for (lx, ly, wall_x, wall_y, corner) in [
                (true, true, LEFT, UP, LEFT_UP),
                (true, false, LEFT, DOWN, LEFT_DOWN),
                (false, true, RIGHT, UP, RIGHT_UP),
                (false, false, RIGHT, DOWN, RIGHT_DOWN),
            ] {
                if is(wall_x) || is(wall_y) || !is(corner) {
                    continue;
                }
                let px = |x: i32| if lx { x } else { box_ - 1 - x };
                let py = |y: i32| if ly { y } else { box_ - 1 - y };
                for x in 0..g {
                    for y in 0..g {
                        res(&mut bm, px(x), py(y));
                    }
                }
                for x in g..g + t {
                    for y in 0..g {
                        set(&mut bm, px(x), py(y));
                    }
                }
                for x in 0..g + t {
                    for y in g..g + t {
                        set(&mut bm, px(x), py(y));
                    }
                }
            }

            // Where a square is in the middle of its piece, carry the raised
            // face across the join instead of leaving the four faces meeting
            // in a point.
            if self.use3d {
                for (lx, ly, wall_x, wall_y, corner) in [
                    (true, true, LEFT, UP, LEFT_UP),
                    (true, false, LEFT, DOWN, LEFT_DOWN),
                    (false, true, RIGHT, UP, RIGHT_UP),
                    (false, false, RIGHT, DOWN, RIGHT_DOWN),
                ] {
                    if is(wall_x) || is(wall_y) || is(corner) {
                        continue;
                    }
                    let xs: Vec<i32> = if lx {
                        (0..box_ / 2 - rr).collect()
                    } else {
                        (box_ / 2 + rr..box_).collect()
                    };
                    let ys: Vec<i32> = if ly {
                        (0..box_ / 2 - rr).collect()
                    } else {
                        (box_ / 2 + rr..box_).collect()
                    };
                    for &x in &xs {
                        for &y in &ys {
                            three_quarters(&mut bm, x, y);
                        }
                    }
                }
            }

            self.bitmaps[n] = Some(bm);
        }
        self.use_bitmaps = true;
    }
}

/***************************************************
Puzzle specific initialization routines.
***************************************************/

impl Polyominoes {
    /// Fill the board with `n` copies of one shape.
    fn set_identical(&mut self, shape: &Shape, w: i32, h: i32, n: usize, check: Check) {
        self.width = w;
        self.height = h;
        let fg = self.mi.white;
        self.polyomino = (0..n).map(|_| Piece::from_shape(shape, true, fg)).collect();
        self.check = check;
    }

    /// The same, but in pairs that share a shuffle, for the two puzzles whose
    /// solutions all have half-turn symmetry.
    fn set_identical_pairs(&mut self, shape: &Shape, w: i32, h: i32, n: usize, check: Check) {
        self.rot180 = true;
        self.width = w;
        self.height = h;
        let fg = self.mi.white;
        let mut v = Vec::with_capacity(n);
        for _ in (0..n).step_by(2) {
            let a = Piece::from_shape(shape, true, fg);
            let b = Piece::from_shape(shape, false, fg);
            v.push(a);
            v.push(b);
        }
        self.polyomino = v;
        self.check = check;
    }

    /// One puzzle out of the nine built from a single repeated piece.
    fn set_identical_puzzle(&mut self) {
        match nrand(9) {
            0 => self.set_identical(&PENTOMINO1, 10, 5, 10, Check::MultipleOf(5)),
            1 => self.set_identical(&HEXOMINO1, 24, 23, 92, Check::MultipleOf(6)),
            2 => self.set_identical_pairs(&HEPTOMINO1, 26, 21, 78, Check::MultipleOf(7)),
            3 => self.set_identical(&HEPTOMINO1, 28, 19, 76, Check::MultipleOf(7)),
            4 => self.set_identical_pairs(&ELEVENOMINO1, 25, 22, 50, Check::MultipleOf(11)),
            5 => self.set_identical(&DEKOMINO1, 32, 30, 96, Check::MultipleOf(10)),
            6 => self.set_identical(&OCTOMINO1, 96, 26, 312, Check::MultipleOf(8)),
            7 => self.set_identical(&PENTOMINO1, 15, 15, 45, Check::MultipleOf(5)),
            _ => self.set_identical(&ELEVENOMINO1, 47, 33, 141, Check::MultipleOf(11)),
        }
    }

    /// A full set of distinct pieces, taken in a random order: the p-th slot
    /// gets the shape the permutation names, so the shuffles are applied in
    /// slot order.
    fn set_shuffled(&mut self, shapes: &[&Shape], w: i32, h: i32, check: Check) {
        self.width = w;
        self.height = h;
        let fg = self.mi.white;
        let perm = random_permutation(shapes.len());
        self.polyomino = perm
            .iter()
            .map(|&i| Piece::from_shape(shapes[i], true, fg))
            .collect();
        self.check = check;
    }

    /// The other way round, which is how the two mixed-size puzzles do it: the
    /// shapes are taken in order and dealt to slots the permutation names, so
    /// the shuffles are applied in shape order instead.
    fn set_dealt(&mut self, shapes: &[&Shape], w: i32, h: i32, check: Check) {
        self.width = w;
        self.height = h;
        let fg = self.mi.white;
        let perm = random_permutation(shapes.len());
        let mut v: Vec<Option<Piece>> = (0..shapes.len()).map(|_| None).collect();
        for (p, &slot) in perm.iter().enumerate() {
            v[slot] = Some(Piece::from_shape(shapes[p], true, fg));
        }
        self.polyomino = v.into_iter().flatten().collect();
        self.check = check;
    }

    fn set_distinct_puzzle(&mut self) {
        // The one-sided sets are the reflections counted as different pieces,
        // which upstream derives by splitting each shape's symmetry list at the
        // point where the reflections start.
        match nrand(5) {
            0 => {
                let shapes: Vec<&Shape> = PENTOMINO.iter().collect();
                let (w, h) = match nrand(4) {
                    0 => (20, 3),
                    1 => (15, 4),
                    2 => (12, 5),
                    _ => (10, 6),
                };
                self.set_shuffled(&shapes, w, h, Check::MultipleOf(5));
            }
            1 => {
                let one_sided = one_sided(&PENTOMINO);
                let shapes: Vec<&Shape> = one_sided.iter().collect();
                let (w, h) = match nrand(4) {
                    0 => (30, 3),
                    1 => (18, 5),
                    2 => (15, 6),
                    _ => (10, 9),
                };
                self.set_shuffled(&shapes, w, h, Check::MultipleOf(5));
            }
            2 => {
                let one_sided = one_sided(&HEXOMINO);
                let shapes: Vec<&Shape> = one_sided.iter().collect();
                let (w, h) = match nrand(8) {
                    0 => (20, 18),
                    1 => (24, 15),
                    2 => (30, 12),
                    3 => (36, 10),
                    4 => (40, 9),
                    5 => (45, 8),
                    6 => (60, 6),
                    _ => (72, 5),
                };
                self.set_shuffled(&shapes, w, h, Check::MultipleOf(6));
            }
            3 => {
                let shapes: Vec<&Shape> = PENTOMINO.iter().chain(HEXOMINO.iter()).collect();
                let (w, h) = match nrand(5) {
                    0 => (54, 5),
                    1 => (45, 6),
                    2 => (30, 9),
                    3 => (27, 10),
                    _ => (18, 15),
                };
                self.set_dealt(&shapes, w, h, Check::CombinationOf(6, 5));
            }
            _ => {
                let shapes: Vec<&Shape> = TETROMINO.iter().chain(PENTOMINO.iter()).collect();
                let (w, h) = match nrand(3) {
                    0 => (20, 4),
                    1 => (16, 5),
                    _ => (10, 8),
                };
                self.set_dealt(&shapes, w, h, Check::CombinationOf(5, 4));
            }
        }
    }

    /// `init_polyominoes`: choose a puzzle, size it to the window and clear.
    fn reset(&mut self, d: &mut Dpy) {
        self.rot180 = false;
        self.counter = 0;

        // Upstream's driver forces full-random mode, in which these two are
        // rolled rather than read. Defer to the panel when it has set one.
        self.identical = if d.res.is_overridden("identical") {
            d.res.bool("identical")
        } else {
            lrand() & 1 != 0
        };
        self.use3d = if d.res.is_overridden("plain") {
            !d.res.bool("plain")
        } else {
            nrand(4) != 0
        };

        if self.identical {
            self.set_identical_puzzle();
        } else {
            self.set_distinct_puzzle();
        }

        if self.mi.height > self.mi.width {
            std::mem::swap(&mut self.width, &mut self.height);
        }

        self.attach_list = vec![0; self.polyomino.len()];
        self.nr_attached = 0;
        self.reason_to_not_attach = if self.identical {
            vec![false; self.polyomino.len() * self.polyomino.len()]
        } else {
            Vec::new()
        };

        self.array = vec![-1; (self.width * self.height) as usize];
        self.changed_array = vec![false; (self.width * self.height) as usize];

        self.left_right = nrand(2) != 0;
        self.top_bottom = nrand(2) != 0;

        let box1 = self.mi.width / (self.width + 2);
        let box2 = self.mi.height / (self.height + 2);
        self.box_ = box1.min(box2);

        if self.mi.width > self.mi.height * 5 || self.mi.height > self.mi.width * 5 {
            let stretch = if self.mi.width > self.mi.height {
                f64::from(self.mi.width) / f64::from(self.mi.height)
            } else {
                f64::from(self.mi.height) / f64::from(self.mi.width)
            };
            self.box_ = (f64::from(self.box_) * stretch) as i32;
        }

        self.use_bitmaps = false;
        self.bitmaps = Vec::new();
        if self.box_ >= 12 {
            self.box_ = (self.box_ / 12) * 12;
            self.create_bitmaps();
        }

        // Every piece gets a colour a fixed distance round the map from the
        // last, starting somewhere at random, so neighbours never collide.
        let perm = random_permutation(self.polyomino.len());
        self.mono = self.mi.npixels() < 12;
        let npixels = self.mi.npixels().max(1);
        let start = nrand(npixels);
        let n = self.polyomino.len();
        let mut i = 0;
        while i < n {
            if !self.mono {
                let c = self
                    .mi
                    .pixel(((perm[i] as i32 * npixels / n as i32 + start) % npixels) as usize);
                self.polyomino[i].color = c;
                if self.rot180 && i + 1 < n {
                    self.polyomino[i + 1].color = c;
                    i += 1;
                }
            } else if self.use_bitmaps {
                self.polyomino[i].color = self.mi.white;
            } else {
                self.polyomino[i].color = self.mi.black;
            }
            i += 1;
        }

        if self.use_bitmaps {
            self.border_color = if self.mono {
                self.mi.white
            } else {
                self.mi.pixel(nrand(npixels) as usize)
            };
        }

        self.x_margin = (self.mi.width - self.box_ * self.width) / 2;
        self.y_margin = (self.mi.height - self.box_ * self.height) / 2;
        self.wait = 0;

        self.mi.clear_window(d);
    }
}

impl Screenhack for Polyominoes {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.mi.cycles != 0 {
            self.counter += 1;
            if self.counter > self.mi.cycles {
                self.reset(d);
                return self.mi.delay;
            }
        }
        if self.box_ == 0 {
            self.reset(d);
            return self.mi.delay;
        }

        self.wait -= 1;
        if self.wait > 0 {
            return self.mi.delay;
        }

        for c in self.changed_array.iter_mut() {
            *c = false;
        }

        let mut cur = Cursor {
            poly_no: self.first_poly_no(),
            point_no: 0,
            transform_index: 0,
            // Upstream leaves this uninitialised, and reads it on the one frame
            // where the board is already full and there is no blank to find.
            attach_point: (0, 0),
        };
        let mut done = false;
        let mut another_attachment_try = true;
        let mut attach_point = cur.attach_point;
        self.find_blank(&mut attach_point);
        cur.attach_point = attach_point;

        let n = self.polyomino.len();
        if self.identical && self.nr_attached < n {
            for i in 0..n {
                let row = self.nr_attached;
                self.set_reason(row, i, false);
            }
        }

        while !done {
            if self.nr_attached < n {
                while !done && another_attachment_try {
                    done = self.attach(
                        cur.poly_no,
                        cur.point_no,
                        cur.transform_index,
                        attach_point,
                        false,
                    );
                    if done && self.rot180 {
                        cur.poly_no = self.first_poly_no();
                        done = self.attach(
                            cur.poly_no,
                            cur.point_no,
                            cur.transform_index,
                            attach_point,
                            true,
                        );
                        if !done {
                            self.detach(&mut cur, false);
                        }
                    }
                    if !done {
                        another_attachment_try = self.next_attach_try(&mut cur);
                    }
                }
            }

            if done {
                continue;
            }
            if self.nr_attached == 0 {
                done = true;
            } else if self.identical {
                // Undo back to a piece that was actually in the way. Anything
                // nearer the top of the stack was irrelevant to the failure and
                // putting it back differently would fail the same way.
                let mut detach_until = self.nr_attached - 1;
                if self.nr_attached < n {
                    while detach_until > 0 && !self.reason(self.nr_attached, detach_until) {
                        detach_until -= 1;
                    }
                }
                while self.nr_attached > detach_until {
                    if self.rot180 {
                        self.detach(&mut cur, true);
                    }
                    self.detach(&mut cur, false);
                    let step = self.nr_attached + 1 + usize::from(self.rot180);
                    if step < n {
                        for i in 0..n {
                            let row = self.nr_attached;
                            let v = self.reason(row, i) | self.reason(step, i);
                            self.set_reason(row, i, v);
                        }
                    }
                }
            } else {
                if self.rot180 {
                    self.detach(&mut cur, true);
                }
                self.detach(&mut cur, false);
            }
            if !done {
                another_attachment_try = self.next_attach_try(&mut cur);
            }
        }

        if self.use_bitmaps {
            self.draw_with_bitmaps(d);
        } else {
            self.draw_without_bitmaps(d);
        }

        self.wait = if self.nr_attached == n { 100 } else { 0 };
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.reset(d);
    }
}

/// The one-sided sets, where a piece and its mirror image count as different
/// pieces. A shape that has reflections among its symmetries becomes two
/// pieces: one keeping the rotations, one keeping the reflections. The split
/// point is where the symmetry list crosses from one to the other, which is
/// why the tables keep them in that order.
fn one_sided(shapes: &'static [Shape]) -> Vec<Shape> {
    let mut out = Vec::new();
    for s in shapes {
        match s.transforms.iter().position(|&t| t >= 4) {
            Some(t) => {
                out.push(Shape {
                    points: s.points,
                    transforms: &s.transforms[..t],
                    max_white: s.max_white,
                });
                out.push(Shape {
                    points: s.points,
                    transforms: &s.transforms[t..],
                    max_white: s.max_white,
                });
            }
            None => out.push(Shape {
                points: s.points,
                transforms: s.transforms,
                max_white: s.max_white,
            }),
        }
    }
    out
}

static PENTOMINO1: Shape = Shape {
    points: &[(0, 0), (1, 0), (2, 0), (3, 0), (1, 1)],
    transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
    max_white: 3,
};

static HEXOMINO1: Shape = Shape {
    points: &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (1, 1)],
    transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
    max_white: 4,
};

static HEPTOMINO1: Shape = Shape {
    points: &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (1, 1), (2, 1)],
    transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
    max_white: 4,
};

static ELEVENOMINO1: Shape = Shape {
    points: &[
        (0, 0),
        (1, 0),
        (2, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 1),
        (0, 2),
        (1, 2),
        (2, 2),
        (3, 2),
    ],
    transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
    max_white: 6,
};

static DEKOMINO1: Shape = Shape {
    points: &[
        (1, -1),
        (1, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 1),
        (0, 2),
        (1, 2),
        (2, 2),
        (3, 2),
    ],
    transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
    max_white: 5,
};

static OCTOMINO1: Shape = Shape {
    points: &[
        (1, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (0, 2),
        (1, 2),
        (2, 2),
        (3, 2),
    ],
    transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
    max_white: 5,
};

static TETROMINO: [Shape; 5] = [
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0)],
        transforms: &[0, 1],
        max_white: 2,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 2,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 0)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 1)],
        transforms: &[0, 1, 4, 5],
        max_white: 2,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (1, 1)],
        transforms: &[0],
        max_white: 2,
    },
];

static PENTOMINO: [Shape; 12] = [
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)],
        transforms: &[0, 1],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (3, 0)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, -1), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (2, 2)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, -1), (2, 0)],
        transforms: &[0, 1, 4, 5],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (1, 1), (2, 0)],
        transforms: &[0],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 1), (2, 2)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
];

static HEXOMINO: [Shape; 35] = [
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0)],
        transforms: &[0, 1],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (4, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (3, 1), (4, 0)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (3, 0), (4, 0)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, -1), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (3, 1), (4, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (3, 0), (3, 1), (3, 2)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, -1), (2, 0), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 0), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (2, 0), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, 0), (3, 0), (3, 1)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, 0), (3, -1), (3, 0)],
        transforms: &[0, 1, 4, 5],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, -1), (2, 0), (2, 1), (3, 0)],
        transforms: &[0, 1, 2, 3],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (3, 0)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (3, 0)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (2, 0), (2, 1), (3, 0)],
        transforms: &[0, 1, 4, 5],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, -1), (2, 0), (2, 1), (3, -1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (2, -1), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, -1), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (3, 1), (4, 1)],
        transforms: &[0, 1, 4, 5],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (3, 1), (3, 2)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (3, 1)],
        transforms: &[0, 1, 4, 5],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (2, 0), (2, 1), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, 0), (2, 1), (3, 1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (2, 2)],
        transforms: &[0, 1, 2, 3],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (1, -1), (1, 0), (1, 1), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3],
        max_white: 4,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)],
        transforms: &[0, 1],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (3, 2)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 2), (2, 0), (2, 1), (2, 2)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, -1), (1, 0), (2, 0), (2, 1)],
        transforms: &[0, 1, 2, 3],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, 0), (2, -1), (2, 0), (3, -1)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (0, 1), (1, -1), (1, 0), (2, -1), (2, 0)],
        transforms: &[0, 1, 2, 3, 4, 5, 6, 7],
        max_white: 3,
    },
    Shape {
        points: &[(0, 0), (1, 0), (1, 1), (2, 1), (2, 2), (3, 2)],
        transforms: &[0, 1, 4, 5],
        max_white: 3,
    },
];

const DEFAULTS: &[&str] = &[
    "*delay: 10000",
    "*cycles: 2000",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*identical: False",
    "*plain: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("cycles", "Duration", 500.0, 5000.0, 100.0, 0, "2000"),
    Opt::slider("ncolors", "Number of colors", 2.0, 255.0, 1.0, 0, "64"),
    Opt::boolean("identical", "Identical pieces", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "polyominoes",
    label: "Polyominoes",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Stephen Montgomery-Smith",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=6j2H2gL8cws"),
        blurb: "Attempts to fill a rectangle with irregular puzzle pieces.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// A dropped shape or a miscounted set would still draw something, and the
    /// generic tests would pass, so check the arithmetic the puzzles rely on:
    /// the pieces have to cover the board exactly.
    #[test]
    fn every_set_covers_its_board() {
        for (set, size) in [(&TETROMINO[..], 4), (&PENTOMINO[..], 5), (&HEXOMINO[..], 6)] {
            for s in set {
                assert_eq!(s.points.len(), size, "a shape has the wrong square count");
                assert!(!s.transforms.is_empty());
                assert!(s.transforms.iter().all(|&t| t < 8));
            }
        }

        // The one-sided sets count a piece and its mirror image separately.
        // Eighteen pentominoes fill 30x3, sixty hexominoes fill 20x18.
        assert_eq!(one_sided(&PENTOMINO).len() * 5, 30 * 3);
        assert_eq!(one_sided(&HEXOMINO).len() * 6, 20 * 18);
        // Neither half of a split may come out empty, or that piece could never
        // be placed and the puzzle could never be finished.
        for s in one_sided(&PENTOMINO)
            .iter()
            .chain(one_sided(&HEXOMINO).iter())
        {
            assert!(!s.transforms.is_empty());
        }

        // The two mixed puzzles, and the plain twelve.
        assert_eq!(PENTOMINO.len() * 5, 20 * 3);
        assert_eq!(TETROMINO.len() * 4 + PENTOMINO.len() * 5, 20 * 4);
        assert_eq!(PENTOMINO.len() * 5 + HEXOMINO.len() * 6, 54 * 5);
    }

    /// The repeated-piece puzzles are stated with a board each: the same check
    /// for those, plus the shapes themselves.
    #[test]
    fn every_repeated_piece_covers_its_board() {
        for (shape, size, n, w, h) in [
            (&PENTOMINO1, 5, 10, 10, 5),
            (&HEXOMINO1, 6, 92, 24, 23),
            (&HEPTOMINO1, 7, 78, 26, 21),
            (&HEPTOMINO1, 7, 76, 28, 19),
            (&ELEVENOMINO1, 11, 50, 25, 22),
            (&DEKOMINO1, 10, 96, 32, 30),
            (&OCTOMINO1, 8, 312, 96, 26),
            (&PENTOMINO1, 5, 45, 15, 15),
            (&ELEVENOMINO1, 11, 141, 47, 33),
        ] {
            assert_eq!(shape.points.len(), size);
            assert_eq!(n * size, w * h, "{n} pieces of {size} do not fill {w}x{h}");
        }
    }
}
