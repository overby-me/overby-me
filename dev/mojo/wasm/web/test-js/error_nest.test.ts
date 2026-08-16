// Phase 32.3 — ErrorNestApp Tests
//
// Tests the ErrorNestApp (en_*) WASM exports which exercise nested
// error boundaries with independent crash/retry lifecycles:
//
// Validates:
//   - en_init state validation (scope IDs, handler IDs, initial values)
//   - en_rebuild produces mutations (RegisterTemplate, mount, child create)
//   - DOM structure: initial, inner error, outer error, recovered
//   - inner crash → inner fallback shown, outer unaffected
//   - inner retry → inner restored
//   - outer crash → outer fallback shown, all inner hidden
//   - outer retry → inner boundary visible again
//   - inner then outer crash → outer fallback shown
//   - outer retry reveals inner fallback (inner still in error)
//   - inner retry after outer retry → full recovery
//   - error messages correct
//   - scope IDs all distinct
//   - handler IDs all distinct
//   - flush returns 0 when clean
//   - inner/outer crash flush produces minimal mutations
//   - 5 inner crash/retry cycles
//   - 5 outer crash/retry cycles
//   - destroy does not crash (normal, double, with errors)
//   - multiple independent instances
//   - rapid alternating crashes
//   - heapStats bounded across error cycles

import { parseHTML } from "npm:linkedom";
import { createErrorNestApp, type ErrorNestAppHandle } from "../runtime/app.ts";
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

// ── Helper: create a mounted ErrorNestApp ───────────────────────────────────

function createEN(fns: Fns): ErrorNestAppHandle {
	const { document, root } = createDOM();
	return createErrorNestApp(fns, root, document);
}

// ══════════════════════════════════════════════════════════════════════════════

