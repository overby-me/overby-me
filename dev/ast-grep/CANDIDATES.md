# Rule candidates from external sources

Working notes for the rule-mining pass. Each entry names the hazard, the
syntactic shape a rule would match, and where the idea came from. Entries are
clean-room: hazards and shapes are described in our own words, never copied
from the source - the Semgrep rules in particular are under the Semgrep Rules
License v1.0, which restricts reuse of the rule text itself.

A candidate is not a rule. The triage pass measures every entry against this
tree (scan, count, inspect each hit) and routes it to the cheapest layer that
holds it: a clippy config line beats an ast-grep rule beats a script. What is
rejected gets its measured reason recorded in NOTES.md; what has zero hits
ships only as a ratchet with a fixture that fires.

Sources being mined, with method notes:

- deslop (chinmay-sawant/deslop, archived 2026-08-01, MIT): source-level read
  of all 637 rule implementations, including the Go and Python trees whose
  hazards may generalise even though this tree has almost no Go or Python.
- semgrep-rules (Semgrep CE): only 10 Rust rules exist in the open tier - the
  registry's Rust depth is Pro-only. Mined for hazards, plus the C tree (maps
  onto unsafe Rust and Mojo FFI) and the go/python/generic trees by id.
- CodeQL (github/codeql, MIT): Rust queries by CWE; only the syntactic residue
  of each survives - the qhelp hazard writeups are the durable value.
- RustSec advisory-db: advisories grouped by bug shape, not by crate.
- Rudra / cargo-geiger: the three memory-safety classes and whether each has a
  syntactic signature.
- clippy 0.1.95's 321 not-yet-enabled allow-by-default lints: swept across all
  55 first-party cargo roots in one pass per root, findings tallied by lint.
  Enabling one is a config line in git-hooks.nix, so any candidate a clippy
  lint covers routes there, never here.

<!-- Findings are appended below as each source read completes. -->

## deslop: Rust + cross-cutting heuristics (source-level read)

Structural finding that reframes the whole source: of deslop's 150-entry Rust
rule table, most entries run through a generic keyword engine - a rule fires if
any auto-generated marker substring appears in the comment-stripped body. Many
specs are non-discriminating by construction, several are INVERTED (they fire
on the presence of the guard: the cloexec rule matches O_CLOEXEC, the
buffered-reader rule matches BufReader), and over a dozen are dead (no markers,
can never fire). Only 14 rules have real matchers, plus the manifest checks.
The known false-positive families all share one root: the evidence primitives
classify calls by NAME (`.read()`, `.write()`, `.get()`, `.join()`), so
`vec.join(",")` counts as blocking and `RwLock.write()` counts as a lock.

Hallucination rules: definitively not portable. Every accept/reject decision
consults a repository-wide index (cross-file symbol tables, module resolution,
recursive re-export chasing). No index-free part survives.

### Candidates (shape = what an ast-grep rule would match)

1. detached spawn: `tokio::spawn(...)` in statement position, or under
   `let _ =` / `drop(...)` - the JoinHandle is discarded, so panics in the task
   vanish. FP: deliberate fire-and-forget; spawn inside join!/select arms.
2. config structs accepting unknown fields: Deserialize struct named
   config/settings/options without deny_unknown_fields - typos ship silently.
   Must suppress when a #[serde(flatten)] field exists (flatten +
   deny_unknown_fields is invalid serde).
3. TOCTOU check-then-open: `$P.exists()` then `File::open($P)` in one fn, SAME
   metavariable - ast-grep's metavar reuse gives the path-identity check
   deslop itself lacked. Keep to File::open/fs::read*; never bare `.open(`.
4. join with an absolute literal: `$P.join($LIT)` where $LIT starts with `/`
   AND is longer than "/" alone - the bare "/" literal is the slice-join
   separator idiom, which is exactly what made the naive version wrong 5/5 on
   this tree. Also gate out files importing url:: (Url::join replaces by
   design).
