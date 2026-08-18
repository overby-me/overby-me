//! Port of `hacks/glx/discoball.c`.
//!
//! ```text
//! discoball, Copyright (c) 2016 Jamie Zawinski <jwz@jwz.org>
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
//! A dusty, dented disco ball. Woop woop.
//!
//! There is no ball. The tiles are laid out on a sphere, each one a little
//! slab of four faces facing outwards, and the sphere they sit on is never
//! drawn: instead a single huge quad is drawn edge-on down the middle, writing
//! only depth, so every tile on the far side fails the depth test. That is the
//! whole substrate, one quad standing in for a foam sphere.
//!
//! The dents are what make it look used. Up to four points are picked just
//! outside the ball, and any tile within reach of one is pushed inwards and has
//! its normal bent away, so the reflections in that patch go astray. Tiles near
//! the very apex of a dent are dropped entirely, and one tile in a hundred and
//! fifty is dropped wherever it is, which is the dust.
//!
//! The rays are a flat textured quad each, drawn additively, spun the opposite
//! way from the ball so the beams sweep the room.
//!
//! Two departures. Upstream draws the masking quad with colour writes turned
//! off; here it is simply drawn in black, which is what the masked version
//! leaves behind on a black background and needs no colour mask in the
//! runtime. And its ray texture is `GL_LUMINANCE_ALPHA`, two bytes a pixel;
//! this stores the same thing as RGBA with the luminance in all three colour
//! channels, which is what the fixed pipeline expands it to anyway.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Mat4, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// The ray texture: a soft-edged rectangle of light.
const TEX_SIZE: i32 = 128;

/// `BELLRAND`: three draws averaged, so the middle is likelier.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

/// `RANDSIGN`.
fn randsign() -> f64 {
    if random() & 1 != 0 { 1.0 } else { -1.0 }
}

fn normalize(p: [f32; 3]) -> [f32; 3] {
    let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    if d < 0.000_000_1 {
        [0.0, 0.0, 0.0]
    } else {
        [p[0] / d, p[1] / d, p[2] / d]
    }
}

/// The angle between two vectors, which is how a tile decides whether it is
/// near enough to the apex of a dent to be dropped.
fn vector_angle(a: [f32; 3], b: [f32; 3]) -> f32 {
    let la = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let lb = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt();
    if la == 0.0 || lb == 0.0 || a == b {
        return 0.0;
    }
    let cc = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (la * lb);
    /* avoid fp rounding error (1.000001 => sqrt error) */
    cc.min(1.0).acos()
}

struct Tile {
    position: [f32; 3],
    normal: [f32; 3],
    size: f32,
    tilt: f32,
}

struct Ray {
    normal: [f32; 3],
    color: [f32; 4],
}

struct Discoball {
    rot: Rotator,
    trackball: Trackball,
    th: f32,
    tiles: Vec<Tile>,
    rays: Vec<Ray>,
    tex: u32,
    speed: f32,
    wireframe: bool,
}

/// The ray's light: white, with an alpha that falls off towards the edges on
/// a cosine in each direction.
fn build_texture(g: &mut Gl) -> u32 {
    let size = TEX_SIZE;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let fx = f64::from(x) / f64::from(size - 1) - 0.5;
            let fy = f64::from(y) / f64::from(size - 1) - 0.5;
            let a = (fx * fx * 6.2).cos().min((fy * fy * 6.2).cos()) * 0.4;
            data.extend_from_slice(&[0xFF, 0xFF, 0xFF, (255.0 * a) as u8]);
        }
    }
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(size, size, data);
    id
}

