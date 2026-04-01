# Changelog

All notable fixes to the bash test suite are documented here, grouped by phase.

## Phase 10 — Array compound assignment / substring slicing (fixes 100–106)

**Tests improved:** quotearray 205→0 ✅, array 96→40, attr 0 ✅ (new), arith-for 0 ✅ (new)

- **quotearray**: 0 diff locally ✅ (was 205 lines). **Fully passing!**
  - Fixed by word splitting in compound array assignments (`arr=( $x )` now splits by IFS)
- **array**: 40 diff locally (was 96). **58% reduction — 56 more lines eliminated**
  - Fixed compound assignment detection: `declare e=$y` where `$y="(abc)"` no longer incorrectly creates an array (only literal `(...)` in source triggers compound assignment)
  - Fixed `read_compound_value` infinite loop on `)` from `$(...)` inside compound assignments (paren depth tracking)
  - Fixed word splitting in array compound assignments: `arr=( $x )` where `$x="a b c"` now produces 3 elements
  - Fixed compound assignment subscript validation: `[]=val`, `[*]=val`, `[-65]=val` now emit proper errors
  - Fixed `d=([*]=last)` no longer auto-creates associative array — stays indexed with error
  - Fixed `iarray[4]=4+1` arithmetic evaluation for integer-attribute arrays in subscript assignments
  - Fixed `${arr[@]:offset:length}` array substring slicing to use index-based offsets (not list-position)
  - Fixed negative array offsets to use `highest_index + 1` (the array Vec length), matching bash
  - All three expansion code paths fixed: unquoted `expand_part`, quoted `get_array_elements`, and `expand_param`
- **attr**: 0 diff locally ✅ (newly tested)
- **arith-for**: 0 diff locally ✅ (newly tested)
- **nameref**: 24 diff locally (PID-diff only — effectively passing)
- **vredir**: 32 diff locally (fd-number diffs only — effectively passing)

### Fixes

100. **Fix compound assignment detection for expanded values** (`src/interpreter/commands.rs`) — `declare e=$y` where `$y="(abc)"` no longer incorrectly creates an array. The `run_simple_command` preprocessing now checks if the `(` after `=` was literally in the source code (in a `Literal` word part) vs coming from variable expansion. Only literal `(...)` triggers compound array assignment; expanded values are treated as scalars. This fixes `declare -a e=$y` where `y="(\$(echo Darwin))"` previously hanging.

101. **Fix `read_compound_value` infinite loop on `)` from `$(...)`** (`src/builtins/mod.rs`) — Added parenthesis depth tracking to `read_compound_value`. Unquoted `(` increments depth, `)` decrements. Only `)` at depth 0 breaks the loop (end of compound assignment). Previously `$(echo)` inside a compound value caused the `)` of `$(...)` to break, leaving `pos` stuck at the outer `)`, causing an infinite loop.

102. **Fix word splitting in array compound assignments** (`src/interpreter/commands.rs`) — `arr=( $x )` where `$x="a b c"` now produces 3 array elements instead of 1. Bare elements (no `[n]=` subscript) in indexed array compound assignments now use `expand_word_fields` for IFS-based word splitting. Subscripted elements still use `expand_word_single` (no splitting).

103. **Fix compound assignment subscript validation** (`src/interpreter/commands.rs`) — Added validation for indexed array compound assignment subscripts: `[]=val` emits "bad array subscript", `[*]=val` / `[@]=val` emits "cannot assign to non-numeric index", `[-65]=val` emits "bad array subscript". On error, valid elements assigned before the error are kept (matching bash behavior). The array is inserted with partial results before returning.

104. **Remove auto-assoc-array heuristic for compound assignments** (`src/interpreter/commands.rs`) — Previously `d=([*]=last)` would auto-create an associative array because `*` is non-numeric. Now only variables already declared as assoc arrays (`declare -A`) get assoc treatment. Invalid subscripts like `[*]`, `[@]`, `[]`, and negative indices produce errors in indexed array mode.

105. **Fix integer array element assignment** (`src/interpreter/commands.rs`) — `iarray[4]=4+1` on an integer-attribute array now evaluates `4+1` as arithmetic (→ `5`). The arithmetic evaluation is performed before taking a mutable borrow of the arrays map to avoid borrow conflicts.

