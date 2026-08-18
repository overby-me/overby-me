# Risks, Estimated Effort & Open Questions

> See also: [architecture](architecture.md), [PLAN.md](../../PLAN.md).

---

## Risks & Mitigations

| Risk | Impact | Mitigation | Status |
|------|--------|------------|--------|
| Mojo package system immaturity | Can't cleanly separate into packages | Mono-repo with path-based imports (`-I` flags) | ✅ Resolved — mono-repo with `-I ../core/src -I ../examples` works |
| `MutExternalOrigin` tied to WASM | Core won't compile natively | Audit and abstract the origin parameter; conditionally compile | ✅ Resolved — `MutExternalOrigin` works for both WASM and native heap buffers |
| Blitz C shim complexity | Desktop renderer takes too long | Start with webview approach as intermediate step; upgrade to Blitz later | ✅ Resolved — C shim, Mojo FFI bindings, mutation interpreter, launcher, Winit event loop, Vello rendering all implemented; cdylib builds cleanly |
| Blitz pre-alpha stability | Rendering bugs, missing CSS features | Track Blitz main branch; contribute upstream fixes; keep webview as fallback | ✅ Mitigated — shim pins to Blitz v0.2.0 (rev `2f83df96`); v0.2.0 provides good CSS coverage via Stylo |
| Blitz Rust build dependency | Complex build toolchain | Pre-build the `cdylib` and distribute as a shared library; Nix flake can automate the Rust build | ✅ Resolved — `cargo build --release` succeeds; Cargo.lock generated (607 packages); Nix derivation (`shim/default.nix`) automates build with GPU/windowing deps |
| Import path breakage | Massive search-and-replace | Script the migration; grep-verify all imports | ✅ Resolved — all imports updated |
| Test suite fragmentation | Tests break across projects | Phase 1 must keep all Mojo tests green; Phase 2 must keep all JS tests green | ✅ Resolved — all tests pass (3,375 JS + 52 Mojo suites) |
| Platform abstraction too leaky | Shared examples break on some targets | Use the cross-target test matrix as a gate; treat cross-target failures as framework bugs | ✅ Mitigated — `GuiApp` trait + `launch()` complete (web + desktop + XR dispatch); all 4 shared examples verified on all 3 targets; CI pending (S-1) |
| `launch()` compile-time dispatch limitations | Mojo may lack the metaprogramming for clean target dispatch | `GuiApp` trait + `@parameter if is_wasm_target()` provides clean dispatch; if trait parametric methods don't work, fall back to conditional imports | ✅ Resolved — `launch[AppType: GuiApp]()` works with `@parameter if`; dispatches to web, desktop, and XR targets |
| Mojo trait limitations for `GuiApp` | Trait may not support parametric methods or associated types needed for generic `@export` wrappers | Start with concrete struct aliases (`alias CurrentApp = CounterApp`); upgrade to full trait generics when Mojo supports it | ✅ Resolved — parametric helpers `gui_app_init[T: GuiApp]()` work; `@export` wrappers call them with concrete types |
| WebKitGTK Linux-only | Desktop renderer not cross-platform | Webview is an intermediate step; Blitz (Phase 4) will provide cross-platform support via Winit | ✅ Resolved — replaced by Blitz native renderer (Phase 4); Winit supports Linux, macOS, and Windows |
| Base64 IPC overhead | ~33% mutation size increase for desktop | Acceptable for now; investigate shared memory or binary transfer for optimization | ✅ Resolved — Blitz desktop renderer eliminates IPC entirely; mutations applied in-process via C FFI |
| Desktop event loop busy-wait | High CPU when idle | Implemented blocking `mwv_step(blocking=True)` when no events/dirty scopes | ✅ Resolved |
| Native target module-level `var` | Global `var` declarations in imported packages not supported on native target | Wrap in struct or use function-local static; to be fixed when native compilation is tested | ✅ Resolved — avoided in current design; config passed as arguments, not globals |
| Mojo `@parameter if` import resolution | Dead branches still trigger import resolution; adding a renderer backend to `launch()` requires updating ALL native build commands | Include all renderer `-I` paths in every native build; documented in justfile and Architecture Decisions | ✅ Mitigated — workaround in place since Step 5.8; will re-evaluate when Mojo improves `@parameter if` semantics |
| Mojo WASM runtime import drift | New Mojo versions may add new WASM imports (e.g. `clock_gettime` in 26.1.0) that break the test harness | Pin Mojo version; update both `wasm_harness.mojo` and `web/runtime/env.ts` when upgrading; check `wasm-objdump -j Import` after Mojo upgrades | ✅ Mitigated — `clock_gettime` added for 26.1.0 |
| OpenXR runtime availability | XR features fail on systems without OpenXR runtime | Runtime detection: check for OpenXR loader at startup; fall back to desktop Blitz renderer if unavailable | 🔧 In progress — headless mode (`mxr_create_headless`) implemented for testing without runtime; `xr_launch` uses headless by default; runtime detection pending (Step 5.2 remaining) |
| DOM-to-texture fidelity (WebXR) | Rendering DOM to WebGL texture may lose interactivity or fidelity | Evaluate multiple approaches: OffscreenCanvas, html2canvas, CSS 3D transforms in DOM overlay; benchmark quality vs. performance | 🔲 Future — Phase 5, Step 5.6 |
| XR input latency | Raycasting → DOM event translation adds latency to controller input | Keep raycast math in the shim (Rust/native) or GPU (WebXR); minimize JS/Mojo roundtrips for input dispatch | ✅ Mitigated — Rust-side raycasting implemented in XR shim; panel hit testing runs natively with no FFI roundtrips |
| Multi-panel mutation routing | Multiple panels need independent mutation streams; current protocol assumes single document | Each panel gets its own mutation buffer and `GuiApp` instance; the XR scene manager multiplexes; no protocol changes needed | ✅ Architecture proven — single-panel routing works end-to-end; multi-panel routing deferred to Step 5.9 |
| XR frame timing constraints | OpenXR requires strict frame pacing; DOM re-render may exceed frame budget | Render panels asynchronously; only re-render dirty panels; cache textures for clean panels; use OpenXR quad layers for compositor-side reprojection | 🔧 In progress — dirty tracking per-panel implemented; idle frame detection working; Vello offscreen rendering pending |

