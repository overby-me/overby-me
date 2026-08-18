// XR Panel Tests — Panel creation, transforms, raycasting, model matrix,
// panel manager lifecycle, focus management, and spatial layout.
//
// Tests the core panel abstraction and panel manager from xr-panel.ts.
// Uses linkedom for headless DOM simulation and canvas stubs for
// texture-related operations.

import { XRPanel, XRPanelManager } from "../runtime/xr-panel.ts";
import type { PanelConfig, Vec3, XRInputRay } from "../runtime/xr-types.ts";
import {
	dashboardPanelConfig,
	defaultPanelConfig,
	tooltipPanelConfig,
} from "../runtime/xr-types.ts";
import {
	createDOMWithCanvas,
	createStubWebGL2,
	type DOMEnvironment,
} from "./dom-helper.ts";
import {
	assert,
	assertClose,
	assertDefined,
	assertFalse,
	assertGreater,
	assertLength,
	assertNull,
	assertTrue,
	suite,
} from "./harness.ts";

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Create an XRPanel with default config in a headless DOM. */
function createTestPanel(
	env?: DOMEnvironment,
	config?: PanelConfig,
	id = 0,
	background = "#ffffff",
): { panel: XRPanel; env: DOMEnvironment } {
	const e = env ?? createDOMWithCanvas();
	const cfg = config ?? defaultPanelConfig();
	const panel = new XRPanel(id, cfg, e.document, background);
	return { panel, env: e };
}

/** Create a ray pointing straight down the -Z axis from a given origin. */
function forwardRay(origin: Vec3 = { x: 0, y: 1.4, z: 0 }): XRInputRay {
	return {
		origin,
		direction: { x: 0, y: 0, z: -1 },
		handedness: "right",
	};
}

/** Create a ray with a specific direction (should be normalized). */
function ray(origin: Vec3, direction: Vec3): XRInputRay {
	return { origin, direction, handedness: "right" };
}

// ── XRPanel Construction ────────────────────────────────────────────────────

export function testPanelConstruction(): void {
	suite("xr-panel — XRPanel constructor basics");
	{
		const { panel } = createTestPanel();
		assert(panel.id, 0, "panel ID is assigned from argument");

		const cfg = defaultPanelConfig();
		assertClose(
			panel.config.widthM,
			cfg.widthM,
			0.001,
			"config.widthM matches",
		);
		assertClose(
			panel.config.heightM,
			cfg.heightM,
			0.001,
			"config.heightM matches",
		);
		assert(
			panel.config.pixelsPerMeter,
			cfg.pixelsPerMeter,
			"config.pixelsPerMeter matches",
		);
		assertFalse(panel.config.curved, "config.curved matches");
		assertTrue(panel.config.interact, "config.interact matches");
	}

	suite("xr-panel — texture dimensions derived from config");
	{
		const { panel } = createTestPanel();
		const expectedW = Math.round(0.8 * 1200);
		const expectedH = Math.round(0.6 * 1200);
		assert(panel.textureWidth, expectedW, `textureWidth = ${expectedW}`);
		assert(panel.textureHeight, expectedH, `textureHeight = ${expectedH}`);
	}

	suite("xr-panel — texture dimensions for dashboard config");
	{
		const cfg = dashboardPanelConfig();
		const { panel } = createTestPanel(undefined, cfg);
		assert(panel.textureWidth, 1600, "dashboard textureWidth = 1600");
		assert(panel.textureHeight, 900, "dashboard textureHeight = 900");
	}

	suite("xr-panel — default position is 1m in front at eye height");
	{
		const { panel } = createTestPanel();
		assertClose(panel.position.x, 0, 0.001, "default position.x = 0");
		assertClose(panel.position.y, 1.4, 0.001, "default position.y = 1.4");
		assertClose(panel.position.z, -1.0, 0.001, "default position.z = -1.0");
	}

	suite("xr-panel — default rotation is identity quaternion");
	{
		const { panel } = createTestPanel();
		assertClose(panel.rotation.x, 0, 0.001, "rotation.x = 0");
		assertClose(panel.rotation.y, 0, 0.001, "rotation.y = 0");
		assertClose(panel.rotation.z, 0, 0.001, "rotation.z = 0");
		assertClose(panel.rotation.w, 1, 0.001, "rotation.w = 1");
	}

	suite("xr-panel — initial state");
	{
		const { panel } = createTestPanel();
		assertTrue(panel.state.visible, "visible on creation");
		assertFalse(panel.state.focused, "not focused on creation");
		assertTrue(panel.state.dirty, "dirty on creation (needs initial render)");
		assertFalse(panel.state.mounted, "not mounted on creation");
	}

	suite("xr-panel — config is frozen (immutable)");
	{
		const { panel } = createTestPanel();
		try {
			// @ts-expect-error — intentionally testing runtime immutability
			panel.config.widthM = 99;
			// In strict mode this would throw; in sloppy mode the assignment is silently ignored
			assertClose(
				panel.config.widthM,
				0.8,
				0.001,
				"config.widthM unchanged after attempted mutation",
			);
		} catch {
			// Strict mode throws TypeError — that's fine too
			assertTrue(true, "config mutation throws in strict mode");
		}
	}
}

