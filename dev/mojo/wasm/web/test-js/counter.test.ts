// Counter App End-to-End Tests
//
// Tests the full counter app lifecycle with the Dioxus-like ergonomic API:
//   init → rebuild → mount → click → flush → DOM update
//
// The counter app uses register_view() with inline event handlers:
//   div > [ h1 > dyn_text("High-Five counter: N"),
//           button("Up high!") + onclick_add(count, 1),
//           button("Down low!") + onclick_sub(count, 1) ]
//
// Uses linkedom for headless DOM and the WASM counter_* exports
// orchestrated through createCounterApp().

import { parseHTML } from "npm:linkedom";
import { createCounterApp } from "../runtime/app.ts";
import { alignedAlloc, getMemory } from "../runtime/memory.ts";
import { MutationReader, Op } from "../runtime/protocol.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, pass, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, unknown>;

// ── DOM helper ──────────────────────────────────────────────────────────────

function createDOM() {
	const { document, window } = parseHTML(
		"<!DOCTYPE html><html><body><div id='root'></div></body></html>",
	);
	const root = document.getElementById("root")!;
	return { document, window, root };
}

// ── Constants ───────────────────────────────────────────────────────────────

const EVT_CLICK = 0;
const BUF_CAPACITY = 16384;

// ══════════════════════════════════════════════════════════════════════════════

