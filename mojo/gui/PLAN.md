# mojo-gui — Project Plan

Multi-renderer reactive GUI framework for Mojo. Write a GUI app **once**, run it in the browser via WASM **and** natively on desktop — like Dioxus does for Rust.

---

## Status Dashboard

| Target | Renderer | Status | Platform |
|--------|----------|--------|----------|
| Web (WASM) | TypeScript DOM interpreter | ✅ Complete | All browsers |
| Desktop | Blitz (Stylo + Vello + Winit) | ✅ Complete | Linux Wayland |
| Desktop | Blitz | 🔲 Untested | macOS |
| Desktop | Blitz (Wine) | ✅ Verified | Windows (via Wine) |
| XR Native | OpenXR + Blitz offscreen | 🔧 In progress (Steps 5.1–5.5, 5.7–5.8, Vello offscreen ✅, OpenXR backend ✅) | Linux (headless + GPU tests pass) |
| XR Browser | WebXR + JS interpreter | 🔧 In progress (Step 5.6, JS+rasterize+E2E tests ✅) | WebXR browsers |
| CI | `nix flake check` | ✅ Complete | Tangled CI (push/PR on main) |

| Area | Metric |
|------|--------|
| Core Mojo test suites | 52 |
| JS integration test suites | 30 (~3,375 tests) |
| XR web runtime JS tests | 5 suites (523 tests — types, panel, input, runtime, rasterize) |
| Desktop integration test suites | 1 (75 tests, verified on Linux + Wine) |
| XR shim integration tests | 48 (37 headless + 11 GPU rendering — Vello offscreen pipeline) |
| XR example verification | 4/4 (Counter, Todo, Benchmark, MultiView — headless build+run) |
| Shared example apps | 4 (Counter, Todo, Benchmark, MultiView) |
| Test/demo app modules | 15 (in `examples/apps/`) |
| Binary mutation opcodes | 18 |
| Desktop Blitz C FFI functions | ~45 |
| XR C FFI functions | ~88 (includes 3 `_into()` output-pointer variants + 3 GPU functions + 2 input state queries) |
| XR Mojo FFI wrapper methods | ~70 (XRBlitz struct) |
| XR compile targets | 3 (web, desktop, XR via `-D MOJO_TARGET_XR`) |
| CI check derivations | 6 (test-desktop, test-xr, test, test-js, test-xr-js, build-all) |

---

## Project Structure

```text
mojo-gui/
├── core/           — Renderer-agnostic reactive GUI framework (Mojo library)
│   ├── src/        — signals/, scope/, scheduler/, arena/, vdom/, mutations/,
│   │                 bridge/, events/, component/, html/, platform/
│   └── test/       — 52 Mojo test suites (run via wasmtime)
├── web/            — Browser renderer (WASM + TypeScript)
│   ├── src/        — @export WASM wrappers, gui_app_exports, web_launcher
│   ├── runtime/    — TypeScript: DOM interpreter, events, templates, protocol
│   ├── examples/   — HTML + JS shells for browser examples
│   ├── test-js/    — 29 JS integration test suites
│   └── scripts/    — Build pipeline (nu scripts)
├── desktop/        — Desktop renderer (Blitz native HTML/CSS engine via Rust cdylib)
│   ├── shim/       — Rust cdylib: BlitzContext, DOM ops, Winit event loop, Vello GPU
│   └── src/        — Mojo FFI bindings, MutationInterpreter, desktop_launch
├── examples/       — Shared example apps (run on every renderer target unchanged)
│   ├── counter/    — Reactive counter with conditional detail
│   ├── todo/       — Full todo app with input binding and keyed list
│   ├── bench/      — JS Framework Benchmark implementation
│   ├── app/        — Multi-view app with client-side routing
│   └── apps/       — 15 test/demo app modules (batch, effects, memos, errors, etc.)
├── xr/             — XR renderer (OpenXR native + WebXR browser, Phase 5)
│   ├── native/     — OpenXR native: Blitz DOM → Vello → offscreen textures → OpenXR
│   │   ├── shim/   — Rust cdylib: multi-panel Blitz + headless DOM + raycasting
│   │   └── src/    — Mojo: XRPanel, XRScene, XRBlitz FFI, XRMutationInterpreter, xr_launch
│   └── web/        — WebXR browser renderer (Step 5.6)
│       ├── runtime/ — TS: XRSessionManager, XRPanelManager, XRQuadRenderer, XRInputHandler, XRRuntime
│       ├── examples/ — XR web entry points (HTML + JS) for shared examples (flat fallback + WebXR)
│       ├── scripts/ — Build pipeline (esbuild TS→JS bundler for browser consumption)
│       └── test-js/ — 5 JS test suites (types, panel, input, runtime, rasterize)
├── docs/plan/      — Detailed plan documents
├── build/          — Build output (gitignored)
├── justfile        — Root task runner (web + desktop + xr commands)
├── default.nix     — Nix dev shell (web + desktop + Wayland deps)
├── CHANGELOG.md    — Full development history (Phases 0–41 + separation)
└── README.md       — Project overview and quick start
```

---

## Plan Documents

### Architecture & Design

| Document | Description |
|----------|-------------|
| [Architecture](docs/plan/architecture.md) | Design principles, module map, project structure, platform abstraction layer, dependency graph |
| [Renderers](docs/plan/renderers.md) | Renderer strategies: Web, Desktop Blitz, XR (OpenXR + WebXR) |
| [CSS Support Scope](docs/plan/css-support.md) | Blitz v0.2.0 CSS feature audit — supported, partial, and unsupported features with app authoring recommendations |
| [XR README](xr/README.md) | XR renderer architecture, key types, build instructions, design decisions, per-step roadmap |

### Phase Documents

| Phase | Document | Status |
|-------|----------|--------|
| Phase 5 | [XR Renderer](docs/plan/phase5-xr.md) | 🔧 In progress (Steps 5.1–5.6, 5.7–5.8 ✅) |
| Phase 6 | [`mojo-web` Raw Web API Bindings](docs/plan/phase6-mojo-web.md) | 📋 Future |

### Cross-Cutting

| Document | Description |
|----------|-------------|
| [Risks, Effort & Open Questions](docs/plan/risks.md) | Risk mitigations, estimated effort, and open design questions |

---

## Testing & Build Infrastructure

### Mojo Tests (52 suites)

Run via wasmtime on WASM-compiled test binaries. Covers: signals, memos, effects, batching, scopes, scheduling, VNode diffing, mutation protocol, templates, DSL, components, conditional rendering, error boundaries, suspense, routing, and all test/demo apps.

```text
just test                    # Build + run all Mojo test suites
just test signals            # Build + run only test_signals
just test signals mutations  # Build + run matching suites
```

### JS Integration Tests (30 suites, ~3,375 tests)

Full end-to-end tests that load the WASM binary, instantiate apps, simulate events, and verify DOM mutations via the TypeScript runtime. Covers every shared example, test/demo app, and mutation protocol conformance.

```text
just test-js                 # Run all JS integration tests (Deno)
```

### Browser End-to-End Tests

Headless browser tests via Servo + WebDriver. Verifies that examples render correctly in a real browser.

```text
just test-browser            # Run all browser tests (headless Servo)
just test-browser-app counter  # Single app
```

### XR Shim Integration Tests (48 tests)

Rust integration tests for the XR Blitz shim. Each panel owns a real Blitz `BaseDocument` with Stylo CSS styling and Taffy layout. Headless DOM tests run without GPU. GPU rendering tests (Vello offscreen pipeline) run when a compatible adapter is available — they verify texture creation, pixel readback, multi-panel rendering, and cleanup. Covers: session lifecycle, panel lifecycle, DOM operations (create/append/insert/replace/remove), attributes, text nodes, placeholders, serialization, events, raycasting, focus, frame loop, reference spaces, ID mapping, stack operations, multi-panel isolation, Blitz document structure, nested elements with attributes, layout resolution, GPU initialisation, offscreen texture rendering, and pixel readback.

```text
just test-xr                 # Run all XR shim integration tests (headless + GPU)
```

### XR Web Runtime JS Tests (4 suites, 414 tests)

Deno-based JS tests for the WebXR browser runtime (`xr/web/runtime/`). Uses `linkedom` for headless DOM simulation. Covers: panel config presets, panel creation/transform/raycasting/model-matrix, panel manager lifecycle/focus/layout, XR input hover tracking/click sequences/focus transitions/drag detection/source removal/multi-source independence, runtime state machine/event listeners/flat fallback/Enter VR button/panel creation/destroy.

```text
just test-xr-js              # Run all XR web runtime JS tests (Deno)
```

### Build Commands

```text
# ── Web ──────────────────────────────────────────────────────
just build                   # Build WASM binary (web/justfile)
just serve                   # Serve examples at localhost:4507
just build-web counter       # Build single example for web

# ── Desktop ──────────────────────────────────────────────────
just build-shim              # Build Blitz cdylib (first time / shim changes)
just build-desktop counter   # Build single example for desktop
just build-desktop-all       # Build all 4 examples for desktop
just run-desktop counter     # Build + run a desktop example (Wayland)
just test-desktop            # Run Blitz shim integration tests (headless)

# ── XR ───────────────────────────────────────────────────────
just build-xr-shim           # Build XR Blitz cdylib (first time / shim changes)
just build-xr counter        # Build single example for XR
just build-xr-all            # Build all 4 examples for XR
just run-xr counter          # Build + run an XR example (headless)
just test-xr                 # Run XR shim integration tests (headless)
just test-xr-js              # Run XR web runtime JS tests (Deno)

# ── Cross-target ─────────────────────────────────────────────
just build-all               # Build web + desktop + windows + XR examples
just test-all                # Run Mojo + JS test suites
just test-all-targets        # Run Mojo + JS + desktop + XR test suites
just clean                   # Remove all build artifacts

# ── CI (Nix check derivations) ───────────────────────────────
nix build .#checks.x86_64-linux.mojo-gui-test-desktop  # 75 Rust tests (Blitz shim)
nix build .#checks.x86_64-linux.mojo-gui-test-xr       # 37 Rust tests (XR shim)
nix build .#checks.x86_64-linux.mojo-gui-test           # 52 Mojo test suites
nix build .#checks.x86_64-linux.mojo-gui-test-js        # 3,375 JS tests (Deno)
nix build .#checks.x86_64-linux.mojo-gui-build-all      # 4 examples × {web, desktop, xr}
nix flake check                                          # Run ALL checks (used by CI)
```

### Import Conventions

Apps and core modules use `-I` flag paths. The build target determines which renderer is linked.

**Note:** All native builds (desktop and XR) must include both `-I desktop/src` and `-I xr/native/src` because Mojo's `@parameter if` does not prevent import resolution in dead branches. The compile-time dispatch in `launch.mojo` imports from both `desktop.launcher` and `xr.launcher` — the linker only pulls in the active branch's code.

