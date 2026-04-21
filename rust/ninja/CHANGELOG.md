# Changelog

All notable changes to rust-ninja.

## [Unreleased]

### Test suite compatibility

Passes 18/18 `Output.test_*` methods from the upstream ninja v1.13.1
`misc/output_test.py`, 4/4 `JobserverTest.test_*` methods from
`misc/jobserver_test.py`, and 7/7 differential roundtrip checks
(4 hand-rolled scenarios + 3 CMake-generated scenarios) against
reference `pkgs.ninja`.

- output_test.py: `test_status`, `test_pr_1685`,
  `test_ninja_status_default`, `test_ninja_status_quiet`,
  `test_entering_directory_on_stdout`, `test_issue_1418`,
  `test_issue_1214`, `test_issue_1966`, `test_issue_2499`,
  `test_issue_2048`, `test_pr_2540`,
  `test_depfile_directory_creation`, `test_tool_inputs`,
  `test_tool_compdb_targets`, `test_tool_multi_inputs`,
  `test_issue_2586`, `test_issue_2621`, `test_explain_output`.
- jobserver_test.py: `test_no_jobserver_client`,
  `test_jobserver_client_with_posix_fifo`,
  `test_jobserver_client_with_posix_pipe`,
  `test_client_passes_MAKEFLAGS`.
- Roundtrip scenarios: `cold-build`, `incremental-noop`,
  `incremental-modify`, `depfile-header-change`, `cmake-cold-build`,
  `cmake-incremental-modify`, `cmake-clean-rebuild`.

### Manifest parsing

- Variable expansion for `command`, `description`, `$in`, `$out`,
  `${var}`, `$$`, `$:`, `$|`, and `$\n` line continuations.
- Variable-name scanner tightened to ninja's `[A-Za-z0-9_-]`
  alphabet (was previously also accepting `.`). Without this,
  `$out.d` resolved as `${out.d}` instead of `${out}` + literal
  `.d`, silently breaking every depfile-driven incremental rebuild.
- `manifest::parse_file(path)` disk-aware entry point recursively
  follows `include` and `subninja` directives relative to each
  manifest's parent directory. Required to load CMake-generated
  trees that split rules across `CMakeFiles/rules.ninja`.
- `pool` declarations parsed including `depth = N`; rule- or
  edge-bound `pool = name` honored.

### Scheduler and parallelism

- Ready-queue scheduler in `build/runner.rs` with dependency
  tracking and concurrent execution with completion-order output.
- `-j N` parallelism cap.
- `-k N` keep-going threshold: `-k 0` runs unlimited, `-k N` stops
  launching new edges once `N` failures have been observed;
  in-flight edges still drain so their output is preserved.
- Per-pool in-flight counters cap concurrent edges by name; the
  implicit `console` pool defaults to depth 1.
- `any_real_work` tracking so an all-up-to-date plan still prints
  `ninja: no work to do.` (caught by the first roundtrip iteration).

### Status line

- `[N/M]` formatting with `NINJA_STATUS` env-var override.
- Smart-terminal vs piped detection via TTY + TERM.
- Smart terminal: `\r{status}\x1b[K` in-place redraw, "before"
  + "after" status pair around each edge.
- Piped: one `{status}\n` per edge, ANSI escapes stripped from
  rule output unless `CLICOLOR_FORCE=1`.
- `--quiet` suppresses status entirely; verbose mode bypasses
  in-place redraw.

### Subprocess execution and error handling

- `sh -c` subprocess spawn, captures stdout+stderr.
- Exit-code propagation per `test_pr_2540`: 124 (timeout), 127
  (command not found), 130 (SIGINT), 137 (SIGKILL), and any
  non-zero exit becomes ninja's exit code.
- `FAILED: [code=N] outputs\ncommand\n` block on failure.
- SIGINT (130) suppresses status and prints "interrupted by user".
- Unknown CLI target prints `ninja: error: unknown target 'X'`.
- `rspfile` / `rspfile_content` written before the command and
  cleaned up after success.
