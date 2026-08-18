//! Where the screensavers' words come from.
//!
//! Ten or so upstream hacks read text and put it on the screen: they glide it
//! in from the edges, rain it down a terminal, scroll it past a starfield. On
//! a desktop each opens a pipe to `xscreensaver-text`, which prints a file, a
//! URL, the output of a program, or the date. In a browser there is no pipe,
//! so a saver takes its words from here, chosen with a `text` query
//! parameter:
//!
//! - `/screensaver/starwars` with nothing set scrolls a poem, which is the
//!   nearest thing to `fortune(6)` still answering;
//! - `/screensaver/phosphor?text=@overby.me` types out that account's posts;
//! - `/screensaver/xmatrix?text=%23caturday` rains whatever anyone posts under
//!   that hashtag, live, as they post it;
//! - `/screensaver/apple2?text=https://example.com/notes.txt` reads any URL you
//!   name.
//!
//! This mirrors [`crate::images`] deliberately: same shapes, same `@handle`
//! and `#tag` spellings, the same route through an account's own PDS.
//!
//! # Why a poem, and why that one
//!
//! `fortune` itself has no surviving public API. The two that existed
//! (`yerkee.com/api/fortune`, `api.fortunecookieapi.com`) no longer answer,
//! and the obvious quote services either send no
//! `access-control-allow-origin` (`zenquotes.io`) or return a single line,
//! which is thin material for a saver that wants a stream.
//!
//! `poetrydb.org` sends `access-control-allow-origin: *`, needs no key, and
//! returns a whole poem with its title and author: a few hundred bytes of real
//! text with an attribution, which is the shape a fortune actually has.
//!
//! # Any URL, within what a browser will allow
//!
//! `?text=<url>` fetches whatever you point it at, but the browser will only
//! let the page read the response if that server sends
//! `access-control-allow-origin`. That is not something this page can grant
//! itself; it belongs to the server being fetched. A URL that does not send it
//! fails, the saver falls back to its compiled-in passage after twenty
//! seconds, and a line goes to the console saying so. No proxy is used to get
//! around this, because the sources that matter here already allow it.
//!
//! What comes back is normalised by content type: plain text is used as it is,
//! HTML is stripped of its tags, and JSON is flattened to its string leaves.
//! That last rule is why one code path serves every quote API worth naming:
//! poetrydb's `lines`, adviceslip's `advice`, chucknorris's `value` are all
//! just strings somewhere in the document.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{MessageEvent, Response};

use crate::atproto_web::{fetch_json, resolve_did, resolve_pds};

/// The default source: a random poem. See the note above on why this one.
const FORTUNE_URL: &str = "https://poetrydb.org/random";

/// The public Jetstream instance, as [`crate::images`] uses.
const JETSTREAM: &str =
    "wss://jetstream2.us-east.bsky.network/subscribe?wantedCollections=app.bsky.feed.post";

/// How many live posts to keep before dropping the oldest.
const TAG_QUEUE_MAX: usize = 64;

/// Where a saver's words come from.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Source {
    /// Nothing configured: a poem, which is the default.
    #[default]
    Fortune,
    /// One account's own posts.
    Account(String),
    /// Posts carrying a hashtag, live.
    Tag(String),
    /// Any URL you name.
    Url(String),
}

impl Source {
    /// Read the `text` parameter out of a saver's query string.
    pub fn from_query(query: &str) -> Self {
        let Ok(params) = web_sys::UrlSearchParams::new_with_str(query) else {
            return Source::Fortune;
        };
        match params.get("text") {
            Some(raw) => Self::parse(&raw),
            None => Source::Fortune,
        }
    }

    fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if raw.is_empty() {
            return Source::Fortune;
        }
        // A URL is checked for first, because a scheme is unambiguous and a
        // handle never contains one.
        if raw.starts_with("https://") || raw.starts_with("http://") {
            return Source::Url(raw.to_string());
        }
        if let Some(tag) = raw.strip_prefix('#') {
            let tag = tag.trim().to_ascii_lowercase();
            return if tag.is_empty() {
                Source::Fortune
            } else {
                Source::Tag(tag)
            };
        }
        let account = raw.trim_start_matches('@').trim();
        if account.is_empty() {
            Source::Fortune
        } else {
            Source::Account(account.to_string())
        }
    }

    /// The value to write back into the `text` query parameter, so a
    /// configured saver stays shareable after the panel rewrites the URL.
    pub fn as_param(&self) -> Option<String> {
        match self {
            Source::Fortune => None,
            Source::Account(a) => Some(format!("%40{a}")),
            Source::Tag(t) => Some(format!("%23{t}")),
            Source::Url(u) => Some(encode(u)),
        }
    }

    /// The panel's two fields for this source: which kind it is, and the name
    /// inside it. See [`crate::images::Source::parts`].
    pub fn parts(&self) -> (&'static str, String) {
        match self {
            Source::Fortune => ("fortune", String::new()),
            Source::Account(a) => ("account", a.clone()),
            Source::Tag(t) => ("tag", t.clone()),
            Source::Url(u) => ("url", u.clone()),
        }
    }

    /// Build a source from the panel's two fields.
    pub fn from_parts(kind: &str, name: &str) -> Self {
        let trimmed = name.trim();
        let bare = trimmed.trim_start_matches(['@', '#']).trim();
        match kind {
            "account" if !bare.is_empty() => Source::Account(bare.to_string()),
            "tag" if !bare.is_empty() => Source::Tag(bare.to_ascii_lowercase()),
            "url" if !trimmed.is_empty() => Source::Url(trimmed.to_string()),
            _ => Source::Fortune,
        }
    }

    /// What to show in the panel.
    pub fn describe(&self) -> String {
        match self {
            Source::Fortune => "a poem".into(),
            Source::Account(a) => format!("@{a}"),
            Source::Tag(t) => format!("#{t}"),
            Source::Url(u) => u.clone(),
        }
    }
}

