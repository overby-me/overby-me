# rust-grep: Plan to Pass All Upstream GNU grep Tests

## Current Status

**100/119 tests passing** (84%) — from the GNU grep 3.12 test suite.

### Remaining failure categories (~19 tests)

- **BRE/ERE regex edge cases (~3)**: Spencer test failures (literal `*` at start, bad interval expressions)
- **PCRE edge cases (~4)**: `-P` with `-w` on non-word patterns, `-P` with `-z`, PCRE backtrack limit, PCRE context with `-z -o`
- **I/O edge cases (~3)**: Input-equals-output detection, max-count stdin overread, write error on `/dev/full`
- **Locale/encoding (~2)**: `c-locale` high-byte matching, `high-bit-range` binary panics
- **Color/env (~1)**: `GREP_COLORS` / `GREP_COLOR` environment variable support
- **Pattern edge cases (~2)**: `null-byte` NUL in patterns, `posix-bracket` collating elements
- **Misc (~4)**: Stack overflow handling, `include-exclude` complex glob patterns, `backref` invalid bracket cross-pattern, `warn-char-classes` misuse warnings

### Recent fixes

- Fixed exit codes: return 2 on file errors, separate `-s` (--no-messages) from `-q`
- Fixed `-f` pattern file: include empty lines, handle `-f /dev/null`, fix arg parsing bug
- Added `--color=always` with ANSI escape highlighting
- Added `-w` with `-F` word boundary checking for fixed strings
- Added `-NUM` context shorthand (e.g., `-3` for `-C 3`)
- Prefer longest match at each position for multiple `-e` patterns
- Added backreference support via automatic `fancy-regex` fallback
- Cross-pattern backref validation (exit 2 for invalid refs across `-e` patterns)
- Improved BRE-to-ERE conversion: `^`/`$` anchor position, literal `|`, bracket expressions
- Added binary file detection ("binary file matches" message)
- Added `-r` default to current directory when no files given
- Added `--exclude-dir` option for recursive search
- Split `-e` patterns on newlines per GNU grep semantics
- Added `-T` (--initial-tab) option
- Added `-a` (--text) and `--binary-files=text` options
- Implemented `-D skip` (--devices=skip) to skip device files
- Fixed empty pattern with `-w` and `-x` (match only empty lines)
- Split `-e` patterns on newlines
- Fixed `-x` line matching for regex mode
- Formatted regex error messages to match GNU grep style

Tests compare rust-grep output against the GNU grep test suite's expected behavior in a Nix sandbox.

Run a test: `nix build .#checks.x86_64-linux.rust-grep-test-{name}`
View failure diff: `nix log .#checks.x86_64-linux.rust-grep-test-{name}`

---

## Failure Categories

### Category 1: Backreference support (5 tests)

The Rust `regex` crate does not support backreferences (`\1`, `\2`, etc.). BRE and ERE backrefs need to fall back to `fancy-regex`.

**Failing tests:** backref, backref-word, triple-backref, case-fold-backref, pcre-wx-backref

**Issues:**

- `\(.\)\1` (BRE backreference) — not supported by regex crate
- Multiple `-e` patterns with backrefs scoped per-pattern (backref locality)
- Case-folded backreferences
- PCRE backrefs with `-w` and `-x`

### Category 2: Color / highlight output (3 tests)

`--color=always` is parsed but no ANSI escape codes are emitted around matches.

**Failing tests:** color-colors, foad1 (color portions), fedora (partially)

**Issues:**

- Need to emit `\033[01;31m\033[K` before match and `\033[m\033[K` after
- Color with `-o`, `-n`, `-b`, `-H` combinations
- Color with `-i` should highlight the original case of matched text
- Color with `-F -w` needs word boundary detection in output
- `GREP_COLORS` environment variable support

### Category 3: Context and group separators (3 tests)

Context output (`-A`, `-B`, `-C`) missing group separator `--` between non-contiguous groups.

**Failing tests:** context-0, max-count-vs-context, count-newline (partially)

**Issues:**

- `-C 0` should still print group separator `--` between non-adjacent matches
- Context interaction with `-m` (max count) — should print trailing context after last match
- Context with `-c` (count) mode

### Category 4: Pattern file and empty pattern handling (4 tests)

`-f FILE` edge cases and empty pattern semantics.

**Failing tests:** empty, count-newline, khadafy, file

**Issues:**

- `-f /dev/null` should mean "no patterns" → match nothing (exit 1)
- Currently `-f` skips empty lines, but should include them as empty patterns
- `-e '' -f /dev/null` should still match (empty pattern matches all)
- `-f` with files containing only newlines