// ── XRPanel DOM Container ───────────────────────────────────────────────────

export function testPanelDOMContainer(): void {
	suite("xr-panel — DOM container created and attached");
	{
		const env = createDOMWithCanvas();
		const panel = new XRPanel(7, defaultPanelConfig(), env.document, "#ffffff");
		assertDefined(panel.container, "container element exists");
		assert(
			panel.container.getAttribute("data-xr-panel"),
			"7",
			"container has data-xr-panel attribute with panel ID",
		);
	}

	suite("xr-panel — DOM container appended to body");
	{
		const env = createDOMWithCanvas();
		const _panel = new XRPanel(
			0,
			defaultPanelConfig(),
			env.document,
			"#ffffff",
		);
		const found = env.document.querySelector('[data-xr-panel="0"]');
		assertDefined(found, "container found in document body");
	}

	suite("xr-panel — DOM container is hidden offscreen");
	{
		const { panel } = createTestPanel();
		const style = panel.container.style.cssText;
		// linkedom strips spaces from cssText: "position:absolute" not "position: absolute"
		assertTrue(
			style.includes("position:absolute") ||
				style.includes("position: absolute"),
			"position: absolute",
		);
		assertTrue(style.includes("-99999px"), "positioned offscreen");
		assertTrue(
			style.includes("visibility:hidden") ||
				style.includes("visibility: hidden"),
			"visibility: hidden",
		);
	}

	suite("xr-panel — DOM container has style element for background");
	{
		const env = createDOMWithCanvas();
		const panel = new XRPanel(3, defaultPanelConfig(), env.document, "#ff0000");
		const styleEl = panel.container.querySelector("style");
		assertDefined(styleEl, "style element exists in container");
		assertTrue(
			(styleEl as Element).textContent?.includes("#ff0000") ?? false,
			"style contains background color",
		);
	}
}

// ── XRPanel Transform Helpers ───────────────────────────────────────────────

export function testPanelTransforms(): void {
	suite("xr-panel — setPosition()");
	{
		const { panel } = createTestPanel();
		panel.setPosition(1.5, 2.0, -3.0);
		assertClose(panel.position.x, 1.5, 0.001, "position.x set to 1.5");
		assertClose(panel.position.y, 2.0, 0.001, "position.y set to 2.0");
		assertClose(panel.position.z, -3.0, 0.001, "position.z set to -3.0");
	}

	suite("xr-panel — setRotation() (direct quaternion)");
	{
		const { panel } = createTestPanel();
		panel.setRotation(0.1, 0.2, 0.3, 0.9);
		assertClose(panel.rotation.x, 0.1, 0.001, "rotation.x set");
		assertClose(panel.rotation.y, 0.2, 0.001, "rotation.y set");
		assertClose(panel.rotation.z, 0.3, 0.001, "rotation.z set");
		assertClose(panel.rotation.w, 0.9, 0.001, "rotation.w set");
	}

	suite("xr-panel — setRotationEuler() identity (0,0,0)");
	{
		const { panel } = createTestPanel();
		panel.setRotationEuler(0, 0, 0);
		assertClose(panel.rotation.x, 0, 0.001, "Euler (0,0,0) → quat.x ≈ 0");
		assertClose(panel.rotation.y, 0, 0.001, "Euler (0,0,0) → quat.y ≈ 0");
		assertClose(panel.rotation.z, 0, 0.001, "Euler (0,0,0) → quat.z ≈ 0");
		assertClose(panel.rotation.w, 1, 0.001, "Euler (0,0,0) → quat.w ≈ 1");
	}

	suite("xr-panel — setRotationEuler() 90° yaw");
	{
		const { panel } = createTestPanel();
		const halfPi = Math.PI / 2;
		panel.setRotationEuler(0, halfPi, 0);
		// 90° yaw → quaternion (0, sin(45°), 0, cos(45°))
		const s = Math.sin(halfPi / 2);
		const c = Math.cos(halfPi / 2);
		assertClose(panel.rotation.x, 0, 0.001, "90° yaw → quat.x ≈ 0");
		assertClose(
			panel.rotation.y,
			s,
			0.001,
			`90° yaw → quat.y ≈ ${s.toFixed(4)}`,
		);
		assertClose(panel.rotation.z, 0, 0.001, "90° yaw → quat.z ≈ 0");
		assertClose(
			panel.rotation.w,
			c,
			0.001,
			`90° yaw → quat.w ≈ ${c.toFixed(4)}`,
		);
	}

	suite("xr-panel — setRotationEuler() 90° pitch");
	{
		const { panel } = createTestPanel();
		const halfPi = Math.PI / 2;
		panel.setRotationEuler(halfPi, 0, 0);
		const s = Math.sin(halfPi / 2);
		const c = Math.cos(halfPi / 2);
		assertClose(
			panel.rotation.x,
			s,
			0.001,
			`90° pitch → quat.x ≈ ${s.toFixed(4)}`,
		);
		assertClose(panel.rotation.y, 0, 0.001, "90° pitch → quat.y ≈ 0");
		assertClose(panel.rotation.z, 0, 0.001, "90° pitch → quat.z ≈ 0");
		assertClose(
			panel.rotation.w,
			c,
			0.001,
			`90° pitch → quat.w ≈ ${c.toFixed(4)}`,
		);
	}

	suite("xr-panel — setRotationEuler() produces unit quaternion");
	{
		const { panel } = createTestPanel();
		panel.setRotationEuler(0.5, 1.2, -0.3);
		const { x, y, z, w } = panel.rotation;
		const len = Math.sqrt(x * x + y * y + z * z + w * w);
		assertClose(len, 1.0, 0.001, "quaternion from Euler is unit length");
	}
}

