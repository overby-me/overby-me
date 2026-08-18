---
name: agentdocs
description: Scaffold an agentdocs/ documentation structure for a project. Analyzes the target codebase and writes seven topic-split convention files (project, workflow, layout, architecture, style, testing, this) plus a linking CLAUDE.md.
triggers:
  - phrase: "/agentdocs"
  - phrase: "scaffold agentdocs"
  - phrase: "set up agentdocs"
  - phrase: "create agentdocs structure"
---

# Agentdocs

Convention setup for a project's docs.

## Purpose

Scaffold an `agentdocs/` directory of topic-split convention files for a project, in the format used by `agentwerk`. Read the target codebase first; write rules that name real identifiers in it. The goal is documentation an agent can load file-by-file based on its task, not a single monolithic CLAUDE.md.

## When to Use

1. **Manual invocation**: user types `/agentdocs` (acts on cwd) or `/agentdocs <path>` (acts on a specific project root)
2. **Setup**: user asks to "set up agentdocs", "scaffold conventions docs", or "split CLAUDE.md by topic"
3. **Proactive**: a project has a CLAUDE.md but no topic-split convention files, and the user is about to add another large rules section

## What It Produces

Seven files under `<target>/agentdocs/`:

- `project.md`: vision and design principles
- `workflow.md`: build, test, release commands
- `layout.md`: where code lives
- `architecture.md`: invariants that shape how code fits together
- `style.md`: naming and comment rules
- `testing.md`: how tests are organized and written
- `this.md`: how the agentdocs files themselves are written

Plus an updated or created `<target>/CLAUDE.md` with a "Conventions" section linking to each file.

## Workflow

### Step 1: Resolve Target

- Default the target to the current working directory.
- If the user passed an argument, treat it as the target.
- Verify the target looks like a project root: at least one of `README.md`, `Makefile`, `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `pom.xml`. If none, ask the user to confirm.

### Step 2: Inventory

- Read `README.md` end to end.
- List the top-level directory tree (one level deep, then a second level for source roots).
- Read every build/manifest file you found in Step 1.
- Skim the source root: top-level files, public modules, test layout.
- Identify the language, the source root path (e.g. `crates/<name>/src/`, `src/`, `lib/`), and any visible conventions (formatter, lint config, test runner).

### Step 3: Create the Directory

- Run `mkdir -p <target>/agentdocs/`.
- If files already exist under `agentdocs/`, list them and ask the user before overwriting any.

### Step 4: Write `this.md`

Copy the template below verbatim into `<target>/agentdocs/this.md`. Substitute `<source-root>` in the "Code grounding" section with the actual source root path you identified in Step 2.

````markdown
# This

How every file under `agentdocs/` is written. This file is itself an example of the format.

## File shape

**One topic per file. Start with a title and a one-sentence description.**

- `# Title`: one word or short phrase, no trailing punctuation.
- One sentence under the title that states what the file covers.
- Sections use plain headings: `## Title Cased Heading`. No numbers: adding a section must not force renumbering.
- Each section is self-contained; a reader can skip to it directly.

## Section shape

**Bold rule first. Bullets second. A closing sentence is optional.**

- The first line after the heading is a bold one-liner stating the rule.
- The rule is an instruction, not a description.
- Bullets follow, optionally preceded by a one-line framing sentence.
- A closing sentence is added only when it carries information the bullets do not.

## Bullets

**Three to five bullets per section. One line each. Imperative voice.**

- Start with a capital letter; end with a period.
- Lead with the verb or with the thing being forbidden.
- Two short sentences per bullet are acceptable; longer bullets are not.
- Nested bullets are used only under a parent line ending in a colon.

## Enumerations

**Use bullets, not tables.**

- Tables produce wide rows that are hard to compare.
- For `name: description` pairs, write `` `Name`: description. ``
- Group related bullets under a one-line header ending in a colon.
- Code fences are acceptable for commands and small code examples.

## Voice

**Direct and neutral. No marketing language. No unnecessary jargon.**

- State the rule; justify only when the rule is not obvious on its own.
- Prefer present tense and second person over passive voice.
- `>` blockquotes are reserved for callouts at the top of a file.

## Emphasis

**Use MUST for non-negotiable rules. Use IMPORTANT for easy-to-miss gotchas.**

- MUST: correctness-critical rules where a violation breaks compilation, the wire protocol, or an architectural invariant.
- IMPORTANT: prefixes a bullet that a reader skimming would miss and regret later.
- Most rules need neither: the bold one-liner is already the rule.
- SHOULD, MAY, and CAN are not used: RFC-2119 without the full spec is noise.

## Code grounding

**Rules name identifiers that exist in the project. No invented vocabulary.**

