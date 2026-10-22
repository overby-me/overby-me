//! The `xlockmore` façade: `ModeInfo` and the `MI_*` accessors.
//!
//! Thirty-eight of the 141 2D hacks came to xscreensaver from xlockmore and are
//! written against a different driver: instead of `screenhack.h`'s
//! `init`/`draw` taking a `Display` and a `Window`, they take a `ModeInfo` and
//! read everything through macros. `hacks/xlockmore.c` upstream is the adaptor
//! that makes them run under xscreensaver; this is the same adaptor.
//!
//! What a 2D xlockmore hack actually reaches for is small: the window size, a
//! GC, a colormap indexed by `MI_PIXEL`, and the four generic knobs
//! (`count`/`cycles`/`size`/`delay`). Porting one is then the same mechanical
//! job as porting a `screenhack.h` one, with `mi.` in front of the accessors.

use super::color::{BLACK, Pixel, WHITE, XColor};
use super::opts::Resources;
use super::{Dpy, Gc, color, random, random_below};

/// The largest colormap xlockmore will build (`MAX_COLORS`).
const MAX_COLORS: i32 = 255;

/// Which colormap a hack wants, from the `*_COLORS` it `#define`s before
/// including `xlockmore.h`. No define means random, non-bright.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ColorScheme {
    /// `#define BRIGHT_COLORS`
    Bright,
    /// `#define SMOOTH_COLORS`
    Smooth,
    /// `#define UNIFORM_COLORS`
    Uniform,
    #[default]
    Random,
}

/// `ModeInfo`: everything the `MI_*` macros read.
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    /// `MI_GC`. Owned by the hack, like every other GC in this port.
    pub gc: Gc,
    /// `MI_COUNT` / `MI_BATCHCOUNT`.
    pub count: i32,
    /// `MI_CYCLES`.
    pub cycles: i32,
    /// `MI_SIZE`.
    pub size: i32,
    /// `MI_DELAY` / `mi->pause`, in microseconds.
    pub delay: u32,
    pub white: Pixel,
    pub black: Pixel,
    colors: Vec<XColor>,
}

impl ModeInfo {
    /// Set a mode up the way `hacks/xlockmore.c` does: read the four generic
    /// resources, then build the colormap the hack asked for.
    pub fn new(d: &Dpy, scheme: ColorScheme) -> Self {
        let mut npixels = d.res.int("ncolors");
        if npixels <= 0 {
            npixels = 64;
        } else if npixels > MAX_COLORS {
            npixels = MAX_COLORS;
        }

        let colors = if d.mono_p || npixels <= 2 {
            Vec::new()
        } else {
            let n = npixels as usize;
            match scheme {
                ColorScheme::Uniform => color::make_uniform_colormap(n),
                ColorScheme::Smooth => color::make_smooth_colormap(n),
                ColorScheme::Bright => color::make_random_colormap(n, true),
                ColorScheme::Random => color::make_random_colormap(n, false),
            }
        };

        let white = white_of(&d.res);
        let black = black_of(&d.res);
        Self {
            width: d.width(),
            height: d.height(),
            gc: Gc::new(white, black),
            count: d.res.int("count"),
            cycles: d.res.int("cycles"),
            size: d.res.int("size"),
            delay: d.res.int("delay").max(0) as u32,
            white,
            black,
            colors,
        }
    }

    /// `MI_NPIXELS`. Two or fewer means the hack should draw in mono.
    pub fn npixels(&self) -> i32 {
        self.colors.len() as i32
    }

    /// `MI_PIXEL`. Wraps rather than panicking: several hacks compute an index
    /// modulo a count they adjusted after the map was built.
    pub fn pixel(&self, i: usize) -> Pixel {
        if self.colors.is_empty() {
            return self.white;
        }
        self.colors[i % self.colors.len()].pixel
    }

    /// `MI_IS_MONO`.
    pub fn is_mono(&self) -> bool {
        self.colors.len() <= 2
    }

    /// Throw the colormap away and ask for a fresh one. A hack that draws
    /// discrete scenes (triangle) does this between them, so each landscape
    /// gets its own palette.
    pub fn remake_colors(&mut self, scheme: ColorScheme, ncolors: usize) {
        self.colors = match scheme {
            ColorScheme::Uniform => color::make_uniform_colormap(ncolors),
            ColorScheme::Smooth => color::make_smooth_colormap(ncolors),
            ColorScheme::Bright => color::make_random_colormap(ncolors, true),
            ColorScheme::Random => color::make_random_colormap(ncolors, false),
        };
    }

