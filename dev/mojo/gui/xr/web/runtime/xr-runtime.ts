// XR Runtime — Main entry point for the WebXR browser renderer.
//
// Ties together all WebXR subsystems:
//   - XRSessionManager — WebXR session lifecycle and frame loop
//   - XRPanelManager  — Panel DOM containers, texture capture, spatial layout
//   - XRQuadRenderer  — WebGL2 textured quad drawing
//   - XRInputHandler  — Controller/hand raycasting → DOM pointer events
//
// The XR runtime extends the existing web renderer by:
//   1. Reusing the same binary mutation protocol (unchanged)
//   2. Reusing the same Interpreter for DOM operations
//   3. Reusing the same EventBridge pattern for event dispatch
//   4. Adding spatial placement (panels as quads in 3D)
//   5. Adding XR input (controller rays → DOM pointer events)
//   6. Replacing requestAnimationFrame with XRSession.requestAnimationFrame
//
// For single-panel apps, the runtime wraps the existing GuiApp in a default
// panel automatically — no app changes needed. The app thinks it's running
// in a normal web page; the runtime captures its DOM output to a texture
// and places it in the XR scene.
//
// Graceful fallback: when WebXR is unavailable, the runtime falls back to
// flat (non-XR) rendering. The app still works as a normal web page.
//
// Usage:
//
//   import { XRRuntime } from "./xr-runtime.ts";
//   import { defaultXRRuntimeConfig } from "./xr-types.ts";
//
//   const runtime = new XRRuntime(document);
//   await runtime.initialize(defaultXRRuntimeConfig());
//
//   // Create a panel and wire a WASM app to it
//   const panel = runtime.createAppPanel({
//     wasmUrl: new URL("./app.wasm", import.meta.url),
//     appName: "counter",
//   });
//
//   // Start the XR session (or fall back to flat rendering)
//   await runtime.start();
//
//   // Later:
//   await runtime.stop();

// Shared mutation interpreter and template cache from web/runtime.
// These replace the inline mutation applier that was previously duplicated
// in this file (~420 lines). Using the shared implementation ensures full
// DOM feature parity with the web renderer.
import { Interpreter } from "../../../web/runtime/interpreter.ts";
import { TemplateCache } from "../../../web/runtime/templates.ts";
import type { XRPointerEventNameType } from "./xr-input.ts";
import { XRInputHandler } from "./xr-input.ts";
import { type XRPanel, XRPanelManager } from "./xr-panel.ts";
import { XRQuadRenderer } from "./xr-renderer.ts";
import { XRSessionManager } from "./xr-session.ts";
import type {
	PanelConfig,
	Vec3,
	XRFrameCompat,
	XRHandedness,
	XRInputSourceCompat,
	XRReferenceSpaceCompat,
	XRRuntimeConfig,
	XRRuntimeEvent,
	XRRuntimeEventListener,
	XRSessionCompat,
	XRViewerPoseCompat,
	XRWebGLLayerCompat,
} from "./xr-types.ts";
import { defaultXRRuntimeConfig } from "./xr-types.ts";

// ── App Panel Configuration ─────────────────────────────────────────────────

/**
 * Configuration for creating an app-connected XR panel.
 *
 * This connects a WASM app (loaded via convention-based export discovery)
 * to an XR panel. The WASM app's mutations are applied to the panel's
 * offscreen DOM container, which is then rasterized to a texture.
 */
export interface AppPanelConfig {
	/** URL to the .wasm file. */
	wasmUrl: string | URL;

	/**
	 * App name prefix for convention-based WASM export discovery.
	 * E.g. "counter" discovers counter_init, counter_rebuild, etc.
	 */
	appName: string;

	/** Panel configuration (size, pixel density, etc.). Optional — uses defaults. */
	panelConfig?: Partial<PanelConfig>;

	/** Initial world-space position (meters). Default: {x: 0, y: 1.4, z: -1}. */
	position?: Vec3;

	/** Mutation buffer capacity in bytes. Default: 65536. */
	bufferCapacity?: number;
}

// ── App Panel Handle ────────────────────────────────────────────────────────

/**
 * A handle to a WASM app running inside an XR panel.
 *
 * Provides lifecycle control (flush, dispatch, destroy) and access
 * to the underlying panel for transform manipulation.
 */
export interface AppPanelHandle {
	/** The XR panel instance. */
	panel: XRPanel;

	/** The WASM app pointer (for direct WASM calls). */
	appPtr: bigint;

	/** Pointer to the mutation buffer in WASM linear memory. */
	bufPtr: bigint;

	/** Mutation buffer capacity. */
	bufCapacity: number;

	/** The WASM exports object. */
	fns: Record<string, CallableFunction>;

	/** Flush pending WASM updates and apply mutations to the panel's DOM. */
	flush(): void;

