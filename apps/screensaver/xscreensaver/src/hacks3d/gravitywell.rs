//! Port of `hacks/glx/gravitywell.c`.
//!
//! ```text
//! gravitywell, Copyright (c) 2019 Jamie Zawinski <jwz@jwz.org>
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
//! Massive objects distort space in a two dimensional universe: the rubber
//! sheet with the bowling balls on it, drawn as a wireframe grid with a well
//! under each star.
//!
//! The height of the sheet at a point is the sum over the stars of the inverse
//! square of the distance to each, which is the one line of physics in it. All
//! the rest is about drawing that cheaply, and it is more interesting than the
//! physics.
//!
//! A grid line is not sampled evenly. Far from every star the field is nearly
//! flat, so the line is drawn as one straight segment across a whole sixteen
//! units; near a star it curves too fast for that and every unit gets its own
//! sample. Each line is walked in three zones, and the middle one, where the
//! slope is steep enough to see but the star is not yet close, is the only one
//! that gets the fine treatment. That is what `segs` is: one flag per coarse
//! segment saying whether it was subdivided, so the drawing pass knows whether
//! to emit one vertex or sixteen.
//!
//! Inside a star's own radius the sheet is not drawn as a well at all: it is
//! held flat at the gravity that would be felt at the star's surface. Otherwise
//! the inverse square runs away to infinity at the middle and the well would be
//! an infinitely deep spike.
//!
//! Fog is what makes it readable. The grid is five hundred units across and
//! runs off to a horizon, and without fog every line at the far end is drawn as
//! brightly as the ones in front and the whole back half is a solid band.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_color_ramp, rgb_to_hsv, unrgb};
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};

const RESOLUTION_BASE: f64 = 512.0;
const GRID_SIZE_BASE: f64 = 7.0;
const SPEED_BASE: f32 = 2.5;
/// How little of a star's pull is worth drawing at all: its outer radius is
/// where the field falls below this.
const MASS_EPSILON: f32 = 0.03;
/// How steep the sheet has to be before a segment is worth subdividing.
const SLOPE_EPSILON: f32 = 0.06;
/// How many samples a coarse segment becomes when it is subdivided.
const GRID_SEG: usize = 16;
const MAX_MASS_COLOR: f32 = 120.0;

struct Star {
    mass: f32,
    /// The squares of the outer, middle and inner radii.
    ro2: f32,
    rm2: f32,
    ri2: f32,
    ro: f32,
    radius: f32,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    surface_gravity: f32,
    depth: f32,
}

struct GravityWell {
    trackball: Trackball,
    stars: Vec<Star>,
    grid_w: usize,
    grid_h: usize,
    /// One row of the sheet, rebuilt for every line drawn.
    grid: Vec<f32>,
    /// Whether each coarse segment of that row was subdivided.
    segs: Vec<bool>,
    colors: Vec<XColor>,
    speed: f32,
    resolution: f32,
    grid_size: f64,
}

/// `WCLIP`: to an integer inside `0..=hi`.
fn wclip(x: f32, hi: usize) -> usize {
    (x as i32).clamp(0, hi as i32) as usize
}

fn ease(r: f32) -> f32 {
    (r * std::f32::consts::FRAC_PI_2).sin()
}

impl GravityWell {
    fn new_star(&self, s: &mut Star) {
        let w = (self.grid_w * GRID_SEG) as f32;

        s.radius = 2.0 * (2.0 + frand(3.0) + frand(3.0) + frand(3.0)) as f32;
        s.mass = s.radius * 150.0 * (2.0 + frand(3.0) + frand(3.0) + frand(3.0)) as f32;

        s.ro2 = s.mass / MASS_EPSILON;
        s.ro = s.ro2.sqrt();
        s.rm2 = (f64::from(s.mass * (2.0 / SLOPE_EPSILON)).powf(2.0 / 3.0)) as f32;
        s.ri2 = s.radius * s.radius;
        if s.rm2 < s.ri2 {
            s.rm2 = s.ri2;
        }
        if s.ro2 < s.rm2 {
            s.ro2 = s.rm2;
        }

        s.dx = ((frand(1.0) as f32 - 0.5) * 0.1) / self.resolution;
        s.dy = (0.1 + frand(0.6) as f32) / self.resolution;

        /* What the experienced gravitation would be at the surface of the
        star, were the mass actually held in a singularity at its center. */
        s.surface_gravity = s.mass / s.ri2;
        s.depth = s.surface_gravity;
        // The caller places it; the first star sits in the middle of the grid
        // and the rest are scattered across it.
        s.x = w * 0.5;
    }

