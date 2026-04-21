# rust-ninja: Plan to Pass Upstream Ninja Tests

## Goal

Rewrite [Ninja](https://ninja-build.org/) in Rust, validated against the
upstream Ninja test suite. Tests run the official Python harness
(`misc/output_test.py`) from the ninja source against the rust-ninja binary
in a Nix sandbox, mirroring the differential-testing pattern established by
`rust/awk` and `rust/perl`.

## Current Status

**18/18 upstream `output_test.py` tests + 4/4 `jobserver_test.py` tests + 7/7 differential roundtrip checks passing** (4 hand-rolled scenarios + 3 CMake-generated scenarios) — `Output.test_*` methods from the upstream
ninja v1.13.1 `misc/output_test.py` run against the rust-ninja binary in a
Nix sandbox.

Passing:

- `test_status` — empty manifest, `--quiet`
- `test_pr_1685` — `-t recompact` / `-t restat` on empty graph
- `test_ninja_status_default` — `[N/M]` status line in smart-terminal
- `test_ninja_status_quiet` — `--quiet` suppresses status
- `test_entering_directory_on_stdout` — `-C` chdir banner
- `test_issue_1418` — parallel `-j3` ordering by completion time
- `test_issue_1214` — ANSI stripping in pipe mode, `CLICOLOR_FORCE`,
  verbose-mode line printing
- `test_issue_1966` — `rspfile` / `rspfile_content`
- `test_issue_2499` — in-place status redraw protocol
- `test_issue_2048` — `.ninja_log` version-mismatch warning
- `test_pr_2540` — exit-code propagation (124/127/130/137), unknown
  target error, parallel keep-going
- `test_depfile_directory_creation` — auto-create `$out` and `$depfile`
  parent directories
- `test_tool_inputs` — `-t inputs` (alpha + dep order, shell escape,
  `--print0`, phony skipping)
- `test_tool_compdb_targets` — `-t compdb-targets` JSON, error/usage paths
- `test_tool_multi_inputs` — `-t multi-inputs` (delim, `--print0`)
- `test_issue_2586` — `pool = console` jobs serialize cleanly without
  hanging (the upstream test only asserts "shouldn't hang" + exact
  output; full console-pool terminal locking is still a TODO)
- `test_issue_2621` — `dyndep = ...` file parsing + post-load
  "multiple rules generate X" detection

Recently added:

- `test_explain_output` — `-d explain` lines emitted before each
  dispatched edge, plus rule-level `restat = true` semantics that prune
  downstream edges when an output's mtime is unchanged across a re-run.

The Nix wiring is in place:

- `default.nix` exposes `pkgs.rust-ninja` (release) and `pkgs.rust-ninja-dev`
  (debug, faster compile) plus a `checks` attrset of per-test derivations.
- `testsuite.nix` extracts `pkgs.ninja.src`, drops the rust-ninja binary in
  as `./ninja`, and runs a single `Output.test_*` method from
  `misc/output_test.py`.

## Recent module additions

- `manifest::parse_file(path)` — disk-aware entry point; recursively
  follows `include` / `subninja` directives relative to each manifest’s
  parent directory. Required to load CMake-generated trees that split
  rules across `CMakeFiles/rules.ninja`.
- Variable-name scanner tightened to ninja’s `[A-Za-z0-9_-]`
  alphabet (was previously also accepting `.`). Without this, `$out.d`
  resolved as `${out.d}` instead of `${out}` + literal `.d`, silently
  breaking every depfile-driven incremental rebuild.

## Module Layout

```text
src/
  main.rs                CLI dispatch, top-level error handling
  cli.rs                 Argument parsing (-j, -k, -v, -d, -t, -C, --quiet)
  graph.rs               State, Rule, Edge data types
  manifest.rs            Tokenize + parse build.ninja, variable expansion
  status.rs              [N/M] status line, smart-terminal vs piped, ANSI strip
  build/
    mod.rs               re-exports `run`
    plan.rs              Target resolution, topological scheduling, dyndep edges
    runner.rs            Parallel scheduler, subprocess execution, rspfile,
                         depfile dir creation, exit-code mapping, eager dyndep
                         merging + multi-producer detection
    expand.rs            Edge-context variable expansion ($in, $out, layered)
    dyndep.rs            Dyndep file parser (ninja_dyndep_version = 1)
  tools/
    mod.rs               Dispatch on tool name
    recompact.rs         -t recompact / -t restat (log version warning only)
    inputs.rs            -t inputs (alpha + dep order, shell escape, --print0)
    multi_inputs.rs      -t multi-inputs
    compdb_targets.rs    -t compdb-targets (JSON output)
```

Run a test:

```sh
nix build .#checks.x86_64-linux.rust-ninja-test-test_status
```

View a failing test's log:

```sh
nix log .#checks.x86_64-linux.rust-ninja-test-test_status
```

## Test Suite Strategy

### Why output_test.py

Ninja's primary regression tests are split between:

1. `ninja_test` — a C++ unit-test binary that links against ninja's internals.
   Not portable to a re-implementation: tests poke at C++ classes directly.
2. `misc/output_test.py` — black-box tests that run the `./ninja` binary
   inside a temp build directory and diff stdout against expected strings.
3. `misc/jobserver_test.py` / `misc/ninja_syntax_test.py` — auxiliary
   Python tests for the GNU make jobserver protocol and the
   `ninja_syntax.py` helper.

Only (2) and parts of (3) are useful for differential testing of a
re-implementation. `output_test.py` is the perfect fit: it exercises the
real CLI surface and its assertions are the actual specification of
behavior we need to match.

### Phase 1: output_test.py (18 tests)

The initial test set is every `Output.test_*` method in
`misc/output_test.py` from ninja v1.13.1:

| Test | Exercises |
|------|-----------|
| `test_status` | Default "no work to do" / `--quiet` |
| `test_ninja_status_default` | Default `[N/M]` status line |
| `test_ninja_status_quiet` | Status suppression with `--quiet` |
| `test_entering_directory_on_stdout` | `-C dir` chdir banner |
| `test_issue_1418` | Parallel `-j3` build ordering |
| `test_issue_1214` | ANSI color stripping when piped, `CLICOLOR_FORCE` |
| `test_issue_1966` | `rspfile` / `rspfile_content` substitution |
| `test_issue_2499` | Status line update format with smart terminal |
| `test_issue_2048` | `-t recompact` warning on too-old log |
| `test_issue_2586` | `console` pool blocking behavior |
| `test_issue_2621` | "multiple rules generate" dyndep error |
| `test_pr_1685` | `-t recompact` / `-t restat` on empty graph |
| `test_pr_2540` | Subprocess exit code propagation (124/127/130/137) |
| `test_depfile_directory_creation` | Auto-create `$depfile` parent dir |
| `test_tool_inputs` | `-t inputs` (alpha + dependency order, escaping, `--print0`) |
| `test_tool_compdb_targets` | `-t compdb-targets` JSON output |
| `test_tool_multi_inputs` | `-t multi-inputs` (delimiter, `--print0`) |
| `test_explain_output` | `-d explain` interleaved with build lines |

### Phase 2: ninja_syntax_test.py

Port or adopt `misc/ninja_syntax.py` semantics in a Rust-side helper if
useful, otherwise drop — the syntax helper only matters for callers
generating manifests.

### Phase 3: jobserver_test.py ✅

GNU make jobserver client implemented in `src/build/jobserver.rs`,
covering the four upstream `JobserverTest.test_*` methods:

- `test_no_jobserver_client` — local `-j N` cap behaves correctly when
  `MAKEFLAGS` is unset.
- `test_jobserver_client_with_posix_fifo` — full FIFO protocol: open
  the named FIFO read+write nonblock, take the implicit slot first,
  then `read(1)` per token; `write(1)` the same byte back on edge
  completion. Combined with a raised local cap (jobs become unlimited
  while the jobserver is active) parallelism is bounded entirely by
  the upstream pool.
- `test_jobserver_client_with_posix_pipe` — `--jobserver-fds=R,W` and
  `--jobserver-auth=R,W` are detected and rejected with the canonical
  `ninja: warning: Pipe-based protocol is not supported!` message,
  matching reference ninja, then we fall back to the local `-j N` cap.
- `test_client_passes_MAKEFLAGS` — `MAKEFLAGS` is forwarded to
  child processes unchanged so downstream make invocations can
  themselves pick up the same jobserver auth string.

`GuessParallelism()` from reference ninja is also mirrored
(`min` of cores+2 with a floor of 2) so the taskset-restricted leg of
the FIFO test gets the expected fan-out.

### Phase 4: Differential roundtrip tests ✅ (initial)

Implemented in `roundtrip.nix`. Each scenario builds a small two-TU
C project (`src/greet.c`, `src/main.c`, `inc/greet.h`) with both
rust-ninja and reference `pkgs.ninja` and compares observable
behavior (gcc embeds absolute build paths in object files, so a strict
`cmp` of artifacts isn’t meaningful — the checks instead validate
file existence, exit message, mtime stability, and that the built
binary actually runs):

| Scenario | Validates |
|----------|-----------|
| `cold-build` | both runners produce all expected outputs and `app` prints `hello` |
| `incremental-noop` | both runners say `ninja: no work to do.` on a re-run |
| `incremental-modify` | touching `src/main.c` rebuilds `main.o` + `app`, leaves `greet.o` mtime stable |
| `depfile-header-change` | both runners rebuild `greet.o` via `gcc -MMD` depfile parsing — mtime must change on both sides |
| `cmake-cold-build` | a real CMake-generated `build.ninja` tree (with `include CMakeFiles/rules.ninja`, `$DEP_FILE` bindings, `restat`) cold-builds, no-ops on re-run, and rebuilds correctly after a header touch on both runners |
| `cmake-incremental-modify` | touching one `.c` in a CMake tree rebuilds only the affected `.o` + `app`; the unrelated `greet.c.o` mtime stays stable on both runners |
| `cmake-clean-rebuild` | each runner invokes its own `-t clean`, the deletion-count strings must match exactly, then a fresh invocation cold-rebuilds the full project on both runners |

The first iteration surfaced one real bug: when every edge in the plan
is up to date, rust-ninja exited silently instead of printing
`ninja: no work to do.` Fixed by tracking `any_real_work` in the
scheduler.

---

## Architecture

Ninja is conceptually modest but operationally rich. The C++ source is
~25k lines; a faithful Rust port should fit in the same order of magnitude
once tests are passing. Module sketch:

```text
src/
  main.rs              CLI dispatch, top-level error handling
  cli.rs               Argument parsing (-j, -k, -v, -d, -t, -C, --quiet, ...)
  manifest/
    lexer.rs           Tokenize build.ninja (vars, rules, builds, includes)
    parser.rs          Parse into the build graph
    eval.rs            Variable expansion, scoping (file/rule/build/per-edge)
  graph/
    node.rs            Files (inputs/outputs)
    edge.rs            Build edges (rule + bindings + ins + outs)
    state.rs           Global graph state, pools
    plan.rs            Topological scheduling, ready queue
  build/
    runner.rs          Edge execution, parallel job pool, -j, -k
    subprocess.rs      Spawn, capture stdout/stderr, exit-code translation
    status.rs          [N/M] status line, smart-terminal vs pipe rendering,
                       ANSI color stripping, CLICOLOR_FORCE
    console_pool.rs    `pool = console` exclusive locking
  deps/
    depfile.rs         Makefile-style depfile parsing (gcc -MD)
    deps_log.rs        .ninja_deps binary log read/write
    build_log.rs       .ninja_log: command hash, mtimes, restat tracking
    dyndep.rs          Dynamic dependency files (ninja_dyndep_version=1)
  tools/
    inputs.rs          -t inputs (alpha + dependency order, --print0,
                       --no-shell-escape, --dependency-order)
    compdb.rs          -t compdb / -t compdb-targets (JSON)
    multi_inputs.rs    -t multi-inputs
    targets.rs         -t targets
    rules.rs           -t rules
    clean.rs           -t clean
    graph.rs           -t graph (dot output)
    query.rs           -t query
    browse.rs          -t browse (HTTP server) — likely skip
    recompact.rs       -t recompact (.ninja_log + .ninja_deps)
    restat.rs          -t restat
  util/
    canon_path.rs      Path canonicalization (// → /, etc.)
    disk_interface.rs  Stat caching
    metrics.rs         -d stats
```

### Key data shapes

- **Node**: a file in the build graph. Has `path`, `mtime`, `dirty` flag,
  optional `in_edge` (the edge that produces it), and a list of
  `out_edges` (edges that consume it).
- **Edge**: a build statement instance. Has a `Rule` reference, an
  `EdgeEnv` (lazy evaluation context for `$in`, `$out`, etc.), input
  arrays partitioned into explicit / implicit / order-only, output
  arrays partitioned into explicit / implicit, and a `pool`.
- **Pool**: a name + depth. The `console` pool has depth 1 and grants
  ownership of the controlling terminal.
- **State**: the whole parsed manifest — bindings (file scope), rules,
  pools, edges, nodes (interned by canonical path).

### Variable expansion

Ninja's variable lookup is layered: per-edge bindings → rule bindings →
file-scope (with subninja/include creating nested scopes). `$in` and
`$out` are computed dynamically per edge. `$in_newline` joins with `\n`.
Missing variables expand to the empty string.

### Status line

`output_test.py` is heavily focused on the status line. Two modes:

- **Smart terminal** (TTY on stdout): emit `\r{status}\x1b[K` to redraw
  in place. ANSI escapes from rule output flow through unchanged.
- **Piped**: emit `{status}\n` once per edge, strip ANSI escapes from
  rule output unless `CLICOLOR_FORCE=1`.

`NINJA_STATUS` env var overrides the format; `--quiet` suppresses status
lines entirely.

### Console pool

Edges in `pool = console` get exclusive access to stdin/stdout/stderr
during execution. Other parallel jobs may run simultaneously but must
not write to the terminal. `test_issue_2586` covers correct ordering
when console-pool edges depend on regular edges.

---

## Implementation Phases

### Phase 0: Scaffolding ✅

- [x] `Cargo.toml` + stub `main.rs`
- [x] `default.nix` with `rust-ninja` and `rust-ninja-dev` packages
- [x] `testsuite.nix` driving `output_test.py` per-method
- [x] PLAN.md

### Phase 1: Trivial output ✅ (`test_status`)

- [x] Manifest lexer/parser for empty files and basic rule/build/default
- [x] CLI: `-C`, `--quiet`, `-f`, `-j`, `-d`, `-t`, `-v`, `--version`
- [x] Empty graph → "ninja: no work to do."

### Phase 2: Single-edge build ✅ (`test_ninja_status_default`, `test_ninja_status_quiet`)

- [x] Parse `rule` and `build` statements with bindings
- [x] Variable expansion for `command`, `description`, `$in`, `$out`,
      `${var}`, `$$`, `$`, `$:`, `$|`, `$\n` continuations
- [x] Spawn subprocess via `sh -c`, capture stdout+stderr
- [x] `--quiet` suppresses status

### Phase 3: Parallel scheduler ✅ (`test_issue_1418`)

- [x] `-j N` flag
- [x] Ready-queue scheduler (`build/runner.rs`) with dependency tracking
- [x] Concurrent execution with completion-order output

### Phase 4: Smart-terminal status ✅ (`test_issue_2499`, `test_issue_1214`, `test_issue_1966`)

- [x] TTY + TERM detection
- [x] `\r…\x1b[K` in-place redraw, "before" + "after" status pair
- [x] ANSI stripping in piped mode, honoring `CLICOLOR_FORCE`
- [x] `rspfile` / `rspfile_content` write/cleanup
- [x] Verbose mode bypasses in-place redraw

### Phase 5: Subprocess error handling ✅ (`test_pr_2540`)

- [x] Exit code propagation (any non-zero → ninja exit code)
- [x] `FAILED: [code=N] outputs\ncommand\n` block
- [x] SIGINT (130) → suppressed status, "interrupted by user"
- [x] Unknown CLI target → `ninja: error: unknown target 'X'`
- [x] `-k N` keep-going threshold — `-k 0` runs unlimited, `-k N`
      stops launching new edges once `N` failures have been observed;
      in-flight edges still drain so their output is preserved

### Phase 6: Tools ✅ (`test_tool_inputs`, `test_tool_compdb_targets`, `test_tool_multi_inputs`, `test_pr_1685`)

- [x] `-t inputs` with `--dependency-order`, `--no-shell-escape`,
      `--print0`, phony skipping
- [x] `-t compdb-targets` JSON, `ninja: error:` / `ninja: fatal:` paths
- [x] `-t multi-inputs` with `-d <delim>` and `--print0`
- [x] `-t recompact`, `-t restat` (log-version check only)
- [x] `-t clean` (with `-r`, `-g`; respects `generator = 1` and
      `deps = gcc`/`deps = msvc` exclusions to match reference
      ninja’s `Cleaning... N files.` count exactly)

### Phase 7: Logs (target: `test_issue_2048` ✅, `test_explain_output` ✅)

- [x] `.ninja_log` v6 version-mismatch warning (subset for `test_issue_2048`)
- [x] Full `.ninja_log` v6 reader/writer in `src/build/log.rs`,
      tab-separated `start_ms\tend_ms\tmtime_ns\toutput\tcommand_hash`
      with FNV-1a-64 over the executed command. Loaded at the start
      of every build and appended to as edges complete; recompacted
      by `-t recompact` (test_issue_2048 still gets its
      "build log version is too old" warning when the existing log
      is older than v6).
- [x] `-d explain` interleaved with build progress
- [ ] `.ninja_deps` binary format

### Phase 8: Pools and console (target: `test_issue_2586` ✅)

- [x] `pool` declaration is parsed including `depth = N`
- [x] `pool = console` runs without hanging — sequential by default
      because we never block our own output stream
- [x] Pool depth limiting — per-pool in-flight counters cap concurrent
      edges by name (rule- or edge-bound `pool = name`); the implicit
      `console` pool defaults to depth 1
- [ ] True console-pool exclusive terminal locking (`SetConsoleLocked`
      buffering of competing edges)

### Phase 9: Depfiles and dyndep ✅ (target: `test_depfile_directory_creation` ✅, `test_issue_2621` ✅, roundtrip `depfile-header-change` ✅)

- [x] `depfile = ...` value resolution + auto-create depfile parent dir
- [x] `dyndep = ...` per-edge OR per-rule dynamic dependencies
- [x] Minimal dyndep file parser (`ninja_dyndep_version = 1`,
      `build OUT [| IMP_OUT]: dyndep [| IMP_IN]`, `restat = 1`)
- [x] Eager dyndep load + "multiple rules generate X" detection
- [x] Makefile-style depfile parser (`target: dep1 dep2 \`,
      `\<space>` escapes, `$$` -> `$`, multi-block merging) wired
      into the scheduler’s initial dirty seeding so a header touch
      reliably re-runs the consuming compile, exactly like reference
      ninja

### Phase 10: `-d explain` ✅ (target: `test_explain_output`)

- [x] Rule-level `restat = true` short-circuits downstream edges when
      output mtime is unchanged across a re-run
- [x] Per-node "is dirty" reasoning, printed inline with build progress
      (`ninja explain: X is dirty` before each dispatched edge)
- [x] Full `.ninja_log` integration so dirtiness survives across
      independent ninja invocations: an output is treated as dirty
      if its recorded command hash or recorded mtime no longer match
      the freshly-expanded command and on-disk file

---

## Reference Material

- Ninja source: <https://github.com/ninja-build/ninja/tree/v1.13.1>
- Manual: <https://ninja-build.org/manual.html>
- File format: <https://ninja-build.org/manual.html#_ninja_file_reference>
- Existing Rust ports for reference (do not copy, only consult):
  - n2 (Evan Martin's own follow-up): <https://github.com/evmar/n2>
  - samurai (C reimpl): <https://github.com/michaelforney/samurai>

## Out of Scope (initially)

- Windows-specific code paths (`output_test.py` skips on Windows)
- `-t browse` (HTTP server)
- `-t graph` (Graphviz dot output) — easy add later
- `subninja` chained scope edge cases beyond what tests exercise
- MSVC `/showIncludes` deps style
