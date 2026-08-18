//! Port of `hacks/glx/esper.c`.
//!
//! ```text
//! esper, Copyright © 2017-2025 Jamie Zawinski <jwz@jwz.org>
//! Enhance 224 to 176. Pull out track right. Center in pull back.
//! Pull back. Wait a minute. Go right. Stop. Enhance 57 19. Track 45 left.
//! Gimme a hardcopy right there.
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
//! The Esper machine from *Blade Runner*: a photograph enhanced past all
//! reason, under a blue grid.
//!
//! A reticle appears in the middle, jumps in discrete steps to somewhere else
//! on the picture, and a box zooms out from the middle to land around it. Then
//! the picture itself moves so that what was inside the box fills the screen,
//! and the whole thing starts again, deeper in. When it has zoomed twenty
//! times its own size it gives up and loads another photograph.
//!
//! Nothing here animates smoothly. Every move is a queue of copies of the same
//! sprite, each with a longer pause before it starts and a short life, so what
//! is seen is a jerky march of stills rather than a slide, with a flash of
//! blue over each step. The pause is what does the work: a sprite that has not
//! reached its own start time simply draws nothing yet.
//!
//! The text under the picture is upstream's joke, faithfully kept: it reads
//! out coordinates that have almost nothing to do with what is on screen,
//! exactly as the numbers in the film do not.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

/// Where a sprite is and how big, in fractions of the window.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Which of the six things a sprite is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Image,
    Reticle,
    Box,
    Grid,
    Flash,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpriteState {
    New,
    In,
    Full,
    Out,
    Dead,
}

/// What the machine as a whole is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Anim {
    Blank,
    GridOn,
    ImageLoad,
    ImageUnload,
    Reticle,
    ReticleMove,
    BoxMove,
    ImageZoom,
}

struct Image {
    id: u32,
    tw: f64,
    th: f64,
    geom: (f64, f64, f64, f64),
    title: String,
    texid: Option<u32>,
    refcount: i32,
}

#[derive(Clone)]
struct Sprite {
    id: u32,
    kind: Kind,
    img_id: Option<u32>,
    /// For a text sprite, which other sprite it is reporting on.
    text_id: u32,
    text: String,
    opacity: f64,
    /// How much fatter the lines are drawn right now, which is the pulse each
    /// sprite gives as it reaches full opacity.
    thickness_scale: f64,
    throb: bool,
    start_time: f64,
    /// How long it stays at full opacity. Zero means for ever.
    duration: f64,
    fade_duration: f64,
    /// How long before it starts fading in at all. This is what staggers a
    /// queue of copies into a march of discrete steps.
    pause_duration: f64,
    /// Whether it waits for ever before fading out.
    remain: bool,
    from: Rect,
    to: Rect,
    current: Rect,
    state: SpriteState,
    /// Whether the picture is drawn with hard pixel edges, which is what makes
    /// a mid-zoom step look like a blown-up photograph.
    fatbits: bool,
    /// A box that is zooming out rather than in.
    back: bool,
}

impl Sprite {
    fn new(id: u32, kind: Kind, now: f64) -> Self {
        Sprite {
            id,
            kind,
            img_id: None,
            text_id: 0,
            text: String::new(),
            opacity: 0.0,
            thickness_scale: 1.0,
            throb: true,
            start_time: now,
            duration: 0.0,
            fade_duration: 0.0,
            pause_duration: 0.0,
            remain: false,
            from: Rect {
                x: 0.5,
                y: 0.5,
                w: 1.0,
                h: 1.0,
            },
            to: Rect {
                x: 0.5,
                y: 0.5,
                w: 1.0,
                h: 1.0,
            },
            current: Rect {
                x: 0.5,
                y: 0.5,
                w: 1.0,
                h: 1.0,
            },
            state: SpriteState::New,
            fatbits: false,
            back: false,
        }
    }
}

/// Three samples averaged, so the middle of the range comes up most often.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

fn randsign() -> f64 {
    if random() & 1 == 1 { 1.0 } else { -1.0 }
}

struct Esper {
    images: Vec<Image>,
    sprites: Vec<Sprite>,
    now: f64,

    font: TexFont,
    sprite_id: u32,
    image_id: u32,

    grid_color: [f32; 4],
    reticle_color: [f32; 4],
    text_color: [f32; 4],

    anim: Anim,
    anim_start: f64,
    anim_duration: f64,

    grid_size: i32,
    grid_thickness: f64,
    do_titles: bool,
    speed: f64,
}

impl Esper {
    fn image(&self, id: u32) -> Option<&Image> {
        self.images.iter().find(|i| i.id == id)
    }

    /// `alloc_image`, `get_image`: one picture, kept until nothing refers to
    /// it. Upstream loads them in the background over several frames; here a
    /// picture either is or is not ready.
    fn get_image(&mut self, g: &mut Gl) -> Option<u32> {
        if let Some(i) = self.images.first() {
            return Some(i.id);
        }
        let (w, h) = (g.width(), g.height());
        let img = g.load_image(w, h)?;
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(img.width, img.height, img.pixels);
        g.glx.tex_clamp(true);

        self.image_id += 1;
        let geom = img.geometry;
        self.images.push(Image {
            id: self.image_id,
            tw: f64::from(img.width),
            th: f64::from(img.height),
            geom: (
                f64::from(geom.x),
                f64::from(geom.y),
                f64::from(geom.width.max(1)),
                f64::from(geom.height.max(1)),
            ),
            title: img.title.unwrap_or_default(),
            texid: Some(id),
            refcount: 0,
        });
        Some(self.image_id)
    }

