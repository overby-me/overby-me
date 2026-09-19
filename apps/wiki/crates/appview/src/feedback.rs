//! Feedback, bug reports and crash reports. A reader, signed in or not, says
//! what happened, or the app says it for them when it has died; an owner of the
//! site reads it all, and everyone else reads what they sent.
//!
//! The interim files a report as a `wiki/feedback` node under the root node,
//! whose `data` holds all of this; here it is a table. The rules are carried
//! over from `backend/src/feedback.rs`: a stack is symbolicated on the way in,
//! because the reader's browser has no DWARF to resolve it against, and a crash
//! that has been seen before becomes a count on the row that is already there.

use crate::AppState;
use crate::db::DbError;
use crate::session::{Caller, MaybeCaller};
use crate::xrpc::{err, forbidden, invalid, write_failed};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Mutex;
use turso::Value;

pub const FEEDBACK_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS feedback (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('bug','feature','other','crash','error')),
  message     TEXT NOT NULL,
  path        TEXT NOT NULL DEFAULT '',
  app_version TEXT NOT NULL DEFAULT '',
  build       TEXT NOT NULL DEFAULT '',                  -- the commit the bundle was built from
  user_agent  TEXT NOT NULL DEFAULT '',
  image       TEXT,                                      -- a screenshot's blob id
  digest      TEXT,                                      -- what a crash is folded by; NULL for a person's account
  seen        INTEGER NOT NULL DEFAULT 1,
  owner_did   TEXT REFERENCES user(did),                 -- NULL: sent by someone not signed in
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS feedback_by_digest ON feedback(digest) WHERE digest IS NOT NULL;
-- Everyone who has hit a folded crash: a DID, or 'anonymous' once for all of
-- those with no account.
CREATE TABLE IF NOT EXISTS feedback_reporter (
  feedback_id TEXT NOT NULL REFERENCES feedback(id),
  reporter    TEXT NOT NULL,
  PRIMARY KEY (feedback_id, reporter)
);
"#;

/// Cap on the message as it ARRIVES (matches the client-side maxlength), so a
/// runaway paste cannot bloat a row.
const MAX_MESSAGE: usize = 4000;

/// Cap on the message as STORED, which has to be far larger, because
/// symbolication multiplies it: one wasm frame becomes every function inlined
/// into it, so a stack that arrives as twenty lines can leave as a hundred.
/// Capping the result at [`MAX_MESSAGE`] cut the resolved stack off partway
/// through, losing precisely the outer frames that name the component.
const MAX_STORED: usize = 32_000;

const MAX_FIELD: usize = 500;

/// Reports a minute taken from people who are not signed in, all of them
/// together. A crash before sign-in has to be reportable, so this cannot ask
/// for a session; it can keep a flood from becoming the database.
const ANONYMOUS_PER_MINUTE: usize = 10;

/// Stands in for a reporter with no account, so anonymous sightings count once
/// rather than once each.
const ANONYMOUS: &str = "anonymous";

/// What this module keeps between requests.
#[derive(Default)]
pub struct Shared {
    /// When each recent anonymous report arrived.
    anonymous_lately: Mutex<Vec<u64>>,
}

impl Shared {
    /// Whether one more anonymous report fits in this minute, counting it if so.
    fn anonymous_fits(&self, now: u64) -> bool {
        let mut lately = self.anonymous_lately.lock().expect("anonymous");
        lately.retain(|at| now.saturating_sub(*at) < 60);
        if lately.len() >= ANONYMOUS_PER_MINUTE {
            return false;
        }
        lately.push(now);
        true
    }
}

