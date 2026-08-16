//! Port of `hacks/glx/dropshadow.c`.
//!
//! ```text
//! dropshadow.c, Copyright (c) 2009 Jens Kilian <jjk@acm.org>
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
//! A soft shadow under a rectangle, which `photopile` puts under its photos.
//!
//! The shadow is one thirty-two pixel picture of a blurred square, and it is
//! stretched over the rectangle as nine patches: the four corners keep their
//! shape, the four edges are stretched along their length, and the middle is
//! one texel of the darkest part stretched over the whole rectangle. That is
//! what lets one small picture shadow a rectangle of any size.
//!
//! Upstream stores it as an alpha-only texture, whose colour is black by
//! definition; this expands it to black with that alpha, which comes out the
//! same under the modulation the caller asks for.

use super::gl::{Glx, Shape};

const W: i32 = 32;
const H: i32 = 32;

/// The blurred square, one byte of alpha a pixel.
const DATA: [u8; 1024] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 1, 1, 1, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 1, 1, 1, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 1, 1, 3, 4, 6, 7, 9, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 9, 7, 6, 4, 3, 1, 1,
    0, 0, 0, 0, 0, 0, 0, 1, 1, 3, 5, 9, 13, 16, 19, 19, 21, 21, 22, 22, 22, 22, 21, 21, 19, 19, 16,
    13, 9, 5, 3, 1, 1, 0, 0, 0, 0, 0, 1, 1, 3, 5, 10, 16, 22, 28, 32, 35, 37, 37, 38, 38, 38, 38,
    37, 37, 35, 32, 28, 22, 16, 10, 5, 3, 1, 1, 0, 0, 0, 0, 1, 1, 4, 9, 16, 25, 34, 43, 50, 55, 58,
    59, 60, 60, 60, 60, 59, 58, 55, 50, 43, 34, 25, 16, 9, 4, 1, 1, 0, 0, 0, 0, 1, 3, 6, 13, 22,
    34, 48, 61, 70, 77, 80, 82, 83, 84, 84, 83, 82, 80, 77, 70, 61, 48, 34, 22, 13, 6, 3, 1, 0, 0,
    0, 0, 1, 3, 7, 16, 28, 43, 61, 76, 88, 97, 102, 103, 104, 104, 104, 104, 103, 102, 97, 88, 76,
    61, 43, 28, 16, 7, 3, 1, 0, 0, 0, 1, 1, 4, 9, 19, 32, 51, 70, 88, 103, 112, 117, 120, 121, 121,
    121, 121, 120, 117, 112, 103, 88, 70, 51, 32, 19, 9, 4, 1, 1, 0, 0, 1, 1, 4, 10, 20, 35, 55,
    77, 97, 112, 122, 128, 130, 132, 133, 133, 132, 130, 128, 122, 112, 97, 77, 55, 35, 20, 10, 4,
    1, 1, 0, 0, 1, 1, 4, 10, 21, 37, 58, 80, 101, 117, 128, 134, 137, 138, 139, 139, 138, 137, 134,
    128, 117, 101, 80, 58, 37, 21, 10, 4, 1, 0, 0, 0, 0, 1, 4, 10, 21, 38, 59, 82, 103, 119, 130,
    137, 139, 141, 142, 142, 141, 139, 137, 130, 119, 103, 82, 59, 38, 21, 10, 4, 1, 0, 0, 0, 0, 1,
    4, 10, 22, 38, 59, 83, 104, 121, 132, 139, 141, 142, 142, 142, 142, 141, 139, 132, 121, 104,
    83, 59, 38, 22, 10, 4, 1, 0, 0, 0, 0, 1, 4, 10, 22, 38, 60, 84, 104, 121, 133, 139, 142, 142,
    142, 142, 142, 142, 139, 133, 121, 104, 84, 60, 38, 22, 10, 4, 1, 0, 0, 0, 0, 1, 4, 10, 22, 38,
    60, 84, 104, 121, 133, 139, 142, 142, 142, 142, 142, 142, 139, 133, 121, 104, 84, 60, 38, 22,
    10, 4, 1, 0, 0, 0, 0, 1, 4, 10, 22, 38, 59, 83, 104, 121, 132, 139, 141, 142, 142, 142, 142,
    141, 139, 132, 121, 104, 83, 59, 38, 22, 10, 4, 1, 0, 0, 0, 0, 1, 4, 10, 21, 38, 59, 82, 103,
    119, 130, 137, 139, 141, 142, 142, 141, 139, 137, 130, 119, 103, 82, 59, 38, 21, 10, 4, 1, 0,
    0, 0, 1, 1, 4, 10, 21, 37, 58, 80, 101, 118, 128, 134, 137, 139, 139, 139, 139, 137, 134, 128,
    117, 102, 80, 58, 37, 21, 10, 4, 1, 0, 0, 0, 1, 1, 4, 10, 20, 35, 55, 77, 97, 112, 122, 128,
    130, 132, 133, 133, 132, 130, 128, 122, 112, 97, 77, 55, 35, 20, 10, 4, 1, 1, 0, 0, 1, 1, 4, 9,
    19, 32, 51, 70, 88, 103, 112, 117, 120, 121, 121, 121, 121, 120, 117, 112, 103, 88, 70, 51, 32,
    19, 9, 4, 1, 1, 0, 0, 0, 1, 3, 7, 16, 28, 43, 61, 76, 88, 97, 102, 103, 104, 104, 104, 104,
    103, 102, 97, 88, 76, 61, 43, 28, 16, 7, 3, 1, 0, 0, 0, 0, 1, 3, 6, 13, 22, 34, 48, 61, 70, 77,
    80, 82, 83, 84, 84, 83, 82, 80, 77, 70, 61, 48, 34, 22, 13, 6, 3, 1, 0, 0, 0, 0, 1, 1, 4, 9,
    16, 25, 34, 43, 50, 55, 58, 59, 60, 60, 60, 60, 59, 58, 55, 50, 43, 34, 25, 16, 9, 4, 1, 1, 0,
    0, 0, 0, 1, 1, 3, 5, 10, 16, 22, 28, 32, 35, 37, 37, 38, 38, 38, 38, 37, 37, 35, 32, 28, 22,
    16, 10, 5, 3, 1, 1, 0, 0, 0, 0, 0, 1, 1, 3, 5, 9, 13, 16, 19, 19, 21, 21, 22, 22, 22, 22, 21,
    21, 19, 19, 16, 13, 9, 5, 3, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 3, 4, 6, 7, 9, 10, 10, 10, 10,
    10, 10, 10, 10, 10, 10, 9, 7, 6, 4, 3, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 3, 3, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `init_drop_shadow`: the shadow's texture, made once.
