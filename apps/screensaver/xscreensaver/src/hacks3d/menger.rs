//! Port of `hacks/glx/menger.c`.
//!
//! ```text
//! menger, Copyright (c) 2001-2014 Jamie Zawinski <jwz@jwz.org>
//!         Copyright (c) 2002 Aurelien Jacobs <aurel@gnuage.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Generates a 3D Menger Sponge gasket.
//!
//!  The straightforward way to generate this object creates way more polygons
//!  than are needed, since there end up being many buried, interior faces.
//!  So during the recursive building of the object we store which face of
//!  each unitary cube we need to draw. Doing this reduces the polygon count
//!  by 40% - 60%.
//! ```
//!
//! A cube with a square hole bored through each face, and the same again in
//! each of the twenty smaller cubes that remain, for as many levels as it is
//! set to. It counts up to the deepest level and then back down, so the sponge
//! is forever being eaten away and rebuilt.
//!
//! The face bookkeeping in [`recurse`] is the whole trick, and the comment
//! above is the reason for it: a sponge drawn naively is mostly faces buried
//! inside other faces. Each of the twenty-seven sub-cubes inherits its parent's
//! faces, then loses the ones that are now against a neighbour and gains back
//! the ones that a hole has just exposed.
//!
//! It draws with two lights rather than one, which is what stops the far side
//! of the sponge from going flat black as it turns.
//!
//! Two knobs differ from what the XML offers. Its `spin` is a menu of axes,
//! `--spin XY` and so on, but the C reads that resource as a *boolean*, so
//! every one of those settings except the default turns spinning off entirely.
//! The C is what runs, so this is a checkbox. And its depth goes to six, which
//! is twenty to the sixth cubes: upstream will happily try, and grind to a
//! halt. It stops at four here, where a rebuild is still a few milliseconds.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent,
    screenhack_event_helper,
};

/// Which faces of a cube to draw, as a set of bits. The three lists the sponge
/// is built into are the X-facing, Y-facing and Z-facing ones.
const X0: u8 = 0x01;
const X1: u8 = 0x02;
const Y0: u8 = 0x04;
const Y1: u8 = 0x08;
const Z0: u8 = 0x10;
const Z1: u8 = 0x20;

/// The three axes, so the three display lists, so the three colours.
const AXES: usize = 3;

struct Sponge {
    lists: [u32; AXES],
    rot: Rotator,
    trackball: Trackball,
    current_depth: i32,
    max_depth: i32,
    speed: i32,
    draw_tick: i32,
    colors: Vec<XColor>,
    ccolor: [usize; AXES],
    wireframe: bool,
}

