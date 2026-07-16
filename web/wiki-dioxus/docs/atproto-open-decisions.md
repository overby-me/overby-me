# atproto Rewrite: Decision Log

A running log of the product/architecture decisions the atproto rewrite forces. Decided items record the
owner's call; open items list the options and a lean, to be settled one at a time. Stack-level technical
decisions live in `stack-futureproofing-eval.md` and `atproto-stack-decisions.md`; this file tracks the
choices that need a human call.

## Decided (2026-07-16)

- **Datastore**: Turso Database (MIT, mostly-pure-Rust, SQLite+Postgres-compatible, embeddable + server +
  WASM, SQLite on-disk file-format, Antithesis-tested, corporate-backed). One engine for the ballot core, the
  rebuildable firehose view, and an optional firehose-fed WASM client cache. View on Turso from day one
  (rebuildable = zero risk); ballot core on Turso once it is 1.0 with proven crash-recovery, with plain SQLite
  as a lossless file-format bridge (or redb+replication as a pure-Rust fallback) in the interim. Mandatory
  off-node replication of the append-only ballot log is the decisive integrity control regardless of engine.
  Chosen because it uniquely hits Rust + corporate-backed + SQL + open-license (MIT, not BSL) + WASM client.
- **UI toolkit**: Dioxus everywhere (it renders beyond the web: desktop/mobile/native via Blitz). Public
  permalinks served as static HTML via `dioxus-ssr` (no hydration). Leptos rejected (web-only). Mojo dropped
  from the frontend.
- **atproto signing curve**: P-256 for minting/signing; verify both P-256 and k256. (Ed25519 is barred by
  atproto for repo/rotation keys.)
- **Identity / PDS**: PDS-agnostic. Members and the org register with whatever host they choose (Eurosky,
  Bluesky, w.social, or self-hosted). The AppView does not run or mandate a PDS; it authenticates whoever
  logs in via atproto OAuth and resolves DIDs method-agnostically.
- **Secret ballot**: full cryptographic end-to-end-verifiable (E2E-V). Not "trust the server."
- **Voting eligibility + delegation**: only selected members may vote per poll (a per-poll eligibility
  roster with a weight per voter); delegation / proxy voting is IN (this reverses the earlier "delegation
  dropped" note). Delegation resolves into the weight before a poll opens.
- **Tally audit**: tallies must be independently auditable by users (universal verifiability).
- **Secret-ballot scheme**: blind-signature eligibility tokens + a public bulletin board that IS atproto
  records (free public audit trail). The voter casts anonymously with the token(s) to the board; a double
  vote collides on token uniqueness. Delegation is resolved to vote-weights server-side BEFORE token
  issuance, so delegation is visible to the org but never on the public board (this is how the
  delegation-vs-anonymity tension is resolved).
- **Token encoding + issuer-key scoping** (decided 2026-07-16; supersedes the earlier "one weighted
  eligibility token per voter" phrasing above and in `atproto-stack-decisions.md`): UNIT tokens and
  PER-POLL issuer keys. A voter with resolved weight N is issued N identical unit tokens, so every board
  entry looks the same: weight never appears on the public board (a weight-carrying token would make a
  lone weight-5 delegate uniquely identifiable, shrinking the anonymity set exactly where delegation
  concentrates power) and the tally stays a plain count, at the cost of board size growing with total
  weight. The org mints a fresh RSA issuer keypair per poll and publishes the pubkey to the board before
  the poll opens: blinding hides the message from the issuer, so per-poll keys are what cryptographically
  bind a token to its poll (a token for poll A cannot spend on poll B), give natural expiry, and cap a key
  compromise at one poll. The two sub-decisions interact: weight-carrying tokens would have required a
  separate issuer key per weight class anyway (the blind signer never sees the message), a second reason
  unit tokens win.
- **Visibility**: public by default (opt-out), with an explicit per-group/event toggle so a group or event
  can set its content public or private. Ballots, roster, and membership-as-affiliation stay always-private
  regardless of the toggle.
- **Migration**: big-bang cutover, accepted as low-risk because the old wiki will be stable by then.
- **Lexicon scope** (decided 2026-07-16, closes the OPEN-1 entry that used to sit below): lexicons are
  canonical at the federation boundary ONLY; hand-authored Rust serde types are canonical for the private
  half, and the DB DDL derives from the Rust types. This is what the shipped `lexicons/` already implement
  de facto (always-private entities deliberately have no lexicon). The losing option (lexicons for all
  entities, one codegen pipeline for everything) was rejected because Lexicon cannot express the private
  half's uniqueness, cross-field, and anonymity invariants, has only closed string enums (no algebraic sum
  types), and would publish schema contracts for records that never federate, taxing private-side iteration
  with versioning ceremony. `atproto-domain-model.md` is reconciled to this stance.

## Open (need a call)

- **README P-256 atproto-exception row**: expanded into the concrete before/after rows below (owner asked to
  see the edits before deciding). Awaiting go-ahead to apply to the monorepo-root README.

## Proposed README edits (for the "expand on this" ask)

Concrete edits to the monorepo-root `README.md`, pending approval:

1. Signing row (Security & Cryptography table). Add an atproto-scoped signing row so the table stops
   blessing atproto while listing a curve atproto forbids:
   - Keep: `Signing (general) | Ed25519` for SSH/age-adjacent, non-atproto use.
   - Add: `Signing (atproto) | P-256 / secp256k1 (ECDSA, low-S)` with a note that did:key uses multicodec
     `0x1200` (p256) and `0xe7` (k256), base58btc "z". Rationale cell: atproto permits only p256/k256 for
     repo-commit and did:plc rotation keys; Ed25519 is barred there.
2. Web Protocol row (Network table). Reword so HTTP/3 is an optional edge enhancement, not the successor:
   `HTTP/2 (primary) | HTTP/3 optional edge for mobile/lossy links`, since HTTP/3 adoption is ~21% and flat.
3. TLS row (Frameworks & Libraries table). Note the provider reality: `Rustls (ring provider today; aws-lc-rs
   only if PQ-hybrid KEX or FIPS is required)`, so the ring-vs-aws-lc-rs choice is explicit rather than assumed.

## Engineering defaults taken (override if you disagree)

- TLS terminated at the Caddy edge; stay on the `ring` rustls provider until PQ-hybrid KEX or FIPS forces
  aws-lc-rs. HTTP/2 + TCP primary; HTTP/3 optional edge only.
- Realtime via one multiplexed axum WebSocket fed by an in-process broadcast channel (the AppView is one
  persistent process holding redb for both core and view, plus the firehose).
- API surface: XRPC + `wiki.radikal.*` lexicons at the public boundary; a typed internal Rust API for the
  Dioxus client. GraphQL/cynic dropped.
- `reqwest` pinned to atrium's resolved version (0.12 today) so the connection pool is shared.
- Files/blobs: PDS blobs for public files, an org object store for private (replacing NHost storage).
