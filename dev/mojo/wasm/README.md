# 🔥 mojo-wasm

A reactive UI framework for the browser, written in [Mojo](https://www.modular.com/mojo) and compiled to WebAssembly.

Built from the ground up — signals, virtual DOM, diffing, event handling, and a binary mutation protocol — all running as WASM with a thin TypeScript runtime.

## Features

- **Reactive signals** — fine-grained reactivity with automatic dependency tracking
- **Memo (derived signals)** — cached computed values with automatic dependency re-tracking
- **Virtual DOM** — template-based VNodes with keyed diffing
- **Binary mutation protocol** — efficient Mojo → JS communication via shared memory
- **Automatic template wiring** — templates defined once in Mojo, auto-registered in JS via `RegisterTemplate` mutations
- **Automatic event wiring** — handler IDs flow through the mutation protocol; `EventBridge` dispatches events without manual mapping
- **Event system** — DOM events delegated through WASM with action-based handlers
- **Scoped components** — hierarchical scopes with hooks, context, error boundaries, and suspense boundaries
- **Suspense** — pending state with skeleton fallback and JS-triggered resolve; nested boundaries with independent inner/outer lifecycle
- **Effects in apps** — reactive side effects with derived state (effect drain-and-run pattern), signal → memo → effect → signal chains
- **Memo type expansion** — `MemoBool` and `MemoString` achieve type-parity with signals (`SignalI32`/`SignalBool`/`SignalString` ↔ `MemoI32`/`MemoBool`/`MemoString`); mixed-type memo chains with ordered recomputation
- **Recursive memo propagation** — `write_signal` automatically propagates dirtiness through memo → memo chains to arbitrary depth via worklist-based traversal; diamond dependencies handled correctly; scope context tagging prevents namespace collisions
- **Equality-gated memo propagation** — memo `end_compute` compares old vs new value before writing; if the value is unchanged, the output signal is NOT written, downstream memos remain value-stable, and `settle_scopes()` removes eagerly-dirtied scopes — skipping unnecessary re-renders and DOM diffs entirely
- **Batch signal writes** — `begin_batch()` / `end_batch()` group multiple signal writes into a single propagation pass; values are stored immediately (reads see new values) but subscriber scanning is deferred until the outermost `end_batch()`, which runs a combined propagation with a shared worklist — eliminating redundant intermediate dirty-marking
- **Ergonomic DSL** — `el_div`, `el_button`, `dyn_text` tag helpers with `to_template()` conversion
- **AppShell abstraction** — single struct bundling runtime, store, allocator, and scheduler
- **ComponentContext** — ergonomic Dioxus-style API with `use_signal()`, `setup_view()`, inline events, auto-numbered `dyn_text()`
- **Three working apps** — counter, todo list, and js-framework-benchmark (all using ComponentContext)
- **ItemBuilder + HandlerAction** — ergonomic per-item building and event dispatch for keyed lists (`begin_item()`, `add_custom_event()`, `get_action()`)
- **String event dispatch** — `ACTION_SIGNAL_SET_STRING` handlers pipe string values from DOM events directly into `SignalString` signals; JS EventBridge extracts `event.target.value` → `writeStringStruct()` → WASM `dispatch_event_with_string` with automatic fallback to numeric/default dispatch
- **Two-way input binding** — Dioxus-style `oninput_set_string(signal)` + `bind_value(signal)` DSL helpers for inline string event handlers and auto-populated `value` attributes; `RenderBuilder.build()` reads `SignalString` at render time
- **Error boundaries** — scope-level error catching with fallback UI and recovery; `use_error_boundary()` marks a scope as a boundary, `report_error()` propagates errors up the parent chain, `has_error()` / `clear_error()` drive flush-time content switching between normal and fallback children
- **4,413 tests** — 1,323 Mojo (52 modules via wasmtime) + 3,090 JS (29 suites via Deno), all passing

## How it works

The build pipeline compiles Mojo source code to WASM through LLVM:

```txt
Mojo → LLVM IR → WASM Object → WASM Binary
```

1. `mojo build` emits LLVM IR as a shared library
2. `llc` compiles the IR to a wasm64-wasi object file
3. `wasm-ld` links the object into a `.wasm` binary
4. `wasmtime` pre-compiles to `.cwasm` for fast instantiation (~70ms)

At runtime, the TypeScript side (`runtime/`) instantiates the WASM module and provides:

- **Memory management** — a size-class free-list allocator for `KGEN_CompilerRT_AlignedAlloc`/`AlignedFree` with safe memory reuse
- **I/O** — `write` routed to `console.log`/`console.error` for stdout/stderr
- **Math builtins** — `fma`, `fmin`, `fmax` and their float variants
- **Libc stubs** — `dup`, `fdopen`, `fflush`, `fclose`, `__cxa_atexit`
- **String ABI** — helpers for reading/writing Mojo `String` structs (including SSO)
- **DOM interpreter** — a stack machine that applies binary mutations to the real DOM
- **Event bridge** — captures DOM events and dispatches them to WASM handlers

## Architecture

```txt
┌─────────────────────────────────────────────────────────┐
│  Browser                                                │
│                                                         │
│  ┌──────────────┐    mutations    ┌──────────────────┐  │
│  │  DOM          │◄──────────────│  JS Interpreter    │  │
│  │  (real nodes) │               │  (stack machine)   │  │
│  └──────┬───────┘               └────────┬───────────┘  │
│         │ events                         ▲ binary buf    │
│         ▼                                │               │
│  ┌──────────────┐               ┌────────┴───────────┐  │
│  │  Event Bridge │──dispatch───►│  WASM Module        │  │
│  │  (JS)         │              │  (Mojo)             │  │
│  └──────────────┘               │                     │  │
│                                 │  ┌─ Signals ──────┐ │  │
│                                 │  │  Memos          │ │  │
│                                 │  │  Scopes         │ │  │
│                                 │  │  VNode Store    │ │  │
│                                 │  │  Diff Engine    │ │  │
│                                 │  │  Mutation Writer│ │  │
│                                 │  └────────────────┘ │  │
│                                 └─────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

## Project structure

```txt
mojo-wasm/
├── src/
│   ├── main.mojo                 # @export wrappers (WASM entry point, ~6,730 lines)
│   ├── apps/                     # Demo/test app modules (Phase 40 extraction)
│   │   ├── batch_demo.mojo       # BatchDemoApp (Phase 38.2)
│   │   ├── child_context_test.mojo # ChildContextTestApp (Phase 31.2)
│   │   ├── child_counter.mojo    # ChildCounterApp (Phase 29)
│   │   ├── context_test.mojo     # ContextTestApp (Phase 31.1)
│   │   ├── data_loader.mojo      # DataLoaderApp + children (Phase 33.2)
│   │   ├── effect_demo.mojo      # EffectDemoApp (Phase 34.1)
│   │   ├── effect_memo.mojo      # EffectMemoApp (Phase 34.2)
│   │   ├── equality_demo.mojo    # EqualityDemoApp (Phase 37.3)
│   │   ├── error_nest.mojo       # ErrorNestApp + children (Phase 32.3)
│   │   ├── memo_chain.mojo       # MemoChainApp (Phase 35.3)
│   │   ├── memo_form.mojo        # MemoFormApp (Phase 35.2)
│   │   ├── props_counter.mojo    # PropsCounterApp + CounterDisplay (Phase 31.3)
│   │   ├── safe_counter.mojo     # SafeCounterApp + children (Phase 32.2)
│   │   ├── suspense_nest.mojo    # SuspenseNestApp + children (Phase 33.3)
│   │   └── theme_counter.mojo    # ThemeCounterApp + children (Phase 31.4)
│   ├── arena/
│   │   └── element_id.mojo       # ElementId type and allocator
│   ├── bridge/
│   │   └── protocol.mojo         # Opcode constants, MutationWriter
│   ├── component/                # Reusable app infrastructure
│   │   ├── app_shell.mojo        # AppShell struct (runtime + store + allocator + scheduler)
│   │   ├── context.mojo          # ComponentContext — ergonomic API, RenderBuilder, view tree processing
│   │   └── lifecycle.mojo        # mount, diff, finalize helpers; FragmentSlot + flush_fragment
│   ├── events/
│   │   └── registry.mojo         # Handler registry and dispatch
│   ├── mutations/
│   │   ├── create.mojo           # CreateEngine (initial mount)
│   │   └── diff.mojo             # DiffEngine (keyed reconciliation)
│   ├── scheduler/
│   │   └── scheduler.mojo        # Height-ordered dirty scope queue with deduplication
│   ├── scope/
│   │   ├── scope.mojo            # ScopeState, hooks, context, error/suspense
│   │   └── arena.mojo            # ScopeArena (slab allocator)
│   ├── signals/
│   │   ├── memo.mojo             # MemoEntry, MemoStore (slab allocator for derived signals)
│   │   └── runtime.mojo          # Reactive runtime, signal store, context tracking
│   └── vdom/
│       ├── builder.mojo          # TemplateBuilder API (manual template construction)
│       ├── dsl.mojo              # Ergonomic DSL: Node union, el_* helpers, to_template()
│       ├── dsl_tests.mojo        # Self-contained DSL test functions (19 tests, extracted from main.mojo)
│       ├── registry.mojo         # Template storage and lookup
│       ├── tags.mojo             # HTML tag constants (TAG_DIV, TAG_SPAN, ...)
│       ├── template.mojo         # Template, TemplateNode (static structure)
│       └── vnode.mojo            # VNode, DynamicNode, AttributeValue, VNodeBuilder
├── runtime/                      # TypeScript runtime (browser)
│   ├── mod.ts                    # Entry point — instantiate WASM
│   ├── types.ts                  # WasmExports interface
│   ├── memory.ts                 # Free-list allocator, WASM memory
│   ├── env.ts                    # Environment imports (I/O, math, libc)
│   ├── strings.ts                # Mojo String ABI helpers (SSO)
│   ├── protocol.ts               # Mutation opcodes (shared with Mojo)
│   ├── interpreter.ts            # DOM stack machine
│   ├── templates.ts              # Template cache (DocumentFragment pool)
│   ├── events.ts                 # Event delegation bridge
│   ├── tags.ts                   # HTML tag name mapping
│   └── app.ts                    # App lifecycle helpers
├── examples/
│   ├── counter/                  # Counter app — simplest example
│   │   ├── counter.mojo          # Mojo app (inline events via setup_view)
│   │   ├── index.html            # Browser entry point
│   │   └── main.js               # JS harness
│   ├── todo/                     # Todo list app
│   │   ├── todo.mojo             # Mojo app (keyed lists, multi-template, custom handlers, SignalString)
│   │   ├── index.html            # Browser entry point
│   │   └── main.js               # JS harness
│   ├── bench/                    # js-framework-benchmark
│   │   ├── bench.mojo            # Mojo app (keyed lists, 7 operations)
│   │   ├── index.html            # Browser entry point
│   │   └── main.js               # JS harness
│   └── lib/                      # Shared JS runtime for examples
│       ├── boot.js               # Re-exports + convenience helpers
│       ├── env.js                # WASM memory management, loadWasm()
│       ├── interpreter.js        # DOM Interpreter class
│       ├── protocol.js           # Op constants + MutationReader
│       └── strings.js            # Mojo String ABI writeStringStruct()
├── test/                         # Mojo tests (52 modules, 1,323 tests via wasmtime)
│   ├── wasm_harness.mojo         # WasmInstance harness using mojo-wasmtime FFI
│   ├── test_signals.mojo         # Reactive signals
│   ├── test_scopes.mojo          # Scope arena and hooks
│   ├── test_templates.mojo       # Template builder, registry, VNode store
│   ├── test_mutations.mojo       # Create/diff engines
│   ├── test_events.mojo          # Event handler registry
│   ├── test_protocol.mojo        # Binary mutation encoding
│   ├── test_dsl.mojo             # Ergonomic DSL builder
│   ├── test_component.mojo       # AppShell and lifecycle
│   ├── test_memo.mojo            # Memo store, runtime API, hooks, propagation
│   ├── test_scheduler.mojo       # Scheduler ordering and dedup
│   └── ...                       # + arithmetic, strings, boundaries, etc.
├── test-js/                      # JS runtime integration tests (3,090 tests via Deno)
│   ├── harness.ts                # Shared WASM loading and test helpers
│   ├── counter.test.ts           # Full counter app lifecycle with DOM
│   ├── todo.test.ts              # Todo app: add, remove, toggle, clear
│   ├── bench.test.ts             # Benchmark operations + timing
│   ├── safe_counter.test.ts      # SafeCounterApp error boundary crash/retry lifecycle
│   ├── error_nest.test.ts        # ErrorNestApp nested boundary inner/outer crash/retry
│   ├── dsl.test.ts               # DSL builder + VNodeBuilder round-trip
│   ├── interpreter.test.ts       # DOM interpreter + template cache
│   ├── memo.test.ts              # Memo lifecycle, dirty tracking, propagation
│   ├── mutations.test.ts         # JS-side MutationReader + memory
│   ├── phase8.test.ts            # Context, error boundaries, suspense
│   └── protocol.test.ts          # Binary protocol parsing
├── scripts/
│   ├── build_test_binaries.sh    # Parallel incremental mojo build for test modules
│   ├── run_test_binaries.sh      # Parallel test binary execution with reporting
│   └── precompile.mojo           # .wasm → .cwasm via wasmtime AOT
├── justfile                      # Build and test commands
├── default.nix                   # Nix dev shell
└── CHANGELOG.md                  # Development history (Phases 0–40)
```

## Mojo version

Built on **Mojo 0.25.x**. Migration to **0.26.1** is tracked in
[MIGRATION_PLAN.md](MIGRATION_PLAN.md).

### Breaking changes (0.26.1)

| ID | Change | Impact | Scope |
|---|---|---|---|
| **B1** | `List[T](a, b, c)` variadic initializer removed — use list literals `[a, b, c]` | Widespread | ~50–80 call sites |
| **B2** | `alias` keyword deprecated — migrate to `comptime` | Pervasive | ~150+ declarations |
| **B3** | `ImplicitlyBoolable` trait removed — `if ptr:` and `if count:` need explicit comparisons | Moderate | ~20–40 sites |
| **B4** | `UInt` is now `Scalar[DType.uint]` — no implicit `Int` ↔ `UInt` conversion | Low | Audit needed |
| **B5** | `Iterator` trait overhaul — `__has_next__()` removed, `__next__()` raises `StopIteration` | None | No custom iterators |
| **B6** | `Error` no longer `Boolable` or `Defaultable` | Low | Grep for `Error()` |
| **B7** | `InlineArray` no longer `ImplicitlyCopyable` | Low | Check implicit copies |
| **B8** | `Writer.write_bytes()` → `write_string()`, `String.__init__(bytes:)` → `unsafe_from_utf8` | Low | Custom `Writer` impls |

### New features to adopt (0.26.1)

| ID | Feature | Opportunity |
|---|---|---|
| **F1** | **Typed errors** (`raises CustomError`) — zero-overhead on WASM | `EventError`, `DiffError`, `MutationError` types |
| **F2** | **String UTF-8 safety** — `from_utf8`, `from_utf8_lossy`, `unsafe_from_utf8` constructors | Explicit safety in WASM ↔ JS string bridge |
| **F3** | **Trait default impls** — `Writable`, `Equatable`, `Hashable` auto-derive from fields | Zero-boilerplate conformance for core structs |
| **F4** | `Copyable` now refines `Movable` — remove redundant `Movable` declarations | Minor cleanup |
| **F5** | `comptime(x)` expression — force compile-time evaluation inline | Cleaner template/config code |
| **F6** | `-Xlinker` flag — pass options to linker from `mojo build` | Potentially simplify wasm-ld pipeline |
| **F7** | `-Werror` flag — treat warnings as errors | Add to CI after migration |
| **F8** | `conforms_to()` + `trait_downcast()` (experimental) — static trait dispatch | Stepping stone to generic `Signal[T]` |
| **F9** | Expanded reflection — `struct_field_count`, `struct_field_names`, `offset_of` | Auto-generated encoders, debug formatters |
| **F10** | `Never` type — functions guaranteed not to return | Annotate `abort()` wrappers |

### Migration order

1. **B3** — fix implicit bool conversions (hard compile errors)
2. **B1** — update `List[T](...)` → list literals (most widespread)
3. **B4–B8** — minor breaks (`UInt`, `Error`, `InlineArray`, `Writer`)
4. **B2** — bulk `alias` → `comptime` find-replace (last, touches every file)
5. **F1–F3** — adopt typed errors, UTF-8 constructors, trait defaults incrementally
6. **F7** — enable `-Werror` in CI after all warnings resolved

Verification: `just test-all` (996 Mojo + 1,222 JS tests) + manual check of all three example apps.

## Known limitations

**`@export` only works in the main module.** Mojo's compiler aggressively eliminates dead code before LLVM IR generation. An `@export` decorator on a function in a submodule (e.g., `poc/arithmetic.mojo`) does **not** prevent it from being removed — the function must be called from `main.mojo` to survive. Importing a submodule function without calling it is also insufficient as a DCE anchor. This is why `main.mojo` contains ~419 thin `@export` wrappers that forward to submodule implementations: it is the only reliable way to guarantee WASM export visibility with the current Mojo toolchain. See [CHANGELOG.md § M10.22](CHANGELOG.md#phase-10--modularization--next-steps-) for the full investigation.

**Handler lifecycle is scope-scoped.** Event handlers registered via `runtime.register_handler()` are automatically cleaned up when their owning scope is destroyed. For dynamic lists (todo items, benchmark rows), each item gets its own child scope. Rebuilding a list destroys old child scopes — which triggers `remove_for_scope` cleanup in the `HandlerRegistry` — before creating new ones. Without this pattern, handler IDs leak: after 100 add/remove cycles on a 10-item list, the registry would accumulate ~2,000 stale entries. The child-scope-per-item pattern ensures handler count stays proportional to visible items.

## Reactive model

The framework follows the same reactive model as [Dioxus](https://dioxuslabs.com/):

1. **Signals** hold state. Reading a signal inside a scope subscribes that scope.
2. **Memos** (derived signals) cache computed values. A memo has its own reactive context: it auto-tracks which signals it reads during computation, caches the result, and marks subscribing scopes dirty when its inputs change. Memos are lazy — they only recompute when read while dirty. Dependency re-tracking on recompute means memos automatically adapt to conditional reads.
3. **Writing** to a signal marks all subscribing scopes *and* all subscribing memos as dirty. Dirty memos propagate dirtiness to their own subscribers.
4. **Equality gating** — when a memo recomputes to the same value, its output signal is NOT written. Downstream memos remain value-stable, and `settle_scopes()` removes eagerly-dirtied scopes whose subscribed signals all have unchanged values — skipping unnecessary re-renders.
5. **Batching** — `begin_batch()` / `end_batch()` defer propagation until all writes complete; the outermost `end_batch()` runs a single combined propagation pass with a shared worklist, deduplicating memo dirty-marking across multiple source signals.
6. **Dirty scopes** are collected into the **Scheduler** (height-ordered, deduplicated).
7. Scopes are re-rendered in parent-before-child order, producing new VNode trees.
8. The **diff engine** compares old and new VNode trees (with keyed reconciliation).
9. Mutations are written to a **binary buffer** in shared WASM memory.
10. The JS **interpreter** reads the buffer and applies DOM operations.

```txt
Signal write → memo dirty → scope dirty → memo recompute → equality gate → settle scopes → scheduler → re-render → diff → mutations → DOM update
(with optional batching: begin_batch → N × signal write → end_batch → single combined propagation)
```

## Binary mutation protocol

Mojo and JS communicate through a binary protocol in shared memory. Each mutation is a compact byte sequence:

| Opcode | Name | Payload |
|--------|------|---------|
| `0x00` | End | — |
| `0x01` | AppendChildren | id: u32, count: u32 |
| `0x02` | AssignId | path: u8[], id: u32 |
| `0x03` | CreatePlaceholder | id: u32 |
| `0x04` | CreateTextNode | id: u32, text: str |
| `0x05` | LoadTemplate | tmpl: u32, index: u32, id: u32 |
| `0x06` | ReplaceWith | id: u32, count: u32 |
| `0x07` | ReplacePlaceholder | path: u8[], count: u32 |
| `0x08` | InsertAfter | id: u32, count: u32 |
| `0x09` | InsertBefore | id: u32, count: u32 |
| `0x0a` | SetAttribute | id: u32, ns: u8, name: str, value: str |
| `0x0b` | SetText | id: u32, text: str |
| `0x0c` | NewEventListener | id: u32, handler_id: u32, name: str |
| `0x0d` | RemoveEventListener | id: u32, name: str |
| `0x0e` | Remove | id: u32 |
| `0x0f` | PushRoot | id: u32 |
| `0x10` | RegisterTemplate | tmpl_id: u32, name: str, nodes[], attrs[], roots[] |

## Prerequisites

Enter the dev shell (requires [Nix](https://nixos.org/)):

```sh
nix develop .#mojo-wasm
```

This provides `just`, `mojo`, `deno`, `llc`, `wasm-ld`, and `wasmtime`.

## Usage

Build the WASM binary:

```sh
just build
```

Run the Mojo tests (precompiled binaries, ~10s):

```sh
just test
```

Run the JS runtime integration tests:

```sh
just test-js
```

Run all tests (Mojo + JS):

```sh
just test-all
```

Serve the examples locally:

```sh
just serve
```

Then open:

- <http://localhost:4507/examples/counter/> — Counter app
- <http://localhost:4507/examples/todo/> — Todo list
- <http://localhost:4507/examples/bench/> — Benchmark

## Test infrastructure

Test execution uses **precompiled binaries** for fast iteration (~10s vs ~5–6 minutes with `mojo test`):

1. Each `test/test_*.mojo` file has an inline `fn main()` that creates one shared `WasmInstance` and calls all test functions sequentially.
2. `scripts/build_test_binaries.sh` compiles each module into a standalone binary in `build/test-bin/` with incremental timestamp checks (parallel, up to `nproc` jobs).
3. `scripts/run_test_binaries.sh` launches all binaries concurrently and reports pass/fail with timing.

| Scenario | Time |
|---|---|
| Cold build (all 26 binaries) | ~92s |
| Incremental build (nothing changed) | <0.1s |
| Run precompiled binaries | ~10s |
| Run single module (`just test-run signals`) | ~100ms |
| Full cycle (`just test`, no code change) | ~11s |
| Full cycle + JS tests (`just test-all`) | ~22s |

Filter by module name (substring match) to target specific tests:

```sh
just test signals             # build + run only test_signals (~100ms)
just test signals mut         # build + run test_signals + test_mutations
just test-run -v dsl          # verbose output for test_dsl only
```

Adding a new test:

1. Write `def test_foo(w: UnsafePointer[WasmInstance])` in the appropriate `test/test_*.mojo` file.
2. Add `test_foo(w)` to the `fn main()` at the bottom of the same file.
3. Run `just test`.

## Test results

4,413 tests across 52 Mojo modules and 29 JS test suites:

- **Signals & reactivity** — create, read, write, subscribe, dirty tracking, context
- **Scopes** — lifecycle, hooks, context propagation, error boundaries, suspense
- **Scheduler** — height-ordered processing, deduplication, multi-scope ordering
- **Templates** — builder, DSL, registry, node queries
- **VNodes** — template refs, text, placeholders, fragments, keyed children
- **Mutations** — create engine, diff engine, binary protocol round-trip
- **Events** — handler registry, dispatch, signal actions, string dispatch (Phase 20), EventBridge string extraction, dispatch fallback chain, WASM integration
- **DSL** — Node union, tag helpers, to_template conversion, VNodeBuilder, `oninput_set_string` / `onchange_set_string` node fields (M20.3), `bind_value` / `bind_attr` node fields and element integration (M20.4), two-way binding element + template conversion
- **Memo** — create/destroy, dirty tracking, auto-track, propagation chain, diamond dependency, dependency re-tracking, cache hit, version bumps, cleanup, hooks
- **Component** — AppShell lifecycle, mount/diff/finalize helpers, FragmentSlot, shell memo helpers, ItemBuilder handler map
- **Counter app** — init, mount, click, flush, DOM verification, memo (doubled count) demo
- **Todo app** — add, remove, toggle, clear, keyed list transitions
- **Benchmark** — create/append/update/swap/select/remove/clear 1000 rows, full DOM integration
- **Context apps** — ComponentContext provide/consume surface, ChildComponentContext test harness, self-rendering child with props, shared context + cross-component communication (ThemeCounterApp)
- **Safe counter app** — error boundary with crash/retry lifecycle, normal↔fallback child switching, count signal preservation across crash/recovery cycles, DOM verification of fallback UI
- **Error nest app** — nested error boundaries with independent crash/retry, inner crash caught by inner boundary (only inner slot swaps), outer crash replaces entire inner tree, mixed crash/retry sequences, full recovery validation
- **Data loader app** — suspense with load/resolve lifecycle, load button sets pending → skeleton shown, JS-triggered resolve → content shown with data, reload cycles, multiple load/resolve cycles, DOM verification
- **Suspense nest app** — nested suspense boundaries with independent inner/outer load/resolve, inner load shows inner skeleton (outer unaffected), outer load shows outer skeleton (hides inner tree), outer resolve reveals persisted inner pending state, mixed load/resolve sequences, full recovery validation
- **Effect demo app** — effect-in-flush pattern with count signal and derived state (doubled, parity), effect drain-and-run lifecycle, effect starts pending → runs on rebuild, increment marks effect pending → flush runs effect → derived state updated, re-subscription each run, rapid 20 increments, heapStats bounded, DOM verification
- **Effect memo app** — signal → memo → effect → signal chain, input signal feeds tripled memo (input × 3), effect reads memo output to derive label ("small"/"big" threshold at tripled ≥ 10), memo recomputed before effects, threshold transition exact (3→small, 4→big), derived state chain consistent, rapid 20 increments, heapStats bounded, DOM verification
- **Memo bool** — MemoBool create/destroy, peek/read, dirty tracking, begin_compute/end_compute, auto-track subscription, hooks integration
- **Memo string** — MemoString create/destroy, peek/read, dirty tracking, begin_compute/end_compute with StringStore, version tracking, lifecycle cleanup, hooks integration
- **Memo form app** — form validation with SignalString input → MemoBool (is_valid) → MemoString (status), memo recomputation order (is_valid before status), two-way input binding (bind_value + oninput_set_string), dirty/clean tracking, derived state consistency, rapid 20 inputs, DOM verification
- **Memo chain app** — mixed-type memo chain (SignalI32 → MemoI32 → MemoBool → MemoString), ordered recomputation, threshold boundary exact (input=5 → is_big flips), chain propagation correctness, Phase 36 independent dirty verification, rapid 20 increments, heapStats bounded, DOM verification
- **Memo propagation** — recursive memo → memo dirty propagation via worklist (Phase 36): 2/3/4-level chains, diamond dependencies (2-input and deep), scope and effect subscribers at end of chains, already-dirty skip (cycle guard), recompute clears dirty, recompute order matters, independent signal writes, re-subscription after recompute, destroyed memo safety, mixed types (I32 → Bool → String), string/bool memos in various positions, no-subscriber memo, memo + scope and memo + effect mixed subscribers, single-memo regression
- **Memo equality** — equality-gated memo propagation (Phase 37): I32/Bool/String value-stable detection, value_changed flag tracking, _changed_signals accumulator, skip-if-unchanged in end_compute, chain cascades (clamped → label both stable), diamond dependencies with mixed stable/changed, regression cases
- **Equality demo app** — clamped + threshold memo chain (SignalI32 → MemoI32(clamp) → MemoString(label)), within-range clamped changes with label stable, threshold crossing (label changes), clamped stabilization above max (zero-byte flush), consecutive stable flushes, full cycle round-trip (0→12→0), scope settling when chain is value-stable, handle_event marks dirty, memo count verification, destroy safety, JS integration tests (DOM structure, clampedChanged/labelChanged queries, flush returns 0 when stable, dirty state after event)
- **Scope settle** — dedicated runtime-level settle_scopes() unit tests: stable memo removes scope, changed memo keeps scope, mixed scopes, direct source signal subscription, scope subscribing to both stable memo and changed signal, no dirty scopes (no crash), all stable (both removed), no changed signals (all removed), 3-level chain cascade all stable, chain partial (A changed, B stable → scope removed), chain fully changed, diamond dependency, effect not affected by settle, idempotent settle, no-memos scenario
- **Batch signal writes** — runtime-level begin_batch/end_batch unit tests (Phase 38): single/multi signal batches, deferred propagation (memo NOT dirty during batch), nested batches (depth 2 and 3), string signals, mixed types, key deduplication, effect pending after end_batch, shared worklist (diamond into one memo), chain propagation, settle after batch, non-batch regression, end-without-begin safety, is_batching flag, large batch (20 signals)
- **Batch demo app** — multi-field form with batched writes (Phase 38): two SignalString fields (first_name, last_name) → MemoString (full_name), SignalI32 (write_count), set_names/reset via begin_batch/end_batch, memo dirty/stable detection, write_count accumulation, rapid 10 sets, DOM verification, independent instances, JS integration (DOM structure, set/reset cycles, batching flag, fullNameChanged query)
- **Memory** — allocation cycles, bounded growth, rapid write stability, free-list reuse, double-free protection, WASM-integrated reuse (text/attr/fragment/template diffs with reuse enabled)
- **Arithmetic/strings** — original PoC interop regression suite

## Ergonomic API

All apps use `ComponentContext` for Dioxus-style ergonomics — constructor-based setup,
`use_signal()` with operator overloading, inline event handlers, auto-numbered
dynamic text slots, and multi-arg `el_*` overloads that eliminate list wrappers:

```mojo
# Dioxus (Rust):
#     fn App() -> Element {
#         let mut count = use_signal(|| 0);
#         rsx! {
#             h1 { "High-Five counter: {count}" }
#             button { onclick: move |_| count += 1, "Up high!" }
#             button { onclick: move |_| count -= 1, "Down low!" }
#         }
#     }

# Mojo equivalent:
struct CounterApp:
    var ctx: ComponentContext
    var count: SignalI32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.setup_view(
            el_div(
                el_h1(dyn_text()),
                el_button(text("Up high!"), onclick_add(self.count, 1)),
                el_button(text("Down low!"), onclick_sub(self.count, 1)),
            ),
            String("counter"),
        )

    fn render(mut self) -> UInt32:
        var vb = self.ctx.render_builder()
        vb.add_dyn_text("High-Five counter: " + String(self.count.peek()))
        return vb.build()
```

> **0.26.1 note:** Code examples below use `alias` and `List[T](...)` syntax from
> Mojo 0.25.x. After migration, `alias` becomes `comptime` and `List[T](a, b, c)`
> becomes `[a, b, c]` with typed list literals. See [MIGRATION_PLAN.md](MIGRATION_PLAN.md).

Multi-template apps (todo, bench) use `KeyedList` with `ItemBuilder` for ergonomic
per-item building, `HandlerAction` for event dispatch, and Phase 18 conditional
helpers (`add_class_if`, `text_when`) to eliminate if/else boilerplate:

```mojo
# Keyed list pattern (todo, bench) — Phase 17 + 18 ergonomics:
alias TODO_ACTION_TOGGLE: UInt8 = 1  # becomes `comptime` in 0.26.1
alias TODO_ACTION_REMOVE: UInt8 = 2  # becomes `comptime` in 0.26.1

struct TodoApp:
    var ctx: ComponentContext
    var list_version: SignalI32
    var items: KeyedList  # bundles template_id + FragmentSlot + scope_ids + handler_map

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.list_version = self.ctx.use_signal(0)
        self.ctx.end_setup()
        self.ctx.register_template(
            el_div(
                el_input(attr("type", "text"), attr("placeholder", "...")),
                el_button(text("Add"), dyn_attr(0)),
                el_ul(dyn_node(0)),
            ),
            String("todo-app"),
        )
        self.items = KeyedList(self.ctx.register_extra_template(
            el_li(
                dyn_attr(2),
                el_span(dyn_text(0)),
                el_button(text("✓"), dyn_attr(0)),
                el_button(text("✕"), dyn_attr(1)),
            ),
            String("todo-item"),
        ))

    fn build_item(mut self, item: TodoItem) -> UInt32:
        var ib = self.items.begin_item(String(item.id), self.ctx)
        # text_when() replaces 4-line if/else for conditional text
        ib.add_dyn_text(text_when(item.completed, "✓ " + item.text, item.text))
        ib.add_custom_event(String("click"), TODO_ACTION_TOGGLE, item.id)
        ib.add_custom_event(String("click"), TODO_ACTION_REMOVE, item.id)
        # add_class_if() replaces 4-line if/else for conditional class
        ib.add_class_if(item.completed, String("completed"))
        return ib.index()

    fn build_items(mut self) -> UInt32:
        var frag = self.items.begin_rebuild(self.ctx)
        for i in range(len(self.data)):
            var idx = self.build_item(self.data[i])
            self.items.push_child(self.ctx, frag, idx)
        return frag

    fn handle_event(mut self, handler_id: UInt32) -> Bool:
        var action = self.items.get_action(handler_id)
        if action.found:
            if action.tag == TODO_ACTION_TOGGLE:
                self.toggle_item(action.data)
            elif action.tag == TODO_ACTION_REMOVE:
                self.remove_item(action.data)
            return True
        return False
```

Phase 18 also adds `SignalBool` for ergonomic boolean signals and standalone
conditional helpers (`class_if`, `class_when`, `text_when`) usable anywhere:

```mojo
# SignalBool — proper boolean API over Int32 signals:
var visible = ctx.use_signal_bool(True)
visible.toggle()            # True ↔ False
if visible.get(): ...       # read without subscribing
visible.set(False)          # write (marks subscribers dirty)

# Conditional helpers — eliminate if/else boilerplate:
var cls = class_if(is_active, String("active"))           # "active" or ""
var cls = class_when(is_open, String("open"), String("closed"))  # either/or
var txt = text_when(done, String("✓ Done"), item.text)    # conditional text
```

Phase 19 adds `SignalString` for reactive string signals. Unlike `SignalI32`
and `SignalBool` which use the type-erased `SignalStore` (memcpy-based,
safe only for fixed-size value types), `SignalString` stores strings in a
separate `StringStore` (safe for heap types) and uses a companion Int32
"version signal" for subscriber tracking:

```mojo
# SignalString — reactive string signal with proper String API:
var name = ctx.use_signal_string(String("hello"))
var v = name.get()              # read without subscribing
var v = name.read()             # read and subscribe context
name.set(String("world"))       # write (marks subscribers dirty)
if name.is_empty(): ...         # convenience check
var display = String("Hi, ") + String(name) + String("!")  # interpolation

# Use with RenderBuilder or ItemBuilder:
var vb = ctx.render_builder()
vb.add_dyn_text_signal(name)    # reads name.get() and adds as dyn text
var idx = vb.build()

# Multiple signal types in one component:
var count = ctx.use_signal(0)
var label = ctx.use_signal_string(String("Count: 0"))
count += 1
label.set(String("Count: ") + String(count.peek()))
```

Phase 20 (M20.3 + M20.4) adds Dioxus-style two-way input binding via inline
DSL helpers. `oninput_set_string(signal)` writes the input's string value into
a `SignalString` on every keystroke; `bind_value(signal)` auto-populates the
`value` attribute at render time by reading the signal. Combined, they give
full two-way binding without any manual handler registration or attribute
management:

```mojo
# Dioxus (Rust):
#     input { value: "{text}", oninput: move |e| text.set(e.value()) }

# Mojo equivalent — two-way input binding (Phase 20):
struct SearchApp:
    var ctx: ComponentContext
    var query: SignalString

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.query = self.ctx.use_signal_string(String(""))
        self.ctx.setup_view(
            el_div(
                el_input(
                    attr(String("type"), String("text")),
                    attr(String("placeholder"), String("Search...")),
                    bind_value(self.query),              # value attr ← signal
                    oninput_set_string(self.query),       # signal ← input event
                ),
                el_p(dyn_text()),                        # display current value
            ),
            String("search"),
        )

    fn render(mut self) -> UInt32:
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(String("You typed: ") + String(self.query.peek()))
        return vb.build()  # auto-adds value attr + event handler

# Also available:
#   onchange_set_string(signal)   — fires on "change" instead of "input"
#   bind_attr("placeholder", sig) — bind any attribute, not just "value"
```

Phase 32 adds error boundaries — scope-level error catching with fallback UI
and recovery. `use_error_boundary()` marks a scope as a boundary;
`report_error()` propagates errors up the parent chain; `has_error()` /
`clear_error()` drive flush-time content switching between normal and fallback
children:

```mojo
# Dioxus (Rust):
#     fn App() -> Element {
#         rsx! {
#             ErrorBoundary {
#                 fallback: |err| rsx! { p { "Error: {err}" } button { onclick: |_| err.clear(), "Retry" } },
#                 ChildComponent {}
#             }
#         }
#     }

# Mojo equivalent — error boundary pattern (Phase 32):
struct SafeCounterApp:
    var ctx: ComponentContext
    var count: SignalI32
    var normal: SCNormalChild            # normal content child
    var fallback: SCFallbackChild        # fallback UI child
    var crash_handler: UInt32
    var retry_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.use_error_boundary()    # mark root as error boundary
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Safe Counter"))),
                el_button(dsl_text(String("+ 1")), onclick_add(self.count, 1)),
                el_button(dsl_text(String("Crash")), onclick_custom()),
                dyn_node(0),             # normal or fallback slot
                dyn_node(1),
            ),
            String("safe-counter"),
        )
        # ... create normal + fallback child contexts ...

    fn flush(mut self, writer: ...) -> Int32:
        if self.ctx.has_error():
            # Error state: hide normal, show fallback with error message
            self.normal.child_ctx.flush_empty(writer)
            var fb_idx = self.render_fallback()
            self.fallback.child_ctx.flush(writer, fb_idx)
        else:
            # Normal state: show child, hide fallback
            self.fallback.child_ctx.flush_empty(writer)
            var child_idx = self.render_child()
            self.normal.child_ctx.flush(writer, child_idx)
        return self.ctx.finalize(writer)

    fn handle_event(mut self, handler_id: UInt32, ...) -> Bool:
        if handler_id == self.crash_handler:
            _ = self.ctx.report_error(String("Simulated crash"))
            return True
        elif handler_id == self.retry_handler:
            self.ctx.clear_error()       # next flush restores normal child
            return True
        return self.ctx.dispatch_event(handler_id, event_type)
