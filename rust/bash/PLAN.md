# Bash Test Suite — Plan

## Current State

**64/77 nix tests passing**, 52/83 local tests passing (0 diff) on bookmark `bash-integration-test`. Goal: full drop-in bash replacement (keeping readline builtins like `compgen`/`complete` available).

See `CHANGELOG.md` for full fix history (108+ fixes across 13 phases).

### Nix test results (64/77 passing)

Passing (64): alias, appendop, arith-for, array2, attr, braces, case, casemod, comsub-eof, comsub-posix, cond, coproc, cprint, dirstack, dollars, dynvar, errors, execscript, exp-tests, exportfunc, extglob, extglob2, extglob3, func, getopts, glob-bracket, glob-test, globstar, herestr, ifs, ifs-posix, input-test, invert, iquote, mapfile, more-exp, nquote, nquote1, nquote2, nquote3, nquote4, nquote5, parser, posix2, posixexp, posixexp2, posixpat, posixpipe, precedence, printf, procsub, quote, read, redir, rhs-exp, set-e, set-x, shopt, strip, test, tilde, tilde2, type, vredir

Failing (13):

| Test | Nix diff lines | Notes |
|------|---------------|-------|
| trap | 1 | Flaky — timing-dependent signal delivery (extra CHLD) |
| comsub | 1 | Spurious `echo: write error: Broken pipe` (flaky timing) |
| lastpipe | 1 | Spurious `echo: write error: Broken pipe` (flaky timing) |
| arith | ~90 | arith10.sub: array subscript quoting (`a[]`, `a[" "]`, `a[\\]`), xtrace spacing |
| heredoc | ~12 | heredoc3.sub: `EOF)` syntax, heredoc7.sub: comsub+heredoc interaction |
| comsub2 | 184 | `${ ... }` dollar-brace comsub (bash 5.3 feature) |
| quotearray | 179 | Assoc array keys with special chars in `((...))` context |
| builtins | 275 | pushd/popd sub-tests, dir stack edge cases |
| new-exp | 310 | Sub-tests: various edge cases |
| varenv | 320 | Sub-tests: env/export edge cases |
| assoc | 462 | Sub-tests: tilde expansion in assoc values, bracket keys |
| array | 647 | Sub-tests: array32/33 (injection guards, assoc↔indexed conversion) |
| nameref | 678 | Sub-tests: complex nameref resolution chains |

### Local test results (52/83 passing, 0 diff)

83 total `.tests` files in `/tmp/bash-5.3/tests/` (superset of the 77 nix tests — includes dbg-support, dbg-support2, dstack2, histexp, history, rsh, invocation, jobs, posixpipe, and others not in the nix harness).

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

## Failing Nix Tests (13/77)

### Near-passing (1-line diffs, likely flaky)

- **trap** (1 line) — Timing-dependent signal delivery (extra CHLD)
- **comsub** (1 line) — Spurious `echo: write error: Broken pipe` (SIGPIPE timing race in nix sandbox)
- **lastpipe** (1 line) — Spurious `echo: write error: Broken pipe` (SIGPIPE timing race in nix sandbox)

### Medium diffs

- **arith** (~90 lines) — arith10.sub: array subscript quoting (`a[]`, `a[" "]`, `a[\\]`), xtrace spacing for expanded vars
- **heredoc** (~12 lines) — heredoc3.sub: `EOF)` delimiter-on-non-own-line syntax, heredoc7.sub: comsub+heredoc interaction
- **comsub2** (184 lines) — `${ ... }` dollar-brace comsub (bash 5.3 feature)
- **quotearray** (179 lines) — Assoc array keys with special chars in `((...))` context

### Large diffs (sub-tests with many edge cases)

- **builtins** (275 lines) — pushd/popd sub-tests, dir stack edge cases
- **new-exp** (310 lines) — Sub-tests: various expansion edge cases
- **varenv** (320 lines) — Sub-tests: env/export edge cases
- **assoc** (462 lines) — Sub-tests: tilde expansion in assoc values, bracket keys
- **array** (647 lines) — Sub-tests: array32/33 (injection guards, assoc↔indexed conversion)
- **nameref** (678 lines) — Sub-tests: complex nameref resolution chains

### Local-only failing tests (not in nix harness)

These exist in `/tmp/bash-5.3/tests/` but not in the nix test list:

