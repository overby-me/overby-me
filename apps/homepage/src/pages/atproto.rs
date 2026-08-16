use dioxus::prelude::*;

use crate::Route;
use crate::graph::Graph;

// Shared styling, tuned to match the graph (dark #222 canvas, Space Grotesk,
// the #ff0072 pink accent used by the tooltip).
const OVERLAY: &str = "position: fixed; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 1.2rem; padding: 1.5rem; text-align: center; background: #222222; color: #ffffff; font-family: 'Space Grotesk', system-ui, sans-serif; -webkit-user-select: none; user-select: none;";
const TITLE: &str = "margin: 0; font-size: clamp(1.6rem, 5vw, 2.6rem); font-weight: 700;";
const SUB: &str =
    "margin: 0; max-width: 34rem; font-size: 1.05rem; line-height: 1.5; color: #c9c9c9;";
const ERR: &str = "margin: 0; max-width: 34rem; font-size: 1rem; line-height: 1.5; color: #ff8fb0;";
const ROW: &str = "display: flex; flex-wrap: wrap; gap: 0.6rem; justify-content: center; width: 100%; max-width: 30rem;";
const INPUT: &str = "flex: 1 1 14rem; min-width: 0; padding: 0.7rem 0.9rem; font: inherit; font-size: 1rem; color: #ffffff; background: #2e2e2e; border: 1px solid #4a4a4a; border-radius: 0.6rem; outline: none;";
const BUTTON: &str = "flex: 0 0 auto; padding: 0.7rem 1.1rem; font: inherit; font-size: 1rem; font-weight: 600; color: #ffffff; background: #ff0072; border: none; border-radius: 0.6rem; cursor: pointer;";
const SPINNER_CSS: &str = ".atp-spinner{width:2.6rem;height:2.6rem;border:3px solid #444;border-top-color:#ff0072;border-radius:50%;animation:atp-spin 0.9s linear infinite}@keyframes atp-spin{to{transform:rotate(360deg)}}";

/// `/@handle` — resolve an atproto account and render its platform graph.
///
/// This target calls no hooks so it can branch freely: a real `@handle` request
/// delegates to [`AtProtoGraph`] (which owns the async hooks), while anything
/// else shows the landing prompt.
#[component]
pub fn AtProto(handle: String) -> Element {
    let target = handle.trim_start_matches('@').trim().to_string();
    if handle.starts_with('@') && !target.is_empty() {
        rsx! { AtProtoGraph { target } }
    } else {
        rsx! {
            div { style: OVERLAY,
                h1 { style: TITLE, "atproto graph" }
                p { style: SUB,
                    "Enter any atproto handle to see every platform that account uses, drawn as a live graph straight from its PDS."
                }
                HandleInput {}
            }
        }
    }
}

/// Resolves `target` and renders loading / graph / error. Always calls the same
/// hooks in the same order; re-resolves when `target` changes.
#[component]
fn AtProtoGraph(target: String) -> Element {
    let resolved = use_resource(use_reactive!(|target| async move {
        crate::atproto_web::resolve_graph(&target).await
    }));

    let body = match &*resolved.read() {
        None => rsx! {
            div { style: OVERLAY,
                document::Style { {SPINNER_CSS} }
                div { class: "atp-spinner" }
                p { style: SUB, "Resolving @{target}…" }
            }
        },
        Some(Ok(data)) => rsx! {
            // Key by handle so a new lookup remounts the graph with fresh state.
            Graph { key: "{target}", data: data.clone() }
        },
        Some(Err(message)) => rsx! {
            div { style: OVERLAY,
                h1 { style: TITLE, "Couldn't build a graph" }
                p { style: ERR, "{message}" }
                HandleInput {}
            }
        },
    };

    rsx! {
        // Title the tab after the handle being viewed. (Note: link-card crawlers
        // like Bluesky's don't run the wasm, so this only affects the live tab,
        // not static social previews.)
        document::Title { "@{target}" }
        {body}
    }
}

/// A handle text box that navigates to `/@<handle>` on submit.
#[component]
fn HandleInput() -> Element {
    let mut value = use_signal(String::new);
    let nav = use_navigator();

    let go = move || {
        let v = value
            .read()
            .trim()
            .trim_start_matches('@')
            .trim()
            .to_string();
        if !v.is_empty() {
            nav.push(Route::AtProto {
                handle: format!("@{v}"),
            });
        }
    };

    rsx! {
        div { style: ROW,
            input {
                style: INPUT,
                value: "{value}",
                placeholder: "yourhandle.com",
                autofocus: true,
                autocomplete: "off",
                autocapitalize: "none",
                spellcheck: "false",
                oninput: move |e| value.set(e.value()),
                onkeydown: move |e| {
                    if matches!(e.key(), Key::Enter) {
                        go();
                    }
                },
            }
            button { style: BUTTON, onclick: move |_| go(), "Show graph" }
        }
    }
}
