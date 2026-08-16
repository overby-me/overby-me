# mojo-gui/core — AI Agent Context

> Compact quick-reference for AI agents working on the renderer-agnostic core library.
> For project overview see [README.md](README.md). For the separation plan see
> [../../mojo-wasm/SEPARATION_PLAN.md](../../mojo-wasm/SEPARATION_PLAN.md).

## Mojo Constraints

- **No closures/function pointers in WASM** — event handlers are action-based structs. 0.26.1 improves function type conversions (non-raising → raising, ref → value) but true closures still missing.
- **`@export` only works in main.mojo** — submodule exports get DCE'd. No `@export` decorators exist in core; those belong to per-renderer entry points (e.g., `mojo-gui/web/src/main.mojo`).
- **Single-threaded** — no sync needed.
- **Operator overloading** works (SignalI32 has `+=`, `-=`, `peek()`, `set()`).
- **Format**: `mojo format <file>` — pre-commit hooks run this automatically.
- **Commit messages**: `feat(mojo-gui): Uppercase description` — commitlint enforced, allowed types: `feat`, `fix`, `chore`, `doc`.

## Key Abstractions (dependency order)

### Signals & Reactivity (`src/signals/`)

- **`Runtime`** (`runtime.mojo`) — Central god-object: `SignalStore`, `StringStore`, `MemoStore`, `EffectStore`, `ScopeArena`, `TemplateRegistry`, `VNodeStore`, `Scheduler`. Created via `create_runtime()`, destroyed via `destroy_runtime()`.
- **`SignalStore`** — Flat `List[SignalEntry]`. Each entry: `value: Int32`, `subscribers: List[UInt32]` (scope indices).
- **`StringStore`** — Parallel string storage for `SignalString`. Index-based: same slot index as `SignalStore`, value stored separately.
- **`MemoStore`** (`memo.mojo`) — `List[MemoEntry]`. Each memo: compute function tag + dependency tracking + cached value. Equality-gated: only propagates if value actually changes.
- **`EffectStore`** (`effect.mojo`) — `List[EffectEntry]`. Effects are drained during flush via `drain_and_run` pattern.
- **Handles** (`handle.mojo`) — `SignalI32`, `SignalBool`, `SignalString`, `MemoI32`, `MemoBool`, `MemoString`, `EffectHandle` — typed wrappers around `(runtime_ptr, slot_index)`.

### Scopes (`src/scope/`)

- **`ScopeState`** — Lifecycle unit: owns hooks (signal/memo/effect indices), parent/child links, height in tree.
- **`ScopeArena`** — Slab allocator for scopes. Supports create/destroy/reparent.
- Hook kinds: `HOOK_SIGNAL`, `HOOK_MEMO`, `HOOK_EFFECT`.

### Virtual DOM (`src/vdom/`)

- **`Template`** (`template.mojo`) — Immutable tree: `List[TemplateNode]` with element/text/dynamic/dynamic_text node types + static/dynamic attributes.
- **`VNode`** (`vnode.mojo`) — Runtime instance: template ID + dynamic nodes + dynamic attrs. Stored in `VNodeStore` (double-buffered `List[VNode]`).
- **`TemplateBuilder`** (`builder.mojo`) — Step-by-step template construction. Low-level API.
- **`TemplateRegistry`** (`registry.mojo`) — Stores templates by ID, deduplicates by name.

### HTML Vocabulary (`src/html/`)

- **`tags.mojo`** — 38 tag constants (`TAG_DIV` = 0, `TAG_SPAN` = 1, ..., `TAG_CODE` = 37) + `tag_name()` lookup.
- **`dsl.mojo`** — High-level declarative API. `Node` struct represents a tree node. Constructors: `el_div()`, `el_button()`, `text()`, `dyn_text()`, `onclick_add()`, `bind_value()`, etc. `to_template()` converts `Node` tree → registered `Template`. `VNodeBuilder` — fluent API for VNode construction.
- **`dsl_tests.mojo`** — Self-contained test functions for the DSL.

**Import convention**: HTML-specific symbols come from `html`, not `vdom`:

```text
from html import el_div, el_button, dyn_text, VNodeBuilder, to_template
from vdom import VNode, VNodeStore, Template
```

### Mutations (`src/mutations/`)

