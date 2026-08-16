//! Port of `hacks/glx/cubestorm.c`.
//!
//! ```text
//! cubestorm, Copyright (c) 2003-2018 Jamie Zawinski <jwz@jwz.org>
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
//! A few skeletal cubes tumbling through space, each leaving a trail of every
//! position it has been in. The trail builds up into a solid-looking knot of
//! frames, then the whole thing is wiped and starts again from nothing.
//!
//! Upstream's comment on how that trail is drawn is the interesting part, and
//! it is worth repeating because it is why this saver is expensive:
//!
//! ```text
//! Originally, this program achieved the "accumulating cubes" effect by
//! simply not clearing the depth or color buffers between frames.  That
//! doesn't work on modern systems, particularly mobile: you can no longer
//! rely on your buffers being unmolested once you have yielded.  So now we
//! must save and re-render every polygon.
//! ```
//!
//! So the history is a list of where each cube was on each past frame, and the
//! whole list is drawn again every frame: two hundred frames of four cubes is
//! eight hundred cubes, each of forty-eight quads. That is what the batch
//! folding in [`crate::runtime::gl`] is for. Without it this would be
//! thirty-eight thousand draw calls a frame; with it, eight hundred.
//!
//! The cubes wind clockwise, which is why this is the first port to say so
//! with `glFrontFace`: with culling on and the usual anticlockwise front, the
//! outsides would be the parts thrown away.
//!
//! The trail knob stops at four hundred rather than the thousand the XML
//! offers. At a thousand and twenty cubes that is twenty thousand frames of
//! forty-eight quads every frame, which upstream will attempt and a browser
//! will not survive.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// Where one cube was on one past frame.
struct HistCube {
    p: [f32; 3],
    r: [f32; 3],
    ccolor: usize,
}

/// One of the tumbling cubes: its own spin, and where it is in the colormap.
struct SubCube {
    rot: Rotator,
    ccolor: usize,
}

struct CubeStorm {
    trackball: Trackball,
    /// Set while the trail is being wiped, so nothing accumulates.
    clear_p: bool,
    cube_list: u32,
    colors: Vec<XColor>,
    subcubes: Vec<SubCube>,
    hist: Vec<HistCube>,
    speed: f32,
    thickness: f32,
    max_length: usize,
    count: usize,
    wireframe: bool,
}

impl CubeStorm {
    /// One face of the frame: four bars around the edge, each with the little
    /// return along its inside so the frame has thickness.
    fn draw_face(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let t = (self.thickness / 2.0).clamp(0.001, 0.5);
        let a = -0.5;
        let b = 0.5;

        g.glx.push_matrix();
        g.glx.front_face_cw(true);
        for _ in 0..4 {
            g.glx.normal3f(0.0, 0.0, -1.0);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            g.glx.vertex3f(a, a, a);
            g.glx.vertex3f(b, a, a);
            g.glx.vertex3f(b - t, a + t, a);
            g.glx.vertex3f(a + t, a + t, a);
            g.glx.end();

            g.glx.normal3f(0.0, 1.0, 0.0);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            g.glx.vertex3f(b - t, a + t, a);
            g.glx.vertex3f(b - t, a + t, a + t);
            g.glx.vertex3f(a + t, a + t, a + t);
            g.glx.vertex3f(a + t, a + t, a);
            g.glx.end();

            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
        }
        g.glx.pop_matrix();
    }

    /// All six faces, each turned to its own side of the cube.
    fn draw_faces(&self, g: &mut Gl) {
        g.glx.push_matrix();
        self.draw_face(g);
        for _ in 0..3 {
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            self.draw_face(g);
        }
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        self.draw_face(g);
        g.glx.rotate(180.0, 1.0, 0.0, 0.0);
        self.draw_face(g);
        g.glx.pop_matrix();
    }

    fn new_cube_colors(&mut self) {
        self.colors = make_smooth_colormap(128);
        let n = self.colors.len().max(1);
        for sc in &mut self.subcubes {
            sc.ccolor = (random() as usize) % n;
        }
    }

    /// Remember where every cube is this frame, dropping the oldest entries
    /// once the trail is longer than it is allowed to be.
    fn push_hist(&mut self, down: bool) {
        let count = self.count;
        if self.hist.len() > self.max_length && self.hist.len() > count && !down {
            /* Drop history off of the end. */
            self.hist.drain(0..count);
        }

        let (px, py, pz) = self.subcubes[0].rot.position(!down);
        // Every cube after the first turns relative to the first, so they move
        // as a loose group rather than independently.
        let mut base = [0.0f64; 3];
        let n = self.colors.len().max(1);
        for i in 0..count {
            let (mut rx, mut ry, mut rz) = self.subcubes[i].rot.rotation(!down);
            if i == 0 {
                base = [rx, ry, rz];
            } else {
                rx += base[0];
                ry += base[1];
                rz += base[2];
            }
            self.hist.push(HistCube {
                p: [px as f32, py as f32, pz as f32],
                r: [rx as f32, ry as f32, rz as f32],
                ccolor: self.subcubes[i].ccolor,
            });
            self.subcubes[i].ccolor = (self.subcubes[i].ccolor + 1) % n;
        }
    }
}

