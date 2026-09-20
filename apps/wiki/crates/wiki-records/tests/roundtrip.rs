//! A row written through a record comes back as it went, and a record is what
//! its lexicon says: the two promises a rebuild of the index stands on.

use serde_json::{Value, json};
use std::collections::BTreeSet;
use wiki_domain_types as rows;
use wiki_records::*;

const ORG: &str = "did:plc:theorganization";

struct Held;

impl Addresses for Held {
    fn space(&self, context_id: &str) -> SpaceUri {
        SpaceUri {
            authority: ORG.into(),
            space_type: CONTEXT_SPACE.into(),
            skey: context_id.into(),
        }
    }
    fn holder(&self, _id: &str) -> String {
        ORG.into()
    }
    fn cid(&self, _collection: &str, id: &str) -> Option<String> {
        (id != "no-record-yet").then(|| format!("bafy-{id}"))
    }
}

fn place(slug: &str, path: &str, parent: &str) -> rows::Place {
    rows::Place {
        slug: slug.into(),
        path: path.into(),
        parent_id: Some(parent.into()),
        idx: 3,
        attachable: false,
        owner_did: Some("did:plc:alice".into()),
        created_at: Some("2026-09-01T10:00:00.000Z".into()),
        updated_at: Some("2026-09-02T10:00:00.000Z".into()),
        deleted_at: Some("2026-09-03T10:00:00.000Z".into()),
        deleted_root: Some("d-folder".into()),
    }
}

fn motion() -> rows::Document {
    rows::Document {
        id: "d-motion".into(),
        context_id: "c-hb".into(),
        kind: rows::DocumentKind::Policy,
        title: "Kontingent".into(),
        place: place("kontingent", "hb/forslag/kontingent", "d-folder"),
        mutable: true,
        content: Some(json!([{"type": "paragraph", "children": [{"text": "Vi foreslår"}]}])),
        data: Some(json!({"image": "file-1"})),
        authors: vec![
            rows::Author::User {
                did: "did:plc:alice".into(),
            },
            rows::Author::FreeText {
                display: "Aarhus".into(),
            },
            rows::Author::Context {
                context_id: "c-aarhus".into(),
                name: None,
                path: None,
            },
        ],
        visibility: rows::Visibility::Public,
        // Split, or the repository's link checker tries to parse it as a URL.
        published_uri: Some(
            concat!(
                "at:",
                "//did:plc:theorganization/wiki.radikal.resolution/3kq"
            )
            .into(),
        ),
        legacy_id: Some("0b7e".into()),
    }
}

/// What the AppView would carry over from the row a rebuild replaces.
fn kept(visibility: rows::Visibility, published_uri: &Option<String>) -> Kept {
    Kept {
        visibility,
        published_uri: published_uri.clone(),
    }
}

#[test]
fn a_document_comes_back_as_it_went() {
    let doc = motion();
    let record = Node::of(&doc, &Held);
    assert_eq!(
        record.parent.as_deref(),
        Some(
            "at://did:plc:theorganization/space/wiki.radikal.context/c-hb/did:plc:theorganization/wiki.radikal.node/d-folder"
        )
    );
    let uri = Held.space("c-hb").record(ORG, NODE, &doc.id);
    // Who it is open to and where it was published are not the record's to say.
    let said = serde_json::to_string(&record).expect("json");
    assert!(
        !said.contains("resolution/3kq") && !said.contains("public"),
        "{said}"
    );
    assert_eq!(
        record
            .row(&uri, "hb/forslag", Kept::default())
            .map(|d| d.visibility),
        Some(rows::Visibility::Private)
    );
    let theirs = kept(doc.visibility, &doc.published_uri);
    assert_eq!(record.row(&uri, "hb/forslag", theirs), Some(doc));
}

/// Whether any number in `json` is one a PDS refuses (found against the alpha:
/// a fraction is an `InvalidRequest`, an integer past 2^53 a server error).
fn has_a_number_a_pds_refuses(json: &Value) -> bool {
    match json {
        Value::Number(n) => n
            .as_i64()
            .or(n.as_u64().map(|u| u as i64))
            .is_none_or(|i| i.unsigned_abs() >= 1 << 53),
        Value::Array(items) => items.iter().any(has_a_number_a_pds_refuses),
        Value::Object(fields) => fields.values().any(has_a_number_a_pds_refuses),
        _ => false,
    }
}

