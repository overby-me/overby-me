//! Known-answer and round-trip vectors for the DAG-CBOR + CIDv1 encode path.
//!
//! Known-answer sources: the DAG-CBOR spec's canonical-form rules (RFC 8949
//! core deterministic encoding: definite lengths, length-first-then-bytewise
//! map key ordering, shortest-form integers) with byte vectors checkable by
//! hand against the CBOR spec's Appendix A examples, plus the IPLD fixture
//! convention for CIDs (CIDv1, dag-cbor 0x71, sha2-256 0x12).

use dagcbor_spike::*;
use std::collections::BTreeMap;

#[test]
fn map_keys_sort_deterministically() {
    // {"b": 2, "a": 1} must encode with "a" FIRST regardless of insertion
    // order (canonical map ordering), and identically to {"a": 1, "b": 2}.
    let mut m1 = BTreeMap::new();
    m1.insert("b".to_string(), 2u64);
    m1.insert("a".to_string(), 1u64);
    let bytes = encode(&m1).expect("encode");
    // Hand-checkable: a2 (map, 2 entries) 61 61 (text "a") 01, 61 62 (text "b") 02.
    assert_eq!(hex::encode(&bytes), "a2616101616202");
    // Serde_json::Value with reversed insertion produces identical bytes.
    let v: serde_json::Value = serde_json::json!({"b": 2, "a": 1});
    let bytes2 = encode(&v).expect("encode json value");
    assert_eq!(bytes, bytes2, "encoding is insertion-order independent");
}

#[test]
fn integers_use_shortest_form() {
    // Canonical CBOR: integers take their shortest representation.
    assert_eq!(hex::encode(encode(&0u64).unwrap()), "00");
    assert_eq!(hex::encode(encode(&23u64).unwrap()), "17");
    assert_eq!(hex::encode(encode(&24u64).unwrap()), "1818");
    assert_eq!(hex::encode(encode(&255u64).unwrap()), "18ff");
    assert_eq!(hex::encode(encode(&256u64).unwrap()), "190100");
    assert_eq!(hex::encode(encode(&(-1i64)).unwrap()), "20");
}

#[test]
fn known_answer_cid_for_fixed_bytes() {
    // The CID of a FIXED encoding must never drift: pin the whole pipeline
    // (encode -> sha2-256 -> multihash 0x12 -> CIDv1 0x71 -> base32 "b...").
    let v: serde_json::Value = serde_json::json!({"hello": "world"});
    let bytes = encode(&v).expect("encode");
    assert_eq!(hex::encode(&bytes), "a16568656c6c6f65776f726c64");
    let cid = cid_of(&bytes);
    assert_eq!(cid.codec(), DAG_CBOR_CODEC);
    assert_eq!(cid.hash().code(), SHA2_256);
    // Known answer, reproducible with any IPLD tool:
    // sha2-256(a16568656c6c6f65776f726c64) under CIDv1/dag-cbor.
    assert_eq!(
        cid.to_string(),
        "bafyreidykglsfhoixmivffc5uwhcgshx4j465xwqntbmu43nb2dzqwfvae"
    );
}

#[test]
fn wiki_post_round_trips_and_addresses_stably() {
    // The real com.example.wiki.post record: encode, address, decode, and the
    // re-encode of the decode is byte-identical (determinism), so the CID is
    // stable across a round trip.
    let post = WikiPost {
        record_type: WikiPost::NSID.into(),
        text: "Hej verden".into(),
        group: None,
        reply: None,
        created_at: "2026-07-16T12:00:00.000Z".into(),
    };
    let bytes = encode(&post).expect("encode");
    let cid1 = cid_of(&bytes);

    let back: WikiPost = decode(&bytes).expect("decode");
    assert_eq!(back, post);

    let bytes2 = encode(&back).expect("re-encode");
    assert_eq!(bytes, bytes2, "deterministic re-encode");
    assert_eq!(cid_of(&bytes2), cid1, "stable CID across round trip");

    // The optional fields are OMITTED when None (atproto convention), not null.
    assert!(
        !hex::encode(&bytes).contains(hex::encode("reply").as_str()),
        "absent optional field is omitted from the encoding"
    );
    assert!(
        !hex::encode(&bytes).contains(hex::encode("group").as_str()),
        "absent optional field is omitted from the encoding"
    );
}

#[test]
fn struct_encoding_is_canonically_key_sorted() {
    // The load-bearing property for a correct CID: a struct encodes IDENTICALLY
    // to the equivalent map (serde_json::Value), i.e. `serde_ipld_dagcbor` sorts
    // struct field keys canonically (length-first, then bytewise), NOT in
    // declaration order. If this ever regresses, every record CID drifts and the
    // network rejects the bytes.
    let post = WikiPost {
        record_type: WikiPost::NSID.into(),
        text: "Hej".into(),
        group: Some("at://did:plc:org/com.example.wiki.group/abc".into()),
        reply: Some(StrongRef {
            uri: "at://did:plc:alice/com.example.wiki.post/xyz".into(),
            cid: "bafyreidykglsfhoixmivffc5uwhcgshx4j465xwqntbmu43nb2dzqwfvae".into(),
        }),
        created_at: "2026-07-16T12:00:00.000Z".into(),
    };
    let struct_bytes = encode(&post).expect("encode struct");
    // The same content as an insertion-order-scrambled JSON map.
    let value: serde_json::Value = serde_json::json!({
        "createdAt": "2026-07-16T12:00:00.000Z",
        "$type": "com.example.wiki.post",
        "reply": {
            "cid": "bafyreidykglsfhoixmivffc5uwhcgshx4j465xwqntbmu43nb2dzqwfvae",
            "uri": "at://did:plc:alice/com.example.wiki.post/xyz",
        },
        "text": "Hej",
        "group": "at://did:plc:org/com.example.wiki.group/abc",
    });
    let value_bytes = encode(&value).expect("encode value");
    assert_eq!(
        struct_bytes, value_bytes,
        "struct fields must encode in canonical key order, not declaration order"
    );
}

