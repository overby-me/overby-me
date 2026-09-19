//! Who people are, for the screens that name them: a profile page, and the
//! pickers that find someone to invite or to credit as an author.
//!
//! All of it takes a session. A DID in the `user` table is someone who has
//! signed in here or been named here, which a stranger has no business being
//! told, and the interim asks the same.

use crate::AppState;
use crate::db::DbError;
use crate::search::{NodeRef, fold};
use crate::session::Caller;
use crate::xrpc::{err, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

/// A picker shows a handful, and the next keystroke narrows it.
const MAX_MATCHES: usize = 10;

/// Rows a search reads. A name is matched here and not in SQL, which folds case
/// for ASCII only and would not find `Åse` for `åse`; an organisation's people
/// are a few thousand rows.
const MAX_SCAN: i64 = 50_000;

#[derive(Debug, Serialize)]
pub struct Person {
    pub did: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DidParam {
    pub did: String,
}

/// `com.example.wiki.getProfile`: who a DID is, for a profile page.
pub async fn get_profile(
    State(state): State<AppState>,
    _caller: Caller,
    Query(p): Query<DidParam>,
) -> Response {
    match crate::Store::new(state.db.clone()).read_user(&p.did).await {
        Ok(Some(user)) => (
            StatusCode::OK,
            Json(Person {
                did: user.did,
                handle: user.handle,
                display_name: user.display_name,
                avatar_url: user.avatar_url,
            }),
        )
            .into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "nobody by that DID here"),
        Err(e) => write_failed("getProfile", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    /// Also the groups and events by that name, for crediting one as an author.
    #[serde(default)]
    pub contexts: bool,
}

/// `com.example.wiki.searchPeople`: the people whose name or handle contains
/// the query, and with `contexts` the groups the caller may read that do. For a
/// picker: ten at most, and nothing for fewer than two letters.
pub async fn search_people(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Query(p): Query<SearchParams>,
) -> Response {
    let needle = fold(p.q.trim());
    if needle.chars().count() < 2 {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "people": [], "contexts": [] })),
        )
            .into_response();
    }
    let found = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT did, handle, display_name, avatar_url FROM user \
                     WHERE handle IS NOT NULL OR display_name IS NOT NULL \
                     ORDER BY display_name LIMIT {MAX_SCAN}"
                ),
                (),
            )
            .await?;
        let text = |row: &turso::Row, i: usize| match row.get_value(i) {
            Ok(turso::Value::Text(s)) => Some(s),
            _ => None,
        };
        let mut people = Vec::new();
        while let Some(row) = rows.next().await? {
            let (handle, name) = (text(&row, 1), text(&row, 2));
            let matches = [&handle, &name]
                .into_iter()
                .flatten()
                .any(|field| fold(field).contains(&needle));
            if matches {
                people.push(Person {
                    did: row.get(0)?,
                    handle,
                    display_name: name,
                    avatar_url: text(&row, 3),
                });
                if people.len() == MAX_MATCHES {
                    break;
                }
            }
        }
        drop(rows);
        let mut contexts: Vec<NodeRef> = Vec::new();
        if p.contexts {
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT c.id, c.kind, c.name, c.path FROM search_index s \
                         JOIN context c ON c.id = s.node_id \
                         WHERE c.deleted_at IS NULL AND c.kind <> 'site' AND {} \
                           AND instr(s.title, ?2) > 0 ORDER BY c.name LIMIT {MAX_MATCHES}",
                        crate::authz::readable_context("c", 1)
                    ),
                    [did.as_str(), needle.as_str()],
                )
                .await?;
            while let Some(row) = rows.next().await? {
                contexts.push(NodeRef {
                    node: "context",
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    title: row.get(2)?,
                    path: row.get(3)?,
                });
            }
        }
        Ok::<_, DbError>((people, contexts))
    };
    match found.await {
        Ok((people, contexts)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "people": people, "contexts": contexts })),
        )
            .into_response(),
        Err(e) => write_failed("searchPeople", e),
    }
}

#[cfg(test)]
mod tests {
    use crate::router;
    use crate::xrpc::tests::{get, get_as, seeded_state, token_for};
    use axum::http::StatusCode;

    #[tokio::test(flavor = "current_thread")]
    async fn a_picker_finds_people_and_groups_by_name_whatever_the_case() {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did, handle, display_name) \
               VALUES ('did:plc:aase', 'aase.example', 'Åse Ørsted');
             INSERT INTO user (did) VALUES ('did:plc:nameless');",
        )
        .await
        .expect("seed");
        let bob = token_for(&state, "did:plc:bob").await;
        let find = |q: &'static str| {
            let (state, bob) = (state.clone(), bob.clone());
            async move {
                let uri = format!("/xrpc/com.example.wiki.searchPeople?q={q}&contexts=true");
                get_as(router(state), &uri, &bob).await.1
            }
        };
        let found = find("åse%20ø").await;
        assert_eq!(found["people"][0]["did"], "did:plc:aase", "{found}");
        assert_eq!(
            find("ALICE.TE").await["people"][0]["did"],
            "did:plc:alice",
            "by handle"
        );
        assert_eq!(
            find("a").await["people"],
            serde_json::json!([]),
            "one letter"
        );

        // Groups the caller may read, for crediting one as an author.
        let groups = find("group").await;
        let names: Vec<&str> = groups["contexts"]
            .as_array()
            .expect("contexts")
            .iter()
            .map(|c| c["title"].as_str().expect("title"))
            .collect();
        assert_eq!(names, ["Closed Group", "Group One"], "{groups}");
        let mallory = token_for(&state, "did:plc:mallory").await;
        let (_, outside) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.searchPeople?q=group&contexts=true",
            &mallory,
        )
        .await;
        assert_eq!(
            outside["contexts"].as_array().expect("contexts").len(),
            1,
            "{outside}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn who_is_known_here_is_told_to_the_signed_in_only() {
        let state = seeded_state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let profile = "/xrpc/com.example.wiki.getProfile?did=did:plc:alice";
        let (status, v) = get_as(router(state.clone()), profile, &bob).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["display_name"], "Alice");
        assert_eq!(
            get(router(state.clone()), profile).await.0,
            StatusCode::UNAUTHORIZED
        );
        let search = "/xrpc/com.example.wiki.searchPeople?q=alice";
        assert_eq!(
            get(router(state.clone()), search).await.0,
            StatusCode::UNAUTHORIZED
        );
        let nobody = "/xrpc/com.example.wiki.getProfile?did=did:plc:nobody";
        assert_eq!(
            get_as(router(state.clone()), nobody, &bob).await.0,
            StatusCode::NOT_FOUND
        );
    }
}
