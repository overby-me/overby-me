# Ballot semantics D1–D8 — owner ratification sheet

Eight semantics the `ballot-spec` crate PINS that no design doc had decided
(`crates/ballot-spec/DECISIONS.md`). Each is already enforced by code + property
tests (`crates/ballot-spec/tests/properties.rs`, 22 tests). They are currently my
assumptions, not your ratified rules — **overturning any one today is a localized
code + test change; after the ballot vertical ships it is expensive.** Ratify or
amend each; a ✅ is "as-stated", an ✏️ is "change to …".

| # | Decision (one line) | Alternative rejected | Pinned by (test) | Verdict |
|---|---------------------|----------------------|------------------|:------:|
| **D1** | Delegation chains are **transitive** — A→B→C lands A's and B's weight on C | single-hop only (would silently keep A's weight on B) | `delegation_resolves_within_roster`, `issued_weight_arithmetic_end_to_end` | ☐ ✅ / ✏️ |
| **D2** | A **cycle** (incl. self-delegation) is **void** — the walking voter keeps their weight and may vote | drop the weight (destroys voting weight) | `delegation_conserves_weight` (arb rosters generate cycles) | ☐ ✅ / ✏️ |
| **D3** | Delegating to an **ineligible DID** is **void** — weight stays with the delegator | weight moves off-roster (weight must stay inside the eligible set) | `delegation_resolves_within_roster` | ☐ ✅ / ✏️ |
| **D4** | Double spend: **first entry stands** — a token reuse is rejected at cast, the first ballot holds even if the second differs | reject-both (lets anyone who learns a spent token void that ballot) | `double_spend_always_collides`, `tally_invariant_under_permutation` | ☐ ✅ / ✏️ |
| **D5** | A **successful delegator is issued no tokens** — resolved weight 0 = zero unit tokens | — (definitional; receiving delegations still adds tokens) | `successful_delegator_has_no_weight` | ☐ ✅ / ✏️ |
| **D6** | **RFC 9474 variant** = RSABSSA-SHA384-PSS-**Randomized**, 2048-bit min modulus | plain/deterministic prep (Randomized defends against message-structure attacks) | `issue_blind_unblind_verify_round_trip`, `cross_poll_token_rejected`, `forged_signature_rejected` | ☐ ✅ / ✏️ |
| **D7** | Token message = **32-byte random nullifier**, no structure (no poll id / weight / time); poll binding is the per-poll key, dedup is board-uniqueness | encode structure into the token (would leak/couple) | `issue_blind_unblind_verify_round_trip` (+ the no-DID-linkage board type) | ☐ ✅ / ✏️ |
| **D8** | An **invalid ballot does not burn the token** — check order is signature → double-spend → validity, so a rejected-invalid ballot leaves the token unspent and recastable | validity-before-dedup (would burn the token on a fixable mistake) | `PersistentBoard::cast` order + `forged_and_invalid_are_rejected_without_burning` (ballot-store) | ☐ ✅ / ✏️ |

## Verify-UX consequences worth a second look (D4, D7, D8)

- **D4 (first-entry-stands)** shapes the voter's inclusion receipt: the receipt
  must reference the *first* board position for a token, and a voter who somehow
  double-submits sees their FIRST ballot counted. If you'd rather the voter be
  able to *replace* their ballot before close, that is a different rule (last-
  entry-stands) and a different receipt story — decide now, not after receipts
  ship.
- **D7 (structureless nullifier)** is what makes the board carry no voter link,
  but it also means the org cannot answer "did voter X vote?" from the board —
  only "was a token spent." That is the anonymity guarantee; confirm it matches
  the assembly's expectation (no per-voter turnout list from the board).
- **D8 (invalid doesn't burn)** means a voter who fat-fingers an out-of-range
  choice can retry with the same token. Good UX, but it means a token can be
  presented multiple times before a valid cast — confirm that is acceptable
  (it never lets a token be *counted* twice; D4 still holds).

## Scope note

Ratifying D1–D8 does **not** by itself take the board-entry crypto field
encodings out of PROVISIONAL — D7 flags exactly that as a separate item (item 9,
the board/poll record byte-level serialization), which is gated on the
board-custody call and pinned later with a known-answer vector. This sheet is
about the *semantics*; the wire encoding is a downstream, independent decision.