    /// Place a fresh star, at the middle if it is the first one.
    fn respawn(&mut self, i: usize) {
        let w = (self.grid_w * GRID_SEG) as f32;
        let mut s = std::mem::replace(&mut self.stars[i], Star::blank());
        self.new_star(&mut s);
        s.x = w * if i == 0 {
            0.5
        } else {
            0.35 + frand(0.3) as f32
        };
        self.stars[i] = s;
    }

    fn move_stars(&mut self) {
        let w = (self.grid_w * GRID_SEG) as f32;
        let h = (self.grid_h * GRID_SEG) as f32;
        let off = self.speed * SPEED_BASE * self.resolution;

        for i in 0..self.stars.len() {
            let s = &mut self.stars[i];
            /* Move stars off screen until most of their influence fades */
            s.x += s.dx * off;
            s.y += s.dy * off;

            if s.x < -s.ro || s.y < -s.ro || s.x >= w + s.ro || s.y >= h + s.ro {
                self.respawn(i);
                let ro = self.stars[i].ro;
                self.stars[i].y = -ro;
            }
        }
    }

    /// The coarse zone: one sample per sixteen units, unless this segment was
    /// already subdivided, in which case the field is interpolated across it.
    fn calc_o(&mut self, mass: f32, cx: f32, y02: f32, from: usize, to: usize) {
        let mut x0 = cx - (from * GRID_SEG) as f32;
        let mut g0 = mass / (x0 * x0 + y02);

        for x in from..to {
            x0 = cx - ((x + 1) * GRID_SEG) as f32;
            let g1 = mass / (x0 * x0 + y02);

            self.grid[x * GRID_SEG] += g0;
            if self.segs[x] {
                let d = (g1 - g0) / GRID_SEG as f32;
                for i in 1..GRID_SEG {
                    g0 += d;
                    self.grid[x * GRID_SEG + i] += g0;
                }
            }
            g0 = g1;
        }
    }

    /// Turn coarse segments into fine ones, filling in the samples between the
    /// two ends by straight interpolation of what is there already.
    fn make_hires(&mut self, from: usize, to: usize, w: usize) {
        /* One bigger than from/to so that there's a good angle between the
        middle and inner zones.

        Don't make the last GRID_SEG high-res. This keeps the length
        consistent. */
        let from = if from > 0 { from - 1 } else { from };
        let from = (from / GRID_SEG).min(w - 1);
        let to = (to / GRID_SEG + 1).min(w - 1);

        for x in from..to {
            if !self.segs[x] {
                let g0 = self.grid[x * GRID_SEG];
                let g1 = self.grid[(x + 1) * GRID_SEG];
                let d = (g1 - g0) / GRID_SEG as f32;
                let mut g = g0;
                for i in 1..GRID_SEG {
                    g += d;
                    self.grid[x * GRID_SEG + i] = g;
                }
                self.segs[x] = true;
            }
        }
    }

    /// The middle zone: one sample per unit, the inverse square of the
    /// distance from the mass as a point source.
    fn calc_m(&mut self, mass: f32, cx: f32, y02: f32, from: usize, to: usize) {
        for x in from..to {
            let x0 = cx - x as f32;
            self.grid[x] += mass / (x0 * x0 + y02);
        }
    }