// ── XRPanel State Helpers ───────────────────────────────────────────────────

export function testPanelStateHelpers(): void {
	suite("xr-panel — markDirty()");
	{
		const { panel } = createTestPanel();
		panel.state.dirty = false;
		panel.markDirty();
		assertTrue(panel.state.dirty, "markDirty() sets dirty to true");
	}

	suite("xr-panel — markMounted()");
	{
		const { panel } = createTestPanel();
		assertFalse(panel.state.mounted, "not mounted initially");
		panel.state.dirty = false;
		panel.markMounted();
		assertTrue(panel.state.mounted, "markMounted() sets mounted to true");
		assertTrue(panel.state.dirty, "markMounted() also marks dirty");
	}
}

// ── XRPanel Model Matrix ────────────────────────────────────────────────────

export function testPanelModelMatrix(): void {
	suite("xr-panel — getModelMatrix() at identity rotation");
	{
		const { panel } = createTestPanel();
		panel.setPosition(0, 0, 0);
		panel.setRotation(0, 0, 0, 1); // identity
		const m = panel.getModelMatrix();

		assertLength(m, 16, "model matrix has 16 elements");
		// With identity rotation, the matrix should be:
		// [ widthM, 0, 0, 0,   0, heightM, 0, 0,   0, 0, 1, 0,   0, 0, 0, 1 ]
		assertClose(m[0], panel.config.widthM, 0.001, "m[0] = widthM");
		assertClose(m[1], 0, 0.001, "m[1] = 0");
		assertClose(m[2], 0, 0.001, "m[2] = 0");
		assertClose(m[3], 0, 0.001, "m[3] = 0");

		assertClose(m[4], 0, 0.001, "m[4] = 0");
		assertClose(m[5], panel.config.heightM, 0.001, "m[5] = heightM");
		assertClose(m[6], 0, 0.001, "m[6] = 0");
		assertClose(m[7], 0, 0.001, "m[7] = 0");

		assertClose(m[8], 0, 0.001, "m[8] = 0");
		assertClose(m[9], 0, 0.001, "m[9] = 0");
		assertClose(m[10], 1, 0.001, "m[10] = 1 (Z axis, no scale)");
		assertClose(m[11], 0, 0.001, "m[11] = 0");

		assertClose(m[15], 1, 0.001, "m[15] = 1 (homogeneous)");
	}

	suite("xr-panel — getModelMatrix() translation");
	{
		const { panel } = createTestPanel();
		panel.setPosition(3.0, 1.5, -2.0);
		panel.setRotation(0, 0, 0, 1);
		const m = panel.getModelMatrix();

		assertClose(m[12], 3.0, 0.001, "m[12] = position.x");
		assertClose(m[13], 1.5, 0.001, "m[13] = position.y");
		assertClose(m[14], -2.0, 0.001, "m[14] = position.z");
	}

	suite("xr-panel — getModelMatrix() 90° Y rotation");
	{
		const { panel } = createTestPanel();
		panel.setPosition(0, 0, 0);
		// 90° around Y: quat = (0, sin(45°), 0, cos(45°))
		const s = Math.SQRT1_2;
		panel.setRotation(0, s, 0, s);
		const m = panel.getModelMatrix();

		// After 90° Y rotation, the X axis should map to Z axis and Z to -X.
		// Column 0 (rotated X axis, scaled by widthM):
		// Original X axis (1,0,0) rotated 90° around Y → (0,0,-1)
		assertClose(m[0], 0, 0.01, "m[0] ≈ 0 after 90° Y rot");
		assertClose(
			m[2],
			-panel.config.widthM,
			0.01,
			"m[2] ≈ -widthM after 90° Y rot",
		);
	}

	suite("xr-panel — getModelMatrix() returns a new array each call");
	{
		const { panel } = createTestPanel();
		const m1 = panel.getModelMatrix();
		const m2 = panel.getModelMatrix();
		assertTrue(m1 !== m2, "each call returns a distinct Float32Array");
		assertClose(m1[0], m2[0], 0.001, "values are identical");
	}
}

