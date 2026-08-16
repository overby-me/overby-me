# Ballot board custody: who publishes the board-entry records

Status: OPTIONS MEMO, pending owner call (pre-rewrite plan, round-2 item 9). The decided scheme
(RFC 9474 blind-signature UNIT tokens, per-poll issuer keys, a public bulletin board that IS atproto
records; see `docs/atproto-open-decisions.md` and `crates/ballot-spec`) deliberately left one question
open: in WHOSE repo do the board-entry records live? Companion drafts:
`lexicons/com/example/wiki/poll.json` and `lexicons/com/example/wiki/ballotEntry.json`.

## Why custody is load-bearing (atproto mechanics)

- Every atproto record lives in a repo owned by exactly one DID. A record's address is
  `at://<did>/<collection>/<rkey>`, and `com.atproto.repo.createRecord` authenticates as that DID.
  There is NO anonymous-authorship primitive in the protocol: whoever publishes, their DID is on
  the record's address forever.
- Every write is firehose-visible: relays broadcast the (did, collection, rkey, CID) of each create,
  update, and delete. This is exactly why the board is atproto records at all (free, CID-addressed,
  universally observable audit trail), and exactly why the publishing DID must not correlate with
  the voter.
- Records are CID-addressed and signed into the repo's MST, but NOT immutable: the repo owner can
  delete or overwrite any record. "Append-only" is therefore a policy the network can WITNESS
  (deletes are firehose events too), not something the data structure enforces, and relays do not
  archive the firehose indefinitely. Any custody choice needs an independent-witness story for the
  append-only claim.

The board entry itself (see `ballot-spec`'s `BoardEntry`) structurally carries no voter identity:
token, optional message randomizer, RSA-PSS signature, choices. Custody decides whether the record's
ADDRESS leaks what the record's shape protects.

## Option A: voter-published (each voter's own PDS repo)

Each voter writes their own `ballotEntry` record. The entry's at-uri then starts with the voter's
DID, and the firehose event names it. Repo ownership deanonymizes the ballot completely: the
carefully blinded token sits in a record that says who cast it. For a secret ballot this is FATAL.

The only patch is publishing from throwaway/anonymous repos: a fresh DID plus PDS account per voter
per poll, unlinkable to the member. Honest assessment of that patch:

- The membership cannot do it. The DID audit found 0 linked DIDs system-wide and 83 percent
  email-only roster rows; item 16's onboarding walkthrough asks whether members can obtain even ONE
  durable DID. A per-poll anonymous-account ritual is out of the question.
- It does not even deliver unlinkability cleanly: `did:plc` creation is logged with timestamps in
  the public PLC directory (creation bursts around a poll correlate), and the PDS that hosts the
  throwaway account knows who registered it (email, IP).
- What A would have bought is censorship resistance at the PUBLICATION step only (the org cannot
  drop a record it never handles). But the tally still has an inclusion step (which entries the
  close-out counts), so censorship would just move there.

Verdict: fatal unless throwaway repos, which these members will not have. Rejected.

## Option B: org-published (the org repo publishes every entry)

The voter casts to the org's cast endpoint (token, optional randomizer, signature, choices); the org
verifies, appends to its `board_entry` mirror (see the casting SQL in `atproto-domain-model.md`),
and publishes the record from the ORG DID's repo.

Anonymity: preserved. Blinding means the org never saw the nullifier at issuance; at cast it
receives only what the public record will contain anyway; and the publishing DID is the same org
DID for every entry, so the record address says only "the org runs the board". This matches the
spec crate's type-level guarantee: no voter field in the shape, no voter DID in the address.
Residual metadata caveat: the cast ENDPOINT still sees network metadata (IP, session, timing). So
the cast endpoint must be unauthenticated (the token IS the authorization, per the scheme), and
publication timing should not mirror cast timing one-to-one (see open question 2).

Censorship: the new risk B introduces. The org could silently drop a cast (verify it, return OK,
never publish). The mitigation is an inclusion-receipt story:

- Signed inclusion receipt at cast: the org returns a signature over (poll ref, token, choices,
  receipt timestamp) in the cast response. The voter's client stores it (item 10's verify-UX doc
  owns the voter-side flow).
- A valid receipt plus a board with no matching entry at close is PUBLIC censorship evidence:
  anyone verifies the receipt signature and greps the board. Presenting a receipt does not
  deanonymize, because the receipt names no voter and the token is unlinkable; a voter can publish
  it anonymously.
- Signed close-out digest: at close the org publishes a final record (board digest plus tally),
  freezing what "the board" means for receipt checking and re-tally.
- Independent mirrors: at least one third-party firehose consumer mirrors the board collection, so
  a later delete or rewrite of an entry is detectable evidence, not a memory-holed event.

Honesty note on receipts and coercion: this scheme (with or without receipts) provides individual
and universal VERIFIABILITY, not coercion resistance. A voter who chooses to reveal their nullifier
can always prove their vote to a coercer; the receipt adds no coercion surface beyond what the
token itself already carries.

## Option C: hybrids

- C1, dedicated board DID: a separate account (not the org's main repo, not any voter's) holds only
  board entries. With org-held keys this is cosmetic namespacing over B (collections already
  separate records by NSID); with third-party-held keys the censorship trust MOVES to that party
  rather than disappearing, and adds a liveness dependency. Honest, but not materially better than
  B plus mirrors.
- C2, voter-published via throwaway repos: the only non-fatal variant of A, rejected above for this
  membership.
- C3, witness co-signatures: independent observers countersign the close-out digest. This is an
  honest STRENGTHENING of B, not an alternative custody location; keep it available as a B
  extension when the assembly wants observers.

No hybrid exists that gives voters custody without the throwaway-repo prerequisite; anything else
is B with different key holders.

## Recommendation

Org-published (B) with signed inclusion receipts, a signed close-out digest, and at least one
independent mirror. Why it fits the decided scheme:

1. The scheme already routes every cast through the org (the casting SQL, the `board_entry`
   mirror), and the org already learns nothing from it (blind signatures, identity-free entries).
   B adds no new trust for ANONYMITY; it only concentrates the existing publication role.
2. It is the only option this membership can actually use, per the DID-audit and onboarding
   reality.
3. It converts the one risk it does add (censorship) from silent to publicly provable, which is
   exactly the E2E-V posture: verifiability everywhere, trust minimized where it cannot be removed.
4. The spec crate's structural no-DID-linkage guarantee survives to the wire: identical publishing
   DID on every entry, no voter field in the schema.

## Open questions for the owner

1. Receipt signing key: the org's atproto signing key, a dedicated receipt keypair, or the poll's
   issuer key? (The issuer pubkey does stay published after close, but reusing the blind-signature
   RSA key for plain signing mixes key usages; a dedicated key or the org DID key is cleaner.)
2. Publication latency: publish each entry immediately at cast (live board, but per-entry firehose
   timing correlates with cast timing), micro-batched and shuffled (recommended default), or
   all-at-close (max timing privacy, no live board, receipts carry all interim trust)?
3. Receipt scope: token only, or token plus choices? (Recommended: plus choices, so content
   substitution, not just omission, is provable.)
4. Is the signed close-out digest MANDATORY before a tally is official?
5. Mirror commitment: who runs the independent board mirror, and is one running a precondition for
   the first binding poll?
6. Board location: the org's main repo, or a dedicated board account (C1) with org-held keys?
