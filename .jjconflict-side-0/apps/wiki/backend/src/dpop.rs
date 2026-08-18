//! DPoP (RFC 9449) for atproto OAuth: a per-link P-256 key that signs DPoP proof
//! JWTs (ES256). The same key is used for PAR, the token exchange and any bound
//! request, so it is carried (as its 32-byte scalar) through the link state.

use crate::util;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub struct DpopKey {
    signing: SigningKey,
}

impl DpopKey {
    pub fn generate() -> Self {
        DpopKey {
            signing: SigningKey::random(&mut rand::rngs::OsRng),
        }
    }

    /// The 32-byte private scalar, for stashing in the (encrypted) link state.
    pub fn to_scalar_bytes(&self) -> Vec<u8> {
        self.signing.to_bytes().to_vec()
    }

    pub fn from_scalar_bytes(b: &[u8]) -> Result<Self, String> {
        SigningKey::from_slice(b)
            .map(|signing| DpopKey { signing })
            .map_err(|e| e.to_string())
    }

    /// The public key as a JOSE JWK `{kty, crv, x, y}` for the DPoP header.
    pub fn public_jwk(&self) -> Value {
        let point = self.signing.verifying_key().to_encoded_point(false);
        let x = point.x().expect("P-256 point has x");
        let y = point.y().expect("P-256 point has y");
        json!({
            "kty": "EC",
            "crv": "P-256",
            "x": util::b64url(x),
            "y": util::b64url(y),
        })
    }

    /// A DPoP proof JWT for the `htm` method + `htu` URL, optionally carrying a
    /// server-issued `nonce` and an access-token hash (`ath`, for bound calls).
    pub fn proof(
        &self,
        htm: &str,
        htu: &str,
        nonce: Option<&str>,
        access_token: Option<&str>,
    ) -> String {
        let header = json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": self.public_jwk() });
        let mut claims = json!({
            "jti": util::random_token(16),
            "htm": htm,
            "htu": htu,
            "iat": util::now_secs(),
        });
        if let Some(n) = nonce {
            claims["nonce"] = json!(n);
        }
        if let Some(at) = access_token {
            claims["ath"] = json!(util::b64url(&Sha256::digest(at.as_bytes())));
        }
        let signing_input = format!(
            "{}.{}",
            util::b64url(header.to_string().as_bytes()),
            util::b64url(claims.to_string().as_bytes())
        );
        let sig: Signature = self.signing.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", util::b64url(&sig.to_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_roundtrip_preserves_public_key() {
        let k = DpopKey::generate();
        let k2 = DpopKey::from_scalar_bytes(&k.to_scalar_bytes()).unwrap();
        assert_eq!(k.public_jwk(), k2.public_jwk());
    }

    #[test]
    fn proof_has_three_parts_and_dpop_header() {
        let k = DpopKey::generate();
        let proof = k.proof("POST", "https://pds.example/token", Some("n1"), None);
        let parts: Vec<&str> = proof.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: Value =
            serde_json::from_slice(&util::b64url_decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["typ"], "dpop+jwt");
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["jwk"]["crv"], "P-256");
        let claims: Value =
            serde_json::from_slice(&util::b64url_decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["htm"], "POST");
        assert_eq!(claims["nonce"], "n1");
    }
}