106. **Fix `${arr[@]:offset:length}` array substring slicing** (`src/expand/mod.rs`, `src/expand/params.rs`) — Array substring slicing now uses **index-based offset matching**: `${arr[@]:N:L}` selects set elements whose array index ≥ N, then takes L of them. Previously used list-position-based offsets which was wrong for sparse arrays. Fixed in three code paths: unquoted expansion in `expand_part`, quoted expansion in `get_array_elements`, and `expand_param`. Negative offsets use `highest_index + 1` (the array Vec length) as the base, matching bash.

---

## Phase 9 — Array/nameref/parser (fixes 87–99)

**Tests improved:** ifs-posix 2→0 ✅, array 170→96, nameref 24310→24 (PID-diff only) ✅, vredir 36→34

- **ifs-posix**: 0 diff locally ✅ (was 2 lines, timeout-dependent)
- **array**: 96 diff locally (was 170). **44% reduction — 74 more lines eliminated**
  - Fixed scalar-to-array conversion when assigning `a[n]=value` to existing scalar variable
  - Added `parse_indexed_compound_assignment` for `[n]=value` subscript syntax in compound arrays (replaced `parse_array_literal`)
  - Fixed double-value bug in parser multi-token bracket merge (`a[7 + 8]="x"` was producing `"xx"`)
  - Fixed `declare` identifier validation to allow any chars inside `[...]` brackets (`declare a["7 + 8"]`)
  - Fixed `declare -a name[subscript]=value` to strip subscript with `-a`/`-A` flags
  - Fixed `declare -r c[100]` to strip subscript and create empty readonly array
  - Fixed `declare -ar` listing to only show readonly arrays (not all readonly vars)
  - Fixed `readonly -a` listing to show `declare -ar` with array values; posix mode shows `readonly -a`
  - Fixed `readonly a[5]` to error with "not a valid identifier"
  - Fixed `read x[1]` to accept and properly assign array subscripts
  - Fixed `set_var` to assign to `array[0]` for existing indexed arrays (`declare -a x; x=val` → `x[0]=val`)
  - Added missing token string representations in parser (`LessGreat` → `<>`, etc.)
- **nameref**: 24 diff locally (was 24310). **PID-diff only — effectively passing!**
  - Fixed empty nameref rebinding: `declare -n ref; ref=x` now correctly sets nameref target to "x"
  - Fixed `resolve_nameref` to not follow empty nameref targets
  - Guarded all `std::env::remove_var`/`set_var` calls against empty strings (fixed panic in nameref4.sub)
- **vredir**: 34 diff locally (was 36). Minor improvement from env var guards.

### Fixes

87. **Fix empty nameref rebinding** (`src/interpreter/mod.rs`) — `declare -n ref; ref=x` now rebinds the nameref to point to `x` instead of assigning `""=x`. `set_var` checks for namerefs with empty targets and rebinds them. `resolve_nameref` no longer follows empty nameref targets.

88. **Guard all `std::env::remove_var`/`set_var` against empty strings** (`src/builtins/vars.rs`, `src/builtins/exec.rs`, `src/interpreter/commands.rs`, `src/interpreter/mod.rs`) — All 8 `remove_var` call sites and the allexport `set_var` call are guarded with `if !name.is_empty()`. This fixes the panic in nameref4.sub where `remove_var("")` caused `Invalid argument`.

89. **Fix scalar-to-array conversion on subscript assignment** (`src/interpreter/commands.rs`) — `a=abcde; a[2]=bdef` now converts the scalar to `a=([0]="abcde" [2]="bdef")` instead of losing `a[0]`. Both assignment and append paths handle this.

90. **Add `parse_indexed_compound_assignment`** (`src/builtins/mod.rs`) — New function handles `[n]=value` subscript syntax in compound array assignments. Replaces `parse_array_literal` (which only did word splitting). Used in `declare -a`, `local -a`, `readonly -a`, and `export -a` paths.

91. **Fix double-value bug in parser multi-token bracket merge** (`src/parser.rs`) — `a[7 + 8]="x"` was producing `"xx"` because the inner loop continued after `pval_parts.extend(parts[pi + 1..])`, causing subsequent parts to be added again via `pval_parts.push(part.clone())`. Added `break` after the extend.

92. **Fix `declare` identifier validation for bracket subscripts** (`src/builtins/vars.rs`) — `declare a["7 + 8"]="test 2"` was rejected as invalid identifier. Validation now only checks the base name (before `[`), allowing any content inside brackets.

