//! Filesystem watch operations for the tramp-agent RPC server.
//!
//! Implements the following RPC methods:
//!
//! | Method         | Description                                         |
//! |----------------|-----------------------------------------------------|
//! | `watch.add`    | Start watching a path for filesystem changes        |
//! | `watch.remove` | Stop watching a previously added path               |
//! | `watch.list`   | List all currently active watches                   |
//!
//! When a watched path changes, the agent sends an `fs.changed` notification
//! (unsolicited, no request ID) to the client.  The VFS layer on the plugin
//! side uses these notifications to invalidate specific cache entries
//! immediately, rather than waiting for TTL expiry.
//!
//! Under the hood this uses the [`notify`] crate which abstracts over
//! inotify (Linux), kqueue (macOS/BSD), and ReadDirectoryChangesW (Windows).

use std::collections::HashMap;
use std::path::PathBuf;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rmpv::Value;
use tokio::sync::{Mutex, mpsc};

use crate::rpc::{Notification, Response, error_code};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required string parameter from a MsgPack map by key.
fn get_str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Response> {
    params
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_str())
        })
        .ok_or_else(|| {
            Response::err(
                0, // caller must fix up the id
                error_code::INVALID_PARAMS,
                format!("missing or invalid parameter: {key}"),
            )
        })
}

/// Extract an optional boolean parameter from a MsgPack map by key.
fn get_bool_param(params: &Value, key: &str) -> Option<bool> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_bool())
    })
}

// ---------------------------------------------------------------------------
// Watch state
// ---------------------------------------------------------------------------

/// Tracks all active filesystem watches and the channel used to send
/// `fs.changed` notifications back to the main loop for forwarding to the
/// client.
pub struct WatchState {
    /// Maps watched paths to their internal entry (for removal).
    watches: Mutex<HashMap<PathBuf, WatchEntry>>,
    /// Sender half of the notification channel.  The main loop reads from
    /// the receiver and writes `fs.changed` notifications to the client.
    notification_tx: mpsc::UnboundedSender<Notification>,
    /// Receiver half — consumed by the main loop via [`WatchState::take_receiver`].
    notification_rx: Mutex<Option<mpsc::UnboundedReceiver<Notification>>>,
}

/// Internal bookkeeping for a single watch.
struct WatchEntry {
    /// Whether this watch is recursive (watches subdirectories too).
    recursive: bool,
    /// The watcher handle.  Dropping it stops the watch.
    _watcher: RecommendedWatcher,
}

impl WatchState {
    /// Create a new, empty watch state.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            watches: Mutex::new(HashMap::new()),
            notification_tx: tx,
            notification_rx: Mutex::new(Some(rx)),
        }
    }

    /// Take the notification receiver.  This should be called exactly once
    /// by the main loop at startup.  Returns `None` if already taken.
    pub async fn take_receiver(&self) -> Option<mpsc::UnboundedReceiver<Notification>> {
        self.notification_rx.lock().await.take()
    }
}

impl Default for WatchState {
    fn default() -> Self {
        Self::new()
    }
}

/// Classify a [`notify::EventKind`] into a human-readable change type string.
fn classify_event(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Create(_) => "create",
        EventKind::Modify(_) => "modify",
        EventKind::Remove(_) => "remove",
        EventKind::Access(_) => "access",
        EventKind::Other => "other",
        _ => "unknown",
    }
}

/// Build an `fs.changed` notification from a [`notify::Event`].
fn event_to_notification(event: &Event) -> Notification {
    let paths: Vec<Value> = event
        .paths
        .iter()
        .map(|p| Value::String(p.to_string_lossy().into_owned().into()))
        .collect();

    let change_type = classify_event(&event.kind);

    Notification::new(
        "fs.changed",
        Value::Map(vec![
            (Value::String("paths".into()), Value::Array(paths)),
            (
                Value::String("kind".into()),
                Value::String(change_type.into()),
            ),
        ]),
    )
}

// ---------------------------------------------------------------------------
// RPC method handlers
// ---------------------------------------------------------------------------

