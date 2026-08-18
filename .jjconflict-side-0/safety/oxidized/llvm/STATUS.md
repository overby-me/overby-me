# Status

What exists, what is a stub, and what claim is *not* being made yet. Written in
the register [safety/fe-c](../fe-c/STATUS.md) uses: a sentence here is either
backed by a check that passes or is marked unmeasured.

**Last updated:** 2026-08-16.
**Tier:** T0 (PLAN.md §8), in progress.

## Pins

| Thing | Value | Notes |
| --- | --- | --- |
| IR dialect | LLVM 21 | The major that `nixpkgs.llvm` (21.1.8) and the repo's rustc both speak. Older dialects are refused, not half-supported. |
| Oracle LLVM | `pkgs.llvm` 21.1.8 | Test-only, inside check derivations. Never a build or runtime dependency of any package output. |
| Rust toolchain | stable, whatever the devshell ships (1.95 today) | No `rust-toolchain.toml` yet on purpose: nothing here needs nightly. The pin arrives with `crates/rustc-codegen-llvmrs` (task B5), which needs `rustc_private`. |
| Third-party crates | none | The whole workspace has zero dependencies, so there is nothing to add to `platform/nix/lib/lib/cargo/index` yet. |

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
| Textual printer | done, byte-identical to upstream's `opt -S` over the corpus | `llvm-roundtrip` |
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
| Builder API: types inferred, alignments filled in | done for the common instructions, unwinding, attributes and metadata | `llvm-builder-smoke` |
| Per-intrinsic attributes, replacing whatever a declaration wrote | done for the 11,785 shapes upstream answers about, one row per name, arity and whether it ends in `...`, swept from LangRef and from its own tests | `llvm-opt-differential`, `llvm-upstream-verifier` |
| Positions of an intrinsic that share one overloaded type have to agree | done for the 161 LangRef documents more than once | `llvm-upstream-assembler`, `llvm-upstream-verifier` |
| Target extension types: size, global, alloca, zeroinitializer, vector element | done for the names upstream's tests mention | `llvm-upstream-verifier` |
| Intrinsic names carrying the types they were instantiated at | done for the 2,127 measured, name and print position both, swept from LangRef and from upstream's own test declarations | `llvm-opt-differential-other`, `llvm-opt-differential-feature`, `llvm-roundtrip` |
| Building a declaration for an undeclared call to an intrinsic upstream knows | done for the 1,790 names used undeclared, plus the 11,792 upstream recognises in a declaration | the eleven tree checks, `llvm-upstream-assembler` |
| A `DICompositeType` with an identifier made `distinct` and uniqued under that identifier | done | `llvm-opt-differential`, `llvm-opt-differential-linker`, `llvm-roundtrip` |
| The data layout a triple implies, filled in when a module writes none | done for the 679 triples upstream's tests name that imply one | `llvm-opt-differential-linker`, `llvm-opt-differential` |
| The ThinLTO summary index read, verified against the module, and not written back | done, which is what upstream's `opt -S` does with one | `llvm-opt-differential`, `llvm-verify-corpus` |
| An intrinsic read under an older name and written under the current one | done for the 120 renames upstream's own tests show | `llvm-opt-differential` |
| Implied declarations printed in upstream's order, sorted by the name as written | done | `llvm-opt-differential` |
| `DIObjCProperty` holding its setter under `getter` and its getter under `setter` | done | `llvm-opt-differential`, `llvm-roundtrip` |
| `memory(...)` in one shape however it was written: fixed location order, defaults dropped | done | `llvm-opt-differential`, `llvm-roundtrip` |
| A composite type's `baseType` written after `line`, and its `runtimeLang` as a language word | done | `llvm-opt-differential`, `llvm-roundtrip` |
| A `DIMacroFile`'s `type` read and never written back | done | `llvm-opt-differential`, `llvm-roundtrip` |
| One written intrinsic name implying a declaration per instantiation it is called at | done | `llvm-upstream-assembler`, `llvm-roundtrip` |
| A call read as an instruction rather than as a call | done for the four `llvm.nvvm.atomic.load.*` upstream's tests exercise | `llvm-upstream-assembler`, `llvm-roundtrip` |
| `!DIExpression` opcodes held as numbers, checked against what upstream reads, and written back as the words it writes | done for the 103 operations it reads, out of the 189 codes it has a word for | `llvm-upstream-verifier`, `llvm-upstream-assembler`, `llvm-roundtrip` |
| Two spellings of one intrinsic merged into the function upstream keeps | done | `llvm-opt-differential` |
| A call written before its intrinsic gained a parameter given the argument upstream gives it | done for the 92 declarations upstream's tests write at an older arity, and the 6 it drops outright | `llvm-verify-corpus`, `llvm-opt-differential` |
| A `DIDerivedType`'s pointer authentication packed into the slot `align` uses, spelled by the tag | done | `llvm-opt-differential`, `llvm-roundtrip` |
| The parameter shape a target extension type insists on | done for the 3 of 38 names upstream's tests spell that have one | `llvm-upstream-assembler` |
| Positions of an intrinsic that share one lane count without sharing a type | done for the 103 LangRef documents more than once at more than one width | `llvm-upstream-verifier` |
| A call inside a funclet carrying the token the intrinsic it calls needs | done for the 25 names that need one, out of 21,452 measured | `llvm-upstream-verifier` |
| The order a specialized node writes its fields in | done for every field its kind can be written with, ten of which no probe carried until now | `llvm-opt-differential`, `llvm-roundtrip` |
| An intrinsic found by the longest documented prefix of its name | done, which is what makes `llvm.objectsize.i32.unnamed` an `llvm.objectsize` | `llvm-opt-differential` |
| The members of an ODR type uniqued under it, keyed on the tag and name or on the linkage name | done | `llvm-opt-differential`, `llvm-opt-differential-linker` |
| The Swift version read out of the Objective-C collector flag it was packed into | done | `llvm-opt-differential-linker` |
| Debug info upstream cannot make sense of stripped rather than refused | done for 4 of the 14 rules `llvm/test/DebugInfo` shows, and no module stripped that upstream keeps | `llvm-opt-differential-debuginfo` |
| An attribute set printed in upstream's order rather than the written one | done for the 100 keywords, measured pairwise both ways round | `llvm-opt-differential-debuginfo`, `llvm-roundtrip` |
| An attribute group number defined twice keeping the last definition | done, and nothing merged | `llvm-opt-differential-debuginfo` |
| Named types found where upstream's type finder finds them, metadata included | done for the four places that reach one and the four that do not | `llvm-opt-differential-debuginfo`, `llvm-roundtrip` |

