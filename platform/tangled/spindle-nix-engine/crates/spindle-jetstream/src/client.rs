//! WebSocket client for the AT Protocol Jetstream.
//!
//! Connects to a Jetstream endpoint and filters for events relevant to
//! this spindle instance. Matches the upstream Go spindle's Jetstream
//! ingestion behavior.
//!
//! # Jetstream Protocol
//!
//! The Jetstream endpoint accepts WebSocket connections with query parameters:
//! - `wantedCollections` — Filter for specific AT Protocol collections.
//! - `wantedDids` — Filter for specific DIDs (added dynamically).
//! - `cursor` — Resume from a specific timestamp (microseconds since epoch).
//!
//! Messages are JSON objects with the following structure:
//! ```json
//! {
//!   "did": "did:plc:...",
//!   "time_us": 1700000000000000,
//!   "kind": "commit",
//!   "commit": {
//!     "rev": "...",
//!     "operation": "create",
//!     "collection": "sh.tangled.spindle.member",
//!     "rkey": "...",
//!     "record": { ... },
//!     "cid": "..."
//!   }
//! }
//! ```

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::JetstreamError;

// ---------------------------------------------------------------------------
// AT Protocol collection NSIDs relevant to the spindle
// ---------------------------------------------------------------------------

/// AT Protocol NSID for spindle membership records.
pub const COLLECTION_SPINDLE_MEMBER: &str = "sh.tangled.spindle.member";

/// AT Protocol NSID for repository records.
pub const COLLECTION_REPO: &str = "sh.tangled.repo";

/// AT Protocol NSID for repository collaborator records.
pub const COLLECTION_REPO_COLLABORATOR: &str = "sh.tangled.repo.collaborator";

/// All collections the Jetstream client filters for.
pub const WANTED_COLLECTIONS: &[&str] = &[
    COLLECTION_SPINDLE_MEMBER,
    COLLECTION_REPO,
    COLLECTION_REPO_COLLABORATOR,
];

// ---------------------------------------------------------------------------
// Jetstream message types
// ---------------------------------------------------------------------------

/// A raw Jetstream event message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JetstreamEvent {
    /// The DID that authored this event.
    pub did: String,

    /// Timestamp in microseconds since Unix epoch.
    pub time_us: i64,

    /// The event kind (e.g. `"commit"`, `"identity"`, `"account"`).
    pub kind: String,

    /// Commit data (present when `kind == "commit"`).
    #[serde(default)]
    pub commit: Option<CommitData>,
}

/// Commit-level data within a Jetstream event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitData {
    /// Revision string.
    #[serde(default)]
    pub rev: String,

    /// The operation: `"create"`, `"update"`, or `"delete"`.
    pub operation: String,

    /// The AT Protocol collection NSID (e.g. `"sh.tangled.spindle.member"`).
    pub collection: String,

    /// The record key.
    pub rkey: String,

    /// The record payload (present for `create` and `update` operations).
    #[serde(default)]
    pub record: Option<serde_json::Value>,

    /// The CID of the record.
    #[serde(default)]
    pub cid: Option<String>,
}

/// The operation type of a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOperation {
    Create,
    Update,
    Delete,
}

impl CommitOperation {
    /// Parse an operation string from the Jetstream protocol.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsed event types (output of the client)
// ---------------------------------------------------------------------------

/// A parsed, typed event emitted by the Jetstream client.
///
/// These are the events the ingestion layer consumes.
#[derive(Debug, Clone)]
pub enum ParsedEvent {
    /// A spindle member record was created, updated, or deleted.
    SpindleMember {
        /// DID of the actor who authored the record.
        did: String,
        /// Record key.
        rkey: String,
        /// The operation.
        operation: CommitOperation,
        /// The record payload (None for deletes).
        record: Option<serde_json::Value>,
        /// Event timestamp in microseconds.
        time_us: i64,
    },

    /// A repository record was created, updated, or deleted.
    Repo {
        /// DID of the repo owner.
        did: String,
        /// Record key.
        rkey: String,
        /// The operation.
        operation: CommitOperation,
        /// The record payload (None for deletes).
        record: Option<serde_json::Value>,
        /// Event timestamp in microseconds.
        time_us: i64,
    },

