//! Port of `hacks/glx/glblur.c`.
//!
//! ```text
//! glblur --- radial blur using GL textures
//! Copyright (c) 2002-2008 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! It does this by rendering the scene into a small texture, then repeatedly
//! rendering increasingly-enlarged and increasingly-transparent versions of
//! that texture onto the frame buffer.
//! ```
//!
//! Radial blur of a spinning cube and a set of spikes.
//!
//! The trick is upstream's own description: the scene is drawn once into a
//! corner of the screen a hundred and twenty-eight pixels square, copied into a
//! texture, and the screen is then wiped. Fifteen copies of that texture are
//! laid over the whole window, each a little larger than the last and each a
//! little more transparent, adding as they go. What comes out is the object
//! smeared outwards from the middle.
//!
//! Most of the time the object itself is never drawn at all: one frame in ten
//! shows the cube and one in twenty the spikes, so what is usually on screen is
//! only the glow. Holding the pointer down brings both back, which is how you
//! see what is making the shape.
//!
//! Upstream copies the small render as `GL_LUMINANCE`, so its blur is grey on
//! a desktop; OpenGL ES cannot copy to that format and its mobile build gets
//! colour instead. This does what the mobile build does.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

/// How big the scene is rendered before it is smeared.
const TEX_SIZE: i32 = 128;

struct GlBlur {
    rot: Rotator,
    trackball: Trackball,

    texture: u32,
    colors: [Vec<XColor>; 4],
    ccolor: usize,
    /// One frame in ten shows the cube and one in twenty the spikes; the rest
    /// of the time there is only the blur.
    show_cube_p: bool,
    show_spikes_p: bool,
    blursize: i32,
    wireframe: bool,
}

