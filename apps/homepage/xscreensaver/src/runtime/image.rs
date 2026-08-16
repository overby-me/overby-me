//! Where a saver's picture comes from.
//!
//! About thirty of the upstream hacks want an image to work on: they melt it,
//! zoom it, ripple it, cut it into sliding blocks. On a desktop
//! `load_image_async` grabs the screen or a file from your pictures directory;
//! on mobile xscreensaver ships a small bundled set instead.
//!
//! This crate has no way to fetch anything, and should not: it is
//! dependency-free and runs in a `cargo test` with no browser. So the contract
//! is a channel. A hack asks for an image, the host notices the request and
//! answers whenever it can, and the runtime draws whatever arrives into the
//! window. If nothing is going to answer, the hack gets colour bars, which is
//! what upstream falls back to as well (`utils/colorbars.c`).
//!
//! That keeps the whole collection testable: the native tests never register a
//! host, so every image-consuming saver runs against the test card.

use super::color::{Pixel, rgb};
use super::fb::{Fb, Gc, XImage, XRectangle};

/// A request for an image, held by the hack while it waits.
///
/// Mirrors upstream's `async_load_state *`: you get one when you ask, you hand
/// it back each frame, and you get `None` once the image has landed.
#[derive(Debug, PartialEq)]
pub struct ImageLoad {
    /// When the request went out, so a host that never answers does not leave a
    /// saver staring at a blank screen forever.
    started: f64,
}

/// How long to wait for a host before falling back to colour bars, in seconds.
///
/// In seconds rather than frames because hacks draw at wildly different rates,
/// and this is a network timeout: long enough to fetch a photograph over a slow
/// connection, short enough that a broken source does not look like a hang.
/// The host reads this too: a picture that arrives after it is a picture the
/// hack has already given up on, and the page restarts the hack rather than
/// leave the colour bars up until it next asks.
pub const PATIENCE: f64 = 20.0;

/// The runtime's half of the channel. Lives on [`crate::Dpy`].
#[derive(Default)]
pub struct ImageChannel {
    /// Set when the host has said it can supply images. Without it a request is
    /// answered with colour bars on the spot, which is what makes the native
    /// tests work without ceremony.
    pub(crate) host_supplies: bool,
    /// Set by a hack asking for an image; cleared when the host takes it.
    pub(crate) requested: bool,
    /// Set the first time a hack asks for a picture, whether or not there was
    /// a host to ask. Unlike `requested` it is never cleared: the page reads
    /// it to decide whether this saver is one of the thirty that work on a
    /// picture, and so whether to offer the controls for choosing one.
    pub(crate) ever_wanted: bool,
    /// Filled by the host, consumed by the next `load_image_async_simple`.
    pub(crate) ready: Option<XImage>,
    /// The most recent image's caption, if the host supplied one.
    pub(crate) title: Option<String>,
    /// Where in the window the last picture actually landed.
    ///
    /// Upstream's `load_image_async_simple` takes this as an out-parameter,
    /// because a photograph rarely has the window's aspect ratio and the hacks
    /// that care want to know which part of the window is picture and which is
    /// the black either side of it.
    pub(crate) geometry: XRectangle,
}

impl ImageChannel {
    /// `load_image_async_simple`. Pass `None` to start a load, then hand back
    /// what you got until it returns `None`, at which point the image has been
    /// drawn into `window`.
    pub(crate) fn poll(
        &mut self,
        window: &mut Fb,
        now: f64,
        pending: Option<ImageLoad>,
    ) -> Option<ImageLoad> {
        let whole = XRectangle {
            x: 0,
            y: 0,
            width: window.width(),
            height: window.height(),
        };
        let Some(load) = pending else {
            self.ever_wanted = true;
            if self.host_supplies {
                self.requested = true;
                return Some(ImageLoad { started: now });
            }
            // Nobody to ask.
            draw_colorbars(window);
            self.geometry = whole;
            return None;
        };

        if let Some(img) = self.ready.take() {
            self.geometry = draw_centred(window, &img);
            return None;
        }

        if now - load.started >= PATIENCE {
            draw_colorbars(window);
            self.geometry = whole;
            return None;
        }
        Some(load)
    }
}

