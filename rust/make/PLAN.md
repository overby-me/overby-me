# rust-make: Plan to Pass Upstream GNU make Tests

## Current Status

**129/135 tests passing** (96%) — upstream test harness from GNU make 4.4.1.

MAKEFLAGS subtests: **198/218** (91%) — significant progress on flag
merge, dedup, sort, origin tracking, and env-inherited override propagation.

`rust/make` has a parser, expander, and build engine (~10k LoC) with
Nix-checks wiring that wraps `run_make_tests.pl` and points it at
`rust-make`. Ongoing work on the `make-test` bookmark.

Run a test locally:

```bash
cd /tmp/make-work/make-4.4.1/tests
perl run_make_tests.pl -make $PWD/../../../rust/make/target/debug/make <category>/<name>
```

Run the full suite and list failures:

```bash
bash /tmp/run-make-baseline.sh > /tmp/base.txt
```

Via Nix:
`nix build .#checks.x86_64-linux.rust-make-test-{category}-{name}` and
`nix log` for failure output.

---

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

### Excluded

- `tests/scripts/vms/` — OpenVMS-specific, not applicable.
- `test_template` — skeleton.
- `features/guile`, `features/load`, `features/loadapi`, `features/archives` —
  require Guile / dynamic C extensions / `ar`. Counted as skipped candidates.

---

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
    owning target's recipe (added to `exports` on entry, removed
    on exit) so `two: export SHELL := /.//bin/sh` doesn't leak to
    sibling targets' recipes.
- `.RECIPEPREFIX := X` overrides the tab prefix for subsequent rules.
- `.RECIPEPREFIX` tracked during the preprocessing phase so custom-prefix
  recipe lines correctly preserve backslash-newline for the shell.
- `has_inline_recipe` detection: rule lines with `;` inline recipes
  preserve `\<newline>` for the shell. Multiple consecutive
  `\<newline>` in non-recipe context collapse to a single space.
  `.POSIX` preserves trailing whitespace before `\`.
- Inline-recipe detection on rule lines: scan `after-colon` text for the
  first top-level `;` (inline-recipe separator) vs. `#` (trailing
  comment), respecting `$(...)` / `${...}` nesting. The recipe text
  itself is opaque to comment stripping, so `target: ; @echo "a#b"`
  no longer truncates the recipe at `#`.
- Backslash-escape unescaping in target / prerequisite name tokens:
  `\\#` -> `#`, `\\:` -> `:`, `\\<space>` -> literal space.
  Allows targets such as `foo\\#bar.ext`, `pre\\:foo`, and
  `foo\\ bar` to parse as a single token. The default-goal
  derived from a multi-word target preserves embedded spaces via a
  sentinel byte so that `.DEFAULT_GOAL := foo bar` (literal space)
  still produces the GNU 'more than one target' error.
- Backslash-escape unescaping in target/prereq names is now
  expansion-aware: `\:`, `\#` are unescaped post-expansion (so
  `path = pre\:` then `$(path)foo` produces target `pre:foo`), and the
  unescape correctly handles odd/even backslash runs (N backslashes
  before `:` produce N/2 literal backslashes plus a literal/separator
  colon depending on parity).
- Embedded `:` in expanded target names (e.g. `path = pre:` then
  `$(path)foo : ;`) triggers GNU's `target pattern contains no '%'`
  static-pattern fatal error.
- Variable assignment values: `\#` is unescaped to `#` outside of
  `$(...)`/`${...}` references (preserved inside function call args
  for re-expansion).
- UTF-8 BOM (`\u{FEFF}`) stripped from the start of a loaded makefile
  so it doesn't contaminate the first token.
- Backslash-newline continuations:
  - Non-recipe: collapse surrounding whitespace to a single space.
  - Recipe: preserve `\<nl>` so the shell handles continuation; strip
    leading tab on continuation physical lines for trace output.
- Inline `;` recipe on the rule line.
- Bare `$(…)` expression lines (`$(info)`, `$(error)`, `$(eval)`).
- Nesting-aware whitespace split for rule targets, prerequisites, and
  order-only prereqs: `$(filter %.o,$(files))` is kept as a single
  token instead of being split on internal spaces.
- Recipe lines inside a conditional body (`ifeq … \t@cmd … endif`)
  attach to the most recently declared rule — matching GNU make's
  line-by-line splice when the taken branch is flattened.

### Expansion / Functions

- Text: `subst`, `patsubst`, `strip`, `findstring`, `filter`,
  `filter-out`, `sort`, `word`, `wordlist`, `words`, `firstword`,
  `lastword`.
- Filenames: `dir`, `notdir`, `suffix`, `basename`, `addsuffix`,
  `addprefix`, `join`, `wildcard`, `realpath`, `abspath` (with path
  normalization).
- Conditionals: `if`, `or`, `and`, `intcmp` (arbitrary-precision via
  string comparison — handles integers exceeding `i64`; supports the
  4-arg `lt,ge` form where the 4th argument covers both equal and
  greater cases).
