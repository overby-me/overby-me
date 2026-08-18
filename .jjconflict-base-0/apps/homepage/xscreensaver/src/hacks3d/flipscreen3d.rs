//! Port of `hacks/glx/flipscreen3d.c`.
//!
//! ```text
//! flipscreen3d - takes snapshots of the screen and flips it around
//!
//! version 1.0 - Oct 24, 2001
//!
//! Copyright (C) 2001 Ben Buxton (bb@cactii.net)
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
//! A picture on a sheet, tumbling.
//!
//! The tumble is three sines of three angles for the position and one rotation
//! about an axis that changes now and then, and the speed of each is re-rolled
//! at random. The rotation has an acceleration as well as a speed, and the
//! acceleration reverses whenever the speed passes five, which is what keeps
//! it from winding up into a blur.
//!
//! The sheet also stretches, in x and in y, by a sine of its own. Every so
//! often, when the spin changes direction, it fades out and comes back with a
//! new picture.
//!
//! Two quads back to back, not one: it is drawn from both sides, and with the
//! depth writes off so that the two faces of the same sheet do not fight.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Glx, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// The sheet is this many units across before the picture's shape is taken
/// into account.
const QW: f32 = 12.0;
const QH: f32 = 12.0;

struct FlipScreen3d {
    trackball: Trackball,
    winw: i32,
    winh: i32,
    /// The size of the picture, and which part of it is the picture rather
    /// than the black it was centred on.
    tw: i32,
    min_tx: f32,
    min_ty: f32,
    max_tx: f32,
    max_ty: f32,
    /// The quad it goes on.
    qx: f32,
    qy: f32,
    qw: f32,
    qh: f32,
    regrab: bool,
    fadetime: bool,
    show_colors: [f32; 4],
    stretch_val_x: f32,
    stretch_val_y: f32,
    stretch_val_dx: f32,
    stretch_val_dy: f32,
    /// Where it is on its way in from, before the tumbling starts.
    curx: f32,
    cury: f32,
    curz: f32,
    /// The axis it turns about, how far it has turned, how fast, and how fast
    /// that is changing.
    rx: f32,
    ry: f32,
    rz: f32,
    rot: f32,
    drot: f32,
    odrot: f32,
    ddrot: f32,
    theta: f32,
    rho: f32,
    gamma: f32,
    dtheta: f32,
    drho: f32,
    dgamma: f32,
    texid: u32,
    waiting_for_image: bool,
    first_image: bool,
    rotate: bool,
    wire: bool,
}

