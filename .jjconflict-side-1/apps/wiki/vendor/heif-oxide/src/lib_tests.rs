//! End-to-end decode tests: real x265-encoded codestreams (committed under
//! `testdata/`, generated losslessly from flat frames with exactly known
//! plane values) wrapped into HEIC containers by `test_builder`.

use super::*;
use crate::test_builder as tb;

/// Load a committed Annex B fixture and split it into (hvcC box, payload).
fn fixture_item(name: &str) -> (Vec<u8>, Vec<u8>) {
    let path = format!("{}/testdata/{name}.h265", env!("CARGO_MANIFEST_DIR"));
    let annex_b = std::fs::read(path).expect("read fixture");
    tb::annex_b_to_item(&annex_b)
}

/// BT.601 limited-range nclx — matches how the flat fixtures were authored.
fn nclx_601() -> Vec<u8> {
    tb::colr_nclx(1, 13, 6, false)
}

fn assert_flat_rgb8(image: &DecodedImage, expected: [u8; 3], tolerance: i16) {
    let Pixels::Rgb8(buf) = &image.pixels else {
        panic!("expected Rgb8 pixels");
    };
    assert_eq!(buf.len(), (image.width * image.height * 3) as usize);
    for px in buf.chunks_exact(3) {
        for c in 0..3 {
            assert!(
                (px[c] as i16 - expected[c] as i16).abs() <= tolerance,
                "pixel {:?} != {:?}",
                &px[..3],
                expected
            );
        }
    }
}

/// Build a single-`hvc1` HEIC around a fixture codestream.
fn single_item_heic(fixture: &str) -> Vec<u8> {
    let (hvcc, payload) = fixture_item(fixture);
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload,
        props: vec![(true, hvcc), (false, tb::ispe(64, 64)), (false, nclx_601())],
    }];
    tb::make_heic(&items, 1, &[])
}

/// Build a 1x2 grid HEIC (left/right fixture tiles) with `extra_grid_props`
/// appended to the grid item's property list.
fn grid_heic(
    left: &str,
    right: &str,
    out_w: u16,
    out_h: u16,
    extra_grid_props: Vec<(bool, Vec<u8>)>,
) -> Vec<u8> {
    let (hvcc_l, payload_l) = fixture_item(left);
    let (hvcc_r, payload_r) = fixture_item(right);
    let mut grid_props = vec![
        (false, tb::ispe(out_w as u32, out_h as u32)),
        (false, nclx_601()),
    ];
    grid_props.extend(extra_grid_props);
    let items = [
        tb::TestItem {
            id: 1,
            item_type: *b"grid",
            payload: tb::grid_payload(1, 2, out_w, out_h),
            props: grid_props,
        },
        tb::TestItem {
            id: 2,
            item_type: *b"hvc1",
            payload: payload_l,
            props: vec![(true, hvcc_l), (false, tb::ispe(64, 64))],
        },
        tb::TestItem {
            id: 3,
            item_type: *b"hvc1",
            payload: payload_r,
            props: vec![(true, hvcc_r), (false, tb::ispe(64, 64))],
        },
    ];
    tb::make_heic(&items, 1, &[(*b"dimg", 1, vec![2, 3])])
}

/// Sample a pixel from an Rgb8 image.
fn px(image: &DecodedImage, x: u32, y: u32) -> [u8; 3] {
    let Pixels::Rgb8(buf) = &image.pixels else {
        panic!("expected Rgb8");
    };
    let i = ((y * image.width + x) * 3) as usize;
    [buf[i], buf[i + 1], buf[i + 2]]
}

fn is_reddish(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 50 && p[2] < 50
}

fn is_bluish(p: [u8; 3]) -> bool {
    p[2] > 200 && p[0] < 50 && p[1] < 50
}

/// A single flat-red picture decodes to the hand-computed sRGB value.
#[test]
fn decodes_single_hvc1_item() {
    let image = decode_bytes(&single_item_heic("flat_red_64")).expect("decode");
    assert_eq!((image.width, image.height), (64, 64));
    assert_eq!(image.channels(), 3);
    assert_flat_rgb8(&image, [254, 0, 0], 1);
    // The colr box was found and reported.
    let nclx = image.color.nclx.expect("nclx");
    assert_eq!(nclx.matrix, 6);
    assert!(!nclx.full_range);
}

/// Grid assembly: red tile left, blue tile right, cropped from the 128x64
/// canvas to a declared 100x50 output.
#[test]
fn decodes_grid_with_crop() {
    let file = grid_heic("flat_red_64", "flat_blue_64", 100, 50, vec![]);
    let image = decode_bytes(&file).expect("decode");
    assert_eq!((image.width, image.height), (100, 50));
    // Probe just inside each tile — the boundary column itself legitimately
    // blends chroma from both tiles (bilinear 4:2:0 upsampling).
    assert!(is_reddish(px(&image, 10, 25)), "{:?}", px(&image, 10, 25));
    assert!(is_reddish(px(&image, 60, 25)), "{:?}", px(&image, 60, 25));
    assert!(is_bluish(px(&image, 68, 25)), "{:?}", px(&image, 68, 25));
    assert!(is_bluish(px(&image, 99, 49)), "{:?}", px(&image, 99, 49));
}

/// irot on the grid item rotates the assembled image: after 90° CCW the
/// right (blue) half becomes the top half.
#[test]
fn applies_grid_rotation() {
    let file = grid_heic(
        "flat_red_64",
        "flat_blue_64",
        100,
        50,
        vec![(true, tb::irot(1))],
    );
    let image = decode_bytes(&file).expect("decode");
    assert_eq!((image.width, image.height), (50, 100));
    assert!(is_bluish(px(&image, 25, 5)), "{:?}", px(&image, 25, 5));
    assert!(is_reddish(px(&image, 25, 95)), "{:?}", px(&image, 25, 95));
}

