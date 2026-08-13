//! The small controls shared by the options panels.
//!
//! These started life inside `pages/cardioid.rs`; `/screensaver/:name` needs the
//! same slider, pill and collapsible section, so they live here now and both
//! pages use them.

use dioxus::prelude::*;

/// A collapsible advanced section (native `<details>`, closed by default).
#[component]
pub fn Details(summary: String, children: Element) -> Element {
    rsx! {
        details {
            style: "margin-top:10px;border-top:1px solid #333;padding-top:8px;",
            summary {
                style: "cursor:pointer;font-size:13px;color:#9aa;font-weight:600;margin-bottom:6px;",
                "{summary}"
            }
            {children}
        }
    }
}

/// A small on/off pill button.
#[component]
pub fn Toggle(label: String, on: bool, onclick: EventHandler<MouseEvent>) -> Element {
    let bg = if on {
        "background:#2f7d32;border-color:#3faf43"
    } else {
        "background:#333;border-color:#555"
    };
    rsx! {
        button {
            style: "flex:0 0 auto;padding:5px 10px;border:1px solid;border-radius:6px;\
                    color:#eee;cursor:pointer;font:inherit;font-size:13px;{bg}",
            onclick: move |e| onclick.call(e),
            "{label}"
        }
    }
}

#[component]
pub fn Slider(
    label: String,
    min: String,
    max: String,
    step: String,
    value: f64,
    decimals: u8,
    oninput: EventHandler<f64>,
) -> Element {
    let shown = format!("{:.1$}", value, decimals as usize);
    rsx! {
        div {
            style: "margin-bottom:8px;",
            div {
                style: "display:flex;justify-content:space-between;font-size:12px;color:#bbb;margin-bottom:2px;",
                span { "{label}" }
                span { style: "color:#ff8fb8;font-variant-numeric:tabular-nums;", "{shown}" }
            }
            input {
                r#type: "range", min, max, step, value: "{value}",
                style: "width:100%;accent-color:#ff4d8d;",
                oninput: move |e| {
                    if let Ok(v) = e.value().parse::<f64>() {
                        oninput.call(v);
                    }
                },
            }
        }
    }
}

/// A labelled `<select>`; the options are `(value, label)` pairs.
#[component]
pub fn Choice(
    label: String,
    value: String,
    options: Vec<(String, String)>,
    onchange: EventHandler<String>,
) -> Element {
    rsx! {
        div {
            style: "margin-bottom:8px;",
            div {
                style: "font-size:12px;color:#bbb;margin-bottom:2px;",
                "{label}"
            }
            select {
                style: "width:100%;padding:4px;background:#222;color:#eee;border:1px solid #555;\
                        border-radius:6px;font:inherit;font-size:13px;",
                onchange: move |e| onchange.call(e.value()),
                for (v , l) in options {
                    option { value: "{v}", selected: v == value, "{l}" }
                }
            }
        }
    }
}

/// Where a saver's pictures or words come from: a kind, and a name in it.
///
/// The two halves are one query parameter (`?images=%23art`), but nobody
/// should have to write that. The kind is a menu and the name is a plain
/// field, so a hashtag is typed as `art` and an account as `overby.me`.
///
/// The first kind in `kinds` is the one that needs no name (colour bars, a
/// poem); it commits the moment it is chosen. The rest commit when the field
/// is left or `Enter` is pressed, so a saver is not restarted per keystroke.
#[component]
pub fn SourcePicker(
    label: String,
    /// `(value, label)`, the first being the kind that needs no name.
    kinds: Vec<(String, String)>,
    /// What the name field should say when it is empty, per kind value.
    hints: Vec<(String, String)>,
    kind: String,
    name: String,
    onchange: EventHandler<(String, String)>,
) -> Element {
    let mut chosen = use_signal(|| kind.clone());
    let mut typed = use_signal(|| name.clone());
    let bare = kinds.first().map(|(v, _)| v.clone()).unwrap_or_default();
    let hint = hints
        .iter()
        .find(|(v, _)| *v == chosen())
        .map(|(_, h)| h.clone())
        .unwrap_or_default();

    rsx! {
        div {
            style: "margin-bottom:8px;",
            div { style: "font-size:12px;color:#bbb;margin-bottom:2px;", "{label}" }
            select {
                style: "width:100%;padding:4px;background:#222;color:#eee;border:1px solid #555;\
                        border-radius:6px;font:inherit;font-size:13px;",
                onchange: move |e| {
                    let v = e.value();
                    chosen.set(v.clone());
                    // Switching to a kind whose name is still blank leaves the
                    // saver where it is: the field is there to be filled in.
                    if v == bare || !typed().trim().is_empty() {
                        onchange.call((v, typed()));
                    }
                },
                for (v , l) in kinds.iter().cloned() {
                    option { value: "{v}", selected: v == chosen(), "{l}" }
                }
            }
            if chosen() != bare {
                input {
                    r#type: "text",
                    value: "{typed}",
                    placeholder: "{hint}",
                    spellcheck: false,
                    autocapitalize: "none",
                    // `box-sizing` because an <input> is content-box by
                    // default where a <select> is border-box, and a 100%-wide
                    // one would hang over the edge of the panel.
                    style: "box-sizing:border-box;width:100%;margin-top:6px;padding:5px 7px;\
                            background:#222;color:#eee;border:1px solid #555;border-radius:6px;\
                            font:inherit;font-size:13px;",
                    oninput: move |e| typed.set(e.value()),
                    onchange: move |e| onchange.call((chosen(), e.value())),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            onchange.call((chosen(), typed()));
                        }
                    },
                }
            }
        }
    }
}
