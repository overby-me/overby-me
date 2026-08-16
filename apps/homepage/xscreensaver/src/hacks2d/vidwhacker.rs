/* vidwhacker, Copyright © 1998-2026 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/vidwhacker`.
//!
//! A picture, wrecked: edge-detected, embossed, oil-painted, subtracted from a
//! mirror image of itself, multiplied by a plaid.
//!
//! Upstream is a shell script, and the interesting thing about it is that it
//! contains almost no image processing. It grabs a frame, picks one of
//! nineteen netpbm pipelines at random, and runs it. So the saver is the list
//! of pipelines, and the work of porting it is porting the tools: they are in
//! [`crate::runtime::netpbm`], ported from netpbm's own C.
//!
//! Where the picture comes from is the other half. Upstream reads a video
//! capture card, which is what dates it; here it is the same
//! `runtime::image` channel every other picture-consuming saver uses, so
//! `?images=@handle` whacks an account's photographs and `?images=%23tag`
//! whacks a hashtag.
//!
//! The pipelines are transcribed as written, including the one that computes
//! an intermediate and then throws it away.
//!
//! One of the tools had to be made faster to be usable at all. `pamoil` takes
//! the commonest value in a seven by seven window, and netpbm clears and then
//! searches a 256-entry histogram for every pixel of every plane, when at most
//! 49 of those entries can be non-zero. Clearing and searching only the
//! entries actually touched, and doing one plane rather than three when the
//! picture is grey (which both pipelines that use it guarantee, since they run
//! `ppmtopgm` first), takes the worst pipeline at 1280 by 800 from 1.9 seconds
//! to 0.49 and the oil one alone from 1.7 to 0.18. The output is identical:
//! the tie-break still has to go to the darker value, because netpbm scans in
//! value order and a running best would go to whichever was walked over
//! first.

use crate::runtime::fb::Fb;
use crate::runtime::netpbm::{self, Arith, Pnm};
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::{About, Dpy, ImageLoad, Opt, Runner, Saver, SaverDef, Screenhack, StartArgs};
#[cfg(target_arch = "wasm32")]
use crate::runtime::{About, Dpy, ImageLoad, Opt, Runner, SaverDef, Screenhack, StartArgs};
use crate::runtime::{XEvent, random};

/// How many craters pipeline 13 asks for.
const CRATERS: i32 = 20_000;

/// `randcolors`: a dark background and a bright foreground, for the pipelines
/// that colourise a greyscale.
fn rand_colors() -> ([u8; 3], [u8; 3]) {
    let d = || (random() % 60) as u8;
    let b = || 120 + (random() % 135) as u8;
    ([d(), d(), d()], [b(), b(), b()])
}

/// How many pipelines there are, which is the range the saver rolls in.
pub const PIPELINES: usize = 19;

/// Run one of upstream's nineteen filter pipelines over a picture.
///
/// The comment above each is the pipeline from the script, with `F1` for the
/// input file and `F2`-`F4` for the temporaries, exactly as upstream writes
/// them.
pub fn whack(n: usize, f1: &Pnm) -> Pnm {
    let (c0, c1) = rand_colors();
    let (w, h) = (f1.w, f1.h);
    let colorize = |p: &Pnm| netpbm::to_ppm(p, c0, c1);

    match n % PIPELINES {
        /* ppmtopgm FILE1 | pgmedge | pgmtoppm COLORS | ppmnorm */
        0 => netpbm::norm(&colorize(&netpbm::edge(&netpbm::to_pgm(f1)))),

        /* ppmtopgm FILE1 | pgmenhance | pgmtoppm COLORS */
        1 => colorize(&netpbm::enhance(&netpbm::to_pgm(f1))),

        /* ppmtopgm FILE1 | pgmoil | pgmtoppm COLORS */
        2 => colorize(&netpbm::oil(&netpbm::to_pgm(f1))),

        /* ppmtopgm FILE1 | pgmbentley | pgmtoppm COLORS */
        3 => colorize(&netpbm::bentley(&netpbm::to_pgm(f1))),

        /* ppmrelief FILE1 | ppmtopgm | pgmedge | ppmrelief | ppmtopgm |
        pgmedge | pnminvert | pgmtoppm COLORS */
        4 => {
            let a = netpbm::edge(&netpbm::to_pgm(&netpbm::relief(f1)));
            let b = netpbm::edge(&netpbm::to_pgm(&netpbm::relief(&a)));
            colorize(&netpbm::invert(&b))
        }

        /* ppmspread 71 FILE1 > FILE2 ; pnmarith -add FILE1 FILE2 */
        5 => {
            let f2 = netpbm::spread(f1, 71);
            netpbm::arith(f1, &f2, Arith::Add)
        }

        /* pnmflip -lr < FILE1 > FILE2 ;
        pnmarith -multiply FILE1 FILE2 > FILE3 ;
        pnmflip -tb FILE3 | ppmnorm > FILE2 ;
        pnmarith -multiply FILE1 FILE2 */
        6 => {
            let f2 = netpbm::flip_lr(f1);
            let f3 = netpbm::arith(f1, &f2, Arith::Multiply);
            let f2 = netpbm::norm(&netpbm::flip_tb(&f3));
            netpbm::arith(f1, &f2, Arith::Multiply)
        }

        /* pnmflip -lr FILE1 > FILE2 ; pnmarith -difference FILE1 FILE2 */
        7 => netpbm::arith(f1, &netpbm::flip_lr(f1), Arith::Difference),

        /* pnmflip -tb FILE1 > FILE2 ; pnmarith -difference FILE1 FILE2 */
        8 => netpbm::arith(f1, &netpbm::flip_tb(f1), Arith::Difference),

        /* pnmflip -lr FILE1 | pnmflip -tb > FILE2 ;
        pnmarith -difference FILE1 FILE2 */
        9 => {
            let f2 = netpbm::flip_tb(&netpbm::flip_lr(f1));
            netpbm::arith(f1, &f2, Arith::Difference)
        }

        /* ppmtopgm < FILE1 | pgmedge > FILE2 ;
        pnmarith -difference FILE1 FILE2 > FILE3 ;
        cp FILE3 FILE1 ;
        ppmtopgm < FILE1 | pgmedge > FILE2 ;
        pnmarith -difference FILE1 FILE2 > FILE3 ;
        ppmnorm < FILE1

        The second round writes FILE3 and then the pipeline outputs FILE1,
        so everything after the copy is discarded. Transcribed as it is,
        less the dead half: it cannot change what comes out. */
        10 => {
            let f2 = netpbm::edge(&netpbm::to_pgm(f1));
            let f3 = netpbm::arith(f1, &f2, Arith::Difference);
            netpbm::norm(&f3)
        }

        /* pnmflip -lr < FILE1 > FILE2 ;
        pnmarith -multiply FILE1 FILE2 | ppmrelief | ppmnorm | pnminvert */
        11 => {
            let f2 = netpbm::flip_lr(f1);
            let m = netpbm::arith(f1, &f2, Arith::Multiply);
            netpbm::invert(&netpbm::norm(&netpbm::relief(&m)))
        }

        /* pnmflip -lr FILE1 > FILE2 ;
        pnmarith -subtract FILE1 FILE2 | ppmrelief | ppmtopgm | pgmedge */
        12 => {
            let f2 = netpbm::flip_lr(f1);
            let s = netpbm::arith(f1, &f2, Arith::Subtract);
            netpbm::edge(&netpbm::to_pgm(&netpbm::relief(&s)))
        }

        /* pgmcrater -number 20000 -width WIDTH -height HEIGHT FILE1 |
          pgmtoppm COLORS > FILE2 ;
        pnmarith -difference FILE1 FILE2 > FILE3 ;
        pnmflip -tb FILE3 | ppmnorm > FILE2 ;
        pnmarith -multiply FILE1 FILE2

        `pgmcrater` is a generator; the FILE1 on the end of its arguments
        is vestigial and it reads nothing. */
        13 => {
            let f2 = colorize(&netpbm::crater(CRATERS, w, h));
            let f3 = netpbm::arith(f1, &f2, Arith::Difference);
            let f2 = netpbm::norm(&netpbm::flip_tb(&f3));
            netpbm::arith(f1, &f2, Arith::Multiply)
        }

        /* ppmshift 30 FILE1 | ppmtopgm | pgmoil | pgmedge |
          pgmtoppm COLORS > FILE2 ;
        pnmarith -difference FILE1 FILE2 */
        14 => {
            let a = netpbm::oil(&netpbm::to_pgm(&netpbm::shift(f1, 30)));
            let f2 = colorize(&netpbm::edge(&a));
            netpbm::arith(f1, &f2, Arith::Difference)
        }

        /* ppmpat -madras WIDTH HEIGHT | pnmdepth 255 > FILE2 ;
        pnmarith -difference FILE1 FILE2 */
        15 => netpbm::arith(f1, &netpbm::pat_madras(w, h), Arith::Difference),

        /* ppmpat -tartan WIDTH HEIGHT | pnmdepth 255 > FILE2 ;
        pnmarith -difference FILE1 FILE2 */
        16 => netpbm::arith(f1, &netpbm::pat_tartan(w, h), Arith::Difference),

        /* ppmpat -camo WIDTH HEIGHT | pnmdepth 255 | ppmshift 50 > FILE2 ;
        pnmarith -multiply FILE1 FILE2 */
        17 => {
            let f2 = netpbm::shift(&netpbm::pat_camo(w, h), 50);
            netpbm::arith(f1, &f2, Arith::Multiply)
        }

        /* pgmnoise WIDTH HEIGHT | pgmedge | pgmtoppm COLORS > FILE2 ;
        pnmarith -difference FILE1 FILE2 | pnmdepth 255 | pnmsmooth */
        _ => {
            let f2 = colorize(&netpbm::edge(&netpbm::noise(w, h)));
            netpbm::smooth(&netpbm::arith(f1, &f2, Arith::Difference))
        }
    }
}

