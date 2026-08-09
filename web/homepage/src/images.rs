//! Where the screensavers' pictures come from.
//!
//! About thirty upstream hacks work on an image: they melt it, zoom it, ripple
//! it, cut it into sliding blocks. On a desktop xscreensaver grabs the screen
//! or a file from your pictures directory. In a browser there is no screen to
//! grab, so a saver takes its pictures from atproto, chosen with an `images`
//! query parameter:
//!
//! - `/screensaver/decayscreen?images=@overby.me` melts that account's own
//!   photographs, newest first;
//! - `/screensaver/decayscreen?images=%23caturday` melts whatever anyone posts
//!   under that hashtag, live, as they post it;
//! - with no parameter the saver gets colour bars, which is what upstream falls
//!   back to when it cannot find an image either.
//!
//! # Getting at the bytes
//!
//! The obvious source, `cdn.bsky.app`, is unusable: it serves images without
//! `access-control-allow-origin`, so the canvas they are drawn into is tainted
//! and cannot be read back. Every route here goes to the *account's own PDS*
//! instead, which does send `access-control-allow-origin: *`:
//!
//! ```text
//! handle -> resolveHandle -> DID -> DID doc -> PDS
//!        -> com.atproto.repo.listRecords (app.bsky.feed.post)
//!        -> embed.images[].image.ref.$link
//!        -> com.atproto.sync.getBlob
//! ```
//!
//! No appview and no authentication anywhere in that chain.
//!
//! Hashtags cannot work the same way: there is no per-tag index in a repo, and
//! the appview's `searchPosts` refuses unauthenticated callers. So tags come
//! off the public Jetstream instead, a WebSocket firehose of records as they
//! are written. That means a tag feed starts empty and fills as people post,
//! which for a screensaver is a feature: the picture on the wall is genuinely
//! live. The blob reference is right there in the record, so it still resolves
//! through the author's own PDS.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap, MessageEvent, Response};
use xscreensaver::runtime::{XImage, color::rgb};

use crate::atproto_web::{fetch_json, resolve_did, resolve_pds};

/// The public Jetstream instance. Records only, no auth, no CORS to negotiate
/// (it is a WebSocket).
const JETSTREAM: &str =
    "wss://jetstream2.us-east.bsky.network/subscribe?wantedCollections=app.bsky.feed.post";

/// Skip anything bigger than this. Blobs are the originals the poster uploaded,
/// with no resized variant available over a CORS-clean route, and a saver that
/// changes picture every couple of minutes should not pull down 8 MB to do it.
const MAX_BLOB_BYTES: u64 = 3_000_000;

/// How many tagged pictures to keep queued from the firehose. Small: they are
/// only references until we fetch them, but a stale one is a dull one.
const TAG_QUEUE_MAX: usize = 32;

/// Where a saver's pictures come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// One account's own posts.
    Account(String),
    /// Posts carrying a hashtag, live.
    Tag(String),
    /// Nothing configured: the runtime draws colour bars.
    None,
}

impl Source {
    /// Read the `images` parameter out of a saver's query string.
    ///
    /// `@handle`, a bare handle, or a `did:` is an account; `#tag` (or `%23tag`
    /// once decoded) is a hashtag.
    pub fn from_query(query: &str) -> Self {
        let Ok(params) = web_sys::UrlSearchParams::new_with_str(query) else {
            return Source::None;
        };
        let Some(raw) = params.get("images") else {
            return Source::None;
        };
        Self::parse(&raw)
    }

    fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Some(tag) = raw.strip_prefix('#') {
            let tag = tag.trim().to_ascii_lowercase();
            if tag.is_empty() {
                return Source::None;
            }
            return Source::Tag(tag);
        }
        let account = raw.trim_start_matches('@').trim();
        if account.is_empty() {
            Source::None
        } else {
            Source::Account(account.to_string())
        }
    }

    /// The value to write back into the `images` query parameter, so a
    /// configured saver stays shareable after the panel rewrites the URL.
    pub fn as_param(&self) -> Option<String> {
        match self {
            Source::Account(a) => Some(format!("%40{a}")),
            Source::Tag(t) => Some(format!("%23{t}")),
            Source::None => None,
        }
    }

    /// What to show in the panel.
    pub fn describe(&self) -> Option<String> {
        match self {
            Source::Account(a) => Some(format!("@{a}")),
            Source::Tag(t) => Some(format!("#{t}")),
            Source::None => None,
        }
    }
}

/// A blob to fetch: which repo holds it and under what CID.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BlobRef {
    did: String,
    cid: String,
    /// The post's alt text, used as the caption.
    alt: Option<String>,
}