    /// `new_sprite`. Returns the index of the new sprite, or `None` if it
    /// wanted a picture and there is not one yet.
    fn new_sprite(&mut self, g: &mut Gl, kind: Kind) -> Option<usize> {
        let img_id = if kind == Kind::Image {
            Some(self.get_image(g)?)
        } else {
            None
        };

        self.sprite_id += 1;
        let mut sp = Sprite::new(self.sprite_id, kind, self.now);

        if let Some(id) = img_id {
            sp.img_id = Some(id);
            sp.duration = 0.0; /* forever, until further notice */
            sp.fade_duration = 0.5;

            let (w, h) = (f64::from(g.width()), f64::from(g.height()));
            let img = self.image(id).expect("just fetched");
            // Scale the sprite so that the picture fills the window, then
            // pan to a random spot on whichever way it overflows.
            let r = (img.geom.3 / img.geom.2) * (w / h);
            if r > 1.0 {
                sp.to.h *= r;
            } else {
                sp.to.w /= r;
            }
            if sp.to.h > 1.0 {
                sp.to.y += frand((sp.to.h - 1.0) / 2.0) * randsign();
            }
            if sp.to.w > 1.0 {
                sp.to.x += frand((sp.to.w - 1.0) / 2.0) * randsign();
            }

            if let Some(m) = self.images.iter_mut().find(|m| m.id == id) {
                m.refcount += 1;
            }
        }

        sp.from = sp.to;
        sp.current = sp.to;
        self.sprites.push(sp);
        Some(self.sprites.len() - 1)
    }

    /// `copy_sprite`: the same sprite again, with a fresh identity and clock.
    fn copy_sprite(&mut self, from: usize) -> usize {
        self.sprite_id += 1;
        let mut sp = self.sprites[from].clone();
        sp.id = self.sprite_id;
        sp.state = SpriteState::New;
        sp.start_time = self.now;
        if let Some(id) = sp.img_id
            && let Some(m) = self.images.iter_mut().find(|m| m.id == id)
        {
            m.refcount += 1;
        }
        self.sprites.push(sp);
        self.sprites.len() - 1
    }

