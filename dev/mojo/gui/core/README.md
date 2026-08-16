# mojo-gui/core — Renderer-Agnostic Reactive GUI Framework

The core library for `mojo-gui`: a reactive GUI framework written in Mojo that compiles to both WASM and native targets. It provides signals, a virtual DOM, a diff engine, and a binary mutation protocol — with **zero dependency on any specific renderer** (browser DOM, desktop Blitz, or native widgets).

## Architecture

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

The **mutation buffer** is the renderer contract. Every renderer implements an interpreter that consumes the same binary opcode stream (`OP_CREATE_TEXT_NODE`, `OP_SET_ATTRIBUTE`, `OP_LOAD_TEMPLATE`, etc.). The opcodes are DOM-oriented by design — all renderer targets can interpret them.

## Sub-packages

| Package | Purpose |
|---------|---------|
| `signals/` | Reactive primitives — `SignalI32`, `SignalBool`, `SignalString`, `MemoI32`, `MemoBool`, `MemoString`, `EffectHandle` |
| `scope/` | Scope lifecycle (`ScopeState`) and slab allocator (`ScopeArena`) |
| `scheduler/` | Height-ordered dirty scope queue for top-down reconciliation |
| `arena/` | `ElementId` type and allocator for tracking DOM node identities |
| `vdom/` | Virtual DOM — `Template`, `TemplateNode`, `VNode`, `VNodeStore`, `TemplateBuilder`, `TemplateRegistry` |
| `mutations/` | `CreateEngine` (initial mount) and `DiffEngine` (reconciliation) — VNode → mutation buffer |
| `bridge/` | `MutationWriter` + binary opcode protocol |
| `events/` | `HandlerRegistry`, `HandlerEntry`, action tags (`ACTION_SIGNAL_ADD_I32`, etc.), event type constants |
| `component/` | `AppShell`, `ComponentContext`, `ChildComponent`, `KeyedList`, `Router`, lifecycle helpers |
| `html/` | HTML vocabulary — tag constants (`TAG_DIV`, ...), DSL element constructors (`el_div()`, `el_button()`, ...), `VNodeBuilder`, `to_template()` |
| `platform/` | Platform abstraction — `GuiApp` trait (app-side lifecycle contract), `launch[AppType]()` (compile-time target dispatch), `AppConfig`, `is_wasm_target()` / `is_native_target()`, `PlatformFeatures` |

## Key Abstractions

### Signals & Reactivity (`signals/`)

Fine-grained reactivity inspired by SolidJS. Signals hold values; memos derive from signals; effects run side-effects when dependencies change.

- **`Runtime`** — Central store for all signals, memos, effects, scopes, and templates.
- **`SignalI32` / `SignalBool` / `SignalString`** — Typed signal handles (index into the store).
- **`MemoI32` / `MemoBool` / `MemoString`** — Derived values, equality-gated to prevent unnecessary propagation.
- **`EffectHandle`** — Side-effect that runs when tracked signals change.

### Virtual DOM (`vdom/`)

Static templates + dynamic slots, inspired by Dioxus/Solid:

- **`Template`** — Immutable tree structure with holes for dynamic content.
- **`VNode`** — Runtime instance of a template with filled-in dynamic values.
- **`VNodeStore`** — Arena allocator for VNode trees (double-buffered for diffing).

### Mutation Protocol (`bridge/` + `mutations/`)

- **`MutationWriter`** — Writes binary opcodes to a byte buffer.
- **`CreateEngine`** — Walks a VNode tree and emits create/append/set mutations.
- **`DiffEngine`** — Compares old and new VNode trees, emits minimal update mutations.

### Component Framework (`component/`)

- **`AppShell`** — Bundles runtime + signal store + allocator + scheduler.
- **`ComponentContext`** — Ergonomic API for building components (signals, views, events).
- **`KeyedList`** — Efficient keyed list rendering with item-level diffing.
- **`Router`** — URL path → branch routing.

### HTML Vocabulary (`html/`)

HTML-specific layer on top of the renderer-agnostic core:

- **Tag constants** — `TAG_DIV`, `TAG_SPAN`, `TAG_BUTTON`, etc. (38 tags).
- **DSL constructors** — `el_div()`, `el_button()`, `dyn_text()`, `onclick_add()`, etc.
- **`VNodeBuilder`** — Fluent API for constructing VNodes with dynamic slots.
- **`to_template()`** — Converts a DSL node tree into a registered `Template`.

### Platform Abstraction (`platform/`)

Compile-time target dispatch that enables shared examples across all renderers:

- **`GuiApp` trait** — App-side lifecycle contract (`render`, `mount`, `flush`, `handle_event`, `has_dirty`, `consume_dirty`, `destroy`). Every app struct implements this trait.
- **`launch[AppType: GuiApp]()`** — Universal entry point. Uses `@parameter if is_wasm_target()` for compile-time dispatch: on WASM it returns immediately (JS drives the loop); on native it calls `desktop_launch[AppType](config)` which creates a Blitz window and enters the event loop.
- **`AppConfig`** — Platform-independent configuration (title, width, height, debug).
- **`is_wasm_target()` / `is_native_target()`** — Compile-time target detection.
- **`PlatformFeatures`** — Runtime feature detection for optional capabilities.

## Directory Structure

```text
core/
├── src/
│   ├── signals/          # Reactive primitives
│   ├── scope/            # Scope lifecycle
│   ├── scheduler/        # Dirty scope queue
│   ├── arena/            # ElementId allocator
│   ├── vdom/             # Virtual DOM (renderer-agnostic)
│   ├── mutations/        # CreateEngine, DiffEngine
│   ├── bridge/           # MutationWriter + binary protocol
│   ├── events/           # HandlerRegistry, action tags
│   ├── component/        # AppShell, ComponentContext, lifecycle
│   ├── html/             # HTML tags, DSL, VNodeBuilder
│   ├── platform/         # ★ GuiApp trait, launch(), target dispatch
│   │   ├── gui_app.mojo  # GuiApp trait — app-side lifecycle contract
│   │   ├── app.mojo      # is_wasm_target(), is_native_target()
│   │   ├── launch.mojo   # launch[AppType: GuiApp]() + AppConfig
│   │   ├── features.mojo # PlatformFeatures, runtime feature detection
│   │   └── __init__.mojo # Re-exports public API
│   └── lib.mojo          # Package root
├── test/                 # Mojo-side unit tests (52+ suites)
└── README.md
```

## Compilation Targets

The core library compiles to both WASM and native:

- **WASM** (`mojo build --target wasm64-wasi`) — for the web renderer (`mojo-gui/web`)
- **Native** (`mojo build`) — for the desktop renderer (`mojo-gui/desktop`)

No `@export` decorators exist in the library code — those belong to per-renderer entry points (e.g., `mojo-gui/web/src/main.mojo`).

## Import Conventions

After the separation from the `mojo-wasm` monolith:

| Old import (monolith) | New import (separated) |
|----------------------|----------------------|
| `from vdom import el_div, ...` | `from html import el_div, ...` |
| `from vdom import VNode, VNodeStore` | `from vdom import VNode, VNodeStore` |
| `from vdom.tags import TAG_DIV` | `from html.tags import TAG_DIV` |
| `from vdom.dsl import el_div` | `from html.dsl import el_div` |

The split: **`vdom/`** holds renderer-agnostic virtual DOM structures. **`html/`** holds the HTML-specific vocabulary (tags, DSL constructors, VNodeBuilder).

## Relationship to Other Packages

- **`mojo-gui/web`** — Browser renderer. Imports `core` for the framework, adds TypeScript runtime + WASM `@export` wrappers. Generic `gui_app_exports.mojo` makes per-app exports one-liners via `GuiApp` trait.
- **`mojo-gui/desktop`** — Desktop renderer (✅ complete, Wayland-only). Imports `core`, renders natively via Blitz (Stylo + Taffy + Vello + Winit + AccessKit). `desktop_launch[AppType: GuiApp]()` drives the native event loop.
- **`mojo-gui/examples`** — Shared examples. Each app implements `GuiApp` and calls `launch[AppType](AppConfig(...))`. Same source compiles for both web and desktop.
- **`mojo-web`** — Raw Web API bindings (future). Apps can use both `mojo-gui` and `mojo-web`.