fn encode(s: &str) -> String {
    js_sys::encode_uri_component(s).into()
}

thread_local! {
    /// One account's posts, harvested once and then drawn from.
    static ACCOUNT_POSTS: RefCell<HashMap<String, Vec<String>>> = RefCell::new(HashMap::new());
    /// Posts arriving live on a hashtag.
    static TAG_QUEUE: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    /// The socket feeding it, and which tag it is following.
    static TAG_SOCKET: RefCell<Option<(String, web_sys::WebSocket)>> = const { RefCell::new(None) };
}

/// The next passage of text, or `None` if this source has nothing right now.
///
/// A hashtag genuinely has nothing until somebody posts, which is the point of
/// it, so `None` is an ordinary answer rather than a failure.
pub async fn next_text(source: &Source) -> Option<String> {
    match source {
        Source::Fortune => match fetch_text(FORTUNE_URL).await {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("screensaver text: {e}");
                None
            }
        },
        Source::Url(url) => match fetch_text(url).await {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("screensaver text: {e}");
                None
            }
        },
        Source::Account(account) => next_account_post(account).await,
        Source::Tag(tag) => {
            ensure_tag_subscription(tag);
            TAG_QUEUE.with(|q| q.borrow_mut().pop_front())
        }
    }
}

/// Fetch a URL and turn whatever came back into readable text.
async fn fetch_text(url: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| {
            format!(
                "could not read {url}. A browser will only let this page read \
                 a response whose server sends access-control-allow-origin, \
                 and that is the server's to give, not ours. ({e:?})"
            )
        })?
        .dyn_into()
        .map_err(|e| format!("unexpected fetch result: {e:?}"))?;
    if !response.ok() {
        return Err(format!("HTTP {} from {url}", response.status()));
    }
    let kind = response
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = JsFuture::from(response.text().map_err(|e| format!("no body: {e:?}"))?)
        .await
        .map_err(|e| format!("could not read the body: {e:?}"))?
        .as_string()
        .ok_or("body was not text")?;

    let text = if kind.contains("json") {
        json_strings(&body)
    } else if kind.contains("html") {
        strip_tags(&body)
    } else {
        body
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        Err(format!("nothing readable at {url}"))
    } else {
        Ok(text)
    }
}

/// Flatten a JSON document to the strings in it, one per line.
///
/// This is what makes `?text=<any quote API>` work without a case for each
/// one. Values that are plainly not prose are skipped: a URL, or an
/// identifier with no spaces and no vowels, would otherwise show up as a line
/// of the poem.
fn json_strings(body: &str) -> String {
    let Ok(value) = js_sys::JSON::parse(body) else {
        return String::new();
    };
    let mut out = Vec::new();
    walk(&value, &mut out);
    out.join("\n")
}

fn walk(v: &JsValue, out: &mut Vec<String>) {
    if let Some(s) = v.as_string() {
        if is_prose(&s) {
            out.push(s);
        }
        return;
    }
    if js_sys::Array::is_array(v) {
        for item in js_sys::Array::from(v).iter() {
            walk(&item, out);
        }
        return;
    }
    if v.is_object() {
        let obj = js_sys::Object::from(v.clone());
        for key in js_sys::Object::keys(&obj).iter() {
            if let Ok(item) = js_sys::Reflect::get(&obj, &key) {
                walk(&item, out);
            }
        }
    }
}

/// Whether a string out of a JSON document looks like something to read.
fn is_prose(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true; // a blank line in a poem is a blank line
    }
    if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("did:") {
        return false;
    }
    // An identifier: one long run of characters with no space in it. Real
    // prose has spaces, and a one-word answer is short.
    t.len() <= 24 || t.contains(' ')
}

