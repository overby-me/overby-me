//! Ports of `utils/hsv.c` and `utils/colors.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! Most of `colors.c` exists to cope with PseudoColor visuals: allocation can
//! fail, so every builder has a retry ladder that asks for fewer colours until
//! the colormap obliges, and a writable-cell path for colour cycling. A canvas
//! is always TrueColor, so allocation never fails and none of that survives the
//! port. What is left is the part that decides what the colours actually are,
//! which is what the hacks' output depends on.
//!
//! One faithful consequence: `rotate_colors` needs writable cells, so upstream
//! it is already a no-op on TrueColor. Hacks that cycle their colormap simply
//! do not, here or on a modern X server.

use super::rand::{frand, random};

/// A pixel value.
///
/// Laid out so the bytes read R, G, B, A on a little-endian target, which is
/// exactly what `putImageData` wants: the host can hand the framebuffer
/// straight to the canvas with no per-pixel swizzle.
pub type Pixel = u32;

/// Pack 8-bit components into a [`Pixel`], fully opaque.
pub const fn rgb(r: u8, g: u8, b: u8) -> Pixel {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF00_0000
}

/// Unpack a [`Pixel`] into 8-bit components.
pub const fn unrgb(p: Pixel) -> (u8, u8, u8) {
    (p as u8, (p >> 8) as u8, (p >> 16) as u8)
}

/// The bits of a pixel a raster operation may touch. Alpha is always forced
/// opaque afterwards, so an XOR of two visible colours stays visible instead of
/// turning the result transparent.
pub const RGB_MASK: u32 = 0x00FF_FFFF;
/// Opaque alpha, OR-ed back in after every raster operation.
pub const ALPHA: u32 = 0xFF00_0000;

pub const BLACK: Pixel = rgb(0, 0, 0);
pub const WHITE: Pixel = rgb(255, 255, 255);

/// `XColor`. The 16-bit components are what the hacks compute in; `pixel` is
/// what they draw with.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XColor {
    pub pixel: Pixel,
    pub red: u16,
    pub green: u16,
    pub blue: u16,
}

impl XColor {
    pub const fn from_rgb16(red: u16, green: u16, blue: u16) -> Self {
        let mut c = Self {
            pixel: 0,
            red,
            green,
            blue,
        };
        c.pixel = rgb((red >> 8) as u8, (green >> 8) as u8, (blue >> 8) as u8);
        c
    }

    pub const fn from_pixel(pixel: Pixel) -> Self {
        let (r, g, b) = unrgb(pixel);
        Self {
            pixel,
            red: ((r as u16) << 8) | r as u16,
            green: ((g as u16) << 8) | g as u16,
            blue: ((b as u16) << 8) | b as u16,
        }
    }

    /// `XAllocColor`: on TrueColor this only has to derive the pixel value from
    /// the components, and it cannot fail.
    pub fn alloc(&mut self) {
        self.pixel = rgb(
            (self.red >> 8) as u8,
            (self.green >> 8) as u8,
            (self.blue >> 8) as u8,
        );
    }
}

/// `hsv_to_rgb`. `h` is degrees, `s` and `v` are 0.0 to 1.0.
pub fn hsv_to_rgb(h: i32, s: f64, v: f64) -> (u16, u16, u16) {
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let hh = (h.rem_euclid(360)) as f64 / 60.0;
    let i = hh as i32;
    let f = hh - i as f64;
    let p1 = v * (1.0 - s);
    let p2 = v * (1.0 - (s * f));
    let p3 = v * (1.0 - (s * (1.0 - f)));
    let (r, g, b) = match i {
        0 => (v, p3, p1),
        1 => (p2, v, p1),
        2 => (p1, v, p3),
        3 => (p1, p2, v),
        4 => (p3, p1, v),
        _ => (v, p1, p2),
    };
    (
        (r * 65535.0) as u16,
        (g * 65535.0) as u16,
        (b * 65535.0) as u16,
    )
}

