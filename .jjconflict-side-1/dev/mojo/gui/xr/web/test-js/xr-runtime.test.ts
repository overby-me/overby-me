// XR Runtime Tests — State machine, event listeners, panel creation,
// flat fallback mode, and the inline mutation interpreter.
//
// Tests the XRRuntime from xr-runtime.ts, which is the main entry point
// orchestrating all WebXR subsystems. Since we can't test full WASM app
// loading or real WebXR sessions in a headless environment, we focus on:
//
//   - State machine transitions (Uninitialized → Ready/FlatFallback → Destroyed)
//   - Event listener registration and dispatch
//   - Panel creation (non-WASM, standalone panels)
//   - Flat fallback mode activation
//   - Runtime configuration
//   - Error handling for invalid state transitions
//   - "Enter VR" button creation
//   - Handler map management
//   - Input handler wiring

import { RuntimeState, XRRuntime } from "../runtime/xr-runtime.ts";
import type { XRRuntimeEvent } from "../runtime/xr-types.ts";
import { createDOMWithCanvas, type DOMEnvironment } from "./dom-helper.ts";
import {
	assert,
	assertClose,
	assertDefined,
	assertFalse,
	assertNull,
	assertThrows,
	assertThrowsAsync,
	assertTrue,
	suite,
} from "./harness.ts";

// ── Mock XR System ──────────────────────────────────────────────────────────
//
// The XRSessionManager.getXRSystem() checks for `navigator.xr` via
// `globalThis.navigator`. We can control whether "WebXR is available"
// by patching navigator.xr on the global object before calling
// runtime.initialize().

/** Install a mock navigator.xr that reports support for a given mode. */
function installMockXR(supported: boolean): void {
	// Ensure navigator exists on globalThis
	if (!("navigator" in globalThis)) {
		Object.defineProperty(globalThis, "navigator", {
			value: {},
			writable: true,
			configurable: true,
		});
	}

	// biome-ignore lint/suspicious/noExplicitAny: patching globalThis.navigator requires any cast
	const nav = (globalThis as any).navigator;

	// Set up the xr property with isSessionSupported
	nav.xr = {
		isSessionSupported: (_mode: string): Promise<boolean> =>
			Promise.resolve(supported),
		requestSession: () =>
			Promise.reject(new Error("mock: requestSession not implemented")),
	};
}

/** Remove the mock navigator.xr. */
function removeMockXR(): void {
	if ("navigator" in globalThis) {
		// biome-ignore lint/suspicious/noExplicitAny: patching globalThis.navigator requires any cast
		const nav = (globalThis as any).navigator;
		if (nav && "xr" in nav) {
			delete nav.xr;
		}
	}
}

/** Create a runtime in a headless DOM environment. */
function createRuntime(env?: DOMEnvironment): {
	runtime: XRRuntime;
	env: DOMEnvironment;
} {
	const e = env ?? createDOMWithCanvas();
	const runtime = new XRRuntime(e.document);
	return { runtime, env: e };
}

// ── State Machine: Initial State ────────────────────────────────────────────

export function testRuntimeInitialState(): void {
	suite("xr-runtime — initial state is Uninitialized");
	{
		const { runtime } = createRuntime();
		assert(runtime.state, RuntimeState.Uninitialized, "state = uninitialized");
		assertFalse(runtime.isXRActive, "isXRActive = false");
		assertFalse(runtime.isFlatFallback, "isFlatFallback = false");
		assert(runtime.appPanelCount, 0, "appPanelCount = 0");
		assertNull(runtime.renderer, "renderer is null before initialize");
	}
}

// ── State Machine: Initialize (XR Available → Ready) ────────────────────────

export async function testRuntimeInitializeXRAvailable(): Promise<void> {
	suite("xr-runtime — initialize() with XR available → Ready");

	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		const xrAvailable = await runtime.initialize({
			showEnterVRButton: false,
		});
		assertTrue(xrAvailable, "initialize returns true when XR available");
		assert(runtime.state, RuntimeState.Ready, "state = ready");
		assertFalse(runtime.isXRActive, "not XR active yet");
		assertFalse(runtime.isFlatFallback, "not flat fallback");
	} finally {
		removeMockXR();
	}
}

