/* xscreensaver, Copyright (c) 2020-2021 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/covid19.c`.
//!
//! A cloud of SARS-CoV-2 virions, tumbling. Each is a membrane with a hundred
//! spikes on it and a few hundred proteins studded over it, at positions drawn
//! at random when the model is built, so the twenty models the saver builds at
//! startup are twenty different virus particles rather than twenty copies.
//!
//! The count is not fixed: it starts at a handful, fades out, comes back with
//! more, and works its way up to the knob's value and back down again. That is
//! why the knob is called a maximum.
//!
//! It was deferred here on the grounds that a hundred virions of a hundred
//! spikes each is too much geometry, which turned out to be exactly backwards.
//! Upstream builds every model twice, coarse and fine, and picks the coarse
//! ones as soon as there are more than forty on screen. So its default of sixty
//! is its *cheap* configuration, and the expensive one is a handful of large
//! ones. The measurement is in the test at the bottom of this file.
//!
//! What did have to be solved is that a virion is three hundred and five
//! separate triangle strips, and a strip cannot merge with the strip beside it,
//! so drawing sixty of them the way upstream does would be eighteen thousand
//! draw calls. Each model is baked once into a single strip, joined with the
//! usual pairs of repeated vertices, and a virion is then one call.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::unrgb;
use crate::runtime::gl::{Glx, Primitive, Shape};
use crate::runtime::shapes::{unit_dome, unit_sphere};
use crate::runtime::tube::tube;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random, screenhack_event_helper,
};

const SPIKE_FACES: i32 = 12;
const SPHERE_SLICES: i32 = 64;
const SPHERE_STACKS: i32 = 32;
const SPHERE_SLICES_2: i32 = 16;
const SPHERE_STACKS_2: i32 = 8;

const SPIKE_FACESB: i32 = 3;
const SPHERE_SLICESB: i32 = 10;
const SPHERE_STACKSB: i32 = 5;
const SPHERE_SLICES_2B: i32 = 5;
const SPHERE_STACKS_2B: i32 = 3;

/// How many models are built. The first half are fine and the second half
/// coarse, and a virion picks one of whichever half suits the count.
const NUM_LISTS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawMode {
    In,
    Draw,
    Out,
}

/// One virion: where it is, how big, which of the twenty models it wears.
struct Ball {
    pos: [f32; 3],
    scale: f32,
    rot: Rotator,
    dlist: usize,
}

/// A baked vertex.
#[derive(Clone, Copy)]
struct Vert {
    pos: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
}

/// One model, baked flat: several hundred triangle strips joined into one.
///
/// A strip cannot merge with the strip beside it, so a virion drawn the way
/// upstream draws it is three hundred draw calls and sixty of them is eighteen
/// thousand. Joining them with the usual pair of repeated vertices makes two
/// triangles of no area, which raster to nothing; every strip these shapes
/// emit is an even number of vertices long, so the winding of what follows a
/// join is unchanged. One call a virion.
#[derive(Default)]
struct Baked {
    verts: Vec<Vert>,
    doubling: bool,
}

impl Baked {
    fn begin(&mut self) {
        if let Some(&v) = self.verts.last() {
            self.verts.push(v);
            self.doubling = true;
        }
    }

    fn push(&mut self, v: Vert) {
        if self.doubling {
            self.verts.push(v);
            self.doubling = false;
        }
        self.verts.push(v);
    }
}

/// `unit_spike`: one of the club-shaped spikes that stud the membrane.
fn unit_spike(g: &mut Glx, color: [f32; 4], lowrez: bool, wire: bool) {
    let r = 0.2;
    let s = 0.2;
    g.push_matrix();

    g.color4f(color[0], color[1], color[2], color[3]);

    g.scale(s, s, s);
    g.translate(0.0, -r, 0.0);
    if !lowrez {
        g.translate(-r, 0.0, 0.0);
    }
    tube(
        g,
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        r,
        0.0,
        if lowrez { SPIKE_FACESB } else { SPIKE_FACES },
        true,
        false,
        wire,
    );
    if !lowrez {
        g.translate(r * 2.0, 0.0, 0.0);
        tube(
            g,
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            r,
            0.0,
            SPIKE_FACES,
            true,
            false,
            wire,
        );
        g.translate(-r, 0.0, 0.0);
    }

    g.translate(0.0, 1.0, 0.0);
    let r = r * 2.0;
    g.scale(r, r, r);

    for i in 0..(if lowrez { 1 } else { 3 }) {
        g.push_matrix();
        g.rotate(360.0 / 3.0 * i as f32, 0.0, 1.0, 0.0);
        if !lowrez {
            g.translate(r, 0.0, 0.0);
        }
        unit_sphere(
            g,
            if lowrez {
                SPHERE_STACKS_2B
            } else {
                SPHERE_STACKS_2
            },
            if lowrez {
                SPHERE_SLICES_2B
            } else {
                SPHERE_SLICES_2
            },
            wire,
        );
        g.pop_matrix();
    }

    g.pop_matrix();
}

/// The five colours a virion is made of, in the order `unit_ball` draws them.
struct Colors {
    membrane: [f32; 4],
    spike: [f32; 4],
    mp: [f32; 4],
    ep: [f32; 4],
    hes: [f32; 4],
}

/// `unit_ball`: one whole virion, built once into the oven.
///
/// The spikes and the proteins are put on at random, so every call builds a
/// different particle, which is why upstream builds twenty of them.
fn unit_ball(g: &mut Glx, c: &Colors, lowrez: bool, wire: bool) {
    for f in 0..5 {
        match f {
            // MEMBRANE
            0 => {
                g.color4f(c.membrane[0], c.membrane[1], c.membrane[2], c.membrane[3]);
                unit_sphere(
                    g,
                    if lowrez {
                        SPHERE_STACKSB
                    } else {
                        SPHERE_STACKS
                    },
                    if lowrez {
                        SPHERE_SLICESB
                    } else {
                        SPHERE_SLICES
                    },
                    wire,
                );
            }
            // SPIKE
            1 => {
                let th0 = 0.5f32.atan(); /* lat division: 26.57 deg */
                let s = std::f32::consts::PI / 5.0; /* lon division: 72 deg */
                let n = if lowrez { 8 } else { 10 };
                for j in 0..n {
                    let _ = j;
                    for i in 0..n {
                        let th1 = s * i as f32;
                        let mut a = th0;
                        let mut o = th1;

                        a += (0.2 + frand(0.9) as f32) * randsign();
                        o += (0.2 + frand(0.9) as f32) * randsign();

                        let x = a.cos() * o.cos();
                        let y = a.cos() * o.sin();
                        let z = a.sin();

                        g.push_matrix();
                        if i & 1 == 0 {
                            g.rotate(180.0, 0.0, 1.0, 0.0);
                            g.rotate(180.0 / 5.0, 0.0, 0.0, 1.0);
                        }
                        g.translate(x, y, z);
                        g.rotate(-x.atan2(y).to_degrees(), 0.0, 0.0, 1.0);
                        g.rotate(z.atan2((x * x + y * y).sqrt()).to_degrees(), 1.0, 0.0, 0.0);
                        unit_spike(g, c.spike, lowrez, wire);
                        g.pop_matrix();
                    }
                }

                g.push_matrix();
                g.rotate(90.0, 1.0, 0.0, 0.0);
                g.translate(0.0, 1.0, 0.0);
                unit_spike(g, c.spike, lowrez, wire);
                g.translate(0.0, -2.0, 0.0);
                g.rotate(180.0, 1.0, 0.0, 0.0);
                unit_spike(g, c.spike, lowrez, wire);
                g.pop_matrix();
            }
            // M_PROTEIN, E_PROTEIN, HES
            _ => {
                let mut s = 0.04;
                let mut n = if lowrez { 50 } else { 200 };
                let col = match f {
                    2 => c.mp,
                    3 => c.ep,
                    _ => c.hes,
                };
                g.color4f(col[0], col[1], col[2], col[3]);
                if f == 4 {
                    s *= 1.5;
                    n /= 8;
                }
                for _ in 0..n {
                    g.push_matrix();
                    g.rotate((random() % 360) as f32, 1.0, 0.0, 0.0);
                    g.rotate((random() % 180) as f32, 0.0, 1.0, 0.0);
                    g.translate(1.0, 0.0, 0.0);
                    g.rotate(90.0, 0.0, 0.0, 1.0);
                    g.scale(s, s, s);
                    unit_dome(
                        g,
                        if lowrez {
                            SPHERE_STACKS_2B
                        } else {
                            SPHERE_STACKS_2
                        },
                        if lowrez {
                            SPHERE_SLICES_2B
                        } else {
                            SPHERE_SLICES_2
                        },
                        wire,
                    );
                    g.pop_matrix();
                }
            }
        }
    }
}

