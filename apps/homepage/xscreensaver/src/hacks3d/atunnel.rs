//! Port of `hacks/glx/atunnel.c` and `tunnel_draw.c`.
//!
//! ```text
//! atunnels --- OpenGL Advanced Tunnel Demo
//!
//! Copyright (c) E. Lassauge, 2002-2004.
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
//! The original code for this mode was written by Roman Podobedov
//! Email: romka@ut.ee
//! WEB: http://romka.demonews.com
//!
//! Eric Lassauge  (May-25-2004) <lassauge@users.sourceforge.net>
//! ```
//!
//! Zooming through a textured tunnel.
//!
//! The tunnel is a Catmull-Rom spline through eighteen fixed points, and its
//! wall is built a ring at a time: take a point a quarter above the curve, spin
//! it ten times about the curve's own tangent, and join each ring to the one
//! before it. Only three curve segments are ever drawn, just in front of the
//! camera, so the tunnel is made and thrown away as it is flown through.
//!
//! The camera rides the same spline, looking at where it will be a step later,
//! and rolls a degree a frame, which is what makes it feel like flying rather
//! than being pushed.
//!
//! At the far end there is nowhere left to build, so the screen flashes white
//! and the ride starts again at the beginning with the next texture.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, random};

/// How many texture names upstream reserves. Only six are ever filled; see
/// [`AtunnelState::current_texture`].
const MAX_TEXTURE: usize = 10;

const TEXTURES: [&[u8]; 6] = [
    crate::images::TUNNEL0,
    crate::images::TUNNEL1,
    crate::images::TUNNEL2,
    crate::images::TUNNEL3,
    crate::images::TUNNEL4,
    crate::images::TUNNEL5,
];

/// The spine of the tunnel. Upstream ends the list with a sentinel point of
/// all minus ones; here the list is just the points.
const INITPATH: [[f32; 3]; 18] = [
    [0.0, 0.0, 0.0],
    [2.0, 1.0, 0.0],
    [4.0, 0.0, 0.0],
    [6.0, 1.0, 0.0],
    [8.0, 0.0, 1.0],
    [10.0, 1.0, 1.0],
    [12.0, 1.5, 0.0],
    [14.0, 0.0, 0.0],
    [16.0, 1.0, 0.0],
    [18.0, 0.0, 0.0],
    [20.0, 0.0, 1.0],
    [22.0, 1.0, 0.0],
    [24.0, 0.0, 1.0],
    [26.0, 0.0, 1.0],
    [28.0, 1.0, 0.0],
    [30.0, 0.0, 2.0],
    [32.0, 1.0, 0.0],
    [34.0, 0.0, 2.0],
];

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / d, v[1] / d, v[2] / d]
}

/// `cvCatmullRom`: where the curve through these four points is at `t`.
fn catmull_rom(p: &[[f32; 3]; 4], t: f32) -> [f32; 3] {
    let t2 = t * t;
    let t3 = t * t * t;
    let t1 = (1.0 - t) * (1.0 - t);
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        *o = (-t * t1 * p[0][k]
            + (2.0 - 5.0 * t2 + 3.0 * t3) * p[1][k]
            + t * (1.0 + 4.0 * t - 3.0 * t2) * p[2][k]
            - t2 * (1.0 - t) * p[3][k])
            / 2.0;
    }
    out
}

/// `RotateAroundLine`: `p` turned by `a` radians about the line through `pp`
/// along the unit vector `pl`.
fn rotate_around_line(p: [f32; 3], pp: [f32; 3], pl: [f32; 3], a: f32) -> [f32; 3] {
    let p1 = [p[0] - pp[0], p[1] - pp[1], p[2] - pp[2]];
    let (l, m, n) = (pl[0], pl[1], pl[2]);
    let (ca, sa) = (a.cos(), a.sin());

    let p2 = [
        p1[0] * (l * l + ca * (1.0 - l * l))
            + p1[1] * (l * (1.0 - ca) * m + n * sa)
            + p1[2] * (l * (1.0 - ca) * n - m * sa),
        p1[0] * (l * (1.0 - ca) * m - n * sa)
            + p1[1] * (m * m + ca * (1.0 - m * m))
            + p1[2] * (m * (1.0 - ca) * n + l * sa),
        p1[0] * (l * (1.0 - ca) * n + m * sa)
            + p1[1] * (m * (1.0 - ca) * n - l * sa)
            + p1[2] * (n * n + ca * (1.0 - n * n)),
    ];
    [p2[0] + pp[0], p2[1] + pp[1], p2[2] + pp[2]]
}