/// Draw an image centred in the window, scaled down to fit if it is larger, and
/// return the rectangle it landed in.
///
/// Upstream's loader hands the hack an image already fitted to the window by
/// `xscreensaver-getimage`; doing it here keeps that contract with a host that
/// just decoded whatever size the source happened to be.
fn draw_centred(window: &mut Fb, img: &XImage) -> XRectangle {
    let (ww, wh) = (window.width(), window.height());
    let (iw, ih) = (img.width(), img.height());
    window.clear(0xFF00_0000);
    if iw <= 0 || ih <= 0 {
        return XRectangle {
            x: 0,
            y: 0,
            width: ww,
            height: wh,
        };
    }

    // Fit inside the window, never enlarging: an upscaled phone photo looks
    // worse than a smaller sharp one, and the hacks do not care about size.
    let scale = (ww as f64 / iw as f64).min(wh as f64 / ih as f64).min(1.0);
    let (dw, dh) = (
        ((iw as f64 * scale) as i32).max(1),
        ((ih as f64 * scale) as i32).max(1),
    );
    let (ox, oy) = ((ww - dw) / 2, (wh - dh) / 2);

    for y in 0..dh {
        let sy = (y as f64 / scale) as i32;
        for x in 0..dw {
            let sx = (x as f64 / scale) as i32;
            window.put_pixel(ox + x, oy + y, img.get_pixel(sx, sy));
        }
    }

    XRectangle {
        x: ox,
        y: oy,
        width: dw,
        height: dh,
    }
}

