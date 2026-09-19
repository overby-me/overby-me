//! Accounts carried over from the interim, and how a person takes theirs over.
//!
//! The interim knew a person by an account id, and here a person is their DID.
//! A carried account keeps its old id as its `did`. No login produces that id,
//! so the account holds its seats, its name and its work, and cannot sign in.
//! Whoever signs in with the address it was registered under takes all of it.
//!
//! The address proves that only because both ends vouch for it: the extractor
//! carries none the interim had not verified, and [`crate::profile`] acts on
//! none that a trusted PDS has not confirmed.

use crate::{AppState, DbError};
use turso::Connection;

pub const LEGACY_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS legacy_account (
  id    TEXT PRIMARY KEY REFERENCES user(did),
  email TEXT NOT NULL                                      -- trimmed and lowercased
);
CREATE INDEX IF NOT EXISTS legacy_account_by_email ON legacy_account(email);
"#;

/// Whether `id` names a carried account and not a person: a seat it holds is
/// still waiting for whoever it belongs to.
pub fn is_carried(id: &str) -> bool {
    !id.starts_with("did:")
}

/// Every column that names an account and may simply be pointed at another.
/// `tests::every_column_that_names_an_account_is_handed_over` reads the schema,
/// so a table added without a line here fails the build's tests, not a person.
const NAMED: &[(&str, &str)] = &[
    ("context", "owner_did"),
    ("document", "owner_did"),
    ("post", "author_did"),
    ("comment", "author_did"),
    ("blob", "owner_did"),
    ("feedback", "owner_did"),
    ("canvas_cell", "painter"),
    ("push_subscription", "did"),
];

/// Columns where a person may appear once per the other columns listed. Where
/// both hold a row the person's own stays and the carried one goes.
const NAMED_ONCE: &[(&str, &str, &[&str])] = &[
    ("document_author", "author_did", &["document_id"]),
    ("reaction", "reactor_did", &["subject_uri", "emoji"]),
    ("speaker_entry", "speaker_did", &["list_id", "kind"]),
    ("canvas_painter", "did", &["canvas_id"]),
    ("feedback_reporter", "reporter", &["feedback_id"]),
];

/// A seat that changed hands: `(context id, member id)`.
pub type Seat = (String, String);

/// Hand `did` every carried account registered under `email`, which the caller
/// has reason to believe is theirs. All of it or none of it. Returns the seats
/// that came with them.
pub async fn adopt(state: &AppState, did: &str, email: &str) -> Result<Vec<Seat>, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query("SELECT id FROM legacy_account WHERE email = ?1", [email])
        .await?;
    let mut accounts: Vec<String> = Vec::new();
    while let Some(row) = rows.next().await? {
        accounts.push(row.get(0)?);
    }
    drop(rows);
    if accounts.is_empty() {
        return Ok(Vec::new());
    }
    let _turn = state.db.write_turn().await;
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    let mut seats = Vec::new();
    for account in &accounts {
        match hand_over(&conn, account, did).await {
            Ok(moved) => seats.extend(moved),
            Err(e) => {
                // Best effort: the error worth reporting is the one that got us here.
                let _ = conn.execute("ROLLBACK", ()).await;
                return Err(e);
            }
        }
    }
    conn.execute("COMMIT", ()).await?;
    Ok(seats)
}