## The round trip

Every file in `corpus/` is canonical upstream output, and parsing one and
printing it back reproduces it byte for byte. As of 2026-07-27 that is 14
files and roughly 4,200 lines: 11 generated from real `rustc --emit=llvm-ir`
(arithmetic, control flow, memory, atomics, calls, unwinding, debug info,
optimised code, statics, enums, inline assembly) and 3 hand-written to pin
syntax rustc never emits (module structure, one of every instruction, one of
every constant and type form).

This is a stronger property than "the parser accepted it". It says we agree
with upstream about slot numbering, predecessor order, blank lines, label
padding, which defaults print as nothing, and the several places where the
same attribute is spelled differently depending on where it sits.

The files are `llvm-as` then `llvm-dis` then `opt -S`, and the last step is
not decoration. `llvm-dis` output is not a fixed point of the *textual*
reader, which is what this project implements: upstream's own `opt -S` on a
corpus file changes it, in twelve lines across the whole corpus, all of them
one rule about `DICompositeType`. Canonicalising without that step would ask
our printer to reproduce output upstream's `opt -S` does not produce.

The debug-info seed is the one that took work. Every other seed is built with
`-Cdebuginfo=0`, and switching it on found three printer bugs at once: a
local named in a `#dbg_declare` printed as `%<badref>` because the metadata
printer could resolve slots and not names, a debug record is indented four
spaces rather than two, and a specialized node's references are numbered in
the order it stores them rather than the order it prints them. The last of
those is `corpus/md-operand-order.nu`, which measures the storage order one
node kind at a time; eight of the eighteen kinds it probes differ from their
printed order, all of them by holding `file` before `scope`.

