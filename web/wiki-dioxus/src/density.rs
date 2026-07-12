//! EXPERIMENT (functional): a UI density preference (comfortable / compact).
//!
//! When compact, lists and cards tighten their spacing so more fits on screen —
//! useful for large rosters and deep folders. Persisted in localStorage and
//! surfaced as `data-density="compact"` on the app shell, which the stylesheet
//! keys off. Mirrors the theme-preference plumbing.

use dioxus::prelude::*;

/// True when the user prefers compact density.
pub static COMPACT_DENSITY: GlobalSignal<bool> = Signal::global(|| false);

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Load the stored density preference into the signal (call once at startup).
pub fn load_density() {
    if let Some(s) = local_storage() {
        if let Ok(Some(v)) = s.get_item("wiki_density") {
            *COMPACT_DENSITY.write() = v == "compact";
        }
    }
}

/// Set and persist the density preference.
pub fn set_compact(compact: bool) {
    *COMPACT_DENSITY.write() = compact;
    if let Some(s) = local_storage() {
        let _ = s.set_item(
            "wiki_density",
            if compact { "compact" } else { "comfortable" },
        );
    }
}
