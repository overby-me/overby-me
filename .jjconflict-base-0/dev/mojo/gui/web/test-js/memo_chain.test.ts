// Phase 35.3 — MemoChainApp Tests
//
// Tests the MemoChainApp (mc_*) WASM exports which exercise
// a mixed-type memo chain: SignalI32 → MemoI32 → MemoBool → MemoString.
//
// Validates:
//   - mc_init state validation (input=0, doubled=0, is_big=false, label="small")
//   - mc_rebuild produces mutations (templates, text nodes)
//   - DOM structure initial (h1 + button + 4 paragraphs)
//   - DOM text initial (all four texts correct)
//   - increment and flush (input=1, doubled=2, is_big=false, label="small")
//   - 5 increments crosses threshold (all four texts updated)
//   - 6 increments stays big (doubled=12, is_big=true, label="BIG")
//   - 10 increments (all correct)
//   - all memos dirty after increment (dirty before flush)
//   - all memos clean after flush
//   - flush returns 0 when clean
//   - chain produces correct derived state (for each increment)
//   - threshold boundary exact (input=5 → is_big flips to true)
//   - threshold stable above (input 6,7,8 all is_big=true)
//   - destroy does not crash
//   - double destroy safe
//   - multiple independent instances
//   - rapid 20 increments
//   - heapStats bounded across increments
//   - DOM updates minimal (SetText only for changed values)
//   - memo count is 3
//   - rebuild + immediate flush (all memos settle on first flush)

import { parseHTML } from "npm:linkedom";
import { createMemoChainApp, type MemoChainAppHandle } from "../runtime/app.ts";
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

// ── Helper: create a mounted MemoChainApp ────────────────────────────────────

