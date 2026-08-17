# safety/oxidized/llvm — LLVM-compatible compiler infrastructure written in Rust

> Plan of record for `safety/oxidized/llvm` in `tangled.org/overby.me/overby.me`.
> Drop this file in as `safety/oxidized/llvm/PLAN.md`. Companion docs to create alongside it,
> following the `safety/fe-c` conventions: `README.md`, `STATUS.md`, `CLAUDE.md`,
> `docs/`, `corpus/`, `default.nix`, `rust-toolchain.toml`.

## 0. Summary and headline decisions

**What:** an LLVM-*compatible* compiler infrastructure in pure Rust — compatible at
the **LLVM IR level** (textual `.ll` now, bitcode `.bc` later) and eventually at the
**LLVM-C / `LLVMRust*` ABI level**, not at the C++ API level. Scope is deliberately
"the LLVM that rustc actually uses", not all of `llvm-project`.

**How rustc consumes it (the investigated question):** neither a brand-new backend
nor untouched reuse, but a **fork of `rustc_codegen_llvm` living inside this
project** as a nightly-pinned crate, de-FFI'd to call safety/oxidized/llvm's native Rust API,
loaded into stock nightly rustc via `-Zcodegen-backend=…/librustc_codegen_llvmrs.so`.
Later, once the surface is broad enough, the *unmodified* in-tree
`rustc_codegen_llvm` becomes a conformance target via an exported C-ABI shim
(`LLVM*`/`LLVMRust*` symbols + a fake `llvm-config`). Rationale in §2.4.

**Placement:** one project at `safety/oxidized/llvm/` (multi-crate workspace, `safety/fe-c`
layout), not two projects. The rustc backend crate versions in lockstep with the IR
library and shares its toolchain pin; splitting it out buys nothing until an
external consumer needs an independent release cadence. Rationale in §2.5.

**Honesty up front:** upstream LLVM's core is millions of lines of C++ refined over
20+ years. Even scoped to "what rustc exercises", this is a multi-year effort. The
plan is structured so that every tier ships something independently useful to this
monorepo (§8), the way safety/oxidized/gcc already does for safety/oxidized/nixpkgs.

## 1. Goal and non-goals

### 1.1 Goals

1. A pure-Rust codegen path for rustc: `rustc → safety/oxidized/llvm → object files → wild`,
   with `safety/oxidized/libc` (Eyra lineage) underneath — closing the last big 💣 in the
   monorepo's toolchain row ("Compiler Framework: MLIR/LLVM 💣").
2. LLVM IR compatibility: parse and print the textual IR of the LLVM major that
   the pinned rustc release uses, with verifier-equivalent semantics, so real
   `opt`/`llc`/`clang` remain available as differential-testing oracles.
3. Correctness as the product: differential testing, translation validation, and
   fuzzing are first-class deliverables, not afterthoughts (§7).
4. Eventually a drop-in `llvmPackages`-shaped attribute for safety/oxidized/nixpkgs
   (`opt`, `llc`, `llvm-as`, `llvm-dis` binaries with compatible CLI subsets),
   mirroring how safety/oxidized/gcc presents itself as `gcc`/`cc`.

### 1.2 Non-goals (explicit, to keep the project survivable)

- **Not** the C++ API. No `llvm::IRBuilder` ABI compat — impossible and pointless
  from Rust. Compatibility layers are IR text/bitcode and the C ABI only.
