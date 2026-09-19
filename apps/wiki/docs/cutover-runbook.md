# Big-bang cutover runbook

The decided migration strategy is a single big-bang cutover
(`docs/atproto-open-decisions.md`): freeze the interim app, move the content and
membership rows into a staging Turso db, verify, then flip the frontend at the
env seams. This is the ordered checklist, the go/no-go verification gates, and
the rollback. It is paper until run; the pieces it assembles are built and
tested.

## Pieces (all EXISTING and tested unless marked)

- **Read-only dump**: `scripts/dump-interim-snapshot.nu`, which queries the
  interim Hasura surface into the `{ nodes, members, users, permissions }`
  snapshot (admin secret from the environment, never committed). `users` holds
  each account's address and whether the interim verified it; `permissions`
  holds the rows that open a context to everyone.
- **Extractor**: `crates/migration-extractor`, which maps the snapshot into the
  canonical domain types and emits a `FieldGapReport`; `extract` binary writes
  `extraction.json` + `report.json`.
- **Generated schema**: `crates/domain-types::DDL` (re-exported as
  `wiki_schema::ENTITY_SCHEMA`), validated on rusqlite + turso by
  `crates/schema/tests/roundtrip.rs`. Every entity table carries
  `legacy_id TEXT UNIQUE` for idempotent load.
- **Load**: `appview import <extraction.json>` (`crates/appview/src/import.rs`).
  It runs `crates/migration-loader` for the entity tables (FK order, idempotent
  by primary key), then loads what lives in the AppView's own tables: what each
  poll came to, canvases and their cells, reports, and the address each interim
  account is recognized by. One transaction, so a load that fails leaves the
  datastore as it found it. It refuses a datastore somebody has signed in to,
  where loading again would bring back whatever was deleted since.
- **Accounts**: `crates/appview/src/legacy.rs`. See "Who people are afterwards".
- **Env seams (the flip)**: `WIKI_GRAPHQL_URL` (`src/nhost.rs:13`) and
  `WIKI_BACKEND_URL` (`src/backend_api.rs:18`), both `option_env!` compile-time
  overrides; the file-blob path flips at the single `backend_api::file_url` seam.
- **Ballot service** (parallel track, not on the content cutover path):
  `crates/ballot-store` (durable board + private eligibility/issuance). The
  interim has only `vote/poll` + anonymous `vote/vote`; historical secret
  ballots are UNMIGRATABLE by design and are reported, not carried.
- **Deploy target**: `crates/appview/default.nix` (the `wiki-appview`
  `buildRustPackage`) + `crates/appview/nixos-module.nix` (the stateful systemd
  unit with a persistent `StateDirectory` for the Turso file, restart-on-failure,
  and `/healthz`). Acceptance (`nixos-rebuild build-vm` behind Ferron, restart
  soak) is the operator step.
- **The AppView itself** answers every data call the frontend makes
  (`docs/appview-api-coverage.md`). What is NOT BUILT is the other end of the
  flip: the frontend's own data layer still speaks GraphQL to Hasura
  (`docs/appview-roadmap.md`, M9), so there is nothing to flip to yet, and this
  runbook is rehearsed against staging first.

## Ordered checklist

1. **Announce + freeze the interim app.** Put the interim app in read-only mode
   (no new nodes/members/votes) so the dump is a consistent point-in-time. Record
   the freeze timestamp.
2. **Read-only dump.** `HASURA_URL=… HASURA_ADMIN_SECRET=… nu
   scripts/dump-interim-snapshot.nu | save --force snapshot.json`. Confirm the
   printed row counts (nodes / members / users) match the census; if Hasura
   capped a table, add pagination and re-dump (a silent cap loses data).
3. **Extract.** `cargo run -p migration-extractor -- snapshot.json` → produces
   `extraction.json` + `report.json`. This step is PII-bearing; run it in the
   owner-approved environment, not CI.
4. **Load.** With the service stopped and `APPVIEW_DB` naming a datastore file
   that does not exist yet: `appview import extraction.json`. It creates the
   schema, loads everything or nothing, and prints what it loaded. Set the
   printed counts against those `extract` printed. The search index is built
   when the service next starts.