    /// One line of the grid, either across or down.
    fn draw_row(&mut self, g: &mut Gl, w: usize, y: usize, swap: bool) {
        let w2 = w * GRID_SEG;
        self.grid[..w2].fill(0.0);
        self.segs[..w].fill(false);

        for i in 0..self.stars.len() {
            let (cx, cy, mass, max, ro2, rm2, ri2) = {
                let s = &self.stars[i];
                let (cx, cy) = if swap { (s.y, s.x) } else { (s.x, s.y) };
                (cx, cy, s.mass, s.surface_gravity, s.ro2, s.rm2, s.ri2)
            };

            let y0 = cy - y as f32;
            let y02 = y0 * y0;
            if y02 > ro2 {
                continue;
            }

            let ro = (ro2 - y02).sqrt();
            let olo = wclip((cx - ro) / GRID_SEG as f32 + 1.0, w);
            let ohi = wclip((cx + ro) / GRID_SEG as f32 + 1.0, w);

            let rm = if rm2 > y02 { (rm2 - y02).sqrt() } else { 0.0 };
            let mut mlo = wclip(cx - rm + 1.0, w2);
            let mut mhi = wclip(cx + rm + 1.0, w2);

            if mlo != mhi {
                let ri = if ri2 > y02 { (ri2 - y02).sqrt() } else { 0.0 };
                let ilo = wclip(cx - ri + 1.0, w2);
                let ihi = wclip(cx + ri + 1.0, w2);

                mlo -= mlo % GRID_SEG;
                mhi += GRID_SEG - 1;
                mhi -= mhi % GRID_SEG;

                /* These go first. */
                self.make_hires(mlo, ilo, w);
                self.make_hires(ihi, mhi, w);

                self.calc_m(mass, cx, y02, mlo, ilo);
                self.calc_m(mass, cx, y02, ihi, mhi);

                /* This does a bit more work than it needs to. */
                // Inside the star's own radius the sheet is held flat, at what
                // would be felt at its surface, rather than diving to infinity.
                for x in ilo..ihi {
                    self.grid[x] += max;
                }
            }

            self.calc_o(mass, cx, y02, olo, mlo / GRID_SEG);
            self.calc_o(mass, cx, y02, mhi / GRID_SEG, ohi);
        }

        let n = self.colors.len();
        let emit = |g: &mut Gl, x: f32, z: f32| {
            let ci = ((ease(z / MAX_MASS_COLOR) * n as f32) as i32).clamp(0, n as i32 - 1) as usize;
            let c = &self.colors[ci];
            g.glx.color4f(
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            );
            // A row runs along x and a column along y, and swapping the two is
            // the whole difference between them.
            if swap {
                g.glx.vertex3f(y as f32, x, z);
            } else {
                g.glx.vertex3f(x, y as f32, z);
            }
        };

        g.glx.begin(Shape::LineStrip);
        for x in 0..w {
            if !self.segs[x] {
                emit(g, (x * GRID_SEG) as f32, self.grid[x * GRID_SEG]);
            } else {
                for i in 0..GRID_SEG {
                    emit(g, (x * GRID_SEG + i) as f32, self.grid[x * GRID_SEG + i]);
                }
            }
        }
        g.glx.end();
    }
}

impl Star {
    fn blank() -> Star {
        Star {
            mass: 0.0,
            ro2: 0.0,
            rm2: 0.0,
            ri2: 1.0,
            ro: 0.0,
            radius: 1.0,
            x: 0.0,
            y: 0.0,
            dx: 0.0,
            dy: 0.0,
            surface_gravity: 0.0,
            depth: 0.0,
        }
    }
}

impl Hack3d for GravityWell {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let gridmod = (self.grid_size * GRID_SIZE_BASE).max(1.0) as usize;

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.translate(
            -(self.grid_w as f32) * (GRID_SEG as f32 / 2.0),
            -(self.grid_h as f32) * (GRID_SEG as f32 * 0.75),
            3.0,
        );

        g.glx.line_width(2.0);
        g.glx.blend(Blend::Alpha);
        g.glx.fog(Some(Fog::Exp2 {
            density: 0.005,
            color: [0.0, 0.0, 0.0, 1.0],
        }));

        /* Find the cumulative gravitational effect at the midpoint of each
        star, for the depth of the foot-circle.  This duplicates some of the
        draw_row() logic. */
        for i in 0..self.stars.len() {
            let (x0, y0) = (self.stars[i].x, self.stars[i].y);
            let mut depth = self.stars[i].surface_gravity;
            for (j, s1) in self.stars.iter().enumerate() {
                if i == j {
                    continue;
                }
                let d2 = (s1.x - x0) * (s1.x - x0) + (s1.y - y0) * (s1.y - y0);
                depth += s1.mass / d2;
            }
            self.stars[i].depth = depth;
        }

        let mut y = 0;
        while y < (self.grid_h - 1) * GRID_SEG {
            self.draw_row(g, self.grid_w, y, false);
            y += gridmod;
        }
        let mut x = 0;
        while x < (self.grid_w - 1) * GRID_SEG {
            self.draw_row(g, self.grid_h, x, true);
            x += gridmod;
        }

        /* Draw a circle around the "footprint" at the bottom of the gravity
        well. */
        let n = self.colors.len();
        for i in 0..self.stars.len() {
            let steps = 16;
            let (sx, sy, radius, depth) = {
                let s = &self.stars[i];
                (s.x, s.y, s.radius, s.depth)
            };
            let ci =
                ((ease(depth / MAX_MASS_COLOR) * n as f32) as i32).clamp(0, n as i32 - 1) as usize;
            let c = &self.colors[ci];
            g.glx.color4f(
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            );
            g.glx.push_matrix();
            g.glx.translate(sx, sy, 0.0);
            g.glx.begin(Shape::LineLoop);
            for k in 0..steps * 2 {
                let th = std::f32::consts::PI * k as f32 / steps as f32;
                g.glx.vertex3f(radius * th.cos(), radius * th.sin(), depth);
            }
            g.glx.end();
            g.glx.pop_matrix();
        }

        g.glx.pop_matrix();

