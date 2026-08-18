# Member-facing ballot verify and audit flow (paper design)

Pre-rewrite plan, round 2, item 10. Paper only: no UI code, no lexicon authoring (record shapes are
item 9's job). This document is nonetheless a PROTOCOL INPUT, not just UX: if the spec crate
(`crates/ballot-spec`) and the board/poll records do not reserve what the voter must keep and query
after casting, individual verifiability cannot be retrofitted. Section 1 states those reservations
explicitly.

Grounding: the decided scheme is RFC 9474 RSA blind signatures (RSABSSA-SHA384-PSS-Randomized), UNIT
tokens (a voter with resolved weight N holds N identical tokens), PER-POLL issuer keys with the pubkey
published to the board before open, and an append-only public bulletin board of atproto records. See
`crates/ballot-spec/src/lib.rs` (the `BoardEntry` shape and `Board::cast` semantics),
`crates/ballot-spec/DECISIONS.md` (D1 to D8), `docs/atproto-domain-model.md` (the voting SQL), and
`docs/atproto-open-decisions.md`. Custody direction (item 9's memo): the board is org-published
(custodian-published), which reintroduces a censorship risk that a custodian-signed INCLUSION RECEIPT
answers; this document designs around that direction.

Two verifiability properties, in the scheme's own terms
(`docs/atproto-stack-decisions.md`, Voting integrity section):

- **Individual verifiability**: the voter finds their own ballot on the public board.
- **Universal verifiability**: anyone re-tallies the public board and reproduces the count.

## 1. The receipt: what the client stores at cast time

A voter with resolved weight N casts N unit tokens and therefore holds N receipts. Each receipt covers
exactly one board entry; the flows below run per receipt, and the UI aggregates ("3 of 3 counted").
Receipts are ALWAYS-PRIVATE voter-held artifacts: they get no lexicon and are never published
(publishing one would link the voter to the entry, which is the whole thing the scheme avoids).

### 1.1 Receipt contents (one per unit token)

Stored locally (client storage, exportable as a file) at the moment `cast` succeeds:

| Field | Source | Why it is needed |
|-|-|-|
| Poll ref | the poll announcement record (AT-URI + CID) | scopes every later query; the poll record carries the issuer pubkey and the ballot rules |
| Token nullifier (32 bytes) | voter-generated (`TokenRequest.nullifier`, D7) | the board match key: find-my-ballot is a byte-equality search for this value |
| Message randomizer | blinding state (`BlindingResult`, the Randomized variant) | required to re-verify the entry's RSA-PSS signature (`pk.verify(sig, msg_randomizer, token)`) |
| Token signature | `finalize_token` output | proves the token was issued under the poll key; already public on the board once cast |
| Choices as cast | the voter's selection | detects substitution: the board signature covers only the token, NOT the choices (see 1.4 and 4.4) |
| Cast timestamp | local clock | display and dispute narrative only; carries no protocol weight |
| Inclusion receipt | custodian-signed, returned by the cast endpoint | censorship and substitution evidence (see 1.4) |

Note what is NOT in the receipt: the voter's DID, the poll-side issuance record, or anything derived
from weight. The receipt file itself must stay unlinkable if leaked; N receipts leaking together
reveals a weight, so the export UI bundles receipts per poll only on explicit voter action and says so.

The nullifier and signature become public on the board the moment the cast lands, so the receipt holds
no long-term secret EXCEPT before casting (a stolen unspent nullifier plus signature is a stealable
vote) and except for linkability (whoever holds the receipt can link the voter to the entry). Client
storage treats receipts at the same protection level as the session key.

### 1.2 What the spec crate must reserve

`crates/ballot-spec` already keeps the voter-side material available (`TokenRequest` exposes the
nullifier and blinding state; `finalize_token` returns the spendable `Signature`). Two reservations
are still required, and they are cheap now, expensive after the AppView ships:

1. **`Board::cast` must return the appended entry's identity, not `()`**. Today it returns
   `Result<(), CastError>`. The custody layer cannot mint an inclusion receipt for "whatever got
   appended" without an acknowledged position: cast must return the entry's board position (its index
   in the append-only sequence) or an equivalent stable handle. This also pins the ordering claim the
   receipt makes ("entry K of the board").
2. **Voter-side types must be persistable**. `TokenRequest` (nullifier + `BlindingResult`) must
   survive an app restart between issuance and cast, and the (nullifier, `MessageRandomizer`,
   `Signature`) triple must persist indefinitely after cast. The spec crate must not hide these behind
   opaque non-serializable types; byte encodings for all three are item 9's call and are marked
   provisional there (D7 note).

### 1.3 What the board and poll records must reserve (item 9 inputs)

The record drafts (item 9) must reserve, or the flows in sections 2 and 3 cannot be built:

- **Board entry**: a `pollRef` strongRef (find-my-ballot and re-tally filter by poll); exact,
  canonical byte encodings for token, message randomizer, and signature so a client can byte-match its
  stored nullifier against fetched records without ambiguity; structurally NO voter identity field
  (already the spec crate's type-level guarantee); deterministic content addressing (the CID is what
  the inclusion receipt points at, so no server-mutable field may live inside the addressed payload).
- **Poll announcement**: the issuer pubkey and the ballot rules (options, min, max, blank), published
  BEFORE open. Re-tally needs both; a pubkey published after open would let the custodian swap keys
  mid-poll.
- **Close announcement** (the poll-close or resolution record): the announced per-option counts, the
  announced outcome, AND the announced total number of unit tokens issued (the sum of resolved
  weights, a single integer that leaks no individual weight). Without a published counts-plus-issued
  figure there is nothing machine-checkable to compare a re-tally against, and no public bound on
  board size (see limits, section 5).

### 1.4 The inclusion receipt (custody consequence)

Because the board is custodian-published, "I cast and the custodian silently dropped it" must be
distinguishable from "I never cast". The cast endpoint therefore returns, atomically with a successful
cast, a receipt signed by the custodian (verifiable against the org DID's published signing key):

```text
InclusionReceipt = sign_custodian(
  pollRef,
  entry CID            (the deterministic address of the exact BoardEntry payload),
  board position       (the index Board::cast acknowledged),
  token, msg_randomizer, signature, choices   (or equivalently: the CID commits to all of these)
)
```

The receipt does two jobs:

1. **Inclusion**: the custodian has signed that this entry is on the board at this position. A later
   board without it is self-incriminating (section 4.2).
2. **Correct recording**: the token signature covers only the 32-byte nullifier (D7: the message
   carries no structure), so choices are NOT bound by the voter's token. The receipt is what binds the
   custodian to the choices as submitted; a board entry with the same token but different choices
   contradicts the custodian's own signature (section 4.4).

No new voter-side key is needed: the receipt is custodian-signed, voter-held. Verifying it needs only
the org DID document, which anyone can resolve.

## 2. Find-my-ballot

**Goal**: individual verifiability. "My ballot is on the board, recorded as I cast it."

Flow, per poll:

1. Load the poll's receipts from local storage (weight N gives N of them).
2. Fetch the poll's board entries. The board is public atproto records, so the source can be the
   AppView, the org repo directly, or any independent mirror; the UI defaults to the AppView and
   offers "check against an independent copy" as the paranoid path.
3. For each receipt, search the entries for a byte-exact token match.
4. For each match: re-verify the entry's RSA-PSS signature under the poll's published issuer pubkey,
   and compare the entry's choices against the receipt's stored choices.

What the voter sees, per receipt (aggregated as "N of N counted" when all pass):

- **Counted**: entry found, signature verifies, choices match. Show the recorded choices and the board
  position, so the voter can find the same entry in any mirror.
- **Recorded differently**: entry found (token matches) but choices differ from the receipt. Alarm
  state, see 4.4.
- **Not on the board**: no token match. Meaning depends on poll state and on whether an inclusion
  receipt is held, see 4.1 and 4.2.

Weight stays invisible publicly: the N entries a weight-N voter finds are identical in shape to every
other entry. Only the voter's own device knows the N receipts belong together.

## 3. One-tap re-tally

**Goal**: universal verifiability. "The announced result is what the public board actually says."
This flow needs NO receipt and works for members who did not vote, and for non-members: everything it
consumes is public.

One tap runs, client-side (the ballot-spec crate compiled to WASM, so the code that defines the count
IS the code that re-checks it):

1. Fetch the poll announcement (issuer pubkey, ballot rules) and the close announcement (announced
   counts, outcome, total tokens issued).
2. Fetch every board entry for the poll, ideally from an independent mirror.
3. Recompute, in board order, exactly what `Board::cast` enforces: drop any entry whose signature does
   not verify under the issuer pubkey; drop any entry whose token already appeared (first entry
   stands, D4); drop any entry whose choices violate the ballot rules.
4. Run `tally` and `outcome` over the surviving entries.
5. Compare: recomputed counts vs announced counts, recomputed outcome vs announced outcome, surviving
   entry count vs announced total tokens issued (board must not exceed it).

What the voter sees:

- **Green**: "Recount matches: <count> ballots, result <outcome>." Plus the board snapshot identity
  (latest entry CID and count) so two people can confirm they recounted the same board.
- **Red**: a mismatch report: which option counts differ, or which entries were dropped and why
  (bad signature, duplicate token, invalid choices), or "board holds more ballots than tokens issued".
  Every line of the report is reproducible by anyone from public data, see 4.3 and 4.5.

Cost honesty: one RSA-PSS verification per entry. Assembly-scale boards (hundreds to low thousands of
entries) verify in seconds in WASM; this is a non-issue at the org's scale and the button can show a
progress count for larger boards.

## 4. Failure states and what each means

| # | Observation | Meaning | Evidence quality | What to do |
|-|-|-|-|-|
| 4.1 | Not on the board, poll OPEN, no inclusion receipt | the cast never landed (network failure, crash before the receipt arrived) | none, and none needed | recast: an unspent token is not burned (D8 covers the invalid-ballot case; a cast that never landed spends nothing) |
| 4.2 | Inclusion receipt held, entry absent (or poll closed and entry never appeared) | censorship, or an equivocating custodian showing different boards | cryptographic: the custodian's own signature vs the custodian's own board | export the evidence bundle; challenge |
| 4.3 | An entry's signature does not verify under the issuer pubkey | forged entry: appended without the poll's issuer key | cryptographic, anyone can check with public data | nothing to fix locally: an honest re-tally excludes it; if the announced count included it, that surfaces as 4.5 |
| 4.4 | Token found, choices differ from the receipt | substitution by the custodian, or the token leaked pre-spend and someone spent it first (D4: first entry stands) | cryptographic against the custodian IF the inclusion receipt's choices contradict the board entry; otherwise a leaked-token loss | with a contradicting receipt: export and challenge; without one: the vote is lost to whoever spent first, treat the device/receipt store as compromised |
| 4.5 | Recount does not match the announcement | bad tally: the announcement counted a different board than the public one | cryptographic in the reproducible sense: anyone re-runs step 3 and gets the same report | publish the recount report; challenge |

Notes per state:

- **4.1, missing without a receipt**: deliberately NOT treated as evidence of anything. The protocol
  makes the ambiguous case safe instead: the token is unspent, so the voter recasts while the poll is
  open. The UI nudges exactly that ("your ballot did not go through, cast again") and only escalates
  if recasting repeatedly fails.
- **4.2, missing with a receipt**: this is what the inclusion receipt exists for. The bundle
  (receipt + a board snapshot with its latest CID) proves the custodian signed an entry into the board
  and now serves a board without it. Verification needs only the org DID document and the public
  board; any third party can confirm it. The remedy is procedural, not cryptographic: the crypto
  proves misconduct, it cannot force the entry back. The UI's job is a one-tap "export evidence"
  producing a self-contained file, plus pointing at the org's dispute channel (the assembly itself;
  the poll's closing authority is on the poll record).
- **4.3, forged entry**: an entry that fails signature verification was not minted by the poll's
  issuer key (a different poll's token lands here too, which is exactly the per-poll-key binding
  working). Anyone can check it: pubkey and entry are both public. It cannot change an honest count,
  because every verifier excludes it by the same rule.
- **4.4, recorded differently**: the subtle one, because the token signature does not cover choices
  (D7). The inclusion receipt closes the gap: custodian-signed choices vs custodian-published entry is
  a self-contradiction anyone can verify. If the voter has NO receipt binding those choices (cast
  succeeded but the receipt was never stored), the honest possibilities cannot be distinguished
  cryptographically; the UI must say so plainly rather than accuse.
- **4.5, counts mismatch**: the board is the ground truth by construction; an announcement that
  disagrees with it is wrong. Because the recount is deterministic from public data, the report is not
  "my app says": it is a recipe anyone re-runs.

## 5. What this proves, and what it does not (honest limits)

- **Proves, per voter**: my N ballots are on the board, recorded as cast (find-my-ballot plus the
  receipt), and the announced result is the arithmetic consequence of the public board (re-tally).
- **Does NOT prove: other ballots' eligibility beyond issuer-signature validity.** A verifying
  signature means "the poll's issuer key signed this token", not "an eligible voter cast it". The
  issuer secret key is held by the custodian during the poll, so a corrupt custodian could mint tokens
  beyond the roster. The public bound is the announced total-tokens-issued figure (section 1.3): the
  board exceeding it is publicly damning, but the figure itself restates the org's private roster
  arithmetic. Auditing THAT (roster, weights, delegation resolution, one-issuance-per-voter) is an
  org-internal audit over always-private data (`eligibility`, `delegation`, `token_issued`), outside
  what a member can verify from public records. This is a stated property of the chosen scheme, not an
  oversight: the roster is private by decision.
- **Not receipt-free.** A voter CAN prove how they voted, to anyone, by revealing a receipt (or just
  the nullifier). That makes vote-selling and coercion technically possible, which is inherent to
  find-your-own-ballot bulletin-board schemes. Assembly context mitigates it socially, not
  cryptographically; the UI never displays anything that makes involuntary disclosure easy (no public
  "share my ballot" affordance, explicit warnings on receipt export).
- **Delegation is invisible here, by design.** Weights resolve before open; a weight-N voter simply
  holds N indistinguishable receipts. Verifying that a delegation was honored (my weight moved to my
  delegate) is a private query against the org, not a board property.
- **Loss of receipts loses individual verifiability only.** A voter who wipes their device can no
  longer find their ballot or prove inclusion; the ballot still counts and re-tally still works.
  Receipts are not recoverable by anyone (the org never sees nullifiers: blinding, D7), and the UI
  must say that at cast time.

## 6. Plain-words copy drafts (i18n source)

One short paragraph per flow, written for a civic assembly member, no crypto vocabulary. These are the
actual copy drafts to seed the i18n catalog later; keys are suggestions.

**`verify.receipt.explainer`** (shown at cast time)

> Your ballot is anonymous. When you vote, this device keeps a private ballot stub: it is the only
> proof that a specific ballot on the public list is yours, and nobody else, not even the organizers,
> can ever recreate it. If you hold extra votes that others handed to you, you get one stub per vote.
> Keep this device or export your stubs: without them your vote still counts, but you lose the ability
> to check it yourself.

**`verify.find.explainer`** (the find-my-ballot screen)

> Every ballot in this vote is published on a public list, with no names attached. Your stub lets this
> device point at the exact entries that are yours and confirm they say what you chose. If everything
> matches, you will see "counted" next to each of your votes. You can also check against an
> independent copy of the list, so you are not just taking our word for it.

**`verify.retally.explainer`** (the re-tally screen)

> Anyone can count this vote, not just the organizers. Tap once and this device downloads the full
> public list of ballots, checks that each one carries the official stamp for this vote and follows
> the ballot rules, and counts them itself. If its count matches the announced result, the result is
> confirmed. You do not need to have voted, and you do not need to trust this app: the list is public
> and anyone can run the same count.

**`verify.fail.missing`** (ballot not found, poll still open)

> Your ballot did not make it onto the public list, which usually means the connection dropped while
> casting. Nothing is lost and nobody has seen your vote: just cast it again now.

**`verify.fail.censored`** (ballot not found, acknowledgment held)

> The organizers' system confirmed your ballot in writing when you cast it, but that ballot is not on
> the public list now. Those two things cannot both be honest, and this device holds the proof. Export
> the evidence file and raise it with the assembly: anyone can check it, and it does not reveal how
> anyone else voted.

**`verify.fail.forged`** (an entry fails the stamp check)

> A ballot on the public list does not carry the official stamp for this vote, so it was not issued to
> any voter here. It is excluded from any honest count, and anyone who checks the list will exclude
> the same ballot for the same reason. Your own vote is not affected.

**`verify.fail.mismatch`** (recount disagrees with the announcement)

> This device counted the public list of ballots and got a different result than the one announced.
> The public list is the ballots; a result that does not match it is wrong. You can export the count
> report, and anyone who repeats the count will get the same numbers.

## Relationship to other round-2 items

- **Item 8** (`crates/ballot-spec`): section 1.2's two reservations are changes to that crate
  (`cast` return value, persistable voter-side types). Its `DECISIONS.md` gains no new semantic;
  D4, D7, D8 are load-bearing here and unchanged.
- **Item 9** (custody memo, poll and board-entry drafts): section 1.3 is that item's input checklist
  (pollRef, canonical byte encodings, deterministic addressing, pre-open pubkey, announced
  counts-plus-issued at close), and section 1.4 is the inclusion-receipt consequence of org custody.
  The receipt itself is always-private and gets NO lexicon.
- **NSID**: no NSID appears in this document by design; everything references records by role. The
  placeholder authority for the drafts remains `com.example.wiki.*` until the domain call lands.
