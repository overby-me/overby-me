use dioxus::prelude::*;

use crate::graphql::{
    self, ChildNodeFields, Jsonb, NodeWithChildren, NodesInsertInput, NodesSetInput, Uuid,
};
use crate::i18n::t;
use crate::session::use_session;

/// Apply the result of a speaker-queue mutation: on success bump `refresh` (which
/// re-reads the queue), on failure surface an error instead of faking success. The
/// live subscription does not re-fire on a failed write, so without this the UI
/// would otherwise sit as if the change had happened.
fn applied<T>(result: Result<T, String>, mut refresh: Signal<u32>, mut busy: Signal<bool>) {
    busy.set(false);
    match result {
        Ok(_) => refresh += 1,
        Err(e) => {
            log::error!("speak mutation failed: {e}");
            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
        }
    }
}

/// A speaker shown optimistically the instant they join, before the insert is
/// confirmed. Reconciled by `key` (the same client-side key the insert uses), so
/// the pending row is dropped once the refetch returns the real entry.
#[derive(Clone, PartialEq)]
struct PendingSpeaker {
    key: String,
    name: String,
}

/// Where a speaker list is shown. On the projector (`Screen`) view the admin and
/// join controls are hidden — it is read-only for the room.
#[derive(Clone, Copy, PartialEq)]
pub enum SpeakMode {
    /// The interactive app view: admin panel + join controls are shown.
    Full,
    /// The projector view: queue only, no controls.
    Screen,
}

/// SpeakApp — speaker queues for a context.
///
/// A context holds one or more hidden `speak/list` children ("talerliste"); each
/// list's queue is its `speak/speak` children. This mirrors the React SpeakApp
/// (`node.get("speakerlist")`), extended to render **every** list in the context
/// (#13), with an owner-only admin panel (#6), priority ordering, and a
/// current/next highlight (#14).
#[component]
pub fn SpeakApp(node: NodeWithChildren, mode: SpeakMode) -> Element {
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let screen = mode == SpeakMode::Screen;
    // The context owner may manage the list(s); Hasura still enforces the rest.
    // Match member.rs: managing capability is node-owner OR context-owner, not just
    // the node's own owner_id (otherwise the chair running the assembly is locked out).
    let is_node_owner = user_id.is_some() && node.owner_id.as_ref().map(|o| o.0.clone()) == user_id;
    let is_owner = is_node_owner || node.is_context_owner.unwrap_or(false);

    let context_id = node.context_id.clone().unwrap_or_else(|| node.id.clone());
    let lists: Vec<ChildNodeFields> = node
        .children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("speak/list"))
        .cloned()
        .collect();

    // The owner may create additional speaker lists (a context can hold several);
    // never on the projector.
    let can_add = is_owner && !screen;

    if lists.is_empty() {
        return rsx! {
            div { class: "card",
                // DESIGN: orb empty state instead of a plain muted line.
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "record_voice_over" }
                    }
                    p { class: "empty-state-body", "{t(\"speak.noLists\")}" }
                    if can_add {
                        div { class: "mt-2",
                            AddSpeakerListButton { context_id: context_id.0.clone() }
                        }
                    }
                }
            }
        };
    }

    rsx! {
        div { class: "stack stack-v",
            if can_add {
                div { class: "stack stack-h", style: "justify-content: flex-end;",
                    AddSpeakerListButton { context_id: context_id.0.clone() }
                }
            }
            for list in lists {
                SpeakList {
                    key: "{list.id.0}",
                    list_id: list.id.0.clone(),
                    list_name: if list.name.is_empty() { node.name.clone() } else { list.name.clone() },
                    context_id: context_id.clone(),
                    is_owner,
                    screen,
                    current_user_id: user_id.clone(),
                }
            }
        }
    }
}

