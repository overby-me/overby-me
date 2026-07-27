# Status

What exists, what is a stub, and what claim is *not* being made yet. Written in
the register [rust/fe-c](../fe-c/STATUS.md) uses: a sentence here is either
backed by a check that passes or is marked unmeasured.

**Last updated:** 2026-07-27.
**Tier:** T0 (PLAN.md §8), in progress.

## Pins

| Thing | Value | Notes |
| --- | --- | --- |
| IR dialect | LLVM 21 | The major that `nixpkgs.llvm` (21.1.8) and the repo's rustc both speak. Older dialects are refused, not half-supported. |
| Oracle LLVM | `pkgs.llvm` 21.1.8 | Test-only, inside check derivations. Never a build or runtime dependency of any package output. |
| Rust toolchain | stable, whatever the devshell ships (1.95 today) | No `rust-toolchain.toml` yet on purpose: nothing here needs nightly. The pin arrives with `crates/rustc-codegen-llvmrs` (task B5), which needs `rustc_private`. |
| Third-party crates | none | The whole workspace has zero dependencies, so there is nothing to add to `nix/lib/cargo/index` yet. |

## What works

This section grows one row per completed task in [CLAUDE.md](./CLAUDE.md) §4,
and each row names the check that backs it.

| Area | State | Backed by |
| --- | --- | --- |
| Workspace, docs, nix wiring | done | `llvm-fmt`, `llvm-clippy`, `llvm-unit` |
| APInt: arbitrary-width integer arithmetic, formatting, parsing | done | `llvm-unit`, cross-checked against native `u128` over an operand grid at eleven widths |
| APFloat: bit-pattern carrier plus LLVM's textual forms | done | `llvm-unit`, including every `half` and `bfloat` bit pattern |
| DataLayout: parsing, alignment queries, verbatim printing | done | `llvm-unit` |
| Triple: component parsing, verbatim printing | done | `llvm-unit` |
| IR data model: types, constants, instructions, attributes, metadata | done | `llvm-unit`, and the round trip below, which cannot pass without it |
| Textual printer | done, byte-identical to `llvm-dis` over the corpus | `llvm-roundtrip` |
| Textual parser | done for everything the corpus contains | `llvm-roundtrip` |
| Type layout: sizes, alignments, struct offsets | done | `llvm-unit` |
| Verifier: structure, types, flags, call signatures, dominance | done for the modelled subset | `llvm-verify-corpus` |
| Verifier: linkage against visibility, module flag shape, atomic orderings, index bounds | done | `llvm-verify-corpus` |
| Verifier: sized-type rules, intrinsic-only types, reserved global shapes | done | `llvm-verify-corpus` |
| Type aliases: `%name = type [8 x i8]` expands where used | done | `llvm-upstream-assembler` |
| Default alignments filled in from the data layout | done | `llvm-opt-differential` |
| Function attributes hoisted into numbered groups on output | done | `llvm-opt-differential`, `llvm-roundtrip` |
| Metadata uniquing and renumbering on output | done | `llvm-opt-differential`, `llvm-roundtrip` |
| Verifier: placement rules for `!range`, `!align`, `!nonnull`, `!prof`, scope lists | done | `llvm-upstream-verifier` |
| `opt`, for the flags it accepts | done | `llvm-roundtrip`, which drives the built binary |
| Builder API: types inferred, alignments filled in | done for the common instructions | `llvm-builder-smoke` |

## The round trip

Every file in `corpus/` is canonical `llvm-dis` output, and parsing one and
printing it back reproduces it byte for byte. As of 2026-07-27 that is 9
files and roughly 3,600 lines: 6 generated from real `rustc --emit=llvm-ir`
(arithmetic, control flow, memory, atomics, calls, unwinding) and 3
hand-written to pin syntax rustc never emits (module structure, one of every
instruction, one of every constant and type form).

This is a stronger property than "the parser accepted it". It says we agree
with upstream about slot numbering, predecessor order, blank lines, label
padding, which defaults print as nothing, and the several places where the
same attribute is spelled differently depending on where it sits.

## Conformance against upstream's suites