5. async on a raw thread with no runtime: thread::spawn whose closure contains
   async/await but the fn never names block_on/Runtime/Handle - the future is
   never polled, the code silently never runs. Complements the ported
   spawn+block_on rule (this is the WITHOUT branch).
6. runtime built per call: Runtime::new / Builder::new_* inside any fn not
   named main - nested construction panics inside async context. Severity up
   inside a loop.
7. blocking Drop: inside `impl Drop`, calls to block_on / JoinHandle-join /
   thread::sleep ONLY - dropping deslop's read/write/open name list, which was
   its FP engine.
8. walker filters disabled: the literal calls .hidden(false), .ignore(false),
   .git_ignore(false), .follow_links(true) on ignore/walkdir builders -
   secrets and artifacts swept in, symlink escape out. Only in files importing
   ignore/walkdir.
9. Condvar::wait not inside a loop: spurious wakeups are real; wait outside
   while/loop skips the predicate re-check. Exclude wait_while/_timeout_while.
10. `static mut` declaration: rustc 2024's static_mut_refs covers uses, not the
    declaration on older editions.
11. #[serde(default)] on a required-looking non-Option field (id/endpoint/
    host/url names) - a missing required value silently becomes zero. Info
    severity; measurable FP rate expected.
12. thread::spawn inside a loop - unbounded thread creation; needs a not-has
    for the spawn-push-join-all idiom to be tolerable.
