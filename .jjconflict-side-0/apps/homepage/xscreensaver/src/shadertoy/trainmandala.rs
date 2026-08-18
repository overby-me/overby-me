//! `trainmandala`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/trainmandala.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  CC0: Quick hack on the train
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/mtjyz1
//! Date:   08-Aug-2023
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/trainmandala.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "trainmandala",
        label: "Train Mandala",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2023",
            video: Some("https://www.youtube.com/watch?v=6p3JbylR3jI"),
            blurb: "Enter the flowing ring.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