	/**
	 * Dispatch an event by handler ID and event type, then flush.
	 * This is the XR equivalent of EventBridge.dispatchAndFlush().
	 */
	dispatchAndFlush(handlerId: number, eventType?: number): void;

	/** Whether this app panel has been destroyed. */
	destroyed: boolean;

	/** Destroy the app panel: free WASM resources, remove DOM, release texture. */
	destroy(): void;
}

// ── Runtime State ───────────────────────────────────────────────────────────

/** Current state of the XR runtime. */
export const RuntimeState = {
	/** Not yet initialized. */
	Uninitialized: "uninitialized",
	/** Initialized but no XR session active. */
	Ready: "ready",
	/** XR session is active (rendering in XR). */
	XRActive: "xr-active",
	/** Flat fallback mode (no XR available, rendering in page). */
	FlatFallback: "flat-fallback",
	/** Destroyed — cannot be reused. */
	Destroyed: "destroyed",
} as const;

export type RuntimeStateName = (typeof RuntimeState)[keyof typeof RuntimeState];

// ── Event-to-Handler Map ────────────────────────────────────────────────────
// Maps DOM event names to the Mojo EVT_* constants (must match core events).

const DOM_EVENT_TO_TYPE: Record<string, number> = {
	click: 0,
	input: 1,
	keydown: 2,
	keyup: 3,
	mousemove: 4,
	focus: 5,
	blur: 6,
	submit: 7,
	change: 8,
	mousedown: 9,
	mouseup: 10,
	mouseenter: 11,
	mouseleave: 12,
};

// Default mutation buffer capacity
const DEFAULT_BUF_CAPACITY = 65536;

// ── XRRuntime ───────────────────────────────────────────────────────────────

/**
 * The main WebXR runtime for mojo-gui.
 *
 * Orchestrates the full pipeline:
 *   1. Initialize subsystems (session manager, panel manager, renderer, input)
 *   2. Create panels and wire WASM apps to them
 *   3. Start the XR session (or fall back to flat rendering)
 *   4. Run the XR frame loop:
 *      a. Process input → raycast → dispatch pointer events to WASM
 *      b. Flush WASM updates → apply mutations to panel DOM
 *      c. Rasterize dirty panels → upload textures
 *      d. Render textured quads into the XR framebuffer
 *   5. Clean shutdown
 *
 * Single-panel apps work unchanged: `createAppPanel()` wraps the app in
 * a panel and handles all XR specifics transparently.
 */
export class XRRuntime {
	// ── Subsystems ────────────────────────────────────────────────────

	/** WebXR session manager (lifecycle, frame loop, reference spaces). */
	readonly session: XRSessionManager;

	/** Panel manager (DOM containers, textures, raycasting, layout). */
	readonly panelManager: XRPanelManager;

	/** WebGL quad renderer (draws panel textures in XR space). */
	private _renderer: XRQuadRenderer | null = null;

	/** Input handler (controller rays → DOM pointer events). */
	readonly inputHandler: XRInputHandler;

	// ── State ─────────────────────────────────────────────────────────

	/** Current runtime state. */
	private _state: RuntimeStateName = RuntimeState.Uninitialized;

	/** Runtime configuration. */
	private _config: XRRuntimeConfig = defaultXRRuntimeConfig();

	/** The Document used for DOM operations. */
	private readonly _doc: Document;

	/** Registered app panel handles (for cleanup and per-frame flushing). */
	private readonly _appPanels: Map<number, AppPanelHandle> = new Map();

	/** Handler map: maps (panelId, elementId, eventName) → handlerId. */
	private readonly _handlerMap: Map<string, number> = new Map();

	/** "Enter VR" button element (if created). */
	private _enterVRButton: HTMLButtonElement | null = null;

	/** Event listeners for runtime lifecycle events. */
	private readonly _eventListeners: XRRuntimeEventListener[] = [];

	/** Whether texture updates should use the fallback rasterizer. */
	private _useFallbackRasterizer = false;

	constructor(doc?: Document) {
		this._doc = doc ?? globalThis.document;

		this.session = new XRSessionManager();
		this.panelManager = new XRPanelManager(this._doc);
		this.inputHandler = new XRInputHandler(this.panelManager);

		// Wire input handler events to WASM dispatch
		this.inputHandler.onPointerEvent = (
			panelId: number,
			eventName: XRPointerEventNameType,
			pixel: { x: number; y: number },
			_handedness: XRHandedness,
		) => {
			this.handlePanelPointerEvent(panelId, eventName, pixel);
		};

		this.inputHandler.onFocusChange = (panelId: number) => {
			this.panelManager.focusPanel(panelId);
		};
	}

	// ── Getters ───────────────────────────────────────────────────────

	/** Current runtime state. */
	get state(): RuntimeStateName {
		return this._state;
	}

