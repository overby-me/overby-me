//! The Jetstream firehose consumer (rewrite kickoff item, the first serving-layer
//! slice): connect to the atproto firehose, decode events, filter for the wiki
//! collections, materialize the public records into the Turso view, and broadcast
//! a change delta to `/ws` clients. This turns the previously-inert broadcast
//! channel into a real data source (nothing wrote deltas before).
//!
//! Jetstream is the DECIDED firehose choice (JSON events, not raw DAG-CBOR), so
//! this is a plain WebSocket + serde_json consumer. It is protocol-independent of
//! the still-open GraphQL-vs-XRPC serving question, which is why it is buildable
//! now. [`ingest`] is the offline-testable core (feed it a raw event, assert the
//! view + the returned delta); [`run`] is the live connect/reconnect loop.

use crate::db::DbError;
use crate::{AppState, Store};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// The wiki record collections the consumer subscribes to and acts on.
pub const WIKI_COLLECTIONS: &[&str] = &[
    "com.example.wiki.post",
    "com.example.wiki.comment",
    "com.example.wiki.resolution",
];

const POST_COLLECTION: &str = "com.example.wiki.post";

/// Live firehose status for `/healthz`: whether the socket is connected and how
/// many events have been seen (a stalled firehose is connected-but-not-advancing,
/// which an uptime check watches).
#[derive(Default)]
pub struct FirehoseStatus {
    pub connected: AtomicBool,
    pub events_seen: AtomicU64,
}

#[derive(Debug, Deserialize)]
struct JetstreamEvent {
    did: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    commit: Option<Commit>,
}

#[derive(Debug, Deserialize)]
struct Commit {
    operation: String,
    collection: String,
    rkey: String,
    #[serde(default)]
    record: Option<serde_json::Value>,
}

/// A change broadcast to `/ws` clients: which record changed and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delta {
    pub collection: String,
    pub operation: String,
    pub did: String,
    pub uri: String,
}

#[derive(Debug)]
pub enum FirehoseError {
    Db(DbError),
    Json(serde_json::Error),
}

impl std::fmt::Display for FirehoseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FirehoseError::Db(e) => write!(f, "firehose db error: {e}"),
            FirehoseError::Json(e) => write!(f, "firehose decode error: {e}"),
        }
    }
}
impl std::error::Error for FirehoseError {}
impl From<DbError> for FirehoseError {
    fn from(e: DbError) -> Self {
        FirehoseError::Db(e)
    }
}
impl From<serde_json::Error> for FirehoseError {
    fn from(e: serde_json::Error) -> Self {
        FirehoseError::Json(e)
    }
}

/// Ingest one raw Jetstream JSON message: parse it, and if it is a wiki-collection
/// commit, materialize it into the Turso view and return the change [`Delta`] to
/// broadcast. Non-wiki / non-commit events return `None`.
///
/// Materialization depth: the public `post` maps 1:1 to an entity table (author
/// DID, text, createdAt), so it is upserted/deleted here. `comment` and
/// `resolution` need subject->context resolution (a NOT NULL context) and are
/// broadcast-only for now (a delta still fires so clients refetch); their
/// materialization is a later slice.
pub async fn ingest(store: &Store, raw: &str) -> Result<Option<Delta>, FirehoseError> {
    let event: JetstreamEvent = serde_json::from_str(raw)?;
    if event.kind != "commit" {
        return Ok(None);
    }
    let Some(commit) = event.commit else {
        return Ok(None);
    };
    if !WIKI_COLLECTIONS.contains(&commit.collection.as_str()) {
        return Ok(None);
    }

    let uri = format!("at://{}/{}/{}", event.did, commit.collection, commit.rkey);

    match commit.operation.as_str() {
        "create" | "update" => {
            if commit.collection == POST_COLLECTION
                && let Some(rec) = &commit.record
            {
                let text = rec.get("text").and_then(|v| v.as_str());
                let created = rec.get("createdAt").and_then(|v| v.as_str());
                // The lexicon requires text + createdAt; skip a malformed
                // record's materialization but still emit the delta.
                if let (Some(text), Some(created)) = (text, created) {
                    let group = rec.get("group").and_then(|v| v.as_str());
                    let reply = rec
                        .get("reply")
                        .and_then(|r| r.get("uri"))
                        .and_then(|v| v.as_str());
                    store.upsert_user_min(&event.did).await?;
                    store
                        .upsert_public_post(&uri, &event.did, text, group, reply, created)
                        .await?;
                }
            }
        }
        "delete" => {
            if commit.collection == POST_COLLECTION {
                store.delete_public_post(&uri).await?;
            }
        }
        _ => return Ok(None),
    }

    Ok(Some(Delta {
        collection: commit.collection,
        operation: commit.operation,
        did: event.did,
        uri,
    }))
}

