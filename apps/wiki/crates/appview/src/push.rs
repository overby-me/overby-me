//! Background Web Push: a device subscribes, and a chair's announcement or a
//! reply to something a member wrote reaches it while the app is closed.
//!
//! The encryption and VAPID are carried over from the interim backend
//! (`backend/src/push.rs`), hand-rolled on RustCrypto primitives because the
//! `web-push` crate pulls libcurl/openssl:
//! - message encryption follows RFC 8291 (ECDH P-256 + HKDF-SHA256) with the
//!   `aes128gcm` content encoding of RFC 8188, checked against the RFC 8291
//!   section 5 test vector;
//! - authorization is the VAPID scheme of RFC 8292: an ES256 JWT plus the
//!   server's public key.
//!
//! A push endpoint is a URL a client hands us to POST to. It is sent through the
//! shared outbound client, which a deployed AppView holds to https on public
//! addresses, resolved once and connected to as resolved. The interim checked
//! the URL's text only and accepted that a name could re-resolve inward.
//!
//! The VAPID key pair must be the interim's at cutover: a browser's subscription
//! is bound to the public half, which the frontend has compiled in.

use crate::AppState;
use crate::config::Config;
use crate::db::DbError;
use crate::session::Caller;
use crate::util;
use crate::xrpc::{err, invalid, member_of, owner_of, write_failed};
use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use hkdf::Hkdf;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use serde::Deserialize;
use serde_json::json;
use sha2::Sha256;
use turso::Value;

pub const PUSH_DDL: &str = r#"
-- A subscription belongs to a device, so the endpoint is its key: whoever signs
-- in on that device next takes it over, and is the one it then rings for.
CREATE TABLE IF NOT EXISTS push_subscription (
  endpoint   TEXT PRIMARY KEY,
  did        TEXT NOT NULL REFERENCES user(did),
  p256dh     TEXT NOT NULL,
  auth       TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS push_by_did ON push_subscription(did);
"#;

/// Deliveries in flight at once. A context of 500 with two devices each is a
/// thousand POSTs; all at once is a thousand sockets.
const FAN_OUT: usize = 32;

/// A push message is one 4096-byte record, header and tag included.
const MAX_PAYLOAD: usize = 3000;

/// One browser push subscription (the `keys` are base64url as the PushManager
/// serialises them).
pub struct Subscription {
    pub endpoint: String,
    /// The client public key (`keys.p256dh`): a 65-byte uncompressed P-256 point.
    pub p256dh: String,
    /// The client auth secret (`keys.auth`): 16 bytes.
    pub auth: String,
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Result<Vec<u8>, String> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut okm = vec![0u8; len];
    hk.expand(info, &mut okm).map_err(|e| e.to_string())?;
    Ok(okm)
}

/// RFC 8291 message encryption with a caller-supplied ephemeral server key and
/// salt (both random in production; fixed only to check the RFC test vector).
/// Returns the `aes128gcm` body (header || single encrypted record).
fn encrypt_with(
    as_secret: &SecretKey,
    salt: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let as_point = as_secret.public_key().to_encoded_point(false);
    let as_public = as_point.as_bytes(); // 65-byte uncompressed point

    // ECDH shared secret = the x-coordinate of as_private * ua_public.
    let ua_pk = PublicKey::from_sec1_bytes(ua_public).map_err(|e| e.to_string())?;
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_pk.as_affine());
    let ecdh_secret = shared.raw_secret_bytes();

    // IKM = HKDF(salt = auth, ikm = ecdh, info = "WebPush: info"\0 || ua_pub || as_pub).
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let ikm = hkdf(auth, ecdh_secret.as_slice(), &key_info, 32)?;

    // Per-record content-encryption key and nonce, keyed by the random salt.
    let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16)?;
    let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12)?;

    // A single record: plaintext || 0x02 (RFC 8188 last-record delimiter), sealed
    // with AES-128-GCM (empty AAD); the 16-byte tag is appended by `encrypt`.
    let mut record = plaintext.to_vec();
    record.push(0x02);
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|e| e.to_string())?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), record.as_ref())
        .map_err(|e| e.to_string())?;

    // Header: salt(16) || rs(4, big-endian) || idlen(1) || keyid(idlen = as_public).
    let rs: u32 = 4096;
    let mut body = Vec::with_capacity(16 + 4 + 1 + as_public.len() + ciphertext.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&rs.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// Encrypt `plaintext` for a subscription, generating a fresh ephemeral key + salt.
fn encrypt(p256dh: &str, auth: &str, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let ua_public = util::b64url_decode(p256dh)?;
    let auth = util::b64url_decode(auth)?;
    let as_secret = SecretKey::random(&mut rand::rngs::OsRng);
    let salt = util::random_bytes(16);
    encrypt_with(&as_secret, &salt, &ua_public, &auth, plaintext)
}

/// Whether a client-registered push endpoint looks like one. Web-push endpoints
/// are always `https://` on a DNS hostname (`fcm.googleapis.com`,
/// `*.push.services.mozilla.com`, `*.notify.windows.com`, `web.push.apple.com`),
/// so anything else is refused when it is registered, and no legitimate browser
/// subscription is. What a name RESOLVES to is the outbound client's to judge,
/// when it is sent to.
pub fn endpoint_allowed(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    // Host is up to the first '/', '?' or '#'; bracketed IPv6 ends at ']'.
    let host = if let Some(after) = rest.strip_prefix('[') {
        after.split(']').next().unwrap_or("")
    } else {
        rest.split(['/', ':', '?', '#']).next().unwrap_or("")
    };
    if host.is_empty() {
        return false;
    }
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
        return false;
    }
    host.parse::<std::net::IpAddr>().is_err()
}