    /// `MI_CLEARWINDOW`.
    pub fn clear_window(&self, d: &mut Dpy) {
        d.clear_window();
    }

    /// Track a resize. Hacks read `MI_WIDTH`/`MI_HEIGHT` constantly and most
    /// restart themselves when it changes.
    pub fn reshape(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}

fn white_of(res: &Resources) -> Pixel {
    match res.get("foreground") {
        Some(_) => res.pixel("foreground"),
        None => WHITE,
    }
}

fn black_of(res: &Resources) -> Pixel {
    match res.get("background") {
        Some(_) => res.pixel("background"),
        None => BLACK,
    }
}

/// `NRAND(n)`: a random value in `0..n`.
pub fn nrand(n: i32) -> i32 {
    random_below(n)
}

/// `MAXRAND`: `1 << 31` as a float. The ceiling [`lrand`] runs up to, and
/// the divisor every xlockmore hack uses to turn one into a fraction.
pub const MAXRAND: f64 = 2147483648.0;

/// `LRAND()`: the raw generator, as xlockmore spells it.
///
/// Masked to 31 bits, which is what upstream's macro does. It matters: hacks
/// divide the result by [`MAXRAND`] to get a fraction, compare it against
/// `MAXRAND / 2` for a coin flip, and shift it down for a cached half.
pub fn lrand() -> u32 {
    random() & 0x7fff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::rand::ya_rand_init;

    const DEFAULTS: &[&str] = &[
        ".background: black",
        ".foreground: white",
        "*delay: 200000",
        "*count: 0",
        "*cycles: 100",
        "*size: 0",
        "*ncolors: 64",
    ];

    fn dpy() -> Dpy {
        Dpy::new(320, 240, Resources::new(DEFAULTS, &[], ""))
    }

    #[test]
    fn reads_the_generic_knobs() {
        ya_rand_init(1);
        let mi = ModeInfo::new(&dpy(), ColorScheme::Random);
        assert_eq!(mi.delay, 200_000);
        assert_eq!(mi.count, 0);
        assert_eq!(mi.cycles, 100);
        assert_eq!(mi.width, 320);
        assert_eq!(mi.height, 240);
        assert_eq!(mi.npixels(), 64);
        assert!(!mi.is_mono());
    }

    #[test]
    fn every_scheme_builds_a_usable_map() {
        ya_rand_init(2);
        for scheme in [
            ColorScheme::Random,
            ColorScheme::Bright,
            ColorScheme::Smooth,
            ColorScheme::Uniform,
        ] {
            let mi = ModeInfo::new(&dpy(), scheme);
            assert_eq!(mi.npixels(), 64, "{scheme:?}");
            let distinct: std::collections::HashSet<Pixel> = (0..64).map(|i| mi.pixel(i)).collect();
            assert!(
                distinct.len() > 8,
                "{scheme:?} gave {} colours",
                distinct.len()
            );
        }
    }

    #[test]
    fn ncolors_is_clamped_and_pixel_wraps() {
        ya_rand_init(3);
        for (ncolors, want) in [("0", 64), ("1", 0), ("2", 0), ("500", MAX_COLORS)] {
            let d = Dpy::new(
                64,
                64,
                Resources::new(DEFAULTS, &[], &format!("?ncolors={ncolors}")),
            );
            // The query only reaches declared options, so set it directly.
            let mut res = Resources::new(DEFAULTS, &[], "");
            res.set("ncolors", ncolors);
            let d = Dpy::new(d.width(), d.height(), res);
            let mi = ModeInfo::new(&d, ColorScheme::Random);
            assert_eq!(mi.npixels(), want, "ncolors={ncolors}");
            // Indexing past the end wraps rather than panicking.
            assert_ne!(mi.pixel(9999), 0);
        }
    }

    #[test]
    fn a_mono_mode_still_has_a_colour_to_draw_with() {
        ya_rand_init(4);
        let mut res = Resources::new(DEFAULTS, &[], "");
        res.set("ncolors", "1");
        let mi = ModeInfo::new(&Dpy::new(64, 64, res), ColorScheme::Random);
        assert!(mi.is_mono());
        assert_eq!(mi.pixel(0), mi.white);
    }
}
