//! `neontriangulator`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/neontriangulator.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Neon Triangulator
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/tXGGRD
//! Date:   09-Jul-2025
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/neontriangulator.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "neontriangulator",
        label: "Neon Triangulator",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2021",
            video: Some("https://www.youtube.com/watch?v=YJ1eYlQb5OA"),
            blurb: "Neon triangles zoom.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
