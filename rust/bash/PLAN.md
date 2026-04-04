# Bash Test Suite — Plan

## Current State

**~73/77 nix tests passing** (estimated), ~57/83 local tests passing (0 diff, sequential) on bookmark `bash-integration-test`. Goal: full drop-in bash replacement (keeping readline builtins like `compgen`/`complete` available).

See `CHANGELOG.md` for full fix history (150+ fixes across 18 phases).

### Nix test results (~73/77 passing, estimated)

Passing (~73): alias, appendop, **arith** ✅, arith-for, **array** ✅, array2, **assoc** ✅, attr, braces, **builtins** ✅, case, casemod, **comsub-eof** ✅, comsub-posix, cond, coproc, cprint, dirstack, dollars, dynvar, errors, execscript, exp-tests, exportfunc, extglob, extglob2, extglob3, func, getopts, glob-bracket, glob-test, globstar, **heredoc** ✅, herestr, ifs, ifs-posix, input-test, invert, iquote, mapfile, more-exp, **nameref** ✅, **new-exp** ✅, nquote, nquote1, nquote2, nquote3, nquote4, nquote5, parser, posix2, posixexp, posixexp2, posixpat, posixpipe, precedence, printf, procsub, quote, ~~quotearray~~, read, redir, rhs-exp, set-e, set-x, shopt, strip, test, tilde, tilde2, type, **varenv** ✅, vredir

**Phase 18 fixes:** xtrace atomic writes (pipeline interleaving fix), funsub `set -e` disabled in non-posix mode, bad interpreter shebang error messages, `${scalar[@]:offset:length}` character-level substring

**Phase 17 flipped to passing:** comsub-eof (1→0 diff, incomplete comsub detection fix + heredoc EOF warning on parse errors), heredoc3.sub (1→0 diff, subshell EOF error reporting)

**Phase 16 flipped to passing:** arith (~90→0 diff, duplicate error fix + subscript quote handling), array (~5→0 diff, subscript error ordering + brace expansion + bad subscript error format), varenv (~6→PID-only, set -k expansion ordering), builtins (~18→PID-only), heredoc (~12→PID-only)

**Phase 15 flipped to passing:** assoc (462→0 diff), quotearray (179→0 diff, from IFS fix), new-exp (310→0 diff), nameref (678→PID-only diff), trap (1→0 locally, may still be flaky in nix)

Failing (~4):

| Test | Nix diff lines | Notes |
|------|---------------|-------|
| trap | 1 | Flaky — timing-dependent signal delivery (extra CHLD) |
| comsub | 1 | Spurious `echo: write error: Broken pipe` (flaky timing) |
| lastpipe | 1 | Spurious `echo: write error: Broken pipe` (flaky timing) |
| set-e | 1 | Spurious `echo: write error: Broken pipe` (flaky timing) |
| heredoc | ~4 | heredoc7.sub case 2: line number off-by-1 in comsub+heredoc interaction |
| comsub2 | ~20 | Line number off-by-1 in funsubs + missing `jobs` output + funsub `$*` ordering |
| quotearray | ~200 | Arithmetic eval of quoted assoc array subscripts (`(( assoc['key']++ ))`) |

**Note on quotearray:** Previously passing in Phase 15 (IFS fix), but now failing due to arithmetic evaluation of single-quoted associative array subscripts inside `(( ... ))`. The bash 5.3 test suite requires `(( assoc['x],b[$(echo uname >&2)']++ ))` to treat the single-quoted key as a literal subscript. Our arithmetic evaluator mishandles the quote parsing.

### Local test results (~57/83 passing, 0 diff sequential)

83 total `.tests` files in `/tmp/bash-5.3/tests/` (superset of the 77 nix tests — includes dbg-support, dbg-support2, dstack2, histexp, history, rsh, invocation, jobs, posixpipe, and others not in the nix harness). **dstack2** now passes (was 26 diff lines — `~N`/`~+N`/`~-N` tilde expansion implemented).

**Important:** Many tests that show diffs when run in parallel (`diff <(our_bash test) <(bash test)`) pass when run sequentially due to race conditions on shared `/tmp` and `/var/tmp` files. Tests like `globstar`, `test`, `redir`, `extglob` pass when run sequentially. Use sequential mode for accurate results:

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

## Failing Nix Tests (~4/77)

### Near-passing (1-line diffs, likely flaky)

- **trap** (1 line) — Timing-dependent signal delivery (extra CHLD)
- **comsub** (1 line) — Spurious `echo: write error: Broken pipe` (SIGPIPE timing race in nix sandbox)
- **lastpipe** (1 line) — Spurious `echo: write error: Broken pipe` (SIGPIPE timing race in nix sandbox)

