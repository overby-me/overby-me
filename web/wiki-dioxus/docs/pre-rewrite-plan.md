
# Pre-rewrite plan: what to do before the atproto rewrite

The interim Hasura/NHost/Postgres stack is throwaway. This list only greenlights work that lives in the kept Dioxus frontend and is wanted regardless, migrates into the axum backend that becomes the AppView, is pure domain/crypto/schema design that transfers close to 100 percent, or is data hygiene that lowers big-bang migration risk. Everything grounded below was verified against the code and docs. Two items from the shortlist were refuted by the adversarial verification and moved to the defer section, with reasons.

## Status (2026-07-16)

Done: #1 (authz-predicate consolidation into `auth::is_active_member`/`is_active_owner`; fixed the real
present-tense bug where the notify paths ignored the durable node_id binding), #5 (`statecookie.rs` hardened to
HKDF-SHA256 + XChaCha20-Poly1305, with a legacy-decrypt fallback so at-rest sessions survive), #6 (blind-sig
crate spike: `blind-rsa-signatures` / RFC 9474 confirmed pure-Rust and fit for purpose), #7 (public wiki.radikal.* lexicons drafted under lexicons/), #10 (SurrealQL domain-model doc
converted to SQL on Turso), #3 (DID-reachability audit run). Remaining: #2 (graphql.rs seam), #4 (DID link
nudge), #8 (orphan purge), #9 (fonts + PWA).

### #3 audit result (2026-07-16, live read-only)

- 0 atproto DIDs linked system-wide: the member-to-DID map is EMPTY. The link flow has never been used, so the
  migration join resolves to nothing today. This makes #4 (the link nudge) the only way to start filling it.
- 1728 contexts, 20516 members, but only 3508 (17%) have a `node_id` (an account); the other 83% are
  roster-only email invites who never logged in.
- 20515 active members, 3507 active + bound; 76 active owners, 0 DID-linked; 0/1728 contexts have any
  DID-linked member.
- Migration implication: the big-bang cutover cannot lean on existing DID bindings (there are none), and most
  "members" are email rows, not accounts, so they migrate as pending invites, not as users. Re-run this near
  cutover to watch the DID number climb as the nudge takes effect.

## Do now (prioritized)

### 1. Consolidate the membership/ownership authz predicates into one backend module
- **Category:** axum-evolves
- **Artifact:** `backend/src/vote.rs:134`, `backend/src/notify.rs:242`, `backend/src/notify.rs:313`, `backend/src/members.rs:144`
- **Why:** These four sites re-implement the SAME logical predicate with FOUR different WHERE clauses, and they already disagree today. `vote.rs` authorizes an active member by `nodeId OR email`; `notify.rs:242` authorizes the same "active member of context" check by `email` only; `notify.rs:313` adds `owner`; `members.rs:144` uses `nodeId` plus `owner`. That is a present-tense authz inconsistency (the durable `node_id` binding is honored on the vote path but not the notify path), worth fixing regardless of any rewrite, and it lives in the backend that becomes the AppView. The predicate as a domain contract also transfers: `docs/atproto-domain-model.md` names these exact two predicates (active-member-of-context, active-owner-of-context) as the AppView authz core, so consolidating forces the team to answer now what "active" means, whether owner is a role or a flag, and whether the node-owner fallback in `members.rs` counts as ownership.
- **Scope trim (from verification):** Consolidate into small Rust predicate FUNCTIONS with typed signatures (`is_active_member(ctx, principal)`, `is_active_owner(ctx, principal)`) and a single membership-role enum, NOT a shared GraphQL string. Model the principal as an abstraction (uuid plus email today) so the later DID swap is one internal change, and do not enshrine email-matching as a first-class part of the interface. Do NOT claim "tests transfer verbatim" as the payoff (tests that stub Hasura JSON test the throwaway layer). The payoff is drift-kill now plus contract-decision now. Keep it in `backend/` only; do not pull node/member/relation CRUD authz out of Hasura RLS as part of this.
- **Cost:** S to M