thread_local! {
    /// Posts already harvested for an account, so browsing pictures from one
    /// person costs one repo listing rather than one per picture.
    // Not `const`: `HashMap::new` is not a const fn (its hasher is seeded).
    static ACCOUNT_IMAGES: RefCell<HashMap<String, Vec<BlobRef>>> =
        RefCell::new(HashMap::new());
    /// Blob references seen on the firehose, oldest first.
    static TAG_QUEUE: RefCell<VecDeque<BlobRef>> = const { RefCell::new(VecDeque::new()) };
    /// The tag we are currently subscribed to, and its socket. Kept so that
    /// switching savers does not open a second firehose.
    static TAG_SOCKET: RefCell<Option<(String, web_sys::WebSocket)>> = const { RefCell::new(None) };
    /// DID to PDS, so a busy hashtag does not re-resolve the same authors.
    static PDS_CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

// --- the shapes we read out of atproto JSON ---

#[derive(Deserialize)]
struct ListRecords {
    #[serde(default)]
    records: Vec<Record>,
}

#[derive(Deserialize)]
struct Record {
    value: PostValue,
}

#[derive(Deserialize)]
struct PostValue {
    embed: Option<Embed>,
}

/// `app.bsky.embed.images` directly, or wrapped in a `recordWithMedia`.
#[derive(Deserialize)]
struct Embed {
    #[serde(default)]
    images: Vec<EmbedImage>,
    media: Option<Box<Embed>>,
}

#[derive(Deserialize)]
struct EmbedImage {
    image: BlobLink,
    #[serde(default)]
    alt: String,
}

#[derive(Deserialize)]
struct BlobLink {
    #[serde(rename = "ref")]
    reference: BlobCid,
    #[serde(default)]
    size: u64,
    /// Optional in the type, not because atproto omits it, but because one
    /// malformed record must not fail the whole listing it arrived in.
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Deserialize)]
struct BlobCid {
    #[serde(rename = "$link")]
    link: String,
}

impl Embed {
    /// Every usable image in this embed, including one nested behind a
    /// quote-post's media.
    fn blobs(&self, did: &str) -> Vec<BlobRef> {
        let mut out: Vec<BlobRef> = self
            .images
            .iter()
            .filter(|i| i.image.size <= MAX_BLOB_BYTES || i.image.size == 0)
            .filter(|i| {
                i.image
                    .mime_type
                    .as_ref()
                    .is_none_or(|m| m.starts_with("image/"))
            })
            .map(|i| BlobRef {
                did: did.to_string(),
                cid: i.image.reference.link.clone(),
                alt: (!i.alt.trim().is_empty()).then(|| i.alt.trim().to_string()),
            })
            .collect();
        if let Some(media) = &self.media {
            out.extend(media.blobs(did));
        }
        out
    }
}

/// A picture, decoded and ready for a saver.
pub struct Picture {
    pub image: XImage,
    pub title: Option<String>,
}

/// Fetch and decode the next picture for `source`, at most `max_w` by `max_h`.
///
/// Returns `None` when there is nothing to show yet, which for a hashtag simply
/// means nobody has posted one since we started listening. The caller asks
/// again; the runtime falls back to colour bars if the wait gets silly.
pub async fn next_picture(source: &Source, max_w: i32, max_h: i32) -> Option<Picture> {
    let blob = match source {
        Source::None => return None,
        Source::Account(account) => next_account_blob(account).await?,
        Source::Tag(tag) => {
            ensure_tag_subscription(tag);
            TAG_QUEUE.with(|q| q.borrow_mut().pop_front())?
        }
    };

    let pds = pds_for(&blob.did).await?;
    let url = format!(
        "{pds}/xrpc/com.atproto.sync.getBlob?did={}&cid={}",
        blob.did, blob.cid
    );
    match decode(&url, max_w, max_h).await {
        Ok(image) => Some(Picture {
            image,
            title: blob.alt,
        }),
        Err(e) => {
            log::warn!("screensaver images: {e}");
            None
        }
    }
}

/// One random image from an account, harvesting its repo on first use.
async fn next_account_blob(account: &str) -> Option<BlobRef> {
    let cached = ACCOUNT_IMAGES.with(|c| c.borrow().get(account).cloned());
    let blobs = match cached {
        Some(blobs) => blobs,
        None => {
            let blobs = harvest_account(account).await.unwrap_or_default();
            ACCOUNT_IMAGES.with(|c| {
                c.borrow_mut().insert(account.to_string(), blobs.clone());
            });
            blobs
        }
    };
    if blobs.is_empty() {
        return None;
    }
    let i = (js_sys::Math::random() * blobs.len() as f64) as usize;
    blobs.get(i.min(blobs.len() - 1)).cloned()
}

