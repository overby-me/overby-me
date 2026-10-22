//! `hexplasma`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/hexplasma.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Hexagon Plasma
//! Author: Nemerix
//! URL:    https://www.shadertoy.com/view/3fy3z3
//! Date:   26-May-2025
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/hexplasma.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "hexplasma",
        label: "Hex Plasma",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Nemerix",
            year: "2025",
            video: Some("https://www.youtube.com/watch?v=KAYPUB3OXWM"),
            blurb: "A hexagon in a plasma field.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
