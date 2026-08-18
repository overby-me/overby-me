// XR Input Handler — Raycasting, pointer event dispatch, and hover tracking.
//
// Bridges WebXR input sources (controllers, hands) to DOM pointer events
// on XR panels. The flow is:
//
//   1. Each XR frame, extract input source poses (targetRaySpace)
//   2. Build input rays from the pose transforms
//   3. Raycast against all panels via XRPanelManager
//   4. Translate hits to DOM pointer events (mousemove, mouseenter,
//      mouseleave, mousedown, mouseup, click)
//   5. Dispatch events through the panel's EventBridge (which calls
//      into WASM handlers, same as regular web events)
//
// The input handler tracks per-source hover state so it can generate
// mouseenter/mouseleave events when the ray moves between panels (or
// enters/leaves a panel). It also tracks select start/end to synthesize
// mousedown → mouseup → click sequences.
//
// This module does NOT depend on any specific EventBridge implementation.
// Instead, it exposes a callback-based API: the XR runtime wires the
// callbacks to the appropriate dispatch functions.
//
// Usage:
//
//   const input = new XRInputHandler(panelManager);
//   input.onPointerEvent = (panelId, eventName, pixel, handedness) => {
//     // Dispatch to the panel's EventBridge / WASM handler
//   };
//
//   // In the XR frame loop:
//   input.processFrame(frame, refSpace);
//
//   // Wire XR session events:
//   session.addEventListener("select", (e) => input.onSelect(e));
//   session.addEventListener("selectstart", (e) => input.onSelectStart(e));
//   session.addEventListener("selectend", (e) => input.onSelectEnd(e));

import type { XRPanelManager } from "./xr-panel.ts";
import type {
	RaycastHit,
	XRFrameCompat,
	XRHandedness,
	XRInputRay,
	XRInputSourceCompat,
	XRPoseCompat,
	XRReferenceSpaceCompat,
} from "./xr-types.ts";

// ── Pointer Event Types ─────────────────────────────────────────────────────

/** DOM event names synthesized by the XR input handler. */
export const XRPointerEventName = {
	MouseMove: "mousemove",
	MouseEnter: "mouseenter",
	MouseLeave: "mouseleave",
	MouseDown: "mousedown",
	MouseUp: "mouseup",
	Click: "click",
	Focus: "focus",
	Blur: "blur",
} as const;

export type XRPointerEventNameType =
	(typeof XRPointerEventName)[keyof typeof XRPointerEventName];

// ── Pointer Event Callback ──────────────────────────────────────────────────

/**
 * Callback invoked when the XR input handler generates a DOM-like event
 * for a panel.
 *
 * The caller is responsible for translating this into an actual DOM event
 * dispatch (via EventBridge) or WASM handler call.
 *
 * @param panelId    - The panel that received the event.
 * @param eventName  - The DOM event name (e.g. "click", "mousemove").
 * @param pixel      - The hit point in pixel coordinates on the panel.
 * @param handedness - Which controller/hand generated the event.
 */
export type XRPointerEventCallback = (
	panelId: number,
	eventName: XRPointerEventNameType,
	pixel: { x: number; y: number },
	handedness: XRHandedness,
) => void;

// ── Per-Source Hover State ───────────────────────────────────────────────────

/**
 * Tracks the hover state for a single XR input source.
 *
 * We track per-source rather than per-panel because each controller
 * independently generates pointer events. A user might point the left
 * controller at one panel and the right at another.
 */
interface SourceHoverState {
	/** The panel ID currently being hovered, or -1 for none. */
	hoveredPanelId: number;

	/** Last pixel coordinates on the hovered panel. */
	lastPixel: { x: number; y: number };

	/** Whether the select button is currently pressed. */
	selectActive: boolean;

	/** The panel ID where the current select (press) started, or -1. */
	selectStartPanelId: number;

	/** Pixel where the select started (for drag detection). */
	selectStartPixel: { x: number; y: number };

	/** Timestamp of the last hover event (for throttling). */
	lastHoverTime: number;
}

