//! Port of `hacks/glx/pulsar.c`.
//!
//! ```text
//! pulsar --- pulsar
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Copyright (C) 1999 David Konerding <dek@cgl.ucsf.edu>
//! ```
//!
//! Intersecting planes, with alpha blending, fog, textures, and mipmaps.
//!
//! Five squares sit on top of each other at the same place and tumble, each
//! about its own two axes at its own rate, so what you see is the moire of
//! translucent sheets cutting through one another. A square's four corners are
//! red, green, blue and white, and three of them are at four tenths alpha, so
//! the overlaps are where the colour is.
//!
//! The pulse is one line: the whole scene is scaled by the cosine and the sine
//! of a slowly advancing angle, ten times over. So it stretches out to ten
//! times its size along one axis while the other shrinks through zero, where
//! every square collapses to a line, and comes back through it inside out.
//!
//! The knobs are unusually raw for a saver of this age: lighting, fog, depth
//! buffering, blending and texturing are each exposed as a switch, because the
//! thing was written to show what those switches do.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, frand};

const CHECK_IMAGE_WIDTH: i32 = 64;
const CHECK_IMAGE_HEIGHT: i32 = 64;

/// One tumbling square: where it is, which way it is turned, and how fast it
/// turns. Upstream also carries a translation rate, which is never set to
/// anything, so the squares stay where they are put.
#[derive(Clone, Copy, Default)]
struct Quad {
    t: [f32; 3],
    r: [f32; 3],
    dr: [f32; 3],
}

struct Pulsar {
    quad_list: u32,
    quads: Vec<Quad>,
    scale: [f32; 3],
    frame: i32,

    do_light: bool,
    do_blend: bool,
    do_fog: bool,
    do_depth: bool,
    texture: Option<u32>,
}

/// `Generate_Image`: the built-in texture, a checkerboard of eight-pixel
/// squares. Upstream can load a file instead; there is no file here, and
/// `BUILTIN` is its default anyway.
fn generate_image() -> (i32, i32, Vec<u8>) {
    let (w, h) = (CHECK_IMAGE_WIDTH, CHECK_IMAGE_HEIGHT);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..w {
        for j in 0..h {
            let c = u8::from(((i & 0x8) == 0) ^ ((j & 0x8) == 0)) * 255;
            out.extend_from_slice(&[c, c, c, 255]);
        }
    }
    (w, h, out)
}

impl Hack3d for Pulsar {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        g.glx.depth_test(self.do_depth);
        g.glx.lighting(self.do_light);
        if self.do_light {
            g.glx.light_enable(0, true);
            // The corners are four different colours, so the colour has to come
            // from the vertex; a material would flatten the square to one.
            g.glx.color_material(true);
        }
        if let Some(t) = self.texture {
            g.glx.texturing(true);
            g.glx.bind_texture(t);
        }
        g.glx.blend(if self.do_blend {
            Blend::Alpha
        } else {
            Blend::Off
        });

        // Reset the projection every frame, as upstream does, which also puts
        // the modelview back to the identity before the scale below.
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.scale(self.scale[0], self.scale[1], self.scale[2]);

        for i in 0..self.quads.len() {
            let q = self.quads[i];
            g.glx.push_matrix();
            g.glx.translate(q.t[0], 0.0, 0.0);
            g.glx.translate(0.0, q.t[1], 0.0);
            g.glx.translate(0.0, 0.0, q.t[2]);
            g.glx.rotate(q.r[0], 1.0, 0.0, 0.0);
            g.glx.rotate(q.r[1], 0.0, 1.0, 0.0);
            g.glx.rotate(q.r[2], 0.0, 0.0, 1.0);
            g.glx.call_list(self.quad_list);
            g.glx.pop_matrix();

            for k in 0..3 {
                self.quads[i].r[k] += q.dr[k];
            }
        }

