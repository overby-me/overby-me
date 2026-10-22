//! `synthwavecity`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/synthwavecity.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Synthwave City
//! Author: 3w36zj6
//! URL:    https://www.shadertoy.com/view/7lKyDD
//! Date:   26-Aug-2022
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/synthwavecity.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "synthwavecity",
        label: "Synthwave City",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Jan Mroz and 3w36zj6",
            year: "2019",
            video: Some("https://www.youtube.com/watch?v=V0oWOW6pjgA"),
            blurb: "Let the neon wash over you.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
