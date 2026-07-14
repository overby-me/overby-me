//! Background Web Push, hand-rolled on RustCrypto primitives (the `web-push` crate
//! pulls libcurl/openssl the minimal container omits).
//!
//! - Message encryption follows RFC 8291 (ECDH P-256 + HKDF-SHA256) with the
//!   `aes128gcm` content encoding of RFC 8188. Validated against the RFC 8291 §5
//!   test vector (see the tests).
//! - Authorization is the VAPID scheme of RFC 8292: an ES256 JWT (reusing the same
//!   P-256 signing as DPoP) plus the server's public key.
//!
//! The public surface is [`send`] (encrypt + VAPID + POST a payload to one push
//! subscription). Storage of subscriptions and the fan-out live in `notify.rs`.

use crate::oauth::Config;
use crate::util;
use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use serde_json::json;
use sha2::Sha256;

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

/// Whether a client-registered push endpoint is safe for the backend to POST to
/// (SSRF guard). Web-push endpoints are always `https://` on public push
/// infrastructure with a DNS hostname (`fcm.googleapis.com`,
/// `*.push.services.mozilla.com`, `*.notify.windows.com`, `web.push.apple.com`),
/// so we reject anything that is not https, any IP-literal host (which blocks
/// the cloud metadata service, loopback, and private ranges), and localhost.
/// This rejects no legitimate browser subscription. Residual (accepted): a
/// hostname that DNS-resolves to an internal address (rebinding) still passes.
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
    // Reject IP-literal hosts outright (real push endpoints use DNS names).
    if host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    true
}

/// The `scheme://host[:port]` origin of a push endpoint, for the VAPID `aud` claim.
fn origin_of(endpoint: &str) -> Result<String, String> {
    let rest = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .ok_or("endpoint is not http(s)")?;
    let scheme = if endpoint.starts_with("https://") {
        "https"
    } else {
        "http"
    };
    let host = rest.split('/').next().unwrap_or(rest);
    Ok(format!("{scheme}://{host}"))
}

/// A VAPID (RFC 8292) `Authorization` header value for `endpoint`: an ES256 JWT
/// (`aud` = the endpoint origin, `exp` ~12h out, `sub` = a contact) plus the
/// server's public key.
fn vapid_header(cfg: &Config, endpoint: &str, now: u64) -> Result<String, String> {
    let scalar = util::b64url_decode(&cfg.vapid_private)?;
    let signing = SigningKey::from_slice(&scalar).map_err(|e| e.to_string())?;
    let header = json!({ "typ": "JWT", "alg": "ES256" });
    let claims = json!({
        "aud": origin_of(endpoint)?,
        "exp": now + 12 * 3600,
        "sub": cfg.vapid_subject,
    });
    let signing_input = format!(
        "{}.{}",
        util::b64url(header.to_string().as_bytes()),
        util::b64url(claims.to_string().as_bytes())
    );
    let sig: Signature = signing.sign(signing_input.as_bytes());
    let jwt = format!("{signing_input}.{}", util::b64url(&sig.to_bytes()));
    Ok(format!("vapid t={jwt}, k={}", cfg.vapid_public))
}

/// Encrypt `payload` (an app-defined JSON string) for `sub` and POST it to the
/// push service. Returns the HTTP status; 404/410 means the subscription is gone
/// and the caller should drop it.
pub async fn send(
    cfg: &Config,
    client: &reqwest::Client,
    sub: &Subscription,
    payload: &[u8],
) -> Result<u16, String> {
    if cfg.vapid_private.is_empty() {
        return Err("push not configured".into());
    }
    // SSRF guard at send time too, in case a disallowed endpoint predates the
    // subscribe-time check (defense in depth).
    if !endpoint_allowed(&sub.endpoint) {
        return Err("disallowed push endpoint".into());
    }
    let body = encrypt(&sub.p256dh, &sub.auth, payload)?;
    let auth = vapid_header(cfg, &sub.endpoint, util::now_secs())?;
    let resp = client
        .post(&sub.endpoint)
        .header("TTL", "86400")
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Urgency", "normal")
        .header(http::header::AUTHORIZATION, auth)
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 8291 §5 "Push Message Encryption Example".
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

    #[test]
    fn endpoint_allowlist_blocks_ssrf_targets() {
        // Real browser push endpoints (https, DNS host) are allowed.
        assert!(endpoint_allowed(
            "https://fcm.googleapis.com/fcm/send/abc123"
        ));
        assert!(endpoint_allowed(
            "https://updates.push.services.mozilla.com/wpush/v2/xyz"
        ));
        assert!(endpoint_allowed("https://web.push.apple.com/QA/def"));

        // SSRF targets and non-https are rejected.
        assert!(!endpoint_allowed("http://fcm.googleapis.com/fcm/send/abc")); // not https
        assert!(!endpoint_allowed(
            "https://169.254.169.254/latest/meta-data/"
        )); // metadata
        assert!(!endpoint_allowed("https://127.0.0.1/internal"));
        assert!(!endpoint_allowed("https://10.0.0.5/x"));
        assert!(!endpoint_allowed("https://192.168.1.1/x"));
        assert!(!endpoint_allowed("https://localhost/x"));
        assert!(!endpoint_allowed("https://[::1]/x")); // IPv6 loopback
        assert!(!endpoint_allowed("ftp://fcm.googleapis.com/x"));
    }

    #[test]
    fn rfc8291_example_matches() {
        let as_secret = SecretKey::from_slice(&util::b64url_decode(AS_PRIVATE).unwrap()).unwrap();
        // The fixed private key must derive the RFC's public key.
        let point = as_secret.public_key().to_encoded_point(false);
        assert_eq!(util::b64url(point.as_bytes()), AS_PUBLIC);

        let body = encrypt_with(
            &as_secret,
            &util::b64url_decode(SALT).unwrap(),
            &util::b64url_decode(UA_PUBLIC).unwrap(),
            &util::b64url_decode(AUTH).unwrap(),
            PLAINTEXT.as_bytes(),
        )
        .unwrap();
        assert_eq!(util::b64url(&body), EXPECTED);
    }

    #[test]
    fn header_frames_salt_rs_and_keyid() {
        let as_secret = SecretKey::from_slice(&util::b64url_decode(AS_PRIVATE).unwrap()).unwrap();
        let body = encrypt_with(
            &as_secret,
            &util::b64url_decode(SALT).unwrap(),
            &util::b64url_decode(UA_PUBLIC).unwrap(),
            &util::b64url_decode(AUTH).unwrap(),
            PLAINTEXT.as_bytes(),
        )
        .unwrap();
        assert_eq!(&body[0..16], util::b64url_decode(SALT).unwrap().as_slice());
        assert_eq!(&body[16..20], &4096u32.to_be_bytes()); // record size
        assert_eq!(body[20], 65); // key id length
        assert_eq!(
            &body[21..86],
            util::b64url_decode(AS_PUBLIC).unwrap().as_slice()
        );
    }

    #[test]
    fn origin_strips_path() {
        // localhost keeps the link checker out of a would-be real push endpoint.
        assert_eq!(
            origin_of("https://localhost/wpush/v2/abc").unwrap(),
            "https://localhost"
        );
    }
}