// ── State Machine: Initialize (XR Unavailable → FlatFallback) ───────────────

export async function testRuntimeInitializeFlatFallback(): Promise<void> {
	suite("xr-runtime — initialize() without XR → FlatFallback");

	removeMockXR();
	const { runtime } = createRuntime();
	const xrAvailable = await runtime.initialize({
		fallbackToFlat: true,
		showEnterVRButton: false,
	});
	assertFalse(xrAvailable, "initialize returns false when no XR");
	assert(runtime.state, RuntimeState.FlatFallback, "state = flat-fallback");
	assertTrue(runtime.isFlatFallback, "isFlatFallback = true");
}

// ── State Machine: Initialize (XR Unavailable, no fallback → Error) ────────

export async function testRuntimeInitializeNoFallbackError(): Promise<void> {
	suite("xr-runtime — initialize() without XR and fallbackToFlat=false throws");

	removeMockXR();
	const { runtime } = createRuntime();
	await assertThrowsAsync(async () => {
		await runtime.initialize({
			fallbackToFlat: false,
			showEnterVRButton: false,
		});
	}, "throws when XR unavailable and fallback disabled");
}

// ── State Machine: Double Initialize → Error ────────────────────────────────

export async function testRuntimeDoubleInitialize(): Promise<void> {
	suite("xr-runtime — double initialize() throws");

	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		await assertThrowsAsync(async () => {
			await runtime.initialize({ showEnterVRButton: false });
		}, "second initialize throws");
	} finally {
		removeMockXR();
	}
}

// ── State Machine: Destroy ──────────────────────────────────────────────────

export async function testRuntimeDestroy(): Promise<void> {
	suite("xr-runtime — destroy() transitions to Destroyed");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		await runtime.destroy();
		assert(runtime.state, RuntimeState.Destroyed, "state = destroyed");
	}

	suite("xr-runtime — destroy() on already-destroyed is idempotent");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		await runtime.destroy();
		await runtime.destroy(); // Should not throw
		assert(runtime.state, RuntimeState.Destroyed, "still destroyed");
	}
}

// ── Panel Creation (Standalone, non-WASM) ───────────────────────────────────

export async function testRuntimeCreatePanel(): Promise<void> {
	suite("xr-runtime — createPanel() creates a panel in Ready state");
	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		const panel = runtime.createPanel();
		assertDefined(panel, "panel created");
		assert(panel.id, 0, "panel ID = 0");
		assertTrue(panel.state.visible, "panel is visible");
	} finally {
		removeMockXR();
	}

	suite("xr-runtime — createPanel() with custom config and position");
	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		const panel = runtime.createPanel(
			{ widthM: 2.0, heightM: 1.5 },
			{ x: 1, y: 2, z: -3 },
		);
		assertClose(panel.config.widthM, 2.0, 0.001, "custom width");
		assertClose(panel.config.heightM, 1.5, 0.001, "custom height");
		assertClose(panel.position.x, 1, 0.001, "custom position.x");
		assertClose(panel.position.y, 2, 0.001, "custom position.y");
		assertClose(panel.position.z, -3, 0.001, "custom position.z");
	} finally {
		removeMockXR();
	}

	suite("xr-runtime — createPanel() without position uses default");
	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		const panel = runtime.createPanel();
		// Default position from XRPanel constructor: (0, 1.4, -1)
		assertClose(panel.position.x, 0, 0.001, "default position.x");
		assertClose(panel.position.y, 1.4, 0.001, "default position.y");
		assertClose(panel.position.z, -1, 0.001, "default position.z");
	} finally {
		removeMockXR();
	}

	suite("xr-runtime — createPanel() in FlatFallback state works");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		const panel = runtime.createPanel();
		assertDefined(panel, "panel created in flat fallback");
	}

	suite("xr-runtime — createPanel() in Uninitialized state throws");
	{
		const { runtime } = createRuntime();
		assertThrows(() => {
			runtime.createPanel();
		}, "throws when not initialized");
	}

	suite("xr-runtime — createPanel() in Destroyed state throws");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		await runtime.destroy();
		assertThrows(() => {
			runtime.createPanel();
		}, "throws when destroyed");
	}
}