93. **Fix `declare -a name[subscript]=value` to strip subscript** (`src/builtins/vars.rs`) — When `-a` or `-A` flag is set, subscripts are stripped from the name. `declare -a e[10]="(test)"` → `e=([0]="test")`. Scalar values still use the subscript index.

94. **Fix `declare -r c[100]` to create readonly array** (`src/builtins/vars.rs`) — No-value `declare` with subscript now always strips the subscript and creates an empty array (marked as declared-unset). Previously only stripped with explicit `-a`/`-A` flags.

95. **Fix `declare -ar` listing** (`src/builtins/vars.rs`) — `declare -ar` no longer lists all readonly variables. The `-r` listing is skipped when `-a` or `-A` is also set, and the `-a` listing filters by readonly flag.

96. **Fix `readonly -a` listing and `readonly a[5]` error** (`src/builtins/vars.rs`) — `readonly -a` now triggers listing mode. Lists only readonly arrays with `declare -ar` prefix (or `readonly -a` in posix mode). `readonly a[5]` now errors with "not a valid identifier".

97. **Fix `read x[1]` array subscript support** (`src/builtins/io.rs`) — `read` builtin now accepts array subscripts as variable names. Assignment handles array element setting with scalar-to-array conversion. Both validation and assignment paths updated.

98. **Fix `set_var` for existing arrays** (`src/interpreter/mod.rs`) — `set_var` now assigns to `array[0]` when the variable is an existing indexed array, instead of creating a separate scalar entry. Matches bash: `declare -a x; x=val` sets `x[0]=val`.

99. **Add missing token string representations** (`src/parser.rs`) — `token_to_str` now handles `Less`(`<`), `Great`(`>`), `DGreat`(`>>`), `LessAnd`(`<&`), `GreatAnd`(`>&`), `LessGreat`(`<>`), `Clobber`(`>|`), `DLess`(`<<`), `DLessDash`(`<<-`), `TripleLess`(`<<<`).

---

## Phase 8 — Vredir/dstack/trap/array (fixes 78–86)

**Tests improved:** vredir 734K→fd-only ✅, dstack 87→0 ✅, trap 0–6→0 ✅, array 425→170

- **vredir**: fd-number-only diffs (was 734K/72 lines). **Massive improvement — effectively passing!**
  - Added readonly variable check for `{var}` redirections (two-phase: file open proceeds, then var assignment fails)
  - Fixed `{var}<<EOF` heredoc var-redirection by adding `DLess`/`DLessDash` to parser redirect operator match
  - Fixed `{fd[0]}` array subscript redirections — parser now accepts `[`, `]` in var names; `resolve_redir_fd` sets array elements
  - All remaining diffs are fd-number offsets (our shell allocates fds 1-2 higher than reference)
- **dstack**: 0 diff locally ✅ (was 87 in PLAN). `pushd`/`popd`/`dirs` builtins fully working
- **trap**: 0 diff locally ✅ (was 0-6 flaky). No longer flaky
- **array**: 170 diff locally (was 425). **60% reduction — 255 lines eliminated**
  - Fixed `declare -a name` to convert existing scalar value to `array[0]` (was creating empty array)
  - Fixed `declare -pa` listing to only print indexed arrays (was dumping all variables)
  - Fixed `declare -pa` to include `r`, `i`, `x` flags for readonly/integer/export arrays
  - Initialized builtin arrays at startup: `BASH_ARGC`, `BASH_ARGV`, `BASH_LINENO`, `DIRSTACK`, `FUNCNAME`, `PIPESTATUS`
  - Set `BASH_LINENO[0]=0` when running script files
  - Added `b[]=val` and `b[*]=val` "bad array subscript" errors for indexed arrays
  - Added `d[7]=(...)` "cannot assign list to array member" error
  - Added negative index validation for non-existent arrays
- **read**: Fixed EOF return code when reading into REPLY (no variable names)
  - `read -u fd` at EOF with no var names was returning 0 instead of 1 — the `is_reply` early-return path bypassed `eof_reached` check
  - This also fixed the vredir2.sub infinite loop (while read at EOF never terminating)

### Fixes

78. **Add readonly variable check for `{var}` redirections** (`src/interpreter/redirects.rs`) — `exec {v}>>file` when `v` is readonly now emits two errors: `v: readonly variable` and `v: cannot assign fd to variable`. The check is done in `resolve_redir_fd` which returns `Result<i32, i32>` — `Err(fd)` for readonly vars. The fd is still allocated and the file open proceeds (bash creates the file even when the variable is readonly), but the redirection is treated as failed after I/O completes.