/** Create a fresh hover state. */
function defaultHoverState(): SourceHoverState {
	return {
		hoveredPanelId: -1,
		lastPixel: { x: 0, y: 0 },
		selectActive: false,
		selectStartPanelId: -1,
		selectStartPixel: { x: 0, y: 0 },
		lastHoverTime: 0,
	};
}

// ── Hover Throttle ──────────────────────────────────────────────────────────

/**
 * Minimum interval between mousemove events (ms).
 *
 * WebXR runs at 72–120 Hz; dispatching a mousemove every frame is
 * excessive. Throttle to ~30 Hz to match the texture update rate and
 * reduce WASM dispatch overhead.
 */
const HOVER_THROTTLE_MS = 33;

// ── XRInputHandler ──────────────────────────────────────────────────────────

/**
 * Translates WebXR input source poses into DOM pointer events on XR panels.
 *
 * Responsibilities:
 *   - Extract input rays from XRInputSource.targetRaySpace poses
 *   - Raycast against all panels via XRPanelManager
 *   - Track hover state per input source (enter/leave/move)
 *   - Synthesize click sequences from select events (down → up → click)
 *   - Manage focus transitions when clicking between panels
 *   - Throttle hover events to avoid excessive dispatch
 *
 * The handler does NOT directly manipulate the DOM or call WASM functions.
 * It invokes the `onPointerEvent` callback for each synthetic event,
 * letting the XR runtime wire the dispatch.
 */
export class XRInputHandler {
	/** The panel manager to raycast against. */
	private readonly _panelManager: XRPanelManager;

	/** Per-source hover state, keyed by a source identifier string. */
	private readonly _sourceStates: Map<string, SourceHoverState> = new Map();

	/**
	 * The most recent raycast hit per source (for cursor rendering).
	 * Keyed by source identifier string.
	 */
	private readonly _currentHits: Map<string, RaycastHit> = new Map();

	/**
	 * Callback invoked for each synthetic pointer event.
	 * Must be set before calling `processFrame()`.
	 */
	onPointerEvent: XRPointerEventCallback | null = null;

	/**
	 * Callback invoked when a panel gains focus (via click).
	 * The runtime can use this to update the XRPanelManager's focus.
	 *
	 * @param panelId - The panel that gained focus, or -1 to clear.
	 */
	onFocusChange: ((panelId: number) => void) | null = null;

	constructor(panelManager: XRPanelManager) {
		this._panelManager = panelManager;
	}

	// ── Public API ────────────────────────────────────────────────────

	/**
	 * Process all active input sources for the current XR frame.
	 *
	 * For each input source with a targetRaySpace, extracts the ray,
	 * raycasts against panels, and generates hover events (mousemove,
	 * mouseenter, mouseleave) as appropriate.
	 *
	 * Call this once per XR frame from the frame callback.
	 *
	 * @param frame    - The current XRFrame.
	 * @param refSpace - The active reference space.
	 * @param sources  - The input sources to process (from session.inputSources).
	 */
	processFrame(
		frame: XRFrameCompat,
		refSpace: XRReferenceSpaceCompat,
		sources: Iterable<XRInputSourceCompat>,
	): void {
		const activeSources = new Set<string>();

		for (const source of sources) {
			// Only process tracked-pointer and gaze sources
			if (
				source.targetRayMode !== "tracked-pointer" &&
				source.targetRayMode !== "gaze" &&
				source.targetRayMode !== "screen"
			) {
				continue;
			}

			const sourceId = getSourceId(source);
			activeSources.add(sourceId);

			// Get the ray pose
			let pose: XRPoseCompat | null = null;
			try {
				pose = frame.getPose(source.targetRaySpace, refSpace);
			} catch {
				// Pose unavailable — skip this source.
				continue;
			}

			if (!pose) continue;

			// Build the input ray from the pose
			const ray = poseToRay(pose, source.handedness);

			// Raycast against all panels
			const hit = this._panelManager.raycast(ray);

			// Store the hit for cursor rendering
			if (hit) {
				this._currentHits.set(sourceId, hit);
			} else {
				this._currentHits.delete(sourceId);
			}

			// Process hover state transitions
			this.processHover(sourceId, hit, source.handedness);
		}

		// Clean up state for sources that are no longer active
		// (e.g. controller disconnected)
		for (const sourceId of this._sourceStates.keys()) {
			if (!activeSources.has(sourceId)) {
				this.sourceRemoved(sourceId);
			}
		}
	}

