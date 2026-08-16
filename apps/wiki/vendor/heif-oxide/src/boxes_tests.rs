//! Container-parser unit tests over in-memory files from `test_builder`.

use super::*;
use crate::test_builder as tb;

/// A minimal one-item file parses: pitm, iinf, iloc payload, properties.
#[test]
fn parses_minimal_single_item_file() {
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload: vec![0xAA; 10],
        props: vec![
            (true, tb::hvcc(&[(33, &[0x42, 0x01])], 4)),
            (false, tb::ispe(640, 480)),
        ],
    }];
    let file = tb::make_heic(&items, 1, &[]);
    let container = parse(&file).expect("parse");
    assert_eq!(container.primary_item, 1);
    assert_eq!(container.items[&1].item_type, *b"hvc1");
    assert_eq!(container.item_payload(1).unwrap(), vec![0xAA; 10]);

    let props: Vec<_> = container.item_properties(1).collect();
    assert_eq!(props.len(), 2);
    // hvcC marked essential, ispe not.
    assert!(props[0].0);
    assert!(matches!(props[0].1, Property::HvcC(_)));
    assert!(!props[1].0);
    assert!(matches!(
        props[1].1,
        Property::Ispe {
            width: 640,
            height: 480
        }
    ));
}

/// Multiple items get distinct payload ranges, and iref entries round-trip.
#[test]
fn parses_multiple_items_and_references() {
    let items = [
        tb::TestItem {
            id: 1,
            item_type: *b"grid",
            payload: tb::grid_payload(1, 2, 100, 50),
            props: vec![(false, tb::ispe(100, 50))],
        },
        tb::TestItem {
            id: 2,
            item_type: *b"hvc1",
            payload: vec![0x11; 5],
            props: vec![],
        },
        tb::TestItem {
            id: 3,
            item_type: *b"hvc1",
            payload: vec![0x22; 7],
            props: vec![],
        },
    ];
    let refs = [(*b"dimg", 1u32, vec![2u32, 3])];
    let file = tb::make_heic(&items, 1, &refs);
    let container = parse(&file).expect("parse");
    assert_eq!(container.item_payload(2).unwrap(), vec![0x11; 5]);
    assert_eq!(container.item_payload(3).unwrap(), vec![0x22; 7]);
    assert_eq!(container.referenced_items(1, b"dimg"), vec![2, 3]);
    assert!(container.referenced_items(1, b"auxl").is_empty());
    // Grid payload itself comes back verbatim.
    assert_eq!(container.item_payload(1).unwrap()[2], 0); // rows - 1
    assert_eq!(container.item_payload(1).unwrap()[3], 1); // cols - 1
}

/// The colr/irot/imir/clap property parsers extract the right fields.
#[test]
fn parses_transform_and_color_properties() {
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload: vec![0],
        props: vec![
            (false, tb::colr_nclx(12, 13, 6, true)),
            (true, tb::irot(3)),
            (true, tb::imir(1)),
        ],
    }];
    let file = tb::make_heic(&items, 1, &[]);
    let container = parse(&file).expect("parse");
    let props: Vec<_> = container
        .item_properties(1)
        .map(|(_, p)| p.clone())
        .collect();
    assert!(matches!(
        props[0],
        Property::Colr(ColorProperty::Nclx {
            primaries: 12,
            transfer: 13,
            matrix: 6,
            full_range: true
        })
    ));
    assert!(matches!(props[1], Property::Irot { angle: 3 }));
    assert!(matches!(props[2], Property::Imir { axis: 1 }));
}

/// A file with no accepted brand is rejected up front. (Real AVIF files
/// usually also list `mif1`, which we accept structurally — those instead
/// fail at decode time with `UnsupportedCodec("av01")`, a better error.)
#[test]
fn rejects_avif_brand() {
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"av01",
        payload: vec![0],
        props: vec![],
    }];
    let file = tb::make_heic_with_brand(&items, 1, &[], b"avif");
    match parse(&file) {
        Err(HeifError::NotHeif(msg)) => assert!(msg.contains("avif"), "{msg}"),
        other => panic!("expected NotHeif, got {other:?}", other = other.err()),
    }
}

