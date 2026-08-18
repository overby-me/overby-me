//! Port of `hacks/glx/boing.c`.
//!
//! ```text
//! boing, Copyright (c) 2005-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! A clone of the Amiga 1000 "Boing" demo.  This was the first graphics demo
//! for the Amiga, written by Dale Luck and RJ Mical during a break at the 1984
//! Consumer Electronics Show (or so the legend goes.)  The boing ball was
//! briefly the official logo of Amiga Inc., until they were bought by
//! Commodore later that year.
//!
//! With no arguments, this program looks a lot like the original Amiga demo.
//! With "-smooth -lighting", it looks... less old.
//!
//! The amiga version made noise when the ball hit the walls.  This version
//! does not, obviously.
//! ```
//!
//! The checkerboard ball bouncing in the magenta grid corner. It is 1984, and
//! this is what a computer that could do this was worth queueing to see.
//!
//! Almost everything here is in service of looking like the original rather
//! than looking good. Lighting is off by default, so the red and white squares
//! are flat colour with no shading at all. The pixels are not square: the
//! projection multiplies the aspect by four thirds, because the machine it is
//! imitating could not afford square ones. The shadow is not cast, it is a flat
//! disc offset up and to the right by a fixed amount. And there are scanlines
//! drawn over the whole thing, because there would have been.
//!
//! The ball is not a sphere primitive but a grid of quads over latitude and
//! longitude, coloured by a checkerboard of the quad indices, which is why the
//! meridians and parallels knobs change the pattern and not just the smoothness.
//! Its normals point inward and it is wound clockwise, which is upstream's, and
//! comes out the same here because the shading is two-sided.
//!
//! One thing does not come across: line widths. The grid and the scanlines both
//! ask for lines a few pixels wide, and WebGL draws every line one pixel wide
//! whatever it is told. At the default thickness the grid asks for under two
//! pixels, so it is close; the scanlines are finer than they should be.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::unrgb;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};

struct Boing {
    trackball: Trackball,

    /// Upstream's `speed` divided by 800: everything below is in units of it.
    speed: f64,

    ball: [f64; 3],
    ball_th: f64,
    ball_d: [f64; 3],
    ball_dth: f64,
    ball_dd: [f64; 3],

    ball_color1: [f32; 4],
    ball_color2: [f32; 4],
    grid_color: [f32; 4],
    shadow_color: [f32; 4],
    lightpos: [f32; 4],

    lighting_p: bool,
    smooth_p: bool,
    scanlines_p: bool,
    angle: f32,
    ball_size: f32,
    meridians: i32,
    parallels: i32,
    tiles: i32,
    thickness: f32,
    wireframe: bool,
}

impl Boing {
    /// How much finer the sphere is subdivided than the checkerboard on it.
    fn scale(&self) -> i32 {
        let mut scale = if self.smooth_p { 5 } else { 1 };
        if self.parallels < 3 {
            scale *= 2;
        }
        scale
    }

    fn draw_grid(&self, g: &mut Gl) {
        let tiles = self.tiles;
        let t2 = tiles as f32 / 2.0;
        let s = 1.0 / (tiles as f32 + self.thickness);
        let z = 0.0;

        let lw = g.height() as f32 * 0.06 * self.thickness;

        g.glx.material_ambient_diffuse(self.grid_color);
        let c = self.grid_color;
        g.glx.color3f(c[0], c[1], c[2]);

        g.glx.push_matrix();
        g.glx.scale(s, s, s);
        g.glx.translate(-t2, -t2, 0.0);

        g.glx.line_width(lw);
        g.glx.begin(Shape::Lines);
        for y in 0..=tiles {
            g.glx.vertex3f(0.0, y as f32, z);
            g.glx.vertex3f(tiles as f32, y as f32, z);
        }
        for x in 0..=tiles {
            g.glx.vertex3f(x as f32, tiles as f32, z);
            g.glx.vertex3f(x as f32, 0.0, z);
        }
        g.glx.end();
        g.glx.pop_matrix();
    }