### Medium diffs

- **heredoc** (~4 lines) — heredoc7.sub case 2: heredoc started outside comsub where delimiter overlaps comsub body (complex parser interaction). Case 1 (`$(cat << EOF)`) now fixed. Main test PID-only.
- **comsub2** (184 lines) — `${ ... }` dollar-brace comsub (bash 5.3 feature)

### Now passing (Phase 17 fixed)

- **~~comsub-eof~~** (1→0 lines) — Fixed incomplete comsub detection: use `result.incomplete` flag instead of `!remaining.contains(')')` heuristic (heredoc body containing `)` caused false positive). Also emit heredoc EOF warnings before syntax error messages. ✅
- **~~heredoc3.sub~~** (1→0 lines) — Fixed `(cat <<EOF ... EOF)` subshell EOF error reporting: emit "unexpected end of file from `(' command on line N" with proper line number when subshell hits EOF. ✅
- **~~heredoc7.sub case 1~~** (~3→0 lines) — Fixed `echo $(cat << EOF)` where heredoc inside comsub has body in outer context: `find_comsub_boundary` now reads the heredoc body from chars after `)` and embeds it in the comsub text. Emits "command substitution: N unterminated here-document" warning. ✅

### Now passing (Phase 16 fixed)

- **~~arith~~** (~90→0 lines) — Fixed duplicate arith error on `a[b[c]d]=e`, subscript `"` handling in lexer ✅
- **~~array~~** (~5→0 lines) — Fixed negative subscript error ordering, brace expansion in subscript, bad subscript error formats ✅
- **~~varenv~~** (~6→PID-only) — Fixed `set -k` keyword assignment expansion ordering ✅
- **~~builtins~~** (~18→PID-only) — All real diffs fixed, only PID noise remains ✅
- **~~heredoc~~** (~12→PID-only) — Main test now PID-only (sub-test diffs remain) ✅

### Now passing (Phase 15 fixed)

- **~~new-exp~~** (310→0 lines) — All expansion edge cases now pass ✅
- **~~assoc~~** (462→0 lines) — All associative array tests pass ✅
- **~~nameref~~** (678→PID-only) — All nameref resolution tests pass ✅
- **~~quotearray~~** (179→0 lines) — Fixed by IFS empty-string handling ✅

### Now passing (Phase 17 fixed, from above)

- **~~comsub-eof~~** — See above ✅

### Local-only failing tests (not in nix harness)

These exist in `/tmp/bash-5.3/tests/` but not in the nix test list:

- **dbg-support** (375 lines) — DEBUG trap, `caller` builtin, BASH_SOURCE/FUNCNAME tracking
- **dbg-support2** (15 lines) — DEBUG trap line number tracking
- **~~dstack2~~** (26→0 lines) — Fixed: `~0`, `~1`, `~-1` tilde expansion for directory stack ✅
- **histexp** (203 lines) — History expansion not implemented
- **history** (179 lines) — History builtin not fully implemented
- **rsh** (26 lines) — Restricted shell mode (`-r` flag) not implemented
- **invocation** (~10 lines) — PID diffs + bad interpreter error prefix format (partially fixed in Phase 18)
- **complete** (116 lines) — Readline-specific completion diffs (local non-readline bash lacks compgen)
- **jobs** (0 lines) — Now passes locally ✅

## Key Source Files

