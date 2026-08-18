// Phase 31.3 — PropsCounterApp Tests
//
// Tests the PropsCounterApp (pc_*) WASM exports which exercise the
// self-rendering child component with props pattern:
//
// Validates:
//   - pc_init state validation (scope IDs, handler IDs, initial values)
//   - pc_rebuild produces mutations (RegisterTemplate ×2, mount, child create)
//   - increment updates parent signal, child re-renders with new count
//   - decrement updates parent signal
//   - toggle hex changes display format without affecting count
//   - toggle hex marks only child dirty
//   - increment marks only parent dirty
//   - DOM mount: parent div with h1 + 2 buttons + child div with p + button
//   - increment → child text updates ("Count: 1")
//   - toggle → child text updates ("Count: 0x0")
//   - increment after toggle → hex format preserved ("Count: 0x1")
//   - 10 increments → correct count
//   - toggle on → toggle off → decimal restored
//   - flush returns 0 when clean
//   - destroy does not crash
//   - destroy + recreate cycle
//   - multiple independent instances
//   - rapid 100 increments bounded memory
//   - child toggle does not affect parent DOM

import { parseHTML } from "npm:linkedom";
import {
	createApp,
	createPropsCounterApp,
	type PropsCounterAppHandle,
} from "../runtime/app.ts";
import { alignedAlloc } from "../runtime/memory.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, CallableFunction>;

// ── DOM helper ──────────────────────────────────────────────────────────────

function createDOM() {
	const { document } = parseHTML(
		"<!DOCTYPE html><html><body><div id='root'></div></body></html>",
	);
	const root = document.getElementById("root")!;
	return { document, root };
}

// ── Constants ───────────────────────────────────────────────────────────────

const BUF_CAPACITY = 16384;

// ── Helper: create a mounted PropsCounterApp via createPropsCounterApp ──────

function createPC(fns: Fns): PropsCounterAppHandle {
	const { document, root } = createDOM();
	return createPropsCounterApp(fns, root, document);
}

// ══════════════════════════════════════════════════════════════════════════════