/// Walk an account's posts straight out of its repo. No appview involved, so
/// this sees exactly what the account wrote.
async fn harvest_account(account: &str) -> Result<Vec<BlobRef>, String> {
    let did = resolve_did(account).await?;
    let pds = resolve_pds(&did).await?;
    PDS_CACHE.with(|c| c.borrow_mut().insert(did.clone(), pds.clone()));

    let url = format!(
        "{pds}/xrpc/com.atproto.repo.listRecords\
         ?repo={did}&collection=app.bsky.feed.post&limit=100"
    );
    let page: ListRecords = fetch_json(&url).await?;
    let blobs: Vec<BlobRef> = page
        .records
        .iter()
        .filter_map(|r| r.value.embed.as_ref())
        .flat_map(|e| e.blobs(&did))
        .collect();
    Ok(blobs)
}

async fn pds_for(did: &str) -> Option<String> {
    if let Some(pds) = PDS_CACHE.with(|c| c.borrow().get(did).cloned()) {
        return Some(pds);
    }
    let pds = resolve_pds(did).await.ok()?;
    PDS_CACHE.with(|c| c.borrow_mut().insert(did.to_string(), pds.clone()));
    Some(pds)
}

/// Subscribe to the firehose for a tag, if we are not already on it.
fn ensure_tag_subscription(tag: &str) {
    let already = TAG_SOCKET.with(|s| {
        s.borrow()
            .as_ref()
            .is_some_and(|(t, sock)| t == tag && sock.ready_state() <= web_sys::WebSocket::OPEN)
    });
    if already {
        return;
    }

    let Ok(socket) = web_sys::WebSocket::new(JETSTREAM) else {
        log::warn!("screensaver images: could not open the firehose");
        return;
    };

    let wanted = tag.to_string();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let Some(text) = ev.data().as_string() else {
            return;
        };
        for blob in tagged_images(&text, &wanted) {
            TAG_QUEUE.with(|q| {
                let mut q = q.borrow_mut();
                if q.len() >= TAG_QUEUE_MAX {
                    q.pop_front();
                }
                q.push_back(blob);
            });
        }
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    TAG_SOCKET.with(|s| {
        // Dropping the previous socket closes it, which is what we want when
        // the saver switches tags.
        if let Some((_, old)) = s.borrow_mut().take() {
            let _ = old.close();
        }
        *s.borrow_mut() = Some((tag.to_string(), socket));
    });
}