    /// The back wall and the floor: the same grid twice, the second one folded
    /// down. There is no third wall, and at this camera angle nobody notices.
    fn draw_box(&self, g: &mut Gl) {
        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -0.5);
        self.draw_grid(g);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, 0.0, 0.5);
        self.draw_grid(g);
        g.glx.pop_matrix();
    }

    fn draw_ball(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let scale = self.scale();
        let xx = self.meridians * scale;
        let yy = self.parallels * scale;

        if self.lighting_p && !wire {
            g.glx.lighting(true);
        }

        g.glx.front_face_cw(true);

        g.glx.push_matrix();
        g.glx.translate(
            self.ball[0] as f32,
            self.ball[1] as f32,
            self.ball[2] as f32,
        );
        g.glx.scale(self.ball_size, self.ball_size, self.ball_size);
        g.glx.rotate(-self.angle, 0.0, 0.0, 1.0);
        g.glx.rotate(self.ball_th as f32, 0.0, 1.0, 0.0);

        let tau = std::f64::consts::PI * 2.0;
        for y in 0..yy {
            let thy0 = f64::from(y) * tau / f64::from(yy * 2) + std::f64::consts::FRAC_PI_2;
            let thy1 = f64::from(y + 1) * tau / f64::from(yy * 2) + std::f64::consts::FRAC_PI_2;

            for x in 0..xx {
                let thx0 = f64::from(x) * tau / f64::from(xx);
                let thx1 = f64::from(x + 1) * tau / f64::from(xx);
                let bgp = ((x / scale) & 1) ^ ((y / scale) & 1) != 0;

                if wire && bgp {
                    continue;
                }

                let c = if bgp {
                    self.ball_color2
                } else {
                    self.ball_color1
                };
                g.glx.material_ambient_diffuse(c);
                g.glx.color3f(c[0], c[1], c[2]);

                g.glx
                    .begin(if wire { Shape::LineLoop } else { Shape::Quads });

                // The normals point inward, and the winding is clockwise to
                // match. Shading is two-sided, so it comes out the same as the
                // outward-facing version would.
                let point = |thy: f64, thx: f64| {
                    [
                        (thy.cos() * thx.cos() / 2.0) as f32,
                        (thy.sin() / 2.0) as f32,
                        (thy.cos() * thx.sin() / 2.0) as f32,
                    ]
                };
                if !self.smooth_p {
                    // The middle of the quad on the unit sphere: one normal
                    // for the whole facet, so it reads as a flat tile.
                    let (thy, thx) = ((thy0 + thy1) / 2.0, (thx0 + thx1) / 2.0);
                    g.glx.normal3f(
                        -(thy.cos() * thx.cos()) as f32,
                        -thy.sin() as f32,
                        -(thy.cos() * thx.sin()) as f32,
                    );
                }
                for (thy, thx) in [(thy0, thx0), (thy1, thx0), (thy1, thx1), (thy0, thx1)] {
                    let p = point(thy, thx);
                    if self.smooth_p {
                        g.glx.normal3f(-p[0], -p[1], -p[2]);
                    }
                    g.glx.vertex3f(p[0], p[1], p[2]);
                }
                g.glx.end();
            }
        }
        g.glx.pop_matrix();

        if self.lighting_p && !wire {
            g.glx.lighting(false);
        }
    }

    /// Not a cast shadow: a flat disc on the back wall, offset by a fixed
    /// amount, which is what the original did and is why it never quite lines
    /// up with where the light is.
    fn draw_shadow(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let xoff = 0.14;
        let yoff = 0.07;
        let yy = self.parallels * self.scale();

        g.glx.push_matrix();
        g.glx.translate(
            self.ball[0] as f32 + xoff,
            self.ball[1] as f32 + yoff,
            -0.49,
        );
        g.glx.scale(self.ball_size, self.ball_size, self.ball_size);
        g.glx.rotate(-self.angle, 0.0, 0.0, 1.0);

        g.glx.material_ambient_diffuse(self.shadow_color);
        let c = self.shadow_color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);

        g.glx.front_face_cw(false);
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.begin(if wire {
            Shape::LineLoop
        } else {
            Shape::TriangleFan
        });
        if !wire {
            g.glx.vertex3f(0.0, 0.0, 0.0);
        }
        for y in 0..yy * 2 + 1 {
            let thy0 = f64::from(y) * (std::f64::consts::PI * 2.0) / f64::from(yy * 2)
                + std::f64::consts::FRAC_PI_2;
            g.glx
                .vertex3f((thy0.cos() / 2.0) as f32, (thy0.sin() / 2.0) as f32, 0.0);
        }
        g.glx.end();
        g.glx.pop_matrix();
    }

    /// Because there would have been.
    fn draw_scanlines(&self, g: &mut Gl) {
        let (w, h) = (g.width(), g.height());
        if h <= 300 {
            return;
        }

        if !self.wireframe {
            g.glx.blend(Blend::Alpha);
            g.glx.depth_test(false);
        }

        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();
        // Upstream puts the orthographic projection on the modelview, which
        // comes to the same thing with the projection left as identity.
        g.glx.ortho(0.0, w as f32, 0.0, h as f32, -1.0, 1.0);

        let (lh, ls) = if h > 500 { (4, 4) } else { (2, 1) };
        if lh == 1 {
            g.glx.blend(Blend::Off);
        }
        g.glx.line_width(lh as f32);
        g.glx.color4f(0.0, 0.0, 0.0, 0.3);

        g.glx.begin(Shape::Lines);
        let mut y = 0;
        while y < h {
            g.glx.vertex3f(0.0, y as f32, 0.0);
            g.glx.vertex3f(w as f32, y as f32, 0.0);
            y += lh + ls;
        }
        g.glx.end();

        g.glx.pop_matrix();
        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();

        if !self.wireframe {
            g.glx.blend(Blend::Off);
            g.glx.depth_test(true);
        }
    }

    fn tick_physics(&mut self) {
        let s2 = self.ball_size / 2.0;
        let max = f64::from(0.5 - s2);
        let min = -max;

        self.ball_th += self.ball_dth;
        while self.ball_th > 360.0 {
            self.ball_th -= 360.0;
        }
        while self.ball_th < 0.0 {
            self.ball_th += 360.0;
        }

        // Sideways is the only axis that is not simply elastic: every wall it
        // hits reverses the spin as well, and adds a random nudge, so the
        // bounce never settles into a period.
        self.ball_d[0] += self.ball_dd[0];
        self.ball[0] += self.ball_d[0];
        if self.ball[0] < min || self.ball[0] > max {
            self.ball[0] = if self.ball[0] < min { min } else { max };
            self.ball_d[0] = -self.ball_d[0];
            self.ball_dth = -self.ball_dth;
            self.ball_d[0] += frand(self.speed / 2.0) - self.speed;
        }

        for k in 1..3 {
            self.ball_d[k] += self.ball_dd[k];
            self.ball[k] += self.ball_d[k];
            if self.ball[k] < min {
                self.ball[k] = min;
                self.ball_d[k] = -self.ball_d[k];
            } else if self.ball[k] > max {
                self.ball[k] = max;
                self.ball_d[k] = -self.ball_d[k];
            }
        }
    }
}

