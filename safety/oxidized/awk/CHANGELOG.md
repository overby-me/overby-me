# Changelog

All notable changes to rust-awk.

## [Unreleased]

### Test suite compatibility

Passes 242/242 of the upstream GNU gawk 5.3.2 BASIC_TESTS. Up from
104/242 at the start of tracked development.

### Encoding and I/O

- Byte-preserving I/O via Latin-1 mapping. Input bytes 0–255 decode to
  Unicode chars U+0000–U+00FF through `bytes_to_string`; output via
  `write_awk` emits those chars as single bytes and higher code points
  as UTF-8. Round-trips non-UTF-8 bytes through the interpreter and
  matches gawk's C-locale byte semantics (rebt8b1, gsubnulli18n,
  trailbs, getnr2tm).
- `print`/`printf` to stdout, files, append files, and pipes all go
  through `write_awk` for consistent byte output.
- Input reader and `-f` source reader use `bytes_to_string` so binary
  input and non-UTF-8 source files (e.g. ISO-8859-encoded scripts in
  the gawk test suite) parse.

### Random numbers

- Port of gawk's bundled BSD `random()` (`support/random.c`) as
  `GawkRng`: TYPE_4 variant with 63-element LFSR state, Park–Miller
  seed-expand via `good_rand`, 512-slot shuffle buffer. Produces
  bit-identical output to gawk for any `srand(seed)` (rand).

### Lexer

- `/=` disambiguation via `slash_eq_looks_like_regex`. When the
  previous two tokens are both value-like (ident/string/regex/number/
  close paren/bracket) *and* a closing `/` appears before any
  statement terminator, the lexer emits a Regex with body starting at
  `=`. Otherwise emits `SlashAssign`. Mirrors gawk's `want_regexp`
  parser-feedback scheme narrowly enough to parse `print $re c /= d/`
  as concatenation of three values while preserving ordinary
  `a /= b` compound assignment (parsefld).
- Binary-operator tokens (`+`, `-`, `*`, `%`) included in the
  can-be-regex context so `print /a/ + /b/` sums regex-match results.
- Regex literal bracket-class state: `/` inside `[...]` doesn't
  terminate the regex; a leading `]` (or `]` after `^`) stays literal
  (regexpbrack).

### Parser

- `getline $target` absorbs one post-inc/dec into the target
  (`$ non_post_simp_exp opt_incdec` production). `parse_postfix` lets
  a Getline result be wrapped in at most one PostInc/PostDec. A
  further `++`/`--` in concat position errors immediately with the
  caret at the offending token (getlnfa).
- Lvalue check for field post-increment assignment (`$i++ = 3`)
  reports the caret at the end of the rhs expression (badassign1).
- `Token::Regex` triggers concatenation in `parse_concatenation` so
  `c /regex/` (regex after a value) produces a Concat.
- Function-name pre-scan for forward references (avoids false
  "unknown function" on `a (b)` concatenation).
- Adjacent-paren function calls for non-declared functions
  (callparam).
- For-loop empty-body semicolon handling (forsimp).
- `$` field-ref with unary/prefix operators (`$+i++`).
- Pre-execution scan for function-as-gsub-target errors (gsubasgn).
- Compile-time div-by-zero check (divzero).
- `$` postfix binding with per-level `++` (parse1).

### Regex compilation pipeline

- `expand_hex_escapes` pre-expands `\xHH` to literal characters before
  bracket-class parsing, so `[^[]\x5b` becomes `[^[][` — an
  unbalanced class that gawk (and now rust-awk) rejects at compile
  (regexpbad).
- Octal escape `\ddd` (1–3 octal digits) converts to `\x{HEX}` so
  Rust's regex crate accepts it (`[\300-\337]` works — range2).
- `[` inside a class is auto-escaped except when introducing a POSIX
  named class like `[[:upper:]]` (rebrackloc).
- Leading `]` in a class becomes `\]` so Rust doesn't read `[]` as
  empty (POSIX literal-first).
- Leading `-` followed by `-X` emits `\x{2D}` so the range survives
  Rust's parse (regrange).