### Category 5: BRE/ERE regex edge cases (7 tests)

Spencer conformance tests and regex edge cases that differ between Rust regex and POSIX.

**Failing tests:** bre, ere, spencer1, inconsistent-range, reversed-range-endpoints, high-bit-range, posix-bracket

**Issues:**

- Interval expressions in BRE (`\{n,m\}`) edge cases
- Character class and bracket expression handling
- Reversed range endpoints (`[z-a]`) — GNU grep treats as error or empty
- High-bit characters in ranges
- POSIX bracket expression compat (`[[:alpha:]]` edge cases)

### Category 6: PCRE edge cases (5 tests)

Perl-compatible regex mode (`-P`) via `fancy-regex` has behavioral differences from GNU grep's PCRE2.

**Failing tests:** pcre, pcre-abort, pcre-count, pcre-w, pcre-context

**Issues:**

- `-P` with `-w` (word boundary) wrapping differs
- `-P` pattern that should abort/error on pathological input
- `-P` with `-c` (count) interaction
- PCRE empty match handling
- PCRE with context lines

### Category 7: Missing command-line options (3 tests)

Options that are parsed but not implemented, or not parsed at all.

**Failing tests:** skip-device, initial-tab, fedora (partially)

**Issues:**

- `-D skip` / `--devices=skip` — skip device files (not implemented)
- `-T` / `--initial-tab` — align output with tab after prefix
- `--binary-files=TYPE` — control binary file handling

### Category 8: Exit code / status bugs (2 tests)

Exit codes don't match GNU grep in all cases.

**Failing tests:** status, fedora (partially)

**Issues:**

- `grep -q` with invalid file should still exit 0 if match found in other files
- `grep -l` should exit 0 after first matching file even if later files have errors
- Exit code 2 for errors not consistent

### Category 9: I/O and system edge cases (5 tests)

**Failing tests:** in-eq-out-infloop, max-count-overread, binary-file-matches, write-error-msg, warn-char-classes

**Issues:**

- Input-is-output detection (prevent infinite loop when `grep pattern file > file`)
- `-m` with multiple files should not overread (stop reading after count reached)
- Binary file detection and "Binary file matches" message
- Write error handling and error message format
- Character class warnings for invalid constructs

### Category 10: Fixed-string matching bugs (2 tests)

**Failing tests:** fgrep-longest, null-byte

**Issues:**

- `-F` with multiple patterns should find the longest match at each position
- NUL byte handling in fixed-string mode
- `-z` (null-data) interaction with `-F`

### Category 11: Locale and encoding (2 tests)

**Failing tests:** c-locale, reversed-range-endpoints

**Issues:**

- Character matching in C locale with high bytes
- Range endpoint handling varies by locale

### Category 12: Miscellaneous (4 tests)

**Failing tests:** stack-overflow, include-exclude, r-dot, fedora (partially)

**Issues:**

- Stack overflow on deeply nested regex — should print error, not crash
- `--include` / `--exclude` glob patterns in recursive mode don't match GNU behavior
- Recursive grep with `.` (current directory) handling
- Various Fedora-reported bugs (combination of multiple issues)

---

## Implementation Plan

### Phase 1: Color output

**Impact: ~3 tests** (color-colors, foad1, fedora)

Implement `--color=always` ANSI escape highlighting:

- Emit `\033[01;31m\033[K` / `\033[m\033[K` around matches
- Handle `-o`, `-n`, `-b`, `-H` with color
- Support `GREP_COLORS` environment variable
- Preserve original case in match highlight with `-i`

### Phase 2: Backreference support

**Impact: ~5 tests** (backref, backref-word, triple-backref, case-fold-backref, pcre-wx-backref)

Fall back to `fancy-regex` when BRE/ERE patterns contain backreferences:

- Detect `\1`..`\9` in BRE patterns after conversion
- Use `fancy-regex` instead of `regex` crate for those patterns
- Ensure backref scope is per `-e` pattern (cross-pattern backrefs = error)
- Handle case-folded backrefs with `-i`

### Phase 3: Pattern file (`-f`) and empty pattern fixes

**Impact: ~4 tests** (empty, count-newline, khadafy, file)

- `-f /dev/null` → zero patterns → no matches (exit 1)
- Include empty lines from `-f` as valid empty patterns
- Fix `-f` combined with `-e` precedence
- Fix pattern file reading from special files

### Phase 4: Context output fixes

**Impact: ~3 tests** (context-0, max-count-vs-context, count-newline)