/// Owner-only: create a new speaker list in this context. Opens a name dialog and
/// drives [`graphql::create_speaker_list`] (which grants the context the
/// speak/list permission first if it lacks it). A context may hold several lists.
#[component]
fn AddSpeakerListButton(context_id: String) -> Element {
    let session = use_session();
    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let submit = move |_| {
        let title = name.read().trim().to_string();
        if title.is_empty() || *busy.read() {
            return;
        }
        let key = crate::components::loader::slugify(&title);
        let ctx = context_id.clone();
        let token = session.read().access_token.clone();
        busy.set(true);
        spawn(async move {
            match graphql::create_speaker_list(token.as_deref(), &ctx, &title, &key).await {
                Ok(_) => {
                    crate::session::bump_data_version();
                    busy.set(false);
                    open.set(false);
                    name.set(String::new());
                }
                Err(e) => {
                    busy.set(false);
                    log::error!("create speaker list failed: {e}");
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        button {
            class: "btn btn-tonal",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "add" }
            " {t(\"speak.newList\")}"
        }
        crate::components::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("speak.newList"),
            icon: "record_voice_over".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: name.read().trim().is_empty() || *busy.read(),
                    onclick: submit,
                    "{t(\"common.add\")}"
                }
            },
            div { class: "text-field",
                label { "{t(\"common.title\")}" }
                input {
                    r#type: "text",
                    maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
            }
        }
    }
}

/// One speaker list: its live queue plus (for the owner) an admin panel and a
/// join panel.
#[component]
fn SpeakList(
    list_id: String,
    list_name: String,
    context_id: Uuid,
    is_owner: bool,
    screen: bool,
    current_user_id: Option<String>,
) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let mut refresh = use_signal(|| 0u32);
    let mut time_box = use_signal(|| 120i32);
    // Guards the destructive "clear the whole queue" action behind a confirm dialog.
    let mut clear_confirm = use_signal(|| false);
    // Disables the queue-mutation buttons while a write is in flight, so a fast
    // double-click cannot fire two mutations (a double-move, or a remove racing a
    // move). Reset by `applied`/`move_timer` when the write completes.
    let mut busy = use_signal(|| false);
    // Optimistic joins: shown at once, reconciled by key against the fetched queue.
    let mut pending = use_signal(Vec::<PendingSpeaker>::new);
    // Optimistic removals (remove + next speaker): ids hidden until the delete
    // lands; an id is dropped from the set on error to restore the row.
    let mut removed = use_signal(std::collections::HashSet::<String>::new);
    // Optimistic reorder: speaker_id -> its target index, applied to the sort so a
    // move-up/down jumps at once. After success the fetched index matches (redundant,
    // harmless); on error the entry is dropped to snap back.
    let mut reorder = use_signal(std::collections::HashMap::<String, i32>::new);

    // Live updates: subscribe to this list's entries over the Hasura WebSocket
    // so entries added/removed by anyone appear at once.
    let sub_list = crate::graphql::gql_escape(&list_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{sub_list}\" }}, mimeId: {{ _eq: \"speak/speak\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let access_token = session.read().access_token.clone();
    let list_for_query = list_id.clone();
    let rev = *refresh.read();
    let state = crate::use_data_resource!(|(list_for_query, access_token, rev)| async move {
        let _ = rev;
        let n = graphql::query_node_by_id(access_token.as_deref(), &list_for_query)
            .await
            .ok()??;
        {
            let speakers = sorted_speakers(&n.children);
            // The speaking time limit + last-start live on the list node's data.
            let time = n
                .data
                .as_ref()
                .and_then(|d| d.0.get("time"))
                .and_then(|t| t.as_f64())
                .unwrap_or(0.0);
            let updated_at = n
                .data
                .as_ref()
                .and_then(|d| d.0.get("updatedAt"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some((n.mutable, time, updated_at, speakers))
        }
    });

    // Tick once a second so the countdown updates (cancelled on unmount).
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1000).await;
            tick += 1;
        }
    });

    let st = state.read().clone().flatten();
    let mutable = st.as_ref().map(|s| s.0).unwrap_or(false);
    // Optimistically hide speakers the user just removed or advanced past. The
    // delete's refetch confirms the absence; on error the id is dropped from the set
    // to restore the row. Filter before deriving count/current so the highlight and
    // timer track the removal at once.
    let mut speakers: Vec<ChildNodeFields> = st
        .as_ref()
        .map(|s| s.3.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !removed.read().contains(&s.id.0))
        .collect();
    // Apply optimistic reorder overrides by re-sorting with the effective index.
    let reord = reorder.read().clone();
    if !reord.is_empty() {
        speakers.sort_by(|a, b| {
            let ai = reord.get(&a.id.0).copied().unwrap_or(a.index);
            let bi = reord.get(&b.id.0).copied().unwrap_or(b.index);
            ai.cmp(&bi)
                .then_with(|| speaker_type(b).cmp(&speaker_type(a)))
                .then_with(|| {
                    let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                    let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                    a_ts.cmp(b_ts)
                })
        });
    }
    let remaining = st.as_ref().map_or(0, |(_, time, updated_at, _)| {
        let _ = tick.read();
        remaining_seconds(*time, updated_at)
    });
    let running = st.as_ref().map(|s| s.1 > 0.0).unwrap_or(false);
    let min_index = speakers.iter().map(|s| s.index).min().unwrap_or(0);
    let max_index = speakers.iter().map(|s| s.index).max().unwrap_or(0);
    let count = speakers.len();
    // Drop optimistic joins once the real entry (same key) has come back.
    let fetched_keys: std::collections::HashSet<String> =
        speakers.iter().map(|s| s.key.clone()).collect();
    let pending_shown = crate::components::optimistic::reconcile_by_key(
        &pending.read(),
        |p| p.key.as_str(),
        &fetched_keys,
    );
    // Announced (screen-reader-only) whenever the person at the front of the
    // queue changes, so blind participants hear turn-changes like the room does.
    let now_speaking = speakers.first().map(|s| s.name.clone()).unwrap_or_default();

    // Native notification when it becomes the user's turn to speak (#139). Keyed
    // on the current speaker so it fires once per turn-change; permission is
    // requested on Join, so only opted-in participants are ever notified.
    {
        let current_id = speakers.first().map(|s| s.id.0.clone()).unwrap_or_default();
        let my_turn = current_user_id.is_some()
            && speakers
                .first()
                .and_then(|s| s.owner_id.as_ref())
                .map(|o| o.0.clone())
                == current_user_id;
        use_effect(use_reactive!(|(current_id, my_turn)| {
            let _ = &current_id;
            if my_turn {
                crate::pwa::notify(&t("speak.yourTurn"), &t("speak.yourTurnBody"));
            }
        }));
    }

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div {
                    class: if mutable { "avatar secondary" } else { "avatar" },
                    title: if mutable { "{t(\"speak.open\")}" } else { "{t(\"speak.close\")}" },
                    if mutable { span { class: "material-icons", "lock_open" } } else { span { class: "material-icons", "lock" } }
                }
                div {
                    h3 { class: "title-medium", "{list_name}" }
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"speak.speakerList\")}"
                    }
                }
                if running {
                    div { class: "flex-grow" }
                    // Keep the timer visible at 00:00 with an unmistakable expired
                    // state, rather than silently vanishing when time is up.
                    div {
                        class: if remaining == 0 { "chip-timer chip-timer-expired" } else { "chip-timer" },
                        title: "{t(\"speak.remaining\")}",
                        span { class: "material-icons", style: "font-size: 18px; vertical-align: middle;", "timer" }
                        " {time_string(remaining)}"
                    }
                }
            }

            // Live region for turn-changes (announces only when the text differs).
            div {
                class: "sr-only",
                role: "status",
                aria_live: "polite",
                if !now_speaking.is_empty() {
                    "{t(\"speak.speakingNow\")}: {now_speaking}"
                }
            }

            if speakers.is_empty() && (screen || pending_shown.is_empty()) {
                // DESIGN: orb empty state for an empty speaker queue.
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "record_voice_over" }
                    }
                    p { class: "empty-state-body", "{t(\"speak.emptyList\")}" }
                }
            } else {
                div { class: if screen { "list speak-projector" } else { "list" },
                    // On the projector (screen) the room only needs the current
                    // speaker and the single next one, who is asked to get ready;
                    // the full queue stays in the interactive SpeakApp/admin view.
                    for (i , speaker) in speakers.iter().enumerate().take(if screen { 2 } else { usize::MAX }) {
                        {
                            let speaker_id = speaker.id.0.clone();
                            let name = speaker.name.clone();
                            // Identity for the popover from the queue entry's owner
                            // (gated on a non-empty name so the card's name and the
                            // linked profile always refer to the same person).
                            let owner = speaker.owner.as_ref().filter(|o| !o.display_name.is_empty());
                            let owner_id = owner.map(|o| o.id.0.clone());
                            let owner_avatar = owner.map(|o| o.avatar_url.clone()).unwrap_or_default();
                            let popover_name =
                                owner.map(|o| o.display_name.clone()).unwrap_or_else(|| name.clone());
                            let (type_icon , type_key) = speak_type_meta(speaker_type(speaker));
                            let secondary = match i {
                                0 => t("speak.speakingNow"),
                                1 if screen => t("speak.getReady"),
                                1 => t("speak.next"),
                                _ => t(type_key),
                            };
                            let row_class = match i {
                                0 => "list-item speak-current",
                                1 if screen => "list-item speak-next speak-getready",
                                1 => "list-item speak-next",
                                _ => "list-item",
                            };
                            let can_remove = is_auth
                                && !screen
                                && (is_owner
                                    || speaker.owner_id.as_ref().map(|o| o.0.clone())
                                        == current_user_id);
                            rsx! {
                                div { class: "{row_class}", key: "{speaker_id}",
                                    div { class: "avatar small secondary", "{i + 1}" }
                                    div { class: "list-item-text",
                                        super::loader::UserPopover {
                                            name: popover_name.clone(),
                                            avatar_url: owner_avatar.clone(),
                                            user_id: owner_id.clone(),
                                            div { class: "list-item-primary", "{name}" }
                                        }
                                        div { class: "list-item-secondary",
                                            span {
                                                class: "material-icons",
                                                style: "font-size: 14px; vertical-align: middle; margin-right: 2px;",
                                                "{type_icon}"
                                            }
                                            "{secondary}"
                                        }
                                    }
                                    // Owner reorder (#7): pull a speaker to the
                                    // front or back of the queue.
                                    if is_owner && !screen {
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"speak.moveUp\")}",
                                            disabled: i == 0 || busy(),
                                            onclick: {
                                                let token = session.read().access_token.clone();
                                                let id = speaker_id.clone();
                                                move |_| {
                                                    let token = token.clone();
                                                    let id = id.clone();
                                                    busy.set(true);
                                                    // Optimistic: jump to the front now.
                                                    reorder.write().insert(id.clone(), min_index - 1);
                                                    spawn(async move {
                                                        let set = NodesSetInput {
                                                            index: Some(min_index - 1),
                                                            ..Default::default()
                                                        };
                                                        match graphql::update_node(token.as_deref(), &id, set).await {
                                                            Ok(_) => {
                                                                busy.set(false);
                                                                refresh += 1;
                                                            }
                                                            Err(e) => {
                                                                reorder.write().remove(&id);
                                                                busy.set(false);
                                                                log::error!("reorder failed: {e}");
                                                                crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                            }
                                                        }
                                                    });
                                                }
                                            },
                                            span { class: "material-icons", "vertical_align_top" }
                                        }
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"speak.moveDown\")}",
                                            disabled: i + 1 == count || busy(),
                                            onclick: {
                                                let token = session.read().access_token.clone();
                                                let id = speaker_id.clone();
                                                move |_| {
                                                    let token = token.clone();
                                                    let id = id.clone();
                                                    busy.set(true);
                                                    // Optimistic: drop to the back now.
                                                    reorder.write().insert(id.clone(), max_index + 1);
                                                    spawn(async move {
                                                        let set = NodesSetInput {
                                                            index: Some(max_index + 1),
                                                            ..Default::default()
                                                        };
                                                        match graphql::update_node(token.as_deref(), &id, set).await {
                                                            Ok(_) => {
                                                                busy.set(false);
                                                                refresh += 1;
                                                            }
                                                            Err(e) => {
                                                                reorder.write().remove(&id);
                                                                busy.set(false);
                                                                log::error!("reorder failed: {e}");
                                                                crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                            }
                                                        }
                                                    });
                                                }
                                            },
                                            span { class: "material-icons", "vertical_align_bottom" }
                                        }
                                    }
                                    if can_remove {
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"speak.removeFromList\")}",
                                            onclick: {
                                                let token = session.read().access_token.clone();
                                                let id = speaker_id.clone();
                                                move |_| {
                                                    let token = token.clone();
                                                    let id = id.clone();
                                                    // Optimistic: hide the row now; restore on error.
                                                    removed.write().insert(id.clone());
                                                    spawn(async move {
                                                        match graphql::delete_node(token.as_deref(), &id).await {
                                                            Ok(_) => refresh += 1,
                                                            Err(e) => {
                                                                removed.write().remove(&id);
                                                                log::error!("remove speaker failed: {e}");
                                                                crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                            }
                                                        }
                                                    });
                                                }
                                            },
                                            span { class: "material-icons", "close" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Optimistic joins (interactive view only): muted "sending" rows
                    // at the tail, dropped by reconcile_by_key once confirmed.
                    if !screen {
                        for p in pending_shown.iter() {
                            div { key: "{p.key}", class: "list-item is-pending",
                                div { class: "list-item-text",
                                    div { class: "list-item-primary", "{p.name}" }
                                    div { class: "list-item-secondary", "{t(\"vote.sending\")}" }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Owner admin panel (#6): open/close the list, clear it, run the timer.
        if is_owner && !screen {
            div { class: "card",
                div { class: "card-header",
                    h3 { class: "title-medium", "{t(\"speak.manageSpeakerList\")}" }
                }
                div { class: "card-content",
                    div { class: "stack stack-h", style: "gap: 8px; flex-wrap: wrap; align-items: center;",
                        // Serve the queue: remove the current speaker and re-anchor
                        // the countdown for the new one, so the projected timer always
                        // reflects the person actually speaking.
                        if count > 0 {
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let token = session.read().access_token.clone();
                                    let list_id = list_id.clone();
                                    let current = speakers.first().map(|s| s.id.0.clone());
                                    let limit = *time_box.read();
                                    move |_| {
                                        let del_token = token.clone();
                                        let current = current.clone();
                                        // Optimistic: advance the queue at once by hiding
                                        // the current speaker; restore on error.
                                        if let Some(cur) = current.clone() {
                                            removed.write().insert(cur);
                                        }
                                        spawn(async move {
                                            if let Some(cur) = current {
                                                match graphql::delete_node(del_token.as_deref(), &cur).await {
                                                    Ok(_) => refresh += 1,
                                                    Err(e) => {
                                                        removed.write().remove(&cur);
                                                        log::error!("next speaker failed: {e}");
                                                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                    }
                                                }
                                            } else {
                                                refresh += 1;
                                            }
                                        });
                                        // Re-anchor the per-turn timer for the next speaker.
                                        if limit > 0 {
                                            move_timer(token.clone(), list_id.clone(), limit, refresh, busy);
                                        }
                                    }
                                },
                                span { class: "material-icons", "skip_next" }
                                " {t(\"speak.nextSpeaker\")}"
                            }
                        }
                        button {
                            class: "btn btn-secondary",
                            onclick: {
                                let token = session.read().access_token.clone();
                                let id = list_id.clone();
                                move |_| {
                                    let token = token.clone();
                                    let id = id.clone();
                                    spawn(async move {
                                        let set = NodesSetInput {
                                            mutable: Some(!mutable),
                                            ..Default::default()
                                        };
                                        applied(graphql::update_node(token.as_deref(), &id, set).await, refresh, busy);
                                    });
                                }
                            },
                            if mutable {
                                span { class: "material-icons", "lock" }
                                " {t(\"speak.close\")}"
                            } else {
                                span { class: "material-icons", "lock_open" }
                                " {t(\"speak.open\")}"
                            }
                        }
                        button {
                            class: "btn btn-outlined",
                            disabled: count == 0,
                            // Clearing the queue is destructive; confirm first.
                            onclick: move |_| clear_confirm.set(true),
                            span { class: "material-icons", "delete" }
                            " {t(\"speak.clear\")}"
                        }
                        crate::components::widgets::Dialog {
                            open: clear_confirm(),
                            on_dismiss: move |_| clear_confirm.set(false),
                            headline: t("speak.clear"),
                            icon: "delete".to_string(),
                            actions: rsx! {
                                button {
                                    class: "btn btn-outlined",
                                    onclick: move |_| clear_confirm.set(false),
                                    "{t(\"common.cancel\")}"
                                }
                                button {
                                    class: "btn btn-primary",
                                    onclick: {
                                        let token = session.read().access_token.clone();
                                        let ids: Vec<String> = speakers.iter().map(|s| s.id.0.clone()).collect();
                                        move |_| {
                                            let token = token.clone();
                                            let ids = ids.clone();
                                            clear_confirm.set(false);
                                            spawn(async move {
                                                let mut ok = true;
                                                for id in ids {
                                                    if let Err(e) = graphql::delete_node(token.as_deref(), &id).await {
                                                        log::error!("clear queue failed: {e}");
                                                        ok = false;
                                                        break;
                                                    }
                                                }
                                                if !ok {
                                                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                }
                                                refresh += 1;
                                            });
                                        }
                                    },
                                    "{t(\"speak.clear\")}"
                                }
                            },
                            p { class: "body-medium", "{t(\"speak.confirmClear\")}" }
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: {
                                let token = session.read().access_token.clone();
                                let id = list_id.clone();
                                move |_| {
                                    let token = token.clone();
                                    let id = id.clone();
                                    // Start sets the limit + a fresh timestamp; stop zeroes it.
                                    let secs = if running { 0 } else { *time_box.read() };
                                    move_timer(token, id, secs, refresh, busy);
                                }
                            },
                            if running {
                                span { class: "material-icons", "stop" }
                                " {t(\"speak.stop\")}"
                            } else {
                                span { class: "material-icons", "play_arrow" }
                                " {t(\"speak.start\")}"
                            }
                        }
                        div { class: "text-field", style: "margin: 0; width: 120px;",
                            label { "{t(\"speak.speakingTime\")}" }
                            input {
                                r#type: "number",
                                min: "0",
                                value: "{time_box}",
                                oninput: move |e| {
                                    if let Ok(v) = e.value().parse::<i32>() {
                                        time_box.set(v);
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }

        // Join panel — insert a speak/speak entry under this list.
        if is_auth && !screen && mutable {
            div { class: "card",
                div { class: "card-header",
                    h3 { class: "title-medium", "{t(\"speak.joinSpeakerList\")}" }
                }
                div { class: "card-content",
                    div { class: "stack stack-v",
                        {
                            // Five join types, indexed 0..4 to match React and the
                            // existing `data` rows ("4" = procedure); higher jumps
                            // the queue (order_by data desc).
                            let speak_types = [
                                ("0", t("speak.talk")),
                                ("1", t("speak.question")),
                                ("2", t("speak.clarify")),
                                ("3", t("speak.misunderstood")),
                                ("4", t("speak.procedure")),
                            ];
                            rsx! {
                                for (type_key , label) in speak_types {
                                    {
                                        let list = list_id.clone();
                                        let context_id = context_id.clone();
                                        let display_name = session
                                            .read()
                                            .user
                                            .as_ref()
                                            .map(|u| u.display_name.clone())
                                            .unwrap_or_default();
                                        let token = session.read().access_token.clone();
                                        rsx! {
                                            button {
                                                class: "btn btn-outlined",
                                                onclick: move |_| {
                                                    // Joining is a user gesture: ask for
                                                    // notification permission so we can ping
                                                    // this speaker when it is their turn (#139).
                                                    crate::pwa::request_notification_permission();
                                                    let name = display_name.clone();
                                                    let key = format!(
                                                        "{}-{}",
                                                        name.to_lowercase().replace(' ', "-"),
                                                        now_ms(),
                                                    );
                                                    let parent = list.clone();
                                                    let ctx = context_id.clone();
                                                    let token = token.clone();
                                                    let type_val = type_key.to_string();
                                                    // Optimistic: show the join at once (a
                                                    // muted "sending" row), reconciled by key
                                                    // once the refetch returns the real entry.
                                                    pending.write().push(PendingSpeaker {
                                                        key: key.clone(),
                                                        name: name.clone(),
                                                    });
                                                    spawn(async move {
                                                        match graphql::insert_node(
                                                            token.as_deref(),
                                                            NodesInsertInput {
                                                                name: Some(name),
                                                                key: Some(key.clone()),
                                                                mime_id: Some("speak/speak".to_string()),
                                                                parent_id: Some(Uuid(parent)),
                                                                context_id: Some(ctx),
                                                                data: Some(
                                                                    Jsonb(serde_json::Value::String(type_val)),
                                                                ),
                                                                mutable: None,
                                                                index: None,
                                                            },
                                                        )
                                                        .await
                                                        {
                                                            Ok(_) => refresh += 1,
                                                            Err(e) => {
                                                                // Roll back the optimistic row.
                                                                pending.write().retain(|p| p.key != key);
                                                                log::error!("join speaker failed: {e}");
                                                                crate::snackbar::show_snackbar(&t(
                                                                    "error.somethingWentWrong",
                                                                ));
                                                            }
                                                        }
                                                    });
                                                },
                                                "{label}"
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
    }
}

/// Start/stop the speaking timer by writing `{ time, updatedAt }` to the list's
/// data (updatedAt is the moment the clock started, used by the countdown).
fn move_timer(
    token: Option<String>,
    list_id: String,
    secs: i32,
    refresh: Signal<u32>,
    busy: Signal<bool>,
) {
    spawn(async move {
        let data = serde_json::json!({ "time": secs, "updatedAt": now_iso() });
        let set = NodesSetInput {
            data: Some(Jsonb(data)),
            ..Default::default()
        };
        applied(
            graphql::update_node(token.as_deref(), &list_id, set).await,
            refresh,
            busy,
        );
    });
}

/// The speak type stored on a `speak/speak` node's data: `"0"`..`"4"` (talk,
/// question, clarify, misunderstood, procedure). Higher numbers are procedural
/// and jump the queue, matching React `order_by: [{ data: desc }, { createdAt: asc }]`.
fn speaker_type(node: &ChildNodeFields) -> i64 {
    node.data
        .as_ref()
        .and_then(|d| match &d.0 {
            serde_json::Value::String(s) => s.parse::<i64>().ok(),
            serde_json::Value::Number(n) => n.as_i64(),
            _ => None,
        })
        .unwrap_or(0)
}

/// Icon + i18n key for a speak type.
fn speak_type_meta(kind: i64) -> (&'static str, &'static str) {
    match kind {
        4 => ("gavel", "speak.procedure"),
        3 => ("campaign", "speak.misunderstood"),
        2 => ("lightbulb", "speak.clarify"),
        1 => ("question_mark", "speak.question"),
        _ => ("record_voice_over", "speak.talk"),
    }
}

/// Order a list's `speak/speak` children the way the queue is served: an owner
/// override (`index`) first, then procedural priority (type desc), then arrival
/// time (createdAt asc). Default index 0 keeps the priority/arrival order.
fn sorted_speakers(children: &[ChildNodeFields]) -> Vec<ChildNodeFields> {
    let mut out: Vec<ChildNodeFields> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("speak/speak"))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.index
            .cmp(&b.index)
            .then_with(|| speaker_type(b).cmp(&speaker_type(a)))
            .then_with(|| {
                let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                a_ts.cmp(b_ts)
            })
    });
    out
}

/// Format seconds as `MM:SS` (matches the React `timeString`).
fn time_string(total: i64) -> String {
    let total = total.max(0);
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// A millisecond timestamp for generating unique node keys.
fn now_ms() -> String {
    let window = web_sys::window().unwrap();
    let performance = window.performance().unwrap();
    format!("{:.0}", performance.now())
}

/// The current wall-clock time as an ISO 8601 string (for the timer start).
fn now_iso() -> String {
    String::from(js_sys::Date::new_0().to_iso_string())
}

/// Seconds left on the current speaker's turn: the `time` limit minus the wall
/// clock elapsed since the list was last updated. Uses JS `Date` for the clock.
fn remaining_seconds(time: f64, updated_at: &str) -> i64 {
    remaining_seconds_at(time, js_sys::Date::parse(updated_at), js_sys::Date::now())
}

/// Pure countdown maths (testable off-browser).
fn remaining_seconds_at(time: f64, updated_ms: f64, now_ms: f64) -> i64 {
    if time <= 0.0 || updated_ms.is_nan() {
        return 0;
    }
    let elapsed = (now_ms - updated_ms) / 1000.0;
    let rem = time - elapsed;
    if rem > 0.0 {
        rem.floor() as i64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::{Jsonb, Timestamptz, Uuid};

    fn speaker(id: &str, kind: &str, index: i32, created: &str) -> ChildNodeFields {
        ChildNodeFields {
            id: Uuid(id.to_string()),
            name: id.to_string(),
            key: id.to_string(),
            mime_id: Some("speak/speak".to_string()),
            mutable: false,
            index,
            created_at: Some(Timestamptz(created.to_string())),
            owner_id: None,
            is_owner: None,
            is_context_owner: None,
            owner: None,
            parent: None,
            data: Some(Jsonb(serde_json::Value::String(kind.to_string()))),
            mime: None,
        }
    }

    #[test]
    fn countdown_subtracts_elapsed_and_clamps() {
        // 60s limit, updated 10s ago -> 50 left.
        assert_eq!(remaining_seconds_at(60.0, 100_000.0, 110_000.0), 50);
        // Past the limit -> 0.
        assert_eq!(remaining_seconds_at(5.0, 100_000.0, 110_000.0), 0);
        // No limit set -> 0.
        assert_eq!(remaining_seconds_at(0.0, 100_000.0, 100_000.0), 0);
    }

    #[test]
    fn time_string_is_mm_ss() {
        assert_eq!(time_string(0), "00:00");
        assert_eq!(time_string(9), "00:09");
        assert_eq!(time_string(75), "01:15");
        assert_eq!(time_string(-5), "00:00");
    }

    #[test]
    fn priority_then_arrival_then_owner_override() {
        // A later procedure (type 4) jumps ahead of an earlier talk (type 0);
        // same-type entries keep arrival order.
        let list = vec![
            speaker("talk-early", "0", 0, "2024-01-01T10:00:00Z"),
            speaker("talk-late", "0", 0, "2024-01-01T10:05:00Z"),
            speaker("procedure", "4", 0, "2024-01-01T10:10:00Z"),
        ];
        let order: Vec<String> = sorted_speakers(&list)
            .iter()
            .map(|s| s.id.0.clone())
            .collect();
        assert_eq!(order, ["procedure", "talk-early", "talk-late"]);

        // A negative index override pulls a talk to the very front.
        let mut list2 = list.clone();
        list2[1].index = -1; // talk-late pinned to top
        let order2: Vec<String> = sorted_speakers(&list2)
            .iter()
            .map(|s| s.id.0.clone())
            .collect();
        assert_eq!(order2, ["talk-late", "procedure", "talk-early"]);
    }
}
