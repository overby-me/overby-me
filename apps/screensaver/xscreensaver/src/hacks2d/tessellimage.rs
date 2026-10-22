//! Port of `hacks/tessellimage.c`.
//!
//! ```text
//! tessellimage, Copyright © 2014-2025 Jamie Zawinski <jwz@jwz.org>
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
//! A picture is redrawn as flat triangles, or as the polygons dual to them, and
//! then redrawn again with more of them, and more, and back down.
//!
//! Where the triangles go is the whole idea. First the picture is turned into a
//! map of how much each pixel differs from the four neighbours above and to its
//! left, in a colour space weighted the way an eye weights green over red over
//! blue. That map is edge detection by another name: it is near zero across a
//! sky and large along the line where the sky meets a roof. Every pixel whose
//! difference clears a threshold becomes a triangulation control point, so
//! detail lands where the picture changes and nowhere else. Sweeping the
//! threshold up and down is what animates it, and because the same threshold
//! always gives the same picture, each one is kept once it has been drawn.
//!
//! Delaunay mode fills each triangle with the colour of the pixel at its
//! middle. Voronoi mode builds the dual: for every control point, the polygon
//! whose corners are the centres of the triangles that touch it. Upstream notes
//! that this leaves out the cells that should meet the edge of the frame, and
//! it does here too.
//!
//! The triangulation itself is [`crate::runtime::delaunay`].
//!
//! Two upstream resources are not on the panel, matching the XML: the cache,
//! which changes nothing you can see, and the resolution cap, which upstream
//! applies by asking its loader for a smaller image and which is applied here
//! by sampling the picture down after it arrives.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, rgb, unrgb};
use crate::runtime::delaunay::{ITriangle, Xyz, delaunay, sort_by_x};
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XEvent, XImage, XPoint, XRectangle, random, screenhack_event_helper,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Delaunay,
    Voronoi,
}

/// One cell of the Voronoi diagram: the corners, and their mean.
struct VoronoiPolygon {
    ctr: XPoint,
    p: Vec<XPoint>,
}

struct Tessellimage {
    delay: u32,
    outline_p: bool,
    cache_p: bool,
    fill_p: bool,
    duration: f64,
    duration2: f64,
    max_depth: i32,
    max_resolution: i32,
    start_time: f64,
    start_time2: f64,

    /// The source picture, and the map of how much each pixel differs from its
    /// neighbours. Both are at the analysis resolution, which the window scales
    /// up from.
    img: Option<XImage>,
    delta: Vec<u8>,
    dw: i32,
    dh: i32,

    output: Option<Pixmap>,
    deltap: Option<Pixmap>,
    /// The thresholds worth drawing, and how many control points each gives.
    threshes: Vec<i32>,
    vsizes: Vec<i32>,
    thresh: i32,
    dthresh: i32,
    cache: Vec<Option<Pixmap>>,

    img_loader: Option<ImageLoad>,
    loading: bool,
    geom: XRectangle,
    button_down_p: bool,
    mode: Mode,
    gc: Gc,
    width: i32,
    height: i32,
}

/// The distance between two colours, weighted the way an eye weights them.
fn pixel_distance(p1: Pixel, p2: Pixel) -> i32 {
    if p1 == 0 && p2 == 0 {
        return 0;
    }
    let (r1, g1, b1) = unrgb(p1);
    let (r2, g2, b2) = unrgb(p2);
    let rd = ((r2 as i32 - r1 as i32) as f64 * 0.2989 / 0.5870) as i32;
    let gd = ((g2 as i32 - g1 as i32) as f64 * 0.5870 / 0.5870) as i32;
    let bd = ((b2 as i32 - b1 as i32) as f64 * 0.1140 / 0.5870) as i32;
    let d = ((rd * rd + gd * gd + bd * bd) as f64).cbrt() as i32;
    d.abs()
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = Tessellimage {
        delay: d.res.int("delay").max(1) as u32,
        outline_p: d.res.bool("outline"),
        cache_p: d.res.bool("cache"),
        fill_p: d.res.bool("fillScreen"),
        duration: d.res.float("duration").max(1.0),
        duration2: d.res.float("duration2").max(0.001),
        max_depth: d.res.int("maxDepth").max(100),
        max_resolution: d.res.int("maxResolution").max(0),
        start_time: 0.0,
        start_time2: 0.0,
        img: None,
        delta: Vec::new(),
        dw: 0,
        dh: 0,
        output: None,
        deltap: None,
        threshes: Vec::new(),
        vsizes: Vec::new(),
        thresh: 0,
        dthresh: 1,
        cache: Vec::new(),
        img_loader: None,
        loading: false,
        geom: XRectangle::default(),
        button_down_p: false,
        mode: Mode::Delaunay,
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        width: d.width(),
        height: d.height(),
    };
    d.clear_window();
    st.start_load(d);
    Box::new(st)
}

