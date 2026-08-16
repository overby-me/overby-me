// Phase 32.2 — SafeCounterApp Tests
//
// Tests the SafeCounterApp (sc_*) WASM exports which exercise the
// error boundary pattern with crash/retry lifecycle:
//
// Validates:
//   - sc_init state validation (scope IDs, handler IDs, initial values)
//   - sc_rebuild produces mutations (RegisterTemplate, mount, child create)
//   - increment updates count, child re-renders
//   - crash sets error state, flush swaps to fallback
//   - retry clears error, flush restores normal child
//   - count preserved across crash/recovery
//   - DOM structure in normal, error, and recovered states
//   - multiple crash/retry cycles
//   - destroy does not crash (normal, error state, double)
//   - multiple independent instances
//   - rapid increments bounded memory

import { parseHTML } from "npm:linkedom";
import {
	createSafeCounterApp,
	type SafeCounterAppHandle,
} from "../runtime/app.ts";
import { heapStats } from "../runtime/memory.ts";
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

// ── Helper: create a mounted SafeCounterApp ─────────────────────────────────

function createSC(fns: Fns): SafeCounterAppHandle {
	const { document, root } = createDOM();
	return createSafeCounterApp(fns, root, document);
}

// ══════════════════════════════════════════════════════════════════════════════