## Conformance against upstream's suites

Measured, not claimed. Every `.ll` file in the suite is run through our
`opt -S -passes=verify` and through real `llvm-as`, and the two agree when
they reach the same verdict about whether the file is a module. Nothing is
skipped, so the denominator is the whole suite.

| Suite | Agreed | Files | Refused but valid | Check |
| --- | --- | --- | --- | --- |
| `llvm/test/Assembler` | 483 | 483 | 0 | `llvm-upstream-assembler` |
| `llvm/test/Verifier` | 327 | 328 | 0 | `llvm-upstream-verifier` |

## Conformance against real IR

Those two suites are written to exercise a parser, which makes them a good
oracle and a poor sample. The rest of `llvm/test` is the opposite: thirty-eight
thousand modules written to exercise passes, backends, linkers and debuggers,
in whatever syntax was convenient at the time. Each bound is one-sided,
because there is nothing to trade against: reading a module upstream reads is
right in every case.

| Tree | Read | llvm-as reads | Check |
| --- | --- | --- | --- |
| `llvm/test/CodeGen` | 22,785 | 22,785 | `llvm-tree-codegen` |
| `llvm/test/Transforms` | 10,305 | 10,305 | `llvm-tree-transforms` |
| `llvm/test/Analysis` | 1,403 | 1,403 | `llvm-tree-analysis` |
| `llvm/test/DebugInfo` | 1,101 | 1,101 | `llvm-tree-debuginfo` |
| `llvm/test/Instrumentation` | 508 | 508 | `llvm-tree-instrumentation` |
| `llvm/test/Linker` | 338 | 338 | `llvm-tree-linker` |
| `llvm/test/ThinLTO` | 260 | 260 | `llvm-tree-thinlto` |
| `llvm/test/Other` | 160 | 160 | `llvm-tree-other` |
| `llvm/test/MC` | 160 | 160 | `llvm-tree-mc` |
| `llvm/test/Bitcode` | 232 | 232 | `llvm-tree-bitcode` |
| `llvm/test/Feature` | 82 | 82 | `llvm-tree-feature` |

That is every one of the 37,817 modules llvm-as reads across eleven trees.
The last gap was 483 modules and one error, a call to an intrinsic whose
name we did not recognise, which is what `corpus/intrinsic-recognised.nu`
closed. Each of these stays a ratchet at its full count, so a change that
loses a module fails rather than passing quietly.

The first sweep read 2,781 of the first 2,992 and the gaps it showed were not
the ones the suites show. Four fixes closed 110 of them: the attribute
spellings that predate `memory(...)`, a phi carrying a `!dbg`, an integer
literal past its type's width (upstream truncates rather than complaining),
and the `u0x` and `s0x` forms for an integer too wide to write in decimal.
The gap after those was one thing: 375 of the 452 remaining files called an
intrinsic without declaring it. That is now read the way upstream reads it,
by building the declaration from the call for any `llvm.*` name LangRef
documents, and appending it after everything the module writes. The
attributes upstream gives an intrinsic go on it too, from a table LangRef
does not document and the assembler does: `corpus/intrinsic-attributes.nu`
writes out each `declare` line and reads back the set upstream replaced it
with, 370 intrinsics of them.
Doing it exposed four verifier rules that had been unreachable, and all four
are real, which is why both suite ratchets ended up better than they started
rather than worse.

Assembler agreement fell from 454 to 447 when the use-list order directives
started parsing: eighteen of that suite's files are negative tests for them,
seven of which check the indexes against a use list this does not build.
Refusing every such module had scored as agreement for a reason that had
nothing to do with what those files test. Ten of them are closed now, and
none of it needed the def-use chains the note above assumed: a use *count*
answers a directive, and both a value's and a block's can be read straight
off the assembler, which takes a directive only when its index count
matches the list.

