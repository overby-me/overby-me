//! What a room's screen shows: the node on it, where in that node, whether the
//! talk and the feed are up beside it, and which board is the room's.
//!
//! The interim keeps each of these as a relation row of its own and the AppView
//! as one projector state per context, so each question here reads that one
//! state and each change writes one field of it.

use super::{ask, ask_quiet, client};
use appview_client::{get_projector, set_projector};

async fn screen(access_token: Option<&str>, context_id: &str) -> Option<get_projector::Output> {
    let client = client(access_token);
    let params = get_projector::Params {
        context: context_id.to_string(),
    };
    // Quiet: a context the reader may not read has no screen to show them.
    ask_quiet(true, || client.get_projector(&params)).await.ok()
}

async fn change(access_token: Option<&str>, change: set_projector::Input) -> Result<(), String> {
    let client = client(access_token);
    ask("setProjector", false, || client.set_projector(&change)).await?;
    Ok(())
}

/// The node on a context's screen. During a vote it is the open poll.
pub async fn active_node_id(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Option<String>, String> {
    Ok(screen(access_token, context_id)
        .await
        .and_then(|s| s.active_id))
}

/// Put a node on the screen, or with `None` take what is there off it.
pub async fn set_active_relation(
    access_token: Option<&str>,
    context_id: &str,
    node_id: Option<&str>,
) -> Result<bool, String> {
    let shown = set_projector::Input {
        context_id: context_id.to_string(),
        active_id: Some(node_id.map(str::to_string)),
        ..Default::default()
    };
    change(access_token, shown).await.map(|()| true)
}

pub async fn screen_focus_anchor(access_token: Option<&str>, context_id: &str) -> Option<String> {
    screen(access_token, context_id).await.and_then(|s| s.focus)
}

pub async fn set_screen_focus(
    access_token: Option<&str>,
    context_id: &str,
    anchor: Option<&str>,
) -> Result<(), String> {
    let focused = set_projector::Input {
        context_id: context_id.to_string(),
        focus: Some(anchor.map(str::to_string)),
        ..Default::default()
    };
    change(access_token, focused).await
}

pub async fn screen_comments_on(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<bool, String> {
    Ok(screen(access_token, context_id)
        .await
        .is_some_and(|s| s.show_comments))
}

pub async fn set_screen_comments(
    access_token: Option<&str>,
    context_id: &str,
    on: bool,
) -> Result<bool, String> {
    let shown = set_projector::Input {
        context_id: context_id.to_string(),
        show_comments: Some(on),
        ..Default::default()
    };
    change(access_token, shown).await.map(|()| true)
}

pub async fn screen_feed_on(access_token: Option<&str>, context_id: &str) -> Result<bool, String> {
    Ok(screen(access_token, context_id)
        .await
        .is_some_and(|s| s.show_feed))
}

pub async fn set_screen_feed(
    access_token: Option<&str>,
    context_id: &str,
    on: bool,
) -> Result<bool, String> {
    let shown = set_projector::Input {
        context_id: context_id.to_string(),
        show_feed: Some(on),
        ..Default::default()
    };
    change(access_token, shown).await.map(|()| true)
}

/// The board a context is showing, if its owner chose one.
pub async fn focused_canvas(access_token: Option<&str>, context_id: &str) -> Option<String> {
    screen(access_token, context_id)
        .await
        .and_then(|s| s.canvas_id)
}

pub async fn set_focused_canvas(
    access_token: Option<&str>,
    context_id: &str,
    canvas_id: Option<&str>,
) -> Result<(), String> {
    let chosen = set_projector::Input {
        context_id: context_id.to_string(),
        canvas_id: Some(canvas_id.map(str::to_string)),
        ..Default::default()
    };
    change(access_token, chosen).await
}
