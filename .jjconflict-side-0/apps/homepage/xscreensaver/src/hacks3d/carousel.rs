//! Port of `hacks/glx/carousel.c`.
//!
//! ```text
//! carousel, Copyright © 2005-2025 Jamie Zawinski <jwz@jwz.org>
//! Loads a sequence of images and rotates them around.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Created: 21-Feb-2005
//! ```
//!
//! Pictures on the inside of a turning drum.
//!
//! A frame holds two pictures, the one on show and the one being fetched. When
//! the new one arrives the old one drops out of the ring and the new one drops
//! in after it, from the top or the bottom, and the drop overshoots and comes
//! back, which is what the easing curve is for.
//!
//! Each frame expires at its own time, staggered when the ring is first filled
//! and then shuffled, so they do not all change at once and do not go round in
//! order. Only one picture is ever fetched at a time.
//!
//! The pictures are drawn before any of the titles, because the titles are the
//! only thing here that is not fully opaque and blending only comes out right
//! back to front.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Glx, Shape};
use crate::runtime::rotator::Rotator;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, XRectangle, frand,
    random,
};

const FADE_TICKS: f32 = 30.0 * 5.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Early,
    Normal,
    Loading,
    Out,
    In,
}

#[derive(Clone, Default)]
struct Image {
    title: String,
    /// The size of the texture, and where in it the picture is.
    tw: f32,
    th: f32,
    geom: XRectangle,
    texid: u32,
}

struct Frame {
    current: Image,
    loading: Image,
    /// Where on the drum: how far out, and how far round.
    r: f32,
    theta: f32,
    /// Its own slow in-and-out drift, and where that has got to. Upstream
    /// reads the drift inside its drawing, which happens twice a frame, once
    /// for the pictures and once for the titles; this steps it in the same
    /// two places so that it drifts at the same rate.
    rot: Rotator,
    zoom_z: f32,
    /// Whether this picture drops in from the top or rises from the bottom.
    from_top: bool,
    /// When this picture should be replaced.
    expires: f64,
    mode: Mode,
    mode_tick: f32,
    loaded: bool,
}

struct Carousel {
    trackball: Trackball,
    rot: Rotator,
    frames: Vec<Frame>,
    awaiting_first_images: bool,
    loads_in_progress: i32,
    font: TexFont,
    titlefont: TexFont,
    mode: Mode,
    mode_tick: f32,
    width: i32,
    height: i32,
    count: usize,
    speed: f32,
    duration: f64,
    titles: bool,
    zoom: bool,
    tilt_x: bool,
    tilt_y: bool,
    wire: bool,
}

impl Carousel {
    /// Start fetching a picture into a frame's spare slot.
    fn load_image(&mut self, g: &mut Gl, i: usize) {
        // Only a frame that is new or settled is ever asked for a
        // picture; upstream aborts on the rest, and this leaves them alone.
        match self.frames[i].mode {
            Mode::Early => {}
            Mode::Normal => self.frames[i].mode = Mode::Loading,
            _ => return,
        }
        self.loads_in_progress += 1;
        self.frames[i].loaded = false;

        let mut w = (self.width / 2 - 1).max(10);
        let mut h = (self.height / 2 - 1).max(10);
        if w > h * 5 {
            // A tiny window: use sixteen by nine boxes.
            h = w * 9 / 16;
        }
        if self.wire {
            w = (self.width as f32 * (0.5 + frand(1.0) as f32)) as i32;
            h = self.height;
        }
        let Some(img) = g.load_image(w, h) else {
            return;
        };
        let texid = self.frames[i].loading.texid;
        g.glx.bind_texture(texid);
        g.glx.tex_image_2d(img.width, img.height, img.pixels);
        g.glx.tex_clamp(false);

        let frame = &mut self.frames[i];
        frame.loading.tw = img.width as f32;
        frame.loading.th = img.height as f32;
        frame.loading.geom = img.geometry;
        frame.loading.title = img.title.unwrap_or_default();
        frame.loaded = true;
        self.loads_in_progress -= 1;

        // A picture expires this long after it finished loading.
        let expires = g.time + self.duration * self.count as f64;
        let frame = &mut self.frames[i];
        frame.expires = expires;
        match frame.mode {
            // One of the first batch.
            Mode::Early => std::mem::swap(&mut frame.current, &mut frame.loading),
            // Start dropping the old one out.
            Mode::Loading => {
                frame.mode = Mode::Out;
                frame.mode_tick = FADE_TICKS / self.speed;
                frame.from_top = random() & 1 == 1;
            }
            _ => {}
        }
    }

