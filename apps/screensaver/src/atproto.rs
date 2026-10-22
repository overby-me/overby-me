//! The atproto lookups behind `?images=@handle` and `?text=@handle`. Every
//! endpoint is CORS-enabled, so a fetched picture can texture WebGL without
//! tainting the canvas.

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

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

/// GET `url` and deserialize the JSON body into `T`.
pub(crate) async fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("network error fetching {url}: {e:?}"))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|e| format!("unexpected fetch result: {e:?}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {} from {url}", resp.status()));
    }
    let promise = resp
        .json()
        .map_err(|e| format!("response was not JSON: {e:?}"))?;
    let json = JsFuture::from(promise)
        .await
        .map_err(|e| format!("malformed JSON response: {e:?}"))?;
    serde_wasm_bindgen::from_value(json).map_err(|e| format!("unexpected JSON shape: {e}"))
}

/// Resolve a handle (or raw DID) to a DID via the public appview.
pub(crate) async fn resolve_did(input: &str) -> Result<String, String> {
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
pub(crate) async fn resolve_pds(did: &str) -> Result<String, String> {
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
