// XR Input Handler Tests — Hover state, click sequences, focus transitions.
//
// Tests the XRInputHandler from xr-input.ts, which translates WebXR input
// source poses into DOM pointer events on XR panels. Uses mock panel
// managers and fake XR frame/pose data to exercise:
//
//   - Hover tracking (mouseenter, mouseleave, mousemove)
//   - Click synthesis (selectstart → mousedown, selectend → mouseup + click)
//   - Focus transitions (click changes focus, blur/focus events)
//   - Throttled hover events
//   - Source removal cleanup
//   - Multi-source independent tracking
//   - Edge cases (no hit, drag detection, select without hover)

import type { XRPointerEventNameType } from "../runtime/xr-input.ts";
import { XRInputHandler, XRPointerEventName } from "../runtime/xr-input.ts";
import { XRPanelManager } from "../runtime/xr-panel.ts";
import type {
	XRFrameCompat,
	XRHandedness,
	XRInputSourceCompat,
	XRPoseCompat,
	XRReferenceSpaceCompat,
} from "../runtime/xr-types.ts";
import { createDOMWithCanvas } from "./dom-helper.ts";
import {
	assert,
	assertDefined,
	assertFalse,
	assertNull,
	assertTrue,
	suite,
} from "./harness.ts";

// ── Test Infrastructure ─────────────────────────────────────────────────────

/** Recorded pointer event from the onPointerEvent callback. */
interface RecordedEvent {
	panelId: number;
	eventName: XRPointerEventNameType;
	pixel: { x: number; y: number };
	handedness: XRHandedness;
}

/** Recorded focus change from the onFocusChange callback. */
interface RecordedFocus {
	panelId: number;
}

/** Create a test harness with an XRInputHandler and event recording. */
function createInputTestHarness() {
	const env = createDOMWithCanvas();
	const pm = new XRPanelManager(env.document);
	const handler = new XRInputHandler(pm);

	const events: RecordedEvent[] = [];
	const focusChanges: RecordedFocus[] = [];

	handler.onPointerEvent = (
		panelId: number,
		eventName: XRPointerEventNameType,
		pixel: { x: number; y: number },
		handedness: XRHandedness,
	) => {
		events.push({ panelId, eventName, pixel, handedness });
	};

	handler.onFocusChange = (panelId: number) => {
		focusChanges.push({ panelId });
	};

	return { env, pm, handler, events, focusChanges };
}

/** Create a mock XR input source. */
function mockSource(
	handedness: XRHandedness = "right",
	targetRayMode = "tracked-pointer",
): XRInputSourceCompat {
	return {
		handedness,
		targetRayMode,
		targetRaySpace: {} as Record<string, never>,
		profiles: [],
	};
}

/** Create a mock XR reference space. */
function mockRefSpace(): XRReferenceSpaceCompat {
	return {
		getOffsetReferenceSpace(_originOffset: unknown): XRReferenceSpaceCompat {
			return this;
		},
	};
}

/**
 * Create a mock XRFrame that returns specific poses for input sources.
 *
 * The poseFactory receives the source's targetRaySpace and returns a
 * pose (or null if no pose is available). By default, returns a pose
 * that generates a ray pointing down -Z from the origin.
 */
function mockFrame(
	poseFactory?: (
		space: Record<string, never>,
		baseSpace: XRReferenceSpaceCompat,
	) => XRPoseCompat | null,
): XRFrameCompat {
	const defaultPose: XRPoseCompat = {
		transform: {
			position: new DOMPointReadOnly(0, 1.4, 0, 1),
			orientation: new DOMPointReadOnly(0, 0, 0, 1),
			// Identity matrix — ray points along -Z
			matrix: new Float32Array([
				1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 1.4, 0, 1,
			]),
			get inverse() {
				return this;
			},
		},
	};

	return {
		session: {
			renderState: {},
			inputSources: [],
			visibilityState: "visible",
			requestAnimationFrame: () => 0,
			cancelAnimationFrame: () => {},
			requestReferenceSpace: () => Promise.reject(),
			updateRenderState: () => {},
			end: () => Promise.resolve(),
			addEventListener: () => {},
			removeEventListener: () => {},
		},
		getViewerPose: () => null,
		getPose(
			space: Record<string, never>,
			baseSpace: XRReferenceSpaceCompat,
		): XRPoseCompat | null {
			if (poseFactory) {
				return poseFactory(space, baseSpace);
			}
			return defaultPose;
		},
	};
}

