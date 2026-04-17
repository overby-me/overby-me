# rust-perl: Plan to Pass Upstream Perl Tests

## Goal

Rewrite Perl in Rust, verified against the upstream Perl 5 test suite (`t/` directory from the perl source tarball).

## Current Status

**37/79 Nix tests passing** (47%) — selected tests from the upstream Perl test suite.

Passing: base/if, base/cond, base/while, base/pat, base/num, base/translate,
base/term, cmd/elsif, cmd/mod, cmd/switch, opbasic/arith, opbasic/qq,
op/arith2, op/auto, op/bop, op/chop, op/closure, op/cond, op/defined,
op/do, op/hash, op/inc, op/index, op/lc, op/oct, op/pack, op/quotemeta,
op/range, op/split, op/sprintf, op/sub, op/vec, io/argv, io/fs, io/open,
re/subst, run/switches.

Major unlock in this cycle: `@_` is now dynamically scoped per-call
(was being written to globals and overwritten on every sub call — this
broke every test that used test.pl's `like()`/`is()` chains). Also
added runtime interpolation of `$var`/`@var` inside `/regex/` patterns,
hash slice (`@h{k1,k2}`) delete, a `splice()` implementation, and a
`DynaLoader::boot_DynaLoader` stub so `is_miniperl()` returns false
(matching reference perl).

Near-passing (local test counts):

- opbasic/concat: 230/254 (Unicode concat)
- cmd/for: 15/16 (DESTROY method)
- cmd/subval: 34/36 (typeglob local aliasing)
- op/my: 47/59 (`my $i` scope leaks between conditionals and loops)
- op/array: 102/195 (nested refs, typeglob coerce)
- op/not: 19/22 (typeglob assignment to read-only scalar)
- op/grep: 39/77 (nested references in list context)
- op/list: 38/73 (chained list assignment: `@a = @b = (1,2)`)
- op/delete: 28/56
- op/splice: 9 new passes (splice added this cycle)
- op/repeat: 3 new passes (`x=` added this cycle)
- op/oct: 79/79 (FIXED — `@_` aliasing repaired)

test.pl integration fully working: plan/ok/is/pass/note/printf produce correct
TAP output with test names. Function calls in argument lists fixed.

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
- `undef EXPR` actually clears the lvalue (was a no-op)
- Float-path modulo operator for values outside i64 range
- `my ($x) = @_` treats RHS in list context (single-var list destructure)
- `return` inside `do { }` now propagates out of enclosing sub
- Magical string increment on `++`: e.g. `"aa" → "ab"`, `"zz" → "aaa"`,
  `"a9" → "b0"`, with case-preserving carry into a new leading letter.
- Prototype-`$` builtins (scalar, defined, ref, lc, uc, chop, chomp, int, abs,
  sqrt, chr, ord, hex, oct, etc.) take exactly one arg when called without
  parens, fixing `is(scalar @arr, N, $name)` parsing.
- `defined(EXPR)` parses EXPR as a full expression when parens are present,
  so `defined($x ? $y : @z)` no longer drops the ternary branches.
- `scalar(a, b, c)` treats the comma as Perl's list operator: evaluates all
  args, returns the value of the last (matches `scalar((a, b, c))`).
- `qr//` produces a regex *value* (string `(?^flags:pat)`), distinct from
  bare `/pat/` which matches against `$_`. `$str =~ $rx_var` now works.
- `eval EXPR if COND` honours the postfix modifier — previously dropped,
  which made `eval '...' if !defined &re::is_regexp;` in test.pl swallow
  every subsequent statement (hiding `is`, `cmp_ok`, etc. from the suite).
- Line-number tracking: lexer records a line per token, parser emits
  `Stmt::LineMark(N)`, interpreter maintains `current_line` + a
  `call_stack` of frames. `caller(N)` now returns the real call-site
  file/line — test.pl's `_where()` output now matches reference perl.
- `(a => 1, b => 2)` and `[a => 1]` recognise `=>` as a list separator.
  Previously hash initialisers collapsed to a single element.
- `keys %h` / `values %h` return the actual keys/values in list context.
- `f(), last unless COND` now gates `f()` AND `last` under the postfix
  modifier (was running `f()` regardless).
- Stubs: `Internals::stack_refcounted()`, minimal `pack`/`unpack` for
  `W*`, `U*`, `C*` formats (enough for test.pl's `display()` helper).
- Compile-time `use MODULE` check — absent modules emit Perl's exact
  `Can't locate MODULE.pm …` / `BEGIN failed--compilation aborted`
  before any run-time output, matching reference perl under the sandbox.
- Bareword `require Module`; inside `eval {}`, module-load failures
  propagate as die into `$@` instead of printing.
- `Stmt::Begin(body, end_line)` records the closing-`}` line for the
  BEGIN-failed diagnostic.
- Heredoc vs shift disambiguated by the next character (quote/tilde/alpha)
  so `print $fh <<'END'` is read as a heredoc.
- `last LABEL` propagates out of sub calls; parens-less dispatch accepts
  `eval`, `qr`, deref tokens; `map({BLOCK} LIST)` parses BLOCK as code.
- `Value::Regex(pat, flags)` so `ref()` on `qr//` returns `"Regexp"`;
  thread-spawn with 256 MiB stack for deeply recursive tests.
- `__FILE__` / `__LINE__` / `__PACKAGE__`, `$::name` shorthand.
- `main::sub` resolution strips the prefix for subs defined in main.
- Typeglobs: `*NAME` produces `Value::Glob`; `local(*F) = *G` aliases the
  local filehandle slot to the source name via `fh_aliases`; all IO ops
  route through `resolve_fh()` so the alias stays transparent.
- `$#ary = N` is an lvalue: truncates or extends the array with `undef`.
- Named-sub hoisting: every `sub NAME { ... }` (nested in blocks, loops,
  conditionals, or sub bodies) is registered in `self.subs` at
  compile-time. Forward calls to subs textually defined later work.
- `package NAME;` inside a block reverts on block exit.
- `$main::foo` and `$foo` (in `main`) share the same storage slot.
- `continue { BLOCK }` on `while`/`until`/`foreach` and on bare blocks
  (one-shot loop) — implements cmd/switch and op/my loop tests.
- Basic references: `Value::ArrayRef` / `HashRef` / `ScalarRef` /
  `CodeRef` backed by `Rc<RefCell<...>>`. `\@arr`, `\%h`, `\$x`, `[...]`,
  `{...}` produce real refs. `@$ref`, `%$ref`, `$$ref` and their braced
  forms dereference. `$ref->[i]` and `$ref->{k}` via arrow. `ref()`
  returns the type.
- `oct()` / `hex()` accept underscores and all Perl prefix variants.
  `0_2_5 === 025`. `"0"x10` (x-repeat without spaces) now parses.
- `use MODULE` that isn't a pragma emits reference perl's exact
  `Can't locate MODULE.pm in @INC ...` / `BEGIN failed--compilation
  aborted` error and exits. `-I` populates `@INC`.

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

### Passing (34)

base/cond, base/if, base/num, base/pat, base/term, base/translate, base/while,
cmd/elsif, cmd/mod, cmd/switch, opbasic/arith, opbasic/qq, op/arith2,
op/auto, op/bop, op/chop, op/closure, op/defined, op/do, op/hash, op/inc,
op/index, op/lc, op/pack, op/quotemeta, op/range, op/split, op/sprintf,
op/sub, op/vec, io/fs, io/open, re/subst, run/switches

### Failing (34)

base/lex, base/rs, cmd/for, cmd/subval, cmd/switch,
opbasic/cmp, opbasic/concat, opbasic/magic_phase,
op/arith2, op/array, op/auto, op/bop, op/chop, op/chr, op/closure, op/cond,
op/context, op/delete, op/die, op/do, op/each, op/eval, op/grep,
op/hash, op/heredoc, op/inc, op/index, op/join, op/lc, op/length, op/list,
op/local, op/my, op/not, op/oct, op/ord, op/pack, op/pos, op/print, op/push,
op/quotemeta, op/range, op/ref, op/repeat, op/reverse, op/sort, op/splice,
op/split, op/sprintf, op/sub, op/substr, op/tr, op/undef, op/unshift, op/vec,
op/wantarray, io/argv, io/fs, io/open, io/print, io/read, io/tell,
re/pat, re/regexp, re/subst, run/exit, run/switches

### Next high-impact targets

The biggest locked-door preventing many tests from passing is:

1. **Real references.** Currently `\$x`, `[...]`, `{...}` return the placeholder
   strings `"REF"`, `"ARRAY_REF"`, `"HASH_REF"`. Dereferences (`@$ref`, `%$ref`,
   `$$ref`, `$ref->[i]`, `$ref->{k}`) all silently give empty. This blocks
   op/ref, op/hash, op/array (nested), op/auto (glob handling), and more —
   any test that iterates over array-refs.
2. **`qr//` regex values.** Currently `qr/pat/` is not stored as a usable
   regex value; `$str =~ $rx` crashes silently. Blocks cmd/for test 13 and
   many regex tests.
3. **Line-number tracking in tokens.** With line info we could emit
   reference-perl-compatible `Can't locate Config.pm in @INC ... at FILE.t
   line NN.` errors, which would instantly pass ~15 tests whose reference
   output is just that error (op/arith2, op/bop, op/chop, op/inc, op/lc,
   op/pack, op/quotemeta, op/range, op/sprintf, op/vec, io/fs, io/open,
   re/pat, re/subst, run/switches).
4. **Typeglobs (`*F`, `*yes = \x`)** — blocks cmd/subval tests 31-36 and
   parts of op/auto, op/not.
