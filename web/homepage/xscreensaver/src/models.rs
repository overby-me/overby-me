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

/// The nine parts of a flying toaster, and the two slices of toast.
pub const TOAST: &str = include_str!("../models/toast.gllist");
pub const TOAST2: &str = include_str!("../models/toast2.gllist");
pub const TOASTER: &str = include_str!("../models/toaster.gllist");
pub const TOASTER_BASE: &str = include_str!("../models/toaster_base.gllist");
pub const TOASTER_HANDLE: &str = include_str!("../models/toaster_handle.gllist");
pub const TOASTER_HANDLE2: &str = include_str!("../models/toaster_handle2.gllist");
pub const TOASTER_JET: &str = include_str!("../models/toaster_jet.gllist");
pub const TOASTER_KNOB: &str = include_str!("../models/toaster_knob.gllist");
pub const TOASTER_SLOTS: &str = include_str!("../models/toaster_slots.gllist");
pub const TOASTER_WING: &str = include_str!("../models/toaster_wing.gllist");

/// The five parts of a security camera in `vigilance`. Four of them are half
/// the camera, drawn again mirrored.
pub const SECCAM_BODY: &str = include_str!("../models/seccam_body.gllist");
pub const SECCAM_CAP: &str = include_str!("../models/seccam_cap.gllist");
pub const SECCAM_HINGE: &str = include_str!("../models/seccam_hinge.gllist");
pub const SECCAM_LENS: &str = include_str!("../models/seccam_lens.gllist");
pub const SECCAM_PIPE: &str = include_str!("../models/seccam_pipe.gllist");

/// The six parts of the cow in `bouncingcow`. It is by some way the biggest
/// model here; the hide alone is thirteen thousand vertices.
pub const COW_FACE: &str = include_str!("../models/cow_face.gllist");
pub const COW_HIDE: &str = include_str!("../models/cow_hide.gllist");
pub const COW_HOOFS: &str = include_str!("../models/cow_hoofs.gllist");
pub const COW_HORNS: &str = include_str!("../models/cow_horns.gllist");
pub const COW_TAIL: &str = include_str!("../models/cow_tail.gllist");
pub const COW_UDDER: &str = include_str!("../models/cow_udder.gllist");

/// The golden apple of `kallisti`, twenty-eight thousand vertices of it.
pub const KALLISTI_MODEL: &str = include_str!("../models/kallisti_model.gllist");

/// The four parts of the skull in `skulloop`, each half of one.
pub const SKULL_MODEL_JAW_HALF: &str = include_str!("../models/skull_model_jaw_half.gllist");
pub const SKULL_MODEL_SKULL_HALF: &str = include_str!("../models/skull_model_skull_half.gllist");
pub const SKULL_MODEL_TEETH_LOWER_HALF: &str =
    include_str!("../models/skull_model_teeth_lower_half.gllist");
pub const SKULL_MODEL_TEETH_UPPER_HALF: &str =
    include_str!("../models/skull_model_teeth_upper_half.gllist");

/// The suit `headroom` wears. Its head is the same model `skulloop` uses.
pub const HEADROOM_MODEL_MASK_HALF: &str =
    include_str!("../models/headroom_model_mask_half.gllist");
pub const HEADROOM_MODEL_SHIRT_HALF: &str =
    include_str!("../models/headroom_model_shirt_half.gllist");
pub const HEADROOM_MODEL_SUIT_CAP_HALF: &str =
    include_str!("../models/headroom_model_suit_cap_half.gllist");
pub const HEADROOM_MODEL_SUIT_HALF: &str =
    include_str!("../models/headroom_model_suit_half.gllist");
pub const HEADROOM_MODEL_TIE_HALF: &str = include_str!("../models/headroom_model_tie_half.gllist");
