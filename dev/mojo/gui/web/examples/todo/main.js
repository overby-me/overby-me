// Todo App — Browser Entry Point
//
// Uses shared launch() from examples/lib/ for convention-based boot.
// All WASM exports are discovered automatically by the "todo" prefix:
//   todo_init, todo_rebuild, todo_flush, todo_handle_event, todo_dispatch_string
//
// The presence of todo_dispatch_string enables automatic string dispatch
// for input/change events (Dioxus-style two-way binding via oninput_set_string)
// and keydown events (WASM-driven Enter key handling via onkeydown_enter_custom).
//
// Phase 22: Zero app-specific JS — Enter key is handled entirely in WASM.
// This main.js is now identical in structure to the counter main.js.

import { launch } from "../lib/app.js";

launch({
	app: "todo",
	wasm: new URL("../../build/out.wasm", import.meta.url),
});
