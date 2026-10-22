//! Port of `hacks/ccurve.c`.
//!
//! ```text
//! ccurve, Copyright (c) 1998, 1999
//!  Rick Campbell <rick@campbellcentral.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draw self-similar linear fractals including the classic ``C Curve''
//!
//! 16 Aug 1999  Rick Campbell <rick@campbellcentral.org>
//!      Eliminated sub-windows-with-backing-store-double-buffering crap in
//!      favor of drawing the new image in a pixmap and then splatting that on
//!      the window.
//!
//! 19 Dec 1998  Rick Campbell <rick@campbellcentral.org>
//!      Original version.
//! ```
//!
//! One rule: take a line, replace it with a short chain of lines, then do the
//! same to each of those. The chain is first normalised so that it runs from
//! the origin to one along the x-axis, which is what lets it be dropped onto
//! any line at any angle and scale. The Levy C curve is the case where the
//! chain is two segments at forty-five degrees; everything else here is that
//! idea with a different chain.
//!
//! The chain is picked at random from a menu of shapes, some fixed and some
//! with random angles and lengths, and the figure is then drawn once per
//! frame at one more level of recursion than the last, so it grows in front
//! of you rather than arriving finished. Colour runs along the curve rather
//! than across the picture: each line takes its colour from how far along the
//! sequence it is, so the hue sweeps once from start to end however deep the
//! recursion has got.
//!
//! The view is recomputed from the extent of the previous frame, not the
//! current one, so the figure is always framed one level behind and appears
//! to settle into place.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{BLACK, WHITE, XColor, make_color_loop};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
    screenhack_event_helper,
};

const MAXIMUM_COLOR_COUNT: usize = 256;
const EPSILON: f64 = 1e-5;
const FRAC_PI_4: f64 = std::f64::consts::FRAC_PI_4;
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;
const PI: f64 = std::f64::consts::PI;
const SQRT_2: f64 = std::f64::consts::SQRT_2;

#[derive(Clone, Copy, Default)]
struct Position {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Default)]
struct Segment {
    angle: f64,
    length: f64,
}

/// A value in `base..limit`, on a grid of `epsilon`.
fn random_double(base: f64, limit: f64, epsilon: f64) -> f64 {
    let range = limit - base;
    let steps = (range / epsilon).floor().max(1.0) as u32;
    base + (random() % steps) as f64 * epsilon
}

/// Alter the sequence to go from (0,0) to (1,0), so it can be dropped onto
/// any line at any angle and scale.
fn normalized_plot(segments: &[Segment], points: &mut [Position]) {
    let mut x = 0.0;
    let mut y = 0.0;
    for (i, segment) in segments.iter().enumerate() {
        x += segment.length * segment.angle.cos();
        y += segment.length * segment.angle.sin();
        points[i].x = x;
        points[i].y = y;
    }
    let angle = -(y.atan2(x));
    let (sine, cosine) = angle.sin_cos();
    let length = (x * x + y * y).sqrt();
    // Rotate and scale.
    for p in points.iter_mut() {
        let (tx, ty) = (p.x, p.y);
        p.x = (tx * cosine + ty * -sine) / length;
        p.y = (tx * sine + ty * cosine) / length;
    }
}

/// Put the normalised chain back onto the line from one point to the other.
fn realign(x1: f64, y1: f64, x2: f64, y2: f64, points: &mut [Position]) {
    let delta_x = x2 - x1;
    let delta_y = y2 - y1;
    let angle = delta_y.atan2(delta_x);
    let (sine, cosine) = angle.sin_cos();
    let length = (delta_x * delta_x + delta_y * delta_y).sqrt();
    // Rotate, scale, then shift.
    for p in points.iter_mut() {
        let (tx, ty) = (p.x, p.y);
        p.x = length * (tx * cosine + ty * -sine) + x1;
        p.y = length * (tx * sine + ty * cosine) + y1;
    }
}

fn select_2_pattern(segments: &mut [Segment]) {
    if random().is_multiple_of(2) {
        if random().is_multiple_of(2) {
            segments[0] = Segment {
                angle: -FRAC_PI_4,
                length: SQRT_2,
            };
            segments[1] = Segment {
                angle: FRAC_PI_4,
                length: SQRT_2,
            };
        } else {
            segments[0] = Segment {
                angle: FRAC_PI_4,
                length: SQRT_2,
            };
            segments[1] = Segment {
                angle: -FRAC_PI_4,
                length: SQRT_2,
            };
        }
    } else {
        segments[0].angle = random_double(PI / 6.0, PI / 3.0, PI / 180.0);
        segments[0].length = random_double(0.25, 0.67, 0.001);
        if random().is_multiple_of(2) {
            segments[1].angle = -segments[0].angle;
            segments[1].length = segments[0].length;
        } else {
            segments[1].angle = random_double(-PI / 3.0, -PI / 6.0, PI / 180.0);
            segments[1].length = random_double(0.25, 0.67, 0.001);
        }
    }
}

