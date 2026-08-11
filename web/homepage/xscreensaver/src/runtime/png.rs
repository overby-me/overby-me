//! Reading the PNG files the hacks carry around with them.
//!
//! This is a different thing from [`super::image`]. That is a channel for
//! *pictures*, the photograph a saver melts or ripples, which arrives from
//! outside and might never arrive at all. These are program data: the Matrix
//! glyph sheet, the noseguy's eight poses, the test cards a television tunes
//! between. Upstream compiles them in as C arrays generated at build time
//! (`images/gen/*_png.h`) and decodes them at startup with libpng through
//! `ximage-loader.c`; here the bytes are `include_bytes!` and this is the
//! decoder.
//!
//! It stays inside the subset upstream's own files use, which is every colour
//! type at bit depths 1 through 8, no interlacing and no 16-bit samples. Meeting
//! anything else returns `None` rather than guessing.

use super::color::{Pixel, RGB_MASK, rgb, unrgb};
use super::fb::{Fb, XImage};

/// Decode a PNG into a colour image and a bitmap of where it is opaque.
///
/// `image_data_to_pixmap`, and the pair is the point. X has no alpha channel
/// and neither does [`Fb`], so a hack draws a sprite by setting the clip mask
/// to the bitmap and copying the colour through it, which is what upstream
/// does. The mask is `None` when nothing is transparent, which tells a hack it
/// can skip the clipping.
///
/// Returns `None` if the bytes are not a PNG this understands, which for
/// compiled-in data means a mistake made once at the desk rather than something
/// to handle at runtime.
pub fn decode(bytes: &[u8]) -> Option<(XImage, Option<Fb>)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return None;
    }

    let mut header: Option<Header> = None;
    let mut palette: Vec<Pixel> = Vec::new();
    let mut compressed: Vec<u8> = Vec::new();
    let mut transparent: Option<&[u8]> = None;

    let mut at = 8;
    while at + 8 <= bytes.len() {
        let len = be32(&bytes[at..])? as usize;
        let kind = &bytes[at + 4..at + 8];
        let data = bytes.get(at + 8..at + 8 + len)?;
        match kind {
            b"IHDR" => header = Some(Header::parse(data)?),
            b"PLTE" => {
                palette = data
                    .chunks_exact(3)
                    .map(|c| rgb(c[0], c[1], c[2]))
                    .collect();
            }
            b"tRNS" => transparent = Some(data),
            b"IDAT" => compressed.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        // Length, type, data, CRC. The CRC is not checked: these bytes came out
        // of the binary, so a bad one is a corrupt build, not a corrupt file.
        at += 12 + len;
    }

    let header = header?;
    if let Some(alpha) = transparent {
        apply_trns(&header, &mut palette, alpha);
    }
    let raw = inflate(&compressed)?;
    let scanlines = defilter(&header, &raw)?;
    header.to_image(&scanlines, &palette)
}

/// What IHDR says.
struct Header {
    width: i32,
    height: i32,
    depth: u8,
    colour: u8,
}