/// The `scheme://host[:port]` origin of a push endpoint, for the VAPID `aud` claim.
fn origin_of(endpoint: &str) -> Result<String, String> {
    let (scheme, rest) = endpoint
        .split_once("://")
        .filter(|(scheme, _)| matches!(*scheme, "https" | "http"))
        .ok_or("endpoint is not http(s)")?;
    let host = rest.split('/').next().unwrap_or(rest);
    Ok(format!("{scheme}://{host}"))
}

/// A VAPID (RFC 8292) `Authorization` header value for `endpoint`: an ES256 JWT
/// (`aud` = the endpoint origin, `exp` ~12h out, `sub` = a contact) plus the
/// server's public key.
fn vapid_header(config: &Config, endpoint: &str, now: u64) -> Result<String, String> {
    let scalar = util::b64url_decode(config.vapid_private.expose())?;
    let signing = SigningKey::from_slice(&scalar).map_err(|e| e.to_string())?;
    let header = json!({ "typ": "JWT", "alg": "ES256" });
    let claims = json!({
        "aud": origin_of(endpoint)?,
        "exp": now + 12 * 3600,
        "sub": config.vapid_subject,
    });
    let signing_input = format!(
        "{}.{}",
        util::b64url(header.to_string().as_bytes()),
        util::b64url(claims.to_string().as_bytes())
    );
    let sig: Signature = signing.sign(signing_input.as_bytes());
    let jwt = format!("{signing_input}.{}", util::b64url(&sig.to_bytes()));
    Ok(format!("vapid t={jwt}, k={}", config.vapid_public))
}

