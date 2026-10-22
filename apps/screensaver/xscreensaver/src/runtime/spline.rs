//! Port of `utils/spline.c`.
//!
//! ```text
//! Copyright (c) 1987, 1988, 1989 Stanford University
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided
//! that the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation, and that the name of Stanford not be used in advertising or
//! publicity pertaining to distribution of the software without specific,
//! written prior permission.  Stanford makes no representations about
//! the suitability of this software for any purpose.  It is provided "as is"
//! without express or implied warranty.
//!
//! STANFORD DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,
//! INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.
//! IN NO EVENT SHALL STANFORD BE LIABLE FOR ANY SPECIAL, INDIRECT OR
//! CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE,
//! DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR
//! OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION
//! WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//!
//! This code came with the InterViews distribution, and was translated
//! from C++ to C by Matthieu Devin <devin@lucid.com> some time in 1992.
//! ```
//!
//! A cardinal spline through a list of control points, flattened into a
//! polyline. Each section between consecutive controls becomes a cubic Bézier,
//! which is then split in half over and over until each half is flat enough to
//! stand in for a straight line, so a gentle curve costs a couple of segments
//! and a tight one costs many.

use super::fb::XPoint;

/// How far a curve may stray from a line before it has to be split again.
const SMOOTHNESS: f64 = 1.0;

/// A ceiling on the subdivision, which upstream does without: its recursion
/// stops when halving no longer moves the control points, and on a machine
/// with a real stack that is enough.
const MAX_DEPTH: u32 = 32;

pub struct Spline {
    pub control_x: Vec<f64>,
    pub control_y: Vec<f64>,
    /// The flattened polyline, rebuilt by each `compute`.
    pub points: Vec<XPoint>,
}

fn mid_point(x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64) {
    ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
}

fn third_point(x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64) {
    ((2.0 * x0 + x1) / 3.0, (2.0 * y0 + y1) / 3.0)
}

/// Is the curve from (x0,y0) to (x3,y3) flat enough to draw as one line? The
/// test is four times the triangle's area against the length of its base.
fn can_approx_with_line(x0: f64, y0: f64, x2: f64, y2: f64, x3: f64, y3: f64) -> bool {
    let mut triangle_area = x0 * y2 - x2 * y0 + x2 * y3 - x3 * y2 + x3 * y0 - x0 * y3;
    triangle_area *= triangle_area;
    let dx = x3 - x0;
    let dy = y3 - y0;
    let side_squared = dx * dx + dy * dy;
    triangle_area <= SMOOTHNESS * side_squared
}

fn add_line(points: &mut Vec<XPoint>, x0: f64, y0: f64, x1: f64, y1: f64) {
    if points.is_empty() {
        points.push(XPoint {
            x: x0 as i32,
            y: y0 as i32,
        });
    }
    points.push(XPoint {
        x: x1 as i32,
        y: y1 as i32,
    });
}

fn add_bezier_arc(
    points: &mut Vec<XPoint>,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
    depth: u32,
) {
    let (midx01, midy01) = mid_point(x0, y0, x1, y1);
    let (midx12, midy12) = mid_point(x1, y1, x2, y2);
    let (midx23, midy23) = mid_point(x2, y2, x3, y3);
    let (midlsegx, midlsegy) = mid_point(midx01, midy01, midx12, midy12);
    let (midrsegx, midrsegy) = mid_point(midx12, midy12, midx23, midy23);
    let (cx, cy) = mid_point(midlsegx, midlsegy, midrsegx, midrsegy);

    if can_approx_with_line(x0, y0, midlsegx, midlsegy, cx, cy) {
        add_line(points, x0, y0, cx, cy);
    } else if depth < MAX_DEPTH
        && (midx01 != x1
            || midy01 != y1
            || midlsegx != x2
            || midlsegy != y2
            || cx != x3
            || cy != y3)
    {
        add_bezier_arc(
            points,
            x0,
            y0,
            midx01,
            midy01,
            midlsegx,
            midlsegy,
            cx,
            cy,
            depth + 1,
        );
    }

    if can_approx_with_line(cx, cy, midx23, midy23, x3, y3) {
        add_line(points, cx, cy, x3, y3);
    } else if depth < MAX_DEPTH
        && (cx != x0
            || cy != y0
            || midrsegx != x1
            || midrsegy != y1
            || midx23 != x2
            || midy23 != y2)
    {
        add_bezier_arc(
            points,
            cx,
            cy,
            midrsegx,
            midrsegy,
            midx23,
            midy23,
            x3,
            y3,
            depth + 1,
        );
    }
}

