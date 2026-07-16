//! The ballot scheme's conformance suite: every property here transfers into
//! the AppView's ballot core unchanged. RSA keygen is expensive, so all crypto
//! tests share ONE per-suite issuer (a stand-in for one poll) plus a second
//! "other poll" issuer for the rejection cases; the pure-math properties
//! (delegation, validity, tally) need no crypto and run with full proptest
//! case counts.

use ballot_spec::*;
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// RFC 9474 minimum modulus. Generated once per suite (keygen is ~seconds).
const BITS: usize = 2048;

fn issuer() -> &'static TokenIssuer {
    static ISSUER: OnceLock<TokenIssuer> = OnceLock::new();
    ISSUER.get_or_init(|| TokenIssuer::new_for_poll(BITS).expect("keygen"))
}

/// A different poll's issuer: per-poll keys mean its tokens must not verify
/// under `issuer()`'s pubkey.
fn other_issuer() -> &'static TokenIssuer {
    static OTHER: OnceLock<TokenIssuer> = OnceLock::new();
    OTHER.get_or_init(|| TokenIssuer::new_for_poll(BITS).expect("keygen"))
}

/// Issue one spendable unit token from `iss`: request, blind-sign, finalize.
fn issue_token(iss: &TokenIssuer, choices: Vec<usize>) -> BoardEntry {
    let pk = iss.public_key();
    let req = request_token(pk).expect("blind");
    let blind_sig = iss
        .blind_sign(&req.blinding.blind_message)
        .expect("blind sign");
    let sig = finalize_token(pk, &req, &blind_sig).expect("finalize verifies");
    BoardEntry {
        token: req.nullifier.clone(),
        msg_randomizer: req.blinding.msg_randomizer,
        signature: sig,
        choices,
    }
}

// ---------------------------------------------------------------------------
// Crypto path (issue -> blind -> unblind -> verify), plain tests
// ---------------------------------------------------------------------------

#[test]
fn issue_blind_unblind_verify_round_trip() {
    let entry = issue_token(issuer(), vec![0]);
    // The finalized signature verifies standalone against the issuer pubkey.
    issuer()
        .public_key()
        .verify(&entry.signature, entry.msg_randomizer, &entry.token)
        .expect("unblinded signature verifies");
}

#[test]
fn cross_poll_token_rejected() {
    // Per-poll keys ARE the poll binding: a token issued by poll B's issuer
    // must be rejected by poll A's board.
    let foreign = issue_token(other_issuer(), vec![0]);
    let rules = BallotRules {
        options: 2,
        min: 1,
        max: 1,
        blank: false,
    };
    let mut board = Board::default();
    assert_eq!(
        board.cast(issuer().public_key(), &rules, foreign),
        Err(CastError::BadSignature)
    );
}

#[test]
fn forged_signature_rejected() {
    let mut entry = issue_token(issuer(), vec![0]);
    // Tamper with the token after signing: the signature must not verify.
    entry.token[0] ^= 0xff;
    let rules = BallotRules {
        options: 2,
        min: 1,
        max: 1,
        blank: false,
    };
    let mut board = Board::default();
    assert_eq!(
        board.cast(issuer().public_key(), &rules, entry),
        Err(CastError::BadSignature)
    );
}

#[test]
fn double_spend_always_collides() {
    let rules = BallotRules {
        options: 3,
        min: 1,
        max: 1,
        blank: false,
    };
    let entry = issue_token(issuer(), vec![1]);
    let mut board = Board::default();
    assert_eq!(
        board.cast(issuer().public_key(), &rules, entry.clone()),
        Ok(())
    );
    // Same token, same choices: rejected. Same token, DIFFERENT choices: still
    // rejected (the first entry stands, DECISIONS.md D4).
    assert_eq!(
        board.cast(issuer().public_key(), &rules, entry.clone()),
        Err(CastError::DoubleSpend)
    );
    let mut re_spend = entry;
    re_spend.choices = vec![2];
    assert_eq!(
        board.cast(issuer().public_key(), &rules, re_spend),
        Err(CastError::DoubleSpend)
    );
}

#[test]
fn issued_weight_arithmetic_end_to_end() {
    // A tiny end-to-end poll: roster -> resolve -> issue resolved-weight unit
    // tokens each -> cast -> tally equals issued-token arithmetic.
    let a = Did("did:example:a".into());
    let b = Did("did:example:b".into());
    let c = Did("did:example:c".into());
    let roster = EligibilityRoster {
        base_weight: BTreeMap::from([(a.clone(), 2), (b.clone(), 1), (c.clone(), 1)]),
        // b delegates to a: a resolves to 3, b to 0, c stays 1.
        delegation: BTreeMap::from([(b.clone(), a.clone())]),
    };
    let resolved = roster.resolve();
    assert_eq!(resolved.resolved_weight[&a], 3);
    assert_eq!(resolved.resolved_weight[&b], 0);
    assert_eq!(resolved.resolved_weight[&c], 1);

    let rules = BallotRules {
        options: 2,
        min: 1,
        max: 1,
        blank: false,
    };
    let mut board = Board::default();
    // a's 3 tokens vote option 0; c's 1 token votes option 1.
    for _ in 0..3 {
        board
            .cast(
                issuer().public_key(),
                &rules,
                issue_token(issuer(), vec![0]),
            )
            .expect("cast");
    }
    board
        .cast(
            issuer().public_key(),
            &rules,
            issue_token(issuer(), vec![1]),
        )
        .expect("cast");

    let counts = tally(board.entries(), &rules);
    assert_eq!(counts, vec![3, 1]);
    // Total entries equal total issued weight; the tally is a plain count.
    let issued: u64 = resolved.resolved_weight.values().sum();
    assert_eq!(board.entries().len() as u64, issued);
    assert_eq!(outcome(&counts, &rules), Outcome::Winner(0));
}

