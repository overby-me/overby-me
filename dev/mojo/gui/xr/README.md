# mojo-gui/xr — XR Renderer (Phase 5)

Render mojo-gui panels into 3D XR (extended reality) environments via OpenXR. Each XR panel owns an independent Blitz DOM document rendered to an offscreen GPU texture by Vello. The OpenXR compositor places these textures as quad layers in the XR scene.

> **Status:** Steps 5.1–5.5, 5.7–5.8 complete. All 4 shared examples (Counter, Todo, Benchmark, MultiView) build and run as XR floating panels in headless mode. 37 integration tests pass. Remaining: Vello offscreen rendering + OpenXR session lifecycle (Step 5.2 remaining), WebXR JS runtime (Step 5.6), multi-panel API (Step 5.9).

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│  Native Process                                                  │
│                                                                  │
│  ┌─────────────────────┐                                         │
│  │  mojo-gui/core       │                                         │
│  │  (compiled native)   │── mutation buffer ──┐                   │
│  │  signals, vdom,      │    (per-panel)      │                   │
│  │  diff, scheduler     │◄── event dispatch ──┤                   │
│  └─────────────────────┘                     │                   │
│                                              ▼                   │
│  ┌─────────────────────────────────────────────────────────┐     │
│  │  XR Panel Manager (xr/native/src/xr/)                    │     │
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

### How It Works

1. **Each panel is a Blitz DOM document.** The same `blitz-dom` + Stylo + Taffy + Vello stack from the desktop renderer is reused. The only difference is the render target: offscreen GPU texture instead of a Winit window surface.

2. **The mutation protocol is unchanged.** Each panel receives the same binary opcode stream (`LOAD_TEMPLATE`, `SET_ATTRIBUTE`, `SET_TEXT`, `APPEND_CHILDREN`, etc.) as the web and desktop renderers. The core framework doesn't know it's running in XR.

3. **Input is bridged, not reinvented.** XR controller rays are intersected with panel quads in 3D space. The resulting 2D hit coordinates are translated to standard DOM pointer/click events and dispatched through the existing `HandlerRegistry`. App code doesn't know the click came from a VR controller.

4. **Panels are the spatial primitive.** A panel is a 2D DOM document placed at a 3D position/rotation in the XR scene. Apps create panels via the `XRScene` manager; each panel can host a separate `GuiApp` instance.

5. **Single-panel apps work unchanged.** Existing apps that call `launch[CounterApp](config)` get XR support for free — the XR launcher wraps them in a default panel automatically. No app code changes required.

## Packages

| Component | Path | Description |
|-----------|------|-------------|
| **Panel types** | `native/src/xr/panel.mojo` | `XRPanel`, `PanelConfig`, `PanelState`, `Vec3`, `Quaternion`, preset configs |
| **Scene manager** | `native/src/xr/scene.mojo` | `XRScene`, `XREvent`, `RaycastHit`, spatial layout helpers (`arrange_arc`, `arrange_grid`, `arrange_stack`) |
| **XR Blitz FFI** | `native/src/xr/xr_blitz.mojo` | `XRBlitz` struct — ~70 typed methods wrapping all `mxr_*` C functions via DLHandle, plus `XREvent`, `XRPose`, `XRRaycastHit` types and constants |
| **XR interpreter** | `native/src/xr/renderer.mojo` | `XRMutationInterpreter` — per-panel binary opcode interpreter (all 18 opcodes), `BufReader` for little-endian buffer decoding |
| **XR launcher** | `native/src/xr/launcher.mojo` | `xr_launch[AppType: GuiApp]()` — creates session, allocates panel, mounts app, runs XR frame loop |
| **XR Blitz shim** | `native/shim/src/lib.rs` | Rust cdylib — multi-panel Blitz BaseDocument, headless mode, raycasting, event ring buffer, DOM serialization, Stylo+Taffy layout, output-pointer FFI variants |
| **C API header** | `native/shim/mojo_xr.h` | Flat C ABI — ~83 functions: session lifecycle, panel management, mutations, events, frame loop, input, raycasting, debug (incl. 3 `_into()` output-pointer variants) |
| **Nix derivation** | `native/shim/default.nix` | Rust build with Blitz + OpenXR + GPU deps |
| **WebXR session** | `web/runtime/xr-session.ts` | `XRSessionManager` — WebXR session lifecycle, feature detection, reference space negotiation, WebGL2 context with `xrCompatible`, frame loop delegation |
| **WebXR panels** | `web/runtime/xr-panel.ts` | `XRPanel` + `XRPanelManager` — offscreen DOM containers, SVG foreignObject DOM→canvas rasterization, WebGL texture upload, raycasting, spatial layout helpers (`arrangeArc`, `arrangeGrid`, `arrangeStack`) |
| **WebXR renderer** | `web/runtime/xr-renderer.ts` | `XRQuadRenderer` — WebGL2 GLSL ES 3.0 textured quad shader, per-view stereo rendering, cursor visualization, GL state save/restore |
| **WebXR input** | `web/runtime/xr-input.ts` | `XRInputHandler` — controller/hand ray extraction, panel raycasting, hover tracking (enter/leave/move with ~30Hz throttle), select→click synthesis, focus management |
| **WebXR runtime** | `web/runtime/xr-runtime.ts` | `XRRuntime` — main entry point, WASM loading, `createAppPanel()`, inline mutation interpreter (all 18 opcodes), handler map for XR input→WASM dispatch, "Enter VR" button, flat-fallback mode |
| **WebXR types** | `web/runtime/xr-types.ts` | TypeScript types mirroring native XR panel types: `Vec3`, `Quaternion`, `PanelConfig` (with presets), `RaycastHit`, `XRRuntimeConfig`, WebXR API compat interfaces |
| **WebXR exports** | `web/runtime/mod.ts` | Module re-exports — single import path for the full WebXR public API |

