# Architecture & Design

> Part of the [mojo-gui plan](../../PLAN.md). See also: [renderers](renderers.md), [risks](risks.md).

## Architectural Inspiration: Dioxus

Dioxus separates concerns as:

| Dioxus crate        | Role                                         |
|---------------------|----------------------------------------------|
| `dioxus-core`       | VirtualDom, signals, scopes, mutations       |
| `dioxus-html`       | HTML elements, attributes, events            |
| `dioxus-web`        | Browser renderer (WASM + JS interop)         |
| `dioxus-native`     | Native renderer (Blitz HTML/CSS engine)      |

Critically, Dioxus examples live in the workspace root and compile against any renderer. The user calls `dioxus::launch(App)` and the renderer is selected at compile time via feature flags. **We follow this same model:** examples are never renderer-specific, and `launch()` is the only platform-dependent call.

Note: Dioxus's desktop renderer evolved through similar stages — early versions used a webview (Wry/Tauri), and later versions introduced Blitz for native HTML/CSS rendering. We follow the same progression: webview first (Phase 3, implemented), Blitz native (Phase 4, implemented — verification pending).

Separately, Rust's `web-sys` crate provides raw bindings to **all** Web APIs (DOM, fetch, WebSocket, WebGL, etc.) via `wasm-bindgen`. Any Rust/WASM project can use `web-sys` directly — Dioxus-web uses it under the hood. `mojo-web` fills this same ecosystem role for Mojo.

