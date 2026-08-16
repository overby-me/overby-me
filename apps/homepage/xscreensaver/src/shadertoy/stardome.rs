//! `stardome`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/stardome.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Stars and galaxy
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/stBcW1
//! Date:   10-Apr-2022
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/stardome.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "stardome",
        label: "Star Dome",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2022",
            video: Some("https://www.youtube.com/watch?v=qPvuutuMQHo"),
            blurb: "Stars and a galaxy under a dome on a moon.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