    /// `find_newest_sprite`: the most recently started sprite of a kind that
    /// has got past its own pause.
    fn find_newest(&self, kind: Kind) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, sp) in self.sprites.iter().enumerate() {
            if sp.kind != kind {
                continue;
            }
            let ok = match best {
                None => true,
                Some(b) => {
                    self.sprites[b].start_time < sp.start_time
                        && self.now >= sp.start_time + sp.pause_duration
                }
            };
            if ok {
                best = Some(i);
            }
        }
        best
    }

    /// `fadeout_sprite`: pretend it has reached the point where it should go.
    fn fadeout_sprite(&mut self, i: usize) {
        let now = self.now;
        let sp = &mut self.sprites[i];
        // If it has not faded in yet, do not fade out either.
        if now <= sp.start_time + sp.pause_duration {
            sp.fade_duration = 0.0;
        }
        sp.pause_duration = 0.0;
        sp.duration = 9999.0;
        sp.remain = false;
        sp.start_time = now - sp.duration;
    }

    fn fadeout_all(&mut self, kind: Kind) {
        for i in 0..self.sprites.len() {
            if self.sprites[i].kind == kind {
                self.fadeout_sprite(i);
            }
        }
    }

    /// `push_text_sprite`: a readout that lives and dies with another sprite.
    fn push_text_sprite(&mut self, g: &mut Gl, of: usize) -> usize {
        let (id, fade, dur, pause) = {
            let sp = &self.sprites[of];
            (sp.id, sp.fade_duration, sp.duration, sp.pause_duration)
        };
        let i = self
            .new_sprite(g, Kind::Text)
            .expect("a text sprite needs no picture");
        let sp = &mut self.sprites[i];
        sp.text_id = id;
        sp.fade_duration = fade;
        sp.duration = dur;
        sp.pause_duration = pause;
        i
    }

    /// `push_flash_sprite`: the blue solarisation over one step of a zoom.
    fn push_flash_sprite(&mut self, g: &mut Gl, of: usize) {
        let (id, fade, pause) = {
            let sp = &self.sprites[of];
            (sp.id, sp.fade_duration, sp.pause_duration)
        };
        let Some(i) = self.new_sprite(g, Kind::Flash) else {
            return;
        };
        let sp = &mut self.sprites[i];
        sp.text_id = id;
        sp.duration = (0.07 / self.speed).max(0.07);
        // Fading these is too fast to see.
        sp.fade_duration = 0.0;
        sp.pause_duration = pause + fade * 0.3;
    }

    /// `compute_sprite_duration`: how long a move takes, from how far the
    /// furthest corner has to travel.
    fn compute_sprite_duration(&mut self, i: usize, blink: bool) {
        let sp = &self.sprites[i];
        let l = |r: Rect| r.x - r.w / 2.0;
        let rr = |r: Rect| 1.0 - (r.x + r.w / 2.0);
        let b = |r: Rect| r.y - r.h / 2.0;
        let t = |r: Rect| 1.0 - (r.y + r.h / 2.0);
        let d = |f: f64, g: f64| (f * f + g * g).sqrt();

        let bl = d(b(sp.from), l(sp.to));
        let br = d(b(sp.from), rr(sp.to));
        let tl = d(t(sp.from), l(sp.to));
        let tr = d(t(sp.from), rr(sp.to));
        let cx = sp.to.x - sp.from.x;
        let cy = sp.to.y - sp.from.y;
        let c = (cx * cx + cy * cy).sqrt();
        let dist = bl.max(br).max(tl).max(tr).max(c);

        let steps = (1.0 + dist * 28.0).min(10.0);
        let mut duration = steps * 0.2 / self.speed;
        // For the linger that `animate_sprite_path` adds.
        duration += 1.5 / self.speed;
        if blink {
            duration += 0.6 / self.speed;
        }
        self.sprites[i].duration = duration;
    }

    /// `animate_sprite_path`: turn a smooth move into a march of stills.
    ///
    /// The sprite is replaced by a queue of copies, each parked at one step
    /// along the way and each with a longer pause before it appears. Only one
    /// is visible at a time, so the thing appears to jump.
    fn animate_sprite_path(&mut self, g: &mut Gl, i: usize, blink: bool) {
        let (from, to, kind, dur0, remain) = {
            let sp = &self.sprites[i];
            (sp.from, sp.to, sp.kind, sp.duration, sp.remain)
        };
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let dw = to.w - from.w;
        let dh = to.h - from.h;
        let mut linger = 1.5 / self.speed;
        let mut blinger = 0.6 / self.speed;
        let dur = dur0 - linger - if blink { blinger } else { 0.0 };

        let mut steps = (dur / 0.3 * self.speed) as i32;
        if kind == Kind::Image {
            steps = (f64::from(steps) * 0.8) as i32;
        }
        steps = steps.clamp(2, 10);
        if dur < 0.01 {
            linger = 0.0;
            blinger = 0.0;
        }

        let current = self.sprites[i].current;
        for k in 0..=steps {
            let j = self.copy_sprite(i);
            let f = f64::from(k) / f64::from(steps);
            let r = Rect {
                x: current.x + f * dx,
                y: current.y + f * dy,
                w: current.w + f * dw,
                h: current.h + f * dh,
            };
            {
                let sp = &mut self.sprites[j];
                sp.to = r;
                sp.from = r;
                sp.current = r;
                sp.duration = dur / f64::from(steps);
                sp.pause_duration += f64::from(k) * sp.duration;
                sp.remain = false;
                sp.fatbits = true;
                if k == steps {
                    // The last one lingers for a bit.
                    sp.duration += linger;
                    if !blink {
                        sp.remain = remain;
                        sp.fatbits = false;
                    }
                }
            }
            if kind == Kind::Image && k > 0 {
                self.push_flash_sprite(g, j);
            }
            if kind == Kind::Reticle || kind == Kind::Box {
                let t = self.push_text_sprite(g, j);
                if k == steps {
                    self.sprites[t].duration += linger * 2.0;
                }
            }
        }

        if blink && blinger > 0.0 {
            // The last one blinks before vanishing.
            let blinkers = 3;
            for k in 1..=blinkers {
                let j = self.copy_sprite(i);
                let sp = &mut self.sprites[j];
                sp.current = to;
                sp.from = to;
                sp.duration = blinger / f64::from(blinkers);
                sp.pause_duration += dur + linger + f64::from(k) * sp.duration;
                sp.remain = false;
                if k == blinkers {
                    sp.remain = remain;
                    sp.fatbits = false;
                }
            }
        }

        // Fade out the sprite the queue was made from. It may never even have
        // appeared.
        self.fadeout_sprite(i);
    }

    /// `tick_sprite`: where one sprite is in its life.
    ///
    /// ```text
    ///          pause        fade  duration        fade
    ///   |------------|------------|---------|-----------|
    ///                 ....----====##########====----....
    ///             from             current            to
    /// ```
    fn tick_sprite(&mut self, i: usize) {
        let now = self.now;
        let sp = &mut self.sprites[i];
        let visible = sp.duration + sp.fade_duration * 2.0;
        let total = sp.pause_duration + visible;
        let secs = now - sp.start_time;

        let ratio = if visible <= 0.0 {
            1.0
        } else {
            ((secs - sp.pause_duration) / visible).clamp(0.0, 1.0)
        };
        sp.current.x = sp.from.x + ratio * (sp.to.x - sp.from.x);
        sp.current.y = sp.from.y + ratio * (sp.to.y - sp.from.y);
        sp.current.w = sp.from.w + ratio * (sp.to.w - sp.from.w);
        sp.current.h = sp.from.h + ratio * (sp.to.h - sp.from.h);
        sp.thickness_scale = 1.0;

        if secs < sp.pause_duration {
            sp.state = SpriteState::In;
            sp.opacity = 0.0;
        } else if secs < sp.pause_duration + sp.fade_duration {
            sp.state = SpriteState::In;
            sp.opacity = (secs - sp.pause_duration) / sp.fade_duration;
        } else if sp.duration == 0.0
            || sp.remain
            || secs < sp.pause_duration + sp.fade_duration + sp.duration
        {
            sp.state = SpriteState::Full;
            sp.opacity = 1.0;
            // Just after reaching full opacity, pulse the width up and down.
            if sp.fade_duration > 0.0 && secs < sp.pause_duration + sp.fade_duration * 2.0 {
                let f = (secs - (sp.pause_duration + sp.fade_duration)) / sp.fade_duration;
                if sp.throb {
                    sp.thickness_scale = 1.0 + 3.0 * if f > 0.5 { 1.0 - f } else { f };
                }
            }
        } else if secs < total {
            sp.state = SpriteState::Out;
            sp.opacity = (total - secs) / sp.fade_duration;
        } else {
            sp.state = SpriteState::Dead;
            sp.opacity = 0.0;
        }
    }

    fn tick_sprites(&mut self) {
        for i in 0..self.sprites.len() {
            self.tick_sprite(i);
        }
        let mut i = 0;
        while i < self.sprites.len() {
            if self.sprites[i].state == SpriteState::Dead {
                if let Some(id) = self.sprites[i].img_id
                    && let Some(m) = self.images.iter_mut().find(|m| m.id == id)
                {
                    m.refcount -= 1;
                }
                self.sprites.remove(i);
            } else {
                i += 1;
            }
        }
        self.images.retain(|m| m.refcount > 0);
    }

    /// `compute_image_rect`: given the box, where the picture has to go so
    /// that only what is inside the box is on screen.
    fn compute_image_rect(r: &mut Rect, img: Rect, inverse: bool) {
        let scale = if inverse { 1.0 / r.w } else { r.w };
        let mut dx = r.x - 0.5;
        let mut dy = r.y - 0.5;

        r.w = img.w / scale;
        r.h = img.h / scale;
        r.x = 0.5 + (img.x - 0.5) / scale;
        r.y = 0.5 + (img.y - 0.5) / scale;

        if inverse {
            // Upstream marks this as close but not quite right.
            dx = -dx;
            dy = -dy;
        }
        r.x -= dx / scale;
        r.y -= dy / scale;
    }

    /// `track_box_with_image`: aim the picture at the box, without letting it
    /// zoom out past the frame or pan off its own edge.
    fn track_box_with_image(&mut self, box_i: usize, img_i: usize) {
        let mut r = self.sprites[box_i].current;
        let back = self.sprites[box_i].back;
        Self::compute_image_rect(&mut r, self.sprites[img_i].current, back);

        if r.w < 1.0 && r.h < 1.0 {
            if r.w > r.h {
                r.w /= r.h;
                r.h = 1.0;
            } else {
                r.h /= r.w;
                r.w = 1.0;
            }
        }
        // Not a clamp: when the picture is narrower than the frame on one
        // axis the two bounds cross over, and upstream's sequential tests
        // leave the second one winning rather than failing.
        if r.x < -r.w / 2.0 + 1.0 {
            r.x = -r.w / 2.0 + 1.0;
        }
        if r.x > r.w / 2.0 {
            r.x = r.w / 2.0;
        }
        if r.y < -r.h / 2.0 + 1.0 {
            r.y = -r.h / 2.0 + 1.0;
        }
        if r.y > r.h / 2.0 {
            r.y = r.h / 2.0;
        }
        self.sprites[img_i].to = r;
    }
}