/// The Jetstream subscribe URL with server-side collection filtering.
fn subscribe_url(base: &str) -> String {
    let params: String = WIKI_COLLECTIONS
        .iter()
        .map(|c| format!("wantedCollections={c}"))
        .collect::<Vec<_>>()
        .join("&");
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{params}")
}

/// Run the firehose consumer for the life of the process: connect to Jetstream
/// (filtered to the wiki collections), materialize + broadcast each event, and
/// reconnect with backoff on any drop. NOT unit-tested (it needs a live network);
/// [`ingest`] is the tested core.
pub async fn run(state: AppState) {
    if state.config.firehose_url.is_empty() {
        tracing::warn!("firehose disabled: no JETSTREAM_URL configured");
        return;
    }
    let url = subscribe_url(&state.config.firehose_url);
    let store = Store::new(state.db.clone());

    loop {
        // A bounded connect: `connect_async` has no timeout of its own, so a
        // stalled DNS/TCP/TLS handshake would hang forever with no retry. Cap it
        // and treat a timeout like any other connect failure.
        let connect = tokio::time::timeout(Duration::from_secs(15), connect_async(&url)).await;
        match connect {
            Ok(Ok((mut ws, _resp))) => {
                state.firehose.connected.store(true, Ordering::Relaxed);
                tracing::info!("firehose connected: {}", state.config.firehose_url);
                while let Some(msg) = ws.next().await {
                    let text = match msg {
                        Ok(Message::Text(t)) => t.to_string(),
                        Ok(Message::Close(_)) => break,
                        Err(e) => {
                            tracing::warn!("firehose stream error: {e}");
                            break;
                        }
                        Ok(_) => continue,
                    };
                    state.firehose.events_seen.fetch_add(1, Ordering::Relaxed);
                    match ingest(&store, &text).await {
                        Ok(Some(delta)) => {
                            if let Ok(json) = serde_json::to_string(&delta) {
                                let _ = state.deltas.send(json);
                            }
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!("firehose ingest error: {e}"),
                    }
                }
                state.firehose.connected.store(false, Ordering::Relaxed);
                tracing::warn!("firehose disconnected; reconnecting");
            }
            Ok(Err(e)) => {
                state.firehose.connected.store(false, Ordering::Relaxed);
                tracing::warn!("firehose connect failed: {e}; retrying");
            }
            Err(_) => {
                state.firehose.connected.store(false, Ordering::Relaxed);
                tracing::warn!("firehose connect timed out; retrying");
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    async fn store_with_db() -> (Store, Db) {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        (Store::new(db.clone()), db)
    }

    fn post_event(op: &str, rkey: &str, text: &str) -> String {
        format!(
            r#"{{"did":"did:plc:alice","kind":"commit","commit":{{"operation":"{op}","collection":"com.example.wiki.post","rkey":"{rkey}","record":{{"$type":"com.example.wiki.post","text":"{text}","createdAt":"2026-07-16T12:00:00.000Z"}}}}}}"#
        )
    }

    async fn post_text(db: &Db, uri: &str) -> Option<String> {
        let conn = db.acquire().await.unwrap();
        let mut rows = conn
            .query("SELECT text FROM post WHERE id = ?1", [uri])
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .map(|r| r.get::<String>(0).unwrap())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_create_materializes_and_returns_a_delta() {
        let (store, db) = store_with_db().await;
        let uri = "at://did:plc:alice/com.example.wiki.post/abc";

        let delta = ingest(&store, &post_event("create", "abc", "Hej verden"))
            .await
            .expect("ingest")
            .expect("a wiki delta");
        assert_eq!(delta.operation, "create");
        assert_eq!(delta.collection, POST_COLLECTION);
        assert_eq!(delta.uri, uri);

        // Materialized: the post row exists with the text, and the author user was
        // upserted so the FK target exists.
        assert_eq!(post_text(&db, uri).await.as_deref(), Some("Hej verden"));
        let conn = db.acquire().await.unwrap();
        let mut u = conn
            .query("SELECT 1 FROM user WHERE did = 'did:plc:alice'", ())
            .await
            .unwrap();
        assert!(u.next().await.unwrap().is_some(), "author user upserted");
        // Published records are public.
        let mut v = conn
            .query("SELECT visibility FROM post WHERE id = ?1", [uri])
            .await
            .unwrap();
        let vis: String = v.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(vis, "public");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_update_then_delete_are_idempotent() {
        let (store, db) = store_with_db().await;
        let uri = "at://did:plc:alice/com.example.wiki.post/abc";

        ingest(&store, &post_event("create", "abc", "first"))
            .await
            .unwrap();
        // Update upserts the same row (no duplicate).
        ingest(&store, &post_event("update", "abc", "second"))
            .await
            .unwrap();
        assert_eq!(post_text(&db, uri).await.as_deref(), Some("second"));
        let conn = db.acquire().await.unwrap();
        let mut c = conn.query("SELECT count(*) FROM post", ()).await.unwrap();
        let n: i64 = c.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(n, 1, "update upserts, not duplicates");

        // Delete removes it, and returns a delta.
        let d = ingest(&store, &post_event("delete", "abc", ""))
            .await
            .unwrap()
            .expect("delete delta");
        assert_eq!(d.operation, "delete");
        assert_eq!(post_text(&db, uri).await, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_wiki_and_non_commit_events_are_ignored() {
        let (store, _db) = store_with_db().await;
        // A bsky post (not a wiki collection).
        let bsky = r#"{"did":"did:plc:x","kind":"commit","commit":{"operation":"create","collection":"app.bsky.feed.post","rkey":"z","record":{"text":"hi"}}}"#;
        assert!(ingest(&store, bsky).await.unwrap().is_none());
        // An identity event (not a commit).
        let ident = r#"{"did":"did:plc:x","kind":"identity"}"#;
        assert!(ingest(&store, ident).await.unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn comment_is_broadcast_but_not_yet_materialized() {
        let (store, db) = store_with_db().await;
        let raw = r#"{"did":"did:plc:alice","kind":"commit","commit":{"operation":"create","collection":"com.example.wiki.comment","rkey":"c1","record":{"text":"agreed","createdAt":"2026-07-16T12:00:00.000Z","subject":{"uri":"at://did:plc:org/com.example.wiki.resolution/r1","cid":"bafy"}}}}"#;
        // A delta fires (clients refetch)...
        let delta = ingest(&store, raw).await.unwrap().expect("comment delta");
        assert_eq!(delta.collection, "com.example.wiki.comment");
        // ...but the comment table stays empty (depth-3: needs subject->context).
        let conn = db.acquire().await.unwrap();
        let mut c = conn
            .query("SELECT count(*) FROM comment", ())
            .await
            .unwrap();
        let n: i64 = c.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(n, 0, "comment materialization is deferred");
    }

    #[test]
    fn subscribe_url_appends_wanted_collections() {
        let u = subscribe_url("wss://jetstream.example/subscribe");
        assert!(u.starts_with("wss://jetstream.example/subscribe?wantedCollections="));
        assert!(u.contains("com.example.wiki.post"));
        assert!(u.contains("com.example.wiki.comment"));
        // A base that already has a query string uses '&'.
        assert!(subscribe_url("wss://x/y?foo=1").contains("?foo=1&wantedCollections="));
    }
}
