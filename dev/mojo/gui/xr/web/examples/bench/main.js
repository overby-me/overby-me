// Benchmark App — WebXR Entry Point
//
// Boots the benchmark app through the XR web runtime. Uses the shared
// launchXR() helper for convention-based WASM export discovery:
//   bench_init, bench_rebuild, bench_flush, bench_handle_event
//
// When WebXR is available, the app renders as a floating panel in VR/AR.
// When WebXR is unavailable, it falls back to flat DOM rendering —
// the panel container becomes a visible styled div in the page.
//
// The WASM binary is shared with the standard web target (web/build/out.wasm).
// The benchmark app uses a larger buffer capacity for its many DOM nodes.

import { launchXR } from "../lib/xr-app.js";

launchXR({
	app: "bench",
	wasm: new URL("../../../../web/build/out.wasm", import.meta.url),
	bufferCapacity: 8 * 1024 * 1024,
});