struct AtunnelState {
    /// How far along the segment starting at `cam_pos` the camera is.
    cam_t: f32,
    cam_pos: usize,
    /// The camera's roll, a degree a frame.
    alpha: f32,

    prev_points: [[f32; 3]; 10],
    /// Which texture the tunnel is wearing. Upstream reserves ten names and
    /// fills six of them, and steps through all ten, so four rides in every
    /// ten are flown with no texture at all. That is upstream's arithmetic and
    /// it is kept: the untextured ride is part of what the saver looks like.
    current_texture: usize,

    /// How much of the white flash is left, and whether the reset that goes
    /// with it has already been done.
    mode_x: f32,
    mode_x_flag: bool,

    do_light: bool,
    do_texture: bool,
    do_wire: bool,
    textures: Vec<Option<u32>>,
}

impl AtunnelState {
    /// Four consecutive path points from `i`, or nothing if the tunnel has run
    /// out of spine to build on.
    fn curve(&self, i: usize) -> Option<[[f32; 3]; 4]> {
        if i + 3 >= INITPATH.len() {
            return None;
        }
        Some([
            INITPATH[i],
            INITPATH[i + 1],
            INITPATH[i + 2],
            INITPATH[i + 3],
        ])
    }

    fn end_of_tunnel(&mut self) {
        self.mode_x = 1.0;
        self.mode_x_flag = false;
    }

    /// `atunnel_DrawTunnel`.
    fn draw_tunnel(&mut self, g: &mut Gl) {
        if self.do_texture
            && let Some(Some(t)) = self.textures.get(self.current_texture)
        {
            g.glx.texturing(true);
            g.glx.bind_texture(*t);
        } else {
            g.glx.texturing(false);
        }

        let mut cmpos = self.cam_pos;
        let Some(p4) = self.curve(self.cam_pos) else {
            self.end_of_tunnel();
            return;
        };
        let op = catmull_rom(&p4, self.cam_t);

        self.cam_t += 0.02;
        if self.cam_t >= 1.0 {
            self.cam_t -= 1.0;
            cmpos = self.cam_pos + 1;
        }

        let Some(p4) = self.curve(cmpos) else {
            self.end_of_tunnel();
            return;
        };
        let op1 = catmull_rom(&p4, self.cam_t);

        g.glx.rotate(self.alpha, 0.0, 0.0, -1.0);
        self.alpha += 1.0;
        g.glx.look_at(op, op1, [0.0, 1.0, 0.0]);

        if self.do_light {
            g.glx.light_position(0, op[0], op[1], op[2], 1.0);
        }

        // Build the wall of the next three segments, a ring of ten every tenth
        // of a segment, joining each ring to the one before.
        let mut p = self.cam_pos;
        let mut flag = false;
        let mut t = 0.0f32;
        let mut k = 0;

        g.glx.begin(Shape::Quads);
        while k < 3 {
            let Some(p4) = self.curve(p) else {
                g.glx.end();
                self.end_of_tunnel();
                return;
            };
            let op = catmull_rom(&p4, t);
            let ppp = [op[0], op[1], op[2] + 0.25];

            t += 0.1;
            if t >= 1.0 {
                t -= 1.0;
                k += 1;
                p += 1;
            }

            let Some(p4) = self.curve(p) else {
                g.glx.end();
                self.end_of_tunnel();
                return;
            };
            let op1 = catmull_rom(&p4, t);

            // The tangent, which the ring is spun about.
            let tangent = normalize([op1[0] - op[0], op1[1] - op[1], op1[2] - op[2]]);

            let mut points = [[0.0f32; 3]; 10];
            for (i, point) in points.iter_mut().enumerate() {
                *point = rotate_around_line(
                    ppp,
                    op,
                    tangent,
                    i as f32 * 36.0 * std::f32::consts::PI / 180.0,
                );
                if !flag {
                    self.prev_points[i] = *point;
                }
            }

            // The first ring has nothing to join to.
            if !flag {
                flag = true;
                continue;
            }

            for i in 0..10 {
                let j = if i + 1 > 9 { 0 } else { i + 1 };
                for (uv, v) in [
                    ([0.0, 0.0], self.prev_points[i]),
                    ([1.0, 0.0], points[i]),
                    ([1.0, 1.0], points[j]),
                    ([0.0, 1.0], self.prev_points[j]),
                ] {
                    g.glx.normal3f(0.0, 0.0, 1.0);
                    g.glx.tex_coord2f(uv[0], uv[1]);
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
            }
            self.prev_points = points;
        }
        g.glx.end();
        self.cam_pos = cmpos;
    }

    /// `atunnel_SplashScreen`: a white sheet over everything, fading out over
    /// twenty frames, which covers the jump back to the start of the path.
    fn splash_screen(&mut self, g: &mut Gl) {
        if self.mode_x <= 0.0 {
            return;
        }
        if !self.mode_x_flag {
            self.cam_pos = 0;
            self.cam_t = 0.0;
            self.mode_x_flag = true;
            self.current_texture = (self.current_texture + 1) % MAX_TEXTURE;
        }

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.depth_test(false);
        g.glx.lighting(false);
        g.glx.fog(None);
        g.glx.cull_face(false);

        // `GL_SRC_ALPHA, GL_DST_ALPHA` upstream. The drawing buffer here has no
        // alpha channel, so its alpha reads as one and that is the same
        // arithmetic as adding the source scaled by its own alpha.
        g.glx.blend(Blend::AlphaAdd);
        g.glx.texturing(false);
        g.glx.color4f(1.0, 1.0, 1.0, self.mode_x);

        g.glx.begin(Shape::Quads);
        for v in [
            [-10.0, -10.0, -1.0],
            [10.0, -10.0, -1.0],
            [10.0, 10.0, -1.0],
            [-10.0, 10.0, -1.0],
        ] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();

        self.mode_x -= 0.05;
        if self.mode_x <= 0.0 {
            self.mode_x = 0.0;
        }

        if !self.do_wire {
            g.glx.cull_face(true);
            g.glx.depth_test(true);
        }
        if self.do_light {
            g.glx.lighting(true);
            g.glx.fog(Some(Fog::Exp {
                density: 0.3,
                color: [0.8, 0.8, 0.8, 1.0],
            }));
        }
        g.glx.blend(Blend::Off);
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
    }
}

impl Hack3d for AtunnelState {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        self.draw_tunnel(g);
        self.splash_screen(g);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = f64::from(height) / f64::from(width.max(1));
        let mut y = 0;
        if width > height * 2 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
            h = f64::from(height) / f64::from(width);
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        let w = (0.1 / h) as f32;
        g.glx.frustum(-w, w, -0.1, 0.1, 0.1, 10.0);
        g.glx.matrix_mode_modelview();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let do_wire = g.res.bool("wireframe");
    let do_light = g.res.bool("light");
    let do_texture = g.res.bool("texture");

