//! Pixel pipeline: YUV 4:2:0 planes → interleaved RGB.
//!
//! Decoded HEVC frames arrive as planar YCbCr 4:2:0 (8- or 10/12-bit). This
//! module holds them in a uniform u16-plane representation (so grid tiles can
//! be blitted into one canvas regardless of storage width), then converts to
//! interleaved RGB using the `colr`/nclx colour description:
//!
//! - matrix coefficients (BT.601 / BT.709 / BT.2020) and full/limited range
//!   select the YCbCr→R'G'B' math;
//! - colour primaries 12 (Display P3 — the iPhone default) and 9 (BT.2020)
//!   are converted to sRGB primaries through linear light, so output is
//!   always display-ready sRGB;
//! - the transfer curve is treated as sRGB for that linearization (correct
//!   for Apple files, which tag transfer 13 = sRGB; a reasonable
//!   approximation for BT.709-tagged content).
//!
//! Chroma is upsampled bilinearly assuming HEVC's default siting (co-sited
//! with luma horizontally on the left, centred vertically).

use rust_h265::Frame;

use crate::error::HeifError;

/// The `colr` nclx colour description (CICP code points, H.273 numbering).
#[derive(Debug, Clone, Copy)]
pub struct Nclx {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

/// Defaults when a file carries no colour information: BT.709 matrix,
/// limited range, sRGB primaries — the safest guess for HEVC content.
impl Default for Nclx {
    fn default() -> Self {
        Nclx {
            primaries: 1,
            transfer: 13,
            matrix: 1,
            full_range: false,
        }
    }
}

/// Planar YCbCr 4:2:0 image with u16 sample storage (holds 8-bit values
/// unscaled; `bit_depth` says how many bits are meaningful).
pub struct YuvImage {
    pub width: usize,
    pub height: usize,
    pub bit_depth: u8,
    /// Luma plane, `width * height`.
    pub y: Vec<u16>,
    /// Chroma planes, `chroma_width() * chroma_height()`.
    pub u: Vec<u16>,
    pub v: Vec<u16>,
}

impl YuvImage {
    pub fn chroma_width(&self) -> usize {
        self.width.div_ceil(2)
    }

    pub fn chroma_height(&self) -> usize {
        self.height.div_ceil(2)
    }

    /// An all-black canvas, used as the target for grid assembly.
    pub fn black(width: usize, height: usize, bit_depth: u8) -> Self {
        let cw = width.div_ceil(2);
        let ch = height.div_ceil(2);
        // Black in YCbCr is Y=0 (or 16 in limited range — irrelevant, every
        // canvas pixel gets overwritten by a tile), Cb=Cr=half scale.
        let half = 1u16 << (bit_depth - 1);
        YuvImage {
            width,
            height,
            bit_depth,
            y: vec![0; width * height],
            u: vec![half; cw * ch],
            v: vec![half; cw * ch],
        }
    }

    /// Wrap a decoded `rust_h265::Frame`, normalizing both storage widths to
    /// u16 and validating the plane sizes the container promised.
    pub fn from_frame(frame: &Frame) -> Result<Self, HeifError> {
        let width = frame.width as usize;
        let height = frame.height as usize;
        let cw = width.div_ceil(2);
        let ch = height.div_ceil(2);
        let widen = |plane: &rust_h265::PixelData,
                     expect: usize,
                     name: &str|
         -> Result<Vec<u16>, HeifError> {
            let out: Vec<u16> = match plane {
                rust_h265::PixelData::U8(v) => v.iter().map(|&s| s as u16).collect(),
                rust_h265::PixelData::U16(v) => v.clone(),
            };
            if out.len() != expect {
                return Err(HeifError::Codec(format!(
                    "decoded {name} plane is {} samples, expected {expect}",
                    out.len()
                )));
            }
            Ok(out)
        };
        Ok(YuvImage {
            width,
            height,
            bit_depth: frame.bit_depth,
            y: widen(&frame.y, width * height, "luma")?,
            u: widen(&frame.u, cw * ch, "Cb")?,
            v: widen(&frame.v, cw * ch, "Cr")?,
        })
    }

