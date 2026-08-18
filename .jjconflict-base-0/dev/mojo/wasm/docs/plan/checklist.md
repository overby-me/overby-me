# Migration Checklist

> Extracted from [SEPARATION_PLAN.md](../../SEPARATION_PLAN.md). See also: [architecture](./architecture.md), [renderers](./renderers.md), and individual phase docs.

---

## Phase 1: `core/` extraction ✅

- [x] Create `core/src/` directory structure
- [x] Copy `src/signals/`, `src/scope/`, `src/scheduler/`, `src/arena/` unchanged to `core/src/`
- [x] Copy `src/mutations/`, `src/bridge/`, `src/events/` unchanged to `core/src/`
- [x] Copy `src/component/` to `core/src/` — updated `child.mojo`, `child_context.mojo`, `context.mojo`, `keyed_list.mojo` to split `from vdom` / `from html` imports
- [x] Move `src/vdom/{template,vnode,builder,registry}.mojo` to `core/src/vdom/`
- [x] Move `src/vdom/{tags,dsl,dsl_tests}.mojo` to `core/src/html/` (new package)
- [x] Create `core/src/vdom/__init__.mojo` — re-exports only template, vnode, builder, registry (tags/DSL removed)
- [x] Create `core/src/html/__init__.mojo` — re-exports tags, DSL helpers, VNodeBuilder, to_template, count_* utilities
- [x] Update `html/dsl.mojo` imports: `from .builder` → `from vdom.builder`, `from .template` → `from vdom.template`, `from .vnode` → `from vdom.vnode`; `.tags` stays relative
- [x] Update `html/dsl_tests.mojo` imports: `from .template` → `from vdom.template`, `from .vnode` → `from vdom.vnode`, `from .builder` → `from vdom.builder`
- [x] Update `vdom/template.mojo`: `from .tags` → `from html.tags` (TAG_UNKNOWN)
- [x] Update `vdom/builder.mojo`: `from .tags` → `from html.tags` (TAG_UNKNOWN)
- [x] Update `component/context.mojo`: split `from vdom import` into `from vdom import` (VNode, VNodeStore) + `from html import` (Node, DSL types, VNodeBuilder, to_template)
- [x] Update `component/child.mojo`: same split as context.mojo
- [x] Update `component/child_context.mojo`: `VNodeBuilder` import moved from `vdom` to `html`
- [x] Update `component/keyed_list.mojo`: `VNodeBuilder` import moved from `vdom` to `html`
- [x] Move `test/` to `core/test/` (52 test suites)
- [x] Update `core/test/test_handles.mojo`: `from vdom import` → `from html import` for DSL symbols (2 locations)
- [x] Write `core/README.md`
- [x] Verify all 52 Mojo test suites pass after restructuring

---

## Phase 2: `web/` extraction ✅

- [x] Create `web/src/` directory structure
- [x] Move `src/main.mojo` to `web/src/main.mojo` — split `from vdom import` into `from vdom import` (primitives) + `from html import` (DSL); changed `from vdom.dsl_tests` → `from html.dsl_tests`
- [x] Move `src/apps/` to `web/src/apps/` — updated all 14 test app files: `from vdom import (` → `from html import (`
- [x] Move `runtime/` to `web/runtime/`
- [x] Move `test-js/` to `web/test-js/`
- [x] Move `scripts/` to `web/scripts/` — updated `build-test-binaries.nu` paths (test_dir → `core/test`, core_src_dir → `core/src`, web_src_dir → `web/src`, examples_dir → root `examples/`)
- [x] Move `deno.json` to `web/deno.json`
- [x] Create `web/justfile` with updated build flags: `-I ../core/src -I ../examples -I src`
- [x] Fix `build-if-changed` recipe: nu `glob` only takes one positional arg → use `[(glob ...), (glob ...)] | flatten`
- [x] Update example `main.js` files: WASM path `../../build/out.wasm` → `../../web/build/out.wasm` (counter, todo, bench, app, lib/app.js)
- [x] Update root `justfile` to delegate all commands to `web/justfile`
- [x] Update root `.gitignore` to cover `web/build/`
- [x] Write `web/README.md`
- [x] Delete old directories: `src/`, `runtime/`, `test/`, `test-js/`, `scripts/`, `build/`, `deno.json`, `deno.lock`
- [x] Verify `just build` (from root) produces `web/build/out.wasm`
- [x] Verify all 3,090 JS tests pass — `just test-js`
- [x] Verify all 52 Mojo test suites pass — `just test`