```text
# Web (WASM):
mojo build examples/counter/main.mojo --target wasm64-wasi -I core/src -I web/src -I examples

# Desktop (native):
mojo build examples/counter/main.mojo -I core/src -I desktop/src -I xr/native/src -I examples

# XR (native + OpenXR):
mojo build examples/counter/main.mojo -D MOJO_TARGET_XR -I core/src -I xr/native/src -I desktop/src -I examples
```

---

## What Was Done

### Phase 5.2d: OpenXR Backend Integration — ✅ Complete

**Goal:** Wire up real OpenXR session lifecycle, Vulkan graphics binding, frame loop, controller input, and per-panel swapchain management in the XR shim — completing the native XR rendering pipeline.

**What was built:**

| File | Description |
|------|-------------|
| `xr/native/shim/src/openxr_backend.rs` | New module (~1,190 lines): `OpenXrBackend` struct encapsulating all OpenXR + Vulkan state. Full initialization sequence: load OpenXR entry via `Entry::load()` (dlopen), create instance with `XR_KHR_vulkan_enable2`, get HMD system, create Vulkan instance/device via `ash` per OpenXR requirements, create OpenXR session with Vulkan graphics binding, create reference spaces (stage/view with fallback to local), set up input actions (grip/aim poses, select/squeeze booleans) with Khronos Simple Controller + Oculus Touch interaction profiles, wrap shared Vulkan device in wgpu via HAL APIs for Vello rendering. |
| `xr/native/shim/src/lib.rs` (modified) | Integrated `OpenXrBackend` into `XrSessionContext`. `mxr_create_session` now attempts real OpenXR initialization with graceful fallback to headless. All frame loop functions (`mxr_wait_frame`, `mxr_begin_frame`, `mxr_render_dirty_panels`, `mxr_end_frame`) dispatch to the OpenXR backend when active. Input functions (`mxr_get_pose`, `mxr_get_aim_ray`) return real tracked poses. Capability queries (`mxr_has_extension`, `mxr_has_hand_tracking`, `mxr_has_passthrough`) delegate to the backend. Two new FFI functions: `mxr_get_select_state`, `mxr_get_squeeze_state`. |
| `xr/native/shim/Cargo.toml` (modified) | Changed `openxr` from `features = ["linked"]` to `features = ["loaded"]` for runtime detection (cdylib loads without OpenXR loader). Added `ash = "0.38"` as direct dependency for Vulkan interop. |

**Key design decisions:**

1. **Runtime detection via `Entry::load()`** — The "loaded" feature uses dlopen at runtime. If the OpenXR loader isn't available, `mxr_create_session` falls back to headless mode transparently. The cdylib can always be loaded by Mojo even on systems without an XR runtime.
2. **Shared Vulkan device** — A single Vulkan instance/device is shared between OpenXR compositing and Vello rendering via wgpu HAL APIs (`Instance::from_raw`, `expose_adapter`, `device_from_raw`, `create_texture_from_hal`). No GPU→CPU→GPU copies needed.
3. **Per-panel swapchains** — Each panel gets its own OpenXR swapchain (`XrCompositionLayerQuad`). Swapchain images are wrapped in wgpu textures for direct Vello rendering. Format negotiation prefers `R8G8B8A8_SRGB` with `R8G8B8A8_UNORM` fallback.
4. **Quad layer compositing** — `mxr_end_frame` submits one `CompositionLayerQuad` per visible panel with pose derived from panel transform (position + quaternion rotation + size in meters).
5. **Input actions** — Action set with grip/aim pose actions and select/squeeze boolean actions. Bindings for Khronos Simple Controller (universal) and Oculus Touch (Meta Quest). Aim ray extracted from aim space pose with quaternion-rotated forward vector.
6. **Graceful degradation** — All functions handle the no-backend case (headless). Session state is `FOCUSED` when falling back (matching headless behavior). Existing 48 tests pass unchanged.

**New FFI functions (2):**

| Function | Description |
|----------|-------------|
| `mxr_get_select_state(session) → i32` | Bitfield: bit 0 = left trigger active, bit 1 = right trigger active. |
| `mxr_get_squeeze_state(session) → i32` | Bitfield: bit 0 = left grip active, bit 1 = right grip active. |

**OpenXR subsystems implemented:**

| Subsystem | Description |
|-----------|-------------|
| Session lifecycle | `xrCreateSession` with Vulkan binding, `xrBeginSession`/`xrEndSession` on state transitions, event polling for state machine |
| Frame loop | `xrWaitFrame` → predicted display time, `xrBeginFrame`, `xrEndFrame` with composition layers |
| Reference spaces | Stage space (fallback to Local), View space — created at session init |
| Input | Action set with 6 actions (left/right grip pose, left/right aim pose, select, squeeze), space creation for pose tracking, `xrSyncActions` per frame |
| Swapchain management | Per-panel swapchain creation/destruction, image acquisition/wait/release, wgpu texture wrapping via HAL |
| Panel rendering | Vello scene painting to acquired swapchain image, layout resolution before paint |
| Capabilities | Extension queries, hand tracking detection, passthrough detection |

**Interaction profiles:**

| Profile | Controller | Bindings |
|---------|-----------|----------|
| `/interaction_profiles/khr/simple_controller` | Universal | grip/aim poses, select/click, squeeze/click |
| `/interaction_profiles/oculus/touch_controller` | Meta Quest | grip/aim poses, trigger/value, squeeze/click |

### Phase 5.2c: Vello Offscreen Rendering — ✅ Complete

Added GPU offscreen rendering to the XR shim via Vello + wgpu. Each panel's Blitz DOM can now be rendered to a GPU texture — the same `paint_scene()` pipeline used by the desktop renderer, but targeting an offscreen `wgpu::Texture` instead of a window surface.

**New capability:** `mxr_render_dirty_panels()` now does two things:

1. Resolves styles and layout (Stylo + Taffy) — same as before
2. Paints each dirty panel's DOM to its offscreen GPU texture via Vello — **new**

When GPU is not available (CI, headless-only), the function falls back to layout-only resolution (no pixel output), preserving backward compatibility.

**New structs:**

| Struct | Description |
|--------|-------------|
| `OffscreenRenderer` | Owns `wgpu::Device`, `wgpu::Queue`, and `vello::Renderer`. Created lazily via `mxr_init_gpu()`. Handles texture creation, Vello scene painting, `render_to_texture()`, and CPU pixel readback. |
| `PanelTexture` | Per-panel `wgpu::Texture` + `TextureView` at the panel's pixel dimensions. Created on first render, reused for subsequent frames. |

**New FFI functions (3):**

| Function | Description |
|----------|-------------|
| `mxr_init_gpu(session) → i32` | Try to initialise the GPU renderer (wgpu adapter + device + Vello). Returns 1 on success, 0 on failure. Idempotent. Intentionally separate from session creation so headless tests keep working without a GPU. |
| `mxr_has_gpu(session) → i32` | Returns 1 if GPU renderer is available, 0 otherwise. |
| `mxr_panel_read_pixels(session, panel_id, buf, buf_len) → u32` | Copy a panel's most-recently-rendered texture to a CPU buffer (RGBA8, row-major). Returns bytes written, or 0 on failure. Uses wgpu buffer mapping with row-stride padding removal. |

**Rendering pipeline:**

```text
mxr_init_gpu(session)                    # One-time GPU init
  └── wgpu::Instance → Adapter → Device + Queue
  └── vello::Renderer::new(device, RendererOptions)

mxr_render_dirty_panels(session)         # Per-frame
  ├── For each dirty panel:
  │   ├── panel.doc.resolve(0.0)         # Stylo + Taffy layout
  │   ├── ensure_texture(panel)          # Create/resize wgpu::Texture
  │   ├── VelloScenePainter + paint_scene()  # Blitz DOM → Vello Scene
  │   └── vello_renderer.render_to_texture() # Scene → GPU texture
  └── Clear dirty flags

mxr_panel_read_pixels(session, id, buf)  # Debug/test readback
  └── copy_texture_to_buffer → map_async → strip row padding → copy to buf
```

**New integration tests (11, 48 total):**

| Test | Description |
|------|-------------|
| `init_gpu_on_named_session` | GPU init on non-headless session doesn't crash, returns consistent result |
| `init_gpu_on_headless_session` | GPU init on headless session works if adapter available |
| `init_gpu_idempotent` | Second `mxr_init_gpu()` call returns 1 if first succeeded |
| `has_gpu_null_session` | Null session returns 0 for both `has_gpu` and `init_gpu` |
| `render_dirty_panel_with_gpu` | Panel with DOM content renders to texture with correct dimensions |
| `read_pixels_without_gpu_returns_zero` | No GPU → `read_pixels` returns 0 |
| `read_pixels_null_args` | Null session/buffer/unknown panel all return 0 |
| `read_pixels_with_gpu` | Renders white-background panel, reads pixels, verifies non-zero content |
| `read_pixels_buffer_too_small` | Undersized buffer returns 0 |
| `render_multiple_panels_with_gpu` | Two panels render to separate textures with correct dimensions; clean panels skip re-render |
| `texture_destroyed_on_panel_destroy` | Panel destruction cleans up GPU texture |

**Modified files:**

