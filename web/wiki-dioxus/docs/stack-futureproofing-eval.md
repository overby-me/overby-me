# Stack Future-Proofing Evaluation (atproto rewrite of the civic-assembly tool)

This document consolidates a per-cluster future-proofing audit of the tech-radar (the monorepo-root
README at `/home/overby.me/Work/overby.me/README.md`) against the atproto rewrite of `web/wiki-dioxus`.
Every component receives a decisive verdict: keep, keep-and-extend, or change. Where the adversarial
verification refuted or corrected a verdict, the corrected recommendation is the one recorded here; where
verification confirmed a verdict, that is stated plainly.

## Verdict table

### Cluster: Data / database layer (the AppView store)

| Component | Current | Verdict | Why | Action |
|-|-|-|-|-|
| Meta Database / authoritative store | Hasura to SurrealDB (BSL 1.1) | change | Needs Rust + corporate-backed + SQL + open-license + WASM-capable; SurrealDB is BSL + a CVE, redb is single-maintainer, bare SQLite/Postgres is C with no client story | Turso Database (see crux 1): MIT, mostly-pure-Rust, SQLite+Postgres-compatible, embeddable + server + WASM, SQLite file-format (portable), Antithesis-tested. View on it from day one; ballot core on it once 1.0-proven (SQLite as a lossless bridge, redb+replication as pure-Rust fallback) plus mandatory off-node ballot-log replication |
| Database | Postgres to TiKV | change | TiKV is heavy distributed infra (Go Placement Driver); bare Postgres/SQLite are C with no WASM-client story | Drop TiKV; the store is Turso Database (SQLite+Postgres-compatible, one engine for core + view + an optional WASM client cache); plain SQLite or redb are named fallbacks/bridges only |
| Storage Engine | Sled / Fjall (R&D), RocksDB (legacy) | change (to N/A-for-AppView) | The AppView workload is relational (uniqueness, joins, aggregation); a raw KV is the wrong abstraction. Only relevant under a self-hosted PDS | Prefer Fjall or redb over Sled if an embedded engine is ever needed under a query engine or PDS; never Sled for durability-critical ballots |
| Realtime / sync transport | SurrealDB LIVE queries | keep-and-extend | The real change-feed is the atproto firehose, not a DB-vendor feature; binding realtime to SurrealDB LIVE re-creates the Hasura lock-in being unwound | Consume Jetstream filtered to `app.radikal.*`; push authoritative deltas over one axum WS fed by an in-process broadcast channel (the AppView is a single process holding Turso for both core and view plus the firehose, so no DB LISTEN/NOTIFY is needed) |
| API surface | GraphQL (Hasura) | change | GraphQL was Hasura's auto-generated surface, not a chosen contract; atproto's native convention is XRPC, and lexicons already drive the types | Expose XRPC query/procedure lexicons + typed axum handlers; drop GraphQL and cynic |
| Web Server | Caddy to Moella | keep (Caddy) / R&D-only (Moella) | Moella/Kvarn is single-maintainer, hobby-scale, WS/reverse-proxy paths only recently stabilized; wrong bet for a tool running elections | Keep Caddy (or terminate TLS directly in axum); pilot Moella on a non-critical surface before promoting |

### Cluster: Federation protocol + identity

