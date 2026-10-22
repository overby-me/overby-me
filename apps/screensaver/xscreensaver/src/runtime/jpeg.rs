//! Baseline JPEG, both ways.
//!
//! One saver needs this. `glitchpeg` corrupts an image file and shows what the
//! decoder makes of the wreckage, and the reason it is a *JPEG* file is that
//! JPEG fails beautifully: a damaged coefficient does not spoil a pixel, it
//! shifts the colour of every block after it until the next restart, so the
//! picture slides and smears in eight-pixel steps instead of turning to noise.
//!
//! Upstream reads the bytes of the file on disk, which is why its own comment
//! says the hack only works where there *is* a file. Here a picture arrives
//! already decoded, from the browser or from the compiled-in test card, so it
//! is encoded to a JPEG first and that is what gets corrupted. The result is
//! the same wreckage from the same kind of file; only the provenance differs.
//!
//! Both halves are baseline (sequential, Huffman, 8-bit) with no subsampling,
//! which is all the encoder emits and so all the decoder is asked to read. The
//! tables are the ones in Annex K of the standard, which is what every encoder
//! ships with.
//!
//! The decoder's defining property is that it does not give up. Every error a
//! corrupt file can produce is treated the way a real decoder treats it: a
//! Huffman code that is not in the table decodes as a zero, a coefficient
//! index that runs off the end of the block is dropped, and a marker in the
//! middle of the scan ends the picture early. Whatever was decoded before the
//! damage is returned.

use super::color::{rgb, unrgb};
use super::fb::Fb;

/// Zig-zag order: for each position in the coefficient stream, where it lives
/// in the 8x8 block.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The luminance quantisation table from Annex K, at quality 50.
const QUANT_LUMA: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// And the chrominance one, which throws away far more.
const QUANT_CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// A Huffman table as the file carries it: how many codes there are of each
/// length from 1 to 16, and then the values in order.
struct Spec {
    counts: [u8; 16],
    values: &'static [u8],
}