impl Esper {
    /// `tick_animation`: the state machine. Each state decides what comes
    /// next, then sets up the sprites for it.
    fn tick_animation(&mut self, g: &mut Gl) {
        self.anim = match self.anim {
            Anim::Blank => Anim::GridOn,
            Anim::GridOn => Anim::ImageLoad,
            // Only advance once a picture has arrived.
            Anim::ImageLoad => {
                if self.find_newest(Kind::Image).is_some() {
                    Anim::Reticle
                } else {
                    Anim::ImageLoad
                }
            }
            Anim::Reticle => Anim::ReticleMove,
            // Most of the time a box follows the reticle; now and then the
            // picture jumps straight to it.
            Anim::ReticleMove => {
                if !random().is_multiple_of(6) {
                    Anim::BoxMove
                } else {
                    Anim::ImageZoom
                }
            }
            Anim::BoxMove => Anim::ImageZoom,
            Anim::ImageZoom => {
                let depth = self.find_newest(Kind::Image).map_or(0.0, |i| {
                    self.sprites[i].current.w.min(self.sprites[i].current.h)
                });
                if depth > 20.0 {
                    Anim::ImageUnload
                } else {
                    Anim::Reticle
                }
            }
            Anim::ImageUnload => Anim::ImageLoad,
        };

        self.anim_start = self.now;
        self.anim_duration = 0.0;
        let speed = self.speed;

        match self.anim {
            Anim::Blank => {}

            Anim::GridOn => {
                // The blue grid over everything, which never goes away.
                if self.find_newest(Kind::Grid).is_none()
                    && let Some(i) = self.new_sprite(g, Kind::Grid)
                {
                    let sp = &mut self.sprites[i];
                    sp.fade_duration = 1.0 / speed;
                    sp.duration = 2.0 / speed;
                    sp.remain = true;
                    self.anim_duration = sp.pause_duration + sp.fade_duration * 2.0 + sp.duration;
                }
            }

            Anim::ImageLoad => {
                self.fadeout_all(Kind::Image);
                let Some(i) = self.new_sprite(g, Kind::Image) else {
                    return;
                };
                {
                    let sp = &mut self.sprites[i];
                    sp.fade_duration = 0.5 / speed;
                    sp.duration = sp.fade_duration * 3.0;
                    sp.remain = true;
                    sp.current = sp.from;
                    self.anim_duration = sp.pause_duration + sp.fade_duration * 2.0 + sp.duration;
                }
                let t = self.push_text_sprite(g, i);
                let sp = &mut self.sprites[t];
                sp.fade_duration = 0.2 / speed;
                sp.pause_duration = 0.0;
                sp.duration = 2.5 / speed;
            }

            Anim::ImageUnload => {
                let fade = 3.0 / speed;
                if let Some(i) = self.find_newest(Kind::Image) {
                    self.sprites[i].fade_duration = fade;
                }
                for k in [Kind::Image, Kind::Reticle, Kind::Box, Kind::Text] {
                    self.fadeout_all(k);
                }
                self.anim_duration = fade + 3.5 / speed;
            }

            Anim::Reticle => {
                // The crosshair, in the middle.
                self.fadeout_all(Kind::Text);
                let Some(i) = self.new_sprite(g, Kind::Reticle) else {
                    return;
                };
                let sp = &mut self.sprites[i];
                sp.fade_duration = 0.2 / speed;
                sp.pause_duration = 1.0 / speed;
                sp.duration = 1.5 / speed;
                self.anim_duration = sp.pause_duration + sp.duration;
            }

            Anim::ReticleMove => {
                // Move it somewhere else, but not somewhere too near.
                let (ox, oy) = (0.5, 0.5);
                let (mut nx, mut ny);
                loop {
                    nx = 0.3 + bellrand(0.4);
                    ny = 0.3 + bellrand(0.4);
                    let d = ((nx - ox) * (nx - ox) + (ny - oy) * (ny - oy)).sqrt();
                    if d >= 0.1 {
                        break;
                    }
                }

                let Some(i) = self.new_sprite(g, Kind::Reticle) else {
                    return;
                };
                {
                    let sp = &mut self.sprites[i];
                    sp.from.x = ox;
                    sp.from.y = oy;
                    sp.to = sp.from;
                    sp.current = sp.from;
                    sp.to.x = nx;
                    sp.to.y = ny;
                    sp.fade_duration = 0.2 / speed;
                    sp.pause_duration = 0.0;
                }
                self.compute_sprite_duration(i, false);
                let sp = &self.sprites[i];
                self.anim_duration = sp.pause_duration + sp.fade_duration * 2.0 + sp.duration - 0.1;
                self.animate_sprite_path(g, i, false);
            }

            Anim::BoxMove => {
                // The box zooms out from the middle to land round the
                // reticle.
                let (nx, ny) = self
                    .sprites
                    .iter()
                    .filter(|s| s.kind == Kind::Reticle)
                    .max_by(|a, b| a.start_time.total_cmp(&b.start_time))
                    .map_or((0.5, 0.5), |s| (s.to.x, s.to.y));

                let mut z = 0.3 + frand(0.5);
                // Keep the box on screen.
                let margin = 0.005;
                let maxw = 2.0 * (1.0 - margin - nx).min(nx - margin);
                let maxh = 2.0 * (1.0 - margin - ny).min(ny - margin);
                z = z.min(maxw.min(maxh));

                let Some(i) = self.new_sprite(g, Kind::Box) else {
                    return;
                };
                {
                    let sp = &mut self.sprites[i];
                    sp.from = Rect {
                        x: 0.5,
                        y: 0.5,
                        w: 1.0,
                        h: 1.0,
                    };
                    sp.current = sp.from;
                    sp.to = Rect {
                        x: nx,
                        y: ny,
                        w: z,
                        h: z,
                    };
                }

                // Sometimes zoom back out instead of in, the more likely the
                // deeper it already is.
                if let Some(img) = self.find_newest(Kind::Image) {
                    let depth = self.sprites[img].current.w.min(self.sprites[img].current.h);
                    let out = if depth < 6.0 {
                        random().is_multiple_of(5)
                    } else if depth < 12.0 {
                        random().is_multiple_of(2)
                    } else {
                        !random().is_multiple_of(3)
                    };
                    if depth > 1.0 && out {
                        self.sprites[i].back = true;
                        // Do not zoom out much past life size.
                        if depth < 1.5 && z < 0.8 {
                            self.sprites[i].to.w = 0.8;
                            self.sprites[i].to.h = 0.8;
                        }
                    }
                }

                {
                    let sp = &mut self.sprites[i];
                    sp.fade_duration = 0.2 / speed;
                    sp.pause_duration = 2.0 / speed;
                }
                self.compute_sprite_duration(i, true);
                let sp = &self.sprites[i];
                self.anim_duration = sp.pause_duration + sp.fade_duration * 2.0 + sp.duration - 0.1;
                self.animate_sprite_path(g, i, true);
            }

            Anim::ImageZoom => {
                // Move the picture so that what was inside the box fills the
                // screen.
                let Some(target) = self
                    .find_newest(Kind::Box)
                    .or_else(|| self.find_newest(Kind::Reticle))
                else {
                    return;
                };
                let Some(img) = self.find_newest(Kind::Image) else {
                    return;
                };

                let j = self.copy_sprite(img);
                self.sprites[j].from = self.sprites[img].current;
                self.fadeout_sprite(img);
                self.track_box_with_image(target, j);

                {
                    let sp = &mut self.sprites[j];
                    sp.fade_duration = 0.2 / speed;
                    sp.pause_duration = 0.5 / speed;
                    sp.remain = true;
                    sp.throb = false;
                }
                self.compute_sprite_duration(j, false);
                let pause = self.sprites[j].pause_duration;
                self.sprites[img].start_time += pause;

                let sp = &self.sprites[j];
                self.anim_duration = sp.pause_duration + sp.fade_duration * 2.0 + sp.duration;
                self.animate_sprite_path(g, j, false);
                self.fadeout_all(Kind::Text);
            }
        }
    }