/// One section of the curve, from the control before it to the one after next.
fn calc_section(
    points: &mut Vec<XPoint>,
    cminus1x: f64,
    cminus1y: f64,
    cx: f64,
    cy: f64,
    cplus1x: f64,
    cplus1y: f64,
    cplus2x: f64,
    cplus2y: f64,
) {
    let (p1x, p1y) = third_point(cx, cy, cplus1x, cplus1y);
    let (p2x, p2y) = third_point(cplus1x, cplus1y, cx, cy);
    let (tempx, tempy) = third_point(cx, cy, cminus1x, cminus1y);
    let (p0x, p0y) = mid_point(tempx, tempy, p1x, p1y);
    let (tempx, tempy) = third_point(cplus1x, cplus1y, cplus2x, cplus2y);
    let (p3x, p3y) = mid_point(tempx, tempy, p2x, p2y);
    add_bezier_arc(points, p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, 0);
}

impl Spline {
    pub fn new(size: usize) -> Self {
        Self {
            control_x: vec![0.0; size],
            control_y: vec![0.0; size],
            points: Vec::with_capacity(size),
        }
    }

    pub fn n_controls(&self) -> usize {
        self.control_x.len()
    }

    /// `compute_spline`: an open curve, with the ends pinned to the first and
    /// last control points.
    pub fn compute(&mut self) {
        self.points.clear();
        let n = self.n_controls();
        if n < 3 {
            return;
        }
        let (cx, cy) = (&self.control_x, &self.control_y);
        let mut pts = std::mem::take(&mut self.points);

        calc_section(
            &mut pts, cx[0], cy[0], cx[0], cy[0], cx[0], cy[0], cx[1], cy[1],
        );
        calc_section(
            &mut pts, cx[0], cy[0], cx[0], cy[0], cx[1], cy[1], cx[2], cy[2],
        );
        for i in 1..n - 2 {
            calc_section(
                &mut pts,
                cx[i - 1],
                cy[i - 1],
                cx[i],
                cy[i],
                cx[i + 1],
                cy[i + 1],
                cx[i + 2],
                cy[i + 2],
            );
        }
        let i = n - 2;
        calc_section(
            &mut pts,
            cx[i - 1],
            cy[i - 1],
            cx[i],
            cy[i],
            cx[i + 1],
            cy[i + 1],
            cx[i + 1],
            cy[i + 1],
        );
        calc_section(
            &mut pts,
            cx[i],
            cy[i],
            cx[i + 1],
            cy[i + 1],
            cx[i + 1],
            cy[i + 1],
            cx[i + 1],
            cy[i + 1],
        );
        self.points = pts;
    }

    /// `compute_closed_spline`: the curve wraps back round to its first
    /// control point.
    pub fn compute_closed(&mut self) {
        self.points.clear();
        let n = self.n_controls();
        if n < 3 {
            return;
        }
        let (cx, cy) = (&self.control_x, &self.control_y);
        let mut pts = std::mem::take(&mut self.points);

        calc_section(
            &mut pts,
            cx[n - 1],
            cy[n - 1],
            cx[0],
            cy[0],
            cx[1],
            cy[1],
            cx[2],
            cy[2],
        );
        for i in 1..n - 2 {
            calc_section(
                &mut pts,
                cx[i - 1],
                cy[i - 1],
                cx[i],
                cy[i],
                cx[i + 1],
                cy[i + 1],
                cx[i + 2],
                cy[i + 2],
            );
        }
        let i = n - 2;
        calc_section(
            &mut pts,
            cx[i - 1],
            cy[i - 1],
            cx[i],
            cy[i],
            cx[i + 1],
            cy[i + 1],
            cx[0],
            cy[0],
        );
        calc_section(
            &mut pts,
            cx[i],
            cy[i],
            cx[i + 1],
            cy[i + 1],
            cx[0],
            cy[0],
            cx[1],
            cy[1],
        );
        self.points = pts;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(n: usize, r: f64) -> Spline {
        let mut s = Spline::new(n);
        for i in 0..n {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            s.control_x[i] = 100.0 + r * a.cos();
            s.control_y[i] = 100.0 + r * a.sin();
        }
        s
    }

    /// A closed spline through points on a circle should stay on that circle,
    /// which is the property everything drawn with one depends on.
    #[test]
    fn a_closed_spline_follows_its_controls() {
        let mut s = circle(12, 50.0);
        s.compute_closed();
        assert!(s.points.len() > 24, "only {} points", s.points.len());
        for p in &s.points {
            let dx = p.x as f64 - 100.0;
            let dy = p.y as f64 - 100.0;
            let r = (dx * dx + dy * dy).sqrt();
            assert!((r - 50.0).abs() < 4.0, "point at radius {r}");
        }
    }

    /// Tighter curves need more segments, which is the whole point of the
    /// flatness test.
    #[test]
    fn a_bigger_curve_costs_more_segments() {
        let mut small = circle(12, 10.0);
        small.compute_closed();
        let mut big = circle(12, 400.0);
        big.compute_closed();
        assert!(
            big.points.len() > small.points.len(),
            "{} vs {}",
            big.points.len(),
            small.points.len()
        );
    }

    #[test]
    fn too_few_controls_is_not_a_panic() {
        for n in 0..3 {
            let mut s = Spline::new(n);
            s.compute();
            s.compute_closed();
            assert!(s.points.is_empty());
        }
    }
}
