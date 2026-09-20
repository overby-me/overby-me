//! Saying which application is asking (proposal 0016, "App access"). A space
//! that admits applications by a list has each prove which one it is when it
//! asks for a credential: a token signed by a key the application publishes
//! where its `client_id` points (`jwks` or `jwks_uri` of its client metadata),
//! addressed to the space's authority and good once, for a minute.

use crate::jwt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The key an application attests with. Kept, where a DPoP key is thrown away:
/// what it proves is that whoever asks holds the key the `client_id` publishes.
pub struct ClientKey(p256::ecdsa::SigningKey);

impl ClientKey {
    /// From 32 bytes of the holder's own keeping. `None` for the few that are no
    /// P-256 scalar, for the caller to derive another.
    pub fn from_seed(seed: &[u8; 32]) -> Option<ClientKey> {
        p256::ecdsa::SigningKey::from_slice(seed)
            .ok()
            .map(ClientKey)
    }

    /// The public half, as the set of keys a `jwks_uri` serves.
    pub fn jwks(&self) -> Value {
        let point = self.0.verifying_key().to_encoded_point(false);
        let coordinate = |bytes: Option<&p256::FieldBytes>| {
            jwt::b64(bytes.map(|b| b.as_slice()).unwrap_or_default())
        };
        let (x, y) = (coordinate(point.x()), coordinate(point.y()));
        json!({"keys": [{
            "kty": "EC", "crv": "P-256", "x": x, "y": y,
            "kid": self.kid(), "alg": "ES256", "use": "sig",
        }]})
    }

    /// RFC 7638, so that the name of a key follows from the key.
    pub fn kid(&self) -> String {
        let point = self.0.verifying_key().to_encoded_point(false);
        let coordinate = |bytes: Option<&p256::FieldBytes>| {
            jwt::b64(bytes.map(|b| b.as_slice()).unwrap_or_default())
        };
        let canonical = format!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            coordinate(point.x()),
            coordinate(point.y())
        );
        jwt::b64(&Sha256::digest(canonical.as_bytes()))
    }

    /// That `client_id` is asking `authority` (a space's DID) for a credential.
    pub fn attest(&self, client_id: &str, authority: &str) -> String {
        let now = jwt::now();
        jwt::sign_es256(
            &json!({"typ": "atproto-client-attestation+jwt", "alg": "ES256", "kid": self.kid()}),
            &json!({
                "iss": client_id, "sub": client_id,
                "aud": format!("{authority}#atproto_space_host"),
                "iat": now, "exp": now + 60, "jti": jwt::nonce(),
            }),
            &self.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    #[test]
    fn an_attestation_is_the_clients_word_to_one_authority() {
        let key = ClientKey::from_seed(&[7u8; 32]).expect("a scalar");
        let client = "https://wiki.test/client-metadata.json";
        let said = jwt::decode(&key.attest(client, "did:plc:org")).expect("a token");
        assert_eq!(said.header["typ"], "atproto-client-attestation+jwt");
        assert_eq!(said.header["kid"], key.kid().as_str());
        assert_eq!(
            (said.claim("iss"), said.claim("sub")),
            (Some(client), Some(client))
        );
        assert_eq!(said.claim("aud"), Some("did:plc:org#atproto_space_host"));
        let lasts =
            said.claims["exp"].as_u64().expect("exp") - said.claims["iat"].as_u64().expect("iat");
        assert_eq!(lasts, 60);

        // By the key its JWKS publishes, under the name the token gives it.
        let jwks = key.jwks();
        let published = &jwks["keys"][0];
        assert_eq!(published["kid"], said.header["kid"]);
        assert!(published.get("d").is_none(), "the public half only");
        let point = |name: &str| {
            use base64::Engine;
            let text = published[name].as_str().expect("a coordinate");
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(text)
                .expect("base64url")
        };
        let encoded = p256::EncodedPoint::from_affine_coordinates(
            p256::FieldBytes::from_slice(&point("x")),
            p256::FieldBytes::from_slice(&point("y")),
            false,
        );
        let verifying = p256::ecdsa::VerifyingKey::from_encoded_point(&encoded).expect("a key");
        assert!(said.signed_by(&keys::p256_multikey(&verifying)));
        // The same seed is the same key, which is what lets one be derived.
        assert_eq!(
            ClientKey::from_seed(&[7u8; 32]).expect("a scalar").kid(),
            key.kid()
        );
        assert_eq!(ClientKey::from_seed(&[0u8; 32]).map(|k| k.kid()), None);
    }
}
