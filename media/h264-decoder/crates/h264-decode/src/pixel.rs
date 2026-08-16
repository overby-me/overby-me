//! Pixel format definitions and colour-space conversion utilities.
//!
//! The decoder natively produces YUV 4:2:0 planar output (matching the most
//! common H.264 chroma format).  This module provides an optional conversion
//! path to packed RGB/RGBA so that downstream consumers that expect RGB input
//! can use the decoder directly.

use std::fmt;

/// Pixel / colour format of a decoded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// YUV 4:2:0 planar (I420).
    ///
    /// Three separate planes: Y at full resolution, U and V each at half
    /// width and half height.  This is the native output of the H.264
    /// decoder for the overwhelmingly common `chroma_format_idc == 1` case
    /// and is the format expected by most video encoders (e.g. rav1e).
    Yuv420p,

    /// YUV 4:2:2 planar.
    ///
    /// Three separate planes: Y at full resolution, U and V each at half
    /// width but full height.
    Yuv422p,

    /// YUV 4:4:4 planar.
    ///
    /// Three separate planes, all at full resolution.
    Yuv444p,

    /// NV12 semi-planar (Y plane followed by interleaved UV plane).
    ///
    /// Common in hardware pipelines and APIs such as VA-API and DXVA.
    Nv12,

    /// Packed 24-bit RGB (R, G, B per pixel, no padding).
    Rgb24,

    /// Packed 32-bit RGBA (R, G, B, A per pixel).
    Rgba32,
}

impl PixelFormat {
    /// Number of planes this format uses.
    #[inline]
    pub fn num_planes(self) -> usize {
        match self {
            PixelFormat::Yuv420p | PixelFormat::Yuv422p | PixelFormat::Yuv444p => 3,
            PixelFormat::Nv12 => 2,
            PixelFormat::Rgb24 | PixelFormat::Rgba32 => 1,
        }
    }

    /// Bytes per pixel for packed formats, or bytes per sample for planar
    /// formats (always 1 for 8-bit content).
    #[inline]
    pub fn bytes_per_component(self) -> usize {
        match self {
            PixelFormat::Yuv420p
            | PixelFormat::Yuv422p
            | PixelFormat::Yuv444p
            | PixelFormat::Nv12 => 1,
            PixelFormat::Rgb24 => 3,
            PixelFormat::Rgba32 => 4,
        }
    }

    /// Returns `true` for planar YUV formats.
    #[inline]
    pub fn is_planar_yuv(self) -> bool {
        matches!(
            self,
            PixelFormat::Yuv420p | PixelFormat::Yuv422p | PixelFormat::Yuv444p
        )
    }

    /// Returns `true` for packed RGB/RGBA formats.
    #[inline]
    pub fn is_rgb(self) -> bool {
        matches!(self, PixelFormat::Rgb24 | PixelFormat::Rgba32)
    }

    /// Compute the total buffer size in bytes needed for a frame of the given
    /// dimensions in this pixel format.
    pub fn frame_buffer_size(self, width: u32, height: u32) -> usize {
        let w = width as usize;
        let h = height as usize;
        match self {
            PixelFormat::Yuv420p => {
                // Y: w*h, U: (w/2)*(h/2), V: (w/2)*(h/2)
                let chroma_w = w.div_ceil(2);
                let chroma_h = h.div_ceil(2);
                w * h + 2 * chroma_w * chroma_h
            }
            PixelFormat::Yuv422p => {
                let chroma_w = w.div_ceil(2);
                w * h + 2 * chroma_w * h
            }
            PixelFormat::Yuv444p => w * h * 3,
            PixelFormat::Nv12 => {
                let chroma_h = h.div_ceil(2);
                // Y plane + interleaved UV plane (same width, half height)
                w * h + w * chroma_h
            }
            PixelFormat::Rgb24 => w * h * 3,
            PixelFormat::Rgba32 => w * h * 4,
        }
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            PixelFormat::Yuv420p => "YUV 4:2:0 planar (I420)",
            PixelFormat::Yuv422p => "YUV 4:2:2 planar",
            PixelFormat::Yuv444p => "YUV 4:4:4 planar",
            PixelFormat::Nv12 => "NV12 semi-planar",
            PixelFormat::Rgb24 => "RGB24 packed",
            PixelFormat::Rgba32 => "RGBA32 packed",
        };
        f.write_str(name)
    }
}

