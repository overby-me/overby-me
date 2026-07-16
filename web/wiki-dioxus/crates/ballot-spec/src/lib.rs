//! Executable specification of the decided E2E-verifiable ballot scheme:
//! RFC 9474 RSA blind signatures (`blind-rsa-signatures`, the RSABSSA
//! SHA-384/PSS/Randomized variant), UNIT tokens, PER-POLL issuer keys, and an
//! append-only public bulletin board (kept ABSTRACT here: a plain sequence, no
//! atproto record/CID shapes, which are item 9's job and stay provisional).
//!
//! This crate is pure domain and crypto: no interim types, no I/O, no store.
//! It becomes the AppView's ballot core verbatim; the property tests in
//! `tests/` are its permanent conformance suite. The ballot validity and
//! outcome rules transcribe the SHIPPED assembly semantics from the interim
//! frontend (`src/components/vote/poll.rs`), so today's real rules transfer
//! instead of being re-guessed.
//!
//! Semantics this crate pins that no design doc decides are recorded in
//! `DECISIONS.md` (pending owner sign-off), not silently frozen in tests.

pub use blind_rsa_signatures::{
    BlindSignature, BlindingResult, DefaultRng, MessageRandomizer, PSS, Signature,
};
use blind_rsa_signatures::{KeyPair, PublicKey, Randomized, SecretKey, Sha384};
use std::collections::{BTreeMap, BTreeSet};

/// A voter's decentralized identifier. Ordered so rosters iterate
/// deterministically (proptest shrinking and tally reproducibility).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Did(pub String);

// ---------------------------------------------------------------------------
// Eligibility + delegation (org-authoritative, always private)
// ---------------------------------------------------------------------------

/// The per-poll eligibility roster: who may vote and with what base weight,
/// plus the signed delegation assignments to resolve BEFORE the poll opens.
/// Mirrors the `eligibility` and `delegation` tables in the domain model.
#[derive(Debug, Clone, Default)]
pub struct EligibilityRoster {
    /// Base weight per eligible voter (0 is allowed and yields no tokens).
    pub base_weight: BTreeMap<Did, u64>,
    /// One outgoing delegation per voter (the `delegation` table's PK).
    /// Signature verification of the assignment happens upstream; the roster
    /// only resolves weights.
    pub delegation: BTreeMap<Did, Did>,
}

/// Weights after delegation resolution, FROZEN at poll open. A voter's
/// resolved weight is the number of unit tokens they are issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoster {
    pub resolved_weight: BTreeMap<Did, u64>,
}

impl EligibilityRoster {
    /// Resolve delegations into per-voter weights (DECISIONS.md D1 to D3):
    /// a delegation chain is followed transitively to its terminal
    /// non-delegating voter; a chain that revisits a voter (cycle) or ends at
    /// an ineligible DID is VOID for the walking voter (they keep their own
    /// weight); a voter whose delegation resolves ends with weight 0.
    /// Deterministic and terminating: each walk visits a voter at most once.
    ///
    /// Weight is CONSERVED: the sum of resolved weights always equals the sum
    /// of base weights (a void delegation keeps weight in place; a resolved
    /// one moves it). This is the property-tested invariant.
    pub fn resolve(&self) -> ResolvedRoster {
        let mut resolved: BTreeMap<Did, u64> =
            self.base_weight.keys().map(|d| (d.clone(), 0)).collect();
        for (voter, base) in &self.base_weight {
            let terminal = self.terminal_of(voter);
            *resolved
                .entry(terminal.unwrap_or_else(|| voter.clone()))
                .or_insert(0) += base;
        }
        ResolvedRoster {
            resolved_weight: resolved,
        }
    }

