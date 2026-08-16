# mojo-gui — Multi-Renderer Reactive GUI Framework for Mojo

Write a Mojo GUI app once, run it in the browser via WASM **and** natively on desktop — inspired by [Dioxus](https://dioxuslabs.com/) for Rust.

## Architecture

```text
                    ┌──────────────┐
                    │  User App    │
                    │ (my_app.mojo)│
                    └──┬───────┬───┘
                       │       │
              imports  │       │  imports (optional,
                       │       │  web-only features)
                       ▼       ▼
              ┌──────────┐  ┌──────────┐
              │ mojo-gui │  │ mojo-web │
              │ /core    │  │ (future) │
              │          │  │          │
              │ signals/ │  │ DOM      │
              │ scope/   │  │ fetch    │
              │ vdom/    │  │ WebSocket│
              │ mutations│  │ storage  │
              │ bridge/  │  │ timers   │
              │ events/  │  │ canvas   │
              │ component│  │ ...      │
              │ html/    │  └──────────┘
              │ platform/│
              └────┬─────┘
                   │ consumed by
              ┌────┼────────────┐
              ▼    ▼            ▼
     ┌──────────┐ ┌──────────┐ ┌──────────┐
     │ mojo-gui │ │ mojo-gui │ │ mojo-gui │
     │ /web     │ │ /desktop │ │ /xr      │
     │          │ │          │ │ (future) │
     │ main.mojo│ │ Blitz    │ │          │
     │ runtime/ │ │ (Stylo + │ │ OpenXR + │
     │ examples/│ │  Vello)  │ │ WebXR    │
     └──────────┘ └──────────┘ └──────────┘
```

## Packages

| Package | Status | Description |
|---------|--------|-------------|
| [`core/`](core/) | ✅ Active | Renderer-agnostic reactive GUI framework — signals, virtual DOM, diff engine, binary mutation protocol, component framework, HTML vocabulary, platform abstraction (`GuiApp` trait, `launch()`) |
| [`web/`](web/) | ✅ Active | Browser renderer — compiles Mojo to WASM, TypeScript runtime interprets mutations into real DOM |
| [`desktop/`](desktop/) | ✅ Complete | Desktop renderer (Wayland-only) — native rendering via [Blitz](https://github.com/DioxusLabs/blitz) (Stylo + Taffy + Vello + Winit + AccessKit). No JS runtime, no IPC — mutations applied in-process via direct C FFI |
| [`examples/`](examples/) | ✅ Active | Shared example apps — identical source compiles for both web (WASM) and desktop (native) via `launch[AppType]()` |
| `xr/` | 🔮 Future | XR renderer — XR panel abstraction, OpenXR + Blitz shim for native, WebXR JS runtime for browser |

## How It Works

The **binary mutation protocol** is the renderer contract. The core framework never touches real DOM, widgets, or any platform API. Instead, it writes a stream of binary opcodes (`OP_LOAD_TEMPLATE`, `OP_SET_ATTRIBUTE`, `OP_SET_TEXT`, `OP_APPEND_CHILDREN`, etc.) to a byte buffer. Each renderer implements an interpreter that consumes this stream:

```text
┌──────────────────────┐     binary mutation buffer      ┌─────────────────────┐
│                      │  ───────────────────────────►   │                     │
│  mojo-gui/core       │     (shared linear memory       │  Renderer           │
│  (reactive framework │      or heap buffer)            │  (web / desktop /   │
│   + virtual DOM      │                                 │   xr)               │
│   + diff engine)     │  ◄───────────────────────────   │                     │
│                      │     event dispatch callbacks     │                     │
└──────────────────────┘                                 └─────────────────────┘
```

| Opcode | Web (DOM) | Desktop (Blitz) |
|--------|-----------|-----------------|
| `LOAD_TEMPLATE` | `cloneNode(true)` | Blitz DOM tree clone via C FFI |
| `SET_ATTRIBUTE` | `el.setAttribute()` | Blitz node attribute via C FFI |
| `SET_TEXT` | `node.textContent = ...` | Blitz text node via C FFI |
| `NEW_EVENT_LISTENER` | `addEventListener()` | Winit event binding via C FFI |
| `APPEND_CHILDREN` | `parent.appendChild()` | Blitz tree append via C FFI |
| `REMOVE` | `node.remove()` | Blitz node remove via C FFI |

### Unified App Lifecycle

All apps implement the `GuiApp` trait and use `launch()` as their single entry point:

```text
trait GuiApp(Movable):
    fn __init__(out self)
    fn render(mut self) -> UInt32
    fn handle_event(mut self, handler_id: UInt32, event_type: UInt8, value: String) -> Bool
    fn mount(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn flush(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn has_dirty(self) -> Bool
    fn consume_dirty(mut self) -> Bool
    fn destroy(mut self)
```

The `launch()` function uses compile-time target dispatch — on WASM it returns immediately (JS drives the loop), on native it calls `desktop_launch()` which creates a Blitz window and enters the event loop:

```text
fn launch[AppType: GuiApp](config: AppConfig = AppConfig()) raises:
    @parameter
    if is_wasm_target():
        pass  # JS runtime drives the loop via @export wrappers
    else:
        desktop_launch[AppType](config)  # Blitz window + blocking event loop
```

### Shared Examples

Each example has a single `main.mojo` that compiles for both targets:

```text
from platform.launch import launch, AppConfig
from counter import CounterApp

fn main() raises:
    launch[CounterApp](AppConfig(title="High-Five Counter", width=400, height=350))
```

Build for different targets:

```text
# Web (WASM) — multi-step pipeline (mojo → LLVM IR → llc → wasm-ld):
just build

# Desktop (native):
mojo build examples/counter/main.mojo -I core/src -I desktop/src -I examples
```

## Quick Start

### Prerequisites

All dependencies are provided by the Nix dev shell (`default.nix`). If not using Nix, you need:

- [Mojo](https://docs.modular.com/mojo/) 0.26.1+
- [Deno](https://deno.land/) (for web renderer)
- [LLVM](https://llvm.org/) (for `llc` — WASM compilation)
- [wabt](https://github.com/WebAssembly/wabt) (for `wasm-ld`)
- [wasmtime](https://wasmtime.dev/) (for Mojo tests)
- [just](https://github.com/casey/just) (task runner)
- [Rust toolchain](https://rustup.rs/) (for desktop Blitz shim)
- GPU/windowing libraries: Vulkan, Wayland, libxkbcommon, fontconfig (for desktop runtime — Wayland-only, X11 not supported)

### Build & Run (Web)

```sh
# Build WASM binary
just build

# Serve examples
just serve

# Open in browser:
#   http://localhost:4507/web/examples/counter/
#   http://localhost:4507/web/examples/todo/
#   http://localhost:4507/web/examples/bench/
#   http://localhost:4507/web/examples/app/
```

### Build & Run (Desktop)

```sh
# Build the Blitz C shim (first time only, or after shim changes)
just build-shim

# Build and run a desktop example
just run-desktop counter
just run-desktop todo
just run-desktop bench
just run-desktop app

# Build all desktop examples
just build-desktop-all
```

### Run Tests

```sh
# Mojo-side tests (via wasmtime)
just test

# JS integration tests (via Deno)
just test-js

# All tests
just test-all

# Browser end-to-end tests (headless Servo)
just test-browser
```

## Project Structure

```text
mojo-gui/
├── core/                         # Renderer-agnostic GUI framework
│   ├── src/
│   │   ├── signals/              # Reactive primitives (signals, memos, effects)
│   │   ├── scope/                # Scope lifecycle, arena allocator
│   │   ├── scheduler/            # Height-ordered dirty scope queue
│   │   ├── arena/                # ElementId type and allocator
│   │   ├── vdom/                 # Virtual DOM (Template, VNode, diff)
│   │   ├── mutations/            # CreateEngine, DiffEngine
│   │   ├── bridge/               # MutationWriter + binary opcode protocol
│   │   ├── events/               # HandlerRegistry, action tags
│   │   ├── component/            # AppShell, ComponentContext, KeyedList, Router
│   │   ├── html/                 # HTML tags, DSL constructors, VNodeBuilder
│   │   ├── platform/             # ★ GuiApp trait, launch(), target dispatch
│   │   └── lib.mojo              # Package root
│   ├── test/                     # Mojo-side unit tests (52+ suites)
│   └── README.md
│
├── web/                          # Browser renderer (WASM + TypeScript)
│   ├── src/
│   │   ├── main.mojo             # @export WASM wrappers
│   │   ├── gui_app_exports.mojo  # Generic @export helpers over GuiApp
│   │   └── web_launcher.mojo     # Web-side launch support
│   ├── runtime/                  # TypeScript runtime (DOM interpreter, events)
│   ├── examples/                 # Browser example apps (HTML + JS shells)
│   ├── test-js/                  # JS integration tests (3,090+ tests)
│   ├── scripts/                  # Build pipeline (nu scripts)
│   ├── justfile                  # Web build commands
│   └── README.md
│
├── desktop/                      # Desktop renderer (Blitz native HTML/CSS)
│   ├── shim/                     # Rust cdylib wrapping Blitz
│   │   ├── src/lib.rs            # BlitzContext, DOM ops, Winit event loop, Vello GPU rendering
│   │   ├── mojo_blitz.h          # C API header (~45 FFI functions)
│   │   ├── Cargo.toml            # blitz-dom, blitz-html, blitz-traits, blitz-paint, winit (Wayland-only), anyrender-vello
│   │   └── default.nix           # Nix derivation with Wayland + GPU deps
│   ├── src/desktop/
│   │   ├── blitz.mojo            # Mojo FFI bindings to libmojo_blitz.so
│   │   ├── renderer.mojo         # MutationInterpreter: binary opcodes → Blitz FFI calls
│   │   ├── launcher.mojo         # desktop_launch[AppType: GuiApp]() — generic Blitz event loop
│   │   └── __init__.mojo
│   └── README.md
│
├── examples/                     # Shared examples — identical source for all targets
│   ├── counter/                  # Reactive counter with conditional detail
│   │   ├── counter.mojo          # CounterApp struct (implements GuiApp)
│   │   └── main.mojo             # launch[CounterApp](AppConfig(...))
│   ├── todo/                     # Full todo app with input binding and keyed list
│   │   ├── todo.mojo             # TodoApp struct (implements GuiApp)
│   │   └── main.mojo             # launch[TodoApp](AppConfig(...))
│   ├── bench/                    # JS Framework Benchmark implementation
│   │   ├── bench.mojo            # BenchmarkApp struct (implements GuiApp)
│   │   └── main.mojo             # launch[BenchmarkApp](AppConfig(...))
│   ├── app/                      # Multi-view app with client-side routing
│   │   ├── app.mojo              # MultiViewApp struct (implements GuiApp)
│   │   └── main.mojo             # launch[MultiViewApp](AppConfig(...))
│   └── apps/                     # Test/demo apps (batch_demo, effect_demo, etc.)
│
├── justfile                      # Root task runner (web + desktop commands)
├── default.nix                   # Nix dev shell (web + desktop Wayland deps)
└── README.md                     # This file
```

## Import Conventions

After the separation from the original `mojo-wasm` monolith, imports are split between `vdom` (renderer-agnostic structures) and `html` (HTML-specific vocabulary):

```text
# Renderer-agnostic virtual DOM types
from vdom import VNode, VNodeStore, Template, TemplateBuilder

# HTML vocabulary — element constructors, VNodeBuilder
from html import el_div, el_button, text, dyn_text, VNodeBuilder, to_template

# Platform abstraction — launch, GuiApp trait
from platform import launch, AppConfig
from platform.gui_app import GuiApp

# Other core packages
from signals import Runtime, SignalI32, SignalBool, SignalString
from component import ComponentContext, AppShell, KeyedList
from bridge import MutationWriter
from events import HandlerRegistry
```

**Rule of thumb**: If it's an HTML element constructor (`el_*`), tag constant (`TAG_*`), `VNodeBuilder`, `to_template`, `Node`, or `NODE_*` constant → import from `html`. If it's `VNode`, `VNodeStore`, `Template`, `TemplateNode`, `DynamicNode`, `AttributeValue` → import from `vdom`.

## Current Status

- **Phases 1–2** ✅ — Monolith split into `core/`, `web/`, and `examples/`
- **Phase 3** ✅ — Unified lifecycle (`GuiApp` trait, `launch()`, compile-time target dispatch)
- **Phase 4** ✅ Complete — Blitz desktop renderer (Wayland-only). All 4 shared examples compile and run for both web and desktop from identical source. Runtime verified — all 4 desktop windows launch and render on Wayland with Vello GPU rendering via `just run-desktop <app>`
- **Phase 5** 🔮 — XR renderer (OpenXR native, WebXR browser)
- **Phase 6** 🔮 — `mojo-web` raw Web API bindings

## Origin

This project was extracted from the `mojo-wasm` monolith. The separation enables:

1. **Multi-renderer support** — The same app code runs on web, desktop, and (future) XR
2. **Clean dependency boundaries** — Core framework has zero browser/WASM dependencies
3. **Independent development** — Renderers can evolve independently
4. **Ecosystem foundation** — Other projects can build on `mojo-gui/core` for custom renderers

## License

See the repository root [LICENSE](../LICENSE).