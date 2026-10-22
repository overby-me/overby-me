//! Port of `utils/erase.c`.
//!
//! ```text
//! erase.c: Erase the screen in various more or less interesting ways.
//! Copyright (c) 1997-2001, 2006 Jamie Zawinski <jwz@jwz.org>
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
//! A hack that finishes a picture calls `erase_window` once per frame until it
//! returns nothing, and the screen wipes itself away over `eraseSeconds`. The
//! wipe is driven by a ratio of elapsed time, not by a frame count, so it takes
//! the same wall-clock time however fast the hack is running.
//!
//! Upstream has twelve wipes; five are ported so far, picked to cover the
//! distinct shapes (line sweep, slat sweep, radial, multi-radial, and a
//! self-scrolling one that exercises overlapped `XCopyArea`). Adding the rest
//! is mechanical.

use super::rand::random;
use super::{Dpy, Gc};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    RandomLines,
    Venetian,
    CircleWipe,
    ThreeCircleWipe,
    SlideLines,
}

const MODES: &[Mode] = &[
    Mode::RandomLines,
    Mode::Venetian,
    Mode::CircleWipe,
    Mode::ThreeCircleWipe,
    Mode::SlideLines,
];

/// An erase in progress.
pub struct Eraser {
    mode: Mode,
    fg_gc: Gc,
    bg_gc: Gc,
    width: i32,
    height: i32,

    start_time: f64,
    stop_time: f64,
    ratio: f64,
    prev_ratio: f64,

    // random_lines, venetian
    horiz_p: bool,
    flip_p: bool,
    lines: Vec<i32>,

    // circle_wipe, three_circle_wipe
    start: i32,
}

impl Eraser {
    fn new(d: &Dpy) -> Self {
        let fg = d.res.pixel("foreground");
        let bg = d.res.pixel("background");
        let mut duration = d.res.float("eraseSeconds");
        if !(0.1..=10.0).contains(&duration) {
            duration = 1.0;
        }
        Self {
            mode: MODES[(random() as usize) % MODES.len()],
            fg_gc: Gc::new(fg, bg),
            bg_gc: Gc::new(bg, fg),
            width: d.width(),
            height: d.height(),
            start_time: d.time,
            stop_time: d.time + duration,
            ratio: 0.0,
            prev_ratio: 0.0,
            horiz_p: false,
            flip_p: false,
            lines: Vec::new(),
            start: 0,
        }
    }

    /// The half-open span of line indices this frame is responsible for.
    fn line_span(&self) -> (usize, usize) {
        let n = self.lines.len() as f64;
        let from = (n * self.prev_ratio).max(0.0) as usize;
        let to = ((n * self.ratio) as usize).min(self.lines.len());
        (from.min(to), to)
    }

    fn draw(&mut self, d: &mut Dpy) {
        match self.mode {
            Mode::RandomLines => self.random_lines(d),
            Mode::Venetian => self.venetian(d),
            Mode::CircleWipe => self.circle_wipe(d),
            Mode::ThreeCircleWipe => self.three_circle_wipe(d),
            Mode::SlideLines => self.slide_lines(d),
        }
    }

    fn random_lines(&mut self, d: &mut Dpy) {
        if self.lines.is_empty() {
            self.horiz_p = random() & 1 == 1;
            let n = if self.horiz_p {
                self.height
            } else {
                self.width
            };
            self.lines = (0..n).collect();
            for i in 0..self.lines.len() {
                let r = (random() as usize) % self.lines.len();
                self.lines.swap(i, r);
            }
        }
        let (from, to) = self.line_span();
        for i in from..to {
            let l = self.lines[i];
            if self.horiz_p {
                d.win().draw_line(&self.bg_gc, 0, l, self.width, l);
            } else {
                d.win().draw_line(&self.bg_gc, l, 0, l, self.height);
            }
        }
    }

    fn venetian(&mut self, d: &mut Dpy) {
        if self.lines.is_empty() {
            self.horiz_p = random() & 1 == 1;
            self.flip_p = random() & 1 == 1;
            let n = if self.horiz_p {
                self.height
            } else {
                self.width
            };
            // Sixteen interleaved slats, each sweeping in turn.
            for i in 0..n * 2 {
                let line = ((i / 16) * 16) - ((i % 16) * 15);
                if line >= 0 && line < n {
                    self.lines.push(if self.flip_p { n - line } else { line });
                }
            }
        }
        let (from, to) = self.line_span();
        for i in from..to {
            let l = self.lines[i];
            if self.horiz_p {
                d.win().draw_line(&self.bg_gc, 0, l, self.width, l);
            } else {
                d.win().draw_line(&self.bg_gc, l, 0, l, self.height);
            }
        }
    }

    fn circle_wipe(&mut self, d: &mut Dpy) {
        let rad = self.width.max(self.height);
        let max = super::fb::FULL_CIRCLE;
        if self.ratio == 0.0 {
            self.flip_p = random() & 1 == 1;
            self.start = (random() % max as u32) as i32;
        }
        let mut th = (max as f64 * self.ratio) as i32;
        let mut oth = (max as f64 * self.prev_ratio) as i32;
        if self.flip_p {
            th = max - th;
            oth = max - oth;
        }
        d.win().fill_arc(
            &self.bg_gc,
            (self.width / 2) - rad,
            (self.height / 2) - rad,
            rad * 2,
            rad * 2,
            (self.start + oth).rem_euclid(max),
            th - oth,
        );
    }

