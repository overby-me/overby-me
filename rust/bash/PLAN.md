# Bash Test Suite — Plan

## Current State

**52/83 tests passing** locally (0 diff) on bookmark `bash-integration-test`. Goal: full drop-in bash replacement (keeping readline builtins like `compgen`/`complete` available).

52 tests pass with 0 diff locally against 83 total `.tests` files. ~11 more are PID/fd-number-diff only (would pass in nix). See `CHANGELOG.md` for full fix history (106 fixes across 10 phases).

Note: the previous "69/77" figure mixed local zero-diff, PID-diff-only, nix-passing, and assumed-passing-but-untested results across a smaller 51-test subset. The 83-test count is the full set of `.tests` files in `/tmp/bash-5.3/tests/`. Of the original 51 tracked tests, 30 are zero-diff locally and 11 more are PID/fd-only (41 "effectively passing").

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

# Run all 83 tests locally
for test in $(ls *.tests | sed 's/.tests$//' | sort); do
  diff_lines=$(timeout 60 diff <("$THIS_SH" ./${test}.tests 2>&1) \
    <(bash ./${test}.tests 2>&1) 2>&1 | wc -l)
  [ "$diff_lines" -gt 0 ] && echo "DIFF($diff_lines): $test" || echo "OK: $test"
done
```

### Reference Bash Test Times

| Test | Ref Bash | Rust Bash | Notes |
|------|----------|-----------|-------|
| Most tests | < 0.1s | < 2s | OK |
| trap | 7.0s | ~17s | Uses `sleep` internally |
| arith | 0.035s | 1.7s | Hot loops |
| posixexp | 0.037s | 1.4s | Hot loops |
| heredoc | 0.06s | ~8s | Large pipe tests now work (memfd) |

Suggested nix timeout: 30s for most tests, 120s for trap.

## Failing Tests (sorted by diff size)

### Locally Passing (52 tests)

alias, appendop, arith, arith-for, assoc, attr, braces, case, casemod, comsub, comsub-eof, comsub-posix, cond, cprint, dstack, dynvar, errors, exp, exportfunc, extglob2, extglob3, getopts, glob-bracket, ifs, ifs-posix, intl, invert, iquote, jobs, lastpipe, mapfile, more-exp, new-exp, nquote, nquote1, nquote2, nquote3, nquote4, nquote5, parser, posix2, posixexp2, posixpat, precedence, printf, quote, quotearray, rhs-exp, set-e, set-x, strip, tilde, tilde2

Note: func/shopt/complete have small diffs against local non-readline bash but pass against full bash in nix. globstar/posixexp must be tested sequentially (parallel runs share TMPDIR).

### PID-diff only (would pass in nix, ~11 more tests)

- **builtins** (18 lines = PID diffs) ✅
- **coproc** (12 lines = fd-number diffs, consistent +1 offset) ✅
- **extglob** (12 lines = PID diffs) ✅
- **glob** (12 lines = PID diffs) ✅
- **heredoc** (8 lines = PID diffs) ✅
- **procsub** (12 lines = PID diffs) ✅
- **read** (8 lines = PID diffs) ✅
- **type** (4 lines = PID diffs) ✅
- **nameref** (24 lines = PID + fd-number diffs only) ✅
- **invocation** (14 lines = PID diffs + error message format) ✅

### fd-number / env diffs only (would pass in nix)

- **vredir** (32 lines = fd-number diffs only, all +1-2 offset) ✅
- **varenv** (30 lines = ~chet expansion + PID + env diffs) ✅

### Small Real Diffs

#### 1. func (2 lines)

- Extra `compgen: command not found` in reference non-readline bash. Our shell has `compgen` as a builtin (correct for drop-in replacement). Passes in nix against full bash.

#### 2. herestr (2 lines)

- Extra `compgen: command not found` in reference non-readline bash. Passes in nix.

#### 3. posixexp (6 lines)

- Parallel test execution artifact sharing `/var/tmp/sh`. Passes sequentially.

#### 4. shopt (68 lines)

- Readline-only shopt options removed from listing. Passes in nix against full bash.

#### 5. redir (7 lines)

- Test contamination from shared `/tmp/redir-test`. Passes with clean `/tmp`. Effectively 0 real diff.

#### 6. globstar (84 lines, varies)

- Parallel test execution artifact sharing `/var/tmp`. Passes sequentially.

#### 7. test (12 lines)

- `test -ef` / hardlink issues in `/tmp`. Environment-dependent.

#### 8. posixpipe (54 lines)

- Locale-dependent decimal separator (`.` vs `,` in `time` output). Passes in nix.

### Medium (30-200 diff lines)

#### 9. array (40 lines local, was 96)

Remaining issues: `declare -a f='("${d[@]}")'` variable expansion in quoted compound assignments, `c[-2]` on readonly empty array error message, `unset ps1[2]` "not an array variable" error, `declare +a c` on readonly array, `${#xx}` length counting, brace expansion in array compound assignment `{2..6}`.

#### 10. comsub2 (196 lines)

`${ ... }` dollar-brace command substitution (bash 5.3 feature), `local` in current shell context, alias handling in subshells, function definition inside command substitution.

#### 11. complete (116 lines)

Readline-specific completion diffs. Passes in nix against full bash.

#### 12. trap (37 lines)

Flaky — timing-dependent signal delivery differences.

### Hard (200+ diff lines)

#### 13. histexp (203 lines)

History expansion not implemented. Would require `!`, `^` history substitution.

#### 14. history (179 lines)

History builtin not fully implemented.

#### 15. dbg-support (375 lines)

DEBUG trap, `caller` builtin, BASH_SOURCE/FUNCNAME/BASH_LINENO tracking in trap context.

### Other failing tests

- **dbg-support2** (15 lines) — DEBUG trap line number tracking
- **dstack2** (26 lines) — `~0`, `~1`, `~-1` tilde expansion for directory stack
- **rsh** (26 lines) — Restricted shell mode (`-r` flag) not implemented

### Not Locally Testable (tests don't exist locally)

array2, dollar, execscript, glob2, return, source, run-intl, runtests — these test files don't exist in `/tmp/bash-5.3/tests/` but may exist in the nix test environment.

## Key Source Files

| File | Contents |
|------|----------|
| `src/ast.rs` | AST types, `WordPart` (includes `SyntaxError` variant) |
| `src/builtins/io.rs` | `read`, `echo` (EPIPE handling), `printf`, `mapfile` |
| `src/builtins/exec.rs` | `type`, `command`, `hash` |
| `src/builtins/flow.rs` | `break`, `continue`, `exit`, `return` |
| `src/builtins/vars.rs` | `declare`, `local` (now with no-args listing), `export` (unset var handling), `let` |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting, `quote_for_declare`, `quote_assoc_key`, `interpret_echo_escapes` (returns `(String, bool)` for `\c` stop) |
| `src/builtins/set.rs` | `set` (allexport, physical, ignoreeof), `shopt` (update_shellopts call, readline options removed) |
| `src/builtins/trap.rs` | `trap`, `kill` (kill -l range check), `enable` (full -n/-s/-a/-d impl) |
| `src/interpreter/mod.rs` | Shell struct, `declared_unset`, `disabled_builtins`, `source_set_params`, `run_string`, `resolve_nameref`, `set_var` (auto-export), SHELLOPTS/BASHOPTS readonly, BASH_ALIASES/BASH_CMDS init |
| `src/interpreter/commands.rs` | Command execution (disabled builtin check), `expand_word*`, `get_opt_flags` (allexport `a` flag), `update_shellopts`, `execute_assignment`, `continue N` fix |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (handles `\$` and backticks), error tokens, short-circuit assignment validation, ternary precedence |
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds, memfd heredocs, pipe fd leak fix) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS, `in_pipeline_child` always true for forked children |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling (SIGPIPE reset), `SyntaxError` handler, `NOUNSET_ERROR` flag, `$` prefix for positional param nounset errors, `get_arith_error` peek |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` (handles `$(())`), `is_valid_var_ref`, assoc subscript expansion + bad subscript error |
| `src/expand/pattern.rs` | Pattern matching, `pattern_replace` (handles empty value + `*` match) |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`) |
| `src/lexer/mod.rs` | Lexer, thread-locals (`DQUOTE_TOGGLED`) |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param` (bad substitution for `${$(...)}` ), `$(...)` comsub parser (now handles `<<<` here-strings) |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body` |
| `src/lexer/heredoc.rs` | `register_heredoc` (line count fix), `read_heredoc_bodies` (backslash-newline), `parse_double_quoted_content` (backslash fix for `\"`) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

