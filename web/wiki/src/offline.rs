//! What the app can still show when the network is gone.
//!
//! The service worker already keeps the app itself available offline (its
//! cache-first rule over `/assets`), which produced a shell that loads
//! instantly and then says every page is empty — arguably worse than not
//! loading at all. This keeps the answers too: the last successful read of a
//! page is written to `localStorage`, and served if the same read later fails
//! because nothing could be reached.
//!
//! Deliberately narrow:
//!
//! - **Network first, always.** The cache is a fallback, never a shortcut. A
//!   reader on a working connection sees live data, so nothing here can serve a
//!   stale agenda to a room following along.
//! - **Only [`Failure::Offline`](crate::errors::Failure::Offline) falls back.**
//!   A refusal is an answer: serving a copy from before someone lost access
//!   would be the app quietly overriding a permission change.
//! - **Only what is read on the way to a page** — the node, its crumbs. Not
//!   rosters, not the bin, not feedback: those carry other people's details and
//!   are worth less offline than they cost to leave lying in storage.
//!
//! Entries are capped and evicted oldest-first, because `localStorage` is a few
//! megabytes for the whole origin and the session lives there too. Losing a
//! cached page costs a reader nothing; losing their session logs them out.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Key prefix, so eviction can tell this cache from everything else the app
/// keeps in `localStorage` (the session, the density and folder-view prefs).
const PREFIX: &str = "wiki.read.";
/// The insertion order of the cached keys, oldest first.
const INDEX_KEY: &str = "wiki.read.index";
/// How many pages to keep. Fifty is a deep session of reading and, at the size
/// of a node with its children, a small fraction of the origin's budget.
const MAX_ENTRIES: usize = 50;

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn index(store: &web_sys::Storage) -> Vec<String> {
    store
        .get_item(INDEX_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn set_index(store: &web_sys::Storage, keys: &[String]) {
    if let Ok(json) = serde_json::to_string(keys) {
        let _ = store.set_item(INDEX_KEY, &json);
    }
}

/// Remember the answer to a read, under a key the caller can rebuild.
///
/// Failures here are silent by design: a full quota or a browser with storage
/// disabled means no offline copy, which is where the app already was.
pub fn put<T: Serialize>(key: &str, value: &T) {
    let Some(store) = storage() else { return };
    let Ok(json) = serde_json::to_string(value) else {
        return;
    };
    let full = format!("{PREFIX}{key}");

    let mut keys = index(&store);
    keys.retain(|k| k != &full);
    keys.push(full.clone());
    // Evict oldest first, and keep evicting while the write fails: a quota error
    // is the only signal the browser gives about how much room is left, and the
    // session must survive this.
    while keys.len() > MAX_ENTRIES {
        let oldest = keys.remove(0);
        let _ = store.remove_item(&oldest);
    }
    let mut attempts = 0;
    while store.set_item(&full, &json).is_err() {
        attempts += 1;
        if keys.len() <= 1 || attempts > MAX_ENTRIES {
            // No room for this page even alone: drop it and leave the rest.
            keys.retain(|k| k != &full);
            set_index(&store, &keys);
            return;
        }
        let oldest = keys.remove(0);
        let _ = store.remove_item(&oldest);
    }
    set_index(&store, &keys);
}

/// The last remembered answer to a read, if there is one and it still parses.
///
/// A copy written by an older build whose shape has since changed simply fails
/// to parse and counts as a miss, so a model change cannot resurrect a value
/// the rest of the app can no longer read.
pub fn get<T: DeserializeOwned>(key: &str) -> Option<T> {
    let store = storage()?;
    let json = store.get_item(&format!("{PREFIX}{key}")).ok().flatten()?;
    serde_json::from_str(&json).ok()
}

/// Forget every cached page, leaving the session and the preferences alone.
/// Called on sign-out: the next person at this device is not the last one.
pub fn clear() {
    let Some(store) = storage() else { return };
    for key in index(&store) {
        let _ = store.remove_item(&key);
    }
    let _ = store.remove_item(INDEX_KEY);
}