	/**
	 * Handle an XR "selectstart" event (trigger press / pinch start).
	 *
	 * Generates a "mousedown" event on the hovered panel (if any).
	 *
	 * @param source - The XR input source that initiated the select.
	 */
	onSelectStart(source: XRInputSourceCompat): void {
		const sourceId = getSourceId(source);
		const state = this.getOrCreateState(sourceId);

		state.selectActive = true;

		if (state.hoveredPanelId >= 0) {
			state.selectStartPanelId = state.hoveredPanelId;
			state.selectStartPixel = { ...state.lastPixel };

			this.emitPointerEvent(
				state.hoveredPanelId,
				XRPointerEventName.MouseDown,
				state.lastPixel,
				source.handedness,
			);
		}
	}

	/**
	 * Handle an XR "selectend" event (trigger release / pinch end).
	 *
	 * Generates a "mouseup" event. If the release is on the same panel
	 * where the press started (and within a reasonable distance), also
	 * generates a "click" event.
	 *
	 * @param source - The XR input source that ended the select.
	 */
	onSelectEnd(source: XRInputSourceCompat): void {
		const sourceId = getSourceId(source);
		const state = this.getOrCreateState(sourceId);

		if (!state.selectActive) return;
		state.selectActive = false;

		const currentPanel = state.hoveredPanelId;

		if (currentPanel >= 0) {
			// Emit mouseup
			this.emitPointerEvent(
				currentPanel,
				XRPointerEventName.MouseUp,
				state.lastPixel,
				source.handedness,
			);

			// Emit click if press and release are on the same panel
			// and within a reasonable pixel distance (not a drag)
			if (
				currentPanel === state.selectStartPanelId &&
				pixelDistance(state.lastPixel, state.selectStartPixel) < 20
			) {
				this.emitPointerEvent(
					currentPanel,
					XRPointerEventName.Click,
					state.lastPixel,
					source.handedness,
				);

				// Click changes focus to this panel
				this.changeFocus(currentPanel);
			}
		}

		state.selectStartPanelId = -1;
	}

	/**
	 * Handle an XR "select" event (complete select gesture).
	 *
	 * This is a convenience for environments that only fire "select"
	 * (not selectstart + selectend). Generates click directly.
	 *
	 * If selectstart/selectend are also being handled, this can be
	 * skipped to avoid duplicate clicks.
	 *
	 * @param source - The XR input source.
	 */
	onSelect(source: XRInputSourceCompat): void {
		const sourceId = getSourceId(source);
		const state = this.getOrCreateState(sourceId);

		// Only handle if we're not already tracking selectstart/selectend
		if (state.selectActive) return;

		if (state.hoveredPanelId >= 0) {
			this.emitPointerEvent(
				state.hoveredPanelId,
				XRPointerEventName.Click,
				state.lastPixel,
				source.handedness,
			);
			this.changeFocus(state.hoveredPanelId);
		}
	}

	// ── Cursor Query ──────────────────────────────────────────────────

	/**
	 * Get the current raycast hit for a given input source.
	 *
	 * Used by the XR renderer to draw a cursor at the hit point.
	 *
	 * @param source - The XR input source.
	 * @returns The current hit, or null if the source isn't hitting any panel.
	 */
	getCurrentHit(source: XRInputSourceCompat): RaycastHit | null {
		return this._currentHits.get(getSourceId(source)) ?? null;
	}

	/**
	 * Get all current raycast hits (for all active sources).
	 *
	 * @returns An iterable of [sourceId, hit] pairs.
	 */
	getAllCurrentHits(): IterableIterator<[string, RaycastHit]> {
		return this._currentHits.entries();
	}

