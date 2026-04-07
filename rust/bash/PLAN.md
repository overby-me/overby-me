# Bash Test Suite — Plan

## Current State

**66/77 nix tests verified passing** (Phase 31), ~67/83 local tests passing (0 diff, sequential) on bookmark `bash-integration-test`. Goal: full drop-in bash replacement (keeping readline builtins like `compgen`/`complete` available). **read** now flaky in nix (sandbox `/dev/tty` timing), **comsub** occasionally passes. Nix harness now normalizes BASHPID/PPID/ref_PID and `$_` nix store paths — **varenv** and **nameref** should flip to passing in nix once verified. **new-exp** now ~4 diff lines locally (only `local b=("${!1}")` compound assignment in new-exp12).

See `CHANGELOG.md` for full fix history (200+ fixes across 31 phases).

### Nix test results (66/77 verified passing, likely 68+ with Phase 29 harness fixes)

Verified passing (66/77): alias, appendop, arith-for, **array2** ✅, **attr** ✅, braces, case, casemod, **comsub-eof** ✅, comsub-posix, cond, coproc, cprint, **dirstack** ✅, dollars, dynvar, errors, execscript, **exp-tests** ✅, **exportfunc** ✅, extglob, extglob2, extglob3, func, getopts, glob-bracket, glob-test, globstar, **heredoc** ✅, herestr, ifs, ifs-posix, **input-test** ✅, invert, iquote, mapfile, more-exp, nquote, nquote1, nquote2, nquote3, nquote4, nquote5, parser, posix2, posixexp, **posixexp2** ✅, posixpat, posixpipe, precedence, printf, **procsub** ✅, quote, **read** ✅, redir, rhs-exp, **set-e** ✅, set-x, shopt, strip, **test** ✅, tilde, tilde2, type, **vredir** ✅ — **quotearray** now 0 diff locally (Phase 28), pending nix verification — **varenv** and **nameref** now PID-only locally (Phase 29 `$_` fix + nix harness PID normalization should flip them)

Verified failing (11/77): arith (nix-only: arith10.sub error format diffs + `let` empty subscript handling in assoc_expand_once mode), array (nix-only: array32/33.sub new tests), assoc (nix-only: tilde expansion in subscripts + empty array formatting), builtins (~130, help output + ulimit flags), comsub (1, flaky SIGPIPE), comsub2 (~8, funsub line number off-by-1), lastpipe (1, flaky SIGPIPE), nameref (nix: nameref resolution bugs revealed by sandbox), new-exp (~4 locally, was ~96 before Phase 31; remaining: `local b=("${!1}")` compound assignment in new-exp12; UTF-8 encoding fixed via PUA raw byte tracking), quotearray (~8, single-quoted `(( ))` error format + `@`/tab key quoting now fixed + `]` invalid operator and `$(echo)` execution issues now fixed), trap (1, flaky extra CHLD)

Note: **read** now flaky in nix (1 line diff, `read -t 1 < /dev/tty` sandbox timing). **comsub** occasionally passes (flaky SIGPIPE). **arith**, **array**, **assoc** pass locally (0 diff) but fail in nix sandbox due to stricter environment revealing edge cases. **varenv** and **nameref** now PID-only locally (Phase 29 `$_` init fix); nix harness updated to normalize BASHPID/PPID/ref_PID and `$_` paths. **builtins** PID-only locally but has `help` output + `ulimit` diffs in nix.

**Phase 31 fixes:** Implement `shopt -s nocasematch` support for pattern matching — add `NOCASEMATCH_ENABLED` thread-local flag (like `DOTGLOB_ENABLED` etc.), propagate from `shell.shopt_nocasematch` in `expand_word_fields`, `expand_word_single`, `run_case`, and `run_conditional`. Pattern matching (`pattern_match_impl` in both `pattern.rs` and `commands.rs`) now does case-insensitive comparison for literal chars, bracket expressions, ranges, and `\x00`-escaped literals when nocasematch is on. `[[ =~ ]]` regex wraps pattern with `(?i)` prefix. Fix `"${!ref}"` indirect array expansion in double-quoted context — when indirect target ends with `[@]`, produce `SplitHere` markers between elements (like `"$@"`); when target ends with `[*]`, join with IFS. Fix `${arr[@]:offset:length}` array slice arithmetic offset parsing — replace all `offset_str.trim().parse().unwrap_or(0)` in `get_array_elements` with `parse_arith_offset()` calls so that expressions like `${#x[@]}-1` are evaluated arithmetically instead of defaulting to 0. Add `cmd_sub: CmdSubFn` parameter to `get_array_elements` for this. Fix `${!name[@]@Q}` / `${!name[@]%b}` lexer parsing — `${!name[@]}` (followed by `}`) is array indices, but `${!name[@]@Q}` (followed by operator) is now correctly treated as indirect expansion (resolve `name[@]`, apply operator on result). Fix `${!target@Q}` invalid variable name error — when indirect expansion resolves to an invalid variable name (e.g. `aaa bbb` from array `[@]` join), emit `invalid variable name` error and set `arith_error` flag to abort the command. Fix `${VAR[@]@A}` for declared-but-unset scalars — when the base variable is a scalar with attributes (not in arrays/assoc_arrays), produce `declare -FLAGS name` instead of empty. Fix `${arr[@]@A}` for declared-but-unset arrays — omit `=()` when `__UNSET__` marker is set, but keep `=()` for explicitly empty arrays (e.g. `B=()`). Add `${var@K}`/`${var@k}` key-value transform operators — for indexed arrays produce `idx "val"` pairs (K) or `idx val` pairs (k); for assoc arrays produce `key "val"` pairs; for scalars/positional params produce single-quoted values; `"${arr[@]@k}"` in double quotes produces `SplitHere`-separated key/value words. Add `parse_arith_offset` pre-expansion of `${...}` and `$(...)` in offset expressions. Fix `string_to_raw_bytes` UTF-8 encoding — introduce PUA-based raw byte tracking (U+E000–U+E0FF) so that `$'\xNN'`/`\NNN` escape sequences produce PUA-encoded characters that `string_to_raw_bytes` converts to single bytes, while regular Unicode characters (from source code) are output as proper UTF-8. Update `shell_quote`, `shell_escape` (printf %q), `quote_for_declare`, and `quote_assoc_key` to decode PUA chars to their original byte values when quoting. Add `char_in_class` helper for POSIX character class matching that decodes PUA chars before classification (e.g. `[[:cntrl:]]` correctly matches PUA-encoded `$'\003'`).

**Phase 31 improved new-exp** — from ~96→~4 diff lines locally (excluding PIDs). `new-exp4.sub` 0 diff (indirect `"${!xx}"` array `[@]` splitting for Case08). `new-exp5.sub` 0 diff (arithmetic offset `${#x[@]}-1` in array slices). `new-exp8.sub` 0 diff (nocasematch in `${var//PAT/rep}` and `[[ ]]` pattern matching). `new-exp9.sub` 0 diff (indirect `"${!tmp}"` where `tmp=arr[@]` produces separate fields). `new-exp10.sub` 0 diff (PUA raw byte fix for `printf '%q'` on `$'\001'` values). `new-exp11.sub` 0 diff (PUA raw byte fix preserves UTF-8 encoding of source-code multibyte chars like `Ã¥`). `new-exp13.sub` 0 diff (`${!varname[@]@Q}` indirect expansion, `${!VAR4[@]@Q}` invalid variable name error, `${VAR1[@]@A}` declared-but-unset scalar, `${VAR3[@]@A}` declared-but-unset array without `=()`). `new-exp14.sub` 0 diff (`@K`/`@k` key-value transform). Remaining: `new-exp12.sub` (~4 lines, `local b=("${!1}")` compound assignment in declare context).

**Phase 30 fixes:** Fix `${!var@Q}` indirect expansion combined with transform operators — lexer now detects `@X` transform after indirect name before the name prefix check, preventing misparse as `${!prefix@}`. Fix `${array[@]@Q}` per-element transform — add `Transform(ch)` handling to `apply_param_op` (was falling through to `_ => val.to_string()`). Fix `${!arr[@]@Q}` parsing — extend `ArrayIndices(char)` to `ArrayIndices(char, Option<char>)` to carry optional transform. Extract `shell_quote`/`expand_backslash_escapes` into `transform_helpers` module. Fix `${var@A}` for declared-but-unset variables (omit `=''` suffix) and plain variables (omit `declare --` prefix). Fix `${@@A}`, `${arr[@]@A}`, `${arr[@]@a}` to produce proper declaration format (`set --`, `declare -a`, `declare -A`). Fix transform operators for truly unset variables to return empty (not `''`). Fix `${var@C}`/`${var@}` to produce bad substitution (bash 5.3). Fix `pattern_replace` for zero-length extglob matches (`?(b)`, `*(b)`) — empty matches now found at position 0 for replace-first, and at every position for replace-all. Remove `set -o emacs`/`set -o vi` (not available without readline).

**Phase 30 improved new-exp** — from ~145→~96 diff lines locally (excluding PIDs). `new-exp10.sub` improved ~13→~8 diff lines (unset `@Q`/`@A` fix, `${@@A}` set -- format, `${arr[@]@A}` declare format, `set -o emacs` error). `new-exp13.sub` improved significantly (`${!var@Q}` indirect+transform, `${VAR[@]@A}` declared-but-unset, `${!varname[@]@Q}` array indices+transform). `new-exp9.sub` improved (zero-length extglob pattern matching).

