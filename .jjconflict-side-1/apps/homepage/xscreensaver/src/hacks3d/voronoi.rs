//! Port of `hacks/glx/voronoi.c`.
//!
//! ```text
//! voronoi, Copyright © 2007-2025 Jamie Zawinski <jwz@jwz.org>
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
//! A Voronoi diagram: the plane divided into the region nearest each of a set
//! of points. Points drift, are added a few at a time, and now and then the
//! whole thing zooms in on one of them until they have drifted off the edges
//! and the field starts again.
//!
//! Nothing here computes a Voronoi diagram. Each site is drawn as a *cone*
//! standing on the plane, apex towards the camera, all the same size, and the
//! depth buffer does the rest: at any pixel the cone that reaches nearest is
//! the one whose apex is closest in the plane, so the visible patchwork is
//! exactly the diagram. It is one of the oldest tricks in graphics and it
//! costs nothing but a depth test.
//!
//! Which is why the depth buffer is cleared in the middle of the frame. The
//! cones fill the whole depth range, so the little markers that show where the
//! sites are would be buried inside them; clearing first puts them on top.
//!
//! Two upstream calls have no equivalent here and are left out: `GL_POINT_SMOOTH`
//! (WebGL has no antialiased points) and a line width below one, which no
//! browser honours anyway. Both only affect the small markers.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

/// How far outside the unit square a site may drift before it is forgotten.
const LIM: f32 = 5.0;

/// Three samples averaged, so the middle is commoner than the edges.
fn bellrand(n: f32) -> f32 {
    (frand(f64::from(n)) + frand(f64::from(n)) + frand(f64::from(n))) as f32 / 3.0
}

struct Node {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    ddx: f32,
    ddy: f32,
    color: [f32; 4],
    /// The same colour darkened, for the marker.
    color2: [f32; 4],
    rot: i32,
}

/// What the field is doing: sitting still, growing new sites, or zooming in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Waiting,
    Adding,
    Zooming,
}

struct Voronoi {
    nodes: Vec<Node>,
    /// Index of the node the pointer has hold of.
    dragging: Option<usize>,
    colors: Vec<XColor>,
    point_size: f32,
    mode: Mode,
    adding: i32,
    last_time: f64,
    /// 1.0 starting zoom, 0.0 no longer zooming.
    zooming: f32,
    zoom_toward: [f32; 2],
    npoints: i32,
    point_speed: f32,
    point_delay: f64,
    zoom_speed: f32,
    zoom_delay: f64,
}