- Print `--` separator between non-contiguous context groups
- `-C 0` should still use group separators
- Fix context interaction with `-m` (max-count)

### Phase 5: BRE/ERE regex conformance

**Impact: ~7 tests** (bre, ere, spencer1, inconsistent-range, reversed-range-endpoints, high-bit-range, posix-bracket)

- Fix Spencer test edge cases (interval expressions, bracket expressions)
- Handle reversed range endpoints as errors
- High-bit character range matching
- POSIX character class edge cases

### Phase 6: Missing options

**Impact: ~3 tests** (skip-device, initial-tab, fedora)

- Implement `-D skip` / `--devices=skip`
- Implement `-T` / `--initial-tab`
- Implement `--binary-files=TYPE`

### Phase 7: PCRE fixes

**Impact: ~5 tests** (pcre, pcre-abort, pcre-count, pcre-w, pcre-context)

- Fix `-P` with `-w` word boundary wrapping
- Handle PCRE pathological pattern errors gracefully
- PCRE with `-c` count mode
- PCRE empty match semantics

### Phase 8: Exit code / status fixes

**Impact: ~2 tests** (status, fedora)

- Fix exit code when `-q` with mix of valid/invalid files
- Fix `-l` early exit behavior
- Consistent exit 2 for errors

### Phase 9: I/O and system handling

**Impact: ~5 tests** (in-eq-out-infloop, max-count-overread, binary-file-matches, write-error-msg, warn-char-classes)

- Detect input-is-output (compare inodes)
- Fix `-m` overread across multiple files
- Binary file detection ("Binary file X matches")
- Proper write error handling

### Phase 10: Fixed-string and remaining fixes

**Impact: ~4 tests** (fgrep-longest, null-byte, stack-overflow, include-exclude, r-dot)

- `-F` longest match selection
- NUL byte handling
- Stack overflow graceful error
- Recursive glob matching fixes

---

## Test Inventory

### Passing (100 tests)

100k-entries, backref-alt, backref-multibyte-slow, backref-word, backslash-dot,
backslash-s-and-repetition-operators, backslash-s-vs-invalid-multibyte, big-hole, big-match,
binary-file-matches, bogus-wctob, case-fold-backref, case-fold-backslash-w,
case-fold-char-class, case-fold-char-range, case-fold-char-type, case-fold-titlecase,
char-class-multibyte, char-class-multibyte2, context-0, count-newline, dfa-coverage,
dfa-heap-overrun, dfa-infloop, dfa-invalid-utf8, dfaexec-multibyte, empty, empty-line,
empty-line-mb, encoding-error, epipe, equiv-classes, euc-mb, false-match-mb-non-utf8,
fedora, fgrep-infloop, fgrep-longest, file, fillbuf-long-line, fmbtest, foad1,
grep-dev-null, grep-dev-null-out, grep-dir, hangul-syllable, hash-collision-perf,
inconsistent-range, initial-tab, invalid-multibyte-infloop, khadafy, kwset-abuse,
long-pattern-perf, many-regex-performance, match-lines, max-count-vs-context,
mb-dot-newline, mb-non-UTF8-overrun, mb-non-UTF8-word-boundary, multibyte-white-space,
multiple-begin-or-end-line, no-perl, options, pcre-ascii-digits, pcre-count,
pcre-infloop, pcre-invalid-utf8-infloop, pcre-invalid-utf8-input, pcre-jitstack, pcre-o,
pcre-utf8, pcre-utf8-bug224, pcre-utf8-w, pcre-wx-backref, pcre-z, prefix-of-multibyte,
proc, r-dot, repetition-overflow, reversed-range-endpoints, sjis-mb, skip-device, status, surrogate-pair,
surrogate-search, triple-backref, turkish-eyes, turkish-I, turkish-I-without-dot,
two-chars, two-files, unibyte-binary, unibyte-bracket-expr, unibyte-negated-circumflex,
utf8-bracket, version-pcre, word-delim-multibyte, word-multi-file, word-multibyte,
y2038-vs-32-bit, z-anchor-newline

### Failing (19 tests)

backref, bre, c-locale, color-colors, ere, high-bit-range,
in-eq-out-infloop, include-exclude, max-count-overread, null-byte, pcre,
pcre-abort, pcre-context, pcre-w, posix-bracket,
spencer1, stack-overflow, warn-char-classes, write-error-msg

### Not yet tested (5 tests)

symlink, skip-read, filename-lineno.pl, help-version, envvar-check

These tests require special system features (symlinks in sandbox, Perl test framework, `--help`/`--version` output matching) or timed out during initial testing.