- **Not** clang, lld (we have wild), MLIR, flang, libc++, compiler-rt builtins
  (Rust `compiler-builtins` already covers rustc's needs), or the sanitizer
  runtimes. MLIR matters for the Mojo side of the monorepo but is a separate
  mountain; revisit only after T3.
- **Not** SelectionDAG/GlobalISel replication. We implement LLVM's *semantics*,
  not its internal architecture (§4.3).
- **Not** every target. Order: x86_64-linux → aarch64-linux → aarch64-darwin →
  riscv64/wasm32/i686. Windows (COFF/SEH) is deferred until there is demand.
- **Not** legacy PM or pre-opaque-pointer *semantics*. We pin to modern IR
  semantics: opaque `ptr` only, and the model never holds a typed pointer.
  The older *spelling* is read, because upstream reads it: `llvm-as` in LLVM
  21 folds `i8*` to `ptr` as it parses, and nothing downstream of either
  parser can tell the two apart. Refusing the spelling was measured at 293
  files across the test trees, three quarters of `llvm/test/Bitcode` among
  them, and bought nothing: it is a fold, not a dialect.

## 2. Investigation: landscape and integration options

### 2.1 What exists today (July 2026)

- **No serious LLVM-in-Rust reimplementation exists.** The idea was proposed as
  "RLLVM" on the Rust forum in 2018 and the community conclusion was "just use
  Cretonne/Cranelift" (<https://users.rust-lang.org/t/proposal-rllvm-rust-implementation-of-llvm-aka-just-use-cretonne/16021>).
  That conclusion is correct for the mainstream; this monorepo's whole premise
  (gcc, systemd, perl, pcre2 rewrites) is to do the unreasonable thing with
  compat-first engineering and heavy oracle testing, so we proceed with eyes open.
- **rustc's backend seam is mature.** rustc has three backends today:
  `rustc_codegen_llvm` (default; talks to LLVM through the LLVM-C API plus C++
  shims in `compiler/rustc_llvm/llvm-wrapper/*.cpp`, declared in
  `rustc_codegen_llvm/src/llvm/`), `rustc_codegen_cranelift` (pure Rust,
  shipped as the rustup component `rustc-codegen-cranelift-preview`, selectable
  with `-Zcodegen-backend`), and `rustc_codegen_gcc` (libgccjit). Shared
  MIR-lowering machinery lives in `rustc_codegen_ssa`. Out-of-tree backends are
  dylibs loaded by path: `-Zcodegen-backend=/path/to/libbackend.so`.
- **Cranelift is the honest benchmark.** The 2025H2 Rust project goal targets a
  production-ready cg_clif for Linux/macOS on x86_64/aarch64; measured wins are
  ~20% of codegen time (~5% of clean-build wall time) on large crates, and
  debuginfo remains a major gap. Takeaway: even a pure-Rust *non-optimizing*
  backend is a years-long effort for a professional team. Our differentiators
  must be (a) LLVM IR compat → oracle-rich testing, (b) an optimizing tier,
  (c) reuse of everything the Rust ecosystem already built (§5).
- **Reusable pure-Rust building blocks exist now that didn't in 2018:**
  `regalloc2`, `object`, `gimli`, `target-lexicon`, `cranelift-isle` (pattern-
  matching DSL for instruction selection), `iced-x86` (complete x86 encoder),
  the Bytecode Alliance `filecheck` crate, wild (linker, already ✅ in this
  repo), and Trail of Bits' Mollusc crates (`llvm-bitstream`/`llvm-mapper`,
  pure-Rust bitcode *reading* — evaluate before writing our own reader).
- **Adjacent in-repo assets:** safety/oxidized/gcc (based on Anthropic's ccc) already
  contains working x86-64/i686/ARM64/RISC-V code emission, a built-in assembler,
  and a built-in linker; safety/oxidized/binutils exists; safety/oxidized/libc is the Eyra-lineage
  substrate; krabby (tracked in the README R&D column) is a from-scratch fast
  Rust *frontend* experiment with no codegen — complementary, not overlapping.

### 2.2 Option analysis: how should rustc talk to safety/oxidized/llvm?

**Option A — C-ABI substitution under unmodified rustc.** Implement the exact
`extern "C"` surface rustc links (`LLVM*` C API subset + every `LLVMRust*`
wrapper), ship it as a staticlib with a fake `llvm-config`, and build stock rustc
against it.
*Pros:* zero rustc-fork maintenance; the strongest possible conformance statement;
enables a pure-Rust rustc *bootstrap* later.
*Cons:* all-or-nothing — rustc initializes targets, passes, diagnostics, LTO
plumbing at startup, so nothing runs until a huge surface exists; terrible for
incremental development; couples us to unstable `LLVMRust*` churn every release.

**Option B — fork `rustc_codegen_llvm` into this project, replace FFI with native
calls, load via `-Zcodegen-backend`.** The fork keeps cg_llvm's battle-tested
logic — attribute mapping, the enormous intrinsic table, debuginfo lowering, ABI
handling via `rustc_codegen_ssa` — and swaps `unsafe { LLVMBuildAdd(...) }` for
safe calls into `llvm-ir`.
*Pros:* incremental from day one (stub everything, `todo!()` on untouched paths,
grow coverage test-by-test); works with stock rustup nightly; follows the proven
cg_clif/cg_gcc out-of-tree development model; the fork's call sites *are* the
scope specification for safety/oxidized/llvm.
*Cons:* tracking rustc nightly churn (mitigated by the Kani model already used in
fe-c: one pinned nightly in `rust-toolchain.toml`, bumped by pipeline); requires
`rustc_private`.

**Option C — brand-new backend implementing `rustc_codegen_ssa` traits directly
against a bespoke IR** (the cg_gcc pattern).
*Cons that kill it:* since our IR *is* LLVM IR, this re-derives exactly the
mapping cg_llvm already encodes, discarding a decade of fixes for zero design
freedom gained. Rejected.

**Option D — abandon LLVM-compat and adopt/extend Cranelift.** Right answer for
most people; wrong for this project's stated goal (LLVM-compatible framework,
optimizing tier, nixpkgs drop-in, oracle testing against real LLVM). We instead
*consume* Cranelift components (ISLE, regalloc2) and treat cg_clif as the bar to
beat on compile speed at O0.

