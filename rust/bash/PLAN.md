# Bash Test Suite — Continuation Notes

## Current State

**64/77 tests passing** on bookmark `push-nkqwvorqmnkn`. All changes committed and pushed.

### Progress This Session

- **Started at**: 64/77 (arith diff 30, heredoc diff 111, comsub-posix diff 20)
- **Now at**: 64/77 (same count — sub-tests still block nix pass)
- **arith**: main test 0 diff ✅ (was 30), sub-tests still have ~100 lines diff
- **heredoc**: main test ~20 diff locally (was 111), no longer hangs in nix, sub-tests ~85 diff
- **comsub-posix**: 0 diff locally ✅ (was 20), still fails in nix due to error message sub-tests
- **posixexp**: 6 diff locally (was 7), nix still fails on IFS/$@ issues
- **trap**: flaky — 1 extra CHLD signal (non-deterministic)
- **printf**: flaky — timing-dependent date format mismatch

### Fixes Applied This Session

1. **Fix arithmetic short-circuit assignment errors** (`src/interpreter/arithmetic.rs`) — `0 && B=42` and `1 || B=88` now correctly error with "attempted assignment to non-variable". Added general non-variable detection for simple `=` after `&&`/`||`. Added ternary `?:` awareness: `1 ? 20 : x+=2` errors (LHS is `(1 ? 20 : x)`) but `0 ? x+=2 : 20` works (inside then-branch). Skip compound `+=`/`-=` when preceded by same char (e.g. `x++=7` is `x++ =7`, not `x +=...`).

2. **Fix `--x++` lvalue error** (`src/interpreter/arithmetic.rs`) — Pre-decrement/increment followed by post-increment/decrement now errors with "assignment requires lvalue". Detects `name.ends_with("++")` or `name.ends_with("--")` in `++`/`--` prefix handlers.

3. **Fix backtick command substitution in arithmetic** (`src/interpreter/arithmetic.rs`) — Added `` `...` `` handling in `expand_comsubs_in_arith`. Also trigger expansion when expr contains `` ` `` (not just `$`). Handles `\`` escaping inside backticks.

4. **Fix arithmetic error token trailing spaces** (`src/interpreter/arithmetic.rs`) — When inner expression is a suffix of top expression, use top-expression suffix to preserve trailing whitespace (bash includes trailing space in error tokens like `$iv`).

5. **Fix arithmetic error abort in assignments** (`src/interpreter/commands.rs`) — After `execute_assignment`, check `take_arith_error()` and return status 1. Fixes `declare -i i; i=0#4` continuing past the error.

6. **Fix heredoc fd redirection `N<<EOF`** (`src/parser.rs`) — Added `DLess`, `DLessDash`, `TripleLess` to `try_parse_redir_fd`'s numeric fd redirect operator match. `3<<EOF` now recognized as fd 3 heredoc.

7. **Fix heredoc body ordering for multiple heredocs** (`src/parser.rs`) — Always use empty placeholder for heredoc bodies in `try_parse_redirection`. Added `resolve_heredoc_in_command` / `resolve_heredoc_in_program` / `resolve_heredoc_in_redirections` helpers that recursively fill in bodies for all command types (While, Until, If, For, Case, BraceGroup, Subshell, FunctionDef, Coproc). Prevents body swapping when `<<EOF1 3<<EOF2` appear on one line.

8. **Fix heredoc pipe fd leak** (`src/interpreter/redirects.rs`) — When the pipe read end is already the target fd (e.g. pipe returns fd 3 and we redirect to fd 3), don't close it after dup2 (which is a no-op).

9. **Use memfd for heredoc content** (`src/interpreter/redirects.rs`) — Replaced pipe with `memfd_create` + `write` + `lseek` for heredoc/herestring content. Pipes deadlock when content exceeds ~64KB buffer because write blocks with nobody reading. memfd is seekable and has no size limit. Fixes `heredoc5.sub` hang.

10. **Handle backslash-newline in heredoc bodies** (`src/lexer/heredoc.rs`) — For unquoted heredocs, when a line ends with `\`, join with the next line before checking for the delimiter. Fixes `cat <<EOF\nnext\<NL>EOF\nEOF` producing `nextEOF`.

## How to Run Tests

```bash
# Single nix test
nix build .#checks.x86_64-linux.rust-bash-test-NAME

