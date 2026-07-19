# mojo-gui Examples — Shared Cross-Platform Apps

This directory contains example applications that run on **every** renderer target unchanged. The same Mojo app code compiles and runs in the browser (via WASM) and natively on desktop — only the build command differs.

## Design Principle

> **If an example doesn't work on a target, it's a framework bug — not an app bug.**

Example apps import only from `mojo-gui/core` (signals, components, HTML DSL). They do NOT import from `mojo-gui/web` or `mojo-gui/desktop`. The `launch()` entry point and compile-time target selection handle the rest.

## Examples

### Main Examples

| Example | Description | Signals | Components | Features |
|---------|-------------|---------|------------|----------|
| **counter** | Counter with conditional detail section | `SignalI32`, `SignalBool` | `ConditionalSlot` | Reactive text, toggle show/hide, even/odd display |
| **todo** | Todo list with input binding | `SignalI32`, `SignalString` | `KeyedList` | Two-way input binding, Enter key handling, add/remove items |
| **bench** | js-framework-benchmark implementation | `SignalI32` | `KeyedList` | Create/update/swap/delete 1K–10K rows, performance timing |
| **app** | Single-page app with client-side routing | `SignalI32`, `SignalString` | `Router`, `ConditionalSlot` | URL-based view switching, persistent nav bar |

### Demo/Test Apps (`apps/`)

| App | Description |
|-----|-------------|
| **batch_demo** | Batch update demonstration |
| **child_counter** | Parent-child component communication |
| **child_context_test** | Child component context testing |
| **context_test** | Context provider/consumer testing |
| **data_loader** | Suspense-based async data loading |
| **effect_demo** | Effect lifecycle demonstration |
| **effect_memo** | Effect + memo interaction |
| **equality_demo** | Signal equality/deduplication |
| **error_nest** | Nested error boundaries |
| **memo_chain** | Chained memo dependencies |
| **memo_form** | Memo-driven form validation |
| **props_counter** | Props-based counter (parent→child) |
| **safe_counter** | Error boundary with crash/retry |
| **suspense_nest** | Nested suspense boundaries |
| **theme_counter** | Context-based theming |

## Directory Structure

```text
examples/
├── counter/
│   ├── counter.mojo           # Shared app logic (platform-agnostic)
│   └── __init__.mojo          # Package re-exports
├── todo/
│   ├── todo.mojo              # Shared app logic
│   └── __init__.mojo
├── bench/
│   ├── bench.mojo             # Shared app logic
│   └── __init__.mojo
├── app/
│   ├── app.mojo               # Multi-view router app
│   └── __init__.mojo
├── apps/                      # Demo/test apps (used by WASM test harness)
│   ├── __init__.mojo
│   ├── batch_demo.mojo
│   ├── child_counter.mojo
│   ├── child_context_test.mojo
│   ├── context_test.mojo
│   ├── data_loader.mojo
│   ├── effect_demo.mojo
│   ├── effect_memo.mojo
│   ├── equality_demo.mojo
│   ├── error_nest.mojo
│   ├── memo_chain.mojo
│   ├── memo_form.mojo
│   ├── props_counter.mojo
│   ├── safe_counter.mojo
│   ├── suspense_nest.mojo
│   └── theme_counter.mojo
└── README.md                  # This file
```

Web-specific assets (HTML shells, JS entry points) live in `mojo-gui/web/examples/`:

```text
web/examples/
├── counter/
│   ├── index.html             # HTML shell with #root mount point
│   └── main.js                # JS glue (loads WASM, connects runtime)
├── todo/
│   ├── index.html
│   └── main.js
├── bench/
│   ├── index.html
│   └── main.js
├── app/
│   ├── index.html
│   └── main.js
└── lib/                       # Shared JS runtime library
    ├── app.js                 # Convention-based WASM app launcher
    ├── boot.js                # WASM instantiation
    ├── env.js                 # WASM environment imports
    ├── events.js              # DOM event delegation
    ├── interpreter.js         # DOM mutation interpreter
    ├── protocol.js            # Binary protocol parser
    └── strings.js             # Mojo String ABI helpers
```

## Building Examples

### Web Target (WASM + Browser)

Build from the `mojo-gui/web/` directory:

```sh
cd mojo-gui/web

# Build all examples (compiles Mojo → WASM, bundles JS runtime)
just build

# Or use the build-examples.nu script:
nu scripts/build-examples.nu

# Serve locally:
just serve
# Open http://localhost:4507/examples/counter/
# Open http://localhost:4507/examples/todo/
# Open http://localhost:4507/examples/bench/
# Open http://localhost:4507/examples/app/
```

The web build:

1. Compiles Mojo source to WASM via `mojo build --target wasm64-wasi`
2. The JS runtime (`runtime/`) instantiates the WASM module
3. The `Interpreter` reads binary mutations from WASM shared memory
4. The `EventBridge` captures DOM events and dispatches to WASM