/// imir mirrors the assembled image left↔right.
#[test]
fn applies_grid_mirror() {
    let file = grid_heic(
        "flat_red_64",
        "flat_blue_64",
        100,
        50,
        vec![(true, tb::imir(0))],
    );
    let image = decode_bytes(&file).expect("decode");
    assert!(is_bluish(px(&image, 5, 25)), "{:?}", px(&image, 5, 25));
    assert!(is_reddish(px(&image, 95, 25)), "{:?}", px(&image, 95, 25));
}

/// A 10-bit source comes out as full-scale Rgb16.
#[test]
fn decodes_ten_bit_to_rgb16() {
    let (hvcc, payload) = fixture_item("flat_mid_64_10bit");
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload,
        props: vec![(true, hvcc), (false, nclx_601())],
    }];
    let image = decode_bytes(&tb::make_heic(&items, 1, &[])).expect("decode");
    assert_eq!(image.bit_depth(), 16);
    let Pixels::Rgb16(buf) = &image.pixels else {
        panic!("expected Rgb16");
    };
    // Limited-range 10-bit Y=512 → (512-64)/876 → 33516 at u16 scale.
    for &s in buf {
        assert!((s as i32 - 33516).abs() <= 2, "sample {s}");
    }
}

/// A 4:2:0-coded alpha auxiliary is decoded and interleaved.
#[test]
fn decodes_alpha_auxiliary() {
    let (hvcc_main, payload_main) = fixture_item("flat_red_64");
    let (hvcc_alpha, payload_alpha) = fixture_item("flat_gray_64");
    let items = [
        tb::TestItem {
            id: 1,
            item_type: *b"hvc1",
            payload: payload_main,
            props: vec![(true, hvcc_main), (false, nclx_601())],
        },
        tb::TestItem {
            id: 2,
            item_type: *b"hvc1",
            payload: payload_alpha,
            props: vec![
                (true, hvcc_alpha),
                (
                    false,
                    tb::full_box(b"auxC", 0, 0, b"urn:mpeg:hevc:2015:auxid:1\0"),
                ),
            ],
        },
    ];
    let file = tb::make_heic(&items, 1, &[(*b"auxl", 2, vec![1])]);
    let image = decode_bytes(&file).expect("decode");
    assert_eq!(image.channels(), 4);
    let Pixels::Rgba8(buf) = &image.pixels else {
        panic!("expected Rgba8");
    };
    // Alpha = luma 128 read full-range → 128; colour channels still red.
    for p in buf.chunks_exact(4) {
        assert!((p[0] as i16 - 254).abs() <= 1 && p[1] <= 1 && p[2] <= 1);
        assert!((p[3] as i16 - 128).abs() <= 1, "alpha {}", p[3]);
    }
}

/// Non-HEVC primary items are rejected with the codec named.
#[test]
fn rejects_av01_item() {
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"av01",
        payload: vec![0; 16],
        props: vec![],
    }];
    match decode_bytes(&tb::make_heic(&items, 1, &[])) {
        Err(HeifError::UnsupportedCodec(codec)) => assert_eq!(codec, "av01"),
        other => panic!("expected UnsupportedCodec, got {:?}", other.err()),
    }
}

/// Unknown *essential* properties abort the decode (spec-required).
#[test]
fn rejects_unknown_essential_property() {
    let (hvcc, payload) = fixture_item("flat_red_64");
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload,
        props: vec![(true, hvcc), (true, tb::plain_box(b"zzzz", &[1, 2, 3]))],
    }];
    match decode_bytes(&tb::make_heic(&items, 1, &[])) {
        Err(HeifError::Unsupported(msg)) => assert!(msg.contains("zzzz"), "{msg}"),
        other => panic!("expected Unsupported, got {:?}", other.err()),
    }
}

/// Corrupt HEVC payload bytes surface as a codec error, not a panic.
#[test]
fn corrupt_payload_is_an_error() {
    let (hvcc, mut payload) = fixture_item("flat_red_64");
    // Scramble the slice data (keep the first length prefix plausible).
    for b in payload.iter_mut().skip(8) {
        *b ^= 0xA5;
    }
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload,
        props: vec![(true, hvcc)],
    }];
    assert!(decode_bytes(&tb::make_heic(&items, 1, &[])).is_err());
}

/// Not a test: writes a small complete `.heic` (the flat-red codestream in
/// a single-item container) to `testdata/` for downstream consumers that
/// want a committed end-to-end fixture. Run explicitly with
/// `cargo test generate_sample_heic -- --ignored`.
#[test]
#[ignore = "fixture generator, writes into testdata/"]
fn generate_sample_heic() {
    let file = single_item_heic("flat_red_64");
    decode_bytes(&file).expect("generated fixture must decode");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/flat_red_64.heic");
    std::fs::write(path, file).expect("write fixture");
}

/// `to_f32_interleaved` and `to_rgba8` round sensibly.
#[test]
fn conversion_helpers() {
    let image = decode_bytes(&single_item_heic("flat_gray_64")).expect("decode");
    let f = image.to_f32_interleaved();
    assert_eq!(f.len(), 64 * 64 * 3);
    assert!((f[0] - 130.0 / 255.0).abs() < 0.01);
    let rgba = image.to_rgba8();
    assert_eq!(rgba.len(), 64 * 64 * 4);
    assert_eq!(rgba[3], 255);
}
