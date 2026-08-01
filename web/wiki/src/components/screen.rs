use dioxus::prelude::*;

use crate::graphql::{self};
use crate::model::NodeWithChildren;
use crate::session::use_session;

use super::loader::MimeLoader;
use super::speak::{SpeakApp, SpeakMode};

/// ScreenApp — the projector/presentation view (`?app=screen`). Shows the
/// context's currently active node next to the speaker list, mirroring the
/// React ScreenApp. Live: the active relation is re-resolved on each poll.
/// The part of a crumb trail the ROOM needs: the content ancestry, without the
/// filing.
///
/// A group, an event and the folders inside them are how the room got here, not
/// what it is looking at, and on a projector every extra glyph is one the eye
/// has to discard from across a hall. What is left is the political line of
/// descent — a policy, the change to it, the change to that — which is exactly
/// what a bare "Change 2" fails to say on its own.
fn content_trail(crumbs: Vec<crate::model::Crumb>) -> Vec<crate::model::Crumb> {
    crumbs
        .into_iter()
        .filter(|c| {
            !matches!(
                c.mime_id.as_deref().unwrap_or(""),
                "wiki/group" | "wiki/event" | "wiki/folder" | "wiki/home"
            )
        })
        .collect()
}

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
        crate::graphql::relations_changed(crate::graphql::relations_named(
            &sub_ctx,
            &["active", "screenComments", "screenFeed"],
        )),
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

    // The line of descent of what is projected, as the same letter/number avatars
    // the rest of the app labels these nodes with: Policy A, its Change 3, and
    // that change's Change 2 read as A · 3 · 2 above the content.
    //
    // The room needs it and only the room lacks it. Everywhere else the trail is
    // on screen — breadcrumbs, the tree, the folder you came through — but the
    // projector is chromeless by design, so a change to a change arrives with no
    // statement of what it changes, and "Change 2" alone is unreadable from the
    // back of a hall.
    let trail_path = active.as_ref().and_then(|n| n.path.clone());
    let trail_token = access_token.clone();
    let trail = crate::use_data_resource!(|(trail_path, trail_token)| async move {
        let Some(path) = trail_path.filter(|p| !p.is_empty()) else {
            return Vec::new();
        };
        let segments: Vec<String> = path.split('/').map(str::to_string).collect();
        graphql::path_crumbs(trail_token.as_deref(), &segments)
            .await
            .unwrap_or_default()
    });
    let trail = content_trail(trail.read().clone().unwrap_or_default());

    // Live presenter focus: the section (heading anchor) the chair chose to bring
    // the room's attention to, for documents too long to show whole. Scrolled into
    // view on the projector when it (or the active node) changes.
    let focus_refresh = use_signal(|| 0u32);
    crate::subscription::use_live(
        crate::graphql::relations_changed(crate::graphql::relations_like(&sub_ctx, "focus:%")),
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
                // Only when there is an ancestry to state: a plain document
                // projected from a folder is its own whole answer, and a row
                // holding one avatar of itself would be furniture.
                if !show_feed && trail.len() > 1 {
                    div { class: "projector-trail",
                        for (i, crumb) in trail.iter().enumerate() {
                            if i > 0 {
                                span { class: "projector-trail-sep", "·" }
                            }
                            div {
                                key: "{crumb.key}",
                                class: if i + 1 == trail.len() { "avatar projector-trail-avatar is-current" } else { "avatar projector-trail-avatar" },
                                title: "{crumb.name}",
                                {super::loader::node_avatar(
                                    &super::loader::node_icon_mime_id(
                                        crumb.mime_id.as_deref().unwrap_or(""),
                                        crumb.data.as_ref().map(|d| &d.0),
                                    ),
                                    &crumb.name,
                                    crumb.ordinal,
                                )}
                            }
                        }
                    }
                }
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
        crate::graphql::relations_changed(crate::graphql::relation_named(&sub_ctx, "active")),
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

#[cfg(test)]
mod tests {
    use super::content_trail;
    use crate::model::Crumb;

    fn crumb(key: &str, mime: &str, ordinal: Option<usize>) -> Crumb {
        Crumb {
            key: key.to_string(),
            name: key.to_string(),
            mime_id: Some(mime.to_string()),
            ordinal,
            data: None,
        }
    }

    #[test]
    fn the_room_sees_the_political_line_only() {
        // radikal_ungdom / landsmøde / resolutioner / Policy A / Change 3 / Change 2
        let trail = content_trail(vec![
            crumb("radikal_ungdom", "wiki/group", None),
            crumb("landsmode", "wiki/event", None),
            crumb("resolutioner", "wiki/folder", None),
            crumb("policy", "vote/policy", Some(0)),
            crumb("change_a", "vote/change", Some(2)),
            crumb("change_b", "vote/change", Some(1)),
        ]);
        let keys: Vec<&str> = trail.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, vec!["policy", "change_a", "change_b"]);
        // A · 3 · 2: the ordinals the avatars label them with survive the filter.
        let ordinals: Vec<Option<usize>> = trail.iter().map(|c| c.ordinal).collect();
        assert_eq!(ordinals, vec![Some(0), Some(2), Some(1)]);
    }

    #[test]
    fn a_document_in_a_folder_states_nothing() {
        // One entry left is the item itself, and a row holding an avatar of what
        // you are already looking at is furniture. The view requires len > 1.
        let trail = content_trail(vec![
            crumb("radikal_ungdom", "wiki/group", None),
            crumb("papers", "wiki/folder", None),
            crumb("dagsorden", "wiki/document", None),
        ]);
        assert_eq!(trail.len(), 1);
    }

    #[test]
    fn a_candidate_keeps_the_position_it_stands_for() {
        let trail = content_trail(vec![
            crumb("event", "wiki/event", None),
            crumb("formand", "vote/position", None),
            crumb("asger", "vote/candidate", None),
        ]);
        assert_eq!(trail.len(), 2);
    }
}