    /// `draw_image_sprite`: the photograph itself.
    fn draw_image_sprite(&self, g: &mut Gl, sp: &Sprite) {
        let Some(img) = sp.img_id.and_then(|id| self.image(id)) else {
            return;
        };
        let Some(texid) = img.texid else { return };

        g.glx.push_matrix();
        // The pulse, scaled right down: a photograph that throbbed as much as
        // a line would look silly.
        let s = (1.0 + (sp.thickness_scale - 1.0) / 40.0) as f32;
        g.glx.translate(0.5, 0.5, 0.0);
        g.glx.scale(s, s, 1.0);
        g.glx.translate(-0.5, -0.5, 0.0);

        g.glx
            .translate(sp.current.x as f32, sp.current.y as f32, 0.0);
        g.glx.scale(sp.current.w as f32, sp.current.h as f32, 1.0);
        g.glx.translate(-0.5, -0.5, 0.0);

        let texw = img.geom.2 / img.tw;
        let texh = img.geom.3 / img.th;
        let texx1 = img.geom.0 / img.tw;
        let texy1 = img.geom.1 / img.th;
        let (texx2, texy2) = (texx1 + texw, texy1 + texh);

        g.glx.texturing(true);
        g.glx.bind_texture(texid);
        // Mid-zoom steps are drawn with hard pixel edges, which is what makes
        // them read as a blown-up photograph rather than a smooth zoom.
        g.glx.tex_nearest(sp.fatbits);
        g.glx.color4f(1.0, 1.0, 1.0, sp.opacity as f32);
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.begin(Shape::Quads);
        for (u, v, x, y) in [
            (texx1, texy2, 0.0, 0.0),
            (texx2, texy2, 1.0, 0.0),
            (texx2, texy1, 1.0, 1.0),
            (texx1, texy1, 0.0, 1.0),
        ] {
            g.glx.tex_coord2f(u as f32, v as f32);
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();
        g.glx.tex_nearest(false);
        g.glx.texturing(false);
        g.glx.pop_matrix();
    }

    /// `draw_line_sprite`: the grid, the reticle and the box, all drawn as
    /// solid quads in window coordinates.
    ///
    /// Each is drawn several times at shrinking thickness and a fraction of
    /// the opacity, which is upstream's way of getting a soft edge out of
    /// hard-edged quads.
    fn draw_line_sprite(&self, g: &mut Gl, sp: &Sprite) {
        let w = f64::from(g.width());
        let h = f64::from(g.height());
        let wh = w.max(h);
        let gs = if sp.kind == Kind::Reticle {
            self.grid_size + 1
        } else {
            self.grid_size
        };
        let sx = (wh / f64::from(gs + 1)).max(10.0);
        let sy = sx;

        let mut t = self.grid_thickness * sp.thickness_scale;
        t = t.min(sx / 3.0).max(1.0);
        let fade = (t as i32).max(1);
        if sp.opacity <= 0.0 {
            return;
        }

        let x = w * sp.current.x;
        let y = h * sp.current.y;
        let bw = w * sp.current.w;
        let bh = h * sp.current.h;

        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.ortho(0.0, w as f32, 0.0, h as f32, -1.0, 1.0);

        let mut color = match sp.kind {
            Kind::Grid => self.grid_color,
            _ => self.reticle_color,
        };

        if sp.kind == Kind::Grid {
            let s = (1.0 + (sp.thickness_scale - 1.0) / 120.0) as f32;
            g.glx.translate((w / 2.0) as f32, (h / 2.0) as f32, 0.0);
            g.glx.scale(s, s, 1.0);
            g.glx.translate(-(w / 2.0) as f32, -(h / 2.0) as f32, 0.0);
        }

        g.glx.texturing(false);
        for k in 0..fade {
            let t2 = t * (1.0 - f64::from(k) / f64::from(fade));
            if t2 <= 0.0 {
                break;
            }
            color[3] = (sp.opacity / f64::from(fade)) as f32;
            g.glx.color4f(color[0], color[1], color[2], color[3]);
            g.glx.begin(Shape::Quads);

            let mut quad = |a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)| {
                for p in [a, b, c, d] {
                    g.glx.vertex3f(p.0 as f32, p.1 as f32, 0.0);
                }
            };

            match sp.kind {
                Kind::Grid => {
                    let xoff = (w - sx * (w / sx).floor()) / 2.0;
                    let yoff = (h - sy * (h / sy).floor()) / 2.0;
                    let mut gy = -sy / 2.0 + t2 / 2.0;
                    while gy < h {
                        let mut gx = -sx / 2.0 - t2 / 2.0;
                        while gx < w {
                            quad(
                                (xoff + gx + t2, yoff + gy),
                                (xoff + gx + t2, yoff + gy + sy - t2),
                                (xoff + gx, yoff + gy + sy - t2),
                                (xoff + gx, yoff + gy),
                            );
                            quad(
                                (xoff + gx, yoff + gy - t2),
                                (xoff + gx + sx, yoff + gy - t2),
                                (xoff + gx + sx, yoff + gy),
                                (xoff + gx, yoff + gy),
                            );
                            gx += sx;
                        }
                        gy += sy;
                    }
                }
                Kind::Box => {
                    let (l, r) = (x - bw / 2.0, x + bw / 2.0);
                    let (b, u) = (y - bh / 2.0, y + bh / 2.0);
                    quad(
                        (l - t2 / 2.0, b - t2 / 2.0),
                        (r + t2 / 2.0, b - t2 / 2.0),
                        (r + t2 / 2.0, b + t2 / 2.0),
                        (l - t2 / 2.0, b + t2 / 2.0),
                    );
                    quad(
                        (l - t2 / 2.0, u - t2 / 2.0),
                        (r + t2 / 2.0, u - t2 / 2.0),
                        (r + t2 / 2.0, u + t2 / 2.0),
                        (l - t2 / 2.0, u + t2 / 2.0),
                    );
                    quad(
                        (l + t2 / 2.0, b + t2 / 2.0),
                        (l + t2 / 2.0, u - t2 / 2.0),
                        (l - t2 / 2.0, u - t2 / 2.0),
                        (l - t2 / 2.0, b + t2 / 2.0),
                    );
                    quad(
                        (r + t2 / 2.0, b + t2 / 2.0),
                        (r + t2 / 2.0, u - t2 / 2.0),
                        (r - t2 / 2.0, u - t2 / 2.0),
                        (r - t2 / 2.0, b + t2 / 2.0),
                    );
                }
                _ => {
                    // The reticle: four bars pointing in at the middle, with
                    // a gap of one grid square around it.
                    quad(
                        (x + t2 / 2.0, y + sy / 2.0 - t2 / 2.0),
                        (x + t2 / 2.0, h),
                        (x - t2 / 2.0, h),
                        (x - t2 / 2.0, y + sy / 2.0 - t2 / 2.0),
                    );
                    quad(
                        (x - t2 / 2.0, y - sy / 2.0 + t2 / 2.0),
                        (x - t2 / 2.0, 0.0),
                        (x + t2 / 2.0, 0.0),
                        (x + t2 / 2.0, y - sy / 2.0 + t2 / 2.0),
                    );
                    quad(
                        (x - sx / 2.0 + t2 / 2.0, y + t2 / 2.0),
                        (0.0, y + t2 / 2.0),
                        (0.0, y - t2 / 2.0),
                        (x - sx / 2.0 + t2 / 2.0, y - t2 / 2.0),
                    );
                    quad(
                        (x + sx / 2.0 - t2 / 2.0, y - t2 / 2.0),
                        (w, y - t2 / 2.0),
                        (w, y + t2 / 2.0),
                        (x + sx / 2.0 - t2 / 2.0, y + t2 / 2.0),
                    );
                }
            }
            g.glx.end();
        }
        g.glx.pop_matrix();
    }

