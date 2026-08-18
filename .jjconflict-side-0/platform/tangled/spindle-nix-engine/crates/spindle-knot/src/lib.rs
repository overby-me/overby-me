//! Knot event consumer for pipeline events in `tangled-spindle-nix`.
//!
//! This crate provides an event consumer that subscribes to knot HTTP event
//! streams and filters for `sh.tangled.pipeline` events. Matches the upstream
//! Go spindle's knot event consumer behavior.
//!
//! # Event Flow
//!
//! 1. When a repository is registered with this spindle (via Jetstream ingestion),
//!    the consumer subscribes to that repo's knot server event stream.
//! 2. The consumer filters for `sh.tangled.pipeline` events on subscribed knots.
//! 3. Pipeline events are forwarded to the main server for processing and
//!    enqueuing into the job queue.
//!
//! # Connection Management
//!
//! - Cursor-based replay on reconnection.
//! - Dynamic source management: knots can be added/removed at runtime as repos
//!   are registered/unregistered with this spindle.
//! - Automatic reconnection with exponential backoff.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use spindle_knot::consumer::{KnotConsumer, PipelineEvent};
//! use tokio::sync::mpsc;
//!
//! # async fn example() {
//! let db = Arc::new(spindle_db::Database::open_in_memory().unwrap());
//! let (tx, mut rx) = mpsc::channel::<PipelineEvent>(256);
//! let shutdown = tokio_util::sync::CancellationToken::new();
//!
//! let consumer = KnotConsumer::new(db, tx, shutdown.clone());
//!
//! // Restore subscriptions from previous session
//! consumer.restore_subscriptions().await.unwrap();
//!
//! // Subscribe to a new knot
//! consumer.subscribe("knot.example.com").await.unwrap();
//!
//! // Process events in another task
//! tokio::spawn(async move {
//!     while let Some(event) = rx.recv().await {
//!         println!("pipeline event from {}: {:?}", event.knot, event.payload);
//!     }
//! });
//! # }
//! ```

pub mod consumer;

pub use consumer::{KnotConsumer, PIPELINE_NSID, PipelineEvent, PipelineRecord, WorkflowManifest};

/// Errors that can occur in the knot event consumer.
#[derive(Debug, thiserror::Error)]
pub enum KnotError {
    /// Failed to connect to or communicate with a knot server.
    #[error("connection error: {0}")]
    Connection(String),

    /// Failed to parse a knot event or record.
    #[error("parse error: {0}")]
    Parse(String),

    /// A database operation failed.
    #[error("database error: {0}")]
    Database(String),
}