impl Header {
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 13 {
            return None;
        }
        let width = be32(data)? as i32;
        let height = be32(&data[4..])? as i32;
        let (depth, colour) = (data[8], data[9]);
        let (compression, filter, interlace) = (data[10], data[11], data[12]);
        if width <= 0 || height <= 0 || compression != 0 || filter != 0 || interlace != 0 {
            return None;
        }
        if !matches!(depth, 1 | 2 | 4 | 8) {
            return None;
        }
        let channels = channels(colour)?;
        // Palette entries are indices, so the other colour types are the ones
        // that may not be packed below a byte.
        if depth < 8 && channels != 1 {
            return None;
        }
        Some(Self {
            width,
            height,
            depth,
            colour,
        })
    }

    /// Bits in one pixel.
    fn pixel_bits(&self) -> usize {
        self.depth as usize * channels(self.colour).unwrap_or(1) as usize
    }

    /// Bytes in one scanline, rounded up.
    fn stride(&self) -> usize {
        (self.width as usize * self.pixel_bits()).div_ceil(8)
    }

    /// Turn defiltered scanlines into pixels, and into the mask beside them.
    fn to_image(&self, rows: &[u8], palette: &[Pixel]) -> Option<(XImage, Option<Fb>)> {
        let mut img = Fb::new(self.width, self.height);
        let mut mask = Fb::new_bitmap(self.width, self.height);
        let mut any_clear = false;
        let stride = self.stride();
        // Samples below eight bits are stretched to fill the byte, so a two-bit
        // 0b10 becomes 0xAA rather than 0x80 and white stays white.
        let scale = (255 / ((1u32 << self.depth) - 1)) as u8;
        for y in 0..self.height as usize {
            let row = rows.get(y * stride..(y + 1) * stride)?;
            for x in 0..self.width as usize {
                let px = match self.colour {
                    0 => {
                        let g = sample(row, self.depth, x)? * scale;
                        rgb(g, g, g)
                    }
                    2 => {
                        let p = row.get(x * 3..x * 3 + 3)?;
                        rgb(p[0], p[1], p[2])
                    }
                    3 => *palette.get(sample(row, self.depth, x)? as usize)?,
                    4 => {
                        let p = row.get(x * 2..x * 2 + 2)?;
                        (rgb(p[0], p[0], p[0]) & RGB_MASK) | u32::from(p[1]) << 24
                    }
                    6 => {
                        let p = row.get(x * 4..x * 4 + 4)?;
                        (rgb(p[0], p[1], p[2]) & RGB_MASK) | u32::from(p[3]) << 24
                    }
                    _ => return None,
                };
                // Upstream thresholds at half, because X can only say yes or no.
                let opaque = (px >> 24) >= 0x80;
                any_clear |= !opaque;
                mask.put_pixel(x as i32, y as i32, opaque as Pixel);
                img.put_pixel(x as i32, y as i32, px);
            }
        }
        Some((img, any_clear.then_some(mask)))
    }
}

/// Samples per pixel for a colour type, or `None` if it is not one.
fn channels(colour: u8) -> Option<u8> {
    match colour {
        0 | 3 => Some(1),
        4 => Some(2),
        2 => Some(3),
        6 => Some(4),
        _ => None,
    }
}

/// One sub-byte or whole-byte sample out of a scanline, big end first.
fn sample(row: &[u8], depth: u8, index: usize) -> Option<u8> {
    if depth == 8 {
        return row.get(index).copied();
    }
    let per_byte = 8 / depth as usize;
    let byte = *row.get(index / per_byte)?;
    let shift = 8 - depth as usize * (index % per_byte + 1);
    Some((byte >> shift) & ((1 << depth) - 1))
}

/// Fold a tRNS chunk into the palette, or into nothing.
///
/// Only the palette form matters here: the grey and truecolour forms nominate
/// one exact value as transparent, which none of the upstream files use, and
/// which would cost a branch in the inner loop of every image that does not.
fn apply_trns(header: &Header, palette: &mut [Pixel], alpha: &[u8]) {
    if header.colour != 3 {
        return;
    }
    for (entry, a) in palette.iter_mut().zip(alpha) {
        *entry = (*entry & RGB_MASK) | u32::from(*a) << 24;
    }
}

