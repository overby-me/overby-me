// Counter App — WebXR Entry Point
//
// Boots the counter app through the XR web runtime. Uses the shared
// launchXR() helper for convention-based WASM export discovery:
//   counter_init, counter_rebuild, counter_flush, counter_handle_event
//
// When WebXR is available, the app renders as a floating panel in VR/AR.
// When WebXR is unavailable, it falls back to flat DOM rendering —
// the panel container becomes a visible styled div in the page.
//
// The WASM binary is shared with the standard web target (web/build/out.wasm).

import { launchXR } from "../lib/xr-app.js";

launchXR({
	app: "counter",
	wasm: new URL("../../../../web/build/out.wasm", import.meta.url),
	bufferCapacity: 16384,
});
