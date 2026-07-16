# atproto Rewrite Stack Decisions

Forward-looking rewrite decisions for the atproto AppView. Each entry records the concrete decision, the
artifact it is grounded in, and a one-line rationale tied to the future-proofing verdict. These are the
"keep-and-extend" and confirmed-"keep" choices, plus how the rewrite adopts or extends each.

## Database engine

Decision: SPLIT the datastore. Use redb 4.1.x (pure Rust, dual MIT/Apache-2.0) as the authoritative
ballot/tally/roster core, and PostgreSQL 18 accessed through `sqlx` (compile-checked queries, no ORM) as the
rebuildable firehose-materialized view. Track Turso/Limbo as the eventual Rust-native replacement for the view.
This supersedes the earlier "Postgres for everything" pick: it moves the one correctness-critical component
where the Rust-native ethos and the crown-jewel data coincide onto pure Rust, and concedes C only on the
disposable, recomputable half.

- Adoption (the core, redb): depend on `redb = "4.1"`. Store the authoritative state in a single redb file with
  a few typed tables. Ballots go in a table keyed by `(poll_id, voter_did)` whose key uniqueness IS the
  one-vote-per-member invariant: an insert that collides is the rejected duplicate vote, so no SQL `UNIQUE`
  constraint is needed. The ballot value is the opaque `ChaCha20-Poly1305` sealed blob (the engine never sees
  plaintext, matching the crypto-decoupling in the crypto cluster). The official tally is a pure fold over the
  poll's ballot range, not a SQL aggregation. Roster and roles live in their own keyed tables. The core never
  performs a join, a recursive query, or any SQL. Run every write transaction with two-phase-commit durability,
  NOT redb's 1PC+C default: redb's own `design.md` recommends 2PC for adversarial/malicious input, and a public
  election is exactly that threat model; the cost is one extra fsync per commit, negligible at hundreds to
  low-thousands of members.
- Adoption (the view, Postgres 18 + sqlx): materialize the firehose into normal tables keyed by
  `(did, collection, rkey)`; codegen'd lexicon serde structs map to `sqlx` typed columns or `jsonb`; the
  membership hierarchy is a recursive CTE and membership edges are join tables. `sqlx` speaks the Postgres wire
  protocol in Rust, so the Rust process links no C client, though the Postgres server itself is C. This is the
  README's own "C now / Rust when ready" pattern; the correct tracked Rust migration for the VIEW is Turso/Limbo
  (pure-Rust SQLite dialect), a near-drop-in future swap, not TiKV (its mandatory Go Placement Driver cluster
  violates the single-node constraint and is not pure Rust).
- Extension (durability and operations, load-bearing): run redb with `Durability::Immediate` plus 2PC on the
  ballot core; pin the exact redb version and vendor the source; add a per-commit crash-injection test in CI
  (redb's own fuzzer diffs a recovered DB against a `BTreeMap` reference model, so mirror that shape for the
  ballot table); deploy on an enterprise SSD with power-loss protection (or with the volatile write cache
  disabled), because fsync durability is only as strong as the disk. Price the two-datastore operational cost
  honestly: two backup and restore regimes, no cross-store joins by rule (authoritative-plus-view questions are
  two lookups in application code), a documented redb copy-on-quiesce backup and a tested restore drill, and a
  warm Postgres-fallback migration script so the tiny opaque-blob core stays portable in days if redb is ever
  abandoned.
- Rationale: the authoritative core needs durable, serializable, all-or-nothing commits on a single node and
  nothing else, and redb is the only pure-Rust engine that delivers election-grade durability for a store this
  small (COW B+trees, serializable single-writer MVCC, fsync-by-default, checksum-validated crash recovery to
  the last durable commit, a stable on-disk format, and empirical crash fuzzing). Its permissive MIT/Apache-2.0
  license makes bus-factor-1 a fork-if-abandoned risk rather than a vendor-death risk, and the tiny replaceable
  core keeps Postgres as a real fallback. The rebuildable view is recomputable from Jetstream and query-shaped,
  so the Rust-native criterion is far weaker there and Postgres 18's ACID, healthy governance, and
  recursive-CTE/JSONB fit win now, with Turso/Limbo the tracked pure-Rust endgame once its MVCC path leaves
  experimental status and libSQL is no longer the production path.

