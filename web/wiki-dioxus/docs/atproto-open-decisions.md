# atproto Rewrite: Decision Log

A running log of the product/architecture decisions the atproto rewrite forces. Decided items record the
owner's call; open items list the options and a lean, to be settled one at a time. Stack-level technical
decisions live in `stack-futureproofing-eval.md` and `atproto-stack-decisions.md`; this file tracks the
choices that need a human call.

## Decided (2026-07-16)

- **Datastore**: split. redb (pure Rust, 2PC durability) for the ballot/tally/roster core; PostgreSQL + sqlx
  for the rebuildable firehose view; Turso/Limbo tracked as the view's Rust replacement. (Honors the
  Rust-native preference where correctness allows; concedes C only on the recomputable half.)
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
- **Migration**: big-bang cutover, accepted as low-risk because the old wiki will be stable by then.

## Open (need a call)

- **E2E-V voting scheme** (new, spawned by "full cryptographic"). Options:
  - Blind-signature tokens + public board: org blind-signs one weighted eligibility token per voter; the
    voter casts anonymously with the token to an append-only public board (which can be atproto records for
    a free public audit trail); double-vote blocked by token uniqueness. Simplest; gives eligibility +
    anonymity + audit.
  - Homomorphic tally (Helios-style exponential ElGamal + threshold decryption): strongest
    privacy-preserving universal verifiability; heavier crypto + trustee/threshold setup.
  - Verifiable mixnet: strong anonymity, most complex to implement/audit.
  - Hard constraint to resolve either way: delegation (liquid democracy) is in fundamental tension with
    unlinkability (a resolved delegation chain can re-identify a voter), so the scheme must fix how much
    delegation is exposed and whether delegated weight is publicly visible.
  - Lean: blind-signature tokens + an atproto-records public board, with delegation resolved to weights
    server-side before issuance (delegation visible to the org, not on the public board). Confirm.
- **Per-item visibility policy**: which entities are public-by-default (posts, opt-in public groups/events,
  resolutions) vs always-private (ballots, roster, and membership-as-affiliation). Lean: membership private
  by default; groups/events opt-in public. Confirm the list.
- **README P-256 atproto-exception row**: apply the edit to the monorepo-root README (it currently blesses
  atproto but lists Ed25519, which atproto forbids for repo keys)? Yes/no.
- **Lexicon scope**: adopt "Lexicon at the federation boundary only, Rust as source of truth for the private
  half" (this contradicts the documented "lexicons as the canonical model for all entities")? Yes/no.

## Engineering defaults taken (override if you disagree)

- TLS terminated at the Caddy edge; stay on the `ring` rustls provider until PQ-hybrid KEX or FIPS forces
  aws-lc-rs. HTTP/2 + TCP primary; HTTP/3 optional edge only.
- Realtime via one multiplexed axum WebSocket fed by an in-process broadcast channel (the AppView is one
  persistent process holding redb + the Postgres view + the firehose).
- API surface: XRPC + `wiki.radikal.*` lexicons at the public boundary; a typed internal Rust API for the
  Dioxus client. GraphQL/cynic dropped.
- `reqwest` pinned to atrium's resolved version (0.12 today) so the connection pool is shared.
- Files/blobs: PDS blobs for public files, an org object store for private (replacing NHost storage).