/// One cube, drawing only the faces it was told to.
fn cube(g: &mut Gl, x: (f32, f32), y: (f32, f32), z: (f32, f32), faces: u8, wire: bool) {
    let (x0, x1) = x;
    let (y0, y1) = y;
    let (z0, z1) = z;
    // Face, its outward normal, and its four corners in the winding upstream
    // gives them, which is what makes the outside the front.
    let sides: [(u8, [f32; 3], [[f32; 3]; 4]); 6] = [
        (
            X0,
            [-1.0, 0.0, 0.0],
            [[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]],
        ),
        (
            X1,
            [1.0, 0.0, 0.0],
            [[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]],
        ),
        (
            Y0,
            [0.0, -1.0, 0.0],
            [[x0, y0, z0], [x0, y0, z1], [x1, y0, z1], [x1, y0, z0]],
        ),
        (
            Y1,
            [0.0, 1.0, 0.0],
            [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        ),
        (
            Z0,
            [0.0, 0.0, -1.0],
            [[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]],
        ),
        (
            Z1,
            [0.0, 0.0, 1.0],
            [[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]],
        ),
    ];
    for (bit, normal, corners) in sides {
        if faces & bit == 0 {
            continue;
        }
        g.glx.begin(if wire {
            Shape::LineLoop
        } else {
            Shape::Polygon
        });
        g.glx.normal3f(normal[0], normal[1], normal[2]);
        for c in corners {
            g.glx.vertex3f(c[0], c[1], c[2]);
        }
        g.glx.end();
    }
}

/// Cut the cube into twenty-seven and keep the twenty that are not in the
/// middle of a face or of the whole, working out as it goes which faces of
/// each are still on the outside of something.
#[allow(clippy::too_many_arguments)]
fn recurse(
    g: &mut Gl,
    level: i32,
    x: (f32, f32),
    y: (f32, f32),
    z: (f32, f32),
    faces: u8,
    wire: bool,
    orig: bool,
    forig: u8,
) {
    if orig && wire {
        cube(g, x, y, z, faces & (X0 | X1 | Y0 | Y1), wire);
    }

    if level == 0 {
        if !wire {
            cube(g, x, y, z, faces, wire);
        }
        return;
    }

    let xi = (x.1 - x.0) / 3.0;
    let yi = (y.1 - y.0) / 3.0;
    let zi = (z.1 - z.0) / 3.0;

    for cx in 0..3 {
        for cy in 0..3 {
            for cz in 0..3 {
                let sub = (
                    (x.0 + cx as f32 * xi, x.0 + (cx + 1) as f32 * xi),
                    (y.0 + cy as f32 * yi, y.0 + (cy + 1) as f32 * yi),
                    (z.0 + cz as f32 * zi, z.0 + (cz + 1) as f32 * zi),
                );
                // The twenty that survive. Upstream writes this as three
                // pairwise tests; what they come to is that at most one of the
                // three coordinates may be the middle one, since two middles
                // is a face's hole and three is the hole through the centre.
                let middles = usize::from(cx == 1) + usize::from(cy == 1) + usize::from(cz == 1);
                let solid = middles <= 1;
                if solid {
                    let mut f = faces;
                    // A face against a neighbour is buried; a face that a hole
                    // has just opened up is exposed again.
                    if cx == 1 || (cx == 2 && (cy != 1 && cz != 1)) {
                        f &= !X0;
                    }
                    if cx == 1 || (cx == 0 && (cy != 1 && cz != 1)) {
                        f &= !X1;
                    }
                    if forig & X0 != 0 && cx == 2 && (cy == 1 || cz == 1) {
                        f |= X0;
                    }
                    if forig & X1 != 0 && cx == 0 && (cy == 1 || cz == 1) {
                        f |= X1;
                    }

                    if cy == 1 || (cy == 2 && (cx != 1 && cz != 1)) {
                        f &= !Y0;
                    }
                    if cy == 1 || (cy == 0 && (cx != 1 && cz != 1)) {
                        f &= !Y1;
                    }
                    if forig & Y0 != 0 && cy == 2 && (cx == 1 || cz == 1) {
                        f |= Y0;
                    }
                    if forig & Y1 != 0 && cy == 0 && (cx == 1 || cz == 1) {
                        f |= Y1;
                    }

                    if cz == 1 || (cz == 2 && (cx != 1 && cy != 1)) {
                        f &= !Z0;
                    }
                    if cz == 1 || (cz == 0 && (cx != 1 && cy != 1)) {
                        f &= !Z1;
                    }
                    if forig & Z0 != 0 && cz == 2 && (cx == 1 || cy == 1) {
                        f |= Z0;
                    }
                    if forig & Z1 != 0 && cz == 0 && (cx == 1 || cy == 1) {
                        f |= Z1;
                    }

                    recurse(g, level - 1, sub.0, sub.1, sub.2, f, wire, false, forig);
                } else if wire && (cx != 1 || cy != 1 || cz != 1) {
                    cube(g, sub.0, sub.1, sub.2, forig & (X0 | X1 | Y0 | Y1), wire);
                }
            }
        }
    }
}

impl Sponge {
    /// Rebuild the three lists at the depth we are now at. One list per axis,
    /// so each can be a different colour.
    fn build(&mut self, g: &mut Gl, level: i32) {
        let wire = self.wireframe;
        for (i, faces) in [X0 | X1, Y0 | Y1, Z0 | Z1].into_iter().enumerate() {
            g.glx.delete_lists(self.lists[i], 1);
            self.lists[i] = g.glx.gen_lists(1);
            g.glx.new_list(self.lists[i]);
            recurse(
                g,
                level,
                (-1.5, 1.5),
                (-1.5, 1.5),
                (-1.5, 1.5),
                faces,
                wire,
                true,
                faces,
            );
            g.glx.end_list();
        }
    }
}

impl Hack3d for Sponge {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if !self.wireframe {
            // Two lights, from opposite sides, so the far face of the sponge
            // does not go flat black as it turns.
            g.glx.light_position(0, -1.0, -1.0, 1.0, 0.1);
            g.glx.light_position(1, 1.0, -0.2, 0.2, 0.1);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_diffuse(1, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_enable(0, true);
            g.glx.light_enable(1, true);
            g.glx.lighting(true);
            g.glx.depth_test(true);
        }

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 15.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        for c in &mut self.ccolor {
            *c = (*c + 1) % self.colors.len().max(1);
        }

        self.draw_tick += 1;
        if self.draw_tick >= self.speed {
            self.draw_tick = 0;
            if self.current_depth >= self.max_depth {
                self.current_depth = -self.max_depth;
            }
            self.current_depth += 1;
            let level = self.current_depth.abs();
            self.build(g, level);
        }

        g.glx.scale(2.0, 2.0, 2.0);
        for i in 0..AXES {
            let c = &self.colors[self.ccolor[i].min(self.colors.len() - 1)];
            g.glx.material_ambient_diffuse([
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            ]);
            g.glx.call_list(self.lists[i]);
        }
        g.glx.pop_matrix();

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
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '=' => {
                    self.draw_tick = self.speed;
                    self.current_depth += if self.current_depth > 0 { 1 } else { -1 };
                    self.current_depth -= 1;
                    return true;
                }
                '-' | '_' => {
                    self.draw_tick = self.speed;
                    self.current_depth -= if self.current_depth > 0 { 1 } else { -1 };
                    self.current_depth -= 1;
                    return true;
                }
                _ => {}
            }
        }
        if screenhack_event_helper(event) {
            self.draw_tick = self.speed;
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let wander = g.res.bool("wander");
    let spin_speed = 1.0;
    let wander_speed = 0.03;
    let ncolors = 128;

    let mut st = Sponge {
        lists: [0; AXES],
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            1.0,
            if wander { wander_speed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        current_depth: 0,
        max_depth: g.res.int("maxDepth").clamp(1, 4),
        speed: g.res.int("speed").max(1),
        // Past the threshold, so the first frame builds rather than showing
        // nothing.
        draw_tick: 9_999_999,
        colors: make_smooth_colormap(ncolors),
        ccolor: [0, ncolors / 3, (ncolors / 3) * 2],
        wireframe: g.res.bool("wireframe"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        150",
    "*maxDepth:     3",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("maxDepth", "Max depth", 1.0, 4.0, 1.0, 0, "3"),
    Opt::slider("speed", "Duration", 2.0, 500.0, 1.0, 0, "150"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "menger",
    label: "Menger",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=qpnuNJH9cLw"),
        blurb: "The 3D Menger Sponge fractal, growing and shrinking.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