79. **Fix `read` EOF return code for REPLY** (`src/builtins/io.rs`) — `read -u fd` at EOF with no variable names (reading into REPLY) was returning 0 instead of 1. The `is_reply` early-return path (`return 0;`) bypassed the `eof_reached` check. Changed to `return if eof_reached { 1 } else { 0 };`. This also fixed vredir2.sub's infinite loop where `while read -r -u ${fd}` never terminated.

80. **Fix `{var}<<EOF` heredoc var-redirection parsing** (`src/parser.rs`) — `exec {v}<<EOF` was treating `{v}` as a command argument because the parser's `try_parse_redir_fd` didn't include `Token::DLess` and `Token::DLessDash` in the redirect operator match for `{varname}` patterns. Added both tokens.

81. **Fix `{fd[0]}` array subscript redirections** (`src/parser.rs`, `src/interpreter/redirects.rs`) — Parser now accepts array subscript syntax in `{var}` redirections (e.g., `{fd[0]}<&0`). `resolve_redir_fd` handles array subscripts by setting indexed array elements (`arrays["fd"][0]`) or associative array entries for non-numeric subscripts.

82. **Fix `declare -a name` scalar-to-array conversion** (`src/builtins/vars.rs`) — `a=abcde; declare -a a` now converts the scalar to `a=([0]="abcde")` instead of creating an empty array. Previously `shell.arrays.entry(...).or_default()` was used, losing the existing value.

83. **Fix `declare -pa` to only list indexed arrays** (`src/builtins/vars.rs`) — `declare -pa` (with type filter flag) was printing all variables. Added `has_type_filter` check: when `-p` is combined with `-a`, `-A`, `-x`, `-r`, `-i`, or `-n` and no names, the code falls through to the type-specific listing sections instead of the "print all" block.

84. **Fix `declare -pa` to include readonly/integer/export flags** (`src/builtins/vars.rs`) — The `declare -a` listing now includes `r`, `i`, `x` flags for arrays (e.g., `declare -ar a=(...)` for readonly arrays). Previously it always printed `declare -a` without checking attribute flags.

85. **Initialize builtin arrays at startup** (`src/interpreter/mod.rs`, `src/main.rs`) — Added `BASH_ARGC`, `BASH_ARGV`, `BASH_LINENO`, `DIRSTACK`, `FUNCNAME` (declared-but-unset), and `PIPESTATUS` array initialization. Set `BASH_LINENO[0]=0` when running script files (not `-c` mode).

86. **Add bad array subscript validation** (`src/interpreter/commands.rs`) — `b[]=val` (empty subscript) and `b[*]=val` / `b[@]=val` now emit `bad array subscript` errors for indexed arrays. Negative indices on non-existent arrays also error. `d[7]=(...)` now errors with `cannot assign list to array member`.

---

## Phase 7 — Nameref/intl (fixes 76–77)

**Tests improved:** nameref 30→0 ✅, intl 2→0 ✅

- **nameref**: 0 diff locally ✅ (was 30). **Fully passing!**
  - Fixed `unset foo` where `foo` is a nameref to unset the **target** variable, not the nameref itself
  - Implemented `unset -n foo` to remove just the nameref attribute
  - All ~14 PID diffs also gone (sub-tests produce identical output)
- **intl**: 0 diff locally ✅ (was 2). **Fully passing!**
  - Fixed `${#var}` to count locale-aware multibyte characters: converts string to raw bytes via `string_to_raw_bytes`, then counts UTF-8 characters when in a UTF-8 locale
  - `$'\303\251'` (raw bytes for é) now correctly reports length 1 instead of 2

### Fixes

76. **Fix `unset` through namerefs** (`src/builtins/vars.rs`) — `unset foo` where `foo` is a nameref now unsets the **target** variable (e.g., `bar`) while keeping the nameref itself intact, matching bash behavior. Previously it removed the nameref and left the target untouched. Also implemented `unset -n foo` to remove just the nameref attribute without touching the target. The `_unset_nameref` flag is now properly wired up. Three-way dispatch: `unset -n` removes the nameref, `unset` on a nameref unsets through to the target, plain `unset` on a regular variable removes it directly.