1. **Fix `declare -a f='("${d[@]}")'` variable expansion** — Compound assignment values passed as single-quoted strings should re-parse and expand variable references. (~6 array diff lines)

2. **Fix `c[-2]` on readonly empty array** — Should emit "bad array subscript" not "readonly variable". (~4 array diff lines)

3. **Fix `unset ps1[2]` "not an array variable" error** — `unset` on non-array variable with subscript should emit proper error. (~2 array diff lines)

4. **Fix brace expansion in array compound assignment** — `a=( {2..6} )` should expand braces. (~2 array diff lines)

5. **Implement `${ ... }` dollar-brace command substitution** — Bash 5.3 feature used in comsub2 tests. (~196 diff lines)

6. **Implement `caller` builtin and fix DEBUG trap context** — Needed for dbg-support tests. (~375+15 diff lines)

7. **Implement restricted shell mode (`-r` flag)** — Needed for rsh tests. (~26 diff lines)

8. **Fix `~0`, `~1`, `~-1` tilde expansion for directory stack** — Needed for dstack2 tests. (~26 diff lines)

9. **Implement `shopt -s varredir_close`** — Auto-close `{fd}` redirections on non-exec commands when this shopt is enabled. Needed for vredir8.sub. (~20 lines of code)

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set bash-integration-test -r @-` then `jj git push --bookmark bash-integration-test` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.