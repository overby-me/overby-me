---
name: deepcopy
description: Port or reimplement a reference codebase into the current project, tests-first. Copies and expands the reference's tests as stubs, derives pseudo-code from the reference, implements bottom-up until every test passes, then audits the finished port against the reference and iterates. Use when reimplementing code from another repo or language.
triggers:
  - phrase: "/deepcopy"
  - phrase: "port this"
  - phrase: "reimplement this"
  - phrase: "port from"
  - phrase: "port the reference"
---

# Deepcopy

Port a reference implementation, tests-first.

## Purpose

Take a reference (another repo, module, file, or snippet, given via `$ARGUMENTS`) and rebuild it in the current project, possibly in a different language or framework, so the result is faithful, well-tested, and no more complex than it needs to be. The reference's *tests* are the contract. The reference's *code* is the algorithm to distill, not transcribe.

Related but distinct:

- **`/reduce`** simplifies code that already exists in this project.
- **`/propose`** explores design options before any code exists.
- **`/deepcopy`** reconstructs code from an external reference, tests first.

## When to Use

1. **Manual invocation**: `/deepcopy <path-or-url-to-reference>`
2. **Cross-language port**: reimplement an algorithm or engine from one language in another
3. **Vendoring without copy-paste**: rebuild an external dependency's behavior inside the project rather than transcribing it line for line
4. **Proactive**: user says "port this", "reimplement this from `<repo>`"

## Workflow

Four stages, run in order. Each stage ends at a natural checkpoint (compiles, tests fail as expected, tests pass, audit clean) so the user can review before the next begins. Do not skip ahead. Implementing before the pseudo-code is derived produces a transcription, not a port.

### Stage 1: Tests as Stubs

The reference's tests define what "done" means, so they come across first.

- Locate the reference's test suite. Translate each test into the target project's language and test idiom, preserving intent and assertions verbatim where possible.
- Brainstorm additional tests: edge cases the reference under-covers, behaviors the target's idioms expose, boundary and anchoring cases. List them explicitly and add them alongside the ported ones.
- Audit the resulting suite with `/deeptest`. Confirm each test has a clear purpose, a name that reflects it, and an API-level focus. Fix vague names and low-level setup noise now, while the tests are the only thing written, because they are the contract the rest of the port is measured against.
- Define the public API as stubs only (types, signatures, module exports) with every body an unimplemented marker (`todo!()`, `throw new Error("unimplemented")`, `pytest.fail(...)`).

**Checkpoint**: the project builds and the new tests run and fail with the unimplemented marker. Report which reference test files were ported and which tests were newly added.

### Stage 2: Derive Pseudo-Code

Distill the reference's algorithm into the stubs before writing any real code.

- For each reference function, distill its abstract recipe via `/reduce`: name what it does, drop source-specific setup, name the minimal state.
- Place the distilled recipe as comments inside the corresponding stub, immediately above the unimplemented marker. Label each block so coverage can be grepped (e.g. `// mirrors <Module>.<function>`), keeping the reference-function to port-location mapping 1:1.
- Where several reference functions collapse into one target function, group their blocks under sub-headings inside that one body. Document functions intentionally *not* ported with a one-line `not ported: <reason>` note.
- Translate features that have no direct analogue in the target into an open-coded equivalent, and record each substitution in a table. Use the reference for *semantics*, never for syntax.
- **Escalation point**: the reference may lean on a capability the target lacks (a weaker standard library, a missing language construct, a built-in the reference got for free), or reproducing a behavior may seem to require a **new dependency**. Do **not** silently pick a substitution or add the dependency. Stop, lay out the options (open-code it / pull in dependency X / accept a behavior gap) with their tradeoffs, and ask the user how to proceed. Resume only after they choose.

**Checkpoint**: still builds, tests still fail as expected, every reference function has a labeled home or a `not ported` note.

### Stage 3: Implement

Replace the stubs with real code, layer by layer.

- Work **bottom-up** so each layer is testable before the next arrives.
- After each layer, run the test suite and report the expected progression, naming which tests now pass.
- Match the target project's style and mirror neighbor conventions.
- If implementation reveals a need for a **new dependency**, or a standard-library gap appears only here, apply the same escalation rule as Stage 2. Stop, present the options and tradeoffs, and ask the user before adding anything.

**Checkpoint**: all ported and brainstormed tests pass. Build, lint, and format are clean.

### Stage 4: Audit Against the Reference and Iterate

A passing suite is necessary but not sufficient. Now compare the finished port against the reference.

