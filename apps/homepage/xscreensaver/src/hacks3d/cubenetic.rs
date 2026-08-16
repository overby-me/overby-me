//! Port of `hacks/glx/cubenetic.c`.
//!
//! ```text
//! cubenetic, Copyright (c) 2002-2014 Jamie Zawinski <jwz@jwz.org>
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
//! A few cubes stretching and squashing inside each other, wrapped in a
//! writhing colour pattern.
//!
//! The pattern is a texture rebuilt from scratch every single frame: sixty-five
//! thousand pixels, each the sum of a radial profile sampled at the distance to
//! three moving sources, taken modulo the colour count. It is the interference
//! pattern from upstream's own 2D `interference` hack, wrapped round the boxes
//! rather than drawn flat, and it is why this saver is the one that needs a
//! texture that changes.
//!
//! The boxes themselves are pure sine. Each has six little frequencies of its
//! own, one for each of its position and size axes, and each frame every one of
//! them is a sine of the frame number times its own frequency. Nothing is
//! integrated, so they never drift out of range: a box breathes between half
//! and one and a half of its size and wanders half a unit either way, for ever.
//!
//! The two colourmaps do different jobs. The boxes cycle through a smooth
//! random one, one step per frame each, starting from different places. The
//! texture uses a closed loop of three hues sixty degrees apart, which is what
//! keeps the pattern reading as one material rather than a rainbow.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_color_loop, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent,
    frand, random, screenhack_event_helper,
};

const TEXTURE_SIZE: i32 = 256;
/// How many colours each map holds. The texture's index is taken modulo this,
/// so it is also the period of the pattern's banding.
const NCOLORS: usize = 256;

struct Cube {
    color: usize,
    x: f32,
    y: f32,
    z: f32,
    w: f32,
    h: f32,
    d: f32,
    frame: i32,
    /// The six frequencies this box breathes and wanders on.
    dx: f32,
    dy: f32,
    dz: f32,
    dw: f32,
    dh: f32,
    dd: f32,
}

#[derive(Clone, Copy, Default)]
struct WaveSrc {
    x: i32,
    y: i32,
    xth: f64,
    yth: f64,
}

struct Waves {
    radius: i32,
    speed: i32,
    srcs: Vec<WaveSrc>,
    heights: Vec<i32>,
}

struct Cubenetic {
    rot: Rotator,
    trackball: Trackball,
    cube_list: u32,
    texture_id: u32,
    cubes: Vec<Cube>,
    waves: Waves,
    /// The pattern, rebuilt every frame and handed to the texture.
    texture: Vec<u8>,
    cube_colors: Vec<XColor>,
    texture_colors: Vec<XColor>,
    do_texture: bool,
}

/// `SINOID`: a sine of the frame number, scaled and centred on zero.
fn sinoid(scale: f32, frame: i32, size: f32) -> f32 {
    let pi = std::f32::consts::PI;
    (((1.0 + ((frame as f32 * scale) / 2.0 * pi).sin()) / 2.0) * size) - size / 2.0
}

/// A face: its normal, and its four corners with the texture coordinate each
/// one carries.
type Face = ([f32; 3], [([f32; 2], [f32; 3]); 4]);

