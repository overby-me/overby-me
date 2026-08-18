//! An M3 Expressive connected button group (single-select).
//!
//! The name is historical: this was a segmented button, which the expressive
//! update deprecated in favour of the connected button group that replaces it.
//! The widget keeps its name so no call site churns; what changed is the
//! behaviour — each button is its own shape and morphs when pressed and when
//! selected, rather than being a slice of one clipped bar.

use dioxus::prelude::*;

/// A single-select connected button group: a row of icon buttons, the chosen one
/// filled with secondary-container and shape-morphed. Emits the chosen value
/// through `on_select`. `segments` is a list of `(value, material-icon)` pairs.
#[component]
pub fn SegmentedButton(
    segments: Vec<(String, String)>,
    selected: String,
    on_select: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "m3-segmented", role: "group",
            for (value , icon) in segments {
                {
                    let is_sel = value == selected;
                    let chosen = value.clone();
                    rsx! {
                        button {
                            key: "{value}",
                            r#type: "button",
                            class: if is_sel { "m3-segment selected state-layer" } else { "m3-segment state-layer" },
                            "aria-pressed": if is_sel { "true" } else { "false" },
                            onclick: move |_| on_select.call(chosen.clone()),
                            if is_sel {
                                span { class: "material-icons m3-segment-check", "check" }
                            }
                            span { class: "material-icons", "{icon}" }
                        }
                    }
                }
            }
        }
    }
}