fn randsign() -> f32 {
    if random() & 1 != 0 { 1.0 } else { -1.0 }
}

/// Build one model and flatten it.
///
/// The oven is the trick `pipes` uses: the shape is drawn into a `Glx` that is
/// never shown, and what comes back out is the vertices with the matrix each
/// was drawn under already multiplied through, so a virion can be drawn
/// anywhere without rebuilding it.
fn bake(c: &Colors, lowrez: bool, wire: bool) -> Baked {
    let mut oven = Glx::new();
    oven.start_frame(1, 1);
    oven.matrix_mode_modelview();
    oven.load_identity();
    unit_ball(&mut oven, c, lowrez, wire);

    let mut out = Baked::default();
    let frame = oven.frame();
    for b in &frame.batches {
        if b.count == 0 || b.primitive != Primitive::TriangleStrip {
            continue;
        }
        let m = b.modelview;
        out.begin();
        for v in &frame.vertices[b.first..b.first + b.count] {
            let n = {
                let a = &m.0;
                let n = v.normal;
                let o = [
                    a[0] * n[0] + a[4] * n[1] + a[8] * n[2],
                    a[1] * n[0] + a[5] * n[1] + a[9] * n[2],
                    a[2] * n[0] + a[6] * n[1] + a[10] * n[2],
                ];
                let d = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
                if d == 0.0 {
                    o
                } else {
                    [o[0] / d, o[1] / d, o[2] / d]
                }
            };
            out.push(Vert {
                pos: m.transform(v.pos),
                normal: n,
                color: v.color,
            });
        }
    }
    out
}