	/** Runtime configuration. */
	get config(): Readonly<XRRuntimeConfig> {
		return this._config;
	}

	/** The WebGL quad renderer (available after initialize()). */
	get renderer(): XRQuadRenderer | null {
		return this._renderer;
	}

	/** Whether an XR session is currently active. */
	get isXRActive(): boolean {
		return this._state === RuntimeState.XRActive;
	}

	/** Whether we're in flat fallback mode. */
	get isFlatFallback(): boolean {
		return this._state === RuntimeState.FlatFallback;
	}

	/** Number of active app panels. */
	get appPanelCount(): number {
		return this._appPanels.size;
	}

	// ── Initialization ────────────────────────────────────────────────

	/**
	 * Initialize the XR runtime.
	 *
	 * Checks for WebXR availability, creates the panel manager with the
	 * configured settings, and optionally creates an "Enter VR" button.
	 *
	 * @param config - Runtime configuration. Uses defaults if omitted.
	 * @returns Whether WebXR is available (true) or the runtime will
	 *          use flat fallback (false).
	 */
	async initialize(config?: Partial<XRRuntimeConfig>): Promise<boolean> {
		if (this._state !== RuntimeState.Uninitialized) {
			throw new Error(
				`XRRuntime: cannot initialize — current state is "${this._state}"`,
			);
		}

		this._config = { ...defaultXRRuntimeConfig(), ...config };

		// Reconfigure panel manager with updated settings
		// (We create a new one since the constructor parameters aren't mutable)
		const pm = new XRPanelManager(
			this._doc,
			this._config.panelBackground,
			this._config.textureUpdateRate,
		);
		// Replace the readonly reference via Object.defineProperty
		Object.defineProperty(this, "panelManager", {
			value: pm,
			writable: false,
			configurable: true,
		});

		// Re-wire input handler to the new panel manager
		const ih = new XRInputHandler(pm);
		ih.onPointerEvent = (
			panelId: number,
			eventName: XRPointerEventNameType,
			pixel: { x: number; y: number },
			_handedness: XRHandedness,
		) => {
			this.handlePanelPointerEvent(panelId, eventName, pixel);
		};
		ih.onFocusChange = (panelId: number) => {
			this.panelManager.focusPanel(panelId);
		};
		Object.defineProperty(this, "inputHandler", {
			value: ih,
			writable: false,
			configurable: true,
		});

		// Check WebXR availability
		const xrAvailable = await this.session.isSupported(
			this._config.sessionMode,
		);

		if (xrAvailable) {
			this._state = RuntimeState.Ready;

			// Optionally create "Enter VR" button
			if (this._config.showEnterVRButton) {
				this.createEnterVRButton();
			}

			return true;
		}

		if (this._config.fallbackToFlat) {
			this._state = RuntimeState.FlatFallback;
			this.emit({ type: "fallback-to-flat" });
			return false;
		}

		throw new Error(
			`WebXR session mode "${this._config.sessionMode}" is not supported ` +
				"and fallbackToFlat is disabled.",
		);
	}

	// ── Panel Creation ────────────────────────────────────────────────

	/**
	 * Create a standalone XR panel (without a WASM app).
	 *
	 * The caller is responsible for applying mutations to the panel's
	 * container element. This is useful for static content or custom
	 * rendering.
	 *
	 * @param config   - Panel configuration (size, pixel density, etc.).
	 * @param position - World-space position. Default: {x: 0, y: 1.4, z: -1}.
	 * @returns The created panel.
	 */
	createPanel(config?: Partial<PanelConfig>, position?: Vec3): XRPanel {
		this.assertReady();

		const panel = this.panelManager.createPanel(config);

		if (position) {
			panel.setPosition(position.x, position.y, position.z);
		}

		return panel;
	}