- **`CreateEngine`** (`create.mojo`) — Walks a VNode tree, emits create/append/set mutations to a `MutationWriter`.
- **`DiffEngine`** (`diff.mojo`) — Compares old VNode (buffer 0) vs new VNode (buffer 1), emits minimal update mutations.

### Events (`src/events/`)

- **`HandlerRegistry`** (`registry.mojo`) — `List[HandlerEntry]` storing handler → action mappings.
- **Action tags**: `ACTION_SIGNAL_ADD_I32`, `ACTION_SIGNAL_SUB_I32`, `ACTION_SIGNAL_SET_I32`, `ACTION_SIGNAL_TOGGLE`, `ACTION_SIGNAL_SET_INPUT`, `ACTION_SIGNAL_SET_STRING`, `ACTION_KEY_ENTER_CUSTOM`, `ACTION_CUSTOM`.
- **Event types**: `EVT_CLICK`, `EVT_INPUT`, `EVT_KEY_DOWN`, `EVT_CHANGE`, `EVT_SUBMIT`, etc.

### Component Layer (`src/component/`)

- **`AppShell`** (`app_shell.mojo`) — Bundles `Runtime` pointer + `MutationWriter` + `ElementIdAllocator` + `Scheduler` + `HandlerRegistry` + `VNodeStore`.
- **`ComponentContext`** (`context.mojo`) — Ergonomic API: `use_signal()`, `use_memo()`, `use_effect()`, `setup_view()`, `rebuild()`. Owns an `AppShell`. Uses `RenderBuilder` for batched VNode construction.
- **`ChildComponent`** (`child.mojo`) — Renders a child subtree within a parent's scope. Uses `ChildRenderBuilder`.
- **`ChildComponentContext`** (`child_context.mojo`) — Like `ComponentContext` but for children that share a parent's `AppShell`.
- **`lifecycle.mojo`** — `mount_vnode`, `diff_and_finalize`, `FragmentSlot`, `ConditionalSlot`, `flush_fragment`, `flush_conditional`.
- **`KeyedList`** (`keyed_list.mojo`) — Efficient keyed list rendering with per-item `VNodeBuilder` and handler actions.
- **`Router`** (`router.mojo`) — URL path → branch index routing.

## Node Kind Tags (`src/html/dsl.mojo`)

| Tag | Value | Meaning |
|-----|-------|---------|
| `NODE_TEXT` | 0 | Static text leaf |
| `NODE_ELEMENT` | 1 | HTML element with children |
| `NODE_DYN_TEXT` | 2 | Dynamic text slot (fills `DynamicNode.text_node`) |
| `NODE_DYN_NODE` | 3 | Dynamic node slot (placeholder) |
| `NODE_STATIC_ATTR` | 4 | Static attribute (`name="value"`) |
| `NODE_DYN_ATTR` | 5 | Dynamic attribute slot |
| `NODE_EVENT` | 6 | Inline event handler (action-based) |
| `NODE_BIND_VALUE` | 7 | Two-way value binding (input ↔ signal) |

## Handler Action Tags (`src/events/registry.mojo`)

| Tag | Value | Meaning |
|-----|-------|---------|
| `ACTION_NONE` | 0 | No-op |
| `ACTION_SIGNAL_SET_I32` | 1 | Set signal to literal value |
| `ACTION_SIGNAL_ADD_I32` | 2 | Add literal to signal |
| `ACTION_SIGNAL_SUB_I32` | 3 | Subtract literal from signal |
| `ACTION_SIGNAL_TOGGLE` | 4 | Toggle bool signal |
| `ACTION_SIGNAL_SET_INPUT` | 5 | Set string signal from input.value |
| `ACTION_SIGNAL_SET_STRING` | 6 | Set string signal from event string |
| `ACTION_KEY_ENTER_CUSTOM` | 7 | Fire custom action on Enter key |
| `ACTION_CUSTOM` | 8 | App-defined custom action tag |

## Binary Mutation Protocol (`src/bridge/protocol.mojo`)

The `MutationWriter` writes opcodes to a byte buffer. Every renderer must implement an interpreter that consumes this stream.