**Phase 29 fixes:** Fix `&` replacement quoting in `${var/pat/rep}` pattern substitution — `expand_replacement_word` now preserves quoting context for `&` using `\x00` markers. Quoted `&` (inside `"..."` or `'...'`) stays literal, unquoted `&` means matched text when `patsub_replacement` is on. `\&` → literal `&`, `\\&` → `\` + matched text (from variable expansions too). Fix tilde expansion in replacement strings — `~` at the start of an unquoted replacement is expanded to `$HOME`. Fix `$_` initialization — `$_` is now set to the shell's own absolute path at startup (via `/proc/self/exe`), matching bash behavior. Update nix test harness — normalize BASHPID/PPID/ref_PID values and `$_` nix store paths to eliminate false PID failures.

**Phase 29 improved new-exp** — `new-exp16.sub` now 0 diff (was ~36 diff lines). Tilde expansion in replacement, `&`/`\&`/`\\&` quoting with `patsub_replacement` on/off, quoted vs unquoted `&` in `"..."` and `'...'` all match bash. **Phase 29 improved varenv** — `$_` diff eliminated (was 2 lines from inherited `timeout` path). **Phase 29 improved nameref** — `$_` diff eliminated. Both varenv and nameref now PID-only locally, and nix harness normalizes PIDs.

**Phase 28 fixes:** Fix single-quoted `(( ))` arithmetic expressions — `'` is now treated as literal in arithmetic (not quoting), matching bash behavior. In `expand_comsubs_in_arith`, when the identifier before `[` is preceded by `'`, don't activate array bracket protection — this allows `$var` expansion to proceed with backslash-escaping of `]`, `[`, `$` in the result. Add `'` to the "operand expected" check at the start of `eval_arith_expr_inner` so expressions starting with `'` produce the correct `((:` prefix error. Fix `printf -v array[@]` and `printf -v array[*]` for indexed arrays — report "bad array subscript" instead of arithmetic error on `@`/`*`, and continue execution (don't abort the script).

**Phase 28 improved quotearray** from ~8→0 diff lines locally (single-quoted `(( 'assoc[$key]++' ))` now expands `$key` with backslash-escaping and uses `((:` error prefix; `printf -v array[@]` now correctly reports "bad array subscript" and preserves the array). **Phase 28 improved quotearray nix** (single-quote error diffs eliminated, printf -v fix eliminates 2 nix diff lines; remaining nix diffs are in quotearray3/4/5.sub — `unset` quoting, `test -v` with `@` keys, `assoc_expand_once` interactions).

**Phase 27 fixes:** Add "invalid arithmetic operator" detection for `]`, `@`, `{`, `}`, `.`, `;`, `\`, `'` after valid identifiers — when a character that's not a valid arithmetic operator follows an identifier or number, report "invalid arithmetic operator" with the error token starting at that character. Fix `expand_comsubs_in_arith` to skip `$var` expansion inside ALL array subscripts (indexed and associative) — previously only associative array subscripts were protected. This prevents expanded values containing `]` from breaking bracket matching (e.g. `a[$key]` where `$key='x],b[$(echo uname >&2)'` no longer executes the `$(echo)` comsub). Fix depth-aware bracket matching in array element detection — use forward scanning with `[`/`]` depth tracking instead of `rfind(']')` which could match the wrong closing bracket. Fix subscript error reporting — temporarily clear `arith_top_expr` during subscript evaluation so errors show the subscript content (e.g. `x],b`) instead of the full outer expression. Fix `declare -p` `@` key quoting — `@` now included in `quote_assoc_key` needs-quoting set. Fix `declare -p` non-printable key formatting — tab, newline, control chars now use `$'...'` ANSI-C quoting instead of literal embedding in `"..."`.

**Phase 27 improved quotearray** from ~27→~8 diff lines locally (invalid arithmetic operator detection + `$var` expansion skip in indexed array subscripts + bracket depth matching fixes eliminate 10 diff lines from `]`-in-subscript errors, spurious `$(echo uname)` execution, and duplicate error messages; `@` key quoting and `$'\t'` formatting improve nix diffs). **Phase 27 improved new-exp** to PID-only diffs locally (was ~16).

**Phase 26 fixes:** Fix `assoc[$var]+=1` append assignment parsing when subscript contains quotes/brackets — `]+=` detection in array subscript assignment context (previously only `]=` was detected). Add tilde expansion in associative array subscript keys (`aa[~/path]=val` → expands `~` to `$HOME`). Add tilde expansion in compound array element values and keys (`declare -A aa=([~/key]=~/Desktop)` → tildes expand in both). Fix `${!prefix* }` bad substitution detection (trailing content after `*`/`@`). Fix `${!1*}` and `${!@*}` bad substitution (prefixes starting with digits or special chars now correctly rejected).

**Phase 26 improved quotearray** from ~32→~27 diff lines locally (`]+=` append assignment fix eliminates `command not found` error + corrects `declare -p` output). **Phase 26 improved assoc** for nix — tilde expansion in subscript keys and compound assignment values now matches bash behavior.

**Phase 25 fixes:** Fix `${A[${a[i]}]}` nested subscript expansion — `${}` inside array subscripts now correctly uses brace-depth-aware matching to find the closing `}`, then recursively calls `lookup_var` to resolve the inner expression. Previously, `rest.find('}')` matched the first `}` which could be inside nested `${...}`, causing misparse. Investigated comsub/funsub LINENO counting — traced bash's `parse_and_execute` line counting model through `shell_getc`, `yy_string_get`, and YACC grammar. Discovered bash counts lines via `shell_getc`'s line-buffer refill (not per-`\n` character), so `\n` after `;` doesn't increment `line_number` in string-eval contexts. Our character-level lexer has fundamentally different counting. The funsub off-by-1 in comsub2 is actually caused by compound commands (`for`/`while`) adding extra line counts in bash's `parse_and_execute` model. Left as known issue.

**Phase 25 improved quotearray** from ~36→~32 diff lines locally (nested subscript `${A[${a[i]}]}` fix eliminates 2 "bad array subscript" errors + 2 missing values).

**Phase 23 fixes:** Fix `\x00`-quoted literal patterns in `pattern_replace` (`\?` in unquoted `${a//\?/X}` now correctly matches literal `?`). Fix `"${@}"` and `"${*}"` with braces to split/join like `"$@"`/`"$*"`. Fix `"$xxx${@}"` with no positional params producing spurious empty arg. Fix `${!foo}` indirect expansion where `foo=@` to produce SplitHere markers in double-quoted context. Fix empty element removal in unquoted `${@%%pattern}`. Fix `${var/#/x}` empty prefix replacement (prepends replacement). Fix `${var///a/}` parsing — `/` after `//` is now part of the pattern. Fix `${var///}` to remove all slashes.

**Phase 23 flipped to passing:** exp (~8→0 diff, `\?` pattern fix + `"${@}"` splitting + `"$xxx${@}"` empty removal), posixexp2 (~41→0 diff, `///` pattern parsing fix).

**Phase 23 reduced diffs:** new-exp (~22→~12, `///a/` pattern parsing + empty element removal + `${var/#/x}` prefix fix; remaining: `'}'` in dquote `${}` default values + PID diffs), quotearray (~26→~24).

**Phase 22 fixes:** Fix command substitution LINENO off-by-one: multi-line `$(\ncmd)` now reports correct line numbers matching bash (first content line = `$(` line). Root cause was `set_line_offset` using relative `+=` after the parser constructor already consumed the leading `\n`; replaced with absolute `set_line_number()` for comsub contexts. Fix `${!prefix*}` to join with first char of IFS (like `"$*"`) instead of always space. Fix `"${!prefix@}"` to split into separate words (like `"$@"`). Fix `$(< $var)` to expand variables/tilde/globs in filenames (was wrapping raw text in `Literal` instead of parsing into word parts). Fix `$(< file)` inside double quotes (separate code path was also not expanding). Fix `$(< nonexistent)` to report error and set exit status 1 (was silently returning empty). Fix `${var:?message}` error prefix to use `EXPAND_ERROR_PREFIX` instead of hardcoded `"bash:"` (now shows script name + line number in `-c` mode with `$0`).

**Phase 22 flipped to passing:** heredoc (~4→0 diff, comsub LINENO off-by-one fixed in heredoc7.sub case 2).

**Phase 22 reduced diffs:** new-exp (many diffs eliminated: `${!prefix*}` IFS separator, `$(< $var)` expansion, `${var:?}` error prefix; remaining: `&` in replacement strings, `//a` vs `/` path simplification).

**Phase 21 fixes:** `let "a[\"\"]"=22` now correctly assigns to `a[0]` when `assoc_expand_once` is unset (empty subscript evaluates to 0 in `let` context), but still errors when `assoc_expand_once` is set (matching bash). `${var/#pat/rep}` and `${var/%pat/rep}` now use longest match (e.g. `${x/#*/yyy}` replaces entire string, not just empty prefix). `pattern_replace` optimized with fast paths: literal patterns use O(n) `str::replace`, single-char patterns (`?`, `[...]`, `[[:class:]]`) use O(n) per-char matching, fixed-length patterns (no `*`) check only one substring length per position, extglob patterns (`*(...)`, `?(...)`, etc.) correctly computed as variable-length. This fixes new-exp8.sub timeout (10K-char `${z//str}` went from >60s to <1s).

**Phase 21 reduced diffs:** new-exp (~50→~30, pattern replacement with `#*/` and `%*/` anchors now correct, new-exp8.sub performance test now completes), arith (let empty subscript fixed for non-assoc_expand_once case, error format diffs remain).

**Phase 20 fixes:** Associative array subscript expansion in assignments (`A[$key]=val` now expands `$key`), proper quote handling in subscripts (single-quoted content is literal, double-quoted expands), arithmetic bracket depth tracking (operators inside `[...]` subscripts no longer split expressions), bare array variable names in arithmetic resolve to element [0] (`$((x))` where `x` is an array), `~-N` bitwise NOT with negative operand, `declare -p` assoc key quoting matches bash (only shell-special chars are quoted), `eval_arith_full` now receives real arrays/assoc_arrays for proper `${string:A[%]:A[$k1]}` offset evaluation, space-only subscripts like `a[" "]` now correctly evaluate to index 0 instead of erroring.

**Phase 20 reduced diffs:** quotearray (~68→~36 locally, improved in nix too), arith (fixed `456 123` reorder bug and `$((a[0]))` non-numeric value recursion, but arith10.sub `let` empty subscript handling now shows more diffs ~10→~49 in nix).

**Phase 19 flipped to passing:** attr (~4→0 diff, readonly error prefix fix + single-quoted compound assignment scalar treatment), exportfunc (~2→0 diff, funsub `$()` terminator detection in redirect targets), read (1→0 diff, poll revents fix). Also newly verified passing: array2, dirstack, input-test, procsub, set-e, test.

**Phase 19 reduced diffs:** arith (~16→~10, empty subscript `a[""]=N` validation in `(( ))` context)

**Phase 18 fixes:** xtrace atomic writes (pipeline interleaving fix), funsub `set -e` disabled in non-posix mode, bad interpreter shebang error messages, `${scalar[@]:offset:length}` character-level substring, associative array subscripts in arithmetic evaluation

**Phase 17 flipped to passing:** comsub-eof (1→0 diff, incomplete comsub detection fix + heredoc EOF warning on parse errors), heredoc3.sub (1→0 diff, subshell EOF error reporting)

**Phase 16 flipped to passing:** arith (~90→0 diff, duplicate error fix + subscript quote handling), array (~5→0 diff, subscript error ordering + brace expansion + bad subscript error format), varenv (~6→PID-only, set -k expansion ordering), builtins (~18→PID-only), heredoc (~12→PID-only)

**Phase 15 flipped to passing:** assoc (462→0 diff), quotearray (179→0 diff, from IFS fix), new-exp (310→0 diff), nameref (678→PID-only diff), trap (1→0 locally, may still be flaky in nix)

Failing (~11 nix):

| Test | Local diff | Nix diff | Notes |
|------|-----------|----------|-------|
| trap | 0 ✅ | 1 | Flaky — timing-dependent signal delivery (extra CHLD) |
| comsub | 0 ✅ | 1 | Spurious `echo: write error: Broken pipe` (flaky timing, sometimes passes) |
| lastpipe | 0 ✅ | 1 | Spurious `echo: write error: Broken pipe` (flaky timing) |
| comsub2 | ~12 | ~12 | Line number off-by-1 in funsubs (compound command line counting) |
| new-exp | ~4 | ~40 | `local b=("${!1}")` compound assignment (new-exp12); nix may also have `'}'` quoting diffs; UTF-8 encoding fixed via PUA |
| quotearray | ~8 | ~60 | Single-quoted `(( ))` error format (2 lines); nix has `declare -p` + `[[` + `unset` diffs |
| arith | 0 ✅ | ~50 | Nix-only: arith10.sub error format + `let` empty subscript in assoc_expand_once |
| array | 0 ✅ | ~20 | Nix-only: array32/33.sub differences |
| assoc | 0 ✅ | ~20 | Nix-only: bash 5.3 `A[]]`/`A[[]` bracket parsing, `$'\001'` key handling, compound assignment diffs |
| builtins | ~18 (PID) | ~130 | `help` output formatting, `ulimit -g` flag, PID diffs |
| nameref | ~16 (PID) | ~678 | Nix reveals nameref resolution bugs (`aa&bb`, nounset, indirect) |
| varenv | ~4 (PID) | ~6 (PID) | PID-only locally after Phase 29 `$_` fix; nix harness now normalizes BASHPID/PPID/`$_` |

**Phase 29 improved new-exp** — `new-exp16.sub` from ~36→0 diff (`&` replacement quoting + tilde in replacements). **Phase 29 improved varenv** from ~6→~4 PID-only (`$_` init fix). **Phase 29 improved nameref** — `$_` diff eliminated.

**Phase 28 improved quotearray** from ~8→0 diff lines locally (single-quoted `(( ))` fix + `printf -v array[@]` fix). **Phase 27 improved quotearray** from ~27→~8 diff lines locally (invalid arithmetic operator detection + indexed array subscript `$var` expansion fix + `@` key quoting + `$'\t'` non-printable key formatting). **Phase 26 improved quotearray** from ~32→~27 diff lines locally (`]+=` append assignment fix). **Phase 25 improved quotearray** from ~36→~32 diff lines locally (nested subscript `${A[${a[i]}]}` fix). **Phase 23 improved quotearray** from ~26→~24 diff lines locally (empty element removal fix). **Phase 20 improved quotearray** from ~68→~36 diff lines locally by fixing assoc subscript expansion, arithmetic bracket depth tracking, and `declare -p` key quoting. No remaining local diffs. Nix diffs remain in quotearray3/4/5.sub (unset quoting, test -v with @ keys, assoc_expand_once).

### Local test results (~67/83 passing, 0 diff sequential — Phase 31)

83 total `.tests` files in `/tmp/bash-5.3/tests/` (superset of the 77 nix tests — includes dbg-support, dbg-support2, dstack2, histexp, history, rsh, invocation, jobs, posixpipe, and others not in the nix harness). **dstack2** now passes (was 26 diff lines — `~N`/`~+N`/`~-N` tilde expansion implemented). **arith**, **array**, **assoc**, **exp**, **posixexp2**, **comsub**, **lastpipe**, **trap**, **quotearray** now pass locally (0 diff). **new-exp** now ~4 diff lines locally (was ~96 pre-Phase 31, down from ~145 pre-Phase 30): only remaining diff is `local b=("${!1}")` compound assignment in new-exp12. UTF-8 encoding (new-exp11) and printf %q quoting (new-exp10) fixed via PUA raw byte tracking. **nameref**, **varenv**, **builtins**, **read**, **heredoc**, **procsub**, **extglob**, **type**, **glob** have PID-only diffs.

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

## Failing Nix Tests (~11/77)

### Near-passing (1-line diffs, likely flaky)

- **trap** (1 line) — Timing-dependent signal delivery (extra CHLD)
- **comsub** (1 line) — Spurious `echo: write error: Broken pipe` (SIGPIPE timing race in nix sandbox)
- **lastpipe** (1 line) — Spurious `echo: write error: Broken pipe` (SIGPIPE timing race in nix sandbox)

### Medium diffs

- **comsub2** (~8 lines) — Line number off-by-1 in funsubs. Root cause: bash's `parse_and_execute` counts lines via `shell_getc` line-buffer refills (not per-`\n` character), and compound commands (`for`/`while`) add extra line increments. Our character-level lexer counts differently.
- **new-exp** (~4 lines locally excluding PIDs, was ~96 pre-Phase 31) — ~~UTF-8 encoding in new-exp11~~ ✅ Fixed via PUA raw byte tracking. `local b=("${!1}")` compound assignment in new-exp12 (parser-level array assignment in `local`/`declare` doesn't preserve `"${!ref}"` word splitting). `'}'` quoting in dquote `${}` default values may still affect nix.
- **quotearray** (~40 lines) — 0 diff locally (Phase 28 fixed single-quoted `(( ))` + `printf -v array[@]`). Nix diffs in quotearray3/4/5.sub: `unset` with complex quoting and `$(echo foo)` keys, `test -v`/`[[ -v ]]` with `@` key for assoc arrays, `assoc_expand_once` interactions, `declare -a array` vs `declare: array: not found` version differences

### Nix-only failures (pass locally, fail in nix sandbox)

- **arith** — Passes locally (0 diff). Nix reveals arith10.sub error format diffs + `let` empty subscript handling in `assoc_expand_once` mode
- **array** — Passes locally (0 diff). Nix reveals array32/33.sub differences
- **assoc** — Passes locally (0 diff). Nix reveals bash 5.3 `A[]]`/`A[[]` bracket parsing, `$'\001'` key handling in compound assignments, and other bash 5.3 differences
- **nameref** — PID-only locally. Nix reveals nameref resolution bugs (invalid variable names like `aa&bb`, nounset behavior with namerefs)
- **varenv** — PID-only locally. Nix reveals `declare` output format diffs, `local` error handling

### Larger diffs

- **builtins** (~130 nix lines) — `help` output formatting, `ulimit -g` not recognized, PID diffs (PID-only locally)

### Now passing (Phase 23 fixed)

- **~~exp~~** (~8→0 lines) — Fixed `\x00`-quoted literal patterns in `pattern_replace` min/max length computation (`\x00` prefix was counted as a character instead of a quoting escape). Fixed `"${@}"` with braces to produce SplitHere markers like `"$@"`. Fixed `"$xxx${@}"` with no positional params to produce zero fields (was producing spurious empty arg). ✅
- **~~posixexp2~~** (~41→0 lines) — Fixed `${var///a/}` parsing: `/` immediately after `//` is now included in the pattern (was treated as the pattern/replacement separator, giving empty pattern). Only applies to ReplaceAll/ReplaceFirst modes, not prefix/suffix. ✅

### Now passing (Phase 22 fixed)

- **~~heredoc~~** (~4→0 lines) — Fixed comsub LINENO off-by-one: `set_line_number()` (absolute set) replaces `set_line_offset()` (relative add) for comsub contexts, so the leading `\n` consumed during parser construction doesn't shift line numbers. Fixes heredoc7.sub case 2 (`cat <<EOF && grep $(`) line number diffs. ✅

### Previously listed as failing, now verified passing locally

- **arith** — Now passes locally (0 diff). May still show diffs in nix due to arith10.sub error format.
- **array** — Now passes locally (0 diff). May still show diffs in nix for array32/33.sub.
- **assoc** — Now passes locally (0 diff). May still show diffs in nix for tilde expansion.

### Now passing (Phase 19 fixed)

- **~~attr~~** (~4→0 lines) — Fixed readonly error prefix: `readonly -a r='(7)'` now uses `readonly:` as context (not function name). Single-quoted compound values without `-a` flag treated as scalar (not compound assignment). ✅
- **~~exportfunc~~** (~2→0 lines) — Fixed funsub `$()` terminator detection: `$( )` closing paren inside funsub no longer incorrectly sets the command-terminator flag, so `${ $() }` without `;` correctly fails to parse. ✅
- **~~read~~** (1→0 lines) — Fixed `read -t` poll revents bug: `poll_fd.revents()` was read from the original variable instead of the mutable array element after `poll()`, always returning empty flags. Non-poll timeout now also checks for POLLHUP without POLLIN. ✅

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

### Newly verified passing (Phase 19)

- **array2**, **dirstack**, **input-test**, **procsub**, **set-e**, **test** — All verified passing in nix (were previously not re-verified) ✅

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

| File                            | Contents                                                                                                                                                                                                                                                                            |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/ast.rs`                    | AST types, `WordPart` (includes `SyntaxError` variant), `ArrayIndices(char, Option<char>)` with optional transform (Phase 31: `[@]` with operators now parsed as indirect expansion instead)                                                                                         |
| `src/builtins/io.rs`            | `read` (prompt suppression on non-tty), `echo` (EPIPE handling), `printf`, `mapfile`                                                                                                                                                                                                |
| `src/builtins/exec.rs`          | `type`, `command`, `hash`                                                                                                                                                                                                                                                           |
| `src/builtins/flow.rs`          | `break`, `continue`, `exit`, `return`                                                                                                                                                                                                                                               |
| `src/builtins/vars.rs`          | `declare` (compound re-expansion, `+a` readonly fix), `local`, `export`, `let`, `unset` (scalar subscript error, `arr[@]` preserves empty array)                                                                                                                                    |
| `src/builtins/mod.rs`           | `parse_array_literal`, function body formatting, `quote_for_declare`, `quote_assoc_key` (shell-special-only quoting), `interpret_echo_escapes` (returns `(String, bool)` for `\c` stop)                                                                                             |
| `src/builtins/set.rs`           | `set` (allexport, physical, ignoreeof), `shopt` (update_shellopts call, readline options removed)                                                                                                                                                                                   |
| `src/builtins/trap.rs`          | `trap`, `kill` (kill -l range check), `enable` (full -n/-s/-a/-d impl)                                                                                                                                                                                                              |
| `src/interpreter/mod.rs`        | Shell struct, `declared_unset`, `disabled_builtins`, `source_set_params`, `run_string`, `resolve_nameref`, `set_var` (auto-export), SHELLOPTS/BASHOPTS readonly, BASH_ALIASES/BASH_CMDS init                                                                                        |
| `src/interpreter/commands.rs`   | Command execution, `expand_word*`, `set -k` keyword assignment scoping (save/restore), inline compound assignment detection (SingleQuoted `(` support), `execute_assignment`, `expand_assoc_subscript` (quote-aware subscript expansion)                                            |
| `src/interpreter/arithmetic.rs` | Arithmetic eval, `expand_comsubs_in_arith` (handles `\$` and backticks), error tokens, short-circuit assignment validation, ternary precedence, bracket depth tracking in operator scanning, `arith_array_get` (recursive non-numeric value eval), bare array name → [0] resolution |
| `src/interpreter/redirects.rs`  | Redirections (vredir `{var}` fds with nameref support, varredir_close, fd validation, memfd heredocs, pipe fd leak fix)                                                                                                                                                             |
| `src/interpreter/pipeline.rs`   | Pipeline execution, PIPESTATUS, `in_pipeline_child` always true for forked children, SIGPIPE reset to SIG_DFL in pipeline/comsub children                                                                                                                                           |
| `src/expand/mod.rs`             | Word expansion, `ExpCtx`, `ifs_first_char()` helper (empty IFS handling), procsub handling, `SyntaxError` handler, `NOUNSET_ERROR` flag, `NOCASEMATCH_ENABLED` flag, empty-element removal in unquoted `${arr[@]%%pattern}`, indirect `[@]`/`[*]` array splitting in dquote context   |
| `src/expand/params.rs`          | Parameter expansion (`${...}` operators), IFS-aware `${arr[*]}` joining, `parse_arith_offset` (with `${}`/`$()` pre-expansion), `is_valid_var_ref`, negative subscript bounds checking, assoc subscript expansion, `@K`/`@k` key-value transform, indirect `!name[@]` with operators  |
| `src/expand/pattern.rs`         | Pattern matching, `pattern_replace` (handles empty value + `*` match, `&` matched-text replacement via `patsub_replacement` shopt, zero-length extglob match handling), `nocasematch` case-insensitive comparison via `chars_eq`/`char_in_range`, PUA-aware `char_in_class` for POSIX character classes |
| `src/expand/transform_helpers.rs` | Shared `shell_quote` and `expand_backslash_escapes` helpers for `@Q`/`@E` transforms (used by both `apply_param_op` and `expand_param`)                                                                                                                                           |
| `src/lexer/mod.rs`              | Lexer, `lex_compound_array_content()` (full-quoting re-parser for `declare -a`), thread-locals (`DQUOTE_TOGGLED`), `force_read_pending_heredocs`, `heredoc_resume`                                                                                                                  |
| `src/lexer/dollar.rs`           | `${}` parsing, `parse_brace_param` (bad substitution for `${$(...)}`, `${!name[@]}` vs `${!name[@]@Q}` indirect dispatch), `$(...)` comsub parser (now handles `<<<` here-strings)                                                                                                  |
| `src/lexer/word.rs`             | `read_param_word_impl`, `skip_comsub` (case state machine), `take_heredoc_body`                                                                                                                                                                                                     |
| `src/lexer/heredoc.rs`          | `register_heredoc` (line count fix), `read_heredoc_bodies` (backslash-newline, `<<-` tab-stripped delimiter matching), `parse_double_quoted_content` (backslash fix for `\"`)                                                                                                       |
| `src/expand/arithmetic.rs`      | `eval_arith_full_with_assoc` (receives real arrays/assoc_arrays/namerefs), `resolve_arith_vars` (handles `${var:-default}`, array subscript lookups)                                                                                                                                |
| `src/parser.rs`                 | Parser, `parse_array_elements` (returns Result), `skip_to_next_command`, heredoc body resolution (full recursive `resolve_heredoc_in_command`), `set_line_number` (absolute line set for comsub)                                                                                    |
| `rust/bash/testsuite.nix`       | Test harness with path/PID normalization                                                                                                                                                                                                                                            |

## Recommended Next Priorities

### Low-hanging fruit (could flip nix tests to passing)

1. **Fix SIGPIPE flaky tests (comsub/lastpipe/trap)** — 1-line diff each, timing race in nix sandbox. SIGPIPE is reset to SIG_DFL in pipeline/comsub children and EPIPE is suppressed in echo builtin for all subprocess contexts, but the nix sandbox timing still occasionally triggers the race. trap has an extra CHLD signal delivery. printf also has a flaky SIGPIPE race (printf6.sub line 40).

2. **Verify nix-only failures (arith/array/nameref/varenv)** — Pass locally but fail in nix sandbox. **varenv** and **nameref** should now pass in nix with Phase 29 harness fixes (BASHPID/PPID/`$_` normalization). Remaining: arith10.sub error format + `let` empty subscript in assoc_expand_once, array32/33.sub bash 5.3 differences. **assoc** tilde expansion fixed in Phase 26 but still fails in nix due to bash 5.3-specific bracket parsing and `$'\001'` key handling. Phase 30/31 transform + nocasematch fixes may also improve nix results for new-exp.

### Medium effort

3. ~~**Fix `string_to_raw_bytes` UTF-8 encoding**~~ ✅ **Fixed in Phase 31.** PUA-based raw byte tracking (U+E000–U+E0FF) distinguishes escape-derived bytes from source-code Unicode.

4. **Fix `local b=("${!1}")` compound assignment** — `local`/`declare` with inline compound array assignment `name=(...)` expands the word as a whole before detecting the `(...)` pattern, so `"${!1}"` (which produces `SplitHere` markers for array elements) gets flattened into a single string. Bash handles this at the parser level. Fix: detect compound array assignment earlier and expand the inner elements with proper word splitting. (~4 diff lines in new-exp12)

5. **Fix `'}'` quoting in dquote `${}` default values** — `"${HOME-'}'}"` should protect `}` from closing the brace expression. Complex interaction with `$()` parsing inside `'...'`. Attempted in Phase 23, 28, and 29 but reverted due to regressions in posixexp/posixexp2. The core difficulties: (a) bash's lexer finds the `}` endpoint first (using squote-depth tracking to protect `}` inside `'...'`), then parses the default value content — our lexer parses inline, so `$(` inside `'...'` starts comsub processing that overshoots the `}` boundary; (b) in POSIX mode (`set -o posix`), `'` does NOT protect `}` — but POSIX mode is a runtime setting, while our lexer pre-parses the entire file. A two-pass scan approach (first find `}`, then parse within boundary) was implemented in Phase 29 and works for non-POSIX mode, but breaks when `set -o posix` is set at runtime because the parse-time decision doesn't match the runtime state. Needs lazy parsing or runtime-mode-aware lexing. (~2 diff lines in new-exp nix)

6. **Fix remaining quotearray nix diffs** — ~~Single-quoted `(( 'assoc[$key]++' ))` expansion.~~ ✅ Fixed in Phase 28. ~~`printf -v array[@]` for indexed arrays.~~ ✅ Fixed in Phase 28. Remaining nix diffs are in quotearray3/4/5.sub: `unset 'a[$key]'` where `$key='$(echo foo)'` — word splitting produces two tokens `a[$(echo` and `foo)]`, our shell only errors on the second; `test -v assoc[$key]`/`[[ -v assoc[$key] ]]` returning 0 vs 1; `assoc_expand_once` interactions with `unset`; `declare -a array` vs `declare: array: not found` version differences. (~40 nix diff lines)

7. ~~**Fix `&` replacement quoting edge cases**~~ ✅ **Fixed in Phase 29.** ~~**Fix `${!var@Q}` indirect + transform**~~ ✅ **Fixed in Phase 30.** ~~**Fix `${var@C}`/`${var@}` bad substitution**~~ ✅ **Fixed in Phase 30.** ~~**Fix `${arr[@]@Q}` per-element transform**~~ ✅ **Fixed in Phase 30.** ~~**Fix `${@@A}`/`${arr[@]@A}` declaration format**~~ ✅ **Fixed in Phase 30.** ~~**Fix `${var@K}` unimplemented**~~ ✅ **Fixed in Phase 31.** ~~**Fix `"${!xx}"` indirect array splitting**~~ ✅ **Fixed in Phase 31.** ~~**Fix nocasematch in pattern matching**~~ ✅ **Fixed in Phase 31.** ~~**Fix `${!name[@]@Q}` lexer parsing**~~ ✅ **Fixed in Phase 31.** ~~**Fix UTF-8 encoding (`string_to_raw_bytes`)**~~ ✅ **Fixed in Phase 31.** Remaining new-exp diffs: `local b=("${!1}")` compound assignment (new-exp12, 4 lines).

8. **Fix comsub2 funsub LINENO** — 8 lines diff. Root cause deeply investigated in Phase 25: bash's `parse_and_execute` counts lines via `shell_getc` line-buffer refills, not per-`\n`. Compound commands (`for`/`while`) inside comsubs get extra line increments from this mechanism. Our character-level lexer fundamentally differs. Needs architectural approach (possibly emulating bash's line-buffered counting in a comsub-specific lexer mode).

### Feature work

9. **Implement `caller` builtin and fix DEBUG trap context** — Needed for dbg-support tests (local-only). (~375+15 diff lines)

10. **Implement restricted shell mode (`-r` flag)** — Needed for rsh tests (local-only). (~26 diff lines)

11. **Performance: optimize hot loops** — `ifs-posix` takes ~4 minutes vs bash's ~1s. `arith` takes ~2s vs bash's 0.035s. Profiling needed.

12. **Fix `help` builtin output + `ulimit` flags** — Needed for builtins test in nix. `help` needs column-formatted output matching bash's layout. `ulimit -g` (max locked memory) not implemented. (~130 nix diff lines)

## Recent Fixes (Phase 31)

- **Implement `shopt -s nocasematch`** — Add `NOCASEMATCH_ENABLED` thread-local flag with `set_nocasematch()`/`get_nocasematch()` API. Propagate `shell.shopt_nocasematch` to pattern matching in `expand_word_fields`, `expand_word_single`, `run_case`, and `run_conditional`. In `pattern_match_impl` (both `pattern.rs` and `commands.rs`), add `chars_eq(a, b, nocase)` and `char_in_range(ch, lo, hi, nocase)` helpers for case-insensitive literal comparison, bracket expression matching, and range matching. For `[[ =~ ]]` regex, wrap pattern with `(?i)` when nocasematch is on. Disable literal fast path in `is_literal_pattern()` when nocasematch is active. Fixes new-exp8.sub (16 diff lines eliminated).
- **Fix `"${!ref}"` indirect array expansion splitting** — In dquoted `WordPart::Param(expr)` with `ParamOp::Indirect`, when the resolved target ends with `[@]`, produce `SplitHere` markers between elements (like `"$@"` does). When target ends with `[*]`, join with IFS. Fixes new-exp9.sub (`<1 2 3 4 5>` → `<1> <2> <3> <4> <5>`) and partially fixes new-exp12.sub.
- **Fix `${arr[@]:offset:length}` arithmetic offset parsing** — Replace all `offset_str.trim().parse().unwrap_or(0)` calls in `get_array_elements` with `parse_arith_offset()` to support expressions like `${#x[@]}-1`. Add `cmd_sub: CmdSubFn` parameter to `get_array_elements`. Add `${...}` and `$(...)` pre-expansion in `parse_arith_offset` using `parse_word_string` + `expand_word_nosplit_ctx`. Fixes new-exp5.sub (`0 1 2 3 4 5 6 7 8 9` → `9`).
- **Fix `${!name[@]@Q}` / `${!name[@]%b}` lexer parsing** — `${!name[@]}` (followed by `}`) is array indices, but `${!name[@]@Q}` or `${!name[@]%b}` (followed by operator) is now correctly treated as indirect expansion. Lexer now checks for `}` after `[@]`/`[*]` before deciding: bare `}` → `ArrayIndices`, operator → indirect expansion with `!name[@]` prefix. Fixes new-exp13.sub `${!varname[@]@Q}`, `${!VAR4[@]@Q}`, `${!varname[@]%b}`.
- **Fix `${!target@Q}` invalid variable name error** — When indirect expansion resolves to a multi-word string (e.g. `aaa bbb` from `VAR4[@]`), emit `invalid variable name` error and call `set_arith_error()` to abort the command (matching bash's behavior of suppressing the echo). Fixes new-exp13.sub line 56.
- **Fix `${VAR[@]@A}` for declared-but-unset scalars** — When the `[@]@A` path finds no array/assoc_array, check if the base variable is a declared-but-unset scalar with attributes and produce `declare -FLAGS name` instead of falling through to the per-element code (which would produce empty). Fixes new-exp13.sub `${VAR1[@]@A}`.
- **Fix `${arr[@]@A}` for declared-but-unset vs empty arrays** — Only omit `=()` when `__UNSET__` marker is set (truly declared-but-unset). Explicitly empty arrays (`B=()`) correctly show `declare -a B=()`. Fixes `declare -a B` vs `declare -a B=()` diff.
- **Implement `${var@K}` / `${var@k}` key-value transform** — For indexed arrays: `0 "val0" 1 "val1"` (K, double-quoted values) or `0 val0 1 val1` (k, unquoted). For assoc arrays: `key "val"` pairs. For scalars/positional params: single-quoted values (same as `@Q`). `"${arr[@]@k}"` in double quotes produces `SplitHere`-separated key/value words. Added exclusion in `is_array_at_expansion` for `Transform('K')`. Fixes new-exp14.sub (8 diff lines eliminated).
- **Fix `string_to_raw_bytes` UTF-8 encoding via PUA raw byte tracking** — Introduce `RAW_BYTE_BASE` (U+E000) and `raw_byte_char(byte)` in `builtins/mod.rs`. Escape sequence handlers (`$'\xNN'`, `\NNN` octal) in `lexer/dollar.rs` and `lexer/word.rs` now produce PUA-encoded characters instead of raw codepoints. `string_to_raw_bytes` detects PUA chars and outputs single bytes; all other Unicode chars output as proper UTF-8. Update `interpret_echo_escapes` (`echo -e`, `printf %b`) to use `raw_byte_char`. Update `shell_quote` in `transform_helpers.rs`, `shell_escape` (printf %q) in `builtins/mod.rs`, `quote_for_declare` (declare -p), and `quote_assoc_key` to decode PUA chars to their original byte values when formatting. Add `char_in_class` helper in `pattern.rs` (re-exported as `pattern_char_in_class`) that decodes PUA chars before checking POSIX character classes (fixes `[[:cntrl:]]` matching `$'\003'`, `[[:graph:]]` not matching `$'\033'`). Update `printf %c` in `io.rs` to handle PUA chars. Fixes new-exp10.sub (`$'\001\001'` quoting) and new-exp11.sub (16 diff lines from UTF-8 multibyte char encoding). Also fixes posixpat regression (3 character class tests).

## Recent Fixes (Phase 30)

- **Fix `${!var@Q}` indirect expansion combined with transform operators** — Lexer now detects `@X` transform (E/Q/P/A/a/K/k/L/U/u) after indirect variable name before the name prefix check at line ~755 in `dollar.rs`. Previously `${!var@Q}` was misinterpreted as `${!prefix@}` (name prefix matching) because `@` triggered the prefix path, then `Q}` after `@` caused "bad substitution". The fix creates `ParamExpr { name: "!var", op: Transform('Q') }` which the existing indirect handler in `expand_param` (line 951) resolves correctly.

- **Fix `${array[@]@Q}` per-element transform** — `apply_param_op` had no `Transform(ch)` arm — it fell through to `_ => val.to_string()`, returning values unchanged. Added full Transform handling (Q/E/U/L/u) to `apply_param_op`. Extracted `shell_quote()` and `expand_backslash_escapes()` into new `src/expand/transform_helpers.rs` module to share between `apply_param_op` (per-element) and `expand_param` (scalar).

- **Fix `${!arr[@]@Q}` — ArrayIndices + Transform parsing** — Extended `ArrayIndices(char)` to `ArrayIndices(char, Option<char>)` in `ast.rs` to carry an optional transform character. Lexer now checks for `@X}` after `[@]`/`[*]` in the indirect expansion path. Expand phase applies the transform per-element to index/key strings.

- **Fix `${var@A}` for unset/declared-but-unset variables** — Declared-but-unset variables (e.g. `declare -lr VAR1`) now emit `declare -rl VAR1` without `=''` suffix, using `__UNSET__` markers injected by `inject_transform_attrs`. Plain variables with no attributes use `name='value'` format (no `declare --` prefix). `${VAR[@]@A}` and `${VAR[@]@a}` now work for array-subscripted forms — `inject_transform_attrs` strips `[@]`/`[*]` subscripts for attribute lookup.

- **Fix `${@@A}`, `${arr[@]@A}`, `${arr[@]@a}` declaration format** — Positional params produce `set -- 'val1' 'val2' ...`. Indexed arrays produce `declare -a name=([0]="val1" [1]="val2" ...)`. Assoc arrays produce `declare -A name=([key]="val" ... )` (trailing space before `)` matches bash). Empty arrays produce `declare -a name=()`. `@a` returns attribute string repeated per-element. Excluded `Transform('A')`/`Transform('a')` from per-element expansion paths in `expand_part` (both double-quoted and unquoted) and `is_array_at_expansion` so they fall through to `expand_param`.

- **Fix transform operators for truly unset variables** — `${unset@Q}` now returns empty string (not `''`). `${unset@A}` returns empty. Distinguishes truly unset (never declared) from declared-but-unset (has `__UNSET__` marker) and set-to-empty. Special variables, positional params, and environment variables are exempt from the unset check.

- **Fix `${var@C}`/`${var@}` bad substitution** — `@C` is not supported in bash 5.3; now returns bad substitution error. `${var@}` (bare `@` with no transform letter) also returns bad substitution. Any unrecognized transform letter after `@` produces bad substitution.

- **Fix `pattern_replace` for zero-length extglob matches** — Patterns like `?(b)` and `*(b)` that can match empty strings now correctly find the empty match at position 0 for replace-first (`${x/?(b)/z}` → `zabcd`). For replace-all (`${x//?(b)/z}`), empty matches are found at every position with proper advancement past the current character to avoid infinite loops. The `min_match_len.max(1)` lower bound that prevented empty matches was replaced with a `can_match_empty` check.

- **Remove `set -o emacs`/`set -o vi`** — These options are not available in bash without readline support. Now report "invalid option name" matching bash 5.3 behavior. Removed from `set -o` option listing as well.

## Recent Fixes (Phase 29)

- **Fix `&` replacement quoting in `${var/pat/rep}` pattern substitution** — New `expand_replacement_word` function preserves quoting context for `&` in replacement strings using `\x00` markers. When `patsub_replacement` is enabled, unquoted `&` means "matched text" while quoted `&` (inside `"..."` or `'...'`) is literal `&`. `\x00` markers are inserted for `&` and `\` in `Quoted` segments during expansion, then processed by `apply_replacement_amp` / `process_replacement_amp` / `unescape_replacement_amp` in both `pattern.rs` and `params.rs`. This fixes: `\&` → literal `&`; `\\&` → `\` + matched text; `"& "` → literal `&` (not matched text); `"\& "` → literal `\&`; variable expansion `$rep` where `rep='\\&'` → `\` + matched text. `new-exp16.sub` goes from ~36 → 0 diff lines.

- **Fix tilde expansion in replacement strings** — `~` at the start of an unquoted replacement word in `${var/pat/rep}` is expanded to `$HOME`, matching bash behavior. Only expands when the `~` is in the first `WordPart::Literal` or `WordPart::Tilde` of the replacement word (not when inside `"~"` double quotes). Both `expand_replacement_word` (patsub on) and the fallback path (patsub off) apply tilde expansion.

- **Fix `$_` initialization at startup** — `$_` is now set to the shell's own absolute path at startup using `std::env::current_exe()` (falling back to `/proc/self/exe` then `argv[0]`). The value is also inserted into `exports`. This matches bash's behavior of overriding the inherited `$_` from the environment. Previously, `$_` was inherited from the parent process (e.g. `/nix/store/.../timeout`), causing diffs in varenv and nameref tests.

- **Update nix test harness PID normalization** — Added `sed` patterns to normalize `BASHPID="<pid>"`, `PPID="<pid>"`, `_PID="<pid>"` in `declare` output, and `$_` nix store paths (`_="/nix/store/..."` → `_="NIXPATH"`). This eliminates false PID-based failures in varenv, nameref, and builtins tests.

- **Investigated `'}'` quoting in dquote `${}` default values** — Implemented a two-pass approach: `find_closing_brace_squote_aware` scans forward to find the real `}` respecting `'...'` pairs, then `read_param_word_impl` uses this boundary. Works correctly for non-POSIX mode (`"${HOME-'}'}"` → `'}'`). However, POSIX mode changes behavior at runtime (`set -o posix` makes `'` NOT protect `}`), and our pre-parsing lexer can't detect this. Caused regressions in posixexp/posixexp2 (264/52 diff lines). Reverted. The fix needs either lazy parsing or a way to defer the squote-boundary decision to runtime.

## Recent Fixes (Phase 28)

- **Fix single-quoted `(( ))` arithmetic expressions** — In `(( ))` arithmetic context, `'` is a literal character (not quoting), matching bash behavior. In `expand_comsubs_in_arith`, when the identifier before `[` is preceded by `'`, bracket protection is NOT activated — this allows `$var` expansion to proceed. The expanded value has `]`, `[`, `$` backslash-escaped to protect bracket matching and match bash's error message format. Added `squote_bracket_escape` flag that tracks this state: when true, `$var` and `${var}` expansions inside `[...]` are performed (instead of being skipped for later `arith_subscript_key` handling) and the results are escaped. When the `]` closes the bracket, the flag is cleared. The `arith_top_expr` update for expressions containing `'` now preserves `\$` escaping (instead of stripping it) so error messages show the backslash-escaped form.

- **Add `'` to arithmetic "operand expected" check** — Expressions starting with `'` (single quote) in `(( ))` now produce `((: 'expr' : arithmetic syntax error: operand expected (error token is "'expr' ")` matching bash's format. Previously fell through to the "invalid arithmetic operator" or array element parsing paths which produced different error formats without the `((:` prefix.

- **Fix `printf -v array[@]` and `printf -v array[*]` for indexed arrays** — For indexed arrays, `@` and `*` are not valid assignment subscripts. `printf -v array[@] "%s" val` now reports `array[@]: bad array subscript` (matching bash) and returns without modifying the array. Previously, `@` was evaluated as an arithmetic expression which errored with "invalid arithmetic operator" and aborted execution. The array is preserved unchanged after the error.

- **Phase 28 improved quotearray** from ~8→0 diff lines locally. The single-quoted `(( 'assoc[$key]++' ))` fix eliminates 4 diff lines (2 error format diffs × 2 test cases). The `printf -v array[@]` fix eliminates the arithmetic error + missing `declare -p` output in quotearray4.sub.

## Recent Fixes (Phase 27)

- **Add "invalid arithmetic operator" detection** — Characters like `]`, `@`, `{`, `}`, `.`, `;`, `\`, `'` that follow a valid identifier or number in arithmetic expressions are now detected as "invalid arithmetic operator" instead of falling through to generic "operand expected" error. The check runs early (before comma/assignment/binary operator scanning) to prevent expressions like `x],b` from being split at the comma. Error format now matches bash: `expr: arithmetic syntax error: invalid arithmetic operator (error token is "rest_of_expr")`. Fixes quotearray lines 140-153 (6 diff lines eliminated).

- **Fix `expand_comsubs_in_arith` to skip `$var` in ALL array subscripts** — Previously, `$var` expansion inside `[...]` was only skipped for associative arrays. Now ALL array subscripts (indexed and associative) preserve raw `$var` text during `expand_comsubs_in_arith`. The subscript content is expanded later by `arith_subscript_key` → `eval_arith_expr_impl`, where the expanded value is a flat expression without surrounding brackets. This prevents expanded values containing `]` from breaking bracket matching (e.g. `a[$key]` where `$key='x],b[$(echo uname >&2)'`). Also prevents spurious command execution: `$(echo uname >&2)` in the variable value is no longer executed during subscript evaluation. Fixes quotearray lines 150-153 (4 diff lines eliminated: "uname" output + wrong exit status).

- **Fix depth-aware bracket matching in array element detection** — Replaced `expr.rfind(']')` with forward-scanning depth-aware `[`/`]` matching to find the correct closing bracket. `rfind` would match the LAST `]` in the expression, which could be from an expanded variable value rather than the matching bracket for the first `[`. The new approach tracks depth and finds the first `]` that closes the opening `[`.

- **Fix subscript error reporting** — Temporarily save and clear `arith_top_expr` during `arith_subscript_key` evaluation, then restore it. This ensures that subscript arithmetic errors report the subscript content as the expression (e.g. `x],b[$(echo uname >&2)`) instead of the full outer expression (e.g. `a[$key]`), matching bash's error format.

- **Fix `declare -p` `@` key quoting** — `quote_assoc_key` now includes `@` in the set of characters that require quoting. Previously `@` was listed as "safe punctuation" but bash quotes it because `@` has special meaning (e.g. `$@`, `${arr[@]}`). `declare -A A=([@]="at")` now correctly outputs `declare -A A=(["@"]="at")`. Affects nix quotearray test (~4 diff lines eliminated).

- **Fix `declare -p` non-printable key formatting** — Keys containing tab, newline, carriage return, or control characters now use `$'...'` ANSI-C quoting in `declare -p` output instead of embedding literal non-printable characters inside `"..."`. Tab → `$'\t'`, newline → `$'\n'`, carriage return → `$'\r'`, other control chars → `$'\NNN'` (octal). Matches bash's `declare -p` output format. Affects nix quotearray test (~4 diff lines eliminated).

- **Fix `declared_unset` tracking for compound array assignments** — `arr=()` and `assoc=()` compound assignments now correctly remove the array name from `declared_unset`, so `declare -p` shows `declare -a arr=()` (assigned empty) instead of `declare -a arr` (declared but unset). Previously, only element-level assignments (`arr[0]=x`, `assoc[key]=val`) cleared `declared_unset`; compound assignments (`arr=(...)`) did not. Added `self.declared_unset.remove(&resolved)` at both the indexed and associative compound assignment paths in `execute_assignment`. Affects nix quotearray and assoc tests.

- **Fix `unset assoc[@]` and `unset assoc[*]` for associative arrays** — For associative arrays, `unset "assoc[@]"` and `unset "assoc[*]"` now remove the literal key `@` or `*` instead of clearing all elements. Bash treats `@` and `*` as literal keys in associative array context (they are only special for indexed arrays). For indexed arrays, `unset arr[@]`/`arr[*]` continues to clear all elements as before. Affects nix quotearray test (~6 diff lines eliminated from `declare -p` output differences).

- **Fix `printf -v` with array subscript syntax** — `printf -v "A[@]" "%s" "X"` now correctly assigns to the key `@` in associative array `A` (or element index in indexed arrays). Previously, `printf -v` rejected any variable name containing `[` as "not a valid identifier". The validation now accepts `name[subscript]` syntax, and the output assignment dispatches to associative key insertion or indexed element assignment as appropriate. Made `expand_assoc_subscript` `pub(crate)` for use from the printf builtin. Affects nix quotearray test (~4 diff lines eliminated).

## Recent Fixes (Phase 26)

- **Fix `assoc[$var]+=1` append assignment parsing** — When an associative array subscript is followed by `]+=` (append assignment), the parser's `try_parse_assignment` multi-part branch only searched for `]=` but not `]+=`. For `assoc[$var]+=1` where `$var` expands to `']`, the expanded token `assoc[']]+=1` was treated as a command (not found) instead of an assignment. Fixed by adding `]+=` detection before `]=` in the literal-scanning loop, correctly extracting the append flag and value. quotearray test reduced from ~32→~27 diff lines locally.

- **Add tilde expansion in associative array subscript keys** — `aa[~/path]=val` now correctly expands `~` to `$HOME` in the subscript key during assignment. Previously, `expand_assoc_subscript()` handled `$var`, `'...'`, `"..."`, backticks, and `\\` but not tilde expansion. Added tilde prefix detection at the start of the subscript: `~` alone or `~/` expands to `$HOME`, `~+` to `$PWD`, `~-` to `$OLDPWD`, and `~user` to the user's home directory via `getpwnam`. Affects nix assoc test (tilde in subscript keys).

- **Add tilde expansion in compound array element values and keys** — `declare -A aa=([~/key]=~/Desktop)` now expands tildes in both keys and values. The parser's `extract_array_index()` was putting the raw value string (e.g. `~/Desktop`) into a `WordPart::Literal`, bypassing tilde recognition. Added `literal_to_parts_with_tilde()` helper that converts a leading `~[user]/` into `WordPart::Tilde(user)` + `WordPart::Literal(rest)`. Applied to all value and key extraction paths in `extract_array_index()` and `find_bracket_close_in_parts()`. Affects nix assoc test.

- **Fix `${!prefix* }` bad substitution detection** — `${!_Q* }` (with trailing space before `}`) was incorrectly accepted, producing output instead of erroring. After consuming `*` or `@` in the `${!prefix*}` parser, the code returned `NamePrefix` even when the next char was not `}`. Now checks that `}` immediately follows `*`/`@`; any other content triggers a `SyntaxError` with "bad substitution" message.

- **Fix `${!1*}` and `${!@*}` bad substitution** — Prefixes starting with digits or special characters (not valid variable name prefixes) were accepted by `${!prefix*}` expansion. Added validation that the prefix starts with a letter or underscore before returning `NamePrefix`. Invalid prefixes now produce `SyntaxError("bad substitution")`. Matches bash 5.3 behavior.

## Recent Fixes (Phase 25)

- **Fix `${A[${a[i]}]}` nested subscript expansion** — When an associative array subscript contains a `${}` expansion like `${a[i]}`, the inner `}` was matched by a naive `rest.find('}')` instead of tracking brace depth. Fixed by implementing brace-depth-aware scanning in the subscript key expansion code in `src/expand/params.rs`: scan through the `${...}` content tracking nested `{`/`}` pairs to find the correct closing brace, then recursively call `lookup_var()` on the extracted variable name (which may itself contain array subscripts like `a[i]`). This enables `${A[${a[i]}]}` where `a` is an indexed array and `A` is an associative array. quotearray test reduced from ~36→~32 diff lines locally.

- **Fix `declared_unset` tracking for array element assignments** — `declare -p` for an empty associative array that previously had elements showed `declare -A a` instead of `declare -A a=()`. Root cause: when assigning to array elements (`a[x]=1`), the `declared_unset` set was not cleared for the array name. Added `self.declared_unset.remove(&resolved)` at all assoc and indexed array element assignment paths in `execute_assignment` (both append `+=` and regular `=`, subscripted and scalar-to-array). Now `declare -p` correctly distinguishes never-assigned arrays (`declare -A a`) from emptied arrays (`declare -A a=()`), matching bash behavior. Affects nix assoc and quotearray tests.

- **Deep investigation of comsub/funsub LINENO counting** — Traced through bash 5.3 source: `builtins/evalstring.c` (`parse_and_execute`), `parse.y` (`shell_getc`, `yy_string_get`, `simplecmd_lineno`), `execute_cmd.c` (`SET_LINE_NUMBER`), `make_cmd.c` (`make_simple_command`). Key discovery: bash's `shell_getc` reads input line-by-line into `shell_input_line` (stripping `\n`, then adding it back). `line_number++` only happens when `shell_getc` refills the line buffer (not per-`\n` character). This means `\n` after `;` doesn't increment `line_number` in string-eval contexts because `;` terminates the command within the current line buffer, and the `\n` is consumed as the `simple_list_terminator` before the next buffer refill. Compound commands (`for`/`while`) cause additional line increments because their body parsing triggers additional buffer refills. Our character-level lexer fundamentally differs — it increments on every `\n` in `advance()`. A simple `comsub_parsing` flag that suppresses `\n` increments after `;` was prototyped but reverted because it doesn't account for compound-command line increments and made funsub results worse (73 instead of 74, vs bash's 75). The proper fix would require emulating bash's line-buffered counting model in a comsub-specific lexer mode.

## Recent Fixes (Phase 24)

- **Implement `&` in replacement strings (`patsub_replacement`)** — When `shopt -s patsub_replacement` is enabled (default in bash 5.3), unescaped `&` in `${var//pat/rep}` replacement strings is substituted with the matched text (like sed's `&`). `\&` produces a literal `&`. Added `apply_replacement_amp()` helper in `src/expand/pattern.rs` that processes the replacement per-match. All code paths in `pattern_replace` (literal fast path, single-char fast path, `*` match, general variable-length match) now support `&` substitution. Also added `process_replacement_amp()` in `src/expand/params.rs` for `ReplacePrefix` (`${var/#pat/rep}`) and `ReplaceSuffix` (`${var/%pat/rep}`) which don't go through `pattern_replace`. Added `PATSUB_REPLACEMENT` thread-local flag synced from `shopt_options` in `expand_word_fields` and `expand_word_single`. new-exp test locally has ~16 diff lines (PID + `&` quoting edge cases remaining).

- **Investigated comsub2 funsub LINENO off-by-1** — Traced through bash 5.3 source (`builtins/evalstring.c`, `parse.y`, `execute_cmd.c`). Root cause: bash's `parse_and_execute()` does `line_number--` before parsing comsub/funsub content, compensating for an implicit `line_number++` when the parser reads its first input line. Our character-level lexer counts `\n` per-character in `advance()` rather than per-line-read, so the `--/++` dance doesn't translate directly. `set_line_number(LINENO-1)` fixes compound-command body lines but breaks single-line funsubs. Left as known issue with clear documentation of root cause for future fix.

## Recent Fixes (Phase 23)

- **Fix `\x00`-quoted literal patterns in `pattern_replace`** — The min/max match length computation in `pattern_replace` now handles `\x00` as a quoting prefix (like `\\`), counting it as 1 matched character and skipping 2 pattern chars. Previously `\x00` fell through to the default case, counting it as a regular character — so `\x00?` (literal `?`) was computed as fixed length 2 instead of 1, causing the match loop to never find a 2-char match. Also added `\x00X` as a recognized single-char pattern in `is_single_char_pattern()` for the O(n) fast path. Fixes `${a//\?/X}` in unquoted context (script mode).
- **Fix `"${@}"` and `"${*}"` with braces** — `${@}` with `ParamOp::None` inside double quotes now produces `SplitHere` markers like `$@`. Previously, `WordPart::Param(ParamExpr { name: "@", op: None })` fell through to the generic Param handler which called `expand_param`, joining all positional params as one string. Added dedicated handlers in the DoubleQuoted expansion path. Similarly `"${*}"` now joins with IFS[0] like `"$*"`.
- **Fix `"$xxx${@}"` with no positional params** — When `$@` or `${@}` expands to nothing AND the accumulated string is empty, the double-quoted word now produces zero fields. Changed `only_at` check (which required ALL parts to be `@`) to `has_at` check (which only requires ANY part to be `@`). Also updated `has_at_expansion` in `expand_word` to recognize `Param` with name `@` and `op: None`.
- **Fix `${!foo}` indirect expansion where `foo=@`** — In double-quoted context, `"${!foo}"` where `foo=@` now correctly produces SplitHere markers (separate fields) like `"$@"`. Similarly `foo=*` joins with IFS[0]. Added `ParamOp::Indirect` check in the DoubleQuoted Param handler that resolves the target variable and dispatches to `@`/`*` field-splitting logic before falling through to the generic handler.
- **Fix empty element removal in unquoted `${@%%pattern}`** — When `apply_param_op` returns an empty string for an element of `$@`, it is now skipped in unquoted context. Previously empty results were pushed as `Segment::Unquoted("")` with `SplitHere` markers, creating spurious empty fields. Fixes `${@%%[!/]*}` where `.` element produces empty.
- **Fix `${var/#/x}` empty prefix replacement** — `${var/#/x}` now correctly prepends `x` to the value (empty pattern matches the empty prefix). The `///` parsing fix was scoped to only ReplaceAll (`//`) and ReplaceFirst (`/`) modes, not prefix (`#`) or suffix (`%`) modes, so the empty pattern is preserved for prefix/suffix replacement.
- **Fix `${var///a/}` parsing** — After `//` (ReplaceAll), if the next character is `/`, it is now included as part of the pattern rather than treated as the pattern/replacement separator. This allows patterns starting with `/` to be specified: `${a///a/}` correctly means "replace all `/a` with empty". Only applies to ReplaceAll and ReplaceFirst modes; prefix (`#`) and suffix (`%`) modes keep the `/` as separator.

## Recent Fixes (Phase 22)

- **Fix command substitution LINENO off-by-one** — Multi-line `$(\ncmd)` now reports correct line numbers matching bash. The root cause was that `set_line_offset()` used `lexer.line += offset` (relative add) after the parser constructor's `next_token()` call had already consumed the leading `\n` (incrementing `lexer.line` from 1 to 2). Added `set_line_number(target)` method that sets `lexer.line` to the absolute target value, discarding whatever the constructor consumed. All comsub execution sites (`capture_output`, `capture_output_nofork`, `capture_valuesub`, procsub runners) now store the actual 1-based LINENO and use `set_line_number()`. Eval continues to use `set_line_offset()` (which works correctly since eval text doesn't start with `\n`). Fixes **heredoc** nix test (heredoc7.sub case 2: `cat <<EOF && grep $(` line numbers now match).
- **Fix `${!prefix*}` IFS separator** — `"${!prefix*}"` now joins matching variable names with the first character of IFS (like `"$*"`) instead of always using space. `"${!prefix@}"` correctly splits into separate words (like `"$@"`). Added `is_array_at_expansion` handling for `NamePrefix('@')` and `get_array_elements` handler to return individual variable names as separate fields.
- **Fix `$(< $var)` variable expansion** — The `$(< filename)` fast path now parses the filename string into proper word parts using `lex_compound_array_content()` so that `$var`, `${var}`, tilde, and other expansions work. Previously wrapped the raw text in `WordPart::Literal` which didn't expand `$`. Fixed in both the unquoted and double-quoted code paths.
- **Fix `$(< file)` glob expansion** — `$(< $TMPDIR/bashtmp.x*)` now performs glob expansion on the filename (unless in posix mode), matching bash behavior. Uses the `glob` crate to resolve single-match patterns.
- **Fix `$(< nonexistent)` error handling** — Now reports the error with proper `strerror`-style message (no `(os error N)` suffix) and sets exit status to 1 via `set_arith_error()`, matching bash behavior. Previously silently returned empty string with exit status 0.
- **Fix `${var:?message}` error prefix** — Now uses `EXPAND_ERROR_PREFIX` (which includes script name and line number) instead of hardcoded `"bash:"`. In `-c` mode with `$0` set to a script name, errors now correctly show `./script: line N: VAR: message` instead of `bash: VAR: message`.

## Recent Fixes (Phase 21)

- **Fix `let` empty subscript handling** — `let "a[\"\"]"=22` now correctly assigns to `a[0]` when `assoc_expand_once` is unset (the default). The `""` inside the subscript is stripped by the arithmetic quote-removal pass, leaving `a[]=22`. In `let` context (not `(( ))`), a pre-stripped expression snapshot detects that the subscript was originally `""` (not truly empty), and treats it as index 0. When `assoc_expand_once` is set, bash still rejects the empty subscript, so the check is conditioned on `!assoc_expand_once`. Fixes first section of arith10.sub `afunc` (assoc_expand_once unset). The second section (assoc_expand_once set) now correctly errors but with different error format than bash (error format is a separate issue).
- **Fix `${var/#pat/rep}` prefix replacement longest match** — `ReplacePrefix` now iterates from longest to shortest match (`(0..=len).rev()`) instead of shortest to longest. This fixes `${x/#*/yyy}` which should replace the entire string with `yyy` (since `*` matches the whole string from the anchor), not just prepend `yyy` (matching empty string at start). Same fix applied to both `apply_param_op` and `expand_param` call sites.
- **Fix `${var/%pat/rep}` suffix replacement longest match** — `ReplaceSuffix` now iterates from position 0 upward (`0..=len`) instead of from the end downward. This ensures `${x/%*/yyy}` replaces the entire string (since `*` starting from position 0 is the longest suffix match), not just appends `yyy` (matching empty string at end). Fixes 24 diff lines in new-exp nix test.
- **Optimize `pattern_replace` for literal patterns** — When the pattern contains no glob metacharacters (`*`, `?`, `[`, `\`, etc.), use `str::replace()` for O(n) performance instead of O(n³) glob matching. Fixes `${z//str}` on 10K-char strings going from >60s timeout to <1s.
- **Optimize `pattern_replace` for single-char patterns** — Patterns that match exactly one character (`?`, `[abc]`, `[^;]`, `[[:alnum:]]`, `[[:alnum:]_]`) now use O(n) per-character matching. Added `is_single_char_pattern()` that recognizes bracket expressions including POSIX character classes `[:name:]`.
- **Optimize `pattern_replace` for fixed-length patterns** — When the pattern has no `*` or extglob variable-length constructs, the match length is fixed (min == max), so only one substring length is checked per position — O(n) instead of O(n²). Added proper `min_match_len` calculation that walks pattern tokens (`[...]` = 1 char, `?` = 1, `*` = 0+, extglob `*(...)` = 0+, `+(...)` = 1+, etc.).
- **Handle extglob in `min_match_len`** — Extglob patterns `*(...)`, `?(...)`, `+(...)`, `@(...)`, `!(...)` are now recognized and correctly computed: `*(...)` and `?(...)` contribute 0 min chars (variable length), `+(...)` contributes 1 min char, `@(...)` and `!(...)` are variable length. Prevents extglob patterns from being miscomputed as fixed-length and missing matches (was causing extglob test regression).
- **Performance: new-exp8.sub completes** — The test creates a 10K-char string and does 16 `${z//pattern}` operations with various patterns. Previously timed out at 300s. Now completes in seconds thanks to the fast paths.

## Recent Fixes (Phase 20)

- **Fix associative array subscript expansion in assignments** — `A[$key]=val` now properly expands `$key` to the variable's value before using it as the assoc array key. Previously stored the literal `$key` as the key. Added `expand_assoc_subscript()` method with full quote handling: single-quoted content is literal, double-quoted and unquoted content expands `$var`, `${var}`, `$(cmd)`, etc. Applied to both regular (`A[$key]=val`) and append (`A[$key]+=val`) assignment paths, plus `BASH_ALIASES[$key]=val`.
- **Preserve quote markers in assignment subscripts** — Parser's `try_parse_assignment` now preserves single-quote and double-quote markers around subscript content in the assignment name (e.g. `A['literal']` keeps the quotes so `expand_assoc_subscript` can detect literal keys). Double-quoted parts also preserve `$var` and other expansions in the name text.
- **Fix `$` expansion in assoc subscript lookup** — When `$` is followed by a non-identifier character (like `(` in `$(echo %)`), the inline expansion in `lookup_var` now leaves the `$` as-is instead of replacing it with empty string. Fixes lookup of keys containing literal `$()` like `$(echo %)`.
- **Fix `declare -p` assoc key quoting** — `quote_assoc_key` now only quotes keys containing shell-special characters (`$`, `!`, `` ` ``, `"`, `\`, `'`, `(`, `)`, `{`, `}`, `<`, `>`, `|`, `&`, `;`, `*`, `?`, `[`, `]`, `~`, `#`, space/tab/newline). Safe punctuation like `%`, `-`, `.`, `/`, `:`, `=`, `@`, `^`, `,`, `+` is left unquoted, matching bash's behavior.
- **Pass real arrays/assoc_arrays to expand arithmetic evaluator** — `eval_arith_full_with_assoc` and `resolve_arith_vars` now receive the actual `arrays`, `assoc_arrays`, and `namerefs` maps instead of empty dummies. This enables `${string:A[%]:A[$k1]}` to correctly look up associative array elements as arithmetic offsets/lengths. All `ExpCtx`-based call sites updated.
- **Add array subscript lookups in `resolve_arith_vars`** — Bare `name[subscript]` patterns in arithmetic expressions (without `$` prefix) are now resolved via `lookup_var` when they're not followed by assignment operators, so `A[%]` in `${string:A[%]:5}` correctly evaluates to the assoc array value.
- **Fix arithmetic bracket depth tracking** — All 9 operator-scanning loops in `eval_arith_expr_inner` now track `[`/`]` bracket depth alongside `(`/`)` parenthesis depth. This prevents operators inside array subscripts from being treated as top-level operators (e.g. `a[1<2]` no longer splits at `<`, `a[7<(4+2)]` now evaluates correctly as `a[0]`=12).
- **Fix `~-N` bitwise NOT with negative operand** — Added `~` to the list of operator characters that prevent `-` from being treated as binary subtraction. `~-2` now correctly evaluates as `~(-2)` = 1 instead of erroring with "operand expected".
- **Fix bare array variable names in arithmetic** — `$((x))` where `x` is an indexed array now correctly resolves to `x[0]` (the first element). Previously returned 0 because `vars.get("x")` didn't find array variables. Applied to both the `${...}` and bare-name variable resolution paths.
- **Fix non-numeric array values in arithmetic** — `arith_array_get` now recursively evaluates non-numeric string values as arithmetic expressions, matching bash behavior where `a[0]="1+2"; echo $((a[0]))` yields 3. Changed from `parse::<i64>().unwrap_or(0)` to try parse then `eval_arith_expr_impl()` fallback.
- **Fix space-only subscripts in arithmetic** — `is_empty_arith_subscript` now only rejects truly empty subscripts (no characters) or quoted empty strings (`""`, `''`). Whitespace-only subscripts like `a[" "]` are no longer rejected — the space evaluates to 0 in arithmetic, so `a[ ]=N` correctly sets `a[0]`. Fixes `(( a[" "]=11 ))` which should succeed.

## Recent Fixes (Phase 19)

- **Fix readonly error prefix for single-quoted compound assignments** — `readonly -a r='(7)'` (quoted, with `-a` flag) now uses `readonly:` as error context instead of the function name. `readonly r='(5)'` (quoted, without `-a`) uses just the variable name. Unquoted compound `readonly -a r=(6)` still uses function name as context. Fixes **attr** nix test (4→0 diff lines).
- **Fix single-quoted values treated as scalar without `-a` flag** — `export r='(5)'` without `-a` flag now correctly stores the literal string `(5)` in `r[0]` instead of parsing `(5)` as a compound array assignment. When `paren_from_single_quote` is true, compound assignment pre-processing in `run_simple_command` only activates if `-a` or `-A` flag is present. Fixes the `declare -ax r=([0]="5")` vs `declare -ax r=([0]="(5)")` nix diff.
- **Fix funsub `$()` terminator detection** — Inside funsub `${ ... }` parsing, `$(...)` command substitutions are now properly skipped as nested constructs. Previously, the `)` from `$()` incorrectly set `has_terminator_at_depth1 = true` (because the `(` and `)` were tracked as subshell parentheses), which allowed `}` to close the funsub without a `;` terminator. Now `$()`, `$((...))`, and `${ ... }` inside funsubs are fully skipped. `${ $() }` correctly fails with "unexpected EOF while looking for matching `}'". Fixes **exportfunc** nix test (2→0 diff lines).
- **Fix `read -t` poll revents bug** — `poll_fd.revents()` was read from the original `PollFd` variable instead of the mutable array element modified by `poll()`, always returning empty flags. Changed to use `poll_fds[0].revents()`. Also added POLLHUP-without-POLLIN check for non-poll timeouts: when the fd has no readable data (only hangup/error), return 142 (timeout) instead of falling through to a read that would immediately return EOF. Fixes **read** nix test (1→0 diff lines).
- **Fix empty arithmetic subscript validation** — `(( a[""]=24 ))`, `a[""]=N` in `(( ))` context, and similar empty-subscript array assignments now correctly error with `` `a[]': not a valid identifier `` instead of silently treating `""` as index 0. Added `is_empty_arith_subscript()` check at all 5 array access paths in the arithmetic evaluator (compound assignment, simple assignment, post-inc/dec, pre-inc, pre-dec). Uses context-aware `arith_cmd_prefix()` for proper `((:`/`let:`/no prefix. Reduced **arith** nix diff from ~16→~10 lines.
- **Newly verified passing tests** — array2, dirstack, input-test, procsub, set-e, test, read all verified passing in nix.

## Recent Fixes (Phase 18)

- **Fix xtrace interleaving in pipelines** — Pipeline children writing xtrace output to stderr could interleave because `writeln!` splits into two `write()` syscalls (message + newline). Changed `xtrace_write` to use a single `write_all()` call with the newline pre-appended, ensuring atomic output. Also flush stderr before fork. Fixes `PS4='+[$LINENO] '; set -x; false | false | false` showing `+[8] false+[8] false` on one line.
- **Disable `set -e` inside funsubs (non-posix mode)** — Bash disables `set -e` (errexit) inside `${ ... }` nofork command substitutions in non-posix mode, matching regular command substitution behavior. In posix mode, `set -e` still propagates. Applied to both `capture_output_nofork` (funsub) and `capture_valuesub` (valuesub). Fixed **comsub22.sub** (`set -e` + funsub + `false` test).
- **Detect bad interpreter shebang error** — When exec fails with ENOENT for a file that exists (bad interpreter in shebang), read the `#!` line and report `script: interp: bad interpreter: No such file or directory` matching bash's error format. Previously reported just `No such file or directory`.
- **Fix `${scalar[@]:offset:length}` substring** — When a scalar variable is accessed with `[@]` subscript and a `:offset:length` operation, perform character-level substring (same as `${var:offset:length}`) instead of returning empty for offset > 0. Fixed in both `expand_param` and `get_array_elements`. Fixed **new-exp** test (18→PID-only diff).
- **Support associative array subscripts in arithmetic evaluation** — Added `ArithSubscript` enum and helper methods (`arith_subscript_key`, `arith_array_get`, `arith_array_set`) so that `(( assoc[key]++ ))`, `(( assoc[$key]=val ))`, `++assoc[key]`, `assoc[key]--` all correctly use string keys for associative arrays while keeping numeric index evaluation for indexed arrays. Applied to all 6 array access patterns in `eval_arith_expr_inner` (compound assignment, simple assignment, post-inc/dec, pre-inc/dec, pre-dec, and value lookup).
- **Skip `$var` expansion inside associative array subscripts in arithmetic** — Modified `expand_comsubs_in_arith` to track whether we're inside `name[...]` where `name` is an associative array. When inside such a subscript, `$var` and `${var}` references are left unexpanded so the arithmetic evaluator's `arith_subscript_key` can handle them properly (expanded values containing `]` would break bracket matching). Indexed array subscripts still expand normally. Reduced **quotearray** from ~200→~68 diff lines.

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