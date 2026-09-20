# Big-bang cutover runbook

The decided migration strategy is a single big-bang cutover
(`docs/atproto-open-decisions.md`): freeze the interim app, move the content and
membership rows into a staging Turso db, verify, then flip the frontend at the
env seams. This is the ordered checklist, the go/no-go verification gates, and
the rollback. Steps 2 to 6 and the smoke test have been rehearsed on a dump of
production (`docs/cutover-rehearsal.md`), and are one command to rehearse
again: `nu scripts/rehearse-cutover.nu <dir>`.

## Pieces (all EXISTING and tested unless marked)

- **Read-only dump**: `scripts/dump-interim-snapshot.nu`, which queries the
  interim Hasura surface into the `{ nodes, members, users, permissions }`
  snapshot (admin secret from the environment, never committed). `users` holds
  each account's address and whether the interim verified it; `permissions`
  holds the rows that open a context to everyone. Each table is read a page at
  a time by id with a pause between, since the interim is one small shared
  project, and held to its own count, so a capped or torn read fails there.
- **Extractor**: `crates/migration-extractor`, which maps the snapshot into the
  canonical domain types and emits a `FieldGapReport`; `extract` binary writes
  `extraction.json` + `report.json`. The report keeps gaps (`unmapped_source`,
  `unmapped_mimes`, `unfilled_required`) apart from decisions (`left_behind`,
  `reshaped`), so that the first can be held to empty.
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
- **The gates**: `appview verify <extraction.json> [<files dir>]`
  (`crates/appview/src/verify.rs`), which asks the verification gates below of
  the loaded datastore and exits non-zero on a red one.
- **Accounts**: `crates/appview/src/legacy.rs`. See "Who people are afterwards".
- **The flip**: the frontend built with the `appview` cargo feature and
  `WIKI_APPVIEW_URL` naming the AppView (`src/appview/mod.rs`). The build
  without the feature is the interim's, and is the rollback.
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
  (`docs/appview-api-coverage.md`), and the frontend has a data layer that
  speaks to it (`docs/appview-roadmap.md`, M9).
- **The rehearsal**: `scripts/rehearse-cutover.nu <dir>`, from a dump already
  in `<dir>`: extract, load, file the files, the gates, then every carried
  account returns at once (`appview-dev --db ... --everyone-returns`, a dev
  tool that is never deployed) and a smoke test over HTTP as nobody, as a
  returned member and as a stranger. `scripts/rehearse-cutover-browser.nu
  <dir>` then walks real pages of every kind in a real browser.

## Ordered checklist

1. **Announce + freeze the interim app.** Put the interim app in read-only mode
   (no new nodes/members/votes) so the dump is a consistent point-in-time. Record
   the freeze timestamp.
2. **Read-only dump.** `HASURA_URL=… HASURA_ADMIN_SECRET=… nu
   scripts/dump-interim-snapshot.nu | save --force snapshot.json`. It holds
   each table to its own count and fails on a difference, which with the
   interim frozen means a capped read. Keep the file private: it holds every
   member's address.
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
   the files, asks the gates, and leaves the service stopped;
   `journalctl -u wiki-appview-import` has what it printed, and a red gate
   fails the unit. The VM test (`crates/appview/nixos-test.nix`) runs exactly
   this.
6. **Verification gates** (below): go/no-go. `appview verify extraction.json
   files` prints each, green or red, and what it counted. Any red gate stops
   the cutover. Then read `report.json`: what was left behind and what was
   carried in another shape are decisions, and each count is to be one you
   expected.
7. **Flip.** Build the frontend with the `appview` feature and
   `WIKI_APPVIEW_URL` naming the AppView. Deploy the frontend.
8. **Smoke test** the live app against the AppView: load a group, open a
   document with multiple authors, post a comment, fetch a file.
9. **Unfreeze** (or, if a gate or smoke test fails, **roll back**).

## Verification gates (go/no-go)

All must be green before the flip. `appview verify` asks each of them, under
the names in bold:

- **Everything arrived.** Every row of the extraction is in its table, by id:
  users, contexts, documents, members, comments, reactions, polls, canvases,
  reports, and the accounts to be recognized by address. What the datastore
  has besides (a configured site owner) is counted and is not red.