	/**
	 * Create an XR panel connected to a WASM app.
	 *
	 * This is the primary API for single-panel apps. It:
	 *   1. Creates a panel with the specified configuration
	 *   2. Loads the WASM module
	 *   3. Discovers exports by naming convention ({appName}_init, etc.)
	 *   4. Initializes the app and performs the initial mount (rebuild)
	 *   5. Sets up an Interpreter on the panel's DOM container
	 *   6. Wires event handlers for XR input dispatch
	 *
	 * The returned handle provides flush/dispatch/destroy controls.
	 *
	 * @param config - App panel configuration (WASM URL, app name, etc.).
	 * @returns A promise resolving to the app panel handle.
	 */
	async createAppPanel(config: AppPanelConfig): Promise<AppPanelHandle> {
		this.assertReady();

		const panelConfig = config.panelConfig ?? {};
		const position = config.position ?? { x: 0, y: 1.4, z: -1 };
		const bufCapacity = config.bufferCapacity ?? DEFAULT_BUF_CAPACITY;

		// 1. Create the panel
		const panel = this.panelManager.createPanel(panelConfig);
		panel.setPosition(position.x, position.y, position.z);

		// 2. Load WASM
		// We use dynamic import of the web runtime's env module for WASM loading.
		// The actual loading is done inline since we can't import from web/runtime
		// at module level (it may not be in the import map).
		const fns = await this.loadWasm(config.wasmUrl, config.appName);

		// 3. Discover exports
		const appName = config.appName;
		const initFn = fns[`${appName}_init`] as (() => bigint) | undefined;
		const rebuildFn = fns[`${appName}_rebuild`] as
			| ((appPtr: bigint, buf: bigint, cap: number) => number)
			| undefined;
		const flushFn = fns[`${appName}_flush`] as
			| ((appPtr: bigint, buf: bigint, cap: number) => number)
			| undefined;
		const handleEventFn = fns[`${appName}_handle_event`] as
			| ((appPtr: bigint, hid: number, evt: number) => number)
			| undefined;
		const destroyFn = fns[`${appName}_destroy`] as
			| ((appPtr: bigint) => void)
			| undefined;

		if (!initFn || !rebuildFn || !flushFn) {
			throw new Error(
				`XRRuntime: WASM module is missing required exports: ` +
					`${appName}_init, ${appName}_rebuild, ${appName}_flush`,
			);
		}

		// 4. Initialize the app
		const appPtr = initFn();

		// 5. Allocate mutation buffer
		// We need access to the WASM memory's allocator. The WASM env module
		// provides alignedAlloc. We call the WASM-exported aligned_alloc if
		// available, otherwise use our memory helper.
		const allocFn = fns.KGEN_CompilerRT_AlignedAlloc as
			| ((align: bigint, size: bigint) => bigint)
			| undefined;
		const freeFn = fns.KGEN_CompilerRT_AlignedFree as
			| ((ptr: bigint) => void)
			| undefined;

		let bufPtr: bigint;
		if (allocFn) {
			bufPtr = allocFn(8n, BigInt(bufCapacity));
		} else {
			// Fallback: use a fixed offset in WASM memory. This is fragile
			// but works for simple cases. Production code should always export
			// the allocator.
			console.warn(
				"XRRuntime: WASM module does not export KGEN_CompilerRT_AlignedAlloc. " +
					"Using zero buffer pointer — mutations may not work correctly.",
			);
			bufPtr = 0n;
		}

		// 6. Set up a shared Interpreter for mutations.
		// We use the web/runtime Interpreter and TemplateCache to apply
		// binary mutation buffers to the panel's DOM container. This gives
		// full DOM feature parity with the web renderer.
		//
		// The handler map tracks (panelId:elementId:eventName) → handlerId
		// for event dispatch from XR input.
		const panelId = panel.id;
		const templateCache = new TemplateCache(this._doc);
		const interpreter = new Interpreter(
			panel.container,
			templateCache,
			this._doc,
		);

		// Wire the Interpreter's listener callbacks to populate the
		// handler map. XR input doesn't use DOM event listeners directly —
		// it raycasts controller rays and dispatches via the handler map.
		interpreter.onNewListener = (
			elementId: number,
			eventName: string,
			handlerId: number,
		): EventListener => {
			this._handlerMap.set(`${panelId}:${elementId}:${eventName}`, handlerId);
			return () => {}; // no-op — XR input bypasses DOM events
		};

		interpreter.onRemoveListener = (
			elementId: number,
			eventName: string,
		): void => {
			this._handlerMap.delete(`${panelId}:${elementId}:${eventName}`);
		};

		const applyMutations = (
			buffer: ArrayBuffer,
			byteOffset: number,
			byteLength: number,
		) => {
			interpreter.applyMutations(buffer, byteOffset, byteLength);
			panel.markDirty();
		};

		// Helper: get WASM memory (from the exports)
		const getWasmMemory = (): WebAssembly.Memory => {
			return fns.memory as unknown as WebAssembly.Memory;
		};

		// 7. Mount — initial rebuild
		const mountLen = rebuildFn(appPtr, bufPtr, bufCapacity);
		if (mountLen > 0) {
			const mem = getWasmMemory();
			applyMutations(mem.buffer, Number(bufPtr), mountLen);
		}
		panel.markMounted();

		// 8. Build the handle
		// Capture `this` for use inside the handle's closure methods
		const _self = this;

		const handle: AppPanelHandle = {
			panel,
			appPtr,
			bufPtr,
			bufCapacity: bufCapacity,
			fns,
			destroyed: false,

			flush(): void {
				if (handle.destroyed) return;
				const len = flushFn(handle.appPtr, handle.bufPtr, handle.bufCapacity);
				if (len > 0) {
					const mem = getWasmMemory();
					applyMutations(mem.buffer, Number(handle.bufPtr), len);
				}
			},

			dispatchAndFlush(handlerId: number, eventType = 0): void {
				if (handle.destroyed) return;
				if (handleEventFn) {
					handleEventFn(handle.appPtr, handlerId, eventType);
				}
				handle.flush();
			},

			destroy(): void {
				if (handle.destroyed) return;
				handle.destroyed = true;

				// Free the mutation buffer
				if (freeFn && handle.bufPtr !== 0n) {
					try {
						freeFn(handle.bufPtr);
					} catch {
						// May fail if WASM instance is already dead
					}
				}

				// Destroy the WASM app
				if (destroyFn) {
					try {
						destroyFn(handle.appPtr);
					} catch {
						// May fail if WASM instance is already dead
					}
				}

				// Clean up handler map entries for this panel
				for (const key of [..._self._handlerMap.keys()]) {
					if (key.startsWith(`${panelId}:`)) {
						_self._handlerMap.delete(key);
					}
				}

				// Destroy the panel (removes DOM, frees texture)
				const gl = _self.session.gl;
				_self.panelManager.destroyPanel(panelId, gl ?? undefined);
				_self._appPanels.delete(panelId);

				handle.appPtr = 0n;
				handle.bufPtr = 0n;
			},
		};

		this._appPanels.set(panelId, handle);
		return handle;
	}

