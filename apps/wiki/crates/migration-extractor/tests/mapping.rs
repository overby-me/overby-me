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

/// A node with more said about it than [`node`] says: `more` is merged over it.
fn node_with(id: &str, mime: &str, more: serde_json::Value) -> InterimNode {
    let mut row = json!({
        "id": id, "name": format!("name-{id}"), "key": format!("key-{id}"),
        "mimeId": mime, "parentId": "ctx1", "contextId": "ctx1",
        "ownerId": null, "data": null, "createdAt": "2026-01-01T00:00:00Z"
    });
    for (key, value) in more.as_object().expect("an object") {
        row[key] = value.clone();
    }
    serde_json::from_value(row).unwrap()
}

/// A poll comes across as what was asked and HOW IT WENT. The interim keeps the
/// question in the node's name and the open state in `mutable`; the extractor
/// read `data.question` and `data.open`, which nothing writes, so every poll
/// came out unnamed, and its result was reported as unmigratable and dropped.
#[test]
fn a_poll_is_carried_with_its_result_and_never_its_ballots() {
    let ballot = |id: &str, poll: &str, chosen: serde_json::Value| {
        node_with(id, "vote/vote", json!({"parentId": poll, "data": chosen}))
    };
    let mut binned = ballot("v4", "p1", json!([1]));
    binned.deleted_at = Some("2026-02-01T00:00:00Z".into());
    let nodes = vec![
        node_with("mo", "vote/policy", json!({"name": "Forslag 1"})),
        node_with(
            "p1",
            "vote/poll",
            json!({
                "name": "Forslag 1", "parentId": "mo", "mutable": false,
                "updatedAt": "2026-01-02T00:00:00Z",
                "data": {"options": ["for", "against", "blank"], "minVote": 1, "maxVote": 1,
                         "hidden": true, "secret": true, "voters": ["someone"]}
            }),
        ),
        ballot("v1", "p1", json!([0])),
        ballot("v2", "p1", json!([0])),
        ballot("v3", "p1", json!([2])),
        binned,
        ballot("v5", "another-poll", json!([1])),
    ];
    let ex = extract(&nodes, &[], &[]);

    assert_eq!(ex.polls.len(), 1);
    let poll = &ex.polls[0];
    assert_eq!(poll.question, "Forslag 1");
    assert_eq!(
        poll.counts,
        [2, 0, 1],
        "a binned ballot, or another poll's, was counted"
    );
    assert_eq!(poll.ballots, 3);
    assert!(poll.blank && poll.secret && poll.hide_tally);
    assert_eq!((poll.min, poll.max), (1, 1));
    assert_eq!(poll.closed_at.as_deref(), Some("2026-01-02T00:00:00Z"));

    // And it has its place in the tree, under the motion it was on.
    let place = ex
        .documents
        .iter()
        .find(|d| d.id == "p1")
        .expect("its document");
    assert_eq!(place.kind, DocumentKind::Poll);
    assert_eq!(place.place.parent_id.as_deref(), Some("mo"));
    assert!(
        ex.report.unmapped_mimes.is_empty(),
        "{:?}",
        ex.report.unmapped_mimes
    );
    assert!(
        ex.report.unmapped_source.is_empty(),
        "{:?}",
        ex.report.unmapped_source
    );
}

#[test]
fn a_poll_open_at_the_dump_comes_across_closed_and_is_reported() {
    let nodes = vec![node_with(
        "p1",
        "vote/poll",
        json!({
            "mutable": true, "data": {"options": ["a", "b"]}
        }),
    )];
    let ex = extract(&nodes, &[], &[]);
    assert_eq!(ex.polls[0].ballots, 0);
    assert!(
        ex.report
            .unmapped_source
            .contains_key("nodes(vote/poll).mutable"),
        "{:?}",
        ex.report.unmapped_source
    );
}

