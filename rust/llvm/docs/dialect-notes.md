# Dialect notes

Where LLVM-rs deliberately differs from upstream, and why. Anything not
listed here is meant to match; a difference found in the wild is a bug until
it appears in this file with a reason.

The dialect is LLVM 21 with opaque pointers.

## Accepted on purpose

**The `; ModuleID = '...'` comment is read, not ignored.** Upstream treats it
as a comment and regenerates it from the input path, so `llvm-dis a.bc` and
`llvm-dis b.bc` of the same module differ in line 1. Keeping it makes a round
trip exact and makes a corpus file self-describing. Nothing downstream reads
it.

**Unnamed values and blocks may be numbered non-consecutively.** Upstream
renumbers on output and, for values, rejects a gap. We keep a map from the
number as written and assign print slots independently, which accepts
strictly more input and changes nothing for canonical files. The cost is that
we accept some modules upstream rejects, which shows up in the
`llvm-upstream-assembler` count.

## Modelled syntactically rather than semantically

**Specialized debug-info nodes.** `!DISubprogram(...)` keeps its tag and its
fields as written, in order, rather than becoming a typed object. Every field
survives a round trip, including ones this tier has never heard of. Typed
debug info belongs in `llvm-debuginfo` at T1.

**Attributes with bespoke argument grammars.** `memory(...)`, `captures(...)`,
`nofpclass(...)`, `uwtable(...)`, `allockind(...)` and `initializes(...)` keep
their argument text. The keyword itself is checked, so an attribute upstream
adds is a loud error rather than a silent drop; only the inside of the
parentheses is uninterpreted.

## Not accepted

**Typed pointers.** `i8*` is an error naming the dialect, not an alias for
`ptr`. PLAN.md section 1.2.

**Constant expressions upstream removed.** `add`, `sub`, `mul`, `select`,
`icmp` and the casts that went with them are errors naming the opcode. What
remains is `getelementptr`, the surviving casts, `extractelement`,
`insertelement` and `shufflevector`.

**Use-list order directives.** `uselistorder` is an error. Re-emit without
`-preserve-ll-uselistorder`.

## Known gaps, measured

`llvm-upstream-assembler` and `llvm-upstream-verifier` run upstream's own
suites and hold an agreement count. The two ratchets are the honest number,
and the gap is a to-do list rather than a divergence. As of 2026-07-27 the
recurring reasons are:

- Semantic rules the verifier does not have yet, which is most of the
  Verifier suite: we accept modules upstream rejects.
- Structural checks the parser does not make: duplicate symbol definitions,
  alignment bounds, attribute argument validation.
- Syntax outside this tier: module summary index entries (`^0 = ...`),
  target-specific calling conventions, metadata integers wider than 64 bits,
  and the `align(16)` spelling of a parameter attribute.

Each of those is a bug, not a decision. When one is fixed the ratchet moves
up in the same commit. Two passes so far have taken the Assembler suite from
146 to 175 and the Verifier suite from 70 to 116, by adding the structural
rules the parser was missing and then the semantic ones that come in
clusters: which types can be stored, which may only cross an intrinsic
boundary, and what shape the globals upstream reserves have to have.
