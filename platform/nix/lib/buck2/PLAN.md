<!-- rumdl-disable MD046 -->

# nix-buck2: pure-eval Buck2 builds, one derivation per action

Build upstream [Buck2](https://github.com/facebook/buck2) projects with Nix,
parsing the Starlark build files at evaluation time and lowering each Buck2
*action* to its own Nix derivation. No import-from-derivation, no `buck2`
binary in the loop, no generated Nix committed to the tree. A sibling to
`platform/nix/lib/cargo` (per-crate derivations from `Cargo.lock`) and `platform/nix/lib/deno`,
exposed through the flakelight `perSystemLib` module.

Status: in progress. See milestones at the bottom.

## Why split Starlark from Buck2

Buck2's build files are [Starlark](https://github.com/bazelbuild/starlark)
(the language formerly called Skylark), the same dialect Bazel, Isolate, and
several other tools use. A Starlark interpreter is therefore useful well
beyond Buck2. This work is two libraries:

- `platform/nix/lib/skylark`: a pure-Nix Starlark interpreter (lexer, parser,
  tree-walking evaluator, standard builtins). Knows nothing about Buck2. A
  Bazel front end, a `.bzl` linter, or a config loader could reuse it
  unchanged.
- `platform/nix/lib/buck2`: the Buck2 semantics layered on top. It supplies the Buck2
  Starlark globals (`rule`, `attrs`, `provider`, `cmd_args`, `glob`,
  `ctx.actions.*`, ...), the cell/label/loader machinery, the analysis
  driver, and the lowering from Buck2's action graph to Nix derivations.

The seam is a small, explicit interface (below): the interpreter takes an
injectable global environment, an injectable `load()` resolver, and threads an
opaque `world` accumulator through evaluation so a host can collect effects
(target registrations, action registrations) without the interpreter knowing
what they are.

## Why per-action derivations, no IFD

`buck2 build //...` runs one opaque tool that owns the whole graph. Wrapping
that in a single Nix derivation gives Nix nothing: no per-target caching, no
parallelism Nix can see, and a full rebuild on any input change. The value of
doing it in Nix is the same as the cargo library's: map each *build unit* to a
derivation so that

- every action is cached independently in the Nix store (and cachix),
- Nix schedules the action DAG across cores and remote builders,
- editing one source file rebuilds only its action and that action's
  dependents, not the project.

For Buck2 the natural unit is the *action* (a single `ctx.actions.run` /
`write` / `download_file`), which is finer-grained than a target: a target can
declare several actions, and each becomes its own derivation.

"No IFD" means the entire load and analysis pipeline runs inside a single Nix
evaluation over source files, producing the action DAG as ordinary Nix data;
we never build a derivation and read its output back to continue evaluating.
This is possible because Starlark is deterministic and total (no ambient I/O,
no unbounded loops, no `Date.now`), and because everything analysis needs is
in the source tree:

Allowed at eval time:

- `builtins.readFile` on `.bzl` / `BUCK` / `PACKAGE` / `.buckconfig` files.
- `builtins.readDir` for `glob(...)` (enumerating a package's sources).
- `builtins.path { path = <file>; }` to turn a source file into a
  content-addressed store path (a "source artifact").
- Fixed-output fetches whose hash is written in the Starlark source
  (`ctx.actions.download_file(out, url, sha256 = "...")` maps directly to
  `fetchurl`). This is how the upstream Go toolchain is obtained, purely.

Banned:

- Reading any derivation output during evaluation (IFD).
- Running `buck2`, Bazel, or any compiler during evaluation.
- Unpinned network access during evaluation.

## Target: the `no_prelude` example first

Upstream ships `examples/no_prelude`, which defines all rules and toolchains
itself instead of pulling the large `buck2-prelude`. That makes it the
tractable first corpus: the whole Starlark surface it uses is a few hundred
lines we can read. Its shape (verbatim from upstream):

```text
examples/no_prelude/
  .buckconfig            # [cells] root = .  /  toolchains = toolchains
  .buckroot              # empty repo-root marker
  cpp/rules.bzl          # cpp_binary, cpp_library (rule(), attrs.*, ctx.actions.run)
  cpp/hello_world/BUCK   # cpp_binary(name="main", srcs=glob(["*.cpp"]), ...)
  cpp/library/BUCK       # cpp_library(..., visibility=["PUBLIC"])
  rust/rules.bzl         # rust_binary
  rust/BUCK              # rust_binary(name="main", file="main.rs", ...)
  go/rules.bzl           # go_binary
  go/BUCK                # go_binary(...)
  toolchains/BUCK        # cpp/rust/go/export_file toolchain targets
  toolchains/cpp_toolchain.bzl   # CxxCompilerInfo provider, is_toolchain_rule
  toolchains/rust_toolchain.bzl  # RustCompilerInfo provider
  toolchains/go_toolchain.bzl    # download_file + write + symlink (the hard one)
  toolchains/export_file.bzl     # trivial pass-through rule
```

Verticals in ascending difficulty:

1. **cpp** and **rust**: a single `ctx.actions.run` invoking a local compiler
   (`clang++`, `rustc`) named by the toolchain's `command` string. One source
   compile, one output. This is the first milestone target.
2. **go**: `ctx.actions.download_file` (a `.tar.gz` with a `sha256` in the
   source: a clean `fetchurl`), then `ctx.actions.write` of an unpack script
   with `allow_args`, a symlink action, and `cmd_args` with `format=`,
   `relative_to=`, `delimiter=`, `hidden=`. Exercises the full artifact and
   `cmd_args` model. Later milestone.

The Starlark constructs these files use (the interpreter must cover all of
them):

- `load("@cell//pkg:file.bzl", "sym", ...)`, `load("//pkg:f.bzl", ...)`,
  `load(":f.bzl", ...)`.
- `def f(ctx): ...` and `def f(ctx: AnalysisContext) -> list[Provider]: ...`
  (type annotations on params and returns, parsed then ignored).
- `if/elif/else`, `for`, `return`, module-level assignments and calls
  (`toolchain_config = _toolchain_config()`).
- Calls with positional and keyword args; `*`-free (no varargs in the corpus,
  but the parser will accept `*args`/`**kwargs` for generality).
- List, dict, tuple literals; list comprehensions; ternary
  `a if cond else b`; `+` on lists and strings; subscripting `x[k]`;
  attribute access `x.y`; method calls `x.y(...)`.
- Tuple-unpacking assignment `script, _ = ctx.actions.write(...)`.
- `struct(a = 1, b = 2)` and field access; `provider(fields = [...])` and
  provider instances `CxxCompilerInfo(compiler_path = ...)`; provider indexing
  `dep[CxxCompilerInfo]`.
- Mutation as statements: `cmd = cmd_args(); cmd.add([...])`;
  `xs = []; xs.append(x); xs.extend([...])`.
- `glob([...])`, `host_info()`, `fail(...)`, `oncall(...)`.

## Skylark interpreter (`platform/nix/lib/skylark`)

Builtins-only (no `pkgs`, no nixpkgs `lib` beyond what we pass in), mirroring
`platform/nix/lib/cargo/lib`: unit tests run with a bare
`nix eval -f platform/nix/lib/skylark/tests/<mod>.nix`.

```text
platform/nix/lib/skylark/
  default.nix    # public API + assembly
  lexer.nix      # source -> token list (indentation, strings, numbers, ...)
  parser.nix     # tokens -> AST (recursive descent, Pratt for expressions)
  values.nix     # value model, type(), truthiness, equality, repr
  eval.nix       # tree-walking evaluator (env + world threading)
  builtins.nix   # standard Starlark builtins + string/list/dict methods
  tests/*.nix    # assert-based eval tests per module
  README.md
```

### Value model

Scalars are represented as native Nix values (`int`, `float`, `bool`,
`string`, and `null` for Starlark `None`). Compound and host values are tagged
attrsets carrying a `__sk` discriminator:

- `{ __sk = "list"; items = [ ... ]; }` and `{ __sk = "tuple"; items = ...; }`
- `{ __sk = "dict"; entries = [ { key; value; } ... ]; }` (insertion-ordered;
  string keys are the common case and get a fast path)
- `{ __sk = "function"; name; params; body; closure; }` (user `def`/`lambda`)
- `{ __sk = "builtin"; name; fn; }` (a Nix function; see calling convention)
- `{ __sk = "struct"; fields = { ... }; }`
- `{ __sk = "object"; attrs = { <name> = value; }; }` for host objects with
  attribute access and methods (this is how Buck2's `ctx`, `ctx.actions`, and
  artifacts are surfaced to Starlark: an `object` whose attrs are `builtin`
  functions or nested `object`s)

`type v` dispatches on native type then on `__sk`. Truthiness follows Starlark
(`None`, `False`, `0`, `0.0`, `""`, and empty list/tuple/dict are falsy).
Equality is structural. Everything the interpreter itself constructs is
immutable Nix data; "mutation" is modeled by rebinding (below).

### Evaluator: environment and world threading

The evaluator threads two things through evaluation:

- `env`: lexical scope, an attrset of name to value, with a parent link for
  closures. Statement execution returns an updated `env`.
- `world`: an opaque accumulator the host uses for effects. The interpreter
  never inspects it. Plain Starlark passes `null` and every builtin returns it
  unchanged. Buck2 uses it as the target registry (load phase) or the action
  registry (analysis phase).

Core signatures:

```text
evalExpr  : node -> env -> world -> { value; world; }
evalStmts : nodes -> env -> world -> { env; world; ctrl; }   # ctrl: return/break/continue
callValue : fn -> posArgs -> namedArgs -> world -> { value; world; }
```

Builtins receive `{ pos; named; world; call; }` and return `{ value; world; }`,
where `call` is a callback into `callValue` for higher-order builtins. Pure
builtins ignore `world` and pass it through; effectful Buck2 builtins (e.g.
`ctx.actions.run`) append to it. Because calls thread the *same* `world` in and
out across the new function-local `env`, effects escape function scope while
variables do not, which is exactly analysis semantics (a rule impl's locals are
private, but its registered actions are the output).

### Mutation without a heap

Starlark values are mutable until frozen; Nix has no mutation. Rather than
build a heap with object identity, the evaluator handles the mutation patterns
that occur in build code at the *statement* level: when a statement is a
method call on a simple lvalue whose method mutates in place
(`name.append(x)`, `name.extend(xs)`, `name.add(...)` on `cmd_args`,
`name[k] = v`, `name += x`), it computes the new container and rebinds `name`
in `env`. Augmented assignment and index/attr assignment targets are handled
the same way. This is correct for local-variable mutation (all of
`no_prelude`, and the overwhelming majority of real `.bzl`); it does not model
mutation through an alias (`b = a; b.append(1)` will not change `a`). That
limitation is documented and guarded by the conformance tests; a heap can be
added later if a corpus needs it.

### `load()` and freezing

`load()` is resolved by a caller-supplied function
`loadModule : currentFile -> loadLabel -> moduleGlobals`. The interpreter
calls it, receives the referenced module's frozen global attrset, and binds the
requested symbols. Buck2 supplies a `loadModule` that maps a load label
through the cell map to a file path, evaluates it once, and memoizes. Freezing
is a no-op in our immutable representation beyond snapshotting a module's
globals when evaluation of that module finishes.

## Buck2 layer (`platform/nix/lib/buck2`)

```text
platform/nix/lib/buck2/
  PLAN.md               this file
  README.md             usage docs (mirrors platform/nix/lib/cargo/README.md)
  default.nix           perSystemLib.{buildBuck2Project, buck2Lib} + skylark re-export
  lib/                  pure eval (builtins only, uses ../skylark)
    buckconfig.nix      parse .buckconfig ([cells], simple INI)
    labels.nix          parse @cell//pkg:name, //pkg:name, :name, subtargets; cell resolution
    loader.nix          load() resolver + module cache; evaluate .bzl and BUCK
    globals.nix         Buck2 Starlark globals (rule, provider, attrs, struct, cmd_args, glob, host_info, select, fail, oncall, DefaultInfo, RunInfo, ...)
    attrs.nix           attr type descriptors + coercion (source, dep, toolchain_dep, list, string, bool, default_only, ...)
    actions.nix         action registry + artifact model (declare_output, run, write, download_file, copy, symlink); cmd_args rendering
    analysis.nix        configured-target analysis: build ctx, run impl, collect providers+actions; memoized over the graph
  build/
    lower.nix           action DAG -> Nix derivations (one per action)
    toolchains.nix      local-toolchain command string -> nixpkgs package (clang++/rustc/go/...)
    action-runner.nu    optional in-sandbox driver: render argv, run (nushell)
  tests/
    *.nix               eval unit tests (labels, buckconfig, load, analysis)
    fixtures/           trimmed no_prelude packages for hermetic eval tests
  checks.nix            flake checks (build+run cpp/rust/go targets)
```

### Phase 1: load (source to unconfigured target graph)

1. Parse `.buckconfig` for `[cells]` (name to path, relative to repo root).
   `.buckroot` marks the root cell. The `no_prelude` config is just
   `root = .` and `toolchains = toolchains`.
2. Evaluate each needed `BUCK` file with the skylark interpreter, using a
   `world` that is a *target registry*. Rule values are callable: invoking
   `cpp_binary(name = "main", ...)` in a BUCK file appends
   `{ rule; attrs; label = //pkg:main; }` to `world.targets`. `load()` pulls in
   `.bzl` modules (rule and provider definitions), cached per resolved path.
3. `glob(["*.cpp"])` reads the BUCK file's own package directory with
   `readDir`, filters by the patterns, and returns source-file names. Purely
   an eval-time directory read.

Output: a map from target label to `{ ruleName; rule; attrs; }`, the
unconfigured target graph, as plain Nix data.

### Phase 2: analysis (target graph to action DAG)

Buck2 splits configured-target analysis from the unconfigured graph. The
`no_prelude` example uses no `select()` / platforms, so configuration is a
single default platform and `host_info()` reports the real build host; the
design leaves room for constraint-based configuration but does not implement it
first.

For each target, memoized over the label:

1. Coerce attrs against the rule's `attrs` descriptors:
   - `attrs.string()` / `attrs.bool()`: identity.
   - `attrs.source()`: resolve a path (or `:label`) to a *source artifact*.
   - `attrs.dep()` / `attrs.toolchain_dep()`: resolve the label, analyze that
     target (recursion), and produce a *dependency* value carrying its provider
     list, indexable by provider type (`dep[CxxCompilerInfo]`).
   - `attrs.list(inner)`: map coercion over items.
   - `attrs.default_only(inner)`: a fixed default, not settable by the caller.
2. Build a `ctx` host object: `ctx.attrs.<name>` returns coerced attrs;
   `ctx.label`; `ctx.actions` is a host object whose methods
   (`declare_output`, `run`, `write`, `download_file`, `copy`, `symlink`)
   append to `world.actions` and mint artifacts.
3. Run the rule's `impl(ctx)` through the interpreter. It returns a list of
   providers (`DefaultInfo`, `RunInfo`, user providers). Collect
   `{ providers; actions; }`.

Artifacts are values, not paths: `{ __sk = "artifact"; kind; owner; name; }`
where `kind` is `source` (a `builtins.path` store path), `output` (produced by
an action in this analysis), or `download`. `out.as_output()` wraps an output
artifact to mark, inside a `cmd_args`, which action produces it. `cmd_args`
accumulates parts (strings, artifacts, nested lists) plus formatting options
(`format`, `delimiter`, `relative_to`, `hidden`, `prepend`), rendered to argv
at lowering time.

Output: for the requested targets and their transitive deps, an action DAG:
each action is `{ id; kind; argv | content | {url,sha256}; inputs; outputs; env; }`,
where inputs and outputs are artifacts. Still plain Nix data, still no IFD.

### Phase 3: lowering (action DAG to Nix derivations)

Walk the action DAG bottom-up; build one derivation per action, memoized by
action id, wiring dependencies through Nix store-path interpolation:

- **source artifact** to path: `builtins.path { path = <abs file>; name; }`,
  interpolated into the argv. Content-addressed per file, so touching one
  source only rebuilds actions consuming it.
- **output artifact of action A** to path: `${drvA}/<name>`; action A's
  derivation produces a `$out` directory holding its declared outputs.
- **`out.as_output()`** in the current action to path: `$out/<name>` inside
  that action's own builder.
- **`ctx.actions.run(argv, ...)`**: a `runCommand` (or `stdenv.mkDerivation`)
  with the toolchain packages on `PATH`, `mkdir -p $out`, then the rendered
  argv. The toolchain's `command` string (`clang++`, `rustc`, `go`) is mapped
  to a nixpkgs package by `build/toolchains.nix` so the build is hermetic
  rather than depending on the host's compilers; the map is overridable.
- **`ctx.actions.write(name, content, ...)`**: `writeText` / a small
  `runCommand`; with `allow_args`, also emit the associated macro files.
- **`ctx.actions.download_file(out, url, sha256)`**: `fetchurl { inherit url
  sha256; }`. Pure FOD, so the Go toolchain download needs no IFD and no
  sandbox relaxation.

A target's default output is `DefaultInfo.default_output` lowered to its
producing action's derivation output. `buildBuck2Project` returns that (and
exposes `RunInfo` for `buck2 run`-style execution) as a normal derivation.

## Public API

```nix
# flakelight package definition
packages.hello = { lib, ... }:
  lib.buildBuck2Project {
    src = ./examples/no_prelude;   # contains .buckconfig, BUCK files, sources
    target = "//cpp/hello_world:main";
    # targets = "//...";           # or build a set / everything
    # toolchainPackages = { "clang++" = pkgs.clang; rustc = pkgs.rustc; };
    # cell = "root";               # default root cell
  };
```

`lib.buck2Lib` exposes the pure phases (`buckconfig`, `labels`, `loader`,
`analysis`, ...) and `lib.skylark` exposes the interpreter (`parse`, `eval`,
`exec`) for tests and reuse by other Starlark tools.

## Testing

- **Skylark unit tests** (`platform/nix/lib/skylark/tests/*.nix`): assert-based,
  runnable with `nix eval -f`. Lexer and parser snapshot token/AST shapes;
  evaluator tests cover scalars, collections, functions and closures, control
  flow, comprehensions, mutation-by-rebind, `load()` injection, and truthiness
  or equality edge cases. Seed from the Starlark spec's own examples so the
  interpreter tracks a real conformance target, not just what Buck2 happens to
  use.
- **Buck2 unit tests** (`platform/nix/lib/buck2/tests/*.nix`): label parsing, cell
  resolution, `.buckconfig` parsing, load-phase target graph for the fixture
  packages, and analysis-phase action graphs (assert the argv and
  input/output artifacts for the cpp target without building anything).
- **End-to-end checks** (`platform/nix/lib/buck2/checks.nix`, wired individually into
  `flake.nix`; never `nix flake check`, per repo rule): build
  `//cpp/hello_world:main`, `//cpp/library:library`, `//rust:main`, and later
  `//go:main`, then run the binary and assert its output. Fixtures are a
  committed trimmed copy of `no_prelude` (source files are tiny) so checks do
  not depend on network access to GitHub.
- **Oracle (later):** where `buck2` is available, diff our lowered action argv
  against `buck2 audit` / `buck2 build -v` output for the same targets, the way
  `cargo` diffs resolution against `cargo tree`.

## Risks

- **Interpreter completeness** is the dominant risk (a whole language). Bound
  it by targeting the `no_prelude` surface first, then widening against the
  Starlark spec test suite, each gap a crisp failing test. Non-goal for now:
  the full `buck2-prelude` (thousands of lines, deep provider graphs).
- **Mutation semantics**: the no-heap rebind model is correct for local
  mutation only. Guarded by conformance tests; escalate to a heap only if a
  real corpus needs aliased mutation.
- **`cmd_args` fidelity**: `relative_to`, `format`, `delimiter`, and hidden
  inputs must render argv exactly, or actions get wrong command lines. The Go
  vertical is the stress test; cpp/rust need only the simple path.
- **Toolchain hermeticity**: upstream "local" toolchains mean "whatever is on
  the machine". Substituting nixpkgs compilers changes the toolchain; outputs
  should be behaviorally identical for these examples, and the map is an
  explicit, overridable seam.
- **Eval cost / recursion depth**: char-by-char lexing and tree-walking in Nix
  are slow, but BUCK/.bzl files are small; the cargo and yaml libraries show
  the idiom scales to real inputs. Measure if a large prelude is ever targeted.
- **Configuration/`select()`**: unimplemented initially. Fine for `no_prelude`;
  a real prelude needs constraints and platforms, a later milestone.

## Milestones

- [x] M0: PLAN.md (this file), directory scaffold, flake wiring.
- [x] M1: skylark lexer + parser, unit tests green on the `no_prelude`
      constructs (13 lexer + 35 parser cases, including parsing every real
      `no_prelude` file).
- [x] M2: skylark evaluator + standard builtins, unit tests green (58 cases:
      scalars, collections, functions, control flow, comprehensions,
      mutation-by-rebind, the late-binding closure, `load`).
- [x] M3: buck2 load phase: `.buckconfig`, labels + cell resolution, loader,
      `glob`, rule / provider / attrs / struct / host_info / oncall globals;
      `no_prelude` BUCK files evaluate to the expected unconfigured target
      graph (13 label + 15 load cases).
- [x] M4: buck2 analysis phase: attr coercion, `ctx` / `ctx.actions`, artifact
      and `cmd_args` model, provider indexing; cpp/rust targets produce the
      expected action DAG (10 analysis cases, nothing built).
- [x] M5: lowering + cpp/rust builds. All three non-go `no_prelude` build
      targets build (one derivation per action, no IFD) and are flake checks:
      `//cpp/hello_world:main` (`buck2-build-cpp`, runs, prints "Hello from
      C++!"), `//cpp/library:library` (`buck2-build-cpp-library`, produces a
      `lib.so` exporting `print_hello`), and `//rust:main` (`buck2-build-rust`,
      runs, prints "Hello from Rust!"). Plus the `buildBuck2Project` API and
      README.
- [x] M6: go vertical. `//go:main` builds and runs (`buck2-build-go`,
      "Hello from Go!"). This exercises the full toolchain dance:
      `download_file` -> `fetchurl` (pure, sha256 in the source), `write` of the
      unpack script with `allow_args`, extract, and the `ln -sf` symlink, plus
      `cmd_args` `format` / `delimiter` / `prepend` / `relative_to`. The fix was
      the buck-out-relative artifact model (see Phase 3): artifact references
      render as working-dir-relative paths (strings), so the unpack `write`
      no longer takes a store-path dependency on the output it names, which
      removes the write<->extract cycle. Downloaded prebuilt binaries are made
      runnable with `autoPatchelfHook`. Known cost: each action carries the
      producer's whole tree (the Go dir is copied a few times); a content-
      addressed or symlink-farm staging is the optimization.
- [ ] M7: `//...` target discovery (currently only explicit labels build),
      conformance widening (Starlark spec subset), a minimal `with_prelude`-
      style example, oracle diff where `buck2` is available.

## Benchmark and performance (no_prelude, 2026-07-19)

Against upstream buck2 (`unstable-2026-04-15`) building the same targets with
the same nixpkgs clang/rustc; Go downloaded by the rule for both. buck2 keeps a
warm daemon; nix-buck2 is stateless and re-evaluates on every invocation.

| Scenario | buck2 | nix-buck2 |
|---|---|---|
| Hot / no-op (cpp+rust) | 10 ms | 610 ms |
| Cold (cpp+rust) | 0.88 s | ~3.3 s |
| Incremental, edit cpp | 0.67 s | ~3.6 s |
| Incremental, edit rust | 0.44 s | ~3.3 s |
| Go incremental (warm toolchain) | 0.14 s | ~6.0 s |
| Go cold (incl. ~100 MB download) | 14.1 s | ~14 s |

Where nix-buck2's wall time goes (measured): per-invocation eval ~0.57 s
(~0.27 s nixpkgs import + ~0.30 s the Starlark interpreter + analysis +
lowering); the clang/rustc compile itself (~1.4 s, `#include <iostream>` is
heavy, paid by buck2 too); per-action stdenv/derivation setup (~0.3-0.5 s
each). Go's incremental floor is the cold GOCACHE: every sandboxed `go build`
recompiles the stdlib, whereas buck2 keeps GOCACHE warm in `buck-out`.

### Structural differences

1. **No daemon.** nix-buck2 re-runs load+analysis+lowering on every `nix
   build`; buck2 caches parsed and analyzed state in a persistent daemon (its
   ~10 ms hot rebuild). This is the whole story for hot and a fixed ~0.6 s
   floor on everything.
2. **Per-action derivations.** Sandbox + stdenv setup per action vs in-process
   execution against one `buck-out`.
3. **Cold compiler caches.** GOCACHE and rustc's incremental cache are stateful
   and nondeterministic, irreconcilable with hermetic per-derivation
   sandboxing (the same wall the cargo library hit for rustc incrementality).

nix-buck2's advantages are structural rather than latency: global
content-addressed caching shared across projects and machines (cachix) vs
per-project `buck-out`, hermetic pinned toolchains (buck2 uses whatever is on
`PATH`), and Nix-native builds (the downloaded Go toolchain runs even in the
pure sandbox / on NixOS via autoPatchelf, where a bare download would not).

### Optimizations applied and measured

- **Symlink-farm staging** (`cp -rs`), build directly in `$out`, and
  autoPatchelf only in the action that materializes a download: the ~500 MB Go
  SDK is never copied per consumer. Go incremental 8.3 s -> 6.0 s; cpp/rust
  unchanged (no large deps to stage). A real disk/IO win, neutral on the tiny
  from-source targets.
- **Opt-in `ifdAnalysis`.** Analysis in a derivation keyed on build files +
  file-name structure only, so source edits never re-interpret (verified: a
  source edit rebuilds only the compile action, not analysis). Net slower on
  no_prelude, the fixed eval-time cost of the content-keyed analysis source
  plus the JSON round-trip exceeds the cheap ~0.3 s interpreter. Intended for
  large prelude graphs where interpretation dominates; off by default.

### Remaining levers (not taken)

- load/analysis memoization: negligible on no_prelude, prevents re-analysis
  blowup on real diamond-dependency graphs.
- Warm compiler caches (GOCACHE/rustc) staged from the toolchain derivation:
  would fix Go incremental but fights hermeticity.
- Remote builders + cachix + ca-derivations: scale cold builds out and share
  per-action outputs across the repo/fleet, the design's real payoff over a
  per-project `buck-out`.
- `//...` target discovery (only explicit labels build today).

## Decision log

- 2026-07-19: Two libraries, `platform/nix/lib/skylark` (reusable interpreter) and
  `platform/nix/lib/buck2` (semantics + lowering), sibling to `platform/nix/lib/cargo` and
  `platform/nix/lib/deno`, exposed via `perSystemLib`. Rationale: Starlark outlives
  Buck2 (Bazel and others), and the cargo library already proves the
  pure-eval, per-unit-derivation, no-IFD pattern in this repo.
- 2026-07-19: The build unit is the Buck2 *action* (finer than a target), one
  Nix derivation each, for maximal caching and parallelism.
- 2026-07-19: Effects (target and action registration) are threaded through
  the interpreter as an opaque `world` accumulator, keeping the interpreter
  Buck2-agnostic. Mutation modeled by statement-level rebinding, no heap.
- 2026-07-19: `no_prelude` is the first corpus; cpp/rust before go; the full
  `buck2-prelude` and `select()`/configuration are explicitly deferred.
- 2026-07-19: Local toolchains map their `command` string to nixpkgs packages
  for hermeticity, overridable via `toolchainPackages`; `download_file`
  toolchains (Go) stay faithful through `fetchurl`.
- 2026-07-19: Staging is by symlink (`cp -rs`), and downloaded binaries are
  autoPatchelf'd once in the action that materializes them, so a large
  toolchain is never copied per consumer. Actions build directly in `$out`.
- 2026-07-19: Analysis was factored to a plain JSON-able action graph
  (`lib/analyze.nix` + `lib/serialize.nix`) feeding `build/lower.nix`. An
  opt-in `ifdAnalysis` runs that analysis in a derivation keyed only on build
  files + file-name structure (one IFD), so source edits never re-interpret.
  Measured slower than pure eval on `no_prelude` (the eval-time cost of the
  content-keyed analysis source plus JSON round-trip exceeds the cheap
  interpreter); kept off by default, intended for large prelude graphs. The
  pure path stays the strict no-IFD default.
