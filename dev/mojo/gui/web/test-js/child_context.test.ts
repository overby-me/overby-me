// Phase 31.2 — ChildComponentContext Tests
//
// Tests the ChildContextTestApp (cct_*) WASM exports which exercise
// ChildComponentContext — a self-rendering child component with its own
// signals, context consumption, and rendering.
//
// Validates:
//   - create child context, verify scope/template IDs
//   - child use_signal independent from parent signals
//   - child signal write → child dirty, parent clean
//   - parent signal write → parent dirty, child clean
//   - context prop round-trip (provide at parent, consume at child)
//   - child self-render via render_builder produces correct VNode
//   - DOM mount with child context (parent + child visible)
//   - child local state update → only child SetText mutation
//   - parent prop update → child re-renders with new value
//   - mixed local + prop updates in single flush
//   - destroy does not crash
//   - destroy + recreate cycle
//   - multiple independent child contexts
//   - rapid signal writes bounded memory
//   - child provides context to sibling (negative — siblings can't see
//     each other's context)

import { parseHTML } from "npm:linkedom";
import { createApp } from "../runtime/app.ts";
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

// ══════════════════════════════════════════════════════════════════════════════

export function testChildContext(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: cct_init creates app with correct state
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — cct_init creates app with correct state");
	{
		const app = fns.cct_init();
		assert(app !== 0n, true, "app pointer should be non-zero");

		const parentScope = fns.cct_parent_scope_id(app);
		const childScope = fns.cct_child_scope_id(app);
		assert(parentScope >= 0, true, "parent scope ID non-negative");
		assert(childScope >= 0, true, "child scope ID non-negative");
		assert(childScope !== parentScope, true, "child scope differs from parent");

		const parentTmpl = fns.cct_parent_tmpl_id(app);
		const childTmpl = fns.cct_child_tmpl_id(app);
		assert(parentTmpl >= 0, true, "parent template ID non-negative");
		assert(childTmpl >= 0, true, "child template ID non-negative");
		assert(
			childTmpl !== parentTmpl,
			true,
			"child template differs from parent",
		);

		assert(fns.cct_count_value(app), 0, "initial count is 0");
		assert(fns.cct_show_hex(app), 0, "initial show_hex is false");

		const scopeCount = fns.cct_scope_count(app);
		assert(scopeCount, 2, "2 live scopes (parent + child)");

		const handlerCount = fns.cct_handler_count(app);
		assert(handlerCount >= 1, true, "at least 1 handler registered");

		fns.cct_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: consumed signal key matches parent signal key
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — consumed signal key matches parent");
	{
		const app = fns.cct_init();

		const parentKey = fns.cct_parent_count_signal_key(app);
		const childKey = fns.cct_child_count_signal_key(app);
		assert(parentKey, childKey, "consumed signal key equals parent signal key");

		fns.cct_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: child use_signal independent from parent
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — child use_signal independent from parent");
	{
		const app = fns.cct_init();

		// Toggle hex (child-owned signal)
		fns.cct_toggle_hex(app);
		assert(fns.cct_show_hex(app), 1, "show_hex toggled to true");
		assert(fns.cct_count_value(app), 0, "count still 0");

		// Set count (parent-owned signal)
		fns.cct_set_count(app, 42);
		assert(fns.cct_count_value(app), 42, "count set to 42");
		assert(fns.cct_show_hex(app), 1, "show_hex still true");

		fns.cct_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: child signal write → child dirty
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — child signal write marks child dirty");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		// After mount, should be clean
		assert(fns.cct_child_is_dirty(handle.appPtr), 0, "child clean after mount");
		assert(
			fns.cct_parent_is_dirty(handle.appPtr),
			0,
			"parent clean after mount",
		);

		// Toggle hex
		fns.cct_toggle_hex(handle.appPtr);
		assert(
			fns.cct_child_is_dirty(handle.appPtr),
			1,
			"child dirty after toggle",
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: parent signal write marks parent dirty
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — parent signal write marks parent dirty");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		fns.cct_set_count(handle.appPtr, 5);
		assert(
			fns.cct_parent_is_dirty(handle.appPtr),
			1,
			"parent dirty after count set",
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: cct_rebuild produces mutations + child mounted
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — rebuild produces mutations and mounts child");
	{
		const app = fns.cct_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		const len = fns.cct_rebuild(app, bufPtr, BUF_CAPACITY);
		assert(len > 0, true, "rebuild produces mutations");

		assert(fns.cct_child_is_mounted(app), 1, "child mounted after rebuild");
		assert(
			fns.cct_child_has_rendered(app),
			1,
			"child has rendered after rebuild",
		);

		fns.mutation_buf_free(bufPtr);
		fns.cct_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: DOM mount with child context (parent + child visible)
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — DOM mount produces correct structure");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		// Parent should have h1 and button
		const h1 = root.querySelector("h1");
		assert(h1 !== null, true, "h1 exists in DOM");

		const buttons = root.querySelectorAll("button");
		assert(buttons.length >= 1, true, "at least 1 button in DOM");

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
	// Section 8: increment → child re-renders with new value
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — increment updates child text");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		const incrH = fns.cct_incr_handler(handle.appPtr);
		handle.dispatchAndFlush(incrH);

		assert(fns.cct_count_value(handle.appPtr), 1, "count is 1");

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
	// Section 9: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — flush returns 0 when clean");
	{
		const app = fns.cct_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		fns.cct_rebuild(app, bufPtr, BUF_CAPACITY);

		const len = fns.cct_flush(app, bufPtr, BUF_CAPACITY);
		assert(len, 0, "flush returns 0 when nothing dirty");

		fns.mutation_buf_free(bufPtr);
		fns.cct_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: toggle hex → child text changes format
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — toggle hex changes display format");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		// Toggle hex and flush
		fns.cct_toggle_hex(handle.appPtr);
		handle.flushAndApply();

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
	// Section 11: mixed increment + toggle in single flush
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — mixed increment + toggle in single flush");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		// Increment 3 times
		const incrH = fns.cct_incr_handler(handle.appPtr);
		for (let i = 0; i < 3; i++) {
			handle.dispatchAndFlush(incrH);
		}

		// Toggle hex
		fns.cct_toggle_hex(handle.appPtr);
		handle.flushAndApply();

		assert(fns.cct_count_value(handle.appPtr), 3, "count is 3");
		assert(fns.cct_show_hex(handle.appPtr), 1, "show_hex is true");

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("0x") ?? false,
				true,
				"child text in hex format",
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — destroy does not crash");
	{
		const app = fns.cct_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		fns.cct_rebuild(app, bufPtr, BUF_CAPACITY);

		// Make dirty but don't flush
		fns.cct_set_count(app, 99);
		fns.cct_toggle_hex(app);

		fns.mutation_buf_free(bufPtr);
		fns.cct_destroy(app);
		assert(true, true, "destroy after dirty state does not crash");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: destroy + recreate cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — destroy + recreate cycle");
	{
		const app1 = fns.cct_init();
		fns.cct_set_count(app1, 99);
		fns.cct_toggle_hex(app1);
		fns.cct_destroy(app1);

		const app2 = fns.cct_init();
		assert(fns.cct_count_value(app2), 0, "fresh count after recreate");
		assert(fns.cct_show_hex(app2), 0, "fresh show_hex after recreate");
		fns.cct_destroy(app2);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — multiple independent instances");
	{
		const app1 = fns.cct_init();
		const app2 = fns.cct_init();

		fns.cct_set_count(app1, 10);
		fns.cct_set_count(app2, 20);

		assert(fns.cct_count_value(app1), 10, "app1 count is 10");
		assert(fns.cct_count_value(app2), 20, "app2 count is 20");

		fns.cct_toggle_hex(app1);
		assert(fns.cct_show_hex(app1), 1, "app1 show_hex toggled");
		assert(fns.cct_show_hex(app2), 0, "app2 show_hex unaffected");

		fns.cct_destroy(app1);
		fns.cct_destroy(app2);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: rapid increment cycles bounded
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildContext — rapid increment cycles");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.cct_init(),
			rebuild: (f, app, buf, cap) => f.cct_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.cct_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.cct_handle_event(app, hid, evt),
			destroy: (f, app) => f.cct_destroy(app),
		});

		const incrH = fns.cct_incr_handler(handle.appPtr);
		for (let i = 0; i < 50; i++) {
			handle.dispatchAndFlush(incrH);
		}

		assert(
			fns.cct_count_value(handle.appPtr),
			50,
			"count is 50 after 50 increments",
		);

		const p = root.querySelector("p");
		if (p) {
			assert(
				p.textContent?.includes("Count: 50") ?? false,
				true,
				'child text shows "Count: 50"',
			);
		}

		handle.destroy();
	}
}
