// Multi-View App — Browser Entry Point
//
// Uses shared launch() from examples/lib/ for convention-based boot.
// All WASM exports are discovered automatically by the "mv" prefix:
//   mv_init, mv_rebuild, mv_flush, mv_handle_event, mv_navigate
//
// The presence of mv_navigate enables automatic client-side routing:
//   - popstate listener for browser back/forward
//   - <a> click interception for in-app navigation
//
// Phase 30: Zero app-specific JS — routing is handled entirely by the
// launch() routing integration + WASM-side Router struct.

import { launch } from "../lib/app.js";

launch({
	app: "mv",
	wasm: new URL("../../web/build/out.wasm", import.meta.url),
	bufferCapacity: 65536,
});
