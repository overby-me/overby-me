# mojo-gui/desktop — Desktop Renderer (Blitz, Wayland-only)

Native desktop GUI backend for **mojo-gui** using [Blitz](https://github.com/DioxusLabs/blitz) (Stylo + Taffy + Vello + Winit + AccessKit).

> **Status: ✅ Complete (Wayland-only)** — The Rust cdylib (`libmojo_blitz.so`) compiles with full Winit event loop integration, and all 4 shared examples compile and run for both web and desktop from identical source. Runtime verified — all 4 desktop windows launch and render on Wayland with Vello GPU rendering via `just run-desktop <app>`.

## Architecture

The desktop renderer interprets the same binary mutation protocol as the web renderer, but instead of targeting a browser DOM, it drives a native rendering pipeline via direct C FFI calls — no JS runtime, no IPC, no base64 encoding.

```text
┌─ Native Mojo Process ─────────────────────────────────────────────┐
│                                                                    │
│  User App (counter.mojo, todo.mojo, ...)                          │
│      │                                                             │
│      ▼                                                             │
│  mojo-gui/core (compiled native — NOT WASM)                       │
│    ├── Signals, Memos, Effects                                     │
│    ├── Virtual DOM + Diff Engine                                   │
│    ├── MutationWriter → heap buffer                                │
│    └── HandlerRegistry (event dispatch)                            │
│         │                            ▲                             │
│         │ mutations (binary)         │ events                      │
│         ▼                            │                             │
│  MutationInterpreter (renderer.mojo)                               │
│    └── Reads binary opcodes → Blitz C FFI calls (all 18 opcodes)  │
│         │                            ▲                             │
│         ▼                            │                             │
│  ┌─ Blitz Rendering Pipeline (libmojo_blitz.so) ──────────────┐   │
│  │                                                             │   │
│  │  Stylo    — CSS parsing + cascade + selector matching       │   │
│  │  Taffy    — Flexbox / Grid / Block layout engine            │   │
│  │  Vello    — GPU-accelerated 2D rendering (via wgpu)         │   │
│  │  Winit    — Wayland window management                       │   │
│  │  AccessKit — Native accessibility tree                      │   │
│  │                                                             │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

### How it works

1. **Mojo compiles to a native binary** (not WASM). The core framework runs at full native speed.

2. **Mutations flow Mojo → Blitz**: The `MutationWriter` writes the same binary opcode stream as the web renderer, but into a heap buffer instead of WASM linear memory. The `MutationInterpreter` maps opcodes directly to Blitz's DOM tree via C FFI calls.

3. **Events flow Winit → Mojo**: Window and input events are captured by Winit's event loop. The Blitz shim's `MojoEventHandler` intercepts DOM events during bubble propagation, maps them to mojo-gui handler IDs, and buffers them for polling via `mblitz_poll_event()`.

4. **Rendering**: Stylo resolves CSS styles, Taffy computes layout, and Vello paints to a GPU surface via wgpu. No browser engine, no JS interpreter, no IPC overhead.

### Event loop

The `desktop_launch[AppType: GuiApp](config)` function in `launcher.mojo` drives the following loop:

```text
1. Create Blitz renderer (window + DOM + GPU rendering pipeline)
2. Instantiate the GuiApp via AppType()
3. Mount: app.mount(writer_ptr) → apply mutations to Blitz DOM
4. Event loop:
   ├── blitz.step(blocking=False)     — process OS events (Winit)
   ├── blitz.poll_event()             — drain buffered DOM events
   ├── app.handle_event(...)          — dispatch to HandlerRegistry
   ├── if app.has_dirty():
   │     app.flush(writer_ptr)        — re-render + diff
   │     apply mutations to Blitz DOM
   │     blitz.request_redraw()
   └── else: blitz.step(blocking=True) — sleep until next event
5. Cleanup: app.destroy() + blitz.destroy()
```

### Comparison with the web renderer

| Aspect | Web (`mojo-gui/web`) | Desktop (`mojo-gui/desktop`) |
|--------|---------------------|------------------------------|
| Mojo target | `wasm64-wasi` | Native (default) |
| Mutation buffer | WASM linear memory | Heap buffer |
| Mutation delivery | JS reads shared memory | C FFI calls (in-process) |
| CSS engine | Browser | Stylo (Firefox CSS engine) |
| Layout engine | Browser | Taffy (Flexbox/Grid) |
| Rendering | Browser compositor | Vello (GPU via wgpu) |
| Windowing | Browser tab | Winit (Wayland) |
| Accessibility | Browser ARIA | AccessKit |
| Event delivery | WASM export calls | Winit event loop → poll |
| Entry point | `@export` wrappers | `fn main()` via `launch()` |
| JS runtime | TypeScript interpreter | None |
| IPC overhead | None (shared memory) | None (in-process FFI) |
| Performance | WASM overhead | Full native speed |

The key insight: **the user's app code is identical**. Only the build target differs — exactly like Dioxus for Rust.

## Key Files

| File | Purpose |
|------|---------|
| `shim/src/lib.rs` | Rust `cdylib`: `BlitzContext` wrapping `blitz-dom`, ID mapping, template registry, event queue, Winit `ApplicationHandler`, Vello GPU rendering |
| `shim/mojo_blitz.h` | C API header (~45 FFI functions: lifecycle, DOM ops, templates, events, stack, debug) |
| `shim/Cargo.toml` | Rust crate config (blitz-dom, blitz-html, blitz-traits, blitz-shell, blitz-paint, winit 0.30 Wayland-only, anyrender-vello 0.6) |
| `shim/default.nix` | Nix derivation with Wayland + GPU deps (Vulkan, Wayland, fontconfig) |
| `src/desktop/blitz.mojo` | Mojo FFI bindings to `libmojo_blitz.so` — typed `Blitz` struct via `_DLHandle` |
| `src/desktop/renderer.mojo` | `MutationInterpreter`: reads binary opcodes → Blitz C FFI calls (all 18 opcodes) |
| `src/desktop/launcher.mojo` | `desktop_launch[AppType: GuiApp]()` — generic Blitz-backed event loop |

## Building

### Build the Blitz C shim

```sh
# From mojo-gui root:
just build-shim

# Or directly:
cd desktop/shim && cargo build --release --lib
```

This produces `desktop/shim/target/release/libmojo_blitz.so` (~23MB, release profile, thin LTO, stripped).

The `build-shim` recipe also runs `patchelf` to bake Nix store rpaths (Wayland, Vulkan, libxkbcommon, fontconfig, freetype, libGL) into the cdylib so that Winit's `dlopen` calls succeed without manual `LD_LIBRARY_PATH` setup.

**Build stats:** 607 crate dependencies, zero warnings, ~23MB ELF 64-bit x86-64 shared library.

### Build a desktop example

```sh
# From mojo-gui root:
just build-desktop counter
just build-desktop todo
just build-desktop bench
just build-desktop app

# Or build all:
just build-desktop-all
```

### Run a desktop example

```sh
just run-desktop counter
just run-desktop todo
just run-desktop bench
just run-desktop app

# Or run all (sequentially):
just run-desktop-all
```

### Build dependencies (Nix)

The `shim/default.nix` provides all build dependencies. For non-Nix environments, you need:

- Rust toolchain (edition 2024, rust-version 1.90.0+)
- pkg-config, cmake, python3
- fontconfig, freetype
- libxkbcommon, wayland
- Vulkan SDK (vulkan-loader, vulkan-headers)
- libGL

## Winit Event Loop Integration

The Blitz shim implements a full Winit `ApplicationHandler`:

- **Window creation** — `resumed()` creates the Winit window with `Arc<Window>`, initializes the Vello GPU renderer via `anyrender_vello::VelloWindowRenderer`
- **Event pumping** — `mblitz_step(blocking)` drives `pump_app_events()` with configurable timeout (non-blocking via `Duration::ZERO`, blocking via 100ms)
- **Window events** — `CloseRequested`, `RedrawRequested`, `Resized`, `ScaleFactorChanged`, `CursorMoved`, `MouseInput` are all handled
- **DOM event extraction** — Custom `MojoEventHandler` intercepts Blitz DOM events during bubble propagation, maps `DomEventData` variants (Click, Input, KeyDown, etc.) to mojo-gui handler IDs
- **GPU rendering** — `RedrawRequested` triggers `doc.resolve()` (Stylo + Taffy), then `paint_scene()` renders to the Vello scene

**Dependency alignment:** Versions are pinned to match Blitz v0.2.0's internal dependencies: anyrender 0.6, anyrender-vello 0.6, winit 0.30 (Wayland-only features enabled; X11 not supported).

## Design Goals

- **No embedded browser engine** — Blitz provides DOM/CSS semantics without a full browser runtime
- **Wayland-native** — Winit targets Wayland on Linux (X11 is not supported)
- **Native accessibility** — AccessKit bridges to platform a11y APIs (AT-SPI on Linux, NSAccessibility on macOS, UIA on Windows)
- **GPU rendering** — Vello renders via wgpu, supporting Vulkan, Metal, and DX12
- **Zero IPC overhead** — Mutations are interpreted in-process; no base64 encoding, no JS eval, no serialization
- **Shared examples** — All 4 examples (counter, todo, bench, app) compile from identical source for both web and desktop

## Remaining Work

1. **macOS support** — Verify on macOS (currently Linux Wayland-only)
2. **Cross-target CI** — Set up CI matrix testing web + desktop-Blitz for every shared example

## Related

- [Blitz](https://github.com/DioxusLabs/blitz) — Native HTML/CSS rendering engine
- [Stylo](https://wiki.mozilla.org/Quantum/Stylo) — Firefox's CSS engine
- [Taffy](https://github.com/DioxusLabs/taffy) — Flexbox/Grid layout engine
- [Vello](https://github.com/linebender/vello) — GPU 2D renderer
- [Winit](https://github.com/rust-windowing/winit) — Cross-platform windowing
- [AccessKit](https://github.com/AccessKit/accesskit) — Native accessibility
- [Separation Plan](../../mojo-wasm/SEPARATION_PLAN.md) — Full project separation plan
- [Phase 4 Details](../../mojo-wasm/docs/plan/phase4-blitz.md) — Detailed Phase 4 plan document