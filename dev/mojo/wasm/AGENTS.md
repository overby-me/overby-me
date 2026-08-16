# mojo-wasm — AI Agent Context

> Compact quick-reference for AI agents. For project overview, architecture,
> build commands, and test infrastructure see [README.md](README.md).
> For development history see [CHANGELOG.md](CHANGELOG.md).

## Mojo Constraints

- **No closures/function pointers in WASM** — event handlers are action-based structs. 0.26.1 improves function type conversions (non-raising → raising, ref → value) but true closures still missing.
- **`@export` only works in main.mojo** — submodule exports get DCE'd. All ~430 WASM exports are thin wrappers in `src/main.mojo` forwarding to submodule implementations.
- **Single-threaded** — no sync needed.
- **Operator overloading** works (SignalI32 has `+=`, `-=`, `peek()`, `set()`).
- **Format**: `mojo format <file>` — pre-commit hooks run this automatically.
- **Commit messages**: `feat(mojo-wasm): Uppercase description` — commitlint enforced, allowed types: `feat`, `fix`, `chore`, `doc`.
- **Mojo 0.26.1 migration in progress** — see [MIGRATION_PLAN.md](MIGRATION_PLAN.md) for full details. Key syntax changes: `List[T](a, b, c)` → `[a, b, c]` list literals, `alias` → `comptime`, explicit `Bool` conversions required (no `ImplicitlyBoolable`).

## Mojo 0.26.1 Migration

Migration to Mojo 0.26.1 is tracked in [MIGRATION_PLAN.md](MIGRATION_PLAN.md). Summary for agents.

> **Phase 39 re-evaluation (post-Phase 38):** All 7 deferred features (F1, F2, F5, F6, F8, F9, F10) were re-examined after Phases 31–38. None have gained natural application points. All remain correctly deferred — see `MIGRATION_PLAN.md` "Phase 39 Re-evaluation" section for details.

### Breaking Changes

| ID | Change | Impact | Scope |
|----|--------|--------|-------|
| **B1** | `List[T](a, b, c)` variadic init removed → use list literals `[a, b, c]` | Widespread | ~50–80 call sites |
| **B2** | `alias` keyword deprecated → use `comptime` | Pervasive | ~150+ declarations |
| **B3** | `ImplicitlyBoolable` removed → explicit `!= 0` / `!= UnsafePointer[T]()` checks | Moderate | ~20–40 sites |
| **B4** | `UInt` is now `Scalar[DType.uint]`, no implicit `Int` ↔ `UInt` conversion | Low | Audit needed |
| **B6** | `Error()` default construction removed, `Error` not `Boolable` | Low | `grep -rn 'Error()' src/` |
| **B8** | `Writer.write_bytes()` → `write_string()`, `String.__init__(bytes:)` → `unsafe_from_utf8` | Low | Only if custom `Writer` impls exist |

### New Syntax Patterns

```mojo
# List literals (B1) — type inferred from first element or annotation:
el_div([
    el_h1([dyn_text(0)]),
    el_button([text("Up!"), onclick_add(count, 1)]),
])
var keys: List[UInt32] = [1, 2, 3]

# comptime (B2) — replaces alias:
comptime OP_END = UInt8(0x00)
comptime TAG_DIV: UInt8 = 0

# Explicit bool (B3) — no implicit truthiness:
if self._free_head != -1:           # not: if self._free_head:
if ptr != UnsafePointer[T]():       # not: if ptr:
```

### New Features to Leverage

| ID | Feature | Opportunity | Priority |
|----|---------|-------------|----------|
| **F1** | Typed errors (`raises CustomError`) | Zero-overhead error types for WASM — define `EventError`, `DiffError`, `MutationError` | Medium |
| **F2** | String UTF-8 constructors (`from_utf8=`, `from_utf8_lossy=`, `unsafe_from_utf8=`) | Explicit safety in WASM ↔ JS string bridge | Medium |
| **F3** | Default trait impls (`Equatable`, `Writable` auto-derived) | Add conformance to `ElementId`, `Node`, `HandlerEntry`, `VNode` with zero boilerplate | Medium |
| **F4** | `Copyable` refines `Movable` | Remove redundant `Movable` declarations | Low |
| **F5** | `comptime(x)` expression | Inline compile-time evaluation without separate declarations | Low |
| **F8** | `conforms_to()` + `trait_downcast()` (experimental) | Stepping stone toward generic `Signal[T]` with static dispatch | Low |
| **F9** | Expanded reflection (`struct_field_count`, `struct_field_names`, `offset_of`) | Auto-generated binary protocol encoders, debug formatters | Low |
| **F10** | `Never` type | Annotate unreachable code paths and `abort()` wrappers | Low |

### Migration Order

1. ✅ **B3** — Fix `ImplicitlyBoolable` (hard compile errors, scattered)
2. ✅ **B1** — Update `List[T](...)` → list literals (most widespread)
3. ✅ **B4–B8** — Minor breaks (`UInt`, `Error`, `InlineArray`, `Writer`)
4. ✅ **B2** — Bulk `alias` → `comptime` (mechanical, last to avoid merge conflicts)
5. ✅ **F3** — Default trait impls (auto-derived `Equatable`, `Writable`)
6. ✅ **F4** — Remove redundant `Movable` declarations
7. ✅ **F7** — Enable `-Werror` in build

### Deferred (re-evaluated at Phase 39 — all remain deferred)

- 🟡 **F1** — Typed errors — no `raises` functions in `src/`; runtime uses `Bool`/`Int32` returns for WASM ABI.
- 🟡 **F2** — UTF-8 constructors — no raw-bytes string construction in `src/` (only in test harness).
- 🟡 **F5** — `comptime(x)` expression — all constants are named module-level declarations; no inline use case.
- 🟡 **F6** — `-Xlinker` flag — permanently not applicable (custom `llc` + `wasm-ld` pipeline).
- 🟡 **F8** — `conforms_to()` / `trait_downcast()` — blocked on generic `Signal[T]` store design.
- 🟡 **F9** — Reflection module — experimental; existing hand-written protocol works.
- 🟡 **F10** — `Never` type — no `abort()` or unreachable code paths in `src/`.

## Key Abstractions (dependency order)

### Signals & Reactivity (`src/signals/`)

- `Runtime` — reactive runtime: signal store, string store, scope tracking, context management.
- `SignalStore` — type-erased storage for fixed-size value signals (Int32). Uses raw-byte memcpy — safe for value types only.
- `StringStore` (`signals/runtime.mojo`) — Phase 19 safe heap-string storage with slab-style free-list slot reuse. Methods: `create(initial) -> UInt32`, `read(key) -> String`, `write(key, value)`, `destroy(key)`, `count()`, `contains(key)`. Lives as `Runtime.strings` field. Solves the problem that `SignalStore` (memcpy-based) is unsafe for heap types like String.
- `SignalI32` (`signals/handle.mojo`) — ergonomic handle with `peek()`, `set()`, `+=`, `-=`. Holds key + runtime pointer.
- `SignalBool` (`signals/handle.mojo`) — Phase 18 ergonomic boolean signal wrapping Int32 (0/1). `get() -> Bool`, `read() -> Bool` (with context subscription), `set(Bool)`, `toggle()`, `peek_i32() -> Int32`, `version()`, `__str__()` ("true"/"false"). Created via `ctx.use_signal_bool(initial)` or `ctx.create_signal_bool(initial)`.
- `SignalString` (`signals/handle.mojo`) — Phase 19 ergonomic reactive string signal. Wraps a `string_key` (index in StringStore) + `version_key` (companion Int32 signal in SignalStore for subscriber tracking). `get() -> String` / `peek() -> String` (read without subscribing), `read() -> String` (subscribe context via version signal), `set(String)` (write + bump version → marks subscribers dirty), `version() -> UInt32`, `is_empty() -> Bool`, `__str__() -> String`. Created via `ctx.use_signal_string(initial)` or `ctx.create_signal_string(initial)`.
- `MemoI32` — derived signal with lazy recomputation and auto dependency tracking.
- `MemoBool` (`signals/handle.mojo`) — Phase 35 ergonomic boolean memo wrapping an Int32 memo entry (0/1). `peek() -> Bool` (read without subscribing), `read() -> Bool` (read with context subscription), `is_dirty() -> Bool`, `begin_compute()`, `end_compute(Bool)`, `copy() -> Self`. Created via `ctx.use_memo_bool(initial)`. Recomputation follows the same begin/end bracket as `MemoI32`.
- `MemoString` (`signals/handle.mojo`) — Phase 35 ergonomic string memo. Wraps a `string_key` (StringStore slot) + `version_key` (companion Int32 memo in MemoStore for dirty/version tracking). `peek() -> String` (read without subscribing), `read() -> String` (read with context subscription via version memo), `is_dirty() -> Bool`, `begin_compute()`, `end_compute(String)`, `copy() -> Self`. Created via `ctx.use_memo_string(initial)`. Lifecycle mirrors `SignalString` (StringStore + version signal) but uses memo infrastructure for lazy recomputation.
- `EffectHandle` — reactive side effects.

### Scopes (`src/scope/`)

- `ScopeState` — lifecycle unit with hooks, context, error boundaries.
- `ScopeArena` — slab allocator for scopes. Parent→child hierarchy.

### Virtual DOM (`src/vdom/`)

- `Node` (DSL union) — `text()`, `dyn_text()`, `dyn_node()`, `attr()`, `dyn_attr()`, `el_div()`, `el_button()`, etc.
- **Multi-arg `el_*` overloads** — 1–5 `Node` argument overloads for all 38 element helpers, eliminating `List[Node](...)` wrappers. Uses `var` ownership + `^` transfer. Example: `el_div(el_h1(dyn_text()), el_button(text("Up!"), onclick_add(count, 1)))`.
- Inline event constructors: `onclick_add(signal, delta)`, `onclick_sub()`, `onclick_set()`, `onclick_toggle()`, `on_event()`.
- **Conditional helpers** (Phase 18): `class_if(condition, name) -> String` (returns name or ""), `class_when(condition, true_class, false_class) -> String`, `text_when(condition, true_text, false_text) -> String`. Eliminate if/else boilerplate for dynamic attributes and text.
- `dyn_text()` with no args → auto-numbered (sentinel `DYN_TEXT_AUTO`).
- `to_template(node, name)` → `Template` (static structure for DOM cloning).
- `VNode` — runtime instance of a template with dynamic slots.
- `VNodeBuilder` — fills dynamic text/attr/event slots on a VNode. `add_dyn_text(value)`, `add_dyn_text_attr(name, value)`, `add_dyn_bool_attr(name, value)`, `add_dyn_event(event, handler_id)`, `add_dyn_placeholder()`.
- `VNodeStore` — arena for VNode storage.

### Mutations (`src/mutations/`)

- `CreateEngine` — walks VNode tree, emits create mutations (initial mount).
- `DiffEngine` — compares old/new VNode trees, emits minimal update mutations (keyed reconciliation).
- `MutationWriter` (`src/bridge/protocol.mojo`) — writes binary opcodes to shared buffer.

### Events (`src/events/`)

- `HandlerEntry` — action-based handler (signal_add, signal_sub, signal_set, signal_toggle, signal_set_string, custom).
  - Phase 20: `HandlerEntry.signal_set_string(scope_id, string_key, version_key, event_name)` creates a handler that writes a string event value to a `SignalString`. Stores `string_key` in the `signal_key` field and `version_key` in the `operand` field (cast to Int32).
