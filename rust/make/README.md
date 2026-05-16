# rust-make

A from-scratch Rust reimplementation of GNU `make` aimed at passing the
upstream GNU make test suite.

## Status

**135/135 upstream tests passing (100%)** against the GNU make 4.4.1 test
harness. Subtest totals:

- MAKEFLAGS: 218/218
- temp_stdin: 8/8
- options/dash-f: 32/32
- features/parallelism: 13/13
- targets/WAIT: 14/14

Parser, expander, and build engine total ~10k LoC. Parallel execution
covers phony-leaf forking, completion-order reaping, per-target
`.NOTPARALLEL`, `.WAIT` barriers, and recipe-less aggregator expansion
(deduping shared leaves across sibling non-leaf prereqs).

## Running the tests

A single test, locally:

```bash
cd /tmp/make-work/make-4.4.1/tests
perl run_make_tests.pl -make $PWD/../../../rust/make/target/debug/make <category>/<name>
```

Full suite with failure listing:

```bash
bash /tmp/run-make-baseline.sh > /tmp/base.txt
```

Via Nix:

```bash
nix build .#checks.x86_64-linux.rust-make-test-<category>-<name>
nix log    # for failure output
```

## Upstream test suite layout

GNU make's tests are Perl scripts driven by `tests/run_make_tests.pl`,
organised in six directories under `tests/scripts/`:

| Category    | Count | Focus                                                                 |
| ----------- | -----:| --------------------------------------------------------------------- |
| `features`  |    42 | Core semantics: conditionals, includes, parallelism, double-colon, … |
| `functions` |    31 | Built-in functions: `$(call)`, `$(eval)`, `$(foreach)`, `$(shell)`, …|
| `misc`      |     9 | General smoke tests, UTF-8, error handling, bs-nl, fopen failure     |
| `options`   |    20 | Command-line flags: `-C`, `-f`, `-n`, `-k`, `-I`, `-W`, `--eval`, …  |
| `targets`   |    12 | Special targets: `.PHONY`, `.DEFAULT`, `.ONESHELL`, `.SECONDARY`, …  |
| `variables` |    21 | Built-in variables: `MAKEFLAGS`, `SHELL`, `MAKELEVEL`, `CURDIR`, …   |

Excluded from scope:

- `tests/scripts/vms/` — OpenVMS-specific, not applicable.
- `test_template` — skeleton.
- `features/guile`, `features/load`, `features/loadapi`, `features/archives` —
  require Guile / dynamic C extensions / `ar`. Counted as skipped candidates.

## What's implemented

### Parsing

- Variable assignments: `=`, `:=`, `::=`, `:::=`, `?=`, `+=`, `!=`.
- `define … endef` multi-line variables; `override define`.
- `override`, `undefine`, `override undefine`.
- `export`, `unexport`, `export VAR = val`, `unexport VAR = val`,
  `.EXPORT_ALL_VARIABLES`. An `unexports` set records explicit
  `unexport` names so they're suppressed during `.EXPORT_ALL_VARIABLES`
  / global export and also removed from env_inherited re-export.
- Conditionals: `ifeq`, `ifneq`, `ifdef`, `ifndef`, chained `else ifX`.
- `include`, `-include`, `sinclude` with glob and `-I` search.
- Rules:
  - Explicit (`targets: prereqs`) with prereq, order-only (`|`), inline
    recipe (`; cmd`), recipe-body lines.
  - Pattern rules (`%.o: %.c`).
  - Static pattern rules (`targets: target-pattern: prereq-patterns`).
  - Grouped targets (`targets &: prereqs`).
  - Double-colon rules (each `::` rule runs its own recipe).
  - Target-specific variables with `private`/`override`/`export`
    modifiers; the `export` modifier scopes the export to the
    owning target's recipe.
- `.RECIPEPREFIX := X` overrides the tab prefix for subsequent rules,
  tracked during preprocessing so custom-prefix recipe lines correctly
  preserve backslash-newline for the shell.
- Inline-recipe detection on rule lines: scans `after-colon` text for
  the first top-level `;` vs. `#` respecting `$(...)` / `${...}` nesting.
  The recipe text itself is opaque to comment stripping.
- Backslash-escape unescaping in target / prerequisite name tokens
  (`\#`, `\:`, `\<space>`), expansion-aware so `path = pre\:` then
  `$(path)foo` produces target `pre:foo`. Handles odd/even backslash
  runs correctly. Embedded `:` in expanded target names triggers GNU's
  `target pattern contains no '%'` static-pattern fatal error.
- Variable assignment values: `\#` is unescaped to `#` outside of
  `$(...)`/`${...}` references.