	/**
	 * Check whether any input source is currently hovering over a panel.
	 */
	get hasActiveHover(): boolean {
		for (const state of this._sourceStates.values()) {
			if (state.hoveredPanelId >= 0) return true;
		}
		return false;
	}

	// ── Cleanup ───────────────────────────────────────────────────────

	/**
	 * Clear all tracking state.
	 *
	 * Emits mouseleave events for any currently hovered panels, then
	 * clears all per-source state. Call this when the XR session ends.
	 */
	reset(): void {
		// Emit leave events for all hovered panels
		for (const [sourceId, state] of this._sourceStates.entries()) {
			if (state.hoveredPanelId >= 0) {
				this.emitPointerEvent(
					state.hoveredPanelId,
					XRPointerEventName.MouseLeave,
					state.lastPixel,
					handednessFromSourceId(sourceId),
				);
			}
		}

		this._sourceStates.clear();
		this._currentHits.clear();
	}

	// ── Internal: Hover Processing ────────────────────────────────────

	/**
	 * Process hover state transitions for a single input source.
	 *
	 * Compares the current raycast result to the tracked hover state
	 * and emits mouseenter/mouseleave/mousemove events as needed.
	 */
	private processHover(
		sourceId: string,
		hit: RaycastHit | null,
		handedness: XRHandedness,
	): void {
		const state = this.getOrCreateState(sourceId);
		const now = performance.now();
		const prevPanelId = state.hoveredPanelId;
		const nextPanelId = hit?.panelId ?? -1;

		// Case 1: Ray moved from one panel to another (or from panel to nothing)
		if (prevPanelId !== nextPanelId) {
			// Leave the previous panel
			if (prevPanelId >= 0) {
				this.emitPointerEvent(
					prevPanelId,
					XRPointerEventName.MouseLeave,
					state.lastPixel,
					handedness,
				);
			}

			// Enter the new panel
			if (nextPanelId >= 0 && hit) {
				state.hoveredPanelId = nextPanelId;
				state.lastPixel = { ...hit.pixel };
				state.lastHoverTime = now;

				this.emitPointerEvent(
					nextPanelId,
					XRPointerEventName.MouseEnter,
					hit.pixel,
					handedness,
				);
			} else {
				state.hoveredPanelId = -1;
			}

			return;
		}

		// Case 2: Ray is still on the same panel — emit throttled mousemove
		if (nextPanelId >= 0 && hit) {
			state.lastPixel = { ...hit.pixel };

			// Throttle mousemove events
			if (now - state.lastHoverTime >= HOVER_THROTTLE_MS) {
				state.lastHoverTime = now;

				this.emitPointerEvent(
					nextPanelId,
					XRPointerEventName.MouseMove,
					hit.pixel,
					handedness,
				);
			}
		}
	}

	/**
	 * Handle removal of an input source (controller disconnected).
	 *
	 * Emits mouseleave if the source was hovering, then cleans up state.
	 */
	private sourceRemoved(sourceId: string): void {
		const state = this._sourceStates.get(sourceId);
		if (!state) return;

		if (state.hoveredPanelId >= 0) {
			this.emitPointerEvent(
				state.hoveredPanelId,
				XRPointerEventName.MouseLeave,
				state.lastPixel,
				handednessFromSourceId(sourceId),
			);
		}

		this._sourceStates.delete(sourceId);
		this._currentHits.delete(sourceId);
	}

	// ── Internal: Focus Management ────────────────────────────────────

	/**
	 * Change focus to a panel.
	 *
	 * Emits blur on the previously focused panel and focus on the new one.
	 */
	private changeFocus(panelId: number): void {
		const currentFocus = this._panelManager.focusedPanelId;

		if (currentFocus === panelId) return;

		// Blur the old panel
		if (currentFocus >= 0) {
			// Use a zero-pixel coordinate for blur events
			this.emitPointerEvent(
				currentFocus,
				XRPointerEventName.Blur,
				{ x: 0, y: 0 },
				"none",
			);
		}

		// Focus the new panel
		this.emitPointerEvent(
			panelId,
			XRPointerEventName.Focus,
			{ x: 0, y: 0 },
			"none",
		);

		// Notify the runtime
		this.onFocusChange?.(panelId);
	}