struct Covid19State {
    mode: DrawMode,
    tick: f32,
    lists: Vec<Baked>,
    max_balls: i32,
    count: i32,
    ball_delta: i32,
    balls: Vec<Ball>,
    trackball: Trackball,
    speed: f32,
    do_spin: bool,
    do_wander: bool,
    aspect: f32,
}

impl Covid19State {
    /// `make_balls`: lay `count` virions out in a grid that fills the window.
    fn make_balls(&mut self, count: i32) {
        /* Distribute the balls into a rectangular grid that fills the window.
        There may be some empty cells.  N items in a W x H rectangle:
        N = W * H
        N = W * W * R
        N/R = W*W
        W = sqrt(N/R) */
        let aspect = self.aspect;
        let nlines = ((count as f32 / aspect).sqrt() + 0.5) as usize;
        let nlines = nlines.max(1);
        let lowrez = count > 40;

        let mut cols = vec![0i32; nlines];
        let mut max = 0;
        for i in 0..count as usize {
            cols[i % nlines] += 1;
            max = max.max(cols[i % nlines]);
        }
        /* That gave us, e.g. 7777666. Redistribute to 6767767. */
        let mut i = 0;
        while i < nlines / 2 {
            let j = nlines - i - 1;
            cols.swap(i, j);
            i += 2;
        }

        let mut scale = 1.0 / nlines as f32; /* Scale for height */
        if scale * max as f32 > aspect {
            /* Shrink if overshot width */
            scale *= aspect / (scale * max as f32);
        }
        scale *= 0.9; /* Add padding */
        let mut spacing = scale * 4.0;
        if count == 1 {
            spacing = 0.0;
        }

        self.balls.clear();
        let n = NUM_LISTS / 2;
        for (y, &col) in cols.iter().enumerate() {
            for x in 0..col {
                let spin_speed = 1.0 * f64::from(self.speed);
                let wander_speed = 0.04 * f64::from(self.speed);
                self.balls.push(Ball {
                    pos: [
                        spacing * (x as f32 - col as f32 / 2.0) + spacing / 2.0,
                        spacing * (y as f32 - nlines as f32 / 2.0) + spacing / 2.0,
                        0.0,
                    ],
                    scale,
                    dlist: (random() as usize % n) + if lowrez { n } else { 0 },
                    rot: Rotator::new(
                        if self.do_spin { spin_speed } else { 0.0 },
                        if self.do_spin { spin_speed } else { 0.0 },
                        if self.do_spin { spin_speed } else { 0.0 },
                        1.0,
                        if self.do_wander { wander_speed } else { 0.0 },
                        true,
                    ),
                });
            }
        }
        self.count = count;
    }
}

