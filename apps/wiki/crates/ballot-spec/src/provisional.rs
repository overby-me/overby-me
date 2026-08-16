//! PROVISIONAL byte-level encoding of a board entry's crypto (decision D7 marks
//! the wire encodings as pending). This pins a *candidate* format --
//! base64url-unpadded token / message-randomizer / signature plus the plain
//! choice indices -- with a known-answer vector, so the eventual `ballotEntry`
//! lexicon and the board publish seam have a concrete, testable serialization to
//! ratify. It is NOT final until the owner signs off D7 and the board-custody
//! batch; the issuer public key's DER SPKI encoding is the remaining field to
//! pin (blocked on choosing the pubkey-distribution channel).
//!
//! The base64url-unpadded (RFC 4648 sec. 5, no `=` padding) choice mirrors how
//! atproto already encodes bytes in JSON-ish records, so a board entry can ride
//! an XRPC body or a lexicon record without a second encoding hop.

use crate::BoardEntry;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};

/// The provisional on-the-wire shape of a board entry's crypto: base64url-unpadded
/// bytes plus the plain option indices. Serde-serializable so it can be embedded
/// in an XRPC body or a record without further transformation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionalEntry {
    /// base64url-unpadded of the unblinded per-poll unit token (the nullifier
    /// that dedups a voter without revealing them).
    pub token: String,
    /// base64url-unpadded of the 32-byte message randomizer, when the issuer
    /// signed in randomized mode (absent otherwise).
    pub msg_randomizer: Option<String>,
    /// base64url-unpadded of the RSA-PSS blind signature over the token.
    pub signature: String,
    /// The selected option indices (small integers, encoded as-is).
    pub choices: Vec<usize>,
}

/// Encode a board entry's crypto into the provisional wire shape. Total (no
/// data loss): every field of `BoardEntry` is represented.
pub fn encode_entry(entry: &BoardEntry) -> ProvisionalEntry {
    ProvisionalEntry {
        token: B64.encode(&entry.token),
        msg_randomizer: entry.msg_randomizer.as_ref().map(|m| B64.encode(m.0)),
        signature: B64.encode(&entry.signature.0),
        choices: entry.choices.clone(),
    }
}

/// Decode the token / randomizer / signature bytes back out (the round-trip
/// check and the importer's read path). Choices need no decoding. Returns
/// `(token, msg_randomizer_bytes, signature)`.
#[allow(clippy::type_complexity)]
pub fn decode_bytes(
    e: &ProvisionalEntry,
) -> Result<(Vec<u8>, Option<Vec<u8>>, Vec<u8>), base64::DecodeError> {
    let token = B64.decode(&e.token)?;
    let msg_randomizer = e
        .msg_randomizer
        .as_ref()
        .map(|s| B64.decode(s))
        .transpose()?;
    let signature = B64.decode(&e.signature)?;
    Ok((token, msg_randomizer, signature))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MessageRandomizer, Signature};

    /// A FIXED entry with hand-chosen bytes (NOT real crypto): the known-answer
    /// vector's input, so the encoding is deterministic and reviewable.
    fn fixed_entry() -> BoardEntry {
        BoardEntry {
            token: vec![0xDE, 0xAD, 0xBE, 0xEF],
            msg_randomizer: Some(MessageRandomizer([0x01; 32])),
            signature: Signature(vec![0x00, 0xFF, 0x10]),
            choices: vec![0, 2],
        }
    }

    /// Known-answer vector: pins the exact base64url-unpadded strings. If the
    /// encoding ever changes, this fails loudly (the whole point of pinning a
    /// provisional format before it is ratified).
    #[test]
    fn known_answer_vector() {
        let p = encode_entry(&fixed_entry());
        // 0xDEADBEEF -> "3q2-7w" (note the url-safe '-' where std base64 has '+').
        assert_eq!(p.token, "3q2-7w");
        // [0x00, 0xFF, 0x10] -> "AP8Q".
        assert_eq!(p.signature, "AP8Q");
        // 32 bytes of 0x01 -> 43 chars, no padding.
        assert_eq!(
            p.msg_randomizer.as_deref(),
            Some("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE")
        );
        assert_eq!(p.choices, vec![0, 2]);
    }

    #[test]
    fn round_trips_to_bytes() {
        let e = fixed_entry();
        let p = encode_entry(&e);
        let (token, mr, sig) = decode_bytes(&p).unwrap();
        assert_eq!(token, e.token);
        assert_eq!(mr, Some(vec![0x01; 32]));
        assert_eq!(sig, e.signature.0);
    }

    #[test]
    fn no_randomizer_encodes_to_none() {
        let mut e = fixed_entry();
        e.msg_randomizer = None;
        let p = encode_entry(&e);
        assert_eq!(p.msg_randomizer, None);
        let (_, mr, _) = decode_bytes(&p).unwrap();
        assert_eq!(mr, None);
    }
}
