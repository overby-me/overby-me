//! Port of `hacks/glx/sballs.c`.
//!
//! ```text
//! sballs --- balls spinning like crazy in GL
//!
//! Copyright (c) E. Lassauge, 2001.
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
//! The original code for this mode was written by
//! Mustata Bogdan (LoneRunner) <lonerunner@planetquake.com>
//! and can be found at http://www.cfxweb.net/lonerunner/
//!
//! Eric Lassauge  (November-07-2000) <lassauge@users.sourceforge.net>
//! ```
//!
//! A ball at every vertex of a polyhedron, all of it spinning. The eight
//! polyhedra are the same table `ico` uses, so the balls sit at the corners of
//! a tetrahedron, cube, octahedron, dodecahedron, icosahedron, plane, pyramid
//! or stellated octahedron, and the radius of the balls is chosen per shape so
//! that they nearly touch.
//!
//! Both textures are drawn with nearest-neighbour filtering, on purpose: the
//! face is a small image and upstream would rather it looked pixellated than
//! smeared.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Shape, TexEnv};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

const MAX_BALLS: usize = 20;

/// One polyhedron: the balls go at its vertices.
struct Polyinfo {
    radius: f32,
    verts: &'static [[f32; 3]],
}

const T1: f32 = 1.0;
const T15: f32 = 1.5;
const T11: f32 = 1.1;
const T09: f32 = 0.9;

const POLYGONS: &[Polyinfo] = &[
    // tetrahedron
    Polyinfo {
        radius: 0.8,
        verts: &[[T1, T1, T1], [T1, -T1, -T1], [-T1, T1, -T1], [-T1, -T1, T1]],
    },
    // hexahedron
    Polyinfo {
        radius: 0.6,
        verts: &[
            [T1, T1, T1],
            [T1, T1, -T1],
            [T1, -T1, -T1],
            [T1, -T1, T1],
            [-T1, T1, T1],
            [-T1, T1, -T1],
            [-T1, -T1, -T1],
            [-T1, -T1, T1],
        ],
    },
    // octahedron
    Polyinfo {
        radius: 0.6,
        verts: &[
            [T15, 0.0, 0.0],
            [-T15, 0.0, 0.0],
            [0.0, T15, 0.0],
            [0.0, -T15, 0.0],
            [0.0, 0.0, T15],
            [0.0, 0.0, -T15],
        ],
    },
    // dodecahedron
    Polyinfo {
        radius: 0.35,
        verts: &[
            [0.0, 0.5, 1.0],
            [0.0, -0.5, 1.0],
            [0.0, -0.5, -1.0],
            [0.0, 0.5, -1.0],
            [1.0, 0.0, 0.5],
            [-1.0, 0.0, 0.5],
            [-1.0, 0.0, -0.5],
            [1.0, 0.0, -0.5],
            [0.5, 1.0, 0.0],
            [-0.5, 1.0, 0.0],
            [-0.5, -1.0, 0.0],
            [0.5, -1.0, 0.0],
            [0.75, 0.75, 0.75],
            [-0.75, 0.75, 0.75],
            [-0.75, -0.75, 0.75],
            [0.75, -0.75, 0.75],
            [0.75, -0.75, -0.75],
            [0.75, 0.75, -0.75],
            [-0.75, 0.75, -0.75],
            [-0.75, -0.75, -0.75],
        ],
    },
    // icosahedron
    Polyinfo {
        radius: 0.4,
        verts: &[
            [0.0, 0.0, -0.951_056_5],
            [0.0, 0.850_650_8, -0.425_325_37],
            [0.809_017, 0.262_865_56, -0.425_325_37],
            [0.5, -0.688_190_95, -0.425_325_37],
            [-0.5, -0.688_190_95, -0.425_325_37],
            [-0.809_017, 0.262_865_56, -0.425_325_37],
            [0.5, 0.688_190_95, 0.425_325_37],
            [0.809_017, -0.262_865_56, 0.425_325_37],
            [0.0, -0.850_650_8, 0.425_325_37],
            [-0.809_017, -0.262_865_56, 0.425_325_37],
            [-0.5, 0.688_190_95, 0.425_325_37],
            [0.0, 0.0, 0.951_056_5],
        ],
    },
    // plane
    Polyinfo {
        radius: 0.7,
        verts: &[
            [T11, 0.0, 0.0],
            [-T11, 0.0, 0.0],
            [0.0, T11, 0.0],
            [0.0, -T11, 0.0],
        ],
    },
    // pyramid
    Polyinfo {
        radius: 0.5,
        verts: &[
            [T1, 0.0, 0.0],
            [-T1, 0.0, 0.0],
            [0.0, T1, 0.0],
            [0.0, -T1, 0.0],
            [0.0, 0.0, T1],
        ],
    },
    // star
    Polyinfo {
        radius: 0.7,
        verts: &[
            [T09, T09, T09],
            [T09, -T09, -T09],
            [-T09, T09, -T09],
            [-T09, -T09, T09],
            [-T09, -T09, -T09],
            [-T09, T09, T09],
            [T09, -T09, T09],
            [T09, T09, -T09],
        ],
    },
];

