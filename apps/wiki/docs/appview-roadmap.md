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
- **Writes trust the caller.** `xrpc::caller_did` reads the DID straight out of
  the `Authorization` header. It is labelled a placeholder; M1 replaces it.
- **Reads are ungated.** Every read serves private content to anyone. M2 gates
  them on visibility and membership.
- **A document cannot be addressed by path.** The entity schema gives `context`
  a slug and `document` none, while the frontend routes every node by its key
  path (`/a/b/c`). Without a slug on documents every existing link breaks at
  cutover. The schema also has no place for `index` (manual order), `mutable`
  (published/locked), `attachable` (the folder lock), the non-content `data`
  keys (file id and type, cover image), `updated_at`, or a deleted marker (the
  bin), and its `context.kind` CHECK rejects `wiki/site`. M3 reconciles these.
- **Speaker lists were classed as ephemeral and dropped by the extractor**, but
  the speak app persists them and an assembly reads them back. M5 gives them
  tables; M9 migrates them or records why not.

## Milestones

Each lands with tests, committed on its own. A box is ticked only when the code
is merged and its tests pass.

### M0: build hygiene

- [x] One rustls `reqwest` client for every outbound call; OpenSSL out of the
  dependency graph; `cargo test --workspace` green in the devshell.

### M1: identity and sessions (replaces NHost auth)

- [ ] `session` table; opaque bearer tokens, stored hashed, with expiry.
- [ ] `GET /login` (handle to authorization URL), `/callback` mints a session and
  returns to an allow-listed frontend origin.
- [ ] `getSession` and `deleteSession`; `Caller` and `MaybeCaller` extractors;
  the placeholder `caller_did` deleted.
- [ ] CORS for the configured frontend origins.
- [ ] Production client metadata (`/client-metadata.json`) when a public URL is
  configured; the loopback profile otherwise.
- [ ] Profile hydration on login (handle, display name, avatar into `user`).

### M2: authorization core

- [ ] DID-keyed `is_active_member` and `is_active_owner`, one module, one role
  enum (the predicates the kickoff plan deferred).
- [ ] Reads gated: public context, or active member, or author.
- [ ] Writes gated: membership to create, authorship or ownership to change.
- [ ] Invite binding: claim token, then confirmed-email match, on login.

### M3: the node tree the frontend routes by

- [ ] Schema reconcile: document slug, index, mutable, attachable, data,
  updated_at, deleted_at; `site` context kind. Generated DDL, both engines.
- [ ] Path resolution across contexts and documents; crumbs; path from id.
- [ ] Node read model in the `src/model.rs` shape (node with children, members,
  permission flags).
- [ ] Create, update, delete, move, reorder, copy; the bin (soft delete,
  restore, purge).
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
