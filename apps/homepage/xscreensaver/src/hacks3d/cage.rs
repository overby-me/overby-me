//! Port of `hacks/glx/cage.c`.
//!
//! ```text
//! cage --- the Impossible Cage, an Escher like scene.
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
//! Copyright (C) 1998 by Marcelo Fernandez Vianna.
//! ```
//!
//! Escher's "Impossible Cage".
//!
//! Twelve wooden planks in a cube, arranged so that at every corner one plank
//! passes both in front of and behind the one it meets. That cannot be built,
//! and the way it is drawn is the point: the depth buffer is switched off, so
//! the planks simply overwrite each other in the order they are drawn rather
//! than in the order they are in. One line of setup does the whole illusion.
//!
//! Turn the depth test back on and the cage falls apart into an ordinary,
//! possible, and much less interesting arrangement of sticks.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, random};

const PLANK_WIDTH: f32 = 3.0;
const PLANK_HEIGHT: f32 = 0.35;
const PLANK_THICKNESS: f32 = 0.15;

const SCALE_4_WINDOW: f32 = 0.3;

const MATERIAL_WHITE: [f32; 4] = [0.7, 0.7, 0.7, 1.0];

struct Cage {
    /// Drives every rotation, at three incommensurate rates, so the cage turns
    /// and rocks without repeating.
    step: f32,
    wireframe: bool,
    texture: Option<u32>,
}

impl Cage {
    /// One plank: a box six faces long, each face carrying the whole wood
    /// texture stretched across it.
    fn draw_woodplank(&self, g: &mut Gl) {
        let (w, h, t) = (PLANK_WIDTH, PLANK_HEIGHT, PLANK_THICKNESS);
        // Wireframe reuses the very same vertex list as `GL_LINES`, so the
        // four corners of a face come out as two unjoined edges. That is
        // upstream's and it is what its wireframe looks like.
        let shape = if self.wireframe {
            Shape::Lines
        } else {
            Shape::Quads
        };

        g.glx.begin(shape);
        for (n, corners) in [
            (
                [0.0, 0.0, 1.0],
                [[-w, -h, t], [w, -h, t], [w, h, t], [-w, h, t]],
            ),
            (
                [0.0, 0.0, -1.0],
                [[-w, h, -t], [w, h, -t], [w, -h, -t], [-w, -h, -t]],
            ),
            (
                [0.0, 1.0, 0.0],
                [[-w, h, t], [w, h, t], [w, h, -t], [-w, h, -t]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[-w, -h, -t], [w, -h, -t], [w, -h, t], [-w, -h, t]],
            ),
            (
                [1.0, 0.0, 0.0],
                [[w, -h, t], [w, -h, -t], [w, h, -t], [w, h, t]],
            ),
            (
                [-1.0, 0.0, 0.0],
                [[-w, h, t], [-w, h, -t], [-w, -h, -t], [-w, -h, t]],
            ),
        ] {
            g.glx.normal3f(n[0], n[1], n[2]);
            for (uv, p) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                .into_iter()
                .zip(corners)
            {
                g.glx.tex_coord2f(uv[0], uv[1]);
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
        }
        g.glx.end();
    }

    /// The twelve planks, in the order that makes the cage impossible. Each is
    /// turned onto one of the three axes and pushed out to an edge of the cube;
    /// which of a meeting pair looks nearer is decided by which is drawn last,
    /// and upstream's order is chosen so that no two corners agree.
    fn draw_impossiblecage(&self, g: &mut Gl) {
        let (w, h, t) = (PLANK_WIDTH, PLANK_HEIGHT, PLANK_THICKNESS);
        // (turn about y, turn about z, where to put it)
        for (ry, rz, at) in [
            (true, false, [0.0, h - w, -t - w]),
            (false, true, [0.0, h - w, w - t]),
            (true, false, [0.0, w - h, -t - w]),
            (false, false, [0.0, w - h, 3.0 * t - w]),
            (false, true, [0.0, w - h, w - t]),
            (false, false, [0.0, w - h, w - 3.0 * t]),
            (false, false, [0.0, h - w, 3.0 * t - w]),
            (false, true, [0.0, h - w, t - w]),
            (false, false, [0.0, h - w, w - 3.0 * t]),
            (true, false, [0.0, h - w, w + t]),
            (false, true, [0.0, w - h, t - w]),
            (true, false, [0.0, w - h, w + t]),
        ] {
            g.glx.push_matrix();
            if ry {
                g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            }
            if rz {
                g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            }
            g.glx.translate(at[0], at[1], at[2]);
            self.draw_woodplank(g);
            g.glx.pop_matrix();
        }
    }
}

impl Hack3d for Cage {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if let Some(t) = self.texture {
            g.glx.texturing(true);
            g.glx.bind_texture(t);
        }

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -10.0);
        g.glx.scale(
            SCALE_4_WINDOW * g.height() as f32 / g.width().max(1) as f32,
            SCALE_4_WINDOW,
            SCALE_4_WINDOW,
        );
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        g.glx.rotate(self.step * 100.0, 0.0, 0.0, 1.0);
        g.glx
            .rotate(25.0 + (self.step * 5.0).cos() * 6.0, 1.0, 0.0, 0.0);
        g.glx
            .rotate(204.5 - (self.step * 5.0).sin() * 8.0, 0.0, 1.0, 0.0);
        self.draw_impossiblecage(g);

