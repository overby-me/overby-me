# atproto Rewrite: Decision Log

A running log of the product/architecture decisions the atproto rewrite forces. Decided items record the
owner's call; open items list the options and a lean, to be settled one at a time. Stack-level technical
decisions live in `stack-futureproofing-eval.md` and `atproto-stack-decisions.md`; this file tracks the
choices that need a human call.

## Decided (2026-07-16)

- **Datastore**: fully Rust, no Postgres. redb (pure Rust, 2PC durability) for BOTH the ballot/tally/roster
  core AND the rebuildable firehose view (hand-written indexes + tantivy for full-text search). Turso/Limbo
  (pure-Rust SQLite) an optional SQL layer for the view only, acceptable because the view is rebuildable.
  Postgres dropped entirely.
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
  records (free public audit trail). The org blind-signs one weighted eligibility token per eligible voter;
  the voter casts anonymously with it to the board; a double vote collides on token uniqueness. Delegation is
  resolved to vote-weights server-side BEFORE token issuance, so delegation is visible to the org but never on
  the public board (this is how the delegation-vs-anonymity tension is resolved).
- **Visibility**: public by default (opt-out), with an explicit per-group/event toggle so a group or event
  can set its content public or private. Ballots, roster, and membership-as-affiliation stay always-private
  regardless of the toggle.
- **Migration**: big-bang cutover, accepted as low-risk because the old wiki will be stable by then.

## Open (need a call)

- **README P-256 atproto-exception row**: expanded into the concrete before/after rows below (owner asked to
  see the edits before deciding). Awaiting go-ahead to apply to the monorepo-root README.
- **Lexicon scope**: NOT settled yet. The question stands: adopt "Lexicon at the federation boundary only,
  Rust as source of truth for the private half" (contradicts the documented "lexicons as the canonical model
  for all entities"), or keep lexicons-for-all-entities? Left open pending the owner.

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