struct Sballs {
    trackball: Trackball,
    eye: [f32; 3],
    rotm: [f32; 3],
    speed: f32,
    object: usize,
    spheres: usize,
    radius: [f32; MAX_BALLS],
    back: u32,
    face: u32,
    width: i32,
    height: i32,
    texture: bool,
    wire: bool,
}

impl Sballs {
    /// `drawSphere`, which is upstream's own rather than the shared one
    /// because it wants texture coordinates.
    fn draw_sphere(&self, g: &mut Gl, n: usize) {
        let v = POLYGONS[self.object].verts[n];
        let (major, minor) = (15usize, 30usize);
        let radius = self.radius[n];
        let major_step = std::f32::consts::PI / major as f32;
        let minor_step = 2.0 * std::f32::consts::PI / minor as f32;

        g.glx.push_matrix();
        g.glx.translate(v[0], v[1], v[2]);
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);

        for i in 0..major {
            let a = i as f32 * major_step;
            let b = a + major_step;
            let (r0, r1) = (radius * a.sin(), radius * b.sin());
            let (z0, z1) = (radius * a.cos(), radius * b.cos());

            g.glx.begin(if self.wire {
                Shape::LineStrip
            } else {
                Shape::TriangleStrip
            });
            for j in 0..=minor {
                let c = j as f32 * minor_step;
                let (x, y) = (c.cos(), c.sin());

                g.glx
                    .normal3f((x * r0) / radius, (y * r0) / radius, z0 / radius);
                g.glx
                    .tex_coord2f(j as f32 / minor as f32, i as f32 / major as f32);
                g.glx.vertex3f(x * r0, y * r0, z0);

                g.glx
                    .normal3f((x * r1) / radius, (y * r1) / radius, z1 / radius);
                g.glx
                    .tex_coord2f(j as f32 / minor as f32, (i + 1) as f32 / major as f32);
                g.glx.vertex3f(x * r1, y * r1, z1);
            }
            g.glx.end();
        }
        g.glx.pop_matrix();
    }
}