## Realtime and the firehose consumer

Decision: consume Jetstream (JSON over WebSocket, filtered to `app.radikal.*` collections and member DIDs)
as the change-feed; push local authoritative deltas over one axum WebSocket fed by an in-process broadcast
channel (`tokio::sync::broadcast`), since the AppView is a single persistent process holding the redb core,
the Postgres view, the firehose connection, and the WebSocket server. No DB LISTEN/NOTIFY is required.

- Adoption: use `microcosm-rs`/`atproto-jetstream` for the Jetstream client with cursor handling and
  auto-reconnect; materialize records into the Postgres view; broadcast authoritative deltas (poll open/close,
  projector focus, roster changes) from the redb write path directly onto the in-process channel.
- Extension: persist the Jetstream cursor and implement refetch-on-reconnect plus PDS backfill-on-gap
  (Jetstream has no missed-event backfill); consider self-hosting a Jetstream/tap instance for sovereignty.
- Rationale: the firehose, not a DB-vendor feature, is the vendor-neutral long-lived sync substrate, which
  is why binding realtime to SurrealDB LIVE queries was rejected.

## Signing curves and the repo integrity layer

Decision: mint and sign with P-256 (ECDSA, mandatory low-S); verify both P-256 and k256.

- Adoption: sign atproto repo commits and mint did:key/did:plc keys with the `p256` crate (v0.13, already a
  backend dependency for DPoP/VAPID ES256) or `atrium-crypto`; encode the did:key multikey with p256
  multicodec `0x1200` and base58btc "z" prefix.
- Extension: the verifier must accept both p256 and k256 because firehose repos from other PDSes use either,
  and the atproto reference implementation defaults to k256; add an atproto-exception row to the monorepo
  README's crypto table so Ed25519 is never treated as the atproto signing standard.
- Rationale: atproto permits only p256 and k256 for repo/rotation keys and a minted DID's curve is
  permanent, so this is the load-bearing longevity decision the signing verdict identified.

Decision: encode public records with the deterministic DRISL profile (successor to DAG-CBOR) plus
CIDv1/multihash/multibase.

- Adoption: use the atrium/ipld stack (`serde_ipld_dagcbor` + `ipld-core`, and `cid`/`multihash`/`multibase`
  for CIDs) as the primary encoder; the signed bytes are SHA-256 of the DRISL serialization of the unsigned
  commit (sig field omitted); CIDs use codec `0x71` for structured data and `0x55` (raw) for blobs.
- Extension: ban floats entirely in lexicon-derived models (integers and strings only) so the DRISL
  negative-zero/NaN edge cases can never arise; pin conformance with known-answer/round-trip test vectors
  from a reference impl; treat single-maintainer `atproto-dasl` as an alternative to track, not the primary
  dependency, given the healthy-governance ethos.
- Rationale: generic CBOR produces wrong CIDs/signatures; the deterministic DRISL profile is what the
  network verifies, which is why the CBOR row is kept-and-extended rather than left generic.

## Lexicon-to-atrium codegen pipeline

Decision: lexicons are canonical at the federation boundary only; Rust serde types are canonical for the
private half.

- Adoption: author `wiki.radikal.*` lexicons for the PUBLIC subset (post, statement, resolution, public
  group/event/document, comment); feed them through `atrium-lex`/`atrium-codegen` to generate Rust record
  types; register NSID authority via a DNS TXT record on an org-owned domain per the Lexicon Resolution
  spec.
- Extension: purely-private entities (ballot, voted-dedup, membership-as-affiliation, projector/speaker) get
  NO lexicon at all; genuinely dual-form entities get a public lexicon plus a separate hand-authored Rust
  private type, mapped at an explicit publish/materialize seam; derive the DB DDL from the Rust types.
- Rationale: Lexicon cannot express the private half's uniqueness/cross-field/anonymity invariants and has
  only closed string enums (no algebraic sum type), so making it the private IDL was the inversion the
  schema verdict rejected.

## Validation tiers