#[test]
fn numbers_atproto_cannot_hold_go_in_their_own_digits_and_come_back() {
    let mut doc = motion();
    doc.content = Some(json!([{"type": "image", "width": 0.75, "children": [{"text": ""}]}]));
    doc.data = Some(json!({
        "threshold": 66.7, "whole": 2.0, "small": -1e-7, "seats": 12,
        "huge": 18446744073709551615u64, "low": -9007199254740993i64,
        "nested": [{"at": [0.5, 1, "0.5"]}],
    }));
    let record = Node::of(&doc, &Held);
    let said = serde_json::to_value(&record).expect("json");
    assert!(!has_a_number_a_pds_refuses(&said), "{said}");
    assert_eq!(
        said["data"]["threshold"],
        json!({"$type": "wiki.radikal.spaceDefs#number", "value": "66.7"})
    );
    assert_eq!(
        (&said["data"]["seats"], &said["data"]["nested"][0]["at"][2]),
        (&json!(12), &json!("0.5"))
    );

    let read: Node = serde_json::from_value(said).expect("a node");
    let uri = Held.space("c-hb").record(ORG, NODE, &doc.id);
    let theirs = kept(doc.visibility, &doc.published_uri);
    assert_eq!(read.row(&uri, "hb/forslag", theirs), Some(doc));
}

#[test]
fn where_a_node_is_follows_from_the_records_alone() {
    let folder = |id: &str, slug: &str, parent: &str| {
        let mut doc = motion();
        doc.id = id.into();
        doc.kind = rows::DocumentKind::Folder;
        doc.place = place(slug, "", parent);
        doc
    };
    let context = |id: &str, slug: &str, hanging: &Hanging| {
        let ctx = rows::Context {
            id: id.into(),
            kind: rows::ContextKind::Group,
            name: slug.into(),
            place: place(slug, "", ""),
            content: None,
            data: None,
            visibility: rows::Visibility::default(),
            published_uri: None,
            legacy_id: None,
        };
        ContextProfile::of(&ctx, hanging, &Held)
    };
    let mut found = Found::default();
    found
        .profiles
        .insert("c-hb".into(), context("c-hb", "hb", &Hanging::default()));
    // An event in a folder of the board's, and a motion in a folder of the event's.
    let in_moeder = Hanging {
        context_id: Some("c-hb".into()),
        folder_id: Some("d-moeder".into()),
    };
    found
        .profiles
        .insert("c-lm".into(), context("c-lm", "landsmoede", &in_moeder));
    for (ctx, doc) in [
        ("c-hb", folder("d-moeder", "moeder", "c-hb")),
        ("c-lm", folder("d-forslag", "forslag", "c-lm")),
        ("c-lm", folder("d-motion", "kontingent", "d-forslag")),
    ] {
        let mut doc = doc;
        doc.context_id = ctx.into();
        found
            .nodes
            .insert(doc.id.clone(), (ctx.into(), Node::of(&doc, &Held)));
    }
    assert_eq!(found.context_parent_path("c-hb").as_deref(), Some(""));
    assert_eq!(
        found.context_parent_path("c-lm").as_deref(),
        Some("hb/moeder")
    );
    assert_eq!(
        found.node_parent_path("d-motion").as_deref(),
        Some("hb/moeder/landsmoede/forslag")
    );

    // A parent that was not found, and parents that go round, are no path.
    assert_eq!(found.node_parent_path("d-nowhere"), None);
    let mut round = folder("d-forslag", "forslag", "d-motion");
    round.context_id = "c-lm".into();
    found
        .nodes
        .insert("d-forslag".into(), ("c-lm".into(), Node::of(&round, &Held)));
    assert_eq!(found.node_parent_path("d-motion"), None);
}

#[test]
fn directly_under_its_context_is_no_parent_at_all() {
    let mut doc = motion();
    doc.place = rows::Place {
        parent_id: Some("c-hb".into()),
        path: "hb/kontingent".into(),
        deleted_at: None,
        deleted_root: None,
        ..doc.place
    };
    let record = Node::of(&doc, &Held);
    assert_eq!(
        (record.parent.as_ref(), record.binned.as_ref()),
        (None, None)
    );
    let uri = Held.space("c-hb").record(ORG, NODE, &doc.id);
    let theirs = kept(doc.visibility, &doc.published_uri);
    assert_eq!(record.row(&uri, "hb", theirs), Some(doc));
}

#[test]
fn a_context_comes_back_as_it_went_wherever_it_hangs() {
    let event = |parent: &str| rows::Context {
        id: "c-lm".into(),
        kind: rows::ContextKind::Event,
        name: "Landsmøde".into(),
        place: rows::Place {
            deleted_root: Some("c-lm".into()),
            ..place("landsmoede", "hb/moeder/landsmoede", parent)
        },
        content: Some(json!([{"children": [{"text": "Velkommen"}]}])),
        data: None,
        visibility: rows::Visibility::default(),
        published_uri: None,
        legacy_id: None,
    };
    // In a folder of its parent context.
    let in_a_folder = Hanging {
        context_id: Some("c-hb".into()),
        folder_id: Some("d-moeder".into()),
    };
    let ctx = event("d-moeder");
    let record = ContextProfile::of(&ctx, &in_a_folder, &Held);
    assert!(
        record
            .parent_node
            .as_deref()
            .is_some_and(|n| n.ends_with("/d-moeder"))
    );
    assert_eq!(
        record.row(&Held.space("c-lm"), "hb/moeder", Kept::default()),
        Some(ctx)
    );

    // Directly in it.
    let directly = Hanging {
        context_id: Some("c-hb".into()),
        folder_id: None,
    };
    let ctx = event("c-hb");
    let record = ContextProfile::of(&ctx, &directly, &Held);
    assert_eq!(record.parent_node, None);
    assert_eq!(
        record.row(&Held.space("c-lm"), "hb/moeder", Kept::default()),
        Some(ctx)
    );
}