    /// The terminal voter `start`'s weight lands on, or None if the chain is
    /// void (cycle, or a hop to an ineligible DID), which keeps the weight
    /// with `start`.
    fn terminal_of(&self, start: &Did) -> Option<Did> {
        let mut seen: BTreeSet<&Did> = BTreeSet::new();
        let mut cur = start;
        loop {
            if !seen.insert(cur) {
                return None; // cycle: void, weight stays with `start`
            }
            match self.delegation.get(cur) {
                None => return Some(cur.clone()),
                Some(next) => {
                    if !self.base_weight.contains_key(next) {
                        return None; // delegate not eligible: void
                    }
                    cur = next;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ballot validity + outcome (transcribed from the shipped frontend)
// ---------------------------------------------------------------------------

/// A poll's choice rules. `options` INCLUDES the trailing "Blank" abstention
/// when `blank` is true (the shipped ballot always appends it last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BallotRules {
    /// Total number of options, including the trailing blank when present.
    pub options: usize,
    /// Minimum selections for a non-blank ballot (`minVote`).
    pub min: usize,
    /// Maximum selections (`maxVote`).
    pub max: usize,
    /// Whether the last option is the "Blank" abstention.
    pub blank: bool,
}

/// Why a ballot's choice set is invalid (poll.rs:358-372 semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidBallot {
    /// An index is out of range or repeated.
    BadIndex,
    /// Blank (the last option) was combined with other selections.
    BlankNotAlone,
    /// Fewer than `min` selections (and not a lone blank, which bypasses min).
    TooFew,
    /// More than `max` selections.
    TooMany,
}

impl BallotRules {
    /// Validate a ballot's selected option indices, exactly as the shipped
    /// ballot does: indices in range and unique; blank only alone; a lone
    /// blank bypasses `min`; otherwise `min <= chosen <= max`.
    pub fn validate(&self, choices: &[usize]) -> Result<(), InvalidBallot> {
        let mut seen = BTreeSet::new();
        for &c in choices {
            if c >= self.options || !seen.insert(c) {
                return Err(InvalidBallot::BadIndex);
            }
        }
        let blank_idx = self.options.saturating_sub(1);
        let contains_blank = self.blank && choices.contains(&blank_idx);
        if contains_blank && choices.len() > 1 {
            return Err(InvalidBallot::BlankNotAlone);
        }
        let blank_alone = contains_blank && choices.len() == 1;
        if !blank_alone && choices.len() < self.min {
            return Err(InvalidBallot::TooFew);
        }
        if choices.len() > self.max {
            return Err(InvalidBallot::TooMany);
        }
        Ok(())
    }
}

/// A poll's outcome (poll.rs:299-332 semantics): the blank abstention is
/// excluded from winning; a single strict maximum wins; two or more options
/// sharing the top non-blank count is a tie; no non-blank votes is no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Winner(usize),
    Tie,
    NoVotes,
}

/// Per-option counts for a board. Pure aggregation: always recomputed, never
/// a mutable counter, and anyone can recompute the same counts from the
/// public board (universal verifiability). Unit tokens make it a plain count.
pub fn tally(entries: &[BoardEntry], rules: &BallotRules) -> Vec<u64> {
    let mut counts = vec![0u64; rules.options];
    for e in entries {
        for &c in &e.choices {
            if c < rules.options {
                counts[c] += 1;
            }
        }
    }
    counts
}

/// The outcome of a tallied board (see [`Outcome`]).
pub fn outcome(counts: &[u64], rules: &BallotRules) -> Outcome {
    let blank_idx = counts.len().saturating_sub(1);
    let is_abstention = |i: usize| rules.blank && counts.len() > 1 && i == blank_idx;
    let max_count = counts
        .iter()
        .enumerate()
        .filter(|(i, _)| !is_abstention(*i))
        .map(|(_, c)| *c)
        .max()
        .unwrap_or(0);
    if max_count == 0 {
        return Outcome::NoVotes;
    }
    let at_max: Vec<usize> = counts
        .iter()
        .enumerate()
        .filter(|(i, c)| !is_abstention(*i) && **c == max_count)
        .map(|(i, _)| i)
        .collect();
    if at_max.len() == 1 {
        Outcome::Winner(at_max[0])
    } else {
        Outcome::Tie
    }
}

// ---------------------------------------------------------------------------
// Token issuance (per-poll issuer keys, unit tokens)
// ---------------------------------------------------------------------------

/// The RFC 9474 variant in use: RSABSSA-SHA384-PSS-Randomized (the RFC's
/// recommended parameter set; DECISIONS.md D6).
pub type IssuerKeyPair = KeyPair<Sha384, PSS, Randomized>;
pub type IssuerPublicKey = PublicKey<Sha384, PSS, Randomized>;
pub type IssuerSecretKey = SecretKey<Sha384, PSS, Randomized>;

/// A unit token's message: a random nullifier the VOTER generates and keeps
/// secret until spend. Its uniqueness on the board is the double-vote
/// rejection; it carries nothing about the voter (DECISIONS.md D7).
pub const TOKEN_NULLIFIER_LEN: usize = 32;

/// The per-poll token issuer: a FRESH keypair per poll. Blinding hides the
/// message from the issuer, so per-poll keys are the only cryptographic poll
/// binding: a token blind-signed under poll A's key can never verify under
/// poll B's. The pubkey is published to the board before the poll opens; the
/// secret key is dropped at close.
pub struct TokenIssuer {
    kp: IssuerKeyPair,
}

impl TokenIssuer {
    /// Mint the poll's issuer keypair. 2048-bit minimum per RFC 9474.
    pub fn new_for_poll(bits: usize) -> Result<Self, blind_rsa_signatures::Error> {
        let kp = IssuerKeyPair::generate(&mut DefaultRng, bits)?;
        Ok(Self { kp })
    }

    /// The pubkey to publish to the board before the poll opens.
    pub fn public_key(&self) -> &IssuerPublicKey {
        &self.kp.pk
    }

    /// Blind-sign one blinded token message. The org calls this N times for a
    /// voter with resolved weight N, AFTER recording the one-shot issuance
    /// marker (`token_issued`); it never sees the nullifier inside.
    pub fn blind_sign(
        &self,
        blind_message: impl AsRef<[u8]>,
    ) -> Result<BlindSignature, blind_rsa_signatures::Error> {
        self.kp.sk.blind_sign(blind_message)
    }
}

/// A voter-side unit token in flight: the secret nullifier plus the blinding
/// state. `blinding.blind_message` goes to the issuer; everything else stays
/// with the voter.
pub struct TokenRequest {
    /// The secret nullifier (the token's message).
    pub nullifier: Vec<u8>,
    /// Blinding state (message randomizer + unblinding secret).
    pub blinding: BlindingResult,
}

/// Generate a fresh unit-token request against the poll's issuer pubkey.
pub fn request_token(pk: &IssuerPublicKey) -> Result<TokenRequest, blind_rsa_signatures::Error> {
    let nullifier = random_nullifier();
    let blinding = pk.blind(&mut DefaultRng, &nullifier)?;
    Ok(TokenRequest {
        nullifier,
        blinding,
    })
}

/// A fresh random token nullifier from the thread CSPRNG (OS-seeded).
fn random_nullifier() -> Vec<u8> {
    use rand::Rng;
    let mut nullifier = vec![0u8; TOKEN_NULLIFIER_LEN];
    rand::rng().fill_bytes(&mut nullifier);
    nullifier
}

/// Unblind the issuer's blind signature into the spendable token signature
/// (also verifies it against the issuer pubkey).
pub fn finalize_token(
    pk: &IssuerPublicKey,
    req: &TokenRequest,
    blind_sig: &BlindSignature,
) -> Result<Signature, blind_rsa_signatures::Error> {
    pk.finalize(blind_sig, &req.blinding, &req.nullifier)
}

// ---------------------------------------------------------------------------
// The bulletin board (abstract: an append-only sequence)
// ---------------------------------------------------------------------------

/// One spent unit token on the public board. STRUCTURALLY carries no voter
/// identity: there is no DID field to leak, which is the type-level
/// no-DID-linkage guarantee. Every entry weighs exactly 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardEntry {
    /// The unblinded nullifier (the token's message).
    pub token: Vec<u8>,
    /// The message randomizer from blinding (the Randomized variant).
    pub msg_randomizer: Option<MessageRandomizer>,
    /// The unblinded RSA-PSS signature over `token`, verifying under the
    /// poll's published issuer pubkey.
    pub signature: Signature,
    /// Selected option indices.
    pub choices: Vec<usize>,
}

/// Why a cast was rejected by the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastError {
    /// The token signature does not verify under the poll's issuer pubkey
    /// (forged, or minted by a different key = a different poll).
    BadSignature,
    /// The token was already spent: the double-vote rejection. The FIRST
    /// entry stands; the second cast is rejected (DECISIONS.md D4).
    DoubleSpend,
    /// The choices violate the poll's ballot rules.
    Invalid(InvalidBallot),
}