	// ── Internal: Event Emission ──────────────────────────────────────

	/**
	 * Emit a pointer event via the callback.
	 */
	private emitPointerEvent(
		panelId: number,
		eventName: XRPointerEventNameType,
		pixel: { x: number; y: number },
		handedness: XRHandedness,
	): void {
		if (this.onPointerEvent) {
			try {
				this.onPointerEvent(panelId, eventName, pixel, handedness);
			} catch (err) {
				console.error(
					`XRInputHandler: error in onPointerEvent(panel=${panelId}, ` +
						`event=${eventName}):`,
					err,
				);
			}
		}
	}

	// ── Internal: State Management ────────────────────────────────────

	/**
	 * Get or create the hover state for an input source.
	 */
	private getOrCreateState(sourceId: string): SourceHoverState {
		let state = this._sourceStates.get(sourceId);
		if (!state) {
			state = defaultHoverState();
			this._sourceStates.set(sourceId, state);
		}
		return state;
	}
}

// ── Utility Functions (module-private) ──────────────────────────────────────

/**
 * Compute a stable identifier for an XR input source.
 *
 * WebXR input sources don't have a built-in stable ID. We derive one
 * from the handedness and target ray mode, which is sufficient for
 * typical setups (one left controller + one right controller, or gaze).
 *
 * For edge cases with multiple sources of the same type (unlikely in
 * practice), this may conflate them. A future improvement would use
 * the source's profile string or index in the inputSources array.
 */
function getSourceId(source: XRInputSourceCompat): string {
	return `${source.handedness}:${source.targetRayMode}`;
}

/**
 * Extract handedness from a source ID string.
 *
 * The source ID format is "handedness:targetRayMode" (from getSourceId).
 */
function handednessFromSourceId(sourceId: string): XRHandedness {
	const colon = sourceId.indexOf(":");
	if (colon === -1) return "none";
	const h = sourceId.substring(0, colon);
	if (h === "left" || h === "right") return h;
	return "none";
}

/**
 * Convert an XR pose to an input ray.
 *
 * The targetRaySpace pose's position is the ray origin, and the
 * negative Z axis of the pose's orientation is the ray direction
 * (WebXR convention: rays point along -Z in the target ray space).
 */
function poseToRay(pose: XRPoseCompat, handedness: XRHandedness): XRInputRay {
	const pos = pose.transform.position;
	const matrix = pose.transform.matrix;

	// The ray direction is the negative Z axis of the transform matrix.
	// In a column-major 4×4 matrix, the Z axis is column 2 (indices 8, 9, 10).
	// We negate it because the ray points forward (which is -Z in WebXR).
	const dirX = -matrix[8];
	const dirY = -matrix[9];
	const dirZ = -matrix[10];

	// Normalize the direction (should already be unit length from the
	// rotation matrix, but normalize for safety)
	const len = Math.sqrt(dirX * dirX + dirY * dirY + dirZ * dirZ);
	const invLen = len > 1e-8 ? 1 / len : 0;

	return {
		origin: { x: pos.x, y: pos.y, z: pos.z },
		direction: {
			x: dirX * invLen,
			y: dirY * invLen,
			z: dirZ * invLen,
		},
		handedness,
	};
}

/**
 * Compute the Euclidean distance between two pixel coordinates.
 *
 * Used for click detection: if the distance between selectstart and
 * selectend is small enough, it's a click rather than a drag.
 */
function pixelDistance(
	a: { x: number; y: number },
	b: { x: number; y: number },
): number {
	const dx = a.x - b.x;
	const dy = a.y - b.y;
	return Math.sqrt(dx * dx + dy * dy);
}
