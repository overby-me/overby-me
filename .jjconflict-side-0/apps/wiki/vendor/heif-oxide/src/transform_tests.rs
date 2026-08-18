//! Orientation transform tests on tiny labeled buffers.

use super::*;
use crate::boxes::Property;

/// Apply one property to a single-channel buffer.
fn apply1(pixels: &[u8], w: usize, h: usize, p: Property) -> (Vec<u8>, usize, usize) {
    apply_transforms(pixels.to_vec(), w, h, 1, &[p]).unwrap()
}

#[test]
fn rotate_90_ccw() {
    // 2x1 image [A, B]: A at x=0, B at x=1. After 90° CCW the right edge
    // becomes the top: expect column [B, A].
    let (out, w, h) = apply1(&[1, 2], 2, 1, Property::Irot { angle: 1 });
    assert_eq!((w, h), (1, 2));
    assert_eq!(out, vec![2, 1]);
}

#[test]
fn rotate_180() {
    let (out, w, h) = apply1(&[1, 2, 3, 4], 2, 2, Property::Irot { angle: 2 });
    assert_eq!((w, h), (2, 2));
    assert_eq!(out, vec![4, 3, 2, 1]);
}

#[test]
fn rotate_270_ccw() {
    // 2x1 [A, B] rotated 270° CCW (= 90° CW): left edge becomes the top.
    let (out, w, h) = apply1(&[1, 2], 2, 1, Property::Irot { angle: 3 });
    assert_eq!((w, h), (1, 2));
    assert_eq!(out, vec![1, 2]);
}

#[test]
fn rotations_compose_to_identity() {
    let src = vec![1u8, 2, 3, 4, 5, 6];
    let (out, w, h) = apply_transforms(
        src.clone(),
        3,
        2,
        1,
        &[Property::Irot { angle: 1 }, Property::Irot { angle: 3 }],
    )
    .unwrap();
    assert_eq!((w, h), (3, 2));
    assert_eq!(out, src);
}

#[test]
fn mirror_axes() {
    // axis 0: about a vertical axis → left↔right.
    let (out, ..) = apply1(&[1, 2, 3, 4], 2, 2, Property::Imir { axis: 0 });
    assert_eq!(out, vec![2, 1, 4, 3]);
    // axis 1: about a horizontal axis → top↔bottom.
    let (out, ..) = apply1(&[1, 2, 3, 4], 2, 2, Property::Imir { axis: 1 });
    assert_eq!(out, vec![3, 4, 1, 2]);
}

#[test]
fn transform_order_matters() {
    // Rotate-then-mirror differs from mirror-then-rotate on an asymmetric
    // image — exactly why ipma association order must be preserved.
    let src = vec![1u8, 2, 3, 4];
    let rot_then_mir = apply_transforms(
        src.clone(),
        2,
        2,
        1,
        &[Property::Irot { angle: 1 }, Property::Imir { axis: 0 }],
    )
    .unwrap()
    .0;
    let mir_then_rot = apply_transforms(
        src,
        2,
        2,
        1,
        &[Property::Imir { axis: 0 }, Property::Irot { angle: 1 }],
    )
    .unwrap()
    .0;
    assert_ne!(rot_then_mir, mir_then_rot);
}

#[test]
fn clean_aperture_centred_crop() {
    // 4x4 numbered 0..15; a centred 2x2 aperture with zero offset picks the
    // middle four samples.
    let src: Vec<u8> = (0..16).collect();
    let (out, w, h) = apply1(
        &src,
        4,
        4,
        Property::Clap {
            width_n: 2,
            width_d: 1,
            height_n: 2,
            height_d: 1,
            horiz_off_n: 0,
            horiz_off_d: 1,
            vert_off_n: 0,
            vert_off_d: 1,
        },
    );
    assert_eq!((w, h), (2, 2));
    assert_eq!(out, vec![5, 6, 9, 10]);
}

#[test]
fn clean_aperture_rejects_oversize() {
    let src = vec![0u8; 4];
    let result = apply_transforms(
        src,
        2,
        2,
        1,
        &[Property::Clap {
            width_n: 5,
            width_d: 1,
            height_n: 2,
            height_d: 1,
            horiz_off_n: 0,
            horiz_off_d: 1,
            vert_off_n: 0,
            vert_off_d: 1,
        }],
    );
    assert!(result.is_err());
}

#[test]
fn crop_top_left_basic() {
    let src: Vec<u8> = (0..12).collect(); // 4x3
    assert_eq!(crop_top_left(&src, 4, 1, 2, 2), vec![0, 1, 4, 5]);
    // Full-width crop takes the fast path.
    assert_eq!(crop_top_left(&src, 4, 1, 4, 2), (0..8).collect::<Vec<u8>>());
}

#[test]
fn multichannel_pixels_stay_together() {
    // 2x1 RGB pixels: red then blue; mirroring must swap whole pixels.
    let src = vec![255u8, 0, 0, 0, 0, 255];
    let (out, ..) = apply_transforms(src, 2, 1, 3, &[Property::Imir { axis: 0 }]).unwrap();
    assert_eq!(out, vec![0, 0, 255, 255, 0, 0]);
}