fn select_3_pattern(segments: &mut [Segment]) {
    match random() % 5 {
        0 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            segments[0] = Segment {
                angle: s * FRAC_PI_4,
                length: SQRT_2 / 4.0,
            };
            segments[1] = Segment {
                angle: -s * FRAC_PI_4,
                length: SQRT_2 / 2.0,
            };
            segments[2] = Segment {
                angle: s * FRAC_PI_4,
                length: SQRT_2 / 4.0,
            };
        }
        1 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            segments[0] = Segment {
                angle: s * PI / 6.0,
                length: 1.0,
            };
            segments[1] = Segment {
                angle: -s * FRAC_PI_2,
                length: 1.0,
            };
            segments[2] = Segment {
                angle: s * PI / 6.0,
                length: 1.0,
            };
        }
        // Three of the five cases are the same random chain.
        _ => {
            segments[0].angle = random_double(PI / 6.0, PI / 3.0, PI / 180.0);
            segments[0].length = random_double(0.25, 0.67, 0.001);
            segments[1].angle = random_double(-PI / 3.0, -PI / 6.0, PI / 180.0);
            segments[1].length = random_double(0.25, 0.67, 0.001);
            if random().is_multiple_of(3) {
                segments[2].angle = if random().is_multiple_of(2) {
                    segments[0].angle
                } else {
                    -segments[0].angle
                };
                segments[2].length = segments[0].length;
            } else {
                segments[2].angle = random_double(-PI / 3.0, -PI / 6.0, PI / 180.0);
                segments[2].length = random_double(0.25, 0.67, 0.001);
            }
        }
    }
}

fn select_4_pattern(segments: &mut [Segment]) {
    /// The shape shared by most of the four-segment cases: out, up, down,
    /// out, mirrored half the time.
    fn spike(segments: &mut [Segment], ends: f64, s: f64, angle: f64, length: f64) {
        segments[0] = Segment {
            angle: 0.0,
            length: ends,
        };
        segments[1] = Segment {
            angle: s * angle,
            length,
        };
        segments[2] = Segment {
            angle: -s * angle,
            length,
        };
        segments[3] = Segment {
            angle: 0.0,
            length: ends,
        };
    }

    match random() % 9 {
        0 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            let length = random_double(0.25, 0.50, 0.001);
            spike(segments, 0.5, s, FRAC_PI_2, length);
        }
        1 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            spike(segments, 0.5, s, FRAC_PI_2, 0.45);
        }
        2 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            spike(segments, 1.0, s, (5.0 * PI) / 12.0, 1.2);
        }
        // Cases three and four are the same block twice over in the source.
        3 | 4 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            let angle = random_double(PI / 4.0, FRAC_PI_2, PI / 180.0);
            spike(segments, 1.0, s, angle, 1.2);
        }
        5 => {
            let s = if random().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            let angle = random_double(PI / 4.0, FRAC_PI_2, PI / 180.0);
            let length = random_double(0.25, 0.50, 0.001);
            spike(segments, 1.0, s, angle, length);
        }
        // The last three cases are the same free-for-all.
        _ => {
            segments[0].angle = random_double(PI / 12.0, (11.0 * PI) / 12.0, 0.001);
            segments[0].length = random_double(0.25, 0.50, 0.001);
            segments[1].angle = random_double(PI / 12.0, (11.0 * PI) / 12.0, 0.001);
            segments[1].length = random_double(0.25, 0.50, 0.001);
            if random().is_multiple_of(3) {
                segments[2].angle = random_double(PI / 12.0, (11.0 * PI) / 12.0, 0.001);
                segments[2].length = random_double(0.25, 0.50, 0.001);
                segments[3].angle = random_double(PI / 12.0, (11.0 * PI) / 12.0, 0.001);
                segments[3].length = random_double(0.25, 0.50, 0.001);
            } else {
                let s = if random().is_multiple_of(2) {
                    -1.0
                } else {
                    1.0
                };
                segments[2].angle = s * segments[1].angle;
                segments[2].length = segments[1].length;
                segments[3].angle = s * segments[0].angle;
                segments[3].length = segments[0].length;
            }
        }
    }
}