#[test]
fn known_answer_cids_for_the_content_records() {
    // Regression pins for the three drafted content records under the placeholder
    // NSID. Each CID is sha2-256 of the canonical DAG-CBOR bytes under CIDv1/
    // dag-cbor (0x71) and reproducible with any IPLD tool from the fixed record.
    // A wrong CID means the network rejects or mis-addresses the published record.
    let post = WikiPost {
        record_type: WikiPost::NSID.into(),
        text: "Hej verden".into(),
        group: Some("at://did:plc:org/com.example.wiki.group/g1".into()),
        reply: None,
        created_at: "2026-07-16T12:00:00.000Z".into(),
    };
    assert_eq!(
        cid_of(&encode(&post).unwrap()).to_string(),
        "bafyreiea5wb5am57qvnczh4vjtzxyxz7pujpg4hzbnygeliyfgyc2v4yty"
    );

    let comment = WikiComment {
        record_type: WikiComment::NSID.into(),
        subject: StrongRef {
            uri: "at://did:plc:org/com.example.wiki.resolution/r1".into(),
            cid: "bafyreidykglsfhoixmivffc5uwhcgshx4j465xwqntbmu43nb2dzqwfvae".into(),
        },
        text: "Enig".into(),
        parent: None,
        created_at: "2026-07-16T12:00:00.000Z".into(),
    };
    assert_eq!(
        cid_of(&encode(&comment).unwrap()).to_string(),
        "bafyreicm5ffwmkpxjbyvx7t4nl7y7qx5kxxjvnt7ahwf36idtkcrkdbode"
    );

    let resolution = WikiResolution {
        record_type: WikiResolution::NSID.into(),
        title: "Vedtaegtsaendring".into(),
        body: Some("Foreningen vedtager...".into()),
        status: "carried".into(),
        context: None,
        created_at: "2026-07-16T12:00:00.000Z".into(),
    };
    assert_eq!(
        cid_of(&encode(&resolution).unwrap()).to_string(),
        "bafyreibaxzqht56gehvdjg4qxfiowyxtpbyzck4jiybs763tme25psqkuq"
    );

    let reaction = WikiReaction {
        record_type: WikiReaction::NSID.into(),
        subject: StrongRef {
            uri: "at://did:plc:org/com.example.wiki.comment/k1".into(),
            cid: "bafyreidykglsfhoixmivffc5uwhcgshx4j465xwqntbmu43nb2dzqwfvae".into(),
        },
        emoji: "👍".into(),
        created_at: "2026-07-16T12:00:00.000Z".into(),
    };
    assert_eq!(
        cid_of(&encode(&reaction).unwrap()).to_string(),
        "bafyreicutzq4x2j4redrmqdq4c6sgeat32x2plryw4x2zhgev7p4f5blnu"
    );

    // All round-trip through decode unchanged.
    assert_eq!(
        decode::<WikiComment>(&encode(&comment).unwrap()).unwrap(),
        comment
    );
    assert_eq!(
        decode::<WikiResolution>(&encode(&resolution).unwrap()).unwrap(),
        resolution
    );
    assert_eq!(
        decode::<WikiReaction>(&encode(&reaction).unwrap()).unwrap(),
        reaction
    );
}

#[test]
fn floats_are_rejected_by_policy_and_representable_ints_are_not_floats() {
    // atproto bans floats in records; our lexicon-derived models use integers
    // and strings only, so the ban is STRUCTURAL (no f64 fields exist). This
    // vector documents the wire-level difference so a float can never sneak
    // in via a generic Value: 1.0 as a float encodes with major type 7,
    // while integer 1 encodes as 01.
    let int_bytes = encode(&1u64).unwrap();
    assert_eq!(hex::encode(&int_bytes), "01");
    let float_val: serde_json::Value = serde_json::json!(1.5);
    let float_bytes = encode(&float_val).unwrap();
    assert_ne!(int_bytes, float_bytes);
    assert!(
        float_bytes[0] >> 5 == 7,
        "a float encodes under major type 7: caught by review/validation, \
         never emitted by the integer-only record types"
    );
}

#[test]
fn nested_structures_round_trip() {
    // Deep nesting (arrays of maps, as poll options / facets will be).
    let v: serde_json::Value = serde_json::json!({
        "$type": "com.example.wiki.poll",
        "question": "Farve?",
        "options": ["Roed", "Groen", "Blank"],
        "minVote": 1,
        "maxVote": 1,
        "blank": true,
    });
    let bytes = encode(&v).expect("encode");
    let back: serde_json::Value = decode(&bytes).expect("decode");
    assert_eq!(back, v);
    let cid = cid_of(&bytes);
    assert_eq!(cid.codec(), DAG_CBOR_CODEC);
    assert!(cid.to_string().starts_with('b'), "CIDv1 base32 form");
}
