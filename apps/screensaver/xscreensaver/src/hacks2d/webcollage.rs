/* webcollage, Copyright © 1999-2026 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/webcollage` and `hacks/webcollage-helper.c`.
//!
//! Pictures pasted over one another, at random sizes, in random places, each
//! one a little transparent and softened at its edges, until the screen is a
//! wall of them.
//!
//! Upstream is two programs. A 3,900-line perl script finds images by feeding
//! random words to search engines and pulling the pictures out of the pages
//! that come back, and a C helper scales, crops, bevels and composites one
//! picture into the collage. This is a port of the *second* one, plus the
//! placement arithmetic out of the first.
//!
//! The search-engine half cannot be ported and does not need to be. It is the
//! part of the program that answers "where does the next picture come from",
//! and this crate already has an answer: `runtime::image` is a channel, and
//! the host fills it from atproto, so `?images=@handle` collages an account's
//! photographs and `?images=%23caturday` collages a hashtag as people post to
//! it. Upstream's own description is "this is what the Internet looks like",
//! and the internet has moved; the saver has not changed at all.
//!
//! That also disposes of the warning in its manual. Upstream feeds random
//! words to image searches and says, in capitals, that the internet sometimes
//! contains pornography. A feed you name yourself is a feed you chose.
//!
//! What is ported exactly is the arithmetic, because it is the whole look of
//! the thing: the repeated halving that gives a wide spread of sizes rather
//! than a uniform one, the crop chance that rises for big images and rises a
//! lot for banner-shaped ones, the bell-distributed crop rectangle, the paste
//! position that deliberately hangs pictures off the edges of the screen, and
//! the sinusoidal bevel that stops every picture having a hard rectangular
//! border.

use crate::runtime::color::Pixel;
use crate::runtime::fb::Fb;
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::{About, Dpy, ImageLoad, Opt, Runner, Saver, SaverDef, Screenhack, StartArgs};
#[cfg(target_arch = "wasm32")]
use crate::runtime::{About, Dpy, ImageLoad, Opt, Runner, SaverDef, Screenhack, StartArgs};
use crate::runtime::{XEvent, frand};

/// The smallest crop upstream will take out of a picture.
const MIN_WIDTH: i32 = 50;
const MIN_HEIGHT: i32 = 50;

/// Below this aspect ratio a picture is treated as a banner: cropped almost
/// always, and cropped uniformly rather than towards its middle.
const MIN_RATIO: f64 = 1.0 / 5.0;

/// `bevel_pct` in `webcollage-helper.c`, where it is a literal with a `####`
/// next to it.
const BEVEL_PCT: i32 = 10;

/// `bellrand`: three uniform numbers averaged, so the middle is likelier than
/// the ends.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

/// One picture's placement, worked out before any pixel is touched.
///
/// Upstream computes this in perl and passes it to the helper on the command
/// line, which is why it is a struct here rather than a tangle of locals: the
/// division is upstream's own.
#[derive(Debug, PartialEq)]
struct Paste {
    /// How much the source is reduced, always 1, 1/2, 1/4...
    scale: i32,
    /// The sub-rectangle of the *scaled* image to paste.
    crop_x: i32,
    crop_y: i32,
    crop_w: i32,
    crop_h: i32,
    /// Where it lands in the window.
    x: i32,
    y: i32,
}

/// The placement arithmetic from `paste_image`, given an image already known
/// to be `iw` by `ih` in a window `w` by `h`.
///
/// Returns `None` when upstream would have given up on the picture, which it
/// does when halving it to fit has made it absurd.
fn plan_paste(mut iw: i32, mut ih: i32, w: i32, h: i32) -> Option<Paste> {
    /* Usually scale the image to fit on the screen -- but sometimes scale it
    to fit on half or a quarter of the screen.  (We do this by reducing the
    size of the target rectangle.)  Note that the image is not merely scaled
    to fit; we instead cut the image in half repeatedly until it fits in the
    target rectangle -- that gives a wider distribution of sizes. */
    let (mut target_w, mut target_h) = (w, h);
    if frand(1.0) < 0.3 {
        target_w /= 2;
        target_h /= 2;
    }
    if frand(1.0) < 0.3 {
        target_w /= 2;
        target_h /= 2;
    }

    let mut scale = 1;
    if iw > target_w || ih > target_h {
        while iw > target_w || ih > target_h {
            iw /= 2;
            ih /= 2;
            scale *= 2;
        }
        if iw <= 10 || ih <= 10 {
            return None; /* would have been bogus */
        }
    }

    let (mut crop_x, mut crop_y) = (0, 0);
    let (mut crop_w, mut crop_h) = (iw, ih);

    /* The chance that we will randomly crop out a section of an image starts
    out fairly low, but goes up for images that are very large, or images
    that have ratios that make them look like banners. */
    let mut crop_chance = 0.2;
    if f64::from(iw) > f64::from(w) * 0.4 || f64::from(ih) > f64::from(h) * 0.4 {
        crop_chance += 0.2;
    }
    if f64::from(iw) > f64::from(w) * 0.7 || f64::from(ih) > f64::from(h) * 0.7 {
        crop_chance += 0.2;
    }
    let banner = f64::from(iw) * MIN_RATIO > f64::from(ih);
    if banner {
        crop_chance += 0.7;
    }

    if frand(1.0) < crop_chance {
        let (ow, oh) = (crop_w, crop_h);
        if crop_w > MIN_WIDTH {
            /* if it's a banner, select the width linearly.
            otherwise, select a bell. */
            let r = if banner { frand(1.0) } else { bellrand(1.0) };
            crop_w = MIN_WIDTH + (r * f64::from(crop_w - MIN_WIDTH)) as i32;
            crop_x = (frand(1.0) * f64::from(ow - crop_w)) as i32;
        }
        if crop_h > MIN_HEIGHT {
            /* height always selects as a bell. */
            crop_h = MIN_HEIGHT + (bellrand(1.0) * f64::from(crop_h - MIN_HEIGHT)) as i32;
            crop_y = (frand(1.0) * f64::from(oh - crop_h)) as i32;
        }

        /* Clip it to the actual post-scaling image size. */
        if crop_x + crop_w > iw {
            crop_w = iw - crop_x;
        }
        if crop_y + crop_h > ih {
            crop_h = ih - crop_y;
        }
        if crop_x < 0 {
            crop_w += crop_x;
            crop_x = 0;
        }
        if crop_y < 0 {
            crop_h += crop_y;
            crop_y = 0;
        }
    }

    /* Where the image should logically land -- this might be negative. */
    let mut x = (frand(1.0) * f64::from(w + crop_w / 2)) as i32 - crop_w * 3 / 4;
    let mut y = (frand(1.0) * f64::from(h + crop_h / 2)) as i32 - crop_h * 3 / 4;

    /* if we have chosen to paste the image outside of the rectangle of the
    screen, then we need to crop it. */
    if x < 0 || y < 0 || x + crop_w > w || y + crop_h > h {
        if x < 0 {
            crop_x -= x;
            crop_w += x;
            x = 0;
        }
        if y < 0 {
            crop_y -= y;
            crop_h += y;
            y = 0;
        }
        if x + crop_w >= w {
            crop_w = w - x - 1;
        }
        if y + crop_h >= h {
            crop_h = h - y - 1;
        }
    }

    if crop_w <= 0 || crop_h <= 0 {
        return None;
    }

    Some(Paste {
        scale,
        crop_x,
        crop_y,
        crop_w,
        crop_h,
        x,
        y,
    })
}