- Re-read the reference end to end. Produce a findings table: for each reference symbol, its port location, any divergence or simplification, and a verdict (faithful / justified / gap / bug).
- For every **gap**, add a test that exposes it, then fix.
- For every **divergence**, decide on purpose. Keep it and note why it is a justified simplification, or correct it.
- Iterate until the table shows no unresolved gaps and every divergence is justified.

**Checkpoint**: findings table with all rows resolved. Full suite green.

## Principles

1. **Tests are the contract.** Port the reference's tests before writing any implementation. They define done.
2. **Distill, don't transcribe.** Read the reference for semantics. The port's code shape is the target language's, not the source's.
3. **1:1 coverage, then justified deletions.** Every reference function gets a home or an explicit `not ported` reason. Nothing is silently dropped.
4. **Bottom-up to green.** Implement and verify layer by layer, not all at once.
5. **Audit closes the loop.** The final pass compares against the reference and resolves every divergence on purpose.
6. **Substitute openly.** When a source feature has no target analogue, open-code it and record the substitution.
7. **Escalate, don't assume.** A missing target capability or a needed new dependency is the user's call. State the options and tradeoffs and wait for a decision before committing.

## Output Shape

```markdown
## Stage 1: Tests

- Ported: <reference test files> → <target test files>
- Added: <new test>, covering <edge case>

## Stage 2: Reference to port mapping

| Reference symbol | Port location | Notes |
|---|---|---|
| <Module.fn> | <file:fn> | mirrors / collapsed-into / not ported: <reason> |

### Substitutions

| Reference feature | Target substitution |
|---|---|
| <feature with no analogue> | <open-coded equivalent> |

## Stage 3: Test-pass progression

1. After <layer>: <n> tests pass
2. After <layer>: <n> tests pass

## Stage 4: Audit findings

| Reference symbol | Port location | Divergence / simplification | Verdict |
|---|---|---|---|
| <Module.fn> | <file:fn> | <none / what changed> | faithful / justified / gap / bug |
```

## Example

**Input**: `/deepcopy ~/dev/opengrep/libs/aliengrep` (OCaml), reimplemented as Rust `crates/agentwerk/src/codegrep/`

**Output** (compressed across the four stages):

````markdown
## Stage 1: Tests

- Ported: `Unit_Pat_parser.ml`, `Unit_Match.ml` → `unit_pat_parser.rs`, `unit_match.rs` (21 tests)
- Added: `test_pure_ellipsis` boundary at end-of-input; case-insensitive backreference
- Public API stubbed: `Conf`, `Token`, `Node`, `Pattern`, `Match`; every body `todo!()`
- Checkpoint: `cargo test -p agentwerk codegrep` builds, 21 tests fail with `todo!`

## Stage 2: Reference to port mapping

| Reference symbol | Port location | Notes |
|---|---|---|
| `Pat_lexer.compile`/`read_string` | `token.rs:tokenize_*` | one hand-rolled scanner, no regex |
| `Pat_parser.parse_seq_until` | `ast.rs:parse_seq_until` | cursor instead of return-rest tuple |
| `Pat_compile.to_regexp` + helpers | `matcher.rs:search/match_seq/match_node` | collapsed; recursive descent |
| `Pcre_util.*`, `Log_aliengrep.*` | none | not ported: PCRE escaping / debug logging |

### Substitutions (escalated to user: "open-code, do not add a PCRE crate")

| Reference feature | Target substitution |
|---|---|
| `(?&name)` subroutine recursion | direct function recursion |
| `\g{N}` backreference | `MetavarEnv` lookup + byte-range compare |
| `\b` word boundary | `not_in_word(left, right)` token-adjacency check |

## Stage 3: Test-pass progression

1. After `Conf::check`: 2 conf tests pass
2. After tokenizer + parser: 8 parser tests pass
3. After matcher: all 21 pass; `clippy` + `fmt` clean

## Stage 4: Audit findings

| Reference symbol | Port location | Divergence / simplification | Verdict |
|---|---|---|---|
| `Pat_compile.long_ellipsis_pat` | `matcher.rs:match_ellipsis` | dropped `(?:\n WS)??` padding | justified, because the target tokenizer emits `Newline` directly |
| `Pat_compile.def_not_in_word` | `matcher.rs:not_in_word` | none | faithful |
| `Match.search` overlap handling | `matcher.rs:search` | zero-width guard added | gap, covered by `test_pure_ellipsis` |
````
