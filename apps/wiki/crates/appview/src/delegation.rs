//! Giving one's vote to another member: a standing delegation within a context.
//!
//! The roster has resolved delegations since it was written
//! (`ballot_spec::EligibilityRoster::resolve`: chains follow through, a cycle
//! or a delegate without voting rights is void, weight is conserved). Nothing
//! wrote one. Here a member with voting rights names another to cast theirs,
//! until they take it back; a poll copies the delegations standing as it opens
//! and freezes them, so one made or taken back later moves nothing in it.
//!
//! What stands behind an assignment is the delegator's own session: only they
//! can make or undo theirs. No member holds a key to sign one with, so what is
//! recorded is how it was authorized, and what keeps it honest is that both of
//! its ends see it, the context's owners see them all, and a poll tells every
//! voter the weight it froze for them.

use crate::AppState;
use crate::authz::Authz;
use crate::db::DbError;
use crate::live::Topic;
use crate::session::Caller;
use crate::xrpc::{err, forbidden, invalid, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use turso::{Connection, Value};

pub const DELEGATION_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS standing_delegation (
  context_id TEXT NOT NULL REFERENCES context(id),
  from_did   TEXT NOT NULL REFERENCES user(did),
  to_did     TEXT NOT NULL REFERENCES user(did),
  made_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (context_id, from_did)                       -- one vote, given once
);
CREATE INDEX IF NOT EXISTS standing_delegation_to ON standing_delegation(context_id, to_did);
"#;

/// Copy the delegations standing in `context_id` into `poll_id`'s roster, to be
/// resolved and frozen with it. `assignment_sig` says how each was authorized:
/// by its delegator's session, at the time given.
pub async fn carry_into(
    conn: &Connection,
    poll_id: &str,
    context_id: &str,
) -> Result<(), turso::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO delegation (poll_id, from_did, to_did, assignment_sig) \
         SELECT ?1, from_did, to_did, 'session:' || made_at FROM standing_delegation \
         WHERE context_id = ?2",
        [poll_id, context_id],
    )
    .await
    .map(|_| ())
}

#[derive(Debug, Deserialize)]
pub struct SetBody {
    pub context_id: String,
    /// Who casts the caller's vote. Absent or null takes it back.
    #[serde(default)]
    pub to_did: Option<String>,
}

