//! Port of `hacks/glx/photopile.c`.
//!
//! ```text
//! photopile, Copyright © 2008-2025 Jamie Zawinski <jwz@jwz.org>
//! Loads a sequence of images and shuffles them into a pile.
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
//! Photographs dropped one at a time onto a pile of the ones before.
//!
//! A photo's journey is a Bezier curve through four control points: where it
//! was, where it is going, and two more, each thrown out from one end in a
//! random direction by about the diagonal of the photo. That is what makes a
//! photo swing out of the pile and back into it rather than sliding across.
//!
//! Everything is drawn in window coordinates, with an orthographic projection
//! and a depth range of one, and the pile is stacked by giving each photo its
//! own slice of that depth.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::dropshadow::{draw_drop_shadow, init_drop_shadow};
use crate::runtime::gl::{Blend, Glx, Shape};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, XRectangle, frand,
};

/// Three uniform numbers averaged: the middle of the range far more often
/// than either end.
fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

const FADE_TICKS: f32 = 60.0;

#[derive(Clone, Copy, Default)]
struct Position {
    x: f32,
    y: f32,
    angle: f32,
}

fn lerp(t: f32, p: Position, q: Position) -> Position {
    Position {
        x: p.x * (1.0 - t) + q.x * t,
        y: p.y * (1.0 - t) + q.y * t,
        angle: p.angle * (1.0 - t) + q.angle * t,
    }
}

/// de Casteljau's algorithm over four control points.
fn interpolate(t: f32, p: [Position; 4]) -> Position {
    let p10 = lerp(t, p[0], p[1]);
    let p11 = lerp(t, p[1], p[2]);
    let p12 = lerp(t, p[2], p[3]);
    let p20 = lerp(t, p10, p11);
    let p21 = lerp(t, p11, p12);
    lerp(t, p20, p21)
}

struct Image {
    loaded: bool,
    title: String,
    /// The size of the picture and of the texture it is in, and where in that
    /// texture the picture is.
    w: f32,
    h: f32,
    tw: f32,
    th: f32,
    geom: XRectangle,
    pos: [Position; 4],
    texid: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Early,
    Shuffle,
    Normal,
    Loading,
}

struct PhotoPile {
    frames: Vec<Image>,
    /// The photo being loaded, which is also the one at the bottom.
    nframe: usize,
    shadow: u32,
    font: TexFont,
    last_time: f64,
    mode: Mode,
    mode_tick: f32,
    width: i32,
    height: i32,
    count: usize,
    scale: f32,
    max_tilt: f32,
    speed: f32,
    duration: f64,
    titles: bool,
    polaroid: bool,
    clip: bool,
    shadows: bool,
    wire: bool,
}

impl PhotoPile {
    /// New places for all of them: where each is now becomes where it starts,
    /// and the two middle control points are thrown out from either end.
    fn set_new_positions(&mut self) {
        let (w, h) = (self.width as f32, self.height as f32);
        let max_tilt = self.max_tilt;
        for frame in &mut self.frames {
            let d = (frame.w * frame.w + frame.h * frame.h).sqrt();
            let leave = frand(std::f64::consts::PI * 2.0) as f32;
            let enter = frand(std::f64::consts::PI * 2.0) as f32;

            frame.pos[0] = frame.pos[3];
            frame.pos[3] = Position {
                // Mostly inside the window: pulled in from the far edge
                // first and then from the near one, so that a photo wider
                // than the window ends up against the near edge rather than
                // nowhere.
                x: bellrand(w as f64).min(w - 0.5 * frame.w).max(0.5 * frame.w),
                y: bellrand(h as f64).min(h - 0.5 * frame.h).max(0.5 * frame.h),
                angle: (frand(2.0) as f32 - 1.0) * max_tilt,
            };

            let offset = |p: Position, th: f32| {
                let r = d * (0.5 + frand(1.0) as f32);
                Position {
                    x: p.x + th.cos() * r,
                    y: p.y + th.sin() * r,
                    angle: (frand(2.0) as f32 - 1.0) * max_tilt,
                }
            };
            frame.pos[1] = offset(frame.pos[0], leave);
            frame.pos[2] = offset(frame.pos[3], enter);
        }
    }

