# Changelog

All notable changes to `rust-grep`. Tests refer to the GNU grep 3.12 suite.

## Unreleased

### PCRE / regex

- Detect nested unbounded quantifiers in rust-pcre2 (e.g. `((a+)*)+$`); return
  `MatchLimit` on non-trivial inputs that fail to match, mirroring PCRE2's
  exponential-backtracking safeguard. Fixes `pcre-abort`.
- Integrate `rust-pcre2` for `-P` mode (replaces `fancy-regex`).
- Cross-pattern backreference validation (exit 2 for `\1` referring to a group
  outside the current `-e` pattern).
- Automatic `fancy-regex` fallback for BRE/ERE patterns containing
  backreferences (the `regex` crate does not support them).
- Prefer longest match at each position for multiple `-e` patterns.
- PCRE `-w` uses `(?<!\w)(?!\w)` instead of `\b` (matches PCRE2 semantics).
- BRE-to-ERE conversion: `^`/`$` anchor position, literal `|`, bracket
  expressions, literal `*` at start, interval validation, unmatched paren
  detection, leading `*` in ERE.
- POSIX character class validation (exit 2 for invalid names).
- Per-pattern validation for unclosed bracket expressions.
- Escape `[` inside bracket expressions for `regex` crate compatibility.
- Stack-overflow detection for deeply nested regex; print error instead of
  crashing.
- Character-class misuse warning (`[:space:]` vs `[[:space:]]`).
- Formatted regex error messages to match GNU grep style.

### Options

- `-L` (files-without-match) prints filenames but exit code tracks real matches,
  not `-L` output (GNU behaviour).
- Exit code 2 (error) now takes precedence over 0 (match), except `-q`
  short-circuits on first match.
- `-r` vs `-R` split: `-R` uses `WalkDir::follow_links(true)`, detects
  recursion loops, and reports walker errors.
- Skip-read optimisation for `-m0`, `-f /dev/null`, `-v ""`: don't open files
  when no match is possible.
- Empty `-f` pattern file no longer early-exits; `-L` can still print.
- `-f FILE` includes empty lines as empty patterns; handles `-f /dev/null`
  correctly; fixed arg-parsing bug.
- `-e` patterns split on newlines per GNU semantics.
- `-NUM` context shorthand (e.g. `-3` for `-C 3`).
- `-T`/`--initial-tab` alignment after prefix.
- `-a`/`--text` and `--binary-files=text`.
- `-D skip`/`--devices=skip` to skip device files.
- `-d`/`--directories` with skip/read/recurse.
- `--exclude-dir` option for recursive search.
- `-r` defaults to current directory when no files are given.
- `-w` with `-F` applies word boundary checking to fixed strings.
- Empty pattern with `-w` and `-x` matches only empty lines.
- `-x` line matching works for regex mode.
- `--line-buffered` accepted as a no-op.
- `--help` output goes to stdout (not stderr).

### Output

- `--color=always` emits `\033[01;31m\033[K` / `\033[m\033[K` around matches.
- `GREP_COLORS` and deprecated `GREP_COLOR` environment variables honored
  (with deprecation warning for the latter).
- Binary-file detection emits "binary file matches"; ahead-peek catches late
  NUL bytes in files.
- Write-error detection via `checked_write!`/`checked_writeln!` macros;
  `safe_exit` flushes stdout and reports failures.

### Files and I/O

- Input-is-output detection (inode comparison) prevents infinite loops when
  `grep pattern file > file`.
- `lseek` stdin after `-m` so subsequent pipeline processes can read the rest.
- Skip input=output check for `-m` (max-count limits reading).
- `--include`/`--exclude` ordering semantics match GNU grep.
- `--include` filter applied to non-recursive file arguments.
- `--exclude-dir` root handling: don't exclude the implicit default `"."` root.
- `--directories=skip` and `--exclude-dir` strip leading `./` when matching.
- Prevent stdin fallback when file arguments were given but all excluded.
- Full glob matching for `--include`/`--exclude` (`*`, `?`, `[...]`).
- Raw byte I/O for non-UTF-8 input (C locale high bytes).
- `args_os()` for non-UTF-8 command-line arguments.

### Exit codes

- Return 2 on file errors; separate `-s` (`--no-messages`) from `-q`.
- `-L` early return and exit code fixes.

### Infrastructure

- 120s test timeout in `testsuite.nix`.
- `main.rs` split into six modules (`args`, `files`, `grep`, `matcher`,
  `pattern`, `main`).