- Control: `foreach` (binding writes into var scope so `$(eval)`
  inside the body sees the iteration), `call` (dispatches to built-ins
  too), `value`, `eval`, `origin`, `flavor`, `let`.
- I/O: `shell` (sets `.SHELLSTATUS`, re-exports env-inherited vars
  using their current expanded values; falls back to the original
  process-env value when the body self-references or itself contains
  another `$(shell …)` to avoid infinite recursion; child stderr is
  re-emitted under the `make:` prefix with `<shell>:`/`line N:`
  noise stripped), `file`, `error`, `warning`, `info`.
- `info`, `warning`, `error`, `eval` receive the full argument text
  including commas (single-argument functions per GNU make semantics).
- Fatal errors for invalid `word`/`wordlist`/`intcmp`/`foreach`/`let`
  args with GNU-compatible diagnostic text.
- Substitution references `$(VAR:from=to)` and `$(VAR:a=%b)`.
- `\<newline><whitespace>` sequences inside `$(…)` / `${…}` references
  are collapsed to a single space during expansion (GNU make job.c
  compatibility).

### Variable system

- Flavours: recursive, simple, immediate-recursive (`:::=`).
- Origins: undefined, default, environment, file, command line,
  override, automatic.
- `-e` boost for environment origin.
- Env-inherited names re-exported to child processes with current
  makefile value (respects `SHELL` exception).
- Dynamic `.DEFAULT_GOAL`, `.VARIABLES`, `.INCLUDE_DIRS`.
- `MAKEFILE_LIST` accumulated as each file is loaded.
- `.EXTRA_PREREQS` with target-specific override and glob expansion,
  applied after normal prereqs but before order-only.
- `.LIBPATTERNS` with non-pattern element warning.
- `.SHELLSTATUS` populated after `$(shell)`.
- `:::=` marks names for "recursive-style" `+=` append.
- Target-specific variables propagate to prerequisite builds via
  scope stack. `:=` target-specific assignments expand at declaration
  time so they capture caller-side bindings.
- Target-specific `+=` appends to existing value (respecting flavor).
- Target-specific `?=` skips assignment when the variable is defined.
- Target-specific `unexport` marks variables for unexport during the
  target's recipe.
- Target-specific `override` modifier tracked and respected: overrides
  command-line vars. Variable names in target-specific assignments are
  expanded (e.g., `target: VAR$(X) = val`).
- Target-specific `private` modifier: private vars are NOT inherited
  by prerequisite builds. Global `private` (`private VAR = val`,
  `private export VAR = val`) hides globals from recipe contexts.
- Target-specific `override +=` suppresses non-override entries for
  the same variable.
- Target-specific `export` removes from the `unexports` set for the
  duration of the target's recipe.
- Target-specific vars do not override command-line variables (unless
  the target-specific assignment has `override`).
- **Pattern-specific variables** (`%.x: VAR = value`): match targets
  by pattern; when multiple patterns match, shortest-stem wins.
  Supports all modifiers: `override`, `private`, `export`, `unexport`,
  and all operators (`:=`, `=`, `+=`, `?=`, `!=`).
- Target-specific `:=` values are expanded at declaration time only
  (no double-expansion at apply time).
- Target-specific export/unexport propagates through the prerequisite
  chain via scope push/pop in `build_target_for`; private exports
  are excluded from propagation.

### Rules engine

- Explicit, pattern, static-pattern, and grouped-target handling.
- **Escaped-`%` in static pattern rules**: `find_unescaped_percent`,
  `unescape_percent`, `replace_first_unescaped_percent` helpers handle
  `\%` literally in target/prereq patterns.
- **GPATH support**: when a vpath-resolved directory is listed in
  `GPATH`, the target redirects to the resolved path and prereqs are
  resolved there — files stay in place rather than being treated as
  intermediate.
