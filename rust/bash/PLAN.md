# Bash Test Suite — Continuation Notes

## Current State

**64/77 tests passing** in nix on bookmark `push-nkqwvorqmnkn`. All changes committed and pushed. Gained comsub, lastpipe, func, shopt, extglob, procsub as new nix passes; builtins/arith/heredoc/assoc regressed due to sub-test issues (pushd/popd, arith10.sub, heredoc3/7/9.sub, BASH_CMDS).

### Progress This Session (Latest)

- **new-exp**: PID diffs only (was 7 real diff lines). **Both remaining real issues FIXED** ✅
  - Fixed backtick command substitution expansion in `parse_arith_offset` — `${HOME:\`echo }\`}` now correctly expands backtick before arithmetic evaluation
  - Fixed complex array subscript arithmetic — `${_ENV[(_$-=0)+(_=1)-_${-%%*i*}]}` now evaluates correctly using `eval_arith_full` instead of naive `.parse::<i64>()`
  - Fixed `resolve_arith_vars` to handle special parameter names (`$-`, `$?`, `$#`, etc.) in `${...}` syntax
  - Added `%%`, `%`, `##`, `#` trim operator support to `resolve_arith_vars`
  - Fixed `$-` expansion in `resolve_arith_vars` to return actual shell option flags (was returning empty string)
  - Threaded `opt_flags` through `eval_arith_full` and `resolve_arith_vars`
  - Fixed bare variable name resolution: names followed by `$` or `=` stay literal (not eagerly resolved)
  - Added variable assignment expression support (`var=expr`, `var+=expr`) to `eval_arith`
  - Added bare identifier handling in `eval_arith` (evaluates to 0 for unset variables)
- **assoc**: 0 real diff locally ✅ (timing diffs only, same as before)
- **nameref**: 266 diff locally (was 260 baseline, +6 cosmetic — `nameref10.sub` vs `./nameref10.sub` path prefix)
- **array**: 425 diff locally (unchanged)

### Progress Previous Session

- **assoc**: 0 real diff locally (was 2). Only timing diff remains ✅
  - Fixed `${#wheat[$unset]}` — empty subscript after expansion now emits "bad array subscript" error for assoc arrays
  - Fixed duplicate error messages from `lookup_var` called twice in `expand_part`
  - Added `get_arith_error()` peek function to check error flag without consuming it
- **new-exp**: 7 diff locally (was 60), ~2 real lines. **Massive improvement — 53 lines reduced**
  - Fixed `${!v}` indirect expansion with invalid variable name (`bad-var: invalid variable name`)
  - Fixed `${6="arg6"}` — cannot assign to positional/special params error
  - Fixed `${var/*/x}` with empty `var` — pattern `*` now matches empty string in `pattern_replace`
  - Fixed `${@:offset:length}` negative length error — now emits `substring expression < 0`
  - Fixed `${$(($#-1))}` — `bad substitution` error for `$` followed by `(` in `${...}`
  - Fixed `parse_arith_offset` to handle `$((...))` arithmetic expansion in substring offsets
- **posixexp**: 0 diff locally ✅ (was 3)
- **shopt**: 0 diff locally ✅ (was 68)
  - Removed 10 readline-only shopt options from listing (`complete_fullquote`, `direxpand`, etc.)
  - Removed `emacs` and `vi` from `set -o` options (readline-dependent)
- **comsub**: 0 diff locally ✅ (was 2)
  - Fixed SIGPIPE handling in process substitution children
- **lastpipe**: 0 diff locally ✅ (was 2)
  - Fixed `in_pipeline_child` — all forked pipeline commands are children regardless of lastpipe
- **procsub**: PID diffs only (was 13, now 12) — should pass in nix ✅
- **varenv**: 18 diff locally (unchanged = ~chet + PID diffs)
- **nameref**: 252 diff locally (was 248)
- **array**: 425 diff locally (was 424)

### Progress Two Sessions Ago