const DC_LUMA: Spec = Spec {
    counts: [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    values: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
};

const DC_CHROMA: Spec = Spec {
    counts: [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
    values: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
};

const AC_LUMA: Spec = Spec {
    counts: [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d],
    values: &[
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61,
        0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52,
        0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25,
        0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45,
        0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64,
        0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
        0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
        0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3,
        0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8,
        0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
};

const AC_CHROMA: Spec = Spec {
    counts: [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77],
    values: &[
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61,
        0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33,
        0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18,
        0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44,
        0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a,
        0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
        0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
        0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca,
        0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7,
        0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
};

/// A Huffman table in the form each half wants: the encoder needs a code per
/// value, the decoder needs to walk bit by bit.
struct Huffman {
    /// (length, code) for each value, indexed by the value itself.
    codes: [(u8, u16); 256],
    /// The smallest and largest code of each length, and where its values
    /// start in `values`.
    min_code: [i32; 17],
    max_code: [i32; 17],
    val_base: [usize; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn from_spec(spec: &Spec) -> Huffman {
        Huffman::new(&spec.counts, spec.values)
    }

    /// Build a table from what the file says, which is how a decoder has to do
    /// it: a damaged table is still a table, and it decodes to nonsense rather
    /// than to nothing.
    fn new(counts: &[u8; 16], values: &[u8]) -> Huffman {
        let mut h = Huffman {
            codes: [(0, 0); 256],
            min_code: [0; 17],
            max_code: [-1; 17],
            val_base: [0; 17],
            values: values.to_vec(),
        };
        let mut code = 0u16;
        let mut k = 0usize;
        for len in 1..=16 {
            let n = counts[len - 1] as usize;
            h.min_code[len] = i32::from(code);
            h.val_base[len] = k;
            for _ in 0..n {
                if let Some(v) = values.get(k) {
                    h.codes[*v as usize] = (len as u8, code);
                }
                code += 1;
                k += 1;
            }
            h.max_code[len] = if n == 0 { -1 } else { i32::from(code) - 1 };
            code <<= 1;
        }
        h
    }
}

/* ------------------------------------------------------------- encoding */

/// Writes bits, most significant first, with the byte stuffing the format
/// demands: an `FF` in the entropy-coded data is followed by a zero so it
/// cannot be mistaken for a marker.
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    /// Called `emit_bits` in libjpeg, and not called `write` here because it
    /// is bits into a buffer rather than anything to do with a file.
    fn emit(&mut self, code: u16, len: u8) {
        if len == 0 {
            return;
        }
        // In `u32`: the longest codes in the standard tables are sixteen bits,
        // and masking those in the code's own width overflows.
        let mask = (1u32 << len.min(16)) - 1;
        self.acc = (self.acc << len) | (u32::from(code) & mask);
        self.n += u32::from(len);
        while self.n >= 8 {
            self.n -= 8;
            let b = ((self.acc >> self.n) & 0xFF) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
        }
    }

    /// Pad the last byte with ones, as the standard says.
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.emit(0xFF, pad as u8);
        }
    }
}

/// How many bits a coefficient needs, and the bits themselves. Negative values
/// are stored as the one's complement, which is what makes the leading bit a
/// sign.
fn magnitude(v: i32) -> (u8, u16) {
    let mut size = 0;
    let mut a = v.unsigned_abs();
    while a != 0 {
        size += 1;
        a >>= 1;
    }
    let bits = if v < 0 { v - 1 } else { v } as u16;
    (size, bits)
}

/// The basis functions, `[u * 8 + x]`, which both transforms are made of.
///
/// Worked out once. Calling `cos` per coefficient is what separates a decoder
/// that keeps up with the frame rate from one that does not.
fn basis() -> &'static [f32; 64] {
    static TABLE: std::sync::OnceLock<[f32; 64]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0.0f32; 64];
        for u in 0..8 {
            let cu = if u == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            for x in 0..8 {
                t[u * 8 + x] = cu
                    * (std::f32::consts::PI * (2.0 * x as f32 + 1.0) * u as f32 / 16.0).cos()
                    * 0.5;
            }
        }
        t
    })
}

/// The forward transform, separable and in floating point. Slower than the
/// integer approximations everyone ships and rather easier to read.
fn fdct(block: &mut [f32; 64]) {
    let b = basis();
    let mut tmp = [0.0f32; 64];
    for u in 0..8 {
        for x in 0..8 {
            let c = b[u * 8 + x];
            for y in 0..8 {
                tmp[y * 8 + u] += block[y * 8 + x] * c;
            }
        }
    }
    *block = [0.0; 64];
    for v in 0..8 {
        for y in 0..8 {
            let c = b[v * 8 + y];
            for u in 0..8 {
                block[v * 8 + u] += tmp[y * 8 + u] * c;
            }
        }
    }
}

/// Scale a quantisation table for a quality between 1 and 100, the way
/// libjpeg does.
fn scale_quant(table: &[u16; 64], quality: i32) -> [u16; 64] {
    let q = quality.clamp(1, 100);
    let scale = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let mut out = [0u16; 64];
    for (o, t) in out.iter_mut().zip(table.iter()) {
        *o = (((i32::from(*t) * scale) + 50) / 100).clamp(1, 255) as u16;
    }
    out
}

fn marker(out: &mut Vec<u8>, m: u8) {
    out.push(0xFF);
    out.push(m);
}

fn segment(out: &mut Vec<u8>, m: u8, body: &[u8]) {
    marker(out, m);
    let len = body.len() + 2;
    out.push((len >> 8) as u8);
    out.push(len as u8);
    out.extend_from_slice(body);
}

