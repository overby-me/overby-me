// Counter App — Browser Entry Point
//
// Uses shared launch() from examples/lib/ for convention-based boot.
// All WASM exports are discovered automatically by the "counter" prefix:
//   counter_init, counter_rebuild, counter_flush, counter_handle_event
//
// This is the minimal mojo-wasm app entry point — zero app-specific JS needed.

import { launch } from "../lib/app.js";

launch({
	app: "counter",
	wasm: new URL("../../web/build/out.wasm", import.meta.url),
	bufferCapacity: 16384,
});