impl Discoball {
    fn draw_rays(&self, g: &mut Gl) {
        let wire = self.wireframe;
        g.glx.texturing(true);
        g.glx.lighting(false);
        g.glx.blend(Blend::AlphaAdd);
        g.glx.depth_test(false);
        g.glx.bind_texture(self.tex);

        let deg = 180.0 / std::f32::consts::PI;
        for r in &self.rays {
            let [x, y, z] = r.normal;
            g.glx.push_matrix();

            /* Orient to direction of ray. */
            g.glx.rotate(-x.atan2(y) * deg, 0.0, 0.0, 1.0);
            g.glx
                .rotate(z.atan2((x * x + y * y).sqrt()) * deg, 1.0, 0.0, 0.0);

            g.glx.scale(5.0, 5.0, 10.0);
            g.glx.translate(0.0, 0.0, 1.1);
            g.glx
                .color4f(r.color[0], r.color[1], r.color[2], r.color[3]);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            for (u, v, px, pz) in [
                (0.0, 0.0, -0.5, -1.0),
                (1.0, 0.0, 0.5, -1.0),
                (1.0, 1.0, 0.5, 1.0),
                (0.0, 1.0, -0.5, 1.0),
            ] {
                g.glx.tex_coord2f(u, v);
                g.glx.vertex3f(px, 0.0, pz);
            }
            g.glx.end();
            g.glx.pop_matrix();
        }

        g.glx.texturing(false);
        g.glx.lighting(!wire);
        g.glx.blend(Blend::Off);
        g.glx.depth_test(true);
    }

    fn draw_ball_1(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let m = g.glx.modelview();
        // The same matrix with its rotation and scale thrown away, keeping
        // only where the ball is. Both the mask and the rays hang off it, so
        // they always face the camera.
        let mut flat = Mat4::IDENTITY;
        flat.0[12] = m.0[12];
        flat.0[13] = m.0[13];
        flat.0[14] = m.0[14];
        flat.0[15] = m.0[15];

        g.glx.front_face_cw(true);

        /* Instead of rendering polygons for the foam ball substrate, let's
        just billboard a quad down the middle to mask out the back-facing
        tiles. */
        {
            g.glx.push_matrix();
            g.glx.load_identity();
            g.glx.mult_matrix(flat);
            g.glx.scale(40.0, 40.0, 40.0);
            g.glx.translate(-0.5, -0.5, -0.01);
            g.glx.lighting(false);
            // Drawn in black rather than with colour writes masked off: on a
            // black background the two are the same picture, and it is the
            // depth it writes that does the work.
            g.glx.color4f(0.0, 0.0, 0.0, 1.0);
            g.glx.begin(Shape::Quads);
            for (x, y) in [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)] {
                g.glx.vertex3f(x, y, 0.0);
            }
            g.glx.end();
            g.glx.lighting(!wire);
            g.glx.pop_matrix();
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        }

