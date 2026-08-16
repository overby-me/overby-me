// XR Types Tests — Panel config presets, default configs, math types.
//
// Tests the type constructors and preset factories from xr-types.ts.
// These are pure functions with no DOM dependencies.

import {
	dashboardPanelConfig,
	defaultPanelConfig,
	defaultXRRuntimeConfig,
	handAnchoredPanelConfig,
	tooltipPanelConfig,
	XRInputEventType,
} from "../runtime/xr-types.ts";
import {
	assert,
	assertClose,
	assertFalse,
	assertTrue,
	suite,
} from "./harness.ts";

// ── Panel Config Presets ────────────────────────────────────────────────────

export function testPanelConfigPresets(): void {
	suite("xr-types — defaultPanelConfig()");
	{
		const cfg = defaultPanelConfig();
		assertClose(cfg.widthM, 0.8, 0.001, "default width is 0.8m");
		assertClose(cfg.heightM, 0.6, 0.001, "default height is 0.6m");
		assert(cfg.pixelsPerMeter, 1200, "default pixelsPerMeter is 1200");
		assertFalse(cfg.curved, "default is not curved");
		assertTrue(cfg.interact, "default is interactive");
	}

	suite("xr-types — dashboardPanelConfig()");
	{
		const cfg = dashboardPanelConfig();
		assertClose(cfg.widthM, 1.6, 0.001, "dashboard width is 1.6m");
		assertClose(cfg.heightM, 0.9, 0.001, "dashboard height is 0.9m");
		assert(cfg.pixelsPerMeter, 1000, "dashboard pixelsPerMeter is 1000");
		assertTrue(cfg.curved, "dashboard is curved");
		assertTrue(cfg.interact, "dashboard is interactive");
	}

	suite("xr-types — tooltipPanelConfig()");
	{
		const cfg = tooltipPanelConfig();
		assertClose(cfg.widthM, 0.3, 0.001, "tooltip width is 0.3m");
		assertClose(cfg.heightM, 0.15, 0.001, "tooltip height is 0.15m");
		assert(cfg.pixelsPerMeter, 800, "tooltip pixelsPerMeter is 800");
		assertFalse(cfg.curved, "tooltip is not curved");
		assertFalse(cfg.interact, "tooltip is NOT interactive");
	}

	suite("xr-types — handAnchoredPanelConfig()");
	{
		const cfg = handAnchoredPanelConfig();
		assertClose(cfg.widthM, 0.2, 0.001, "hand-anchored width is 0.2m");
		assertClose(cfg.heightM, 0.15, 0.001, "hand-anchored height is 0.15m");
		assert(cfg.pixelsPerMeter, 1400, "hand-anchored pixelsPerMeter is 1400");
		assertFalse(cfg.curved, "hand-anchored is not curved");
		assertTrue(cfg.interact, "hand-anchored is interactive");
	}
}

// ── Preset Independence ─────────────────────────────────────────────────────

export function testPresetIndependence(): void {
	suite("xr-types — presets return independent objects");
	{
		const a = defaultPanelConfig();
		const b = defaultPanelConfig();
		a.widthM = 99;
		assertClose(
			b.widthM,
			0.8,
			0.001,
			"mutating one default does not affect another",
		);

		const c = dashboardPanelConfig();
		const d = dashboardPanelConfig();
		c.heightM = 42;
		assertClose(
			d.heightM,
			0.9,
			0.001,
			"mutating one dashboard does not affect another",
		);
	}
}

// ── Texture Dimension Derivation ────────────────────────────────────────────

export function testTextureDimensions(): void {
	suite("xr-types — texture dimensions derived from config");
	{
		const cfg = defaultPanelConfig();
		const expectedW = Math.round(cfg.widthM * cfg.pixelsPerMeter);
		const expectedH = Math.round(cfg.heightM * cfg.pixelsPerMeter);
		assert(expectedW, 960, "default texture width = 0.8 * 1200 = 960");
		assert(expectedH, 720, "default texture height = 0.6 * 1200 = 720");
	}

	{
		const cfg = dashboardPanelConfig();
		const expectedW = Math.round(cfg.widthM * cfg.pixelsPerMeter);
		const expectedH = Math.round(cfg.heightM * cfg.pixelsPerMeter);
		assert(expectedW, 1600, "dashboard texture width = 1.6 * 1000 = 1600");
		assert(expectedH, 900, "dashboard texture height = 0.9 * 1000 = 900");
	}

	{
		const cfg = tooltipPanelConfig();
		const expectedW = Math.round(cfg.widthM * cfg.pixelsPerMeter);
		const expectedH = Math.round(cfg.heightM * cfg.pixelsPerMeter);
		assert(expectedW, 240, "tooltip texture width = 0.3 * 800 = 240");
		assert(expectedH, 120, "tooltip texture height = 0.15 * 800 = 120");
	}

	{
		const cfg = handAnchoredPanelConfig();
		const expectedW = Math.round(cfg.widthM * cfg.pixelsPerMeter);
		const expectedH = Math.round(cfg.heightM * cfg.pixelsPerMeter);
		assert(expectedW, 280, "hand-anchored texture width = 0.2 * 1400 = 280");
		assert(expectedH, 210, "hand-anchored texture height = 0.15 * 1400 = 210");
	}
}

// ── XR Runtime Config ───────────────────────────────────────────────────────

