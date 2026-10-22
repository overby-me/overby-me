//! Port of `hacks/glx/blocktube.c`.
//!
//! ```text
//! blocktube.c, Copyright (c) 2003 Lars R. Damerow <lars@oddment.org>
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
//! A swirling, falling tunnel of reflective slabs. They fade from hue to hue.
//!
//! A thousand long thin blocks are scattered round the wall of a tube two
//! hundred units deep, each turned to its own angle about the axis and drifting
//! towards the camera; one that reaches it is put back at the far end. The
//! whole tunnel is one colour at a time, which walks to a new random one over
//! `changetime` frames, sits there for `holdtime`, and walks again; a block's
//! own shade is that colour scaled by a fixed fraction it was given at birth,
//! so the tube is one hue in many depths.
//!
//! What makes the slabs look like polished metal is not lighting: it is a
//! photograph of a lit room used as a sphere map, so where a face reads from
//! the texture depends only on which way it faces. Turn the texture off and
//! there is nothing to see, which is why upstream switches a light on instead
//! in that case.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Fog, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, frand, random};

const MAX_ENTITIES: usize = 1000;

/// One slab: where it sits round the tube, how fast it turns, and how bright a
/// share of the tunnel's colour it gets.
#[derive(Clone, Copy, Default)]
struct Entity {
    /// A fraction of the tunnel colour, between two thirds and one.
    t_val: f32,
    angle: f32,
    angular_velocity: f32,
    position: [f32; 3],
}

struct BlockTube {
    block_dlist: u32,
    entities: Vec<Entity>,

    target: [f32; 3],
    current: [f32; 3],
    delta: [f32; 3],
    counter: i32,
    changing: bool,

    zoom: f32,
    tilt: f32,
    tunnel_length: f32,
    tunnel_width: f32,

    hold_time: i32,
    change_time: i32,
    texture: Option<u32>,
}

impl BlockTube {
    /// A new colour to walk to, rejected until it is bright enough to see.
    fn new_target_color(&mut self) {
        let mut luminance = 0.0f32;
        while luminance <= 150.0 {
            for i in 0..3 {
                self.target[i] = (random() % 256) as f32;
                self.delta[i] = (self.target[i] - self.current[i]) / self.change_time as f32;
            }
            luminance = 0.3 * self.target[0] + 0.59 * self.target[1] + 0.11 * self.target[2];
        }
    }

    fn randomize_entity(&self, ent: &mut Entity) {
        ent.t_val = 1.0 - (frand(1.0) / 1.5) as f32;
        ent.angle = (random() % 360) as f32;
        ent.angular_velocity = 0.5 - frand(1.0) as f32;
        ent.position = [
            frand(1.0) as f32 + self.tunnel_width,
            frand(1.0) as f32 * 2.0,
            -frand(1.0) as f32 * self.tunnel_length,
        ];
    }

    /// Turn a little, come a little closer, and go back to the far end on
    /// arrival. Upstream also ages every block by a tenth of a frame into an
    /// integer, which is to say not at all, and nothing reads the age anyway.
    fn entity_tick(&self, ent: &mut Entity) {
        ent.angle += ent.angular_velocity;
        ent.position[2] += 0.1;
        if ent.position[2] > self.zoom {
            ent.position[2] = -self.tunnel_length + frand(1.0) as f32 * 20.0;
        }
    }

    /// The colour clock: walk for `changetime`, sit for `holdtime`, repeat.
    fn tick(&mut self) {
        self.counter -= 1;
        if self.counter == 0 {
            if self.changing {
                self.counter = self.hold_time;
            } else {
                self.new_target_color();
                self.counter = self.change_time;
            }
            self.changing = !self.changing;
        } else if self.changing {
            for i in 0..3 {
                self.current[i] += self.delta[i];
            }
        }
    }

