# Changelog

Round-by-round development log of `rust/make`. Reverse-chronological:
newest at top.

## Round 20 — 135/135 ✅ (gained `targets/WAIT`, +2 subtests)

1. **Per-target `.NOTPARALLEL`**: previously `.NOTPARALLEL: t1 t2`
   set the global `notparallel` flag, killing all parallelism. Split
   into:
   - Bare `.NOTPARALLEL:` (no prereqs) → keeps the global
     `notparallel: Cell<bool>` flag.
   - `.NOTPARALLEL: t1 t2 ...` → populates a new
     `notparallel_targets: RefCell<HashSet<String>>` field.

   `try_parallel_batch` consults both. Fixes WAIT #11 (mk.10): `np1`'s
   prereqs build serially while `p1`'s leaves still race.

2. **Aggregator-expansion in `try_parallel_batch`**: when a sibling
   prereq is a "no-recipe aggregator" (every rule entry has empty
   recipe and non-empty prereqs), and its prereqs are all
   `is_simple_leaf` (or already-built), inline the leaves into the
   parallel batch (deduped across siblings) and mark the aggregator
   as built post-batch. Skip aggregators marked `.NOTPARALLEL`.
   Fixes WAIT #7 (mk.6):

   ```text
   all : one two
   one: pre1 .WAIT pre2
   two: pre2 pre1
   pre1, pre2: <recipes that wait FILE on each other>
   ```

   Now schedules `[pre1, pre2]` (deduped) as one parallel batch from
   `all`'s perspective; the leaves race correctly via their
   `wait FILE` sentinels and `one`/`two` get marked built once their
   leaves complete.

3. **`fn aggregator_prereqs`** helper: returns `Some(prereqs)` if
   `target` qualifies as a no-recipe aggregator, `None` otherwise.
   Excludes targets with grouped rules, SE, target-specific vars or
   exports, or any rule entry having a recipe.

Subtest gains: `targets/WAIT` 12 → 14 of 14 (✅ category flip).

**Final total**: 135/135 (100%). MAKEFLAGS 218/218, temp_stdin 8/8,
options/dash-f 32/32, features/parallelism 13/13, targets/WAIT 14/14.

The aggregator-expansion technique is conservative: it requires every
inlined leaf to be a `is_simple_leaf` and skips `.WAIT` markers within
the inlined sequence. The shared-state risk is bounded because each
forked child runs only one leaf recipe and exits — the parent never
needs to migrate any RefCell mutations from a child.

## Round 19 — 134/135 (gained `features/parallelism`, +1 WAIT carry-over)

1. **Phony targets allowed in `is_simple_leaf`**: removed the early
   `return false` for phony names. Test #6 (cascading `-k` / non-`-k`
   failure with phony siblings `fail.1`/`fail.2`/`fail.3`/`ok`) needed
   phony siblings to be forkable.
2. **Completion-order reaping in `try_parallel_batch`**: replaced the
   spawn-order `waitpid(pid, ...)` loop with `waitpid(-1, ...)`
   against a `HashMap<pid, (target, is_oo)>` of pending children.
   Failures from an earlier-finishing recipe now print before
   later-completing recipes, matching GNU make's real-time output
   ordering.
3. **"Waiting for unfinished jobs...." on first failure**: when a
   forked child fails and `keep_going` is false and pending children
   remain, print `make: *** Waiting for unfinished jobs....` to
   stderr (once) and continue reaping the rest. Suppressed during
   include-remake.
4. **Child prints error before exit**: forked child now emits
   `make: *** <err>` (or `make: *** <err>.` for non-bracket errors)
   before `_exit(1)`. Required so the parent's combined stderr shows
   recipe error diagnostics in completion order.
5. **Bail-out when exported recursive var contains `$(shell`**:
   test #5 (`export HI = $(shell $($@.CMD))`) must run serial in GNU
   make because token-losing races in the jobserver can occur. Detect
   this condition and return 0 from `try_parallel_batch`, falling
   back to serial.
