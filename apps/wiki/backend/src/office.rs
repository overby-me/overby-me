//! Office-document proxy, so the Microsoft web viewer can render a private file.
//!
//! That viewer takes a URL and fetches it from MICROSOFT'S servers, then fetches
//! it again on later renders. It therefore needs a URL that is reachable without
//! a session and stays reachable for a while. Neither is true of storage any
//! more: a file is readable only through the node that references it, and the
//! presigned URL that grants access lives 30 seconds.
//!
//! So the browser never hands Microsoft a storage URL. It asks here for a signed
//! link; this module checks the caller may read that file, mints a capability URL
//! valid for [`LINK_TTL_SECS`], and serves the bytes on that URL by fetching them
//! from storage with the service credential.
//!
//! Two properties worth keeping in mind:
//!
//! * The link IS the capability. Anyone holding it reads that one document until
//!   it expires — necessarily, since Microsoft presents no credentials. It is
//!   unguessable (HMAC over file id + expiry) and scoped to a single file.
//! * Access is decided by the CALLER'S permissions, not the service credential:
//!   the check below queries Hasura with the caller's own token, so the row-level
//!   rules apply exactly as they do in the app. The admin credential is used only
//!   afterwards, to read bytes the caller has already been shown to be entitled
//!   to.
//!
//! Routes:
//!   GET /office/sign?fileId=  (authenticated) -> { url, expires }
//!   GET /office/file?f=&e=&s= (public, signed) -> the document bytes

use crate::error::AppError;
use crate::oauth::Config;
use axum::body::Body;
use axum::response::Response;
use hmac::{Hmac, Mac};
use http::StatusCode;
use serde_json::json;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// How long a minted link stays valid. Long enough for Microsoft to fetch, and
/// to re-fetch when the user scrolls or the iframe re-renders; short enough that
/// a leaked link stops working the same afternoon.
const LINK_TTL_SECS: u64 = 2 * 60 * 60;

/// Mint a signed, time-limited URL for a document the caller may read.
pub async fn sign(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match sign_inner(cfg, client, query, bearer).await {
        Ok(body) => crate::json(StatusCode::OK, body.to_string()),
        Err(e) => e.respond("office sign"),
    }
}