/// `bevel_image`: the alpha ramp along one axis, as a fraction of full.
///
/// Upstream has a linear version behind an `#if 0` and a sinusoidal one live;
/// this is the live one. A bevel smaller than five pixels is not worth having
/// and upstream returns without doing anything, which is what a zero-length
/// ramp means here.
fn bevel_size(w: i32, h: i32) -> i32 {
    let small_size = w.min(h);
    let mut bevel = small_size * BEVEL_PCT / 100;

    /* Use a proportionally larger bevel size for especially small images. */
    if bevel < 20 && small_size > 40 {
        bevel = 20;
    } else if bevel < 10 && small_size > 20 {
        bevel = 10;
    } else if bevel < 5 {
        return 0; /* too small to bother bevelling */
    }
    bevel
}

/// How much of the picture shows at one position inside it, along one axis.
fn bevel_ramp(i: i32, extent: i32, bevel: i32) -> f64 {
    if bevel == 0 {
        return 1.0;
    }
    let at = if i < bevel {
        i
    } else if i >= extent - bevel {
        extent - i - 1
    } else {
        return 1.0;
    };
    let p = f64::from(at) / f64::from(bevel);
    (p * std::f64::consts::FRAC_PI_2).sin()
}

struct WebcollageState {
    /// Where each picture lands before it is pasted. The window is the collage
    /// itself and is never cleared, so the two cannot be the same buffer.
    scratch: Fb,
    loader: Option<ImageLoad>,
    /// Upstream pastes its first picture whole, scaled to fill, so the collage
    /// starts from a background rather than from black.
    first: bool,
    /// When the next picture may be pasted.
    next_at: f64,
    delay: f64,
    opacity: f64,
    width: i32,
    height: i32,
}

