# mojo-gui/web — Browser Renderer (WASM + TypeScript)

The web package is the browser renderer for mojo-gui. It compiles Mojo GUI apps to WebAssembly and provides a TypeScript runtime that interprets the binary mutation protocol to drive the real DOM.

## Directory Structure

```text
web/
├── src/
│   ├── main.mojo          # @export WASM wrappers — bridges WASM exports to app lifecycle
│   └── apps/              # Test/demo app modules (child_counter, effect_demo, etc.)
├── runtime/               # TypeScript runtime (runs in the browser)
│   ├── mod.ts             # Entry point — WASM instantiation
│   ├── interpreter.ts     # DOM stack machine — reads binary opcodes, mutates the DOM
│   ├── events.ts          # DOM event delegation — EventBridge
│   ├── templates.ts       # Template cache (DocumentFragment cloning)
│   ├── memory.ts          # WASM linear memory helpers
│   ├── env.ts             # WASM environment imports (aligned alloc/free, scratch allocator)
│   ├── strings.ts         # Mojo String ABI — read/write string structs across WASM boundary
│   ├── protocol.ts        # JS-side mutation opcode definitions (mirrors bridge/protocol.mojo)
│   ├── tags.ts            # HTML tag name lookup table (mirrors html/tags.mojo)
│   ├── app.ts             # App lifecycle helpers
│   └── types.ts           # WasmExports interface
├── test-js/               # JavaScript integration tests (Deno)
│   ├── run.ts             # Test runner — loads WASM, runs all test suites
│   ├── harness.ts         # Test harness utilities (assert, summary)
│   ├── counter.test.ts    # Counter app integration tests
│   ├── todo.test.ts       # Todo app integration tests
│   └── ...                # ~30 test files covering all apps and subsystems
├── scripts/               # Build pipeline
│   ├── build-test-binaries.nu  # Compile Mojo test binaries (parallel, incremental)
│   ├── run-test-binaries.nu    # Run precompiled test binaries (parallel)
│   └── precompile.mojo         # AOT compilation helper
├── build/                 # Build artifacts (generated)
│   ├── out.wasm           # Compiled WASM binary
│   ├── out.cwasm          # Precompiled (AOT) WASM binary
│   └── test-bin/          # Precompiled Mojo test binaries
├── justfile               # Build commands
├── deno.json              # Deno configuration
└── README.md
```

## How It Works

1. **Mojo → WASM**: `src/main.mojo` defines `@export` functions that expose app lifecycle operations (init, mount, handle_event, flush, destroy) to JavaScript.

2. **Binary Mutation Protocol**: When an app mounts or updates, core's `MutationWriter` fills a shared buffer with binary opcodes (`OP_LOAD_TEMPLATE`, `OP_SET_TEXT`, `OP_APPEND_CHILDREN`, etc.).

3. **TypeScript Interpreter**: `runtime/interpreter.ts` reads the buffer and translates opcodes into real DOM mutations (`createElement`, `setAttribute`, `appendChild`, etc.). Templates are cached as `DocumentFragment` instances for efficient cloning.

4. **Event Bridge**: `runtime/events.ts` provides `EventBridge` — when the interpreter processes `OP_NEW_EVENT_LISTENER`, it attaches DOM event listeners that call back into WASM (`handle_event` / `dispatch_string`), which updates reactive state and triggers a flush cycle.

## Build & Run

All commands are run from the `web/` directory:

```sh
# Build the WASM binary
just build

# Build + precompile for faster loading
just precompile

# Run Mojo tests (builds test binaries, then runs them)
just test

# Run JS integration tests
just test-js

# Run all tests
just test-all

# Serve examples in browser
just serve
```

### Build Flags

The build command compiles `src/main.mojo` with these include paths:

```sh
mojo build -I ../core/src -I ../examples -I src -o build/out.ll src/main.mojo
```

- `-I ../core/src` — Core framework packages (signals, vdom, html, component, bridge, mutations, events, scope, scheduler, arena)
- `-I ../examples` — Shared example apps (counter, todo, bench, app)
- `-I src` — Web-specific modules (apps/ test demos)

## Shared Examples

The web renderer builds the same example apps that run on every target:

| Example | URL | Description |
|---------|-----|-------------|
| Counter | `/examples/counter/` | Reactive counter with conditional detail |
| Todo    | `/examples/todo/`    | Full todo app with input binding and keyed list |
| Bench   | `/examples/bench/`   | JS Framework Benchmark implementation |
| App     | `/examples/app/`     | Multi-view app with client-side routing |

Each example has:

- **Shared Mojo code** in `../examples/<name>/` — platform-agnostic app logic
- **Web assets** in `../examples/<name>/` — `index.html` + `main.js` (minimal browser entry point)
- **Shared JS runtime** in `../examples/lib/` — `app.js` convention-based launcher

## Testing

### JS Tests (~3,090 tests)

```sh
just test-js
```

Runs `test-js/run.ts` via Deno. Loads the compiled WASM binary and exercises every app, the mutation interpreter, event bridge, DSL, and protocol through JavaScript.

### Mojo Tests (~52 suites)

```sh
just test
```

Compiles each `core/test/test_*.mojo` into a standalone binary (using mojo-wasmtime FFI bindings) and runs them in parallel. Tests verify signals, scopes, memos, effects, mutations, templates, DSL, events, and more.

## Relationship to Other Packages

```text
core/          ← Renderer-agnostic framework (signals, vdom, html, component, bridge)
examples/      ← Shared example apps (same code runs on web + desktop)
web/           ← You are here (browser renderer)
desktop/       ← Desktop renderer (Blitz native) [future]
```