// ── XRPanel Raycasting ──────────────────────────────────────────────────────

export function testPanelRaycast(): void {
	suite("xr-panel — raycast: direct hit on default panel");
	{
		const { panel } = createTestPanel();
		// Default panel at (0, 1.4, -1), facing forward (identity rotation).
		// A ray from (0, 1.4, 0) pointing -Z should hit the panel center.
		const hit = panel.raycast(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assertDefined(hit, "ray hits the panel");
		if (hit) {
			assert(hit.panelId, panel.id, "hit.panelId matches");
			assertClose(hit.distance, 1.0, 0.01, "distance is ~1m");
			assertClose(hit.uv.u, 0.5, 0.01, "UV u ≈ 0.5 (center)");
			assertClose(hit.uv.v, 0.5, 0.01, "UV v ≈ 0.5 (center)");
		}
	}

	suite("xr-panel — raycast: hit pixel coordinates at center");
	{
		const { panel } = createTestPanel();
		const hit = panel.raycast(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assertDefined(hit, "ray hits");
		if (hit) {
			const expectedPx = Math.round(0.5 * panel.textureWidth);
			const expectedPy = Math.round(0.5 * panel.textureHeight);
			assert(hit.pixel.x, expectedPx, `pixel.x = ${expectedPx} (center)`);
			assert(hit.pixel.y, expectedPy, `pixel.y = ${expectedPy} (center)`);
		}
	}

	suite("xr-panel — raycast: hit near top-left corner");
	{
		const { panel } = createTestPanel();
		// Panel center at (0, 1.4, -1), width 0.8m, height 0.6m
		// Slightly inset from top-left to avoid boundary floating-point issues
		// Top-left corner: x = -0.4, y = 1.4+0.3 = 1.7; inset by 0.01m
		const hit = panel.raycast(forwardRay({ x: -0.39, y: 1.69, z: 0 }));
		assertDefined(hit, "ray hits near top-left corner");
		if (hit) {
			assertClose(hit.uv.u, 0.0, 0.05, "UV u ≈ 0.0 (near left edge)");
			assertClose(hit.uv.v, 0.0, 0.05, "UV v ≈ 0.0 (near top edge)");
		}
	}

	suite("xr-panel — raycast: hit at bottom-right corner");
	{
		const { panel } = createTestPanel();
		// Bottom-right corner: x = 0.4, y = 1.4-0.3 = 1.1
		const hit = panel.raycast(forwardRay({ x: 0.4, y: 1.1, z: 0 }));
		assertDefined(hit, "ray hits bottom-right corner");
		if (hit) {
			assertClose(hit.uv.u, 1.0, 0.02, "UV u ≈ 1.0 (right edge)");
			assertClose(hit.uv.v, 1.0, 0.02, "UV v ≈ 1.0 (bottom edge)");
		}
	}

	suite("xr-panel — raycast: miss — ray off to the side");
	{
		const { panel } = createTestPanel();
		// Ray 2 meters to the right — should miss
		const hit = panel.raycast(forwardRay({ x: 2.0, y: 1.4, z: 0 }));
		assertNull(hit, "ray misses the panel");
	}

	suite("xr-panel — raycast: miss — ray parallel to panel");
	{
		const { panel } = createTestPanel();
		// Ray going sideways (+X), parallel to the panel's face
		const r = ray({ x: -2, y: 1.4, z: -1 }, { x: 1, y: 0, z: 0 });
		const hit = panel.raycast(r);
		assertNull(hit, "parallel ray misses");
	}

	suite("xr-panel — raycast: miss — ray behind the panel");
	{
		const { panel } = createTestPanel();
		// Ray from behind the panel (z = -2), pointing further away (-Z)
		const hit = panel.raycast(forwardRay({ x: 0, y: 1.4, z: -2 }));
		// The intersection would be at z = -1 which is in front of the ray origin
		// since the ray goes -Z and origin is at -2, it goes to -3, -4, etc.
		// The panel is at z = -1, which is behind the ray origin direction.
		assertNull(hit, "ray from behind misses");
	}

	suite("xr-panel — raycast: miss — non-interactive panel");
	{
		const cfg = tooltipPanelConfig(); // interact = false
		const { panel } = createTestPanel(undefined, cfg);
		panel.setPosition(0, 1.4, -1);
		const hit = panel.raycast(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assertNull(hit, "non-interactive panel returns null");
	}

	suite("xr-panel — raycast: miss — invisible panel");
	{
		const { panel } = createTestPanel();
		panel.state.visible = false;
		const hit = panel.raycast(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assertNull(hit, "invisible panel returns null");
	}

	suite("xr-panel — raycast: distance is positive");
	{
		const { panel } = createTestPanel();
		const hit = panel.raycast(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assertDefined(hit, "hit exists");
		if (hit) {
			assertGreater(hit.distance, 0, "distance > 0");
		}
	}

	suite("xr-panel — raycast: repositioned panel");
	{
		const { panel } = createTestPanel();
		panel.setPosition(5, 3, -2);
		const hit = panel.raycast(forwardRay({ x: 5, y: 3, z: 0 }));
		assertDefined(hit, "ray hits repositioned panel");
		if (hit) {
			assertClose(hit.distance, 2.0, 0.01, "distance ≈ 2m");
			assertClose(hit.uv.u, 0.5, 0.01, "UV u ≈ 0.5");
			assertClose(hit.uv.v, 0.5, 0.01, "UV v ≈ 0.5");
		}
	}
}

// ── XRPanel Rasterization (fallback) ────────────────────────────────────────

export function testPanelRasterizeFallback(): void {
	suite("xr-panel — rasterizeFallback() clears dirty flag");
	{
		const { panel } = createTestPanel();
		assertTrue(panel.state.dirty, "dirty before rasterize");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "not dirty after rasterize");
	}

	suite("xr-panel — rasterizeFallback() doesn't throw on empty panel");
	{
		const { panel } = createTestPanel();
		try {
			panel.rasterizeFallback();
			assertTrue(true, "rasterizeFallback does not throw");
		} catch (e) {
			assertTrue(false, `rasterizeFallback threw: ${e}`);
		}
	}
}

// ── XRPanel Destroy ─────────────────────────────────────────────────────────

export function testPanelDestroy(): void {
	suite("xr-panel — destroy() removes container from DOM");
	{
		const env = createDOMWithCanvas();
		const panel = new XRPanel(0, defaultPanelConfig(), env.document, "#ffffff");
		const containerInDOM = env.document.querySelector('[data-xr-panel="0"]');
		assertDefined(containerInDOM, "container in DOM before destroy");

		panel.destroy();
		const containerAfter = env.document.querySelector('[data-xr-panel="0"]');
		assertNull(containerAfter, "container removed from DOM after destroy");
	}

	suite("xr-panel — destroy() resets state");
	{
		const { panel } = createTestPanel();
		panel.state.visible = true;
		panel.state.mounted = true;
		panel.destroy();
		assertFalse(panel.state.visible, "visible = false after destroy");
		assertFalse(panel.state.mounted, "mounted = false after destroy");
	}

	suite("xr-panel — destroy() deletes GL texture");
	{
		const { panel } = createTestPanel();
		const gl = createStubWebGL2();
		// Simulate texture creation
		panel.glTexture = gl.createTexture() as unknown as WebGLTexture;
		assert(gl._textureCount, 1, "one texture created");

		panel.destroy(gl as unknown as WebGL2RenderingContext);
		assertNull(panel.glTexture, "glTexture set to null");
		assert(gl._deleteTextureCount, 1, "deleteTexture called once");
	}

	suite("xr-panel — destroy() without GL context is safe");
	{
		const { panel } = createTestPanel();
		panel.glTexture = { _id: 1 } as unknown as WebGLTexture;
		// No GL context — should not throw
		panel.destroy();
		assertNull(panel.glTexture, "glTexture set to null without GL");
	}
}

// ── XRPanelManager Lifecycle ────────────────────────────────────────────────

export function testPanelManagerLifecycle(): void {
	suite("xr-panel — PanelManager.createPanel()");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		assertDefined(p, "panel created");
		assert(pm.panelCount, 1, "panelCount = 1");
	}

	suite("xr-panel — PanelManager assigns sequential IDs");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		const p2 = pm.createPanel();
		assert(p0.id, 0, "first panel ID = 0");
		assert(p1.id, 1, "second panel ID = 1");
		assert(p2.id, 2, "third panel ID = 2");
	}

	suite("xr-panel — PanelManager.getPanel()");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		const got = pm.getPanel(p.id);
		assertTrue(got === p, "getPanel returns the same panel instance");
	}

	suite("xr-panel — PanelManager.getPanel() returns undefined for unknown ID");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const got = pm.getPanel(999);
		assert(got, undefined, "unknown ID returns undefined");
	}

	suite("xr-panel — PanelManager.destroyPanel()");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		assert(pm.panelCount, 1, "one panel before destroy");
		pm.destroyPanel(p.id);
		assert(pm.panelCount, 0, "zero panels after destroy");
		assert(pm.getPanel(p.id), undefined, "getPanel returns undefined");
	}

	suite("xr-panel — PanelManager.destroyPanel() with GL context");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		const gl = createStubWebGL2();
		pm.destroyPanel(p.id, gl as unknown as WebGL2RenderingContext);
		assert(pm.panelCount, 0, "panel destroyed");
	}

	suite("xr-panel — PanelManager.destroyPanel() unknown ID is a no-op");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		pm.createPanel();
		pm.destroyPanel(999); // should not throw
		assert(pm.panelCount, 1, "panel count unchanged for unknown ID");
	}

	suite("xr-panel — PanelManager.destroyAll()");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		pm.createPanel();
		pm.createPanel();
		pm.createPanel();
		assert(pm.panelCount, 3, "three panels before destroyAll");
		pm.destroyAll();
		assert(pm.panelCount, 0, "zero panels after destroyAll");
	}

	suite("xr-panel — PanelManager.panels iterator");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		pm.createPanel();
		pm.createPanel();
		let count = 0;
		for (const _p of pm.panels) {
			count++;
		}
		assert(count, 2, "panels iterator yields 2 panels");
	}

	suite("xr-panel — PanelManager createPanel with partial config");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel({ widthM: 2.0, curved: true });
		assertClose(p.config.widthM, 2.0, 0.001, "widthM overridden");
		assertClose(p.config.heightM, 0.6, 0.001, "heightM is default");
		assertTrue(p.config.curved, "curved overridden");
	}
}