#[test]
fn a_thread_comes_back_as_it_went() {
    let answer = rows::Comment {
        id: "k-2".into(),
        on_id: "k-1".into(),
        root_id: "d-motion".into(),
        context_id: "c-hb".into(),
        author: rows::Author::User {
            did: "did:plc:bob".into(),
        },
        text: "Enig".into(),
        image: Some("file-9".into()),
        tombstone: false,
        created_at: Some("2026-09-04T10:00:00.000Z".into()),
        deleted_at: None,
        deleted_root: None,
        legacy_id: None,
    };
    let record = Comment::of(&answer, &Held).expect("both ends have records");
    assert!(record.subject.uri.ends_with("/wiki.radikal.node/d-motion"));
    assert_eq!(
        record.parent.as_ref().map(|p| p.cid.as_str()),
        Some("bafy-k-1")
    );
    let uri = Held.space("c-hb").record(ORG, COMMENT, "k-2");
    assert_eq!(record.row(&uri), Some(answer.clone()));

    let orphan = rows::Comment {
        on_id: "no-record-yet".into(),
        ..answer
    };
    assert_eq!(Comment::of(&orphan, &Held), None, "nothing to pin");
}

#[test]
fn a_reaction_comes_back_as_it_went_with_or_without_its_reactor() {
    for reactor in [Some("did:plc:bob".to_string()), None] {
        let row = rows::Reaction {
            id: "r-1".into(),
            subject_uri: "k-1".into(),
            reactor_did: reactor,
            emoji: "🎉".into(),
            created_at: Some("2026-09-04T10:00:00.000Z".into()),
            legacy_id: None,
        };
        let record = Reaction::of(&row, "c-hb", &Held).expect("a record");
        let uri = Held.space("c-hb").record(ORG, REACTION, "r-1");
        assert_eq!(record.row(&uri), Some(row));
    }
    // In a member's own repository the repository says who.
    let own = Reaction {
        subject: StrongRef {
            uri: Held.space("c-hb").record(ORG, NODE, "d-motion").to_string(),
            cid: "bafy".into(),
        },
        emoji: "👍".into(),
        author: None,
        legacy_id: None,
        created_at: "2026-09-04T10:00:00.000Z".into(),
    };
    let uri = Held.space("c-hb").record("did:plc:carol", REACTION, "r-2");
    assert_eq!(
        own.row(&uri).and_then(|r| r.reactor_did).as_deref(),
        Some("did:plc:carol")
    );
}

#[test]
fn an_address_parses_back_to_what_made_it() {
    let space = Held.space("c-hb");
    assert_eq!(space.to_string().parse::<SpaceUri>(), Ok(space.clone()));
    let record = space.record("did:plc:alice", NODE, "d-1");
    assert_eq!(record.to_string().parse::<RecordUri>(), Ok(record));
    for not_one in [
        "at://did:plc:x/wiki.radikal.node/d-1",
        "at://did:plc:x/space/wiki.radikal.context",
        "https://did:plc:x/space/a/b",
        "at://did:plc:x/space//b",
    ] {
        assert!(not_one.parse::<SpaceUri>().is_err(), "{not_one}");
        assert!(not_one.parse::<RecordUri>().is_err(), "{not_one}");
    }
}

// -- What the lexicons say. --

