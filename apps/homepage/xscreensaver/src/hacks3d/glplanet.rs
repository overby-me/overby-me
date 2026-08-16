//! Port of `hacks/glx/glplanet.c`.
//!
//! ```text
//! glplanet --- Animates texture mapped sphere (planet)
//!
//! Copyright (c) 1997-2002 by David Konerding
//! Copyright (c) 1998-2022 by Jamie Zawinski
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
//! ```
//!
//! The Earth, bouncing around in space, drawn from satellite photographs of
//! it in daylight and at night. Sometimes it is a globe, sometimes the map is
//! unrolled flat onto a cylinder as Mercator or as plain latitude and
//! longitude. Every so often the lines of latitude and longitude come up over
//! it, or the ragged boundaries of the time zones.
//!
//! The day and night sides are two separate photographs of the same map, and
//! the whole difficulty is the line between them. Lighting the sphere from a
//! lamp puts the terminator wherever the polygon edges happen to fall, which
//! looks like a bitten edge however many polygons there are, so upstream does
//! it in the framebuffer instead: it fills the alpha channel from a
//! half-sphere turned to face the sun, adds a soft ring at the rim of that
//! half-sphere for dusk, and blends the night map in wherever alpha is low.
//!
//! This canvas has no alpha channel to fill, so the same falloff is worked
//! out per vertex instead: how far a point of the sphere is from the plane of
//! the terminator, softened over the width upstream gives dusk, straight into
//! the vertex's own alpha. Ordinary transparency then blends the two maps in
//! exactly the proportion upstream's destination alpha would have. It is the
//! same picture and it needs nothing of the framebuffer.
//!
//! The maps themselves are a quarter of upstream's size in each direction:
//! see the note in the README. Upstream also flips them with a texture
//! matrix, which this runtime has none of, so the coordinate is negated where
//! it is emitted; the textures repeat, so that samples the same row.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, DepthFunc, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};
use std::f32::consts::PI;

/// How wide dusk is, as a fraction of the sphere's radius. Upstream's
/// terminator tube is this thick, and calls it about an hour.
const DUSK: f32 = 0.1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Globe,
    Equirectangular,
    Mercator,
}

/// One vertex of a generated surface.
#[derive(Clone, Copy)]
struct Vert {
    p: [f32; 3],
    n: [f32; 3],
    s: f32,
    t: f32,
}

/// A star, and how big to draw it.
struct Star {
    p: [f32; 3],
    c: [f32; 3],
    size: f32,
}

struct Planet {
    rot: Rotator,
    trackball: Trackball,

    mode: Mode,
    wireframe: bool,
    do_texture: bool,
    do_stars: bool,
    do_rotate: bool,
    do_roll: bool,
    spin: f32,

    /// The surface, in the order a triangle strip wants it.
    plate: Vec<Vert>,
    /// The wireframe grid of latitude and longitude, as line pairs.
    latlong: Vec<[f32; 3]>,
    /// The time zone boundaries, as line pairs on the globe.
    tz: Vec<[f32; 3]>,
    stars: Vec<Star>,

    tex_day: u32,
    tex_night: u32,

    /// How far round the earth has turned, from nought to one.
    z: f32,
    /// The tilt of the earth's axis against the sun.
    tilt: f32,
    /// Frames left to keep the grid or the time zones up, and which.
    draw_axis: i32,
    timezones_p: bool,
}

/// `unit_sphere`, kept rather than drawn: upstream's own strip order, with
/// the texture coordinates it puts on it.
fn unit_sphere(stacks: i32, slices: i32) -> Vec<Vert> {
    let stacks2 = stacks * 2;
    let mut out = Vec::with_capacity(((stacks + 1) * (slices + 1) * 2) as usize);
    for j in 0..stacks {
        let theta1 = j as f32 * (PI + PI) / stacks2 as f32 - PI / 2.0;
        let theta2 = (j + 1) as f32 * (PI + PI) / stacks2 as f32 - PI / 2.0;
        for i in (0..=slices).rev() {
            let theta3 = i as f32 * (PI + PI) / slices as f32;
            for (theta, tt) in [(theta2, 2 * (j + 1)), (theta1, 2 * j)] {
                let n = [
                    theta.cos() * theta3.cos(),
                    theta.sin(),
                    theta.cos() * theta3.sin(),
                ];
                out.push(Vert {
                    p: n,
                    n,
                    s: i as f32 / slices as f32,
                    t: tt as f32 / stacks2 as f32,
                });
            }
        }
    }
    out
}

