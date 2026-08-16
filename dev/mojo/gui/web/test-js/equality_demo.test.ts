// Phase 37.7 — EqualityDemoApp JS Integration Tests
//
// Tests the EqualityDemoApp (eq_*) WASM exports which exercise
// an equality-gated memo chain: SignalI32 → MemoI32(clamp) → MemoString(label).
//
// Validates:
//   - init and destroy lifecycle (no crash)
//   - initial render (correct DOM after rebuild)
//   - initial DOM text (paragraphs show correct values)
//   - increment within range (clamped changes, label stable)
//   - increment across threshold (5→6, label "low"→"high")
//   - increment at max (10→11, clamped stable)
//   - increment above max (15→16, chain fully stable)
//   - decrement within range (clamped changes, label stable)
//   - decrement across threshold (6→5, label "high"→"low")
//   - decrement at min (0→-1, clamped stable)
//   - clamped_changed after stable (returns false)
//   - label_changed after stable (returns false)
//   - clamped_changed after value change (returns true)
//   - label_changed after value change (returns true)
//   - flush returns 0 when stable
//   - flush returns nonzero when changed
//   - multiple stable flushes (5 increments above max, each 0)
//   - full cycle round-trip (0→12→0)
//   - scope count (assert 1)
//   - memo count (assert 2)
//   - dirty state after event (hasDirty true before flush)
//   - destroy is clean (no errors after destroy)

import { parseHTML } from "npm:linkedom";
import {
	createEqualityDemoApp,
	type EqualityDemoAppHandle,
} from "../runtime/app.ts";
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

// ── Helper: create a mounted EqualityDemoApp ─────────────────────────────────

