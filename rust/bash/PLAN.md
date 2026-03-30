# Bash Test Suite — Continuation Notes

## Current State

**64/77 tests passing** on bookmark `push-nkqwvorqmnkn`. All changes committed and pushed.

### Progress This Session

- **Started at**: 64/77 (arith diff 30, heredoc diff 111, comsub-posix diff 20)
- **Now at**: 64/77 (same count — sub-tests still block nix pass)
- **heredoc**: main test 0 real diff locally ✅ (was ~20, only PID diffs remain), nix sub-tests ~85 diff
- **arith**: main test 0 diff ✅, sub-tests still have ~100 lines diff
- **comsub-posix**: 0 diff locally ✅, still fails in nix due to error message sub-tests
- **posixexp**: 2 diff locally (was 6), nix still fails on IFS/$@ issues
- **varenv**: 6 diff locally (was 36), 4 are PID diffs — only ~chet expansion remains
- **builtins**: 40 diff locally (was 93), implemented enable -n, fixed continue N, hash msg, exit, set -o, kill -l
- **trap**: flaky — 1 extra CHLD signal (non-deterministic)
- **printf**: flaky — timing-dependent date format mismatch

### Fixes Applied This Session

1. **Fix heredoc backslash handling for `\"`** (`src/lexer/heredoc.rs`) — In unquoted heredoc body parsing (`parse_double_quoted_content`), `\"` should remain literal (not strip backslash). Only `$`, `` ` ``, `\`, and `\n` are special after backslash in heredocs. Removed `'"'` from the match pattern.

2. **Fix here-string `<<<` inside `$(...)` command substitution** (`src/lexer/dollar.rs`) — Added `<<<` handler before the `<<` (heredoc) handler in the `$(...)` comsub parser. Previously, `<<<` was misrecognized: the first `<` fell through, then the remaining `<<` was parsed as a heredoc, consuming the rest of the input. Now `<<<` is passed through as three literal `<` characters and the here-string word is handled by normal comsub parsing.

3. **Fix double line counting in heredoc delimiter backslash-newline** (`src/lexer/heredoc.rs`) — In `register_heredoc`, when a `\<newline>` line continuation appears in the delimiter (e.g., `cat << EO\<NL>F`), `self.advance()` already increments `self.line` when consuming `\n`. Removed the duplicate `self.line += 1` that caused all subsequent line numbers to be off by 1.

4. **Fix `set -a` (allexport) to actually export variables** (`src/builtins/set.rs`, `src/interpreter/commands.rs`, `src/interpreter/mod.rs`) — The `-a` flag in `builtin_set` was in the "known but not fully implemented" group, silently accepted without setting `opt_allexport`. Fixed to properly set the flag. Added `'a'` to `get_opt_flags()` for `$-` reflection. Added auto-export logic in `set_var()`: when `opt_allexport` is true, newly assigned variables are automatically added to `exports`.

5. **Fix `shopt -so physical` and other options** (`src/builtins/set.rs`) — The `shopt -so` handler for set-options fell through to a default case that only updated `shopt_options` but didn't set the corresponding struct fields. Added explicit handling for `physical` (→ `opt_physical`), `hashall` (→ `opt_hashall`), `keyword` (→ `opt_keyword`), `noexec` (→ `opt_noexec`), `monitor` (→ `opt_monitor`). Also called `update_shellopts()` after `shopt -so/-uo` changes to keep `$SHELLOPTS` in sync.

6. **Fix `set -o ignoreeof` to set `IGNOREEOF=10`** (`src/builtins/set.rs`) — Separated `ignoreeof` from the compound match arm (which incorrectly set `opt_monitor` for all grouped options). Now `set -o ignoreeof` sets `IGNOREEOF=10` and `set +o ignoreeof` unsets it. Same handling added in `shopt -so/-uo ignoreeof`.

7. **Fix `set -o monitor` leaking to other options** (`src/builtins/set.rs`) — The compound match arm for `braceexpand|emacs|errtrace|functrace|histexpand|history|ignoreeof|interactive-comments|monitor` unconditionally set `shell.opt_monitor = enable`. Separated `monitor` and `ignoreeof` into their own arms. Now only `monitor` sets `opt_monitor`.

8. **Fix `set -o -B` option parsing** (`src/builtins/set.rs`) — When `set -o` is followed by an argument starting with `-` or `+` (like `-B`), it should display the option list AND then process the flag, not treat `-B` as an option name. Changed the condition to check whether the next arg starts with `-`/`+` before consuming it as an option name.

9. **Mark `SHELLOPTS` and `BASHOPTS` as readonly** (`src/interpreter/mod.rs`) — Added `readonly_vars.insert("SHELLOPTS")` and `readonly_vars.insert("BASHOPTS")` during shell initialization (after `update_shellopts()`). `update_shellopts()` uses `vars.insert()` directly (bypassing `set_var`), so it can still update the value.

10. **Fix `export` of unset variables** (`src/builtins/vars.rs`) — `export ivar` after `unset ivar` should mark the variable for export without setting a value. Previously, the code used `unwrap_or_default()` which inserted an empty string into both `exports` and effectively "set" the variable. Now, if the variable doesn't exist in `vars` or environment, it's added to `declared_unset` + `exports` without inserting into `vars`, so `${ivar-unset}` correctly expands to `unset`.

11. **Implement `local` with no arguments** (`src/builtins/vars.rs`) — `local` with no args now lists all local variables in the current scope using `declare` format, matching bash behavior. Handles scalars, indexed arrays, and associative arrays with proper flag display (`-a`, `-A`, `-i`, `-r`, `-x`).

12. **Fix `continue N` for nested loops** (`src/interpreter/commands.rs`) — `continue 2` in an inner loop should break out of the inner loop and continue the outer loop. Previously, decrementing `continuing` just continued the current loop's next iteration, causing extra iterations. Now, after decrementing, if `continuing` is still > 0, the loop `break`s to propagate to the parent. Applied to `run_for_inner`, `run_arith_for_inner`, `run_while`, and `run_until`.

13. **Fix `hash` empty message format** (`src/builtins/exec.rs`) — `hash: hash table empty` should not include the script name/line prefix. Changed from `eprintln!("{}: hash: hash table empty", shell.error_prefix())` to plain `eprintln!("hash: hash table empty")`.

14. **Fix `exit` with non-numeric argument** (`src/builtins/flow.rs`) — `exit status` (non-numeric) should print an error but NOT actually exit the shell in script mode. Changed to `return 2` instead of falling through to `std::process::exit()`.

15. **Fix `kill -l` with out-of-range signal numbers** (`src/builtins/trap.rs`) — `kill -l 4096` should report "invalid signal specification". Previously, when the number parsed successfully but wasn't found in the signal table, it silently produced no output. Added an else branch to print the error.

16. **Implement `enable -n` builtin disabling** (`src/builtins/trap.rs`, `src/interpreter/mod.rs`, `src/interpreter/commands.rs`, `src/builtins/exec.rs`) — Full implementation of `enable -n NAME` (disable builtin), `enable NAME` (re-enable), `enable -n` (list disabled), `enable -ps` (list special builtins), `enable -aps` (list all), `enable -d` (dynamic unload error). Added `disabled_builtins: HashSet<String>` to Shell struct. Builtin dispatch in `run_simple_command` now skips disabled builtins (falls through to external command lookup). `type -t` also respects disabled builtins. Reduced builtins diff from 79 to 40.

17. **Fix `enable -d` error messages** (`src/builtins/trap.rs`) — Unknown builtins get "not a shell builtin", known builtins get "not dynamically loaded".

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

#### 1. posixexp (2 lines local, 3 issues remain in nix)

Three issues in posixexp4.sub:

- **`<12>` vs `<1>\n<2>`**: `recho $a` after `IFS=; a=$@` with `set -- 1 2`. Our shell produces `<12>` (single arg), bash produces `<1><2>` (two args). Relates to how `$@` assignment interacts with null IFS — the assigned value should preserve the field boundaries for later `$a` expansion.
- **IFS splitting in `${var-$@}`**: With `IFS=:` or unset IFS, `${var-$@}` should join positional params by space (not split them individually). Our shell splits `$@` in the default word into separate fields. Two instances in posixexp4.sub. This requires `$@` in `${var-word}` to produce multiple fields, which needs refactoring of the expansion system to propagate field boundaries.

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

#### 4. heredoc (0 real lines local, ~85 lines nix)

Main `heredoc.tests` now matches perfectly (only PID diffs remain, normalized by nix). Sub-test issues:

- **heredoc3.sub**: Tab-stripped heredoc delimiter parsing, backslash continuation edge cases, `cat <<x*x` glob in delimiter, `(cat <<EOF\n...\nEOF)` — EOF not on its own line.
- **heredoc7.sub**: Command substitution interacting with heredocs — `cat << EOF)` unterminated heredoc in comsub.
- **heredoc9.sub**: `HERE; then` and `HERE; do` — heredoc delimiter followed by `;` and keyword on same line in function body printing.

#### 5. varenv (6 lines local, was 36 → was 340)

Massive improvement. Only remaining real issue:

- **`~chet` expansion**: Produces `/a/b/c` instead of `/usr/chet`. User-specific, differs by environment. The nix test also differs here.
- **Nix sub-tests**: varenv3.sub (local scoping), varenv4.sub (assoc array conversion), varenv25.sub (local -p).

#### 6. assoc (75 lines local, was 527)

Significant improvement. Remaining:

- Associative array `declare -p` key quoting differences (trailing space in `([key]="val" )`).
- `[*]` key handling — should be quoted as `["*"]`.
- Tilde expansion in associative array keys/values.
- `BASH_ALIASES` and `BASH_CMDS` arrays appearing in `declare -A` output.
- `chaff[hello world]` subscript with spaces not handled.

#### 7. builtins (40 lines local, was 93 → was 336)

Major improvement. `enable -n` now implemented. Remaining:

- `pushd`/`popd` with numeric args and error handling (~4 lines).
- `declare -p` after pre-command assignments (`foo="" export foo`) (~8 lines).
- `-printenv` error format difference (~2 lines).
- PID differences (~6 lines, normalized in nix).

#### 8. new-exp (87 lines local, was ~375)

Remaining issues:

- `${HOME-'}'}` — single quotes don't protect `}` inside `${:-}` in dquote context.
- Backtick command substitution not expanded in `${var:offset}` arithmetic.
- `${#z}` used as substring offset not evaluated to variable length.
- `set -u` / `$9` unbound variable error message format differences.
- `$((${#RECEIVED}-1))` arithmetic syntax errors.
- Various expansion edge cases.
- Note: many tests depend on `recho` which isn't available locally.

#### 9. quotearray (185 lines)

Associative array keys with special chars in arithmetic contexts.

#### 10. comsub2 (196 lines)

`local` outside function context, alias handling in subshells, function definition inside command substitution.

### Hard (200+ diff lines)

#### 11. nameref (258 lines local, was 750)

`declare -n` improvements but still substantially broken: wrong variable resolution, unset through nameref, nameref chains.

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
| `src/builtins/flow.rs` | `break`, `continue`, `exit`, `return` |
| `src/builtins/vars.rs` | `declare`, `local` (now with no-args listing), `export` (unset var handling), `let` |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting, `quote_for_declare` |
| `src/builtins/set.rs` | `set` (allexport, physical, ignoreeof), `shopt` (update_shellopts call) |
| `src/builtins/trap.rs` | `trap`, `kill` (kill -l range check), `enable` (full -n/-s/-a/-d impl) |
| `src/interpreter/mod.rs` | Shell struct, `declared_unset`, `disabled_builtins`, `run_string`, `resolve_nameref`, `set_var` (auto-export), SHELLOPTS/BASHOPTS readonly |
| `src/interpreter/commands.rs` | Command execution (disabled builtin check), `expand_word*`, `get_opt_flags` (allexport `a` flag), `update_shellopts`, `execute_assignment`, `continue N` fix |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (handles `\$` and backticks), error tokens, short-circuit assignment validation, ternary precedence |
| `src/interpreter/redirects.rs` | Redirections (vredir `{var}` fds, memfd heredocs, pipe fd leak fix) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling, `SyntaxError` handler |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` |
| `src/expand/arithmetic.rs` | `eval_arith_full`, `resolve_arith_vars` (handles `${var:-default}`) |
| `src/parser.rs` | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`) |
| `src/lexer/mod.rs` | Lexer, thread-locals (`DQUOTE_TOGGLED`) |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param`, `$(...)` comsub parser (now handles `<<<` here-strings) |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body` |
| `src/lexer/heredoc.rs` | `register_heredoc` (line count fix), `read_heredoc_bodies` (backslash-newline), `parse_double_quoted_content` (backslash fix for `\"`) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

