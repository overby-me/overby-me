//! Port of `hacks/glx/stairs.c`.
//!
//! ```text
//! stairs --- Infinite Stairs, an Escher-like scene.
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
//! Escher's infinite staircase.
//!
//! Sixteen wooden blocks in four flights round a square, each block a little
//! shorter than the one before it, so that going once round loses exactly the
//! height that the steps gained. From one particular angle the flights line up
//! and the staircase closes on itself; the camera is fixed at that angle, which
//! is the whole trick, and every so often the scene turns a full circle to show
//! that there is nothing there but sixteen ordinary boxes.
//!
//! A yellow ball hops round the loop for ever, sixteen positions and thirty-two
//! frames between each, arcing on a sine so it looks like it is jumping rather
//! than sliding. The one that crosses the seam where the staircase does not
//! really join is drawn slightly smaller, which hides the discrepancy.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

const SCALE_4_WINDOW: f32 = 0.3;

const MATERIAL_YELLOW: [f32; 4] = [0.7, 0.7, 0.0, 1.0];
const MATERIAL_WHITE: [f32; 4] = [0.7, 0.7, 0.7, 1.0];

/// Where the ball rests on each step of the loop.
const BALL_POSITIONS: [[f32; 3]; 16] = [
    [-3.0, 3.0, 1.0],
    [-3.0, 2.8, 2.0],
    [-3.0, 2.6, 3.0],
    [-2.0, 2.4, 3.0],
    [-1.0, 2.2, 3.0],
    [0.0, 2.0, 3.0],
    [1.0, 1.8, 3.0],
    [2.0, 1.6, 3.0],
    [2.0, 1.5, 2.0],
    [2.0, 1.4, 1.0],
    [2.0, 1.3, 0.0],
    [2.0, 1.2, -1.0],
    [2.0, 1.1, -2.0],
    [1.0, 0.9, -2.0],
    [0.0, 0.7, -2.0],
    [-1.0, 0.5, -2.0],
];

/// Frames the ball takes to get from one step to the next.
const SPHERE_TICKS: i32 = 32;

struct Stairs {
    trackball: Trackball,
    /// How far through a full turn the scene is, and which way it is turning.
    step: f32,
    rotating: i32,
    sphere_position: usize,
    sphere_tick: i32,
    texture: Option<u32>,
}