- **assoc**: 2 diff locally (was 65). **Massive improvement — 63 lines reduced**
  - Fixed `declare -Ai` arithmetic evaluation for assoc array compound assignments
  - Fixed bare values in assoc compound assignment to error ("must use subscript")
  - Fixed `declare fluff[qux]=assigned` — subscripted names in declare for assoc/indexed arrays
  - Fixed `declare -p` to show `-Ai` and `-Ar` flags for assoc arrays (all output paths)
  - Fixed `declare +A chaff` — "cannot destroy array variables in this way" error
  - Fixed `declare +i`, `+x`, `+u`, `+l`, `+c`, `+t` to actually unset variable attributes
  - Fixed `declare -A chaff[200]` — strip subscript from name when `-A`/`-a` flag set
  - Fixed readonly error message to show base name (`waste` not `waste[stuff]`)
  - Fixed compound assignment with spaced keys (`wheat=([foo bar]="qux qix")`) — parser merges tokens until `]=` found
  - Fixed compound assignment with quoted keys (`hash=(["key"]="value")`) — parser walks across WordParts
  - Fixed element-level `+=` in assoc compound assignments (`assoc+=([one]+=more)`)
  - Fixed scalar assignment to assoc array — assigns to element `[0]`
  - Fixed scalar-to-assoc conversion (`assoc=assoc; declare -A assoc` → `[0]="assoc"`)
  - Fixed `${xpath["0"]}` — strip surrounding quotes from assoc array subscript keys
  - Fixed `chaff[hello world]=flip` at command level — parser multi-token bracket merge
  - Fixed subscripted append to check assoc arrays first (string key, not arithmetic eval)
  - Fixed `declare -A` listing mode to include `i` and `r` flags
  - Fixed `parse_assoc_literal` to use full value when `\x1F` separators present
  - Fixed arithmetic panic on unclosed `[` expressions (e.g., `[foo` with no `]`)
- **posixexp**: 3 diff locally (was 6). Improved by 3 lines
- **varenv**: 14 diff locally (was 18). Improved by 4 lines
- **nameref**: 248 diff locally (was 258). Improved by 10 lines
- **array**: 424 diff locally (was 446). Improved by 22 lines
- **arith**: 0 diff ✅
- **builtins**: 18 diff locally (PID only) ✅
- **heredoc**: 8 diff locally (PID diffs + sub-tests)
- **comsub-posix**: 0 diff locally ✅

### Progress Three Sessions Ago

- **builtins**: 18 diff locally → **all PID diffs** (was 40). Should now pass in nix ✅
  - Fixed `exec -c` (clear env), `exec -l` (login shell argv[0] prefix)
  - Fixed `foo="" export foo` prefix assignment persistence
  - Fixed `FOO='$$' declare -p FOO` showing `-x` flag and proper `\$\$` quoting
  - Fixed POSIX special builtins list (added `:` and `times`)
  - Fixed source/dot with args: `set --` in sourced file now persists
- **new-exp**: 60 diff locally (was 87). Improved by 27 lines
  - Fixed `echo -e "\c"` to suppress trailing newline
  - Fixed `set -u` nounset for positional params (`$9: unbound variable`)
  - Fixed nounset errors to exit shell/subshell (no more "after N" continuation)
- **varenv**: 18 diff locally (was 6 real + PID). ~chet + PID diffs only
- **assoc**: 65 diff locally (was 75). Fixed declare -p formatting, BASH_ALIASES/BASH_CMDS, key quoting
- **arith**: 0 diff ✅
- **heredoc**: 8 diff locally (PID diffs + sub-tests)
- **comsub-posix**: 0 diff locally ✅

### Progress Four Sessions Ago

- **Started at**: 64/77 (arith diff 30, heredoc diff 111, comsub-posix diff 20)
- **heredoc**: main test 0 real diff locally ✅ (was ~20, only PID diffs remain), nix sub-tests ~85 diff
- **arith**: main test 0 diff ✅, sub-tests still have ~100 lines diff
- **comsub-posix**: 0 diff locally ✅, still fails in nix due to error message sub-tests
- **posixexp**: 2 diff locally (was 6), nix still fails on IFS/$@ issues
- **trap**: flaky — 1 extra CHLD signal (non-deterministic)
- **printf**: flaky — timing-dependent date format mismatch

