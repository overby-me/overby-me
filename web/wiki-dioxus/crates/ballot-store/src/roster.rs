//! The PRIVATE, org-authoritative half of the ballot path (rewrite kickoff item
//! 13): the always-private eligibility / delegation / token-issuance tables and
//! the freeze-at-open routine that resolves delegations into a FROZEN
//! `resolved_weight` before the poll opens. This half never touches the public
//! board (that is `board.rs`); it decides WHO may vote and with what weight, and
//! records THAT a voter was issued tokens (never the tokens, which would relink
//! a later spend to the DID).

use ballot_spec::{Did, EligibilityRoster};
use turso::{Connection, Value};

/// The private ballot DDL, matching `docs/atproto-domain-model.md`. `poll` is
/// the FK target the other three reference. `board_entry` is INTENTIONALLY
/// excluded: the public board lives in `board.rs` in its opaque provisional
/// form, not these named columns. The `did` columns reference `user(did)` in the
/// AppView's combined schema; that FK is omitted HERE so this DDL validates
/// standalone (the `user` table is the separate entity schema).
pub const BALLOT_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS poll (
  id            TEXT PRIMARY KEY,
  context_id    TEXT NOT NULL,
  question      TEXT NOT NULL,
  options       TEXT NOT NULL,                           -- JSON array of strings
  open          INTEGER NOT NULL DEFAULT 1,
  secret        INTEGER NOT NULL DEFAULT 0,
  issuer_pubkey TEXT,                                    -- published before open; dropped at close
  created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS eligibility (
  poll_id         TEXT NOT NULL REFERENCES poll(id),
  did             TEXT NOT NULL,
  base_weight     INTEGER NOT NULL DEFAULT 1,
  resolved_weight INTEGER,                               -- set at open; NULL until frozen
  PRIMARY KEY (poll_id, did)
);
CREATE TABLE IF NOT EXISTS delegation (
  poll_id        TEXT NOT NULL REFERENCES poll(id),
  from_did       TEXT NOT NULL,
  to_did         TEXT NOT NULL,
  assignment_sig TEXT NOT NULL,
  PRIMARY KEY (poll_id, from_did)                        -- one outgoing delegation per voter
);
CREATE TABLE IF NOT EXISTS token_issued (
  poll_id   TEXT NOT NULL REFERENCES poll(id),
  did       TEXT NOT NULL,
  issued_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (poll_id, did)                             -- issuance happens once per voter
);
"#;

/// Apply the private ballot schema to `conn`.
pub async fn init_schema(conn: &Connection) -> Result<(), turso::Error> {
    conn.execute_batch(BALLOT_DDL).await
}

/// Freeze eligibility at poll open: read the poll's `base_weight` and
/// `delegation` rows, resolve them via `ballot_spec::EligibilityRoster::resolve`
/// (the property-tested D1-D3 rules: transitive chains, cycles/ineligible hops
/// void, weight conserved), and write each voter's `resolved_weight` ONCE.
///
/// Idempotent: if any row for the poll is already frozen (`resolved_weight` set),
/// this is a no-op returning `false` — re-freezing an open poll must never move
/// weight (delegation changes after open do not count). Returns `true` when it
/// froze.
pub async fn freeze_at_open(conn: &Connection, poll_id: &str) -> Result<bool, turso::Error> {
    if already_frozen(conn, poll_id).await? {
        return Ok(false);
    }

    // Load the roster (base weights + delegation edges) from the poll's tables.
    let mut roster = EligibilityRoster::default();
    let mut rows = conn
        .query(
            "SELECT did, base_weight FROM eligibility WHERE poll_id = ?1",
            [poll_id],
        )
        .await?;
    while let Some(r) = rows.next().await? {
        let did: String = r.get(0)?;
        let base: i64 = r.get(1)?;
        roster.base_weight.insert(Did(did), base as u64);
    }
    let mut rows = conn
        .query(
            "SELECT from_did, to_did FROM delegation WHERE poll_id = ?1",
            [poll_id],
        )
        .await?;
    while let Some(r) = rows.next().await? {
        let from: String = r.get(0)?;
        let to: String = r.get(1)?;
        roster.delegation.insert(Did(from), Did(to));
    }

    let resolved = roster.resolve();
    for (did, weight) in &resolved.resolved_weight {
        conn.execute(
            "UPDATE eligibility SET resolved_weight = ?1 WHERE poll_id = ?2 AND did = ?3",
            vec![
                Value::Integer(*weight as i64),
                Value::Text(poll_id.to_string()),
                Value::Text(did.0.clone()),
            ],
        )
        .await?;
    }
    Ok(true)
}

async fn already_frozen(conn: &Connection, poll_id: &str) -> Result<bool, turso::Error> {
    let mut rows = conn
        .query(
            "SELECT count(*) FROM eligibility WHERE poll_id = ?1 AND resolved_weight IS NOT NULL",
            [poll_id],
        )
        .await?;
    let row = rows.next().await?.expect("count returns a row");
    Ok(row.get::<i64>(0)? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballot_spec::{TokenIssuer, finalize_token, request_token};

    async fn seeded_poll() -> Connection {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        let conn = db.connect().expect("connect");
        init_schema(&conn).await.expect("schema");
        conn.execute_batch(
            "INSERT INTO poll (id, context_id, question, options) \
               VALUES ('p1', 'c1', 'Farve?', '[\"Roed\",\"Groen\"]');
             INSERT INTO eligibility (poll_id, did, base_weight) VALUES ('p1', 'did:plc:a', 1);
             INSERT INTO eligibility (poll_id, did, base_weight) VALUES ('p1', 'did:plc:b', 1);
             INSERT INTO eligibility (poll_id, did, base_weight) VALUES ('p1', 'did:plc:c', 1);
             -- b delegates to a; a and c vote themselves.
             INSERT INTO delegation (poll_id, from_did, to_did, assignment_sig) \
               VALUES ('p1', 'did:plc:b', 'did:plc:a', 'sig');",
        )
        .await
        .expect("seed");
        conn
    }

    async fn resolved_weight(conn: &Connection, did: &str) -> Option<i64> {
        let mut rows = conn
            .query(
                "SELECT resolved_weight FROM eligibility WHERE poll_id = 'p1' AND did = ?1",
                [did],
            )
            .await
            .expect("q");
        let row = rows.next().await.expect("next").expect("row");
        match row.get_value(0).expect("val") {
            Value::Integer(n) => Some(n),
            _ => None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn freeze_resolves_delegation_and_conserves_weight() {
        let conn = seeded_poll().await;
        // Before freeze: NULL.
        assert_eq!(resolved_weight(&conn, "did:plc:a").await, None);

        let froze = freeze_at_open(&conn, "p1").await.expect("freeze");
        assert!(froze, "first freeze writes weights");

        // b's weight moved to a (delegation); a=2, b=0, c=1. Total conserved = 3.
        assert_eq!(resolved_weight(&conn, "did:plc:a").await, Some(2));
        assert_eq!(resolved_weight(&conn, "did:plc:b").await, Some(0));
        assert_eq!(resolved_weight(&conn, "did:plc:c").await, Some(1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refreezing_an_open_poll_is_a_noop() {
        let conn = seeded_poll().await;
        assert!(freeze_at_open(&conn, "p1").await.expect("freeze"));

        // A late delegation change after open must NOT move weight.
        conn.execute(
            "INSERT INTO delegation (poll_id, from_did, to_did, assignment_sig) \
             VALUES ('p1', 'did:plc:a', 'did:plc:c', 'late')",
            (),
        )
        .await
        .expect("late delegation");

        let froze_again = freeze_at_open(&conn, "p1").await.expect("re-freeze");
        assert!(!froze_again, "re-freeze is a no-op");
        // a still holds 2 (the frozen value), unaffected by the late delegation.
        assert_eq!(resolved_weight(&conn, "did:plc:a").await, Some(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blind_sign_issuance_produces_a_verifiable_token() {
        // The issuance crypto exercised as a unit, fed by request_token(): the
        // org blind-signs a voter's blinded token WITHOUT seeing the nullifier,
        // and the finalized token verifies under the poll's issuer pubkey.
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let pk = issuer.public_key();
        let req = request_token(pk).expect("request");
        let blind_sig = issuer
            .blind_sign(&req.blinding.blind_message)
            .expect("blind sign");
        let signature = finalize_token(pk, &req, &blind_sig).expect("finalize");
        // The finalized unit token verifies (the same check the board runs).
        assert!(
            pk.verify(&signature, req.blinding.msg_randomizer, &req.nullifier)
                .is_ok(),
            "issued token verifies under the poll issuer key"
        );
    }
}