impl Hack3d for Covid19State {
    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = f64::from(height) / f64::from(width.max(1));
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = f64::from(height) / f64::from(width);
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, (1.0 / h) as f32, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        self.aspect = width as f32 / height.max(1) as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            self.mode = DrawMode::Out;
            self.tick = 1.0;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.color_material(true);

        g.glx.push_matrix();
        g.glx.scale(4.0, 4.0, 4.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);

        let s;
        match self.mode {
            DrawMode::Draw => {
                self.tick -= 1.0 / 30.0 / 5.0; /* No more often than 5 sec */
                if self.tick <= 0.0 {
                    self.tick = 1.0;
                    if random().is_multiple_of(20) {
                        self.mode = DrawMode::Out;
                        self.tick = 1.0;
                    }
                }
                s = 1.0;
            }
            DrawMode::In => {
                self.tick += 1.0 / 12.0;
                if self.tick >= 1.0 {
                    self.tick = 1.0;
                    self.mode = DrawMode::Draw;
                }
                s = self.tick;
            }
            DrawMode::Out => {
                self.tick -= 1.0 / 12.0;
                let mut t = self.tick;
                if self.tick <= 0.0 {
                    self.tick = 0.0;
                    self.mode = DrawMode::In;
                    let n = if self.count < 5 {
                        2
                    } else if self.count < 20 {
                        5
                    } else {
                        20
                    };
                    let mut c2 = self.count + (1 + (random() % n) as i32) * self.ball_delta;
                    if c2 < 1 {
                        c2 = 1;
                        self.ball_delta = 1;
                    } else if c2 > self.max_balls {
                        c2 = self.max_balls;
                        self.ball_delta = -1;
                    }
                    self.make_balls(c2);
                    t = 0.0;
                }
                s = t;
            }
        }

        if s > 0.0 {
            let button_down = self.trackball.button_down();
            for i in 0..self.balls.len() {
                let (pos, scale, dlist) = {
                    let b = &self.balls[i];
                    (b.pos, b.scale, b.dlist)
                };
                let far = if self.count > 8 { 3.0 } else { 1.0 };
                let (px, py, pz) = self.balls[i].rot.position(!button_down);
                let (rx, ry, rz) = self.balls[i].rot.rotation(!button_down);

                g.glx.push_matrix();
                g.glx.translate(pos[0], pos[1], pos[2]);
                g.glx.scale(scale, scale, scale);
                g.glx.translate(
                    (px as f32 - 0.5) * 2.0,
                    (py as f32 - 0.5) * 2.0,
                    (pz as f32 - 0.5) * 8.0 * far,
                );
                g.glx.rotate(rx as f32 * 360.0, 1.0, 0.0, 0.0);
                g.glx.rotate(ry as f32 * 360.0, 0.0, 1.0, 0.0);
                g.glx.rotate(rz as f32 * 360.0, 0.0, 0.0, 1.0);
                g.glx.scale(s, s, s);

                let baked = &self.lists[dlist];
                g.glx.begin(Shape::TriangleStrip);
                for v in &baked.verts {
                    g.glx
                        .color4f(v.color[0], v.color[1], v.color[2], v.color[3]);
                    g.glx.normal3f(v.normal[0], v.normal[1], v.normal[2]);
                    g.glx.vertex3f(v.pos[0], v.pos[1], v.pos[2]);
                }
                g.glx.end();
                g.glx.pop_matrix();
            }
        }
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }
}

