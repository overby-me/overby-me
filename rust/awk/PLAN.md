# rust-awk: Plan to Pass All Upstream gawk Tests

## Current Status

**186/241 tests passing** (77%) — BASIC_TESTS from the GNU gawk 5.3.2 test suite.

### Remaining failure categories (~77 tests)

- **Error detection (~27)**: Tests expect gawk-compatible error messages we don't produce (scalar/array conflicts, duplicate params, syntax errors, etc.)
- **Array aliasing (~14)**: Need true reference semantics for nested function calls with shared arrays
- **Other (~15)**: CONVFMT caching, operator precedence, ARGV-based file processing, backslash handling, pipe close, etc.
- **Regex (~7)**: Character class edge cases (`---`, `[^]]`), Rust vs POSIX regex differences
- **Getline (~4)**: Complex forms (getline with array subscript side effects, pipe getline with expressions)
- **Printf (~2)**: Comprehensive flag combinations (hsprint), infinity formatting edge cases

### Recent fixes

- Split main.rs into modules: lexer, ast, parser, value, format, interpreter
- Fixed `printf`/`print` with parenthesized args: `printf(fmt, args...)`
- Fixed `%d`/`%i` with precision (`%8.5d`), `%.0d` with zero, `%#o`/`%#x` prefix
- Fixed zero-flag ignored when precision given for integers (POSIX)
- Fixed `%c` to prefer string's first char over numeric conversion
- Fixed array auto-vivification on element read
- OFMT support in `print`, CONVFMT in string concatenation
- Unary plus operator, hex parsing in `parse_num`
- Infinity/NaN formatting in printf `%f`/`%e`/`%g`
- `#` flag for floats (`%#g` keeps trailing zeros, `%#f` forces decimal point)
- Dynamic width with `*` (negative = left-align)
- Fixed regex literal extraction from `Expr::Match` wrapper in function args
- Split `awk_replace` (sub/gsub) from `gensub_replace` (gensub with backrefs)
- Fixed `split()` with regex third argument
- Fixed awk regex semantics: quantifiers after anchors treated as literals
- Regex pattern preprocessing for awk compatibility
- Function name pre-scan for forward references (avoids false "unknown function" on `a (b)` concatenation)
- Range pattern same-line end detection (`/foo/,/bar/` on line containing both)
- Single-char RS splitting (RS != "\n" now reads full input and splits)
- String patterns treated as regex in sub/gsub (not escaped)
- Fix split() anchor edge cases (empty leading/trailing elements from `^`/`$`)
- StrNum value type for input strings (numeric boolean/comparison semantics)
- Fields stored as Value (preserves Str vs StrNum from assignment vs input)
- Bare getline reads from current input stream (pre-read all records)
- Remove hex from parse_num (only in lexer literals per POSIX)
- $0 assignment from string literal preserves Str type for boolean
- Single-char RS splitting reads all input then splits

Tests compare rust-awk output against reference gawk output in a Nix sandbox.

Run a test: `nix build .#checks.x86_64-linux.rust-awk-test-{name}`
View failure diff: `nix log .#checks.x86_64-linux.rust-awk-test-{name}`

---

## Failure Categories

### Category 1: printf/sprintf formatting (20 tests)

`printf` and `sprintf` produce wrong output for many format specifiers.

**Failing tests:** addcomma, convfmt, getnr2tb, getnr2tm, hex, hex2, hsprint, intprec, ofmt, ofmta, ofmtbig, printf-corners, printf1, printfchar, prmarscl, zeroflag, dynlj, strtod, strnum1, uplus

**Issues:**

- `%d` with width/precision (`%8.5d`) not handled — outputs format string literally
- `%o` and `%x` with `#` flag not handled
- `%.0d` with zero precision doesn't suppress zero output
- Zero-flag (`%05d`) not working
- `OFMT` not applied when printing numeric values
- `CONVFMT` not applied during string↔number coercions
- Hex/octal input literals (`0x1F`, `011`) not recognized
- Width formatting in `printf "%20s"` incorrect

### Category 2: gsub/sub/gensub backslash handling (12 tests)

Backslash semantics in substitution functions differ from gawk.

**Failing tests:** backgsub, gsubtest, gsubtst2, gsubtst4, gsubtst5, gsubtst6, gsubtst7, gsubtst8, longsub, subback, subi18n, anchgsub

