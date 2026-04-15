# rust-sed: Plan to Pass Upstream GNU sed Tests

## Current Status

**31/61 tests passing** (51%) — shell tests from the GNU sed 4.9 test suite.

Run a test: `nix build .#checks.x86_64-linux.rust-sed-test-{name}`
View failure: `nix log .#checks.x86_64-linux.rust-sed-test-{name}`

### Recent fixes

- Split main.rs into modules: types, parser, regex_util, engine, util
- `#n` quiet mode, inline `#` comments, `e`/`F`/`v` commands
- `m`/`M` multiline substitution flag, `e` substitution flag (`pe`/`ep` ordering)
- `\U`/`\L`/`\u`/`\l`/`\E` case conversion, `\0` backreference in replacement
- `\c`/`\d`/`\o`/`\x` escape sequences (replacement and transliterate)
- `ctrl_char()` case handling (`\ca` == `\cA`)
- Per-command address range state tracking
- `l` command line wrapping with configurable width
- `R` command successive line reads per file
- Branch propagation from `{...}` blocks to parent command list
- BRE `\` inside `[]` character classes (POSIX literal vs Rust regex escape)
- In-place editing: `-i` arg parsing, `*` suffix expansion, fresh engine per file
- POSIX mode: `N` on last line suppresses print, `POSIXLY_CORRECT` env var
- `-f -` reads script from stdin
- Multi-line `a\`/`i\`/`c\` text continuation
- IO error messages stripped of Rust's `(os error N)` suffix

---

## Remaining Failure Categories (30 tests)

### Category 1: Error message format (11 tests)

GNU sed error messages include expression number and character position:
`sed: -e expression #1, char N: <message>`. Our errors are simpler: `sed: <message>`.

**Failing tests:** colon-with-no-label, compile-errors, compile-tests, missing-filename, normalize-text, panic-tests, posix-mode-bad-ref, posix-mode-s, recursive-escape-c, regex-errors, temp-file-cleanup

**Fix:** Track expression index and character position in the parser. Format errors as `sed: -e expression #N, char M: <message>` (or `sed: file <name> line N: <message>` for `-f` scripts). This is a single cross-cutting change that would fix or partially fix all 11 tests.

**Specific sub-issues:**

- `compile-errors` — Also needs detection of duplicate `p`/`g` flags in `s` command
- `compile-tests` — Also needs `a\` end-of-buffer handling to produce empty append (partially done)
- `missing-filename` — Needs `r`/`R`/`w`/`W` with empty filename to be an error
- `panic-tests` — Needs specific panic/signal handling and temp file error messages
- `posix-mode-s` — Needs POSIX mode to reject GNU extension `s` flags (`I`, `m`, `e`)
- `recursive-escape-c` — Needs `\c\d` to produce "recursive escaping after \c" error, exit code 1

### Category 2: Backreferences — needs `fancy-regex` (3 tests)

Rust's `regex` crate does not support backreferences (`\1` in patterns). GNU sed uses them heavily.

**Failing tests:** binary, dc, uniq

**Fix:** Switch from `regex` to `fancy-regex` crate (or use it as fallback when backreferences are detected). This is a significant change since `fancy-regex` has different performance characteristics and API.

**Impact:** `dc` is a dc calculator implemented in sed. `uniq` is a uniq implementation. Both are complex integration tests that exercise many features beyond backreferences.

### Category 3: Binary / non-UTF-8 input (3 tests)

Our sed uses `BufRead::lines()` which requires valid UTF-8. Non-UTF-8 input causes `stream did not contain valid UTF-8` errors.

**Failing tests:** 8to7, mac-mf, obinary (skips on Linux — platform-specific)

**Fix:** Read input as raw bytes instead of strings. Process lines as `Vec<u8>` rather than `String`. The regex crate supports matching on `&[u8]` via `regex::bytes::Regex`. This is a fundamental change affecting the entire engine.

### Category 4: Null-data mode `-z` (2 tests)

The `-z` flag should use NUL (0x00) as line separator instead of newline. Currently not implemented.

**Failing tests:** nulldata, execute-tests (partial — only the `-z` sub-test fails)

**Fix:** When `-z` is set, split input on NUL bytes instead of newlines. Output NUL-terminated records instead of newline-terminated. The `l`, `=`, `F` commands need null-aware output.

### Category 5: GNU extensions not implemented (4 tests)

**Failing tests:** cmd-0r, follow-symlinks, follow-symlinks-stdin, sandbox

- `cmd-0r` — Address `0` (before first line) and `0,/regex/` ranges not supported
- `follow-symlinks` — `--follow-symlinks` flag not implemented
- `follow-symlinks-stdin` — Same
- `sandbox` — `--sandbox` flag not implemented (disables `e`/`r`/`w` commands)

**Fix:** Implement `0` address support, `--follow-symlinks`, and `--sandbox` flags.

### Category 6: Unbuffered I/O (1 test)

`sed -u` should read/write line-by-line without buffering. Our implementation reads all input into memory at once, so `sed -u 1q` consumes all stdin even though it only processes one line.

**Failing test:** unbuffered

**Fix:** Implement true line-by-line reading mode when `-u` is set. Read one line, process it, flush output, then read the next. This requires restructuring the `Engine::run` method.

### Category 7: `l` command width with `-l` flag (1 test)

The `l` command line wrapping works, but the `-l N` command-line flag (set default width) is not parsed.

**Failing test:** cmd-l (partial — default width and per-command width work, `-l` flag doesn't)

**Fix:** Add `-l N` to argument parser and pass to engine's `line_wrap_width`.

### Category 8: POSIX mode edge cases (1 test)

**Failing test:** posix-mode-addr

- `0,/regex/` address range (address `0`) not supported
- `addr1,+N` and `addr1,~N` GNU address extensions not supported
- POSIX mode should reject address `0`

### Category 9: Test infrastructure issues (2 tests)

**Failing tests:** bsd-wrapper, help

- `bsd-wrapper` — The `bsd.sh` script expects sed at `../sed/sed` relative path, which doesn't match our Nix sandbox layout
- `help` — Expects `--help` output to match between `-e` and `--help` invocations (our help text differs from GNU sed)

### Category 10: Escape sequences in replacement (1 test)

**Failing test:** convert-number

- `\dNNN` (decimal), `\oNNN` (octal), `\xNN` (hex) escape sequences in replacement partially work but produce wrong byte values for multi-byte sequences and interact incorrectly with the sed script's own escaping

### Category 11: POSIX `\t` in regex (1 test)

**Failing test:** posix-char-class (partial — test 3 only)

- In `--posix` mode, `\t` in regex patterns should mean literal `\` + `t` per POSIX (undefined escape), but GNU sed still treats `\t` as tab even in POSIX mode when inside character classes. Complex edge case.

---

## Implementation Priorities

### Priority 1: Error message format (11 tests)

Highest impact — fixes or partially fixes 11 tests. Track source location in parser and format errors as `sed: -e expression #N, char M: <message>`.