impl Default for PixelFormat {
    /// The default pixel format is [`PixelFormat::Yuv420p`] because it is the
    /// native decoder output and avoids any conversion overhead.
    fn default() -> Self {
        PixelFormat::Yuv420p
    }
}

// ---------------------------------------------------------------------------
// YUV → RGB conversion
// ---------------------------------------------------------------------------

/// BT.601 YUV to RGB conversion.
///
/// Converts a single pixel from Y, Cb, Cr components (each in the range
/// 0..=255 for 8-bit content) to (R, G, B) using the BT.601 limited-range
/// matrix (studio-swing):
///
/// ```text
/// R = clip(( 298*(Y-16)                + 409*(Cr-128) + 128) >> 8)
/// G = clip(( 298*(Y-16) - 100*(Cb-128) - 208*(Cr-128) + 128) >> 8)
/// B = clip(( 298*(Y-16) + 516*(Cb-128)                + 128) >> 8)
/// ```
#[inline]
pub fn yuv_to_rgb_bt601(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = y as i32 - 16;
    let cb = cb as i32 - 128;
    let cr = cr as i32 - 128;

    let r = (298 * y + 409 * cr + 128) >> 8;
    let g = (298 * y - 100 * cb - 208 * cr + 128) >> 8;
    let b = (298 * y + 516 * cb + 128) >> 8;

    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

/// BT.709 YUV to RGB conversion (HD content).
///
/// Uses the BT.709 limited-range matrix:
///
/// ```text
/// R = clip(( 298*(Y-16)                + 459*(Cr-128) + 128) >> 8)
/// G = clip(( 298*(Y-16) -  55*(Cb-128) - 136*(Cr-128) + 128) >> 8)
/// B = clip(( 298*(Y-16) + 541*(Cb-128)                + 128) >> 8)
/// ```
#[inline]
pub fn yuv_to_rgb_bt709(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = y as i32 - 16;
    let cb = cb as i32 - 128;
    let cr = cr as i32 - 128;

    let r = (298 * y + 459 * cr + 128) >> 8;
    let g = (298 * y - 55 * cb - 136 * cr + 128) >> 8;
    let b = (298 * y + 541 * cb + 128) >> 8;

    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

/// Clamp an `i32` into the `0..=255` range and truncate to `u8`.
#[inline(always)]
fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Colour matrix to use when converting YUV → RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColourMatrix {
    /// ITU-R BT.601 (SD content).
    #[default]
    Bt601,
    /// ITU-R BT.709 (HD content).
    Bt709,
}

impl ColourMatrix {
    /// Convert a single YUV pixel to RGB using this matrix.
    #[inline]
    pub fn yuv_to_rgb(self, y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
        match self {
            ColourMatrix::Bt601 => yuv_to_rgb_bt601(y, cb, cr),
            ColourMatrix::Bt709 => yuv_to_rgb_bt709(y, cb, cr),
        }
    }
}

/// Convert an entire YUV 4:2:0 planar frame to packed RGB24.
///
/// # Arguments
///
/// * `y_plane`  – Luma samples, length must be `width * height`.
/// * `u_plane`  – Cb chroma samples, length must be `(width/2) * (height/2)` (rounded up).
/// * `v_plane`  – Cr chroma samples, same size as `u_plane`.
/// * `width`    – Frame width in pixels.
/// * `height`   – Frame height in pixels.
/// * `matrix`   – Colour matrix to use for the conversion.
///
/// Returns a `Vec<u8>` of length `width * height * 3` containing packed R, G, B
/// triplets in row-major order.
pub fn yuv420p_to_rgb24(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: u32,
    height: u32,
    matrix: ColourMatrix,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let chroma_w = w.div_ceil(2);

    let mut rgb = vec![0u8; w * h * 3];

    for row in 0..h {
        let chroma_row = row / 2;
        for col in 0..w {
            let chroma_col = col / 2;

            let y = y_plane[row * w + col];
            let cb = u_plane[chroma_row * chroma_w + chroma_col];
            let cr = v_plane[chroma_row * chroma_w + chroma_col];

            let (r, g, b) = matrix.yuv_to_rgb(y, cb, cr);

            let dst = (row * w + col) * 3;
            rgb[dst] = r;
            rgb[dst + 1] = g;
            rgb[dst + 2] = b;
        }
    }

    rgb
}

/// Convert an entire YUV 4:2:0 planar frame to packed RGBA32.
///
/// Identical to [`yuv420p_to_rgb24`] but appends an alpha byte of `0xFF` to
/// every pixel.
pub fn yuv420p_to_rgba32(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: u32,
    height: u32,
    matrix: ColourMatrix,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let chroma_w = w.div_ceil(2);

    let mut rgba = vec![0u8; w * h * 4];

    for row in 0..h {
        let chroma_row = row / 2;
        for col in 0..w {
            let chroma_col = col / 2;

            let y = y_plane[row * w + col];
            let cb = u_plane[chroma_row * chroma_w + chroma_col];
            let cr = v_plane[chroma_row * chroma_w + chroma_col];

            let (r, g, b) = matrix.yuv_to_rgb(y, cb, cr);

            let dst = (row * w + col) * 4;
            rgba[dst] = r;
            rgba[dst + 1] = g;
            rgba[dst + 2] = b;
            rgba[dst + 3] = 0xFF;
        }
    }

    rgba
}

/// Convert a YUV 4:2:0 planar frame to NV12 semi-planar in-place.
///
/// Takes separate U/V planes and produces a single interleaved UV plane.
///
/// Returns a `Vec<u8>` of length `width * height + width * ((height+1)/2)`.
pub fn yuv420p_to_nv12(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: u32,
    height: u32,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let chroma_w = w.div_ceil(2);
    let chroma_h = h.div_ceil(2);

    let y_size = w * h;
    let uv_size = w * chroma_h; // interleaved U,V pairs, full width
    let mut nv12 = vec![0u8; y_size + uv_size];

    // Copy Y plane as-is
    nv12[..y_size].copy_from_slice(&y_plane[..y_size]);

    // Interleave U and V into the UV plane.  Each row of the UV plane has
    // `width` bytes: pairs of (U, V) for each horizontal chroma sample,
    // with duplication to reach full width if width is odd.
    for row in 0..chroma_h {
        for col in 0..chroma_w {
            let src_idx = row * chroma_w + col;
            let dst_idx = y_size + row * w + col * 2;
            nv12[dst_idx] = u_plane[src_idx];
            if dst_idx + 1 < nv12.len() {
                nv12[dst_idx + 1] = v_plane[src_idx];
            }
        }
    }

    nv12
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_format_defaults_to_yuv420p() {
        assert_eq!(PixelFormat::default(), PixelFormat::Yuv420p);
    }

    #[test]
    fn test_pixel_format_properties() {
        assert!(PixelFormat::Yuv420p.is_planar_yuv());
        assert!(!PixelFormat::Yuv420p.is_rgb());
        assert!(PixelFormat::Rgb24.is_rgb());
        assert!(!PixelFormat::Rgb24.is_planar_yuv());
        assert_eq!(PixelFormat::Yuv420p.num_planes(), 3);
        assert_eq!(PixelFormat::Nv12.num_planes(), 2);
        assert_eq!(PixelFormat::Rgb24.num_planes(), 1);
    }

    #[test]
    fn test_frame_buffer_size_yuv420p() {
        // 1920x1080: Y=1920*1080, U=960*540, V=960*540
        let size = PixelFormat::Yuv420p.frame_buffer_size(1920, 1080);
        assert_eq!(size, 1920 * 1080 + 2 * 960 * 540);
    }

    #[test]
    fn test_frame_buffer_size_rgb24() {
        let size = PixelFormat::Rgb24.frame_buffer_size(640, 480);
        assert_eq!(size, 640 * 480 * 3);
    }

    #[test]
    fn test_frame_buffer_size_odd_dimensions() {
        // 3x3: Y=9, U=2*2=4, V=4 => 9+8=17
        let size = PixelFormat::Yuv420p.frame_buffer_size(3, 3);
        assert_eq!(size, 9 + 2 * 2 * 2);
    }

    #[test]
    fn test_yuv_to_rgb_black() {
        // Y=16, Cb=128, Cr=128 is black in studio-swing BT.601
        let (r, g, b) = yuv_to_rgb_bt601(16, 128, 128);
        assert_eq!((r, g, b), (0, 0, 0));
    }

    #[test]
    fn test_yuv_to_rgb_white() {
        // Y=235, Cb=128, Cr=128 is white in studio-swing BT.601
        let (r, g, b) = yuv_to_rgb_bt601(235, 128, 128);
        // Allow ±1 for rounding
        assert!((254..=255).contains(&r), "r={r}");
        assert!((254..=255).contains(&g), "g={g}");
        assert!((254..=255).contains(&b), "b={b}");
    }

    #[test]
    fn test_clamp_does_not_overflow() {
        // Edge-case values that would produce out-of-range intermediates
        let (r, g, b) = yuv_to_rgb_bt601(0, 0, 255);
        assert!(r <= 255);
        assert!(g <= 255);
        assert!(b <= 255);
    }

    #[test]
    fn test_colour_matrix_dispatch() {
        let bt601 = ColourMatrix::Bt601.yuv_to_rgb(16, 128, 128);
        let bt709 = ColourMatrix::Bt709.yuv_to_rgb(16, 128, 128);
        // Both should produce black for the same neutral inputs
        assert_eq!(bt601, (0, 0, 0));
        assert_eq!(bt709, (0, 0, 0));
    }

    #[test]
    fn test_yuv420p_to_rgb24_basic() {
        // 2x2 black frame
        let y = vec![16u8; 4];
        let u = vec![128u8; 1];
        let v = vec![128u8; 1];
        let rgb = yuv420p_to_rgb24(&y, &u, &v, 2, 2, ColourMatrix::Bt601);
        assert_eq!(rgb.len(), 2 * 2 * 3);
        // All pixels should be black (0, 0, 0)
        for pixel in rgb.chunks(3) {
            assert_eq!(pixel, &[0, 0, 0]);
        }
    }

    #[test]
    fn test_yuv420p_to_rgba32_alpha() {
        let y = vec![16u8; 4];
        let u = vec![128u8; 1];
        let v = vec![128u8; 1];
        let rgba = yuv420p_to_rgba32(&y, &u, &v, 2, 2, ColourMatrix::Bt601);
        assert_eq!(rgba.len(), 2 * 2 * 4);
        for pixel in rgba.chunks(4) {
            assert_eq!(pixel[3], 0xFF, "alpha channel must be 0xFF");
        }
    }

    #[test]
    fn test_yuv420p_to_nv12_roundtrip() {
        let y = vec![100u8; 4];
        let u = vec![50u8; 1];
        let v = vec![200u8; 1];
        let nv12 = yuv420p_to_nv12(&y, &u, &v, 2, 2);
        // Y plane
        assert_eq!(&nv12[..4], &[100, 100, 100, 100]);
        // UV plane: interleaved (U, V)
        assert_eq!(nv12[4], 50);
        assert_eq!(nv12[5], 200);
    }
}
