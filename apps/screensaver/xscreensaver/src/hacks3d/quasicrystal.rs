//! Port of `hacks/glx/quasicrystal.c`.
//!
//! ```text
//! quasicrystal, Copyright (c) 2013 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Overlapping sine waves create interesting plane-tiling interference
//! patterns.  Created by jwz, Jul 2013.  Inspired by
//! http://mainisusuallyafunction.blogspot.com/2011/10/quasicrystals-as-sums-of-waves-in-plane.html
//! ```
//!
//! A quasicrystal is ordered but aperiodic, and this makes one the cheapest
//! way there is: seventeen copies of the same striped square, laid over each
//! other at evenly spread angles and added up. Where the stripes agree they
//! reinforce and where they disagree they cancel, and the pattern that falls
//! out never repeats but is not random either.
//!
//! Nothing is computed per pixel. The stripes are a single one-dimensional
//! texture of one period of a sine, and each plane is one textured quad whose
//! texture coordinates run hundreds of times across it, so the wave is drawn
//! by the sampler repeating. The planes are blended a seventeenth each, which
//! is the sum. Then two more full-screen quads finish it: one multiplies a
//! colour over the grey to tint it, and one clips the result to raise the
//! contrast.
//!
//! Two departures, both in the last pass. Upstream clips the contrast with a
//! bitwise logic op on the framebuffer, `GL_AND_REVERSE`, which WebGL has no
//! equivalent for at all. What it does arithmetically is invert what is there
//! and scale it, and that is expressible as a blend: source times one minus
//! destination. The result is very close and not bit for bit. And a
//! one-dimensional texture is stored here as a two-dimensional one a single
//! row high, since WebGL has no `GL_TEXTURE_1D`; sampling it repeats along the
//! row exactly as the 1D one did.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, XEvent, screenhack_event_helper,
};

/// One period of a sine, as a texture. Wide, because the stripes are stretched
/// hundreds of times across a plane and a coarse one would show its steps.
const TEX_WIDTH: i32 = 4096;

struct Plane {
    rot: Rotator,
    rot2: Rotator,
    texid: u32,
}

struct QuasiCrystal {
    button_down_p: bool,
    symmetric_p: bool,
    contrast: f32,
    count: usize,
    colors: Vec<XColor>,
    ccolor: usize,
    planes: Vec<Plane>,
    mousey: i32,
    wireframe: bool,
}

impl Hack3d for QuasiCrystal {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;
        g.glx.clear();

        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        g.glx.texturing(!wire);
        g.glx.blend(Blend::Alpha);

        g.glx.push_matrix();
        g.glx.translate(0.5, 0.5, 0.0);
        g.glx.scale(3.0, 3.0, 3.0);
        if wire {
            g.glx.scale(0.2, 0.2, 0.2);
        }

        let mut r = 0.0;
        let mut ps = 0.0;
        let down = self.button_down_p;
        let tau = std::f64::consts::PI * 2.0;
        for i in 0..self.count {
            let scale = if wire {
                10.0
            } else {
                700.0 / self.count as f64
            };

            g.glx.push_matrix();

            let (x, y, _) = self.planes[i].rot.position(!down);
            g.glx.translate(
                ((x - 0.5) * 0.3333) as f32,
                ((y - 0.5) * 0.3333) as f32,
                0.0,
            );

            /* With -symmetry, keep the planes' scales in sync.
            Otherwise, they scale independently. */
            let pscale = if self.symmetric_p && i > 0 {
                ps
            } else {
                let (_, _, z) = self.planes[i].rot2.position(!down);
                ps = 1.0 + (4.0 * z);
                ps
            };
            let scale = scale * pscale;

            /* With -symmetry, evenly distribute the planes' rotation.
            Otherwise, they rotate independently. */
            let z = if self.symmetric_p && i > 0 {
                r + (i as f64 * tau / self.count as f64)
            } else {
                let (_, _, z) = self.planes[i].rot.rotation(!down);
                r = z;
                z
            };

            g.glx.rotate((z * 360.0) as f32, 0.0, 0.0, 1.0);
            g.glx.translate(-0.5, -0.5, 0.0);

            let alpha = if wire { 0.5 } else { 1.0 / self.count as f32 };
            g.glx.color4f(1.0, 1.0, 1.0, alpha);

            if !wire {
                g.glx.bind_texture(self.planes[i].texid);
            }

            let s = (scale / 2.0) as f32;
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            g.glx.normal3f(0.0, 0.0, 1.0);
            for (u, v, px, py) in [
                (-s, s, 0.0, 1.0),
                (s, s, 1.0, 1.0),
                (s, -s, 1.0, 0.0),
                (-s, -s, 0.0, 0.0),
            ] {
                g.glx.tex_coord2f(u, v);
                g.glx.vertex3f(px, py, 0.0);
            }
            g.glx.end();

            if wire {
                // Without a texture there are no stripes, so draw them.
                g.glx.texturing(false);
                g.glx.color4f(1.0, 1.0, 1.0, 1.0 / self.count as f32);
                let mut j = 0.0;
                while j < 1.0 {
                    g.glx.begin(Shape::Lines);
                    g.glx.vertex3f(j, 0.0, 0.0);
                    g.glx.vertex3f(j, 1.0, 0.0);
                    g.glx.end();
                    j += (1.0 / scale) as f32;
                }
            }

            g.glx.pop_matrix();
        }