- `HandlerRegistry` — maps handler IDs → entries. Scope-scoped cleanup.
- Action tags: `ACTION_NONE` (0), `ACTION_SIGNAL_SET_I32` (1), `ACTION_SIGNAL_ADD_I32` (2), `ACTION_SIGNAL_SUB_I32` (3), `ACTION_SIGNAL_TOGGLE` (4), `ACTION_SIGNAL_SET_INPUT` (5), `ACTION_SIGNAL_SET_STRING` (6, Phase 20), `ACTION_CUSTOM` (255).

### Component Layer (`src/component/`)

- **`AppShell`** — bundles Runtime + VNodeStore + ElementIdAllocator + Scheduler. Low-level API.
- **`ComponentContext`** — ergonomic wrapper over AppShell. High-level API for apps:
  - `ComponentContext.create()` → allocates shell, root scope, begins render bracket.
  - `ctx.use_signal(initial)` → `SignalI32` (auto-subscribes scope).
  - `ctx.use_signal_bool(initial)` → `SignalBool` (auto-subscribes scope).
  - `ctx.use_signal_string(initial)` → `SignalString` (auto-subscribes scope). Phase 19.
  - `ctx.use_memo(initial)` → `MemoI32`.
  - `ctx.use_effect()` → `EffectHandle`.
  - `ctx.end_setup()` — closes render bracket.
  - `ctx.create_signal_string(initial)` → `SignalString` (no hooks, no subscription). Phase 19.
  - `ctx.register_template(view, name)` — sets `ctx.template_id`.
  - `ctx.register_extra_template(view, name) -> UInt32` — for multi-template apps.
  - `ctx.setup_view(view, name)` — combines `end_setup()` + `register_view()` (with inline event extraction + auto-numbered dyn_text).
  - `ctx.register_view(view, name)` — processes inline events (`onclick_add` etc.), auto-numbers `dyn_text()`, registers handlers.
  - `ctx.render_builder()` → `RenderBuilder` (auto-adds registered event attrs on `build()`). Phase 19 adds `add_dyn_text_signal(SignalString)`.
  - `ctx.mount(writer, vnode_idx)` — emit templates + create + append to root.
  - `ctx.flush(writer, new_idx)` — diff + finalize (convenience).
  - `ctx.dispatch_event(handler_id, event_type)` → Bool.
  - `ctx.dispatch_event_with_string(handler_id, event_type, value: String)` → Bool. Phase 20: dispatches string payloads — for `ACTION_SIGNAL_SET_STRING` handlers, writes value to the target `SignalString`; falls back to normal dispatch for other action types.
  - `ctx.consume_dirty()` → Bool.
  - `ctx.on_click_add()`, `on_click_sub()`, `on_click_set()`, `on_click_toggle()` — manual handler registration.
  - `ctx.register_handler(entry)` — raw handler registration.
  - `ctx.create_child_scope()` / `ctx.destroy_child_scopes(ids)` — for keyed list items.
  - `ctx.flush_fragment(writer, slot, frag_idx)` / `ctx.build_empty_fragment()` / `ctx.push_fragment_child()` — fragment lifecycle.
  - `ctx.vnode_builder()` / `ctx.vnode_builder_for(tmpl_id)` — VNode construction.
- **`FragmentSlot`** — tracks empty↔populated transitions for dynamic keyed lists.
- **`KeyedList`** (`src/component/keyed_list.mojo`) — bundles `FragmentSlot` + child scope IDs + item template ID + handler map for keyed-list components. Methods: `begin_rebuild(ctx)` (destroy old scopes + clear handler map, return empty fragment), `begin_item(key, ctx)` → `ItemBuilder` (Phase 17 — create scope + keyed VNodeBuilder in one call), `get_action(handler_id)` → `HandlerAction` (Phase 17 — dispatch lookup), `create_scope(ctx)` (create + track child scope), `item_builder(key, ctx)` (keyed VNodeBuilder), `push_child(ctx, frag, child)`, `flush(ctx, writer, frag)` (fragment transitions), `init_slot(anchor, frag)`, `handler_count()`.
- **`ItemBuilder`** — Phase 17 ergonomic per-item builder wrapping VNodeBuilder + child scope + handler map pointer. Methods: `add_dyn_text(value)`, `add_dyn_text_signal(SignalString)` (Phase 19 — read signal + add as dyn text), `add_dyn_text_attr(name, value)`, `add_dyn_bool_attr(name, value)`, `add_dyn_event(event, handler_id)`, `add_custom_event(event, action_tag, data)` (registers handler + maps action + adds event attr in one call), `add_class_if(condition, class_name)` (Phase 18 — conditional CSS class in one call), `add_class_when(condition, true_class, false_class)` (Phase 18 — binary class switching), `add_dyn_placeholder()`, `index()`.
- **`HandlerAction`** — Phase 17 result of `KeyedList.get_action(handler_id)`. Fields: `tag: UInt8` (app-defined action), `data: Int32` (e.g. item ID), `found: Bool`.
- **Lifecycle helpers**: `mount_vnode()`, `diff_and_finalize()`, `flush_fragment()`.
- **Error boundary methods** on `ComponentContext`:
  - `ctx.use_error_boundary()` — mark root scope as an error boundary (call during setup).
  - `ctx.report_error(message) -> Int` — propagate error to nearest boundary; returns boundary scope ID or -1.
  - `ctx.has_error() -> Bool` — check if this boundary has captured an error.
  - `ctx.error_message() -> String` — get the captured error message.
  - `ctx.clear_error()` — clear error state and mark scope dirty for re-render.
- **Error boundary methods** on `ChildComponentContext`:
  - `child_ctx.use_error_boundary()` — mark child scope as an error boundary.
  - `child_ctx.report_error(message) -> Int` — propagate error from child scope to nearest boundary.
  - `child_ctx.has_error() -> Bool` — check if this child boundary has captured an error.
  - `child_ctx.error_message() -> String` — get the captured error message.
  - `child_ctx.clear_error()` — clear error state on child boundary.
- **Suspense methods** on `ComponentContext`:
  - `ctx.use_suspense_boundary()` — mark root scope as a suspense boundary (call during setup).
  - `ctx.set_pending(pending: Bool)` — set or clear pending state; marks scope dirty.
  - `ctx.has_pending() -> Bool` — check if any descendant scope is pending.
  - `ctx.is_pending() -> Bool` — check if this scope itself is pending.
- **Suspense methods** on `ChildComponentContext`:
  - `child_ctx.use_suspense_boundary()` — mark child scope as a suspense boundary.
  - `child_ctx.set_pending(pending: Bool)` — set or clear pending state on child scope; marks dirty.
  - `child_ctx.has_pending() -> Bool` — check if any descendant of child scope is pending.
  - `child_ctx.is_pending() -> Bool` — check if the child scope itself is pending.

## App Architectures (`examples/`, `src/apps/`, and `src/main.mojo`)

All app struct and lifecycle code lives in dedicated modules — example apps in `examples/` (counter, todo, bench) and demo/test apps in `src/apps/`. Only the thin `@export` WASM wrappers remain in `src/main.mojo` (Mojo requires exports in the main compilation unit). All apps use `ComponentContext` with constructor-based setup and multi-arg `el_*` overloads. TodoApp and BenchmarkApp use Phase 17 `ItemBuilder` + `HandlerAction` for ergonomic per-item building and dispatch, with Phase 18 conditional helpers (`add_class_if`, `text_when`) to eliminate if/else boilerplate. Phase 19 adds `SignalString` for reactive string state — TodoApp's `input_text` field was migrated from plain `String` to `SignalString` via `create_signal_string()` (M19.7). Phase 20 adds string event dispatch infrastructure (`ACTION_SIGNAL_SET_STRING`, `dispatch_event_with_string`) enabling JS → WASM string value flow for input events. Phase 20.5 migrates the TodoApp to fully WASM-driven input binding using `bind_value()`, `oninput_set_string()`, and `onclick_custom()` — JS has no special-casing for any handler. Phase 21 introduces `launch()` (`examples/lib/app.js`) — a convention-based app launcher that eliminates per-app boot boilerplate. Phase 22 adds WASM-driven Enter key handling; counter and todo now use identical zero-config launch() calls. Phase 23 converges bench to launch() with `onBoot` for toolbar wiring and event delegation — all three apps now use the shared boot infrastructure. Phase 32 adds error boundary demo apps (SafeCounterApp, ErrorNestApp) — these are test-only apps exercising `use_error_boundary()`, `report_error()`, `has_error()`, `clear_error()` with fallback UI switching. Phase 33 adds suspense demo apps (DataLoaderApp, SuspenseNestApp) — these are test-only apps exercising `use_suspense_boundary()`, `set_pending()`, `is_pending()` with content/skeleton switching and JS-triggered resolve.

### CounterApp (`counter.mojo`) — simplest example

```txt
struct CounterApp:
    var ctx: ComponentContext
    var count: SignalI32
    fn __init__: ctx.create() → use_signal → setup_view(inline events, multi-arg el_*)
    fn render: ctx.render_builder() → add_dyn_text → build()
```

Lifecycle: `counter_app_init()` → `counter_app_rebuild()` → `counter_app_handle_event()` → `counter_app_flush()`.

### TodoApp (`todo.mojo`) — keyed lists, two-way input binding, Enter key, custom handlers, SignalString

```txt
struct TodoApp:
    var ctx: ComponentContext
    var list_version: SignalI32
    var input_text: SignalString   # create_signal_string (no subscription, write-buffer)
    var items: KeyedList          # bundles template_id + FragmentSlot + scope_ids + handler_map
    var data: List[TodoItem]
    var add_handler: UInt32       # auto-registered by register_view() via onclick_custom()
    fn __init__: create_signal_string → register_view("todo-app" with bind_value + oninput_set_string
                 + onclick_custom) → view_event_handler_id(1) for add_handler
                 + KeyedList(register_extra_template("todo-item"))
    fn render: ctx.render_builder() → add_dyn_placeholder → build() (auto-populates bindings)
    fn build_item_vnode: items.begin_item(key, ctx) → ib.add_custom_event() (Phase 17)
    fn build_items_fragment: items.begin_rebuild → build each item → items.push_child
    fn handle_event: if add_handler → read input_text.peek(), add_item, clear signal
                     else → items.get_action(handler_id) → toggle/remove item
```

Phase 20.5 migration: TodoApp now uses `register_view()` with inline event/binding helpers for the app template. The input element has `bind_value(input_text)` + `oninput_set_string(input_text)` for Dioxus-style two-way binding. The Add button uses `onclick_custom()` — an inline custom handler auto-registered by `register_view()`. `handle_event()` handles the Add action entirely in WASM: reads `input_text.peek()`, calls `add_item()`, clears via `input_text.set("")`. `render()` uses `render_builder()` which auto-populates bind_value (reads signal → "value" attr) and event listeners. `todo_app_flush()` re-renders the app shell via `ctx.diff()` (to catch bind_value changes) before flushing items via KeyedList. JS dispatches events uniformly — `input`/`change` events go through `todo_dispatch_string()`, all others through `todo_handle_event()` — no special-casing. WASM exports: `todo_dispatch_string` for string events, `todo_add_handler_id` for the Add handler ID, plus existing `todo_set_input`, `todo_input_version`, `todo_input_is_empty`.

