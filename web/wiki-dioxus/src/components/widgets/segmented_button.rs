//! An M3 single-select segmented button.

use dioxus::prelude::*;

/// An M3 single-select segmented button: a connected row of icon segments with
/// the selected one filled (secondary-container). Emits the chosen value through
/// `on_select`. `segments` is a list of `(value, material-icon)` pairs.
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