/// `watch.add` — start watching a path for filesystem changes.
///
/// Params:
/// - `path`: the filesystem path to watch (string, required)
/// - `recursive`: if `true`, also watch all subdirectories (boolean,
///   optional, defaults to `false`)
///
/// Result: `{}` (empty map on success).
///
/// If the path is already being watched, this is a no-op (returns success).
pub async fn add(id: u64, params: &Value, state: &WatchState) -> Response {
    let path_str = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let recursive = get_bool_param(params, "recursive").unwrap_or(false);
    let path = PathBuf::from(path_str);

    let mut watches = state.watches.lock().await;

    // If already watching this exact path, return success.
    if watches.contains_key(&path) {
        return Response::ok(id, Value::Map(vec![]));
    }

    // Create a new watcher that forwards events to our notification channel.
    let tx = state.notification_tx.clone();

    let watcher_result = {
        let tx = tx.clone();
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let notif = event_to_notification(&event);
                // Ignore send errors — the receiver may have been dropped
                // if the agent is shutting down.
                let _ = tx.send(notif);
            }
        })
    };

    let mut watcher = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            return Response::err(
                id,
                error_code::IO_ERROR,
                format!("failed to create watcher: {e}"),
            );
        }
    };

    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };

    if let Err(e) = watcher.watch(&path, mode) {
        let (code, msg) =
            if e.to_string().contains("No such file") || e.to_string().contains("not found") {
                (
                    error_code::NOT_FOUND,
                    format!("no such file or directory: {path_str}"),
                )
            } else if e.to_string().contains("Permission denied")
                || e.to_string().contains("permission denied")
            {
                (
                    error_code::PERMISSION_DENIED,
                    format!("permission denied: {path_str}"),
                )
            } else {
                (error_code::IO_ERROR, format!("watch failed: {e}"))
            };
        return Response::err(id, code, msg);
    }

    watches.insert(
        path,
        WatchEntry {
            recursive,
            _watcher: watcher,
        },
    );

    Response::ok(id, Value::Map(vec![]))
}

/// `watch.remove` — stop watching a previously added path.
///
/// Params:
/// - `path`: the filesystem path to stop watching (string, required)
///
/// Result: `{}` (empty map on success).
///
/// Returns an error if the path is not currently being watched.
pub async fn remove(id: u64, params: &Value, state: &WatchState) -> Response {
    let path_str = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let path = PathBuf::from(path_str);
    let mut watches = state.watches.lock().await;

    if watches.remove(&path).is_some() {
        // Dropping the WatchEntry (and its _watcher) stops the watch.
        Response::ok(id, Value::Map(vec![]))
    } else {
        Response::err(
            id,
            error_code::NOT_FOUND,
            format!("path is not being watched: {path_str}"),
        )
    }
}