### BenchmarkApp (`bench.mojo`) — js-framework-benchmark, WASM-rendered toolbar + keyed rows

```txt
struct BenchmarkApp:
    var ctx: ComponentContext
    var version: SignalI32
    var selected: SignalI32
    var rows_list: KeyedList      # bundles template_id + FragmentSlot + scope_ids + handler_map
    var rows: List[BenchRow]
    var op_name: String           # P24.4: dyn_text[0] — "Ready" or operation name
    var timing_text: String       # P24.4: dyn_text[1] — "" or " — X.Yms"
    var row_count_text: String    # P24.4: dyn_text[2] — "" or " · N rows"
    var create1k_handler: UInt32  # auto-registered by register_view() via onclick_custom()
    var create10k_handler: UInt32
    var append_handler: UInt32
    var update_handler: UInt32
    var swap_handler: UInt32
    var clear_handler: UInt32
    fn __init__: ctx.create() → use_signal(version, selected) → setup_view("bench-app"
                 with 6 onclick_custom buttons, 3 dyn_text status, dyn_node(3) for rows)
                 → view_event_handler_id(0..5) for toolbar handlers
                 + KeyedList(register_extra_template("bench-row"))
                 + op_name = "Ready", timing_text = "", row_count_text = ""
    fn render: ctx.render_builder() → add_dyn_text(op_name) → add_dyn_text(timing_text)
               → add_dyn_text(row_count_text) → add_dyn_placeholder → build()
    fn build_row_vnode: rows_list.begin_item(key, ctx) → ib.add_custom_event() (Phase 17)
    fn build_rows_fragment: rows_list.begin_rebuild → build each row → rows_list.push_child
    fn handle_event: t0 = performance_now()
                     if create1k_handler → create_rows(1000) → op_name/timing_text/row_count_text
                     elif create10k_handler → create_rows(10000) → op_name/timing_text/row_count_text
                     elif append_handler → append_rows(1000) → op_name/timing_text/row_count_text
                     elif update_handler → update_every_10th() → op_name/timing_text/row_count_text
                     elif swap_handler → swap_rows(1, 998) → op_name/timing_text/row_count_text
                     elif clear_handler → clear_rows() → op_name/timing_text/row_count_text
                     else → rows_list.get_action(handler_id) → select/remove row
```

Two signals: `version` (list changes), `selected` (highlight row).
Operations: create_rows, append_rows, update_every_10th, select_row, swap_rows, remove_row, clear_rows.
Per-row build uses `begin_item()` + `add_custom_event()` (Phase 17) + `add_class_if()` (Phase 18).

Phase 24.4: The status bar uses 3 separate `dyn_text` nodes: `dyn_text[0]` = `op_name` ("Ready" or operation name), `dyn_text[1]` = `timing_text` ("" or " — X.Yms"), `dyn_text[2]` = `row_count_text` ("" or " · N rows"). The row list placeholder is at `dyn_node(3)` (indices 0-2 occupied by dyn_text). `format_timing_ms(ms) -> String` returns timing with leading em-dash separator. `format_row_count(count) -> String` returns row count with leading middle-dot separator and comma-formatted number via `_format_number()`. Only changed text nodes receive `SetText` mutations on flush — e.g. update-every-10th changes timing but not row count. New exports: `bench_op_name(app_ptr) -> String`, `bench_timing_text(app_ptr) -> String`, `bench_row_count_text(app_ptr) -> String`. `bench_status_text` returns the concatenation of all three fields for backward compatibility.

Phase 24.3: `performance_now() -> Float64` WASM import via `external_call["performance_now", Float64]()`. Each toolbar operation in `handle_event()` is wrapped with before/after `performance_now()` calls; the elapsed time is formatted to 1 decimal place via `format_timing_ms(ms) -> String` and stored in `timing_text`. Zero JS-side timing code.

Phase 24.2: Uses `setup_view()` for the app shell template ("bench-app") with inline `onclick_custom()` for 6 toolbar buttons. The entire UI — heading, buttons, status bar, table with thead + tbody — is rendered from WASM. Root is `#root` (not `#tbody`). `handle_event()` routes both toolbar button clicks (via handler ID comparison) and row clicks (via `get_action()`). `render()` uses `render_builder()` which auto-populates event handlers; provides 3 `dyn_text()` nodes for status (dynamic_nodes[0-2]) and `dyn_placeholder()` for the keyed row list (dynamic_nodes[3]). **Important:** `dyn_text` and `dyn_node` share the same `dynamic_nodes` index space — three auto-numbered `dyn_text()` get indices 0-2, so `dyn_node` must use index 3. `bench_app_rebuild()` follows the todo pattern: emit templates → render shell → CreateEngine → extract `dyn_node[3]` anchor → init KeyedList slot. `bench_app_flush()` diffs app shell + flushes KeyedList. `bench/main.js` is now a 7-line `launch()` call (only `bufferCapacity` is bench-specific).

**Phase 24 — Bench zero app-specific JS convergence** (see also `examples/bench/main.js` header):

- **P24.1** ✅ — `bench_handle_event` with handler_map dispatch. `BenchmarkApp.handle_event(handler_id)` calls `rows_list.get_action(handler_id)` and routes to `select_row`/`remove_row` (same pattern as `TodoApp.handle_event`). `bench_handle_event` WASM export in `main.mojo`. EventBridge now dispatches row clicks directly — tbody event delegation JS eliminated.
- **P24.2** ✅ — WASM-rendered toolbar with `onclick_custom` handlers. Entire app shell (h1, 6 buttons, status 3 `dyn_text` at dynamic_nodes[0-2], table with thead + tbody > `dyn_node(3)` at dynamic_nodes[3]) rendered from WASM via `setup_view()`. Root changed from `#tbody` to `#root`. 6 handler IDs extracted via `view_event_handler_id()`. `handle_event()` routes toolbar button clicks to benchmark operations + existing row click dispatch. `bench/index.html` simplified to `<div id="root">` + styles. `bench/main.js` reduced to 7-line `launch()` call. Tests updated: `createDOM()` creates root div, DOM tests query rendered tbody, handler lifecycle tests account for 6 toolbar base handlers. **Gotcha:** `dyn_text` and `dyn_node` share the `dynamic_nodes` index space — three auto-numbered `dyn_text()` get indices 0-2, so `dyn_node` must use 3.
- **P24.3** ✅ — `performance.now()` WASM import for timing. `performance_now() -> Float64` via `external_call` — Mojo compiler emits unresolved symbol, `wasm-ld --allow-undefined` turns it into WASM import from `env` module, JS host provides `performance_now: () => performance.now()`. `format_timing_ms(ms) -> String` formats elapsed time to 1 decimal place with em-dash separator. `handle_event()` wraps each toolbar op with before/after `performance_now()`, stores formatted result in `timing_text`. `render()` emits `timing_text` as `dyn_text[1]` — diff detects change on flush, emits `SetText`. Added to `env.js` (browser), `env.ts` (Deno runtime), and `wasm_harness.mojo` (func[16]: deterministic mock clock, increments by 1.0 per call). WASM import count: 16 → 17. Exports: `bench_status_text(app_ptr) -> String`, `bench_handler_id_at(app_ptr, index) -> i32`.
- **P24.4** ✅ — Fine-grained status bar with 3 `dyn_text` nodes. Split single `status_text` into `op_name` (dyn_text[0]), `timing_text` (dyn_text[1]), `row_count_text` (dyn_text[2]). Row list placeholder moved from `dyn_node(1)` to `dyn_node(3)`. Added `format_timing_ms(ms) -> String` (timing only with separator), `format_row_count(count) -> String` (comma-formatted with separator), `_format_number(n) -> String` (comma thousands). New exports: `bench_op_name`, `bench_timing_text`, `bench_row_count_text`. `bench_status_text` returns concatenation for backward compat. `bench/main.js` structurally identical to counter/todo (only `bufferCapacity` override remains).

### ContextTestApp (`src/apps/context_test.mojo`) — context (DI) surface test

```txt
struct ContextTestApp:
    var ctx: ComponentContext
    var child_scope_id: UInt32
    var count: SignalI32
    fn __init__: ctx.create() → use_signal → end_setup() → create_child_scope()
```

Minimal test app exercising `provide_context()`, `consume_context()`, `has_context()`, and typed signal-sharing helpers (`provide_signal_i32`, `consume_signal_i32`). Root scope + one child scope for parent-chain walk-up verification. WASM exports: `cta_init`, `cta_destroy`, `cta_provide_context`, `cta_consume_context`, `cta_has_context`, `cta_provide_signal_i32`, `cta_consume_signal_i32_from_child`, `cta_write_signal_via_child`, etc.

### ChildContextTestApp (`src/apps/child_context_test.mojo`) — self-rendering child test

```txt
struct ChildContextTestApp:
    var ctx: ComponentContext
    var count: SignalI32
    var child_ctx: ChildComponentContext
    var child_count: SignalI32      # consumed from parent context
    var child_show_hex: SignalBool  # child-owned local state
    fn __init__: ctx.create() → use_signal → setup_view → provide_signal_i32
                 → create_child_context → child use_signal_bool
    fn flush: parent diff + child flush (if dirty)
```

Lifecycle: `cct_init` → `cct_rebuild` → `cct_handle_event` → `cct_flush`. Parent provides count signal via context; child consumes it and owns a local `show_hex` toggle. Child self-renders via `child_ctx.render_builder()`. DOM shows parent h1 + buttons + child p with count text.

### PropsCounterApp (`src/apps/props_counter.mojo`) — self-rendering child with props

```txt
struct PropsCounterApp:
    var ctx: ComponentContext
    var count: SignalI32
    var display: CounterDisplay     # self-rendering child
    fn __init__: ctx.create() → use_signal → setup_view(h1 + buttons + dyn_node)
                 → create_child_context → child consume_signal_i32 + use_signal_bool
    fn flush: parent diff + child flush (error boundary not involved)
```

Lifecycle: `pc_init` → `pc_rebuild` → `pc_handle_event` → `pc_flush`. Parent has increment/decrement buttons; child (`CounterDisplay`) displays "Count: N" or "Count: 0xN" with a local `show_hex` toggle button. Count signal shared from parent to child via context props.

### ThemeCounterApp (`src/apps/theme_counter.mojo`) — shared context + cross-component

```txt
struct ThemeCounterApp:
    var ctx: ComponentContext
    var count: SignalI32
    var theme: SignalBool            # dark/light toggle
    var on_reset: SignalI32          # callback signal for upward communication
    var counter_child: TCCounterChild   # displays count + theme label + Reset button
    var summary_child: TCSummaryChild   # displays summary text + theme class
    fn __init__: ctx.create() → use_signal(count, theme, on_reset) → provide all via context
                 → setup_view(buttons + dyn_node ×2) → create 2 child contexts
    fn flush: check on_reset callback → parent diff + both children flush
```

Lifecycle: `tc_init` → `tc_rebuild` → `tc_handle_event` → `tc_flush`. Parent with theme toggle and two child components both consuming theme + count context. `TCCounterChild` has a Reset button that writes to a callback signal consumed by the parent — demonstrating upward communication via shared context.

### SafeCounterApp (`src/apps/safe_counter.mojo`) — error boundary with crash/retry