export function testDefaultRuntimeConfig(): void {
	suite("xr-types — defaultXRRuntimeConfig()");
	{
		const cfg = defaultXRRuntimeConfig();
		assert(
			cfg.sessionMode,
			"immersive-vr",
			"default session mode is immersive-vr",
		);
		assertTrue(cfg.fallbackToFlat, "fallbackToFlat is true by default");
		assert(cfg.panelBackground, "#ffffff", "default panel background is white");
		assertTrue(cfg.showEnterVRButton, "showEnterVRButton is true by default");
		assert(cfg.textureUpdateRate, 30, "default texture update rate is 30 Hz");
	}

	suite("xr-types — defaultXRRuntimeConfig() required features");
	{
		const cfg = defaultXRRuntimeConfig();
		assert(cfg.requiredFeatures.length, 1, "one required feature");
		assert(cfg.requiredFeatures[0], "local-floor", "required: local-floor");
	}

	suite("xr-types — defaultXRRuntimeConfig() optional features");
	{
		const cfg = defaultXRRuntimeConfig();
		assert(cfg.optionalFeatures.length, 2, "two optional features");
		assertTrue(
			cfg.optionalFeatures.includes("hand-tracking"),
			"optional: hand-tracking",
		);
		assertTrue(
			cfg.optionalFeatures.includes("bounded-floor"),
			"optional: bounded-floor",
		);
	}

	suite("xr-types — runtime configs are independent");
	{
		const a = defaultXRRuntimeConfig();
		const b = defaultXRRuntimeConfig();
		a.sessionMode = "immersive-ar";
		assert(
			b.sessionMode,
			"immersive-vr",
			"mutating one config does not affect another",
		);

		a.requiredFeatures.push("extra");
		assert(
			b.requiredFeatures.length,
			1,
			"mutating array does not affect another",
		);
	}
}

// ── XR Input Event Type Constants ───────────────────────────────────────────

export function testXRInputEventTypes(): void {
	suite("xr-types — XRInputEventType constants");
	assert(XRInputEventType.Select, "select", "Select = 'select'");
	assert(
		XRInputEventType.SelectStart,
		"selectstart",
		"SelectStart = 'selectstart'",
	);
	assert(XRInputEventType.SelectEnd, "selectend", "SelectEnd = 'selectend'");
	assert(XRInputEventType.Squeeze, "squeeze", "Squeeze = 'squeeze'");
	assert(XRInputEventType.Hover, "hover", "Hover = 'hover'");

	suite("xr-types — XRInputEventType is frozen-like (const assertion)");
	{
		// The type system prevents mutation via `as const`, but at runtime
		// the object is a plain object. Verify the values are strings.
		const keys = Object.keys(XRInputEventType);
		assert(keys.length, 5, "5 event type constants");
		for (const key of keys) {
			assertTrue(
				typeof (XRInputEventType as Record<string, unknown>)[key] === "string",
				`${key} is a string`,
			);
		}
	}
}

// ── Panel Config Spread Patterns ────────────────────────────────────────────

export function testConfigSpreadPatterns(): void {
	suite("xr-types — partial config override via spread");
	{
		// This is the pattern used by XRPanelManager.createPanel()
		const partial = { widthM: 2.0, curved: true };
		const full = { ...defaultPanelConfig(), ...partial };

		assertClose(full.widthM, 2.0, 0.001, "widthM overridden to 2.0");
		assertClose(full.heightM, 0.6, 0.001, "heightM retains default 0.6");
		assert(full.pixelsPerMeter, 1200, "pixelsPerMeter retains default 1200");
		assertTrue(full.curved, "curved overridden to true");
		assertTrue(full.interact, "interact retains default true");
	}

	suite("xr-types — empty partial keeps all defaults");
	{
		const full = { ...defaultPanelConfig(), ...{} };
		assertClose(full.widthM, 0.8, 0.001, "widthM is default");
		assertClose(full.heightM, 0.6, 0.001, "heightM is default");
		assert(full.pixelsPerMeter, 1200, "pixelsPerMeter is default");
		assertFalse(full.curved, "curved is default");
		assertTrue(full.interact, "interact is default");
	}

	suite("xr-types — runtime config partial override");
	{
		const partial = {
			sessionMode: "immersive-ar" as const,
			textureUpdateRate: 60,
		};
		const full = { ...defaultXRRuntimeConfig(), ...partial };

		assert(full.sessionMode, "immersive-ar", "sessionMode overridden");
		assert(full.textureUpdateRate, 60, "textureUpdateRate overridden to 60");
		assertTrue(full.fallbackToFlat, "fallbackToFlat retains default");
		assertTrue(full.showEnterVRButton, "showEnterVRButton retains default");
	}
}

// ── Physical Size Ratios ────────────────────────────────────────────────────

export function testPhysicalSizeRatios(): void {
	suite("xr-types — panel aspect ratios");
	{
		const def = defaultPanelConfig();
		const defRatio = def.widthM / def.heightM;
		assertClose(defRatio, 4 / 3, 0.01, "default aspect ratio ≈ 4:3");

		const dash = dashboardPanelConfig();
		const dashRatio = dash.widthM / dash.heightM;
		assertClose(dashRatio, 16 / 9, 0.01, "dashboard aspect ratio ≈ 16:9");

		const tip = tooltipPanelConfig();
		const tipRatio = tip.widthM / tip.heightM;
		assertClose(tipRatio, 2.0, 0.01, "tooltip aspect ratio ≈ 2:1");

		const hand = handAnchoredPanelConfig();
		const handRatio = hand.widthM / hand.heightM;
		assertClose(handRatio, 4 / 3, 0.01, "hand-anchored aspect ratio ≈ 4:3");
	}
}

// ── Aggregate ───────────────────────────────────────────────────────────────

export function testXRTypes(): void {
	testPanelConfigPresets();
	testPresetIndependence();
	testTextureDimensions();
	testDefaultRuntimeConfig();
	testXRInputEventTypes();
	testConfigSpreadPatterns();
	testPhysicalSizeRatios();
}