The two halves of the gap are not equally bad, so each suite has two
bounds. We **refuse nothing llvm-as reads**. That count is the failure that
matters and it is a ceiling that may only fall; it is at nought in both
suites now, and the Assembler suite agrees on every one of its 483 files.
We **read 1 module llvm-as refuses**, in Verifier, which is a missing
verifier rule, and agreement is a floor that may only rise. Everything
left in these two suites is on that side.

**The eleven tree ratchets are at 100%**: every module `llvm-as` reads
across `Transforms`, `Analysis`, `CodeGen`, `DebugInfo`, `Instrumentation`,
`Linker`, `ThinLTO`, `Other`, `MC`, `Feature` and `Bitcode`, which is 37,817
of them, we read too. The last gap was 483 modules failing at one thing, a
call to an intrinsic whose name we did not recognise, and
`corpus/intrinsic-recognised.nu` closed it.

Most of what is left on the second count is one thing: upstream knows what
each intrinsic means and we know only what LangRef's `declare` lines say
it takes, plus what the assembler will say when asked. All three halves of
that were built and measured.
`corpus/intrinsic-names.nu` harvests the 421 base names LangRef documents;
auto-declaring an undeclared intrinsic on that basis fixes three of the
modules we refuse and costs eight new wrong acceptances, because the parse
error it removes was standing in for the signature check, so it is a
script and not a table. `corpus/intrinsic-recognised.nu` answers the same
question where LangRef cannot, upstream knowing 1,790 names that it does
not document: the coroutine and exception-handling intrinsics are
documented in other files, `llvm.vector.interleave4` in none, and every
target's in its backend. A name used in a file `llvm-as` reads, where the
file itself never gives it a body, is a name upstream recognised, which
makes the exit code the whole oracle and needs no probing. `corpus/intrinsic-signatures.nu` harvests the
signatures from the same lines, 314 intrinsics, recording a position only
where its type is the same in every documented instantiation.
`corpus/intrinsic-attributes.nu` asks the assembler rather than LangRef,
writing each `declare` line out and reading back the attributes upstream
replaced them with. It reads LangRef's 1,803 lines and the 40,636 that
upstream's own tests write, which is 11,785 shapes: LangRef documents
370 of them and every target's are documented only in its backend. The same
readback says which names upstream knew, `; Unknown intrinsic` being what it
writes above one it does not, and that is the half
`corpus/intrinsic-recognised.nu` cannot see, since a name every test declares
for itself is never used undeclared.
`corpus/intrinsic-overloads.nu` reads the same lines a fourth way, comparing
the positions to each other rather than to a fixed type: two whose types
vary *together* across every documented instantiation are one overloaded
type, so 161 intrinsics know which of their positions have to agree.
`corpus/intrinsic-mangling.nu` asks a fifth question, which is about the
name rather than the signature: an overloaded intrinsic carries the types
it was instantiated at, so `llvm.umax` at `i8` is `llvm.umax.i8` and a
module that writes the shorter name is one upstream renames. Writing a bare
`declare` and reading back the name upstream gave it says which positions
go in, for 239 intrinsics, and every row is held against the 37,134
intrinsic declarations in `llvm/test`: 986 of the names those write are
ones upstream rewrites the way we would, and the three rows a test
contradicted are dropped. What a type spells is measured separately,
through the one intrinsic overloaded on any type at all, and is
`crates/llvm-ir/src/intrinsic/mangle.rs`. Where a renamed declaration
prints is measured too: upstream builds a new function and erases the old,
so it lands after everything the module wrote and after the declarations
the calls implied, and the attribute groups follow it.