1. **Fix `$@` expansion in `${var-$@}`** — Requires refactoring expansion to propagate field boundaries through `ParamOp::Default`. Currently `expand_word_nosplit_ctx` returns a single string, but `$@` in default words should produce multiple fields. This is architecturally hard but would fix 4 of 6 remaining posixexp nix diff lines.

2. **Fix comsub-posix error messages** — Improve error reporting for intentional syntax errors inside `$(...)` in case patterns. Need parser to detect reserved words like `done` in wrong context. (~35 nix diff lines)

3. **Fix arith10.sub array subscript quoting** — Handle `a[" "]`, `a[\ \]`, `a[\\]` in arithmetic array subscripts. (~100 nix diff lines)

4. **Fix heredoc sub-test issues** — heredoc3.sub (delimiter edge cases), heredoc7.sub (comsub+heredoc interaction), heredoc9.sub (function body printing). (~85 nix diff lines)

5. **Fix varenv nix sub-tests** — varenv3.sub (local scoping), varenv4.sub (assoc array conversion), varenv25.sub (local -p).

6. **Fix builtins remaining issues** — pushd/popd numeric args, declare -p after pre-command assignments. (~12 real diff lines)

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set push-nkqwvorqmnkn -r @-` then `jj git push --bookmark push-nkqwvorqmnkn` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.