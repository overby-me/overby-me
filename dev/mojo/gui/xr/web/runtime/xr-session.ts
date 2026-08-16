// WebXR Session Manager — Session lifecycle, reference spaces, and GL setup.
//
// Manages the WebXR session lifecycle:
//   1. Feature detection (is WebXR available? is the requested mode supported?)
//   2. Session request (immersive-vr, immersive-ar, or inline)
//   3. Reference space creation (local-floor, bounded-floor, local, viewer)
//   4. WebGL context + XRWebGLLayer setup
//   5. Frame loop delegation (hands off to the XR runtime's render callback)
//   6. Session end + cleanup
//
// This module is environment-aware: it checks for `navigator.xr` and degrades
// gracefully when WebXR is unavailable (the runtime falls back to flat
// rendering via the standard web renderer).
//
// Usage:
//
//   const session = new XRSessionManager();
//   const supported = await session.isSupported("immersive-vr");
//   if (supported) {
//     const canvas = document.createElement("canvas");
//     await session.start(canvas, {
//       sessionMode: "immersive-vr",
//       requiredFeatures: ["local-floor"],
//       optionalFeatures: ["hand-tracking"],
//     });
//     session.onFrame = (time, frame, refSpace) => { /* render */ };
//   }
//   // later:
//   await session.end();

import type {
	XRFrameCompat,
	XRInputSourceCompat,
	XRReferenceSpaceCompat,
	XRRuntimeEvent,
	XRRuntimeEventListener,
	XRSessionCompat,
	XRSessionMode,
	XRSystemCompat,
	XRViewerPoseCompat,
	XRWebGLLayerCompat,
	XRWebGLLayerConstructor,
} from "./xr-types.ts";

// ── Session State ───────────────────────────────────────────────────────────

/** Current state of the XR session. */
export const SessionState = {
	/** No session active; ready to start. */
	Idle: "idle",
	/** Session is being requested (async). */
	Starting: "starting",
	/** Session is active and rendering. */
	Active: "active",
	/** Session is ending (async). */
	Ending: "ending",
} as const;

export type SessionStateName = (typeof SessionState)[keyof typeof SessionState];

// ── Configuration ───────────────────────────────────────────────────────────

/** Options passed to `XRSessionManager.start()`. */
export interface SessionStartOptions {
	/** WebXR session mode. Default: "immersive-vr". */
	sessionMode?: XRSessionMode;

	/** Required WebXR features. Default: ["local-floor"]. */
	requiredFeatures?: string[];

	/** Optional WebXR features. Default: ["hand-tracking", "bounded-floor"]. */
	optionalFeatures?: string[];

	/**
	 * Preferred reference space type, in order of preference.
	 * The manager tries each in order and uses the first that succeeds.
	 * Default: ["local-floor", "bounded-floor", "local", "viewer"].
	 */
	preferredReferenceSpaces?: string[];

	/**
	 * WebGL context attributes passed to `canvas.getContext("webgl2")`.
	 * The `xrCompatible` attribute is always forced to `true`.
	 */
	glAttributes?: WebGLContextAttributes;
}

const DEFAULT_START_OPTIONS: Required<SessionStartOptions> = {
	sessionMode: "immersive-vr",
	requiredFeatures: ["local-floor"],
	optionalFeatures: ["hand-tracking", "bounded-floor"],
	preferredReferenceSpaces: ["local-floor", "bounded-floor", "local", "viewer"],
	glAttributes: {},
};

// ── Frame Callback ──────────────────────────────────────────────────────────

/**
 * Signature for the per-frame render callback.
 *
 * @param time        - High-resolution timestamp (from XRSession.requestAnimationFrame).
 * @param frame       - The current XRFrame (use to get poses, hit tests, etc.).
 * @param refSpace    - The active reference space.
 * @param viewerPose  - The viewer pose for this frame (null if tracking lost).
 * @param gl          - The WebGL2 rendering context.
 * @param glLayer     - The XRWebGLLayer (for framebuffer and viewport access).
 */
