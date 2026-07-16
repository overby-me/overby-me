//! PKCE (RFC 7636), S256 method: a random `code_verifier` and its
//! `code_challenge = base64url(sha256(verifier))`.

use crate::util;
use sha2::{Digest, Sha256};

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn generate() -> Pkce {
    // 32 bytes of entropy -> a 43-char verifier (RFC 7636 allows 43..128).
    let verifier = util::random_token(32);
    let challenge = challenge_for(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

pub fn challenge_for(verifier: &str) -> String {
    util::b64url(&Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_sha256_b64url_of_verifier() {
        let p = generate();
        assert_eq!(p.challenge, challenge_for(&p.verifier));
    }

    #[test]
    fn matches_rfc7636_test_vector() {
        // RFC 7636 Appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