| File | Contents |
|------|----------|
| `src/ast.rs` | AST types, `WordPart` (includes `SyntaxError` variant) |
| `src/builtins/io.rs` | `read` (prompt suppression on non-tty), `echo` (EPIPE handling), `printf`, `mapfile` |
| `src/builtins/exec.rs` | `type`, `command`, `hash` |
| `src/builtins/flow.rs` | `break`, `continue`, `exit`, `return` |
| `src/builtins/vars.rs` | `declare` (compound re-expansion, `+a` readonly fix), `local`, `export`, `let`, `unset` (scalar subscript error, `arr[@]` preserves empty array) |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting, `quote_for_declare`, `quote_assoc_key`, `interpret_echo_escapes` (returns `(String, bool)` for `\c` stop) |
| `src/builtins/set.rs` | `set` (allexport, physical, ignoreeof), `shopt` (update_shellopts call, readline options removed) |
| `src/builtins/trap.rs` | `trap`, `kill` (kill -l range check), `enable` (full -n/-s/-a/-d impl) |
| `src/interpreter/mod.rs` | Shell struct, `declared_unset`, `disabled_builtins`, `source_set_params`, `run_string`, `resolve_nameref`, `set_var` (auto-export), SHELLOPTS/BASHOPTS readonly, BASH_ALIASES/BASH_CMDS init |
| `src/interpreter/commands.rs` | Command execution, `expand_word*`, `set -k` keyword assignment scoping (save/restore), inline compound assignment detection (SingleQuoted `(` support), `execute_assignment` |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (handles `\$` and backticks), error tokens, short-circuit assignment validation, ternary precedence |
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds with nameref support, varredir_close, fd validation, memfd heredocs, pipe fd leak fix) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS, `in_pipeline_child` always true for forked children, SIGPIPE reset to SIG_DFL in pipeline/comsub children |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, `ifs_first_char()` helper (empty IFS handling), procsub handling, `SyntaxError` handler, `NOUNSET_ERROR` flag, empty-element removal in unquoted `${arr[@]%%pattern}` |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), IFS-aware `${arr[*]}` joining, `parse_arith_offset`, `is_valid_var_ref`, negative subscript bounds checking, assoc subscript expansion |
| `src/expand/pattern.rs` | Pattern matching, `pattern_replace` (handles empty value + `*` match) |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`) |
| `src/lexer/mod.rs` | Lexer, `lex_compound_array_content()` (full-quoting re-parser for `declare -a`), thread-locals (`DQUOTE_TOGGLED`), `force_read_pending_heredocs`, `heredoc_resume` |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param` (bad substitution for `${$(...)}` ), `$(...)` comsub parser (now handles `<<<` here-strings) |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body` |
| `src/lexer/heredoc.rs` | `register_heredoc` (line count fix), `read_heredoc_bodies` (backslash-newline, `<<-` tab-stripped delimiter matching), `parse_double_quoted_content` (backslash fix for `\"`) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

### Low-hanging fruit (could flip nix tests to passing)

1. **Fix SIGPIPE flaky tests (comsub/lastpipe/trap/set-e)** — 1-line diff each, timing race in nix sandbox. SIGPIPE is reset to SIG_DFL in pipeline/comsub children and EPIPE is suppressed in echo builtin for all subprocess contexts, but the nix sandbox timing still occasionally triggers the race. trap has an extra CHLD signal delivery. printf also has a flaky SIGPIPE race (printf6.sub line 40).

2. **Fix quotearray regression** — Arithmetic evaluation of single-quoted associative array subscripts inside `(( ... ))`. Need to handle `(( assoc['key']++ ))` where the single-quoted content should be used as a literal subscript key. (~200 diff lines)

### Feature work

3. **Fix remaining heredoc7.sub case 2** — heredoc started outside comsub (`cat <<EOF && grep $(`) where the heredoc delimiter `EOF` appears on a line consumed by the comsub body. Line numbers off by 1. (~4 nix diff lines)

4. **Fix comsub2 remaining diffs** — (a) funsub `$*` ordering issue: `"$*${ set -- a b c;}$*"` should see updated positional params for the second `$*` — requires expansion layer to re-read shell state after funsub callback; (b) `jobs` builtin stub needs real job table access in funsubs; (c) line number off-by-1 in multi-line funsubs. (~20 nix diff lines)

5. **Implement `caller` builtin and fix DEBUG trap context** — Needed for dbg-support tests (local-only). (~375+15 diff lines)

6. **Implement restricted shell mode (`-r` flag)** — Needed for rsh tests (local-only). (~26 diff lines)

7. **Performance: optimize hot loops** — `ifs-posix` takes ~4 minutes vs bash's ~1s. `arith` takes ~2s vs bash's 0.035s. Profiling needed.

## Recent Fixes (Phase 18)

- **Fix xtrace interleaving in pipelines** — Pipeline children writing xtrace output to stderr could interleave because `writeln!` splits into two `write()` syscalls (message + newline). Changed `xtrace_write` to use a single `write_all()` call with the newline pre-appended, ensuring atomic output. Also flush stderr before fork. Fixes `PS4='+[$LINENO] '; set -x; false | false | false` showing `+[8] false+[8] false` on one line.
- **Disable `set -e` inside funsubs (non-posix mode)** — Bash disables `set -e` (errexit) inside `${ ... }` nofork command substitutions in non-posix mode, matching regular command substitution behavior. In posix mode, `set -e` still propagates. Applied to both `capture_output_nofork` (funsub) and `capture_valuesub` (valuesub). Fixed **comsub22.sub** (`set -e` + funsub + `false` test).
- **Detect bad interpreter shebang error** — When exec fails with ENOENT for a file that exists (bad interpreter in shebang), read the `#!` line and report `script: interp: bad interpreter: No such file or directory` matching bash's error format. Previously reported just `No such file or directory`.
- **Fix `${scalar[@]:offset:length}` substring** — When a scalar variable is accessed with `[@]` subscript and a `:offset:length` operation, perform character-level substring (same as `${var:offset:length}`) instead of returning empty for offset > 0. Fixed in both `expand_param` and `get_array_elements`. Fixed **new-exp** test (18→PID-only diff).

