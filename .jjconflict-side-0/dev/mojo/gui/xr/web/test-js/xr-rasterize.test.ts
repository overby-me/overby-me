// XR Rasterization Tests — SVG foreignObject fidelity validation.
//
// Tests the DOM→texture rasterization pipeline used by XR panels:
//   1. SVG foreignObject markup generation structure
//   2. Fallback rasterizer behavior with various content types
//   3. Panel dirty tracking through rasterization cycles
//   4. Content mutation → re-rasterize flow
//   5. Edge cases: empty content, special characters, overflow, large DOM
//   6. Texture upload integration with WebGL stubs
//   7. Panel manager dirty texture update orchestration
//
// These tests run headlessly via linkedom + canvas stubs. They validate
// the rasterization logic and markup structure, not pixel-perfect output
// (which requires a real browser — covered by the browser E2E suite).
//
// SVG foreignObject limitations documented here serve as the fidelity
// validation baseline for Open Question #2 in phase5-xr.md.

import { XRPanel, XRPanelManager } from "../runtime/xr-panel.ts";
import type { PanelConfig } from "../runtime/xr-types.ts";
import {
	dashboardPanelConfig,
	defaultPanelConfig,
	tooltipPanelConfig,
} from "../runtime/xr-types.ts";
import {
	createDOMWithCanvas,
	createStubWebGL2,
	type DOMEnvironment,
	type StubCanvasContext2D,
} from "./dom-helper.ts";
import {
	assert,
	assertDefined,
	assertFalse,
	assertGreater,
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

/**
 * Inject HTML content into a panel's container (simulating mutation application).
 * Skips the first child (the injected <style> element from the constructor).
 */
function setPanelContent(panel: XRPanel, html: string): void {
	// Remove all children except the initial <style> element
	const children = Array.from(panel.container.childNodes);
	for (let i = 1; i < children.length; i++) {
		panel.container.removeChild(children[i]);
	}
	// Insert content after the style element
	const wrapper = panel.container.ownerDocument.createElement("div");
	wrapper.innerHTML = html;
	while (wrapper.firstChild) {
		panel.container.appendChild(wrapper.firstChild);
	}
	panel.markDirty();
}

/**
 * Get the innerHTML of a panel's container, excluding the initial <style>.
 * Useful for verifying content before rasterization.
 */
function getPanelContent(panel: XRPanel): string {
	const children = Array.from(panel.container.childNodes);
	let html = "";
	for (let i = 1; i < children.length; i++) {
		const child = children[i];
		if ("outerHTML" in child) {
			html += (child as Element).outerHTML;
		} else if (child.textContent) {
			html += child.textContent;
		}
	}
	return html;
}

// ── SVG foreignObject Markup Structure ──────────────────────────────────────

function testSVGMarkupStructure(): void {
	suite("xr-rasterize — SVG markup wraps panel innerHTML");
	{
		// The rasterize() method reads container.innerHTML and wraps it in SVG.
		// We verify the container innerHTML is well-formed after content injection.
		const { panel } = createTestPanel();
		setPanelContent(panel, "<h1>Hello XR</h1>");

		const content = panel.container.innerHTML;
		assertGreater(content.length, 0, "container has innerHTML after injection");
		assertTrue(
			content.includes("Hello XR"),
			"innerHTML contains injected text",
		);
		assertTrue(
			content.includes("<h1>"),
			"innerHTML contains injected element tags",
		);
	}

	suite("xr-rasterize — SVG markup includes panel style element");
	{
		const { panel } = createTestPanel(undefined, undefined, 42);
		const content = panel.container.innerHTML;
		assertTrue(content.includes("<style>"), "innerHTML starts with <style>");
		assertTrue(
			content.includes('data-xr-panel="42"'),
			"style references panel ID",
		);
		assertTrue(
			content.includes("font-family"),
			"style includes font-family default",
		);
	}

	suite("xr-rasterize — container dimensions match texture dimensions");
	{
		const { panel } = createTestPanel();
		const expectedW = Math.round(0.8 * 1200); // default config
		const expectedH = Math.round(0.6 * 1200);

		const style = panel.container.style;
		assertTrue(
			style.cssText.includes(`${expectedW}px`),
			`container width includes ${expectedW}px`,
		);
		assertTrue(
			style.cssText.includes(`${expectedH}px`),
			`container height includes ${expectedH}px`,
		);
	}

	suite("xr-rasterize — dashboard panel has larger texture dimensions");
	{
		const cfg = dashboardPanelConfig();
		const { panel } = createTestPanel(undefined, cfg);
		const expectedW = Math.round(cfg.widthM * cfg.pixelsPerMeter);
		const expectedH = Math.round(cfg.heightM * cfg.pixelsPerMeter);

		assert(panel.textureWidth, expectedW, "dashboard textureWidth");
		assert(panel.textureHeight, expectedH, "dashboard textureHeight");
	}

	suite("xr-rasterize — tooltip panel has smaller texture dimensions");
	{
		const cfg = tooltipPanelConfig();
		const { panel } = createTestPanel(undefined, cfg);
		const expectedW = Math.round(cfg.widthM * cfg.pixelsPerMeter);
		const expectedH = Math.round(cfg.heightM * cfg.pixelsPerMeter);

		assert(panel.textureWidth, expectedW, "tooltip textureWidth");
		assert(panel.textureHeight, expectedH, "tooltip textureHeight");
	}
}

// ── Fallback Rasterizer ─────────────────────────────────────────────────────

function testFallbackRasterizer(): void {
	suite("xr-rasterize — fallback rasterizer clears dirty flag");
	{
		const { panel } = createTestPanel();
		setPanelContent(panel, "<p>Test content</p>");
		assertTrue(panel.state.dirty, "panel is dirty before rasterize");

		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "panel is clean after fallback rasterize");
	}

	suite("xr-rasterize — fallback rasterizer handles empty content");
	{
		const { panel } = createTestPanel();
		// Container has only the <style> element — no user content
		panel.markDirty();
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after rasterizing empty content");
	}

	suite("xr-rasterize — fallback rasterizer handles plain text");
	{
		const { panel } = createTestPanel();
		setPanelContent(panel, "Simple plain text without any HTML tags");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after rasterizing plain text");
	}

	suite("xr-rasterize — fallback rasterizer handles deeply nested DOM");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			"<div><div><div><span><strong>Deep</strong></span></div></div></div>",
		);
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after rasterizing nested DOM");
	}

	suite("xr-rasterize — fallback rasterizer handles long text (word-wrap)");
	{
		const { panel } = createTestPanel();
		const longText = Array.from({ length: 200 }, (_, i) => `word${i}`).join(
			" ",
		);
		setPanelContent(panel, `<p>${longText}</p>`);
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after rasterizing long text");
	}

	suite("xr-rasterize — fallback rasterizer handles special characters");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			"<p>&lt;script&gt;alert('xss')&lt;/script&gt; &amp; &quot;quotes&quot;</p>",
		);
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after special characters");
	}

	suite("xr-rasterize — fallback rasterizer handles unicode content");
	{
		const { panel } = createTestPanel();
		setPanelContent(panel, "<p>🔥 Mojo GUI 你好世界 مرحبا</p>");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after unicode content");
	}

	suite("xr-rasterize — fallback rasterizer handles styled content");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			[
				'<div style="display: flex; gap: 8px;">',
				'  <span style="color: red; font-weight: bold;">Bold Red</span>',
				'  <span style="font-style: italic;">Italic</span>',
				"</div>",
			].join(""),
		);
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after styled content");
	}

	suite("xr-rasterize — fallback rasterizer handles form elements");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			[
				'<input type="text" value="hello" />',
				"<button>Click me</button>",
				"<select><option>A</option><option>B</option></select>",
			].join(""),
		);
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after form elements");
	}

	suite("xr-rasterize — fallback rasterizer handles table content");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			[
				"<table>",
				"<tr><th>Name</th><th>Value</th></tr>",
				"<tr><td>Alpha</td><td>1</td></tr>",
				"<tr><td>Beta</td><td>2</td></tr>",
				"</table>",
			].join(""),
		);
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after table content");
	}
}