---

## Phase 3: `mojo-gui/desktop` — webview renderer ✅ (infra), unified lifecycle ✅

> Implemented in the `mojo-gui/` sibling project. The webview infrastructure (Steps 3.1–3.8) was later superseded by the Blitz renderer (Phase 4), but the unified lifecycle work (Step 3.9) is the lasting contribution.

### Steps 3.1–3.8 — Desktop webview infrastructure ✅

- [x] Design desktop webview architecture — polling-based C shim, heap mutation buffer, base64 IPC, JSON event bridge
- [x] Build C shim (`shim/mojo_webview.c`) — GTK4 + WebKitGTK, ring buffer events, base64 mutation delivery, non-blocking step API
- [x] Write C header (`shim/mojo_webview.h`) — lifecycle, window, content, event loop, event polling, mutations, diagnostics
- [x] Write Nix derivation (`shim/default.nix`) — automated build of libmojo_webview.so
- [x] Implement Mojo FFI bindings (`src/desktop/webview.mojo`) — typed `Webview` struct via `OwnedDLHandle`, library search (env var → NIX_LDFLAGS → LD_LIBRARY_PATH)
- [x] Implement desktop bridge (`src/desktop/bridge.mojo`) — `DesktopBridge` (heap mutation buffer, flush, poll), `DesktopEvent` (parsed JSON), `parse_event()` (minimal JSON parser)
- [x] Implement `DesktopApp` (`src/desktop/app.mojo`) — webview lifecycle, JS runtime injection, shell HTML loading, multiple event loop styles (blocking, mount+run, interactive, manual step)
- [x] Create desktop JS runtime (`runtime/desktop-runtime.js`) — standalone 900+ line JS: MutationReader, TemplateCache, Interpreter (all opcodes), event dispatch via `window.mojo_post()`
- [x] Create HTML shell (`runtime/shell.html`) — minimal `#root` mount point with dark mode support
- [x] Verify counter example runs on desktop (`desktop/examples/counter.mojo`) — full interactive event loop with ConditionalSlot (temporary duplicate; replaced by shared example via `launch()`)
- [x] Create build system (`justfile`) — build-shim, build-counter, run-counter, dev-counter, test-shim, test-runtime
- [x] Create Nix dev shell (`default.nix`) — GTK4, WebKitGTK 6.0, pkg-config, libmojo-webview, environment variables
- [x] Write `mojo-gui/desktop/README.md` — architecture, build instructions, API reference, IPC protocol docs

### Step 3.9 — Unified app lifecycle ✅

- [x] Define `GuiApp` trait (`core/src/platform/gui_app.mojo`) — app-side lifecycle contract with `mount`, `handle_event`, `flush`, `has_dirty`, `consume_dirty`, `destroy` (Step 3.9.1)
- [x] Implement generic desktop event loop (`desktop/src/desktop/launcher.mojo`) — `desktop_launch[AppType: GuiApp]()` with Blitz mutation interpreter (Step 3.9.2)
- [x] Wire `launch()` compile-time dispatch — `launch[AppType: GuiApp]()` with `@parameter if is_wasm_target()` in `core/src/platform/launch.mojo`; native targets now call `desktop_launch[AppType](config)` (Step 3.9.3)
- [x] Refactor app structs to implement `GuiApp` — all 4 main apps (CounterApp, TodoApp, BenchmarkApp, MultiViewApp) now implement `GuiApp`; backwards-compatible free functions removed (Step 3.9.4)
- [x] Genericize `main.mojo` `@export` wrappers over `GuiApp` — `web/src/gui_app_exports.mojo` provides `gui_app_init`, `gui_app_mount`, `gui_app_handle_event`, `gui_app_flush`, etc.; all 4 main app @exports are now one-liners; 3,090 JS tests + 52 Mojo test suites pass (Step 3.9.5)
- [x] Add `launch[AppType](...)` to shared examples (Step 3.9.6) — `main.mojo` entry points added to all 4 shared examples; no per-renderer duplicates existed to delete
- [x] Verify all 4 shared examples build on both web and desktop from identical source (Step 3.9.7 — build verification complete)

