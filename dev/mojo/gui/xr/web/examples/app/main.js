// Multi-View App — WebXR Entry Point
//
// Boots the multi-view app through the XR web runtime. Uses the shared
// launchXR() helper for convention-based WASM export discovery:
//   mv_init, mv_rebuild, mv_flush, mv_handle_event
//
// The prefix is "mv", the one the module actually exports and the one
// web/examples/app/main.js asks for. Naming it "app" after the directory
// found no exports at all and the panel stayed empty and hidden.
//
// When WebXR is available, the app renders as a floating panel in VR/AR.
// When WebXR is unavailable, it falls back to flat DOM rendering —
// the panel container becomes a visible styled div in the page.
//
// The WASM binary is shared with the standard web target (web/build/out.wasm).

import { launchXR } from "../lib/xr-app.js";

launchXR({
	app: "mv",
	wasm: new URL("../../../../web/build/out.wasm", import.meta.url),
	bufferCapacity: 65536,
});
