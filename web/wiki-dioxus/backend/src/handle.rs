//! Scaleway serverless function backing the RadikalWiki (wiki-dioxus) frontend.
//!
//! Its first job is the "Link Bluesky account" (atproto OAuth) flow. The browser
//! cannot do this cleanly on its own — PAR, DPoP-bound token exchange and the
//! redirect callback all belong server-side — so it lives here (a stateless
//! version of the earlier `wiki-auth` idea).
//!
//! Routes:
//!   GET /atproto/client-metadata.json  the OAuth client document (also client_id)
//!   GET /atproto/start?handle=&token=  begin linking (token = NHost access token)
//!   GET /atproto/callback              finish linking, write user_providers
//!   GET /health                        liveness
//!
//! Config comes from Scaleway function secrets (see [`oauth::Config::from_env`]).

use axum::{body::Body, extract::Request, response::Response};
use http::StatusCode;

mod dpop;
mod nhost;
mod notify;
mod oauth;
mod pkce;
mod push;
mod roster;
mod statecookie;
mod util;
mod vote;

pub async fn handle(req: Request<Body>) -> Response<Body> {
    let cfg = oauth::Config::from_env();
    let client = reqwest::Client::new();

    // CORS preflight: the header-authenticated fetches are non-simple requests.
    if req.method() == http::Method::OPTIONS {
        return preflight();
    }

    // Owned up front so the roster route can consume the request body afterwards.
    let path = req.uri().path().to_string();
    // Prefer the session JWT from the `Authorization: Bearer` header — it keeps the
    // token out of the URL (browser history / Referer / access logs); the `?token=`
    // query param stays a fallback (and is the only option for the `start`
    // full-page redirect).
    let bearer_owned = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);

    // Roster parse reads the uploaded .xlsx from the request body (consumes req).
    if path == "/roster/parse" {
        return roster::handle_parse(&cfg, req, bearer_owned.as_deref()).await;
    }

    // Extract the rest as `&str` (Send) — never hold a `&Request` across an await,
    // since axum's Body is Send but not Sync, making the future non-Send.
    let query = req.uri().query();
    let cookie_header = req
        .headers()
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok());
    let bearer = bearer_owned.as_deref();

    match path.as_str() {
        "/atproto/client-metadata.json" => client_metadata(&cfg),
        "/atproto/start" => oauth::start(&cfg, &client, query).await,
        "/atproto/callback" => oauth::callback(&cfg, &client, query, cookie_header).await,
        "/atproto/status" => oauth::status(&cfg, &client, query, bearer).await,
        "/atproto/unlink" => oauth::unlink(&cfg, &client, query, bearer).await,
        "/atproto/post" => oauth::post(&cfg, &client, query, bearer).await,
        "/vote/cast" => vote::cast(&cfg, &client, query, bearer).await,
        "/vote/status" => vote::status(&cfg, &client, query, bearer).await,
        "/push/subscribe" => notify::subscribe(&cfg, &client, query, bearer).await,
        "/push/unsubscribe" => notify::unsubscribe(&cfg, &client, query, bearer).await,
        "/push/notify" => notify::notify(&cfg, &client, query, bearer).await,
        "/push/reply" => notify::reply(&cfg, &client, query, bearer).await,
        "/health" => text(StatusCode::OK, "ok"),
        _ => text(StatusCode::NOT_FOUND, "not found"),
    }
}

/// CORS preflight response allowing the app's header-authenticated fetches.
fn preflight() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header(
            "Access-Control-Allow-Headers",
            "authorization, content-type",
        )
        .header("Access-Control-Max-Age", "86400")
        .body(Body::empty())
        .unwrap()
}

/// The atproto OAuth *client metadata* document, served at a stable public URL
/// that doubles as the `client_id`. A public client (`token_endpoint_auth_method
/// = none`) with PKCE + DPoP, so no client secret / JWKS is required.
fn client_metadata(cfg: &oauth::Config) -> Response<Body> {
    let doc = serde_json::json!({
        "client_id": cfg.client_id(),
        "client_name": "RadikalWiki",
        "client_uri": cfg.app_origin,
        "redirect_uris": [cfg.redirect_uri()],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "scope": oauth::SCOPE,
        "token_endpoint_auth_method": "none",
        "application_type": "web",
        "dpop_bound_access_tokens": true,
    });
    json(StatusCode::OK, doc.to_string())
}

// --- response helpers (shared with the oauth module) -----------------------

pub(crate) fn text(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub(crate) fn json(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(body))
        .unwrap()
}

pub(crate) fn redirect(location: &str, err: Option<&str>) -> Response<Body> {
    if let Some(e) = err {
        eprintln!("atproto link error: {e}");
    }
    Response::builder()
        .status(StatusCode::FOUND)
        .header("Location", location)
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn redirect_set_cookie(location: &str, cookie: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header("Location", location)
        .header("Set-Cookie", cookie)
        .body(Body::empty())
        .unwrap()
}
