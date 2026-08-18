# Rewrite kickoff plan

Pre-rewrite is done. Rounds 1 and 2 shipped the seven spike/keeper crates (`crates/Cargo.toml:8`), the anti-corruption seam (`model.rs`), the env repoint knobs (`src/backend_api.rs:18`, `src/nhost.rs`), and every load-bearing decision in `docs/atproto-open-decisions.md`. The question is no longer *whether* the custom Rust AppView is buildable, it is *how* to start building it. Three things define this phase. First, a walking-skeleton first slice: an 8th workspace member (`crates/appview`) that owns config, the Turso pool with the per-connection FK-pragma helper, the in-process broadcast channel, and one `/ws` handler, into which every transferable module and every service slice then lands. Second, a deploy-topology shift: the AppView is a single stateful always-on process and cannot run on the scale-to-zero serverless the interim backend uses (`backend/default.nix`), so a persistent-process deploy target with replication and observability is now real work, not an afterthought. Third, a handful of paper owner sign-offs sit directly on the critical path: they are cheap to close now and expensive to guess wrong after the AppView ships.

## Critical path

The ordered spine from today to a working AppView against staging:

1. **Stand up the `crates/appview` skeleton** (config, Turso pool + FK-pragma helper, broadcast channel, one `/ws` handler; no handlers, no persistence). Gated by nothing. Every item below consumes this empty slot.
2. **Reconcile the schema to a single generated source of truth** (`domain-types::ddl()`), fixing the author-provenance model (join table, free-text-capable `document`/`post`/`comment`, user-row emission). Gated by nothing. Blocks the Store port and the loader, which both hard-code columns.
3. **Port the Store/authz seam** (content + membership fns) onto the reconciled schema against a Turso db. Gated by step 1 + step 2. The identity-keyed predicates `is_active_member`/`is_active_owner` are a separate DID-keyed rewrite, not a mechanical body swap: they wait on the DID-binding flow.
4. **Complete the atrium-oauth interactive slice** (`/callback` + durable SQLite stores) in the appview crate. Gated by step 1. Identity is the top migration risk (0 DIDs linked), so this is the untested half of the foundation the big-bang cutover assumes.
5. **Migration pipeline back-to-front**: dump script, then the staging-Turso loader with `legacy_id` idempotency. Gated by step 2 (the loader needs a shape that holds extractor output, including user rows).
6. **Cutover runbook + a persistent-process deploy target** so a staging AppView is reachable and the importer front half can run against it. Gated by step 1 (the deploy needs a real binary to package).