/// `wiki.radikal.setDelegation` (procedure): the caller gives their vote in
/// a context to another member with voting rights, or takes it back.
pub async fn set_delegation(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<SetBody>,
) -> Response {
    let what = "setDelegation";
    let authz = Authz::new(state.db.clone());
    match authz.is_active_member(&body.context_id, &did).await {
        Ok(true) => {}
        Ok(false) => match crate::Store::new(state.db.clone())
            .read_context(&body.context_id, Some(&did))
            .await
        {
            Ok(Some(_)) => return forbidden("you hold no vote here to give"),
            Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such context"),
            Err(e) => return write_failed(what, e),
        },
        Err(e) => return write_failed(what, e),
    }
    let to = body
        .to_did
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    if let Some(to) = to {
        if to == did {
            return invalid("your vote is yours already");
        }
        // Someone who has never signed in could not cast it, and the vote would
        // be lost with them: the roster counts a carried account as eligible.
        if crate::legacy::is_carried(to) {
            return invalid("they have not signed in here yet, so they could not cast it");
        }
        match authz.is_active_member(&body.context_id, to).await {
            Ok(true) => {}
            Ok(false) => return invalid("they hold no vote here, so they could not cast yours"),
            Err(e) => return write_failed(what, e),
        }
    }
    let written = async {
        let conn = state.db.acquire().await?;
        match to {
            Some(to) => {
                conn.execute(
                    "INSERT INTO standing_delegation (context_id, from_did, to_did) \
                     VALUES (?1, ?2, ?3) \
                     ON CONFLICT(context_id, from_did) DO UPDATE SET to_did = excluded.to_did, \
                       made_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                    [body.context_id.as_str(), did.as_str(), to],
                )
                .await?;
            }
            None => {
                conn.execute(
                    "DELETE FROM standing_delegation WHERE context_id = ?1 AND from_did = ?2",
                    [body.context_id.as_str(), did.as_str()],
                )
                .await?;
            }
        }
        Ok::<_, DbError>(())
    };
    match written.await {
        Ok(()) => {
            state.publish(Topic::Context(body.context_id.clone()), "delegation", &did);
            (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Serialize)]
pub struct DelegationView {
    pub from_did: String,
    pub to_did: String,
    pub made_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ContextParam {
    pub context: String,
}

/// `wiki.radikal.listDelegations`: the delegations standing in a context
/// that are the caller's to see. Their own, given and received; all of them for
/// an owner of the context, who chairs its votes.
pub async fn list_delegations(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Query(p): Query<ContextParam>,
) -> Response {
    let what = "listDelegations";
    let membership = match crate::xrpc::member_of(&state, &p.context, &did, what).await {
        Ok(membership) => membership,
        Err(refusal) => return refusal,
    };
    let all = crate::xrpc::owns(membership);
    let read = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT from_did, to_did, made_at FROM standing_delegation \
                 WHERE context_id = ?1 AND (?2 = 1 OR from_did = ?3 OR to_did = ?3) \
                 ORDER BY made_at, from_did",
                vec![
                    Value::Text(p.context.clone()),
                    Value::Integer(i64::from(all)),
                    Value::Text(did.clone()),
                ],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(DelegationView {
                from_did: row.get(0)?,
                to_did: row.get(1)?,
                made_at: row.get(2)?,
            });
        }
        Ok::<_, DbError>(out)
    };
    let delegations = match read.await {
        Ok(delegations) => delegations,
        Err(e) => return write_failed(what, e),
    };
    let dids: std::collections::BTreeSet<String> = delegations
        .iter()
        .flat_map(|d| [d.from_did.clone(), d.to_did.clone()])
        .collect();
    let profiles = match crate::Store::new(state.db.clone()).profiles(&dids).await {
        Ok(profiles) => profiles,
        Err(e) => return write_failed(what, e),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "delegations": delegations, "all": all, "profiles": profiles })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use crate::router;
    use crate::xrpc::tests::{get_as, join, post, seeded_state, token_for};
    use axum::http::StatusCode;
    use serde_json::json;

    const SET: &str = "/xrpc/wiki.radikal.setDelegation";
    const LIST: &str = "/xrpc/wiki.radikal.listDelegations?context=c9";

    /// c9 with a motion to vote on: alice chairs, bob and carol hold a vote
    /// each, and ivan is a member without one.
    async fn assembly() -> crate::AppState {
        let state = seeded_state().await;
        token_for(&state, "did:plc:carol").await;
        join(&state, "did:plc:carol", "c9").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
             VALUES ('mo1', 'c9', 'c9', 'policy', 'Motion One', 'motion-one', 'closed/motion-one')",
            (),
        )
        .await
        .expect("motion");
        state
    }

    async fn give(
        state: &crate::AppState,
        who: &str,
        to: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let body = json!({ "context_id": "c9", "to_did": to });
        post(router(state.clone()), SET, Some(who), body).await
    }

    async fn open(state: &crate::AppState, chair: &str) -> String {
        let body =
            json!({"parent_id": "mo1", "title": "Motion One", "options": ["for", "against"]});
        let uri = "/xrpc/wiki.radikal.openPoll";
        let (status, v) = post(router(state.clone()), uri, Some(chair), body).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v["id"].as_str().expect("id").to_string()
    }

    async fn weight(state: &crate::AppState, poll: &str, who: &str) -> i64 {
        let uri = format!("/xrpc/wiki.radikal.getPoll?id={poll}");
        let (_, v) = get_as(router(state.clone()), &uri, who).await;
        v["viewer"]["weight"].as_i64().expect("a weight")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_vote_given_away_is_cast_by_whoever_it_was_given_to() {
        let state = assembly().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, "did:plc:carol").await;
        // A chain follows through: carol's goes to bob, and with bob's to alice.
        assert_eq!(
            give(&state, &carol, Some("did:plc:bob")).await.0,
            StatusCode::OK
        );
        assert_eq!(
            give(&state, &bob, Some("did:plc:alice")).await.0,
            StatusCode::OK
        );

        let poll = open(&state, &alice).await;
        assert_eq!(weight(&state, &poll, &alice).await, 3);
        assert_eq!(weight(&state, &poll, &bob).await, 0);
        let cast = "/xrpc/wiki.radikal.castOpenBallot";
        let ballot = json!({ "poll": poll, "choices": [0] });
        let (status, v) = post(router(state.clone()), cast, Some(&bob), ballot.clone()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a vote given away was cast twice: {v}"
        );
        let (status, v) = post(router(state.clone()), cast, Some(&alice), ballot).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let uri = format!("/xrpc/wiki.radikal.getPoll?id={poll}");
        let (_, tally) = get_as(router(state.clone()), &uri, &alice).await;
        assert_eq!(
            (&tally["ballots"], &tally["eligible"]),
            (&json!(3), &json!(3))
        );
        assert_eq!(tally["counts"], json!([3, 0]));

        // Taken back while that poll is open: it moves nothing there, and the
        // next poll is hers to vote in again.
        assert_eq!(give(&state, &carol, None).await.0, StatusCode::OK);
        assert_eq!(weight(&state, &poll, &carol).await, 0);
        let close = "/xrpc/wiki.radikal.closePoll";
        post(
            router(state.clone()),
            close,
            Some(&alice),
            json!({ "id": poll }),
        )
        .await;
        let next = open(&state, &alice).await;
        assert_eq!(weight(&state, &next, &carol).await, 1);
        assert_eq!(weight(&state, &next, &alice).await, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_vote_goes_only_from_and_to_someone_who_holds_one() {
        let state = assembly().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let ivan = token_for(&state, "did:plc:ivan").await;
        let zoe = token_for(&state, "did:plc:zoe").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did) VALUES ('0b9f-an-interim-account');
             INSERT INTO member (id, user_did, context_id, role, active) \
               VALUES ('m-old', '0b9f-an-interim-account', 'c9', 'member', 1);",
        )
        .await
        .expect("a carried seat");

        assert_eq!(
            give(&state, &ivan, Some("did:plc:bob")).await.0,
            StatusCode::FORBIDDEN,
            "a member without voting rights gave a vote away"
        );
        assert_eq!(
            give(&state, &zoe, Some("did:plc:bob")).await.0,
            StatusCode::NOT_FOUND,
            "a closed group answered someone outside it"
        );
        for (to, why) in [
            ("did:plc:bob", "to themselves"),
            ("did:plc:ivan", "to someone with no vote to cast it with"),
            ("did:plc:nobody", "to a stranger"),
            (
                "0b9f-an-interim-account",
                "to an account nobody can sign in as",
            ),
        ] {
            let (status, v) = give(&state, &bob, Some(to)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_delegation_is_seen_by_its_two_ends_and_by_the_chair() {
        let state = assembly().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let zoe = token_for(&state, "did:plc:zoe").await;
        give(&state, &bob, Some("did:plc:alice")).await;

        let seen = |v: &serde_json::Value| v["delegations"].as_array().expect("a list").len();
        let (_, bobs) = get_as(router(state.clone()), LIST, &bob).await;
        assert_eq!((seen(&bobs), &bobs["all"]), (1, &json!(false)));
        assert_eq!(bobs["delegations"][0]["to_did"], "did:plc:alice");
        assert_eq!(bobs["profiles"]["did:plc:alice"]["display_name"], "Alice");
        let (_, carols) = get_as(router(state.clone()), LIST, &carol).await;
        assert_eq!(
            seen(&carols),
            0,
            "who bob gave his vote to is not carol's to know"
        );
        give(&state, &carol, Some("did:plc:bob")).await;
        let (_, chairs) = get_as(router(state.clone()), LIST, &alice).await;
        assert_eq!((seen(&chairs), &chairs["all"]), (2, &json!(true)));
        let (status, _) = get_as(router(state.clone()), LIST, &zoe).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Put out of the group, carol takes what she gave with her.
        let remove = "/xrpc/wiki.radikal.removeMember";
        let seat = json!({ "id": "m-did:plc:carol-c9" });
        let (status, v) = post(router(state.clone()), remove, Some(&alice), seat).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (_, chairs) = get_as(router(state.clone()), LIST, &alice).await;
        assert_eq!(seen(&chairs), 1, "{chairs}");
        join(&state, "did:plc:carol", "c9").await;
        give(&state, &carol, Some("did:plc:bob")).await;

        // Given again, it moves: one vote, given once.
        give(&state, &bob, Some("did:plc:carol")).await;
        let (_, bobs) = get_as(router(state.clone()), LIST, &bob).await;
        let given: Vec<&serde_json::Value> = bobs["delegations"]
            .as_array()
            .expect("a list")
            .iter()
            .filter(|d| d["from_did"] == "did:plc:bob")
            .collect();
        assert_eq!(given.len(), 1, "{bobs}");
        assert_eq!(given[0]["to_did"], "did:plc:carol");
        assert_eq!(seen(&bobs), 2, "and the one he was given is his to see too");
    }
}