function createEQ(fns: Fns): EqualityDemoAppHandle {
	const { document, root } = createDOM();
	return createEqualityDemoApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: EqualityDemoAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ── Helper: dispatch increment N times (raw, no flush) ──────────────────────

function incrementN(h: EqualityDemoAppHandle, n: number): void {
	for (let i = 0; i < n; i++) {
		h.increment();
	}
}

function decrementN(h: EqualityDemoAppHandle, n: number): void {
	for (let i = 0; i < n; i++) {
		h.decrement();
	}
}

// ══════════════════════════════════════════════════════════════════════════════

export function testEqualityDemo(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// 1. init and destroy
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — init and destroy");
	{
		const h = createEQ(fns);
		assert(h.destroyed, false, "not destroyed after create");
		h.destroy();
		assert(h.destroyed, true, "destroyed after destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// 2. initial render
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — initial render");
	{
		const h = createEQ(fns);
		const text = rootText(h);
		assert(text.includes("Equality Gate"), true, "h1 text present");
		assert(text.includes("+ 1"), true, "increment button text present");
		assert(text.includes("- 1"), true, "decrement button text present");
		assert(text.includes("Clamped:"), true, "clamped paragraph present");
		assert(text.includes("Label:"), true, "label paragraph present");
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 3. initial DOM text
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — initial DOM text");
	{
		const h = createEQ(fns);
		const div = h.root.firstElementChild!;
		const children = div.children;

		// Structure: h1, button(+1), button(-1), p(clamped), p(label)
		assert(children.length, 5, "div has 5 children (h1 + 2 buttons + 2 p)");
		assert(children[3].textContent, "Clamped: 0", 'p[0] text = "Clamped: 0"');
		assert(children[4].textContent, "Label: low", 'p[1] text = "Label: low"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 4. increment within range
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — increment within range");
	{
		const h = createEQ(fns);
		h.increment();

		assert(h.getInput(), 1, "input = 1 after increment");
		assert(h.getClamped(), 1, "clamped = 1 (within range)");
		assert(h.getLabel(), "low", 'label = "low" (1 <= 5)');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[3].textContent, "Clamped: 1", "DOM clamped updated");
		assert(children[4].textContent, "Label: low", "DOM label unchanged");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 5. increment across threshold
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — increment across threshold");
	{
		const h = createEQ(fns);
		// Increment to 6 (threshold is > 5)
		incrementN(h, 6);

		assert(h.getInput(), 6, "input = 6");
		assert(h.getClamped(), 6, "clamped = 6");
		assert(h.getLabel(), "high", 'label = "high" (6 > 5)');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[3].textContent, "Clamped: 6", "DOM clamped = 6");
		assert(children[4].textContent, "Label: high", 'DOM label = "Label: high"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 6. increment at max (clamped stable)
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — increment at max (clamped stable)");
	{
		const h = createEQ(fns);
		// Go to 10 (max)
		incrementN(h, 10);
		assert(h.getClamped(), 10, "clamped = 10 at max");

		// One more — input 11, clamped stays 10
		h.increment();
		assert(h.getInput(), 11, "input = 11");
		assert(h.getClamped(), 10, "clamped still = 10 (clamped at max)");
		assert(h.getLabel(), "high", 'label still = "high"');

		// DOM should not have changed from last real update
		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(
			children[3].textContent,
			"Clamped: 10",
			"DOM clamped stays 10 (stable)",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 7. increment above max (chain stable)
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — increment above max (chain stable)");
	{
		const h = createEQ(fns);
		// Go well above max
		incrementN(h, 16);
		assert(h.getInput(), 16, "input = 16");
		assert(h.getClamped(), 10, "clamped = 10 (clamped at max)");
		assert(h.getLabel(), "high", 'label = "high"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 8. decrement within range
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — decrement within range");
	{
		const h = createEQ(fns);
		incrementN(h, 5);
		assert(h.getClamped(), 5, "clamped = 5 before decrement");

		h.decrement();
		assert(h.getInput(), 4, "input = 4");
		assert(h.getClamped(), 4, "clamped = 4");
		assert(h.getLabel(), "low", 'label = "low" (4 <= 5)');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 9. decrement across threshold
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — decrement across threshold");
	{
		const h = createEQ(fns);
		incrementN(h, 6); // input=6, clamped=6, label="high"
		assert(h.getLabel(), "high", 'label = "high" at 6');

		h.decrement(); // input=5, clamped=5, label="low"
		assert(h.getInput(), 5, "input = 5");
		assert(h.getClamped(), 5, "clamped = 5");
		assert(h.getLabel(), "low", 'label = "low" (5 <= 5)');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[4].textContent, "Label: low", 'DOM label = "Label: low"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 10. decrement at min (clamped stable)
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — decrement at min (clamped stable)");
	{
		const h = createEQ(fns);
		// Start at 0, decrement → input = -1, clamped stays 0
		h.decrement();
		assert(h.getInput(), -1, "input = -1");
		assert(h.getClamped(), 0, "clamped = 0 (clamped at min)");
		assert(h.getLabel(), "low", 'label = "low"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 11. clamped_changed after stable
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — clamped_changed after stable");
	{
		const h = createEQ(fns);
		incrementN(h, 12); // well above max
		// Now increment one more — clamped stays 10
		h.increment();
		assert(
			h.clampedChanged(),
			false,
			"clamped_changed = false (10 → 10 stable)",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 12. label_changed after stable
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — label_changed after stable");
	{
		const h = createEQ(fns);
		incrementN(h, 12);
		h.increment(); // chain fully stable
		assert(
			h.labelChanged(),
			false,
			'label_changed = false ("high" → "high" stable)',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 13. clamped_changed after value change
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — clamped_changed after value change");
	{
		const h = createEQ(fns);
		h.increment(); // 0 → 1 — clamped changes
		assert(h.clampedChanged(), true, "clamped_changed = true (0 → 1 changed)");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 14. label_changed after value change
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — label_changed after value change");
	{
		const h = createEQ(fns);
		incrementN(h, 6); // cross threshold: label "low" → "high"
		assert(
			h.labelChanged(),
			true,
			'label_changed = true ("low" → "high" changed)',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 15. flush returns 0 when stable
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — flush returns 0 when stable");
	{
		const h = createEQ(fns);
		incrementN(h, 12); // well above max, chain stable after this
		// Dispatch one more raw event (no auto-flush)
		h.fns.eq_handle_event(h.appPtr, h.incrHandler, 1);
		// Raw flush — should return 0 because chain is stable
		const len = h.fns.eq_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush returns 0 when memo chain is stable");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 16. flush returns nonzero when changed
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — flush returns nonzero when changed");
	{
		const h = createEQ(fns);
		// Dispatch raw increment (0 → 1, clamped changes)
		h.fns.eq_handle_event(h.appPtr, h.incrHandler, 1);
		const len = h.fns.eq_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len > 0, true, "flush returns > 0 when memo chain changed");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 17. multiple stable flushes
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — multiple stable flushes");
	{
		const h = createEQ(fns);
		incrementN(h, 12); // go above max, chain stabilizes
		let allZero = true;
		for (let i = 0; i < 5; i++) {
			h.fns.eq_handle_event(h.appPtr, h.incrHandler, 1);
			const len = h.fns.eq_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
			if (len !== 0) allZero = false;
		}
		assert(allZero, true, "all 5 stable flushes returned 0");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 18. full cycle round-trip
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — full cycle round-trip");
	{
		const h = createEQ(fns);

		// Increment 0 → 12
		incrementN(h, 12);
		assert(h.getInput(), 12, "input = 12 after 12 increments");
		assert(h.getClamped(), 10, "clamped = 10 (max)");
		assert(h.getLabel(), "high", 'label = "high"');

		// Decrement 12 → 0
		decrementN(h, 12);
		assert(h.getInput(), 0, "input = 0 after 12 decrements");
		assert(h.getClamped(), 0, "clamped = 0 (back to start)");
		assert(h.getLabel(), "low", 'label = "low"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[3].textContent, "Clamped: 0", "DOM clamped = 0");
		assert(children[4].textContent, "Label: low", 'DOM label = "Label: low"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 19. scope count
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — scope count");
	{
		const h = createEQ(fns);
		assert(h.scopeCount(), 1, "scope count = 1 (single root scope)");
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 20. memo count
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — memo count");
	{
		const h = createEQ(fns);
		assert(h.memoCount(), 2, "memo count = 2 (clamped + label)");
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 21. dirty state after event
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — dirty state after event");
	{
		const h = createEQ(fns);
		// Dispatch raw event without flushing
		h.fns.eq_handle_event(h.appPtr, h.incrHandler, 1);
		assert(h.hasDirty(), true, "hasDirty = true after event before flush");
		// Flush to clean up
		h.fns.eq_flush(h.appPtr, h.bufPtr, h.bufCapacity);
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 22. destroy is clean
	// ═════════════════════════════════════════════════════════════════════

	suite("EqualityDemo — destroy is clean");
	{
		const h = createEQ(fns);
		h.destroy();
		assert(h.destroyed, true, "no errors after destroy");
		// Double destroy should be safe
		h.destroy();
		assert(h.destroyed, true, "double destroy is safe");
	}
}