/// `unit_mercator`: the map rolled round a cylinder rather than a ball, with
/// the poles either stretched off to infinity or simply left off.
fn unit_mercator(stacks: i32, slices: i32, mercp: bool) -> Vec<Vert> {
    let stacks = stacks / 2;
    let xs = 1.0 / slices as f32;
    let ys = 1.0 / stacks as f32;
    let outer = 1.8;
    let r = 0.35; /* Grids are roughly square at equator */

    let (north, south) = if mercp {
        /* The poles go to infinity. The traditional Mercator projection
        omits the Northern and Southern latitudes asymmetrically to move
        Europe toward the center.  How Colonial! */
        (85.0 / 180.0, -66.0 / 180.0)
    } else {
        /* Empirically, this puts the parallels in the right place. */
        (73.5 / 180.0, -73.5 / 180.0)
    };

    // One quad ring at a time, each ring its own strip, all scaled up by the
    // factor upstream applies with a matrix.
    let mut out = Vec::new();
    let mut lasty = -0.5f32;
    let mut lastty = 0.0f32;
    let mut y = -0.5f32;
    for j in 0..=stacks {
        let mut ty = (0.5 - y) * (south - north) - south + 0.5;
        if mercp {
            /* Obviously I have no idea what I'm doing here */
            let th = PI * (ty - 0.5);
            ty = 2.0 * (th.exp().atan() - PI / 4.0);
            ty *= 0.41;
            ty += 0.5;
        }

        if j > 0 {
            for i in 1..=slices {
                let x = i as f32 * xs;
                let lastx = (i - 1) as f32 * xs;
                let xx = r * (PI * 2.0 * x).cos();
                let yy = r * (PI * 2.0 * x).sin();
                let lx = r * (PI * 2.0 * lastx).cos();
                let ly = r * (PI * 2.0 * lastx).sin();
                // Two triangles a quad, wound as the quad was.
                let corners = [
                    ([lx, lasty, ly], [lx, 0.0, ly], lastx, lastty),
                    ([xx, lasty, yy], [xx, 0.0, yy], x, lastty),
                    ([xx, y, yy], [xx, 0.0, yy], x, ty),
                    ([lx, lasty, ly], [lx, 0.0, ly], lastx, lastty),
                    ([xx, y, yy], [xx, 0.0, yy], x, ty),
                    ([lx, y, ly], [lx, 0.0, ly], lastx, ty),
                ];
                for (p, n, s, t) in corners {
                    out.push(Vert {
                        p: [p[0] * outer, p[1] * outer, p[2] * outer],
                        n,
                        s,
                        t,
                    });
                }
            }
        }

        lasty = y;
        lastty = ty;
        y += ys;
    }
    out
}

impl Planet {
    /// `init_stars`.
    fn init_stars(&mut self, width: i32, height: i32) {
        let size = width.max(height);
        let nstars = size * size / 80;
        let max_size = 3;
        let inc = 0.5f32;
        let steps = (max_size as f32 / inc) as i32;
        let scale = 1.0;

        for j in 1..=steps {
            for _ in 0..nstars / steps {
                let mut d = 0.1;
                let r = 0.15 + frand(0.3) as f32;
                let g = r + frand(d) as f32 - d as f32;
                let b = r + frand(d) as f32 - d as f32;
                let x = frand(1.0) as f32 - 0.5;
                let y = frand(1.0) as f32 - 0.5;
                let z = if random() & 1 != 0 {
                    frand(1.0) as f32 - 0.5
                } else {
                    // The milky way: a band rather than an even scattering.
                    ((frand(1.0) + frand(1.0) + frand(1.0)) as f32 / 3.0 - 0.5) / 12.0
                };
                d = f64::from(x * x + y * y + z * z).sqrt();
                let d = d as f32;
                self.stars.push(Star {
                    p: [x / d, y / d, z / d],
                    c: [r, g, b],
                    size: inc * j as f32 * scale,
                });
            }
        }
    }