// ── XRPanelManager Focus Management ─────────────────────────────────────────

export function testPanelManagerFocus(): void {
	suite("xr-panel — first panel gets automatic focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		assert(pm.focusedPanelId, p.id, "first panel is auto-focused");
		assertTrue(p.state.focused, "panel state.focused = true");
	}

	suite("xr-panel — second panel does not steal focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		assert(pm.focusedPanelId, p0.id, "first panel retains focus");
		assertTrue(p0.state.focused, "p0 focused");
		assertFalse(p1.state.focused, "p1 not focused");
	}

	suite("xr-panel — focusPanel() transfers focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		pm.focusPanel(p1.id);
		assert(pm.focusedPanelId, p1.id, "focus transferred to p1");
		assertFalse(p0.state.focused, "p0 no longer focused");
		assertTrue(p1.state.focused, "p1 now focused");
	}

	suite("xr-panel — focusPanel(-1) clears focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		pm.focusPanel(-1);
		assert(pm.focusedPanelId, -1, "focusedPanelId = -1");
		assertFalse(p0.state.focused, "p0 unfocused");
	}

	suite("xr-panel — focusedPanel getter");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		const fp = pm.focusedPanel;
		assertTrue(fp === p, "focusedPanel returns the focused panel");
	}

	suite("xr-panel — focusedPanel returns null when no focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		pm.createPanel();
		pm.focusPanel(-1);
		assertNull(pm.focusedPanel, "focusedPanel is null");
	}

	suite("xr-panel — destroying focused panel transfers focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		pm.focusPanel(p0.id);
		pm.destroyPanel(p0.id);
		// Focus should transfer to the next available panel
		assert(
			pm.focusedPanelId,
			p1.id,
			"focus transferred to p1 after p0 destroyed",
		);
		assertTrue(p1.state.focused, "p1 state.focused = true");
	}

	suite("xr-panel — destroying the only panel clears focus");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		pm.destroyPanel(p.id);
		assert(pm.focusedPanelId, -1, "focus cleared when last panel destroyed");
	}
}