/// Cut `text` to at most `max` bytes without splitting a character.
///
/// `String::truncate` panics when the index lands mid-character, and a Danish
/// message reaches that on any æ, ø or å sitting across the limit.
fn clamp(mut text: String, max: usize) -> String {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Identify a crash by WHERE it happened, not by the exact bytes of its report.
///
/// Built from the panic text plus the source locations of the resolved frames,
/// deliberately ignoring wasm offsets, function indices and asset URLs. Those
/// change with every build, so hashing the whole message would start a fresh row
/// at each deploy and the count would reset exactly when a recurring crash
/// becomes most interesting.
///
/// A report with nothing resolved falls back to the whole message, which at
/// least groups identical unresolved reports together.
fn crash_digest(message: &str) -> String {
    let mut material = String::new();
    let mut resolved = 0usize;
    for line in message.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("at ") {
            // Keep only the trailing `(file:line)`; the function name carries
            // monomorphised type parameters that shift between builds.
            if let Some(open) = rest.rfind(" (")
                && rest.ends_with(')')
            {
                material.push_str(&rest[open + 2..rest.len() - 1]);
                material.push('\n');
                resolved += 1;
            }
        } else if !trimmed.contains("wasm-function[") && !trimmed.contains("://") {
            // The panic itself and its message; everything else on these lines
            // is an engine artefact.
            material.push_str(trimmed);
            material.push('\n');
        }
    }
    if resolved == 0 {
        material = message.to_string();
    }
    // FNV-1a: the interim's, so a crash seen before the cutover keeps its row.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in material.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Deserialize)]
pub struct SubmitBody {
    #[serde(default)]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub path: String,
    /// The crate version.
    #[serde(default)]
    pub app: String,
    /// The commit the bundle was built from, which is what ties a report to code.
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub ua: String,
    /// A screenshot, as the id `uploadBlob` gave it.
    #[serde(default)]
    pub image: Option<String>,
}

/// `com.example.wiki.submitFeedback` (procedure): file a report. A session is
/// taken if there is one and not asked for: someone who cannot sign in has the
/// most to report.
pub async fn submit_feedback(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Json(body): Json<SubmitBody>,
) -> Response {
    let what = "submitFeedback";
    let message = body.message.trim().to_string();
    if message.is_empty() {
        return invalid("a report needs a message");
    }
    if caller.did().is_none() && !state.feedback.anonymous_fits(crate::util::now_secs()) {
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "RateLimited",
            "too many reports from people who are not signed in; try again in a minute",
        );
    }
    let kind = match body.kind.as_str() {
        known @ ("bug" | "feature" | "crash" | "error") => known,
        _ => "other",
    };
    // A screenshot is the reporter's own upload. Whoever runs the site is let
    // read what a report shows (`blob::readable`), so a report must not be a
    // way to show them somebody else's file.
    if let Some(image) = body.image.as_deref().filter(|id| !id.is_empty()) {
        match crate::blob::meta(&state, image).await {
            Ok(Some(blob))
                if blob.owner_did.as_deref() == caller.did() && caller.did().is_some() => {}
            Ok(_) => return invalid("no such screenshot of yours"),
            Err(e) => return write_failed(what, e),
        }
    }
    // Bounded BEFORE it is resolved: the cap exists to stop a runaway paste, and
    // applying it afterwards would instead have trimmed the work.
    let message = clamp(message, MAX_MESSAGE);
    // A crash report carries the panic's stack in its message. Ordinary feedback
    // has no wasm frames and passes through untouched.
    let message =
        crate::symbolicate::resolve_stack(&state.http, &state.config.app_origin, &message).await;
    let message = clamp(message, MAX_STORED);
    // Automatic reports fold; a person's account of what happened does not.
    let digest = matches!(kind, "crash" | "error").then(|| crash_digest(&message));
    let reporter = caller.did().unwrap_or(ANONYMOUS);
    let short = |field: String| Value::Text(clamp(field, MAX_FIELD));

    let filed = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        let id = format!("f-{}", crate::util::random_token(16));
        // A crash seen before keeps its first message and gains a sighting.
        let mut rows = conn
            .query(
                "INSERT INTO feedback \
                   (id, kind, message, path, app_version, build, user_agent, image, digest, owner_did) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT(digest) WHERE digest IS NOT NULL DO UPDATE SET \
                   seen = seen + 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                 RETURNING id",
                vec![
                    Value::Text(id),
                    Value::Text(kind.to_string()),
                    Value::Text(message.clone()),
                    short(body.path.clone()),
                    short(body.app.clone()),
                    short(body.commit.clone()),
                    short(body.ua.clone()),
                    body.image.clone().map_or(Value::Null, short),
                    digest.clone().map_or(Value::Null, Value::Text),
                    caller
                        .did()
                        .map_or(Value::Null, |did| Value::Text(did.to_string())),
                ],
            )
            .await?;
        let filed_as: String = match rows.next().await? {
            Some(row) => row.get(0)?,
            None => return Ok(None),
        };
        drop(rows);
        if digest.is_some() {
            conn.execute(
                "INSERT OR IGNORE INTO feedback_reporter (feedback_id, reporter) VALUES (?1, ?2)",
                [filed_as.as_str(), reporter],
            )
            .await?;
        }
        Ok::<_, DbError>(Some(filed_as))
    };
    match filed.await {
        Ok(Some(id)) => (StatusCode::OK, Json(serde_json::json!({ "id": id }))).into_response(),
        // Losing the report would be worse than filing it somewhere awkward: the
        // process log is shipped, and says loudly that this one was not filed.
        failed => {
            let why = failed
                .err()
                .map_or("no row came back".to_string(), |e| e.to_string());
            tracing::error!(
                "feedback [{kind}] from {reporter} NOT FILED ({why}): {message} (path={})",
                body.path
            );
            write_failed(what, why)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FeedbackView {
    pub id: String,
    pub kind: String,
    pub message: String,
    pub path: String,
    pub app_version: String,
    pub commit: String,
    pub user_agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// How often a folded crash has been reported. 1 for anything else.
    pub seen: u64,
    /// Everyone who has hit a folded crash: DIDs, and `anonymous` at most once.
    pub reporters: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_did: Option<String>,
    pub created_at: String,
    /// When it was last seen.
    pub updated_at: String,
}

/// `com.example.wiki.listFeedback`: every report, for an owner of the site; for
/// anyone else, the ones they sent. Newest sighting first, with the profile
/// behind every DID named.
pub async fn list_feedback(State(state): State<AppState>, Caller { did }: Caller) -> Response {
    let what = "listFeedback";
    let all = match crate::authz::Authz::new(state.db.clone())
        .owns_the_site(&did)
        .await
    {
        Ok(all) => all,
        Err(e) => return write_failed(what, e),
    };
    let listed = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT id, kind, message, path, app_version, build, user_agent, image, seen, \
                   owner_did, created_at, updated_at \
                 FROM feedback WHERE ?1 = 1 OR owner_did = ?2 \
                 ORDER BY updated_at DESC, id LIMIT 1000",
                vec![Value::Integer(i64::from(all)), Value::Text(did.clone())],
            )
            .await?;
        let text = |row: &turso::Row, i: usize| match row.get_value(i) {
            Ok(Value::Text(s)) => Some(s),
            _ => None,
        };
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            items.push(FeedbackView {
                id: row.get(0)?,
                kind: row.get(1)?,
                message: row.get(2)?,
                path: row.get(3)?,
                app_version: row.get(4)?,
                commit: row.get(5)?,
                user_agent: row.get(6)?,
                image: text(&row, 7),
                seen: u64::try_from(row.get::<i64>(8)?).unwrap_or(1),
                reporters: Vec::new(),
                owner_did: text(&row, 9),
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            });
        }
        for item in &mut items {
            let mut rows = conn
                .query(
                    "SELECT reporter FROM feedback_reporter WHERE feedback_id = ?1 \
                     ORDER BY reporter",
                    [item.id.as_str()],
                )
                .await?;
            while let Some(row) = rows.next().await? {
                item.reporters.push(row.get(0)?);
            }
        }
        let named: BTreeSet<String> = items
            .iter()
            .flat_map(|item| item.owner_did.iter().chain(&item.reporters))
            .filter(|who| *who != ANONYMOUS)
            .cloned()
            .collect();
        let profiles = crate::Store::new(state.db.clone()).profiles(&named).await?;
        Ok::<_, DbError>((items, profiles))
    };
    match listed.await {
        Ok((feedback, profiles)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "feedback": feedback, "profiles": profiles, "all": all })),
        )
            .into_response(),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct DeleteBody {
    pub id: String,
}

