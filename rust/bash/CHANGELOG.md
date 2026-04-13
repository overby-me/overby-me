# Changelog

All notable fixes to the bash test suite are documented here, grouped by phase.

## Phase 89 — Duplicate nounset fix, `declare +r` through nameref, unset subscripted nameref, auto-create array for nameref targets (fixes 290–295)

1. **Duplicate nounset error eliminated** (`src/interpreter/commands.rs`) — `"${a[k]}"` used as a command (e.g. `a=() k=; "${a[k]}"` with `set -u`) no longer emits the "unbound variable" error message twice. Root cause: `run_simple_command` expanded the first word via `expand_word_single` to check for assignment builtins (`declare`/`export`/etc.), then re-expanded it via `expand_word_fields` in the main expansion loop. Each expansion triggered the nounset error independently. Fix: cache the first word expansion result from `expand_word_fields` and reuse it in the loop iteration for the first word. The `is_assignment_builtin` check now uses the cached result instead of a separate expansion call. **nameref25.sub** duplicate `a[k]: unbound variable` eliminated (~1 nix diff line).

2. **`declare +r` through nameref resolves to target** (`src/builtins/vars.rs`) — `declare +r ref` where `ref` is a readonly nameref now follows the nameref chain and checks readonly on the *target* variable, not the nameref itself. If the target isn't readonly, no error is produced even if the nameref itself is readonly. Added `flag_unset_readonly` to the `has_attr_flags` condition that triggers nameref resolution in `builtin_declare`. The readonly check now uses the resolved `attr_name` instead of the raw `name`. **nameref17.sub** `declare +r foo0` no longer errors when target `bar` isn't readonly (~2 nix diff lines).

3. **`declare +r` no-op for unbound namerefs** (`src/builtins/vars.rs`) — `declare +r ref` where `ref` is a readonly nameref without a target (resolves to itself) is now a no-op with exit 0, matching bash behavior. A `nameref_resolved_to_self` guard skips the readonly error when the nameref has no target and `-n` is not also specified. When `-n` IS specified (`declare +r -n ref`), the check applies to the nameref itself because `-n` indicates operating on the nameref directly. **nameref17.sub** `typeset +r foo1` (unbound readonly nameref) no longer errors (~2 nix diff lines).

4. **`+n` on readonly nameref sets exit status 1** (`src/builtins/vars.rs`) — `declare +n ref` on a readonly nameref variable now correctly returns exit status 1 from the builtin function. Previously `shell.last_status` was set to 1 but the function's `status` return variable was not updated because the `flag_unset_nameref_global` block ran before `status` was declared. Added `early_status` variable in the deferred `+n` processing that feeds into the main `status` variable. **nameref17.sub** `declare +n foo0` now returns correct exit status.

5. **`unset` through subscripted nameref** (`src/builtins/vars.rs`) — `unset n` where `n` is a nameref to a subscripted target like `v[1]` now correctly unsets the specific array element instead of removing the entire target variable. The `builtin_unset` nameref path now checks if the resolved target contains a bracket subscript. For subscripted targets: indexed arrays evaluate the subscript arithmetically and unset the specific element, associative arrays remove the key, and scalars are removed entirely. Readonly checks use the base variable name. **nameref15.sub** `unset n` where `n→v[1]` now produces `declare -a v=([0]="0")` instead of `declare -a v=([0]="0" [1]="1")` (~2 nix diff lines).

6. **Auto-create indexed array for subscripted nameref targets** (`src/interpreter/mod.rs`) — `set_var` with a resolved nameref target containing a subscript (e.g. `var[123]`) now creates an indexed array `var` with element `[123]` instead of falling through to create a scalar named `var[123]`. Previously, when the base variable didn't exist as an array, a scalar with the literal name `var[123]` was created. Now the code evaluates the subscript arithmetically and creates/extends the indexed array. Guards prevent auto-creation for `@`/`*` special subscripts (which are array-wide references, not element indices) and when the base name is itself a nameref (circular nameref protection — e.g. `a→b→a[1]` would conflict with `a` being both a nameref and an array). **nameref18.sub** reduced from ~16 to ~9 nix diff lines (subscripted nameref assignments like `declare -n ref=var[123]; ref=hello` now produce `declare -a var=([123]="hello")`).

**Phase 89 summary:** **nameref17.sub** flipped to 0 nix diff (was ~16 lines in Phase 86: `declare +r` through nameref, unbound nameref `+r` no-op, `+r -n` flag interaction, readonly error now reports target name). **nameref18.sub** reduced from ~16 to ~9 nix diff lines (subscripted nameref target auto-creates array; remaining diffs from `XXX[0]` subscripted nameref builtins like `mapfile`/`read -a`). **nameref25.sub** reduced by ~1 line (duplicate nounset eliminated). No previously passing tests regressed (all 75 nix-passing tests verified). Total nix passing: **75/77** (unchanged). Estimated nameref nix diff reduction from ~160 to ~136.

## Phase 88 — Subscripted nameref integer attribute, `declare -in` conflict, nameref integer flag display, nameref conversion cleanup (fixes 286–289)

1. **`declare b+=1` through subscripted nameref respects target's integer attribute** (`src/builtins/vars.rs`) — When a nameref `b` points to a subscripted array element like `a[0]`, `declare b+=1` now extracts the base name from the resolved nameref target (e.g. `a` from `a[0]`) and checks it for the integer attribute. Previously, only the full resolved name `a[0]` and the nameref name `b` were checked in `integer_vars`, missing the integer attribute on the base array `a`. This caused `declare b+=1` to do string concatenation (`"5" + "1" = "51"`) instead of arithmetic addition (`5 + 1 = 6`). **nameref23.sub** arithmetic diff fixed (`declare -ai a=([0]="51")` → `declare -ai a=([0]="6")`).

2. **`declare -in name=value` conflict handling** (`src/builtins/vars.rs`) — When both `-i` and `-n` flags are specified with a value in the same `declare`/`typeset` command, bash silently fails with exit status 1 and does NOT create the variable. These flags are incompatible because `-i` requires arithmetic evaluation while `-n` requires a variable name as the target. Added early `continue` in the `flag_nameref` branch when `flag_integer` is also set. Note: `declare -in name` (no value) still creates a declared-but-unset variable with both flags, and `declare -ni b` on an existing nameref adds the integer attribute — only the with-value case conflicts. **nameref23.sub** `declare -in b="a[0]"` diffs fixed (~7 lines: `b` now correctly "not found" instead of being created as a nameref).

3. **`declare -ni b` integer flag display in `declare -p` output** (`src/builtins/vars.rs`) — `declare -p` for namerefs now always shows the `-i` flag when the nameref variable has the integer attribute (`integer_vars` contains the name), regardless of whether the nameref target is empty or non-empty. Previously, the `-i` flag was only shown when `target.is_empty()`, so `declare -ni b` on an existing nameref with target `a[0]` would output `declare -n b="a[0]"` instead of the correct `declare -in b="a[0]"`. Fixed in all three `declare -p` nameref output paths: named variable lookup, all-variables listing, and namerefs-not-in-vars listing. **nameref23.sub** and **nameref21.sub** `declare -p` output diffs fixed.