That table moves neither ratchet and is in the tree anyway. What it
catches is a module that declares an intrinsic *consistently* wrongly, so
the call matches its own declaration and only LangRef knows better;
upstream's suites contain no such module and a compiler reading real IR
will meet one. What it cannot reach is the rest of the gap, because those rules are prose
rather than types. Those are being written one at a time instead, keyed on the base name the
table knows how to find: `llvm.bswap` swapping a whole number
of byte pairs, a masked access taking an alignment that is one,
`get_active_lane_mask` producing a mask of `i1`, `llvm.ptrmask` masking a
pointer, `get.vector.length` asking for a factor above zero,
`get.dynamic.area.offset` producing a scalar integer, `vector.splice`
indexing inside its own vector, `vector.extract` and `vector.insert`
starting at a multiple of the subvector's length, `llvm.stepvector`
stepping through lanes of at least eight bits, and the intrinsics that
reach through a pointer needing an `elementtype` to say what they reach. Checking the argument *count* was
tried and reverted, since upstream auto-upgrades the older spelling of an
intrinsic and demanding LangRef's arity cost two files.

Two bounds rather than one, because agreement alone can be gamed. Refusing
a module for a rule that does not exist scores as agreement whenever
upstream refuses it for another reason, so deleting that wrong rule lowers
the agreement count while making the parser more correct. That is exactly
what happened when five such rules came out at once: Verifier agreement
fell 215 to 212 while the modules we wrongly refuse fell 53 to 45. Without
the second bound the ratchet would have argued for keeping the bugs.

The typed-pointer spelling used to be refused here on purpose and is now
read the way upstream reads it, by folding `i8*` to `ptr` at parse time. The
model never holds a typed pointer either way; what changed is that a module
written in the older spelling is a module rather than an error, which is
what `llvm-as` says it is.

Two derivations feed the metadata schema, both measured against upstream
rather than reasoned about: `corpus/md-required-fields.nu` says which fields
a node cannot be written without, and `corpus/md-field-defaults.nu` says
which are dropped when written at their default.

`corpus/intrinsic-arity.nu` asks what upstream makes of a declaration written
at an older arity, which is the seventh reading of the same `declare` lines:
the call is given the probe function's own parameters rather than constants,
because an argument upstream works out of a constant is indistinguishable
from one it synthesises, and because a call on constants folds away and reads
exactly like one upstream removed. 92 declarations it reads at an older
arity, 6 it drops outright, and 638 it rewrites into other instructions,
which is a transformation rather than a table and is reported rather than
guessed at.

`corpus/dwarf-expression.nu` does the same for what a `!DIExpression` holds,
and it asks two kinds of question. Validity is the exit code of
`opt -S -passes=verify` on a two-line module. The word and the arity come
off upstream's own output instead, because it writes an opcode as a word
only for an expression it finds valid: a register operation ends its
checking, so `DW_OP_reg0` in front of any code makes that code answer
whether or not it would verify. Every `DW_OP_*` word written anywhere in
`llvm/test` is either in the table or one the assembler refuses, which the
script checks by asking it rather than by assuming its sweep went far
enough.

A third asks the assembler about the intrinsics rather than the metadata.
`corpus/intrinsic-attributes.nu` writes out every `declare` line LangRef
documents and reads back what upstream replaced it with, which is the whole
per-intrinsic attribute set: 370 intrinsics, 27 distinct function attribute
sets, and the `immarg` positions that had been a separate blocker. The
derivation had been recorded as impossible four times, on the grounds that
LangRef writes an attribute on only fourteen of its eight hundred `declare`
lines. That is true and it is the wrong question: LangRef has to supply the
signature, and the oracle supplies the attributes.

The oracle is `llvm-as`'s exit code, not its output. Those differ: some
verifier checks print a diagnostic and still return zero, so `set1.ll`
prints "invalid set base type" and is a module upstream reads. Reading the
message instead would invent rules that do not exist, which it did once
before the difference was noticed.