/// Encode a drawable as a baseline JPEG.
///
/// No subsampling: the chrominance planes are full size. A photograph would
/// normally be encoded 4:2:0, but this exists to be broken rather than to be
/// small, and it keeps both halves to one loop.
pub fn encode(img: &Fb, quality: i32) -> Vec<u8> {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let qy = scale_quant(&QUANT_LUMA, quality);
    let qc = scale_quant(&QUANT_CHROMA, quality);

    let mut out = Vec::with_capacity((w * h) as usize / 4 + 1024);
    marker(&mut out, 0xD8); /* SOI */

    /* APP0, the JFIF header: units and density, and no thumbnail. */
    segment(
        &mut out,
        0xE0,
        &[b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0],
    );

    /* DQT, one segment per table. */
    for (id, q) in [(0u8, &qy), (1u8, &qc)] {
        let mut body = Vec::with_capacity(65);
        body.push(id); /* 8-bit precision, table id */
        for z in ZIGZAG {
            body.push(q[z] as u8);
        }
        segment(&mut out, 0xDB, &body);
    }

    /* SOF0: baseline, three components at 1x1. */
    let mut body = vec![8, (h >> 8) as u8, h as u8, (w >> 8) as u8, w as u8, 3];
    for (id, tq) in [(1u8, 0u8), (2, 1), (3, 1)] {
        body.extend_from_slice(&[id, 0x11, tq]);
    }
    segment(&mut out, 0xC0, &body);

    /* DHT, the four standard tables. */
    for (class_id, spec) in [
        (0x00u8, &DC_LUMA),
        (0x10, &AC_LUMA),
        (0x01, &DC_CHROMA),
        (0x11, &AC_CHROMA),
    ] {
        let mut body = vec![class_id];
        body.extend_from_slice(&spec.counts);
        body.extend_from_slice(spec.values);
        segment(&mut out, 0xC4, &body);
    }

    /* DRI: a restart marker at the end of every row of blocks.
    This is what makes damage local. Without it one bad bit desynchronises the
    decoder for the rest of the picture and there is nothing left to look at;
    with it, the decoder picks itself up at the next marker, and the damage
    shows as a band. Cameras write these for exactly the same reason. */
    let restart_interval = ((w + 7) / 8).max(1) as usize;
    segment(
        &mut out,
        0xDD,
        &[(restart_interval >> 8) as u8, restart_interval as u8],
    );

    /* SOS. */
    let mut body = vec![3u8];
    for (id, tables) in [(1u8, 0x00u8), (2, 0x11), (3, 0x11)] {
        body.extend_from_slice(&[id, tables]);
    }
    body.extend_from_slice(&[0, 63, 0]);
    segment(&mut out, 0xDA, &body);

    let dc_luma = Huffman::from_spec(&DC_LUMA);
    let ac_luma = Huffman::from_spec(&AC_LUMA);
    let dc_chroma = Huffman::from_spec(&DC_CHROMA);
    let ac_chroma = Huffman::from_spec(&AC_CHROMA);

    let mut bw = BitWriter { out, acc: 0, n: 0 };
    let mut prev_dc = [0i32; 3];

    let mcus_x = (w + 7) / 8;
    let mcus_y = (h + 7) / 8;
    let mut since_restart = 0usize;
    let mut restart_n = 0u8;
    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if since_restart == restart_interval {
                bw.flush();
                bw.out.push(0xFF);
                bw.out.push(0xD0 + restart_n);
                restart_n = (restart_n + 1) % 8;
                since_restart = 0;
                prev_dc = [0; 3];
            }
            since_restart += 1;

            // The three planes of this block, level-shifted.
            let mut planes = [[0.0f32; 64]; 3];
            for y in 0..8 {
                for x in 0..8 {
                    let (r, g, b) =
                        unrgb(img.get_pixel((mx * 8 + x).min(w - 1), (my * 8 + y).min(h - 1)));
                    let (r, g, b) = (f32::from(r), f32::from(g), f32::from(b));
                    let i = (y * 8 + x) as usize;
                    planes[0][i] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    planes[1][i] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                    planes[2][i] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
                }
            }

            for (c, plane) in planes.iter_mut().enumerate() {
                let (q, dc, ac) = if c == 0 {
                    (&qy, &dc_luma, &ac_luma)
                } else {
                    (&qc, &dc_chroma, &ac_chroma)
                };

                fdct(plane);
                let mut coef = [0i32; 64];
                for (k, z) in ZIGZAG.iter().enumerate() {
                    // The tables in Annex K have codes for coefficients of up
                    // to ten bits, and eleven for a DC difference. At a high
                    // quality the divisors are small enough to produce more
                    // than that, and a value with no code would be written
                    // with no code at all: a bit stream that says something
                    // else from there on. Clamp, as a conforming encoder must.
                    let limit = if k == 0 { 2047 } else { 1023 };
                    coef[k] = ((plane[*z] / f32::from(q[*z])).round() as i32).clamp(-limit, limit);
                }

                /* DC, as a difference from the last block's. */
                let diff = (coef[0] - prev_dc[c]).clamp(-2047, 2047);
                prev_dc[c] = coef[0];
                let (size, bits) = magnitude(diff);
                let (l, code) = dc.codes[size as usize];
                bw.emit(code, l);
                bw.emit(bits, size);

                /* AC, as runs of zeros and a value. */
                let mut run = 0;
                for c in coef.iter().skip(1) {
                    if *c == 0 {
                        run += 1;
                        continue;
                    }
                    while run > 15 {
                        let (l, code) = ac.codes[0xF0]; /* ZRL */
                        bw.emit(code, l);
                        run -= 16;
                    }
                    let (size, bits) = magnitude(*c);
                    let (l, code) = ac.codes[(run << 4 | u32::from(size)) as usize];
                    bw.emit(code, l);
                    bw.emit(bits, size);
                    run = 0;
                }
                if run > 0 {
                    let (l, code) = ac.codes[0x00]; /* EOB */
                    bw.emit(code, l);
                }
            }
        }
    }

    bw.flush();
    let mut out = bw.out;
    marker(&mut out, 0xD9); /* EOI */
    out
}