```

Phase 33 adds suspense — pending state with skeleton fallback and JS-triggered
resolve. `use_suspense_boundary()` marks a scope as a suspense boundary;
`set_pending(True)` enters pending state; JS calls a resolve export to store
data and clear pending; flush switches between content and skeleton children:

```mojo
# Mojo equivalent — suspense pattern (Phase 33):
struct DataLoaderApp:
    var ctx: ComponentContext
    var content: DLContentChild          # content child: p > "Data: ..."
    var skeleton: DLSkeletonChild        # skeleton child: p > "Loading..."
    var data_text: String
    var load_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.ctx.use_suspense_boundary()   # mark root as suspense boundary
        self.data_text = String("(none)")
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Data Loader"))),
                el_button(dsl_text(String("Load")), onclick_custom()),
                dyn_node(0),               # content slot
                dyn_node(1),               # skeleton slot
            ),
            String("data-loader"),
        )
        # ... create content + skeleton child contexts ...

    fn flush(mut self, writer: ...) -> Int32:
        if self.ctx.is_pending():
            # Pending: hide content, show skeleton
            self.content.child_ctx.flush_empty(writer)
            var skel_idx = self.skeleton.render()
            self.skeleton.child_ctx.flush(writer, skel_idx)
        else:
            # Resolved: show content with data, hide skeleton
            self.skeleton.child_ctx.flush_empty(writer)
            var content_idx = self.content.render(self.data_text)
            self.content.child_ctx.flush(writer, content_idx)
        return self.ctx.finalize(writer)

    fn handle_event(mut self, handler_id: UInt32, ...) -> Bool:
        if handler_id == self.load_handler:
            self.ctx.set_pending(True)     # next flush shows skeleton
            return True
        return self.ctx.dispatch_event(handler_id, event_type)