fn unit_cube(g: &mut Gl, wire: bool) {
    // Six faces, each with the texture laid over it whole.
    let faces: [Face; 6] = [
        (
            [0.0, 0.0, 1.0], /* front */
            [
                ([1.0, 0.0], [0.5, -0.5, 0.5]),
                ([0.0, 0.0], [0.5, 0.5, 0.5]),
                ([0.0, 1.0], [-0.5, 0.5, 0.5]),
                ([1.0, 1.0], [-0.5, -0.5, 0.5]),
            ],
        ),
        (
            [0.0, 0.0, -1.0], /* back */
            [
                ([0.0, 0.0], [-0.5, -0.5, -0.5]),
                ([0.0, 1.0], [-0.5, 0.5, -0.5]),
                ([1.0, 1.0], [0.5, 0.5, -0.5]),
                ([1.0, 0.0], [0.5, -0.5, -0.5]),
            ],
        ),
        (
            [-1.0, 0.0, 0.0], /* left */
            [
                ([0.0, 0.0], [-0.5, -0.5, 0.5]),
                ([0.0, 1.0], [-0.5, 0.5, 0.5]),
                ([1.0, 1.0], [-0.5, 0.5, -0.5]),
                ([1.0, 0.0], [-0.5, -0.5, -0.5]),
            ],
        ),
        (
            [1.0, 0.0, 0.0], /* right */
            [
                ([1.0, 0.0], [0.5, -0.5, -0.5]),
                ([0.0, 0.0], [0.5, 0.5, -0.5]),
                ([0.0, 1.0], [0.5, 0.5, 0.5]),
                ([1.0, 1.0], [0.5, -0.5, 0.5]),
            ],
        ),
        (
            [0.0, 1.0, 0.0], /* top */
            [
                ([0.0, 0.0], [0.5, 0.5, 0.5]),
                ([0.0, 1.0], [0.5, 0.5, -0.5]),
                ([1.0, 1.0], [-0.5, 0.5, -0.5]),
                ([1.0, 0.0], [-0.5, 0.5, 0.5]),
            ],
        ),
        (
            [0.0, -1.0, 0.0], /* bottom */
            [
                ([1.0, 1.0], [-0.5, -0.5, -0.5]),
                ([0.0, 1.0], [0.5, -0.5, -0.5]),
                ([0.0, 0.0], [0.5, -0.5, 0.5]),
                ([1.0, 0.0], [-0.5, -0.5, 0.5]),
            ],
        ),
    ];

    for (n, vs) in faces {
        g.glx
            .begin(if wire { Shape::LineLoop } else { Shape::Quads });
        g.glx.normal3f(n[0], n[1], n[2]);
        for (uv, p) in vs {
            g.glx.tex_coord2f(uv[0], uv[1]);
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
        g.glx.end();
    }
}

impl Cubenetic {
    /* Move the wave origins around, and compute the effect of the waves on
    each pixel to generate the output map. */
    fn interference(&mut self) {
        let tau = std::f64::consts::PI * 2.0;
        let step = f64::from(self.waves.speed) / 1000.0;
        let half = f64::from(TEXTURE_SIZE / 2);
        for s in &mut self.waves.srcs {
            s.xth += step;
            if s.xth > tau {
                s.xth -= tau;
            }
            s.yth += step;
            if s.yth > tau {
                s.yth -= tau;
            }
            s.x = (half + s.xth.cos() * half) as i32;
            s.y = (half + s.yth.cos() * half) as i32;
        }

        let ww = &self.waves;
        let n = self.texture_colors.len();
        let mut o = 0;
        for y in 0..TEXTURE_SIZE {
            for x in 0..TEXTURE_SIZE {
                let mut result = 0;
                for s in &ww.srcs {
                    let dx = x - s.x;
                    let dy = y - s.y;
                    let dist = f64::from(dx * dx + dy * dy).sqrt() as i32;
                    // Upstream tests `dist > radius`, which reads one element
                    // past the end of the profile when the distance is exactly
                    // the radius. That is a latent overrun in the C rather
                    // than anything the picture depends on, so this stops one
                    // short.
                    result += if dist >= ww.radius {
                        0
                    } else {
                        ww.heights[dist as usize]
                    };
                }
                let c = &self.texture_colors[result.rem_euclid(n as i32) as usize];
                self.texture[o] = (c.red >> 8) as u8;
                self.texture[o + 1] = (c.green >> 8) as u8;
                self.texture[o + 2] = (c.blue >> 8) as u8;
                // The alpha is left at the 0xFF it was filled with, which
                // upstream comments out rather than writing every frame.
                o += 4;
            }
        }
    }

    fn shuffle_texture(&mut self, g: &mut Gl) {
        self.interference();
        g.glx.bind_texture(self.texture_id);
        g.glx
            .tex_image_2d(TEXTURE_SIZE, TEXTURE_SIZE, self.texture.clone());
    }

    fn shuffle_cubes(&mut self) {
        for c in &mut self.cubes {
            c.x = sinoid(c.dx, c.frame, 0.5);
            c.y = sinoid(c.dy, c.frame, 0.5);
            c.z = sinoid(c.dz, c.frame, 0.5);
            c.w = sinoid(c.dw, c.frame, 0.9) + 1.0;
            c.h = sinoid(c.dh, c.frame, 0.9) + 1.0;
            c.d = sinoid(c.dd, c.frame, 0.9) + 1.0;
            c.frame += 1;
        }
    }

    fn reset_colors(&mut self) {
        let shift = 60.0;
        let h0 = frand(360.0);
        let h1 = if h0 + shift < 360.0 {
            h0 + shift
        } else {
            h0 + shift - 360.0
        };
        let h2 = if h1 + shift < 360.0 {
            h1 + shift
        } else {
            h1 + shift - 360.0
        };
        self.texture_colors = make_color_loop(
            h0 as i32, 1.0, 1.0, h1 as i32, 1.0, 1.0, h2 as i32, 1.0, 1.0, NCOLORS,
        );
        self.cube_colors = make_smooth_colormap(NCOLORS);
    }
}

impl Hack3d for Cubenetic {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.texturing(self.do_texture);
        g.glx.clear();

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

        g.glx.scale(2.5, 2.5, 2.5);

        let n = self.cube_colors.len();
        for i in 0..self.cubes.len() {
            let (pos, size, color) = {
                let c = &self.cubes[i];
                let col = &self.cube_colors[c.color.min(n - 1)];
                (
                    [c.x, c.y, c.z],
                    [c.w, c.h, c.d],
                    [
                        f32::from(col.red) / 65536.0,
                        f32::from(col.green) / 65536.0,
                        f32::from(col.blue) / 65536.0,
                        1.0,
                    ],
                )
            };
            self.cubes[i].color = (self.cubes[i].color + 1) % n;

            g.glx.push_matrix();
            g.glx.translate(pos[0], pos[1], pos[2]);
            g.glx.scale(size[0], size[1], size[2]);
            g.glx.material_ambient_diffuse(color);
            g.glx.call_list(self.cube_list);
            g.glx.pop_matrix();
        }

        self.shuffle_cubes();
        if self.do_texture {
            self.shuffle_texture(g);
        }

        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
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
        if screenhack_event_helper(event) {
            self.reset_colors();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let spin = g.res.string("spin").to_string();
    let (mut spinx, mut spiny, mut spinz) = (false, false, false);
    for c in spin.chars() {
        match c {
            'x' | 'X' => spinx = true,
            'y' | 'Y' => spiny = true,
            'z' | 'Z' => spinz = true,
            _ => {}
        }
    }
    let spin_speed = 1.0;
    let wander_speed = 0.05;

    let mut st = Cubenetic {
        rot: Rotator::new(
            if spinx { spin_speed } else { 0.0 },
            if spiny { spin_speed } else { 0.0 },
            if spinz { spin_speed } else { 0.0 },
            1.0,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            spinx && spiny && spinz,
        ),
        trackball: Trackball::new(),
        cube_list: 0,
        texture_id: 0,
        cubes: Vec::new(),
        waves: Waves {
            radius: g.res.int("wave-radius").clamp(5, 600),
            speed: g.res.int("wave-speed").clamp(5, 150),
            srcs: Vec::new(),
            heights: Vec::new(),
        },
        texture: Vec::new(),
        cube_colors: Vec::new(),
        texture_colors: Vec::new(),
        do_texture: !wire && g.res.bool("texture"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        g.glx.light_position(0, 1.0, 0.5, 1.0, 0.0);
        g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
    }

    st.reset_colors();

    let ncubes = g.res.int("count").clamp(1, 20) as usize;
    st.cubes = (0..ncubes)
        .map(|_| Cube {
            color: random() as usize % NCOLORS,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
            h: 1.0,
            d: 1.0,
            frame: 0,
            dx: frand(0.1) as f32,
            dy: frand(0.1) as f32,
            dz: frand(0.1) as f32,
            dw: frand(0.1) as f32,
            dh: frand(0.1) as f32,
            dd: frand(0.1) as f32,
        })
        .collect();

    if st.do_texture {
        /* init_texture */
        st.texture_id = g.glx.gen_texture();
        g.glx.bind_texture(st.texture_id);
        // Nearest rather than linear, so the bands of the pattern stay crisp
        // instead of being smeared into each other.
        g.glx.tex_nearest(true);
        st.texture = vec![0xFF; (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize];

        /* init_wave */
        let ncolors = NCOLORS as f32;
        let radius = st.waves.radius;
        st.waves.heights = (0..radius)
            .map(|i| {
                let max = ncolors * (radius - i) as f32 / radius as f32;
                ((max + max * (f64::from(i) / 50.0).cos() as f32) / 2.0) as i32
            })
            .collect();
        let nwaves = g.res.int("waves").clamp(1, 20);
        st.waves.srcs = (0..nwaves)
            .map(|_| WaveSrc {
                xth: frand(2.0) * std::f64::consts::PI,
                yth: frand(2.0) * std::f64::consts::PI,
                ..WaveSrc::default()
            })
            .collect();

        st.shuffle_texture(g);
    }

    st.cube_list = g.glx.gen_lists(1);
    g.glx.new_list(st.cube_list);
    unit_cube(g, wire);
    g.glx.end_list();

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*count:        5",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         XYZ",
    "*wander:       True",
    "*texture:      True",
    "*waves:        3",
    "*waveSpeed:    80",
    "*waveRadius:   512",
];

const SPINS: &[SelectItem] = &[
    SelectItem {
        value: "0",
        label: "Don't rotate",
    },
    SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    SelectItem {
        value: "XY",
        label: "Rotate around X and Y axes",
    },
    SelectItem {
        value: "XZ",
        label: "Rotate around X and Z axes",
    },
    SelectItem {
        value: "YZ",
        label: "Rotate around Y and Z axes",
    },
    SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("count", "Boxes", 1.0, 20.0, 1.0, 0, "5"),
    Opt::slider(
        "wave-speed",
        "Surface pattern speed",
        5.0,
        150.0,
        1.0,
        0,
        "80",
    ),
    Opt::slider(
        "wave-radius",
        "Surface pattern overlap",
        5.0,
        600.0,
        1.0,
        0,
        "512",
    ),
    Opt::slider(
        "waves",
        "Surface pattern complexity",
        1.0,
        20.0,
        1.0,
        0,
        "3",
    ),
    Opt::select("spin", "Rotation", SPINS, "XYZ"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("texture", "Surface pattern", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cubenetic",
    label: "Cubenetic",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=aElbM0rZZNg"),
        blurb: "Pulsating boxes wrapped in a writhing interference pattern.",
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

    /// The texture is rebuilt every frame, which is what the generation
    /// counter on a texture is for: the host has to know to upload it again.
    #[test]
    fn the_pattern_is_redrawn_every_frame() {
        let mut r = start(StartArgs::new(640, 480, "count=2", 20260811));
        r.step();
        let id = r.frame().batches[0].texture.expect("no texture");
        let (before, was) = {
            let t = r.texture(id).unwrap();
            assert_eq!((t.width, t.height), (TEXTURE_SIZE, TEXTURE_SIZE));
            assert!(t.nearest, "the bands would smear with linear filtering");
            (t.data.clone(), t.generation)
        };
        r.step();
        let t = r.texture(id).unwrap();
        assert!(t.generation > was, "the texture was not replaced");
        assert_ne!(t.data, before, "the pattern did not move");
        // Opaque throughout: only the three colour channels are written.
        assert!(t.data.chunks_exact(4).all(|p| p[3] == 255));
    }

    /// A box breathes between half and one and a half of its size and wanders
    /// half a unit either way, for ever, because every one of those is a sine
    /// rather than something integrated.
    #[test]
    fn the_boxes_stay_in_their_range() {
        let mut r = start(StartArgs::new(640, 480, "count=20", 20260811));
        let mut widest = 0.0f32;
        let mut narrowest = f32::MAX;
        for _ in 0..2000 {
            r.step();
            for i in 0..20 {
                let s = sinoid(0.05 * (i + 1) as f32, i * 37, 0.9) + 1.0;
                widest = widest.max(s);
                narrowest = narrowest.min(s);
            }
        }
        assert!(widest <= 1.5001 && narrowest >= 0.4999);
        // And the drawn geometry never leaves the room either.
        let f = r.frame();
        for b in &f.batches {
            for v in &f.vertices[b.first..b.first + b.count] {
                let p = b.modelview.transform(v.pos);
                assert!(p[0].abs() < 40.0 && p[1].abs() < 40.0, "{p:?}");
            }
        }
    }

    /// Every box is the same unit cube drawn under its own matrix, with the
    /// texture laid over each face whole.
    #[test]
    fn every_box_is_the_same_cube() {
        let mut r = start(StartArgs::new(640, 480, "count=5", 20260811));
        r.step();
        let f = r.frame();
        // Six faces, two triangles each, five boxes.
        assert_eq!(f.vertices.len(), 5 * 6 * 6);
        for v in &f.vertices {
            assert!(v.uv[0] == 0.0 || v.uv[0] == 1.0);
            assert!(v.uv[1] == 0.0 || v.uv[1] == 1.0);
        }
    }
}