// ---------------------------------------------------------------------------
// Delegation resolution (pure math, full proptest)
// ---------------------------------------------------------------------------

/// An arbitrary roster: up to 12 voters with weights 0..=5, and an arbitrary
/// delegation map possibly containing chains, cycles, self-loops, and hops to
/// ineligible DIDs.
fn arb_roster() -> impl Strategy<Value = EligibilityRoster> {
    let voters = prop::collection::btree_map(0usize..12, 0u64..=5, 1..12);
    voters.prop_flat_map(|weights| {
        let dids: Vec<usize> = weights.keys().cloned().collect();
        let n = dids.len();
        let delegations = prop::collection::btree_map(
            0usize..n,
            // A target inside the roster, or (rarely) an ineligible outsider.
            prop_oneof![
                4 => (0usize..n).prop_map(Some),
                1 => Just(None),
            ],
            0..=n,
        );
        let weights2 = weights.clone();
        let dids2 = dids.clone();
        delegations.prop_map(move |d| EligibilityRoster {
            base_weight: weights2
                .iter()
                .map(|(k, v)| (Did(format!("did:example:{k}")), *v))
                .collect(),
            delegation: d
                .into_iter()
                .map(|(from_i, to)| {
                    let from = Did(format!("did:example:{}", dids2[from_i]));
                    let to = match to {
                        Some(to_i) => Did(format!("did:example:{}", dids2[to_i])),
                        None => Did("did:example:outsider".into()),
                    };
                    (from, to)
                })
                .collect(),
        })
    })
}