    /// `draw_flash_sprite`: one frame of blue over everything.
    ///
    /// Only the blue and alpha channels are written, so what is underneath
    /// keeps its reds and greens and is tinted rather than covered. That is
    /// the solarisation the film's zoom steps flash with.
    fn draw_flash_sprite(&self, g: &mut Gl) {
        g.glx.push_matrix();
        g.glx.texturing(false);
        // Too fast to see, so keep it consistent.
        g.glx.color4f(0.0, 0.0, 1.0, 0.7);
        g.glx.color_mask_rgba([false, false, true, true]);
        g.glx.begin(Shape::Quads);
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();
        g.glx.color_mask(true);
        g.glx.pop_matrix();
    }

    /// `draw_text_sprite`: the readout under the picture.
    fn draw_text_sprite(&mut self, g: &mut Gl, i: usize) {
        if self.sprites[i].opacity <= 0.0 {
            return;
        }
        let id = self.sprites[i].text_id;
        let target = self
            .sprites
            .iter()
            .position(|s| s.id == id && s.state != SpriteState::Dead);

        let text = if let Some(t) = target {
            let ts = self.sprites[t].clone();
            if ts.opacity <= 0.0 && (ts.state == SpriteState::New || ts.state == SpriteState::In) {
                return;
            }
            let mut r = ts.current;
            if let Some(img) = self.find_newest(Kind::Image) {
                Self::compute_image_rect(&mut r, self.sprites[img].current, ts.back);
            }

            // Upstream's joke, kept: these numbers have almost nothing to do
            // with what is on screen, exactly as they do not in the film.
            let x = ((r.x * 10000.0) as i64).abs() % 10000;
            let y = ((r.y * 10000.0) as i64).abs() % 10000;
            let z = ((r.w * 10000.0) as i64).abs() % 10000;
            let mut text = format!("ZM {z:04}  NS {y:04}  EW {x:04}");

            let boring = |v: i64| v == 0 || v == 5000;
            if boring(x) && boring(y) && boring(z) {
                text.clear();
            }

            // The picture that lingers gets its filename instead.
            if self.do_titles && ts.kind == Kind::Image && ts.remain {
                let title = ts
                    .img_id
                    .and_then(|id| self.image(id))
                    .map(|m| m.title.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Loading".to_string());
                let tail: String = title.chars().rev().take(23).collect();
                let tail: String = tail.chars().rev().collect();
                text = format!(">>{tail:<23}");
                text = text
                    .chars()
                    .map(|c| match c {
                        'a'..='z' => c.to_ascii_uppercase(),
                        '/' | '-' | '.' => '_',
                        _ => c,
                    })
                    .collect();
            }

            if text.is_empty() {
                return;
            }
            self.sprites[i].text = text.clone();
            text
        } else if !self.sprites[i].text.is_empty() {
            // The sprite it was reporting on may be gone, but the last thing
            // it said is kept.
            self.sprites[i].text.clone()
        } else {
            return;
        };

        let (w, h) = (g.width(), g.height());
        let mut color = self.text_color;
        color[3] = self.sprites[i].opacity as f32;
        self.font.print_label(&mut g.glx, &text, w, h, 2, color);
    }
}

impl Hack3d for Esper {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.now = g.time;

