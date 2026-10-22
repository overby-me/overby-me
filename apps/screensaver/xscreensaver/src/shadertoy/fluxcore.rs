//! `fluxcore`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/fluxcore.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Flux Core
//! Author: otaviogood
//! URL:    https://www.shadertoy.com/view/ltlSWf
//! Date:   24-Aug-2015
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/fluxcore.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "fluxcore",
        label: "Flux Core",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2015",
            video: Some("https://www.youtube.com/watch?v=LHsx3JGB2is"),
            blurb: "Long range space-based energy transmission requires a flux core to amplify and concentrate energy.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