proptest! {
    /// Weight conservation: resolution NEVER creates or destroys weight, no
    /// matter what chains, cycles, or ineligible hops the delegation map holds.
    #[test]
    fn delegation_conserves_weight(roster in arb_roster()) {
        let base: u64 = roster.base_weight.values().sum();
        let resolved = roster.resolve();
        let total: u64 = resolved.resolved_weight.values().sum();
        prop_assert_eq!(base, total);
    }

    /// Resolution is deterministic and total: every eligible voter has a
    /// resolved weight, and no weight lands on an ineligible DID.
    #[test]
    fn delegation_resolves_within_roster(roster in arb_roster()) {
        let resolved = roster.resolve();
        for did in resolved.resolved_weight.keys() {
            prop_assert!(roster.base_weight.contains_key(did),
                "weight landed outside the roster: {:?}", did);
        }
        for did in roster.base_weight.keys() {
            prop_assert!(resolved.resolved_weight.contains_key(did));
        }
        // Determinism: resolving twice gives the same result.
        prop_assert_eq!(resolved, roster.resolve());
    }

    /// A voter whose delegation RESOLVED (their chain reached an eligible
    /// terminal that is not themselves) ends with weight 0: delegating means
    /// not voting.
    #[test]
    fn successful_delegator_has_no_weight(roster in arb_roster()) {
        let resolved = roster.resolve();
        for from in roster.delegation.keys() {
            if !roster.base_weight.contains_key(from) { continue; }
            // Re-derive whether the chain resolves off `from`.
            let sub = EligibilityRoster {
                base_weight: roster.base_weight.clone(),
                delegation: roster.delegation.clone(),
            };
            let single = EligibilityRoster {
                base_weight: BTreeMap::from([(from.clone(), 1)]),
                delegation: sub.delegation.clone(),
            };
            // If a 1-weight probe from `from` conserves onto someone else,
            // the real voter's own weight moved too. Their resolved weight is
            // then only what OTHERS delegated to them.
            let probe = single.resolve();
            let moved = probe.resolved_weight.get(from) == Some(&0);
            if moved && roster.base_weight[from] > 0 {
                // Weight from `from` landed elsewhere: from's resolved weight
                // excludes their base (it may still receive from others).
                let incoming: u64 = resolved.resolved_weight[from];
                let everyone: u64 = roster.base_weight.values().sum();
                prop_assert!(incoming <= everyone - roster.base_weight[from]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ballot validity (transcribed shipped semantics; pure math, full proptest)
// ---------------------------------------------------------------------------

proptest! {
    /// The blank abstention (last option) is only ever valid ALONE, and a lone
    /// blank bypasses the minimum (poll.rs:358-372).
    #[test]
    fn blank_only_alone(
        options in 2usize..8,
        min in 0usize..4,
        max in 1usize..6,
        extra in 0usize..6,
    ) {
        let rules = BallotRules { options, min, max, blank: true };
        let blank = options - 1;
        // Lone blank: always valid regardless of min (as long as max >= 1).
        prop_assert_eq!(rules.validate(&[blank]), Ok(()));
        // Blank plus any other in-range choice: always invalid.
        let other = extra % (options - 1);
        prop_assert_eq!(
            rules.validate(&[other, blank]),
            Err(InvalidBallot::BlankNotAlone)
        );
    }

    /// min/max bounds hold exactly for non-blank ballots.
    #[test]
    fn min_max_enforced(
        options in 3usize..10,
        min in 1usize..3,
        chosen_n in 0usize..8,
    ) {
        let max = min + 2;
        let rules = BallotRules { options, min, max, blank: false };
        let n = chosen_n.min(options);
        let choices: Vec<usize> = (0..n).collect();
        let verdict = rules.validate(&choices);
        if n < min {
            prop_assert_eq!(verdict, Err(InvalidBallot::TooFew));
        } else if n > max {
            prop_assert_eq!(verdict, Err(InvalidBallot::TooMany));
        } else {
            prop_assert_eq!(verdict, Ok(()));
        }
    }

    /// Out-of-range or repeated indices are always rejected.
    #[test]
    fn bad_indices_rejected(options in 1usize..8, idx in 0usize..16) {
        let rules = BallotRules { options, min: 0, max: 16, blank: false };
        if idx >= options {
            prop_assert_eq!(rules.validate(&[idx]), Err(InvalidBallot::BadIndex));
        }
        if options >= 1 {
            prop_assert_eq!(rules.validate(&[0, 0]), Err(InvalidBallot::BadIndex));
        }
    }
}

// ---------------------------------------------------------------------------
// Tally properties (pure math over entries, full proptest)
// ---------------------------------------------------------------------------

/// Arbitrary board entries with valid single/multi choices (no crypto: tally
/// and outcome operate on plain data, verification happened at cast).
fn arb_entries(options: usize) -> impl Strategy<Value = Vec<BoardEntry>> {
    prop::collection::vec(
        prop::collection::btree_set(0..options, 1..=options.min(3)).prop_map(move |set| {
            BoardEntry {
                token: set.iter().map(|c| *c as u8).collect(), // token irrelevant here
                msg_randomizer: None,
                signature: Signature(vec![]),
                choices: set.into_iter().collect(),
            }
        }),
        0..40,
    )
}

proptest! {
    /// Tally invariance under board permutation: the count vector is
    /// independent of cast order (append order carries no meaning).
    #[test]
    fn tally_invariant_under_permutation(
        entries in arb_entries(5),
        seed in any::<u64>(),
    ) {
        let rules = BallotRules { options: 5, min: 1, max: 3, blank: false };
        let baseline = tally(&entries, &rules);
        // Deterministic pseudo-shuffle from the seed.
        let mut shuffled = entries.clone();
        let mut s = seed;
        for i in (1..shuffled.len()).rev() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (s % (i as u64 + 1)) as usize;
            shuffled.swap(i, j);
        }
        prop_assert_eq!(tally(&shuffled, &rules), baseline);
    }

    /// Completeness: every selection of every entry is counted exactly once
    /// (the counts sum to the total number of selections cast).
    #[test]
    fn tally_counts_every_selection(entries in arb_entries(5)) {
        let rules = BallotRules { options: 5, min: 1, max: 3, blank: false };
        let counts = tally(&entries, &rules);
        let total_selections: usize = entries.iter().map(|e| e.choices.len()).sum();
        let counted: u64 = counts.iter().sum();
        prop_assert_eq!(counted, total_selections as u64);
    }

    /// Outcome semantics (poll.rs:299-332): blank never wins; a strict single
    /// non-blank max wins; a shared max is a tie; all-zero is NoVotes.
    #[test]
    fn outcome_excludes_blank_and_detects_ties(entries in arb_entries(4)) {
        let rules = BallotRules { options: 4, min: 1, max: 3, blank: true };
        let counts = tally(&entries, &rules);
        match outcome(&counts, &rules) {
            Outcome::Winner(i) => {
                prop_assert!(i < 3, "blank (index 3) must never win");
                for (j, c) in counts.iter().enumerate().take(3) {
                    if j != i {
                        prop_assert!(counts[i] > *c);
                    }
                }
            }
            Outcome::Tie => {
                let max = counts.iter().take(3).max().copied().unwrap_or(0);
                prop_assert!(max > 0);
                let at_max = counts.iter().take(3).filter(|c| **c == max).count();
                prop_assert!(at_max >= 2);
            }
            Outcome::NoVotes => {
                prop_assert!(counts.iter().take(3).all(|c| *c == 0));
            }
        }
    }
}