### Step 3.9.7 — Cross-target verification (runtime ✅, CI remaining)

- [x] Verify all 4 shared examples run interactively on desktop (Wayland) via `just run-desktop <app>`
- [ ] Set up cross-target CI test matrix (web + desktop for every shared example)

---

## Phase 4: `desktop/` — Blitz renderer ✅ Complete (Wayland-only)

> Replaced the webview dependency with [Blitz](https://github.com/DioxusLabs/blitz), a native HTML/CSS rendering engine. Implemented in the `mojo-gui/desktop/` directory. No JS runtime, no IPC — mutations are applied in-process via direct C FFI calls. Wayland-only on Linux (X11 not supported).

### Steps 4.1–4.3.1 — Blitz shim + Mojo bindings ✅

- [x] Build Blitz C shim (`desktop/shim/src/lib.rs`) — Rust `cdylib` wrapping `blitz-dom` via `extern "C"` functions; `BlitzContext` with DOM tree, ID mapping, template registry, event queue, interpreter stack
- [x] Write C header (`desktop/shim/mojo_blitz.h`) — ~45 FFI functions: lifecycle, DOM operations, templates, events, stack, debug
- [x] Write Nix derivation (`desktop/shim/default.nix`) — Rust build with Wayland + GPU deps (Vulkan, Wayland, fontconfig)
- [x] Write `desktop/shim/Cargo.toml` — cdylib depending on blitz-dom, blitz-html, blitz-traits, blitz-shell, blitz-paint, winit (Wayland-only), anyrender-vello
- [x] Implement Mojo FFI bindings (`desktop/src/desktop/blitz.mojo`) — typed `Blitz` struct via `_DLHandle`
- [x] Implement Mojo-side mutation interpreter (`desktop/src/desktop/renderer.mojo`) — `MutationInterpreter`: reads binary opcodes → Blitz FFI calls (all 18 opcodes)
- [x] Build the Rust cdylib (`cargo build --release`) — `libmojo_blitz.so` ~23MB (release, thin LTO, stripped), 607 crate dependencies, zero warnings

### Step 4.4 — Shared example builds + runtime ✅

- [x] All 4 shared examples compile for desktop-Blitz from identical source
- [x] Mojo 0.26.1 API migration completed as part of build verification
- [x] Counter example runs interactively on desktop (Wayland)
- [x] Todo example runs interactively on desktop (Wayland)
- [x] Bench example runs interactively on desktop (Wayland)
- [x] Multi-view app example runs interactively on desktop (Wayland)

### Step 4.6 — Winit event loop integration ✅

- [x] `ApplicationHandler` impl for `BlitzContext` — `resumed()` creates Winit window with `Arc<Window>`, initializes Vello GPU renderer via `anyrender_vello::VelloWindowRenderer`, viewport update
- [x] `mblitz_step(blocking)` wired to `pump_app_events()` — non-blocking via `Duration::ZERO`, blocking via 100ms timeout
- [x] Winit window event routing — `CloseRequested`, `RedrawRequested`, `Resized`, `ScaleFactorChanged`, `CursorMoved`, `MouseInput`
- [x] DOM event extraction via `MojoEventHandler` — custom `EventHandler` intercepts Blitz DOM events during bubble propagation, maps `DomEventData` to mojo-gui handler IDs, buffers in `event_queue`
- [x] GPU rendering via Vello + blitz-paint — `RedrawRequested` triggers `doc.resolve()` (Stylo + Taffy), then `paint_scene()` renders to Vello scene
- [x] Dependency version alignment — downgraded to match Blitz v0.2.0 pins: anyrender 0.6, winit 0.30; ported from winit 0.31 API

### Step 4.7 — Project infrastructure ✅

- [x] Create root `mojo-gui/justfile` — web commands (delegate to `web/justfile`), desktop commands (`build-shim` + patchelf rpaths, `build-desktop <app>`, `run-desktop <app>`, `run-desktop-all`, `build-desktop-all`), cross-target commands (`build-web <app>`, `build-all`), cleanup (`clean`)
- [x] Create root `mojo-gui/default.nix` — Nix dev shell combining web deps (deno, wabt, llvm, lld, wasmtime, servo) and desktop Wayland deps (rustup, pkg-config, cmake, python3, fontconfig, freetype, libxkbcommon, wayland, vulkan-loader, vulkan-headers, libGL)
- [x] Update `mojo-gui/README.md` — desktop status "🔮 Future" → "✅ Complete"; added `examples/` and `platform/` to package table; added "Unified App Lifecycle", "Shared Examples", "Build & Run (Desktop)" sections; updated project structure tree and import conventions
- [x] Update `mojo-gui/desktop/README.md` — status "🔮 Future" → "✅ Complete (Wayland-only)"; updated architecture diagram to show `MutationInterpreter`; added "Event loop", "Key Files", "Building", "Winit Event Loop Integration", "Remaining Work" sections

### Step 4.5 + CI — macOS + CI (remaining)

- [ ] macOS support (currently Linux Wayland-only)
- [ ] Set up cross-target CI test matrix (web + desktop-Blitz for every shared example)

---

## Phase 5: `mojo-gui/xr` — XR renderer (future)

- [ ] Design XR panel abstraction — `XRPanel` struct (DOM document + 3D transform + texture + input surface), `XRScene` (panel registry + spatial layout + raycasting)
- [ ] Build OpenXR + Blitz Rust shim (`xr/native/shim/src/lib.rs`) — extend Blitz stack with `openxr` crate; multi-document management (one `blitz-dom` per panel); Vello → offscreen `wgpu::Texture` per panel; OpenXR session lifecycle + frame loop; quad layer compositing; controller pose tracking + panel raycasting
- [ ] Write C header (`xr/native/shim/mojo_xr.h`) — session, panel, mutation, event, frame loop, input, reference space functions
- [ ] Write Nix derivation (`xr/native/shim/default.nix`) — Rust build with OpenXR + GPU deps
- [ ] Implement Mojo FFI bindings (`xr/native/src/xr_blitz.mojo`) — typed `XRBlitz` struct via `DLHandle`
- [ ] Implement XR scene manager (`xr/native/src/scene.mojo`) — panel lifecycle, mutation routing, event multiplexing
- [ ] Implement XR panel manager (`xr/native/src/panel.mojo`) — per-panel `GuiApp` + mutation buffer, 3D transform API
- [ ] Implement `xr_launch[AppType: GuiApp]()` (`xr/native/src/xr_launcher.mojo`) — OpenXR frame loop (wait → poll input → raycast → dispatch → render dirty panels → composite → end frame)
- [ ] Build WebXR JS runtime (`xr/web/runtime/`) — XR session lifecycle, DOM-to-texture panel rendering, XR input → DOM event bridging
- [ ] Wire `launch()` for XR targets — add `has_feature("xr")` / `has_feature("webxr")` branches to `core/src/platform/launch.mojo`
- [ ] Verify all shared examples as single-panel XR apps — counter, todo, bench, app should work unchanged in XR
- [ ] Multi-panel XR API (stretch goal) — `XRGuiApp` trait for apps that manage multiple panels in 3D space