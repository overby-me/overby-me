//! The ballot DDL must execute, enforce its keys, and round-trip rows on BOTH
//! engines: the decided Turso Database and the lossless SQLite bridge (rusqlite,
//! bundled). Mirrors `crates/schema/tests/roundtrip.rs` for the private ballot
//! tables (item 13's DDL) plus the durable board schema (item 12).

use ballot_store::{BALLOT_DDL, BOARD_DDL};

// ---------------------------------------------------------------------------
// Real SQLite (rusqlite): the bridge target, with FK + PK enforcement.
// ---------------------------------------------------------------------------

#[test]
fn ballot_ddl_executes_and_enforces_on_sqlite() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    conn.pragma_update(None, "foreign_keys", true)
        .expect("fk on");
    conn.execute_batch(BALLOT_DDL).expect("ballot DDL executes");
    conn.execute_batch(BOARD_DDL).expect("board DDL executes");

    conn.execute_batch(
        "INSERT INTO poll (id, context_id, question, options) VALUES ('p1', 'c1', 'Q', '[]');
         INSERT INTO eligibility (poll_id, did, base_weight) VALUES ('p1', 'did:plc:a', 2);
         INSERT INTO delegation (poll_id, from_did, to_did, assignment_sig) VALUES ('p1', 'did:plc:b', 'did:plc:a', 's');
         INSERT INTO token_issued (poll_id, did) VALUES ('p1', 'did:plc:a');",
    )
    .expect("seed");

    // eligibility (poll_id, did) PK rejects a duplicate voter row.
    assert!(
        conn.execute(
            "INSERT INTO eligibility (poll_id, did, base_weight) VALUES ('p1', 'did:plc:a', 9)",
            [],
        )
        .is_err(),
        "duplicate (poll_id, did) rejected by the eligibility PK"
    );

    // delegation (poll_id, from_did) PK rejects a second outgoing delegation.
    assert!(
        conn.execute(
            "INSERT INTO delegation (poll_id, from_did, to_did, assignment_sig) VALUES ('p1', 'did:plc:b', 'did:plc:c', 's2')",
            [],
        )
        .is_err(),
        "a voter may have only one outgoing delegation"
    );

    // token_issued (poll_id, did) PK: issuance happens once per voter.
    assert!(
        conn.execute(
            "INSERT INTO token_issued (poll_id, did) VALUES ('p1', 'did:plc:a')",
            [],
        )
        .is_err(),
        "issuance marker is one-shot per voter"
    );

    // The board's UNIQUE token is the double-spend guard.
    conn.execute(
        "INSERT INTO board_nullifier (token, position) VALUES (x'01', 0)",
        [],
    )
    .expect("first token");
    assert!(
        conn.execute(
            "INSERT INTO board_nullifier (token, position) VALUES (x'01', 1)",
            [],
        )
        .is_err(),
        "a reused token collides on the board (double-spend rejection)"
    );

    // resolved_weight starts NULL (frozen only at open).
    let w: Option<i64> = conn
        .query_row(
            "SELECT resolved_weight FROM eligibility WHERE poll_id='p1' AND did='did:plc:a'",
            [],
            |r| r.get(0),
        )
        .expect("select");
    assert_eq!(w, None, "resolved_weight is NULL until the freeze");
}

// ---------------------------------------------------------------------------
// Turso Database (the decided engine).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn ballot_ddl_executes_and_rows_round_trip_on_turso() {
    let db = turso::Builder::new_local(":memory:")
        .build()
        .await
        .expect("build");
    let conn = db.connect().expect("connect");
    conn.execute_batch(BALLOT_DDL)
        .await
        .expect("ballot DDL on turso");
    conn.execute_batch(BOARD_DDL)
        .await
        .expect("board DDL on turso");

    conn.execute(
        "INSERT INTO poll (id, context_id, question, options) VALUES ('p1', 'c1', 'Q', '[]')",
        (),
    )
    .await
    .expect("insert poll");
    conn.execute(
        "INSERT INTO eligibility (poll_id, did, base_weight, resolved_weight) VALUES ('p1', 'did:plc:a', 1, NULL)",
        (),
    )
    .await
    .expect("insert eligibility");

    // The `open` default applies.
    let mut rows = conn
        .query("SELECT open FROM poll WHERE id = 'p1'", ())
        .await
        .expect("query");
    let row = rows.next().await.expect("next").expect("row");
    let open: i64 = row.get(0).expect("open");
    assert_eq!(open, 1, "poll.open defaults to 1");

    // The board's UNIQUE token is enforced on turso too.
    conn.execute(
        "INSERT INTO board_nullifier (token, position) VALUES (x'02', 0)",
        (),
    )
    .await
    .expect("first token");
    let dup = conn
        .execute(
            "INSERT INTO board_nullifier (token, position) VALUES (x'02', 1)",
            (),
        )
        .await;
    assert!(dup.is_err(), "turso enforces the board token UNIQUE");
}