export type XRFrameCallback = (
	time: number,
	frame: XRFrameCompat,
	refSpace: XRReferenceSpaceCompat,
	viewerPose: XRViewerPoseCompat | null,
	gl: WebGL2RenderingContext,
	glLayer: XRWebGLLayerCompat,
) => void;

// ── XRSessionManager ────────────────────────────────────────────────────────

/**
 * Manages the full WebXR session lifecycle.
 *
 * Responsibilities:
 *   - Feature detection and capability queries
 *   - Session creation with the appropriate mode and features
 *   - WebGL context creation with `xrCompatible: true`
 *   - XRWebGLLayer binding
 *   - Reference space negotiation (tries preferred spaces in order)
 *   - XR frame loop (delegates to `onFrame` callback each frame)
 *   - Input source tracking (exposes current input sources)
 *   - Clean session teardown
 *
 * The session manager does NOT handle rendering or DOM manipulation —
 * that's the responsibility of the XR runtime and panel manager.
 */
export class XRSessionManager {
	// ── State ─────────────────────────────────────────────────────────

	/** Current session state. */
	private _state: SessionStateName = SessionState.Idle;

	/** The active XR session, or null. */
	private _session: XRSessionCompat | null = null;

	/** The active reference space, or null. */
	private _refSpace: XRReferenceSpaceCompat | null = null;

	/** Type name of the active reference space (e.g. "local-floor"). */
	private _refSpaceType: string | null = null;

	/** The WebGL2 rendering context, or null. */
	private _gl: WebGL2RenderingContext | null = null;

	/** The XRWebGLLayer bound to the session, or null. */
	private _glLayer: XRWebGLLayerCompat | null = null;

	/** The canvas element backing the GL context. */
	private _canvas: HTMLCanvasElement | null = null;

	/** The current XR animation frame request ID (for cancellation). */
	private _frameHandle: number | null = null;

	/** Registered event listeners for cleanup. */
	private _eventListeners: Array<{
		target: XRSessionCompat;
		type: string;
		listener: EventListenerOrEventListenerObject;
	}> = [];

	/** Runtime event listeners. */
	private _runtimeListeners: XRRuntimeEventListener[] = [];

	// ── Public callbacks ──────────────────────────────────────────────

	/**
	 * Per-frame render callback. Set this before calling `start()`.
	 * Called once per XR frame with the current pose and GL context.
	 */
	onFrame: XRFrameCallback | null = null;

	// ── Getters ───────────────────────────────────────────────────────

	/** Current session state. */
	get state(): SessionStateName {
		return this._state;
	}

	/** Whether a session is currently active. */
	get isActive(): boolean {
		return this._state === SessionState.Active;
	}

	/** The active XR session, or null. */
	get session(): XRSessionCompat | null {
		return this._session;
	}

	/** The active reference space, or null. */
	get referenceSpace(): XRReferenceSpaceCompat | null {
		return this._refSpace;
	}

	/** Type name of the active reference space. */
	get referenceSpaceType(): string | null {
		return this._refSpaceType;
	}

	/** The WebGL2 rendering context, or null. */
	get gl(): WebGL2RenderingContext | null {
		return this._gl;
	}

	/** The XRWebGLLayer, or null. */
	get glLayer(): XRWebGLLayerCompat | null {
		return this._glLayer;
	}

	/** The canvas element, or null. */
	get canvas(): HTMLCanvasElement | null {
		return this._canvas;
	}

	/** Current XR input sources (empty array if no session). */
	get inputSources(): readonly XRInputSourceCompat[] {
		if (!this._session) return [];
		return [...this._session.inputSources];
	}

	// ── Feature Detection ─────────────────────────────────────────────

