//! Port of `hacks/glx/glslideshow.c`.
//!
//! ```text
//! glslideshow, Copyright © 2003-2026 Jamie Zawinski <jwz@jwz.org>
//! Loads a sequence of images and smoothly pans around them; crossfades
//! when loading new images.
//!
//! Originally written by Mike Oliphant <oliphant@gtk.org> (c) 2002, 2003.
//! Rewritten by jwz, 21-Jun-2003.
//! Rewritten by jwz again, 6-Feb-2005.
//! Modified by Richard Weeks <rtweeks21@gmail.com> Copyright (c) 2020
//! Rewritten by jwz again, 27-Nov-2025.
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
//! A slideshow, with panning, zooming and crossfading.
//!
//! Everything on screen is a *sprite*: a rectangle carrying one picture, with
//! a start and an end and an easing curve between them. A new picture arrives
//! by sliding in from an edge, spinning, flipping, zooming out of the middle
//! or simply fading up, while the one it replaces leaves the same way. Once it
//! has landed it starts panning: a second sprite showing the *same* picture is
//! launched from somewhere else on it and crossfaded with the first, over and
//! over, so the view drifts about the image without ever cutting.
//!
//! The two halves are stitched together by two states that exist only to make
//! the join invisible. `IN_PANZOOM` turns the freshly landed still picture
//! into the first panner without a jump, and `RECENTER` pans the last one back
//! to the middle so the next transition has something square to work with.
//!
//! Upstream loads its pictures in slices spread over several frames, since it
//! has to pull them out of the X server and convert them by hand. Here a
//! picture either is or is not ready, so the pipeline stays and the slicing
//! goes.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
    screenhack_event_helper,
};

/// Where a sprite is, how big, and how it is turned.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rx: f64,
    ry: f64,
    rz: f64,
}

/// How a sprite comes on and goes off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transition {
    /// Drifting across the picture, fading into the next drift.
    PanZoom,
    Fade,
    Left,
    Right,
    Top,
    Bottom,
    Flip,
    Spin,
}

/// Where a sprite is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpriteState {
    New,
    In,
    Full,
    Out,
    Dead,
}

/// What the show as a whole is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AnimState {
    Loading,
    FirstIn,
    /// Turning the picture that has just landed into a panner.
    InPanZoom,
    MidPanZoom,
    /// Panning back to the middle, ready to be transitioned away.
    Recenter,
    TransitionOut,
    TransitionPanZoom,
}

struct Image {
    id: u32,
    /// How big the texture holding the picture is. Upstream also keeps the
    /// picture's own size, which it needs only for the wireframe grid and for
    /// scaling up an image the X server had to shrink; neither applies here.
    tw: f64,
    th: f64,
    /// Where in the texture the picture actually is.
    geom: (f64, f64, f64, f64),
    loaded: bool,
    /// Whether it has been on screen yet.
    used: bool,
    texid: Option<u32>,
    refcount: i32,
    title: String,
}

#[derive(Clone, Copy)]
struct Sprite {
    id: u32,
    img_id: u32,
    opacity: f64,
    start_time: f64,
    duration: f64,
    transition: Transition,
    easing: Ease,
    /// This sprite's own share of the zoom range.
    zoom: f64,
    from: Rect,
    to: Rect,
    current: Rect,
    state: SpriteState,
}

/// The transitions a new picture may arrive by, and the easings it may use.
/// Both are picked at random; the easings repeat so that some are likelier
/// than others, which is upstream's table verbatim.
///
/// Upstream also has a ZOOM transition, and code to animate it, but its table
/// does not list it, so it never happens. It is left out here rather than
/// carried as unreachable code.
const MODES: [Transition; 7] = [
    Transition::Fade,
    Transition::Left,
    Transition::Right,
    Transition::Top,
    Transition::Bottom,
    Transition::Flip,
    Transition::Spin,
];
const EASINGS: [Ease; 10] = [
    Ease::InOutQuad,
    Ease::InOutQuad,
    Ease::InOutQuad,
    Ease::InOutQuad,
    Ease::InOutQuint,
    Ease::InOutQuint,
    Ease::InOutBack,
    Ease::InOutBack,
    Ease::InCubic,
    Ease::InCubic,
];

struct Slideshow {
    images: Vec<Image>,
    sprites: Vec<Sprite>,
    state: AnimState,
    title_opacity: f64,

    now: f64,
    /// When this picture went up.
    start_time: f64,
    change_now: bool,

    font: TexFont,
    sprite_id: u32,
    image_id: u32,

