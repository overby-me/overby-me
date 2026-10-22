//! `batteredplanet`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/batteredplanet.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  A battered alien planet
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/wsjBD3
//! Date:   01-Jun-2020
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/batteredplanet.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "batteredplanet",
        label: "Battered Planet",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2020",
            video: Some("https://www.youtube.com/watch?v=FIj2oOuIXvw"),
            blurb: "An alien planet full of craters, with other celestial objects in the sky.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
