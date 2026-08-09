//! The drawable: a pixel buffer plus the Xlib drawing primitives, in software.
//!
//! Upstream draws into an X `Drawable`, which is a `Window`, a `Pixmap` or an
//! `XImage` depending on the call. Here all three are the same thing, an [`Fb`],
//! which is why `XGetImage` / `XPutImage` / `XCopyArea` collapse into plain
//! memory moves and why a hack can read its own output back for free.
//!
//! Coordinates are `i32` and everything clips, so a hack that computes an
//! off-screen point (many do, deliberately) draws nothing rather than panicking.

use super::color::{ALPHA, Pixel, RGB_MASK, WHITE};

/// A `Pixmap`. Same representation as the window.
pub type Pixmap = Fb;
/// An `XImage`. Same representation as the window.
pub type XImage = Fb;

/// `XPoint`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XPoint {
    pub x: i32,
    pub y: i32,
}

/// `XRectangle`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XRectangle {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `XSegment`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XSegment {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

/// `XArc`. Angles are in units of 1/64 degree, measured counter-clockwise from
/// three o'clock, exactly as X does it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XArc {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub angle1: i32,
    pub angle2: i32,
}

/// A full circle, in the units `XArc::angle2` wants.
pub const FULL_CIRCLE: i32 = 360 * 64;

/// The GC's raster operation (`XSetFunction`).
///
/// Only the operations the hacks actually reach for. Alpha is never touched:
/// every result is forced opaque, so XOR of two visible colours stays visible
/// instead of producing a transparent pixel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GXFunc {
    #[default]
    Copy,
    Xor,
    And,
    Or,
    AndInverted,
    OrInverted,
    Invert,
    Set,
    Clear,
    Nop,
}

impl GXFunc {
    /// The same operation on a single bit, for depth-1 drawables. There is no
    /// alpha to preserve, so the result is just 0 or 1.
    #[inline]
    fn apply_bit(self, dst: Pixel, src: Pixel) -> Pixel {
        let (d, s) = (dst & 1, src & 1);
        let v = match self {
            GXFunc::Copy => s,
            GXFunc::Xor => d ^ s,
            GXFunc::And => d & s,
            GXFunc::Or => d | s,
            GXFunc::AndInverted => (!d) & s,
            GXFunc::OrInverted => (!d) | s,
            GXFunc::Invert => !d,
            GXFunc::Set => 1,
            GXFunc::Clear => 0,
            GXFunc::Nop => d,
        };
        v & 1
    }

    #[inline]
    fn apply(self, dst: Pixel, src: Pixel) -> Pixel {
        let (d, s) = (dst & RGB_MASK, src & RGB_MASK);
        let v = match self {
            GXFunc::Copy => s,
            GXFunc::Xor => d ^ s,
            GXFunc::And => d & s,
            GXFunc::Or => d | s,
            GXFunc::AndInverted => (!d) & s,
            GXFunc::OrInverted => (!d) | s,
            GXFunc::Invert => !d,
            GXFunc::Set => RGB_MASK,
            GXFunc::Clear => 0,
            GXFunc::Nop => d,
        };
        (v & RGB_MASK) | ALPHA
    }
}

/// How `fill_polygon` decides what is inside.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FillRule {
    #[default]
    EvenOdd,
    Winding,
}

/// A graphics context.
///
/// Upstream a `GC` is a server-side handle mutated through `XChangeGC`; here it
/// is a plain value the hack owns, which is both simpler and easier on the
/// borrow checker (`d.win().fill_rectangle(&st.gc, ..)` borrows the two halves
/// separately).
#[derive(Clone, Debug)]
pub struct Gc {
    pub foreground: Pixel,
    pub background: Pixel,
    pub function: GXFunc,
    pub line_width: i32,
    pub fill_rule: FillRule,
    /// `XSetClipMask` / `XSetClipRectangles`, reduced to a single rectangle.
    /// Arbitrary bitmap clip masks are not supported yet; no ported hack needs
    /// one so far, and the ones that will (`blitspin`, `slidescreen`) can grow
    /// it when they land.
    pub clip: Option<XRectangle>,
}

impl Default for Gc {
    fn default() -> Self {
        Self {
            foreground: WHITE,
            background: ALPHA,
            function: GXFunc::Copy,
            line_width: 0,
            fill_rule: FillRule::EvenOdd,
            clip: None,
        }
    }
}