/// Arbitrary bytes are rejected, never panicked on.
#[test]
fn rejects_garbage_input() {
    assert!(parse(b"this is not a heif file at all").is_err());
    assert!(parse(&[]).is_err());
    // A valid-looking box header whose size overruns the buffer.
    let mut file = Vec::new();
    file.extend_from_slice(&1000u32.to_be_bytes());
    file.extend_from_slice(b"ftyp");
    assert!(matches!(parse(&file), Err(HeifError::Truncated(_))));
}

/// Truncating a valid file mid-meta errors instead of panicking, wherever
/// the cut lands.
#[test]
fn truncation_never_panics() {
    let items = [tb::TestItem {
        id: 1,
        item_type: *b"hvc1",
        payload: vec![0xAB; 32],
        props: vec![
            (true, tb::hvcc(&[(33, &[0x42])], 4)),
            (false, tb::ispe(8, 8)),
        ],
    }];
    let file = tb::make_heic(&items, 1, &[]);
    for len in 0..file.len() {
        // Either an error or (once meta is complete) a successful parse —
        // the point is no slice panics on any prefix.
        let _ = parse(&file[..len]);
    }
}

/// iloc construction_method 1 reads payload bytes out of `idat`.
#[test]
fn reads_idat_payloads() {
    // Hand-build: meta { pitm, iinf(1 item), iloc(v1, method 1), idat }.
    let pitm = tb::full_box(b"pitm", 0, 0, &1u16.to_be_bytes());
    let mut infe_body = Vec::new();
    infe_body.extend_from_slice(&1u16.to_be_bytes());
    infe_body.extend_from_slice(&0u16.to_be_bytes());
    infe_body.extend_from_slice(b"hvc1");
    infe_body.push(0);
    let mut iinf_body = 1u16.to_be_bytes().to_vec();
    iinf_body.extend_from_slice(&tb::full_box(b"infe", 2, 0, &infe_body));
    let iinf = tb::full_box(b"iinf", 0, 0, &iinf_body);
    // iloc v1: offset_size 4 | length_size 4, base_offset_size 0 | index_size 0
    let mut iloc_body = vec![0x44, 0x00];
    iloc_body.extend_from_slice(&1u16.to_be_bytes()); // item_count
    iloc_body.extend_from_slice(&1u16.to_be_bytes()); // item_id
    iloc_body.extend_from_slice(&1u16.to_be_bytes()); // construction_method 1
    iloc_body.extend_from_slice(&0u16.to_be_bytes()); // data_reference_index
    iloc_body.extend_from_slice(&1u16.to_be_bytes()); // extent_count
    iloc_body.extend_from_slice(&2u32.to_be_bytes()); // extent_offset (into idat)
    iloc_body.extend_from_slice(&3u32.to_be_bytes()); // extent_length
    let iloc = tb::full_box(b"iloc", 1, 0, &iloc_body);
    let idat = tb::plain_box(b"idat", &[9, 9, 7, 7, 7, 9]);

    let mut meta_body = pitm;
    meta_body.extend_from_slice(&iinf);
    meta_body.extend_from_slice(&iloc);
    meta_body.extend_from_slice(&idat);
    let meta = tb::full_box(b"meta", 0, 0, &meta_body);

    let mut ftyp_body = b"heic".to_vec();
    ftyp_body.extend_from_slice(&0u32.to_be_bytes());
    let ftyp = tb::plain_box(b"ftyp", &ftyp_body);

    let mut file = ftyp;
    file.extend_from_slice(&meta);
    let container = parse(&file).expect("parse");
    assert_eq!(container.item_payload(1).unwrap(), vec![7, 7, 7]);
}