async fn hand_over(conn: &Connection, account: &str, did: &str) -> Result<Vec<Seat>, DbError> {
    let mut rows = conn
        .query(
            "SELECT context_id FROM member WHERE user_did = ?1",
            [account],
        )
        .await?;
    let mut contexts: Vec<String> = Vec::new();
    while let Some(row) = rows.next().await? {
        contexts.push(row.get(0)?);
    }
    drop(rows);

    // Two seats in one context are one person's: both were granted to them, so
    // theirs keeps the better of each grant.
    let theirs = "SELECT 1 FROM member AS old \
                  WHERE old.context_id = member.context_id AND old.user_did = ?2";
    conn.execute(
        &format!(
            "UPDATE member SET \
               role = CASE WHEN EXISTS ({theirs} AND old.role = 'owner') THEN 'owner' ELSE role END, \
               active = CASE WHEN EXISTS ({theirs} AND old.active = 1) THEN 1 ELSE active END, \
               accepted = CASE WHEN EXISTS ({theirs} AND old.accepted = 1) THEN 1 ELSE accepted END \
             WHERE user_did = ?1 AND EXISTS ({theirs})"
        ),
        [did, account],
    )
    .await?;
    conn.execute(
        "DELETE FROM member WHERE user_did = ?2 AND EXISTS \
           (SELECT 1 FROM member AS mine \
            WHERE mine.context_id = member.context_id AND mine.user_did = ?1)",
        [did, account],
    )
    .await?;
    conn.execute(
        "UPDATE member SET user_did = ?1 WHERE user_did = ?2",
        [did, account],
    )
    .await?;

    for (table, column, once_per) in NAMED_ONCE {
        let same = once_per
            .iter()
            .map(|key| format!("mine.{key} = {table}.{key}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE {column} = ?2 AND EXISTS \
                   (SELECT 1 FROM {table} AS mine WHERE mine.{column} = ?1 AND {same})"
            ),
            [did, account],
        )
        .await?;
        conn.execute(
            &format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2"),
            [did, account],
        )
        .await?;
    }
    for (table, column) in NAMED {
        conn.execute(
            &format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2"),
            [did, account],
        )
        .await?;
    }

    // What their PDS did not say, the carried account still may.
    conn.execute(
        "UPDATE user SET \
           display_name = coalesce(display_name, (SELECT display_name FROM user WHERE did = ?2)), \
           avatar_url = coalesce(avatar_url, (SELECT avatar_url FROM user WHERE did = ?2)) \
         WHERE did = ?1",
        [did, account],
    )
    .await?;
    conn.execute("DELETE FROM legacy_account WHERE id = ?1", [account])
        .await?;
    // Refused by the foreign keys while anything still names the account, which
    // undoes the whole handover rather than leave half a person behind.
    conn.execute("DELETE FROM user WHERE did = ?1", [account])
        .await?;
    // After the delete: `legacy_id` is unique, and the carried row held it.
    conn.execute(
        "UPDATE user SET legacy_id = ?2 WHERE did = ?1 AND legacy_id IS NULL",
        [did, account],
    )
    .await?;

    let mut seats = Vec::new();
    for context in contexts {
        let mut rows = conn
            .query(
                "SELECT id FROM member WHERE context_id = ?1 AND user_did = ?2",
                [context.as_str(), did],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            seats.push((context, row.get(0)?));
        }
    }
    Ok(seats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{account_from, apply};
    use crate::router;
    use crate::xrpc::tests::{get_as, post, seeded_state, token_for};
    use axum::http::StatusCode;
    use serde_json::json;

    const CAROL: &str = "did:plc:carol";
    const OLD: &str = "7b0c1c1e-0000-4000-8000-0000000000c4";

    /// Every `REFERENCES user(did)` in the schema, as `(table, column)`.
    fn columns_naming_an_account() -> Vec<(String, String)> {
        let ballot = ballot_store::BALLOT_DDL;
        let schema = [
            wiki_schema::ENTITY_SCHEMA,
            crate::schema::RUNTIME_DDL,
            crate::speak::SPEAK_DDL,
            crate::projector::PROJECTOR_DDL,
            crate::blob::BLOB_DDL,
            crate::push::PUSH_DDL,
            crate::feedback::FEEDBACK_DDL,
            crate::search::SEARCH_DDL,
            crate::canvas::CANVAS_DDL,
            LEGACY_DDL,
            ballot,
        ]
        .join("\n");
        let mut table = String::new();
        let mut found = Vec::new();
        for line in schema.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("CREATE TABLE ") {
                let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
                table = rest
                    .split([' ', '('])
                    .next()
                    .unwrap_or_default()
                    .to_string();
            } else if line.contains("REFERENCES user(did)") {
                let column = line.split(' ').next().unwrap_or_default().to_string();
                found.push((table.clone(), column));
            }
        }
        found
    }

    #[test]
    fn every_column_that_names_an_account_is_handed_over() {
        let found = columns_naming_an_account();
        assert!(found.len() >= 12, "the schema was read: {found:?}");
        for (table, column) in found {
            let handled = (table == "member" && column == "user_did")
                || (table == "legacy_account" && column == "id")
                || NAMED.contains(&(table.as_str(), column.as_str()))
                || NAMED_ONCE
                    .iter()
                    .any(|(t, c, _)| (*t, *c) == (table.as_str(), column.as_str()));
            assert!(
                handled,
                "{table}.{column} names an account, and `hand_over` would leave it behind"
            );
        }
    }

    /// Carol as the interim knew her: an owner of the closed group with a vote
    /// there, an author, a commenter, and someone a crash happened to.
    async fn carried() -> AppState {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(&format!(
            "INSERT INTO user (did, display_name, avatar_url, legacy_id) \
               VALUES ('{OLD}', 'Carol fra HB', 'https://old.example/carol.png', '{OLD}');
             INSERT INTO legacy_account (id, email) VALUES ('{OLD}', 'carol@example.org');
             INSERT INTO member (id, user_did, context_id, role, active, accepted, email) \
               VALUES ('seat-9', '{OLD}', 'c9', 'owner', 1, 1, 'carol@example.org');
             INSERT INTO member (id, user_did, context_id, role, active, accepted) \
               VALUES ('seat-1', '{OLD}', 'c1', 'member', 1, 1);
             INSERT INTO document_author (document_id, author_did, ord) VALUES ('s1', '{OLD}', 0);
             INSERT INTO comment (id, on_id, context_id, author_did, text) \
               VALUES ('k-old', 's1', 'c9', '{OLD}', 'Enig');
             INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
               VALUES ('r-old', 'ks', '{OLD}', '🎉');
             INSERT INTO feedback (id, kind, message, digest, owner_did) \
               VALUES ('fb', 'crash', 'panicked', '00ff', '{OLD}');
             INSERT INTO feedback_reporter (feedback_id, reporter) VALUES ('fb', '{OLD}');
             UPDATE document SET owner_did = '{OLD}' WHERE id = 's1';"
        ))
        .await
        .expect("seed");
        state
    }

    fn carol_at(pds: &str, confirmed: bool) -> crate::profile::PdsAccount {
        let session = json!({
            "did": CAROL, "handle": "carol.example", "email": "Carol@Example.org",
            "emailConfirmed": confirmed
        });
        account_from(CAROL, pds, &session, None)
    }

    async fn one(state: &AppState, sql: &str) -> Option<String> {
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn.query(sql, ()).await.expect("q");
        let row = rows.next().await.expect("next")?;
        match row.get_value(0).expect("value") {
            turso::Value::Text(text) => Some(text),
            turso::Value::Integer(n) => Some(n.to_string()),
            _ => None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn signing_in_with_the_address_takes_the_old_account_over() {
        let state = carried().await;
        let carol = token_for(&state, CAROL).await;
        let closed = "/xrpc/com.example.wiki.getNode?path=closed/secret_minutes";
        let (status, _) = get_as(router(state.clone()), closed, &carol).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "she is nobody here yet");

        let trusted = "https://morel.us-east.host.bsky.network";
        let mut changes = state.changes.subscribe();
        let seats = apply(&state, CAROL, &carol_at(trusted, true)).await;
        assert_eq!(seats.expect("apply"), 2);
        let mut told = Vec::new();
        while let Ok(change) = changes.try_recv() {
            told.push(change.kind);
        }
        assert_eq!(told, ["member", "member", "invitation"]);

        let (status, v) = get_as(router(state.clone()), closed, &carol).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["viewer"]["is_context_owner"], true, "{v}");
        assert_eq!(v["viewer"]["can_vote"], true, "{v}");
        for (what, sql) in [
            (
                "her seats",
                "SELECT count(*) FROM member WHERE user_did = 'did:plc:carol'",
            ),
            (
                "her authorship",
                "SELECT count(*) FROM document_author WHERE author_did = 'did:plc:carol'",
            ),
            (
                "her comment",
                "SELECT count(*) FROM comment WHERE author_did = 'did:plc:carol'",
            ),
            (
                "her reaction",
                "SELECT count(*) FROM reaction WHERE reactor_did = 'did:plc:carol'",
            ),
            (
                "her document",
                "SELECT count(*) FROM document WHERE owner_did = 'did:plc:carol'",
            ),
            (
                "her report",
                "SELECT count(*) FROM feedback WHERE owner_did = 'did:plc:carol'",
            ),
            (
                "her crash",
                "SELECT count(*) FROM feedback_reporter WHERE reporter = 'did:plc:carol'",
            ),
        ] {
            let expected = if what == "her seats" { "2" } else { "1" };
            assert_eq!(one(&state, sql).await.as_deref(), Some(expected), "{what}");
        }
        assert_eq!(
            one(
                &state,
                &format!("SELECT count(*) FROM user WHERE did = '{OLD}'")
            )
            .await,
            Some("0".into()),
            "one person, one row"
        );
        assert_eq!(
            one(&state, "SELECT count(*) FROM legacy_account").await,
            Some("0".into())
        );
        let carol_row =
            "SELECT display_name || '|' || legacy_id FROM user WHERE did = 'did:plc:carol'";
        assert_eq!(
            one(&state, carol_row).await,
            Some(format!("Carol fra HB|{OLD}")),
            "her PDS gave no name, so the one she had is kept"
        );

        assert_eq!(
            apply(&state, CAROL, &carol_at(trusted, true))
                .await
                .expect("again"),
            0,
            "there is nothing left to take"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_address_nobody_vouches_for_takes_nothing_over() {
        let state = carried().await;
        token_for(&state, CAROL).await;
        let trusted = "https://bsky.social";
        for account in [
            carol_at("https://pds.example", true),
            carol_at(trusted, false),
        ] {
            assert_eq!(apply(&state, CAROL, &account).await.expect("apply"), 0);
        }
        assert_eq!(
            one(
                &state,
                &format!("SELECT count(*) FROM member WHERE user_did = '{OLD}'")
            )
            .await,
            Some("2".into())
        );
    }

    /// What the schema test cannot see: a table added at run time, or one it
    /// fails to read. The account must then stay whole rather than half move.
    #[tokio::test(flavor = "current_thread")]
    async fn an_account_something_else_still_names_is_not_taken_apart() {
        let state = carried().await;
        token_for(&state, CAROL).await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(&format!(
            "CREATE TABLE stray (who TEXT REFERENCES user(did));
             INSERT INTO stray (who) VALUES ('{OLD}');"
        ))
        .await
        .expect("seed");
        let account = carol_at("https://bsky.social", true);
        assert!(apply(&state, CAROL, &account).await.is_err());
        assert_eq!(
            one(
                &state,
                &format!("SELECT count(*) FROM member WHERE user_did = '{OLD}'")
            )
            .await,
            Some("2".into()),
            "nothing moved"
        );
        assert_eq!(
            one(&state, "SELECT count(*) FROM legacy_account").await,
            Some("1".into())
        );
    }

    /// She was invited afresh and took the seat before her address was
    /// confirmed. The two seats are one person's, and she is seated once.
    #[tokio::test(flavor = "current_thread")]
    async fn a_seat_held_twice_becomes_one_with_the_better_of_both() {
        let state = carried().await;
        token_for(&state, CAROL).await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO member (id, user_did, context_id, role, active, accepted) \
               VALUES ('seat-new', 'did:plc:carol', 'c9', 'member', 0, 1);
             INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
               VALUES ('r-new', 'ks', 'did:plc:carol', '🎉');",
        )
        .await
        .expect("seed");
        let account = carol_at("https://bsky.social", true);
        assert_eq!(apply(&state, CAROL, &account).await.expect("apply"), 2);
        let seat = "SELECT id || '|' || role || '|' || active FROM member \
                    WHERE context_id = 'c9' AND user_did = 'did:plc:carol'";
        assert_eq!(one(&state, seat).await.as_deref(), Some("seat-new|owner|1"));
        assert_eq!(
            one(&state, "SELECT count(*) FROM member WHERE id = 'seat-9'").await,
            Some("0".into())
        );
        assert_eq!(
            one(
                &state,
                "SELECT count(*) FROM reaction WHERE subject_uri = 'ks'"
            )
            .await,
            Some("1".into()),
            "she reacted once, whichever account it was with"
        );
    }

    /// For someone whose address cannot be vouched for. A link is an owner's to
    /// give, and an owner has a say over their own context only, so it hands
    /// over the seat there and nothing else the old account holds.
    #[tokio::test(flavor = "current_thread")]
    async fn a_claim_link_hands_over_one_seat_and_no_more() {
        let state = carried().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, CAROL).await;
        let link = "/xrpc/com.example.wiki.getMemberClaimLink?member=seat-9";

        let (status, _) = get_as(router(state.clone()), link, &bob).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a member is no owner");
        assert_eq!(
            one(&state, "SELECT claim_token FROM member WHERE id = 'seat-9'").await,
            None,
            "and asking minted nothing"
        );
        let (status, v) = get_as(router(state.clone()), link, &alice).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let token = v["token"].as_str().expect("token").to_string();
        let (_, again) = get_as(router(state.clone()), link, &alice).await;
        assert_eq!(
            again["token"],
            token.as_str(),
            "one link, however often asked for"
        );

        let claim = "/xrpc/com.example.wiki.claimMembership";
        let (status, v) = post(
            router(state.clone()),
            claim,
            Some(&carol),
            json!({"token": token}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, v) = post(
            router(state.clone()),
            claim,
            Some(&bob),
            json!({"token": token}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "the seat is a person's now: {v}"
        );

        assert_eq!(
            one(&state, "SELECT user_did FROM member WHERE id = 'seat-9'")
                .await
                .as_deref(),
            Some(CAROL)
        );
        assert_eq!(
            one(&state, "SELECT user_did FROM member WHERE id = 'seat-1'")
                .await
                .as_deref(),
            Some(OLD),
            "the seat in another context is not this owner's to give"
        );
        assert_eq!(
            one(
                &state,
                &format!("SELECT count(*) FROM comment WHERE author_did = '{OLD}'")
            )
            .await,
            Some("1".into())
        );
    }
}
