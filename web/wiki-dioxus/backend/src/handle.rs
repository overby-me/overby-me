//! Scaleway serverless function backing the RadikalWiki (wiki-dioxus) frontend.
//!
//! Its first job is the "Link Bluesky account" (atproto OAuth) flow. The browser
//! cannot do this cleanly on its own — PAR, DPoP-bound token exchange and the
//! redirect callback all belong server-side — so it lives here (the same idea as
//! the earlier `wiki-auth` service, but as a stateless Scaleway function).
//!
//! Routes:
//!   GET /atproto/client-metadata.json  the OAuth client document (also client_id)
//!   GET /atproto/start                 begin linking: verify the NHost user,
//!                                       resolve handle -> DID -> PDS/auth server,
//!                                       run PAR, then 302 to the authorize URL
//!   GET /atproto/callback              exchange code -> DPoP tokens, read the DID,
//!                                       write the user_providers link, 302 back
//!   GET /health                        liveness
//!
//! Config comes from Scaleway function secrets (environment variables):
//!   APP_ORIGIN           the wiki origin to redirect back to after linking
//!   FUNCTION_ORIGIN      this function's public base URL (the OAuth client_id host)
//!   HASURA_GRAPHQL_URL   GraphQL endpoint used to write user_providers
//!   HASURA_ADMIN_SECRET  service access to insert the link row
//!   NHOST_JWKS_URL       to verify the caller's NHost access token

use axum::{body::Body, extract::Request, response::Response};
use http::StatusCode;

pub async fn handle(req: Request<Body>) -> Response<Body> {
    match req.uri().path() {
        "/atproto/client-metadata.json" => client_metadata(),
        "/atproto/start" => atproto_start(req).await,
        "/atproto/callback" => atproto_callback(req).await,
        "/health" => text(StatusCode::OK, "ok"),
        _ => text(StatusCode::NOT_FOUND, "not found"),
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

fn text(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn json(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(body))
        .unwrap()
}

/// The atproto OAuth *client metadata* document. Served at a stable public URL
/// which doubles as the `client_id`. A public client (`token_endpoint_auth_method
/// = none`) with PKCE + DPoP, so no client secret / JWKS is needed.
/// See https://atproto.com/specs/oauth.
fn client_metadata() -> Response<Body> {
    let func = env("FUNCTION_ORIGIN");
    let doc = serde_json::json!({
        "client_id": format!("{func}/atproto/client-metadata.json"),
        "client_name": "RadikalWiki",
        "client_uri": env("APP_ORIGIN"),
        "redirect_uris": [format!("{func}/atproto/callback")],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "scope": "atproto",
        "token_endpoint_auth_method": "none",
        "application_type": "web",
        "dpop_bound_access_tokens": true,
    });
    json(StatusCode::OK, doc.to_string())
}

async fn atproto_start(_req: Request<Body>) -> Response<Body> {
    // TODO: verify the NHost access token (query param) to know which user is
    // linking; resolve the handle to a DID, PDS and authorization server;
    // generate PKCE + DPoP key; run a pushed authorization request (PAR); then
    // 302 to the authorization endpoint with the returned request_uri.
    text(
        StatusCode::NOT_IMPLEMENTED,
        "atproto link start: not yet implemented",
    )
}

async fn atproto_callback(_req: Request<Body>) -> Response<Body> {
    // TODO: exchange the authorization code for DPoP-bound tokens, read the DID
    // from the token response, insert the user_providers row (provider=atproto,
    // provider_id=DID, handle) via the Hasura admin secret, then 302 back to
    // APP_ORIGIN with a success flag.
    text(
        StatusCode::NOT_IMPLEMENTED,
        "atproto link callback: not yet implemented",
    )
}