/// `com.example.wiki.deleteFeedback` (procedure): an owner of the site clears a
/// report away.
pub async fn delete_feedback(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<DeleteBody>,
) -> Response {
    let what = "deleteFeedback";
    match crate::authz::Authz::new(state.db.clone())
        .owns_the_site(&did)
        .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden("only an owner of the site clears feedback away"),
        Err(e) => return write_failed(what, e),
    }
    let deleted = async {
        let conn = state.db.acquire().await?;
        conn.execute(
            "DELETE FROM feedback_reporter WHERE feedback_id = ?1",
            [body.id.as_str()],
        )
        .await?;
        let gone = conn
            .execute("DELETE FROM feedback WHERE id = ?1", [body.id.as_str()])
            .await?;
        Ok::<_, DbError>(gone)
    };
    match deleted.await {
        Ok(0) => err(StatusCode::NOT_FOUND, "NotFound", "no such report"),
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "id": body.id }))).into_response(),
        Err(e) => write_failed(what, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{get_as, join_as, post, seeded_state, token_for};
    use serde_json::json;

    const SUBMIT: &str = "/xrpc/com.example.wiki.submitFeedback";
    const LIST: &str = "/xrpc/com.example.wiki.listFeedback";

    /// The seeded state with a site that carol owns.
    async fn state() -> AppState {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO context (id, kind, name, slug, path) \
             VALUES ('home', 'home', 'Home', '', '')",
            (),
        )
        .await
        .expect("site");
        token_for(&state, "did:plc:carol").await;
        join_as(&state, "did:plc:carol", "home", "owner").await;
        state
    }

    const CRASH: &str = "panicked at src/components/vote/poll.rs:412:9: index out of bounds\n\
        at wiki::vote::poll::PollApp (src/components/vote/poll.rs:412)\n\
        at dioxus_core::render<T> (dioxus-core-0.7.0/src/render.rs:88)";

    /// A screenshot sits in the reporter's own group, which whoever reads the
    /// reports is likely no member of. They are let open it all the same, and
    /// a report is no way to show them a file that is not the reporter's.
    #[tokio::test(flavor = "current_thread")]
    async fn a_reports_screenshot_is_for_whoever_reads_the_reports() {
        use crate::blob::tests::{fetch, upload};
        let mut state = state().await;
        state.config.blob_dir = std::env::temp_dir()
            .join(format!("feedback-blobs-{}", crate::util::random_token(8)))
            .to_string_lossy()
            .into_owned();
        let bob = token_for(&state, "did:plc:bob").await;
        let alice = token_for(&state, "did:plc:alice").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let (_, shot) = upload(&state, &bob, "c9", "image/png", b"a screenshot").await;
        let (_, hers) = upload(&state, &alice, "c9", "image/png", b"somebody else's").await;
        let uri = format!("/blob/{}", shot["id"].as_str().expect("id"));

        let (status, _, _) = fetch(&state, &uri, Some(&carol), None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a closed group's file, and no report yet"
        );
        let report =
            |image: &serde_json::Value| json!({"message": "Knappen er grå", "image": image});
        let (status, v) = post(
            router(state.clone()),
            SUBMIT,
            Some(&bob),
            report(&hers["id"]),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "not his to show: {v}");
        let (status, v) = post(router(state.clone()), SUBMIT, None, report(&shot["id"])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "nobody's, signed out: {v}");
        let (status, v) = post(
            router(state.clone()),
            SUBMIT,
            Some(&bob),
            report(&shot["id"]),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");

        let (status, _, bytes) = fetch(&state, &uri, Some(&carol), None).await;
        assert_eq!(
            (status, bytes.as_slice()),
            (StatusCode::OK, &b"a screenshot"[..])
        );
        let zoe = token_for(&state, "did:plc:zoe").await;
        let (status, _, _) = fetch(&state, &uri, Some(&zoe), None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "and for nobody else outside the group"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_member_reads_what_they_sent_and_a_site_owner_reads_it_all() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let alice = token_for(&state, "did:plc:alice").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let report = |message: &str| json!({"kind": "bug", "message": message, "path": "/closed"});
        for (who, message) in [
            (&bob, "Knappen virker ikke"),
            (&alice, "Æ, ø og å bliver til ?"),
        ] {
            let (status, v) = post(router(state.clone()), SUBMIT, Some(who), report(message)).await;
            assert_eq!(status, StatusCode::OK, "{v}");
        }

        let (_, bobs) = get_as(router(state.clone()), LIST, &bob).await;
        assert_eq!(bobs["all"], false);
        let mine = bobs["feedback"].as_array().expect("feedback");
        assert_eq!(mine.len(), 1, "{bobs}");
        assert_eq!(mine[0]["message"], "Knappen virker ikke");
        assert_eq!(mine[0]["seen"], 1);

        let (_, everything) = get_as(router(state.clone()), LIST, &carol).await;
        assert_eq!(everything["all"], true);
        assert_eq!(
            everything["feedback"].as_array().expect("feedback").len(),
            2
        );
        assert!(
            everything["profiles"]["did:plc:alice"].is_object(),
            "{everything}"
        );

        let delete = "/xrpc/com.example.wiki.deleteFeedback";
        let id = mine[0]["id"].clone();
        let (status, _) = post(router(state.clone()), delete, Some(&bob), json!({"id": id})).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a member cleared a report away"
        );
        let (status, _) = post(
            router(state.clone()),
            delete,
            Some(&carol),
            json!({"id": id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, bobs) = get_as(router(state.clone()), LIST, &bob).await;
        assert_eq!(bobs["feedback"], json!([]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_crash_seen_again_is_one_row_with_a_count_and_its_people() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let crash = |build: &str| json!({"kind": "crash", "message": CRASH, "commit": build});
        // The same crash from the next build: offsets and URLs differ, the place
        // it happened does not.
        let moved = CRASH.replace("render<T>", "render<U>")
            + "\n@https://wiki.example/assets/wiki_bg-dxhNEW.wasm:wasm-function[9]:0x1";
        let again = json!({"kind": "crash", "message": moved, "commit": "b2"});

        let (_, first) = post(router(state.clone()), SUBMIT, Some(&bob), crash("b1")).await;
        let (_, second) = post(router(state.clone()), SUBMIT, Some(&bob), again.clone()).await;
        let (_, third) = post(router(state.clone()), SUBMIT, None, again.clone()).await;
        let (_, fourth) = post(router(state.clone()), SUBMIT, None, again).await;
        assert_eq!(
            first["id"], second["id"],
            "the next build started a new row"
        );
        assert_eq!(first["id"], third["id"]);
        assert_eq!(first["id"], fourth["id"]);

        let (_, v) = get_as(router(state.clone()), LIST, &carol).await;
        let rows = v["feedback"].as_array().expect("feedback");
        assert_eq!(rows.len(), 1, "{v}");
        assert_eq!(rows[0]["seen"], 4);
        assert_eq!(rows[0]["reporters"], json!(["anonymous", "did:plc:bob"]));
        assert_eq!(
            rows[0]["commit"], "b1",
            "the first sighting is the one kept"
        );

        // A person's account of the same thing is theirs, and is not folded.
        let typed = json!({"kind": "bug", "message": CRASH});
        let (_, a) = post(router(state.clone()), SUBMIT, Some(&bob), typed.clone()).await;
        let (_, b) = post(router(state.clone()), SUBMIT, Some(&bob), typed).await;
        assert_ne!(a["id"], b["id"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_report_needs_no_session_but_a_flood_of_them_is_stopped() {
        let state = state().await;
        let (status, v) = post(
            router(state.clone()),
            SUBMIT,
            None,
            json!({"message": "  "}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        let (status, v) = post(
            router(state.clone()),
            SUBMIT,
            None,
            json!({"kind": "nonsense", "message": "x".repeat(MAX_MESSAGE * 2)}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query("SELECT kind, length(message), owner_did FROM feedback", ())
            .await
            .expect("q");
        let row = rows.next().await.expect("next").expect("row");
        assert_eq!(row.get::<String>(0).expect("kind"), "other");
        assert_eq!(row.get::<i64>(1).expect("length"), MAX_MESSAGE as i64);
        assert!(matches!(row.get_value(2).expect("owner"), Value::Null));

        let now = crate::util::now_secs();
        while state.feedback.anonymous_fits(now) {}
        let (status, v) = post(router(state.clone()), SUBMIT, None, json!({"message": "x"})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
        let bob = token_for(&state, "did:plc:bob").await;
        let member = post(
            router(state.clone()),
            SUBMIT,
            Some(&bob),
            json!({"message": "x"}),
        )
        .await;
        assert_eq!(
            member.0,
            StatusCode::OK,
            "a flood from outside shut a member out"
        );
        assert!(!state.feedback.anonymous_fits(now + 59));
        assert!(
            state.feedback.anonymous_fits(now + 61),
            "the minute passing did not let one through"
        );
    }

    #[test]
    fn a_crash_is_known_by_where_it_happened() {
        let same_place = CRASH.replace("PollApp", "PollApp<Web>").replace(
            "index out of bounds\n",
            "index out of bounds\n@wasm://x/assets/wiki_bg-dxhOLD.wasm:wasm-function[77]:0x2a\n",
        );
        assert_eq!(crash_digest(CRASH), crash_digest(&same_place));
        assert_ne!(
            crash_digest(CRASH),
            crash_digest(&CRASH.replace(":412)", ":413)"))
        );
        assert_ne!(crash_digest("unresolved a"), crash_digest("unresolved b"));
    }

    #[test]
    fn a_cap_never_splits_a_letter() {
        assert_eq!(clamp("blåbær".to_string(), 3), "bl", "å is two bytes");
        assert_eq!(clamp("blåbær".to_string(), 4), "blå");
        assert_eq!(clamp("kort".to_string(), 400), "kort");
    }
}