#[test]
fn a_canvas_is_carried_with_what_was_painted_on_it() {
    let cell = |id: &str, key: &str, colour: u64, by: &str| {
        node_with(
            id,
            "canvas/pixel",
            json!({
                "parentId": "cv", "key": key, "ownerId": by, "data": {"c": colour},
                "updatedAt": "2026-01-03T00:00:00Z"
            }),
        )
    };
    let nodes = vec![
        node_with(
            "cv",
            "canvas/canvas",
            json!({
                "name": "Tavlen", "mutable": false, "data": {"w": 16, "h": 500, "cooldown": 20}
            }),
        ),
        cell("x1", "p_3_4", 7, "u-bob"),
        cell("x2", "p_0_0", 25, "u-alice"),
        cell("x3", "not-a-cell", 1, "u-bob"),
    ];
    let ex = extract(&nodes, &[], &[]);
    let canvas = &ex.canvases[0];
    assert_eq!(
        (canvas.width, canvas.height, canvas.cooldown),
        (16, 128, 20),
        "a side is capped"
    );
    assert!(!canvas.open);
    assert_eq!(canvas.cells.len(), 2, "what is not a cell is not painted");
    let cell = canvas
        .cells
        .iter()
        .find(|c| (c.x, c.y) == (3, 4))
        .expect("the cell");
    assert_eq!(
        (cell.colour, cell.painter_did.as_deref()),
        (7, Some("u-bob"))
    );
    assert_eq!(ex.documents[0].kind, DocumentKind::Canvas);
    assert!(
        ex.report.unmapped_mimes.is_empty(),
        "{:?}",
        ex.report.unmapped_mimes
    );
}

#[test]
fn feedback_and_reactions_are_carried() {
    let react = |id: &str, by: &str, emoji: &str| {
        node_with(
            id,
            "vote/reaction",
            json!({
                "parentId": "k1", "ownerId": by, "name": emoji, "data": {"emoji": emoji}
            }),
        )
    };
    let nodes = vec![
        node_with(
            "fb",
            "wiki/feedback",
            json!({
                "name": "panicked at poll.rs", "ownerId": "u-bob", "updatedAt": "2026-01-09T00:00:00Z",
                "data": {"kind": "crash", "message": "panicked at poll.rs:412", "path": "/closed",
                         "appVersion": "1.2.3", "commit": "abc", "userAgent": "Firefox",
                         "crashDigest": "00ff", "seen": 5, "reporters": ["u-bob", "anonymous"]}
            }),
        ),
        react("r1", "u-bob", "🎉"),
        react("r2", "u-bob", "🎉"),
        react("r3", "u-alice", "🎉"),
        node_with("r4", "vote/reaction", json!({"parentId": "k1", "name": ""})),
    ];
    let ex = extract(&nodes, &[], &[]);
    let report = &ex.feedback[0];
    assert_eq!((report.kind.as_str(), report.seen), ("crash", 5));
    assert_eq!(
        report.digest.as_deref(),
        Some("00ff"),
        "a known crash keeps its row"
    );
    assert_eq!(report.reporters, ["u-bob", "anonymous"]);
    assert_eq!(report.updated_at.as_deref(), Some("2026-01-09T00:00:00Z"));

    assert_eq!(ex.reactions.len(), 2, "one per person per emoji");
    assert!(ex.reactions.iter().all(|r| r.subject_uri == "k1"));
    assert_eq!(ex.report.unmapped_source["nodes(vote/reaction)"].count, 1);
}

/// An interim account cannot sign in here: a person is their DID now. It is
/// handed to whoever proves the address it was registered under, so only an
/// address the interim had VERIFIED may be carried. An unverified one is an
/// address somebody typed, and they would inherit the account.
#[test]
fn an_account_is_recognized_by_a_verified_address_only() {
    let users: Vec<InterimUser> = serde_json::from_value(json!([
        {"id": "u-alice", "displayName": "Alice", "email": " Alice@X.dk ", "emailVerified": true},
        {"id": "u-bob", "email": "bob@x.dk", "emailVerified": false},
        {"id": "u-old-dump"},
    ]))
    .unwrap();
    let ex = extract(&[], &[], &users);
    assert_eq!(
        ex.users.len(),
        3,
        "every account keeps its name and its work"
    );
    assert_eq!(
        ex.accounts,
        [LegacyAccount {
            id: "u-alice".into(),
            email: "alice@x.dk".into()
        }]
    );
    assert_eq!(ex.report.left_behind["users.email, not verified"], 2);
}