- **`xr/native/shim/Cargo.toml`** — Added `vello = "0.6"`, `pollster = "0.4"` as direct dependencies. Updated `wgpu` from v24 to v26 (matching vello 0.6's wgpu version).
- **`xr/native/shim/src/lib.rs`** — Added `OffscreenRenderer`, `PanelTexture` structs. Added `gpu_texture` field to `Panel`, `renderer` field to `XrSessionContext`. Updated `mxr_render_dirty_panels()` to paint via Vello when GPU available. Added 3 new FFI functions (`mxr_init_gpu`, `mxr_has_gpu`, `mxr_panel_read_pixels`) and 11 integration tests.
- **`xr/native/shim/mojo_xr.h`** — Added GPU initialisation section with declarations for `mxr_init_gpu`, `mxr_has_gpu`, `mxr_panel_read_pixels`.

**Key design decisions:**

1. **Lazy GPU init** — `mxr_init_gpu()` is separate from session creation. Headless tests don't need a GPU; the rendering pipeline gracefully degrades to layout-only mode.
2. **Same paint pipeline as desktop** — Uses `blitz_paint::paint_scene()` with `anyrender_vello::VelloScenePainter`, ensuring pixel-identical rendering between desktop windows and XR panel textures.
3. **Per-panel textures** — Each panel gets its own `wgpu::Texture` at its pixel dimensions (`texture_width × texture_height`). Textures are created on first render and reused for subsequent frames.
4. **Renderer borrow split** — The renderer is temporarily `take()`n out of `XrSessionContext` during `mxr_render_dirty_panels()` to avoid borrow conflicts (renderer needs `&mut` while iterating panels also needs `&mut`).

**Remaining Step 5.2 work:**

- OpenXR session lifecycle (`xrCreateSession`, `xrWaitFrame`, `xrBeginFrame`, `xrEndFrame`)
- Quad layer compositing (submit panel GPU textures as OpenXR `CompositionLayerQuad`)
- Controller pose tracking via OpenXR input actions

---

### S-1: Cross-Target CI Pipeline — ✅ Complete

Added 5 Nix check derivations to `mojo-gui/default.nix` that build and run all test suites inside the Nix sandbox. The Tangled CI pipeline (`.tangled/workflows/ci.yml`) already runs `nix flake check` on push/PR to main — these checks are now automatically included.

**Check derivations:**

| Check | Type | Tests | Time |
|-------|------|-------|------|
| `mojo-gui-test-desktop` | `rustPlatform.buildRustPackage` | 75 Rust integration tests (headless Blitz shim) | ~1m 23s |
| `mojo-gui-test-xr` | `rustPlatform.buildRustPackage` | 37 Rust integration tests (headless XR shim) | ~1m |
| `mojo-gui-test` | `stdenv.mkDerivation` | 52 Mojo test suites via wasmtime | ~1m 12s |
| `mojo-gui-test-js` | `stdenv.mkDerivation` | 3,375 JS integration tests via Deno | ~1m |
| `mojo-gui-build-all` | `stdenv.mkDerivation` | 4 examples × {web, desktop, xr} = 12 builds | ~50s |

**Key implementation details:**

- **Rust checks** use `rustPlatform.buildRustPackage` with `doCheck = true`. Shared Blitz build dependencies (pkg-config, cmake, python3, fontconfig, freetype, libxkbcommon, wayland, vulkan-loader, libGL, openssl) are factored out. XR additionally needs `openxr-loader`.

- **Mojo test check** builds the WASM binary inline (mojo → llc → wasm-ld pipeline), precompiles via `wasmtime compile`, then runs test binary build + execution via nu scripts. System libraries for the Mojo native linker (zlib, ncurses) are in `buildInputs`. `LD_LIBRARY_PATH` is set for `libwasmtime.so` dlopen at test runtime.

- **JS test check** uses a **fixed-output derivation** (`denoDeps`) to pre-fetch the `npm:linkedom` dependency tree from npm into a Deno cache. The test check copies this cache to a writable `$TMPDIR/deno-cache` and sets `DENO_DIR`. No network access needed during test execution.

- **Build-all check** runs the WASM build pipeline and `mojo build` for desktop (native) and XR (`-D MOJO_TARGET_XR`) targets directly — bypasses justfile recipes that use `nix eval` (unavailable in sandbox) or `/usr/bin/env nu` shebangs.

- **Sandbox workaround:** The justfile's incremental build recipes use `#!/usr/bin/env nu` shebangs which fail in the Nix sandbox (no `/usr/bin/env`). The Mojo and JS checks run build commands directly instead of via `just test` / `just test-js`.

**Verification:**

```text
$ nix build .#checks.x86_64-linux.mojo-gui-test-desktop  # ✅ 75/75
$ nix build .#checks.x86_64-linux.mojo-gui-test-xr       # ✅ 37/37
$ nix build .#checks.x86_64-linux.mojo-gui-test           # ✅ 52/52 suites
$ nix build .#checks.x86_64-linux.mojo-gui-test-js        # ✅ 3375/3375
$ nix build .#checks.x86_64-linux.mojo-gui-build-all      # ✅ 12/12 builds
```

**Modified files:**

- **`mojo-gui/default.nix`** — Added `checks` attribute with 5 check derivations and a `denoDeps` fixed-output derivation for Deno npm cache. Shared Blitz build dependencies factored into `blitzNativeBuildInputs` / `blitzBuildInputs`.

---

### Phase 5.6: WebXR JS Runtime — 🔧 In Progress

**Goal:** Create `xr/web/runtime/` — the browser-side WebXR renderer that reuses the binary mutation protocol unchanged, rendering panel DOM content as textured quads in an immersive WebXR scene.

**What was built:**

| File | Description |
|------|-------------|
| `xr/web/runtime/xr-types.ts` | TypeScript types mirroring native XR panel types: `Vec3`, `Quaternion`, `PanelConfig` (with presets: default, dashboard, tooltip, hand-anchored), `PanelState`, `XRPanelDescriptor`, `RaycastHit`, `XRInputRay`, `XRRuntimeConfig`, WebXR API compat interfaces (`XRSessionCompat`, `XRFrameCompat`, `XRViewCompat`, etc.) |
| `xr/web/runtime/xr-session.ts` | `XRSessionManager` — full WebXR session lifecycle: feature detection (`navigator.xr`), session request (immersive-vr/ar/inline), WebGL2 context creation with `xrCompatible`, `XRWebGLLayer` binding, reference space negotiation (tries `local-floor` → `bounded-floor` → `local` → `viewer`), XR frame loop delegation, session end + cleanup, runtime event emission |
| `xr/web/runtime/xr-panel.ts` | `XRPanel` — offscreen DOM container per panel, SVG foreignObject DOM→canvas rasterization (async), fallback text rasterizer, WebGL texture upload (`texImage2D`/`texSubImage2D`), ray-plane intersection raycasting, 4×4 model matrix computation from position/rotation/scale. `XRPanelManager` — panel lifecycle, focus management (exclusive), throttled dirty texture updates, raycasting across all panels, spatial layout helpers (`arrangeArc`, `arrangeGrid`, `arrangeStack`) |
| `xr/web/runtime/xr-renderer.ts` | `XRQuadRenderer` — WebGL2 GLSL ES 3.0 shader program (textured quad with alpha + opacity uniform), VAO/VBO/EBO for unit quad geometry, per-view stereo rendering (`setView()` from `XRView.projectionMatrix` + `transform.inverse.matrix`), cursor dot visualization at UV hit point, GL state save/restore |
| `xr/web/runtime/xr-input.ts` | `XRInputHandler` — extracts input rays from `XRInputSource.targetRaySpace` poses, raycasts against panel quads, tracks per-source hover state (mouseenter/mouseleave/mousemove with ~30Hz throttle), synthesizes click sequences from XR select events (selectstart → mousedown, selectend → mouseup + click if same panel within 20px), focus transitions on click, callback-based dispatch (no DOM/WASM coupling) |
| `xr/web/runtime/xr-runtime.ts` | `XRRuntime` — main entry point orchestrating all subsystems. WASM app loading with full env imports (matching `web/runtime/env.ts`). `createAppPanel()` for convention-based WASM export discovery (`{name}_init/rebuild/flush/handle_event/destroy`). Uses shared `Interpreter` + `TemplateCache` from `web/runtime/` for full DOM feature parity (all 18 opcodes including `RegisterTemplate`). Handler map for XR input → WASM dispatch (wired via `onNewListener`/`onRemoveListener` callbacks). "Enter VR" button. Flat-fallback mode (panels visible as normal DOM when no WebXR). Per-frame pipeline: process input → flush WASM → rasterize dirty panels → render quads → draw cursors |
| `xr/web/runtime/mod.ts` | Module re-exports — single import path for the full public API |
| `xr/web/examples/lib/xr-app.js` | Shared XR app launcher — `launchXR()` initializes XRRuntime, creates an app panel from WASM via convention-based export discovery, starts in XR or flat fallback mode, status display helpers, event listener wiring |
| `xr/web/examples/counter/` | XR counter entry point: `index.html` (flat-fallback panel styling, `#xr-status` element) + `main.js` (loads shared WASM via `launchXR()`) |
| `xr/web/examples/todo/` | XR todo entry point: `index.html` + `main.js` |
| `xr/web/examples/bench/` | XR benchmark entry point: `index.html` + `main.js` (8 MiB buffer for large DOM) |
| `xr/web/examples/app/` | XR multi-view app entry point: `index.html` + `main.js` |
| `xr/web/scripts/bundle.ts` | esbuild-based TS→JS bundler for browser consumption. Bundles each XR example entry point + full runtime into a self-contained ES module (`bundle.js`). Supports per-app or all-app builds, `--clean` flag, source maps |
| `xr/web/test-browser.nu` | Browser E2E test script for XR flat-fallback mode via headless Servo + W3C WebDriver. Verifies: panel containers appear with correct `data-xr-panel` attributes, flat-fallback visibility (position/visibility/pointer-events CSS), `#xr-status` reflects runtime state, WASM mutations applied to panel DOM, no "Enter VR" button when WebXR unavailable, structural properties (panel is child of body, has pixel dimensions, overflow hidden). Tests all 4 example apps. Uses different ports (4508/7124) to coexist with web browser tests |
| `xr/web/test-js/harness.ts` | Test harness with assert helpers (equality, close, defined, null, throws, async throws, length, greater-than, boolean) |
| `xr/web/test-js/dom-helper.ts` | Headless DOM environment via `linkedom`, canvas context stubs, WebGL2 stubs for testing panel texture upload |
| `xr/web/test-js/xr-types.test.ts` | Tests for panel config presets, texture dimension derivation, runtime config, event type constants, config spread patterns, aspect ratios |
| `xr/web/test-js/xr-panel.test.ts` | Tests for panel construction, DOM container, transforms (setPosition/setRotation/setRotationEuler), state helpers, model matrix (identity/translation/rotation), raycasting (center/corners/miss/repositioned), rasterize fallback, destroy, panel manager lifecycle/focus/dirty-tracking/raycasting/layout (arc/grid/stack) |
| `xr/web/test-js/xr-input.test.ts` | Tests for hover tracking (enter/leave/move/panel-transition), click sequences (selectstart/selectend/onSelect), focus transitions, source removal/reset, multi-source independence, cursor queries, source filtering (gaze/screen/transient), drag detection, callback error handling, getPose exception handling |
| `xr/web/test-js/xr-runtime.test.ts` | Tests for state machine (Uninitialized→Ready/FlatFallback→Destroyed), initialize with/without XR, double-init error, destroy idempotency, panel creation (standalone, with config/position, in various states), event listeners (subscribe/unsubscribe/multiple/error-resilient), configuration overrides, Enter VR button lifecycle, flat fallback panel visibility (start/stop), input handler wiring, state getter consistency, event type mapping |
| `xr/web/test-js/xr-rasterize.test.ts` | SVG foreignObject fidelity validation tests: SVG markup structure (innerHTML wrapping, style injection, container/texture dimensions), fallback rasterizer (empty/plain/nested/long/special-char/unicode/styled/form/table content), dirty tracking cycles, content mutation → re-rasterize flow (counter increment, todo add, rapid mutations, content removal), WebGL texture upload integration (create/reuse/destroy/no-GL-safe), panel manager dirty orchestration (getDirtyPanels, updateDirtyTextures, selective re-raster, destroyAll), fidelity edge cases (inline styles, CSS classes, data attributes, flexbox, nested SVG, empty elements, 100-element DOM, overflow clipping), multi-panel independence, canvas state, panel background customization, async rasterize method existence |
| `xr/web/test-js/run.ts` | Test runner executing all 5 test suites (523 tests total) |
| `xr/web/deno.json` | Deno compiler configuration for the XR web module |

**Key design decisions:**

1. **Mutation protocol unchanged** — each panel receives the same binary opcode stream; the shared `Interpreter` from `web/runtime/` processes all 18 opcodes
2. **DOM→texture via SVG foreignObject** — real CSS rendering fidelity with async rasterization; falls back to simple text renderer when SVG fails
3. **Callback-based input dispatch** — `XRInputHandler` emits synthetic DOM event names but doesn't touch the DOM directly; the runtime wires callbacks to WASM dispatch
4. **Flat fallback** — when WebXR is unavailable, panel containers become visible DOM elements with standard CSS styling
5. **Shared `web/runtime/` Interpreter** — the XR runtime imports the full `Interpreter` and `TemplateCache` from `web/runtime/`, ensuring complete DOM feature parity. Handler map wiring uses `onNewListener`/`onRemoveListener` callbacks added to the shared Interpreter for XR integration
6. **esbuild bundling for browser** — XR runtime is TypeScript; browsers can't load `.ts` modules directly. `scripts/bundle.ts` uses esbuild to produce self-contained ES module bundles (`bundle.js`) for each example app, including the full runtime + shared web interpreter

**Test coverage (523 tests, 5 suites):**

| Suite | Tests | Coverage |
|-------|-------|----------|
| `xr-types.test.ts` | 68 | Panel config presets (all 4), texture dimensions, runtime config, event constants, spread patterns, aspect ratios |
| `xr-panel.test.ts` | 174 | Panel construction, DOM container, transforms, Euler→quaternion, state helpers, model matrix math, ray-plane intersection raycasting (hit center/corners/miss/repositioned/boundary cases), rasterize fallback, destroy, panel manager CRUD, focus management, dirty tracking, multi-panel raycasting, spatial layout (arc/grid/stack) |
| `xr-input.test.ts` | 78 | Hover state machine (enter/leave/move/panel-transition), click synthesis (selectstart→mousedown, selectend→mouseup+click, drag distance threshold), focus transitions (blur/focus events), source removal/reset cleanup, multi-source independence, cursor queries, source filtering (tracked-pointer/gaze/screen), callback error resilience, getPose exception handling |
| `xr-runtime.test.ts` | 94 | State machine transitions, mock navigator.xr for XR-available/unavailable paths, double-initialize error, destroy idempotency, panel creation in all states, event listener lifecycle, config override/defaults, Enter VR button creation/cleanup, flat fallback panel visibility toggle, input handler wiring, state getter consistency |
| `xr-rasterize.test.ts` | 109 | SVG markup structure, fallback rasterizer (9 content types), dirty tracking cycles, mutation→rasterize flow (counter/todo/rapid/removal), texture upload integration (create/reuse/destroy), panel manager dirty orchestration (getDirtyPanels/updateDirtyTextures/selective/destroyAll), fidelity edge cases (inline styles/classes/data-attrs/flexbox/SVG/empty-elements/100-node DOM/overflow), multi-panel independence, canvas state, panel background, async rasterize |

**Remaining work:**

- End-to-end testing with a real WebXR device or browser emulator (browser E2E test script created but not yet run against Servo — requires `just build-xr-web` + Servo with ES module support)
- Real-device SVG foreignObject fidelity validation (unit tests cover markup structure and fallback; pixel-level rendering needs a real browser)

**Recently completed:**

- ✅ **SVG foreignObject fidelity test suite** (`xr-rasterize.test.ts`, 109 tests) — validates the DOM→texture rasterization pipeline: SVG markup structure, fallback rasterizer with 9 content types (empty, plain text, deeply nested, long text word-wrap, special characters, unicode, styled, form elements, tables), dirty tracking through rasterization cycles, content mutation → re-rasterize flow, WebGL texture upload integration, panel manager dirty texture orchestration, fidelity edge cases (inline styles, CSS classes, data attributes, flexbox layout, nested SVG, empty elements, 100-element DOM, overflow clipping), multi-panel independent content/rasterization, canvas state verification, panel background customization. All 523 XR web tests pass.
- ✅ **XR web example entry points** — HTML + JS for all 4 shared examples (counter, todo, bench, app) at `xr/web/examples/`. Shared `xr-app.js` launcher: `launchXR()` initializes XRRuntime, creates app panel from WASM, starts XR or flat fallback, status display, event wiring. HTML pages styled for flat-fallback panels (`[data-xr-panel]` selectors). Loads bundled JS (`bundle.js`) for browser compatibility.
- ✅ **esbuild TS→JS bundler** (`xr/web/scripts/bundle.ts`) — bundles each XR example entry point + full runtime (XR session/panel/renderer/input/runtime + shared web Interpreter/TemplateCache/protocol) into a single browser-ready ES module. Supports per-app build, all-app build, `--clean` flag, source maps. All 4 apps bundle successfully (~3,700 lines each).
- ✅ **Browser E2E test script** (`xr/web/test-browser.nu`) — headless Servo + WebDriver test suite for XR flat-fallback mode. Verifies panel containers, flat-fallback CSS (position/visibility/pointer-events), `#xr-status` element, WASM mutation application, absence of "Enter VR" button, structural properties. Tests all 4 example apps. Uses ports 4508/7124 to coexist with web tests.
- ✅ **Justfile recipes** — `build-xr-web` (bundle all), `build-xr-web-app` (single app), `clean-xr-web`, `serve-xr`, `test-browser-xr`, `test-browser-xr-verbose`, `test-browser-xr-app`, `test-all-browser`. Updated `build-all` and `clean`.
- ✅ **Integration with `web/runtime/Interpreter`** — replaced the ~420-line inline mutation applier with imports from the shared `Interpreter` class and `TemplateCache` from `web/runtime/`. Added `onRemoveListener` callback to the shared Interpreter for handler map cleanup. All 523 XR web tests + 3,375 web JS tests pass unchanged.
- ✅ **Nix check derivation `mojo-gui-test-xr-js`** — added to `default.nix` as the 6th CI check. Uses `monoSrc` (since XR runtime imports from `web/runtime/`), pre-fetched Deno dependency cache (`denoXrDeps` FOD), runs 523 tests in the Nix sandbox without network access.

### Phase 5.8: Verify Shared Examples in XR — ✅ Complete

Verified all 4 shared examples (Counter, Todo, Benchmark, MultiView) build and run as XR floating panels in headless mode. Fixed three issues discovered during verification.

**Issue 1: Mojo `@parameter if` import resolution in dead branches**

The `launch()` function in `core/src/platform/launch.mojo` uses `@parameter if` / `elif` / `else` for compile-time dispatch, importing `desktop.launcher` and `xr.launcher` in different branches. However, Mojo's `@parameter if` does not suppress import resolution in dead branches — the compiler still resolves all imports regardless of which branch is active. This caused build failures when only one renderer's include path was provided.

**Fix:** All native builds now include both `-I desktop/src` and `-I xr/native/src`. The linker only pulls in the active branch's code. Updated `justfile` recipes: `build-desktop`, `build-xr`, `build-desktop-windows`.

**Issue 2: `performance_now()` link failure on native targets**

The benchmark app (`examples/bench/bench.mojo`) used `external_call["performance_now", Float64]()` unconditionally, which emits an unresolved symbol. On WASM this becomes an import; on native targets it causes a linker error.

**Fix:** Made `performance_now()` cross-platform with `@parameter if is_wasm_target()`: WASM path uses `external_call` (unchanged), native path uses `time.perf_counter_ns() / 1_000_000.0`.

## Issue 3: Headless XR frame loop never exits

The XR launcher's break condition checked `predicted_time == 0` as a "headless sentinel", but the headless `mxr_wait_frame()` returns real `SystemTime` timestamps (not 0). The frame loop ran indefinitely.

**Fix:** Replaced the `predicted_time == 0` check with an idle frame counter. After mount + flush, if no events arrive and no dirty scopes exist for 1 consecutive frame, the loop exits. Real OpenXR sessions block in `wait_frame()` and never hit this counter.

**Verification results:**

```text
$ just build-xr-all    # ✅ All 4 examples build
$ just run-xr counter  # ✅ Exit code 0
$ just run-xr todo     # ✅ Exit code 0
$ just run-xr bench    # ✅ Exit code 0
$ just run-xr app      # ✅ Exit code 0
$ just test-xr         # ✅ 37/37 shim tests pass
$ just test-desktop    # ✅ 75/75 shim tests pass (no regressions)
```

**Modified files:**

- **`justfile`** — Added `-I xr/native/src` to desktop build commands and `-I desktop/src` to XR build commands. Added comment explaining the Mojo `@parameter if` workaround.
- **`examples/bench/bench.mojo`** — Made `performance_now()` cross-platform via `@parameter if is_wasm_target()`. Added `from platform.app import is_wasm_target`.
- **`xr/native/src/xr/launcher.mojo`** — Replaced `predicted_time == 0` headless exit condition with idle frame counter. Tracks consecutive frames with no events and no dirty scopes; breaks after 1 idle frame.

---

### Mojo 26.1.0 WASM Test Infrastructure Fix — ✅ Complete

Mojo 26.1.0 introduced `clock_gettime` as a new WASM import (`env::clock_gettime`) used by the runtime internally. Both the Mojo wasmtime test harness and the JS/Deno runtime were missing this import, causing all 52 Mojo test suites and all 30 JS test suites to fail with `unknown import: env::clock_gettime`.

**Fix:** Added `clock_gettime` implementation to both runtimes.

- **`core/test/wasm_harness.mojo`** — Added `_cb_clock_gettime` callback (deterministic mock: `tv_sec = mock_time`, `tv_nsec = 0`) and `define_func` registration. Updated import count comment from 15 to 16.
- **`web/runtime/env.ts`** — Added `clock_gettime` to the env object. Uses `performance.now()` to fill `struct timespec { tv_sec, tv_nsec }` at the pointer in WASM memory.

**Signature:** `clock_gettime(clockid: i32, timespec_ptr: i64) -> i32` where `struct timespec { i64 tv_sec; i64 tv_nsec; }` (WASM64 layout).

**Verification:** All 52 Mojo test suites pass. All 3,375 JS integration tests pass.

---

### Phase 5.5+5.7: XR Launcher & Compile-Time Dispatch — ✅ Complete

Implemented `xr_launch[AppType: GuiApp]()` — the XR-side counterpart to `desktop_launch`. Wraps any GuiApp in a single XR panel and enters the XR frame loop. Also wired `launch()` to dispatch to XR targets at compile time via `-D MOJO_TARGET_XR`.

**New files:**

- **`xr/native/src/xr/launcher.mojo`** — `xr_launch[AppType: GuiApp](config)`. Creates an XR session (headless or OpenXR), allocates a default panel sized from AppConfig, applies UA stylesheet, mounts the app, and enters the XR frame loop: `wait_frame → begin_frame → poll_event → handle_event → flush → apply mutations → render_dirty_panels → end_frame`. Follows the exact same architecture as `desktop_launch` (same mutation buffer management, same GuiApp lifecycle). XR-specific UA stylesheet with larger fonts and dark background for headset legibility.

**Modified files:**

- **`core/src/platform/app.mojo`** — Added `is_xr_target()` compile-time target detection (checks `MOJO_TARGET_XR` define).
- **`core/src/platform/launch.mojo`** — Added `elif is_xr_target()` branch to `launch()` that imports and calls `xr_launch[AppType](config)`. Compile-time dispatch: WASM → web, XR → xr_launch, native → desktop_launch.
- **`core/src/platform/__init__.mojo`** — Re-exports `is_xr_target`.
- **`xr/native/src/xr/__init__.mojo`** — Re-exports `xr_launch`.
- **`justfile`** — Added `build-xr-shim`, `build-xr`, `build-xr-all`, `run-xr` recipes. Updated `build-all` to include XR.

**Compile targets after this step:**

```text
mojo build --target wasm64-wasi -I core/src -I web/src                        → web
mojo build -I core/src -I desktop/src -I xr/native/src                        → desktop (Blitz)
mojo build -D MOJO_TARGET_XR -I core/src -I xr/native/src -I desktop/src      → XR (OpenXR native)
```

---

### Phase 5.2b: Output-Pointer FFI Variants — ✅ Complete

Added `_into()` output-pointer variants for three XR shim functions that return C structs too large for reliable DLHandle struct-return (>16 bytes on x86_64 SysV ABI). This resolves all known FFI limitations from Phase 5.3.

**New Rust FFI functions** (in `xr/native/shim/src/lib.rs`):

- **`mxr_poll_event_into()`** — Polls the next event, writing panel_id, handler_id, event_type, value_ptr, value_len, hit_u, hit_v, hand to caller-provided output pointers. Returns 1 if event available, 0 if queue empty.
- **`mxr_raycast_panels_into()`** — Raycasts against all visible panels, writing panel_id, u, v, distance to output pointers. Returns 1 if hit, 0 if miss.
- **`mxr_get_pose_into()`** — Gets controller/head pose, writing px/py/pz + qx/qy/qz/qw to output pointers. Returns 1 if valid, 0 if not tracked.

**7 new Rust integration tests** (37 total, up from 30):

- `poll_event_into_empty_queue_returns_zero` — Empty queue returns 0, output pointers untouched.
- `poll_event_into_click_event` — Click event fields correctly written to output pointers.
- `poll_event_into_input_event_with_value` — Input event with unicode string payload reconstructed correctly.
- `poll_event_into_multiple_events_in_order` — Events polled in FIFO order, queue drains properly.
- `raycast_panels_into_hit` — Ray hitting a panel writes correct panel_id, UV, and distance.
- `raycast_panels_into_miss` — Ray missing all panels returns 0.
- `get_pose_into_headless_returns_invalid` — Headless mode returns 0 with identity quaternion.

**Updated Mojo FFI wrappers** (in `xr/native/src/xr/xr_blitz.mojo`):

- `poll_event()` — Now uses `mxr_poll_event_into()` with per-field output pointers. Fully functional (was previously returning empty events).
- `raycast_panels()` — Now uses `mxr_raycast_panels_into()` with output pointers. Fully functional (was previously returning miss).
- `get_pose()` — Now uses `mxr_get_pose_into()` with output pointers. Fully functional (was previously returning invalid pose).

**Updated C header** (`xr/native/shim/mojo_xr.h`) — Added declarations for all three `_into()` functions with full documentation.

**Resolved limitations from Phase 5.3:**

- ✅ `poll_event()` — Now works via `mxr_poll_event_into()`.
- ✅ `raycast_panels()` — Now works via `mxr_raycast_panels_into()`.
- ✅ `get_pose()` — Now works via `mxr_get_pose_into()`.
- ⏳ Template registration via Mojo-side interpreter — Still requires `mxr_panel_register_template_by_node()` for live DOM node registration. Works fine via Rust-side interpreter (`panel_apply_mutations`).

---

**Recently completed:**

- ✅ **OpenXR backend integration** (`openxr_backend.rs`, ~1,190 lines) — full OpenXR + Vulkan session lifecycle, frame loop, input, swapchain management, wgpu/Vello rendering integration. All 48 existing tests pass. Graceful fallback to headless when OpenXR runtime unavailable.

### Phase 5.3: Mojo FFI Bindings for OpenXR Shim — ✅ Complete

Implemented typed Mojo FFI bindings for all ~80 XR shim C functions, plus a per-panel mutation interpreter that translates binary opcodes into XR Blitz FFI calls. Follows the same architecture as the desktop renderer (`blitz.mojo` + `renderer.mojo`).

**New files:**

- **`xr/native/src/xr/xr_blitz.mojo`** — `XRBlitz` struct wrapping all `mxr_*` C functions via `DLHandle`. ~70 typed methods covering:
  - **Session lifecycle** — `create_session()`, `create_headless()`, `session_state()`, `is_alive()`, `destroy()`
  - **Panel lifecycle** — `create_panel()`, `destroy_panel()`, `panel_count()`
  - **Panel transform & display** — `panel_set_transform()`, `panel_set_size()`, `panel_set_visible()`, `panel_is_visible()`, `panel_set_curved()`
  - **Mutation batching** — `panel_begin_mutations()`, `panel_end_mutations()`, `panel_apply_mutations()` (Rust-side interpreter)
  - **Per-panel DOM operations** — `panel_create_element()`, `panel_create_text_node()`, `panel_create_placeholder()`, `panel_set_attribute()`, `panel_remove_attribute()`, `panel_set_text_content()`, `panel_append_children()`, `panel_insert_before()`, `panel_insert_after()`, `panel_replace_with()`, `panel_remove_node()`
  - **Templates** — `panel_register_template()`, `panel_clone_template()`
  - **Tree traversal** — `panel_node_at_path()`, `panel_child_at()`, `panel_child_count()`
  - **Events** — `panel_add_event_listener()`, `panel_add_event_listener_by_name()`, `panel_remove_event_listener()`, `poll_event()`, `event_count()`, `event_clear()`, `panel_inject_event()`
  - **Raycasting** — `raycast_panels()`, `set_focused_panel()`, `get_focused_panel()`
  - **Frame loop** — `wait_frame()`, `begin_frame()`, `render_dirty_panels()`, `end_frame()`
  - **Input** — `get_pose()`, `get_aim_ray()` (output-pointer pattern)
  - **Reference spaces** — `set_reference_space()`, `get_reference_space()`
  - **Capabilities** — `has_extension()`, `has_hand_tracking()`, `has_passthrough()`
  - **ID mapping & stack** — `panel_assign_id()`, `panel_resolve_id()`, `panel_stack_push()`, `panel_stack_pop()`
  - **Debug/inspection** — `panel_print_tree()`, `panel_serialize_subtree()`, `panel_get_node_tag()`, `panel_get_text_content()`, `panel_get_attribute_value()`, `panel_get_child_mojo_id()`, `version()`
  - **Helper types** — `XREvent` (with panel targeting + UV hit coords + hand), `XRPose`, `XRRaycastHit`
  - **Constants** — All `EVT_*`, `HAND_*`, `SPACE_*`, `STATE_*` constants mirroring `mojo_xr.h`
  - **Library search** — `MOJO_XR_LIB` env var → `NIX_LDFLAGS` → `LD_LIBRARY_PATH` → fallback

- **`xr/native/src/xr/renderer.mojo`** — `XRMutationInterpreter` struct. Per-panel opcode interpreter that reads the same binary mutation buffer as the desktop interpreter but targets `XRBlitz` FFI calls scoped to a `panel_id`. Handles all 18 opcodes: `END`, `APPEND_CHILDREN`, `ASSIGN_ID`, `CREATE_PLACEHOLDER`, `CREATE_TEXT_NODE`, `LOAD_TEMPLATE`, `REPLACE_WITH`, `REPLACE_PLACEHOLDER`, `INSERT_AFTER`, `INSERT_BEFORE`, `SET_ATTRIBUTE`, `SET_TEXT`, `NEW_EVENT_LISTENER`, `REMOVE_EVENT_LISTENER`, `REMOVE`, `PUSH_ROOT`, `REGISTER_TEMPLATE`, `REMOVE_ATTRIBUTE`. Includes `BufReader` for little-endian buffer decoding (same as desktop).

- **`xr/native/src/xr/__init__.mojo`** — Updated with re-exports for `XRBlitz`, `XRMutationInterpreter`, `XRPose`, `XRRaycastHit`, and all constants.

**Known limitations** (resolved in Step 5.2b):

- ~~`poll_event()` — Returns empty event; needs `mxr_poll_event_into()` shim function.~~ ✅ Resolved — `mxr_poll_event_into()` added in Step 5.2b.
- ~~`raycast_panels()` — Returns miss; needs `mxr_raycast_panels_into()`.~~ ✅ Resolved — `mxr_raycast_panels_into()` added in Step 5.2b.
- ~~`get_pose()` — Returns invalid pose; needs `mxr_get_pose_into()`.~~ ✅ Resolved — `mxr_get_pose_into()` added in Step 5.2b.
- Template registration via Mojo-side interpreter — Templates built as live DOM nodes can't be registered for clone-based instantiation without `mxr_panel_register_template_by_node()`. Works fine when using Rust-side interpreter (`panel_apply_mutations`).

---

### Phase 5.2: Real Blitz Documents in XR Shim — ✅ Complete

Replaced the lightweight `HeadlessNode` DOM tree in the XR shim with real Blitz `BaseDocument` instances — the same CSS engine used by the desktop renderer. Each XR panel now owns a full Blitz document with Stylo styling and Taffy layout.

**Key changes** (`xr/native/shim/src/lib.rs`):

- **Panel now owns a `BaseDocument`** — replaced `nodes: HashMap<u32, HeadlessNode>` with `doc: BaseDocument` plus `id_to_node`/`node_to_id` maps (same pattern as desktop shim). Mount point is `<body>` (was `<div>`).
- **All DOM operations delegate to Blitz** — `create_element` → `doc.mutate().create_element(QualName)`, `set_attribute` → `doc.mutate().set_attribute()`, etc. No more manual parent/child tracking.
- **Template cloning via `deep_clone_node`** — templates are stored as detached Blitz subtrees, deep-cloned on use (same as desktop shim).
- **DOM inspection uses Blitz node API** — `get_node_tag`, `get_text_content`, `get_attribute_value`, `serialize_subtree` all use `doc.get_node()` and `NodeData` matching.
- **Layout resolution in render loop** — `mxr_render_dirty_panels` calls `panel.doc.resolve(0.0)` to exercise Stylo + Taffy (future: Vello offscreen rendering).
- **New FFI functions** — `mxr_panel_assign_id`, `mxr_panel_resolve_id`, `mxr_panel_stack_push`, `mxr_panel_stack_pop` (mutation interpreter support).
- **6 new tests** (30 total, up from 24) — `id_mapping_assign_and_resolve`, `stack_push_and_pop`, `multi_panel_dom_isolation`, `blitz_document_structure`, `blitz_nested_elements_with_attributes`, `layout_resolve_in_render`.
- **Version bumped** to 0.2.0.

**What's NOT yet wired up** (deferred to Step 5.2b or 5.3):

- Vello offscreen rendering to GPU textures (needs wgpu device setup)
- OpenXR session lifecycle (`openxr` crate integration)
- UA stylesheet application to Blitz documents
- Binary opcode interpreter on the Rust side (Mojo-side interpreter calls individual FFI functions)

---

### Phase 5.1: XR Panel Abstraction Design — ✅ Complete

Designed and implemented the XR panel abstraction, scene manager, and Rust shim scaffold. Created the `xr/` directory structure with native and web sub-projects.

**Mojo types** (`xr/native/src/xr/`):

- `panel.mojo` — `XRPanel` (2D DOM document + 3D transform), `PanelConfig`, `PanelState`, `Vec3`, `Quaternion`. Panel presets: `default_panel_config()` (0.8m × 0.6m, 1200 ppm), `dashboard_panel_config()` (1.6m × 0.9m curved), `tooltip_panel_config()` (0.3m × 0.15m non-interactive), `hand_anchored_panel_config()` (0.2m × 0.15m).
- `scene.mojo` — `XRScene` (panel registry, focus management, dirty tracking, raycasting via ray-plane intersection, spatial layout helpers). `XREvent` with panel targeting and UV hit coordinates. `RaycastHit`. Layout helpers: `arrange_arc()`, `arrange_grid()`, `arrange_stack()`. Convenience constructors: `create_single_panel_scene()`, `create_dual_panel_scene()`.
- `__init__.mojo` — Package root with re-exports.

**Rust shim scaffold** (`xr/native/shim/`):

- `src/lib.rs` — `XrSessionContext` with headless mode (`mxr_create_headless`), multi-panel DOM (`Panel` with `HeadlessNode` tree), ID mapping, interpreter stack, event ring buffer, per-panel DOM operations (create/set/remove element/text/attribute/children), raycasting, DOM serialization, and 20+ integration tests covering: session lifecycle, panel creation/destruction, visibility, DOM element creation, text nodes, attributes, insert before/after, replace/remove, events (inject + poll, listener registration), raycasting (hit/miss/hidden-panel skip), focus management, frame loop, reference spaces, serialization, placeholder nodes, path navigation, UA stylesheets, version string.
- `mojo_xr.h` — C API header (~80 functions): session lifecycle, panel management, mutations, per-panel DOM operations, events, raycasting, frame loop, input (pose/aim ray), reference spaces, capabilities, debug/inspection.
- `Cargo.toml` — Blitz v0.2.0 (same rev as desktop), anyrender, anyrender_vello, wgpu, openxr.
- `default.nix` — Nix derivation with Blitz + OpenXR + GPU dependencies.

**Core platform updates** (`core/src/platform/features.mojo`):

- Added `has_xr`, `has_xr_hand_tracking`, `has_xr_passthrough` fields to `PlatformFeatures`.
- Added `xr_native_features()` and `xr_web_features()` preset constructors.
- Updated all existing presets (`web_features`, `desktop_blitz_features`, `native_features`) to include the new XR fields (all False for non-XR renderers).

---

### Pre-separation: Reactive Framework (Phases 0–41) — ✅ Complete

The core reactive framework was built incrementally over 42 phases in the original `mojo-wasm` monolith. See [CHANGELOG.md](CHANGELOG.md) for the full history. Key capabilities:

- **Signals & reactivity** — `SignalI32`, `SignalBool`, `SignalString` with automatic subscriber tracking
- **Derived signals (Memos)** — `MemoI32`, `MemoBool`, `MemoString` with equality-gated propagation and recursive worklist-based dirtying
- **Effects** — Reactive side effects with drain-and-run flush pattern
- **Batch signal writes** — `begin_batch()` / `end_batch()` for grouped writes with single propagation pass
- **Virtual DOM** — Templates, VNodes, DSL builders, create/diff engines
- **Binary mutation protocol** — 18 opcodes serialized to a shared buffer, interpreted by each renderer
- **Component framework** — `AppShell`, `ComponentContext`, `ChildComponentContext`, `KeyedList`, `ConditionalSlot`, `Router`
- **Error boundaries & suspense** — Nested error boundaries with crash/retry, suspense boundaries with pending/resolve lifecycle
- **HTML DSL** — `el_div(el_h1(dyn_text()), el_button(text("Up!"), onclick_add(count, 1)))` — Dioxus `rsx!`-style nesting
- **Two-way input binding** — `bind_value(signal)` + `oninput_set_string(signal)` for Dioxus-style controlled inputs
- **Client-side routing** — `Router` struct with URL path matching, `ConditionalSlot` view switching, JS history integration

### Separation Phase 1: Extract `core/` — ✅ Complete

Moved all renderer-agnostic modules from the monolith into `core/src/`:

- `signals/`, `scope/`, `scheduler/`, `arena/`, `mutations/`, `bridge/`, `events/`, `component/`
- Split `vdom/` into `vdom/` (renderer-agnostic primitives) + `html/` (HTML vocabulary and DSL helpers)
- Moved tests to `core/test/` (52 test suites)

### Separation Phase 2: Create `web/` — ✅ Complete

Moved all browser/WASM-specific files into `web/`:

- `web/src/main.mojo` — @export WASM wrappers
- `web/src/apps/` → later moved to `examples/apps/` — 15 test/demo app modules
- `web/runtime/` — TypeScript runtime (DOM interpreter, events, templates) — 11 modules
- `web/test-js/` — JS integration tests (29 suites, ~3,090 tests)
- `web/scripts/` — Build pipeline (nu scripts)

### Separation Phase 3: Desktop + Unified Lifecycle — ✅ Complete

**`GuiApp` trait** (`core/src/platform/gui_app.mojo`) — the app-side lifecycle contract:

```text
trait GuiApp(Movable):
    fn __init__(out self)
    fn render(mut self) -> UInt32
    fn handle_event(mut self, handler_id: UInt32, event_type: UInt8, value: String) -> Bool
    fn mount(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn flush(mut self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]) -> Int32
    fn has_dirty(self) -> Bool
    fn consume_dirty(mut self) -> Bool
    fn destroy(mut self)
```

**Compile-time target dispatch** (`core/src/platform/launch.mojo`):

```text
fn launch[AppType: GuiApp](config: AppConfig = AppConfig()) raises:
    @parameter
    if is_wasm_target():
        pass  # JS runtime drives the loop; @export wrappers call GuiApp methods
    else:
        from desktop.launcher import desktop_launch
        desktop_launch[AppType](config)
```

All 4 shared example apps (Counter, Todo, Benchmark, MultiView) implement `GuiApp` and compile for both targets from identical source. Each has a single `main.mojo`:

```text
from platform.launch import launch, AppConfig
from counter import CounterApp

fn main() raises:
    launch[CounterApp](AppConfig(title="High-Five Counter", width=400, height=350))
```

### Separation Phase 4: Desktop Blitz Renderer — ✅ Complete

Replaced the initial webview dependency with [Blitz](https://github.com/DioxusLabs/blitz), a native HTML/CSS rendering engine using Stylo (CSS) + Taffy (layout) + Vello (GPU rendering) + Winit (windowing, Wayland-only) + AccessKit (a11y). No JS runtime, no IPC — mutations applied in-process via direct C FFI calls.

- **Blitz C shim** (`desktop/shim/src/lib.rs`) — Rust cdylib with ~37 FFI functions (lifecycle, DOM ops, templates, events, stack ops, ID mapping, debug)
- **C header** (`desktop/shim/mojo_blitz.h`) — flat C ABI with opaque `MblitzContext*`, event struct, event type constants
- **Mojo FFI bindings** (`desktop/src/desktop/blitz.mojo`) — typed `Blitz` struct wrapping the C shim
- **Mutation interpreter** (`desktop/src/desktop/renderer.mojo`) — binary opcodes → Blitz FFI calls (Mojo equivalent of the JS `Interpreter`)
- **Generic desktop launcher** (`desktop/src/desktop/launcher.mojo`) — `desktop_launch[AppType: GuiApp](config)` with Winit event loop integration, blocking/non-blocking step, event polling, dirty-scope flush cycle
- **Nix derivation** (`desktop/shim/default.nix`) — automates Rust build with Wayland + GPU deps
- **User-agent CSS** — default stylesheet for consistent rendering of HTML elements

Runtime verified — all 4 desktop windows launch and render on Wayland with Vello GPU rendering.

### Phase 4.7: Project Infrastructure — ✅ Complete

- Root `justfile` with web + desktop commands (build, test, serve, clean)
- Root `default.nix` combining web and desktop dependencies (Mojo, Deno, LLVM, wabt, wasmtime, Rust, Wayland/Vulkan libs)
- READMs updated across all sub-projects

### Phase 4.8: Stabilization & Verification — ✅ Complete (I-1, I-2, I-3, I-4)

**I-1: Cross-target build verification** — Verified all 4 shared examples (Counter, Todo, Benchmark, MultiView) build for both web (WASM) and desktop (native). 52 Mojo test suites pass (via wasmtime). 3,090 JS integration tests pass (via Deno). No regressions from the separation.

**I-3: Mutation protocol conformance tests** — New `conformance.test.ts` test suite (285 tests) covering:

- **Binary round-trip** — Every opcode (16 of 16) verified through MutationBuilder → MutationReader decode cycle, including unicode text, empty strings, and edge cases (max u32 IDs, deep paths, long strings, special characters)
- **RegisterTemplate serialization** — Round-trip verification for element nodes, static/dynamic children, static/dynamic attributes, dynamic text slots, and dynamic node slots
- **Canonical DOM output** — 12 UI patterns verified through the Interpreter with deterministic DOM serialization:
  - Text node mount, placeholder mount, template element mount
  - Template + dynamic text + SetText, template + dynamic attr + SetAttribute
  - Static template with text and attributes
  - SetText update, SetAttribute + RemoveAttribute cycle
  - Remove, ReplaceWith, InsertAfter, InsertBefore (including multi-node)
  - Counter-like mount + increment sequence
  - Todo-like mount + add item + remove item sequence
  - Conditional rendering (show/hide detail via placeholder swap)
  - Keyed list reorder simulation
  - Multiple templates, accumulation across apply calls
  - Complex app with all opcode types exercised (6-step lifecycle)
- **WASM ↔ JS byte-level comparison** — All 16 opcodes verified byte-identical between Mojo `write_op_*` exports and JS `MutationBuilder`, including string-carrying opcodes with `writeStringStruct`, unicode text, and multi-op sequences
- **Binary layout verification** — Exact byte offsets verified for PushRoot, AppendChildren, CreateTextNode, SetAttribute, End, Remove, AssignId, and LoadTemplate against the protocol spec
- **End-to-end WASM → Interpreter → DOM** — WASM-generated mutation buffers applied through the JS Interpreter, verifying correct DOM output for mount, text update, and attribute set/remove sequences

Total JS test count: 30 suites, ~3,375 tests (up from 29 suites, ~3,090 tests).

**I-2: Desktop integration tests** — Headless mode (`mblitz_create_headless`) and DOM inspection API added to the Blitz shim. 69 Rust integration tests in `desktop/shim/tests/integration.rs` covering:

- **Context lifecycle** — headless creation, mount point resolution, alive state
- **DOM operations** — element creation (21 tag types), text nodes (incl. unicode, empty), placeholder/comment nodes
- **Attributes** — set, get, overwrite, remove, multiple per element, nonexistent lookups
- **Tree structure** — append children (single, multiple), nested trees, insert before/after, replace (single/multi-node), remove with ID cleanup
- **Templates** — register, clone, verify independent copies
- **Path navigation** — `node_at_path` traversal to deeply nested children
- **Events** — inject click/input events, poll ordering, handler registration/removal, unicode values
- **Mutation batching** — batch flag state transitions
- **DOM serialization** — empty mount point, single/nested children, text, attributes, placeholders, subtree-only, quote escaping
- **ID mapping & stack** — bidirectional mapping, push/pop/pop-more-than-available, child mojo ID lookups (incl. out-of-bounds and unmapped)
- **Integration scenarios** — counter-like mount+update, conditional rendering (placeholder ↔ element swap), todo-like list add/remove
- **Stress tests** — 100 children, 20-deep nesting, 1000 rapid text updates, ID reassignment

New shim FFI functions: `mblitz_create_headless`, `mblitz_get_node_tag`, `mblitz_get_text_content`, `mblitz_get_attribute_value`, `mblitz_serialize_subtree`, `mblitz_inject_event`, `mblitz_get_child_mojo_id`. C header and Cargo config updated. `just test-desktop` recipe added.

**I-4: Document CSS support scope** — Created [docs/plan/css-support.md](docs/plan/css-support.md) documenting Blitz v0.2.0 CSS feature support across parsing (Stylo), layout (Taffy), and rendering (Vello). Covers fully supported features (box model, flexbox, grid, positioning, typography, backgrounds, borders, shadows, selectors, variables), partially supported features (fixed/sticky positioning, table layout, 2D transforms, intrinsic sizing), and unsupported features (transitions, animations, 3D transforms, filters, float, multi-column). Includes safe-to-use CSS subset for cross-renderer apps and upgrade audit checklist.

---

## Roadmap

### Immediate — Stabilization & Verification — ✅ Complete

All four stabilization tasks are complete.

| # | Task | Effort | Notes |
|---|------|--------|-------|
| I-1 | **Cross-target build verification** | 1–2 days | ✅ Complete — All 4 shared examples (Counter, Todo, Benchmark, MultiView) build for both web (WASM) and desktop (native). 52 Mojo test suites pass (via wasmtime). 3,090 JS integration tests pass (via Deno). No regressions from the separation. |
| I-2 | **Desktop integration tests** | 3–5 days | ✅ Complete — Headless mode added to Blitz shim (`mblitz_create_headless`). 69 Rust integration tests covering: context lifecycle, DOM element creation, text nodes (incl. unicode), attribute get/set/remove, tree structure (append/insert/replace/remove), template registration & cloning, event injection & polling, mutation batch markers, DOM serialization, node ID mapping & stack ops, counter-like mount+update scenario, conditional rendering (placeholder swap), todo-like list add/remove, stress tests (100 children, 20-deep nesting, 1000 rapid text updates). New DOM inspection FFI: `mblitz_get_node_tag`, `mblitz_get_text_content`, `mblitz_get_attribute_value`, `mblitz_serialize_subtree`, `mblitz_inject_event`, `mblitz_get_child_mojo_id`. Run via `just test-desktop`. |
| I-3 | **Mutation protocol conformance tests** | 2–3 days | ✅ Complete — `conformance.test.ts` (285 tests): binary round-trip for every opcode, RegisterTemplate serialization, canonical DOM output for 12 UI patterns (counter, todo, conditional, keyed list, nested templates, complex app), byte-level WASM↔JS comparison for all 16 opcodes + unicode, exact binary layout verification, and end-to-end WASM→Interpreter→DOM rendering. |
| I-4 | **Document CSS support scope** | 1 day | ✅ Complete — [docs/plan/css-support.md](docs/plan/css-support.md): comprehensive audit of Blitz v0.2.0 CSS support. Fully supported: box model, flexbox, CSS grid, relative/absolute positioning, typography, colors/backgrounds, borders, box shadows, overflow, selectors/cascade, CSS variables, calc(), media queries. Partially supported: `position: fixed/sticky`, `display: table/contents`, 2D transforms, `min-content`/`max-content`. Not supported: transitions, animations, 3D transforms, filters, float, multi-column, text-shadow, clip-path. Includes app authoring recommendations and desktop-safe CSS subset. |

### Short-term — Cross-Platform & CI

| # | Task | Effort | Notes |
|---|------|--------|-------|
| S-1 | **Cross-target CI pipeline** | 3–5 days | ✅ Complete — 6 Nix check derivations in `mojo-gui/default.nix`: `mojo-gui-test-desktop` (75 Rust tests), `mojo-gui-test-xr` (48 Rust tests incl. GPU), `mojo-gui-test` (52 Mojo suites via wasmtime), `mojo-gui-test-js` (3,375 JS tests via Deno with pre-fetched npm cache FOD), `mojo-gui-test-xr-js` (414 XR web JS tests via Deno), `mojo-gui-build-all` (4 examples × 3 targets). All run in the Nix sandbox without network access. Gated on `nix flake check` via Tangled CI. |
| S-2 | **macOS desktop verification** | 2–3 days | Blitz uses Winit which supports macOS. Build the Blitz shim on macOS, verify `cargo build --release` succeeds, run the Counter example. Document any platform-specific quirks (GPU backend selection, font fallback, etc.). |
| S-3 | **Windows desktop verification (Wine)** | 2–3 days | ✅ Complete — Cross-compiled to `x86_64-pc-windows-gnu` from Linux via MinGW-w64. Produces `mojo_blitz.dll` (26MB PE32+ DLL). All 69 Rust integration tests pass under Wine (single-threaded; Wine's COM layer crashes with parallel threads — misaligned pointer in `windows-core` interface dispatch). Nix dev shell provides: `rust-bin` with Windows target std, MinGW-w64 cross-linker (stripped setup hooks to avoid polluting native CC/AR), Wine 10.0 for test execution. New justfile recipes: `build-shim-windows`, `test-desktop-wine`, `build-shim-all`. Cargo config (`.cargo/config.toml`) sets MinGW linker and Wine runner for the Windows target. |

### Medium-term — Phase 5: XR Renderer

> **Full plan:** [docs/plan/phase5-xr.md](docs/plan/phase5-xr.md) — **Effort estimate: 4–8 weeks**

XR panel abstraction that reuses the binary mutation protocol unchanged. Each XR panel gets its own `GuiApp` instance and mutation buffer.

| Step | Description | Status |
|------|-------------|--------|
| 5.1 | Design the XR panel abstraction (`XRPanel` struct, scene graph, placement) | ✅ Complete — `XRPanel`, `PanelConfig`, `Vec3`, `Quaternion`, `PanelState` (Mojo). `XRScene` with focus management, dirty tracking, raycasting (ray-plane intersection), spatial layout helpers (`arrange_arc`, `arrange_grid`, `arrange_stack`). Rust shim scaffold (`xr/native/shim/src/lib.rs`) with headless multi-panel DOM, event ring buffer, DOM serialization, raycasting, and 20+ integration tests. C API header (`mojo_xr.h`, ~80 functions). `PlatformFeatures` extended with `has_xr`, `has_xr_hand_tracking`, `has_xr_passthrough` and `xr_native_features()` / `xr_web_features()` presets. Panel presets: default, dashboard, tooltip, hand-anchored. |
| 5.2 | Build the OpenXR + Blitz Rust shim (offscreen Vello rendering → OpenXR swapchain textures) | ✅ Complete — **real Blitz documents ✅** (HeadlessNode replaced with BaseDocument, Stylo+Taffy layout resolves). **Output-pointer FFI variants ✅** (`mxr_poll_event_into`, `mxr_raycast_panels_into`, `mxr_get_pose_into`). **Vello offscreen rendering ✅** (`OffscreenRenderer` with wgpu+Vello, `mxr_init_gpu`/`mxr_has_gpu`/`mxr_panel_read_pixels`, 48 tests pass including 11 GPU tests). **OpenXR backend ✅** (`openxr_backend.rs`: session lifecycle via `Entry::load()` + Vulkan graphics binding, frame loop with `xrWaitFrame`/`xrBeginFrame`/`xrEndFrame`, per-panel swapchain management with quad layer compositing, controller input via action sets with grip/aim poses + select/squeeze, wgpu/Vello rendering to swapchain images via shared Vulkan device, graceful headless fallback). |
| 5.3 | Mojo FFI bindings for the OpenXR shim | ✅ Complete — `XRBlitz` struct (~70 methods wrapping all `mxr_*` C functions via DLHandle). `XRMutationInterpreter` (per-panel binary opcode interpreter, all 18 opcodes). Helper types: `XREvent`, `XRPose`, `XRRaycastHit`. Constants for events, hands, spaces, states. Library search via env vars / Nix / ld paths. `poll_event()`, `raycast_panels()`, `get_pose()` now fully functional via `_into()` output-pointer variants. |
| 5.4 | XR scene manager and panel routing (multiplexes mutation buffers) | ✅ Complete (single-panel) — `XRScene` provides panel registry, focus management, dirty tracking, Mojo-side raycasting (ray-plane intersection), and spatial layout helpers (`arrange_arc`, `arrange_grid`, `arrange_stack`). For single-panel apps, `xr_launch` (Step 5.5) manages the panel directly via `XRBlitz` FFI — bypassing the scene for simplicity. Multi-panel routing through `XRScene` (scene creates/destroys panels via shim, multiplexes mutation buffers to correct panel's `GuiApp`) deferred to Step 5.9 (multi-panel XR API). |
| 5.5 | `xr_launch[AppType: GuiApp]()` — single-panel apps get XR for free | ✅ Complete — `xr/native/src/xr/launcher.mojo`. Creates headless/OpenXR session, allocates default panel (size from AppConfig), applies XR UA stylesheet, mounts app, enters XR frame loop (wait_frame → poll_event → handle_event → flush → apply mutations → render → end_frame). Same mutation buffer management as desktop launcher. |
| 5.6 | WebXR JS runtime (DOM → texture, XR session management) | 🔧 In progress — `xr/web/runtime/` created: `XRSessionManager` (WebXR session lifecycle, reference space negotiation, frame loop delegation), `XRPanelManager` + `XRPanel` (offscreen DOM containers, SVG foreignObject rasterization, WebGL texture upload, raycasting, spatial layout helpers), `XRQuadRenderer` (WebGL2 textured quad shader, per-view stereo rendering, cursor visualization), `XRInputHandler` (controller/hand ray → panel raycast → DOM pointer event synthesis with hover tracking, select→click sequences, focus management), `XRRuntime` (main entry point tying all subsystems together, WASM app loading, shared `Interpreter` + `TemplateCache` from `web/runtime/`, "Enter VR" button, flat-fallback mode). **JS test suite ✅** (523 tests across 5 suites: types, panel, input, runtime, rasterize — run via `just test-xr-js`). **Shared Interpreter integration ✅** (replaced ~420-line inline mutation applier with `web/runtime/Interpreter`; added `onRemoveListener` callback). **SVG foreignObject fidelity tests ✅** (109 tests validating markup structure, fallback rasterizer, dirty tracking, mutation→rasterize flow, texture upload, manager orchestration, edge cases). **XR web examples ✅** (4 apps with HTML+JS entry points, shared `launchXR()` launcher, esbuild TS→JS bundler). **Browser E2E test script ✅** (flat-fallback validation via headless Servo + WebDriver). **Nix check ✅** (`mojo-gui-test-xr-js`). Remaining: end-to-end testing with a real WebXR device/emulator, real-device SVG foreignObject pixel-level fidelity validation. |
| 5.7 | Wire `launch()` for XR targets (`@parameter if is_xr_target()`) | ✅ Complete — `launch()` now dispatches: WASM → web, `-D MOJO_TARGET_XR` → `xr_launch`, native → `desktop_launch`. Added `is_xr_target()` compile-time detection. |
| 5.8 | Verify shared examples in XR (all 4 apps render as floating panels) | ✅ Complete — All 4 shared examples (Counter, Todo, Benchmark, MultiView) build and run in XR headless mode. Fixed: Mojo `@parameter if` import resolution (all renderer `-I` paths needed), `performance_now()` cross-platform support (native uses `perf_counter_ns`), headless frame loop exit (idle frame counter replaces broken `predicted_time == 0` check). |
| 5.9 | Multi-panel XR API — `XRGuiApp` trait for apps managing multiple panels (stretch goal) | 🔮 Future |

**Compile targets after Phase 5:**

```text
mojo build --target wasm64-wasi -I core/src -I web/src                        → web
mojo build --target wasm64-wasi --feature webxr                               → WebXR browser (future)
mojo build -I core/src -I desktop/src -I xr/native/src                        → desktop (Blitz)
mojo build -D MOJO_TARGET_XR -I core/src -I xr/native/src -I desktop/src      → OpenXR native
```

### Medium-term — Phase 6: `mojo-web` Raw Bindings

> **Full plan:** [docs/plan/phase6-mojo-web.md](docs/plan/phase6-mojo-web.md) — **Effort estimate: 2–3 weeks**

Standalone Mojo library providing typed bindings to Web APIs — the Mojo equivalent of Rust's `web-sys`. Independent of `mojo-gui`; usable by any Mojo/WASM project.

| Module | Web APIs |
|--------|----------|
| `dom` | Document, Element, Node, Text, Event |
| `fetch` | fetch, Request, Response, Headers |
| `timers` | setTimeout, setInterval, requestAnimationFrame |
| `storage` | localStorage, sessionStorage |
| `console` | console.log, warn, error |
| `url` | URL, URLSearchParams |
| `websocket` | WebSocket |
| `canvas` | Canvas2D (WebGL future) |

Architecture: JS-side handle table (integer IDs → JS objects) + WASM imports + Mojo typed wrappers. Same proven pattern as the existing `mojo-wasm` JS interop.

**Relationship to `mojo-gui`:** `mojo-gui` uses the mutation protocol for rendering, NOT `mojo-web`. Apps can optionally import `mojo-web` for non-rendering web features (fetch for suspense, localStorage, WebSocket) behind `@parameter if is_wasm_target()` gates.

### Long-term

| # | Task | Notes |
|---|------|-------|
| L-1 | **Animation framework** | Declarative transitions and spring animations integrated with the reactive system. Mutations include interpolated style updates. |
| L-2 | **Hot reload (web)** | Watch mode that recompiles WASM on source change and hot-swaps into the running page. Build on the existing `just serve` + file watcher. |
| L-3 | **Component library** | Reusable UI components (Modal, Dropdown, Tabs, Toast, etc.) built on the HTML DSL. Ships as a Mojo package alongside `core`. |
| L-4 | **Devtools** | Reactive graph inspector — visualize signals, memos, effects, and their dependencies. Desktop: overlay via Blitz. Web: browser extension or side panel. |
| L-5 | **Server-side rendering** | Render the initial VNode tree to static HTML on the server, hydrate on the client. Requires serializing the VNode store. |
| L-6 | **Mobile targets** | Investigate Winit's Android/iOS support for Blitz-based mobile rendering. |

---

## Key Architecture Decisions

These decisions are settled and documented here for reference. See [architecture.md](docs/plan/architecture.md) for full rationale.

1. **Binary mutation protocol as the renderer contract.** Core never touches platform APIs. It writes opcodes to a byte buffer; each renderer interprets them. This enables multi-renderer support from identical app source.

2. **DOM-oriented model in core.** HTML/CSS is the universal UI description language across all targets: real DOM (web), Blitz DOM (desktop), Blitz DOM per-panel (XR native), real DOM per-panel (WebXR). The HTML vocabulary (`html/`) stays in core.

3. **`GuiApp` trait + `launch()` for platform abstraction.** Apps implement `GuiApp`; `launch[AppType]()` dispatches to the right renderer at compile time via `@parameter if is_wasm_target()`. No per-renderer app code.

4. **Shared examples as the correctness gate.** If an example doesn't work on a target, it's a framework bug — not an app authoring problem. Examples live in `examples/`, never per-renderer.

5. **Mono-repo with path-based imports.** `mojo-gui/` is the workspace root. `-I core/src -I examples` for cross-package imports. Works around Mojo's current package system limitations.

6. **Blitz over webview for desktop.** No JS runtime, no IPC, no base64 encoding. Direct in-process C FFI. Cross-platform via Winit. Consistent CSS via Stylo. Pinned to Blitz v0.2.0 (rev `2f83df96`).

7. **All renderer include paths in every native build.** Mojo's `@parameter if` does not suppress import resolution in dead branches — the compiler resolves all `from X import Y` statements regardless of which compile-time branch is active. The `launch()` function imports from `desktop.launcher` and `xr.launcher` in different `@parameter if` branches, so all native builds must provide `-I desktop/src -I xr/native/src` even though only one renderer is linked. The linker only pulls in the active branch's code; the extra `-I` paths are a compile-time workaround only. This was discovered during Step 5.8 verification and applies to all current and future renderer backends added to `launch()`.

---

## Open Risks

| Risk | Impact | Mitigation | Status |
|------|--------|------------|--------|
| Platform abstraction too leaky | Shared examples break on some targets | Cross-target CI matrix as gate; treat failures as framework bugs | ✅ Mitigated — `GuiApp` + `launch()` complete; 6 CI checks gate all PRs via `nix flake check` |
| Blitz pre-alpha stability | Rendering bugs, missing CSS | Track Blitz releases; pin versions; document CSS support scope | Mitigated — pinned to v0.2.0 |
| Native target module-level `var` | Global `var` not supported on native | Avoided in current design; config passed as arguments, not globals | ✅ Resolved |
| Mojo trait limitations | `GuiApp` may not support future needs | `alias CurrentApp = ...` as fallback; upgrade when Mojo improves | ✅ Resolved for current scope |
| Mojo `@parameter if` import resolution | Dead branches still trigger import resolution; adding a new renderer backend to `launch()` requires updating ALL native build commands | Include all renderer `-I` paths in every native build; document in justfile and Architecture Decisions (§7) | ✅ Mitigated — workaround in place since Step 5.8; will re-evaluate when Mojo improves `@parameter if` semantics |
| Mojo WASM runtime import drift | New Mojo versions may add new WASM imports (e.g. `clock_gettime` in 26.1.0) that break the test harness | Pin Mojo version; update both `wasm_harness.mojo` and `web/runtime/env.ts` when upgrading; check `wasm-objdump -j Import` after Mojo upgrades | ✅ Mitigated — `clock_gettime` added for 26.1.0 |
| OpenXR runtime availability (Phase 5) | XR fails without runtime | Detect at startup; fall back to headless | ✅ Mitigated — `OpenXrBackend::try_new()` loads OpenXR via dlopen; `mxr_create_session` falls back to headless transparently; "loaded" openxr feature decouples cdylib from loader at link time |
| DOM-to-texture fidelity for WebXR (Phase 5) | Rendering quality/interactivity loss | SVG foreignObject rasterization implemented with fallback text renderer; evaluate OffscreenCanvas, html2canvas for higher fidelity | 🔧 In progress — initial SVG foreignObject approach in `xr/web/runtime/xr-panel.ts`; needs real-device validation |
| XR input latency (Phase 5) | Raycasting → DOM event adds latency to controller input | Keep raycast math in the shim (Rust); minimize FFI roundtrips | In progress — Rust-side raycasting implemented |
| XR frame timing constraints (Phase 5) | OpenXR requires strict frame pacing; DOM re-render may exceed budget | Only re-render dirty panels; cache textures; use quad layers for compositor-side reprojection | In progress — dirty tracking per-panel implemented |

See [docs/plan/risks.md](docs/plan/risks.md) for the full risk register with 19 entries.