//! `truchetzoom`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/truchetzoom.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Another truchet experiment
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/4cBcDy
//! Date:   05-Aug-2024
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/truchetzoom.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "truchetzoom",
        label: "Truchet Zoom",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2024",
            video: Some("https://www.youtube.com/watch?v=ts6A_H7R64w"),
            blurb: "A looping, distorting truchet chain with scanlines.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
