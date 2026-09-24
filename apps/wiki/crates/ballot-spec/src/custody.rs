//! What the board's custodian signs, and how anyone checks it.
//!
//! The board is published by the organization, which is what lets it be
//! anonymous (`docs/ballot-board-custody.md`) and what lets a ballot be dropped
//! or rewritten by the one party every ballot passes through. Two signatures
//! turn that from silent into provable:
//!
//! - a [`Receipt`] for every ballot, handed to its voter as it lands: a board
//!   without that entry, or with other choices under that token, contradicts
//!   the custodian's own signature;
//! - a [`CloseOut`] at the close: what the board was, by digest, and what it
//!   came to. A tally is official once this is signed, and a recount is checked
//!   against it.
//!
//! Both are signed by a key kept for nothing else (not a poll's issuer key,
//! whose use is blind-signing tokens), named as a `did:key`. What is signed is a
//! line-per-field text, so that a verifier in any language rebuilds the exact
//! bytes without a canonical-JSON rule to get wrong.

use crate::provisional::ProvisionalEntry;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const RECEIPT_TAG: &str = "wiki-ballot-receipt-v1";
const CLOSEOUT_TAG: &str = "wiki-poll-closeout-v1";

/// The multicodec prefix of a compressed P-256 public key, as `did:key` has it.
const P256_PUB: [u8; 2] = [0x80, 0x24];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn joined(choices: &[usize]) -> String {
    let each: Vec<String> = choices.iter().map(usize::to_string).collect();
    each.join(",")
}

/// The address of one entry's content: every field a voter chose or was given,
/// so that a receipt naming it commits the custodian to all of them.
pub fn entry_digest(entry: &ProvisionalEntry) -> String {
    let text = format!(
        "{}\n{}\n{}\n{}",
        entry.token,
        entry.msg_randomizer.as_deref().unwrap_or(""),
        entry.signature,
        joined(&entry.choices)
    );
    hex(&Sha256::digest(text.as_bytes()))
}

/// The address of a whole board: its entries by token, which is an order anyone
/// can put them in, whatever order they were cast or published in.
pub fn board_digest(entries: &[ProvisionalEntry]) -> String {
    let mut by_token: Vec<&ProvisionalEntry> = entries.iter().collect();
    by_token.sort_by(|a, b| a.token.cmp(&b.token));
    let mut all = Sha256::new();
    for entry in by_token {
        all.update(entry_digest(entry).as_bytes());
        all.update(b"\n");
    }
    hex(&all.finalize())
}

/// The custodian's word that one ballot is on the board, saying what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub poll: String,
    /// Where on the board it landed.
    pub position: u64,
    pub token: String,
    pub choices: Vec<usize>,
    pub entry_digest: String,
    /// To the minute: enough for a dispute, too coarse to pair with a log.
    pub at: String,
}

impl Receipt {
    pub fn payload(&self) -> String {
        format!(
            "{RECEIPT_TAG}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.poll,
            self.position,
            self.token,
            joined(&self.choices),
            self.entry_digest,
            self.at
        )
    }
}

/// The custodian's word on what a board was when it closed and what it came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseOut {
    pub poll: String,
    pub entries: u64,
    /// Unit tokens handed out: the bound on `entries` that anyone can check.
    pub issued: u64,
    pub counts: Vec<u64>,
    pub board_digest: String,
    pub closed_at: String,
}

impl CloseOut {
    pub fn payload(&self) -> String {
        let counts: Vec<String> = self.counts.iter().map(u64::to_string).collect();
        format!(
            "{CLOSEOUT_TAG}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.poll,
            self.entries,
            self.issued,
            counts.join(","),
            self.board_digest,
            self.closed_at
        )
    }
}

/// Whoever holds the custody key.
pub struct Custodian(SigningKey);

impl Custodian {
    /// From 32 bytes of key material. `None` for the vanishing few that are no
    /// valid P-256 scalar: derive another.
    pub fn from_seed(seed: &[u8; 32]) -> Option<Self> {
        SigningKey::from_slice(seed).ok().map(Custodian)
    }

    /// The key as anyone names it: `did:key:zDn...`.
    pub fn did_key(&self) -> String {
        let point = self.0.verifying_key().to_encoded_point(true);
        let mut bytes = P256_PUB.to_vec();
        bytes.extend_from_slice(point.as_bytes());
        format!(
            "did:key:{}",
            multibase::encode(multibase::Base::Base58Btc, bytes)
        )
    }

