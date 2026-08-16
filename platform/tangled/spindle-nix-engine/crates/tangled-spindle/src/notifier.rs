//! Event broadcast channel for `tangled-spindle-nix`.
//!
//! The [`Notifier`] bridges event producers (Jetstream ingester, knot consumer,
//! engine status updates) and WebSocket consumers. It wraps a
//! `tokio::sync::broadcast` channel that fans out pipeline events to all
//! connected `/events` WebSocket clients.

use spindle_db::events::Event;
use tokio::sync::broadcast;

/// Broadcast notifier for pipeline events.
///
/// Producers call [`notify`](Notifier::notify) when a new event is inserted
/// into the database. WebSocket handlers call [`subscribe`](Notifier::subscribe)
/// to receive a live stream of events.
///
/// Designed to be shared as `Arc<Notifier>` across the server.
pub struct Notifier {
    tx: broadcast::Sender<Event>,
}

impl Notifier {
    /// Create a new notifier with the given broadcast channel capacity.
    ///
    /// A capacity of 1024 is a reasonable default — if a slow consumer falls
    /// more than 1024 events behind, it will receive a `Lagged` error and
    /// can re-backfill from the database.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast an event to all subscribed WebSocket clients.
    ///
    /// If no clients are currently subscribed, the event is silently dropped.
    pub fn notify(&self, event: Event) {
        // Ignore send errors — they just mean no receivers are active.
        let _ = self.tx.send(event);
    }

    /// Subscribe to the event stream.
    ///
    /// Returns a receiver that will get all events broadcast after this call.
    /// The receiver should be created **before** backfilling from the database
    /// to avoid a race window where events could be missed.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

impl std::fmt::Debug for Notifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notifier")
            .field("receiver_count", &self.tx.receiver_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn notify_and_receive() {
        let notifier = Notifier::new(16);
        let mut rx = notifier.subscribe();

        let event = Event {
            id: 1,
            kind: "sh.tangled.pipeline.status".into(),
            payload: r#"{"status":"running"}"#.into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            rkey: "1000".into(),
            nsid: "sh.tangled.pipeline.status".into(),
            created: 1000,
        };

        notifier.notify(event.clone());

        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, 1);
        assert_eq!(received.nsid, "sh.tangled.pipeline.status");
    }

    #[tokio::test]
    async fn notify_without_subscribers_does_not_panic() {
        let notifier = Notifier::new(16);

        let event = Event {
            id: 1,
            kind: "test".into(),
            payload: "{}".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            rkey: "1000".into(),
            nsid: "test".into(),
            created: 1000,
        };

        // Should not panic even with no subscribers.
        notifier.notify(event);
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let notifier = Notifier::new(16);
        let mut rx1 = notifier.subscribe();
        let mut rx2 = notifier.subscribe();

        let event = Event {
            id: 42,
            kind: "test".into(),
            payload: "{}".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            rkey: "1000".into(),
            nsid: "test".into(),
            created: 1000,
        };

        notifier.notify(event);

        assert_eq!(rx1.recv().await.unwrap().id, 42);
        assert_eq!(rx2.recv().await.unwrap().id, 42);
    }

    #[test]
    fn debug_impl() {
        let notifier = Notifier::new(16);
        let debug = format!("{:?}", notifier);
        assert!(debug.contains("Notifier"));
    }
}