## Project Structure

```text
xr/
├── native/                       # OpenXR native renderer
│   ├── shim/
│   │   ├── src/lib.rs            # Rust cdylib: multi-panel Blitz BaseDocument + raycasting + layout
│   │   │                         #   + output-pointer FFI variants for large struct returns
│   │   │                         #   + 37 integration tests (headless)
│   │   ├── tests/                # Rust integration tests
│   │   ├── mojo_xr.h            # C API header (~83 functions incl. 3 _into() variants)
│   │   ├── Cargo.toml           # Blitz + OpenXR + wgpu + Vello deps
│   │   └── default.nix          # Nix derivation with OpenXR/GPU deps
│   └── src/xr/
│       ├── __init__.mojo        # Package root with re-exports
│       ├── launcher.mojo        # xr_launch[AppType: GuiApp]() — XR frame loop
│       ├── xr_blitz.mojo        # XRBlitz FFI bindings to libmojo_xr.so (~70 methods)
│       ├── renderer.mojo        # XRMutationInterpreter (per-panel opcode interpreter)
│       ├── panel.mojo           # XRPanel, PanelConfig, Vec3, Quaternion, presets
│       └── scene.mojo           # XRScene, XREvent, RaycastHit, layout helpers
├── web/                          # WebXR browser renderer (Step 5.6 — 🔧 in progress)
│   ├── runtime/
│   │   ├── mod.ts                # Module re-exports — single import path
│   │   ├── xr-types.ts           # Vec3, Quaternion, PanelConfig, RaycastHit, XRRuntimeConfig, compat types
│   │   ├── xr-session.ts         # XRSessionManager — lifecycle, ref spaces, GL setup, frame loop
│   │   ├── xr-panel.ts           # XRPanel + XRPanelManager — DOM containers, SVG rasterization, textures
│   │   ├── xr-renderer.ts        # XRQuadRenderer — WebGL2 textured quad shader, stereo, cursor
│   │   ├── xr-input.ts           # XRInputHandler — raycasting, hover, select→click, focus
│   │   └── xr-runtime.ts         # XRRuntime — main entry, WASM loading, inline interpreter, flat fallback
│   └── src/                      # (future — Mojo WASM exports for WebXR feature flag)
└── README.md                     # This file
```

## Key Types

### XRPanel

A 2D DOM document placed in 3D XR space. Each panel has:

- **Identity** — unique `panel_id` assigned by the shim
- **3D Transform** — `position` (Vec3, meters), `rotation` (Quaternion)
- **Physical size** — `width_m`, `height_m` in meters
- **Pixel density** — `pixels_per_meter` (determines texture resolution and text legibility)
- **Display options** — `curved` (cylindrical surface), `interact` (accepts pointer input)
- **Runtime state** — `visible`, `focused`, `dirty`, `mounted`

### PanelConfig

Configuration for creating a panel. Provides presets for common use cases:

| Preset | Size | PPM | Use Case |
|--------|------|-----|----------|
| `default_panel_config()` | 0.8m × 0.6m | 1200 | General-purpose reading panel (similar to a 27" monitor) |
| `dashboard_panel_config()` | 1.6m × 0.9m | 1000 | Wide curved dashboard |
| `tooltip_panel_config()` | 0.3m × 0.15m | 800 | Small non-interactive HUD overlay |
| `hand_anchored_panel_config()` | 0.2m × 0.15m | 1400 | Panel attached to a controller/hand |

### XRScene

Top-level manager for all XR panels in a session:

- **Panel lifecycle** — `create_panel()`, `destroy_panel()`
- **Focus management** — exclusive keyboard/text focus, click-to-focus
- **Dirty tracking** — only re-render panels whose DOM has changed
- **Raycasting** — ray-plane intersection against all visible panels
- **Spatial layout helpers** — `arrange_arc()`, `arrange_grid()`, `arrange_stack()`

## Build & Test

### Prerequisites

All dependencies are provided by the Nix dev shell. If not using Nix, you need:

- [Rust toolchain](https://rustup.rs/) (for the XR shim)
- GPU/rendering libraries: Vulkan, libxkbcommon, fontconfig (for Vello)
- [OpenXR loader](https://github.com/KhronosGroup/OpenXR-SDK) (for XR session management — not needed for headless testing)
- An OpenXR runtime: [Monado](https://monado.freedesktop.org/) (open source), SteamVR, or Meta Quest Link (not needed for headless testing)

### Build the XR shim

```sh
just build-xr-shim           # Build Rust cdylib (headless mode works without OpenXR runtime)
just test-xr                  # Run all 37 XR shim integration tests (headless)
```

### Build and run shared examples in XR (headless)

```sh
just build-xr counter         # Build single example for XR
just build-xr-all             # Build all 4 examples for XR
just run-xr counter           # Build + run a single example (headless, exit code 0)
```

### Compile targets

```text
# Web (WASM):
mojo build examples/counter/main.mojo --target wasm64-wasi -I core/src -I web/src -I examples

# Desktop (native):
mojo build examples/counter/main.mojo -I core/src -I desktop/src -I xr/native/src -I examples

# XR (native + OpenXR):
mojo build examples/counter/main.mojo -D MOJO_TARGET_XR -I core/src -I xr/native/src -I desktop/src -I examples
```

> **Note:** All native builds include both `-I desktop/src` and `-I xr/native/src` because Mojo's `@parameter if` does not suppress import resolution in dead branches. The compile-time dispatch in `launch.mojo` imports from both `desktop.launcher` and `xr.launcher` — the linker only pulls in the active branch's code.

## Design Decisions

1. **Separate shim from desktop.** The XR shim (`xr/native/shim/`) is independent from the desktop shim (`desktop/shim/`). They share the same Blitz crate dependencies but have different responsibilities: the desktop shim manages a single Winit window; the XR shim manages an OpenXR session with multiple offscreen panels. Shared logic can be extracted into a common Rust crate if duplication becomes significant.

2. **Mutation protocol unchanged.** Each XR panel receives the same binary opcode stream as the web and desktop renderers. No protocol changes are needed for XR — the scene manager multiplexes mutation buffers to the correct panel.

3. **Headless mode for testing.** The shim provides `mxr_create_headless()` which creates a fully functional session without OpenXR or GPU. All DOM operations, raycasting, and event dispatch work in headless mode, enabling CI integration tests.

4. **Single-panel apps use GuiApp directly.** The XR launcher wraps any `GuiApp` in a default panel automatically. Multi-panel apps will use an `XRGuiApp` trait (Step 5.9, stretch goal) that receives the scene manager.

5. **Output-pointer FFI for large structs.** Functions returning C structs >16 bytes (which break x86_64 SysV ABI struct-return via DLHandle) use `_into()` variants that write to caller-provided output pointers: `mxr_poll_event_into()`, `mxr_raycast_panels_into()`, `mxr_get_pose_into()`.

6. **wgpu unifies GPU access.** Both the native path (Vello → wgpu → Vulkan/Metal) and the future WebXR path (Vello → wgpu → WebGPU) use the same GPU abstraction, minimizing platform-specific code.

7. **XR-specific UA stylesheet.** The XR launcher applies a user-agent stylesheet with larger fonts and dark background for headset legibility, separate from the desktop UA stylesheet.

## Roadmap

| Step | Description | Status |
|------|-------------|--------|
| 5.1 | Design the XR panel abstraction | ✅ Complete — `XRPanel`, `PanelConfig`, `XRScene`, `XREvent`, `RaycastHit`, layout helpers, C API header (~83 functions), Rust shim scaffold with headless DOM and integration tests |
| 5.2 | Build the OpenXR + Blitz Rust shim | 🔧 In progress — **real Blitz documents ✅** (HeadlessNode replaced with BaseDocument, 37 tests pass, Stylo+Taffy layout resolves). **Output-pointer FFI variants ✅** (`mxr_poll_event_into`, `mxr_raycast_panels_into`, `mxr_get_pose_into`). Remaining: Vello offscreen rendering, OpenXR session lifecycle |
| 5.3 | Mojo FFI bindings (`xr_blitz.mojo`) | ✅ Complete — `XRBlitz` struct (~70 methods), `XRMutationInterpreter` (all 18 opcodes), `XREvent`/`XRPose`/`XRRaycastHit` types, all constants. `poll_event()`, `raycast_panels()`, `get_pose()` fully functional via `_into()` output-pointer variants |
| 5.4 | XR scene manager and panel routing | ✅ Complete (single-panel) — `XRScene` provides panel registry, focus management, dirty tracking, raycasting, spatial layout helpers. Multi-panel routing deferred to Step 5.9 |
| 5.5 | `xr_launch[AppType: GuiApp]()` | ✅ Complete — creates headless/OpenXR session, allocates default panel from AppConfig, applies XR UA stylesheet, mounts app, enters XR frame loop |
| 5.6 | WebXR JS runtime | 🔧 In progress — `xr/web/runtime/` created: `XRSessionManager`, `XRPanelManager` + `XRPanel` (SVG foreignObject rasterization, WebGL textures, raycasting), `XRQuadRenderer` (WebGL2 stereo rendering), `XRInputHandler` (hover tracking, select→click, focus), `XRRuntime` (WASM loading, inline interpreter, "Enter VR" button, flat fallback). Remaining: E2E testing with real device/emulator |
| 5.7 | Wire `launch()` for XR targets | ✅ Complete — `launch()` dispatches: WASM → web, `-D MOJO_TARGET_XR` → `xr_launch`, native → `desktop_launch`. Added `is_xr_target()` compile-time detection |
| 5.8 | Verify shared examples in XR | ✅ Complete — all 4 examples build and run in headless mode (exit code 0). Fixed: `@parameter if` import resolution, cross-platform `performance_now()`, headless frame loop exit |
| 5.9 | Multi-panel XR API (stretch goal) | 🔮 Future |

## XR-Specific Risks

| Risk | Impact | Mitigation | Status |
|------|--------|------------|--------|
| OpenXR runtime unavailable | XR features fail without runtime | Runtime detection at startup; fall back to desktop Blitz; headless mode for testing | 🔧 Headless mode implemented; runtime detection pending |
| XR input latency | Raycasting → DOM event adds latency | Keep raycast math in Rust (shim-side); minimize FFI roundtrips | ✅ Rust-side raycasting implemented |
| Frame timing constraints | OpenXR requires strict frame pacing | Only re-render dirty panels; cache textures; use quad layers for reprojection | 🔧 Dirty tracking per-panel implemented |
| DOM-to-texture fidelity (WebXR) | Rendering DOM to WebGL texture may lose interactivity | SVG foreignObject rasterization implemented with fallback text renderer; evaluate OffscreenCanvas, html2canvas for higher fidelity | 🔧 In progress — initial approach in `xr/web/runtime/xr-panel.ts`; needs real-device validation |
| Mojo `@parameter if` import resolution | Dead branches resolve imports; all renderer `-I` paths needed | All native builds include all renderer include paths; documented | ✅ Workaround in place |

## See Also

- [Phase 5 detailed plan](../docs/plan/phase5-xr.md) — full step-by-step design with architecture diagrams
- [Architecture](../docs/plan/architecture.md) — platform abstraction layer and dependency graph
- [Renderers](../docs/plan/renderers.md) — comparison of all renderer strategies
- [Desktop renderer](../desktop/) — the Blitz desktop renderer that XR extends
- [Project plan](../PLAN.md) — overall roadmap and status dashboard