//! `starnest`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/starnest.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Star Nest
//! Author: Kali
//! URL:    https://www.shadertoy.com/view/XlfGRj
//! Date:   16-Jun-2013
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/starnest.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "starnest",
        label: "Star Nest",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Pablo Roman Andrioli",
            year: "2013",
            video: Some("https://www.youtube.com/watch?v=EcntiVDT6Fo"),
            blurb: "A star field via 3D kaliset fractal and volumetric rendering.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
