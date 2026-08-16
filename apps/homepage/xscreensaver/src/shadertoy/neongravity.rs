//! `neongravity`, the one Shadertoy saver here with more than one pass.
//!
//! Upstream names its files `neongravity-0.glsl` and `neongravity-1.glsl`, and
//! the digit after the dash is a *pass*: the first renders into a texture as
//! `BufferA`, the second reads it back through `iChannel0` and produces the
//! picture. `BufferA` also reads its own previous output, which is the frame
//! blur, and the reason each pass in [`super`] has two textures rather than
//! one.
//!
//! ```text
//! Title:  Abstract gravitational well II
//! Author: mrange
//! URL:    https://www.shadertoy.com/view/43G3Wc
//! Date:   13-Jun-2024
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[
        include_str!("../../glsl/neongravity-0.glsl"),
        include_str!("../../glsl/neongravity-1.glsl"),
    ],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "neongravity",
        label: "Neon Gravity",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "mrange",
            year: "2023",
            video: Some("https://www.youtube.com/watch?v=YvK5pBph9UU"),
            blurb: "The stylish interior of a torus.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
