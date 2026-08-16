// XR App Launcher — Generic boot sequence for mojo-gui XR web examples.
//
// Convention-based WASM export discovery via XRRuntime.createAppPanel().
// Given an app name (e.g. "counter"), the XR runtime discovers exports:
//   counter_init, counter_rebuild, counter_flush, counter_handle_event
//
// The launcher initializes the XR runtime, creates an app panel, and
// starts in either WebXR immersive mode or flat fallback depending on
// browser capabilities.
//
// Usage:
//
//   import { launchXR } from "../lib/xr-app.js";
//
//   launchXR({
//     app: "counter",
//     wasm: new URL("../../../../web/build/out.wasm", import.meta.url),
//   });

import { XRRuntime } from "../../runtime/xr-runtime.ts";
import { defaultXRRuntimeConfig } from "../../runtime/xr-types.ts";

// ── Defaults ────────────────────────────────────────────────────────────────

/** Default mutation buffer capacity (64 KiB). */
const DEFAULT_BUF_CAPACITY = 65536;

/** Default panel position: 1m in front of user at eye height. */
const DEFAULT_POSITION = { x: 0, y: 1.4, z: -1 };

// ── Types (documented via JSDoc) ────────────────────────────────────────────

/**
 * @typedef {Object} XRLaunchOptions
 * @property {string}           app            - App name prefix for WASM export discovery.
 * @property {URL|string}       wasm           - URL to the .wasm file.
 * @property {number}           [bufferCapacity=65536] - Mutation buffer size in bytes.
 * @property {Object}           [panelConfig]  - Partial PanelConfig overrides.
 * @property {Object}           [position]     - Initial panel position {x, y, z}.
 * @property {Object}           [runtimeConfig] - Partial XRRuntimeConfig overrides.
 * @property {(handle: Object) => void} [onBoot] - Callback after panel creation.
 */

// ── Status display helpers ──────────────────────────────────────────────────

/**
 * Show a status message in the #xr-status element (if it exists).
 *
 * @param {string} message - Status text to display.
 * @param {"info"|"error"|"success"} [level="info"] - Message level for styling.
 */
function showStatus(message, level = "info") {
	const el = document.getElementById("xr-status");
	if (!el) return;

	el.textContent = message;
	el.className = `xr-status xr-status-${level}`;
}

// ── Launcher ────────────────────────────────────────────────────────────────

/**
 * Boot a mojo-gui app in the XR web runtime.
 *
 * Initializes the XRRuntime, creates an app panel connected to the WASM
 * module, and starts rendering. When WebXR is available, the user sees
 * an "Enter VR" button. When WebXR is unavailable, the app falls back
 * to flat DOM rendering (panels visible as styled divs).
 *
 * @param {XRLaunchOptions} options
 * @returns {Promise<{runtime: XRRuntime, handle: Object}>}
 */
export async function launchXR(options) {
	const {
		app: appName,
		wasm: wasmUrl,
		bufferCapacity = DEFAULT_BUF_CAPACITY,
		panelConfig = {},
		position = DEFAULT_POSITION,
		runtimeConfig = {},
		onBoot = null,
	} = options;

	showStatus("Initializing XR runtime…");

	try {
		// 1. Create and initialize the XR runtime
		const runtime = new XRRuntime(document);

		const config = {
			...defaultXRRuntimeConfig(),
			...runtimeConfig,
			// Always enable flat fallback for graceful degradation
			fallbackToFlat: true,
		};

		const xrAvailable = await runtime.initialize(config);

		if (xrAvailable) {
			showStatus("WebXR available — click 'Enter VR' to start", "success");
		} else {
			showStatus("WebXR not available — using flat fallback", "info");
		}

		// 2. Create an app panel connected to the WASM module
		const handle = await runtime.createAppPanel({
			wasmUrl: wasmUrl,
			appName: appName,
			panelConfig: panelConfig,
			position: position,
			bufferCapacity: bufferCapacity,
		});

		// 3. Start rendering (XR session or flat fallback)
		await runtime.start();

		// 4. Listen for runtime events
		runtime.addEventListener((event) => {
			switch (event.type) {
				case "session-started":
					showStatus("XR session active", "success");
					break;
				case "session-ended":
					showStatus("XR session ended — click 'Enter VR' to restart", "info");
					break;
				case "fallback-to-flat":
					showStatus("Flat fallback mode — panels visible as DOM", "info");
					break;
				case "error":
					showStatus(`Error: ${event.message || "unknown"}`, "error");
					break;
			}
		});

		const result = { runtime, handle };

		// 5. App-specific post-boot wiring
		if (onBoot) onBoot(result);

		console.log(`🔥 Mojo ${appName} XR app running!`);
		return result;
	} catch (err) {
		console.error(`Failed to boot XR ${appName}:`, err);
		showStatus(`Failed to load: ${err.message}`, "error");

		const root = document.getElementById("root");
		if (root) {
			root.innerHTML = `<p class="error">Failed to load XR app: ${err.message}</p>`;
		}
		throw err;
	}
}
