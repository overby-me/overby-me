# rust-perl: Plan to Pass Upstream Perl Tests

## Goal

Rewrite Perl in Rust, verified against the upstream Perl 5 test suite (`t/` directory from the perl source tarball).

## Current Status

**9/68 Nix tests passing** (13%) — selected tests from the upstream Perl test suite.

Passing: base/if, base/cond, base/while, base/pat, base/num (56 tests),
base/translate (257 tests), base/term (7 tests), cmd/elsif (4 tests),
cmd/mod (15 tests).

Near-passing (local test counts):

- opbasic/arith: 174/183 (integer overflow edge cases)
- opbasic/concat: 228/254 (Unicode concat)
- opbasic/qq: 15/30 (\\o{} octal escapes)
- cmd/for: 14/16 (Internals::stack_refcounted)
- cmd/subval: 19/36 (caller, wantarray, file I/O)

test.pl integration working: plan/ok/is/printf produce TAP output.

Tests compare rust-perl output against reference perl output in a Nix sandbox.

Run a test: `nix build .#checks.x86_64-linux.rust-perl-test-{category}-{name}`
View failure diff: `nix log .#checks.x86_64-linux.rust-perl-test-{category}-{name}`

### Recent fixes

- Lexer, parser, AST, and tree-walking interpreter for Perl 5
- Scalar variables, arrays, hashes, string interpolation
- if/else/elsif, unless, while/until, for/foreach, do-while
- Subroutines with my/local, implicit return (last expression value)
- Regex matching, range operator (..), ternary operator
- Number formatting (%.15g), binary/octal/hex literals
- File I/O (open/close/readline), backtick command execution
- String operators (eq/ne/lt/gt/le/ge/cmp), numeric comparison
- Logical operators (&&/||/!/not/and/or//), bitwise operators
- Postfix modifiers (if/unless/while/until/for), die/warn with postfix
- BEGIN/END blocks, eval string, &func() call syntax
- Variable scoping fix: non-my variables default to global scope
- s/// substitution with g/i flags
- require for loading Perl files, %INC tracking
- map/grep/sort { BLOCK } LIST parsing
- Hash-vs-block disambiguation with look-ahead scan
- Function args expand @arrays in list context
- return accepts postfix if/unless modifiers
- All builtin keywords in expects_operand for regex-after-keyword

---

## Test Suite Strategy

The upstream Perl test suite lives in `t/` within the perl source tarball. Tests produce TAP (Test Anything Protocol) output. We compare our output against reference perl.

### Test tiers (in implementation order)

| Tier | Directory | Tests | Description |
|------|-----------|-------|-------------|
| 1 | `t/base/` | 9 | Absolute basics: if, while, lexer, numbers, patterns, record separator, terms, tr. Raw `print "ok/not ok"` — no test libraries. |
| 2 | `t/opbasic/` | 5 | Core operators that `t/test.pl` itself depends on: arithmetic, comparison, concatenation, qq. |
| 3 | `t/cmd/` | 5 | Control flow: for, elsif, statement modifiers, subroutine return values, switch. |
| 4 | `t/op/` (selected) | 40 | Operators and builtins: arrays, hashes, strings, math, eval, closures, references, sort, split, sprintf, regex ops, etc. |
| 5 | `t/io/` (selected) | 6 | I/O: open, read, print, argv, filesystem, tell/seek. |
| 6 | `t/re/` (selected) | 3 | Regular expressions: pattern matching, substitution. |

Total tracked: **68 tests** (expandable as the interpreter matures).

---

## Architecture

### Module plan

```text
src/
  main.rs          CLI argument parsing, script loading, entry point
  lexer.rs         Tokenization of Perl source
  parser.rs        Recursive-descent parser → AST
  ast.rs           AST node definitions
  interpreter.rs   Tree-walking execution engine
  value.rs         Perl value types (scalar, array, hash, reference, undef)
  regex.rs         Perl regex engine interface (m//, s///, =~)
  io.rs            Filehandle management, open/close/read/print
  builtins.rs      Built-in functions (chomp, split, join, sprintf, etc.)
  context.rs       Scalar/list context propagation
```

### Value system

Perl values are fundamentally different from awk. Key types:

- **Scalar**: string, number, or reference (with dual string/number nature)
- **Array**: ordered list of scalars (`@arr`)
- **Hash**: key-value map of scalars (`%hash`)
- **Reference**: pointer to any value (`\$x`, `\@arr`, `\%hash`, `\&sub`, anonymous constructors)
- **Undef**: uninitialized value
- **Filehandle**: I/O handle (STDIN, STDOUT, STDERR, user-opened)

Scalars have the "dual-var" property: a scalar can be both a string and a number simultaneously, with conversion on demand (like awk's StrNum but more pervasive).

### Scoping

Perl has three scoping mechanisms that must all work:

- **`my`**: lexical scope (block-scoped, visible in nested blocks/closures)
- **`local`**: dynamic scope (temporarily overrides a package global for the duration of the call stack)
- **Package globals**: `$Foo::bar` or `$main::var`, accessible anywhere

### Context

Every expression in Perl evaluates in either scalar or list context. This affects return values:

- `@arr` in scalar context → length
- `localtime()` in list context → 9-element list; in scalar context → formatted string
- Subroutines can check with `wantarray()`

---

## Implementation Phases

### Phase 0: Scaffolding (target: `t/base/if`)

Get the most trivial test passing. `t/base/if.t` tests `if`/`else` with `eq`/`ne` and simple `print`.

**Required features:**

- Lexer: string literals, barewords, operators (`eq`, `ne`), semicolons, braces, parens
- Parser: `print` statement, `if`/`else`, string comparison
- Interpreter: execute print, evaluate string equality
- CLI: `-e` flag, script file execution

### Phase 1: Base tier (`t/base/*` — 9 tests)

**Features needed for all of `t/base/`:**

- **`if.t`**: `if`/`else`, `eq`/`ne`
- **`cond.t`**: `&&`, `||`, `==`, `!=`, conditional expressions
- **`while.t`**: `while` loops, `last`, `next`, `redo`, loop labels
- **`term.t`**: basic terms — variables (`$x`), array access (`$a[0]`), hash access (`$h{k}`), string literals (single/double-quoted), numeric literals, list construction, `qw//`
- **`num.t`**: number stringification, binary/octal/hex/float/scientific literals, `inf`/`nan`
- **`lex.t`**: string interpolation (`"$var"`, `"${var}"`), heredocs (`<<EOF`), special variables (`$_`, `$/`, `$\`, `$,`), POD (`=head1`...`=cut`), comments
- **`pat.t`**: basic regex matching (`=~`, `!~`, `m//`), captures (`$1`, `$2`), match modifiers (`/i`, `/g`, `/m`, `/s`)
- **`rs.t`**: record separator (`$/`), `<>` (readline) behavior with different `$/` values
- **`translate.t`**: `tr///` / `y///` transliteration operator

### Phase 2: Opbasic tier (`t/opbasic/*` — 5 tests)

- **`arith.t`**: integer and floating-point arithmetic, overflow, underflow
- **`cmp.t`**: `<=>`, `cmp`, chained comparisons
- **`concat.t`**: `.` operator, `.=` assignment, stringification
- **`qq.t`**: `qq{}`, `q{}`, `qw{}` quoting operators, interpolation in `qq`
- **`magic_phase.t`**: `${^GLOBAL_PHASE}` — BEGIN/CHECK/INIT/RUN/END phase tracking

### Phase 3: Control flow (`t/cmd/*` — 5 tests)

- **`elsif.t`**: `elsif` chains
- **`for.t`**: C-style `for`, `foreach`, `for my $x (@list)`, loop variable aliasing
- **`mod.t`**: statement modifiers (`if`, `unless`, `while`, `until`, `for`, `foreach` as postfix)
- **`subval.t`**: subroutine return values, `return`, `wantarray`
- **`switch.t`**: `given`/`when` (if tested) or the smartmatch-based switch

### Phase 4: Core operators (`t/op/*` — 40 tests)

This is the largest phase. Key clusters:

**Data structures:**

- `array.t`: push/pop/shift/unshift, splice, slices, $#arr, wantarray
- `hash.t`: keys/values/each/exists/delete, hash slices, hash in boolean context
- `list.t`: list assignment, list in scalar context
- `ref.t`: references, dereferencing, `ref()`, anonymous constructors `[]`/`{}`/`sub{}`

**String operations:**

- `chop.t` / `chr.t` / `ord.t`: character manipulation
- `substr.t` / `index.t`: substring extraction and search
- `join.t` / `split.t`: string joining and splitting
- `sprintf.t`: format strings (similar to awk but with Perl extensions)
- `lc.t` / `quotemeta.t`: case conversion, regex quoting
- `length.t`: string/array length
- `heredoc.t`: heredoc variations (indented, interpolated, etc.)
- `tr.t`: transliteration (more thorough than `t/base/translate.t`)

**Numeric operations:**

- `arith2.t`: extended arithmetic tests
- `auto.t`: `++`/`--` auto-increment (including magical string increment `"aa"`→`"ab"`)
- `bop.t`: bitwise operators (`&`, `|`, `^`, `~`, `<<`, `>>`)
- `inc.t`: increment edge cases
- `oct.t`: `oct()` function, `hex()` function
- `range.t`: `..` range operator (list context: generates list; scalar context: flip-flop)
- `repeat.t`: `x` repeat operator (`"ab" x 3`, list repeat)
- `vec.t`: `vec()` bit-vector operations

**Control & evaluation:**

- `cond.t`: ternary `?:`, short-circuit `&&`/`||`/`//`
- `eval.t`: `eval BLOCK`, `eval STRING`, `$@` error variable
- `die.t`: `die`, `warn`, exception objects
- `closure.t`: lexical closures, closure over loop variables
- `context.t`: scalar/list context propagation
- `do.t`: `do BLOCK`, `do FILE`
- `grep.t`: `grep`, `map`
- `local.t` / `my.t`: dynamic vs lexical scoping
- `sort.t`: `sort`, custom comparison, Schwartzian transform
- `wantarray.t`: `wantarray()` detection

**Misc:**

- `defined.t` / `undef.t`: `defined()`, `undef`
- `delete.t`: `delete` on arrays/hashes
- `not.t`: `not`, `!`, `unless`
- `pack.t`: `pack`/`unpack` (binary data)
- `pos.t`: `pos()` for regex position tracking
- `print.t`: `print`, `say`, output to filehandles
- `push.t` / `splice.t` / `unshift.t`: array mutation
- `sub.t`: subroutine definitions, prototypes, anonymous subs

### Phase 5: I/O (`t/io/*` — 6 tests)

- `open.t`: `open()` modes (read/write/append/pipe), 3-arg open, `open my $fh`
- `print.t`: `print`, `printf`, `say`, output to filehandles
- `read.t`: `read()`, `sysread()`, buffered I/O
- `argv.t`: `@ARGV`, `<>`, `-` as stdin
- `fs.t`: filesystem operations (`-e`, `-f`, `-d`, `stat`, `rename`, `unlink`, `mkdir`)
- `tell.t`: `tell()`, `seek()`, file position

### Phase 6: Regex (`t/re/*` — 3 tests)

- `pat.t`: comprehensive pattern matching (character classes, anchors, quantifiers, alternation, grouping, backreferences, lookahead/lookbehind)
- `regexp.t`: regex engine edge cases, special patterns
- `subst.t`: `s///` substitution with all modifiers (`/g`, `/e`, `/r`, `/i`, `/m`, `/s`, `/x`)

---

## Key Differences from rust-awk

| Aspect | rust-awk | rust-perl |
|--------|----------|-----------|
| Value types | Str, Num, StrNum, Uninitialized | Scalar (dual string/number), Array, Hash, Reference, Undef, Filehandle |
| Scoping | Global + function-local | Lexical (`my`), dynamic (`local`), package globals |
| Context | N/A | Scalar vs list context everywhere |
| Regex | awk-style `/pattern/` | Full Perl regex (backrefs, lookahead, `(?:...)`, modifiers, `$1`...) |
| Data structures | Arrays (associative only) | Arrays (ordered), Hashes (associative), References, nested structures |
| OOP | N/A | `bless`, `->` method calls, `@ISA` inheritance |
| Closures | N/A | Full lexical closures |
| String eval | N/A | `eval STRING` — compile and execute at runtime |
| I/O | Simple print/getline/pipes | Filehandles, 3-arg open, layers, binmode, formats |
| Modules | N/A | `use`/`require`, `@INC`, `%INC`, `BEGIN`/`END` blocks |
| Test format | Output comparison (diff) | TAP output comparison (diff) |

---

## Milestones

| Milestone | Tests passing | Description |
|-----------|---------------|-------------|
| M0 | 1/68 | First test (`base/if`) passes |
| M1 | 9/68 (13%) | All `t/base/` tests pass — fundamental language works |
| M2 | 14/68 (21%) | `t/base/` + `t/opbasic/` — core operators work |
| M3 | 19/68 (28%) | + `t/cmd/` — control flow complete |
| M4 | 40/68 (59%) | + selected `t/op/` — bulk of language features |
| M5 | 59/68 (87%) | + remaining `t/op/` — operators comprehensive |
| M6 | 65/68 (96%) | + `t/io/` — I/O works |
| M7 | 68/68 (100%) | + `t/re/` — regex complete for tracked tests |
| M8 | expand | Add more `t/op/`, `t/comp/`, `t/uni/`, `t/run/` tests |

---

## Test Inventory

### Tracked tests (68)

**base (9):** cond, if, lex, num, pat, rs, term, translate, while

**opbasic (5):** arith, cmp, concat, magic_phase, qq

**cmd (5):** elsif, for, mod, subval, switch

**op (40):** arith2, array, auto, bop, chop, chr, closure, cond, context, defined, delete, die, do, each, eval, grep, hash, heredoc, inc, index, join, lc, length, list, local, my, not, oct, ord, pack, pos, print, push, quotemeta, range, ref, repeat, reverse, sort, splice, split, sprintf, sub, substr, tr, undef, unshift, vec, wantarray

**io (6):** argv, fs, open, print, read, tell

**re (3):** pat, regexp, subst

### Passing (9)

base/cond, base/if, base/num, base/pat, base/term, base/translate, base/while,
cmd/elsif, cmd/mod

### Failing (59)

base/lex, base/rs, cmd/for, cmd/subval, cmd/switch,
opbasic/arith, opbasic/cmp, opbasic/concat, opbasic/magic_phase, opbasic/qq,
op/arith2, op/array, op/auto, op/bop, op/chop, op/chr, op/closure, op/cond,
op/context, op/defined, op/delete, op/die, op/do, op/each, op/eval, op/grep,
op/hash, op/heredoc, op/inc, op/index, op/join, op/lc, op/length, op/list,
op/local, op/my, op/not, op/oct, op/ord, op/pack, op/pos, op/print, op/push,
op/quotemeta, op/range, op/ref, op/repeat, op/reverse, op/sort, op/splice,
op/split, op/sprintf, op/sub, op/substr, op/tr, op/undef, op/unshift, op/vec,
op/wantarray, io/argv, io/fs, io/open, io/print, io/read, io/tell,
re/pat, re/regexp, re/subst, run/exit, run/switches
