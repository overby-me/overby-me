use std::sync::atomic::{AtomicBool, Ordering};

use crate::model;
use dioxus::prelude::*;

use crate::components::richtext;
use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// Maximum length for a node's display name (#111). Applied as `maxlength` on
/// the name inputs (editor title, add-content form).
pub const NODE_NAME_MAXLEN: usize = 120;

/// DOM id of the `contenteditable` editing surface.
const EDITOR_ID: &str = "rich-editor";

/// Idle time after the last keystroke before the editor silently autosaves the
/// draft, so a crash or accidental close never loses more than this window.
const AUTOSAVE_DEBOUNCE_MS: u32 = 2500;

/// Set while the open editor holds unsaved edits. A plain atomic (not a signal)
/// so the `beforeunload` callback can read it without touching Dioxus's reactive
/// graph — the callback outlives the editor, but the flag is cleared on unmount.
static EDITOR_DIRTY: AtomicBool = AtomicBool::new(false);

/// Mark / clear the unsaved-edits flag the `beforeunload` guard consults.
fn set_editor_dirty(dirty: bool) {
    EDITOR_DIRTY.store(dirty, Ordering::Relaxed);
}

/// Install a one-time, app-lifetime `beforeunload` guard that prompts the user
/// before a tab close / reload while [`EDITOR_DIRTY`] is set. In-app navigation
/// is handled by saving before we route away; this covers the browser-level
/// exits the router cannot intercept.
#[cfg(target_arch = "wasm32")]
fn install_unsaved_guard() {
    use std::sync::Once;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(win) = web_sys::window() else {
            return;
        };
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(|e: web_sys::Event| {
            if EDITOR_DIRTY.load(Ordering::Relaxed) {
                // Cancelling beforeunload makes modern engines show their generic
                // "leave site?" prompt; `returnValue` is the legacy trigger some
                // still require.
                e.prevent_default();
                let _ = js_sys::Reflect::set(
                    e.as_ref(),
                    &JsValue::from_str("returnValue"),
                    &JsValue::from_str(""),
                );
            }
        });
        let _ = win.add_event_listener_with_callback("beforeunload", cb.as_ref().unchecked_ref());
        // The listener must outlive this call; it is installed exactly once.
        cb.forget();
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn install_unsaved_guard() {}

/// The block-type key for the caret, from the browser's `formatBlock` state.
fn current_block() -> String {
    match richtext::query_value("formatBlock").as_str() {
        "h1" => "heading-one",
        "h2" => "heading-two",
        "h3" => "heading-three",
        "h4" => "heading-four",
        "h5" => "heading-five",
        "h6" => "heading-six",
        "blockquote" => "block-quote",
        "pre" => "block-pre",
        _ => "paragraph",
    }
    .to_string()
}

/// Sync the toolbar's active-state signals from the current selection.
fn refresh_toolbar(
    mut bold: Signal<bool>,
    mut italic: Signal<bool>,
    mut underline: Signal<bool>,
    mut strike: Signal<bool>,
    mut block: Signal<String>,
) {
    bold.set(richtext::query_state("bold"));
    italic.set(richtext::query_state("italic"));
    underline.set(richtext::query_state("underline"));
    strike.set(richtext::query_state("strikeThrough"));
    block.set(current_block());
}

/// Whether a node type carries authors (members). Contexts and a few vote types
/// do not, matching the React editor.
fn node_takes_authors(mime_id: Option<&str>) -> bool {
    // wiki/file is excluded too: FileApp never displays author chips, so collecting
    // authors on files only created hidden member rows (an inconsistency with how
    // ContentApp both stores and shows them).
    !matches!(
        mime_id,
        Some(
            "wiki/group"
                | "wiki/event"
                | "vote/position"
                | "vote/candidate"
                | "wiki/folder"
                | "wiki/file"
        )
    )
}

/// Append an author (deduped) and clear the input + suggestions.
fn add_author(
    mut authors: Signal<Vec<graphql::Author>>,
    mut input: Signal<String>,
    mut suggestions: Signal<Vec<graphql::Author>>,
    author: graphql::Author,
) {
    let exists = authors
        .read()
        .iter()
        .any(|a| a.name == author.name && a.node_id == author.node_id);
    if !exists && !author.name.trim().is_empty() {
        authors.write().push(author);
    }
    input.set(String::new());
    suggestions.set(vec![]);
}

/// Author autocomplete: type a name to search groups and users (or add a
/// free-text author with Enter), shown as removable chips. Mirrors the React
/// `AuthorTextField`; the list is persisted as the node's members on save.
#[component]
fn AuthorField(authors: Signal<Vec<graphql::Author>>) -> Element {
    let session = use_session();
    let mut authors = authors;
    let mut input = use_signal(String::new);
    let mut suggestions = use_signal(Vec::<graphql::Author>::new);
    // Monotonic id so out-of-order search responses don't clobber newer ones.
    let mut seq = use_signal(|| 0u32);

    rsx! {
        div { class: "author-field mb-2",
            label { "{t(\"content.authors\")}" }
            div { class: "chip-row",
                for (i , a) in authors.read().iter().enumerate() {
                    div { class: "chip", key: "{i}",
                        super::loader::UserPopover {
                            name: a.name.clone(),
                            avatar_url: a.avatar_url.clone(),
                            user_id: a.user_id.clone(),
                            span { class: "material-icons",
                                if a.node_id.is_some() { "groups" } else { "face" }
                            }
                            span { "{a.name}" }
                        }
                        button {
                            class: "chip-remove",
                            r#type: "button",
                            aria_label: "{t(\"common.remove\")}",
                            onclick: move |_| {
                                authors.write().remove(i);
                            },
                            span { class: "material-icons", "close" }
                        }
                    }
                }
            }
            div { class: "author-input-wrap",
                input {
                    r#type: "text",
                    placeholder: "{t(\"content.addAuthor\")}",
                    value: "{input}",
                    oninput: move |evt| {
                        let val = evt.value();
                        input.set(val.clone());
                        let my = *seq.read() + 1;
                        seq.set(my);
                        if val.trim().is_empty() {
                            suggestions.set(vec![]);
                            return;
                        }
                        let token = session.read().access_token.clone();
                        spawn(async move {
                            let res = graphql::search_authors(token.as_deref(), &val).await;
                            if *seq.read() == my {
                                suggestions.set(res);
                            }
                        });
                    },
                    onkeydown: move |evt| {
                        if evt.key().to_string() == "Enter" {
                            evt.prevent_default();
                            let name = input.read().trim().to_string();
                            add_author(authors, input, suggestions, graphql::Author { name, node_id: None, avatar_url: String::new(), user_id: None });
                        }
                    },
                }
                if !suggestions.read().is_empty() {
                    div { class: "author-suggestions",
                        for s in suggestions.read().iter() {
                            {
                                let chosen = s.clone();
                                let icon = if s.node_id.is_some() { "groups" } else { "face" };
                                rsx! {
                                    button {
                                        class: "author-suggestion",
                                        r#type: "button",
                                        key: "{s.node_id:?}{s.name}",
                                        onclick: move |_| add_author(authors, input, suggestions, chosen.clone()),
                                        span { class: "material-icons", "{icon}" }
                                        span { "{s.name}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// EditorApp: a WYSIWYG rich text editor over a `contenteditable` surface,
/// reading and writing the same Slate JSON model as the reference wiki.
#[component]
pub fn EditorApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let node_id = node.id.0.clone();
    let nav = use_navigator();
    // The node's own path, so saving/publishing returns to the rendered node
    // (its non-app view) rather than leaving the user in the editor.
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };

    let mut title = use_signal(|| node.name.clone());
    let mut saving = use_signal(|| false);
    // Submit (publish) is irreversible — makes the node immutable — so it goes
    // through a confirm dialog carrying the submit warning.
    let mut confirm_submit = use_signal(|| false);

    // Authors (members): content nodes carry a list of authors that the editor
    // maintains; contexts and a few vote types do not.
    let takes_authors = node_takes_authors(node.mime_id.as_deref());
    let authors = use_signal(|| {
        node.members
            .iter()
            .filter(|m| !m.hidden)
            .map(|m| graphql::Author {
                name: m.label(),
                node_id: m.node_id.as_ref().map(|u| u.0.clone()),
                avatar_url: m
                    .user
                    .as_ref()
                    .map(|u| u.avatar_url.clone())
                    .unwrap_or_default(),
                user_id: m.user.as_ref().map(|u| u.id.0.clone()),
            })
            .filter(|a| !a.name.is_empty())
            .collect::<Vec<_>>()
    });

    // The initial editor HTML, rendered once from the stored Slate content.
    let initial_html = use_hook(|| {
        node.data
            .as_ref()
            .and_then(|d| d.0.get("content"))
            .map(richtext::slate_to_html)
            .unwrap_or_else(|| "<p><br></p>".to_string())
    });

    // Content metadata (mirrors the old wiki's editor sidebar): an optional cover
    // image stored as `data.image`, and — for context owners — the node's date
    // (`createdAt`) so agenda items and minutes can be dated. The image file id is
    // seeded from the node's data and replaced via the uploader; a fresh upload
    // records its filename so the UI can confirm it before save.
    let is_owner = node.is_context_owner.unwrap_or(false);
    let initial_image = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("image"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut image_id = use_signal(|| initial_image.clone());
    let mut image_name = use_signal(String::new);
    let mut image_uploading = use_signal(|| false);
    let mut created_date = use_signal(|| {
        node.created_at
            .as_ref()
            .map(|t| t.0.chars().take(10).collect::<String>())
            .unwrap_or_default()
    });
    // The existing cover thumbnail, resolved once on mount (a fresh upload shows a
    // filename confirmation instead, since this hook does not re-fetch).
    let existing_image_url =
        super::loader::use_file_object_url(initial_image.clone().unwrap_or_default());

    // Toolbar active state (reflects the caret).
    let st_bold = use_signal(|| false);
    let st_italic = use_signal(|| false);
    let st_underline = use_signal(|| false);
    let st_strike = use_signal(|| false);
    let st_block = use_signal(|| "paragraph".to_string());

    // Link editor popover.
    let mut link_open = use_signal(|| false);
    let mut link_url = use_signal(String::new);

    // Unsaved-changes tracking (#144). `dirty` drives the debounced autosave and
    // mirrors the atomic the beforeunload guard reads; `autosave_seq` lets a
    // scheduled save bail out when the user has kept typing.
    let is_mutable = node.mutable;
    let mut dirty = use_signal(|| false);
    let mut autosave_seq = use_signal(|| 0u32);
    // Install the beforeunload guard once, and clear the unsaved flag when this
    // editor unmounts (saved or routed away) so no stale guard stays armed.
    use_hook(install_unsaved_guard);
    use_drop(|| set_editor_dirty(false));

    let handle_save = {
        let token = session.read().access_token.clone();
        let node_id = node_id.clone();
        let segments = segments.clone();
        // Preserve the node's other `data` keys (e.g. a cover `image`) that this
        // editor does not manage; save only overwrites the `content` tree.
        let base_data = node
            .data
            .as_ref()
            .and_then(|d| d.0.as_object().cloned())
            .unwrap_or_default();
        move |mutable: bool| {
            let token = token.clone();
            let node_id = node_id.clone();
            let segments = segments.clone();
            let base_data = base_data.clone();
            let title_val = title.read().clone();
            spawn(async move {
                saving.set(true);

                // Content nodes require at least one author; replace the node's
                // members with the edited list.
                if takes_authors {
                    let author_list = authors.read().clone();
                    if author_list.is_empty() {
                        show_snackbar(&t("content.addAtLeastOneAuthor"));
                        saving.set(false);
                        return;
                    }
                    if let Err(e) =
                        graphql::set_node_authors(token.as_deref(), &node_id, &author_list).await
                    {
                        log::error!("Saving authors failed: {e}");
                        show_snackbar(&t("error.somethingWentWrong"));
                        saving.set(false);
                        return;
                    }
                }

                // Serialize the live editor DOM back to Slate JSON.
                let content_json = richtext::serialize_editor(EDITOR_ID).unwrap_or_else(
                    || serde_json::json!([{ "type": "paragraph", "children": [{"text": ""}] }]),
                );
                // Drop a stray blank first line the contenteditable may leave.
                let content_json = richtext::strip_leading_empty_paragraph(content_json);
                let mut data_obj = base_data;
                data_obj.insert("content".to_string(), content_json);
                // Cover image: persist `data.image` when set, drop it when cleared.
                match image_id.read().clone() {
                    Some(id) => {
                        data_obj.insert("image".to_string(), serde_json::Value::String(id));
                    }
                    None => {
                        data_obj.remove("image");
                    }
                }
                let data = serde_json::Value::Object(data_obj);
                // Backdating: owners may set the node's date; others leave it.
                let created_at = if is_owner {
                    let d = created_date.read().trim().to_string();
                    if d.is_empty() {
                        None
                    } else {
                        Some(model::Timestamptz(d))
                    }
                } else {
                    None
                };

                let set = model::NodesSetInput {
                    name: Some(title_val),
                    data: Some(model::Jsonb(data)),
                    mutable: Some(mutable),
                    created_at,
                    ..Default::default()
                };

                match graphql::update_node(token.as_deref(), &node_id, set).await {
                    Ok(true) => {
                        // Saved: disarm the unsaved-changes guard before we route
                        // away from the editor.
                        dirty.set(false);
                        set_editor_dirty(false);
                        show_snackbar(&t("common.save"));
                        // Invalidate the cached node so the view we return to
                        // shows the saved content instead of the stale copy.
                        crate::session::bump_data_version();
                        // Return to the node we just edited. Empty segments is the
                        // root node, which has its own `/` route (`Home`).
                        if segments.is_empty() {
                            nav.push(Route::Home { app: None });
                        } else {
                            nav.push(Route::PathPage {
                                segments: segments.clone(),
                                app: None,
                            });
                        }
                    }
                    Ok(false) => show_snackbar(&t("error.somethingWentWrong")),
                    Err(e) => {
                        log::error!("Save failed: {e}");
                        show_snackbar(&t("error.somethingWentWrong"));
                    }
                }

                saving.set(false);
            });
        }
    };

    // Debounced silent autosave of the in-progress draft (#144). Content only —
    // it never touches authors, never navigates, and never bumps the data
    // version (which would remount and blank the live editor surface).
    let autosave = {
        let token = session.read().access_token.clone();
        let node_id = node_id.clone();
        let base_data = node
            .data
            .as_ref()
            .and_then(|d| d.0.as_object().cloned())
            .unwrap_or_default();
        move || {
            // Only autosave a still-mutable draft, and never while a manual
            // save/submit is in flight (that navigates away on success).
            if !is_mutable || *saving.peek() {
                return;
            }
            let token = token.clone();
            let node_id = node_id.clone();
            let base_data = base_data.clone();
            let title_val = title.peek().clone();
            let seq = *autosave_seq.peek();
            spawn(async move {
                let content_json = richtext::serialize_editor(EDITOR_ID).unwrap_or_else(
                    || serde_json::json!([{ "type": "paragraph", "children": [{"text": ""}] }]),
                );
                let content_json = richtext::strip_leading_empty_paragraph(content_json);
                let mut data_obj = base_data;
                data_obj.insert("content".to_string(), content_json);
                // Carry the current cover image so an autosave between an image
                // change and a manual save does not drop it (peek: no subscribe).
                match image_id.peek().clone() {
                    Some(id) => {
                        data_obj.insert("image".to_string(), serde_json::Value::String(id));
                    }
                    None => {
                        data_obj.remove("image");
                    }
                }
                let set = model::NodesSetInput {
                    name: Some(title_val),
                    data: Some(model::Jsonb(serde_json::Value::Object(data_obj))),
                    mutable: Some(true),
                    ..Default::default()
                };
                if let Ok(true) = graphql::update_node(token.as_deref(), &node_id, set).await {
                    // Only disarm the guard if no newer keystroke queued another
                    // autosave since we captured `seq`.
                    if *autosave_seq.peek() == seq {
                        dirty.set(false);
                        set_editor_dirty(false);
                    }
                }
            });
        }
    };

    // Mark dirty and (re)start the autosave debounce. Each keystroke bumps the
    // sequence; the scheduled task only fires if it is still the latest edit.
    let schedule_autosave = move || {
        dirty.set(true);
        set_editor_dirty(true);
        let seq = *autosave_seq.peek() + 1;
        autosave_seq.set(seq);
        let autosave = autosave.clone();
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(AUTOSAVE_DEBOUNCE_MS).await;
            if *autosave_seq.peek() == seq {
                autosave();
            }
        });
    };
    let mut schedule_title = schedule_autosave.clone();
    let mut schedule_editor = schedule_autosave;

    // Upload a chosen cover image to storage, then remember its id + filename so
    // the next save writes `data.image`. Mirrors the folder file uploader.
    let on_pick_image = move |evt: FormEvent| {
        let files = evt.files();
        let Some(fd) = files.into_iter().next() else {
            return;
        };
        let name = fd.name();
        let ctype = fd.content_type().unwrap_or_default();
        let token = session.read().access_token.clone();
        image_uploading.set(true);
        spawn(async move {
            match fd.read_bytes().await {
                Ok(bytes) => {
                    match crate::nhost::upload_file(token.as_deref(), bytes.to_vec(), &name, &ctype)
                        .await
                    {
                        Ok(up) => {
                            image_id.set(Some(up.id));
                            image_name.set(name);
                            dirty.set(true);
                            set_editor_dirty(true);
                        }
                        Err(e) => show_snackbar(&format!("{}: {e}", t("error.somethingWentWrong"))),
                    }
                }
                Err(_) => show_snackbar(&t("error.somethingWentWrong")),
            }
            image_uploading.set(false);
        });
    };

    if !is_auth {
        // DESIGN: an expressive locked-barrier state instead of a plain card.
        return rsx! {
            div { class: "card",
                div { class: "empty-state",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "lock" }
                    }
                    p { class: "empty-state-body", "{t(\"node.documentUnavailable\")}" }
                }
            }
        };
    }

    // Seed HTML moved into the mount handler (the surface is populated once).
    let cmd_seed = initial_html.clone();

    rsx! {
        div { class: "card",
            div { class: "card-content",
                // Title field. maxlength caps the node name length (#111).
                div { class: "text-field mb-2",
                    label { "{t(\"common.title\")}" }
                    input {
                        r#type: "text",
                        maxlength: "{NODE_NAME_MAXLEN}",
                        value: "{title}",
                        oninput: move |evt| {
                            title.set(evt.value());
                            schedule_title();
                        },
                    }
                }

                // Authors (members), content nodes only.
                if takes_authors {
                    AuthorField { authors }
                }

                // Content metadata: an optional cover image, and (owners) the date.
                if takes_authors {
                    div { class: "mt-2 mb-2",
                        div { class: "file-upload-label", "{t(\"content.coverImage\")}" }
                        label { class: "file-upload",
                            input {
                                r#type: "file",
                                accept: "image/*",
                                class: "file-upload-input",
                                onchange: on_pick_image,
                            }
                            span { class: "material-icons", "image" }
                            span { class: "file-upload-text", "{t(\"content.uploadImage\")}" }
                        }
                        if *image_uploading.read() {
                            div {
                                class: "stack stack-h mt-1",
                                style: "align-items: center; gap: 8px;",
                                div { class: "spinner" }
                                span { class: "body-small text-muted",
                                    "{t(\"content.uploadImage\")}\u{2026}"
                                }
                            }
                        } else if !image_name.read().is_empty() {
                            div {
                                class: "file-upload-done stack stack-h mt-1",
                                style: "align-items: center; gap: 8px;",
                                span { class: "material-icons", "check_circle" }
                                span { class: "flex-grow", "{image_name}" }
                                button {
                                    class: "btn btn-text",
                                    onclick: move |_| {
                                        image_id.set(None);
                                        image_name.set(String::new());
                                        dirty.set(true);
                                        set_editor_dirty(true);
                                    },
                                    "{t(\"content.removeImage\")}"
                                }
                            }
                        } else if image_id.read().is_some() {
                            div {
                                class: "stack stack-h mt-1",
                                style: "align-items: center; gap: 8px;",
                                if let Some(url) = existing_image_url.clone() {
                                    img {
                                        src: "{url}",
                                        alt: t("content.imageAlt"),
                                        style: "width: 56px; height: 56px; object-fit: cover; border-radius: var(--md-sys-shape-corner-small, 8px);",
                                    }
                                }
                                span { class: "flex-grow body-small text-muted",
                                    "{t(\"content.coverImage\")}"
                                }
                                button {
                                    class: "btn btn-text",
                                    onclick: move |_| {
                                        image_id.set(None);
                                        dirty.set(true);
                                        set_editor_dirty(true);
                                    },
                                    "{t(\"content.removeImage\")}"
                                }
                            }
                        }
                    }
                    if is_owner {
                        div { class: "text-field mb-2",
                            label { "{t(\"content.date\")}" }
                            input {
                                r#type: "date",
                                value: "{created_date}",
                                oninput: move |e| {
                                    created_date.set(e.value());
                                    dirty.set(true);
                                    set_editor_dirty(true);
                                },
                            }
                        }
                    }
                }

                // Sticky toolbar (#94): action buttons + formatting controls
                // stay pinned while scrolling a long document.
                div { class: "editor-toolbar",
                    // Action buttons (save / submit).
                    div { class: "stack stack-h mb-1",
                        button {
                            class: "btn btn-primary",
                            disabled: *saving.read(),
                            onclick: {
                                let save = handle_save.clone();
                                move |_| save(true)
                            },
                            span { class: "material-icons", "save" }
                            " {t(\"common.save\")}"
                        }
                        if node.mutable {
                            button {
                                class: "btn btn-secondary",
                                disabled: *saving.read(),
                                onclick: move |_| confirm_submit.set(true),
                                span { class: "material-icons", "publish" }
                                " {t(\"content.submit\")}"
                            }
                            super::widgets::Dialog {
                                open: confirm_submit(),
                                on_dismiss: move |_| confirm_submit.set(false),
                                headline: t("content.submit"),
                                icon: "publish".to_string(),
                                actions: rsx! {
                                    button {
                                        class: "btn btn-outlined",
                                        onclick: move |_| confirm_submit.set(false),
                                        "{t(\"common.cancel\")}"
                                    }
                                    button {
                                        class: "btn btn-primary",
                                        onclick: {
                                            let save = handle_save.clone();
                                            move |_| {
                                                confirm_submit.set(false);
                                                save(false);
                                            }
                                        },
                                        "{t(\"content.submit\")}"
                                    }
                                },
                                p { class: "body-medium", "{t(\"content.submitWarning\")}" }
                            }
                        }
                        if *saving.read() {
                            div { class: "spinner" }
                        }
                    }

                    // Formatting controls.
                    div { class: "editor-tools",
                        // Block style.
                        select {
                            class: "editor-select",
                            title: "{t(\"editor.style\")}",
                            value: "{st_block}",
                            onmousedown: move |_| richtext::save_selection(),
                            onchange: move |evt| {
                                let tag = match evt.value().as_str() {
                                    "heading-one" => "<h1>",
                                    "heading-two" => "<h2>",
                                    "heading-three" => "<h3>",
                                    "heading-four" => "<h4>",
                                    "heading-five" => "<h5>",
                                    "heading-six" => "<h6>",
                                    "block-quote" => "<blockquote>",
                                    "block-pre" => "<pre>",
                                    _ => "<p>",
                                };
                                richtext::focus_editor(EDITOR_ID);
                                richtext::restore_selection();
                                richtext::exec_value("formatBlock", tag);
                                refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block);
                            },
                            option { value: "paragraph", "{t(\"editor.paragraph\")}" }
                            option { value: "heading-one", "{t(\"editor.headingOne\")}" }
                            option { value: "heading-two", "{t(\"editor.headingTwo\")}" }
                            option { value: "heading-three", "{t(\"editor.headingThree\")}" }
                            option { value: "heading-four", "{t(\"editor.headingFour\")}" }
                            option { value: "heading-five", "{t(\"editor.headingFive\")}" }
                            option { value: "heading-six", "{t(\"editor.headingSix\")}" }
                            option { value: "block-quote", "{t(\"editor.blockQuote\")}" }
                            option { value: "block-pre", "{t(\"editor.blockPre\")}" }
                        }

                        div { class: "editor-divider" }

                        // Undo / redo.
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.undo\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("undo"); },
                            span { class: "material-icons", "undo" }
                        }
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.redo\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("redo"); },
                            span { class: "material-icons", "redo" }
                        }

                        div { class: "editor-divider" }

                        // Inline marks.
                        button {
                            class: if st_bold() { "btn-icon active" } else { "btn-icon" },
                            "aria-pressed": if st_bold() { "true" } else { "false" },
                            title: "{t(\"editor.bold\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                richtext::exec("bold");
                                refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block);
                            },
                            span { class: "material-icons", "format_bold" }
                        }
                        button {
                            class: if st_italic() { "btn-icon active" } else { "btn-icon" },
                            "aria-pressed": if st_italic() { "true" } else { "false" },
                            title: "{t(\"editor.italic\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                richtext::exec("italic");
                                refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block);
                            },
                            span { class: "material-icons", "format_italic" }
                        }
                        button {
                            class: if st_underline() { "btn-icon active" } else { "btn-icon" },
                            "aria-pressed": if st_underline() { "true" } else { "false" },
                            title: "{t(\"editor.underline\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                richtext::exec("underline");
                                refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block);
                            },
                            span { class: "material-icons", "format_underlined" }
                        }
                        button {
                            class: if st_strike() { "btn-icon active" } else { "btn-icon" },
                            "aria-pressed": if st_strike() { "true" } else { "false" },
                            title: "{t(\"editor.strikethrough\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                richtext::exec("strikeThrough");
                                refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block);
                            },
                            span { class: "material-icons", "strikethrough_s" }
                        }
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.code\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::wrap_selection_code(); },
                            span { class: "material-icons", "code" }
                        }

                        div { class: "editor-divider" }

                        // Lists.
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.bulletedList\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("insertUnorderedList"); },
                            span { class: "material-icons", "format_list_bulleted" }
                        }
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.numberedList\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("insertOrderedList"); },
                            span { class: "material-icons", "format_list_numbered" }
                        }

                        div { class: "editor-divider" }

                        // Alignment.
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.alignLeft\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("justifyLeft"); },
                            span { class: "material-icons", "format_align_left" }
                        }
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.alignCenter\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("justifyCenter"); },
                            span { class: "material-icons", "format_align_center" }
                        }
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.alignRight\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("justifyRight"); },
                            span { class: "material-icons", "format_align_right" }
                        }
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.alignJustify\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| { richtext::exec("justifyFull"); },
                            span { class: "material-icons", "format_align_justify" }
                        }

                        div { class: "editor-divider" }

                        // Link.
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.link\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                richtext::save_selection();
                                link_url.set(richtext::current_link().unwrap_or_default());
                                let open = link_open();
                                link_open.set(!open);
                            },
                            span { class: "material-icons", "link" }
                        }
                    }

                    // Link URL popover.
                    if link_open() {
                        div { class: "link-popover",
                            input {
                                r#type: "url",
                                class: "link-input",
                                placeholder: "{t(\"editor.linkUrl\")}",
                                value: "{link_url}",
                                oninput: move |evt| link_url.set(evt.value()),
                            }
                            button {
                                class: "btn btn-primary btn-sm",
                                onclick: move |_| {
                                    let url = link_url.read().clone();
                                    richtext::focus_editor(EDITOR_ID);
                                    richtext::restore_selection();
                                    if url.trim().is_empty() {
                                        richtext::exec("unlink");
                                    } else {
                                        // Neutralize javascript:/data: schemes on insert too.
                                        richtext::exec_value(
                                            "createLink",
                                            &super::content::safe_href(&url),
                                        );
                                    }
                                    link_open.set(false);
                                },
                                "{t(\"editor.addLink\")}"
                            }
                            button {
                                class: "btn btn-secondary btn-sm",
                                onclick: move |_| {
                                    richtext::focus_editor(EDITOR_ID);
                                    richtext::restore_selection();
                                    richtext::exec("unlink");
                                    link_open.set(false);
                                },
                                "{t(\"editor.removeLink\")}"
                            }
                        }
                    }
                }

                // The editing surface. Seeded once on mount; the browser owns its
                // DOM thereafter (Dioxus renders no children into it), and it is
                // serialized back to Slate on save.
                div {
                    id: EDITOR_ID,
                    class: "editor-area rich-editor",
                    contenteditable: "true",
                    spellcheck: "true",
                    onmounted: move |_| {
                        richtext::seed_editor(EDITOR_ID, &cmd_seed);
                        richtext::use_semantic_tags();
                        // Sanitize pasted HTML to the editor's semantic subset.
                        richtext::install_paste_handler(EDITOR_ID);
                    },
                    // Ctrl/Cmd + ` toggles code (bold/italic/underline shortcuts
                    // are handled natively by the contenteditable surface).
                    onkeydown: move |evt| {
                        let m = evt.modifiers();
                        if (m.ctrl() || m.meta()) && evt.key().to_string() == "`" {
                            evt.prevent_default();
                            richtext::wrap_selection_code();
                        }
                    },
                    onkeyup: move |_| {
                        refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block)
                    },
                    onmouseup: move |_| {
                        refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block)
                    },
                    oninput: move |_| {
                        refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block);
                        schedule_editor();
                    },
                    onblur: move |_| richtext::save_selection(),
                }
            }
        }
    }
}
