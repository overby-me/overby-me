// Phase 34.1 — EffectDemoApp Tests
//
// Tests the EffectDemoApp (ed_*) WASM exports which exercise effects
// in the flush cycle with derived state (doubled, parity):
//
// Validates:
//   - ed_init state validation (count=0, doubled=0, parity="even")
//   - ed_rebuild produces mutations (templates, text nodes)
//   - DOM structure initial (h1 + button + 3 paragraphs)
//   - DOM text initial ("Count: 0", "Doubled: 0", "Parity: even")
//   - increment and flush ("Count: 1", "Doubled: 2", "Parity: odd")
//   - two increments ("Count: 2", "Doubled: 4", "Parity: even")
//   - 10 increments (all correct)
//   - effect pending after increment (before flush)
//   - effect cleared after flush
//   - flush returns 0 when clean
//   - derived state always consistent (doubled = count * 2)
//   - parity alternates (odd/even sequence for 5 increments)
//   - destroy does not crash
//   - double destroy safe
//   - multiple independent instances
//   - rapid 20 increments
//   - heapStats bounded across increments
//   - DOM updates minimal (only changed text nodes get SetText)
//   - rebuild + immediate flush (effect runs on first flush)
//   - increment without flush (state stale until flushed)

import { parseHTML } from "npm:linkedom";
import {
	createEffectDemoApp,
	type EffectDemoAppHandle,
} from "../runtime/app.ts";
import { heapStats } from "../runtime/memory.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, CallableFunction>;

// ── DOM helper ──────────────────────────────────────────────────────────────

function createDOM() {
	const { document } = parseHTML(
		'<!DOCTYPE html><html><body><div id="root"></div></body></html>',
	);
	const root = document.getElementById("root")!;
	return { document, root };
}

// ── Helper: create a mounted EffectDemoApp ──────────────────────────────────

