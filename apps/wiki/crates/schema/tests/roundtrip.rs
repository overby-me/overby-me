//! Executable validation of the entity-subset DDL (round-2 item 11): the
//! schema must actually run, enforce its constraints, and round-trip rows on
//! BOTH engines: real SQLite (rusqlite, bundled: the dialect-claim baseline
//! and the file-format bridge) and the decided Turso Database (the `turso`
//! crate). What each engine cannot do is a recorded finding, not a silent gap.

use wiki_schema::ENTITY_SCHEMA;

// ---------------------------------------------------------------------------
// Real SQLite (rusqlite): full constraint assertions
// ---------------------------------------------------------------------------

fn sqlite_mem() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    conn.pragma_update(None, "foreign_keys", true)
        .expect("fk on");
    conn.execute_batch(ENTITY_SCHEMA).expect("DDL executes");
    conn
}

/// Minimal happy-path seed: one user, one context.
fn seed(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "INSERT INTO user (did, handle, display_name) VALUES ('did:plc:alice', 'alice.test', 'Alice');
         INSERT INTO context (id, kind, name, slug) VALUES ('c1', 'group', 'Group One', 'group-one');",
    )
    .expect("seed");
}

#[test]
fn ddl_executes_and_rows_round_trip_on_sqlite() {
    let conn = sqlite_mem();
    seed(&conn);
    // One row per remaining table, exercising defaults and FKs. Authorship is
    // via the document_author join, not a scalar document.author_did.
    conn.execute_batch(
        "INSERT INTO document (id, context_id, kind, title, content)
           VALUES ('d1', 'c1', 'document', 'Doc', '{\"blocks\":[{\"text\":\"hi\"}]}');
         INSERT INTO document_author (document_id, author_did, ord) VALUES ('d1', 'did:plc:alice', 0);
         INSERT INTO post (id, author_did, group_id, text) VALUES ('p1', 'did:plc:alice', 'c1', 'hello');
         INSERT INTO member (id, user_did, context_id, role) VALUES ('m1', 'did:plc:alice', 'c1', 'owner');
         INSERT INTO comment (id, on_id, context_id, author_did, text) VALUES ('k1', 'd1', 'c1', 'did:plc:alice', 'nice');",
    )
    .expect("inserts");
    let authors: i64 = conn
        .query_row(
            "SELECT count(*) FROM document_author WHERE document_id = 'd1'",
            [],
            |r| r.get(0),
        )
        .expect("author count");
    assert_eq!(authors, 1, "document author join row round-trips");

    // Round trip: read each row back.
    for (table, id_col, id) in [
        ("user", "did", "did:plc:alice"),
        ("context", "id", "c1"),
        ("document", "id", "d1"),
        ("post", "id", "p1"),
        ("member", "id", "m1"),
        ("comment", "id", "k1"),
    ] {
        let n: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {id_col} = ?1"),
                [id],
                |r| r.get(0),
            )
            .expect("select");
        assert_eq!(n, 1, "{table} row round-trips");
    }

    // datetime('now') text default populated.
    let created: String = conn
        .query_row("SELECT created_at FROM context WHERE id = 'c1'", [], |r| {
            r.get(0)
        })
        .expect("created_at");
    assert!(
        created.starts_with("20"),
        "text datetime default: {created}"
    );

    // JSON column round-trips losslessly through TEXT.
    let content: String = conn
        .query_row("SELECT content FROM document WHERE id = 'd1'", [], |r| {
            r.get(0)
        })
        .expect("content");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON back");
    assert_eq!(parsed["blocks"][0]["text"], "hi");
}