	// ── Session Lifecycle ─────────────────────────────────────────────

	/**
	 * Start the XR session and begin rendering.
	 *
	 * If in flat fallback mode, this makes the panels visible in the
	 * page instead of entering an XR session.
	 *
	 * @param canvas - Optional canvas element for WebGL rendering.
	 */
	async start(canvas?: HTMLCanvasElement): Promise<void> {
		if (this._state === RuntimeState.FlatFallback) {
			// In flat mode, just make the panels visible in the DOM
			this.startFlatMode();
			return;
		}

		if (this._state !== RuntimeState.Ready) {
			throw new Error(
				`XRRuntime: cannot start — current state is "${this._state}". ` +
					"Call initialize() first.",
			);
		}

		// Wire the session's frame callback
		this.session.onFrame = this.onXRFrame.bind(this);

		// Wire session lifecycle events
		this.session.addEventListener((event: XRRuntimeEvent) => {
			this.emit(event);

			if (event.type === "session-ended") {
				this._state = RuntimeState.Ready;
				this.inputHandler.reset();
				if (this._renderer) {
					this._renderer.destroy();
					this._renderer = null;
				}
			}
		});

		// Start the XR session
		await this.session.start(canvas, {
			sessionMode: this._config.sessionMode,
			requiredFeatures: this._config.requiredFeatures,
			optionalFeatures: this._config.optionalFeatures,
		});

		// Create the quad renderer
		const gl = this.session.gl;
		if (!gl) {
			throw new Error("XRRuntime: no WebGL context after session start");
		}

		this._renderer = new XRQuadRenderer(gl);
		this._renderer.initialize();

		// Wire XR input events
		const session = this.session.session;
		if (session) {
			this.wireSessionInputEvents(session);
		}

		// Do an initial texture capture for all mounted panels
		try {
			await this.panelManager.updateDirtyTextures(
				gl,
				this._useFallbackRasterizer,
			);
		} catch {
			// SVG foreignObject may not work in all WebXR browsers.
			// Switch to fallback rasterizer.
			this._useFallbackRasterizer = true;
			await this.panelManager.updateDirtyTextures(gl, true);
		}

		this._state = RuntimeState.XRActive;

		// Hide the "Enter VR" button
		if (this._enterVRButton) {
			this._enterVRButton.style.display = "none";
		}
	}

	/**
	 * Stop the XR session and clean up.
	 *
	 * Panels and app handles remain alive — you can restart the session
	 * later. Call `destroy()` for full cleanup.
	 */
	async stop(): Promise<void> {
		if (this._state === RuntimeState.XRActive) {
			this.inputHandler.reset();
			await this.session.end();
			// State transition happens in the session-ended event handler
		}

		if (this._state === RuntimeState.FlatFallback) {
			this.stopFlatMode();
		}

		// Show the "Enter VR" button again
		if (this._enterVRButton) {
			this._enterVRButton.style.display = "";
		}
	}

	/**
	 * Destroy the runtime and all panels.
	 *
	 * Ends the XR session, destroys all app panels, and releases all
	 * resources. The runtime cannot be reused after this.
	 */
	async destroy(): Promise<void> {
		if (this._state === RuntimeState.Destroyed) return;

		// Stop the session if active
		try {
			await this.stop();
		} catch {
			// Ignore errors during shutdown
		}

		// Destroy all app panels
		for (const handle of this._appPanels.values()) {
			if (!handle.destroyed) {
				handle.destroy();
			}
		}
		this._appPanels.clear();

		// Destroy the panel manager
		const gl = this.session.gl;
		this.panelManager.destroyAll(gl ?? undefined);

		// Destroy the renderer
		if (this._renderer) {
			this._renderer.destroy();
			this._renderer = null;
		}

		// Clean up the "Enter VR" button
		if (this._enterVRButton?.parentNode) {
			this._enterVRButton.parentNode.removeChild(this._enterVRButton);
			this._enterVRButton = null;
		}

		// Clear handler map
		this._handlerMap.clear();

		// Reset input handler
		this.inputHandler.reset();

		this._state = RuntimeState.Destroyed;
	}