---

## Estimated Effort

| Phase | Effort | Description | Status |
|-------|--------|-------------|--------|
| Phase 1 | 2–3 days | File moves, import path updates, platform abstraction layer, shared examples setup, verify compilation + tests | ✅ Complete |
| Phase 2 | 1–2 days | Move web runtime, `WebApp` trait impl, shared example web builds, verify browser tests | ✅ Complete |
| Phase 3 (infra) | 1–2 weeks | GTK4/WebKitGTK C shim, Mojo FFI, `DesktopApp`, JS runtime for webview, counter example, Nix integration | ✅ Complete |
| Phase 3.9 | 3–5 days | `GuiApp` trait, generic desktop event loop, `launch()` dispatch, refactor app structs, genericize `@export` wrappers, shared `main.mojo` entry points, cross-target CI | ✅ Complete |
| Phase 4 | 2–4 weeks | Blitz C shim (Rust cdylib), Mojo-side mutation interpreter, Winit event loop, Vello rendering, cross-platform testing | ✅ Complete |
| Phase 5 | 4–8 weeks | XR renderer: OpenXR native shim, WebXR JS runtime, panel abstraction, XR input → DOM event bridging, `launch()` XR dispatch | 🔧 ~60% complete — Steps 5.1–5.5, 5.7–5.8 done; remaining: Vello offscreen + OpenXR session (5.2), WebXR (5.6), multi-panel (5.9) |
| Phase 6 | 2–3 weeks | `mojo-web` MVP: handle table, DOM, fetch, timers, storage | 🔲 Future |

---

## Open Questions

1. **~~Mono-repo vs. multi-repo?~~** — ✅ Resolved: Mono-repo. `mojo-gui/` is the workspace root containing `core/`, `web/`, `desktop/`, `xr/`, and `examples/`. Path-based imports (`-I ../core/src -I ../examples`) work well. `mojo-web` will live alongside as a sibling.

