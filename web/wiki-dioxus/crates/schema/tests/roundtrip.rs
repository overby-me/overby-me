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
    // One row per remaining table, exercising defaults and FKs.
    conn.execute_batch(
        "INSERT INTO document (id, context_id, kind, title, content, author_did)
           VALUES ('d1', 'c1', 'document', 'Doc', '{\"blocks\":[{\"text\":\"hi\"}]}', 'did:plc:alice');
         INSERT INTO post (id, author_did, group_id, text) VALUES ('p1', 'did:plc:alice', 'c1', 'hello');
         INSERT INTO member (id, user_did, context_id, role) VALUES ('m1', 'did:plc:alice', 'c1', 'owner');
         INSERT INTO comment (id, on_id, context_id, author_did, text) VALUES ('k1', 'd1', 'c1', 'did:plc:alice', 'nice');",
    )
    .expect("inserts");

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

    // FINDING (turso 0.2.2): an INSERT that OMITS a nullable UNIQUE column is
    // rejected at parse time ("column X is not nullable"), while an explicit
    // NULL works and SQLite NULL-uniqueness semantics hold (multiple NULLs
    // fine). Dialect gap to re-test at the 1.0 gate; inserts below therefore
    // pass explicit NULLs where stock SQLite would allow omission.
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