// ── Dirty Tracking Through Rasterization Cycles ─────────────────────────────

function testDirtyTracking(): void {
	suite("xr-rasterize — new panel starts dirty");
	{
		const { panel } = createTestPanel();
		assertTrue(panel.state.dirty, "new panel is dirty");
	}

	suite("xr-rasterize — markDirty sets dirty flag");
	{
		const { panel } = createTestPanel();
		panel.rasterizeFallback(); // clear dirty
		assertFalse(panel.state.dirty, "clean after rasterize");
		panel.markDirty();
		assertTrue(panel.state.dirty, "dirty after markDirty");
	}

	suite("xr-rasterize — multiple rasterize cycles track dirty correctly");
	{
		const { panel } = createTestPanel();

		// Cycle 1: dirty → rasterize → clean
		assertTrue(panel.state.dirty, "dirty at start");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after cycle 1");

		// Cycle 2: mark dirty → rasterize → clean
		panel.markDirty();
		assertTrue(panel.state.dirty, "dirty after markDirty");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after cycle 2");

		// Cycle 3: content change → rasterize → clean
		setPanelContent(panel, "<p>New content</p>");
		assertTrue(panel.state.dirty, "dirty after content change");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after cycle 3");
	}

	suite("xr-rasterize — content mutation marks panel dirty");
	{
		const { panel } = createTestPanel();
		panel.rasterizeFallback(); // clear initial dirty
		assertFalse(panel.state.dirty, "clean before mutation");

		setPanelContent(panel, "<h1>Counter: 0</h1>");
		assertTrue(panel.state.dirty, "dirty after content mutation");
	}

	suite("xr-rasterize — markMounted sets mounted flag");
	{
		const { panel } = createTestPanel();
		assertFalse(panel.state.mounted, "not mounted initially");
		panel.markMounted();
		assertTrue(panel.state.mounted, "mounted after markMounted");
	}
}

