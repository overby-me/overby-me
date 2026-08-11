//! `darktransit`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/darktransit.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Dark Transit
//! Author: Matt Vianueva <diatribes@gmail.com>
//! URL:    https://www.shadertoy.com/view/WcdczB
//! Date:   21-Nov-2025
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/darktransit.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "darktransit",
        label: "Dark Transit",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Matt Vianueva",
            year: "2025",
            video: Some("https://www.youtube.com/watch?v=CObDhka6UW8"),
            blurb: "Flying through a generative cyber manifold.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