/// A claim link is spent once its seat is taken, and the interim keeps it on
/// the row all the same. Here a seat held by a carried account can be claimed,
/// so the old link would open it to whoever still has the invitation.
#[test]
fn a_spent_claim_link_is_not_carried() {
    let row = |id: &str, account: Option<&str>| -> InterimMember {
        serde_json::from_value(json!({
            "id": id, "email": format!("{id}@x.dk"), "nodeId": account, "parentId": "ctx1",
            "accepted": account.is_some(), "active": true, "claimToken": format!("tok-{id}"),
        }))
        .unwrap()
    };
    let nodes = vec![node_with("ctx1", "wiki/group", json!({"parentId": null}))];
    let ex = extract(
        &nodes,
        &[row("seated", Some("u-alice")), row("waiting", None)],
        &[],
    );
    let token = |id: &str| {
        let member = ex.members.iter().find(|m| m.id == id).expect("member");
        member.claim_token.clone()
    };
    assert_eq!(token("seated"), None);
    assert_eq!(token("waiting").as_deref(), Some("tok-waiting"));
    assert_eq!(ex.report.left_behind["members.claim_token, spent"], 1);
}

/// The interim bins a comment where the new table deletes it, so a comment
/// somebody deleted would have come back at the cutover. One binned along with
/// its document is another matter: it is hidden with the document, and has to
/// be there when the document is restored.
#[test]
fn what_was_deleted_stays_deleted() {
    let binned = |id: &str, mime: &str, parent: &str, root: &str| {
        node_with(
            id,
            mime,
            json!({
                "parentId": parent, "name": "🎉", "data": {"text": "x", "emoji": "🎉"},
                "deleted_at": "2026-02-01T00:00:00Z", "deleted_root": root
            }),
        )
    };
    let nodes = vec![
        node_with("ctx1", "wiki/group", json!({"parentId": null})),
        node_with("doc", "wiki/document", json!({})),
        node_with(
            "gone",
            "wiki/document",
            json!({"deleted_at": "2026-02-01T00:00:00Z", "deleted_root": "gone"}),
        ),
        node_with("k-live", "vote/comment", json!({"parentId": "doc"})),
        binned("k-deleted", "vote/comment", "doc", "k-deleted"),
        binned("k-reply", "vote/comment", "k-deleted", "k-deleted"),
        binned("r-deleted", "vote/reaction", "k-deleted", "k-deleted"),
        binned("k-with-doc", "vote/comment", "gone", "gone"),
        binned("r-with-doc", "vote/reaction", "k-with-doc", "gone"),
        binned("fb-deleted", "wiki/feedback", "home", "fb-deleted"),
    ];
    let ex = extract(&nodes, &[], &[]);

    let mut carried: Vec<&str> = ex.comments.iter().map(|k| k.id.as_str()).collect();
    carried.sort_unstable();
    assert_eq!(carried, ["k-live", "k-with-doc"]);
    assert_eq!(ex.reactions.len(), 1);
    assert_eq!(ex.reactions[0].id, "r-with-doc");
    assert!(ex.feedback.is_empty());

    assert_eq!(ex.report.left_behind["vote/comment, deleted"], 2);
    assert_eq!(ex.report.left_behind["vote/reaction, deleted"], 1);
    assert_eq!(ex.report.left_behind["wiki/feedback, deleted"], 1);
    assert!(
        ex.report.unmapped_source.is_empty() && ex.report.unmapped_mimes.is_empty(),
        "left behind on purpose is not a gap: {:?}",
        ex.report
    );
}

