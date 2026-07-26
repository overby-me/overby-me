# LLVM-rs, agent brief

**Point an agent at this file.** Read this, then [PLAN.md](./PLAN.md) and
[STATUS.md](./STATUS.md). Everything else is reference material, linked from
the place where it matters.

Root conventions win over anything inferred here: nix-first, `jj` not `git`,
nushell for scripts, `rtk`-prefixed shell commands, never `nix flake check`.

---

## 1. What this is

An LLVM-compatible compiler infrastructure in Rust, compatible at the IR level
(textual `.ll` now, bitcode later) and eventually at the LLVM-C ABI level.
Scope is "the LLVM that rustc uses". The tier ladder and its exit criteria are
[PLAN.md](./PLAN.md) §8; where we actually are is [STATUS.md](./STATUS.md).

Today: **T0, IR core only.** Parse, verify, print. No optimizer, no codegen,
no rustc backend.

## 2. Hard rules

Violating one of these is a correctness bug, not a style disagreement.

| Rule | Why |
| --- | --- |
| **Round-trip fidelity is a test, not an aspiration.** A parser change that makes a corpus file re-print differently from its input fails `llvm-roundtrip`. Fix the printer or the parser, never the corpus. | The corpus is real rustc output. If we cannot reproduce it byte for byte, we do not understand the format yet, and every later differential test inherits the ambiguity. |
| **Never invent syntax.** Every accepted construct must exist in LangRef or in real LLVM output. When they disagree, real LLVM output wins and the divergence goes in `docs/dialect-notes.md`. | We are a compatibility project. A dialect of our own is a fork with extra steps. |
| **Unknown input is a hard error with a source location, never a silent skip.** No "best effort" parse, no dropping an attribute we do not model. | A silently dropped `noalias` is a miscompilation two tiers later. Loud degradation is the repo-wide rule. |
| **No C++ LLVM source is read or ported.** LangRef, the textual format and upstream `.ll` test inputs are specifications; `llvm/lib/**/*.cpp` is off limits. | Licensing (PLAN §12) and the clean-room claim in the README. |
| **Opaque pointers only.** Typed-pointer IR (`i8*`) is rejected, not silently accepted. | PLAN §1.2. Half-supporting a dead dialect costs more than refusing it. |
| **Every claim in STATUS.md is measured.** "Works", "complete", "supported" require a check that passes. Otherwise write "unmeasured" or "partial". | fe-c's rule, and it is the only thing that keeps a multi-year project honest. |
| **Everything builds and tests through nix.** Cargo is for iteration; a feature is not done until its check is wired and green. | Repo convention. |
| **Library crates stay dependency-free and stable-Rust for as long as that is free.** Adding a crate means a `nix/lib/cargo/index` commit; adding nightly means a toolchain pin. Both are fine when a tier needs them, neither is free. | PLAN §11 lists vendored-index friction as a live risk, and today the whole workspace builds with zero third-party crates. |

## 3. Settled, do not re-open

- **Module owns its Context.** `Module { ctx: Context, ... }`, so a module is
  self-contained and `Send + Sync` with no `LLVMContext` juggling. Type and
  constant ids are only meaningful together with their module. Cross-module
  work (linking, LTO) is explicit merging, later.
- **Arena plus per-block `Vec<InstId>`**, not intrusive instruction lists.
  PLAN §4.2 asks for intrusive *use-lists*, which is a different structure and
  arrives with the first pass that needs def-use chains. Instruction ids are
  stable under insertion and removal, which is the property passes actually
  need; a `Vec` per block is simpler to read and to verify.
- **Types and constants are interned** in the context, so structural equality
  is id equality.
- **Named structs have identity, literal structs are structural.** Two named
  structs with identical bodies are different types, per LangRef.
- **Metadata that we do not model semantically is modelled syntactically.**
  Specialized debug-info nodes (`!DILocation(...)`) parse into a generic
  keyed-field form that round-trips exactly. This is deliberate: DWARF
  modelling belongs in `llvm-debuginfo` at T1, and until then dropping to a
  faithful syntactic representation beats dropping the node.
- **`DataLayout` and `Triple` keep their source string** and print it back
  verbatim, while also exposing parsed queries. Byte-identical round-trip is a
  goal (PLAN §3) and re-canonicalising is a way to fail it.
- **The oracle is nixpkgs, not a checkout.** Upstream LLVM sources come from
  `pkgs.llvm.src` inside check derivations. Nothing is cloned from a forge and
  nothing upstream is vendored into the tree.

