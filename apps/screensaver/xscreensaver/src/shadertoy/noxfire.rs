//! `noxfire`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/noxfire.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Nox Fire
//! Author: Matt Vianueva <diatribes@gmail.com>
//! URL:    https://www.shadertoy.com/view/wfG3Dz
//! Date:   24-May-2025
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/noxfire.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "noxfire",
        label: "Nox Fire",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Matt Vianueva",
            year: "2025",
            video: Some("https://www.youtube.com/watch?v=AkPIg2UilMY"),
            blurb: "I fell in to a burning ring of fire; I went down, down, down and the flames went higher.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
