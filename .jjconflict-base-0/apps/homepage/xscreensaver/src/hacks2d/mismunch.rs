//! `mismunch`, which is [`super::munch`] with its broken half turned on.
//!
//! ```text
//! Munching errors!
//!
//! Copyright (c) 2004 Steven Hazel <sah@thalassocracy.org>
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
//! Upstream retired this one in version 5.08 and merged it into `munch`, which
//! since then has drawn either kind depending on a resource. The configuration
//! file for it is still shipped, though, and so is the name; this is that
//! name, pointed at the same code with the resource nailed down. Everything
//! that draws it is in [`super::munch`].

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{About, Opt, Runner, SaverDef, StartArgs};

const DEFAULTS: &[&str] = &[
    ".background:       black",
    ".foreground:       white",
    "*fpsSolid:	      true",
    "*delay:            10000",
    "*mismunch:         True",
    "*simul:            5",
    "*clear:            65",
    "*xor:              True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("clear", "Duration", 1.0, 200.0, 1.0, 0, "65"),
    Opt::slider("simul", "Simultaneous squares", 1.0, 20.0, 1.0, 0, "5"),
    Opt::select("xor", "Drawing mode", super::munch::DRAW_MODES, "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "mismunch",
    label: "Mismunch",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Steven Hazel",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=aXNIYpdh8Ug"),
        blurb: "Munching errors! This is a creatively broken misimplementation \
                of the classic munching squares graphics hack. See the Munch \
                screen saver for the original.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, super::munch::init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_only_ever_draws_the_broken_kind() {
        // `munch` rolls a coin for which of the two to draw and re-rolls it
        // whenever it starts over; this one has no coin to roll. The broken
        // one leaves square holes in the pattern, so the two never agree on
        // what the screen looks like.
        let mut mis = start(StartArgs::new(200, 200, "", 20260812));
        let mut plain =
            super::super::munch::start(StartArgs::new(200, 200, "mismunch=false", 20260812));
        let mut same = 0;
        for _ in 0..400 {
            mis.step();
            plain.step();
            if mis.dpy.win_ref().pixels() == plain.dpy.win_ref().pixels() {
                same += 1;
            }
        }
        assert!(same < 40, "the two drew the same thing {same} times of 400");
    }
}