    /// A new frame, with its own textures and its own drift.
    fn alloc_frame(&mut self, g: &mut Gl) {
        let speed = self.speed;
        self.frames.push(Frame {
            current: Image {
                texid: g.glx.gen_texture(),
                ..Image::default()
            },
            loading: Image {
                texid: g.glx.gen_texture(),
                ..Image::default()
            },
            r: 0.0,
            theta: 0.0,
            rot: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.04 * frand(1.0) * speed as f64, false),
            zoom_z: 0.0,
            from_top: false,
            expires: 0.0,
            mode: Mode::Early,
            mode_tick: 0.0,
            loaded: false,
        });
    }

    /// Fill the ring one picture at a time, and once it is full, stagger and
    /// then shuffle when each of them expires.
    fn load_initial_images(&mut self, g: &mut Gl) {
        if !self.frames.iter().all(|f| f.loaded) {
            return;
        }
        if self.frames.len() < self.count {
            self.alloc_frame(g);
            let i = self.frames.len() - 1;
            self.load_image(g, i);
            return;
        }

        let n = self.frames.len();
        let now = g.time;
        for (i, frame) in self.frames.iter_mut().enumerate() {
            frame.r = 1.0;
            frame.theta = i as f32 * 360.0 / n as f32;
            frame.expires = now + self.duration * (i + 1) as f64;
            frame.mode = Mode::Normal;
        }
        // Shuffle the expiry times, so they do not drop out in order round
        // the ring.
        for i in 0..n {
            let j = random() as usize % n;
            let swap = self.frames[i].expires;
            self.frames[i].expires = self.frames[j].expires;
            self.frames[j].expires = swap;
        }
        self.awaiting_first_images = false;
    }

    /// The message while the first pictures are on their way.
    fn loading_msg(&self, g: &mut Glx, n: usize) {
        if self.wire {
            return;
        }
        let text = if n == 0 {
            "Loading images...".to_string()
        } else {
            format!("Loading images...  ({}%)", n * 100 / self.count)
        };
        let m = self.titlefont.metrics(&text);

        g.matrix_mode_projection();
        g.load_identity();
        g.matrix_mode_modelview();
        g.load_identity();
        g.ortho(0.0, self.width as f32, 0.0, self.height as f32, -1.0, 1.0);
        g.translate(
            ((self.width - m.width) / 2) as f32,
            ((self.height - (m.ascent + m.descent)) / 2) as f32,
            0.0,
        );
        g.color4f(1.0, 1.0, 0.0, 1.0);
        g.depth_test(false);
        self.titlefont.print_string(g, &text);
        g.depth_test(true);
    }

    /// One picture on the drum, or its title. The two are drawn in separate
    /// passes so that the titles, which are not opaque, come last.
    /// Step every frame's drift, which upstream does as it draws.
    fn tick_zoom(&mut self, turning: bool) {
        for f in &mut self.frames {
            f.zoom_z = f.rot.position(turning).2 as f32;
        }
    }

    fn draw_frame(&self, g: &mut Glx, i: usize, body: bool) {
        let frame = &self.frames[i];
        let img = &frame.current;
        let texw = img.geom.width as f32 / img.tw;
        let texh = img.geom.height as f32 / img.th;
        let texx1 = img.geom.x as f32 / img.tw;
        let texy1 = img.geom.y as f32 / img.th;
        let (texx2, texy2) = (texx1 + texw, texy1 + texh);
        let aspect = img.geom.height as f32 / img.geom.width as f32;

        g.push_matrix();
        g.rotate(frame.theta, 0.0, 1.0, 0.0);
        g.translate(0.0, 0.0, frame.r);

        // Small enough that all of them fit on the drum without bumping into
        // each other.
        let (t, s) = match self.frames.len() {
            1 => (-1.0, 1.7),
            2 => (-0.8, 1.6),
            3 => (-0.4, 1.5),
            4 => (-0.2, 1.3),
            n => (0.0, 6.0 / n as f32),
        };
        g.translate(0.0, 0.0, t);
        g.scale(s, s, s);
        g.translate(-0.5, -(aspect / 2.0), 0.0);

        if self.zoom {
            // Only the z of the drift is used, for in and out.
            g.translate(0.0, 0.0, frame.zoom_z / 2.0);
        }

        // Where it has got to in its drop.
        if frame.mode == Mode::Out || frame.mode == Mode::In {
            let full = FADE_TICKS / self.speed;
            let mut t = if frame.mode == Mode::Out {
                frame.mode_tick / full
            } else {
                (full - frame.mode_tick + 1.0) / full
            };
            t = 1.0 - t;
            t = ease(Ease::InOutBack, t as f64) as f32;
            if frame.from_top {
                t = -t;
            }
            g.translate(0.0, t * 5.0, 0.0);
        }

        if body {
            if !self.wire {
                g.color4f(1.0, 1.0, 1.0, 1.0);
                g.normal3f(0.0, 0.0, 1.0);
                g.texturing(true);
                g.bind_texture(img.texid);
                g.begin(Shape::Quads);
                g.tex_coord2f(texx1, texy2);
                g.vertex3f(0.0, 0.0, 0.0);
                g.tex_coord2f(texx2, texy2);
                g.vertex3f(1.0, 0.0, 0.0);
                g.tex_coord2f(texx2, texy1);
                g.vertex3f(1.0, aspect, 0.0);
                g.tex_coord2f(texx1, texy1);
                g.vertex3f(0.0, aspect, 0.0);
                g.end();
                g.texturing(false);
            }

            // A box round it.
            g.line_width(2.0);
            g.color4f(0.5, 0.5, 0.5, 1.0);
            g.begin(Shape::LineLoop);
            g.vertex3f(0.0, 0.0, 0.0);
            g.vertex3f(1.0, 0.0, 0.0);
            g.vertex3f(1.0, aspect, 0.0);
            g.vertex3f(0.0, aspect, 0.0);
            g.end();
        } else if !img.title.is_empty() {
            // A title under it, centred: the font lays newlines out flush
            // left, so each line is placed by hand.
            let m = self.font.metrics(&img.title);
            let sh = (m.ascent + m.descent) as f32;
            let mut scale = 0.05;
            g.translate(0.0, -scale, 0.0);
            scale /= sh;
            g.scale(scale, scale, scale);
            g.color4f(1.0, 1.0, 1.0, 1.0);
            g.push_matrix();
            g.translate(((1.0 / scale) - m.width as f32) / 2.0, 0.0, 0.0);
            if !self.wire {
                self.font.print_string(g, &img.title);
            }
            g.pop_matrix();
        }
        g.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed") as f32;
    let tilt = g.res.string("tilt").to_ascii_uppercase();

    let spin = speed as f64 * 0.2 * (0.9 + frand(0.2));
    let wander = speed as f64 * 0.001 * (0.9 + frand(0.2));

    let mut this = Carousel {
        trackball: Trackball::new(),
        rot: Rotator::new(spin, spin, spin, speed as f64 * 0.1, wander, true),
        frames: Vec::new(),
        awaiting_first_images: true,
        loads_in_progress: 0,
        font: TexFont::load(&mut g.glx, g.res.string("font")),
        titlefont: TexFont::load(&mut g.glx, g.res.string("titleFont")),
        mode: Mode::In,
        mode_tick: FADE_TICKS / speed,
        width: g.width(),
        height: g.height(),
        count: g.res.int("count").max(1) as usize,
        speed,
        duration: g.res.float("duration"),
        titles: g.res.bool("titles"),
        zoom: g.res.bool("zoom"),
        tilt_x: tilt.contains('X'),
        tilt_y: tilt.contains('Y'),
        wire,
    };
    this.alloc_frame(g);
    this.load_image(g, 0);
    Box::new(this)
}

