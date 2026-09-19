//! Extractor mapping tests over SYNTHETIC interim rows (no live data). They
//! pin the shape decisions the census surfaced: multi-author documents,
//! free-text authors preserved, roster email normalization, unknown mimes
//! landing in the gap report, non-content data keys carried rather than lost,
//! every node keeping its place in the tree, voting/ephemeral nodes excluded
//! rather than mis-mapped.

use migration_extractor::*;
use serde_json::json;
use wiki_domain_types::*;

fn node(id: &str, mime: &str, data: serde_json::Value) -> InterimNode {
    serde_json::from_value(json!({
        "id": id, "name": format!("name-{id}"), "key": format!("key-{id}"),
        "mimeId": mime, "parentId": "ctx1", "contextId": "ctx1",
        "ownerId": null, "data": data, "createdAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

fn member(id: &str, parent: &str, node_id: Option<&str>, email: Option<&str>) -> InterimMember {
    serde_json::from_value(json!({
        "id": id, "name": format!("Member {id}"), "email": email,
        "nodeId": node_id, "parentId": parent,
        "accepted": true, "active": true, "owner": false,
    }))
    .unwrap()
}

#[test]
fn poll_extracts_and_cast_ballot_is_reported() {
    let nodes = vec![
        node(
            "p1",
            "vote/poll",
            json!({"question": "Farve?", "options": ["Rod", "Gron"], "open": true, "secret": false}),
        ),
        node("v1", "vote/vote", json!({})),
    ];
    let ex = extract(&nodes, &[], &[]);
    assert_eq!(
        ex.polls.len(),
        1,
        "the poll is the one migratable voting entity"
    );
    let poll = &ex.polls[0];
    assert_eq!(poll.question, "Farve?");
    assert_eq!(poll.options, vec!["Rod".to_string(), "Gron".to_string()]);
    assert!(poll.open);
    assert!(!poll.secret);
    assert_eq!(poll.context_id, "ctx1");
    // The cast ballot is reported as unmigratable, not extracted.
    assert!(
        ex.report
            .unmapped_source
            .keys()
            .any(|k| k.contains("vote/vote")),
        "the cast ballot is reported unmigratable"
    );
}

#[test]
fn context_and_roster_member_map() {
    let nodes = vec![
        serde_json::from_value::<InterimNode>(json!({
            "id": "ctx1", "name": "Local Chapter", "key": "local", "mimeId": "wiki/group",
            "parentId": "root", "contextId": null, "ownerId": null, "data": null,
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap(),
    ];
    let members = vec![member("m1", "ctx1", Some("did:x"), Some("  Alice@X.DK "))];
    let ex = extract(&nodes, &members, &[]);

    assert_eq!(ex.contexts.len(), 1);
    assert_eq!(ex.contexts[0].kind, ContextKind::Group);
    assert_eq!(ex.contexts[0].place.slug, "local");
    assert_eq!(ex.members.len(), 1);
    // Email normalized (lowercased + trimmed): the census's 11 variant clusters.
    assert_eq!(ex.members[0].email.as_deref(), Some("alice@x.dk"));
    assert!(!ex.members[0].is_pending_invite());
}

#[test]
fn document_collects_multiple_authors_including_free_text() {
    let nodes = vec![node(
        "d1",
        "vote/policy",
        json!({"content": {"blocks": []}}),
    )];
    let members = vec![
        member("a1", "d1", Some("did:bound"), None), // bound author chip
        member("a2", "d1", None, None),              // free-text author chip
    ];
    let ex = extract(&nodes, &members, &[]);

    assert_eq!(ex.documents.len(), 1);
    let doc = &ex.documents[0];
    assert_eq!(doc.kind, DocumentKind::Policy);
    assert_eq!(
        doc.authors.len(),
        2,
        "both author chips collected (census: up to 8)"
    );
    assert!(doc.authors.iter().any(|a| matches!(a, Author::User { .. })));
    assert!(
        doc.authors
            .iter()
            .any(|a| matches!(a, Author::FreeText { .. }))
    );
    // Author chips are NOT roster members.
    assert_eq!(ex.members.len(), 0);
    // Slate content carried verbatim.
    assert!(doc.content.is_some());
}

#[test]
fn non_content_data_is_carried_and_unknown_mimes_hit_the_report() {
    let nodes = vec![
        node(
            "d1",
            "vote/candidate",
            json!({"content": {"ok": 1}, "image": "fileid-1"}),
        ),
        node(
            "f1",
            "wiki/file",
            json!({"fileId": "x", "type": "image/png"}),
        ),
        node("x1", "conference/conference", json!(null)), // legacy one-off
        node("p1", "vote/poll", json!({"options": ["a"], "voters": []})), // excluded, not unknown
    ];
    let ex = extract(&nodes, &[], &[]);

    // A file IS its `fileId` and `type`, and a candidate's photo is its `image`:
    // dropping them would migrate every attachment as an empty page.
    let candidate = ex.documents.iter().find(|d| d.id == "d1").expect("d1");
    assert_eq!(candidate.content, Some(json!({"ok": 1})));
    assert_eq!(candidate.data, Some(json!({"image": "fileid-1"})));
    let file = ex.documents.iter().find(|d| d.id == "f1").expect("f1");
    assert_eq!(file.content, None);
    assert_eq!(file.data, Some(json!({"fileId": "x", "type": "image/png"})));
    assert!(
        !ex.report
            .unmapped_source
            .keys()
            .any(|k| k.contains(".data.")),
        "carried keys are not gaps: {:?}",
        ex.report.unmapped_source.keys().collect::<Vec<_>>()
    );

    // The legacy mime is unknown; the poll is a known-excluded mime (not flagged).
    assert!(
        ex.report
            .unmapped_mimes
            .contains_key("conference/conference")
    );
    assert!(!ex.report.unmapped_mimes.contains_key("vote/poll"));
    // Two content docs extracted (candidate + file); poll and conference excluded.
    assert_eq!(ex.documents.len(), 2);
}

/// A small interim tree: home > group > folder > (event, doc). The event sits in
/// a FOLDER, which the interim allows and the old context foreign key did not.
fn tree() -> Vec<InterimNode> {
    let n = |id: &str, mime: &str, key: &str, parent: Option<&str>, extra: serde_json::Value| {
        let mut v = json!({
            "id": id, "name": id, "key": key, "mimeId": mime,
            "parentId": parent, "contextId": "g", "data": null,
        });
        v.as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        serde_json::from_value::<InterimNode>(v).expect("node")
    };
    vec![
        n("home", "wiki/home", "", None, json!({})),
        n("g", "wiki/group", "ungdom", Some("home"), json!({})),
        n(
            "f",
            "wiki/folder",
            "møder",
            Some("g"),
            json!({"index": 3, "attachable": false}),
        ),
        n("e", "wiki/event", "landsmøde", Some("f"), json!({})),
        n(
            "d",
            "wiki/document",
            "referat",
            Some("f"),
            json!({"mutable": false, "ownerId": "u1", "updatedAt": "2026-02-02T00:00:00Z",
                   "deleted_at": "2026-03-03T00:00:00Z"}),
        ),
        n("s", "wiki/site", "blog", Some("home"), json!({})),
    ]
}

#[test]
fn every_node_keeps_its_place_in_the_tree() {
    let ex = extract(&tree(), &[], &[]);
    let ctx = |id: &str| ex.contexts.iter().find(|c| c.id == id).expect("context");
    let doc = |id: &str| ex.documents.iter().find(|d| d.id == id).expect("document");

    // The root is not a row, so its children are roots and its key is no segment.
    assert_eq!(ctx("g").place.parent_id, None);
    assert_eq!(ctx("g").place.path, "ungdom");
    assert_eq!(ctx("s").kind, ContextKind::Site);

    let folder = doc("f");
    assert_eq!(folder.place.slug, "møder");
    assert_eq!(folder.place.path, "ungdom/møder");
    assert_eq!(folder.place.idx, 3);
    assert!(!folder.place.attachable, "the folder lock was lost");

    // An event inside a folder keeps the folder as its parent.
    assert_eq!(ctx("e").place.parent_id.as_deref(), Some("f"));
    assert_eq!(ctx("e").place.path, "ungdom/møder/landsmøde");

    let minutes = doc("d");
    assert!(
        !minutes.mutable,
        "a submitted document became editable again"
    );
    assert_eq!(minutes.place.owner_did.as_deref(), Some("u1"));
    assert_eq!(
        minutes.place.updated_at.as_deref(),
        Some("2026-02-02T00:00:00Z")
    );
    assert!(
        minutes.place.deleted_at.is_some(),
        "a binned node was restored"
    );

    assert!(
        ex.report.unmapped_source.is_empty(),
        "hanging off the root is expected, not a gap: {:?}",
        ex.report.unmapped_source.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_dumped_path_wins_over_one_rebuilt_from_keys() {
    let mut nodes = tree();
    let folder = nodes.iter_mut().find(|n| n.id == "f").expect("f");
    folder.path = Some("as/the/trigger/wrote/it".into());
    let ex = extract(&nodes, &[], &[]);
    let folder = ex.documents.iter().find(|d| d.id == "f").expect("f");
    assert_eq!(folder.place.path, "as/the/trigger/wrote/it");
}

#[test]
fn a_node_under_a_kind_that_does_not_migrate_is_reported_not_silently_rerooted() {
    let mut nodes = tree();
    nodes.push(
        serde_json::from_value(json!({
            "id": "p", "name": "p", "key": "afstemning", "mimeId": "vote/poll",
            "parentId": "d", "contextId": "g", "data": {}
        }))
        .expect("poll"),
    );
    nodes.push(
        serde_json::from_value(json!({
            "id": "q", "name": "q", "key": "spørgsmål", "mimeId": "vote/question",
            "parentId": "p", "contextId": "g", "data": null
        }))
        .expect("question"),
    );
    let ex = extract(&nodes, &[], &[]);
    let question = ex.documents.iter().find(|d| d.id == "q").expect("q");
    assert_eq!(question.place.parent_id, None);
    assert!(
        ex.report
            .unmapped_source
            .contains_key("nodes.parentId -> vote/poll"),
        "{:?}",
        ex.report.unmapped_source.keys().collect::<Vec<_>>()
    );
}

#[test]
fn comment_text_extracted_from_data() {
    let nodes = vec![serde_json::from_value::<InterimNode>(json!({
        "id": "k1", "name": "commenter", "key": "k1", "mimeId": "vote/comment",
        "parentId": "d1", "contextId": "ctx1", "ownerId": "did:c", "data": {"text": "nice work"},
        "createdAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap()];
    let ex = extract(&nodes, &[], &[]);
    assert_eq!(ex.comments.len(), 1);
    assert_eq!(ex.comments[0].text, "nice work");
    assert!(matches!(ex.comments[0].author, Author::User { .. }));
}

#[test]
fn extraction_round_trips_through_serde() {
    // The fixtures the extractor emits must serialize and re-parse (they are
    // the importer's input and the crate's regression fixtures).
    let nodes = vec![node(
        "d1",
        "wiki/document",
        json!({"content": {"ok": true}}),
    )];
    let ex = extract(&nodes, &[member("a1", "d1", None, None)], &[]);
    let s = serde_json::to_string(&ex.documents).unwrap();
    let back: Vec<Document> = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ex.documents);
}
