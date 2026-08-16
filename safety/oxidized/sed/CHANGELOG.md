# Changelog

All notable changes to rust-sed.

## [Unreleased]

### Test suite compatibility

Passes 61/61 of the upstream GNU sed 4.9 shell test suite.

### Regex engine

- `.` matches any character including `\n` inside pattern space
  (GNU sed behavior).
- BRE: `^` is an anchor only at the very start of the pattern, and `$`
  only at the very end; both are literal characters elsewhere (POSIX
  BRE rule). Fixes dc.sed's multiplication loop, which relies on `^`
  being literal.
- `fancy-regex` fallback engine for patterns with backreferences.

### Parser

- UTF-8-aware literal pushes: multi-byte characters are preserved as
  single Unicode code points (rather than split into individual Latin-1
  bytes).
- Script files: UTF-8 when valid, Latin-1 byte-preserving fallback so
  raw bytes (`\xc4`, …) survive the round trip through parser and
  engine.

### Encoding

- Raw-byte preservation for non-UTF-8 input data.
- Latin-1 output encoding reproduces the original bytes from `\d`/`\o`/
  `\x` escapes.
- `\d`/`\o` values > 255 wrap modulo 256.

### Engine

- `l` wrapping keeps multi-byte escape sequences (e.g. `\035`) atomic
  across line breaks.
- `N`/`n` flush the append queue before reading the next line (GNU sed
  compat).
- Branch into nested-block labels: `b <label>` where `:label` lives
  inside `{...}` enters the block at the label and resumes after the
  block on normal return.
- Range past-end detection: line-number/relative ranges close without
  matching when a branch has skipped past the end-line.
- `D` clears the suppress flag so `P;D` patterns print correctly.
- Pre-print queue for `r` with address `0` (emit file contents before
  the first line).

### Commands

- All standard commands and GNU extensions: `s`, `y`, `a`/`i`/`c`,
  `d`/`D`, `p`/`P`, `n`/`N`, `g`/`G`/`h`/`H`/`x`, `b`/`t`/`T`, `r`/`R`,
  `w`/`W`, `l`, `=`, `q`/`Q`, `:label`, `{...}`, `e`, `F`, `v`, `Q`.
- `a\`/`i\`/`c\` multi-line text continuation (including across `-e`
  boundaries in GNU mode).
- `#n` quiet mode and inline `#` comments.
- `v` version check against 4.9.

### Substitution

- Flags: `g`, `p`, `N` (nth match), `i`/`I`, `m`/`M`, `e`, `w file`.
- Replacement: `\0`–`\9` backreferences, `&`, `\U`/`\L`/`\u`/`\l`/`\E`
  case conversion, `\n`/`\t`/`\\` escapes, `\c`/`\d`/`\o`/`\x`.
- Duplicate-flag detection, backreference count validation.

### Addresses

- `N`, `$`, `0` (pre-first), `+N`, `~N`, `first~step`.
- `/regex/` with `I`/`M` modifiers; empty `//` reuses the last regex.
- Two-address ranges with correct end-check semantics.
- `!` negation with duplicate-`!` diagnostics.

### I/O

- In-place editing: `-i[SUFFIX]`, `*` expansion, backup file creation,
  fresh engine per file.
- `--follow-symlinks` resolves symlinks for `F` and in-place editing.
- FIFO/device detection for in-place targets.
- `-u` unbuffered mode: byte-by-byte stdin reads via raw fd.
- `-z` null-data mode: NUL-separated records, null-aware output for
  `=`/`l`/`F`.
- `l` command with configurable width (`-l N`, `COLS` env var).
- Trailing-newline preservation (no spurious `\n` if input had none).
- IO error messages strip the `(os error N)` suffix.
- `Q` exit-code propagation from file-processing mode.
- File read error exit-code propagation.
- Multi-file cumulative line numbering (`$` matches only the last
  file's last line).

### Validation

- GNU sed error format: `sed: -e expression #N, char M: ...` with
  exit code 1.
- Compile-time detection of: undefined branch labels, unterminated
  `s///`, unterminated `a`/`c`/`i` text blocks (including across `-e`
  boundaries), unmatched `}`, `}` with an address, `+N`/`~N` as a
  first address, missing command after address at EOF, duplicate `s`
  flags, multiple `!`, `#` with an address, unknown `s` options,
  one-address-only violations in POSIX mode, block-depth tracking for
  `}` context awareness.
- Address regex modifiers: only uppercase `I`/`M` (lowercase conflicts
  with `i` insert command).
- `c` range-middle suppression (output only at range end).
- Range end-past-start detection (e.g. `12,3p` = single line).
- `q`/`Q` unconditional one-address rejection.
- POSIX char class: `\` is literal inside `[]`; `[[:]]` unterminated-
  class detection.
- POSIX `a`/`c`/`i` require `\` after command.
- POSIX replacement: `\l`/`\u` treated as literal in `--posix`.
- GNU mode: `-e` expressions joined with newlines (enables `a\`/`c\`/
  `i\` continuation).
- POSIX mode: incomplete `a`/`c`/`i` detection across `-e` boundaries.

### Modes

- `--posix`: rejects GNU extensions (extra commands, `s` flags, `l`
  width, address `0`, case conversion).
- `--sandbox`: compile-time rejection of `e`, `r`, `w`.
- `-E`/`-r`: extended regex syntax.
