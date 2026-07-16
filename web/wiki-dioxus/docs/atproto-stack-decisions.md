# atproto Rewrite Stack Decisions

Forward-looking rewrite decisions for the atproto AppView. Each entry records the concrete decision, the
artifact it is grounded in, and a one-line rationale tied to the future-proofing verdict. These are the
"keep-and-extend" and confirmed-"keep" choices, plus how the rewrite adopts or extends each.

## Database engine

Decision: PostgreSQL as the authoritative store, accessed through `sqlx` with compile-checked queries and no
ORM.

- Adoption: materialize the firehose into normal tables keyed by `(did, collection, rkey)`; codegen'd
  lexicon serde structs map to `sqlx` typed columns or `jsonb`.
- Extension: model ballots append-only with a `UNIQUE (poll, voter)` constraint and compute the tally by
  aggregation; store sealed sessions and encrypted secret-ballot rows as opaque `BYTEA` so the engine never
  sees plaintext and the crypto is decoupled from the DB choice.
- Rationale: real ACID for the official-tally and secret-ballot-dedup path, plus PostgreSQL-License
  governance, is the future-proof choice the correctness verdict demands over BSL/unverified SurrealDB.

## Realtime and the firehose consumer

Decision: consume Jetstream (JSON over WebSocket, filtered to `app.radikal.*` collections and member DIDs)
as the change-feed; push local authoritative deltas over one axum WebSocket fed by Postgres LISTEN/NOTIFY.

- Adoption: use `microcosm-rs`/`atproto-jetstream` for the Jetstream client with cursor handling and
  auto-reconnect; materialize records into Postgres.
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
- Extension: run or consume a self-auditing PLC read-replica (the Feb-2026 reference pattern on the
  independent `go-didplc` codebase, `/export/stream` websocket, PLC spec v0.3.0) to keep member identities
  verifiable during a plc.directory outage; issue the org's own DID and resolve the org signing-key custody
  question (HSM/multi-sig backups, or run your own PDS) before launch.
- Rationale: DID-as-identity is the best migration decision, and a method-agnostic resolver plus a mirrored,
  self-auditing directory earns the "portable identity beats ActivityPub" claim under the sovereignty ethos.

## Crypto for the private half

Decision: ChaCha20-Poly1305 seals the private, server-only material; HKDF-SHA256 derives the keys.

- Adoption: carry the existing `seal()`/`open()` (ChaCha20-Poly1305, `chacha20poly1305` v0.10) forward for
  the at-rest `atproto_session` blob and secret-ballot storage; these rows live ONLY in Postgres, never as
  public atproto records, because a DID-signed record cannot be anonymous by construction.
- Extension: upgrade to XChaCha20-Poly1305 (24-byte nonce) or per-record KDF-derived keys to remove the
  96-bit random-nonce birthday ceiling as ballot volume grows; replace the bare `SHA-256(secret)` key
  derivation with HKDF-SHA256 (the `hkdf` v0.12 crate, already used on the RFC 8291 push path) using
  per-purpose `info` labels for domain separation between the cookie key and the ballot-store key; keep
  Argon2id reserved for a hypothetical future admin/recovery-code password and `age` reserved for
  out-of-band operator secrets only.
- Rationale: ChaCha20-Poly1305 is a constant-time pure-Rust AEAD that avoids OpenSSL, and HKDF is the right
  KDF for already-high-entropy inputs, so using Argon2 to stretch them would be a category error.