impl Hack3d for CubeStorm {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }

        // The trail is wiped now and then and allowed to build up again. Both
        // sides of this are random, so it is neither regular nor predictable:
        // roughly one frame in 25/speed to stop wiping, one in 200/speed to
        // start.
        let s = self.speed.max(0.01);
        if self.clear_p {
            self.hist.clear();
            if random().is_multiple_of(((25.0 / s) as u32).max(1)) {
                self.clear_p = false;
            }
        } else if random().is_multiple_of(((200.0 / s) as u32).max(1)) {
            self.clear_p = true;
            self.new_cube_colors();
        }

        let down = self.trackball.button_down();
        self.push_hist(down);

        let m = self.trackball.matrix();
        for i in 0..self.hist.len() {
            let (p, r, ccolor) = {
                let hc = &self.hist[i];
                (hc.p, hc.r, hc.ccolor)
            };
            g.glx.push_matrix();
            g.glx.scale(1.1, 1.1, 1.1);
            g.glx.translate(
                (p[0] - 0.5) * 15.0,
                (p[1] - 0.5) * 15.0,
                (p[2] - 0.5) * 30.0,
            );
            g.glx.mult_matrix(m);
            g.glx.scale(4.0, 4.0, 4.0);
            g.glx.rotate(r[0] * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(r[1] * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(r[2] * 360.0, 0.0, 0.0, 1.0);

            let c = &self.colors[ccolor.min(self.colors.len() - 1)];
            let color = [
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            ];
            if self.wireframe {
                g.glx.color4f(color[0], color[1], color[2], 1.0);
            } else {
                g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
                g.glx.material_shininess(128.0);
                g.glx.material_ambient_diffuse(color);
            }
            g.glx.call_list(self.cube_list);
            g.glx.pop_matrix();
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
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 45.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

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
        if let XEvent::KeyPress { key: ' ' } = event {
            /* Space wipes the trail without changing anything else. */
            self.hist.clear();
            return true;
        }
        if screenhack_event_helper(event) {
            self.new_cube_colors();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let wander = g.res.bool("wander");
    let speed = g.res.float("speed");
    let count = g.res.int("count").clamp(1, 20) as usize;

    let subcubes = (0..count)
        .map(|i| {
            // The first cube is the one that wanders; the rest only spin, and
            // faster changes of direction, so the group frays as it goes.
            let (wander_speed, spin_speed, spin_accel) = if i == 0 {
                (0.05 * speed, 10.0 * speed, 4.0 * speed)
            } else {
                (0.0, 4.0 * speed, 2.0 * speed)
            };
            SubCube {
                rot: Rotator::new(
                    if spin { spin_speed } else { 0.0 },
                    if spin { spin_speed } else { 0.0 },
                    if spin { spin_speed } else { 0.0 },
                    spin_accel,
                    if wander { wander_speed } else { 0.0 },
                    true,
                ),
                ccolor: 0,
            }
        })
        .collect();

    let mut st = CubeStorm {
        trackball: Trackball::new(),
        clear_p: false,
        cube_list: 0,
        colors: Vec::new(),
        subcubes,
        hist: Vec::new(),
        speed: speed as f32,
        thickness: g.res.float("thickness") as f32,
        max_length: g.res.int("length").clamp(1, 400) as usize,
        count,
        wireframe: g.res.bool("wireframe"),
    };
    st.new_cube_colors();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !st.wireframe {
        // Set after the reshape, so the light is fixed relative to the camera
        // rather than to the cubes, which is where upstream sets it too. The
        // specular colour is cyan rather than white, which is what gives the
        // struts their blue-green highlights whatever colour they are.
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    st.cube_list = g.glx.gen_lists(1);
    g.glx.new_list(st.cube_list);
    st.draw_faces(g);
    g.glx.end_list();

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        4",
    "*showFPS:      False",
    "*fpsSolid:     True",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*thickness:    0.06",
    "*length:       200",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Cubes", 1.0, 20.0, 1.0, 0, "4"),
    Opt::slider("length", "Length", 20.0, 400.0, 10.0, 0, "200"),
    Opt::slider("speed", "Speed", 0.01, 5.0, 0.01, 2, "1.0"),
    Opt::slider("thickness", "Struts", 0.01, 1.0, 0.01, 2, "0.06"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cubestorm",
    label: "Cube Storm",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=enuZbkMiqCE"),
        blurb: "Boxes change shape and intersect each other, filling space.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