```txt
struct SafeCounterApp:
    var ctx: ComponentContext       # root scope = error boundary
    var count: SignalI32
    var normal: SCNormalChild       # display child (count text)
    var fallback: SCFallbackChild   # fallback child (error + retry button)
    var crash_handler: UInt32       # onclick_custom for crash button
    var retry_handler: UInt32       # onclick_custom for retry button
    fn __init__: ctx.create() → use_signal → use_error_boundary() → provide_signal_i32
                 → setup_view(h1 + button(+1) + button(Crash) + dyn_node(0) + dyn_node(1))
                 → create_child_context(normal) → create_child_context(fallback)
    fn flush: if ctx.has_error() → normal.flush_empty + fallback.flush
              else → fallback.flush_empty + normal.flush
    fn handle_event: crash_handler → ctx.report_error("Simulated crash")
                     retry_handler → ctx.clear_error()
                     else → ctx.dispatch_event()
```

Lifecycle: `sc_init` → `sc_rebuild` → `sc_handle_event` → `sc_flush`. Error boundary alternates between normal child (count display) and fallback child (error message + retry button). Count signal persists across crash/recovery cycles.

### ErrorNestApp (`src/apps/error_nest.mojo`) — nested error boundaries

```txt
struct ErrorNestApp:
    var ctx: ComponentContext           # outer boundary (root scope)
    var outer_normal: ENOuterNormalChild  # outer normal child (inner boundary)
    var outer_fallback: ENOuterFallbackChild  # outer fallback
    var outer_crash_handler: UInt32
    var outer_retry_handler: UInt32
    var inner_crash_handler: UInt32
    var inner_retry_handler: UInt32
    fn __init__: ctx.create() → use_error_boundary() → setup_view(h1 + buttons + dyn_node)
                 → outer_normal child (use_error_boundary on child scope)
                   └── inner_normal child + inner_fallback child
                 → outer_fallback child
    fn flush: if ctx.has_error() → outer_normal.flush_empty + outer_fallback.flush
              else → outer_fallback.flush_empty + outer_normal.flush (recurses into inner boundary)
    fn handle_event: outer_crash → ctx.report_error()
                     outer_retry → ctx.clear_error()
                     inner_crash → outer_normal.child_ctx.report_error()  (walks to inner boundary)
                     inner_retry → outer_normal.child_ctx.clear_error()
```

Lifecycle: `en_init` → `en_rebuild` → `en_handle_event` → `en_flush`. Inner crash caught by inner boundary (only inner slot swaps). Outer crash caught by outer boundary (entire inner tree replaced by outer fallback). Recovery at each level is independent.

### DataLoaderApp (`src/apps/data_loader.mojo`) — suspense with load/resolve lifecycle

```txt
struct DataLoaderApp:
    var ctx: ComponentContext           # suspense boundary (root scope)
    var content: DLContentChild        # content child: p > dyn_text("Data: ...")
    var skeleton: DLSkeletonChild      # skeleton child: p > dyn_text("Loading...")
    var data_text: String
    var load_handler: UInt32
    fn __init__: ctx.create() → use_suspense_boundary() → setup_view(h1 + button + dyn_node[0] + dyn_node[1])
                 → content child + skeleton child
    fn flush: if ctx.is_pending() → content.flush_empty + skeleton.flush
              else → skeleton.flush_empty + content.flush(data)
    fn handle_event: load_handler → ctx.set_pending(True)
    fn resolve(data): data_text = data, ctx.set_pending(False)
```

Lifecycle: `dl_init` → `dl_rebuild` → `dl_handle_event` (load) → `dl_flush` (skeleton shown) → `dl_resolve` (JS-triggered) → `dl_flush` (content shown with data). Load button sets pending, skeleton replaces content. JS calls `dl_resolve(data)` to clear pending and store data. Next flush restores content with loaded data.

### SuspenseNestApp (`src/apps/suspense_nest.mojo`) — nested suspense boundaries

```txt
struct SuspenseNestApp:
    var ctx: ComponentContext              # outer suspense boundary (root scope)
    var outer_content: SNOuterContentChild # outer content (inner suspense boundary)
    var outer_skeleton: SNOuterSkeletonChild # outer skeleton: p > "Outer loading..."
    var outer_data: String
    var inner_data: String
    var outer_load_handler: UInt32
    var inner_load_handler: UInt32
    fn __init__: ctx.create() → use_suspense_boundary() → setup_view(h1 + button + dyn_node)
                 → outer_content child (use_suspense_boundary on child scope)
                   └── inner_content child + inner_skeleton child
                 → outer_skeleton child
    fn flush: if ctx.is_pending() → hide inner children + outer_content.flush_empty + outer_skeleton.flush
              elif not outer_content.mounted → restore outer_content + inner tree (check inner pending)
              else → handle inner pending/resolved (inner slot swaps only)
    fn handle_event: outer_load → ctx.set_pending(True)
                     inner_load → outer_content.child_ctx.set_pending(True)
    fn outer_resolve(data): outer_data = data, ctx.set_pending(False)
    fn inner_resolve(data): inner_data = data, outer_content.child_ctx.set_pending(False)
```

Lifecycle: `sn_init` → `sn_rebuild` → `sn_handle_event` → `sn_flush`. Inner load shows inner skeleton (outer content unaffected). Outer load shows outer skeleton (hides entire inner tree). Outer resolve reveals inner boundary (may still be pending). Inner resolve shows inner content. Both boundaries operate independently.

### EffectDemoApp (`src/apps/effect_demo.mojo`) — effect-in-flush pattern

```txt
struct EffectDemoApp:
    var ctx: ComponentContext           # single root scope
    var count: SignalI32               # input signal
    var doubled: SignalI32             # derived by effect (count * 2)
    var parity: SignalString           # derived by effect ("even" / "odd")
    var count_effect: EffectHandle     # reacts to count, writes doubled + parity
    var incr_handler: UInt32
    fn __init__: ctx.create() → use_signal(0) × 2 + use_signal_string("even")
                 + use_effect() → setup_view(h1 + button + 3 × p > dyn_text)
    fn run_effects: if count_effect.is_pending() → begin_run → read count
                    → set doubled(count*2) + set parity → end_run
    fn render: 3 dyn_text slots (Count/Doubled/Parity)
    fn flush: consume_dirty → run_effects → render → diff → finalize
```

Lifecycle: `ed_init` → `ed_rebuild` (run_effects + mount) → `ed_handle_event` (onclick_add count) → `ed_flush` (effect runs, derived state updated, DOM diffed). Effect drain-and-run pattern: effects execute between `consume_dirty()` and `render()` so all derived state is settled before rendering.

### EffectMemoApp (`src/apps/effect_memo.mojo`) — signal → memo → effect → signal chain

```txt
struct EffectMemoApp:
    var ctx: ComponentContext           # single root scope
    var input: SignalI32               # input signal
    var tripled: MemoI32               # memo: input * 3
    var label: SignalString            # derived by effect ("small" if tripled<10, "big" otherwise)
    var label_effect: EffectHandle     # reads tripled memo, writes label
    var incr_handler: UInt32
    fn __init__: ctx.create() → use_signal(0) + use_memo(0) + use_signal_string("small")
                 + use_effect() → setup_view(h1 + button + 3 × p > dyn_text)
    fn run_memos_and_effects: if tripled.is_dirty() → begin_compute → read input
                              → end_compute(input*3)
                              if label_effect.is_pending() → begin_run → read tripled
                              → set label → end_run
    fn render: 3 dyn_text slots (Input/Tripled/Label)
    fn flush: consume_dirty → run_memos_and_effects → render → diff → finalize
```

Lifecycle: `em_init` → `em_rebuild` (recompute memo + run effect + mount) → `em_handle_event` (onclick_add input) → `em_flush` (memo recomputed, effect runs, label updated, DOM diffed). Chain: input write → memo dirty → recompute memo → output signal write → effect pending → run effect → label signal write → render. Memos MUST be recomputed before effects that read their output.

### MemoFormApp (`src/apps/memo_form.mojo`) — MemoBool + MemoString form validation

```txt
struct MemoFormApp:
    var ctx: ComponentContext           # single root scope
    var input: SignalString             # text input signal
    var is_valid: MemoBool             # memo: len(input) > 0
    var status: MemoString             # memo: "✓ Valid: ..." or "✗ Empty"
    var input_handler: UInt32
    fn __init__: ctx.create() → use_signal_string("") + use_memo_bool(False)
                 + use_memo_string("✗ Empty") → setup_view(h1 + input[bind_value
                 + oninput_set_string] + 2 × p > dyn_text)
    fn run_memos: if is_valid.is_dirty() → begin_compute → read input
                  → end_compute(len > 0)
                  if status.is_dirty() → begin_compute → read input + is_valid
                  → end_compute("✓ Valid: ..." or "✗ Empty")
    fn render: 2 dyn_text slots (Valid: true/false, Status: ...)
    fn flush: consume_dirty → run_memos → render → diff → finalize
```

Lifecycle: `mf_init` → `mf_rebuild` (run_memos + mount) → `mf_handle_event_string` (oninput_set_string writes input) → `mf_flush` (memos recomputed, DOM diffed). Memo recomputation order: is_valid FIRST (depends on input), then status (depends on input + is_valid). Uses two-way input binding (`bind_value` + `oninput_set_string`). WASM exports: `mf_init`, `mf_destroy`, `mf_rebuild`, `mf_handle_event`, `mf_handle_event_string`, `mf_flush`, `mf_input_text`, `mf_is_valid`, `mf_status_text`, `mf_is_valid_dirty`, `mf_status_dirty`, `mf_set_input`, `mf_input_handler`, `mf_has_dirty`, `mf_scope_count`, `mf_memo_count`.

### MemoChainApp (`src/apps/memo_chain.mojo`) — mixed-type memo chain

```txt
struct MemoChainApp:
    var ctx: ComponentContext           # single root scope
    var input: SignalI32               # input signal
    var doubled: MemoI32               # memo: input * 2
    var is_big: MemoBool               # memo: doubled >= 10
    var label: MemoString              # memo: "BIG" if is_big else "small"
    var incr_handler: UInt32
    fn __init__: ctx.create() → use_signal(0) + use_memo(0) + use_memo_bool(False)
                 + use_memo_string("small") → setup_view(h1 + button + 4 × p > dyn_text)
    fn run_memos: if doubled.is_dirty() → begin_compute → read input → end_compute(input * 2)
                  if is_big.is_dirty() → begin_compute → read doubled → end_compute(doubled >= 10)
                  if label.is_dirty() → begin_compute → read is_big → end_compute("BIG" or "small")
    fn render: 4 dyn_text slots (Input/Doubled/Is Big/Label)
    fn flush: has_dirty → run_memos → settle_scopes → consume_dirty → render → diff → finalize
```

Lifecycle: `mc_init` → `mc_rebuild` (run_memos + mount) → `mc_handle_event` (onclick_add input) → `mc_flush` (memo chain recomputed, settle_scopes filters stable scopes, DOM diffed). Chain: `SignalI32` → `MemoI32` → `MemoBool` → `MemoString`. The runtime automatically propagates dirtiness through memo → memo chains (Phase 36 worklist-based propagation), so each memo checks `is_dirty()` independently. Recomputation order (doubled → is_big → label) is still maintained by code order to ensure upstream values are fresh. Phase 37 equality gating: if a memo recomputes to the same value, its output signal is NOT written and `settle_scopes()` removes scopes that only subscribed to stable signals. WASM exports: `mc_init`, `mc_destroy`, `mc_rebuild`, `mc_handle_event`, `mc_flush`, `mc_input_value`, `mc_doubled_value`, `mc_is_big`, `mc_label_text`, `mc_doubled_dirty`, `mc_is_big_dirty`, `mc_label_dirty`, `mc_incr_handler`, `mc_has_dirty`, `mc_scope_count`, `mc_memo_count`.