	/**
	 * Get the WebXR system object (`navigator.xr`), or null if unavailable.
	 *
	 * This checks for the presence of the WebXR Device API in the current
	 * environment. Returns null in non-browser environments, in browsers
	 * without WebXR support, or in insecure contexts (WebXR requires HTTPS).
	 */
	getXRSystem(): XRSystemCompat | null {
		if (
			typeof globalThis !== "undefined" &&
			"navigator" in globalThis &&
			"xr" in (globalThis as { navigator: { xr?: unknown } }).navigator
		) {
			return (globalThis as { navigator: { xr: XRSystemCompat } }).navigator.xr;
		}
		return null;
	}

	/**
	 * Check whether a given XR session mode is supported.
	 *
	 * @param mode - The session mode to check (e.g. "immersive-vr").
	 * @returns `true` if the mode is supported, `false` otherwise.
	 *          Returns `false` if WebXR is not available at all.
	 */
	async isSupported(mode: XRSessionMode = "immersive-vr"): Promise<boolean> {
		const xr = this.getXRSystem();
		if (!xr) return false;

		try {
			return await xr.isSessionSupported(mode);
		} catch {
			return false;
		}
	}

	// ── Session Lifecycle ─────────────────────────────────────────────

	/**
	 * Start a WebXR session.
	 *
	 * Creates a WebGL2 context on the provided canvas (or creates one),
	 * requests an XR session, sets up the XRWebGLLayer, negotiates a
	 * reference space, and starts the XR frame loop.
	 *
	 * @param canvas  - The canvas element to use for WebGL rendering.
	 *                  If not provided, one is created and appended to `document.body`.
	 * @param options - Session configuration (mode, features, etc.).
	 * @throws If WebXR is not available, the mode is not supported, or
	 *         session creation fails.
	 */
	async start(
		canvas?: HTMLCanvasElement,
		options?: SessionStartOptions,
	): Promise<void> {
		if (this._state !== SessionState.Idle) {
			throw new Error(
				`XRSessionManager: cannot start — current state is "${this._state}"`,
			);
		}

		this._state = SessionState.Starting;

		const opts = { ...DEFAULT_START_OPTIONS, ...options };

		try {
			// 1. Get the XR system
			const xr = this.getXRSystem();
			if (!xr) {
				throw new Error(
					"WebXR is not available in this environment. " +
						"Ensure you are on HTTPS and using a WebXR-capable browser.",
				);
			}

			// 2. Check support
			const supported = await xr.isSessionSupported(opts.sessionMode);
			if (!supported) {
				throw new Error(
					`XR session mode "${opts.sessionMode}" is not supported by this device/browser.`,
				);
			}

			// 3. Create or reuse the canvas
			if (!canvas) {
				canvas = document.createElement("canvas");
				// XR canvas doesn't need explicit size — the XRWebGLLayer
				// manages the framebuffer dimensions.
				canvas.style.display = "none";
				document.body.appendChild(canvas);
			}
			this._canvas = canvas;

			// 4. Create WebGL2 context with xrCompatible
			const glAttrs: WebGLContextAttributes = {
				...opts.glAttributes,
				xrCompatible: true,
				alpha: true,
				antialias: true,
				depth: true,
				stencil: false,
				premultipliedAlpha: true,
				preserveDrawingBuffer: false,
			};

			const gl = canvas.getContext("webgl2", glAttrs);
			if (!gl) {
				throw new Error(
					"Failed to create WebGL2 context with xrCompatible. " +
						"Ensure your browser supports WebGL2.",
				);
			}
			this._gl = gl;

			// 5. Request the XR session
			const session = await xr.requestSession(opts.sessionMode, {
				requiredFeatures: opts.requiredFeatures,
				optionalFeatures: opts.optionalFeatures,
			});
			this._session = session;

			// 6. Create and bind XRWebGLLayer
			// Access XRWebGLLayer constructor from globalThis (it's a global in
			// WebXR-capable browsers).
			const XRWebGLLayerCtor = (
				globalThis as unknown as Record<string, unknown>
			).XRWebGLLayer as XRWebGLLayerConstructor | undefined;
			if (!XRWebGLLayerCtor) {
				throw new Error("XRWebGLLayer constructor not found in globalThis.");
			}

			const glLayer = new XRWebGLLayerCtor(session, gl);
			this._glLayer = glLayer;
			session.updateRenderState({ baseLayer: glLayer });

			// 7. Negotiate reference space
			const refSpace = await this.negotiateReferenceSpace(
				session,
				opts.preferredReferenceSpaces,
			);
			this._refSpace = refSpace.space;
			this._refSpaceType = refSpace.type;

			// 8. Wire session event listeners
			this.addSessionListener(session, "end", () => {
				this.handleSessionEnd();
			});

			this.addSessionListener(session, "visibilitychange", () => {
				this.emit({
					type: "visibility-changed",
					state: session.visibilityState,
				});
			});

			this.addSessionListener(session, "inputsourceschange", () => {
				this.emit({ type: "input-sources-changed" });
			});

			// 9. Transition to active and start frame loop
			this._state = SessionState.Active;
			this.emit({ type: "session-started" });
			this.scheduleFrame();
		} catch (err) {
			// Clean up partial initialization on failure
			this.cleanupResources();
			this._state = SessionState.Idle;

			const error = err instanceof Error ? err : new Error(String(err));
			this.emit({ type: "error", error });
			throw error;
		}
	}