export function testSafeCounter(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: sc_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — sc_init state validation");
	{
		const h = createSC(fns);

		assert(h.getCount(), 0, "initial count = 0");
		assert(h.hasError(), false, "no error initially");
		assert(h.isNormalMounted(), true, "normal child mounted after init");
		assert(h.isFallbackMounted(), false, "fallback not mounted initially");

		const ps = h.parentScopeId;
		const ns = h.normalScopeId;
		const fs = h.fallbackScopeId;
		assert(ps >= 0, true, "parent scope ID non-negative");
		assert(ns >= 0, true, "normal scope ID non-negative");
		assert(fs >= 0, true, "fallback scope ID non-negative");
		assert(ps !== ns, true, "parent != normal scope");
		assert(ps !== fs, true, "parent != fallback scope");
		assert(ns !== fs, true, "normal != fallback scope");

		assert(h.scopeCount(), 3, "scope count = 3");
		assert(h.handlerCount() >= 3, true, "at least 3 handlers");

		assert(h.incrHandler >= 0, true, "incr handler valid");
		assert(h.crashHandler >= 0, true, "crash handler valid");
		assert(h.retryHandler >= 0, true, "retry handler valid");
		assert(h.incrHandler !== h.crashHandler, true, "incr != crash handler");
		assert(h.incrHandler !== h.retryHandler, true, "incr != retry handler");
		assert(h.crashHandler !== h.retryHandler, true, "crash != retry handler");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: sc_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — rebuild produces mutations");
	{
		const h = createSC(fns);

		// After rebuild the root should have content
		const rootEl = h.root;
		assert(rootEl.childNodes.length > 0, true, "root has children");

		// Should have an h1 with "Safe Counter"
		const h1 = rootEl.querySelector("h1");
		assert(h1 !== null, true, "h1 present");
		assert(
			h1!.textContent!.includes("Safe Counter"),
			true,
			'h1 contains "Safe Counter"',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: increment updates count
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — increment updates count");
	{
		const h = createSC(fns);

		h.increment();
		assert(h.getCount(), 1, "count = 1 after increment");

		h.increment();
		h.increment();
		assert(h.getCount(), 3, "count = 3 after 3 increments");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: flush after increment produces SetText
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — flush after increment");
	{
		const h = createSC(fns);

		h.increment();

		// The normal child should show "Count: 1"
		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let found = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Count: 1")) {
				found = true;
				break;
			}
		}
		assert(found, true, 'p with "Count: 1" found after increment');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: crash sets error state
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — crash sets error state");
	{
		const h = createSC(fns);

		h.crash();
		assert(h.hasError(), true, "has_error true after crash");
		assert(
			h.getErrorMessage().includes("Simulated crash"),
			true,
			'error message contains "Simulated crash"',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: flush after crash swaps to fallback DOM
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — flush after crash swaps to fallback");
	{
		const h = createSC(fns);

		h.crash();
		assert(h.isNormalMounted(), false, "normal unmounted after crash");
		assert(h.isFallbackMounted(), true, "fallback mounted after crash");

		// The fallback should show the error message
		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundError = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Error:")) {
				foundError = true;
				break;
			}
		}
		assert(foundError, true, 'p with "Error:" found in fallback DOM');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: normal hidden after crash
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — normal hidden after crash");
	{
		const h = createSC(fns);

		h.crash();

		// The normal child p("Count: 0") should NOT be in the DOM
		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundCount = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Count:")) {
				foundCount = true;
				break;
			}
		}
		assert(foundCount, false, 'no p with "Count:" after crash');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: retry clears error
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — retry clears error");
	{
		const h = createSC(fns);

		h.crash();
		assert(h.hasError(), true, "has error after crash");

		h.retry();
		assert(h.hasError(), false, "no error after retry");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: flush after retry restores normal
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — flush after retry restores normal");
	{
		const h = createSC(fns);

		h.crash();
		h.retry();

		assert(h.isNormalMounted(), true, "normal mounted after retry");
		assert(h.isFallbackMounted(), false, "fallback unmounted after retry");

		// Normal child should show count again
		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundCount = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Count:")) {
				foundCount = true;
				break;
			}
		}
		assert(foundCount, true, 'p with "Count:" restored after retry');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: fallback hidden after retry
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — fallback hidden after retry");
	{
		const h = createSC(fns);

		h.crash();
		h.retry();

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundError = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Error:")) {
				foundError = true;
				break;
			}
		}
		assert(foundError, false, 'no "Error:" text after retry');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: count preserved across crash/retry
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — count preserved across crash/retry");
	{
		const h = createSC(fns);

		h.increment();
		h.increment();
		h.increment();
		assert(h.getCount(), 3, "count = 3 before crash");

		h.crash();
		assert(h.getCount(), 3, "count = 3 during crash");

		h.retry();
		assert(h.getCount(), 3, "count = 3 after retry");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: increment after recovery works
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — increment after recovery");
	{
		const h = createSC(fns);

		h.increment();
		h.increment();
		assert(h.getCount(), 2, "count = 2");

		h.crash();
		h.retry();

		h.increment();
		assert(h.getCount(), 3, "count = 3 after recovery + increment");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — DOM structure initial");
	{
		const h = createSC(fns);
		const rootEl = h.root;

		// h1 present
		assert(rootEl.querySelector("h1") !== null, true, "h1 present");

		// At least 2 buttons (increment + crash)
		const buttons = rootEl.querySelectorAll("button");
		assert(buttons.length >= 2, true, "at least 2 buttons");

		// Normal child: p with count text
		const pElements = rootEl.querySelectorAll("p");
		let foundCount = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Count: 0")) {
				foundCount = true;
				break;
			}
		}
		assert(foundCount, true, 'p with "Count: 0" in initial DOM');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: DOM structure error state
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — DOM structure error state");
	{
		const h = createSC(fns);

		h.crash();

		const rootEl = h.root;

		// h1 still present (parent shell is unchanged)
		assert(rootEl.querySelector("h1") !== null, true, "h1 still present");

		// Fallback: error text present
		const pElements = rootEl.querySelectorAll("p");
		let foundError = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Error: Simulated crash")) {
				foundError = true;
				break;
			}
		}
		assert(foundError, true, 'p with "Error: Simulated crash" in DOM');

		// Retry button should be visible
		const buttons = rootEl.querySelectorAll("button");
		let foundRetry = false;
		for (const b of buttons) {
			if (b.textContent && b.textContent.includes("Retry")) {
				foundRetry = true;
				break;
			}
		}
		assert(foundRetry, true, '"Retry" button present in fallback');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: DOM structure recovered
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — DOM structure recovered");
	{
		const h = createSC(fns);

		h.increment();
		h.increment();
		h.crash();
		h.retry();

		const rootEl = h.root;

		// h1 still present
		assert(rootEl.querySelector("h1") !== null, true, "h1 present");

		// Count text should show "Count: 2"
		const pElements = rootEl.querySelectorAll("p");
		let foundCount = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Count: 2")) {
				foundCount = true;
				break;
			}
		}
		assert(foundCount, true, 'p with "Count: 2" after recovery');

		// No error text
		let foundError = false;
		for (const p of pElements) {
			if (p.textContent && p.textContent.includes("Error:")) {
				foundError = true;
				break;
			}
		}
		assert(foundError, false, 'no "Error:" text after recovery');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: multiple crash/retry cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — multiple crash/retry cycles");
	{
		const h = createSC(fns);

		for (let i = 0; i < 3; i++) {
			h.crash();
			assert(h.hasError(), true, `cycle ${i}: has error after crash`);
			assert(
				h.isNormalMounted(),
				false,
				`cycle ${i}: normal unmounted after crash`,
			);
			assert(
				h.isFallbackMounted(),
				true,
				`cycle ${i}: fallback mounted after crash`,
			);

			h.retry();
			assert(h.hasError(), false, `cycle ${i}: no error after retry`);
			assert(
				h.isNormalMounted(),
				true,
				`cycle ${i}: normal mounted after retry`,
			);
			assert(
				h.isFallbackMounted(),
				false,
				`cycle ${i}: fallback unmounted after retry`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: crash without increment
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — crash without increment");
	{
		const h = createSC(fns);

		h.crash();
		assert(h.hasError(), true, "error at count=0");
		assert(h.getCount(), 0, "count still 0");

		h.retry();
		assert(h.getCount(), 0, "count still 0 after retry");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: rapid increments then crash
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — rapid increments then crash");
	{
		const h = createSC(fns);

		for (let i = 0; i < 10; i++) {
			h.increment();
		}
		assert(h.getCount(), 10, "count = 10 after 10 increments");

		h.crash();
		assert(h.hasError(), true, "has error after crash");
		assert(h.getCount(), 10, "count preserved during crash");

		h.retry();
		assert(h.getCount(), 10, "count preserved after retry");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — destroy does not crash");
	{
		const h = createSC(fns);
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — double destroy safe");
	{
		const h = createSC(fns);
		h.destroy();
		h.destroy(); // should not throw
		assert(h.destroyed, true, "still destroyed");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — multiple independent instances");
	{
		const h1 = createSC(fns);
		const h2 = createSC(fns);

		h1.increment();
		h1.increment();
		assert(h1.getCount(), 2, "instance 1: count = 2");
		assert(h2.getCount(), 0, "instance 2: count = 0");

		h2.crash();
		assert(h1.hasError(), false, "instance 1: no error");
		assert(h2.hasError(), true, "instance 2: has error");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 22: heapStats bounded across error cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("SafeCounter — heapStats bounded across error cycles");
	{
		const h = createSC(fns);
		const before = heapStats();

		for (let i = 0; i < 20; i++) {
			h.increment();
			h.crash();
			h.retry();
		}

		const after = heapStats();
		const growth = Number(after.heapPointer - before.heapPointer);
		assert(growth < 524288, true, `heap growth bounded: ${growth} < 524288`);

		h.destroy();
	}
}
