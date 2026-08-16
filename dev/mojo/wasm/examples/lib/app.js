// App Launcher — Generic boot sequence for mojo-wasm examples.
//
// Convention-based WASM export discovery: given an app name (e.g. "counter"),
// the launcher looks for exports named `{name}_init`, `{name}_rebuild`,
// `{name}_flush`, and optionally `{name}_handle_event` and `{name}_dispatch_string`.
//
// This abstraction captures the common boot sequence shared by all standard
// mojo-wasm apps (load WASM → init → interpreter → EventBridge → mount).
// App-specific post-boot wiring is supported via the `onBoot` callback.
//
// The goal is convergence: as more features move into WASM (e.g. keydown
// handlers, event delegation), the `onBoot` hooks shrink and eventually
// all apps use an identical main.js:
//
//   import { launch } from "../lib/app.js";
//   launch({ app: "myapp", wasm: new URL("../../web/build/out.wasm", import.meta.url) });
//
// Usage:
//
//   // Counter — zero-config launch
//   launch({
//     app: "counter",
//     wasm: new URL("../../web/build/out.wasm", import.meta.url),
//   });
//
//   // Todo — zero-config launch (Enter key handled in WASM via onkeydown_enter_custom)
//   launch({
//     app: "todo",
//     wasm: new URL("../../web/build/out.wasm", import.meta.url),
//   });
//
//   // Bench — near-zero-config launch (toolbar rendered from WASM, only needs large buffer)
//   launch({
//     app: "bench",
//     wasm: new URL("../../web/build/out.wasm", import.meta.url),
//     bufferCapacity: 8 * 1024 * 1024,
//   });

import {
	alignedAlloc,
	alignedFree,
	getMemory,
	loadWasm,
	scratchFreeAll,
} from "./env.js";
import { EventBridge } from "./events.js";
import { Interpreter } from "./interpreter.js";
import { writeStringStruct } from "./strings.js";

// ── Defaults ────────────────────────────────────────────────────────────────

/** Default mutation buffer capacity (64 KiB — generous for most apps). */
const DEFAULT_BUF_CAPACITY = 65536;

/** Default root element selector. */
const DEFAULT_ROOT_SELECTOR = "#root";

/** Default event type constant (click = 0). */
const EVT_CLICK = 0;

// ── Types (documented via JSDoc) ────────────────────────────────────────────

/**
 * @typedef {Object} LaunchOptions
 * @property {string}           app            - App name prefix for WASM export discovery.
 * @property {URL|string}       wasm           - URL to the .wasm file.
 * @property {string}           [root="#root"] - CSS selector for the mount-point element.
 * @property {number}           [bufferCapacity=65536] - Mutation buffer size in bytes.
 * @property {boolean}          [clearRoot=true] - Whether to clear the root element before mount.
 * @property {(handle: AppHandle) => void} [onBoot] - Callback after mount for app-specific wiring.
 */

/**
 * @typedef {Object} AppHandle
 * @property {Object}  fns     - WASM instance exports.
 * @property {bigint}  appPtr  - Pointer to the WASM-side app state.
 * @property {Object}  interp  - The DOM interpreter instance.
 * @property {bigint}  bufPtr  - Pointer to the mutation buffer in WASM memory.
 * @property {number}  bufferCapacity - Mutation buffer capacity in bytes.
 * @property {Element} rootEl  - The mount-point DOM element.
 * @property {() => void} flush - Flush pending WASM updates and apply mutations to the DOM.
 * @property {() => void} destroy - Destroy the app: free WASM resources, clear DOM, remove listeners.
 * @property {boolean} destroyed - Whether the app has been destroyed.
 */

// ── Launcher ────────────────────────────────────────────────────────────────

/**
 * Boot a mojo-wasm app with convention-based WASM export discovery.
 *
 * Given `app: "counter"`, discovers exports:
 *   - `counter_init() -> appPtr`            (required)
 *   - `counter_rebuild(appPtr, buf, cap)`   (required)
 *   - `counter_flush(appPtr, buf, cap)`     (required)
 *   - `counter_handle_event(appPtr, hid, evt)` (optional — enables EventBridge dispatch)
 *   - `counter_dispatch_string(appPtr, hid, evt, strPtr)` (optional — enables string dispatch)
 *
 * When `{app}_handle_event` is missing, the EventBridge is still created
 * (so NewEventListener mutations are processed and DOM listeners attached)
 * but the dispatch callback is a no-op.
 *
 * When `{app}_dispatch_string` exists, the EventBridge automatically
 * routes string-carrying events through the string dispatch path:
 *   - `input`/`change` events: extracts `event.target.value` → WASM SignalString
 *   - `keydown` events: extracts `event.key` → WASM key filtering
 *     (ACTION_KEY_ENTER_CUSTOM checks for "Enter"; accepted keys also
 *     dispatch through handle_event for app-level routing)
 *
 * This enables Dioxus-style two-way input binding and WASM-driven
 * keyboard shortcuts with zero app-specific JS.
 *
 * @param {LaunchOptions} options
 * @returns {Promise<AppHandle>} Resolves after mount (and onBoot if provided).
 */