# JS calls dl_resolve(data) to clear pending:
# fn resolve(data: String):
#     self.data_text = data
#     self.ctx.set_pending(False)          # next flush restores content
```

### Memo type expansion (Phase 35)

`MemoBool` and `MemoString` provide the same ergonomic handle pattern as `MemoI32`,
enabling derived state of any supported type. The runtime automatically propagates
dirtiness through memo → memo chains (Phase 36), so each memo checks `is_dirty()`
independently. Recomputation order still matters (upstream before downstream):

```mojo
# Mixed-type memo chain: SignalI32 → MemoI32 → MemoBool → MemoString
struct MemoChainApp:
    var ctx: ComponentContext
    var input: SignalI32
    var doubled: MemoI32               # input * 2
    var is_big: MemoBool               # doubled >= 10
    var label: MemoString              # "BIG" if is_big else "small"

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.input = self.ctx.use_signal(0)
        self.doubled = self.ctx.use_memo(0)
        self.is_big = self.ctx.use_memo_bool(False)
        self.label = self.ctx.use_memo_string(String("small"))
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Memo Chain"))),
                el_button(dsl_text(String("+ 1")), dsl_onclick_add(self.input, 1)),
                el_p(dsl_dyn_text()),   # "Input: N"
                el_p(dsl_dyn_text()),   # "Doubled: N"
                el_p(dsl_dyn_text()),   # "Is Big: true/false"
                el_p(dsl_dyn_text()),   # "Label: small/BIG"
            ),
            String("memo-chain"),
        )

    fn run_memos(mut self):
        # Each memo checks is_dirty() independently — the runtime's
        # worklist-based propagation (Phase 36) marks all downstream
        # memos dirty when the input signal is written.
        if self.doubled.is_dirty():
            self.doubled.begin_compute()
            var i = self.input.read()
            self.doubled.end_compute(i * 2)

        if self.is_big.is_dirty():
            self.is_big.begin_compute()
            var d = self.doubled.read()
            self.is_big.end_compute(d >= 10)

        if self.label.is_dirty():
            self.label.begin_compute()
            var big = self.is_big.read()
            if big:
                self.label.end_compute(String("BIG"))
            else:
                self.label.end_compute(String("small"))

    fn flush(mut self, writer: ...) -> Int32:
        if not self.ctx.consume_dirty():
            return 0
        self.run_memos()             # settle all derived state
        var idx = self.render()
        self.ctx.diff(writer, idx)
        return self.ctx.finalize(writer)
