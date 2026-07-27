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

**A2. [done] `llvm-support`: APInt, APFloat, DataLayout, Triple.** *(2026-07-26)*
Acceptance: unit tests including APInt cross-checked against `u128`/`i128`
native arithmetic over a wide operand grid, and datalayout strings from the
corpus round-tripping byte for byte.

**A3. [done] `llvm-ir`: types, constants, instructions, attributes, metadata.** *(2026-07-27)*
The data model expresses every construct in the corpus, which A4 and A5
demonstrate by round-tripping it.

**A4. [done] `llvm-ir-print`: textual printer with LLVM's slot numbering.** *(2026-07-27)*
Byte-identical to `llvm-dis` over the whole corpus.

**A5. [done] `llvm-ir-parse`: lexer and recursive-descent parser.** *(2026-07-27)*
Same check; every parse error carries a line and a column.

**A6. [done] Verifier.** *(2026-07-27)*
Structural, type, flag, signature and dominance rules. The corpus verifies
clean, which is a real check because real `llvm-as` verified every one of
those files, and a table of 18 deliberately broken modules is rejected with
the message each rule owns.

**A7. [done] `opt` with an upstream-compatible CLI subset.** *(2026-07-27)*
`-S`, `-o`, `-passes=`, `--verify-each`, `-` for stdin. `llvm-roundtrip`
drives the installed binary over the corpus, not just the library. An
unimplemented pass and bitcode output are both errors that say so.

### T0.2, conformance and the rustc seam

**B1. [done] Upstream `llvm/test/Assembler` and `llvm/test/Verifier` as an oracle.** *(2026-07-27)*
From `pkgs.llvm.src`, not vendored. The oracle is real `llvm-as`'s verdict
on each file, so every `.ll` in the suite counts and nothing is skipped.
It already found a parser hang.
The first version read each test's RUN lines instead, and that was wrong
often enough to matter: a test piping llvm-as's stderr into FileCheck is
expecting a diagnostic, and `Verifier` has 286 of those against 74 that
spell it `not llvm-as`, so our own wrong acceptances scored as agreement.
Numbers from before that change are not comparable to numbers after it.
The ratchets live in `default.nix` and only move up.