#[test]
fn multi_author_and_free_text_authorship_on_sqlite() {
    // The reconciliation this schema exists to prove: a document with MANY
    // authors (census: up to 8), mixing an account (DID) and a free-text name
    // (42% of author chips), which the old scalar document.author_did could not
    // represent and no free-text authorship could survive.
    let conn = sqlite_mem();
    seed(&conn);
    conn.execute_batch(
        "INSERT INTO document (id, context_id, kind, title) VALUES ('d1', 'c1', 'policy', 'Motion');
         INSERT INTO document_author (document_id, author_did, ord) VALUES ('d1', 'did:plc:alice', 0);
         INSERT INTO document_author (document_id, author_text, ord) VALUES ('d1', 'Anonymous Delegate', 1);",
    )
    .expect("multi-author + free-text insert");
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM document_author WHERE document_id = 'd1'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 2, "both authors (one DID, one free-text) round-trip");

    // The scalar column the old schema.sql carried is GONE: this is what makes
    // multi-author and free-text representable at all.
    assert!(
        conn.execute("SELECT author_did FROM document LIMIT 1", [])
            .is_err(),
        "document.author_did no longer exists (authorship is in document_author)"
    );

    // A free-text comment (no account) is now valid; the old NOT NULL author_did
    // would have rejected it.
    conn.execute(
        "INSERT INTO comment (id, on_id, context_id, author_text, text) VALUES ('k1', 'd1', 'c1', 'A Guest', 'nice')",
        [],
    )
    .expect("free-text comment insert");
    // But a comment with NEITHER a DID nor a text author is rejected by the CHECK.
    assert!(
        conn.execute(
            "INSERT INTO comment (id, on_id, context_id, text) VALUES ('k2', 'd1', 'c1', 'orphan')",
            [],
        )
        .is_err(),
        "a comment with no author (neither did nor text) is rejected by the CHECK"
    );
}

#[test]
fn constraints_enforced_on_sqlite() {
    let conn = sqlite_mem();
    seed(&conn);

    // CHECK constraints.
    assert!(
        conn.execute(
            "INSERT INTO context (id, kind, name, slug) VALUES ('cx', 'club', 'X', 'x')",
            []
        )
        .is_err(),
        "kind CHECK rejects unknown kind"
    );
    assert!(conn
        .execute(
            "INSERT INTO document (id, context_id, kind, title, visibility) VALUES ('dx', 'c1', 'document', 'X', 'secret')",
            [],
        )
        .is_err(), "visibility CHECK rejects unknown value");
    assert!(
        conn.execute(
            "INSERT INTO member (id, context_id, role) VALUES ('mx', 'c1', 'admin')",
            []
        )
        .is_err(),
        "role CHECK rejects unknown role"
    );

    // context (parent_id, slug) uniqueness.
    conn.execute("INSERT INTO context (id, kind, name, slug, parent_id) VALUES ('c2', 'event', 'E', 'ev', 'c1')", [])
        .expect("child context");
    assert!(conn
        .execute("INSERT INTO context (id, kind, name, slug, parent_id) VALUES ('c3', 'event', 'E2', 'ev', 'c1')", [])
        .is_err(), "duplicate slug under one parent rejected");

    // FK enforcement (with the pragma ON).
    assert!(
        conn.execute(
            "INSERT INTO post (id, author_did, text) VALUES ('px', 'did:plc:ghost', 'x')",
            []
        )
        .is_err(),
        "FK rejects unknown author"
    );
}

#[test]
fn member_partial_uniques_enforce_each_state_on_sqlite() {
    let conn = sqlite_mem();
    seed(&conn);
    // Two DID-less pending invites with different emails: fine.
    conn.execute_batch(
        "INSERT INTO member (id, context_id, email, claim_token) VALUES ('m1', 'c1', 'a@x.dk', 't1');
         INSERT INTO member (id, context_id, email, claim_token) VALUES ('m2', 'c1', 'b@x.dk', 't2');",
    )
    .expect("pending invites");
    // A second pending invite for the SAME email in the same context: rejected
    // (this is exactly the dedup a (user_did, context_id) PK silently missed).
    assert!(conn
        .execute("INSERT INTO member (id, context_id, email, claim_token) VALUES ('m3', 'c1', 'a@x.dk', 't3')", [])
        .is_err(), "duplicate pending invite rejected");
    // Bind m1 to a DID: it leaves the pending index scope...
    conn.execute(
        "UPDATE member SET user_did = 'did:plc:alice' WHERE id = 'm1'",
        [],
    )
    .expect("bind");
    // ...so the same email may now be re-invited as a fresh pending row (the
    // documented re-invite semantics; the application checks membership first).
    conn.execute("INSERT INTO member (id, context_id, email, claim_token) VALUES ('m4', 'c1', 'a@x.dk', 't4')", [])
        .expect("re-invite after bind");
    // But a SECOND bound row for the same (context, DID) is rejected.
    assert!(
        conn.execute(
            "INSERT INTO member (id, user_did, context_id) VALUES ('m5', 'did:plc:alice', 'c1')",
            []
        )
        .is_err(),
        "duplicate bound membership rejected"
    );
}

