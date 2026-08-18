//! Browser-side atproto resolution: the thin, `web-sys`-dependent glue that
//! fetches from PDS/appview/PLC and hands the pure logic in [`crate::atproto`]
//! the data it needs. All endpoints used here are CORS-enabled (verified), so
//! the avatar (loaded via the PDS blob store) can texture WebGL without tainting
//! the canvas; platform icons are generated badges, not fetched.

use std::collections::HashMap;

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use crate::atproto::{
    CATEGORY_ORDER, Icon, Platform, bridged_mastodon, category_meta, detect_platforms,
    external_platforms, fallback_avatar,
};
use crate::graph::data::{GraphData, GraphLink, GraphNode};

/// The atstore.fyi app directory: its catalog is a public collection of records,
/// each mapping an app (by website) to an icon blob. We use it as a live logo
/// source for apps we don't ship a hand-picked logo for (prototype).
const ATSTORE_DID: &str = "did:plc:dvy6bdnofdfc4php4s5b457d";
const ATSTORE_LISTINGS: &str = "fyi.atstore.listing.detail";

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
    description: Option<String>,
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

#[derive(Deserialize)]
struct ListRecords<T> {
    #[serde(default = "Vec::new")]
    records: Vec<Record<T>>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct Record<T> {
    value: T,
}

#[derive(Deserialize)]
struct AtstoreListing {
    #[serde(rename = "externalUrl")]
    external_url: Option<String>,
    icon: Option<Blob>,
}

// --- External-link record shapes (only the URL-bearing fields) ---

/// `id.sifa.profile.externalAccount`: a structured link to an off-atproto account.
#[derive(Deserialize)]
struct SifaExternalAccount {
    url: Option<String>,
    label: Option<String>,
}

/// `app.lanyards.actor.biography.affiliation`: an org the user is affiliated with.
#[derive(Deserialize)]
struct LanyardsAffiliation {
    website: Option<String>,
    #[serde(rename = "organizationName")]
    organization_name: Option<String>,
}

/// `link.woosh.linkPage`: a link-in-bio board grouped into labelled collections.
#[derive(Deserialize)]
struct WooshLinkPage {
    #[serde(default)]
    collections: Vec<WooshCollection>,
}

#[derive(Deserialize)]
struct WooshCollection {
    #[serde(default)]
    links: Vec<WooshLink>,
}

#[derive(Deserialize)]
struct WooshLink {
    uri: Option<String>,
    title: Option<String>,
}

/// `blue.linkat.board`: a Linktree-style board of cards.
#[derive(Deserialize)]
struct LinkatBoard {
    #[serde(default)]
    cards: Vec<LinkatCard>,
}

#[derive(Deserialize)]
struct LinkatCard {
    url: Option<String>,
    text: Option<String>,
}

/// The host of a URL, lowercased and without a `www.` prefix or query/path.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// Fetch the atstore.fyi catalog and index it by app domain -> icon URL (served
/// as a blob from atstore's PDS, which is CORS-enabled). Best-effort: any error
/// just yields a smaller/empty index and the graph falls back to badges.
async fn fetch_atstore_index() -> HashMap<String, String> {
    let mut index = HashMap::new();
    let Ok(pds) = resolve_pds(ATSTORE_DID).await else {
        return index;
    };
    let mut cursor: Option<String> = None;
    // The catalog is a few hundred records; cap the paging as a safety net.
    for _ in 0..8 {
        let url = format!(
            "{pds}/xrpc/com.atproto.repo.listRecords?repo={ATSTORE_DID}&collection={ATSTORE_LISTINGS}&limit=100{}",
            cursor
                .as_deref()
                .map(|c| format!("&cursor={c}"))
                .unwrap_or_default()
        );
        let Ok(page) = fetch_json::<ListRecords<AtstoreListing>>(&url).await else {
            break;
        };
        for rec in &page.records {
            let (Some(site), Some(icon)) = (&rec.value.external_url, &rec.value.icon) else {
                continue;
            };
            let Some(cid) = icon
                .blob_ref
                .as_ref()
                .and_then(|r| r.link.clone())
                .or_else(|| icon.cid.clone())
            else {
                continue;
            };
            if let Some(host) = url_host(site) {
                // First listing for a host wins (skip seed/test dupes later).
                index.entry(host).or_insert_with(|| {
                    format!("{pds}/xrpc/com.atproto.sync.getBlob?did={ATSTORE_DID}&cid={cid}")
                });
            }
        }
        match page.cursor {
            Some(c) if !c.is_empty() => cursor = Some(c),
            _ => break,
        }
    }
    index
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

/// Best-effort fetch of the Bluesky profile record for a display name, a
/// CORS-safe avatar URL (served through the account's own PDS blob store), and
/// the bio text (scanned later for prose links). Missing profile is not an
/// error — many atproto accounts have no bsky profile.
async fn fetch_profile(pds: &str, did: &str) -> (Option<String>, Option<String>, Option<String>) {
    let url = format!(
        "{pds}/xrpc/com.atproto.repo.getRecord?repo={did}&collection=app.bsky.actor.profile&rkey=self"
    );
    let Ok(rec) = fetch_json::<ProfileRecord>(&url).await else {
        return (None, None, None);
    };
    let Some(value) = rec.value else {
        return (None, None, None);
    };
    let avatar_cid = value
        .avatar
        .and_then(|a| a.blob_ref.and_then(|r| r.link).or(a.cid));
    let avatar_url =
        avatar_cid.map(|cid| format!("{pds}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}"));
    (value.display_name, avatar_url, value.description)
}

/// GET `url` and report only whether it returned a success status — an existence
/// probe, used for the Bridgy Fed webfinger where we don't need the body.
async fn fetch_ok(url: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(value) = JsFuture::from(window.fetch_with_str(url)).await else {
        return false;
    };
    value
        .dyn_into::<Response>()
        .map(|resp| resp.ok())
        .unwrap_or(false)
}

/// Bridgy Fed's fediverse host for the Bluesky bridge.
const BRIDGY_HOST: &str = "bsky.brid.gy";

/// Whether Bridgy Fed has bridged this account into the fediverse. Bridged
/// (opt-in) accounts resolve at Bridgy's webfinger; others 404. The endpoint is
/// CORS-enabled; a raw DID is never bridge-addressable by handle.
async fn is_bridged(handle: &str) -> bool {
    if handle.starts_with("did:") {
        return false;
    }
    let resource = format!("acct:{handle}@{BRIDGY_HOST}");
    let url = format!("https://{BRIDGY_HOST}/.well-known/webfinger?resource={resource}");
    fetch_ok(&url).await
}

/// A `listRecords` URL for one collection (first page, generous limit).
fn list_url(pds: &str, did: &str, collection: &str) -> String {
    format!("{pds}/xrpc/com.atproto.repo.listRecords?repo={did}&collection={collection}&limit=100")
}

/// Harvest external (non-atproto) profile links from the repo's link-aggregator
/// records — Sifa external accounts, Lanyards affiliations, Woosh/Linkat boards —
/// plus any bare URLs in the Bluesky bio, and turn them into "Elsewhere" leaves.
/// Best-effort: a missing or malformed collection just contributes nothing.
async fn fetch_external_links(
    pds: &str,
    did: &str,
    collections: &[String],
    description: Option<&str>,
    detected: &[Platform],
) -> Vec<Platform> {
    // `(url, label)` pairs in priority order: structured/labelled sources first
    // so a bio URL never overrides a nicely-captioned link to the same place.
    let mut candidates: Vec<(String, Option<String>)> = Vec::new();
    let present = |c: &str| collections.iter().any(|x| x == c);

    // Sifa: one record per external account, already labelled and typed.
    if present("id.sifa.profile.externalAccount")
        && let Ok(page) = fetch_json::<ListRecords<SifaExternalAccount>>(&list_url(
            pds,
            did,
            "id.sifa.profile.externalAccount",
        ))
        .await
    {
        for rec in page.records {
            if let Some(url) = rec.value.url {
                candidates.push((url, rec.value.label));
            }
        }
    }

    // Lanyards: an affiliation's website, captioned by the organization name.
    if present("app.lanyards.actor.biography.affiliation")
        && let Ok(page) = fetch_json::<ListRecords<LanyardsAffiliation>>(&list_url(
            pds,
            did,
            "app.lanyards.actor.biography.affiliation",
        ))
        .await
    {
        for rec in page.records {
            if let Some(url) = rec.value.website {
                candidates.push((url, rec.value.organization_name));
            }
        }
    }

    // Woosh: link boards grouped into labelled collections of `{uri, title}`.
    if present("link.woosh.linkPage")
        && let Ok(page) =
            fetch_json::<ListRecords<WooshLinkPage>>(&list_url(pds, did, "link.woosh.linkPage"))
                .await
    {
        for rec in page.records {
            for group in rec.value.collections {
                for link in group.links {
                    if let Some(uri) = link.uri {
                        candidates.push((uri, link.title));
                    }
                }
            }
        }
    }

    // Linkat: Linktree-style cards of `{url, text}` (heading cards have no url).
    if present("blue.linkat.board")
        && let Ok(page) =
            fetch_json::<ListRecords<LinkatBoard>>(&list_url(pds, did, "blue.linkat.board")).await
    {
        for rec in page.records {
            for card in rec.value.cards {
                if let Some(url) = card.url.filter(|u| !u.is_empty()) {
                    candidates.push((url, card.text));
                }
            }
        }
    }

    // Indirect: plain URLs written into the Bluesky bio.
    if let Some(desc) = description {
        for url in crate::atproto::extract_urls(desc) {
            candidates.push((url, None));
        }
    }

    external_platforms(&candidates, detected)
}

/// Assemble the graph: the account at the center, intermediate category hubs
/// (Connect / Create / …), and each detected platform as a leaf under its hub.
fn build_graph(
    display_name: Option<String>,
    avatar_url: Option<String>,
    handle: &str,
    did: &str,
    platforms: Vec<Platform>,
    atstore: &HashMap<String, String>,
) -> GraphData {
    // Node ids are namespaced with a control char so the center and category
    // hubs can never collide with a (user-controlled) platform name.
    const CENTER_ID: &str = "\u{1}center";
    let cat_id = |c: &str| format!("\u{1}{c}");

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
        hub: false,
    }];
    let mut links = Vec::new();