    transition_seconds: f64,
    fade_seconds: f64,
    pan_seconds: f64,
    image_seconds: f64,
    /// How much of the picture must stay visible at the deepest zoom, as a
    /// percentage.
    zoom: f64,
    letterbox: bool,
    do_titles: bool,
}

impl Slideshow {
    fn image(&self, id: u32) -> Option<&Image> {
        self.images.iter().find(|i| i.id == id)
    }

    /// `alloc_image_incremental`: ask for a picture. Upstream starts a
    /// background load here and steps it over several frames; this either
    /// gets one or does not.
    fn alloc_image(&mut self, g: &mut Gl) {
        let (w, h) = (g.width(), g.height());
        let Some(img) = g.load_image(w, h) else {
            return;
        };
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
            loaded: true,
            used: false,
            texid: Some(id),
            refcount: 0,
            title: img.title.unwrap_or_default(),
        });
    }

    /// `get_image`: an unused picture if one is wanted, or an old one to pan
    /// over again. Keeps one spare in the pipe either way.
    fn get_image(&mut self, g: &mut Gl, want_new: bool) -> Option<u32> {
        let mut new_img = None;
        let mut old_img = None;
        let mut loading = false;
        for i in &self.images {
            if !i.loaded {
                loading = true;
            } else if !i.used {
                new_img = Some(i.id);
            } else {
                old_img = Some(i.id);
            }
        }

        let ret = if want_new { new_img } else { old_img };
        if new_img.is_none() && !loading {
            self.alloc_image(g);
        }
        ret
    }

    /// `new_sprite`: a fresh rectangle carrying a picture. `None` when there
    /// is no picture to be had yet.
    fn new_sprite(&mut self, g: &mut Gl, want_new: bool) -> Option<usize> {
        let img_id = self.get_image(g, want_new)?;

        self.sprite_id += 1;
        let sp = Sprite {
            id: self.sprite_id,
            img_id,
            opacity: 0.0,
            start_time: self.now,
            duration: 0.0,
            transition: Transition::Fade,
            easing: Ease::InOutQuad,
            // 75% => [1.0 - 1.33]
            zoom: if self.zoom > 0.0 {
                1.0 + frand(100.0 / self.zoom - 1.0)
            } else {
                1.0
            },
            from: Rect::default(),
            to: Rect::default(),
            current: Rect::default(),
            state: SpriteState::New,
        };

        if let Some(img) = self.images.iter_mut().find(|i| i.id == img_id) {
            img.refcount += 1;
            img.used = true;
        }
        if want_new {
            self.start_time = self.now;
        }
        self.sprites.push(sp);
        Some(self.sprites.len() - 1)
    }

    /// How wide a picture of this shape comes out on this screen, before its
    /// own zoom is applied. Letterboxing fits it inside; cropping fills.
    fn fitted_width(&self, ratio: f64, vp_w: f64, vp_h: f64) -> f64 {
        if self.letterbox {
            if vp_w * ratio < vp_h {
                vp_w /* full width, smaller height */
            } else {
                vp_h / ratio /* full height, smaller width */
            }
        } else if vp_w * ratio < vp_h {
            vp_h / ratio /* full height, crop width */
        } else {
            vp_w /* full width, crop height */
        }
    }

    /// `launch_sprite`: start whatever the state machine has just asked for.
    fn launch_sprite(&mut self, g: &mut Gl) {
        let vp_w = f64::from(g.width());
        let vp_h = f64::from(g.height());

        if self.state == AnimState::Loading {
            return;
        }

        let out = if self.state == AnimState::FirstIn {
            None
        } else {
            self.sprites.last().map(|s| s.id)
        };

        let want_new = matches!(
            self.state,
            AnimState::FirstIn | AnimState::TransitionOut | AnimState::TransitionPanZoom
        );
        let Some(in_i) = self.new_sprite(g, want_new) else {
            return;
        };
        let in_id = self.sprites[in_i].id;

        match self.state {
            AnimState::FirstIn | AnimState::TransitionOut => {
                self.launch_transition(out, in_id, vp_w, vp_h);
            }
            AnimState::InPanZoom => {
                // Turn the picture that has just landed into a panner, in
                // place, so there is no jump into the pan.
                if let Some(out_id) = out
                    && let Some(sp) = self.sprites.iter_mut().find(|s| s.id == out_id)
                {
                    sp.duration = self.fade_seconds + self.pan_seconds;
                    sp.start_time = self.now - self.pan_seconds;
                    sp.transition = Transition::PanZoom;
                    sp.state = SpriteState::Full;
                    sp.to = sp.current;
                    sp.from = sp.current;
                }
                self.launch_panzoom(in_id, vp_w, vp_h);
            }
            AnimState::MidPanZoom | AnimState::Recenter | AnimState::TransitionPanZoom => {
                self.launch_panzoom(in_id, vp_w, vp_h);
            }
            AnimState::Loading => {}
        }
    }

    /// The in-and-out half: two sprites crossing over the same distance, so
    /// the sliding ones stay locked together.
    fn launch_transition(&mut self, out: Option<u32>, in_id: u32, vp_w: f64, vp_h: f64) {
        let mut sprite_w = [0.0f64; 2];
        let mut max_w = vp_w;
        let mut max_h = vp_h;

        let ids = [out, Some(in_id)];
        for (i, id) in ids.iter().enumerate() {
            let Some(id) = id else { continue };
            let Some(sp) = self.sprites.iter().find(|s| s.id == *id) else {
                continue;
            };
            let Some(img) = self.image(sp.img_id) else {
                continue;
            };
            let ratio = img.geom.3 / img.geom.2;
            let w = self.fitted_width(ratio, vp_w, vp_h) * sp.zoom;
            sprite_w[i] = w;
            max_w = max_w.max(w);
            max_h = max_h.max(w * ratio);
        }

        let t = MODES[(random() as usize) % MODES.len()];
        let e = EASINGS[(random() as usize) % EASINGS.len()];
        // Both halves of a flip or a spin have to agree on which way, so the
        // decisions are taken once here rather than per sprite.
        let quads = 2 + (random() % 6) as i32;
        let spin_dir = if random() & 1 == 1 { 1.0 } else { -1.0 };
        let horiz = random() & 1 == 1;
        let flip_sign = if random() & 1 == 1 { 1.0 } else { -1.0 };
        let mut out_rot = (0.0, 0.0, 0.0);

        for (i, id) in ids.iter().enumerate() {
            let out_p = i == 0;
            let Some(id) = id else { continue };
            let Some(idx) = self.sprites.iter().position(|s| s.id == *id) else {
                continue;
            };
            let img_id = self.sprites[idx].img_id;
            let Some(img) = self.image(img_id) else {
                continue;
            };
            let ratio = img.geom.3 / img.geom.2;

            let sp = &mut self.sprites[idx];
            sp.state = if out_p {
                SpriteState::Out
            } else {
                SpriteState::In
            };
            sp.start_time = self.now;
            sp.duration = self.transition_seconds;
            if !out_p && self.pan_seconds <= 0.0 {
                // With no panning, this picture stays put for its whole turn.
                sp.duration += self.image_seconds;
            }

            // Centred, unless the transition says otherwise.
            sp.to.w = sprite_w[i];
            sp.to.h = sp.to.w * ratio;
            sp.to.x = if sp.to.w > vp_w {
                -(sp.to.w - vp_w) / 2.0
            } else {
                (vp_w - sp.to.w) / 2.0
            };
            sp.to.y = if sp.to.h > vp_h {
                -(sp.to.h - vp_h) / 2.0
            } else {
                (vp_h - sp.to.h) / 2.0
            };
            sp.to.rx = 0.0;
            sp.to.ry = 0.0;
            sp.to.rz = 0.0;
            sp.from = sp.to;
            sp.transition = t;
            sp.easing = e;

            match t {
                // No motion, only alpha.
                Transition::Fade => {}
                Transition::Left => {
                    if out_p {
                        sp.to.x += max_w;
                    } else {
                        sp.from.x = sp.to.x - max_w;
                    }
                }
                Transition::Right => {
                    if out_p {
                        sp.to.x -= max_w;
                    } else {
                        sp.from.x = sp.to.x + max_w;
                    }
                }
                Transition::Top => {
                    if out_p {
                        sp.to.y -= max_h;
                    } else {
                        sp.from.y = sp.to.y + max_h;
                    }
                }
                Transition::Bottom => {
                    if out_p {
                        sp.to.y += max_h;
                    } else {
                        sp.from.y = sp.to.y - max_h;
                    }
                }
                Transition::Spin => {
                    // The one coming in unwinds exactly what the one going
                    // out wound up, so they turn as one.
                    let spin = if !out_p && out.is_some() {
                        -out_rot.2
                    } else {
                        90.0 * f64::from(quads) * spin_dir
                    };
                    let scale = 0.0001;
                    let r = if out_p { &mut sp.to } else { &mut sp.from };
                    r.rz = spin;
                    r.w *= scale;
                    r.h *= scale;
                    r.x = (vp_w - r.w) / 2.0;
                    r.y = (vp_h - r.h) / 2.0;
                }
                Transition::Flip => {
                    let (fx, fy) = if !out_p && out.is_some() {
                        (-out_rot.0, -out_rot.1)
                    } else if horiz {
                        (180.0 * flip_sign, 0.0)
                    } else {
                        (0.0, 180.0 * flip_sign)
                    };
                    let r = if out_p { &mut sp.to } else { &mut sp.from };
                    r.rx = fx;
                    r.ry = fy;
                }
                Transition::PanZoom => {}
            }

            if out_p {
                out_rot = (sp.to.rx, sp.to.ry, sp.to.rz);
            }
        }
    }

    /// The panning half: one drift across the picture, fading in over the
    /// drift before it and out under the drift after.
    fn launch_panzoom(&mut self, in_id: u32, vp_w: f64, vp_h: f64) {
        let Some(idx) = self.sprites.iter().position(|s| s.id == in_id) else {
            return;
        };
        let img_id = self.sprites[idx].img_id;
        let Some(img) = self.image(img_id) else {
            return;
        };
        let ratio = img.geom.3 / img.geom.2;
        let w = self.fitted_width(ratio, vp_w, vp_h);

        let z = |zoom: f64| {
            if zoom > 0.0 {
                1.0 + frand(100.0 / zoom - 1.0)
            } else {
                1.0
            }
        };
        let z0 = z(self.zoom);
        let mut z1 = z(self.zoom);
        let recenter = self.state == AnimState::Recenter;
        let own_zoom = self.sprites[idx].zoom;
        if recenter {
            z1 = own_zoom;
        }

        let sp = &mut self.sprites[idx];
        sp.duration = self.fade_seconds + self.pan_seconds;
        sp.start_time = self.now;
        sp.transition = Transition::PanZoom;
        sp.state = SpriteState::In;

        sp.from.w = w * z0;
        sp.from.h = sp.from.w * ratio;
        sp.from.rx = 0.0;
        sp.from.ry = 0.0;
        sp.from.rz = 0.0;
        sp.from.x = if sp.from.w > vp_w {
            -frand(sp.from.w - vp_w)
        } else {
            frand(vp_w - sp.from.w)
        };
        sp.from.y = if sp.from.h > vp_h {
            -frand(sp.from.h - vp_h)
        } else {
            frand(vp_h - sp.from.h)
        };

        sp.to.w = w * z1;
        sp.to.h = sp.to.w * ratio;
        sp.to.rx = 0.0;
        sp.to.ry = 0.0;
        sp.to.rz = 0.0;
        if recenter {
            // Land it square in the middle, so the next transition has
            // something to work with.
            sp.to.x = if sp.to.w > vp_w {
                -(sp.to.w - vp_w) / 2.0
            } else {
                (vp_w - sp.to.w) / 2.0
            };
            sp.to.y = if sp.to.h > vp_h {
                -(sp.to.h - vp_h) / 2.0
            } else {
                (vp_h - sp.to.h) / 2.0
            };
        } else {
            sp.to.x = if sp.to.w > vp_w {
                -frand(sp.to.w - vp_w)
            } else {
                frand(vp_w - sp.to.w)
            };
            sp.to.y = if sp.to.h > vp_h {
                -frand(sp.to.h - vp_h)
            } else {
                frand(vp_h - sp.to.h)
            };
        }
    }

    /// `tick_sprites`: run the state machine, then move every sprite along
    /// its own curve.
    fn tick_sprites(&mut self, g: &mut Gl) {
        let ostate = self.state;
        let total_secs = self.now - self.start_time;
        let sp_secs = self.sprites.last().map_or(0.0, |s| self.now - s.start_time);
        let have_sprite = !self.sprites.is_empty();
        let mut launch = false;

        // Keep one spare picture in the pipe.
        self.get_image(g, true);
        let image_p = self.images.last().is_some_and(|i| i.loaded && !i.used);

        match self.state {
            AnimState::Loading => {
                if image_p {
                    self.state = AnimState::FirstIn;
                }
            }
            AnimState::FirstIn | AnimState::TransitionOut => {
                if self.change_now && image_p {
                    self.state = AnimState::TransitionOut;
                    launch = true;
                } else if total_secs >= self.transition_seconds && self.pan_seconds > 0.0 {
                    self.state = AnimState::InPanZoom;
                } else if total_secs >= self.image_seconds && image_p {
                    self.state = AnimState::TransitionOut;
                    // Do TRANSITION_OUT again.
                    launch = true;
                }
            }
            AnimState::InPanZoom => {
                if !have_sprite {
                } else if self.change_now && image_p {
                    self.state = if self.transition_seconds <= 0.0 {
                        AnimState::TransitionPanZoom
                    } else {
                        AnimState::Recenter
                    };
                } else if sp_secs >= self.pan_seconds {
                    self.state = AnimState::MidPanZoom;
                }
            }
            AnimState::MidPanZoom | AnimState::TransitionPanZoom => {
                if have_sprite && image_p && (self.change_now || sp_secs >= self.pan_seconds) {
                    if self.change_now
                        || total_secs >= self.image_seconds - (self.fade_seconds + self.pan_seconds)
                    {
                        if self.transition_seconds <= 0.0 {
                            // Just like MidPanZoom, but on a new picture.
                            self.state = AnimState::TransitionPanZoom;
                            launch = true;
                        } else {
                            self.state = AnimState::Recenter;
                        }
                    } else {
                        self.state = AnimState::MidPanZoom;
                        launch = true;
                    }
                }
            }
            AnimState::Recenter => {
                if have_sprite
                    && sp_secs >= self.pan_seconds + self.fade_seconds
                    && total_secs >= self.image_seconds
                {
                    self.state = AnimState::TransitionOut;
                }
            }
        }

        self.change_now = false;
        if ostate != self.state {
            launch = true;
        }
        if launch {
            self.launch_sprite(g);
            return;
        }

        let n = self.sprites.len();
        for i in 0..n {
            let secs = self.now - self.sprites[i].start_time;
            let mut ratio;

            match self.sprites[i].transition {
                Transition::PanZoom => {
                    // Three transitions in one: fade up, hold, fade down.
                    ratio = secs / (self.fade_seconds + self.pan_seconds).max(1e-6);
                    let sp = &mut self.sprites[i];
                    if secs <= self.fade_seconds {
                        sp.opacity = (secs / self.fade_seconds.max(1e-6)).min(1.0);
                        sp.state = SpriteState::In;
                    } else if secs <= self.pan_seconds {
                        sp.state = SpriteState::Full;
                        sp.opacity = 1.0;
                    } else if secs <= self.fade_seconds + self.pan_seconds {
                        sp.opacity =
                            1.0 - ((secs - self.pan_seconds) / self.fade_seconds.max(1e-6));
                        sp.state = SpriteState::Out;
                        // The one being recentred has to stay solid, since
                        // there is nothing behind it.
                        if self.state == AnimState::Recenter && i == n - 1 {
                            sp.opacity = 1.0;
                        }
                    } else {
                        sp.state = SpriteState::Dead;
                        sp.opacity = 0.0;
                    }
                }
                other => {
                    ratio = secs / self.transition_seconds.max(1e-6);
                    let sp = &mut self.sprites[i];
                    if ratio <= 1.0 {
                        sp.opacity = if matches!(other, Transition::Fade | Transition::Spin) {
                            if sp.state == SpriteState::In {
                                ratio
                            } else {
                                1.0 - ratio
                            }
                        } else {
                            1.0
                        };
                    } else if secs <= sp.duration {
                        // Linger.
                        ratio = 1.0;
                        sp.opacity = 1.0;
                    } else {
                        sp.state = SpriteState::Dead;
                        ratio = 1.0;
                        sp.opacity = 0.0;
                    }
                }
            }

            let sp = &mut self.sprites[i];
            let r = ease(sp.easing, ratio);
            sp.current.x = sp.from.x + r * (sp.to.x - sp.from.x);
            sp.current.y = sp.from.y + r * (sp.to.y - sp.from.y);
            sp.current.w = sp.from.w + r * (sp.to.w - sp.from.w);
            sp.current.h = sp.from.h + r * (sp.to.h - sp.from.h);
            sp.current.rx = sp.from.rx + r * (sp.to.rx - sp.from.rx);
            sp.current.ry = sp.from.ry + r * (sp.to.ry - sp.from.ry);
            sp.current.rz = sp.from.rz + r * (sp.to.rz - sp.from.rz);
        }

        // Bury the dead, but never the last one while there is nothing to
        // replace it with.
        let mut i = 0;
        while i < self.sprites.len() {
            if self.sprites[i].state == SpriteState::Dead {
                if self.sprites.len() < 2 && !image_p {
                    i += 1;
                    continue;
                }
                let img_id = self.sprites[i].img_id;
                self.sprites.remove(i);
                if let Some(img) = self.images.iter_mut().find(|m| m.id == img_id) {
                    img.refcount -= 1;
                }
                self.images.retain(|m| m.refcount > 0 || !m.used);
            } else {
                i += 1;
            }
        }

        self.tick_title();
    }

    /// The title has a life of its own, since one title spans several
    /// sprites: up once the picture is fully there, down before the next one
    /// starts arriving.
    fn tick_title(&mut self) {
        if !self.do_titles {
            return;
        }
        let t = self.now - self.start_time;
        let start = if self.transition_seconds > 0.0 {
            self.transition_seconds
        } else {
            self.fade_seconds
        };
        let end = if self.transition_seconds > 0.0 && self.fade_seconds > 0.0 {
            self.image_seconds + self.fade_seconds
        } else if self.transition_seconds > 0.0 {
            self.image_seconds
        } else {
            self.image_seconds - self.fade_seconds
        };

        let dur = end - start;
        let mut sec_fade = 3.0f64;
        if sec_fade > dur / 2.0 {
            sec_fade = dur / 2.0;
        }
        // If the pictures are jump-cutting, so do the titles.
        if self.transition_seconds == 0.0 && self.fade_seconds == 0.0 {
            sec_fade = 0.0;
        }

        // Too short a turn to bother with, or not started yet.
        self.title_opacity = if (dur <= 0.5 && sec_fade > 0.0) || t <= start {
            0.0
        } else if t <= start + sec_fade {
            (t - start) / sec_fade
        } else if t <= end - sec_fade {
            1.0
        } else if t <= end {
            (end - t) / sec_fade
        } else {
            0.0
        };
    }

    /// `draw_sprite`: one picture, at the point on its journey the clock says.
    fn draw_sprite(&self, g: &mut Gl, sp: &Sprite) {
        let Some(img) = self.image(sp.img_id) else {
            return;
        };
        let Some(texid) = img.texid else { return };
        let vp_w = f64::from(g.width());
        let vp_h = f64::from(g.height());

        g.glx.push_matrix();
        // The turns happen in a square space so a flip does not squash the
        // picture, and then it is put back.
        let aspect = (vp_w / vp_h) as f32;
        g.glx.scale(1.0, aspect, 1.0);
        g.glx.rotate(sp.current.rx as f32, 1.0, 0.0, 0.0);
        g.glx.rotate(sp.current.ry as f32, 0.0, 1.0, 0.0);
        g.glx.rotate(sp.current.rz as f32, 0.0, 0.0, 1.0);
        g.glx.scale(1.0, 1.0 / aspect, 1.0);

        g.glx.translate(
            (sp.current.x / vp_w - 0.5) as f32,
            (sp.current.y / vp_h - 0.5) as f32,
            0.0,
        );
        g.glx.scale(
            (sp.current.w / vp_w) as f32,
            (sp.current.h / vp_h) as f32,
            1.0,
        );

        let texw = img.geom.2 / img.tw;
        let texh = img.geom.3 / img.th;
        let texx1 = img.geom.0 / img.tw;
        let texy1 = img.geom.1 / img.th;
        let (texx2, texy2) = (texx1 + texw, texy1 + texh);

        g.glx.texturing(true);
        g.glx.bind_texture(texid);
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
        g.glx.texturing(false);

        g.glx.pop_matrix();
    }
}

