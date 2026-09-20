# crates/ (the rewrite workspace)

Crates that BECOME the atproto AppView. A separate Cargo workspace with its
own lockfile, deliberately NOT merged into the frontend manifest (whose
cargoLock is Nix-FOD-pinned; a root-manifest merge would destabilize the
vendoring hashes) or the backend container manifest. The transferable backend
modules (push, dpop, pkce, statecookie, oauth, util) migrate INTO this
workspace when the rewrite starts.

## Crates

- `ballot-spec`: the executable specification of the E2E-verifiable ballot
  scheme (RFC 9474 blind signatures, unit tokens, per-poll issuer keys, the
  abstract bulletin board) with its property-test conformance suite. See its
  `DECISIONS.md` for the semantics it pins.
- `schema`: the entity-subset target DDL as an executable `schema.sql`,
  round-trip tested on real SQLite AND the turso crate (dialect findings
  recorded in the tests).
- `dagcbor-spike`: known-answer and round-trip vectors for the DAG-CBOR +
  CIDv1 encode path (the exact path that becomes the AppView publish seam).
- `durability-harness`: the ballot-core kill -9 crash harness (BEGIN
  IMMEDIATE dedup + ballot transaction) on both engines, plus the
  Turso-to-stock-SQLite file-format bridge assertion. Verdict recorded in
  `docs/atproto-stack-decisions.md` (Gate measurement).
- `oauth-spike`: the mandated thin wrapper over `atrium-oauth`, proving
  PDS-agnostic server-side login (handle to DID to PDS resolution, PAR, DPoP,
  PKCE) against independent non-Bluesky PDSes. Its network test is `#[ignore]`
  (run with `--ignored`); findings in `oauth-spike/FINDINGS.md`.
- `domain-types`: the canonical backend serde types (user, context, document,
  post, member, comment, reaction), and the shapes a migrated poll, canvas and
  report travel in.
- `migration-extractor`: the read-only interim-to-domain-types mapping with a
  field-gap report, which keeps gaps apart from decisions: what is left behind
  on purpose (orphans under a row that was deleted outright, among others) and
  what is carried in another shape. Pure and hermetic (tested on synthetic
  fixtures; a live dump is an owner-approved separate step). The `extract`
  binary reads a dumped snapshot and writes `extraction.json` + `report.json`.
- `migration-loader`: writes an extraction's entity rows into a Turso db,
  parents first and idempotently, with foreign keys enforced so a dump that
  points at a row it does not contain fails at the rehearsal, and by name: a
  parent that is nowhere, an account nothing loads. A library: the command that
  runs it is `appview import`, which also loads what lives in the AppView's own
  tables; `appview verify` then asks the cutover's gates of the result.
- `ballot-store`: the durable half of the ballot scheme: every poll's public
  board in one store, with its kill-9-proven atomic cast and a seal that ends
  it; the private roster DDL; and the off-node replica log.
- `lexgen` and `appview-client`: a typed client for the AppView, GENERATED from
  the lexicons (`cargo run -p lexgen`), and the contract tests that run it
  against the real router over HTTP with unknown fields refused. That holds
  the three to one another: what the server sends and no lexicon names fails
  there, as does what a lexicon promises and the server does not send, and a
  lexicon added without a call in the tests. The frontend's data layer is
  written over this client (`apps/wiki/src/appview/`). It also reads the
  server's clock off every answer, for a frontend on a device whose own is off.
- `appview-dev`: a DEV server, in no package and on no host: an in-memory
  AppView with a home, and a session ready for every DID named on its command
  line, since signing in takes a PDS and a browser. `did:plc:carol` is Carol,
  `carol.test`. It is what the frontend's data layer is tested against
  (`apps/wiki/src/appview/live.rs`), what its browser run is driven against
  (`apps/wiki/scripts/test-browser-appview.nu`), and what a frontend on a
  laptop can talk to:
  `APPVIEW_FRONTEND_ORIGINS=http://127.0.0.1:8080 appview-dev --port 8136 did:plc:me`,
  then `WIKI_APPVIEW_URL=http://127.0.0.1:8136 dx serve --features appview`.
  It also rehearses a cutover (`apps/wiki/scripts/rehearse-cutover.nu`):
  `--db FILE` serves a datastore `appview import` filled, `did:plc:me=<address>`
  signs in as though a PDS had confirmed that address, which is how a person
  takes their old account over, and `--everyone-returns` does so for every
  carried account and says how it went.
- `board-mirror`: for someone who is NOT the organization. It keeps its own
  append-only copy of a published ballot board from the board account's public
  repo, raises an alarm when a record it has seen is gone or rewritten, and
  recounts every closed poll against its signed close-out. With `--space` it
  does the same of a board in a group's atproto space, as a member, by an app
  password of their own account. Packaged as `wiki-board-mirror`.
- `atproto-spaces`: atproto spaces (proposal 0016, non-public records) as far as
  the AppView needs them: tokens and DPoP, the credential flow, the record and
  sync calls, a repo's set hash and signed commit, keeping a copy of a repo
  honest, and checking the service tokens a PDS calls back with. Its ignored
  test runs against the real alpha PDS in a container
  (`apps/wiki/scripts/test-spaces.nu`), which is how a breaking change of the
  alpha's shows. The AppView's use of it is `appview/src/spaces.rs`, off unless
  configured: the wiki mirrored into a space per context, and the AppView as
  the managing app the organization's PDS asks. That has an ignored test of its
  own in the same script.
- `wiki-records`: a context's atproto space as Rust types (the lexicons
  `contextProfile`, `node`, `comment` and `reaction` under
  `lexicons/wiki/radikal/`), and the mapping between those records and the rows
  the AppView keeps. Tested to lose nothing either way and to say what the
  lexicons say: a write through records and a rebuild of the index from records
  both stand on it (`docs/atproto-spaces-redesign.md`, S1). Where a node is
  follows from the records too (`Found`), and what no record carries is named
  by a type (`Kept`).
- `spaces-spike`: can a redesign of the wiki on atproto spaces (proposal 0016,
  non-public records) stand on what the alpha does, and can a syncer's checks
  be made from Rust? The set hash (LtHash over BLAKE3) and the deniable signed
  commit, tested over a repo a real spaces PDS served; what else the PDS was
  asked is in `spaces-spike/FINDINGS.md`, the design in
  `docs/atproto-spaces-redesign.md`.
- `fake-pds`: a PDS for tests, in no package: one account, its records in
  memory, the calls the board's publisher and the mirror make of a real one,
  and the space calls the AppView makes, answered as the alpha was found to
  answer them (`spaces-spike/FINDINGS.md`).
- `fake-plc`: a `did:plc` directory for a sign-in rehearsed on one machine, in
  no package. A REAL PDS registers its accounts with a directory, and the
  public one is no place for made-up accounts; this one keeps what it is told
  and checks none of it (`apps/wiki/scripts/test-real-login.nu`, with
  `APPVIEW_PLC_URL` pointing the AppView at it).
- `appview`: the backend itself. `docs/appview-roadmap.md` says what it does
  today and what is left.

## Checks

`cargo test --workspace` in this directory is the check, run locally and by
the flake-check executor of record. There is deliberately NO tangled microVM
CI job for it: the ~2 GiB microVM cannot compile a Rust crypto dependency
tree (the same constraint that scopes `.tangled/workflows.ncl` to the
formatting check; see workflows.ncl:23-37).
