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
- `domain-types`: the canonical backend serde types for the content and
  membership half (user, context, document, post, member, comment). Voting
  entities excluded until the ballot spec and voting SQL settle.
- `migration-extractor`: the read-only interim-to-domain-types mapping with a
  field-gap report. Pure and hermetic (tested on synthetic fixtures; a live
  dump is an owner-approved separate step). The `extract` binary reads a
  dumped snapshot and emits fixtures + `report.json`.

## Checks

`cargo test --workspace` in this directory is the check, run locally and by
the flake-check executor of record. There is deliberately NO tangled microVM
CI job for it: the ~2 GiB microVM cannot compile a Rust crypto dependency
tree (the same constraint that scopes `.tangled/workflows.ncl` to the
formatting check; see workflows.ncl:23-37).
