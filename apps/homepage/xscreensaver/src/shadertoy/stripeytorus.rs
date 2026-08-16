//! `stripeytorus`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/stripeytorus.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Saturday Torus
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/fd33zn
//! Date:   14-Aug-2021
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/stripeytorus.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "stripeytorus",
        label: "Stripey Torus",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2021",
            video: Some("https://www.youtube.com/watch?v=rVFg7F_6478"),
            blurb: "The stylish interior of a torus.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