impl Gc {
    pub fn new(foreground: Pixel, background: Pixel) -> Self {
        Self {
            foreground,
            background,
            ..Self::default()
        }
    }

    /// `XSetForeground`.
    pub fn set_foreground(&mut self, p: Pixel) -> &mut Self {
        self.foreground = p;
        self
    }

    /// `XSetBackground`.
    pub fn set_background(&mut self, p: Pixel) -> &mut Self {
        self.background = p;
        self
    }

    /// `XSetFunction`.
    pub fn set_function(&mut self, f: GXFunc) -> &mut Self {
        self.function = f;
        self
    }

    /// `XSetLineAttributes`. Only the width is honoured; X's line style, cap
    /// and join are cosmetic at the widths the hacks use.
    pub fn set_line_width(&mut self, w: i32) -> &mut Self {
        self.line_width = w;
        self
    }

    /// `XSetClipMask (.., None)`.
    pub fn set_clip_none(&mut self) -> &mut Self {
        self.clip = None;
        self
    }

    /// `XSetClipRectangles` with a single rectangle.
    pub fn set_clip_rect(&mut self, r: XRectangle) -> &mut Self {
        self.clip = Some(r);
        self
    }
}

/// A drawable: window, pixmap or image.
///
/// Almost always full colour, but X also has depth-1 drawables (bitmaps), and
/// ten of the hacks build their picture in one before blitting it through
/// [`Fb::copy_plane`]. A depth-1 `Fb` stores 0 or 1 per pixel rather than a
/// colour, and its raster operations work on that single bit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fb {
    width: i32,
    height: i32,
    px: Vec<Pixel>,
    depth: u8,
}

