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

/// `tunnel0.png` to `tunnel5.png`: the six walls `atunnel` flies through,
/// 64 or 128 square, greyscale or palettised.
/// `atlantis`'s water: a small tile of noise laid over the whole tank.
pub const SEA_TEXTURE: &[u8] = include_bytes!("../images/sea-texture.png");
pub const TUNNEL0: &[u8] = include_bytes!("../images/tunnel0.png");
pub const TUNNEL1: &[u8] = include_bytes!("../images/tunnel1.png");
pub const TUNNEL2: &[u8] = include_bytes!("../images/tunnel2.png");
pub const TUNNEL3: &[u8] = include_bytes!("../images/tunnel3.png");
pub const TUNNEL4: &[u8] = include_bytes!("../images/tunnel4.png");
pub const TUNNEL5: &[u8] = include_bytes!("../images/tunnel5.png");

/// `wood.png`: 128x32 of woodgrain, palettised. The planks of `cage` are
/// each one of these stretched over a face.
pub const WOOD: &[u8] = include_bytes!("../images/wood.png");

/// `blocktube.png`: a 256x256 greyscale photograph of a lit room, used as a
/// sphere map so that the slabs look like polished metal.
pub const BLOCKTUBE: &[u8] = include_bytes!("../images/blocktube.png");

/// `ground.png`: the forest floor of `glforestfire`, tiled sixteen times
/// across a forty-unit square of ground.
pub const GROUND: &[u8] = include_bytes!("../images/ground.png");

/// `tree.png`: one tree on a transparent background. `glforestfire` stands two
/// copies of it back to back in a cross, so it reads from any side.
pub const TREE: &[u8] = include_bytes!("../images/tree.png");

/// The six pictures `maze3d` is built out of: brick for the walls, small
/// tiles for the ceiling, boards for the floor, and the START and FINISH
/// signs. The rat it sometimes passes is `bob.png`.
pub const BRICK1: &[u8] = include_bytes!("../images/brick1.png");
pub const BRICK2: &[u8] = include_bytes!("../images/brick2.png");
pub const WOOD2: &[u8] = include_bytes!("../images/wood2.png");
pub const START: &[u8] = include_bytes!("../images/start.png");
pub const LOGO_32: &[u8] = include_bytes!("../images/logo-32.png");

/// `bob.png`: the face flag waves when it is not waving words. From xlockmore
/// by way of XScreenSaver, 64x64, four bits of palette.
pub const BOB: &[u8] = include_bytes!("../images/bob.png");

/// `6x10font.png`: 256 glyphs of 7x10 in one row, black on transparent. What
/// `analogtv_make_font` draws straight into a video signal.
pub const FONT_6X10: &[u8] = include_bytes!("../images/6x10font.png");

/// The machines `bsod` imitates, in the pictures each of them showed while it
/// was failing: the Guru Meditation, the Atari bomb, the sad Mac and its
/// successors, the Sun and Apple logos, the ATM, the DVD player, the Android,
/// the ransom note, and the two GNOME crash dialogs.
pub mod bsod {
    pub const AMIGA: &[u8] = include_bytes!("../images/bsod/amiga.png");
    pub const ANDROID: &[u8] = include_bytes!("../images/bsod/android.png");
    pub const APPLE: &[u8] = include_bytes!("../images/bsod/apple.png");
    pub const ATARI: &[u8] = include_bytes!("../images/bsod/atari.png");
    pub const ATM: &[u8] = include_bytes!("../images/bsod/atm.png");
    pub const DVD: &[u8] = include_bytes!("../images/bsod/dvd.png");
    pub const GNOME1: &[u8] = include_bytes!("../images/bsod/gnome1.png");
    pub const GNOME2: &[u8] = include_bytes!("../images/bsod/gnome2.png");
    pub const HMAC: &[u8] = include_bytes!("../images/bsod/hmac.png");
    pub const MAC: &[u8] = include_bytes!("../images/bsod/mac.png");
    pub const MACBOMB: &[u8] = include_bytes!("../images/bsod/macbomb.png");
    pub const OSX_10_2: &[u8] = include_bytes!("../images/bsod/osx_10_2.png");
    pub const OSX_10_3: &[u8] = include_bytes!("../images/bsod/osx_10_3.png");
    pub const RANSOMWARE: &[u8] = include_bytes!("../images/bsod/ransomware.png");
    pub const SUN: &[u8] = include_bytes!("../images/bsod/sun.png");
}

/// `apple2font.png`: the Apple ][ character generator, 64 glyphs of 7x8 in one
/// row. jwz dumped it out of X's 6x10 with the machine's own tweaks already
/// applied, a slash through the zero among them, because MacOS has no "6x10".
pub const APPLE2FONT: &[u8] = include_bytes!("../images/apple2font.png");

/// The XScreenSaver logo, with an alpha channel, at the three sizes upstream
/// ships. A hack picks one by how big the screen is.
pub const LOGO_50: &[u8] = include_bytes!("../images/logo-50.png");
pub const LOGO_180: &[u8] = include_bytes!("../images/logo-180.png");
pub const LOGO_360: &[u8] = include_bytes!("../images/logo-360.png");