## Recent Fixes (Phase 17)

- **Fix incomplete comsub detection for heredoc-containing command substitutions** — When `$(cat <<EOF ... EOF)` had a heredoc that consumed the `)` character as body text (because EOF wasn't found), `parse_comsub` correctly returned `Incomplete` but `parse_dollar` failed to detect it. The heuristic `!remaining.contains(')')` was wrong when `)` existed inside the heredoc body. Added `incomplete` field to `ComsubParseResult` and use it directly instead of the heuristic. Fixed **comsub-eof** nix test (1→0 diff).
- **Emit heredoc EOF warnings before syntax error messages** — When a parse error occurs (e.g. unmatched `(`), any accumulated heredoc EOF warnings are now printed before the error message, matching bash's output ordering.
- **Fix subshell EOF error reporting** — `(cat <<EOF\nbody\nEOF)` where `EOF)` is not the delimiter now correctly reports: (1) heredoc-terminated-by-EOF warning, (2) `syntax error: unexpected end of file from '(' command on line N` with proper line number. Previously reported only `expected ')'` without warnings. Fixed **heredoc3.sub** (1→0 diff).
- **Handle "unexpected end of file" errors with line numbers** — EOF errors from unclosed compound commands now include `line N:` in the prefix even in non-script non-dash_c mode, matching bash.
- **Handle heredoc inside command substitution — read body from outer context** — `echo $(cat << EOF)` where `<<EOF` is inside the comsub but the heredoc body follows in the outer script now works correctly. In `find_comsub_boundary`, when `)` closes the comsub at depth 1 with a pending `<<DELIM`, the heredoc body is read from the remaining chars after `)` and embedded in the comsub text. The "command substitution: N unterminated here-document" warning is emitted via a `\x00COMSUB_UNTERMINATED:N` sentinel in `heredoc_eof_warnings`. Fixed **heredoc7.sub case 1** (~3→0 diff). Reduced **heredoc** nix diff from ~7→~4 lines.

## Recent Fixes (Phase 16)

- **Fix `set -k` keyword assignment expansion ordering** — When `set -k` is active, keyword-looking words are now identified in the AST BEFORE expansion. If no command word remains after extraction (all words are assignments), keyword assignments are expanded AFTER prefix assignments are applied, so `HOME=/a/b/c $EMPTY a=$HOME` correctly gives `a` the new HOME value. Reduced **varenv** from ~6→PID-only diff.
- **Fix array negative subscript error ordering** — For `c[-2]=4` on a readonly array with out-of-bounds negative index, the "bad array subscript" error is now reported BEFORE the "readonly variable" error (matching bash). Added pre-check in `execute_assignment` before the readonly guard.
- **Implement brace expansion in array subscripts** — `"${letters["{2..6}"]}"` now correctly expands `{2..6}` to indices 2,3,4,5,6 and looks up each array element. Fixed lexer to handle `"` quote-toggling inside `[...]` subscripts of `${arr[...]}`, and added brace expansion detection in `lookup_var`.
- **Fix bad array subscript error formats** — `${arr[-N]}` (value access) now uses `arr: bad array subscript` format; `${#arr[-N]}` (length) uses `[-N]: bad array subscript` format. Added `BAD_SUBSCRIPT` thread-local flag to prevent duplicate errors without aborting commands (bash prints the error but still runs the command with empty expansion).
- **Fix duplicate arith error on `a[b[c]d]=e`** — The pre-check for negative subscripts in `execute_assignment` now checks for arith_error after evaluating the subscript and returns early, preventing the duplicate error message.
- **Implement `~N`/`~+N`/`~-N` tilde expansion** — Directory stack indices in tilde expansion now look up `DIRSTACK[N]` (from top) or `DIRSTACK[len-1-N]` (from bottom). Fixed **dstack2** test (26→0 diff lines).
- **Fix `dirs -v` with positional index** — `dirs -v -1` now shows the index number prefix (e.g. `1  /usr`) matching bash's format.
- **Make `brace_expand` pub(crate)** — Exposed for use in array subscript expansion from `expand/params.rs`.

## Recent Fixes (Phase 15)

