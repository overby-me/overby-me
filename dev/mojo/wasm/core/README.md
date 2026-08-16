# mojo-gui/core — Renderer-Agnostic Reactive GUI Framework

The core package contains the renderer-agnostic reactive GUI framework. Everything here is independent of any specific rendering target (browser, desktop, XR) — it can be compiled for WASM, native, or any other Mojo target.

## Package Structure

```text
core/
├── src/
│   ├── signals/       # Reactive primitives (signals, memos, effects)
│   ├── scope/         # Scope lifecycle and arena allocator
│   ├── scheduler/     # Height-ordered dirty scope queue
│   ├── arena/         # ElementId type and allocator
│   ├── vdom/          # Virtual DOM primitives (template, vnode, builder, registry)
│   ├── html/          # HTML vocabulary — tags, DSL helpers, DSL tests (moved from vdom/)
│   ├── mutations/     # Mutation engines (create, diff)
│   ├── bridge/        # Binary mutation protocol (MutationWriter + opcodes)
│   ├── events/        # Event handler registry and action tags
│   └── component/     # Component framework (AppShell, ComponentContext, lifecycle, KeyedList, Router)
├── test/              # Mojo-side unit tests (test_signals, test_scopes, test_dsl, etc.)
└── README.md
```

## Key Modules

### Reactive System (`signals/`, `scope/`, `scheduler/`)

- **Signals** — `SignalI32`, `SignalBool`, `SignalString` reactive state primitives
- **Memos** — `MemoI32`, `MemoBool`, `MemoString` derived computations
- **Effects** — `EffectHandle` side-effect subscriptions
- **Scopes** — `ScopeState` with hook-based lifecycle (signal, memo, effect)
- **Scheduler** — Height-ordered dirty scope queue for efficient top-down re-rendering

### Virtual DOM (`vdom/`, `html/`)

The virtual DOM is split into two packages:

- **`vdom/`** — Renderer-agnostic primitives: `Template`, `TemplateNode`, `VNode`, `VNodeStore`, `TemplateBuilder`, `TemplateRegistry`
- **`html/`** — HTML-specific vocabulary: tag constants (`TAG_DIV`, `TAG_BUTTON`, ...), the declarative DSL (`el_div()`, `text()`, `dyn_text()`, `onclick_add()`, ...), `VNodeBuilder`, template conversion (`to_template`)

This split was made during the Phase 1 separation. Code that is renderer-agnostic imports from `vdom`, while code that uses HTML element constructors imports from `html`:

```mojo
from vdom import VNode, VNodeStore, Template
from html import el_div, el_button, text, dyn_text, onclick_add, VNodeBuilder
```

### Mutation Protocol (`bridge/`, `mutations/`)

- **`bridge/protocol.mojo`** — `MutationWriter` and binary opcodes (`OP_LOAD_TEMPLATE`, `OP_ASSIGN_ID`, `OP_SET_TEXT`, etc.)
- **`mutations/create.mojo`** — `CreateEngine` for initial mount
- **`mutations/diff.mojo`** — `DiffEngine` for reconciliation

The binary mutation protocol is the **abstraction boundary** between core and renderers. Core writes opcodes into a buffer; each renderer reads and interprets them for its target (DOM mutations for web, Blitz API calls for desktop, etc.).

### Component Framework (`component/`)

- **`AppShell`** — Owns the reactive runtime, VNode store, template registry, and scheduler
- **`ComponentContext`** — High-level API for building components (signal creation, view registration, event binding, diff/flush)
- **`ConditionalSlot` / `FragmentSlot`** — Manages conditional and list rendering
- **`KeyedList`** — Efficient keyed list diffing
- **`ChildComponent` / `ChildComponentContext`** — Parent-child component composition
- **`Router`** — Client-side URL path → branch routing

### Events (`events/`)

- **`HandlerRegistry`** — Maps handler IDs to action descriptors
- **Action tags** — `ACTION_SIGNAL_ADD_I32`, `ACTION_SIGNAL_TOGGLE`, `ACTION_CUSTOM`, etc.

## Usage

Core is a library — it's consumed by renderer packages (`web/`, `desktop/`) and shared example apps (`examples/`).

To include core packages in a Mojo build:

```sh
mojo build -I core/src ...
```

This makes all core packages available for import:

```mojo
from signals import SignalI32, Runtime
from component import ComponentContext
from html import el_div, el_button, text, dyn_text
from vdom import VNode, VNodeStore
from bridge import MutationWriter
```

## Relationship to Other Packages

```text
core/          ← You are here (renderer-agnostic framework)
examples/      ← Shared example apps (import from core)
web/           ← Browser renderer (WASM + TypeScript, imports from core)
desktop/       ← Desktop renderer (Blitz native, imports from core) [future]
```