function createMC(fns: Fns): MemoChainAppHandle {
	const { document, root } = createDOM();
	return createMemoChainApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: MemoChainAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ══════════════════════════════════════════════════════════════════════════════

export function testMemoChain(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: mc_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — mc_init state validation");
	{
		const h = createMC(fns);

		assert(h.getInput(), 0, "input = 0 initially");
		assert(h.getDoubled(), 0, "doubled = 0 initially");
		assert(h.isBig(), false, "is_big = false initially");
		assert(h.getLabel(), "small", 'label = "small" initially');
		assert(h.hasDirty(), false, "no dirty scopes after mount");
		assert(h.scopeCount(), 1, "scope count = 1 (single root scope)");
		assert(h.incrHandler >= 0, true, "incr handler valid");
		assert(h.getMemoCount(), 3, "memo count = 3");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: mc_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — mc_rebuild produces mutations");
	{
		const h = createMC(fns);

		const text = rootText(h);
		assert(text.includes("Memo Chain"), true, "h1 text present");
		assert(text.includes("+ 1"), true, "button text present");
		assert(text.includes("Input: 0"), true, "input text present");
		assert(text.includes("Doubled: 0"), true, "doubled text present");
		assert(text.includes("Is Big: false"), true, "is_big text present");
		assert(text.includes("Label: small"), true, "label text present");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — DOM structure initial");
	{
		const h = createMC(fns);

		const rootEl = h.root;
		const div = rootEl.firstElementChild;
		assert(div !== null, true, "root has a child div");
		assert((div as Element).tagName.toLowerCase(), "div", "child is a div");

		const children = (div as Element).children;
		assert(children.length, 6, "div has 6 children (h1 + button + 4 p)");
		assert(children[0].tagName.toLowerCase(), "h1", "first child is h1");
		assert(
			children[1].tagName.toLowerCase(),
			"button",
			"second child is button",
		);
		assert(children[2].tagName.toLowerCase(), "p", "third child is p");
		assert(children[3].tagName.toLowerCase(), "p", "fourth child is p");
		assert(children[4].tagName.toLowerCase(), "p", "fifth child is p");
		assert(children[5].tagName.toLowerCase(), "p", "sixth child is p");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: DOM text initial
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — DOM text initial");
	{
		const h = createMC(fns);

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 0", 'p[0] text = "Input: 0"');
		assert(children[3].textContent, "Doubled: 0", 'p[1] text = "Doubled: 0"');
		assert(
			children[4].textContent,
			"Is Big: false",
			'p[2] text = "Is Big: false"',
		);
		assert(
			children[5].textContent,
			"Label: small",
			'p[3] text = "Label: small"',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: increment and flush
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — increment and flush");
	{
		const h = createMC(fns);

		h.increment();

		assert(h.getInput(), 1, "input = 1 after increment");
		assert(h.getDoubled(), 2, "doubled = 2 after increment");
		assert(h.isBig(), false, "is_big = false (2 < 10)");
		assert(h.getLabel(), "small", 'label = "small"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 1", "DOM input updated");
		assert(children[3].textContent, "Doubled: 2", "DOM doubled updated");
		assert(children[4].textContent, "Is Big: false", "DOM is_big unchanged");
		assert(children[5].textContent, "Label: small", "DOM label unchanged");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: 5 increments crosses threshold
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — 5 increments crosses threshold");
	{
		const h = createMC(fns);

		for (let i = 0; i < 5; i++) {
			h.increment();
		}

		assert(h.getInput(), 5, "input = 5");
		assert(h.getDoubled(), 10, "doubled = 10");
		assert(h.isBig(), true, "is_big = true (10 >= 10)");
		assert(h.getLabel(), "BIG", 'label = "BIG"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 5", "DOM input = 5");
		assert(children[3].textContent, "Doubled: 10", "DOM doubled = 10");
		assert(children[4].textContent, "Is Big: true", "DOM is_big = true");
		assert(children[5].textContent, "Label: BIG", "DOM label = BIG");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: 6 increments stays big
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — 6 increments stays big");
	{
		const h = createMC(fns);

		for (let i = 0; i < 6; i++) {
			h.increment();
		}

		assert(h.getInput(), 6, "input = 6");
		assert(h.getDoubled(), 12, "doubled = 12");
		assert(h.isBig(), true, "is_big = true (12 >= 10)");
		assert(h.getLabel(), "BIG", 'label = "BIG"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: 10 increments
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — 10 increments");
	{
		const h = createMC(fns);

		for (let i = 0; i < 10; i++) {
			h.increment();
		}

		assert(h.getInput(), 10, "input = 10");
		assert(h.getDoubled(), 20, "doubled = 20");
		assert(h.isBig(), true, "is_big = true (20 >= 10)");
		assert(h.getLabel(), "BIG", 'label = "BIG"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 10", "DOM input = 10");
		assert(children[3].textContent, "Doubled: 20", "DOM doubled = 20");
		assert(children[4].textContent, "Is Big: true", "DOM is_big = true");
		assert(children[5].textContent, "Label: BIG", "DOM label = BIG");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: all memos dirty after increment (Phase 36 propagation)
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — all memos dirty after increment");
	{
		const h = createMC(fns);

		// Dispatch increment without flush
		fns.mc_handle_event(h.appPtr, h.incrHandler, 0);

		// Phase 36: recursive memo propagation marks ALL downstream
		// memos dirty through the chain, not just the direct subscriber.
		assert(h.isDoubledDirty(), true, "doubled dirty after increment");
		assert(h.isBigDirty(), true, "is_big dirty after increment (propagated)");
		assert(h.isLabelDirty(), true, "label dirty after increment (propagated)");

		h.flushAndApply();
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: all memos clean after flush
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — all memos clean after flush");
	{
		const h = createMC(fns);

		h.increment();

		assert(h.isDoubledDirty(), false, "doubled clean after flush");
		assert(h.isBigDirty(), false, "is_big clean after flush");
		assert(h.isLabelDirty(), false, "label clean after flush");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — flush returns 0 when clean");
	{
		const h = createMC(fns);

		const len = fns.mc_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush returns 0 when nothing dirty");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: chain produces correct derived state
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — chain produces correct derived state");
	{
		const h = createMC(fns);

		for (let i = 1; i <= 8; i++) {
			h.increment();
			const input = h.getInput();
			const doubled = h.getDoubled();
			const big = h.isBig();
			const expectedBig = doubled >= 10;

			assert(input, i, `input = ${i}`);
			assert(doubled, i * 2, `doubled = ${i * 2}`);
			assert(big, expectedBig, `is_big = ${expectedBig} at doubled=${doubled}`);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: threshold boundary exact
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — threshold boundary exact");
	{
		const h = createMC(fns);

		// input=4 → doubled=8 → is_big=false
		for (let i = 0; i < 4; i++) h.increment();
		assert(h.getDoubled(), 8, "doubled = 8 at input=4");
		assert(h.isBig(), false, "is_big = false at doubled=8");
		assert(h.getLabel(), "small", 'label = "small" at doubled=8');

		// input=5 → doubled=10 → is_big=true
		h.increment();
		assert(h.getDoubled(), 10, "doubled = 10 at input=5");
		assert(h.isBig(), true, "is_big = true at doubled=10");
		assert(h.getLabel(), "BIG", 'label = "BIG" at doubled=10');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: threshold stable above
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — threshold stable above");
	{
		const h = createMC(fns);

		for (let i = 0; i < 8; i++) h.increment();

		// input 6,7,8 should all be is_big=true
		assert(h.getInput(), 8, "input = 8");
		assert(h.isBig(), true, "is_big = true at input=8");
		assert(h.getLabel(), "BIG", 'label = "BIG" at input=8');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — destroy does not crash");
	{
		const h = createMC(fns);
		for (let i = 0; i < 5; i++) h.increment();
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — double destroy safe");
	{
		const h = createMC(fns);
		h.destroy();
		h.destroy(); // should not crash
		assert(h.destroyed, true, "still destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — multiple independent instances");
	{
		const h1 = createMC(fns);
		const h2 = createMC(fns);

		for (let i = 0; i < 3; i++) h1.increment();
		for (let i = 0; i < 6; i++) h2.increment();

		assert(h1.getInput(), 3, "h1 input = 3");
		assert(h2.getInput(), 6, "h2 input = 6");
		assert(h1.isBig(), false, "h1 is_big = false (doubled=6)");
		assert(h2.isBig(), true, "h2 is_big = true (doubled=12)");
		assert(h1.getLabel(), "small", 'h1 label = "small"');
		assert(h2.getLabel(), "BIG", 'h2 label = "BIG"');

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: rapid 20 increments
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — rapid 20 increments");
	{
		const h = createMC(fns);

		for (let i = 1; i <= 20; i++) {
			h.increment();
			assert(h.getInput(), i, `input = ${i}`);
			assert(h.getDoubled(), i * 2, `doubled = ${i * 2}`);
		}

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Input: 20", "final DOM input = 20");
		assert(children[3].textContent, "Doubled: 40", "final DOM doubled = 40");
		assert(children[4].textContent, "Is Big: true", "final DOM is_big = true");
		assert(children[5].textContent, "Label: BIG", "final DOM label = BIG");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: heapStats bounded across increments
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — heapStats bounded across increments");
	{
		const h = createMC(fns);

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
	// Section 20: DOM updates minimal
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — DOM updates minimal");
	{
		const h = createMC(fns);

		// Get initial text
		const div = h.root.firstElementChild!;
		const children = div.children;
		const h1Text = children[0].textContent;
		const btnText = children[1].textContent;

		h.increment(); // input=1, doubled=2, is_big=false, label="small"

		// h1 and button should NOT have changed
		assert(
			children[0].textContent,
			h1Text,
			"h1 text unchanged after increment",
		);
		assert(
			children[1].textContent,
			btnText,
			"button text unchanged after increment",
		);
		// p elements with new values should have changed
		assert(children[2].textContent, "Input: 1", "input text changed");
		assert(children[3].textContent, "Doubled: 2", "doubled text changed");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: memo count is 3
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — memo count is 3");
	{
		const h = createMC(fns);

		assert(h.getMemoCount(), 3, "memo count = 3 (doubled + is_big + label)");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 22: rebuild + immediate flush
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — rebuild + immediate flush");
	{
		const h = createMC(fns);

		// After rebuild, all memos should be settled — flush should be a no-op
		const len = fns.mc_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush after rebuild returns 0 (all settled)");

		// Values should be correct
		assert(h.getDoubled(), 0, "doubled = 0 after rebuild");
		assert(h.isBig(), false, "is_big = false after rebuild");
		assert(h.getLabel(), "small", 'label = "small" after rebuild');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 23: all memos independently dirty after increment (Phase 36)
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — all memos independently dirty after increment (Phase 36)");
	{
		const h = createMC(fns);

		// Dispatch increment WITHOUT flushing
		fns.mc_handle_event(h.appPtr, h.incrHandler, 0);

		// Phase 36 recursive propagation: all three memos should be
		// independently dirty after a single signal write.
		assert(h.isDoubledDirty(), true, "doubled dirty (direct subscriber)");
		assert(h.isBigDirty(), true, "is_big dirty (propagated via doubled)");
		assert(h.isLabelDirty(), true, "label dirty (propagated via is_big)");

		h.flushAndApply();
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 24: partial recompute — memos settle independently (Phase 36)
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoChain — partial recompute settles independently (Phase 36)");
	{
		const h = createMC(fns);

		// Dispatch increment — all three memos dirty
		fns.mc_handle_event(h.appPtr, h.incrHandler, 0);
		assert(h.isDoubledDirty(), true, "doubled dirty before flush");
		assert(h.isBigDirty(), true, "is_big dirty before flush");
		assert(h.isLabelDirty(), true, "label dirty before flush");

		// Flush recomputes all memos — all clean afterwards
		h.flushAndApply();
		assert(h.isDoubledDirty(), false, "doubled clean after flush");
		assert(h.isBigDirty(), false, "is_big clean after flush");
		assert(h.isLabelDirty(), false, "label clean after flush");

		// Final state correct: input=1, doubled=2, is_big=false, label="small"
		assert(h.getInput(), 1, "input = 1");
		assert(h.getDoubled(), 2, "doubled = 2");
		assert(h.isBig(), false, "is_big = false (doubled < 10)");
		assert(h.getLabel(), "small", 'label = "small"');

		h.destroy();
	}
}
