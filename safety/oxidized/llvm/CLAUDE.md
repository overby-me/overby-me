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
| **Library crates stay dependency-free and stable-Rust for as long as that is free.** Adding a crate means a `platform/nix/lib/cargo/index` commit; adding nightly means a toolchain pin. Both are fine when a tier needs them, neither is free. | PLAN §11 lists vendored-index friction as a live risk, and today the whole workspace builds with zero third-party crates. |

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
changes, copy safety/fe-c's `vendorFor`.

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

**B1a. [partial] Raise the ratchets.** *(2026-07-27: Assembler 453 of 483 with 13 wrongly refused, Verifier 286 of 328 with 4)*
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
`crates/llvm-ir/src/intrinsic/table.rs` and moves neither ratchet: what it
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
A forty-second pass took the phi and the atomic, both from probing. A phi
has one entry per arrival rather than per predecessor, so `br i1 %c, label
%b, label %b` needs two and a switch with two cases going to one block
needs two, and two entries from one block cannot disagree about the value
because they arrive at the same time.
An atomic access moves one scalar the target can move in a single
instruction: a size that is a power of two and at least eight bits, which
is why `x86_fp80` cannot be moved atomically and `fp128` can, and a kind
that is an integer, a float or a pointer. `atomicrmw` takes the size half
of that rule only, its own operand check already saying which kinds each
operation takes, and a vector of floats is one of them: `<2 x half>` is
thirty-two bits and fine while `<3 x half>` is forty-eight and not. That
distinction came out of the table of modules that must verify clean, which
caught the first attempt refusing a vector outright.
An atomic load or store also writes its own alignment rather than taking
one from the data layout, which is a parse rule because the parser fills
the default in, the way upstream's does.
With them, a shuffle mask picks lanes out of the two operands laid end to
end, so `<i32 0, i32 4>` across two two-lane vectors picks nothing. A lane
that does not matter is written `undef` or `poison` rather than as an
out-of-range index, and a scalable mask is not checked because its length
is not known here.
A forty-third pass left the verifier alone and went at the corpus, which
had been six rustc seeds all built with `-Cdebuginfo=0`. Five more seeds:
debug info, optimised code, statics, enums, and inline assembly. Ten of the
eleven round-tripped on the first try. The eleventh, debug info, found three
printer bugs.
A local named in a `#dbg_declare` printed as `%<badref>`, because the
metadata printer could resolve a slot and not a name, and every local in a
debug record has a name. A debug record is indented four spaces rather than
two. And a specialized node's references are numbered in the order the node
stores them rather than the order it prints them: `DISubprogram` writes
`scope` before `file` and stores `file` first, so a subprogram whose file and
scope are both new gives the file the lower number, and numbering in written
order swapped those two and everything after them.
That last one is not in LangRef, which documents syntax rather than layout,
and this project does not read upstream's C++, so it was measured:
`corpus/md-operand-order.nu` builds one probe per node kind with every
reference field pointing at a node named after the field, and reads the
numbering back. Eight of the eighteen kinds differ from their printed order,
all by holding `file` before `scope`, and the table those eight became lives
in `crates/llvm-ir-print/src/md_slots.rs`.
The optimised seed needed `no_mangle` on every function: at `-Copt-level=2`
and above a plain `pub fn` in a library crate is internalised and then
deleted, and the first version of that seed came out holding nothing at all.
It needs one codegen unit for the same reason, `--emit=llvm-ir -o` writing a
single module while rustc splits an optimised crate across sixteen.
A forty-fourth pass went somewhere the project had not looked: the rest of
`llvm/test`. The two suites are written to exercise a parser, which makes
them a good oracle and a poor sample, and a sweep of three thousand
`Transforms` tests read 2,781 of the 2,992 that llvm-as reads. The gaps were
not the ones the suites show.
Four fixes closed 110 of them. The attribute spellings that predate
`memory(...)` are read and upgraded the way upstream does: `argmemonly` and
its two friends say where, `readnone`, `readonly` and `writeonly` say how,
and a function carrying one of each means the intersection, so
`argmemonly readonly` is `memory(argmem: read)`. The same three access
keywords stay as they are on a parameter, which is why only the function and
call-site positions upgrade. A phi may carry a `!dbg`, and the comma before
it looks exactly like the comma before another edge. An integer literal past
its type's width truncates rather than being refused, which is what
`store i16 65536` relies on and what `i1 2` reads as false. And `u0x` and
`s0x` write an integer too wide for decimal, the second of them reading its
digits as two's complement in the narrowest width that holds them, so `s0x1`
is -1 and `s0x10` is -16.
Three more parse gaps went with them: `alloca swifterror ptr`, a global
carrying a metadata node written in place rather than by number, and the
`!dbg` after a phi's last edge.
Accepting those exposed three verifier rules that had been unreachable
behind the parse error, and the Verifier ratchet dropped three before they
went in. A `llvm.experimental.noalias.scope.decl` declares one scope, being
the declaration of where one begins. And a `musttail` call in a `tailcc` or
`swifttailcc` function has no `inreg` anywhere, on the caller's parameters
or on its own arguments, because those conventions hand the frame over whole
and an argument in a register is not part of the frame.
The sweep is now `llvm-tree-transforms`, a third ratchet over all 10,328
files: 9,853 of the 10,305 that llvm-as reads. It has one bound rather than
two, because there is nothing to trade against.
What is left in that tree is led by intrinsics called without a declaration,
84 files in the sample. Auto-declaring was measured a third time, and the
answer is still no, for a new reason: upstream materialises the declaration
with the intrinsic's own attributes and parameter attributes, so
`@llvm.assume` comes back as `declare void @llvm.assume(i1 noundef) #0` with
`nocallback nofree nosync nounwind willreturn memory(inaccessiblemem: write)`
behind it. Reproducing that needs a per-intrinsic attribute table LangRef
does not document, and without it the module parses and prints back wrong,
which is worse than refusing it. On the suites the trade is five files
gained against nine wrongly accepted.
A forty-fifth pass answered the intrinsic-declaration question for the
fourth time, and this time yes. 375 of the 452 files the tree sweep still
refused called an intrinsic without declaring it, which is five times
everything else put together. Upstream builds the declaration from the call
for any name it recognises, and appends it after everything the module
writes, in the order the names were first used. So does this now, gated on
`intrinsic::table::signature` so that an undocumented `llvm.*` name is still
"use of undefined value". The ids have to be reserved in the pre-scan, next
to every other global symbol, or the pre-scan and the parse disagree about
which function is which.
What is still not built is the attributes upstream gives an intrinsic:
`@llvm.assume` comes back with five function attributes and an `i1 noundef`
parameter, from a table LangRef does not document. Those declarations print
back without them, which is a divergence, and a smaller one than refusing
the module.
The three earlier measurements said no because they scored the trade on the
suites alone, where it was five files gained against nine wrongly accepted.
The nine turned out to be nine rules, four of them worth writing here. A
three-way compare answers lane by lane in a result wide enough to hold three
answers, so `i1` is too narrow. A predicated cast casts lane by lane like
any other. And `llvm.threadlocal.address` takes the address of a thread's
copy of something, so its argument is a thread-local global, or an alias to
one, which is what upstream's own threadlocal-pass.ll leans on and what the
first attempt at the rule refused.
Every bound moved the right way: Assembler refusals 13 to 12, Verifier 286
to 287 with refusals 4 to 2, and the tree 9,853 to 10,102 of 10,305.
Then the gate widened. The signature table names 165 intrinsics, and the 126
files still refused were calling the other ones, `llvm.memcpy` among them.
`corpus/intrinsic-names.nu` had been harvesting LangRef's 419 base names into
a text file since the twenty-second pass, when the answer was no and a script
was enough; it now writes `crates/llvm-ir/src/intrinsic/names.rs`, next to it under one module, and the
gate takes either table. The tree went to 10,146.
That cost one file and paid for it: a naked function has no prologue, so
nothing put its arguments anywhere the body could read them, and reading one
is an error. Taking one and leaving it alone is not.
A forty-sixth pass took four more of the tree's parse gaps, and one of them
turned out to be a printer gap underneath. Tree 10,146 to 10,162.
A global's `align` with no comma before it is an attribute rather than the
alignment clause, which is what tells the two apart and what upstream prints
as `#0 = { align=4 }`. A NaN narrows to a `float` when the mantissa bits
that fall off the bottom are zero, which is why `0x7FF1000000000000` is a
float and `0x7FF0000000000001` is not; comparing bit patterns the way a
finite value is compared refused every NaN. Widening one back for printing
had the same shape of bug in reverse: the machine's own conversion quietens
a signalling NaN, and the payload is what says which NaN it was.
The fourth was a function attachment written in place, `!prof !{...}`, and
accepting it showed the printer keeping it there. Upstream has no inline
node at all except `DIExpression` and `DIArgList`: every other node is
numbered once and referred to, whether it was written inside an attachment
or inside another node. So the parser hoists them now, at parse time, into
numbers past every number the text writes. That fixed the global case from
the pass before as well, which had the same divergence and had not been
noticed because only the parse was measured.
A forty-seventh pass swept `llvm/test/Analysis`, 1,415 files that are older
on average than the pass tests and use the implicit numbering far more. It
read 1,387 of the 1,403 llvm-as reads, and four of the sixteen it refused
were not refusals at all: they aborted.
The bug was the value half of the one the forty-first pass fixed for blocks.
An instruction written without a `%N =` still takes the next number, and a
phi above it may already have made a placeholder for that number. Reserving
a second slot rather than reusing the placeholder left the phi pointing at
an instruction that never arrived. That is 1,391 now, and the tree is a
fifth ratchet.
A forty-eighth pass swept `llvm/test/CodeGen`, 23,311 files and by far the
largest tree, and took the classes it showed that the other two do not.
The largest is not a gap: 179 of the 644 files it refused write typed
pointers, which is the deliberate divergence of PLAN 1.2, and 410 more call
a target intrinsic no LangRef line names. Those are recorded rather than
closed. What was left came to four things.
A uniform aggregate folds, and this is a printer rule rather than a parse
one: all zero is `zeroinitializer`, all undef is `undef`, all poison is
`poison`, and a vector whose lanes are all the same is `splat (T v)`, which
an array of the same shape is not. Zero is checked element by element rather
than by identity, because a struct's fields need not be the same constant to
be all zero. That moved the differential ratchet, which no parse fix had
done.
`i16-1` is a type and a negative number rather than one word, because
upstream's identifiers hold no hyphen. Taking the hyphen out of the word
lexer broke six files in a way no unit test and neither suite caught: a
*label* may hold one, so `for.cond2thread-pre-split:` stopped lexing. The
run is scanned with hyphens and given back to the first one when no colon
follows, which is the only way to tell the two apart. The tree ratchets are
what caught that, which is the first time they have caught a regression the
suites missed.
Eight calling conventions the CodeGen tree uses and the suites do not, two
of which upstream prints with a doubled space, its own spelling of
`avr_intrcc` and `avr_signalcc` ending in one. And `hybrid_patchable`, which
llvm-as reads and llvm-dis cannot read back: it writes an attribute kind its
own reader rejects, so that one is pinned on the exit code alone.
A forty-ninth pass took four more of CodeGen's classes and the three rules
they uncovered. Transforms 10,166 to 10,174.
`[u0xedcba x i8]` writes a length the way a wide integer literal is written.
A backslash that begins no escape keeps itself in a string constant, so
`c"c:\temp"` holds a backslash and a `t`, which is the rule metadata names
already had. `x86_amx` may sit in a struct and in no other aggregate,
because an intrinsic returning two tiles returns them in one. And a
`DIArgList` holds SSA values, which meant threading the function being
parsed down through the metadata node parser, since that is the one place
inside metadata where a `%name` can appear.
Accepting the last of those cost three Assembler files before it paid for
them. A module writes its debug info as records or as calls to the
`llvm.dbg.*` intrinsics and upstream refuses one holding both, and there are
four kinds of debug record, so `#dbg_invalid` is a misspelling rather than a
kind this parser has not met.
A fiftieth pass closed that divergence rather than leaving it recorded.
Upstream reads a call to one of the four `llvm.dbg.*` intrinsics as the
record it is the older spelling of, takes the location from the call's
`!dbg`, and drops the declaration whether it was called or not. So does
this now. The declaration stays in the model, because a constant built
while parsing still points at it, and goes unprinted.
Doing it took the mixed-spelling rule out of the verifier and put it in the
parser, where it now has to live: once every call is a record, there is
nothing left to tell the two apart by, so the two spellings are counted as
they are read. Verifier 287 to 288, with the modules we refuse that llvm-as
reads down to one.
A fifty-first pass swept the eight trees nobody had measured: DebugInfo,
Instrumentation, Linker, ThinLTO, Bitcode, Other, MC and Feature. Seven of
them were already above 97 per cent, which is what the last four passes were
worth. The eighth was Bitcode at 65, and that is not a gap: 78 of its 81
refusals write typed pointers, the tree existing to test reading older
bitcode. It is now a ratchet of its own, so the cost of that decision has a
number that moves when the decision does.
Six real gaps came out of the other seven, and one of them was worth two
files on its own. `extern_weak` is a linkage rather than the `external`
keyword, and it declares just as `external` does, so a global carrying it has
no initializer; reading one anyway swallowed the next global and reported the
error a line late. A wide hexadecimal literal is a constant like any other. A
named metadata list may hold one of the two node kinds written at every use
rather than numbered, and may not hold any other node written in place. And a
metadata field may be an aggregate constant, `extraData: [4 x i32] [...]`.
Assembler 453 to 454 with refusals 12 to 11, Transforms to 10,177, CodeGen to
22,187, and eleven trees now have a bound: 36,526 of the 37,334 modules
llvm-as reads.
A fifty-second pass took the use-list order directives, which had been a
recorded non-divergence since the start and were the largest closable class
left: 45 files across five trees.
They turned out to need no model at all. A directive says what order a
value's uses were in, so that bitcode round-trips through the textual form
without reordering them, and `llvm-dis` prints none of them unless asked to.
Reading one and keeping nothing is therefore what reproduces upstream's
output; keeping them would print something upstream does not.
Transforms 10,177 to 10,207, ThinLTO 251 to 259, Analysis to 1,393, Bitcode
to 153.
It cost Assembler agreement, 454 to 447, and that is the trade the second
bound exists to make legible. Eighteen of that suite's files are negative
tests for these directives. Eight of them check things that need no use
list, and those are checked: `uselistorder_bb` names a function this module
has, with a body, and a block that function defines and that is named rather
than numbered; the indexes are distinct; and indexes already in order say
nothing. The other seven check the indexes against the use list itself, and
that needs the def-use chains PLAN 4.2 is still waiting on. Refusing every
such module scored as agreement for a reason that had nothing to do with
what those files test. The bound that matters went the right way, 11 to 8.
Two printer divergences surfaced while measuring, both unrelated to the
directives. An alias whose aliasee is a constant expression writes no type
in front of it, where a bare symbol does: `@b = alias i1, getelementptr (...)`
and `@b = alias i32, ptr @a`. That one moved the differential ratchet 123 to
128. The other is not closable: upstream's predecessor comments follow the
use list, so a module whose directives permute it lists its predecessors in
that permuted order, and reproducing that needs the same chains.
A fifty-third pass reversed a scope decision, having measured what it cost.
PLAN 1.2 said typed pointers were refused; 293 files across the test trees
wrote them, three quarters of `llvm/test/Bitcode` among them, and refusing
them bought nothing. `i8*` is not a dialect, it is a spelling: `llvm-as` in
LLVM 21 folds it to `ptr` as it parses, and nothing downstream of either
parser can tell the two apart. So this folds it too, in `parse_type_suffix`
and in the pointer-only variant a return type needs, and the model still
never holds a typed pointer. The plan now says what it always meant: not
pre-opaque-pointer *semantics*.
Six trees are read whole now: DebugInfo, Linker, ThinLTO, Other, MC and
Feature. Bitcode goes 153 to 229 of 232, Transforms to 10,223, CodeGen to
22,369, Analysis to 1,394, Instrumentation to 505. Assembler 447 to 449 with
refusals 8 to 5, Verifier refusals 1 to nought, and the differential 128 to
129. Across the eleven trees that is 36,824 of the 37,334 modules llvm-as
reads, up from 36,526.
A fifty-fourth pass finished what the fold started. `void ()*` is a pointer
to a function, and three places had to learn it: a return type reads the
parenthesised half only when a `*` follows it, which is the only thing that
tells a function type from a parameter list, and `ret void` returns nothing
only when nothing follows the word. Two more calling conventions, `hhvmcc`
and `hhvm_ccc`. Bitcode is read whole now, 232 of 232, and seven trees are.
The other recorded decision was measured and left alone. A target intrinsic
no LangRef line names cannot be auto-declared, which is 94 files outside
CodeGen and most of what CodeGen still refuses. Harvesting the names from
`declare` lines across `llvm/test` would cover 56 of the 68 distinct names
those 94 files call, which is a derivation from what upstream's tests happen
to declare rather than from what exists. The set that does exist is in
`llvm/include/llvm/IR/Intrinsics*.td`, which is neither C++ nor under
`llvm/lib`, and whether reading it is within the clean-room claim of PLAN 12
is the user's call rather than this loop's. The number is recorded either
way.
A fifty-fifth pass went at the differential, which was the weakest number,
and found the oracle was measuring the wrong transformation. It compared our
`opt -S` against `llvm-as | llvm-dis`, which is not the same thing: writing
bitcode and reading it back applies the compatibility upgrades the bitcode
reader owes older files. The clearest case is the data layout, which the
bitcode reader rewrites for the target the triple names and the textual
reader leaves alone: `target datalayout = "e"` with an x86 triple comes back
as `"e-i128:128"` through bitcode and as `"e"` through `opt -S`, which is
what we print. Thirteen of the differences were that and nothing else. The
check now runs upstream's own `opt -S`, which is the transformation this
performs, and the ratchet counts 137 of 223 where the old one counted 129 of
216.
The largest real class was the identified structs. Upstream prints the ones
its type finder reaches from the module and drops the rest, and the order is
the order the walk meets them rather than the order they were written, so a
module defining `%A` then `%B = type { %A }` and using only `%B` prints `%B`
first. The walk is in `crates/llvm-ir-print/src/type_finder.rs`, and it has
to reach through attributes as well as operands: the corpus caught that, an
`sret(%pair)` being the only place that struct is named.
A fifty-sixth pass took five more of what upstream folds as it reads, all
found by the new oracle and all pinned against it. A struct with no fields
is `zeroinitializer` and an array with no elements is `poison`, which is not
a pair a reader would guess. The default address space is not written on an
alloca. An alloca's count of one goes only when it is written in the width a
count defaults to, because dropping `i64 1` would change the width back to
i32. And a getelementptr that moves nowhere is the pointer it started from,
whether it has no indices at all or all-zero ones. 137 to 142 of 223.
What is left in that suite is mostly not printing. The largest remaining
classes are compatibility upgrades this does not perform: the debug-info
metadata upgrade that fills in a `DICompileUnit`'s file and a
`DISubprogram`'s scope, the intrinsic name upgrades that add a mangling
suffix, and the per-intrinsic attributes that come with the table LangRef
does not document.
A fifty-seventh pass widened the print measurement past the Assembler suite,
to Feature, Linker and Other, and closed the largest class the wider sample
showed: the order upstream writes an attribute set in.
It is three runs. The plain keywords first, then the ones that take an
argument, then the quoted ones by key. Neither of the first two is
alphabetical. The plain ones go in the order LLVM declares them, which is
why `nounwind` comes before `nonlazybind`; sorting them alphabetically
looked right, passed every probe I had written, and broke all eleven rustc
corpus files at once. `EnumAttr` is declared in that order already, so
sorting on the variant reproduces it. The second run's order was measured by
handing all six to `opt -S` at once, and `uwtable` sorts there whether or
not it carries a kind, the bare spelling being the same attribute carrying
its default. That last one the corpus caught too.
Assembler 142 to 143, Feature 48 to 53, Other 121 to 126, Linker unchanged
at 167. All four are ratchets now.
A fifty-eighth pass fixed the alignment an alloca gets when it writes none.
A struct's fields decide its ABI alignment, and the layout's aggregate
preference can ask for more: with the default `a:0:64`, an alloca of
`{ i8 }` is eight-aligned even though nothing in it needs to be. The comment
in `layout.rs` had said a struct's preferred alignment is its ABI alignment,
which is the one case where that is not so. Assembler 143 to 144, Feature 53
to 54.
What is left across the four print suites is led by the debug-info metadata
upgrade, seventeen files between the `DICompileUnit` whose file gets filled
in, the `DISubprogram` whose scope does, and the attribute group that goes
with the declaration the upgrade drops. Six more want a data layout upstream
supplies from the triple when the module writes none, which is target
knowledge this tier does not have.
A fifty-ninth pass took `nocapture`, which is the older spelling of
`captures(none)` and the only parameter attribute that upgrades that way:
`readonly` and `writeonly` keep their spelling there, where on a function
they become `memory(...)`. Reading one and printing the other showed that a
parameter's attributes are written in upstream's order too, not only a
function's set, so the same comparison now sorts both. Assembler 144 to 146.
A sixtieth pass: a pointer operand is written with the address space it
points through, and the printer had `ptr` spelled out as a literal in the
three places one appears, a load, a store and a `cmpxchg`. Address space
zero prints the same either way, which is why nothing noticed until a module
used another one. 146 to 147.
A sixty-first pass: a metadata field written with the value it would have
had anyway is not written back, and two nodes that differ only in one are the
same node once it is gone, so a `!named` list holding both comes back naming
one twice. 147 to 149.
Which fields behave that way has no shape. `DIEnumerator`'s
`isUnsigned: false` goes and `DICompileUnit`'s `splitDebugInlining: false`
stays; `DIDerivedType`'s `offset: 0` goes and `DISubrange`'s `lowerBound: 0`
stays, that one being a metadata operand where a bare number is a constant
rather than an absence. So `corpus/md-field-defaults.nu` writes each field at
its default and reports whether it survives, and the table is that script's
output. It is an allow-list rather than a rule with exceptions, which is the
second thing the corpus taught this pass: the rule-with-exceptions version
dropped `isOptimized: false` from every rustc compile unit, and the probe set
had no `DICompileUnit` in it to catch that.
A sixty-second pass asked whether the debug-info metadata upgrade was worth
doing and found something else on the way in. A `DICompileUnit` with no
`file:` is "missing required field 'file'" upstream and was a module here,
so `corpus/md-required-fields.nu` now writes each node once per field with
that field left out and reports which absences upstream refuses. Seven were
missing from the schema: a compile unit's file, a `DIGlobalVariableExpression`
without either of its two, a `DIMacro` without its type or name, and a
`DIModule` without its scope or name. Neither suite ratchet moves, those
files being ones upstream refuses for other reasons too, but the modules we
wrongly accept is a count the suites do not show and this is seven fewer.
The upgrade itself is still not done. It is not one transformation but a
family: filling a `DICompileUnit`'s file from a `DISubprogram` that names it,
moving a `DISubprogram`'s scope out of a field that no longer exists,
rewriting `DIExpression` opcodes. Each needs the shape of the *older* debug
info to be modelled, which is a dialect this tier does not read, and the
files that want it are twenty-four of the four print suites' 165. Recorded
rather than started.
A sixty-third pass went at the Verifier suite's remaining forty, the modules
we accept that llvm-as refuses, and took the one that needed no table: an
argument past the callee's declared parameters is one the callee has no name
for, so `sret` may not name a place there and `returned` may not promise
something about it. A statepoint is the exception, its variadic part being
the wrapped call's own arguments, which upstream's statepoint.ll relies on
and which cost a file before it was noticed. Verifier 288 to 289.
Three more were probed and left. A load of `target("foo")` is refused where
a load of `target("spirv.Event")` is read, and `<2 x target("spirv.Image")>`
is refused where `<2 x target("llvm.test.vectorelement")>` is read: both need
a table of which target extension types exist and what they may be used for,
which is the same blocker as the target intrinsics. The comment in
`is_valid_vector_element` was right to allow them, and a blanket rule would
have refused a file upstream reads.
`inalloca` on a variadic argument segfaults llvm-as, so there is no verdict
to derive from it. Recorded rather than guessed at.
A sixty-fourth pass took three more from the same pool, all about the shape
of a node rather than what it means: `SemanticInterposition` answers yes or
no with a number rather than a word, a `CG Profile` edge names a caller, a
callee and a count, and a type-based alias tag has three operands or four.
Verifier 289 to 292.
A sixty-fifth pass took three more, and one of them was recorded as
blocked and was not. `llvm.used`, `llvm.compiler.used`, `llvm.global_ctors`
and `llvm.global_dtors` say something to whoever consumes the module rather
than to the module itself, so nothing in the module may read one back.
That had been filed as needing def-use chains, and it does not: the
question is whether any constant reachable from a global's initializer, an
aliasee, a resolver or an instruction operand names one of the four, which
is a walk forwards rather than a use list. `llvm.compiler_used`, spelled
with an underscore, is not one of the four and may be read like any other
global, which is the kind of boundary only a probe finds.
An x86 interrupt handler is called by the processor with the interrupt
frame already on the stack, so its first parameter is a `ptr byval(T)` and
nothing else; what follows it is the error code and is unconstrained. And
what a function does to AArch64's streaming mode and its two matrix state
registers is one answer per register out of six, not six independent
claims, while `aarch64_zt0_undef` describes one call rather than the
function it calls. Verifier 292 to 296.
A sixty-sixth pass took the alignments, three files and three rules that
bisecting found rather than reading. An alignment written as an attribute
is capped the way one written as a clause already was, at two to the
thirty-second, except `alignstack`, which is capped one bit lower. A
vector's lane count is held in thirty-two bits, scalable or not.
And a type crossing a call boundary has to be one the target can place,
which means an alignment inside the cap: a vector's is its own size
rounded up to a power of two, so `<2147483649 x i16>` wants eight
gigabytes and `<2147483648 x i16>` wants four, and an array or a struct
holding one inherits the answer. That rule holds at the call and not in
the signature, and an intrinsic is exempt, being lowered rather than
called. Both boundaries were found by halving the interval against
llvm-as rather than by reasoning about the layout, which is what caught
that `<4294967296 x i8>` and `<2147483649 x i16>` fail for two different
reasons. Verifier 296 to 299.
A sixty-seventh pass took two more and unlearned a rule on the way. A
struct holding a scalable vector has no offset for whatever follows it,
so a `getelementptr` cannot target one even when the index picks the
first field; an array of such structs it indexes happily, the question
being whether this type has fixed field offsets rather than whether a
scalable vector is anywhere nearby.
The rule it unlearned was `speculatable` at a call site, which had been
written as "a call site may not carry it" and is not: a call site may
repeat a promise its callee makes, and only repeat it, so an indirect
call and a call through an alias are the refusals rather than every call.
The suites do not cover the accepting half, which is why the ceiling
never showed it; a probe did. Resolving the attribute groups a call site
names came with it, `#0` on a call having reached no rule until now.
A third rule came with them, and it is not the one its test's message
names: `inalloca` says the argument was pushed onto the stack where the
callee expects to find it, which is what `alloca inalloca` does. The
types need not match at all, so "mismatched alloca" is about the marking
rather than the type, and a value that is not an alloca says nothing
about where it came from and is asked nothing.
Two more came with them, both about what a name has to be. Which
pointer-authentication ABI an AArch64 ELF object was built for is a
platform and a version together, so a module writes both flags or
neither. And what `llvm.assume` is told is written as a bundle whose tag
is the attribute being asserted, so the tag has to name one: `adazdazd`
is not an attribute, `frame-pointer` is a quoted key rather than an
attribute name, and `Nonnull` is not `nonnull`. Two tags are the
assumption's own rather than an attribute's, `ignore` for one that was
dropped and `separate_storage` for two allocations that do not overlap,
and only the second of those has a shape. `dereferenceable` is the one
whose arguments upstream reads: a pointer and how many bytes are behind
it, exactly two.
The last of the pass was the type-based alias graph, which needed
probing to find the shape rather than reading. A type node reaches a root
by way of its parent, so a chain that comes back round to itself
describes nothing, and both of a tag's two types are walked for that. The
access type is the narrower of the two: a base may hold a struct type
node anywhere, because a base says where in an object the access lands,
while an access says what was read and what was read is one value, so its
chain is scalars all the way up. That asymmetry is not in the message the
test checks and would not have been guessed.
Last was the mangled name of a vector variant,
`_ZGV<isa><mask><lanes><parameters>_<scalar>(<vector>)`, which is a
grammar and was derived one field at a time: the isa is one character or
the spelling `_LLVM_`, the mask is `M` or `N`, the lanes are a count and
so not nought, a linear parameter walks by a constant stride or a
negative one or one held in another parameter, and an alignment suffix is
a power of two. The one part that has to agree with something outside the
name is the parameter list, which has one descriptor per parameter the
scalar function takes and at least one either way. That last clause is
why the test's own module is refused: `_ZGV_LLVM_M4v_foo` describes one
parameter and `@foo` takes none, which the message about an invalid name
does not say.
Two more, and the second is why the ceiling exists. A tile is a register
the hardware fills, so there is no constant of type `x86_amx` for a
caller to hand over. And relocating a pointer, or reading what a call
returned, is asking about one safepoint, which a statepoint is what
makes; `token none` marks no point at all.
The first shape of that second rule refused two modules upstream reads,
and the ceiling went 0 to 2 and named them. `token poison` and `undef`
stand for any value, so there is nothing to be wrong about, and a
statepoint written as an `invoke` makes a safepoint on both its edges, so
the `landingpad` the unwind edge opens with carries the same token. A
token that is an argument of the enclosing function is a third case, and
one with no answer: `llvm-as` segfaults on it. That is the fourth crash
this suite has produced.
The last one needed no new table, only noticing what an old one already
says. An intrinsic is selected by its name, so its argument list is the
one the name owns rather than one a caller chooses, and calling it
through a variadic type claims otherwise. Which intrinsics are variadic
is a fact `corpus/intrinsic-signatures.nu` has been discarding since it
was written: it drops LangRef's variadic `declare` lines because their
arity is not fixed, so having a signature in the table at all is the
evidence that the intrinsic is not variadic. Four are, and LangRef names
them. The other half of that file's rule, an intrinsic that is variadic
called through a fixed type, needs to know that
`llvm.experimental.stackmap` is variadic, and LangRef does not document
it at all.
A sixty-eighth pass re-measured a rule the twentieth had measured away,
and the twentieth was wrong. "Every DICompileUnit is listed in
llvm.dbg.cu" is real; what the first measurement missed is the
reachability the twenty-first pass went on to discover, and the reach
here is narrower than the debug-info rules use. A unit a *named list*
leads to has to be listed. A unit only an attachment leads to does not,
so a `DISubprogram` written into a `!named` list takes its `unit:` with
it while the same subprogram hung off a function does not. Both halves
were probed, and the DebugInfo tree, 1,101 files that are all debug
info, is unmoved. Verifier 309 to 310.
A second rule came from the same file's neighbour, and it needed a fifth
derivation script. A field holds text, or a number, or a node; `!"text"`
is none of those, being a reference to a metadata string, which is what a
module writes when it names a type it has not described. Upstream refuses
one nearly everywhere. Where it does not is a list with no shape, so
`corpus/md-string-fields.nu` writes every field of every node kind as
`!"probe"` and reports which survive: sixteen, and only one of them was
the one I would have guessed. `DIModule`'s scope takes a string and
`DISubprogram`'s does not.
The script had to learn to treat a crash as no verdict rather than as a
refusal, `DIGlobalVariableExpression`'s expr being a fifth thing that
segfaults llvm-as.
What is still not derived is the rest of that test's rule: a `baseType`
naming a tuple is refused too, so the field wants a *type* node rather
than merely a node, and which kinds each field accepts is a larger
measurement than this one. Recorded rather than started.
Verifier 299 to 311.
A sixty-ninth pass left the Verifier's pool, which is down to seventeen
and all of it blocked, and went back to the Assembler suite. Four rules,
four files.
A symbol's name reaches the object file, where it ends at the first NUL,
so a name holding one names something else. Local names, labels and
global names all refuse one; a section name does not, that being a string
rather than a name.
An attribute written before the return type says something about the
result, and most attributes say nothing about a result. Which ones do was
measured by writing each of the ninety-nine keywords in that position
against three return types: seven bare ones, `inreg`, `noalias`, `noext`,
`nonnull`, `noundef`, `signext` and `zeroext`, and five that take an
argument. That sweep found a false positive of ours as well: `signext`
and `zeroext` say how a narrow integer fills a register, which nothing
but an integer is narrow in, and `noext` says not to widen it, which
upstream lets anything be told.
And a caller cannot pass a struct whose body this module has never seen,
there being no knowing how much of it to copy. A return type may be one,
the caller having only to name the place it goes.
Assembler 449 to 453, refusals still five.
A seventieth pass took four more from the same suite, and the second bound
earned its keep again. A `musttail` call hands the frame over whole, so a
caller with variable arguments hands those over too: `f(%a, ...)` in an
argument list needs `musttail` and needs the caller to have some, and a
`musttail` call in a varargs function needs the ellipsis at the end. Both
directions, four files with the alias rule.
That alias rule was wrong on its first shape and the ceiling said so, 5 to
7, naming `addrspacecast-alias.ll` and `associated-metadata.ll`. What is
refused is a *bare* reference to a symbol written with an address space
the symbol does not have, because a bare reference has the symbol's own
pointer type. Crossing address spaces is what `addrspacecast` is for, and
an aliasee that is one is not asked.
Assembler 453 to 457.
A seventy-first pass took two more, one of them a rule that existed and
never fired. A call through a value goes through the program's address
space unless it says otherwise, and that check only ran when the module
wrote a data layout; a module that writes none gets the default one,
whose program space is nought, so there is always a space to compare
against. Three files.
And `ptr*` has both dialects in it at once: the older spelling is read,
and writing it around the newer one is not, which upstream says in as
many words. `i8**` is still fine, the inner `*` being what makes the
pointer the outer one points to.
A seventy-second pass left acceptance and went at the print differentials,
which are the weakest numbers now: 496 of 655 across four suites. Diffing
every differing file against upstream's own `opt -S` sorts them into
classes, and the largest are the ones already recorded as blocked: the
per-intrinsic attributes an auto-declared intrinsic comes with, the
debug-info metadata upgrade, and the intrinsic name upgrades. Constant
folding is the largest closable one and is not started.
Five smaller ones closed. A declaration's parameters have no bodies to be
named in, so upstream drops names written there. `default` is what a
symbol has when nothing says otherwise, so upstream writes the other two
visibilities and not that one. A scalar's zero has a spelling of its own,
`0` or `null` or `0.000000e+00`, and only an aggregate keeps the word
`zeroinitializer`. A splat is how a vector of repeated *data* is written,
so a vector of the same symbol goes lane by lane, the lanes being
addresses a linker fills in. And `captures(...)` is a set rather than a
list: naming a component twice says it once, `address` covers
`address_is_null` and `provenance` covers `read_provenance`, and upstream
writes what is left in its own order.
Feature 54 to 57, Linker 167 to 171.
A sixth class went with them, the largest closable one: upstream computes
a constant cast rather than carrying it, so a module that writes one
prints back the answer. A cast that changes nothing is what it was given;
`trunc` keeps the low bits; a `bitcast` keeps the bits and changes what
reads them, either way between an integer and a float; and the two that
cross between an address and a number agree at the one value both spell
the same way, `inttoptr 0` being `null` and `ptrtoint null` being nought.
Every cast but a bitcast works lane by lane, so a cast of a vector of
literals is a vector of the answers, and the operand may be written out
lane by lane or folded to a splat or to zero, all three describing the
same lanes. Feature 57 to 60, Other 126 to 128.
A seventy-third pass finished the constant folding and found where the
differential's ceiling is. Three arithmetic expressions answer with one
of their operands: `add` and `xor` with nought on either side, `sub` only
on the right, subtraction not being commutative. A cast undoes the one
under it when nothing was lost in between, and the width upstream
compares against there is a fixed sixty-four rather than the module's own
pointer size, which a `p:32:32` layout is what shows. Walking from a
pointer nobody chose arrives nowhere in particular, so a `getelementptr`
from `undef` or `poison` is that. And a bitcast between vectors is
answerable in two shapes without laying the bits out: a lane count that
does not change, and a pattern that reads the same at any lane width,
which all-zero, all-one, undef and poison are. Assembler 149 to 150.
Then the ceiling. `!DICompositeType` with an ODR identifier comes back
`distinct` from `opt -S` and plain from `llvm-as | llvm-dis`, the two
upstream tools disagreeing on the same input, because `opt` enables ODR
type uniquing and `llvm-dis` does not. Implementing what the differential
asks for broke the corpus, which is real rustc output printed by
`llvm-dis` and is the headline property. So it was reverted and measured
instead: five of the eighty-seven remaining differences across the four
suites are ones where we already match `llvm-dis` and `opt` does
something more. Those are not closable, and the differential's reach is
that much short of its denominator. The other eighty-two are ours, and
sorting them showed the shape of what is left: sixteen the debug-info
upgrade, fifteen the per-intrinsic attributes, five the declarations
those come with, seven metadata numbering, three the order a specialized
node's fields print in, and the rest one-offs.
Two of the one-offs closed. A scalable vector has no fixed size and still
has an alignment, the one the minimum-size vector would have, the target
scaling the length rather than the alignment; `alloca <vscale x 4 x i32>`
is sixteen-aligned and `<vscale x 1 x i8>` is one. And four module flags
have their behaviour rewritten as the module is read: `PIC Level` and the
three branch-protection ones take the larger of two, `PIE Level` the
smaller, where each was once written `Error` and had to match exactly.
`PIC Level` takes `Min` as well as `Error`, which the other three do not,
and that asymmetry is why the rule is a table rather than a shape.
Linker 171 to 173, Other 128 to 129.
The last of the pass was the order a specialized node's fields print in,
which is a sixth derivation script. Upstream writes them in an order of
its own rather than the one they were read in, so
`!DIBasicType(size: 32, name: "int")` comes back with the name first, and
a printer that keeps the written order diverges on any module that did
not already use upstream's. `corpus/md-field-order.nu` writes each kind's
fields backwards and reads back the order they come out in, and thirteen
of the twenty-four kinds print in an order the grammar does not give:
`!DIBasicType` writes `flags` after `num_extra_inhabitants` and the
grammar has it before.
The script had to learn two things the corpus taught it. A field written
at its default does not come back, so it would be missing from the order
and sort to the end; the probe values are non-default now and the script
reports any that still drop. And a `!DICompositeType`'s fields cannot all
be written at once, `dataLocation` and its neighbours belonging to an
array and `identifier` and its to a structure, so that kind takes two
probes and the orders are merged on what they share.
It moved no ratchet: the files whose fields were out of order are also
waiting on the debug-info upgrade. It is still a divergence closed, and
the corpus caught two of the ways it can be got wrong.
The same measurement showed one more thing, and that one did close.
Upstream writes some fields the module never wrote, and a compile unit is
the only node it does that for: `isOptimized`, `runtimeVersion` and
`emissionKind` appear whether or not they were written. They say
something about the whole translation unit, so leaving one out says
nothing rather than saying the default, which is the opposite of how
every other node treats an absent field. Assembler 150 to 151, Linker 173
to 174.
A seventy-fourth pass went at metadata numbering, which was the largest
class left that nothing blocks, and it was the defaults table being
short. Two nodes that differ only in a field written at its default are
the same node once it is gone, so a `!named` list holding both names one
twice; `!DINamespace(name: "", scope: !0)` and `!DINamespace(scope: !0)`
are one node upstream and were two here.
The reason the table missed it is that `corpus/md-field-defaults.nu` only
probed the fields outside each node's legal skeleton, and a skeleton
field can drop too. It probes every field now, replacing the skeleton's
value rather than writing the field twice, which is what upstream refuses
and what made the first attempt at this report nothing. Fifty-nine pairs
became seventy-four: a name is droppable on seven kinds, a template value
parameter's type is, and a macro's value is.
Fixing the script also fixed a probe that had never worked: a `distinct`
`!DISubprogram` is a definition, and a definition needs a compile unit,
so every one of that kind's fields had been reported as refused rather
than measured. Assembler 151 to 152.
Two more things stood between a dropped field and a uniqued node, and
neither is in any table. A field that names a node names none when it is
written `null`, which is the same as not writing it, and `is_default` did
not say so, so `entity: null` survived and the node stayed its own. And an
empty argument list says nothing about which of the two spellings it is:
`!DIObjCProperty()` parsed as positional while a node whose fields all
dropped ended up named and empty, so uniquing saw two nodes where
upstream sees one. They are the same thing now.
The three files this class owns still differ, by the debug-info upgrade
rather than by numbering, so no ratchet moved. What moved is the defaults
table, from fifty-nine pairs to a hundred and fifteen, and three
divergences that were real whether or not a test file showed them.
Two one-offs closed with them. A comdat stands on its own, so each is
preceded by a blank line rather than the group being preceded by one. And
an `llvm.` name upstream does not know is not an intrinsic at all, only a
function whose name looks like one, and upstream says `; Unknown
intrinsic` above the declaration; whether it knows one is the same gate
the parser already uses to decide whether to materialise a declaration
from a call. Assembler 152 to 154, Linker 174 to 176.
One more divergence closed without moving a number: upstream numbers each
attachment kind as it first meets it and writes them in that order rather
than the order they were read, so `!prof` comes before `!llvm.loop`
however the module wrote them and `!tbaa` before `!range`. Twenty of them
were written on one call, backwards, and the order they came back in is
the table. The three that only a terminator takes came from a second
probe, and where they sit relative to the load-only kinds cannot be seen
at all, no instruction taking both.
Two other differences in that batch were probed and are not ours: a
`target datalayout` upstream supplies from the triple when the module
writes none, which is the target knowledge already recorded as missing,
and a valueless quoted attribute, which turned out to print correctly in
isolation and to differ only inside a file that has the debug-info
upgrade in it too.
A seventy-fifth pass took one more of the same shape. A node is uniqued by
what it holds, so a node that holds itself cannot be, and upstream makes
one distinct whether or not the module wrote the word. Two nodes that name
each other are not this and stay as they were, which is the boundary a
probe found and a rule about cycles would have got wrong.
Assembler 154 to 155, Linker 176 to 177. What is left across the four
print suites is fifty-four files, forty-one of them the three recorded
blockers, three the data layout upstream supplies from a triple, two the
ODR-uniquing ceiling that cannot move at all, and eight one-offs.
One of those eight closed with the same rule the forty-sixth pass wrote
and only half applied: upstream has no inline node but the two kinds
written at every use, and a node written in place inside a *specialized
field* was still being kept there, so `types: !{}` printed as itself
rather than as `types: !5` with `!5 = !{}`. The reference case had been
hoisting since that pass; the field case had not.
A seventy-sixth pass took the debug-info upgrade, which had been recorded
as blocked since the sixty-second and is not. It was written off as "not
one transformation but a family, each needing the shape of the *older*
debug info modelled", and one member of that family is nineteen of the
fifty-four files and is a mechanical field mapping.
A subprogram's `isLocal`, `isDefinition`, `isOptimized` and `virtuality`
were four fields saying four things and are one `spFlags` set now, so a
node writing any of them is written back with the set instead. The bits
go in an order of their own, virtual before local before definition
before optimised, and `isDefinition` is the one whose absence does not
mean false: a subprogram in the old format is a definition unless it says
otherwise, which is why `isLocal: true` alone comes back as a definition
too. That last is what the probes were for; it is not a shape anyone
would guess.
One rule came with it that is not part of the upgrade at all: which slot
in the vtable a subprogram occupies is nought for most of them and a
virtual subprogram writes it anyway, whichever spelling set the flag.
Assembler 155 to 157, Linker 177 to 187, Other 129 to 131. Ten files in
Linker alone. What remains of the family is the compile unit whose file
is filled in from a subprogram that names it, and the `DIExpression`
opcode rewrites; those do want the older dialect modelled.
A third member closed with them: `llvm.dbg.value` and `llvm.dbg.declare`
once took an offset into the variable between the value and the variable,
and the expression took its place, so upstream drops the argument as it
reads. The fiftieth pass read the newer four-argument spelling as a
record and left the older five-argument one alone.
The other class was measured and is blocked after all. The per-intrinsic
attributes want a table LangRef does not have: fourteen of its eight
hundred `declare` lines carry an attribute, which is too few to harvest
from, so `@llvm.assume` coming back with five function attributes and an
`i1 noundef` is not something this project can reproduce. Eighteen files,
and the number is recorded rather than guessed at.
Two more of the subprogram's rules came out of the same files. A `unit:`
belongs to a subprogram that has a body, so one carrying it is a
definition however it was spelled; and every subprogram says what it is
scoped to, even when that is nothing, `scope: null` being written where
the module wrote no scope at all. Linker 187 to 189, Other 131 to 132.
A seventy-seventh pass took the DWARF vocabulary, which has been recorded
as unobtainable since the sixty-third and is not. The reasoning was that
no specification this project may read enumerates every `DW_TAG_*`, which
is true and beside the point: the oracle knows. A field that takes a word
takes the number behind it too, and upstream prints the word back, so
writing `encoding: 5` and reading `DW_ATE_signed` is a question `llvm-as`
answers. `corpus/dwarf-vocabulary.nu` sweeps each such field over a range
of numbers and writes down what comes back.
Six vocabularies came out whole: fifty-eight languages, twenty-four
encodings, eighty-four calling conventions, four emission kinds, three
name table kinds and the macro types. `tag` is per-node, each kind taking
the tags that make sense for it, so it is swept on five kinds and unioned.
Two came back empty, `virtuality` and `checksumkind`, their probes still
being refused for reasons the sweep reports rather than hides.
Assembler 157 to 160.
The tables were then used to refuse a word they do not have, which the
three files waiting on `DW_TAG_badtag` want, and that was wrong. The map
is sound and the vocabulary is not complete: a value equal to the field's
own default never prints, so the sweep never learns its word.
`nameTableKind: Default` is the case that showed it, along with a vendor
tag past the range the sweep first covered. The DebugInfo ratchet went
1,101 to 1,098 and the Assembler ceiling 5 to 6, which is the third time
a bound has caught a rule that looked right.
So the tables print a number as a word and do not refuse a word as
unknown. The `DW_TAG_badtag` entry stays open, and the reason it is open
is now a sentence about defaults rather than about specifications.
A seventy-eighth pass took three more from the same files. A global
variable writes whether it is local to its unit and whether it is defined
here whether or not the module said, which makes it the third node with
fields of that kind, after the compile unit's three and the subprogram's
scope; they are one table now rather than a special case each.
Upstream works out the three arithmetic expressions when both sides are
known, lane by lane for a vector, where before only the identities were
folded. And `captures(...)` has a second half: `ret:` introduces what the
return value captures and everything after it belongs to that list, so
the two are reduced apart, `ret: none` alone is `none`, and a return that
captures what the argument does says nothing the first half has not.
Assembler 160 to 164.
A seventy-ninth pass found that a field's default is not always its type's
zero, which every derivation so far had assumed. A compile unit inlines
its split debug info unless it says otherwise, so `splitDebugInlining:
true` drops and `false` stays, the opposite way round from every other
boolean. And a global variable is a definition unless it says otherwise
where local-to-its-unit is false unless it says otherwise: two booleans
next to each other defaulting opposite ways.
So `corpus/md-field-defaults.nu` probes every boolean at both polarities
now and records which value drops, and the table carries the value rather
than the field alone. A compile unit's producer and flags went in with
them, having never been probed. A `!DILocation` always writes its line,
which is the fourth node with a field of that kind.
Assembler 164 to 167, Other 132 to 133.
Four node kinds had never been probed at all, `DILexicalBlockFile` among
them, which is why two of its nodes that differ only by a `file: null`
stayed two nodes here and are one upstream. A hundred and twenty-two
pairs now. And a `splat` a module writes is expanded the same way one
this reads would have been folded: the shorthand is for repeated data, so
a vector of the same symbol goes lane by lane whichever way it arrived.
Assembler 167 to 168.
An eightieth pass moved the field sort from the printer to the parser,
which is where it always belonged. Two nodes that differ only in the
order their fields were written are one node upstream, and uniquing
compares what is stored rather than what is printed, so sorting at the
end left `!DILocation(line: 3, column: 7, scope: !0)` and
`!DILocation(scope: !0, column: 7, line: 3)` as two nodes that print the
same. The table moved to `llvm-ir` so both sides can reach it, and the
printer still sorts, for the nodes the builder makes rather than the
parser. Assembler 168 to 169.
Also measured and left: the intrinsic name upgrades add a mangling suffix
worked out from the overloaded positions, `llvm.stacksave` becoming
`llvm.stacksave.p0` and `llvm.ctlz` becoming `llvm.ctlz.i32`. Which
positions those are is the per-intrinsic table LangRef does not give, and
it is three files.
One more went with the same pass, and it was the fiftieth's leftover. A
debug-info intrinsic's declaration is not printed, so a group it names has
one fewer user, and a group with no users is not printed either; the
collection that gathers groups was still counting the declarations the
printer then skips. Assembler 169 to 171, Linker 189 to 196: seven files
in Linker, which is more than the rule looks worth from its statement.
Re-measuring the ceiling after all that: twelve of the seventy-five
remaining differences are ones where this already matches
`llvm-as | llvm-dis` and `opt -S` does something else, up from five, the
proportion rising as the real differences close. The clearest new one is
`!DIObjCProperty`, where `opt -S` prints the setter and the getter
swapped and `llvm-dis` does not; it is a bug in one upstream tool rather
than a rule to copy. The attainable maximum across the four suites is
643 of 655, and sixty-three of the differences are genuinely ours.
An eighty-first pass took two more of those. A symbol in the comdat its
own name makes writes the bare `comdat` rather than naming it, which is
four files in Linker and reads oddly until you see that a comdat is
usually the symbol's own. And a shuffle mask lane that picks nothing is
a lane nobody reads, which upstream spells `poison` rather than `undef`:
the two say the same thing there and it writes back the one that says it
of a value never chosen.
Linker 196 to 200, Feature 60 to 61.
Assembler 457 to 461. What is left in that suite is eleven use-list order
negative tests that need the def-use chains PLAN 4.2 is waiting on, the
DWARF vocabulary (three files), the target extension type table and the
per-intrinsic `immarg` positions.
One rule was probed and left. A `DILocation` inside a plain metadata node
is refused when an instruction attachment reaches it, except under
`llvm.loop`, whose whole subtree is exempt, and except from a named list.
Where the exemption ends is not measurable here: `llvm-as` aborts on a
`DILocation` inside an `!annotation` node, and a crash is not a verdict.
That is the second crash this suite has produced, after `inalloca` on a
variadic argument, and a third came out of the tbaa probing: a struct
type node with a null field segfaults it.
Still open, and each entry says what it is waiting on rather than only
what it is. Which argument of an intrinsic is `immarg` when the
declaration does not say so (four files): LangRef writes `immarg` in five
`declare` lines out of eight hundred, so there is nothing to harvest.
The DWARF
vocabulary, so that `DW_TAG_badtag` is refused (three files): needs a list
of every valid enumerator that no specification we may read enumerates.
`DIExpression` opcode sequences (three files): needs the stack discipline
of a DWARF expression, which is `llvm-debuginfo`'s at T1. Module flag
value shapes, and a handful of one-off rules that each cost more to state
than they return.
An eighty-second pass took three print classes and found a distinction
under the third. Staying inside the object walked from is the stronger
promise and has staying inside the signed range in it, so `getelementptr
inbounds nusw` comes back as `inbounds` alone, while `nusw` on its own is
written. Fast-math flags belong to the two casts that change nothing but
precision: `fpext` and `fptrunc` carry them, and the others are refused
where the word stands, an integer having no NaN for a promise to be
about.
The third was a `tag:` written at the one its kind assumes, which drops
the way any other defaulted field does. Which word that is has no zero to
probe at, so `corpus/md-field-defaults.nu` sweeps the whole vocabulary
per kind instead: three kinds have a default tag and each has exactly
one, `DIBasicType`'s being the only one the table already had.
Underneath it was something no derivation had asked about. A field that
is not written back may still be stored: `!DIBasicType()` and
`!DIBasicType(size: 0)` print the same and are two nodes upstream,
because a size is held as an operand and nought is a size where nothing
is not. So each dropped field is now asked a second question, whether the
two unique, and six answer no: `size` on four kinds and `offset` on two.
Those six are dropped from the printing and kept in the node, which is
the first time the two halves of "at its default" have had to come apart.
The question can only be asked of a kind that uniques at all, a
`distinct` one being its own node whatever it holds, and the script says
so rather than reporting every field of those four kinds as stored.
Three print rules came with them, all from the wider sample. A comdat is
a group for symbols to join, so one nothing joins says nothing and is not
written back. A declaration has no body for a `!dbg` to point into and
writes what it carries between the word and the return type,
`declare !attach !0 void @f()`, where a definition writes it after the
signature. And `; Unknown intrinsic` goes between the attributes a
function has and the line declaring it rather than above both.
Three more went with them, and the first two were bugs of ours rather
than rules we had not met. A run of function attributes that starts with
a quoted key is the same run: it may name a group and it may hold the
older memory spellings, and reading only the attributes there dropped
both, so `define void @f() "a"="b" #0` lost everything `#0` held. And a
node takes its number when it is first written, not when it was read, so
attachments have to be walked in the sorted order they print in: a
terminator carrying `!llvm.loop` before `!prof` numbered them backwards.
The third is how six significant digits are written out. Upstream rounds
to six and pads the seventh with a nought where `%.6e` keeps seven
digits, so `bitcast (i64 42 to double)` is `2.075080e-322` and not
`2.075076e-322`. The two agree wherever the value ends by the sixth
digit, which is why only the subnormals showed it: adjacent values are
far enough apart there that six digits still read back.
Five more came from the same sample, four of them ours and one a whole
vocabulary. A float class is a set of the ten kinds a float can be, and
upstream names it rather than writing the bits: `nofpclass(504)` is
`nofpclass(zero sub norm)`. The order it tries the names in is not the
bit order, `pzero` coming before `nsub`, so it was measured by writing
all 1,023 masks at once as the parameters of one function and reading
back what each came out as. A module that writes the words already gets
upstream's order back, which is why the naming happens as the attribute
is read rather than as it is written.
A `musttail` call in a function with variable arguments hands those over
too, and the ellipsis that says so was read and not written back. A
struct of scalable vectors has an alignment where it has no fixed size,
the strictest its fields ask for, and asking for the whole layout to get
it failed on the first field that has no size. And a metadata name that
opens with a digit reads as a number, so upstream escapes the first
character alone: `!\3111` is the name `111`.
Four more, and one of them had a shape nothing else in the printer has. A
walk that answers with a vector answers lane by lane, so an index written
once stands for the same index in every lane and upstream writes it out
as one: `getelementptr ([4 x i32], ptr @G, i32 0, <4 x i32> ...)` comes
back with `<4 x i32> zeroinitializer` in the first position. A struct
field is the exception and goes the other way, every lane picking the
same field, so a `<2 x i32> <i32 1, i32 1>` there comes back `i32 1`.
Telling the two apart needs the type walk, which the verifier had and the
parser did not.
With them: a walk that moves nowhere is the pointer it started from
unless it carries an `inrange`, which says something the pointer does
not; and an ifunc resolver that is a constant expression writes no type
in front of it, on the way in as well as on the way out, which is the
rule an alias aliasee has had since the fifty-second pass.
Two more were measured and are not ours. `opt -S` drops a ThinLTO summary
index that `llvm-as | llvm-dis` keeps, which is two files on the wrong
side of the ceiling for the same reason ODR uniquing is.
Four more, and the last of them was a spelling the model had kept apart.
A field that takes a word takes the number behind it, and the two were
stored differently, so `!GenericDINode(tag: 3)` and the same node written
`tag: DW_TAG_entry_point` were two nodes here and are one upstream. The
word is read into its number now, next to where a defaulted field is
dropped, which is the same reason the field sort moved there in the
eightieth pass: uniquing compares what is stored. The verifier's two tag
checks read through the vocabulary rather than matching the word.
A scalable vector has no lane count to write out, so a splat of a symbol
is written as the construction that makes one: the value in the first
lane and a shuffle across the rest. A splat of data stays the shorthand.
And `operands:` holds the node's own operands, written with braces and no
leading `!`, which is a print rule and a refusal both: `operands: !0`
names a node that holds them and upstream wants the braces. An operand
list with nothing in it lists nothing, so it drops the way any other
defaulted field does, which the derivation script now records rather than
reporting the probe as refused.
One more, and it is a reader upgrade rather than a print rule. A module
says which debug-info format it holds with a module flag, and upstream
drops the lot rather than reading an older one: the `!dbg` attachments,
the debug records and the `llvm.dbg.cu` list, leaving everything else.
What is left is ordinary metadata, so a node some other named list still
reaches survives and one only the debug info reached does not, which is
the reachability the twenty-first pass built. An attachment naming a node
the module never defines is left where it is, because that is a
reference upstream refuses rather than debug info it drops, and the
rejection table is what caught the first shape of that.
A neighbouring file was probed and is not this: `opt` runs the verifier,
finds a `#dbg_value` with no location, and strips the debug info on that
basis rather than on the version. Reproducing that means stripping where
this reports, which is a decision rather than a rule.
Four more, two about where code lives and two about Objective-C. A
function and a call both live in the program address space unless they
say otherwise, so under a `P42` layout a function that writes nothing
comes back `addrspace(42)` and one that writes `addrspace(0)` keeps it,
nought being worth saying where the default is not nought. A call writes
it before the return type rather than after.
A module that names an Objective-C image-info version is saying it has no
class properties unless it says otherwise, so the flag that says so is
added; and how the collector is configured is eight bits wide however
wide the module wrote it, so `i32 512` comes back `i8 0`.
`remangle.ll` was looked at again and is blocked rather than a type-order
bug: upstream renames the two intrinsics as it reads, which is what puts
the type definitions in the order it prints them, and the renaming needs
the per-intrinsic overloaded positions.
Assembler 171 to 188, Feature 61 to 65, Linker 200 to 205, Other 133 to
135. What is left across the four suites is fifty-five differences and
every one of them is a recorded blocker: thirty-five the per-intrinsic
attributes, twelve the `opt`-versus-`llvm-dis` ceiling, six the data
layout upstream supplies from a triple, and three one-offs. That avenue
is finished at this tier.
So the next pass went back to acceptance, where the Assembler suite still
wrongly accepted seventeen modules, and took the DWARF vocabulary, which
has been open since the sixty-third pass and half-open since the
seventy-seventh. What blocked it was that the sweep only sampled: a word
it had not seen might still be one upstream knows, so refusing an unknown
word refused three files it should not have.
The sweep covers each field's whole range now, which needs the values
asked in batches: one module holding a node per value, sixty-five
thousand of them for `tag`, read back through `llvm-as | llvm-dis`
because a value that is legal to write and refused by the verifier would
otherwise take the whole batch with it. `tag` goes from seventy-nine
words to a hundred and fourteen and `language` from fifty-eight to
sixty-three.
Six of the nine vocabularies are complete that way and refuse a word they
do not have. Three are not, and say so: `nameTableKind: Default`,
`virtuality: DW_VIRTUALITY_none` and `checksumkind: CSK_MD5` are words
upstream takes and no sweep can learn, a value equal to a field's own
default never printing its word. `tag`'s one gap is `DW_TAG_null`, which
upstream refuses anyway.
The field's node kind had to come with it, because `type:` is a macinfo
kind on a macro and a node reference on the four kinds that say what
something is.
The `flags:` set went the same way, and it is a set rather than a
vocabulary: words joined by `|`, each of which has to be one. They were
swept out of every single bit and every pair of them, thirty-two words,
and the mask names LLVM groups them under are not among them:
`DIFlagAccessibility` is refused where `DIFlagPublic` is read.
Turning a number back into those words is measured and not done. The
order is two grouped fields, accessibility and inheritance, before the
rest in ascending bit order, and a bit no word names is dropped rather
than written, so `flags: 1073741825` comes back `DIFlagPrivate`. No
module in the suites writes a `flags:` as a number, so the reach is
nothing and the shape is recorded instead.
Assembler 461 to 464, with the modules we wrongly accept seventeen to
fourteen and the ones we wrongly refuse still five. Ten of the fourteen
are the use-list order tests that need the def-use chains PLAN 4.2 is
waiting on; the other four are the per-intrinsic signature, `immarg`
positions, `DIExpression` opcodes and the target extension type table,
each recorded already.
The next pass took the per-intrinsic attributes, which had been recorded as
impossible four times and are not. The reasoning each time was that LangRef
writes an attribute on fourteen of its eight hundred `declare` lines and
fourteen is too few to harvest from. That is true, and it is the wrong
question, the same shape of wrong question the DWARF vocabulary was: what
LangRef has to supply is the *signature*, and the oracle supplies the
attributes. A declaration written bare comes back carrying them, so writing
out each `declare` line LangRef documents and reading it back is the whole
derivation. `corpus/intrinsic-attributes.nu`, and 369 intrinsics in 22
distinct function attribute sets.
Four things had to be measured before the table was right, and each was a
probe rather than an argument. Upstream *replaces* what the module wrote
rather than adding to it, parameter attributes included, so
`declare void @llvm.assume(i1 nonnull) #7` comes back `(i1 noundef)` with
`#7`'s contents gone. A declaration whose types are not the intrinsic's is
left alone entirely, which is what tells a probe that found nothing from an
intrinsic that carries nothing, and is why the wrong guesses in a sweep cost
nothing. Every instantiation of one base name agrees, so the table keys on
the base name; the two that do not are `llvm.scmp` and `llvm.ucmp`, whose
`range` follows the result width, and they are reported and left out rather
than having one of their answers picked.
Harvesting LangRef was most of the work and none of the insight. A long
declaration is wrapped across lines with the return type on one of its own,
so `declare <ty2>` is not a declaration and neither is the argument list
under it; thirty-seven write their function attributes after the argument
list, `declare void @llvm.trap() cold noreturn nounwind`, which cost
`llvm.trap` and the `memcpy` family; and the ones written schematically are
instantiated rather than dropped, `<ty2>` filled in with each of four
concrete types and the return type substituted apart from the arguments,
because a conversion takes one kind and produces another. The candidates go
in rounds of one per name, LangRef writing no mangling suffix where the type
is a placeholder, so a batch of them would be fifteen redefinitions out of
sixteen.
One limit is deliberate and recorded rather than worked around: a variadic
intrinsic is left alone, there being no arity to check a declaration against
when the declaration's own is open. That is four intrinsics.
The `immarg` positions came with it, having been a blocker of their own, and
that is what moved the Verifier: 311 to 314, with the modules we wrongly
accept seventeen to fourteen. Assembler 188 to 189, Feature 65 to 67, Linker
205 to 207 and Other 135 to 140, which is 593 to 603 across the four print
suites.
Measured on the way and not done: upstream recomputes an intrinsic's
mangling suffix from the types rather than reading it off the name, so
`llvm.ctlz` and `llvm.ctlz.i64` both come back `llvm.ctlz.i32`. That is the
recorded intrinsic-name-upgrade blocker and it is now reachable the same
way, wanting the overloaded positions per intrinsic, which the same sweep
could ask for. The target intrinsics are still blocked, `llvm.aarch64.*`
being names no specification this project may read enumerates.
With the attributes in, the print differential is at its floor for this
tier, and the remaining fifty-odd files were sorted to say so rather than
assumed to be. Fourteen are those aarch64 target intrinsics, thirteen the
`opt`-versus-`llvm-dis` ceiling (seven ODR type uniquing, six the ThinLTO
summary index `opt -S` drops and `llvm-dis` keeps), five the data layout
upstream supplies from a triple, and the rest debug-info one-offs.
The three that looked like closable one-offs were each measured and each is
blocked. `asm-path-writer.ll` is the summary index, `ifunc-asm.ll` the data
layout, and `target-types.ll` wants an alignment per target extension type:
`spirv.Event`, `spirv.DeviceEvent` and `spirv.Image` are eight,
`aarch64.svcount` is two and `x86.AMX` has no size at all, so there is no
default to fall back on and it is the same table the type parameters want.
The pass after it took the fourth reading of the same `declare` lines, and
it needed no oracle at all. `corpus/intrinsic-signatures.nu` records a
position whose type is the same in every documented instantiation and
records nothing where it varies, and what that throws away is the other
half: two positions whose types vary *together* are one overloaded type.
`llvm.umax` is documented `i32, i32 -> i32` and
`<4 x i32>, <4 x i32> -> <4 x i32>`, so both arguments and the result are
one type and `llvm.umax(i8 0, i16 1)` names no instantiation there is,
which is the "invalid intrinsic signature" upstream reports.
`corpus/intrinsic-overloads.nu`, 161 intrinsics, counting the result as
position nought because upstream ties a result to an argument as readily as
two arguments to each other. The conclusion is only drawn where the type
actually varies and only from an intrinsic documented more than once: two
positions that are `i1` everywhere are equal by being fixed rather than by
being tied, which the signature table already says, and concluding an
overload from a single `declare` line would be inventing one.
Both halves of the rule were probed first, and they land in different
places. An undeclared call reports at parse time, there being no
declaration left to build from it. A declaration written out with the same
mismatch is read, and the verifier refuses the *call* rather than the
declaration: `declare i8 @llvm.umax.i8(i8, i16)` on its own is a module
upstream reads, an unused declaration never being looked at.
The rule cost a CodeGen file before it was right, which is the tree ratchet
catching something neither suite does for the second time. The bug was not
the table but the lookup: it dropped trailing components until something
matched, so `llvm.vp.cttz.elts.i32.nxv16i1` reduced to `llvm.vp.cttz`,
which is a different intrinsic whose result is its operand's type where
`vp.cttz.elts` counts into an `i32`. Both new tables key on
`table::strip_mangling` now, which drops only components shaped like a
mangled type and is what the derivations use, so a name can no longer walk
into a shorter one that is a prefix of it.
`table::signature` and `table::base_name` still use the loose reading. That
is recorded rather than changed here: it is the same bug, and moving it
moves numbers that want their own measurement.
The pass after it did that measurement, and the first answer was wrong in
the direction that matters. Reducing a name with `strip_mangling` alone
left the suites, the corpus and all four print differentials unmoved, which
looked like the loose reading having bought nothing; the trees then said it
had bought fifty-four modules, CodeGen 22,369 to 22,352, Transforms 10,223
to 10,217 and Analysis 1,394 to 1,363. Only the trees could say so, the
suites having no instantiation of the kind at stake.
What they were buying was the shapes `mangled` did not know. It was a list
of prefixes, `nxv` and `v` and `p` and `i` and `f` and `a`, which is the
right idea written in a form that has to be kept current and was not: it
knew `f128` and not `fp128`, and no spelling of `bfloat` at all, so
`llvm.fabs.bf16` and `llvm.sqrt.fp128` stopped being intrinsics.
The rule is a property rather than a list now. A mangled type carries the
width or the count of what it describes, so it has a digit in it. A
component with no digit is a word and a word belongs to the name, which is
what `elts` is.
That took two goes as well, and the tree said so again: the digit alone
left Transforms one short, at `llvm.is.fpclass.half`, because the types the
IR spells out carry no width to write down. So the property has a closed
set beside it, and a measured one rather than a guessed one: every
digit-free type spelling that follows a documented name anywhere in
`llvm/test` is in it, which is `half` on twenty-three names, then `ptr`,
`void`, `token`, `bfloat`, `metadata`, `float`, `double` and `isVoid`.
Everything else digit-free stays a word.
That alone would break the other direction, because a name can end in
something shaped exactly like a type and still be a name: LangRef documents
`llvm.convert.to.fp16`. So the reduction is a sequence rather than a single
answer. `intrinsic::candidates` offers the whole name first and then drops
trailing mangled types one at a time, stopping at the first word, and each
table takes the first candidate it holds.
It lives in `intrinsic/mod.rs` rather than in a generated file, which is
the second thing this pass found: `base_name` and `strip_mangling` had been
hand-written into `table.rs`, which `corpus/intrinsic-signatures.nu`
generates and does not emit them, so regenerating that table would have
deleted them without a word. Nothing had regenerated it since they were
added.
The pass after that went back to the use-list order directives, which the
fifty-second pass left as ten files wanting "the def-use chains PLAN 4.2 is
still waiting on". Reading what the ten actually check says otherwise.
Eight want the *number* of uses rather than their order: "value has no
uses", "value only has one use", "wrong number of indexes, expected 3" and
"expected distinct uselistorder indexes in range [0, size)" all need a
count, which is a walk forwards over the operand slots rather than a use
list. One wants a placement rule and one wants no count at all.
That last one is this pass. A directive names its value with the type the
value was defined with, and upstream reports it two ways: a local against
its own definition, a global for not being a pointer, a symbol reference
having the symbol's own pointer type. Both were probed, both directions.
The value had been skipped rather than read, on the grounds that nothing
needed to know which value it was, so the type is read now and the rest of
the reference is still skipped, it being a constant expression with commas
of its own in the general case. Assembler 465 to 466.
The pass after it built that count, and the blocker recorded for it was
half wrong. `InstKind::operand_values` already walks every value an
instruction reads and `ConstExpr::parts` already walks an expression's, so
only `Constant::operand_constants` was missing, which is one match.
`Module::use_count` is the walk: every interned constant's operands, every
instruction's, global initializers, aliasees and resolvers, and a
function's personality, prefix and prologue. It cannot be taken while the
module is being read, a global used by a later function not yet being used
when the directive is parsed, so the directives are collected and checked
after the parse.
The count was validated before any rule leaned on it, against upstream's
own message: "wrong number of indexes, expected N" names the number, so the
test holds six shapes with N read off `llvm-as`. Two of them are the ones
worth having: `icmp eq ptr @g, @g` is two uses, and one `getelementptr`
written into two globals is one, constants being uniqued.
Turning it on cost four tree modules anyway, and neither cause was one that
reasoning would have found. A `ptr null` has no use list at all: upstream
takes any number of indexes for it, and sweeping the kinds gives a clean
boundary, `null`, a literal, `undef`, `zeroinitializer` and a splat are
never checked where a symbol, an expression and an aggregate holding
either are. That is constant *data*, shared across the context rather than
owned by the module, and `Constant::has_use_list` says so. The other three
permute `@llvm.dbg.declare`'s use list, and its calls are read into debug
records the way upstream reads them, so the uses are gone from the model
before the count is taken; a record is the call, so the count adds one per
record whose name matches.
Two fixtures of ours turned out to be modules upstream refuses, which is
worse than a missing rule because they were asserting the opposite. One
said `uselistorder ptr @a` parses where nothing uses `@a`. The other sat in
the table of modules upstream *accepts* and had three faults at once: a
directive among the instructions rather than after the terminator, a value
with no uses, and `uselistorder_bb` on the entry block, which nothing
branches to. Nothing had ever asked `llvm-as` about it.
Assembler 466 to 472, with the modules we wrongly accept twelve to six.
The pass after it took the rule that broken fixture had been hiding, which
is the one thing it was right about by accident. A body's directives come
last, in a run: once one is written, an instruction after it is "expected
uselistorder directive" and so is a label, while a second directive is
fine. Written before the terminator it is not a directive at all but an
instruction upstream cannot name. All three were probed, and the first
attempt at probing them measured nothing, the count error firing before
the placement one because the probe gave the wrong number of indexes.
Assembler 472 to 473, and nine of the ten are closed.
The tenth came with the same pass: a local is counted inside its own
function, nothing outside one being able to use it, and it needs no waiting
the way a constant does because a body's directives come after every
instruction in it. A name the body never defines has no uses, which is what
upstream says of it.
Two things went wrong first and both were caught by a bound rather than by
reading. The ceiling rose, 5 to 6, on `uselistorder label %preexit`: a
label names a block, whose uses are the branches that reach it rather than
the operand slots that read a value, so it is a different count and is left
alone. Excluding labels fixed the ceiling and gained a file, `uselistorder.ll`
being one of the suite's own.
Then Transforms fell three, and the cause is a distinction worth keeping.
`operand_values` leaves out a phi's incoming values on purpose: it answers
the dominance check, which wants a phi's value to dominate the
predecessor's terminator rather than the phi. That is right there and wrong
for a count, so `use_count_values` puts them back, and a debug record's
values with them, without touching the walk dominance uses.
Finding it took longer than it should have because the position an error
reports is the token after the one it is about: `self.error` fires once the
index list is consumed, which is the next line. Three files were diagnosed
against the wrong directive before that was noticed.
Assembler 473 to 475, with the modules we wrongly accept six to three. All
ten use-list files are closed.
Two of the fixtures those rules tripped over turned out to be modules
upstream refuses, sitting in tables that claim the opposite, so the next
pass asked upstream about every one of them.
`every_fixture_agrees_with_llvm_as` runs each of the four tables past the
assembler when `LLVM_AS` names one, off by default because the unit checks
carry no LLVM. Each table is a different claim and needs a different
question: `ACCEPTED` says the text parses, so the assembler is asked with
its verifier off; `VERIFIES` says a whole module is well formed, so it is
asked with the verifier on; `REJECTED` and `BROKEN` say upstream refuses,
and the exit code is the whole of that, the stage it refuses at being
nobody's business here. The first shape of the audit asked all four the
same way and reported seventy-seven disagreements, nearly all of them
`BROKEN` entries upstream refuses at parse time where we refuse at verify,
which is agreement rather than a fault.
Asked properly it found three, all in `VERIFIES`, and each is a rule this
does not have rather than only a bad fixture. An alias scope has two
operands or three, itself and its domain. An array says what it is an
array of, so `DW_TAG_array_type` without a `baseType:` is refused. And an
`addrspacecast` has to cross address spaces: `addrspacecast (ptr @r to ptr)`
is "invalid cast opcode" at parse time.
The fixtures are rebuilt from shapes the assembler was asked about first,
so each still tests what it meant to. The three rules are recorded and not
written: each wants its own measurement.
Then the block use list, left read and unchecked by the pass before and
recorded there as "its predecessors, derivable from the terminators'
successors". That was half of it, and the wrong half to have guessed at.
The count comes off the assembler directly: a directive assembles only when
its index count matches the list, so scanning `k` over
`uselistorder label %b, { k indexes }` reads the number out of the exit
code, and twenty shapes answered in a few seconds.
A block is used once per terminator *slot* that names it, so
`br i1 %c, label %b, label %b` is two and a switch with two cases to one
block is two. A phi's incoming blocks are not uses at all: upstream keeps
those beside the operand list rather than in it. The other half is not
predecessors but `blockaddress`, and that constant is uniqued per block, so
ten globals holding one block's address are one entry in its list, while a
directive that only *names* the constant does not use it.
The two directives are checked at different times, and it is the same
distinction the constant rule already draws. `uselistorder label` in a body
is checked at the end of its function, so a `blockaddress` written below
the function is not a use yet; `uselistorder_bb` is checked at the end of
the module and sees every one. Both placements were measured against both,
and the matrix agrees on all six rows.
The probe found a rule on the way. `uselistorder_bb` is a top-level
directive: written among the instructions, upstream calls it an unknown
opcode, where ours took it in either place.
Then the three rules the audit had recorded, and the first thing measuring
them settled was that there were only two. The probe that found them ran
`opt -S` without `-passes=verify`, so it had been asking a question the
verifier never heard: asked again with the verifier on, the alias scope
rule was already there and already right, both at one operand and at four.
That is twice now that a wrong oracle has manufactured a finding, and both
times the tell was the same, a result too tidy for how little had been
looked at.
The two that are real are small. `addrspacecast` has to cross address
spaces, since crossing them is the whole of what it does, and upstream says
so in two places rather than one: an instruction is caught on verifying, an
expression as it folds while being read, which is why the rule is written
twice here too. It looks through a vector, so a cast between two vectors of
pointers answers the way the pointers do. And `DW_TAG_array_type` has to
name a `baseType:`, alone among the composite tags: a structure, a union,
an enumeration, a class and a variant part are each read without one, which
is measured rather than assumed.
No bound moves for any of it. The suites do not test these shapes, which is
what the fixture audit exists to catch and why the rules were found by
asking upstream about our own tests rather than by running its.
Next the ceiling, the count of modules we refuse that `llvm-as` reads,
which STATUS calls the failure that matters. Two of the five looked like
parse gaps rather than blocked work, and one of them was.
`Assembler/block-labels.ll` writes `-N-:` and `$N:`, and neither reached
the label path: a leading `-` went to the number lexer and a leading `$` to
the comdat sigil. The character set was measured one character at a time,
leading and continuing separately, and it is
`[-a-zA-Z$._][-a-zA-Z$._0-9]*`. Upstream resolves the ambiguity by scanning
that set first and deciding afterwards, the colon being what says which
grammar was meant, and `word_or_label` already did exactly that for a
hyphen in the middle. Lifting it into `label_ahead` and calling it from the
two other starts is the whole fix, and `i16-1` stays a type and a negative
number because no colon follows it.
Assembler 475 to 476 with the ceiling 5 to 4, and its differential 189 to
190: the file is not only read now but printed the way upstream prints it.
The other of the two is a bigger rule than it looks.
`skip-value-numbers-globals.ll` fails on `@""`, which we call a
redefinition the second time it appears, and upstream does not call a name
at all: an empty quoted name is unnamed, and the value takes the next slot
number. Measuring it turned up the rest of the rule, which is that upstream
renumbers every unnamed global densely from zero, so `@5` and `@7` print as
`@0` and `@1` where we print what was written. That is a printing change as
well as a parsing one, and it took a pass of its own.
The model that fits every shape measured is two numberings rather than one.
Reading has a single counter over every kind in source order, and a
definition takes from it: `@N =` says which slot to start at and may only
skip ahead, `@"" =` takes the next. That is what makes the test's `@""`
after `@5` reachable as `@6`, and it is why `@7` then `@5` is refused where
`@0` then `@5` is not. Printing has a second, per-kind numbering: every
unnamed global, then the aliases, the ifuncs and the functions, each in
module order, renumbered from zero. A function written above the globals
still numbers after them, which is measured and not what definition order
would give.
The parse side is one rewrite rather than a rule threaded through: the
pre-scan turns an unnamed definition's token into the slot it took, so
everything after it sees one spelling and a reference by number finds it.
The print side is `ModuleSlots` beside the `FunctionSlots` that already
existed, and the slots module's own comment had said module-scope symbols
needed none of this.
Assembler 476 to 477 with the ceiling 4 to 3, its differential 190 to 191,
and CodeGen 22,384 to 22,385. The corpus is untouched, being `llvm-dis`
output already numbered from zero, which is the reassuring part: the change
only moves files that were written by hand.
Then the target extension types, which four Verifier files and one Assembler
file were waiting on and which the tree had recorded as a blocker twice, once
in `fits_in_a_global_step` saying whether one can be a global "is a property
of the target rather than of the IR". It is, and the assembler will say which
property, one question at a time. `corpus/target-extension-types.nu` asks
five: has a size, may be a global, may be an `alloca`, takes a
`zeroinitializer`, may be a vector element. Each is a module that assembles
only when the type has that property.
The shape of the answer is two levels. A namespace carries defaults, so
`target("spirv.anything")` is sized and global for any name at all and
`target("dx.anything")` is sized and global and takes no zero; a registered
name may then override them, which is why `spirv.Image` differs from its own
namespace. Probing an invented name is what separates the two, no override
being able to apply to a name nobody registered. Everything else has no
properties, which is what makes `target("foo")` unsized.
Two probes were wrong first, and the tree caught both. Asking about
`zeroinitializer` by writing `@g = global T zeroinitializer` also asks
whether `T` may be a global, so `aarch64.svcount` answered no to a question
about zero; passing the constant to a call instead isolates it. And a name
carrying required parameters is not a type without them, so
`target("riscv.vector.tuple")` read false for every property when the
parameterised spelling reads true for four. Both errors have the same shape
as the alias-scope one: a probe that fails for a reason other than the one
being asked about. The spellings are harvested whole from upstream's tests
now, and each name is probed through one the assembler accepts.
`x86_amx` came out of the same sweep. It was listed with `void` and `label`
as having no size, and it has one: upstream loads and stores it. What it may
not be is a function parameter, which is a different rule and was doing the
refusing in the probe that first suggested otherwise.
Verifier 316 to 320, its wrongly-read count 12 to 8. Nothing else moves, and
the generated table is byte-identical on regeneration, the generator running
`rustfmt` itself because a row is wider than the line limit.
The next cluster was `DIExpression`, four files across the two suites all
failing with "invalid expression", and it is recorded here as measured and
not written, which is a different thing from the blockers that were only
arguments.
The obvious shape is a per-opcode table: which numbers are opcodes and how
many operands each takes. Three probes were built for it and all three
measure something else. Writing `DIExpression(N, 0, 0)` and taking the
arities that verify says 80 through 143 accept every arity, because the
trailing zeros are themselves elements and a register op swallows what
follows. Anchoring with `DW_OP_deref` and asking again says everything takes
no operands, because the anchor is the number 6 and `DW_OP_plus_uconst` is
glad to take 6 as its operand. Spelling the anchor by name changes nothing:
a name is a number to the parser, so `DIExpression(DW_OP_plus_uconst,
DW_OP_deref)` is `plus_uconst 6` and verifies.
What settles it is a pair: `DIExpression(DW_OP_swap)` is refused and
`DIExpression(DW_OP_swap, DW_OP_deref)` is not. No arity explains that.
Validity is a property of the whole sequence, a stack the operations leave
in a shape the verifier will accept, so the rule is a small interpreter
rather than a table and it wants a pass of its own. What is measured and
worth keeping is that a standalone `!DIExpression` reached from a named list
is verified, which makes the oracle cheap for whoever writes it.
Chasing that lookup turned up two intrinsics missing from the name set
altogether, which is what decides whether an undeclared call is built into
a declaration or is "use of undefined value". `corpus/intrinsic-names.nu`
matches `@llvm.*`, and LangRef writes `declare i64 llvm.vscale.i64()` and
`declare ptrty llvm.ptrmask(...)` without the sigil every other mention
has. Two real intrinsics were therefore refused wherever a module called
one without declaring it.
The harvest reads a `declare` line's name with or without the `@` now, and
only a `declare` line: LangRef gives `llvm.loop` and `llvm.access.group`
headings of their own and they are metadata rather than intrinsics, so
harvesting headings would have us build a function declaration for a node
kind. 419 base names to 421.
`llvm.vscale` picked up its attributes from the same fix, the attribute
sweep having missed it for the same reason. `llvm.ptrmask` did not:
LangRef writes its declaration with `ptrty` and `intty` where a type goes,
which is a placeholder spelling the sweep does not instantiate, so it is
auto-declared without attributes. No file in the four print suites shows
that, the differentials being unmoved, so it is recorded rather than
worked around.
The earlier regression's recorded reason was corrected too.
`llvm.vp.cttz.elts` has entries of its own in the signature and attribute
tables, so the loose reading gave a wrong answer only where a name is
missing from the table being asked, which the tied-position table is. The
unit tests pin each table separately for that reason rather than asserting
the same thing three times.
Assembler 464 to 465 and Verifier 314 to 316, with the modules we wrongly
accept fourteen to thirteen and seventeen to twelve. The eleven trees are
back where they were.
Then the names themselves. An overloaded intrinsic carries the types it was
instantiated at in its own name, and a module may write the name without
them: upstream reads `declare void @llvm.lifetime.start(i64, ptr)` and
prints `@llvm.lifetime.start.p0`. We printed what the module wrote, which is
a difference on every such module, and there are many, most of them written
before opaque pointers when the pointer said what it pointed at and the
name did not have to.
Two halves, both measured. What a type spells is asked of `llvm.ssa.copy`,
which is overloaded on a single position that takes any first class type, so
`declare T @llvm.ssa.copy(T)` comes back named with the spelling of `T` and
one module answers one question. Three answers are not what the spelling
suggests: `token` and `label` are both `i0`, `metadata` is `Metadata` with a
capital, and a packed struct spells exactly like the unpacked one. `void` is
`isVoid` and cannot be asked directly, `declare void @llvm.ssa.copy(void)`
being refused, so every type goes inside a `target("w", T)` wrapper instead
and the spelling is read out of the middle. That is
`crates/llvm-ir/src/intrinsic/mangle.rs`, whose unit tests are the answers.
Which positions go in is `corpus/intrinsic-mangling.nu`: write a bare
`declare` of the base name with a documented signature, read back the name
upstream gave it, and match its components against the spellings of the
signature's own types. It is not a rule a signature yields.
`llvm.masked.load` is `<2 x double> (ptr, i32, <2 x i1>, <2 x double>)` and
its name is `llvm.masked.load.v2f64.p0`: the result and the pointer go in
and the mask does not, though the mask varies with the result as surely as
the passthrough does.
Where one instantiation leaves two positions spelling the same thing, the
answer is a signature LangRef does not document: the same one with a single
position moved to a neighbouring width or address space.
`llvm.invariant.start` takes a `ptr` and returns one, and only its answer at
`ptr addrspace(1)` says the name carries the argument's space and not the
result's. That mutation corrected two rows that were otherwise wrong, and
136 of the 239 rows have positions nothing measured tells apart, which for
tied positions is a distinction without a difference.
The rows are then held against upstream's own tests, all 37,134 intrinsic
declarations in `llvm/test`: apply a row and the name it builds should be
the one the test wrote. 1,066 disagree, which is expected, those being the
stale names this exists for, so each is put to the assembler. 986 come back
as names upstream rewrites exactly as we would, and three come back
contradicted and are dropped. That is what the drop is for: a row that
renames a function upstream leaves alone is worse than no row.
The parser rewrites a declaration's name after the calls have implied
theirs, using the same fit gate the attributes go through, and leaves the
written name alone where the canonical one is already taken. The verifier
needed one fix for it: `calls_intrinsic` compared whole names, so renaming
`llvm.va_start` to `llvm.va_start.p0` walked the "va_start in a non-varargs
function" rule past its own case. It compares the reduced name now, which
it should always have done, every module writing the components out having
been missed before.
Then the order, which the names alone left wrong. Upstream does not rename
a declaration in place: it builds a new function and erases the old, so a
renamed one prints after everything the module wrote. Renaming in place got
the name right and left the position wrong, which is still a difference, so
three of the four files this was for only moved from one kind of difference
to the other until this landed.
Moving a function means moving its id, which every constant naming it
holds, so `Module::function_order` records the order instead and the arena
keeps the ids. Attribute groups follow it, a group's number being its first
use as printed: a renamed declaration that moves to the end takes the last
number with it, which upstream does too. The module-scope slots do not
follow it and do not need to, an intrinsic being named and that numbering
counting only the unnamed.
Where the two kinds of declaration land is a third measurement.
`declare i8 @llvm.smax(i8, i8)` implied by a call prints *before* a renamed
`@llvm.lifetime.start.p0` even when the module wrote the second one first,
because upstream materialises an implied declaration where the call is read
and rewrites a written one at the end. So an implied one is given its
canonical name as it is built rather than renamed afterwards, which keeps
it out of the move.
Feature 67 to 70 and Other 141 to 143, each suite down to a single file
that differs for a reason that is not the name: one wants the attributes of
an nvvm intrinsic, the other a pass-timing report.
Then the eleven trees, all at once, because they were all failing at the
same thing. Recording our own error for every module upstream reads and we
refuse, and grouping by the shape of the message rather than its text, gives
one line: 483 refusals, 483 of them "reference to undefined symbol @llvm.*".
Three hundred and thirty names, and the namespaces say what they are:
amdgcn 132, dx 95, aarch64 90, spirv and riscv 30 each, x86 27, nvvm 22,
then coro 16, vector 15, experimental 9, eh and dbg 4.
An intrinsic needs no declaration, upstream recognising the name at the call
and building one from the call's own signature, and our name set was
LangRef's 421. Upstream knows far more. The coroutine, exception-handling
and debug-info intrinsics are documented in other files entirely;
`llvm.vector.interleave4` is documented nowhere, LangRef stopping at three;
and a target's intrinsics are documented only in its backend.
No probe was needed, which took a wrong turn to find out. The first oracle
built read the assembler's message, "invalid intrinsic signature" for a name
it knows against "use of undefined value" for one it does not, which is the
message and not the exit code. The exit code answers on its own: a name used
in a file `llvm-as` reads, where that file never gives the name a body, is a
name upstream recognised, because nothing else would let the module resolve.
So the derivation is a scan of the files it accepts and no probing at all.
Three things had to come out of that scan before it measured the right thing.
`@llvm.used` and `@llvm.global_ctors` are reserved globals rather than
intrinsics, and a filter that only looked at `declare` lines counted a global
definition as not giving the name a body. Half these files are FileCheck
lines quoting IR, where a name upstream never parsed says nothing about what
it recognises. And a symbol has to be read exactly as written rather than
trimmed to what a name may end with: `CodeGen/AMDGPU/wmma-gfx12-w32.ll`
declares `@llvm.amdgcn.wmma.i32.16x16x16.iu8.v8i32.v2i32.` with a trailing
dot and calls the name without it, so the call is undeclared and upstream
materialises it, while a scan that trimmed the dot read the typo as the
declaration and dropped the name. Two CodeGen modules said so, which is what
a ratchet at the full count is for.
The order of the two steps is what makes it finish: scan first, and ask the
assembler only about a file that offers a name nothing has offered yet.
Nearly every file offers nothing new, and running `llvm-as` on all
thirty-seven thousand of them was most of an hour; 435 files answer for all
1,790 names. Names are stored with their instantiation types dropped, which
is what a lookup reduces to and collapses the cost-model tests' thousands of
spellings into the names behind them.
`; Unknown intrinsic` came along with it. Upstream prints that above an
`llvm.` name it does not know, and we were printing it above every target
intrinsic, which upstream knows perfectly well. The printer asks the same
question the parser's gate asks now.
The reduction had to be rewritten before any of it was safe, and a unit test
found that rather than a ratchet. "A component with a digit in it is a
mangled type" is the loose statement of the rule, and loosely is wrong both
ways: `interleave4` is not a type, so `llvm.vector.interleave4` reduced to
`llvm.vector`, and `llvm.amdgcn.fdot2` reduced to `llvm.amdgcn`, which as a
stored key answers for every name that target has. What replaced it is the
grammar `mangle.rs` had already measured, read backwards: `i` and a width,
`p` and an address space, `v`/`nxv`/`a` and a count and then the element's
own spelling, `sl_` and fields and `s`, and a closed set for the rest. The
pair that pins it is `llvm.vector.interleave4` against
`llvm.vector.interleave9`: the same call shape assembles at four operands
and is refused at nine, and LangRef documents neither.
Every one of the eleven trees now reads every module `llvm-as` reads:
Transforms 10,232 to 10,305, Analysis 1,396 to 1,403, CodeGen 22,385 to
22,785, Instrumentation 505 to 508, and the other seven were already there.
Assembler 477 to 478 with the ceiling 3 to 2, which is the count that
matters, and it took sweeping every directory of `llvm/test` rather than the
eleven the tree ratchets cover: `llvm.amdgcn.ds.append` is named only in
`Assembler`, and a name is recognised or it is not whatever suite it appears
in. Verifier and the four differentials are where they were.
With the trees full, the print differences are what is left, and the same
histogram picked the next piece: 48 files, and the largest group is eleven
that want a `DICompositeType` carrying an `identifier` to be `distinct`. A
type with one is a type the language gives a single definition across every
translation unit, so upstream keeps one node per identifier.
Four measurements make the rule. An identifier is enough on its own, at
every tag, class through variant part. `identifier: ""` is dropped from the
output and buys nothing. Two nodes under one identifier are one node and the
first written wins, the survivor keeping its own name and both references
pointing at it. And a second node writing that identifier under a different
tag claims nothing at all: it neither merges nor becomes distinct, so the
lookup is keyed on the identifier alone with the tag checked against
whatever already holds it. Keying on the pair would have let that node claim
an identifier of its own, which is what the first attempt did.
A fifth was a wrong probe rather than a rule. `DW_TAG_array_type` seemed to
answer without needing a `baseType`, and what was being read was the
verifier's error dump, which prints the offending node and reads like output.
Same shape as the alias-scope and zeroinitializer probes: a module that
failed for a reason other than the one being asked about.
The rule then broke the corpus round trip, which is worth recording because
the two checks genuinely disagreed. `debug.ll` is `llvm-dis` output and holds
six identified composite types that are not distinct; upstream's own
`opt -S` on that very file makes all six distinct. So `llvm-dis` output is
not a fixed point of the textual reader, and the corpus was asking our
printer to reproduce something upstream's `opt -S` does not.
`corpus/regen.nu` canonicalises with `opt -S` after the bitcode round trip
now, for the same reason `check-differential.nu` already compares against
`opt -S`: that is the transformation this project implements. Across the
whole corpus it is twelve lines in one file, all of them this rule.
Its differential Assembler 191 to 192 and Linker 207 to 214.
Then the attributes, which is the same sweep pointed at a wider source. It
read LangRef's `declare` lines and got 370 intrinsics; upstream knows
thousands, so every module naming a target intrinsic printed no attributes
where upstream prints some. Reading the 40,636 `declare` lines upstream's own
tests write, as well as the 1,803 from LangRef, gives 11,842.
The readback answers a second question at the same time. Upstream writes
`; Unknown intrinsic` above an `llvm.` name it does not know, so a
declaration it does not say that about is one it does. That is the half
`corpus/intrinsic-recognised.nu` cannot see: it reads names used *without* a
declaration, which is what proves upstream built one, and a name every test
declares for itself never appears that way. 11,865 names, into
`intrinsic/declared.rs` beside the attributes, and `is_known` asks it too.
It also stops a test file's own attributes being recorded as an intrinsic's,
which is what an unrecognised declaration would otherwise have contributed.
Two bugs in the harvest, both from reusing LangRef's cleaner on real IR.
`clean-declare` strips an angle-bracketed word, that being how LangRef writes
an operand placeholder, and a scalable vector type is an angle-bracketed
word: `@llvm.masked.load.nxv4i1.p0(ptr, i32, <vscale x 4 x i1>, ...)` became
`(ptr, i32,,)`. And harvesting with `\([^)]*\)` stops at the first bracket,
so `ptr addrspace(1)` cut the line in half. A written line gets its own
cleaning now, which takes the attributes off and leaves the types alone.
A third was in the argument splitter and had been there all along, waiting
for a type that could reach it: the depth count knew `<`, `[` and `{` and not
`(`, so `target("dx.Layout", { float }, 4, 0)` split at its first space and
the rest of the type was recorded as an attribute. It reached the generated
file as an unquoted string and rustc refused it, which is the good failure;
the strings are escaped on the way out now so that the bad one stays
impossible.
Nine intrinsics disagree with themselves across instantiations and are left
out rather than guessed at, `llvm.ucmp` among them: its `range` attribute is
a property of the instantiation, `range(i8 -1, 2)` at one width and
`range(i32 -1, 2)` at another, so there is no one answer for the base name.
Differential Assembler 192 to 197, Linker 214 to 215, and Feature and Other
to 71 of 71 and 144 of 144, which is every module we both accept printed
exactly as upstream prints it. Those two are held rather than raised now.
Then the data layout, which a module naming a target gets whether it wrote
one or not: `target triple = "x86_64-unknown-linux-gnu"` alone comes back
carrying `target datalayout = "e-m:e-p270:32:32-..."`. That counts twice,
the layout being where the default alignments are read from, so a module
without one printed different alignments as well as a missing line.
`corpus/target-data-layouts.nu` writes each triple alone and reads back what
upstream put beside it, which is one module per question again. 719 triples
appear in upstream's tests, 679 imply a layout and 39 imply none, and there
are only 55 distinct layouts behind the 679. A triple with no row is left
alone, and so is a module that wrote a layout of its own, upstream replacing
neither.
This is coverage of what appears rather than of what could, and worth being
plain about: a triple no test names gets nothing. Deriving a layout from a
triple's parts instead means reimplementing every backend, and there is no
specification of that outside them.
Differential Assembler 197 to 198 and Linker 215 to 219 of 220.
The summary index turned out to be the opposite of what was recorded. Four
files looked like a trailing blank line, and what upstream actually does with
a `^0 = module: (...)` is drop it: `opt -S` reads the index and prints the
module without it, body and all else intact. The index is a thing beside the
module rather than part of it, and the only tool that writes one is
`llvm-dis`, which writes what the bitcode reader built rather than what was
read, path and hash from the file it opened and a `; guid` comment appended.
Printing back what the module wrote was neither, and a unit test had pinned
it: that test said so in its own comment, that the property was "ours"
because the corpus could not hold it. It was never put to upstream. It is
now, and it asserts the measured behaviour instead: the entries are read,
the verifier still checks them, and none of them prints.
Differential Assembler 198 to 207, nine files rather than the four the
histogram attributed to it, the rest having been counted under whatever
differed in them first.
The renames came next and turned up an ordering rule on the way in. Two of
the files filed under renaming were not renames at all: an implied
declaration prints in a different place than we put it. Ours were appended
in the order the calls first named them, which the comment in the pre-scan
said in so many words, and it is not what upstream does. Five intrinsics
called in reverse alphabetical order come back alphabetical, so it is a
sort; and `@llvm.umax` called at `i8` prints before `@llvm.umax.i32` though
`llvm.umax.i8` sorts after it, so the key is the name as written rather than
the name it ends up with. That is the shape of a forward reference held in a
map until the module is finished rather than a declaration built where the
call was read.
The renames themselves are `corpus/intrinsic-renames.nu`:
`llvm.wasm.laneselect` is read as `llvm.wasm.relaxed.laneselect`,
`llvm.arm.thread.pointer` as `llvm.thread.pointer`,
`llvm.arm.neon.vclz` as `llvm.ctlz`. 120 of them.
The sweep needed telling what a rename is. A batch of declarations goes in
and a batch of names comes out, and a renamed one cannot be matched by name,
so the two are compared as sets and the odd one out is paired with the odd
one in. The first run found 2,958 renames and 2,719 conflicts, which is a
measurement of the wrong thing: a *remangling* looks exactly the same from
outside, one name in and another out, and `llvm.smax.v4i32` was recorded as
renamed twice and differently. Comparing the two names with their
instantiation types dropped is what tells them apart, and it leaves 311
pairs and no conflicts at all.
Two files stay open and are recorded as tasks rather than worked around.
One declares both `llvm.aarch64.thread.pointer` and
`llvm.arm.thread.pointer`, which rename to the same thing; upstream merges
them into one function and we rename the first and leave the second, because
renaming it on top would leave two functions sharing a name. The other is an
argument count rather than a name: `declare i8 @llvm.ctlz.i8(i8)` is the
one-argument spelling of an intrinsic that now takes two.
Differential Assembler 207 to 209.
Two singletons after that, one rule each, gated together.
A `DIObjCProperty` keeps the name written as its setter under `getter` and
the one written as its getter under `setter`. It is exactly a swap: a lone
`setter: "S"` comes back as `getter: "S"`, and writing the two the other way
round changes nothing. Whatever upstream's reason, a module read through it
has them exchanged, so one read through us has to as well; it is done at
parse time because it is what the node holds rather than how it is written.
And `memory(...)` prints in one shape however it was written. The locations
go in a fixed order, `argmem` then `inaccessiblemem` then `errnomem`,
whatever order they came in. A location saying what the default already says
is dropped, the default being `none` when nothing states it. The default is
written only when it is not `none` or when nothing else is left to write,
which is what keeps `memory(none)` from printing as `memory()`.
A probe read the wrong thing again on the way, for the third time in this
tree and always the same way: `memory(argmem: write, read)` seemed to print
unchanged, and what was being read was the error saying the default has to
come first. Checking the exit code before reading the output is the whole
fix, and it is now how every probe here is written.
Differential Assembler 209 to 211.
Three more debug-info rules after that, and the first is a gap in how the
tables are kept rather than in what was measured. `corpus/md-field-order.nu`
prints the order it finds and the table in `metadata.rs` is transcribed from
that by hand, so a field no probe covers has no entry and no way to notice.
`baseType` was one: the structure probe carries no `baseType` and the array
probe no `file`, so between them nothing said where it goes and it sat next
to `name` where the array probe alone suggested. Upstream writes it after
`line`. An enumeration carries both and is a third probe now.
A composite type's `runtimeLang` takes a compile unit's language vocabulary
and was not wired to it, so `runtimeLang: 6` printed as a number where
upstream writes `DW_LANG_Cobol85`. Wiring it fixed the printing and one
acceptance case with it: upstream refuses a word the vocabulary has no
number for, "invalid DWARF language", and we had been taking any word.
And a `DIMacroFile` is the start of a file by being a `DIMacroFile`, so its
`type` is read and never written back, `DW_MACINFO_end_file` included.
`debug-info.ll` wanted a fourth and does not get it here. Upstream prints
the three ptrauth booleans of a `DW_TAG_LLVM_ptrauth_type` whether or not
they are false, which is easy, but the five ptrauth fields share storage
with other fields in a way that a table of defaults cannot express:
`ptrAuthIsaPointer: true` written alone comes back false, the same field
written beside `ptrAuthKey` comes back true, and `ptrAuthKey: 2` on a
`DW_TAG_pointer_type` comes back as `align: 2`. That wants a sweep of its
own and is recorded as one.
Differential Assembler 211 to 212.
Then the ceiling, which is the count that matters, and half of it came down.
A call site names an instantiation, so one written name can stand for more
than one declaration: `@llvm.umax` called at `i8` and at `i16` is two
functions and not one that has to fit both. We recorded the first call's
signature and checked every later call against it, so the second was "calls
an intrinsic with an incompatible signature", which is upstream's own
`implicit-intrinsic-declaration.ll`.
The awkward part is that a `FunctionId` is an index into the arena, so an
extra declaration cannot be conjured while parsing: the pre-scan reserves
every id before the first token is read, and everything after depends on
that count. So the pre-scan counts how often an implied name is mentioned
beyond its first and reserves a block that size, an upper bound rather than
a count, and a call whose signature is new takes the next id from it. What is
left over is never referred to and never built, and sits past the end of the
arena where nothing looks.
The order is measured as well, and it is two keys rather than one: by the
name the module wrote, and among the ones that wrote the same name, by the
name each ended up with. `@llvm.umax` at `i8` and at `i16` beside
`@llvm.umax.i32` comes back `i16`, `i8`, `i32`, which no single key gives.
Assembler 478 to 479 with the ceiling 2 to 1, and its differential 212 to
213.
The last one was `auto_upgrade_nvvm_intrinsics.ll`, and it wanted a
capability rather than a table: that module's own declarations disagree with
its calls, `declare i32 @llvm.nvvm.atomic.load.add.f32.p0(ptr, float)`
against `call float @llvm.nvvm.atomic.load.add.f32.p0(...)`, and upstream
never minds because by the time anything checks, the call is not a call.
`@llvm.nvvm.atomic.load.inc.32.p0(ptr %p, i32 %v)` is read as
`atomicrmw uinc_wrap ptr %p, i32 %v seq_cst, align 4`, and the type comes
from the value the call was given rather than from anything the declaration
claimed. Four of them, each measured a module at a time.
Three things about it had to be asked separately rather than assumed. The
declaration is dropped, and a declaration nothing calls is dropped too, so
the second is not a consequence of the first. The result loses its name,
upstream building a fresh instruction rather than editing the one that was
there, so `%r = call` comes back `%1 = atomicrmw`. And the alignment is
written out rather than left to the reader.
The declaration is kept in the model and left unprinted, which is what the
four debug-info intrinsics already do: removing a function would move every
id after it, and an id is what every constant naming one holds.
This is a table of four and not a sweep, which is worth saying plainly.
A rewrite is a fact about what an intrinsic means rather than about how it
is spelled, and there is no oracle that lists them: these are what upstream's
own tests exercise. The same file wants three other kinds of upgrade that are
not written, an added argument and a multi-instruction expansion among them,
and it prints differently for want of them. It is read, which is what the
ceiling counts.
Assembler 479 to 480 and the ceiling 1 to 0. Nothing upstream reads is
refused now. Its differential holds at 213 with the denominator one larger,
that file having joined the modules we both accept.
The fourth is `llvm.ptr.annotation`, and it is a limit of reading LangRef
rather than of the method: LangRef documents a four-argument form the
assembler does not recognise, and the one upstream's own tests call takes
five. Deriving the rows from the tests instead of the documentation would
reach it, and every target intrinsic with it, at the cost of a table three
orders of magnitude wider.
Its differential Other 140 to 141, everything else where it was.
The slop scanner had moved on under the tree meanwhile, and its new
findings were on code the last three commits had already passed it with,
which the parent commit's own tree confirms. The name reduction moved out
of `intrinsic/mod.rs` into `intrinsic/reduce.rs`, which is what that rule
actually asks for and leaves the module declarations where a reader looks
for them; the other two are recorded in `.deslop.toml` with reasons, a
sixty-line crate root not being an oversized file and a doc comment that
records how a rule was measured not being a tutorial.
The next pass took `DIExpression`, which was recorded as measured and not
written, wanting "the stack discipline of a DWARF expression, which is a
small interpreter rather than a table". That was wrong, and the way it was
got wrong is worth keeping: three probe designs had each measured something
other than arity, and the pair that settled it was `DIExpression(DW_OP_swap)`
refused against `DIExpression(DW_OP_swap, DW_OP_deref)` accepted, which no
arity explains. What no arity explains is one rule beside the table rather
than the absence of a table, and asking every opcode the same questions is
what shows that where asking one opcode many questions did not.
It is a table of 103 operations upstream reads, out of 189 codes it has a
word for, and four rules. An opcode may not stand alone, which is
`DW_OP_swap` and only it. Two have to be last, `DW_OP_stack_value` and
`DW_OP_LLVM_fragment`. Sixty-four end the checking: `DW_OP_reg0` through
`DW_OP_breg31` accept anything at all after them, where `DW_OP_regx` and
`DW_OP_bregx` do not, which is why that is a range rather than a notion.
Not quite anything: an operation written short of its operands after one of
those is where `opt` segfaults rather than answering, `DW_OP_reg0,
DW_OP_constu` among them, which is another crash upstream and so no verdict
at all. The walk stops at the register operation rather than reading on,
which agrees with every shape that does have one.
The fourth is the entry value, and it needed a dozen modules of its own.
`DW_OP_LLVM_entry_value` covers exactly one operation, whatever follows it,
so its operand is one and nothing else; and it has to be the first operation
or the one directly after a leading `DW_OP_LLVM_arg 0`. Like a register
operation it then ends the checking, which is what makes
`DW_OP_LLVM_entry_value, 1, DW_OP_deref, DW_OP_LLVM_entry_value, 2,
DW_OP_deref` a module upstream reads: the second entry value is never asked
about. Reading that as "an entry value may appear anywhere once one is
first" was the wrong shape and cost four more probes to correct.
Two questions per opcode rather than one, and the first is what the second
needs. Upstream writes an opcode as a word only for an expression it finds
valid, and a register operation ends the checking, so `DW_OP_reg0` in front
of any code makes it answer: the word comes back, and the elements written
after it come back as numbers where they are operands and as words where
they are the next opcode. That reaches every code upstream can write rather
than only the ones it verifies, which is what the printer needs and what no
validity probe can give: `DW_OP_LLVM_entry_value` is refused by every shape
the validity questions ask and still has an arity to print with.
The filler written after each code has to be an opcode with a word of its
own. Filling with nought could not tell an operand from the next opcode,
both coming back as nothing, and that made `DW_OP_LLVM_convert` look like it
takes one operand where it takes two. Which is a rule of its own: what a
conversion converts to is an encoding, so `DW_OP_LLVM_convert, 8, 5` comes
back `DW_OP_LLVM_convert, 8, DW_ATE_signed`, and it is the one operand
anywhere in an expression that upstream writes as a word.
With the words measured, the elements are stored as numbers, the way the
`tag:` fields have been since the eighty-second pass and for the same
reason: upstream's own error dump prints them as numbers, so that is what it
holds, and two nodes that differ only in the spelling are one node. A word
upstream does not know is a parse error now rather than something carried,
and which words it knows is not a guess: every `DW_OP_*` in the whole of
`llvm/test` is either in the table or one the assembler refuses, which the
generator checks by asking it. Reading an encoding's word anywhere in an
expression came out of that audit rather than out of the convert rule.
An element is unsigned, which came out of comparing the printing rather
than the accepting: `!DIExpression(DW_OP_reg0, -1)` printed here and did not
upstream, and the reason is that upstream refuses a signed element wherever
one is written.
Assembler 480 to 481 and Verifier 320 to 323, the ceilings staying at
nought, and the eleven trees unmoved.
Three more verifier rules went in beside it, each a module upstream was
asked about in both directions. A `range` on an `immarg` parameter
constrains the literal the call writes, half-open and unsigned, so
`range(i32 -3, 4)` refuses -4 and 4 while `range(i32 4, -3)` is the wrap
round the end and refuses nought. `llvm.stepvector` counts lanes, so it
counts into lanes wide enough to hold a count: integers of at least eight
bits, which makes `<vscale x 16 x i1>` too narrow where `<4 x i8>` is fine.
And a `!DILocation` says where in the source something came from, so an
attachment may be one and may not reach one through a plain node, on an
instruction or on a function; `llvm.loop` is exempt with its whole subtree,
and a global's attachments and a named list are not asked at all. That last
was recorded as unmeasurable because `llvm-as` aborts on `!prof` and
`!annotation` holding a location. It aborts on those two and answers for
every other kind, and a crash is not a verdict either way, so the rule is
written for all of them.
Assembler 481 to 482 and Verifier 323 to 325.
The pass after it took the two declarations that resolve to one name, which
the renaming pass had recorded as deliberate rather than closed. Upstream
builds one function per name, so a module declaring both
`@llvm.aarch64.thread.pointer` and `@llvm.arm.thread.pointer` comes back with
a single `@llvm.thread.pointer.p0` and two call sites naming it. The second
declaration is merged rather than renamed onto the first: its calls are
pointed at the survivor and it is left out of the print order. Only calls are
redirected, which is all there is, an intrinsic's address not being something
a module may take.
Leaving it out of the order is what took the model change. `function_order`
had been "the order the functions print in" with a guard treating any order
shorter than the arena as no order at all, so a dropped function silently
fell back to printing everything. It is what prints as well as in what order
now, which is the same thing the four debug-info intrinsics have needed since
the fiftieth pass and got a name test instead.
The type finder went with it, walking the print order rather than the arena:
upstream walks the module it is about to write, so a declaration it erased
takes its types with it and one it moved to the end meets them later.
Measured on a module declaring `%late @llvm.ssa.copy(%late)` above a function
taking `%early`, where upstream writes `%early` first. Nothing reaches that
yet, a named struct being a mangling this does not build, so no file moves.
Assembler differential 213 to 214.
The other half of that task was the argument count, and it took four probe
designs to measure. An intrinsic that has gained a parameter since a module
was written is read through an upgrade rather than refused: `declare i8
@llvm.ctlz.i8(i8)` comes back with an `i1 immarg` and every call to it with
an `i1 false`. `corpus/intrinsic-arity.nu` writes each `declare` line
upstream's own tests hold, calls it with what it says it takes, and reads
back what upstream made of the call, which needs no guess about which
arities ever existed: the old spellings live in those tests because that is
what they test.
What the four designs were is the useful part. Calling with constants makes
a synthesised argument indistinguishable from one upstream folded out of
what was passed, so `llvm.x86.avx512.mask.load.d` read as gaining a
passthrough it had actually computed; calling with the probe function's own
parameters tells them apart, because anything worked out of them comes back
mentioning one. It also tells a drop from an expansion: an intrinsic
upstream rewrites into other instructions leaves them behind, where the same
call on constants folds to nothing and reads exactly like a call upstream
removed. A drop is only recorded for an intrinsic returning nothing, because
a call that folded to a constant leaves no instruction either and removing
that would leave whatever read the result with nothing to read. And the
arguments written have to have survived in order: `llvm.nvvm.rotate.b64`
comes back as an `llvm.fshl.i64` whose operands upstream worked out, which
reads as the same call with an argument appended unless they are checked.
Ninety-two declarations upstream reads at an older arity, six it drops
outright, six hundred and thirty-eight it rewrites into other instructions,
which is a transformation rather than a table and is reported rather than
guessed at. The name is recorded as written and both sides are reduced by
the measured grammar at lookup: reducing in the sweep with a loose rule
turned `llvm.aarch64.sve.ld2.sret.nxv16i8` into `llvm.aarch64.sve`, which as
a key answers for every intrinsic that target has.
A call carries the callee's function type as well as its arguments, and
widening one without the other is a call nothing reads; the fixture table
caught that, which is what it is for.
No ratchet moves. `auto_upgrade_intrinsics.ll` is down from forty differing
lines to ten, and the ten are one thing: upstream finds an intrinsic by the
longest known prefix of the name, so `llvm.objectsize.i32.unnamed` is
`llvm.objectsize` with the rest ignored, where the reduction here stops at
the first component that is not a mangled type. That is the same reduction
question a named struct raises and is recorded with it.
The pass after it took the ptrauth fields of a `DIDerivedType`, which had
been recorded as needing a sweep of its own and does: the five of them share
the slot `align` uses, and the tag is what decides which name the slot prints
under. Writing them on a `DW_TAG_pointer_type` and reading the number back
out of `align` is what shows the layout, one bit at a time. The key is bits 0
to 3, whether the address discriminates is bit 4, the extra discriminator is
bits 5 to 20, whether it is an isa pointer is bit 21, and whether it
authenticates null values is bit 22. Read the other way, `align: 2097153` on
a ptrauth type comes back `ptrAuthKey: 1, ptrAuthIsaPointer: true`, which is
the same word and is what pins it.
Two rules go with the layout and neither is a shape a table of defaults can
hold. The ptrauth spelling fills the slot only when a non-zero key is
written, so `ptrAuthIsaPointer: true` on its own comes back false, which is
what the task recorded as inexplicable, and `align: 16, ptrAuthKey: 0` keeps
the sixteen. And a ptrauth type writes its three booleans back whether or not
they are false, where every other tag writes an `align` and only when it is
not nought.
The key is four bits wide in storage and the field is limited to seven, which
had to be measured separately: `align: 15` comes back `ptrAuthKey: 15` where
`ptrAuthKey: 15` is refused, "limit is 7".
`corpus/md-field-order.nu` gained the two probes that would have said where
these fields print, and one for `num_extra_inhabitants`, which was the last
line of `debug-info.ll` to differ after the ptrauth fields landed. Both are
the shape that file's own comment already warns about: a kind whose fields
cannot all be written at once needs a probe per shape, and a field no probe
carries has no entry and no way to notice.
Assembler differential 214 to 215, and `debug-info.ll` is byte-identical.
The pass after it took the parameters a target extension type is allowed to
have, which `Assembler/target-type-param-errors.ll` was the last file in that
suite waiting on. `corpus/target-extension-types.nu` already asks the
assembler five questions per name and this is a sixth: a grid of nought to
two type parameters against nought to three integer ones, each cell a module
that assembles or does not.
The answer is smaller than the question. Three of the thirty-eight names
upstream's own tests spell insist on a shape, and each insists on exactly one
cell: `aarch64.svcount` takes no parameters, `amdgcn.named.barrier` takes one
integer, `riscv.vector.tuple` takes one type and one integer. Everything
else, an unregistered name included, takes whatever it is given, so a name
with no row needs no check.
A type parameter is written as each of four spellings and a cell counts as
accepted when any of them assembles, so a name wanting a vector is not
recorded as wanting no parameters at all. Whether `riscv.vector.tuple` also
insists on *which* type is a further question this does not ask, upstream
reporting only the counts here.
Assembler 482 to 483 of 483, which is every file in that suite agreeing, with
the ceiling still at nought.
The pass after it read LangRef's `declare` lines an eighth way, and it is the
half `corpus/intrinsic-overloads.nu` was throwing out. Two positions whose
types vary together are one overloaded type, which that table already says;
two whose *lane counts* vary together are one shape without being one type,
which it did not. A mask is `<4 x i1>` where the value it masks is
`<4 x double>`, so nothing about the types ties them and the lengths are tied
all the same: `llvm.masked.load` is documented at sixteen lanes, at two and
at eight, and its mask is as wide as its result in each.
The reading is the same one with a different comparison, so the same two
guards apply: a position that is not a vector in every documented
instantiation has no lane count to tie, and one whose count never varies is
fixed rather than tied. 103 intrinsics have lane classes against 161 with
type classes, and the two tables sit beside each other rather than being
merged, a call being able to get one right and the other wrong.
Verifier 325 to 326, with the modules we read that llvm-as refuses down to
two. Both of those are the prefix reduction task 18 records.
The pass after it took the funclet token, which is a table of twenty-five
and reads as a family until it is measured. Windows exception handling runs a
catch or a cleanup as a funclet of its own, and a call upstream may lower
into a real function call has to say which funclet it is in;
`Verifier/operand-bundles-wineh.ll` is `llvm.objc.retain` called without one.
Six probes said the objc intrinsics need a token and `llvm.memcpy`,
`llvm.trap`, `llvm.stacksave`, `llvm.eh.typeid.for` and
`llvm.launder.invariant.group` do not, which is a family and a guess.
`corpus/intrinsic-funclet.nu` asks all of them: 33,681 declarations, each
called from inside a funclet twice, once with a bundle and once without. A
name refused both ways is refused for a reason that has nothing to do with
the bundle, which most of them are, and only a name refused without it and
read with it needs one. 21,452 answered, and the twenty-five that need a
token are every `llvm.objc.*` there is and nothing else.
Which blocks are inside a funclet is a colouring rather than a lookup: every
block reached from one a pad opens, stopping where a `catchret` or a
`cleanupret` hands control back out. A block reached both from a pad and from
the entry is a module upstream refuses for its colouring rather than for
this.
Verifier 326 to 327, and there is one module left in the two suites that we
read and llvm-as refuses.
The pass after it went back to `corpus/md-field-order.nu` with the question
the last two passes kept running into: which fields no probe carries. It is
answerable by comparing each kind's probe against the schema, and ten kinds
had one.
`DISubrange` was the one that showed: a subrange is described from one end or
the other and never both, so the probe carrying `count` cannot carry
`upperBound`, and upstream writes `upperBound` between `lowerBound` and
`stride` where the table had nothing. Three DebugInfo files print identically
now that it does.
Nine more fields found their place with it. A DWARF address space goes
between a derived type's `extraData` and its `annotations`, and only a
pointer may carry one, so that is a probe of its own. A global variable's
`declaration` goes before its `templateParams`, and wants a static data
member rather than another global. A label's `isArtificial` and
`coroSuspendIdx` follow its column, a location's `inlinedAt` precedes
`isImplicitCode` and its `atomGroup` and `atomRank` follow, a subprogram's
`targetFuncName` and `keyInstructions` come last, and a composite type's
`specification`, `enumKind` and `bitStride` follow `annotations` in that
order, which took a probe carrying all three because no other probe carries
two.
One of the ten was not a gap at all but a field we accept and upstream does
not: `!DILocalVariable(tag: ...)` is "invalid field 'tag'" there and was in
the schema here. Written out with a scope, so that neither refuses it for
missing one, we read a module upstream refuses. That is the fourth time an
order probe has found an acceptance bug rather than a printing one.
The pass after it took the reduction, which two files were waiting on and
which turned out to be one rule rather than the two the task recorded.
Upstream finds an intrinsic by the longest prefix of the name it knows and
ignores whatever follows: `llvm.objectsize.i32.unnamed`, `llvm.objectsize.zzz`
and `llvm.objectsize.i32.p0.zzz` all come back `llvm.objectsize.i32.p0`, and
`llvm.ssa.copy.s_tys` is an `llvm.ssa.copy` whatever `s_tys` is. So the
mangled-type grammar is not what decides where a name ends.
The prefix goes last in the candidate sequence rather than first, and only
LangRef's documented names count as one. Both halves are measured rather than
chosen. Asked first, a prefix would answer for the wrong intrinsic wherever a
table is missing the longer name, `llvm.memcpy.element.unordered` not being
an `llvm.memcpy`. And asked of the recognised or declared tables, which store
what a reduction gave them, it reads their artefacts back: `llvm.dbg.label`
reduces to `llvm.dbg` because `label` is also a type spelling, so `llvm.dbg`
is in the recognised table, and taking that as a prefix made every
`llvm.dbg.*` an `llvm.dbg`. Two Linker files said so.
Two more things came with it. A declaration is rebuilt once however many
things about it changed, so one the arity upgrade already moved stays where
that pass put it rather than moving again when it is renamed. And the rename
is two-phase: whether a canonical name is taken depends on what the other
declarations end up called, not on what they are called now.
`Assembler/remangle.ll` is two declarations whose canonical names are each
other's and upstream swaps them, where a module writing two spellings of one
intrinsic has one merged away. What tells those apart is whether the name is
held by a declaration that keeps it.
The last of it is the shape the second bound exists for. Refusing
`Verifier/memset-pattern-unsized.ll` for a name we did not recognise scored
as agreement, and recognising the name exposed the rule that was standing in
for: a memset pattern is written into memory however many times it fits, so
it has to have a size, and `target("foo")` has none where
`target("spirv.Event")` does.
Assembler differential 215 to 217, Verifier holding at 327 with the rule that
replaced the wrong one, and the Linker differential back where it was.
The pass after it took the artefact the one before had to work around. The
closed set the reduction reads a digit-free type spelling from held `label`,
`token` and `metadata`, and the measured mangling says a label and a token
both spell `i0` and metadata spells `Metadata`, so none of the three can ever
be a component. What put them there was harvesting the words that follow a
documented name, which cannot tell a type from the last word of a name.
`label` was the one that cost something: the only name in the tree ending in
it is `llvm.dbg.label`, so the recognised sweep stored `llvm.dbg`, and any
rule reading a prefix out of that table made every `llvm.dbg.*` an
`llvm.dbg`. Regenerating with the three gone is one line each way, `llvm.dbg`
out and `llvm.dbg.label` in.
`void` stays, and the difference is evidence rather than taste: it follows
`llvm.experimental.deoptimize` and `llvm.experimental.patchpoint`, both real
intrinsics, so there it is the older spelling of a result that `isVoid` is
the current one for. The other three follow `llvm.dbg.label`,
`llvm.return.token`, `llvm.uses.token` and `llvm.random.metadata`, of which
the first is a name and the rest are test inventions.
`corpus/intrinsic-attributes.nu` needed nothing: it reduces with a regex that
never had the four in it, which is why `declared.rs` carried no such entry.
That the two scripts spell the same rule differently is worth knowing and is
recorded rather than fixed here.
Nothing moves, which is the point: the artefact was already worked around by
asking only documented names for a prefix, and this is the same answer with
nothing to work around.
The pass after it took the other half of the ODR rule, which the first pass
of this queue had left: a type that gives itself an identifier has one
definition across every translation unit, and so do its members.
`Assembler/dicompositetype-members.ll` says so in its own comment, two
members of an identified type written in different files being one member
where the same pair under an unidentified type are two.
The key was measured a field at a time, and it is not the symmetric thing it
looks like. For a `DIDerivedType` it is the tag and the name, so two members
differing only in `file:`, `line:` or `size:` merge and two differing in tag
do not. For a `DISubprogram` it is the linkage name alone: two with different
names and one linkage name merge, and two with one name and no linkage name
do not. A node with no key merges with nothing, and no other kind merges at
all, a nested composite type having its own identifier rule and an
enumerator no scope to be a member of.
A `distinct` node is its own node whatever it holds, and leaving that out
cost four Linker files before it went in: a subprogram's definition is
`distinct` and shares both its linkage name and its scope with the
declaration it came from, so without the guard the definition merged onto
the declaration. The merged members are held out of the structural pass as
well, since two that merged here differ structurally by the file they were
written in if nothing else.
Assembler differential 217 to 218.
The pass after it took the last Linker file, which is one flag standing in
for four. A Swift compiler once wrote its own version into the Objective-C
collector flag, so `!{i32 4, !"Objective-C Garbage Collection", i32
83953408}` is not a collector configuration at all: the low byte is, and
upstream splits the rest out into `Swift ABI Version`, `Swift Major Version`
and `Swift Minor Version` beside it.
Which bits are which was swept rather than read off the one file: bits 8 to
15 are the ABI, 16 to 23 the minor and 24 to 31 the major, which
`0x05010700` coming back as ABI 7, major 5 and minor 1 pins against
`0x06010600` coming back as ABI 6, major 6, minor 1. The `Swift Version`
flag the same file carries has nothing to do with it and survives untouched,
which one probe without it settled.
Two more things the sweep said and the file did not. The upgrade fires on a
value wider than eight bits whatever the behaviour says, and the behaviour it
leaves is always `Error`, where the module wrote `Override`. And a flag
written `i8` already is left alone entirely, keeping the behaviour it had, so
narrowing is not what triggers it.
Linker 219 to 220 of 220, which is every module we both accept printed
exactly as upstream prints it. Three of the four print suites are there now,
Feature and Other being the others.
The pass after it widened the mangling table the way the attribute table was
widened. `corpus/intrinsic-mangling.nu` measured which positions feed a name
by writing out a signature LangRef documents, so it knew 239 names where
upstream recognises 11,865 in a declaration, and a name with no row prints as
the module wrote it where upstream fills in its types. The tests are the
other source and the script already read them, to hold its rows against, so
the same lines now go in as signatures too: a test's declaration is a
signature like any other once its name is reduced to the base. 239 rows to
1,567.
Three probe designs had to be fixed before that was sound, and each was
invisible while LangRef was the only source. A round that comes back silent
says nothing, where before it discarded the base: with dozens of signatures
per base, one batch refused for an unrelated line threw away every heavily
declared target intrinsic. A base declared at two arities is two entries, not
one, because a row is keyed on the arity and accumulating one assignment
across both gives positions that index past the shorter signature.
And the assignments are counted rather than intersected. That is the one that
matters: with one or two signatures per base every one of them was right, so
intersecting was sound, and upstream's tests bring signatures their own
module never compiles, so a single odd one emptied the intersection and took
the base with it. `llvm.memcpy` lost its row that way, which the unit test
caught and the Assembler ceiling caught again at 0 to 1. The assignment the
most signatures support is the answer, and the mutation probes still cut down
whatever ties.
Assembler differential 218 to 219, `remangle.ll` being a module whose two
`llvm.ssa.copy` declarations upstream swaps and which now prints exactly as
it does.
Measured and not done: interning the attribute table.
`crates/llvm-ir/src/intrinsic/attributes.rs` is 2.5 MB and 95,000 lines,
one row per intrinsic with its attribute strings written out in full, and
11,842 rows carry only 540 distinct sets between them. Emitting the sets
once and rows of `(name, index)` makes it 582 KB, and a clean
`cargo build -p llvm-ir` goes from 4.1 seconds to 3.3; with the table
removed altogether it is 3.0, so eight tenths of a second is what the
indirection would buy and one and a tenth is all the table costs. That is
under half a minute across the whole twenty-three-check gate, which runs
for over an hour, against a permanent indirection in the one artifact a
reader consults to see what an intrinsic carries. Not worth it, and the
numbers are here so it stays a decision rather than a thing nobody
measured.
The pass after it made the three sweeps spell one reduction. Each of them has
to cut a written name down to the base its table is keyed on, and each had
its own idea of what a component of a mangled type looks like:
`intrinsic-recognised.nu` used the measured grammar, `intrinsic-attributes.nu`
a regex that knew neither `ptr` nor the float spellings, and
`intrinsic-mangling.nu` still the rule an earlier pass measured wrong, that a
component with a digit in it is a type. That is the rule which reduces
`llvm.vector.interleave4` to `llvm.vector`, and the pass before this one had
widened that script's input by eleven thousand test declarations while it
reduced with it.
All three carry byte-identical copies of the grammar now: a closed set of
spellings, `sl_` and a run of them for a literal struct, `i` or `p` and
digits, and `nxv`, `v` or `a` with a count and an element that has itself to
be a spelling. That last clause is the whole difference. `v2` is not a vector
of anything, so `llvm.aarch64.sve.bfdot.lane.v2` is a name rather than
`llvm.aarch64.sve.bfdot.lane` instantiated at one, and `bf8` is not a float,
so `llvm.amdgcn.cvt.f16.bf8` keeps its last component too. 126 over-reduced
names leave the recognised set and 53 whole ones join it, 11,865 to 11,792,
and the attribute rows go 11,842 to 11,768.
The mangling table gains 560 rows, 1,567 to 2,127, every one of them a name
whose last component has a digit and is not a type: `llvm.aarch64.neon.tbl1`,
`st1x2`, `vcadd.rot270`, `frint32x`. It loses 27, and those are worth naming
because they were never intrinsics at all. Upstream's own tests write
`llvm.aarch64.sve.bdep.x.nx16i8`, misspelling `nxv16i8`, and the loose rule
read the misspelling as a type and made a base out of what was left.
Rows dropped for disagreeing with the assembler go 32 to 5, which is what
says the reduction was behind them rather than the mangling ambiguity they
were blamed on: `llvm.amdgcn.image.atomic.swap.1d` has its row now.
Assembler differential 219 to 220.
The pass after it gave `llvm/test/DebugInfo` a print differential of its own,
which is the tree the debug-info work keeps moving and the one where three
files started printing identically without any number recording it. 50 of
the 57 modules at the top of that tree print exactly as upstream prints
them, and the seven left are four causes rather than seven:
Four of them are one rule we do not have. Upstream's verifier has a
debug-info half whose failures do not refuse a module, they strip every bit
of debug info out of it and warn: "ignoring debug info with an invalid
version (0)" in `strip-DIGlobalVariable.ll`, "missing global variable type"
in `pr34186.ll`, "invalid type ref" in `pr34672.ll`, and "definition
subprograms cannot be nested within DICompositeType when enabling ODR" in
`cross-cu-scope.ll`. We read all four and print the debug info back.
`sroa-handle-dbg-value.ll` is one parameter, written `noalias nocapture
sret(%T)`, which upstream prints as `noalias sret(%T) captures(none)` and we
print as `noalias captures(none) sret(%T)`. The printer sorts the attributes
that take an argument by a six-name list and everything outside it ties, so
what comes out is the order the module wrote them in.
`unrolled-loop-remainder.ll` defines `attributes #0` twice, once with
`readnone` and once without, and upstream's answer carries `memory(none)`
where ours does not. Whether that is the later definition winning or the two
merging is a question one probe settles, and neither is what we do now.
`type-finder-w-dbg-records.ll` is four named struct types used nowhere but
inside the constant expressions a `#dbg_value` and a `#dbg_assign` record
carry. Upstream's type finder walks those; ours walks instructions.
Each of the four is a task rather than a paragraph here, and the ratchet
holds 50 in the meantime.
The pass after it closed the two mangling gaps the widened sweep left, and
one of them was already closed by the reduction: `amdgcn-image-atomic-
attributes.ll` prints identically now, because `llvm.amdgcn.image.atomic
.swap.1d` was a name whose last component the loose rule ate rather than an
ambiguity between two positions that always spell the same.
The other was `llvm.ptr.annotation`, and it turned out to be LangRef being
stale rather than anything about mangling. Three names are documented at one
arity and recognised at another, and the whole set is small enough to name:
`llvm.ptr.annotation` and `llvm.var.annotation` are documented with four
arguments where upstream refuses that form outright, "Callsite was not
defined with variable arguments!", and recognises the five-argument one it
renames to `llvm.ptr.annotation.p0.p0`; `llvm.donothing` is the other way
round, documented with an argument and recognised with none, upstream
answering "Intrinsic has incorrect argument type!" to the documented form.
Each was asked both ways, which is what says the measured arity is the
recognised one rather than merely another one that works.
So a documented signature of a different arity from the measured one says
nothing now, where it used to refuse, and refusing was costing the
attributes as well as the name: `apply_intrinsic_attributes` asks the same
question with the measured arity in hand.
The lookup had a second bug behind the first. A base declared at two arities
gets a row for each, which is what the sweep was fixed to do, and
`positions` searched by name alone and returned whichever row the search
landed on. It takes the arity now and looks inside the run of rows that
share the base, so the arity is asked rather than checked afterwards.
No differential moved, `opaque-ptr-intrinsic-remangling.ll` still wanting
the struct-returning expansion, and the six files left in the Assembler
differential are six causes: that expansion; the attributes a variadic
intrinsic declaration gets, which
`amdgcn-unreachable.ll` wants; the nvvm upgrades that change the shape of an
instruction rather than a name, in `auto_upgrade_nvvm_intrinsics.ll`; the
alignment a target extension type implies, which `target-types.ll` writes
out and we leave off; a debug record upstream drops in
`drop-debug-info-nonzero-alloca.ll`, which belongs with the stripping above;
and the order the predecessor comment lists blocks in, which `uselistorder
.ll` permutes and we print in block order.
The pass after it built the half of upstream's verifier that does not refuse
a module. A debug-info failure is not an error there: upstream says what is
wrong, takes every bit of debug info out of the module and reads what is
left, which is why `DebugInfo/pr34186.ll` comes back with no debug info at
all rather than as a diagnostic. `llvm-as` does the same, so it is not one
tool's habit, and both tools read all four of the files this pass closed.
The stripping itself was already here for a module whose `Debug Info
Version` flag is not 3, and it was missing the `!dbg` on a global. What is
taken out, measured by feeding upstream a module that is valid apart from one
node: the attachments on globals, functions and instructions, the debug
records and `llvm.dbg.cu`. What is left is ordinary metadata, so a node
another named list still names survives, and one broken subprogram takes the
other function's line numbers with it, the strip being the whole module's.
Four rules, and every one of them needed the shape that passes measured
beside the shape that fails, because a rule measured only where it fires is a
rule that strips everything. The first draft of three of them stripped 97 of
the 1,093 modules in that tree that upstream keeps.
A global variable that is a definition has to have a type: `pr34186.ll` has
none and upstream says "missing global variable type". A declaration does
not, which `BPF/extern-void.ll` is, describing a variable it knows only the
name of, and `isDefinition:` left out means a definition.
Whatever names a type has to name one: `pr34672.ll` has `type: !3` where
`!3` is a `DIFile`, and upstream says "invalid type ref". The same question
at a local variable and at a derived type's base, where the field may be
absent or `null`: `baseType: null` is how a pointer to void is written, and
asking whether the field is *there* rather than whether it *names something
else* was most of that 97.
A definition subprogram scoped inside a `DICompositeType` with an identifier
is what `cross-cu-scope.ll` has, and upstream says "definition subprograms
cannot be nested within DICompositeType when enabling ODR". Four shapes said
what the rule is: the plain one strips, marking the composite `distinct` does
not save it, and a `declaration:` naming the in-class declaration does, which
is what every C++ member definition in `llvm/test/DebugInfo/COFF` carries.
And `spFlags: 0` is not a definition while no `spFlags` at all is one, the
older `isDefinition:` defaulting to true, which upstream shows by refusing a
`!DISubprogram` written with neither unless it is `distinct`. The bit is 8,
read back from `spFlags: 8`.
Both directions over the tree, which is the check that found each of those:
of the 1,093 modules we and upstream both read, there is now not one we
strip and upstream keeps, and ten we keep and upstream strips, each of them
a rule not written yet. The distinct messages in that tree are the rest of
the list: an array type with no base type, a `DIFlagAllCallsDescribed` on
something that is not a definition, a subprogram definition with no compile
unit, an invalid `!dbg` attachment, an invalid expression, an invalid file,
an invalid compile unit, a `DVRAssign` in another function, an atom group
without key instructions, and a subprogram scoped below an ODR type through
a lexical block, which upstream calls an invalid local scope.
Assembler, Feature, Linker and Other unmoved, and no module we strip that
upstream keeps in any of them either, nor in the 22,524 of `CodeGen` we both
read, which is where a rule that fires too widely would show up first.
DebugInfo differential 50 to 54.
The pass after it measured the order an attribute set prints in, which the
printer had been half guessing at. A set is held sorted rather than written
back as it was read, so `noalias nocapture sret(%T)` comes back
`noalias sret(%T) captures(none)`, and the printer knew that for six
keywords out of a hundred: `allockind`, `allocsize`, `memory`, `alignstack`,
`uwtable`, `vscale_range`, with everything else tying and keeping whatever
order the module wrote.
`corpus/attribute-order.nu` asks pairwise. Two attributes on one
declaration, read back to see which comes first, both ways round so that the
answer is upstream's order rather than the order the probe wrote. 4,950
pairs, 2,435 of them placed, no pair coming back both ways and no cycle,
which is what says there is one order rather than a rule per position.
A pair no position reads is an absent edge rather than a refused module,
which is most of the other 2,515: `byval` and `sret` are two ways of passing
the same argument, `range` wants an integer where `nofpclass` wants a float,
and `memory` is a function's where `align` is a parameter's, so an order
between them is not a thing to get wrong. Upstream's own error messages
supplied three positions the first draft was missing: `returned` needs a
result of its argument's type, `immarg` is refused outside an intrinsic and
beside anything but `range`, and `jumptable` needs `unnamed_addr`. Two
keywords stay unplaced for reasons upstream states: `elementtype` it allows
only on a call site, and `nocapture` never comes back at all, being read as
`captures(none)`.
The finding was in the half nobody was asking about. The bare keywords are
not alphabetical either: upstream prints `noalias noundef nonnull readonly`,
and `noundef` before `nonnull` is not what sorting the keywords gives. The
table covers all hundred rather than the nineteen the task was about, and
`compare_attributes` is now the measured rank and the quoted ones last.
DebugInfo differential 54 to 55, `sroa-handle-dbg-value.ll` being one
parameter written `noalias nocapture sret(%T)`. Nothing else moved, and the
corpus still reproduces byte for byte.
The pass after it settled a one-probe question the file it came from could
not answer. `DebugInfo/unrolled-loop-remainder.ll` writes `attributes #0`
twice, once with `readnone` and once without, and upstream's answer carries
`memory(none)`, which is the second. That file cannot tell the later
definition winning from the two merging, the second set being the first plus
one attribute, so it was asked with two disjoint sets: `#0 = { norecurse }`
then `#0 = { nounwind }` comes back `{ nounwind }` alone. The last one wins
and nothing merges.
Three more shapes, because a rule read off one probe is a rule read off one
probe. A conflicting pair, `noinline` before `alwaysinline`, is not
diagnosed at all: the earlier definition is gone rather than in conflict.
Three definitions keep the third. And a use written between two definitions
takes the later one, so this is the module's last word on a number rather
than what was in force where the number was used. `llvm-as` agrees with
`opt` on every one of them.
We kept the first, the lookup being a search for the number. The parser
replaces the entry now, which is what upstream holds: one group per number.
DebugInfo differential 55 to 56.
The pass after it took the last DebugInfo file, and the question was where
upstream's type finder walks rather than what it does when it gets there.
`type-finder-w-dbg-records.ll` defines four named struct types mentioned
nowhere but inside the constant expressions two debug records carry, and
upstream prints all four where we printed none.
One module answered five questions at once, each type mentioned in exactly
one place and a control mentioned nowhere. Walked: a function's personality,
a metadata node attached to an instruction, a debug record's operands, and a
node an ordinary named list reaches. Not walked, from a second module of the
same shape: a metadata attachment on a global, one on a function, a
`DIArgList`'s operands inside a record, and a `DICompileUnit`'s
`retainedTypes` chain. A third said the two absences are the same rule: a
tuple two deep inside a named list is reached, so nesting is not the limit
and entering a specialized node is what upstream does not do.
The order came out of a fourth. A record sitting above the first of two
instructions that carry attachments gives the first instruction's type, then
the record's, then the second's, which no pass over the records of their own
could give: a record is walked with the instruction it sits above and after
that instruction's own metadata. The named lists come last, after every
function.
DebugInfo differential 56 to 57, which is every module at the top of that
tree printed exactly as upstream prints it. Four of the five print suites
are there now; Assembler is the one with anything left.
The pass after it was meant to be about `...` and turned out to be about the
key. `Assembler/amdgcn-unreachable.ll` declares
`llvm.amdgcn.cs.chain.p0.i64.i32.i32(ptr, i64, i32, i32, i32 immarg, ...)`
and upstream gives it `convergent noreturn nounwind` where we gave it
nothing, and the reason was not the variadic gate in
`intrinsic_declaration_fits`: that name has no row in the attribute table at
all.
Upstream's own tests declare it at three arities, five fixed parameters, six
and eight, all variadic, all with the same function attributes and parameter
attributes of their own. The sweep grouped every instantiation of a base
name together, found the parameter lists disagreeing and dropped the name,
which is the same mistake the mangling table had before it was keyed on the
arity. The attribute table is keyed on the shape now, one row per base,
arity and whether it ends in `...`, and the conflicts fall from ten to
three: `llvm.scmp` and `llvm.ucmp`, whose `range` return attribute is a
property of the instantiation, and `llvm.amdgcn.interp`, which really is
written two ways at one arity.
The `...` is part of that shape rather than a parameter, which two probes
settle: `llvm.assume(i1)` gets its attributes and `llvm.assume(i1, ...)`
gets none, and a `cs.chain` with the wrong number of fixed parameters gets
none either. So the lookup asks for the declaration's own arity and its own
variadic-ness, and the blanket refusal of a variadic declaration is gone.
The sweep found a second thing on the way, which is why the row it wrote for
that name carried `"}"` as a parameter attribute: the argument splitter
counted `<`, `(` and `[` and not `{`, so a literal struct parameter split
into as many arguments as it had fields. Six corpus scripts carried a copy
of that splitter, the same way three carried a copy of the reduction, and
all six count the brace now. The two tables built from LangRef alone,
`table.rs` and `overloads.rs`, regenerate byte-identical, so nothing
LangRef writes has a struct in an argument list.
Assembler differential 220 to 221.
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
- Commit style: `feat(safety/oxidized/llvm): ...`, one scope per commit, no co-author
  trailers, and run `rtk jj diff --stat` before writing the message.
- Before committing Rust: `cargo fmt`, then
  `cargo clippy --workspace --all-targets -- -D warnings`. The pre-commit hooks
  run both and the abort-retry cycle is slower than doing it first.
- `deslop scan safety/oxidized/llvm` catches AI-slop patterns; `.deslop.toml` records the
  rules that are disabled and why. Run it *before* the nix checks rather than
  at commit time: a finding blocks the commit, and the fix for one is a source
  change, which invalidates every check derivation and costs the whole run
  again. The scanner indexes one file at a time, so a call through a sibling
  module reads as unresolved: prefer rewriting the call to disabling the rule,
  which is what `for (code, spelling) in dwarf::ENCODING` is doing in
  `metadata/expression.rs`.
