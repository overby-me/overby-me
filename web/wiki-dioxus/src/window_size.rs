//! Reactive Material 3 window size class (adaptive breakpoints 600/840/1200/1600
//! dp). It drives which navigation surface the shell mounts and how many panes a
//! [`crate::components`] scaffold shows, keying off window WIDTH only (never a
//! device type) so foldables and split-window resolve for free.
//!
//! A raw JS `resize` callback cannot write a `GlobalSignal` (that needs the
//! Dioxus runtime), so widths are bridged through a `use_coroutine`: the listener
//! `send`s widths into the channel and the coroutine — which runs inside the
//! runtime — is the only writer of [`WINDOW_SIZE`].

use dioxus::prelude::*;
use futures_util::StreamExt;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// The M3 window size classes, split at 600 / 840 / 1200 / 1600 dp.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowSizeClass {
    /// `< 600` — phones. Bottom navigation bar + modal drawer for the tree.
    Compact,
    /// `600–839` — small tablets. Collapsed navigation rail + modal drawer.
    Medium,
    /// `840–1199` — tablets / small laptops. Collapsed rail + modal drawer.
    Expanded,
    /// `1200–1599` — desktops. Expanded rail hosting the tree pane inline.
    Large,
    /// `>= 1600` — large desktops. Expanded rail; scaffolds may add a third pane.
    ExtraLarge,
}

impl WindowSizeClass {
    pub fn from_width(width: f64) -> Self {
        if width < 600.0 {
            Self::Compact
        } else if width < 840.0 {
            Self::Medium
        } else if width < 1200.0 {
            Self::Expanded
        } else if width < 1600.0 {
            Self::Large
        } else {
            Self::ExtraLarge
        }
    }

    /// Value for a `data-size-class` attribute (CSS hooks + debugging).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Medium => "medium",
            Self::Expanded => "expanded",
            Self::Large => "large",
            Self::ExtraLarge => "extra-large",
        }
    }

    /// Compact: primary nav is the bottom navigation bar.
    pub fn is_compact(self) -> bool {
        matches!(self, Self::Compact)
    }

    /// Medium/Expanded: primary nav is the collapsed navigation rail; the tree
    /// lives behind a modal drawer.
    pub fn is_rail(self) -> bool {
        matches!(self, Self::Medium | Self::Expanded)
    }

    /// Large/Extra-large: the rail expands to host the groups/events tree inline
    /// (the 2025 replacement for the permanent navigation drawer).
    pub fn is_expanded_rail(self) -> bool {
        matches!(self, Self::Large | Self::ExtraLarge)
    }

    /// Whether two panes (list + detail) fit side by side.
    pub fn is_dual_pane(self) -> bool {
        matches!(self, Self::Large | Self::ExtraLarge)
    }
}

fn current_width() -> f64 {
    web_sys::window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(1280.0)
}

/// The current window size class. Lazily initialised from the real window width,
/// then kept current by [`use_window_size`].
pub static WINDOW_SIZE: GlobalSignal<WindowSizeClass> =
    Signal::global(|| WindowSizeClass::from_width(current_width()));

/// Install the window `resize` -> [`WINDOW_SIZE`] bridge and return the current
/// class (reactively). Call once from the root layout.
pub fn use_window_size() -> WindowSizeClass {
    // The coroutine runs in the runtime and is the ONLY writer of WINDOW_SIZE.
    let feed = use_coroutine(|mut rx: UnboundedReceiver<f64>| async move {
        while let Some(width) = rx.next().await {
            let class = WindowSizeClass::from_width(width);
            if *WINDOW_SIZE.peek() != class {
                *WINDOW_SIZE.write() = class;
            }
        }
    });

    // Install the listener once; leak it (it lives for the app's lifetime).
    use_hook(move || {
        if let Some(win) = web_sys::window() {
            feed.send(current_width());
            let cb = Closure::<dyn FnMut()>::new(move || {
                feed.send(current_width());
            });
            let _ = win.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
            cb.forget();
        }
    });

    WINDOW_SIZE()
}