- UTF-8 BOM stripped from the start of a loaded makefile.
- Backslash-newline continuations:
  - Non-recipe: collapse surrounding whitespace to a single space.
  - Recipe: preserve `\<nl>` so the shell handles continuation; strip
    leading tab on continuation lines for trace output.
- Bare `$(…)` expression lines (`$(info)`, `$(error)`, `$(eval)`).
- Nesting-aware whitespace split for rule targets, prerequisites, and
  order-only prereqs: `$(filter %.o,$(files))` is kept as a single
  token instead of being split on internal spaces.
- Recipe lines inside a conditional body attach to the most recently
  declared rule — matching GNU make's line-by-line splice when the
  taken branch is flattened.

### Expansion / functions

- Text: `subst`, `patsubst`, `strip`, `findstring`, `filter`,
  `filter-out`, `sort`, `word`, `wordlist`, `words`, `firstword`,
  `lastword`.
- Filenames: `dir`, `notdir`, `suffix`, `basename`, `addsuffix`,
  `addprefix`, `join`, `wildcard`, `realpath`, `abspath` (with path
  normalization).
- Conditionals: `if`, `or`, `and`, `intcmp` (arbitrary-precision via
  string comparison; supports the 4-arg `lt,ge` form).
- Control: `foreach` (binding writes into var scope so `$(eval)`
  inside the body sees the iteration), `call` (dispatches to built-ins
  too), `value`, `eval`, `origin`, `flavor`, `let`.
- I/O: `shell` (sets `.SHELLSTATUS`, re-exports env-inherited vars
  using current expanded values; falls back to original process-env
  value when the body self-references; child stderr re-emitted under
  the `make:` prefix), `file`, `error`, `warning`, `info`.
- `info`, `warning`, `error`, `eval` receive the full argument text
  including commas (single-argument functions per GNU make semantics).
- Fatal errors for invalid `word`/`wordlist`/`intcmp`/`foreach`/`let`
  args with GNU-compatible diagnostic text.
- Substitution references `$(VAR:from=to)` and `$(VAR:a=%b)`.
- `\<newline><whitespace>` sequences inside `$(…)` / `${…}` references
  are collapsed to a single space during expansion.

### Variable system

- Flavours: recursive, simple, immediate-recursive (`:::=`).
- Origins: undefined, default, environment, file, command line,
  override, automatic.
- `-e` boost for environment origin.
- Env-inherited names re-exported to child processes with current
  makefile value (respects `SHELL` exception).
- Dynamic `.DEFAULT_GOAL`, `.VARIABLES`, `.INCLUDE_DIRS`.
- `MAKEFILE_LIST` accumulated as each file is loaded.
- `.EXTRA_PREREQS` with target-specific override and glob expansion.
- `.LIBPATTERNS` with non-pattern element warning.
- `.SHELLSTATUS` populated after `$(shell)`.
- `:::=` marks names for "recursive-style" `+=` append.
- Target-specific variables propagate to prerequisite builds via
  scope stack. `:=` target-specific assignments expand at declaration
  time so they capture caller-side bindings.
- Target-specific `+=`, `?=`, `unexport`, `override`, `private`,
  `override +=`, `export` all supported with full GNU semantics.
- Pattern-specific variables (`%.x: VAR = value`): match targets by
  pattern; when multiple patterns match, shortest-stem wins. Supports
  all modifiers and operators.

### Rules engine

- Explicit, pattern, static-pattern, and grouped-target handling.
- Escaped-`%` in static pattern rules: `\%` literal in target/stem/prereq
  patterns.
- GPATH support: when a vpath-resolved directory is listed in `GPATH`,
  the target redirects to the resolved path and prereqs are resolved
  there.