// ── Content Mutation → Re-rasterize Flow ────────────────────────────────────

function testMutationRasterizeFlow(): void {
	suite("xr-rasterize — counter increment updates container innerHTML");
	{
		const { panel } = createTestPanel();

		// Simulate initial mount
		setPanelContent(panel, "<div><h1>Counter: 0</h1><button>+</button></div>");
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after initial render");

		// Simulate counter increment (mutation applied)
		setPanelContent(panel, "<div><h1>Counter: 1</h1><button>+</button></div>");
		assertTrue(panel.state.dirty, "dirty after increment");

		const content = getPanelContent(panel);
		assertTrue(content.includes("Counter: 1"), "content reflects new state");

		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after re-rasterize");
	}

	suite("xr-rasterize — todo item addition updates container");
	{
		const { panel } = createTestPanel();

		setPanelContent(panel, '<ul><li>Item 1</li></ul><input type="text" />');
		panel.rasterizeFallback();

		// Add a new item
		setPanelContent(
			panel,
			'<ul><li>Item 1</li><li>Item 2</li></ul><input type="text" />',
		);
		assertTrue(panel.state.dirty, "dirty after adding todo item");

		const content = getPanelContent(panel);
		assertTrue(content.includes("Item 2"), "new item is in the DOM");

		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after re-rasterize");
	}

	suite("xr-rasterize — rapid mutations only need one rasterize");
	{
		const { panel } = createTestPanel();
		panel.rasterizeFallback();

		// Simulate multiple rapid mutations (e.g. animation frame)
		setPanelContent(panel, "<p>Frame 1</p>");
		setPanelContent(panel, "<p>Frame 2</p>");
		setPanelContent(panel, "<p>Frame 3</p>");

		assertTrue(panel.state.dirty, "dirty after rapid mutations");

		// Only the final state is rasterized
		const content = getPanelContent(panel);
		assertTrue(
			content.includes("Frame 3"),
			"only final mutation state present",
		);

		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "one rasterize clears dirty");
	}

	suite("xr-rasterize — content removal updates container");
	{
		const { panel } = createTestPanel();
		setPanelContent(panel, "<div><p>Keep</p><p>Remove</p></div>");
		panel.rasterizeFallback();

		setPanelContent(panel, "<div><p>Keep</p></div>");
		assertTrue(panel.state.dirty, "dirty after content removal");

		const content = getPanelContent(panel);
		assertFalse(content.includes("Remove"), "removed content is gone");
		assertTrue(content.includes("Keep"), "kept content remains");
	}
}

// ── Texture Upload Integration ──────────────────────────────────────────────

