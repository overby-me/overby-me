# ballot-spec decision log

Semantics this crate PINS that no design doc had decided. Each is enforced by
the code and its property tests; none is silently frozen. Status: taken as
stated on 2026-09-20, when the owner asked for the open items to be finished
(`docs/ballot-decisions-signoff.md`); delegation and the published board are
built on them. Overturning one is a code + test change, cheap before a binding
vote has been held under it and expensive after.

- **D1 (delegation chains are transitive).** A delegation chain follows to its
  terminal non-delegating voter: A delegates to B, B delegates to C, then A's
  and B's weight both land on C. Alternative (single-hop only) rejected as
  surprising: B's outgoing delegation would silently keep A's weight on B.
- **D2 (cycles are void, weight stays).** A chain that revisits a voter (incl.
  a self-delegation) is VOID for the walking voter: they keep their own weight
  and may vote themselves. Deterministic and terminating; no tie-break rules.
  Alternative (drop the weight) rejected: it would destroy voting weight.
- **D3 (delegating to an ineligible DID is void).** A hop to a DID not on the
  poll's eligibility roster voids the walking voter's delegation (weight
  stays). Alternative (weight moves off-roster) rejected: weight must stay
  inside the eligible set.
- **D4 (double spend: first entry stands).** A token reuse is rejected at cast
  time; the FIRST board entry stands, even if the second carries different
  choices. Alternative (reject-both) rejected: it would let anyone who learns
  a spent token retroactively void that ballot.
- **D5 (a successful delegator is issued no tokens).** Resolved weight 0 means
  zero unit tokens: delegating means not voting in that poll. (Receiving
  delegations still adds to the delegate's token count.)
- **D6 (RFC 9474 variant).** RSABSSA-SHA384-PSS-Randomized, the RFC's
  recommended parameter set, 2048-bit minimum modulus. The `Randomized`
  message preparation defends against message-structure attacks; PSS keeps
  standard salt semantics.
- **D7 (token message = 32-byte random nullifier).** The voter generates the
  nullifier; it carries NO structure (no poll id, no weight, no time). Poll
  binding comes from the per-poll issuer key, not the message; uniqueness on
  the board is the dedup. PROVISIONAL detail for item 9: the board-entry
  record's field encodings.
- **D8 (invalid ballot does not burn the token).** Check order at cast is
  signature, then double-spend, then ballot validity, so a ballot rejected as
  invalid leaves its token unspent and the voter may recast.