	/**
	 * End the active XR session.
	 *
	 * Cancels the frame loop, ends the session, and releases all resources.
	 * Safe to call when no session is active (no-op).
	 */
	async end(): Promise<void> {
		if (this._state === SessionState.Idle) return;
		if (this._state === SessionState.Ending) return;

		if (this._state === SessionState.Starting) {
			// Session is still being created — defer end to after start completes.
			// This is a race condition that shouldn't happen in practice, but
			// we handle it gracefully.
			throw new Error(
				"XRSessionManager: cannot end session while it is starting. " +
					"Wait for start() to resolve first.",
			);
		}

		this._state = SessionState.Ending;

		// Cancel pending frame
		if (this._frameHandle !== null && this._session) {
			this._session.cancelAnimationFrame(this._frameHandle);
			this._frameHandle = null;
		}

		// End the session (triggers the "end" event, which calls handleSessionEnd)
		if (this._session) {
			try {
				await this._session.end();
			} catch {
				// Session may already be ended — ignore.
			}
		}

		// handleSessionEnd may not have fired if the session was already
		// ended. Ensure cleanup happens.
		if (this._state !== SessionState.Idle) {
			this.handleSessionEnd();
		}
	}

	// ── Event Listener Registration ─────────────────────────────────

	/**
	 * Register a listener for XR runtime lifecycle events.
	 *
	 * @param listener - Callback invoked on session-started, session-ended,
	 *                   visibility changes, input source changes, errors, etc.
	 * @returns An unsubscribe function.
	 */
	addEventListener(listener: XRRuntimeEventListener): () => void {
		this._runtimeListeners.push(listener);
		return () => {
			const idx = this._runtimeListeners.indexOf(listener);
			if (idx !== -1) this._runtimeListeners.splice(idx, 1);
		};
	}

	// ── Internal: Reference Space Negotiation ─────────────────────────

	/**
	 * Try each preferred reference space type in order. Return the first
	 * that succeeds.
	 */
	private async negotiateReferenceSpace(
		session: XRSessionCompat,
		preferred: string[],
	): Promise<{ space: XRReferenceSpaceCompat; type: string }> {
		const errors: string[] = [];

		for (const type of preferred) {
			try {
				const space = await session.requestReferenceSpace(type);
				return { space, type };
			} catch (err) {
				const msg = err instanceof Error ? err.message : String(err);
				errors.push(`"${type}": ${msg}`);
			}
		}

		throw new Error(
			"XRSessionManager: no supported reference space found. Tried: " +
				errors.join("; "),
		);
	}