Owner decisions that gate this spine, closable now as paper: the **onboarding-walkthrough result** decides whether an org-assisted DID provisioning path must enter the cutover runbook (gates step 6's runbook, and the window closes when the interim app retires). The **ballot D1-D8 sign-off** and the **board-custody batch** gate the ballot vertical, which is deliberately *not* on the AppView kickoff spine but should be unfrozen in parallel while it is still paper.

## Do now (prioritized)

Decision-closure items that gate the critical path come first, then execution ordered by unblocking power, then de-risking, then cost.

### 1. Run the onboarding walkthrough to decide whether org-assisted DID provisioning enters the cutover plan

- **Kind**: decision-closure
- **Why**: The single biggest migration unknown is whether members can self-obtain and link a DID. 0 DIDs are linked system-wide and the link flow has never been used (`docs/onboarding-walkthrough.md:11`), while the big-bang cutover assumes self-service. The link surface is already shipped (`src/components/member.rs:32-38` renders the not-linked nudge via `backend_api::atproto_status`; `src/components/profile.rs:222-238` drives `atproto_start_url`), so the walkthrough needs no new code, only execution. Any trigger firing (T1-T4) forces an org-assisted path (batch account creation, org-run PDS, or in-app signup wizard) into the cutover plan NOW rather than at migration time, which `docs/pre-rewrite-plan.md:293` calls a plan-invalidating finding if discovered late. The window closes when the interim app retires at big-bang.
- **First step**: Get owner sign-off on running `docs/onboarding-walkthrough.md`, have the owner nominate the low-tech pilot P1, and do the facilitator dry-run (throwaway signup + link on both assigned PDSes) at most 24h before the first session.
- **Blocked by**: owner sign-off + member consent + a live interim app session.
- **Cost**: M

### 2. Stand up the AppView service crate skeleton in `crates/`

- **Kind**: appview-build
- **Why**: This is the literal first named rewrite step in `docs/pre-rewrite-plan.md:139-142` and `:357`, and the empty slot every other execution item consumes. The workspace today is the 7 spikes/keepers only (`crates/Cargo.toml:8`); no `axum`/`tokio::broadcast`/`websocket`/`jetstream` source exists. Standing up the 8th member depending on `wiki-domain-types` and `ballot-spec` proves the pool + pragma + broadcast wiring in isolation and gives `push`/`util`/`statecookie`/`store`/`auth`/XRPC/firehose/OAuth-callback a real place to land. Every decision it needs is closed in `docs/atproto-open-decisions.md`; the one open call (NSID domain) does not gate it because the skeleton has no lexicon/XRPC handlers.
- **First step**: Add an `appview` member to `crates/Cargo.toml` and create `crates/appview/src/main.rs` with an axum Router bound to `$PORT`, `Config::from_env`, a Turso/libsql pool, an `acquire()` that runs `PRAGMA foreign_keys=ON` then asserts the read-back is 1 (the per-connection build-time default, `crates/schema/schema.sql:15`, `crates/schema/tests/roundtrip.rs:15,176-186`), a `tokio::sync::broadcast` channel, and one `/ws` handler; path-depend on `wiki-domain-types` + `ballot-spec`; confirm `cargo build -p appview` succeeds.
- **Blocked by**: nothing.
- **Cost**: M

### 3. Make `schema.sql` a generated artifact and reconcile it to the extractor's author/user model

- **Kind**: schema
- **Why**: The stack decision is explicit that the canonical DDL is RE-DERIVED from the Rust types (`crates/schema/schema.sql:7-13`, `docs/atproto-stack-decisions.md:111`), but `domain-types` has no derivation: `schema.sql` and `domain-types` are two hand-authored sources of truth that can silently drift. The extract and load halves are grounded on incompatible shapes and nothing has ever round-tripped extractor output against the DDL: (1) `schema.sql:49` has `document.author_did` as a single scalar FK, but `Document.authors` is `Vec<Author>` up to 8 with 42% free-text and there is no author join table; (2) `schema.sql:95` has `comment.author_did TEXT NOT NULL` yet the extractor emits free-text authors (`crates/migration-extractor/src/lib.rs:234-239`); (3) `post.author_did` is `NOT NULL` at `schema.sql:60` but `Post.author` can be free-text; (4) `extract()` never constructs a `User`, so nothing populates the `user` table every FK requires. Collapse both sources into one generator and reconcile to the ALREADY-MADE provenance decision before the Store port and the loader hard-code column names.
- **First step**: Add a `ddl()` emitter to `crates/domain-types` for user/context/document/post/member/comment (each with `legacy_id`), add a `document_author` join table (`document_id`, `author_did NULL`, `author_text NULL`), make comment AND post authorship free-text-capable (nullable `author_did` + `author_text`), and emit a user-row realization path. Route the emitted DDL through `wiki-schema`'s existing dual-engine harness (`crates/schema/tests/roundtrip.rs`), add one assertion that a real multi-author + free-text `Extraction` loads against the generated DDL and FAILS against today's `schema.sql`, then retire `schema.sql` (and fix `docs/atproto-domain-model.md:249,259,300`) as generated output.
- **Blocked by**: nothing. (Framing: reconcile to a decision already made, not a new owner call.)
- **Cost**: M

### 4. Port the Store + auth seam (content/membership fns) onto Turso SQL

- **Kind**: migration
- **Why**: `store.rs`/`auth.rs` are the seam where at cutover only this module is rewritten against Turso (`backend/src/store.rs:3-4`). Every body today is admin GraphQL (`backend/src/auth.rs:22-50` `admin_gql`; `store.rs:30-38` posts to `cfg.hasura_url`). The intent-named surface and `Principal{uid,email}` shape stay; the query bodies change from GraphQL to SQL against the reconciled DDL. Doing it against the dialect-validated schema surfaces turso gaps (the recorded 0.2.2 nullable-UNIQUE-INSERT rejection) at the query layer rather than mid-cutover, and unblocks every handler that acts on the caller's behalf.
- **First step**: In `crates/appview` define a `Store` trait mirroring `store.rs`'s signatures and port `node_owner_and_context` first (a plain node/context read, no voting, no identity coupling) as a parameterized SQL query against a `schema.sql`-seeded Turso db in a unit test asserting the same struct comes back. Then `member_by_claim_token`, `bind_member_to_user`, `active_member_emails`, `subscriptions_for_emails`, `upsert_push_subscription`.
- **Blocked by**: appview skeleton + Turso pool (item 2); schema reconcile (item 3). EXCLUDES `is_active_member`/`is_active_owner`: they cannot be honestly ported keyed on `uid` while the target keys authz on `user_did` (`docs/atproto-domain-model.md:399-403`), 0 DIDs are linked, and no uid->DID resolution exists. Defer them to the DID-binding flow, or treat as a deliberate DID-keyed rewrite. `poll_meta` and any voting fn are deferred with the voting shapes.
- **Cost**: L

### 5. Complete the atrium-oauth interactive slice: `/callback` + durable SQLite stores

- **Kind**: risk-spike
- **Why**: The single named open technical remainder of the identity plan. The spike PROVED the PDS-agnostic part live (resolution + PAR + authorize against non-Bluesky PDSes) but `WikiOAuth` exposes only `new()` + `begin_login()` (`crates/oauth-spike/src/lib.rs`), and `docs/atproto-stack-decisions.md:70-72` records interactive token exchange + refresh + durable stores as NOT yet exercised. A negative answer (atrium-oauth 0.1.7 cannot complete server-side exchange or persist durably) forces a fork/patch or hand-rolled exchange, exactly the surprise cheaper to find before the AppView ships. The `/callback` needs an HTTP handler, so it is an early appview-crate slice. NOT gated by the NSID domain: the localhost client-metadata profile (`127.0.0.1/callback`) needs no served metadata document.
- **First step**: (a) Agent-completable code: add a `/callback` route driving `WikiOAuth::callback(params)` to a token, implement SQLite-backed `StateStore` + `SessionStore` replacing the `Memory*` impls, compiling and unit-covered, plus a documented run harness. (b) Live confirmation: drive one full `begin_login` -> redirect -> callback -> token-exchange -> refresh against a test/local PDS where the redirect can be scripted, recording how far it gets.
- **Blocked by**: appview skeleton (item 2). The against-a-real-independent-PDS interactive run may need a human browser step (per `FINDINGS.md` honest limits); gate only that sub-step on human execution, not the code.
- **Cost**: M

### 6. Move the genuinely decoupled modules (`util`, `statecookie`) into the appview crate

- **Kind**: appview-build
- **Why**: These are the transferable modules with the least entanglement, so moving them first banks progress and establishes the module-migration mechanics on code that needs no query rebinding. `util.rs` depends only on base64/rand; `statecookie.rs`'s only coupling is the payload shape (`LinkState.nhost_user_id` at `statecookie.rs:20`; the HKDF-SHA256 + XChaCha20-Poly1305 seal/open are pure). The DID swap is the one real edit: at cutover identity is the atproto DID. EXCLUDE `push.rs` from this slice: it is NOT zero-coupling (`push.rs:13` uses `crate::oauth::Config`, `oauth.rs:32`, a struct carrying `hasura_url`/`admin_secret`/`nhost_jwt_secret`), so it needs a separate step that first extracts a narrow VAPID-only config type. Exclude `dpop.rs`/`pkce.rs`/`oauth.rs`: atrium-oauth supersedes them per the spike, so moving them is churn on soon-dead code.
- **First step**: Copy `backend/src/{util.rs,statecookie.rs}` into `crates/appview/src/`, wire them as modules, change `LinkState.nhost_user_id` to `did: String` at `statecookie.rs:20`, and get `cargo build -p appview` green.
- **Blocked by**: appview skeleton (item 2).
- **Cost**: S

### 7. Bind real lexicon-typed content record structs to the dagcbor encode + CID path

- **Kind**: appview-build
- **Why**: The dagcbor-spike encode path (`crates/dagcbor-spike/src/lib.rs:19-38`) is a verbatim keeper for the AppView publish seam but is wired only to the throwaway `SamplePost` demo (`lib.rs:42-51`) with a hardcoded `$type`. No drafted lexicon (post/comment/resolution under `lexicons/com/example/wiki/`) has a Rust record struct bound. A wrong CID means every published record is network-rejected, so wiring the real content records through `cid_of` proves the DRISL/DAG-CBOR no-floats/deterministic rules hold for production shapes and gives the publish seam its typed input. Content records are decided public-by-default; NSID stays the `com.example.wiki.*` placeholder (a mechanical find-replace later) so nothing is blocked as long as no record is minted.
- **First step**: Add serde record structs for `com.example.wiki.post`, `com.example.wiki.comment`, and `com.example.wiki.resolution` mirroring the drafted lexicon fields, round-trip them through dagcbor-spike `encode`/`decode`, add CID known-answer vectors alongside `SamplePost`'s, and replace `SamplePost` as the demo.
- **Blocked by**: nothing (local encode/vectors under the placeholder). DROPPED from this item: the ballot crypto byte-encodings (nullifier/MessageRandomizer/Signature) and adding serde to `ballot-spec`, which are item 9 (board/poll record design), gated on the D1-D8 batch and the board-custody call.
- **Cost**: M

### 8. Write the read-only interim dump script (front of the migration pipeline)

- **Kind**: migration
- **Why**: The extractor consumes a `{ nodes, members }` snapshot (`crates/migration-extractor/src/main.rs:2-18`) that a separate read-only script is documented to produce, but that script does not exist (`scripts/` holds only `check-css-spacing.nu`, `gen-theme.ts`, `serve-up.nu`, `test-reuse.nu`). The prior census queries were throwaway and never committed, so there is no re-runnable, owner-reviewable way to produce the extractor's input. Committing it read-only and PII-free-by-construction turns the owner-approved live run into a reviewable artifact.
- **First step**: Create `scripts/dump-interim-snapshot` as a read-only query over the admin-secret Hasura GraphQL surface (not a Postgres replica, which is unverified), selecting exactly the `InterimNode` fields (id,name,key,mimeId,parentId,contextId,ownerId,data,createdAt) and `InterimMember` fields (id,name,email,nodeId,parentId,accepted,active,owner,claimToken) into `{ nodes, members }` JSON. Add a committed tiny `snapshot.json` fixture and a test asserting the `Snapshot` wrapper parses (`main.rs:26`) and `extract` runs.
- **Blocked by**: owner approval to run the dump against live data (the script itself needs no decision). Not the kickoff; a pre-staged pipeline artifact.
- **Cost**: M

### 9. Build the staging-Turso loader (back of the migration pipeline)

- **Kind**: migration
- **Why**: The pipeline stops at `extraction.json`: `extract()` emits `Extraction` but nothing writes those rows into a db, and no crate depends on both `migration-extractor` and `schema`. The loader is the natural home for the idempotency the schema was designed for (every table carries `legacy_id UNIQUE`, `schema.sql:24,37,53,67,84,98`), for FK-ordering (users and contexts before their dependents), and for exercising the two dialect findings (FK pragma per connection; turso 0.2.2 nullable-UNIQUE-INSERT rejection). It converts the field-gap report from paper into a load-fails-here signal.
- **First step**: Add `crates/migration-loader` depending on `wiki-schema` + `wiki-domain-types` + `migration-extractor` that runs the generated DDL on a rusqlite/turso file and INSERTs an `Extraction` in FK order using `legacy_id` for idempotency; prove idempotency with a test loading the same synthetic `Extraction` twice and asserting no duplicate rows.
- **Blocked by**: the schema reconcile (item 3), which must first add user-row emission and the author join table so the loader has a shape that can hold extractor output.
- **Cost**: L

### 10. Draft the cutover runbook

- **Kind**: ops-deploy
- **Why**: Big-bang cutover is the decided strategy (`docs/atproto-open-decisions.md:50`) and `pre-rewrite-plan.md:357` names its rehearsal (dump -> extract -> load to staging Turso -> repoint via the env knobs), but no runbook enumerates the ordered steps, the verification gates, or the rollback. The env repoint mechanism exists (`src/backend_api.rs:18`, `src/nhost.rs:14`). Writing it now sequences the three pipeline pieces, pins the verification queries that turn the field-gap report into a go/no-go gate, and surfaces the assisted-DID branch item 1 may force. It is paper, cheap, and what the migration items assemble into.
- **First step**: Create `docs/cutover-runbook.md` with an ordered checklist (interim freeze -> read-only dump -> extract -> load to staging Turso -> verification gates -> flip via env knobs -> rollback). Cite as EXISTING only the verified artifacts (`crates/migration-extractor`, `crates/schema/schema.sql` with per-table `legacy_id UNIQUE`, the two env knobs) and mark the dump script and loader as TO-BE-BUILT, naming the interfaces they must satisfy. Gates: per-table row counts, `legacy_id` coverage vs source uuid counts, `FieldGapReport` unmapped/unfilled all empty, and the 1962-distinct-emails-behind-17655-rows dedup landing under the `member_pending` partial unique.
- **Blocked by**: nothing (the assisted-DID branch stays a placeholder until item 1 lands).
- **Cost**: S

### 11. Build the stateful-AppView deploy target

- **Kind**: ops-deploy
- **Why**: The only deploy derivations are `backend/default.nix` (Scaleway Serverless, scale-to-zero) and the static frontend; no unit or VM derivation for a persistent process exists. The AppView is a single stateful process (Turso core+view, firehose, in-process broadcast, WebSocket server) that CANNOT run scale-to-zero (`docs/atproto-port.md:183-186`). It must not ship blind: `src/logging.rs` is a WASM/browser Log impl (server-unusable) and the interim backend has no process telemetry, so structured logging + a health signal are table stakes for a single always-on process where a stalled firehose is invisible until users complain.
- **First step**: Add a Nix `buildRustPackage` derivation for the appview binary plus a systemd/NixOS unit with restart-on-failure, a `StateDirectory` mounting a persistent volume for the Turso file, env for firehose+port, a `tracing-subscriber` JSON layer (server-side, NOT `src/logging.rs`; reuse the backend's `tracing-subscriber` dep and the `BETTERSTACK_INGEST_HOST` sink, `backend/src/oauth.rs:78`), and a `/healthz` reporting firehose-connected + DB-reachable. Confirm `nixos-rebuild build-vm` brings it up behind Ferron, survives a simulated restart with the data dir intact, `/healthz` returns liveness, and an induced error appears in the sink.
- **Blocked by**: appview skeleton (item 2) must exist and open a Turso file + firehose stub so `/healthz` and the state-directory soak are exercisable. Host vendor (Hetzner vs Scaleway Instance vs UpCloud) defaults to a single reproducible VM image.
- **Cost**: L

### 12. Durable ballot core: bind `ballot-spec`'s in-memory Board to a crash-tested store

- **Kind**: appview-build
- **Why**: The missing load-bearing integrity piece, buildable now. `ballot-spec`'s Board is a pure in-memory `Vec` (`crates/ballot-spec/src/lib.rs:333`) while `Board::cast` already verifies/dedups/validates and returns board position; the exact atomic cast transaction (`BEGIN IMMEDIATE` + UNIQUE-token dedup insert) is ALREADY proven kill-9 crash-safe on SQLite and Turso by `crates/durability-harness/src/main.rs`, but the harness uses a synthetic inline INSERT and never calls `Board::cast`. The two halves are proven separately and never joined. Needs NO custody call and NO D1-D8 call: it depends solely on the already-decided UNIQUE-token dedup (D4 first-entry-stands) and the check-order invariant, and `board_entry` carries no voter column regardless of encoding.
- **First step**: Add a persistent Board backend (module or crate over `ballot-spec`) whose `cast()` runs the exact `BEGIN IMMEDIATE` transaction from `durability-harness/src/main.rs`, persisting only the token nullifier under `UNIQUE(token)` for dedup plus a monotonic position, and storing the rest of the entry body (`msg_randomizer`, `signature`, `choices`) as an OPAQUE provisional blob rather than named pinned columns (D7 marks those encodings PROVISIONAL for item 9; `schema.sql:12-13` defers the table shape). Point `durability-harness/tests/kill9.rs` at that path; assert a reused token yields `CastError::DoubleSpend` from the constraint and kill9 stays atomic over rows written by `Board::cast`. Add the off-node replication seam as a hook boundary only (no wire format).
- **Blocked by**: nothing.
- **Cost**: L

### 13. Stand up the eligibility + delegation + token-issuance service half (always-private, org-authoritative)

- **Kind**: appview-build
- **Why**: The private org-side half of the live ballot path, unblocked and shape-pinned. `EligibilityRoster::resolve()` and `TokenIssuer::new_for_poll`/`blind_sign` are pure and property-tested (weight conservation, cycles-void, ineligible-hop-void), and the eligibility/delegation/token_issued DDL is pinned in `docs/atproto-domain-model.md:328-355` with frozen-at-open `resolved_weight` and the one-shot `(poll_id,did)` marker. Needs NO board-custody call (it never touches the public board) and builds against the current D1-D3 rules with the property tests as enforcement, since overturning a D-decision is a code+test change.
- **First step**: Add poll/eligibility/delegation/token_issued tables (matching the domain-model DDL; include `poll`, which carries `issuer_pubkey` and is the FK target the other three reference; keep `board_entry` OUT) to `crates/schema/schema.sql` and validate on rusqlite+turso via the roundtrip harness, then write a freeze-at-open routine that calls `EligibilityRoster::resolve()` and writes `resolved_weight`, guarded so re-freezing an open poll is a no-op. Exercise `blind_sign` as a tested unit fed by `request_token()`, not wired to a live client.
- **Blocked by**: nothing. (Not on the AppView kickoff spine; the private ballot half, ahead of the custody-gated public-board half.)
- **Cost**: L

### 14. Route file/blob fetches through `backend_api.rs`

- **Kind**: frontend-migration
- **Why**: The seam shields the entity surface, but component paths fetch bytes by building URLs directly from `nhost::storage_url()` and bypass it. NHost Storage dies at cutover and the AppView serves blobs via a different path, so these sites break silently while every model.rs-mediated call keeps working. Relocating blob-URL construction behind one helper makes the file-fetch swap a one-line change at cutover instead of a scavenger hunt.
- **First step**: Add `pub fn file_url(file_id: &str, token: &str) -> String` to `src/backend_api.rs` (default body `format!("{}/files/{file_id}?token={token}", nhost::storage_url())`) and repoint the four genuine read sites: `loader.rs:450`, `file.rs:110`, `position.rs:180`, `position.rs:215`. Do NOT touch `src/export.rs:315` (`fetch_image_bytes` over arbitrary content-embedded image URLs, not an NHost blob). The `nhost.rs:369` upload path is a write, out of scope.
- **Blocked by**: nothing.
- **Cost**: S

### 15. Inventory the 11 `use_live` filters as scope-keyed live topics (paper)

- **Kind**: decision-closure
- **Why**: `use_live` ignores its payload (`src/subscription.rs:26-30`) and every consumer routes through it; the `SubState` reconnect/backoff machine (`subscription.rs:66-262`) lifts wholesale, only the Hasura where-strings need redesign. Capturing the 11 filter predicates as code-grounded facts feeds the still-open AppView realtime topic-granularity + subscription-restrictor design. Do NOT recast them as a frozen `(change-kind, scope-id)` broadcast API presented as the AppView build target: `docs/atproto-stack-decisions.md:138` decides only one multiplexed `/ws`, and `:59` leaves even the channel mechanism open, so freezing a topic taxonomy from the frontend side ahead of the restrictor is the exact guessing `pre-rewrite-plan.md:114` deferred. The read/write half is redundant: `model.rs` already IS the code-derived read/write contract.
- **First step**: Produce a plain inventory of the 11 `use_live` call sites (e.g. `poll.rs:180` `parentId _eq` + `mimeId vote/vote`; `comments.rs:120` `contextId`/`parentId` + `mimeId vote/comment`): for each, the WHERE fields and the scope key it keys to, plus the note that the public signature and `SubState` transfer while only the transport repoints. Frame it as decision-forcing input, not a frozen target. Drop the read/write table.
- **Blocked by**: nothing (the code swap onto topics waits on the AppView multiplexed-WS API, which does not exist yet).
- **Cost**: S

## Force these owner decisions now

Each is paper, on or adjacent to the critical path, and cheaper to close now than to guess.

- **Onboarding-walkthrough result (org-assisted DID provisioning yes/no).** Blocks: whether the cutover runbook must contain an assisted-DID path; the window closes when the interim app retires. Cheapest close: item 1 above, n=2-3 facilitated sessions with a nominated low-tech pilot; needs owner sign-off + consent + a live interim session.
- **Ballot-spec D1-D8 semantics.** Blocks: scoping the ballot issuance/board-publish slice without re-freezing wrong semantics; the board-entry byte encodings (D7). Cheapest close: a one-page sheet listing D1-D8 verbatim from `crates/ballot-spec/DECISIONS.md` with the property test that pins each (17 in `tests/properties.rs`) and the verify-UX consequence for D4/D7/D8, for the owner to ratify or amend. Overturning any is a localized code+test change today.
- **Board-custody batch (six sub-calls, `docs/ballot-board-custody.md:115-128`).** Blocks: the board-publish path and the inclusion-receipt design (`docs/ballot-verify-ux.md:89-114`), plus the custody-dependent encoding decisions. Cheapest close: one memo putting the six questions (receipt signing key, publication latency, receipt scope, mandatory close-out digest, mirror commitment, board location) with the doc's recommended defaults pre-filled (`ballot-board-custody.md:100-113`), verdicts recorded in `docs/atproto-open-decisions.md`. Note: closing custody does NOT by itself take `poll.json`/`ballotEntry.json` out of PROVISIONAL, because both lexicons mark their crypto fields pending the `ballot-spec` crate's pinned serialization, which does not exist yet; pinning that byte-level encoding (base64url-unpadded token/randomizer/signature, DER SPKI issuer pubkey, plus a known-answer vector) is a separate buildable-now scaffold task independent of the owner call.
- **NSID authority domain.** Blocks: minting/publishing any record (a mechanical find-replace before codegen). Does NOT block items 2, 5, or 7, which run under the `com.example.wiki.*` placeholder with nothing minted. Close whenever the real domain is chosen; not on the immediate critical path.

## Explicitly defer / not yet

- **XRPC read-parity handler echoed to the Dioxus app via `WIKI_GRAPHQL_URL`** (refuted): the seam speaks GraphQL, not XRPC; pointing `WIKI_GRAPHQL_URL` at an XRPC handler is the wrong protocol at the seam, and the blocked_by is false.
- **Firehose -> view echo / walking-skeleton loop** (refuted): premise is false; the read handler, Turso store, and publish seam it claims exist do not, so this is several items ahead of itself.
- **`WIKI_BACKEND_URL` knob in `test-browser.nu` / WebDriver staging-repoint proof** (refuted): the premise that the suite has no backend knob mischaracterizes the two mechanisms; not a symmetric gap to fill.
- **Ballot crypto byte-encodings on `ballot-spec` (nullifier/MessageRandomizer/Signature serde), `ballotEntry.json`/`poll.json` de-PROVISIONAL**: item 9 (board/poll record design), gated on the D1-D8 batch and the board-custody call. Force via memo, do not land as code now.
- **Eligibility/delegation/board_entry migration types + extraction**: no interim source rows exist (only `vote/poll` and anonymous `vote/vote` ballots), encodings are PROVISIONAL, and `resolved_weight` semantics are gated on D1-D8. Only `Poll` migration is buildable now (author a `Poll` type, map `vote/poll` nodes, report historical secret ballots as unmigratable in the field-gap report), and even that follows the schema reconcile.
- **Porting `is_active_member`/`is_active_owner`**: cannot be honestly ported keyed on `uid` while the target keys authz on `user_did` and 0 DIDs are linked; wait on the DID-binding flow or treat as a deliberate DID-keyed rewrite.
- **Moving `push.rs`**: not zero-coupling (`push.rs:13` uses `crate::oauth::Config`); needs a prior narrow-VAPID-config extraction, grouped with the atrium-oauth wiring.
- **`dpop.rs`/`pkce.rs`/`oauth.rs` move**: superseded by atrium-oauth per the spike; only session/authz glue transfers. Moving them is churn on soon-dead code.
- **Off-node ballot-log replication + rebuild-from-replica test**: the load-bearing integrity control (`docs/atproto-stack-decisions.md:41-42`), but it needs a durable persistent Board store to replicate. Build item 12 first (which settles the `board_entry` DDL and durable store), then add continuous WAL-shipping replication + the recovery test as the immediate next step.

## Deploy + ops shift

The interim backend is stateless and scales to zero on Scaleway Serverless Containers (`backend/default.nix`). The AppView is the opposite: a single persistent process holding the Turso core+view, a live Jetstream firehose connection, an in-process `tokio::broadcast` channel, and the WebSocket server (`docs/atproto-port.md:183-186`). It cannot run scale-to-zero, and that forces four things this phase must plan for, not discover at cutover. First, a host that runs an always-on VM/bare-metal process (Hetzner, Scaleway Instance, or UpCloud), defaulting to a single reproducible VM image so vendor stays an owner preference not a blocker. Second, Nix packaging that does not exist yet: a `buildRustPackage` native binary plus a systemd/NixOS unit with restart-on-failure and a persistent `StateDirectory` for the Turso file, behind a Ferron edge (item 11). Third, the load-bearing integrity control: off-node append-only ballot-log replication (`docs/atproto-stack-decisions.md:41-42`), because a public bulletin board that lives on one Turso file with no off-node copy degrades the E2E-V argument to trust-the-org; the durability harness only proves single-process crash atomicity, not power loss. Fourth, observability from day one: `src/logging.rs` is browser-only WASM, so the server process needs its own structured JSON logging to the existing `BETTERSTACK_INGEST_HOST` sink plus a `/healthz` reporting firehose-connected + DB-reachable, since a stalled firehose or wedged WebSocket is otherwise invisible until users complain.

## Sequencing

Weeks 1-2 are the spine's foundation and the parallel paper closes. Land item 2 (the `crates/appview` skeleton) first, since everything else consumes it, and item 3 (the generated, reconciled schema) alongside it, since the Store port and the loader both hard-code its columns. In parallel and off the compiler's critical path, force the three cheap owner decisions: run item 1 (onboarding walkthrough, the one whose window closes when the interim app retires) and circulate the D1-D8 and board-custody memos. Week 2-3 builds on the skeleton: port the identity-free Store fns (item 4), complete the atrium-oauth `/callback` + durable stores (item 5), move `util`/`statecookie` (item 6), and write the dump script (item 8) and loader (item 9) once the schema reconcile lands. Item 10 (cutover runbook) and item 11 (persistent-process deploy target) turn the pieces into a reachable staging AppView. The exit condition of the kickoff phase is a working walking skeleton against staging: the appview binary running as a persistent process behind Ferron with `/healthz` green and structured logs shipping, a reconciled schema loaded on a staging Turso db by the importer front half, the Store seam answering content/membership reads out of that db, and a member completing the atrium-oauth `/callback` login. The ballot vertical (items 12-13) and the frontend swap advance in parallel but are not part of that first exit gate.
