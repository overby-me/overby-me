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