- **Known by the id they had.** Every loaded entity row carries its
  `legacy_id`, which is what makes a second load add nothing.
- **Nothing without a home.** `report.json`'s `unmapped_source`,
  `unmapped_mimes`, and `unfilled_required` are all empty. A non-empty
  `unfilled_required` means a NOT NULL or a meaning was dropped; a non-empty
  `unmapped_*` means a source field or mime had no home and must be triaged
  (mapping rule, interim junk sweep, or schema amendment) before flipping.
  `left_behind` and `reshaped` are not gaps and need not be empty. The first
  counts what is dropped on purpose: deleted reactions and reports, spent claim
  links, speaker lists, addresses nobody verified, a legacy one-off page with
  nothing in it, and ORPHANS, the rows under a parent that was deleted outright
  before the interim had a bin, which no URL reaches there and which would
  come back at the top of their group if carried. The second counts what is
  carried in another shape: a context's owner as an owner membership, a poll
  open at the dump as a closed one (close the polls before the freeze, or
  accept that), a seat or an author chip of a DELETED account as a seat waiting
  for its address and an author by name. Read both, and see that each count is
  one you expected.
- **People can get back in.** How many accounts are recognized by address; the
  rest need a claim link for each seat. Red below half: then the interim never
  verified addresses, and that is a decision to take before the flip, not
  after.
- **One waiting seat to an address.** No address waits twice in one context
  (the `member_pending` partial unique, `context_id, email` where `user_did IS
  NULL`), so whoever takes a seat leaves none behind them.
- **Every node is where its URL says.** Each live context and document has a
  path, no two share one across the two tables, and each path is its parent's
  path and its own slug, so the tree and the URLs agree. The rehearsal's smoke
  test opens a real page by its old URL; a sample of the most-visited ones by
  hand does no harm.
- **Every file came across.** Each file something points at has a `blob` row,
  of the size NHost reported, whose bytes on disk hash to its `sha256`. A file
  the interim's own storage no longer had was lost before the move: counted,
  not red. The rehearsal's smoke test opens one through `/blob/<id>` as a
  member of its context, and is refused as a stranger.
- **Authorship preserved.** Every document has the authors its chips named, by
  account, by name or as a group (the free-text authors, about 42 percent,
  survived rather than being dropped by the old scalar `author_did`).

## Mail, and the ballot board

Neither is needed for the flip, and both are settings of the service
(`crates/appview/nixos-module.nix`), with their secrets in `secretsFile`:

- **Mail.** `mailFrom` and `APPVIEW_SMTP_URL`: an address put on a roster is
  mailed the link to its seat. The account at a mail provider is to be made
  first. Without them an owner hands claim links out, as before. After the
  flip, `sendInvitation` mails a carried seat its link for the first time.
- **The board account.** `board.pds`, `board.identifier` and
  `APPVIEW_BOARD_PASSWORD` (an app password): an atproto account kept for
  nothing but ballot boards, whose keys the organization holds. With it, a
  secret poll opened as public has its board published for anyone to read.
  Without it nothing is published; receipts and close-outs are signed all the
  same.
- **The mirror, before the first binding public vote.** Someone who is not the
  organization runs `board-mirror follow --pds <the board's PDS> --repo <the
  board account's DID> --dir <dir> --every 60`, and after a vote
  `board-mirror check --dir <dir> --key <the custody key>`, the key being what
  `getBoardKey` answers and what was read out to the assembly. Until someone
  does, a ballot unpublished after the fact is nobody's to notice, and the
  claim that censorship is provable is a claim about software nobody ran.

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
SEPARATE staging Turso db, so rollback is: deploy the frontend built without
the `appview` feature, which is the interim's, and unfreeze the interim app. No
interim data was mutated, so there is nothing to restore. Keep the interim project alive until the AppView has
run clean for an agreed soak window.

## Assisted-DID branch (placeholder)

If the onboarding walkthrough (`docs/onboarding-walkthrough.md`, a pending
owner-run step) finds that members cannot self-obtain and link a DID, an
org-assisted DID-provisioning step (batch account creation, org-run PDS, or an
in-app signup wizard) enters BETWEEN steps 6 and 7 here. Until that walkthrough
runs, this branch stays a placeholder; the window to decide it closes when the
interim app retires at cutover.