**B1a. [partial] Raise the ratchets.** *(2026-07-27: Assembler 398 of 483 with 23 wrongly refused, Verifier 212 of 328 with 4)*
Landed in two passes: duplicate symbols, alignment bounds, aggregate and
vector element types, linkage against visibility, cmpxchg orderings,
getelementptr and aggregate index rules, module flag and ident node shapes,
metadata values that are not node references, metadata attachments written
in place, non-struct type aliases, sized-type rules for alloca and globals,
token and x86_amx being intrinsic-only, and the shapes of `llvm.used` and
`llvm.global_ctors`.
A third pass went the other way, at false positives rather than gaps: four
rules rejected IR upstream accepts, which is worse than being permissive.
An undefined attribute group is an empty set, not an error; a call need not
match the signature its callee was declared with, because opaque pointers
put the signature at the call site; a load of a type with no computable
layout still has no alignment to report; and an instruction after a
terminator opens a new anonymous block rather than being a second
terminator. (The second of those was recorded as removed and was not: the
code was still there and still rejecting IR real llvm-as accepts. The
eighth pass removed it for real, and found the narrower rule it should
have been.)
A fourth pass took the reject side: numbered struct types, the deprecated
sized aggregate alignment, `align(4)` as well as `align 4`, a phi with no
edges, the three arithmetic constant expressions that outlived the others,
inline string attributes on a global, more calling conventions, an address
space written before a call's type, and metadata integers past 64 bits.
Fixing the layout parser exposed eleven files whose real rule was that a
bitcast may not change an address space or a size, which now applies to
constant expressions as well as instructions.
A fifth pass was one thing: the grammar of specialized metadata nodes, in
`crates/llvm-ir-parse/src/md_schema.rs`. Field names, repeated fields,
required fields, fields that may not be null or empty, numeric ranges and
the two nodes that have to be `distinct` are all upstream *parse* errors,
and modelling debug info syntactically had made every one of them silently
fine. That alone was 45 files, with nothing on the accept side lost.
A sixth pass was the Verifier's turn, and attributes were most of it: the
eleven that describe something only a pointer has, `nofpclass` on a type
with no float in it, a `range` whose width is not the width it constrains,
the five that may appear on only one parameter, `swifterror` on a return
value, `vscale_range` bounds, `jumptable` without `unnamed_addr`, and the
quoted attributes whose value upstream reads rather than carries
(`frame-pointer`, `denormal-fp-math`, `patchable-function-entry`). One
debug-info rule came with them, worth nine files on its own: a DWARF
address space says where a pointer points, so a typedef or a qualifier
cannot carry one.
A seventh pass took the symbol rules: a comdat needs a definition to pick
and a name the linker can see, an ifunc resolver has to name a function
through any number of casts but not through arithmetic, and `!associated`
and `!absolute_symbol` have shapes. Both of the last two were too strict
on the first try and cost an Assembler file each, which is why
`what_upstream_verifies_verifies` now exists: a table of modules that must
verify clean, next to the table of modules that must not.
An eighth pass took the rules an intrinsic owes without needing a table of
intrinsic signatures: it may not be defined, its address may not be taken,
an `immarg` parameter takes a literal (or a constant expression, which
upstream folds before the verifier looks) and not `undef` or an SSA value,
and `llvm.localescape` is called once per function. The signature rule
came back here in the shape it should always have had: an ordinary call is
not compared against its callee's declaration, but an intrinsic call is,
because an intrinsic is selected by its name and mangled suffix together.
A ninth pass was the debug-info rules that need a node's neighbours rather
than its own grammar: a subrange is described from one end or the other and
never both, a bound that is a node has to be a variable or an expression, a
generic subrange needs a stride, `rank`, `allocated`, `associated` and
`dataLocation` belong on an array, a discriminator belongs on a variant
part, and template parameters have to be a tuple of template parameter
nodes. A tenth rule was written and then deleted: upstream's own `set1.ll`
has a composite type whose `elements` is `!{null}` and llvm-as reads it, so
"no null entry" is not the rule it looked like.
A tenth pass went at the other half of the gap, the modules we refuse that
llvm-as reads, and most of what it found were rules of ours that should
not exist. Dominance says nothing about a block the entry cannot reach, so
`%x = add i32 %x, 1` in dead code is fine. A struct may hold scalable
vectors and a vector may hold a target extension type. `preallocated` may
appear on more than one parameter. Private linkage in a comdat is COFF's
rule, not the IR's, and upstream reports it only for a Windows triple. The
`s` datalayout specification is dead and still parsed.
Deleting those five cost three files of Verifier agreement, because a wrong
rule scores as agreement whenever upstream refuses the same file for a
different reason. That is why each suite now has a second bound: the count
of modules we refuse that llvm-as reads, which may only fall. Without it
the ratchet would have argued for keeping the bugs.
Implicit intrinsic declarations were tried and reverted. Upstream
materialises a declaration for an undeclared `llvm.*` name only when it
recognises the name; `@llvm.not.a.real.intrinsic` is still "use of
undefined value". Declaring every `llvm.*` name we see gained five files
and lost eleven. Doing it properly needs the base-name table, which is the
same table per-intrinsic signatures need.
An eleventh pass took the ThinLTO summary index, `^0 = module: (...)`,
which was the largest single parse gap at ten files. It is modelled
syntactically in `crates/llvm-ir/src/summary.rs`, the way specialized
debug-info nodes are, because the grammar is uniform all the way down: a
keyword, a value, and tuples of keyed or positional values nested to any
depth. Nothing reads what the keywords mean.
One verifier rule came with it, worth the eleventh file: a `gv` entry that
names a symbol has to name one this module has.
A twelfth pass cleared the scattered parse gaps: an alias writes an
expression aliasee with no type in front, so the expression has to say what
it produces; wrapping flags on a constant expression; the sanitizer clauses
a global carries; the AMDGPU shader conventions and `riscv_vls_cc(N)`; and
a struct indexed lanewise by a vector every element of which picks the same
field.
Four of those opened verifier rules that had been unreachable behind the
parse error, and all four are real: an alias needs something this module
defines, the kernel conventions return nothing and take no variable
argument list, a chain call is only ever a tail call, and the vector
indices of one getelementptr all have the same width. The last of those had
to reach constant expressions too, which is why the verifier now walks
every interned constant once rather than finding them again at each use.
Still open, largest first: per-intrinsic signatures (`bswap` on an odd
number of bytes, `masked_load` alignment, `get_active_lane_mask` element
type), which is the last big Verifier cluster and does need the table;
module summary index syntax (`^0 = ...`, nine files); uses of `llvm.used`
and friends (which needs def-use chains we do not build yet); the DWARF
vocabulary itself (`DW_TAG_badtag` and friends, three files, which needs a
list of every valid enumerator that no readable specification in the tree
provides), and `ptrauth` and `splat` constants.
Acceptance: both numbers up again, recorded in the same commit.