#[test]
fn foreign_key_enforcement_depends_on_the_pragma_not_the_default() {
    // FINDING: the FK default is a BUILD-TIME choice, not a SQLite constant.
    // Stock distro SQLite defaults foreign_keys OFF; rusqlite's bundled build
    // compiles with it ON (this test caught that). The AppView must therefore
    // always SET AND READ BACK the pragma per connection, never assume a
    // default. Both behaviours asserted explicitly:
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    conn.execute_batch(ENTITY_SCHEMA).expect("DDL");
    conn.pragma_update(None, "foreign_keys", false)
        .expect("fk off");
    conn.execute(
        "INSERT INTO post (id, author_did, text) VALUES ('px', 'did:plc:ghost', 'x')",
        [],
    )
    .expect("dangling FK accepted with enforcement off");
    conn.pragma_update(None, "foreign_keys", true)
        .expect("fk on");
    let on: i64 = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .expect("readback");
    assert_eq!(on, 1, "pragma readback verifies enforcement");
    assert!(
        conn.execute(
            "INSERT INTO post (id, author_did, text) VALUES ('py', 'did:plc:ghost', 'y')",
            []
        )
        .is_err(),
        "dangling FK rejected with enforcement on"
    );
}

// ---------------------------------------------------------------------------
// Turso Database (the decided engine)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn ddl_executes_and_rows_round_trip_on_turso() {
    let db = turso::Builder::new_local(":memory:")
        .build()
        .await
        .expect("build");
    let conn = db.connect().expect("connect");
    conn.execute_batch(ENTITY_SCHEMA)
        .await
        .expect("DDL executes on turso");

    conn.execute(
        "INSERT INTO user (did, handle, display_name, legacy_id) VALUES ('did:plc:alice', 'alice.test', 'Alice', NULL)",
        (),
    )
    .await
    .expect("insert user");
    conn.execute(
        "INSERT INTO context (id, kind, name, slug, legacy_id) VALUES ('c1', 'group', 'Group One', 'group-one', NULL)",
        (),
    )
    .await
    .expect("insert context");
    conn.execute(
        "INSERT INTO member (id, context_id, email, claim_token, legacy_id) VALUES ('m1', 'c1', 'a@x.dk', 't1', NULL)",
        (),
    )
    .await
    .expect("insert pending member");

    // Round trip.
    let mut rows = conn
        .query("SELECT name, created_at FROM context WHERE id = 'c1'", ())
        .await
        .expect("query");
    let row = rows.next().await.expect("next").expect("row");
    let name: String = row.get(0).expect("name");
    assert_eq!(name, "Group One");
    let created: String = row.get(1).expect("created_at");
    assert!(
        created.starts_with("20"),
        "datetime default on turso: {created}"
    );

    // The partial unique index: duplicate pending invite must be rejected.
    let dup = conn
        .execute(
            "INSERT INTO member (id, context_id, email, claim_token, legacy_id) VALUES ('m2', 'c1', 'a@x.dk', 't2', NULL)",
            (),
        )
        .await;
    assert!(
        dup.is_err(),
        "turso enforces the member_pending partial unique"
    );
}