```

### Effect drain-and-run pattern (Phase 34)

Effects run between `consume_dirty()` and `render()` to settle derived state before rendering.
The `begin_run()` / `end_run()` bracket re-subscribes the effect to its dependencies each run:

```mojo
struct EffectDemoApp:
    var ctx: ComponentContext
    var count: SignalI32
    var doubled: SignalI32          # written by effect
    var parity: SignalString        # written by effect
    var count_effect: EffectHandle

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.doubled = self.ctx.use_signal(0)
        self.parity = self.ctx.use_signal_string(String("even"))
        self.count_effect = self.ctx.use_effect()
        self.ctx.setup_view(
            el_div(
                el_h1(text(String("Effect Demo"))),
                el_button(text(String("+ 1")), onclick_add(self.count, 1)),
                el_p(dyn_text()),   # "Count: N"
                el_p(dyn_text()),   # "Doubled: N"
                el_p(dyn_text()),   # "Parity: even/odd"
            ),
            String("effect-demo"),
        )

    fn run_effects(mut self):
        if self.count_effect.is_pending():
            self.count_effect.begin_run()
            var c = self.count.read()   # re-subscribe to count
            self.doubled.set(c * 2)
            self.parity.set(String("even") if c % 2 == 0 else String("odd"))
            self.count_effect.end_run()

    fn flush(mut self, writer: ...) -> Int32:
        if not self.ctx.consume_dirty():
            return 0
        self.run_effects()   # effects settle derived state
        var idx = self.render()
        self.ctx.diff(writer, idx)
        return self.ctx.finalize(writer)