impl Hack3d for Boing {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if !self.trackball.button_down() {
            self.tick_physics();
        }

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let l = self.lightpos;
        g.glx.light_position(0, l[0], l[1], l[2], l[3]);

        g.glx.cull_face(false);
        g.glx.depth_test(false);
        g.glx.blend(Blend::Alpha);

        self.draw_box(g);
        self.draw_shadow(g);

        g.glx.cull_face(true);
        g.glx.depth_test(true);

        self.draw_ball(g);
        if self.scanlines_p {
            self.draw_scanlines(g);
        }

        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        /* Back in the caveman days we couldn't even afford square pixels! */
        let mut h = (height as f32 / width.max(1) as f32) * 4.0 / 3.0;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 3 / 4;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if height > width {
            let s = width as f32 / height as f32;
            g.glx.scale(s, s, s);
        }
        g.glx.perspective(8.0, 1.0 / h, 1.0, 10.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 8.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

/// A colour resource as the four floats the GL side wants.
fn color_of(g: &Gl, key: &str) -> [f32; 4] {
    let (r, gr, b) = unrgb(g.res.pixel(key));
    [
        f32::from(r) / 255.0,
        f32::from(gr) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    ]
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let smooth_p = g.res.bool("smooth");
    let lighting_p = g.res.bool("lighting");

    let tiles = g.res.int("tiles").max(1);

    let (mut meridians, mut parallels) = (g.res.int("meridians"), g.res.int("parallels"));
    if smooth_p {
        meridians = meridians.max(1);
        parallels = parallels.max(1);
    } else {
        meridians = meridians.max(3);
        parallels = parallels.max(2);
    }
    if meridians > 1 && meridians & 1 != 0 {
        meridians += 1; /* odd numbers look bad */
    }

    let thickness = g.res.float("thickness").clamp(0.001, 1.0) as f32;
    let ball_size = g.res.float("size") as f32;

    let mut shadow_color = color_of(g, "shadowColor");
    shadow_color[3] = 0.9;

    let bg = color_of(g, "boingBackground");
    g.glx.clear_color(bg[0], bg[1], bg[2], 1.0);

    let speed = g.res.float("speed") / 800.0;
    let spin = g.res.bool("spin");

    let mut st = Boing {
        trackball: Trackball::new(),
        speed,
        ball: [
            f64::from(0.5 - (ball_size / 2.0)) - frand(f64::from(1.0 - ball_size)),
            0.2,
            0.0,
        ],
        ball_th: 0.0,
        ball_d: [speed * 6.0 + frand(speed), 0.0, speed * 6.0 + frand(speed)],
        ball_dth: if spin { -speed * 7.0 * 360.0 } else { 0.0 },
        ball_dd: [0.0, -speed, 0.0],
        ball_color1: color_of(g, "ballColor1"),
        ball_color2: color_of(g, "ballColor2"),
        grid_color: color_of(g, "gridColor"),
        shadow_color,
        lightpos: [0.5, 0.5, -1.0, 0.0],
        lighting_p,
        smooth_p,
        scanlines_p: g.res.bool("scanlines"),
        angle: g.res.int("angle") as f32,
        ball_size,
        meridians,
        parallels,
        tiles,
        thickness,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if lighting_p && !wire {
        g.glx.light_enable(0, true);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*spin:         True",
    "*lighting:     False",
    "*smooth:       False",
    "*scanlines:    True",
    "*speed:        1.0",
    "*angle:        15",
    "*size:         0.5",
    "*meridians:    16",
    "*parallels:    8",
    "*tiles:        12",
    "*thickness:    0.05",
    "*ballColor1:   #CC1919",
    "*ballColor2:   #F2F2F2",
    "*gridColor:    #991999",
    "*shadowColor:  #303030",
    "*boingBackground: #8C8C8C",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("size", "Size", 0.02, 0.9, 0.01, 2, "0.5"),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::spin("meridians", "Meridians", 1.0, 90.0, "16"),
    Opt::spin("parallels", "Parallels", 1.0, 90.0, "8"),
    Opt::boolean("smooth", "Smoothing", "false"),
    Opt::boolean("lighting", "Lighting", "false"),
    Opt::boolean("scanlines", "Scanlines", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "boing",
    label: "Boing",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=J3KAsV31d6M"),
        blurb: "A clone of the first graphics demo for the Amiga 1000.",
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

    /// Where the ball is, in eye space. The ball is the only thing drawn as
    /// triangles: the grid and the scanlines are lines and the shadow is a
    /// fan, so the first triangle batch is one of its quads, and every quad
    /// hangs off the ball's own matrix.
    fn ball_at(r: &Runner3d) -> Option<[f32; 3]> {
        let f = r.frame();
        f.batches
            .iter()
            .find(|b| b.primitive == Primitive::Triangles)
            .map(|b| b.modelview.transform([0.0, 0.0, 0.0]))
    }

    /// The ball stays in the box. It is the whole of the physics, and getting
    /// the bounds wrong is a ball that leaves the picture and never returns.
    #[test]
    fn the_ball_stays_in_the_box() {
        let mut r = start(StartArgs::new(640, 480, "speed=10", 20260811));
        for _ in 0..2000 {
            r.step();
            let Some(o) = ball_at(&r) else { continue };
            // In eye space, with the camera eight away looking down -z. Half a
            // unit each way plus the ball, comfortably inside this.
            assert!(
                o[0].abs() < 1.0 && o[1].abs() < 1.0,
                "the ball escaped: {o:?}"
            );
        }
    }

    /// And it does bounce: it has to reach both the top and the bottom of the
    /// box, or the gravity and the walls are not doing anything.
    #[test]
    fn the_ball_bounces() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for _ in 0..2000 {
            r.step();
            let Some(o) = ball_at(&r) else { continue };
            lo = lo.min(o[1]);
            hi = hi.max(o[1]);
        }
        assert!(hi - lo > 0.2, "it only moved from {lo} to {hi}");
    }

    /// The checkerboard is a checkerboard: the two colours alternate along
    /// both axes, so the number of quads of each is equal when the counts are
    /// even.
    #[test]
    fn the_ball_is_a_checkerboard() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "meridians=16&parallels=8",
            20260811,
        ));
        r.step();
        let f = r.frame();
        // Counted in vertices rather than batches: two quads of the same
        // colour that happen to be adjacent are folded into one batch, which
        // happens at the seam of every second row.
        let mut reds = 0;
        let mut whites = 0;
        for b in f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
        {
            let c = b.material.ambient_diffuse;
            if (c[0] - 0.8).abs() < 0.02 && c[1] < 0.2 {
                reds += b.count;
            } else if c[0] > 0.9 && c[1] > 0.9 && c[2] > 0.9 {
                whites += b.count;
            }
        }
        assert_eq!(reds, whites, "{reds} red and {whites} white");
        // Sixteen by eight quads, each cut into two triangles.
        assert_eq!(reds + whites, 16 * 8 * 6);
    }
}