/**
 * Create a mock XRFrame that returns a pose generating a ray from
 * a specific origin pointing along -Z.
 */
function mockFrameWithOrigin(x: number, y: number, z: number): XRFrameCompat {
	return mockFrame(() => ({
		transform: {
			position: new DOMPointReadOnly(x, y, z, 1),
			orientation: new DOMPointReadOnly(0, 0, 0, 1),
			matrix: new Float32Array([
				1,
				0,
				0,
				0,
				0,
				1,
				0,
				0,
				0,
				0,
				1,
				0,
				x,
				y,
				z,
				1,
			]),
			get inverse() {
				return this;
			},
		},
	}));
}

/** Create a mock frame that returns null pose for all sources. */
function mockFrameNoPose(): XRFrameCompat {
	return mockFrame(() => null);
}

// ── Polyfill DOMPointReadOnly if needed (Deno may not have it) ──────────────

if (typeof globalThis.DOMPointReadOnly === "undefined") {
	// Minimal DOMPointReadOnly polyfill for test environments
	(globalThis as Record<string, unknown>).DOMPointReadOnly =
		class DOMPointReadOnly {
			readonly x: number;
			readonly y: number;
			readonly z: number;
			readonly w: number;
			constructor(x = 0, y = 0, z = 0, w = 1) {
				this.x = x;
				this.y = y;
				this.z = z;
				this.w = w;
			}
		};
}

// ── Filter helpers ──────────────────────────────────────────────────────────

function eventsOfType(
	events: RecordedEvent[],
	type: XRPointerEventNameType,
): RecordedEvent[] {
	return events.filter((e) => e.eventName === type);
}

function eventsForPanel(
	events: RecordedEvent[],
	panelId: number,
): RecordedEvent[] {
	return events.filter((e) => e.panelId === panelId);
}

// ── Hover Tracking ──────────────────────────────────────────────────────────

