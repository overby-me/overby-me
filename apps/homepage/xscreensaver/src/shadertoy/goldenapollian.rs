//! `goldenapollian`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/goldenapollian.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Golden apollian
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/WlcfRS
//! Date:   09-Feb-2021
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/goldenapollian.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "goldenapollian",
        label: "Golden Apollian",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2021",
            video: Some("https://www.youtube.com/watch?v=k2aSDAeFuaQ"),
            blurb: "Enter the golden fractal doorways.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