### Fixes Applied This Session (Latest)

57. **Add backtick expansion in `parse_arith_offset`** (`src/expand/params.rs`) — Added `expand_backticks_in_str()` helper that scans for `` `cmd` `` patterns and executes them via `cmd_sub`. `parse_arith_offset` now accepts `cmd_sub: CmdSubFn` and pre-expands backticks before arithmetic evaluation. Fixes `${HOME:\`echo }\`}` showing raw backtick in error message.

58. **Use `eval_arith_full` for indexed array subscript evaluation** (`src/expand/params.rs`) — Replaced naive `expanded_idx.parse::<i64>().unwrap_or(0)` with full arithmetic evaluation via `eval_arith_full` for complex subscript expressions. Removed crude `$var`/`${var}` inline expansion in `lookup_var` (was unable to handle `$-`, `${-%%*i*}`, etc.).

59. **Fix `resolve_arith_vars` special parameter handling** (`src/expand/arithmetic.rs`) — Added support for special parameter names (`-`, `?`, `#`, `!`, `$`, `@`, `*`) in `${...}` syntax inside arithmetic expressions. Previously only alphanumeric/underscore names were recognized, so `${-%%*i*}` parsed incorrectly.

60. **Add `%%`/`%`/`##`/`#` trim operators to `resolve_arith_vars`** (`src/expand/arithmetic.rs`) — `${var%%pat}`, `${var%pat}`, `${var##pat}`, `${var#pat}` now work inside arithmetic expressions. Uses `crate::expand::pattern::trim_pattern` for pattern matching.

61. **Fix `$-` expansion in arithmetic context** (`src/expand/arithmetic.rs`) — `$-` in `resolve_arith_vars` now returns actual shell option flags via `opt_flags` parameter instead of hardcoded empty string. Added `opt_flags: &str` parameter to `eval_arith_full` and `resolve_arith_vars`. Fixed both `ctx_dummy` instances to pass through `opt_flags`.

62. **Fix bare variable names before `$` and `=` in arithmetic** (`src/expand/arithmetic.rs`) — When a bare variable name (e.g., `_`) is immediately followed by `$`, it's kept literal so the `$` expansion concatenates to form a new name (e.g., `_$-` → `_hB`). When followed by `=` (but not `==`), or compound assignment operators (`+=`, `-=`, etc.), the name is also kept literal since it's an assignment target, not a value to resolve.

63. **Add variable assignment support to `eval_arith`** (`src/expand/arithmetic.rs`) — `eval_arith` now handles `var=expr` and `var+=expr` (and other compound assignments) by evaluating the RHS and returning its value. Cannot actually store the assignment (no `&mut vars`), but produces correct expression results.

64. **Handle bare identifier names in `eval_arith`** (`src/expand/arithmetic.rs`) — Valid identifier names that remain after `resolve_arith_vars` expansion (e.g., `_hB` from `_${-%%*i*}`) now evaluate to `0` (unset variable default) instead of causing a parse error.

### Fixes Applied Two Sessions Ago

47. **Fix `${#wheat[$unset]}` bad array subscript for assoc arrays** (`src/expand/params.rs`) — When an associative array subscript expands to empty (e.g., `$unset` is not set), now emits `[raw_subscript]: bad array subscript` error. Added `$` variable expansion in assoc array subscript keys inside `lookup_var`. Added `get_arith_error()` peek function to avoid duplicate errors when `expand_part` and `expand_param` both call `lookup_var`.

48. **Fix `${!v}` indirect expansion with invalid variable name** (`src/expand/params.rs`) — Added `is_valid_var_ref()` helper that validates variable names (special params, positional, arrays, identifiers). `${!v}` where `v=bad-var` now emits `bad-var: invalid variable name` error instead of silently returning empty.

49. **Fix `${6="arg6"}` assignment to positional/special params** (`src/expand/params.rs`) — `ParamOp::Assign` now checks if `expr.name` is a positional parameter or special parameter and emits `$6: cannot assign in this way` error, matching bash behavior.