- Vpath same-file merging (Savannah bug #62650): when `vpath` resolves
  an explicit-rule target to another explicit-rule target's path, the
  rules are merged and a "same file" warning is emitted.
- Library search order rewrite: `resolve_vpath_with_index` provides
  earliest-match semantics across `.LIBPATTERNS` candidates and vpath
  entries.
- Double-colon rules execute each rule's recipe independently.
- Pattern rule matching picks the first candidate whose prereqs exist
  or can be built; falls back to the last matching user-defined
  pattern.
- Pattern rule search: user-defined rules tried first (definition
  order, first-wins); built-in rules tried only when no user rule
  matches.
- Terminal pattern rule matching accepts explicitly-mentioned files
  and suppresses further implicit rule chaining for those prereqs.
- Order-only prereqs promoted to normal when a prereq appears in both
  positions.
- A `|` appearing in *expanded* prereq text splits normal from
  order-only prereqs after expansion.
- `.SECONDEXPANSION:`: each rule registered while the flag is active
  stores raw (first-pass-expanded) prereq text. At build time,
  `expand_with_auto` re-runs over that text with full automatic
  variables (`$@`, `$<`, `$^`, `$+`, `$|`, `$*`, plus `D`/`F`
  variants) and target/pattern-specific vars temporarily applied.
- Filesystem glob expansion in prerequisites and target names.
  Unmatched globs retain literal form.
- Pattern rules carry order-only prereqs through match.
- `.DELETE_ON_ERROR`, `.SILENT`, `.POSIX`, `.SUFFIXES`, `.DEFAULT`
  special targets. `.POSIX` installs POSIX-standard built-in variable
  defaults and sets `.SHELLFLAGS=-ec`.
- Pattern-implied prerequisites visible via `$<`.
- `.WAIT` filtered from automatic variables.
- D/F directory variants for all automatic variables.
- `$*` stem computed for explicit rules with recognized `.SUFFIXES`.
- Default goal allows path-prefixed targets.
- Two-pass implicit rule search: first pass requires prereqs to exist
  directly; second pass allows chaining (depth 6).
- `.ONESHELL` passes entire recipe body to a single shell invocation.
- `.NOTINTERMEDIATE` (per-file, pattern, global), `.SECONDARY`,
  `.INTERMEDIATE` with conflict detection.
- Intermediate file auto-deletion: pattern-derived intermediates
  deleted in reverse build order; explicit `.INTERMEDIATE` deleted
  in declaration order. Missing intermediates don't trigger
  unnecessary rebuilds — sources checked transitively.
- Single-suffix rules (`.c:` → `%: %.c`) and double-suffix rules
  (`.c.o:` → `%.o: %.c`).
- `.WAIT` declared as a target with prerequisites or a recipe emits
  GNU's warning; an empty `.WAIT:` declaration is a harmless no-op.

### Command-line options

- `-f` / `--file=` / `--makefile=` (multiple `-f` allowed; `-` = stdin).
- `-C`, `-I`, `-I-` (clear include dirs), `-W`, `-o`.
- `-j N` (rejects invalid integers), `-n`, `-s`/`--no-silent`,
  `-k`/`--no-keep-going`, `-t`, `-q`, `-B`, `-i`, `-e`,
  `-w`/`--no-print-directory`, `--trace`, `-d`.
- `-l` / `--load-average` / `--max-load` with Linux load sampling
  via `/proc/loadavg`.
- `-O` output-sync (`none`, `line`, `target`; `recurse` and `job`
  alias to `target`).
- `-r` / `-R` disable built-in rules / built-in variables.
- `-p` / `--print-data-base` dump (header, Variables with origin
  labels, Pattern-specific Variable Values, Implicit Rules, Files,
  `.SUFFIXES`, vpath, footer).
- `--eval=TEXT`, `--warn-undefined-variables` (parses).
- Cluster-flag parsing (`-erR`) and short-flag-with-arg forms (`-Wfoo`).
- MAKEFLAGS assembled with canonical ` -- ` separator before
  command-line variable assignments.
- GNUMAKEFLAGS prepended once, then cleared for sub-makes.
- `--shuffle[=MODE]` reorders prerequisites and goals. Modes:
  `reverse`, `none`/`identity`, numeric seed (deterministic xorshift),
  and `random`.
- Unknown long options produce GNU-compatible error and exit 2 with
  the `built for` banner.

### Shell / recipe execution

- `SHELL` may itself contain arguments (e.g. `SHELL := echo hi`).
- `.SHELLFLAGS` is tokenized with shell-style quote handling.
- Direct execution optimization: simple commands (no shell
  metacharacters or builtins) are exec'd directly; ENOEXEC falls
  back to `/bin/sh`, ENOENT falls back to `$SHELL` for custom shells
  or produces GNU make-style error for default shell.
- `EACCES` (Permission denied) in direct-exec path prints
  `make: {cmd}: Permission denied` and synthesizes exit 127.

### Environment / sub-makes

- `MAKE` set to argv[0]; MAKELEVEL incremented per recursion.
- MAKEFLAGS re-exported through the env; `MAKE` always propagated.
- `--print-directory` auto-on for sub-makes, last-wins overrides.
- `touch` and `question` modes short-circuit recipe execution with
  the right diagnostics and exit codes.
- `-k` mode emits each prereq error and continues, marks the goal as
  "not remade because of errors".
- `MAKE_RESTARTS` cleared from child environment to prevent spurious
  suppression of "Entering directory" messages in sub-makes.
- "Entering directory" printed before makefile loading so `$(info)`
  directives in sub-makes appear after the directory banner.

### Parallel scheduling

- Fork-based scheduler for sibling prereqs that are "simple leaves":
  non-dot name, no target-specific vars/exports, exactly one rule
  entry with a non-empty recipe, no further prereqs/order-only deps,
  no `&:` group, no `.SECONDEXPANSION`, no `$(MAKE)` in recipe lines.
- Phony-leaf forking; completion-order reaping (replaces spawn-order
  `waitpid`).
- "Waiting for unfinished jobs...." emitted on first failure when
  `keep_going` is false and pending children remain.
- Per-target `.NOTPARALLEL` (`notparallel_targets` set) alongside
  bare-`.NOTPARALLEL:` global flag.
- Aggregator-expansion: when a sibling prereq is a "no-recipe
  aggregator" (every rule entry has empty recipe and non-empty
  prereqs), and its prereqs are all simple leaves, inline the leaves
  into the parallel batch (deduped across siblings) and mark the
  aggregator as built post-batch.