export function testCounter(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: Low-level counter app exports
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — counter_init creates app with correct state");
	{
		const app = fns.counter_init();

		// Query exports should return valid values
		const rtPtr = fns.counter_rt_ptr(app);
		assert(rtPtr !== 0n, true, "runtime pointer is non-zero");

		const tmplId = fns.counter_tmpl_id(app);
		assert(tmplId >= 0, true, "template ID is non-negative");

		const incrH = fns.counter_incr_handler(app);
		assert(incrH >= 0, true, "incr handler ID is non-negative");

		const decrH = fns.counter_decr_handler(app);
		assert(decrH >= 0, true, "decr handler ID is non-negative");
		assert(incrH !== decrH, true, "incr and decr handlers are different");

		const scopeId = fns.counter_scope_id(app);
		assert(scopeId >= 0, true, "scope ID is non-negative");

		const sigKey = fns.counter_count_signal(app);
		assert(sigKey >= 0, true, "signal key is non-negative");

		// Initial count is 0
		assert(fns.counter_count_value(app), 0, "initial count is 0");

		// No dirty scopes yet
		assert(fns.counter_has_dirty(app), 0, "no dirty scopes initially");

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: counter_rebuild emits mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — counter_rebuild produces mutation buffer");
	{
		const app = fns.counter_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		const offset = fns.counter_rebuild(app, bufPtr, BUF_CAPACITY);
		assert(offset > 0, true, "rebuild wrote mutations (offset > 0)");

		// Decode the mutations
		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			offset,
		).readAll();

		assert(mutations.length > 0, true, "at least one mutation decoded");

		// Verify we have a LoadTemplate mutation
		const loadTemplates = mutations.filter((m) => m.op === 0x05);
		assert(loadTemplates.length > 0, true, "contains LoadTemplate mutation");

		// Verify we have an AppendChildren mutation (mounting to root)
		const appendChildren = mutations.filter((m) => m.op === 0x01);
		assert(appendChildren.length > 0, true, "contains AppendChildren mutation");

		// Verify we have a SetText mutation for "High-Five counter: 0"
		const setTexts = mutations.filter(
			(m) => m.op === 0x0b && "text" in m,
		) as Array<{ op: number; id: number; text: string }>;
		const countText = setTexts.find((m) => m.text === "High-Five counter: 0");
		assert(
			countText !== undefined,
			true,
			'SetText with "High-Five counter: 0" found',
		);

		// Verify we have NewEventListener mutations for click handlers
		const newListeners = mutations.filter((m) => m.op === 0x0c);
		assert(
			newListeners.length >= 2,
			true,
			"at least 2 NewEventListener mutations (+ and - buttons)",
		);

		// Verify RegisterTemplate appears before LoadTemplate (M11.5 prepend strategy)
		const regIdx = mutations.findIndex((m) => m.op === Op.RegisterTemplate);
		const loadIdx = mutations.findIndex((m) => m.op === Op.LoadTemplate);
		assert(regIdx >= 0, true, "contains RegisterTemplate mutation");
		assert(loadIdx >= 0, true, "contains LoadTemplate mutation");
		assert(
			regIdx < loadIdx,
			true,
			"RegisterTemplate precedes LoadTemplate in mutation buffer",
		);

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: counter_handle_event modifies signal
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — counter_handle_event increments signal");
	{
		const app = fns.counter_init();

		const incrH = fns.counter_incr_handler(app);
		const decrH = fns.counter_decr_handler(app);

		// Increment
		const r1 = fns.counter_handle_event(app, incrH, EVT_CLICK);
		assert(r1, 1, "increment dispatch returned 1 (action executed)");
		assert(fns.counter_count_value(app), 1, "count is 1 after increment");
		assert(fns.counter_has_dirty(app), 1, "scope is dirty after event");

		// Increment again
		fns.counter_handle_event(app, incrH, EVT_CLICK);
		assert(fns.counter_count_value(app), 2, "count is 2 after 2nd increment");

		// Decrement
		fns.counter_handle_event(app, decrH, EVT_CLICK);
		assert(fns.counter_count_value(app), 1, "count is 1 after decrement");

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: counter_flush produces diff mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — counter_flush emits SetText after increment");
	{
		const app = fns.counter_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		// Initial rebuild
		fns.counter_rebuild(app, bufPtr, BUF_CAPACITY);

		// Dispatch increment
		const incrH = fns.counter_incr_handler(app);
		fns.counter_handle_event(app, incrH, EVT_CLICK);

		// Flush
		const flushLen = fns.counter_flush(app, bufPtr, BUF_CAPACITY);
		assert(flushLen > 0, true, "flush wrote mutations (len > 0)");

		// Decode flush mutations
		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			flushLen,
		).readAll();

		// Should contain a SetText mutation for "High-Five counter: 1"
		const setTexts = mutations.filter(
			(m) => m.op === 0x0b && "text" in m,
		) as Array<{ op: number; id: number; text: string }>;
		const countText = setTexts.find((m) => m.text === "High-Five counter: 1");
		assert(
			countText !== undefined,
			true,
			'flush contains SetText "High-Five counter: 1"',
		);

		// After flush, no more dirty scopes
		assert(fns.counter_has_dirty(app), 0, "clean after flush");

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: counter_flush with no changes returns 0
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — counter_flush returns 0 when nothing dirty");
	{
		const app = fns.counter_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		// Rebuild (no events dispatched)
		fns.counter_rebuild(app, bufPtr, BUF_CAPACITY);

		// Flush without any event → should return 0
		const len = fns.counter_flush(app, bufPtr, BUF_CAPACITY);
		assert(len, 0, "flush returns 0 when nothing dirty");

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: Multiple flush cycles produce correct text
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — multiple increment/flush cycles update text correctly");
	{
		const app = fns.counter_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		fns.counter_rebuild(app, bufPtr, BUF_CAPACITY);
		const incrH = fns.counter_incr_handler(app);
		const mem = getMemory();

		for (let i = 1; i <= 5; i++) {
			fns.counter_handle_event(app, incrH, EVT_CLICK);
			const len = fns.counter_flush(app, bufPtr, BUF_CAPACITY);
			assert(len > 0, true, `flush ${i} produced mutations`);

			const mutations = new MutationReader(
				mem.buffer,
				Number(bufPtr),
				len,
			).readAll();

			const setTexts = mutations.filter(
				(m) => m.op === 0x0b && "text" in m,
			) as Array<{ op: number; id: number; text: string }>;
			const expected = `High-Five counter: ${i}`;
			const found = setTexts.find((m) => m.text === expected);
			assert(
				found !== undefined,
				true,
				`flush ${i} contains SetText "${expected}"`,
			);
		}

		assert(fns.counter_count_value(app), 5, "count is 5 after 5 increments");

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: Mixed increment/decrement
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — mixed increment/decrement produces correct count");
	{
		const app = fns.counter_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		fns.counter_rebuild(app, bufPtr, BUF_CAPACITY);
		const incrH = fns.counter_incr_handler(app);
		const decrH = fns.counter_decr_handler(app);
		const mem = getMemory();

		// +1, +1, +1, -1, -1 → count = 1
		fns.counter_handle_event(app, incrH, EVT_CLICK);
		fns.counter_flush(app, bufPtr, BUF_CAPACITY);

		fns.counter_handle_event(app, incrH, EVT_CLICK);
		fns.counter_flush(app, bufPtr, BUF_CAPACITY);

		fns.counter_handle_event(app, incrH, EVT_CLICK);
		fns.counter_flush(app, bufPtr, BUF_CAPACITY);

		fns.counter_handle_event(app, decrH, EVT_CLICK);
		fns.counter_flush(app, bufPtr, BUF_CAPACITY);

		fns.counter_handle_event(app, decrH, EVT_CLICK);
		const len = fns.counter_flush(app, bufPtr, BUF_CAPACITY);

		assert(fns.counter_count_value(app), 1, "count is 1 after +3 -2");

		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			len,
		).readAll();

		const setTexts = mutations.filter(
			(m) => m.op === 0x0b && "text" in m,
		) as Array<{ op: number; id: number; text: string }>;
		const found = setTexts.find((m) => m.text === "High-Five counter: 1");
		assert(
			found !== undefined,
			true,
			'final flush has SetText "High-Five counter: 1"',
		);

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: Template registered in runtime via counter_init
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — template is queryable from runtime");
	{
		const app = fns.counter_init();
		const rtPtr = fns.counter_rt_ptr(app);
		const tmplId = fns.counter_tmpl_id(app);

		// Template should be registered
		assert(fns.tmpl_count(rtPtr) >= 1, true, "at least 1 template");

		// Root count: 1 (single div root)
		assert(fns.tmpl_root_count(rtPtr, tmplId), 1, "template has 1 root");

		// Root is an element node
		const rootIdx = fns.tmpl_get_root_index(rtPtr, tmplId, 0);
		assert(
			fns.tmpl_node_kind(rtPtr, tmplId, rootIdx),
			0,
			"root is TNODE_ELEMENT (0)",
		);

		// Root is a div (TAG_DIV = 0)
		assert(fns.tmpl_node_tag(rtPtr, tmplId, rootIdx), 0, "root tag is div");

		// Root has 5 children: h1, button(up), button(down), button(toggle), dyn_node
		const childCount = fns.tmpl_node_child_count(rtPtr, tmplId, rootIdx);
		assert(childCount, 5, "root div has 5 children");

		// First child is h1 (TAG_H1 = 10) — "High-Five counter: N"
		const h1Idx = fns.tmpl_node_child_at(rtPtr, tmplId, rootIdx, 0);
		assert(fns.tmpl_node_tag(rtPtr, tmplId, h1Idx), 10, "first child is h1");

		// Second child is button (TAG_BUTTON = 19)
		const btn1Idx = fns.tmpl_node_child_at(rtPtr, tmplId, rootIdx, 1);
		assert(
			fns.tmpl_node_tag(rtPtr, tmplId, btn1Idx),
			19,
			"second child is button",
		);

		// Third child is button
		const btn2Idx = fns.tmpl_node_child_at(rtPtr, tmplId, rootIdx, 2);
		assert(
			fns.tmpl_node_tag(rtPtr, tmplId, btn2Idx),
			19,
			"third child is button",
		);

		// Fourth child is button (toggle detail)
		const btn3Idx = fns.tmpl_node_child_at(rtPtr, tmplId, rootIdx, 3);
		assert(
			fns.tmpl_node_tag(rtPtr, tmplId, btn3Idx),
			19,
			"fourth child is button (toggle)",
		);

		// Template has dynamic text nodes
		assert(
			fns.tmpl_dynamic_text_count(rtPtr, tmplId) >= 1,
			true,
			"template has at least 1 dynamic text slot",
		);

		// Template has dynamic attrs
		assert(
			fns.tmpl_dynamic_attr_count(rtPtr, tmplId) >= 3,
			true,
			"template has at least 3 dynamic attr slots",
		);

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: Full DOM integration — createCounterApp
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — createCounterApp mounts to DOM");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Root should have children after mount
		assert(
			dom.root.childNodes.length > 0,
			true,
			"root has children after mount",
		);

		// Should have a div as the first child
		const divEl = dom.root.childNodes[0] as Element;
		assert(divEl.nodeName.toLowerCase(), "div", "first child is a div");

		// Div should have 5 children: h1, button(up), button(down), button(toggle), placeholder
		assert(divEl.childNodes.length, 5, "div has 5 children");

		const h1El = divEl.childNodes[0] as Element;
		assert(h1El.nodeName.toLowerCase(), "h1", "first div child is h1");

		const btn1 = divEl.childNodes[1] as Element;
		assert(btn1.nodeName.toLowerCase(), "button", "second div child is button");

		const btn2 = divEl.childNodes[2] as Element;
		assert(btn2.nodeName.toLowerCase(), "button", "third div child is button");

		const btn3 = divEl.childNodes[3] as Element;
		assert(
			btn3.nodeName.toLowerCase(),
			"button",
			"fourth div child is button (toggle)",
		);

		// Button text content
		assert(btn1.textContent, "Up high!", 'first button text is "Up high!"');
		assert(btn2.textContent, "Down low!", 'second button text is "Down low!"');

		// H1 should display "High-Five counter: 0"
		assert(
			h1El.textContent,
			"High-Five counter: 0",
			'h1 displays "High-Five counter: 0"',
		);

		// Initial count
		assert(handle.getCount(), 0, "getCount() returns 0");
		assert(handle.getDoubled(), 0, "getDoubled() returns 0 (computed inline)");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: Full DOM integration — increment updates display
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — increment updates DOM text");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl.childNodes[0] as Element;

		// Click increment
		handle.increment();

		assert(handle.getCount(), 1, "count is 1 after increment");
		assert(
			h1El.textContent,
			"High-Five counter: 1",
			'h1 displays "High-Five counter: 1" after increment',
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: Full DOM integration — multiple increments
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — 10 increments updates DOM correctly");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl.childNodes[0] as Element;

		for (let i = 0; i < 10; i++) {
			handle.increment();
		}

		assert(handle.getCount(), 10, "count is 10 after 10 increments");
		assert(
			h1El.textContent,
			"High-Five counter: 10",
			'h1 displays "High-Five counter: 10"',
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: Full DOM integration — decrement
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — decrement updates DOM text");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl.childNodes[0] as Element;

		// Increment 3 times, decrement once → 2
		handle.increment();
		handle.increment();
		handle.increment();
		handle.decrement();

		assert(handle.getCount(), 2, "count is 2 after +3 -1");
		assert(
			h1El.textContent,
			"High-Five counter: 2",
			'h1 displays "High-Five counter: 2"',
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: Full DOM integration — decrement below zero
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — decrement below zero works");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl.childNodes[0] as Element;

		handle.decrement();
		handle.decrement();

		assert(handle.getCount(), -2, "count is -2 after 2 decrements");
		assert(
			h1El.textContent,
			"High-Five counter: -2",
			'h1 displays "High-Five counter: -2"',
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: Minimal mutations per click
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — minimal mutations per click (only SetText)");
	{
		const app = fns.counter_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		fns.counter_rebuild(app, bufPtr, BUF_CAPACITY);

		const incrH = fns.counter_incr_handler(app);
		fns.counter_handle_event(app, incrH, EVT_CLICK);

		const len = fns.counter_flush(app, bufPtr, BUF_CAPACITY);
		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			len,
		).readAll();

		// The diff should produce only a SetText mutation (0x0b)
		// No LoadTemplate, no AppendChildren, etc.
		const nonSetText = mutations.filter((m) => m.op !== 0x0b);
		assert(
			nonSetText.length,
			0,
			"flush contains only SetText mutations (minimal diff)",
		);

		assert(mutations.length, 1, "exactly 1 mutation (SetText for count)");

		fns.counter_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: DOM structure preserved across updates
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — DOM structure preserved across updates");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Click 5 times
		for (let i = 0; i < 5; i++) {
			handle.increment();
		}

		// Structure should be the same objects (no re-creation)
		const newDiv = dom.root.childNodes[0] as Element;
		assert(newDiv.childNodes.length, 5, "div still has 5 children");

		// Buttons should still have their text
		assert(
			(newDiv.childNodes[1] as Element).textContent,
			"Up high!",
			'button 1 still says "Up high!"',
		);
		assert(
			(newDiv.childNodes[2] as Element).textContent,
			"Down low!",
			'button 2 still says "Down low!"',
		);

		// H1 should show updated count
		assert(
			(newDiv.childNodes[0] as Element).textContent,
			"High-Five counter: 5",
			'h1 says "High-Five counter: 5"',
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: Multiple independent app instances
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — multiple independent app instances");
	{
		const dom1 = createDOM();
		const dom2 = createDOM();

		const h1 = createCounterApp(fns, dom1.root, dom1.document);
		const h2 = createCounterApp(fns, dom2.root, dom2.document);

		// Increment h1 three times
		h1.increment();
		h1.increment();
		h1.increment();

		// Increment h2 once
		h2.increment();

		assert(h1.getCount(), 3, "app1 count is 3");
		assert(h2.getCount(), 1, "app2 count is 1");

		const span1 = (dom1.root.childNodes[0] as Element).childNodes[0] as Element;
		const span2 = (dom2.root.childNodes[0] as Element).childNodes[0] as Element;

		assert(
			span1.textContent,
			"High-Five counter: 3",
			'app1 h1 shows "High-Five counter: 3"',
		);
		assert(
			span2.textContent,
			"High-Five counter: 1",
			'app2 h1 shows "High-Five counter: 1"',
		);

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: Rapid increment stress test
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — rapid 100 increments");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		for (let i = 0; i < 100; i++) {
			handle.increment();
		}

		assert(handle.getCount(), 100, "count is 100 after 100 increments");

		const h1El = (dom.root.childNodes[0] as Element).childNodes[0] as Element;
		assert(
			h1El.textContent,
			"High-Five counter: 100",
			'h1 displays "High-Five counter: 100"',
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: Destroy cleans up
	// ═════════════════════════════════════════════════════════════════════

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: Memo demo — doubled value
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — getDoubled returns computed inline value");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		assert(handle.getDoubled(), 0, "getDoubled() is 0 initially");
		handle.increment();
		assert(handle.getDoubled(), 2, "getDoubled() is 2 after increment");
		handle.increment();
		assert(handle.getDoubled(), 4, "getDoubled() is 4 after 2 increments");
		handle.decrement();
		assert(handle.getDoubled(), 2, "getDoubled() is 2 after decrement");
		handle.destroy();
	}

	suite("Counter — DOM structure: buttons are second and third children");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		handle.increment();
		handle.increment();
		handle.increment();
		const divEl = dom.root.childNodes[0] as Element;
		const secondChild = divEl.childNodes[1] as Element;
		assert(
			secondChild.nodeName.toLowerCase(),
			"button",
			"second child is a button",
		);
		assert(
			secondChild.textContent,
			"Up high!",
			'second child (button) contains "Up high!"',
		);
		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: Destroy safety
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — destroy does not crash");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		handle.increment();
		handle.increment();

		// Destroy should not throw
		handle.destroy();
		pass(1);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: Phase 28 — Conditional Rendering (show/hide detail)
	// ═════════════════════════════════════════════════════════════════════

	suite("Counter — toggle handler ID is valid");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		assert(
			handle.toggleHandler >= 0,
			true,
			"toggle handler ID is non-negative",
		);
		assert(
			handle.toggleHandler !== handle.incrHandler,
			true,
			"toggle handler differs from incr",
		);
		assert(
			handle.toggleHandler !== handle.decrHandler,
			true,
			"toggle handler differs from decr",
		);
		handle.destroy();
	}

	suite("Counter — show_detail starts as false");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		assert(handle.getShowDetail(), false, "show_detail is false initially");
		assert(handle.isDetailMounted(), false, "detail is not mounted initially");
		handle.destroy();
	}

	suite("Counter — toggle detail on → detail DOM appears");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		const divEl = dom.root.childNodes[0] as Element;
		// Before toggle: 5 children (h1, btn, btn, btn, placeholder)
		assert(divEl.childNodes.length, 5, "div has 5 children before toggle");

		// Toggle detail ON
		handle.toggleDetail();
		assert(handle.getShowDetail(), true, "show_detail is true after toggle");
		assert(handle.isDetailMounted(), true, "detail is mounted after toggle");

		// The placeholder (5th child) should be replaced by the detail div
		// Detail div has 2 <p> children
		const detailDiv = divEl.childNodes[4] as Element;
		assert(detailDiv.nodeName.toLowerCase(), "div", "detail is a div element");
		assert(detailDiv.childNodes.length, 2, "detail div has 2 children");

		const p1 = detailDiv.childNodes[0] as Element;
		assert(p1.nodeName.toLowerCase(), "p", "first detail child is a <p>");
		assert(
			p1.textContent,
			"Count is even",
			"p1 says 'Count is even' (count=0)",
		);

		const p2 = detailDiv.childNodes[1] as Element;
		assert(p2.nodeName.toLowerCase(), "p", "second detail child is a <p>");
		assert(p2.textContent, "Doubled: 0", "p2 says 'Doubled: 0'");

		handle.destroy();
	}

	suite("Counter — toggle detail off → detail DOM removed");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Toggle ON then OFF
		handle.toggleDetail();
		assert(handle.isDetailMounted(), true, "detail mounted after toggle on");

		handle.toggleDetail();
		assert(
			handle.getShowDetail(),
			false,
			"show_detail is false after second toggle",
		);
		assert(
			handle.isDetailMounted(),
			false,
			"detail not mounted after toggle off",
		);

		const divEl = dom.root.childNodes[0] as Element;
		// Should be back to 5 children (placeholder restored)
		assert(divEl.childNodes.length, 5, "div has 5 children after toggle off");

		// The 5th child should be a comment/placeholder, not a div
		const lastChild = divEl.childNodes[4];
		const isNotDiv = lastChild.nodeName.toLowerCase() !== "div";
		assert(isNotDiv, true, "5th child is not a div (placeholder restored)");

		handle.destroy();
	}

	suite("Counter — toggle on → off → on restores correct content");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Increment first so count=2
		handle.increment();
		handle.increment();

		// Toggle ON
		handle.toggleDetail();
		const divEl = dom.root.childNodes[0] as Element;
		let detailDiv = divEl.childNodes[4] as Element;
		let p1 = detailDiv.childNodes[0] as Element;
		let p2 = detailDiv.childNodes[1] as Element;
		assert(p1.textContent, "Count is even", "count=2 is even");
		assert(p2.textContent, "Doubled: 4", "doubled of 2 is 4");

		// Toggle OFF
		handle.toggleDetail();
		assert(handle.isDetailMounted(), false, "detail removed");

		// Toggle ON again
		handle.toggleDetail();
		assert(handle.isDetailMounted(), true, "detail re-mounted");

		detailDiv = divEl.childNodes[4] as Element;
		p1 = detailDiv.childNodes[0] as Element;
		p2 = detailDiv.childNodes[1] as Element;
		assert(p1.textContent, "Count is even", "count still even after re-toggle");
		assert(p2.textContent, "Doubled: 4", "doubled still 4 after re-toggle");

		handle.destroy();
	}

	suite("Counter — detail updates when count changes while visible");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Toggle ON (count=0)
		handle.toggleDetail();

		const divEl = dom.root.childNodes[0] as Element;
		let detailDiv = divEl.childNodes[4] as Element;
		assert(
			(detailDiv.childNodes[0] as Element).textContent,
			"Count is even",
			"count=0 → even",
		);

		// Increment to 1 (odd)
		handle.increment();
		detailDiv = divEl.childNodes[4] as Element;
		assert(
			(detailDiv.childNodes[0] as Element).textContent,
			"Count is odd",
			"count=1 → odd",
		);
		assert(
			(detailDiv.childNodes[1] as Element).textContent,
			"Doubled: 2",
			"count=1 → doubled=2",
		);

		// Increment to 2 (even again)
		handle.increment();
		detailDiv = divEl.childNodes[4] as Element;
		assert(
			(detailDiv.childNodes[0] as Element).textContent,
			"Count is even",
			"count=2 → even",
		);
		assert(
			(detailDiv.childNodes[1] as Element).textContent,
			"Doubled: 4",
			"count=2 → doubled=4",
		);

		handle.destroy();
	}

	suite(
		"Counter — detail hidden + count changes → detail shows updated content",
	);
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Increment to 3 while detail is hidden
		handle.increment();
		handle.increment();
		handle.increment();
		assert(handle.getCount(), 3, "count is 3");
		assert(handle.isDetailMounted(), false, "detail not mounted");

		// Now toggle detail ON — should show count=3 content
		handle.toggleDetail();
		const divEl = dom.root.childNodes[0] as Element;
		const detailDiv = divEl.childNodes[4] as Element;
		assert(
			(detailDiv.childNodes[0] as Element).textContent,
			"Count is odd",
			"count=3 → odd on first show",
		);
		assert(
			(detailDiv.childNodes[1] as Element).textContent,
			"Doubled: 6",
			"count=3 → doubled=6 on first show",
		);

		handle.destroy();
	}

	suite("Counter — detail is preserved across multiple increment clicks");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Toggle detail ON
		handle.toggleDetail();

		// Rapidly increment 5 times
		for (let i = 0; i < 5; i++) {
			handle.increment();
		}

		const divEl = dom.root.childNodes[0] as Element;
		const detailDiv = divEl.childNodes[4] as Element;
		assert(detailDiv.nodeName.toLowerCase(), "div", "detail div still exists");
		assert(
			(detailDiv.childNodes[0] as Element).textContent,
			"Count is odd",
			"count=5 → odd after 5 increments",
		);
		assert(
			(detailDiv.childNodes[1] as Element).textContent,
			"Doubled: 10",
			"count=5 → doubled=10",
		);

		handle.destroy();
	}

	suite("Counter — h1 and buttons unaffected by detail toggle");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		handle.increment();
		handle.increment();

		// Toggle detail ON
		handle.toggleDetail();

		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl.childNodes[0] as Element;
		assert(
			h1El.textContent,
			"High-Five counter: 2",
			"h1 text correct with detail on",
		);

		const btn1 = divEl.childNodes[1] as Element;
		assert(btn1.textContent, "Up high!", "button 1 text preserved");

		const btn2 = divEl.childNodes[2] as Element;
		assert(btn2.textContent, "Down low!", "button 2 text preserved");

		const btn3 = divEl.childNodes[3] as Element;
		assert(btn3.textContent, "Toggle detail", "button 3 text preserved");

		// Increment while detail is on
		handle.increment();
		assert(
			(divEl.childNodes[0] as Element).textContent,
			"High-Five counter: 3",
			"h1 text updates with detail on",
		);

		// Toggle OFF and verify h1 still correct
		handle.toggleDetail();
		assert(
			(divEl.childNodes[0] as Element).textContent,
			"High-Five counter: 3",
			"h1 text correct with detail off",
		);

		handle.destroy();
	}

	suite("Counter — decrement with detail visible shows negative doubled");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		handle.toggleDetail();
		handle.decrement(); // count = -1

		const divEl = dom.root.childNodes[0] as Element;
		const detailDiv = divEl.childNodes[4] as Element;
		assert(
			(detailDiv.childNodes[0] as Element).textContent,
			"Count is odd",
			"count=-1 → odd",
		);
		assert(
			(detailDiv.childNodes[1] as Element).textContent,
			"Doubled: -2",
			"count=-1 → doubled=-2",
		);

		handle.destroy();
	}
}
