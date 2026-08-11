//! `rigrekt`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/rigrekt.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Rig Rekt
//! Author: Matt Vianueva <diatribes@gmail.com>
//! URL:    https://www.shadertoy.com/view/3XKfDV
//! Date:   26-Feb-2026
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/rigrekt.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "rigrekt",
        label: "Rig Rekt",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Matt Vianueva",
            year: "2025",
            video: Some("https://www.youtube.com/watch?v=jEgJQmvps3E"),
            blurb: "Exploring a flooded mega-structure.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