/// `pacman.png`: sixty 64x64 cells in one column. Four ghosts in four
/// directions with two leg positions, then the scared ghost and its flash, the
/// eyes on their way home, Pac-Man himself, and the eight frames of his death.
pub const PACMAN: &[u8] = include_bytes!("../images/pacman.png");

/// The bubbles, eleven sizes of each of four liquids, ray-traced by James
/// Macnicol in 1996 (the POV-Ray scenes are still beside them upstream).
pub const BUBBLES: [[&[u8]; 11]; 4] = [
    [
        include_bytes!("../images/bubbles/blood1.png"),
        include_bytes!("../images/bubbles/blood2.png"),
        include_bytes!("../images/bubbles/blood3.png"),
        include_bytes!("../images/bubbles/blood4.png"),
        include_bytes!("../images/bubbles/blood5.png"),
        include_bytes!("../images/bubbles/blood6.png"),
        include_bytes!("../images/bubbles/blood7.png"),
        include_bytes!("../images/bubbles/blood8.png"),
        include_bytes!("../images/bubbles/blood9.png"),
        include_bytes!("../images/bubbles/blood10.png"),
        include_bytes!("../images/bubbles/blood11.png"),
    ],
    [
        include_bytes!("../images/bubbles/blue1.png"),
        include_bytes!("../images/bubbles/blue2.png"),
        include_bytes!("../images/bubbles/blue3.png"),
        include_bytes!("../images/bubbles/blue4.png"),
        include_bytes!("../images/bubbles/blue5.png"),
        include_bytes!("../images/bubbles/blue6.png"),
        include_bytes!("../images/bubbles/blue7.png"),
        include_bytes!("../images/bubbles/blue8.png"),
        include_bytes!("../images/bubbles/blue9.png"),
        include_bytes!("../images/bubbles/blue10.png"),
        include_bytes!("../images/bubbles/blue11.png"),
    ],
    [
        include_bytes!("../images/bubbles/glass1.png"),
        include_bytes!("../images/bubbles/glass2.png"),
        include_bytes!("../images/bubbles/glass3.png"),
        include_bytes!("../images/bubbles/glass4.png"),
        include_bytes!("../images/bubbles/glass5.png"),
        include_bytes!("../images/bubbles/glass6.png"),
        include_bytes!("../images/bubbles/glass7.png"),
        include_bytes!("../images/bubbles/glass8.png"),
        include_bytes!("../images/bubbles/glass9.png"),
        include_bytes!("../images/bubbles/glass10.png"),
        include_bytes!("../images/bubbles/glass11.png"),
    ],
    [
        include_bytes!("../images/bubbles/jade1.png"),
        include_bytes!("../images/bubbles/jade2.png"),
        include_bytes!("../images/bubbles/jade3.png"),
        include_bytes!("../images/bubbles/jade4.png"),
        include_bytes!("../images/bubbles/jade5.png"),
        include_bytes!("../images/bubbles/jade6.png"),
        include_bytes!("../images/bubbles/jade7.png"),
        include_bytes!("../images/bubbles/jade8.png"),
        include_bytes!("../images/bubbles/jade9.png"),
        include_bytes!("../images/bubbles/jade10.png"),
        include_bytes!("../images/bubbles/jade11.png"),
    ],
];

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

/// `curlicue.h`: the curling arrow the four topology savers draw over their
/// surface to show its orientation. 64 square, one byte a pixel, converted
/// from upstream's header by `web/homepage/gen-curlicue.nu`.
pub const CURLICUE: &[u8] = include_bytes!("../images/curlicue.gray");

/// The two pictures `sballs` maps onto its balls and its backdrop.
pub const SBALL: &[u8] = include_bytes!("../images/sball.png");
pub const SBALL_BG: &[u8] = include_bytes!("../images/sball-bg.png");

/// The mirrored ball `flyingtoasters` wraps onto its chrome, and the picture
/// of toast it lays on its toast.
pub const CHROMESPHERE: &[u8] = include_bytes!("../images/chromesphere.png");

/// `jigglymap.png`: the sky `jigglypuff` reflects in its chrome, which is not
/// there.
pub const JIGGLYMAP: &[u8] = include_bytes!("../images/jigglymap.png");

/// `boxed.h`: the picture on the box `boxed`'s balls fall into, kept as
/// upstream's GIMP header rather than converted, and unpacked by the saver.
pub const BOXED_TEXTURE: &str = include_str!("../images/boxed.h");

/// The two photographs `peepers` wraps its eyeballs in: the white of the eye
/// with its veins, and the iris.
pub const SCLERA: &[u8] = include_bytes!("../images/sclera.png");
pub const IRIS: &[u8] = include_bytes!("../images/iris.png");
/// The glyph sheet `glmatrix` rains down: sixteen by thirteen characters.
pub const MATRIX3: &[u8] = include_bytes!("../images/matrix3.png");
/// The skin of the thing in `skytentacles`.
pub const SCALES: &[u8] = include_bytes!("../images/scales.png");
pub const TOAST_PNG: &[u8] = include_bytes!("../images/toast.png");