Measured, not claimed. Each upstream test says in its own RUN lines whether
`llvm-as` should accept or reject it, so agreement counts both halves: a file
we accept that upstream accepts, and a file we reject that upstream rejects.
Files whose RUN lines need a tool or a pass this tier does not have are
skipped and counted separately rather than scored.

| Suite | Agreed | Considered | Skipped | Check |
| --- | --- | --- | --- | --- |
| `llvm/test/Assembler` | 175 | 306 | 177 | `llvm-upstream-assembler` |
| `llvm/test/Verifier` | 117 | 254 | 74 | `llvm-upstream-verifier` |

A third check asks a different question: not whether we accept the same
files, but whether we print the same text. For every Assembler file both we
and upstream accept, `llvm-opt-differential` compares our `opt -S` output
against `llvm-as | llvm-dis`, and **105 of 160** are identical. Two
path-derived lines are normalised away, because upstream regenerates the
ModuleID from whatever path it read and synthesises a `source_filename` when
the file has none; the corpus round trip pins both fields properly against
files that carry them.

The first measurement was 146 and 70. Both numbers are still low and both are
the point: the gap is a to-do list, and
`docs/dialect-notes.md` groups the recurring reasons. The ratchets are in
`default.nix` and only ever move up. The suites earned their place on the
first run by finding a parser hang that the corpus never triggered.

## How large the job is

Measured, not estimated. `rustc_codegen_llvm` at rustc 1.95.0 declares **363
distinct LLVM entry points** and mentions them **532 times**; **125** of them
are `LLVMRust*` shims rather than the stable C API. Nothing is declared and
unused.

Two thirds of that surface (240 entry points) is IR construction, attributes
and metadata, module and context, and targets, which is the area T0 has been
building. Debug info is the largest area outside it at 40. The areas that are
almost entirely `LLVMRust*` shims, and so carry the churn risk PLAN.md
section 2.2 flags, are the later tiers: all 19 LTO, PGO and coverage entry
points are shims.

The full table and what it does and does not mean are in
[docs/surface-inventory.md](./docs/surface-inventory.md), reproducible with
`nu docs/surface-inventory.nu`.

## What is not started

Everything else. Concretely, and to forestall the usual optimistic reading of
a compiler project's README:

- **No code generation.** No instruction selection, no register allocation, no
  object emission, no assembler. `llc` does not exist.
- **No optimizer.** No pass manager beyond the identity, no analyses, no
  transforms. `opt -passes=instcombine` is an error, not a no-op. The
  dominator tree the verifier computes is the only analysis that exists, and
  it is not exposed.
- **No bitcode.** Reading and writing `.bc` are both T3. `llvm-as` and
  `llvm-dis` do not exist because their contract is bitcode.
- **No rustc backend.** `rustc_codegen_llvm` is not vendored, and no Rust
  program compiles through this project.
- **No debug info modelling.** Debug-info nodes round-trip as written, but
  nothing knows what a `DICompileUnit` field means, so a field left at its
  default is printed back rather than omitted the way upstream omits it.
  That is the largest remaining cause of print differences and it wants the
  `llvm-debuginfo` crate at T1, not a table of defaults.
- **No unwinding, no LTO, no PGO, no coverage.**
- **No C ABI.** `llvm-c-abi` is T5.

## Divergences from PLAN.md

Recorded so that the plan stays the plan and the deltas stay visible.

| Plan says | Reality | Why |
| --- | --- | --- |
| `rust-toolchain.toml` in the tree from the start (§4.1) | not present | Nothing in T0.1 needs nightly, and an unused pin is a maintenance cost plus a slower nix build. It lands with task B5, which is the first thing that needs `rustc_private`. |
| Use-lists as intrusive lists over indices (§4.2) | instruction storage is an arena, blocks hold `Vec<InstId>`, no use-lists yet | Def-use chains have no consumer before the first analysis pass. Instruction ids are stable regardless, which is the property that matters for retrofitting. |
| Vendor upstream tests under `corpus/upstream/` (§7.1) | upstream tests are read from `pkgs.llvm.src` in check derivations | Same coverage, no third-party import into the tree, and the oracle version is pinned by the flake lock rather than by a copy that silently ages. |