impl Stairs {
    /// One block: a box with the wood stretched over each of its six faces.
    fn draw_block(g: &mut Gl, width: f32, height: f32, thickness: f32) {
        let (w, h, t) = (width, height, thickness);
        g.glx.front_face_cw(false);
        g.glx.begin(Shape::Quads);
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

    /// The staircase: two blocks aside, then three flights of six, five and
    /// three, each block a tenth shorter than the last.
    fn draw_stairs_internal(g: &mut Gl) {
        g.glx.push_matrix();
        g.glx.push_matrix();
        g.glx.translate(-3.0, 0.1, 2.0);
        for x in 0..2 {
            Self::draw_block(g, 0.5, 2.7 + 0.1 * x as f32, 0.5);
            g.glx.translate(0.0, 0.1, -1.0);
        }
        g.glx.pop_matrix();
        g.glx.translate(-3.0, 0.0, 3.0);
        g.glx.push_matrix();

        for x in 0..6 {
            Self::draw_block(g, 0.5, 2.6 - 0.1 * x as f32, 0.5);
            g.glx.translate(1.0, -0.1, 0.0);
        }
        g.glx.translate(-1.0, -0.9, -1.0);
        for x in 0..5 {
            Self::draw_block(g, 0.5, 3.0 - 0.1 * x as f32, 0.5);
            g.glx.translate(0.0, 0.0, -1.0);
        }
        g.glx.translate(-1.0, -1.1, 1.0);
        for x in 0..3 {
            Self::draw_block(g, 0.5, 3.5 - 0.1 * x as f32, 0.5);
            g.glx.translate(-1.0, -0.1, 0.0);
        }
        g.glx.pop_matrix();
        g.glx.pop_matrix();
    }

    /// The ball, part way between one step and the next, arcing on a sine.
    fn draw_sphere(&self, g: &mut Gl) {
        let pos = self.sphere_position;
        let pos2 = (pos + 1) % BALL_POSITIONS.len();
        let (a, b) = (BALL_POSITIONS[pos], BALL_POSITIONS[pos2]);
        let frac = self.sphere_tick as f32 / SPHERE_TICKS as f32;

        let x = a[0] + (b[0] - a[0]) * frac;
        let y = a[1] + (b[1] - a[1]) * frac + 2.0 * (std::f32::consts::PI * frac).sin();
        let z = a[2] + (b[2] - a[2]) * frac;

        g.glx.push_matrix();
        g.glx.translate(x, y + 0.5, z);
        g.glx.scale(0.5, 0.5, 0.5);

        // A little smaller across the gap, which obscures the distance the
        // staircase does not really close.
        if pos == BALL_POSITIONS.len() - 1 {
            g.glx.scale(0.95, 0.95, 0.95);
        }

        g.glx.material_ambient_diffuse(MATERIAL_YELLOW);
        g.glx.texturing(false);
        g.glx.front_face_cw(false);
        unit_sphere(&mut g.glx, 32, 32, false);
        if self.texture.is_some() {
            g.glx.texturing(true);
        }
        g.glx.pop_matrix();
    }
}

impl Hack3d for Stairs {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if let Some(t) = self.texture {
            g.glx.texturing(true);
            g.glx.bind_texture(t);
        }
        g.glx.material_ambient_diffuse(MATERIAL_WHITE);

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

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        // The one angle from which the flights line up.
        g.glx.translate(0.0, 0.5, 0.0);
        g.glx.rotate(44.5, 1.0, 0.0, 0.0);
        g.glx.rotate(50.0, 0.0, 1.0, 0.0);

        // Every so often, turn the whole thing round once to show that it is
        // only sixteen boxes.
        if self.rotating == 0 && random().is_multiple_of(500) {
            self.rotating = if random() & 1 == 1 { 1 } else { -1 };
        }
        if self.rotating != 0 {
            g.glx
                .rotate(self.rotating as f32 * self.step, 0.0, 1.0, 0.0);
            if self.step >= 360.0 {
                self.rotating = 0;
                self.step = 0.0;
            }
            if !self.trackball.button_down() {
                self.step += 2.0;
            }
        }

        Self::draw_stairs_internal(g);
        self.draw_sphere(g);

        if !self.trackball.button_down() {
            self.sphere_tick += 1;
            if self.sphere_tick >= SPHERE_TICKS {
                self.sphere_tick = 0;
                self.sphere_position = (self.sphere_position + 1) % BALL_POSITIONS.len();
            }
        }

        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 5.0, 15.0);
        g.glx.matrix_mode_modelview();

        let n = if width >= 1024 {
            3.0
        } else if width >= 512 {
            2.0
        } else {
            1.0
        };
        g.glx.line_width(n);
        g.glx.point_size(n);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.trackball = Trackball::new();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut st = Stairs {
        trackball: Trackball::new(),
        step: 0.0,
        rotating: 0,
        sphere_position: (random() as usize) % BALL_POSITIONS.len(),
        sphere_tick: 0,
        texture: None,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

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
    g.glx.depth_test(true);
    g.glx.cull_face(true);
    g.glx.front_face_cw(false);

    if let Some((tw, th, px)) = crate::runtime::png::decode_rgba(crate::images::WOOD) {
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(tw, th, px);
        // Repeat, and no smoothing: the grain is the few big blocks of colour
        // it is.
        g.glx.tex_clamp(false);
        g.glx.tex_nearest(true);
        st.texture = Some(id);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     20000",
    "*showFPS:   False",
    "*wireframe: False",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "stairs",
    label: "Stairs",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Marcelo Vianna",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=Y1ceRT30qr0"),
        blurb: "Escher's infinite staircase.",
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

    fn run(frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// Sixteen blocks of six faces each, plus the ball.
    #[test]
    fn sixteen_blocks_and_a_ball() {
        let r = run(2);
        let f = r.frame();

        // A block is one run of six quads; the ball is a triangle strip.
        let blocks = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles && b.count == 6 * 6)
            .count();
        assert_eq!(blocks, 16, "the staircase should be sixteen blocks");

        let balls = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::TriangleStrip)
            .count();
        assert_eq!(balls, 1, "one ball");
    }

