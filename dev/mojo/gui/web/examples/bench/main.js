// Benchmark App — Browser Entry Point
//
// Uses shared launch() from examples/lib/ for convention-based boot.
// All WASM exports are discovered automatically by the "bench" prefix:
//   bench_init, bench_rebuild, bench_flush, bench_handle_event
//
// Phase 24.2: The entire app shell (heading, toolbar buttons, status bar,
// table structure) is now rendered from WASM via register_view() with
// onclick_custom() handlers.  EventBridge dispatches all button and row
// clicks directly — zero app-specific JS needed.
//
// The only bench-specific config is bufferCapacity (8 MB for large row sets).
//
// ── Phase 24 — Zero app-specific JS convergence ─────────────────────
//
// DONE(P24.3): performance.now() WASM import for timing.
//   Added `performance_now() -> Float64` import to env.js and a
//   corresponding Mojo FFI declaration via external_call.  Each toolbar
//   operation in handle_event() is wrapped with before/after
//   performance_now() calls; elapsed time is formatted to 1 decimal
//   place and stored in status fields, emitted as dyn_text nodes on flush.
//   Zero JS-side timing code needed.
//
// DONE(P24.4): Status bar as WASM template with fine-grained dynamic text.
//   The status bar div uses 3 separate dyn_text nodes for operation name
//   (dyn_text[0]), timing (dyn_text[1]), and row count (dyn_text[2]).
//   Only changed text nodes receive SetText mutations on flush, enabling
//   finer-grained DOM updates.  bench/main.js is now structurally
//   identical to counter/main.js and todo/main.js — only bufferCapacity
//   override remains as bench-specific config.

import { launch } from "../lib/app.js";

const BUF_CAPACITY = 8 * 1024 * 1024; // 8 MB mutation buffer

launch({
	app: "bench",
	wasm: new URL("../../build/out.wasm", import.meta.url),
	bufferCapacity: BUF_CAPACITY,
});