```

For memo + effect chains, recompute memos first — memo output changes mark dependent effects pending:

```mojo
# Signal → Memo → Effect → Signal chain (EffectMemoApp)
fn run_memos_and_effects(mut self):
    # Step 1: Recompute dirty memos
    if self.tripled.is_dirty():
        self.tripled.begin_compute()
        var i = self.input.read()        # re-subscribe memo to input
        self.tripled.end_compute(i * 3)
    # Step 2: Run effects that read memo output
    if self.label_effect.is_pending():
        self.label_effect.begin_run()
        var t = self.tripled.read()      # re-subscribe to memo output
        if t < 10:
            self.label.set(String("small"))
        else:
            self.label.set(String("big"))
        self.label_effect.end_run()
```

## Deferred abstractions

Some Dioxus features cannot be idiomatically expressed in Mojo today due to
language limitations tracked on the [Mojo roadmap](https://docs.modular.com/mojo/roadmap/).
They are documented here so they can be revisited as Mojo evolves:

| Dioxus feature | Mojo blocker | Roadmap item | Status |
|---|---|---|---|
| **Closure event handlers** (`onclick: move \|_\| count += 1`) | No closures/function pointers in WASM; handlers use action-based structs. 0.26.1 improves function type conversions (non-raising → raising, ref → value) but true closures still missing | Lambda syntax (Phase 1), Closure refinement (Phase 1) | 🚧 In progress |
| **`rsx!` macro** (compile-time DSL) | No hygienic macros | Hygienic importable macros (Phase 2) | ⏰ Not started |
| **`for` loops in views** (`for item in items { ... }`) | Views are static templates; iteration happens in build functions | Hygienic macros (Phase 2) | ⏰ Not started |
| **Generic `Signal[T]`** (`use_signal(\|\| vec![])`) | Runtime stores fixed `Int32` signals; parametric stores need conditional conformance. Phase 18 added `SignalBool`, Phase 19 added `SignalString`, Phase 35 added `MemoBool` + `MemoString` — three memo types now match three signal types, reducing urgency. **0.26.1 adds `conforms_to()` + `trait_downcast()` (experimental) enabling static dispatch on trait conformance, plus expanded reflection (`struct_field_count`, `struct_field_names`, `struct_field_types`, `offset_of`) — stepping stones toward a generic signal store** | Conditional conformance (Phase 1) | 🚧 Partially unblocked |
| **Dynamic component dispatch** (trait objects for components) | No existentials/dynamic traits. 0.26.1: `AnyType` no longer requires `__del__()` (explicitly-destroyed types) helps but doesn't solve dispatch | Existentials / dynamic traits (Phase 2) | ⏰ Not started |
| **Pattern matching on actions** | `if/elif` chains instead of `match` | Algebraic data types & pattern matching (Phase 2) | ⏰ Not started |
| ~~**Async data loading / suspense**~~ | **Suspense (simulated) implemented in Phase 33.** True async still blocked on first-class async. Synchronous suspense with JS-triggered resolve available now | First-class async support (Phase 2) | ✅ Simulated |
| **Untyped Python-style code** | Explicit types required everywhere | Phase 3: Dynamic OOP | ⏰ Not started |

When these Mojo features land, the corresponding Dioxus patterns can be
adopted — closures would eliminate `ItemBuilder.add_custom_event()` + `get_action()`,
macros would enable an `rsx!`-like DSL, and generic signals would replace the
current `SignalI32` / `SignalBool` / `SignalString` / `MemoI32` / `MemoBool` / `MemoString` handles with
`Signal[Int32]`, `Signal[Bool]`, `Signal[String]`, `Memo[Int32]`, `Memo[Bool]`, `Memo[String]`, etc.

### 0.26.1 new features applicable to existing code

Beyond unblocking deferred abstractions, Mojo 0.26.1 brings features that can
improve mojo-wasm incrementally during the migration:

- **Typed errors** (F1) — `raises CustomError` compiles as alternate return values with zero stack unwinding, ideal for WASM. Define `EventError`, `DiffError`, `MutationError` for the dispatch, diff, and mutation paths.
- **String UTF-8 safety** (F2) — `String(from_utf8=span)`, `String(from_utf8_lossy=span)`, `String(unsafe_from_utf8=span)` for explicit guarantees in the WASM ↔ JS string bridge.
- **Trait default impls** (F3) — `Writable`, `Equatable`, `Hashable` auto-derive from struct fields via reflection. Add conformance to `ElementId`, `Node`, `HandlerEntry`, `VNode` with zero boilerplate.
- **`Never` type** (F10) — annotate unreachable code paths and `abort()` wrappers for compile-time safety.