impl Voronoi {
    fn add_node(&mut self, x: f32, y: f32) -> usize {
        let i = (random() as usize) % self.colors.len().max(1);
        let c = &self.colors[i];
        let color = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            1.0,
        ];
        let sign = || if random() & 1 != 0 { 1.0 } else { -1.0 };
        self.nodes.push(Node {
            x,
            y,
            dx: 0.0,
            dy: 0.0,
            ddx: frand(f64::from(0.000_001 * self.point_speed)) as f32 * sign(),
            ddy: frand(f64::from(0.000_001 * self.point_speed)) as f32 * sign(),
            color,
            color2: [color[0] * 0.7, color[1] * 0.7, color[2] * 0.7, 1.0],
            rot: ((random() % 360) as i32) * if random() & 1 != 0 { 1 } else { -1 },
        });
        self.nodes.len() - 1
    }

    /// One cone, apex towards the viewer, standing on the plane. Sixty-four
    /// sides is enough that the joins between regions look straight.
    fn cone(g: &mut Gl) {
        let faces = 64;
        let step = std::f32::consts::PI * 2.0 / faces as f32;
        g.glx.begin(Shape::TriangleFan);
        g.glx.vertex3f(0.0, 0.0, 1.0);
        let (mut x, mut y, mut th) = (1.0f32, 0.0f32, 0.0f32);
        for _ in 0..faces {
            g.glx.vertex3f(x, y, 0.0);
            th += step;
            x = th.cos();
            y = th.sin();
        }
        g.glx.vertex3f(1.0, 0.0, 0.0);
        g.glx.end();
    }

    fn move_points(&mut self) {
        let waiting = self.mode == Mode::Waiting;
        for (i, nn) in self.nodes.iter_mut().enumerate() {
            if self.dragging == Some(i) {
                continue;
            }
            nn.x += nn.dx;
            nn.y += nn.dy;
            if waiting {
                nn.dx += nn.ddx;
                nn.dy += nn.ddy;
            }
        }
    }

    /// Forget the sites that have drifted away. Upstream walks a linked list
    /// and, through a quirk of its loop, stops early when the one it removes is
    /// at the head; the difference is invisible, because a site out of bounds
    /// is not drawn either way.
    fn prune_points(&mut self) {
        let dragged = self.dragging.map(|i| (self.nodes[i].x, self.nodes[i].y));
        self.nodes
            .retain(|n| n.x >= -LIM && n.x <= LIM && n.y >= -LIM && n.y <= LIM);
        // The index may have moved; find it again by where it was.
        self.dragging =
            dragged.and_then(|(x, y)| self.nodes.iter().position(|n| n.x == x && n.y == y));
    }

    /// Push every site away from the one being zoomed towards, a little more
    /// each frame, easing off as the zoom finishes.
    fn zoom_points(&mut self) {
        let tick = (self.zooming * std::f32::consts::PI).sin();
        let scale = (1.0 + (tick * 0.02 * self.zoom_speed)).max(1.0);

        self.zooming -= 0.01 * self.zoom_speed;
        if self.zooming < 0.0 {
            self.zooming = 0.0;
        }
        if self.zooming <= 0.0 {
            return;
        }
        for nn in &mut self.nodes {
            nn.x = (nn.x - self.zoom_toward[0]) * scale + self.zoom_toward[0];
            nn.y = (nn.y - self.zoom_toward[1]) * scale + self.zoom_toward[1];
        }
    }

    fn draw_cells(&mut self, g: &mut Gl) {
        for i in 0..self.nodes.len() {
            let (x, y, color) = {
                let n = &self.nodes[i];
                (n.x, n.y, n.color)
            };
            if !(-LIM..=LIM).contains(&x) || !(-LIM..=LIM).contains(&y) {
                continue;
            }
            g.glx.push_matrix();
            g.glx.translate(x, y, 0.0);
            g.glx.scale(LIM * 2.0, LIM * 2.0, 1.0);
            g.glx.color4f(color[0], color[1], color[2], color[3]);
            Voronoi::cone(g);
            g.glx.pop_matrix();
        }

        // The markers go on top of the cones rather than inside them.
        g.glx.clear_depth();

        if self.point_size <= 0.0 {
            return;
        }
        if self.point_size < 3.0 {
            g.glx.point_size(self.point_size);
            for n in &self.nodes {
                g.glx.begin(Shape::Points);
                g.glx
                    .color4f(n.color2[0], n.color2[1], n.color2[2], n.color2[3]);
                g.glx.vertex3f(n.x, n.y, 0.0);
                g.glx.end();
            }
            return;
        }

        // Big enough to be a shape rather than a dot: a five-pointed star,
        // turning, its size fixed in pixels rather than in the field.
        let (w, h) = (g.width().max(1) as f32, g.height().max(1) as f32);
        let s = self.point_size;
        for i in 0..self.nodes.len() {
            let (x, y, color2) = {
                let n = &mut self.nodes[i];
                n.rot += if n.rot < 0 { -1 } else { 1 };
                (n.x, n.y, n.color2)
            };
            let rot = self.nodes[i].rot as f32;
            g.glx.color4f(color2[0], color2[1], color2[2], color2[3]);
            g.glx.push_matrix();
            g.glx.translate(x, y, 0.0);
            g.glx.scale(s / w, s / h, 1.0);
            g.glx.line_width(self.point_size / 10.0);
            g.glx.rotate(rot, 0.0, 0.0, 1.0);
            g.glx.rotate(180.0, 0.0, 0.0, 1.0);
            for _ in 0..5 {
                g.glx.begin(Shape::Triangles);
                g.glx.vertex3f(0.0, 1.0, 0.0);
                g.glx.vertex3f(-0.2, 0.0, 0.0);
                g.glx.vertex3f(0.2, 0.0, 0.0);
                g.glx.end();
                g.glx.rotate(360.0 / 5.0, 0.0, 0.0, 1.0);
            }
            g.glx.pop_matrix();
        }
    }

    /// The cycle: wait, then zoom in, then add points to fill the space the
    /// zoom made, then wait again.
    fn state_change(&mut self, now: f64) {
        if self.dragging.is_some() {
            self.last_time = now;
            self.adding = 0;
            self.zooming = 0.0;
            return;
        }

        match self.mode {
            Mode::Waiting => {
                if self.last_time + self.zoom_delay <= now {
                    let (tx, ty) = self.nodes.first().map_or((0.5, 0.5), |n| (n.x, n.y));
                    self.zoom_toward = [tx, ty];
                    self.mode = Mode::Zooming;
                    self.zooming = 1.0;
                    self.last_time = now;
                }
            }
            Mode::Adding => {
                if self.last_time + self.point_delay <= now {
                    let (x, y) = (bellrand(0.5) + 0.25, bellrand(0.5) + 0.25);
                    self.add_node(x, y);
                    self.last_time = now;
                    self.adding -= 1;
                    if self.adding <= 0 {
                        self.adding = 0;
                        self.mode = Mode::Waiting;
                        self.last_time = now;
                    }
                }
            }
            Mode::Zooming => {
                self.zoom_points();
                if self.zooming <= 0.0 {
                    self.mode = Mode::Adding;
                    self.adding = self.npoints;
                    self.last_time = now;
                }
            }
        }
    }

    /// The site near a point, if there is one. The tolerance is in pixels, so
    /// a site is as easy to grab whatever the window size.
    fn find_node(&self, g: &Gl, x: f32, y: f32) -> Option<usize> {
        let ps = self.point_size.max(5.0);
        let hysteresis = ps / g.width().max(1) as f32;
        self.nodes.iter().position(|n| {
            n.x > x - hysteresis
                && n.x < x + hysteresis
                && n.y > y - hysteresis
                && n.y < y + hysteresis
        })
    }
}

