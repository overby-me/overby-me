//! `elementalring`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/elementalring.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Elemental Ring
//! Author: otaviogood
//! URL:    https://www.shadertoy.com/view/MsVXDt
//! Date:   19-Jul-2016
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/elementalring.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "elementalring",
        label: "Elemental Ring",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2016",
            video: Some("https://www.youtube.com/watch?v=gmgIuvYGf0Y"),
            blurb: "A rotating ring knots and un-knots itself.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