/// Pull the image blobs out of one Jetstream frame, if it is a new post
/// carrying `tag`.
///
/// Separate from the socket plumbing so it can be tested; the shape is
/// `{did, commit: {operation, collection, record}}`.
fn tagged_images(frame: &str, tag: &str) -> Vec<BlobRef> {
    let Ok(value) = js_sys::JSON::parse(frame) else {
        return Vec::new();
    };
    let Ok(event) = serde_wasm_bindgen::from_value::<JetstreamEvent>(value) else {
        return Vec::new();
    };
    let (Some(did), Some(commit)) = (event.did, event.commit) else {
        return Vec::new();
    };
    if commit.operation.as_deref() != Some("create") {
        return Vec::new();
    }
    let Some(record) = commit.record else {
        return Vec::new();
    };
    if !record.mentions_tag(tag) {
        return Vec::new();
    }
    record
        .embed
        .as_ref()
        .map(|e| e.blobs(&did))
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct JetstreamEvent {
    did: Option<String>,
    commit: Option<JetstreamCommit>,
}

#[derive(Deserialize)]
struct JetstreamCommit {
    operation: Option<String>,
    record: Option<PostRecord>,
}

#[derive(Deserialize)]
struct PostRecord {
    #[serde(default)]
    text: String,
    #[serde(default)]
    facets: Vec<Facet>,
    embed: Option<Embed>,
}

#[derive(Deserialize)]
struct Facet {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    #[serde(rename = "$type")]
    kind: Option<String>,
    tag: Option<String>,
}

impl PostRecord {
    /// A post carries a tag if it is a real richtext tag facet, or failing
    /// that if the text simply contains `#tag`. Posters are inconsistent about
    /// whether their client makes facets, so both count.
    fn mentions_tag(&self, tag: &str) -> bool {
        let faceted = self.facets.iter().flat_map(|f| &f.features).any(|f| {
            f.kind.as_deref() == Some("app.bsky.richtext.facet#tag")
                && f.tag
                    .as_ref()
                    .is_some_and(|t| t.to_ascii_lowercase() == tag)
        });
        faceted
            || self
                .text
                .to_ascii_lowercase()
                .split(|c: char| !(c.is_alphanumeric() || c == '#' || c == '_'))
                .any(|w| w.strip_prefix('#') == Some(tag))
    }
}

/// Fetch an image and decode it into a framebuffer.
///
/// `createImageBitmap` on a blob we already fetched keeps this same-origin as
/// far as the canvas is concerned, so `getImageData` works. Scaling happens on
/// the way in: the saver's window is at most 1600 pixels across and the source
/// is whatever a phone camera produced.
async fn decode(url: &str, max_w: i32, max_h: i32) -> Result<XImage, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|_| format!("network error fetching {url}"))?
        .dyn_into()
        .map_err(|_| "unexpected fetch result".to_string())?;
    if !response.ok() {
        return Err(format!("HTTP {} from {url}", response.status()));
    }
    let blob = JsFuture::from(response.blob().map_err(|_| "no blob")?)
        .await
        .map_err(|_| "could not read the blob".to_string())?;
    let bitmap: ImageBitmap = JsFuture::from(
        window
            .create_image_bitmap_with_blob(&blob.dyn_into().map_err(|_| "not a blob")?)
            .map_err(|_| "could not decode the image".to_string())?,
    )
    .await
    .map_err(|_| "could not decode the image".to_string())?
    .dyn_into()
    .map_err(|_| "decoded to something that is not an image".to_string())?;

    let (bw, bh) = (bitmap.width() as f64, bitmap.height() as f64);
    if bw < 1.0 || bh < 1.0 {
        return Err("image has no pixels".to_string());
    }
    let scale = (max_w as f64 / bw).min(max_h as f64 / bh).min(1.0);
    let (w, h) = (((bw * scale) as u32).max(1), ((bh * scale) as u32).max(1));

    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|_| "could not make a canvas")?
        .dyn_into()
        .map_err(|_| "not a canvas".to_string())?;
    canvas.set_width(w);
    canvas.set_height(h);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|_| "no 2d context")?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|_| "not a 2d context".to_string())?;
    ctx.draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, 0.0, 0.0, w as f64, h as f64)
        .map_err(|_| "could not draw the image".to_string())?;
    bitmap.close();

    let data = ctx
        .get_image_data(0.0, 0.0, w as f64, h as f64)
        .map_err(|_| "could not read the canvas back (tainted?)".to_string())?
        .data();

    let mut image = XImage::new(w as i32, h as i32);
    for (i, px) in image.pixels_mut().iter_mut().enumerate() {
        let o = i * 4;
        *px = rgb(data[o], data[o + 1], data[o + 2]);
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_images_parameter() {
        assert_eq!(
            Source::parse("@overby.me"),
            Source::Account("overby.me".into())
        );
        assert_eq!(
            Source::parse("overby.me"),
            Source::Account("overby.me".into())
        );
        assert_eq!(
            Source::parse("did:plc:abc"),
            Source::Account("did:plc:abc".into())
        );
        assert_eq!(Source::parse("#Caturday"), Source::Tag("caturday".into()));
        assert_eq!(Source::parse("  #art "), Source::Tag("art".into()));
        assert_eq!(Source::parse(""), Source::None);
        assert_eq!(Source::parse("#"), Source::None);
        assert_eq!(Source::parse("@"), Source::None);
    }

    #[test]
    fn round_trips_through_the_query_parameter() {
        for original in [
            Source::Account("overby.me".into()),
            Source::Tag("caturday".into()),
        ] {
            let param = original.as_param().expect("has a param");
            // Percent-decoding is the browser's job; do the two escapes we emit.
            let decoded = param.replace("%40", "@").replace("%23", "#");
            assert_eq!(Source::parse(&decoded), original);
        }
        assert_eq!(Source::None.as_param(), None);
    }

    #[test]
    fn describes_itself_for_the_panel() {
        assert_eq!(
            Source::Account("a.b".into()).describe().as_deref(),
            Some("@a.b")
        );
        assert_eq!(
            Source::Tag("art".into()).describe().as_deref(),
            Some("#art")
        );
        assert_eq!(Source::None.describe(), None);
    }

    #[test]
    fn a_tag_matches_a_facet_or_bare_text() {
        let faceted = PostRecord {
            text: "no tag in the words".into(),
            facets: vec![Facet {
                features: vec![Feature {
                    kind: Some("app.bsky.richtext.facet#tag".into()),
                    tag: Some("Art".into()),
                }],
            }],
            embed: None,
        };
        assert!(faceted.mentions_tag("art"), "facet tag missed");

        let textual = PostRecord {
            text: "a drawing #Art of a heron".into(),
            facets: vec![],
            embed: None,
        };
        assert!(textual.mentions_tag("art"), "text tag missed");

        let unrelated = PostRecord {
            text: "#artichoke season".into(),
            facets: vec![],
            embed: None,
        };
        assert!(!unrelated.mentions_tag("art"), "matched a prefix");

        let mention_only = PostRecord {
            text: "art without a hash".into(),
            facets: vec![],
            embed: None,
        };
        assert!(!mention_only.mentions_tag("art"), "matched a bare word");
    }
}