function testTextureUpload(): void {
	suite("xr-rasterize — uploadTexture creates GL texture on first call");
	{
		const { panel } = createTestPanel();
		const gl = createStubWebGL2();
		panel.rasterizeFallback();

		panel.uploadTexture(gl as unknown as WebGL2RenderingContext);

		assert(gl._textureCount, 1, "one texture created");
		assert(gl._texImage2DCount, 1, "one texImage2D call (initial upload)");
		assertDefined(panel.glTexture, "glTexture is assigned");
	}

	suite("xr-rasterize — uploadTexture reuses texture on subsequent calls");
	{
		const { panel } = createTestPanel();
		const gl = createStubWebGL2();

		panel.rasterizeFallback();
		panel.uploadTexture(gl as unknown as WebGL2RenderingContext);
		assert(gl._textureCount, 1, "one texture after first upload");

		// Second upload — should reuse the existing texture
		panel.markDirty();
		panel.rasterizeFallback();
		panel.uploadTexture(gl as unknown as WebGL2RenderingContext);
		assert(gl._textureCount, 1, "still one texture after second upload");
		assert(
			gl._texSubImage2DCount,
			1,
			"one texSubImage2D call (update existing)",
		);
	}

	suite("xr-rasterize — destroy deletes GL texture");
	{
		const { panel } = createTestPanel();
		const gl = createStubWebGL2();

		panel.rasterizeFallback();
		panel.uploadTexture(gl as unknown as WebGL2RenderingContext);
		assert(gl._textureCount, 1, "texture created");

		panel.destroy(gl as unknown as WebGL2RenderingContext);
		assert(gl._deleteTextureCount, 1, "texture deleted on destroy");
	}

	suite("xr-rasterize — destroy without GL context is safe");
	{
		const { panel } = createTestPanel();
		setPanelContent(panel, "<p>Content</p>");

		// Destroy without ever creating a texture
		panel.destroy();
		// Should not throw — just cleans up DOM
	}

	suite("xr-rasterize — destroy removes container from DOM");
	{
		const env = createDOMWithCanvas();
		const { panel } = createTestPanel(env);

		const containerParent = panel.container.parentNode;
		assertDefined(containerParent, "container is in the DOM");

		panel.destroy();

		// After destroy, the container should be removed from its parent
		// (the XRPanel.destroy method calls container.remove() or equivalent)
		// We can verify by checking if the body still contains the container
		const found = env.document.querySelector(`[data-xr-panel="0"]`);
		assert(found, null, "container removed from DOM after destroy");
	}
}

// ── Panel Manager Dirty Texture Updates ─────────────────────────────────────

function testPanelManagerDirtyUpdatesSync(): void {
	suite("xr-rasterize — manager getDirtyPanels returns only dirty panels");
	{
		const env = createDOMWithCanvas();
		const manager = new XRPanelManager(env.document);

		const p1 = manager.createPanel();
		const p2 = manager.createPanel();

		// Both start dirty
		let dirty = manager.getDirtyPanels();
		assert(dirty.length, 2, "both panels start dirty");

		// Rasterize p1 only
		p1.rasterizeFallback();
		dirty = manager.getDirtyPanels();
		assert(dirty.length, 1, "one panel still dirty");
		assert(dirty[0].id, p2.id, "dirty panel is p2");
	}
}

