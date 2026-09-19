//! Files: what a `file` node points at, the pictures in a document, a
//! candidate's photo. This replaces NHost storage.
//!
//! A blob belongs to a context and is readable by whoever may read that
//! context, the line every other read draws. The bytes live on disk under their
//! SHA-256, so a file uploaded twice is stored once. The row is what a node
//! refers to, by an id that for a migrated file is its old storage id, so a
//! node's `data.fileId` keeps meaning what it meant.
//!
//! An `<iframe>`, a `<video>` and Microsoft's document viewer cannot send a
//! header, so they get a link that carries its own authority: an HMAC over the
//! blob id and an expiry. The link IS the capability, as in the interim
//! (`backend/src/office.rs`): whoever holds it reads that one file until it
//! expires. It is minted only for a caller who may read the file themselves.

use crate::AppState;
use crate::config::Config;
use crate::db::DbError;
use crate::session::{Caller, MaybeCaller};
use crate::xrpc::{err, forbidden, invalid, member_of, owns, write_failed};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_SECURITY_POLICY, CONTENT_TYPE, RANGE, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use turso::Value;

pub const BLOB_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS blob (
  id         TEXT PRIMARY KEY,
  context_id TEXT NOT NULL REFERENCES context(id),
  owner_did  TEXT REFERENCES user(did),
  sha256     TEXT NOT NULL,
  size       INTEGER NOT NULL,
  mime       TEXT NOT NULL,
  name       TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS blob_by_sha ON blob(sha256);
"#;

/// The interim's figure: Microsoft's viewer fetches a document more than once,
/// and a leaked link should be dead the same afternoon.
const LINK_TTL_SECS: u64 = 2 * 60 * 60;

const MAX_NAME_CHARS: usize = 255;

/// Held while a file and its row are made to agree. Without it a delete could
/// unlink bytes between an upload's rename and its insert: a row with no file.
static FILES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type Failure = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlobMeta {
    pub id: String,
    pub context_id: String,
    pub owner_did: Option<String>,
    pub sha256: String,
    pub size: i64,
    pub mime: String,
    pub name: Option<String>,
}

async fn meta(state: &AppState, id: &str) -> Result<Option<BlobMeta>, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT id, context_id, owner_did, sha256, size, mime, name FROM blob WHERE id = ?1",
            [id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let text = |i: usize| match row.get_value(i) {
        Ok(Value::Text(s)) => Some(s),
        _ => None,
    };
    Ok(Some(BlobMeta {
        id: row.get::<String>(0)?,
        context_id: row.get::<String>(1)?,
        owner_did: text(2),
        sha256: row.get::<String>(3)?,
        size: row.get::<i64>(4)?,
        mime: row.get::<String>(5)?,
        name: text(6),
    }))
}

/// The blob `id`, if `did` may read its context. Unreadable and missing are one
/// answer.
async fn readable(
    state: &AppState,
    id: &str,
    did: Option<&str>,
) -> Result<Option<BlobMeta>, DbError> {
    let Some(blob) = meta(state, id).await? else {
        return Ok(None);
    };
    let context = crate::Store::new(state.db.clone())
        .read_context(&blob.context_id, did)
        .await?;
    Ok(context.map(|_| blob))
}

fn path_of(config: &Config, sha256: &str) -> PathBuf {
    let shard = sha256.get(..2).unwrap_or("00");
    PathBuf::from(&config.blob_dir).join(shard).join(sha256)
}

/// A content type worth storing: `type/subtype` with its parameters dropped, or
/// the generic one. It comes back out in a header, so it is not taken on trust.
fn clean_mime(raw: Option<&str>) -> String {
    let essence = raw
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let token = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    match essence.split_once('/') {
        Some((kind, sub)) if token(kind) && token(sub) => essence,
        _ => "application/octet-stream".to_string(),
    }
}

/// Whether a browser may show this type in place. A list of what is safe, not
/// of what is dangerous: a page, an SVG or any XML a member uploaded would run
/// as this origin, and a type nobody thought of must land on the safe side.
fn shown_in_place(mime: &str) -> bool {
    let media = ["image/", "video/", "audio/"]
        .iter()
        .any(|kind| mime.starts_with(kind));
    (media && !mime.ends_with("+xml")) || mime == "application/pdf" || mime == "text/plain"
}

/// `Content-Disposition` for a stored name: an ASCII rendering every client
/// reads, and the real name (RFC 6266 `filename*`) for those that can.
fn disposition(kind: &str, name: Option<&str>) -> String {
    let name = name.unwrap_or("file");
    let ascii: String = name
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() || "._- ".contains(c) => c,
            _ => '_',
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b if b.is_ascii_alphanumeric() || b"-._~".contains(&b) => (b as char).to_string(),
            b => format!("%{b:02X}"),
        })
        .collect();
    format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[derive(Debug, PartialEq)]
