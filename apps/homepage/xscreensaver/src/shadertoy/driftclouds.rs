//! `driftclouds`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/driftclouds.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  2D Clouds
//! Author: drift
//! URL:    https://www.shadertoy.com/view/4tdSWr
//! Date:   12-Nov-2016
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/driftclouds.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "driftclouds",
        label: "Drift Clouds",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "drift",
            year: "2016",
            video: Some("https://www.youtube.com/watch?v=iKD0Fv37FtE"),
            blurb: "Clouds. Little fluffy clouds.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
