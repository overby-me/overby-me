# Phase 3: Create `mojo-gui/desktop` (Desktop Webview Renderer) ✅

> **Status:** Complete (infrastructure ✅, unified lifecycle ✅)
>
> Back to [SEPARATION_PLAN.md](../../SEPARATION_PLAN.md) · See also: [Architecture](architecture.md) · [Renderers](renderers.md) · [Checklist](checklist.md)

---

## Step 3.1 — Design the desktop webview architecture ✅

The webview approach was chosen as a pragmatic first step, following the same evolution Dioxus took (webview → Blitz). It reuses the existing JS mutation interpreter inside a native GTK4 window with an embedded WebKitGTK webview.

```text
┌─ Native Mojo Process ─────────────────────────────────────────────┐
│                                                                    │
│  User App Code (counter.mojo, todo.mojo, ...)                     │
│      │                                                             │
│      ▼                                                             │
│  mojo-gui/core (compiled native — NOT WASM)                       │
│    ├── Signals, Memos, Effects                                     │
│    ├── Virtual DOM + Diff Engine                                   │
│    ├── MutationWriter → heap buffer                                │
│    └── HandlerRegistry (event dispatch)                            │
│         │                            ▲                             │
│         │ mutations (binary)         │ events (JSON)               │
│         ▼                            │                             │
│  DesktopBridge                                                     │
│    ├── Owns heap-allocated mutation buffer (64 KiB)                │
│    ├── flush_mutations() → base64 → webview eval                   │
│    └── poll_event() ← JSON ← ring buffer ← JS                    │
│         │                            ▲                             │
│         ▼                            │                             │
│  ┌─ Embedded Webview (GTK4 + WebKitGTK) ──────────────────────┐   │
│  │  desktop-runtime.js                                         │   │
│  │    ├── MutationReader (decodes base64 → binary protocol)    │   │
│  │    ├── Interpreter (applies mutations to real DOM)           │   │
│  │    ├── TemplateCache (DocumentFragment cloning)              │   │
│  │    └── Event dispatch → window.mojo_post(JSON)              │   │
│  │  shell.html  <div id="root"></div>                          │   │
│  └─────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────┘
```

## Step 3.2 — Implement the C shim (`libmojo_webview.so`) ✅

Built a C shim (`shim/mojo_webview.c`) wrapping GTK4 + WebKitGTK with a Mojo-friendly **polling** API. Key design decisions:

- **No function-pointer callbacks** — Mojo's FFI (`DLHandle`) cannot easily pass managed closures as C function pointers. Instead, JS sends events via `window.mojo_post(json)` into a ring buffer, and Mojo polls with `mwv_poll_event()`.
- **Ring buffer for events** — capacity 256 events × 4 KiB each, FIFO with oldest-drop overflow.
- **Base64 mutation delivery** — `mwv_apply_mutations(buf, len)` base64-encodes the binary buffer and calls `window.__mojo_apply_mutations(base64)` in the webview.
- **Non-blocking step API** — `mwv_step(w, blocking)` allows cooperative event loop interleaving.

**C shim API surface (`shim/mojo_webview.h`):**

| Category    | Functions                                                    |
|-------------|--------------------------------------------------------------|
| Lifecycle   | `mwv_create(title, w, h, debug)`, `mwv_destroy(w)`          |
| Window      | `mwv_set_title(w, title)`, `mwv_set_size(w, w, h, hints)`   |
| Content     | `mwv_set_html(w, html)`, `mwv_navigate(w, url)`, `mwv_init(w, js)`, `mwv_eval(w, js)` |
| Event loop  | `mwv_run(w)`, `mwv_step(w, blocking)`, `mwv_terminate(w)`   |
| Events      | `mwv_poll_event(w, buf, len)`, `mwv_event_count(w)`, `mwv_event_clear(w)` |
| Mutations   | `mwv_apply_mutations(w, buf, len)` — base64-encode + eval    |
| Diagnostics | `mwv_is_alive(w)`, `mwv_get_window(w)`                       |