### 2.3 Decision

**B now, A later.** Develop against Option B as the daily driver from T0. Build
Option A's `llvm-c-abi` crate incrementally *behind* B (same underlying library),
and promote "stock rustc builds and passes its testsuite against our staticlib"
to the T5 conformance milestone. This sequencing means the C ABI is implemented
against a library that already works, not as a wall of stubs.

### 2.4 Why reusing (a fork of) the existing LLVM backend beats writing a new one

`rustc_codegen_llvm` encodes thousands of accumulated decisions that are
invisible until they bite: exact attribute sets per callconv, `noalias`/
`dereferenceable`/`range` metadata placement, the `core::arch` intrinsic name
mapping, personality-function and landing-pad shapes, coverage/PGO plumbing,
16-byte-aligned i128 quirks, wasm/exception-ABI special cases. A fresh backend
would rediscover each one as a miscompilation. Forking keeps them all and turns
every remaining `llvm::ffi` call site into a checklist item. The fork also
inherits `rustc_codegen_ssa`'s driver (parallel codegen units, archive/metadata
handling), which cg_clif had to partially reimplement.

### 2.5 Why one project, not two

The backend crate (`rustc-codegen-llvmrs`) needs the same pinned nightly, the
same `Cargo.lock`, and API-lockstep with `llvm-ir`; in this monorepo each
project carries its own lock, toolchain pin, and cargo-index wiring, so a
separate `rust/rustc-backend` project would mean cross-project version pinning
with zero benefit. fe-c sets the precedent: rustc-coupled driver + stable
library crates in one workspace, one `rust-toolchain.toml`. Extract later only
if prebuilt per-toolchain dylib distribution to third parties becomes a goal.

## 3. Scope: "the LLVM that rustc uses"

The fork's FFI call sites define v1 scope precisely. Inventory to encode in
`docs/surface-inventory.md` during T0, grouped roughly as:

| Area | Contents (indicative) | Tier |
| --- | --- | --- |
| IR construction | Context/Module/Builder, all instructions rustc emits, opaque `ptr`, aggregates, `i1..i128`, `half/float/double/fp128`, vectors, atomics (all orderings), `invoke`/`landingpad`/`cleanuppad`, inline asm, intrinsic declarations | T0–T1 |
| Attributes & metadata | param/ret/fn attributes rustc sets; `!range`, `!nonnull`, `!align`, `!noalias`/`!alias.scope`, `!dbg`, `!prof`; module flags; data layout strings | T0–T1 |
| Verifier | Structural + dominance + type rules for the emitted subset; hard-fail on anything outside the pinned dialect | T0 |
| Targets | TargetMachine equivalents: x86_64 then aarch64; data layouts must match real LLVM byte-for-byte | T0–T2 |
| Optimization | Semantic equivalent of `default<O0..O3>` pipelines over our own pass set (§4.4) | T2 |
| Object emission | ELF (relocations, sections, comdat, TLS models, `.eh_frame`, symbol visibility) via `object`; Mach-O at T4 | T0–T2 |
| Debug info | DIBuilder-equivalent → DWARF 5 via `gimli`; line tables first, then vars/types | T1–T2 |
| Unwinding | DWARF CFI, LSDA/`gcc_except_table`, Rust personality; `panic=abort` ships first | T1 |
| Intrinsics | `llvm.*` generic set (memcpy, ctpop, overflow ops, fma, masked ops…), then `core::arch` per-target sets, autogenerated from stdarch metadata where possible, soft fallbacks elsewhere | T1–T2 |
| LTO/PGO/coverage | Fat LTO (IR-level, own serialization) → ThinLTO summaries + bitcode; PGO instr/use; coverage mapping + Rust profile runtime (evaluate `minicov`) | T3 |
| Bitcode | Reader (evaluate Mollusc) then writer; required for ThinLTO and `llvm-as`/`llvm-dis` parity | T3 |