50. **Fix `${var/*/x}` with empty `var`** (`src/expand/pattern.rs`) — `pattern_replace` now checks after the main loop if the value is empty and the pattern matches empty string (`shell_pattern_match("", pattern)`), and if so, appends the replacement. This fixes `*` matching empty strings.

51. **Fix `${@:offset:length}` negative length error** (`src/expand/params.rs`) — For `$@`/`$*` substring operations, negative length now emits `{len_str}: substring expression < 0` error instead of silently clamping. Also changed offset/length parsing to use `parse_arith_offset` instead of `.trim().parse().unwrap_or()` for proper arithmetic evaluation.

52. **Fix `parse_arith_offset` to handle `$((...))` expansion** (`src/expand/params.rs`) — Added early detection for `$((expr))` syntax: strips outer delimiters and evaluates via `eval_arith_full`. Previously `$(($# - 2))` would fail integer parse and default to wrong value.

53. **Fix `${$(($#-1))}` bad substitution** (`src/lexer/dollar.rs`) — When `parse_brace_param` encounters param name `$` followed by `(`, scans to closing `}` and returns `WordPart::BadSubstitution`. Previously parsed as `$$` (PID) followed by operator recovery.

54. **Fix SIGPIPE handling in process substitution** (`src/expand/mod.rs`) — Reset `SIGPIPE` to `SIG_DFL` in process substitution child before running inline procsub. Previously `echo` inside `<(echo a)` would get "write error: Broken pipe" instead of being silently killed.

55. **Fix `in_pipeline_child` for forked pipeline commands** (`src/interpreter/pipeline.rs`) — Changed `self.in_pipeline_child = !self.shopt_lastpipe` to `self.in_pipeline_child = true` in the fork child. All forked pipeline commands are children regardless of `lastpipe` setting. Fixed `echo g h i | bar=7` producing spurious "Broken pipe" error.

56. **Remove readline-only shopt options** (`src/builtins/set.rs`) — Removed 10 options from shopt listing: `complete_fullquote`, `direxpand`, `dirspell`, `force_fignore`, `histreedit`, `histverify`, `hostcomplete`, `no_empty_cmd_completion`, `progcomp`, `progcomp_alias`. Also removed `emacs` and `vi` from `set -o` options. These require readline/completion support not present in our build.

### Fixes Applied Three Sessions Ago

31. **Fix `declare -Ai` arithmetic evaluation for assoc arrays** (`src/interpreter/commands.rs`) — Compound assignment to assoc arrays with `-i` flag now evaluates values as arithmetic (e.g., `[zero]=1+4` → `5`). Also handles element-level `+=` for assoc compound assignments.

32. **Error for bare values in assoc compound assignment** (`src/interpreter/commands.rs`) — `chaff=([zero]=1+4 four)` now reports `chaff: four: must use subscript when assigning associative array` instead of silently assigning to key `"0"`.

33. **Fix `declare fluff[qux]=assigned` subscripted names** (`src/builtins/vars.rs`) — `declare` with subscripted names like `fluff[qux]=assigned` now correctly assigns to the assoc/indexed array element instead of treating `fluff[qux]` as the variable name.

34. **Fix `declare -p` flags for assoc arrays** (`src/builtins/vars.rs`) — All `declare -p` output paths now include `i` (integer) and `r` (readonly) flags for associative arrays. Previously `-Ai` showed as `-A` and `-Ar` showed as `-A`.

35. **Fix `declare +A` and `declare +a`** (`src/builtins/vars.rs`) — `declare +A chaff` now emits `cannot destroy array variables in this way` error. Also added proper handling for `+i`, `+x`, `+u`, `+l`, `+c`, `+t` to remove variable attributes.

36. **Fix readonly error to show base name** (`src/interpreter/commands.rs`) — `waste[stuff]=other` where `waste` is readonly now reports `waste: readonly variable` instead of `waste[stuff]: readonly variable`.

37. **Fix compound assignment with spaced keys** (`src/parser.rs`) — `wheat=([foo bar]="qux qix")` now works. Added `find_bracket_close_in_parts` and token-merging logic in `parse_array_elements` to merge tokens split at spaces inside `[...]` subscripts.

