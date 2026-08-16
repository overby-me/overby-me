# Phase 1: Extract `mojo-gui/core` Library ✅

> **Status:** Complete. All steps verified.

## Step 1.1 — Create `mojo-gui/core` directory structure

Create the new project skeleton. The reactive core, vdom, component framework, and **platform abstraction layer** become a standalone Mojo library.

**Files to move (Mojo source):**

| From (`mojo-wasm/`)                  | To (`mojo-gui/core/`)                 |
|--------------------------------------|---------------------------------------|
| `src/signals/*`                      | `src/signals/*`                       |
| `src/scope/*`                        | `src/scope/*`                         |
| `src/scheduler/*`                    | `src/scheduler/*`                     |
| `src/arena/*`                        | `src/arena/*`                         |
| `src/vdom/template.mojo`            | `src/vdom/template.mojo`             |
| `src/vdom/vnode.mojo`               | `src/vdom/vnode.mojo`                |
| `src/vdom/builder.mojo`             | `src/vdom/builder.mojo`              |
| `src/vdom/registry.mojo`            | `src/vdom/registry.mojo`             |
| `src/mutations/*`                    | `src/mutations/*`                     |
| `src/bridge/*`                       | `src/bridge/*`                        |
| `src/events/*`                       | `src/events/*`                        |
| `src/component/*`                    | `src/component/*`                     |
| `src/vdom/tags.mojo`                | `src/html/tags.mojo`                 |
| `src/vdom/dsl.mojo`                 | `src/html/dsl.mojo`                  |
| `src/vdom/dsl_tests.mojo`           | `src/html/dsl_tests.mojo`            |
| *(new)*                              | `src/platform/launch.mojo`           |
| *(new)*                              | `src/platform/app.mojo`              |
| *(new)*                              | `src/platform/features.mojo`         |
| `test/*.mojo` (Mojo-side tests)     | `test/*`                              |

**Files to move to shared examples:**

| From (`mojo-wasm/`)                  | To (`mojo-gui/examples/`)            |
|--------------------------------------|---------------------------------------|
| `src/apps/counter.mojo`             | `counter/app.mojo`                   |
| `src/apps/todo.mojo`                | `todo/app.mojo`                      |
| `src/apps/bench.mojo`               | `bench/app.mojo`                     |
| `src/apps/*.mojo` (others)          | `<name>/app.mojo`                    |

**Import path changes:**

| Old import                           | New import                            |
|--------------------------------------|---------------------------------------|
| `from vdom.tags import TAG_DIV, ...` | `from html.tags import TAG_DIV, ...`  |
| `from vdom.dsl import el_div, ...`   | `from html.dsl import el_div, ...`    |

The `vdom/dsl.mojo` module currently imports from `vdom.tags`, `vdom.template`, `vdom.vnode`, and `events.registry`. When moved to `html/dsl.mojo`, imports from `vdom.*` stay the same (sibling package), only `html.tags` changes.

## Step 1.2 — Introduce the Platform Abstraction Layer

Create `core/src/platform/` with the `App` trait and `launch()` function. This is the key enabler for shared examples.

**`core/src/platform/app.mojo`** — The trait every renderer must implement:

```text
trait App:
    fn init(inout self, shell: AppShell) -> None
    fn flush_mutations(inout self, buffer: UnsafePointer[UInt8], len: Int) -> None
    fn poll_events(inout self) -> None
    fn request_redraw(inout self) -> None
    fn run(inout self) -> None
```

**`core/src/platform/launch.mojo`** — Compile-time target dispatch:

```text
fn launch[app_builder: fn(ctx: ComponentContext) -> None]():
    """Launch the app. Renderer is selected by build target."""
    @parameter
    if _is_wasm_target():
        # Web path: the JS runtime calls @export functions.
        # launch() registers the app_builder for the JS runtime to invoke.
        _register_web_app[app_builder]()
    else:
        # Desktop path: create a Blitz window and run the event loop.
        _run_desktop_app[app_builder]()
```

**Why this matters for shared examples:** With `launch()`, every example app calls the same function. The compile target determines the renderer. No `#ifdef`, no renderer-specific imports in app code.

## Step 1.3 — Make `mojo-gui/core` compile to both WASM and native

The core Mojo code should compile with both:

- `mojo build --target wasm64-wasi` (for web renderer)
- `mojo build` (for native, default target)

**Blockers to check:**

- The `MutationWriter` uses `UnsafePointer[UInt8, MutExternalOrigin]` — the `MutExternalOrigin` origin attribute might be WASM-specific. Need to verify it compiles natively.
- No `@export` decorators in the library code (those stay in `main.mojo` per-renderer).
- No WASM-specific memory layout assumptions (the code uses `alloc`/`UnsafePointer` which work natively too).

**Expected result:** The core library compiles cleanly for both targets with no changes beyond import paths.

## Step 1.4 — Mojo-side test suite

Move all `test/*.mojo` files to `mojo-gui/core/test/`. These tests use `wasmtime` to run WASM binaries — this works for both targets:

- **WASM target:** Tests compile app to WASM, run via wasmtime (existing flow)
- **Native target:** Tests can also compile and run as native binaries directly

Update `scripts/build_test_binaries.sh` and `scripts/run_test_binaries.sh` to support both modes.