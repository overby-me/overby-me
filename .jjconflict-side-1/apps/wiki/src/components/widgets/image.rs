//! A click-to-zoom image with a full-screen lightbox and a broken-image fallback.

use dioxus::prelude::*;

/// An image that opens full-screen on click (#114). Click the overlay (or press
/// its close affordance) to dismiss. Keeps its own open/closed state.
#[component]
pub fn ZoomableImage(src: String, alt: String) -> Element {
    let mut zoomed = use_signal(|| false);
    let mut errored = use_signal(|| false);
    rsx! {
        if *errored.read() {
            // Error state: a broken-image placeholder instead of a dead <img>.
            div { class: "image-error", title: "{alt}",
                span { class: "material-icons", "broken_image" }
            }
        } else {
            img {
                src: "{src}",
                alt: "{alt}",
                // `.zoomable` fades the image in on mount (see CSS).
                class: "zoomable",
                loading: "lazy",
                decoding: "async",
                referrerpolicy: "no-referrer",
                onerror: move |_| errored.set(true),
                onclick: move |_| zoomed.set(true),
            }
        }
        if *zoomed.read() {
            // M3-style expand: a scrim fades in and the image scales up (emphasized
            // decelerate). Dismiss by clicking the scrim/image, the close button, or
            // pressing Escape.
            div {
                class: "image-lightbox",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "{alt}",
                tabindex: "-1",
                onclick: move |_| zoomed.set(false),
                onkeydown: move |e| {
                    if e.key() == Key::Escape {
                        zoomed.set(false);
                    }
                },
                // Pull focus into the overlay so Escape works immediately.
                onmounted: move |e| {
                    spawn(async move {
                        let _ = e.set_focus(true).await;
                    });
                },
                button {
                    class: "image-lightbox-close btn-icon state-layer",
                    aria_label: "close",
                    onclick: move |e| {
                        e.stop_propagation();
                        zoomed.set(false);
                    },
                    span { class: "material-icons", "close" }
                }
                img { class: "image-lightbox-img", src: "{src}", alt: "{alt}" }
            }
        }
    }
}