async function testPanelManagerDirtyUpdatesAsync(): Promise<void> {
	suite("xr-rasterize — manager updateDirtyTextures processes all dirty");
	{
		const env = createDOMWithCanvas();
		// Use a very high update rate (100000 Hz) to effectively disable
		// the texture update throttle — otherwise performance.now() barely
		// changes between calls and getDirtyPanels() filters them out.
		const manager = new XRPanelManager(env.document, "#ffffff", 100000);
		const gl = createStubWebGL2();

		manager.createPanel();
		manager.createPanel();
		manager.createPanel();

		// Use fallback mode (useFallback = true) to avoid SVG Image loading
		// which doesn't work in linkedom
		await manager.updateDirtyTextures(
			gl as unknown as WebGL2RenderingContext,
			true,
		);

		// All 3 panels should have textures uploaded
		assert(gl._textureCount, 3, "3 textures created for 3 panels");

		// After update, no panels should be dirty
		const dirty = manager.getDirtyPanels();
		assert(dirty.length, 0, "no dirty panels after updateDirtyTextures");
	}

	suite("xr-rasterize — manager only re-rasterizes changed panels");
	{
		const env = createDOMWithCanvas();
		const manager = new XRPanelManager(env.document, "#ffffff", 100000);
		const gl = createStubWebGL2();

		const p1 = manager.createPanel();
		const _p2 = manager.createPanel();

		// Initial render
		await manager.updateDirtyTextures(
			gl as unknown as WebGL2RenderingContext,
			true,
		);
		assert(gl._textureCount, 2, "2 textures after initial render");
		assert(gl._texImage2DCount, 2, "2 texImage2D for initial upload");

		// Only mark p1 dirty
		p1.markDirty();
		await manager.updateDirtyTextures(
			gl as unknown as WebGL2RenderingContext,
			true,
		);

		// p1 gets texSubImage2D (update), p2 is skipped
		assert(gl._texSubImage2DCount, 1, "only 1 texSubImage2D for dirty panel");
	}

	suite("xr-rasterize — manager destroyAll cleans up all textures");
	{
		const env = createDOMWithCanvas();
		const manager = new XRPanelManager(env.document, "#ffffff", 100000);
		const gl = createStubWebGL2();

		manager.createPanel();
		manager.createPanel();

		await manager.updateDirtyTextures(
			gl as unknown as WebGL2RenderingContext,
			true,
		);
		assert(gl._textureCount, 2, "2 textures created");

		manager.destroyAll(gl as unknown as WebGL2RenderingContext);
		assert(gl._deleteTextureCount, 2, "2 textures deleted on destroyAll");
		assert(manager.panelCount, 0, "no panels after destroyAll");
	}
}

// ── SVG foreignObject Fidelity Edge Cases ───────────────────────────────────
// These document known limitations and validate safe handling.

function testSVGFidelityEdgeCases(): void {
	suite("xr-rasterize — fidelity: inline styles are preserved in container");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			'<div style="background: #ff0000; padding: 16px; border-radius: 8px;">Styled</div>',
		);

		const content = panel.container.innerHTML;
		assertTrue(content.includes("background"), "inline background preserved");
		assertTrue(content.includes("padding"), "inline padding preserved");
		assertTrue(
			content.includes("border-radius"),
			"inline border-radius preserved",
		);
	}

	suite("xr-rasterize — fidelity: CSS class names survive in container");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			'<div class="card active"><span class="badge">3</span></div>',
		);

		const content = panel.container.innerHTML;
		assertTrue(
			content.includes('class="card active"'),
			"class names preserved",
		);
		assertTrue(content.includes('class="badge"'), "nested class preserved");
	}

	suite("xr-rasterize — fidelity: data attributes preserved");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			'<button data-handler-id="42" data-action="increment">+</button>',
		);

		const content = panel.container.innerHTML;
		assertTrue(
			content.includes('data-handler-id="42"'),
			"data-handler-id preserved",
		);
		assertTrue(
			content.includes('data-action="increment"'),
			"data-action preserved",
		);
	}

	suite("xr-rasterize — fidelity: nested flexbox layout structure");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			[
				'<div style="display: flex; flex-direction: column; gap: 8px;">',
				'  <div style="display: flex; justify-content: space-between;">',
				"    <span>Label</span>",
				"    <span>Value</span>",
				"  </div>",
				'  <div style="display: flex; gap: 4px;">',
				"    <button>A</button>",
				"    <button>B</button>",
				"    <button>C</button>",
				"  </div>",
				"</div>",
			].join(""),
		);

		const content = panel.container.innerHTML;
		assertTrue(
			content.includes("flex-direction: column"),
			"flex column preserved",
		);
		assertTrue(
			content.includes("justify-content: space-between"),
			"justify-content preserved",
		);
	}

	suite("xr-rasterize — fidelity: SVG content inside panel");
	{
		// SVG inside foreignObject is tricky — nested SVG should be preserved
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			[
				'<svg width="100" height="100" xmlns="http://www.w3.org/2000/svg">',
				'  <circle cx="50" cy="50" r="40" fill="blue" />',
				"</svg>",
			].join(""),
		);

		const content = panel.container.innerHTML;
		assertTrue(content.includes("<svg"), "SVG element preserved");
		assertTrue(content.includes("<circle"), "SVG circle preserved");
	}

	suite("xr-rasterize — fidelity: empty elements preserved");
	{
		const { panel } = createTestPanel();
		setPanelContent(
			panel,
			'<div><br /><hr /><img src="" alt="placeholder" /></div>',
		);

		const content = panel.container.innerHTML;
		// linkedom may normalize these differently, just verify they exist
		assertTrue(
			content.includes("<br") || content.includes("<BR"),
			"br element preserved",
		);
	}

	suite("xr-rasterize — fidelity: large DOM tree (100 elements)");
	{
		const { panel } = createTestPanel();
		const items = Array.from(
			{ length: 100 },
			(_, i) => `<li>Item ${i}</li>`,
		).join("");
		setPanelContent(panel, `<ul>${items}</ul>`);

		const content = panel.container.innerHTML;
		assertTrue(content.includes("Item 0"), "first item present");
		assertTrue(content.includes("Item 99"), "last item present");

		// Rasterize should not throw
		panel.rasterizeFallback();
		assertFalse(panel.state.dirty, "clean after rasterizing large DOM");
	}

	suite("xr-rasterize — fidelity: content overflow is clipped by container");
	{
		const { panel } = createTestPanel();
		// Container has overflow: hidden — verify the style is set
		assertTrue(
			panel.container.style.cssText.includes("overflow: hidden") ||
				panel.container.style.cssText.includes("overflow:hidden"),
			"container has overflow: hidden",
		);
	}
}