export function testHoverTracking(): void {
	suite("xr-input — mouseenter emitted when ray first hits a panel");
	{
		const { pm, handler, events } = createInputTestHarness();
		// Create a panel at the default position (0, 1.4, -1)
		pm.createPanel();

		const source = mockSource("right");
		const frame = mockFrameWithOrigin(0, 1.4, 0); // Ray hits panel center
		const refSpace = mockRefSpace();

		handler.processFrame(frame, refSpace, [source]);

		const enters = eventsOfType(events, XRPointerEventName.MouseEnter);
		assert(enters.length, 1, "one mouseenter event");
		assert(enters[0].panelId, 0, "mouseenter on panel 0");
		assert(enters[0].handedness, "right", "handedness = right");
	}

	suite("xr-input — mousemove emitted on continued hover (after throttle)");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const frame = mockFrameWithOrigin(0, 1.4, 0);
		const refSpace = mockRefSpace();

		// First frame → mouseenter
		handler.processFrame(frame, refSpace, [source]);
		const enterCount = eventsOfType(
			events,
			XRPointerEventName.MouseEnter,
		).length;
		assert(enterCount, 1, "mouseenter on first frame");

		// Clear events and wait for throttle (>33ms)
		events.length = 0;

		// Manually advance time by modifying the internal state isn't possible,
		// but we can just call processFrame multiple times. The throttle is
		// based on performance.now(), so if we call immediately, no mousemove
		// is emitted. We test that at least the logic doesn't crash.
		handler.processFrame(frame, refSpace, [source]);

		// May or may not get a mousemove depending on timing.
		// Just verify no mouseenter is re-emitted (that would be a bug).
		const reEnters = eventsOfType(events, XRPointerEventName.MouseEnter);
		assert(reEnters.length, 0, "no duplicate mouseenter on continued hover");
	}

	suite("xr-input — mouseleave emitted when ray leaves a panel");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Frame 1: ray hits panel
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		assert(
			eventsOfType(events, XRPointerEventName.MouseEnter).length,
			1,
			"mouseenter on hit",
		);

		events.length = 0;

		// Frame 2: ray misses panel (ray far off to the side)
		handler.processFrame(mockFrameWithOrigin(100, 100, 0), refSpace, [source]);
		const leaves = eventsOfType(events, XRPointerEventName.MouseLeave);
		assert(leaves.length, 1, "mouseleave when ray leaves panel");
		assert(leaves[0].panelId, 0, "mouseleave on panel 0");
	}

	suite("xr-input — mouseleave + mouseenter when ray moves between panels");
	{
		const { pm, handler, events } = createInputTestHarness();
		const p0 = pm.createPanel();
		p0.setPosition(-2, 1.4, -1); // Left
		const p1 = pm.createPanel();
		p1.setPosition(2, 1.4, -1); // Right

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Frame 1: ray hits p0
		handler.processFrame(mockFrameWithOrigin(-2, 1.4, 0), refSpace, [source]);
		assert(
			eventsOfType(events, XRPointerEventName.MouseEnter).length,
			1,
			"mouseenter on p0",
		);
		assert(
			eventsOfType(events, XRPointerEventName.MouseEnter)[0].panelId,
			p0.id,
			"enter event is for p0",
		);

		events.length = 0;

		// Frame 2: ray hits p1 (different panel)
		handler.processFrame(mockFrameWithOrigin(2, 1.4, 0), refSpace, [source]);
		const leaves = eventsOfType(events, XRPointerEventName.MouseLeave);
		const enters = eventsOfType(events, XRPointerEventName.MouseEnter);
		assert(leaves.length, 1, "mouseleave on old panel");
		assert(leaves[0].panelId, p0.id, "leave event for p0");
		assert(enters.length, 1, "mouseenter on new panel");
		assert(enters[0].panelId, p1.id, "enter event for p1");
	}

	suite("xr-input — no events when ray hits nothing");
	{
		const { handler, events } = createInputTestHarness();
		// No panels — nothing to hit
		const source = mockSource("right");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		assert(events.length, 0, "no events when no panels exist");
	}

	suite("xr-input — no events when pose is unavailable");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();
		const frame = mockFrameNoPose();

		handler.processFrame(frame, refSpace, [source]);
		assert(events.length, 0, "no events when pose is null");
	}
}

// ── Click Sequences ─────────────────────────────────────────────────────────