/// The append-only board for one poll, kept deliberately abstract (a plain
/// sequence). How entries become atproto records, and in whose repo (custody),
/// is item 9's design and does not belong in the spec.
#[derive(Default)]
pub struct Board {
    entries: Vec<BoardEntry>,
}

impl Board {
    /// Verify and append a cast, returning the entry's POSITION on the board.
    /// The position is a protocol output, not a convenience: the voter's
    /// inclusion receipt (see docs/ballot-verify-ux.md) needs a stable
    /// reference to the appended entry. Order of checks: signature, then
    /// double spend, then ballot validity, so a forged token can never probe
    /// the spent-set and an invalid ballot does not burn the token.
    pub fn cast(
        &mut self,
        pk: &IssuerPublicKey,
        rules: &BallotRules,
        entry: BoardEntry,
    ) -> Result<usize, CastError> {
        pk.verify(&entry.signature, entry.msg_randomizer, &entry.token)
            .map_err(|_| CastError::BadSignature)?;
        if self.entries.iter().any(|e| e.token == entry.token) {
            return Err(CastError::DoubleSpend);
        }
        rules.validate(&entry.choices).map_err(CastError::Invalid)?;
        self.entries.push(entry);
        Ok(self.entries.len() - 1)
    }

    /// The board's entries, in cast order.
    pub fn entries(&self) -> &[BoardEntry] {
        &self.entries
    }
}