38. **Fix compound assignment with quoted keys** (`src/parser.rs`) — `hash=(["key"]="value")` now works. Added `extract_array_index` helper that walks across multiple `WordPart`s to find `]=` in quoted subscripts.

39. **Fix scalar assignment to assoc array** (`src/interpreter/commands.rs`) — `T='([a]=1)'` on an assoc array now assigns the literal string to key `"0"` instead of dropping it. Also handles scalar assignment to indexed arrays.

40. **Fix scalar-to-assoc conversion** (`src/builtins/vars.rs`) — `assoc=assoc; declare -A assoc` now converts the scalar value to `[0]="assoc"` in the new assoc array instead of creating an empty array.

41. **Fix `${xpath["0"]}` quoted subscript** (`src/expand/params.rs`) — Assoc array lookups now strip surrounding quotes from subscript keys (e.g., `"0"` → `0`, `'key'` → `key`).

42. **Fix command-level `chaff[hello world]=flip`** (`src/parser.rs`) — Added multi-token bracket merge in `try_parse_assignment`: when a token has `name[` but no `]=`, subsequent tokens are consumed and merged until `]=` is found.

43. **Fix subscripted append for assoc arrays** (`src/interpreter/commands.rs`) — `wheat[foo bar]+=" blat"` now checks for assoc arrays first, using string key instead of arithmetic evaluation, preventing spurious arithmetic errors.

44. **Fix `declare -A chaff[200]`** (`src/builtins/vars.rs`) — Strip `[...]` subscripts from names when `-A` or `-a` flag is set, matching bash behavior.

45. **Fix `parse_assoc_literal` value truncation** (`src/builtins/vars.rs`) — When `\x1F` separators are present, the entire remainder after `]=` is the value (no whitespace splitting), fixing `declare -A wheat=([foo bar]="qux qix")` via builtins.

46. **Fix arithmetic panic on unclosed brackets** (`src/interpreter/arithmetic.rs`) — `eval_arith_expr_inner` no longer panics when `]` is not found (e.g., `[foo` without closing `]`).

### Fixes Applied Four Sessions Ago

18. **Fix `exec -c` to actually clear environment** (`src/builtins/exec.rs`) — `exec -c` was clearing env vars then re-applying all shell exports, defeating the purpose. Now the else branch only applies exports when `-c` is not set.

19. **Implement `exec -l` login shell flag** (`src/builtins/exec.rs`) — `exec -l` now prepends `-` to argv[0] to indicate a login shell, matching bash behavior.

20. **Fix source/dot positional params with `set --`** (`src/builtins/exec.rs`, `src/builtins/set.rs`, `src/interpreter/mod.rs`) — When `. file args` is used and the sourced file calls `set --`, the new positional params now persist after sourcing. Added `source_set_params` flag to Shell struct, set by `builtin_set`, checked by `builtin_source` to decide whether to restore saved params.

21. **Fix prefix assignments for `export` and `declare -x`** (`src/interpreter/commands.rs`) — `foo="" export foo` now persists the assignment (export always persists prefix assignments, even outside POSIX mode). `FOO='$$' declare -x FOO` also persists. Prefix assignments to builtins now set both `vars` and `exports` so `declare -p` sees the `-x` flag.

22. **Add `:` and `times` to POSIX special builtins list** (`src/interpreter/commands.rs`) — Both `is_special` and `is_posix_special_builtin` were missing `:` and `times`. Now `AVAR=foo :` in POSIX mode correctly persists the assignment.

23. **Fix `quote_for_declare` to escape special chars** (`src/builtins/mod.rs`) — `declare -p` output now escapes `$`, `` ` ``, `\`, and `"` inside double-quoted values, matching bash (e.g., `declare -x FOO="\$\$"`).

24. **Fix `echo -e "\c"` to suppress trailing newline** (`src/builtins/mod.rs`, `src/builtins/io.rs`) — `interpret_echo_escapes` now returns `(String, bool)` where the bool signals `\c` was found. The caller suppresses the trailing newline when `\c` is encountered, matching bash's `echo -e "bar\c "; echo foo` → `barfoo`.

