// Multi-View App — WebXR Entry Point
//
// Boots the multi-view app through the XR web runtime. Uses the shared
// launchXR() helper for convention-based WASM export discovery:
//   app_init, app_rebuild, app_flush, app_handle_event
//
// When WebXR is available, the app renders as a floating panel in VR/AR.
// When WebXR is unavailable, it falls back to flat DOM rendering —
// the panel container becomes a visible styled div in the page.
//
// The WASM binary is shared with the standard web target (web/build/out.wasm).

import { launchXR } from "../lib/xr-app.js";

launchXR({
	app: "app",
	wasm: new URL("../../../../web/build/out.wasm", import.meta.url),
	bufferCapacity: 65536,
});