| Component | Current | Verdict | Why | Action |
|-|-|-|-|-|
| Social Protocol | AT Protocol (current) | keep-and-extend | Right substrate: portable DID identity + firehose; IETF ATP WG chartered in early 2026. ActivityPub ties identity to the home instance; Nostr has no structured-schema story | Keep atproto behind a protocol-agnostic core; treat lexicons + the private-data model as project-owned artifacts (the IETF charter deliberately scopes them out) |
| Identity / DID method | did:plc via plc.directory | keep-and-extend (nuanced) | DID-as-identity is the best migration decision; did:plc's single-directory dependency is the sharpest civic risk, but Feb-2026 self-auditing PLC read-replicas now mitigate it | Method-agnostic resolver (did:plc + did:web); run/consume a self-auditing PLC replica for members; resolve the org signing-key custody question before launch |
| Org-authoritative state + secret ballots | DB-only, not records | keep | A DID-signed record cannot be anonymous by construction, so server-mediated secret ballots are a permanent domain requirement, not a workaround; atproto private-data is still a draft | Keep ballots/dedup/roster/ephemeral state DB-only; publish only the opt-in public subset as records; default membership to always-private |
| Web Protocol + Transport | HTTP/2 to HTTP/3; TCP to QUIC | keep-and-extend (corrected) | HTTP/3 adoption is ~21% and plateaued/falling (not ~35% and rising); HTTP/2 is ~51% and growing and outperforms QUIC on high-bandwidth links | Keep HTTP/2+TCP as the non-negotiable primary; offer HTTP/3+QUIC as an optional edge enhancement for mobile/lossy clients only; never HTTP/3-only |

### Cluster: Frontend / UI toolkit

| Component | Current | Verdict | Why | Action |
|-|-|-|-|-|
| UI Toolkit | React/MUI to Dioxus 0.7 WASM | keep-and-extend | Dioxus is the ethos fit (MIT/Apache, pure Rust) AND the only mature Rust UI toolkit that targets beyond the web (one component tree renders to web WASM, desktop, mobile, and native via the Blitz/wgpu renderer); a web-only framework such as Leptos fails this cross-platform requirement outright. The one real gap is that a pure CSR WASM SPA breaks OG/AI-crawler unfurling and cold-load on the public surface | Keep Dioxus everywhere; serve public permalinks as static HTML via `dioxus-ssr` (no client hydration), with the interactive Dioxus app mounting only on the authenticated surface; drop Mojo from this row |
| UI Components | MUI to dioxus-primitives (0.0.1, git-pinned) | keep-and-extend | Headless ARIA primitives under an owned M3 skin is the right architecture; the hand-rolled M3 widget layer is the real, portable asset | Consolidate widgets into an in-repo M3 crate; add axe/Playwright a11y assertions targeting the hand-rolled focus/overlay code; bind widget props to codegen'd lexicon models |
| Browser Engine | Gecko to Servo | keep | Servo (LF Europe, Igalia) is the right long-horizon target but is 0.0.x research-grade; correct as an aspirational test target only | Test primarily on a mainstream engine; SSR public pages so Servo's incomplete coverage never blocks a voter |
| ECMAScript Engine | V8 to Boa / Nova | keep | Off the critical path (WASM-first, JS-light app); Boa is mature, Nova is a longer-odds bet | Prefer Boa over Nova for any embedded-JS need; keep the app's JS surface minimal so this row stays off the hot path |
| Web Language (frontend) | TypeScript to Mojo | change | Mojo is single-vendor (Modular), compiler still closed under a proprietary license, and has no browser-WASM UI or a11y story; disqualifying for the frontend | Keep the frontend on Rust; confine Mojo to backend GPU/numeric/inference; remove "Mojo WASM Toolkit" from the UI Toolkit R&D cell |

### Cluster: Cryptography

