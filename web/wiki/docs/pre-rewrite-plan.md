
# Pre-rewrite plan: what to do before the atproto rewrite

The interim Hasura/NHost/Postgres stack is throwaway. This list only greenlights work that lives in the kept Dioxus frontend and is wanted regardless, migrates into the axum backend that becomes the AppView, is pure domain/crypto/schema design that transfers close to 100 percent, or is data hygiene that lowers big-bang migration risk. Everything grounded below was verified against the code and docs. Two items from the shortlist were refuted by the adversarial verification and moved to the defer section, with reasons.

## Status (2026-07-16)

Done: #1 (authz-predicate consolidation into `auth::is_active_member`/`is_active_owner`; fixed the real
present-tense bug where the notify paths ignored the durable node_id binding), #5 (`statecookie.rs` hardened to
HKDF-SHA256 + XChaCha20-Poly1305, with a legacy-decrypt fallback so at-rest sessions survive), #6 (blind-sig
crate spike: `blind-rsa-signatures` / RFC 9474 confirmed pure-Rust and fit for purpose), #7 (public wiki.radikal.* lexicons drafted under lexicons/), #10 (SurrealQL domain-model doc
converted to SQL on Turso), #3 (DID-reachability audit run), #4 (atproto-link nudge in the member roster), and
also #8 (orphan purge action in the Missing-Parent admin), #9 (fonts self-hosted via `asset!()` + `@font-face`,
Google Fonts CDN dropped, service-worker cache bumped so they cache offline), and #2 (the graphql.rs
anti-corruption seam: a new cynic-free `src/model.rs` owns every domain type components touch; `graphql.rs`
keeps the cynic wire types internal and converts at each query/mutation boundary, so the backend swap is
contained to that mapping layer). All 10 do-now items are complete. A second deep analysis produced the
Round 2 plan below (22 verified items: owner calls, ballot-math spec crate, risk spikes, extractor).

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

## Round 1 (2026-07, complete)

### Do now (prioritized)

#### 1. Consolidate the membership/ownership authz predicates into one backend module

- **Category:** axum-evolves
- **Artifact:** `backend/src/vote.rs:134`, `backend/src/notify.rs:242`, `backend/src/notify.rs:313`, `backend/src/members.rs:144`
- **Why:** These four sites re-implement the SAME logical predicate with FOUR different WHERE clauses, and they already disagree today. `vote.rs` authorizes an active member by `nodeId OR email`; `notify.rs:242` authorizes the same "active member of context" check by `email` only; `notify.rs:313` adds `owner`; `members.rs:144` uses `nodeId` plus `owner`. That is a present-tense authz inconsistency (the durable `node_id` binding is honored on the vote path but not the notify path), worth fixing regardless of any rewrite, and it lives in the backend that becomes the AppView. The predicate as a domain contract also transfers: `docs/atproto-domain-model.md` names these exact two predicates (active-member-of-context, active-owner-of-context) as the AppView authz core, so consolidating forces the team to answer now what "active" means, whether owner is a role or a flag, and whether the node-owner fallback in `members.rs` counts as ownership.
- **Scope trim (from verification):** Consolidate into small Rust predicate FUNCTIONS with typed signatures (`is_active_member(ctx, principal)`, `is_active_owner(ctx, principal)`) and a single membership-role enum, NOT a shared GraphQL string. Model the principal as an abstraction (uuid plus email today) so the later DID swap is one internal change, and do not enshrine email-matching as a first-class part of the interface. Do NOT claim "tests transfer verbatim" as the payoff (tests that stub Hasura JSON test the throwaway layer). The payoff is drift-kill now plus contract-decision now. Keep it in `backend/` only; do not pull node/member/relation CRUD authz out of Hasura RLS as part of this.
- **Cost:** S to M

#### 2. Map cynic types to frontend-owned serde structs at the graphql.rs boundary