- **IFS empty-string handling: fix `"${a[*]}"` join separator** — When IFS is set to empty string (`IFS=""`), `"${a[*]}"` and `"$*"` now correctly join elements with no separator (was using space). Fixed 6 occurrences in `expand/mod.rs` using new `ifs_first_char()` helper, plus all `join(" ")` calls in `expand/params.rs` (`lookup_var` and `expand_param`) to use IFS-aware joining. This fixed **quotearray** (179→0), **assoc** (462→0), **globstar** (275→~84), and the `55 vs 49` array diff.
- **`set -k` keyword assignment scoping** — Keyword assignments extracted by `set -k` are now temporary when there's a command to run (save/restore pattern), and only persist when no command remains after extraction. Moved keyword extraction BEFORE the permanent-assignment decision so that `a=5 b=6 $EMPTY c=7 $EMPTY d=8` correctly persists all assignments when command words are empty. Fixed `|| self.opt_keyword` condition that caused prefix assignments to persist even with commands. Reduced **varenv** from 320→~6 diff lines.
- **`read -p` prompt suppression** — The `-p prompt` option now only prints the prompt when the input fd is a terminal (matching bash). Fixes the `array test:` prefix appearing in heredoc-fed `read` output.
- **`unset scalar[n]` error** — `unset var[n]` where `var` is a scalar (not an array) and `n != 0` now correctly errors with "not an array variable". `unset var[0]` unsets the scalar. Completely unset variables with subscript are silently ignored (matching bash).
- **`unset arr[@]`/`arr[*]` preserves array** — Now clears all elements but keeps the array variable (as an empty array `()`), matching bash. Previously removed the entire array. Fixes missing `declare -a e=()` in array test output.
- **`declare +a` on readonly array** — Error message now correctly says "readonly variable" (taking precedence) instead of "cannot destroy array variables in this way".
- **Negative array subscript bounds checking** — `${arr[-N]}` where N exceeds the array length now correctly errors with "bad array subscript" instead of silently returning element 0.
- **`declare -a f='("${d[@]}")'` compound re-expansion** — Single-quoted compound array assignments containing `$`-expansions are now properly re-parsed with full shell quoting (double quotes, single quotes, `$`-expansions) and expanded. Added `lex_compound_array_content()` to the lexer for proper re-parsing. Also fixed inline compound assignment detection to recognize `(` from `SingleQuoted` word parts. Reduced **array** from 647→~5 diff lines.
- **Empty array element removal in unquoted context** — `${arr[@]%%pattern}` where an element becomes empty after pattern removal now correctly drops the empty element in unquoted context (matching bash word splitting). Fixes the double-space issue in path expansion.

## Recent Fixes (Phase 14)

- **builtins: 275 → ~170 diff lines** (~100 lines fixed)
  - Fix `exec -a specialname`: resolve executable path BEFORE clearing env with `-c`, and fix `$0` to use `argv[0]` when `-c` is used without explicit arg0 (e.g. `exec -a specialname bash -c 'echo $0'` now correctly outputs `specialname`)
  - Fix `source`/`.` error message format: in POSIX mode with bare names (no `/`), use `. notthere: file not found` format; for paths or non-POSIX mode, use `notthere: No such file or directory` format
  - Fix `hash -lt` combined flags: rewrite hash option parsing to handle combined flags like `-lt` (was rejected as invalid option); also fix `-d` to report "not found" for non-existent entries, `-p /dir` to report "Is a directory"
  - Fix `pushd`/`popd`/`dirs` `--` handling: all three builtins now correctly handle `--` to terminate option processing; `popd --` ignores subsequent args (matching bash); `popd dir` now reports "invalid argument"
  - Fix `echo` with `xpg_echo` shopt: when `shopt -s xpg_echo` or POSIX mode is active, `echo` interprets escape sequences by default; only `-n` is recognized as a flag (not `-e`/`-E`)
  - Fix `cd` with CDPATH: implement CDPATH search for relative directory names (not starting with `/`, `./`, `../`); prints new directory when found via non-current-dir CDPATH entry
  - Fix `unset name` (without `-f`/`-v`): now falls through to unset functions when no variable by that name exists; with explicit `-v`, only variables are targeted (no function fallback)
  - Fix `declare -f name` vs `declare -f -p name`: only print "not found" error when `-p` flag is present; plain `declare -f name` silently returns 1 for missing functions
- **comsub-eof: regression from Phase 13** — 1-line diff in comsub-eof3.sub heredoc error line (line 5 vs 8), caused by Phase 13 heredoc changes
- **No other regressions** — all 63 previously-passing nix tests still pass (excluding comsub-eof which regressed in Phase 13)

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