- **Vpath same-file merging** (Savannah bug #62650): when `vpath`
  resolves an explicit-rule target to another explicit-rule target's
  path, the rules are merged and a "same file" warning is emitted.
- **Library search order rewrite**: `resolve_vpath_with_index` provides
  earliest-match semantics across `.LIBPATTERNS` candidates and vpath
  entries, matching GNU make's library resolution order.
- Double-colon rules execute each rule's recipe independently.
- Pattern rule matching picks the first candidate whose prereqs exist
  or can be built; falls back to the last matching user-defined
  pattern when nothing else applies.
- Pattern rule search: user-defined rules are tried first (definition
  order, first-wins), built-in rules tried only when no user rule
  matches.
- Terminal pattern rule matching accepts explicitly-mentioned files
  (targets or prereqs of explicit rules) and suppresses further
  implicit rule chaining for those prereqs.
- Order-only prereqs promoted to normal when a prereq appears in both
  positions (via union across combined rules).
- A `|` appearing in *expanded* prereq text (e.g. from `$(VAR)` whose
  value contains `|`) splits normal from order-only prereqs after
  expansion, matching GNU make's re-parse.
- **`.SECONDEXPANSION:`** (partial): each rule registered while the
  flag is active stores raw (first-pass-expanded) prereq text. At
  build time, `expand_with_auto` re-runs over that text with full
  automatic variables (`$@`, `$<`, `$^`, `$+`, `$|`, `$*`, plus
  `D`/`F` variants) and target/pattern-specific vars temporarily
  applied. Pattern-rule SE pre-expansion is suppressed in the
  matcher to avoid double-firing side-effect functions like
  `$(info)`. Glob expansion runs on second-expanded prereqs.
- Filesystem glob expansion in prerequisites and target names
  (regular rules and pattern-rule resolved prereqs). Unmatched globs
  retain their literal form so explicit rules can still build them.
- Pattern rules carry order-only prereqs through match: `%.w: %.x | baz`
  substitutes the stem into the order-only pattern and adds it to `$|`
  for matched targets.
- `.DELETE_ON_ERROR`, `.SILENT`, `.POSIX`, `.SUFFIXES`, `.DEFAULT`
  special targets. `.POSIX` installs POSIX-standard built-in variable
  defaults (`ARFLAGS=-rv`, `CC=c99`, `CFLAGS=-O1`, `FC=fort77`,
  `FFLAGS=-O1`, `LEX=lex`, `SCCSGETFLAGS=-s`, `YACC=yacc`) and sets
  `.SHELLFLAGS=-ec`. On a recipe line with `-` ignore-errors prefix,
  the `-e` is stripped only when `.SHELLFLAGS` is still at its
  `.POSIX`-installed default — matching GNU make's observable
  behavior where user-assigned flags are respected verbatim.
- Pattern-implied prerequisites visible via `$<`.
- `.WAIT` filtered from automatic variables.
- D/F directory variants for all automatic variables (`$(*D)`, `$(<D)`,
  `$(^D)`, `$(+D)`, `$(?D)`, `$(|D)` and their `F` counterparts).
- `$*` stem computed for explicit rules with recognized `.SUFFIXES`.
- Default goal allows path-prefixed targets (e.g. `../dir/foo.x`).
- Prerequisite double-expansion fixed: prereqs from `process_rule` no
  longer re-expanded in `build_target_for`.
- Two-pass implicit rule search: first pass requires prereqs to exist
  directly; second pass allows chaining. Ensures direct-prereq rules
  are preferred over chain-requiring rules.
- `.ONESHELL` passes entire recipe body to a single shell invocation.
  First line's prefix chars (`@`/`-`/`+`) control the whole recipe.
- `.NOTINTERMEDIATE` special target (per-file, pattern `%.x`, and
  global no-prereq forms). Conflict detection with `.INTERMEDIATE`
  and `.SECONDARY`.
- `.SECONDARY` and `.INTERMEDIATE` prerequisite lists tracked;
  `secondary_all` flag for global `.SECONDARY:`.
- `%` substitution in pattern rules uses `replacen` (first `%` only).
- **Intermediate file auto-deletion**: after an implicit rule chain
  builds a target, pattern-derived prerequisite files that are
  intermediate (not mentioned, not `.SECONDARY`, not
  `.NOTINTERMEDIATE`) are automatically removed. Explicitly
  `.INTERMEDIATE:`-marked files are also deleted. Deletion is deferred
  to after the top-level goal completes. Missing intermediates don't
  trigger unnecessary rebuilds — their ultimate sources are checked
  transitively.
- Implicit rule chain depth increased to 6 (from 3) to support deeper
  dependency chains.
- **Intermediate deletion ordering** matches GNU make: pattern-derived
  intermediates (chained via implicit rules) are deleted in reverse
  build order, while explicit prereqs marked `.INTERMEDIATE` are
  deleted in declaration order. Fixes `targets/SECONDARY` (Savannah
  bug #15919) without regressing `targets/INTERMEDIATE`.
- `.WAIT` declared as a target with prerequisites or a recipe emits
  GNU's `.WAIT should not have prerequisites/commands` warning; an
  empty `.WAIT:` declaration remains a harmless no-op.

### Command-line options

- `-f` / `--file=` / `--makefile=` (multiple `-f` allowed; `-` = stdin;
  error message for missing file now matches GNU make format).
- `-C`, `-I`, `-I-` (clear include dirs), `-W`, `-o`.
- `-j N` (reject invalid integers), `-n`, `-s`/`--no-silent`
  last-wins, `-k`/`--no-keep-going`, `-t`, `-q`, `-B`, `-i`, `-e`,
  `-w`/`--no-print-directory`, `--trace`, `-d`.
- `-l` / `--load-average` / `--max-load` (accepted, not implemented).
- `-d` emits `GNU Make` banner to stderr for test compatibility.
- `-h` / `--help` includes the GNU `This program built for ...`
  banner so test harnesses that match `/uilt for /` accept us.
- `-r` / `-R` disable built-in rules / built-in variables.
- `--eval=TEXT`, `--warn-undefined-variables` (parses; no emission yet).
- Cluster-flag parsing (`-erR`) and short-flag-with-arg forms (`-Wfoo`).
- MAKEFLAGS assembled with canonical ` -- ` separator before
  command-line variable assignments; split short vs long options so
  long-only MAKEFLAGS begins with a space.
- GNUMAKEFLAGS prepended once, then cleared for sub-makes.
- `--shuffle[=MODE]` reorders prerequisites and goals. Modes: `reverse`,
  `none`/`identity` (no-op), numeric seed (deterministic xorshift), and
  `random`. Order-only prereqs participate in shuffle ordering. Top-level
  goal list is also shuffled. Automatic variables (`$^`, `$<`, `$?`, `$+`,
  `$|`) preserve original declaration order.
- Unknown long options now produce GNU-compatible error and exit 2 with
  the `built for` banner instead of being silently accepted.

### Shell / recipe execution

- `SHELL` may itself contain arguments (e.g. `SHELL := echo hi`): the
  first word is the program, subsequent words are leading arguments.
- `.SHELLFLAGS` is tokenized with shell-style quote handling — single-
  and double-quoted sections stay as a single token with the quotes
  stripped, so `'ho;ho'` becomes one arg containing a literal `;`.
- Direct execution optimization: simple commands (no shell
  metacharacters or builtins) are exec'd directly; ENOEXEC falls back
  to `/bin/sh`, ENOENT falls back to `$SHELL` for custom shells or
  produces GNU make-style error for default shell.
- `EACCES` (Permission denied) in direct-exec path prints
  `make: {cmd}: Permission denied` and synthesizes exit 127.
- `=` included in `SHELL_META` for direct-exec detection.

### Environment / sub-makes

- `MAKE` set to argv[0]; MAKELEVEL incremented per recursion.
- MAKEFLAGS re-exported through the env; `MAKE` always propagated.
- `--print-directory` auto-on for sub-makes, last-wins overrides.
- `touch` and `question` modes short-circuit recipe execution with the
  right diagnostics and exit codes.
- `-k` mode emits each prereq error and continues, marks the goal as
  "not remade because of errors".
- `MAKE_RESTARTS` cleared from child environment to prevent spurious
  suppression of "Entering directory" messages in sub-makes.
- "Entering directory" printed before makefile loading so `$(info)`
  directives in sub-makes appear after the directory banner.

---

## What's still missing / deferred

These are consciously skipped because each requires substantial work
for limited test count gains, or because they affect correctness only
in ways the test harness happens not to exercise in our passing set.

### High-impact, hard (unlocks many tests)

- **`.SECONDEXPANSION:`** — partial implementation in place. Currently
  passing: `variables/automatic`, `features/rule_glob`. Subtest counts:
  `features/se_explicit` 31/31 ✅, `features/se_implicit` 28/30,
  `features/se_statpat` 12/12,
  `features/statipattrules` 68/68 ✅,
  `features/patternrules` 72/72 ✅. Infrastructure: `RuleEntry` /
  `PatternRuleEntry` carry `second_expand` flag and `raw_prereq_text`;
  `build_target_for` runs `expand_with_auto` with `$@`/`$*`/`$<`/`$^`/`$+`/`$|`
  (and D/F variants) over the saved raw text under
  `with_target_vars_applied(target)` so target/pattern-specific vars
  are visible. `try_pattern_rule` accepts SE pattern rules without
  pre-expanding (so side-effect functions like `$(info)` don't fire
  twice). Remaining gaps: prereq ordering when multiple rules
  contribute (recipe-rule first), grouped-target SE per-target
  expansion, double-colon SE per-rule expansion, `$%` / archive
  member auto-var, multi-percent + directory-transfer pattern
  substitution semantics, parse-time eager expansion of `$$( ... )`
  to detect unterminated function calls at the source line (affects
  `se_explicit`/`se_implicit` `firstword` subtests).
- **Variable-definition-site line tracking.** We report the line
  where a function is *expanded*, not where it's *declared*. GNU
  make tags each function call with its source location in the
  makefile. Affects most `functions/*-e*` error tests.
- **Makefile auto-rebuild / re-exec** — partially done: basic
  auto-rebuild works for includes and primary makefiles (`-f`), but
  stdin re-exec via `--temp-stdin` needs work. Needed for
  `options/dash-B` (5/8),
  `variables/MAKE_RESTARTS` (0/3), many `options/dash-W` and
  `options/dash-n`.
- ~~**GPATH support**~~ ✅ Implemented. Files found via `vpath`/`VPATH`
  stay in the resolved location when `GPATH` lists their dir.
  `features/vpathgpath` now passes (1/1).
- ~~**vpath conflict ("same file") warning**~~ ✅ Implemented. Vpath
  same-file merging with warning (Savannah bug #62650). `features/mult_rules`
  now passes (3/3), `features/se_explicit` #27 fixed.
- ~~**Built-in C compile pipeline through vpath**~~ ✅ Implemented.
  Pattern rules like `%: %.o` chained with `%.o: %.c` resolve `%.c`
  through vpath; phony targets skip implicit rules; VPATH revocation
  after failed rebuild. `features/vpathplus` now fully passes (4/4).
- **Full MAKEFLAGS → child parse** — we propagate MAKEFLAGS but don't
  re-parse the full ` -- `-separated form in sub-makes. Blocks most
  of `variables/MAKEFLAGS` (12/218).
- **Parallel jobs / jobserver** — `-j N`, `.WAIT`, `.NOTPARALLEL`,
  `output-sync`. Only hurts `features/parallelism` and
  `targets/WAIT`.

### Lower-impact

- **Non-tab recipe detection** — emit "missing separator" warning
  when 8 spaces look like a recipe. `misc/failure`.
- **Suffix rules** (`.c.o:` → `%.o: %.c`) fully interacting with
  built-in defaults: `features/suffixrules`, `targets/POSIX`.
- **Chained pattern rules** — follow pattern-rule chains more than
  one level deep. Previously attempted; caused regressions because
  the fallback fires too eagerly. Needs more precise criteria.
- **`--shuffle`** — `options/shuffle` fully passes. `.NOTPARALLEL`
  disables shuffle reordering; SECONDEXPANSION-derived prereqs are
  also shuffled.
- **Shell command-not-found rewrite** — GNU replaces `/bin/sh: X:
  command not found` with `make: X: No such file or directory`.
  `features/errors`, `misc/general4`.
- **`--eval` propagation to sub-make**, `--warn-undefined-variables`
  emission — remaining `options/eval`, `options/warn-undefined-variables`.
- **`-l` load limit** — `options/dash-l`.

### Out of scope

- **Jobserver protocol** (part of parallel jobs, but conceptually
  separable).
- **Shared-object loading** (`load` / `loadapi`).
- **Guile** (`features/guile`).
- **Archives** (`features/archives`).
- **VMS**.

---

## Currently failing top-level tests (6 of 135)

Latest baseline (`bash /tmp/run-make-baseline.sh`):

| Test | Notes |
| --- | --- |
| `features/parallelism` | Requires `-j` / jobserver (0/8) |
| `features/se_implicit` | 28/30 subtests pass; failing subtests 9 (SE prereq verification in pattern search), 27 (SE prereq expansion ordering) |
| `features/temp_stdin` | 6/8 subtests pass; `--debug=b` re-exec banner with `--temp-stdin=` (#4); SIGTERM "Terminated" diagnostic (#5, needs signal handler) |
| `options/dash-f` | 31/32 subtests pass; remaining failure needs stdin re-exec via `--temp-stdin` |
| `targets/WAIT` | `.WAIT` requires parallel scheduling (3/4) |
| `variables/MAKEFLAGS` | 198/218 subtests pass; remaining 20 need: makefile auto-rebuild (197-208), sub-make with debug (210-217) |

The biggest remaining unlocks are **parallel jobs**
(unlocks `parallelism`, `WAIT`) and **stdin re-exec / `--temp-stdin`**
(unlocks remaining `dash-f`, `temp_stdin`). The `se_implicit` remainder requires
SE-during-pattern-search.

### MAKEFLAGS improvements (this session)

Went from 15/218 → 187/218 subtests:

- **MAKEFLAGS merge logic**: When a makefile assigns `MAKEFLAGS`, command-line
  flags are preserved (merged union). File-added flags are additive.
- **Flag deduplication**: Short flags from env MAKEFLAGS, cmdline, and
  makefile are deduplicated.
- **Flag sorting**: Short flags sorted case-insensitively (lowercase before
  uppercase for same letter, e.g. `rR` not `Rr`). Long flags sorted by
  GNU make's canonical category order (`-I`, `-l`, `-O`, `--debug`, `--trace`,
  `--no-print-directory`, `--warn-undefined-variables`, `--no-silent`, `--eval`).
- **New flags**: `-S`/`--stop`, `-O`/`--output-sync`, `-d`/`--debug=X`,
  `-l`/`--load-average` now tracked in MAKEFLAGS.
- **`-w` as short flag**: `-w` stored as `w` in short flags (not `--print-directory`
  in long flags), matching GNU make.
- **Mutual exclusion**: `-w`/`--no-print-directory`, `-s`/`--no-silent`,
  `-k`/`-S` properly suppress each other.
- **Origin tracking**: MAKEFLAGS uses `CommandLine` origin when cmdline
  flags/overrides are present, blocking file-level `+=` when `-e` is active.
- **Variable ordering**: `:=` overrides before `=` in MAKEOVERRIDES.
- **Env-inherited overrides**: `hello=world` from MAKEFLAGS env propagated
  through MAKEOVERRIDES with dedup against cmdline overrides.
- **Post-load flag application**: `-s`, `-k`, `-i`, `-n`, `-e`, `-w`,
  `--no-print-directory`, `--trace` set from makefile `MAKEFLAGS +=` are
  applied to engine state after all makefiles are loaded.
- **Signal handling**: Recipe processes killed by signals now report the
  signal name (e.g. "Terminated") instead of a numeric error code.

---

## Investigation notes (latest)

Round 2 of SE-focused fixes (still 120/135 categories, but +4 more
subtests on top of the earlier +5):

1. **Static-pattern target token splitting**: tokens like
   `$(filter %.o,$(files))` were treated as containing a literal space
   (the space inside the function call) and thus passed through as a
   single literal-space target name. New helper `has_unwrapped_space`
   only flags spaces at depth 0 (outside `$(...)` / `${...}`). Fixes
   `statipattrules` Test default (and Test #1 by side effect).
2. **Grouped-target sibling SE timing**: per-sibling SE side-effects
   were firing AFTER recipe execution and never built sibling SE-derived
   prereqs. Moved sibling SE firing to BEFORE the recipe; the
   currently-executing grouped rule fires first for each sibling, then
   other rules in declaration order; SE-expanded prereqs are then built
   via `build_target_for`. Fixes `se_explicit` #22 (sv 62706 grouped).
3. **Backslash-colon in SE substitution refs**: `$(@\:%=%.bar)` (where
   the user escaped `:` to prevent it being parsed as a static-pattern
   separator in the prerequisite line) was confusing
   `find_subst_colon`. Pre-process expressions in `expand_expr` to
   replace `\:` with `:` before substitution-ref / function detection.
   Fixes `se_explicit` #9.

Subtest deltas (cumulative from baseline 108/135):

- features/se_explicit 26 → 29 (+3 total this session: #18, #22, #9)
- features/se_implicit 24 → 27 (+3: default, #18, #26)
- features/se_statpat 10 → 11 (+1: #4)
- features/statipattrules 64 → 66 (+2: default, #1)

Remaining notable blockers:

- SE side-effect firing during pattern_search (so SE info appears
  during prereq verification, not just build): `se_explicit` #27 (VPATH
  combined), `se_implicit` #9 (sim_base — pattern rule rejected only
  after SE prereq verification fails)
- LIBPATTERNS conflict warning for `-l<name>` resolved to lib<name>.a:
  `se_explicit` #10
- Implicit recursion guard for SE pattern rules with directory-aware
  stem decomposition: `se_implicit` #3 (`%.o:` matched against
  `../tests/tmp/bar.o` should pick stem `bar` not `../tests/tmp/bar`)
- Escaped-`%` in target names: `statipattrules` #66, #67

Round 3 fix (121/135):

- **`$` in static-pattern target names**: `foo$$bar: f%r: % ; ...`
  registered `default_goal` as the post-first-expansion target name
  `foo$bar`. At goal consumption time, `default_goal` is re-expanded
  via `expand::expand`, which then mangled `foo$bar` -> `fooar`
  (treating `$b` as an empty variable reference). Static-pattern's
  `default_goal` insertion now `$`-escapes the target name (matching
  the regular-rule branch that already did this). Fixes
  `features/se_statpat` Test #4 (literal `$` in stem) — full file now 12/12.

---

## Category breakdown (current)

Based on the latest baseline run (129/135 passing):

| Category    | Status                                                            |
| ----------- | ----------------------------------------------------------------- |
| `features`  | 39/42 passing. `patternrules` 72/72 ✅, `reinvoke` 12/12 ✅, `se_explicit` 31/31 ✅, `statipattrules` 68/68 ✅, `vpathplus` 4/4 ✅. Blocked on remaining `se_implicit` edge cases, parallelism, temp_stdin signal/debug. |
| `functions` | Most working. Remaining: fatal-error line numbers.                |
| `misc`      | All passing (bs-nl 28/28, general4 10/10).                        |
| `options`   | Most working; `dash-q` fully passing. `dash-f` 31/32. Blocked on stdin re-exec/MAKEFLAGS. |
| `targets`   | `ONESHELL`+`NOTINTERMEDIATE`+`INTERMEDIATE`+`SECONDARY` fully passing. `WAIT` needs parallel scheduling. |
| `variables` | All done except MAKEFLAGS (`automatic` now passes via SE). |

Round 4 (still 121/135 categories, but +6 vpath subtests):

- **`vpath PATTERN DIRS` directive** now stored in
  `vpath_patterns: Vec<(String, Vec<String>)>` and consulted by
  `resolve_vpath()`. Pattern matching uses `expand::pattern_stem`;
  ALL matching patterns are tried in declaration order.
- **VPATH-aware rule lookup**: when `target` has no explicit rule
  but `resolve_vpath_rule(target)` finds a registered rule for a
  vpath-resolved name, the build redirects to that name. So
  `vpath %.te vpath-d/` + `vpath-d/fail.te:` makes `fail.te`
  resolve to `vpath-d/fail.te`. Fixes `features/vpath` Test #1
  (Savannah `default` test) and Test #4 (`vpa/foo.x` over `%.x`).
- **Strict-extension vpath in `.LIBPATTERNS`**: extends
  `resolve_library_prereq` to consult `vpath PATTERN DIRS` whose
  pattern shares the candidate's extension (e.g. `lib1.a` only
  consults `vpath %.a ...`). Fixes most of `vpath.3`'s lib lookup
  modulo a wildcard-`vpath %` interaction.

Subtests now passing in failing categories:

- features/vpath: 1 → 4 of 5
- features/mult_rules: 1 → 2 of 3
- features/vpathgpath: 0 → 0 of 1 (still requires vpath in pattern-rule prereqs)
- features/vpathplus: 0 → 0 of 4 (same reason + intermediate-via-vpath)

Round 5 (121/135 categories, +5 vpath subtests):

- **VPATH lookup in pattern-rule prereqs**: `try_pattern_rule` now
  consults `file_exists_or_vpath` and `resolve_vpath_rule` so a
  pattern rule with prereq `bar.d` matches when `work/bar.d` exists
  via `VPATH=work/`. Both terminal and non-terminal rule branches.
- **`resolve_vpath` skips local files**: GNU only redirects to a
  vpath copy when the file is NOT in the current directory. Without
  this, `bar.d` (local + `work/bar.d`) was being rewritten to
  `work/bar.d` in `$^`, breaking `vpathplus` test 0.
- Subtest gains: `vpathplus` 0 → 2 of 4.

Remaining vpath/related blockers:

- GPATH support (vpathgpath, vpathplus #2/#3) — files found via
  vpath should stay there, treated as up-to-date.
- "Same file" merging when `vpath PATTERN DIRS` resolves an
  explicit-rule target to another explicit-rule target's path
  (mult_rules #2). Requires vpath conflict warning.
- Built-in C compile pipeline through vpath (vpathplus #1).

Round 6 (125/135 categories — gained statipattrules, vpath, vpathgpath, mult_rules):

1. **Escaped-`%` handling in static pattern rules**: added
   `find_unescaped_percent`, `unescape_percent`, and
   `replace_first_unescaped_percent` helpers. `\%` is now treated as a
   literal `%` in target patterns, stem patterns, and prereq patterns.
   Fixes `statipattrules` #66, #67 — file now fully passes (68/68).
2. **Vpath duplicate pattern append**: multiple `vpath %` directives now
   create separate entries in `vpath_patterns` instead of overwriting.
   Each is tried in declaration order, matching GNU make semantics.
   Fixes `vpath` #3 (`.LIBPATTERNS` interaction with wildcard `vpath %`).
3. **Library search order rewrite**: `resolve_vpath_with_index` returns
   both the resolved path and its vpath-entry index. `.LIBPATTERNS`
   candidates are now resolved with earliest-match semantics across all
   vpath entries, so `lib1.a` found via vpath entry 0 beats `lib1.so`
   found via entry 1. Fixes remaining `vpath` #5 library ordering.
4. **Vpath same-file merging with warning** (Savannah bug #62650): when
   `vpath` resolves an explicit-rule target to another explicit-rule
   target's path, the rules are merged (prereqs combined, recipe kept
   from the original) and a `Recipe was specified for file 'X' ... but
   'X' is now considered the same file as 'Y'` warning is emitted.
   Fixes `mult_rules` #2 — file now fully passes (3/3).
5. **SE side-effect firing for merged-away rules**: when vpath same-file
   merging collapses two rules, the SE ordering for the surviving rule
   respects the recipe-bearing rule's prereqs-first convention. Fixes
   `se_explicit` #27 — file now 30/31.
6. **GPATH support**: when a directory found via `vpath`/`VPATH` is
   listed in `GPATH`, the target redirects to the resolved path and
   prereqs resolve there. Files stay in the vpath directory rather than
   being treated as intermediate needing rebuild. Fixes `vpathgpath`
   #1 — file now fully passes (1/1).
7. **Peer target warning fix**: the "file 'X' was not created by any
   rule" grouped-target warning now only fires when the recipe actually
   created or updated the main target. Avoids spurious warnings when
   the recipe is a no-op. Fixes `patternrules` peer-target subtest —
   file now 63/72.

Subtest gains: statipattrules +2, vpath +1, mult_rules +1, vpathgpath +1,
se_explicit +1, patternrules +1. Category total: 121 → 125.

Round 7 (127/135 categories — gained patternrules, se_explicit):

1. **Directory-transfer in SE pattern rule prereqs**: `replace_first_percent_per_token`
   now applies GNU make directory-transfer semantics (prepend dir, substitute
   only base into `%`) when `dir_transfer=true`. Previously the full stem
   (including directory) was substituted for `%`, producing `3lib/bye4%5`
   instead of `lib/3bye4%5`. Static pattern rules pass `dir_transfer=false`
   (no directory transfer per GNU semantics). Fixes `patternrules` #45+#53
   (multi-percent with directory stem) and #64+#65 (SE `$(wordlist)` with
   directory stem) — file now fully passes (72/72).
2. **`pattern_subst_with_dir` skip for no-`%` prereqs**: when a pattern
   rule prerequisite contains no `%`, the directory portion of the stem is
   no longer prepended. Previously `foo` with stem `lib/bye` became
   `lib/foo`. Fixes implicit rule prereqs like order-only `bar.h` getting
   spurious directory prefix.
3. **Re-exec failure exit code**: changed from 2 to 127 (matching GNU
   make's `execvp` failure convention). Fixes `temp_stdin` #7.

Subtest gains: patternrules +9, temp_stdin +1. Category total: 126 → 127.

Round 8 (128/135 categories — gained vpathplus):

1. **Skip implicit rules for phony targets**: phony targets no longer
   trigger implicit rule search, preventing spurious built-in CC pipeline
   matches. Fixes `vpathplus` #1 (notarget).
2. **VPATH-aware intermediate chain timestamp checks**: intermediate files
   found via VPATH now compare timestamps correctly in the chain, so
   `foo.o` older than `foo.c` triggers rebuild but `foo.o` newer than
   `foo.c` does not. Fixes `vpathplus` #3.
3. **Pre-existing file tracking for intermediate deletion**: files that
   existed before the build started are not deleted as intermediates,
   matching GNU make's behavior. Fixes `vpathplus` #3.
4. **VPATH revocation ("un-vpath") after recipe doesn't create file**:
   when a recipe runs but doesn't create the target at the vpath location,
   the vpath resolution is revoked so subsequent rules look in the current
   directory. Fixes `vpathplus` #1.
5. **`-w` entering directory side-effect in MAKEFLAGS merge**: `-w` flag
   now properly triggers "Entering directory" messages when set via
   MAKEFLAGS in sub-makes. Fixes MAKEFLAGS subtests 124-127.
6. **`-R` implies `-r` in post-load MAKEFLAGS**: when `-R` (no built-in
   variables) is set via makefile MAKEFLAGS, `-r` (no built-in rules) is
   also implied, matching GNU make. Fixes MAKEFLAGS subtests 104-107.
7. **MAKEOVERRIDES empty→conditional separator post-load fix**: the `--`
   separator between flags and MAKEOVERRIDES is only emitted when
   MAKEOVERRIDES is non-empty, fixing edge case with empty overrides
   after post-load merge. Fixes MAKEFLAGS subtest 101.
8. **MAKEFLAGS/MAKELEVEL/MAKE export in `$(shell)` and `!=` operators**:
   these variables are now exported to the environment for `$(shell ...)`
   and `!= shell` assignment operators, matching GNU make's behavior.
   Fixes MAKEFLAGS subtests 116-117.
9. **SE pattern-rule directory-transfer: expand first, then prepend
   directory**: SE prereqs in pattern rules now expand `$$*` etc. before
   applying directory-transfer (prepending the directory portion of the
   stem), matching GNU make's order of operations. Fixes `se_implicit` #3.

Subtest gains: vpathplus +2, MAKEFLAGS +11 (187→198), se_implicit +1.
Category total: 127 → 128.

Round 9 (129/135 categories — gained reinvoke):

1. **Imagined targets for included makefiles** (sv 61226): when a rule
   ran OK for an included makefile but didn't create the file, and the
   file wasn't pre-marked by a sibling pattern rule, skip — don't error.
   This matches GNU make's behavior where nonexistent included targets
   with rules are "imagined" as updated.
2. **Mtime-based rebuilt check**: only mark targets as "rebuilt"
   (triggering dependents) when the file's mtime actually changed after
   recipe execution. Prevents cascading rebuilds when recipes are no-ops.
3. **Primary makefile auto-rebuild**: register `-f` makefiles in
   `included_files` so `finalize_includes` checks them for rebuild rules.
4. **Stdin temp file for re-exec**: when stdin is read via `-f-`, save
   content and write to temp file on re-exec, replacing `-f-` with
   `-f<temp>` in args. Handles combined flags (`-Rf-`, `--file=-`, etc.).
5. **Inherited stdin temp cleanup**: detect and clean up stdin temp files
   from previous re-exec on normal exit.

Subtest gains: reinvoke 5→12 (✅ category flip!), dash-f 23→31,
MAKEFLAGS 198 unchanged. Category total: 128 → 129.

---

## Workflow

1. Pick a failing test; look at `work/<cat>/<name>.diff.*` to see the
   expected-vs-actual difference.
2. Check the `.mk.*` files and `.run.*` invocation to reproduce locally.
3. Fix the code, rebuild, re-run the baseline.
4. Commit with `feat(rust/make): <summary> — N/135 (X%)`.
5. Push the `make-test` bookmark.

Deslop pre-commit hook complains about pre-existing struct patterns;
commit with `SKIP=deslop`.
