//! Port of `hacks/glx/cubicgrid.c`.
//!
//! ```text
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
//! Cubic Grid - a 3D lattice. The observer is located in the centre of
//! a spinning finite lattice. As it rotates, various view-throughs appear and
//! evolve. A simple idea with interesting results.
//!
//! Vasek Potocek (Dec-28-2007)
//! vasek.potocek@post.cz
//! ```
//!
//! Twenty-seven thousand points on a lattice, and you are inside it. The whole
//! effect is that a regular grid seen from within lines up into corridors and
//! planes as it turns, and then falls out of alignment again: nothing moves
//! relative to anything else, only the angle you see it from.
//!
//! The points are compiled into a display list once, because the lattice never
//! changes; every frame is the same list drawn under a different matrix. That
//! is what the display list in [`crate::runtime::gl`] is for.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent,
    random_below,
};

const ZPOS: f32 = -18.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Symmetry {
    Cubic,
    Hexagonal,
}

struct CubicGrid {
    ratio: f32,
    list: u32,
    rot: Rotator,
    trackball: Trackball,
    size: f32,
    ticks: i32,
}

impl CubicGrid {
    /// Build the lattice. One point per cell, coloured by where it is, which is
    /// what makes the corridors readable when they line up.
    fn build(&mut self, g: &mut Gl, symmetry: Symmetry, bigdots: bool) {
        let tf = self.ticks as f32;
        let sqrt_3 = 3.0f32.sqrt();
        let sqrt_6 = 6.0f32.sqrt();
        let mut ps = if bigdots { 2.5 } else { 1.0 };
        if g.width() > 2560 {
            ps *= 2.0; /* Retina displays */
        }
        g.glx.point_size(ps);

        self.list = g.glx.gen_lists(1);
        g.glx.new_list(self.list);
        let mono = g.mono_p;
        if mono {
            g.glx.color3f(1.0, 1.0, 1.0);
        }
        g.glx.begin(Shape::Points);
        for x in 0..self.ticks {
            for y in 0..self.ticks {
                for z in 0..self.ticks {
                    if !mono {
                        g.glx.color3f(x as f32 / tf, y as f32 / tf, z as f32 / tf);
                    }
                    if symmetry == Symmetry::Hexagonal {
                        let i = (2 * x + (y + z) % 2) as f32;
                        let j = sqrt_3 * (y as f32 + (1.0 / 3.0) * (z % 2) as f32);
                        let k = (2.0 / 3.0) * sqrt_6 * z as f32;
                        g.glx.vertex3f(i, j, k);
                    } else {
                        g.glx.vertex3f(x as f32, y as f32, z as f32);
                    }
                }
            }
        }
        g.glx.end();
        g.glx.end_list();
    }
}

impl Hack3d for CubicGrid {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.load_identity();

        g.glx.rotate(180.0, 1.0, 0.0, 0.0); /* Make trackball track the right way */
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);

        g.glx.translate(0.0, 0.0, ZPOS);

        let s = self.size / self.ticks as f32;
        g.glx.scale(s, s, s);

        // A window narrower than it is tall gets the lattice shrunk to fit,
        // rather than cropped.
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(-x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(-z as f32 * 360.0, 0.0, 0.0, 1.0);

        let t = -self.ticks as f32 / 2.0;
        g.glx.translate(t, t, t);
        g.glx.call_list(self.list);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height.max(1);
        let mut y = 0;
        self.ratio = width as f32 / height as f32;

        if width > height * 3 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
            self.ratio = width as f32 / height as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.ratio, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let symmetry = match g.res.string("symmetry") {
        "hexagonal" => Symmetry::Hexagonal,
        "cubic" => Symmetry::Cubic,
        // "random", and anything else, which upstream treats as an error and
        // exits on. There is nothing to exit to here.
        _ => {
            if random_below(2) == 0 {
                Symmetry::Cubic
            } else {
                Symmetry::Hexagonal
            }
        }
    };
    let speed = g.res.float("speed");
    let spin_speed = 0.045 * speed;
    let spin_accel = 0.005 * speed;

    let mut st = CubicGrid {
        ratio: 1.0,
        list: 0,
        rot: Rotator::new(spin_speed, spin_speed, spin_speed, spin_accel, 0.0, true),
        trackball: Trackball::new(),
        size: g.res.float("zoom") as f32,
        ticks: g.res.int("ticks").clamp(1, 60),
    };
    st.build(g, symmetry, g.res.bool("bigdots"));
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:         20000",
    "*showFPS:       False",
    "*wireframe:     False",
    "*suppressRotationAnimation: True",
    "*speed:         1.0",
    "*zoom:          20",
    "*ticks:         30",
    "*bigdots:       True",
    "*symmetry:      random",
];

const SYMMETRIES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random symmetry",
    },
    SelectItem {
        value: "cubic",
        label: "Cubic symmetry",
    },
    SelectItem {
        value: "hexagonal",
        label: "Hexagonal symmetry",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.2, 10.0, 0.1, 1, "1.0"),
    Opt::slider("zoom", "Dot spacing", 15.0, 100.0, 1.0, 0, "20"),
    Opt::boolean("bigdots", "Big dots", "true"),
    Opt::select("symmetry", "Symmetry", SYMMETRIES, "random"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cubicgrid",
    label: "Cubic Grid",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Vasek Potocek",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=nOTi7gy9l-I"),
        blurb: "A rotating lattice of colored points, seen from inside it.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