### 2. Map cynic types to frontend-owned serde structs at the graphql.rs boundary
- **Category:** frontend-kept
- **Artifact:** `src/graphql.rs` return boundary; leak confirmed across 25+ component files (`src/components/speak.rs`, `loader.rs`, `member.rs`, `layout/home_list.rs`, `profile.rs`, `folder.rs`, `node.rs`, `screen.rs`, `editor.rs`, and more), plus the scalar wrappers `Uuid`/`Jsonb`/`Timestamptz` and the write-side input types `NodesSetInput`/`NodesInsertInput`
- **Why:** `docs/atproto-port.md` section 9 step 4 explicitly greenlights this exact work, conditioned on the Dioxus frontend surviving (decided): have components read domain types through a repository/service boundary rather than naming `graphql.rs`/cynic types directly, so the backend swap is contained to that layer, not every component. The leak is real and cynic-macro-coupled (`ContextNodeFields`, `NodeWithChildren`, `MemberFields`, and the scalars all carry `cynic` derives bound to `graphql/schema.graphql`). Precedent already exists (`Author`, `BallotRules`, `Crumb` are already plain serde structs).
- **Scope trim (from verification):** The value that transfers is the ANTI-CORRUPTION SEAM (components depend on frontend-owned serde types), NOT the field shapes. Do not claim "these structs ARE the shared-model crate's frontend view"; the shapes get re-derived from lexicons and atrium codegen at cutover (the domain model splits today's flat `nodes` plus `mimeId` taxonomy into distinct `context`/`document`/`post` kinds, turns `Uuid` foreign keys into typed record links, and derives `is_owner` from a member relation edge). Widen scope to also shield the write-side input types (`NodesSetInput`, `NodesInsertInput`) so the seam actually contains the swap; otherwise those components still break at cutover. Keep it a thin edge mapping, not a trait.
- **Cost:** M

### 3. Read-only DID-reachability audit per context
- **Category:** data-hygiene
- **Artifact:** throwaway SQL against `members.nodeId` -> `users.id` -> `user_providers.provider_id` (DID); binding confirmed in `backend/src/nhost.rs` (`user_providers` keyed on `(provider, provider_id=DID)`)
- **Why:** This IS the big-bang migration's join, run as a dry-run: for each context, how many members have a `node_id`, of those how many resolve to a DID, and how many active or owner members land with NO DID. `docs/atproto-port.md` sections 3.1 and 9.3 flag member-to-DID as the riskiest migration and recommend filling the binding map incrementally during the interim rather than a mass re-link at cutover. That only works if you can see the gap now, while these users still log into the interim app. The SQL is throwaway but the number is keeper knowledge; it turns a silent cutover risk into a number to drive to zero.
- **Scope trim (from verification):** Frame it as a cheap re-runnable read-only measurement (baseline now, re-run near cutover), NOT a one-time report, and explicitly NOT a mandate to build any DID-linking-drive infrastructure.
- **Cost:** S

### 4. Surface and nudge atproto-OAuth DID linking for active and owner members
- **Category:** frontend-kept (plus migration data hygiene)
- **Artifact:** existing link flow in `backend/src/oauth.rs` (PKCE, DPoP, DID confirm, upsert, status endpoint) and `backend/src/nhost.rs` `upsert_atproto_link`; the profile card already ships in `src/components/profile.rs`
- **Why:** Every member linked now is a member pre-migrated. The captured asset is not the throwaway `user_providers` table, it is the member-to-DID map, which the rewrite's own binding flow consumes verbatim; `atproto-open-decisions.md` decided identity is PDS-agnostic with no re-issuance step, so an interim-linked DID is the same DID that logs in post-cutover. The value is time-accrued: each member links once, when they log in, which cannot be recovered after cutover. Pairs with the audit in item 3.
- **Scope trim (from verification):** The profile-surface half is ALREADY shipped in `src/components/profile.rs`; do not rebuild it. The only genuinely-new work is (a) a "not yet linked" indicator in the roster/member view (`src/components/member.rs` has zero link surfacing today) reusing the existing status endpoint, and (b) the owner/active-member nudge. Both are thin and frontend-only; the one throwaway piece is the status data call, a trivial one-line swap to XRPC at cutover.
- **Cost:** S

### 5. Harden statecookie.rs to HKDF-SHA256 plus XChaCha20-Poly1305
- **Category:** crypto-schema
- **Artifact:** `backend/src/statecookie.rs` (`cipher()` currently does `Sha256::digest(secret)` as the KDF at line 34; `seal()` uses a 12-byte random nonce); `hkdf = "0.12"` and `chacha20poly1305 = "0.10"` are already deps in `backend/Cargo.toml`
- **Why:** `atproto-stack-decisions.md` (Crypto for the private half) names this precise upgrade: replace the bare `SHA-256(secret)` derivation with HKDF-SHA256 using per-purpose `info` labels for domain separation, and move to XChaCha20-Poly1305 (24-byte nonce) to remove the 96-bit random-nonce birthday ceiling. This is a rare do-now on the crypto side: the `seal`/`open` code and the at-rest `atproto_session` blob are kept verbatim and live in the redb core, so this survives cutover unchanged.
- **Cost:** S

### 6. Spike-only: prove pure-Rust blind-signature crate availability
- **Category:** crypto-schema
- **Artifact:** a throwaway scratch crate; grounded in `atproto-stack-decisions.md:210-211`, which defers "the blind-signature primitive (RSA blind signatures per RFC 9474, or blind BLS/Schnorr), preferring a maintained pure-Rust implementation" to implementation time
- **Why:** This is the single biggest load-bearing unknown. The whole decided ballot architecture (blind-signature eligibility tokens plus a public bulletin board) rests on a primitive whose pure-Rust availability is explicitly unverified. Pure-Rust RFC 9474 support is genuinely thin. Prove in a scratch crate that you can issue an RFC 9474 RSA blind signature and/or verify blind BLS/Schnorr with a MAINTAINED pure-Rust crate; record crate, license, and maintenance status. If none exists, the ballot architecture needs rethinking BEFORE the rewrite starts, not during it.
- **Cost:** S

### 7. Draft the PUBLIC-subset wiki.radikal.* lexicon JSON
- **Category:** crypto-schema (domain/schema design)
- **Artifact:** new lexicon files (none exist yet; confirmed empty search); fields already sketched in `atproto-domain-model.md`
- **Why:** `atproto-port.md` section 9 step 1 calls the data model the one artifact that transfers 100 percent, and `atproto-stack-decisions.md` (Lexicon-to-atrium codegen) confirms lexicons are canonical at the federation boundary and feed `atrium-codegen` unchanged. Drafting the public records (post, statement, resolution, public group/event/document, comment) forces the public/private and always-private split (ballots, roster, affiliation) to be pinned on paper while it is cheap.
- **Scope trim:** Draft ONLY the public subset. The overall lexicon SCOPE is still explicitly marked OPEN in `atproto-open-decisions.md` (lexicons-at-the-boundary vs lexicons-for-all-entities is unsettled). The private half gets hand-authored Rust types, not lexicons, per the stack decision, so do not author lexicons for ballot, dedup, affiliation, or projector state.
- **Cost:** S to M

### 8. Add a purge/reparent action to the Missing-Parent orphan admin
- **Category:** data-hygiene
- **Artifact:** `src/components/parent.rs` (the `?app=parent` "Missing parent" view, #149, currently only LISTS null-parent orphans via `query_orphans` plus an `is_orphan` filter; no action)
- **Why:** Orphans are junk regardless: each breaks the redb `parent_id` tree walk and has no context to materialize into, so removing them now deletes an importer edge case for the migration. The detection view exists; only the action is missing. Wanted anyway.
- **Cost:** S

### 9. Self-host fonts and finish PWA offline caching
- **Category:** frontend-kept
- **Artifact:** two render-blocking `@import url("https://fonts.googleapis.com/...")` in `assets/style.css:16` (Atkinson Hyperlegible) and `:21` (Material Icons); PWA `sw.js`/manifest already in `src/pwa.rs`
- **Why:** Fully frontend-shell work that outlives the backend swap and is wanted for a civic tool used on flaky wifi. Removing the Google-hosted font imports also cuts a third-party dependency and two render-blocking round trips.
- **Scope trim:** Keep it small. The app font is Atkinson Hyperlegible (not arbitrary "two Google Fonts"), so self-hosting needs the unsubsetted files plus a browser check for Danish glyph coverage before shipping.
- **Cost:** S

### 10. Fix the stale SurrealQL domain-model doc
- **Category:** data-hygiene (doc hygiene)
- **Artifact:** `docs/atproto-domain-model.md` (lines 9, 65, 209, 259, 312, 319, 321, 347, 354 still describe SurrealDB/SurrealQL and LIVE queries)
- **Why:** The datastore decision is now redb (`atproto-open-decisions.md`, `atproto-stack-decisions.md`), and realtime is one multiplexed axum WebSocket fed by an in-process broadcast channel, not SurrealDB LIVE queries. The domain-model doc still expresses ballot tally and owner authz as SurrealQL `SELECT ... role = 'owner'`, contradicting the decided engine. This is pure hygiene: rewrite the "DB realisation" section against redb keyed tables and a Rust graph walk so the canonical domain doc stops pointing at a rejected engine. This is a doc fix, not a schema prototype.
- **Cost:** S

## Explicitly defer to the rewrite

- **Wrapping the 11 inline Hasura subscription strings in named typed fns (REFUTED, do not do now).** The verification overturned this. The endgame is ONE multiplexed axum WebSocket fed by a broadcast channel with nested, declarative, server-restricted subscription trees (`atproto-stack-decisions.md`, Realtime and the firehose consumer), not 8 flat `live_votes(poll_id)` topic fns. `atproto-port.md` lists the per-query WebSocket subscriptions under throwaway and calls today's `use_live`-per-query model a degenerate hand-rolled sync engine. Only about 8 function names would transfer (cheap to re-derive); the load-bearing Hasura string bodies are throwaway, and this de-risks nothing hard (the restrictor and sync engine are untouched). Do it as part of the frontend data-layer migration, when the nested subscription shape is known, so the boundary matches the endgame API.
- **A poll open/close backend endpoint (REFUTED, do not do now).** The verification overturned this too. Verified: `src/graphql.rs` `update_node` (line 2064) executes with the caller's `access_token`, so poll close already runs under RLS as the authenticated user. The client-side `is_context_owner` flag in `poll.rs` is only UI gating and cannot be bypassed to mutate. This is fundamentally unlike `/vote/cast`, which HAD to move server-side because the anonymous-ballot path uses the admin secret and bypasses RLS entirely. So there is no live trust gap. A new `/poll/close` endpoint would be admin-secret GraphQL strings plus an `active`-relation upsert plus the `mutable` flag, every piece replaced by redb keyed-table lookups plus a write-path delta broadcast in the AppView, and it violates `atproto-port.md` section 9 point 5 (no new Hasura-coupled backend features to unwind). What DOES transfer is pure lexicon/domain design: specify the vote/poll lexicon's open/close semantics and the owner-authority rule as schema (folded into item 7). If any interim hardening is wanted, it is a one-line label fix (`consistency-audit.md:135`), not a lifecycle endpoint.
- **Executable-spec of tally/verify/dedup/delegation-to-weight math with property tests.** Roughly 100 percent transfer and catches catastrophic anonymity/audit bugs, but only worth doing once the crate spike (item 6) confirms a viable primitive; otherwise the stubbed blind-signature trait shape may be wrong. Sequence it immediately after the spike.
- **Standalone read-only migration extractor (Postgres to future serde shapes).** Strong transfer and surfaces the field-gap list, but L cost, and the target types depend on the redb schema not yet built. Do the cheaper DID-reachability audit first; graduate to a full extractor once the serde domain types settle.
- **Free-text-author to DID mapping decision.** A real identity hole, but a measure-and-decide against real data, best folded into the extractor once that exists. Sequence after the audit.
- **Canonical backend-side Rust serde domain types in a shared crate.** Keeper artifact, but binding them to interim Hasura JSON now is throwaway glue. Do the frontend-facing structs first (item 2); do the backend-canonical set with the extractor.
- **Full data-trait seam over all ~204 graphql/nhost call sites.** Highest mechanical transfer but L cost and risks guessing XRPC shapes; the two targeted seams above (cynic-type mapping and the eventual subscription reshape) capture most of the value at a fraction of the cost. Finish the incremental seam later, not as a big-bang refactor now.
- **Interim-stack throwaway (forbidden by guidance).** NHost HS256/Hasura admin-secret to atproto DID auth swap now (about nothing transfers, destabilizes working auth, runs two identity systems in parallel; `atrium-oauth` is a documented drop-in); moving the full node/member/relation CRUD authz out of RLS into interim axum endpoints (tied to Hasura's permission matrix; the transferable predicates are already captured by item 1); wrapping the interactive mutations in a mutation-trait boundary (the most Hasura-shaped part, replaced wholesale); reshaping the Hasura schema or the `node.data` JSONB into lexicon-shaped columns (Slate JSON carries over as-is; lexicon shaping happens in Rust at the publish seam); node key/slug uniqueness normalization (redb assigns fresh rkeys/slugs at import); and polishing the interim secret-ballot anonymity crypto (fully replaced by blind-signature tokens plus the public board).
- **UI that depends on rewrite-only data or unspecified crypto.** Public/private visibility toggle UI (the interim stack has no visibility column and no publish path, so the toggle would lie); eligibility/delegation UI and ballot audit/verify UI (every screen depends on tokens, weights, and the bulletin board that the crypto scheme defines and the interim model lacks; only a paper design of the verify UX is honest now); rebuilding the execCommand rich-text editor (isolated behind the `richtext::exec` seam, works in all current browsers, and likely reworked when the rewrite reworks content). Optimistic writes are already shipped (`reconcile_by_key`, casting busy guard, 17 components); do not redo them.

## Sequencing

Start with the two things that unblock or invalidate everything else: the blind-signature crate spike (item 6), because a negative result forces a ballot-architecture rethink before any rewrite work, and the DID-reachability audit (item 3), because its number sets how hard to push DID linking (item 4) during the interim. In parallel, land the cheap high-transfer keepers that need no prerequisite: the authz-predicate consolidation (item 1, which also fixes a live drift bug), the cynic-to-serde seam (item 2), the statecookie hardening (item 5), the orphan purge action (item 8), the font/PWA polish (item 9), and the two doc/schema artifacts (items 7 and 10). Only after the crate spike confirms a viable primitive and the serde domain types begin to settle should you graduate to the deferred tally executable-spec and the full migration extractor; sequencing those before their prerequisites risks freezing the wrong trait shape or the wrong target types.