        /* Colorize the grayscale image. */
        {
            let c = &self.colors[self.ccolor.min(self.colors.len() - 1)];
            // Brighten the colors.
            let c = [
                0.6666 + f32::from(c.red) / 65536.0 / 3.0,
                0.6666 + f32::from(c.green) / 65536.0 / 3.0,
                0.6666 + f32::from(c.blue) / 65536.0 / 3.0,
                1.0,
            ];
            g.glx.blend(Blend::DstColorSrcColor);
            g.glx.texturing(false);
            g.glx.color4f(c[0], c[1], c[2], c[3]);
            g.glx.translate(-0.5, -0.5, 0.0);
            full_quad(g);
        }

        /* Clip the colors to simulate contrast. */
        if self.contrast > 0.0 {
            /* If c > 0, map 0 - 100 to 0.5 - 1.0, and use (s & ~d) */
            let c = 1.0 - (self.contrast / 2.0 / 100.0);
            g.glx.texturing(false);
            g.glx.blend(Blend::InverseDst);
            g.glx.color4f(c, c, c, 1.0);
            full_quad(g);
        }

        /* Rotate colors. */
        self.ccolor += 1;
        if self.ccolor >= self.colors.len() {
            self.ccolor = 0;
        }

        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width.max(1) as f32;

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.ortho(0.0, 1.0, 1.0, 0.0, -1.0, 1.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.translate(0.5, 0.5, 0.0);
        g.glx.scale(h, 1.0, 1.0);
        if width > height {
            g.glx.scale(1.0 / h, 1.0 / h, 1.0);
        }
        g.glx.translate(-0.5, -0.5, 0.0);
        g.glx.clear();
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        match *event {
            XEvent::ButtonPress { y, button: 1, .. } => {
                self.button_down_p = true;
                self.mousey = y;
                true
            }
            XEvent::ButtonRelease { button: 1, .. } => {
                self.button_down_p = false;
                true
            }
            // Wheel up or right raises the contrast, down or left lowers it.
            XEvent::ButtonPress { button: 4..=7, .. } => {
                let up = matches!(event, XEvent::ButtonPress { button: 4 | 7, .. });
                if up && self.contrast <= 0.0 {
                    return false;
                }
                if !up && self.contrast >= 100.0 {
                    return false;
                }
                self.contrast += if up { -1.0 } else { 1.0 };
                true
            }
            XEvent::MotionNotify { y, .. } if self.button_down_p => {
                /* Dragging up and down tweaks contrast */
                let dy = y - self.mousey;
                self.contrast = (self.contrast + dy as f32 / 40.0).clamp(0.0, 100.0);
                self.mousey = y;
                true
            }
            XEvent::KeyPress { key } => match key {
                '<' | ',' | '-' | '_' if self.contrast > 0.0 => {
                    self.contrast -= 1.0;
                    true
                }
                '>' | '.' | '=' | '+' if self.contrast < 100.0 => {
                    self.contrast += 1.0;
                    true
                }
                _ => screenhack_event_helper(event),
            },
            _ => screenhack_event_helper(event),
        }
    }
}