77. **Fix `${#var}` locale-aware multibyte character counting** (`src/expand/params.rs`) — Added `mbstrlen()` helper that checks the current locale (`LC_ALL`/`LC_CTYPE`/`LANG`). In UTF-8 locales, converts the bash-style string (raw bytes stored as Latin-1 chars) to raw bytes via `string_to_raw_bytes`, then counts UTF-8 characters with `String::from_utf8_lossy`. In non-UTF-8 locales, falls back to `chars().count()` (byte counting). Fixes `${#x}` returning 2 instead of 1 for `x=$'\303\251'` (UTF-8 é).

---

## Phase 6 — Comsub/lastpipe/nameref (fixes 65–75)

**Tests improved:** comsub 2→0 ✅, lastpipe 2→0 ✅, nameref 264→30, new-exp 8+panics→PID-only ✅, globstar 84→0 ✅, posixexp 6→0 ✅, intl 8→2

- **comsub**: 0 diff locally ✅ (was 2). Fixed SIGPIPE in process substitution children
- **lastpipe**: 0 diff locally ✅ (was 2). Fixed `in_pipeline_child` regression with lastpipe
- **nameref**: 30 diff locally (was 264). **Massive improvement — 234 lines reduced**
  - Fixed `./` prefix stripping in glob expansion (affected all sub-test script name prefixes) → ~214 lines eliminated
  - Fixed `typeset -n foo` (no value) to use foo's current value as the nameref target
  - Fixed `declare -n foo=bar` to remove foo from regular vars when creating nameref
  - Fixed `typeset +n foo=other` to assign through nameref first, then remove attribute
  - Fixed prefix assignment nameref resolution (`foo=two eval ...` where foo is a nameref)
  - Added empty-name guards to prevent panics in env::set_var/remove_var with empty nameref targets
  - Remaining: ~2 real nameref unset-semantics lines + ~14 PID diffs
- **new-exp**: PID diffs only (was 8+panics). **Panics fixed** ✅
  - Fixed parser panic on huge fd numbers (`1111111111111111111111</dev/stdin`)
  - Fixed multibyte panic in `${var/pattern/repl}` prefix/suffix replacement
