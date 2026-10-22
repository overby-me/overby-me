//! `selfreflect`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/selfreflect.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Let's self reflect
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/XfyXRV
//! Date:   11-May-2024
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/selfreflect.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "selfreflect",
        label: "Self Reflect",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2024",
            video: Some("https://www.youtube.com/watch?v=ZVbtTLHJimQ"),
            blurb: "Platonic solids with inner mirrors.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