function createED(fns: Fns): EffectDemoAppHandle {
	const { document, root } = createDOM();
	return createEffectDemoApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: EffectDemoAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ══════════════════════════════════════════════════════════════════════════════

export function testEffectDemo(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: ed_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — ed_init state validation");
	{
		const h = createED(fns);

		assert(h.getCount(), 0, "count = 0 initially");
		assert(h.getDoubled(), 0, "doubled = 0 initially");
		assert(h.getParity(), "even", 'parity = "even" initially');
		assert(h.isEffectPending(), false, "effect not pending after mount");
		assert(h.hasDirty(), false, "no dirty scopes after mount");
		assert(h.scopeCount(), 1, "scope count = 1 (single root scope)");
		assert(h.incrHandler >= 0, true, "incr handler valid");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: ed_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — ed_rebuild produces mutations");
	{
		const h = createED(fns);

		const text = rootText(h);
		assert(text.includes("Effect Demo"), true, "h1 text present");
		assert(text.includes("+ 1"), true, "button text present");
		assert(text.includes("Count: 0"), true, "count text present");
		assert(text.includes("Doubled: 0"), true, "doubled text present");
		assert(text.includes("Parity: even"), true, "parity text present");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — DOM structure initial");
	{
		const h = createED(fns);

		// The root should have a single div child containing h1, button, 3 p
		const rootEl = h.root;
		const div = rootEl.firstElementChild;
		assert(div !== null, true, "root has a child div");
		assert((div as Element).tagName.toLowerCase(), "div", "child is a div");

		const children = (div as Element).children;
		assert(children.length, 5, "div has 5 children (h1 + button + 3 p)");
		assert(children[0].tagName.toLowerCase(), "h1", "first child is h1");
		assert(
			children[1].tagName.toLowerCase(),
			"button",
			"second child is button",
		);
		assert(children[2].tagName.toLowerCase(), "p", "third child is p");
		assert(children[3].tagName.toLowerCase(), "p", "fourth child is p");
		assert(children[4].tagName.toLowerCase(), "p", "fifth child is p");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: DOM text initial
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — DOM text initial");
	{
		const h = createED(fns);

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Count: 0", 'p[0] text = "Count: 0"');
		assert(children[3].textContent, "Doubled: 0", 'p[1] text = "Doubled: 0"');
		assert(
			children[4].textContent,
			"Parity: even",
			'p[2] text = "Parity: even"',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: increment and flush
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — increment and flush");
	{
		const h = createED(fns);

		h.increment();

		assert(h.getCount(), 1, "count = 1 after increment");
		assert(h.getDoubled(), 2, "doubled = 2 after increment");
		assert(h.getParity(), "odd", 'parity = "odd" after increment');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Count: 1", "DOM count updated");
		assert(children[3].textContent, "Doubled: 2", "DOM doubled updated");
		assert(children[4].textContent, "Parity: odd", "DOM parity updated");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: two increments
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — two increments");
	{
		const h = createED(fns);

		h.increment();
		h.increment();

		assert(h.getCount(), 2, "count = 2");
		assert(h.getDoubled(), 4, "doubled = 4");
		assert(h.getParity(), "even", 'parity = "even"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Count: 2", "DOM count = 2");
		assert(children[3].textContent, "Doubled: 4", "DOM doubled = 4");
		assert(children[4].textContent, "Parity: even", "DOM parity = even");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: 10 increments
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — 10 increments");
	{
		const h = createED(fns);

		for (let i = 0; i < 10; i++) {
			h.increment();
		}

		assert(h.getCount(), 10, "count = 10");
		assert(h.getDoubled(), 20, "doubled = 20");
		assert(h.getParity(), "even", 'parity = "even"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Count: 10", "DOM count = 10");
		assert(children[3].textContent, "Doubled: 20", "DOM doubled = 20");
		assert(children[4].textContent, "Parity: even", "DOM parity = even");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: effect pending after increment
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — effect pending after increment");
	{
		const h = createED(fns);

		// Dispatch event without flushing — call WASM export directly
		fns.ed_handle_event(h.appPtr, h.incrHandler, 0);

		assert(
			h.isEffectPending(),
			true,
			"effect is pending after increment (before flush)",
		);

		// Clean up by flushing
		h.flushAndApply();

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: effect cleared after flush
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — effect cleared after flush");
	{
		const h = createED(fns);

		h.increment();

		assert(
			h.isEffectPending(),
			false,
			"effect not pending after increment + flush",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — flush returns 0 when clean");
	{
		const h = createED(fns);

		const len = h.fns.ed_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush returns 0 when no state changed");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: derived state always consistent
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — derived state always consistent");
	{
		const h = createED(fns);

		for (let i = 1; i <= 5; i++) {
			h.increment();
			const count = h.getCount();
			const doubled = h.getDoubled();
			assert(doubled, count * 2, `doubled = count*2 at step ${i}`);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: parity alternates
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — parity alternates");
	{
		const h = createED(fns);

		const expected = ["odd", "even", "odd", "even", "odd"];
		for (let i = 0; i < 5; i++) {
			h.increment();
			const parity = h.getParity();
			assert(parity, expected[i], `parity at step ${i + 1} = "${expected[i]}"`);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — destroy does not crash");
	{
		const h = createED(fns);
		h.increment();
		h.increment();
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — double destroy safe");
	{
		const h = createED(fns);
		h.destroy();
		h.destroy(); // should not throw
		assert(h.destroyed, true, "still destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — multiple independent instances");
	{
		const h1 = createED(fns);
		const h2 = createED(fns);

		h1.increment();
		h1.increment();
		h1.increment();

		h2.increment();

		assert(h1.getCount(), 3, "instance 1 count = 3");
		assert(h1.getDoubled(), 6, "instance 1 doubled = 6");
		assert(h2.getCount(), 1, "instance 2 count = 1");
		assert(h2.getDoubled(), 2, "instance 2 doubled = 2");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: rapid 20 increments
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — rapid 20 increments");
	{
		const h = createED(fns);

		for (let i = 0; i < 20; i++) {
			h.increment();
		}

		assert(h.getCount(), 20, "count = 20 after 20 increments");
		assert(h.getDoubled(), 40, "doubled = 40");
		assert(h.getParity(), "even", 'parity = "even"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Count: 20", "DOM count = 20");
		assert(children[3].textContent, "Doubled: 40", "DOM doubled = 40");
		assert(children[4].textContent, "Parity: even", "DOM parity = even");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: heapStats bounded across increments
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — heapStats bounded across increments");
	{
		const h = createED(fns);

		// Warm up
		for (let i = 0; i < 5; i++) h.increment();
		const before = heapStats();

		for (let i = 0; i < 50; i++) h.increment();
		const after = heapStats();

		// Heap should not grow unboundedly
		const growth = Number(after.heapPointer - before.heapPointer);
		assert(growth < 524288, true, `heap growth bounded (${growth} bytes)`);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: DOM updates minimal
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — DOM updates minimal");
	{
		const h = createED(fns);

		// After increment, only the three p text nodes should change
		// The h1 and button should remain unchanged
		h.increment();

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(
			children[0].textContent,
			"Effect Demo",
			"h1 unchanged after increment",
		);
		assert(children[1].textContent, "+ 1", "button unchanged after increment");
		assert(children[2].textContent, "Count: 1", "count text updated");
		assert(children[3].textContent, "Doubled: 2", "doubled text updated");
		assert(children[4].textContent, "Parity: odd", "parity text updated");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: rebuild + immediate flush
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — rebuild + immediate flush");
	{
		const h = createED(fns);

		// Effect should have already run during rebuild
		assert(h.getDoubled(), 0, "doubled = 0 after mount");
		assert(h.getParity(), "even", 'parity = "even" after mount');
		assert(h.isEffectPending(), false, "effect not pending after mount");

		// Flush should produce no mutations (everything settled)
		const len = h.fns.ed_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush returns 0 immediately after mount");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: increment without flush
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectDemo — increment without flush");
	{
		const h = createED(fns);

		// Dispatch increment but do NOT flush — call WASM export directly
		fns.ed_handle_event(h.appPtr, h.incrHandler, 0);

		// WASM state should reflect the increment
		assert(h.getCount(), 1, "count = 1 after dispatch (no flush)");

		// But doubled/parity should still be stale (effect hasn't run)
		// because effects run during flush, not during event dispatch
		assert(h.getDoubled(), 0, "doubled still 0 before flush (effect not run)");
		assert(
			h.getParity(),
			"even",
			'parity still "even" before flush (effect not run)',
		);

		// DOM should also be stale
		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(
			children[2].textContent,
			"Count: 0",
			"DOM count still 0 before flush",
		);

		// Now flush — effect runs, DOM updates
		h.flushAndApply();

		assert(h.getDoubled(), 2, "doubled = 2 after flush");
		assert(h.getParity(), "odd", 'parity = "odd" after flush');
		assert(children[2].textContent, "Count: 1", "DOM count updated");
		assert(children[3].textContent, "Doubled: 2", "DOM doubled updated");
		assert(children[4].textContent, "Parity: odd", "DOM parity updated");

		h.destroy();
	}
}