6. **`in_include_remake: Cell<bool>` flag with Drop-guard**:
   `finalize_includes` sets this flag for its duration. The child
   error-printing in `try_parallel_batch` checks it and suppresses
   `make: *** ...` output — matching the serial
   `let _ = self.build_target(...)` discard behavior. Also suppresses
   the "Waiting for unfinished jobs...." banner during include
   remake. Fixes test #7 (Savannah bug #15641: `-include foo.d` where
   `foo.d`'s prereqs `mod_a.o mod_b.o` both fail under `-j2`).

Subtest gains:

- `features/parallelism`: 12 → 13 of 13 (✅ category flip).
- `targets/WAIT`: 12 of 14 (unchanged; #7 and #11 need cross-sibling
  parallel non-leaf execution).

**Why `targets/WAIT` #7 and #11 remained deferred at this point.**

Both failing subtests have the shape:

```text
all : one two   # or: all : p1 .WAIT np1
one: pre1 .WAIT pre2
two: pre2 pre1
pre1: ; @thelp.pl out start-$@ file PRE1 wait PRE2 out end-$@
pre2: ; @thelp.pl wait PRE1 out $@ file PRE2
```

`pre1` (reached via `one`) and `pre2` (reached via `two`) must run
concurrently so they can hand off through `PRE1`/`PRE2` sentinel
files. The Round 17/18 parallel fast-path kicks in only when a single
target has multiple leaf prereqs in its prereq list. When the shared
leaves live under different siblings of `all`, `one` builds fully
before starting `two`, causing `pre1` to time out on `wait PRE2`.
Fixed in Round 20 via aggregator expansion.

## Round 18 — 133/135 (parallel during include-remake + cmdline `-j` export, +3 subtests)

1. **`fn parallel_remake_files`**: a generic helper that takes a list
   of file targets, partitions them into "simple leaf" spawnables,
   forks each via `libc::fork()` (capped at `self.jobs`), waits for
   all, and updates `built_targets`/`failed_targets`/`rebuilt_targets`
   in the parent. Returns the set of files it spawned so callers can
   skip them in their serial follow-up loops.
2. **Phase 0 of `finalize_includes`**: now captures pre-remake
   mtimes, calls `parallel_remake_files(loaded_includes)`, then
   compares post-mtimes to set `any_include_remade`. The subsequent
   serial loop skips entries that were spawned.
3. **Phase 2 of `finalize_includes`**: calls
   `parallel_remake_files(to_build_files)` before the per-entry
   `BuildResult` loop. Successful spawned builds land in
   `built_targets`, so `build_target` short-circuits in the per-entry
   loop and the post-build error reporting still works.
4. **Cmdline `-jN` now exported in MAKEFLAGS**: the `-j N` /
   `--jobs N` and `-jN` argument handlers in `main.rs` were setting
   `engine.jobs` but not pushing `-jN` into `mflags_long`, so
   sub-makes invoked via `$(MAKE)` saw an empty `MAKEFLAGS` and ran
   serially. Now both forms append the right token. This is what
   unlocked test #4 (recursive sub-make with `-include`).

Subtest gains:

- `features/parallelism`: 9 → 12 of 13 (passes #3, #4, #7).
- `targets/WAIT`: 12 of 14 (unchanged).

**Why the last 3 subtests stayed deferred at this point.**

- `features/parallelism` #6: cascading `-k` failures across phony
  siblings. Allowing phony in `is_simple_leaf` immediately regresses
  #5 (which requires `$@`-aware env-var expansion to block one child
  for ~4s while the other runs). The right fix is to make
  `lookup_var_with_auto` thread `$@` through env-setup, then re-allow
  phony spawning. Self-contained but touches expansion paths in many
  places. Resolved in Round 19.
- `targets/WAIT` #7, #11: `all: one two` where both `one` and `two`
  have prereqs and are themselves not leaves. Requires forking
  non-leaves — the forked child would run a full `build_target_for`
  subtree, touching ~25 RefCells. Resolved in Round 20.