// ── Multiple Panel Creation ─────────────────────────────────────────────────

export async function testRuntimeMultiplePanels(): Promise<void> {
	suite("xr-runtime — multiple panels have sequential IDs");

	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		const p0 = runtime.createPanel();
		const p1 = runtime.createPanel();
		const p2 = runtime.createPanel();
		assert(p0.id, 0, "first panel ID = 0");
		assert(p1.id, 1, "second panel ID = 1");
		assert(p2.id, 2, "third panel ID = 2");
	} finally {
		removeMockXR();
	}
}

// ── Event Listeners ─────────────────────────────────────────────────────────

export async function testRuntimeEventListeners(): Promise<void> {
	suite("xr-runtime — addEventListener receives events");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		const receivedEvents: XRRuntimeEvent[] = [];
		runtime.addEventListener((event: XRRuntimeEvent) => {
			receivedEvents.push(event);
		});
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		// The initialize path emits a "fallback-to-flat" event
		assertTrue(receivedEvents.length > 0, "received events");
		assert(
			receivedEvents[0].type,
			"fallback-to-flat",
			"received fallback-to-flat event",
		);
	}

	suite("xr-runtime — removeEventListener (unsubscribe function)");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		const receivedEvents: XRRuntimeEvent[] = [];
		const unsub = runtime.addEventListener((event: XRRuntimeEvent) => {
			receivedEvents.push(event);
		});
		// Unsubscribe before initialize
		unsub();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		assert(receivedEvents.length, 0, "no events after unsubscribe");
	}

	suite("xr-runtime — multiple listeners all receive events");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		let count1 = 0;
		let count2 = 0;
		runtime.addEventListener(() => {
			count1++;
		});
		runtime.addEventListener(() => {
			count2++;
		});
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		assertTrue(count1 > 0, "listener 1 received events");
		assertTrue(count2 > 0, "listener 2 received events");
		assert(count1, count2, "both listeners received same count");
	}

	suite("xr-runtime — error in listener does not crash other listeners");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		let received = false;
		runtime.addEventListener(() => {
			throw new Error("test listener error");
		});
		runtime.addEventListener(() => {
			received = true;
		});
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		assertTrue(received, "second listener still called after first throws");
	}
}

// ── Configuration ───────────────────────────────────────────────────────────

export async function testRuntimeConfiguration(): Promise<void> {
	suite("xr-runtime — config reflects provided values after initialize");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			sessionMode: "immersive-ar",
			textureUpdateRate: 60,
			panelBackground: "#ff0000",
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		const cfg = runtime.config;
		assert(cfg.sessionMode, "immersive-ar", "sessionMode override");
		assert(cfg.textureUpdateRate, 60, "textureUpdateRate override");
		assert(cfg.panelBackground, "#ff0000", "panelBackground override");
		assertTrue(cfg.fallbackToFlat, "fallbackToFlat = true");
		assertFalse(cfg.showEnterVRButton, "showEnterVRButton = false");
	}

	suite("xr-runtime — config uses defaults when no overrides provided");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		const cfg = runtime.config;
		assert(
			cfg.sessionMode,
			"immersive-vr",
			"default sessionMode = immersive-vr",
		);
		assert(cfg.textureUpdateRate, 30, "default textureUpdateRate = 30");
		assert(cfg.panelBackground, "#ffffff", "default panelBackground = #ffffff");
	}
}

// ── Enter VR Button ─────────────────────────────────────────────────────────