/* ------------------------------------------------------------- decoding */

/// Reads the entropy-coded data a bit at a time, unstuffing the `FF 00` pairs
/// and stopping at the next real marker.
struct BitReader<'a> {
    data: &'a [u8],
    at: usize,
    acc: u32,
    n: u32,
    /// Set once a marker or the end of the data has been reached.
    done: bool,
}

impl BitReader<'_> {
    fn bit(&mut self) -> u32 {
        if self.n == 0 {
            let Some(&b) = self.data.get(self.at) else {
                self.done = true;
                return 0;
            };
            self.at += 1;
            if b == 0xFF {
                match self.data.get(self.at) {
                    Some(0) => self.at += 1,
                    _ => {
                        // A marker. Stop, and leave the position on the `FF`
                        // rather than after it, so that whoever looks for the
                        // marker next can still see it.
                        self.at -= 1;
                        self.done = true;
                        return 0;
                    }
                }
            }
            self.acc = u32::from(b);
            self.n = 8;
        }
        self.n -= 1;
        (self.acc >> self.n) & 1
    }

    /// Read up to sixteen bits. A corrupt table can ask for more than a
    /// coefficient can hold; a real decoder does not oblige it.
    fn bits(&mut self, n: u8) -> i32 {
        let mut v = 0;
        for _ in 0..n.min(16) {
            v = (v << 1) | self.bit() as i32;
        }
        v
    }

    /// Walk the tree bit by bit. A code that is not in the table decodes as
    /// zero, which for a DC coefficient means "no change" and for an AC one
    /// means "end of block": the two most forgiving things it could mean.
    fn huffman(&mut self, h: &Huffman) -> u8 {
        let mut code = 0i32;
        for len in 1..=16 {
            code = (code << 1) | self.bit() as i32;
            if h.max_code[len] >= code && code >= h.min_code[len] {
                let i = h.val_base[len] + (code - h.min_code[len]) as usize;
                return h.values.get(i).copied().unwrap_or(0);
            }
            if self.done {
                return 0;
            }
        }
        0
    }

    /// Find the next restart marker and carry on from after it.
    ///
    /// This is the whole reason a damaged JPEG is worth looking at. The bits
    /// have gone out of step, so everything since the last marker is nonsense;
    /// but the markers are byte-aligned and unmistakable, so the decoder can
    /// find the next one, throw away the difference, and be right again from
    /// there. Damage shows as a band rather than as the end of the picture.
    ///
    /// False when there is no marker left, which means the scan is over.
    fn restart(&mut self) -> bool {
        self.n = 0;
        while self.at + 1 < self.data.len() {
            if self.data[self.at] == 0xFF {
                let m = self.data[self.at + 1];
                if (0xD0..=0xD7).contains(&m) {
                    self.at += 2;
                    self.done = false;
                    return true;
                }
                if m == 0xD9 {
                    /* End of the picture. */
                    break;
                }
                if m != 0 && m != 0xFF {
                    // Some other marker where a restart should have been.
                    // Damage does this constantly, because the byte after an
                    // `FF` in the data is a zero and changing it makes a
                    // marker out of nothing. Step over it and keep looking,
                    // which is what libjpeg's resynchronisation does.
                    self.at += 2;
                    continue;
                }
            }
            self.at += 1;
        }
        self.done = true;
        false
    }
}