/// `rgb_to_hsv`.
pub fn rgb_to_hsv(r: u16, g: u16, b: u16) -> (i32, f64, f64) {
    let rr = r as f64 / 65535.0;
    let gg = g as f64 / 65535.0;
    let bb = b as f64 / 65535.0;

    let (mut cmax, mut cmin, mut imax) = (rr, gg, 1);
    if cmax < gg {
        cmax = gg;
        cmin = rr;
        imax = 2;
    }
    if cmax < bb {
        cmax = bb;
        imax = 3;
    }
    if cmin > bb {
        cmin = bb;
    }
    let cmm = cmax - cmin;
    let v = cmax;
    if cmm == 0.0 {
        return (0, 0.0, v);
    }
    let s = cmm / cmax;
    let mut h = match imax {
        1 => (gg - bb) / cmm,
        2 => 2.0 + (bb - rr) / cmm,
        _ => 4.0 + (rr - gg) / cmm,
    };
    if h < 0.0 {
        h += 6.0;
    }
    ((h * 60.0) as i32, s, v)
}

/// `make_color_ramp`: interpolate in HSV from one point to another.
///
/// If `closed_p`, the ramp runs out and back so the map loops seamlessly.
/// Unlike the other builders this always walks from `h1` to `h2` in the
/// direction given, never the shorter way round; `make_uniform_colormap`
/// depends on that.
pub fn make_color_ramp(
    h1: i32,
    s1: f64,
    v1: f64,
    h2: i32,
    s2: f64,
    v2: f64,
    total: usize,
    closed_p: bool,
) -> Vec<XColor> {
    if total == 0 {
        return Vec::new();
    }
    let mut colors = vec![XColor::default(); total];
    let ncolors = if closed_p { (total / 2) + 1 } else { total };

    let dh = (h2 as f64 - h1 as f64) / ncolors as f64;
    let ds = (s2 - s1) / ncolors as f64;
    let dv = (v2 - v1) / ncolors as f64;

    for (i, c) in colors.iter_mut().enumerate().take(ncolors) {
        let i = i as f64;
        let (r, g, b) = hsv_to_rgb((h1 as f64 + i * dh) as i32, s1 + i * ds, v1 + i * dv);
        *c = XColor::from_rgb16(r, g, b);
    }
    if closed_p {
        for i in ncolors..total {
            colors[i] = colors[total - i];
        }
    }
    colors
}