        g.glx.pop_matrix();
        self.step += 0.025;

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 5.0, 15.0);
        g.glx.matrix_mode_modelview();

        let i = (width / 512 + 1) as f32;
        g.glx.line_width(i);
        g.glx.point_size(i);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mut st = Cage {
        step: (random() % 90) as f32,
        wireframe: wire,
        texture: None,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        g.glx.lighting(true);
        for (i, pos) in [[1.0, 1.0, 1.0, 0.0], [-1.0, -1.0, 1.0, 0.0]]
            .into_iter()
            .enumerate()
        {
            g.glx.light_enable(i, true);
            g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
            g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
        }
        g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
        g.glx.material_ambient_diffuse(MATERIAL_WHITE);
        g.glx.material_specular([0.7, 0.7, 0.7, 1.0]);
        g.glx.material_shininess(60.0);
        g.glx.front_face_cw(false);
        g.glx.cull_face(true);

        // This is the whole trick. With no depth test, a plank drawn later
        // covers one drawn earlier however far away it is, so the same two
        // planks can each pass in front of the other at opposite corners.
        g.glx.depth_test(false);

        if let Some((tw, th, px)) = crate::runtime::png::decode_rgba(crate::images::WOOD) {
            let id = g.glx.gen_texture();
            g.glx.bind_texture(id);
            g.glx.tex_image_2d(tw, th, px);
            // Repeat rather than clamp, and no smoothing: the grain is meant
            // to be the few big blocks of colour it is.
            g.glx.tex_clamp(false);
            g.glx.tex_nearest(true);
            st.texture = Some(id);
        }
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     25000",
    "*showFPS:   False",
    "*wireframe: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "25000").inverted(),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cage",
    label: "Cage",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Marcelo Vianna",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=BxGHUFvI2Zo"),
        blurb: "Escher's \"Impossible Cage\".",
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

    /// No depth test anywhere. This is what makes the cage impossible, so it is
    /// worth asserting rather than assuming.
    #[test]
    fn nothing_is_depth_tested() {
        let r = run("", 3);
        assert!(
            r.frame().batches.iter().all(|b| !b.depth_test),
            "the depth test is on, and the cage is merely possible"
        );
    }

    /// Twelve planks of six faces each, and the faces of one plank fold into a
    /// single batch since only the matrix differs between planks.
    #[test]
    fn twelve_planks_of_six_faces() {
        let r = run("", 3);
        let f = r.frame();
        assert_eq!(f.batches.len(), 12, "a plank apiece");
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == Primitive::Triangles && b.count == 6 * 6),
            "a plank is six quads"
        );
    }

    /// The planks sit on the edges of a cube: each one the same distance out
    /// from the middle, which is what a cube's twelve edge midpoints are.
    ///
    /// Measured on a square window, where the camera's scale is the same on
    /// every axis and so distances survive it; the wide-window case squashes x
    /// and would say nothing.
    #[test]
    fn the_planks_frame_a_cube() {
        let mut r = start(StartArgs::new(640, 640, "", 20260811));
        r.step();
        let f = r.frame();

        // Where each plank's box is centred, which is its own translation seen
        // through the camera.
        let middles: Vec<[f32; 3]> = f
            .batches
            .iter()
            .map(|b| b.modelview.transform([0.0, 0.0, 0.0]))
            .collect();
        assert_eq!(middles.len(), 12);

        let mut centre = [0.0f32; 3];
        for m in &middles {
            for k in 0..3 {
                centre[k] += m[k] / 12.0;
            }
        }
        let out: Vec<f32> = middles
            .iter()
            .map(|m| {
                (0..3)
                    .map(|k| (m[k] - centre[k]).powi(2))
                    .sum::<f32>()
                    .sqrt()
            })
            .collect();

        // Every plank is a plank-width less a height out along one axis, and
        // out along a second by a width shifted by one to three thicknesses:
        // that shift is the interleaving, and it is why they are not all at
        // exactly the same radius.
        let (w, h, t) = (PLANK_WIDTH, PLANK_HEIGHT, PLANK_THICKNESS);
        let near = ((w - h).powi(2) + (w - 3.0 * t).powi(2)).sqrt() * SCALE_4_WINDOW;
        let far = ((w - h).powi(2) + (w + t).powi(2)).sqrt() * SCALE_4_WINDOW;
        for (i, d) in out.iter().enumerate() {
            assert!(
                (near - 1e-4..=far + 1e-4).contains(d),
                "plank {i} is {d} out, not between {near} and {far}"
            );
        }
        // And both ends of that band are used, so the interleaving is there.
        assert!(out.iter().any(|d| (d - near).abs() < 1e-3));
        assert!(out.iter().any(|d| (d - far).abs() < 1e-3));
    }

    /// The wood is a real picture, repeated rather than clamped and left
    /// unsmoothed.
    #[test]
    fn the_planks_are_wooden() {
        let r = run("", 2);
        let f = r.frame();
        let t = f.batches[0].texture.expect("no texture bound");
        assert!(f.batches.iter().all(|b| b.texture == Some(t)));

        let image = r.texture(t).expect("the texture has no image");
        assert_eq!((image.width, image.height), (128, 32));
        assert!(!image.clamp, "the grain should repeat");
        assert!(image.nearest, "the grain should not be smoothed");

        // Wood: more red than blue, over the picture as a whole.
        let red: u64 = image.data.iter().step_by(4).map(|v| u64::from(*v)).sum();
        let blue: u64 = image
            .data
            .iter()
            .skip(2)
            .step_by(4)
            .map(|v| u64::from(*v))
            .sum();
        assert!(
            red > blue * 2,
            "this wood is not brown: {red} red, {blue} blue"
        );
    }

    /// It turns on three rates at once, so it never comes back to where it was.
    #[test]
    fn the_cage_rocks_as_it_turns() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..200 {
            r.step();
            seen.insert(r.frame().batches[0].mvp.0.map(f32::to_bits));
        }
        assert_eq!(seen.len(), 200, "the cage repeated itself");
    }
}
