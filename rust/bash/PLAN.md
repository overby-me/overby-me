# Bash Test Suite — Continuation Notes

## Current State

**64/77 tests passing** on bookmark `push-nkqwvorqmnkn`. All changes committed and pushed.

### Progress This Session

- **Started at**: 64/77
- **Now at**: 64/77 (same count, but diff reductions in arith and posixexp)
- **Trap**: flaky — passes sometimes, fails with extra CHLD signals other times
- **Significant diff reductions**: arith (79→30), posixexp fixed dquote `}` detection

### Fixes Applied This Session

1. **Detect missing `}` in param expansion when nested dquote consumes it** (`src/ast.rs`, `src/lexer/dollar.rs`, `src/lexer/mod.rs`, `src/expand/mod.rs`, `src/builtins/mod.rs`) — Added `SyntaxError` WordPart variant. In `parse_brace_param`, after `read_param_op` returns, check if `}` is present; if not (consumed by inner `"..."` pair), return `SyntaxError("unexpected EOF while looking for matching '}'")`. Fixes `echo "${foo:-"a}"` producing `bar` instead of error. Posixexp dquote toggle issue resolved.

2. **Rewrite `skip_comsub` with proper case statement state machine** (`src/lexer/word.rs`) — Replaced simple `case_depth` counter with a `Vec<i32>` state stack tracking: 0=not-in-case, 1=saw-case-waiting-for-in, 2=in-pattern-position, 3=in-case-body. Added `;;`/`;&`/`;;&` detection to transition back to pattern state. Added `(` handling in pattern position (optional leading paren). Added comment skipping and `\` escape handling. Fixes normal `$(case x in x) echo y;; esac)` patterns.

3. **Fix arithmetic error tokens for literal dollar expressions** (`src/interpreter/arithmetic.rs`) — Error token at fallthrough now shows the inner expression (e.g. `$iv`) instead of the full top-level expression (`jv += $iv`). Added `\$` handling in `expand_comsubs_in_arith` to preserve backslash-dollar as literal (skip expansion). Added `\\$` prefix check in `eval_arith_expr_inner` to skip `$var` expansion for `\$var` from `$(( \$iv ))`. Arith diff reduced from 79 to ~30 lines.

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
| heredoc | 0.06s | **fails** | fd / backslash issues |
| quotearray | 0.01s | **fails** | Assoc array + arith |

Suggested nix timeout: 30s for most tests, 120s for trap.

## 13 Failing Tests (sorted by diff size)

### Easiest (< 30 diff lines)

#### 1. posixexp (7 lines nix, 3 issues remain)

Three issues (dquote `}` detection FIXED):

- **`<12>` vs `<1>\n<2>`**: `recho $a` after `IFS=; a=$@` with `set -- 1 2`. Our shell produces `<12>` (single arg), bash produces `<1><2>` (two args). Relates to how `$@` assignment interacts with null IFS — the assigned value should preserve the field boundaries for later `$a` expansion.
- **IFS splitting in `${var-$@}`**: With `IFS=:` or unset IFS, `${var-$@}` should join positional params by space (not split them individually). Our shell splits `$@` in the default word into separate fields. Two instances in posixexp4.sub.

#### 2. comsub-posix (20 lines)

Normal `case` inside `$(...)` now works. Remaining issues are error messages from intentionally-bad syntax in `${THIS_SH} -c '...'` invocations:

- `$(case x in x) ;; x) done esac)` — our shell silently succeeds, bash detects `done` as unexpected reserved word
- `$(case x in x) (esac) esac)` — wrong error message format
- Error line numbers off by 1 in multi-line `-c` scripts
- Script continues after syntax error instead of stopping

#### 3. arith (~30 lines, was 79)

Remaining issues (literal `$` and error token FIXED):

- **Short-circuit assignment**: `0 && B=42` in `$(( ))` should error "attempted assignment to non-variable". Similarly `1 || B=88`.
- **`--x++`**: Should error "assignment requires lvalue" instead of evaluating.
- **Backtick comsub**: `` `echo 1+1` `` not expanded inside `$(( ))`.
- **`2#110#11`**: Double hash not detected as invalid number.
- **Escaped array subscripts**: `a[" "]`, `a[\ \]`, `a[\\]` — quoting in array subscript arithmetic.
- **Minor**: Trailing space missing in some error tokens (e.g. `$iv` vs `$iv`).

### Medium (30-200 diff lines)

#### 4. heredoc (111 lines)