struct VidwhackerState {
    /// Where the picture lands before it is wrecked.
    scratch: Fb,
    loader: Option<ImageLoad>,
    /// When to fetch the next one.
    next_at: f64,
    delay: f64,
    width: i32,
    height: i32,
}

impl Screenhack for VidwhackerState {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let waiting = self.loader.is_some();
        if waiting || d.time >= self.next_at {
            if !waiting {
                self.scratch = Fb::new(self.width, self.height);
            }
            let mut scratch = std::mem::replace(&mut self.scratch, Fb::new(1, 1));
            self.loader = d.load_image_into(&mut scratch, self.loader.take());
            self.scratch = scratch;
            if self.loader.is_none() {
                let src = Pnm::from_fb(&self.scratch);
                let out = whack(random() as usize, &src);
                out.to_fb(d.win());
                self.next_at = d.time + self.delay;
            }
        }
        100_000
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.scratch = Fb::new(width, height);
        self.next_at = 0.0;
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.win_ref().width(), d.win_ref().height());
    Box::new(VidwhackerState {
        scratch: Fb::new(width, height),
        loader: None,
        next_at: 0.0,
        delay: f64::from(d.res.int("delay2").max(1)),
        width,
        height,
    })
}

const DEFAULTS: &[&str] = &["*delay: 100000", "*delay2: 5", "*background: black"];