/// Sign-extend a coefficient of `size` bits, undoing [`magnitude`].
fn extend(v: i32, size: u8) -> i32 {
    if size == 0 || size > 16 {
        0
    } else if v < (1 << (size - 1)) {
        v - (1 << size) + 1
    } else {
        v
    }
}

/// The inverse transform, the mirror of [`fdct`].
///
/// Two short cuts, both of them what every decoder does: a block with nothing
/// but a DC coefficient is a flat square, and a row or column of zeros
/// contributes nothing and can be skipped. Between them they take most of the
/// work out of a photograph, where the great majority of coefficients are
/// zero.
fn idct(block: &[f32; 64], out: &mut [f32; 64]) {
    let b = basis();

    if block[1..].iter().all(|c| *c == 0.0) {
        let flat = block[0] * 0.125;
        *out = [flat; 64];
        return;
    }

    let mut tmp = [0.0f32; 64];
    for u in 0..8 {
        // Is this column of coefficients empty?
        if (0..8).all(|v| block[v * 8 + u] == 0.0) {
            continue;
        }
        for x in 0..8 {
            let c = b[u * 8 + x];
            for v in 0..8 {
                tmp[v * 8 + x] += block[v * 8 + u] * c;
            }
        }
    }
    *out = [0.0; 64];
    for v in 0..8 {
        if (0..8).all(|x| tmp[v * 8 + x] == 0.0) {
            continue;
        }
        for y in 0..8 {
            let c = b[v * 8 + y];
            for x in 0..8 {
                out[y * 8 + x] += tmp[v * 8 + x] * c;
            }
        }
    }
}

/// What the file said before the scan started.
struct Frame {
    width: i32,
    height: i32,
    // Fields below are filled in from the file; the defaults are what a file
    // that never mentions them would mean.
    /// Quantisation tables by id.
    quant: [[u16; 64]; 4],
    /// Which quantisation table each of the three components uses.
    comp_quant: [usize; 3],
    /// And which pair of Huffman tables.
    comp_dc: [usize; 3],
    comp_ac: [usize; 3],
    /// How many blocks between restart markers, or zero for none.
    restart_interval: usize,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            width: 0,
            height: 0,
            quant: [[1; 64]; 4],
            comp_quant: [0; 3],
            comp_dc: [0; 3],
            comp_ac: [0; 3],
            restart_interval: 0,
        }
    }
}