Nix derivation (`shim/default.nix`) automates the build and provides the library path via `MOJO_WEBVIEW_LIB`.

## Step 3.3 — Implement Mojo FFI bindings (`webview.mojo`) ✅

Created `desktop/src/desktop/webview.mojo` with typed Mojo wrappers around the C shim API via `OwnedDLHandle`. The `Webview` struct provides:

- `create(title, width, height, debug)` — open a window
- `set_html(html)` / `init_js(js)` / `eval_js(js)` — content injection
- `step(blocking)` / `run()` — event loop control
- `poll_event()` — drain events from the ring buffer
- `apply_mutations(buf, len)` — send mutation buffer to JS interpreter
- Library search: `MOJO_WEBVIEW_LIB` env var → `NIX_LDFLAGS` → `LD_LIBRARY_PATH` → common paths

## Step 3.4 — Implement the desktop bridge (`bridge.mojo`) ✅

Created `desktop/src/desktop/bridge.mojo` with:

- **`DesktopBridge`** struct — owns a heap-allocated mutation buffer (64 KiB default), provides `buf_ptr()` for `MutationWriter`, `flush_mutations(len)` to send to webview, `poll_event()` to drain events.
- **`DesktopEvent`** struct — parsed event with `handler_id`, `event_type`, optional `value` string.
- **`parse_event(json)`** — minimal JSON parser for the `{"h":42,"t":0,"v":"..."}` format.

## Step 3.5 — Implement `DesktopApp` (`app.mojo`) ✅

Created `desktop/src/desktop/app.mojo` with the `DesktopApp` struct that orchestrates:

1. Webview creation and JS runtime injection
2. HTML shell loading (inline `SHELL_HTML` with `#root` mount point)
3. `desktop-runtime.js` loading (env var → relative path search)
4. Multiple event loop styles: `run()` (blocking), `run_with_mount(len)` (mount + run), `run_interactive()` (drain events), or manual `step()` + `poll_event()` for full control.

## Step 3.6 — Create the desktop JS runtime (`desktop-runtime.js`) ✅

Created `desktop/runtime/desktop-runtime.js` — a standalone 900+ line JS file containing:

- **`MutationReader`** — reads binary opcodes from an ArrayBuffer (base64-decoded)
- **`TemplateCache`** — registers and clones DocumentFragment templates
- **`Interpreter`** — full stack machine implementing all mutation opcodes (LoadTemplate, SetAttribute, SetText, AppendChildren, NewEventListener, Remove, ReplaceWith, ReplacePlaceholder, InsertAfter, InsertBefore, AssignId, CreateTextNode, CreatePlaceholder, PushRoot, RegisterTemplate, RemoveAttribute, RemoveEventListener)
- **Event dispatch** — DOM event listeners that serialize events as JSON and call `window.mojo_post()`
- **`window.__mojo_apply_mutations(base64)`** — entry point called by the C shim's `mwv_apply_mutations()`

This is a self-contained adaptation of the web renderer's TypeScript runtime, transpiled to plain JS for webview injection.

## Step 3.7 — Build the counter example ✅

Created `desktop/examples/counter.mojo` — a working counter app demonstrating:

- Same `CounterApp` struct and reactive logic as the web version
- `DesktopApp` entry point instead of `@export` WASM wrappers
- Heap buffer instead of WASM linear memory
- Full interactive event loop: mount → poll events → dispatch → re-render → flush mutations
- `ConditionalSlot` for show/hide detail section (even/odd, doubled value)

Build and run:

```text
cd mojo-gui/desktop
just run-counter
```

## Step 3.8 — Build system and Nix integration ✅

- **`justfile`** — `build-shim`, `build-counter`, `run-counter`, `dev-counter`, `test-shim`, `test-runtime`
- **`default.nix`** — dev shell with GTK4, WebKitGTK 6.0, pkg-config, `libmojo-webview` derivation, environment variables
- **`shim/default.nix`** — standalone Nix derivation for the C shim library

---

## Step 3.9 — Unified app lifecycle and `launch()`