- Real jobserver protocol: inherits an existing pipe via
  `--jobserver-auth=R,W` (or legacy `--jobserver-fds=`) from the
  parent make's MAKEFLAGS, or creates a fresh one and pre-fills
  `jobs - 1` tokens. `try_parallel_batch` and
  `parallel_remake_files` gate spawns on `jobserver_try_acquire()`.
  Tokens released after waitpid reaps each child. `+`-prefixed
  recipes bypass token gating.
- Output-sync (`-O`): each forked child's combined stdout/stderr
  captured via a pipe and drained by a `std::thread`; buffered
  output flushed atomically under a process-wide `Mutex`. In `target`
  mode the drainer accumulates the whole child's output and flushes
  after `waitpid`; in `line` mode each newline-terminated line
  flushes as it arrives.
- Parallel during include-remake: `parallel_remake_files` partitions
  loaded includes / to-build files into spawnables and processes them
  in parallel before falling back to serial for non-spawnables.

## Limitations

The remaining gaps would not affect the test suite but are worth
noting for real-world makefile compatibility.

### Real-world gaps not exercised by the test suite

- **Variable-definition-site line tracking.** Reports the line where
  a function is *expanded*, not where it's *declared*. Partial:
  `var_source_locs` is populated for `Assignment`, `ExportAssign`,
  `UnexportAssign`, `Override`, `PrivateAssign`,
  `PrivateExportAssign`, `Define`, and `OverrideDefine`, and
  `lookup_var_with_auto`'s recursive-expansion path propagates the
  location via `expand_chain_source`. Still missing: per-function-call
  site tracking (would require `(file, line)` metadata on every
  `$(...)` AST node and threading through expansion).
- **Chained pattern rules.** Two-pass implicit-rule search bounded at
  depth 6 covers the upstream tests; more aggressive chaining could
  match GNU's behavior more precisely in edge cases.
- **Fork-based parallel non-leaf scheduling.** A sibling that's a
  non-leaf, non-aggregator target falls back to serial. A real async
  scheduler (per-job `RecipeJob` context, Vec of in-flight children,
  completion-driven `build_target_for` continuations) would cover
  arbitrary parallel DAGs.

### Out of scope

- Shared-object loading (`load` / `loadapi`).
- Guile (`features/guile`).
- Archives (`features/archives`).
- VMS.

## Future work

These items are tracked as future work. None are required for any
passing test in the upstream suite, so they're prioritized below
anything that affects the test count.

1. **Variable-definition-site line tracking.** *Partial.* Completing
   it requires the parser (`parse.rs`) to attach `(file, line)`
   metadata to every `$(...)` function-call AST node, and `expand.rs`
   to thread that location through the expansion call-chain when
   emitting diagnostics. GNU make does this via `floc` structures on
   each `chartok`. Substantial parser/expander refactor; defer until
   a user reports that a missing line number caused real debugging
   friction.
2. **Deeper chained pattern rules.** The two-pass implicit-rule
   search bounded at depth 6 covers the upstream tests, but more
   aggressive chaining could match GNU's behavior more precisely
   in edge cases. Needs careful regression testing.
3. **Fork-based parallel non-leaf scheduling beyond aggregator
   expansion.** A real async scheduler — per-job `RecipeJob` context,
   Vec of in-flight children, completion-driven `build_target_for`
   continuations — would cover arbitrary parallel DAGs. The deferred
   Round 17/18/20 work in CHANGELOG.md covers what's needed for the
   current 135/135.

## Workflow

1. Pick a failing test; look at `work/<cat>/<name>.diff.*` to see the
   expected-vs-actual difference.
2. Check the `.mk.*` files and `.run.*` invocation to reproduce locally.
3. Fix the code, rebuild, re-run the baseline.
4. Commit with `feat(rust/make): <summary> — N/135 (X%)`.
5. Push the `make-test` bookmark.

The Deslop pre-commit hook complains about pre-existing struct
patterns; commit with `SKIP=deslop`.