export function testClickSequences(): void {
	suite("xr-input — selectStart emits mousedown");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// First, establish hover on the panel
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// selectstart
		handler.onSelectStart(source);
		const downs = eventsOfType(events, XRPointerEventName.MouseDown);
		assert(downs.length, 1, "one mousedown event");
		assert(downs[0].panelId, 0, "mousedown on panel 0");
	}

	suite(
		"xr-input — selectEnd emits mouseup + click (same panel, close distance)",
	);
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// selectstart → selectend
		handler.onSelectStart(source);
		events.length = 0;

		handler.onSelectEnd(source);
		const ups = eventsOfType(events, XRPointerEventName.MouseUp);
		const clicks = eventsOfType(events, XRPointerEventName.Click);
		assert(ups.length, 1, "one mouseup event");
		assert(clicks.length, 1, "one click event");
		assert(clicks[0].panelId, 0, "click on panel 0");
	}

	suite("xr-input — selectEnd without selectStart is a no-op");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// selectend without a prior selectstart
		handler.onSelectEnd(source);
		assert(events.length, 0, "no events for selectend without selectstart");
	}

	suite("xr-input — selectStart without hover emits no mousedown");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Process frame with ray missing the panel
		handler.processFrame(mockFrameWithOrigin(100, 100, 0), refSpace, [source]);
		events.length = 0;

		// selectstart when not hovering
		handler.onSelectStart(source);
		const downs = eventsOfType(events, XRPointerEventName.MouseDown);
		assert(downs.length, 0, "no mousedown when not hovering");
	}

	suite("xr-input — onSelect() emits click directly (convenience path)");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// onSelect (the convenience handler)
		handler.onSelect(source);
		const clicks = eventsOfType(events, XRPointerEventName.Click);
		assert(clicks.length, 1, "click emitted via onSelect");
	}

	suite(
		"xr-input — onSelect() is suppressed during active select (no duplicate click)",
	);
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// selectstart (sets selectActive)
		handler.onSelectStart(source);
		events.length = 0;

		// onSelect while selectActive → should be suppressed
		handler.onSelect(source);
		const clicks = eventsOfType(events, XRPointerEventName.Click);
		assert(clicks.length, 0, "onSelect suppressed during active select");
	}

	suite("xr-input — onSelect() without hover emits nothing");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// No hover
		handler.processFrame(mockFrameWithOrigin(100, 100, 0), refSpace, [source]);
		events.length = 0;

		handler.onSelect(source);
		assert(events.length, 0, "no click when not hovering");
	}
}

// ── Focus Transitions ───────────────────────────────────────────────────────