An earlier version of this measurement read each test's RUN lines to decide
what upstream would do with it. That was wrong often enough to matter: a
test that pipes `llvm-as`'s stderr into FileCheck is expecting a
diagnostic, and `Verifier` has 286 of those against the 74 that spell it
`not llvm-as`. Scoring those as "upstream accepts this" turned our own
wrong acceptances into agreement, and skipping a third of each suite hid
the rest. Asking the tool is both simpler and true. The numbers before the
change were 226 of 304 considered and 181 of 252 considered; they are not
comparable to these.

A third check asks a different question: not whether we accept the same
files, but whether we print the same text. For every Assembler file both we
and upstream accept, `llvm-opt-differential` compares our `opt -S` output
against upstream's own `opt -S`, and **221 of 226** are identical, with
four more suites measured the same way: **71 of 71** in `Feature`, **220 of
220** in `Linker`, **144 of 144** in `Other` and **57 of 57** in
`DebugInfo`. Four of the five print every module both tools accept exactly
as upstream prints it; the one that does not has its five remaining files
sorted by cause in CLAUDE.md. Some of the
remaining differences are ones where we already match
`llvm-as | llvm-dis` and `opt -S` does something else. ODR type uniquing is most of them; the
clearest is `!DIObjCProperty`, where `opt -S` prints the setter and the
getter swapped and `llvm-dis` does not. The corpus is `llvm-dis` output
and is the headline property, so where the two upstream tools disagree
this follows `llvm-dis`. Two
path-derived lines are normalised away, because upstream regenerates the
ModuleID from whatever path it read and synthesises a `source_filename` when
the file has none; the corpus round trip pins both fields properly against
files that carry them.

The gap is a to-do list, and `docs/dialect-notes.md` groups the recurring
reasons. The ratchets are in `default.nix` and only ever move up. The suites
earned their place on the first run by finding a parser hang that the corpus
never triggered.

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
- **No debug info modelling.** Debug-info nodes are modelled syntactically
  and nothing knows what a `DICompileUnit` field *means*. What that costs is
  narrower than it was: which fields drop at their default, which are stored
  even when they drop, the order they print in and the DWARF vocabularies
  behind them are all measured against upstream rather than reasoned about,
  so the nodes print back the way upstream prints them. A `DIExpression`'s
  opcodes are measured the same way: which numbers upstream reads as
  operations, how many elements each takes, and the four rules beside that
  table, so an expression it refuses we refuse and one it writes back as
  words we write the same. What is still missing is everything that needs
  the semantics: rewriting an expression written for an older dialect, and
  filling a compile unit's file in from a subprogram that names it. Both
  want the `llvm-debuginfo` crate at T1.
  Four rules of the half of the verifier that strips debug info rather than
  refusing a module are here, measured with the passing shape beside the
  failing one, and ten more that upstream applies in that tree are not: over
  the 1,093 modules of `llvm/test/DebugInfo` we and upstream both read,
  there is no module we strip that upstream keeps, and ten we keep that it
  strips.
- **No unwinding, no LTO, no PGO, no coverage.**
- **No C ABI.** `llvm-c-abi` is T5.

## Divergences from PLAN.md

Recorded so that the plan stays the plan and the deltas stay visible.

| Plan says | Reality | Why |
| --- | --- | --- |
| `rust-toolchain.toml` in the tree from the start (§4.1) | not present | Nothing in T0.1 needs nightly, and an unused pin is a maintenance cost plus a slower nix build. It lands with task B5, which is the first thing that needs `rustc_private`. |
| Use-lists as intrusive lists over indices (§4.2) | instruction storage is an arena, blocks hold `Vec<InstId>`, no use-lists yet; a use *count* is walked on demand for `uselistorder` | Def-use chains have no consumer before the first analysis pass. The one thing that did want them turned out to want only a count, for a value or for a block, and a walk answers that. Instruction ids are stable regardless, which is the property that matters for retrofitting. |
| Vendor upstream tests under `corpus/upstream/` (§7.1) | upstream tests are read from `pkgs.llvm.src` in check derivations | Same coverage, no third-party import into the tree, and the oracle version is pinned by the flake lock rather than by a copy that silently ages. |