export function testErrorNest(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: en_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — en_init state validation");
	{
		const h = createEN(fns);

		assert(h.hasOuterError(), false, "no outer error initially");
		assert(h.hasInnerError(), false, "no inner error initially");
		assert(h.outerNormalMounted(), true, "outer normal mounted after init");
		assert(
			h.outerFallbackMounted(),
			false,
			"outer fallback not mounted initially",
		);
		assert(h.innerNormalMounted(), true, "inner normal mounted after init");
		assert(
			h.innerFallbackMounted(),
			false,
			"inner fallback not mounted initially",
		);

		const os = h.outerScopeId;
		const ibs = h.innerBoundaryScopeId;
		const ins = h.innerNormalScopeId;
		const ifs = h.innerFallbackScopeId;
		const ofs = h.outerFallbackScopeId;
		assert(os >= 0, true, "outer scope ID non-negative");
		assert(ibs >= 0, true, "inner boundary scope ID non-negative");
		assert(ins >= 0, true, "inner normal scope ID non-negative");
		assert(ifs >= 0, true, "inner fallback scope ID non-negative");
		assert(ofs >= 0, true, "outer fallback scope ID non-negative");

		assert(h.scopeCount(), 5, "scope count = 5");
		assert(h.handlerCount() >= 4, true, "at least 4 handlers");

		assert(h.outerCrashHandler >= 0, true, "outer crash handler valid");
		assert(h.innerCrashHandler >= 0, true, "inner crash handler valid");
		assert(h.outerRetryHandler >= 0, true, "outer retry handler valid");
		assert(h.innerRetryHandler >= 0, true, "inner retry handler valid");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — rebuild produces mutations");
	{
		const h = createEN(fns);
		const rootEl = h.root;
		assert(rootEl.childNodes.length > 0, true, "root has children after mount");

		// Should have an h1 with "Nested Boundaries"
		const h1 = rootEl.querySelector("h1");
		assert(h1 !== null, true, "h1 element exists");
		assert(
			h1?.textContent?.includes("Nested Boundaries") ?? false,
			true,
			"h1 contains 'Nested Boundaries'",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — DOM structure initial");
	{
		const h = createEN(fns);
		const rootEl = h.root;

		// Should have Outer Crash button
		const buttons = rootEl.querySelectorAll("button");
		let foundOuterCrash = false;
		let foundInnerCrash = false;
		for (const btn of buttons) {
			if (btn.textContent?.includes("Outer Crash")) foundOuterCrash = true;
			if (btn.textContent?.includes("Inner Crash")) foundInnerCrash = true;
		}
		assert(foundOuterCrash, true, "Outer Crash button exists");
		assert(foundInnerCrash, true, "Inner Crash button exists");

		// Status OK text
		const pElements = rootEl.querySelectorAll("p");
		let foundStatus = false;
		let foundInnerWorking = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Status: OK")) foundStatus = true;
			if (p.textContent?.includes("Inner: working")) foundInnerWorking = true;
		}
		assert(foundStatus, true, "Status: OK text visible");
		assert(foundInnerWorking, true, "Inner: working text visible");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: inner crash — DOM shows inner fallback
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — inner crash — DOM shows inner fallback");
	{
		const h = createEN(fns);

		h.innerCrash();
		assert(h.hasInnerError(), true, "inner error set");
		assert(h.hasOuterError(), false, "outer error not set");

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundInnerError = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Inner error:")) foundInnerError = true;
		}
		assert(foundInnerError, true, "inner error text visible");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: inner crash — outer content unaffected
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — inner crash — outer content unaffected");
	{
		const h = createEN(fns);

		h.innerCrash();

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundStatus = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Status: OK")) foundStatus = true;
		}
		assert(foundStatus, true, "Status: OK still visible after inner crash");
		assert(h.outerNormalMounted(), true, "outer normal still mounted");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: inner retry — DOM restored
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — inner retry — DOM restored");
	{
		const h = createEN(fns);

		h.innerCrash();
		h.innerRetry();

		assert(h.hasInnerError(), false, "inner error cleared");
		assert(h.innerNormalMounted(), true, "inner normal restored");
		assert(h.innerFallbackMounted(), false, "inner fallback hidden");

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundInnerWorking = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Inner: working")) foundInnerWorking = true;
		}
		assert(foundInnerWorking, true, "Inner: working text restored");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: outer crash — DOM shows outer fallback
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — outer crash — DOM shows outer fallback");
	{
		const h = createEN(fns);

		h.outerCrash();
		assert(h.hasOuterError(), true, "outer error set");

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundOuterError = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Outer error:")) foundOuterError = true;
		}
		assert(foundOuterError, true, "outer error text visible");
		assert(h.outerFallbackMounted(), true, "outer fallback mounted");
		assert(h.outerNormalMounted(), false, "outer normal hidden");
		assert(h.innerNormalMounted(), false, "inner normal hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: outer retry — DOM restored with inner
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — outer retry — DOM restored with inner");
	{
		const h = createEN(fns);

		h.outerCrash();
		h.outerRetry();

		assert(h.hasOuterError(), false, "outer error cleared");
		assert(h.outerNormalMounted(), true, "outer normal restored");
		assert(h.outerFallbackMounted(), false, "outer fallback hidden");
		assert(h.innerNormalMounted(), true, "inner normal restored");

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundStatus = false;
		let foundInnerWorking = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Status: OK")) foundStatus = true;
			if (p.textContent?.includes("Inner: working")) foundInnerWorking = true;
		}
		assert(foundStatus, true, "Status: OK restored");
		assert(foundInnerWorking, true, "Inner: working restored");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: inner then outer crash
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — inner then outer crash");
	{
		const h = createEN(fns);

		h.innerCrash();
		h.outerCrash();

		assert(h.hasInnerError(), true, "inner error still set");
		assert(h.hasOuterError(), true, "outer error set");
		assert(h.outerFallbackMounted(), true, "outer fallback shown");
		assert(h.outerNormalMounted(), false, "outer normal hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: outer retry reveals inner fallback
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — outer retry reveals inner fallback");
	{
		const h = createEN(fns);

		h.innerCrash();
		h.outerCrash();
		h.outerRetry();

		assert(h.hasOuterError(), false, "outer error cleared");
		assert(h.hasInnerError(), true, "inner error still set");
		assert(h.outerNormalMounted(), true, "outer normal restored");
		assert(h.innerFallbackMounted(), true, "inner fallback visible");
		assert(h.innerNormalMounted(), false, "inner normal hidden");

		const rootEl = h.root;
		const pElements = rootEl.querySelectorAll("p");
		let foundInnerError = false;
		for (const p of pElements) {
			if (p.textContent?.includes("Inner error:")) foundInnerError = true;
		}
		assert(foundInnerError, true, "inner error text visible after outer retry");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: inner retry after outer retry — full recovery
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — inner retry after outer retry — full recovery");
	{
		const h = createEN(fns);

		h.innerCrash();
		h.outerCrash();
		h.outerRetry();
		h.innerRetry();

		assert(h.hasOuterError(), false, "outer error cleared");
		assert(h.hasInnerError(), false, "inner error cleared");
		assert(h.outerNormalMounted(), true, "outer normal mounted");
		assert(h.innerNormalMounted(), true, "inner normal mounted");
		assert(h.outerFallbackMounted(), false, "outer fallback hidden");
		assert(h.innerFallbackMounted(), false, "inner fallback hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: error messages correct
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — error messages correct");
	{
		const h = createEN(fns);

		h.innerCrash();
		assert(
			h.getInnerErrorMessage().includes("Inner crash"),
			true,
			"inner error message contains 'Inner crash'",
		);

		h.outerCrash();
		assert(
			h.getOuterErrorMessage().includes("Outer crash"),
			true,
			"outer error message contains 'Outer crash'",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: scope IDs all distinct
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — scope IDs all distinct");
	{
		const h = createEN(fns);

		const ids = [
			h.outerScopeId,
			h.innerBoundaryScopeId,
			h.innerNormalScopeId,
			h.innerFallbackScopeId,
			h.outerFallbackScopeId,
		];
		const unique = new Set(ids);
		assert(unique.size, 5, "5 unique scope IDs");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: handler IDs all distinct
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — handler IDs all distinct");
	{
		const h = createEN(fns);

		const ids = [
			h.outerCrashHandler,
			h.innerCrashHandler,
			h.outerRetryHandler,
			h.innerRetryHandler,
		];
		const unique = new Set(ids);
		assert(unique.size, 4, "4 unique handler IDs");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — flush returns 0 when clean");
	{
		const h = createEN(fns);

		assert(h.hasDirty(), false, "no dirty scopes after mount");
		h.flushAndApply();
		assert(h.hasDirty(), false, "flush returns 0 when no changes");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: inner crash flush produces minimal mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — inner crash flush produces minimal mutations");
	{
		const h = createEN(fns);

		h.innerCrash();
		// After inner crash + flush, only inner slot should change
		assert(h.outerNormalMounted(), true, "outer normal still mounted");
		assert(h.innerFallbackMounted(), true, "inner fallback now mounted");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: outer crash flush produces minimal mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — outer crash flush produces minimal mutations");
	{
		const h = createEN(fns);

		h.outerCrash();
		// After outer crash + flush, only outer slot changes
		assert(h.outerFallbackMounted(), true, "outer fallback now mounted");
		assert(h.outerNormalMounted(), false, "outer normal hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: 5 inner crash/retry cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — 5 inner crash/retry cycles");
	{
		const h = createEN(fns);

		for (let i = 0; i < 5; i++) {
			h.innerCrash();
			assert(h.hasInnerError(), true, `cycle ${i}: inner error set`);
			assert(
				h.innerFallbackMounted(),
				true,
				`cycle ${i}: inner fallback mounted`,
			);
			assert(h.innerNormalMounted(), false, `cycle ${i}: inner normal hidden`);
			assert(
				h.outerNormalMounted(),
				true,
				`cycle ${i}: outer normal still mounted`,
			);

			h.innerRetry();
			assert(h.hasInnerError(), false, `cycle ${i}: inner error cleared`);
			assert(h.innerNormalMounted(), true, `cycle ${i}: inner normal restored`);
			assert(
				h.innerFallbackMounted(),
				false,
				`cycle ${i}: inner fallback hidden`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: 5 outer crash/retry cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — 5 outer crash/retry cycles");
	{
		const h = createEN(fns);

		for (let i = 0; i < 5; i++) {
			h.outerCrash();
			assert(h.hasOuterError(), true, `cycle ${i}: outer error set`);
			assert(
				h.outerFallbackMounted(),
				true,
				`cycle ${i}: outer fallback mounted`,
			);
			assert(h.outerNormalMounted(), false, `cycle ${i}: outer normal hidden`);

			h.outerRetry();
			assert(h.hasOuterError(), false, `cycle ${i}: outer error cleared`);
			assert(h.outerNormalMounted(), true, `cycle ${i}: outer normal restored`);
			assert(
				h.outerFallbackMounted(),
				false,
				`cycle ${i}: outer fallback hidden`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — destroy does not crash");
	{
		const h = createEN(fns);
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — double destroy safe");
	{
		const h = createEN(fns);
		h.destroy();
		h.destroy(); // should not throw
		assert(h.destroyed, true, "still destroyed");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 22: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — multiple independent instances");
	{
		const h1 = createEN(fns);
		const h2 = createEN(fns);

		h1.innerCrash();
		assert(h1.hasInnerError(), true, "instance 1: inner error set");
		assert(h2.hasInnerError(), false, "instance 2: no inner error");

		h2.outerCrash();
		assert(h1.hasOuterError(), false, "instance 1: no outer error");
		assert(h2.hasOuterError(), true, "instance 2: outer error set");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 23: rapid alternating crashes
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — rapid alternating crashes");
	{
		const h = createEN(fns);

		for (let i = 0; i < 10; i++) {
			if (i % 2 === 0) {
				h.innerCrash();
				assert(h.hasInnerError(), true, `alt ${i}: inner error set`);
				h.innerRetry();
			} else {
				h.outerCrash();
				assert(h.hasOuterError(), true, `alt ${i}: outer error set`);
				h.outerRetry();
			}
		}

		// Fully recovered
		assert(h.hasOuterError(), false, "no outer error after alternations");
		assert(h.hasInnerError(), false, "no inner error after alternations");
		assert(h.outerNormalMounted(), true, "outer normal mounted");
		assert(h.innerNormalMounted(), true, "inner normal mounted");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 24: heapStats bounded across error cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — heapStats bounded across error cycles");
	{
		const h = createEN(fns);
		const before = heapStats();

		for (let i = 0; i < 20; i++) {
			h.innerCrash();
			h.innerRetry();
			h.outerCrash();
			h.outerRetry();
		}

		const after = heapStats();
		const growth = Number(after.heapPointer - before.heapPointer);
		assert(growth < 524288, true, `heap growth bounded: ${growth} < 524288`);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 25: destroy with active errors
	// ═════════════════════════════════════════════════════════════════════

	suite("ErrorNest — destroy with active errors");
	{
		const h = createEN(fns);

		h.innerCrash();
		h.outerCrash();
		// Destroy while both errors are active — should not crash
		h.destroy();
		assert(h.destroyed, true, "destroyed with active errors");
	}
}
