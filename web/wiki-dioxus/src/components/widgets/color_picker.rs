//! A reusable Material 3 colour-swatch row + the freeform custom-colour chip.

use dioxus::prelude::*;

/// A reusable Material Design 3 colour-swatch row: a label, preset swatches, and
/// a trailing custom "any colour" chip ([`CustomColorSwatch`]) as the last circle.
/// Emits the chosen `#rrggbb` hex through `on_change`; `value` is the active
/// colour (it highlights a matching swatch). App-agnostic: labels are passed in.
#[component]
pub fn ColorPicker(
    label: String,
    value: String,
    swatches: Vec<String>,
    on_change: EventHandler<String>,
    custom_title: Option<String>,
) -> Element {
    rsx! {
        div { class: "color-picker",
            span { class: "color-picker-label", "{label}" }
            div { class: "color-swatches",
                for swatch in swatches.iter().cloned() {
                    {
                        let is_active = swatch.eq_ignore_ascii_case(&value);
                        let picked = swatch.clone();
                        rsx! {
                            button {
                                key: "{swatch}",
                                r#type: "button",
                                class: if is_active { "color-swatch active" } else { "color-swatch" },
                                style: "background-color: {swatch};",
                                title: "{swatch}",
                                onclick: move |_| on_change.call(picked.clone()),
                                if is_active {
                                    span { class: "material-icons", "check" }
                                }
                            }
                        }
                    }
                }
                // The last circle is the freeform custom picker.
                CustomColorSwatch { value: value.clone(), on_change, title: custom_title.clone() }
            }
        }
    }
}

/// The "any colour" chip: a rainbow swatch wrapping a native OS colour input.
/// Emits the chosen `#rrggbb` through `on_change`. Pairs with [`ColorPicker`].
#[component]
pub fn CustomColorSwatch(
    value: String,
    on_change: EventHandler<String>,
    title: Option<String>,
) -> Element {
    rsx! {
        label {
            class: "color-swatch color-swatch-custom",
            title: title.clone().unwrap_or_default(),
            span { class: "material-icons", "colorize" }
            input {
                r#type: "color",
                class: "color-input-native",
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}
