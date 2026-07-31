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

/// DOM id of the hidden file input the insert-image toolbar button triggers.
const INLINE_IMAGE_INPUT_ID: &str = "inline-image-input";

/// Cap on an inline image inserted as a data URI. Inline images are self-
/// contained (no upload, so no token in the persisted content), but that puts the
/// bytes in the document's JSON, so a large photo belongs in the cover-image slot
/// / storage instead. ~2 MiB keeps a document reasonable.
const MAX_INLINE_IMAGE_BYTES: usize = 2 * 1024 * 1024;

/// Programmatically click a DOM element by id (used to open the hidden inline-
/// image file picker from the toolbar button).
fn click_element_by_id(id: &str) {
    use wasm_bindgen::JsCast;
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        if let Ok(h) = el.dyn_into::<web_sys::HtmlElement>() {
            h.click();
        }
    }
}

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
/// Whether the editor offers a `data.image` picker for this node. Content nodes
/// carry a cover image; a candidate carries its photo in the very same field, and
/// adding one now lands here, so the photo is set where its text is written.
fn node_takes_cover_image(mime_id: Option<&str>) -> bool {
    node_takes_authors(mime_id) || matches!(mime_id, Some("vote/candidate"))
}

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
    mut authors: Signal<Vec<model::Author>>,
    mut input: Signal<String>,
    mut suggestions: Signal<Vec<model::Author>>,
    author: model::Author,
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
fn AuthorField(authors: Signal<Vec<model::Author>>) -> Element {
    let session = use_session();
    let mut authors = authors;
    let mut input = use_signal(String::new);
    let mut suggestions = use_signal(Vec::<model::Author>::new);
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
                            add_author(authors, input, suggestions, model::Author { name, node_id: None, avatar_url: String::new(), user_id: None });
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
    let takes_cover_image = node_takes_cover_image(node.mime_id.as_deref());
    // A candidate's `data.image` is its photo, not a cover, so it says so.
    let is_candidate = node.mime_id.as_deref() == Some("vote/candidate");
    let image_label = if is_candidate {
        t("vote.candidatePhoto")
    } else {
        t("content.coverImage")
    };
    let image_cta = if is_candidate {
        t("vote.uploadPhoto")
    } else {
        t("content.uploadImage")
    };
    let mut authors = use_signal(|| {
        node.members
            .iter()
            .filter(|m| !m.hidden)
            .map(|m| model::Author {
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
    // The existing cover thumbnail, resolved once on mount.
    let existing_image_url =
        super::loader::use_file_object_url(initial_image.clone().unwrap_or_default());
    // Preview of a freshly picked image, built from the bytes already in hand as a
    // local blob rather than fetched back from storage.
    //
    // It paints immediately, with no round trip, and it does not depend on the
    // uploaded file being readable before it is attached to a node — which it is
    // not: storage.files is now readable only through the node that references it,
    // and until this editor saves, nothing references this upload.
    let mut picked_preview = use_signal(|| None::<String>);
    use_drop(move || {
        if let Some(url) = picked_preview.peek().clone() {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
    });

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

    // Reset every seeded field + re-seed the editor surface when the node changes
    // WITHOUT a remount (a sibling editor→editor navigation reuses this component
    // instance; the web renderer doesn't reliably remount on a key change). The
    // `seeded_for` guard means this fires ONLY on an actual node-id change, never
    // mid-edit — so it cannot clobber in-progress work — and every reset value is
    // recomputed fresh from the current `node` prop rather than a frozen capture.
    // Without this, editing node B would show/keep node A's title/authors/body and
    // a save would write A's stale content onto B.
    let mut seeded_for = use_signal(|| node_id.clone());
    {
        let node_id_dep = node_id.clone();
        let reset_name = node.name.clone();
        let reset_authors: Vec<model::Author> = node
            .members
            .iter()
            .filter(|m| !m.hidden)
            .map(|m| model::Author {
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
            .collect();
        let reset_image = initial_image.clone();
        let reset_date = node
            .created_at
            .as_ref()
            .map(|t| t.0.chars().take(10).collect::<String>())
            .unwrap_or_default();
        let reset_html = node
            .data
            .as_ref()
            .and_then(|d| d.0.get("content"))
            .map(richtext::slate_to_html)
            .unwrap_or_else(|| "<p><br></p>".to_string());
        use_effect(use_reactive!(|(
            node_id_dep,
            reset_name,
            reset_authors,
            reset_image,
            reset_date,
            reset_html,
        )| {
            if *seeded_for.peek() == node_id_dep {
                return; // initial mount — onmounted already seeded this node
            }
            seeded_for.set(node_id_dep);
            title.set(reset_name);
            authors.set(reset_authors);
            image_id.set(reset_image);
            image_name.set(String::new());
            created_date.set(reset_date);
            richtext::seed_editor(EDITOR_ID, &reset_html);
            dirty.set(false);
            set_editor_dirty(false);
        }));
    }

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
    // A third handle, for the key handler: shift-enter changes the surface
    // without an `input` event, so it has to start the debounce itself.
    let mut schedule_break = schedule_autosave.clone();
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
                    // Show it before the upload even starts: the bytes are here.
                    // Bind before writing: a peek() guard held across the write
                    // would still be alive inside an `if let` body and panic.
                    let previous = picked_preview.peek().clone();
                    if let Some(old) = previous {
                        let _ = web_sys::Url::revoke_object_url(&old);
                        picked_preview.set(None);
                    }
                    let arr = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
                    arr.copy_from(&bytes);
                    let parts = js_sys::Array::of1(&arr);
                    if let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(parts.as_ref()) {
                        if let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) {
                            picked_preview.set(Some(url));
                        }
                    }
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

    // Insert a chosen image inline, as a self-contained `data:` URI: no upload, so
    // no session token ends up in the persisted content (unlike a protected file
    // URL), and it renders as-is in both the editor and the SlateRenderer. Capped
    // so the document JSON does not balloon (see MAX_INLINE_IMAGE_BYTES). The
    // selection was saved when the toolbar button was pressed; restore it so the
    // image lands where the caret was, not wherever focus returned from the dialog.
    let on_pick_inline_image = move |evt: FormEvent| {
        let files = evt.files();
        let Some(fd) = files.into_iter().next() else {
            return;
        };
        let ctype = fd
            .content_type()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "image/png".to_string());
        spawn(async move {
            match fd.read_bytes().await {
                Ok(bytes) => {
                    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
                        show_snackbar(&t("editor.imageTooLarge"));
                        return;
                    }
                    let Some(window) = web_sys::window() else {
                        return;
                    };
                    // btoa needs a binary string (each char < 256); map bytes 1:1.
                    let binary: String = bytes.iter().map(|&b| b as char).collect();
                    let Ok(b64) = window.btoa(&binary) else {
                        show_snackbar(&t("error.somethingWentWrong"));
                        return;
                    };
                    let data_uri = format!("data:{ctype};base64,{b64}");
                    richtext::focus_editor(EDITOR_ID);
                    richtext::restore_selection();
                    richtext::exec_value("insertImage", &data_uri);
                    dirty.set(true);
                    set_editor_dirty(true);
                }
                Err(_) => show_snackbar(&t("error.somethingWentWrong")),
            }
        });
    };

    if !is_auth {
        // DESIGN: an expressive locked-barrier state instead of a plain card.
        return rsx! {
            div { class: "card",
                div { class: "empty-state empty-state-sm",
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

                // The `data.image` picker: a cover for content, the photo for a
                // candidate (whose add dialog no longer asks for one, since adding
                // a candidate lands here).
                if takes_cover_image {
                    div { class: "mt-2 mb-2",
                        div { class: "file-upload-label", "{image_label}" }
                        label { class: "file-upload",
                            input {
                                r#type: "file",
                                accept: "image/*",
                                class: "file-upload-input",
                                onchange: on_pick_image,
                            }
                            span { class: "material-icons", "image" }
                            span { class: "file-upload-text", "{image_cta}" }
                        }
                        if *image_uploading.read() {
                            div {
                                class: "stack stack-h mt-1",
                                div { class: "spinner spinner-sm" }
                                span { class: "body-small text-muted", "{image_cta}\u{2026}" }
                            }
                        } else if !image_name.read().is_empty() {
                            // A fresh upload: the picture itself, click-to-zoom, with
                            // its file name written over the base of it.
                            div {
                                if let Some(src) = picked_preview.read().clone() {
                                    div { class: "upload-preview",
                                        super::widgets::ZoomableImage {
                                            src,
                                            alt: image_name.read().clone(),
                                        }
                                        div { class: "upload-preview-name", "{image_name}" }
                                    }
                                } else {
                                    // No id to build a URL from: name it, as before.
                                    div { class: "file-upload-done",
                                        span { class: "material-icons", "check_circle" }
                                        span { class: "file-upload-name", "{image_name}" }
                                    }
                                }
                                button {
                                    class: "btn btn-text",
                                    onclick: move |_| {
                                        image_id.set(None);
                                        image_name.set(String::new());
                                        // Let go of the blob too, or it is held
                                        // until the editor unmounts.
                                        let previous = picked_preview.peek().clone();
                                        if let Some(url) = previous {
                                            let _ = web_sys::Url::revoke_object_url(&url);
                                            picked_preview.set(None);
                                        }
                                        dirty.set(true);
                                        set_editor_dirty(true);
                                    },
                                    "{t(\"content.removeImage\")}"
                                }
                            }
                        } else if image_id.read().is_some() {
                            // The stored image, same preview. There is no file name to
                            // write on it: only the id was ever persisted.
                            div {
                                if let Some(url) = existing_image_url.clone() {
                                    div { class: "upload-preview",
                                        super::widgets::ZoomableImage {
                                            src: url,
                                            alt: image_label.clone(),
                                        }
                                    }
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
                }

                // The node's date, for context owners on content nodes only.
                if takes_authors && is_owner {
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

                // Sticky toolbar (#94): action buttons + formatting controls
                // stay pinned while scrolling a long document.
                // Editing something already submitted is a context owner's
                // privilege, and it should never be a surprise: autosave stays
                // off for a submitted node (see the guard above), so say both
                // things rather than let a chair discover the second by losing
                // a paragraph.
                if !is_mutable {
                    div { class: "status-banner is-notice",
                        span { class: "material-icons", "lock" }
                        span { "{t(\"content.editingSubmitted\")}" }
                    }
                }
                // A docked M3 toolbar (see .m3-toolbar): the standard colour,
                // since this is a working surface rather than an emphasis one.
                div { class: "m3-toolbar m3-toolbar-standard editor-toolbar",
                    // Save and Submit, both visible.
                    //
                    // This was briefly an M3 split button, with submit in its
                    // menu — the component fits the pair, since saving happens
                    // constantly and submitting once. It went back before it
                    // ever shipped: the same week, members told us they could
                    // not find where to write in a new resolution at all. On a
                    // deadline, with people who use this twice a year, the last
                    // thing to do is move the button that ends the task one tap
                    // deeper. The widget stays in the library for a calmer
                    // surface (widgets::SplitButton).
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
                            div { class: "spinner spinner-sm" }
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

                        div { class: "editor-divider" }

                        // Insert an inline image (stored as a self-contained data
                        // URI). Opens the hidden file picker; the selection is saved
                        // first so the image lands at the caret after the dialog.
                        button {
                            class: "btn-icon",
                            title: "{t(\"editor.insertImage\")}",
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                richtext::save_selection();
                                click_element_by_id(INLINE_IMAGE_INPUT_ID);
                            },
                            span { class: "material-icons", "image" }
                        }
                        input {
                            id: INLINE_IMAGE_INPUT_ID,
                            class: "file-upload-input",
                            r#type: "file",
                            accept: "image/*",
                            onchange: on_pick_inline_image,
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
                    //
                    // Shift+Enter is a line break WITHIN the block, not a new
                    // one: the address in a resolution, the lines of a motion's
                    // preamble. Browsers do this natively in a contenteditable,
                    // but not all of them and not the same way, and the editor
                    // has to agree with what the serializer stores — so it is
                    // driven explicitly rather than left to the surface.
                    onkeydown: move |evt: Event<KeyboardData>| {
                        let m = evt.modifiers();
                        let key = evt.key().to_string();
                        if (m.ctrl() || m.meta()) && key == "`" {
                            evt.prevent_default();
                            richtext::wrap_selection_code();
                        } else if key == "Enter" && m.shift() {
                            evt.prevent_default();
                            richtext::insert_line_break();
                            // The surface changed without an `input` event, so the
                            // autosave has to be told, or a break typed just before
                            // a tab closes is the one edit that does not survive.
                            schedule_break();
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