        g.glx.lighting(false);
        g.glx.depth_test(false);
        g.glx.depth_mask(false);
        g.glx.cull_face(false);
        g.glx.color_material(true);
        g.glx.blend(Blend::Alpha);

        self.tick_sprites();

        g.glx.clear();
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        // A 1x1 quad at the origin fills the window.
        g.glx.ortho(0.0, 1.0, 0.0, 1.0, -1.0, 1.0);

        // The pictures first, then everything over the top of them.
        for i in 0..self.sprites.len() {
            if self.sprites[i].kind == Kind::Image {
                let sp = self.sprites[i].clone();
                self.draw_image_sprite(g, &sp);
            }
        }
        for i in 0..self.sprites.len() {
            match self.sprites[i].kind {
                Kind::Image => {}
                Kind::Flash => {
                    if self.sprites[i].opacity > 0.0 {
                        self.draw_flash_sprite(g);
                    }
                }
                Kind::Text => self.draw_text_sprite(g, i),
                _ => {
                    let sp = self.sprites[i].clone();
                    self.draw_line_sprite(g, &sp);
                }
            }
        }

        if self.now >= self.anim_start + self.anim_duration {
            self.tick_animation(g);
        }

        g.glx.depth_mask(true);
        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.clear();
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        // Upstream's manual reticle wants arrow keys held down and a
        // key-release to let go with, which this runtime has not got. A poke
        // throws the current picture away instead, which is upstream's Tab.
        if matches!(event, XEvent::KeyPress { .. } | XEvent::ButtonPress { .. }) {
            self.anim = Anim::ImageZoom;
            self.anim_duration = 0.0;
            return true;
        }
        false
    }
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [
        f32::from(r) / 255.0,
        f32::from(gg) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    ]
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // A small point size keeps it nice and grainy.
    let font = TexFont::load(&mut g.glx, "monospace 10");

