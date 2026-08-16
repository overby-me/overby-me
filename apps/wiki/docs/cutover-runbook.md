# Big-bang cutover runbook

The decided migration strategy is a single big-bang cutover
(`docs/atproto-open-decisions.md`): freeze the interim app, move the content and
membership rows into a staging Turso db, verify, then flip the frontend at the
env seams. This is the ordered checklist, the go/no-go verification gates, and
the rollback. It is paper until run; the pieces it assembles are built and
tested.

## Pieces (all EXISTING and tested unless marked)

- **Read-only dump**: `scripts/dump-interim-snapshot.nu` — queries the interim
  Hasura surface into the `{ nodes, members, users }` snapshot (admin secret
  from the environment, never committed).
- **Extractor**: `crates/migration-extractor` — maps the snapshot into the
  canonical domain types + emits a `FieldGapReport`; `extract` binary writes
  `extraction.json` + `report.json`.
- **Generated schema**: `crates/domain-types::DDL` (re-exported as
  `wiki_schema::ENTITY_SCHEMA`), validated on rusqlite + turso by
  `crates/schema/tests/roundtrip.rs`. Every entity table carries
  `legacy_id TEXT UNIQUE` for idempotent load.
- **Loader**: `crates/migration-loader` — writes an `Extraction` into a staging
  Turso db under `ENTITY_SCHEMA`, in FK order and idempotently by primary key +
  `legacy_id` (a re-run is a no-op).
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
- **TO-BE-BUILT**: the AppView read/write handlers behind the env seams (the
  Store seam port in `crates/appview` is started, the read/write XRPC handlers are
  not). Until those serve, the flip target does not answer queries — this runbook
  is rehearsed against staging first.

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
4. **Load to staging Turso.** Apply `ENTITY_SCHEMA` to a fresh staging Turso db,
   then run the loader over `extraction.json`. The load is idempotent, so a
   partial run can be safely re-run.
5. **Verification gates** (below) — go/no-go. Any red gate stops the cutover.
6. **Flip.** Build the frontend with `WIKI_GRAPHQL_URL` / `WIKI_BACKEND_URL`
   pointed at the AppView, and change the `backend_api::file_url` body to the
   AppView blob path. Deploy the frontend.
7. **Smoke test** the live app against the AppView: load a group, open a
   document with multiple authors, post a comment, fetch a file.
8. **Unfreeze** (or, if a gate or smoke test fails, **roll back**).

## Verification gates (go/no-go)

All must be green before the flip:

- **Row counts.** Per-table counts in staging Turso equal the expected mapped
  counts from the dump (contexts = group+event nodes; documents = content nodes;
  members = roster rows; users = interim users; comments = `vote/comment` nodes).
- **`legacy_id` coverage.** Every loaded entity row has a non-NULL `legacy_id`,
  and the count of distinct `legacy_id`s per table equals the source uuid count
  for that table (no row silently dropped or merged).
- **Field-gap report is clean.** `report.json`'s `unmapped_source`,
  `unmapped_mimes`, and `unfilled_required` are all empty — a non-empty
  `unfilled_required` means a NOT NULL / meaning was dropped; a non-empty
  `unmapped_*` means a source field or mime had no home and must be triaged
  (mapping rule, interim junk sweep, or schema amendment) before flipping.
- **Membership dedup landed.** The census's ~1962 distinct invite emails behind
  ~17655 roster rows collapse under the `member_pending` partial unique
  (`context_id, email` where `user_did IS NULL`): the count of pending-invite
  rows equals the distinct `(context, normalized-email)` pairs, with no duplicate
  pending invite per context.
- **Authorship preserved.** `document_author` row count ≥ document count and no
  document with a source author chip has zero author rows (the free-text authors
  — ~42% — survived rather than being dropped by the old scalar `author_did`).

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
in-app signup wizard) enters BETWEEN steps 5 and 6 here. Until that walkthrough
runs, this branch stays a placeholder; the window to decide it closes when the
interim app retires at cutover.