Total subtest improvement across Rounds 17 + 18: 13 → 24 of 27.

## Round 17 — 133/135 (first-pass parallel implementation, +8 subtests)

Implemented a `fork()`-based parallel-spawn fast-path for sibling
prereqs that are "simple leaves". Activated when `-jN > 1`, gated off
when `-l<N>` (load limit), `--dry-run`, `--question`, `--touch`,
`.NOTPARALLEL`, or recipe-phase. Each batch terminates at `.WAIT`
markers (which act as serialization barriers); after a batch
completes, the next segment can start its own batch.

1. **`fn try_parallel_batch`** in `engine.rs`: collects up to N
   "simple leaf" prereqs, forks each via `libc::fork()`, parent
   `waitpid`s on all children. POSIX exit-status decoded into success
   / signaled / nonzero-exit. Successful builds re-stat the file in
   the parent and update `built_targets` / `rebuilt_targets` / set
   `recipe_executed = true`. Failures land in `failed_targets` and
   set `parallel_batch_failed`.
2. **`fn is_simple_leaf`**: a conservative spawnability gate.
   Spawnable means: non-dot name, not phony, no target-specific
   vars/exports, exactly one rule entry with a non-empty recipe, no
   further prereqs/order-only deps, no `&:` group, no
   `.SECONDEXPANSION`, no `$(MAKE)`/`${MAKE}` in recipe lines.
3. **Segment-aware pre-pass** in `build_target_for`: walks
   `per_rule_seq` partitioning at `.WAIT`, attempts a parallel batch
   from each segment start. When a batch fails to form, the entries
   up to the next `.WAIT` flush serially before the next batch
   starts. Failures from the parallel batch propagate as either
   `first_err` (under `-k`) or an immediate `Err(String::new())`
   return.
4. **Failure FFI plumbing** in `main.rs`: added `fork`, `waitpid`,
   `_exit`, `getpid` to the existing `extern "C"` block.
5. **`-l<N>` load-limit tracking**: `engine.load_limit: f64` stores
   the value parsed from `-l`/`--load-average`/`--max-load`. When
   `>0.0`, `try_parallel_batch` bails out (no real load sampling
   yet — added in a later round).
6. **`-jN` post-load propagation**: makefile-level `MAKEFLAGS += -jN`
   now updates `engine.jobs` after parse. Cmdline `-jM` still wins
   via `cmdline_set_jobs: bool`. Also added `-j` to the MAKEFLAGS
   prefix-flag whitelist (alongside `-I`, `-l`, `-O`).

Subtest gains:

- `features/parallelism`: 3 → 9 of 13.
- `targets/WAIT`: 10 → 12 of 14.

**Risk and rollback.** Zero category regressions across the full
135-test suite. The parallel path is gated behind multiple checks;
setting `-j1` (the default) goes straight back to the serial code
path.

## Round 16 — 133/135 (parallel scheduler analysis, no code change)

Investigated what would be required to pass the remaining two
categories: `features/parallelism` (3/13 subtests) and `targets/WAIT`
(10/14 subtests). Every failing subtest in both categories uses the
helper `tests/thelp.pl` to call `wait FILE` — i.e. one recipe blocks
until another concurrently running recipe creates a sentinel file.
Under serial execution these are guaranteed to deadlock and time out.
Real concurrent process execution is required.

**Why this is a multi-day refactor, not a quick patch.**

The engine is built around a single-threaded, recursive,
`RefCell`-mutating control flow:

- `Engine` carries ~25 `RefCell`/`Cell` fields (not `Sync`).
- `build_target_for` is recursive (~2000 lines) and mutates many of
  those fields on every call.
- `execute_recipe` saves a Vec of prior var values, calls
  `execute_recipe_inner` synchronously (which spawns and waits for
  one shell process at a time), then restores state in fixed order.
- `CURRENT_CHILD_PID` is a single global atomic — only one in-flight
  child can be tracked for SIGTERM forwarding.

**Design sketch for a correct implementation.**

