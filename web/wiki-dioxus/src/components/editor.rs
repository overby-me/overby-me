use dioxus::prelude::*;

use crate::components::richtext;
use crate::components::ui::alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogCancel, AlertDialogDescription,
    AlertDialogTitle,
};
use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// Maximum length for a node's display name (#111). Applied as `maxlength` on
/// the name inputs (editor title, add-content form).
pub const NODE_NAME_MAXLEN: usize = 120;

/// DOM id of the `contenteditable` editing surface.
const EDITOR_ID: &str = "rich-editor";

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
    !matches!(
        mime_id,
        Some("wiki/group" | "wiki/event" | "vote/position" | "vote/candidate" | "wiki/folder")
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
                        span { class: "material-icons",
                            if a.node_id.is_some() { "groups" } else { "face" }
                        }
                        span { "{a.name}" }
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
                            add_author(authors, input, suggestions, graphql::Author { name, node_id: None });
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

    // Toolbar active state (reflects the caret).
    let st_bold = use_signal(|| false);
    let st_italic = use_signal(|| false);
    let st_underline = use_signal(|| false);
    let st_strike = use_signal(|| false);
    let st_block = use_signal(|| "paragraph".to_string());

    // Link editor popover.
    let mut link_open = use_signal(|| false);
    let mut link_url = use_signal(String::new);

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
                let data = serde_json::Value::Object(data_obj);

                let set = graphql::NodesSetInput {
                    name: Some(title_val),
                    data: Some(graphql::Jsonb(data)),
                    mutable: Some(mutable),
                    ..Default::default()
                };

                match graphql::update_node(token.as_deref(), &node_id, set).await {
                    Ok(true) => {
                        show_snackbar(&t("common.save"));
                        // Invalidate the cached node so the view we return to
                        // shows the saved content instead of the stale copy.
                        crate::session::bump_data_version();
                        // Empty segments means the parent-less root node (its
                        // content backs the welcome page), which lives at the
                        // Home route rather than a `PathPage`.
                        if segments.is_empty() {
                            nav.push(Route::HomeApp {});
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

    if !is_auth {
        return rsx! {
            div { class: "card",
                div { class: "card-content",
                    p { class: "body-large", "{t(\"node.documentUnavailable\")}" }
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
                        oninput: move |evt| title.set(evt.value()),
                    }
                }

                // Authors (members), content nodes only.
                if takes_authors {
                    AuthorField { authors }
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
                            AlertDialog {
                                open: Some(confirm_submit()),
                                on_open_change: move |v| confirm_submit.set(v),
                                AlertDialogTitle { "{t(\"content.submit\")}" }
                                AlertDialogDescription { "{t(\"content.submitWarning\")}" }
                                AlertDialogActions {
                                    AlertDialogCancel { "{t(\"common.cancel\")}" }
                                    AlertDialogAction {
                                        on_click: {
                                            let save = handle_save.clone();
                                            move |_| {
                                                confirm_submit.set(false);
                                                save(false);
                                            }
                                        },
                                        "{t(\"content.submit\")}"
                                    }
                                }
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
                                        richtext::exec_value("createLink", url.trim());
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
                        refresh_toolbar(st_bold, st_italic, st_underline, st_strike, st_block)
                    },
                    onblur: move |_| richtext::save_selection(),
                }
            }
        }
    }
}