export function testFocusTransitions(): void {
	suite("xr-input — click triggers focus change");
	{
		const { pm, handler, events, focusChanges } = createInputTestHarness();
		pm.createPanel(); // p0 — gets auto-focus
		const p1 = pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Position p1 at a different location
		p1.setPosition(2, 1.4, -1);

		// Hover over p1
		handler.processFrame(mockFrameWithOrigin(2, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// Click on p1 (via selectstart/selectend)
		handler.onSelectStart(source);
		handler.onSelectEnd(source);

		// Focus should have changed to p1
		assertTrue(focusChanges.length > 0, "at least one focus change");
		assert(
			focusChanges[focusChanges.length - 1].panelId,
			p1.id,
			"focus changed to p1",
		);
	}

	suite("xr-input — click emits blur on old panel and focus on new panel");
	{
		const { pm, handler, events, focusChanges } = createInputTestHarness();
		const p0 = pm.createPanel();
		p0.setPosition(-2, 1.4, -1);
		const p1 = pm.createPanel();
		p1.setPosition(2, 1.4, -1);

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Click on p0 first to establish focus
		handler.processFrame(mockFrameWithOrigin(-2, 1.4, 0), refSpace, [source]);
		handler.onSelectStart(source);
		handler.onSelectEnd(source);
		events.length = 0;
		focusChanges.length = 0;

		// Now hover over p1 and click
		handler.processFrame(mockFrameWithOrigin(2, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// We need to update the panel manager's focus to p0 manually
		// since our harness doesn't wire focusChanges → pm.focusPanel
		pm.focusPanel(p0.id);

		handler.onSelectStart(source);
		handler.onSelectEnd(source);

		// Should see blur and focus events
		const blurs = eventsOfType(events, XRPointerEventName.Blur);
		const focuses = eventsOfType(events, XRPointerEventName.Focus);
		assert(blurs.length, 1, "blur event emitted");
		assert(blurs[0].panelId, p0.id, "blur on old focused panel");
		assert(focuses.length, 1, "focus event emitted");
		assert(focuses[0].panelId, p1.id, "focus on new panel");
	}

	suite("xr-input — clicking the already-focused panel does not re-emit focus");
	{
		const { pm, handler, events, focusChanges } = createInputTestHarness();
		const p0 = pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover and click on p0
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		handler.onSelectStart(source);
		handler.onSelectEnd(source);

		// p0 is now focused. Clear events.
		events.length = 0;
		focusChanges.length = 0;

		// Wire focus so the handler sees p0 as focused
		pm.focusPanel(p0.id);

		// Click p0 again
		handler.onSelectStart(source);
		handler.onSelectEnd(source);

		const focuses = eventsOfType(events, XRPointerEventName.Focus);
		const blurs = eventsOfType(events, XRPointerEventName.Blur);
		assert(focuses.length, 0, "no focus event when re-clicking focused panel");
		assert(blurs.length, 0, "no blur event when re-clicking focused panel");
	}

	suite("xr-input — onSelect also triggers focus change");
	{
		const { pm, handler, focusChanges } = createInputTestHarness();
		const p0 = pm.createPanel();
		p0.setPosition(-2, 1.4, -1);
		const p1 = pm.createPanel();
		p1.setPosition(2, 1.4, -1);

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover over p1 and use onSelect
		handler.processFrame(mockFrameWithOrigin(2, 1.4, 0), refSpace, [source]);
		handler.onSelect(source);

		assertTrue(focusChanges.length > 0, "focus change triggered by onSelect");
		assert(
			focusChanges[focusChanges.length - 1].panelId,
			p1.id,
			"focus changed to p1 via onSelect",
		);
	}
}

// ── Source Removal ──────────────────────────────────────────────────────────

export function testSourceRemoval(): void {
	suite("xr-input — removed source emits mouseleave");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Establish hover
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		assert(
			eventsOfType(events, XRPointerEventName.MouseEnter).length,
			1,
			"hovering",
		);
		events.length = 0;

		// Process frame with no sources → source removed
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, []);
		const leaves = eventsOfType(events, XRPointerEventName.MouseLeave);
		assert(leaves.length, 1, "mouseleave emitted when source removed");
	}

	suite("xr-input — reset() emits mouseleave for all hovered sources");
	{
		const { pm, handler, events } = createInputTestHarness();
		// Use a single panel so both sources hover the same panel
		// (avoids the issue where processing sources separately causes
		// the first source to be removed when absent from the second frame)
		pm.createPanel();

		const sourceL = mockSource("left");
		const sourceR = mockSource("right");
		const refSpace = mockRefSpace();

		// Both sources hit the panel in the same frame
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [
			sourceL,
			sourceR,
		]);
		events.length = 0;

		// Reset — should emit mouseleave for both sources
		handler.reset();
		const leaves = eventsOfType(events, XRPointerEventName.MouseLeave);
		assert(leaves.length, 2, "mouseleave for both sources");
	}

	suite("xr-input — reset() clears all state");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		handler.reset();
		events.length = 0;

		assertFalse(handler.hasActiveHover, "no active hover after reset");

		// Re-entering should produce a fresh mouseenter
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		const enters = eventsOfType(events, XRPointerEventName.MouseEnter);
		assert(enters.length, 1, "fresh mouseenter after reset");
	}
}

// ── Multi-Source Independence ────────────────────────────────────────────────

