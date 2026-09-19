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
  extracted as private. Extracted polls are never loaded. M3 and M9.
- **There are no schema migrations.** The entity tables are plain
  `CREATE TABLE`, so a datastore file made by an older binary keeps its old
  columns. Until migrations exist the file records its schema version and a
  binary refuses to start on another one, rather than failing a query at a
  time. Pre-cutover that costs nothing, since the view is rebuilt from the
  migration pipeline; after it, migrations become real work (M9).
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
- [ ] Authorship or ownership to CHANGE a node: arrives with update and delete
  in M3, which is where those procedures are.

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
- Kinds that are nodes in the frontend but have their own state (polls, speaker
  lists, canvases) become `document` kinds with a side table keyed by the same
  id, when their milestone arrives.
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
- [ ] The rest of the `src/model.rs` node shape: the members and author chips
  on a node, the parent reference, `get_index` (the A/B/C of a policy).
- [x] Update, with the interim's edit rule (an author edits a draft, an owner of
  the context edits and arranges anything); reorder and the lock with it.
- [x] The bin: soft delete of a subtree, restore of exactly what went together,
  the listing.
- [ ] Move, copy, purge.
- [ ] Search, the recent feed, contributions, orphans.

### M4: membership and roster

- [ ] Paged, filtered member list; counts.
- [ ] Invite by email and by user, bulk roster import, update, remove.
- [ ] Invitations for the caller: list, accept, decline; claim links.
- [ ] Author chips (`set_node_authors`).

### M5: meetings

- [ ] Speaker lists and entries.
- [ ] Projector state (`active`, screen comments, screen feed, focus).

### M6: voting

- [ ] Open and close a poll; eligibility freeze; per-poll issuer key.
- [ ] Blind token issuance; cast to the board; tally; status.
- [ ] Open (non-secret) polls.

### M7: live updates

- [ ] `/ws` topic protocol: subscribe by scope key and discriminator
  (`docs/use-live-topic-inventory.md`), authorized per topic.
- [ ] Every write path publishes its delta.

### M8: blobs and the carried-over endpoints

- [ ] Upload and download with read authorization; signed links for third-party
  viewers.
- [ ] Push (subscribe, notify, reply), feedback, log proxy, symbolication,
  metafile rendering, roster parsing, handle typeahead.

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