4. **Integer attribute removed when converting variable to nameref** (`src/builtins/vars.rs`) — `declare -n ivar=foo` on an existing integer variable now removes the `-i` attribute from `ivar`, unless `-i` is also explicitly specified in the same command (e.g. `declare -in`). This matches bash where converting a variable to a nameref strips the incompatible integer attribute. Applied to both the with-value path (`declare -n name=value`) and the no-value path (`declare -n name` where name's current value becomes the target), including the circular-nameref-in-function-scope path. The guard `if !flag_integer` ensures that explicit `declare -in` preserves the integer attribute. **nameref19.sub** `declare -in ivar="foo"` → `declare -n ivar="foo"` fixed (~2 lines). **nameref20.sub** `declare -in ivar="foo"` output also corrected.

**Phase 88 summary:** **nameref23.sub** flipped to 0 nix diff (was ~16 lines). **nameref19.sub** ivar output fixed (~2 lines). No previously passing tests regressed (all 75 nix-passing tests verified). Total nix passing: **75/77** (unchanged). Estimated nameref nix diff reduction from ~168 to ~160.

## Phase 87 — Circular nameref read/append through enclosing scope, error message context prefixes (fixes 282–285)

1. **Circular nameref read from enclosing scope** (`src/interpreter/mod.rs`) — When reading a variable through a circular nameref in function scope (e.g. `function f { typeset -n v=$1; echo $v; }; v=hello; f v`), `get_var` now detects the circular nameref and reads the value from the saved scope entry instead of the local nameref variable. Additionally, `set_var` for circular namerefs now updates `self.vars[name]` alongside the saved scope entry so that subsequent reads within the function (via `ctx.vars.get` in the expansion code) see the updated value immediately. Previously, `echo $v` inside the function would return the stale local value instead of the enclosing scope's value.

2. **Circular nameref append through enclosing scope** (`src/interpreter/commands.rs`) — `ref+=X` through a circular nameref in function scope now correctly reads the existing value from the enclosing scope's saved entry, appends the new value, and writes the result back to both the saved scope entry and `self.vars`. E.g. `function f { typeset -n ref=$1; ref+=X; echo $ref; }; ref=; f ref` now outputs `X` inside the function and `X` outside (was outputting empty inside). Integer append through circular namerefs also handled via `eval_arith_expr`. **nameref15.sub** value diffs fixed for `add_X_echo` and `xxx_func` cases.

3. **Error message context prefixes** (`src/interpreter/mod.rs`) — When `set_var` validates a nameref target binding (empty nameref receiving an invalid value like `/`, `0`, or empty string), the error message now includes the appropriate context prefix: `((:` for arithmetic commands, `let:` for let expressions, or the builtin name (e.g. `printf:`, `getopts:`) when `self.current_builtin` is set. Previously, the error was just `'value': not a valid identifier` without indicating the originating context. Matches bash's error format for `((r=0))`, `printf -v r /`, `getopts x r -h`, etc. **nameref11.sub** reduced by ~4 diff lines (error prefix fixes for `((:`, `printf:`, `getopts:`).

**Phase 87 summary:** **nameref8.sub** further improved (circular read within function now correct). **nameref15.sub** value diffs fixed for `ref+=X` and `xxx=foo` circular cases (~4 lines). **nameref11.sub** error message prefixes improved (~4 lines). No previously passing tests regressed (all 75 nix-passing tests verified). Total nix passing: **75/77** (unchanged). Estimated nameref nix diff reduction from ~180 to ~168.

## Phase 86 — Circular nameref assignment to enclosing scope, prefix assignment duplicate key fix, readonly +n nameref protection (fixes 279–281)

1. **Circular nameref assignment to enclosing scope** (`src/interpreter/mod.rs`) — When a nameref is circular (e.g. `function f { typeset -n v=$1; v=inside; }; v=global; f v`) and the assignment is inside a function scope, bash assigns to the variable at the enclosing scope rather than the local nameref variable. Added `is_circular_nameref()` helper that detects cycles in the nameref chain. In `set_var`, when a circular nameref is detected and local scopes exist, the assignment now walks `local_scopes` from innermost to outermost to find the saved scope entry for the variable and updates its `scalar` (and `was_exported` if applicable). This propagates the value to the enclosing scope on function exit. Integer/uppercase/lowercase transforms are applied before scope iteration to avoid borrow-checker conflicts. **nameref8.sub** reduced from ~8→~3 diff lines (value diffs for `inside`/`global`, `foo`/empty, `local`/`global` all fixed; remaining diffs are line number offsets and one error message type difference).

2. **Prefix assignment duplicate nameref-resolved key fix** (`src/interpreter/commands.rs`) — When multiple prefix assignments resolve to the same key via nameref (e.g. `var= ref=5 typeset -p ref var` where `ref` is nameref to `var`), the save/restore mechanism now correctly tracks the ORIGINAL pre-prefix value. Added `prefix_seen` HashMap that records the first saved value for each key. When a duplicate key is encountered during the save phase, the original (pre-any-prefix) value is reused instead of capturing the intermediate value set by the earlier prefix assignment. Previously, the second save captured the value from the first prefix assignment, causing `var=""` to persist after restore instead of `var` being properly unset. **nameref14.sub** flipped to 0 diff (was 4 lines — `typeset -p ref var` after `var= ref=5 typeset -p ref var` now correctly shows `var: not found`).

3. **Readonly +n nameref protection** (`src/builtins/vars.rs`) — `typeset +n foo` on a readonly nameref with a non-empty target now correctly reports `foo: readonly variable` and preserves the nameref attribute, matching bash behavior. The `+n` (unset nameref) deferred processing now checks `shell.readonly_vars.contains(pure)` before removing the nameref. Readonly namerefs with empty targets (e.g. `declare -r -n foo5` with no value) are still allowed to have their nameref attribute removed, matching bash's distinction. **nameref17.sub** reduced from ~28→~16 diff lines (foo4 and foo5 cases now match bash; remaining diffs are about `declare +r` following namerefs vs. checking the nameref itself).

**Phase 86 summary:** **nameref8.sub** reduced from ~8→~3 diff lines (circular assignment semantics). **nameref14.sub** flipped to 0 diff (was 4 lines — prefix assignment duplicate key). **nameref17.sub** reduced from ~28→~16 diff lines (readonly +n). No previously passing tests regressed (all 75 nix-passing tests verified). Total nix passing: **75/77** (unchanged). Estimated nameref nix diff reduction from ~195 to ~180.

## Phase 85 — Exported nameref env value, export follows nameref, prefix assignment nameref resolution, declare -n compound array, scope-aware error messages, empty nameref validation (fixes 273–278)

1. **Exported nameref env value** (`src/builtins/vars.rs`) — `typeset -nx ref=var` now correctly exports `ref=var` (the nameref's target name) to the process environment, so `printenv ref` returns `var`. In `builtin_declare`, when `flag_export` is set for a nameref variable, the export now stores the nameref's target name as the environment value instead of resolving through the nameref to get the target variable's value. Previously, `shell.get_var(name)` was called which resolved through the nameref chain and exported the target's value under the nameref's name — e.g. `ref=foo` instead of the correct `ref=var`. **nameref14.sub** reduced from 20→4 diff lines.

2. **`export ref` follows nameref to target** (`src/builtins/vars.rs`) — `export ref` where `ref` is a nameref to `var` now resolves through the nameref and exports the target variable (`var=foo`). In `builtin_export`, when the argument is a nameref, `resolve_nameref()` is called to find the target variable name, and that target is exported with its current value. Previously, `export ref` tried to look up `ref` directly in `shell.vars` which doesn't exist for namerefs, resulting in the variable being marked as declared-but-unset for export.

3. **External command prefix assignment nameref resolution** (`src/interpreter/commands.rs`) — In `run_external`, prefix assignments in the child process now resolve namerefs via `self.resolve_nameref(&assign.name)` before setting environment variables. `ref=foo printenv ref` where `ref` is an exported nameref to `var` now correctly sets `var=foo` in the child's environment without overwriting the `ref=var` env entry. Previously, `std::env::set_var(&assign.name, &value)` used the raw nameref name, clobbering the exported nameref value. The existing value lookup for append mode (`+=`) also uses the resolved name.

4. **`declare -n array=(compound)` creates array with error** (`src/interpreter/commands.rs`) — When `declare -n` receives a real compound assignment (not from single-quoted source), the compound assignment is now pre-processed to create the array, and the "reference variable cannot be an array" error is emitted by the declare builtin without applying the nameref attribute. The pre-processing condition was updated from `!has_nameref_flag` to `(!has_nameref_flag || !paren_from_single_quote)`, allowing real compound assignments through while still blocking single-quoted targets like `declare -n foo='(one two three)'` (Phase 84 fix). **nameref22.sub** reduced from 10→4 diff lines.

5. **Scope-aware empty nameref target error messages** (`src/builtins/vars.rs`) — When `declare x=42` is called at global scope and `x` is a nameref with an empty target, the error is now "`42': not a valid identifier" (generic, matching bash) and the nameref is removed from`shell.namerefs`. In function scope (`local_scopes`non-empty), the error remains "`42': invalid variable name for name reference" and the nameref is preserved (matching bash). This scope-aware behavior applies to both the `flag_integer` path (where the raw value is validated before arithmetic evaluation — e.g. `7*6` not `42`) and the general assignment path. **nameref12.sub** reduced from 20→6 diff lines. **nameref11.sub** reduced from 82→78 (improved error messages).

6. **Empty nameref subscript assignment and readonly validation** (`src/interpreter/commands.rs`, `src/builtins/vars.rs`) — `ref[0]=foo` where `ref` is a nameref with an empty target now correctly reports "`': not a valid identifier" (matching bash). In`execute_assignment`, the nameref target is checked directly via`namerefs.get()` for empty targets before proceeding with the subscripted assignment. Additionally, `readonly ref` where `ref` resolves through a nameref to a subscripted target like `var[0]`now correctly rejects with "`var[0]': not a valid identifier" in `builtin_readonly`, since `readonly` operates on whole variables, not array elements.

## Phase 84 — Arithmetic nameref resolution, nounset nameref fix, declare -n compound assignment skip, append through nameref, scalar cleanup (fixes 268–272)

1. **Arithmetic nameref resolution** (`src/interpreter/arithmetic.rs`) — `$(( iref+4 ))` where `iref` is a `typeset -n` nameref to `ivar` now correctly resolves through the nameref chain to look up the target variable's value. Previously, arithmetic evaluation used `self.vars.get(name)` directly without resolving namerefs, so `$(( iref ))` returned 0 instead of the target's value. Both the `${var}` syntax path and the bare variable reference path in `eval_arith_expr_inner` now call `resolve_nameref()` before value lookup. Nameref targets with subscripts (e.g. `bar[0]`) are also correctly resolved to the specific array element value — the resolved name is checked for `[` brackets, and the base/subscript are used to look up the correct element from `arrays` or `assoc_arrays`. **nameref4.sub** flipped to 0 diff (was 6 lines — `$(( iref+4 ))` now returns 16 instead of 4, and `ckval foo $bar` no longer fails).

2. **Nounset nameref resolution in arithmetic** (`src/interpreter/arithmetic.rs`) — `set -u; echo $(( r ))` where `r` is a nameref to unset variable `k` now correctly reports `k: unbound variable` (the resolved target name) instead of silently returning 0. The nounset check now examines whether the resolved target variable exists in `vars`, `arrays`, `assoc_arrays`, or environment — a nameref itself is no longer considered "existing" for nounset purposes. Previously, `self.namerefs.contains_key(expr)` caused the check to pass even when the resolved target didn't exist. The error message also uses the resolved name (e.g. `k`) instead of the nameref name (e.g. `r`), matching bash behavior.

3. **Skip compound assignment pre-processing for `declare -n`** (`src/interpreter/commands.rs`) — Added `has_nameref_flag` detection in `run_simple_command`'s assignment builtin pre-processing. When `-n` flag is present in `declare`/`typeset`/`local` args, compound assignment syntax like `declare -n array='(one two three)'` is no longer pre-processed as an array assignment. Instead, the argument is passed through to the builtin as `array=(one two three)`, which the nameref validation correctly rejects with `` `(one two three)': invalid variable name for name reference ``. Previously, the compound assignment handler would execute the array assignment first (modifying the array from `(zero)` to `(one two three)`), then the builtin would see just the bare name and report the wrong error ("reference variable cannot be an array"). The flag detection checks `args` for any `-...n...` pattern (excluding `=`-containing args to avoid false matches on values). **nameref22.sub** reduced from 16→10 diff lines.

4. **`declare foo+=value` through nameref to array element** (`src/builtins/vars.rs`) — Added `get_existing_through_nameref()` helper function that resolves namerefs (including subscripted targets like `bar[0]`) to retrieve the current value before appending. `declare foo+=" more"` where `foo` is a nameref to `bar[0]` now correctly gets the existing value from `bar[0]`, appends to it, and stores back through `set_var`. The helper resolves the nameref, checks if the resolved name contains a subscript, and looks up the value from the appropriate array/assoc/scalar map. This is used in three places in `builtin_declare`: the integer append path, the integer-attribute append path, and the general string append path. Previously, `shell.vars.get(name)` was used which looked up the nameref name itself (not the target), resulting in an empty existing value and the append becoming a replacement. **nameref22.sub** improved (combined with fix #3).

5. **Scalar cleanup on compound array assignment** (`src/interpreter/commands.rs`) — When a compound array assignment like `bar=(one two)` overwrites an existing scalar `bar=4`, the stale scalar entry is now removed from `self.vars` via `self.vars.remove(&resolved)`. This ensures `$bar` (without subscript) correctly resolves to element [0] of the new array instead of returning the old scalar value. Applied to both indexed array and associative array compound assignments in `execute_assignment`'s `AssignValue::Array` handler. Previously, the scalar entry persisted alongside the array, and `$bar` would find the scalar first. **nameref4.sub** improved (combined with fix #1 — `$bar` after `bar=(one two three four)` now correctly returns `one` instead of the old scalar `4`).

## Phase 83 — Nameref attribute removal, array conflict validation, mapfile/for-loop nameref handling (fixes 265–267)

See `PLAN.md` for Phase 83 details.

## Phase 82 — Export state save/restore, varenv fixes (fixes 262–264)

See `PLAN.md` for Phase 82 details.

## Phase 81 — `unset` scope peeling, invalid indirect expansion, nameref declare error message (fixes 262–264)

1. **`unset` scope peeling for parent function locals** (`src/builtins/vars.rs`) — When a child function calls `unset var` and `var` is a local in a parent function scope (not the current scope), bash "peels" one scope layer: it removes the parent's local and restores the saved value from before that local was created, revealing the enclosing scope's variable. Previously, `unset` just removed the variable from the flat maps without updating the scope stack, so the parent function's scope restore on exit would overwrite any new assignments made after the unset. The fix searches `local_scopes` from the second-to-last (innermost parent) outward to find the innermost parent scope that has the variable saved. When found (and `localvar_unset` shopt is OFF), it removes the saved entry from that scope and restores the saved state to the flat maps — scalar, array, assoc, integer, readonly, declared_unset, and nameref attributes are all restored, plus process environment is re-synced for exported variables. When `localvar_unset` shopt is ON, the behavior changes: unset from a child function marks the parent's local as declared-but-unset (keeping the scope slot) instead of peeling the scope, matching bash's `localvar_unset` semantics. The existing behavior for current-scope locals (marking as declared-but-unset) is preserved. **varenv10.sub** flipped to 0 diff (was ~8 lines — `inner()` calling `unset res` on `outer()`'s local now correctly reveals global scope; subsequent array assignments persist after `outer()` returns). **varenv24.sub** flipped to 0 diff (was ~2 lines — `f2()` calling `unset x` on `f1()`'s local now reveals global `x=global`).

2. **Invalid indirect expansion error for `${!name-word}` when `name` is unset** (`src/expand/params.rs`) — `${!name-word}`, `${!name+word}`, `${!name:-word}`, `${!name:+word}`, `${!name:?msg}`, etc. now produce "invalid indirect expansion" error when the indirect variable `name` is completely unset (not found in vars, arrays, assoc_arrays, namerefs, positional params, special variables, or process env), matching bash. Previously, the expansion silently succeeded with the default value or produced no error. The check is added at the beginning of the indirect-expansion-with-operator path (before `lookup_var` is called). It extracts the base variable name before any `[` subscript (e.g. `varname[@]` → `varname`) so that `${!varname[@]@Q}` correctly checks whether `varname` exists. Positional parameters are handled specially: any non-negative integer (e.g. `9`, `99999`) is always considered valid since `$9` etc. are just unset positional params, not nonexistent variables. Special variables (`@`, `*`, `#`, `?`, `-`, `!`, `$`, `0`, `_`) are also always considered valid. The error uses `EXPAND_ERROR_PREFIX` for correct script/line reporting and sets `arith_error` to abort the current command. **nameref3.sub** flipped to 0 diff (was ~4 lines — `${!foo-unset}` after `unset -n foo` now correctly errors with "invalid indirect expansion" instead of silently expanding).

3. **Nameref-specific error message in declare builtin** (`src/builtins/vars.rs`) — `typeset foo=12345` where `foo` is a nameref with empty target now produces `` typeset: `12345': invalid variable name for name reference `` (matching bash) instead of the generic `` `12345': not a valid identifier ``. The fix adds a check in the declare builtin's scalar assignment path: before calling `shell.set_var(name, value)`, if `name` is a nameref with an empty target (`shell.namerefs.get(name)` returns `Some("")`) and the value fails `is_valid_nameref_target`, the nameref-specific error message including the command name (`cmd_name`) is emitted and the assignment is skipped. The generic error in `set_var` (which lacks the command name context) is no longer reached for this case. **nameref13.sub** flipped to 0 diff (was ~4 lines — error message now includes `typeset:` prefix and uses "invalid variable name for name reference" wording).

## Phase 80 — Function prefix assignment attribute restoration, assoc hash bucket count, empty nameref target validation (fixes 259–261)

1. **Function prefix assignment attribute restoration** (`src/interpreter/commands.rs`) — When a function is called with prefix assignments (e.g. `a=7 f1`), the save/restore of prefix variables now includes `readonly_vars`, `integer_vars`, and `declared_unset` attributes in addition to `vars` and `exports`. Previously, if a function made a temp-env variable readonly (e.g. `f1() { a=3 readonly a; }`), the readonly attribute persisted after the function returned even though the variable value was correctly restored/removed. This caused subsequent calls (especially in POSIX mode) to fail with "readonly variable" errors. The `prefix_saves` tuple in `run_simple_command` is extended from `(String, String, Option<String>, Option<String>)` to a 7-element tuple adding `(bool, bool, bool)` for `old_readonly`, `old_integer`, and `old_declared_unset`. On restore (non-POSIX mode), attributes are reset to their pre-prefix state — if the variable wasn't readonly before the prefix, it's removed from `readonly_vars` after the function returns. In POSIX mode, the "variable was modified" check still skips restoration, preserving special builtin prefix persistence. **varenv23.sub** flipped to 0 diff (was ~5 lines).

2. **Associative array hash bucket count for `convert_var_to_assoc`** (`src/interpreter/commands.rs`, `src/builtins/vars.rs`, `src/interpreter/mod.rs`) — When converting an existing scalar or indexed array to an associative array (e.g. `declare -gA foo=(...)` where `foo` was previously a scalar), the new assoc now uses 128 hash buckets (matching bash's `convert_var_to_assoc` → `assoc_create(0)` → `DEFAULT_HASH_BUCKETS=128`). Previously, all assoc arrays were created with 1024 buckets (`ASSOC_HASH_BUCKETS`), causing different hash iteration ordering when the variable was converted from another type. Three cases are now distinguished: (a) variable is already an assoc → re-use its existing `nbuckets()` (bash flushes and reuses the same hash table); (b) variable exists as scalar or indexed array → 128 buckets (conversion path); (c) variable doesn't exist → 1024 buckets (fresh creation). Added `parse_assoc_literal_with_buckets(s, nbuckets)` function that delegates from the existing `parse_assoc_literal(s)` (which passes 1024). Added `AssocArray::nbuckets()` public accessor. For `declare -gA` with `-g` flag, the check looks through saved local scopes to determine the variable's type at global scope. **varenv11.sub** flipped to 0 diff (was ~1 line).

3. **Empty nameref target validation** (`src/builtins/vars.rs`) — `declare -n name=` (with an `=` sign but empty target value) now correctly reports `` `': not a valid identifier `` error and does NOT create the variable, matching bash behavior. Previously, the empty target was silently accepted because the validation check `!value.is_empty() && !is_valid_nameref_target(value)` skipped empty values entirely. Added explicit `value.is_empty()` check before the existing `is_valid_nameref_target` check, with a `continue` statement to skip all remaining processing for the name (bash doesn't create the variable at all on this error). The error message uses "not a valid identifier" (not the nameref-specific "invalid variable name for name reference") matching bash's behavior for the empty-string case. Reduces **nameref** nix diff by ~2 lines (nameref24.sub).

## Phase 77 — `local -` all-options save/restore, IGNOREEOF dynamic variable, unset local declared-but-unset, compound readonly scope, prefix subscript validation (fixes 251–258)

1. **`local -` saves/restores ALL shell options** (`src/interpreter/mod.rs`, `src/builtins/vars.rs`, `src/interpreter/commands.rs`, `src/interpreter/pipeline.rs`) — Expanded `SavedOpts` from a 6-field tuple `(errexit, nounset, xtrace, noclobber, noglob, pipefail)` to a full struct capturing all `opt_*` boolean fields (`errexit`, `nounset`, `xtrace`, `noclobber`, `noglob`, `pipefail`, `keyword`, `hashall`, `allexport`, `monitor`, `physical`, `posix`, `noexec`), the `shopt_options` HashMap, and all dedicated shopt fields (`nullglob`, `extglob`, `globstar`, `inherit_errexit`, `nocasematch`, `lastpipe`, `expand_aliases`). Added `SavedOpts::capture()` and `SavedOpts::restore()` methods. On restore, `update_shellopts()` is called to sync `SHELLOPTS`/`BASHOPTS` readonly variables, and `IGNOREEOF` variable is synced with the `ignoreeof` shopt state. Updated `run_function` and `teardown_funsub_scope` to use `saved_opts.restore(self)` instead of tuple destructuring. Fixes `$-` and `$SHELLOPTS` mismatch after function return when `local -; set -m -H +B; set -u` was used inside a function. **varenv21.sub** flipped to 0 diff.

2. **`IGNOREEOF` dynamic variable handling** (`src/interpreter/mod.rs`, `src/builtins/vars.rs`) — Setting `IGNOREEOF=N` (via `set_var`) now automatically enables the `ignoreeof` shopt option, matching bash behavior where `IGNOREEOF=0; shopt -o ignoreeof` shows `on`. Unsetting `IGNOREEOF` (via `builtin_unset`) disables the `ignoreeof` shopt option. Previously, `IGNOREEOF` assignment had no effect on the shell option state.

3. **`unset` of local variable leaves declared-but-unset** (`src/builtins/vars.rs`) — When `unset v` is called inside a function and `v` is in the current local scope (`local_scopes.last().contains_key(name)`), the variable is now marked as `declared_unset` instead of being fully removed. `declare -p v` correctly shows `declare -- v` (or `declare -x v` if the variable had export attribute from temp env). Preserves export attribute for temp-env locals: after `v=t f` then `local v=x; unset v`, `declare -p v` shows `declare -x v` matching bash. **varenv20.sub** flipped to 0 diff.

4. **Compound assignment readonly scope-awareness** (`src/interpreter/commands.rs`) — The readonly check for compound assignments (`local qux=(one two)`) now uses the same scope-aware logic as scalar `local name=value` assignments: checks whether the readonly comes from global scope (error), current function scope (error), or an outer function scope (allow shadowing via `declare_local`). Moved the readonly check before `declare_local()` so the flag is still set when checked. For `local` commands, pushes the bare name to args so `builtin_local` also emits its own "local: qux: readonly variable" error (matching bash's two-error output). For `declare`/`typeset`, falls through to push bare name without triggering a duplicate error. Prevents false "readonly variable" errors when nested functions use `local x=(...)` and `x` is readonly in a calling function. **varenv19.sub** no regression, **varenv11.sub** readonly errors now correct.

5. **Bare `local name` readonly check** (`src/builtins/vars.rs`) — `local name` (bare, no `=`) on a globally readonly variable now correctly errors with "local: name: readonly variable" instead of silently creating a declared-but-unset shadow. Uses the same scope-aware logic as the `name=value` path: errors if globally readonly or readonly in current scope, allows shadowing if readonly comes from an outer function scope.

6. **Prefix assignment subscript validation** (`src/interpreter/commands.rs`) — `var[0]=X var[@]=Y f` (prefix assignments with array subscripts used as temporary environment for function/external command calls) now correctly rejected with `` `var[0]': not a valid identifier `` error matching bash. Subscripted names are filtered out of the temp-env `prefix_saves` vector via `.filter(|a| !a.name.contains('['))`. **varenv13.sub** flipped to 0 diff.

7. **`shopt -o` output alignment** (`src/builtins/set.rs`) — `shopt -o OPTION` (specific option query) now uses 20-char left-alignment (`{:<20}`) matching bash, while `shopt -o` (list all), `shopt -s -o`, and `shopt -u -o` continue to use 15-char alignment (`{:<15}`) matching bash's inconsistent behavior between single-option and all-option display modes.

8. **`local -p` prints `local -`** (`src/builtins/vars.rs`) — When `local -` was used in a function (i.e., `saved_opts_stack.last()` is `Some`), `local -p` now includes `local -` in its output after the variable declarations, matching bash behavior.

**Phase 77 summary:** Reduces **varenv** from ~102 to ~79 nix diff lines (~23 line reduction). **varenv20.sub**, **varenv21.sub**, **varenv13.sub** flipped to 0 diff. No previously passing tests regressed (verified all 74 passing nix tests still pass). Total nix passing: **74/77** (unchanged).

## Phase 75 — Bracket matching for assoc `]` keys, deferred +n nameref removal, local/declare declared-but-unset (fixes 245–250)

1. **`read`/`printf -v` bracket matching for assoc arrays with `]` key** (`src/builtins/io.rs`) — When `assoc_expand_once` is ON and the base variable name is a known associative array (or nameref to one), use `rfind(']')` for bracket matching so that unquoted `A[$rkey]` where `rkey=]` correctly treats the last `]` as the structural close. For non-assoc arrays or AEO-off, use first-`]` forward scan matching bash's `valid_array_reference` → `skipsubscript`. Applied to all six bracket-matching paths: `printf -v` validation, `printf -v` assignment, `read` argument parsing (two locations), `read` second-pass validation, and `read` assignment. Fixes assoc18.sub (was regressed by naive first-`]`-only approach). array27.sub remains ~4 nix diff — double-quoted `"A[$k]"` where the base IS assoc still uses `rfind` (can't distinguish quoting context at builtin level).

2. **Deferred `+n` nameref removal with attribute propagation** (`src/builtins/vars.rs`) — `declare +n -i foo=7+4` now correctly applies the `-i` attribute to the nameref target variable `bar` and evaluates `7+4` as integer `11`. Previously, `+n` processing was inline during the flag-parsing loop, so `-i` (which appears later in the argument list) wasn't yet set when the nameref was removed. Fix: record `flag_unset_nameref_global` during parsing and defer all nameref removal to a new block after the flag-parsing loop completes, when all attribute flags (`-i`, `-x`, `-r`, `-u`, `-l`, `-c`) are available. Also applies those flags to the nameref's target. Added `nameref_consumed.is_empty()` guards to `declare -i`, `declare -x`, `declare -r` listing paths to prevent spurious "list all" output when names were consumed by `+n` processing. Fixes nameref19.sub `declare -- bar="7+4"` → `declare -i bar="11"`.

3. **Empty nameref `+n` creates declared-but-unset** (`src/builtins/vars.rs`) — `declare +n foo5` where `foo5` was a nameref with no target (empty string) now marks `foo5` as `declared_unset` instead of inserting an empty string into `shell.vars`. Applied to both the value and no-value branches of the deferred `+n` processing. Fixes nameref17.sub `declare -r foo5=""` → `declare -r foo5`.

4. **`local v` (no `=`) creates declared-but-unset local** (`src/builtins/vars.rs`) — `local v` without an assignment now creates a declared-but-unset local variable that shadows any outer/global value, matching bash's `declare -- v` output. Exception: if the variable was set via temp env (`v=t f`), the exported value is inherited (detected by checking `shell.exports`). Previously, `shell.vars.entry().or_default()` kept the inherited global value. Reduces varenv20.sub diffs.

5. **`declare v` in function scope creates declared-but-unset** (`src/builtins/vars.rs`) — Same fix as `local v` applied to the `declare` builtin's no-value path when `make_local` is true. `declare v` inside a function now removes the inherited global from `vars` and marks as `declared_unset`, unless the variable is exported (temp env).

6. **`declare -ix foo6` on declared-but-unset nameref** (`src/builtins/vars.rs`) — Export flag on declared-but-unset variables no longer forces an env var with empty value. When the variable is in `declared_unset` and not in `vars`, the export is recorded in `shell.exports` but `std::env::set_var` is skipped. Fixes nameref19.sub `declare -ix foo6=""` → `declare -ix foo6`.

**Phase 75 summary:** Reduces **varenv** from ~127 to ~121 nix diff lines. **nameref** nameref19.sub flipped to 0 diff locally (was ~4 lines for `+n -i` and `foo5`/`foo6` issues). **array** unchanged at ~4 nix diff. Total nix passing: **74/77** (unchanged). No previously passing tests regressed.

## Phase 71 — Readonly through namerefs, declare -p flags, target validation, scope restore (fixes 239–244)

1. **`readonly` resolves through namerefs** (`src/builtins/vars.rs`) — `readonly ref` where `ref` is a nameref now marks the *target* variable readonly (not the nameref itself), matching bash behavior where `declare -n ref=foo; readonly ref` makes `foo` readonly. `builtin_readonly` now calls `shell.resolve_nameref(name)` before inserting into `readonly_vars`. The already-readonly check also resolves through namerefs. Error messages use the resolved target name. Fixes nameref2.sub (was 5 diff lines, now 0) and nameref5.sub (was 8 diff lines, now 0).

2. **`declare -p` nameref attribute flags** (`src/builtins/vars.rs`) — `declare -p` output for namerefs now includes all attribute flags in alphabetical order: `-inrx` (integer when empty target, nameref, readonly, export). Empty namerefs (no target) show just `declare -n name` without `=""`. When nameref has a target, integer/export flags are NOT shown on the nameref itself (they belong on the target). Three code paths updated: print-all vars, print-all namerefs-not-in-vars, and print-specific-names. Fixes nameref17.sub `declare -nr foo0="bar"` output (was showing just `declare -n`).

3. **Self-reference: function scope vs global scope** (`src/builtins/vars.rs`) — `typeset -n v=$1` where `$1=v` inside a function now produces "warning: circular name reference" (not "self references not allowed") and still creates the nameref (matching bash). At global scope, `declare -n x=x` still produces "self references not allowed" and does NOT create the nameref. The distinction is based on `shell.local_scopes.is_empty()`. Both the `declare name=value` path and the bare `declare name` path handle this correctly. Reduces nameref8.sub diff from 28→24 lines.

4. **`resolve_nameref_warn` for circular reference warnings** (`src/interpreter/mod.rs`) — New `Shell::resolve_nameref_warn()` method emits "warning: {name}: circular name reference" on first cycle detection and "warning: {name}: maximum nameref depth (8) exceeded" after depth limit is hit. Unlike `resolve_nameref()` (silent), this is used in `get_var` and `set_var` for actual variable access. Does NOT break on circular detection — continues iterating up to depth 8, matching bash's behavior of producing both the circular warning and the depth exceeded warning.

5. **Nameref target validation** (`src/interpreter/mod.rs`, `src/builtins/vars.rs`) — New `is_valid_nameref_target()` function validates that a nameref target is a valid identifier (optionally with `[subscript]`). Three validation points: (a) `declare -n name=value` rejects invalid targets with "invalid variable name for name reference"; (b) bare `declare -n name` where existing value is invalid rejects with same error and puts value back; (c) `set_var` empty-nameref rebinding rejects invalid targets with "not a valid identifier". Fixes nameref12.sub (46→24), nameref24.sub (9→2).

6. **Nameref local scope save/restore** (`src/interpreter/mod.rs`, `src/interpreter/commands.rs`) — `SavedVar` now includes a `nameref: Option<String>` field storing the previous nameref target. `declare_local()` saves the current nameref state and removes it for the local scope (so the local variable starts fresh). `run_function` scope restoration restores namerefs on function exit (inserting saved target or removing the nameref). Previously, namerefs created with `declare -n` inside functions leaked into global scope after return. Also fixed: `declare -n ref` on an existing nameref is now a no-op (keeps existing target) instead of clearing it to empty. Fixes nameref7.sub (4→0), reduces nameref13.sub (16→4), nameref20.sub (46→36).

**Phase 71 summary:** Reduces **nameref** from ~445 to ~347 nix diff lines (~22% reduction, 98 lines eliminated). nameref2.sub 5→0 ✅, nameref5.sub 8→0 ✅, nameref7.sub 4→0 ✅, nameref8.sub 28→24, nameref12.sub 46→24, nameref13.sub 16→4, nameref17.sub 64→47, nameref20.sub 46→36, nameref24.sub 9→2. Total nix passing: **74/77** (unchanged). **varenv** unchanged at ~260.

## Phase 70 — Nameref for-loop, indirect expansion, declare validation (fixes 234–238)

1. **Nameref-aware `for` loop iteration** (`src/interpreter/commands.rs`) — `for ref in v1 v2` where `ref` is a nameref now updates the nameref target (changes what it points to) via `namerefs.insert()` instead of directly overwriting with `vars.insert()`. Matches bash's `execute_for_command` which does `nameref_cell(v) = savestring(val)` when the loop variable is a nameref. Loop items are validated as valid variable names; invalid names produce "invalid variable name" and skip the iteration (matching bash's `valid_nameref_value` check). Non-nameref for-loop variables now use `set_var()` for proper handling of integer attributes, exports, uppercase/lowercase transforms, and array element [0] assignment. Fixes nameref5.sub for-loop sections producing "I am first: invalid variable name" errors instead of iterating through nameref targets.

2. **`${!var}` for namerefs returns target name** (`src/expand/params.rs`) — `declare -n ref=foo; echo ${!ref}` now correctly returns "foo" (the nameref target name). Previously, the `ParamOp::Indirect` handler called `lookup_var` which resolved through the nameref chain and returned the *value* of the target variable, then tried to use that value as a variable name for indirect expansion (producing empty output). Now checks `ctx.namerefs.contains_key()` first and returns the resolved nameref chain target via `ctx.resolve_nameref()`. Fixes `${!ref}` producing empty string for all nameref variables across nameref5.sub, nameref4.sub, nameref20.sub, and the main nameref.tests.

3. **`unset` nameref readonly error uses resolved target name** (`src/builtins/vars.rs`) — `unset foo` where `foo` is a nameref to readonly `bar` now correctly reports "unset: bar: cannot unset: readonly variable" instead of "unset: foo: cannot unset: readonly variable". The error message in the `namerefs.contains_key(name)` branch now uses the `resolved` variable name instead of `name`. Fixes nameref3.sub and nameref5.sub unset error messages.

4. **`declare -n` self-reference and array validation** (`src/builtins/vars.rs`) — Three new validation checks for nameref declarations: (a) `declare -n x=x` now produces "nameref variable self references not allowed" instead of the incorrect "circular name reference" warning; (b) `declare -n x[3]=y` (subscripted nameref target) produces "reference variable cannot be an array" — checked both in the subscript processing branch (for `name[idx]=value` form) and in the nameref flag branch; (c) `declare -n x=y` where `x` is already a populated indexed or associative array also produces "reference variable cannot be an array" (only triggers for arrays with actual elements to avoid false positives from mapfile's nameref bug which may leave empty residual arrays). Fixes nameref6.sub from 9→0 diff lines.

5. **`unset -n` array cleanup** (`src/builtins/vars.rs`) — `unset -n ref` now also removes indexed arrays (`shell.arrays.remove(name)`) and associative arrays (`shell.assoc_arrays.remove(name)`) for the nameref variable, in addition to the existing cleanup of namerefs, vars, exports, and attributes. Previously, operations like `mapfile` that wrote to the variable name instead of through the nameref would leave residual array data after `unset -n`, causing subsequent `declare -n ref=target` to fail with "reference variable cannot be an array". Fixes nameref18.sub from ~255→~28 diff lines locally.

**Phase 70 summary:** Reduces **nameref** from ~587 to ~445 nix diff lines (~24% reduction, 142 lines eliminated). nameref5.sub reduced from ~8→~6, nameref6.sub reduced from ~9→0 (now passes), nameref18.sub reduced from ~255→~28 locally. Total nix passing: **74/77** (unchanged). **varenv** slightly improved from ~262→~260.

## Phase 60 — Set builtin array output, wait -p, subscript bracket fixes (fixes 226–233)

1. **Fix `set` builtin to output arrays and associative arrays** (`src/builtins/set.rs`) — `set` with no arguments now outputs indexed arrays as `name=([0]="val" [1]="val" ...)` and associative arrays as `name=([key]="val" ...)` alongside scalar variables, all sorted alphabetically by name. Previously `set` only output scalars, so `set | grep ^myarray=` returned nothing for arrays. Declared-but-unset variables (with no elements) are excluded. Uses `quote_for_declare` for value quoting and `quote_assoc_key` for assoc key quoting, matching bash's `set` output format.

2. **Implement `wait -p var` flag** (`src/builtins/trap.rs`) — `wait` now supports `-p var` to store the PID of the completed process in a variable. Supports plain variables, indexed array subscripts (`wait -p arr[0]`), and associative array subscripts (`wait -p A[$key]`). With `assoc_expand_once` ON, uses `rfind(']')` for bracket matching to allow `]` as an assoc key. Fixes assoc18.sub `wait -p A[$rkey] -n %2 %3` (was outputting `bad 1`, now correctly outputs `5: ok 1`).

3. **Implement `wait -n` with specific job specs/PIDs** (`src/builtins/trap.rs`) — `wait -n %2 %3` now waits for the next of the specified jobs to complete (was only waiting for any child). Resolves job specs (`%N`, `%%`, `%+`, `%-`, `%string`) to PIDs via the job table. Uses non-blocking `WNOHANG` polling loop for targeted jobs. Also handles `-f` flag and combined flags like `-fn`, `-np var`. Full rewrite of `builtin_wait` with proper option parsing.

4. **Fix `\]` backslash-escaped `]` in assoc subscript key lookup** (`src/expand/params.rs`) — `${m[\]]}` now correctly strips the backslash escape to look up key `]` in the associative array. After quote stripping and variable expansion, backslash escapes in the expanded key are processed (`\X` → `X`). Fixes assoc5.sub `echo ${myarray[\]]}` producing empty instead of `def`.

5. **Fix single-quote protection of `]` in `${...}` subscript bracket matching** (`src/lexer/dollar.rs`) — In `read_param_name_with_subscript`, single-quoted content inside array subscripts now prevents `]` from closing the bracket. When `'` is encountered (outside double quotes, at depth > 0), everything until the matching closing `'` is consumed as literal subscript text. The quote characters are kept as part of the subscript (they become part of the assoc key). Fixes assoc5.sub `echo "${myarray['a]=test1;#a']}"` which was producing "unexpected EOF while looking for matching `}'" instead of the correct value.

6. **Fix `declare`/`typeset` bracket validation for unbalanced brackets** (`src/builtins/vars.rs`) — `declare myarray["foo[bar"]=bleh` (after quote stripping: `myarray[foo[bar]=bleh`) now correctly reports "not a valid identifier" due to unbalanced brackets. Uses depth-tracking bracket matching: `[` increments depth, `]` decrements; must reach depth 0 for the subscript to be valid, and the closing `]` must be the last character of the name portion. Fixes assoc5.sub line 26.

7. **Fix `declare` bracket matching with `assoc_expand_once` ON** (`src/builtins/vars.rs`) — When AEO is ON and the variable is an existing associative array, `declare` uses first-`]` matching (find the first `]` after `[`, ignoring nested `[`) instead of depth-based matching. This allows keys containing `[` — e.g., `declare myarray["foo[bar"]=bleh` finds the first `]` at the end of the name, accepting key `foo[bar`. But `typeset foo["foo]bar"]=bax` (where first `]` is after `foo`, leaving stray `bar]`) is still correctly rejected. Fixes assoc9.sub line 120.

**Phase 60 summary:** Reduces **assoc** from ~37 to ~20 nix diff lines (46% reduction). Flips assoc5.sub, assoc9.sub, assoc18.sub to 0 diff in nix. Remaining assoc diffs: assoc1/2 hash iteration ordering (~20 lines). Total nix passing: **74/77** (unchanged).

## Phase 59 — Assoc subscript comsub expansion, bracket parsing, scope restoration (fixes 218–225)

1. **Fix double command substitution execution in assoc subscripts** (`src/expand/mod.rs`) — `$(...)` inside `${A[$(cmd)]...}` was expanded once in `expand_part` (for the `lookup_var` call to get `orig_val` and check `orig_set`) and again in `expand_param` (for the param op handling), producing duplicate stderr output. Fix: when `expand_part` has already pre-expanded comsubs in the subscript (detected by comparing `lookup_name_ref` to `expr.name`), create a new `ParamExpr` with the pre-expanded name and pass it to `expand_param`, avoiding re-execution. Fixes assoc16.sub producing 8 extra `stderr` lines (one per `${A[$(echo Darwin ; echo stderr>&2)]...}` pair).

2. **Fix `is_param_set` not stripping quotes from assoc subscript keys** (`src/expand/mod.rs`) — `${A['$(echo Darwin ; echo stderr>&2)']:-value}` was incorrectly returning `value` because `is_param_set` checked `assoc.contains_key("'$(echo Darwin ; echo stderr>&2)'")` (with surrounding quotes) instead of stripping them first. Now strip surrounding single or double quotes before `assoc.contains_key()`, matching the behavior in `lookup_var`. Fixes assoc16.sub `:-default` and `:+alt` operators returning wrong results for single-quoted keys.

3. **Fix `80's` key validation in `read`/`printf -v`** (`src/builtins/io.rs`) — When `assoc_expand_once` is OFF, `read a[80's]` and `printf -v a[80's]` (from expansion of `a[$b]` where `b="80's"`) now correctly report "not a valid identifier" by checking for unbalanced single/double quotes in the subscript. When `assoc_expand_once` is ON, quotes are accepted as literal key characters (matching bash's `skipsubscript` behavior). Also fix `printf -v` to skip `expand_assoc_subscript` when `assoc_expand_once` is ON, preserving `80's` as the literal key instead of stripping the quote to produce `80s`. Flips assoc9.sub to 0 diff.

4. **Fix `declare -a`/`-A` inside functions not creating empty local arrays** (`src/builtins/vars.rs`) — `declare -a a` inside a function where `a=7` exists globally was incorrectly carrying the global value as `a[0]="7"` (and similarly `declare -A a` creating `a["0"]="42"`). Root cause: `declare_local` saves the old value but doesn't clear `shell.vars`, so the subsequent array creation code found and converted the global scalar. Fix: when `make_local` is true, remove the scalar from `shell.vars` and create an empty array/assoc with `declared_unset`, matching bash behavior where `declare -a a` in a function creates a new empty local array. Flips assoc10.sub to 0 diff.

5. **Fix bracket parsing with `assoc_expand_once` for `]` key** (`src/builtins/io.rs`) — `printf -v A[]]`, `read A[]]`, and identifier validation now use `rfind(']')` (last `]`) instead of first `]` after `[` when `assoc_expand_once` is ON, allowing `]` as an associative array key. Previously, `find(']')` matched the first `]` (part of the key) and rejected the trailing `]` as stray characters. Applied consistently across all three validation points in `read` (argument parsing, var_names loop, and assignment extraction) and `printf -v` (validation and subscript extraction). Fixes assoc18.sub `printf -v`/`read` sections (~9 diff lines eliminated).

6. **Fix `declared_unset` not removed when `read` inserts into assoc array** (`src/builtins/io.rs`) — After `declare -A A` (which sets `declared_unset`), `read A[key] <<<value` was not clearing the `declared_unset` flag via `shell.declared_unset.remove()`. Subsequent `unset A[key]` followed by `declare -p A` would incorrectly show `declare -A A` (declared-but-unset format) instead of `declare -A A=()` (empty array format).

7. **Add `was_declared_unset` to `SavedVar` for proper scope restoration** (`src/interpreter/mod.rs`, `src/interpreter/commands.rs`, `src/interpreter/pipeline.rs`) — Function-local `declare` that sets `declared_unset` (e.g., `declare -a a` creating empty array) now properly saves and restores the `declared_unset` state on function return, preventing the flag from leaking to the outer scope. Applied in both `run_function` scope restoration and `teardown_funsub_scope`.

**Phase 59 summary:** Reduces **assoc** from ~71 to ~37 nix diff lines (48% reduction). Flips assoc9.sub, assoc10.sub, assoc16.sub to 0 diff. Reduces **varenv** from ~281 to ~279 nix diff lines. Remaining assoc diffs: assoc1/2 (hash iteration ordering ~22 lines), assoc5 (bracket parsing in keys ~10 lines), assoc18 (`wait -p` unimplemented ~1 line). Total nix passing: **74/77** (unchanged).

## Phase 39 — Braces regression fix, umask/ulimit/hash improvements (fixes 213–217)

1. **Fix braces Phase 37 regression: `'$('` inside `"${a-...}"` default values** (`src/lexer/word.rs`) — When `in_squote` is true (inside `'...'` that protects `}` in double-quoted `${...}` default/alt values), `$` followed by `(` previously triggered the full recursive comsub parser via `parse_dollar`, which would consume past the `}` delimiter and cause an "unexpected EOF while looking for matching `}'" fatal error, aborting the entire script. Fix: added a bounded paren-depth scanner that runs instead of `parse_dollar` when `in_squote` is true and `$(` is encountered. The scanner counts paren depth with quote awareness (single quotes, double quotes, backticks, nested `$(...)`), stopping at the first unquoted `'` (the squote boundary). If a matching `)` is found within the squote region, a normal `CommandSub` node is produced (so `'$(echo hello)'` inside `"${a-...}"` still expands correctly). If no matching `)` is found (e.g., `'$('` with no closing paren), a `SILENT_COMSUB` marker is produced instead of a `SyntaxError` — this suppresses the echo output without printing an error message, matching bash's observable behavior where `extract_dollar_brace_string` calls `skip_single_quoted` to skip `$(` entirely during the extraction phase. Flips **braces** from ~77 nix diff lines to 0.

2. **Implement full POSIX symbolic umask** (`src/builtins/misc.rs`) — Rewrote `builtin_umask` symbolic mode parsing to support the complete POSIX grammar: multiple operators per clause (`u=r+w` means set read then add write; `u+w=r+x` means add write, then set read, then add execute), permission copying between classes (`g+u` copies user's allowed perms to group; `o=u` sets other to match user), `X` conditional execute (sets execute only if any execute bit is currently allowed in the intermediate mask), and `s`/`t` flags (ignored for umask). Uses `class_perms` helper to extract the 3-bit rwx allowed permissions for a class from the current mask, and `expand_perm` to apply a 3-bit value to the positions selected by the who-mask. Eliminates ~12 nix diff lines from the builtins test.

3. **Rewrite `ulimit` builtin** (`src/builtins/trap.rs`) — Full reimplementation supporting all bash 5.3 resource flags: `-c` (core), `-d` (data), `-e` (nice), `-f` (fsize), `-i` (sigpending), `-k` (msgqueue), `-l` (memlock), `-m` (rss), `-n` (nofile), `-p` (pipe), `-q` (msgqueue), `-r` (rtprio), `-s` (stack), `-t` (cpu), `-u` (nproc), `-v` (as), `-x` (locks), `-P` (pseudoterminals), `-R` (rttime), `-T` (threads). Supports `-S`/`-H` soft/hard limit selection (defaults to soft for display, both for set), `soft`/`hard`/`unlimited` value keywords, `--` option terminator, `+N` rejection with "invalid number" error, `-a` flag for printing all limits, proper value scaling (512-byte blocks for `-c`/`-f`, 1024-byte for `-d`/`-l`/`-m`/`-s`/`-v`), combined flag parsing (`-Sc`, `-Hn`), and `nix::errno::Errno::last().desc()` for strerror-style error messages without Rust's `(os error N)` suffix. Eliminates ~8 nix diff lines from the builtins test.

4. **Fix `checkhash` shopt behavior** (`src/interpreter/commands.rs`) — When `shopt -s checkhash` is enabled and a hashed command path doesn't exist on disk, the stale hash table entry is now removed and command lookup falls back to `$PATH`. If the `$PATH` lookup succeeds, the newly found path is re-added to the hash table so that subsequent `hash -t` lookups work correctly. Previously, stale hash entries were used unconditionally regardless of the `checkhash` setting. Reads the option from `shell.shopt_options` HashMap. Eliminates 2 nix diff lines from the builtins test.

5. **Fix exec error messages for hashed paths** (`src/interpreter/commands.rs`) — When a command's path came from the hash table (e.g., after `hash -p /nosuchdir/nosuchfile cat`) and the subsequent exec fails with `ENOENT`, the error now reports the actual hashed path (`/nosuchdir/nosuchfile: No such file or directory`) instead of just `cat: command not found`. Tracks a `from_hash_table` boolean through the exec path to choose the appropriate display name. Eliminates 2 nix diff lines from the builtins test.

**Phase 39 summary:** Flips **braces** to passing (0 nix diff, was ~77). Reduces **builtins** from ~28 to ~3 nix diff lines (only `BASH_CMDS[cmd]=path` hash table sync remains). Total nix passing: **75/77** (was 74/77).

## Phase 32 — Compound array assignment in local/declare (fixes 210–212)

1. **Fix `local b=("${!1}")` compound array assignment detection** (`src/interpreter/commands.rs`) — The `is_quoted_arg` guard in `run_simple_command`'s compound assignment handler blocked detection when the word contained `DoubleQuoted` parts (e.g., `"${!1}"`), even though the `(` was literally in the source code. When `has_literal_paren` is true (verified via AST word part inspection), compound assignment is now allowed regardless of whether the word also contains double-quoted parts. The `is_quoted_arg` check was only relevant when the `(` came from expansion (where `has_literal_paren` would be false), so it was removed entirely — `has_literal_paren` already provides the necessary distinction. This fixes `new-exp12.sub` where `local a=("${!1}")` with `$1=array_1[@]` was incorrectly treated as a scalar assignment `a="(HELLO)"` instead of an array `a=([0]="HELLO")`.

2. **Fix `local` compound array scope restoration** (`src/interpreter/commands.rs`) — `declare_local` (which saves the old value for restoration on function exit) was called AFTER the compound assignment handler had already overwritten the array via `self.arrays.insert()`. This caused local array variables to leak into outer scope — e.g., `local array_1=('HELLO')` inside a function would persist `HELLO` after the function returned instead of restoring the original value. Fix: call `declare_local(name)` BEFORE performing the compound assignment so the previous value is properly saved. The subsequent `builtin_local` call (which receives just the name) is a no-op since the scope already contains the variable.

3. **Fix `"${!ref}"` word splitting in compound array assignments** (`src/interpreter/commands.rs`) — When `"${!ref}"` where `ref=arr[@]` appeared as a compound assignment element (e.g., `local b=("${!2}")`), the value was expanded via `expand_word_single` which joins `"$@"`-like splits with space, losing element boundaries. Then `parse_indexed_compound_assignment` would re-split on whitespace, incorrectly splitting `"1 foo"` into separate elements `1`, `foo`. Fix: when the compound assignment content contains `DoubleQuoted` word parts (or `\x1F` element separators from the parser), re-expand from the original word parts using `expand_word_fields`, which preserves `SplitHere` markers from `"${!ref}"` with `[@]` as separate fields. This makes `local b=("${!2}")` with `$2=array_2[@]` (where `array_2=("1 foo" "2 foo")`) correctly produce `b=([0]="1 foo" [1]="2 foo")` instead of `b=([0]="1" [1]="foo" [2]="2" [3]="foo")`.

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