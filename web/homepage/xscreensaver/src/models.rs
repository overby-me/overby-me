//! The shapes the hacks are made of.
//!
//! Some savers are a program wrapped around a model someone drew: a toaster, a
//! skull, a golden apple. Upstream converts each to C source at build time, a
//! flat array of interleaved floats plus a header saying how to read it, and
//! draws it through `gllist.c`.
//!
//! Here the same arrays are assets rather than source, because a Rust file with
//! tens of thousands of float literals in it takes minutes to compile. They are
//! upstream's numbers character for character, converted by
//! `web/homepage/gen-gllist.nu` and read back by [`crate::runtime::gllist`].

/// The four shapes the Bit takes: two idling polyhedra, the spiky red no, and
/// the yellow tetrahedral yes.
pub const TRONBIT_IDLE1: &str = include_str!("../models/tronbit_idle1.gllist");
pub const TRONBIT_IDLE2: &str = include_str!("../models/tronbit_idle2.gllist");
pub const TRONBIT_NO: &str = include_str!("../models/tronbit_no.gllist");
pub const TRONBIT_YES: &str = include_str!("../models/tronbit_yes.gllist");

/// Every bundled model, so a test can check that all of them still parse.
pub const ALL: &[&str] = &[TRONBIT_IDLE1, TRONBIT_IDLE2, TRONBIT_NO, TRONBIT_YES];

/// The eight dazzle-camouflaged ships of `razzledazzle`, from upstream's
/// `ships.c`. Only ever one on screen at a time.
pub const SHIPS_SHIP1: &str = include_str!("../models/ships_ship1.gllist");
pub const SHIPS_SHIP2: &str = include_str!("../models/ships_ship2.gllist");
pub const SHIPS_SHIP3: &str = include_str!("../models/ships_ship3.gllist");
pub const SHIPS_SHIP4: &str = include_str!("../models/ships_ship4.gllist");
pub const SHIPS_SHIP5: &str = include_str!("../models/ships_ship5.gllist");
pub const SHIPS_SHIP6: &str = include_str!("../models/ships_ship6.gllist");
pub const SHIPS_SHIP7: &str = include_str!("../models/ships_ship7.gllist");
pub const SHIPS_SHIP8: &str = include_str!("../models/ships_ship8.gllist");

/// The seven parts of `dumpsterfire`'s dumpster. Four of them are half the
/// box, drawn again mirrored.
pub const DUMPSTER_MODEL_AXLE: &str = include_str!("../models/dumpster_model_axle.gllist");
pub const DUMPSTER_MODEL_FRAME_HALF: &str =
    include_str!("../models/dumpster_model_frame_half.gllist");
pub const DUMPSTER_MODEL_HINGES_HALF: &str =
    include_str!("../models/dumpster_model_hinges_half.gllist");
pub const DUMPSTER_MODEL_INSIDE_HALF: &str =
    include_str!("../models/dumpster_model_inside_half.gllist");
pub const DUMPSTER_MODEL_LID: &str = include_str!("../models/dumpster_model_lid.gllist");
pub const DUMPSTER_MODEL_LID_PANELS: &str =
    include_str!("../models/dumpster_model_lid_panels.gllist");
pub const DUMPSTER_MODEL_PANELS_HALF: &str =
    include_str!("../models/dumpster_model_panels_half.gllist");