impl WebcollageState {
    /// The compositing half of `paste_image`: `webcollage-helper.c`'s
    /// `paste`, with the bevel folded into the per-pixel alpha rather than
    /// written into an alpha channel the framebuffer does not have.
    fn paste(&mut self, d: &mut Dpy, p: &Paste, src: crate::runtime::XRectangle) {
        let bevel = bevel_size(p.crop_w, p.crop_h);
        for dy in 0..p.crop_h {
            let ry = bevel_ramp(dy, p.crop_h, bevel);
            for dx in 0..p.crop_w {
                let rx = bevel_ramp(dx, p.crop_w, bevel);
                let a = self.opacity * rx * ry;

                // The source is the unscaled picture, so a scaled-down paste
                // reads every `scale`th pixel of it. Upstream scales the image
                // first and then reads it one for one; the result is the same
                // sampling, without a second buffer.
                let sx = src.x + (p.crop_x + dx) * p.scale;
                let sy = src.y + (p.crop_y + dy) * p.scale;
                if sx < 0 || sy < 0 || sx >= src.x + src.width || sy >= src.y + src.height {
                    continue;
                }
                let s = self.scratch.get_pixel(sx, sy);
                let dst = d.win_ref().get_pixel(p.x + dx, p.y + dy);
                d.win().put_pixel(p.x + dx, p.y + dy, blend(dst, s, a));
            }
        }
    }

    /// The first paste, upstream's `init_p`: the whole picture, scaled to fill
    /// the window, opaque and unbevelled.
    fn paste_first(&mut self, d: &mut Dpy, src: crate::runtime::XRectangle) {
        let (w, h) = (self.width, self.height);
        if src.width <= 0 || src.height <= 0 {
            return;
        }
        for y in 0..h {
            let sy = src.y + y * src.height / h;
            for x in 0..w {
                let sx = src.x + x * src.width / w;
                let p = self.scratch.get_pixel(sx, sy);
                d.win().put_pixel(x, y, p);
            }
        }
    }
}