- `$out` and `$depfile` parent directories auto-created before the
  edge runs.

### Console pool

- True console-pool exclusive terminal locking. Console edges run
  synchronously on the main thread with stdio inherited from ninja,
  so the child can drive the terminal directly.
- Status printer's `lock_console` / `unlock_console` pair buffers
  any non-console completions that arrive during the lock and
  drains them once the console edge releases the terminal.

### Logs

- `.ninja_log` v6 reader/writer in `src/build/log.rs`.
  Tab-separated `start_ms\tend_ms\tmtime_ns\toutput\tcommand_hash`
  with FNV-1a-64 over the executed command. Loaded at the start
  of every build, appended as edges complete; recompacted by
  `-t recompact`.
- `test_issue_2048` "build log version is too old" warning when
  the existing log is older than v6.
- `.ninja_deps` v4 binary log in `src/build/deps_log.rs`. Path
  records carry a `~node_id` checksum and 4-byte NUL padding;
  deps records pack `(out_id, mtime_lo, mtime_hi, in_id, ...)`
  as little-endian i32s with the high bit of the size header set.
- Both logs loaded together at startup; populated by the runner
  after each successful `deps = gcc|msvc` edge (parses + unlinks
  the depfile, records discovered headers under the output's id)
  so subsequent invocations can answer dirtiness questions
  without the depfile.

### Depfiles and dyndep

- Makefile-style depfile parser: `target: dep1 dep2 \`,
  `\<space>` escapes, `$$` -> `$`, multi-block merging. Wired
  into the scheduler's initial dirty seeding so a header touch
  reliably re-runs the consuming compile, exactly like reference
  ninja.
- Dyndep file parser (`ninja_dyndep_version = 1`,
  `build OUT [| IMP_OUT]: dyndep [| IMP_IN]`, `restat = 1`).
- Eager dyndep load with "multiple rules generate X" detection.
- `depfile = ...` value resolution + auto-create depfile parent
  dir.

### `-d explain` and restat

- Per-node "is dirty" reasoning printed inline with build progress
  (`ninja explain: X is dirty` before each dispatched edge).
- Rule-level `restat = true` short-circuits downstream edges when
  output mtime is unchanged across a re-run.
- Full `.ninja_log` integration so dirtiness survives across
  independent ninja invocations: an output is treated as dirty if
  its recorded command hash or recorded mtime no longer matches
  the freshly-expanded command and on-disk file.

### Jobserver client

- GNU make jobserver client in `src/build/jobserver.rs`.
- Posix FIFO protocol (`--jobserver-auth=fifo:PATH`): open both
  ends of the named FIFO read+write nonblock, take the implicit
  slot first, then `read(1)` per token; `write(1)` the same byte
  back on edge completion. Combined with a raised local cap (jobs
  become unlimited while the jobserver is active), parallelism is
  bounded entirely by the upstream pool.
- FD-pair (`--jobserver-fds=R,W` / `--jobserver-auth=R,W`) detected
  and rejected with the canonical `ninja: warning: Pipe-based
  protocol is not supported!` message, then falls back to the
  local `-j N` cap.
- `MAKEFLAGS` forwarded to child processes unchanged so downstream
  make invocations pick up the same jobserver auth string.
- `GuessParallelism()` mirrored: `min` of cores+2 with a floor of 2,
  so the taskset-restricted leg of the FIFO test gets the expected
  fan-out.

### Tools

- `-t inputs` with `--dependency-order`, `--no-shell-escape`,
  `--print0`, phony skipping.
- `-t compdb-targets` JSON output, `ninja: error:` / `ninja: fatal:`
  paths.
- `-t multi-inputs` with `-d <delim>` and `--print0`.
- `-t recompact` / `-t restat` (log-version warning).
- `-t clean` with `-r` and `-g`; respects `generator = 1` and
  `deps = gcc`/`deps = msvc` exclusions to match reference ninja's
  `Cleaning... N files.` count exactly.