    // Intermediate category hubs: one per category that has platforms, each
    // linked to the center and drawn with a translucent colored halo.
    for &cat in CATEGORY_ORDER {
        if !platforms.iter().any(|p| p.category == cat) {
            continue;
        }
        let (color, icon) = category_meta(cat);
        links.push(GraphLink {
            source: CENTER_ID.to_string(),
            target: cat_id(cat),
        });
        nodes.push(GraphNode {
            id: cat_id(cat),
            desc: cat.to_string(),
            icon,
            color: Some(color.to_string()),
            opacity: None,
            url: None,
            center: false,
            hub: true,
        });
    }

    // Platform leaves, each linked to its category hub. Icon precedence: a
    // hand-picked bundled logo, else the atstore.fyi registry logo (by domain),
    // else the generated badge.
    for p in platforms {
        let icon = match &p.icon {
            Icon::Bundled(_) => p.icon.resolve(),
            Icon::Badge(_) => atstore
                .get(&p.domain)
                .cloned()
                .unwrap_or_else(|| p.icon.resolve()),
        };
        links.push(GraphLink {
            source: cat_id(p.category),
            target: p.name.clone(),
        });
        nodes.push(GraphNode {
            id: p.name.clone(),
            desc: format!("{}\nProfile", p.name),
            icon,
            color: None,
            opacity: None,
            url: Some(p.profile_url),
            center: false,
            hub: false,
        });
    }