## 4. Task queue

Work in order. Each task is done when its acceptance check passes, not when it
looks finished. Marks are `[todo]`, `[partial]` (real work landed, acceptance
not met: say what remains in one sentence) and `[done]`.

**Keep this queue current.** It is the durable record across sessions and
context compactions. A task whose body describes finished work while its mark
says `[todo]` is a bug in this file.

### T0.1, IR core

**A1. [done] Workspace, docs and nix skeleton.** *(2026-07-26)*
Flakelight module, flake import, `llvm-fmt`, `llvm-clippy`, `llvm-unit`
checks, all three building. The cargo-check harness needs no vendor
directory because the workspace has no third-party dependencies; when that
changes, copy rust/fe-c's `vendorFor`.

**A2. [todo] `llvm-support`: APInt, APFloat, DataLayout, Triple.**
Acceptance: unit tests including APInt cross-checked against `u128`/`i128`
native arithmetic over a wide operand grid, and datalayout strings from the
corpus round-tripping byte for byte.

**A3. [todo] `llvm-ir`: types, constants, instructions, attributes, metadata.**
Acceptance: the data model expresses every construct in the seed corpus, shown
by A4 and A5 round-tripping it.

**A4. [todo] `llvm-ir-print`: textual printer with LLVM's slot numbering.**
Acceptance: `llvm-roundtrip` green over the corpus.

**A5. [todo] `llvm-ir-parse`: lexer and recursive-descent parser.**
Acceptance: same check; plus every parse error carries a line and column.

**A6. [todo] Verifier.**
Structural, type and dominance rules for the emitted subset.
Acceptance: `llvm-verify-corpus` green, and each rule has a test that a
deliberately broken module is rejected with the expected message.

**A7. [todo] `opt` with an upstream-compatible CLI subset.**
Acceptance: `llvm-roundtrip` drives the real binary, not a test harness.

### T0.2, conformance and the rustc seam

**B1. [todo] Upstream `llvm/test/Assembler` and `llvm/test/Verifier` as an oracle.**
From `pkgs.llvm.src`, not vendored. Ratcheted pass rates in STATUS.md, checks
`llvm-upstream-assembler` and `llvm-upstream-verifier`.
Acceptance: both checks green at the recorded ratchet, and the ratchet only
ever moves up.

**B2. [todo] Differential check against real `opt -S -passes=verify`.**
For every corpus file, compare our output to the oracle's after normalising
the differences we have consciously accepted (recorded in
`docs/dialect-notes.md`).
Acceptance: `llvm-opt-differential` green, divergence list shrinking.

**B3. [todo] Surface inventory from `rustc_codegen_llvm`.**
Extract every FFI call site from the pinned rustc's source (nixpkgs
`rustc.src`, not a clone) into `docs/surface-inventory.md`, mapped to the
areas in PLAN §3, with a count per area. This is what turns "scope" from a
guess into a number.
Acceptance: the document exists, its counts are reproducible by a committed
nushell script, and STATUS.md cites them.

**B4. [todo] IR builder API sufficient for the fork's call sites.**
Whatever B3 says the fork needs, phrased as safe Rust rather than as an FFI
mirror.
Acceptance: `llvm-builder-smoke`, a check that builds hello-world-shaped IR
programmatically and verifies plus prints it.

**B5. [todo] Vendor `rustc_codegen_llvm` into `crates/rustc-codegen-llvmrs`.**
Needs a pinned nightly with `rustc-dev` and a `rust-toolchain.toml` (PLAN §6.1).
Everything unimplemented panics with `llvmrs-todo: <symbol>`.
Acceptance: it compiles.

### Later tiers

T1 onwards is [PLAN.md](./PLAN.md) §8. Do not start a T1 task while a T0 task
is `[todo]`.

## 5. Working here

- Build and test: `cargo test --workspace` for iteration,
  `nix build .#checks.x86_64-linux.llvm-<name>` before claiming anything.
- The corpus regenerates with `nu corpus/regen.nu` (needs `rustc` on PATH). It
  is committed, so a check never shells out to rustc.
- Commit style: `feat(rust/llvm): ...`, one scope per commit, no co-author
  trailers, and run `rtk jj diff --stat` before writing the message.
- Before committing Rust: `cargo fmt`, then
  `cargo clippy --workspace --all-targets -- -D warnings`. The pre-commit hooks
  run both and the abort-retry cycle is slower than doing it first.
- `deslop scan rust/llvm` catches AI-slop patterns; `.deslop.toml` records the
  rules that are disabled and why.