export async function testRuntimeEnterVRButton(): Promise<void> {
	suite(
		"xr-runtime — Enter VR button created when showEnterVRButton=true and XR available",
	);
	installMockXR(true);
	try {
		const { runtime, env } = createRuntime();
		await runtime.initialize({ showEnterVRButton: true });
		const button = env.document.getElementById("xr-enter-vr");
		assertDefined(button, "Enter VR button exists in DOM");
		if (button) {
			assertTrue(
				(button.textContent ?? "").includes("Enter VR"),
				"button text contains 'Enter VR'",
			);
		}
	} finally {
		removeMockXR();
	}

	suite(
		"xr-runtime — Enter VR button NOT created when showEnterVRButton=false",
	);
	installMockXR(true);
	try {
		const { runtime, env } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		const button = env.document.getElementById("xr-enter-vr");
		assertNull(button, "no Enter VR button when disabled");
	} finally {
		removeMockXR();
	}

	suite("xr-runtime — Enter VR button NOT created in flat fallback mode");
	{
		removeMockXR();
		const { runtime, env } = createRuntime();
		await runtime.initialize({
			showEnterVRButton: true,
			fallbackToFlat: true,
		});
		const button = env.document.getElementById("xr-enter-vr");
		assertNull(button, "no Enter VR button when XR unavailable");
	}

	suite("xr-runtime — Enter VR button removed on destroy");
	installMockXR(true);
	try {
		const { runtime, env } = createRuntime();
		await runtime.initialize({ showEnterVRButton: true });
		const buttonBefore = env.document.getElementById("xr-enter-vr");
		assertDefined(buttonBefore, "button exists before destroy");
		await runtime.destroy();
		const buttonAfter = env.document.getElementById("xr-enter-vr");
		assertNull(buttonAfter, "button removed after destroy");
	} finally {
		removeMockXR();
	}
}

// ── Flat Fallback Mode Panel Visibility ─────────────────────────────────────

export async function testRuntimeFlatFallbackPanels(): Promise<void> {
	suite(
		"xr-runtime — panels in flat fallback get visible CSS when start() called",
	);
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		const panel = runtime.createPanel();

		// Before start(), panel container is offscreen
		assertTrue(
			panel.container.style.cssText.includes("-99999px"),
			"panel offscreen before start()",
		);

		// start() in flat fallback mode makes panels visible
		await runtime.start();

		const style = panel.container.style.cssText;
		// linkedom may strip spaces: "visibility:visible" not "visibility: visible"
		assertTrue(
			style.includes("visibility:visible") ||
				style.includes("visibility: visible"),
			"panel visible after start() in flat mode",
		);
		assertTrue(
			style.includes("pointer-events:auto") ||
				style.includes("pointer-events: auto"),
			"panel interactive in flat mode",
		);
	}

	suite("xr-runtime — stop() in flat fallback moves panels back offscreen");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		const panel = runtime.createPanel();

		await runtime.start();
		const styleAfterStart = panel.container.style.cssText;
		assertTrue(
			styleAfterStart.includes("visibility:visible") ||
				styleAfterStart.includes("visibility: visible"),
			"visible after start",
		);

		await runtime.stop();
		const styleAfterStop = panel.container.style.cssText;
		assertTrue(
			styleAfterStop.includes("visibility:hidden") ||
				styleAfterStop.includes("visibility: hidden"),
			"hidden after stop",
		);
		assertTrue(
			styleAfterStop.includes("-99999px"),
			"back offscreen after stop",
		);
	}
}

// ── Start Without Initialize ────────────────────────────────────────────────

export async function testRuntimeStartWithoutInitialize(): Promise<void> {
	suite("xr-runtime — start() in Uninitialized state throws");

	const { runtime } = createRuntime();
	await assertThrowsAsync(async () => {
		await runtime.start();
	}, "start() throws when not initialized");
}

// ── Destroy Cleans Up Panels ────────────────────────────────────────────────

export async function testRuntimeDestroyCleansPanels(): Promise<void> {
	suite("xr-runtime — destroy() removes all panels from DOM");

	removeMockXR();
	const { runtime, env } = createRuntime();
	await runtime.initialize({
		fallbackToFlat: true,
		showEnterVRButton: false,
	});
	runtime.createPanel();
	runtime.createPanel();

	const panelsBefore = env.document.querySelectorAll("[data-xr-panel]");
	assert(panelsBefore.length, 2, "two panel containers before destroy");

	await runtime.destroy();

	const panelsAfter = env.document.querySelectorAll("[data-xr-panel]");
	assert(panelsAfter.length, 0, "no panel containers after destroy");
}