| Component | Current | Verdict | Why | Action |
|-|-|-|-|-|
| Signing curve | Ed25519 (README) | change | atproto permits ONLY p256 and k256 for repo commits and did:key/did:plc; Ed25519 is barred for repo-auth and rotation keys. A minted DID's curve is permanent | Mint and sign with P-256 (the `p256` crate is already a backend dep); verify both p256 and k256; enforce low-S; add an atproto-exception README row |
| Binary Object Notation | CBOR (generic) | keep-and-extend | CBOR (RFC 8949) is the right open base, but atproto requires the deterministic DRISL profile (successor to DAG-CBOR) + CIDv1/multihash/multibase | Encode public records with DRISL via atrium/ipld crates; ban floats in lexicon-derived models; add a README note that the atproto row is DRISL-over-CBOR |
| Symmetric AEAD | AES-GCM + ChaCha20-Poly1305 | keep | ChaCha20-Poly1305 (RFC 8439) is a constant-time pure-Rust primitive that avoids OpenSSL; correct for sealing sessions and secret-ballot rows | Carry seal/open forward; upgrade to XChaCha20-Poly1305 or per-record KDF-derived keys to remove the 96-bit random-nonce ceiling |
| Asymmetric (age) | age (GPG legacy) | keep | Small, open, maintained pure-Rust format for ops secrets; orthogonal to the atproto data plane | Keep age for out-of-band operator secrets only; not for per-record ballot/session encryption |
| Key Derivation | Argon2 | keep-and-extend (nuanced) | Argon2id is the right password KDF, but the rewrite has no password store (identity is atproto DID + OAuth). Its relevance, not its longevity, is the issue | Keep Argon2id in the table for future admin/recovery-code hashing; use HKDF-SHA256 with per-purpose `info` labels to derive keys from STATE_SECRET |
| Meta Database (crypto-material store) | SurrealDB (BSL) vs Postgres+sqlx | change | The crown-jewel encrypted private state belongs on the correctness-verified store, decoupled from the crypto | Store sealed sessions and encrypted ballots in the Turso ballot core as opaque bytes (per crux 1), so the engine choice is decoupled from the crypto |

### Cluster: Schema language + data notation

| Component | Current | Verdict | Why | Action |
|-|-|-|-|-|
| Lexicon (public wire format) | atproto Lexicon | keep | Lexicon is the protocol's contract at the federation boundary; the Lexicon Resolution RFC moves NSID authority to DID+DNS TXT, de-risking the single steward | Author `wiki.radikal.*` lexicons for the public subset; register NSID authority via a DNS TXT record on an org-owned domain; codegen with atrium |
| Lexicon for the private half | one lexicon drives both record + DB row | change | Coupling private schema to atproto's data-model constraints (integer-only numbers, no cross-field/uniqueness/sum types) for zero federation benefit; those invariants live in DB DDL and Rust anyway | Make Rust the source of truth for private entities; purely-private entities (ballot, voted, membership-as-secret, projector) get NO lexicon; map only genuinely dual-form entities at the publish seam |
| Binary Object Notation (DRISL) | CBOR | keep-and-extend | See crypto cluster; DRISL is the current spec vocabulary for atproto's deterministic CBOR | Encode with a strict DRISL/DAG-CBOR profile; conformance-test against real PDS output; prefer atrium/ipld crates over single-maintainer alternatives |
| Schema Validation | JSON Schema to Nickel (R&D) | keep-and-extend | Nickel (MIT, healthy) expresses refinement/cross-field invariants Lexicon and JSON Schema cannot; complementary to, not a competitor of, Lexicon | Three-tier: Lexicon owns record shape (atrium types + esquema for content constraints), Rust owns imperative tally/roster logic, Nickel owns declarative config authoring; drop JSON Schema as a third copy |
| Object Notation | JSON to KDL / EON | keep | JSON is non-negotiable: lexicon files and Jetstream are JSON; ECMA-404/RFC 8259 governance is maximally stable | Keep all lexicons and firehose ingestion in JSON; confine KDL/EON to human-authored config/docs |
| Text | UTF-8 | keep | UTF-8 is baked into atproto (grapheme/byte-counted string limits, UTF-8 CBOR text) | Adopt grapheme-aware counting (`unicode-segmentation`) so client validation matches lexicon enforcement |

### Cluster: Backend Rust libraries + runtime