/// `make_color_path`: walk a closed loop through `npoints` HSV points, spacing
/// the colours evenly along its circumference.
fn make_color_path(h: &[i32], s: &[f64], v: &[f64], total: usize) -> Vec<XColor> {
    let npoints = h.len();
    if npoints == 0 || total == 0 {
        return Vec::new();
    }
    if npoints == 2 {
        // A ramp is the same thing and faster.
        return make_color_ramp(h[0], s[0], v[0], h[1], s[1], v[1], total, true);
    }

    // Distance between hues the short way round the circle: 10 to 350 is 20.
    let mut dh_short = vec![0.0f64; npoints];
    for i in 0..npoints {
        let j = (i + 1) % npoints;
        let mut d = (h[i] - h[j]) as f64 / 360.0;
        if d < 0.0 {
            d = -d;
        }
        if d > 0.5 {
            d = 0.5 - (d - 0.5);
        }
        dh_short[i] = d;
    }

    let mut edge = vec![0.0f64; npoints];
    let mut circum = 0.0;
    for i in 0..npoints {
        let j = (i + 1) % npoints;
        edge[i] = ((dh_short[i] * dh_short[j])
            + (s[j] - s[i]) * (s[j] - s[i])
            + (v[j] - v[i]) * (v[j] - v[i]))
            .sqrt();
        circum += edge[i];
    }
    if circum < 0.0001 {
        return Vec::new();
    }

    // Number of pixels on an edge is proportional to that edge's length.
    let mut ncolors = vec![0usize; npoints];
    let mut dh = vec![0.0f64; npoints];
    let mut ds = vec![0.0f64; npoints];
    let mut dv = vec![0.0f64; npoints];
    for i in 0..npoints {
        let j = (i + 1) % npoints;
        ncolors[i] = (total as f64 * (edge[i] / circum)) as usize;
        if ncolors[i] > 0 {
            let n = ncolors[i] as f64;
            dh[i] = 360.0 * (dh_short[i] / n);
            ds[i] = (s[j] - s[i]) / n;
            dv[i] = (v[j] - v[i]) / n;
        }
    }

    let mut colors = vec![XColor::default(); total];
    let mut k = 0;
    for i in 0..npoints {
        let distance = h[(i + 1) % npoints] - h[i];
        let mut direction = if distance >= 0 { -1.0 } else { 1.0 };
        if (-180..=180).contains(&distance) {
            direction = -direction;
        }
        for j in 0..ncolors[i] {
            if k >= total {
                break;
            }
            let j = j as f64;
            let mut hh = h[i] as f64 + (j * dh[i] * direction);
            if hh < 0.0 {
                hh += 360.0;
            }
            let (r, g, b) = hsv_to_rgb(hh as i32, s[i] + j * ds[i], v[i] + j * dv[i]);
            colors[k] = XColor::from_rgb16(r, g, b);
            k += 1;
        }
    }

    // Rounding can leave the tail unfilled. Upstream used to shrink the map,
    // but repeated regeneration then walked the count down to zero; padding
    // with the last colour keeps the size stable.
    //
    // With very few colours asked for, rounding can wipe out every edge and
    // leave nothing at all. Upstream hands back a map of the requested size
    // full of zeroes, which is a black colormap and useless to the caller; take
    // the first point instead, so a one-colour map is that colour.
    if k == 0 {
        let (r, g, b) = hsv_to_rgb(h[0], s[0], v[0]);
        colors[0] = XColor::from_rgb16(r, g, b);
        k = 1;
    }
    let last = colors[k - 1];
    for c in colors.iter_mut().skip(k) {
        *c = last;
    }
    colors
}

/// `make_color_loop`: a closed path through three HSV points.
pub fn make_color_loop(
    h0: i32,
    s0: f64,
    v0: f64,
    h1: i32,
    s1: f64,
    v1: f64,
    h2: i32,
    s2: f64,
    v2: f64,
    total: usize,
) -> Vec<XColor> {
    make_color_path(&[h0, h1, h2], &[s0, s1, s2], &[v0, v1, v2], total)
}

/// `make_smooth_colormap`: a random closed loop of two to five HSV points,
/// rejecting picks that are too close together, too grey or too dark.
pub fn make_smooth_colormap(total: usize) -> Vec<XColor> {
    if total == 0 {
        return Vec::new();
    }
    let npoints = {
        let n = random() % 20;
        if n <= 5 {
            2 // 30% of the time
        } else if n <= 15 {
            3 // 50%
        } else if n <= 18 {
            4 // 15%
        } else {
            5 //  5%
        }
    };

    let mut h = vec![0i32; npoints];
    let mut s = vec![0.0f64; npoints];
    let mut v = vec![0.0f64; npoints];

    // Upstream aborts after 10000 tries rather than spinning; do the same, but
    // by giving up on the constraint instead of killing the tab.
    let mut loops = 0;
    'all: loop {
        let mut total_s = 0.0;
        let mut total_v = 0.0;
        for i in 0..npoints {
            loop {
                loops += 1;
                h[i] = (random() % 360) as i32;
                s[i] = frand(1.0);
                v[i] = frand(0.8) + 0.2;

                if i == 0 || loops > 10000 {
                    break;
                }
                // No two adjacent colours may be too close together.
                let j = if i + 1 == npoints { 0 } else { i - 1 };
                let hi = h[i] as f64 / 360.0;
                let hj = h[j] as f64 / 360.0;
                let mut dh = hj - hi;
                if dh < 0.0 {
                    dh = -dh;
                }
                if dh > 0.5 {
                    dh = 0.5 - (dh - 0.5);
                }
                let distance =
                    ((dh * dh) + (s[j] - s[i]) * (s[j] - s[i]) + (v[j] - v[i]) * (v[j] - v[i]))
                        .sqrt();
                if distance >= 0.2 {
                    break;
                }
            }
            total_s += s[i];
            total_v += v[i];
        }

        // Don't end up with a black-and-white or too-dark map.
        if loops > 10000 || (total_s / npoints as f64 >= 0.2 && total_v / npoints as f64 >= 0.3) {
            break 'all;
        }
    }

    make_color_path(&h, &s, &v, total)
}