    /// One slab, as six faces with texture coordinates that the sphere map then
    /// ignores. They are upstream's and are what the untextured build would
    /// use, so they stay.
    fn cube_vertices(g: &mut Gl, x: f32, y: f32, z: f32, wire: bool) {
        let (x2, y2, z2) = (x / 2.0, y / 2.0, z / 2.0);
        let nv = 0.7;
        let shape = if wire { Shape::LineLoop } else { Shape::Quads };

        g.glx.front_face_cw(true);

        /// A face: which way it points, and its four corners with the texture
        /// coordinate each one carries.
        type Face = ([f32; 3], [([f32; 2], [f32; 3]); 4]);

        let faces: [Face; 6] = [
            (
                [0.0, 0.0, nv],
                [
                    ([0.0, 0.0], [-x2, y2, z2]),
                    ([1.0, 0.0], [x2, y2, z2]),
                    ([1.0, 1.0], [x2, -y2, z2]),
                    ([0.0, 1.0], [-x2, -y2, z2]),
                ],
            ),
            (
                [0.0, 0.0, -nv],
                [
                    ([1.0, 0.0], [-x2, -y2, -z2]),
                    ([1.0, 1.0], [x2, -y2, -z2]),
                    ([0.0, 1.0], [x2, y2, -z2]),
                    ([0.0, 0.0], [-x2, y2, -z2]),
                ],
            ),
            (
                [0.0, nv, 0.0],
                [
                    ([0.0, 1.0], [-x2, y2, -z2]),
                    ([0.0, 0.0], [x2, y2, -z2]),
                    ([1.0, 0.0], [x2, y2, z2]),
                    ([1.0, 1.0], [-x2, y2, z2]),
                ],
            ),
            (
                [0.0, -nv, 0.0],
                [
                    ([1.0, 1.0], [-x2, -y2, -z2]),
                    ([0.0, 1.0], [-x2, -y2, z2]),
                    ([0.0, 0.0], [x2, -y2, z2]),
                    ([1.0, 0.0], [x2, -y2, -z2]),
                ],
            ),
            (
                [nv, 0.0, 0.0],
                [
                    ([1.0, 0.0], [x2, -y2, -z2]),
                    ([1.0, 1.0], [x2, -y2, z2]),
                    ([0.0, 1.0], [x2, y2, z2]),
                    ([0.0, 0.0], [x2, y2, -z2]),
                ],
            ),
            (
                [-nv, 0.0, 0.0],
                [
                    ([0.0, 0.0], [-x2, -y2, -z2]),
                    ([1.0, 0.0], [-x2, y2, -z2]),
                    ([1.0, 1.0], [-x2, y2, z2]),
                    ([0.0, 1.0], [-x2, -y2, z2]),
                ],
            ),
        ];

        for (i, (n, corners)) in faces.into_iter().enumerate() {
            // Wireframe draws the four sides and stops: upstream returns before
            // the two ends.
            if wire && i >= 4 {
                break;
            }
            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx.begin(shape);
            for (uv, p) in corners {
                g.glx.tex_coord2f(uv[0], uv[1]);
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
        }
    }
}

impl Hack3d for BlockTube {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if let Some(t) = self.texture {
            g.glx.tex_gen_sphere(true);
            g.glx.texturing(true);
            g.glx.bind_texture(t);
        }