const OPTS: &[Opt] = &[Opt::slider("delay2", "Duration", 2.0, 120.0, 1.0, 0, "5")];

pub static DEF: SaverDef = SaverDef {
    slug: "vidwhacker",
    label: "Vid Whacker",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=u8esWjcR4eI"),
        blurb: "A picture put through a random series of filters.",
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

    fn picture(w: i32, h: i32) -> Pnm {
        let mut p = Pnm::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.set(
                    x,
                    y,
                    [
                        ((x * 255) / w.max(1)) as u8,
                        ((y * 255) / h.max(1)) as u8,
                        (((x + y) * 255) / (w + h).max(1)) as u8,
                    ],
                );
            }
        }
        p
    }

    /// Every pipeline runs, keeps the picture's size, and actually changes it.
    /// A pipeline that silently returned its input would look like a working
    /// saver on a quick glance and be doing nothing.
    #[test]
    fn every_pipeline_wrecks_the_picture() {
        crate::runtime::rand::ya_rand_init(20260812);
        let src = picture(96, 72);
        for n in 0..PIPELINES {
            let out = whack(n, &src);
            assert_eq!((out.w, out.h), (src.w, src.h), "pipeline {n} resized it");
            assert_ne!(out.px, src.px, "pipeline {n} did nothing");
        }
    }

    /// And none of them panics on a picture too small to have a neighbourhood,
    /// which is the shape every convolution here assumes.
    #[test]
    fn every_pipeline_survives_a_tiny_picture() {
        crate::runtime::rand::ya_rand_init(20260812);
        for (w, h) in [(1, 1), (2, 2), (3, 1), (1, 3), (4, 5)] {
            let src = picture(w, h);
            for n in 0..PIPELINES {
                let out = whack(n, &src);
                assert_eq!((out.w, out.h), (w, h), "pipeline {n} at {w}x{h}");
            }
        }
    }

    /// The mirror pipelines are symmetric by construction: a difference
    /// against a left-right flip is itself left-right symmetric, whatever the
    /// picture was. That is a property of the pipeline rather than of the
    /// numbers, so it is the one worth asserting.
    #[test]
    fn a_mirror_difference_is_symmetric() {
        crate::runtime::rand::ya_rand_init(20260812);
        let src = picture(64, 48);
        let out = whack(7, &src);
        for y in 0..out.h {
            for x in 0..out.w {
                assert_eq!(
                    out.get(x, y),
                    out.get(out.w - 1 - x, y),
                    "not symmetric at {x},{y}"
                );
            }
        }
        // And the top-bottom one the same way.
        let out = whack(8, &src);
        for y in 0..out.h {
            for x in 0..out.w {
                assert_eq!(out.get(x, y), out.get(x, out.h - 1 - y));
            }
        }
    }

    /// The saver draws something and then leaves it alone until the delay is
    /// up, which is the whole of its pacing.
    #[test]
    fn a_picture_stays_up_for_its_delay() {
        let mut r = Runner::start(&DEF, init, StartArgs::new(320, 240, "delay2=5", 20260812));
        r.step();
        let first = r.frame_hash();
        assert_ne!(first, 0);
        // At 100ms a frame, five seconds is fifty frames; well inside that
        // nothing should change.
        for _ in 0..20 {
            r.step();
        }
        assert_eq!(r.frame_hash(), first, "the picture changed early");
    }
}
