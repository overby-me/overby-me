//! The compact JWTs the protocol passes around: read without trusting, checked
//! against a key from a DID document, and signed with a P-256 key of our own.

use crate::{Error, keys};
use base64::Engine;
use serde_json::Value;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

/// A token taken apart. Nothing in it is to be believed before
/// [`Decoded::signed_by`] says so.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub header: Value,
    pub claims: Value,
    signing_input: String,
    sig: Vec<u8>,
}

pub fn decode(token: &str) -> Result<Decoded, Error> {
    let bad = || Error::Malformed("not a compact JWT".into());
    let mut parts = token.split('.');
    let (Some(h), Some(c), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let json = |part: &str| -> Result<Value, Error> {
        serde_json::from_slice(&B64.decode(part).map_err(|_| bad())?).map_err(|_| bad())
    };
    Ok(Decoded {
        header: json(h)?,
        claims: json(c)?,
        signing_input: format!("{h}.{c}"),
        sig: B64.decode(s).map_err(|_| bad())?,
    })
}

impl Decoded {
    /// Whether the key `multikey` names signed this, under an algorithm the
    /// header admits to. `alg: none` and its kin verify under no key.
    pub fn signed_by(&self, multikey: &str) -> bool {
        matches!(self.header["alg"].as_str(), Some("ES256" | "ES256K"))
            && keys::verifies(multikey, self.signing_input.as_bytes(), &self.sig) == Some(true)
    }

    pub fn claim(&self, name: &str) -> Option<&str> {
        self.claims[name].as_str()
    }
}

/// Sign `claims` with ES256.
pub fn sign_es256(header: &Value, claims: &Value, key: &p256::ecdsa::SigningKey) -> String {
    use p256::ecdsa::signature::Signer;
    let input = format!(
        "{}.{}",
        b64(header.to_string().as_bytes()),
        b64(claims.to_string().as_bytes())
    );
    let sig: p256::ecdsa::Signature = key.sign(input.as_bytes());
    format!("{input}.{}", b64(&sig.to_bytes()))
}

/// Random bytes as hex, for a `jti`.
pub fn nonce() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn what_we_sign_verifies_under_our_key_and_no_other() {
        let key = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
        let other = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
        let token = sign_es256(&json!({"alg": "ES256"}), &json!({"iss": "did:plc:a"}), &key);
        let read = decode(&token).expect("a token");
        assert_eq!(read.claim("iss"), Some("did:plc:a"));
        assert!(read.signed_by(&keys::p256_multikey(key.verifying_key())));
        assert!(!read.signed_by(&keys::p256_multikey(other.verifying_key())));

        let unsigned = sign_es256(&json!({"alg": "none"}), &json!({}), &key);
        assert!(
            !decode(&unsigned)
                .expect("a token")
                .signed_by(&keys::p256_multikey(key.verifying_key())),
            "an algorithm nobody asked for"
        );
        assert!(decode("a.b").is_err() && decode("a.b.c.d").is_err());
    }
}