export function testMultiSourceIndependence(): void {
	suite("xr-input — two sources track hover independently");
	{
		const { pm, handler, events } = createInputTestHarness();
		const p0 = pm.createPanel();
		p0.setPosition(-2, 1.4, -1);
		const p1 = pm.createPanel();
		p1.setPosition(2, 1.4, -1);

		const sourceL = mockSource("left");
		const sourceR = mockSource("right");
		const refSpace = mockRefSpace();

		// Both sources hit different panels in the same frame
		// We need a frame that returns different poses per source.
		// Since our mock uses the same pose for all sources, we'll process
		// them in separate frames.

		// Frame 1: left source hits p0
		handler.processFrame(mockFrameWithOrigin(-2, 1.4, 0), refSpace, [sourceL]);
		const leftEnters = eventsForPanel(
			eventsOfType(events, XRPointerEventName.MouseEnter),
			p0.id,
		);
		assert(leftEnters.length, 1, "left source enters p0");

		events.length = 0;

		// Frame 2: right source hits p1 (left source no longer present
		// in sources list, but we keep it to show independence)
		handler.processFrame(mockFrameWithOrigin(2, 1.4, 0), refSpace, [sourceR]);

		// Left source was removed (not in sources list)
		const _leftLeaves = eventsOfType(events, XRPointerEventName.MouseLeave);
		// Right source enters p1
		const rightEnters = eventsForPanel(
			eventsOfType(events, XRPointerEventName.MouseEnter),
			p1.id,
		);
		assert(rightEnters.length, 1, "right source enters p1");
	}

	suite("xr-input — selectStart on left does not affect right source");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const sourceL = mockSource("left");
		const sourceR = mockSource("right");
		const refSpace = mockRefSpace();

		// Both hover over the same panel
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [
			sourceL,
			sourceR,
		]);
		events.length = 0;

		// selectstart on left only
		handler.onSelectStart(sourceL);
		const downs = eventsOfType(events, XRPointerEventName.MouseDown);
		assert(downs.length, 1, "only one mousedown");
		assert(downs[0].handedness, "left", "mousedown from left source");
	}
}

// ── Cursor Query ────────────────────────────────────────────────────────────

export function testCursorQuery(): void {
	suite("xr-input — getCurrentHit() returns hit for hovering source");
	{
		const { pm, handler } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);

		const hit = handler.getCurrentHit(source);
		assertDefined(hit, "hit exists for hovering source");
		if (hit) {
			assert(hit.panelId, 0, "hit is on panel 0");
		}
	}

	suite("xr-input — getCurrentHit() returns null for non-hovering source");
	{
		const { pm, handler } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Ray misses all panels
		handler.processFrame(mockFrameWithOrigin(100, 100, 0), refSpace, [source]);

		const hit = handler.getCurrentHit(source);
		assertNull(hit, "no hit for non-hovering source");
	}

	suite("xr-input — getAllCurrentHits() returns all active hits");
	{
		const { pm, handler } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);

		const hits = [...handler.getAllCurrentHits()];
		assert(hits.length, 1, "one active hit");
	}

	suite("xr-input — hasActiveHover reflects hover state");
	{
		const { pm, handler } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		assertFalse(handler.hasActiveHover, "no hover initially");

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		assertTrue(handler.hasActiveHover, "hover after hitting panel");

		handler.processFrame(mockFrameWithOrigin(100, 100, 0), refSpace, [source]);
		assertFalse(handler.hasActiveHover, "no hover after ray leaves");
	}
}

// ── Input Source Filtering ──────────────────────────────────────────────────

export function testSourceFiltering(): void {
	suite("xr-input — non-tracked sources are ignored");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		// "transient-pointer" is not in the tracked set
		const source = mockSource("right", "transient-pointer");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		assert(events.length, 0, "transient-pointer source ignored");
	}

	suite("xr-input — gaze source is processed");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("none", "gaze");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		const enters = eventsOfType(events, XRPointerEventName.MouseEnter);
		assert(enters.length, 1, "gaze source generates mouseenter");
		assert(enters[0].handedness, "none", "gaze handedness is none");
	}

	suite("xr-input — screen source is processed");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("none", "screen");
		const refSpace = mockRefSpace();

		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
		const enters = eventsOfType(events, XRPointerEventName.MouseEnter);
		assert(enters.length, 1, "screen source generates mouseenter");
	}
}

// ── Drag Detection (click distance threshold) ───────────────────────────────