- POSIX single-char collating elements `[.c.]` rewrite to `c`.

### Record and field handling

- RS splitting tracks the matched separator per record and populates
  `RT`. Shared `split_by_rs()` between `process_stream` and pipe
  getline (`pipe_records: HashMap<String, PipeRecordState>` cache
  keyed by command string).
- Paragraph mode (`RS = ""`): skips leading newlines, uses `\n\n+`
  regex, keeps trailing newline as the last record's `RT` (rsnullre,
  rsnulw, rsnul1nl; no regression on swaplns).
- Zero-width regex RS (e.g. `RS = "()"`) treated as no-split — whole
  input becomes one record with `RT = ""`.
- Single-char RS splitting reads full input then splits.
- Range pattern same-line end detection (`/foo/,/bar/` on a line
  containing both).
- Always rebuild `$0` on field assignment (fscaret).

### Arrays and function parameters

- Per-frame alias map (`array_aliases: HashMap<String, String>`) on
  the interpreter. `resolve_array_name()` runs before every
  `self.arrays.*` access. Call setup builds aliases for Var args,
  flattens chains through the caller's aliases, skips self-aliases,
  and for scalar params whose name coincides with a canonical alias
  target it skips the `arrays[name]` save/remove so the caller's
  array isn't orphaned. Multiple function parameters sharing the same
  caller array now see each other's writes; uninitialized vars
  passed by reference promote to arrays (aryprm8, arryref2 and
  siblings).
- Scalar/array conflict detection with provenance chains through
  aliased parameters (aryprm1–7, arryref3–5, fnaryscl, arrayparm).
- Array auto-vivification on element read.

### `sub` / `gsub` / `gensub`

- String patterns treated as regex (not escaped).
- Backslash handling: `\\` at end preserved, `\\&` → `\` + matched
  (backgsub, subback).
- `split()` anchor edge cases: empty leading/trailing elements from
  `^`/`$` removed.
- Split `awk_replace` (sub/gsub) from `gensub_replace` (backrefs).

### Getline

- `cmd | getline` honors non-default RS by caching the full pipe
  output on first access and splitting by the current RS. Subsequent
  getlines advance through cached records so paragraph-mode reads see
  every paragraph (rstest5).
- Bare getline reads from the current input stream via pre-read
  records.

### Printf / sprintf

- All standard specifiers: `%d`/`%i`/`%o`/`%x`/`%X`/`%e`/`%E`/`%f`/
  `%F`/`%g`/`%G`/`%s`/`%c`/`%%`.
- Width, precision, flags (`-`, `+`, `0`, `#`, space).
- Dynamic width `*` (negative = left-align).
- `%s` width uses character count (not bytes) so U+FFFD pads the
  same visual width as gawk's byte count on pre-decode bytes.
- `%c` prefers first char of a string over numeric conversion.
- `%.0d` with zero precision suppresses zero output; zero-flag
  ignored when precision is given for integers (POSIX).
- Infinity/NaN formatting; `%#g` keeps trailing zeros, `%#f` forces
  decimal point.
- `OFMT` for numeric `print`; `CONVFMT` for string concatenation.
- Parenthesized `printf(fmt, args...)`.
- Unary plus, hex literal parsing in `parse_num`, hex parsing removed
  from `parse_num` (lexer-only per POSIX).

### Errors and diagnostics

- Runtime regex errors for dynamic patterns use gawk's
  `(FILENAME=- FNR=1) fatal: invalid regexp: Trailing backslash:
  /regex/` format, written via `write_awk` (trailbs).
- Syntax errors with source line and caret position (parseme,
  noparms, synerr1–3).
- Function-as-array/variable detection (delfunc, fnarray, fnarray2,
  fnamedat, fnasgnm, gsubasgn target).
- Function-as-variable error with space between name and `(`.
- Compile-time division-by-zero.
- `NF` assignment with negative value fatal.
- Scalar parameter error message.

### Values

- `StrNum` type for input strings and fields: numeric
  boolean/comparison semantics.
- Fields stored as Value (preserves Str vs StrNum from assignment vs
  input).
- `$0` assignment from string literal preserves Str type for
  boolean.
