# rust-ninja

A GNU Ninja-compatible build tool written in Rust.

## Status

**18/18 upstream `output_test.py` + 4/4 `jobserver_test.py` + 7/7
differential roundtrip checks passing**

The Python harnesses (`misc/output_test.py`, `misc/jobserver_test.py`)
are extracted from the upstream ninja v1.13.1 source in a Nix sandbox
and run against the rust-ninja binary, mirroring the
differential-testing pattern established by `rust/awk` and `rust/perl`.
The roundtrip checks in `roundtrip.nix` build the same C project with
both rust-ninja and reference `pkgs.ninja` and compare observable
behavior.

## Usage

Run a single upstream test:

```sh
nix build .#checks.x86_64-linux.rust-ninja-test-{name}
```

View a failing test's log:

```sh
nix log .#checks.x86_64-linux.rust-ninja-test-{name}
```

Batch-run every test in a single evaluator (much faster than looping):

```sh
nix build .#checks.x86_64-linux.rust-ninja-test-* --keep-going --no-link
```

The binary is available as `ninja` from `pkgs.rust-ninja` (release
build) or `pkgs.rust-ninja-dev` (debug build, faster compile).

## Architecture

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
    depfile.rs           Makefile-style depfile parser (gcc -MMD output)
    log.rs               .ninja_log v6 reader/writer (FNV-1a command hash)
    deps_log.rs          .ninja_deps v4 binary reader/writer
    jobserver.rs         GNU make jobserver client (FIFO + posix-fd auth)
  tools/
    mod.rs               Dispatch on tool name
    recompact.rs         -t recompact / -t restat (log version warning only)
    inputs.rs            -t inputs (alpha + dep order, shell escape, --print0)
    multi_inputs.rs      -t multi-inputs
    compdb_targets.rs    -t compdb-targets (JSON output)
    clean.rs             -t clean (-r, -g, generator/deps exclusions)