/// Decode one of the bundled pictures into a texture, pixellated rather than
/// smoothed because that is what upstream asks for.
fn load_texture(g: &mut Gl, png: &[u8]) -> u32 {
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    match crate::runtime::png::decode_rgba(png) {
        Some((w, h, px)) => g.glx.tex_image_2d(w, h, px),
        // A picture that will not decode is not worth stopping for: one
        // opaque white texel leaves whatever is under it alone.
        None => g.glx.tex_image_2d(1, 1, vec![255, 255, 255, 255]),
    }
    g.glx.tex_nearest(true);
    id
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let texture = g.res.bool("texture") && !wire;

    // Upstream numbers the objects from one on the command line and takes
    // zero to mean any of them.
    let arg = g.res.int("object") - 1;
    let object = if arg < 0 || arg as usize >= POLYGONS.len() {
        random() as usize % POLYGONS.len()
    } else {
        arg as usize
    };

    let count = g.res.int("count") as usize;
    let verts = POLYGONS[object].verts.len();
    let spheres = if count < 1 || count > verts {
        verts
    } else {
        count
    };

    let (back, face) = if texture {
        (
            load_texture(g, crate::images::SBALL_BG),
            load_texture(g, crate::images::SBALL),
        )
    } else {
        (0, 0)
    };

    let mut this = Sballs {
        trackball: Trackball::new(),
        eye: [0.0, 0.0, 6.0],
        rotm: [0.0, 0.0, 0.0],
        speed: g.res.int("cycles") as f32,
        object,
        spheres,
        radius: [POLYGONS[object].radius; MAX_BALLS],
        back,
        face,
        width: 1,
        height: 1,
        texture,
        wire,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Sballs {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            self.height = (self.width as f32 * 0.75) as i32;
        }
        g.glx.viewport(
            (width - self.width) / 2,
            (height - self.height) / 2,
            self.width,
            self.height,
        );
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .perspective(55.0, self.width as f32 / self.height as f32, 1.0, 300.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.clear();
        g.glx.push_matrix();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        g.glx.light_enable(1, true);
        g.glx.light_ambient(1, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_diffuse(1, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(1, 0.0, 0.0, 4.0, 1.0);
        g.glx.color_material(true);

        // Move the eyes.
        g.glx.translate(-self.eye[0], -self.eye[1], -self.eye[2]);

        // The background, a single quad behind everything.
        if self.texture {
            g.glx.lighting(true);
            g.glx.texturing(true);
            g.glx.tex_env(TexEnv::Modulate);
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
            g.glx.bind_texture(self.back);
        } else {
            g.glx.lighting(false);
            g.glx.texturing(false);
            g.glx.color4f(0.0, 0.0, 0.0, 1.0);
        }
        g.glx.begin(Shape::QuadStrip);
        // Letterbox the background on a wide screen, fill a tall one.
        let (w, h) = if self.width > self.height {
            (8.0, 4.1)
        } else {
            (4.0, 5.2)
        };
        for (x, y, u, v) in [
            (w, h, 0.0, 0.0),
            (w, -h, 0.0, 1.0),
            (-w, h, 1.0, 0.0),
            (-w, -h, 1.0, 1.0),
        ] {
            g.glx.normal3f(0.0, 0.0, 1.0);
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, -4.0);
        }
        g.glx.end();

        g.glx.mult_matrix(self.trackball.matrix());

        g.glx.rotate(self.rotm[0], 1.0, 0.0, 0.0);
        g.glx.rotate(self.rotm[1], 0.0, 1.0, 0.0);
        g.glx.rotate(self.rotm[2], 0.0, 0.0, 1.0);
        if !self.trackball.button_down() {
            self.rotm[0] += self.speed;
            self.rotm[1] -= self.speed;
        }

        if self.texture {
            g.glx.bind_texture(self.face);
        } else {
            g.glx.lighting(true);
        }
        for n in 0..self.spheres {
            self.draw_sphere(g, n);
        }

        g.glx.texturing(false);
        g.glx.depth_test(false);
        g.glx.lighting(false);
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*count:     0",
    "*cycles:    4",
    "*showFPS:   False",
    "*wireframe: False",
    "*texture:   True",
    "*object:    0",
];

const OBJECTS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Random object",
    },
    crate::runtime::opts::SelectItem {
        value: "1",
        label: "Tetrahedron",
    },
    crate::runtime::opts::SelectItem {
        value: "2",
        label: "Hexahedron",
    },
    crate::runtime::opts::SelectItem {
        value: "3",
        label: "Octahedron",
    },
    crate::runtime::opts::SelectItem {
        value: "4",
        label: "Dodecahedron",
    },
    crate::runtime::opts::SelectItem {
        value: "5",
        label: "Icosahedron",
    },
    crate::runtime::opts::SelectItem {
        value: "6",
        label: "Plane",
    },
    crate::runtime::opts::SelectItem {
        value: "7",
        label: "Pyramid",
    },
    crate::runtime::opts::SelectItem {
        value: "8",
        label: "Star",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::select("object", "Object", OBJECTS, "0"),
    Opt::slider("cycles", "Speed", 0.0, 20.0, 1.0, 0, "4"),
    Opt::slider("count", "Balls", 0.0, 20.0, 1.0, 0, "0"),
    Opt::boolean("texture", "Textured", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sballs",
    label: "Sballs",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Eric Lassauge",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=pcfqdvvPG8k"),
        blurb: "Balls spinning like crazy, one at every vertex of one of \
                eight polyhedra.",
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

    /// Every ball sits at a vertex of the chosen polyhedron, and the balls are
    /// sized so that they nearly touch without overlapping.
    #[test]
    fn the_balls_are_the_corners_of_the_solid() {
        for (n, p) in POLYGONS.iter().enumerate() {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("object={}&texture=false", n + 1),
                20260811,
            ));
            r.step();
            let f = r.frame();
            // One sphere is fifteen strips, so the batch count says how many
            // balls were drawn.
            let strips = f
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
                .count();
            assert_eq!(strips, 15 * p.verts.len(), "object {n} drew {strips}");

            // No two vertices are closer together than a ball is wide.
            for (i, a) in p.verts.iter().enumerate() {
                for b in p.verts.iter().skip(i + 1) {
                    let d: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
                    assert!(
                        d > p.radius,
                        "object {n}: two balls of {} overlap at {d} apart",
                        p.radius
                    );
                }
            }
        }
    }

    /// The count knob takes fewer balls than the solid has corners, but never
    /// more, and zero means all of them.
    #[test]
    fn the_count_never_asks_for_more_corners_than_there_are() {
        let balls = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
                .count()
                / 15
        };
        // The tetrahedron has four corners.
        assert_eq!(balls("object=1&texture=false&count=0"), 4);
        assert_eq!(balls("object=1&texture=false&count=2"), 2);
        assert_eq!(balls("object=1&texture=false&count=99"), 4);
    }

    /// The background is one quad, and it is behind everything else.
    #[test]
    fn the_background_is_furthest_away() {
        let mut r = start(StartArgs::new(640, 480, "object=2&texture=false", 20260811));
        r.step();
        let f = r.frame();
        let quads: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .collect();
        assert_eq!(quads.len(), 1, "the background is one quad");
        for v in &f.vertices[quads[0].first..quads[0].first + quads[0].count] {
            assert_eq!(v.pos[2], -4.0, "the background moved off its plane");
        }
    }

    /// Both bundled pictures decode, since a saver whose whole look is two
    /// textures is not much use without them.
    #[test]
    fn the_textures_decode() {
        for (name, png) in [
            ("sball", crate::images::SBALL),
            ("sball-bg", crate::images::SBALL_BG),
        ] {
            let (w, h, px) = crate::runtime::png::decode_rgba(png)
                .unwrap_or_else(|| panic!("{name} did not decode"));
            assert!(w > 0 && h > 0, "{name} is empty");
            assert_eq!(px.len(), (w * h * 4) as usize, "{name} is the wrong size");
        }
    }

    #[test]
    fn it_spins() {
        let mut r = start(StartArgs::new(640, 480, "object=5&texture=false", 20260811));
        r.step();
        let first = r.frame().batches[1].modelview.0;
        for _ in 0..10 {
            r.step();
        }
        let later = r.frame().batches[1].modelview.0;
        assert!(
            first
                .iter()
                .zip(later.iter())
                .any(|(a, b)| (a - b).abs() > 0.01),
            "the balls are not turning"
        );
    }
}