async fn sign_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<serde_json::Value, AppError> {
    let params = crate::util::parse_query(query);
    let file_id = params
        .iter()
        .find(|(k, _)| k == "fileId")
        .map(|(_, v)| v.clone())
        .ok_or(AppError::BadRequest("missing fileId".into()))?;
    // Rejects a malformed id before it reaches the signature or storage.
    if !is_uuid(&file_id) {
        return Err(AppError::BadRequest("bad fileId".into()));
    }
    let token = crate::auth::token_from(query, bearer)
        .ok_or(AppError::Unauthorized("missing token".into()))?;
    // Verify the session before spending a round trip on it.
    crate::auth::caller_uid(cfg, query, bearer)?;

    // The caller's OWN token, deliberately: whether they may read this file is a
    // question for the row-level permissions, not for this service.
    let q = json!({
        "query": "query($id: uuid!) { files(where: {id: {_eq: $id}}, limit: 1) { id } }",
        "variables": { "id": file_id },
    });
    let resp = client
        .post(&cfg.hasura_url)
        .bearer_auth(&token)
        .json(&q)
        .send()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    let visible = body
        .pointer("/data/files")
        .and_then(|a| a.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if !visible {
        return Err(AppError::Forbidden("no access to that file".into()));
    }

    let expires = crate::util::now_secs() + LINK_TTL_SECS;
    let sig = sign_link(&cfg.state_secret, &file_id, expires);
    Ok(json!({
        "url": format!("{}/office/file?f={file_id}&e={expires}&s={sig}", cfg.function_origin),
        "expires": expires,
    }))
}

/// Serve the bytes on a link minted above. No session: the signature is the
/// authorization, because the fetch comes from Microsoft, not from a browser.
pub async fn file(cfg: &Config, client: &reqwest::Client, query: Option<&str>) -> Response<Body> {
    match file_inner(cfg, client, query).await {
        Ok(resp) => resp,
        Err(e) => e.respond("office file"),
    }
}

async fn file_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
) -> Result<Response<Body>, AppError> {
    let params = crate::util::parse_query(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let file_id = get("f");
    let expires: u64 = get("e").parse().unwrap_or(0);
    let sig = get("s");

    if !is_uuid(&file_id) {
        return Err(AppError::BadRequest("bad file".into()));
    }
    if expires <= crate::util::now_secs() {
        return Err(AppError::Forbidden("link expired".into()));
    }
    // Constant-time: `Mac::verify_slice` compares the tag without leaking where
    // it first differs.
    let mut mac = HmacSha256::new_from_slice(cfg.state_secret.as_bytes())
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    mac.update(format!("{file_id}.{expires}").as_bytes());
    let expected = crate::util::b64url_decode(&sig)
        .map_err(|_mac_err| AppError::Forbidden("bad signature".into()))?;
    mac.verify_slice(&expected)
        .map_err(|_mac_err| AppError::Forbidden("bad signature".into()))?;

    // Entitlement was settled when the link was minted; read the bytes with the
    // service credential, which is the only identity this request has.
    let upstream = client
        .get(format!("{}/files/{file_id}", storage_url(cfg)))
        .header("x-hasura-admin-secret", &cfg.admin_secret)
        .send()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    if !upstream.status().is_success() {
        return Err(AppError::Upstream(format!(
            "storage returned {}",
            upstream.status()
        )));
    }
    let content_type = upstream
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = upstream
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, content_type)
        // Microsoft fetches this from its own infrastructure, so it must not be
        // treated as same-origin content, and no shared cache should keep it: the
        // URL is a capability and it expires.
        .header(http::header::CACHE_CONTROL, "private, max-age=0, no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Upstream(e.to_string()))
}

/// The storage origin. Derived from the GraphQL URL by default, since both are
/// the same NHost project under different subdomains, and overridable for a
/// deployment where that does not hold.
fn storage_url(cfg: &Config) -> String {
    std::env::var("NHOST_STORAGE_URL").unwrap_or_else(|_| {
        cfg.hasura_url
            .replace(".hasura.", ".storage.")
            .replace("/v1/graphql", "/v1")
    })
}

fn sign_link(secret: &str, file_id: &str, expires: u64) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key size");
    mac.update(format!("{file_id}.{expires}").as_bytes());
    crate::util::b64url(&mac.finalize().into_bytes())
}

/// Shape check only — the id is echoed into a URL and a storage path, so it must
/// not carry anything else.
fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.as_bytes()[8] == b'-'
        && s.as_bytes()[13] == b'-'
        && s.as_bytes()[18] == b'-'
        && s.as_bytes()[23] == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_bound_to_both_file_and_expiry() {
        let a = sign_link("secret", "0fd9c055-36cf-4551-92a8-e3ac7e32636c", 1000);
        // Same file, different deadline.
        let b = sign_link("secret", "0fd9c055-36cf-4551-92a8-e3ac7e32636c", 1001);
        // Different file, same deadline.
        let c = sign_link("secret", "d4e8c1c5-0409-4e6c-a85e-7504c3329282", 1000);
        assert_ne!(a, b, "expiry must be covered, or a link never dies");
        assert_ne!(a, c, "file must be covered, or one link reads any file");
        // A different secret must not produce a link this deployment accepts.
        assert_ne!(
            a,
            sign_link("other", "0fd9c055-36cf-4551-92a8-e3ac7e32636c", 1000)
        );
    }

    #[test]
    fn uuid_shape_is_enforced() {
        assert!(is_uuid("0fd9c055-36cf-4551-92a8-e3ac7e32636c"));
        assert!(!is_uuid("../../etc/passwd"));
        assert!(!is_uuid("0fd9c055-36cf-4551-92a8-e3ac7e32636"));
        assert!(!is_uuid("0fd9c055/36cf/4551/92a8/e3ac7e32636c"));
        assert!(!is_uuid(""));
    }
}
