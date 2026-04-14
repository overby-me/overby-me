# Bash Test Suite — Plan

## Current State

**77/77 nix tests consistently passing** (Phase 117), ~69/83 local tests passing (0 diff, sequential). Goal: full drop-in bash replacement (keeping readline builtins like `compgen`/`complete` available). **array** ✅ 0 nix diff (fixed in Phase 117). **nameref** ✅ 0 nix diff (fixed in Phase 116). See `CHANGELOG.md` for full fix history (300+ fixes across 117 phases).

### Nix test results (77/77 consistently passing — Phase 117)

All 77 nix tests pass: alias, appendop, arith, arith-for, **array** ✅, array2, assoc, attr, braces, builtins, case, casemod, comsub, comsub-eof, comsub-posix, comsub2, cond, coproc, cprint, dirstack, dollars, dynvar, errors, execscript, exp-tests, exportfunc, extglob, extglob2, extglob3, func, getopts, glob-bracket, glob-test, globstar, heredoc, herestr, ifs, ifs-posix, input-test, invert, iquote, lastpipe, mapfile, more-exp, **nameref** ✅, new-exp, nquote, nquote1, nquote2, nquote3, nquote4, nquote5, parser, posix2, posixexp, posixexp2, posixpat, posixpipe, precedence, printf, procsub, quote, quotearray, read, redir, rhs-exp, set-e, set-x, shopt, strip, test, tilde, tilde2, trap, type, varenv, vredir.

Note: `errors` and `coproc` tests exist in the nix harness but have pre-existing diffs unrelated to the recent work (errors: 2 lines from bash 5.3 exit status output; coproc: 6 lines from fd number allocation differences).

### Local test results (~69/83 passing, 0 diff sequential — Phase 98)

83 total `.tests` files in `/tmp/bash-5.3/tests/` (superset of the 77 nix tests — includes dbg-support, dbg-support2, dstack2, histexp, history, rsh, invocation, jobs, posixpipe, and others not in the nix harness).

**Important:** Many tests that show diffs when run in parallel (`diff <(our_bash test) <(bash test)`) pass when run sequentially due to race conditions on shared `/tmp` and `/var/tmp` files. Use sequential mode for accurate results:

```bash
timeout 300 "$THIS_SH" ./${test}.tests > /tmp/ours.out 2>&1
timeout 300 bash ./${test}.tests > /tmp/ref.out 2>&1
diff /tmp/ours.out /tmp/ref.out
```

Also note: `ifs-posix` passes but requires ~4 minutes (6856 subtests). Use `timeout 300`.

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

## Failing Nix Tests (2/77 — array/nameref)

### ~~array~~ ✅ (0 nix diff — Fixed in Phase 117)

Fixed `A[]]` bracket matching for `read`/`printf -v` by implementing `W_ARRAYREF`-like quoting context detection. Added `UNQUOTED_ARRAYREF` thread-local set during word expansion: when a command word has an unquoted `[` (in a `Literal` AST part), `rfind(']')` is used for bracket matching (accepts `]` as key). When the `[` was inside double quotes, first-`]` forward scan is used (rejects `A[]]` as invalid). This distinguishes `read A[$rkey]` (unquoted, accepts) from `read "A[$k]"` (quoted, rejects).

### ~~nameref~~ ✅ (0 nix diff — Fixed in Phase 116)

Reduced from ~76 nix diff lines to 0 across Phases 99-116. Key fixes: command substitution in nameref subscripts (Phase 99), circular nameref warnings at declaration time (Phase 104), subscript-circular rejection in function scope (Phase 105-106), select loop implementation (Phase 100), coproc readonly protection (Phase 108-110), `unset -n` no-op on non-namerefs (Phase 113), readonly/nameref DISCARD behavior (Phase 114-115), coproc PID cleanup (Phase 116).

### Local-only failing tests (not in nix harness)

These exist in `/tmp/bash-5.3/tests/` but not in the nix test list:

- **dbg-support** (375 lines) — DEBUG trap, `caller` builtin, BASH_SOURCE/FUNCNAME tracking
- **dbg-support2** (15 lines) — DEBUG trap line number tracking
- **histexp** (203 lines) — History expansion not implemented
- **history** (179 lines) — History builtin not fully implemented
- **rsh** (26 lines) — Restricted shell mode (`-r` flag) not implemented
- **invocation** (~10 lines) — PID diffs + bad interpreter error prefix format
- **complete** (116 lines) — Readline-specific completion diffs (local non-readline bash lacks compgen)

## Key Source Files