fn select_pattern(segments: &mut [Segment]) {
    match segments.len() {
        2 => select_2_pattern(segments),
        3 => select_3_pattern(segments),
        _ => select_4_pattern(segments),
    }
}

struct State {
    color_count: usize,
    colors: Vec<XColor>,
    line_count: i32,
    maximum_lines: i32,
    plot_maximum_x: f64,
    plot_maximum_y: f64,
    plot_minimum_x: f64,
    plot_minimum_y: f64,
    total_lines: i32,
    background: crate::runtime::Pixel,
    gc: Gc,
    width: i32,
    height: i32,
    /// Both delays are in seconds: one between finished figures, one between
    /// the frames that build one up.
    delay: f64,
    delay2: f64,

    /// How deep the recursion goes this frame.
    draw_index: i32,
    draw_iterations: i32,
    draw_maximum_x: f64,
    draw_maximum_y: f64,
    draw_minimum_x: f64,
    draw_minimum_y: f64,
    draw_segments: Vec<Segment>,
    draw_x1: f64,
    draw_y1: f64,
    draw_x2: f64,
    draw_y2: f64,
}

impl State {
    /// The recursion. At the bottom it draws one line; above it, it drops the
    /// normalised chain onto the line and recurses into each piece.
    #[allow(clippy::too_many_arguments)]
    fn self_similar_normalized(
        &mut self,
        d: &mut Dpy,
        iterations: i32,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        maximum_x: f64,
        maximum_y: f64,
        minimum_x: f64,
        minimum_y: f64,
        points: &[Position],
    ) -> bool {
        if iterations == 0 {
            let delta_x = maximum_x - minimum_x;
            let delta_y = maximum_y - minimum_y;
            let color_index = ((self.line_count as f64 * self.color_count as f64)
                / self.total_lines.max(1) as f64) as usize;
            self.line_count += 1;
            self.gc
                .set_foreground(self.colors[color_index.min(self.colors.len() - 1)].pixel);

            self.plot_maximum_x = self.plot_maximum_x.max(x1).max(x2);
            self.plot_maximum_y = self.plot_maximum_y.max(y1).max(y2);
            self.plot_minimum_x = self.plot_minimum_x.min(x1).min(x2);
            self.plot_minimum_y = self.plot_minimum_y.min(y1).min(y2);

            d.win().draw_line(
                &self.gc,
                (((x1 - minimum_x) / delta_x) * self.width as f64) as i32,
                (((maximum_y - y1) / delta_y) * self.height as f64) as i32,
                (((x2 - minimum_x) / delta_x) * self.width as f64) as i32,
                (((maximum_y - y2) / delta_y) * self.height as f64) as i32,
            );
            return true;
        }

        let mut replacement: Vec<Position> = points.to_vec();
        realign(x1, y1, x2, y2, &mut replacement);

        // jwz: I do not understand what these assertions are supposed to be
        // detecting, but let us just bail on the fractal instead of crashing.
        let last = replacement[replacement.len() - 1];
        if (x2 - last.x).abs() >= EPSILON || (y2 - last.y).abs() >= EPSILON {
            return false;
        }

        let mut x = x1;
        let mut y = y1;
        for p in &replacement {
            let (next_x, next_y) = (p.x, p.y);
            if !self.self_similar_normalized(
                d,
                iterations - 1,
                x,
                y,
                next_x,
                next_y,
                maximum_x,
                maximum_y,
                minimum_x,
                minimum_y,
                points,
            ) {
                return false;
            }
            x = next_x;
            y = next_y;
        }
        true
    }