    /// Take the next picture into the frame that is due to be replaced.
    fn load_image(&mut self, g: &mut Gl) {
        let size = ((self.width.max(self.height) as f32 * self.scale) as i32).max(10);
        if self.wire {
            let frame = &mut self.frames[self.nframe];
            let (w, h) = (
                (self.width as f32 * self.scale) as i32 - 1,
                (self.height as f32 * self.scale) as i32 - 1,
            );
            let (w, h) = (w.max(10) as f32, h.max(10) as f32);
            frame.w = w;
            frame.h = h;
            frame.geom = XRectangle {
                x: 0,
                y: 0,
                width: w as i32,
                height: h as i32,
            };
            frame.loaded = true;
            return;
        }
        let Some(img) = g.load_image(size, size) else {
            return;
        };
        let texid = self.frames[self.nframe].texid;
        g.glx.bind_texture(texid);
        g.glx.tex_image_2d(img.width, img.height, img.pixels);
        g.glx.tex_clamp(false);

        let frame = &mut self.frames[self.nframe];
        frame.w = img.width as f32;
        frame.h = img.height as f32;
        frame.tw = img.width as f32;
        frame.th = img.height as f32;
        frame.geom = img.geometry;
        frame.title = img.title.unwrap_or_default();
        frame.loaded = true;
    }