/// SMPTE colour bars, the stand-in when there is no image to be had.
///
/// Port of the shape of `utils/colorbars.c`, without its xscreensaver logo:
/// seven bars across the top two thirds, the reversed set below, and the
/// pluge/black strip at the bottom.
pub fn draw_colorbars(window: &mut Fb) {
    const TOP: [Pixel; 7] = [
        rgb(192, 192, 192),
        rgb(192, 192, 0),
        rgb(0, 192, 192),
        rgb(0, 192, 0),
        rgb(192, 0, 192),
        rgb(192, 0, 0),
        rgb(0, 0, 192),
    ];
    const MIDDLE: [Pixel; 7] = [
        rgb(0, 0, 192),
        rgb(19, 19, 19),
        rgb(192, 0, 192),
        rgb(19, 19, 19),
        rgb(0, 192, 192),
        rgb(19, 19, 19),
        rgb(192, 192, 192),
    ];
    const BOTTOM: [Pixel; 7] = [
        rgb(0, 33, 76),
        rgb(255, 255, 255),
        rgb(50, 0, 106),
        rgb(19, 19, 19),
        rgb(9, 9, 9),
        rgb(19, 19, 19),
        rgb(29, 29, 29),
    ];

    let (w, h) = (window.width(), window.height());
    let mut gc = Gc::default();
    let band = |window: &mut Fb, gc: &mut Gc, colors: &[Pixel; 7], y: i32, height: i32| {
        if height <= 0 {
            return;
        }
        for (i, c) in colors.iter().enumerate() {
            let x0 = (w as i64 * i as i64 / 7) as i32;
            let x1 = (w as i64 * (i as i64 + 1) / 7) as i32;
            gc.set_foreground(*c);
            window.fill_rectangle(gc, x0, y, x1 - x0, height);
        }
    };

    let top_h = h * 2 / 3;
    let mid_h = h / 12;
    band(window, &mut gc, &TOP, 0, top_h);
    band(window, &mut gc, &MIDDLE, top_h, mid_h);
    band(window, &mut gc, &BOTTOM, top_h + mid_h, h - top_h - mid_h);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::color::ALPHA;

    #[test]
    fn colorbars_fill_the_window() {
        let mut fb = Fb::new(70, 60);
        draw_colorbars(&mut fb);
        assert!(
            fb.pixels().iter().all(|p| *p & ALPHA == ALPHA),
            "left a hole"
        );
        // Seven distinct bars across the top.
        let row: std::collections::HashSet<Pixel> = (0..70).map(|x| fb.get_pixel(x, 5)).collect();
        assert_eq!(row.len(), 7);
    }

    #[test]
    fn colorbars_survive_a_tiny_window() {
        for (w, h) in [(1, 1), (3, 1), (1, 40), (7, 2)] {
            let mut fb = Fb::new(w, h);
            draw_colorbars(&mut fb);
        }
    }

    #[test]
    fn without_a_host_a_request_is_answered_at_once() {
        let mut ch = ImageChannel::default();
        let mut fb = Fb::new(64, 48);
        assert_eq!(
            ch.poll(&mut fb, 0.0, None),
            None,
            "should not wait for nobody"
        );
        assert!(
            fb.pixels().iter().any(|p| *p != ALPHA),
            "no test card drawn"
        );
    }

    /// A hack says it works on a picture by asking for one, and it says so
    /// whether or not anybody was listening. The page needs the answer before
    /// a source is configured, which is exactly when there is no host.
    #[test]
    fn a_hack_that_asks_says_so_with_no_host_at_all() {
        let mut ch = ImageChannel::default();
        let mut fb = Fb::new(64, 48);
        assert!(!ch.ever_wanted, "nothing has asked yet");

        assert!(ch.poll(&mut fb, 0.0, None).is_none(), "no host to wait for");
        assert!(ch.ever_wanted, "the hack asked and it went unrecorded");
    }

    #[test]
    fn with_a_host_the_request_waits_for_the_image() {
        let mut ch = ImageChannel {
            host_supplies: true,
            ..Default::default()
        };
        let mut fb = Fb::new(64, 48);

        let pending = ch.poll(&mut fb, 0.0, None);
        assert!(pending.is_some());
        assert!(ch.requested, "host was not told");

        let pending = ch.poll(&mut fb, 0.5, pending);
        assert!(pending.is_some(), "gave up before the image arrived");

        let mut img = XImage::new(32, 24);
        img.clear(rgb(10, 200, 30));
        ch.ready = Some(img);
        assert_eq!(ch.poll(&mut fb, 1.0, pending), None, "image did not land");
        assert_eq!(fb.get_pixel(32, 24), rgb(10, 200, 30), "not centred");
        assert_eq!(fb.get_pixel(1, 1), ALPHA, "should be letterboxed");
    }

    #[test]
    fn a_host_that_never_answers_still_gets_a_picture() {
        let mut ch = ImageChannel {
            host_supplies: true,
            ..Default::default()
        };
        let mut fb = Fb::new(64, 48);
        let mut pending = ch.poll(&mut fb, 0.0, None);
        let mut now = 0.0;
        while pending.is_some() && now < PATIENCE * 2.0 {
            now += 1.0;
            pending = ch.poll(&mut fb, now, pending);
        }
        assert_eq!(pending, None, "waited forever");
        assert!(now <= PATIENCE + 1.0, "gave up late, after {now}s");
        assert!(fb.pixels().iter().any(|p| *p != ALPHA));
    }

    #[test]
    fn a_huge_image_is_scaled_down_a_small_one_is_not() {
        let mut fb = Fb::new(64, 48);
        let mut big = XImage::new(640, 480);
        big.clear(rgb(200, 0, 0));
        draw_centred(&mut fb, &big);
        assert_eq!(fb.get_pixel(0, 0), rgb(200, 0, 0), "should fill the window");

        let mut fb = Fb::new(64, 48);
        let mut small = XImage::new(8, 8);
        small.clear(rgb(0, 0, 200));
        draw_centred(&mut fb, &small);
        assert_eq!(fb.get_pixel(32, 24), rgb(0, 0, 200));
        assert_eq!(fb.get_pixel(0, 0), ALPHA, "should not have been enlarged");
    }
}