| Component | Current | Verdict | Why | Action |
|-|-|-|-|-|
| Systems Language (Rust) | Rust | keep | Memory-safe, Rust-Foundation-governed, the lingua franca of the atproto server ecosystem; one language spans frontend, AppView, firehose | Single Rust workspace; shared serde model crate; pin MSRV; gate server-only deps behind features to keep WASM bundle small |
| HTTP Protocol (Hyper) | Hyper | keep | hyper v1 is stable with a 3-year support window; load-bearing under axum + reqwest | Consume via axum (inbound) and reqwest (outbound); never build XRPC directly on hyper |
| HTTP Client (Reqwest) | Reqwest 0.12 (rustls-tls, no OpenSSL) | keep-and-extend (corrected) | De-facto client, aligns with the OpenSSL-avoidance ethos; atrium-xrpc ships a reqwest backend | Reuse ONE configured client; pin the SAME reqwest version atrium resolves to (0.12 today) so the pool is shared; do NOT bump to 0.13 until atrium does; keep the SSRF guard |
| TLS (Rustls) | Rustls (OpenSSL legacy) | keep (corrected) | Strongest governance signal (Rust Innovation Lab); but the build resolves to the `ring` provider, not aws-lc-rs, so there is no FIPS path today and post-quantum hybrid KEX (aws-lc-rs-only) is the real future driver | Keep rustls outbound; prefer Caddy/Moella edge for inbound TLS over new in-process ACME surface; frame ring-vs-aws-lc-rs explicitly; note the Mar-2026 maintainer-contract cliff |
| SSH (Russh) | Russh (OpenSSH legacy) | keep (out of scope for the AppView) | The AppView speaks HTTP/XRPC/WS, not SSH; any SSH is deploy-time infra | Do NOT add an SSH crate to the backend; Colmena/OpenSSH or Russh both fine at deploy time |
| WebAssembly Runtime (Wasmtime) | Wasmtime | keep (dormant) | Healthiest server-side WASM runtime but not on this rewrite's path; the tally/authz must be inspectable native Rust, not sandboxed plugins | Do not embed in v1; reserve for a future org-supplied-logic sandbox |
| ECMAScript Runtime (Deno to Bun) | Deno to Bun | keep (build-time only) | The rewrite removes the JS backend entirely; no server JS runtime at runtime | Keep Deno for TS build/codegen scripts; prefer Rust lexicon codegen so JS stays optional |
| atproto SDK (atrium) | not in README; hand-rolled OAuth | keep-and-extend | Maturest Rust atproto SDK (MIT, moved to the atrium-rs org); replaces the hand-rolled DPoP/PKCE | Adopt atrium-oauth/api/identity behind a thin wrapper; pin exact 0.25.x; validate `app.radikal.*` codegen early |
| Firehose / Jetstream consumer | not in README | keep-and-extend | Rust Jetstream ecosystem (microcosm-rs, atproto-jetstream) is production-proven at this scale; avoids raw CBOR subscribeRepos + MST | Consume Jetstream filtered to your collections/DIDs; persist cursor; refetch-on-reconnect + PDS backfill-on-gap; keep authoritative state out of the firehose |
| XRPC / axum + WebSocket | not in README; interim axum 0.8 | keep-and-extend | axum is already in the tree (zero migration); XRPC is JSON/CBOR over HTTP; one axum WS replaces the 9-sockets-per-page problem | One `/ws` per client multiplexing all live channels, driven by the DB change-feed; run as a persistent process |

## Crux decision 1: the AppView datastore (Turso Database)

Recommended decision: Turso Database is the datastore for the whole stack. It is the from-scratch,
MIT-licensed, mostly-pure-Rust rewrite of SQLite: embeddable in-process (the `turso` crate), also a server,
WASM-capable, SQLite-compatible at the language AND on-disk-file-format level, with an in-tree experimental
PostgreSQL wire-protocol + dialect frontend. This supersedes the earlier redb-everywhere and SQLite-everywhere
picks; those remain named fallbacks below.

### Why Turso is the future-proof pick

It is the only candidate that satisfies every constraint this decision accumulated:

