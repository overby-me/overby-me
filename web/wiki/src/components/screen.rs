use dioxus::prelude::*;

use crate::graphql::{self};
use crate::model::NodeWithChildren;
use crate::session::use_session;

use super::loader::MimeLoader;
use super::speak::{SpeakApp, SpeakMode};

/// ScreenApp — the projector/presentation view (`?app=screen`). Shows the
/// context's currently active node next to the speaker list, mirroring the
/// React ScreenApp. Live: the active relation is re-resolved on each poll.
#[component]
pub fn ScreenApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    // Live projector: subscribe to the context's `active` (what to show) and
    // `screenComments` (whether the owner opted to show comments) relations so
    // remote changes update the projected pane without a reload (React's useSubsGet).
    let refresh = use_signal(|| 0u32);
    let sub_ctx = crate::graphql::gql_escape(&context_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ relations(where: {{ parentId: {{ _eq: \"{sub_ctx}\" }}, name: {{ _in: [\"active\", \"screenComments\", \"screenFeed\"] }} }}) {{ nodeId name }} }}"
        ),
        refresh,
    );
    let rev = *refresh.read();

    let active_ctx = context_id.clone();
    let active_token = access_token.clone();
    let active = crate::use_data_resource!(|(active_ctx, active_token, rev)| async move {
        let _ = rev;
        let id = graphql::active_node_id(active_token.as_deref(), &active_ctx)
            .await
            .ok()
            .flatten()?;
        graphql::query_node_by_id(active_token.as_deref(), &id)
            .await
            .ok()?
    });
    let active = active.read().clone().flatten();

    let comments_ctx = context_id.clone();
    let comments_token = session.read().access_token.clone();
    let show_comments =
        crate::use_data_resource!(|(comments_ctx, comments_token, rev)| async move {
            let _ = rev;
            graphql::screen_comments_on(comments_token.as_deref(), &comments_ctx)
                .await
                .unwrap_or(false)
        });
    let show_comments = show_comments.read().unwrap_or(false);

    // The feed as a projection target: the chair can put what the room has been
    // posting on the screen itself, rather than only the item under discussion.
    let feed_ctx = context_id.clone();
    let feed_token = access_token.clone();
    let show_feed = crate::use_data_resource!(|(feed_ctx, feed_token, rev)| async move {
        let _ = rev;
        graphql::screen_feed_on(feed_token.as_deref(), &feed_ctx)
            .await
            .unwrap_or(false)
    });
    let show_feed = show_feed.read().unwrap_or(false);

    // Live presenter focus: the section (heading anchor) the chair chose to bring
    // the room's attention to, for documents too long to show whole. Scrolled into
    // view on the projector when it (or the active node) changes.
    let focus_refresh = use_signal(|| 0u32);
    crate::subscription::use_live(
        format!(
            "subscription {{ relations(where: {{ parentId: {{ _eq: \"{sub_ctx}\" }}, name: {{ _like: \"focus:%\" }} }}) {{ name }} }}"
        ),
        focus_refresh,
    );
    let frev = *focus_refresh.read();
    let focus_ctx = context_id.clone();
    let focus_token = access_token.clone();
    let focus = crate::use_data_resource!(|(focus_ctx, focus_token, frev)| async move {
        let _ = frev;
        graphql::screen_focus_anchor(focus_token.as_deref(), &focus_ctx).await
    });
    let focus_anchor = focus.read().clone().flatten();
    {
        let anchor = focus_anchor.clone();
        let active_id = active.as_ref().map(|n| n.id.0.clone());
        use_effect(use_reactive!(|(anchor, active_id)| {
            let _ = &active_id;
            if let Some(a) = anchor.clone() {
                spawn(async move {
                    // Let the active node render before scrolling to its section.
                    gloo_timers::future::TimeoutFuture::new(160).await;
                    if let Some(el) = web_sys::window()
                        .and_then(|w| w.document())
                        .and_then(|d| d.get_element_by_id(&a))
                    {
                        let opts = web_sys::ScrollIntoViewOptions::new();
                        opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                        opts.set_block(web_sys::ScrollLogicalPosition::Start);
                        el.scroll_into_view_with_scroll_into_view_options(&opts);
                    }
                });
            }
        }));
    }

    rsx! {
        // Projector: a chromeless, room-distance 2-pane layout — a dominant hero
        // (the active content the room is looking at) + a supporting rail (the
        // current + on-deck speaker). Tonal, overscan-safe (see `.projector`).
        div { class: "projector",
            div { class: "projector-hero",
                match active.clone() {
                    // Whatever is projected loses to an explicit "show the feed":
                    // the chair asked for this one, and the active node is often
                    // just whatever was last discussed.
                    _ if show_feed => rsx! {
                        div { class: "card projector-feed",
                            div { class: "card-header",
                                div { class: "avatar", span { class: "material-icons", "view_agenda" } }
                                h3 { class: "title-medium", "{crate::i18n::t(\"layout.feed\")}" }
                            }
                            crate::components::feed::FeedList { context_id: context_id.clone(), instant: true }
                        }
                    },
                    Some(n) => rsx! { MimeLoader { key: "{n.id.0}", node: n, path: Vec::new(), projector: true } },
                    None => rsx! {
                        // DESIGN: an expressive idle state instead of a bare "…".
                        // The LARGE orb on purpose: this is the projector, read
                        // from across a room, and is the one place a full-size
                        // empty state belongs outside a page-level state.
                        div { class: "card",
                            div { class: "empty-state",
                                div { class: "empty-state-orb",
                                    span { class: "material-icons", "cast" }
                                }
                                p { class: "empty-state-body", "{crate::i18n::t(\"common.noContent\")}" }
                            }
                        }
                    },
                }
            }
            div { class: "projector-rail",
                SpeakApp { node: node.clone(), mode: SpeakMode::Screen }
                // Comments only when the owner opted in (screenComments relation).
                // Read-only on the room screen — the composer is hidden via CSS.
                if show_comments {
                    if let Some(n) = active.as_ref() {
                        div { class: "projector-comments",
                            super::comments::CommentSection {
                                node_id: n.id.0.clone(),
                                context_id: n.context_id.as_ref().map(|u| u.0.clone()),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// FollowApp — a personal-device "follow the room" view (`?app=follow`). Tracks the
/// context's `active` node (the item the chair projected) live and renders it
/// INTERACTIVELY, so a member reads the motion or casts a vote as focus moves,
/// auto-updating without a reload. The device-side mirror of [`ScreenApp`], which
/// shows the same active node room-facing.
#[component]
pub fn FollowApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let refresh = use_signal(|| 0u32);
    let sub_ctx = crate::graphql::gql_escape(&context_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ relations(where: {{ parentId: {{ _eq: \"{sub_ctx}\" }}, name: {{ _eq: \"active\" }} }}) {{ nodeId }} }}"
        ),
        refresh,
    );
    let rev = *refresh.read();

    let active = crate::use_data_resource!(|(context_id, access_token, rev)| async move {
        let _ = rev;
        let id = graphql::active_node_id(access_token.as_deref(), &context_id)
            .await
            .ok()
            .flatten()?;
        graphql::query_node_by_id(access_token.as_deref(), &id)
            .await
            .ok()?
    });
    let active = active.read().clone().flatten();

    rsx! {
        div { class: "follow-view",
            div { class: "status-banner is-live",
                span { class: "material-icons follow-pulse", "sensors" }
                span { "{crate::i18n::t(\"follow.live\")}" }
            }
            match active {
                Some(n) => rsx! { MimeLoader { key: "{n.id.0}", node: n, path: Vec::new() } },
                None => rsx! {
                    div { class: "card",
                        // In-card, so the small orb — the large one belongs to a
                        // page-level state (not found) or to the projector, which
                        // is read across a room.
                        div { class: "empty-state empty-state-sm",
                            div { class: "empty-state-orb empty-state-orb-sm",
                                span { class: "material-icons", "sensors_off" }
                            }
                            p { class: "empty-state-body", "{crate::i18n::t(\"follow.idle\")}" }
                        }
                    }
                },
            }
        }
    }
}