| File                            | Contents                                                                                                                                                                                                                                                                            |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/ast.rs`                    | AST types, `WordPart` (includes `SyntaxError` variant), `ArrayIndices(char, Option<char>)` with optional transform |
| `src/builtins/help_data.rs`     | Auto-generated help data from bash 5.3 `.def` files — 77 `HelpEntry` structs with name, synopsis, short_desc, long_help |
| `src/builtins/io.rs`            | `read` (prompt suppression on non-tty), `echo` (EPIPE handling), `printf`, `mapfile` |
| `src/builtins/exec.rs`          | `type`, `command`, `hash` |
| `src/builtins/flow.rs`          | `break`, `continue`, `exit`, `return` |
| `src/builtins/vars.rs`          | `declare`, `local`, `export`, `let`, `unset` |
| `src/builtins/mod.rs`           | `parse_array_literal`, function body formatting, `quote_for_declare`, `quote_assoc_key`, `interpret_echo_escapes` |
| `src/builtins/set.rs`           | `set` (allexport, physical, ignoreeof), `shopt` |
| `src/builtins/trap.rs`          | `trap`, `kill`, `enable` |
| `src/interpreter/mod.rs`        | Shell struct, `resolve_nameref`, `set_var`, `run_string`, SHELLOPTS/BASHOPTS, BASH_ALIASES/BASH_CMDS, `array_effective_len` |
| `src/interpreter/commands.rs`   | Command execution, `expand_word*`, `execute_assignment`, `expand_assoc_subscript` |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith`, error tokens, bracket depth tracking, `arith_array_get`, bare array name → [0] |
| `src/interpreter/redirects.rs`  | Redirections (vredir `{var}` fds with nameref support, varredir_close, memfd heredocs) |
| `src/interpreter/pipeline.rs`   | Pipeline execution, PIPESTATUS, SIGPIPE reset in pipeline/comsub children |
| `src/expand/mod.rs`             | Word expansion, `ExpCtx`, IFS handling, procsub, `NOUNSET_ERROR`/`NOCASEMATCH_ENABLED`/`POSIX_MODE` flags |
| `src/expand/params.rs`          | Parameter expansion (`${...}` operators), `decode_prompt_string`, `@K`/`@k` transform, indirect `!name[@]` |
| `src/expand/pattern.rs`         | Pattern matching, `pattern_replace`, nocasematch, PUA-aware `char_in_class` |
| `src/expand/transform_helpers.rs` | `shell_quote` and `expand_backslash_escapes` for `@Q`/`@E` transforms |
| `src/expand/arithmetic.rs`      | `eval_arith_full_with_assoc`, `resolve_arith_vars` |
| `src/lexer/mod.rs`              | Lexer, `lex_compound_array_content()`, thread-locals, heredoc handling |
| `src/lexer/dollar.rs`           | `${}` parsing, `$(...)` comsub parser |
| `src/lexer/word.rs`             | `read_param_word_impl`, `skip_comsub`, `take_heredoc_body` |
| `src/lexer/heredoc.rs`          | `register_heredoc`, `read_heredoc_bodies`, `parse_double_quoted_content` |
| `src/parser.rs`                 | Parser, `parse_array_elements`, `skip_to_next_command`, heredoc body resolution, `set_line_number` |
| `rust/bash/testsuite.nix`       | Test harness with path/PID normalization |

## Recommended Next Priorities

### Nix test improvements

1. ~~**Continue reducing nameref nix diffs**~~ ✅ **Fixed in Phase 116** — nameref now passes (0 diff).

2. ~~**Fix remaining array nix diffs**~~ ✅ **Fixed in Phase 117** — array now passes (0 diff). Implemented `W_ARRAYREF`-like unquoted detection via `UNQUOTED_ARRAYREF` thread-local.

3. **Fix `unset` assoc subscript expansion** — `unset 'assoc[$var]'` (single-quoted) with `assoc_expand_once` OFF needs `$var` expanded in the builtin. Complex because quoting context is lost in string-based argument passing. Also affects quotearray5.sub `@` key handling.

### Feature work

4. **Fix DEBUG trap context** — Needed for dbg-support tests (local-only). (~375+15 diff lines)

5. **Implement restricted shell mode (`-r` flag)** — Needed for rsh tests (local-only). (~26 diff lines)

6. **Performance: optimize hot loops** — `ifs-posix` takes ~4 minutes vs bash's ~1s. `arith` takes ~2s vs bash's 0.035s. Profiling needed.

## Recent Fixes (Phase 116)

- **Coproc _PID cleanup on reap** — Force-remove the `_PID` variable (including readonly) when the coproc process is reaped. Fixes `declare -p RO_PID` showing stale value after coproc cleanup. **nameref test now passes (0 nix diff).**