// ── XRPanelManager Dirty Tracking ───────────────────────────────────────────

export function testPanelManagerDirtyTracking(): void {
	suite("xr-panel — getDirtyPanels() includes dirty visible panels");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document, "#ffffff", 1000); // very high rate for no throttle issues
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		p0.state.dirty = true;
		p0.lastTextureUpdate = 0; // Long ago
		p1.state.dirty = false;
		const dirty = pm.getDirtyPanels();
		assert(dirty.length, 1, "one dirty panel");
		assert(dirty[0].id, p0.id, "the dirty panel is p0");
	}

	suite("xr-panel — getDirtyPanels() excludes invisible panels");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document, "#ffffff", 1000);
		const p = pm.createPanel();
		p.state.dirty = true;
		p.state.visible = false;
		p.lastTextureUpdate = 0;
		const dirty = pm.getDirtyPanels();
		assert(dirty.length, 0, "invisible panel excluded");
	}

	suite("xr-panel — getDirtyPanels() respects throttle interval");
	{
		const env = createDOMWithCanvas();
		// Very low rate: 1 Hz → 1000ms interval
		const pm = new XRPanelManager(env.document, "#ffffff", 1);
		const p = pm.createPanel();
		p.state.dirty = true;
		// Set lastTextureUpdate to "just now"
		p.lastTextureUpdate = performance.now();
		const dirty = pm.getDirtyPanels();
		assert(dirty.length, 0, "panel throttled — too recently updated");
	}
}