### EqualityDemoApp (`src/apps/equality_demo.mojo`) — equality-gated memo chain

```txt
struct EqualityDemoApp:
    var ctx: ComponentContext           # single root scope
    var input: SignalI32               # input signal (create_signal — no scope auto-subscribe)
    var clamped: MemoI32               # memo: clamp(input, 0, 10)
    var label: MemoString              # memo: "high" if clamped > 5 else "low"
    var incr_handler: UInt32
    var decr_handler: UInt32
    fn __init__: ctx.create() → create_signal(0) + use_memo(0) + use_memo_string("low")
                 → setup_view(h1 + 2 × button + 2 × p > dyn_text)
    fn run_memos: if clamped.is_dirty() → begin_compute → read input → clamp(0,10) → end_compute
                  if label.is_dirty() → begin_compute → read clamped → "high"/"low" → end_compute
    fn render: 2 dyn_text slots (Clamped/Label)
    fn flush: has_dirty → run_memos → settle_scopes → consume_dirty → render → diff → finalize
```

Lifecycle: `eq_init` → `eq_rebuild` (run_memos + mount) → `eq_handle_event` (onclick_add/sub input) → `eq_flush` (memo chain recomputed with equality gating, settle_scopes filters stable scopes, DOM diffed only if needed). Chain: `SignalI32(input)` → `MemoI32(clamped)` → `MemoString(label)`. The input signal uses `create_signal` (not `use_signal`) so the scope does NOT auto-subscribe to it — the scope only subscribes to memo outputs (clamped, label) via `use_memo` / `use_memo_string`. When the memo chain is value-stable (e.g. input exceeds the clamp max of 10), `settle_scopes()` removes the scope and flush returns 0 bytes (no mutations, no DOM work). WASM exports: `eq_init`, `eq_destroy`, `eq_rebuild`, `eq_handle_event`, `eq_flush`, `eq_input_value`, `eq_clamped_value`, `eq_label_text`, `eq_clamped_dirty`, `eq_label_dirty`, `eq_clamped_changed`, `eq_label_changed`, `eq_incr_handler`, `eq_decr_handler`, `eq_has_dirty`, `eq_scope_count`, `eq_memo_count`.

### BatchDemoApp (`src/apps/batch_demo.mojo`) — batch signal writes

```txt
struct BatchDemoApp:
    var ctx: ComponentContext           # single root scope
    var first_name: SignalString        # string signal (create_signal_string — no scope auto-subscribe)
    var last_name: SignalString         # string signal (create_signal_string — no scope auto-subscribe)
    var full_name: MemoString           # memo: first_name + " " + last_name
    var write_count: SignalI32          # counts batch operations (use_signal — scope subscribes)
    var set_handler: UInt32
    var reset_handler: UInt32
    fn __init__: ctx.create() → create_signal_string("") × 2 + use_memo_string(" ") + use_signal(0)
                 → setup_view(h1 + 2 × button(onclick_custom) + 2 × p > dyn_text)
    fn run_memos: if full_name.is_dirty() → begin_compute → read first/last → concat → end_compute
    fn set_names(first, last): begin_batch → set first/last + write_count += 1 → end_batch
    fn reset: begin_batch → set first/last="" + write_count=0 → end_batch
    fn render: 2 dyn_text slots (Full/Writes)
    fn flush: has_dirty → run_memos → settle_scopes → consume_dirty → render → diff → finalize
```

Lifecycle: `bd_init` → `bd_rebuild` (run_memos + mount) → `bd_set_names(first, last)` (batch writes 3 signals, single propagation) → `bd_flush` (memo recomputed, DOM diffed). The string signals use `create_signal_string` (not `use_signal_string`) so the scope does NOT auto-subscribe to them — the scope only subscribes to the memo output (full_name) and write_count. `set_names` and `reset` wrap all writes in `begin_batch`/`end_batch` so that only one propagation pass occurs regardless of how many signals are written. WASM exports: `bd_init`, `bd_destroy`, `bd_rebuild`, `bd_handle_event`, `bd_flush`, `bd_set_names`, `bd_reset`, `bd_full_name_text`, `bd_write_count`, `bd_first_name_text`, `bd_last_name_text`, `bd_full_name_dirty`, `bd_full_name_changed`, `bd_has_dirty`, `bd_is_batching`, `bd_set_handler`, `bd_reset_handler`, `bd_scope_count`, `bd_memo_count`.

## WASM Export Pattern (`src/main.mojo`)

All app logic lives in dedicated modules (`src/apps/*.mojo` or `examples/*/*.mojo`). Each module defines:

- The app **struct** (e.g. `BatchDemoApp`)
- Private **lifecycle functions** (e.g. `_bd_init`, `_bd_rebuild`, `_bd_flush`)

`src/main.mojo` imports these and re-exports them as thin `@export` wrappers:

```txt
# In src/apps/batch_demo.mojo:
fn _bd_init() -> UnsafePointer[BatchDemoApp, MutExternalOrigin]: ...
fn _bd_flush(mut app: BatchDemoApp, writer_ptr: ...) -> Int32: ...

# In src/main.mojo:
from apps.batch_demo import BatchDemoApp, _bd_init, ...

@export fn bd_init() -> Int64:  return _to_i64(_bd_init())
@export fn bd_flush(...) -> Int32:  ...alloc writer..._bd_flush(_get[...](ptr)[0], writer)...free writer
@export fn bd_full_name_text(app_ptr: Int64) -> String:
    return _get[BatchDemoApp](app_ptr)[0].full_name.peek()
```

Helpers: `_to_i64(ptr)`, `_get[T](i64) -> UnsafePointer[T]`, `_b2i(Bool) -> Int32`, `_alloc_writer()`, `_free_writer()`.

**Safe reference pattern (Phase 41):** `UnsafePointer` is confined to two places:

1. **`_init` / `_destroy`** — heap allocation/deallocation (inherently pointer-based).
2. **`main.mojo` @export wrappers** — the `Int64` → pointer → reference conversion at the WASM ABI boundary.

All lifecycle functions (`_xx_rebuild`, `_xx_flush`, `_xx_handle_event`, `_xx_resolve`) take `mut app: App` (safe mutable reference) instead of `UnsafePointer[App]`. The `_get[App](ptr)[0]` dereference at the call site in `main.mojo` converts the pointer to a mutable reference. Inside the lifecycle function, the borrow checker tracks `app` as a normal mutable reference — no `app[0].` dereferences needed.

**Note:** `MutationWriter` stays as `UnsafePointer` because `ComponentContext.mount()`, `.diff()`, `.finalize()` and the `CreateEngine`/`DiffEngine` structs store `UnsafePointer[MutationWriter]` as struct fields. Changing this requires lifetime-parameterized structs, which Mojo does not yet support.

`main.mojo` is organized into three sections:

1. **Shared Utilities** — pointer/writer helpers used by all exports
2. **Framework Test & Runtime Exports** — low-level subsystem test exports
3. **App WASM Export Wrappers** — per-app re-export wrappers (grouped by app)

**Naming convention for `launch()`**: The JS `launch({ app: "NAME" })` function discovers WASM exports by prefix — `{NAME}_init`, `{NAME}_rebuild`, `{NAME}_flush` (required), `{NAME}_handle_event` (optional — enables EventBridge dispatch; when missing, EventBridge is a no-op), and `{NAME}_dispatch_string` (optional, enables auto string dispatch for input/change/keydown events). New apps MUST follow this naming convention to be compatible with `launch()`.

## Browser Runtime (`examples/lib/`)

- `app.js` — **`launch(options)`**: Convention-based app launcher (Phase 21, updated Phase 22–24). Given `app: "counter"`, auto-discovers WASM exports by naming convention, sets up interpreter + EventBridge with smart dispatch (auto string dispatch when `{app}_dispatch_string` exists), runs initial mount, and calls optional `onBoot(handle)` for app-specific post-boot wiring. Returns `AppHandle` with `{ fns, appPtr, interp, bufPtr, rootEl, flush }`. Options: `app` (required), `wasm` (required URL), `root` (CSS selector, default `"#root"`), `bufferCapacity` (default 65536), `clearRoot` (default true), `onBoot` (optional callback). **Phase 22**: EventBridge smart dispatch extended to route `keydown` events through `dispatch_string`. **Phase 23**: `{app}_handle_event` made optional — when missing, EventBridge dispatch is a no-op (DOM listeners still attached). **Phase 24.2**: All three apps now use near-zero-config `launch()` (bench only needs `bufferCapacity`; no `onBoot`, no custom root).
- `boot.js` — Re-exports from `app.js`, `env.js`, `events.js`, `interpreter.js`, `protocol.js`, `strings.js`. Low-level API for advanced use cases that need direct control over the boot sequence.
- `env.js` — WASM memory management (size-class free-list allocator with double-free protection and reuse enabled by default) + import object + `loadWasm()` loader.
- `events.js` — `EventBridge` wires interpreter event mutations to a WASM dispatch callback.
- `interpreter.js` — DOM stack machine applying binary mutation buffers (shared with `runtime/interpreter.ts`).
- `protocol.js` — Op constants + `MutationReader` for binary mutation decoding.
- `strings.js` — `writeStringStruct()` writes JS strings into WASM linear memory as Mojo String structs.

**Example main.js files (Phase 23)**:

Counter — zero app-specific JS:

```txt
import { launch } from "../lib/app.js";
launch({ app: "counter", wasm: new URL("../../build/out.wasm", import.meta.url) });
```

Todo — zero app-specific JS (Enter key handled in WASM via `onkeydown_enter_custom`):

```txt
import { launch } from "../lib/app.js";
launch({ app: "todo", wasm: new URL("../../build/out.wasm", import.meta.url) });
```

Bench — launch() with `onBoot` for toolbar wiring & timing (row clicks handled by `bench_handle_event`):

```txt
import { launch } from "../lib/app.js";
launch({
  app: "bench",
  wasm: new URL("../../build/out.wasm", import.meta.url),
  root: "#tbody",
  bufferCapacity: 8 * 1024 * 1024,
  clearRoot: false,
  onBoot: ({ fns, appPtr, rootEl, flush }) => { /* toolbar buttons + timing */ },
});
```

## TypeScript Runtime (`runtime/`)

- `mod.ts` — WASM instantiation entry point.
- `interpreter.ts` — DOM stack machine reading binary mutations.
- `events.ts` — `EventBridge` captures DOM events, dispatches handler IDs to WASM. For `input`/`change` events, extracts `event.target.value` as a string and dispatches via `DispatchWithStringFn` → `writeStringStruct()` → WASM `dispatch_event_with_string` (Phase 20, M20.2). Falls back to numeric then default dispatch. Note: the browser `app.js` EventBridge additionally routes `keydown` events through string dispatch (Phase 22), but the TypeScript runtime does not yet implement this path.
- `templates.ts` — `TemplateCache` registers templates from `RegisterTemplate` mutations.
- `strings.ts` — Mojo `String` ABI (SSO layout: inline ≤23 bytes, heap pointer otherwise).
- `memory.ts` — size-class free-list allocator for WASM linear memory. Tracks every allocation in a JS-side `ptrSize` map and recycles freed blocks via `freeMap` size-class buckets. Double-free protection (pointer removed from `ptrSize` on first free) ensures safe reuse even with WASM code that frees the same pointer twice. Reuse is enabled by default. Includes a scratch arena (`scratchAlloc`/`scratchFreeAll`) for transient `writeStringStruct` allocations.

