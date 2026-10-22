//! `alienbeacon`, one of the Shadertoy savers.
//!
//! The program itself is `glsl/alienbeacon.glsl`, vendored from upstream with
//! its own licence notice intact. Its header says:
//!
//! ```text
//! Title:  Alien Beacon
//! Author: otaviogood
//! URL:    https://www.shadertoy.com/view/ld2SzK
//! Date:   26-Oct-2014
//! ```
//!
//! [`super`] is the runner all thirty of them share.

use super::{DEFAULTS, OPTS, Shadertoy, ShadertoyDef, Variant};
use crate::runtime::{About, SaverDef, StartArgs};

static VARIANTS: [Variant; 1] = [Variant {
    common: "",
    passes: &[include_str!("../../glsl/alienbeacon.glsl")],
}];

pub static DEF: ShadertoyDef = ShadertoyDef {
    def: SaverDef {
        slug: "alienbeacon",
        label: "Alien Beacon",
        defaults: DEFAULTS,
        opts: OPTS,
        about: About {
            author: "Otavio Good",
            year: "2014",
            video: Some("https://www.youtube.com/watch?v=na3do17whcw"),
            blurb: "Our investigation of the signal from the planet's surface brought us to what seems to be an alien beacon.",
        },
    },
    variants: &VARIANTS,
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Shadertoy {
    Shadertoy::start(&DEF, args)
}