// ── XRPanelManager Raycasting ───────────────────────────────────────────────

export function testPanelManagerRaycast(): void {
	suite("xr-panel — PanelManager.raycast() hits closest panel");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const pFar = pm.createPanel();
		pFar.setPosition(0, 1.4, -3); // Far
		const pNear = pm.createPanel();
		pNear.setPosition(0, 1.4, -1); // Near

		const hit = pm.raycast(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assertDefined(hit, "ray hits a panel");
		if (hit) {
			assert(hit.panelId, pNear.id, "closest panel is hit");
		}
	}

	suite("xr-panel — PanelManager.raycast() returns null for no hit");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		pm.createPanel(); // at default pos (0, 1.4, -1)
		const hit = pm.raycast(forwardRay({ x: 100, y: 100, z: 0 }));
		assertNull(hit, "no panel hit");
	}

	suite("xr-panel — PanelManager.raycastAll() returns sorted hits");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p1 = pm.createPanel();
		p1.setPosition(0, 1.4, -3);
		const p2 = pm.createPanel();
		p2.setPosition(0, 1.4, -1);
		const p3 = pm.createPanel();
		p3.setPosition(0, 1.4, -2);

		const hits = pm.raycastAll(forwardRay({ x: 0, y: 1.4, z: 0 }));
		assert(hits.length, 3, "all 3 panels hit");
		assert(hits[0].panelId, p2.id, "nearest panel first (z = -1)");
		assert(hits[1].panelId, p3.id, "middle panel second (z = -2)");
		assert(hits[2].panelId, p1.id, "farthest panel last (z = -3)");
	}

	suite("xr-panel — PanelManager.raycastAll() returns empty for no hits");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const hits = pm.raycastAll(forwardRay({ x: 0, y: 0, z: 0 }));
		assert(hits.length, 0, "empty array for no hits");
	}
}

// ── XRPanelManager Layout: arrangeArc ───────────────────────────────────────

export function testPanelManagerArrangeArc(): void {
	suite("xr-panel — arrangeArc() single panel");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		pm.arrangeArc([p.id], { x: 0, y: 0, z: 0 }, 2.0, 1.5);
		// Single panel with step=0 → angle=startAngle = -totalAngle/2
		// Default totalAngle = PI/2 → startAngle = -PI/4
		const angle = -Math.PI / 4;
		assertClose(
			p.position.x,
			Math.sin(angle) * 2.0,
			0.01,
			"single panel arc: x = sin(angle) * radius",
		);
		assertClose(p.position.y, 1.5, 0.01, "single panel arc: y = height");
		assertClose(
			p.position.z,
			-Math.cos(angle) * 2.0,
			0.01,
			"single panel arc: z = -cos(angle) * radius",
		);
	}

	suite("xr-panel — arrangeArc() three panels");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		const p2 = pm.createPanel();
		const radius = 2.0;
		const height = 1.5;
		const totalAngle = Math.PI / 2;
		pm.arrangeArc(
			[p0.id, p1.id, p2.id],
			{ x: 0, y: 0, z: 0 },
			radius,
			height,
			totalAngle,
		);

		// 3 panels → step = (PI/2) / 2 = PI/4
		// angles: -PI/4, 0, +PI/4
		const angles = [-totalAngle / 2, 0, totalAngle / 2];

		assertClose(p0.position.x, Math.sin(angles[0]) * radius, 0.01, "p0 arc x");
		assertClose(p0.position.y, height, 0.01, "p0 arc y");
		assertClose(p0.position.z, -Math.cos(angles[0]) * radius, 0.01, "p0 arc z");

		assertClose(
			p1.position.x,
			Math.sin(angles[1]) * radius,
			0.01,
			"p1 arc x (center)",
		);
		assertClose(
			p1.position.z,
			-Math.cos(angles[1]) * radius,
			0.01,
			"p1 arc z (center)",
		);

		assertClose(p2.position.x, Math.sin(angles[2]) * radius, 0.01, "p2 arc x");
		assertClose(p2.position.z, -Math.cos(angles[2]) * radius, 0.01, "p2 arc z");
	}

	suite("xr-panel — arrangeArc() empty array is a no-op");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		pm.arrangeArc([], { x: 0, y: 0, z: 0 }, 2, 1.5);
		assertTrue(true, "arrangeArc with empty array does not throw");
	}
}

// ── XRPanelManager Layout: arrangeGrid ──────────────────────────────────────