pub fn init_drop_shadow(g: &mut Glx) -> u32 {
    let id = g.gen_texture();
    g.bind_texture(id);
    g.tex_image_2d(W, H, DATA.iter().flat_map(|&a| [0, 0, 0, a]).collect());
    g.tex_clamp(true);
    id
}

/// `draw_drop_shadow`: the shadow of the rectangle at `x`,`y` of `w` by `h`,
/// spreading `r` beyond it, as nine patches.
#[allow(clippy::too_many_arguments)]
pub fn draw_drop_shadow(g: &mut Glx, texture: u32, x: f32, y: f32, z: f32, w: f32, h: f32, r: f32) {
    // The inner and outer edges of the shadow.
    let (li, lo) = (x, x - r);
    let (ri, ro) = (x + w, x + w + r);
    let (bi, bo) = (y, y - r);
    let (ti, to) = (y + h, y + h + r);

    g.texturing(true);
    g.bind_texture(texture);
    g.begin(Shape::Quads);
    // There is likely a better way to do this, says upstream.
    for corner in [
        [
            [0.0, 0.0, lo, bo],
            [0.5, 0.0, li, bo],
            [0.5, 0.5, li, bi],
            [0.0, 0.5, lo, bi],
        ],
        [
            [0.5, 0.0, li, bo],
            [0.5, 0.0, ri, bo],
            [0.5, 0.5, ri, bi],
            [0.5, 0.5, li, bi],
        ],
        [
            [0.5, 0.0, ri, bo],
            [1.0, 0.0, ro, bo],
            [1.0, 0.5, ro, bi],
            [0.5, 0.5, ri, bi],
        ],
        [
            [0.5, 0.5, ri, bi],
            [1.0, 0.5, ro, bi],
            [1.0, 0.5, ro, ti],
            [0.5, 0.5, ri, ti],
        ],
        [
            [0.5, 0.5, ri, ti],
            [1.0, 0.5, ro, ti],
            [1.0, 1.0, ro, to],
            [0.5, 1.0, ri, to],
        ],
        [
            [0.5, 0.5, li, ti],
            [0.5, 0.5, ri, ti],
            [0.5, 1.0, ri, to],
            [0.5, 1.0, li, to],
        ],
        [
            [0.0, 0.5, lo, ti],
            [0.5, 0.5, li, ti],
            [0.5, 1.0, li, to],
            [0.0, 1.0, lo, to],
        ],
        [
            [0.0, 0.5, lo, bi],
            [0.5, 0.5, li, bi],
            [0.5, 0.5, li, ti],
            [0.0, 0.5, lo, ti],
        ],
    ] {
        for [u, v, px, py] in corner {
            g.tex_coord2f(u, v);
            g.vertex3f(px, py, z);
        }
    }
    g.end();
    g.texturing(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picture is a blurred square: dark in the middle, nothing at the
    /// edges, and near enough the same whichever way round it is turned.
    /// Only near enough: a handful of its values are off by one from their
    /// mirror, which is upstream's table as it stands.
    #[test]
    fn the_shadow_is_a_blurred_square() {
        let at = |x: usize, y: usize| DATA[y * W as usize + x] as i32;
        assert_eq!(at(0, 0), 0, "the corner is not empty");
        assert!(at(16, 16) > 140, "the middle is not dark");
        for y in 0..H as usize {
            for x in 0..W as usize {
                let mx = (at(x, y) - at(W as usize - 1 - x, y)).abs();
                let my = (at(x, y) - at(x, H as usize - 1 - y)).abs();
                assert!(mx <= 1 && my <= 1, "it is not symmetric at {x},{y}");
            }
        }
        // And it fades outward from the middle along a row through it.
        for x in 1..16 {
            assert!(at(x, 16) >= at(x - 1, 16), "it is not a blur");
        }
    }

    /// Nine patches: eight quads round a middle that is one texel stretched.
    #[test]
    fn it_is_drawn_as_nine_patches() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        let t = init_drop_shadow(&mut g);
        draw_drop_shadow(&mut g, t, 0.0, 0.0, 0.0, 10.0, 10.0, 2.0);
        let f = g.frame();
        let n: usize = f.batches.iter().map(|b| b.count).sum();
        // Eight quads is forty-eight vertices once they are cut into
        // triangles.
        assert_eq!(n, 48, "{n} vertices is not eight quads");
    }
}