Key insight: **the mutation protocol stays DOM-oriented even in core**. The desktop webview renderer reuses the same JS mutation interpreter inside an embedded webview. Future desktop renderers can use a native HTML/CSS rendering engine (like [Blitz](https://github.com/DioxusLabs/blitz)) that provides a real DOM without a browser, while XR renderers render DOM content to textures placed in 3D space. This is pragmatic — HTML/DOM is a universal UI description language. `mojo-gui` uses the mutation protocol (not `mojo-web`) for rendering, keeping the multi-renderer architecture intact. `mojo-web` is for everything else an app needs from the browser: data fetching, storage, timers, canvas, etc.

---

## Design Principle: Shared Examples, Abstracted Platform

A core design principle of this separation is that **all example apps are platform-agnostic and shared across every renderer target**. This means:

1. **App code never imports a renderer.** Apps import only from `mojo-gui/core` (signals, components, HTML DSL). They do NOT import from `mojo-gui/web` or `mojo-gui/desktop`.

2. **The `launch()` function is the abstraction boundary.** Each renderer provides a `launch()` entry point. The app defines its root component; the renderer drives the event loop.

3. **Examples live in `mojo-gui/examples/`, not per-renderer.** There is ONE counter app, ONE todo app, ONE bench app. Each is built for web via `mojo build --target wasm64-wasi` and for desktop via `mojo build` (native). The app source is identical — only the build target differs.

4. **Renderer-specific entry points are thin wrappers.** The `web/` and `desktop/` directories provide only the machinery to drive the shared app code on their respective platforms. They do not contain app logic.

5. **If an example doesn't work on a target, it's a framework bug.** The framework must abstract away platform differences so that any app written against `core` works on every supported renderer.

This principle applies equally to user-authored apps: if you write an app against `mojo-gui/core`, it should run on web, desktop, and (eventually) native without modification.

---

## Current Module Map & Classification

### Renderer-Agnostic (→ `mojo-gui/core`)

These modules have **zero DOM/browser dependencies** — pure reactive infrastructure:

| Module                      | Purpose                                        |
|-----------------------------|------------------------------------------------|
| `src/signals/runtime.mojo`  | Reactive runtime, signal store, string store, context tracking |
| `src/signals/memo.mojo`     | MemoEntry, MemoStore (derived signals)         |
| `src/signals/effect.mojo`   | Effect infrastructure                          |
| `src/signals/handle.mojo`   | SignalI32, SignalBool, SignalString, MemoI32, MemoBool, MemoString, EffectHandle |
| `src/scope/scope.mojo`      | ScopeState, hooks, context, error/suspense     |
| `src/scope/arena.mojo`      | ScopeArena (slab allocator)                    |
| `src/scheduler/scheduler.mojo` | Height-ordered dirty scope queue            |
| `src/arena/element_id.mojo` | ElementId type and allocator                   |

### Virtual DOM Layer (→ `mojo-gui/core`)

The VNode/Template/diff machinery is *structurally* DOM-oriented but is renderer-agnostic in implementation — it never touches real DOM, only emits mutations to a buffer:

| Module                         | Purpose                                     |
|--------------------------------|---------------------------------------------|
| `src/vdom/template.mojo`       | Template, TemplateNode (static structure)   |
| `src/vdom/vnode.mojo`          | VNode, DynamicNode, AttributeValue, VNodeBuilder |
| `src/vdom/builder.mojo`        | TemplateBuilder API                         |
| `src/vdom/registry.mojo`       | Template storage and lookup                 |
| `src/mutations/create.mojo`    | CreateEngine (VNode → mutation buffer)      |
| `src/mutations/diff.mojo`      | DiffEngine (old/new VNode → minimal mutations) |
| `src/bridge/protocol.mojo`     | MutationWriter + binary opcodes             |

### HTML-Specific (→ `mojo-gui/core/html` submodule)

These define **what** elements/events exist — the HTML vocabulary:

| Module                      | Purpose                                        |
|-----------------------------|------------------------------------------------|
| `src/vdom/tags.mojo`        | TAG_DIV, TAG_SPAN, TAG_BUTTON, ... (38 tags)  |
| `src/vdom/dsl.mojo`         | `el_div()`, `el_button()`, `dyn_text()`, `onclick_add()`, inline event constructors |
| `src/vdom/dsl_tests.mojo`   | DSL test functions                             |
| `src/events/registry.mojo`  | HandlerEntry, HandlerRegistry, action tags, event type constants (EVT_CLICK, EVT_INPUT, ...) |

### Component Framework (→ `mojo-gui/core`, mixed concerns)

These bundle reactive + vdom + mutations into an ergonomic app framework. They reference HTML-specific types but the *structure* is renderer-agnostic:

| Module                              | Purpose                                  |
|--------------------------------------|------------------------------------------|
| `src/component/app_shell.mojo`       | AppShell: runtime + store + allocator + scheduler |
| `src/component/context.mojo`         | ComponentContext: ergonomic API, RenderBuilder |
| `src/component/child.mojo`           | ChildComponent rendering                 |
| `src/component/child_context.mojo`   | ChildComponentContext                    |
| `src/component/lifecycle.mojo`       | mount, diff, finalize, FragmentSlot, ConditionalSlot |
| `src/component/keyed_list.mojo`      | KeyedList, ItemBuilder, HandlerAction    |
| `src/component/router.mojo`          | URL path → branch router                 |

### Platform Abstraction (→ `mojo-gui/core`, new)

A thin abstraction layer that lets apps remain platform-agnostic:

| Module                              | Purpose                                  |
|--------------------------------------|------------------------------------------|
| `src/platform/launch.mojo`          | `launch()` — platform-dispatching entry point (compile-time target selection) |
| `src/platform/app.mojo`             | `App` trait — interface every renderer's app host must implement |
| `src/platform/features.mojo`        | Feature detection — what capabilities are available on the current target |

### Browser/WASM Runtime (→ `mojo-gui/web`)

Everything that runs in the browser or manages WASM instantiation:

| Module                        | Purpose                                      |
|-------------------------------|----------------------------------------------|
| `runtime/interpreter.ts`      | DOM stack machine (applies binary mutations) |
| `runtime/events.ts`           | DOM event delegation bridge                  |
| `runtime/templates.ts`        | Template cache (DocumentFragment cloning)    |
| `runtime/memory.ts`           | WASM memory management, free-list allocator  |
| `runtime/env.ts`              | WASM environment imports (I/O, math, libc)   |
| `runtime/strings.ts`          | Mojo String ABI helpers (SSO)                |
| `runtime/protocol.ts`         | JS-side mutation opcode parser               |
| `runtime/tags.ts`             | HTML tag name mapping (JS side)              |
| `runtime/app.ts`              | App lifecycle helpers, per-app handles       |
| `runtime/types.ts`            | WasmExports interface                        |
| `runtime/mod.ts`              | Entry point (instantiate WASM)               |
| `src/main.mojo`               | @export wrappers (WASM entry point)          |
| `test-js/`                    | JS integration tests                         |
| `scripts/`                    | Build scripts (Mojo → WASM pipeline)         |
| `justfile`                    | Build commands                               |
| `default.nix`                 | Nix dev shell                                |

### Desktop Runtime (→ `mojo-gui/desktop`, implemented)

Everything for the native desktop application. The webview approach (GTK4 + WebKitGTK) was built first as Phase 3, then superseded by the Blitz native renderer (Phase 4).

**Blitz renderer (Phase 4 — implementation complete, verification pending):**

| Module                                | Purpose                                      |
|---------------------------------------|----------------------------------------------|
| `shim/src/lib.rs`                     | Rust `cdylib`: `BlitzContext` wrapping `blitz-dom`, ID mapping, template registry, event queue, interpreter stack |
| `shim/mojo_blitz.h`                   | C API header (~45 FFI functions: lifecycle, DOM, templates, events, stack, debug) |
| `shim/Cargo.toml`                     | Rust crate config (blitz-dom, blitz-html, blitz-traits, blitz-shell, blitz-paint, winit, etc.) |
| `shim/default.nix`                    | Nix derivation with Wayland + GPU deps (Vulkan, Wayland, fontconfig) |
| `src/desktop/blitz.mojo`              | Mojo FFI bindings to `libmojo_blitz_shim.so` via `DLHandle` |
| `src/desktop/renderer.mojo`           | `MutationInterpreter`: reads binary opcodes → Blitz C FFI calls (all 18 opcodes) |
| `src/desktop/launcher.mojo`           | `desktop_launch[AppType: GuiApp]()` — generic Blitz-backed event loop |

**Webview approach (Phase 3 — removed, kept for reference):**

| Module                                | Purpose                                      |
|---------------------------------------|----------------------------------------------|
| `shim/mojo_webview.c`                 | C shim: GTK4 + WebKitGTK, ring buffer events |
| `shim/mojo_webview.h`                 | C API header for the webview shim            |
| `runtime/desktop-runtime.js`          | Standalone JS mutation interpreter for webview |
| `runtime/shell.html`                  | HTML shell with `#root` mount point          |
| `src/desktop/webview.mojo`            | Mojo FFI bindings to libmojo_webview.so      |
| `src/desktop/bridge.mojo`             | Mutation buffer + event polling bridge        |
| `src/desktop/app.mojo`                | DesktopApp: lifecycle, event loop, init       |

**Shared:**

| Module                                | Purpose                                      |
|---------------------------------------|----------------------------------------------|
| `src/desktop/__init__.mojo`           | Package root                                 |
| `examples/counter.mojo`               | Desktop counter example (temporary — to be replaced by shared `examples/counter/` via `launch()`) |
| `justfile`                            | Build commands (build-shim, run-counter)      |
| `default.nix`                         | Nix dev shell with desktop deps               |

### Example Apps (→ `mojo-gui/examples/`, shared across all targets)

| Module                        | Destination                                  |
|-------------------------------|----------------------------------------------|
| `src/apps/counter.mojo`      | `mojo-gui/examples/counter/app.mojo` — shared app logic |
| `src/apps/todo.mojo`         | `mojo-gui/examples/todo/app.mojo` — shared app logic |
| `src/apps/bench.mojo`        | `mojo-gui/examples/bench/app.mojo` — shared app logic |
| `examples/counter/`          | `mojo-gui/examples/counter/web/` — web-specific assets (HTML, JS glue) |
| `examples/todo/`             | `mojo-gui/examples/todo/web/` — web-specific assets |
| `examples/bench/`            | `mojo-gui/examples/bench/web/` — web-specific assets |
| `test/*.mojo`                | `mojo-gui/core/test/` (Mojo-side unit tests) |
| `test-js/*.test.ts`          | `mojo-gui/web/test-js/` (browser integration tests) |

---

## Target Project Structure

```text
mojo-gui/
├── core/                             # Renderer-agnostic GUI framework
│   ├── src/
│   │   ├── signals/                  # Reactive primitives
│   │   │   ├── runtime.mojo          # Runtime, SignalStore, StringStore, context
│   │   │   ├── memo.mojo             # MemoEntry, MemoStore
│   │   │   ├── effect.mojo           # EffectHandle
│   │   │   └── handle.mojo           # SignalI32, SignalBool, SignalString, Memo*
│   │   ├── scope/                    # Scope lifecycle
│   │   │   ├── scope.mojo            # ScopeState, hooks, context, error/suspense
│   │   │   └── arena.mojo            # ScopeArena (slab allocator)
│   │   ├── scheduler/
│   │   │   └── scheduler.mojo        # Height-ordered dirty scope queue
│   │   ├── arena/
│   │   │   └── element_id.mojo       # ElementId type and allocator
│   │   ├── vdom/                     # Virtual DOM (renderer-agnostic)
│   │   │   ├── template.mojo         # Template, TemplateNode
│   │   │   ├── vnode.mojo            # VNode, DynamicNode, AttributeValue
│   │   │   ├── builder.mojo          # TemplateBuilder API
│   │   │   └── registry.mojo         # Template storage and lookup
│   │   ├── mutations/                # Mutation engines
│   │   │   ├── create.mojo           # CreateEngine (initial mount)
│   │   │   └── diff.mojo             # DiffEngine (reconciliation)
│   │   ├── bridge/
│   │   │   └── protocol.mojo         # MutationWriter + binary opcodes
│   │   ├── events/
│   │   │   └── registry.mojo         # HandlerEntry, HandlerRegistry, action tags
│   │   ├── component/                # Component framework
│   │   │   ├── app_shell.mojo        # AppShell
│   │   │   ├── context.mojo          # ComponentContext, RenderBuilder
│   │   │   ├── child.mojo            # ChildComponent
│   │   │   ├── child_context.mojo    # ChildComponentContext
│   │   │   ├── lifecycle.mojo        # mount, diff, finalize, Fragment/ConditionalSlot
│   │   │   ├── keyed_list.mojo       # KeyedList, ItemBuilder, HandlerAction
│   │   │   └── router.mojo           # URL path → branch router
│   │   ├── platform/                 # Platform abstraction layer
│   │   │   ├── launch.mojo           # launch() — target-dispatching entry point
│   │   │   ├── app.mojo              # PlatformApp trait — interface renderers implement
│   │   │   ├── gui_app.mojo          # GuiApp trait — interface apps implement
│   │   │   └── features.mojo         # Runtime feature detection
│   │   ├── html/                     # HTML vocabulary (submodule)
│   │   │   ├── tags.mojo             # TAG_DIV, TAG_SPAN, ... (moved from vdom/tags.mojo)
│   │   │   ├── dsl.mojo              # el_div(), el_button(), ... (moved from vdom/dsl.mojo)
│   │   │   └── dsl_tests.mojo        # DSL tests (moved from vdom/dsl_tests.mojo)
│   │   └── lib.mojo                  # Package root: re-exports public API
│   ├── test/                         # Mojo-side unit tests
│   │   ├── test_signals.mojo
│   │   ├── test_scopes.mojo
│   │   ├── test_memo.mojo
│   │   └── ...
│   ├── AGENTS.md
│   ├── README.md
│   └── CHANGELOG.md
│
├── examples/                         # Shared example apps (run on ALL targets)
│   ├── counter/
│   │   ├── app.mojo                  # Counter app logic (platform-agnostic)
│   │   └── web/                      # Web-specific assets (HTML shell, JS glue)
│   │       ├── index.html
│   │       └── main.ts
│   ├── todo/
│   │   ├── app.mojo                  # Todo app logic (platform-agnostic)
│   │   └── web/
│   │       ├── index.html
│   │       └── main.ts
│   ├── bench/
│   │   ├── app.mojo                  # Bench app logic (platform-agnostic)
│   │   └── web/
│   │       ├── index.html
│   │       └── main.ts
│   └── README.md                     # How to build & run examples on each target
│
├── web/                              # Browser renderer (WASM + TypeScript)
│   ├── runtime/                      # TypeScript runtime (from mojo-wasm/runtime/)
│   │   ├── mod.ts
│   │   ├── interpreter.ts            # DOM stack machine
│   │   ├── events.ts                 # DOM event delegation
│   │   ├── templates.ts              # Template cache (DocumentFragment)
│   │   ├── memory.ts                 # WASM memory management
│   │   ├── env.ts                    # WASM environment imports
│   │   ├── strings.ts                # Mojo String ABI
│   │   ├── protocol.ts              # JS mutation opcode parser
│   │   ├── tags.ts                   # HTML tag names (JS side)
│   │   ├── app.ts                    # App lifecycle helpers
│   │   └── types.ts                  # WasmExports interface
│   ├── src/
│   │   ├── main.mojo                 # @export WASM wrappers
│   │   └── web_launcher.mojo         # WebApp — implements App trait for WASM target
│   ├── test-js/                      # JS integration tests
│   │   ├── harness.ts
│   │   ├── counter.test.ts
│   │   └── ...
│   ├── scripts/                      # Build pipeline (Mojo → WASM)
│   │   ├── build-test-binaries.nu
│   │   ├── run-test-binaries.nu
│   │   ├── build-examples.nu         # Builds all shared examples for web target
│   │   └── precompile.mojo
│   ├── deno.json
│   ├── justfile
│   └── README.md
│
├── desktop/                          # Desktop renderer (Phase 4 — Blitz native renderer)
│   ├── src/
│   │   └── desktop/                  # Desktop renderer package
│   │       ├── __init__.mojo         # Package root
│   │       ├── blitz.mojo            # Mojo FFI bindings to libmojo_blitz_shim.so via DLHandle (Phase 4)
│   │       ├── renderer.mojo         # MutationInterpreter — binary opcodes → Blitz FFI calls (Phase 4)
│   │       ├── launcher.mojo         # desktop_launch[AppType: GuiApp]() — Blitz-backed event loop
│   │       ├── app.mojo              # DesktopApp — webview lifecycle (Phase 3, legacy)
│   │       ├── bridge.mojo           # DesktopBridge — mutation buffer + event polling (Phase 3, legacy)
│   │       └── webview.mojo          # Mojo FFI bindings to libmojo_webview.so (Phase 3, legacy)
│   ├── shim/
│   │   ├── src/lib.rs                # Rust cdylib: BlitzContext wrapping blitz-dom (Phase 4)
│   │   ├── mojo_blitz.h              # C API header — ~45 FFI functions (Phase 4)
│   │   ├── Cargo.toml                # Rust crate config (blitz, winit, etc.) (Phase 4)
│   │   ├── default.nix               # Nix derivation with GPU/windowing deps (Phase 4)
│   │   ├── mojo_webview.h            # C API header for webview (Phase 3, legacy)
│   │   └── mojo_webview.c            # C implementation GTK4+WebKitGTK (Phase 3, legacy)
│   ├── runtime/
│   │   ├── desktop-runtime.js        # Standalone JS interpreter (Phase 3, legacy)
│   │   └── shell.html                # HTML shell with #root mount point (Phase 3, legacy)
│   ├── examples/                     # TEMPORARY — to be deleted once Blitz cdylib builds
│   │   └── counter.mojo              # Desktop counter demo (temporary duplicate)
│   ├── build/                        # Build artifacts (libmojo_blitz_shim.so, binaries)
│   ├── default.nix                   # Nix dev shell with all desktop dependencies
│   ├── justfile                      # Build commands (build-shim, run-counter, etc.)
│   └── README.md
│
├── xr/                               # XR renderer (Phase 5 — future, OpenXR + WebXR)
│   ├── native/                       # OpenXR native renderer (Blitz DOM → Vello → OpenXR swapchain)
│   │   ├── shim/
│   │   │   ├── src/lib.rs            # Rust cdylib: Blitz + Vello + openxr crate, panel management
│   │   │   ├── mojo_xr.h            # C API header — XR session, panels, input
│   │   │   ├── Cargo.toml           # Rust crate config (blitz-dom, vello, openxr, winit)
│   │   │   └── default.nix          # Nix derivation with OpenXR/GPU deps
│   │   └── src/
│   │       ├── xr_launcher.mojo     # xr_launch[AppType: GuiApp]() — OpenXR event loop
│   │       ├── xr_blitz.mojo        # Mojo FFI bindings to libmojo_xr.so
│   │       ├── panel.mojo           # XRPanel — DOM document + 3D transform + input mapping
│   │       └── scene.mojo           # XRScene — panel registry, spatial layout, raycasting
│   ├── web/                          # WebXR browser renderer (extends mojo-gui/web)
│   │   ├── runtime/
│   │   │   ├── xr-session.ts        # WebXR session lifecycle, reference spaces
│   │   │   ├── xr-panels.ts         # DOM → texture rendering, 3D panel placement
│   │   │   ├── xr-input.ts          # XR controller ray → DOM event translation
│   │   │   └── xr-runtime.ts        # Entry point — extends web runtime with XR
│   │   └── src/
│   │       └── webxr_launcher.mojo  # WebXR-specific launch configuration
│   └── README.md
│
└── README.md
```

---

## The Abstraction Boundary: Binary Mutation Protocol

The **mutation buffer** is the renderer contract. Every renderer must implement an interpreter that consumes the same binary opcode stream:

```text
┌──────────────────────┐     binary mutation buffer      ┌─────────────────────┐
│                      │  ───────────────────────────►   │                     │
│  mojo-gui/core       │     (shared linear memory       │  Renderer           │
│  (reactive framework │      or pipe/socket)            │  (web / desktop /   │
│   + virtual DOM      │                                 │   native)           │
│   + diff engine)     │  ◄───────────────────────────   │                     │
│                      │     event dispatch callbacks     │                     │
└──────────────────────┘                                 └─────────────────────┘
```

The opcodes (`OP_CREATE_TEXT_NODE`, `OP_SET_ATTRIBUTE`, `OP_LOAD_TEMPLATE`, etc.) are DOM-oriented by design. This is intentional — all three renderer targets can interpret them:

| Opcode              | Web (DOM)                     | Desktop (Blitz)                       | XR Native (OpenXR + Blitz)                    | XR Web (WebXR)                        |
|---------------------|-------------------------------|---------------------------------------|------------------------------------------------|---------------------------------------|
| `LOAD_TEMPLATE`     | `cloneNode(true)`             | `blitz_clone_template()` via C FFI    | `blitz_clone_template()` → panel document      | `cloneNode(true)` → panel DOM         |
| `SET_ATTRIBUTE`     | `el.setAttribute()`           | `blitz_set_attribute()` via C FFI     | `blitz_set_attribute()` → panel document       | `el.setAttribute()` → panel DOM      |
| `SET_TEXT`          | `node.textContent = ...`      | `blitz_set_text_content()` via C FFI  | `blitz_set_text_content()` → panel document    | `node.textContent` → panel DOM       |
| `NEW_EVENT_LISTENER`| `addEventListener()`          | `blitz_add_event_listener()` via C FFI| `blitz_add_event_listener()` → panel document  | `addEventListener()` → panel DOM     |
| `APPEND_CHILDREN`  | `parent.appendChild()`        | `blitz_append_child()` via C FFI      | `blitz_append_child()` → panel document        | `parent.appendChild()` → panel DOM   |
| `REMOVE`           | `node.remove()`               | `blitz_remove_node()` via C FFI       | `blitz_remove_node()` → panel document         | `node.remove()` → panel DOM          |

---

## Platform Abstraction Layer

The platform abstraction layer is **not optional** — it is a core architectural requirement that enables shared examples and write-once app code. It lives in `mojo-gui/core/src/platform/`.

### The `PlatformApp` Trait (renderer side)

Every renderer implements the `PlatformApp` trait, which provides the lifecycle contract between the framework and the platform:

```text
# core/src/platform/app.mojo

trait PlatformApp(Movable):
    """Platform host that drives the reactive framework."""
    fn init(mut self) raises -> None
    fn flush_mutations(mut self, buf: UnsafePointer[UInt8], length: Int) raises -> None
    fn request_animation_frame(mut self) -> None
    fn should_quit(self) -> Bool
    fn destroy(mut self) -> None
```

This trait is the **only** thing that differs between platforms. App code never sees it directly — it interacts only with `ComponentContext`, signals, and the HTML DSL.

### The `GuiApp` Trait (app side)

Every app implements the `GuiApp` trait, which provides the lifecycle contract between the app and the framework's generic event loop:

```text
# core/src/platform/gui_app.mojo

trait GuiApp(Movable):
    """User-facing app that the framework drives."""
    fn render(mut self) -> UInt32
    fn handle_event(mut self, handler_id: UInt32, event_type: UInt8, value: String) -> Bool
    fn flush(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn mount(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn context(mut self) -> UnsafePointer[ComponentContext]
```

This trait captures the per-app lifecycle that currently lives as free functions (`counter_app_rebuild`, `counter_app_flush`, `counter_app_handle_event`). By implementing `GuiApp`, an app struct can be driven by **any** renderer's event loop without the renderer knowing anything about the app's internals.

`handle_event` takes an optional `value: String` parameter (empty string when no value). This unifies `dispatch_event()` and `dispatch_event_with_string()` — the renderer always passes the value through; the core framework ignores it for click events and uses it for input/keydown events.

### The `launch()` Function

The `launch()` function is the single entry point that all apps use. The renderer is selected at **compile time** based on the build target:

```text
# core/src/platform/launch.mojo

fn launch[AppType: GuiApp]():
    """Launch the app on the current platform.

    - WASM target → web renderer (JS runtime drives the event loop)
    - Native target → desktop renderer (webview/Blitz drives the event loop)
    """
    @parameter
    if is_wasm_target():
        _register_web_app[AppType]()
    else:
        _run_desktop_app[AppType]()
```

For **native targets**, `_run_desktop_app` is a generic event loop that works for any `GuiApp`:

```text
fn _run_desktop_app[AppType: GuiApp]() raises:
    var config = get_launch_config()
    var desktop = DesktopApp(title=config.title, width=config.width, ...)
    var app = AppType()
    desktop.init()

    # Mount
    var writer_ptr = _alloc_writer(desktop.buf_ptr(), desktop.buf_capacity())
    var mount_len = app.mount(writer_ptr)
    if mount_len > 0:
        desktop.flush_mutations(Int(mount_len))

    # Event loop
    while not desktop.should_quit():
        desktop.step(blocking=False)
        while True:
            var event = desktop.poll_event()
            if not event.is_valid(): break
            _ = app.handle_event(UInt32(event.handler_id), UInt8(event.event_type), event.value)
        if app.context()[].consume_dirty():
            _reset_writer(writer_ptr, ...)
            var flush_len = app.flush(writer_ptr)
            if flush_len > 0:
                desktop.flush_mutations(Int(flush_len))
        else:
            desktop.step(blocking=True)

    _free_writer(writer_ptr)
    app.context()[].destroy()
    desktop.destroy()
```

For **WASM targets**, `_register_web_app` stores the `AppType` so the JS runtime can invoke the lifecycle via `@export` wrappers. The `@export` surface in `main.mojo` becomes generic over `GuiApp` — one set of wrappers works for every app.

### How Apps Use It

A shared example app looks like this:

```text
# examples/counter/counter.mojo

from component import ComponentContext, ConditionalSlot
from signals import SignalI32, SignalBool
from html import el_div, el_h1, el_button, text, dyn_text, onclick_add, onclick_sub, onclick_toggle
from platform import launch, AppConfig

struct CounterApp(GuiApp):
    var ctx: ComponentContext
    var count: SignalI32
    # ... (same struct as today, unchanged)

    fn render(mut self) -> UInt32: ...
    fn handle_event(mut self, handler_id: UInt32, event_type: UInt8, value: String) -> Bool: ...
    fn flush(mut self, writer_ptr: ...) -> Int32: ...
    fn mount(mut self, writer_ptr: ...) -> Int32: ...
    fn context(mut self) -> UnsafePointer[ComponentContext]: ...

fn main() raises:
    launch[CounterApp](AppConfig(title="Counter", width=400, height=350))
```

**This exact code compiles and runs on every target:**

- `mojo build examples/counter/counter.mojo --target wasm64-wasi -I core/src -I web/src` → WASM binary for browser
- `mojo build examples/counter/counter.mojo -I core/src -I desktop/src` → native binary for desktop

The only difference is the build command. The source code is identical.

### What Each Renderer Provides

| Renderer             | Entry mechanism                      | Event loop driver                      |
|----------------------|--------------------------------------|----------------------------------------|
| **Web**              | JS runtime instantiates WASM, calls `@export` init | JS `requestAnimationFrame` + event listeners |
| **Desktop (webview)**| Native `main()` creates GTK4 window with webview | GTK main loop via `mwv_step()` polling  |
| **Desktop (Blitz)**  | Native `main()` creates Blitz window (future) | Winit event loop via Blitz C shim       |
| **XR (WebXR)**       | JS runtime creates WebXR session, renders DOM panels to textures in 3D | WebXR `requestAnimationFrame` + XR input sources |
| **XR (OpenXR)**      | Native `main()` creates OpenXR session, Blitz panels render to swapchain textures | OpenXR frame loop + Winit/Vello for panel rendering |

---

## Dependency Graph

```text
                    ┌──────────────┐
                    │  Shared      │
                    │  Examples    │
                    │ (examples/)  │
                    └──┬───────┬───┘
                       │       │
              imports  │       │  imports (optional,
                       │       │  web-only features)
                       ▼       ▼
              ┌──────────┐  ┌──────────┐
              │ mojo-gui │  │ mojo-web │
              │ /core    │  │ (future) │
              │          │  │ DOM      │
              │ signals/ │  │ fetch    │
              │ scope/   │  │ WebSocket│
              │ vdom/    │  │ storage  │
              │ mutations│  │ timers   │
              │ bridge/  │  │ canvas   │
              │ events/  │  │ ...      │
              │ component│  └──────────┘
              │ html/    │
              │ platform/│ ◄── PlatformApp trait + launch()
              └────┬─────┘
                   │ implements PlatformApp trait
         ┌─────────┼──────────┬────────────┐
         ▼         ▼          ▼            ▼
  ┌──────────┐ ┌────────────┐ ┌──────────────────────┐
  │ mojo-gui │ │ mojo-gui   │ │ mojo-gui/xr          │
  │ /web     │ │ /desktop   │ │ (future)              │
  │          │ │            │ │                        │
  │ WebApp   │ │ DesktopApp │ │ ┌─────────┬──────────┐│
  │ main.mojo│ │ Blitz +    │ │ │ native  │ web      ││
  │ runtime/ │ │ Winit +    │ │ │ OpenXR  │ WebXR    ││
  │ (TS/JS)  │ │ Vello      │ │ │ + Blitz │ + DOM    ││
  └──────────┘ └────────────┘ │ │ + Vello │ + WebGPU ││
                              │ └─────────┴──────────┘│
                              └──────────────────────┘
```

Key points:

- **Examples depend only on `core`** — they never import from `web/`, `desktop/`, or `xr/`
- **Renderers implement the `PlatformApp` trait** defined in `core/platform/`
- **`launch()` is the only platform-dispatching call** — it routes to the correct renderer at compile time
- **Desktop uses Blitz** (Winit + Vello for windowed rendering)
- **XR reuses the Blitz stack** — same DOM + CSS + rendering pipeline, targeting OpenXR swapchains instead of Winit windows; WebXR extends the web renderer
- **`mojo-web` is independent** — apps can optionally import it for non-rendering browser features