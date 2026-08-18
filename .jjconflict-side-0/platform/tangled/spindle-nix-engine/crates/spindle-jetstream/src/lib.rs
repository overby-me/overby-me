//! Jetstream client for AT Protocol event ingestion in `tangled-spindle-nix`.
//!
//! This crate provides a WebSocket client that connects to the AT Protocol
//! Jetstream endpoint and ingests events relevant to the spindle runner.
//! Matches the upstream Go spindle's `ingester.go` behavior.
//!
//! # Event Types
//!
//! The client filters for the following AT Protocol collections:
//! - `sh.tangled.spindle.member` — Spindle membership records.
//! - `sh.tangled.repo` — Repository records pointing at this spindle.
//! - `sh.tangled.repo.collaborator` — Repository collaborator records.
//!
//! # Connection Management
//!
//! - Cursor-based replay on reconnection (persisted via `spindle-db`).
//! - DID-based subscription management (`add_did`, `remove_did`).
//! - Automatic reconnection with exponential backoff.
//!
//! # Architecture
//!
//! The client runs as a long-lived async task, feeding parsed events into
//! a channel for the main server to process via ingestion handlers
//! (`ingest_member`, `ingest_repo`, `ingest_collaborator`).
//!
//! # Example
//!
//! ```no_run
//! use std::collections::HashSet;
//! use spindle_jetstream::client::{JetstreamClient, ParsedEvent};
//! use tokio::sync::mpsc;
//!
//! # async fn example() {
//! let (tx, mut rx) = mpsc::channel::<ParsedEvent>(256);
//!
//! let initial_dids = HashSet::from(["did:plc:owner123".to_string()]);
//! let client = JetstreamClient::new(
//!     "wss://jetstream1.us-west.bsky.network/subscribe",
//!     initial_dids,
//!     tx,
//! );
//!
//! // Process events in another task
//! tokio::spawn(async move {
//!     while let Some(event) = rx.recv().await {
//!         println!("received event: {event:?}");
//!     }
//! });
//!
//! // Run the client (blocks until shutdown)
//! let shutdown = tokio_util::sync::CancellationToken::new();
//! client.run(0, |_cursor| Ok(()), shutdown).await;
//! # }
//! ```

pub mod client;
pub mod ingester;

pub use client::{
    COLLECTION_REPO, COLLECTION_REPO_COLLABORATOR, COLLECTION_SPINDLE_MEMBER, CommitOperation,
    JetstreamClient, JetstreamEvent, ParsedEvent, WANTED_COLLECTIONS,
};
pub use ingester::{
    IngestionContext, KnotSubscriber, RepoCollaboratorRecord, RepoRecord, SpindleMemberRecord,
    ingest_collaborator, ingest_event, ingest_member, ingest_repo,
};

/// Errors that can occur in the Jetstream client and ingester.
#[derive(Debug, thiserror::Error)]
pub enum JetstreamError {
    /// Failed to connect to or communicate with the Jetstream endpoint.
    #[error("connection error: {0}")]
    Connection(String),

    /// Failed to parse a Jetstream message or record.
    #[error("parse error: {0}")]
    Parse(String),

    /// Failed to process an ingested event (database, RBAC, etc.).
    #[error("ingestion error: {0}")]
    Ingestion(String),
}