        if !self.trackball.button_down() {
            self.move_stars();
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(40.0, 1.0 / h, 10.0, 1000.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let resolution = g.res.float("resolution").clamp(1.0, 5.0);
    let grid_w = ((RESOLUTION_BASE * resolution) as usize / GRID_SEG).max(2);
    let vtx_max = grid_w * GRID_SEG;

    // The grid colour runs from one end of a two-colour ramp to the other, and
    // a line's place on it is how deep the sheet is under it.
    let hsv = |g: &Gl, key: &str| {
        let (r, gr, b) = unrgb(g.res.pixel(key));
        rgb_to_hsv(
            u16::from(r) << 8 | u16::from(r),
            u16::from(gr) << 8 | u16::from(gr),
            u16::from(b) << 8 | u16::from(b),
        )
    };
    let (h1, s1, v1) = hsv(g, "gridColor");
    let (h2, s2, v2) = hsv(g, "gridColor2");
    let colors = make_color_ramp(h1, s1, v1, h2, s2, v2, 128, false);

    let mut st = GravityWell {
        trackball: Trackball::new(),
        stars: Vec::new(),
        grid_w,
        grid_h: grid_w,
        grid: vec![0.0; vtx_max],
        segs: vec![false; grid_w],
        colors,
        speed: g.res.float("speed") as f32,
        resolution: resolution as f32,
        grid_size: g.res.float("grid-size"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    let nstars = g.res.int("count").clamp(1, 40) as usize;
    for i in 0..nstars {
        st.stars.push(Star::blank());
        st.respawn(i);
        let ro = st.stars[i].ro;
        st.stars[i].y = frand(f64::from(ro * 2.0 + (st.grid_h * GRID_SEG) as f32)) as f32 - ro;
    }

    /* Let's tilt the floor a little. */
    st.trackball.reset(-0.4 + frand(0.8), -0.3 + frand(0.2));

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        15",
    "*gridColor:    #00FF00",
    "*gridColor2:   #FF0000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*speed:        1.0",
    "*resolution:   1.0",
    "*grid-size:    1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::slider("resolution", "Resolution", 1.0, 5.0, 0.1, 1, "1.0"),
    Opt::slider("grid-size", "Grid size", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::slider("count", "Number of stars", 1.0, 40.0, 1.0, 0, "15"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "gravitywell",
    label: "Gravity Well",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2019",
        video: Some("https://www.youtube.com/watch?v=yhsw0QhIjjs"),
        blurb: "Massive objects distort space in a two dimensional universe.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid lines, which are the only strips in the frame: the stars'
    /// foot circles are loops and would otherwise be mistaken for deep sheet.
    fn sheet(r: &Runner3d) -> Vec<f32> {
        let f = r.frame();
        f.batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::LineStrip)
            .flat_map(|b| {
                f.vertices[b.first..b.first + b.count]
                    .iter()
                    .map(|v| v.pos[2])
            })
            .collect()
    }

    /// The sheet dips towards a star and is flat away from it, which is the
    /// one piece of physics in the whole saver.
    #[test]
    fn the_sheet_dips_where_a_star_is() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let mut zs = sheet(&r);
        let deepest = zs.iter().copied().fold(0.0f32, f32::max);
        assert!(deepest > 1.0, "the sheet is flat: deepest {deepest}");
        // Wells rather than a uniform slope: most of the sheet is nearly flat
        // and a small part of it is far deeper than the rest.
        zs.sort_by(f32::total_cmp);
        let median = zs[zs.len() / 2];
        assert!(
            deepest > median * 10.0,
            "deepest {deepest} against a median of {median}"
        );
    }

    /// Near a star the line is sampled sixteen times as often, which is what
    /// lets a five-hundred-unit grid have a smooth well in it at all.
    #[test]
    fn the_grid_is_finer_near_a_star() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        // A line crossing a star has more vertices than the coarse count.
        let coarse = 512 / GRID_SEG;
        let longest = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::LineStrip)
            .map(|b| b.count)
            .max()
            .unwrap_or(0);
        assert!(
            longest > coarse,
            "the longest line is {longest} for a grid {coarse} coarse segments across"
        );
    }

    /// Fog is on, or the far half of the grid is an unreadable band.
    #[test]
    fn the_grid_is_fogged() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        assert!(r.frame().batches.iter().all(|b| b.fog.is_some()));
    }

    /// The stars drift, and one that leaves the field comes back somewhere
    /// else rather than being lost.
    #[test]
    fn the_stars_keep_coming() {
        let mut r = start(StartArgs::new(640, 480, "speed=8", 20260811));
        let mut deep = 0;
        for _ in 0..300 {
            r.step();
            if sheet(&r).iter().any(|z| *z > 1.0) {
                deep += 1;
            }
        }
        assert!(deep > 150, "the sheet was only bent on {deep} frames");
    }
}