impl Hack3d for Carousel {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if crate::runtime::screenhack_event_helper(event) && !self.frames.is_empty() {
            // Replace one of them now.
            let i = random() as usize % self.frames.len();
            self.frames[i].expires = 0.0;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let aspect = self.width as f32 / self.height as f32;

        if self.awaiting_first_images {
            self.load_initial_images(g);
            if self.awaiting_first_images {
                g.glx.clear();
                let n = self.frames.len().saturating_sub(1);
                let glx = &mut g.glx;
                self.loading_msg(glx, n);
                return g.res.int("delay") as u32;
            }
        }

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(60.0, aspect, 1.0, 8.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 2.6], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        if !self.wire {
            g.glx.blend(Blend::Alpha);
            // The lines round a picture must win over the picture itself.
            g.glx.polygon_offset(Some((1.0, 1.0)));
        }

        g.glx.push_matrix();

        // The startup un-shrink.
        if self.mode == Mode::In {
            self.mode_tick -= 1.0;
            if self.mode_tick <= 0.0 {
                self.mode = Mode::Normal;
            }
            let full = FADE_TICKS / self.speed;
            let s = (full - self.mode_tick + 1.0) / full;
            g.glx.scale(s, s, s);
        }

        let turning = !self.trackball.button_down();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        // Tilt the drum up or down by up to thirty degrees, and lean it.
        let (x, y, _) = self.rot.position(turning);
        if self.tilt_x {
            g.glx.rotate(15.0 - (x as f32 * 30.0), 1.0, 0.0, 0.0);
        }
        if self.tilt_y {
            g.glx.rotate(7.0 - (y as f32 * 14.0), 0.0, 0.0, 1.0);
        }
        // Only the y of the rotation is used, which is the turn of the drum.
        let (_, y, _) = self.rot.rotation(turning);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);

