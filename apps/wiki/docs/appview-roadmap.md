# AppView roadmap: the new backend

## Goal

`crates/appview` becomes the ONLY backend the Dioxus app talks to. NHost auth,
Hasura GraphQL, NHost storage, the Postgres schema and the serverless axum
sidecar (`backend/`) all retire at the big-bang cutover
(`docs/cutover-runbook.md`).

Done means all of these hold:

1. Every read, write and live update the frontend performs today
   (`src/graphql/*`, `src/nhost.rs`, `src/backend_api.rs`, `src/subscription.rs`)
   is served by the AppView, behind the `src/model.rs` seam, with no request
   leaving for NHost or Hasura.
2. Every request is authenticated by an AppView session bound to an atproto DID,
   and authorized by DID-keyed membership. No placeholder auth remains.
3. `cargo test --workspace` in `crates/` exits 0, and the browser suite
   (`just test-browser`) passes against an AppView instead of NHost.
4. The migration pipeline (dump, extract, load) carries every interim row the
   frontend can show, and the runbook's verification gates are green on staging.

The kickoff phase (`docs/rewrite-kickoff-plan.md`) is finished: the skeleton, the
generated schema, the Store seam, the OAuth stores and callback, the firehose
consumer, the durable ballot core, the loader and the deploy unit all exist. This
document is the phase after it: turning that skeleton into the backend.

## Where it stands (2026-09-19)

Built and tested in `crates/appview`: config, the Turso handle, `/healthz`, a
relay-only `/ws`, the atproto OAuth client with durable stores and `/callback`,
the Jetstream consumer, nine identity-free XRPC reads, four write procedures, the
ballot board and roster DDL wired into the datastore, and the NixOS unit with its
VM test.

Not built: sessions, authorization, the membership surface, the node tree the
frontend routes by, meetings (speaker lists, projector), the voting procedures,
live topics, blobs, the carried-over sidecar endpoints, and the frontend's own
data layer.

## Findings that shape the plan

- **The workspace did not build in the devshell.** `atrium-oauth`'s default HTTP
  client pulls `reqwest/default-tls`, so OpenSSL, against the recorded
  rustls-only decision (`docs/atproto-stack-decisions.md`, Rust server
  libraries). Fixed in M0 by supplying the one rustls client the decision asks
  for.
- **Writes trusted the caller.** `xrpc::caller_did` read the DID straight out of
  the `Authorization` header, so anyone could write as anyone by naming them.
  Replaced in M1 by sessions.
- **`/login` is an SSRF primitive unless guarded.** It is unauthenticated and
  fetches whatever host the caller names. The outbound client now refuses
  non-https and non-public addresses, at resolution time, so DNS rebinding gains
  nothing. A dev instance (no public URL) stays unrestricted for a local PDS.
- **The engine was five minor versions behind, and it mattered.** turso 0.2.2
  had no `EXISTS`, no `IN (subquery)`, no upsert, and ignored the foreign-key
  pragma, so nothing was referentially checked. 0.7.2 has all four. Turning
  enforcement on surfaced two real defects at once (the firehose and the
  loader, both fixed). Recursive CTEs are still missing, which M3's path
  resolution has to design around.
- **The extractor leaves gaps the loader will now refuse.** A context's
  `parent_id` is carried over verbatim, but in the interim tree a group or event
  can sit under a folder, which is a `document` here, so that key dangles. The
  node that owns a context without holding a membership row in it (a real case:
  `docs/read-permissions.md`) gets no owner row. Interim public contexts are all
  extracted as private. Extracted polls are never loaded, and are extracted
  wrong: the question is read from `data.question` and the state from
  `data.open`, neither of which the interim writes (a poll's name is its node's
  name, and open is `mutable`), and `minVote`, `maxVote`, `hidden`, the place in
  the tree and the RESULT are not read at all. Ballots cannot be carried; the
  outcome of every past vote can, and `poll.counts` is where it goes. M3 and M9.
- **There are no schema migrations.** The entity tables are plain
  `CREATE TABLE`, so a datastore file made by an older binary keeps its old
  columns. Until migrations exist the file records its schema version and a
  binary refuses to start on another one, rather than failing a query at a
  time. Pre-cutover that costs nothing, since the view is rebuilt from the
  migration pipeline; after it, migrations become real work (M9).
- **The interim serves every member's email to every member.** A column
  permission is per role and an owner is role `user` too, so one plain member
  could read 1,467 of the organisation's 2,007 addresses. The AppView can draw
  the line Hasura could not: `listMembers` serves an address to an owner of
  that context and to nobody else, and matches a search against one for an
  owner only.
- **Reads were ungated.** Every read served private content to anyone. Gated in
  M2 on visibility and membership.
