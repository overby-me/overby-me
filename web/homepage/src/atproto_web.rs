//! Browser-side atproto resolution: the thin, `web-sys`-dependent glue that
//! fetches from PDS/appview/PLC and hands the pure logic in [`crate::atproto`]
//! the data it needs. All endpoints used here are CORS-enabled (verified), so
//! the avatar (loaded via the PDS blob store) can texture WebGL without tainting
//! the canvas; platform icons are generated badges, not fetched.

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use crate::atproto::{Platform, detect_platforms, fallback_avatar};
use crate::graph::data::{GraphData, GraphLink, GraphNode};

// --- JSON response shapes (only the fields we consume) ---

#[derive(Deserialize)]
struct ResolveHandle {
    did: String,
}

#[derive(Deserialize)]
struct DidDoc {
    #[serde(default)]
    service: Vec<Service>,
}

#[derive(Deserialize)]
struct Service {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "serviceEndpoint")]
    endpoint: String,
}

#[derive(Deserialize)]
struct DescribeRepo {
    #[serde(default)]
    collections: Vec<String>,
    handle: Option<String>,
}

#[derive(Deserialize)]
struct ProfileRecord {
    value: Option<ProfileValue>,
}

#[derive(Deserialize)]
struct ProfileValue {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    avatar: Option<Blob>,
}

#[derive(Deserialize)]
struct Blob {
    #[serde(rename = "ref")]
    blob_ref: Option<CidLink>,
    // Legacy blobs carry the CID directly rather than under `ref.$link`.
    cid: Option<String>,
}

#[derive(Deserialize)]
struct CidLink {
    #[serde(rename = "$link")]
    link: Option<String>,
}

/// GET `url` and deserialize the JSON body into `T`.
async fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|_| format!("network error fetching {url}"))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| "unexpected fetch result".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {} from {url}", resp.status()));
    }
    let promise = resp
        .json()
        .map_err(|_| "response was not JSON".to_string())?;
    let json = JsFuture::from(promise)
        .await
        .map_err(|_| "malformed JSON response".to_string())?;
    serde_wasm_bindgen::from_value(json).map_err(|e| format!("unexpected JSON shape: {e}"))
}

/// Resolve a handle (or raw DID) to a DID via the public appview.
async fn resolve_did(input: &str) -> Result<String, String> {
    if input.starts_with("did:") {
        return Ok(input.to_string());
    }
    let url = format!(
        "https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle?handle={input}"
    );
    let r: ResolveHandle = fetch_json(&url).await?;
    Ok(r.did)
}

/// Resolve a DID to its PDS endpoint via its DID document (PLC or did:web).
async fn resolve_pds(did: &str) -> Result<String, String> {
    let doc_url = if did.starts_with("did:plc:") {
        format!("https://plc.directory/{did}")
    } else if let Some(rest) = did.strip_prefix("did:web:") {
        // did:web maps ':' to '/'. With no sub-path the document lives at
        // "/.well-known/did.json" on the host, otherwise at "<sub-path>/did.json".
        match rest.split_once(':') {
            None => format!("https://{rest}/.well-known/did.json"),
            Some((host, sub)) => format!("https://{host}/{}/did.json", sub.replace(':', "/")),
        }
    } else {
        return Err(format!("unsupported DID method in {did}"));
    };

    let doc: DidDoc = fetch_json(&doc_url).await?;
    doc.service
        .into_iter()
        .find(|s| s.id.ends_with("atproto_pds") || s.kind == "AtprotoPersonalDataServer")
        .map(|s| s.endpoint.trim_end_matches('/').to_string())
        .filter(|e| !e.is_empty())
        .ok_or_else(|| "no atproto PDS listed in DID document".to_string())
}

/// Best-effort fetch of the Bluesky profile record for a display name and a
/// CORS-safe avatar URL (served through the account's own PDS blob store).
/// Missing profile is not an error — many atproto accounts have no bsky profile.
async fn fetch_profile(pds: &str, did: &str) -> (Option<String>, Option<String>) {
    let url = format!(
        "{pds}/xrpc/com.atproto.repo.getRecord?repo={did}&collection=app.bsky.actor.profile&rkey=self"
    );
    let Ok(rec) = fetch_json::<ProfileRecord>(&url).await else {
        return (None, None);
    };
    let Some(value) = rec.value else {
        return (None, None);
    };
    let avatar_cid = value
        .avatar
        .and_then(|a| a.blob_ref.and_then(|r| r.link).or(a.cid));
    let avatar_url =
        avatar_cid.map(|cid| format!("{pds}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}"));
    (value.display_name, avatar_url)
}

/// Assemble the hub-and-spoke graph: the account at the center, one leaf per
/// detected platform, each linked to the center.
fn build_graph(
    display_name: Option<String>,
    avatar_url: Option<String>,
    handle: &str,
    did: &str,
    platforms: Vec<Platform>,
) -> GraphData {
    // A stable center id independent of the (user-controlled) display text, so
    // it can never collide with a platform node's id.
    const CENTER_ID: &str = "\u{1}center";

    let is_did_handle = handle.starts_with("did:");
    let name = display_name.unwrap_or_else(|| handle.to_string());
    let desc = if is_did_handle {
        name.clone()
    } else {
        format!("{name}\n@{handle}")
    };
    // Prefer the real avatar; otherwise a matching generated badge.
    let center_icon = avatar_url.unwrap_or_else(|| fallback_avatar(&name, handle));

    let mut nodes = vec![GraphNode {
        id: CENTER_ID.to_string(),
        desc,
        icon: center_icon,
        color: None,
        opacity: None,
        url: Some(format!(
            "https://bsky.app/profile/{}",
            if is_did_handle { did } else { handle }
        )),
        center: true,
    }];
    let mut links = Vec::with_capacity(platforms.len());

    for p in platforms {
        links.push(GraphLink {
            source: CENTER_ID.to_string(),
            target: p.name.clone(),
        });
        nodes.push(GraphNode {
            id: p.name.clone(),
            desc: format!("{}\nProfile", p.name),
            icon: p.icon,
            color: None,
            opacity: None,
            url: Some(p.profile_url),
            center: false,
        });
    }

    // Atproto badges + the avatar are square textures; render them as circles.
    GraphData {
        nodes,
        links,
        circular_icons: true,
    }
}

/// Resolve an atproto handle (as typed after the `@`, or a raw DID) into a
/// ready-to-render graph of every platform the account uses.
pub async fn resolve_graph(raw: &str) -> Result<GraphData, String> {
    let input = raw.trim().trim_start_matches('@').trim().to_lowercase();
    if input.is_empty() {
        return Err("Enter an atproto handle, e.g. @overby.me".to_string());
    }

    let did = resolve_did(&input)
        .await
        .map_err(|e| format!("Couldn't resolve @{input} ({e})"))?;
    let pds = resolve_pds(&did)
        .await
        .map_err(|e| format!("Couldn't find the PDS for {did} ({e})"))?;
    let repo: DescribeRepo = fetch_json(&format!(
        "{pds}/xrpc/com.atproto.repo.describeRepo?repo={did}"
    ))
    .await
    .map_err(|e| format!("Couldn't read the repo ({e})"))?;

    if repo.collections.is_empty() {
        return Err(format!("@{input} has no atproto records yet"));
    }

    let handle = repo
        .handle
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| input.clone());
    let (display_name, avatar_url) = fetch_profile(&pds, &did).await;
    let platforms = detect_platforms(&repo.collections, &handle, &did);

    Ok(build_graph(
        display_name,
        avatar_url,
        &handle,
        &did,
        platforms,
    ))
}