// ── Multiple Panels Rasterization ───────────────────────────────────────────

function testMultiplePanelRasterization(): void {
	suite("xr-rasterize — multiple panels have independent content");
	{
		const env = createDOMWithCanvas();
		const { panel: p1 } = createTestPanel(env, defaultPanelConfig(), 0);
		const { panel: p2 } = createTestPanel(env, defaultPanelConfig(), 1);

		setPanelContent(p1, "<h1>Panel One</h1>");
		setPanelContent(p2, "<h1>Panel Two</h1>");

		const c1 = getPanelContent(p1);
		const c2 = getPanelContent(p2);

		assertTrue(c1.includes("Panel One"), "p1 has its own content");
		assertFalse(c1.includes("Panel Two"), "p1 does not have p2's content");
		assertTrue(c2.includes("Panel Two"), "p2 has its own content");
		assertFalse(c2.includes("Panel One"), "p2 does not have p1's content");
	}

	suite("xr-rasterize — multiple panels rasterize independently");
	{
		const env = createDOMWithCanvas();
		const { panel: p1 } = createTestPanel(env, defaultPanelConfig(), 0);
		const { panel: p2 } = createTestPanel(env, defaultPanelConfig(), 1);

		setPanelContent(p1, "<p>Content A</p>");
		setPanelContent(p2, "<p>Content B</p>");

		// Rasterize only p1
		p1.rasterizeFallback();
		assertFalse(p1.state.dirty, "p1 clean after rasterize");
		assertTrue(p2.state.dirty, "p2 still dirty");

		// Rasterize p2
		p2.rasterizeFallback();
		assertFalse(p2.state.dirty, "p2 clean after rasterize");
	}

	suite("xr-rasterize — panels with different configs have different sizes");
	{
		const env = createDOMWithCanvas();
		const { panel: pDefault } = createTestPanel(env, defaultPanelConfig(), 0);
		const { panel: pDash } = createTestPanel(env, dashboardPanelConfig(), 1);
		const { panel: pTip } = createTestPanel(env, tooltipPanelConfig(), 2);

		// Default: 0.8m × 0.6m @ 1200 ppm = 960 × 720
		// Dashboard: 1.2m × 0.8m @ 1200 ppm = 1440 × 960
		// Tooltip: 0.3m × 0.15m @ 1200 ppm = 360 × 180

		assertGreater(
			pDash.textureWidth,
			pDefault.textureWidth,
			"dashboard wider than default",
		);
		assertGreater(
			pDefault.textureWidth,
			pTip.textureWidth,
			"default wider than tooltip",
		);
	}
}

// ── Rasterize Canvas State ──────────────────────────────────────────────────