        /* Draw all the tiles. */
        let deg = 180.0 / std::f32::consts::PI;
        for t in &self.tiles {
            let [x, y, z] = t.normal;
            let s = t.size / 2.0;
            g.glx.push_matrix();

            /* Move to location of tile. */
            g.glx.translate(t.position[0], t.position[1], t.position[2]);

            /* Orient to direction tile is facing. */
            g.glx.rotate(-x.atan2(y) * deg, 0.0, 0.0, 1.0);
            g.glx
                .rotate(z.atan2((x * x + y * y).sqrt()) * deg, 1.0, 0.0, 0.0);
            g.glx.rotate(t.tilt, 0.0, 1.0, 0.0);
            g.glx.scale(s, s, s);

            let face = |g: &mut Gl, n: [f32; 3], vs: [[f32; 3]; 4]| {
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx
                    .begin(if wire { Shape::LineLoop } else { Shape::Quads });
                for v in vs {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            };

            face(
                g,
                [0.0, 1.0, 0.0],
                [
                    [-1.0, 0.0, -1.0],
                    [1.0, 0.0, -1.0],
                    [1.0, 0.0, 1.0],
                    [-1.0, 0.0, 1.0],
                ],
            );

            if !wire {
                // The four little sides, which is what gives a tile an edge to
                // catch the light on.
                let d = 0.2;
                face(
                    g,
                    [0.0, 0.0, -1.0],
                    [
                        [-1.0, 0.0, -1.0],
                        [-1.0, -d, -1.0],
                        [1.0, -d, -1.0],
                        [1.0, 0.0, -1.0],
                    ],
                );
                face(
                    g,
                    [0.0, 0.0, 1.0],
                    [
                        [1.0, 0.0, 1.0],
                        [1.0, -d, 1.0],
                        [-1.0, -d, 1.0],
                        [-1.0, 0.0, 1.0],
                    ],
                );
                face(
                    g,
                    [1.0, 0.0, 0.0],
                    [
                        [1.0, 0.0, -1.0],
                        [1.0, -d, -1.0],
                        [1.0, -d, 1.0],
                        [1.0, 0.0, 1.0],
                    ],
                );
                face(
                    g,
                    [-1.0, 0.0, 0.0],
                    [
                        [-1.0, 0.0, 1.0],
                        [-1.0, -d, 1.0],
                        [-1.0, -d, -1.0],
                        [-1.0, 0.0, -1.0],
                    ],
                );
            }

            g.glx.pop_matrix();
        }

        /* Draw the front rays. */
        if !wire {
            g.glx.push_matrix();
            g.glx.load_identity();
            g.glx.mult_matrix(flat);
            g.glx.translate(0.0, 0.0, 4.1);
            g.glx.rotate(-self.th, 0.0, 0.0, 1.0);
            self.draw_rays(g);
            g.glx.pop_matrix();
        }
    }
}

impl Hack3d for Discoball {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 6.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 2.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.rotate(50.0, 1.0, 0.0, 0.0);
        g.glx.scale(4.0, 4.0, 4.0);
        g.glx.rotate(self.th, 0.0, 0.0, 1.0);
        if !down {
            // It turns whichever way it was already going, and it was started
            // pointing at a random one of the two.
            self.th += if self.th > 0.0 {
                self.speed
            } else {
                -self.speed
            };
            while self.th > 360.0 {
                self.th -= 360.0;
            }
            while self.th < -360.0 {
                self.th += 360.0;
            }
        }

        self.draw_ball_1(g);
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
        self.trackball.event(event, g.width(), g.height())
    }
}