/// The new table holds one row per crash. The interim looks a crash up and then
/// files it, so two people hitting it at once left two rows behind.
#[test]
fn a_crash_filed_twice_comes_across_once() {
    let crash = |id: &str, seen: u64, reporters: serde_json::Value, updated: &str| {
        node_with(
            id,
            "wiki/feedback",
            json!({
                "updatedAt": updated,
                "data": {"kind": "crash", "message": "panicked", "crashDigest": "00ff",
                         "seen": seen, "reporters": reporters}
            }),
        )
    };
    let nodes = vec![
        crash(
            "fb1",
            5,
            json!(["u-bob", "anonymous"]),
            "2026-01-09T00:00:00Z",
        ),
        crash(
            "fb2",
            2,
            json!(["u-alice", "anonymous"]),
            "2026-03-09T00:00:00Z",
        ),
        node_with(
            "fb3",
            "wiki/feedback",
            json!({"data": {"kind": "bug", "message": "the button is grey"}}),
        ),
        node_with(
            "fb4",
            "wiki/feedback",
            json!({"data": {"kind": "bug", "message": "the button is grey"}}),
        ),
    ];
    let ex = extract(&nodes, &[], &[]);
    let ids: Vec<&str> = ex.feedback.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        ids,
        ["fb1", "fb3", "fb4"],
        "what two people wrote is two reports"
    );
    let crash = &ex.feedback[0];
    assert_eq!(crash.seen, 7);
    assert_eq!(crash.reporters, ["u-bob", "anonymous", "u-alice"]);
    assert_eq!(crash.updated_at.as_deref(), Some("2026-03-09T00:00:00Z"));
}