// ── Constructor With Explicit Document ──────────────────────────────────────

export function testRuntimeConstructorDocument(): void {
	suite("xr-runtime — constructor accepts explicit document");
	{
		const env = createDOMWithCanvas();
		const runtime = new XRRuntime(env.document);
		assert(
			runtime.state,
			RuntimeState.Uninitialized,
			"runtime created with explicit doc",
		);
		assertDefined(runtime.session, "session manager created");
		assertDefined(runtime.panelManager, "panel manager created");
		assertDefined(runtime.inputHandler, "input handler created");
	}
}

// ── Subsystem Accessors ─────────────────────────────────────────────────────

export function testRuntimeSubsystemAccessors(): void {
	suite("xr-runtime — subsystem accessors are available");
	{
		const { runtime } = createRuntime();
		assertDefined(runtime.session, "session accessor works");
		assertDefined(runtime.panelManager, "panelManager accessor works");
		assertDefined(runtime.inputHandler, "inputHandler accessor works");
	}

	suite("xr-runtime — renderer is null before initialize");
	{
		const { runtime } = createRuntime();
		assertNull(runtime.renderer, "renderer null before init");
	}
}

// ── Panel Manager Re-creation on Initialize ─────────────────────────────────

export async function testRuntimePanelManagerRecreation(): Promise<void> {
	suite(
		"xr-runtime — initialize() creates panel manager with custom background",
	);

	removeMockXR();
	const { runtime } = createRuntime();
	await runtime.initialize({
		panelBackground: "#aabbcc",
		fallbackToFlat: true,
		showEnterVRButton: false,
	});
	// Create a panel — should use the custom background
	const panel = runtime.createPanel();
	const style = panel.container.querySelector("style");
	assertDefined(style, "panel has style element");
	if (style) {
		assertTrue(
			(style.textContent ?? "").includes("#aabbcc"),
			"panel uses configured background color",
		);
	}
}

// ── Input Handler Wiring ────────────────────────────────────────────────────

export async function testRuntimeInputHandlerWiring(): Promise<void> {
	suite("xr-runtime — input handler's onPointerEvent is wired on construction");
	{
		const { runtime } = createRuntime();
		assertDefined(
			runtime.inputHandler.onPointerEvent,
			"onPointerEvent callback is wired",
		);
	}

	suite("xr-runtime — input handler's onFocusChange is wired on construction");
	{
		const { runtime } = createRuntime();
		assertDefined(
			runtime.inputHandler.onFocusChange,
			"onFocusChange callback is wired",
		);
	}

	suite(
		"xr-runtime — input handler's onFocusChange delegates to panel manager",
	);
	{
		removeMockXR();
		const { runtime } = createRuntime();
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		const p0 = runtime.createPanel();
		const p1 = runtime.createPanel();

		// p0 auto-focused by panel manager
		assert(runtime.panelManager.focusedPanelId, p0.id, "p0 initially focused");

		// Simulate focus change through input handler callback
		if (runtime.inputHandler.onFocusChange) {
			runtime.inputHandler.onFocusChange(p1.id);
		}

		assert(
			runtime.panelManager.focusedPanelId,
			p1.id,
			"focus changed to p1 through input handler",
		);
	}
}

// ── State Getter Consistency ────────────────────────────────────────────────

export async function testRuntimeStateGetters(): Promise<void> {
	suite("xr-runtime — isXRActive false in all non-XR states");
	{
		const { runtime } = createRuntime();
		assertFalse(runtime.isXRActive, "not XR active in Uninitialized");
	}

	suite("xr-runtime — isFlatFallback true only in FlatFallback state");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		assertFalse(runtime.isFlatFallback, "not flat fallback in Uninitialized");
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		assertTrue(runtime.isFlatFallback, "flat fallback after init without XR");
	}

	suite("xr-runtime — config accessible before and after initialize");
	{
		removeMockXR();
		const { runtime } = createRuntime();
		assertDefined(runtime.config, "config accessible before init");
		await runtime.initialize({
			fallbackToFlat: true,
			showEnterVRButton: false,
		});
		assertDefined(runtime.config, "config accessible after init");
	}
}