impl GlBlur {
    /// The four faces of the cube that face east and west, then north and
    /// south, then up and down, each pair its own colour.
    fn draw_cube(&self, g: &mut Gl, colors: [[f32; 4]; 3]) {
        let shape = if self.wireframe {
            Shape::LineLoop
        } else {
            Shape::Quads
        };
        /// A face: which way it points, and its four corners with the
        /// texture coordinate each one carries.
        type Face = ([f32; 3], [([f32; 2], [f32; 3]); 4]);

        let faces: [Face; 6] = [
            // front and back
            (
                [0.0, 0.0, 1.0],
                [
                    ([1.0, 0.0], [0.5, -0.5, 0.5]),
                    ([0.0, 0.0], [0.5, 0.5, 0.5]),
                    ([0.0, 1.0], [-0.5, 0.5, 0.5]),
                    ([1.0, 1.0], [-0.5, -0.5, 0.5]),
                ],
            ),
            (
                [0.0, 0.0, -1.0],
                [
                    ([0.0, 0.0], [-0.5, -0.5, -0.5]),
                    ([0.0, 1.0], [-0.5, 0.5, -0.5]),
                    ([1.0, 1.0], [0.5, 0.5, -0.5]),
                    ([1.0, 0.0], [0.5, -0.5, -0.5]),
                ],
            ),
            // left and right
            (
                [-1.0, 0.0, 0.0],
                [
                    ([1.0, 1.0], [-0.5, 0.5, 0.5]),
                    ([1.0, 0.0], [-0.5, 0.5, -0.5]),
                    ([0.0, 0.0], [-0.5, -0.5, -0.5]),
                    ([0.0, 1.0], [-0.5, -0.5, 0.5]),
                ],
            ),
            (
                [1.0, 0.0, 0.0],
                [
                    ([1.0, 1.0], [0.5, -0.5, -0.5]),
                    ([1.0, 0.0], [0.5, 0.5, -0.5]),
                    ([0.0, 0.0], [0.5, 0.5, 0.5]),
                    ([0.0, 1.0], [0.5, -0.5, 0.5]),
                ],
            ),
            // top and bottom
            (
                [0.0, 1.0, 0.0],
                [
                    ([0.0, 0.0], [0.5, 0.5, 0.5]),
                    ([0.0, 1.0], [0.5, 0.5, -0.5]),
                    ([1.0, 1.0], [-0.5, 0.5, -0.5]),
                    ([1.0, 0.0], [-0.5, 0.5, 0.5]),
                ],
            ),
            (
                [0.0, -1.0, 0.0],
                [
                    ([1.0, 0.0], [-0.5, -0.5, 0.5]),
                    ([0.0, 0.0], [-0.5, -0.5, -0.5]),
                    ([0.0, 1.0], [0.5, -0.5, -0.5]),
                    ([1.0, 1.0], [0.5, -0.5, 0.5]),
                ],
            ),
        ];

        for (i, (n, corners)) in faces.into_iter().enumerate() {
            g.glx.material_ambient_diffuse(colors[i / 2]);
            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx.begin(shape);
            for (uv, p) in corners {
                g.glx.tex_coord2f(uv[0], uv[1]);
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
        }
    }

    /// Three spikes out through the faces and four out through the corners.
    fn draw_spikes(&self, g: &mut Gl, color: [f32; 4]) {
        let s = 10.0;
        g.glx.material_ambient_diffuse(color);

        g.glx.line_width(1.0);
        g.glx.begin(Shape::Lines);
        for v in [[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, s]] {
            g.glx.vertex3f(-v[0], -v[1], -v[2]);
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();

        g.glx.line_width(8.0);
        g.glx.begin(Shape::Lines);
        for v in [[s, s, s], [s, s, -s], [s, -s, s], [-s, s, s]] {
            g.glx.vertex3f(-v[0], -v[1], -v[2]);
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }

    /// The scene, rotated and carried into place. Drawn twice a frame: once
    /// small to be copied into the texture, and once at full size if it is
    /// being shown at all.
    #[allow(clippy::too_many_arguments)]
    fn draw_scene(
        &mut self,
        g: &mut Gl,
        colors: [[f32; 4]; 4],
        at: (f64, f64, f64),
        turn: (f64, f64, f64),
        cube: bool,
        spikes: bool,
        down: bool,
    ) {
        let (x, y, z) = at;
        let (rx, ry, rz) = turn;

        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.translate(
            ((x - 0.5) * 2.0) as f32,
            ((y - 0.5) * 2.0) as f32,
            ((z - 0.5) * 8.0) as f32,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate((rx * 360.0) as f32, 1.0, 0.0, 0.0);
        g.glx.rotate((ry * 360.0) as f32, 0.0, 1.0, 0.0);
        g.glx.rotate((rz * 360.0) as f32, 0.0, 0.0, 1.0);

        if cube || down {
            g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
            self.draw_cube(g, [colors[0], colors[1], colors[2]]);
        }
        if spikes || down {
            self.draw_spikes(g, colors[3]);
        }
        g.glx.pop_matrix();
    }

    /// The blur: the small render laid over the window fifteen times, each a
    /// little larger and a little fainter, adding as it goes.
    fn overlay_blur_texture(&self, g: &mut Gl, w: i32, h: i32) {
        let times = self.blursize.max(1);
        let inc = 0.02 * (25.0 / times as f32);
        let mut spost = 0.0f32; /* how far in from the texture's edge */
        let mut alpha = 0.2f32;
        let alpha_inc = alpha / times as f32;

        g.glx.texturing(true);
        g.glx.bind_texture(self.texture);
        g.glx.depth_test(false);
        g.glx.lighting(false);
        g.glx.blend(Blend::AlphaAdd);

        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.ortho(0.0, w as f32, h as f32, 0.0, -1.0, 1.0);
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();

        g.glx.begin(Shape::Quads);
        for _ in 0..times {
            g.glx.color4f(1.0, 1.0, 1.0, alpha);
            // Reading further inside the texture each time makes the same
            // picture cover more of the screen, which is the enlargement.
            for (u, v, x, y) in [
                (spost, 1.0 - spost, 0.0, 0.0),
                (spost, spost, 0.0, h as f32),
                (1.0 - spost, spost, w as f32, h as f32),
                (1.0 - spost, 1.0 - spost, w as f32, 0.0),
            ] {
                g.glx.tex_coord2f(u, v);
                g.glx.vertex3f(x, y, 0.0);
            }
            spost += inc;
            alpha -= alpha_inc;
        }
        g.glx.end();

        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();
        g.glx.pop_matrix();
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
    }
}

impl Hack3d for GlBlur {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let down = self.trackball.button_down();

        // What is being shown changes rarely, so the blur has time to settle.
        if random().is_multiple_of(30) {
            self.show_cube_p = random().is_multiple_of(10);
            self.show_spikes_p = random().is_multiple_of(20);
        }

        // Four colours walking their own smooth maps, a step a frame.
        let mut colors = [[0.0f32; 4]; 4];
        for (i, c) in colors.iter_mut().enumerate() {
            let x = self.colors[i][self.ccolor];
            *c = [
                f32::from(x.red) / 65535.0,
                f32::from(x.green) / 65535.0,
                f32::from(x.blue) / 65535.0,
                1.0,
            ];
        }
        self.ccolor = (self.ccolor + 1) % self.colors[0].len();

        // Advanced once for the whole frame: the scene is drawn twice and the
        // two must agree.
        let at = self.rot.position(!down);
        let turn = self.rot.rotation(!down);

        let (w, h) = (g.width(), g.height());
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(!self.wireframe);

        // Draw the scene small, into the corner, and take a copy of it.
        g.glx.viewport(0, 0, TEX_SIZE, TEX_SIZE);
        self.draw_scene(g, colors, at, turn, true, true, true);
        g.glx.texturing(true);
        g.glx.bind_texture(self.texture);
        g.glx.copy_tex_sub_image_2d();
        g.glx.texturing(false);
        g.glx.viewport(0, 0, w, h);

        // Wipe it: what was in the corner was only scaffolding.
        g.glx.clear();

        if self.show_cube_p || self.show_spikes_p || down {
            self.draw_scene(
                g,
                colors,
                at,
                turn,
                self.show_cube_p,
                self.show_spikes_p,
                down,
            );
        }
        self.overlay_blur_texture(g, w, h);

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
            .look_at([0.0, 0.0, 8.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // The spin knob is a string of axis letters rather than a switch.
    let spin = g.res.get("spin").unwrap_or("XYZ").to_string();
    let spin_speed = 0.9;
    let wander_speed = 0.06;
    let axis = |c: char| {
        if spin.contains(c) || spin.contains(c.to_ascii_lowercase()) {
            spin_speed
        } else {
            0.0
        }
    };
    let wire = g.res.bool("wireframe");

    let mut st = GlBlur {
        rot: Rotator::new(
            axis('X'),
            axis('Y'),
            axis('Z'),
            1.0,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        trackball: Trackball::new(),
        texture: 0,
        colors: [
            make_smooth_colormap(128),
            make_smooth_colormap(128),
            make_smooth_colormap(128),
            make_smooth_colormap(128),
        ],
        ccolor: 0,
        show_cube_p: true,
        show_spikes_p: true,
        blursize: g.res.int("blursize").clamp(0, 200),
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    // The texture the scene is copied into: reserved at size, never uploaded.
    st.texture = g.glx.gen_texture();
    g.glx.bind_texture(st.texture);
    g.glx.tex_image_2d(TEX_SIZE, TEX_SIZE, Vec::new());
    g.glx.tex_clamp(true);
    g.glx.tex_nearest(false);

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 0.0, 5.0, 10.0, 1.0);
        g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
        g.glx.light_diffuse(0, [0.3, 0.3, 0.3, 1.0]);
        g.glx.light_specular(0, [0.8, 0.8, 0.8, 1.0]);
        g.glx.light_model_ambient([0.2, 0.2, 0.2, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.depth_test(true);
        g.glx.cull_face(true);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      10000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*spin:       XYZ",
    "*wander:     True",
    "*blursize:   15",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("blursize", "Blur smoothness", 0.0, 50.0, 1.0, 0, "15"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glblur",
    label: "GL Blur",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=wUWwQXRp8lE"),
        blurb: "Radial blur of a spinning cube and a set of spikes.",
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
    use crate::runtime::gl::Primitive;

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// The scene is drawn small into the corner, copied, and then wiped: the
    /// small render is scaffolding and must not survive into the picture.
    #[test]
    fn the_scene_is_rendered_small_then_wiped() {
        let r = run("", 3);
        let f = r.frame();

        // Something is drawn at the texture's size, before anything else.
        let small = f
            .batches
            .iter()
            .position(|b| b.viewport == [0, 0, TEX_SIZE, TEX_SIZE])
            .expect("nothing was drawn small");

        // Then it is copied.
        let copy = f
            .batches
            .iter()
            .position(|b| b.copy_to_texture.is_some())
            .expect("nothing was copied to the texture");
        assert!(copy > small, "the copy came before the small render");

        // And then the screen is wiped before anything else is drawn.
        let wipe = f
            .batches
            .iter()
            .position(|b| b.clear_color_first)
            .expect("the small render was never wiped");
        assert!(wipe > copy, "the wipe came before the copy");
        assert_eq!(
            f.batches[wipe].viewport,
            [0, 0, 640, 480],
            "the viewport should be back to the window by then"
        );
    }

    /// The blur is the texture laid over the window as many times as the knob
    /// says, each fainter and each reading further into the texture.
    #[test]
    fn the_blur_is_stacked_copies_of_the_texture() {
        for times in [1, 5, 15, 40] {
            let r = run(&format!("blursize={times}"), 3);
            let f = r.frame();
            let overlay: Vec<_> = f
                .batches
                .iter()
                .filter(|b| b.texture == Some(1) && !b.lighting && b.blend == Blend::AlphaAdd)
                .collect();
            assert_eq!(overlay.len(), 1, "the blur should be one run of quads");
            assert_eq!(
                overlay[0].count,
                times * 6,
                "{times} copies, two triangles each"
            );

            // Each is fainter than the last, and each reads further in.
            let vs = &f.vertices[overlay[0].first..overlay[0].first + overlay[0].count];
            let alphas: Vec<f32> = vs.iter().step_by(6).map(|v| v.color[3]).collect();
            for w in alphas.windows(2) {
                assert!(w[1] < w[0], "the copies do not fade: {alphas:?}");
            }
            let us: Vec<f32> = vs.iter().step_by(6).map(|v| v.uv[0]).collect();
            for w in us.windows(2) {
                assert!(w[1] > w[0], "the copies do not enlarge: {us:?}");
            }
        }
    }

    /// The texture is the size the scene is rendered at, and is reserved
    /// rather than uploaded: nothing ever puts an image in it but the copy.
    #[test]
    fn the_texture_is_reserved_not_uploaded() {
        let r = run("", 2);
        let t = r.texture(1).expect("no texture");
        assert_eq!((t.width, t.height), (TEX_SIZE, TEX_SIZE));
        assert!(t.data.is_empty(), "the texture should carry no image");
        assert!(!t.nearest, "the blur wants a smooth texture");
    }

    /// Most of the time the object itself is not drawn at all, and what is on
    /// screen is only its glow.
    #[test]
    fn the_object_is_usually_invisible() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut shown = 0;
        for _ in 0..300 {
            r.step();
            // Anything lit and drawn at the window's size is the object.
            if r.frame().batches.iter().any(|b| {
                b.lighting && b.viewport == [0, 0, 640, 480] && b.primitive != Primitive::Points
            }) {
                shown += 1;
            }
        }
        assert!(shown < 200, "the object was shown on {shown} frames of 300");
    }

    /// Holding the pointer down brings the object back, so you can see what is
    /// making the shape.
    #[test]
    fn holding_the_pointer_shows_the_object() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        r.event(XEvent::ButtonPress {
            x: 320,
            y: 240,
            button: 1,
        });
        r.step();
        let shown = r
            .frame()
            .batches
            .iter()
            .any(|b| b.lighting && b.viewport == [0, 0, 640, 480]);
        assert!(shown, "the object stayed hidden with the pointer down");
    }

    /// The colours walk their maps, so the object is never the same colour for
    /// long.
    #[test]
    fn the_colours_walk() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..40 {
            r.step();
            // The small render is always drawn, whatever is being shown.
            if let Some(b) = r
                .frame()
                .batches
                .iter()
                .find(|b| b.viewport == [0, 0, TEX_SIZE, TEX_SIZE] && b.lighting)
            {
                seen.insert(b.material.ambient_diffuse.map(f32::to_bits));
            }
        }
        assert!(seen.len() > 10, "only {} colours in forty", seen.len());
    }
}