fn lexicon(name: &str) -> Value {
    let path = format!(
        "{}/../../lexicons/wiki/radikal/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("the lexicon")).expect("json")
}

/// Every key the record has is one its lexicon declares, and everything the
/// lexicon requires is there even when the row had the least to say.
fn held_to(name: &str, full: Value, least: Value) {
    let record = &lexicon(name)["defs"]["main"]["record"];
    let declared: BTreeSet<&str> = record["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .map(String::as_str)
        .collect();
    let keys = |v: &Value| -> BTreeSet<String> {
        v.as_object().expect("an object").keys().cloned().collect()
    };
    for key in keys(&full) {
        assert!(
            declared.contains(key.as_str()),
            "{name} declares no `{key}`"
        );
    }
    for required in record["required"].as_array().expect("required") {
        let required = required.as_str().expect("a name");
        assert!(
            keys(&least).contains(required),
            "{name}: `{required}` missing"
        );
    }
}

#[test]
fn every_record_is_what_its_lexicon_says() {
    let doc = motion();
    let bare = rows::Document {
        place: rows::Place::default(),
        content: None,
        data: None,
        authors: Vec::new(),
        legacy_id: None,
        ..doc.clone()
    };
    let full = serde_json::to_value(Node::of(&doc, &Held)).expect("json");
    held_to(
        "node",
        full,
        serde_json::to_value(Node::of(&bare, &Held)).expect("json"),
    );

    let ctx = rows::Context {
        id: "c-hb".into(),
        kind: rows::ContextKind::Group,
        name: "Hovedbestyrelsen".into(),
        place: place("hb", "hb", "home"),
        content: Some(json!([])),
        data: Some(json!({})),
        visibility: rows::Visibility::default(),
        published_uri: None,
        legacy_id: Some("1".into()),
    };
    let hanging = Hanging {
        context_id: Some("home".into()),
        folder_id: Some("d-f".into()),
    };
    let full = serde_json::to_value(ContextProfile::of(&ctx, &hanging, &Held)).expect("json");
    let least = rows::Context {
        place: rows::Place::default(),
        content: None,
        data: None,
        legacy_id: None,
        ..ctx.clone()
    };
    held_to(
        "contextProfile",
        full,
        serde_json::to_value(ContextProfile::of(&least, &Hanging::default(), &Held)).expect("json"),
    );

    let said = rows::Comment {
        id: "k-2".into(),
        on_id: "k-1".into(),
        root_id: "d-motion".into(),
        context_id: "c-hb".into(),
        author: rows::Author::User {
            did: "did:plc:bob".into(),
        },
        text: "Enig".into(),
        image: Some("file-9".into()),
        tombstone: true,
        created_at: None,
        deleted_at: Some("2026-09-05T10:00:00.000Z".into()),
        deleted_root: Some("k-1".into()),
        legacy_id: Some("2".into()),
    };
    let full = serde_json::to_value(Comment::of(&said, &Held)).expect("json");
    let least = rows::Comment {
        on_id: "d-motion".into(),
        tombstone: false,
        deleted_at: None,
        deleted_root: None,
        legacy_id: None,
        ..said
    };
    held_to(
        "comment",
        full,
        serde_json::to_value(Comment::of(&least, &Held)).expect("json"),
    );

    let given = rows::Reaction {
        id: "r-1".into(),
        subject_uri: "k-1".into(),
        reactor_did: Some("did:plc:bob".into()),
        emoji: "🎉".into(),
        created_at: None,
        legacy_id: Some("3".into()),
    };
    let full = serde_json::to_value(Reaction::of(&given, "c-hb", &Held)).expect("json");
    held_to("reaction", full.clone(), full);

    let file = FileRow {
        id: "file-1".into(),
        context_id: "c-hb".into(),
        owner_did: Some("did:plc:alice".into()),
        sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".into(),
        size: 4,
        mime: "application/pdf".into(),
        name: Some("dagsorden.pdf".into()),
        created_at: "2026-09-01T09:00:00.000Z".into(),
    };
    let blob = json!({"$type": "blob", "ref": {"$link": "bafkrei"}, "mimeType": "application/pdf", "size": 4});
    let full = File::of(&file, blob.clone());
    let uri = Held.space("c-hb").record(ORG, FILE, "file-1");
    assert_eq!(full.row(&uri), Some(file.clone()));
    let least = FileRow {
        owner_did: None,
        name: None,
        ..file
    };
    held_to(
        "file",
        serde_json::to_value(full).expect("json"),
        serde_json::to_value(File::of(&least, blob)).expect("json"),
    );

    // Every kind a row can have is one the lexicon knows.
    let known = |name: &str| -> BTreeSet<String> {
        lexicon(name)["defs"]["main"]["record"]["properties"]["kind"]["knownValues"]
            .as_array()
            .expect("knownValues")
            .iter()
            .map(|v| v.as_str().expect("a kind").to_string())
            .collect()
    };
    use rows::{ContextKind as C, DocumentKind as D};
    for kind in [
        D::Document,
        D::Folder,
        D::File,
        D::Policy,
        D::Position,
        D::Candidate,
        D::Change,
        D::Question,
        D::Poll,
        D::Canvas,
    ] {
        let name = serde_json::to_value(kind).expect("json");
        assert!(
            known("node").contains(name.as_str().expect("a string")),
            "{name}"
        );
    }
    for kind in [C::Home, C::Group, C::Event, C::Site] {
        let name = serde_json::to_value(kind).expect("json");
        assert!(
            known("contextProfile").contains(name.as_str().expect("a string")),
            "{name}"
        );
    }
}
