# rust-make: Plan to Pass Upstream GNU make Tests

## Current Status

**74/135 tests passing** (55%) — upstream test harness from GNU make 4.4.1.

`rust/make` has a parser, expander, and build engine (~5k LoC) with
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
- UTF-8 BOM (`\u{FEFF}`) stripped from the start of a loaded makefile
  so it doesn't contaminate the first token.
- Backslash-newline continuations:
  - Non-recipe: collapse surrounding whitespace to a single space.
  - Recipe: preserve `\<nl>` so the shell handles continuation; strip
    leading tab on continuation physical lines for trace output.
- Inline `;` recipe on the rule line.
- Bare `$(…)` expression lines (`$(info)`, `$(error)`, `$(eval)`).
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
- Fatal errors for invalid `word`/`wordlist`/`intcmp`/`foreach`/`let`
  args with GNU-compatible diagnostic text.
- Substitution references `$(VAR:from=to)` and `$(VAR:a=%b)`.

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

### Rules engine

- Explicit, pattern, static-pattern, and grouped-target handling.
- Double-colon rules execute each rule's recipe independently.
- Pattern rule matching picks the first candidate whose prereqs exist
  or can be built; falls back to the last matching user-defined
  pattern when nothing else applies.
- Order-only prereqs promoted to normal when a prereq appears in both
  positions (via union across combined rules).
- A `|` appearing in *expanded* prereq text (e.g. from `$(VAR)` whose
  value contains `|`) splits normal from order-only prereqs after
  expansion, matching GNU make's re-parse. Fully exercised only once
  `.SECONDEXPANSION` lands.
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

### Command-line options

- `-f` / `--file=` / `--makefile=` (multiple `-f` allowed; `-` = stdin).
- `-C`, `-I`, `-I-` (clear include dirs), `-W`, `-o`.
- `-j N` (reject invalid integers), `-n`, `-s`/`--no-silent`
  last-wins, `-k`/`--no-keep-going`, `-t`, `-q`, `-B`, `-i`, `-e`,
  `-w`/`--no-print-directory`, `--trace`, `-d`.
- `-r` / `-R` disable built-in rules / built-in variables.
- `--eval=TEXT`, `--warn-undefined-variables` (parses; no emission yet).
- Cluster-flag parsing (`-erR`) and short-flag-with-arg forms (`-Wfoo`).
- MAKEFLAGS assembled with canonical ` -- ` separator before
  command-line variable assignments; split short vs long options so
  long-only MAKEFLAGS begins with a space.
- GNUMAKEFLAGS prepended once, then cleared for sub-makes.

### Shell / recipe execution

- `SHELL` may itself contain arguments (e.g. `SHELL := echo hi`): the
  first word is the program, subsequent words are leading arguments.
- `.SHELLFLAGS` is tokenized with shell-style quote handling — single-
  and double-quoted sections stay as a single token with the quotes
  stripped, so `'ho;ho'` becomes one arg containing a literal `;`.

### Environment / sub-makes

- `MAKE` set to argv[0]; MAKELEVEL incremented per recursion.
- MAKEFLAGS re-exported through the env; `MAKE` always propagated.
- `--print-directory` auto-on for sub-makes, last-wins overrides.
- `touch` and `question` modes short-circuit recipe execution with the
  right diagnostics and exit codes.
- `-k` mode emits each prereq error and continues, marks the goal as
  "not remade because of errors".

---

## What's still missing / deferred

These are consciously skipped because each requires substantial work
for limited test count gains, or because they affect correctness only
in ways the test harness happens not to exercise in our passing set.

### High-impact, hard (unlocks many tests)

- **`.SECONDEXPANSION:`** — second expansion pass on prereqs with
  `$$@` semantics. Needed for `features/se_explicit` (3/31),
  `features/se_implicit` (3/30), `features/se_statpat` (2/12),
  many `features/patternrules` and `features/implicit_search`
  cases. Core challenge: deferred evaluation of prereq text.
- **Variable-definition-site line tracking.** We report the line
  where a function is *expanded*, not where it's *declared*. GNU
  make tags each function call with its source location in the
  makefile. Affects most `functions/*-e*` error tests.
- **Makefile auto-rebuild / re-exec** — when an included or primary
  makefile has a rule, re-run make on it, then re-exec. Needed for
  `features/reinvoke` (1/12), `options/dash-B` (5/8),
  `variables/MAKE_RESTARTS` (0/3), many `options/dash-W` and
  `options/dash-n`.
- **VPATH / vpath** — search paths for prerequisites and targets.
  Unlocks `features/vpath`, `features/vpathplus`,
  `features/vpathgpath`, `features/mult_rules`, `misc/general1`.
- **Full MAKEFLAGS → child parse** — we propagate MAKEFLAGS but don't
  re-parse the full ` -- `-separated form in sub-makes. Blocks most
  of `variables/MAKEFLAGS` (12/218).
- **Parallel jobs / jobserver** — `-j N`, `.WAIT`, `.NOTPARALLEL`,
  `output-sync`. Only hurts `features/parallelism` and
  `targets/WAIT`.
- **`.INTERMEDIATE` / `.NOTINTERMEDIATE` / `.SECONDARY` auto-delete**
  — after chain rules, remove temporary files; report the deletion.
  Hits `targets/INTERMEDIATE`, `targets/NOTINTERMEDIATE`,
  `targets/SECONDARY`.

### Lower-impact

- **`.ONESHELL:`** — pass the whole recipe body to a single shell
  invocation. Hits `targets/ONESHELL` (3/11).
- **Pattern-specific variables** (`b%: FOO = bar`) — hits
  `features/patspecific_vars` (0/10).
- **Non-tab recipe detection** — emit "missing separator" warning
  when 8 spaces look like a recipe. `misc/failure`.
- **Suffix rules** (`.c.o:` → `%.o: %.c`) fully interacting with
  built-in defaults: `features/suffixrules`, `targets/POSIX`.
- **Chained pattern rules** — follow pattern-rule chains more than
  one level deep. Previously attempted; caused regressions because
  the fallback fires too eagerly. Needs more precise criteria.
- **`--shuffle`** — `options/shuffle` (4/12).
- **Shell command-not-found rewrite** — GNU replaces `/bin/sh: X:
  command not found` with `make: X: No such file or directory`.
  `features/errors`, `misc/general4`.
- **Backslash escapes in target names** — `features/escape`.
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

## Category breakdown (current)

Based on the latest baseline run (approximate per-category pass ratios):

| Category    | Status                                                            |
| ----------- | ----------------------------------------------------------------- |
| `features`  | Partial across most sub-areas; blocked heavily on SE + VPATH.     |
| `functions` | Most working. Remaining: fatal-error line numbers, `$(eval)`-inside-variables. |
| `misc`      | `bs-nl` 16/28; `general1`–`general4` blocked on VPATH / shell err.|
| `options`   | Most parse and work; blocked on re-exec / MAKEFLAGS parse.        |
| `targets`   | POSIX / ONESHELL / INTERMEDIATE pending.                          |
| `variables` | Most done. MAKEFLAGS subcategory is the huge outlier.             |

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