/// Decode a baseline JPEG, returning whatever was decoded before the file ran
/// out or stopped making sense.
///
/// `None` only when there was never a picture to decode: no frame header, or
/// one with no size.
pub fn decode(bytes: &[u8]) -> Option<Fb> {
    let mut frame = Frame::default();
    let mut dc_tables: Vec<Option<Huffman>> = (0..4).map(|_| None).collect();
    let mut ac_tables: Vec<Option<Huffman>> = (0..4).map(|_| None).collect();
    let mut at = 0usize;
    let mut seen_frame = false;

    // Marker loop. Everything not needed for a baseline picture is skipped by
    // its length, which is also how a decoder survives the metadata that
    // cameras bury in these files.
    while at + 1 < bytes.len() {
        if bytes[at] != 0xFF {
            at += 1;
            continue;
        }
        let m = bytes[at + 1];
        at += 2;
        match m {
            0xD8 | 0x01 | 0xD0..=0xD7 => {} /* SOI, TEM, RSTn: no body */
            0xD9 => break,                  /* EOI */
            0xC0 | 0xC1 => {
                /* SOF0/1: baseline */
                let (body, next) = segment_body(bytes, at)?;
                if body.len() < 6 {
                    return None;
                }
                frame.height = i32::from(body[1]) << 8 | i32::from(body[2]);
                frame.width = i32::from(body[3]) << 8 | i32::from(body[4]);
                let n = body[5] as usize;
                for c in 0..n.min(3) {
                    if let Some(q) = body.get(6 + c * 3 + 2) {
                        frame.comp_quant[c] = (*q as usize).min(3);
                    }
                }
                seen_frame = true;
                at = next;
            }
            0xC4 => {
                /* DHT: one or more tables in one segment */
                let (body, next) = segment_body(bytes, at)?;
                let mut i = 0;
                while i + 17 <= body.len() {
                    let class = body[i] >> 4;
                    let id = (body[i] & 0xF) as usize;
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&body[i + 1..i + 17]);
                    let total: usize = counts.iter().map(|c| *c as usize).sum();
                    if i + 17 + total > body.len() {
                        break;
                    }
                    if id < 4 {
                        let h = Huffman::new(&counts, &body[i + 17..i + 17 + total]);
                        if class == 0 {
                            dc_tables[id] = Some(h);
                        } else {
                            ac_tables[id] = Some(h);
                        }
                    }
                    i += 17 + total;
                }
                at = next;
            }
            0xDD => {
                /* DRI */
                let (body, next) = segment_body(bytes, at)?;
                if body.len() >= 2 {
                    frame.restart_interval = usize::from(body[0]) << 8 | usize::from(body[1]);
                }
                at = next;
            }
            0xDB => {
                /* DQT */
                let (body, next) = segment_body(bytes, at)?;
                let mut i = 0;
                while i + 65 <= body.len() {
                    let id = (body[i] & 0xF) as usize;
                    if id < 4 {
                        for (k, z) in ZIGZAG.iter().enumerate() {
                            frame.quant[id][*z] = u16::from(body[i + 1 + k]);
                        }
                    }
                    i += 65;
                }
                at = next;
            }
            0xDA => {
                /* SOS: the scan itself follows the segment */
                let (body, next) = segment_body(bytes, at)?;
                let n = body.first().copied().unwrap_or(0) as usize;
                for c in 0..n.min(3) {
                    if let Some(t) = body.get(2 + c * 2) {
                        frame.comp_dc[c] = (*t >> 4) as usize & 3;
                        frame.comp_ac[c] = (*t & 0xF) as usize & 3;
                    }
                }
                if !seen_frame || frame.width <= 0 || frame.height <= 0 {
                    return None;
                }
                return Some(scan(bytes, next, &frame, &dc_tables, &ac_tables));
            }
            _ => {
                let (_, next) = segment_body(bytes, at)?;
                at = next;
            }
        }
    }

    None
}