**Issues:**

- `&` in replacement string should insert matched text — not working for all cases
- `\\` in replacement should produce single `\` — doubling instead
- `gsub` return value (count of replacements) wrong in some cases
- Anchored substitutions (`^`, `$`) not handled correctly
- `gensub` third argument (count) handling incomplete

### Category 3: Missing error detection / diagnostics (21 tests)

gawk rejects invalid programs with error/fatal messages; rust-awk silently accepts them or produces wrong output.

**Failing tests:** aryprm1, aryprm2, aryprm3, aryprm4, aryprm5, aryprm6, aryprm7, arrayparm, badassign1, delfunc, divzero, fnarray, fnarray2, fnaryscl, fnmisc, funsmnam, nfneg, nulinsrc, paramdup, paramres, scalar, sclforin, sclifin

**Issues:**

- Array-scalar conflicts not detected (using array as scalar or vice versa)
- Duplicate parameter names in function definitions not caught
- Function name same as parameter name not caught
- Function name same as builtin not caught
- Division by zero should be fatal — silently returns 0
- Assigning to `NF` with negative value not caught
- NUL bytes in source code not detected
- `$i++ = 3` (assign to post-increment field) not caught as error
- Space between function name and `(` not diagnosed

### Category 4: Missing builtin functions (5 tests)

**Failing tests:** arrayind2, memleak, rebuild, sortempty (need `typeof`, `asort`)

**Issues:**

- `typeof()` — returns type of value as string ("array", "number", "string", "uninitialized")
- `asort()` / `asorti()` — sort array values/indices

### Category 5: Getline bugs (7 tests)

**Failing tests:** getline, getline3, getline5, getlnfa, getnr2tb, getnr2tm, inpref

**Issues:**

- `getline var < file` return value wrong (should be 1 on success, 0 on EOF, -1 on error)
- `getline` from pipe not working correctly
- `getline` with command (`cmd | getline var`) not updating `$0`/fields properly
- NR/FNR not updated correctly after getline
- Getline syntax errors not caught

### Category 6: Record separator (RS) bugs (7 tests)

**Failing tests:** rsnullre, rsnulw, rstest4, rstest5, rswhite, fsnul1, nlfldsep

**Issues:**

- RS as regex not working (RS can be a multi-char regex in gawk)
- RS="" (paragraph mode — split on blank lines) not implemented correctly
- RS="\0" (NUL separator) not handled
- Whitespace-only RS handling wrong

### Category 7: Field separator (FS) bugs (5 tests)

**Failing tests:** fscaret, fsrs, fieldassign, fldchgnf, splitwht

**Issues:**

- FS="^" treated as regex anchor instead of literal caret
- Field assignment (`$3 = "x"`) not rebuilding record correctly
- Changing NF (e.g., `NF = 3`) should truncate/extend fields
- `split()` with single-space FS should trim leading/trailing whitespace

### Category 8: Array/function parameter passing (12 tests)

**Failing tests:** arrayprm3, arrayref, arrymem1, arryref2, arryref3, arryref4, arryref5, aryprm8, callparam, delarpm2, fnasgnm, tailrecurse

**Issues:**

- Arrays not passed by reference to functions correctly
- Local array variables in functions not isolated from global scope
- Deleting array elements inside functions doesn't propagate
- Recursive function calls corrupt local variables
- Function call with undefined function should be a different error

### Category 9: Regex engine bugs (12 tests)

**Failing tests:** back89, negrange, rebrackloc, rebt8b1, regeq, regex3minus, regexpbad, regexpbrack2, regexprange, regrange, reindops, reparse

**Issues:**

- Bracket expressions with special chars (`[^]]`, `[a-d]`) not matching correctly
- Character ranges in bracket expressions off-by-one
- Negated ranges `[^a-z]` matching wrong set
- `\` in regex not handled the same as gawk (gawk treats unknown escapes as literal)
- Multi-byte/UTF-8 characters in bracket expressions
- Interval expressions `{n,m}` in regex

### Category 10: Pipe/close/I/O bugs (5 tests)

**Failing tests:** close_status, clsflnam, status-close, noparms, readbuf

**Issues:**

- `close()` return value not matching gawk (should return exit status of command)
- `close()` on never-opened file should return -1 with message
- Two-way pipes (`|&`) not supported
- Pipe output buffering differences

### Category 11: Parser/syntax error reporting (5 tests)

**Failing tests:** parseme, parse1, parsefld, synerr1, synerr2, synerr3, unterm, badbuild, concat4

**Issues:**

- Syntax errors should produce gawk-compatible messages with line numbers
- Unterminated strings not caught at parse time
- Some valid gawk syntax not parsed (e.g., concatenation edge cases)
- Error recovery after syntax error differs

### Category 12: Miscellaneous bugs (13 tests)

**Failing tests:** concat3, funstack, match4, matchuninitialized, nasty2, nfldstr, numrange, opasnidx, opasnslf, range1, splitwht, strsubscript, substr, swaplns, trailbs, wideidx, wideidx2, widesub, widesub2, widesub4, leadnl, wjposer1, dfamb1

**Issues:**

- `substr()` with length beyond string end should return rest of string, not truncate differently
- `match()` third argument (array) not supported
- Operator associativity/precedence edge cases (`a[i] += 1` vs `a[i++]`)
- Range patterns (`/start/,/stop/`) state management bugs
- Uninitialized variable comparisons
- Multi-byte character handling in string functions
- Trailing backslash at end of line (continuation) not handled
- `delete array` (delete entire array) not working

---

## Implementation Plan

### Phase 1: printf/sprintf engine rewrite

**Impact: ~20 tests**

Replace the printf implementation with a proper format string parser that handles:

- Width, precision, flags (`-`, `+`, `0`, `#`, space)
- All conversion specifiers (`%d`, `%i`, `%o`, `%x`, `%X`, `%e`, `%f`, `%g`, `%s`, `%c`, `%%`)
- `OFMT` for default numeric output, `CONVFMT` for string coercion
- Hex (`0x`) and octal (`0`) input literal recognition
- `*` width/precision from arguments