export function testPanelManagerArrangeGrid(): void {
	suite("xr-panel — arrangeGrid() 2x2 layout");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const panels = [
			pm.createPanel(),
			pm.createPanel(),
			pm.createPanel(),
			pm.createPanel(),
		];
		const ids = panels.map((p) => p.id);

		const origin: Vec3 = { x: 0, y: 2, z: -2 };
		const columns = 2;
		const spacingX = 1.0;
		const spacingY = 0.8;

		pm.arrangeGrid(ids, origin, columns, spacingX, spacingY);

		// Panel 0: col=0, row=0
		assertClose(panels[0].position.x, 0, 0.01, "p0 grid x = origin.x");
		assertClose(panels[0].position.y, 2, 0.01, "p0 grid y = origin.y");
		assertClose(panels[0].position.z, -2, 0.01, "p0 grid z = origin.z");

		// Panel 1: col=1, row=0
		assertClose(
			panels[1].position.x,
			1.0,
			0.01,
			"p1 grid x = origin.x + spacingX",
		);

		// Panel 2: col=0, row=1
		assertClose(panels[2].position.x, 0, 0.01, "p2 grid x = origin.x");
		assertClose(
			panels[2].position.y,
			1.2,
			0.01,
			"p2 grid y = origin.y - spacingY",
		);

		// Panel 3: col=1, row=1
		assertClose(panels[3].position.x, 1.0, 0.01, "p3 grid x");
		assertClose(panels[3].position.y, 1.2, 0.01, "p3 grid y");
	}

	suite("xr-panel — arrangeGrid() identity rotation applied");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		pm.arrangeGrid([p.id], { x: 0, y: 1, z: -1 }, 1);
		assertClose(p.rotation.x, 0, 0.001, "grid rotation x = 0");
		assertClose(p.rotation.y, 0, 0.001, "grid rotation y = 0");
		assertClose(p.rotation.z, 0, 0.001, "grid rotation z = 0");
		assertClose(p.rotation.w, 1, 0.001, "grid rotation w = 1");
	}
}

// ── XRPanelManager Layout: arrangeStack ─────────────────────────────────────

export function testPanelManagerArrangeStack(): void {
	suite("xr-panel — arrangeStack() vertical arrangement");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();
		const p2 = pm.createPanel();

		const top: Vec3 = { x: 0, y: 2.5, z: -1.5 };
		const spacing = 0.8;
		pm.arrangeStack([p0.id, p1.id, p2.id], top, spacing);

		assertClose(p0.position.x, 0, 0.01, "p0 stack x");
		assertClose(p0.position.y, 2.5, 0.01, "p0 stack y = top.y");
		assertClose(p0.position.z, -1.5, 0.01, "p0 stack z");

		assertClose(p1.position.y, 1.7, 0.01, "p1 stack y = top.y - spacing");
		assertClose(p2.position.y, 0.9, 0.01, "p2 stack y = top.y - 2*spacing");
	}

	suite("xr-panel — arrangeStack() default spacing");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p0 = pm.createPanel();
		const p1 = pm.createPanel();

		pm.arrangeStack([p0.id, p1.id], { x: 0, y: 2.0, z: -1 });
		// Default spacing = 0.7
		assertClose(p0.position.y, 2.0, 0.01, "p0 at top");
		assertClose(p1.position.y, 1.3, 0.01, "p1 at top - 0.7 (default spacing)");
	}

	suite("xr-panel — arrangeStack() identity rotation");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		const p = pm.createPanel();
		pm.arrangeStack([p.id], { x: 0, y: 2, z: -1 });
		assertClose(p.rotation.w, 1, 0.001, "stack rotation is identity");
	}
}

// ── XRPanelManager Constructor Options ──────────────────────────────────────

export function testPanelManagerConstructor(): void {
	suite("xr-panel — PanelManager custom background color");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document, "#123456");
		const p = pm.createPanel();
		const style = p.container.querySelector("style");
		assertDefined(style, "style element present");
		if (style) {
			assertTrue(
				style.textContent?.includes("#123456") ?? false,
				"custom background color in panel style",
			);
		}
	}

	suite("xr-panel — PanelManager default values");
	{
		const env = createDOMWithCanvas();
		const pm = new XRPanelManager(env.document);
		assert(pm.panelCount, 0, "starts with 0 panels");
		assert(pm.focusedPanelId, -1, "no focused panel initially");
		assertNull(pm.focusedPanel, "focusedPanel is null initially");
	}
}

// ── Aggregate ───────────────────────────────────────────────────────────────

export function testXRPanel(): void {
	testPanelConstruction();
	testPanelDOMContainer();
	testPanelTransforms();
	testPanelStateHelpers();
	testPanelModelMatrix();
	testPanelRaycast();
	testPanelRasterizeFallback();
	testPanelDestroy();
	testPanelManagerLifecycle();
	testPanelManagerFocus();
	testPanelManagerDirtyTracking();
	testPanelManagerRaycast();
	testPanelManagerArrangeArc();
	testPanelManagerArrangeGrid();
	testPanelManagerArrangeStack();
	testPanelManagerConstructor();
}