/// A colour resource as the four floats the GL side wants.
fn color_of(g: &Gl, key: &str) -> [f32; 4] {
    let (r, gr, b) = unrgb(g.res.pixel(key));
    [
        f32::from(r) / 255.0,
        f32::from(gr) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    ]
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let colors = Colors {
        membrane: color_of(g, "membraneColor"),
        spike: color_of(g, "spikeColor"),
        mp: color_of(g, "mpColor"),
        ep: color_of(g, "epColor"),
        hes: color_of(g, "hesColor"),
    };

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    // Twenty models: the first eleven fine and the rest coarse. Upstream's
    // `i > countof/2` rather than `>=` makes the eleventh a fine one that the
    // coarse half then draws from, and that is kept.
    let lists: Vec<Baked> = (0..NUM_LISTS)
        .map(|i| bake(&colors, i > NUM_LISTS / 2, wire))
        .collect();

    let max_balls = g.res.int("count").max(1);
    let count = if max_balls > 10 {
        1 + (random() % 5) as i32
    } else if max_balls > 5 {
        1 + (random() % 3) as i32
    } else {
        1
    };

    let mut st = Covid19State {
        mode: DrawMode::Draw,
        tick: 1.0,
        lists,
        max_balls,
        count,
        ball_delta: 1,
        balls: Vec::new(),
        trackball: Trackball::new(),
        speed: g.res.float("speed") as f32,
        do_spin: g.res.bool("spin"),
        do_wander: g.res.bool("wander"),
        aspect: 1.0,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    st.make_balls(count);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:         30000",
    "*count:         60",
    "*showFPS:       False",
    "*wireframe:     False",
    "*membraneColor: #AAFFAA",
    "*spikeColor:    #DD0000",
    "*mpColor:       #8888FF",
    "*epColor:       #FF8888",
    "*hesColor:      #880088",
    "*suppressRotationAnimation: True",
    "*spin:          True",
    "*wander:        True",
    "*speed:         1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 4.0, 0.1, 1, "1.0"),
    Opt::slider("count", "Max virus count", 1.0, 400.0, 1.0, 0, "60"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "covid19",
    label: "COVID19",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2020",
        video: Some("https://www.youtube.com/watch?v=xJDxZXbO8mY"),
        blurb: "A cloud of SARS-CoV-2 virions, tumbling.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

/// The same saver under a name that does not date it, which is what upstream
/// ships to the App Store. Upstream generates the second file from the first
/// at build time; here it is the same code with a different slug.
pub mod renamed {
    use super::*;

    pub static DEF: SaverDef = SaverDef {
        slug: "co____9",
        label: "Co____9",
        defaults: super::DEFAULTS,
        opts: super::OPTS,
        about: About {
            author: "Jamie Zawinski",
            year: "2020",
            video: Some("https://www.youtube.com/watch?v=xJDxZXbO8mY"),
            blurb: "A cloud of SARS-CoV-2 virions, tumbling.",
        },
    };

    pub fn start(args: StartArgs) -> Runner3d {
        Runner3d::start(&DEF, super::init, args)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// The count is not fixed: it fades out, comes back with more, and works
    /// its way up to the maximum and back down.
    ///
    /// Upstream only considers changing it once every hundred and fifty
    /// frames, and then only one time in twenty, so a run long enough to watch
    /// it happen is not something to put in a test suite. The walk itself is
    /// what is checked here, by stepping it directly.
    #[test]
    fn the_cloud_grows_and_shrinks() {
        let mut r = start(StartArgs::new(640, 480, "count=60", 20260812));
        r.step();
        let mut st = Covid19State {
            mode: DrawMode::Draw,
            tick: 1.0,
            lists: Vec::new(),
            max_balls: 60,
            count: 1,
            ball_delta: 1,
            balls: Vec::new(),
            trackball: Trackball::new(),
            speed: 1.0,
            do_spin: true,
            do_wander: true,
            aspect: 640.0 / 480.0,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut hit_max = false;
        let mut came_back = false;
        for _ in 0..200 {
            let n = if st.count < 5 {
                2
            } else if st.count < 20 {
                5
            } else {
                20
            };
            let mut c2 = st.count + (1 + (random() % n) as i32) * st.ball_delta;
            if c2 < 1 {
                c2 = 1;
                st.ball_delta = 1;
                came_back = true;
            } else if c2 > st.max_balls {
                c2 = st.max_balls;
                st.ball_delta = -1;
                hit_max = true;
            }
            st.make_balls(c2);
            assert_eq!(st.balls.len(), c2 as usize, "asked for {c2} virions");
            seen.insert(c2);
        }
        assert!(hit_max, "it never reached the maximum");
        assert!(came_back, "it never came back down to one");
        assert!(seen.len() > 10, "only {} sizes", seen.len());
    }

    /// The virions are laid out in a grid that fills the window rather than
    /// piled up in the middle.
    #[test]
    fn the_virions_fill_the_window() {
        let r = run("count=30", 30);
        let f = r.frame();
        let mut lo = [f32::MAX; 2];
        let mut hi = [-f32::MAX; 2];
        for b in &f.batches {
            // The middle of each virion, which is where its matrix puts it.
            for k in 0..2 {
                lo[k] = lo[k].min(b.modelview.0[12 + k]);
                hi[k] = hi[k].max(b.modelview.0[12 + k]);
            }
        }
        assert!(hi[0] > lo[0], "they were all in one column");
        assert!(hi[1] > lo[1], "they were all in one row");
    }

    /// One virion is one draw call, however many strips it is made of.
    #[test]
    fn a_virion_is_one_draw_call() {
        let r = run("count=60", 400);
        let f = r.frame();
        assert!(!f.batches.is_empty());
        // A batch per virion, and nothing else is drawn.
        assert!(
            f.batches.len() <= 60,
            "{} batches for {} vertices",
            f.batches.len(),
            f.vertices.len()
        );
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == Primitive::TriangleStrip),
            "something was not a strip"
        );
    }

    /// The measurement the deferral turned on, and it is the other way round
    /// from what the deferral assumed: upstream draws the *coarse* models once
    /// there are more than forty virions, so its default of sixty is cheaper
    /// than a dozen large ones.
    #[test]
    fn many_small_virions_are_cheaper_than_few_large_ones() {
        let colors = Colors {
            membrane: [1.0; 4],
            spike: [1.0; 4],
            mp: [1.0; 4],
            ep: [1.0; 4],
            hes: [1.0; 4],
        };
        crate::runtime::ya_rand_init(20260812);
        let fine = bake(&colors, false, false).verts.len();
        let coarse = bake(&colors, true, false).verts.len();
        assert!(
            coarse * 8 < fine,
            "coarse {coarse} against fine {fine}: not the saving upstream counts on"
        );
        // Sixty coarse ones against upstream's own threshold of forty fine.
        assert!(
            coarse * 60 < fine * 40,
            "sixty coarse ({}) is not cheaper than forty fine ({})",
            coarse * 60,
            fine * 40
        );
        assert!(
            coarse * 60 < 700_000,
            "a full cloud comes to {} vertices",
            coarse * 60
        );
    }

    /// Every strip a virion is made of is an even number of vertices long,
    /// which is what makes joining them safe: an odd one would flip the
    /// winding of everything after it and cull half the model away.
    #[test]
    fn every_strip_is_an_even_length() {
        let colors = Colors {
            membrane: [1.0; 4],
            spike: [1.0; 4],
            mp: [1.0; 4],
            ep: [1.0; 4],
            hes: [1.0; 4],
        };
        crate::runtime::ya_rand_init(20260812);
        for lowrez in [false, true] {
            let mut oven = Glx::new();
            oven.start_frame(1, 1);
            unit_ball(&mut oven, &colors, lowrez, false);
            let f = oven.frame();
            let mut strips = 0;
            for b in &f.batches {
                assert_eq!(
                    b.primitive,
                    Primitive::TriangleStrip,
                    "a virion drew something that is not a strip"
                );
                assert!(
                    b.count.is_multiple_of(2),
                    "a strip of {} vertices, which is odd",
                    b.count
                );
                strips += 1;
            }
            assert!(strips > 100, "only {strips} strips");
        }
    }

    /// The two slugs are the same saver.
    #[test]
    fn the_renamed_slug_is_the_same_saver() {
        assert_eq!(DEF.defaults, renamed::DEF.defaults);
        assert_eq!(DEF.opts.len(), renamed::DEF.opts.len());
        let a = run("count=8", 20);
        let mut b = renamed::start(StartArgs::new(640, 480, "count=8", 20260812));
        for _ in 0..20 {
            b.step();
        }
        assert_eq!(a.frame().vertices.len(), b.frame().vertices.len());
    }
}