Decision: three complementary validation layers, not one schema doing all three.

- Adoption: Lexicon owns record shape at the XRPC/firehose boundary (atrium codegens the types; `esquema` or
  `atproto-lexicon` enforces content constraints like minGraphemes/maxLength at runtime, because atrium
  itself validates only structural shape via serde).
- Extension: Rust type-level invariants own the imperative voting logic (tally arithmetic, roster/role
  enforcement); Nickel contracts (MIT, single-binary, healthy) own declarative config authoring (permission
  matrices, lexicon-NSID manifests) where lazy composition and readable error messages are the
  differentiator. Keep Nickel to build/config-time evaluation, never per-ballot hot paths; do not keep JSON
  Schema as a third hand-maintained copy of the record shape.
- Rationale: Lexicon validates shape and Nickel validates declarative semantics, so conflating them was the
  defect; UTF-8 with grapheme-aware counting (`unicode-segmentation`) keeps client validation matching
  lexicon enforcement.

## Rust server libraries

Decision: axum (on hyper/tower/tokio) is the server framework; reqwest (rustls-tls) is the single outbound
client; rustls is the TLS stack.

- Adoption: axum routes expose XRPC (`com.atproto.*` proxied plus `app.radikal.*` queries), the OAuth
  callback, and one `/ws` WebSocket per client multiplexing all live channels; reuse ONE configured
  `reqwest::Client` (connection-pooled, HTTP/2, `default-features=false` + `rustls-tls` + webpki-roots) as
  the transport for `atrium-xrpc` and all PDS/plc.directory/DID-doc/handle-resolution calls; run as a
  persistent process so the firehose connection, WS server, and DB pool stay live.
- Extension: pin reqwest to the SAME version atrium resolves (0.12 today) so the connection pool is actually
  shared, and defer the 0.13 bump until `atrium-xrpc-client` releases a 0.13-based version (0.12 and 0.13
  are semver-incompatible for a 0.x crate and would fork the pool and TLS stack). Keep the SSRF allow/deny
  host validation on any user-supplied PDS endpoint.
- Rationale: axum is already in the interim tree (zero migration) and reqwest/rustls align with the
  OpenSSL-avoidance ethos, which is why these were confirmed keep/keep-and-extend.

Decision on TLS provider and inbound termination (corrected): keep rustls, but prefer edge termination for
inbound TLS and frame the crypto-provider choice explicitly.

- Adoption: the current build resolves to the `ring` provider (reqwest 0.12's `rustls-tls` hardcodes ring;
  the Cargo.lock files contain zero aws-lc entries), which keeps cross-compile/musl builds trivial. There is
  NO FIPS path today, contrary to the original "aws-lc-rs default" assumption.
- Extension: prefer terminating inbound TLS at the Caddy/Moella edge over adding new in-process ACME
  lifecycle and private-key custody surface via tokio-rustls/rustls-acme; if post-quantum hybrid key
  exchange (X25519MLKEM768, aws-lc-rs-only) or FIPS ever becomes a requirement, that is a deliberate switch
  to aws-lc-rs, at which point NASM/CMake/musl cross-compile pinning becomes a live concern. Note the
  Prossimo/ISRG maintainer-contract cliff (Mar 2026) while treating the Rust Innovation Lab fiscal
  sponsorship as the durable governance signal.
- Rationale: post-quantum hybrid KEX, not FIPS, is the stronger future-proofing lever for a
  sovereignty-minded civic tool, and the ring-vs-aws-lc-rs decision should be made deliberately rather than
  assumed.

## atproto OAuth and identity

Decision: adopt `atrium-oauth` to retire the hand-rolled PKCE/DPoP/JWK code; resolve DIDs through a
method-agnostic resolver.

- Adoption: `atrium-oauth` for the atproto OAuth client (DPoP P-256, PAR, PKCE S256); `atrium-identity` for
  did:plc + did:web + did:key resolution; own a thin wrapper layer so a future breaking atrium 0.x bump is a
  one-file change and pin exact 0.25.x versions gated behind the vote/ballot test suite.