        // Read before it is updated, so the first frame is drawn unscaled and
        // the second is the widest and flattest it ever gets.
        self.scale = [
            (f32::from(self.frame as i16) / 360.0).cos() * 10.0,
            (f32::from(self.frame as i16) / 360.0).sin() * 10.0,
            1.0,
        ];
        self.frame += 1;

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        // No aspect correction: upstream's frustum is square whatever the
        // window is, so a wide window stretches the picture.
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let num_quads = g.res.int("quads").clamp(1, 500) as usize;
    let do_texture = g.res.bool("texture");
    // Antialiasing is a line-smoothing hint here, which WebGL has no equivalent
    // of, but it also switches blending on, and that part is real.
    let do_blend = g.res.bool("blend") || g.res.bool("antialias");

    let mut st = Pulsar {
        quad_list: 0,
        quads: vec![Quad::default(); num_quads],
        scale: [1.0; 3],
        frame: 0,
        do_light: g.res.bool("light"),
        do_blend,
        do_fog: g.res.bool("fog"),
        do_depth: g.res.bool("do_depth"),
        texture: None,
    };

    if do_texture {
        let (w, h, px) = generate_image();
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(w, h, px);
        g.glx.tex_clamp(false);
        // Mipmapping is what upstream's quality switch really turns on; with
        // none to build, the choice it leaves is between smoothing and not.
        g.glx.tex_nearest(!g.res.bool("texture_quality"));
        st.texture = Some(id);
    }

    if st.do_fog {
        g.glx.fog(Some(Fog::Linear {
            start: 50.0,
            end: 100.0,
            color: [0.1, 0.1, 0.1, 0.1],
        }));
    }

    // One square, four corners, four colours, and all but the last of them
    // translucent.
    st.quad_list = g.glx.gen_lists(1);
    g.glx.new_list(st.quad_list);
    g.glx.begin(Shape::Quads);
    for (c, uv, p) in [
        ([1.0, 0.0, 0.0, 0.4], [0.0, 0.0], [-1.0, -1.0]),
        ([0.0, 1.0, 0.0, 0.4], [0.0, 1.0], [-1.0, 1.0]),
        ([0.0, 0.0, 1.0, 0.4], [1.0, 1.0], [1.0, 1.0]),
        ([1.0, 1.0, 1.0, 1.0], [1.0, 0.0], [1.0, -1.0]),
    ] {
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.tex_coord2f(uv[0], uv[1]);
        g.glx.vertex3f(p[0], p[1], 0.0);
    }
    g.glx.end();
    g.glx.end_list();

    for q in &mut st.quads {
        q.t = [0.0, 0.0, -10.0];
        q.dr = [frand(5.0) as f32, frand(5.0) as f32, 0.0];
    }

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:           10000",
    "*showFPS:         False",
    "*quads:           5",
    "*light:           False",
    "*blend:           True",
    "*fog:             False",
    "*antialias:       False",
    "*texture:         False",
    "*texture_quality: False",
    "*mipmap:          False",
    "*do_depth:        False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("quads", "Quad count", 1.0, 50.0, 1.0, 0, "5"),
    Opt::boolean("blend", "Enable blending", "true"),
    Opt::boolean("light", "Enable lighting", "false"),
    Opt::boolean("fog", "Enable fog", "false"),
    Opt::boolean("texture", "Enable texturing", "false"),
    Opt::boolean("texture_quality", "Enable texture filtering", "false"),
    Opt::boolean("antialias", "Anti-alias lines", "false"),
    Opt::boolean("do_depth", "Enable depth buffer", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pulsar",
    label: "Pulsar",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Konerding",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=pR0lpvOAbUo"),
        blurb: "Intersecting planes, with alpha blending, fog, textures, \
                and mipmaps.",
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

