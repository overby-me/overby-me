# Bash Test Suite — Continuation Notes

## Current State

**62/77 tests passing** on bookmark `push-nkqwvorqmnkn`. All changes committed and pushed.

## How to Run Tests

```bash
# Single nix test
nix build .#checks.x86_64-linux.rust-bash-test-NAME

# All tests, keep going on failure
nix build --keep-going .#checks.x86_64-linux.rust-bash-test-{alias,appendop,...}

# View failure diff
nix log .#checks.x86_64-linux.rust-bash-test-NAME

# Local testing (faster iteration)
cd /tmp/bash-tests/bash-5.3/tests
export PATH="/tmp/bash-tests:$PATH"
export THIS_SH=/home/noverby/Work/overby.me/rust/bash/target/debug/bash
diff <("$THIS_SH" ./NAME.tests 2>&1) <(bash ./NAME.tests 2>&1)
```

## Recent Changes (This Session)

1. **Sparse array support** — Changed `pub arrays: HashMap<String, Vec<String>>` to `Vec<Option<String>>` across the entire codebase (~96 call sites). `None` = unset slot, `Some("")` = set-to-empty. Fixed `${!A[@]}` indices, `${#A[@]}` length, `unset arr[n]`, declare -p formatting, test -v checks.

2. **IFS trailing delimiter stripping in `read`** — Matches bash's complex rules for multi-var read in `src/builtins/io.rs` L~1820-1945. Rules: single-var strips all trailing IFS; multi-var strips one trailing non-ws IFS delimiter only when remainder has no internal non-ws IFS delimiters AND no IFS-whitespace between non-IFS content.

3. **Hash table** — Added `hash_order: Vec<String>` to Shell struct for insertion-order printing. Increment hit counts in `type` builtin lookups (`src/builtins/exec.rs`).

4. **`set -P` / `set -o physical`** — Added `opt_physical` field to Shell, wired to `get_opt_flags()` and `update_shellopts()`.

5. **Function body formatting** (`src/builtins/mod.rs` L~798-840) — Fixed `ends_with_heredoc` heuristic to exclude `done`/`fi`/`esac` lines. Added `is_closing_keyword_with_heredoc` to suppress semicolons after `done`/`fi`/`esac` when the compound command contains a heredoc.

6. **PID normalization** in `rust/bash/testsuite.nix` — sed regexes normalize PIDs in temp paths and Rust panic messages.

7. **Echo EPIPE** — Currently suppressed (`src/builtins/io.rs` L~57). Causes comsub/lastpipe regressions because reference bash reports the error in some contexts.

## 15 Failing Tests

### Easiest / Highest Impact

#### 1. comsub (1 line diff)

Reference bash prints `echo: write error: Broken pipe` on line 90 of comsub.tests; our shell doesn't. The echo is inside a process substitution `<(echo a)` whose pipe reader closes early. Our shell suppresses EPIPE from echo (`src/builtins/io.rs` L~57). Bash's behavior is context-dependent: it reports EPIPE in process substitutions but not in pipeline children. Fix: report EPIPE when NOT `in_pipeline_child` (the original code before suppression). This was reverted because it caused comsub to fail. The real issue is that reference nix bash reports it but we don't — possibly our procsub doesn't set up the pipe the same way so we never get EPIPE. Check `src/expand/mod.rs` procsub handling.

#### 2. lastpipe (1 line diff)

Same EPIPE pattern. Reference bash reports Broken Pipe on line 46 of lastpipe.tests. Fix comsub EPIPE handling and this likely fixes too.

#### 3. posixexp (~10 lines)

Two issues:

- **Test environment**: `cp ${THIS_SH} $TMPDIR/sh` fails for some sub-tests. Minor.
- **Parser bug**: `echo "${foo:-"a}"` should be a parse error in non-POSIX mode. The inner `"` inside `${var:-...}` should close the outer double-quote context, but our parser in `src/lexer/dollar.rs` treats it as a nested quote inside the expansion. The fix is in the `${}` parameter expansion parser — when already inside double quotes, a `"` in the default/alt word should close the outer quote.

#### 4. arith (~27 lines locally)

Multiple issues:

- **`let` and literal `$`**: `let 'jv += $iv'` should error because `$iv` is a literal dollar sign (single-quoted). Our `expand_comsubs_in_arith` in `src/interpreter/arithmetic.rs` L~1300+ expands `$var` references but shouldn't for literal `$` from `let`. Fix: skip `$var` expansion when expression comes from `let` (check `self.arith_is_let`).
- **Short-circuit assignment**: `0 && B=42` in `$(( ))` should error "attempted assignment to non-variable" but our shell evaluates it silently. The `&&` short-circuit should not allow assignment in the false branch.
- **`--x++`**: Should error "assignment requires lvalue" but our shell handles it differently.
- **Backtick comsub**: `` `echo 1+1` `` not expanded inside `$(( ))`.
- **Error prefix**: `let:` prefix missing from some error messages. Check `arith_cmd_prefix()` in `src/interpreter/commands.rs`.

