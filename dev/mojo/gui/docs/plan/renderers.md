# Renderer Strategies

> **Parent document:** [PLAN.md](../../PLAN.md)

---

## Web Renderer (existing — move to `mojo-gui/web/`) ✅

**How it works today:**

1. Mojo compiles to WASM via `mojo build` → `llc` → `wasm-ld`
2. TypeScript runtime instantiates WASM, provides env imports
3. Mojo writes mutations to shared linear memory
4. JS `Interpreter` reads mutation buffer, applies to real DOM
5. JS `EventBridge` captures DOM events, dispatches to WASM

**Changes needed (all done):**

- ✅ Implement `WebApp` in `web/src/web_launcher.mojo` conforming to the `PlatformApp` trait
- ✅ `web/src/main.mojo` wires `@export` functions to the app structs
- ✅ Build scripts updated to compile shared examples from `examples/` for the WASM target
- ✅ Per-example `web/` subdirectories contain only HTML shell and JS glue (no app logic)

---

## Desktop Webview Renderer (implemented — `mojo-gui/desktop/`) ✅

Strategy: embed a GTK4 + WebKitGTK webview inside a native window. This is the pragmatic first step — it reuses the same JS mutation interpreter from the web renderer inside a native process, without requiring a browser tab.

**How it works:**

1. Mojo compiles to a **native binary** (not WASM)
2. The binary loads `libmojo_webview.so` (C shim around GTK4 + WebKitGTK) via FFI
3. Creates a native GTK4 window with an embedded WebKitGTK webview
4. Injects `desktop-runtime.js` (standalone mutation interpreter + event bridge) into the webview
5. Mojo writes mutations to a **heap buffer** (not WASM linear memory)
6. The `DesktopBridge` base64-encodes the buffer and sends it to the webview's JS interpreter via `mwv_apply_mutations()`
7. DOM events are captured by JS, serialized as JSON (`{"h":42,"t":0}`), and sent to the native side via `window.mojo_post()`
8. The C shim buffers events in a ring buffer; Mojo polls them with `mwv_poll_event()`

**Architecture:**