enum Span {
    Whole,
    /// Inclusive offsets.
    Part(u64, u64),
    Beyond,
}

/// What a `Range` header asks of a file of `size`. Several ranges at once, or
/// nonsense, is answered with the whole file, which a client must accept.
fn span(range: Option<&str>, size: u64) -> Span {
    let Some((first, last)) = range
        .and_then(|r| r.strip_prefix("bytes="))
        .filter(|spec| !spec.contains(','))
        .and_then(|spec| spec.split_once('-'))
    else {
        return Span::Whole;
    };
    let end = size.saturating_sub(1);
    let asked = match (first.trim().parse::<u64>(), last.trim().parse::<u64>()) {
        (Ok(first), Ok(last)) if first <= last => (first, last.min(end)),
        (Ok(first), Err(_)) if last.trim().is_empty() => (first, end),
        // `-n`: the last n bytes.
        (Err(_), Ok(n)) if first.trim().is_empty() && n > 0 => (size.saturating_sub(n), end),
        _ => return Span::Whole,
    };
    if asked.0 >= size {
        Span::Beyond
    } else {
        Span::Part(asked.0, asked.1)
    }
}

/// Clear away what an upload cut short by a crash left behind.
pub async fn sweep_incoming(config: &Config) {
    let Ok(mut entries) = tokio::fs::read_dir(&config.blob_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with("incoming-") {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Signed links.
// ---------------------------------------------------------------------------

type HmacSha256 = Hmac<Sha256>;

fn mac(config: &Config, id: &str, expires: u64) -> Option<HmacSha256> {
    let mut mac = HmacSha256::new_from_slice(config.secret.as_bytes()).ok()?;
    // Labelled, so this signature is good for nothing else the secret may come
    // to sign.
    mac.update(b"wiki-appview blob link v1\n");
    mac.update(id.as_bytes());
    mac.update(b"\n");
    mac.update(expires.to_string().as_bytes());
    Some(mac)
}

fn sign(config: &Config, id: &str, expires: u64) -> Option<String> {
    let mac = mac(config, id, expires)?;
    Some(crate::util::b64url(&mac.finalize().into_bytes()))
}

fn verify(config: &Config, id: &str, expires: u64, signature: &str, now: u64) -> bool {
    if expires <= now {
        return false;
    }
    match (
        mac(config, id, expires),
        crate::util::b64url_decode(signature),
    ) {
        (Some(mac), Ok(given)) => mac.verify_slice(&given).is_ok(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Storing and forgetting.
// ---------------------------------------------------------------------------

/// Stream `body` into `incoming`, hashing it on the way. `None` when it outgrew
/// the limit.
async fn receive(
    config: &Config,
    incoming: &std::path::Path,
    body: Body,
) -> Result<Option<(String, u64)>, Failure> {
    tokio::fs::create_dir_all(&config.blob_dir).await?;
    let mut file = tokio::fs::File::create(incoming).await?;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut chunks = body.into_data_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        if size > config.max_blob_bytes {
            return Ok(None);
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Some((sha256, size)))
}

/// Move the received bytes under their hash and record the row.
async fn keep(
    state: &AppState,
    incoming: &std::path::Path,
    blob: &BlobMeta,
) -> Result<(), Failure> {
    let at = path_of(&state.config, &blob.sha256);
    let _files = FILES.lock().await;
    if let Some(shard) = at.parent() {
        tokio::fs::create_dir_all(shard).await?;
        // The same bytes are the same file: a rename over them changes nothing.
        tokio::fs::rename(incoming, &at).await?;
        // The row is durable once written; the name it relies on must be too.
        tokio::fs::File::open(shard).await?.sync_all().await?;
    }
    let text = |s: &Option<String>| s.clone().map_or(Value::Null, Value::Text);
    let conn = state.db.acquire().await?;
    conn.execute(
        "INSERT INTO blob (id, context_id, owner_did, sha256, size, mime, name) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        vec![
            Value::Text(blob.id.clone()),
            Value::Text(blob.context_id.clone()),
            text(&blob.owner_did),
            Value::Text(blob.sha256.clone()),
            Value::Integer(blob.size),
            Value::Text(blob.mime.clone()),
            text(&blob.name),
        ],
    )
    .await?;
    Ok(())
}

/// Drop the row, and the bytes with it unless another row shares them.
async fn forget(state: &AppState, blob: &BlobMeta) -> Result<(), DbError> {
    let _files = FILES.lock().await;
    let conn = state.db.acquire().await?;
    conn.execute("DELETE FROM blob WHERE id = ?1", [blob.id.as_str()])
        .await?;
    let mut shared = conn
        .query(
            "SELECT 1 FROM blob WHERE sha256 = ?1",
            [blob.sha256.as_str()],
        )
        .await?;
    if shared.next().await?.is_none() {
        let _ = tokio::fs::remove_file(path_of(&state.config, &blob.sha256)).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The handlers.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UploadParams {
    pub context: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// `com.example.wiki.uploadBlob` (procedure): the raw bytes as the body, their
/// type as `Content-Type`. A member of the context may. The body goes to disk
/// as it arrives, so an upload costs no memory of its size.
pub async fn upload_blob(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Query(p): Query<UploadParams>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let what = "uploadBlob";
    // Before a byte is read: a stranger must not be able to make this write.
    if let Err(refusal) = member_of(&state, &p.context, &did, what).await {
        return refusal;
    }
    let too_large = || {
        err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "BlobTooLarge",
            "the file is too large",
        )
    };
    let declared = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > state.config.max_blob_bytes) {
        return too_large();
    }

    let incoming = PathBuf::from(&state.config.blob_dir)
        .join(format!("incoming-{}", crate::util::random_token(12)));
    let (sha256, size) = match receive(&state.config, &incoming, body).await {
        Ok(Some(received)) if received.1 > 0 => received,
        outcome => {
            let _ = tokio::fs::remove_file(&incoming).await;
            return match outcome {
                Ok(Some(_)) => invalid("an empty file"),
                Ok(None) => too_large(),
                Err(e) => write_failed(what, e),
            };
        }
    };
    let blob = BlobMeta {
        id: format!("b-{}", crate::util::random_token(16)),
        context_id: p.context,
        owner_did: Some(did),
        sha256,
        size: i64::try_from(size).unwrap_or(i64::MAX),
        mime: clean_mime(headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok())),
        name: p
            .name
            .map(|name| name.trim().chars().take(MAX_NAME_CHARS).collect::<String>())
            .filter(|name| !name.is_empty()),
    };
    match keep(&state, &incoming, &blob).await {
        Ok(()) => (StatusCode::OK, Json(blob)).into_response(),
        Err(e) => {
            let _ = tokio::fs::remove_file(&incoming).await;
            write_failed(what, e)
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct IdParam {
    pub id: String,
}

fn no_such_file() -> Response {
    err(StatusCode::NOT_FOUND, "NotFound", "no such file")
}

/// `com.example.wiki.getBlobLink`: a link to a blob that carries its own
/// authority, for what cannot send a header.
pub async fn get_blob_link(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<IdParam>,
) -> Response {
    let what = "getBlobLink";
    // The link outlives this check, so it is only ever made for someone who
    // could have fetched the file themselves.
    match readable(&state, &p.id, caller.did()).await {
        Ok(Some(_)) => {}
        Ok(None) => return no_such_file(),
        Err(e) => return write_failed(what, e),
    }
    let expires = crate::util::now_secs() + LINK_TTL_SECS;
    let Some(signature) = sign(&state.config, &p.id, expires) else {
        return write_failed(what, "no signing key");
    };
    let url = format!(
        "{}/blob/{}?exp={expires}&sig={signature}",
        state.config.base_url(),
        p.id
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "url": url, "expires": expires })),
    )
        .into_response()
}

/// `com.example.wiki.deleteBlob` (procedure): a member deletes what they
/// uploaded, an owner of the context anything in it. Membership is asked for
/// even of the uploader: someone put out of a group must not be able to pull
/// their files out of its documents on the way.
pub async fn delete_blob(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(p): Json<IdParam>,
) -> Response {
    let what = "deleteBlob";
    let blob = match readable(&state, &p.id, Some(&did)).await {
        Ok(Some(blob)) => blob,
        Ok(None) => return no_such_file(),
        Err(e) => return write_failed(what, e),
    };
    let membership = match member_of(&state, &blob.context_id, &did, what).await {
        Ok(membership) => membership,
        Err(refusal) => return refusal,
    };
    if !owns(membership) && blob.owner_did.as_deref() != Some(did.as_str()) {
        return forbidden("only whoever uploaded a file, or an owner, may delete it");
    }
    match forget(&state, &blob).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "id": blob.id }))).into_response(),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct LinkParams {
    #[serde(default)]
    pub exp: Option<u64>,
    #[serde(default)]
    pub sig: Option<String>,
}

/// `GET /blob/<id>`: the bytes, to a session that may read the blob or to
/// whoever holds a signed link that has not expired.
pub async fn serve_blob(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(link): Query<LinkParams>,
    caller: MaybeCaller,
    headers: HeaderMap,
) -> Response {
    let found = match (link.exp, link.sig.as_deref()) {
        (Some(expires), Some(signature)) => {
            let now = crate::util::now_secs();
            if !verify(&state.config, &id, expires, signature, now) {
                return err(
                    StatusCode::FORBIDDEN,
                    "BadLink",
                    "the link is invalid or has expired",
                );
            }
            meta(&state, &id).await
        }
        _ => readable(&state, &id, caller.did()).await,
    };
    let blob = match found {
        Ok(Some(blob)) => blob,
        Ok(None) => return no_such_file(),
        Err(e) => return write_failed("serveBlob", e),
    };
    let Ok(mut file) = tokio::fs::File::open(path_of(&state.config, &blob.sha256)).await else {
        tracing::error!("blob {} has a row and no file", blob.id);
        return no_such_file();
    };

    let size = u64::try_from(blob.size).unwrap_or(0);
    let range = headers.get(RANGE).and_then(|v| v.to_str().ok());
    let (status, first, last) = match span(range, size) {
        Span::Whole => (StatusCode::OK, 0, size.saturating_sub(1)),
        Span::Part(first, last) => (StatusCode::PARTIAL_CONTENT, first, last),
        Span::Beyond => {
            let mut resp = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}")) {
                resp.headers_mut().insert(CONTENT_RANGE, v);
            }
            return resp;
        }
    };
    let len = if size == 0 { 0 } else { last - first + 1 };
    if first > 0 && file.seek(std::io::SeekFrom::Start(first)).await.is_err() {
        return no_such_file();
    }
    let bytes = tokio_util::io::ReaderStream::new(file.take(len));

    let mut resp = (status, Body::from_stream(bytes)).into_response();
    let h = resp.headers_mut();
    let mut set = |name, value: &str| {
        if let Ok(value) = HeaderValue::from_str(value) {
            h.insert(name, value);
        }
    };
    set(CONTENT_TYPE, &blob.mime);
    set(CONTENT_LENGTH, &len.to_string());
    set(ACCEPT_RANGES, "bytes");
    if status == StatusCode::PARTIAL_CONTENT {
        set(CONTENT_RANGE, &format!("bytes {first}-{last}/{size}"));
    }
    // Who asks decides what may be read, and the next person at a shared
    // browser is someone else. Long, since an id never means other bytes.
    set(CACHE_CONTROL, "private, max-age=86400");
    set(VARY, "Authorization");
    set(X_CONTENT_TYPE_OPTIONS, "nosniff");
    if shown_in_place(&blob.mime) {
        // No CSP here: `sandbox` stops a browser's own PDF viewer from loading.
        set(
            CONTENT_DISPOSITION,
            &disposition("inline", blob.name.as_deref()),
        );
    } else {
        set(
            CONTENT_DISPOSITION,
            &disposition("attachment", blob.name.as_deref()),
        );
        set(CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'");
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{join, seeded_state, token_for};
    use axum::http::Request;
    use tower::ServiceExt;

    /// A state whose blobs land in a directory of their own.
    async fn state() -> AppState {
        let mut state = seeded_state().await;
        state.config.blob_dir = std::env::temp_dir()
            .join(format!("appview-blobs-{}", crate::util::random_token(8)))
            .to_string_lossy()
            .into_owned();
        state
    }

    async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let resp = router(state.clone()).oneshot(req).await.expect("response");
        let (status, headers) = (resp.status(), resp.headers().clone());
        let bytes = axum::body::to_bytes(resp.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body");
        (status, headers, bytes.to_vec())
    }

    async fn upload_named(
        state: &AppState,
        who: &str,
        context: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("POST")
            .uri(format!(
                "/xrpc/com.example.wiki.uploadBlob?context={context}&name={name}"
            ))
            .header("authorization", format!("Bearer {who}"))
            .header("content-type", mime)
            .body(Body::from(bytes.to_vec()))
            .expect("request");
        let (status, _, body) = send(state, req).await;
        (status, serde_json::from_slice(&body).unwrap_or_default())
    }

    async fn upload(
        state: &AppState,
        who: &str,
        context: &str,
        mime: &str,
        bytes: &[u8],
    ) -> (StatusCode, serde_json::Value) {
        upload_named(state, who, context, "file", mime, bytes).await
    }

    async fn fetch(
        state: &AppState,
        uri: &str,
        who: Option<&str>,
        range: Option<&str>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder().uri(uri);
        if let Some(who) = who {
            req = req.header("authorization", format!("Bearer {who}"));
        }
        if let Some(range) = range {
            req = req.header("range", range);
        }
        send(state, req.body(Body::empty()).expect("request")).await
    }

    async fn delete(state: &AppState, who: &str, id: &serde_json::Value) -> StatusCode {
        let req = Request::builder()
            .method("POST")
            .uri("/xrpc/com.example.wiki.deleteBlob")
            .header("authorization", format!("Bearer {who}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "id": id }).to_string()))
            .expect("request");
        send(state, req).await.0
    }

    fn uri_of(uploaded: &serde_json::Value) -> String {
        format!("/blob/{}", uploaded["id"].as_str().expect("id"))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_file_goes_up_and_comes_back_to_those_who_may_read_its_context() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let pdf = "application/pdf; charset=binary";
        let (status, v) = upload(&state, &bob, "c9", pdf, b"%PDF-1.7 hello").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["size"], 14);
        assert_eq!(v["mime"], "application/pdf", "parameters are dropped");
        assert_eq!(v["context_id"], "c9");

        let alice = token_for(&state, "did:plc:alice").await;
        let (status, headers, body) = fetch(&state, &uri_of(&v), Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"%PDF-1.7 hello");
        assert_eq!(headers[CONTENT_TYPE], "application/pdf");
        assert_eq!(headers[CONTENT_LENGTH], "14");
        assert_eq!(headers[X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[VARY], "Authorization");

        let mallory = token_for(&state, "did:plc:mallory").await;
        for stranger in [Some(mallory.as_str()), None] {
            assert_eq!(
                fetch(&state, &uri_of(&v), stranger, None).await.0,
                StatusCode::NOT_FOUND,
                "a closed group's file was served outside it"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_public_contexts_file_needs_no_session() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        join(&state, "did:plc:bob", "c1").await;
        let (_, v) = upload(&state, &bob, "c1", "image/png", b"\x89PNG").await;
        let (status, _, body) = fetch(&state, &uri_of(&v), None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"\x89PNG");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_a_member_may_put_a_file_into_a_context() {
        let state = state().await;
        let mallory = token_for(&state, "did:plc:mallory").await;
        assert_eq!(
            upload(&state, &mallory, "c9", "text/plain", b"x").await.0,
            StatusCode::NOT_FOUND
        );
        // c1 is public, so she may read it, and that is all.
        assert_eq!(
            upload(&state, &mallory, "c1", "text/plain", b"x").await.0,
            StatusCode::FORBIDDEN
        );
        assert!(
            tokio::fs::read_dir(&state.config.blob_dir).await.is_err(),
            "a refused upload reached the disk"
        );
        let bob = token_for(&state, "did:plc:bob").await;
        assert_eq!(
            upload(&state, &bob, "c9", "text/plain", b"").await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_file_over_the_limit_is_refused_and_leaves_nothing_behind() {
        let mut state = state().await;
        state.config.max_blob_bytes = 8;
        let bob = token_for(&state, "did:plc:bob").await;

        let (status, v) = upload(&state, &bob, "c9", "text/plain", b"nine byte").await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{v}");

        // The same again with no length declared, so it is the counting that
        // refuses it and not the header.
        let chunks =
            futures_util::stream::iter([&b"nine"[..], &b" byte"[..]].map(Ok::<_, std::io::Error>));
        let req = Request::builder()
            .method("POST")
            .uri("/xrpc/com.example.wiki.uploadBlob?context=c9")
            .header("authorization", format!("Bearer {bob}"))
            .body(Body::from_stream(chunks))
            .expect("request");
        assert_eq!(send(&state, req).await.0, StatusCode::PAYLOAD_TOO_LARGE);

        let mut left = tokio::fs::read_dir(&state.config.blob_dir)
            .await
            .expect("dir");
        assert!(
            left.next_entry().await.expect("entry").is_none(),
            "a refused upload left its bytes on disk"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_signed_link_opens_one_file_until_it_expires() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let (_, up) = upload(&state, &bob, "c9", "image/png", b"\x89PNG....").await;
        let id = up["id"].as_str().expect("id");

        let mallory = token_for(&state, "did:plc:mallory").await;
        let link = format!("/xrpc/com.example.wiki.getBlobLink?id={id}");
        assert_eq!(
            fetch(&state, &link, Some(&mallory), None).await.0,
            StatusCode::NOT_FOUND,
            "a link was minted for someone who may not read the file"
        );
        let (status, _, body) = fetch(&state, &link, Some(&bob), None).await;
        assert_eq!(status, StatusCode::OK);
        let minted: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let url = minted["url"].as_str().expect("url");

        // No session at all: the link is the authority.
        let (status, _, bytes) = fetch(&state, url, None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, b"\x89PNG....");

        let expires = minted["expires"].as_u64().expect("expires");
        let signature = url.split("sig=").nth(1).expect("sig");
        let config = &state.config;
        assert!(verify(config, id, expires, signature, expires - 1));
        assert!(!verify(config, id, expires, signature, expires), "expired");
        assert!(!verify(config, "another", expires, signature, 0));
        assert!(!verify(config, id, expires + 1, signature, 0), "stretched");
        let mut other = state.config.clone();
        other.secret = crate::config::Secret::new("another secret");
        assert!(!verify(&other, id, expires, signature, 0), "another key");

        let forged = format!("/blob/{id}?exp={}&sig=AAAA", expires + 10_000);
        assert_eq!(
            fetch(&state, &forged, Some(&bob), None).await.0,
            StatusCode::FORBIDDEN,
            "a bad link is refused even to someone who could read the file"
        );
    }

    /// A page or an SVG a member uploads must not run as this origin, where it
    /// could act with the session of whoever opened it.
    #[tokio::test(flavor = "current_thread")]
    async fn only_what_cannot_run_is_shown_in_place() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let script = b"<script>alert(1)</script>";
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/xhtml+xml",
            "application/atom+xml",
            "text/xml",
            "application/x-something-new",
        ] {
            let (_, up) = upload(&state, &bob, "c9", mime, script).await;
            let (_, headers, _) = fetch(&state, &uri_of(&up), Some(&bob), None).await;
            let shown = headers[CONTENT_DISPOSITION].to_str().expect("ascii");
            assert!(shown.starts_with("attachment"), "{mime} was served inline");
            assert!(
                headers[CONTENT_SECURITY_POLICY]
                    .to_str()
                    .expect("ascii")
                    .contains("sandbox")
            );
        }
        for mime in ["application/pdf", "image/jpeg", "video/mp4", "text/plain"] {
            let (_, up) = upload(&state, &bob, "c9", mime, b"bytes").await;
            let (_, headers, _) = fetch(&state, &uri_of(&up), Some(&bob), None).await;
            let shown = headers[CONTENT_DISPOSITION].to_str().expect("ascii");
            assert!(shown.starts_with("inline"), "{mime} was made a download");
            assert!(
                !headers.contains_key(CONTENT_SECURITY_POLICY),
                "{mime}: a sandbox stops the browser's own PDF viewer"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_name_survives_in_full_for_clients_that_can_read_it() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let name = "%C3%85rsm%C3%B8de%20%22referat%22.pdf";
        let (_, up) = upload_named(&state, &bob, "c9", name, "application/pdf", b"%PDF").await;
        assert_eq!(up["name"], "Årsmøde \"referat\".pdf");
        let (_, headers, _) = fetch(&state, &uri_of(&up), Some(&bob), None).await;
        assert_eq!(
            headers[CONTENT_DISPOSITION],
            "inline; filename=\"_rsm_de _referat_.pdf\"; \
             filename*=UTF-8''%C3%85rsm%C3%B8de%20%22referat%22.pdf"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_range_is_served_as_a_range() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let (_, up) = upload(&state, &bob, "c9", "video/mp4", b"0123456789").await;
        let uri = uri_of(&up);
        let (status, headers, body) = fetch(&state, &uri, Some(&bob), Some("bytes=2-5")).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(body, b"2345");
        assert_eq!(headers[CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(headers[CONTENT_LENGTH], "4");

        let (status, headers, _) = fetch(&state, &uri, Some(&bob), Some("bytes=10-")).await;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(headers[CONTENT_RANGE], "bytes */10");
    }

    #[test]
    fn a_range_header_is_read_the_way_players_send_it() {
        assert_eq!(span(None, 10), Span::Whole);
        assert_eq!(span(Some("bytes=0-"), 10), Span::Part(0, 9));
        assert_eq!(span(Some("bytes=7-"), 10), Span::Part(7, 9));
        assert_eq!(span(Some("bytes=2-5"), 10), Span::Part(2, 5));
        assert_eq!(span(Some("bytes=2-500"), 10), Span::Part(2, 9));
        assert_eq!(span(Some("bytes=-3"), 10), Span::Part(7, 9));
        assert_eq!(span(Some("bytes=-30"), 10), Span::Part(0, 9));
        assert_eq!(span(Some("bytes=10-"), 10), Span::Beyond);
        for whole in [
            "bytes=9-2",
            "bytes=0-1,4-5",
            "bytes=-0",
            "bytes=a-b",
            "lines=1-2",
        ] {
            assert_eq!(span(Some(whole), 10), Span::Whole, "{whole}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_same_bytes_are_stored_once_and_outlive_one_of_their_rows() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let alice = token_for(&state, "did:plc:alice").await;
        let (_, first) = upload(&state, &bob, "c9", "text/plain", b"same").await;
        let (_, second) = upload(&state, &alice, "c9", "text/plain", b"same").await;
        assert_eq!(first["sha256"], second["sha256"]);
        assert_ne!(first["id"], second["id"]);
        let on_disk = path_of(&state.config, first["sha256"].as_str().expect("sha"));

        assert_eq!(delete(&state, &bob, &first["id"]).await, StatusCode::OK);
        let (status, _, body) = fetch(&state, &uri_of(&second), Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK, "the other row lost its bytes");
        assert_eq!(body, b"same");

        assert_eq!(delete(&state, &alice, &second["id"]).await, StatusCode::OK);
        assert!(!on_disk.exists(), "the last row went and its bytes stayed");
        assert_eq!(
            fetch(&state, &uri_of(&second), Some(&alice), None).await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_file_is_deleted_by_its_uploader_or_an_owner_and_only_from_inside() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let alice = token_for(&state, "did:plc:alice").await;
        let carol = token_for(&state, "did:plc:carol").await;
        join(&state, "did:plc:carol", "c9").await;
        let (_, bobs) = upload(&state, &bob, "c9", "text/plain", b"bob's").await;
        let (_, also_bobs) = upload(&state, &bob, "c9", "text/plain", b"also bob's").await;

        assert_eq!(
            delete(&state, &carol, &bobs["id"]).await,
            StatusCode::FORBIDDEN,
            "a member deleted someone else's file"
        );
        assert_eq!(delete(&state, &alice, &bobs["id"]).await, StatusCode::OK);

        let conn = state.db.acquire().await.expect("conn");
        conn.execute("DELETE FROM member WHERE user_did = 'did:plc:bob'", ())
            .await
            .expect("bob is put out");
        assert_eq!(
            delete(&state, &bob, &also_bobs["id"]).await,
            StatusCode::NOT_FOUND,
            "someone put out of the group still reached into it"
        );
    }

    /// Through a real socket, which the in-process router never exercises: a
    /// body arriving in many reads, and a reply streamed off the disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_large_file_round_trips_over_http() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let served = router(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, served).await;
        });

        let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let http = reqwest::Client::new();
        let up: serde_json::Value = http
            .post(format!(
                "{base}/xrpc/com.example.wiki.uploadBlob?context=c9"
            ))
            .bearer_auth(&bob)
            .header("content-type", "video/mp4")
            .body(bytes.clone())
            .send()
            .await
            .expect("upload")
            .bytes()
            .await
            .map(|body| serde_json::from_slice(&body).expect("json"))
            .expect("body");
        let expected: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(up["sha256"], expected.as_str());

        let url = format!("{base}/blob/{}", up["id"].as_str().expect("id"));
        let whole = http.get(&url).bearer_auth(&bob).send().await.expect("get");
        assert_eq!(whole.content_length(), Some(3_000_000));
        assert_eq!(whole.bytes().await.expect("body").as_ref(), &bytes[..]);

        let part = http
            .get(&url)
            .bearer_auth(&bob)
            .header("range", "bytes=2999990-")
            .send()
            .await
            .expect("range");
        assert_eq!(part.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            part.bytes().await.expect("body").as_ref(),
            &bytes[2_999_990..]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_a_crash_cut_short_is_cleared_away() {
        let state = state().await;
        let dir = PathBuf::from(&state.config.blob_dir);
        tokio::fs::create_dir_all(dir.join("ab"))
            .await
            .expect("dir");
        tokio::fs::write(dir.join("incoming-half"), b"half")
            .await
            .expect("write");
        tokio::fs::write(dir.join("ab").join("abcd"), b"whole")
            .await
            .expect("write");
        sweep_incoming(&state.config).await;
        assert!(!dir.join("incoming-half").exists());
        assert!(
            dir.join("ab").join("abcd").exists(),
            "a stored file was swept"
        );
    }

    #[test]
    fn a_content_type_is_cleaned_not_trusted() {
        assert_eq!(clean_mime(Some("Image/PNG")), "image/png");
        assert_eq!(clean_mime(Some("text/plain; charset=utf-8")), "text/plain");
        for bad in [
            None,
            Some(""),
            Some("nonsense"),
            Some("text/pl ain"),
            Some("a/b\r\nX: y"),
        ] {
            assert_eq!(clean_mime(bad), "application/octet-stream", "{bad:?}");
        }
    }
}