	// ── Event Listeners ───────────────────────────────────────────────

	/**
	 * Register a listener for runtime lifecycle events.
	 *
	 * @param listener - Callback for session-started, session-ended, errors, etc.
	 * @returns An unsubscribe function.
	 */
	addEventListener(listener: XRRuntimeEventListener): () => void {
		this._eventListeners.push(listener);
		return () => {
			const idx = this._eventListeners.indexOf(listener);
			if (idx !== -1) this._eventListeners.splice(idx, 1);
		};
	}

	// ── XR Frame Loop ─────────────────────────────────────────────────

	/**
	 * Per-frame callback invoked by the XRSessionManager.
	 *
	 * This is the heart of the XR rendering pipeline:
	 *   1. Process XR input → raycast → dispatch pointer events
	 *   2. Flush all dirty app panels (WASM updates → DOM mutations)
	 *   3. Update dirty panel textures (DOM → canvas → WebGL texture)
	 *   4. Render all panels as textured quads in the XR scene
	 */
	private onXRFrame(
		_time: number,
		frame: XRFrameCompat,
		refSpace: XRReferenceSpaceCompat,
		viewerPose: XRViewerPoseCompat | null,
		gl: WebGL2RenderingContext,
		glLayer: XRWebGLLayerCompat,
	): void {
		// 1. Process XR input
		const session = frame.session;
		this.inputHandler.processFrame(frame, refSpace, session.inputSources);

		// 2. Update dirty textures (synchronous fallback only in the frame loop;
		//    async SVG rasterization is handled between frames)
		// For the frame loop, we use the synchronous fallback rasterizer to
		// avoid async operations that could miss the frame deadline.
		const dirtyPanels = this.panelManager.getDirtyPanels();
		for (const panel of dirtyPanels) {
			panel.rasterizeFallback();
			panel.uploadTexture(gl);
		}

		// 3. Render
		if (!this._renderer || !viewerPose) return;

		this._renderer.beginFrame(glLayer);
		this._renderer.renderAllViews(
			viewerPose,
			glLayer,
			this.panelManager.panels,
		);

		// 4. Draw cursors for active input hits
		for (const [_sourceId, hit] of this.inputHandler.getAllCurrentHits()) {
			const panel = this.panelManager.getPanel(hit.panelId);
			if (panel) {
				// Draw cursor for each view
				for (const view of viewerPose.views) {
					this._renderer.setView(view, glLayer);
					this._renderer.drawCursor(panel, hit.uv.u, hit.uv.v);
				}
			}
		}

		this._renderer.endFrame();
	}

	// ── XR Input Event Wiring ─────────────────────────────────────────

	/**
	 * Wire WebXR session input events (select, selectstart, selectend)
	 * to the input handler.
	 */
	private wireSessionInputEvents(session: XRSessionCompat): void {
		session.addEventListener("select", ((e: Event) => {
			const xrEvent = e as Event & { inputSource?: XRInputSourceCompat };
			if (xrEvent.inputSource) {
				this.inputHandler.onSelect(xrEvent.inputSource);
			}
		}) as EventListener);

		session.addEventListener("selectstart", ((e: Event) => {
			const xrEvent = e as Event & { inputSource?: XRInputSourceCompat };
			if (xrEvent.inputSource) {
				this.inputHandler.onSelectStart(xrEvent.inputSource);
			}
		}) as EventListener);

		session.addEventListener("selectend", ((e: Event) => {
			const xrEvent = e as Event & { inputSource?: XRInputSourceCompat };
			if (xrEvent.inputSource) {
				this.inputHandler.onSelectEnd(xrEvent.inputSource);
			}
		}) as EventListener);
	}

	// ── Pointer Event → WASM Dispatch ─────────────────────────────────