### Priority 2: `fancy-regex` for backreferences (3 tests)

Required for `dc`, `uniq`, `binary`. These are complex integration tests that also validate other features. Consider using `fancy-regex` as fallback only when `\1`-`\9` is detected in patterns (to avoid performance impact).

### Priority 3: Binary/byte-mode input (3 tests)

Switch to `regex::bytes::Regex` and process `Vec<u8>` lines. Required for `8to7`, `mac-mf`, and would improve robustness for all real-world usage.

### Priority 4: Address `0` and GNU address extensions (2 tests)

Implement `0` address, `0,/regex/` ranges, `addr,+N`, `addr,~N`. Fixes `cmd-0r` and `posix-mode-addr`.

### Priority 5: `-z` null-data mode (2 tests)

Split on NUL instead of newline. Fixes `nulldata` and part of `execute-tests`.

### Priority 6: `-l N` flag, `--sandbox`, `--follow-symlinks` (4 tests)

Low-hanging fruit for flag implementation. Fixes `cmd-l`, `sandbox`, `follow-symlinks`, `follow-symlinks-stdin`.

### Priority 7: Unbuffered I/O (1 test)

Restructure input reading for `-u` mode. Fixes `unbuffered`.

---

## Test Inventory

### Passing (31 tests)

bsd, bug32082, bug32271-1, bug32271-2, cmd-R, command-endings, comment-n,
distrib, eval, in-place-hyphen, in-place-suffix-backup, inplace-hold,
inplace-selinux (skip), madding, mb-bad-delim, mb-charclass-non-utf8,
mb-match-slash, mb-y-translate, newline-dfa-bug, posix-mode-ERE,
posix-mode-N, range-overlap, regex-max-int, stdin, stdin-prog,
subst-mb-incomplete, subst-options, subst-replacement, title-case,
word-delim, xemacs

### Failing (30 tests)

| Test | Primary blocker |
|-|-|
| 8to7 | Binary/non-UTF-8 input |
| binary | Backreferences (`fancy-regex`) |
| bsd-wrapper | Test infrastructure (path layout) |
| cmd-0r | Address `0` not implemented |
| cmd-l | `-l N` flag not parsed |
| colon-with-no-label | Error message format |
| compile-errors | Error message format + duplicate flag detection |
| compile-tests | Error message format |
| convert-number | Replacement escape edge cases |
| dc | Backreferences (`fancy-regex`) |
| execute-tests | `-z` null-data mode |
| follow-symlinks | `--follow-symlinks` not implemented |
| follow-symlinks-stdin | `--follow-symlinks` not implemented |
| help | Help text mismatch |
| mac-mf | Binary/non-UTF-8 input |
| missing-filename | Error message format + empty filename validation |
| normalize-text | Error message format |
| nulldata | `-z` null-data mode |
| obinary | Skips on Linux (platform-specific) |
| panic-tests | Error message format + signal handling |
| posix-char-class | POSIX `\t` edge case |
| posix-mode-addr | Address `0` + GNU extensions |
| posix-mode-bad-ref | Error message format |
| posix-mode-s | Error message format + POSIX flag validation |
| recursive-escape-c | Error message format + `\c\d` error |
| regex-errors | Error message format + backreference errors |
| sandbox | `--sandbox` not implemented |
| temp-file-cleanup | Error message format |
| unbuffered | Unbuffered I/O not implemented |
| uniq | Backreferences (`fancy-regex`) |

### Tests excluded from harness (6 tests)

Not included in `default.nix` due to environment requirements:

- `8bit` — Non-UTF-8 binary input file
- `badenc` — Requires specific locale
- `help-version` — Requires exact GNU version strings
- `invalid-mb-seq-UMR` — Requires specific locale
- `newjis` — Requires Japanese shift-JIS locale
- `utf8-ru` — Requires Russian UTF-8 locale