impl Fb {
    pub fn new(width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            px: vec![ALPHA; (width * height) as usize],
            depth: 32,
        }
    }

    /// `XCreatePixmap` with a depth of 1: a bitmap, all bits clear.
    pub fn new_bitmap(width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            px: vec![0; (width * height) as usize],
            depth: 1,
        }
    }

    /// 1 for a bitmap, 32 for a full-colour drawable.
    #[inline]
    pub fn depth(&self) -> u8 {
        self.depth
    }

    /// Coerce a value into what this drawable stores: a single bit for a
    /// bitmap, an opaque colour otherwise.
    #[inline]
    fn store(&self, v: Pixel) -> Pixel {
        if self.depth == 1 {
            v & 1
        } else {
            (v & RGB_MASK) | ALPHA
        }
    }

    /// Apply a raster operation in this drawable's depth.
    #[inline]
    fn combine(&self, dst: Pixel, src: Pixel, func: GXFunc) -> Pixel {
        if self.depth == 1 {
            func.apply_bit(dst, src)
        } else {
            func.apply(dst, src)
        }
    }

    pub fn filled(width: i32, height: i32, p: Pixel) -> Self {
        let mut fb = Self::new(width, height);
        fb.px.fill(p);
        fb
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    pub fn pixels(&self) -> &[Pixel] {
        &self.px
    }

    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [Pixel] {
        &mut self.px
    }

    /// The buffer as RGBA bytes, ready for `putImageData`. Only meaningful for
    /// a full-colour drawable; a bitmap's bytes are bits, not pixels.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Pixel` is `u32`, which has no padding and no invalid bit
        // patterns, so any `[u32]` is a valid `[u8]` four times as long. The
        // borrow keeps the source alive for the lifetime of the slice.
        unsafe { std::slice::from_raw_parts(self.px.as_ptr().cast::<u8>(), self.px.len() * 4) }
    }

    /// Resize, discarding the contents. Used when the window changes size.
    pub fn resize(&mut self, width: i32, height: i32) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        let blank = if self.depth == 1 { 0 } else { ALPHA };
        self.px.clear();
        self.px.resize((width * height) as usize, blank);
    }

    #[inline]
    fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    #[inline]
    fn clipped_out(gc: &Gc, x: i32, y: i32) -> bool {
        match gc.clip {
            None => false,
            Some(r) => x < r.x || y < r.y || x >= r.x + r.width || y >= r.y + r.height,
        }
    }

    /// `XGetPixel`. Out of bounds reads as opaque black, matching what an X
    /// server does for a request it cannot serve rather than trapping.
    #[inline]
    pub fn get_pixel(&self, x: i32, y: i32) -> Pixel {
        if self.in_bounds(x, y) {
            self.px[(y * self.width + x) as usize]
        } else {
            ALPHA
        }
    }

    /// `XPutPixel`. Ignores the GC, as X does.
    #[inline]
    pub fn put_pixel(&mut self, x: i32, y: i32, p: Pixel) {
        if self.in_bounds(x, y) {
            let v = self.store(p);
            self.px[(y * self.width + x) as usize] = v;
        }
    }

    #[inline]
    fn plot(&mut self, gc: &Gc, x: i32, y: i32) {
        if !self.in_bounds(x, y) || Self::clipped_out(gc, x, y) {
            return;
        }
        let i = (y * self.width + x) as usize;
        self.px[i] = self.combine(self.px[i], gc.foreground, gc.function);
    }

    /// A horizontal span, the workhorse for every filled shape.
    #[inline]
    fn span(&mut self, gc: &Gc, mut x0: i32, mut x1: i32, y: i32) {
        if y < 0 || y >= self.height {
            return;
        }
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if let Some(r) = gc.clip {
            if y < r.y || y >= r.y + r.height {
                return;
            }
            x0 = x0.max(r.x);
            x1 = x1.min(r.x + r.width - 1);
        }
        x0 = x0.max(0);
        x1 = x1.min(self.width - 1);
        if x0 > x1 {
            return;
        }
        let row = (y * self.width) as usize;
        let fg = gc.foreground;
        if gc.function == GXFunc::Copy {
            let fg = self.store(fg);
            self.px[row + x0 as usize..=row + x1 as usize].fill(fg);
        } else {
            for i in row + x0 as usize..=row + x1 as usize {
                self.px[i] = self.combine(self.px[i], fg, gc.function);
            }
        }
    }

    // ---- rectangles -------------------------------------------------------

    /// `XClearWindow`: fill with the background colour.
    pub fn clear(&mut self, background: Pixel) {
        let v = self.store(background);
        self.px.fill(v);
    }

    /// `XClearArea`.
    pub fn clear_area(&mut self, background: Pixel, x: i32, y: i32, w: i32, h: i32) {
        let gc = Gc {
            foreground: background,
            ..Gc::default()
        };
        self.fill_rectangle(&gc, x, y, w, h);
    }

    /// `XFillRectangle`.
    pub fn fill_rectangle(&mut self, gc: &Gc, x: i32, y: i32, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let y1 = y.saturating_add(h);
        for yy in y.max(0)..y1.min(self.height) {
            self.span(gc, x, x + w - 1, yy);
        }
    }

    /// `XFillRectangles`.
    pub fn fill_rectangles(&mut self, gc: &Gc, rects: &[XRectangle]) {
        for r in rects {
            self.fill_rectangle(gc, r.x, r.y, r.width, r.height);
        }
    }

    /// `XDrawRectangle`: the outline only, which in X is one pixel wider and
    /// taller than the corresponding `XFillRectangle`.
    pub fn draw_rectangle(&mut self, gc: &Gc, x: i32, y: i32, w: i32, h: i32) {
        if w < 0 || h < 0 {
            return;
        }
        self.draw_line(gc, x, y, x + w, y);
        self.draw_line(gc, x, y + h, x + w, y + h);
        self.draw_line(gc, x, y, x, y + h);
        self.draw_line(gc, x + w, y, x + w, y + h);
    }

    // ---- points -----------------------------------------------------------

    /// `XDrawPoint`.
    pub fn draw_point(&mut self, gc: &Gc, x: i32, y: i32) {
        self.plot(gc, x, y);
    }

    /// `XDrawPoints` with `CoordModeOrigin`.
    pub fn draw_points(&mut self, gc: &Gc, points: &[XPoint]) {
        for p in points {
            self.plot(gc, p.x, p.y);
        }
    }

    // ---- lines ------------------------------------------------------------

    /// `XDrawLine`. Honours `line_width`; widths above 1 are drawn as a
    /// filled quad plus round caps, which is close enough to X's join and cap
    /// defaults at the widths the hacks use.
    pub fn draw_line(&mut self, gc: &Gc, x1: i32, y1: i32, x2: i32, y2: i32) {
        if gc.line_width > 1 {
            self.thick_line(gc, x1, y1, x2, y2);
            return;
        }
        self.bresenham(gc, x1, y1, x2, y2);
    }

    fn bresenham(&mut self, gc: &Gc, x1: i32, y1: i32, x2: i32, y2: i32) {
        let dx = (x2 - x1).abs();
        let dy = -(y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x1, y1);
        loop {
            self.plot(gc, x, y);
            if x == x2 && y == y2 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                if x == x2 {
                    break;
                }
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                if y == y2 {
                    break;
                }
                err += dx;
                y += sy;
            }
        }
    }

    fn thick_line(&mut self, gc: &Gc, x1: i32, y1: i32, x2: i32, y2: i32) {
        let w = gc.line_width as f64;
        let (dx, dy) = ((x2 - x1) as f64, (y2 - y1) as f64);
        let len = dx.hypot(dy);
        if len < 0.5 {
            self.fill_arc(
                gc,
                x1 - gc.line_width / 2,
                y1 - gc.line_width / 2,
                gc.line_width,
                gc.line_width,
                0,
                FULL_CIRCLE,
            );
            return;
        }
        let (nx, ny) = (-dy / len * w / 2.0, dx / len * w / 2.0);
        let quad = [
            XPoint {
                x: (x1 as f64 + nx).round() as i32,
                y: (y1 as f64 + ny).round() as i32,
            },
            XPoint {
                x: (x2 as f64 + nx).round() as i32,
                y: (y2 as f64 + ny).round() as i32,
            },
            XPoint {
                x: (x2 as f64 - nx).round() as i32,
                y: (y2 as f64 - ny).round() as i32,
            },
            XPoint {
                x: (x1 as f64 - nx).round() as i32,
                y: (y1 as f64 - ny).round() as i32,
            },
        ];
        self.fill_polygon(gc, &quad);
        // Round the ends so a polyline has no notches at its joints.
        let r = gc.line_width;
        self.fill_arc(gc, x1 - r / 2, y1 - r / 2, r, r, 0, FULL_CIRCLE);
        self.fill_arc(gc, x2 - r / 2, y2 - r / 2, r, r, 0, FULL_CIRCLE);
    }

    /// `XDrawLines` with `CoordModeOrigin`.
    pub fn draw_lines(&mut self, gc: &Gc, points: &[XPoint]) {
        for pair in points.windows(2) {
            self.draw_line(gc, pair[0].x, pair[0].y, pair[1].x, pair[1].y);
        }
    }

    /// `XDrawLines` with `CoordModePrevious`: each point is relative to the one
    /// before it.
    pub fn draw_lines_relative(&mut self, gc: &Gc, points: &[XPoint]) {
        let Some(first) = points.first() else { return };
        let (mut x, mut y) = (first.x, first.y);
        for p in &points[1..] {
            let (nx, ny) = (x + p.x, y + p.y);
            self.draw_line(gc, x, y, nx, ny);
            x = nx;
            y = ny;
        }
    }

    /// `XDrawSegments`.
    pub fn draw_segments(&mut self, gc: &Gc, segs: &[XSegment]) {
        for s in segs {
            self.draw_line(gc, s.x1, s.y1, s.x2, s.y2);
        }
    }

    // ---- polygons ---------------------------------------------------------

    /// `XFillPolygon`. The shape argument is only a performance hint in X, so
    /// it is not taken; the fill rule comes from the GC.
    pub fn fill_polygon(&mut self, gc: &Gc, points: &[XPoint]) {
        if points.len() < 3 {
            return;
        }
        let ymin = points.iter().map(|p| p.y).min().unwrap_or(0).max(0);
        let ymax = points
            .iter()
            .map(|p| p.y)
            .max()
            .unwrap_or(0)
            .min(self.height - 1);

        // Crossings for one scanline: x where the edge cuts it, and which way
        // the edge is pointing (for the winding rule).
        let mut xs: Vec<(f64, i32)> = Vec::with_capacity(points.len());
        for y in ymin..=ymax {
            xs.clear();
            let yc = y as f64 + 0.5;
            for i in 0..points.len() {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                let (ay, by) = (a.y as f64, b.y as f64);
                if (ay <= yc && by > yc) || (by <= yc && ay > yc) {
                    let t = (yc - ay) / (by - ay);
                    xs.push((
                        a.x as f64 + t * (b.x - a.x) as f64,
                        if by > ay { 1 } else { -1 },
                    ));
                }
            }
            if xs.is_empty() {
                continue;
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            match gc.fill_rule {
                FillRule::EvenOdd => {
                    for pair in xs.chunks(2) {
                        if let [a, b] = pair {
                            self.span(gc, a.0.ceil() as i32, (b.0 - 1.0).floor() as i32 + 1, y);
                        }
                    }
                }
                FillRule::Winding => {
                    let mut wind = 0;
                    for i in 0..xs.len().saturating_sub(1) {
                        wind += xs[i].1;
                        if wind != 0 {
                            self.span(
                                gc,
                                xs[i].0.ceil() as i32,
                                (xs[i + 1].0 - 1.0).floor() as i32 + 1,
                                y,
                            );
                        }
                    }
                }
            }
        }
    }

    // ---- arcs -------------------------------------------------------------

    /// Sample an X arc into a point list, angles in units of 1/64 degree.
    fn arc_points(x: i32, y: i32, w: i32, h: i32, angle1: i32, angle2: i32) -> Vec<XPoint> {
        let (rx, ry) = (w as f64 / 2.0, h as f64 / 2.0);
        let (cx, cy) = (x as f64 + rx, y as f64 + ry);
        // One sample per pixel of the longer axis, so a big circle stays round
        // and a tiny one costs almost nothing.
        let steps = ((rx.abs().max(ry.abs()) * 4.0) as usize).clamp(8, 720);
        let a1 = angle1 as f64 * std::f64::consts::PI / (180.0 * 64.0);
        let a2 = angle2 as f64 * std::f64::consts::PI / (180.0 * 64.0);
        (0..=steps)
            .map(|i| {
                let a = a1 + a2 * (i as f64 / steps as f64);
                // X's Y axis points down, so a positive angle is counter-clockwise
                // on screen only if we negate the sine.
                XPoint {
                    x: (cx + rx * a.cos()).round() as i32,
                    y: (cy - ry * a.sin()).round() as i32,
                }
            })
            .collect()
    }

    /// `XDrawArc`.
    pub fn draw_arc(&mut self, gc: &Gc, x: i32, y: i32, w: i32, h: i32, angle1: i32, angle2: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let pts = Self::arc_points(x, y, w, h, angle1, angle2);
        self.draw_lines(gc, &pts);
    }

    /// `XFillArc` with the default `ArcPieSlice` mode: a full ellipse when the
    /// arc closes, a pie slice otherwise.
    pub fn fill_arc(&mut self, gc: &Gc, x: i32, y: i32, w: i32, h: i32, angle1: i32, angle2: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        if angle2.abs() >= FULL_CIRCLE {
            self.fill_ellipse(gc, x, y, w, h);
            return;
        }
        let mut pts = Self::arc_points(x, y, w, h, angle1, angle2);
        pts.push(XPoint {
            x: x + w / 2,
            y: y + h / 2,
        });
        self.fill_polygon(gc, &pts);
    }

    /// `XFillArcs`.
    pub fn fill_arcs(&mut self, gc: &Gc, arcs: &[XArc]) {
        for a in arcs {
            self.fill_arc(gc, a.x, a.y, a.width, a.height, a.angle1, a.angle2);
        }
    }

    /// The common case, worth doing directly: a filled ellipse by spans.
    fn fill_ellipse(&mut self, gc: &Gc, x: i32, y: i32, w: i32, h: i32) {
        let (rx, ry) = (w as f64 / 2.0, h as f64 / 2.0);
        let (cx, cy) = (x as f64 + rx, y as f64 + ry);
        let y0 = y.max(0);
        let y1 = (y + h - 1).min(self.height - 1);
        for yy in y0..=y1 {
            let dy = (yy as f64 + 0.5 - cy) / ry;
            let s = 1.0 - dy * dy;
            if s <= 0.0 {
                continue;
            }
            let dx = rx * s.sqrt();
            self.span(
                gc,
                (cx - dx).round() as i32,
                (cx + dx).round() as i32 - 1,
                yy,
            );
        }
    }

    // ---- blits ------------------------------------------------------------

    /// `XCopyArea` from another drawable.
    pub fn copy_area(
        &mut self,
        gc: &Gc,
        src: &Fb,
        src_x: i32,
        src_y: i32,
        w: i32,
        h: i32,
        dst_x: i32,
        dst_y: i32,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        for j in 0..h {
            let sy = src_y + j;
            let dy = dst_y + j;
            if dy < 0 || dy >= self.height || sy < 0 || sy >= src.height {
                continue;
            }
            for i in 0..w {
                let sx = src_x + i;
                let dx = dst_x + i;
                if sx < 0 || sx >= src.width {
                    continue;
                }
                let p = src.px[(sy * src.width + sx) as usize];
                if !self.in_bounds(dx, dy) || Self::clipped_out(gc, dx, dy) {
                    continue;
                }
                let idx = (dy * self.width + dx) as usize;
                self.px[idx] = self.combine(self.px[idx], p, gc.function);
            }
        }
    }

    /// `XCopyArea` with the same drawable as source and destination. Overlap
    /// is handled, which is the whole point: hacks scroll the screen with it.
    pub fn copy_area_self(
        &mut self,
        gc: &Gc,
        src_x: i32,
        src_y: i32,
        w: i32,
        h: i32,
        dst_x: i32,
        dst_y: i32,
    ) {
        if w <= 0 || h <= 0 || (src_x == dst_x && src_y == dst_y) {
            return;
        }
        let src = self.sub_image(src_x, src_y, w, h);
        self.copy_area(gc, &src, 0, 0, w, h, dst_x, dst_y);
    }

    /// `XCopyPlane` from a depth-1 bitmap: set bits are drawn in the GC's
    /// foreground, clear bits in its background.
    ///
    /// This is how the hacks that compose a picture as a bitmap get it onto the
    /// screen, and the only way a depth-1 drawable becomes visible.
    pub fn copy_plane(
        &mut self,
        gc: &Gc,
        src: &Fb,
        src_x: i32,
        src_y: i32,
        w: i32,
        h: i32,
        dst_x: i32,
        dst_y: i32,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        for j in 0..h {
            let (sy, dy) = (src_y + j, dst_y + j);
            if dy < 0 || dy >= self.height || sy < 0 || sy >= src.height {
                continue;
            }
            for i in 0..w {
                let (sx, dx) = (src_x + i, dst_x + i);
                if sx < 0 || sx >= src.width {
                    continue;
                }
                if !self.in_bounds(dx, dy) || Self::clipped_out(gc, dx, dy) {
                    continue;
                }
                let bit = src.px[(sy * src.width + sx) as usize] & 1;
                let color = if bit != 0 {
                    gc.foreground
                } else {
                    gc.background
                };
                let idx = (dy * self.width + dx) as usize;
                self.px[idx] = self.combine(self.px[idx], color, gc.function);
            }
        }
    }

    /// `XGetImage`: a copy of a rectangle. Areas outside the drawable come back
    /// opaque black.
    pub fn sub_image(&self, x: i32, y: i32, w: i32, h: i32) -> XImage {
        let mut out = Fb::new(w, h);
        for j in 0..h.min(out.height) {
            for i in 0..w.min(out.width) {
                let p = self.get_pixel(x + i, y + j);
                out.px[(j * out.width + i) as usize] = p;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::color::{BLACK, rgb};

    fn white_gc() -> Gc {
        Gc::new(WHITE, BLACK)
    }

    #[test]
    fn starts_opaque_and_black() {
        let fb = Fb::new(4, 4);
        assert!(fb.pixels().iter().all(|p| *p == ALPHA));
        assert_eq!(fb.as_bytes().len(), 4 * 4 * 4);
    }

    #[test]
    fn fill_rectangle_clips_to_the_drawable() {
        let mut fb = Fb::new(8, 8);
        // Straddles all four edges; must not panic and must not wrap around.
        fb.fill_rectangle(&white_gc(), -4, -4, 16, 16);
        assert!(fb.pixels().iter().all(|p| *p == WHITE));

        let mut fb = Fb::new(8, 8);
        fb.fill_rectangle(&white_gc(), 6, 6, 100, 100);
        assert_eq!(fb.get_pixel(7, 7), WHITE);
        assert_eq!(fb.get_pixel(5, 7), ALPHA);
        // No wrap onto the next row.
        assert_eq!(fb.get_pixel(0, 7), ALPHA);
    }

    #[test]
    fn zero_and_negative_sizes_draw_nothing() {
        let mut fb = Fb::new(8, 8);
        fb.fill_rectangle(&white_gc(), 1, 1, 0, 5);
        fb.fill_rectangle(&white_gc(), 1, 1, 5, -5);
        fb.fill_arc(&white_gc(), 1, 1, 0, 0, 0, FULL_CIRCLE);
        assert!(fb.pixels().iter().all(|p| *p == ALPHA));
    }

    #[test]
    fn xor_twice_is_identity() {
        let mut fb = Fb::new(16, 16);
        let mut gc = white_gc();
        gc.set_function(GXFunc::Xor);
        let before = fb.clone();
        fb.fill_rectangle(&gc, 2, 2, 8, 8);
        assert_ne!(fb, before);
        fb.fill_rectangle(&gc, 2, 2, 8, 8);
        assert_eq!(fb, before, "XOR is not self-inverse");
    }

    #[test]
    fn raster_ops_keep_alpha_opaque() {
        let mut fb = Fb::new(4, 4);
        for f in [
            GXFunc::Xor,
            GXFunc::Invert,
            GXFunc::Clear,
            GXFunc::Set,
            GXFunc::And,
            GXFunc::Or,
        ] {
            let mut gc = white_gc();
            gc.set_function(f);
            fb.fill_rectangle(&gc, 0, 0, 4, 4);
            assert!(
                fb.pixels().iter().all(|p| p & ALPHA == ALPHA),
                "{f:?} produced a transparent pixel"
            );
        }
    }

    #[test]
    fn clip_rectangle_is_honoured() {
        let mut fb = Fb::new(16, 16);
        let mut gc = white_gc();
        gc.set_clip_rect(XRectangle {
            x: 4,
            y: 4,
            width: 4,
            height: 4,
        });
        fb.fill_rectangle(&gc, 0, 0, 16, 16);
        assert_eq!(fb.get_pixel(4, 4), WHITE);
        assert_eq!(fb.get_pixel(7, 7), WHITE);
        assert_eq!(fb.get_pixel(3, 4), ALPHA);
        assert_eq!(fb.get_pixel(8, 4), ALPHA);

        let mut fb = Fb::new(16, 16);
        fb.draw_line(&gc, 0, 6, 15, 6);
        assert_eq!(fb.get_pixel(0, 6), ALPHA);
        assert_eq!(fb.get_pixel(5, 6), WHITE);
    }

    #[test]
    fn lines_reach_both_endpoints() {
        let mut fb = Fb::new(32, 32);
        let gc = white_gc();
        for (x2, y2) in [(31, 0), (0, 31), (31, 31), (20, 5), (5, 20)] {
            let mut fb2 = fb.clone();
            fb2.draw_line(&gc, 0, 0, x2, y2);
            assert_eq!(fb2.get_pixel(0, 0), WHITE, "start missing for {x2},{y2}");
            assert_eq!(fb2.get_pixel(x2, y2), WHITE, "end missing for {x2},{y2}");
        }
        // Degenerate line is a single point.
        fb.draw_line(&gc, 5, 5, 5, 5);
        assert_eq!(fb.get_pixel(5, 5), WHITE);
    }

    #[test]
    fn polygon_fills_its_interior_not_its_exterior() {
        let mut fb = Fb::new(32, 32);
        let tri = [
            XPoint { x: 16, y: 4 },
            XPoint { x: 28, y: 28 },
            XPoint { x: 4, y: 28 },
        ];
        fb.fill_polygon(&white_gc(), &tri);
        assert_eq!(fb.get_pixel(16, 20), WHITE, "centre not filled");
        assert_eq!(fb.get_pixel(5, 6), ALPHA, "outside got filled");
        assert_eq!(fb.get_pixel(31, 31), ALPHA);
    }

    #[test]
    fn filled_circle_is_round() {
        let mut fb = Fb::new(64, 64);
        fb.fill_arc(&white_gc(), 8, 8, 48, 48, 0, FULL_CIRCLE);
        assert_eq!(fb.get_pixel(32, 32), WHITE, "centre");
        assert_eq!(fb.get_pixel(32, 10), WHITE, "top");
        assert_eq!(fb.get_pixel(10, 32), WHITE, "left");
        assert_eq!(fb.get_pixel(9, 9), ALPHA, "corner should be outside");
        assert_eq!(fb.get_pixel(54, 54), ALPHA, "corner should be outside");
    }

    #[test]
    fn arc_angles_run_counter_clockwise_from_three_oclock() {
        // A quarter arc from 0 to 90 degrees covers the upper right quadrant.
        let mut fb = Fb::new(64, 64);
        fb.fill_arc(&white_gc(), 0, 0, 64, 64, 0, 90 * 64);
        assert_eq!(fb.get_pixel(50, 14), WHITE, "upper right should be filled");
        assert_eq!(fb.get_pixel(14, 50), ALPHA, "lower left should be empty");
    }

    #[test]
    fn copy_area_moves_pixels() {
        let mut src = Fb::new(8, 8);
        src.fill_rectangle(&white_gc(), 0, 0, 4, 4);
        let mut dst = Fb::new(8, 8);
        dst.copy_area(&white_gc(), &src, 0, 0, 4, 4, 4, 4);
        assert_eq!(dst.get_pixel(5, 5), WHITE);
        assert_eq!(dst.get_pixel(1, 1), ALPHA);
    }

    #[test]
    fn copy_area_self_handles_overlap() {
        let mut fb = Fb::new(8, 8);
        fb.fill_rectangle(&white_gc(), 0, 0, 8, 4);
        // Scroll down by two rows; the overlap must not smear.
        fb.copy_area_self(&white_gc(), 0, 0, 8, 4, 0, 2);
        assert_eq!(fb.get_pixel(0, 5), WHITE);
        assert_eq!(fb.get_pixel(0, 6), ALPHA);
    }

    #[test]
    fn get_pixel_outside_is_black_not_a_panic() {
        let fb = Fb::new(4, 4);
        assert_eq!(fb.get_pixel(-1, -1), ALPHA);
        assert_eq!(fb.get_pixel(99, 99), ALPHA);
    }

    #[test]
    fn resize_keeps_the_buffer_consistent() {
        let mut fb = Fb::new(4, 4);
        fb.resize(9, 3);
        assert_eq!(fb.width(), 9);
        assert_eq!(fb.height(), 3);
        assert_eq!(fb.pixels().len(), 27);
        // Degenerate sizes are clamped rather than producing an empty buffer.
        fb.resize(0, 0);
        assert_eq!(fb.pixels().len(), 1);
    }

    #[test]
    fn a_bitmap_stores_bits_not_colours() {
        let mut bm = Fb::new_bitmap(8, 8);
        assert_eq!(bm.depth(), 1);
        assert!(bm.pixels().iter().all(|p| *p == 0), "should start clear");

        let mut gc = Gc::default();
        gc.set_foreground(1);
        bm.fill_rectangle(&gc, 2, 2, 4, 4);
        assert_eq!(bm.get_pixel(3, 3), 1);
        assert_eq!(bm.get_pixel(0, 0), 0);

        // XOR on a bitmap toggles the bit, and stays a bit.
        gc.set_function(GXFunc::Xor);
        bm.fill_rectangle(&gc, 2, 2, 4, 4);
        assert!(bm.pixels().iter().all(|p| *p == 0), "XOR did not clear");
    }

    #[test]
    fn copy_plane_paints_set_bits_and_clear_ones() {
        let mut bm = Fb::new_bitmap(8, 8);
        let mut one = Gc::default();
        one.set_foreground(1);
        bm.fill_rectangle(&one, 0, 0, 4, 8);

        let mut fb = Fb::new(8, 8);
        let gc = Gc::new(WHITE, rgb(9, 9, 9));
        fb.copy_plane(&gc, &bm, 0, 0, 8, 8, 0, 0);
        assert_eq!(fb.get_pixel(1, 1), WHITE, "set bit should be foreground");
        assert_eq!(
            fb.get_pixel(6, 1),
            rgb(9, 9, 9),
            "clear bit should be background"
        );
    }

    #[test]
    fn a_resized_bitmap_is_still_a_bitmap() {
        let mut bm = Fb::new_bitmap(4, 4);
        bm.resize(9, 3);
        assert_eq!(bm.depth(), 1);
        assert!(bm.pixels().iter().all(|p| *p == 0));
    }

    #[test]
    fn put_and_get_pixel_round_trip() {
        let mut fb = Fb::new(4, 4);
        let p = rgb(1, 2, 3);
        fb.put_pixel(2, 2, p);
        assert_eq!(fb.get_pixel(2, 2), p);
        fb.put_pixel(-1, 0, p);
        fb.put_pixel(0, 99, p);
    }
}