- A type, function, field, or method named in a rule MUST be greppable in `<source-root>`.
- Verbs describe what the code does, not how it feels: avoid "wires", "magic", "ergonomic", "seamless".
- A rule that cannot point at code is opinion, not architecture: drop it or move it to the consuming application.
- When a name changes in code, the docs change in the same commit.

## Cross-linking

**Each fact lives in one file. Other files link to it.**

- Commands belong in `workflow.md`; other files link to it rather than restating commands.
- File and module placement belongs in `layout.md`; `architecture.md` describes invariants and assumes placement is known.
- Naming and comment rules belong in `style.md`; `testing.md` covers test-specific naming and links out for the rest.
- A duplicated fact is a future inconsistency: when two files would say the same thing, one of them links instead.

## Length

**If agentdocs are getting too long, consolidate them. Information loss is acceptable.**

- Drop sections that restate what a careful reader of the code would already see.
- Merge two short, overlapping sections before splitting one long section.
- A rule that has not earned its line is cut, not rewritten shorter.
- Skim cost matters more than completeness: a forgotten file teaches nothing.
````

### Step 5: Generate the Other Six Files

Write each file in turn, applying the rules in `this.md`. For every file:

- Start with `# <Title>` (one word, no period) and one sentence stating its scope.
- Use `##` plain headings; no numbering.
- Each section: bold one-liner rule, then 3–5 imperative bullets.
- Every named identifier must be greppable in the project. Verify before writing.
- Cross-link rather than restate (e.g. `testing.md` links to `workflow.md` for commands).

Per-file scope:

- `project.md`: design principles. What the project is, what shapes its API surface, what it explicitly does **not** do. 4–6 sections.
- `workflow.md`: commands the developer runs. Build, format, test, release, and any project-specific runners. Lift commands verbatim from the Makefile or package scripts.
- `layout.md`: where code lives. One section per top-level source directory; one section for tests; one section for any internal-only modules.
- `architecture.md`: invariants that shape how code fits together. The boundaries between modules and the rules that hold across them. Drop anything that just restates layout.
- `style.md`: naming and comment rules. Type/function/field naming, error and event variant conventions, doc-comment rules, README structure if there is one.
- `testing.md`: how tests are organized and written. Test layers, naming, what is mocked vs. real, coverage shape. Links to `workflow.md` for the commands.

If a section would be empty for this project (e.g. the project has no release process), drop the section rather than write filler.

### Step 6: Update or Create CLAUDE.md

- If `<target>/CLAUDE.md` exists, locate the conventions/rules section. If one is absent, append a new `## Conventions` section. If one exists, replace its body with the link list.
- If `<target>/CLAUDE.md` is absent, create one with a short header (project name + one-sentence description from the README) and the conventions section.

The conventions section body is exactly:

```markdown
## Conventions

The project's conventions are split across the following files. Read the one matching your task.

- [agentdocs/project.md](agentdocs/project.md): vision and design philosophy.
- [agentdocs/workflow.md](agentdocs/workflow.md): build, test, release commands.
- [agentdocs/layout.md](agentdocs/layout.md): where code lives.
- [agentdocs/architecture.md](agentdocs/architecture.md): rules that shape how code is organized.
- [agentdocs/style.md](agentdocs/style.md): naming, comment, and README style.
- [agentdocs/testing.md](agentdocs/testing.md): how tests are organized and written.
- [agentdocs/this.md](agentdocs/this.md): how the agentdocs files themselves are written.
```

## Principles

1. **Rule-first prose**: every section opens with a bold one-liner. Bullets justify or unpack it.
2. **Code-grounded**: a rule that does not name an identifier in the target project is opinion, not architecture. Drop it.
3. **One topic per file**: when two files would say the same thing, one of them links instead.
4. **Imperative voice, no decoration**: no em dashes, no marketing adjectives, no borrowed metaphors.
5. **Skim cost over completeness**: drop anything a careful reader of the code would already see.

## Example

A snippet of generated `layout.md` for a small Rust crate with `src/`, `src/provider/`, and `tests/`:

```markdown
# Layout

Where code lives and the rules that govern placement.

## Top-level files

**Each top-level source file is one concern the caller observes directly.**

- `lib.rs` holds public re-exports only.
- `error.rs` defines the `Error` enum and the `Result` alias.
- `client.rs` holds `Client` and its builder.
- `util.rs` holds shared helpers used by two or more sibling modules.

## The `provider/` module

**Contains every backend plus the shared transport code.**

- `trait.rs` defines `Provider` and the request/response types.
- `http.rs` and `mock.rs` are concrete providers.
- `retry.rs` holds the shared retry policy.

## Tests

**Unit, integration, and inline tests live in three separate locations.**

- `tests/unit/` uses the mock provider; bundled by `tests/unit.rs`.
- `tests/integration/` hits a real backend; bundled by `tests/integration.rs`.
- Every module also carries its own `#[cfg(test)] mod tests`.
```