/// `make_uniform_colormap`: every hue at one random saturation and value.
pub fn make_uniform_colormap(total: usize) -> Vec<XColor> {
    if total == 0 {
        return Vec::new();
    }
    let s = ((random() % 34) as f64 + 66.0) / 100.0; // 66%-100%
    let v = ((random() % 34) as f64 + 66.0) / 100.0; // 66%-100%
    make_color_ramp(0, s, v, 359, s, v, total, false)
}

/// `make_random_colormap`. `bright_p` keeps saturation and value high.
pub fn make_random_colormap(total: usize, bright_p: bool) -> Vec<XColor> {
    if total == 0 {
        return Vec::new();
    }
    let mut colors = vec![XColor::default(); total];
    loop {
        for c in colors.iter_mut() {
            if bright_p {
                let h = (random() % 360) as i32;
                let s = ((random() % 70) as f64 + 30.0) / 100.0; // 30%-100%
                let v = ((random() % 34) as f64 + 66.0) / 100.0; // 66%-100%
                let (r, g, b) = hsv_to_rgb(h, s, v);
                *c = XColor::from_rgb16(r, g, b);
            } else {
                *c = XColor::from_rgb16(
                    (random() % 0xFFFF) as u16,
                    (random() % 0xFFFF) as u16,
                    (random() % 0xFFFF) as u16,
                );
            }
        }

        // With only a few colours, make sure the first two contrast.
        if !bright_p && (2..=4).contains(&total) {
            let (_, _, v0) = rgb_to_hsv(colors[0].red, colors[0].green, colors[0].blue);
            let (_, _, v1) = rgb_to_hsv(colors[1].red, colors[1].green, colors[1].blue);
            if (v1 - v0).abs() < 0.5 {
                continue;
            }
        }
        return colors;
    }
}

/// The X11 colour names that turn up in the hacks' resource defaults, plus a
/// few neighbours. Anything else has to be `#rrggbb`.
const NAMED: &[(&str, Pixel)] = &[
    ("black", rgb(0, 0, 0)),
    ("white", rgb(255, 255, 255)),
    ("red", rgb(255, 0, 0)),
    ("green", rgb(0, 255, 0)),
    ("blue", rgb(0, 0, 255)),
    ("cyan", rgb(0, 255, 255)),
    ("magenta", rgb(255, 0, 255)),
    ("yellow", rgb(255, 255, 0)),
    ("gray", rgb(190, 190, 190)),
    ("grey", rgb(190, 190, 190)),
    ("darkgray", rgb(169, 169, 169)),
    ("darkgrey", rgb(169, 169, 169)),
    ("lightgray", rgb(211, 211, 211)),
    ("lightgrey", rgb(211, 211, 211)),
    ("orange", rgb(255, 165, 0)),
    ("purple", rgb(160, 32, 240)),
    ("pink", rgb(255, 192, 203)),
    ("brown", rgb(165, 42, 42)),
    ("gold", rgb(255, 215, 0)),
    ("navy", rgb(0, 0, 128)),
    ("cadetblue", rgb(95, 158, 160)),
    ("steelblue", rgb(70, 130, 180)),
    ("forestgreen", rgb(34, 139, 34)),
    ("darkgreen", rgb(0, 100, 0)),
    ("darkred", rgb(139, 0, 0)),
    ("darkblue", rgb(0, 0, 139)),
    ("violet", rgb(238, 130, 238)),
    ("tan", rgb(210, 180, 140)),
    ("salmon", rgb(250, 128, 114)),
    ("darksalmon", rgb(233, 150, 122)),
];