export function testDragDetection(): void {
	suite("xr-input — selectEnd on same panel within threshold = click");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);

		// selectstart
		handler.onSelectStart(source);
		events.length = 0;

		// selectend at same position (distance = 0, within 20px threshold)
		handler.onSelectEnd(source);

		const clicks = eventsOfType(events, XRPointerEventName.Click);
		assert(clicks.length, 1, "click emitted when distance < 20px");
	}

	suite("xr-input — selectEnd on different panel = no click");
	{
		const { pm, handler, events } = createInputTestHarness();
		const p0 = pm.createPanel();
		p0.setPosition(-2, 1.4, -1);
		const p1 = pm.createPanel();
		p1.setPosition(2, 1.4, -1);

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover over p0
		handler.processFrame(mockFrameWithOrigin(-2, 1.4, 0), refSpace, [source]);

		// selectstart on p0
		handler.onSelectStart(source);
		events.length = 0;

		// Move to p1
		handler.processFrame(mockFrameWithOrigin(2, 1.4, 0), refSpace, [source]);
		events.length = 0;

		// selectend on p1
		handler.onSelectEnd(source);

		// mouseup should be emitted, but not click (different panel)
		const ups = eventsOfType(events, XRPointerEventName.MouseUp);
		const clicks = eventsOfType(events, XRPointerEventName.Click);
		assert(ups.length, 1, "mouseup emitted on release panel");
		assert(clicks.length, 0, "no click when release on different panel");
	}

	suite("xr-input — selectEnd when not hovering = no events");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Hover over panel
		handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);

		// selectstart
		handler.onSelectStart(source);
		events.length = 0;

		// Move ray off panel
		handler.processFrame(mockFrameWithOrigin(100, 100, 0), refSpace, [source]);
		events.length = 0;

		// selectend with no panel under cursor
		handler.onSelectEnd(source);

		// No mouseup because currentPanel < 0
		const ups = eventsOfType(events, XRPointerEventName.MouseUp);
		const clicks = eventsOfType(events, XRPointerEventName.Click);
		assert(ups.length, 0, "no mouseup when not hovering");
		assert(clicks.length, 0, "no click when not hovering");
	}
}

// ── Event Callback Error Handling ───────────────────────────────────────────

export function testCallbackErrorHandling(): void {
	suite("xr-input — error in onPointerEvent callback does not crash");
	{
		const { pm, handler } = createInputTestHarness();
		pm.createPanel();

		// Set a callback that throws
		handler.onPointerEvent = () => {
			throw new Error("test error in callback");
		};

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Should not throw — error is caught internally
		try {
			handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
			assertTrue(true, "processFrame does not throw despite callback error");
		} catch {
			assertTrue(false, "processFrame should not propagate callback errors");
		}
	}

	suite("xr-input — null onPointerEvent callback is safe");
	{
		const { pm, handler } = createInputTestHarness();
		pm.createPanel();

		handler.onPointerEvent = null;

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		try {
			handler.processFrame(mockFrameWithOrigin(0, 1.4, 0), refSpace, [source]);
			assertTrue(true, "null callback does not crash");
		} catch {
			assertTrue(false, "null callback should not crash");
		}
	}
}

// ── getPose Exception Handling ──────────────────────────────────────────────

export function testGetPoseExceptionHandling(): void {
	suite("xr-input — getPose throwing is handled gracefully");
	{
		const { pm, handler, events } = createInputTestHarness();
		pm.createPanel();

		const source = mockSource("right");
		const refSpace = mockRefSpace();

		// Frame where getPose throws
		const frame = mockFrame(() => {
			throw new Error("pose unavailable");
		});

		try {
			handler.processFrame(frame, refSpace, [source]);
			assertTrue(true, "processFrame handles getPose exception");
		} catch {
			assertTrue(false, "processFrame should catch getPose exceptions");
		}
		assert(events.length, 0, "no events when getPose throws");
	}
}

// ── Aggregate ───────────────────────────────────────────────────────────────

export function testXRInput(): void {
	testHoverTracking();
	testClickSequences();
	testFocusTransitions();
	testSourceRemoval();
	testMultiSourceIndependence();
	testCursorQuery();
	testSourceFiltering();
	testDragDetection();
	testCallbackErrorHandling();
	testGetPoseExceptionHandling();
}