2. **~~Should `html/` stay in `mojo-gui/core` or become a separate `mojo-gui/html` package?~~** — ✅ Resolved: Keep in `core`. The HTML/CSS/DOM model is universal across all renderers: web uses real DOM, desktop uses Blitz DOM, XR uses Blitz DOM per-panel (native) or real DOM per-panel (WebXR). All renderers consume the same DOM-oriented mutation protocol, so the HTML vocabulary stays in core.

3. **~~How to handle the `@export` boilerplate in `main.mojo`?~~** — ✅ Resolved by Phase 3.9 design: the `GuiApp` trait provides a uniform lifecycle interface. `@export` wrappers become generic over `GuiApp` — one set of wrappers works for every app. Each example builds with a compile-time alias (`alias CurrentApp = CounterApp`). The ~6,730 lines of per-app wrappers collapse to a small generic set.

4. **~~Blitz C shim API granularity?~~** — ✅ Resolved: The shim (`mojo_blitz.h`) exposes ~45 functions covering lifecycle, DOM operations, templates, events, stack operations, ID mapping, and debug. The API follows the same polling-based, no-callback, flat C ABI pattern as the webview shim. Blitz's `BaseDocument` is accessed via an opaque `BlitzContext` pointer. The shim maintains its own ID mapping (mojo element IDs ↔ Blitz slab node IDs) and interpreter stack.

5. **~~Should the Mojo-side mutation interpreter share code with the JS `Interpreter`?~~** — ✅ Resolved: The Mojo `MutationInterpreter` (`desktop/src/desktop/renderer.mojo`) and JS `Interpreter` (`web/runtime/interpreter.ts` / `desktop/runtime/desktop-runtime.js`) are parallel implementations of the same stack machine, reading the same binary opcode format. They cannot share code (different languages), but they share the opcode definitions and wire format specification from `core/src/bridge/protocol.mojo`. Correctness is verified by running the same shared examples on both renderers.

6. **Should `mojo-web` reuse `mojo-gui/web`'s existing JS runtime code?** — Partially. `memory.ts`, `env.ts`, and `strings.ts` solve the same WASM↔JS interop problems. Extract a shared `mojo-wasm-runtime` base, or let `mojo-web` depend on just those modules.