/// `XParseColor`: `#rgb`, `#rrggbb`, `#rrrrggggbbbb` or a name from the table.
pub fn parse_color(spec: &str) -> Option<Pixel> {
    let spec = spec.trim();
    if let Some(hex) = spec.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        // X allows 4, 8 or 12 bits per component; take the high 8 of each.
        let per = match hex.len() {
            3 => 1,
            6 => 2,
            12 => 4,
            _ => return None,
        };
        let comp = |i: usize| -> u8 {
            let s = &hex[i * per..i * per + per];
            let v = u32::from_str_radix(s, 16).unwrap_or(0);
            match per {
                1 => (v * 17) as u8,
                2 => v as u8,
                _ => (v >> 8) as u8,
            }
        };
        return Some(rgb(comp(0), comp(1), comp(2)));
    }
    let lower = spec.to_ascii_lowercase();
    NAMED.iter().find(|(n, _)| *n == lower).map(|(_, p)| *p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::rand::ya_rand_init;

    #[test]
    fn pixel_bytes_are_rgba() {
        // The host blits the framebuffer straight into an ImageData, so the
        // byte order has to be R, G, B, A with no swizzle.
        let p = rgb(0x11, 0x22, 0x33);
        assert_eq!(p.to_le_bytes(), [0x11, 0x22, 0x33, 0xFF]);
    }

    #[test]
    fn hsv_round_trips() {
        for h in (0..360).step_by(7) {
            let (r, g, b) = hsv_to_rgb(h, 1.0, 1.0);
            let (h2, s2, v2) = rgb_to_hsv(r, g, b);
            assert!((h2 - h).abs() <= 1, "hue drifted: {h} -> {h2}");
            assert!((s2 - 1.0).abs() < 0.01);
            assert!((v2 - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn hsv_clamps_out_of_range_input() {
        assert_eq!(hsv_to_rgb(0, -1.0, 2.0), hsv_to_rgb(0, 0.0, 1.0));
        // Negative hues wrap rather than indexing off the end of the sextant.
        assert_eq!(hsv_to_rgb(-60, 1.0, 1.0), hsv_to_rgb(300, 1.0, 1.0));
    }

    #[test]
    fn ramps_are_the_length_asked_for() {
        for closed in [false, true] {
            let c = make_color_ramp(0, 1.0, 1.0, 359, 1.0, 1.0, 64, closed);
            assert_eq!(c.len(), 64);
            assert!(c.iter().all(|c| c.pixel & ALPHA == ALPHA));
        }
    }

    #[test]
    fn colormaps_are_the_length_asked_for() {
        ya_rand_init(1);
        for n in [1usize, 2, 4, 17, 64, 256] {
            assert_eq!(make_smooth_colormap(n).len(), n, "smooth {n}");
            assert_eq!(make_uniform_colormap(n).len(), n, "uniform {n}");
            assert_eq!(make_random_colormap(n, true).len(), n, "random {n}");
            assert_eq!(make_random_colormap(n, false).len(), n, "dim {n}");
        }
        assert!(make_smooth_colormap(0).is_empty());
    }

    #[test]
    fn smooth_colormap_is_not_all_one_colour() {
        ya_rand_init(42);
        let c = make_smooth_colormap(64);
        let distinct = c
            .iter()
            .map(|c| c.pixel)
            .collect::<std::collections::HashSet<_>>();
        assert!(
            distinct.len() > 8,
            "only {} distinct colours",
            distinct.len()
        );
    }

    #[test]
    fn parses_colours() {
        assert_eq!(parse_color("black"), Some(BLACK));
        assert_eq!(parse_color("White"), Some(WHITE));
        assert_eq!(parse_color("#00FF00"), Some(rgb(0, 255, 0)));
        assert_eq!(parse_color("#0f0"), Some(rgb(0, 255, 0)));
        assert_eq!(parse_color("#00000000FFFF"), Some(rgb(0, 0, 255)));
        assert_eq!(parse_color("nonesuch"), None);
        assert_eq!(parse_color("#xyz"), None);
    }
}