/// A context is open to everyone when it has an ACTIVE permission row for the
/// `public` role that grants `select`. Without the rows every context came out
/// closed, so a cutover would have shut the public pages.
#[test]
fn a_context_is_as_open_as_its_permission_rows_say() {
    let ctx = |id: &str| {
        node_with(
            id,
            "wiki/group",
            json!({"parentId": "root", "contextId": null}),
        )
    };
    let row = |ctx: &str, role: &str, select: bool, active: bool| json!({"contextId": ctx, "role": role, "select": select, "active": active});
    let mut snap: Snapshot = serde_json::from_value(json!({
        "nodes": [], "members": [], "users": [],
        "permissions": [
            row("open", "public", true, true),
            row("open", "member", true, true),
            row("shut-again", "public", true, false),
            row("members-only", "member", true, true),
            row("no-select", "public", false, true),
        ]
    }))
    .unwrap();
    snap.nodes = ["open", "shut-again", "members-only", "no-select"]
        .map(ctx)
        .into();
    let ex = extract_snapshot(&snap);
    let public: Vec<&str> = ex
        .contexts
        .iter()
        .filter(|c| c.visibility == Visibility::Public)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(public, ["open"]);
    assert!(ex.report.unmapped_source.is_empty());

    // An older dump has no rows at all, and says what that costs.
    snap.permissions = None;
    let ex = extract_snapshot(&snap);
    assert!(
        ex.contexts
            .iter()
            .all(|c| c.visibility == Visibility::Private)
    );
    assert!(ex.report.unmapped_source.contains_key("permissions"));
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

/// A chip points at a node, and a group is a node too. Read as a person, a
/// branch that put a motion forward became an account with the group's id, which
/// the loader then made a user row for.
#[test]
fn a_group_named_as_an_author_stays_a_group() {
    let mut nodes = vec![node(
        "d1",
        "vote/policy",
        json!({"content": {"blocks": []}}),
    )];
    nodes.push(
        serde_json::from_value::<InterimNode>(json!({
            "id": "branch", "name": "Aarhus", "key": "aarhus", "mimeId": "wiki/group",
            "parentId": "root", "contextId": null, "ownerId": null, "data": null,
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap(),
    );
    let members = vec![
        member("a1", "d1", Some("branch"), None),
        member("a2", "d1", Some("did:bound"), None),
    ];
    let ex = extract(&nodes, &members, &[]);

    let authors = &ex.documents[0].authors;
    assert_eq!(authors[0].context(), Some("branch"), "{authors:?}");
    assert_eq!(authors[1].did(), Some("did:bound"));
    assert!(
        ex.users.iter().all(|u| u.did != "branch"),
        "the group was made an account"
    );
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
        node("sl", "speak/list", json!(null)),            // known, and left behind on purpose
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

    // The legacy mime is unknown; a speaker list is known and not carried, so it
    // is not flagged.
    assert!(
        ex.report
            .unmapped_mimes
            .contains_key("conference/conference")
    );
    assert!(!ex.report.unmapped_mimes.contains_key("speak/list"));
    // Two content docs extracted (candidate + file).
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
            "id": "p", "name": "p", "key": "talerliste", "mimeId": "speak/list",
            "parentId": "d", "contextId": "g", "data": {}
        }))
        .expect("speaker list"),
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
            .contains_key("nodes.parentId -> speak/list"),
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

fn context_owned_by(owner: Option<&str>) -> InterimNode {
    serde_json::from_value(json!({
        "id": "ctx1", "name": "Landsmøde", "key": "landsmøde", "mimeId": "wiki/event",
        "parentId": null, "contextId": "ctx1", "ownerId": owner, "data": null
    }))
    .expect("context")
}

#[test]
fn a_roster_row_keeps_the_only_name_it_has() {
    let mut invited = member("m1", "ctx1", None, Some("bo@x.dk"));
    invited.name = Some("Bo Jensen".into());
    invited.hidden = true;
    invited.accepted = false;
    let ex = extract(&[context_owned_by(None)], &[invited], &[]);
    let m = &ex.members[0];
    assert_eq!(
        m.name.as_deref(),
        Some("Bo Jensen"),
        "a pending invitation has no account to take a name from"
    );
    assert!(m.hidden);
    assert!(
        !m.accepted,
        "an unanswered invitation was accepted on their behalf"
    );
    assert!(
        !ex.report.unmapped_source.contains_key("members.accepted"),
        "accepted is carried now, so it is no gap"
    );
}

/// The general secretary owns Landsmøde 2026 and holds no membership row in it.
#[test]
fn whoever_made_a_context_still_owns_it_after_the_move() {
    let owns = |ex: &Extraction, did: &str| {
        ex.members
            .iter()
            .any(|m| m.user_did.as_deref() == Some(did) && m.role == Role::Owner)
    };

    // No row at all: one is added, hidden, since they were never on the list.
    let ex = extract(&[context_owned_by(Some("gs"))], &[], &[]);
    assert!(owns(&ex, "gs"), "the owner of the meeting lost it");
    assert!(ex.members[0].hidden);
    assert!(ex.members[0].accepted && ex.members[0].active);

    // A plain member row: it is raised, not duplicated.
    let plain = member("m1", "ctx1", Some("gs"), None);
    let ex = extract(&[context_owned_by(Some("gs"))], &[plain], &[]);
    assert_eq!(ex.members.len(), 1);
    assert!(owns(&ex, "gs"));
    assert_eq!(
        ex.report.unmapped_source["nodes.ownerId (context)"].count,
        1
    );

    // Already an owner by their row: nothing to do, and nothing to report.
    let mut already = member("m1", "ctx1", Some("gs"), None);
    already.owner = true;
    let ex = extract(&[context_owned_by(Some("gs"))], &[already], &[]);
    assert_eq!(ex.members.len(), 1);
    assert!(ex.report.unmapped_source.is_empty());
}

#[test]
fn a_member_row_on_something_that_is_no_context_is_reported_not_loaded() {
    let nodes = vec![context_owned_by(None), node("p1", "vote/poll", json!({}))];
    let ex = extract(&nodes, &[member("m1", "p1", Some("u"), None)], &[]);
    assert!(
        ex.members.is_empty(),
        "a membership of a poll would fail the context foreign key"
    );
    assert!(
        ex.report
            .unmapped_source
            .contains_key("members(on vote/poll)")
    );
}