- Rust + corporate-backed (Turso Inc.) + MIT. This answers BOTH the Rust-native hard line AND the
  "not a single-maintainer hobby project" concern that ruled out redb, without SurrealDB's BSL license or its
  auth-bypass CVE surface.
- SQL, so the view needs no hand-written query layer (recursive queries, JSON, secondary indexes come from
  the engine). SQLite dialect today; an experimental Postgres wire + dialect frontend is merging in under an
  "LLVM of databases" architecture (one dialect-agnostic core of storage/B-trees/WAL/MVCC, pluggable
  SQLite/Postgres frontends).
- SQLite file-format compatible (same on-disk B-tree and page format). The on-disk format is therefore stable
  and portable, SQLite tooling reads it, and it provides a zero-friction migration bridge to and from plain
  SQLite.
- Embeddable in-process AND a server AND WASM. One engine can back the axum AppView and, optionally, a
  firehose-fed query cache inside the Dioxus WASM client, so client and server share one schema and one SQL
  dialect. (atproto remains the sync layer; the WASM engine is a local cache, not a competing replicator.)
- Correctness engineering: built with Antithesis deterministic-simulation testing targeting SQLite-or-better
  reliability (the FoundationDB / TigerBeetle approach), not merely field-aging.

Status (mid-2026): v0.x, in production at some organizations, 1.0 estimated roughly 6 months out. Because the
rewrite is future-dated, Turso is likely at or past 1.0 by cutover.

### The view

On Turso from day one. The view is firehose-rebuildable, so any residual pre-1.0 durability risk is repaired
by a replay; the risk there is effectively zero.

### The ballot core, and the one caveat

The core (secret ballots, one-vote dedup, tally, roster, eligibility) is the unrecoverable, adversarial path,
so it wants durability that is battle-proven at ship time. Two things make this safe:

1. At cutover, confirm Turso is 1.0 with Antithesis coverage of the crash and recovery paths. If it is not yet
   there, the SQLite file-format compatibility means the core can run on PLAIN SQLite and migrate to Turso
   losslessly later, with no rework. (redb + replication remains a pure-Rust fallback if SQLite's C engine is
   unacceptable even as a bridge.)
2. The decisive election-integrity control is mandatory off-node replication of the append-only ballot log,
   which sits on top of whatever engine backs the core. This does not exist in the backend yet and is the real
   work; the engine choice is secondary to it.

### What this supersedes

redb-everywhere (pure Rust, but a single-maintainer engine and a hand-written view query layer) and
SQLite-everywhere (proven but C, and no client/WASM story) were the interim answers while Turso matured. The
trajectory (SQLite + Postgres dialects, WASM, Antithesis testing, SQLite file format, 1.0 near, corporate
backing) makes Turso the target and both of those the fallbacks.

## Crux decision 2: atproto signing curve (Ed25519 vs P-256 / k256)

Recommended decision: change. P-256 is the curve this project MINTS and SIGNS with; the AppView MUST verify
both P-256 and k256. Ed25519 must NOT be listed as the signing standard for anything atproto-facing.

Verification confirmed every load-bearing claim against the live spec:

- atproto supports ONLY p256 (NIST P-256/secp256r1) and k256 (secp256k1) for repo-commit signing and
  did:key/did:plc verification and rotation keys. Ed25519 is not permitted for repo-auth or rotation keys.
  The June 2025 did:plc relaxation loosened only the non-atproto verificationMethod field; rotation and
  repo-auth keys remain p256/k256-only.
- A DID minted on the wrong curve is a permanent identity mistake, and low-S normalization is mandatory or
  conformant verifiers reject the signature.
- P-256 is the deliberate choice for THIS project because the `p256` crate (v0.13, features ecdh/ecdsa/jwk)
  is ALREADY a backend dependency for DPoP/VAPID ES256, so it adds no supply-chain surface; it is also in
  WebCrypto, TPMs, HSMs, and Secure Enclaves, enabling hardware-backed org keys. The verification's one
  caveat, folded in: k256 is the atproto REFERENCE-IMPLEMENTATION default, so the "canonical" framing is a
  deliberate project choice, not the ecosystem default, and interop cost is zero because conformant
  verifiers must handle both curves.

