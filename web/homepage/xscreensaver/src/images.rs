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

/// `lament512.png`: eight 512-square tiles stacked into one tall picture. The
/// six walls of Lemarchand's Box in gold leaf, then the inside of it, then the
/// Leviathan.
pub const LAMENT512: &[u8] = include_bytes!("../images/lament512.png");

/// The Earth in daylight and at night, as satellite photographs on an
/// equirectangular projection, plus the flat political map and the mask that
/// says which of it is water. Upstream ships all four at 4096x2048; these are
/// a quarter of that in each direction, which is what upstream's own
/// `dymaxionmap` shrinks them to before it uses them.
pub const EARTH: &[u8] = include_bytes!("../images/earth.png");
pub const EARTH_NIGHT: &[u8] = include_bytes!("../images/earth_night.png");
pub const EARTH_FLAT: &[u8] = include_bytes!("../images/earth_flat.png");
pub const EARTH_WATER: &[u8] = include_bytes!("../images/earth_water.png");

/// The fifty-two cards `klondike` deals, plus two backs. Upstream renders
/// these from SVG at 360x540 and pads them with a drop shadow, which comes to
/// three megabytes; here they are half that size with a thirty-two colour
/// palette, which is 392 KB and looks the same at the size a card is drawn.
/// The originals and their licences are named in `images/klondike/
/// attribution.txt`, which is upstream's file.
pub const KLONDIKE_CARDS: &[(&str, &[u8])] = &[
    ("C2", include_bytes!("../images/klondike/C2.png")),
    ("C3", include_bytes!("../images/klondike/C3.png")),
    ("C4", include_bytes!("../images/klondike/C4.png")),
    ("C5", include_bytes!("../images/klondike/C5.png")),
    ("C6", include_bytes!("../images/klondike/C6.png")),
    ("C7", include_bytes!("../images/klondike/C7.png")),
    ("C8", include_bytes!("../images/klondike/C8.png")),
    ("C9", include_bytes!("../images/klondike/C9.png")),
    ("CA", include_bytes!("../images/klondike/CA.png")),
    ("CJ", include_bytes!("../images/klondike/CJ.png")),
    ("CK", include_bytes!("../images/klondike/CK.png")),
    ("CQ", include_bytes!("../images/klondike/CQ.png")),
    ("CT", include_bytes!("../images/klondike/CT.png")),
    ("D2", include_bytes!("../images/klondike/D2.png")),
    ("D3", include_bytes!("../images/klondike/D3.png")),
    ("D4", include_bytes!("../images/klondike/D4.png")),
    ("D5", include_bytes!("../images/klondike/D5.png")),
    ("D6", include_bytes!("../images/klondike/D6.png")),
    ("D7", include_bytes!("../images/klondike/D7.png")),
    ("D8", include_bytes!("../images/klondike/D8.png")),
    ("D9", include_bytes!("../images/klondike/D9.png")),
    ("DA", include_bytes!("../images/klondike/DA.png")),
    ("DJ", include_bytes!("../images/klondike/DJ.png")),
    ("DK", include_bytes!("../images/klondike/DK.png")),
    ("DQ", include_bytes!("../images/klondike/DQ.png")),
    ("DT", include_bytes!("../images/klondike/DT.png")),
    ("H2", include_bytes!("../images/klondike/H2.png")),
    ("H3", include_bytes!("../images/klondike/H3.png")),
    ("H4", include_bytes!("../images/klondike/H4.png")),
    ("H5", include_bytes!("../images/klondike/H5.png")),
    ("H6", include_bytes!("../images/klondike/H6.png")),
    ("H7", include_bytes!("../images/klondike/H7.png")),
    ("H8", include_bytes!("../images/klondike/H8.png")),
    ("H9", include_bytes!("../images/klondike/H9.png")),
    ("HA", include_bytes!("../images/klondike/HA.png")),
    ("HJ", include_bytes!("../images/klondike/HJ.png")),
    ("HK", include_bytes!("../images/klondike/HK.png")),
    ("HQ", include_bytes!("../images/klondike/HQ.png")),
    ("HT", include_bytes!("../images/klondike/HT.png")),
    ("S2", include_bytes!("../images/klondike/S2.png")),
    ("S3", include_bytes!("../images/klondike/S3.png")),
    ("S4", include_bytes!("../images/klondike/S4.png")),
    ("S5", include_bytes!("../images/klondike/S5.png")),
    ("S6", include_bytes!("../images/klondike/S6.png")),
    ("S7", include_bytes!("../images/klondike/S7.png")),
    ("S8", include_bytes!("../images/klondike/S8.png")),
    ("S9", include_bytes!("../images/klondike/S9.png")),
    ("SA", include_bytes!("../images/klondike/SA.png")),
    ("SJ", include_bytes!("../images/klondike/SJ.png")),
    ("SK", include_bytes!("../images/klondike/SK.png")),
    ("SQ", include_bytes!("../images/klondike/SQ.png")),
    ("ST", include_bytes!("../images/klondike/ST.png")),
    ("back", include_bytes!("../images/klondike/back.png")),
    ("back0", include_bytes!("../images/klondike/back0.png")),
];

/// `timetunnel`'s own textures: the three tunnel walls it scrolls, and the
/// star that bursts out of the middle of them.
pub const TIMETUNNEL0: &[u8] = include_bytes!("../images/timetunnel0.png");
pub const TIMETUNNEL1: &[u8] = include_bytes!("../images/timetunnel1.png");
pub const TIMETUNNEL2: &[u8] = include_bytes!("../images/timetunnel2.png");
pub const TUNNELSTAR: &[u8] = include_bytes!("../images/tunnelstar.png");