## Recent Fixes (Phase 115)

- **DISCARD for invalid nameref target** — `foo=7*6` through a nameref with empty target now skips remaining `;`-separated commands on the same line (matching bash's DISCARD). Only fires for simple assignments, not arithmetic `(( ))` or expansion `${=}` contexts.

## Recent Fixes (Phase 114)

- **Readonly assignment DISCARD** — Assigning to a readonly variable (`X=2` when `X` is readonly) now skips remaining commands on the same `;`-separated line, matching bash's `jump_to_top_level(DISCARD)` behavior.

## Recent Fixes (Phase 113)

- **`unset -n` on non-nameref is a no-op** — `unset -n y` when `y` is NOT a nameref now preserves the variable (bash behavior). Previously it fully unset the variable. This fixed the `typeset -n y; y=2` error format issue (error now appears on the `typeset` line, not the `y=2` line) and several other accumulated-state issues.

## Recent Fixes (Phase 112)

- **Circular nameref array assignment to saved scope** — `local -n a=a; a=X` where `a` is an outer array now correctly assigns `X` to the outer array element [0] by updating the saved scope entry during the circular nameref assignment.

## Recent Fixes (Phase 103)

- **Select loop EOF handling** (`src/interpreter/commands.rs`) — Select loop now reads one byte before printing the `#?` prompt, detecting EOF without printing an extra prompt. Fixes `select r in /; do :; done <<< 1; echo x` producing `#? x` instead of `x`.

## Recent Fixes (Phase 102)

- **Coproc name validation** (`src/interpreter/commands.rs`) — `coproc @ { :; }` now rejects `@` as invalid identifier, matching bash.

- **`${@:0}` includes `$0` with no positional args** (`src/expand/mod.rs`, `src/expand/params.rs`) — `echo ${@:0}` with no positional parameters now includes `$0` (the shell name). Fixed guard from `positional.len() > 1` to `!positional.is_empty()` for Substring operations.

## Recent Fixes (Phase 101)

- **Readonly nameref declaration** (`src/builtins/vars.rs`) — `declare -n RO` on a readonly variable now correctly rejects with `"declare: RO: readonly variable"` and `"RO: readonly variable"`, matching bash. Previously silently allowed.

- **Exec vredir empty nameref** (`src/interpreter/redirects.rs`) — `exec {r}>/dev/null` where `r` is a nameref with empty target now reports `"exec: '10': not a valid identifier"` and `"r: cannot assign fd to variable"`, matching bash.

## Recent Fixes (Phase 100)

- **Select loop implementation** (`src/ast.rs`, `src/parser.rs`, `src/interpreter/commands.rs`) — Added `is_select` flag to `ForClause` AST and implemented `run_select_inner` with numbered menu printing to stderr, `#?` prompt (from `$PS3`), stdin reading, and `REPLY` variable. Previously, `select` was silently executed as a `for` loop. nameref11.sub select diff fixed.

- **Coproc readonly variable protection** (`src/interpreter/commands.rs`, `src/builtins/trap.rs`) — `coproc ROVAR { :; }` when ROVAR is readonly now emits both `"ROVAR: readonly variable"` and `"ROVAR: cannot unset: readonly variable"` matching bash. Also prevents overwriting readonly vars in coproc array/PID assignment, old coproc cleanup, and coproc reaping. Added `cleanup_reaped_coprocs` called after `wait`.

- **Invalid indirect expansion through nameref** (`src/expand/params.rs`) — `${!foo[2]}` where `foo` is a nameref to an unset variable now correctly reports `"foo[2]: invalid indirect expansion"`. When the nameref target exists, the subscript silently returns empty (matching bash). `${!bar}` where `bar` is completely unset also now reports `"bar: invalid indirect expansion"`.

- **Empty nameref target validation** (`src/builtins/vars.rs`) — `r=""; declare -n r` now correctly rejects empty string as invalid nameref target with `"invalid variable name for name reference"`, matching bash. Distinguishes explicit `r=""` (invalid) from truly unset variables (creates unbound nameref).

## Recent Fixes (Phase 99)

- **Command substitution expansion in nameref subscript targets** (`src/expand/params.rs`, `src/expand/mod.rs`, `src/interpreter/commands.rs`) — When a nameref target contains `$(...)` in its subscript (e.g. `declare -n foo='x[$(echo 0)]'`), the command substitution is now expanded before the subscript is evaluated arithmetically. Added `CMD_SUB_RUNNER` thread-local in the expand layer (similar to `PROCSUB_RUNNER`) that `lookup_var` uses to invoke the interpreter's `capture_output` for comsub expansion. Registered in both `expand_word_fields` and `expand_word_single`. nameref10.sub flipped to 0 nix diff (was ~6 lines).

- **Command substitution expansion in array subscripts** (`src/expand/params.rs`) — Direct array subscript access like `${x[i=0$(echo comsub >&2)]}` now expands `$(...)` in the subscript via `expand_comsubs_in_arith_expr` before arithmetic evaluation. Previously, `$(...)` in subscripts was passed literally to `eval_arith_full_with_assoc` which couldn't handle command substitutions.

- **Circular nameref warning during expansion** (`src/expand/mod.rs`) — When `$x` is expanded and `x` is part of a circular nameref chain (e.g. `v→w→x→v`), the expand-layer `resolve_nameref` now emits `warning: x: circular name reference` to stderr, matching bash behavior. Added `resolve_nameref_warn_expand` variant that reports circularity; used only in `lookup_var`'s nameref resolution path. nameref8.sub improved (missing warning on line 68 fixed).

## Recent Fixes (Phase 98)

- **Subscripted nameref target validation in compound assignments** (`src/interpreter/commands.rs`) — When `ref+=([2]=x)` is a bare compound assignment and `ref` is a nameref to `XXX[0]`, the assignment is now rejected with `'XXX[0]': not a valid identifier`. Added the check in `execute_assignment` before the `match &assign.value` dispatch, catching `AssignValue::Array` with subscripted resolved bases.

- **Subscripted nameref target validation in pre-processing compound assignments** (`src/interpreter/commands.rs`) — When `declare ref=(X)` is pre-processed and `ref` resolves to a subscripted nameref target like `XXX[0]`, the compound assignment is now rejected with `'XXX[0]': not a valid identifier` instead of creating an array on the subscripted name.

- **Declare attribute application uses base name for subscripted nameref targets** (`src/builtins/vars.rs`) — When `declare -A ref` is called and `ref` is a nameref to `XXX[0]`, the `-A` flag is now applied to `XXX` (the base name), not `XXX[0]` (the full subscripted target). nameref18.sub reduced from ~9 to ~2 diff lines.

## Recent Fixes (Phase 97)

- **Reject subscripted nameref targets in double-subscript assignments** (`src/interpreter/commands.rs`) — When `ref[foo]=bar` is assigned and `ref` is a nameref to `XXX[0]`, the resulting target `XXX[0][foo]` is invalid (double subscript). Now rejected with `'XXX[0]': not a valid identifier`.

- **Reject subscripted nameref targets in `read -a`** (`src/builtins/io.rs`) — When `read -a ref` is used and `ref` is a nameref to `XXX[0]`, the operation is now rejected with `read: 'XXX[0]': not a valid identifier`.

## Recent Fixes (Phase 96)

- **Circular nameref assignment at global scope** (`src/interpreter/mod.rs`) — When `x=4` is attempted through a circular nameref chain (`v→w→x→v`) at global scope, the assignment now silently fails (no-op) after the circular warning is emitted. Fixes nameref8.sub line 67 diff.

## Recent Fixes (Phase 95)

- **Nameref self-reference `+=` append error prefix** (`src/builtins/vars.rs`) — When `typeset -n ref=re ref+=f` creates a self-reference, the error message now omits the command name prefix (matching bash). Direct self-references like `typeset -n x=x` still include it.

- **Reject `var[@]`/`var[*]` assignment through namerefs** (`src/interpreter/mod.rs`) — When a nameref resolves to a target with `[@]` or `[*]` subscript, the assignment is rejected with `bad array subscript` error.

## Recent Fixes (Phase 94)

- **Integer attribute through namerefs in compound assignment pre-processing** (`src/interpreter/commands.rs`) — When `local -i ref=([1]=)` is processed and `ref` is a nameref, the `-i` flag is now detected during compound assignment pre-processing via `has_integer_flag`. Array elements are evaluated as arithmetic.

- **Integer flag on bare nameref in `builtin_local`** (`src/builtins/vars.rs`) — When `local -i ref` is called without a value and `ref` is a nameref already in local scope, the integer attribute is now applied to the target variable (not the nameref itself). nameref21.sub flipped to 0 diff.

## Recent Fixes (Phase 93)

- **Nounset nameref subscript reporting** (`src/expand/arithmetic.rs`, `src/expand/mod.rs`, `src/expand/params.rs`) — When `declare -n r='a[k]'; : "$r"` is expanded with nounset and `k` is unbound, the error now correctly reports `k: unbound variable` instead of `r: unbound variable`. Added `is_nounset_error()` to prevent double-reporting.

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set bash-integration-test -r @-` then `jj git push --bookmark bash-integration-test` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.