    fn three_circle_wipe(&mut self, d: &mut Dpy) {
        let rad = self.width.max(self.height);
        let max = super::fb::FULL_CIRCLE;
        if self.ratio == 0.0 {
            self.start = (random() % max as u32) as i32;
        }
        let th = (max as f64 / 6.0 * self.ratio) as i32;
        let oth = (max as f64 / 6.0 * self.prev_ratio) as i32;
        for i in 0..3 {
            let off = i * max / 3;
            let (x, y, w, h) = (
                (self.width / 2) - rad,
                (self.height / 2) - rad,
                rad * 2,
                rad * 2,
            );
            d.win().fill_arc(
                &self.bg_gc,
                x,
                y,
                w,
                h,
                (self.start + off + oth).rem_euclid(max),
                th - oth,
            );
            d.win().fill_arc(
                &self.bg_gc,
                x,
                y,
                w,
                h,
                (self.start + off - oth).rem_euclid(max),
                oth - th,
            );
        }
    }

    fn slide_lines(&mut self, d: &mut Dpy) {
        let max = (self.width as f64 * 1.1) as i32;
        let nlines = 40;
        let h = (self.height / nlines).max(10);
        let mut step = (max as f64 * self.ratio) as i32 - (max as f64 * self.prev_ratio) as i32;
        if step <= 0 {
            step = 1;
        }
        if self.width <= step {
            return;
        }
        let mut tick = 0;
        let mut y = 0;
        while y < self.height {
            if tick & 1 == 1 {
                d.win()
                    .copy_area_self(&self.fg_gc, 0, y, self.width - step, h, step, y);
                d.win().fill_rectangle(&self.bg_gc, 0, y, step, h);
            } else {
                d.win()
                    .copy_area_self(&self.fg_gc, step, y, self.width - step, h, 0, y);
                d.win()
                    .fill_rectangle(&self.bg_gc, self.width - step, y, step, h);
            }
            tick += 1;
            y += h;
        }
    }
}

/// `erase_window`: advance the wipe by one frame.
///
/// Pass `None` to start one; keep passing back what you get until it returns
/// `None`, which is when the screen is clear.
pub fn erase_window(d: &mut Dpy, state: Option<Eraser>) -> Option<Eraser> {
    let mut st = match state {
        Some(st) => st,
        None => Eraser::new(d),
    };

    let duration = st.stop_time - st.start_time;
    st.prev_ratio = st.ratio;
    st.ratio = if duration > 0.0 {
        (d.time - st.start_time) / duration
    } else {
        1.0
    };

    if st.ratio < 1.0 {
        st.draw(d);
        Some(st)
    } else {
        // The last pass is black on black, so just clear.
        d.clear_window();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::color::{ALPHA, WHITE};
    use crate::runtime::opts::Resources;
    use crate::runtime::rand::ya_rand_init;
    use crate::runtime::{Dpy, Gc};

    const DEFAULTS: &[&str] = &[
        ".background: black",
        ".foreground: white",
        "*eraseSeconds: 1",
    ];

    fn white_screen() -> Dpy {
        let mut d = Dpy::new(80, 60, Resources::new(DEFAULTS, &[], ""));
        let gc = Gc::new(WHITE, ALPHA);
        d.win().fill_rectangle(&gc, 0, 0, 80, 60);
        d
    }

    fn run_to_completion(seed: u32) -> (usize, Dpy) {
        ya_rand_init(seed);
        let mut d = white_screen();
        let mut st = None;
        let mut frames = 0;
        loop {
            d.time = frames as f64 / 60.0;
            st = erase_window(&mut d, st);
            frames += 1;
            if st.is_none() {
                break;
            }
            assert!(frames < 1000, "erase never finished");
        }
        (frames, d)
    }

    #[test]
    fn every_mode_finishes_and_leaves_a_clear_screen() {
        // Seeds chosen to walk across the mode table.
        for seed in 1..=20 {
            let (frames, d) = run_to_completion(seed);
            assert!(frames > 1, "seed {seed} finished instantly");
            assert!(
                d.win_ref().pixels().iter().all(|p| *p == ALPHA),
                "seed {seed} left something behind"
            );
        }
    }

    #[test]
    fn erasing_actually_removes_pixels_partway_through() {
        ya_rand_init(3);
        let mut d = white_screen();
        let before = d.win_ref().pixels().iter().filter(|p| **p == WHITE).count();
        let mut st = erase_window(&mut d, None);
        for i in 1..30 {
            d.time = i as f64 / 60.0;
            st = erase_window(&mut d, st);
            if st.is_none() {
                break;
            }
        }
        let after = d.win_ref().pixels().iter().filter(|p| **p == WHITE).count();
        assert!(after < before, "nothing was erased: {before} -> {after}");
    }

    #[test]
    fn a_zero_length_erase_still_terminates() {
        ya_rand_init(1);
        let mut d = Dpy::new(
            40,
            40,
            Resources::new(&[".background: black", "*eraseSeconds: 0"], &[], ""),
        );
        // Out-of-range durations fall back to one second upstream, so this
        // should behave like any other erase rather than dividing by zero.
        let st = erase_window(&mut d, None);
        assert!(st.is_some());
    }
}