/// What the AppView's SQL assumes of turso, and the one thing it still cannot
/// assume. turso 0.2.2 had none of the first group: no `EXISTS`, no
/// `IN (subquery)`, no upsert, no omitted nullable-UNIQUE column, and it ignored
/// the foreign-key pragma. Each assertion is a tripwire for a future bump.
#[tokio::test(flavor = "current_thread")]
async fn turso_dialect_the_appview_relies_on() {
    let db = turso::Builder::new_local(":memory:")
        .build()
        .await
        .expect("build");
    let conn = db.connect().expect("connect");
    conn.execute_batch(ENTITY_SCHEMA).await.expect("DDL");

    // Foreign keys are OFF until asked for, as on stock SQLite, so every
    // connection must set the pragma and read it back.
    let dangling = "INSERT INTO post (id, author_did, text) VALUES (?1, 'did:plc:ghost', 'x')";
    conn.execute(dangling, ["p-off"])
        .await
        .expect("a dangling reference is accepted while enforcement is off");
    conn.execute("PRAGMA foreign_keys=ON", ())
        .await
        .expect("the pragma is recognized");
    let mut on = conn
        .query("PRAGMA foreign_keys", ())
        .await
        .expect("readback");
    let row = on.next().await.expect("next").expect("a row");
    assert_eq!(row.get::<i64>(0).expect("int"), 1, "pragma readback");
    drop(on);
    assert!(
        conn.execute(dangling, ["p-on"]).await.is_err(),
        "turso enforces foreign keys once the pragma is on"
    );

    // An omitted nullable UNIQUE column, and both kinds of upsert.
    conn.execute("INSERT INTO user (did) VALUES ('did:plc:alice')", ())
        .await
        .expect("legacy_id may be omitted");
    conn.execute(
        "INSERT OR IGNORE INTO user (did) VALUES ('did:plc:alice')",
        (),
    )
    .await
    .expect("INSERT OR IGNORE");
    conn.execute(
        "INSERT INTO user (did, handle) VALUES ('did:plc:alice', 'alice.test') \
         ON CONFLICT(did) DO UPDATE SET handle = excluded.handle",
        (),
    )
    .await
    .expect("ON CONFLICT DO UPDATE");
    let mut rows = conn
        .query("SELECT handle FROM user WHERE did = 'did:plc:alice'", ())
        .await
        .expect("query");
    let handle: String = rows
        .next()
        .await
        .expect("next")
        .expect("row")
        .get(0)
        .expect("handle");
    assert_eq!(handle, "alice.test", "the upsert updated in place");
    drop(rows);

    // Subqueries, which the read gate is written in.
    conn.execute_batch(
        "INSERT INTO context (id, kind, name, slug, visibility) VALUES ('open', 'group', 'O', 'o', 'public');
         INSERT INTO context (id, kind, name, slug) VALUES ('shut', 'group', 'S', 's');
         INSERT INTO document (id, context_id, kind, title) VALUES ('d-open', 'open', 'document', 'A');
         INSERT INTO document (id, context_id, kind, title) VALUES ('d-shut', 'shut', 'document', 'B');",
    )
    .await
    .expect("seed");
    for gated in [
        "SELECT d.id FROM document d WHERE EXISTS \
           (SELECT 1 FROM context c WHERE c.id = d.context_id AND c.visibility = 'public')",
        "SELECT d.id FROM document d WHERE d.context_id IN \
           (SELECT id FROM context WHERE visibility = 'public')",
    ] {
        let mut rows = conn.query(gated, ()).await.expect("a subquery in WHERE");
        let id: String = rows
            .next()
            .await
            .expect("next")
            .expect("one row")
            .get(0)
            .expect("id");
        assert_eq!(id, "d-open", "{gated}");
        assert!(rows.next().await.expect("next").is_none(), "{gated}");
    }

    // Still missing: a tree cannot be walked in one query, so a path is resolved
    // a segment at a time. When this starts passing, that can be revisited.
    let recursive = conn
        .query(
            "WITH RECURSIVE up(id, parent_id) AS ( \
               SELECT id, parent_id FROM context WHERE id = 'shut' \
               UNION ALL \
               SELECT c.id, c.parent_id FROM context c JOIN up ON c.id = up.parent_id) \
             SELECT id FROM up",
            (),
        )
        .await;
    assert!(
        recursive.is_err(),
        "turso gained recursive CTEs: path resolution can become one query"
    );
}