    /// A repository collaborator record was created, updated, or deleted.
    RepoCollaborator {
        /// DID of the actor who authored the record.
        did: String,
        /// Record key.
        rkey: String,
        /// The operation.
        operation: CommitOperation,
        /// The record payload (None for deletes).
        record: Option<serde_json::Value>,
        /// Event timestamp in microseconds.
        time_us: i64,
    },
}

// ---------------------------------------------------------------------------
// Backoff configuration
// ---------------------------------------------------------------------------

/// Configuration for exponential backoff on reconnection.
#[derive(Debug, Clone)]
struct BackoffConfig {
    /// Initial delay before the first reconnection attempt.
    initial: Duration,
    /// Maximum delay between reconnection attempts.
    max: Duration,
    /// Multiplier applied to the delay after each failed attempt.
    multiplier: f64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            multiplier: 2.0,
        }
    }
}

impl BackoffConfig {
    /// Compute the delay for the given attempt number (zero-based).
    fn delay(&self, attempt: u32) -> Duration {
        let delay_secs = self.initial.as_secs_f64() * self.multiplier.powi(attempt as i32);
        let capped = delay_secs.min(self.max.as_secs_f64());
        Duration::from_secs_f64(capped)
    }
}

// ---------------------------------------------------------------------------
// Jetstream client
// ---------------------------------------------------------------------------

/// A Jetstream WebSocket client that connects to the AT Protocol Jetstream,
/// filters events by DID and collection, and sends parsed events to a channel.
///
/// # DID Management
///
/// DIDs can be added or removed at runtime via [`add_did`](Self::add_did) and
/// [`remove_did`](Self::remove_did). Changes take effect on the next
/// reconnection cycle (the client periodically reconnects to refresh its
/// DID subscription list).
///
/// # Cursor Persistence
///
/// The client tracks the last-seen `time_us` and persists it via the provided
/// `save_cursor` callback. On startup, the caller provides the initial cursor
/// so the client can resume from where it left off.
pub struct JetstreamClient {
    /// The Jetstream WebSocket endpoint URL (e.g. `wss://jetstream1.us-west.bsky.network/subscribe`).
    endpoint: String,

    /// Set of DIDs to watch. Shared with the run loop via `Arc<RwLock<_>>`.
    watched_dids: Arc<RwLock<HashSet<String>>>,

    /// Channel sender for parsed events.
    event_tx: mpsc::Sender<ParsedEvent>,

    /// Backoff configuration for reconnection.
    backoff: BackoffConfig,
}

