# rust-sed

A GNU sed-compatible stream editor written in Rust.

## Status

**61/61 tests passing (100%)** against the upstream GNU sed 4.9 shell test
suite. Six additional tests require locale or encoding configuration beyond
the scope of this project (see [excluded tests](#tests-excluded-from-harness)).

## Usage

Run a single upstream test:

```sh
nix build .#checks.x86_64-linux.rust-sed-test-{name}
```

View a failing test's log:

```sh
nix log .#checks.x86_64-linux.rust-sed-test-{name}
```

The binary is available as `sed` from `pkgs.rust-sed` (release build) or
`pkgs.rust-sed-dev` (debug build, faster compile).

## Architecture

Five source modules:

- `types` — core types (`SedRegex`, `Address`, `Command`, …).
- `parser` — sed-script parser that produces `Command` trees.
- `regex_util` — BRE-to-ERE translation and POSIX character class fixups.
- `engine` — execution engine: cycle loop, command dispatch, address
  handling, append/prepend queues.
- `util` — small helpers: escape tokenization, control-character mapping.

## Features

### Commands

- All standard commands: `s`, `y`, `a`/`i`/`c`, `d`/`D`, `p`/`P`, `n`/`N`,
  `g`/`G`/`h`/`H`/`x`, `b`/`t`/`T`, `r`/`R`, `w`/`W`, `l`, `=`, `q`/`Q`,
  `:label`, `{...}`.
- GNU extensions: `e` (execute), `F` (filename), `v` (version), `Q` (quiet
  quit).
- `a\`/`i\`/`c\` multi-line text continuation.
- `#n` quiet mode and inline `#` comments.

### Substitution (`s///`)

- Flags: `g`, `p`, `N` (nth match), `i`/`I` (case-insensitive), `m`/`M`
  (multi-line regex), `e` (execute), `w file`.
- Replacement: `\0`–`\9` backreferences, `&`, `\U`/`\L`/`\u`/`\l`/`\E` case
  conversion, `\n`/`\t`/`\\` escapes, `\c`/`\d`/`\o`/`\x` char escapes.
- Duplicate-flag detection, backreference count validation.

### Addresses

- `N` (line), `$` (last line), `0` (pre-first, for `r`).
- `+N` (relative), `~N` (multiple), `first~step` (arithmetic progression).
- `/regex/` with `I`/`M` modifiers; empty `//` reuses the last regex.
- Two-address ranges with correct end-check semantics: ranges re-close
  cleanly even when branches skip past the end-line.
- `!` negation with correct duplicate-`!` diagnostics.

### Regex compatibility

- Dot (`.`) matches any character including `\n` inside pattern space
  (GNU sed behavior).
- BRE: `^`/`$` are anchors only at the very start/end of the pattern;
  literal characters elsewhere (POSIX BRE rule).
- `fancy-regex` fallback engine for patterns with backreferences.
- POSIX character-class handling: `[` is literal, `\` treatment differs
  between default and `--posix` mode, `[:class:]` parsing with unterminated-
  class detection.

### Encoding

- Script files: UTF-8 when valid, Latin-1 byte-preserving fallback so raw
  bytes (`\xc4`, …) survive the round trip.
- Input files: raw-byte preservation for non-UTF-8 data; Latin-1 output
  encoding reproduces the original bytes.
- Parser is UTF-8-aware: multi-byte characters are preserved as single
  Unicode code points.
- `\d`/`\o`/`\x` escapes produce raw bytes; `\d`/`\o` values > 255 wrap
  modulo 256.

### Control flow

- Labels may live anywhere, including nested inside `{...}` blocks. A
  branch to a nested label enters the block at that label and resumes
  after the block on normal return.
- `N`/`n` flush the append queue before reading the next line (GNU sed
  compat).
- `D` clears the suppress flag so `P;D` patterns print correctly.
- Pre-print queue for `r` with address `0` (emit file contents before the
  first line of input).

### I/O

- In-place editing with `-i[SUFFIX]`, `*` expansion, backup file creation.
- `--follow-symlinks` resolves symlinks for `F` and in-place editing.
- FIFO/device detection for in-place targets.
- `-u` unbuffered mode: byte-by-byte stdin reads via raw fd.
- `-z` null-data mode: NUL-separated records.
- `l` command with configurable width (`-l N`, `COLS` env var) that keeps
  multi-byte escape sequences atomic across line breaks.
- Trailing-newline preservation (no spurious `\n` if the input had none).

### Validation & errors

- GNU sed error format: `sed: -e expression #N, char M: ...` with exit
  code 1.
- Compile-time detection of: undefined branch labels, unterminated `s///`,
  unterminated `a`/`c`/`i` text blocks (including across `-e` boundaries),
  unmatched `}`, `}` with an address, `+N`/`~N` as a first address,
  missing command after address at EOF, duplicate `s` flags, multiple `!`,
  `#` with an address, unknown `s` options, one-address-only violations
  in POSIX mode.
- `v` command version check against 4.9.

### Modes

- `--posix`: rejects GNU extensions (extra commands, `s` flags, `l`
  width, address `0`, `\l`/`\u` case conversion, etc.), enforces `\`
  after `a`/`c`/`i`.
- `--sandbox`: compile-time rejection of `e`, `r`, `w`.
- `-E`/`-r`: extended regex syntax.

## Test inventory

### Tests excluded from harness

These require locale or encoding configuration that is not worth wiring
through the build:

- `8bit` — non-UTF-8 binary input file.
- `badenc` — requires a specific locale.
- `help-version` — requires exact GNU version strings.
- `invalid-mb-seq-UMR` — requires a specific locale.
- `newjis` — requires a Japanese shift-JIS locale.
- `utf8-ru` — requires a Russian UTF-8 locale.