Key opcodes: `OP_LOAD_TEMPLATE`, `OP_SET_ATTRIBUTE`, `OP_SET_TEXT`, `OP_NEW_EVENT_LISTENER`, `OP_APPEND_CHILDREN`, `OP_REMOVE`, `OP_CREATE_TEXT_NODE`, `OP_REPLACE_WITH`, `OP_INSERT_BEFORE`, `OP_REGISTER_TEMPLATE`.

The buffer is the **renderer contract** — renderers never touch core internals, only consume the opcode stream and dispatch events back.

## Directory Structure

```text
core/
├── src/
│   ├── signals/          # Runtime, SignalStore, MemoStore, EffectStore, handles
│   ├── scope/            # ScopeState, ScopeArena
│   ├── scheduler/        # Scheduler (height-ordered dirty queue)
│   ├── arena/            # ElementId, ElementIdAllocator
│   ├── vdom/             # Template, VNode, VNodeStore, TemplateBuilder, TemplateRegistry
│   ├── mutations/        # CreateEngine, DiffEngine
│   ├── bridge/           # MutationWriter + binary opcodes
│   ├── events/           # HandlerRegistry, action tags, event type constants
│   ├── component/        # AppShell, ComponentContext, lifecycle, KeyedList, Router
│   ├── html/             # HTML tags, DSL constructors, VNodeBuilder
│   └── lib.mojo          # Package root
├── apps/                 # Demo/test apps (counter, todo, bench, ...)
├── test/                 # Mojo-side unit tests (run via wasmtime)
├── AGENTS.md             # This file
└── README.md
```

## Common Patterns

### Creating a component

```text
struct MyApp(Movable):
    var ctx: ComponentContext
    var count: SignalI32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal_i32(0)
        var view = el_div([
            el_h1([dyn_text(0)]),
            el_button([text("+"), onclick_add(self.count, 1)]),
        ])
        self.ctx.setup_view(view, String("my-app"))
```

### Flush cycle (called by renderer)

1. `app.dispatch_event(handler_id, event_type, payload)` — mutates signals
2. `app.rebuild()` — `scheduler.consume_dirty()` → run effects → re-render dirty scopes → diff → write mutations
3. Renderer reads mutation buffer and applies to DOM/widgets

### Signal → Memo → Effect chain

```text
var count = ctx.use_signal_i32(0)
var doubled = ctx.use_memo_i32(MEMO_DOUBLED, count)  # auto-tracks count
var effect = ctx.use_effect(EFFECT_LOG, doubled)       # runs when doubled changes
```

### KeyedList pattern

```text
var list = KeyedList(ctx, item_builder_fn, max_items)
list.set_items(items)           # diffed, keyed by item ID
list.flush(mutation_writer)     # emits create/update/remove mutations
```

## Compilation Targets

- **WASM** (`mojo build --target wasm64-wasi`) — for `mojo-gui/web`
- **Native** (`mojo build`) — for `mojo-gui/desktop` (future)

No `@export` decorators in core. Those belong to renderer entry points.

## Import Path Changes (from mojo-wasm monolith)

| Old import | New import |
|-----------|-----------|
| `from vdom import el_div, text, dyn_text, ...` | `from html import el_div, text, dyn_text, ...` |
| `from vdom import VNode, VNodeStore, Template` | `from vdom import VNode, VNodeStore, Template` |
| `from vdom.tags import TAG_DIV` | `from html.tags import TAG_DIV` |
| `from vdom.dsl import el_div` | `from html.dsl import el_div` |
| `from vdom.dsl_tests import ...` | `from html.dsl_tests import ...` |

**Rule of thumb**: if it's an HTML element constructor, tag constant, `VNodeBuilder`, `to_template`, `Node`, or `NODE_*` constant → import from `html`. If it's `VNode`, `VNodeStore`, `Template`, `TemplateNode`, `DynamicNode`, `AttributeValue` → import from `vdom`.

## Deferred Abstractions (Blocked on Mojo Roadmap)

- **Renderer trait** — The mutation buffer protocol IS the trait, de facto. No formal `trait Renderer` until Mojo traits are more mature.
- **Closures for event handlers** — Currently action-tag-based. Will switch to closures when Mojo supports them in WASM.
- **Package manager** — Mono-repo with path-based imports until Mojo has a proper package manager.
- **Generics for signal types** — Currently separate `SignalI32`/`SignalBool`/`SignalString`. Will unify with generics when available.