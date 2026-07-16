//! Shared optimistic-UI helpers.
//!
//! The app updates the UI after a mutation via a round-trip (a refetch or a live
//! subscription), so a user's action is not reflected until the server confirms.
//! These helpers make that instant: reflect the action locally at once, then
//! reconcile against the fetched data and roll back on error.
//!
//! Three recipes are built on this:
//!   (a) reconcile-by-key for list INSERTS: push a pending row keyed by a
//!       client-generated key, render it muted, and drop it once the refetch
//!       returns a real row with the same key (see [`reconcile_by_key`]).
//!   (b) a local override signal for TOGGLES/reorders: shadow the server value,
//!       flip it immediately, clear it when the refetch matches or on error.
//!   (c) immediate local REMOVAL for deletes: hold a set of removed ids and
//!       filter them from the render, restoring on error.
//!
//! The comments feature (src/components/comments.rs) is the original of recipe (a).

use std::collections::HashSet;

/// Keep only the pending (optimistic) rows whose key has NOT yet come back from
/// the server. Once the refetch includes a row with the same key, its optimistic
/// row is dropped: no duplicate, no flicker. Generic over the pending row type via
/// a key extractor, so every feature can reuse one reconciliation.
pub fn reconcile_by_key<T, F>(pending: &[T], key_of: F, fetched_keys: &HashSet<String>) -> Vec<T>
where
    T: Clone,
    F: Fn(&T) -> &str,
{
    pending
        .iter()
        .filter(|p| !fetched_keys.contains(key_of(p)))
        .cloned()
        .collect()
}