7. **Should `mojo-gui/web` eventually use `mojo-web` for its JS runtime?** — Possibly for non-rendering parts (e.g., the `EventBridge` could use `mojo-web`'s DOM bindings). The mutation protocol interpreter should stay as-is for performance (batched application vs. per-call overhead).

8. **~~Blitz version pinning?~~** — ✅ Resolved: Pinned to Blitz v0.2.0 release (rev `2f83df96220561316611ecf857e20cd1feed8ca0`). All Blitz git dependencies use this exact commit. The `Cargo.lock` file locks all transitive dependencies for reproducible builds. Markup5ever types are re-exported from `blitz_dom` to avoid version mismatch issues (Blitz v0.2.0 uses markup5ever 0.35.0 internally). Update the rev deliberately when a new Blitz release is available.

9. **CSS support scope?** — Blitz supports modern CSS (flexbox, grid, selectors, variables, media queries) via Stylo, but not all CSS features are implemented yet. Document which CSS features are supported and test the Blitz desktop renderer against the same shared examples as the web and webview renderers.

10. **~~Fallback for `launch()` compile-time dispatch?~~** — ✅ Resolved by Phase 3.9 + Phase 4 + Phase 5 design: `launch[AppType: GuiApp](config)` uses `@parameter if is_wasm_target()` / `elif is_xr_target()` / `else` to dispatch. For WASM, the JS runtime drives the loop via `@export` wrappers generic over `GuiApp`. For XR, `xr_launch[AppType](config)` runs the XR frame loop. For native, `desktop_launch[AppType](config)` runs the Blitz event loop. No per-renderer entry-point files needed — every example has a single `fn main()` calling `launch()`.

11. **How to handle web-only features in shared examples?** — Examples that need web-specific APIs (e.g., `fetch`, `localStorage`) should use compile-time feature gates: `@parameter if is_wasm_target(): ...`. Platform-specific timing is handled the same way: `performance_now()` uses `external_call` on WASM and `time.perf_counter_ns()` on native, selected at compile time inside the shared source file. XR-specific features (panel placement, hand tracking) use `@parameter if is_xr_target(): ...`.

12. **~~Desktop webview cross-platform support?~~** — ✅ Resolved: The webview approach was removed in favor of the Blitz-based native renderer (Phase 4). Blitz uses Winit, which supports Linux, macOS, and Windows natively. No need for platform-specific webview shims.

13. **~~Desktop example sharing vs. duplication?~~** — ✅ Resolved by Phase 3.9 design: **no duplication, ever.** Examples live in `examples/` and implement the `GuiApp` trait. The `launch()` function and generic event loops (`desktop_launch`, `xr_launch`, `@export` wrappers) drive them on every target. Per-renderer example directories are an anti-pattern — if an example doesn't compile on a target, it's a framework bug.

14. **~~Base64 IPC optimization?~~** — ✅ Resolved: The Blitz desktop renderer eliminates IPC entirely. Mutations are applied in-process via the Mojo `MutationInterpreter` → Blitz C FFI calls. No base64 encoding, no JS eval, no webview.

15. **Can Mojo traits be parametric enough for `GuiApp`?** — The `GuiApp` trait needs to work as a compile-time parameter to `launch[]`, `desktop_launch[]`, `xr_launch[]`, and the `@export` wrapper pattern. If Mojo's trait system doesn't support this (e.g., no parametric methods on trait-constrained types), the fallback is concrete `alias CurrentApp = CounterApp` per build, with a shared `@export` module that references the alias. This is still a single source file per example — the alias is a build system concern, not an app authoring concern.

16. **~~Should the XR native shim share code with the desktop Blitz shim?~~** — ✅ Resolved (Step 5.1): Separate `xr/native/shim/` created. Both reuse the same Blitz crates (blitz-dom, Stylo, Taffy, Vello) but serve different purposes — the desktop shim targets a single Winit window; the XR shim targets an OpenXR session with multiple offscreen panels. Code duplication is minimal and manageable.

17. **How to handle DOM-to-texture rendering for WebXR?** — Several approaches exist: (a) OffscreenCanvas with `drawImage()` from a DOM-rendered element, (b) `html2canvas` or similar rasterization libraries, (c) WebXR DOM Overlay API (limited to a single flat layer, not spatially placed), (d) render mutation protocol directly to a WebGL/WebGPU canvas using a custom 2D renderer (bypassing the DOM entirely on the WebXR path). Evaluate fidelity, performance, and interactivity tradeoffs. Approach (d) would be the most consistent with the native path (Vello-like rendering to a texture) but requires a JS/WASM 2D rendering engine. *Deferred to Step 5.6.*

18. **~~Should single-panel XR apps use `GuiApp` directly or always go through `XRGuiApp`?~~** — ✅ Resolved (Step 5.5): Single-panel apps use the existing `GuiApp` trait unchanged. `xr_launch` wraps them in a default panel automatically. `XRGuiApp` (Step 5.9) is only needed for apps that explicitly manage multiple panels or need XR-specific features (hand tracking, spatial anchors). This preserves the "write once, run everywhere" principle — existing apps get XR support for free.

19. **What OpenXR extensions are required?** — The MVP needs: `XR_KHR_opengl_enable` or `XR_KHR_vulkan_enable` (GPU interop), `XR_KHR_composition_layer_quad` (panel placement). Nice to have: `XR_EXT_hand_tracking` (hand input), `XR_FB_passthrough` (AR), `XR_EXTX_overlay` (overlay apps). The shim detects available extensions at runtime and exposes capability flags to Mojo (`has_extension()`, `has_hand_tracking()`, `has_passthrough()` — implemented in Step 5.3). *Actual OpenXR integration deferred to Step 5.2 remaining work.*