    // Atproto badges + the avatar are square textures; render them as circles.
    // Start collapsed: the graph shows hubs first, then leaves on tap.
    GraphData {
        nodes,
        links,
        circular_icons: true,
        collapsible: true,
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
    let (display_name, avatar_url, description) = fetch_profile(&pds, &did).await;
    let mut platforms = detect_platforms(&repo.collections, &handle, &did);

    // Only pay for the atstore.fyi catalog when a detected atproto app lacks a
    // bundled logo — external links are never in that registry.
    let need_atstore = platforms.iter().any(|p| matches!(p.icon, Icon::Badge(_)));

    // A fediverse presence via Bridgy Fed (opt-in bridge) shows as a Bridgy Fed
    // leaf. Added before external links so those dedupe against it.
    if is_bridged(&handle).await {
        platforms.push(bridged_mastodon(&handle));
    }

    // Enrich with any off-atproto profile links the account has published in
    // link-aggregator records or its bio.
    let externals = fetch_external_links(
        &pds,
        &did,
        &repo.collections,
        description.as_deref(),
        &platforms,
    )
    .await;
    platforms.extend(externals);

    let atstore = if need_atstore {
        fetch_atstore_index().await
    } else {
        HashMap::new()
    };

    Ok(build_graph(
        display_name,
        avatar_url,
        &handle,
        &did,
        platforms,
        &atstore,
    ))
}
