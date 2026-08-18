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

/// How far back to replay the firehose when a tag is first asked for.
///
/// Jetstream takes a `cursor` and replays from it as fast as it can, which is
/// what makes a hashtag usable at all. Measured on the live firehose: posts
/// arrive at about 52 a second and one in seven carries a picture, but only
/// about one in a thousand carries any particular tag. Waiting for `#art` to
/// come past live took 12 seconds; for `#caturday`, nothing in 30. The saver
/// gives up after 20 and draws colour bars, which is what makes a hashtag look
/// broken. Replaying five minutes of backlog found the first `#art` picture in
/// one second, after 81 KB.
const REPLAY_WINDOW_SECONDS: f64 = 300.0;

/// Stop replaying once this many pictures are queued: one to show now and one
/// to show next. The rest arrive live.
///
/// Asking for more is what makes a quiet tag expensive. Replaying for `#cat`,
/// the first picture cost 4.8 MB and the second 16 MB, because the cost is
/// the whole firehose whatever you are looking for.
const REPLAY_ENOUGH: usize = 2;

/// ...or once the replay has cost this much, which is what stops a tag nobody
/// posts under from reading the backlog to the end.
///
/// A busy tag never reaches this: `#art` had its two in about 100 KB. A quiet
/// one spends it and takes what it found, which for `#cat` is one picture.
/// Five minutes of backlog in full would be 14 MB.
const REPLAY_MAX_BYTES: usize = 6_000_000;

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

    /// The panel's two fields for this source: which kind it is, and the name
    /// inside it.
    pub fn parts(&self) -> (&'static str, String) {
        match self {
            Source::None => ("none", String::new()),
            Source::Account(a) => ("account", a.clone()),
            Source::Tag(t) => ("tag", t.clone()),
        }
    }

    /// Build a source from the panel's two fields.
    ///
    /// The name is taken as typed: nobody should have to know that a hashtag
    /// has to reach the query string as `%23`. A leading `@` or `#` is
    /// stripped rather than doubled, so pasting `#art` works too.
    pub fn from_parts(kind: &str, name: &str) -> Self {
        let name = name.trim().trim_start_matches(['@', '#']).trim();
        if name.is_empty() {
            return Source::None;
        }
        match kind {
            "account" => Source::Account(name.to_string()),
            "tag" => Source::Tag(name.to_ascii_lowercase()),
            _ => Source::None,
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
    /// What to call the picture: the alt text if the poster wrote any, and
    /// failing that the words of the post it was attached to.
    alt: Option<String>,
}

impl BlobRef {
    /// Fall back to `caption` where the poster left the alt text empty, which
    /// most do. Savers that write the picture's name under it, `photopile`
    /// above all, otherwise have nothing to write.
    fn captioned(mut self, caption: Option<String>) -> Self {
        if self.alt.is_none() {
            self.alt = caption;
        }
        self
    }
}

/// The longest caption worth drawing under a picture.
const CAPTION_MAX_CHARS: usize = 72;

/// A one-line caption from a post's text.
///
/// The first non-empty line, cut at a word boundary: a saver draws this in one
/// line in a bitmap font, and a whole thread under a photograph is not a
/// caption.
fn caption_from_text(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.chars().count() <= CAPTION_MAX_CHARS {
        return Some(line.to_string());
    }
    let cut: String = line.chars().take(CAPTION_MAX_CHARS).collect();
    let end = cut.rfind(' ').unwrap_or(cut.len());
    Some(format!("{}\u{2026}", cut[..end].trim_end()))
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
    /// The words of the post, which caption a picture with no alt text.
    #[serde(default)]
    text: String,
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
            // A post with neither alt text nor words still came from
            // somewhere, and that is better than "(untitled)".
            title: blob.alt.or_else(|| source.describe()),
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
        .flat_map(|r| {
            let caption = caption_from_text(&r.value.text);
            r.value
                .embed
                .iter()
                .flat_map(|e| e.blobs(&did))
                .map(|b| b.captioned(caption.clone()))
                .collect::<Vec<_>>()
        })
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
///
/// The subscription starts in the recent past: see [`REPLAY_WINDOW_SECONDS`]
/// for why a live-only firehose makes a hashtag look broken.
fn ensure_tag_subscription(tag: &str) {
    let already = TAG_SOCKET.with(|s| {
        s.borrow()
            .as_ref()
            .is_some_and(|(t, sock)| t == tag && sock.ready_state() <= web_sys::WebSocket::OPEN)
    });
    if already {
        return;
    }
    // A different tag than the one queued: those pictures belong to the old
    // one, and showing them under the new tag would be a lie.
    TAG_QUEUE.with(|q| q.borrow_mut().clear());
    subscribe(tag.to_string(), Replay::Yes);
}

/// Whether to open the firehose in the past or at the live edge.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Replay {
    /// Start [`REPLAY_WINDOW_SECONDS`] ago, so there is something to show now.
    Yes,
    /// Start where the firehose is, which costs nothing to keep open.
    No,
}

/// Open the firehose for `tag`.
///
/// A replaying socket swaps itself for a live one as soon as it has found
/// enough pictures, or spent enough trying. Reconnecting rather than reading
/// on is the point: the whole backlog is a hundred times the size of the part
/// of it worth having.
fn subscribe(tag: String, replay: Replay) {
    let url = match replay {
        Replay::Yes => {
            let from = (js_sys::Date::now() - REPLAY_WINDOW_SECONDS * 1000.0) * 1000.0;
            format!("{JETSTREAM}&cursor={from:.0}")
        }
        Replay::No => JETSTREAM.to_string(),
    };
    let Ok(socket) = web_sys::WebSocket::new(&url) else {
        log::warn!("screensaver images: could not open the firehose");
        return;
    };

    let spent = std::cell::Cell::new(0usize);
    let swapped = std::cell::Cell::new(false);
    let wanted = tag.clone();
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
        if replay == Replay::No || swapped.get() {
            return;
        }
        spent.set(spent.get() + text.len());
        let queued = TAG_QUEUE.with(|q| q.borrow().len());
        if queued >= REPLAY_ENOUGH || spent.get() >= REPLAY_MAX_BYTES {
            // Installing the live socket closes this one, so this closure
            // stops being called; the flag guards any frames already behind
            // it in the event loop.
            swapped.set(true);
            subscribe(wanted.clone(), Replay::No);
        }
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    TAG_SOCKET.with(|s| {
        // Dropping the previous socket closes it, which is what we want when
        // the saver switches tags, and what retires a replay once it has done
        // its job.
        if let Some((_, old)) = s.borrow_mut().take() {
            let _ = old.close();
        }
        *s.borrow_mut() = Some((tag, socket));
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
    let caption = caption_from_text(&record.text);
    record
        .embed
        .as_ref()
        .map(|e| e.blobs(&did))
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.captioned(caption.clone()))
        .collect()
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
        .map_err(|e| format!("network error fetching {url}: {e:?}"))?
        .dyn_into()
        .map_err(|e| format!("unexpected fetch result: {e:?}"))?;
    if !response.ok() {
        return Err(format!("HTTP {} from {url}", response.status()));
    }
    let blob = JsFuture::from(response.blob().map_err(|e| format!("no blob: {e:?}"))?)
        .await
        .map_err(|e| format!("could not read the blob: {e:?}"))?;
    let bitmap: ImageBitmap = JsFuture::from(
        window
            .create_image_bitmap_with_blob(
                &blob.dyn_into().map_err(|e| format!("not a blob: {e:?}"))?,
            )
            .map_err(|e| format!("could not decode the image: {e:?}"))?,
    )
    .await
    .map_err(|e| format!("could not decode the image: {e:?}"))?
    .dyn_into()
    .map_err(|e| format!("decoded to something that is not an image: {e:?}"))?;

    let (bw, bh) = (bitmap.width() as f64, bitmap.height() as f64);
    if bw < 1.0 || bh < 1.0 {
        return Err("image has no pixels".to_string());
    }
    let scale = (max_w as f64 / bw).min(max_h as f64 / bh).min(1.0);
    let (w, h) = (((bw * scale) as u32).max(1), ((bh * scale) as u32).max(1));

    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|e| format!("could not make a canvas: {e:?}"))?
        .dyn_into()
        .map_err(|e| format!("not a canvas: {e:?}"))?;
    canvas.set_width(w);
    canvas.set_height(h);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|e| format!("no 2d context: {e:?}"))?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|e| format!("not a 2d context: {e:?}"))?;
    ctx.draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, 0.0, 0.0, w as f64, h as f64)
        .map_err(|e| format!("could not draw the image: {e:?}"))?;
    bitmap.close();

    let data = ctx
        .get_image_data(0.0, 0.0, w as f64, h as f64)
        .map_err(|e| format!("could not read the canvas back (tainted?): {e:?}"))?
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

    /// A picture with no alt text is captioned with the words of its post.
    #[test]
    fn a_caption_is_one_line_of_the_post() {
        assert_eq!(
            caption_from_text("  A photograph of a bridge.  "),
            Some("A photograph of a bridge.".to_string())
        );
        // The first line that says anything, not the first line.
        assert_eq!(
            caption_from_text("\n\n  second line has the words\nand a third"),
            Some("second line has the words".to_string())
        );
        assert_eq!(caption_from_text("   \n  "), None);
    }

    /// A whole thread is not a caption; it is cut at a word.
    #[test]
    fn a_long_caption_is_cut_at_a_word() {
        let long = "the quick brown fox jumps over the lazy dog and keeps on jumping until it is tired out";
        let cut = caption_from_text(long).expect("a caption");
        assert!(cut.ends_with('\u{2026}'), "not elided: {cut}");
        assert!(
            cut.chars().count() <= CAPTION_MAX_CHARS + 1,
            "too long: {cut}"
        );
        assert!(long.starts_with(cut.trim_end_matches('\u{2026}').trim_end()));
        // Cut between words, so no half a word is left on screen.
        assert!(!cut.trim_end_matches('\u{2026}').ends_with(' '));
    }

    /// The panel hands over what was typed, and nobody types `%23`.
    #[test]
    fn the_panel_fields_build_a_source_without_escapes() {
        assert_eq!(Source::from_parts("tag", "art"), Source::Tag("art".into()));
        assert_eq!(
            Source::from_parts("account", "overby.me"),
            Source::Account("overby.me".into())
        );
        // Pasting the sigil in is not an error, and does not double it.
        assert_eq!(Source::from_parts("tag", "#Art"), Source::Tag("art".into()));
        assert_eq!(
            Source::from_parts("account", " @overby.me "),
            Source::Account("overby.me".into())
        );
        // A kind with nothing in it is no source at all.
        assert_eq!(Source::from_parts("tag", "  "), Source::None);
        assert_eq!(Source::from_parts("none", "art"), Source::None);
    }

    /// What the panel shows is what the URL says, both ways round.
    #[test]
    fn parts_and_from_parts_round_trip() {
        for source in [
            Source::None,
            Source::Account("overby.me".into()),
            Source::Tag("caturday".into()),
        ] {
            let (kind, name) = source.parts();
            assert_eq!(Source::from_parts(kind, &name), source);
        }
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