25. **Fix `declare -p` output for empty arrays** (`src/builtins/vars.rs`) — Empty associative and indexed arrays: `declare -A name` (no `=()`) for declared-but-unset arrays, `declare -A name=()` for explicitly-set empty arrays. Uses `declared_unset` set to distinguish.

26. **Fix `declare -p` trailing space for indexed vs assoc arrays** (`src/builtins/vars.rs`) — Bash uses `([0]="x" [1]="y")` for indexed arrays (no trailing space) but `([key]="val" )` for associative arrays (trailing space). Fixed all indexed array outputs to omit the trailing space.

27. **Fix `set -u` (nounset) for positional params** (`src/expand/mod.rs`, `src/expand/params.rs`) — `$9` with `set -u` now correctly reports `$9: unbound variable` (with `$` prefix for unbraced positional params). `${9}` reports `9: unbound variable` (no `$` prefix for braced). Regular variables like `$UNSET` report `UNSET: unbound variable` (no `$` prefix), matching bash exactly.

28. **Fix nounset errors to exit shell/subshell** (`src/expand/mod.rs`, `src/interpreter/commands.rs`) — Added `NOUNSET_ERROR` thread-local flag. When `set -u` triggers on an unset variable, the shell/subshell now exits immediately (via `std::process::exit(1)`), preventing subsequent commands from running. This matches bash behavior: `( echo $UNSET ; echo after )` no longer prints "after".

29. **Initialize `BASH_ALIASES` and `BASH_CMDS`** (`src/interpreter/mod.rs`) — Added empty `BASH_ALIASES` and `BASH_CMDS` associative arrays at shell startup, so `declare -A` output matches bash.

30. **Quote associative array keys in `declare -p`** (`src/builtins/mod.rs`, `src/builtins/vars.rs`) — Keys containing non-alphanumeric/underscore characters are now quoted with `"..."` in `declare -p` output (e.g., `["*"]`, `["hello world"]`, `["\$x"]`), matching bash behavior.

### Fixes Applied Five Sessions Ago

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

## Failing Tests (sorted by diff size)

### Locally Passing (28 tests)

alias, appendop, arith, braces, case, casemod, comsub, comsub-posix, globstar, lastpipe, mapfile, nquote, nquote2, nquote3, nquote4, nquote5, parser, posix2, posixexp, posixpat, precedence, printf, quote, rhs-exp, set-e, set-x, shopt, tilde

### PID-diff only (would pass in nix, ~8 more tests)

- **new-exp** (PID diffs only) — **NEW: was 7 real diff lines, now fully fixed** ✅
- **assoc** (4 lines = timing diff only)
- **builtins** (18 lines = PID diffs)
- **glob** (12 lines = PID diffs)
- **heredoc** (8 lines = PID diffs)
- **procsub** (12 lines = PID diffs)
- **read** (8 lines = PID diffs)
- **type** (4 lines = PID diffs)

### Small Real Diffs

#### 1. func (2 lines)

- Missing `compgen: command not found` — our shell has `compgen` as a builtin but bash's non-interactive/non-readline build doesn't. Minor issue.

#### 2. varenv (18 lines local = ~chet + PID diffs)

- **`~chet` expansion**: Produces `/a/b/c` instead of `/usr/chet`. User-specific, differs by environment.
- **Nix sub-tests**: varenv3.sub (local scoping), varenv4.sub (assoc array conversion), varenv25.sub (local -p).

### Medium (30-200 diff lines)

#### 3. comsub2 (196 lines)

`${ ... }` dollar-brace command substitution (bash 5.3 feature), `local` in current shell context, alias handling in subshells, function definition inside command substitution.

#### 4. quotearray (212 lines)

Associative array keys with special chars in arithmetic contexts.

### Hard (200+ diff lines)

#### 5. nameref (266 lines local)

`declare -n` improvements but still substantially broken: wrong variable resolution, unset through nameref, nameref chains.

#### 6. array (425 lines local)

