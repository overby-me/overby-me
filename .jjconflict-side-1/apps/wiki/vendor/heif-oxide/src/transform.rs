//! Transformative item properties: clean-aperture crop, rotation, mirror.
//!
//! HEIF stores display orientation as container-level properties rather than
//! baking it into the codestream: `clap` (crop), `irot` (rotation in 90°
//! steps, anti-clockwise) and `imir` (mirror). They must be applied **in the
//! order the `ipma` box associates them with the item** (ISO 23008-12
//! §6.5.1) — iPhones record physical camera orientation this way, so
//! skipping or reordering them ships sideways photos.
//!
//! All functions operate on interleaved buffers generically (`u8` or `u16`
//! samples, any channel count), returning new buffers plus the transformed
//! dimensions.

use crate::boxes::Property;
use crate::error::HeifError;

/// Crop to the clean aperture. The spec defines fractional sizes/offsets
/// with the aperture centred on the image centre; offsets shift it.
/// Fractions are rounded to the nearest pixel (Apple writes integral ones).
#[allow(clippy::too_many_arguments)]
fn clean_aperture<T: Copy>(
    pixels: &[T],
    width: usize,
    height: usize,
    channels: usize,
    width_n: u32,
    width_d: u32,
    height_n: u32,
    height_d: u32,
    horiz_off_n: i32,
    horiz_off_d: u32,
    vert_off_n: i32,
    vert_off_d: u32,
) -> Result<(Vec<T>, usize, usize), HeifError> {
    let frac = |n: i64, d: u32| -> f64 {
        if d == 0 {
            0.0
        } else {
            n as f64 / d as f64
        }
    };
    let out_w = frac(width_n as i64, width_d).round() as i64;
    let out_h = frac(height_n as i64, height_d).round() as i64;
    if out_w <= 0 || out_h <= 0 || out_w as usize > width || out_h as usize > height {
        return Err(HeifError::Invalid(format!(
            "clean aperture {out_w}x{out_h} outside image {width}x{height}"
        )));
    }
    // Per ISO 14496-12: the aperture centre is the image centre displaced
    // by the offset; convert to a top-left corner.
    let centre_x = (width as f64 - 1.0) / 2.0 + frac(horiz_off_n as i64, horiz_off_d);
    let centre_y = (height as f64 - 1.0) / 2.0 + frac(vert_off_n as i64, vert_off_d);
    let left = (centre_x - (out_w as f64 - 1.0) / 2.0).round() as i64;
    let top = (centre_y - (out_h as f64 - 1.0) / 2.0).round() as i64;
    if left < 0 || top < 0 || left + out_w > width as i64 || top + out_h > height as i64 {
        return Err(HeifError::Invalid(
            "clean aperture extends outside the image".into(),
        ));
    }
    let (out_w, out_h, left, top) = (out_w as usize, out_h as usize, left as usize, top as usize);
    let mut out = Vec::with_capacity(out_w * out_h * channels);
    for row in 0..out_h {
        let start = ((top + row) * width + left) * channels;
        out.extend_from_slice(&pixels[start..start + out_w * channels]);
    }
    Ok((out, out_w, out_h))
}

/// Rotate anti-clockwise by `angle * 90` degrees.
fn rotate<T: Copy + Default>(
    pixels: &[T],
    width: usize,
    height: usize,
    channels: usize,
    angle: u8,
) -> (Vec<T>, usize, usize) {
    if angle == 0 {
        return (pixels.to_vec(), width, height);
    }
    let (out_w, out_h) = if angle % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    };
    let mut out = vec![T::default(); pixels.len()];
    for y in 0..height {
        for x in 0..width {
            // Destination of source pixel (x, y) for each CCW angle.
            let (dx, dy) = match angle {
                1 => (y, width - 1 - x),              // 90° CCW
                2 => (width - 1 - x, height - 1 - y), // 180°
                _ => (height - 1 - y, x),             // 270° CCW (= 90° CW)
            };
            let src = (y * width + x) * channels;
            let dst = (dy * out_w + dx) * channels;
            out[dst..dst + channels].copy_from_slice(&pixels[src..src + channels]);
        }
    }
    (out, out_w, out_h)
}

/// Mirror about a vertical axis (axis 0: left↔right) or horizontal axis
/// (axis 1: top↔bottom).
fn mirror<T: Copy>(pixels: &[T], width: usize, height: usize, channels: usize, axis: u8) -> Vec<T> {
    let mut out = pixels.to_vec();
    if axis == 0 {
        for y in 0..height {
            for x in 0..width {
                let src = (y * width + (width - 1 - x)) * channels;
                let dst = (y * width + x) * channels;
                out[dst..dst + channels].copy_from_slice(&pixels[src..src + channels]);
            }
        }
    } else {
        for y in 0..height {
            let src = (height - 1 - y) * width * channels;
            let dst = y * width * channels;
            out[dst..dst + width * channels].copy_from_slice(&pixels[src..src + width * channels]);
        }
    }
    out
}

/// Crop to the top-left `new_w x new_h` region — how a grid canvas (a
/// whole number of tiles) is reduced to the grid's declared output size.
pub fn crop_top_left<T: Copy>(
    pixels: &[T],
    width: usize,
    channels: usize,
    new_w: usize,
    new_h: usize,
) -> Vec<T> {
    if new_w == width {
        return pixels[..new_w * new_h * channels].to_vec();
    }
    let mut out = Vec::with_capacity(new_w * new_h * channels);
    for row in 0..new_h {
        let start = row * width * channels;
        out.extend_from_slice(&pixels[start..start + new_w * channels]);
    }
    out
}

/// Apply an item's transformative properties, in association order, to an
/// interleaved buffer. Returns the transformed buffer and dimensions.
pub fn apply_transforms<T: Copy + Default>(
    mut pixels: Vec<T>,
    mut width: usize,
    mut height: usize,
    channels: usize,
    transforms: &[Property],
) -> Result<(Vec<T>, usize, usize), HeifError> {
    for property in transforms {
        match *property {
            Property::Clap {
                width_n,
                width_d,
                height_n,
                height_d,
                horiz_off_n,
                horiz_off_d,
                vert_off_n,
                vert_off_d,
            } => {
                let (p, w, h) = clean_aperture(
                    &pixels,
                    width,
                    height,
                    channels,
                    width_n,
                    width_d,
                    height_n,
                    height_d,
                    horiz_off_n,
                    horiz_off_d,
                    vert_off_n,
                    vert_off_d,
                )?;
                pixels = p;
                width = w;
                height = h;
            }
            Property::Irot { angle } => {
                let (p, w, h) = rotate(&pixels, width, height, channels, angle);
                pixels = p;
                width = w;
                height = h;
            }
            Property::Imir { axis } => {
                pixels = mirror(&pixels, width, height, channels, axis);
            }
            // Non-transformative properties are filtered out by the caller.
            _ => {}
        }
    }
    Ok((pixels, width, height))
}

#[cfg(test)]
#[path = "transform_tests.rs"]
mod tests;