13. Manifest/build layer (NOT ast-grep; route to the nu-script layer):
    release profile without overflow-checks; build.rs invoking git/curl/
    reqwest or containing http URLs (hermeticity); wildcard or git/path deps;
    dev-dependency imported from src/**.

### Rejected with source-level reasons

- split_at "external input": both halves need dataflow; residue ("split_at
  without an assert in the fn") does not discriminate.
- partial-init escape: `: None` in a constructor is indistinguishable from the
  ubiquitous correct pattern without invariant knowledge.
- builder-without-validate: needs struct-to-impl method-set reasoning; even
  deslop only flags infallible builders, many infallible by design.
- default-produces-invalid: pure field-name guessing.
- lock/permit/blocking across await: the loose name tables ARE the defect;
  no scope tracking exists to port.
- async lock-order cycle: cross-function aggregation.
- test-quality counters: helper-fn assertions are the norm here; already
  rejected once with measurement (NOTES.md).

## semgrep-rules (clean-room: hazards in our words, no rule text copied)

Only 10 Rust rules exist in the open tier; the registry's Rust depth is
Pro-only. Assessed all 10 plus the C/Go/Python/generic trees. The miner
verified tree surface for each claim below.

### Rust tier: worth taking

- TLS validation switched off, two shapes. (a) reqwest builder told to accept
  invalid certs/hostnames with a true argument: reqwest is in 11 manifests,
  zero current calls - a clean ratchet on a large live surface, near-zero FP.
  (b) rustls dangerous-config escape hatch installing a custom verifier: ONE
  live hit, safety/oxidized/curl connection.rs ~1130, which is the port's -k
  flag and gated on opts.insecure - one justified suppression, then a ratchet.
- Broken hash construction (Md2/Md4/Md5/Sha1 constructor calls): LIVE - curl
  (HTTP digest/NTLM) and cachix use md-5/md4 by protocol. Day one is a handful
  of justified suppressions in ports; thereafter catches new weak-hash use.
  Match both qualified and bare constructor spellings (no import resolution).
- Predictable names under the shared tmp dir (narrowed from the blanket
  temp-dir rule, whose bare form would fire on 34 legitimate files): join of a
  temp-dir call with a string literal, or a literal starting /tmp/ used as a
  path/socket. LIVE hits: cachix daemon socket at a fixed /tmp literal
  (squat-able), gcc driver debug dump at a predictable /tmp name. Known FP
  shape: container-internal paths in flatpak bwrap arg lists; test fixture
  dirs (scope out test modules).

### Rust tier: skipped with reasons

argv/current_exe trust rules (every CLI port reads args legitimately; the
systemd port deliberately self-re-execs via current_exe); auth-header
sensitivity (needs ordered multi-statement matching; 42 sites would fire day
one); blanket unsafe-block flag (1,907 files of FFI; clippy already gates
multiple_unsafe_ops_per_block); openssl verify-none (tree is rustls-only -
fold into the TLS family as ratchet or drop).

### C-derived

- Repeat-free of the same pointer: relational rule, `$P.free()` following an
  earlier `$P.free()` with no intervening reassignment of $P - metavariable
  unification across relational sub-rules makes this expressible. Target MOJO
  first (205 pointer/alloc lines; existing rules cover leak-on-raise and
  destructor-free but not double-free). Rust surface ~3 sites (from_raw twice)
  - ratchet. Blind spot: reassignment inside a nested block is invisible.
- Banned legacy C symbols through FFI (gets/strcpy/strcat/strtok/sprintf via
  libc:: or Mojo external_call): zero current hits, zero FP by construction -
  a free ratchet guarding future FFI in both languages.
- Use-after-free and format-string rules: not portable (flow); Rust format
  macros require literal format strings anyway.
- Secret zeroing: not a lint, but the miner surfaced an ARCHITECTURE GAP -
  credential-holding code (spindle-secrets, cachix auth) with no zeroize-style
  dependency anywhere in the tree. Record as its own follow-up, not a rule.

### Go/Python-derived

- Blocking thread-sleep inside async fn, excluding closures (the
  spawn_blocking escape): 197 sleep sites and 138 async files tree-wide,
  overlap unmeasured; any hit is near-certainly a bug. Complements the
  existing block_on rule - same executor-stall family, different call.
- String-formatted SQL (execute/query call whose argument is a format!
  invocation): TWO live hits in apps/wiki (oauth.rs ~149 DELETE FROM {},
  migration-loader ~389), both interpolating identifiers - the canonical
  near-FP for this class, but that is what audit severity + a visible
  justification is for.
- reqwest Client::new() with no timeout in server code: 40+ sites in wiki's
  backend_api.rs alone. CAVEAT the triage must settle: that file may compile
  to wasm where the browser owns the timeout and the builder has none - scope
  to native crates or it false-fires on the whole frontend. Also incidentally
  exposes per-call client construction (no pooling).
- dangerous_inner_html bound to any expression: Dioxus's only unescaped-HTML
  door, in an app that renders user-authored documents. ONE current hit
  (static font CSS in wiki main.rs - one suppression), then a ratchet on the
  app's actual XSS sink.
- Shell-string execution (Command naming sh/bash/nu with -c and a non-literal
  string): heavy legitimate use in the awk/make/ninja/bash ports whose SPEC is
  "run this string" - path-scope to apps/ and platform/ where there are zero
  hits today; ratchet for service code.
- Archive-entry name joined into extraction path (zip/tar-slip): ratchet;
  the tar port builds paths differently, no current shape. Valuable the day a
  service accepts uploaded archives.
- Decompression without a size cap: LIVE partial-coverage hits -
  pptx-parser reads zip entries to string with no visible cap (~2296, ~2324)
  in the wiki's user-upload path; most other ooxml reads are take-capped.
  Coverage is partial by construction (reader bound to a variable first is
  invisible) - must be labeled audit, not gate.
- Permissive file modes: skip tree-wide (ports mirror upstream umask idioms
  by spec); service-scoped variant has near-zero surface.

### Generic tier (257 rules)

~225 are secrets regexes: ripsecrets already owns this engine class. Two
residues worth a hook line rather than ast-grep rules: credentials embedded in
URLs (scheme://user:password@host - low-entropy, so entropy-based detection
misses it), and Trojan Source bidi-override codepoints (tree currently clean;
free ratchet; one grep over invisible-control ranges beats N per-grammar
rules). Everything else (nginx/dockerfile/hugo/gradle/...) has no surface.

## deslop: Go + Python heuristics (source-level read)

Same structural verdict as the Rust tree, now measured precisely: the Python
tree's ~430 rules split into ~40 real AST walks (T1), ~60 indentation-based
line walks (T2), and the rest substring co-occurrence (T3) or rules whose
matcher is COMPILED FROM THE RULE ID ITSELF (T4 - 100 rules per language,
markers derived from the id tokens, satisfied by comments; the generated tests
pass by placing markers in comments). Several rule names claim ordering or
scoping their matcher does not implement. The Go tree is honest line-substring
matching with an import-alias resolver and a brace-depth loop tracker; its
blocking-io rule resolves MODULES rather than method names, which is the right
version of the trick deslop's Rust tree got wrong.

### Convergent candidates (found independently by 2-3 readers; strongest)

- TOCTOU exists-then-open, same-metavariable: all three readers arrived at
  `$P.exists()` followed by `File::open($P)`/`fs::read($P)` with $P unified.
  Match only the positive-exists direction; exclude create-if-missing and
  OpenOptions.create(true) flows. Mojo analogue: os.path.exists + open.
- Detached spawn (statement-position tokio::spawn / let _ = / drop(...)):
  Go and Rust readers; Python reader notes JoinHandle is #[must_use] and
  clippy has let_underscore_future - the triage must measure what rustc
  already reports before shipping this.
- Invariant construction inside a loop, literal-argument constrained:
  Regex::new($LIT) / client builders / parse-with-literal-format inside a
  loop node. The string-literal argument IS the invariance proof. Check
  clippy 0.1.95 for regex_creation_in_loops coverage first; client/engine
  shapes are uncovered.
- Blocking under a held sync lock: .await / thread::sleep / Command::new
  following `let $G = $M.lock().unwrap()` in the same block, excluding
  .lock().await receivers. FP: guard dropped or scoped before the await -
  accept misses, keep the shape tight.
- Outbound client without a deadline: reqwest::Client::new() (can never set
  a timeout), builder chains without .timeout(, and reqwest::get() (fresh
  default client per call). Converges with the semgrep miner's finding and
  its wasm caveat: wiki frontend code may compile to wasm where the browser
  owns timeouts - scope to native crates.
- Unbounded ingest: read_to_end/read_to_string/.bytes()/.text() in fns
  without .take(/limit markers, scoped to server code; converges with the
  pptx-parser live hits from the semgrep pass.
- Shell -c with a non-literal string; TLS verification disabled; weak hash
  constructors; hardcoded secret literals (identifier-regex + string-literal
  RHS, with `token` and `key` EXCLUDED from the word list - this monorepo
  uses both pervasively for lexer tokens and map keys); permissive file
  modes (world-writable octal regex, sticky-bit excluded): all converge with
  the semgrep list; single-call-node shapes, glob tests out.

### Distinct candidates worth triage (single-reader)

From Python:

- Swallowed failure arms: `Err(_) => {}` / `if let Err(_) = $E {}` empty
  blocks only - deliberately excluding `let _ =` (idiomatic best-effort).
  Nix: (builtins.tryEval $E).value used without consulting .success.
- Cause discarded on rewrap: `.map_err(|_| $NEW)` - the wildcard closure
  parameter IS the discard, so this is precise; exclude fmt::Error and ()
  payloads (std convention).
- Equality-or chains on one scrutinee (matches!/elem rewrite): Rust, Mojo,
  and Nix (`$X == $A || $X == $B` -> builtins.elem); near-zero FP.
- Eval-time work in NIX (the high-value half): builtins.getEnv (returns ""
  under pure eval - a correctness trap), currentSystem, fetchTarball/fetchurl
  with no hash attr, builtins.exec. statix covers none of these. Rust half:
  lazy_static/LazyLock initializers doing env/File/Command.
- Push-then-sort in the same loop ($V unified); sort-then-take-first;
  Vec::remove(0) in a loop (queue misuse); .flush() inside a loop.
- Comment hygiene: commented-out code by regex on comment nodes (keyword +
  trailing paren/=/colon; NEVER bare `!` - //! is a doc comment), emoji/slop
  subset near-zero-FP.
- Secret-named identifiers interpolated into format!/panic!/tracing macros
  (same reduced word list as above).

From Go:

- Sleep/spin polling inside a loop: REPO CAVEAT from two memories - the VM
  test harness and the /loop cadence poll deliberately; scope out test trees
  or expect the suppressions to outnumber the catches.
- Remote round-trip per iteration (N+1): await-bearing client/query calls
  inside for loops, info severity only - pagination loops are the protocol.
- Panic on expected-error messages: panic!/unreachable! whose message
  matches (?i)(invalid|missing|not found|unsupported|unexpected).
- Errors reduced to strings: .map_err(|$E| format!(...)), branch-on
  $E.to_string().contains(...) - glob out tests and bin boundaries.

### Not portable (agreed across readers, with the reason)

Match-count aggregation (duplication, repeated-same-arg caching, N-of-M
rules); cross-file anything (import graphs, interface impl counting,
hallucination index); dataflow (taint sinks, assigned-but-never-awaited,
guard-lifetime across await); type information (async-aware lock detection,
TypedDict/Optional, naive-vs-aware datetime); hazards Rust's ownership or
RAII already forbids (channel close/send-after-close, loop-var capture,
context-manager family, mutex copies); Python-only semantics (mutable
default arguments - def-time evaluation, late-binding closures, f-string
lazy logging); everything clippy already owns (needless_range_loop,
map_entry, float_cmp, manual_strip, len_zero, print/dbg leftovers).

## clippy 0.1.95 allow-by-default sweep (measured, all 321 candidates)

One `cargo clippy` pass per root with every not-yet-enabled allow-by-default
lint as -W, across 54 first-party cargo roots (one root failed to compile
standalone), findings tallied by lint code from the JSON stream: 344,611
findings, 258 lints firing, 97 lints with ZERO findings. Raw tallies in the
sweep artifacts; the numbers below are the routing-relevant cut. The clippy
hook excludes safety/oxidized, so first-party counts are what enabling
actually costs.

### Free to enable now (zero findings tree-wide, correctness-relevant)

path_buf_push_overwrite (the clippy cousin of the join-absolute hazard),
suspicious_xor_used_as_pow (2^8 typo), mutex_integer, rc_clone_in_vec_init
(one Rc cloned N times when N Rcs were meant), tests_outside_test_module,
disallowed_script_idents (Trojan-source adjacent), string_to_string.
Zero-cost ratchets; a config line each.

### Cheap to clear first-party (hook excludes oxidized)

unnecessary_safety_comment 0 first-party / create_dir 1 / empty_drop 1 /
mixed_read_write_in_expression 3 / missing_assert_message 2 first-party.

### Real decisions (first-party vs oxidized split measured)

- undocumented_unsafe_blocks: 134 first-party vs 1,458 oxidized. The deslop
  unsafe_without_safety_comment equivalent, type-aware. 134 is a clearable
  backlog; this is the highest-value single enablement on the table.
- map_err_ignore: 77 first-party. This is clippy's native version of the
  `.map_err(|_| ...)` cause-discard candidate from the deslop mining - ROUTE
  THE AST-GREP CANDIDATE HERE instead of writing a rule, if the 77 inspect
  as real.
- unwrap_in_result: 0 first-party (all 408 are oxidized) - free for the
  gated tree.
- iter_over_hash_type: 11 first-party - nondeterministic iteration order;
  cheap and real.
- shadow_unrelated 210 / let_underscore_must_use 295 first-party: worth a
  look, likely too noisy; triage decides.
- allow_attributes_without_reason 123 first-party: would force a reason
  string on every allow - pairs well with the unused-suppression rule
  already shipped.

### Explicitly not worth enabling (measured noise)

implicit_return 13,391 / min_ident_chars 8,094 / missing_docs_in_private_items
6,075 / arbitrary_source_item_ordering 3,304 / question_mark_used 2,942 /
single_call_fn, pub_use, mod_module_files (style-war lints);
arithmetic_side_effects 20,814 and indexing_slicing 17,639 (real hazard class
but unpayable here - the decoders and ports index and add by design; the
narrow ast-grep rules already cover the unsafe subset that matters).

## CodeQL + RustSec + Rudra (concept mining)

CodeQL's Rust pack is 19 security queries; most are taint-based and contribute
only their sink lists. The ones that survive whole as syntactic rules, with
MIT-licensed reusable pieces noted:

- Disabled cert check: converges with the semgrep/deslop finding; CodeQL's own
  sink heuristic IS the syntactic form (method name + literal true).
- http:// string literal, with CodeQL's private-host exclusion regex reusable
  verbatim (localhost/127./192.168./10./172.16-31/[::1]/fc-fd). FP: xmlns and
  schema URLs - needs an allowlist.
- Weak-crypto BY NAME: CodeQL ships this dataflow-free already. Reusable
  algorithm lists (MIT): DES/3DES/RC2/RC4/RC5 family, MD2/MD4/MD5/RIPEMD/
  SHA0/SHA1, mode ECB. Overlaps the semgrep insecure-hashes candidate; the
  cipher and ECB names extend it.
- Hardcoded key/IV/nonce/salt: literal or all-literal array passed to
  Key/Nonce/Iv constructors or nonce/iv/salt/password-named params. CodeQL
  deliberately does NOT match the name "key" - "too many false positives" -
  which independently confirms this tree's token/key exclusion decision.
- Cookie built without .secure(true) in one expression chain.
- std calls inside #[ctor]/#[dtor] fns (std is only guaranteed inside main).
- Sensitive-name-in-log: CodeQL's sensitive-name regex families are MIT and
  reusable verbatim; audit severity only.
- format!-at-sink for sqlx query calls (converges with the live wiki hits).
- Unchecked * or + directly in with_capacity/reserve/Layout/from_raw_parts
  args (see RustSec shape 5); audit severity, scope to unsafe-bearing files.

RustSec: 573 category-tagged advisories censused, 49 read. Bug shapes by
incidence, each with advisory anchors:

1. `unsafe impl Send/Sync` with missing bounds - THE largest cluster (~50
   advisories) and Rudra's highest-yield class. The fact is entirely in the
   impl header: match every unsafe Send/Sync impl whose type params lack
   Send/Sync bounds. Which bound the param needs (Send-for-Sync cases) is
   type-level - flag-for-audit is the honest ceiling. RUSTSEC-2020-0121/0122.
2. Panic-unsafety: ptr::read/copy then a call to a generic value (closure,
   .clone(), .next()) with no ManuallyDrop/forget guard in the same fn -
   unwinding double-frees. Rudra class 1. Audit-grade. RUSTSEC-2021-0018/0042.
3. set_len(n) after with_capacity, buffer then handed to .read()/.read_exact()
   - ~25 advisories. The FFI-fills-buffer idiom (common in THIS tree) is the
   FP; the read-handoff tier is the shippable rule. RUSTSEC-2021-0138.
4. mem::uninitialized() and MaybeUninit::uninit().assume_init() chained in one
   expression: near-zero FP. (The chained form narrows the existing
   assume_init rule; keep both.)
5. Length arithmetic overflow feeding allocation, and `$A * $B == $X.len()`
   safety comparisons. RUSTSEC-2026-0007 (bytes), 2023-0080 (transpose).
6. size_hint() trusted inside a fn that also contains unsafe. RUSTSEC-2021-0003.
7. Truncating len casts in wire/FFI code: `$X.len() as u32` etc. Diesel's DEF
   CON protocol-injection fix was literally denying cast_possible_truncation
   (measured this sweep: 3,261 hits tree-wide - too noisy repo-wide, viable
   scoped to protocol/FFI dirs). RUSTSEC-2024-0365.
8. CString temporary: CString::new($X).unwrap().as_ptr() in one expression -
   freed at end of statement. rustc lints this now (temporary_cstring_as_ptr);
   the MOJO analogue at the C-ABI does not exist anywhere - worth a Mojo rule.
   RUSTSEC-2025-0022. Also CStr::from_ptr on fixed-size struct fields.
9. Safe pub fn taking *const/*mut params (or named from_raw/from_ptr) with an
   unsafe body: the audit worklist for an FFI-heavy tree. RUSTSEC-2021-0152.
10. repr(packed) without C in the same repr list: exact match, near-zero FP,
    definite fix; rustc 1.80 field reordering broke real crates.
    RUSTSEC-2024-0346.
11. extern "C" fn whose body can panic with no catch_unwind: today an abort
    (reliability), was UB. Directly portable to Mojo C-ABI callbacks.
    RUSTSEC-2019-0038.
12. env/locale mutation thread-safety (chrono/gettext-rs advisories) -
    already shipped as rust-env-set-var; libc::setenv/setlocale spellings
    extend it.
13. Constant-true verifier callbacks: `|_, _| true` or bodies that are just
    Ok(())/0 passed to verify/certificate/host-named params. Near-zero FP.
    The inverted-boolean bug (RUSTSEC-2026-0141) has NO syntactic signature -
    known miss, recorded. RUSTSEC-2023-0002.
14. Archive-entry name joined into a destination path - converges with the
    semgrep candidate; the zip crate's own docs point at enclosed_name.
    RUSTSEC-2021-0126.

Rudra's three classes map onto the above: class 3 (Send/Sync variance) is
fully syntactic and was the highest-yield; class 1 (panic safety) is shape 2
at audit grade; class 2 (higher-order invariants) is not syntactic in general
but its two dominant instances are shapes 3 and 6.

cargo-geiger: counts unsafe per crate across the whole dependency graph with
a used/unused split - keep as a tool, do not reimplement; repo-local ast-grep
cannot see the dependency tree.

Clones for the triage pass: scratchpad/{codeql,advisory-db} + rudra/geiger
readmes, /tmp/deslop-src, /tmp/semgrep-rules, sweep artifacts in /tmp.

## Triage verdicts (measured against this tree, every hit inspected)

Method: prototype rules run via a scratch sgconfig over the real tree,
first-party (fp) counted separately from vendor/oxidized; every fp hit read
before judging. Full location lists in the session triage artifacts.

### Implement as ast-grep rules (#39)

Ratchets at 0 first-party findings, each requiring a firing fixture:
unsafe Send/Sync impls GENERIC-ONLY (the 3 non-generic fe-c impls carry
SAFETY comments and are the shape done right - generic-with-missing-bounds
is the bug class); repr(packed) without C; mem::uninitialized + chained
uninit().assume_init(); set_len-then-read_exact handoff; size_hint inside
unsafe-bearing fns; TLS danger_accept_* (the bare |_,_| true closure form
DROPPED: 11/11 hits were Dioxus UI predicates - keep only the reqwest/rustls
API forms); weak-hash constructors (2 protocol-mandated oxidized uses live
outside the hook); thread-sleep-in-async (197 sleep sites, ZERO intersect
async fns); TOCTOU exists-then-open same-$P (three sources converged; tree
clean); await-after-sync-lock-guard; Err(_)=>{} swallow; Condvar wait outside
loop; static mut; thread::spawn in loop; walker filters disabled; Runtime
built outside main; sh -c scoped to apps/platform/ai; alloc arithmetic
NARROWED to from_raw_parts/Layout (the general with_capacity form hit 45
benign graphics-grid sites); http:// URL literals with tests excluded;
Path::join("/...") with the [^"] regex (the naive form was 5/5 slice-join
separators - `.` matched the closing quote); secret-named identifiers in LOG
macros only (format! included pulled in 4 legitimate auth-header builds);
Mojo double-free ($P.free() follows $P.free()); Mojo external_call naming
banned C string functions; Nix builtins.getEnv (zero uses today!);
Nix fetchTarball/fetchurl without a hash attr; Nix currentSystem (1 hit,
guarded by `or` fallback - suppress or not-inside-or refinement).

Live findings to act on in #39:

- extern "C" fn with unwrap and no catch_unwind: 1 real hit,
  dev/mojo/gui/xr/native/shim lib.rs ~1881 (panic across the C ABI aborts).
- reqwest::Client::new() with no possible timeout: glob out apps/wiki (dual
  wasm/native target where the browser owns timeouts); tangled-api (7),
  nhost excluded with wiki, spindle router (1), oauth (2) are native and
  real - fix or justify in-line.
- SQL built by format!: the corrected pattern (trailing args allowed) found
  17, all in wiki's migration-loader and appview store DDL assembly where
  identifiers cannot be bound params - files-ignore those two crates, rule
  ships as a ratchet for runtime SQL.

### Route to clippy config (git-hooks.nix lines, not rules)

Zero-finding frees: path_buf_push_overwrite, suspicious_xor_used_as_pow,
mutex_integer, rc_clone_in_vec_init, tests_outside_test_module,
disallowed_script_idents, string_to_string, unwrap_in_result (0 fp),
unnecessary_safety_comment (0 fp). Near-free (inspect while enabling):
create_dir 1, empty_drop 1, mixed_read_write_in_expression 3,
self_named_module_files 5, missing_assert_message 2 fp, iter_over_hash_type
11 fp. Deferred as backlogs with real counts: map_err_ignore 77 fp,
undocumented_unsafe_blocks 134 fp - each its own clearing task, same shape
as the wiki-clippy backlog task.

### Route to a pre-commit grep (not ast-grep)

Trojan-source bidi codepoints over rs/nix/mojo/nu (tree clean; one line).

### Rejected, with the measured reason

Fixed /tmp literals: 63 fp, dominated by sandbox-internal paths (spindle
nix builds, plugin-tramp remote agent) - the flatpak-FP class predicted by
the miner; the real hits live in oxidized outside the hook. Vec::remove(0)
in loop: 7/7 are bounded LRU eviction on tiny queues. flush-in-loop: 6/6
are decoder DPB.flush() - a domain method, not IO; matching by method name
repeats deslop's core mistake. equality-or chains: pattern inexpressible
(no regex backreferences in Rust's regex crate) AND zero grep surface.
Bare always-true closures: 11/11 Dioxus predicates. General allocation
arithmetic: 45 benign graphics sites. Commented-out code: 70 hits but the
deepcomment editorial pass owns this space. Credentials-in-URL grep: 6
hits, all port test fixtures plus one deliberate publish.nu design.
Unbounded read_to_end in server crates: 9/9 read trusted local artifacts.
Detached tokio::spawn: JoinHandle is #[must_use], statement position
already warns via rustc, and `let _ =` is the deliberate-suppression idiom

- redundant. CString-temporary as_ptr: rustc's
dangling_pointers_from_temporaries covers it (verified by compiling the
shape); the Mojo C-ABI analogue has no such guard and stays on the Mojo
list. Cookie-without-Secure: no first-party cookie issuance (Hasura/PDS own
sessions). #[ctor]/#[dtor]: zero uses.