/// Encrypt `payload` (an app-defined JSON string) for `sub` and POST it to the
/// push service. Returns the HTTP status; 404/410 means the subscription is gone
/// and the caller should drop it.
pub async fn send(state: &AppState, sub: &Subscription, payload: &[u8]) -> Result<u16, String> {
    let body = encrypt(&sub.p256dh, &sub.auth, payload)?;
    let auth = vapid_header(&state.config, &sub.endpoint, util::now_secs())?;
    let resp = state
        .http
        .post(&sub.endpoint)
        .map_err(|e| e.to_string())?
        .header("TTL", "86400")
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Urgency", "normal")
        .header(AUTHORIZATION, auth)
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

/// Push `payload` to every device of `dids`, dropping the subscriptions the push
/// service says are gone. Returns (devices, delivered).
async fn push_to(
    state: &AppState,
    dids: &[String],
    payload: &str,
) -> Result<(usize, usize), DbError> {
    let mut subs = Vec::new();
    let conn = state.db.acquire().await?;
    for did in dids {
        let mut rows = conn
            .query(
                "SELECT endpoint, p256dh, auth FROM push_subscription WHERE did = ?1",
                [did.as_str()],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            subs.push(Subscription {
                endpoint: row.get::<String>(0)?,
                p256dh: row.get::<String>(1)?,
                auth: row.get::<String>(2)?,
            });
        }
    }
    let devices = subs.len();
    // Owned, not borrowed: a closure over `&Subscription` is not general enough
    // for the handler's future to be `Send`.
    let outcomes: Vec<(Subscription, Result<u16, String>)> = futures_util::stream::iter(subs)
        .map(|sub| async move {
            let outcome = send(state, &sub, payload.as_bytes()).await;
            (sub, outcome)
        })
        .buffer_unordered(FAN_OUT)
        .collect()
        .await;

    let mut delivered = 0;
    for (sub, outcome) in outcomes {
        // Only the endpoint's origin is logged: its path is a per-device secret.
        let origin = origin_of(&sub.endpoint).unwrap_or_default();
        match outcome {
            Ok(status) if (200..300).contains(&status) => delivered += 1,
            Ok(404 | 410) => {
                conn.execute(
                    "DELETE FROM push_subscription WHERE endpoint = ?1",
                    [sub.endpoint.as_str()],
                )
                .await?;
            }
            Ok(status) => tracing::warn!("push to {origin} answered {status}"),
            Err(e) => tracing::warn!("push to {origin} failed: {e}"),
        }
    }
    Ok((devices, delivered))
}

#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

fn done() -> Response {
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `com.example.wiki.subscribePush` (procedure): this device rings for the
/// caller from now on.
pub async fn subscribe_push(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<SubscribeBody>,
) -> Response {
    if !endpoint_allowed(&body.endpoint) {
        return invalid("that is not a push service's endpoint");
    }
    let keys_fit = |b64: &str, len: usize| util::b64url_decode(b64).is_ok_and(|k| k.len() == len);
    if !keys_fit(&body.p256dh, 65) || !keys_fit(&body.auth, 16) {
        return invalid("the subscription's keys are not a P-256 point and a 16-byte secret");
    }
    let stored = async {
        let conn = state.db.acquire().await?;
        conn.execute(
            "INSERT INTO push_subscription (endpoint, did, p256dh, auth) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(endpoint) DO UPDATE SET did = excluded.did, p256dh = excluded.p256dh, \
               auth = excluded.auth",
            [
                body.endpoint.as_str(),
                did.as_str(),
                body.p256dh.as_str(),
                body.auth.as_str(),
            ],
        )
        .await?;
        Ok::<_, DbError>(())
    };
    match stored.await {
        Ok(()) => done(),
        Err(e) => write_failed("subscribePush", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribeBody {
    pub endpoint: String,
}

/// `com.example.wiki.unsubscribePush` (procedure): this device stops ringing.
/// Any signed-in caller may drop an endpoint they know: knowing it is holding
/// the device, which is how signing out on a shared phone has to work.
pub async fn unsubscribe_push(
    State(state): State<AppState>,
    _caller: Caller,
    Json(body): Json<UnsubscribeBody>,
) -> Response {
    let dropped = async {
        let conn = state.db.acquire().await?;
        conn.execute(
            "DELETE FROM push_subscription WHERE endpoint = ?1",
            [body.endpoint.as_str()],
        )
        .await?;
        Ok::<_, DbError>(())
    };
    match dropped.await {
        Ok(()) => done(),
        Err(e) => write_failed("unsubscribePush", e),
    }
}

/// What a notification says. The words are the sender's client's, in the
/// sender's language.
#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub url: String,
}

impl Message {
    /// The payload a service worker reads, or why it may not be sent.
    ///
    /// The link is what a tap opens, on the phones of people who trust where a
    /// notification from this app leads: a path in the app, or a configured
    /// frontend origin, and nothing else.
    fn payload(&self, config: &Config) -> Result<String, &'static str> {
        let local = self.url.starts_with('/') && !self.url.starts_with("//");
        if !(self.url.is_empty() || local || config.allows_return(&self.url)) {
            return Err("a notification links into the app, not elsewhere");
        }
        let payload = json!({
            "title": self.title.as_deref().unwrap_or("RadikalWiki"),
            "body": self.body,
            "url": self.url,
        })
        .to_string();
        if payload.len() > MAX_PAYLOAD {
            return Err("the notification is too long for a push message");
        }
        Ok(payload)
    }
}

fn not_configured() -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "PushNotConfigured",
        "this AppView has no VAPID key",
    )
}

async fn deliver(state: &AppState, dids: &[String], payload: &str, what: &str) -> Response {
    match push_to(state, dids, payload).await {
        Ok((recipients, sent)) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "recipients": recipients, "sent": sent })),
        )
            .into_response(),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct NotifyContextBody {
    pub context_id: String,
    #[serde(flatten)]
    pub message: Message,
}