```text
┌─ Native Mojo Process ─────────────────────────────────────────────┐
│                                                                    │
│  User App Code (counter.mojo, todo.mojo, ...)                     │
│      │                                                             │
│      ▼                                                             │
│  mojo-gui/core (compiled native — NOT WASM)                       │
│    ├── Signals, Memos, Effects                                     │
│    ├── Virtual DOM + Diff Engine                                   │
│    ├── MutationWriter → heap buffer                                │
│    └── HandlerRegistry (event dispatch)                            │
│         │                            ▲                             │
│         │ mutations (binary)         │ events (JSON)               │
│         ▼                            │                             │
│  DesktopBridge                                                     │
│    ├── Owns heap-allocated mutation buffer (64 KiB)                │
│    ├── flush_mutations() → base64 → webview eval                   │
│    └── poll_event() ← JSON ← ring buffer ← JS                    │
│         │                            ▲                             │
│         ▼                            │                             │
│  ┌─ Embedded Webview (GTK4 + WebKitGTK) ──────────────────────┐   │
│  │                                                             │   │
│  │  desktop-runtime.js                                         │   │
│  │    ├── MutationReader (decodes binary protocol from base64) │   │
│  │    ├── Interpreter (applies mutations to real DOM)           │   │
│  │    ├── TemplateCache (DocumentFragment cloning)              │   │
│  │    └── Event dispatch → window.mojo_post(JSON)              │   │
│  │                                                             │   │
│  │  shell.html                                                 │   │
│  │    └── <div id="root"></div>  (mount point)                 │   │
│  │                                                             │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

**Key difference from web:** The Mojo code runs as a native process (not WASM), and writes mutations to a heap buffer instead of WASM linear memory. The bridge base64-encodes the buffer and sends it to an embedded webview via `webview_eval()`. There is no separate browser process — the webview is embedded in the GTK4 window.

**Key difference from Blitz (future):** The webview approach still uses a real browser engine (WebKitGTK) for rendering. Blitz would replace this with a standalone HTML/CSS engine (Stylo + Taffy + Vello), eliminating the WebKitGTK dependency entirely.

**C shim API surface (`shim/mojo_webview.h`):**

| Category   | Functions                                                    |
|------------|--------------------------------------------------------------|
| Lifecycle  | `mwv_create(title, w, h, debug)`, `mwv_destroy(w)`          |
| Window     | `mwv_set_title(w, title)`, `mwv_set_size(w, w, h, hints)`   |
| Content    | `mwv_set_html(w, html)`, `mwv_navigate(w, url)`, `mwv_init(w, js)`, `mwv_eval(w, js)` |
| Event loop | `mwv_run(w)`, `mwv_step(w, blocking)`, `mwv_terminate(w)`   |
| Events     | `mwv_poll_event(w, buf, len)`, `mwv_event_count(w)`, `mwv_event_clear(w)` |
| Mutations  | `mwv_apply_mutations(w, buf, len)` — base64-encode + eval    |
| Diagnostics| `mwv_is_alive(w)`, `mwv_get_window(w)`                       |

**Advantages of webview approach:**

- **Reuses existing JS runtime** — the same mutation interpreter and event bridge from the web renderer, adapted as `desktop-runtime.js`
- **Full CSS support** — WebKitGTK provides a complete browser engine with all CSS features
- **Rapid development** — leverages proven web runtime code instead of writing a new native interpreter
- **Good enough for many apps** — suitable for dashboards, tools, and apps where native rendering isn't critical

**Limitations:**

- **WebKitGTK dependency** — ~50+ MB on disk; Linux only (GTK4 + WebKitGTK 6.0)
- **Base64 IPC overhead** — every mutation buffer is base64-encoded (+33% size) and sent via `webview_eval()`
- **No direct DOM access** — mutations flow through JS string eval, not direct API calls
- **Single platform** — GTK4/WebKitGTK is Linux-only; macOS (WKWebView) and Windows (WebView2) would need separate shim implementations

---

## Desktop Blitz Renderer (implemented — `mojo-gui/desktop/`, Phase 4) ✅

Strategy: native HTML/CSS rendering via [Blitz](https://github.com/DioxusLabs/blitz). This is the same approach Dioxus uses for `dioxus-native`. Blitz is a radically modular HTML/CSS rendering engine that provides:

- **Stylo** (Firefox's CSS engine) — CSS parsing and style resolution
- **Taffy** — Flexbox, grid, and block layout
- **Parley** — Text layout and shaping
- **Vello** via **anyrender** — GPU-accelerated 2D rendering
- **Winit** — Cross-platform windowing and input
- **AccessKit** — Accessibility

Blitz provides a real DOM (`blitz-dom`) without requiring a browser or webview. The mutation protocol maps naturally to Blitz's DOM operations.

1. Mojo compiles to a **native binary** (no WASM)
2. The native binary links against a Blitz C shim (Rust `cdylib` exposing `blitz-dom` + `blitz-shell` via C ABI)
3. Mojo mutation interpreter reads the byte buffer and calls Blitz DOM operations via FFI (createElement, setAttribute, appendChild, etc.)
4. Blitz handles style resolution, layout, and GPU rendering
5. Winit/Blitz events flow back to Mojo via callback or polling

**Architecture:**

```text
┌──────────────────────────────────────────────────────────┐
│  Native Process                                           │
│                                                           │
│  ┌─────────────────────┐                                  │
│  │  mojo-gui/core       │                                  │
│  │  (compiled native)   │                                  │
│  │                      │─── mutation buffer ──┐           │
│  │  signals, vdom,      │                      │           │
│  │  diff, scheduler     │◄── event dispatch ──┐│           │
│  └─────────────────────┘                     ││           │
│                                              ▼│           │
│  ┌──────────────────────────────────────────┐ │           │
│  │  desktop/renderer.mojo                    │ │           │
│  │  (Mutation interpreter → Blitz FFI calls) │ │           │
│  └──────────┬───────────────────────────────┘ │           │
│             │ C FFI                            │           │
│  ┌──────────▼───────────────────────────────┐ │           │
│  │  Blitz (Rust cdylib via C shim)           │ │           │
│  │  ┌────────────────────────────────────┐   │ │           │
│  │  │  blitz-dom    — DOM tree + styles  │   │ │           │
│  │  │  Stylo        — CSS resolution     │   │ │           │
│  │  │  Taffy        — Layout engine      │   │ │           │
│  │  │  Vello        — GPU rendering      │   │ │           │
│  │  │  Winit        — Window + input ────│───┘ │           │
│  │  └────────────────────────────────────┘     │           │
│  └─────────────────────────────────────────────┘           │
└────────────────────────────────────────────────────────────┘
```

**Key difference from webview approach:** No webview, no JS runtime, and no IPC — mutations are applied in-process via direct C FFI calls. This eliminates the base64 encoding overhead and WebKitGTK dependency.

**Key difference from web:** The Mojo code runs as a native process (not WASM), and manipulates the Blitz DOM directly via C FFI instead of shared WASM linear memory + JS interpreter.

**Adaptation needed in `mojo-gui/core`:**

- The `MutationWriter` currently writes to WASM linear memory (`UnsafePointer[UInt8, MutExternalOrigin]`). For native, it writes to a heap buffer. The writer itself doesn't care — it just writes bytes to a pointer. ✅ Already works (proven by the webview desktop renderer).
- The Blitz desktop renderer implements a Mojo-side mutation interpreter that reads the byte buffer and translates each opcode to the corresponding Blitz C FFI call (similar to how the JS `Interpreter` class reads the buffer and calls DOM methods, but in Mojo instead of JS).

**Advantages of Blitz over the webview approach:**

- **No JS runtime** — no need to bundle or inject JavaScript; the mutation interpreter runs in Mojo
- **No IPC overhead** — mutations are applied in-process via direct FFI calls, not base64-encoded over webview eval
- **Smaller binary** — no browser engine dependency (WebKitGTK is ~50+ MB); Blitz is much lighter
- **Cross-platform** — Blitz uses Winit, which supports Linux, macOS, and Windows natively
- **Better integration** — native window chrome, system menus, accessibility via AccessKit
- **Consistent rendering** — Stylo (Firefox's CSS engine) provides standards-compliant CSS everywhere

---

## XR Renderer (in progress — `mojo-gui/xr/`)

Strategy: render DOM-based UI panels into XR environments (VR/AR). Each panel is a separate DOM document that receives the standard mutation protocol, is rendered to a GPU texture, and placed as a quad in 3D space. Two backends:

### OpenXR Native (`mojo-gui/xr/native/`)

Reuses the Blitz stack from the desktop renderer:

- Each `XRPanel` owns a `blitz-dom` document (same as desktop, but rendered to an offscreen texture instead of a Winit window)
- Vello renders each panel's DOM to a `wgpu::Texture` (Vello already supports arbitrary render targets)
- The OpenXR runtime composites these textures as quad layers in 3D space, or the shim renders them into the XR swapchain via a simple 3D compositor
- XR controller raycasting → intersect panel quad → compute 2D hit point → dispatch as DOM pointer events through the existing event protocol
- The `openxr` Rust crate provides session management, pose tracking, input actions, reference spaces

```text
┌─────────────────────────────────────────────────────────────────┐
│  Native Process                                                  │
│                                                                  │
│  ┌─────────────────────┐                                         │
│  │  mojo-gui/core       │                                         │
│  │  (compiled native)   │── mutation buffer ──┐                   │
│  │  signals, vdom,      │                     │                   │
│  │  diff, scheduler     │◄── event dispatch ──┤                   │
│  └─────────────────────┘                     │                   │
│                                              ▼                   │
│  ┌─────────────────────────────────────────────────────────┐     │
│  │  XR Panel Manager                                        │     │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐               │     │
│  │  │ Panel 0   │  │ Panel 1   │  │ Panel N   │  ...         │     │
│  │  │ blitz-dom │  │ blitz-dom │  │ blitz-dom │              │     │
│  │  │ + Stylo   │  │ + Stylo   │  │ + Stylo   │              │     │
│  │  │ + Taffy   │  │ + Taffy   │  │ + Taffy   │              │     │
│  │  │ → Vello   │  │ → Vello   │  │ → Vello   │              │     │
│  │  │ → texture │  │ → texture │  │ → texture │              │     │
│  │  └─────┬────┘  └─────┬────┘  └─────┬────┘               │     │
│  │        │              │              │                     │     │
│  │        ▼              ▼              ▼                     │     │
│  │  ┌──────────────────────────────────────────────────┐    │     │
│  │  │  OpenXR compositor / 3D scene                     │    │     │
│  │  │  (place textures as quads at world positions)     │    │     │
│  │  │  + controller raycasting → 2D hit → DOM events    │    │     │
│  │  └──────────────────────────────────────────────────┘    │     │
│  └─────────────────────────────────────────────────────────┘     │
│                              │                                    │
│                              ▼                                    │
│                     OpenXR Runtime → HMD                          │
└─────────────────────────────────────────────────────────────────┘
```

### WebXR Browser (`mojo-gui/xr/web/`)

Extends the existing web renderer:

- The existing JS mutation interpreter applies mutations to real DOM elements
- A WebXR session manager creates an immersive session and manages reference spaces
- DOM panel content is rendered to WebGL/WebGPU textures (via OffscreenCanvas or html-to-texture techniques) and placed as quads in the WebXR scene
- XR input sources (controllers, hands) are raycasted against panel quads; hits are translated to standard DOM pointer events that flow back through the existing event bridge
- Falls back gracefully to flat web rendering when no XR device is available

```text
┌─ Browser ──────────────────────────────────────────────────────┐
│                                                                 │
│  ┌─────────────────────┐                                        │
│  │  mojo-gui/core       │                                        │
│  │  (WASM)              │── mutation buffer ──┐                  │
│  │                      │                     │                  │
│  │                      │◄── event dispatch ──┤                  │
│  └─────────────────────┘                     │                  │
│                                              ▼                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  XR Panel Manager (JS)                                    │   │
│  │  ┌─────────────┐  ┌─────────────┐                         │   │
│  │  │ Panel 0      │  │ Panel 1      │  ...                   │   │
│  │  │ DOM subtree  │  │ DOM subtree  │                        │   │
│  │  │ → texture    │  │ → texture    │                        │   │
│  │  └──────┬──────┘  └──────┬──────┘                         │   │
│  │         │                │                                 │   │
│  │         ▼                ▼                                 │   │
│  │  ┌──────────────────────────────────────────────────┐     │   │
│  │  │  WebXR session (WebGL/WebGPU)                     │     │   │
│  │  │  (place textures as quads in XR reference space)  │     │   │
│  │  │  + XRInputSource raycasting → DOM pointer events  │     │   │
│  │  └──────────────────────────────────────────────────┘     │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│                              ▼                                   │
│                     WebXR Runtime → HMD                           │
└─────────────────────────────────────────────────────────────────┘
```

### Key XR Design Decisions

- **The mutation protocol is unchanged.** Each XR panel receives the same binary opcode stream as any other renderer. The core framework doesn't know it's running in XR.
- **Blitz stack is reused, not forked.** The OpenXR native renderer uses the same `blitz-dom` + Stylo + Taffy + Vello pipeline as the desktop renderer. The only difference is the final render target (offscreen texture vs. Winit surface) and the compositor (OpenXR quad layers vs. window manager).
- **Panels are the spatial primitive.** A panel is a 2D DOM document placed at a 3D position/rotation in the XR scene. Apps create panels via a new `XRPanel` API; each panel can host a separate `GuiApp` or a view within one.
- **Input is bridged, not reinvented.** XR controller rays are intersected with panel quads in 3D; the resulting 2D hit coordinates are translated to standard DOM pointer/click events and dispatched through the existing `HandlerRegistry`. App code doesn't know the click came from a VR controller.
- **wgpu is the unifying GPU layer.** It targets Vulkan/Metal/DX12 natively (for OpenXR) and WebGPU in the browser (for WebXR), providing a single rendering abstraction across both XR backends.

### What Each Renderer Provides

| Renderer             | Entry mechanism                      | Event loop driver                      |
|----------------------|--------------------------------------|----------------------------------------|
| **Web**              | JS runtime instantiates WASM, calls `@export` init | JS `requestAnimationFrame` + event listeners |
| **Desktop (webview)**| Native `main()` creates GTK4 window with webview | GTK main loop via `mwv_step()` polling  |
| **Desktop (Blitz)**  | Native `main()` creates Blitz window (future) | Winit event loop via Blitz C shim       |
| **XR (WebXR)**       | JS runtime creates WebXR session, renders DOM panels to textures in 3D | WebXR `requestAnimationFrame` + XR input sources |
| **XR (OpenXR)**      | Native `main()` creates OpenXR session, Blitz panels render to swapchain textures | OpenXR frame loop + Winit/Vello for panel rendering |