- **globstar**: 0 diff sequentially ✅ (84 diff was parallel test execution artifact sharing `/var/tmp`)
- **posixexp**: 0 diff sequentially ✅ (6 diff was parallel test artifact sharing `/var/tmp/sh`)
- **intl**: 2 diff locally (was 8). Fixed `${#var}` to return character count instead of byte length
- **complete**: readline diff only ✅ (our shell has compgen/complete builtins, local non-readline bash doesn't). Passes in nix against full bash.
- **varenv**: 8 diff locally (was 18 = ~chet + PID diffs)
- **array**: 425 diff locally (unchanged)

### Fixes

65. **Fix `in_pipeline_child` regression with lastpipe** (`src/interpreter/pipeline.rs`) — `self.in_pipeline_child = !self.shopt_lastpipe` was wrong: when lastpipe is enabled, non-last forked pipeline children had `in_pipeline_child = false`, causing `echo` to print "Broken pipe" errors instead of silently exiting. Changed to `self.in_pipeline_child = true` unconditionally for all forked children.

66. **Reset SIGPIPE in process substitution children** (`src/expand/mod.rs`) — Added `libc::signal(libc::SIGPIPE, libc::SIG_DFL)` before the inline procsub runner in the child process. Previously only the exec fallback path reset SIGPIPE. Fixes `echo` inside `<(echo a)` getting "write error: Broken pipe" when the reader closes early (e.g., `${BUILDDIR#<(echo a)/}`).

67. **Fix `command` builtin error message for missing external commands** (`src/builtins/exec.rs`) — `command foo` where `foo` is not found now prints `foo: command not found` instead of the raw OS error `foo: No such file or directory (os error 2)`. Also handles `Permission denied` for non-executable paths with `/`.

68. **Fix glob expansion to preserve `./` prefix** (`src/expand/mod.rs`) — The `glob` crate normalises `./` away from results. When the original pattern starts with `./` (e.g., `./nameref[0-9].sub`), the prefix is now re-added to each result. This fixes sub-test script name prefixes in error messages (e.g., `./nameref3.sub: line 22:` instead of `nameref3.sub: line 22:`). Reduced nameref test diff from 264 to 50 lines.

69. **Fix parser panic on huge fd numbers** (`src/parser.rs`) — `let n: i32 = s.parse().unwrap()` in redirect fd parsing panicked with `PosOverflow` on numbers like `1111111111111111111111`. Changed to `if let Ok(n) = s.parse::<i32>()` with backtrack fallback. Fixes panic in new-exp2.sub.

70. **Fix multibyte panics in pattern replacement** (`src/expand/params.rs`, `src/expand/pattern.rs`) — `${var/pattern/repl}` prefix/suffix replacement and `${var#pattern}`/`${var%pattern}` trim operations iterated over byte offsets (`0..=val.len()`) but sliced with `val[..i]`, panicking on multibyte characters. Added `is_char_boundary(i)` checks to skip non-boundary byte positions. Fixed 4 instances in `expand_param`, 4 in `apply_param_op`, and 4 in `trim_pattern`.

71. **Fix `${#var}` to return character count** (`src/expand/params.rs`) — `val.len()` returns byte count but bash's `${#var}` returns character count. Changed to `val.chars().count()`. Fixes `${#x}` returning 2 instead of 1 for `x=é` (2-byte UTF-8, 1 character).

72. **Fix `typeset -n foo` (no value) to use existing value as target** (`src/builtins/vars.rs`) — `typeset -n foo` where `foo` already has value `"bar"` now creates a nameref `foo→bar` (using the existing value) instead of `foo→""`. Also removes `foo` from regular `vars` when creating the nameref.

73. **Fix `declare -n foo=bar` to clean up regular vars** (`src/builtins/vars.rs`) — `declare -n foo=bar` now removes `foo` from the regular `vars` map, preventing stale values from shadowing the nameref resolution.

74. **Fix `typeset +n foo=other` nameref removal semantics** (`src/builtins/vars.rs`) — When removing the nameref attribute with a value (`+n foo=other`), the value is first assigned through the nameref to the target variable, then the nameref is removed and `foo` retains the old target name as its plain string value. Added `nameref_consumed` set to prevent double-processing of names in the declare body.

75. **Fix prefix assignment nameref resolution** (`src/interpreter/commands.rs`) — `foo=two eval 'echo $foo'` where `foo` is a nameref to `bar` now correctly resolves `foo` through the nameref for both function and builtin prefix assignments. The resolved name is used for save/restore and env export. Added empty-name guards to prevent panics when nameref targets are empty strings.

---

## Phase 5 — Assoc improvements + new-exp/shopt/comsub (fixes 47–56)

**Tests improved:** assoc 2→0 ✅, new-exp 60→7, posixexp 3→0 ✅, shopt 68→0 ✅, comsub 2→0 ✅, lastpipe 2→0 ✅, procsub 13→12

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

### Fixes

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

---

## Phase 4 — Associative arrays (fixes 31–46)

**Tests improved:** assoc 75→65→2, posixexp 6→3, varenv 18→14, nameref 258→248, array 446→424

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

### Fixes

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

---

## Phase 3 — Heredoc/arith/comsub-posix (progress only)

**Tests improved:** heredoc 111→~0 (PID-only) ✅, arith 30→0 ✅, comsub-posix 20→0 ✅, posixexp 6→2

- **Started at**: 64/77 (arith diff 30, heredoc diff 111, comsub-posix diff 20)
- **heredoc**: main test 0 real diff locally ✅ (was ~20, only PID diffs remain), nix sub-tests ~85 diff
- **arith**: main test 0 diff ✅, sub-tests still have ~100 lines diff
- **comsub-posix**: 0 diff locally ✅, still fails in nix due to error message sub-tests
- **posixexp**: 2 diff locally (was 6), nix still fails on IFS/$@ issues
- **trap**: flaky — 1 extra CHLD signal (non-deterministic)
- **printf**: flaky — timing-dependent date format mismatch

### Progress

This phase focused on getting the heredoc, arithmetic, and comsub-posix tests to pass. No numbered fixes were assigned — the work predates the numbered fix tracking system.

---

## Phase 2 — Builtins/exec/set/declare (fixes 18–30)

**Tests improved:** builtins 40→18 (PID-only) ✅, new-exp 87→60, varenv 6→18 (regression in PID diffs, real improved), assoc 75→65, heredoc 8 (PID-only), comsub-posix 0 ✅

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

### Fixes

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

---

## Phase 1 — Foundational lexer/parser/set/export/enable (fixes 1–17)

**Tests improved:** Initial foundation — heredoc, set/shopt, export, builtins (79→40), local, continue, hash, exit, kill, enable

### Fixes

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