    /// Each block is a tenth shorter than the one before it in its flight,
    /// which is what makes the loop lose exactly the height the steps gain.
    #[test]
    fn every_step_is_a_little_shorter() {
        let r = run(1);
        let f = r.frame();

        // A block's height is half its extent in y, in its own frame.
        let heights: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles && b.count == 36)
            .map(|b| {
                let vs = &f.vertices[b.first..b.first + b.count];
                let hi = vs.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
                let lo = vs.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
                (hi - lo) / 2.0
            })
            .collect();
        assert_eq!(heights.len(), 16);

        // The three flights, in the order they are drawn.
        let flights: [&[f32]; 4] = [
            &heights[0..2],
            &heights[2..8],
            &heights[8..13],
            &heights[13..16],
        ];
        for (n, flight) in flights.iter().enumerate() {
            for w in flight.windows(2) {
                let step = w[1] - w[0];
                assert!(
                    (step.abs() - 0.1).abs() < 1e-4,
                    "flight {n} steps by {step}, not a tenth"
                );
            }
        }
    }

    /// The ball hops: it arcs above the straight line between two steps, and
    /// comes back down onto the next one.
    #[test]
    fn the_ball_hops_from_step_to_step() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut heights = Vec::new();
        // One whole hop, and a little of the next.
        for _ in 0..SPHERE_TICKS + 4 {
            r.step();
            let f = r.frame();
            let ball = f
                .batches
                .iter()
                .find(|b| b.primitive == Primitive::TriangleStrip)
                .expect("no ball");
            heights.push(ball.modelview.0[13]);
        }

        // It leaves the ground, reaches a top, and comes back.
        let lo = heights.iter().copied().fold(f32::MAX, f32::min);
        let hi = heights.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.3, "the ball barely moved: {lo} to {hi}");
        let top = heights
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert!(
            (4..SPHERE_TICKS as usize - 4).contains(&top),
            "the top of the arc is at frame {top}, not in the middle"
        );
    }

    /// The ball goes all the way round and comes back to where it started.
    #[test]
    fn the_ball_goes_round_for_ever() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let at = |r: &Runner3d| {
            let f = r.frame();
            let b = f
                .batches
                .iter()
                .find(|b| b.primitive == Primitive::TriangleStrip)
                .unwrap();
            [b.modelview.0[12], b.modelview.0[13], b.modelview.0[14]]
        };
        let first = at(&r);

        // Sixteen steps of thirty-two frames is one lap.
        for _ in 0..SPHERE_TICKS * BALL_POSITIONS.len() as i32 {
            r.step();
        }
        let after = at(&r);
        for k in 0..3 {
            assert!(
                (first[k] - after[k]).abs() < 1e-3,
                "a lap did not come back: {first:?} against {after:?}"
            );
        }
    }

    /// The blocks are wooden and the ball is not: the ball is drawn untextured
    /// so that it stays a flat yellow.
    #[test]
    fn the_blocks_are_wooden_and_the_ball_is_yellow() {
        let r = run(2);
        let f = r.frame();
        let wood = f.batches.iter().find(|b| b.count == 36).unwrap();
        assert!(wood.texture.is_some(), "the blocks should be wooden");
        assert_eq!(wood.material.ambient_diffuse, MATERIAL_WHITE);

        let ball = f
            .batches
            .iter()
            .find(|b| b.primitive == Primitive::TriangleStrip)
            .unwrap();
        assert!(ball.texture.is_none(), "the ball should not be wooden");
        assert_eq!(ball.material.ambient_diffuse, MATERIAL_YELLOW);
    }
}