/// Take the text out of an HTML document.
fn strip_tags(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut depth = 0usize;
    let mut skipping: Option<&str> = None;
    let lower = body.to_ascii_lowercase();
    let mut i = 0;
    while i < body.len() {
        if let Some(end) = skipping {
            if let Some(at) = lower[i..].find(end) {
                i += at + end.len();
                skipping = None;
            } else {
                break;
            }
            continue;
        }
        let c = body[i..].chars().next().unwrap_or(' ');
        match c {
            '<' => {
                if lower[i..].starts_with("<script") {
                    skipping = Some("</script>");
                    i += 7;
                    continue;
                }
                if lower[i..].starts_with("<style") {
                    skipping = Some("</style>");
                    i += 6;
                    continue;
                }
                depth += 1;
            }
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
        i += c.len_utf8();
    }
    collapse_blank_lines(&out)
}

/// Markup leaves long runs of blank lines behind. Keep at most one, so a page
/// does not arrive as a screenful of nothing.
///
/// Split on the line feed rather than by `lines()`: each line is trimmed
/// anyway, so a carriage return in front of one makes no difference.
fn collapse_blank_lines(s: &str) -> String {
    let mut text = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// One post from an account, harvested straight out of its repo.
async fn next_account_post(account: &str) -> Option<String> {
    let cached = ACCOUNT_POSTS.with(|c| c.borrow().get(account).cloned());
    let posts = match cached {
        Some(posts) => posts,
        None => {
            let posts = harvest_account(account).await.unwrap_or_else(|e| {
                log::warn!("screensaver text: {e}");
                Vec::new()
            });
            ACCOUNT_POSTS.with(|c| {
                c.borrow_mut().insert(account.to_string(), posts.clone());
            });
            posts
        }
    };
    if posts.is_empty() {
        return None;
    }
    let i = (js_sys::Math::random() * posts.len() as f64) as usize;
    posts.get(i.min(posts.len() - 1)).cloned()
}

/// Walk an account's posts out of its own PDS. No appview, no authentication.
async fn harvest_account(account: &str) -> Result<Vec<String>, String> {
    let did = resolve_did(account).await?;
    let pds = resolve_pds(&did).await?;
    let url = format!(
        "{pds}/xrpc/com.atproto.repo.listRecords\
         ?repo={did}&collection=app.bsky.feed.post&limit=100"
    );
    let page: ListRecords = fetch_json(&url).await?;
    Ok(page
        .records
        .into_iter()
        .map(|r| r.value.text)
        .filter(|t| !t.trim().is_empty())
        .collect())
}

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
    #[serde(default)]
    text: String,
}

/// Follow a hashtag on the firehose, the same way the image source does.
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
        log::warn!("screensaver text: could not open the firehose");
        return;
    };

    let wanted = tag.to_string();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let Some(frame) = ev.data().as_string() else {
            return;
        };
        if let Some(text) = tagged_text(&frame, &wanted) {
            TAG_QUEUE.with(|q| {
                let mut q = q.borrow_mut();
                if q.len() >= TAG_QUEUE_MAX {
                    q.pop_front();
                }
                q.push_back(text);
            });
        }
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    TAG_SOCKET.with(|s| *s.borrow_mut() = Some((tag.to_string(), socket)));
}

/// The text of a firehose frame, if it is a new post carrying the tag.
fn tagged_text(frame: &str, tag: &str) -> Option<String> {
    let value = js_sys::JSON::parse(frame).ok()?;
    let event: JetstreamEvent = serde_wasm_bindgen::from_value(value).ok()?;
    let commit = event.commit?;
    if commit.operation.as_deref() != Some("create") {
        return None;
    }
    let record = commit.record?;
    if !record.mentions_tag(tag) || record.text.trim().is_empty() {
        return None;
    }
    Some(record.text)
}

#[derive(Deserialize)]
struct JetstreamEvent {
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

#[cfg(test)]
mod tests {
    use super::Source;

    /// The panel hands over what was typed. A hashtag is `poetry`, not
    /// `%23poetry`, and a pasted `#poetry` is not doubled.
    #[test]
    fn the_panel_fields_build_a_source_without_escapes() {
        assert_eq!(
            Source::from_parts("tag", "Poetry"),
            Source::Tag("poetry".into())
        );
        assert_eq!(
            Source::from_parts("tag", "#poetry"),
            Source::Tag("poetry".into())
        );
        assert_eq!(
            Source::from_parts("url", "https://example.com/a"),
            Source::Url("https://example.com/a".into())
        );
        // Nothing typed falls back to the poem, which is the default.
        assert_eq!(Source::from_parts("account", " "), Source::Fortune);
    }

    #[test]
    fn parts_and_from_parts_round_trip() {
        for source in [
            Source::Fortune,
            Source::Account("overby.me".into()),
            Source::Tag("poetry".into()),
            Source::Url("https://example.com/a".into()),
        ] {
            let (kind, name) = source.parts();
            assert_eq!(Source::from_parts(kind, &name), source);
        }
    }
}
