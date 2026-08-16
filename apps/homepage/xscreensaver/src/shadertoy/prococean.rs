//! `prococean`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/prococean.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Very fast procedural ocean
//! Author: afl_ext
//! URL:    https://www.shadertoy.com/view/MdXyzX
//! Date:   09-Mar-2017
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/prococean.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "prococean",
        label: "Proc Ocean",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "afl_ext",
            year: "2017",
            video: Some("https://www.youtube.com/watch?v=Ho0Obet_TZU"),
            blurb: "A very fast procedural ocean.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
