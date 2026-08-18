//! `logarithmiccircles`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/logarithmiccircles.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  B/W logarithmic circles II
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/mljcWR
//! Date:   10-Aug-2023
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/logarithmiccircles.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "logarithmiccircles",
        label: "Logarithmic Circles",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2023",
            video: Some("https://www.youtube.com/watch?v=8E1sgc_GISk"),
            blurb: "Zooming black and white circles.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