- **`active` is voting rights, not a read gate.** The interim reads by
  membership alone, and `migrations/0024` records what reinterpreting a write
  model as a read model cost: a context went dark for its own members. The
  AppView's gate follows the same line.
- **A document could not be addressed by path.** The entity schema gave
  `context` a slug and `document` none, while the frontend routes every node by
  its key path (`/a/b/c`), so every existing link would have broken at cutover.
  The extractor dropped the key without a word, and with it `index`, `mutable`,
  `attachable`, the owner, `updatedAt` and the bin; it REPORTED a file's
  `fileId` and `type` as gaps rather than carrying them, which would have
  migrated every attachment as an empty page; and `context.kind` rejected
  `wiki/site`. Reconciled in M3.
- **Two people writing at once, and one of them failed.** The engine's default
  is to refuse a write the moment another connection holds the write lock, and
  every request here has its own connection. Measured with 16 writers: 371 of
  400 transactions refused with "database is locked". A meeting is exactly
  that load (a room joining a speaker list, 500 ballots in a minute). Every
  connection now waits its turn, up to ten seconds.
- **Speaker lists were classed as ephemeral and dropped by the extractor**, but
  the speak app persists them and an assembly reads them back. M5 gives them
  tables; M9 migrates them or records why not.

## Milestones

Each lands with tests, committed on its own. A box is ticked only when the code
is merged and its tests pass.

### M0: build hygiene

- [x] One rustls `reqwest` client for every outbound call; OpenSSL out of the
  dependency graph; `cargo test --workspace` green in the devshell.
- [x] turso 0.2.2 to 0.7.2; foreign keys enforced and verified on every
  connection; the two-step upserts collapsed into real ones.
- [x] Concurrent writers wait for the write lock instead of failing.

### M1: identity and sessions (replaces NHost auth)

- [x] `session` table; opaque bearer tokens, stored hashed, with expiry.
- [x] `GET /login` (handle to authorization URL), bound to the browser by a
  cookie against login CSRF; `/callback` returns a one-time code to an
  allow-listed frontend origin, and `createSession` redeems it, so no credential
  is ever in a URL.
- [x] `getSession` and `deleteSession`; `Caller` and `MaybeCaller` extractors;
  the placeholder `caller_did` deleted.
- [x] CORS for the configured frontend origins.
- [x] Production client metadata (`/client-metadata.json`) when a public URL is
  configured; the loopback profile otherwise.
- [x] Outbound requests confined to public https addresses.
- [ ] Profile hydration on login (handle, display name, avatar into `user`).
- [ ] A confidential client (`private_key_jwt` and a served JWKS). Not needed
  until the publish seam writes to members' repos and wants long-lived PDS
  tokens; the AppView's own sessions do not depend on them.
- [ ] One full login in a real browser against a real PDS. Everything around
  the token exchange is covered; the exchange itself needs a human.

### M2: authorization core

- [x] DID-keyed `is_member`, `is_active_member` and `is_active_owner`, one module
  (the predicates the kickoff plan deferred). Membership reads and writes;
  `active` is voting rights and gates only what the interim predicates gated.
- [x] Reads gated in SQL, before any `LIMIT`: public, or member, or author. A row
  the caller may not read is indistinguishable from a missing one.
- [x] Writes gated: membership to create or comment; a comment's context comes
  from its subject, a document's parent must be in its context.
- [x] Invite binding by claim token (`claimMembership`), and the owner's claim
  link.
- [ ] Invite binding by email. Needs the account's confirmed address from its
  PDS (`transition:email` and an authenticated `getSession`), so it waits on
  the first authenticated PDS call.
- [x] Authorship or ownership to change a node (`Standing`, with
  `updateDocument` in M3).

### M3: the node tree the frontend routes by

The frontend knows one tree of typed nodes, each reachable by the path of keys
in its URL; the store keeps typed entities in separate tables. The design that
joins them:

- `context` and `document` are the two spines of the tree. Both carry `slug`
  (the interim `key`), a stored `path`, `idx`, `attachable`, `owner_did`,
  `updated_at` and `deleted_at`; `document` also `mutable` and `data`. A parent
  may be either kind (a group can sit in a folder), so `parent_id` is a plain
  column on both and the write path keeps it honest.
- `path` is STORED and maintained on write, as the interim does with a trigger.
  turso has no recursive CTEs, and a stored path makes resolution one indexed
  lookup at any depth. A rename or a move rewrites the subtree's prefix. It is
  unique among live rows only, so a binned node does not hold its URL hostage.
- The server picks the slug (`name`, then `name-2`, ...), because only it can
  see every sibling; the frontend finds collisions today by attempting inserts.
