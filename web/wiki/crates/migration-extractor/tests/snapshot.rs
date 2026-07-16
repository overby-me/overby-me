//! The FRONT of the migration pipeline: the committed example snapshot (shaped
//! exactly like `scripts/dump-interim-snapshot.nu` output) must parse into the
//! `Snapshot` wrapper and run through `extract()`. This is the reviewable proof
//! that the dump format and the extractor's input contract agree, without any
//! live-DB access.

use migration_extractor::{Snapshot, extract};

#[test]
fn example_snapshot_parses_and_extracts() {
    let raw = include_str!("snapshot.example.json");
    let snap: Snapshot = serde_json::from_str(raw).expect("snapshot parses into the wrapper");

    let ex = extract(&snap.nodes, &snap.members, &snap.users);

    // The group node becomes a context; the document node a document; the user a
    // realized User row; only the roster member (m1, hung on the context) is a
    // membership, while the author chip (a1, hung on the content node) becomes
    // the document's author rather than a member.
    assert_eq!(ex.contexts.len(), 1, "the group is a context");
    assert_eq!(ex.documents.len(), 1, "the document node is a document");
    assert_eq!(ex.users.len(), 1, "the interim user is realized");
    assert_eq!(
        ex.members.len(),
        1,
        "only the roster member, not the author chip"
    );
    assert_eq!(
        ex.documents[0].authors.len(),
        1,
        "the author chip landed on the document"
    );
}