impl Tessellimage {
    fn start_load(&mut self, d: &mut Dpy) {
        d.clear_window();
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        self.loading = false;
        self.geom = d.image_geometry();
        self.analyze(d);
        self.start_time = d.time;
        self.start_time2 = self.start_time;
    }

    /// Take the picture off the window, and shrink it if the window is bigger
    /// than we are willing to analyse. Upstream asks its loader for a smaller
    /// image instead; the effect is the same.
    fn snapshot(&mut self, d: &Dpy) -> XImage {
        let (w, h) = (self.width, self.height);
        let full = d.win_ref().sub_image(0, 0, w, h);
        let m = self.max_resolution;
        if m <= 10 || (w <= m && h <= m) {
            return full;
        }
        let (sw, sh) = if w > h {
            (m, (m * h / w).max(1))
        } else {
            ((m * w / h).max(1), m)
        };
        let mut small = XImage::new(sw, sh);
        for y in 0..sh {
            for x in 0..sw {
                small.put_pixel(x, y, full.get_pixel(x * w / sw, y * h / sh));
            }
        }
        // The picture's place in the window shrinks with it.
        self.geom = XRectangle {
            x: self.geom.x * sw / w,
            y: self.geom.y * sh / h,
            width: (self.geom.width * sw / w).max(1),
            height: (self.geom.height * sh / h).max(1),
        };
        small
    }

    /// Blow the picture up so the part of it that is actually picture covers
    /// the whole frame, cropping what falls off.
    fn scale_image(&mut self, img: XImage) -> XImage {
        if self.geom.width <= 0 || self.geom.height <= 0 {
            return img;
        }
        let s1 = self.geom.width as f64 / img.width() as f64;
        let s2 = self.geom.height as f64 / img.height() as f64;
        let scale = s1.min(s2);

        let mut out = XImage::new(img.width(), img.height());
        let cx = img.width() / 2;
        let mut cy = img.height() / 2;
        if self.geom.width < self.geom.height {
            // Portrait: aim toward the top.
            cy = (img.height() as f64 / (2.0 / scale)) as i32;
        }

        for y in 0..out.height() {
            for x in 0..out.width() {
                let x2 = cx + ((x - cx) as f64 * scale) as i32;
                let y2 = cy + ((y - cy) as f64 * scale) as i32;
                let p = if x2 >= 0 && y2 >= 0 && x2 < img.width() && y2 < img.height() {
                    img.get_pixel(x2, y2)
                } else {
                    0
                };
                out.put_pixel(x, y, p);
            }
        }

        self.geom = XRectangle {
            x: 0,
            y: 0,
            width: out.width(),
            height: out.height(),
        };
        out
    }

    fn delta_at(&self, x: i32, y: i32) -> u8 {
        self.delta[(y * self.dw + x) as usize]
    }

