# Bash Test Suite — Continuation Notes

## Current State

**64/77 tests passing** on bookmark `push-nkqwvorqmnkn`. All changes committed and pushed.

### Progress This Session

- **Started at**: 62/77
- **Now at**: 64/77 (+2: comsub, lastpipe)
- **Trap**: flaky — passes sometimes, fails with extra CHLD signals other times
- **Significant diff reductions**: new-exp (719→382), array (1595→1755 but now produces output instead of aborting)

### Fixes Applied This Session

1. **EPIPE handling** (`src/builtins/io.rs`) — Report "Broken pipe" from echo when NOT in a pipeline child. Fixes comsub + lastpipe tests.

2. **Parser error recovery for array compound assignments** (`src/parser.rs`, `src/interpreter/mod.rs`) — `parse_array_elements` returns `Result`, detects unexpected tokens (e.g. `&`), reports correct error token, skips to `)`, marks error as recoverable. `run_string` no longer exits on recoverable syntax errors.

3. **Backtick tracking in `${var:offset}` substring parsing** (`src/lexer/dollar.rs`) — Added `` ` ``, `"`, and `$(...)` tracking in the Substring offset and length character loops so `}` inside backtick/dquote/comsub contexts doesn't prematurely close the `${...}` expansion. Fixes cascading parse failure in new-exp.

4. **`${var:-default}` in arithmetic evaluation** (`src/expand/arithmetic.rs`) — Added `${...}` parameter expansion handling to `resolve_arith_vars`, supporting `${var}`, `${var:-default}`, `${var:+alt}`, `${var:=assign}`, `${#var}`, and subscript syntax. Fixes `${PARAM:${OFFSET:-0}}` in arith7.sub.

5. **Declared-but-unset variable tracking** (`src/interpreter/mod.rs`, `src/builtins/vars.rs`) — Added `declared_unset: HashSet<String>` to Shell. `declare name` without `=` marks the variable as declared-but-unset instead of inserting empty string into vars. `declare -p` prints without `=""` for these. `set_var` clears the flag on assignment.

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
| heredoc | 0.06s | **hangs** | Timeout in nix |
| quotearray | 0.01s | **hangs** | Timeout in nix |

Suggested nix timeout: 30s for most tests, 120s for trap. heredoc and quotearray hang.

## 13 Failing Tests (sorted by diff size)

### Easiest (< 30 diff lines)

#### 1. posixexp (13 lines)

Four issues:

- **`<12>` vs `<1>\n<2>`**: Word splitting in some expansion context (nix-only, likely sub-test environment difference).
- **IFS splitting**: `< abc def ghi jkl >` not split into `< abc>`, `<def ghi>`, `<jkl >` — two instances.
- **Parser bug**: `echo "${foo:-"a}"` should produce `unexpected EOF while looking for matching '}'`. In bash, `"` inside `${var:-word}` when in dquote context toggles the quote state. Our `read_param_word_impl` in `src/lexer/word.rs` treats inner `"` as opening a nested dquote, which incorrectly protects `}` from closing the expansion.

#### 2. comsub-posix (20 lines)

`case` statement parsing inside `$(...)` command substitutions. The `)` in case patterns like `x)` confuses the comsub delimiter matching in `skip_comsub` (`src/lexer/word.rs` L889-995). The function tracks `case_depth` but has edge cases with patterns like `$(case x in x) ;; x) done esac)`.

### Medium (30-200 diff lines)

#### 3. arith (79 lines)

Multiple issues:

- **`let` and literal `$`**: `let 'jv += $iv'` should error — `$iv` is literal (single-quoted). Our shell expands it.
- **Short-circuit assignment**: `0 && B=42` in `$(( ))` should error "attempted assignment to non-variable".
- **`--x++`**: Should error "assignment requires lvalue".
- **Backtick comsub**: `` `echo 1+1` `` not expanded inside `$(( ))`.
- **Error prefix**: `let:` prefix missing; `((:` prefix missing from some messages.
- **Escaped array subscripts**: `a[" "]`, `a[\ \]`, `a[\\]` — quoting in array subscript arithmetic.

#### 4. heredoc (111 lines, hangs in nix)

- Heredoc fd redirection (`3<<EOF`) not working properly.
- Tab-stripped heredocs (`<<-EOF`) issues.
- Backslash-newline in heredoc delimiters.
- Sub-file tests (`heredoc1.sub` through `heredoc10.sub`).

#### 5. quotearray (178 lines, hangs in nix)

Associative array keys with special chars in arithmetic contexts. Hangs locally after ~30s.

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

#### 9. new-exp (382 lines)

Cascading failure fixed (was 719). Remaining issues:

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
| `src/builtins/io.rs` | `read`, `echo` (EPIPE handling), `printf`, `mapfile` |
| `src/builtins/exec.rs` | `type`, `command`, `hash` |
| `src/builtins/vars.rs` | `declare`, `local`, `export`, `let` |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting |
| `src/builtins/set.rs` | `set`, `shopt` |
| `src/interpreter/mod.rs` | Shell struct, `declared_unset`, `run_string` (error recovery), `resolve_nameref` |
| `src/interpreter/commands.rs` | Command execution, `expand_word*`, `get_opt_flags`, `update_shellopts` |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` |
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (now handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (now returns Result), `skip_to_next_command` |
| `src/lexer/dollar.rs` | `${}` parsing, substring offset loop (now tracks backticks/dquotes) |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case depth tracking) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

1. **Fix posixexp dquote toggle** — Make `"` inside `${:-word}` when `in_dquote=true` close the outer dquote instead of opening nested dquote. Tricky: must still allow `"${foo:-"a"}"` (balanced inner quotes). (+1 test if all 4 issues fixed)

2. **Fix `case` inside `$()` in comsub-posix** — Improve `skip_comsub` to handle case patterns where `)` appears without leading `(`. (+1 test)

3. **Fix `let` literal `$` in arith** — Skip `$var` expansion when `arith_is_let` is true and the expression came from single-quoted `let` arg.

4. **Fix heredoc hanging** — Investigate what causes the test to hang (likely an fd issue or infinite read loop).

5. **Fix vredir** — The `{var}` fd mechanism exists but has issues with save/restore and closing. The test produces 734K diff lines suggesting an infinite loop or incorrect fd handling.

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set push-nkqwvorqmnkn -r @-` then `jj git push --bookmark push-nkqwvorqmnkn` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.