- Kinds that are nodes in the frontend but have their own state and a URL of
  their own (polls, canvases) become `document` kinds with a side table keyed by
  the same id, when their milestone arrives. Speaker lists and the projector
  have no URL, so they are plain tables beside the tree (M5).
- Who may create what under what is the interim's per-context template
  (`context_permission_objects`), as one static table: (kind, role, parents).
  Reading stays by membership (M2); this is the write model only.

The steps:

- [x] Schema reconcile: slug, stored path, idx, attachable, owner, updated_at and
  deleted_at on both spines; mutable and data on documents; the `site` kind.
  Generated DDL on both engines, and carried by the dump, extractor and loader.
- [x] Path resolution across contexts and documents, in one lookup.
- [x] The server picks a new document's slug, in one write transaction, across
  both tables' namespace.
- [x] `getNode`: a node by path or id with its children of both kinds in one
  ordered list, a crumb per segment (nameless where the caller may not read
  it), and the viewer's standing. One call where the interim makes several.
- [x] The write model: which kind a role may create under which parent, and the
  folder lock with its owner and discussion exemptions. One static table,
  carried over from the interim's per-context template. (A closed poll refusing
  votes belongs to M6.)
- [x] The ordinal that letters submitted motions (the interim's `get_index`,
  by its rule: submitted nodes only, by index, then last update, then id), on
  the page and in the listing alike; the profile behind every DID a page names.
  It inherits the interim's weakness: an edit bumps `updated_at`, so correcting
  a submitted motion can change its letter unless the chair has set the order.
- [ ] The parent reference on a node ("in <parent>"), for the feed rows that
  quote what they are about.
- [x] Update, with the interim's edit rule (an author edits a draft, an owner of
  the context edits and arranges anything); reorder and the lock with it.
- [x] The bin: soft delete of a subtree, restore of exactly what went together,
  the listing.
- [x] Move: a subtree to a new parent, every stored path in it rewritten, the
  binned ones too so a later restore lands where its parent now is.
- [ ] Copy and purge. Moving between contexts, which has to change the context
  of a whole subtree, and of the files its nodes point at: a blob is read
  through its own context, so one left behind would stay readable by the old
  group and not by the new.
- [ ] Search, the recent feed, contributions, orphans.

### M4: membership and roster

- [x] What a member row holds: the roster's name (the only label 83 percent of
  rows have, and which the extractor dropped), `hidden` and `accepted`. Whoever
  made a context is realized as an owner of it, or the general secretary loses
  Landsmøde 2026 at cutover. A member row on something that is no context is
  reported, not loaded.
- [x] Paged, filtered member list, for members only, with addresses and hidden
  rows for owners only; the voter count.
- [x] Invite by address, by name and by account, as one invitation or a whole
  roster, skipping whoever is already there; update and remove, with a context
  always keeping an owner who can sign in.
- [x] The caller's invitations: list, accept, and decline or leave by removing
  oneself; claim links for every roster row.
- [ ] Reaching an invited address. Nothing sends mail yet, so an owner hands out
  claim links by hand; binding by the account's confirmed address (M2) would
  make most of that unnecessary.
- [x] Author chips (`setDocumentAuthors`): accounts, or names with no account.

### M5: meetings

- [x] Speaker lists: several per context, a queue served by the chair's
  override, then the kind of contribution, then arrival; a second tap takes no
  second place; a per-turn clock anchored on the server's time.
- [x] Projector state: one row per context (the node on screen, a focus anchor
  that does not follow the screen to another node, comments and feed toggles),
  where the interim packs it into relation names.

Neither is migrated, on purpose: a queue or a screen from a meeting that has
ended is of no use to the next one, and a list is one click to make again.

### M6: voting

A poll is a node (a document of kind `poll`, so it has a URL, moves with its
motion and goes to the bin with it) and a `poll` row of the same id
(`crates/appview/src/poll.rs`, over `crates/ballot-store`).

- [x] Open and close a poll; eligibility freeze; per-poll issuer key. The roster
  is whoever holds voting rights in the context when the poll opens. The issuer
  key is kept sealed under `APPVIEW_SECRET` while the poll is open, so a copy
  of the database forges nothing, and is destroyed at close. Closing seals the
  board under the write lock a cast takes, so no ballot lands after the count.