    /// The lines of latitude and longitude, drawn as the wireframe of a much
    /// coarser sphere, plus the axis through the poles.
    fn init_latlong(&mut self) {
        let (stacks, slices) = if self.mode == Mode::Globe {
            (12, 24)
        } else {
            (20, 24)
        };
        let verts = if self.mode == Mode::Globe {
            unit_sphere(stacks, slices)
        } else {
            unit_mercator(stacks, slices, self.mode == Mode::Mercator)
        };
        // The strip drawn as a wireframe is every edge of every triangle.
        for w in verts.chunks(3) {
            if w.len() < 3 {
                break;
            }
            for k in 0..3 {
                self.latlong.push(w[k].p);
                self.latlong.push(w[(k + 1) % 3].p);
            }
        }
        self.latlong.push([0.0, -2.0, 0.0]);
        self.latlong.push([0.0, 2.0, 0.0]);
    }

    /// The time zone boundaries. They arrive as line segments on a flat
    /// equirectangular map, so each one is cut into pieces and every piece
    /// projected onto the sphere, which is what makes them curve.
    fn init_timezones(&mut self) {
        let list = GlList::parse(crate::models::TIMEZONES);
        let p = &list.data;
        let min_seg = 0.05;

        let mut minx = f32::MAX;
        let mut miny = f32::MAX;
        let mut maxx = f32::MIN;
        let mut maxy = f32::MIN;
        for v in p.chunks_exact(3) {
            minx = minx.min(v[0]);
            maxx = maxx.max(v[0]);
            miny = miny.min(v[1]);
            maxy = maxy.max(v[1]);
        }

        for seg in p.chunks_exact(6) {
            let x0 = minx + seg[0] / (maxx - minx);
            let y0 = miny + seg[1] / (maxy - miny);
            let x1 = minx + seg[3] / (maxx - minx);
            let y1 = miny + seg[4] / (maxy - miny);

            let d = ((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)).sqrt();
            let steps = ((d / min_seg) as i32).max(1);

            let at = |i: i32| {
                let r = i as f32 / steps as f32;
                let x2 = x0 + r * (x1 - x0);
                let y2 = y0 + r * (y1 - y0);
                let th1 = y2 * PI - PI / 2.0; /* longitude radians */
                let th2 = -x2 * PI * 2.0; /* latitude radians */
                [th1.cos() * th2.cos(), th1.sin(), th1.cos() * th2.sin()]
            };
            for i in 0..steps {
                self.tz.push(at(i));
                self.tz.push(at(i + 1));
            }
        }
    }

    /// Emit the surface, with one alpha per vertex.
    ///
    /// `night` gives the coefficients of the plane the terminator lies in:
    /// their dot product with a vertex says which side of the sun it is on,
    /// and the width of dusk softens the change from one to the other.
    fn draw_plate(&self, g: &mut Gl, night: Option<[f32; 3]>) {
        g.glx.begin(if self.mode == Mode::Globe {
            Shape::TriangleStrip
        } else {
            Shape::Triangles
        });
        for v in &self.plate {
            let a = match night {
                None => 1.0,
                Some(k) => {
                    let d = k[0] * v.p[0] + k[1] * v.p[1] + k[2] * v.p[2];
                    ((d + DUSK) / (2.0 * DUSK)).clamp(0.0, 1.0)
                }
            };
            g.glx.color4f(1.0, 1.0, 1.0, a);
            // Upstream flips the map with a texture matrix; negating the
            // coordinate reads the same row of a repeating texture.
            g.glx.tex_coord2f(v.s, -v.t);
            g.glx.normal3f(v.n[0], v.n[1], v.n[2]);
            g.glx.vertex3f(v.p[0], v.p[1], v.p[2]);
        }
        g.glx.end();
    }