	/**
	 * Handle a pointer event from the XR input handler.
	 *
	 * Looks up the nearest handler for the hit pixel coordinates on the
	 * panel's DOM, and dispatches to the WASM app via the handler map.
	 */
	private handlePanelPointerEvent(
		panelId: number,
		eventName: XRPointerEventNameType,
		_pixel: { x: number; y: number },
	): void {
		const handle = this._appPanels.get(panelId);
		if (!handle || handle.destroyed) return;

		const panel = handle.panel;
		if (!panel.state.mounted) return;

		const eventType = DOM_EVENT_TO_TYPE[eventName];
		if (eventType === undefined) return;

		// Find the DOM element at the pixel coordinates by walking
		// the handler map for this panel. The handler map is populated
		// by NewEventListener mutations during mount/update.
		//
		// Strategy: look for handlers registered for this panel + event name.
		// The most recently registered handler for this event type wins.
		// (In a full implementation, we'd use elementFromPoint or
		// hit-test the DOM tree at the pixel coordinates.)
		//
		// For single-panel single-button apps (Counter, etc.), there's
		// typically only one handler per event type, so this simple
		// approach works. Future: proper DOM hit testing.
		let bestHandlerId: number | undefined;
		const prefix = `${panelId}:`;
		const suffix = `:${eventName}`;

		for (const [key, hid] of this._handlerMap.entries()) {
			if (key.startsWith(prefix) && key.endsWith(suffix)) {
				bestHandlerId = hid;
				// Don't break — keep the last one (highest element ID).
				// A smarter implementation would pick the element closest
				// to the hit pixel using bounding rect intersection.
			}
		}

		if (bestHandlerId !== undefined) {
			handle.dispatchAndFlush(bestHandlerId, eventType);
		}
	}

	// ── WASM Loading ──────────────────────────────────────────────────

	/**
	 * Load and instantiate a WASM module.
	 *
	 * Provides the minimal environment imports that the Mojo WASM runtime
	 * expects. This is a simplified version of web/runtime/env.ts for the
	 * XR context.
	 *
	 * @param wasmUrl - URL to the .wasm file.
	 * @param _appName - App name (for error messages).
	 * @returns The WASM instance exports.
	 */
	private async loadWasm(
		wasmUrl: string | URL,
		_appName: string,
	): Promise<Record<string, CallableFunction>> {
		const response = await fetch(wasmUrl);
		const wasmBuffer = await response.arrayBuffer();

		// Minimal WASM environment imports (matching web/runtime/env.ts)
		const decoder = new TextDecoder();
		const importMemory = new WebAssembly.Memory({ initial: 4096 });
		let wasmMemory: WebAssembly.Memory = importMemory;

		const env: WebAssembly.ModuleImports = {
			memory: importMemory,

			__cxa_atexit: () => 0,

			KGEN_CompilerRT_AlignedAlloc: (align: bigint, size: bigint): bigint => {
				// Simple bump allocator within WASM memory.
				// In production, this would use the WASM module's own allocator.
				const alignNum = Number(align);
				const sizeNum = Number(size);
				const currentPages = wasmMemory.buffer.byteLength / 65536;
				const needed = Math.ceil(sizeNum / 65536) + 1;
				if (needed > currentPages) {
					try {
						wasmMemory.grow(needed - currentPages);
					} catch {
						return 0n;
					}
				}
				// Allocate from the end of the current memory
				// This is a simplified allocator — real code uses the module's allocator
				const offset = wasmMemory.buffer.byteLength - sizeNum;
				const aligned = offset - (offset % alignNum);
				return BigInt(aligned);
			},

			KGEN_CompilerRT_AlignedFree: (_ptr: bigint): void => {
				// No-op in simplified allocator
			},

			KGEN_CompilerRT_GetStackTrace: (): bigint => 0n,
			KGEN_CompilerRT_fprintf: (): number => 0,

			write: (fd: bigint, ptr: bigint, len: bigint): number => {
				if (len === 0n) return 0;
				try {
					const data = new Uint8Array(
						wasmMemory.buffer,
						Number(ptr),
						Number(len),
					);
					const text = decoder.decode(data);
					if (fd === 1n) {
						console.log(text);
					} else if (fd === 2n) {
						console.error(text);
					}
					return Number(len);
				} catch {
					return -1;
				}
			},

			free: (_ptr: bigint): void => {},
			dup: (): number => 1,
			fdopen: (): number => 1,
			fflush: (): number => 1,
			fclose: (): number => 1,

			__multi3: (
				resultPtr: bigint,
				aLo: bigint,
				aHi: bigint,
				bLo: bigint,
				bHi: bigint,
			): void => {
				const mask64 = 0xffffffffffffffffn;
				const a = ((aHi & mask64) << 64n) | (aLo & mask64);
				const b = ((bHi & mask64) << 64n) | (bLo & mask64);
				const product = a * b;
				const lo = product & mask64;
				const hi = (product >> 64n) & mask64;
				const view = new DataView(wasmMemory.buffer);
				const ptr = Number(resultPtr);
				view.setBigInt64(ptr, lo, true);
				view.setBigInt64(ptr + 8, hi, true);
			},

			clock_gettime: (_clockid: number, tsPtr: bigint): number => {
				const now = performance.now();
				const sec = BigInt(Math.floor(now / 1000));
				const nsec = BigInt(Math.floor((now % 1000) * 1_000_000));
				const view = new DataView(wasmMemory.buffer);
				const ptr = Number(tsPtr);
				view.setBigInt64(ptr, sec, true);
				view.setBigInt64(ptr + 8, nsec, true);
				return 0;
			},

			performance_now: (): number => performance.now(),

			push_state: (): void => {},
			replace_state: (): void => {},

			fmaf: (x: number, y: number, z: number): number =>
				Math.fround(Math.fround(x * y) + z),
			fminf: (x: number, y: number): number => Math.min(x, y),
			fmaxf: (x: number, y: number): number => Math.max(x, y),
			fma: (x: number, y: number, z: number): number => x * y + z,
			fmin: (x: number, y: number): number => Math.min(x, y),
			fmax: (x: number, y: number): number => Math.max(x, y),
		};

		const { instance } = await WebAssembly.instantiate(wasmBuffer, { env });

		// If the WASM module exports its own memory, use that
		if (instance.exports.memory) {
			wasmMemory = instance.exports.memory as WebAssembly.Memory;
		}

		return instance.exports as unknown as Record<string, CallableFunction>;
	}