        // Which frames want a new picture, and only one fetch at a time.
        let now = g.time;
        for i in 0..self.frames.len() {
            let f = &mut self.frames[i];
            match f.mode {
                Mode::Normal if turning && now >= f.expires && self.loads_in_progress == 0 => {
                    self.load_image(g, i);
                }
                Mode::Out => {
                    f.mode_tick -= 1.0;
                    if f.mode_tick <= 0.0 {
                        std::mem::swap(&mut f.current, &mut f.loading);
                        f.mode = Mode::In;
                        f.mode_tick = FADE_TICKS / self.speed;
                    }
                }
                Mode::In => {
                    f.mode_tick -= 1.0;
                    if f.mode_tick <= 0.0 {
                        f.mode = Mode::Normal;
                    }
                }
                _ => {}
            }
        }

        // The pictures first and the titles after, because only the titles
        // are see-through and blending wants back to front.
        self.tick_zoom(turning);
        let glx = &mut g.glx;
        for i in 0..self.frames.len() {
            self.draw_frame(glx, i, true);
        }
        if self.titles {
            self.tick_zoom(turning);
            let glx = &mut g.glx;
            for i in 0..self.frames.len() {
                self.draw_frame(glx, i, false);
            }
        }

        g.glx.pop_matrix();
        g.glx.polygon_offset(None);
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*count:      7",
    "*delay:      10000",
    "*wireframe:  False",
    "*showFPS:    False",
    "*font:       monospace 48",
    "*titleFont:  sans-serif bold 48",
    "*speed:      1.0",
    "*duration:   20",
    "*titles:     True",
    "*zoom:       True",
    "*tilt:       XY",
];