The remaining work is **not** about porting individual examples to desktop. Each example must be exactly the same source file for every target — no per-renderer copies, no `desktop/examples/` duplicates. The framework must abstract the platform away so that `launch[MyApp]()` works on web and desktop from a single source file.

### Step 3.9.1 — Define the `GuiApp` trait ✅

Created `core/src/platform/gui_app.mojo` with the app-side lifecycle trait:

```text
trait GuiApp(Movable):
    fn __init__(out self)
    fn render(mut self) -> UInt32
    fn handle_event(mut self, handler_id: UInt32, event_type: UInt8, value: String) -> Bool
    fn flush(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn mount(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn has_dirty(self) -> Bool
    fn consume_dirty(mut self) -> Bool
    fn destroy(mut self)
```

This captures the lifecycle that currently lives as free functions (`counter_app_rebuild`, `todo_app_flush`, etc.). Each existing app struct (CounterApp, TodoApp, BenchmarkApp, MultiViewApp) already has `render()`, the lifecycle free functions, and a `ctx` field — the refactor is mechanical: move the free functions into the struct as methods, add the trait conformance.

The trait uses `has_dirty()` / `consume_dirty()` / `destroy()` instead of a raw `context()` pointer, keeping the `ComponentContext` internals private to the app. This is cleaner than exposing a pointer and allows apps with additional dirty state (e.g., MultiViewApp's `router.dirty`) to compose it naturally.

`handle_event` takes a `value: String` parameter (empty when not applicable). This unifies `dispatch_event()` and `dispatch_event_with_string()` — the renderer always passes the value through. This resolves the input event value binding issue: the desktop event loop no longer needs app-specific branching on `event.has_value`.

The trait is exported from the `platform` package via `__init__.mojo`.

### Step 3.9.2 — Implement the generic desktop event loop ✅

Created `desktop/src/desktop/launcher.mojo` with a generic `desktop_launch[AppType: GuiApp]()` function backed by the Blitz rendering engine:

```text
fn desktop_launch[AppType: GuiApp](config: AppConfig) raises:
    var blitz = Blitz.create(config.title, config.width, config.height, debug=config.debug)
    blitz.add_ua_stylesheet(_DEFAULT_UA_CSS)
    var app = AppType()

    var buf_ptr = _alloc_mutation_buffer(_DEFAULT_BUF_CAPACITY)
    var writer_ptr = _alloc_writer(buf_ptr, _DEFAULT_BUF_CAPACITY)
    var interpreter = MutationInterpreter(blitz)

    var mount_len = app.mount(writer_ptr)
    if mount_len > 0:
        blitz.begin_mutations()
        interpreter.apply(buf_ptr, Int(mount_len))
        blitz.end_mutations()
        blitz.request_redraw()

    while blitz.is_alive():
        _ = blitz.step(blocking=False)
        var had_event = False
        while True:
            var event = blitz.poll_event()
            if not event.valid: break
            had_event = True
            _ = app.handle_event(event.handler_id, event.event_type, event.value)
        if app.has_dirty():
            _reset_writer(writer_ptr, buf_ptr, _DEFAULT_BUF_CAPACITY)
            var flush_len = app.flush(writer_ptr)
            if flush_len > 0:
                blitz.begin_mutations()
                interpreter.apply(buf_ptr, Int(flush_len))
                blitz.end_mutations()
                blitz.request_redraw()
        elif not had_event:
            _ = blitz.step(blocking=True)

    _free_writer(writer_ptr)
    buf_ptr.free()
    app.destroy()
    blitz.destroy()
```

This single function replaces every `desktop/examples/*.mojo` file — the event loop is identical for counter, todo, bench, and app. The `GuiApp` trait methods encapsulate all app-specific logic (ConditionalSlot management, KeyedList flush, custom event routing, etc.). Note: `has_dirty()` is used instead of `consume_dirty()` in the event loop check because `flush()` calls `consume_dirty()` internally. `destroy()` is called directly on the app (no need to reach into `context()`).

Key difference from the webview approach: mutations are applied in-process via the Mojo `MutationInterpreter` → Blitz C FFI calls (no base64 encoding, no JS eval, no IPC). The interpreter reads the same binary opcode buffer and translates each opcode to the corresponding Blitz DOM operation.

### Step 3.9.3 — Wire `launch()` to dispatch by target ✅

Updated `core/src/platform/launch.mojo` so `launch[AppType: GuiApp]()` dispatches at compile time:

```text
fn launch[AppType: GuiApp](config: AppConfig = AppConfig()) raises:
    @parameter
    if is_wasm_target():
        pass  # WASM: JS runtime drives the loop; @export wrappers call GuiApp methods
    else:
        # Desktop path: create Blitz window and enter event loop.
        # The config is passed directly — no need for global state.
        from desktop.launcher import desktop_launch
        desktop_launch[AppType](config)
```

The previous non-parametric `launch(config)` overload and module-level `var _global_config` / `var _launched` globals were removed. Mojo does not support module-level `var` on native targets, so the current design avoids global mutable state entirely: on native, config is passed directly to `desktop_launch()` as an argument; on WASM, `@export` wrappers receive config through compile-time type parameters and constructor arguments. The `get_launch_config()` and `has_launched()` functions return defaults for API compatibility — callers should use the config passed directly to them.

The native target dispatch was updated from a placeholder print statement to the actual `desktop_launch` call as part of Phase 4 Blitz implementation.

### Step 3.9.4 — Refactor existing app structs to implement `GuiApp` ✅

All four main app structs now implement the `GuiApp` trait. The refactor was mechanical — free functions were moved into struct methods, and the struct declarations changed from `(Movable)` to `(GuiApp)`:

| App struct | Refactored methods | Notes |
|---|---|---|
| `CounterApp` | `mount()`, `handle_event()`, `flush()`, `has_dirty()`, `consume_dirty()`, `destroy()` | Simple — click events only, ConditionalSlot for detail |
| `TodoApp` | `mount()`, `handle_event()`, `flush()`, `has_dirty()`, `consume_dirty()`, `destroy()` | String events dispatched via `dispatch_event_with_string` when `len(value) > 0` |
| `BenchmarkApp` | `mount()`, `handle_event()`, `flush()`, `has_dirty()`, `consume_dirty()`, `destroy()` | Toolbar routing + KeyedList row events, performance timing |
| `MultiViewApp` | `mount()`, `handle_event()`, `flush()`, `has_dirty()`, `consume_dirty()`, `destroy()` | Router dirty state composed into `has_dirty()`, nav + signal dispatch |

The unified `handle_event(handler_id, event_type, value)` pattern works for all apps:

- Apps without string events (counter, bench) pass through to `dispatch_event()` when `value` is empty
- Apps with string events (todo) dispatch via `dispatch_event_with_string()` when `len(value) > 0`
- Apps with custom routing (multi-view) check app-level handlers first, then fall back to `dispatch_event()`

**Backwards compatibility:** The original free functions (`counter_app_rebuild`, `counter_app_flush`, etc.) are retained as thin one-line wrappers that delegate to the new trait methods. This preserves compatibility with the existing `@export` wrappers in `web/src/main.mojo`. These wrappers can be removed once the `@export` surface is genericized over `GuiApp` (Step 3.9.5).

Platform-specific APIs like `performance_now()` need conditional compilation:

```text
fn performance_now() -> Float64:
    @parameter
    if is_wasm_target():
        return external_call["performance_now", Float64]()
    else:
        from time import perf_counter_ns
        return Float64(perf_counter_ns()) / 1_000_000.0
```

This stays in the shared example code — no duplication needed.

### Step 3.9.5 — Refactor `main.mojo` WASM exports to be generic over `GuiApp` ✅

Created `web/src/gui_app_exports.mojo` with parametric lifecycle helper functions:

```text
fn gui_app_init[T: GuiApp]() -> Int64          # heap alloc + T.__init__()
fn gui_app_destroy[T: GuiApp](app_ptr: Int64)   # T.destroy() + free
fn gui_app_mount[T: GuiApp](app_ptr, buf, cap) -> Int32    # T.mount()
fn gui_app_handle_event[T: GuiApp](app_ptr, hid, et) -> Int32  # T.handle_event(..., "")
fn gui_app_handle_event_string[T: GuiApp](app_ptr, hid, et, value) -> Int32  # T.handle_event(..., value)
fn gui_app_flush[T: GuiApp](app_ptr, buf, cap) -> Int32    # T.flush()
fn gui_app_has_dirty[T: GuiApp](app_ptr) -> Int32           # T.has_dirty()
fn gui_app_consume_dirty[T: GuiApp](app_ptr) -> Int32       # T.consume_dirty()
```

The per-app `@export` wrappers in `main.mojo` are now one-liners:

```text
@export fn counter_init() -> Int64:
    return gui_app_init[CounterApp]()
@export fn counter_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    return gui_app_mount[CounterApp](app_ptr, buf_ptr, capacity)
@export fn counter_handle_event(app_ptr: Int64, handler_id: Int32, event_type: Int32) -> Int32:
    return gui_app_handle_event[CounterApp](app_ptr, handler_id, event_type)
@export fn counter_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    return gui_app_flush[CounterApp](app_ptr, buf_ptr, capacity)
@export fn counter_destroy(app_ptr: Int64):
    gui_app_destroy[CounterApp](app_ptr)
```

All four main apps (CounterApp, TodoApp, BenchmarkApp, MultiViewApp) now use the generic helpers. The backwards-compatible free functions (`counter_app_init`, `counter_app_rebuild`, etc.) have been removed from the example files — the generic helpers call `GuiApp` trait methods directly. App-specific query exports (e.g., `counter_count_value`, `todo_item_count`) remain as hand-written `@export` functions since they reach into app-specific fields. All 3,090 JS tests and 52 Mojo test suites pass.

### Step 3.9.6 — Delete `desktop/examples/` and add `launch()` to shared examples ✅

Steps 3.9.1–3.9.5 are complete, the cdylib builds, and Winit integration (Step 4.6) is done.

1. ✅ No `desktop/examples/` directory exists (never created as separate duplicates — the webview counter example was not carried forward to Blitz).
2. ✅ Added `main.mojo` with `fn main() raises: launch[AppType](AppConfig(...))` to each shared example:
   - `examples/counter/main.mojo` — `launch[CounterApp](AppConfig(title="High-Five Counter", width=400, height=350))`
   - `examples/todo/main.mojo` — `launch[TodoApp](AppConfig(title="Todo List", width=500, height=600))`
   - `examples/bench/main.mojo` — `launch[BenchmarkApp](AppConfig(title="js-framework-benchmark — mojo-gui", width=1000, height=800))`
   - `examples/app/main.mojo` — `launch[MultiViewApp](AppConfig(title="Multi-View App", width=600, height=500))`
3. Each example compiles for both targets with identical source:
   - `mojo build examples/counter/main.mojo --target wasm64-wasi -I core/src -I web/src -I examples` → WASM
   - `mojo build examples/counter/main.mojo -I core/src -I desktop/src -I examples` → native

### Step 3.9.7 — Cross-target verification and CI

- [ ] Cross-target CI test matrix (web + desktop for every shared example)
- [ ] Verify all 4 examples (counter, todo, bench, app) build and run on both targets from identical source
- [ ] Window lifecycle events (close confirmation, minimize/maximize state)
- [ ] Investigate replacing base64 IPC with more efficient binary transfer (custom URI scheme or shared memory)

**Cross-target status (target state after Step 3.9):**

| Example   | Source location | Web (WASM) | Desktop (Blitz) | Same source? |
|-----------|----------------|------------|-----------------|--------------|
| counter   | `examples/counter/main.mojo` | ✅ | ⏳ (needs build verification) | ✅ |
| todo      | `examples/todo/main.mojo` | ✅ | ⏳ (needs build verification) | ✅ |
| bench     | `examples/bench/main.mojo` | ✅ | ⏳ (needs build verification) | ✅ |
| app       | `examples/app/main.mojo` | ✅ | ⏳ (needs build verification) | ✅ |