# Libc-rs — Plan

Goal: an adopted, monorepo-owned, all-Rust libc substrate for Linux that
(a) runs the `rust/` tree's own tools, (b) becomes fully instrumentable under
Fe-C, and (c) feeds the NixOS-rs Rust-userspace trajectory.

## 1. Vendoring procedure (P0 entry)

1. Pin upstream commits (HEAD as of 2026-07-21; re-verify at import):
   - `sunfishcode/eyra` @ `cfa264ee062277101fd54043d8edb337e5ff98c2`
   - `sunfishcode/c-ward` @ `ac7e0f124aa678a7045d549aceb7e28ac233fe0f` (workspace: `c-scape`, `c-gull`)
   - `sunfishcode/origin` @ `00afc4488f81a6b2abbcd52856ee2fb4e44ac0cb`
2. Plain source import (no submodules; jj has no story for them anyway) into
   `rust/libc/{eyra,c-ward,origin}`. Strip upstream CI/workflows; keep
   READMEs, licenses, COPYRIGHT.
3. Write `UPSTREAM` per tree: repo, commit, date, and a `patches:` list that
   grows with every local change (the adoption ledger).
4. Add as workspace path members; `[patch]` any transitive references so the
   graph resolves to the vendored trees.
5. Dependency policy: `rustix`, `rustix-dlmalloc`, `unwinding`, etc. remain
   crates.io deps while actively maintained; the vendor trigger is upstream
   abandonment or a supply-chain policy call, nothing else.

## 2. Bring-up phases

| Phase | Deliverable | Gate |
| ----- | ----------- | ---- |
| **P0** | Builds under nix on the pinned nightly | `libc-hello` (+ `-static`, `-static-pie`) and static-DNS check green |
| **P1** | Runs the monorepo's own tools | A selected set (e.g. `bash-rs`, `make-rs`, `grep-rs`) built against the substrate as flake checks; upstream example smoke (ripgrep, coreutils subset) |
| **P2** | Fe-C `case`-mode substrate | `fe-c-sysroot-case-*` builds the substrate instrumented; whole-process demo: one P1 tool, every crate incl. libc checked; RustSec-style FFI repro traps end to end |
| **P3** | NixOS-rs integration | Substrate consumable from `rust/nixos`; static-service scope first; `eyra-c` (upstream's libc.a build) evaluated for residual C packages |
| **P4** | `through`-mode substrate | Tracks Fe-C v1; the libc itself dialed up |

## 3. Sync & adoption policy

- Quarterly spindle job diffs vendored trees against upstream HEAD; report
  artifact, no auto-merge.
- Local fixes land here first with an `UPSTREAM`-ledger entry; forwarded
  upstream while the project answers (it is quiescent — last tagged release
  Oct 2023 — not archived).
- Missing libc surface discovered during P1/P3 goes on a contribution list in
  this file rather than ad-hoc hacks in callers.

## 4. Nix integration ❄️

- Flakelight module exposing:
  - `packages.libc-hello{,-static,-static-pie}`
  - `packages.libc-sysroot` — eyra-linked std as a derivation keyed on the
    pinned nightly; the artifact other `rust/` projects (and Fe-C's
    instrumented sysroots) consume
  - `checks.libc-{hello,static-dns,examples,fec-case}` (last one phase-gated)
- `nix/lib/cargo` (the in-house per-crate/gradual build lib) for the
  workspace; corpus/example sources vendored so checks stay pure and offline;
  harmonia caches the sysroot derivations.
- Single `rust-toolchain.toml` shared with `../fe-c` — one nightly to bump,
  one pipeline bumping it.

## 5. Risks & mitigations

| Risk | Position |
| ---- | -------- |
| Upstream quiescence | Accepted — that's what adoption means; the ledger keeps a future re-sync or hand-back possible |
| Nightly churn | Shared pin + weekly bump pipeline (Fe-C §8); breakage budget one sitting per bump |
| Missing libc surface | Patch queue + contribution list; conservative stubs abort loudly rather than lie |
| No dynamic linking | Scope P3 to static services first; dynamic linking is an open question for the distro ambition, tracked, not hand-waved |
| Miri gap (asm syscalls) | Fe-C `case` mode covers what Miri can't reach here; Miri still runs on everything above the substrate |
| glibc compat edges | Static NSS/DNS is upstream-solved; PAM/NSS plugin ecosystems are explicitly out of scope |

## 6. Relationship to ../fe-c

This directory is why Fe-C gets to say "whole process." Fe-C's v0 gate builds
this substrate under `case` mode; this plan's P2 gate is that same event from
the other side. The two `rust-toolchain.toml` pins are one file.
