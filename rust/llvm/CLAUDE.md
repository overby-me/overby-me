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
