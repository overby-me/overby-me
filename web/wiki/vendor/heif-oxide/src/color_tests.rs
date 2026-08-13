//! YUV→RGB conversion tests against hand-computed reference values.

use super::*;

/// Flat image helper: every luma sample `y`, every chroma sample `u`/`v`.
fn flat(width: usize, height: usize, bit_depth: u8, y: u16, u: u16, v: u16) -> YuvImage {
    let cw = width.div_ceil(2);
    let ch = height.div_ceil(2);
    YuvImage {
        width,
        height,
        bit_depth,
        y: vec![y; width * height],
        u: vec![u; cw * ch],
        v: vec![v; cw * ch],
    }
}

fn as_rgb8(buf: RgbBuffer) -> Vec<u8> {
    match buf {
        RgbBuffer::Rgb8(v) => v,
        RgbBuffer::Rgb16(_) => panic!("expected 8-bit output"),
    }
}

fn assert_flat_rgb(buf: &[u8], expected: [u8; 3], tolerance: i16) {
    for px in buf.chunks_exact(3) {
        for c in 0..3 {
            let diff = (px[c] as i16 - expected[c] as i16).abs();
            assert!(
                diff <= tolerance,
                "pixel {:?} != expected {:?}",
                &px[..3],
                expected
            );
        }
    }
}

/// Limited-range mid gray: Y=128 → (128-16)/219 = 0.51142 → 130.
#[test]
fn limited_range_gray() {
    let img = flat(4, 4, 8, 128, 128, 128);
    let nclx = Nclx {
        matrix: 6,
        full_range: false,
        ..Nclx::default()
    };
    assert_flat_rgb(&as_rgb8(yuv_to_rgb(&img, &nclx)), [130, 130, 130], 1);
}

/// Full-range gray passes through numerically: Y=128 → 128.
#[test]
fn full_range_gray() {
    let img = flat(4, 4, 8, 128, 128, 128);
    let nclx = Nclx {
        matrix: 6,
        full_range: true,
        ..Nclx::default()
    };
    assert_flat_rgb(&as_rgb8(yuv_to_rgb(&img, &nclx)), [128, 128, 128], 1);
}

/// BT.601 limited-range pure red: YCbCr (81, 90, 240) → RGB (254, 0, 0).
/// (Reference values derived by hand from the Kr=0.299/Kb=0.114 equations.)
#[test]
fn bt601_limited_red() {
    let img = flat(4, 4, 8, 81, 90, 240);
    let nclx = Nclx {
        matrix: 6,
        full_range: false,
        ..Nclx::default()
    };
    assert_flat_rgb(&as_rgb8(yuv_to_rgb(&img, &nclx)), [254, 0, 0], 1);
}

/// BT.601 limited-range pure green and blue.
#[test]
fn bt601_limited_green_blue() {
    let nclx = Nclx {
        matrix: 6,
        full_range: false,
        ..Nclx::default()
    };
    let green = flat(4, 4, 8, 145, 54, 34);
    assert_flat_rgb(&as_rgb8(yuv_to_rgb(&green, &nclx)), [0, 255, 1], 1);
    let blue = flat(4, 4, 8, 41, 240, 110);
    assert_flat_rgb(&as_rgb8(yuv_to_rgb(&blue, &nclx)), [0, 0, 255], 1);
}

/// A 10-bit source produces full-scale u16 output: limited-range Y=512 →
/// (512-64)/876 = 0.511416 → 33516.
#[test]
fn ten_bit_outputs_rgb16() {
    let img = flat(4, 4, 10, 512, 512, 512);
    let nclx = Nclx {
        matrix: 6,
        full_range: false,
        ..Nclx::default()
    };
    match yuv_to_rgb(&img, &nclx) {
        RgbBuffer::Rgb16(v) => {
            for &s in &v {
                assert!((s as i32 - 33516).abs() <= 2, "sample {s}");
            }
        }
        RgbBuffer::Rgb8(_) => panic!("expected 16-bit output"),
    }
}

/// Display P3 primaries change the output: a saturated-but-in-gamut red
/// becomes *more* saturated when reinterpreted from the wider P3 gamut
/// into sRGB (R rises, G falls). Sanity check, not a colorimetric one.
#[test]
fn p3_primaries_shift_colors() {
    // Full-range BT.601 encoding of R'G'B' = (0.60, 0.30, 0.30):
    // Y' = 0.299*.6+0.587*.3+0.114*.3 = 0.3897 → 99.4
    // Cb = (0.3 - 0.3897)/1.772 * 255 + 128 = 115.1
    // Cr = (0.6 - 0.3897)/1.402 * 255 + 128 = 166.3
    let img = flat(4, 4, 8, 99, 115, 166);
    let srgb_tagged = Nclx {
        primaries: 1,
        matrix: 6,
        full_range: true,
        ..Nclx::default()
    };
    let p3_tagged = Nclx {
        primaries: 12,
        matrix: 6,
        full_range: true,
        ..Nclx::default()
    };
    let srgb = as_rgb8(yuv_to_rgb(&img, &srgb_tagged));
    let p3 = as_rgb8(yuv_to_rgb(&img, &p3_tagged));
    assert!(p3[0] > srgb[0], "P3 red should map above sRGB red");
    assert!(p3[1] < srgb[1], "P3 green should map below sRGB green");
}

/// Grid blitting places tiles at the right plane offsets, chroma included.
#[test]
fn blit_places_tiles() {
    let mut canvas = YuvImage::black(4, 2, 8);
    let left = flat(2, 2, 8, 10, 20, 30);
    let right = flat(2, 2, 8, 40, 50, 60);
    canvas.blit(&left, 0, 0).unwrap();
    canvas.blit(&right, 2, 0).unwrap();
    assert_eq!(canvas.y, vec![10, 10, 40, 40, 10, 10, 40, 40]);
    assert_eq!(canvas.u, vec![20, 50]);
    assert_eq!(canvas.v, vec![30, 60]);
}

/// Odd blit offsets (which would shear the chroma grid) are rejected.
#[test]
fn blit_rejects_odd_offsets() {
    let mut canvas = YuvImage::black(4, 4, 8);
    let tile = flat(2, 2, 8, 0, 0, 0);
    assert!(canvas.blit(&tile, 1, 0).is_err());
    assert!(canvas.blit(&tile, 0, 1).is_err());
}