    /// One photo: its shadow, the instant-film border, the picture, a box
    /// round it and its title.
    fn draw_image(&self, g: &mut Glx, i: usize, t: f32, s: f32, z: f32) {
        let frame = &self.frames[i];
        let pos = interpolate(t, frame.pos);
        let mut w = frame.geom.width as f32 * 0.5;
        let mut h = frame.geom.height as f32 * 0.5;
        let z1 = z - 0.25 / (self.count + 1) as f32;
        let z2 = z - 0.5 / (self.count + 1) as f32;
        let mut w1 = w;
        let mut h1 = h;
        let mut h2 = h;
        let mut s = s;

        if self.polaroid {
            let min = w.min(h);
            let max = w.max(h);
            // Clip the picture to the frame, or scale it to fit.
            if self.clip {
                w = min;
                h = min;
            } else {
                let s2 = min / max;
                w *= s2;
                h *= s2;
            }
            w1 = min * 1.16; // A border round it.
            h1 = min * 1.5;
            h2 = w1;
            s /= 1.5; // Which the photo shrinks to make room for.
        }

        g.push_matrix();
        g.translate(pos.x, pos.y, 0.0);
        g.rotate(pos.angle, 0.0, 0.0, 1.0);
        g.scale(s, s, 1.0);

        if self.shadows && !self.wire {
            g.color4f(0.0, 0.0, 0.0, 1.0);
            g.blend(Blend::Alpha);
            draw_drop_shadow(g, self.shadow, -w1, -h1, z2, 2.0 * w1, h1 + h2, 20.0);
            g.blend(Blend::Off);
        }

        // The retro instant-film frame.
        if self.polaroid {
            if !self.wire {
                g.color4f(1.0, 1.0, 1.0, 1.0);
                g.begin(Shape::Quads);
                g.vertex3f(-w1, -h1, z2);
                g.vertex3f(w1, -h1, z2);
                g.vertex3f(w1, h2, z2);
                g.vertex3f(-w1, h2, z2);
                g.end();
            }
            g.line_width(1.0);
            g.color4f(0.5, 0.5, 0.5, 1.0);
            g.begin(Shape::LineLoop);
            g.vertex3f(-w1, -h1, z);
            g.vertex3f(w1, -h1, z);
            g.vertex3f(w1, h2, z);
            g.vertex3f(-w1, h2, z);
            g.end();
        }

        // The picture.
        if !self.wire {
            let texw = w / frame.tw;
            let texh = h / frame.th;
            let texx = (frame.geom.x as f32 + 0.5 * frame.geom.width as f32) / frame.tw;
            let texy = (frame.geom.y as f32 + 0.5 * frame.geom.height as f32) / frame.th;
            g.texturing(true);
            g.bind_texture(frame.texid);
            g.color4f(1.0, 1.0, 1.0, 1.0);
            g.begin(Shape::Quads);
            g.tex_coord2f(texx - texw, texy + texh);
            g.vertex3f(-w, -h, z1);
            g.tex_coord2f(texx + texw, texy + texh);
            g.vertex3f(w, -h, z1);
            g.tex_coord2f(texx + texw, texy - texh);
            g.vertex3f(w, h, z1);
            g.tex_coord2f(texx - texw, texy - texh);
            g.vertex3f(-w, h, z1);
            g.end();
            g.texturing(false);
        }

        // A box round it.
        g.line_width(1.0);
        g.color4f(0.5, 0.5, 0.5, 1.0);
        g.begin(Shape::LineLoop);
        g.vertex3f(-w, -h, z);
        g.vertex3f(w, -h, z);
        g.vertex3f(w, h, z);
        g.vertex3f(-w, h, z);
        g.end();

        // And a title under it.
        if self.titles {
            let title = if frame.title.is_empty() {
                "(untitled)"
            } else {
                &frame.title
            };
            let m = self.font.metrics(title);
            let sh = (m.ascent + m.descent) as f32;
            let tpad = w * 0.05;
            let tboxh = if self.polaroid {
                h1 - h - tpad * 2.0
            } else {
                sh * 3.0
            };
            // Three lines of text fit the space under the picture.
            let tscale = tboxh / (sh * 3.0);

            g.translate(0.0, -(h + tpad), 0.0);
            if self.wire || !self.polaroid {
                g.color4f(1.0, 1.0, 1.0, 1.0);
            } else {
                g.color4f(0.5, 0.5, 0.5, 1.0);
            }
            g.scale(tscale, tscale, 1.0);
            g.translate(0.0, -(m.ascent as f32), 0.0);
            g.translate(0.0, -sh, 0.0);

            if !self.wire {
                g.blend(Blend::Alpha);
                g.depth_test(false);
                // Upstream centres each line itself, because the font's own
                // newline handling is flush left.
                let sw = m.width as f32;
                g.push_matrix();
                g.translate(-sw / 2.0, 0.0, 0.0);
                self.font.print_string(g, title);
                g.pop_matrix();
                g.depth_test(true);
                g.blend(Blend::Off);
            }
        }
        g.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let count = g.res.int("count").max(1) as usize;
    let font = TexFont::load(&mut g.glx, g.res.string("font"));
    let shadow = init_drop_shadow(&mut g.glx);

    let frames = (0..count + 1)
        .map(|_| Image {
            loaded: false,
            title: String::new(),
            w: 0.0,
            h: 0.0,
            tw: 1.0,
            th: 1.0,
            geom: XRectangle {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            pos: [Position::default(); 4],
            texid: if wire { 0 } else { g.glx.gen_texture() },
        })
        .collect();

    let mut this = PhotoPile {
        frames,
        nframe: 0,
        shadow,
        font,
        last_time: 0.0,
        mode: Mode::Early,
        mode_tick: 0.0,
        width: g.width(),
        height: g.height(),
        count,
        scale: g.res.float("imgScale") as f32,
        max_tilt: g.res.float("maxTilt") as f32,
        speed: g.res.float("speed") as f32,
        duration: g.res.float("duration"),
        titles: g.res.bool("titles"),
        polaroid: g.res.bool("polaroid"),
        clip: g.res.bool("clip"),
        shadows: g.res.bool("shadows"),
        wire,
    };
    // Start loading the first picture.
    this.load_image(g);
    Box::new(this)
}

impl Hack3d for PhotoPile {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .ortho(0.0, self.width as f32, 0.0, self.height as f32, -1.0, 1.0);
        let s = if self.width < self.height {
            self.width as f32 / self.height as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        g.glx.lighting(false);

        if self.mode == Mode::Early {
            if !self.frames[self.nframe].loaded {
                self.load_image(g);
            }
            if !self.frames[self.nframe].loaded {
                // Nothing to show yet.
                return g.res.int("delay") as u32;
            }
            // The first one has arrived: put the rest in the middle and let
            // them shuffle out from there.
            let (w, h) = (self.width as f32 * 0.5, self.height as f32 * 0.5);
            for i in 0..self.nframe {
                self.frames[i].pos[3] = Position {
                    x: w,
                    y: h,
                    angle: 0.0,
                };
            }
            self.set_new_positions();
            self.mode = Mode::Shuffle;
            self.mode_tick = FADE_TICKS / self.speed;
        }

        match self.mode {
            Mode::Shuffle => {
                self.mode_tick -= 1.0;
                if self.mode_tick <= 0.0 {
                    self.nframe = (self.nframe + 1) % (self.count + 1);
                    self.mode = Mode::Normal;
                    self.last_time = g.time;
                }
            }
            Mode::Normal => {
                if g.time - self.last_time > self.duration {
                    self.mode = Mode::Loading;
                    self.frames[self.nframe].loaded = false;
                    self.load_image(g);
                }
            }
            Mode::Loading => {
                if !self.frames[self.nframe].loaded {
                    self.load_image(g);
                }
                if self.frames[self.nframe].loaded {
                    self.set_new_positions();
                    self.mode = Mode::Shuffle;
                    self.mode_tick = FADE_TICKS / self.speed;
                }
            }
            Mode::Early => {}
        }

        let mut t = 1.0 - self.mode_tick / (FADE_TICKS / self.speed);
        t = 0.5 * (1.0 - (std::f32::consts::PI * t).cos());

        let n = self.count + usize::from(self.mode == Mode::Shuffle);
        for i in 0..n {
            let j = (self.nframe + i + 1) % (self.count + 1);
            if !self.frames[j].loaded {
                continue;
            }
            let mut s = 1.0;
            let z = i as f32 / (self.count + 1) as f32;
            let mut tt = t;
            match self.mode {
                Mode::Shuffle => {
                    // The one arriving grows in and the one leaving shrinks
                    // out.
                    if i == self.count {
                        s *= t;
                    } else if i == 0 {
                        s *= 1.0 - t;
                    }
                }
                _ => tt = 1.0,
            }
            let glx = &mut g.glx;
            self.draw_image(glx, j, tt, s, z);
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*count:      7",
    "*delay:      10000",
    "*wireframe:  False",
    "*showFPS:    False",
    "*font:       monospace 18",
    "*imgScale:   0.4",
    "*maxTilt:    50",
    "*speed:      1.0",
    "*duration:   5",
    "*titles:     True",
    "*polaroid:   True",
    "*clip:       True",
    "*shadows:    True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("speed", "Animation speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("duration", "Duration", 1.0, 60.0, 1.0, 0, "5"),
    Opt::slider("count", "Number of images", 1.0, 20.0, 1.0, 0, "7"),
    Opt::slider("imgScale", "Image size", 0.1, 1.0, 0.05, 2, "0.4"),
    Opt::slider("maxTilt", "Max tilt", 0.0, 90.0, 1.0, 0, "50"),
    Opt::boolean("titles", "Titles", "true"),
    Opt::boolean("polaroid", "Instant film look", "true"),
    Opt::boolean("clip", "Clip to square", "true"),
    Opt::boolean("shadows", "Drop shadows", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "photopile",
    label: "Photopile",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2008",
        video: Some("https://www.youtube.com/watch?v=snm7o95AR8E"),
        blurb: "Loads a sequence of images and shuffles them into a pile.",
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

    /// A photo's journey is a Bezier through four points, so it leaves and
    /// arrives along the directions the middle two set rather than sliding
    /// straight across.
    #[test]
    fn a_photo_swings_out_and_back() {
        let p = [
            Position {
                x: 0.0,
                y: 0.0,
                angle: 0.0,
            },
            Position {
                x: 0.0,
                y: 100.0,
                angle: 0.0,
            },
            Position {
                x: 100.0,
                y: 100.0,
                angle: 0.0,
            },
            Position {
                x: 100.0,
                y: 0.0,
                angle: 0.0,
            },
        ];
        assert_eq!(interpolate(0.0, p).x, 0.0);
        assert_eq!(interpolate(1.0, p).x, 100.0);
        let mid = interpolate(0.5, p);
        assert!((mid.x - 50.0).abs() < 0.01, "it is not symmetric");
        // It bulges toward the middle control points.
        assert!(mid.y > 50.0, "it went straight across");
    }

    /// The pile stacks: every photo gets its own slice of the depth range,
    /// and the newest is at the top.
    #[test]
    fn the_pile_is_stacked() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..200 {
            r.step();
        }
        let f = r.frame();
        let zs: Vec<f32> = f.vertices.iter().map(|v| v.pos[2]).collect();
        let lo = zs.iter().copied().fold(f32::MAX, f32::min);
        let hi = zs.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi > lo, "every photo is at the same depth");
        assert!(
            hi <= 1.0 && lo >= -1.0,
            "a photo is outside the depth range"
        );
    }

    /// Each photo has a shadow under it, a white border, the picture, and a
    /// box round it. A new one arrives every few seconds, so this has to run
    /// for a while to see more than the first.
    #[test]
    fn a_photo_has_a_shadow_and_a_border() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..2000 {
            r.step();
        }
        let f = r.frame();
        let textured = f.batches.iter().filter(|b| b.texture.is_some()).count();
        // The shadow and the picture are both textured, so there are at least
        // two of them a photo.
        assert!(textured >= 4, "only {textured} textured draws");
        assert!(
            f.batches
                .iter()
                .any(|b| b.primitive == crate::runtime::gl::Primitive::LineLoop),
            "there is no box round it"
        );
    }

    /// Without the instant-film look the photo is not clipped square and has
    /// no border, which is a different shape entirely.
    #[test]
    fn plain_photos_have_no_border() {
        let mut r = start(StartArgs::new(640, 480, "polaroid=false", 20260811));
        for _ in 0..200 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing was drawn");
        // No white quad behind the picture.
        let white = f
            .batches
            .iter()
            .filter(|b| b.texture.is_none())
            .flat_map(|b| f.vertices[b.first..b.first + b.count].iter())
            .filter(|v| v.color[0] > 0.9 && v.color[1] > 0.9 && v.color[2] > 0.9)
            .count();
        assert_eq!(white, 0, "the instant-film border is still there");
    }
}