- [x] Blind token issuance; cast to the board; tally; status. Issuance is signed
  in and records only THAT a voter was served: no token, no time, no order (a
  random rowid and the poll's opening time), since either would line up with
  board positions. A cast carries no session. A lost reply can be asked for
  again, with the same blinded tokens, and mints nothing new.
- [x] Open (non-secret) polls: one named ballot per voter, at their frozen weight.
- [x] A board for every poll in one store. `ballot-store` held a single poll.
- [ ] Publishing the board as atproto records, signed inclusion receipts, and a
  signed close-out digest. All three wait on the owner's custody call
  (`docs/ballot-board-custody.md`). Until then the board is served from here:
  to whoever may see the counts, in token order (board order is the order the
  room voted in), and one entry at a time to a voter who knows their token.
- [ ] Delegation. The roster resolves it (`freeze_at_open`) and issuance and
  counting honour the weights, but nothing writes a delegation: the interim has
  none, and what signs an assignment is undecided.

Decided here without the owner, and cheap to change now:

- **A hidden tally stays hidden after the close**, for everyone but the owners
  of the context, and the board with it. That is the interim's rule, and an
  election is where it is used: who won is announced, not by how much. The cost
  is that only the owners can recount such a poll. A voter can still check that
  their own ballot is there and says what they said.
- **Nobody joins a poll that is already open.** Someone given voting rights
  after it opened votes in the next one. The interim checked at the moment of
  casting.
- **A poll with nobody to vote in it does not open** (`NoVoters`), which is
  what a chair sees who forgot to hand out voting rights.

### M7: live updates

- [x] `/ws` topic protocol (`crates/appview/src/live.rs`): a listener
  authenticates with its first frame, subscribes to `context:<id>`,
  `user:<did>` or `public`, and is granted a topic only if it may read it. A
  change names what changed and never what it changed to, so a client refetches
  through the gated reads. The relay it replaces sent every delta to every
  connection, unauthenticated.
- [x] Every write path built so far publishes its change.
- [ ] Re-checking a standing subscription when its listener's membership ends.
  Until then a removed member keeps hearing THAT their old context changed, and
  nothing of what.

### M8: blobs and the carried-over endpoints

- [x] Upload and download with read authorization; signed links for third-party
  viewers (`crates/appview/src/blob.rs`). A file belongs to a context and is
  read by whoever may read that context. The bytes sit on disk under their
  SHA-256, streamed both ways, with ranges for a player. Only types that cannot
  run are shown in place; a page, an SVG or any XML is a download.
- [ ] A ceiling on what one member or one context may store. A single file is
  capped; their sum is not, as it is not in the interim.
- [ ] Copying the interim's files across, each under its old storage id so that
  `data.fileId` on a node keeps working (a cutover step, listed here because
  the blob table is its target).

#### The interim sidecar's other endpoints

Carried over one at a time. Handle typeahead is not among them: the browser
asks Bluesky's public API itself. The steps:

- [x] Roster parsing (`parseRoster`): the office's .xlsx into the rows
  `inviteMembers` takes.
- [x] The log proxy (`POST /log`), with symbolication of the wasm frames passing
  through. It takes no session, so what it remembers about builds is bounded:
  made-up build hashes cannot push a real build's symbols out or grow without
  limit.
- [x] Push: subscribe, unsubscribe, notify, reply (`crates/appview/src/push.rs`).
  The encryption and VAPID are the interim's, against the RFC 8291 vector. A
  subscription is a device's and is keyed by DID, where the interim keyed by
  email. A push endpoint is a URL a client hands over, so it goes out through
  the guarded client, which also closes the DNS-rebinding hole the interim
  accepted. A notification may link only into the app. The cutover must carry
  the interim's VAPID key pair over: the frontend has the public half compiled
  in, and every browser's subscription is bound to it.
- [x] Feedback and crash reports (`crates/appview/src/feedback.rs`). The interim
  files them as nodes under the root node; here they have a table. A crash seen
  again is one row with a count and its people, under the interim's digest, so
  a crash known before the cutover keeps its row once migrated. Reports need no
  session, so those without one share a budget of ten a minute. Not migrated
  yet: the interim's existing reports (M9).
- [ ] Metafile rendering (EMF/WMF figures in Word and PowerPoint files to PNG).
- [ ] Posting to a member's own PDS.

### M9: frontend swap, migration, cutover rehearsal

- [ ] An AppView client behind `src/model.rs`, replacing `src/graphql/*` and
  `src/nhost.rs`; the session module on AppView tokens.
- [ ] Extractor and loader cover every migrated kind; field-gap report empty.
- [ ] Staging rehearsal of the runbook; browser suite green against the AppView.

## Working rules

- `cargo test --workspace` in `crates/` is the check. There is no CI job for it
  (`crates/README.md` says why), so run it before every commit.
- Anything that stands in for real behaviour says so in its doc comment and is
  listed under "Findings" until it is replaced.
- The frontend is not touched before M9 except to add to the seam, so the live
  app keeps shipping from `main` throughout.