Should the README note an exception? Yes, explicitly. The signing row lives in the monorepo-root README's
Cryptographic Primitives table (not in the `web/wiki-dioxus` README, which has no crypto table), and that
same table already blesses AT Protocol and ATProto DID as current standards, so it is internally
self-contradictory: it commits to atproto while listing a curve atproto forbids for the load-bearing keys.
Add an atproto-exception row there: Signing (atproto/wiki context) = P-256/secp256k1 (ECDSA, low-S), with
did:key multicodec 0x1200 for p256 and base58btc "z" prefix. Ed25519 may remain a general-purpose,
non-atproto signing note (SSH/age-adjacent) but never the atproto signing standard.

## Crux decision 3: Dioxus and the app shape

Recommended decision: keep Dioxus, change the app shape. The cross-platform requirement (the toolkit must
work beyond the web) reinforces this and removes Leptos from contention: Leptos is a web-only framework
(WASM + server rendering), whereas Dioxus renders one component tree to web WASM, desktop, mobile, and
native (the Blitz/wgpu renderer, no webview). For a civic tool that may want a desktop chair console or a
mobile delegate app later, that single-codebase reach is the decisive long-term reason to stay on Dioxus.

Verification confirmed the "nuanced" verdict but corrected its justification, so the corrected rationale
stands:

- Reframe away from accessibility and no-JS operation. WCAG 2.1 AA and the EU EAA are outcome-based and do
  NOT require server-rendering or no-JS operation; a correctly built WASM SPA can meet AA client-side, and
  Googlebot can render CSR in a delayed second wave. So bundling "screen-reader-first" and "usable with JS
  disabled" into the SSR argument was partly a category error.
- The real drivers that genuinely break under CSR are social/OpenGraph link unfurling, AI-search crawlers,
  and cold-load latency on poor connections, none of which execute JS/WASM. Those alone justify
  server-rendering the public permalinks.
- The SSR mechanism is the genuine open decision, and it stays inside Dioxus (Leptos is out on the
  cross-platform requirement above). Dioxus 0.7 fullstack HYDRATION has confirmed bugs in exactly the path
  you would rely on (wasm-split hydration duplication, routes rendering twice, use_server_future hydration
  mismatches). The fix is to avoid hydration, not to leave Dioxus: render the public permalinks to static
  HTML with the `dioxus-ssr` crate (a plain string render with NO client-side WASM or hydration), and mount
  the interactive Dioxus WASM app only on the authenticated surface. That keeps one component library and
  one renderer across public HTML, the web app, and any future desktop/mobile target, while sidestepping the
  0.7 hydration bugs entirely. Do not bet the public surface on Dioxus fullstack hydration in its current
  0.7 state, and do not split the frontend across two toolkits.
- Governance risk is real but not acute: Dioxus is a small YC-backed startup with no foundation backstop and
  is still 0.x, but its 0.8 roadmap commits to no drastic state-management/fullstack changes, and its raw
  adoption is ~1.5x Leptos's.
- Drop Mojo from this row: no browser-WASM UI story, compiler still closed under a proprietary license,
  contradicts the open/sovereign ethos.

## Crux decision 4: Lexicon as the canonical schema

Recommended decision: keep Lexicon at the federation boundary (public subset); change it for the private
half.

Verification confirmed the "change" verdict and sharpened two phrasings:

- The plan does make Lexicon load-bearing, not merely documentation: `atproto-domain-model.md` states one
  atrium-codegen'd type model drives both the public record wire-format AND the private DB rows, including
  ballot and membership. That inverts ownership, making the private Rust type downstream of a foreign JSON
  IDL for state that will never be a record. That inversion is the defect.
