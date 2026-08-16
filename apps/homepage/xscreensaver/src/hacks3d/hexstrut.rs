//! Port of `hacks/glx/hexstrut.c`.
//!
//! ```text
//! hexstrut, Copyright (c) 2016-2017 Jamie Zawinski <jwz@jwz.org>
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
//! A plane of triangles, each drawn as three struts reaching out from its
//! centre. Every so often one triangle starts turning, and a few frames later
//! it sets each of its neighbours turning too, so the disturbance spreads
//! across the sheet as a wave. Each triangle takes a third of a turn and stops,
//! which is why the pattern reassembles itself: at a third of a turn the struts
//! line up with a different set of neighbours and the tiling is the same again.
//!
//! Nothing here is lit: the colours come straight from a smooth colormap, and
//! a triangle takes the next colour along every frame it is turning, so the
//! wavefront is visible as a colour change as well as a movement.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// How many neighbours a triangle can have: the three that share an edge and
/// the three that share only a corner.
const NEIGHBOURS: usize = 6;

struct Triangle {
    p: [[f32; 3]; 3],
    /// Indices into the plane, `usize::MAX` for an empty slot.
    neighbors: [usize; NEIGHBOURS],
    /// How far through a third of a turn this one is, and how many complete
    /// thirds it has already made.
    orot: f32,
    rot: f32,
    delay: i32,
    odelay: i32,
    ccolor: usize,
}

struct Hexstrut {
    rot: Rotator,
    trackball: Trackball,
    triangles: Vec<Triangle>,
    colors: Vec<XColor>,
    speed: f32,
    thickness: f32,
}

/// `make_plane`. A triangular grid: each cell contributes one upward triangle,
/// and odd rows are offset half a cell, which is what makes them tile.
fn make_plane(count: i32) -> Vec<Triangle> {
    let n = (count * 2).max(2);
    let size = 2.0 / n as f32;
    let w = size;
    let h = size * 3.0f32.sqrt() / 2.0;
    let mut triangles: Vec<Triangle> = Vec::with_capacity((n * n) as usize);
    // Where each grid cell's triangle ended up, so neighbours can be linked.
    let mut grid = vec![usize::MAX; (n * n) as usize];

    for y in 0..n {
        for x in 0..n {
            let mut px = (x - n / 2) as f32 * w;
            let py = (y - n / 2) as f32 * h;
            if y & 1 != 0 {
                px += w / 2.0;
            }
            let t = Triangle {
                p: [
                    [px, py, 0.0],
                    [px - w / 2.0, py + h, 0.0],
                    [px + w / 2.0, py + h, 0.0],
                ],
                neighbors: [usize::MAX; NEIGHBOURS],
                orot: 0.0,
                rot: 0.0,
                delay: 0,
                odelay: 0,
                ccolor: 0,
            };
            let me = triangles.len();
            triangles.push(t);

            let link = |a: usize, b: usize, ts: &mut Vec<Triangle>| {
                if a != b {
                    link_neighbor(ts, a, b);
                    link_neighbor(ts, b, a);
                }
            };
            if x > 0 {
                link(me, grid[(y * n + (x - 1)) as usize], &mut triangles);
            }
            if y > 0 {
                link(me, grid[((y - 1) * n + x) as usize], &mut triangles);
                if x < n - 1 {
                    link(me, grid[((y - 1) * n + (x + 1)) as usize], &mut triangles);
                }
            }
            grid[(y * n + x) as usize] = me;
        }
    }
    triangles
}

/// Add `b` to `a`'s neighbour list, if it is not there already and there is
/// room. Upstream aborts when there is not; there never is not.
fn link_neighbor(triangles: &mut [Triangle], a: usize, b: usize) {
    if a == b || a >= triangles.len() || b >= triangles.len() {
        return;
    }
    for slot in &mut triangles[a].neighbors {
        if *slot == b {
            return;
        }
        if *slot == usize::MAX {
            *slot = b;
            return;
        }
    }
}

impl Hexstrut {
    /// Start one triangle turning now and then, and carry on the ones that are
    /// already turning, handing the disturbance to their neighbours as their
    /// delays run out.
    fn tick(&mut self) {
        let step = 0.01 + (0.04 * self.speed);
        let ncolors = self.colors.len();

        if random().is_multiple_of(80) && !self.triangles.is_empty() {
            let n = (random() as usize) % self.triangles.len();
            let t = &mut self.triangles[n];
            if t.rot == 0.0 {
                t.rot += step * if random() & 1 != 0 { 1.0 } else { -1.0 };
                t.odelay = 4;
                t.delay = 4;
            }
        }

        // Which triangles are to be woken, gathered first: upstream walks a
        // linked list and pokes its neighbours as it goes, which is the one
        // thing a Vec of indices cannot do while it is being iterated.
        let mut wake: Vec<(usize, f32, i32)> = Vec::new();
        for i in 0..self.triangles.len() {
            let t = &mut self.triangles[i];
            /* If this triangle is rotating, continue until done. */
            if t.rot != 0.0 {
                t.rot += step * if t.rot > 0.0 { 1.0 } else { -1.0 };
                t.ccolor += 1;
                if t.ccolor >= ncolors {
                    t.ccolor = 0;
                }
                if t.rot > 1.0 || t.rot < -1.0 {
                    t.orot += if t.rot > 1.0 { 1.0 } else { -1.0 };
                    t.rot = 0.0;
                }
            }

            /* If this triangle's propagation delay hasn't hit zero, decrement
            it. When it does, start its neighbors rotating. */
            if t.delay != 0 {
                t.delay -= 1;
                if t.delay == 0 {
                    let (rot, odelay) = (t.rot, t.odelay);
                    let neighbors = t.neighbors;
                    for n in neighbors {
                        if n != usize::MAX {
                            wake.push((n, rot, odelay));
                        }
                    }
                }
            }
        }
        for (i, rot, odelay) in wake {
            let step = step * if rot > 0.0 { 1.0 } else { -1.0 };
            let t = &mut self.triangles[i];
            if t.rot == 0.0 {
                t.rot += step;
                t.delay = odelay;
                t.odelay = odelay;
            }
        }
    }