    /// Sign a payload: ECDSA over SHA-256, `r || s`, base64url without padding.
    pub fn sign(&self, payload: &str) -> String {
        use base64::Engine;
        let signature: Signature = self.0.sign(payload.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes())
    }
}

/// Whether `signature` is `did_key`'s over `payload`.
pub fn verify(did_key: &str, payload: &str, signature: &str) -> bool {
    use base64::Engine;
    let checked = || {
        let (_, bytes) = multibase::decode(did_key.strip_prefix("did:key:")?).ok()?;
        let point = bytes.strip_prefix(&P256_PUB)?;
        let key = VerifyingKey::from_sec1_bytes(point).ok()?;
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature)
            .ok()?;
        let signature = Signature::from_slice(&raw).ok()?;
        key.verify(payload.as_bytes(), &signature).ok()
    };
    checked().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(token: &str, choices: &[usize]) -> ProvisionalEntry {
        ProvisionalEntry {
            token: token.to_string(),
            msg_randomizer: Some("cmFuZA".to_string()),
            signature: "c2ln".to_string(),
            choices: choices.to_vec(),
        }
    }

    fn custodian() -> Custodian {
        Custodian::from_seed(&[7u8; 32]).expect("a valid scalar")
    }

    #[test]
    fn a_receipt_is_the_custodians_and_nobody_elses() {
        let custodian = custodian();
        let ballot = entry("dG9rZW4", &[1]);
        let receipt = Receipt {
            poll: "d-poll".into(),
            position: 4,
            token: ballot.token.clone(),
            choices: ballot.choices.clone(),
            entry_digest: entry_digest(&ballot),
            at: "2026-05-01T18:30Z".into(),
        };
        let key = custodian.did_key();
        assert!(key.starts_with("did:key:zDn"), "a P-256 did:key: {key}");
        let signature = custodian.sign(&receipt.payload());
        assert!(verify(&key, &receipt.payload(), &signature));

        // Other choices under the same token is not what was signed.
        let swapped = Receipt {
            choices: vec![0],
            ..receipt.clone()
        };
        assert!(!verify(&key, &swapped.payload(), &signature));
        let another = Custodian::from_seed(&[8u8; 32]).expect("scalar").did_key();
        assert!(!verify(&another, &receipt.payload(), &signature));
        assert!(!verify("did:key:zNotAKey", &receipt.payload(), &signature));
        assert!(!verify(&key, &receipt.payload(), "not-a-signature"));
    }

    #[test]
    fn an_entry_is_addressed_by_everything_in_it() {
        let ballot = entry("dG9rZW4", &[1]);
        let same = entry_digest(&ballot);
        assert_eq!(same, entry_digest(&ballot.clone()));
        assert_ne!(same, entry_digest(&entry("dG9rZW4", &[0])), "its choices");
        assert_ne!(same, entry_digest(&entry("b3RoZXI", &[1])), "its token");
        let unrandomized = ProvisionalEntry {
            msg_randomizer: None,
            ..ballot
        };
        assert_ne!(same, entry_digest(&unrandomized));
    }

    /// Ballots are published shuffled, and a mirror reads them in yet another
    /// order: the board they make is the same board.
    #[test]
    fn a_board_is_the_same_board_in_any_order() {
        let (a, b, c) = (entry("YQ", &[0]), entry("Yg", &[1]), entry("Yw", &[0]));
        let cast = board_digest(&[a.clone(), b.clone(), c.clone()]);
        assert_eq!(cast, board_digest(&[c.clone(), a.clone(), b.clone()]));
        assert_ne!(cast, board_digest(&[a.clone(), b.clone()]), "one dropped");
        let rewritten = entry("Yw", &[1]);
        assert_ne!(cast, board_digest(&[a, b, rewritten]), "one rewritten");
        assert_ne!(board_digest(&[]), cast);
    }

    #[test]
    fn a_close_out_pins_the_count_and_the_board() {
        let custodian = custodian();
        let close = CloseOut {
            poll: "d-poll".into(),
            entries: 3,
            issued: 4,
            counts: vec![2, 1, 0],
            board_digest: board_digest(&[entry("YQ", &[0])]),
            closed_at: "2026-05-01T19:00:00.000Z".into(),
        };
        let signature = custodian.sign(&close.payload());
        assert!(verify(&custodian.did_key(), &close.payload(), &signature));
        let recounted = CloseOut {
            counts: vec![1, 2, 0],
            ..close.clone()
        };
        assert!(!verify(
            &custodian.did_key(),
            &recounted.payload(),
            &signature
        ));
    }
}