    let mut textures = Vec::new();
    if do_texture {
        // Ten names, six pictures: the four spare names have no image, and a
        // ride that lands on one is simply untextured.
        for i in 0..MAX_TEXTURE {
            let id = g.glx.gen_texture();
            match TEXTURES
                .get(i)
                .and_then(|b| crate::runtime::png::decode_rgba(b))
            {
                Some((w, h, px)) => {
                    g.glx.bind_texture(id);
                    g.glx.tex_image_2d(w, h, px);
                    g.glx.tex_clamp(false);
                    g.glx.tex_nearest(false);
                    textures.push(Some(id));
                }
                None => textures.push(None),
            }
        }
    }

    let mut st = AtunnelState {
        cam_t: 0.0,
        cam_pos: 0,
        alpha: 0.0,
        prev_points: [[0.0; 3]; 10],
        current_texture: random() as usize % MAX_TEXTURE,
        mode_x: 0.0,
        mode_x_flag: false,
        do_light,
        do_texture,
        do_wire,
        textures,
    };

    if do_light {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        // A white ambient as well as a white diffuse, so the wall is lit from
        // everywhere and the light only picks out which way it faces.
        g.glx.light_ambient(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(0, 0.0, 0.0, 1.0, 0.0);
    }

    if do_wire {
        g.glx.cull_face(false);
        g.glx.depth_test(false);
    } else {
        g.glx.depth_test(true);
        g.glx.fog(Some(Fog::Exp {
            density: 0.3,
            color: [0.8, 0.8, 0.8, 1.0],
        }));
        // `glCullFace (GL_FRONT)`: the camera is *inside* the tunnel, so it is
        // the faces pointing at it that have to go. The runtime culls the back,
        // so the winding is flipped instead, which removes exactly the same
        // triangles.
        g.glx.front_face_cw(true);
        g.glx.cull_face(true);
    }

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     10000",
    "*showFPS:   False",
    "*suppressRotationAnimation: True",
    "*light:     True",
    "*wireframe: False",
    "*texture:   True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::boolean("texture", "Textured", "true"),
    Opt::boolean("light", "Lighting", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "atunnel",
    label: "Atunnel",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Eric Lassauge and Roman Podobedov",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=mpCRbi3jkuc"),
        blurb: "Zooming through a textured tunnel.",
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

    /// A Catmull-Rom curve passes through its two middle control points and
    /// nowhere near the outer two, which is what makes it a path through the
    /// points rather than towards them.
    #[test]
    fn the_curve_goes_through_its_middle_points() {
        let p = [
            [0.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [4.0, 0.0, 0.0],
            [6.0, 1.0, 0.0],
        ];
        let start = catmull_rom(&p, 0.0);
        let end = catmull_rom(&p, 1.0);
        for k in 0..3 {
            assert!((start[k] - p[1][k]).abs() < 1e-5, "t=0 is not p1");
            assert!((end[k] - p[2][k]).abs() < 1e-5, "t=1 is not p2");
        }
        // And in between it leaves the chord, which is what the outer two
        // points are for. Not measured at the halfway mark: these four points
        // are symmetric about it, so the curve crosses the chord there.
        let quarter = catmull_rom(&p, 0.25);
        let chord = p[1][1] + 0.25 * (p[2][1] - p[1][1]);
        assert!(
            (quarter[1] - chord).abs() > 0.05,
            "the curve is a straight line: {} against {chord}",
            quarter[1]
        );
    }

    /// Spinning a point about a line keeps it the same distance from that line,
    /// and a whole turn brings it back.
    #[test]
    fn a_spin_about_a_line_is_a_circle() {
        let axis = normalize([1.0, 2.0, -0.5]);
        let on_axis = [3.0, 1.0, 2.0];
        let p = [3.25, 1.5, 2.0];

        let radius = |q: [f32; 3]| {
            let d = [q[0] - on_axis[0], q[1] - on_axis[1], q[2] - on_axis[2]];
            let along = d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2];
            let perp = [
                d[0] - along * axis[0],
                d[1] - along * axis[1],
                d[2] - along * axis[2],
            ];
            (perp[0] * perp[0] + perp[1] * perp[1] + perp[2] * perp[2]).sqrt()
        };

        let r0 = radius(p);
        for i in 0..10 {
            let a = i as f32 * 36.0 * std::f32::consts::PI / 180.0;
            let q = rotate_around_line(p, on_axis, axis, a);
            assert!((radius(q) - r0).abs() < 1e-4, "step {i} left the circle");
        }
        let round = rotate_around_line(p, on_axis, axis, std::f32::consts::PI * 2.0);
        for k in 0..3 {
            assert!((round[k] - p[k]).abs() < 1e-4, "a whole turn moved it");
        }
    }

    /// The wall is rings of ten joined to the ring before, three segments'
    /// worth at a time.
    #[test]
    fn the_wall_is_rings_of_ten() {
        let r = run("", 5);
        let f = r.frame();
        let wall: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles && b.count > 60)
            .collect();
        assert_eq!(wall.len(), 1, "the wall should be one run of quads");

        // Thirty rings' worth of joins, ten quads each, six vertices a quad,
        // less the first ring which has nothing to join to.
        let quads = wall[0].count / 6;
        assert_eq!(quads % 10, 0, "{quads} quads is not whole rings");
        assert!(
            (250..=300).contains(&quads),
            "{quads} quads is not three segments"
        );
    }