export async function launch(options) {
	const {
		app: appName,
		wasm: wasmUrl,
		root: rootSelector = DEFAULT_ROOT_SELECTOR,
		bufferCapacity = DEFAULT_BUF_CAPACITY,
		clearRoot = true,
		onBoot = null,
	} = options;

	const rootEl = document.querySelector(rootSelector);
	if (!rootEl) {
		throw new Error(
			`launch("${appName}"): root element "${rootSelector}" not found`,
		);
	}

	try {
		// 1. Load WASM and discover exports by naming convention
		const fns = await loadWasm(wasmUrl);

		const initFn = fns[`${appName}_init`];
		const rebuildFn = fns[`${appName}_rebuild`];
		const flushFn = fns[`${appName}_flush`];
		const destroyFn = fns[`${appName}_destroy`]; // optional but expected
		const handleEventFn = fns[`${appName}_handle_event`]; // optional
		const dispatchStringFn = fns[`${appName}_dispatch_string`]; // optional
		const navigateFn = fns[`${appName}_navigate`]; // optional — enables routing

		if (!initFn) {
			throw new Error(`WASM export "${appName}_init" not found`);
		}
		if (!rebuildFn) {
			throw new Error(`WASM export "${appName}_rebuild" not found`);
		}
		if (!flushFn) {
			throw new Error(`WASM export "${appName}_flush" not found`);
		}

		// 2. Initialize WASM-side app
		const appPtr = initFn();

		// 3. Prepare DOM — clear loading indicator and create interpreter
		if (clearRoot) {
			rootEl.innerHTML = "";
		}
		const interp = new Interpreter(rootEl, new Map());
		const bufPtr = alignedAlloc(8n, BigInt(bufferCapacity));

		// 4. Flush helper — reusable by EventBridge and onBoot hook
		function flush() {
			const len = flushFn(appPtr, bufPtr, bufferCapacity);
			if (len > 0) {
				const mem = getMemory();
				interp.applyMutations(mem.buffer, Number(bufPtr), len);
			}
			// Free transient scratch allocations (e.g. writeStringStruct)
			// now that WASM has consumed the data and mutations are applied.
			scratchFreeAll();
		}

		// 5. Wire events via EventBridge with smart dispatch
		//    - If `{app}_handle_event` is missing, EventBridge dispatch is a no-op
		//      (DOM listeners are still attached so NewEventListener mutations work;
		//      apps like bench wire their own handlers via onBoot)
		//    - If `{app}_dispatch_string` exists:
		//      · input/change → string dispatch (event.target.value → WASM SignalString)
		//      · keydown → string dispatch (event.key → WASM key filtering);
		//        if accepted (e.g. Enter), also call handle_event for app routing
		//    - All other events use normal numeric dispatch
		new EventBridge(interp, (handlerId, eventName, domEvent) => {
			if (
				dispatchStringFn &&
				(eventName === "input" || eventName === "change") &&
				domEvent?.target?.value !== undefined
			) {
				const strPtr = writeStringStruct(domEvent.target.value);
				dispatchStringFn(appPtr, handlerId, EVT_CLICK, strPtr);
			} else if (
				dispatchStringFn &&
				eventName === "keydown" &&
				domEvent?.key !== undefined
			) {
				// Send the key name through string dispatch for WASM-side filtering.
				// ACTION_KEY_ENTER_CUSTOM checks value == "Enter" and marks scope dirty.
				// If accepted (returns 1), also call handle_event for app-level routing.
				const strPtr = writeStringStruct(domEvent.key);
				const accepted = dispatchStringFn(appPtr, handlerId, EVT_CLICK, strPtr);
				if (accepted && handleEventFn) {
					handleEventFn(appPtr, handlerId, EVT_CLICK);
				}
			} else if (handleEventFn) {
				handleEventFn(appPtr, handlerId, EVT_CLICK);
			}
			flush();
		});

		// 6. Initial mount (RegisterTemplate + LoadTemplate + events in one pass)
		const mountLen = rebuildFn(appPtr, bufPtr, bufferCapacity);
		if (mountLen > 0) {
			const mem = getMemory();
			interp.applyMutations(mem.buffer, Number(bufPtr), mountLen);
		}

		// 7. Build app handle for onBoot callback and return value
		const handle = {
			fns,
			appPtr,
			interp,
			bufPtr,
			bufferCapacity,
			rootEl,
			flush,
			destroyed: false,

			/**
			 * Destroy the app: free WASM resources, clear DOM, prevent reuse.
			 *
			 * 1. Free the mutation buffer (JS-side allocator).
			 * 2. Destroy the WASM-side app state (if export exists).
			 * 3. Clear the root element (removes all rendered DOM + listeners).
			 * 4. Null out handle fields to prevent use-after-destroy.
			 *
			 * Idempotent — calling destroy() twice is a safe no-op.
			 */
			destroy() {
				if (handle.destroyed) return;
				handle.destroyed = true;

				// Free the mutation buffer in the JS-side allocator
				alignedFree(handle.bufPtr);

				// Destroy the WASM-side app state
				if (destroyFn) {
					destroyFn(handle.appPtr);
				}

				// Clear rendered DOM (also removes event listeners on child elements)
				rootEl.replaceChildren();

				// Null out fields to prevent use-after-destroy
				handle.appPtr = 0n;
				handle.bufPtr = 0n;
				handle.interp = null;
			},
		};

		// 8. Client-side routing (Phase 30)
		//    Auto-detected when `{app}_navigate` export exists.
		//    - popstate listener → calls navigate + flush on browser back/forward
		//    - <a> click interception → prevents default, pushState, navigate + flush
		let popstateHandler = null;
		let linkClickHandler = null;

		if (navigateFn) {
			// Navigate helper: call WASM navigate + flush + pushState
			function routerNavigate(path, pushHistory = true) {
				if (handle.destroyed) return;
				const pathPtr = writeStringStruct(path);
				const result = navigateFn(handle.appPtr, pathPtr);
				if (result) {
					flush(); // flush calls scratchFreeAll — frees pathPtr
					if (pushHistory) {
						try {
							history.pushState(null, "", path);
						} catch (_) {
							/* non-browser */
						}
					}
				} else {
					// No flush happened — free the scratch string manually
					scratchFreeAll();
				}
			}

			// popstate → browser back/forward
			popstateHandler = () => {
				const path = location.pathname || "/";
				routerNavigate(path, false); // don't pushState on popstate
			};
			try {
				window.addEventListener("popstate", popstateHandler);
			} catch (_) {
				/* non-browser */
			}

			// <a> click interception on root element (delegation)
			linkClickHandler = (e) => {
				// Walk up from target to find <a> with href
				let el = e.target;
				while (el && el !== rootEl) {
					if (el.tagName === "A" && el.getAttribute("href")) {
						const href = el.getAttribute("href");
						// Only intercept same-origin relative paths
						if (href.startsWith("/")) {
							e.preventDefault();
							routerNavigate(href, true);
							return;
						}
					}
					el = el.parentElement;
				}
			};
			rootEl.addEventListener("click", linkClickHandler);

			// Expose navigate on the handle for programmatic use
			handle.navigate = routerNavigate;

			// Navigate to the current URL path on initial load
			const initialPath =
				(typeof location !== "undefined" && location.pathname) || "/";
			// The WASM side already navigated to "/" in __init__, so only
			// navigate if the browser URL differs from the default route.
			if (initialPath !== "/") {
				routerNavigate(initialPath, false);
			}
		}

		// Augment destroy to clean up routing listeners
		const originalDestroy = handle.destroy;
		handle.destroy = () => {
			if (popstateHandler) {
				try {
					window.removeEventListener("popstate", popstateHandler);
				} catch (_) {}
			}
			if (linkClickHandler) {
				rootEl.removeEventListener("click", linkClickHandler);
			}
			originalDestroy.call(handle);
		};

		// 9. App-specific post-boot wiring
		if (onBoot) onBoot(handle);

		console.log(`🔥 Mojo ${appName} app running!`);
		return handle;
	} catch (err) {
		console.error(`Failed to boot ${appName}:`, err);
		rootEl.innerHTML = `<p class="error">Failed to load: ${err.message}</p>`;
		throw err;
	}
}
