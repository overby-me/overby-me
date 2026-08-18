// Phase 34.2 — EffectMemoApp Tests
//
// Tests the EffectMemoApp (em_*) WASM exports which exercise the
// signal → memo → effect → signal reactive chain:
//
// Validates:
//   - em_init state validation (input=0, tripled=0, label="small")
//   - em_rebuild produces mutations (templates, text nodes)
//   - DOM structure initial (h1 + button + 3 paragraphs)
//   - DOM text initial ("Input: 0", "Tripled: 0", "Label: small")
//   - increment and flush ("Input: 1", "Tripled: 3", "Label: small")
//   - 4 increments crosses threshold (label changes to "big")
//   - 10 increments (all correct)
//   - memo + effect both update on same flush (consistent state)
//   - flush returns 0 when clean
//   - destroy does not crash
//   - double destroy safe
//   - multiple independent instances
//   - rapid 20 increments
//   - heapStats bounded
//   - DOM updates minimal (only changed text nodes)
//   - threshold transition exact (3→4 is small→big)
//   - derived state chain consistent (tripled always input*3)
//   - memo value matches tripled

import { parseHTML } from "npm:linkedom";
import {
	createEffectMemoApp,
	type EffectMemoAppHandle,
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

// ── Helper: create a mounted EffectMemoApp ──────────────────────────────────

function createEM(fns: Fns): EffectMemoAppHandle {
	const { document, root } = createDOM();
	return createEffectMemoApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: EffectMemoAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ══════════════════════════════════════════════════════════════════════════════

export function testEffectMemo(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: em_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — em_init state validation");
	{
		const h = createEM(fns);

		assert(h.getInput(), 0, "input = 0 initially");
		assert(h.getTripled(), 0, "tripled = 0 initially");
		assert(h.getLabel(), "small", 'label = "small" initially');
		assert(h.isEffectPending(), false, "effect not pending after mount");
		assert(h.hasDirty(), false, "no dirty scopes after mount");
		assert(h.scopeCount(), 1, "scope count = 1 (single root scope)");
		assert(h.incrHandler >= 0, true, "incr handler valid");
		assert(h.getMemoValue(), 0, "memo value = 0 initially");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: em_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — em_rebuild produces mutations");
	{
		const h = createEM(fns);

		const text = rootText(h);
		assert(text.includes("Effect + Memo"), true, "h1 text present");
		assert(text.includes("+ 1"), true, "button text present");
		assert(text.includes("Input: 0"), true, "input text present");
		assert(text.includes("Tripled: 0"), true, "tripled text present");
		assert(text.includes("Label: small"), true, "label text present");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — DOM structure initial");
	{
		const h = createEM(fns);

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

	suite("EffectMemo — DOM text initial");
	{
		const h = createEM(fns);

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 0", 'p[0] text = "Input: 0"');
		assert(children[3].textContent, "Tripled: 0", 'p[1] text = "Tripled: 0"');
		assert(
			children[4].textContent,
			"Label: small",
			'p[2] text = "Label: small"',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: increment and flush
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — increment and flush");
	{
		const h = createEM(fns);

		h.increment();

		assert(h.getInput(), 1, "input = 1 after increment");
		assert(h.getTripled(), 3, "tripled = 3 after increment");
		assert(h.getLabel(), "small", 'label = "small" (3 < 10)');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 1", "DOM input updated");
		assert(children[3].textContent, "Tripled: 3", "DOM tripled updated");
		assert(children[4].textContent, "Label: small", "DOM label unchanged");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: 4 increments crosses threshold
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — 4 increments crosses threshold");
	{
		const h = createEM(fns);

		for (let i = 0; i < 4; i++) {
			h.increment();
		}

		assert(h.getInput(), 4, "input = 4");
		assert(h.getTripled(), 12, "tripled = 12");
		assert(h.getLabel(), "big", 'label = "big" (12 >= 10)');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 4", "DOM input = 4");
		assert(children[3].textContent, "Tripled: 12", "DOM tripled = 12");
		assert(children[4].textContent, "Label: big", "DOM label = big");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: 10 increments
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — 10 increments");
	{
		const h = createEM(fns);

		for (let i = 0; i < 10; i++) {
			h.increment();
		}

		assert(h.getInput(), 10, "input = 10");
		assert(h.getTripled(), 30, "tripled = 30");
		assert(h.getLabel(), "big", 'label = "big"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 10", "DOM input = 10");
		assert(children[3].textContent, "Tripled: 30", "DOM tripled = 30");
		assert(children[4].textContent, "Label: big", "DOM label = big");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: memo + effect both update on same flush
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — memo + effect both update on same flush");
	{
		const h = createEM(fns);

		h.increment();

		// Both memo and effect ran in the same flush cycle
		assert(h.getTripled(), 3, "memo ran: tripled = 3");
		assert(h.getLabel(), "small", "effect ran: label = small");
		assert(h.isEffectPending(), false, "effect not pending after flush");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — flush returns 0 when clean");
	{
		const h = createEM(fns);

		const len = h.fns.em_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush returns 0 when no state changed");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — destroy does not crash");
	{
		const h = createEM(fns);
		h.increment();
		h.increment();
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — double destroy safe");
	{
		const h = createEM(fns);
		h.destroy();
		h.destroy(); // should not throw
		assert(h.destroyed, true, "still destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — multiple independent instances");
	{
		const h1 = createEM(fns);
		const h2 = createEM(fns);

		// h1: 4 increments → tripled=12, label="big"
		for (let i = 0; i < 4; i++) h1.increment();

		// h2: 1 increment → tripled=3, label="small"
		h2.increment();

		assert(h1.getInput(), 4, "instance 1 input = 4");
		assert(h1.getTripled(), 12, "instance 1 tripled = 12");
		assert(h1.getLabel(), "big", 'instance 1 label = "big"');
		assert(h2.getInput(), 1, "instance 2 input = 1");
		assert(h2.getTripled(), 3, "instance 2 tripled = 3");
		assert(h2.getLabel(), "small", 'instance 2 label = "small"');

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: rapid 20 increments
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — rapid 20 increments");
	{
		const h = createEM(fns);

		for (let i = 0; i < 20; i++) {
			h.increment();
		}

		assert(h.getInput(), 20, "input = 20 after 20 increments");
		assert(h.getTripled(), 60, "tripled = 60");
		assert(h.getLabel(), "big", 'label = "big"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 20", "DOM input = 20");
		assert(children[3].textContent, "Tripled: 60", "DOM tripled = 60");
		assert(children[4].textContent, "Label: big", "DOM label = big");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: heapStats bounded
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — heapStats bounded");
	{
		const h = createEM(fns);

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
	// Section 15: DOM updates minimal
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — DOM updates minimal");
	{
		const h = createEM(fns);

		// After increment, only the text nodes should change
		h.increment();

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(
			children[0].textContent,
			"Effect + Memo",
			"h1 unchanged after increment",
		);
		assert(children[1].textContent, "+ 1", "button unchanged after increment");
		assert(children[2].textContent, "Input: 1", "input text updated");
		assert(children[3].textContent, "Tripled: 3", "tripled text updated");
		assert(children[4].textContent, "Label: small", "label text correct");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: threshold transition exact
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — threshold transition exact");
	{
		const h = createEM(fns);

		// input=3 → tripled=9 → "small"
		for (let i = 0; i < 3; i++) h.increment();
		assert(h.getTripled(), 9, "tripled = 9 at input 3");
		assert(h.getLabel(), "small", 'label = "small" at tripled 9');

		// input=4 → tripled=12 → "big"
		h.increment();
		assert(h.getTripled(), 12, "tripled = 12 at input 4");
		assert(h.getLabel(), "big", 'label = "big" at tripled 12');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: derived state chain consistent
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — derived state chain consistent");
	{
		const h = createEM(fns);

		for (let i = 1; i <= 8; i++) {
			h.increment();
			const input = h.getInput();
			const tripled = h.getTripled();
			const label = h.getLabel();
			assert(tripled, input * 3, `tripled = input*3 at step ${i}`);
			const expectedLabel = tripled < 10 ? "small" : "big";
			assert(
				label,
				expectedLabel,
				`label correct for tripled=${tripled} at step ${i}`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: memo value matches tripled
	// ═════════════════════════════════════════════════════════════════════

	suite("EffectMemo — memo value matches tripled");
	{
		const h = createEM(fns);

		h.increment();
		h.increment();
		h.increment();

		assert(h.getMemoValue(), h.getTripled(), "getMemoValue() === getTripled()");
		assert(h.getMemoValue(), 9, "memo value = 9");

		h.increment();
		assert(h.getMemoValue(), 12, "memo value = 12 after 4th increment");
		assert(h.getMemoValue(), h.getTripled(), "still equal after threshold");

		h.destroy();
	}
}