- Heredoc fd redirection (`3<<EOF`) not working properly — Bad file descriptor.
- Backslash-newline in heredoc delimiters — `next\` + `EOF` should produce `nextEOF`.
- `EOF: command not found` — heredoc delimiter not recognized.
- Sub-file tests (`heredoc1.sub` through `heredoc10.sub`).

#### 5. quotearray (178 lines)

Associative array keys with special chars in arithmetic contexts.

#### 6. comsub2 (185 lines)

`local` outside function context, alias handling in subshells, function definition inside command substitution.

### Hard (200+ diff lines)

#### 7. builtins (336 lines)

- `enable -n` not implemented (disable builtins at runtime).
- `pushd`/`popd` with numeric args and error handling.
- `cd` with `--` argument.
- Hash table empty message format.

#### 8. varenv (340 lines)

- PATH handling differences.
- `local` inside function not finding variables from outer scope.
- SHELLOPTS/physical differences.
- Export propagation issues.
- `declare -p` for readonly variables in function scope.

#### 9. new-exp (~375 lines)

Remaining issues:

- `${HOME-'}'}` — single quotes don't protect `}` inside `${:-}` in dquote context.
- Backtick command substitution not expanded in `${var:offset}` arithmetic.
- `${#z}` used as substring offset not evaluated to variable length.
- Various expansion edge cases.

#### 10. assoc (527 lines)

- Associative array `declare -p` key quoting differences.
- `[*]` key handling.
- Tilde expansion in associative array keys/values.

#### 11. nameref (750 lines)

`declare -n` extensively broken: wrong variable resolution, unset through nameref, nameref chains. The nameref implementation in `src/interpreter/mod.rs` `resolve_nameref()` is too simple.

#### 12. array (1755 lines)

Parser error recovery now works (was aborting on line 34). Remaining issues are array feature bugs in array32.sub, array33.sub (injection protection, type conversion errors).

#### 13. vredir (734K lines)

`{var}>file` variable fd redirection is partially implemented (parser + `resolve_redir_fd` work) but produces massive output differences. The test uses `exec {v}>>file`, heredocs with variable fds, and closing variable fds with `{v}>&-`. Sub-files vredir1-8.sub test complex scenarios.

### Flaky

#### trap (0-4 lines)

Extra CHLD signals (non-deterministic). Sometimes passes, sometimes fails with 2 extra `CHLD` lines.

## Key Source Files

| File | Contents |
|------|----------|
| `src/ast.rs` | AST types, `WordPart` (now includes `SyntaxError` variant) |
| `src/builtins/io.rs` | `read`, `echo` (EPIPE handling), `printf`, `mapfile` |
| `src/builtins/exec.rs` | `type`, `command`, `hash` |
| `src/builtins/vars.rs` | `declare`, `local`, `export`, `let` |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting |
| `src/builtins/set.rs` | `set`, `shopt` |
| `src/interpreter/mod.rs` | Shell struct, `declared_unset`, `run_string` (error recovery), `resolve_nameref` |
| `src/interpreter/commands.rs` | Command execution, `expand_word*`, `get_opt_flags`, `update_shellopts` |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (now handles `\$`), error tokens |
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling, `SyntaxError` handler |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (now handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (now returns Result), `skip_to_next_command` |
| `src/lexer/mod.rs` | Lexer, thread-locals (`DQUOTE_TOGGLED` added) |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param` (now detects missing `}`), substring offset loop |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (rewritten with case state machine) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

1. **Fix short-circuit assignment in arith** — `0 && B=42` and `1 || B=88` in `$(( ))` should error. Need to detect assignment operators in the RHS of `&&`/`||` when the short-circuit means the RHS shouldn't execute but the parser still validates it. (~6 diff lines)

2. **Fix `--x++` in arith** — Should error "assignment requires lvalue". Need pre-decrement followed by post-increment detection. (~4 diff lines)

3. **Fix heredoc fd redirection** — `3<<EOF` should work. Likely need to wire up non-standard fd numbers in heredoc setup. Could unblock many heredoc sub-tests.

4. **Fix IFS splitting in `${var-$@}`** — In `read_param_word_impl` or the expansion engine, `$@` inside `${var-word}` should be joined by space (not IFS) before field splitting. Affects posixexp.

5. **Fix comsub-posix error messages** — Improve error reporting for intentional syntax errors inside `$(...)` in case patterns. Needs parser changes to detect reserved words like `done` in wrong context.

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set push-nkqwvorqmnkn -r @-` then `jj git push --bookmark push-nkqwvorqmnkn` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.