impl Hack3d for Voronoi {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.clear();

        self.draw_cells(g);
        self.move_points();
        self.prune_points();
        let now = g.time;
        self.state_change(now);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        // The field is the unit square with y downwards, which is why the
        // pointer's window coordinates go straight in as positions.
        g.glx.ortho(0.0, 1.0, 1.0, 0.0, -1.0, 1.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.clear();
    }

    /// The pointer picks a site up, or makes one where there was none.
    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        let (w, h) = (g.width().max(1) as f32, g.height().max(1) as f32);
        match event {
            XEvent::ButtonPress { x, y, .. } => {
                let (x, y) = (*x as f32 / w, *y as f32 / h);
                let n = self
                    .find_node(g, x, y)
                    .unwrap_or_else(|| self.add_node(x, y));
                self.dragging = Some(n);
                true
            }
            XEvent::ButtonRelease { .. } if self.dragging.is_some() => {
                self.dragging = None;
                true
            }
            XEvent::MotionNotify { x, y } => {
                let Some(i) = self.dragging else { return false };
                self.nodes[i].x = *x as f32 / w;
                self.nodes[i].y = *y as f32 / h;
                true
            }
            _ => false,
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut point_size = g.res.float("pointSize") as f32;
    if point_size < 0.0 {
        point_size = 10.0;
    }
    if g.width() > 2560 {
        point_size *= 2.0; /* Retina displays */
    }
    let npoints = g.res.int("points").clamp(1, 200);

    let mut st = Voronoi {
        nodes: Vec::new(),
        dragging: None,
        colors: make_smooth_colormap(128),
        point_size,
        mode: Mode::Adding,
        adding: npoints * 2,
        last_time: 0.0,
        zooming: 0.0,
        zoom_toward: [0.5, 0.5],
        npoints,
        point_speed: g.res.float("pointSpeed") as f32,
        point_delay: g.res.float("pointDelay"),
        zoom_speed: g.res.float("zoomSpeed") as f32,
        zoom_delay: g.res.float("zoomDelay"),
    };
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*suppressRotationAnimation: True",
    "*points:       25",
    "*pointSize:    9",
    "*pointSpeed:   1.0",
    "*pointDelay:   0.05",
    "*zoomSpeed:    1.0",
    "*zoomDelay:    15",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("points", "Points", 1.0, 100.0, 1.0, 0, "25"),
    Opt::slider("pointSize", "Point size", 0.0, 50.0, 1.0, 0, "9"),
    Opt::slider("pointSpeed", "Wander speed", 0.0, 10.0, 0.1, 1, "1.0"),
    Opt::slider("pointDelay", "Insertion speed", 0.0, 3.0, 0.01, 2, "0.05").inverted(),
    Opt::slider("zoomSpeed", "Zoom speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("zoomDelay", "Zoom frequency", 0.0, 60.0, 1.0, 0, "15").inverted(),
];

pub static DEF: SaverDef = SaverDef {
    slug: "voronoi",
    label: "Voronoi",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=hD_8cBvknUM"),
        blurb: "A Voronoi diagram of moving points, zooming in for ever.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