impl Hack3d for Slideshow {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.now = g.time;
        self.tick_sprites(g);

        g.glx.depth_test(false);
        g.glx.lighting(false);
        g.glx.color_material(true);
        g.glx.blend(Blend::Alpha);
        g.glx.clear();

        g.glx.push_matrix();
        if self.state == AnimState::Loading {
            let (w, h) = (g.width(), g.height());
            let opacity = ((self.now) / 6.0).min(1.0) as f32;
            self.font
                .print_label(&mut g.glx, "Loading...", w, h, 0, [1.0, 1.0, 0.0, opacity]);
        }

        for i in 0..self.sprites.len() {
            // `draw_sprite` only reads, but the borrow checker cannot see
            // that through `&mut Gl`, so the sprite is copied out of the way.
            let sp = self.sprites[i];
            self.draw_sprite(g, &sp);
        }

        if self.do_titles && self.title_opacity > 0.0 {
            let title = self
                .sprites
                .last()
                .and_then(|sp| self.image(sp.img_id))
                .map(|img| img.title.clone())
                .unwrap_or_default();
            if !title.is_empty() {
                let (w, h) = (g.width(), g.height());
                self.font.print_label(
                    &mut g.glx,
                    &title,
                    w,
                    h,
                    1,
                    [1.0, 1.0, 1.0, self.title_opacity as f32],
                );
            }
        }
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        // Upstream's note: these numbers give a projection where a 1x1 quad
        // centred at the origin fills the viewport exactly while still not
        // being orthographic. All three are interdependent.
        let fov = 30.0;
        let cam = 15.0;
        let scale = 8.038_45;

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(fov, 1.0, 0.01, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, cam], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(scale, scale, scale);
        g.glx.clear();
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.change_now = true;
            return true;
        }
        false
    }
}