# All tests, keep going on failure
nix build --keep-going .#checks.x86_64-linux.rust-bash-test-{alias,appendop,...}

# View failure diff
nix log .#checks.x86_64-linux.rust-bash-test-NAME

# Local testing (faster iteration)
cd /tmp/bash-5.3/tests
export PATH="/tmp/bash-5.3/tests:$PATH"
export THIS_SH=/home/noverby/Work/overby.me/rust/bash/target/debug/bash
diff <("$THIS_SH" ./NAME.tests 2>&1) <(bash ./NAME.tests 2>&1)
```

### Reference Bash Test Times

| Test | Ref Bash | Rust Bash | Notes |
|------|----------|-----------|-------|
| Most tests | < 0.1s | < 2s | OK |
| trap | 7.0s | ~17s | Uses `sleep` internally |
| arith | 0.035s | 1.7s | Hot loops |
| posixexp | 0.037s | 1.4s | Hot loops |
| heredoc | 0.06s | ~8s | Large pipe tests now work (memfd) |
| quotearray | 0.01s | **fails** | Assoc array + arith |

Suggested nix timeout: 30s for most tests, 120s for trap.

## 13 Failing Tests (sorted by diff size)

### Easiest (< 30 diff lines)

#### 1. posixexp (6 lines local, 3 issues remain in nix)

Three issues:

- **`<12>` vs `<1>\n<2>`**: `recho $a` after `IFS=; a=$@` with `set -- 1 2`. Our shell produces `<12>` (single arg), bash produces `<1><2>` (two args). Relates to how `$@` assignment interacts with null IFS — the assigned value should preserve the field boundaries for later `$a` expansion.
- **IFS splitting in `${var-$@}`**: With `IFS=:` or unset IFS, `${var-$@}` should join positional params by space (not split them individually). Our shell splits `$@` in the default word into separate fields. Two instances in posixexp4.sub.

#### 2. comsub-posix (0 lines local, ~35 lines nix)

Main test passes locally. Nix failures are error messages from intentionally-bad syntax in `${THIS_SH} -c '...'` invocations:

- `$(case x in x) ;; x) done esac)` — our shell reports wrong unexpected token
- `$(case x in x) (esac) esac)` — wrong error message format
- Error line numbers off by 1 in multi-line `-c` scripts
- Script continues after syntax error instead of stopping in some cases

#### 3. arith (0 lines main test, ~100 lines sub-tests)

Main `arith.tests` now passes with 0 diff. Sub-test issues in arith10.sub:

- **Array subscripts with special chars**: `a[" "]`, `a[\ \]`, `a[\\]` — quoting in array subscript arithmetic not handled correctly.
- **Empty array subscript `a[]`**: Should be "not a valid identifier" error.
- **`let` error formatting**: `let '0 - ""'` produces wrong error token.

### Medium (30-200 diff lines)

#### 4. heredoc (~20 lines local, ~85 lines nix)

Major improvements this session (111→20 local, no longer hangs in nix). Remaining:

- **heredoc3.sub**: Tab-stripped heredoc delimiter parsing, backslash continuation edge cases, `cat <<x*x` glob in delimiter.
- **heredoc7.sub**: Command substitution interacting with heredocs — unterminated heredoc in comsub.
- **heredoc9.sub**: `HERE; then` and `HERE; do` — heredoc delimiter followed by `;` and keyword on same line in function body printing.
- **Last block**: `echo $(cat <<< "comsub here-string")` and `cat <<''` (empty delimiter).
- **`echo "` vs `echo \"`**: Quoting difference in heredoc display.

#### 5. assoc (75 lines local, was 527)

Significant improvement. Remaining:

- Associative array `declare -p` key quoting differences.
- `[*]` key handling.
- Tilde expansion in associative array keys/values.

#### 6. new-exp (87 lines local, was ~375)

Remaining issues:

- `${HOME-'}'}` — single quotes don't protect `}` inside `${:-}` in dquote context.
- Backtick command substitution not expanded in `${var:offset}` arithmetic.
- `${#z}` used as substring offset not evaluated to variable length.
- Various expansion edge cases.

#### 7. builtins (93 lines local, was 336)

Significant improvement. Remaining:

- `enable -n` not implemented (disable builtins at runtime).
- `pushd`/`popd` with numeric args and error handling.
- `cd` with `--` argument.
- Hash table empty message format.

#### 8. quotearray (185 lines)

Associative array keys with special chars in arithmetic contexts.

#### 9. comsub2 (196 lines)

`local` outside function context, alias handling in subshells, function definition inside command substitution.

### Hard (200+ diff lines)

#### 10. nameref (258 lines local, was 750)

`declare -n` improvements but still substantially broken: wrong variable resolution, unset through nameref, nameref chains.

#### 11. varenv (36 lines local, was 340)

Significant improvement. Remaining:

- PATH handling differences.
- `local` inside function not finding variables from outer scope.
- SHELLOPTS/physical differences.
- Export propagation issues.

#### 12. array (446 lines local, was 1755)

Major improvement. Remaining issues in array32.sub, array33.sub (injection protection, type conversion errors).

#### 13. vredir (734K lines)

`{var}>file` variable fd redirection is partially implemented but produces massive output differences.

### Flaky

#### trap (0-4 lines)

Extra CHLD signals (non-deterministic). Sometimes passes, sometimes fails with 1-2 extra `CHLD` lines.

#### printf (0-3 lines)

Timing-dependent: `%(fmt)T` date format test can mismatch if test crosses a second boundary.

## Key Source Files

| File | Contents |
|------|----------|
| `src/ast.rs` | AST types, `WordPart` (includes `SyntaxError` variant) |
| `src/builtins/io.rs` | `read`, `echo` (EPIPE handling), `printf`, `mapfile` |
| `src/builtins/exec.rs` | `type`, `command`, `hash` |
| `src/builtins/vars.rs` | `declare`, `local`, `export`, `let` |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting |
| `src/builtins/set.rs` | `set`, `shopt` |
| `src/interpreter/mod.rs` | Shell struct, `declared_unset`, `run_string` (error recovery), `resolve_nameref` |
| `src/interpreter/commands.rs` | Command execution, `expand_word*`, `get_opt_flags`, `update_shellopts`, `execute_assignment` (now checks arith errors) |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (handles `\$` and backticks), error tokens, short-circuit assignment validation, ternary precedence |
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds, memfd heredocs, pipe fd leak fix) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling, `SyntaxError` handler |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`) |
| `src/lexer/mod.rs` | Lexer, thread-locals (`DQUOTE_TOGGLED`) |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param` (detects missing `}`), substring offset loop |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body` |
| `src/lexer/heredoc.rs` | `register_heredoc`, `read_heredoc_bodies` (backslash-newline continuation) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

1. **Fix IFS splitting in `${var-$@}`** — In `read_param_word_impl` or the expansion engine, `$@` inside `${var-word}` should be joined by space (not IFS) before field splitting. Would fix 4 of 6 remaining posixexp diff lines. (~4 diff lines)

2. **Fix `$@` assignment with null IFS** — `IFS=; a=$@` should preserve field boundaries so later `$a` splits into separate args. Would fix remaining 2 posixexp lines. (~2 diff lines)

3. **Fix comsub-posix error messages** — Improve error reporting for intentional syntax errors inside `$(...)` in case patterns. Need parser to detect reserved words like `done` in wrong context. (~35 nix diff lines)

4. **Fix arith10.sub array subscript quoting** — Handle `a[" "]`, `a[\ \]`, `a[\\]` in arithmetic array subscripts. (~100 nix diff lines)

5. **Fix heredoc sub-test issues** — heredoc3.sub (delimiter edge cases), heredoc7.sub (comsub+heredoc interaction), heredoc9.sub (function body printing). (~85 nix diff lines)

6. **Fix varenv remaining issues** — PATH handling, `local` scope, SHELLOPTS. Down to ~36 lines.

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set push-nkqwvorqmnkn -r @-` then `jj git push --bookmark push-nkqwvorqmnkn` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.