    /// The camera flies down the tunnel and rolls as it goes, so no two frames
    /// look out from the same place.
    #[test]
    fn the_camera_flies_and_rolls() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..100 {
            r.step();
            seen.insert(r.frame().batches[0].modelview.0.map(f32::to_bits));
        }
        assert_eq!(seen.len(), 100, "the camera stood still");
    }

    /// At the end of the path there is nothing left to build, so the screen
    /// flashes white and the ride begins again with the next texture.
    #[test]
    fn the_ride_restarts_with_a_flash() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut flashes = 0;
        let mut textures = std::collections::BTreeSet::new();
        for _ in 0..3000 {
            r.step();
            let f = r.frame();
            // The flash is one quad drawn unlit and white over everything.
            if f.batches
                .iter()
                .any(|b| b.count == 6 && !b.lighting && f.vertices[b.first].color[3] > 0.0)
            {
                flashes += 1;
            }
            if let Some(t) = f.batches.first().and_then(|b| b.texture) {
                textures.insert(t);
            }
        }
        assert!(
            flashes > 20,
            "only {flashes} frames of flash in three thousand"
        );
        assert!(
            textures.len() > 1,
            "the ride never changed texture: {textures:?}"
        );
    }

    /// Six of the ten texture names carry a picture. The other four are
    /// upstream's oversight and are kept, so a ride can be untextured.
    #[test]
    fn six_of_ten_textures_are_real() {
        let r = run("", 1);
        let mut real = 0;
        for id in 1..=MAX_TEXTURE as u32 {
            if let Some(t) = r.texture(id) {
                assert!(t.width >= 64 && t.height >= 64);
                real += 1;
            }
        }
        assert_eq!(real, 6);
    }
}
