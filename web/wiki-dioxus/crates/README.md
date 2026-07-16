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

## Checks

`cargo test --workspace` in this directory is the check, run locally and by
the flake-check executor of record. There is deliberately NO tangled microVM
CI job for it: the ~2 GiB microVM cannot compile a Rust crypto dependency
tree (the same constraint that scopes `.tangled/workflows.ncl` to the
formatting check; see workflows.ncl:23-37).