**B2. [partial] Differential check against real `opt -S -passes=verify`.** *(2026-07-27)*
Runs over upstream's Assembler suite rather than the corpus, because the
corpus is upstream's output already and would pass trivially. 100 of the 160
files we both accept print identically. Only the two path-derived lines are
normalised away; everything else counts.
Metadata uniquing landed, taking it to 105. The largest remaining cause is
debug-info field defaults: upstream knows what a `DICompileUnit` field means
and omits one left at its default, while we print back what was written.
That is T1's `llvm-debuginfo`, not a table of defaults bolted to the printer.
Acceptance: the number keeps rising.

**B3. [done] Surface inventory from `rustc_codegen_llvm`.** *(2026-07-27)*
363 entry points, 532 call sites, 125 of them `LLVMRust*` shims, measured
against rustc 1.95.0 from `nixpkgs#rustc.src`. Reproducible with
`nu docs/surface-inventory.nu <crate>`; the reading is in
`docs/surface-inventory.md` and STATUS.md cites the totals.
The useful shape: two thirds of the surface is the tier that already exists,
and the areas that are almost all `LLVMRust*` shims are the later tiers.

**B4. [done] IR builder API sufficient for the fork's call sites.** *(2026-07-27)*
`llvm_ir::builder::Builder`, phrased as safe Rust rather than as an FFI
mirror. It works out each instruction's result type from its operands and
fills in the alignment upstream would have computed, so built IR verifies and
prints exactly like parsed IR.
`llvm-builder-smoke` builds a loop that calls a declared function and
accumulates its result, then checks three things: it verifies, it prints the
text upstream prints for that module (hand-written, and confirmed by feeding
it to real llvm-as and llvm-dis), and parsing our own output back gives a
module that prints identically.
**B4a. [done] The rest of what B5 needs from the builder.** *(2026-07-27)*
invoke, landingpad, resume, switch, extractvalue, insertvalue, freeze;
attributes on a function, its return and its parameters; the personality
routine; metadata nodes, named metadata and instruction attachments.
`llvm-builder-smoke` now builds an unwinding function with all of it, and
that text was confirmed against real llvm-as and llvm-dis too.
It found a printer bug on the way: `!0 = !"text"` is not legal upstream,
because a string is an operand and never a numbered node. The parser used
to accept it, so we could emit something llvm-as would refuse.
Still not covered: debug info, which is T1's `llvm-debuginfo`, and the
funclet-based personality (catchswitch, catchpad, cleanuppad), which is
Windows and deferred with the rest of that target.

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