    /// Work out the difference map and, from its histogram, which thresholds
    /// are worth drawing at all.
    fn analyze(&mut self, d: &mut Dpy) {
        self.mode = match d.res.string("mode") {
            "delaunay" => Mode::Delaunay,
            "voronoi" => Mode::Voronoi,
            // "random", or anything else.
            _ => {
                if random() & 1 != 0 {
                    Mode::Delaunay
                } else {
                    Mode::Voronoi
                }
            }
        };

        self.flush_cache();

        let mut img = self.snapshot(d);
        if self.fill_p {
            img = self.scale_image(img);
        }
        let (w, h) = (img.width(), img.height());
        self.dw = w;
        self.dh = h;

        // The first derivative of the picture: how far each pixel is from the
        // four neighbours above and to its left.
        self.delta = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let here = img.get_pixel(x, y);
                let neighbours = [
                    if x > 0 && y > 0 {
                        img.get_pixel(x - 1, y - 1)
                    } else {
                        0
                    },
                    if y > 0 { img.get_pixel(x, y - 1) } else { 0 },
                    if x > 0 { img.get_pixel(x - 1, y) } else { 0 },
                    if x > 0 && y < h - 1 {
                        img.get_pixel(x - 1, y + 1)
                    } else {
                        0
                    },
                ];
                let mut distance = 0;
                for n in neighbours {
                    distance += pixel_distance(here, n);
                }
                distance /= 4;
                self.delta[(y * w + x) as usize] = distance.clamp(0, 255) as u8;
            }
        }
        self.img = Some(img);

        // How many pixels are at each difference, then how many are at least
        // that much, which is how many control points a threshold would give.
        let mut histo = [0u64; 256];
        for v in &self.delta {
            histo[*v as usize] += 1;
        }
        for i in (1..histo.len()).rev() {
            histo[i - 1] += histo[i];
        }

        // Keep the thresholds that give a usefully different number of control
        // points; ones that give nearly the same number look nearly the same.
        let max_vsize = self.max_depth as u64;
        let min_vsize = 20u64.min(max_vsize / 100);
        let min_delta = 100u64.min(max_vsize / 1000);

        self.threshes.clear();
        self.vsizes.clear();
        for i in (0..histo.len()).rev() {
            let vsize = histo[i];
            if vsize >= min_vsize
                && vsize <= max_vsize
                && (self.vsizes.is_empty()
                    || vsize >= *self.vsizes.last().unwrap() as u64 + min_delta)
            {
                self.threshes.push(i as i32);
                self.vsizes.push(vsize as i32);
            }
        }

        self.thresh = 0; // Startup.
        self.dthresh = 1; // Forward.
        self.output = None;
        self.cache = (0..self.threshes.len()).map(|_| None).collect();
    }

    fn flush_cache(&mut self) {
        self.cache.clear();
        self.deltap = None;
    }

    /// The corners are too close together for an outline to be worth drawing.
    fn small_triangle_p(p: &[XPoint; 3]) -> bool {
        let min = 4;
        (p[0].x - p[1].x).abs() < min
            || (p[0].y - p[1].y).abs() < min
            || (p[1].x - p[2].x).abs() < min
            || (p[1].y - p[2].y).abs() < min
            || (p[2].x - p[0].x).abs() < min
            || (p[2].y - p[0].y).abs() < min
    }

    fn small_cell_p(p: &VoronoiPolygon) -> bool {
        let min = 4;
        (p.p[0].x - p.ctr.x).abs() < min || (p.p[0].y - p.ctr.y).abs() < min
    }

    /// Sort a cell's corners so they wind around its middle rather than
    /// zig-zagging across it.
    fn sort_ccw(ctr: &XPoint, p: &mut [XPoint]) {
        p.sort_by(|a, b| {
            let sa = ((a.x - ctr.x) as f64).atan2((a.y - ctr.y) as f64);
            let sb = ((b.x - ctr.x) as f64).atan2((b.y - ctr.y) as f64);
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Turn a triangulation into its dual: for every control point, the polygon
    /// whose corners are the centres of the triangles that touch it.
    fn delaunay_to_voronoi(
        np: usize,
        p: &[Xyz],
        v: &[ITriangle],
        scale: f64,
    ) -> Vec<VoronoiPolygon> {
        let mut vert_to_tri: Vec<Vec<usize>> = vec![Vec::new(); np + 1];
        for (i, t) in v.iter().enumerate() {
            for c in t.corners() {
                if c < np {
                    vert_to_tri[c].push(i);
                }
            }
        }

        let mut out = Vec::with_capacity(np);
        for tris in vert_to_tri.iter().take(np) {
            // Fewer than three centres is not a polygon.
            if tris.len() < 3 {
                out.push(VoronoiPolygon {
                    ctr: XPoint::default(),
                    p: Vec::new(),
                });
                continue;
            }
            let mut pts = Vec::with_capacity(tris.len());
            let (mut cx, mut cy) = (0i64, 0i64);
            for &ti in tris {
                let t = &v[ti];
                let x = (scale * (p[t.p1].x + p[t.p2].x + p[t.p3].x) / 3.0) as i32;
                let y = (scale * (p[t.p1].y + p[t.p2].y + p[t.p3].y) / 3.0) as i32;
                pts.push(XPoint { x, y });
                cx += x as i64;
                cy += y as i64;
            }
            let ctr = XPoint {
                x: (cx / pts.len() as i64) as i32,
                y: (cy / pts.len() as i64) as i32,
            };
            Self::sort_ccw(&ctr, &mut pts);
            out.push(VoronoiPolygon { ctr, p: pts });
        }
        out
    }

    /// A colour a fifth of the way towards black, for the outlines.
    fn darker(color: Pixel) -> Pixel {
        let (r, g, b) = unrgb(color);
        let s = |c: u8| (c as u16 * 8 / 10) as u8;
        rgb(s(r), s(g), s(b))
    }

    /// Build the picture for the current threshold, unless it is already built.
    fn tessellate(&mut self, d: &mut Dpy) {
        if self.threshes.is_empty() {
            return;
        }
        let mut ticked_p = false;

        if !self.button_down_p && self.start_time2 + self.duration2 < d.time {
            self.start_time2 = d.time;
            self.thresh += self.dthresh;
            ticked_p = true;
            if self.thresh >= self.threshes.len() as i32 {
                self.thresh = self.threshes.len() as i32 - 1;
                self.dthresh = -1;
            } else if self.thresh < 0 {
                self.thresh = 0;
                self.dthresh = 1;
            }
        }
        if self.output.is_none() {
            ticked_p = true;
        }
        if !ticked_p {
            return;
        }

        let k = self.thresh as usize;
        let (w, h) = (self.width, self.height);

        if let Some(cached) = self.cache.get(k).and_then(|c| c.as_ref())
            && let Some(mut out) = self.output.take()
        {
            out.copy_area(&self.gc, cached, 0, 0, w, h, 0, 0);
            self.output = Some(out);
            return;
        }

        let threshold = self.threshes[k];
        let vsize = self.vsizes[k] as usize + 8; // Corners of screen and image.
        let wscale = w as f64 / self.dw as f64;

        // A control point at every pixel whose difference clears the threshold,
        // plus the corners so the triangulation covers the whole frame.
        let mut p: Vec<Xyz> = Vec::with_capacity(vsize + 4);
        if self.geom.width <= 0 {
            self.geom.width = self.dw;
        }
        if self.geom.height <= 0 {
            self.geom.height = self.dh;
        }
        for y in 0..=1 {
            for x in 0..=1 {
                let (px, py) = (
                    if x == 1 { self.dw - 1 } else { 0 },
                    if y == 1 { self.dh - 1 } else { 0 },
                );
                p.push(Xyz {
                    x: px as f64,
                    y: py as f64,
                    z: self.delta_at(px, py) as f64,
                });
                let gx = (self.geom.x + if x == 1 { self.geom.width - 1 } else { 0 })
                    .clamp(0, self.dw - 1);
                let gy = (self.geom.y + if y == 1 { self.geom.height - 1 } else { 0 })
                    .clamp(0, self.dh - 1);
                p.push(Xyz {
                    x: gx as f64,
                    y: gy as f64,
                    z: self.delta_at(gx, gy) as f64,
                });
            }
        }
        for y in 0..self.dh {
            for x in 0..self.dw {
                let px = self.delta_at(x, y) as i32;
                if px >= threshold {
                    p.push(Xyz {
                        x: x as f64,
                        y: y as f64,
                        z: px as f64,
                    });
                }
            }
        }

        sort_by_x(&mut p);
        let v = delaunay(&mut p);

        let mut out = Pixmap::new(w, h);
        self.gc.set_foreground(d.res.pixel("background"));
        out.fill_rectangle(&self.gc, 0, 0, w, h);

        let img = self.img.take().unwrap_or_else(|| XImage::new(1, 1));
        match self.mode {
            Mode::Voronoi => {
                let polys = Self::delaunay_to_voronoi(p.len(), &p, &v, wscale);
                for poly in &polys {
                    if poly.p.len() < 3 {
                        continue;
                    }
                    let color = img.get_pixel(
                        (poly.ctr.x as f64 / wscale) as i32,
                        (poly.ctr.y as f64 / wscale) as i32,
                    );
                    self.gc.set_foreground(color);
                    out.fill_polygon(&self.gc, &poly.p);

                    if self.outline_p && !Self::small_cell_p(poly) {
                        self.gc.set_foreground(Self::darker(color));
                        out.draw_lines(&self.gc, &poly.p);
                    }
                }
            }
            Mode::Delaunay => {
                for t in &v {
                    let xp = [
                        XPoint {
                            x: (p[t.p1].x * wscale) as i32,
                            y: (p[t.p1].y * wscale) as i32,
                        },
                        XPoint {
                            x: (p[t.p2].x * wscale) as i32,
                            y: (p[t.p2].y * wscale) as i32,
                        },
                        XPoint {
                            x: (p[t.p3].x * wscale) as i32,
                            y: (p[t.p3].y * wscale) as i32,
                        },
                    ];
                    // The triangle takes the colour of the pixel at its middle.
                    let color = img.get_pixel(
                        ((xp[0].x + xp[1].x + xp[2].x) as f64 / (3.0 * wscale)) as i32,
                        ((xp[0].y + xp[1].y + xp[2].y) as f64 / (3.0 * wscale)) as i32,
                    );
                    self.gc.set_foreground(color);
                    out.fill_polygon(&self.gc, &xp);

                    if self.outline_p && !Self::small_triangle_p(&xp) {
                        self.gc.set_foreground(Self::darker(color));
                        out.draw_lines(&self.gc, &xp);
                    }
                }
            }
        }
        self.img = Some(img);

        if self.cache_p && self.cache.get(k).is_some_and(|c| c.is_none()) {
            let mut copy = Pixmap::new(w, h);
            copy.copy_area(&self.gc, &out, 0, 0, w, h, 0, 0);
            self.cache[k] = Some(copy);
        }
        self.output = Some(out);
    }

    /// The difference map as something you can look at, which is what holding
    /// the button down shows.
    fn get_deltap(&mut self) -> &Pixmap {
        if self.deltap.is_none() {
            let (w, h) = (self.width, self.height);
            let wscale = w as f64 / self.dw as f64;
            let mut pm = Pixmap::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let sx = ((x as f64 / wscale) as i32).clamp(0, self.dw - 1);
                    let sy = ((y as f64 / wscale) as i32).clamp(0, self.dh - 1);
                    let v = (self.delta_at(sx, sy) as u32) << 5;
                    let c = v.min(255) as u8;
                    pm.put_pixel(x, y, rgb(c, c, c));
                }
            }
            self.deltap = Some(pm);
        }
        self.deltap.as_ref().unwrap()
    }
}