```

## Features

### Manifest parsing

- Variable expansion with `$in`, `$out`, `${var}`, `$$`, `$:`, `$|`,
  and `$\n` line continuations.
- Variable-name alphabet matches ninja's `[A-Za-z0-9_-]` (no `.`),
  so `$out.d` resolves as `${out}` + literal `.d` and depfile-driven
  incremental rebuilds work.
- `manifest::parse_file(path)` is the disk-aware entry point;
  recursively follows `include` and `subninja` directives relative to
  each manifest's parent directory. Loads CMake-generated trees that
  split rules across `CMakeFiles/rules.ninja`.
- `pool` declarations parsed including `depth = N`.

### Scheduler

- Ready-queue scheduler with dependency tracking.
- `-j N` parallelism cap and parallel completion-order output.
- `-k N` keep-going threshold: `-k 0` runs unlimited, `-k N` stops
  launching new edges once `N` failures have been observed; in-flight
  edges still drain so their output is preserved.
- Jobserver-aware token gating (see below).
- Per-pool in-flight counters cap concurrent edges by name; the
  implicit `console` pool defaults to depth 1.

### Status line

- `[N/M]` formatting with `NINJA_STATUS` env override.
- Smart-terminal vs piped detection via TTY + TERM.
- Smart terminal: `\r{status}\x1b[K` in-place redraw, "before" +
  "after" status pair around each edge.
- Piped: one `{status}\n` per edge, ANSI escapes stripped from rule
  output unless `CLICOLOR_FORCE=1`.
- `--quiet` suppresses status entirely; verbose mode bypasses
  in-place redraw.

### Subprocess execution

- `sh -c` subprocess spawn, captures stdout+stderr.
- Exit-code propagation: 124 (timeout), 127 (command not found),
  130 (SIGINT), 137 (SIGKILL).
- `FAILED: [code=N] outputs\ncommand\n` block on failure.
- SIGINT (130) suppresses status and prints "interrupted by user".
- `rspfile` / `rspfile_content` written before the command and
  cleaned up after success.
- `$out` and `$depfile` parent directories are auto-created before
  the edge runs.

### Console pool

- True exclusive terminal locking. Edges in `pool = console` run
  synchronously on the main thread with stdio inherited from ninja
  (so the child can drive the terminal directly).
- The status printer's `lock_console` / `unlock_console` pair buffers
  any non-console completions that arrive during the lock and drains
  them once the console edge releases the terminal.

### Logs

- `.ninja_log` v6: tab-separated
  `start_ms\tend_ms\tmtime_ns\toutput\tcommand_hash` with FNV-1a-64
  over the executed command. Loaded at the start of every build,
  appended to as edges complete; recompacted by `-t recompact`.
- `.ninja_deps` v4 binary log. Path records carry a `~node_id`
  checksum and 4-byte NUL padding; deps records pack
  `(out_id, mtime_lo, mtime_hi, in_id, ...)` as little-endian i32s
  with the high bit of the size header set.
- Both logs loaded together at startup; populated after each
  successful `deps = gcc|msvc` edge (parses + unlinks the depfile,
  records discovered headers under the output's id) so subsequent
  invocations can answer dirtiness questions without the depfile.

### Depfiles and dyndep

- Makefile-style depfile parser: `target: dep1 dep2 \`,
  `\<space>` escapes, `$$` -> `$`, multi-block merging.
- Wired into the scheduler's initial dirty seeding so a header touch
  reliably re-runs the consuming compile, exactly like reference
  ninja.
- Dyndep file parser: `ninja_dyndep_version = 1`,
  `build OUT [| IMP_OUT]: dyndep [| IMP_IN]`, `restat = 1`.
- Eager dyndep load with "multiple rules generate X" detection.

### `-d explain` and restat

- Per-node "is dirty" reasoning printed inline with build progress
  (`ninja explain: X is dirty` before each dispatched edge).
- Rule-level `restat = true` short-circuits downstream edges when
  output mtime is unchanged across a re-run.
- Full `.ninja_log` integration so dirtiness survives across
  independent ninja invocations: an output is treated as dirty if
  its recorded command hash or recorded mtime no longer matches the
  freshly-expanded command and on-disk file.

### Jobserver client

- GNU make jobserver client in `src/build/jobserver.rs`.
- Posix FIFO protocol (`--jobserver-auth=fifo:PATH`): open both ends
  nonblock, take the implicit slot first, then `read(1)` per token
  and `write(1)` the same byte back on edge completion. Local cap
  is raised while the jobserver is active so parallelism is bounded
  entirely by the upstream pool.
- FD-pair (`--jobserver-fds=R,W` / `--jobserver-auth=R,W`) emits the
  canonical `ninja: warning: Pipe-based protocol is not supported!`
  matching reference ninja, then falls back to the local `-j N` cap.
- `MAKEFLAGS` forwarded unchanged to children so downstream make
  invocations pick up the same auth string.
- `GuessParallelism()` mirrored: `min(cores+2, 2)` floor.

### Tools

- `-t inputs` with `--dependency-order`, `--no-shell-escape`,
  `--print0`, phony skipping.
- `-t compdb-targets` JSON, `ninja: error:` / `ninja: fatal:` paths.
- `-t multi-inputs` with `-d <delim>` and `--print0`.
- `-t recompact` / `-t restat` (log-version warning).
- `-t clean` with `-r`, `-g`; respects `generator = 1` and
  `deps = gcc|msvc` exclusions to match reference ninja's
  `Cleaning... N files.` count exactly.

## Test suite strategy

### Why output_test.py

Ninja's primary regression tests are split between:

1. `ninja_test` — a C++ unit-test binary that links against ninja's
   internals. Not portable to a re-implementation: tests poke at
   C++ classes directly.
2. `misc/output_test.py` — black-box tests that run the `./ninja`
   binary inside a temp build directory and diff stdout against
   expected strings.
3. `misc/jobserver_test.py` / `misc/ninja_syntax_test.py` —
   auxiliary Python tests for the GNU make jobserver protocol and
   the `ninja_syntax.py` helper.

Only (2) and (3) are useful for differential testing of a
re-implementation. `output_test.py` is the perfect fit: it exercises
the real CLI surface and its assertions are the actual specification
of behavior we need to match.

### Differential roundtrip tests

`roundtrip.nix` builds a small two-TU C project (`src/greet.c`,
`src/main.c`, `inc/greet.h`) with both rust-ninja and reference
`pkgs.ninja`, then compares observable behavior. (gcc embeds absolute
build paths in object files, so a strict `cmp` of artifacts is not
meaningful — the checks instead validate file existence, exit
message, mtime stability, and that the built binary actually runs.)

| Scenario | Validates |
|----------|-----------|
| `cold-build` | both runners produce all expected outputs and `app` prints `hello` |
| `incremental-noop` | both runners say `ninja: no work to do.` on a re-run |
| `incremental-modify` | touching `src/main.c` rebuilds `main.o` + `app`, leaves `greet.o` mtime stable |
| `depfile-header-change` | both runners rebuild `greet.o` via `gcc -MMD` depfile parsing — mtime must change on both sides |
| `cmake-cold-build` | a real CMake-generated `build.ninja` tree (with `include CMakeFiles/rules.ninja`, `$DEP_FILE` bindings, `restat`) cold-builds, no-ops on re-run, and rebuilds correctly after a header touch on both runners |
| `cmake-incremental-modify` | touching one `.c` in a CMake tree rebuilds only the affected `.o` + `app`; the unrelated `greet.c.o` mtime stays stable on both runners |
| `cmake-clean-rebuild` | each runner invokes its own `-t clean`, the deletion-count strings must match exactly, then a fresh invocation cold-rebuilds the full project on both runners |

## Out of scope

- Windows-specific code paths (`output_test.py` skips on Windows).
- `-t browse` (HTTP server).
- `-t graph` (Graphviz dot output).
- MSVC `/showIncludes` deps style.
- `subninja` chained scope edge cases beyond what tests exercise.

## Reference material

- Ninja source: <https://github.com/ninja-build/ninja/tree/v1.13.1>
- Manual: <https://ninja-build.org/manual.html>
- Existing Rust ports for reference (do not copy, only consult):
  - n2 (Evan Martin's own follow-up): <https://github.com/evmar/n2>
  - samurai (C reimpl): <https://github.com/michaelforney/samurai>
