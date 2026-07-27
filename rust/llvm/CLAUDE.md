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

**B1a. [partial] Raise the ratchets.** *(2026-07-27: Assembler 453 of 483 with 13 wrongly refused, Verifier 285 of 328 with 4)*
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
A thirteenth pass took the odd corners of the text grammar: a metadata
name escapes what the bare grammar has no room for (`!\23pragma`), an
escape that is not one keeps its backslash (`!\xfoo` prints `!\5Cxfoo`),
a metadata attachment follows the same comma an index list uses, the
summary index writes a space before a colon and lets a word qualify the
value after it, and a block label can be named `"2"` or `-3`.
A fourteenth pass was fourteen verifier rules that need no intrinsic
table: an invoke's unwind edge lands on a pad, a gc barrier needs a
collector named on the function, `alwaysinline` contradicts `noinline`,
`signext` and its siblings mean nothing off an integer, `!fpmath` needs a
floating-point result, `!invariant.group` is only for loads and stores, a
`dllexport` symbol has to be visible, and four more quoted attributes
whose value upstream reads (`warn-stack-size`, `sign-return-address`, its
key, and `alloc-variant-zeroed`).
A fifteenth pass took five more parse features: `splat (i32 7)`, the
`ptrauth` constant, `/* */` comments, a brace-delimited operand list in a
metadata field, and an enumerator wider than 128 bits, which is kept as
written the way a data layout string is rather than being read.
As before, accepting the syntax exposed the rules behind it: an
unterminated block comment is an error rather than a run to the end of
the file, and each of a signed pointer's four operands says something
specific about its own type.
A sixteenth pass was sizedness and alignment. A struct holds scalable
vectors only when it holds nothing else, because mixing them leaves no
offset for whatever follows; an opaque struct has no layout at all; and a
global holds nothing scalable whether or not it is defined, while needing
a body to lay out only when it is. Each of those came from asking llvm-as
rather than reasoning: a test of my own invention claimed
`@g = external global %scalable_struct` verifies, and it does not, while
`@g = external global %opaque` does. Alongside them, an alignment is a
power of two wherever it is written, which needed the call site's own
argument attributes to be checked at all.
A seventeenth pass was what a call site and an attachment owe on their
own: `!mmra` belongs on something that touches memory, `!annotation` and
`!memprof` need an operand, `!noalias.addrspace` takes ranges of two, a
function carries one `!kcfi_type`, `llvm.va_start` needs a variable
argument list to start, `inalloca` marks the argument pushed last,
`speculatable` promises something about a function rather than one call to
it, and an indirect call has no declaration to name a token in so it may
not produce one.
An eighteenth pass took the rules that count things: a call carries one
bundle of each tag that says one thing, `allocsize` names parameters that
have to exist, `allockind` names exactly one of alloc, realloc and free,
an atomic store moves something the target can move in one instruction, an
ifunc has a linkage the loader can resolve, and seven more quoted
attributes take a word for true or false.
A nineteenth pass: every member of `llvm.used` names a symbol, because
keeping a null alive keeps nothing alive; a function's `!prof` node starts
with the annotation's name; a `musttail` call hands its frame over, so the
conventions have to agree; and a byval of four gigabytes is not something
a caller copies onto the stack.
A twentieth pass found one rule and unlearned two. A type is passed by
reference or by value and not both. Against that, "a composite type's
elements may not contain a null" and "every DICompileUnit is listed in
llvm.dbg.cu" both looked right, print a diagnostic from llvm-as, and were
measured as wrong. Some verifier checks warn without failing, so the
oracle reads the exit code and never the message.
The twenty-first pass found what the first two were missing. The elements
rule is real; what was wrong was applying it to a node nothing reaches.
Upstream verifies the debug info it can reach, and `set1.ll` leans on
exactly that, so `verify.rs` now walks metadata from its roots (named
lists, global and instruction attachments) and checks only what it finds.
Two more rules were tried and measured away in the same pass: `llvm.loop`
metadata may hold a `DILocation` after all, and the parameter attributes
on a variadic argument are legal in more shapes than the one test showed.
A twenty-second pass added three rules a call owes about itself: a
`callbr` has one `!` constraint for each label it can jump to, an indirect
inline-asm constraint reaches through an operand that carries
`elementtype`, and a rounding mode or exception behaviour written as
metadata is one of the six or three there are.
It also answered the intrinsic-table question with measurements instead of
a guess, twice. `corpus/intrinsic-names.nu` harvests the 419 base names
LangRef documents; auto-declaring on that basis fixes three refusals and
costs eight wrong acceptances, so the names are a script and not a table.
`corpus/intrinsic-signatures.nu` then harvested the signatures from the
same `declare` lines, 314 of them, recording a position only where its
type is the same in every documented instantiation. That table is in
`crates/llvm-ir/src/intrinsic_table.rs` and moves neither ratchet: what it
catches is a module that declares an intrinsic consistently wrongly, which
upstream's suites do not contain and real IR will. Checking the argument
count was tried and reverted, because upstream auto-upgrades the older
spelling of an intrinsic.
The rest of the intrinsic gap is not signatures at all, and a
twenty-third pass started writing it out by hand instead: `llvm.bswap`
swaps a whole number of byte pairs, a masked access takes an alignment
that is one, `get_active_lane_mask` produces a mask of `i1`, and
`llvm.ptrmask` masks a pointer. A twenty-fourth pass added five more:
`get.vector.length` asks for a factor above zero, `get.dynamic.area.offset`
produces a scalar integer, `vector.splice` indexes inside its own vector,
`vector.extract` and `vector.insert` start at a multiple of the
subvector's length, and the statepoint and the load-exclusive families
reach through a pointer whose pointee the type system no longer records,
so the call carries an `elementtype` that says what it is. A twenty-fifth pass added four more, and the count is
now thirteen rules across twenty-five files: a deoptimising call does not
come back so a return is the only thing that may follow it, a guard
carries exactly one deopt bundle, and anything passed by value needs a
type with a size and a size below four gigabytes.
A fifth was tried and reverted: whether a target extension type can be a
global is the target's business rather than the IR's, and upstream reads
`target("spirv.DeviceEvent")` while refusing `target("opaque")`.
A twenty-sixth added two more: `allocsize` names two different parameters,
and a matrix's two dimensions multiply out to the lanes of the vector
holding it.
Auto-declaring undeclared intrinsics was measured a second time now that
the prose rules exist, and it has moved from three refusals fixed for
eight wrong acceptances to three for six. Still reverted, and worth
re-measuring again as the verifier grows: the trade improves with it, and
rustc declares its intrinsics anyway, so the practical benefit is close to
nothing.
A twenty-seventh pass went at the Assembler suite's own wrongly-accepted
pile for the first time, which had been the larger of the two all along.
An `atomicrmw` operates on what its operation can operate on, and the
floating-point ones are the only ones a target does lane by lane. Every
cast but a bitcast works lane by lane, so both sides are vectors of the
same width or neither is. An address space is twenty-four bits. A symbol
cannot be imported from another image and local to this one at once.
Nine files, and the boundary between the atomic rules was found by asking
llvm-as which operations take a vector rather than by assuming they all
did.
A twenty-eighth pass stayed there for five more: `immarg` and `builtin`
describe a call site rather than a function or a result, a comdat clause
names a comdat the module declares, and an `insertvalue` writes something
the field it names can hold.
A twenty-ninth added three, and both of the first attempts were too
strict. An alloca's address space is its last clause, so the error is
writing something after it rather than writing it before the alignment,
which four files showed. A call's own `align` describes an argument rather
than the call, but its `preallocated(T)` does not, so banning every
type-valued attribute there cost a file. The alloca address space in a
data layout is twenty-four bits.
A thirtieth pass added five value rules and found a crash. A parameter
holds something a caller can pass, which a label and a function type are
not. `safestack` describes a frame rather than a value, so it belongs on
neither a parameter nor a result. A phi produces something a register can
hold. An empty `range` constrains nothing.
The crash was `%s = type { %s }` as a global: three of the sizedness
walks recursed through a named struct's fields without remembering where
they had been, and a type containing itself made them run until the stack
ran out. Each now carries a trail, and a type on it is unsized rather than
an abort. It was found by testing an upstream file rather than by
reasoning about the walk, which is the second time a crash has come out of
this suite.
A thirty-first pass added three: an integer is at most 2^23 bits wide, a
`cmpxchg` compares and stores the same type, and `uwtable` names one of
the two kinds of unwind table there are.
A thirty-second pass: a comdat keys on a name and a symbol with only a
number has none, a target extension type writes its types before its
integers, and a debug record has a fixed shape. The last of those needed
a correction the measurement caught: `#dbg_assign` carries two values, the
one assigned and the address it was assigned to, so checking that
everything after the first is metadata refused a file upstream reads.
A thirty-third pass: a name is defined once in a function body, and
`immarg` says an argument is written as a literal, which only an intrinsic
can require of its caller.
One rule was attempted and abandoned rather than measured: a named type
that nothing defines. `Context::named_struct` makes a forward reference
and a genuine `type opaque` the same thing on purpose, so there is nothing
to tell them apart with, and the rule needs a model change rather than a
check.
A thirty-fourth pass: an indirect call goes through a pointer in the
program's address space, and a metadata node does not hold metadata
wrapped in a value. The first was written for every call at first, which
refused `ifunc-program-addrspace.ll`, because a named symbol carries its
own address space and only a call through a value has to match the
program's.
A false positive from the thirty-fourth pass was found by probing rather
than by the suite, which does not cover it: a call may write the address
space it goes through, and then that is what the callee has to match
rather than the program's. `call addrspace(0)` under a `P42` layout is a
module llvm-as reads and we were refusing it.
A thirty-fifth pass: a struct that reaches itself by value has no size and
upstream says so where the type is defined rather than where it is used,
which is two files; a comdat is declared once; and a function attribute on
a parameter describes nothing. The recursion check walks fields and array
elements and stops at pointers, which is what keeps a linked list legal.
A thirty-sixth pass: `immarg` says the argument is written as a literal in
the call, so it cannot also carry an attribute that puts it somewhere,
and a vector reduction folds down to one of the vector's own lanes.
A thirty-seventh pass added two rules that move neither ratchet, because
upstream's suites do not contain either case: a `musttail` call hands its
frame over, so its caller returns what it returns, and an alias chain ends
at a symbol rather than coming back round to itself. Both were found by
writing the case and asking llvm-as, which is now part of how a rule gets
proposed rather than only how it gets checked.
A thirty-eighth pass, also from probing rather than the suites: an
`insertelement` inserts what the vector holds, and a `shufflevector`
shuffles two vectors of the same type. The second is a parser rule because
the model keeps one type for both halves, there being only one.
A thirty-ninth pass, again from probing: a load reads so it can acquire
and cannot release, a store writes so it can release and cannot acquire,
and a fence with an ordering that names no direction orders nothing.
A fortieth pass: a `select` picks lane by lane so its condition has as
many lanes as what it picks between, an alloca counts its elements once,
and a block is written once.
A fourth rule was written, measured, reverted, and then earned: every
block a terminator names is one the function defines. Enabling it refused
three files upstream reads, because a numbered block reference like `%1`
created a new block rather than resolving to the unnamed one holding that
slot. Fixing that fixed `%0` in a phi as well, which had been noticed and
left twice. An unnamed block now takes its slot through `block_by_name`,
which reuses the placeholder a forward reference already made instead of
shadowing it, and the rule went back in.
A forty-first pass took the unwinding rules, all five from probing. A
landing pad needs a personality routine to sort the exception and an edge
that lands on it, so a `landingpad` in a block no invoke unwinds to is
unreachable by construction; the same routine is what a `resume`, a
`catchpad` and a `cleanuppad` need. No pad opens the entry block, because
the entry block is reached by calling the function rather than by
unwinding into it, and a `catchswitch` hands to blocks that open with a
`catchpad`, that being what catching means.
With them, a `blockaddress` names a label its function defines. That check
waits for the whole module, because the function may not have been read
when the constant is built, and it looks at named labels only: matching
`%3` needs the slot numbers, which the printer works out and the verifier
does not have.
Still open, and each entry says what it is waiting on rather than only
what it is. Which argument of an intrinsic is `immarg` when the
declaration does not say so (four files): LangRef writes `immarg` in five
`declare` lines out of eight hundred, so there is nothing to harvest.
Uses of `llvm.used` and `llvm.global_ctors` (two files): needs the def-use
chains PLAN §4.2 puts off until the first pass that wants them. The DWARF
vocabulary, so that `DW_TAG_badtag` is refused (three files): needs a list
of every valid enumerator that no specification we may read enumerates.
`DIExpression` opcode sequences (three files): needs the stack discipline
of a DWARF expression, which is `llvm-debuginfo`'s at T1. Module flag
value shapes, and a handful of one-off rules that each cost more to state
than they return.
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

**B6. [done] Metadata text is bytes, not UTF-8.** *(2026-07-27)*
`llvm_ir::ByteString` is a `Vec<u8>` that compares equal to a `&str`, so
`attachment.kind == "prof"` still reads the way it did. `Token::Quoted`,
`Token::MetadataString` and `Token::MetadataName` carry bytes; the fields
that hold text from outside the compiler are `ByteString`
(`MdField::Str`, `Metadata::String`, `MdOperand::String`,
`NamedMetadata::name`, `MdAttachment::kind`); everything else validates
UTF-8 at the parser with an error that says so.
The line is drawn where the bytes come from: debug info carries file
paths, and a path is not text on every system. Symbol names, section
names, attribute keys and values, summary strings and block labels are
still `String`, and refusing a non-UTF-8 one is now a loud parse error
rather than a lexer accident. `docs/dialect-notes.md` records that.
Assembler 400 to 402, differential 116 to 117.

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
