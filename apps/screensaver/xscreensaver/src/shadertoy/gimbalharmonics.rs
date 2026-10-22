//! `gimbalharmonics`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/gimbalharmonics.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Gimbal Harmonics
//! Author: otaviogood
//! URL:    https://www.shadertoy.com/view/llS3zd
//! Date:   12-May-2015
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/gimbalharmonics.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "gimbalharmonics",
        label: "Gimbal Harmonics",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2015",
            video: Some("https://www.youtube.com/watch?v=wMl2XUMlcLk"),
            blurb: "Disc-based visualization of different frequencies next to each other.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