impl FlipScreen3d {
    /// The sheet: the picture on two quads back to back, and a line round the
    /// edge of it.
    fn showscreen(&mut self, g: &mut Glx, frozen: bool, turning: bool) {
        if self.fadetime {
            self.show_colors[3] -= 0.02;
            if self.show_colors[3] < 0.0 {
                self.regrab = true;
                self.fadetime = false;
            }
        } else if self.show_colors[3] < 0.0 {
            self.show_colors = [1.0; 4];
            self.stretch_val_x = 0.0;
            self.stretch_val_y = 0.0;
            self.stretch_val_dx = 0.0;
            self.stretch_val_dy = 0.0;
        }
        if self.stretch_val_dx == 0.0 && !frozen && random().is_multiple_of(25) {
            self.stretch_val_dx = (random() % 100) as f32 / 5000.0;
        }
        if self.stretch_val_dy == 0.0 && !frozen && random().is_multiple_of(25) {
            self.stretch_val_dy = (random() % 100) as f32 / 5000.0;
        }

        let mut x = self.qx;
        let mut y = self.qy;
        let mut w = self.qx + self.qw;
        let mut h = self.qy - self.qh;

        if !frozen {
            w *= self.stretch_val_x.sin() + 1.0;
            x *= self.stretch_val_x.sin() + 1.0;
            if turning {
                if !self.fadetime {
                    self.stretch_val_x += self.stretch_val_dx;
                }
                if self.stretch_val_x > std::f32::consts::TAU && random().is_multiple_of(5) {
                    self.stretch_val_dx = (random() % 100) as f32 / 5000.0;
                } else {
                    self.stretch_val_x -= std::f32::consts::TAU;
                }
            }

            if turning && !self.fadetime {
                self.stretch_val_y += self.stretch_val_dy;
            }
            h *= self.stretch_val_y.sin() / 2.0 + 1.0;
            y *= self.stretch_val_y.sin() / 2.0 + 1.0;
            if turning {
                if self.stretch_val_y > std::f32::consts::TAU && random().is_multiple_of(5) {
                    self.stretch_val_dy = (random() % 100) as f32 / 5000.0;
                } else {
                    self.stretch_val_y -= std::f32::consts::TAU;
                }
            }
        }

        let c = self.show_colors;
        g.color4f(c[0], c[1], c[2], c[3]);

        if !self.wire {
            g.texturing(true);
            g.bind_texture(self.texid);
            g.blend(Blend::Alpha);
            // The two faces of the same sheet must not fight over the depth
            // buffer.
            g.depth_mask(false);
        }

        g.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        g.normal3f(0.0, 0.0, 1.0);
        g.tex_coord2f(self.max_tx, self.max_ty);
        g.vertex3f(w, h, 0.0);
        g.tex_coord2f(self.max_tx, self.min_ty);
        g.vertex3f(w, y, 0.0);
        g.tex_coord2f(self.min_tx, self.min_ty);
        g.vertex3f(x, y, 0.0);
        g.tex_coord2f(self.min_tx, self.max_ty);
        g.vertex3f(x, h, 0.0);

        g.normal3f(0.0, 0.0, -1.0);
        g.tex_coord2f(self.min_tx, self.min_ty);
        g.vertex3f(x, y, -0.05);
        g.tex_coord2f(self.max_tx, self.min_ty);
        g.vertex3f(w, y, -0.05);
        g.tex_coord2f(self.max_tx, self.max_ty);
        g.vertex3f(w, h, -0.05);
        g.tex_coord2f(self.min_tx, self.max_ty);
        g.vertex3f(x, h, -0.05);
        g.end();

        g.texturing(false);
        g.depth_mask(true);

        g.begin(Shape::LineLoop);
        g.vertex3f(x, y, 0.0);
        g.vertex3f(x, h, 0.0);
        g.vertex3f(w, h, 0.0);
        g.vertex3f(w, y, 0.0);
        g.end();
        g.blend(Blend::Off);
    }

    /// Zoom the sheet back and put it in the middle after a new picture has
    /// arrived. True once it is where it belongs and the tumbling can start.
    fn inposition(&mut self, g: &mut Glx) -> bool {
        let wx = -(self.qw / 2.0);
        let wy = self.qh / 2.0;

        if self.curx == 0.0 {
            self.curx = self.qx;
        }
        if self.cury == 0.0 {
            self.cury = self.qy;
        }
        if self.regrab {
            self.curz = 0.0;
            self.curx = self.qx;
            self.cury = self.qy;
            self.regrab = false;
        }
        if self.curz > -10.0
            || self.curx > wx + 0.1
            || self.curx < wx - 0.1
            || self.cury > wy + 0.1
            || self.cury < wy - 0.1
        {
            if self.curz > -10.0 {
                self.curz -= 0.05;
            }
            if self.curx > wx {
                self.qx -= 0.02;
                self.curx -= 0.02;
            }
            if self.curx < wx {
                self.qx += 0.02;
                self.curx += 0.02;
            }
            if self.cury > wy {
                self.qy -= 0.02;
                self.cury -= 0.02;
            }
            if self.cury < wy {
                self.qy += 0.02;
                self.cury += 0.02;
            }
            g.translate(0.0, 0.0, self.curz);
            return false;
        }
        g.translate(0.0, 0.0, self.curz);
        true
    }