5. **Copy the files.** `HASURA_URL=… NHOST_STORAGE_URL=… HASURA_ADMIN_SECRET=…
   nu scripts/dump-interim-files.nu files` downloads every file in NHost storage
   into `files/`, with what storage says of each in `files/manifest.json`. Then
   `appview import-files extraction.json files` files each in the blob store
   under its OLD storage id, under the context of what points at it, so nothing
   that points at a file is rewritten. Both can be run again after a failure:
   what is already there is passed over. `import-files` exits non-zero and lists
   every file it did not copy (never downloaded, or short of the size storage
   reported), and lists without copying every file that nothing points at.
   On the NixOS host, steps 4 and 5 are one unit. The service runs as a systemd
   DynamicUser with a private state directory, so `appview import` cannot be run
   against it by hand. Set `services.wiki-appview.import.extraction` and
   `.files` to where the two are ON THE HOST (strings, never Nix paths: a path
   would copy every member's address into the world-readable store), rebuild,
   and `systemctl start wiki-appview-import`. It stops the service, loads, files
   the files, and leaves the service stopped for the gates below;
   `journalctl -u wiki-appview-import` has what it printed. The VM test
   (`crates/appview/nixos-test.nix`) runs exactly this.
6. **Verification gates** (below): go/no-go. Any red gate stops the cutover.
7. **Flip.** Build the frontend with `WIKI_GRAPHQL_URL` / `WIKI_BACKEND_URL`
   pointed at the AppView, and change the `backend_api::file_url` body to the
   AppView blob path (`/blob/<id>`). Deploy the frontend.
8. **Smoke test** the live app against the AppView: load a group, open a
   document with multiple authors, post a comment, fetch a file.
9. **Unfreeze** (or, if a gate or smoke test fails, **roll back**).

## Verification gates (go/no-go)

All must be green before the flip:

- **Row counts.** Per-table counts in staging Turso equal the expected mapped
  counts from the dump (contexts = group+event+site nodes and the one home;
  documents = content nodes, polls and canvases;
  members = roster rows; users = interim users; comments = `vote/comment` nodes).
- **`legacy_id` coverage.** Every loaded entity row has a non-NULL `legacy_id`,
  and the count of distinct `legacy_id`s per table equals the source uuid count
  for that table (no row silently dropped or merged).
- **Field-gap report is clean.** `report.json`'s `unmapped_source`,
  `unmapped_mimes`, and `unfilled_required` are all empty. A non-empty
  `unfilled_required` means a NOT NULL or a meaning was dropped; a non-empty
  `unmapped_*` means a source field or mime had no home and must be triaged
  (mapping rule, interim junk sweep, or schema amendment) before flipping.
  `left_behind` is not a gap and need not be empty: it counts what is dropped on
  purpose (deleted reactions and reports, spent claim links, speaker lists,
  addresses nobody verified). Read it, and see that each count is one you
  expected.
- **People can get back in.** The count under `users.email, not verified` is how
  many account holders cannot be recognized by address and will need a claim
  link for each seat. If it is most of them, the interim never verified
  addresses, and that is a decision to take before the flip, not after.
- **Membership dedup landed.** The census's ~1962 distinct invite emails behind
  ~17655 roster rows collapse under the `member_pending` partial unique
  (`context_id, email` where `user_did IS NULL`): the count of pending-invite
  rows equals the distinct `(context, normalized-email)` pairs, with no duplicate
  pending invite per context.
- **Every node is where its URL says.** Each loaded context and document has a
  non-empty `path`, no two live rows share one across the two tables, and a
  sample of the interim's most-visited paths resolves through `getNode` to the
  row with the same `legacy_id`. A node the extractor re-rooted appears in
  `report.json` under `nodes.parentId -> <kind>`, and that list is triaged.
- **Every file came across.** Each `data.fileId` on a loaded document names a
  `blob` row, whose `size` equals what NHost reported and whose bytes on disk
  hash to its `sha256`. A sample opens through `/blob/<id>` as a member of its
  context, and is refused to a stranger.
- **Authorship preserved.** `document_author` row count ≥ document count and no
  document with a source author chip has zero author rows (the free-text authors,
  about 42 percent, survived rather than being dropped by the old scalar
  `author_did`).

## Who runs the site afterwards

The interim's root comes across as the home, with its owners as the owners of
the home: they start what sits at the top, and the reports are theirs. If none
of them can sign in (no verified address, no claim link anyone can hand them,
since a link is an owner's to give), set `APPVIEW_SITE_OWNER` (the NixOS
module's `siteOwner`) to a DID you hold and restart: it is seated as an owner of
the home, and hands out the claim links from there.

## Who people are afterwards

The interim knew a person by an account id; the AppView knows them by their
DID. An interim account comes across under its old id, which no login produces:
it keeps its seats, its name and what it wrote, and cannot sign in.

- **By address.** Whoever signs in with the address an account was registered
  under takes all of it over, in one transaction: seats (the better of each
  grant where they hold two in one context), authorship, comments, reactions,
  cells and reports. Both ends have to vouch for the address. The extractor
  carries only addresses the interim had VERIFIED, and the AppView believes
  only a PDS configured as trusted (`APPVIEW_TRUSTED_EMAIL_PDS`) that says the
  address is confirmed. The same sign-in hands over the invitations sent to
  that address, as before.
- **By claim link**, for everyone else. An owner asks for a member's link
  (`getMemberClaimLink`) and hands it over. It gives the seat in that owner's
  context and nothing else the old account holds, because a seat in their own
  context is all an owner has to give.
- **Spent claim links stay spent.** The interim keeps a token on its row after
  the seat is taken. Those are not carried, or an old invitation in somebody's
  inbox would open a seat again; a seat that needs a link gets a new one.

## Rollback

The interim app is untouched by the dump (read-only) and the load targets a
SEPARATE staging Turso db, so rollback is: revert the frontend to the build
pointed at NHost/Hasura (drop the `WIKI_*_URL` overrides and the `file_url`
change), redeploy, and unfreeze the interim app. No interim data was mutated, so
there is nothing to restore. Keep the interim project alive until the AppView has
run clean for an agreed soak window.

## Assisted-DID branch (placeholder)

If the onboarding walkthrough (`docs/onboarding-walkthrough.md`, a pending
owner-run step) finds that members cannot self-obtain and link a DID, an
org-assisted DID-provisioning step (batch account creation, org-run PDS, or an
in-app signup wizard) enters BETWEEN steps 6 and 7 here. Until that walkthrough
runs, this branch stays a placeholder; the window to decide it closes when the
interim app retires at cutover.
