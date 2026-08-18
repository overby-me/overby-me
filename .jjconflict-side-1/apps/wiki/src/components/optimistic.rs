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
//!
//! # Clear pending rows when the thing they belong to changes
//!
//! A pending row belongs to ONE node — the folder it was added under, the post it
//! was commented on, the speaker list it joined. The route components here are
//! deliberately not remounted when the route changes (they refetch reactively
//! instead; see `FolderApp`), so a `use_signal` holding pending rows survives
//! navigation and is then reconciled against a DIFFERENT node's data. Its keys
//! can never appear there, so [`reconcile_by_key`] keeps every row forever and a
//! muted "sending" row sits on a page that finished loading.
//!
//! Adding a folder made this certain rather than merely possible, since it
//! navigates into the node it just created. Every holder of pending state
//! therefore resets it on the identity it belongs to:
//!
//! ```ignore
//! let shown_node = node_id.clone();
//! use_effect(use_reactive!(|(shown_node)| {
//!     let _ = &shown_node;
//!     if !pending.peek().is_empty() {
//!         pending.set(Vec::new());
//!     }
//! }));
//! ```

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