/// `sanity_check`: the four durations are nested inside one another, so any
/// set of settings has to be made consistent before anything reads them.
/// Returns them in the order transition, fade, pan, image.
fn sanity_check(transition: i32, fade: i32, pan: i32, image: i32) -> (f64, f64, f64, f64) {
    let transition = f64::from(transition.max(0));
    // A picture's turn is inclusive of the transition that brings it on, and
    // no turn may be empty.
    let mut image = f64::from(image);
    if image < transition {
        image = transition;
    }
    if image <= 0.0 {
        image = 1.0;
    }

    let fade = f64::from(fade.max(0));
    // A pan is inclusive of the fade that opens it, and panning without
    // fading looks terrible.
    let mut pan = f64::from(pan.max(0));
    if pan < fade {
        pan = fade;
    }
    if fade == 0.0 {
        pan = 0.0;
    }

    (transition, fade, pan, image)
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let zoom = f64::from(g.res.int("zoom").clamp(1, 100));
    let (transition_seconds, fade_seconds, pan_seconds, image_seconds) = sanity_check(
        g.res.int("transitionDuration"),
        g.res.int("fadeDuration"),
        g.res.int("panDuration"),
        g.res.int("imageDuration"),
    );

    let font = TexFont::load(&mut g.glx, "sans-serif 18");

    let mut st = Slideshow {
        images: Vec::new(),
        sprites: Vec::new(),
        state: AnimState::Loading,
        title_opacity: 0.0,
        now: 0.0,
        start_time: 0.0,
        change_now: false,
        font,
        sprite_id: 0,
        image_id: 0,
        transition_seconds,
        fade_seconds,
        pan_seconds,
        image_seconds,
        zoom,
        letterbox: g.res.bool("letterbox"),
        do_titles: g.res.bool("titles"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:              20000",
    "*showFPS:            False",
    "*titleFont:          sans-serif 18",
    "*transitionDuration: 3",
    "*fadeDuration:       2",
    "*panDuration:        6",
    "*imageDuration:      30",
    "*zoom:               75",
    "*titles:             False",
    "*letterbox:          True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider(
        "imageDuration",
        "Time until loading a new image",
        1.0,
        300.0,
        1.0,
        0,
        "30",
    ),
    Opt::slider(
        "transitionDuration",
        "Image loading animation duration",
        0.0,
        30.0,
        1.0,
        0,
        "3",
    ),
    Opt::slider(
        "zoom",
        "Always show at least this much of the image",
        50.0,
        100.0,
        1.0,
        0,
        "75",
    ),
    Opt::slider("panDuration", "Pan / zoom duration", 0.0, 30.0, 1.0, 0, "6"),
    Opt::slider("fadeDuration", "Crossfade duration", 0.0, 30.0, 1.0, 0, "2"),
    Opt::boolean("letterbox", "Letterbox", "true"),
    Opt::boolean("titles", "Show file names", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glslideshow",
    label: "GL Slideshow",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=1sfAXDzA6eM"),
        blurb: "A slideshow of images, with panning, zooming and crossfading.",
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

    /// A show with no GL behind it, for the arithmetic.
    fn a_show(transition: f64, fade: f64, pan: f64, image: f64) -> Slideshow {
        Slideshow {
            images: Vec::new(),
            sprites: Vec::new(),
            state: AnimState::Loading,
            title_opacity: 0.0,
            now: 0.0,
            start_time: 0.0,
            change_now: false,
            font: TexFont::load(&mut crate::runtime::gl::Glx::new(), "sans-serif 18"),
            sprite_id: 0,
            image_id: 0,
            transition_seconds: transition,
            fade_seconds: fade,
            pan_seconds: pan,
            image_seconds: image,
            zoom: 75.0,
            letterbox: true,
            do_titles: true,
        }
    }

    /// Letterboxing fits the whole picture inside the frame; cropping fills
    /// the frame and lets the picture overflow. Which way round depends on
    /// whether the picture is wider or taller than the frame.
    #[test]
    fn letterboxing_fits_and_cropping_fills() {
        let mut st = a_show(3.0, 2.0, 6.0, 30.0);
        let (vw, vh) = (800.0, 600.0);

        // A picture wider than the frame: 2:1 against 4:3.
        let wide = 0.5;
        st.letterbox = true;
        let w = st.fitted_width(wide, vw, vh);
        assert_eq!(w, vw, "letterboxed, a wide picture is as wide as the frame");
        assert!(w * wide < vh, "and shorter than it");

        st.letterbox = false;
        let w = st.fitted_width(wide, vw, vh);
        assert!(w > vw, "cropped, a wide picture overflows the sides");
        assert!(
            (w * wide - vh).abs() < 1e-9,
            "and is exactly as tall as the frame"
        );

        // And a picture taller than the frame: 1:2.
        let tall = 2.0;
        st.letterbox = true;
        let w = st.fitted_width(tall, vw, vh);
        assert!((w * tall - vh).abs() < 1e-9, "letterboxed to full height");
        assert!(w < vw, "and narrower than the frame");

        st.letterbox = false;
        let w = st.fitted_width(tall, vw, vh);
        assert_eq!(w, vw, "cropped to full width");
        assert!(w * tall > vh, "and taller than the frame");
    }

    /// The durations nest inside one another: an image's turn includes its
    /// transition, and a pan includes its fade. Panning with no fade is
    /// turned off outright because it looks terrible.
    #[test]
    fn the_durations_are_made_to_nest() {
        // (transition, fade, pan, image) in, and the same four out.
        let cases = [
            ((3, 2, 6, 30), (3.0, 2.0, 6.0, 30.0)),
            // A pan shorter than its fade is stretched to it.
            ((3, 8, 2, 30), (3.0, 8.0, 8.0, 30.0)),
            // No fade means no pan.
            ((3, 0, 9, 30), (3.0, 0.0, 0.0, 30.0)),
            // A turn shorter than its transition is stretched to it.
            ((20, 2, 6, 5), (20.0, 2.0, 6.0, 20.0)),
            // Nothing may be negative, and no turn may be empty.
            ((-5, -5, -5, 0), (0.0, 0.0, 0.0, 1.0)),
        ];
        for ((t, f, p, i), want) in cases {
            assert_eq!(sanity_check(t, f, p, i), want, "for {t},{f},{p},{i}");
        }
    }

    /// The title fades up once the picture is fully there and down before the
    /// next one starts arriving, and is never on screen outside that window.
    #[test]
    fn the_title_fades_up_and_down_inside_the_picture() {
        let mut st = a_show(3.0, 2.0, 6.0, 30.0);
        let mut saw_full = false;
        let mut prev = 0.0;
        let mut rising = true;

        for i in 0..=400 {
            st.now = f64::from(i) * 0.1;
            st.tick_title();
            let o = st.title_opacity;
            assert!((0.0..=1.0).contains(&o), "opacity {o} at {}", st.now);

            // Before the transition finishes there is nothing to name.
            if st.now < st.transition_seconds {
                assert_eq!(o, 0.0, "a title at {} seconds", st.now);
            }
            if o >= 1.0 {
                saw_full = true;
            }
            if saw_full && o < prev {
                rising = false;
            }
            assert!(
                rising || o <= prev + 1e-9,
                "the title came back up at {}",
                st.now
            );
            prev = o;
        }
        assert!(saw_full, "the title never became solid");
        assert_eq!(st.title_opacity, 0.0, "the title never went away");
    }

    /// With no fade at all the titles jump-cut too, rather than fading over a
    /// crossfade that is not happening.
    #[test]
    fn titles_jump_cut_when_the_pictures_do() {
        let mut st = a_show(0.0, 0.0, 0.0, 30.0);
        let mut seen: Vec<f64> = Vec::new();
        for i in 0..=400 {
            st.now = f64::from(i) * 0.1;
            st.tick_title();
            seen.push(st.title_opacity);
        }
        assert!(
            seen.iter().all(|o| *o == 0.0 || *o == 1.0),
            "a title faded when it should have cut"
        );
        assert!(seen.contains(&1.0), "the title never appeared");
    }

    /// It draws: a picture on screen, textured.
    #[test]
    fn the_picture_is_shown() {
        let r = run("", 3);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(
            f.batches.iter().any(|b| b.texture.is_some()),
            "no picture was drawn"
        );
    }

    /// It keeps running through a whole cycle: the picture arrives, pans,
    /// recentres and is replaced, without the sprite list running away.
    #[test]
    fn a_whole_cycle_runs_without_piling_up() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "imageDuration=4&transitionDuration=1&panDuration=2&fadeDuration=1",
            20260812,
        ));
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            assert!(
                f.batches.len() < 100,
                "{} batches, so sprites are piling up",
                f.batches.len()
            );
        }
        assert!(!r.frame().vertices.is_empty(), "it stopped drawing");
    }
}