	// ── Flat Fallback Mode ────────────────────────────────────────────

	/**
	 * Start flat (non-XR) rendering mode.
	 *
	 * Makes panel containers visible in the page so they render as
	 * normal DOM elements. No WebGL, no 3D — just the DOM.
	 */
	private startFlatMode(): void {
		for (const panel of this.panelManager.panels) {
			// Make the container visible in the page
			panel.container.style.cssText = [
				"position: relative",
				"left: auto",
				"top: auto",
				`width: ${panel.textureWidth}px`,
				`height: ${panel.textureHeight}px`,
				"overflow: hidden",
				"visibility: visible",
				"pointer-events: auto",
				"margin: 1rem auto",
				"border: 1px solid #ccc",
				"border-radius: 8px",
				"box-shadow: 0 2px 8px rgba(0,0,0,0.1)",
			].join("; ");
		}
	}

	/**
	 * Stop flat rendering mode.
	 *
	 * Moves panel containers back to offscreen positions.
	 */
	private stopFlatMode(): void {
		for (const panel of this.panelManager.panels) {
			panel.container.style.cssText = [
				"position: absolute",
				"left: -99999px",
				"top: -99999px",
				`width: ${panel.textureWidth}px`,
				`height: ${panel.textureHeight}px`,
				"overflow: hidden",
				"visibility: hidden",
				"pointer-events: none",
			].join("; ");
		}
	}

	// ── "Enter VR" Button ─────────────────────────────────────────────

	/**
	 * Create a styled "Enter VR" button and append it to the page.
	 *
	 * Clicking the button starts the XR session. The button is hidden
	 * while the session is active and shown again when it ends.
	 */
	private createEnterVRButton(): void {
		const button = this._doc.createElement("button");
		button.textContent = "🥽 Enter VR";
		button.id = "xr-enter-vr";
		button.style.cssText = [
			"position: fixed",
			"bottom: 2rem",
			"right: 2rem",
			"padding: 0.8rem 1.5rem",
			"font-size: 1.1rem",
			"font-weight: 600",
			"background: linear-gradient(135deg, #6366f1, #8b5cf6)",
			"color: white",
			"border: none",
			"border-radius: 12px",
			"cursor: pointer",
			"box-shadow: 0 4px 15px rgba(99, 102, 241, 0.4)",
			"z-index: 10000",
			"transition: transform 0.1s, box-shadow 0.1s",
		].join("; ");

		button.addEventListener("mouseenter", () => {
			button.style.transform = "translateY(-2px)";
			button.style.boxShadow = "0 6px 20px rgba(99, 102, 241, 0.5)";
		});

		button.addEventListener("mouseleave", () => {
			button.style.transform = "";
			button.style.boxShadow = "0 4px 15px rgba(99, 102, 241, 0.4)";
		});

		button.addEventListener("click", () => {
			this.start().catch((err) => {
				console.error("Failed to start XR session:", err);
			});
		});

		this._doc.body.appendChild(button);
		this._enterVRButton = button;
	}

	// ── Internal Helpers ──────────────────────────────────────────────

	/** Assert that the runtime is in a state where operations are allowed. */
	private assertReady(): void {
		if (
			this._state !== RuntimeState.Ready &&
			this._state !== RuntimeState.FlatFallback &&
			this._state !== RuntimeState.XRActive
		) {
			throw new Error(
				`XRRuntime: operation not allowed in state "${this._state}". ` +
					"Call initialize() first.",
			);
		}
	}

	/** Emit a runtime event to all registered listeners. */
	private emit(event: XRRuntimeEvent): void {
		for (const listener of this._eventListeners) {
			try {
				listener(event);
			} catch (err) {
				console.error("XRRuntime: error in event listener:", err);
			}
		}
	}
}