	// ── Internal: Frame Loop ──────────────────────────────────────────

	/** Schedule the next XR animation frame. */
	private scheduleFrame(): void {
		if (!this._session || this._state !== SessionState.Active) return;

		this._frameHandle = this._session.requestAnimationFrame(
			(time: number, frame: XRFrameCompat) => {
				this._frameHandle = null;
				this.onXRFrame(time, frame);
			},
		);
	}

	/**
	 * XR frame callback — invoked once per XR display refresh.
	 *
	 * Gets the viewer pose, then delegates to the user's `onFrame` callback
	 * for actual rendering. Schedules the next frame at the end.
	 */
	private onXRFrame(time: number, frame: XRFrameCompat): void {
		// Guard against frames arriving after session end
		if (this._state !== SessionState.Active) return;
		if (!this._session || !this._refSpace || !this._gl || !this._glLayer) {
			return;
		}

		// Get the viewer pose for this frame
		let viewerPose: XRViewerPoseCompat | null = null;
		try {
			viewerPose = frame.getViewerPose(this._refSpace);
		} catch {
			// Pose may be unavailable if tracking is lost — this is normal.
		}

		// Delegate to the user's render callback
		if (this.onFrame) {
			try {
				this.onFrame(
					time,
					frame,
					this._refSpace,
					viewerPose,
					this._gl,
					this._glLayer,
				);
			} catch (err) {
				console.error("XRSessionManager: error in onFrame callback:", err);
				const error = err instanceof Error ? err : new Error(String(err));
				this.emit({ type: "error", error });
			}
		}

		// Schedule next frame
		this.scheduleFrame();
	}

	// ── Internal: Session Event Handling ──────────────────────────────

	/** Handle the XR session "end" event. */
	private handleSessionEnd(): void {
		this.cleanupResources();
		this._state = SessionState.Idle;
		this.emit({ type: "session-ended" });
	}

	/**
	 * Add an event listener to the session with tracking for cleanup.
	 */
	private addSessionListener(
		session: XRSessionCompat,
		type: string,
		handler: () => void,
	): void {
		const listener: EventListener = () => handler();
		session.addEventListener(type, listener);
		this._eventListeners.push({ target: session, type, listener });
	}

	/** Remove all tracked session event listeners and release resources. */
	private cleanupResources(): void {
		// Cancel pending frame
		if (this._frameHandle !== null && this._session) {
			try {
				this._session.cancelAnimationFrame(this._frameHandle);
			} catch {
				// Session may already be ended.
			}
			this._frameHandle = null;
		}

		// Remove event listeners
		for (const { target, type, listener } of this._eventListeners) {
			try {
				target.removeEventListener(type, listener);
			} catch {
				// Session may already be ended.
			}
		}
		this._eventListeners = [];

		// Release GL resources
		if (this._gl) {
			// Lose the WebGL context to free GPU resources.
			const loseCtx = this._gl.getExtension("WEBGL_lose_context");
			if (loseCtx) {
				try {
					loseCtx.loseContext();
				} catch {
					// May fail if context already lost.
				}
			}
		}

		// Remove auto-created canvas from DOM
		if (
			this._canvas &&
			this._canvas.style.display === "none" &&
			this._canvas.parentNode
		) {
			this._canvas.parentNode.removeChild(this._canvas);
		}

		// Null out all references
		this._session = null;
		this._refSpace = null;
		this._refSpaceType = null;
		this._gl = null;
		this._glLayer = null;
		this._canvas = null;
	}

	// ── Internal: Event Emission ──────────────────────────────────────

	/** Emit a runtime event to all registered listeners. */
	private emit(event: XRRuntimeEvent): void {
		for (const listener of this._runtimeListeners) {
			try {
				listener(event);
			} catch (err) {
				console.error("XRSessionManager: error in event listener:", err);
			}
		}
	}
}