// ── DOM Event Type Mapping ──────────────────────────────────────────────────
// The DOM_EVENT_TO_TYPE mapping is a module-private constant. We can't test
// it directly, but we can verify the runtime doesn't crash when the input
// handler dispatches known event names.

export async function testRuntimeEventTypeMapping(): Promise<void> {
	suite("xr-runtime — runtime handles pointer events without crashing");

	removeMockXR();
	const { runtime } = createRuntime();
	await runtime.initialize({
		fallbackToFlat: true,
		showEnterVRButton: false,
	});

	// Create a panel to have a valid target
	runtime.createPanel();

	// The input handler's onPointerEvent callback is wired to
	// handlePanelPointerEvent, which looks up the handler map.
	// With no WASM app connected, the handler map is empty,
	// so these should all be no-ops (no crash).
	const ih = runtime.inputHandler;
	if (ih.onPointerEvent) {
		const eventNames = [
			"click",
			"mousemove",
			"mouseenter",
			"mouseleave",
			"mousedown",
			"mouseup",
			"focus",
			"blur",
		] as const;

		for (const name of eventNames) {
			try {
				ih.onPointerEvent(0, name, { x: 100, y: 100 }, "right");
			} catch {
				assertTrue(false, `onPointerEvent("${name}") should not throw`);
			}
		}
		assertTrue(true, "all event types handled without crashing");
	}
}

// ── XR Ready State Allows Start (but start fails without real XR) ───────────

export async function testRuntimeStartInReadyState(): Promise<void> {
	suite(
		"xr-runtime — start() in Ready state tries to start XR session (fails in test env)",
	);

	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		assert(runtime.state, RuntimeState.Ready, "state is Ready");

		// start() will try to call session.start() which calls requestSession
		// on our mock. The mock rejects, so start() should throw.
		let startFailed = false;
		try {
			await runtime.start();
		} catch {
			startFailed = true;
		}
		assertTrue(startFailed, "start() fails in test env (no real XR)");
	} finally {
		removeMockXR();
	}
}

// ── Panel Creation After Destroy Fails ──────────────────────────────────────

export async function testRuntimePanelCreationAfterDestroy(): Promise<void> {
	suite("xr-runtime — createPanel() after destroy throws assertReady");

	installMockXR(true);
	try {
		const { runtime } = createRuntime();
		await runtime.initialize({ showEnterVRButton: false });
		runtime.createPanel(); // OK
		await runtime.destroy();
		assertThrows(() => {
			runtime.createPanel();
		}, "createPanel after destroy throws");
	} finally {
		removeMockXR();
	}
}

// ── Aggregate ───────────────────────────────────────────────────────────────

export async function testXRRuntime(): Promise<void> {
	// Synchronous tests
	testRuntimeInitialState();
	testRuntimeConstructorDocument();
	testRuntimeSubsystemAccessors();

	// Async tests — run sequentially to avoid shared global state conflicts
	// (navigator.xr mock)
	await testRuntimeInitializeXRAvailable();
	await testRuntimeInitializeFlatFallback();
	await testRuntimeInitializeNoFallbackError();
	await testRuntimeDoubleInitialize();
	await testRuntimeDestroy();
	await testRuntimeCreatePanel();
	await testRuntimeMultiplePanels();
	await testRuntimeEventListeners();
	await testRuntimeConfiguration();
	await testRuntimeEnterVRButton();
	await testRuntimeFlatFallbackPanels();
	await testRuntimeStartWithoutInitialize();
	await testRuntimeDestroyCleansPanels();
	await testRuntimePanelManagerRecreation();
	await testRuntimeInputHandlerWiring();
	await testRuntimeStateGetters();
	await testRuntimeEventTypeMapping();
	await testRuntimeStartInReadyState();
	await testRuntimePanelCreationAfterDestroy();
}