- PDS-agnostic (owner decision): the AppView does NOT run or mandate its own PDS. Members and the org each
  register with whatever atproto host they choose (Eurosky, Bluesky, w.social, or a self-hosted PDS); the
  AppView authenticates whoever logs in via atproto OAuth and resolves their DID method-agnostically. The
  org's signing-key custody is therefore the responsibility of whatever PDS the org picked, not this codebase.
- Extension (optional, not required): consume a self-auditing PLC read-replica (the Feb-2026 reference pattern
  on the independent `go-didplc` codebase, `/export/stream` websocket, PLC spec v0.3.0) to keep member
  identities verifiable during a plc.directory outage. This is a resilience nice-to-have, not a launch blocker.
- Rationale: DID-as-identity is the best migration decision, and staying host-agnostic (register anywhere)
  maximizes the portability/sovereignty win over ActivityPub without the AppView taking on PDS operations.

## Crypto for the private half

Decision: ChaCha20-Poly1305 seals the at-rest server-held material; HKDF-SHA256 derives the keys. (Ballot
ANONYMITY is now handled by a full cryptographic end-to-end-verifiable scheme, see "Voting integrity" below,
not by server-side sealing alone; these AEAD primitives cover the session blob and any server-held keys.)

- Adoption: carry the existing `seal()`/`open()` (ChaCha20-Poly1305, `chacha20poly1305` v0.10) forward for
  the at-rest `atproto_session` blob and any server-held key material; the `atproto_session` blob lives ONLY
  in the redb core, never as a public atproto record.
- Extension: upgrade to XChaCha20-Poly1305 (24-byte nonce) or per-record KDF-derived keys to remove the
  96-bit random-nonce birthday ceiling; replace the bare `SHA-256(secret)` key derivation with HKDF-SHA256
  (the `hkdf` v0.12 crate, already used on the RFC 8291 push path) using per-purpose `info` labels for domain
  separation between the cookie key and any ballot-key material; keep Argon2id reserved for a hypothetical
  future admin/recovery-code password and `age` reserved for out-of-band operator secrets only.
- Rationale: ChaCha20-Poly1305 is a constant-time pure-Rust AEAD that avoids OpenSSL, and HKDF is the right
  KDF for already-high-entropy inputs, so using Argon2 to stretch them would be a category error.

## Voting integrity (owner decisions: full cryptographic, eligibility, delegation, public audit)

Decision: move from the interim "authenticated insert, no owner_id" secret ballot to a FULL cryptographic
end-to-end-verifiable (E2E-V) scheme, with per-poll voting eligibility (only selected members may vote),
delegation rules (proxy/weighted voting is IN, reversing the earlier "dropped" note), and INDEPENDENTLY
auditable tallies. The specific E2E-V scheme is an open sub-decision (see below and the open-decisions doc).

- Eligibility + delegation (org-authoritative, redb core): the org maintains a per-poll eligibility roster
  with a weight per eligible voter; a delegation is a signed assignment that moves a voter's weight to a
  delegate, resolved into the weight column before a poll opens. This is authoritative state, so it lives in
  redb, never as a public record.
- Anonymity + audit (the E2E-V layer): eligibility is separated from the ballot so the tally is publicly
  verifiable without linking a ballot to a voter. The atproto angle is the natural fit for the public audit
  trail: the append-only public bulletin board of cast ballots can BE atproto records (CID-addressed,
  firehose-visible, immutable), giving universal verifiability for free, while a blind-signature or
  homomorphic layer provides the anonymity + eligibility.
- Open sub-decision (the scheme): blind-signature tokens + public board (simplest; org blind-signs one
  weighted eligibility token per voter, cast anonymously to the board, double-vote blocked by token) vs
  homomorphic tally (Helios-style exponential ElGamal + threshold decryption; strongest privacy-preserving
  universal verifiability) vs a verifiable mixnet. Note the hard tension: delegation/liquid-democracy is in
  fundamental conflict with unlinkability (a resolved delegation chain can re-identify), so the scheme must
  fix HOW much delegation is exposed. To be settled before implementation.
- Rationale: a public election tool must be independently auditable AND ballot-secret; server-trust is not
  sufficient, which is why the owner chose the full cryptographic route.