/// Undo the per-scanline filters, returning the raw scanlines end to end.
///
/// Each line begins with a filter byte and is reconstructed from the bytes to
/// its left and the line above, which is why this cannot be folded into the
/// pixel loop: the line above has to already be finished.
fn defilter(header: &Header, raw: &[u8]) -> Option<Vec<u8>> {
    let stride = header.stride();
    let bpp = header.pixel_bits().div_ceil(8);
    let height = header.height as usize;
    let mut out = vec![0u8; stride * height];
    for y in 0..height {
        let filter = *raw.get(y * (stride + 1))?;
        let line = raw.get(y * (stride + 1) + 1..(y + 1) * (stride + 1))?;
        for x in 0..stride {
            let a = if x >= bpp {
                out[y * stride + x - bpp]
            } else {
                0
            };
            let b = if y > 0 { out[(y - 1) * stride + x] } else { 0 };
            let c = if x >= bpp && y > 0 {
                out[(y - 1) * stride + x - bpp]
            } else {
                0
            };
            let v = line[x];
            out[y * stride + x] = match filter {
                0 => v,
                1 => v.wrapping_add(a),
                2 => v.wrapping_add(b),
                3 => v.wrapping_add((((a as u16) + (b as u16)) / 2) as u8),
                4 => v.wrapping_add(paeth(a, b, c)),
                _ => return None,
            };
        }
    }
    Some(out)
}

/// The Paeth predictor: whichever of left, above and above-left is closest to
/// their linear combination.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = (
        (p - a as i16).abs(),
        (p - b as i16).abs(),
        (p - c as i16).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn be32(b: &[u8]) -> Option<u32> {
    let b: [u8; 4] = b.get(..4)?.try_into().ok()?;
    Some(u32::from_be_bytes(b))
}

// ---------------------------------------------------------------------------
// DEFLATE
// ---------------------------------------------------------------------------

/// Decompress a zlib stream (RFC 1950 wrapping RFC 1951).
///
/// Bit-at-a-time Huffman decoding rather than a lookup table. The largest thing
/// this ever unpacks is a few hundred kilobytes, once, at startup.
fn inflate(zlib: &[u8]) -> Option<Vec<u8>> {
    // Two-byte header: compression method and window size, then flags. A preset
    // dictionary would need one we do not have, so refuse it.
    let (cmf, flg) = (*zlib.first()?, *zlib.get(1)?);
    let check = u16::from(cmf) * 256 + u16::from(flg);
    if cmf & 0x0F != 8 || !check.is_multiple_of(31) || flg & 0x20 != 0 {
        return None;
    }

    let mut bits = BitReader {
        bytes: &zlib[2..],
        at: 0,
        bit: 0,
    };
    let mut out = Vec::new();

    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => {
                bits.align();
                let len = bits.bits(16)? as usize;
                let nlen = bits.bits(16)?;
                if len as u32 != !nlen & 0xFFFF {
                    return None;
                }
                for _ in 0..len {
                    out.push(bits.byte()?);
                }
            }
            1 => {
                let (lit, dist) = fixed_trees();
                inflate_block(&mut bits, &lit, &dist, &mut out)?;
            }
            2 => {
                let (lit, dist) = dynamic_trees(&mut bits)?;
                inflate_block(&mut bits, &lit, &dist, &mut out)?;
            }
            _ => return None,
        }
        if last {
            return Some(out);
        }
    }
}

