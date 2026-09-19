//! Bluesky's public AppView, asked directly. It takes no session and is no part
//! of the wiki's backend, so it is the same call whichever backend that is.

use serde::Deserialize;

const HOST: &str = "public.api.bsky.app";

/// A Bluesky account suggestion for the link-handle typeahead.
#[derive(Clone, PartialEq, Deserialize, Default)]
pub struct BskyActor {
    pub handle: String,
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    #[serde(default)]
    pub avatar: String,
}

/// Typeahead search for Bluesky accounts matching `query`, so a handle can be
/// picked instead of typed exactly. Empty on any error or a too-short query.
pub async fn search_bsky_actors(query: &str) -> Vec<BskyActor> {
    let q = query.trim();
    if q.len() < 2 {
        return Vec::new();
    }
    let url = format!("https://{HOST}/xrpc/app.bsky.actor.searchActorsTypeahead");
    match reqwest::Client::new()
        .get(url)
        .query(&[("q", q), ("limit", "6")])
        .send()
        .await
    {
        Ok(resp) => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v.get("actors").cloned().unwrap_or_default()).ok())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}
