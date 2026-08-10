//! The pictures the hacks are made of.
//!
//! Not the photographs a saver works on, which come from outside and are
//! [`crate::runtime::image`]'s business. These are parts of the programs: a
//! sprite sheet, a glyph table, a test card. Upstream keeps them in
//! `hacks/images/` and turns each into a C array at build time
//! (`images/gen/NAME_png.h`); here the same files are in `xscreensaver/images/`
//! and arrive through `include_bytes!`, decoded by [`crate::runtime::png`].
//!
//! They are upstream's files byte for byte, under the same notice as the code
//! that draws them.

/// `bob.png`: the face flag waves when it is not waving words. From xlockmore
/// by way of XScreenSaver, 64x64, four bits of palette.
pub const BOB: &[u8] = include_bytes!("../images/bob.png");

/// `6x10font.png`: 256 glyphs of 7x10 in one row, black on transparent. What
/// `analogtv_make_font` draws straight into a video signal.
pub const FONT_6X10: &[u8] = include_bytes!("../images/6x10font.png");

/// The XScreenSaver logo, with an alpha channel, at the three sizes upstream
/// ships. A hack picks one by how big the screen is.
pub const LOGO_50: &[u8] = include_bytes!("../images/logo-50.png");
pub const LOGO_180: &[u8] = include_bytes!("../images/logo-180.png");
pub const LOGO_360: &[u8] = include_bytes!("../images/logo-360.png");

/// The Matrix glyph sheets: 16 by 13 characters each, plain and glowing, at
/// the two sizes upstream ships. Mirror-image katakana, as the film had them.
pub const MATRIX_PLAIN: &[u8] = include_bytes!("../images/matrix1.png");
pub const MATRIX_GLOW: &[u8] = include_bytes!("../images/matrix2.png");
pub const MATRIX_PLAIN_SMALL: &[u8] = include_bytes!("../images/matrix1b.png");
pub const MATRIX_GLOW_SMALL: &[u8] = include_bytes!("../images/matrix2b.png");

/// The three test cards `xanalogtv` tunes between: the RCA Indian-head card,
/// the Philips PM5544, and the BBC's Test Card F.
pub const TESTCARDS: [&[u8]; 3] = [
    include_bytes!("../images/testcard_rca.png"),
    include_bytes!("../images/testcard_pm5544.png"),
    include_bytes!("../images/testcard_bbcf.png"),
];

/// The noseguy's eight poses, 64x64 each: two frames walking left, two walking
/// right, two three-quarter views, one facing forward and one looking down.
/// Drawn by Dan Heller for xnlock, which is where the whole saver comes from.
pub const NOSE: [&[u8]; 8] = [
    include_bytes!("../images/noseguy/nose-l1.png"),
    include_bytes!("../images/noseguy/nose-l2.png"),
    include_bytes!("../images/noseguy/nose-r1.png"),
    include_bytes!("../images/noseguy/nose-r2.png"),
    include_bytes!("../images/noseguy/nose-f2.png"),
    include_bytes!("../images/noseguy/nose-f3.png"),
    include_bytes!("../images/noseguy/nose-f1.png"),
    include_bytes!("../images/noseguy/nose-f4.png"),
];