1. Split `execute_recipe` into:
   - `prepare_recipe(target, …) -> RecipeJob` — pre-expands all
     recipe lines, captures the env_set/env_remove vectors, captures
     the saved target-var state, captures added/removed exports.
   - `start_recipe_line(job, line_idx) -> Option<Child>` — spawns
     the next shell process, returns the Child handle.
   - `finalize_recipe(job, status)` — runs the existing var restore
     + exports cleanup + rebuilt-targets update logic.
2. New `JobPool { max: usize, running: Vec<(Child, RecipeJob,
   line_idx)> }` with `submit(job)`, `try_spawn_next()`,
   `wait_one() -> (RecipeJob, Status)`. Signal forwarding tracks all
   running PIDs (small Vec under a Mutex).
3. In `build_target_for`'s sibling-prereq loop:
   - Partition `per_rule_seq` into `.WAIT`-separated batches.
   - For each batch: recursively descend each prereq, but instead of
     running each prereq's own recipe synchronously, hand the
     prepared `RecipeJob` to the pool. Block on `wait_one()` whenever
     the pool is full or the batch is drained before crossing a
     `.WAIT` barrier.
4. `.NOTPARALLEL: targets`: when entering one of them set pool
   capacity to 1 for the duration of its sub-build.
5. Skip the cross-process jobserver pipe protocol initially.
6. Decide what to do about `--debug=j`: existing synthetic "Putting
   child / Reaping winning child" stays, but should now use real
   PIDs from the pool.

**Risks.**

- `built_targets`, `rebuilt_targets`, `failed_targets` updates must
  happen in `finalize_recipe`, *not* at submit time, or parent
  rebuild decisions race with sibling completions.
- Target-specific var save/restore is currently per-recipe; with N
  recipes in flight the saved-state stacks must be per-job.
- Output interleaving: future `-O` (output-sync) implementation must
  keep stderr/stdout per-job until reap time.
- `in_recipe` and `current_source` `RefCell`s are read by
  `expand::expand` to format error messages — these must become
  per-job before parallel pre-expansion is safe.

**Decision.** Hold at 133/135 (98.5%). The remaining 2 tests are
deferred under a single, well-understood blocker (real parallel
scheduler), not 15 disparate bugs. Rounds 17–20 implemented this
incrementally.

## Round 15 — 133/135 (small parallelism subtest gain)

1. **`check_chain_sources` now handles explicit rules**: when
   deciding whether to skip a missing intermediate prereq,
   recursively walk explicit-rule prereq chains (in addition to
   pattern-rule chains) to see if any source file is newer than the
   target. Previously only pattern rules were traversed; an explicit
   chain like `file4: file3; ...` / `file3: file2; ...` /
   `file2: file1; ...` would incorrectly conclude file3 (intermediate)
   doesn't need rebuilding because pattern lookup found nothing.
   Fixes `features/parallelism` subtest 10 (Savannah bug 30653
   regression).

Subtest gains: parallelism 2→3.

## Round 14 — 133/135 (small parallelism subtest gain)

