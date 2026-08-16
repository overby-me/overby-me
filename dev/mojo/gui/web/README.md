# mojo-gui/web — Browser Renderer

The browser renderer for `mojo-gui`: compiles Mojo GUI apps to WebAssembly and runs them in the browser using a TypeScript runtime that interprets the binary mutation protocol into real DOM operations.

## How It Works

```text
┌─────────────────────────┐     shared WASM linear memory     ┌──────────────────────────┐
│                         │  ────────────────────────────────► │                          │
│  Mojo (compiled to WASM)│     binary mutation buffer         │  TypeScript Runtime      │
│                         │                                    │                          │
│  mojo-gui/core          │                                    │  Interpreter → real DOM  │
│  (signals, vdom, diff)  │  ◄──────────────────────────────── │  EventBridge → dispatch  │
│                         │     event dispatch callbacks        │                          │
└─────────────────────────┘                                    └──────────────────────────┘
```

1. Mojo compiles to WASM via `mojo build` → `llc` → `wasm-ld`
2. TypeScript runtime instantiates the WASM module and provides environment imports
3. Mojo core writes mutations to shared linear memory (binary opcode buffer)
4. JS `Interpreter` reads the mutation buffer and applies operations to the real DOM
5. JS `EventBridge` captures DOM events and dispatches them back into WASM

## Directory Structure

```text
web/
├── runtime/                  # TypeScript runtime (runs in the browser)
│   ├── mod.ts                # Entry point — WASM instantiation
│   ├── interpreter.ts        # DOM stack machine (applies binary mutations)
│   ├── events.ts             # DOM event delegation bridge
│   ├── templates.ts          # Template cache (DocumentFragment cloning)
│   ├── memory.ts             # WASM memory management, free-list allocator
│   ├── env.ts                # WASM environment imports (I/O, math, libc)
│   ├── strings.ts            # Mojo String ABI helpers (SSO)
│   ├── protocol.ts           # JS-side mutation opcode parser
│   ├── tags.ts               # HTML tag name mapping (JS side)
│   ├── app.ts                # App lifecycle helpers, per-app handles
│   └── types.ts              # WasmExports interface
├── src/
│   └── main.mojo             # @export WASM wrappers (~430 exports)
├── examples/                 # Browser example apps
│   ├── counter/              # Simple counter with conditional detail
│   ├── todo/                 # Todo list with keyed items, two-way binding
│   ├── bench/                # js-framework-benchmark implementation
│   ├── app/                  # Multi-page app with router
│   └── lib/                  # Shared JS runtime for examples
├── test-js/                  # JS integration tests (Deno)
│   ├── harness.ts            # Test harness for WASM + DOM testing
│   ├── run.ts                # Test runner
│   └── *.test.ts             # Individual test suites
├── scripts/                  # Build pipeline
│   ├── build-test-binaries.nu
│   ├── run-test-binaries.nu
│   └── precompile.mojo
├── justfile                  # Build commands
├── deno.json                 # Deno configuration
├── deno.lock                 # Deno lockfile
├── default.nix               # Nix dev shell
└── README.md
```

## Prerequisites

- [Mojo](https://docs.modular.com/mojo/) 0.26.1+
- [Deno](https://deno.land/) (for TypeScript runtime and tests)
- [LLVM](https://llvm.org/) (`llc` for WASM codegen)
- [wabt](https://github.com/WebAssembly/wabt) (`wasm-ld` for linking)
- [wasmtime](https://wasmtime.dev/) (for precompilation and Mojo-side tests)
- [just](https://github.com/casey/just) (command runner)

All dependencies are provided by the Nix dev shell (`default.nix`).

## Build & Run

```sh
# Build WASM binary
just build

# Serve examples locally
just serve
# Then open:
#   http://localhost:4507/examples/counter/
#   http://localhost:4507/examples/todo/
#   http://localhost:4507/examples/bench/
#   http://localhost:4507/examples/app/
```

## Testing

```sh
# Build + run Mojo-side tests (via wasmtime)
just test

# Run JS integration tests (via Deno)
just test-js

# Run all tests
just test-all

# Run browser end-to-end tests (headless Servo)
just test-browser

# Filter tests by module name
just test signals           # only test_signals
just test signals mutations # test_signals + test_mutations
```

## Relationship to `mojo-gui/core`

This package is a **renderer** for `mojo-gui/core`. It does not contain any reactive framework logic — that all lives in `core/`. The web renderer's job is:

1. **Instantiate WASM** — Load the compiled Mojo binary, provide environment imports
2. **Interpret mutations** — Read the binary opcode buffer and apply DOM operations
3. **Bridge events** — Capture DOM events and dispatch them back to Mojo's `HandlerRegistry`
4. **Provide WASM exports** — `src/main.mojo` contains `@export` wrappers that expose core functionality to the JS runtime

The `justfile` passes `-I ../core/src` to `mojo build` so that all core packages (`signals`, `vdom`, `html`, `component`, etc.) are found at compile time.

## Import Conventions

`src/main.mojo` imports from both `core` packages and `html`:

```text
# Core virtual DOM types
from vdom import VNode, VNodeStore, TemplateBuilder, ...

# HTML vocabulary (DSL constructors, VNodeBuilder)
from html import Node, text, dyn_text, el_div, VNodeBuilder, ...

# Other core packages
from signals import Runtime, create_runtime, destroy_runtime
from component import AppShell, ComponentContext
from bridge import MutationWriter
```

Example apps follow the same pattern — `from vdom import` for core types, `from html import` for HTML-specific constructors.

## TypeScript Runtime

The `runtime/` directory contains the browser-side code that makes WASM↔DOM communication work:

| Module | Purpose |
|--------|---------|
| `interpreter.ts` | DOM stack machine — processes binary opcodes into DOM operations |
| `events.ts` | Event delegation — captures DOM events, sends handler IDs back to WASM |
| `templates.ts` | Template cache — clones `DocumentFragment`s for fast instantiation |
| `memory.ts` | WASM memory management — free-list allocator for shared linear memory |
| `env.ts` | Environment imports — I/O, math, libc functions provided to WASM |
| `strings.ts` | Mojo String ABI — reads SSO and heap strings from WASM memory |
| `protocol.ts` | Opcode parser — decodes the binary mutation buffer format |
| `tags.ts` | Tag mapping — converts tag IDs to HTML element names |
| `app.ts` | App lifecycle — per-app state management and handles |
| `mod.ts` | Entry point — WASM module loading and instantiation |