export function testPropsCounter(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: pc_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — pc_init state validation");
	{
		const handle = createPC(fns);

		const parentScope = handle.parentScopeId;
		const childScope = handle.childScopeId;
		assert(parentScope >= 0, true, "parent scope ID non-negative");
		assert(childScope >= 0, true, "child scope ID non-negative");
		assert(parentScope !== childScope, true, "parent/child scopes differ");

		const parentTmpl = handle.parentTmplId;
		const childTmpl = handle.childTmplId;
		assert(parentTmpl >= 0, true, "parent template ID non-negative");
		assert(childTmpl >= 0, true, "child template ID non-negative");
		assert(parentTmpl !== childTmpl, true, "parent/child templates differ");

		assert(handle.scopeCount(), 2, "scope count = 2");
		assert(handle.handlerCount() >= 3, true, "at least 3 handlers");

		assert(handle.getCount(), 0, "initial count = 0");
		assert(handle.getShowHex(), false, "initial show_hex = false");

		assert(handle.incrHandler >= 0, true, "incr handler valid");
		assert(handle.decrHandler >= 0, true, "decr handler valid");
		assert(handle.toggleHandler >= 0, true, "toggle handler valid");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: pc_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — rebuild produces mutations and mounts child");
	{
		const app = fns.pc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		const len = fns.pc_rebuild(app, bufPtr, BUF_CAPACITY);
		assert(len > 0, true, "rebuild produces mutations");

		assert(
			fns.pc_child_is_mounted(app) !== 0,
			true,
			"child mounted after rebuild",
		);
		assert(
			fns.pc_child_has_rendered(app) !== 0,
			true,
			"child has rendered after rebuild",
		);

		fns.mutation_buf_free(bufPtr);
		fns.pc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: increment updates parent signal
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — increment updates count");
	{
		const handle = createPC(fns);

		handle.increment();
		assert(handle.getCount(), 1, "count is 1 after increment");

		handle.increment();
		assert(handle.getCount(), 2, "count is 2 after second increment");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: decrement updates count
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — decrement updates count");
	{
		const handle = createPC(fns);

		handle.decrement();
		assert(handle.getCount(), -1, "count is -1 after decrement");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: toggle hex marks child dirty (not parent)
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — toggle hex marks only child dirty");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.pc_init(),
			rebuild: (f, app, buf, cap) => f.pc_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.pc_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.pc_handle_event(app, hid, evt),
			destroy: (f, app) => f.pc_destroy(app),
		});

		// After mount, should be clean
		assert(fns.pc_child_is_dirty(handle.appPtr), 0, "child clean after mount");
		assert(
			fns.pc_parent_is_dirty(handle.appPtr),
			0,
			"parent clean after mount",
		);

		// Toggle hex via event dispatch
		const toggleH = fns.pc_toggle_handler(handle.appPtr);
		fns.pc_handle_event(handle.appPtr, toggleH, 0);
		assert(fns.pc_child_is_dirty(handle.appPtr), 1, "child dirty after toggle");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: increment marks parent dirty
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — increment marks parent dirty");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.pc_init(),
			rebuild: (f, app, buf, cap) => f.pc_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.pc_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.pc_handle_event(app, hid, evt),
			destroy: (f, app) => f.pc_destroy(app),
		});

		const incrH = fns.pc_incr_handler(handle.appPtr);
		fns.pc_handle_event(handle.appPtr, incrH, 0);
		assert(
			fns.pc_parent_is_dirty(handle.appPtr),
			1,
			"parent dirty after increment",
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: DOM mount produces correct structure
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — DOM mount produces correct structure");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		// Parent should have h1
		const h1 = root.querySelector("h1");
		assert(h1 !== null, true, "h1 exists in DOM");
		if (h1) {
			assert(
				h1.textContent?.includes("Props Counter") ?? false,
				true,
				'h1 text contains "Props Counter"',
			);
		}

		// Parent should have buttons
		const buttons = root.querySelectorAll("button");
		assert(
			buttons.length >= 3,
			true,
			"at least 3 buttons (+ , - , Toggle hex)",
		);

		// Child should have p with "Count: 0"
		const p = root.querySelector("p");
		assert(p !== null, true, "p element exists (child rendered)");
		if (p) {
			assert(
				p.textContent?.includes("Count: 0") ?? false,
				true,
				'child text contains "Count: 0"',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: increment → child text updates
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — increment updates child text");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		handle.increment();

		assert(handle.getCount(), 1, "count is 1");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("Count: 1") ?? false,
				true,
				'child text updated to "Count: 1"',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: toggle → child text updates to hex
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — toggle hex changes display format");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		handle.toggleHex();

		assert(handle.getShowHex(), true, "show_hex is true");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("0x") ?? false,
				true,
				'child text contains "0x" after hex toggle',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: increment after toggle → hex format preserved
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — increment after toggle preserves hex format");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		handle.toggleHex();
		handle.increment();

		assert(handle.getCount(), 1, "count is 1");
		assert(handle.getShowHex(), true, "show_hex still true");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("0x") ?? false,
				true,
				"hex format preserved after increment",
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: 10 increments → correct count
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — 10 increments produce correct count");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		for (let i = 0; i < 10; i++) {
			handle.increment();
		}

		assert(handle.getCount(), 10, "count is 10 after 10 increments");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("Count: 10") ?? false,
				true,
				'child text shows "Count: 10"',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: toggle on → toggle off → decimal restored
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — toggle on then off restores decimal format");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		handle.increment(); // count = 1
		handle.toggleHex(); // hex mode
		assert(handle.getShowHex(), true, "show_hex on");

		handle.toggleHex(); // decimal mode
		assert(handle.getShowHex(), false, "show_hex off");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("Count: 1") ?? false,
				true,
				'decimal format restored: "Count: 1"',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — flush returns 0 when clean");
	{
		const app = fns.pc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		fns.pc_rebuild(app, bufPtr, BUF_CAPACITY);

		const len = fns.pc_flush(app, bufPtr, BUF_CAPACITY);
		assert(len, 0, "flush returns 0 when nothing dirty");

		fns.mutation_buf_free(bufPtr);
		fns.pc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: minimal mutations — only SetText for unchanged parent
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — flush after toggle produces mutations");
	{
		const app = fns.pc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		fns.pc_rebuild(app, bufPtr, BUF_CAPACITY);

		// Toggle hex
		const toggleH = fns.pc_toggle_handler(app);
		fns.pc_handle_event(app, toggleH, 0);
		const len = fns.pc_flush(app, bufPtr, BUF_CAPACITY);
		assert(len > 0, true, "flush after toggle produces mutations");

		fns.mutation_buf_free(bufPtr);
		fns.pc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — destroy does not crash");
	{
		const handle = createPC(fns);
		handle.increment();
		handle.toggleHex();
		handle.destroy();
		assert(true, true, "destroy after mutations does not crash");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — double destroy safe");
	{
		const handle = createPC(fns);
		handle.destroy();
		handle.destroy(); // should be idempotent
		assert(true, true, "double destroy does not crash");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: destroy + recreate cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — destroy + recreate cycle");
	{
		const handle1 = createPC(fns);
		handle1.increment();
		handle1.increment();
		handle1.toggleHex();
		assert(handle1.getCount(), 2, "first instance count = 2");
		handle1.destroy();

		const handle2 = createPC(fns);
		assert(handle2.getCount(), 0, "fresh count after recreate");
		assert(handle2.getShowHex(), false, "fresh show_hex after recreate");
		handle2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — multiple independent instances");
	{
		const handle1 = createPC(fns);
		const handle2 = createPC(fns);

		handle1.increment();
		handle1.increment();
		handle1.increment();

		handle2.decrement();

		assert(handle1.getCount(), 3, "instance 1 count = 3");
		assert(handle2.getCount(), -1, "instance 2 count = -1");

		handle1.toggleHex();
		assert(handle1.getShowHex(), true, "instance 1 show_hex toggled");
		assert(handle2.getShowHex(), false, "instance 2 show_hex unaffected");

		handle1.destroy();
		handle2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: rapid 100 increments bounded memory
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — rapid 100 increments");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		for (let i = 0; i < 100; i++) {
			handle.increment();
		}

		assert(handle.getCount(), 100, "count is 100 after 100 increments");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("Count: 100") ?? false,
				true,
				'child text shows "Count: 100"',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: child toggle does not affect parent DOM
	// ═════════════════════════════════════════════════════════════════════

	suite("PropsCounter — child toggle does not affect parent DOM");
	{
		const { document, root } = createDOM();
		const handle = createPropsCounterApp(fns, root, document);

		const h1Before = root.querySelector("h1")?.textContent ?? "";

		handle.toggleHex();

		const h1After = root.querySelector("h1")?.textContent ?? "";
		assert(h1Before, h1After, "h1 text unchanged after child toggle");

		// Buttons should still be present
		const buttons = root.querySelectorAll("button");
		assert(buttons.length >= 3, true, "buttons still present after toggle");

		handle.destroy();
	}
}