- **dbg-support** (375 lines) — DEBUG trap, `caller` builtin, BASH_SOURCE/FUNCNAME tracking
- **dbg-support2** (15 lines) — DEBUG trap line number tracking
- **dstack2** (26 lines) — `~0`, `~1`, `~-1` tilde expansion for directory stack
- **histexp** (203 lines) — History expansion not implemented
- **history** (179 lines) — History builtin not fully implemented
- **rsh** (26 lines) — Restricted shell mode (`-r` flag) not implemented
- **invocation** (14 lines) — PID diffs + error message format
- **complete** (116 lines) — Readline-specific completion diffs (local non-readline bash lacks compgen)

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
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds with nameref support, varredir_close, fd validation, memfd heredocs, pipe fd leak fix) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS, `in_pipeline_child` always true for forked children, SIGPIPE reset to SIG_DFL in pipeline/comsub children |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling (SIGPIPE reset), `SyntaxError` handler, `NOUNSET_ERROR` flag, `$` prefix for positional param nounset errors, `get_arith_error` peek, `SplitHereStar` segment for `$*` null-IFS splitting |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` (handles `$(())`), `is_valid_var_ref`, assoc subscript expansion + bad subscript error |
| `src/expand/pattern.rs` | Pattern matching, `pattern_replace` (handles empty value + `*` match) |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`) |
| `src/lexer/mod.rs` | Lexer, thread-locals (`DQUOTE_TOGGLED`), `force_read_pending_heredocs` (save/restore position for `&` + heredoc), `heredoc_resume` |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param` (bad substitution for `${$(...)}` ), `$(...)` comsub parser (now handles `<<<` here-strings) |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body` |
| `src/lexer/heredoc.rs` | `register_heredoc` (line count fix), `read_heredoc_bodies` (backslash-newline, `<<-` tab-stripped delimiter matching), `parse_double_quoted_content` (backslash fix for `\"`) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

### Low-hanging fruit (could flip nix tests to passing)

1. **Fix SIGPIPE flaky tests (comsub/lastpipe/trap)** — 1-line diff each, timing race in nix sandbox. SIGPIPE is reset to SIG_DFL in pipeline/comsub children and EPIPE is suppressed in echo builtin for all subprocess contexts, but the nix sandbox timing still occasionally triggers the race. trap has an extra CHLD signal delivery.

### Array/assoc improvements (largest nix diff contributors)

2. **Fix arith10.sub array subscript quoting** — Handle `a[]`, `a[" "]`, `a[\ \]`, `a[\\]` in arithmetic array subscripts. (~90 nix diff lines)

3. **Fix assoc sub-tests** — Tilde expansion in assoc array values (`declare -A aa=([key]="~/Desktop")`), bracket key handling. (~462 nix diff lines)

4. **Fix array32/33 sub-tests** — Command injection guards in array subscripts, assoc↔indexed conversion errors. (~647 nix diff lines — large but many are the same root cause)

5. **Fix nameref sub-tests** — Complex nameref resolution chains in sub-tests. (~678 nix diff lines)

### Feature work

6. **Implement `${ ... }` dollar-brace command substitution** — Bash 5.3 feature used in comsub2 tests. (~184 nix diff lines)

7. **Fix remaining heredoc sub-tests** — heredoc3.sub: `EOF)` delimiter-not-on-own-line, heredoc7.sub: comsub+heredoc interaction. (~12 nix diff lines)

8. **Fix builtins sub-tests** — pushd/popd edge cases, dir stack numeric args. (~275 nix diff lines)

9. **Implement `caller` builtin and fix DEBUG trap context** — Needed for dbg-support tests (local-only). (~375+15 diff lines)

10. **Implement restricted shell mode (`-r` flag)** — Needed for rsh tests (local-only). (~26 diff lines)

## Recent Fixes (Phase 13)

- **comsub-posix: 20 → 0 diff lines (now passing)**
  - Fix LINENO for COMSUB parse errors: capture lexer line before `advance()` in `take_word_checked` and embed in error via `COMSUB_LINE:N:` prefix, so multi-line commands report the correct error line
  - Only update LINENO for COMSUB errors (non-COMSUB errors use pre-parse LINENO which is already correct)
- **heredoc: 66 → ~12 diff lines**
  - Fix `<<-` tab stripping to also strip leading tabs from the delimiter itself when matching (e.g. `<<-'\tEND'` now correctly matches `\tEND` body lines)
  - Fix function body printing: `then`/`do` after heredoc delimiters now appear on their own indented line instead of `HERE; then` (matches bash's `declare -pf` output)
  - Fix heredoc body assignment for backgrounded commands: when `cmd <<DELIM & cmd2` appears on one line, force-read pending heredoc bodies before resolving, using save/restore of lexer position so the foreground command tokens aren't lost
  - Merge `While`/`Until` formatting into a single match arm, using the `keyword` variable correctly
- **No regressions** — all 63 previously-passing nix tests still pass

## Recent Fixes (Phase 12)

- **vredir: 32 → 0 diff lines (now passing)**
  - Fix `{v}>&-` / `{v}<&-` close operations to read variable value instead of allocating new fd
  - Resolve namerefs in `{var}` redirect fd allocation
  - Validate source fd exists before saving in DupOutput/DupInput (prevents F_DUPFD_CLOEXEC from masking invalid redirect targets)
  - Print dup error before restoring redirections so errors flow through already-setup redirect chains
  - Close inherited non-CLOEXEC fds >= 10 on startup to prevent leaked parent fds from shifting `{var}` allocation base
  - Implement `varredir_close` shopt (close `{var}` fds when command finishes)
  - Fix `{var}<&-` to print as `{var}>&-` in function body display
  - Handle fd allocation failure (ulimit) with proper error chain
  - Use `F_DUPFD(10)` instead of scanning with `dup()` for fd allocation
- **arith xtrace: expand $var in ((...)) trace output** — shows expanded values instead of literal `$var`, matching bash
- **redir: no regressions** — fd validation fix also resolved redir11.sub grep tests

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set bash-integration-test -r @-` then `jj git push --bookmark bash-integration-test` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.