/// A canonical Huffman table: how many codes there are of each length, and the
/// symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    /// Build from a code length per symbol, as both DEFLATE tree forms give it.
    fn new(lengths: &[u8]) -> Self {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;

        // Where each length's symbols start in the flat list.
        let mut offsets = [0u16; 16];
        for l in 1..15 {
            offsets[l + 1] = offsets[l] + counts[l];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        Self { counts, symbols }
    }

    /// Read one symbol. Walks the code lengths shortest first, which is exactly
    /// the property canonical codes are built to have.
    fn decode(&self, bits: &mut BitReader) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return self.symbols.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

/// The tree DEFLATE uses when a block says "the usual one".
fn fixed_trees() -> (Huffman, Huffman) {
    let mut lit = [0u8; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (Huffman::new(&lit), Huffman::new(&[5u8; 30]))
}

/// The tree a block carries with it, itself Huffman-coded.
fn dynamic_trees(bits: &mut BitReader) -> Option<(Huffman, Huffman)> {
    // The order the code-length code's own lengths are stored in, which puts the
    // lengths most likely to be used first so the tail can be omitted.
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let nlit = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlit > 286 || ndist > 30 {
        return None;
    }

    let mut code_lengths = [0u8; 19];
    for &slot in ORDER.iter().take(ncode) {
        code_lengths[slot] = bits.bits(3)? as u8;
    }
    let code_tree = Huffman::new(&code_lengths);

    let mut lengths = vec![0u8; nlit + ndist];
    let mut at = 0;
    while at < lengths.len() {
        let sym = code_tree.decode(bits)?;
        let (repeat, value) = match sym {
            0..=15 => {
                lengths[at] = sym as u8;
                at += 1;
                continue;
            }
            // Repeat the previous length, or a run of zeroes.
            16 => (bits.bits(2)? + 3, *lengths.get(at.checked_sub(1)?)?),
            17 => (bits.bits(3)? + 3, 0),
            18 => (bits.bits(7)? + 11, 0),
            _ => return None,
        };
        for _ in 0..repeat {
            *lengths.get_mut(at)? = value;
            at += 1;
        }
    }

    Some((
        Huffman::new(&lengths[..nlit]),
        Huffman::new(&lengths[nlit..]),
    ))
}

/// The body of a Huffman-coded block: literals straight out, matches copied
/// from what has already been written.
fn inflate_block(
    bits: &mut BitReader,
    lit: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
) -> Option<()> {
    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    loop {
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Some(()),
            257..=285 => {
                let i = sym as usize - 257;
                let length = LEN_BASE[i] as usize + bits.bits(LEN_EXTRA[i] as u32)? as usize;
                let d = dist.decode(bits)? as usize;
                let distance =
                    *DIST_BASE.get(d)? as usize + bits.bits(*DIST_EXTRA.get(d)? as u32)? as usize;
                let from = out.len().checked_sub(distance)?;
                // Byte at a time on purpose: a match may overlap what it is
                // producing, which is how a run of one byte is coded.
                for i in 0..length {
                    out.push(out[from + i]);
                }
            }
            _ => return None,
        }
    }
}

/// Bits out of a byte slice, least significant first, which is the order
/// DEFLATE packs them in.
struct BitReader<'a> {
    bytes: &'a [u8],
    at: usize,
    bit: u32,
}

impl BitReader<'_> {
    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut v = 0u32;
        for i in 0..count {
            let byte = *self.bytes.get(self.at)?;
            v |= ((byte >> self.bit) as u32 & 1) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.at += 1;
            }
        }
        Some(v)
    }

    /// Drop to the next byte boundary, as a stored block requires.
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.at += 1;
        }
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(b)
    }
}

