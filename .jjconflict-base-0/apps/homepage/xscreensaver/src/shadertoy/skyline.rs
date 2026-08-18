//! `skyline`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/skyline.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Skyline
//! Author: otaviogood
//! URL:    https://www.shadertoy.com/view/XtsSWs
//! Date:   23-Sep-2015
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/skyline.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "skyline",
        label: "Skyline",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2015",
            video: Some("https://www.youtube.com/watch?v=sao3ZCm2vTY"),
            blurb: "A procedurally-generated cityscape.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