/// `com.example.wiki.notifyContext` (procedure): an owner tells the members of
/// a context something, a poll having opened for one. It reaches those who hold
/// voting rights and have accepted, and not the sender.
pub async fn notify_context(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<NotifyContextBody>,
) -> Response {
    let what = "notifyContext";
    if let Err(refusal) = owner_of(&state, &body.context_id, &did, what).await {
        return refusal;
    }
    if state.config.vapid_private.is_empty() {
        return not_configured();
    }
    let payload = match body.message.payload(&state.config) {
        Ok(payload) => payload,
        Err(why) => return invalid(why),
    };
    let members = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT user_did FROM member WHERE context_id = ?1 AND active = 1 \
                   AND accepted = 1 AND user_did IS NOT NULL AND user_did <> ?2",
                [body.context_id.as_str(), did.as_str()],
            )
            .await?;
        let mut dids = Vec::new();
        while let Some(row) = rows.next().await? {
            dids.push(row.get::<String>(0)?);
        }
        Ok::<_, DbError>(dids)
    };
    match members.await {
        Ok(dids) => deliver(&state, &dids, &payload, what).await,
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct NotifyReplyBody {
    /// The node that was replied to.
    pub parent: String,
    #[serde(flatten)]
    pub message: Message,
}

/// `com.example.wiki.notifyReply` (procedure): tell whoever wrote a node that it
/// has been answered. Only from inside its context, so that an author cannot be
/// pinged by a stranger; and never about one's own reply.
pub async fn notify_reply(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<NotifyReplyBody>,
) -> Response {
    let what = "notifyReply";
    let node = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT context_id, owner_did FROM document \
                 WHERE id = ?1 AND deleted_at IS NULL",
                [body.parent.as_str()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let owner = match row.get_value(1)? {
            Value::Text(owner) => Some(owner),
            _ => None,
        };
        Ok::<_, DbError>(Some((row.get::<String>(0)?, owner)))
    };
    let (context_id, owner) = match node.await {
        Ok(Some(node)) => node,
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such node"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = member_of(&state, &context_id, &did, what).await {
        return refusal;
    }
    if state.config.vapid_private.is_empty() {
        return not_configured();
    }
    let payload = match body.message.payload(&state.config) {
        Ok(payload) => payload,
        Err(why) => return invalid(why),
    };
    let to: Vec<String> = owner.into_iter().filter(|owner| *owner != did).collect();
    deliver(&state, &to, &payload, what).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;
    use crate::router;
    use crate::xrpc::tests::{post, seeded_state, token_for};
    use axum::http::HeaderMap;
    use std::sync::{Arc, Mutex};

    // RFC 8291 section 5, "Push Message Encryption Example".
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    // Split mid-token so the spell-checker doesn't read a false word in the blob.
    const UA_PUBLIC: &str = concat!(
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkN",
        "toIAiw4"
    );
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const EXPECTED: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    fn rfc_body() -> Vec<u8> {
        let decode = |b64: &str| util::b64url_decode(b64).expect("base64url");
        let as_secret = SecretKey::from_slice(&decode(AS_PRIVATE)).expect("key");
        encrypt_with(
            &as_secret,
            &decode(SALT),
            &decode(UA_PUBLIC),
            &decode(AUTH),
            PLAINTEXT.as_bytes(),
        )
        .expect("encrypt")
    }

    #[test]
    fn rfc8291_example_matches() {
        let as_secret =
            SecretKey::from_slice(&util::b64url_decode(AS_PRIVATE).expect("b64")).expect("key");
        let point = as_secret.public_key().to_encoded_point(false);
        assert_eq!(util::b64url(point.as_bytes()), AS_PUBLIC);
        assert_eq!(util::b64url(&rfc_body()), EXPECTED);
    }

    #[test]
    fn header_frames_salt_rs_and_keyid() {
        let body = rfc_body();
        let decode = |b64: &str| util::b64url_decode(b64).expect("base64url");
        assert_eq!(&body[0..16], decode(SALT).as_slice());
        assert_eq!(&body[16..20], &4096u32.to_be_bytes(), "record size");
        assert_eq!(body[20], 65, "key id length");
        assert_eq!(&body[21..86], decode(AS_PUBLIC).as_slice());
    }

    #[test]
    fn an_endpoint_has_to_look_like_a_push_services() {
        // Built from bare hosts so the link checker does not go and probe them.
        let at = |scheme: &str, host: &str| format!("{scheme}://{host}/send/abc123");
        for real in [
            "fcm.googleapis.com",
            "updates.push.services.mozilla.com",
            "web.push.apple.com",
        ] {
            assert!(endpoint_allowed(&at("https", real)), "{real}");
        }
        assert!(
            !endpoint_allowed(&at("http", "fcm.googleapis.com")),
            "not https"
        );
        assert!(!endpoint_allowed(&at("ftp", "fcm.googleapis.com")));
        for inward in [
            "169.254.169.254",
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "localhost",
            "[::1]",
            "printer.local",
        ] {
            assert!(!endpoint_allowed(&at("https", inward)), "{inward}");
        }
        assert_eq!(
            origin_of(&at("https", "push.example")).expect("origin"),
            "https://push.example"
        );
    }

    type Rang = Arc<Mutex<Vec<(String, usize)>>>;

    /// A stand-in push service: notes the path rung and the size of what arrived,
    /// and answers 410 for a device it has been told is gone.
    async fn push_service() -> (String, Rang) {
        let rang = Rang::default();
        let note = rang.clone();
        let app = axum::Router::new().fallback(
            move |uri: axum::http::Uri, headers: HeaderMap, body: axum::body::Bytes| async move {
                assert_eq!(headers["content-encoding"], "aes128gcm");
                assert!(
                    headers["authorization"]
                        .to_str()
                        .expect("ascii")
                        .starts_with("vapid t=")
                );
                note.lock()
                    .expect("rang")
                    .push((uri.path().to_string(), body.len()));
                if uri.path().contains("gone") {
                    StatusCode::GONE
                } else {
                    StatusCode::CREATED
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (base, rang)
    }

    async fn configured() -> AppState {
        let mut state = seeded_state().await;
        state.config.vapid_private = Secret::new(AS_PRIVATE);
        state.config.vapid_public = AS_PUBLIC.to_string();
        state.config.vapid_subject = "https://wiki.example".to_string();
        state
    }

    /// A device of `did`, as `subscribePush` would have stored it. Put in directly
    /// because the stand-in service is on loopback, which `subscribePush` refuses.
    async fn device(state: &AppState, did: &str, endpoint: &str) {
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO push_subscription (endpoint, did, p256dh, auth) VALUES (?1, ?2, ?3, ?4)",
            [endpoint, did, UA_PUBLIC, AUTH],
        )
        .await
        .expect("device");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_owner_rings_the_members_and_a_gone_device_is_forgotten() {
        let (service, rang) = push_service().await;
        let state = configured().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute("UPDATE member SET accepted = 1 WHERE context_id = 'c9'", ())
            .await
            .expect("accepted");
        device(&state, "did:plc:bob", &format!("{service}/bob-phone")).await;
        device(&state, "did:plc:bob", &format!("{service}/bob-gone")).await;
        device(&state, "did:plc:alice", &format!("{service}/alice-phone")).await;
        device(&state, "did:plc:ivan", &format!("{service}/ivan-phone")).await;

        let message = json!({
            "context_id": "c9", "title": "Afstemning", "body": "Motion One", "url": "/closed/motion-one"
        });
        let uri = "/xrpc/com.example.wiki.notifyContext";
        let (status, _) = post(router(state.clone()), uri, Some(&bob), message.clone()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a member rang the whole group"
        );

        let (status, v) = post(router(state.clone()), uri, Some(&alice), message).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            v["recipients"], 2,
            "bob's two devices; not the sender, not ivan: {v}"
        );
        assert_eq!(v["sent"], 1);
        let mut paths: Vec<String> = rang
            .lock()
            .expect("rang")
            .iter()
            .map(|r| r.0.clone())
            .collect();
        paths.sort();
        assert_eq!(paths, ["/bob-gone", "/bob-phone"]);
        assert!(
            rang.lock().expect("rang").iter().all(|r| r.1 > 86),
            "an empty message"
        );

        let mut rows = conn
            .query(
                "SELECT count(*) FROM push_subscription WHERE endpoint LIKE '%gone'",
                (),
            )
            .await
            .expect("q");
        let left: i64 = rows
            .next()
            .await
            .expect("next")
            .expect("row")
            .get(0)
            .expect("n");
        assert_eq!(left, 0, "a device the service says is gone is still kept");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reply_rings_the_author_from_inside_the_context_only() {
        let (service, rang) = push_service().await;
        let state = configured().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "UPDATE document SET owner_did = 'did:plc:alice' WHERE id = 's1'",
            (),
        )
        .await
        .expect("owner");
        device(&state, "did:plc:alice", &format!("{service}/alice-phone")).await;
        let uri = "/xrpc/com.example.wiki.notifyReply";
        let reply = json!({"parent": "s1", "body": "bob svarede", "url": "/closed/secret_minutes"});

        let mallory = token_for(&state, "did:plc:mallory").await;
        let (status, _) = post(router(state.clone()), uri, Some(&mallory), reply.clone()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "a stranger pinged an author");
        let alice = token_for(&state, "did:plc:alice").await;
        let (_, v) = post(router(state.clone()), uri, Some(&alice), reply.clone()).await;
        assert_eq!(v["recipients"], 0, "told about her own reply: {v}");
        assert!(rang.lock().expect("rang").is_empty());

        let bob = token_for(&state, "did:plc:bob").await;
        let (status, v) = post(router(state.clone()), uri, Some(&bob), reply).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            (v["recipients"].as_u64(), v["sent"].as_u64()),
            (Some(1), Some(1))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_device_subscribes_and_the_next_person_to_sign_in_takes_it_over() {
        let state = configured().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let alice = token_for(&state, "did:plc:alice").await;
        let uri = "/xrpc/com.example.wiki.subscribePush";
        let endpoint = format!("https://{}/send/device-1", "push.example");
        let sub = json!({"endpoint": endpoint, "p256dh": UA_PUBLIC, "auth": AUTH});

        assert_eq!(
            post(router(state.clone()), uri, Some(&bob), sub.clone())
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            post(router(state.clone()), uri, Some(&alice), sub.clone())
                .await
                .0,
            StatusCode::OK
        );
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query("SELECT did FROM push_subscription", ())
            .await
            .expect("q");
        let owner: String = rows
            .next()
            .await
            .expect("next")
            .expect("row")
            .get(0)
            .expect("did");
        assert_eq!(owner, "did:plc:alice");
        assert!(
            rows.next().await.expect("next").is_none(),
            "one device, two rows"
        );

        for (why, bad) in [
            (
                "loopback",
                json!({"endpoint": "https://127.0.0.1/x", "p256dh": UA_PUBLIC, "auth": AUTH}),
            ),
            (
                "a short key",
                json!({"endpoint": endpoint, "p256dh": "AAAA", "auth": AUTH}),
            ),
            (
                "a short secret",
                json!({"endpoint": endpoint, "p256dh": UA_PUBLIC, "auth": "AAAA"}),
            ),
        ] {
            let (status, v) = post(router(state.clone()), uri, Some(&bob), bad).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        }
        assert_eq!(
            post(router(state.clone()), uri, None, sub).await.0,
            StatusCode::UNAUTHORIZED
        );

        let gone = "/xrpc/com.example.wiki.unsubscribePush";
        let (status, _) = post(
            router(state.clone()),
            gone,
            Some(&bob),
            json!({"endpoint": endpoint}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut rows = conn
            .query("SELECT count(*) FROM push_subscription", ())
            .await
            .expect("q");
        let left: i64 = rows
            .next()
            .await
            .expect("next")
            .expect("row")
            .get(0)
            .expect("n");
        assert_eq!(left, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_notification_leads_into_the_app_and_nowhere_else() {
        let mut state = configured().await;
        state.config.frontend_origins = vec!["https://wiki.example".to_string()];
        let says = |url: &str| Message {
            title: None,
            body: "x".to_string(),
            url: url.to_string(),
        };
        for fine in ["", "/closed/motion-one", "https://wiki.example/closed"] {
            assert!(says(fine).payload(&state.config).is_ok(), "{fine}");
        }
        for elsewhere in [
            "https://evil.example/",
            "//evil.example/",
            "javascript:alert(1)",
        ] {
            assert!(
                says(elsewhere).payload(&state.config).is_err(),
                "{elsewhere}"
            );
        }
        let long = Message {
            title: None,
            body: "x".repeat(MAX_PAYLOAD),
            url: String::new(),
        };
        assert!(long.payload(&state.config).is_err());

        // And with no key, nothing pretends to have been sent.
        state.config.vapid_private = Secret::default();
        let alice = token_for(&state, "did:plc:alice").await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.notifyContext",
            Some(&alice),
            json!({"context_id": "c9", "body": "x"}),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    }
}
