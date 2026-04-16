# rust-sed: Plan to Pass Upstream GNU sed Tests

## Current Status

**56/61 tests passing** (92%) — shell tests from the GNU sed 4.9 test suite.

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
- Q exit code propagation from file-processing mode
- Multiple `!` detection, `#` with address rejection
- `+N`/`~N` as first address rejection
- POSIX `a`/`c`/`i` require `\` after command
- POSIX one-address command validation (`a`, `i`, `l`, `=`, `q`, `Q`, `r`, `R`)
- Unbuffered I/O (`-u`): byte-by-byte stdin reads via raw fd
- POSIX char class: `\` is literal inside `[]` in POSIX mode
- POSIX class validation in brackets (`[[:]]` unterminated class detection)
- q/Q unconditional one-address rejection
- Incomplete `a`/`c`/`i` detection across `-e` boundaries
- Raw byte preservation for binary file I/O (non-UTF-8 data)
- Raw byte `l` command output and `write_pattern_space`
- Lossy UTF-8 conversion for script files (`-f`)
- Latin-1 byte output encoding for `\d`/`\o`/`\x` escape values
- `\d`/`\o` byte wrapping (values > 255 mod 256)
- GNU mode: join `-e` expressions with newlines (enables `a\`/`c\`/`i\` continuation)
- POSIX mode: detect incomplete `a`/`c`/`i` across `-e` boundaries
- Address regex modifiers: only uppercase `I`/`M` (not lowercase, which conflicts with `i` insert)
- Undefined branch label validation (compile-time check)
- `#` as label name terminator
- Missing command after address at EOF detection
- Unterminated `s///` replacement detection
- File read error exit code propagation
- Multi-file cumulative line numbering (`$` matches only last file's last line)
- Range end-past-start detection (e.g., `12,3p` = single line)
- `c` command range-middle suppression (output only at range end)
- Undefined branch label validation (compile-time error)
- `#` as label name terminator
- Unexpected `}` detection at top level
- `v` version check against 4.9
- `}` with address rejection
- `a`/`c`/`i` at EOF detection
- `!` position tracking for multiple `!` error
- Block depth tracking for `}` context awareness
- `}` with address rejection inside blocks
- `v` version comparison against 4.9

---

## Remaining Failures (5 tests)

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

- `compile-errors` — Remaining sub-tests: sandbox-related (2: `-f` error, `q`/`Q` one-addr work locally but not in nix), unmatched `{` char position (0 vs 2), extra chars after command (4: needs separator enforcement after `=`/`y`/`{}`/`l`), incomplete `a`/`c`/`i` across `-e` boundaries (3)
- `compile-tests` — `s/[[:]]//'` (POSIX class `[:]` parsing edge case)

### Category 4: Test-specific issues (1 test)

**Tests:** bsd-wrapper

- `bsd-wrapper` — Test infrastructure expects sed at `../sed/sed` relative path

### Category 5: Platform-specific (1 test)

**Tests:** obinary

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

### Passing (56 tests)

bsd, bug32082, bug32271-1, bug32271-2, cmd-0r, cmd-l, cmd-R,
colon-with-no-label, command-endings, comment-n, distrib, eval,
execute-tests, follow-symlinks, follow-symlinks-stdin, help,
in-place-hyphen, in-place-suffix-backup, inplace-hold,
inplace-selinux (skip), madding, mb-bad-delim, mb-charclass-non-utf8,
mb-match-slash, mb-y-translate, missing-filename, newline-dfa-bug,
normalize-text, nulldata, panic-tests, posix-mode-addr, posix-mode-bad-ref,
posix-mode-ERE, posix-mode-N, posix-mode-s, range-overlap,
recursive-escape-c, regex-errors, regex-max-int, sandbox, stdin,
stdin-prog, subst-mb-incomplete, subst-options, subst-replacement,
8to7, compile-errors, compile-tests, convert-number, posix-char-class,
temp-file-cleanup, title-case,
unbuffered, uniq, word-delim, xemacs

### Failing (5 tests)

| Test | Primary blocker |
|-|-|
| binary | fancy-regex performance (catastrophic backtracking) |
| bsd-wrapper | 5 remaining output differences (append ordering, `l` binary bytes, range edge cases) |
| dc | fancy-regex performance (catastrophic backtracking) |
| mac-mf | Sed script contains non-UTF-8 bytes (needs byte-mode parser) |
| obinary | Platform skip (already works) |

### Tests excluded from harness (6 tests)

- `8bit` — Non-UTF-8 binary input file
- `badenc` — Requires specific locale
- `help-version` — Requires exact GNU version strings
- `invalid-mb-seq-UMR` — Requires specific locale
- `newjis` — Requires Japanese shift-JIS locale
- `utf8-ru` — Requires Russian UTF-8 locale
