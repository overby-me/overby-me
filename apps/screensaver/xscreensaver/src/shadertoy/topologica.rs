//! `topologica`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/topologica.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Topologica
//! Author: otaviogood
//! URL:    https://www.shadertoy.com/view/4djXzz
//! Date:   20-Aug-2014
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/topologica.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "topologica",
        label: "Topologica",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2014",
            video: Some("https://www.youtube.com/watch?v=wBJD2NZTzgI"),
            blurb: "Pulsing lines of light.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