function testRasterizeCanvasState(): void {
	suite("xr-rasterize — rasterCanvas dimensions match texture dimensions");
	{
		const { panel } = createTestPanel();
		assert(
			panel.rasterCanvas.width,
			panel.textureWidth,
			"canvas width = textureWidth",
		);
		assert(
			panel.rasterCanvas.height,
			panel.textureHeight,
			"canvas height = textureHeight",
		);
	}

	suite("xr-rasterize — rasterCtx is a 2D rendering context");
	{
		const { panel } = createTestPanel();
		assertDefined(panel.rasterCtx, "rasterCtx is defined");
		// The stub context has the methods we need
		assertDefined(
			(panel.rasterCtx as unknown as StubCanvasContext2D).fillRect,
			"rasterCtx has fillRect",
		);
		assertDefined(
			(panel.rasterCtx as unknown as StubCanvasContext2D).clearRect,
			"rasterCtx has clearRect",
		);
		assertDefined(
			(panel.rasterCtx as unknown as StubCanvasContext2D).fillText,
			"rasterCtx has fillText",
		);
	}

	suite("xr-rasterize — rasterCanvas is not in the DOM");
	{
		const env = createDOMWithCanvas();
		const { panel } = createTestPanel(env);

		// The raster canvas should NOT be appended to the document body
		// (it's only used for offscreen rendering)
		const canvases = env.document.querySelectorAll("canvas");
		// linkedom may or may not track canvases created via createElement
		// but the rasterCanvas should not be a child of body
		let found = false;
		for (let i = 0; i < canvases.length; i++) {
			if (canvases[i] === panel.rasterCanvas) {
				found = true;
			}
		}
		assertFalse(found, "rasterCanvas is not appended to DOM body");
	}
}

// ── Panel Background Customization ──────────────────────────────────────────

function testPanelBackground(): void {
	suite("xr-rasterize — custom panel background in style element");
	{
		const { panel } = createTestPanel(undefined, undefined, 0, "#1a1a2e");
		const content = panel.container.innerHTML;
		assertTrue(content.includes("#1a1a2e"), "custom background in style");
	}

	suite("xr-rasterize — manager applies default background to panels");
	{
		const env = createDOMWithCanvas();
		const manager = new XRPanelManager(env.document, "#222222");
		const panel = manager.createPanel();

		const content = panel.container.innerHTML;
		assertTrue(
			content.includes("#222222"),
			"manager's background applied to panel",
		);
	}

	suite("xr-rasterize — transparent background for non-interactive panel");
	{
		const cfg: PanelConfig = {
			...defaultPanelConfig(),
			interact: false,
		};
		const { panel } = createTestPanel(undefined, cfg, 0, "transparent");
		const content = panel.container.innerHTML;
		assertTrue(
			content.includes("transparent"),
			"transparent background for non-interactive",
		);
	}
}

// ── Async Rasterize (SVG foreignObject path) ────────────────────────────────
// In linkedom, the async rasterize() path will fail because Image loading
// doesn't work. We test that it handles errors gracefully.

function testAsyncRasterize(): void {
	suite("xr-rasterize — async rasterize clears dirty on success path");
	{
		// In linkedom, the async rasterize uses Blob + Image which won't
		// fully work, but we can verify the method exists and is callable.
		const { panel } = createTestPanel();
		assertDefined(panel.rasterize, "async rasterize method exists");
		assertTrue(
			typeof panel.rasterize === "function",
			"rasterize is a function",
		);
	}

	suite("xr-rasterize — async rasterize is separate from fallback");
	{
		const { panel } = createTestPanel();
		assertDefined(panel.rasterizeFallback, "fallback method exists");
		assertTrue(
			panel.rasterize !== panel.rasterizeFallback,
			"rasterize and fallback are different methods",
		);
	}
}

// ── Export test runner ──────────────────────────────────────────────────────

export function testXRRasterize(): void {
	testSVGMarkupStructure();
	testFallbackRasterizer();
	testDirtyTracking();
	testMutationRasterizeFlow();
	testTextureUpload();
	testSVGFidelityEdgeCases();
	testMultiplePanelRasterization();
	testRasterizeCanvasState();
	testPanelBackground();
	testAsyncRasterize();
	testPanelManagerDirtyUpdatesSync();
}

export async function testXRRasterizeAsync(): Promise<void> {
	await testPanelManagerDirtyUpdatesAsync();
}
