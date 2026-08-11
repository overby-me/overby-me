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
