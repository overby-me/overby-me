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
        visibility: rows::Visibility::default(),
        published_uri: None,
        legacy_id: Some("0b7e".into()),
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
    assert_eq!(record.row(&uri, "hb/forslag"), Some(doc));
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
    assert_eq!(record.row(&uri, "hb"), Some(doc));
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
    assert_eq!(record.row(&Held.space("c-lm"), "hb/moeder"), Some(ctx));

    // Directly in it.
    let directly = Hanging {
        context_id: Some("c-hb".into()),
        folder_id: None,
    };
    let ctx = event("c-hb");
    let record = ContextProfile::of(&ctx, &directly, &Held);
    assert_eq!(record.parent_node, None);
    assert_eq!(record.row(&Held.space("c-lm"), "hb/moeder"), Some(ctx));
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
    assert_eq!(
        record.row(&uri, Some("file-9".into())),
        Some(answer.clone())
    );

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
    let mut full = serde_json::to_value(Node::of(&doc, &Held)).expect("json");
    // Blobs are attached where a record is written, not by the mapping.
    full["image"] = json!({});
    full["file"] = json!({});
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
    let mut full = serde_json::to_value(ContextProfile::of(&ctx, &hanging, &Held)).expect("json");
    full["image"] = json!({});
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
        image: None,
        tombstone: true,
        created_at: None,
        deleted_at: Some("2026-09-05T10:00:00.000Z".into()),
        deleted_root: Some("k-1".into()),
        legacy_id: Some("2".into()),
    };
    let mut full = serde_json::to_value(Comment::of(&said, &Held)).expect("json");
    full["image"] = json!({});
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