## Memory Allocator (Phase 25)

All three runtimes use a **JS-side size-class free-list allocator** — no headers in WASM linear memory.

**Why JS-side, not WASM headers?** Some allocations happen during WASM instantiation before JS has a memory reference (can't write headers). Headers also waste 16 bytes per allocation and require slow `DataView` reads on every free. A JS-side `Map.get()` is O(1) with zero WASM memory overhead.

**Design:**

```txt
alignedAlloc(align, size)
  ├─ if reuseEnabled: check freeMap[size] for a cached pointer (O(1) pop)
  │    └─ re-register ptr in ptrSize (so future free works)
  └─ else: bump allocator fallback (O(1))
       └─ record ptr→size in ptrSize Map

alignedFree(ptr)
  ├─ look up size = ptrSize.get(ptr)
  ├─ if not found → ignore (double-free or unknown pointer)
  ├─ delete ptr from ptrSize (prevents double-free stacking)
  └─ push ptr onto freeMap[size] bucket
```

**State:**

- `ptrSize: Map<bigint, bigint>` — every live pointer → its allocation size.
- `freeMap: Map<bigint, bigint[]>` — size-class buckets of freed pointers (LIFO stacks).
- `reuseEnabled: boolean` — gates whether `alignedAlloc` pops from `freeMap` (default: true).

**Double-free protection:** Compiled WASM emits double-free calls (same pointer freed twice) due to Mojo destructor mechanics. `alignedFree` removes the pointer from `ptrSize` on first free, so subsequent frees are detected as "unknown" and silently ignored — no duplicate entries in the free list.

**Scratch arena:** `scratchAlloc` / `scratchFreeAll` for transient `writeStringStruct` allocations. `writeStringStruct()` uses `scratchAlloc`; the arena is bulk-freed after each event dispatch (TS runtime) or flush cycle (JS browser runtime). WASM consumes string data synchronously before free.

**Mojo harness:** `SharedState.aligned_alloc` / `aligned_free` in `test/wasm_harness.mojo` mirrors the JS design using `Dict[Int, Int]` for `ptr_size` and `Dict[Int, List[Int]]` for `free_map`.

**Mutation buffer zero-init:** `mutation_buf_alloc` in `src/main.mojo` calls `memset_zero` on the allocated buffer because reused blocks may contain stale protocol data (OP_END = 0x00 must be the default).

## String Event Dispatch (Phase 20)

Phase 20 adds the infrastructure for passing string values from DOM events to WASM `SignalString` signals, culminating in Dioxus-style two-way input binding. Phase 20.5 completes the story by migrating TodoApp to a fully WASM-driven Add flow.

**Dispatch path (M20.1 Mojo + M20.2 JS)**: JS EventBridge `handleEvent()` → for `input`/`change` events: extract `event.target.value` → `writeStringStruct(value)` → `dispatchWithStringFn(hid, eventType, stringPtr)` → WASM `dispatch_event_with_string(rt, handler_id, event_type, string_ptr)` → Runtime looks up handler → for `ACTION_SIGNAL_SET_STRING`: `write_signal_string(string_key, version_key, value)` → bumps version signal → marks subscriber scopes dirty. If string dispatch returns 0: try numeric fallback (`parseInt` + `dispatchWithValueFn`), then default no-payload dispatch. Non-input events (click, keydown, etc.) bypass string dispatch entirely.

**JS wiring (M20.2)**: `EventBridge.setDispatch(dispatch, dispatchWithValue?, dispatchWithString?)` — third parameter enables string dispatch. `AppConfig.handleEventWithString` optional callback; `createApp()` wires it to `EventBridge.dispatchWithStringFn` when provided. `DispatchWithStringFn` type: `(handlerId, eventType, stringPtr) => number`.

**Handler encoding**: `HandlerEntry.signal_set_string(scope_id, string_key, version_key, event_name)` repurposes existing fields — `signal_key` holds the `string_key` (StringStore index), `operand` holds the `version_key` (cast to Int32).

**WASM exports**: `handler_register_signal_set_string`, `dispatch_event_with_string`, `shell_dispatch_event_with_string`, `signal_create_string` (returns packed i64), `signal_string_key`, `signal_version_key`, `signal_peek_string`, `signal_write_string`, `signal_string_count`.

**DSL helpers (M20.3)**: `oninput_set_string(signal: SignalString) -> Node` creates a `NODE_EVENT` for `"input"` with `ACTION_SIGNAL_SET_STRING`. `onchange_set_string(signal: SignalString) -> Node` does the same for `"change"`. Both store `string_key` in `dynamic_index` and `Int32(version_key)` in `operand`, matching `HandlerEntry.signal_set_string()` encoding. Processed by `register_view()` / `setup_view()` which auto-assigns dyn_attr indices and registers handlers.

**Value binding (M20.4)**: `NODE_BIND_VALUE` node kind (tag 7) carries a SignalString reference (attr_name in `text`, string_key in `dynamic_index`, version_key in `operand`). `bind_value(signal: SignalString) -> Node` creates one with `attr_name="value"`; `bind_attr(attr_name, signal) -> Node` supports arbitrary attribute names. `_process_view_tree()` handles `NODE_BIND_VALUE` like `NODE_EVENT` — collects `_ValueBindingInfo` and replaces with `NODE_DYN_ATTR`. New `AutoBinding` tagged union (`AUTO_BIND_EVENT` / `AUTO_BIND_VALUE`) stores both events and value bindings in tree-walk order. `register_view()` interleaves them by `attr_idx`. `RenderBuilder.build()` auto-populates: events via `add_dyn_event()`, value bindings via `peek_signal_string()` + `add_dyn_text_attr()`. Falls back to legacy `EventBinding` path when no auto-bindings present.

**Custom inline handler (M20.5)**: `onclick_custom() -> Node` creates a `NODE_EVENT` for `"click"` with `ACTION_CUSTOM` (value 255), `signal_key=0`, `operand=0`. When dispatched, the runtime marks the scope dirty and returns False — the app's event handler then performs custom routing based on the handler ID. Use `ctx.view_event_handler_id(index)` after `register_view()` to retrieve the auto-registered handler ID.

**Two-way binding + custom action pattern (M20.3 + M20.4 + M20.5)**:

```mojo
el_input(
    attr("type", "text"),
    bind_value(input_text),          # M20.4: value attr ← signal
    oninput_set_string(input_text),   # M20.3: signal ← input event
),
el_button(text("Add"), onclick_custom()),  # M20.5: custom action in WASM
```

Equivalent Dioxus: `input { value: "{text}", oninput: move |e| text.set(e.value()) }` + `button { onclick: move |_| { add(&text); text.set(""); }, "Add" }`

**view_event_handler_id (M20.5)**: `ctx.view_event_handler_id(index: Int) -> UInt32` returns the handler ID for the Nth event registered by `register_view()` in tree-walk order. Example: after `register_view(el_div(el_input(bind_value(sig), oninput_set_string(sig)), el_button(text("Add"), onclick_custom()), ...))`, `view_event_handler_id(0)` = oninput handler, `view_event_handler_id(1)` = Add button handler.

**Keydown Enter handler (Phase 22)**: `onkeydown_enter_custom() -> Node` creates a `NODE_EVENT` for `"keydown"` with `ACTION_KEY_ENTER_CUSTOM` (value 7), `signal_key=0`, `operand=0`. When dispatched via `dispatch_event_with_string()`, the runtime checks the string payload (the key name from `event.key`) — only `"Enter"` triggers the action (marks scope dirty, returns True); all other keys are silently ignored (returns False). The app's `handle_event()` then performs custom routing based on the handler ID, same as `ACTION_CUSTOM`. Use `ctx.view_event_handler_id(index)` after `register_view()` to retrieve the auto-registered handler ID.

**JS keydown dispatch (Phase 22)**: The `launch()` EventBridge in `app.js` routes `keydown` events through `dispatch_string` when `{app}_dispatch_string` exists. It sends `event.key` as the string payload. If the WASM handler accepts the key (returns 1 — e.g. `ACTION_KEY_ENTER_CUSTOM` matched "Enter"), the bridge also calls `handle_event` for app-level routing. If rejected (returns 0), no further dispatch occurs. This two-step dispatch (string filter → app routing) enables WASM-driven keyboard shortcuts with zero app-specific JS.

**Phase 22 TodoApp pattern (Enter key + Add button)**: `view_event_handler_id(0)` = oninput handler, `view_event_handler_id(1)` = Enter key handler, `view_event_handler_id(2)` = Add button handler. Both Enter key and Add button handler IDs are checked in `handle_event()` to trigger the same Add logic. The template uses:

```mojo
el_input(
    attr("type", "text"),
    bind_value(input_text),
    oninput_set_string(input_text),
    onkeydown_enter_custom(),
),
el_button(text("Add"), onclick_custom()),
```

## Node Kind Tags (`src/vdom/dsl.mojo`)

| Tag | Value | Description |
|-----|-------|-------------|
| `NODE_TEXT` | 0 | Static text content |
| `NODE_ELEMENT` | 1 | HTML element with tag, children, attrs |
| `NODE_DYN_TEXT` | 2 | Dynamic text placeholder (slot index) |
| `NODE_DYN_NODE` | 3 | Dynamic node placeholder (slot index) |
| `NODE_STATIC_ATTR` | 4 | Static attribute (name + value) |
| `NODE_DYN_ATTR` | 5 | Dynamic attribute placeholder (slot index) |
| `NODE_EVENT` | 6 | Inline event handler (action + signal + operand) |
| `NODE_BIND_VALUE` | 7 | Value binding (SignalString → dynamic attr) (Phase 20, M20.4) |

## Handler Action Tags (`src/events/registry.mojo`)

| Tag | Value | Description |
|-----|-------|-------------|
| `ACTION_NONE` | 0 | No-op (marks scope dirty) |
| `ACTION_SIGNAL_SET_I32` | 1 | `signal.set(operand)` |
| `ACTION_SIGNAL_ADD_I32` | 2 | `signal += operand` |
| `ACTION_SIGNAL_SUB_I32` | 3 | `signal -= operand` |
| `ACTION_SIGNAL_TOGGLE` | 4 | `signal.set(!signal.get())` |
| `ACTION_SIGNAL_SET_INPUT` | 5 | `signal.set(input_value)` |
| `ACTION_SIGNAL_SET_STRING` | 6 | `string_signal.set(string_value)` (Phase 20) |
| `ACTION_KEY_ENTER_CUSTOM` | 7 | Fires only when key == "Enter" (Phase 22) |
| `ACTION_CUSTOM` | 255 | No Mojo-side action; marks scope dirty for app routing |

## File Size Reference

| File | Lines | Role |
|------|-------|------|
| `src/main.mojo` | ~6,730 | Shared utilities + framework test exports + thin @export wrappers for all apps |
| `src/apps/` (15 modules) | ~3,660 | Demo/test app structs + lifecycle functions (Phase 40 extraction) |
| `src/signals/handle.mojo` | ~980 | SignalI32 + SignalBool + SignalString + MemoI32 + MemoBool + MemoString + EffectHandle |
| `src/signals/memo.mojo` | ~458 | MemoEntry + MemoStore (value_changed flag, Phase 37) |
| `src/signals/runtime.mojo` | ~1,850 | Reactive runtime + SignalStore + StringStore + memo bool/string methods + worklist propagation (Phase 36) + equality-gated end_compute + settle_scopes + _changed_signals (Phase 37) + batch signal writes (Phase 38) |
| `src/component/context.mojo` | ~2,295 | ComponentContext + RenderBuilder + tree processing + error boundary + view_event_handler_id + memo bool/string hooks + settle_scopes wrapper + batch wrappers |
| `src/component/child_context.mojo` | ~570 | ChildComponentContext (child scope API + error boundary methods + memo bool/string hooks) |
| `src/component/app_shell.mojo` | ~530 | AppShell (low-level + memo bool/string wrappers + settle_scopes wrapper + batch wrappers) |
| `examples/counter/counter.mojo` | ~115 | Counter app |
| `examples/todo/todo.mojo` | ~520 | Todo app (M20.5: WASM-driven Add, bind_value, oninput_set_string, onclick_custom) |
| `examples/bench/bench.mojo` | ~985 | Benchmark app (uses KeyedList + ItemBuilder + performance_now timing + 3 dyn_text status bar) |
| `src/component/keyed_list.mojo` | ~670 | KeyedList + ItemBuilder + HandlerAction |
| `src/vdom/dsl.mojo` | ~3,630 | Node DSL + el_* helpers + multi-arg overloads + conditional helpers + onclick_custom + to_template |
| `src/vdom/vnode.mojo` | ~800 | VNode + VNodeStore + VNodeBuilder |
| `src/mutations/diff.mojo` | ~970 | DiffEngine (keyed reconciliation) |
| `runtime/memory.ts` | ~290 | Free-list allocator + scratch arena (Phase 25) |
| `runtime/events.ts` | ~375 | EventBridge + DispatchWithStringFn (M20.2) |
| `runtime/app.ts` | ~2,670 | createApp + app handles (Counter, Todo, Bench, SafeCounter, ErrorNest, DataLoader, SuspenseNest, EffectDemo, EffectMemo, MemoForm, MemoChain, EqualityDemo, BatchDemo, etc.) |
| `runtime/types.ts` | ~690 | WasmExports interface (Phase 20 string dispatch exports) |
| `examples/lib/env.js` | ~250 | Browser free-list allocator + WASM imports (Phase 25) |
| `test-js/allocator.test.ts` | ~980 | Allocator unit tests + WASM-integrated reuse tests (Phase 25) |
| `test-js/events.test.ts` | ~650 | EventBridge string dispatch tests (unit + WASM integration) |
| `test-js/dsl.test.ts` | ~620 | DSL tests incl. M20.3/M20.4/M20.5 binding + onclick_custom tests |
| `test-js/todo.test.ts` | ~1,060 | Todo app tests incl. M20.5 WASM-driven Add flow tests |
| `test-js/context.test.ts` | ~280 | ContextTestApp provide/consume tests (Phase 31.1) |
| `test-js/child_context.test.ts` | ~480 | ChildContextTestApp self-rendering child tests (Phase 31.2) |
| `test-js/child_component.test.ts` | ~710 | ChildComponent low-level tests |
| `test-js/props_counter.test.ts` | ~550 | PropsCounterApp props + self-rendering child tests (Phase 31.3) |
| `test-js/theme_counter.test.ts` | ~690 | ThemeCounterApp shared context + upward communication tests (Phase 31.4) |
| `test-js/safe_counter.test.ts` | ~600 | SafeCounterApp error boundary tests (crash/retry lifecycle, DOM) |
| `test-js/error_nest.test.ts` | ~645 | ErrorNestApp nested boundary tests (inner/outer crash/retry, DOM) |
| `test-js/data_loader.test.ts` | ~500 | DataLoaderApp suspense tests (load/resolve lifecycle, DOM) |
| `test-js/suspense_nest.test.ts` | ~690 | SuspenseNestApp nested suspense tests (inner/outer load/resolve, DOM) |
| `test-js/effect_demo.test.ts` | ~560 | EffectDemoApp effect-in-flush tests (derived state, DOM, heapStats) |
| `test-js/effect_memo.test.ts` | ~495 | EffectMemoApp memo+effect chain tests (threshold, derived state, DOM) |
| `test/test_effect_demo.mojo` | ~475 | EffectDemoApp Mojo tests (18 tests: lifecycle, derived state, rapid) |
| `test/test_effect_memo.mojo` | ~455 | EffectMemoApp Mojo tests (16 tests: memo chain, threshold, rapid) |
| `test/test_memo_bool.mojo` | ~694 | MemoBool Mojo tests (15 tests: create, peek, dirty, recompute, hooks) |
| `test/test_memo_string.mojo` | ~831 | MemoString Mojo tests (17 tests: create, peek, dirty, recompute, lifecycle, hooks) |
| `test/test_memo_form.mojo` | ~514 | MemoFormApp Mojo tests (18 tests: lifecycle, derived state, dirty tracking, form validation) |
| `test/test_memo_propagation.mojo` | ~1,341 | Recursive memo propagation Mojo tests (20 tests: chain depth, diamond, mixed types, scope/effect at end) |
| `test/test_memo_chain.mojo` | ~703 | MemoChainApp Mojo tests (22 tests: memo chain, threshold, propagation, Phase 36 independent dirty) |
| `test/test_memo_equality.mojo` | ~1,506 | Equality-gated memo propagation Mojo tests (22 tests: I32/Bool/String equality gates, value_changed flag, _changed_signals, chain cascades, diamond) |
| `test/test_equality_demo.mojo` | ~886 | EqualityDemoApp Mojo tests (20 tests: clamp stabilization, threshold crossing, zero-byte flush, scope settling, round-trip) |
| `test/test_scope_settle.mojo` | ~1,418 | Scope settle Mojo tests (16 tests: stable/changed scopes, mixed, chain cascade, diamond, effects, idempotent) |
| `test-js/memo_form.test.ts` | ~495 | MemoFormApp JS tests (20 suites: DOM, input binding, memo dirty/clean, form validation) |
| `test-js/memo_chain.test.ts` | ~582 | MemoChainApp JS tests (24 suites: DOM, chain propagation, threshold, heapStats, Phase 36 independent dirty) |
| `test-js/equality_demo.test.ts` | ~490 | EqualityDemoApp JS tests (22 suites: DOM, clamp/label stability, flush returns 0, round-trip, dirty state) |
| `test/test_batch.mojo` | ~1,350 | Batch signal writes Mojo tests (22 tests: single/multi signal, deferred propagation, nested batches, string signals, chain propagation, settle) |
| `test/test_batch_demo.mojo` | ~718 | BatchDemoApp Mojo tests (19 tests: lifecycle, set_names, reset, memo dirty/stable, write_count, rapid sets, dirty flag) |
| `test-js/batch_demo.test.ts` | ~452 | BatchDemoApp JS tests (20 suites: DOM, set/reset cycle, multiple sets, write count, memo stable, batching flag, independent instances, rapid sets) |
| `test/wasm_harness.mojo` | ~1,400 | Mojo WASM test harness (includes free-list allocator, Phase 25) |
| `CHANGELOG.md` | ~530 | Development history (Phases 0–40) |

## Common Patterns

**Effect drain-and-run pattern (Phase 34):** Effects run between `consume_dirty()` and `render()` to settle derived state before rendering. The effect's `begin_run()` / `end_run()` bracket establishes a reactive context so signal reads during the effect body are tracked as dependencies:

```text
fn flush():
    if not ctx.consume_dirty(): return 0
    run_effects()       # drain pending effects → may write signals
    var idx = render()  # all derived state settled
    ctx.diff(writer, idx)
    return ctx.finalize(writer)

fn run_effects():
    if effect.is_pending():
        effect.begin_run()
        var val = source_signal.read()   # re-subscribe
        derived_signal.set(compute(val)) # write derived state
        effect.end_run()
```

**Effect + memo chain pattern (Phase 34):** When effects depend on memos, recompute memos FIRST, then run effects. Memo recomputation may change the output signal, which marks dependent effects pending. After both memos and effects have run, call `settle_scopes()` (Phase 37) to remove scopes whose subscribed signals are all value-stable before consuming dirty scopes:

```text
fn run_memos_and_effects():
    # Step 1: Recompute dirty memos
    if memo.is_dirty():
        memo.begin_compute()
        var input = input_signal.read()  # re-subscribe memo
        memo.end_compute(input * 3)
    # Step 2: Run pending effects (may read memo output)
    if effect.is_pending():
        effect.begin_run()
        var t = memo.read()              # re-subscribe effect to memo
        label.set("small" if t < 10 else "big")
        effect.end_run()

fn flush():
    if not ctx.has_dirty(): return 0
    run_memos_and_effects()
    ctx.settle_scopes()     # Phase 37: filter stable scopes
    if not ctx.has_dirty(): return 0
    _ = ctx.consume_dirty()
    ...
```

**Suspense flush pattern (Phase 33):** Check `ctx.is_pending()` in flush to switch between content and skeleton children. Uses the same `flush` / `flush_empty` alternation as error boundaries:

```text
if ctx.is_pending():
    content_child.flush_empty(writer)      # hide content
    skeleton_child.flush(writer, skel_vnode) # show skeleton
else:
    skeleton_child.flush_empty(writer)     # hide skeleton
    content_child.flush(writer, content_vnode) # show content with data
```

JS triggers resolution via a WASM export that calls `ctx.set_pending(False)` and stores the loaded data. The next flush cycle restores the content child with the resolved data.

**Error boundary flush pattern (Phase 32):** Check `ctx.has_error()` in flush to switch between normal and fallback children. Uses the same `flush` / `flush_empty` alternation as `ConditionalSlot`:

```text
if ctx.has_error():
    normal_child.flush_empty(writer)       # hide normal content
    fallback_child.flush(writer, fb_vnode) # show fallback with error message
else:
    fallback_child.flush_empty(writer)     # hide fallback
    normal_child.flush(writer, child_vnode) # show normal content
```

Error propagation: `ctx.report_error(msg)` walks the scope parent chain via `ScopeArena.propagate_error()` to the nearest `is_error_boundary` ancestor, sets the error, and marks the boundary dirty. `ctx.clear_error()` clears the error and marks dirty. No new JS runtime infrastructure — fallback UI renders through the same mutation protocol.

**String event dispatch (Phase 20 — manual)**: Register a handler with `HandlerEntry.signal_set_string(scope_id, signal.string_key, signal.version_key, String("input"))`, then dispatch from JS via `dispatch_event_with_string(rt, handler_id, event_type, string_value)`. The runtime writes the string to the `SignalString` and bumps the version signal.

**Inline string event binding (Phase 20 — M20.3)**: `oninput_set_string(signal)` / `onchange_set_string(signal)` create `NODE_EVENT` nodes with `ACTION_SIGNAL_SET_STRING`. Used with `register_view()` / `setup_view()` for automatic handler registration: `el_input(oninput_set_string(name))`.

**Two-way input binding (Phase 20 — M20.3 + M20.4)**: Combine `bind_value(signal)` (auto-populates `value` attribute at render time) with `oninput_set_string(signal)` (writes input value back to signal): `el_input(attr("type", "text"), bind_value(text), oninput_set_string(text))`. The `RenderBuilder.build()` reads the signal and emits the `value` attr automatically. For custom attribute names, use `bind_attr("placeholder", signal)`.

**Adding a signal to a component**: `var foo = self.ctx.use_signal(0)` in setup, `foo.peek()` to read, `foo += 1` or `foo.set(v)` to write.

**Adding a bool signal**: `var flag = self.ctx.use_signal_bool(False)` in setup, `flag.get()` to read, `flag.set(True)` or `flag.toggle()` to write.

**Adding a string signal**: `var name = self.ctx.use_signal_string(String("hello"))` in setup, `name.get()` / `name.peek()` to read, `name.set(String("world"))` to write, `name.read()` to read with subscription, `name.is_empty()` to check, `String(name)` for interpolation. For non-reactive string state (write-buffer), use `ctx.create_signal_string(initial)` instead — no hook registration, no scope subscription (see TodoApp `input_text`).

**Bump version signal**: `self.version += 1` (triggers re-render via scope subscription).

**Inline events in DSL**: `el_button(text("Up!"), onclick_add(count, 1))` — multi-arg overloads, extracted by `register_view()` / `setup_view()`.

**Inline custom events (M20.5)**: `el_button(text("Add"), onclick_custom())` — creates NODE_EVENT with ACTION_CUSTOM, auto-registered by `register_view()`. Retrieve handler ID via `ctx.view_event_handler_id(index)` for app-specific routing.

**Manual events**: `var hid = ctx.register_handler(HandlerEntry.custom(scope_id, "click"))`, then `vb.add_dyn_event("click", hid)`.

**Keyed list rebuild (Phase 17+18 — via ItemBuilder)**: `var frag = self.items.begin_rebuild(ctx)` → for each item: `var ib = items.begin_item(key, ctx)` → `ib.add_dyn_text(...)` → `ib.add_custom_event("click", ACTION_TAG, item_id)` → `ib.add_class_if(condition, "class")` → `items.push_child(ctx, frag, ib.index())`.

**Conditional helpers (Phase 18)**: `class_if(cond, "name")` → `"name"` or `""`. `class_when(cond, "a", "b")` → `"a"` or `"b"`. `text_when(cond, "yes", "no")` → conditional text. `ib.add_class_if(cond, "name")` → one-call shortcut on ItemBuilder/RenderBuilder.

**String signal in render (Phase 19)**: `vb.add_dyn_text_signal(name)` → reads `name.get()` and adds as dynamic text. Works on both `RenderBuilder` and `ItemBuilder`.

**Memo type expansion pattern (Phase 35):** `MemoBool` and `MemoString` mirror the signal type expansion (Phase 18–19). `MemoBool` wraps an Int32 memo entry with boolean ergonomics; `MemoString` wraps a StringStore slot + version memo. Memo recomputation order is critical for correctness in chains — recompute upstream memos before downstream memos that read their output. The runtime automatically propagates dirtiness through memo → memo chains (Phase 36 worklist-based propagation), so each memo checks `is_dirty()` independently:

```text
fn run_memos():
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
        self.label.end_compute("BIG" if big else "small")
```

**Worklist-based memo propagation (Phase 36):** `write_signal` uses a two-phase approach to propagate dirtiness through memo → memo chains to arbitrary depth. Phase 1 scans the written signal's direct subscribers — memos are marked dirty and added to a worklist; effects are marked pending; scopes are added to `dirty_scopes`. Phase 2 drains the worklist: for each memo, its output signal's subscribers are scanned with the same memo/effect/scope classification. The `is_dirty()` check serves as a cycle guard — a memo already dirty is not re-added to the worklist, guaranteeing termination (each memo processed at most once). Diamond dependencies (signal → A, signal → B, A+B → C) are handled correctly: C is marked dirty once when the first parent is processed, then skipped when the second parent is processed. Scope reactive contexts are tagged with `SCOPE_CONTEXT_TAG` (bit 31) to prevent false matches against memo/effect context IDs which are bare signal keys.

**Equality-gated memo propagation (Phase 37):** All three `memo_end_compute_*` methods (I32, Bool, String) compare old vs new value before writing. If the value is unchanged (value-stable), the output signal is NOT written — so downstream memos that read it see the same value and can themselves be value-stable. The `MemoEntry.value_changed` flag records whether the last `end_compute` produced a new value. The runtime's `_changed_signals: List[UInt32]` accumulates signal keys whose values actually changed during a flush cycle — populated by `write_signal` (source signals always change) and `end_compute` (only when new != old). `settle_scopes()` uses this set to remove eagerly-dirtied scopes whose subscribed signals are all value-stable:

```text
fn flush():
    if not ctx.has_dirty(): return 0
    run_memos()              # recompute memo chain (sets _changed_signals)
    ctx.settle_scopes()     # remove scopes with no actual signal changes
    if not ctx.has_dirty(): return 0   # all scopes settled → skip render
    _ = ctx.consume_dirty() # drain remaining dirty scopes via scheduler
    var idx = render()
    ctx.diff(writer, idx)
    return ctx.finalize(writer)
```

Key design points: (1) `settle_scopes()` must run BEFORE `consume_dirty()` because `consume_dirty()` drains `dirty_scopes`; (2) `_changed_signals` is cleared at the end of `settle_scopes()` to prepare for the next flush cycle; (3) the algorithm is O(C × avg_subscribers × D) where C = changed signals, avg_subscribers ≈ 1–3, D = dirty scopes — efficient for typical apps; (4) scopes that subscribe directly to source signals (via `use_signal`) will always be kept dirty when those signals change — use `create_signal` (no auto-subscribe) for signals that feed into memo chains where the scope should only react to memo output changes.

**Batch signal writes (Phase 38):** `begin_batch()` / `end_batch()` group multiple signal writes into a single propagation pass. During a batch, signal values are stored immediately (reads see the new value) but subscriber scanning and worklist propagation are deferred. The outermost `end_batch()` runs a combined propagation pass over all written signals using a single shared worklist — a memo that subscribes to multiple batched signals is added to the worklist at most once (`is_dirty()` deduplicates). Batches can be nested (depth counter); only the outermost `end_batch()` triggers propagation. String signals bump their version key immediately during batch but defer propagation of that version key:

```text
fn set_names(mut self, first: String, last: String):
    self.ctx.begin_batch()
    self.first_name.set(first)   # stores value, defers propagation
    self.last_name.set(last)     # stores value, defers propagation
    self.write_count += 1        # stores value, defers propagation
    self.ctx.end_batch()         # single combined propagation pass
```

**MemoString lifecycle:** `MemoString` uses the same dual-key pattern as `SignalString`: a `string_key` in `StringStore` for heap-safe string storage, and a `version_key` in `MemoStore` for dirty/version tracking. `end_compute(String)` writes to both: the string value to `StringStore` and the version to the memo entry. `read()` subscribes via the version memo. Cleanup requires `destroy_memo_string()` to free the StringStore slot.

**Keyed list dispatch (Phase 17 — via HandlerAction)**: `var action = self.items.get_action(handler_id)` → `if action.found: match action.tag`.

**Keyed list rebuild (Phase 16 — manual)**: `var frag = self.items.begin_rebuild(ctx)` → for each item: `items.create_scope(ctx)` → `items.item_builder(key, ctx)` → register handlers → `items.push_child(ctx, frag, idx)`.

**Keyed list flush (via KeyedList)**: `items.flush(ctx, writer, frag_idx)` + `writer.finalize()`.

**Flush lifecycle**: `if not ctx.consume_dirty(): return 0` → rebuild → `ctx.flush(writer, new_idx)` or `items.flush(ctx, writer, frag_idx)` + `writer.finalize()`.

**Combined flush (M20.5 TodoApp)**: When the app shell has dynamic bindings (bind_value) AND a KeyedList: `ctx.consume_dirty()` → `render()` → `ctx.diff(writer, new_app_idx)` (catches bind_value changes) → `items.flush(ctx, writer, new_frag_idx)` → `writer.finalize()`. The diff emits SetAttribute for changed value bindings; dyn_node(0) stays as placeholder (diff no-ops, KeyedList manages content separately).

## Deferred Abstractions (Blocked on Mojo Roadmap)

- **`UnsafePointer[MutationWriter]` in component infrastructure** → blocked on Lifetime-parameterized structs. `ComponentContext.mount()`, `.diff()`, `.finalize()` and the `CreateEngine`/`DiffEngine` structs all store `UnsafePointer[MutationWriter]` as struct fields because Mojo does not yet support borrowing a reference into a struct field with a lifetime parameter. Once Mojo gains `struct Foo[lt: Lifetime] { ref [lt] writer: MutationWriter }`, this can be refactored to pass `MutationWriter` by safe reference throughout the component infrastructure.
- **Closure event handlers** → blocked on Lambda syntax + Closure refinement (Phase 1, 🚧). Would eliminate `ItemBuilder.add_custom_event()` + `get_action()`. Phase 20 string dispatch + inline DSL helpers (`oninput_set_string`, `bind_value`) address this for input events. **0.26.1 progress:** Function type conversions improved (non-raising → raising, ref → value), but true closures/function pointers in WASM still missing.
- **`rsx!` macro** → blocked on Hygienic importable macros (Phase 2, ⏰). Would enable compile-time DSL like Dioxus. **0.26.1 progress:** None.
- **`for` loops in views** → blocked on macros (Phase 2, ⏰). Currently iteration happens in build functions. **0.26.1 progress:** None.
- **Generic `Signal[T]`** → blocked on Conditional conformance (Phase 1, 🚧). Currently `SignalI32` / `SignalBool` / `SignalString` / `MemoI32` / `MemoBool` / `MemoString` (Phase 18 added `SignalBool`, Phase 19 added `SignalString`, Phase 35 added `MemoBool` + `MemoString` — three memo types now match three signal types, reducing urgency). **0.26.1 progress:** Experimental `conforms_to()` + `trait_downcast()` enable static dispatch on trait conformance; expanded reflection (`struct_field_count`, `struct_field_names`, `struct_field_types`) enables field introspection. Still blocked on full conditional conformance for parametric stores.
- **Dynamic component dispatch** → blocked on Existentials / dynamic traits (Phase 2, ⏰). **0.26.1 progress:** `AnyType` no longer requires `__del__()` (explicitly-destroyed types help), but doesn't solve dispatch.
- **Pattern matching on actions** → blocked on ADTs & pattern matching (Phase 2, ⏰). Currently `if/elif` chains. **0.26.1 progress:** None.
- ~~**Async data loading / suspense**~~ → **Suspense (simulated) implemented in Phase 33.** True async/await still blocked on First-class async (Phase 2, ⏰), but synchronous suspense with JS-triggered resolve is now available. `use_suspense_boundary()` marks a scope as a suspense boundary; `set_pending(True/False)` toggles pending state; `is_pending()` drives flush-time content/skeleton switching. JS resolves by calling a WASM export that stores data and clears pending. Demonstrated with DataLoaderApp (single boundary) and SuspenseNestApp (nested boundaries).
- ~~**Error boundaries**~~ → **Implemented in Phase 32.** Scope-level error boundary infrastructure (Phase 8.4) is now surfaced on `ComponentContext` and `ChildComponentContext` with `use_error_boundary()`, `report_error()`, `has_error()`, `error_message()`, `clear_error()`. Demonstrated with SafeCounterApp (single boundary) and ErrorNestApp (nested boundaries).