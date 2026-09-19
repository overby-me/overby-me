//! Sharing a page to the member's own Bluesky account: an `app.bsky.feed.post`
//! written into THEIR repo, on THEIR PDS, with the OAuth session their login
//! left here. Carried over from the interim backend's `/atproto/post`, which
//! kept its own sealed copy of the session and refreshed it by hand; atrium's
//! session store does both now.

use crate::AppState;
use crate::session::Caller;
use crate::xrpc::{conflict, err, invalid};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

/// Bluesky caps a post at 300 graphemes. Counted in `char`s, which is never
/// fewer, so nothing the PDS would refuse for length is sent.
const MAX_POST_CHARS: usize = 300;

#[derive(Debug, Deserialize)]
pub struct ShareBody {
    pub text: String,
    /// The page being shared, shown as a link card.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// The post record. Bluesky does not linkify: where the text contains the URL,
/// a facet over its BYTE range makes it a link, and the URL always rides along
/// as an external card.
fn post_record(text: &str, url: Option<&str>, title: &str, created_at: &str) -> serde_json::Value {
    let mut record = json!({
        "$type": "app.bsky.feed.post",
        "text": text,
        "createdAt": created_at,
    });
    if let Some(url) = url {
        if let Some(start) = text.find(url) {
            record["facets"] = json!([{
                "index": { "byteStart": start, "byteEnd": start + url.len() },
                "features": [{ "$type": "app.bsky.richtext.facet#link", "uri": url }],
            }]);
        }
        record["embed"] = json!({
            "$type": "app.bsky.embed.external",
            "external": { "uri": url, "title": title, "description": "" },
        });
    }
    record
}

/// `com.example.wiki.shareToBluesky` (procedure): post to the caller's own
/// account. Only a page of this app can be the card: what is posted carries the
/// member's name, and must not be pointed elsewhere by whoever crafted the call.
pub async fn share_to_bluesky(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ShareBody>,
) -> Response {
    let text = body.text.trim();
    if text.is_empty() {
        return invalid("a post needs some text");
    }
    if text.chars().count() > MAX_POST_CHARS {
        return invalid("a post is at most 300 characters");
    }
    let url = body.url.as_deref().filter(|url| !url.is_empty());
    if url.is_some_and(|url| !state.config.allows_return(url)) {
        return invalid("only a page of this app can be shared");
    }
    let Some(oauth) = &state.oauth else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuthNotConfigured",
            "this AppView cannot reach a PDS",
        );
    };
    let record = post_record(
        text,
        url,
        body.title.as_deref().unwrap_or_default(),
        &crate::util::rfc3339_utc(crate::util::now_secs()),
    );
    match oauth
        .create_record(&did, "app.bsky.feed.post", record)
        .await
    {
        Ok(uri) => (StatusCode::OK, Json(json!({ "uri": uri }))).into_response(),
        Err(e) => {
            // The usual reason: the grant their login left here has lapsed or
            // been withdrawn, which signing in again repairs.
            tracing::warn!("share to bluesky failed for {did}: {e}");
            conflict(
                "PdsRefused",
                "your account's server did not take the post; signing in again usually fixes that",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{post, seeded_state, token_for};

    #[test]
    fn a_link_in_the_text_is_marked_by_its_bytes_not_its_letters() {
        let url = format!("https://{}/foreningen/årsmøde", "wiki.example");
        let text = format!("Læs referatet fra årsmødet: {url}");
        let record = post_record(&text, Some(&url), "Årsmøde", "2026-09-19T12:00:00.000Z");
        let index = &record["facets"][0]["index"];
        let (start, end) = (
            index["byteStart"].as_u64().expect("start") as usize,
            index["byteEnd"].as_u64().expect("end") as usize,
        );
        assert_eq!(&text.as_bytes()[start..end], url.as_bytes());
        assert!(
            start > text.chars().take_while(|c| *c != 'h').count(),
            "counted in letters"
        );
        assert_eq!(record["embed"]["external"]["uri"], url.as_str());
        assert_eq!(record["embed"]["external"]["title"], "Årsmøde");

        let bare = post_record("Kun tekst", None, "", "2026-09-19T12:00:00.000Z");
        assert!(bare.get("facets").is_none() && bare.get("embed").is_none());
        let card_only = post_record("Se her", Some(&url), "", "2026-09-19T12:00:00.000Z");
        assert!(
            card_only.get("facets").is_none(),
            "a facet over text that is not there"
        );
        assert!(card_only.get("embed").is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_may_not_be_posted_is_refused_before_any_pds_is_asked() {
        let mut state = seeded_state().await;
        state.config.frontend_origins = vec!["https://wiki.example".to_string()];
        let bob = token_for(&state, "did:plc:bob").await;
        let uri = "/xrpc/com.example.wiki.shareToBluesky";
        for (why, body) in [
            ("no text", json!({"text": "  "})),
            ("too long", json!({"text": "x".repeat(301)})),
            (
                "a card for somewhere else",
                json!({"text": "se", "url": "https://evil.example/"}),
            ),
        ] {
            let (status, v) = post(router(state.clone()), uri, Some(&bob), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        }
        let (status, _) = post(router(state.clone()), uri, None, json!({"text": "hej"})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // The test state has no OAuth client, which is said and not hidden.
        let page = json!({"text": "hej", "url": "https://wiki.example/closed"});
        let (status, v) = post(router(state.clone()), uri, Some(&bob), page).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    }
}