Remaining issues in array32.sub, array33.sub (injection protection, type conversion errors).

#### 7. vredir (734K lines)

`{var}>file` variable fd redirection is partially implemented but produces massive output differences.

### Not Locally Testable (tests don't exist locally)

array2, dollar, execscript, glob2, return, source, run-intl, runtests — these test files don't exist in `/tmp/bash-5.3/tests/` but may exist in the nix test environment.

### Flaky

#### trap (0-6 lines)

Extra CHLD signals (non-deterministic). Sometimes passes, sometimes fails with 1-2 extra `CHLD` lines.

### Other Failing Tests (not in main test list)

- **complete** (116 lines) — `complete` builtin not available in non-interactive bash builds
- **coproc** (12 lines) — fd number differences (61 vs 59)
- **dstack** (87 lines) — Missing `pushd`, `popd`, `dirs` builtins
- **histexp** (203 lines) — History expansion not implemented
- **ifs-posix** (2 lines) — timeout-dependent summary line
- **intl** (8 lines) — Internationalization/multibyte character issues, panic on non-ASCII

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
| `src/expand/params.rs` | Parameter expansion (`${...}` operators), `parse_arith_offset` (handles `$(()`), backtick expansion), `expand_backticks_in_str`, `is_valid_var_ref`, assoc subscript expansion + bad subscript error |
| `src/expand/pattern.rs` | Pattern matching, `pattern_replace` (handles empty value + `*` match) |
| `src/expand/arithmetic.rs` | `eval_arith_full` (with `opt_flags`), `resolve_arith_vars` (handles `${var:-default}`, `${var%%pat}`, special params `$-`/`$?`/etc., assignment detection), `eval_arith` (variable assignments, bare identifiers) |
| `src/parser.rs` | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`) |
| `src/lexer/mod.rs` | Lexer, thread-locals (`DQUOTE_TOGGLED`) |
| `src/lexer/dollar.rs` | `${}` parsing, `parse_brace_param` (bad substitution for `${$(...)}` ), `$(...)` comsub parser (now handles `<<<` here-strings) |
| `src/lexer/word.rs` | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body` |
| `src/lexer/heredoc.rs` | `register_heredoc` (line count fix), `read_heredoc_bodies` (backslash-newline), `parse_double_quoted_content` (backslash fix for `\"`) |
| `rust/bash/testsuite.nix` | Test harness with path/PID normalization |

## Recommended Next Priorities

1. **Fix comsub-posix nix error messages** — Improve error reporting for intentional syntax errors inside `$(...)` in case patterns. Need parser to detect reserved words like `done` in wrong context. (~35 nix diff lines)

2. **Fix arith10.sub array subscript quoting** — Handle `a[" "]`, `a[\ \]`, `a[\\]` in arithmetic array subscripts. (~100 nix diff lines)

3. **Fix heredoc sub-test issues** — heredoc3.sub (delimiter edge cases), heredoc7.sub (comsub+heredoc interaction), heredoc9.sub (function body printing). (~85 nix diff lines)

4. **Fix varenv nix sub-tests** — varenv3.sub (local scoping), varenv4.sub (assoc array conversion), varenv25.sub (local -p).

5. **Fix nameref basic operations** — `declare -n foo=bar; echo $foo` doesn't resolve correctly. Deep refactor needed for nameref resolution. (~266 diff lines)

6. **Fix quotearray arithmetic assoc key handling** — Assoc array subscripts with special chars in `((...))` context. (~212 diff lines)

7. **Implement `${ ... }` dollar-brace command substitution** — Bash 5.3 feature used in comsub2 tests. (~196 diff lines)

## Approach

Focus on **one test at a time**. Build with `cargo build`, test locally first with `diff`, then `nix build` to confirm. Clippy must pass with `-D warnings`. Use `jj commit -m 'fix(rust/bash): ...'` then `jj bookmark set push-nkqwvorqmnkn -r @-` then `jj git push --bookmark push-nkqwvorqmnkn` to push. Follow `.commitlintrc.yml` format. Do NOT add Co-Authored-By lines.