    /// One square a quad, as many as the knob asks for, and each with its four
    /// coloured corners.
    #[test]
    fn one_square_per_quad() {
        for n in [1usize, 5, 20] {
            let r = run(&format!("quads={n}"), 3);
            let f = r.frame();
            assert_eq!(f.batches.len(), n);
            for b in &f.batches {
                assert_eq!(b.primitive, Primitive::Triangles);
                assert_eq!(b.count, 6, "a square is two triangles");
            }
            // Red, green, blue and white, three of them translucent.
            let colours: std::collections::BTreeSet<_> = f.vertices[..6]
                .iter()
                .map(|v| v.color.map(f32::to_bits))
                .collect();
            assert_eq!(colours.len(), 4);
            // Cutting the quad in two repeats the corners on the shared edge,
            // so six vertices carry the four colours; only the white one is
            // solid, and it is not on that edge.
            assert_eq!(
                f.vertices[..6].iter().filter(|v| v.color[3] == 1.0).count(),
                1,
                "exactly one corner is opaque"
            );
        }
    }

    /// The pulse: the scene is scaled by a cosine and a sine of the same angle,
    /// so it stretches one way as it flattens the other, and passes through
    /// nothing at all.
    #[test]
    fn the_pulse_sweeps_through_flat() {
        let mut r = start(StartArgs::new(640, 480, "quads=1", 20260811));
        let mut widths = Vec::new();
        let mut heights = Vec::new();
        for _ in 0..1200 {
            r.step();
            let m = r.frame().batches[0].modelview.0;
            widths.push(m[0].abs());
            heights.push(m[5].abs());
        }

        let big = |v: &Vec<f32>| v.iter().copied().fold(0.0f32, f32::max);
        let small = |v: &Vec<f32>| v.iter().copied().fold(f32::MAX, f32::min);
        assert!(big(&widths) > 9.0, "never stretched out");
        assert!(big(&heights) > 9.0);
        assert!(small(&widths) < 0.5, "never flattened");
        assert!(small(&heights) < 0.5);

        // And the two are a quarter turn apart: where one is widest the other
        // is flattest.
        let at_widest = widths
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert!(
            heights[at_widest] < 1.0,
            "both axes are wide at once, so it is not pulsing"
        );
    }

    /// Every square tumbles about two axes at its own rate, so they come apart
    /// even though they all start in the same place.
    #[test]
    fn the_squares_come_apart() {
        let r = run("quads=5", 60);
        let f = r.frame();
        let seen: std::collections::BTreeSet<_> = f
            .batches
            .iter()
            .map(|b| b.modelview.0.map(f32::to_bits))
            .collect();
        assert_eq!(seen.len(), 5, "the squares are still stacked");
    }

    /// The switches are what this saver is for, and each one has to reach the
    /// frame.
    #[test]
    fn each_switch_reaches_the_frame() {
        let plain = run("", 3);
        let b = &plain.frame().batches[0];
        assert_eq!(b.blend, Blend::Alpha, "blending is on by default");
        assert!(!b.depth_test && !b.lighting && b.texture.is_none() && b.fog.is_none());

        let all = run("light=true&fog=true&texture=true&do_depth=true", 3);
        let b = &all.frame().batches[0];
        assert!(b.depth_test, "the depth switch did nothing");
        assert!(
            b.lighting && b.color_material,
            "the light switch did nothing"
        );
        assert!(b.texture.is_some(), "the texture switch did nothing");
        assert!(
            matches!(b.fog, Some(Fog::Linear { start, end, .. }) if start == 50.0 && end == 100.0),
            "the fog switch did nothing"
        );

        let none = run("blend=false", 3);
        assert_eq!(none.frame().batches[0].blend, Blend::Off);
    }

    /// The built-in texture is a checkerboard of eight-pixel squares, black and
    /// white and nothing between.
    #[test]
    fn the_builtin_texture_is_a_checkerboard() {
        let r = run("texture=true", 2);
        let f = r.frame();
        let t = f.batches[0].texture.unwrap();
        let image = r.texture(t).unwrap();
        assert_eq!((image.width, image.height), (64, 64));
        assert!(image.data.iter().all(|v| *v == 0 || *v == 255));

        // Eight across, eight down, and the corner of each square differs from
        // its neighbour.
        let at = |x: usize, y: usize| image.data[(y * 64 + x) * 4];
        assert_ne!(at(0, 0), at(8, 0));
        assert_ne!(at(0, 0), at(0, 8));
        assert_eq!(at(0, 0), at(8, 8));
    }
}