## 4. Architecture

### 4.1 Directory layout (fe-c conventions)

```text
safety/oxidized/llvm/
├── Cargo.toml            # workspace
├── Cargo.lock
├── rust-toolchain.toml   # one pinned nightly (Kani/fe-c model)
├── default.nix           # flakelight module: packages + named checks
├── README.md  PLAN.md  STATUS.md  CLAUDE.md  .deslop.toml
├── docs/                 # surface-inventory, design records, evaluations
├── corpus/               # .ll conformance corpus + vendored LLVM tests
├── platform/nix/                  # project-local nix helpers (oracle pins, check harness)
└── crates/
    ├── llvm-support      # APInt/APFloat, DataLayout, Triple (target-lexicon)
    ├── llvm-ir           # Context/Module/Value/Type/Instr, attrs, metadata, verifier
    ├── llvm-ir-parse     # textual .ll parser
    ├── llvm-ir-print     # textual .ll printer (round-trip fidelity is a feature)
    ├── llvm-bitcode      # .bc reader/writer (T3; evaluate Mollusc for reading)
    ├── llvm-analysis     # dominators, loops, BasicAA-lite, (SCEV-lite later)
    ├── llvm-transforms   # mem2reg, sroa, simplifycfg, instcombine-lite, inline,
    │                     # gvn-lite, licm, dse-lite, adce, loop-unroll-lite, …
    ├── llvm-codegen      # legalization → MIR-lite → ISLE isel → regalloc2 →
    │                     # prologue/epilogue → CFI/unwind tables
    ├── llvm-target-x86   # lowering patterns + encoder (iced-x86 underneath)
    ├── llvm-target-aarch64
    ├── llvm-mc           # per-target asm parsing (inline/module asm), object
    │                     # emission via `object`
    ├── llvm-debuginfo    # DIBuilder-equivalent → gimli::write DWARF 5
    ├── llvm-lto          # fat LTO now, ThinLTO summaries at T3
    ├── llvm-c-abi        # extern "C" LLVM-C + LLVMRust* surface (Option A)
    ├── llvm-tools        # bins: llvm-as, llvm-dis, opt, llc, llvm-filecheck glue
    └── rustc-codegen-llvmrs   # forked rustc_codegen_llvm; rustc_private;
                               # builds the -Zcodegen-backend dylib
```

### 4.2 IR core decisions

- **Dialect pin:** the LLVM major that the pinned rustc fork
  (`rust-lang/llvm-project`) uses at project start; bump with the toolchain, one
  major at a time. Refuse to parse older dialects rather than half-support them.
- **Semantics:** opaque pointers only; `poison`/`undef` and UB rules per LangRef;
  fast-math flags carried but only exploited where a pass explicitly justifies it;
  integer semantics via a real APInt (arbitrary width up to i128 fast-path).
- **Memory model:** SSA values as arena indices (`u32` newtypes), instructions in
  per-function arenas, use-lists as intrusive lists over indices. `Send + Sync`
  modules from day one — the one structural advantage over C++ LLVM (per-cgu
  parallel codegen with shared read-only context, no `LLVMContext` juggling).
- **Verifier-first:** every builder invariant checked in debug builds; the
  verifier is the contract the fork develops against.

### 4.3 Backend pipeline (no SelectionDAG clone)