    fn draw_triangles(&self, g: &mut Gl) {
        let length = 3.0f32.sqrt() / 3.0;
        let t2 = length * self.thickness / 2.0;
        let Some(first) = self.triangles.first() else {
            return;
        };
        let scale = {
            let x = first.p[0][0] - first.p[1][0];
            let y = first.p[0][1] - first.p[1][1];
            let z = first.p[0][2] - first.p[1][2];
            (x * x + y * y + z * z).sqrt()
        };

        g.glx.begin(Shape::Quads);
        g.glx.normal3f(0.0, 0.0, 1.0);
        for t in &self.triangles {
            let angle = (std::f32::consts::PI * 2.0 / 3.0) * t.rot;
            let (cr, sr) = (angle.cos(), angle.sin());
            let c = [
                (t.p[0][0] + t.p[1][0] + t.p[2][0]) / 3.0,
                (t.p[0][1] + t.p[1][1] + t.p[2][1]) / 3.0,
                (t.p[0][2] + t.p[1][2] + t.p[2][2]) / 3.0,
            ];

            let col = &self.colors[t.ccolor.min(self.colors.len() - 1)];
            /* Brighter */
            let bright = |v: u16| (f32::from(v) / 65535.0) * 0.75 + 0.25;
            g.glx
                .color4f(bright(col.red), bright(col.green), bright(col.blue), 1.0);

            for corner in &t.p {
                /* Orient to direction of corner. */
                let x = corner[0] - c[0];
                let y = corner[1] - c[1];
                let z = corner[2] - c[2];

                let smc = sr * y - cr * x;
                let spc = cr * y + sr * x;

                let st2 = t2 * scale / (x * x + y * y).sqrt();
                let slength = length * scale / (x * x + y * y + z * z).sqrt();

                let xt2 = spc * st2;
                let yt2 = smc * st2;
                let xlength = c[0] - slength * smc;
                let ylength = c[1] + slength * spc;
                let zlength = c[2] + slength * z;

                g.glx.vertex3f(c[0] - xt2, c[1] - yt2, c[2]);
                g.glx.vertex3f(c[0] + xt2, c[1] + yt2, c[2]);
                g.glx.vertex3f(xlength + xt2, ylength + yt2, zlength);
                g.glx.vertex3f(xlength - xt2, ylength - yt2, zlength);
            }
        }
        g.glx.end();
    }
}

impl Hack3d for Hexstrut {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        // The sheet is flat and its struts overlap; drawing it in order rather
        // than by depth is what upstream asks for.
        g.glx.depth_test(false);
        g.glx.clear();

        g.glx.push_matrix();
        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 6.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 12.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (_, _, z) = self.rot.rotation(!down);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(30.0, 30.0, 30.0);

        if !down {
            self.tick();
        }
        self.draw_triangles(g);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 3 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.colors = make_smooth_colormap(64);
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let wander = g.res.bool("wander");
    let spin_speed = 0.002;
    let wander_speed = 0.003;
    let spin_accel = 1.0;

    let mut st = Hexstrut {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if wander { wander_speed } else { 0.0 },
            false,
        ),
        trackball: Trackball::new(),
        triangles: Vec::new(),
        colors: make_smooth_colormap(64),
        speed: g.res.float("speed").min(2.0) as f32,
        thickness: g.res.float("thickness").clamp(0.05, 1.7) as f32,
    };

    /* Let's tilt the scene a little. */
    st.trackball.reset(-0.4 + frand(0.8), -0.4 + frand(0.8));

    st.triangles = make_plane(g.res.int("count").clamp(2, 40));

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*count:        20",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*thickness:    0.2",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Hexagon Size", 2.0, 40.0, 1.0, 0, "20"),
    Opt::slider("speed", "Speed", 0.1, 2.0, 0.1, 1, "1.0"),
    Opt::slider("thickness", "Line Thickness", 0.05, 1.7, 0.05, 2, "0.2"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hexstrut",
    label: "Hex Strut",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=iOCffj3ZmgE"),
        blurb: "A grid of hexagons composed of rotating Y-shaped struts.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