    /// A new picture, when there is one to be had.
    fn get_snapshot(&mut self, g: &mut Gl) {
        if self.wire {
            return;
        }
        let Some(img) = g.load_image(512, 512) else {
            return;
        };
        let (tw, th) = (img.width, img.height);
        let geom = img.geometry;
        self.tw = tw;
        self.min_tx = geom.x as f32 / tw as f32;
        self.min_ty = geom.y as f32 / th as f32;
        self.max_tx = (geom.x + geom.width as i16 as i32) as f32 / tw as f32;
        self.max_ty = (geom.y + geom.height as i16 as i32) as f32 / th as f32;

        self.qx = -QW / 2.0 + (geom.x as f32 * QW / tw as f32);
        self.qy = QH / 2.0 - (geom.y as f32 * QH / th as f32);
        self.qw = QW * (geom.width as f32 / tw as f32);
        self.qh = QH * (geom.height as f32 / th as f32);

        g.glx.bind_texture(self.texid);
        g.glx.tex_image_2d(tw, th, img.pixels);
        g.glx.tex_clamp(true);
        self.waiting_for_image = false;
        self.first_image = false;
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let texid = g.glx.gen_texture();
    let mut this = FlipScreen3d {
        trackball: Trackball::new(),
        winw: g.width(),
        winh: g.height(),
        tw: 0,
        min_tx: 0.0,
        min_ty: 0.0,
        max_tx: 1.0,
        max_ty: 1.0,
        qx: -6.0,
        qy: 6.0,
        qw: QW,
        qh: QH,
        regrab: false,
        fadetime: false,
        show_colors: [1.0; 4],
        stretch_val_x: 0.0,
        stretch_val_y: 0.0,
        stretch_val_dx: 0.0,
        stretch_val_dy: 0.0,
        curx: 0.0,
        cury: 0.0,
        curz: 0.0,
        rx: 1.0,
        ry: 1.0,
        rz: 0.0,
        rot: 0.0,
        drot: 0.0,
        odrot: 1.0,
        ddrot: 0.0,
        theta: 0.0,
        rho: 0.0,
        gamma: 0.0,
        dtheta: 0.0,
        drho: 0.0,
        dgamma: 0.0,
        texid,
        waiting_for_image: true,
        first_image: true,
        rotate: g.res.bool("rotate"),
        wire,
    };
    this.get_snapshot(g);
    Box::new(this)
}

impl Hack3d for FlipScreen3d {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.winw = width;
        self.winh = height;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) && !self.waiting_for_image {
            self.waiting_for_image = true;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        // Wait for the first picture; later ones arrive while it animates.
        if self.waiting_for_image {
            self.get_snapshot(g);
            if self.first_image {
                return g.res.int("delay") as u32;
            }
        }
        if self.regrab {
            self.waiting_for_image = true;
            self.get_snapshot(g);
        }

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0, 2.0, 85.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 15.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.lighting(false);
        if !self.wire {
            g.glx.depth_test(true);
            g.glx.cull_face(true);
        }

        let turning = !self.trackball.button_down();
        g.glx.push_matrix();

        let glx = &mut g.glx;
        let frozen = if self.inposition(glx) {
            glx.translate(
                5.0 * self.theta.sin(),
                5.0 * self.rho.sin(),
                10.0 * self.gamma.cos() - 10.0,
            );
            // Now and then, a new speed for each of the three.
            if turning && random().is_multiple_of(300) {
                if random() % 2 == 1 {
                    self.drho = -((random() % 100) as f32 / 3000.0);
                }
                if random() % 2 == 1 {
                    self.dtheta = -((random() % 100) as f32 / 3000.0);
                }
                if random() % 2 == 1 {
                    self.dgamma = -((random() % 100) as f32 / 3000.0);
                }
            }
            let m = self.trackball.matrix();
            glx.mult_matrix(m);
            if self.rotate {
                glx.rotate(self.rot, self.rx, self.ry, self.rz);
            }
            if turning && !self.fadetime {
                self.theta += self.dtheta;
                self.rho += self.drho;
                self.gamma += self.dgamma;
                self.rot += self.drot;
                self.drot += self.ddrot;
            }
            // Do not let the spin wind up into a blur.
            if self.drot > 5.0 && self.ddrot > 0.0 {
                self.ddrot = -((random() % 100) as f32 / 1000.0);
            } else if self.drot < -5.0 && self.ddrot < 0.0 {
                self.ddrot = (random() % 100) as f32 / 1000.0;
            }
            false
        } else {
            // Still on its way in.
            self.ddrot = 0.05 - (random() % 100) as f32 / 1000.0;
            self.theta = 0.0;
            self.rho = 0.0;
            self.gamma = 0.0;
            self.rot = 0.0;
            true
        };

        if turning
            && !self.fadetime
            && (self.rot >= 360.0 || self.rot <= -360.0)
            && random().is_multiple_of(7)
        {
            self.rx = (random() % 100) as f32 / 100.0;
            self.ry = (random() % 100) as f32 / 100.0;
            self.rz = (random() % 100) as f32 / 100.0;
        }
        // When the spin changes direction, sometimes fade out and get a new
        // picture. Only if the picture is smaller than the window: a picture
        // as big as the screen is the screen, and there is nothing new to be
        // had.
        if self.odrot * self.drot < 0.0 && self.tw < self.winw && random().is_multiple_of(10) {
            self.fadetime = true;
        }
        self.odrot = self.drot;
        if self.rot > 360.0 || self.rot < -360.0 {
            self.rot -= self.rot;
        }

        let glx = &mut g.glx;
        self.showscreen(glx, frozen, turning);
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      20000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*rotate:     True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("rotate", "Rotate", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flipscreen3d",
    label: "Flip Screen 3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ben Buxton",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=mu3iN_BSpt4"),
        blurb: "Takes a picture and flips it around.",
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

    /// The sheet is two quads back to back, drawn with the depth writes off
    /// so that its own two faces do not fight, and a line round the edge.
    #[test]
    fn the_sheet_is_two_faced() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..10 {
            r.step();
        }
        let f = r.frame();
        let quads: Vec<_> = f.batches.iter().filter(|b| b.texture.is_some()).collect();
        assert!(!quads.is_empty(), "the picture is not on it");
        assert!(
            quads.iter().all(|b| !b.depth_mask),
            "the sheet writes depth"
        );
        // Two quads is twelve vertices once they are cut into triangles.
        let n: usize = quads.iter().map(|b| b.count).sum();
        assert_eq!(n, 12, "{n} vertices is not two quads");
        assert!(
            f.batches
                .iter()
                .any(|b| b.primitive == crate::runtime::gl::Primitive::LineLoop),
            "there is no line round the edge"
        );
    }

    /// It tumbles: the sheet's matrix keeps changing, and the spin never
    /// winds up into a blur.
    #[test]
    fn it_tumbles_without_winding_up() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut seen: Vec<f32> = Vec::new();
        for _ in 0..2000 {
            r.step();
            if let Some(b) = r.frame().batches.first() {
                seen.push(b.modelview.0[0]);
            }
        }
        let lo = seen.iter().copied().fold(f32::MAX, f32::min);
        let hi = seen.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.1, "it never turned");
        // Every step of it is small: a frame never jumps.
        let step = seen
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(step < 0.5, "it jumped by {step}");
    }

    /// It fades out and comes back with a new picture, which is what the
    /// alpha of its colour is for.
    #[test]
    fn it_fades_before_it_regrabs() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut faded = false;
        for _ in 0..4000 {
            r.step();
            let f = r.frame();
            if let Some(v) = f.vertices.first()
                && v.color[3] < 0.9
            {
                faded = true;
            }
        }
        assert!(faded, "it never faded");
    }
}
