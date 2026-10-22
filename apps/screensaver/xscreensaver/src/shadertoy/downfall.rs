//! `downfall`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/downfall.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Downfall
//! Author: Matt Vianueva <diatribes@gmail.com>
//! URL:    https://www.shadertoy.com/view/w3sBWl
//! Date:   01-Nov-2025
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/downfall.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "downfall",
        label: "Downfall",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Matt Vianueva",
            year: "2025",
            video: Some("https://www.youtube.com/watch?v=0eYnlyXlkb8"),
            blurb: "A close-up view of a grayscale waterfall.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
