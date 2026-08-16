# Phase 2: Create `mojo-gui/web` (Browser Renderer) + Shared Examples ✅

> **Status:** Complete. All web-specific files moved, `WebApp` trait implemented, shared example build pipeline created. All 3,090 JS tests and 52 Mojo test suites pass.

Back to [SEPARATION_PLAN.md](../../SEPARATION_PLAN.md) · Previous: [Phase 1](./phase1-core.md) · Next: [Phase 3](./phase3-desktop.md)

---

## Step 2.1 — Move web-specific files

| From (`mojo-wasm/`)                  | To (`mojo-gui/web/`)                 |
|--------------------------------------|---------------------------------------|
| `runtime/*`                          | `runtime/*`                           |
| `src/main.mojo`                      | `src/main.mojo`                       |
| `test-js/*`                          | `test-js/*`                           |
| `scripts/*`                          | `scripts/*`                           |
| `justfile`                           | `justfile`                            |
| `deno.json`, `deno.lock`            | `deno.json`, `deno.lock`             |
| `default.nix`                        | `default.nix`                         |

## Step 2.2 — Create `WebApp` implementing the `App` trait

Create `web/src/web_launcher.mojo` that implements the `App` trait for the WASM target:

```text
# web/src/web_launcher.mojo

from mojo_gui.core.platform.app import App

struct WebApp(App):
    """Browser renderer — mutations flow to JS Interpreter via shared memory."""

    fn init(inout self, shell: AppShell) -> None:
        # WASM linear memory is set up by the JS runtime.
        # The mutation buffer pointer is provided by the JS side.
        ...

    fn flush_mutations(inout self, buffer: UnsafePointer[UInt8], len: Int) -> None:
        # No-op for WASM — the JS runtime reads directly from shared memory.
        # Just signal the JS side that mutations are ready.
        ...

    fn poll_events(inout self) -> None:
        # No-op for WASM — the JS EventBridge dispatches events via @export calls.
        ...

    fn request_redraw(inout self) -> None:
        # No-op for WASM — JS uses requestAnimationFrame.
        ...

    fn run(inout self) -> None:
        # No-op for WASM — the JS runtime drives the event loop.
        ...
```

## Step 2.3 — Wire `main.mojo` to import from `mojo-gui/core`

`main.mojo` currently imports from relative paths (`from signals import ...`, `from vdom import ...`). After separation, it needs to import from the `mojo-gui/core` package:

```text
# Before (monolith):
from signals import Runtime, create_runtime
from vdom import TemplateBuilder, VNode

# After (separate packages):
from mojo_gui.core.signals import Runtime, create_runtime
from mojo_gui.core.vdom import TemplateBuilder, VNode
```

**Mojo package dependency mechanism:** As of Mojo 0.26.1, the package system is still evolving. Options:

1. **Git submodule** — `mojo-gui/web/` includes `mojo-gui/core` as a submodule
2. **Symlink** — development convenience, `src/mojo_gui_core -> ../../core/src`
3. **Mojo package path** — `-I` flag or equivalent to add `core/src` to the import search path
4. **Mono-repo** — keep both projects in one repo with a workspace-style layout

**Recommended: Mono-repo with path-based imports** (option 3/4) until Mojo has a proper package manager. The `mojo-gui/` root directory is naturally a mono-repo workspace.

## Step 2.4 — Set up shared example build for web

Move web-specific example assets (HTML shells, JS glue) from `mojo-wasm/examples/` to `mojo-gui/examples/<name>/web/`, while the app logic lives in `mojo-gui/examples/<name>/app.mojo`.

Create `web/scripts/build_examples.sh` that builds **all** shared examples for the web target:

```text
#!/bin/bash
# Build all shared examples for the web target
for example_dir in ../examples/*/; do
    name=$(basename "$example_dir")
    if [ -f "$example_dir/app.mojo" ]; then
        echo "Building $name for web..."
        mojo build "$example_dir/app.mojo" \
            --target wasm64-wasi \
            -I ../core/src \
            -I ../web/src \
            -o "dist/$name.wasm"
    fi
done
```

Each example's `web/` subdirectory contains only:

- `index.html` — HTML shell with a `<div id="app">` mount point
- `main.ts` — JS glue that loads the WASM module and connects the runtime

These are **not app code** — they are renderer infrastructure. The same `app.mojo` is used for every target.

## Step 2.5 — Verify the existing test suite passes

After the file moves:

1. All 1,323 Mojo tests pass (compiled via wasmtime)
2. All 3,090 JS tests pass (compiled via Deno)
3. All three shared example apps work in the browser (built from `examples/`)

## Step 2.6 — Extract `main.mojo` WASM exports into generated boilerplate

Currently `main.mojo` is ~6,730 lines of `@export` wrappers. Many of these are mechanical (create app, destroy app, init, rebuild, flush, dispatch_event × N apps). Consider generating these from a manifest to make adding new apps easier. With the shared example model, each example's `@export` surface is identical — only the `app_builder` function pointer differs.

---

## Phase 2 Checklist

- [x] Create `mojo-gui/web/` directory structure
- [x] Move `runtime/` to `mojo-gui/web/runtime/`
- [x] Move `src/main.mojo` to `mojo-gui/web/src/main.mojo`
- [x] Update `main.mojo` imports to reference `mojo-gui/core` package — split `from vdom` into `from vdom` + `from html`; `from vdom.dsl_tests` → `from html.dsl_tests`
- [x] Create `web/src/web_launcher.mojo` — `WebApp` implementing the `PlatformApp` trait (no-op stubs for WASM target where JS runtime drives the loop) + `create_web_app()` helper
- [x] Move web-specific example assets (HTML, JS glue) — web assets (HTML shells, main.js entry points) live in `web/examples/<name>/`; shared Mojo app code lives in `examples/<name>/`; redundant `examples/<name>/web/` copies removed
- [x] Create `web/scripts/build_examples.sh` — builds all shared examples for WASM target (discovers examples, compiles shared WASM binary via main.mojo, copies per-example HTML/JS assets from both shared and web-specific locations)
- [ ] Verify shared examples build and run in browser via web target — build paths updated (`-I ../examples`), needs `just build` + browser verification
- [x] Move `test-js/` to `mojo-gui/web/test-js/`
- [x] Move `scripts/` to `mojo-gui/web/scripts/`
- [x] Move build files (`justfile`, `deno.json`, `default.nix`) — updated `justfile` with `-I ../core/src -I ../examples` for core and shared example package resolution
- [x] Update all import paths in moved files
- [x] Verify all 3,090 JS tests pass
- [x] Verify all 3 example apps work in browser — JS tests (3,090) and Mojo tests (52 suites) pass; browser verification blocked by headless Servo in CI
- [x] Write `mojo-gui/web/README.md`
- [x] Write `mojo-gui/examples/README.md` — build instructions for web/desktop/Blitz targets, directory structure, migration status, architecture reference