        let (zoom, tilt) = (self.zoom, self.tilt);
        for i in 0..self.entities.len() {
            let ent = self.entities[i];
            g.glx.matrix_mode_modelview();
            g.glx.load_identity();
            g.glx.translate(0.0, 0.0, zoom);
            g.glx.rotate(tilt, 1.0, 0.0, 0.0);
            g.glx.rotate(ent.angle, 0.0, 0.0, 1.0);
            g.glx
                .translate(ent.position[0], ent.position[1], ent.position[2]);
            g.glx.color4f(
                (self.current[0] * ent.t_val) as i32 as f32 / 255.0,
                (self.current[1] * ent.t_val) as i32 as f32 / 255.0,
                (self.current[2] * ent.t_val) as i32 as f32 / 255.0,
                1.0,
            );
            g.glx.call_list(self.block_dlist);

            let mut ent = ent;
            self.entity_tick(&mut ent);
            self.entities[i] = ent;
        }
        self.tick();

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
        g.glx.perspective(45.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();

        // Upstream scales the modelview here to keep a portrait window from
        // cropping. It has no effect: every frame loads the identity over it
        // before drawing anything, so the scale is gone by the time it would
        // have mattered. Kept as it is rather than fixed.
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mut do_texture = g.res.bool("texture");
    let mut do_fog = g.res.bool("fog");
    if wire {
        do_fog = false;
        do_texture = false;
    }

    let mut st = BlockTube {
        block_dlist: 0,
        entities: vec![Entity::default(); MAX_ENTITIES],
        target: [0.0; 3],
        current: [0.0; 3],
        delta: [0.0; 3],
        counter: 0,
        changing: false,
        zoom: 30.0,
        tilt: 4.5,
        tunnel_length: 200.0,
        tunnel_width: 5.0,
        hold_time: g.res.int("holdtime").max(1),
        change_time: g.res.int("changetime").max(1),
        texture: None,
    };

    st.block_dlist = g.glx.gen_lists(1);
    g.glx.new_list(st.block_dlist);
    BlockTube::cube_vertices(g, 0.15, 1.2, 5.25, wire);
    g.glx.end_list();

    if do_texture
        && let Some((w, h, px)) = crate::runtime::png::decode_rgba(crate::images::BLOCKTUBE)
    {
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(w, h, px);
        st.texture = Some(id);
    }

    if do_fog {
        g.glx.fog(Some(Fog::Linear {
            start: 0.0,
            end: st.tunnel_length / 1.8,
            color: [0.0, 0.0, 0.0, 1.0],
        }));
    }
    g.glx.depth_test(true);
    g.glx.cull_face(true);

    if st.texture.is_none() && !wire {
        // With no texture the slabs do not show up at all, so light them.
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 0.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
    }

    st.counter = st.hold_time;
    for i in 0..3 {
        st.current[i] = (random() % 256) as f32;
    }
    st.new_target_color();
    for i in 0..MAX_ENTITIES {
        let mut e = st.entities[i];
        st.randomize_entity(&mut e);
        st.entities[i] = e;
    }

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      40000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*holdtime:   1000",
    "*changetime: 200",
    "*texture:    True",
    "*fog:        True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "40000").inverted(),
    Opt::slider("holdtime", "Color hold time", 10.0, 2000.0, 10.0, 0, "1000"),
    Opt::slider(
        "changetime",
        "Color change time",
        10.0,
        1000.0,
        10.0,
        0,
        "200",
    ),
    Opt::boolean("texture", "Textured", "true"),
    Opt::boolean("fog", "Fog", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "blocktube",
    label: "Block Tube",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Lars R. Damerow",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=L0JUBhpZlMw"),
        blurb: "A swirling, falling tunnel of reflective slabs. \
                They fade from hue to hue.",
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

    /// The sphere map is a real picture, and it is what the slabs are drawn
    /// with rather than the texture coordinates they carry.
    #[test]
    fn the_slabs_are_sphere_mapped() {
        let r = run("", 2);
        let f = r.frame();
        let t = f.batches[0].texture.expect("no texture bound");
        assert!(
            f.batches
                .iter()
                .all(|b| b.texture == Some(t) && b.tex_gen_sphere),
            "something was drawn without the sphere map"
        );

        let image = r.texture(t).expect("the texture has no image");
        assert_eq!((image.width, image.height), (256, 256));
        assert_eq!(image.data.len(), 256 * 256 * 4);
        assert!(
            image.data.iter().any(|v| *v > 200),
            "the sphere map came out black"
        );
    }

    /// Turning the texture off puts a light on instead, because unlit
    /// untextured slabs are invisible.
    #[test]
    fn without_the_texture_there_is_a_light() {
        let r = run("texture=false", 2);
        let f = r.frame();
        assert!(f.batches.iter().all(|b| b.texture.is_none()));
        assert!(f.batches.iter().all(|b| b.lighting), "nothing was lit");
    }

    /// A thousand slabs, each one solid cube of six faces folded into a single
    /// batch because nothing but the matrix changes between its faces.
    #[test]
    fn a_thousand_slabs() {
        let r = run("", 2);
        let f = r.frame();
        assert_eq!(f.batches.len(), MAX_ENTITIES);
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == Primitive::Triangles && b.count == 6 * 6),
            "a slab is not six quads"
        );
    }

    /// Nothing escapes the tube: a slab that reaches the camera goes back to
    /// the far end, so the tunnel never empties.
    #[test]
    fn the_tunnel_refills_itself() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..1200 {
            r.step();
            let f = r.frame();
            assert_eq!(f.batches.len(), MAX_ENTITIES);
            // Every slab is somewhere between the far end of the tube and
            // just past the camera. The bounds are the tube itself, two
            // hundred deep and thirty in front, give or take the four and a
            // half degrees of tilt tipping a little of the radius into z.
            for b in &f.batches {
                let z = b.modelview.0[14];
                assert!(
                    (-175.0..=62.0).contains(&z),
                    "a slab has got to z = {z}, outside the tunnel"
                );
            }
        }
    }

    /// The tunnel walks to a new colour over `changetime` frames, holds it for
    /// `holdtime`, and starts again.
    #[test]
    fn the_colour_walks_then_holds() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "holdtime=20&changetime=10",
            20260811,
        ));
        let mut colours = Vec::new();
        for _ in 0..200 {
            r.step();
            let f = r.frame();
            colours.push(f.vertices[f.batches[0].first].color);
        }

        // Somewhere in there it moved, and somewhere in there it sat still.
        let moves = colours.windows(2).filter(|w| w[0] != w[1]).count();
        let holds = colours.windows(2).filter(|w| w[0] == w[1]).count();
        assert!(moves > 10, "the colour never changed");
        assert!(holds > 10, "the colour never held");

        // And the brightness it walks to is one worth looking at.
        let luminance = |c: [f32; 4]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
        let brightest = colours
            .iter()
            .copied()
            .fold(0.0f32, |m, c| m.max(luminance(c)));
        assert!(brightest > 0.2, "every colour it picked was near black");
    }
}