impl JetstreamClient {
    /// Create a new Jetstream client.
    ///
    /// # Arguments
    /// * `endpoint` — The Jetstream WebSocket URL.
    /// * `initial_dids` — Initial set of DIDs to watch.
    /// * `event_tx` — Channel sender for parsed events.
    pub fn new(
        endpoint: impl Into<String>,
        initial_dids: HashSet<String>,
        event_tx: mpsc::Sender<ParsedEvent>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            watched_dids: Arc::new(RwLock::new(initial_dids)),
            event_tx,
            backoff: BackoffConfig::default(),
        }
    }

    /// Add a DID to the watch list.
    ///
    /// The change takes effect on the next reconnection cycle.
    pub async fn add_did(&self, did: impl Into<String>) {
        let did = did.into();
        debug!(did = %did, "adding DID to Jetstream watch list");
        self.watched_dids.write().await.insert(did);
    }

    /// Remove a DID from the watch list.
    ///
    /// The change takes effect on the next reconnection cycle.
    pub async fn remove_did(&self, did: &str) {
        debug!(did = %did, "removing DID from Jetstream watch list");
        self.watched_dids.write().await.remove(did);
    }

    /// Get a snapshot of the current watched DIDs.
    pub async fn watched_dids(&self) -> HashSet<String> {
        self.watched_dids.read().await.clone()
    }

    /// Build the WebSocket URL with query parameters for the current state.
    fn build_url(&self, dids: &HashSet<String>, cursor: i64) -> Result<String, JetstreamError> {
        let mut url = url::Url::parse(&self.endpoint).map_err(|e| {
            JetstreamError::Connection(format!("invalid Jetstream endpoint URL: {e}"))
        })?;

        // Add wanted collections
        for collection in WANTED_COLLECTIONS {
            url.query_pairs_mut()
                .append_pair("wantedCollections", collection);
        }

        // Add wanted DIDs
        for did in dids {
            url.query_pairs_mut().append_pair("wantedDids", did);
        }

        // Add cursor if we have one (non-zero means we have a saved position)
        if cursor > 0 {
            url.query_pairs_mut()
                .append_pair("cursor", &cursor.to_string());
        }

        Ok(url.to_string())
    }

    /// Run the Jetstream client loop.
    ///
    /// This is the main entry point. It connects to the Jetstream endpoint,
    /// processes events, and reconnects with exponential backoff on failures.
    /// The loop runs until the provided `shutdown` token is cancelled.
    ///
    /// # Arguments
    /// * `initial_cursor` — The cursor to resume from (0 means start from now).
    /// * `save_cursor` — Callback to persist the cursor after processing events.
    /// * `shutdown` — Cancellation token to stop the client.
    pub async fn run<F>(
        &self,
        initial_cursor: i64,
        save_cursor: F,
        shutdown: tokio_util::sync::CancellationToken,
    ) where
        F: Fn(i64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
    {
        let mut cursor = initial_cursor;
        let mut attempt: u32 = 0;

        loop {
            if shutdown.is_cancelled() {
                info!("Jetstream client shutting down");
                return;
            }

            // Snapshot the current DIDs for this connection
            let dids = self.watched_dids.read().await.clone();

            if dids.is_empty() {
                debug!("no DIDs to watch, waiting before retry");
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(5)) => continue,
                    () = shutdown.cancelled() => return,
                }
            }

            let url = match self.build_url(&dids, cursor) {
                Ok(url) => url,
                Err(e) => {
                    error!(%e, "failed to build Jetstream URL");
                    tokio::select! {
                        () = tokio::time::sleep(self.backoff.delay(attempt)) => {
                            attempt = attempt.saturating_add(1);
                            continue;
                        }
                        () = shutdown.cancelled() => return,
                    }
                }
            };

            info!(
                url = %url,
                dids = dids.len(),
                cursor = cursor,
                "connecting to Jetstream"
            );

            match self
                .connect_and_process(&url, &mut cursor, &save_cursor, &shutdown)
                .await
            {
                Ok(()) => {
                    // Clean shutdown or intentional disconnect
                    info!("Jetstream connection closed cleanly");
                    attempt = 0;
                }
                Err(e) => {
                    let delay = self.backoff.delay(attempt);
                    warn!(
                        %e,
                        attempt = attempt,
                        delay_secs = delay.as_secs_f64(),
                        "Jetstream connection failed, reconnecting"
                    );
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {
                            attempt = attempt.saturating_add(1);
                        }
                        () = shutdown.cancelled() => return,
                    }
                }
            }
        }
    }

    /// Connect to the Jetstream endpoint and process messages until
    /// disconnection or shutdown.
    async fn connect_and_process<F>(
        &self,
        url: &str,
        cursor: &mut i64,
        save_cursor: &F,
        shutdown: &tokio_util::sync::CancellationToken,
    ) -> Result<(), JetstreamError>
    where
        F: Fn(i64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
    {
        let (ws_stream, _response) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| JetstreamError::Connection(format!("WebSocket connect failed: {e}")))?;

        info!("connected to Jetstream");

        let (mut _write, mut read) = ws_stream.split();

        // Track how many events since last cursor save for batching
        let mut events_since_save: u64 = 0;
        const CURSOR_SAVE_INTERVAL: u64 = 100;

        loop {
            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            match self.handle_message(&text, cursor).await {
                                Ok(true) => {
                                    // Event was processed and cursor updated
                                    events_since_save += 1;
                                    if events_since_save >= CURSOR_SAVE_INTERVAL {
                                        if let Err(e) = save_cursor(*cursor) {
                                            warn!(%e, cursor = *cursor, "failed to persist Jetstream cursor");
                                        }
                                        events_since_save = 0;
                                    }
                                }
                                Ok(false) => {
                                    // Message was not a relevant event (ignored)
                                }
                                Err(e) => {
                                    debug!(%e, "failed to process Jetstream message");
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            debug!("received ping, sending pong");
                            if let Err(e) = _write.send(Message::Pong(data)).await {
                                warn!(%e, "failed to send pong");
                            }
                        }
                        Some(Ok(Message::Close(frame))) => {
                            info!(?frame, "Jetstream server closed connection");
                            // Save cursor before disconnecting
                            if events_since_save > 0
                                && let Err(e) = save_cursor(*cursor)
                            {
                                warn!(%e, "failed to persist cursor on close");
                            }
                            return Ok(());
                        }
                        Some(Ok(_)) => {
                            // Binary, Pong, Frame — ignore
                        }
                        Some(Err(e)) => {
                            // Save cursor before returning error
                            if events_since_save > 0 {
                                let _ = save_cursor(*cursor);
                            }
                            return Err(JetstreamError::Connection(format!(
                                "WebSocket read error: {e}"
                            )));
                        }
                        None => {
                            // Stream ended
                            if events_since_save > 0 {
                                let _ = save_cursor(*cursor);
                            }
                            return Ok(());
                        }
                    }
                }
                () = shutdown.cancelled() => {
                    info!("shutdown requested, closing Jetstream connection");
                    if events_since_save > 0 {
                        let _ = save_cursor(*cursor);
                    }
                    let _ = _write.send(Message::Close(None)).await;
                    return Ok(());
                }
            }
        }
    }

    /// Parse and handle a single text message from the Jetstream.
    ///
    /// Returns `Ok(true)` if a relevant event was parsed and sent,
    /// `Ok(false)` if the message was ignored, or `Err` on parse failure.
    async fn handle_message(&self, text: &str, cursor: &mut i64) -> Result<bool, JetstreamError> {
        let event: JetstreamEvent = serde_json::from_str(text)
            .map_err(|e| JetstreamError::Parse(format!("failed to parse Jetstream event: {e}")))?;

        // Always update cursor to the latest timestamp
        if event.time_us > *cursor {
            *cursor = event.time_us;
        }

        // We only care about commit events
        if event.kind != "commit" {
            return Ok(false);
        }

        let commit = match &event.commit {
            Some(c) => c,
            None => return Ok(false),
        };

        let operation = match CommitOperation::parse(&commit.operation) {
            Some(op) => op,
            None => {
                debug!(
                    operation = %commit.operation,
                    "unknown commit operation, ignoring"
                );
                return Ok(false);
            }
        };

        let parsed = match commit.collection.as_str() {
            COLLECTION_SPINDLE_MEMBER => ParsedEvent::SpindleMember {
                did: event.did,
                rkey: commit.rkey.clone(),
                operation,
                record: commit.record.clone(),
                time_us: event.time_us,
            },
            COLLECTION_REPO => ParsedEvent::Repo {
                did: event.did,
                rkey: commit.rkey.clone(),
                operation,
                record: commit.record.clone(),
                time_us: event.time_us,
            },
            COLLECTION_REPO_COLLABORATOR => ParsedEvent::RepoCollaborator {
                did: event.did,
                rkey: commit.rkey.clone(),
                operation,
                record: commit.record.clone(),
                time_us: event.time_us,
            },
            _ => {
                // Collection not in our wanted list — the server shouldn't
                // send these, but ignore gracefully.
                return Ok(false);
            }
        };

        // Send to the ingestion channel
        if self.event_tx.send(parsed).await.is_err() {
            debug!("event channel closed, receiver dropped");
            return Err(JetstreamError::Connection(
                "event channel closed".to_string(),
            ));
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jetstream_event_commit() {
        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000000,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "create",
                "collection": "sh.tangled.spindle.member",
                "rkey": "self",
                "record": {"$type": "sh.tangled.spindle.member", "did": "did:plc:bob"},
                "cid": "bafyrei..."
            }
        }"#;

        let event: JetstreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.did, "did:plc:alice");
        assert_eq!(event.time_us, 1700000000000000);
        assert_eq!(event.kind, "commit");

        let commit = event.commit.unwrap();
        assert_eq!(commit.operation, "create");
        assert_eq!(commit.collection, COLLECTION_SPINDLE_MEMBER);
        assert_eq!(commit.rkey, "self");
        assert!(commit.record.is_some());
    }

    #[test]
    fn parse_jetstream_event_identity() {
        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000000,
            "kind": "identity"
        }"#;

        let event: JetstreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.kind, "identity");
        assert!(event.commit.is_none());
    }

    #[test]
    fn parse_jetstream_event_delete() {
        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000000,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "delete",
                "collection": "sh.tangled.repo",
                "rkey": "myrepo"
            }
        }"#;

        let event: JetstreamEvent = serde_json::from_str(json).unwrap();
        let commit = event.commit.unwrap();
        assert_eq!(commit.operation, "delete");
        assert!(commit.record.is_none());
        assert!(commit.cid.is_none());
    }

    #[test]
    fn commit_operation_parse() {
        assert_eq!(
            CommitOperation::parse("create"),
            Some(CommitOperation::Create)
        );
        assert_eq!(
            CommitOperation::parse("update"),
            Some(CommitOperation::Update)
        );
        assert_eq!(
            CommitOperation::parse("delete"),
            Some(CommitOperation::Delete)
        );
        assert_eq!(CommitOperation::parse("unknown"), None);
        assert_eq!(CommitOperation::parse(""), None);
    }

    #[test]
    fn backoff_delay_exponential() {
        let config = BackoffConfig::default();

        let d0 = config.delay(0);
        assert_eq!(d0, Duration::from_secs(1));

        let d1 = config.delay(1);
        assert_eq!(d1, Duration::from_secs(2));

        let d2 = config.delay(2);
        assert_eq!(d2, Duration::from_secs(4));

        let d3 = config.delay(3);
        assert_eq!(d3, Duration::from_secs(8));
    }

    #[test]
    fn backoff_delay_capped_at_max() {
        let config = BackoffConfig::default();

        // 2^10 = 1024 seconds, but max is 60
        let d = config.delay(10);
        assert_eq!(d, Duration::from_secs(60));
    }

    #[test]
    fn build_url_with_dids_and_cursor() {
        let (tx, _rx) = mpsc::channel(1);
        let mut dids = HashSet::new();
        dids.insert("did:plc:alice".to_string());

        let client = JetstreamClient::new("wss://jetstream.example.com/subscribe", dids, tx);

        let watched = HashSet::from(["did:plc:alice".to_string()]);
        let url = client.build_url(&watched, 1700000000000000).unwrap();

        assert!(url.starts_with("wss://jetstream.example.com/subscribe?"));
        assert!(url.contains("wantedCollections=sh.tangled.spindle.member"));
        assert!(url.contains("wantedCollections=sh.tangled.repo"));
        assert!(url.contains("wantedCollections=sh.tangled.repo.collaborator"));
        assert!(url.contains("wantedDids=did%3Aplc%3Aalice"));
        assert!(url.contains("cursor=1700000000000000"));
    }

    #[test]
    fn build_url_no_cursor_when_zero() {
        let (tx, _rx) = mpsc::channel(1);
        let dids = HashSet::from(["did:plc:alice".to_string()]);

        let client = JetstreamClient::new("wss://jetstream.example.com/subscribe", dids, tx);

        let watched = HashSet::from(["did:plc:alice".to_string()]);
        let url = client.build_url(&watched, 0).unwrap();

        assert!(!url.contains("cursor="));
    }

    #[tokio::test]
    async fn add_and_remove_dids() {
        let (tx, _rx) = mpsc::channel(1);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        assert!(client.watched_dids().await.is_empty());

        client.add_did("did:plc:alice").await;
        client.add_did("did:plc:bob").await;

        let dids = client.watched_dids().await;
        assert_eq!(dids.len(), 2);
        assert!(dids.contains("did:plc:alice"));
        assert!(dids.contains("did:plc:bob"));

        client.remove_did("did:plc:alice").await;
        let dids = client.watched_dids().await;
        assert_eq!(dids.len(), 1);
        assert!(!dids.contains("did:plc:alice"));
        assert!(dids.contains("did:plc:bob"));
    }

    #[tokio::test]
    async fn handle_message_parses_member_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000001,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "create",
                "collection": "sh.tangled.spindle.member",
                "rkey": "self",
                "record": {"$type": "sh.tangled.spindle.member", "did": "did:plc:bob"}
            }
        }"#;

        let mut cursor = 0i64;
        let result = client.handle_message(json, &mut cursor).await;
        assert!(result.is_ok());
        assert!(result.unwrap()); // was relevant

        assert_eq!(cursor, 1700000000000001);

        let event = rx.try_recv().unwrap();
        match event {
            ParsedEvent::SpindleMember {
                did,
                rkey,
                operation,
                ..
            } => {
                assert_eq!(did, "did:plc:alice");
                assert_eq!(rkey, "self");
                assert_eq!(operation, CommitOperation::Create);
            }
            _ => panic!("expected SpindleMember event"),
        }
    }

    #[tokio::test]
    async fn handle_message_ignores_identity_events() {
        let (tx, _rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000002,
            "kind": "identity"
        }"#;

        let mut cursor = 0i64;
        let result = client.handle_message(json, &mut cursor).await;
        assert!(result.is_ok());
        assert!(!result.unwrap()); // not relevant

        // Cursor should still be updated
        assert_eq!(cursor, 1700000000000002);
    }

    #[tokio::test]
    async fn handle_message_ignores_unknown_collection() {
        let (tx, _rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000003,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "create",
                "collection": "app.bsky.feed.post",
                "rkey": "abc123",
                "record": {}
            }
        }"#;

        let mut cursor = 0i64;
        let result = client.handle_message(json, &mut cursor).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn handle_message_parses_repo_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000004,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "create",
                "collection": "sh.tangled.repo",
                "rkey": "myrepo",
                "record": {
                    "$type": "sh.tangled.repo",
                    "name": "myrepo",
                    "knot": "knot.example.com",
                    "spindle": "spindle.example.com"
                }
            }
        }"#;

        let mut cursor = 0i64;
        let result = client.handle_message(json, &mut cursor).await.unwrap();
        assert!(result);

        let event = rx.try_recv().unwrap();
        match event {
            ParsedEvent::Repo {
                did,
                rkey,
                operation,
                record,
                ..
            } => {
                assert_eq!(did, "did:plc:alice");
                assert_eq!(rkey, "myrepo");
                assert_eq!(operation, CommitOperation::Create);
                let rec = record.unwrap();
                assert_eq!(rec["spindle"], "spindle.example.com");
            }
            _ => panic!("expected Repo event"),
        }
    }

    #[tokio::test]
    async fn handle_message_parses_collaborator_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000005,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "create",
                "collection": "sh.tangled.repo.collaborator",
                "rkey": "abc123",
                "record": {
                    "$type": "sh.tangled.repo.collaborator",
                    "did": "did:plc:bob",
                    "repo": "myrepo"
                }
            }
        }"#;

        let mut cursor = 0i64;
        let result = client.handle_message(json, &mut cursor).await.unwrap();
        assert!(result);

        let event = rx.try_recv().unwrap();
        match event {
            ParsedEvent::RepoCollaborator {
                did,
                rkey,
                operation,
                ..
            } => {
                assert_eq!(did, "did:plc:alice");
                assert_eq!(rkey, "abc123");
                assert_eq!(operation, CommitOperation::Create);
            }
            _ => panic!("expected RepoCollaborator event"),
        }
    }

    #[tokio::test]
    async fn handle_message_delete_has_no_record() {
        let (tx, mut rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 1700000000000006,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "delete",
                "collection": "sh.tangled.repo",
                "rkey": "myrepo"
            }
        }"#;

        let mut cursor = 0i64;
        let result = client.handle_message(json, &mut cursor).await.unwrap();
        assert!(result);

        let event = rx.try_recv().unwrap();
        match event {
            ParsedEvent::Repo {
                operation, record, ..
            } => {
                assert_eq!(operation, CommitOperation::Delete);
                assert!(record.is_none());
            }
            _ => panic!("expected Repo event"),
        }
    }

    #[tokio::test]
    async fn cursor_does_not_go_backwards() {
        let (tx, _rx) = mpsc::channel(16);
        let client =
            JetstreamClient::new("wss://jetstream.example.com/subscribe", HashSet::new(), tx);

        let json = r#"{
            "did": "did:plc:alice",
            "time_us": 100,
            "kind": "commit",
            "commit": {
                "rev": "rev1",
                "operation": "create",
                "collection": "sh.tangled.repo",
                "rkey": "myrepo",
                "record": {}
            }
        }"#;

        let mut cursor = 200i64;
        let _ = client.handle_message(json, &mut cursor).await;
        // Cursor should not have gone backwards
        assert_eq!(cursor, 200);
    }
}