/// Composite one pixel over another at the given alpha.
fn blend(dst: Pixel, src: Pixel, a: f64) -> Pixel {
    let f = |shift: u32| {
        let (d, s) = ((dst >> shift) & 0xff, (src >> shift) & 0xff);
        let v = f64::from(d) * (1.0 - a) + f64::from(s) * a;
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    f(16) | f(8) | f(0)
}

impl Screenhack for WebcollageState {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // A load is either in flight or due. Either way the poll is the same
        // call; upstream's loader signals completion by returning None, and
        // with no host that happens on the very first one.
        let waiting = self.loader.is_some();
        if waiting || d.time >= self.next_at {
            if !waiting {
                self.scratch = Fb::new(self.width, self.height);
            }
            let mut scratch = std::mem::replace(&mut self.scratch, Fb::new(1, 1));
            self.loader = d.load_image_into(&mut scratch, self.loader.take());
            self.scratch = scratch;
            if self.loader.is_none() {
                self.landed(d);
            }
        }

        100_000
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.scratch = Fb::new(width, height);
        let _ = d;
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

impl WebcollageState {
    /// A picture has arrived in the scratch buffer: place it and paste it.
    fn landed(&mut self, d: &mut Dpy) {
        let src = d.image_geometry();
        if self.first {
            self.paste_first(d, src);
            self.first = false;
        } else if let Some(p) = plan_paste(src.width, src.height, self.width, self.height) {
            self.paste(d, &p, src);
        }
        self.next_at = d.time + self.delay;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.win_ref().width(), d.win_ref().height());
    Box::new(WebcollageState {
        scratch: Fb::new(width, height),
        loader: None,
        first: true,
        next_at: 0.0,
        delay: d.res.float("delay2").max(0.0),
        opacity: d.res.float("opacity").clamp(0.1, 1.0),
        width,
        height,
    })
}

const DEFAULTS: &[&str] = &[
    "*delay:     100000",
    "*delay2:    2",
    "*opacity:   0.85",
    "*background: black",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay2", "Delay between images", 0.0, 30.0, 1.0, 0, "2"),
    Opt::slider("opacity", "Image opacity", 0.1, 1.0, 0.05, 2, "0.85"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "webcollage",
    label: "Web Collage",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=u8esWjcR4eI"),
        blurb: "Pictures pasted over one another until the screen is a wall of them.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// Halving to fit gives a wide spread of sizes, which is the point of
    /// doing it that way rather than scaling to fit: every result is the
    /// original over a power of two, and the target rectangle it is fitting
    /// into is itself sometimes halved.
    #[test]
    fn scaling_is_always_a_power_of_two() {
        crate::runtime::rand::ya_rand_init(20260812);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2000 {
            if let Some(p) = plan_paste(1600, 1200, 640, 480) {
                assert!(
                    p.scale > 0 && (p.scale as u32).is_power_of_two(),
                    "scale {} is not",
                    p.scale
                );
                seen.insert(p.scale);
            }
        }
        assert!(
            seen.len() > 2,
            "only {seen:?} came up, so the halved targets are not being used"
        );
    }

    /// A banner is cropped almost always: 0.2 base, plus 0.2 and 0.2 for being
    /// large, plus 0.7 for the ratio, which saturates the roll.
    #[test]
    fn banners_are_always_cropped() {
        crate::runtime::rand::ya_rand_init(20260812);
        let mut cropped = 0;
        let n = 400;
        for _ in 0..n {
            // Wide enough that iw * (1/5) > ih.
            if let Some(p) = plan_paste(600, 60, 640, 480) {
                if p.crop_w < 600 || p.crop_h < 60 {
                    cropped += 1;
                }
            }
        }
        assert!(
            cropped > n * 3 / 4,
            "only {cropped} of {n} banners were cropped"
        );
    }

    /// Every plan lands inside the window. Upstream deliberately places
    /// pictures hanging off the edges and then crops them back, so this is the
    /// arithmetic that has to hold rather than an accident of the numbers.
    #[test]
    fn a_paste_never_leaves_the_window() {
        crate::runtime::rand::ya_rand_init(20260812);
        let (w, h) = (640, 480);
        for _ in 0..5000 {
            let iw = 20 + (frand(3000.0) as i32);
            let ih = 20 + (frand(3000.0) as i32);
            let Some(p) = plan_paste(iw, ih, w, h) else {
                continue;
            };
            assert!(p.x >= 0 && p.y >= 0, "{p:?} starts outside");
            assert!(
                p.x + p.crop_w <= w && p.y + p.crop_h <= h,
                "{p:?} runs off a {w}x{h} window"
            );
            assert!(p.crop_w > 0 && p.crop_h > 0, "{p:?} is empty");
            assert!(
                p.crop_x >= 0 && p.crop_y >= 0,
                "{p:?} reads before the image"
            );
        }
    }

    /// The bevel is a quarter sine: nothing at the very edge, everything one
    /// bevel in, and monotonic between.
    #[test]
    fn the_bevel_ramps_up_and_stays_up() {
        let bevel = bevel_size(200, 200);
        assert!(bevel > 0);
        assert_eq!(bevel_ramp(0, 200, bevel), 0.0);
        let mut prev = -1.0;
        for i in 0..bevel {
            let r = bevel_ramp(i, 200, bevel);
            assert!(r > prev, "the ramp went backwards at {i}");
            prev = r;
        }
        assert_eq!(bevel_ramp(100, 200, bevel), 1.0);
        // Symmetric: the far edge fades the same way.
        for i in 0..bevel {
            assert!(
                (bevel_ramp(i, 200, bevel) - bevel_ramp(199 - i, 200, bevel)).abs() < 1e-12,
                "the two ends differ at {i}"
            );
        }
    }

    /// A picture too small to bevel is left alone rather than being faded to
    /// nothing, which is what upstream's early return means.
    #[test]
    fn a_tiny_picture_is_not_bevelled() {
        assert_eq!(bevel_size(8, 8), 0);
        assert_eq!(bevel_ramp(0, 8, 0), 1.0);
    }

    /// A paste touches its own rectangle and nothing else. Upstream places
    /// pictures deliberately overhanging the screen and crops them back, so
    /// the clipping in `plan_paste` is load bearing: get it wrong and the
    /// collage writes outside the window or reads outside the picture.
    #[test]
    fn a_paste_stays_inside_its_rectangle() {
        crate::runtime::rand::ya_rand_init(20260812);
        let (w, h) = (200, 150);
        let mut d = Dpy::new(w, h, crate::runtime::Resources::new(DEFAULTS, OPTS, ""));
        let mut st = WebcollageState {
            scratch: Fb::filled(w, h, 0x00ff_00ff),
            loader: None,
            first: false,
            next_at: 0.0,
            delay: 0.0,
            opacity: 1.0,
            width: w,
            height: h,
        };
        let src = crate::runtime::XRectangle {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let p = Paste {
            scale: 1,
            crop_x: 0,
            crop_y: 0,
            crop_w: 40,
            crop_h: 30,
            x: 20,
            y: 10,
        };
        let before: Vec<u32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| d.win_ref().get_pixel(x, y))
            .collect();
        st.paste(&mut d, &p, src);

        for y in 0..h {
            for x in 0..w {
                let inside = x >= p.x && x < p.x + p.crop_w && y >= p.y && y < p.y + p.crop_h;
                let changed = d.win_ref().get_pixel(x, y) != before[(y * w + x) as usize];
                assert!(
                    !(changed && !inside),
                    "pixel {x},{y} was painted outside the paste rectangle"
                );
            }
        }
        // And the middle of it really was painted, so the check above is not
        // passing because nothing happened at all.
        let mid = ((p.y + 15) * w + p.x + 20) as usize;
        assert_ne!(d.win_ref().get_pixel(p.x + 20, p.y + 15), before[mid]);
    }

    /// The collage keeps taking pictures: the window goes on changing rather
    /// than settling after the first one.
    #[test]
    fn the_collage_keeps_pasting() {
        let mut r = Runner::start(&DEF, init, StartArgs::new(640, 480, "", 20260812));
        for _ in 0..20 {
            r.step();
        }
        let first = r.frame_hash();
        let mut changed = false;
        for _ in 0..200 {
            r.step();
            if r.frame_hash() != first {
                changed = true;
            }
        }
        assert!(changed, "the collage stopped after its first picture");
    }
}