/// The body of the segment starting at `at`, and where the next marker is.
fn segment_body(bytes: &[u8], at: usize) -> Option<(&[u8], usize)> {
    if at + 1 >= bytes.len() {
        return None;
    }
    let len = (usize::from(bytes[at]) << 8 | usize::from(bytes[at + 1])).max(2);
    let end = (at + len).min(bytes.len());
    Some((&bytes[at + 2..end], end))
}

/// Decode the entropy-coded data into a picture.
fn scan(
    bytes: &[u8],
    at: usize,
    frame: &Frame,
    dc_tables: &[Option<Huffman>],
    ac_tables: &[Option<Huffman>],
) -> Fb {
    let (w, h) = (frame.width, frame.height);
    let mut fb = Fb::new(w, h);
    fb.clear(rgb(0, 0, 0));

    let mut br = BitReader {
        data: bytes,
        at,
        acc: 0,
        n: 0,
        done: false,
    };
    let mut prev_dc = [0i32; 3];

    let mcus_x = (w + 7) / 8;
    let mcus_y = (h + 7) / 8;
    let mut since_restart = 0usize;
    'picture: for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if frame.restart_interval > 0 && since_restart == frame.restart_interval {
                since_restart = 0;
                prev_dc = [0; 3];
                if !br.restart() {
                    break 'picture;
                }
            }
            since_restart += 1;

            let mut planes = [[0.0f32; 64]; 3];

            for (c, plane) in planes.iter_mut().enumerate() {
                let q = &frame.quant[frame.comp_quant[c]];
                let (Some(dc), Some(ac)) = (
                    dc_tables[frame.comp_dc[c]].as_ref(),
                    ac_tables[frame.comp_ac[c]].as_ref(),
                ) else {
                    break 'picture;
                };

                let mut coef = [0.0f32; 64];

                let size = br.huffman(dc);
                let diff = extend(br.bits(size), size);
                prev_dc[c] += diff;
                coef[0] = prev_dc[c] as f32 * f32::from(q[0].max(1));

                let mut k = 1;
                while k < 64 {
                    let rs = br.huffman(ac);
                    let run = (rs >> 4) as usize;
                    let size = rs & 0xF;
                    if size == 0 {
                        if run == 15 {
                            k += 16; /* ZRL */
                            continue;
                        }
                        break; /* EOB */
                    }
                    k += run;
                    if k >= 64 {
                        break;
                    }
                    let z = ZIGZAG[k];
                    coef[z] = extend(br.bits(size), size) as f32 * f32::from(q[z].max(1));
                    k += 1;
                }

                idct(&coef, plane);
            }

            for y in 0..8 {
                for x in 0..8 {
                    let i = (y * 8 + x) as usize;
                    let yy = planes[0][i] + 128.0;
                    let cb = planes[1][i];
                    let cr = planes[2][i];
                    let r = yy + 1.402 * cr;
                    let g = yy - 0.344_136 * cb - 0.714_136 * cr;
                    let b = yy + 1.772 * cb;
                    let px = rgb(
                        r.clamp(0.0, 255.0) as u8,
                        g.clamp(0.0, 255.0) as u8,
                        b.clamp(0.0, 255.0) as u8,
                    );
                    fb.put_pixel(mx * 8 + x, my * 8 + y, px);
                }
            }

            if br.done {
                break 'picture;
            }
        }
    }

    fb
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture with edges, flat areas and saturated colour: everything the
    /// transform treats differently.
    fn test_image(w: i32, h: i32) -> Fb {
        let mut fb = Fb::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = if (x / 16 + y / 16) % 2 == 0 {
                    rgb((x * 255 / w) as u8, 40, (y * 255 / h) as u8)
                } else {
                    rgb(230, 230, 20)
                };
                fb.put_pixel(x, y, p);
            }
        }
        fb
    }

    /// Both halves have to agree about every code in every table, or the bit
    /// stream goes out of step at the first symbol they disagree on and
    /// everything after it is nonsense.
    #[test]
    fn every_code_in_every_table_round_trips() {
        for spec in [&DC_LUMA, &DC_CHROMA, &AC_LUMA, &AC_CHROMA] {
            let h = Huffman::from_spec(spec);
            let mut bw = BitWriter {
                out: Vec::new(),
                acc: 0,
                n: 0,
            };
            for v in spec.values {
                let (len, code) = h.codes[*v as usize];
                assert!(len > 0, "value {v:#04x} has no code");
                bw.emit(code, len);
            }
            bw.flush();

            let mut br = BitReader {
                data: &bw.out,
                at: 0,
                acc: 0,
                n: 0,
                done: false,
            };
            for v in spec.values {
                assert_eq!(br.huffman(&h), *v, "table disagrees about {v:#04x}");
            }
        }
    }

    #[test]
    fn a_picture_survives_the_round_trip() {
        let src = test_image(64, 48);
        let bytes = encode(&src, 90);
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "no SOI");
        assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9], "no EOI");

        let out = decode(&bytes).expect("did not decode");
        assert_eq!((out.width(), out.height()), (64, 48));

        // Lossy, but not by much at this quality: the mean error over the
        // whole picture should be a few levels, not tens.
        let mut total = 0u64;
        for y in 0..48 {
            for x in 0..64 {
                let (r1, g1, b1) = unrgb(src.get_pixel(x, y));
                let (r2, g2, b2) = unrgb(out.get_pixel(x, y));
                total += u64::from(r1.abs_diff(r2))
                    + u64::from(g1.abs_diff(g2))
                    + u64::from(b1.abs_diff(b2));
            }
        }
        let mean = total / (64 * 48 * 3);
        assert!(mean < 12, "lost too much: mean error {mean}");
    }

    /// The whole point of the codec: it has to produce a picture from a file
    /// that has been damaged, rather than refusing.
    #[test]
    fn a_corrupted_file_still_decodes_to_something() {
        crate::runtime::ya_rand_init(3);
        let src = test_image(96, 96);
        let clean = encode(&src, 75);

        let reference = decode(&clean).map(|f| f.pixels().to_vec());
        let mut decoded = 0;
        let mut different = 0;
        for _ in 0..40 {
            let mut broken = clean.clone();
            for _ in 0..20 {
                let i = 255 + (crate::runtime::random() as usize) % (broken.len() - 510);
                broken[i] = (crate::runtime::random() & 0xFF) as u8;
            }
            if let Some(out) = decode(&broken) {
                decoded += 1;
                if Some(out.pixels().to_vec()) != reference {
                    different += 1;
                }
            }
        }
        // Not every file: damage to the frame header or to a segment length
        // leaves nothing to decode, and upstream expects that too ("might be
        // null if decode fails"). Almost every file, though.
        assert!(decoded > 30, "gave up too often: {decoded}/40");
        assert!(different > 30, "damage did not show: {different}/40");
    }

    /// Truncation is the other half of it: a file that stops in the middle of
    /// the scan should come back as the part of the picture that arrived.
    #[test]
    fn a_truncated_file_decodes_as_far_as_it_got() {
        let src = test_image(128, 128);
        let bytes = encode(&src, 75);
        let half = &bytes[..bytes.len() / 2];
        let out = decode(half).expect("no picture at all");
        assert_eq!((out.width(), out.height()), (128, 128));

        // The top of the picture is there and the bottom is not.
        let lit = |fb: &Fb, y: i32| {
            (0..128)
                .filter(|x| fb.get_pixel(*x, y) != rgb(0, 0, 0))
                .count()
        };
        assert!(lit(&out, 4) > 100, "the top did not decode");
        assert_eq!(lit(&out, 124), 0, "the bottom should be missing");
    }

    #[test]
    fn nonsense_is_not_a_picture() {
        assert!(decode(&[]).is_none());
        assert!(decode(&[0xFF, 0xD8, 0xFF, 0xD9]).is_none());
        assert!(decode(b"this is not a jpeg at all").is_none());
    }
}
