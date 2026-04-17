# rust-awk

A GNU awk-compatible text-processing tool written in Rust.

## Status

**242/242 tests passing (100%)** — BASIC_TESTS from the upstream GNU gawk
5.3.2 test suite. Each test runs both rust-awk and the reference gawk
against the same script/input in a sandbox and diffs the output
byte-for-byte.

## Usage

Run a single upstream test:

```sh
nix build .#checks.x86_64-linux.rust-awk-test-{name}
```

View a failing test's log:

```sh
nix log .#checks.x86_64-linux.rust-awk-test-{name}
```

Batch-run every test in a single evaluator (much faster than looping):

```sh
nix build .#checks.x86_64-linux.rust-awk-test-* --keep-going --no-link
```

The binary is available as `awk` from `pkgs.rust-awk` (release build) or
`pkgs.rust-awk-dev` (debug build, faster compile). It installs `gawk` as
an alias.

## Architecture

Seven source modules:

- `ast` — expression and statement node types.
- `lexer` — tokenizer with regex-literal bracket-class state and
  context-sensitive `/=` disambiguation.
- `parser` — expression/statement parser with gawk-compatible operator
  precedence and getline grammar.
- `value` — `Value` enum (`Num`, `Str`, `StrNum`, `Uninitialized`) with
  comparison and coercion rules.
- `format` — printf/sprintf engine: width, precision, flags, all
  conversion specifiers, `OFMT`/`CONVFMT`, char-count `%s` width.
- `interpreter` — execution engine: rules, statements, builtins, I/O,
  array-aliasing call frames, record splitting, BSD random port.
- `main` — argv parsing, source loading, pipeline wiring.

## Features

### Expression semantics

- Full expression precedence matching gawk, including `?:`, `in`,
  match/not-match, concatenation-by-juxtaposition, pre/post-inc on
  fields.
- `StrNum` value type for input strings and fields: numeric comparison
  when both sides are numeric-looking, string comparison otherwise.
- `CONVFMT`/`OFMT` applied during string coercion and `print`.
- `$` field-ref with unary/prefix operators (`$+i++`, `$-x`).

### Arrays

- Reference semantics via per-frame alias map. Multiple function
  parameters sharing the same caller array all see each other's
  writes. Uninitialized variables passed by reference promote to
  arrays in the caller when the callee uses them as arrays.
- Scalar/array conflict detection with provenance chains — when a
  function parameter is used as scalar while a sibling alias uses it
  as array, the error names the origin variable.
- `asort`/`asorti`, `delete array`, `delete array[key]`, `in`
  membership, for-in iteration with deterministic ordering.

### Regex

- awk-style literal lexing: `/` inside `[...]` doesn't terminate; a
  leading `]` (or `]` after `^`) stays literal; `/=` ambiguous-case
  look-ahead picks regex vs compound-assign based on surrounding
  tokens.
- Compatibility fixups for the Rust regex crate: octal `\ddd` → hex,
  `\xHH` pre-expansion, `[` inside a class auto-escaped (unless
  introducing `[[:class:]]`), POSIX collating `[.c.]` rewritten,
  leading-`-` range preserved via hex, quantifier-after-anchor treated
  as literal.
- POSIX named character classes (`[:alpha:]`, etc.).
- Used by `~`/`!~`, `match`, `sub`/`gsub`/`gensub`, `split`, `FS`/`RS`.

### Record and field handling

- `RS` as literal, single char, empty (paragraph mode), or multi-char
  regex. Zero-width regex match treated as no-split.
- `RT` populated with the actual matched separator per record.
- `FS` as single char, literal, or regex; `FS=" "` default whitespace
  mode.
- `NF` read/write rebuilds `$0` via `OFS`.
- Paragraph mode skips leading newlines and preserves trailing
  newlines as the last record's `RT`.

### Input / output

- **Byte-preserving I/O.** Input decodes bytes 0–255 to Unicode chars
  U+0000–U+00FF so non-UTF-8 data round-trips exactly. Output writes
  chars U+0000–U+00FF as single bytes and higher code points as UTF-8,
  matching gawk's C-locale byte-level semantics in the test sandbox.
- `getline` forms: `getline`, `getline var`, `getline < file`,
  `getline var < file`, `cmd | getline`, `cmd | getline var`. Pipe
  reads honor the current `RS` by caching the full output and
  splitting on first access, so paragraph-mode pipe reads see every
  paragraph.
- `print` / `printf` with redirection (`>`, `>>`, `|`).
- `close()`, `system()`, pipe lifecycle tied to child processes.

### printf / sprintf

- All standard specifiers: `%d`, `%i`, `%o`, `%x`, `%X`, `%e`, `%E`,
  `%f`, `%F`, `%g`, `%G`, `%s`, `%c`, `%%`.
- Width, precision, flags (`-`, `+`, `0`, `#`, space), dynamic `*`
  width/precision.
- `%s` width uses character count (not byte count) so U+FFFD from
  lossy-decoded input pads to the same visual width as gawk's byte
  count on the pre-decode bytes.
- `%c` prefers first char of a string argument over numeric
  conversion.
- Infinity/NaN formatting, `%#g` keeping trailing zeros, `%#f` forcing
  decimal point.

### Random numbers

- `srand()`/`rand()` use a port of gawk's bundled BSD `random()`
  (`support/random.c`): TYPE_4 with 63-element LFSR, Park–Miller
  seed-expand, 512-slot shuffle buffer. Produces bit-identical output
  to gawk for any seed.

### Errors and diagnostics

- gawk-format syntax errors with source line and caret position.
- Runtime regex errors for dynamic patterns include
  `(FILENAME=.. FNR=..) fatal: invalid regexp: Trailing backslash:
  /regex/` wording, matching gawk's per-record reporting.
- Lvalue checks: assignment to post-increment field (`$i++ = 3`) is
  rejected with the caret at the end of the rhs expression.
- Array-scalar conflicts, function-as-variable misuse, space between
  function name and `(`, `NF = negative`, `NUL` in source code,
  compile-time division by zero.

### gawk-specific grammar corners

- Adjacent-paren function calls (`callparam`).
- `getline $target++` — `opt_incdec` absorbed into the getline target,
  matching the `$ non_post_simp_exp opt_incdec` production.
- `print a /regex/` concatenation with a regex literal right after an
  identifier or other value-like token.
- `$/= b/` parses as `$(regex)` thanks to the `/=` look-ahead.