`llvm-ir → legalize (types/ops per target) → MIR-lite (vreg machine IR)
→ ISLE-driven instruction selection → regalloc2 → frame lowering
→ encode (iced-x86 / aarch64 tables) → object` — the Cranelift-shaped pipeline,
because it is proven in Rust, but consuming LLVM IR and keeping room for
machine-level peepholes. Inline asm requires a real per-target assembler in
`llvm-mc`; x86 leans on iced-x86's encoder, aarch64 gets hand-built encode
tables (evaluate lifting safety/oxidized/gcc's ARM64 emitter, §10).

### 4.4 Optimization tier (T2) philosophy

Implement the *semantics* of `-O0/-O1/-O2/-O3` with our own pass set; never chase
pass-for-pass parity. Priority order is what moves Rust code: mem2reg+SROA,
inlining, simplifycfg, instcombine-lite (a curated, Alive2-verified rewrite set —
not 10k patterns), GVN-lite, LICM, DSE-lite, bounds-check-friendly range
propagation, loop unrolling, straightforward vector legalization. Every rewrite
pattern lands with an Alive2-checked `.ll` pair in `corpus/`.

## 5. Dependency policy

Rule: never rewrite what the Rust ecosystem already does well; everything must be
pure Rust and vendorable into `platform/nix/lib/cargo/index`.

| Use | Crate | Notes |
| --- | --- | --- |
| Register allocation | `regalloc2` | Battle-tested in Cranelift |
| Object files | `object` | ELF now; Mach-O/COFF later for free |
| DWARF | `gimli` (read+write) | Debuginfo and `.eh_frame`/CFI |
| Triples | `target-lexicon` | |
| ISel DSL | `cranelift-isle` | Standalone codegen tool; evaluate early, fall back to hand-written matchers if it fights the IR shape |
| x86 encoding | `iced-x86` | Complete encoder/decoder |
| FileCheck | `filecheck` (Bytecode Alliance) | Test harness |
| Bitcode read | Mollusc (`llvm-bitstream`, `llvm-mapper`) | Evaluate; write path is ours |
| Misc | `hashbrown`, `smallvec`, `indexmap`, `rayon` | |

Real LLVM (`opt`, `llc`, `clang`, `alive2`) appears **only** in test-oracle nix
checks, never as a build or runtime dependency of any package output — same
separation safety/oxidized/gcc keeps with real gcc.

## 6. rustc integration mechanics

### 6.1 Primary path (Option B)

1. Vendor `compiler/rustc_codegen_llvm` at the pinned nightly's commit into
   `crates/rustc-codegen-llvmrs` (MIT/Apache-2.0 headers kept). Record the exact
   upstream commit in `STATUS.md`; re-vendor on every toolchain bump.
2. Replace `llvm::ffi` types with `llvm-ir` handles behind a thin `llvm.rs`
   compatibility module so upstream diffs stay reviewable (`jj`-friendly).
3. Everything unimplemented panics with a stable message
   (`llvmrs-todo: <symbol>`); a nix check greps panics from testsuite runs into a
   ranked "next work" queue for `CLAUDE.md` — the ordered-task-queue convention
   fe-c already uses.
4. Consumption: `RUSTFLAGS="-Zcodegen-backend=$(nix build …)/lib/librustc_codegen_llvmrs.so"`
   or `CARGO_PROFILE_DEV_CODEGEN_BACKEND` — identical UX to cg_clif.
5. Distribution inside the monorepo: a wrapper package `oxidized-llvm-rustc` that
   pairs the dylib with its exact matching nightly, so safety/oxidized/nixpkgs can consume
   it as one coherent toolchain.

### 6.2 Conformance path (Option A, grown behind B)

`llvm-c-abi` exports the C surface by delegating to the same library the fork
calls. T5 gate: build stock rustc from source against it (staticlib + shim
`llvm-config` reporting our lib/include paths), run the rustc testsuite. This is
also the door to the fully-bootstrapped pure-Rust rustc, and to non-rustc
consumers (anything speaking LLVM-C).

### 6.3 Upstreaming posture

Long-term, propose distribution as a rustup component (the cg_clif precedent)
once T2 exit criteria hold. Until then the project stays out-of-tree and tracks
one pinned nightly; no attempt to keep pace with every nightly.

## 7. Testing and conformance (the actual product)

1. **Upstream test suites — the same bar the other rust/* rewrites hold
   themselves to,** with one LLVM-specific wrinkle: unlike GNU sed/awk suites,
   `llvm/test/` mixes behavioral conformance with tests of LLVM's *internal
   architecture*, so a blanket "run the suite" would mismeasure both ways.
   Policy (per-suite ratcheted pass rates in `STATUS.md`, checks named
   `llvm-upstream-<suite>`):
   - *Pass verbatim:* `llvm/test/Assembler`, `llvm/test/Verifier`,
     `llvm/test/Bitcode` (from T3), applicable `llvm/test/DebugInfo`, and tool
     CLI tests for the `llvm-as`/`llvm-dis`/`opt`/`llc` subsets we ship — these
     encode observable, architecture-independent behavior.
   - *Semantic, not textual:* `llvm/test/Transforms` — its FileCheck lines pin
     LLVM's exact output IR, which an equivalent-but-different optimizer
     legitimately won't reproduce. Run these as *inputs* gated by Alive2 +
     execution differential (items 2–3), and count them separately.
   - *Input corpus only:* `llvm/test/CodeGen` exact-asm tests
     (SelectionDAG/GlobalISel-shaped); reuse their `.ll` inputs for
     execution-diff, never chase their asm CHECK lines.
   - *Whole-program, verbatim:* the separate `llvm-test-suite` repo
     (compile-and-run, output-compared) — the direct analog of running GNU's
     testsuite against safety/oxidized/sed, and a straight conformance target.
   All vendored under `corpus/upstream/` (Apache-2.0 WITH LLVM-exception
   headers kept), driven by the Rust `filecheck` crate and a small lit-style
   runner in `llvm-tools`.
2. **Differential vs real LLVM.** For every `.ll` in the corpus: our
   `opt -passes=X` vs real `opt`, our `llc` vs real `llc`, compared at the
   behavior level (execute both objects under the same harness) rather than
   byte level. Oracle LLVM comes pinned from nixpkgs inside check derivations.
3. **Translation validation.** Alive2 (external C++ oracle, test-only) gates
   every instcombine/GVN pattern; a transform without an Alive2-green corpus
   pair does not merge.
4. **ABI conformance.** abi-cafe run cg_llvm-vs-llvmrs across the calling-
   convention matrix (i128, f16/f128, SIMD, sret/byval are where backends die).
5. **rustc testsuite.** The fork's north star: `tests/ui` + `tests/codegen` +
   run-pass under `-Zcodegen-backend`, pass-rate ratcheted in CI (never
   decreases). Then crater-style runs over the monorepo's own rust/* projects —
   a uniquely good in-house corpus (build safety/oxidized/grep, safety/oxidized/sed, wild itself…).
6. **Fuzzing.** rustlantis (MIR-level differential fuzzing cg_llvm vs llvmrs);
   csmith+creduce via safety/oxidized/gcc once it can target llvmrs IR (§10); IR-level
   structured fuzzing of parser/verifier/passes with cargo-fuzz.
7. **Execution differential in CI:** every green tier keeps a check that builds
   a pinned set of real programs with both backends and diffs observable
   behavior and (later) performance.

## 8. Roadmap: tiers with exit criteria

Every exit criterion is an individually runnable flake check
(`nix build .#checks.x86_64-linux.llvm-<name>`); `nix flake check` stays
forbidden repo-wide as usual.

### T0 — Skeleton and hello world (the proof)

- `llvm-ir` + parser/printer/verifier for the rustc-emitted subset; round-trip
  fidelity check over a seeded corpus of rustc `--emit=llvm-ir` output.
- x86_64 `-O0`, `panic=abort`, no debuginfo: `fn main() { println!(…) }` and the
  `core`/`alloc` smoke set compile through `rustc-codegen-llvmrs` and run.
- Checks: `llvm-fmt`, `llvm-clippy`, `llvm-unit`, `llvm-roundtrip`,
  `llvm-t0-hello`, `llvm-verify-corpus`.
- Deliverable value: the surface inventory (§3) is now measured, not guessed.

### T1 — Usable development backend (pure-Rust dev toolchain)

- Unwinding (`invoke`/landingpad, `.eh_frame` via gimli, LSDA, Rust
  personality), i128, atomics/TLS/threads, common inline-asm subset, generic
  `llvm.*` intrinsics + `core::arch` soft-fallbacks, debuginfo line tables.
- aarch64-linux bring-up (second target forces the target abstraction honest).
- rustc `tests/ui` pass rate ≥ 95% at O0 on x86_64-linux; abi-cafe green.
- Monorepo dogfood: at least five sibling `rust/*` projects build and pass
  their own checks with `-Zcodegen-backend=llvmrs`.
- **This tier already delivers the headline:** rustc + safety/oxidized/llvm + wild +
  safety/oxidized/libc = an all-Rust debug-build toolchain, before any optimizer exists.

### T2 — Optimizing tier

- §4.4 pass set behind `default<O1/O2>`; every pattern Alive2-gated.
- Full debuginfo (vars/types), full `core::arch` for x86_64+aarch64
  (autogenerated from stdarch metadata where possible).
- Exit: rustc testsuite green at O2 minus an allowlist; runtime geomean of a
  pinned benchmark set ≤ 2.0× real LLVM `-O2` (ratchet down per release);
  compile speed at O0 competitive with cg_clif on the same corpus.

### T3 — Release-parity features

- Bitcode reader/writer; ThinLTO (summaries, import/export, internalization);
  fat LTO ships earlier via own IR serialization.
- PGO instrument/use; coverage mapping + pure-Rust profile runtime
  (evaluate `minicov` lineage). Sanitizers stay out of scope (runtimes are
  their own project).

### T4 — Targets and platforms

- aarch64-darwin (Mach-O via `object`, compact-unwind is the hard part),
  riscv64, wasm32, i686. Windows/COFF/SEH deferred until demand.

### T5 — Drop-in conformance

- Option A: stock rustc builds against `llvm-c-abi` + shim `llvm-config`,
  testsuite green ⇒ fully bootstrapped pure-Rust rustc.
- safety/oxidized/nixpkgs exposes `llvmPackages_rs`; monorepo README flips the Compiler
  Framework row from 💣 to 🦀.

## 9. Nix and monorepo integration

### 9.1 `safety/oxidized/llvm/default.nix` (flakelight module, safety/oxidized/gcc + fe-c pattern)

```nix
{
  packages.oxidized-llvm = {lib, ...}:
    lib.buildCargoProject {
      pname = "oxidized-llvm";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
        ];
      };

      index = ../../../platform/nix/lib/cargo/index;

      rootAttrs.postInstall = ''
        # Upstream-compatible tool names, like safety/oxidized/gcc does for gcc/cc
        ln -s $out/bin/opt $out/bin/opt-rs
        ln -s $out/bin/llc $out/bin/llc-rs
      '';

      meta = {
        description = "LLVM-compatible compiler infrastructure written in Rust";
        license = lib.licenses.asl20-llvm;
        mainProgram = "llc";
        platforms = lib.platforms.linux ++ lib.platforms.darwin;
      };
    };

  # rustc backend dylib + matched-nightly wrapper as separate outputs,
  # mirroring how fe-c splits cargo-fe-c / fe-c-driver / cementite.
  # checks.llvm-* wired here, one derivation per check (mirror fe-c's
  # default.nix wiring; heavy differential/oracle suites are their own
  # opt-in checks so nothing OOMs).
}
```

### 9.2 Wiring checklist

- `flake.nix`: add `./safety/oxidized/llvm` to `imports`, alphabetically between
  `./safety/oxidized/help2man` and `./safety/oxidized/make` (note: `safety/oxidized/libc` exists in-tree but is
  deliberately not in the imports list today — don't cargo-cult that; llvm
  ships packages and checks, so it belongs in the list).
- `platform/nix/lib/cargo/index`: add `regalloc2`, `object`, `gimli`, `target-lexicon`,
  `cranelift-isle`, `iced-x86`, `filecheck`, Mollusc crates, `hashbrown`,
  `smallvec`, `indexmap`, `rayon` (+ transitive closure). Do this as its own
  commit — the index is the known landmine field.
- `rust-toolchain.toml`: one pinned nightly, components
  `rustc-dev`, `rust-src`, `llvm-tools` off; bump via the same automated
  pipeline cadence fe-c uses.
- Oracle pins: real LLVM + alive2 come from nixpkgs *inside check derivations
  only*; record the oracle LLVM version in `STATUS.md`.
- CI: checks run on Spindle (`.tangled/`);
  tier-gating checks (`llvm-t0-hello`, `llvm-ui-ratchet`, …) are the pipeline.
- Root `README.md`, two edits:
  - Projects → Rust table row:
    `| [LLVM-rs 🦀](https://tangled.org/@overby.me/overby.me/tree/main/safety/oxidized/llvm) | LLVM-compatible compiler infrastructure written in Rust |`
  - 🦀 Systems → Compiler Framework row: add
    `[LLVM-rs 🦀](…/rust/llvm)` under Research & Development next to Cranelift
    and Krabby; move to Current when T2 lands.
- Commits: `feat(safety/oxidized/llvm): …` conventional style, `Signed-off-by`, jj-native.
- Project files: `CLAUDE.md` (hard rules + the ranked `llvmrs-todo` queue from
  §6.1), `STATUS.md` (tier, pass rates, oracle/nightly pins), `.deslop.toml`.

## 10. Relationship to sibling projects

- **safety/oxidized/gcc**: has working x86-64/i686/ARM64/RISC-V emitters, an assembler and
  a linker (ccc lineage). Two moves to evaluate at T1: (a) lift its ARM64/RISC-V
  encoders into shared crates under safety/oxidized/llvm, (b) longer-term, retarget
  safety/oxidized/gcc's frontend at `llvm-ir`, making it the clang-analog and doubling our
  frontend test pressure (unlocks csmith differential fuzzing for free).
- **safety/oxidized/binutils + wild**: wild links our output from T0; safety/oxidized/binutils
  provides objdump-alike tools for debugging emitted objects.
- **safety/oxidized/libc**: the substrate that makes "pure Rust" mean whole-process, same
  as fe-c's pairing.
- **safety/oxidized/nixpkgs**: consumer of the T1 toolchain bundle and the T5
  `llvmPackages_rs`.
- **dev/mojo/**: MLIR compat is the only path to serving Mojo; out of scope through
  T3, tracked as a possible post-T5 direction.
- **krabby**: if it ever wants codegen, `llvm-ir` is a natural target; no
  coupling planned.

## 11. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Sheer scale / stall risk | Tier structure where T1 alone is a monorepo win; ranked todo queue keeps agent/human work always unblocked |
| Miscompilations at O2 | Alive2 gate per pattern, differential execution in CI, rustlantis, conservative pass set, per-pass `-mllvm`-style kill switches |
| ABI corner cases | abi-cafe in CI from T1; data layout strings byte-identical to real LLVM |
| Intrinsic explosion (`core::arch`) | Generate lowering tables from stdarch metadata; soft fallbacks first; per-intrinsic conformance tests vs cg_llvm |
| Inline asm breadth | Ship the common subset early, hard-error loudly otherwise; iced-x86 carries x86 |
| rustc nightly churn on the fork | Kani model: one pin, scripted bumps, re-vendor cg_llvm per bump; drift is a scheduled chore, not a surprise |
| `LLVMRust*` churn (Option A) | A is a conformance target, not the dev path; only chase it at T5 |
| ISLE impedance mismatch | Time-boxed T0 spike; hand-written matchers are an acceptable fallback |
| Compile-time regressions | O0 speed benchmarked vs cg_clif and cg_llvm continuously; arena/index IR design from day one |
| Vendored-index friction | Dedicated index commits; minimize dependency count; no C/C++ build-script deps allowed at all |

## 12. Licensing

- Project license: **Apache-2.0 WITH LLVM-exception** (upstream-compatible, so
  `corpus/upstream/` vendoring and any future two-way flow are clean). This
  diverges deliberately from safety/oxidized/gcc's CC0 — vendored LLVM tests and the
  vendored `rustc_codegen_llvm` (MIT/Apache-2.0) can't be CC0.
- `crates/rustc-codegen-llvmrs` keeps rustc's MIT/Apache-2.0 headers;
  `corpus/upstream/` keeps LLVM's headers; everything is arm's-length clean-room
  from C++ LLVM *code* (reading LangRef/tests: yes; porting C++: no).

## 13. First two weeks (concrete)

1. Scaffold `safety/oxidized/llvm/` per §4.1; land the flakelight module + flake import +
   cargo-index additions as three separate commits; `llvm-fmt`/`llvm-clippy`/
   `llvm-unit` checks green in Spindle.
2. Vendor `rustc_codegen_llvm` at the pinned nightly; make it *compile* against
   stub `llvm-ir` handles (everything `todo!()`); wire the panic-grep check that
   emits the ranked todo queue into `CLAUDE.md`.
3. Implement `llvm-ir` core + parser/printer/verifier for the subset appearing
   in `rustc --emit=llvm-ir` of a seed corpus (hello world, core smoke tests);
   land `llvm-roundtrip` + `llvm-verify-corpus` checks with real `opt
   -passes=verify` as the differential oracle.
4. Time-boxed ISLE spike on x86_64 add/load/store/ret; decide ISLE vs
   hand-matchers; record the decision in `docs/`.
5. Drive `llvm-t0-hello` to green: `-O0`, `panic=abort`, ELF out through
   `object`, linked with wild, executed, output diffed against cg_llvm.
6. Write `STATUS.md` in fe-c's register: what works, what's a stub, what claim
   is *not* being made yet.