- **Category:** frontend-kept
- **Artifact:** `src/graphql.rs` return boundary; leak confirmed across 25+ component files (`src/components/speak.rs`, `loader.rs`, `member.rs`, `layout/home_list.rs`, `profile.rs`, `folder.rs`, `node.rs`, `screen.rs`, `editor.rs`, and more), plus the scalar wrappers `Uuid`/`Jsonb`/`Timestamptz` and the write-side input types `NodesSetInput`/`NodesInsertInput`
- **Why:** `docs/atproto-port.md` section 9 step 4 explicitly greenlights this exact work, conditioned on the Dioxus frontend surviving (decided): have components read domain types through a repository/service boundary rather than naming `graphql.rs`/cynic types directly, so the backend swap is contained to that layer, not every component. The leak is real and cynic-macro-coupled (`ContextNodeFields`, `NodeWithChildren`, `MemberFields`, and the scalars all carry `cynic` derives bound to `graphql/schema.graphql`). Precedent already exists (`Author`, `BallotRules`, `Crumb` are already plain serde structs).
- **Scope trim (from verification):** The value that transfers is the ANTI-CORRUPTION SEAM (components depend on frontend-owned serde types), NOT the field shapes. Do not claim "these structs ARE the shared-model crate's frontend view"; the shapes get re-derived from lexicons and atrium codegen at cutover (the domain model splits today's flat `nodes` plus `mimeId` taxonomy into distinct `context`/`document`/`post` kinds, turns `Uuid` foreign keys into typed record links, and derives `is_owner` from a member relation edge). Widen scope to also shield the write-side input types (`NodesSetInput`, `NodesInsertInput`) so the seam actually contains the swap; otherwise those components still break at cutover. Keep it a thin edge mapping, not a trait.
- **Cost:** M

#### 3. Read-only DID-reachability audit per context

- **Category:** data-hygiene
- **Artifact:** throwaway SQL against `members.nodeId` -> `users.id` -> `user_providers.provider_id` (DID); binding confirmed in `backend/src/nhost.rs` (`user_providers` keyed on `(provider, provider_id=DID)`)
- **Why:** This IS the big-bang migration's join, run as a dry-run: for each context, how many members have a `node_id`, of those how many resolve to a DID, and how many active or owner members land with NO DID. `docs/atproto-port.md` sections 3.1 and 9.3 flag member-to-DID as the riskiest migration and recommend filling the binding map incrementally during the interim rather than a mass re-link at cutover. That only works if you can see the gap now, while these users still log into the interim app. The SQL is throwaway but the number is keeper knowledge; it turns a silent cutover risk into a number to drive to zero.
- **Scope trim (from verification):** Frame it as a cheap re-runnable read-only measurement (baseline now, re-run near cutover), NOT a one-time report, and explicitly NOT a mandate to build any DID-linking-drive infrastructure.
- **Cost:** S

#### 4. Surface and nudge atproto-OAuth DID linking for active and owner members

- **Category:** frontend-kept (plus migration data hygiene)
- **Artifact:** existing link flow in `backend/src/oauth.rs` (PKCE, DPoP, DID confirm, upsert, status endpoint) and `backend/src/nhost.rs` `upsert_atproto_link`; the profile card already ships in `src/components/profile.rs`
- **Why:** Every member linked now is a member pre-migrated. The captured asset is not the throwaway `user_providers` table, it is the member-to-DID map, which the rewrite's own binding flow consumes verbatim; `atproto-open-decisions.md` decided identity is PDS-agnostic with no re-issuance step, so an interim-linked DID is the same DID that logs in post-cutover. The value is time-accrued: each member links once, when they log in, which cannot be recovered after cutover. Pairs with the audit in item 3.
- **Scope trim (from verification):** The profile-surface half is ALREADY shipped in `src/components/profile.rs`; do not rebuild it. The only genuinely-new work is (a) a "not yet linked" indicator in the roster/member view (`src/components/member.rs` has zero link surfacing today) reusing the existing status endpoint, and (b) the owner/active-member nudge. Both are thin and frontend-only; the one throwaway piece is the status data call, a trivial one-line swap to XRPC at cutover.
- **Cost:** S

#### 5. Harden statecookie.rs to HKDF-SHA256 plus XChaCha20-Poly1305

- **Category:** crypto-schema
- **Artifact:** `backend/src/statecookie.rs` (`cipher()` currently does `Sha256::digest(secret)` as the KDF at line 34; `seal()` uses a 12-byte random nonce); `hkdf = "0.12"` and `chacha20poly1305 = "0.10"` are already deps in `backend/Cargo.toml`
- **Why:** `atproto-stack-decisions.md` (Crypto for the private half) names this precise upgrade: replace the bare `SHA-256(secret)` derivation with HKDF-SHA256 using per-purpose `info` labels for domain separation, and move to XChaCha20-Poly1305 (24-byte nonce) to remove the 96-bit random-nonce birthday ceiling. This is a rare do-now on the crypto side: the `seal`/`open` code and the at-rest `atproto_session` blob are kept verbatim and live in the Turso ballot core, so this survives cutover unchanged.
- **Cost:** S

#### 6. Spike-only: prove pure-Rust blind-signature crate availability

- **Category:** crypto-schema
- **Artifact:** a throwaway scratch crate; grounded in `atproto-stack-decisions.md:210-211`, which defers "the blind-signature primitive (RSA blind signatures per RFC 9474, or blind BLS/Schnorr), preferring a maintained pure-Rust implementation" to implementation time
- **Why:** This is the single biggest load-bearing unknown. The whole decided ballot architecture (blind-signature eligibility tokens plus a public bulletin board) rests on a primitive whose pure-Rust availability is explicitly unverified. Pure-Rust RFC 9474 support is genuinely thin. Prove in a scratch crate that you can issue an RFC 9474 RSA blind signature and/or verify blind BLS/Schnorr with a MAINTAINED pure-Rust crate; record crate, license, and maintenance status. If none exists, the ballot architecture needs rethinking BEFORE the rewrite starts, not during it.
- **Cost:** S

#### 7. Draft the PUBLIC-subset wiki.radikal.* lexicon JSON

- **Category:** crypto-schema (domain/schema design)
- **Artifact:** new lexicon files (none exist yet; confirmed empty search); fields already sketched in `atproto-domain-model.md`
- **Why:** `atproto-port.md` section 9 step 1 calls the data model the one artifact that transfers 100 percent, and `atproto-stack-decisions.md` (Lexicon-to-atrium codegen) confirms lexicons are canonical at the federation boundary and feed `atrium-codegen` unchanged. Drafting the public records (post, statement, resolution, public group/event/document, comment) forces the public/private and always-private split (ballots, roster, affiliation) to be pinned on paper while it is cheap.
- **Scope trim:** Draft ONLY the public subset. The overall lexicon SCOPE is still explicitly marked OPEN in `atproto-open-decisions.md` (lexicons-at-the-boundary vs lexicons-for-all-entities is unsettled). The private half gets hand-authored Rust types, not lexicons, per the stack decision, so do not author lexicons for ballot, dedup, affiliation, or projector state.
- **Cost:** S to M

#### 8. Add a purge/reparent action to the Missing-Parent orphan admin

- **Category:** data-hygiene
- **Artifact:** `src/components/parent.rs` (the `?app=parent` "Missing parent" view, #149, currently only LISTS null-parent orphans via `query_orphans` plus an `is_orphan` filter; no action)
- **Why:** Orphans are junk regardless: each breaks the `parent_id` tree walk and has no context to materialize into, so removing them now deletes an importer edge case for the migration. The detection view exists; only the action is missing. Wanted anyway.
- **Cost:** S

#### 9. Self-host fonts and finish PWA offline caching

- **Category:** frontend-kept
- **Artifact:** two render-blocking `@import url("https://fonts.googleapis.com/...")` in `assets/style.css:16` (Atkinson Hyperlegible) and `:21` (Material Icons); PWA `sw.js`/manifest already in `src/pwa.rs`
- **Why:** Fully frontend-shell work that outlives the backend swap and is wanted for a civic tool used on flaky wifi. Removing the Google-hosted font imports also cuts a third-party dependency and two render-blocking round trips.
- **Scope trim:** Keep it small. The app font is Atkinson Hyperlegible (not arbitrary "two Google Fonts"), so self-hosting needs the unsubsetted files plus a browser check for Danish glyph coverage before shipping.
- **Cost:** S

#### 10. Fix the stale SurrealQL domain-model doc

- **Category:** data-hygiene (doc hygiene)
- **Artifact:** `docs/atproto-domain-model.md` (lines 9, 65, 209, 259, 312, 319, 321, 347, 354 still describe SurrealDB/SurrealQL and LIVE queries)
- **Why:** The datastore decision is now Turso (`atproto-open-decisions.md`, `atproto-stack-decisions.md`), and realtime is one multiplexed axum WebSocket fed by an in-process broadcast channel, not SurrealDB LIVE queries. The domain-model doc still expresses ballot tally and owner authz as SurrealQL `SELECT ... role = 'owner'`, contradicting the decided engine. This is pure hygiene: rewrite the "DB realisation" section against Turso tables and a recursive query so the canonical domain doc stops pointing at a rejected engine. This is a doc fix, not a schema prototype.
- **Cost:** S

### Explicitly defer to the rewrite

- **Wrapping the 11 inline Hasura subscription strings in named typed fns (REFUTED, do not do now).** The verification overturned this. The endgame is ONE multiplexed axum WebSocket fed by a broadcast channel with nested, declarative, server-restricted subscription trees (`atproto-stack-decisions.md`, Realtime and the firehose consumer), not 8 flat `live_votes(poll_id)` topic fns. `atproto-port.md` lists the per-query WebSocket subscriptions under throwaway and calls today's `use_live`-per-query model a degenerate hand-rolled sync engine. Only about 8 function names would transfer (cheap to re-derive); the load-bearing Hasura string bodies are throwaway, and this de-risks nothing hard (the restrictor and sync engine are untouched). Do it as part of the frontend data-layer migration, when the nested subscription shape is known, so the boundary matches the endgame API.
- **A poll open/close backend endpoint (REFUTED, do not do now).** The verification overturned this too. Verified: `src/graphql.rs` `update_node` (line 2064) executes with the caller's `access_token`, so poll close already runs under RLS as the authenticated user. The client-side `is_context_owner` flag in `poll.rs` is only UI gating and cannot be bypassed to mutate. This is fundamentally unlike `/vote/cast`, which HAD to move server-side because the anonymous-ballot path uses the admin secret and bypasses RLS entirely. So there is no live trust gap. A new `/poll/close` endpoint would be admin-secret GraphQL strings plus an `active`-relation upsert plus the `mutable` flag, every piece replaced by Turso table lookups plus a write-path delta broadcast in the AppView, and it violates `atproto-port.md` section 9 point 5 (no new Hasura-coupled backend features to unwind). What DOES transfer is pure lexicon/domain design: specify the vote/poll lexicon's open/close semantics and the owner-authority rule as schema (folded into item 7). If any interim hardening is wanted, it is a one-line label fix (`consistency-audit.md:135`), not a lifecycle endpoint.
- **Executable-spec of tally/verify/dedup/delegation-to-weight math with property tests.** Roughly 100 percent transfer and catches catastrophic anonymity/audit bugs, but only worth doing once the crate spike (item 6) confirms a viable primitive; otherwise the stubbed blind-signature trait shape may be wrong. Sequence it immediately after the spike.
- **Standalone read-only migration extractor (Postgres to future serde shapes).** Strong transfer and surfaces the field-gap list, but L cost, and the target types depend on the Turso schema not yet built. Do the cheaper DID-reachability audit first; graduate to a full extractor once the serde domain types settle.
- **Free-text-author to DID mapping decision.** A real identity hole, but a measure-and-decide against real data, best folded into the extractor once that exists. Sequence after the audit.
- **Canonical backend-side Rust serde domain types in a shared crate.** Keeper artifact, but binding them to interim Hasura JSON now is throwaway glue. Do the frontend-facing structs first (item 2); do the backend-canonical set with the extractor.
- **Full data-trait seam over all ~204 graphql/nhost call sites.** Highest mechanical transfer but L cost and risks guessing XRPC shapes; the two targeted seams above (cynic-type mapping and the eventual subscription reshape) capture most of the value at a fraction of the cost. Finish the incremental seam later, not as a big-bang refactor now.
- **Interim-stack throwaway (forbidden by guidance).** NHost HS256/Hasura admin-secret to atproto DID auth swap now (about nothing transfers, destabilizes working auth, runs two identity systems in parallel; `atrium-oauth` is a documented drop-in); moving the full node/member/relation CRUD authz out of RLS into interim axum endpoints (tied to Hasura's permission matrix; the transferable predicates are already captured by item 1); wrapping the interactive mutations in a mutation-trait boundary (the most Hasura-shaped part, replaced wholesale); reshaping the Hasura schema or the `node.data` JSONB into lexicon-shaped columns (Slate JSON carries over as-is; lexicon shaping happens in Rust at the publish seam); node key/slug uniqueness normalization (the importer assigns fresh rkeys/slugs at import); and polishing the interim secret-ballot anonymity crypto (fully replaced by blind-signature tokens plus the public board).
- **UI that depends on rewrite-only data or unspecified crypto.** Public/private visibility toggle UI (the interim stack has no visibility column and no publish path, so the toggle would lie); eligibility/delegation UI and ballot audit/verify UI (every screen depends on tokens, weights, and the bulletin board that the crypto scheme defines and the interim model lacks; only a paper design of the verify UX is honest now); rebuilding the execCommand rich-text editor (isolated behind the `richtext::exec` seam, works in all current browsers, and likely reworked when the rewrite reworks content). Optimistic writes are already shipped (`reconcile_by_key`, casting busy guard, 17 components); do not redo them.

### Sequencing

Start with the two things that unblock or invalidate everything else: the blind-signature crate spike (item 6), because a negative result forces a ballot-architecture rethink before any rewrite work, and the DID-reachability audit (item 3), because its number sets how hard to push DID linking (item 4) during the interim. In parallel, land the cheap high-transfer keepers that need no prerequisite: the authz-predicate consolidation (item 1, which also fixes a live drift bug), the cynic-to-serde seam (item 2), the statecookie hardening (item 5), the orphan purge action (item 8), the font/PWA polish (item 9), and the two doc/schema artifacts (items 7 and 10). Only after the crate spike confirms a viable primitive and the serde domain types begin to settle should you graduate to the deferred tally executable-spec and the full migration extractor; sequencing those before their prerequisites risks freezing the wrong trait shape or the wrong target types.

## Round 2 (2026-07-16)

> ALL 22 round-2 items are COMPLETE (2026-07-16). Owner calls made (unit tokens, per-poll keys,
> boundary-only lexicons); NSID neutralized to the `com.example.wiki.*` placeholder. The `crates/`
> workspace holds the ballot-math spec (property-tested), the entity schema (validated on SQLite and
> Turso), the DAG-CBOR/CID vectors, the Turso kill-9 durability harness, the atrium-oauth PDS-agnostic
> spike, and the canonical domain-types crate plus the migration extractor with its field-gap report.
> The member DDL and voting SQL are reconciled; the custody memo, verify-UX paper, onboarding script,
> extended lexicons, and the live censuses are landed. Remaining owner sign-offs (not blockers, tracked
> in `atproto-open-decisions.md` Open): the ballot-spec D1 to D8 semantics, the board-custody
> recommendation, the NSID domain, and running the onboarding walkthrough with real members. The rewrite
> can now start: stand up the AppView service crate in `crates/` against the reconciled schema, move the
> transferable backend modules (push, dpop, pkce, statecookie, oauth, util) into the workspace, wire
> atrium-oauth behind the spike's wrapper, and run the extractor's importer front half against a staging
> Turso database, repointing the interim app via the item-21 env knobs for the cutover rehearsal.

Round 1 is fully landed: all 10 do-now items are done. Three facts changed the board. The blind-signature spike PASSED (`blind-rsa-signatures`, RFC 9474, pure Rust), which unlocks deferral (a), the executable ballot-math spec. The frontend anti-corruption seam exists (`src/model.rs`), and the domain-model doc now carries SQL on Turso, which together unlock the coupled deferrals (b)+(c)+(d), the canonical types and the migration extractor. The DID audit came back brutal: 0 DIDs linked system-wide, 20516 members of which 83 percent are email-only roster rows, so the migration join is empty and the target member DDL cannot even represent its dominant case. Round 2's theme follows: close the small set of paper decisions that gate everything (weight encoding, lexicon scope, NSID authority, ballot custody), turn the decided E2E-V ballot scheme into executable, property-tested artifacts in a new `crates/` workspace, retire the four remaining load-bearing unknowns by spike (DAG-CBOR/CID, atrium-oauth, Turso durability, member onboarding reality), and graduate the extractor for the stable content/membership half only. Every item below passed adversarial verification; trims from that verification are folded in as the actual scope.

Status: items 1 and 2 are DONE (owner calls made 2026-07-16: UNIT tokens, PER-POLL issuer keys, and
boundary-only lexicon scope; recorded in `atproto-open-decisions.md` under Decided, with the weighted-token
phrasing superseded in `atproto-stack-decisions.md` and the "lexicons model ALL data" section of
`atproto-domain-model.md` rewritten to boundary-only). Item 3 resolved differently than written: the owner
has NOT decided a domain, so instead of picking a root, every NSID reference (lexicon ids, docs, the old
`app.radikal.*` vs `wiki.radikal.*` split) was normalized to the reserved placeholder `com.example.wiki.*`
(RFC 2606, cannot collide, obviously unminted), the lexicons moved to `lexicons/com/example/wiki/`, the
rebrand procedure documented in `lexicons/README.md`, and the domain choice recorded as an Open decision.
This achieves item 3's irreversibility guard (nothing can be accidentally minted under a wrong name) while
deferring the actual domain call. The crypto track (items 5, 8, 9, 10) and the types track behind item 2
are now unblocked.

Item 6 census results (2026-07-16, live read-only, aggregates only):

- Author provenance: 2345 author-chip member rows on content nodes; 975 (42 percent) are FREE-TEXT
  (nodeId null), 1370 bound. 195 distinct free-text names, of which only 67 match a `users.displayName`
  (a name-join recovers about a third). Multi-author is real: 272 nodes have 2 authors, tails up to 8.
  CONSEQUENCE for item 17 and the DDL: the single nullable `author_did` column cannot represent this;
  the extractor needs an author join table (did-or-display-string per author) or an explicit
  display-string fallback next to `author_did`. Folded into item 17's mapping.
- Email identity: 17655 member rows carry an email but only 1962 distinct emails (case-insensitive):
  the same people are invited across many contexts (fan-out mode around 4 to 6 contexts, tail beyond 8).
  11 emails exist in case/whitespace variant clusters, so the importer must normalize (lowercase, trim)
  before keying. ZERO violations of the proposed `UNIQUE(context_id, email) WHERE user_did IS NULL`
  against real data: the item-4 member DDL is safe to freeze.
- Constraint/shape preflight: 0 nodes with NULL contextId; 1 node with a DANGLING contextId (junk sweep
  of one row); 3 mimeIds with no target kind, one node each (`wiki/home` is the root, `conference/
  conference` and `map/map` are legacy one-offs: extractor mapping rules or explicit drops). JSONB
  top-level key sets are finite and clean per mime (content / content+image / null-or-empty dominate);
  notable: `vote/poll.data` carries an undocumented `voters` key in 7 of 8 polls, `speak/speak.data` is
  a bare string, `vote/vote.data` a bare array. Every shape is enumerable: the importer edge-case list
  is a finite checklist, not an unknown.

### Do now (prioritized)

#### 1. Issuer-key lifecycle and weight-encoding decision entry: per-poll RSA keys plus unit tokens vs weight-carrying tokens

- **Category:** crypto-schema
- **Artifact:** `docs/atproto-stack-decisions.md:201-205` (crate pinned, silent on key scoping) and `:193-194` plus `docs/atproto-open-decisions.md:31-33` (one weighted eligibility token per voter, recorded as Decided but never deliberated)
- **Why:** Two undecided load-bearing scheme parameters that the spec crate cannot leave generic. (a) Key scoping: RFC 9474 blinding hides the message from the issuer, so the org cannot bind a token to a poll by inspecting it; without per-poll issuer keys, a token issued for poll A spends on poll B. Per-poll keys give cryptographic poll binding, natural expiry, a one-poll compromise blast radius, and require pre-open pubkey publication to the board for verifiability. (b) Weight encoding: a weight-carrying token is not even implementable without key partitioning per weight class (the fully blind signer never sees the message), and it puts each ballot's weight on the public board, shrinking the anonymity set for rare weights (a lone weight-5 delegate is uniquely identifiable); N unit tokens for weight N keep the tally a plain count at the cost of board size. This is a NEW rewrite-scheme decision, distinct from the deferred interim de-anonymization finding. The new Open entry must explicitly reference and supersede the weighted-token phrasing currently sitting under Decided at `open-decisions.md:30-34` and `stack-decisions.md:193-194`, not silently contradict it, and must note the key-partitioning interaction between the two sub-decisions.
- **Cost:** S

#### 2. Close OPEN-1 (lexicon scope) with a one-page decision memo and reconcile the three disagreeing docs

- **Category:** doc-hygiene
- **Artifact:** `docs/atproto-open-decisions.md:44-46` (marked NOT settled) vs `docs/atproto-stack-decisions.md:82-83` (boundary-only stated as a Decision), `lexicons/README.md:5-8` (boundary-only asserted), `docs/atproto-domain-model.md:44-67` (asserts lexicons model ALL data)
- **Why:** The single decision gating the two big unlocked deferrals: boundary-only means hand-authored Rust private types, all-entities means codegen from lexicons for everything, so starting the canonical-types crate or the extractor before this closes risks freezing the wrong source of truth, the exact failure `docs/pre-rewrite-plan.md:113` warns about. Closable cheaply: the shipped artifacts already implement boundary-only de facto (`lexicons/README.md:20-21` gives always-private entities NO lexicon by design). The memo must present both options neutrally for the owner call, record the losing rationale, and fix whichever docs lose. MUST close before item 17 starts.
- **Cost:** S (owner call only)

#### 3. Close OPEN-3: verify durable org control of the NSID authority domain, pick ONE root, normalize app.radikal.* vs wiki.radikal.*

- **Category:** doc-hygiene
- **Artifact:** `lexicons/README.md:25-27` (placeholder; a minted NSID is effectively permanent); the live split: `app.radikal.*` at `docs/atproto-stack-decisions.md:40`, `:117`, `docs/atproto-port.md:50`, `:218` vs `wiki.radikal.*` at `docs/atproto-stack-decisions.md:85`, `docs/atproto-domain-model.md:12`, and all three lexicon id fields
- **Why:** The NSID bakes into every lexicon id, codegen'd type path, Jetstream collection filter, and XRPC route name, and becomes permanent at first mint, making this the cheapest irreversibility guard in the plan. A registrar/DNS lookup plus one owner call (does the org durably control radikal.wiki, with DNS for the future Lexicon Resolution TXT record), then one mechanical pass replacing the losing placeholder across four docs and three JSON files. Zero code references exist yet (verified grep), so the rename is still cheap; the deferred type-path and codegen work would embed it. wiki.radikal.* is the de facto winner already; the call likely just confirms domain control. The DNS TXT registration becomes a listed org task, not code.
- **Cost:** S (owner call only)

#### 4. Fix the target member DDL so it can represent the 83 percent DID-less pending invites

- **Category:** crypto-schema
- **Artifact:** `docs/atproto-domain-model.md:269-276` (member table, PRIMARY KEY `(user_did, context_id)`) contradicted by `:354` (import creates member rows with email set and no user) and `:357` (claim-token flow with no `claim_token` column)
- **Why:** The decided identity flow imports roster rows with `user_did` NULL, but the table keys on `(user_did, context_id)`. In SQLite rowid tables NULL PK columns are each distinct, so dedup is silently unenforced for exactly the pending-invite majority (17008 of 20516 members); under a stricter dialect the import is rejected outright. Either way the schema cannot hold the dominant migration case. Fix is pure schema design: surrogate key plus partial uniques (`UNIQUE(context_id, user_did) WHERE user_did IS NOT NULL`, `UNIQUE(context_id, email) WHERE user_did IS NULL`) and the `claim_token` column the flow at `:357` already assumes. In the same edit, state whether re-inviting an email whose row has since bound a DID is meant to create a second pending row (once bound, the row leaves the email partial-unique's scope). The email census (item 6) validates the proposed unique against real collisions before freeze. Gates item 17.
- **Cost:** S

#### 5. Reconcile the domain-model voting SQL with the decided E2E-V scheme

- **Category:** crypto-schema
- **Artifact:** `docs/atproto-domain-model.md:289-316` (voted PK `(poll_id, voter_did)` at `:301-305`; ballot `cast_bucket` "same anonymity design as the interim fix" at `:307-314`; zero eligibility/weight/delegation/token tables) contradicting `docs/atproto-stack-decisions.md:188-200` and `docs/atproto-open-decisions.md:26-34`; redb/SurrealQL residue at `stack-decisions.md:191` and `domain-model.md:357`
- **Why:** The canonical model doc is the rewrite's source of truth and its voting section still specifies the ballot core the owner already replaced; both the spec crate and the canonical backend types would otherwise be derived from the wrong ballot schema, exactly the freeze-the-wrong-target-types risk at `pre-rewrite-plan.md:113`. Add: `eligibility(poll_id, did, base_weight)`, `delegation(poll_id, from_did, to_did, signed assignment)`, resolved weight frozen at open, `token_issued(poll_id, did)` issuance marker (records THAT a token was issued, never the token, preserving unlinkability), and a board-mirror table; drop the `voter_did` dedup and `cast_bucket` rows. Write the weight-encoding columns as an explicitly marked open-decision variant (both encodings sketched) rather than blocking the reconciliation on item 1. Fixes the redb and SurrealQL residue in passing.
- **Cost:** S

#### 6. Read-only pre-extractor censuses: author provenance (deferral c), email identity, target-constraint and JSONB-shape preflight

- **Category:** data-hygiene
- **Artifact:** author: `src/graphql.rs:3185-3211` (`set_node_authors` stores free-text authors as member rows with `node_id` None), `src/components/editor.rs:205`, `src/graphql.rs:2971-2975`, vs one nullable `author_did` at `docs/atproto-domain-model.md:247`. Email: `domain-model.md:350-361` (import keyed on `member.email`), `backend/src/roster.rs` (freeform .xlsx origin). Preflight: `domain-model.md:229`, `:240-252` (kind CHECK, `document.context_id` NOT NULL), `src/components/parent.rs:28-29` (purge covers only null-parent orphans), `src/model.rs:25-27` (one Jsonb column carries Slate JSON, poll options, file metadata)
- **Why:** Three sibling measurements, all PROPOSED read-only queries against live data for the team to run, feeding the extractor mapping and the DDL before either freezes. (a) Author provenance: count free-text author rows, distinct names, overlap with `users.display_name`, multi-author distribution; decides join table vs display-string fallback vs drop, possibly amending the single-`author_did` target; removes a silent-data-loss path. (b) Email identity: distinct emails vs rows, case/whitespace duplicate clusters, per-email context fan-out; decides the import unit and normalization and validates item 4's proposed `UNIQUE(context_id, email)` (a proposed constraint, not yet in the target DDL) against real collisions. (c) Constraint/shape preflight: NULL or dangling `context_id`, mimeIds with no target kind, distinct top-level JSON key sets per mimeId (legacy pre-Slate content, odd poll-option formats); each nonzero count becomes an interim-admin junk sweep or an extractor mapping rule, decided while data owners can still inspect rows in the live app. Pure measure-then-decide, converting unknown importer edge cases into a finite checklist before the big bang.
- **Cost:** M

#### 7. Rewrite-crate infrastructure: sibling `crates/` workspace, seeded non-empty, with a crates-only CI check

- **Category:** axum-evolves
- **Artifact:** `Cargo.toml:1-4` (single package, no `[workspace]`); `default.nix:58-66` (frontend cargoLock FOD-pinned, so a root-manifest merge would destabilize Nix vendoring); `backend/default.nix:20-22` (`doCheck = false` with a comment falsely claiming tests run in CI); `.tangled/workflows/flake-check.yml` builds only the formatting check
- **Why:** The rewrite needs homes for at least three crates (ballot spec, canonical types, the AppView service) and today there is no slot: both existing manifests are throwaway-coupled and Nix-pinned. A new `crates/` directory with its own `[workspace]` Cargo.toml and lockfile lets those crates accrete without touching the frontend FOD hashes or the backend container build; the transferable backend modules (push, dpop, pkce, statecookie, oauth, util) later move INTO it. Deliberately not a root-manifest merge.
- **Scope trim (from verification):** Land the workspace WITH its first real crate (the ballot-math spec, item 8), not empty. CI: one Nix checks derivation running `cargo test` for the `crates/` workspace only, wired via one entry in `.tangled/workflows.ncl` (never the YAML), gated on first verifying the build fits the documented ~2 GiB microVM (`workflows.ncl:23-37`); if it does not fit, the check runs in local `just check`/flake-check, the existing executor of record. Drop the backend-manifest cargo-test CI job entirely (12 of its 34 tests are throwaway Hasura-stack tests and the microVM cannot compile the backend dep tree); fix the false `backend/default.nix:20-22` comment by flipping `doCheck` to true instead.
- **Cost:** S with the trim (excluding item 8's own cost)

#### 8. Executable ballot-math spec crate with property tests

- **Category:** crypto-schema
- **Artifact:** deferral (a) at `docs/pre-rewrite-plan.md:103`, unlock condition met at `:10-11` (spike PASSED); scheme at `docs/atproto-stack-decisions.md:188-205`; shipped validity and tally semantics at `src/components/vote/poll.rs:358-372` and `:299-332`; verified absent: no blind-rsa, proptest, or ballot-math code anywhere
- **Why:** Pure domain/crypto transferring near 100 percent, becoming the AppView ballot core verbatim. The deferral's stated risk (freezing a wrong stubbed trait shape) is gone because RFC 9474 fixes the API surface. A standalone crate (deps only `blind-rsa-signatures`, `sha2`, `proptest`; zero interim types) with an EligibilityRoster (did to weight), pre-open delegation resolution, TokenIssuer, an append-only Board, and tally as a pure fn. Properties: issue-blind-unblind-verify round trip against the real crate; weight conservation across delegation chains and cycles (terminating deterministically); same-token double cast always collides; tally invariance under board permutation and equality with issued-weight arithmetic; completeness; non-issuer signatures rejected; and validity rules matching the shipped frontend semantics (blank-alone, min/max at `poll.rs:358-372`; blank excluded from winner, tie detection at `:299-332`) so today's real assembly rules transfer instead of being re-guessed. Guardrails from verification: keep the Board abstract (plain append-only sequence, no guessed atproto record/CID shapes); every semantic the spec pins that no doc decides (first-wins vs reject-both, chain/cycle termination) gets a decision-log entry with owner sign-off, not silent freezing in tests; the no-DID-linkage property is enforced structurally at the type level, not proptest'd. Co-designs with items 1 and 5.
- **Cost:** M

#### 9. Ballot custody decision plus drafts of the poll-announcement lexicon and the anonymized public board-entry record

- **Category:** crypto-schema
- **Artifact:** `lexicons/wiki/radikal/` holds only post, resolution, comment (verified) yet `docs/pre-rewrite-plan.md:102` claims poll open/close semantics were folded into item 7 of round 1; `docs/atproto-stack-decisions.md:193-197` (board IS atproto records) contradicts `lexicons/README.md:20-21` (ballot deliberately NO lexicon) and `docs/atproto-domain-model.md:89`
- **Why:** Two verifiable gaps: (a) the poll lexicon with open/close lifecycle and the org-authority rule, the transferable remainder of the refuted round-1 poll-endpoint item, never landed; (b) the decided scheme's public board records have no schema and no custody answer. Custody is load-bearing: every atproto record lives in a DID-owned repo, so a voter-published ballot record deanonymizes by repo ownership, while an org-published board reintroduces censorship risk needing an inclusion-receipt story; getting this wrong invalidates the anonymity property of the entire scheme. Deliverables: a custody options memo plus recommendation landed in `docs/atproto-open-decisions.md`'s Open section for the owner call; a `wiki.radikal.poll` draft (question, options, open/close, closing-authority DID, issuer pubkey); the board-entry draft (pollRef, unblinded token, signature, choices, weight field per item 1's call, explicitly no voter identity), crypto field encodings marked provisional pending item 8's message format; and a `README:20-21` correction distinguishing the always-private org-side ballot row from the public anonymized board entry. NSIDs stay placeholders, so item 3 gates only publication.
- **Cost:** M

#### 10. Paper design of the member-facing ballot verify and audit flow (paper only)

- **Category:** crypto-schema
- **Artifact:** `docs/pre-rewrite-plan.md:109` (only a paper design of the verify UX is honest now); `docs/atproto-stack-decisions.md:198-200` (individual and universal verifiability)
- **Why:** The receipt is a protocol input, not just UI: if the spec crate and board record do not reserve what the voter must keep after casting (the unblinded token message and/or record CID), individual verifiability is impossible to retrofit, so this must be written while items 8 and 9 pin the token and record shapes. Content: what the client stores locally at cast time, the find-my-ballot flow against the board, one-tap re-tally with the published issuer pubkey, failure states (a missing ballot is censorship evidence, a non-verifying signature is a forged board entry), and the non-cryptographer story for a civic assembly audience. Deliberately a document with zero UI code; it states what the receipt and record shape must reserve but does not author the lexicon itself (item 9's job). The forbidden ballot-audit UI stays untouched.
- **Cost:** S

#### 11. Land the entity-subset DDL as an executable schema.sql plus a libsql in-memory round-trip test

- **Category:** risk-spike
- **Artifact:** no .sql file and no libsql/Turso dependency exist anywhere (verified); `docs/atproto-domain-model.md:209-216` ("SQL is shown in the SQLite dialect") with entity DDL at `:217-287`; `docs/atproto-stack-decisions.md:89-91` ("derive the DB DDL from the Rust types")
- **Why:** The decided schema exists only as never-executed markdown SQL while the plan is Turso from day one. Making the entity subset executable proves the dialect claim, constraint enforcement, and JSON-column choices before any migration code is written; the passing test plus its setup fn seeds the local Turso dev harness.
- **Scope trim (from verification):** Extract ONLY the entity-subset DDL (user, context, document, post, member, comment; `domain-model.md:217-287`) into schema.sql, adding the `legacy_id` import-mapping column and folding dialect fixes back into the doc. One `#[test]` in the `crates/` workspace opens in-memory libsql with `PRAGMA foreign_keys=ON`, executes the DDL, round-trips one row per table, and asserts the invariants that actually exist in the doc: the `context(parent_id, slug)` UNIQUE index, the kind/visibility/role CHECK constraints, FK enforcement (including that it is OFF without the pragma), JSON-column round-trip, `datetime('now')` text defaults. DROP the token-collision and weight-CHECK tests (no such columns exist in the doc DDL; E2E-V material) and the BEGIN IMMEDIATE cast-transaction test (touches the deferred voted/ballot tables). REFRAME: schema.sql is a provisional dialect-validation artifact and dev-harness seed, explicitly marked as to-be-regenerated by the Rust-type-to-DDL derivation per `stack-decisions.md:91`, not a competing canonical source. Poll/ballot tables join once item 5 lands.
- **Cost:** S

#### 12. Extend the public-subset lexicons (statement, group, event) and codify the versioning policy

- **Category:** crypto-schema
- **Artifact:** `lexicons/wiki/radikal/` holds only post, resolution, comment; `lexicons/README.md:17-18` marks group/event/document "later"; statement is in the decided public subset (`docs/atproto-stack-decisions.md:85`) and fully sketched at `docs/atproto-domain-model.md:136-157` but has no file; `README:39-41` conventions lack any additive-evolution rule
- **Why:** Transfers 100 percent through atrium-lex codegen and is unblocked regardless of item 2: both scope options agree the public subset gets lexicons. The three shipped files set the house style, so marginal cost per file is low; the versioning section is the cheap insurance that keeps every published record readable forever.
- **Scope trim (from verification):** Draft statement.json (transcribe the sketch at `domain-model.md:136-157`), plus group.json and event.json (fields from the context DDL at `domain-model.md:227-237`: name, slug, optional strongRef to a public parent, createdAt; kind split across the two NSIDs). Add a Versioning and evolution section to `lexicons/README.md`: new fields optional only, never retyped or promoted to required, enums extend only via knownValues, any breaking change mints a new NSID. EXCLUDE document.json until the public rich-text representation of Slate JSON is decided during the rewrite; record that exclusion and its reason in the README.
- **Cost:** S with the trim

#### 13. DRISL/DAG-CBOR plus CIDv1 known-answer round-trip spike

- **Category:** risk-spike
- **Artifact:** `docs/atproto-stack-decisions.md:67-78`, specifically `:74-75` ("pin conformance with known-answer/round-trip test vectors from a reference impl"), a decided extension with zero implementation: no ipld, dagcbor, cid, multihash, or multibase dependency anywhere (verified grep)
- **Why:** A scratch crate that hand-constructs sample records conforming to the drafted lexicons (which also smoke-tests the schemas), encodes them with `serde_ipld_dagcbor` plus cid/multihash, asserts byte-deterministic output and correct CIDs (codec 0x71) against reference vectors (the atproto interop fixtures), round-trips, and proves the no-floats rule holds. This is the exact encode path that migrates into the AppView publish seam, and the vector suite becomes its permanent conformance tests. A wrong CID means every published record is rejected or mis-addressed; same class of load-bearing unknown the blind-signature spike just retired. Stays out of repo commit signing, MST construction, and the signed-commit flow, which belong to the rewrite proper.
- **Cost:** S

#### 14. atrium-oauth spike: PDS-agnostic server-side login against two different PDS hosts

- **Category:** risk-spike
- **Artifact:** `docs/atproto-stack-decisions.md:147-152` (decision: adopt atrium-oauth, "own a thin wrapper layer", "pin exact 0.25.x versions"); `backend/src/oauth.rs` (28.8K hand-rolled PKCE/DPoP/PAR client it would replace); no atrium dependency exists anywhere
- **Why:** The identity plan assumes a 0.x crate supports the full server-side flow (handle to DID to PDS resolution, PAR, DPoP P-256, PKCE S256, session persistence, token refresh) against ARBITRARY member-chosen PDSes, not just bsky.social. A scratch binary wraps atrium-oauth behind exactly the one-file wrapper the stack doc mandates and runs login plus refresh against bsky.social AND one non-Bluesky PDS (eurosky or w.social) with a persisted session store. A negative answer makes `oauth.rs`, `dpop.rs`, and `pkce.rs` the permanent implementation and rewrites that plan step, far cheaper to learn now. The wrapper file is the same wrapper the AppView ships; the finding is keeper knowledge either way.
- **Cost:** S

#### 15. Turso ballot-core gate harness: kill-9 crash-recovery plus the SQLite file-format bridge claim

- **Category:** risk-spike
- **Artifact:** `docs/atproto-stack-decisions.md:24-33` (gate: ballot core on Turso "once it is 1.0 with proven crash-recovery (Antithesis coverage)"; "WAL + synchronous FULL + fullfsync, verified via PRAGMA readback"; lossless plain-SQLite bridge claim); the atomic cast transaction at `docs/atproto-domain-model.md:318-327`
- **Why:** The doc's own trust gate for the most safety-critical datastore has no defined pass criteria and no trigger, and is not even tracked in the open-decisions list. An engine-parameterized harness loops the exact BEGIN IMMEDIATE dedup-plus-ballot transaction under `kill -9` against both rusqlite and the turso crate, asserting post-crash atomicity (both rows present or both absent, `PRAGMA integrity_check` clean) and that a Turso-written file opens unmodified in stock sqlite3, the lossless-bridge claim the migration story rests on. Record crate version and 1.0/Antithesis status as the measurement. The tested shape (unique-constrained dedup insert plus append-only ballot insert under BEGIN IMMEDIATE) is invariant under the coming dedup-key change to token nullifiers, so the harness transfers unchanged as the ballot core's durability suite. Honest limits: kill -9 exercises process-crash atomicity, not power loss; Antithesis coverage is recorded, not claimed locally testable.
- **Cost:** M

#### 16. Onboarding-reality walkthrough: can a non-technical assembly member actually obtain a DID and link it

- **Category:** risk-spike
- **Artifact:** `docs/pre-rewrite-plan.md:21-29` (0 DIDs linked, the link flow has never been used, 83 percent roster-only); PDS-agnostic decision at `docs/atproto-open-decisions.md:22-24`; the invite-to-bind flow at `docs/atproto-domain-model.md:350-361`
- **Why:** The entire identity migration assumes members self-register at some PDS and link, and that assumption is completely untested against real users. A timed scripted walkthrough with 2 or 3 real pilot members across two PDS choices, recording every step, minute, and stall point, plus a PROPOSED re-run of the read-only DID audit in 4 to 6 weeks to watch conversion. A negative answer forces designing an org-assisted DID provisioning path into the cutover plan before the invite-to-bind flow hardens, a plan-invalidating finding if discovered at migration time. Only possible while the interim app is live and members log into it.
- **Cost:** S

#### 17. Graduate deferrals (b)+(c)+(d), phase 1: canonical domain-types crate plus the read-only migration extractor, content and membership entities only

- **Category:** data-hygiene
- **Artifact:** `docs/pre-rewrite-plan.md:104-106` (the coupled deferrals; the "schema not yet built" blocker text is stale, the target SQL now exists at `docs/atproto-domain-model.md:209-316`); `docs/atproto-port.md:223-226` (step 2 greenlights the mapping); verified: zero backend-canonical serde types anywhere
- **Why:** The deferrals form a dependency cycle (extractor blocked on types, types coupled to the extractor) that only resolves as one unit, and every external prerequisite is now met. The mapping code is the front half of the real importer the AppView runs; the field-gap report is keeper knowledge even if rerun at cutover; only the trivial read SQL is throwaway.
- **Scope trim (from verification):** Voting entities removed rather than gated. (1) Item 4's member-DDL fix lands first. (2) A canonical domain-types crate in `crates/` hand-authors ONLY the stable content/membership entities: user, context, document, post, comment, member (the public trio's DB-side types marked provisional pending atrium codegen; no binding to interim Hasura JSON). (3) A standalone read-only Postgres extractor maps nodes, members, and the mimeId taxonomy into those types, emitting serde fixtures (synthetic or owner-approved data only) plus the field-gap report (every source column or JSONB key with no target slot, every target NOT NULL with no source), folding the free-text-author-to-DID census (item 6a) into it. EXCLUDED until item 5 and the spec crate settle shapes: any poll, voted, ballot, eligibility, or delegation types and their extraction; eligibility and delegation have no schema in any doc today. Any run against live data is proposed only, pending owner sign-off.
- **Cost:** L

#### 18. Extract the surviving handler-side Hasura query bodies behind one backend store module

- **Category:** axum-evolves
- **Artifact:** `admin_gql` call sites outside auth.rs: `backend/src/vote.rs:97, 142, 174, 211`; `backend/src/notify.rs:60, 109, 127, 218, 250, 317`; `backend/src/members.rs:53, 79, 127` (13 total, verified grep)
- **Why:** The claim flow, notify fan-out, and poll-meta lookups are domain logic the AppView keeps, but each embeds inline Hasura GraphQL. Intent-named store fns leave the handlers Hasura-free; at cutover only store.rs is rewritten against Turso. Same seam class round 1 already greenlit for auth.rs.
- **Scope trim (from verification):** Extract only the 9 surviving-intent sites into `backend/src/store.rs`: `vote.rs:97` (poll_meta), `members.rs:53` (member_by_claim_token), `members.rs:79` (bind_member_to_user, race-guarded), `members.rs:127` (member_claim_token), `notify.rs:60` (upsert_push_subscription), `notify.rs:109` (delete_subscriptions_by_endpoint), `notify.rs:127` (subscriptions_for_emails), `notify.rs:218` (node_owner_and_context), `notify.rs:317` (active_member_emails). Explicitly EXCLUDE `vote.rs:142, 174, 211` (interim secret-ballot marker/insert/status, replaced wholesale by blind signatures plus the bulletin board) and `notify.rs:250` (NHost users email lookup, dies with NHost identity); leave those inline with a one-line comment marking them interim-protocol code the rewrite deletes. Store fns return small file-local structs, not `serde_json::Value` and not a preemption of item 17's shared-crate types.
- **Cost:** S with the trim

#### 19. Typed backend error enum with correct status codes plus tracing events

- **Category:** axum-evolves
- **Artifact:** `backend/src/handle.rs:148-154` (`error_json` collapses its failures to 400 and logs via eprintln); `Result<_, String>` at `backend/src/auth.rs:28, 52, 63, 107, 124`; no tracing or log dependency in `backend/Cargo.toml` (verified)
- **Why:** The enum plus one response mapping is backbone code the AppView keeps wholesale; the auth.rs signatures are the Principal seam round 1 already declared rewrite-surviving, and typing their error type IS that surviving signature. Note the honest baseline: `vote.rs:30-51` already returns 409/403 by hand-matched strings and `roster.rs:76-82` already returns 401; the real gap is 401 for missing tokens on the other paths, 502 for upstream failures, and typing the string matches.
- **Scope trim (from verification):** Define AppError (Unauthorized 401, Forbidden 403, Conflict 409, BadRequest 400, Upstream 502) with one to-Response mapping replacing `error_json`'s body shape; convert the five auth.rs signatures and the fallthrough sites so missing token maps to 401 and `admin_gql` failure maps to 502; replace the hand-rolled string-match arms in `vote.rs:30-51` with typed variants. Add the tracing crate with a minimal subscriber in main.rs and swap the existing eprintln sites (`handle.rs:149`, `:158`, `auth.rs:41`) for `tracing::error` events. Out of scope: span instrumentation of handlers (polish on code that largely dies at cutover), restructuring the `admin_gql` chain beyond its error type, any XRPC-shaped error body (a one-function change during the rewrite). Cheapest done together with item 18 (same files).
- **Cost:** S with the trim

#### 20. Reconnect-with-backoff plus refresh-on-reconnect in the subscription hook layer

- **Category:** frontend-kept
- **Artifact:** `src/subscription.rs:89-158` (`open_subscription` wires only onopen and onmessage; no onclose, no onerror, no retry); the sole recovery path is window-focus refresh (`src/subscription.rs:37-64`); the projector screen holds three long-lived focused subscriptions (`src/components/screen.rs:28, :65, :159`)
- **Why:** A dropped socket (server restart, venue wifi blip) silently freezes live views, and the assembly projector is exactly the tab that stays focused for hours, so the focus-refresh fallback never fires and the live tally freezes with no recovery. The fix lands once in the shared hook layer all 11 subscription call sites go through, touching zero components: onclose/onerror handlers, capped exponential backoff, reopen, bump the refresh signal on reconnect, re-read the session token on reopen (the captured one may be stale). Reconnect-plus-refetch-on-reconnect IS the client discipline the rewrite's single multiplexed axum WS requires (`docs/atproto-domain-model.md:369-371`); only the graphql-transport-ws handshake inside `open_subscription` is throwaway. Must NOT grow a shared connection manager (WS multiplexing stays deferred); the per-socket state machine gets lifted mechanically later.
- **Cost:** S to M

#### 21. Split the surviving backend API client into src/backend_api.rs with env-overridable endpoint URLs

- **Category:** frontend-kept
- **Artifact:** `src/nhost.rs:22-23` (BACKEND_URL const pointing at the prod Scaleway container, so every local dev build hits production) and its 13 endpoint fns (`nhost.rs:41` through `:349`, covering /atproto/*, /roster/parse, /vote/*, /push/*, /members/*); NHost auth glue at `nhost.rs:455-686`; seam leak at `src/components/profile.rs:230-233` (inline `{BACKEND_URL}/atproto/start`); `test-browser.nu:610, 1555, 1815` (one Hasura URL hardcoded three times)
- **Why:** The 13 backend fns, the public.api.bsky.app search, and AtprotoLink/BskyActor survive cutover (the axum backend already serves these exact paths per `backend/src/handle.rs`'s route table) while the NHost auth and storage fns die with NHost; today both halves share one module, so deleting NHost means editing all 12 importing component files at cutover. Moving the survivors to `src/backend_api.rs` (with an `atproto_start_url` helper absorbing profile.rs's inline URL) makes the deletion boundary exact: delete nhost.rs, keep backend_api.rs. Plus `option_env!("WIKI_BACKEND_URL")` / `option_env!("WIKI_GRAPHQL_URL")` fallbacks with today's constants as defaults (bit-identical prod builds; precedent at `src/logging.rs:25-27`) and one env-read gql-url def in test-browser.nu, so the kept frontend and the 2078-line WebDriver harness can point at a local or staging AppView without editing five hardcoded sites across two languages. Mechanical relocation plus configuration only; zero call-site or type reshaping.
- **Cost:** S

#### 22. Relocate the four component-facing plain types (Author, Crumb, BallotRules, MemberPageFilter) from graphql.rs to model.rs

- **Category:** frontend-kept
- **Artifact:** definitions at `src/graphql.rs:2917, :1977, :1121, :1362`; component imports at `src/components/editor.rs:126-127, :148, :263`, `member.rs:50, :98, :184`, `layout/mod.rs:30`, `vote/poll.rs:814`; the in-code note at `src/model.rs:251-253`
- **Why:** The model.rs note kept these in graphql.rs because they never carried a cynic derive, but that rationale addresses wire coupling, not module lifetime: graphql.rs is the 3671-line throwaway mapping layer deleted at cutover, and these are the last four types components import from it (verified exhaustively: every other `graphql::` reference in components is a function). Moving them completes the seam invariant (components take domain types only from model.rs) as a zero-behavior import-path change across four component files now instead of during cutover. The pure helper `deepest_context_depth(&[Crumb])` may reasonably move along with Crumb.
- **Cost:** S

### Explicitly defer to the rewrite

No items were refuted this round; the adversarial verification confirmed or trimmed all of them, and the trims are folded in above. What remains deferred:

- **document.json lexicon.** The public rich-text representation of Slate JSON (facets-style structured text vs blob vs serialized string) is undecided; guessing it now freezes a wrong record shape. Decide at the publish seam during the rewrite; the exclusion is recorded in `lexicons/README.md` by item 12.
- **Voting-entity canonical types and extraction (poll, voted, ballot, eligibility, delegation).** Nothing settled to author against until item 5 lands and the spec crate pins token/board shapes; eligibility and delegation have no schema in any doc today. Removed from item 17's scope, not gated inside it.
- **Store extraction of `vote.rs:142, 174, 211` and `notify.rs:250`.** Interim secret-ballot protocol and NHost email lookup; both die wholesale at cutover, so wrapping them is money spent on doomed code. Marked inline as interim by item 18.
- **A backend-manifest cargo-test CI job.** 12 of the 34 backend tests are throwaway Hasura-stack tests and the ~2 GiB tangled microVM cannot compile the backend dep tree (`workflows.ncl:23-37`); `doCheck = true` plus the crates-only check cover the keepers.
- **Tracing span instrumentation of handlers.** Instruments call chains (cast_inner, admin_gql) that are replaced by blind signatures and Turso; events plus a subscriber transfer, span trees do not.
- **XRPC-shaped error bodies.** A one-function serialization change during the rewrite; the AppError enum and status mapping are what transfer.
- **NSID minting and lexicon publication.** Gated on item 3's owner call; all drafting proceeds on placeholders, rename is a mechanical find-replace.
- **A shared WS connection manager or multiplexing.** Still the standing round-1 deferral; item 20 must stay per-socket.
- **Everything on the round-1 refuted and forbidden lists** (`docs/pre-rewrite-plan.md:101-109`) **stays refuted.** No unlock condition changed except where an item above states exactly what changed (the spec crate via the passed spike, the poll-lexicon remainder via its never-landed fold-in).

### Sequencing

Open with the three owner calls (items 1 to 3): all paper, all S, and together they gate the crypto track, the types-and-extractor track, and NSID permanence; nothing else should freeze shapes before they close. In parallel, land the independent keepers that need no decision (items 18 to 22, cheapest with 18 and 19 done together) and launch the four spikes (13 to 16), since each can invalidate a plan step and all are decision-free. Once item 1's encoding call lands, do the schema fixes (4, then 5) and stand up the `crates/` workspace seeded with the ballot-math spec crate (7 and 8 land together), co-designing the custody memo, poll and board-entry lexicons, and the verify-UX paper (9 and 10) against the spec crate's token and message shapes; items 11 and 12 slot in anywhere after 5. The censuses (6) run as soon as the team can execute the read-only queries, because their numbers must precede the DDL freeze and the extractor mapping. Item 17 goes last: it needs item 2 closed, item 4 landed, and census input, and its field-gap report is the round's exit artifact. Round 2 ends when the spec crate's property suite is green in the crates check, the custody and encoding calls are recorded in `atproto-open-decisions.md`, all four spikes have recorded verdicts, and the extractor emits fixtures with a field-gap report containing no unexplained rows. At that point the rewrite itself starts by standing up the AppView service crate in `crates/` against the reconciled schema, moving the transferable backend modules (push, dpop, pkce, statecookie, oauth, util) into the workspace, wiring atrium-oauth behind the spike's wrapper, and running the importer front half against a staging Turso database, with the interim app repointed via item 21's env knobs for the cutover rehearsal.