/// Lay the tiles out on a sphere, dent it, and drop the dust.
fn build_ball(rows: i32) -> (Vec<Tile>, Vec<Ray>) {
    let pi = std::f32::consts::PI;
    let tile_size = pi / rows as f32;

    struct Dent {
        position: [f32; 3],
        strength: f32,
    }
    let dent_count = (random() % 5) as usize;
    let mut dents = Vec::with_capacity(dent_count);
    for _ in 0..dent_count {
        let position = [
            (randsign() * (2.0 - bellrand(0.2))) as f32,
            (randsign() * (2.0 - bellrand(0.2))) as f32,
            (randsign() * (2.0 - bellrand(0.2))) as f32,
        ];
        let dist =
            (position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
                .sqrt();
        // Upstream computes this twice over, the second overwriting the
        // first. Kept, because the first one draws three random numbers and
        // everything after it depends on where the sequence is.
        let _ = dist - (1.0 - bellrand(0.3)) as f32;
        let strength = dist - (1.0 - bellrand(0.3)) as f32;
        dents.push(Dent { position, strength });
    }

    let mut tiles = Vec::new();
    let mut th1 = pi / 2.0;
    while th1 > -(pi / 2.0 + tile_size / 2.0) {
        let x = th1.cos();
        let y = th1.sin();
        let x0 = (th1 - tile_size / 2.0).cos();
        let x1 = (th1 + tile_size / 2.0).cos();
        let circ = (pi * x0 * 2.0).min(pi * x1 * 2.0);
        let row_tiles = (circ.max(0.0) / tile_size).floor() as i32;
        let row_tiles = row_tiles.max(1);
        let spacing = pi * 2.0 / row_tiles as f32;
        let dropsy = 0.13 + frand(0.04) as f32;

        let mut th0 = 0.0f32;
        while th0 < pi * 2.0 {
            let mut position = [th0.cos() * x, th0.sin() * x, y];
            let mut normal = position;
            let mut dropped = false;

            /* Apply pressure on position from the dents. */
            for d in &dents {
                if random().is_multiple_of(150) {
                    /* Drop tiles randomly */
                    dropped = true;
                    break;
                }

                let direction = [
                    position[0] - d.position[0],
                    position[1] - d.position[1],
                    position[2] - d.position[2],
                ];
                let dist = (direction[0] * direction[0]
                    + direction[1] * direction[1]
                    + direction[2] * direction[2])
                    .sqrt();
                if dist < d.strength {
                    let s = 1.0 - (d.strength - dist) * 0.66;
                    let mut n2 = normal;
                    let angle = vector_angle(position, d.position);

                    /* Drop out the tiles near the apex of the dent. */
                    if angle < dropsy {
                        dropped = true;
                        break;
                    }

                    for p in &mut position {
                        *p *= s;
                    }
                    let direction = normalize(direction);
                    for k in 0..3 {
                        n2[k] -= direction[k];
                        normal[k] = (normal[k] + n2[k]) / 2.0;
                    }
                }
            }

            if !dropped {
                /* Skew the direction the tile is facing slightly. */
                for n in &mut normal {
                    *n += 0.12 - frand(0.06) as f32;
                }
                tiles.push(Tile {
                    position,
                    normal,
                    size: tile_size * 0.85,
                    tilt: 4.0 - bellrand(8.0) as f32,
                });
            }
            th0 += spacing;
        }
        th1 -= tile_size;
    }

    let nrays = 5 + bellrand(10.0) as usize;
    let rays = (0..nrays)
        .map(|_| {
            let th = frand(f64::from(pi) * 2.0) as f32;
            Ray {
                normal: normalize([th.cos(), th.sin(), 1.0]),
                color: [
                    0.9 + frand(0.1) as f32,
                    0.6 + frand(0.4) as f32,
                    0.6 + frand(0.2) as f32,
                    1.0,
                ],
            }
        })
        .collect();

    (tiles, rays)
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let tex = if wire { 0 } else { build_texture(g) };

    let th = 180.0 - frand(360.0) as f32;
    let rows = g.res.int("count").clamp(10, 200);

    let spin = g.res.bool("spin");
    let spin_speed = 0.1;
    let wander_speed = 0.003;
    let spin_accel = 1.0;

    let mut st = Discoball {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        trackball: Trackball::new(),
        th,
        tiles: Vec::new(),
        rays: Vec::new(),
        tex,
        speed: g.res.float("speed") as f32,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    let (tiles, rays) = build_ball(rows);
    st.tiles = tiles;
    st.rays = rays;

    if !wire {
        // Two lights, both from below, which is where a disco ball's light
        // comes from. The material and the specular are each nudged at random
        // so no two balls are quite the same colour.
        g.glx.lighting(true);
        for (i, pos) in [[0.5, -1.0, -0.5, 0.0], [-0.75, -1.0, 0.0, 0.0]]
            .into_iter()
            .enumerate()
        {
            g.glx.light_enable(i, true);
            g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
            g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(i, [1.0, 1.0, 1.0, 1.0]);
        }
        let color = [
            0.5 + frand(0.2) as f32,
            0.5 + frand(0.2) as f32,
            0.5 + frand(0.2) as f32,
            1.0,
        ];
        let cspec = [
            1.0 - frand(0.2) as f32,
            1.0 - frand(0.2) as f32,
            1.0 - frand(0.2) as f32,
            1.0,
        ];
        g.glx.material_ambient_diffuse(color);
        g.glx.material_specular(cspec);
        g.glx.material_shininess(10.0);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*spin:         False",
    "*wander:       True",
    "*speed:        1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::slider("count", "Size", 10.0, 100.0, 1.0, 0, "30"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "discoball",
    label: "Discoball",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=8yd4PYJQrMw"),
        blurb: "A dusty, dented disco ball. Woop woop.",
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
    use crate::runtime::ya_rand_init;

    /// The tiles sit on a sphere, all but the dented ones, and the dents only
    /// ever push inwards.
    #[test]
    fn the_tiles_lie_on_a_ball() {
        ya_rand_init(20260811);
        let (tiles, _) = build_ball(30);
        assert!(tiles.len() > 500, "only {} tiles", tiles.len());
        let mut dented = 0;
        for t in &tiles {
            let r = (t.position[0] * t.position[0]
                + t.position[1] * t.position[1]
                + t.position[2] * t.position[2])
                .sqrt();
            assert!(r <= 1.001, "a tile {r} from the middle, outside the ball");
            if r < 0.99 {
                dented += 1;
            }
        }
        assert!(dented < tiles.len() / 2, "the whole ball is dented");
    }

    /// The rays are flat quads with the light texture on them, drawn
    /// additively so they add up where they cross.
    #[test]
    fn the_rays_are_textured_and_additive() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let rays: Vec<_> = f.batches.iter().filter(|b| b.texture.is_some()).collect();
        assert!((5..=15).contains(&rays.len()), "{} rays", rays.len());
        assert!(rays.iter().all(|b| b.blend == Blend::AlphaAdd));
        assert!(rays.iter().all(|b| !b.lighting));
        // Soft edged: transparent at the corners and brightest in the middle.
        let id = rays[0].texture.unwrap();
        let t = r.texture(id).unwrap();
        let alpha = |x: i32, y: i32| t.data[((y * TEX_SIZE + x) * 4 + 3) as usize];
        assert!(alpha(TEX_SIZE / 2, TEX_SIZE / 2) > alpha(2, 2));
        assert!(t.data.chunks_exact(4).all(|p| p[0] == 255 && p[1] == 255));
    }

    /// The ball is masked by one quad drawn edge-on down its middle, which is
    /// what stands in for the sphere the tiles are glued to.
    #[test]
    fn one_quad_stands_in_for_the_sphere() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        // It is drawn first, in black, and it is enormous.
        let b = &f.batches[0];
        let vs = &f.vertices[b.first..b.first + b.count];
        assert!(vs.iter().all(|v| v.color[..3] == [0.0, 0.0, 0.0]));
        // Measured in eye space, since the size is in the matrix: forty units
        // across, against a ball of radius four.
        let xs: Vec<f32> = vs.iter().map(|v| b.modelview.transform(v.pos)[0]).collect();
        let span = xs.iter().copied().fold(f32::MIN, f32::max)
            - xs.iter().copied().fold(f32::MAX, f32::min);
        assert!((span - 40.0).abs() < 1e-3, "the mask is {span} across");
    }

    /// It turns, and it keeps turning the way it started rather than
    /// oscillating.
    #[test]
    fn the_ball_turns_one_way() {
        let mut r = start(StartArgs::new(640, 480, "count=10", 20260811));
        let mut last = None;
        let mut steps = Vec::new();
        for _ in 0..100 {
            r.step();
            // The tiles all hang off the ball's own rotation, and the last
            // batch of the frame is one of the rays, so use a tile.
            let f = r.frame();
            let b = &f.batches[1];
            let angle = b.modelview.0[1].atan2(b.modelview.0[0]);
            if let Some(prev) = last {
                let d: f32 = angle - prev;
                if d.abs() < 1.0 {
                    steps.push(d);
                }
            }
            last = Some(angle);
        }
        assert!(steps.len() > 50);
        let forward = steps.iter().filter(|d| **d > 0.0).count();
        assert!(
            forward == 0 || forward == steps.len(),
            "it changed direction: {forward} of {}",
            steps.len()
        );
    }
}