- Lexicon genuinely cannot express what the authoritative half needs: uniqueness constraints (one-vote
  dedup), cross-field refinement (tallyFor + tallyAgainst == turnout), the voted/ballot anonymity
  separation, or a Rust-style tagged union. Phrasing correction folded in: Lexicon DOES have a closed string
  `enum` plus knownValues/const, so the accurate charge is "no algebraic sum type; only closed,
  evolution-brittle string enums," and atproto's data model is integer-only (no float/decimal at all,
  stronger than "no native decimal").
- Scope refinement folded in: purely-private entities with no public projection (ballot, voted,
  membership-as-affiliation, projector/speaker) should have NO lexicon at all, not a mapped pair. Only the
  genuinely dual-form entities (post, statement, resolution, document) get a public lexicon plus a separate
  private Rust type, mapped at the publish seam. This shrinks the two-schema surface the verdict itself
  flagged as the drift risk.

Make Rust the single source of truth for the private half, derive the DB DDL from the Rust types, keep
Lexicon strictly at the public boundary, and use Nickel contracts for declarative authoritative-rule and
config validation.

## Change these (shortlist)

- Datastore is Turso Database (MIT, mostly-pure-Rust, SQLite+Postgres-compatible, embeddable + server + WASM,
  SQLite file-format, Antithesis-tested): one engine for the ballot core + the rebuildable view + an optional
  WASM client cache. Plain SQLite (a lossless file-format bridge) and redb+replication are named fallbacks for
  the core until Turso is 1.0-proven; mandatory off-node ballot-log replication is the real integrity control.
- Ed25519 out as the atproto signing standard; P-256 in for minting/signing, verify both p256 and k256, add
  an atproto-exception README row.
- GraphQL/cynic out; XRPC + typed axum handlers in.
- Mojo out of the frontend/UI-toolkit R&D cell; Rust stays the frontend language.
- Lexicon out as the IDL for purely-private DB-only entities; Rust serde types in as their source of truth.
- Public read surface out of the pure-CSR WASM SPA; static HTML via `dioxus-ssr` (no hydration) in. Leptos
  rejected: it is web-only, and the toolkit must work beyond the web (desktop/mobile/native via Dioxus/Blitz).
- Do NOT bump reqwest to 0.13 yet; pin to atrium's 0.12 so the connection pool is shared.
- Correct the TLS row: no FIPS path today (build is on ring, not aws-lc-rs); frame ring-vs-aws-lc-rs
  explicitly and name post-quantum hybrid KEX as the real future driver.
- Correct the HTTP/3 row: HTTP/2+TCP is the non-negotiable primary; HTTP/3 is an optional mobile-edge
  enhancement only.

## Already right, extend in the rewrite (shortlist)

- AT Protocol as the substrate, behind a protocol-agnostic core, with lexicons and the private-data model
  vendored as project-owned artifacts.
- DID-as-identity, extended with a method-agnostic resolver (did:plc + did:web) and a self-auditing PLC
  read-replica for members.
- Rust, hyper, axum, reqwest (rustls-tls), rustls, tokio: the entire backend runtime spine, extended with
  atrium (oauth/api/identity) and a Jetstream consumer.
- ChaCha20-Poly1305 for the private half, extended to XChaCha20 or per-record derived keys for the ballot
  store.
- Argon2id kept in the table, with HKDF-SHA256 doing the actual per-purpose key derivation from
  STATE_SECRET.
- CBOR, extended to the strict DRISL profile plus CIDv1/multihash/multibase via atrium/ipld crates.
- UTF-8 and JSON kept as-is (both are baked into atproto), with grapheme-aware length counting.
- dioxus-primitives kept as a thin a11y-behavior layer under owned M3 widgets, extended with axe/Playwright
  a11y assertions.
- Servo, Boa, Wasmtime, age, Russh kept as correct-but-off-the-critical-path picks for this rewrite.
