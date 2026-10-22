//! Port of `hacks/glx/sierpinski3d.c`.
//!
//! ```text
//! Sierpinski3D --- 3D sierpinski gasket
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! Revision History:
//! 1999: written by Tim Robinson <the_luggage@bigfoot.com>
//!       a 3-D representation of the Sierpinski gasket fractal.
//!
//! 10-Dec-99  jwz   rewrote to draw a set of tetrahedrons instead of a
//!                  random scattering of points.
//! ```
//!
//! A tetrahedron with its middle taken out, four times over, and again, for as
//! many levels as it is set to: the three-dimensional Sierpinski gasket. It
//! counts up to the deepest level, then counts back down, so the shape keeps
//! turning inside out.
//!
//! It is drawn as four separate display lists rather than one, and that is not
//! an optimisation: every face of a tetrahedron points one of four ways, and
//! each list holds the faces pointing one way, so each can be given a colour of
//! its own. Four colours, a quarter of the way apart in the same colormap, is
//! what makes the shape readable rather than a single-coloured tangle.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent,
    screenhack_event_helper,
};

/// The four faces, and so the four display lists.
const FACES: usize = 4;

struct Gasket {
    lists: [u32; FACES],
    rot: Rotator,
    trackball: Trackball,
    /// How deep the recursion goes now. Negative means counting back down,
    /// which is upstream's way of storing the direction with the value.
    current_depth: i32,
    max_depth: i32,
    speed: i32,
    tick: i32,
    colors: Vec<XColor>,
    ccolor: [usize; FACES],
    wireframe: bool,
}

/// The corners of the outer tetrahedron, and the way each of its faces points.
const VERTEX: [[f32; 3]; 4] = [
    [-1.0, -1.0, -1.0],
    [1.0, 1.0, -1.0],
    [1.0, -1.0, 1.0],
    [-1.0, 1.0, 1.0],
];
const NORMAL: [[f32; 3]; FACES] = [
    [1.0, -1.0, -1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
];

/// Which three corners make each face, in the winding upstream uses.
const FACE_CORNERS: [[usize; 3]; FACES] = [[0, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]];

/// Recurse: either draw this tetrahedron's face, or cut the tetrahedron into
/// the four half-sized ones at its corners and do those instead.
///
/// The middle is what is left out. A tetrahedron has six edge midpoints, and
/// the four corner tetrahedra between them are the gasket; the octahedron in
/// the centre is the hole.
fn four_tetras(g: &mut Gl, outer: &[[f32; 3]; 4], countdown: i32, which: usize, wire: bool) {
    if countdown <= 0 {
        let n = NORMAL[which];
        g.glx.normal3f(n[0], n[1], n[2]);
        g.glx.begin(if wire {
            Shape::LineLoop
        } else {
            Shape::Triangles
        });
        for i in FACE_CORNERS[which] {
            let p = outer[i];
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
        g.glx.end();
        return;
    }

    let mid = |a: usize, b: usize| {
        [
            (outer[a][0] + outer[b][0]) / 2.0,
            (outer[a][1] + outer[b][1]) / 2.0,
            (outer[a][2] + outer[b][2]) / 2.0,
        ]
    };
    let (m01, m02, m03) = (mid(0, 1), mid(0, 2), mid(0, 3));
    let (m12, m13, m23) = (mid(1, 2), mid(1, 3), mid(2, 3));
    let countdown = countdown - 1;

    for corner in [
        [outer[0], m01, m02, m03],
        [m01, outer[1], m12, m13],
        [m02, m12, outer[2], m23],
        [m03, m13, m23, outer[3]],
    ] {
        four_tetras(g, &corner, countdown, which, wire);
    }
}

impl Gasket {
    /// Recompile all four lists at the depth we are now at.
    fn compile(&mut self, g: &mut Gl) {
        let depth = self.current_depth.abs();
        for which in 0..FACES {
            g.glx.delete_lists(self.lists[which], 1);
            self.lists[which] = g.glx.gen_lists(1);
            g.glx.new_list(self.lists[which]);
            four_tetras(g, &VERTEX, depth, which, self.wireframe);
            g.glx.end_list();
        }
    }
}

impl Hack3d for Gasket {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if !self.wireframe {
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
            g.glx.light_position(0, -4.0, 3.0, 10.0, 1.0);
            g.glx.light_enable(0, true);
            for c in &mut self.ccolor {
                *c = (*c + 1) % self.colors.len().max(1);
            }
            g.glx.lighting(true);
        }
        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.push_matrix();
        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 10.0,
            (y as f32 - 0.5) * 10.0,
            (z as f32 - 0.5) * 20.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(4.0, 4.0, 4.0);

        for which in 0..FACES {
            let c = &self.colors[self.ccolor[which].min(self.colors.len() - 1)];
            g.glx.material_ambient_diffuse([
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            ]);
            g.glx.call_list(self.lists[which]);
        }
        g.glx.pop_matrix();

        self.tick += 1;
        if self.tick >= self.speed {
            self.tick = 0;
            if self.current_depth >= self.max_depth {
                self.current_depth = -self.max_depth;
            }
            self.current_depth += 1;
            self.compile(g);
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
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    /// The arrow keys and +/- step the depth by hand, and anything else pokes
    /// it into recompiling at the next level along.
    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '=' => {
                    self.tick = self.speed;
                    self.current_depth += if self.current_depth > 0 { 1 } else { -1 };
                    self.current_depth -= 1;
                    return true;
                }
                '-' | '_' => {
                    self.tick = self.speed;
                    self.current_depth -= if self.current_depth > 0 { 1 } else { -1 };
                    self.current_depth -= 1;
                    return true;
                }
                _ => {}
            }
        }
        if screenhack_event_helper(event) {
            self.tick = self.speed;
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

    let ncolors = 255;
    let colors = make_smooth_colormap(ncolors);
    let mut st = Gasket {
        lists: [0; FACES],
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            1.0,
            if wander { wander_speed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        /* start out at level 1, not 0 */
        current_depth: 1,
        max_depth: g.res.int("maxDepth").clamp(1, 6),
        speed: g.res.int("speed").max(1),
        // Upstream starts the counter past the threshold so the first frame
        // compiles rather than showing nothing.
        tick: 999_999,
        ccolor: [0, ncolors / 4, ncolors / 2, ncolors * 3 / 4],
        colors,
        wireframe: g.res.bool("wireframe"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        150",
    "*maxDepth:     5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("maxDepth", "Max depth", 1.0, 6.0, 1.0, 0, "5"),
    Opt::slider("speed", "Duration", 2.0, 500.0, 1.0, 0, "150"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sierpinski3d",
    label: "Sierpinski 3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and Tim Robinson",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=TGQRLAhDLv0"),
        blurb: "The 3D Sierpinski pyramid fractal, growing and shrinking.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