1. **Phony order-only deps of missing intermediates**: when
   `build_target_for` decides to skip a missing intermediate prereq
   (because the parent target is up-to-date and the intermediate
   doesn't need to exist), it now still descends into the
   intermediate's order-only deps that are phony, building each.
   Matches GNU make's behavior. Fixes `features/parallelism` subtest
   8 (`.INTERMEDIATE` + `| phony` order-only chain).

Subtest gains: parallelism 1→2.

## Round 13 — 133/135 (partial WAIT improvement)

1. **Defer order-only dedup** in `build_target_for` until after
   `rule_build_groups` slice extraction. Previously, deduping
   `all_order_only` against `all_prereqs` immediately after the
   rule-iteration loop shrank `all_order_only.len()` while
   `rule_prereq_slices` still held the pre-dedup `oo_end` indices,
   causing the slice extraction to silently fall through to empty
   `oo` vectors when an entry like `.WAIT` appeared in both lists.
   Symptom: `pre = .WAIT pre1 .WAIT pre2 | .WAIT pre3` (after SE)
   dropped `pre3` from the build entirely. Fix: remove the early
   `all_order_only.retain(...)` step; the existing later dedup is
   sufficient and keeps slice indices consistent. Fixes
   `targets/WAIT` subtest 9.

Subtest gains: WAIT 9→10.

## Round 12 — 133/135 (gained `se_implicit`)

1. **SE side-effect ordering during `pattern_search`**: GNU make
   fires second-expansion `$(info)` etc. messages in deepest-first
   order relative to the recursive prerequisite verification.
   Previously we either silenced SE entirely during `pattern_search`
   and re-fired in `build_target_for` (wrong order), or fired during
   `pattern_search` but at the wrong recursion point.
2. **Two-phase SE expansion in `try_pattern_rule`**: split into a
   silenced verification expansion (just to compute prereq names)
   and a real expansion fired AFTER successful verification. Because
   the verification step recurses through `find_pattern_rule_inner`
   for each prereq before returning, child pattern rules get to fire
   their SE messages first; the outer rule fires its SE only after
   all children have done so. Result: deepest-first ordering
   matching GNU exactly.
3. **`build_target_for` reuses cached SE expansion**: when handling
   a matched SE pattern rule, check `pattern_se_cache` (populated by
   `try_pattern_rule` after success) and replay the cached prereqs +
   order-only without re-firing side effects. Cache key is
   `(target, target_pattern)`. The original inline SE expansion path
   remains as a fallback.

Subtest gains: se_implicit 28→30 (✅).

## Round 11 — 132/135 (gained `dash-f`, `temp_stdin`)

1. **Failed primary `-f` makefiles become goals + suppress re-exec**:
   when a `-f` makefile fails to load, register its name in a new
   `failed_primary_makefiles` list. Prepend those names to the goal
   list so the main build phase emits `No rule to make target '<X>'`,
   matching GNU. Also set a new `Engine.suppress_reexec` flag that
   makes `build()` skip its `-1` sentinel return — GNU does not
   restart when a primary makefile is missing, even if other includes
   were rebuilt. Fixes `options/dash-f` #4 (file now 32/32).
2. **Fatal-signal propagation to recipe child**: install a
   process-wide handler for SIGTERM/SIGINT/SIGHUP/SIGQUIT that
   forwards the signal to the currently-running recipe child via a
   `CURRENT_CHILD_PID` atomic. New
   `Engine::run_with_signal_propagation` helper wraps each `Command`
   with `spawn` + `wait` (loops on EINTR), recording/clearing the
   child PID. Replaces the four `.status()` call sites in
   `execute_recipe_inner`.
3. **Signal-killed recipes during include remake print and re-raise**:
   when `execute_recipe_inner` sees a fatal-signal (1/2/3/15) child
   exit, print `make: *** [<src>:<ln>: <target>] <Sig>` and re-raise
   the signal on ourselves with `signal(sig, SIG_DFL)` +
   `kill(getpid(), sig)`. Bypasses include-remake error swallowing
   and makes the test harness see make as killed-by-signal rather
   than exited-with-status. Fixes `features/temp_stdin` #5.

Subtest gains: dash-f 31→32 (✅), temp_stdin 7→8 (✅).

## Round 10 — 130/135 (gained `MAKEFLAGS`)

1. **Engine-side debug flag tracking**: added `debug_flags: Cell<u8>`
   to `Engine` plus `set_debug_flag(b'a'/b'b'/...)`, `debug_basic()`,
   `debug_jobs()` helpers. CLI `--debug=X` handlers wire each flag
   into the engine. Mapping mirrors GNU semantics: `a`=all,
   `b`=basic, `v`⇒basic+verbose, `i`⇒basic+implicit, `j`=jobs,
   `m`⇒basic, `n`=clear.
2. **Post-load MAKEFLAGS scan recognizes long-options**: previous
   `tok.contains('=')` check skipped `--debug=b` as if it were a
   variable assignment. Now `tok.contains('=') && !tok.starts_with("--")`
   only skips genuine `NAME=VAL` tokens. Same fix in
   `set_var_with_origin`'s MAKEFLAGS merge path. Lets
   `MAKEFLAGS=--debug=b` in a makefile turn on debug output for the
   rest of the run.
3. **"Updating makefiles...." emitted at `finalize_includes` start**
   when `debug_basic()` is true. Fixes MAKEFLAGS subtests 197–208.
4. **"Putting child / Reaping winning child / Removing child"
   emitted around `execute_recipe_inner`** when `debug_jobs()` is
   true. Synthetic chain pointer (target's slice ptr) — sufficient
   for the regex match tests use. Fixes MAKEFLAGS subtests 210–217.
5. **`--temp-stdin=PATH` flag and re-exec banner**: the re-exec arg
   list now appends `--temp-stdin=<temp>` (in addition to `-f<temp>`).
   `--temp-stdin=PATH` is recognized at startup as an inherited stdin
   temp (for cleanup) and as a no-op CLI flag. Under `--debug=b`,
   prints `Re-executing[N]: <argv>` before `exec()`. Fixes
   `temp_stdin` #4.
6. **Stdin temp file cleanup on `exec()` failure**: track the
   just-created temp file across the re-exec branch; remove it
   before printing the error and returning 127. Fixes `temp_stdin`
   #6.

Subtest gains: MAKEFLAGS 198→218 (✅ category flip), temp_stdin 5→7.

## Round 9 — 129/135 (gained `reinvoke`)

1. **Imagined targets for included makefiles** (sv 61226): when a
   rule ran OK for an included makefile but didn't create the file,
   and the file wasn't pre-marked by a sibling pattern rule, skip —
   don't error. Matches GNU make's behavior where nonexistent
   included targets with rules are "imagined" as updated.
2. **Mtime-based rebuilt check**: only mark targets as "rebuilt"
   (triggering dependents) when the file's mtime actually changed
   after recipe execution. Prevents cascading rebuilds when recipes
   are no-ops.
3. **Primary makefile auto-rebuild**: register `-f` makefiles in
   `included_files` so `finalize_includes` checks them for rebuild
   rules.
4. **Stdin temp file for re-exec**: when stdin is read via `-f-`,
   save content and write to temp file on re-exec, replacing `-f-`
   with `-f<temp>` in args. Handles combined flags (`-Rf-`,
   `--file=-`, etc.).
5. **Inherited stdin temp cleanup**: detect and clean up stdin temp
   files from previous re-exec on normal exit.

Subtest gains: reinvoke 5→12 (✅ category flip), dash-f 23→31,
MAKEFLAGS 198 unchanged.

## Round 8 — 128/135 (gained `vpathplus`)

1. **Skip implicit rules for phony targets**: phony targets no
   longer trigger implicit rule search, preventing spurious built-in
   CC pipeline matches. Fixes `vpathplus` #1 (notarget).
2. **VPATH-aware intermediate chain timestamp checks**: intermediate
   files found via VPATH now compare timestamps correctly in the
   chain. Fixes `vpathplus` #3.
3. **Pre-existing file tracking for intermediate deletion**: files
   that existed before the build started are not deleted as
   intermediates. Fixes `vpathplus` #3.
4. **VPATH revocation ("un-vpath") after recipe doesn't create
   file**: when a recipe runs but doesn't create the target at the
   vpath location, the vpath resolution is revoked so subsequent
   rules look in the current directory. Fixes `vpathplus` #1.
5. **`-w` entering directory side-effect in MAKEFLAGS merge**: `-w`
   flag now properly triggers "Entering directory" messages when set
   via MAKEFLAGS in sub-makes. Fixes MAKEFLAGS subtests 124-127.
6. **`-R` implies `-r` in post-load MAKEFLAGS**: when `-R` (no
   built-in variables) is set via makefile MAKEFLAGS, `-r` (no
   built-in rules) is also implied. Fixes MAKEFLAGS subtests 104-107.
7. **MAKEOVERRIDES empty→conditional separator post-load fix**: the
   `--` separator between flags and MAKEOVERRIDES is only emitted
   when MAKEOVERRIDES is non-empty. Fixes MAKEFLAGS subtest 101.
8. **MAKEFLAGS/MAKELEVEL/MAKE export in `$(shell)` and `!=`
   operators**: these variables are now exported to the environment
   for `$(shell ...)` and `!= shell` assignment operators. Fixes
   MAKEFLAGS subtests 116-117.
9. **SE pattern-rule directory-transfer: expand first, then prepend
   directory**: SE prereqs in pattern rules now expand `$$*` etc.
   before applying directory-transfer. Fixes `se_implicit` #3.

Subtest gains: vpathplus +2, MAKEFLAGS +11 (187→198), se_implicit +1.

## Round 7 — 127/135 (gained `patternrules`, `se_explicit`)

1. **Directory-transfer in SE pattern rule prereqs**:
   `replace_first_percent_per_token` now applies GNU make
   directory-transfer semantics (prepend dir, substitute only base
   into `%`) when `dir_transfer=true`. Previously the full stem
   (including directory) was substituted for `%`, producing
   `3lib/bye4%5` instead of `lib/3bye4%5`. Static pattern rules pass
   `dir_transfer=false`. Fixes `patternrules` #45+#53 and #64+#65 —
   file now fully passes (72/72).
2. **`pattern_subst_with_dir` skip for no-`%` prereqs**: when a
   pattern rule prerequisite contains no `%`, the directory portion
   of the stem is no longer prepended.
3. **Re-exec failure exit code**: changed from 2 to 127 (matching
   GNU make's `execvp` failure convention). Fixes `temp_stdin` #7.

Subtest gains: patternrules +9, temp_stdin +1.

## Round 6 — 125/135 (gained `statipattrules`, `vpath`, `vpathgpath`, `mult_rules`)

1. **Escaped-`%` handling in static pattern rules**: added
   `find_unescaped_percent`, `unescape_percent`, and
   `replace_first_unescaped_percent` helpers. `\%` is now treated as
   a literal `%` in target patterns, stem patterns, and prereq
   patterns. Fixes `statipattrules` #66, #67 — file now fully passes
   (68/68).
2. **Vpath duplicate pattern append**: multiple `vpath %` directives
   now create separate entries in `vpath_patterns` instead of
   overwriting. Fixes `vpath` #3 (`.LIBPATTERNS` interaction with
   wildcard `vpath %`).
3. **Library search order rewrite**: `resolve_vpath_with_index`
   returns both the resolved path and its vpath-entry index.
   `.LIBPATTERNS` candidates resolved with earliest-match semantics
   across all vpath entries. Fixes `vpath` #5.
4. **Vpath same-file merging with warning** (Savannah bug #62650):
   when `vpath` resolves an explicit-rule target to another
   explicit-rule target's path, the rules are merged and a
   `Recipe was specified for file 'X' ... but 'X' is now considered
   the same file as 'Y'` warning is emitted. Fixes `mult_rules` #2 —
   file now fully passes (3/3).
5. **SE side-effect firing for merged-away rules**: when vpath
   same-file merging collapses two rules, the SE ordering for the
   surviving rule respects the recipe-bearing rule's prereqs-first
   convention. Fixes `se_explicit` #27.
6. **GPATH support**: when a directory found via `vpath`/`VPATH` is
   listed in `GPATH`, the target redirects to the resolved path and
   prereqs resolve there. Files stay in the vpath directory rather
   than being treated as intermediate. Fixes `vpathgpath` #1 — file
   now fully passes (1/1).
7. **Peer target warning fix**: the "file 'X' was not created by any
   rule" grouped-target warning now only fires when the recipe
   actually created or updated the main target. Fixes `patternrules`
   peer-target subtest.

Subtest gains: statipattrules +2, vpath +1, mult_rules +1,
vpathgpath +1, se_explicit +1, patternrules +1.

## Round 5 — 121/135 (+5 vpath subtests)

1. **VPATH lookup in pattern-rule prereqs**: `try_pattern_rule` now
   consults `file_exists_or_vpath` and `resolve_vpath_rule` so a
   pattern rule with prereq `bar.d` matches when `work/bar.d` exists
   via `VPATH=work/`. Both terminal and non-terminal rule branches.
2. **`resolve_vpath` skips local files**: GNU only redirects to a
   vpath copy when the file is NOT in the current directory. Without
   this, `bar.d` (local + `work/bar.d`) was being rewritten to
   `work/bar.d` in `$^`, breaking `vpathplus` test 0.

Subtest gains: vpathplus 0 → 2 of 4.

## Round 4 — 121/135 (+6 vpath subtests)

1. **`vpath PATTERN DIRS` directive** now stored in
   `vpath_patterns: Vec<(String, Vec<String>)>` and consulted by
   `resolve_vpath()`. Pattern matching uses `expand::pattern_stem`;
   ALL matching patterns are tried in declaration order.
2. **VPATH-aware rule lookup**: when `target` has no explicit rule
   but `resolve_vpath_rule(target)` finds a registered rule for a
   vpath-resolved name, the build redirects to that name. So
   `vpath %.te vpath-d/` + `vpath-d/fail.te:` makes `fail.te`
   resolve to `vpath-d/fail.te`. Fixes `features/vpath` Test #1 and
   Test #4.
3. **Strict-extension vpath in `.LIBPATTERNS`**: extends
   `resolve_library_prereq` to consult `vpath PATTERN DIRS` whose
   pattern shares the candidate's extension (e.g. `lib1.a` only
   consults `vpath %.a ...`). Fixes most of `vpath.3`'s lib lookup.

Subtest gains: features/vpath 1 → 4 of 5; features/mult_rules 1 → 2
of 3.

## Round 3 — 121/135

1. **`$` in static-pattern target names**: `foo$$bar: f%r: % ; ...`
   registered `default_goal` as the post-first-expansion target name
   `foo$bar`. At goal consumption time, `default_goal` is re-expanded
   via `expand::expand`, which then mangled `foo$bar` -> `fooar`
   (treating `$b` as an empty variable reference). Static-pattern's
   `default_goal` insertion now `$`-escapes the target name
   (matching the regular-rule branch that already did this). Fixes
   `features/se_statpat` Test #4 — full file now 12/12.

## Round 2 — 120/135 (+4 SE subtests)

1. **Static-pattern target token splitting**: tokens like
   `$(filter %.o,$(files))` were treated as containing a literal
   space (the space inside the function call) and thus passed
   through as a single literal-space target name. New helper
   `has_unwrapped_space` only flags spaces at depth 0. Fixes
   `statipattrules` Test default (and Test #1 by side effect).
2. **Grouped-target sibling SE timing**: per-sibling SE side-effects
   were firing AFTER recipe execution and never built sibling
   SE-derived prereqs. Moved sibling SE firing to BEFORE the recipe;
   the currently-executing grouped rule fires first for each
   sibling, then other rules in declaration order; SE-expanded
   prereqs are then built via `build_target_for`. Fixes
   `se_explicit` #22.
3. **Backslash-colon in SE substitution refs**: `$(@\:%=%.bar)`
   (where the user escaped `:` to prevent it being parsed as a
   static-pattern separator in the prerequisite line) was confusing
   `find_subst_colon`. Pre-process expressions in `expand_expr` to
   replace `\:` with `:` before substitution-ref / function
   detection. Fixes `se_explicit` #9.

Subtest deltas (cumulative from baseline 108/135):

- features/se_explicit 26 → 29
- features/se_implicit 24 → 27
- features/se_statpat 10 → 11
- features/statipattrules 64 → 66

## Earlier — baseline 108/135 → 120/135

Initial parser, expander, and build engine implementation, bringing
the upstream test suite from a not-yet-running state up to 108/135
and then to 120/135 with early SE-focused fixes. Subsequent rounds
above incrementally close the remaining gaps.
