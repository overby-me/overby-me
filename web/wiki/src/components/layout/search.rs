use dioxus::prelude::*;

use super::*;
use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeFields;
use crate::route::Route;
use crate::session::use_session;

/// Issue a search for `value` with the given scope (`scoped` = a context id to
/// restrict to, or `None` for site-wide), applying the result only if it is
/// still the latest request. Signals are `Copy`, so callers pass them by value.
pub(super) fn search_run(
    value: String,
    mut results: Signal<Vec<NodeFields>>,
    mut seq: Signal<u32>,
    token: Option<String>,
    scoped: Option<String>,
) {
    if value.trim().is_empty() {
        results.set(vec![]);
        return;
    }
    let my = seq() + 1;
    seq.set(my);
    spawn(async move {
        let nodes = graphql::search_nodes(token.as_deref(), &value, scoped.as_deref())
            .await
            .unwrap_or_default();
        if seq() == my {
            results.set(nodes);
        }
    });
}

/// Search bar with live GraphQL results
#[component]
pub(super) fn SearchBar(
    input: Signal<String>,
    results: Signal<Vec<NodeFields>>,
    on_close: EventHandler,
) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let mut input = input;
    // Monotonic request id so out-of-order responses don't clobber newer ones
    // (typing fires a query per keystroke; the last issued must win, not the
    // last to return). `search_run` owns the writes, so these stay immutable here.
    let seq = use_signal(|| 0u32);
    // Keyboard-highlighted result (arrow keys move it, Enter opens it).
    let mut selected = use_signal(|| 0usize);

    // Resolve the current context (nearest group/event) so the search can be
    // scoped to it. `in_context` toggles between context-only and site-wide.
    let route = use_route::<Route>();
    let segments = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => Vec::new(),
    };
    let cp = context_path(&segments);
    let ctx_token = session.read().access_token.clone();
    let context = use_resource(use_reactive!(|(cp, ctx_token)| async move {
        if cp.is_empty() {
            return None;
        }
        graphql::resolve_path(ctx_token.as_deref(), &cp)
            .await
            .ok()
            .flatten()
            // The NAME as well as the id: the chip has to say what it is scoping
            // to, or it is just a box the search happens to be in.
            .map(|n| (n.id.0, n.name))
    }));
    let scope_name = context
        .read()
        .clone()
        .flatten()
        .map(|(_, name)| name)
        .unwrap_or_default();
    let has_context = !scope_name.is_empty();
    // Start scoped to the section you are standing in: that is nearly always
    // what you meant, and the button widens to the whole site in one click.
    // Outside any group or event there is nothing to scope to, so this reads as
    // site-wide anyway (`scoped` resolves to None) and the button is not shown.
    let mut in_context = use_signal(|| true);

    // The bar mounts only when search opens, so the context id above resolves
    // AFTER the first keystrokes can land — and those would run site-wide while
    // the button says "in section". Re-issue the query once the id arrives.
    // Peeked, not read: this must react to the id resolving, not to the toggle
    // (whose own handler already re-runs the search).
    let resolved = context.read().clone().flatten().map(|(id, _)| id);
    use_effect(use_reactive!(|(resolved)| {
        let Some(id) = resolved.clone() else { return };
        if !*in_context.peek() {
            return;
        }
        let value = input.peek().clone();
        if value.is_empty() {
            return;
        }
        let token = session.peek().access_token.clone();
        search_run(value, results, seq, token, Some(id));
    }));

    rsx! {
        div { class: "search-box",
            // Where the search is looking, as a chip you can take off. A toggle
            // button said the same thing in an icon nobody had to read; a chip
            // NAMES the section, and removing it is the obvious way to widen the
            // search rather than something to discover. Gone means everywhere,
            // and it comes back when search is opened from that section again.
            if has_context && in_context() {
                span { class: "search-scope",
                    span { class: "material-icons search-scope-icon", "folder" }
                    span { class: "search-scope-name", "{scope_name}" }
                    button {
                        class: "search-scope-clear",
                        r#type: "button",
                        aria_label: "{t(\"common.searchEverywhere\")}",
                        title: "{t(\"common.searchEverywhere\")}",
                        onclick: move |_| {
                            in_context.set(false);
                            let token = session.read().access_token.clone();
                            search_run(input.read().clone(), results, seq, token, None);
                        },
                        span { class: "material-icons", "close" }
                    }
                }
            }
            input {
                class: "search-input",
                placeholder: "{t(\"common.search\")}",
                aria_label: "{t(\"common.search\")}",
                role: "combobox",
                aria_autocomplete: "list",
                aria_expanded: "{!results.read().is_empty()}",
                aria_controls: "search-results-list",
                // Point assistive tech at the highlighted option (keyboard focus
                // stays in the input; the option is "active" not focused).
                aria_activedescendant: if results.read().is_empty() { String::new() } else { format!("search-opt-{}", selected.read()) },
                value: "{input}",
                oninput: move |evt| {
                    let value = evt.value();
                    input.set(value.clone());
                    selected.set(0);
                    let scoped = if in_context() { context.read().clone().flatten().map(|(id, _)| id) } else { None };
                    let token = session.read().access_token.clone();
                    search_run(value, results, seq, token, scoped);
                },
                onkeydown: move |evt| {
                    let len = results.read().len();
                    match evt.key() {
                        Key::Escape => on_close.call(()),
                        Key::ArrowDown if len > 0 => {
                            let s = (*selected.read() + 1).min(len - 1);
                            selected.set(s);
                            evt.prevent_default();
                        }
                        Key::ArrowUp if len > 0 => {
                            let s = selected.read().saturating_sub(1);
                            selected.set(s);
                            evt.prevent_default();
                        }
                        Key::Enter => {
                            let idx = *selected.read();
                            if let Some(node) = results.read().get(idx) {
                                let node_id = node.id.0.clone();
                                let key = node.key.clone();
                                let token = session.read().access_token.clone();
                                spawn(async move {
                                    let segments = resolve_result_path(node_id, key, token).await;
                                    nav.push(Route::PathPage { segments, app: None });
                                    on_close.call(());
                                });
                            }
                        }
                        _ => {}
                    }
                },
            }
            // Search results dropdown
            if !results.read().is_empty() {
                div {
                    class: "search-results",
                    id: "search-results-list",
                    role: "listbox",
                    aria_label: "{t(\"common.search\")}",
                    for (idx , node) in results.read().iter().enumerate() {
                        div {
                            class: if idx == *selected.read() { "list-item selected" } else { "list-item" },
                            role: "option",
                            id: "search-opt-{idx}",
                            aria_selected: "{idx == *selected.read()}",
                            key: "{node.id.0}",
                            onclick: {
                                // A search hit can live anywhere in the tree, so
                                // resolve its full ancestor path (root excluded)
                                // rather than treating the key as a top-level
                                // segment. Fall back to the bare key if the walk
                                // yields nothing.
                                let node_id = node.id.0.clone();
                                let key = node.key.clone();
                                let on_close = on_close;
                                move |_| {
                                    let node_id = node_id.clone();
                                    let key = key.clone();
                                    let token = session.read().access_token.clone();
                                    // Resolve first, THEN navigate + close: closing
                                    // unmounts the SearchBar, cancelling the task.
                                    spawn(async move {
                                        let segments = resolve_result_path(node_id, key, token).await;
                                        nav.push(Route::PathPage { segments, app: None });
                                        on_close.call(());
                                    });
                                }
                            },
                            div { class: "avatar small",
                                {crate::components::loader::node_icon_el(node.mime_id.as_deref().unwrap_or(""), node.data.as_ref().map(|d| &d.0))}
                            }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{node.name}" }
                                if let Some(parent) = node.parent.as_ref() {
                                    div { class: "list-item-secondary", "{parent.name}" }
                                }
                            }
                        }
                    }
                }
            } else if !input.read().is_empty() {
                // DESIGN (functional): a clear "no results" state instead of an
                // empty dropdown when a query matches nothing.
                div { class: "search-results",
                    div { class: "search-no-results",
                        span { class: "material-icons", "search_off" }
                        span { "{t(\"common.noResults\")}" }
                    }
                }
            }
        }
        button {
            class: "btn-icon",
            aria_label: "{t(\"common.close\")}",
            onclick: move |_| on_close.call(()),
            span { class: "material-icons", "close" }
        }
    }
}

/// Resolve a search hit's full ancestor path (root excluded), falling back to the
/// bare key. A hit can live anywhere in the tree, so we resolve its ancestors
/// rather than treating the key as a top-level segment. Shared by a result click
/// and the Enter key.
pub(super) async fn resolve_result_path(
    node_id: String,
    key: String,
    token: Option<String>,
) -> Vec<String> {
    let mut segments = graphql::path_from_id(token.as_deref(), &node_id)
        .await
        .unwrap_or_default();
    if segments.is_empty() {
        segments = vec![key];
    }
    segments
}