### Phase 2: gsub/sub/gensub rewrite

**Impact: ~12 tests**

Fix replacement string handling per POSIX + gawk extensions:

- `&` → matched text
- `\&` → literal `&`
- `\\` → literal `\`
- `\n` (in gensub) → n-th capture group
- Anchored patterns (`^`, `$`) in gsub iterate correctly
- Return value = count of replacements

### Phase 3: Error detection and diagnostics

**Impact: ~21 tests**

Add semantic checks in parser and interpreter:

- Array/scalar type conflicts (fatal error)
- Duplicate function parameter names (parse error)
- Function name conflicts with builtins (parse error)
- Division by zero (fatal error)
- Negative NF assignment (fatal error)
- NUL bytes in source (fatal error)
- Invalid lvalue detection (parse error)
- Error messages with file:line format matching gawk

### Phase 4: Array/function parameter semantics

**Impact: ~12 tests**

Fix function call mechanics:

- Pass arrays by reference (share the HashMap)
- Isolate local scalar/array variables per call frame
- Handle recursive calls with proper stack frames
- Delete propagation through references
- Undefined function detection at parse time

### Phase 5: Getline implementation

**Impact: ~7 tests**

Fix all getline forms:

- `getline` — read next record from current input
- `getline var` — read into var instead of `$0`
- `getline < file` — read from file
- `getline var < file`
- `cmd | getline` — read from pipe
- `cmd | getline var`
- Correct return values: 1 (success), 0 (EOF), -1 (error)
- Update NR/FNR appropriately

### Phase 6: Record/field separator fixes

**Impact: ~12 tests**

- RS as multi-char regex
- RS="" paragraph mode (blank-line delimited records)
- FS="^" as literal (not regex anchor)
- Single-space FS trims leading/trailing whitespace
- Field assignment rebuilds `$0` using OFS
- NF assignment truncates/extends field list

### Phase 7: Regex engine fixes

**Impact: ~12 tests**

- Fix bracket expression parsing (`[^]]`, `[a-d-z]`)
- Character range edge cases
- Unknown escape sequences in regex → treat as literal
- Interval expressions (`{n,m}`)
- Multi-byte character support in character classes

### Phase 8: Missing builtins

**Impact: ~5 tests**

- `typeof(x)` → "array", "number", "string", "uninitialized", "regexp"
- `asort(arr)` / `asorti(arr)` — sort by values/indices
- `mktime()`, `systime()`, `strftime()` — verify time functions work

### Phase 9: Pipe/close/I/O

**Impact: ~5 tests**

- `close()` returns child process exit status
- Track open files/pipes for close-of-unopened detection
- Two-way pipes (`|&`)

### Phase 10: Parser and error reporting

**Impact: ~9 tests**

- Syntax errors with gawk-compatible messages (file:line: source ^ error)
- Unterminated string detection
- Error recovery improvements
- Concatenation edge cases

### Phase 11: Remaining fixes

**Impact: ~13 tests**

- `substr()` edge cases (length past end, negative start)
- `match()` third argument (capture groups into array)
- Operator precedence fixes
- Range pattern state machine
- Line continuation (trailing `\`)
- `delete array` (entire array)
- Multi-byte string function support

---

## Test Inventory

### Passing (135 tests)

anchor, arrayind3, arrayprm2, arynasty, aryprm9, aryprm8, arysubnm, arrymem1, asgext,
assignnumfield, assignnumfield2, back89, childin, closebad, compare2, concat1, concat2,
concat5, datanonl, delarprm, dfacheck2, dfastress, divzero2, dynlj, eofsplit, exit2,
exitval2, exitval3, fcall_exit, fcall_exit2, fldchg, fldchgnf, fldterm, fordel, forref,
forsimp, fsbs, fscaret, fsrs, fstabplus, funsemnl, getline4, getnr2tb, gsubnulli18n,
gsubtst2, gsubtst4, gsubtst6, gsubtst8, hex2, inputred, intest, intprec, iobug1,
leaddig, leadnl, manglprm, match4, matchuninitialized, math, membug1, minusstr, mmap8k,
nasty, nasty2, negexp, negrange, nested, nfloop, nfset, nlinstr, nlstrina, noloop1,
noloop2, nulrsend, numindex, numstr1, numsubstr, octsub, ofmt, ofmtbig, ofmtfidl, ofmts,
ofmtstrnum, ofs1, onlynl, paramtyp, paramuninitglobal, pcntplus, prdupval, prec, printf1,
printfchar, prmreuse, prt1eval, prtoeval, range2, regeq, reindops, reparse, resplit,
rri1, rs, rsnul1nl, rstest1, rstest2, rstest3, rswhite, setrec0, setrec1, sigpipe1,
sortglos, splitargv, splitarr, splitdef, splitvar, splitwht, splitwht2, strcat1,
strfieldnum, strnum1, strnum2, subamp, subback, subi18n, subsepnm, subslash, substr,
uparrfs, uplus, wideidx, wideidx2, widesub, widesub2, widesub3, wjposer1, zero2, zeroe0,
zeroflag

### Failing (106 tests)

arrayind1, arrayind2, arrayparm, arrayprm3, arrayref, arryref2, arryref3, arryref4,
arryref5, aryprm1, aryprm2, aryprm3, aryprm4, aryprm5, aryprm6, aryprm7, aryprm8,
aryunasgn, back89, backgsub, badassign1, badbuild, callparam, close_status, clsflnam,
concat3, concat4, convfmt, delargv, delarpm2, delfunc, dfamb1, divzero, fieldassign,
fnamedat, fnarray, fnarray2, fnaryscl, fnasgnm, fnmisc, fsnul1, funsmnam, funstack,
getline, getline3, getline5, getlnfa, getnr2tm, gsubasgn, gsubnulli18n, gsubtest,
gsubtst5, gsubtst7, hex, hex2, hsprint, inpref, longsub, memleak, nfldstr, nfneg,
nlfldsep, noparms, nulinsrc, numrange, ofmta, opasnidx, opasnslf, paramdup, paramres,
parse1, parsefld, parseme, printf-corners, prmarscl, rand, range1, readbuf, rebrackloc,
rebt8b1, rebuild, regex3minus, regexpbad, regexprange, regrange, rsnullre, rsnulw,
rstest4, rstest5, scalar, sclforin, sclifin, sortempty, splitwht2, status-close,
strsubscript, strtod, swaplns, synerr1, synerr2, synerr3, tailrecurse, trailbs, unterm,
wideidx, widesub4