/// The screen, as one quad. The last two passes each cover it once.
fn full_quad(g: &mut Gl) {
    g.glx.begin(Shape::Quads);
    for (x, y) in [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)] {
        g.glx.vertex3f(x, y, 0.0);
    }
    g.glx.end();
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let count = g.res.int("count").clamp(1, 37) as usize;

    // One period of a sine, in grey. Every plane gets its own name for it, as
    // upstream does, though they all hold the same bytes.
    let tex_data: Vec<u8> = (0..TEX_WIDTH)
        .flat_map(|i| {
            let y = (255.0
                * (1.0 + (f64::from(i) * std::f64::consts::PI * 2.0 / f64::from(TEX_WIDTH)).sin())
                / 2.0) as u8;
            [y, y, y, 255]
        })
        .collect();

    let contrast = g.res.float("contrast") as f32;
    let contrast = if (0.0..=100.0).contains(&contrast) {
        contrast
    } else {
        0.0
    };

    let spinp = g.res.bool("spin");
    let wanderp = g.res.bool("wander");
    let spin_speed = 0.01;
    let wander_speed = 0.0001;
    let spin_accel = 10.0;
    let scale_speed = 0.005;

    let mut st = QuasiCrystal {
        button_down_p: false,
        symmetric_p: g.res.bool("symmetric"),
        contrast,
        count,
        // ncolors affects color-cycling speed
        colors: make_smooth_colormap(256),
        ccolor: 0,
        planes: Vec::with_capacity(count),
        mousey: 0,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    for _ in 0..count {
        let rot = Rotator::new(
            0.0,
            0.0,
            if spinp { spin_speed } else { 0.0 },
            spin_accel,
            if wanderp { wander_speed } else { 0.0 },
            true,
        );
        let rot2 = Rotator::new(0.0, 0.0, 0.0, 0.0, scale_speed, true);
        let texid = if wire {
            0
        } else {
            let id = g.glx.gen_texture();
            g.glx.bind_texture(id);
            g.glx.tex_image_2d(TEX_WIDTH, 1, tex_data.clone());
            id
        };
        st.planes.push(Plane { rot, rot2, texid });
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*spin:         True",
    "*wander:       True",
    "*symmetric:    True",
    "*count:        17",
    "*contrast:     30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*speed:        1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Density", 7.0, 37.0, 1.0, 0, "17").inverted(),
    Opt::slider("contrast", "Contrast", 0.0, 100.0, 1.0, 0, "30"),
    Opt::boolean("wander", "Displacement", "true"),
    Opt::boolean("spin", "Rotation", "true"),
    Opt::boolean("symmetric", "Symmetry", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "quasicrystal",
    label: "Quasi-Crystal",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=JsGf65d5TfM"),
        blurb: "Overlapping sine waves make an ordered but aperiodic tiling.",
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

    /// The stripes are a texture rather than geometry, so there has to be one,
    /// and it has to be one period of a sine.
    #[test]
    fn the_stripes_are_a_sine() {
        let mut r = start(StartArgs::new(640, 480, "count=7", 20260811));
        r.step();
        let f = r.frame();
        let id = f
            .batches
            .iter()
            .find_map(|b| b.texture)
            .expect("no texture was bound");
        let t = r.texture(id).expect("the texture has no image");
        assert_eq!(t.width, TEX_WIDTH);
        assert_eq!(t.height, 1);
        assert_eq!(t.data.len(), TEX_WIDTH as usize * 4);
        // Grey, opaque, and running from the middle up to white, back down
        // through black and home again.
        for px in t.data.chunks_exact(4) {
            assert_eq!(px[0], px[1]);
            assert_eq!(px[1], px[2]);
            assert_eq!(px[3], 255);
        }
        let at = |i: usize| t.data[i * 4];
        assert_eq!(at(0), 127);
        assert_eq!(at(TEX_WIDTH as usize / 4), 255);
        assert_eq!(at(TEX_WIDTH as usize * 3 / 4), 0);
    }

    /// One quad per plane, plus the two passes that tint and clip. Each plane
    /// carries its own texture and a seventeenth of the alpha, which is what
    /// adding them up means.
    #[test]
    fn every_plane_is_one_textured_quad() {
        let mut r = start(StartArgs::new(640, 480, "count=7", 20260811));
        r.step();
        let f = r.frame();
        let textured = f.batches.iter().filter(|b| b.texture.is_some()).count();
        assert_eq!(textured, 7, "one quad a plane");
        assert_eq!(f.batches.len(), 9, "the planes, the tint and the clip");
        // Two triangles a quad.
        assert!(f.batches.iter().all(|b| b.count == 6));
        for v in &f.vertices[..6] {
            assert!((v.color[3] - 1.0 / 7.0).abs() < 1e-6, "{:?}", v.color);
        }
    }

    /// The texture coordinates run far outside the unit square: that is how
    /// the one period of sine becomes hundreds of stripes.
    #[test]
    fn the_texture_repeats_across_a_plane() {
        let mut r = start(StartArgs::new(640, 480, "count=7", 20260811));
        r.step();
        let f = r.frame();
        let widest = f
            .vertices
            .iter()
            .map(|v| v.uv[0].abs())
            .fold(0.0f32, f32::max);
        assert!(widest > 20.0, "the stripes only repeat {widest} times");
    }

    /// The contrast knob decides whether the last pass happens at all.
    #[test]
    fn contrast_adds_a_pass() {
        let count = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame().batches.len()
        };
        assert_eq!(
            count("count=7&contrast=0") + 1,
            count("count=7&contrast=30")
        );
    }
}