    let mut st = Esper {
        images: Vec::new(),
        sprites: Vec::new(),
        now: 0.0,
        font,
        sprite_id: 0,
        image_id: 0,
        grid_color: resource_color(g, "gridColor"),
        reticle_color: resource_color(g, "reticleColor"),
        text_color: resource_color(g, "textColor"),
        anim: Anim::Blank,
        anim_start: 0.0,
        anim_duration: 0.0,
        grid_size: g.res.int("gridSize").clamp(2, 40),
        grid_thickness: g.res.float("gridThickness").clamp(1.0, 60.0),
        do_titles: g.res.bool("titles"),
        speed: g.res.float("speed").clamp(0.2, 20.0),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:         20000",
    "*showFPS:       False",
    "*titleFont:     monospace 10",
    "*gridColor:     #4444FF",
    "*reticleColor:  #FFFF77",
    "*textColor:     #FFFFBB",
    "*gridSize:      11",
    "*gridThickness: 15",
    "*titles:        True",
    "*speed:         1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.2, 20.0, 0.2, 1, "1.0"),
    Opt::boolean("titles", "Show file names", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "esper",
    label: "Esper",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2017",
        video: Some("https://www.youtube.com/watch?v=_er7xZd7zUU"),
        blurb: "The Esper machine from Blade Runner: enhance 224 to 176.",
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

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// A machine with no GL behind it.
    fn a_machine() -> Esper {
        Esper {
            images: Vec::new(),
            sprites: Vec::new(),
            now: 0.0,
            font: TexFont::load(&mut crate::runtime::gl::Glx::new(), "monospace 10"),
            sprite_id: 0,
            image_id: 0,
            grid_color: [0.27, 0.27, 1.0, 1.0],
            reticle_color: [1.0, 1.0, 0.47, 1.0],
            text_color: [1.0, 1.0, 0.73, 1.0],
            anim: Anim::Blank,
            anim_start: 0.0,
            anim_duration: 0.0,
            grid_size: 11,
            grid_thickness: 15.0,
            do_titles: true,
            speed: 1.0,
        }
    }

    /// A sprite's life is pause, fade in, hold, fade out, gone. Nothing is on
    /// screen during the pause, which is what lets a queue of copies read as
    /// a march of discrete steps rather than a slide.
    #[test]
    fn a_sprite_waits_then_fades_up_and_down() {
        let mut st = a_machine();
        st.sprites.push(Sprite {
            pause_duration: 1.0,
            fade_duration: 0.5,
            duration: 2.0,
            ..Sprite::new(1, Kind::Reticle, 0.0)
        });

        let at = |st: &mut Esper, t: f64| {
            st.now = t;
            st.tick_sprite(0);
            (st.sprites[0].opacity, st.sprites[0].state)
        };

        assert_eq!(at(&mut st, 0.5).0, 0.0, "it showed during its pause");
        assert!((at(&mut st, 1.25).0 - 0.5).abs() < 1e-9, "half faded in");
        assert_eq!(at(&mut st, 1.5), (1.0, SpriteState::Full));
        assert_eq!(at(&mut st, 3.0), (1.0, SpriteState::Full));
        assert!((at(&mut st, 3.75).0 - 0.5).abs() < 1e-9, "half faded out");
        assert_eq!(at(&mut st, 4.5).1, SpriteState::Dead);

        // And a sprite with `remain` set never leaves.
        st.sprites[0].remain = true;
        assert_eq!(at(&mut st, 9999.0), (1.0, SpriteState::Full));
    }

    /// A move is a queue of copies, each parked one step further along and
    /// each waiting longer before it appears. The last one lands exactly on
    /// the destination.
    #[test]
    fn a_move_becomes_a_march_of_stills() {
        let mut st = a_machine();
        let mut sp = Sprite::new(1, Kind::Reticle, 0.0);
        sp.from = Rect {
            x: 0.5,
            y: 0.5,
            w: 1.0,
            h: 1.0,
        };
        sp.current = sp.from;
        sp.to = Rect {
            x: 0.9,
            y: 0.8,
            w: 1.0,
            h: 1.0,
        };
        sp.fade_duration = 0.2;
        st.sprites.push(sp);
        st.sprite_id = 1;
        st.compute_sprite_duration(0, false);

        let mut g = crate::runtime::hack3d::Gl::for_test(640, 480);
        st.animate_sprite_path(&mut g, 0, false);

        // Every copy is parked: it does not move over its own life.
        let steps: Vec<&Sprite> = st
            .sprites
            .iter()
            .filter(|s| s.kind == Kind::Reticle && s.id != 1)
            .collect();
        assert!(steps.len() >= 3, "only {} steps", steps.len());
        for s in &steps {
            assert_eq!(s.from, s.to, "a step is still moving");
        }

        // The pauses increase, so only one is on screen at a time.
        let mut pauses: Vec<f64> = steps.iter().map(|s| s.pause_duration).collect();
        pauses.sort_by(f64::total_cmp);
        assert!(
            pauses.windows(2).all(|w| w[1] > w[0]),
            "two steps start together: {pauses:?}"
        );

        // The last lands on the destination.
        let last = steps
            .iter()
            .max_by(|a, b| a.pause_duration.total_cmp(&b.pause_duration))
            .expect("steps");
        assert!((last.to.x - 0.9).abs() < 1e-9, "landed at {}", last.to.x);
        assert!((last.to.y - 0.8).abs() < 1e-9);

        // Every step of a reticle gets its own readout.
        let texts = st.sprites.iter().filter(|s| s.kind == Kind::Text).count();
        assert_eq!(texts, steps.len(), "a step has no readout");
    }

    /// Zooming in on the box puts what was inside it on the whole screen, and
    /// zooming back out undoes that.
    #[test]
    fn the_picture_ends_up_showing_what_the_box_held() {
        // A picture filling the screen, and a box round its middle-right.
        let img = Rect {
            x: 0.5,
            y: 0.5,
            w: 1.0,
            h: 1.0,
        };
        let mut r = Rect {
            x: 0.75,
            y: 0.5,
            w: 0.5,
            h: 0.5,
        };
        Esper::compute_image_rect(&mut r, img, false);

        // Half the width means twice the magnification.
        assert!((r.w - 2.0).abs() < 1e-9, "width came out {}", r.w);
        assert!((r.h - 2.0).abs() < 1e-9);
        // And the picture moves left, since the box was to the right.
        assert!(r.x < 0.5, "it moved the wrong way: {}", r.x);

        // Zooming out is the other way about: the picture shrinks.
        let mut r = Rect {
            x: 0.5,
            y: 0.5,
            w: 0.5,
            h: 0.5,
        };
        Esper::compute_image_rect(&mut r, img, true);
        assert!((r.w - 0.5).abs() < 1e-9, "width came out {}", r.w);
    }

    /// The picture never zooms out past the frame nor pans off its own edge.
    #[test]
    fn the_picture_stays_covering_the_screen() {
        let mut st = a_machine();
        let mut b = Sprite::new(1, Kind::Box, 0.0);
        b.current = Rect {
            x: 0.9,
            y: 0.9,
            w: 0.9,
            h: 0.9,
        };
        b.back = true;
        st.sprites.push(b);
        let mut i = Sprite::new(2, Kind::Image, 0.0);
        i.current = Rect {
            x: 0.5,
            y: 0.5,
            w: 1.2,
            h: 1.0,
        };
        st.sprites.push(i);

        st.track_box_with_image(0, 1);
        let to = st.sprites[1].to;
        assert!(
            to.w >= 1.0 || to.h >= 1.0,
            "the picture shrank away from the frame: {to:?}"
        );
        assert!(to.x >= -to.w / 2.0 + 1.0 - 1e-9, "it panned off the left");
        assert!(to.x <= to.w / 2.0 + 1e-9, "it panned off the right");
    }

    /// It runs: the grid appears, then a picture, then the reticle, the box
    /// and the zoom, without the sprite queue running away.
    #[test]
    fn the_machine_runs_through_its_cycle() {
        let mut r = start(StartArgs::new(640, 480, "speed=8", 20260812));
        let mut saw_texture = false;
        for _ in 0..900 {
            r.step();
            let f = r.frame();
            if f.batches.iter().any(|b| b.texture.is_some()) {
                saw_texture = true;
            }
            assert!(
                f.batches.len() < 400,
                "{} batches, so sprites are piling up",
                f.batches.len()
            );
        }
        assert!(saw_texture, "the picture never appeared");
    }

    /// The grid is over everything and never goes away.
    #[test]
    fn the_grid_is_drawn_over_the_picture() {
        let r = run("speed=8", 60);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        // The grid is untextured quads, drawn after the picture.
        let last_tex = f.batches.iter().rposition(|b| b.texture.is_some());
        let last_plain = f.batches.iter().rposition(|b| b.texture.is_none());
        if let (Some(t), Some(p)) = (last_tex, last_plain) {
            assert!(p > t, "the grid was drawn under the picture");
        }
    }

    /// The flash writes only blue and alpha, so what is under it keeps its
    /// reds and greens and comes out tinted rather than covered.
    #[test]
    fn the_flash_only_touches_blue() {
        let st = a_machine();
        let mut g = crate::runtime::hack3d::Gl::for_test(640, 480);
        st.draw_flash_sprite(&mut g);
        let f = g.glx.frame();
        let masked: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.color_mask != [true; 4])
            .collect();
        assert_eq!(masked.len(), 1, "the flash was not masked");
        assert_eq!(masked[0].color_mask, [false, false, true, true]);
    }
}