/// The same PNG, but as the bytes a GL texture wants: RGBA, row zero at the
/// top, ready for `glTexImage2D`.
///
/// [`decode`] hands back what a 2D hack needs, a colour image and a separate
/// bitmap saying where it is opaque, because a framebuffer has nowhere to put
/// an alpha channel. A texture does, so the two are put back together here and
/// the transparent pixels come out with an alpha of zero.
pub fn decode_rgba(bytes: &[u8]) -> Option<(i32, i32, Vec<u8>)> {
    let (image, mask) = decode(bytes)?;
    let (w, h) = (image.width(), image.height());
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            // A Pixel keeps red in the low byte, so unpack rather than shift.
            let (r, g, b) = unrgb(image.get_pixel(x, y));
            let opaque = mask
                .as_ref()
                .is_none_or(|m| m.get_pixel(x, y) & RGB_MASK != 0);
            out.extend_from_slice(&[r, g, b, if opaque { 0xFF } else { 0 }]);
        }
    }
    Some((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real file, written by a real encoder, so the Huffman path runs against
    /// something this test did not also produce.
    ///
    /// `bob.png` is 64x64 with a four-bit palette and a one-entry tRNS: a dark
    /// line drawing of a face on a transparent ground. Checking the drawing is
    /// there catches a decoder that gets the size right and the pixels wrong.
    #[test]
    fn the_bundled_image_decodes() {
        let (img, mask) = decode(crate::images::BOB).expect("bob");
        assert_eq!((img.width(), img.height()), (64, 64));
        let mask = mask.expect("the ground is transparent");
        assert_eq!(mask.get_pixel(0, 0), 0, "the corner is not the face");

        let dark = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|(x, y)| img.get_pixel(*x, *y) & 0xFF < 0x80)
            .count();
        assert!(
            (300..2000).contains(&dark),
            "a line drawing, not a smear: {dark} dark pixels of 4096"
        );
    }

    #[test]
    fn a_transparent_image_gets_a_mask_an_opaque_one_does_not() {
        // Colour type 6, with alpha rising across the row from nothing.
        let (_, mask) = decode(&build_png(8, 6, &[])).expect("rgba");
        let mask = mask.expect("the left of the image is transparent");
        assert_eq!(mask.depth(), 1);
        assert_eq!(mask.get_pixel(0, 0), 0, "should be clear");
        assert_eq!(mask.get_pixel(3, 0), 1, "should be set");

        let (_, mask) = decode(&build_png(8, 2, &[])).expect("truecolour");
        assert!(mask.is_none(), "nothing here has any alpha to lose");
    }

    /// A round trip through the filter types, since a wrong Paeth or Average
    /// shows up as a smear rather than as a failure to decode.
    ///
    /// Four scanlines using filters 1 to 4, and the fifth type is what the
    /// palette images below use.
    #[test]
    fn every_scanline_filter_reconstructs() {
        let (img, _) = decode(&build_png(8, 2, &[])).expect("hand-built PNG");
        assert_eq!((img.width(), img.height()), (4, 4));
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(img.get_pixel(x, y), truecolour(x, y), "at {x},{y}");
            }
        }
    }

    /// Every colour type and bit depth the upstream files are stored in.
    ///
    /// Grey, palette and alpha take different routes out of the same scanline,
    /// and a sub-byte sample is read out of the middle of a byte, so each is a
    /// separate chance to be off by a shift.
    #[test]
    fn every_colour_type_and_bit_depth_survives() {
        let palette: Vec<u8> = (0..16u8)
            .flat_map(|i| [i * 17, 255 - i * 17, i * 8])
            .collect();

        for (depth, colour) in [
            (1, 0),
            (2, 0),
            (4, 0),
            (8, 0),
            (1, 3),
            (2, 3),
            (4, 3),
            (8, 3),
        ] {
            let (img, _) = decode(&build_png(depth, colour, &palette)).expect("hand-built PNG");
            for y in 0..4 {
                for x in 0..4 {
                    let v = sample_value(depth, x, y);
                    let want = if colour == 0 {
                        let g = (u32::from(v) * 255 / ((1 << depth) - 1)) as u8;
                        rgb(g, g, g)
                    } else {
                        let i = v as usize * 3;
                        rgb(palette[i], palette[i + 1], palette[i + 2])
                    };
                    assert_eq!(img.get_pixel(x, y), want, "depth {depth} type {colour}");
                }
            }
        }

        // Grey plus alpha, and the RGBA the sprite sheets use. The alpha lands
        // in the mask rather than the image, which has none to put it in.
        for colour in [4, 6] {
            let (img, mask) = decode(&build_png(8, colour, &[])).expect("hand-built PNG");
            let mask = mask.expect("alpha runs from 0 to 180 across the row");
            for y in 0..4 {
                for x in 0..4 {
                    let p = img.get_pixel(x, y);
                    assert_eq!(p >> 24, 0xFF, "the image itself is always opaque");
                    // Alpha is 0, 60, 120, 180, so only the last clears the half.
                    assert_eq!(mask.get_pixel(x, y), u32::from(x > 2), "mask at {x},{y}");
                    let want = if colour == 4 {
                        let g = (y * 60) as u8;
                        rgb(g, g, g)
                    } else {
                        truecolour(x, y)
                    };
                    assert_eq!(p & RGB_MASK, want & RGB_MASK, "colour at {x},{y}");
                }
            }
        }
    }

    /// The colour of a truecolour test pixel. Distinct in all three channels,
    /// because a decoder that puts red where blue goes is right about the size
    /// and the shape and wrong about everything the viewer sees.
    fn truecolour(x: i32, y: i32) -> Pixel {
        rgb((x * 40) as u8, (y * 40) as u8, (x * y * 5) as u8)
    }

    /// The value of a single-sample test pixel, which has to fit the bit depth.
    fn sample_value(depth: u8, x: i32, y: i32) -> u8 {
        ((x + y) as u32 & ((1u32 << depth) - 1)) as u8
    }

    /// The same colour as `truecolour`, but in the order a PNG stores it.
    fn channels_of(x: i32, y: i32) -> [u8; 3] {
        [(x * 40) as u8, (y * 40) as u8, (x * y * 5) as u8]
    }

    /// Build a 4x4 PNG in the given format, filtering each scanline differently.
    fn build_png(depth: u8, colour: u8, palette: &[u8]) -> Vec<u8> {
        let channels = match colour {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        };
        let stride = (4 * depth as usize * channels).div_ceil(8);
        let bpp = (depth as usize * channels).div_ceil(8);

        let mut raw = Vec::new();
        let mut previous = vec![0u8; stride];
        for y in 0..4i32 {
            let mut line = vec![0u8; stride];
            for x in 0..4i32 {
                match colour {
                    0 | 3 => {
                        let v = sample_value(depth, x, y);
                        if depth == 8 {
                            line[x as usize] = v;
                        } else {
                            let per_byte = 8 / depth as usize;
                            let shift = 8 - depth as usize * (x as usize % per_byte + 1);
                            line[x as usize / per_byte] |= v << shift;
                        }
                    }
                    2 => {
                        let at = x as usize * 3;
                        line[at..at + 3].copy_from_slice(&channels_of(x, y));
                    }
                    4 => {
                        line[x as usize * 2] = (y * 60) as u8;
                        line[x as usize * 2 + 1] = (x * 60) as u8;
                    }
                    _ => {
                        let at = x as usize * 4;
                        line[at..at + 3].copy_from_slice(&channels_of(x, y));
                        line[at + 3] = (x * 60) as u8;
                    }
                }
            }

            let filter = (y + 1) as u8;
            raw.push(filter);
            for x in 0..stride {
                let a = if x >= bpp { line[x - bpp] } else { 0 };
                let b = previous[x];
                let c = if x >= bpp { previous[x - bpp] } else { 0 };
                raw.push(match filter {
                    1 => line[x].wrapping_sub(a),
                    2 => line[x].wrapping_sub(b),
                    3 => line[x].wrapping_sub((((a as u16) + (b as u16)) / 2) as u8),
                    _ => line[x].wrapping_sub(paeth(a, b, c)),
                });
            }
            previous = line;
        }

        // A zlib stream of one stored block: writing a compressor to test a
        // decompressor proves nothing, and the real file above covers the two
        // Huffman block types.
        let mut z = vec![0x78, 0x01, 1];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(&raw);
        let (mut lo, mut hi) = (1u32, 0u32);
        for b in &raw {
            lo = (lo + u32::from(*b)) % 65521;
            hi = (hi + lo) % 65521;
        }
        z.extend_from_slice(&(hi << 16 | lo).to_be_bytes());

        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        // The CRC is written as zero: `decode` does not check it, deliberately.
        let mut chunk = |kind: &[u8; 4], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            png.extend_from_slice(&[0, 0, 0, 0]);
        };
        let mut ihdr = 4u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&4u32.to_be_bytes());
        ihdr.extend_from_slice(&[depth, colour, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        if colour == 3 {
            chunk(b"PLTE", palette);
        }
        chunk(b"IDAT", &z);
        chunk(b"IEND", &[]);
        png
    }
}