    fn self_similar(&mut self, d: &mut Dpy, iterations: i32) {
        let mut points = vec![Position::default(); self.draw_segments.len()];
        normalized_plot(&self.draw_segments, &mut points);
        let (x1, y1, x2, y2) = (self.draw_x1, self.draw_y1, self.draw_x2, self.draw_y2);
        let (mx, my, nx, ny) = (
            self.draw_maximum_x,
            self.draw_maximum_y,
            self.draw_minimum_x,
            self.draw_minimum_y,
        );
        self.self_similar_normalized(d, iterations, x1, y1, x2, y2, mx, my, nx, ny, &points);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut colors = make_color_loop(
        0,
        1.0,
        1.0,
        120,
        1.0,
        1.0,
        240,
        1.0,
        1.0,
        MAXIMUM_COLOR_COUNT,
    );
    if colors.is_empty() {
        colors.push(XColor::from_rgb16(0xFFFF, 0xFFFF, 0xFFFF));
    }
    Box::new(State {
        color_count: colors.len(),
        colors,
        line_count: 0,
        maximum_lines: d.res.int("limit").max(2),
        plot_maximum_x: 0.0,
        plot_maximum_y: 0.0,
        plot_minimum_x: 0.0,
        plot_minimum_y: 0.0,
        total_lines: 1,
        background: BLACK,
        gc: Gc::new(WHITE, BLACK),
        width: d.width(),
        height: d.height(),
        delay: d.res.float("delay"),
        delay2: d.res.float("pause"),
        draw_index: 0,
        draw_iterations: 0,
        draw_maximum_x: 1.20,
        draw_maximum_y: 0.525,
        draw_minimum_x: -0.20,
        draw_minimum_y: -0.525,
        draw_segments: Vec::new(),
        draw_x1: 0.0,
        draw_y1: 0.0,
        draw_x2: 1.0,
        draw_y2: 0.0,
    })
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        const LENGTHS: [usize; 9] = [4, 4, 4, 4, 4, 3, 3, 3, 2];

        if self.draw_index == 0 {
            let segment_count = LENGTHS[(random() % LENGTHS.len() as u32) as usize];
            self.draw_segments = vec![Segment::default(); segment_count];
            select_pattern(&mut self.draw_segments);
            self.draw_iterations =
                ((self.maximum_lines as f64).ln() / (segment_count as f64).ln()).floor() as i32;

            // Two times out of three, shift the ends of the starting line, so
            // the same chain gives a different figure.
            if !random().is_multiple_of(3) {
                let factor = 0.45;
                self.draw_x1 += random_double(-factor, factor, 0.001);
                self.draw_y1 += random_double(-factor, factor, 0.001);
                self.draw_x2 += random_double(-factor, factor, 0.001);
                self.draw_y2 += random_double(-factor, factor, 0.001);
            }
        }

        self.gc.set_foreground(self.background);
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        self.line_count = 0;
        self.total_lines = (self.draw_segments.len() as f64).powi(self.draw_index) as i32;
        self.plot_maximum_x = -1000.00;
        self.plot_maximum_y = -1000.00;
        self.plot_minimum_x = 1000.00;
        self.plot_minimum_y = 1000.00;

        let iterations = self.draw_index;
        self.self_similar(d, iterations);

        // The view for the next frame comes from the extent of this one, with
        // a fifth of the span added as a margin, then squared up to the window.
        let mut delta_x = self.plot_maximum_x - self.plot_minimum_x;
        let mut delta_y = self.plot_maximum_y - self.plot_minimum_y;
        self.draw_maximum_x = self.plot_maximum_x + delta_x * 0.2;
        self.draw_maximum_y = self.plot_maximum_y + delta_y * 0.2;
        self.draw_minimum_x = self.plot_minimum_x - delta_x * 0.2;
        self.draw_minimum_y = self.plot_minimum_y - delta_y * 0.2;
        delta_x = self.draw_maximum_x - self.draw_minimum_x;
        delta_y = self.draw_maximum_y - self.draw_minimum_y;
        if delta_y / delta_x > self.height as f64 / self.width as f64 {
            let new_delta_x = delta_y * self.width as f64 / self.height as f64;
            self.draw_minimum_x -= (new_delta_x - delta_x) / 2.0;
            self.draw_maximum_x += (new_delta_x - delta_x) / 2.0;
        } else {
            let new_delta_y = delta_x * self.height as f64 / self.width as f64;
            self.draw_minimum_y -= (new_delta_y - delta_y) / 2.0;
            self.draw_maximum_y += (new_delta_y - delta_y) / 2.0;
        }

        self.draw_index += 1;
        if self.draw_index >= self.draw_iterations {
            self.draw_index = 0;
            self.draw_segments.clear();
            (1_000_000.0 * self.delay) as u32
        } else {
            (1_000_000.0 * self.delay2) as u32
        }
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.draw_index = 0;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    ".delay: 3",
    ".pause: 0.4",
    ".limit: 200000",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Change image every", 0.0, 30.0, 1.0, 0, "3"),
    Opt::slider("pause", "Animation speed", 0.0, 5.0, 0.1, 1, "0.4").inverted(),
    Opt::slider("limit", "Density", 3.0, 300_000.0, 1000.0, 0, "200000"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "ccurve",
    label: "C Curve",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Rick Campbell",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=zqIlWzUHOz8"),
        blurb: "Generates self-similar linear fractals, including the classic C Curve.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
