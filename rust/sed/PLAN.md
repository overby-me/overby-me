# rust-sed: Plan to Pass Upstream GNU sed Tests

## Current Status

**48/61 tests passing** (79%) — shell tests from the GNU sed 4.9 test suite.

Run a test: `nix build .#checks.x86_64-linux.rust-sed-test-{name}`
View failure: `nix log .#checks.x86_64-linux.rust-sed-test-{name}`

### Implemented features

- Module structure: types, parser, regex_util, engine, util
- `#n` quiet mode, inline `#` comments
- Commands: `e` (execute), `F` (filename), `v` (version), `Q` (quit no-print)
- `s` flags: `m`/`M` (multiline), `e` (execute), `i`/`I` (case-insensitive), `p`/`e` ordering
- Replacement: `\U`/`\L`/`\u`/`\l`/`\E` case conversion, `\0` backreference
- Escape sequences: `\c`/`\d`/`\o`/`\x` in replacement and transliterate
- `ctrl_char()` case handling (`\ca` == `\cA`)
- Address types: `0` (pre-first), `+N` (relative), `~N` (multiple), `first~step`
- Address regex modifiers: `/regex/I`, `/regex/M`
- Per-command range state tracking, range end-check on line after start
- `l` command: line wrapping with configurable width, `COLS` env var, `-l N` flag
- `R` command: successive line reads per file
- `r` with address 0: prepend queue (output before first line)
- `-z` null-data mode: NUL-separated records, null-aware `=`/`l`/`F` output
- Trailing newline preservation (no extra `\n` on last line if input didn't have one)
- Branch propagation from `{...}` blocks to parent command list
- `b` without label: normal cycle end with default print (not suppress)
- `D` restart: clear suppress flag for correct `P;D` patterns
- Empty `//` regex in addresses: reuse last regex
- BRE `\` inside `[]` character classes (POSIX literal vs Rust regex escape)
- `fancy-regex` fallback for backreference patterns (`\1` in regex)
- In-place editing: `-i` suffix with `*` expansion, fresh engine per file
- `--follow-symlinks`: resolve symlinks for `F` command and in-place editing
- `--sandbox`: compile-time rejection of `e`/`r`/`w` commands
- POSIX mode (`--posix`): reject GNU extension commands, `s` flags, `l` width, address 0
- POSIX replacement: `\l`/`\u` treated as literal in `--posix` mode
- Error messages: GNU sed format (`-e expression #N, char M: ...`), exit code 1
- Validation: duplicate `p`/`g` flags, missing filenames, unknown `s` options, `:` addresses
- Backreference count validation on `s` command RHS
- `a\`/`i\`/`c\` multi-line text continuation
- Help output matching GNU sed format, `E-mail` line
- Temp file check for in-place editing, FIFO/device detection
- IO error messages: stripped `(os error N)` suffix

---

## Remaining Failures (13 tests)

### Category 1: fancy-regex performance (2 tests)

**Tests:** dc, binary

Complex sed programs (dc calculator, binary operations) use backreference patterns that cause catastrophic backtracking in `fancy-regex`. The programs run correctly with simple inputs but hang on the test inputs.

**Fix:** Optimize regex patterns to avoid backtracking, add per-match timeouts, or implement a custom backtracking-limited NFA for backreferences. Alternatively, switch to a different regex engine.

### Category 2: Byte-mode I/O (3 tests)

**Tests:** 8to7, mac-mf, convert-number

These tests require processing non-UTF-8 binary data. Our engine uses `String` (UTF-8) for pattern/hold spaces. `\d`/`\o`/`\x` escapes in replacement produce Unicode chars instead of raw bytes.

**Fix:** Change engine internals from `String` to `Vec<u8>`. Use `regex::bytes::Regex` instead of `regex::Regex`. This is a fundamental refactor affecting the entire engine.

### Category 3: Error validation (3 tests)

**Tests:** compile-errors, compile-tests, recursive-escape-c

- `compile-errors` — Needs many validation checks: one-address commands (`a`, `i`, `l`, `=`, `q`, `Q`), `#` with addresses, `}` without `{`, `v` version check, `a`/`c`/`i` require `\` in POSIX mode, incomplete `a`/`c`/`i` across `-e` boundaries, `~N`/`+N` as first address, multiple `!`
- `compile-tests` — `s/[[:]]//'` (POSIX class `[:]` parsing edge case)
- `recursive-escape-c` — Error char position off (8 vs 10)

### Category 4: Test-specific issues (3 tests)

**Tests:** execute-tests, bsd-wrapper, unbuffered

- `execute-tests` — All sub-tests pass locally but test exits with code 1 in Nix sandbox (test framework issue with `$fail` variable or subtle `F` command behavior across multiple files)
- `bsd-wrapper` — Test infrastructure expects sed at `../sed/sed` relative path
- `unbuffered` — Needs true line-by-line I/O (currently reads all input into memory)

### Category 5: Edge cases (2 tests)

**Tests:** posix-char-class, obinary

- `posix-char-class` — `\t` inside `[\t]` in `--posix` mode: GNU sed still treats as tab
- `obinary` — Platform-specific (skips on Linux, already passes)

---

## Implementation Priorities

### Priority 1: Byte-mode engine (3 tests)

Switch from `String` to `Vec<u8>` for pattern/hold spaces. Enables 8to7, mac-mf, convert-number. Also improves robustness for all binary input.

### Priority 2: fancy-regex optimization (2 tests)

Add per-match timeouts or optimize patterns. Enables dc, binary.

### Priority 3: Error validation (3 tests)

Many small validation checks for compile-errors. Medium effort, many sub-checks needed.

### Priority 4: Test infrastructure fixes (2 tests)

Fix execute-tests $fail issue, bsd-wrapper path layout.

---

## Test Inventory

### Passing (48 tests)

bsd, bug32082, bug32271-1, bug32271-2, cmd-0r, cmd-l, cmd-R,
colon-with-no-label, command-endings, comment-n, distrib, eval,
follow-symlinks, follow-symlinks-stdin, help, in-place-hyphen,
in-place-suffix-backup, inplace-hold, inplace-selinux (skip), madding,
mb-bad-delim, mb-charclass-non-utf8, mb-match-slash, mb-y-translate,
missing-filename, newline-dfa-bug, normalize-text, nulldata,
panic-tests, posix-mode-addr, posix-mode-bad-ref, posix-mode-ERE,
posix-mode-N, posix-mode-s, range-overlap, regex-errors, regex-max-int,
sandbox, stdin, stdin-prog, subst-mb-incomplete, subst-options,
subst-replacement, temp-file-cleanup, title-case, uniq, word-delim,
xemacs

### Failing (13 tests)

| Test | Primary blocker |
|-|-|
| 8to7 | Byte-mode I/O |
| binary | fancy-regex performance |
| bsd-wrapper | Test infrastructure path |
| compile-errors | Many validation sub-tests |
| compile-tests | `[[:]]` POSIX class edge case |
| convert-number | Byte-mode output |
| dc | fancy-regex performance |
| execute-tests | Test framework $fail issue |
| mac-mf | Byte-mode I/O |
| obinary | Platform skip (already works) |
| posix-char-class | POSIX `\t` semantics |
| recursive-escape-c | Error char position |
| unbuffered | Line-by-line I/O |

### Tests excluded from harness (6 tests)

- `8bit` — Non-UTF-8 binary input file
- `badenc` — Requires specific locale
- `help-version` — Requires exact GNU version strings
- `invalid-mb-seq-UMR` — Requires specific locale
- `newjis` — Requires Japanese shift-JIS locale
- `utf8-ru` — Requires Russian UTF-8 locale