impl Screenhack for Tessellimage {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.start_time + self.duration < d.time {
            self.start_load(d);
            return self.delay;
        }

        self.tessellate(d);

        let (w, h) = (self.width, self.height);
        d.clear_window();
        if self.output.is_some() {
            if self.button_down_p {
                let src = self.get_deltap().clone();
                d.win().copy_area(&self.gc, &src, 0, 0, w, h, 0, 0);
            } else {
                let src = self.output.take().unwrap();
                d.win().copy_area(&self.gc, &src, 0, 0, w, h, 0, 0);
                self.output = Some(src);
            }
        } else if self.threshes.is_empty()
            && let Some(img) = self.img.take()
        {
            // Nothing worth tessellating: show the picture as it came.
            for y in 0..h.min(img.height()) {
                for x in 0..w.min(img.width()) {
                    let p = img.get_pixel(x, y);
                    d.win().put_pixel(x, y, p);
                }
            }
            self.img = Some(img);
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.flush_cache();
        self.output = None;
        self.start_load(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { .. } => {
                self.button_down_p = true;
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down_p = false;
                true
            }
            _ if screenhack_event_helper(event) => {
                self.start_time = 0.0; // Load the next image.
                true
            }
            _ => false,
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*dontClearRoot: True",
    "*fpsSolid: true",
    "*mode: random",
    "*delay: 30000",
    "*duration: 120",
    "*duration2: 0.4",
    "*maxDepth: 30000",
    "*maxResolution: 1024",
    "*outline: True",
    "*fillScreen: True",
    "*cache: True",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Delaunay or voronoi",
    },
    SelectItem {
        value: "delaunay",
        label: "Delaunay",
    },
    SelectItem {
        value: "voronoi",
        label: "Voronoi",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("duration2", "Speed", 0.1, 4.0, 0.1, 1, "0.4"),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider(
        "maxDepth",
        "Complexity",
        1000.0,
        100_000.0,
        1000.0,
        0,
        "30000",
    ),
    Opt::select("mode", "Tessellation", MODES, "random"),
    Opt::boolean("fillScreen", "Fill screen", "True"),
    Opt::boolean("outline", "Outline triangles", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "tessellimage",
    label: "Tessellimage",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2014",
        video: Some("https://www.youtube.com/watch?v=JNgybysnYU8"),
        blurb: "Converts an image to triangles using Delaunay tessellation, or to polygons using Voronoi tessellation.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