    fn draw_lines(g: &mut Gl, lines: &[[f32; 3]]) {
        g.glx.begin(Shape::Lines);
        for p in lines {
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
        g.glx.end();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let mode_arg = g.res.string("mode").to_string();
    let (mode, random_p) = match mode_arg.to_ascii_lowercase().as_str() {
        "globe" => (Mode::Globe, false),
        "equirectangular" | "rectangular" | "rect" | "equi" | "eq" => {
            (Mode::Equirectangular, false)
        }
        "mercator" | "merc" => (Mode::Mercator, false),
        // Random: mostly the globe, and the flat one now and then.
        _ if !random().is_multiple_of(6) => (Mode::Globe, false),
        _ => (Mode::Mercator, true),
    };

    let do_roll = g.res.bool("roll");
    let do_wander = g.res.bool("wander");
    let resolution = g.res.int("resolution").clamp(4, 256);
    let spin_speed = 0.1;
    let wander_speed = 0.005;

    let mut this = Planet {
        rot: Rotator::new(
            if do_roll { spin_speed } else { 0.0 },
            if do_roll { spin_speed } else { 0.0 },
            0.0,
            1.0,
            if do_wander && !random_p {
                wander_speed
            } else {
                0.0
            },
            true,
        ),
        trackball: Trackball::new(),
        mode,
        wireframe,
        do_texture: g.res.bool("texture") && !wireframe,
        do_stars: g.res.bool("stars"),
        do_rotate: g.res.bool("rotate"),
        do_roll,
        spin: g.res.float("spin") as f32,
        plate: Vec::new(),
        latlong: Vec::new(),
        tz: Vec::new(),
        stars: Vec::new(),
        tex_day: 0,
        tex_night: 0,
        z: frand(1.0) as f32,
        tilt: frand(23.4) as f32,
        draw_axis: 0,
        timezones_p: false,
    };

    if this.do_texture {
        for (bytes, slot) in [(crate::images::EARTH, 0), (crate::images::EARTH_NIGHT, 1)] {
            if let Some((w, h, px)) = crate::runtime::png::decode_rgba(bytes) {
                let id = g.glx.gen_texture();
                g.glx.bind_texture(id);
                g.glx.tex_image_2d(w, h, px);
                g.glx.tex_clamp(false);
                g.glx.tex_nearest(false);
                if slot == 0 {
                    this.tex_day = id;
                } else {
                    this.tex_night = id;
                }
            }
        }
        if this.tex_day == 0 {
            this.do_texture = false;
        }
    }

    this.plate = if mode == Mode::Globe {
        unit_sphere(resolution, resolution)
    } else {
        unit_mercator(resolution, resolution, mode == Mode::Mercator)
    };
    this.init_latlong();
    if mode == Mode::Globe {
        this.init_timezones();
    }
    if this.do_stars {
        let (w, h) = (g.width(), g.height());
        this.init_stars(w, h);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Planet {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -h, h, 5.0, 200.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.translate(0.0, 0.0, -40.0);
        if width <= height {
            g.glx.scale(h, h, h);
        }
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;
        let down = self.trackball.button_down();

        // Every so often, and whenever the mouse is down, bring the grid or
        // the time zones up for a while.
        if down {
            if self.draw_axis == 0 {
                self.timezones_p = self.mode == Mode::Globe && random().is_multiple_of(2);
            }
            self.draw_axis = 60;
        } else if self.draw_axis == 0 && random().is_multiple_of(1000) {
            self.draw_axis = 60 + (random() % 90) as i32;
            self.timezones_p = self.mode == Mode::Globe && random().is_multiple_of(10);
        }

        if self.do_rotate && !down {
            let wat = if self.mode == Mode::Globe { 1.0 } else { -1.0 };
            self.z -= 0.001 * self.spin * wat; /* the sun sets in the west */
            if self.z < 0.0 {
                self.z += 1.0;
            }
        }

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.front_face_cw(false);
        g.glx.blend(Blend::Off);
        g.glx.depth_func(DepthFunc::Less);
        g.glx.color_material(true);
        g.glx.lighting(false);

        g.glx.push_matrix();

        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 6.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 3.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.do_roll {
            let (rx, ry, _) = self.rot.rotation(!down);
            g.glx.rotate(rx as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(ry as f32 * 360.0, 0.0, 1.0, 0.0);
        }

        if self.do_stars {
            g.glx.texturing(false);
            g.glx.push_matrix();
            g.glx.scale(60.0, 60.0, 60.0);
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
            g.glx.rotate(35.0, 1.0, 0.0, 0.0);
            let mut size = 0.0;
            for s in &self.stars {
                if s.size != size {
                    if size != 0.0 {
                        g.glx.end();
                    }
                    size = s.size;
                    g.glx.point_size(size);
                    g.glx.begin(Shape::Points);
                }
                g.glx.color3f(s.c[0], s.c[1], s.c[2]);
                g.glx.vertex3f(s.p[0], s.p[1], s.p[2]);
            }
            if size != 0.0 {
                g.glx.end();
            }
            g.glx.pop_matrix();
            // The stars are a backdrop: nothing in front of them may be
            // hidden by them however far away they were drawn.
            g.glx.clear_depth();
        }

        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(35.0, 1.0, 0.0, 0.0);
        g.glx.rotate(10.0, 0.0, 1.0, 0.0);
        g.glx.rotate(120.0, 0.0, 0.0, 1.0);
        g.glx.scale(3.0, 3.0, 3.0);

        if wire {
            g.glx.color3f(0.0, 0.0, 0.5);
        } else if self.do_texture {
            g.glx.texturing(true);
            g.glx.bind_texture(self.tex_day);
        } else {
            g.glx.texturing(false);
            g.glx.color3f(0.0, 0.5, 0.0);
        }

        g.glx.push_matrix();
        g.glx.rotate(self.z * 360.0, 0.0, 0.0, 1.0);
        self.draw_plate(g, None);
        g.glx.pop_matrix();

        // The night side, blended in wherever the sun is not.
        if !wire && self.mode == Mode::Globe && self.tex_night != 0 {
            // Which side of the sun a point of the sphere is on. Upstream
            // turns the map upright, spins it by the time of day and holds a
            // half-sphere against it tilted by the axis; a point is in
            // daylight when it is inside that half. Composing those three
            // rotations and keeping only the height leaves one plane, and
            // these are its coefficients.
            let a = self.z * 2.0 * PI;
            let b = self.tilt * PI / 180.0;
            let k = [a.sin() * b.cos(), b.sin(), -a.cos() * b.cos()];

            g.glx.blend(Blend::Alpha);
            g.glx.depth_func(DepthFunc::LessEqual);
            g.glx.texturing(true);
            g.glx.bind_texture(self.tex_night);
            g.glx.push_matrix();
            g.glx.rotate(self.z * 360.0, 0.0, 0.0, 1.0);
            self.draw_plate(g, Some(k));
            g.glx.pop_matrix();
            g.glx.blend(Blend::Off);
            g.glx.depth_func(DepthFunc::Less);
        }

        if self.draw_axis > 0 {
            g.glx.push_matrix();
            g.glx.rotate(self.z * 360.0, 0.0, 0.0, 1.0);
            g.glx.scale(1.02, 1.02, 1.02);
            g.glx.texturing(false);
            g.glx.color3f(0.1, 0.3, 0.1);
            if self.timezones_p {
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(180.0, 0.0, 0.0, 1.0);
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
                Self::draw_lines(g, &self.tz);
            } else {
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(8.0, 0.0, 1.0, 0.0);
                Self::draw_lines(g, &self.latlong);
            }
            g.glx.pop_matrix();
            self.draw_axis -= 1;
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*rotate:       True",
    "*roll:         True",
    "*wander:       True",
    "*texture:      True",
    "*stars:        True",
    "*spin:         1.0",
    "*resolution:   128",
    "*mode:         RANDOM",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random Shape",
    },
    SelectItem {
        value: "globe",
        label: "Globe",
    },
    SelectItem {
        value: "mercator",
        label: "Mercator",
    },
    SelectItem {
        value: "equirectangular",
        label: "Equirectangular",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::select("mode", "Shape", MODES, "random"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("rotate", "Rotate", "true"),
    Opt::boolean("roll", "Roll", "true"),
    Opt::boolean("stars", "Stars", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glplanet",
    label: "GL Planet",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Konerding and Jamie Zawinski",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=ohcJ1bVkLZ4"),
        blurb: "The Earth, bouncing around in space, rendered with satellite \
                imagery of the planet in both sunlight and darkness.",
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

    #[test]
    fn the_globe_is_a_unit_sphere_with_the_map_laid_over_it() {
        let v = unit_sphere(32, 32);
        for w in &v {
            let r = (w.p[0] * w.p[0] + w.p[1] * w.p[1] + w.p[2] * w.p[2]).sqrt();
            assert!((r - 1.0).abs() < 1e-5, "{r}");
            assert!((0.0..=1.0).contains(&w.s), "s is {}", w.s);
            assert!((0.0..=1.0).contains(&w.t), "t is {}", w.t);
        }
        // The whole map is used: both poles and both edges.
        let lo = v.iter().map(|w| w.t).fold(f32::MAX, f32::min);
        let hi = v.iter().map(|w| w.t).fold(f32::MIN, f32::max);
        assert!(lo < 0.01 && hi > 0.99, "{lo} to {hi}");
    }

    #[test]
    fn the_night_side_is_opposite_the_day_side() {
        // The alpha the night map is blended with runs from nothing on the
        // sunlit side to all of it on the dark side, over the width of dusk,
        // and the two sides are the same size.
        let v = unit_sphere(32, 32);
        let k = [0.0, 0.0, 1.0]; /* the terminator through the equator */
        let alpha = |w: &Vert| {
            let d = k[0] * w.p[0] + k[1] * w.p[1] + k[2] * w.p[2];
            ((d + DUSK) / (2.0 * DUSK)).clamp(0.0, 1.0)
        };
        let day = v.iter().filter(|w| alpha(w) == 0.0).count();
        let night = v.iter().filter(|w| alpha(w) == 1.0).count();
        let dusk = v.iter().filter(|w| {
            let a = alpha(w);
            a > 0.0 && a < 1.0
        });
        assert!(day > 0 && night > 0);
        let ratio = day as f32 / night as f32;
        assert!((0.8..1.25).contains(&ratio), "{day} lit and {night} dark");
        // And there really is a band between them rather than a hard edge.
        assert!(dusk.count() > 20);
    }

    #[test]
    fn the_map_is_drawn_twice_over_and_blended() {
        let mut r = start(StartArgs::new(640, 480, "mode=globe&stars=false", 20260812));
        r.step();
        let f = r.frame();
        let textured: Vec<_> = f.batches.iter().filter(|b| b.texture.is_some()).collect();
        assert_eq!(textured.len(), 2, "day and night");
        assert_eq!(textured[0].blend, Blend::Off, "the day side is opaque");
        assert_eq!(textured[1].blend, Blend::Alpha, "the night side blends");
        assert_ne!(textured[0].texture, textured[1].texture);
        // The night pass really does vary its alpha rather than being flat.
        let b = textured[1];
        let alphas: Vec<f32> = f.vertices[b.first..b.first + b.count]
            .iter()
            .map(|v| v.color[3])
            .collect();
        let lo = alphas.iter().copied().fold(f32::MAX, f32::min);
        let hi = alphas.iter().copied().fold(f32::MIN, f32::max);
        assert!(lo < 0.01 && hi > 0.99, "{lo} to {hi}");
    }

    #[test]
    fn the_flat_modes_roll_the_map_round_a_cylinder() {
        for mode in ["mercator", "equirectangular"] {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("mode={mode}&stars=false"),
                20260812,
            ));
            r.step();
            let f = r.frame();
            assert!(!f.batches.is_empty());
            // A cylinder: every vertex the same distance from the axis, and
            // taller than it is wide is not it.
            let b = f.batches.iter().find(|b| b.texture.is_some()).unwrap();
            let mut lo = f32::MAX;
            let mut hi = 0.0f32;
            for v in &f.vertices[b.first..b.first + b.count] {
                let d = (v.pos[0] * v.pos[0] + v.pos[2] * v.pos[2]).sqrt();
                lo = lo.min(d);
                hi = hi.max(d);
            }
            assert!((hi - lo).abs() < 1e-4, "{mode}: {lo} to {hi}");
            assert!(hi > 0.5, "{mode}: the cylinder has no radius");
        }
    }

    #[test]
    fn the_grid_and_the_time_zones_come_up_and_go_away_again() {
        let mut r = start(StartArgs::new(640, 480, "mode=globe&stars=false", 20260812));
        let mut with = 0;
        let mut without = 0;
        for _ in 0..3000 {
            r.step();
            let lines = r
                .frame()
                .batches
                .iter()
                .any(|b| b.primitive == crate::runtime::gl::Primitive::Lines);
            if lines {
                with += 1;
            } else {
                without += 1;
            }
        }
        assert!(with > 0, "the grid never came up");
        assert!(without > 0, "the grid never went away");
    }
}