The build command uses `-I ../examples` to resolve the shared example imports (`from counter import ...`, `from apps.safe_counter import ...`, etc.).

### Desktop Target — Blitz (Future)

> **Status: 🔮 Future** — The Blitz-based desktop renderer is not yet implemented.

The desktop renderer will use Blitz (Stylo + Taffy + Vello + Winit + AccessKit) to render natively without a browser engine:

```sh
cd mojo-gui/desktop

# Build with Blitz native HTML/CSS engine:
mojo build ../examples/counter/counter.mojo \
    -I ../core/src \
    -I ../examples \
    -I src \
    --link-against libmojo_blitz.so \
    -o dist/counter
```

The desktop build will:

1. Compile Mojo source to a native binary (no WASM)
2. Create a Winit window with a Vello GPU rendering surface
3. Interpret binary mutations directly into Blitz's DOM tree
4. Resolve CSS via Stylo and compute layout via Taffy
5. Events flow from Winit's event loop to the core framework

See `SEPARATION_PLAN.md` Phase 3 for details.

## How Web Assets Work

Each example's web assets in `web/examples/<name>/` contain only renderer infrastructure — no app logic:

- **`index.html`** — Minimal HTML shell with a `<div id="root">` mount point and app-specific styling.
- **`main.js`** — Entry point that uses the shared `launch()` function from `web/examples/lib/app.js`. Convention-based WASM export discovery means zero app-specific JS is needed:

```js
import { launch } from "../lib/app.js";

launch({
    app: "counter",
    wasm: new URL("../../build/out.wasm", import.meta.url),
});
```

The `launch()` function automatically discovers WASM exports by naming convention (`counter_init`, `counter_rebuild`, `counter_flush`, `counter_handle_event`) and wires up the Interpreter and EventBridge.

## Writing a New Example

1. **Create the app struct** in `mojo-gui/examples/<name>/<name>.mojo`:

```mojo
from component import ComponentContext, ConditionalSlot
from signals import SignalI32
from html import el_div, el_h1, el_button, text, dyn_text, onclick_add

struct MyApp(Movable):
    var ctx: ComponentContext
    var count: SignalI32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.setup_view(
            el_div(
                el_h1(dyn_text()),
                el_button(text(String("+1")), onclick_add(self.count, 1)),
            ),
            String("my-app"),
        )
```

2. **Create `__init__.mojo`** in the same directory to re-export the app and lifecycle functions.

3. **Add `@export` wrappers** in `web/src/main.mojo` for the web target.

4. **Add web assets** in `web/examples/<name>/`:
   - `index.html` — copy from an existing example and customize the title/styles
   - `main.js` — use the shared `launch()` with your app name

5. **Add desktop entry point** (optional) in `desktop/examples/<name>.mojo`.

6. **Verify on all targets** — the app must work identically on every renderer.

## Migration Status

Progress toward fully shared, platform-agnostic examples:

### Phase 1: Core extraction ✅

- [x] Platform abstraction layer (`core/src/platform/`)
- [x] `PlatformApp` trait definition
- [x] `launch()` function with `AppConfig`
- [x] `PlatformFeatures` runtime capability detection
- [x] Extract shared example apps from web/desktop to `examples/`
- [x] Move demo/test apps from `core/apps/` to `examples/apps/`

### Phase 2: Web extraction ✅

- [x] `WebApp` implementing `PlatformApp` trait
- [x] Web-specific assets in `web/examples/<name>/`
- [x] Updated build paths (`-I ../examples`)
- [ ] Refactor web examples to use `launch()` pattern
- [ ] Generate `@export` boilerplate from manifest

### Phase 3: Desktop (Blitz)

- [ ] Blitz C shim and FFI bindings
- [ ] `DesktopApp` (Blitz) implementing `PlatformApp` trait
- [ ] Mojo-side mutation interpreter
- [ ] Verify all shared examples on desktop
- [ ] Cross-target CI test matrix

## Architecture Reference

```text
┌────────────────────────────────────────────────┐
│  Shared Example App (examples/)                 │
│                                                 │
│  imports only:                                  │
│    mojo-gui/core (signals, components, DSL)     │
│    platform.launch() (entry point)              │
│                                                 │
│  NEVER imports:                                 │
│    mojo-gui/web (JS runtime, WASM exports)      │
│    mojo-gui/desktop (Blitz FFI)                 │
└───────────────┬─────────────────────────────────┘
                │ compile target selects renderer
        ┌───────┼───────────┐
        ▼       ▼           ▼
   ┌────────┐ ┌─────────┐ ┌────────┐
   │  web   │ │ desktop │ │ native │
   │ (WASM) │ │ (Blitz) │ │(future)│
   └────────┘ └─────────┘ └────────┘
```