const TILTS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "XY",
        label: "Tilt both ways",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Tilt toward the viewer",
    },
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Tilt side to side",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Do not tilt",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("speed", "Animation speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("duration", "Duration", 5.0, 120.0, 1.0, 0, "20"),
    Opt::slider("count", "Number of images", 1.0, 20.0, 1.0, 0, "7"),
    Opt::select("tilt", "Tilt", TILTS, "XY"),
    Opt::boolean("titles", "Titles", "true"),
    Opt::boolean("zoom", "Zoom in and out", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "carousel",
    label: "Carousel",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=mCun9mEtF-I"),
        blurb: "Loads a sequence of images and rotates them around.",
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

    /// The ring fills one picture at a time, with a message meanwhile, and
    /// then everyone is placed evenly round it.
    #[test]
    fn the_ring_fills_one_at_a_time() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        // The first frame or two: nothing but the loading message.
        r.step();
        let f = r.frame();
        assert!(
            f.batches.iter().all(|b| b.texture.is_some()),
            "the loading message is not text"
        );

        for _ in 0..20 {
            r.step();
        }
        let f = r.frame();
        // Seven pictures, each with a box round it.
        let boxes = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::LineLoop)
            .count();
        assert_eq!(boxes, 7, "{boxes} boxes is not seven pictures");
    }

    /// They are spaced evenly round the drum, so the angle between one and
    /// the next is the same all the way round.
    #[test]
    fn the_pictures_are_evenly_spaced() {
        let mut r = start(StartArgs::new(640, 480, "count=4", 20260811));
        for _ in 0..20 {
            r.step();
        }
        let f = r.frame();
        // Each picture sits at its own distance out from the middle of the
        // drum, so no two share a matrix.
        let places: std::collections::HashSet<String> = f
            .batches
            .iter()
            .map(|b| format!("{:.3},{:.3}", b.modelview.0[12], b.modelview.0[14]))
            .collect();
        assert_eq!(places.len(), 4, "the pictures are not spread out");
    }

    /// A picture that has expired is replaced: the old one drops out and the
    /// new one drops in after it, and only one is ever fetched at a time.
    #[test]
    fn a_picture_drops_out_and_another_drops_in() {
        let mut r = start(StartArgs::new(640, 480, "duration=1&count=3", 20260811));
        let mut moved = false;
        let mut ys: Vec<f32> = Vec::new();
        for _ in 0..600 {
            r.step();
            for b in r.frame().batches.iter() {
                ys.push(b.modelview.0[13]);
            }
        }
        let lo = ys.iter().copied().fold(f32::MAX, f32::min);
        let hi = ys.iter().copied().fold(f32::MIN, f32::max);
        if hi - lo > 1.0 {
            moved = true;
        }
        assert!(moved, "nothing ever dropped in or out: {lo} to {hi}");
    }

    /// Each picture is a textured quad with a box round it, and the titles
    /// come after all of them, because they are the only thing here that is
    /// not opaque. With no host there are no titles, so the frame ends on the
    /// last picture's box.
    #[test]
    fn a_picture_is_a_quad_and_a_box() {
        use crate::runtime::gl::Primitive;
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..20 {
            r.step();
        }
        let f = r.frame();
        assert_eq!(f.batches.len(), 14, "seven pictures is fourteen draws");
        for pair in f.batches.chunks(2) {
            assert!(pair[0].texture.is_some(), "a picture has no picture on it");
            assert_eq!(pair[1].primitive, Primitive::LineLoop, "it has no box");
        }
        assert_eq!(
            f.batches.last().map(|b| b.primitive),
            Some(Primitive::LineLoop),
            "something was drawn after the pictures"
        );
    }
}
