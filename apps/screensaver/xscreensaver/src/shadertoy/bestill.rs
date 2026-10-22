//! `bestill`, the one Shadertoy saver that is six programs rather than one.
//!
//! Upstream names its files `bestill0-0.glsl` through `bestill5-0.glsl`, and
//! the digit before the dash is a *variant*: a whole alternative program, not
//! another pass. The saver runs one of them, and every `duration` seconds it
//! moves to the next and starts its clock again. That is why this is the only
//! saver in the tier whose XML has a duration slider.
//!
//! The six, in order, with their own licence notices intact in `glsl/`:
//!
//! ```text
//! Be Still               https://www.shadertoy.com/view/tfXcRn
//! Everything is Temporary https://www.shadertoy.com/view/w32BDD
//! Night Cloud Dance      https://www.shadertoy.com/view/3cjcWD
//! Cloud LIghts           https://www.shadertoy.com/view/wXXBRX
//! Desert Duo             https://www.shadertoy.com/view/3cXyzB
//! Water [237]            https://www.shadertoy.com/view/tXjXDy
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, Opt, SaverDef, StartArgs};

/// Six programs, each with no `Common` source and one pass.
static VARIANTS: [Variant; 6] = [
    Variant {
        common: "",
        passes: &[include_str!("../../glsl/bestill0-0.glsl")],
    },
    Variant {
        common: "",
        passes: &[include_str!("../../glsl/bestill1-0.glsl")],
    },
    Variant {
        common: "",
        passes: &[include_str!("../../glsl/bestill2-0.glsl")],
    },
    Variant {
        common: "",
        passes: &[include_str!("../../glsl/bestill3-0.glsl")],
    },
    Variant {
        common: "",
        passes: &[include_str!("../../glsl/bestill4-0.glsl")],
    },
    Variant {
        common: "",
        passes: &[include_str!("../../glsl/bestill5-0.glsl")],
    },
];

/// The tier's knobs plus the one only this saver has.
const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 10.0, 0.01, 2, "1.0"),
    Opt::slider("duration", "Change every", 10.0, 600.0, 5.0, 0, "120"),
    Opt::slider("scale", "Resolution", 0.1, 1.0, 0.05, 2, "1.0"),
];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "bestill",
        label: "Be Still",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Matt Vianueva",
            year: "2025",
            video: Some("https://www.youtube.com/watch?v=-z8OBZ2DBU8"),
            blurb: "Various scenes of lights playing above the clouds.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