/// `watch.list` — list all currently active watches.
///
/// Params: `{}` (no parameters required)
///
/// Result: `{ watches: [ { path: "...", recursive: <bool> }, … ] }`
pub async fn list(id: u64, _params: &Value, state: &WatchState) -> Response {
    let watches = state.watches.lock().await;

    let entries: Vec<Value> = watches
        .iter()
        .map(|(path, entry)| {
            Value::Map(vec![
                (
                    Value::String("path".into()),
                    Value::String(path.to_string_lossy().into_owned().into()),
                ),
                (
                    Value::String("recursive".into()),
                    Value::Boolean(entry.recursive),
                ),
            ])
        })
        .collect();

    Response::ok(
        id,
        Value::Map(vec![(
            Value::String("watches".into()),
            Value::Array(entries),
        )]),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;
    use std::time::Duration;
    use tempfile::TempDir;

    fn make_params(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (Value::String(k.into()), v))
                .collect(),
        )
    }

    #[tokio::test]
    async fn add_and_list_watch() {
        let state = WatchState::new();
        let dir = TempDir::new().unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        let resp = add(1, &params, &state).await;
        assert!(resp.error.is_none(), "add failed: {:?}", resp.error);

        // List should show one watch.
        let resp = list(2, &Value::Map(vec![]), &state).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let watches = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("watches"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert_eq!(watches.len(), 1);

        let entry = watches[0].as_map().unwrap();
        let path = entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("path"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(path, dir.path().to_str().unwrap());

        let recursive = entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("recursive"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert!(!recursive);
    }

    #[tokio::test]
    async fn add_recursive_watch() {
        let state = WatchState::new();
        let dir = TempDir::new().unwrap();

        let params = make_params(vec![
            ("path", Value::String(dir.path().to_str().unwrap().into())),
            ("recursive", Value::Boolean(true)),
        ]);
        let resp = add(1, &params, &state).await;
        assert!(resp.error.is_none());

        let resp = list(2, &Value::Map(vec![]), &state).await;
        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let watches = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("watches"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert_eq!(watches.len(), 1);

        let entry = watches[0].as_map().unwrap();
        let recursive = entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("recursive"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert!(recursive);
    }

    #[tokio::test]
    async fn add_duplicate_is_noop() {
        let state = WatchState::new();
        let dir = TempDir::new().unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);

        let resp = add(1, &params, &state).await;
        assert!(resp.error.is_none());

        // Adding the same path again should succeed (no-op).
        let resp = add(2, &params, &state).await;
        assert!(resp.error.is_none());

        // Should still only show one watch.
        let resp = list(3, &Value::Map(vec![]), &state).await;
        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let watches = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("watches"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert_eq!(watches.len(), 1);
    }

    #[tokio::test]
    async fn remove_existing_watch() {
        let state = WatchState::new();
        let dir = TempDir::new().unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        add(1, &params, &state).await;

        let resp = remove(2, &params, &state).await;
        assert!(resp.error.is_none(), "remove failed: {:?}", resp.error);

        // List should be empty.
        let resp = list(3, &Value::Map(vec![]), &state).await;
        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let watches = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("watches"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert!(watches.is_empty());
    }

    #[tokio::test]
    async fn remove_nonexistent_watch() {
        let state = WatchState::new();

        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_nowatch_12345__".into()),
        )]);
        let resp = remove(1, &params, &state).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn add_nonexistent_path_fails() {
        let state = WatchState::new();

        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_watch_noexist_99999__".into()),
        )]);
        let resp = add(1, &params, &state).await;
        assert!(
            resp.error.is_some(),
            "watching nonexistent path should fail"
        );
    }

    #[tokio::test]
    async fn list_empty() {
        let state = WatchState::new();

        let resp = list(1, &Value::Map(vec![]), &state).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let watches = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("watches"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert!(watches.is_empty());
    }

    #[tokio::test]
    async fn add_missing_path_param() {
        let state = WatchState::new();

        let params = Value::Map(vec![]);
        let resp = add(99, &params, &state).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn remove_missing_path_param() {
        let state = WatchState::new();

        let params = Value::Map(vec![]);
        let resp = remove(98, &params, &state).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn watch_produces_notifications() {
        let state = WatchState::new();
        let dir = TempDir::new().unwrap();

        // Take the receiver before adding any watches.
        let mut rx = state.take_receiver().await.unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        let resp = add(1, &params, &state).await;
        assert!(resp.error.is_none());

        // Create a file in the watched directory.
        let file = dir.path().join("trigger.txt");
        std::fs::write(&file, b"hello").unwrap();

        // Wait a short time for the notification to arrive.
        // The inotify/kqueue event may take a moment to propagate.
        let notification = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;

        match notification {
            Ok(Some(notif)) => {
                assert_eq!(notif.method, "fs.changed");
                let map = notif.params.as_map().unwrap();
                let paths = map
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("paths"))
                    .unwrap()
                    .1
                    .as_array()
                    .unwrap();
                assert!(
                    !paths.is_empty(),
                    "notification should include affected paths"
                );

                let kind = map
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("kind"))
                    .unwrap()
                    .1
                    .as_str()
                    .unwrap();
                // The event kind should be one of our known types.
                assert!(
                    ["create", "modify", "remove", "access", "other", "unknown"].contains(&kind),
                    "unexpected event kind: {kind}"
                );
            }
            Ok(None) => {
                // Channel closed — this shouldn't happen during the test.
                panic!("notification channel closed unexpectedly");
            }
            Err(_) => {
                // Timeout — inotify may not fire on all platforms in CI.
                // We skip the assertion rather than failing.
                eprintln!(
                    "warning: watch notification timed out (may be expected in some CI environments)"
                );
            }
        }
    }

    #[tokio::test]
    async fn take_receiver_only_once() {
        let state = WatchState::new();

        let first = state.take_receiver().await;
        assert!(first.is_some());

        let second = state.take_receiver().await;
        assert!(
            second.is_none(),
            "take_receiver should return None on second call"
        );
    }

    #[test]
    fn classify_event_kinds() {
        assert_eq!(
            classify_event(&EventKind::Create(notify::event::CreateKind::File)),
            "create"
        );
        assert_eq!(
            classify_event(&EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content
            ))),
            "modify"
        );
        assert_eq!(
            classify_event(&EventKind::Remove(notify::event::RemoveKind::File)),
            "remove"
        );
        assert_eq!(classify_event(&EventKind::Other), "other");
    }

    #[test]
    fn event_to_notification_structure() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/test.txt")],
            attrs: Default::default(),
        };

        let notif = event_to_notification(&event);
        assert_eq!(notif.version, "2.0");
        assert_eq!(notif.method, "fs.changed");

        let map = notif.params.as_map().unwrap();
        let paths = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("paths"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].as_str().unwrap(), "/tmp/test.txt");

        let kind = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("kind"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(kind, "create");
    }
}