#### 5. trap (flaky, 1-3 lines)

- Extra CHLD signal (non-deterministic — sometimes passes, sometimes fails).
- xtrace formatting: `+[8] false+[8] false` (concatenated) vs `+[8] false\n+[8] false` (separate lines). Likely a stdout flushing issue in xtrace output.
- Broken Pipe error from reference bash that our shell doesn't produce.

### Medium

#### 6. varenv (~many lines in nix)

- `declare -- string=""` vs `declare -- string` — unset variables should print without `=""`.
- `local` inside function not finding variables from outer scope in some cases.
- Missing `declare -x FOOFOO` — export not propagating.
- Some SHELLOPTS/physical differences (partially fixed).

#### 7. heredoc (~48 lines)

- Tab-stripped heredocs (`<<-EOF`) not stripping tabs in some contexts.
- Backslash-newline in heredoc delimiters not handled.
- Heredoc inside eval'd function body doesn't create files properly (seen in type3.sub).

#### 8. builtins (~90 lines)

- `enable -n` not implemented — the feature to disable builtins at runtime. Would need a disabled-builtins set in Shell and checking it before builtin dispatch.
- Extra output from `export` with appended values.
- Hash table empty message format.

#### 9. assoc (~114 lines)

- Associative array formatting in `declare -p` — key quoting differences.
- `[*]` key handling.

### Hard

#### 10. quotearray (~178 lines)

Associative array keys with special chars in arithmetic contexts.

#### 11. comsub2 (~189 lines)

`local` outside function context, alias handling in subshells, function definition inside command substitution.

#### 12. comsub-posix (~33 lines in nix)

`case` statement parsing inside `$(...)` command substitutions. The parser doesn't correctly handle `case ... esac` inside `$()`. The `)` in case patterns confuses the comsub delimiter matching.

#### 13. nameref (~238 lines)

`declare -n` extensively broken: wrong variable resolution, unset through nameref, nameref chains. The nameref implementation in `src/interpreter/mod.rs` `resolve_nameref()` is too simple.

#### 14. new-exp (~719 lines)

Parser cascading failure from `${HOME'}` — single-quote inside `${...}` expansion causes a parse error that makes the parser lose track, and all subsequent commands fail. Fix parser error recovery in `src/interpreter/mod.rs` `run_string()` L~784 — `skip_to_next_command()` doesn't advance properly past certain syntax errors.

#### 15. array (~1595 lines)

Parser error recovery: `test=(first & second)` produces a syntax error (expected) but our shell aborts instead of continuing. The error token differs (`')'` vs `'&'`). After the error, `skip_to_next_command()` gets stuck and the rest of the test produces no output.

#### 16. vredir (huge output)

`{var}>file` variable fd redirection not implemented. Causes infinite loop/huge output. Implementation needed in `src/interpreter/redirects.rs` — the `{var}` syntax allocates a new fd ≥10 and stores the fd number in `var`.

## Key Source Files

| File | Contents |
|------|----------|
| `src/builtins/io.rs` | `read` (L1153-1940), `echo` (L3-89, EPIPE at L57), `printf`, `mapfile` |
| `src/builtins/exec.rs` | `type`, `command`, `hash` |
| `src/builtins/vars.rs` | `declare`, `local`, `export`, `let` |
| `src/builtins/mod.rs` | `parse_array_literal`, function body formatting (L798+) |
| `src/builtins/set.rs` | `set`, `shopt` |
| `src/interpreter/mod.rs` | Shell struct, `run_string` (error recovery L784+), `resolve_nameref` |
| `src/interpreter/commands.rs` | Command execution, `expand_word*`, `get_opt_flags`, `update_shellopts` |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (L1300+) |
| `src/interpreter/redirects.rs` | Redirections (vredir) |
| `src/interpreter/pipeline.rs` | Pipeline execution, PIPESTATUS |
| `src/expand/mod.rs` | Word expansion, `ExpCtx`, procsub handling |
| `src/expand/params.rs` | Parameter expansion (`${...}` operators) |
| `src/parser.rs` | Parser |
| `src/lexer/dollar.rs` | `${}` parsing (posixexp nested quote issue) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Priority

1. **Fix EPIPE handling** to pass comsub + lastpipe (+2 tests, net +2)
2. **Fix parser error recovery** for new-exp and array (huge line-count reduction, net +2 if both fixed)
3. **Implement `{var}` fd redirection** to fix vredir (infinite loop → potential crash, +1)
4. **Fix `declare` unset variable formatting** for varenv (many small fixes, +1)
5. **Fix `case` inside `$()` parsing** for comsub-posix (+1)
6. **Fix `let` literal `$` handling** for arith (+1)
7. **Fix posixexp nested quote parsing** (+1)

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set push-nkqwvorqmnkn -r @-` then `jj git push --bookmark push-nkqwvorqmnkn` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.