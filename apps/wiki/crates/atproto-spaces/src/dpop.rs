//! DPoP (RFC 9449), as spaces use it: a space credential is bound to a key of
//! the application's, and every request with it carries a proof by that key
//! naming the host it is for, so that a host handed a credential cannot replay
//! it against the others. No server nonces.

use crate::jwt;
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// One per credential, and gone with it.
pub struct DpopKey(p256::ecdsa::SigningKey);

impl Default for DpopKey {
    fn default() -> Self {
        Self::generate()
    }
}

impl DpopKey {
    pub fn generate() -> Self {
        DpopKey(p256::ecdsa::SigningKey::random(&mut rand_core::OsRng))
    }

    pub fn jwk(&self) -> Value {
        let point = self.0.verifying_key().to_encoded_point(false);
        let coordinate = |bytes: Option<&p256::FieldBytes>| {
            jwt::b64(bytes.map(|b| b.as_slice()).unwrap_or_default())
        };
        json!({"kty": "EC", "crv": "P-256", "x": coordinate(point.x()), "y": coordinate(point.y())})
    }

    /// RFC 7638: what a credential's `cnf.jkt` is compared with.
    pub fn thumbprint(&self) -> String {
        let jwk = self.jwk();
        let canonical = format!(
            r#"{{"crv":"P-256","kty":"EC","x":{},"y":{}}}"#,
            jwk["x"], jwk["y"]
        );
        jwt::b64(&Sha256::digest(canonical.as_bytes()))
    }

    /// A proof for one request. `bound_to` is the credential the request
    /// carries, absent only when asking for one.
    pub fn proof(&self, method: &str, url: &str, bound_to: Option<&str>) -> String {
        let mut claims = json!({
            "jti": jwt::nonce(),
            "htm": method,
            "htu": htu(url),
            "iat": jwt::now(),
        });
        if let Some(credential) = bound_to {
            claims["ath"] = Value::String(
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(Sha256::digest(credential.as_bytes())),
            );
        }
        let header = json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": self.jwk()});
        jwt::sign_es256(&header, &claims, &self.0)
    }
}

/// The URL a proof names: scheme, host and path, with no query and no fragment.
fn htu(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    #[test]
    fn a_proof_names_the_request_and_not_its_query() {
        let key = DpopKey::generate();
        let proof = key.proof(
            "GET",
            "https://pds.example/xrpc/com.atproto.space.getRepo?space=at%3A%2F%2Fx",
            Some("the-credential"),
        );
        let read = jwt::decode(&proof).expect("a jwt");
        assert_eq!(read.header["typ"], "dpop+jwt");
        assert_eq!(
            read.claim("htu"),
            Some("https://pds.example/xrpc/com.atproto.space.getRepo")
        );
        assert_eq!(read.claim("htm"), Some("GET"));
        assert!(read.claim("ath").is_some() && read.claim("jti").is_some());
        assert!(read.signed_by(&keys::p256_multikey(key.0.verifying_key())));
        assert_eq!(key.thumbprint().len(), 43, "32 bytes, base64url");
        assert!(
            jwt::decode(&key.proof("POST", "https://a/b", None))
                .expect("a jwt")
                .claim("ath")
                .is_none()
        );
    }

    /// The example of RFC 7638 is an RSA key; this is the P-256 one of RFC 9449
    /// (section 4.1), whose thumbprint its section 6.1 gives.
    #[test]
    fn the_thumbprint_is_the_rfcs() {
        let canonical = r#"{"crv":"P-256","kty":"EC","x":"l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs","y":"9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA"}"#;
        assert_eq!(
            jwt::b64(&Sha256::digest(canonical.as_bytes())),
            "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I"
        );
    }
}
