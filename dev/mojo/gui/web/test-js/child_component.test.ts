// Child Component Composition Tests (Phase 29)
//
// Tests the ChildCounterApp which demonstrates component composition:
//   - Parent: div > [ h1("Child Counter"), button("Up"), button("Down"), dyn_node[0] ]
//   - Child: p > dyn_text("Count: N")  — with its own scope
//
// Validates:
//   - Initial mount produces correct DOM structure
//   - Child component renders inside parent's dyn_node slot
//   - Increment/decrement updates only the child's text (minimal mutations)
//   - Child has its own scope ID distinct from parent
//   - Child template is registered separately from parent
//   - Destroy cleans up child scope and handlers
//   - Multiple independent instances work correctly
//   - Rapid increment cycles produce correct results

import { parseHTML } from "npm:linkedom";
import { createChildCounterApp } from "../runtime/app.ts";
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

export function testChildComponent(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: Low-level WASM exports
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — cc_init creates app with correct state");
	{
		const app = fns.cc_init();
		assert(app !== 0n, true, "app pointer should be non-zero");

		const parentScope = fns.cc_parent_scope_id(app);
		assert(parentScope >= 0, true, "parent scope ID should be non-negative");

		const childScope = fns.cc_child_scope_id(app);
		assert(childScope >= 0, true, "child scope ID should be non-negative");
		assert(
			childScope !== parentScope,
			true,
			"child scope ID differs from parent",
		);

		const parentTmpl = fns.cc_parent_tmpl_id(app);
		assert(parentTmpl >= 0, true, "parent template ID should be non-negative");

		const childTmpl = fns.cc_child_tmpl_id(app);
		assert(childTmpl >= 0, true, "child template ID should be non-negative");
		assert(
			childTmpl !== parentTmpl,
			true,
			"child template ID differs from parent",
		);

		const count = fns.cc_count_value(app);
		assert(count, 0, "initial count should be 0");

		const incrH = fns.cc_incr_handler(app);
		assert(incrH >= 0, true, "increment handler should be non-negative");

		const decrH = fns.cc_decr_handler(app);
		assert(decrH >= 0, true, "decrement handler should be non-negative");
		assert(decrH !== incrH, true, "decrement handler differs from increment");

		const childEvents = fns.cc_child_event_count(app);
		assert(childEvents, 0, "child has no event bindings (display only)");

		const handlerCount = fns.cc_handler_count(app);
		assert(
			handlerCount >= 2,
			true,
			"at least 2 handlers registered (incr + decr)",
		);

		fns.cc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — cc_rebuild produces mutation buffer");
	{
		const app = fns.cc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		const offset = fns.cc_rebuild(app, bufPtr, BUF_CAPACITY);
		assert(offset > 0, true, "rebuild should produce mutations");

		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			offset,
		).readAll();

		// Should contain RegisterTemplate mutations for both templates
		const regTemplates = mutations.filter((m) => m.op === Op.RegisterTemplate);
		assert(
			regTemplates.length >= 2,
			true,
			`should have at least 2 RegisterTemplate (got ${regTemplates.length})`,
		);

		// Should contain LoadTemplate mutations
		const loadTemplates = mutations.filter((m) => m.op === Op.LoadTemplate);
		assert(
			loadTemplates.length >= 1,
			true,
			`should have at least 1 LoadTemplate (got ${loadTemplates.length})`,
		);

		// Should contain AppendChildren
		const appends = mutations.filter((m) => m.op === Op.AppendChildren);
		assert(appends.length >= 1, true, "should have AppendChildren");

		// Should contain a SetText for "Count: 0" (child's dynamic text)
		const setTexts = mutations.filter((m) => m.op === Op.SetText);
		const hasCountText = setTexts.some(
			(m) => (m as { text?: string }).text === "Count: 0",
		);
		assert(hasCountText, true, 'should have SetText "Count: 0"');

		// Should contain ReplaceWith (child replaces placeholder)
		const replaceOps = mutations.filter((m) => m.op === Op.ReplaceWith);
		assert(
			replaceOps.length >= 1,
			true,
			`should have ReplaceWith for child mount (got ${replaceOps.length})`,
		);

		fns.cc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — cc_handle_event increments signal");
	{
		const app = fns.cc_init();
		const incrH = fns.cc_incr_handler(app);
		const decrH = fns.cc_decr_handler(app);

		const r1 = fns.cc_handle_event(app, incrH, EVT_CLICK);
		assert(r1, 1, "increment should return 1 (handled)");
		assert(fns.cc_count_value(app), 1, "count should be 1 after increment");

		fns.cc_handle_event(app, incrH, EVT_CLICK);
		assert(
			fns.cc_count_value(app),
			2,
			"count should be 2 after second increment",
		);

		fns.cc_handle_event(app, decrH, EVT_CLICK);
		assert(fns.cc_count_value(app), 1, "count should be 1 after decrement");

		fns.cc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — cc_flush emits SetText after increment");
	{
		const app = fns.cc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		// Mount first
		fns.cc_rebuild(app, bufPtr, BUF_CAPACITY);

		const incrH = fns.cc_incr_handler(app);
		fns.cc_handle_event(app, incrH, EVT_CLICK);

		const flushLen = fns.cc_flush(app, bufPtr, BUF_CAPACITY);
		assert(flushLen > 0, true, "flush should produce mutations");

		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			flushLen,
		).readAll();

		const setTexts = mutations.filter((m) => m.op === Op.SetText);
		const countText = setTexts.find(
			(m) => (m as { text?: string }).text === "Count: 1",
		);
		assert(countText !== undefined, true, 'should have SetText "Count: 1"');

		fns.cc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — cc_flush returns 0 when nothing dirty");
	{
		const app = fns.cc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		fns.cc_rebuild(app, bufPtr, BUF_CAPACITY);

		const len = fns.cc_flush(app, bufPtr, BUF_CAPACITY);
		assert(len, 0, "flush with no changes should return 0");

		fns.cc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — minimal mutations per click (only SetText)");
	{
		const app = fns.cc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		// Mount
		fns.cc_rebuild(app, bufPtr, BUF_CAPACITY);

		// Increment
		const incrH = fns.cc_incr_handler(app);
		fns.cc_handle_event(app, incrH, EVT_CLICK);

		const len = fns.cc_flush(app, bufPtr, BUF_CAPACITY);
		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			len,
		).readAll();

		// After increment, only the child's text changes.
		// The parent's structure (h1, buttons) is static and should NOT
		// produce any mutations.  We expect only SetText mutations.
		const nonSetText = mutations.filter(
			(m) => m.op !== Op.SetText && m.op !== Op.End,
		);
		assert(
			nonSetText.length,
			0,
			`only SetText + End expected, got extra: ${nonSetText.map((m) => Op[m.op] ?? m.op).join(", ")}`,
		);

		const setTexts = mutations.filter((m) => m.op === Op.SetText);
		assert(setTexts.length, 1, "exactly 1 SetText mutation (child text)");
		assert(
			(setTexts[0] as { text?: string }).text,
			"Count: 1",
			'SetText should be "Count: 1"',
		);

		fns.cc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: DOM integration via createChildCounterApp
	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — createChildCounterApp mounts to DOM");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		// Parent: div > [ h1, button, button, <child> ]
		// Child: p > "Count: 0"
		const divEl = dom.root.firstElementChild;
		assert(divEl !== null, true, "root should have a child div");
		assert(divEl!.tagName, "DIV", "root child should be DIV");

		const children = divEl!.children;
		// h1, button, button, p (child replaced the placeholder)
		assert(children.length, 4, "div should have 4 children (h1, btn, btn, p)");

		const h1El = children[0];
		assert(h1El.tagName, "H1", "first child should be H1");
		assert(
			h1El.textContent,
			"Child Counter",
			'h1 text should be "Child Counter"',
		);

		const btn1 = children[1];
		assert(btn1.tagName, "BUTTON", "second child should be BUTTON");
		assert(btn1.textContent, "Up", 'first button text should be "Up"');

		const btn2 = children[2];
		assert(btn2.tagName, "BUTTON", "third child should be BUTTON");
		assert(btn2.textContent, "Down", 'second button text should be "Down"');

		const pEl = children[3];
		assert(pEl.tagName, "P", "fourth child should be P (child component)");
		assert(pEl.textContent, "Count: 0", 'child p text should be "Count: 0"');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — increment updates child text in DOM");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		handle.increment();

		const divEl = dom.root.firstElementChild!;
		const pEl = divEl.children[3];
		assert(pEl.tagName, "P", "fourth child should still be P");
		assert(pEl.textContent, "Count: 1", 'child text should be "Count: 1"');
		assert(handle.getCount(), 1, "getCount() should return 1");

		handle.increment();
		assert(pEl.textContent, "Count: 2", 'child text should be "Count: 2"');
		assert(handle.getCount(), 2, "getCount() should return 2");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — decrement updates child text in DOM");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		handle.decrement();

		const divEl = dom.root.firstElementChild!;
		const pEl = divEl.children[3];
		assert(pEl.textContent, "Count: -1", 'child text should be "Count: -1"');
		assert(handle.getCount(), -1, "getCount() should return -1");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — 10 increments update DOM correctly");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		const divEl = dom.root.firstElementChild!;
		const pEl = divEl.children[3];

		for (let i = 1; i <= 10; i++) {
			handle.increment();
			assert(
				pEl.textContent,
				`Count: ${i}`,
				`after ${i} increments, text should be "Count: ${i}"`,
			);
		}
		assert(handle.getCount(), 10, "final count should be 10");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — mixed increment/decrement");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		handle.increment(); // 1
		handle.increment(); // 2
		handle.increment(); // 3
		handle.decrement(); // 2
		handle.decrement(); // 1

		const divEl = dom.root.firstElementChild!;
		const pEl = divEl.children[3];
		assert(pEl.textContent, "Count: 1", 'child text should be "Count: 1"');
		assert(handle.getCount(), 1, "getCount() should return 1");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — parent DOM structure preserved across updates");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		const divEl = dom.root.firstElementChild!;
		const h1Before = divEl.children[0];
		const btn1Before = divEl.children[1];
		const btn2Before = divEl.children[2];

		handle.increment();
		handle.increment();
		handle.increment();

		// Same parent elements should still be there
		const newDiv = dom.root.firstElementChild!;
		assert(newDiv === divEl, true, "div element should be the same object");
		assert(newDiv.children.length, 4, "still 4 children");
		assert(
			newDiv.children[0] === h1Before,
			true,
			"h1 should be the same object",
		);
		assert(
			newDiv.children[1] === btn1Before,
			true,
			"first button should be the same object",
		);
		assert(
			newDiv.children[2] === btn2Before,
			true,
			"second button should be the same object",
		);
		assert(
			newDiv.children[0].textContent,
			"Child Counter",
			"h1 text unchanged",
		);
		assert(newDiv.children[1].textContent, "Up", "button 1 text unchanged");
		assert(newDiv.children[2].textContent, "Down", "button 2 text unchanged");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — scope IDs are distinct");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		assert(
			handle.childScopeId !== handle.parentScopeId,
			true,
			"child scope should differ from parent scope",
		);
		assert(handle.parentScopeId >= 0, true, "parent scope ID is valid");
		assert(handle.childScopeId >= 0, true, "child scope ID is valid");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — template IDs are distinct");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		assert(
			handle.childTmplId !== handle.parentTmplId,
			true,
			"child template should differ from parent template",
		);
		assert(handle.parentTmplId >= 0, true, "parent template ID is valid");
		assert(handle.childTmplId >= 0, true, "child template ID is valid");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — child has no event bindings (display only)");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		assert(handle.childEventCount, 0, "child component has no event bindings");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — child has rendered after mount");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		assert(
			handle.childHasRendered(),
			true,
			"child should have rendered after mount",
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — child is mounted after mount");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		assert(handle.childIsMounted(), true, "child should be mounted in the DOM");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — handler count is at least 2");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		assert(
			handle.handlerCount() >= 2,
			true,
			`handler count should be >= 2 (got ${handle.handlerCount()})`,
		);

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — destroy does not crash");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		handle.increment();
		handle.increment();

		// Should not throw
		handle.destroy();
		pass(1);

		// Root should be empty after destroy
		assert(dom.root.children.length, 0, "root should be empty after destroy");
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — double destroy is safe");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		handle.destroy();
		handle.destroy(); // should be a no-op
		pass(1);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — destroy with dirty state does not crash");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		// Dispatch event but do NOT flush — dirty state
		fns.cc_handle_event(handle.appPtr, handle.incrHandler, EVT_CLICK);

		// Destroy with unflushed state
		handle.destroy();
		pass(1);
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — multiple independent instances");
	{
		const dom1 = createDOM();
		const dom2 = createDOM();

		const h1 = createChildCounterApp(fns, dom1.root, dom1.document);
		const h2 = createChildCounterApp(fns, dom2.root, dom2.document);

		// Increment h1 three times
		h1.increment();
		h1.increment();
		h1.increment();

		// Increment h2 once
		h2.increment();

		const p1 = dom1.root.firstElementChild!.children[3];
		const p2 = dom2.root.firstElementChild!.children[3];

		assert(p1.textContent, "Count: 3", "instance 1 should show Count: 3");
		assert(p2.textContent, "Count: 1", "instance 2 should show Count: 1");
		assert(h1.getCount(), 3, "instance 1 getCount() = 3");
		assert(h2.getCount(), 1, "instance 2 getCount() = 1");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — rapid 100 increments");
	{
		const dom = createDOM();
		const handle = createChildCounterApp(fns, dom.root, dom.document);

		for (let i = 0; i < 100; i++) {
			handle.increment();
		}

		const pEl = dom.root.firstElementChild!.children[3];
		assert(pEl.textContent, "Count: 100", "after 100 increments");
		assert(handle.getCount(), 100, "getCount() returns 100");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — destroy → recreate cycle");
	{
		const dom = createDOM();

		// First instance
		let handle = createChildCounterApp(fns, dom.root, dom.document);
		handle.increment();
		handle.increment();
		assert(handle.getCount(), 2, "first instance count should be 2");
		handle.destroy();
		assert(dom.root.children.length, 0, "root empty after first destroy");

		// Second instance on same root
		handle = createChildCounterApp(fns, dom.root, dom.document);
		assert(handle.getCount(), 0, "second instance starts at 0");

		const pEl = dom.root.firstElementChild!.children[3];
		assert(pEl.textContent, "Count: 0", "new instance shows Count: 0");

		handle.increment();
		assert(pEl.textContent, "Count: 1", "new instance increments to 1");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — 5 create/destroy cycles");
	{
		const dom = createDOM();

		for (let cycle = 0; cycle < 5; cycle++) {
			const handle = createChildCounterApp(fns, dom.root, dom.document);
			for (let i = 0; i < 3; i++) {
				handle.increment();
			}
			assert(handle.getCount(), 3, `cycle ${cycle}: count should be 3`);
			const pEl = dom.root.firstElementChild!.children[3];
			assert(
				pEl.textContent,
				"Count: 3",
				`cycle ${cycle}: child text should be "Count: 3"`,
			);
			handle.destroy();
		}
		pass(1); // survived all cycles
	}

	// ═════════════════════════════════════════════════════════════════════

	suite("ChildComponent — multiple flush cycles update text correctly");
	{
		const app = fns.cc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		const incrH = fns.cc_incr_handler(app);
		const mem = getMemory();

		// Mount
		fns.cc_rebuild(app, bufPtr, BUF_CAPACITY);

		for (let i = 1; i <= 5; i++) {
			fns.cc_handle_event(app, incrH, EVT_CLICK);
			const len = fns.cc_flush(app, bufPtr, BUF_CAPACITY);

			const mutations = new MutationReader(
				mem.buffer,
				Number(bufPtr),
				len,
			).readAll();

			const setTexts = mutations.filter((m) => m.op === Op.SetText);
			const expected = `Count: ${i}`;
			const found = setTexts.some(
				(m) => (m as { text?: string }).text === expected,
			);
			assert(found, true, `flush ${i}: should have SetText "${expected}"`);
		}

		fns.cc_destroy(app);
	}
}
