//! Extractor mapping tests over SYNTHETIC interim rows (no live data). They
//! pin the shape decisions the census surfaced: multi-author documents,
//! free-text authors preserved, roster email normalization, unknown mimes and
//! non-content data keys landing in the gap report, voting/ephemeral nodes
//! excluded rather than mis-mapped.

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
    assert_eq!(ex.contexts[0].slug, "local");
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
fn non_content_data_keys_and_unknown_mimes_hit_the_report() {
    let nodes = vec![
        node(
            "d1",
            "vote/candidate",
            json!({"content": {}, "image": "fileid-1"}),
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

    // The candidate's `image` and the file's `fileId`/`type` are flagged.
    assert!(
        ex.report
            .unmapped_source
            .contains_key("vote/candidate.data.image")
    );
    assert!(
        ex.report
            .unmapped_source
            .contains_key("wiki/file.data.fileId")
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