    /// Copy `tile` into this canvas with its top-left at `(x, y)` (luma
    /// coordinates), clipping to the canvas edges. `(x, y)` must be even so
    /// the 4:2:0 chroma grids stay aligned — grid tiling guarantees this
    /// for every sane file (tile offsets are multiples of the tile size).
    pub fn blit(&mut self, tile: &YuvImage, x: usize, y: usize) -> Result<(), HeifError> {
        if x % 2 != 0 || y % 2 != 0 {
            return Err(HeifError::Unsupported(
                "grid tiles at odd offsets (chroma grids misaligned)".into(),
            ));
        }
        if tile.bit_depth != self.bit_depth {
            return Err(HeifError::Invalid(
                "grid tiles with mixed bit depths".into(),
            ));
        }
        // Luma rows.
        let copy_w = tile.width.min(self.width.saturating_sub(x));
        let copy_h = tile.height.min(self.height.saturating_sub(y));
        for row in 0..copy_h {
            let src = &tile.y[row * tile.width..row * tile.width + copy_w];
            let dst_start = (y + row) * self.width + x;
            self.y[dst_start..dst_start + copy_w].copy_from_slice(src);
        }
        // Chroma rows (both planes share geometry).
        let (scw, dcw) = (tile.chroma_width(), self.chroma_width());
        let copy_cw = scw.min(dcw.saturating_sub(x / 2));
        let copy_ch = tile
            .chroma_height()
            .min(self.chroma_height().saturating_sub(y / 2));
        for row in 0..copy_ch {
            let s = row * scw;
            let d = (y / 2 + row) * dcw + x / 2;
            self.u[d..d + copy_cw].copy_from_slice(&tile.u[s..s + copy_cw]);
            self.v[d..d + copy_cw].copy_from_slice(&tile.v[s..s + copy_cw]);
        }
        Ok(())
    }
}

/// Interleaved RGB output buffer; storage width follows the source bit
/// depth (8-bit sources → `Rgb8`, 10/12-bit → `Rgb16` scaled to 0..65535).
pub enum RgbBuffer {
    Rgb8(Vec<u8>),
    Rgb16(Vec<u16>),
}

/// Kr/Kb luma weights for the supported matrix coefficients.
fn luma_weights(matrix: u16) -> (f32, f32) {
    match matrix {
        5 | 6 => (0.299, 0.114),    // BT.601 (Apple HEIC tags matrix 6)
        9 | 10 => (0.2627, 0.0593), // BT.2020
        _ => (0.2126, 0.0722),      // BT.709 and everything unrecognized
    }
}

/// sRGB EOTF (decode gamma) for one channel.
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB OETF (encode gamma) for one channel.
fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear-light RGB→RGB matrix converting the tagged primaries to sRGB,
/// or `None` when the pixels are already sRGB (or we don't know better).
fn primaries_to_srgb_matrix(primaries: u16) -> Option<[[f32; 3]; 3]> {
    match primaries {
        // Display P3 (D65) → sRGB.
        12 => Some([
            [1.224_940_2, -0.224_940_18, 0.0],
            [-0.042_056_95, 1.042_056_9, 0.0],
            [-0.019_637_55, -0.078_636_05, 1.098_273_6],
        ]),
        // BT.2020 → sRGB.
        9 => Some([
            [1.660_491, -0.587_641_1, -0.072_849_86],
            [-0.124_550_47, 1.132_899_9, -0.008_349_42],
            [-0.018_150_76, -0.100_578_9, 1.118_729_7],
        ]),
        _ => None,
    }
}

/// Convert a YUV image to interleaved RGB, applying the nclx description.
///
/// Row-parallel across available cores — a 12-megapixel conversion is a
/// few hundred milliseconds single-threaded, which is worth splitting.
pub fn yuv_to_rgb(img: &YuvImage, nclx: &Nclx) -> RgbBuffer {
    let (kr, kb) = luma_weights(nclx.matrix);
    let kg = 1.0 - kr - kb;
    let scale = (1u32 << (img.bit_depth - 8)) as f32;
    let max = ((1u32 << img.bit_depth) - 1) as f32;
    let primaries_matrix = primaries_to_srgb_matrix(nclx.primaries);

    // Range expansion: limited range puts Y in [16..235]*2^(d-8) and chroma
    // in [16..240]*2^(d-8); full range uses the whole code range.
    let (y_off, y_span, c_span) = if nclx.full_range {
        (0.0, max, max)
    } else {
        (16.0 * scale, 219.0 * scale, 224.0 * scale)
    };
    let c_off = if nclx.full_range {
        // Full range centres chroma at 2^(d-1) exactly.
        (1u32 << (img.bit_depth - 1)) as f32
    } else {
        128.0 * scale
    };

    let cw = img.chroma_width();
    let ch = img.chroma_height();
    let w = img.width;
    let is_8bit = img.bit_depth == 8;
    let mut out8 = if is_8bit {
        vec![0u8; w * img.height * 3]
    } else {
        Vec::new()
    };
    let mut out16 = if is_8bit {
        Vec::new()
    } else {
        vec![0u16; w * img.height * 3]
    };

    // One closure converts a strip of rows [row_start, row_end) into the
    // matching slice of the output buffer; threads each take a strip.
    let convert_rows = |row_start: usize, rows8: &mut [u8], rows16: &mut [u16]| {
        for (local_row, y_coord) in (row_start..).enumerate().take(if is_8bit {
            rows8.len() / (w * 3)
        } else {
            rows16.len() / (w * 3)
        }) {
            // Vertical chroma position: samples are centred between luma
            // rows, so luma row y maps to chroma coordinate (y - 0.5) / 2.
            let cy = ((y_coord as f32 - 0.5) / 2.0).clamp(0.0, (ch - 1) as f32);
            let cy0 = cy.floor() as usize;
            let cy1 = (cy0 + 1).min(ch - 1);
            let fy = cy - cy0 as f32;
            for x in 0..w {
                // Horizontal chroma position: co-sited left → chroma sample
                // i sits at luma column 2i, so column x maps to x / 2.
                let cx = (x as f32 / 2.0).clamp(0.0, (cw - 1) as f32);
                let cx0 = cx.floor() as usize;
                let cx1 = (cx0 + 1).min(cw - 1);
                let fx = cx - cx0 as f32;

                let sample = |plane: &[u16]| -> f32 {
                    let s00 = plane[cy0 * cw + cx0] as f32;
                    let s01 = plane[cy0 * cw + cx1] as f32;
                    let s10 = plane[cy1 * cw + cx0] as f32;
                    let s11 = plane[cy1 * cw + cx1] as f32;
                    s00 * (1.0 - fx) * (1.0 - fy)
                        + s01 * fx * (1.0 - fy)
                        + s10 * (1.0 - fx) * fy
                        + s11 * fx * fy
                };

                let yv = ((img.y[y_coord * w + x] as f32 - y_off) / y_span).clamp(0.0, 1.0);
                let cb = (sample(&img.u) - c_off) / c_span;
                let cr = (sample(&img.v) - c_off) / c_span;

                // Standard YCbCr → R'G'B' from the Kr/Kb weights.
                let mut r = yv + 2.0 * (1.0 - kr) * cr;
                let mut b = yv + 2.0 * (1.0 - kb) * cb;
                let mut g = (yv - kr * r - kb * b) / kg;

                if let Some(m) = &primaries_matrix {
                    // Convert wide-gamut primaries to sRGB through linear
                    // light, clamping out-of-gamut results.
                    let rl = srgb_to_linear(r.clamp(0.0, 1.0));
                    let gl = srgb_to_linear(g.clamp(0.0, 1.0));
                    let bl = srgb_to_linear(b.clamp(0.0, 1.0));
                    r = linear_to_srgb(
                        (m[0][0] * rl + m[0][1] * gl + m[0][2] * bl).clamp(0.0, 1.0),
                    );
                    g = linear_to_srgb(
                        (m[1][0] * rl + m[1][1] * gl + m[1][2] * bl).clamp(0.0, 1.0),
                    );
                    b = linear_to_srgb(
                        (m[2][0] * rl + m[2][1] * gl + m[2][2] * bl).clamp(0.0, 1.0),
                    );
                }

                let i = local_row * w * 3 + x * 3;
                if is_8bit {
                    rows8[i] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    rows8[i + 1] = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    rows8[i + 2] = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                } else {
                    rows16[i] = (r.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                    rows16[i + 1] = (g.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                    rows16[i + 2] = (b.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                }
            }
        }
    };

    // Split the output into per-thread row strips and convert in parallel.
    // The closure is shared by reference so each spawned thread can call it.
    let convert_rows = &convert_rows;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let rows_per_strip = ((img.height + threads - 1) / threads.max(1)).max(1);
    if is_8bit {
        let strips: Vec<(usize, &mut [u8])> = out8
            .chunks_mut(rows_per_strip * w * 3)
            .enumerate()
            .map(|(i, c)| (i * rows_per_strip, c))
            .collect();
        crate::par::scope(|s| {
            for (row_start, strip) in strips {
                s.spawn(move || convert_rows(row_start, strip, &mut []));
            }
        });
        RgbBuffer::Rgb8(out8)
    } else {
        let strips: Vec<(usize, &mut [u16])> = out16
            .chunks_mut(rows_per_strip * w * 3)
            .enumerate()
            .map(|(i, c)| (i * rows_per_strip, c))
            .collect();
        crate::par::scope(|s| {
            for (row_start, strip) in strips {
                s.spawn(move || convert_rows(row_start, &mut [], strip));
            }
        });
        RgbBuffer::Rgb16(out16)
    }
}

#[cfg(test)]
#[path = "color_tests.rs"]
mod tests;
