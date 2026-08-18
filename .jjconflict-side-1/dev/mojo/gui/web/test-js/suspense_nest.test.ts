// Phase 33.3 — SuspenseNestApp Tests
//
// Tests the SuspenseNestApp (sn_*) WASM exports which exercise nested
// suspense boundaries with independent load/resolve lifecycles:
//
// Validates:
//   - sn_init state validation (scope IDs, handler IDs, initial values)
//   - sn_rebuild produces mutations (RegisterTemplate, mount, child create)
//   - DOM structure: initial, inner pending, outer pending, recovered
//   - inner load → inner skeleton shown, outer content unaffected
//   - inner resolve → inner content shown with data
//   - outer load → outer skeleton shown, all inner hidden
//   - outer resolve → inner boundary visible again
//   - inner then outer load → outer skeleton shown
//   - outer resolve reveals inner skeleton (inner still pending)
//   - inner resolve after outer resolve → full recovery
//   - data text correct for inner vs outer
//   - scope IDs all distinct
//   - handler IDs all distinct
//   - flush returns 0 when clean
//   - inner/outer load flush produces minimal mutations
//   - 5 inner/outer load/resolve cycles
//   - destroy does not crash (normal, double, with pending)
//   - multiple independent instances
//   - rapid alternating loads
//   - heapStats bounded across load cycles

import { parseHTML } from "npm:linkedom";
import {
	createSuspenseNestApp,
	type SuspenseNestAppHandle,
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

// ── Helper: create a mounted SuspenseNestApp ────────────────────────────────

function createSN(fns: Fns): SuspenseNestAppHandle {
	const { document, root } = createDOM();
	return createSuspenseNestApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: SuspenseNestAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ══════════════════════════════════════════════════════════════════════════════

export function testSuspenseNest(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: sn_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — sn_init state validation");
	{
		const h = createSN(fns);

		assert(h.isOuterPending(), false, "outer not pending initially");
		assert(h.isInnerPending(), false, "inner not pending initially");
		assert(h.getOuterData(), "ready", 'outer data initially "ready"');
		assert(h.getInnerData(), "(none)", 'inner data initially "(none)"');
		assert(h.outerContentMounted(), true, "outer content mounted after init");
		assert(
			h.outerSkeletonMounted(),
			false,
			"outer skeleton not mounted initially",
		);
		assert(h.innerContentMounted(), true, "inner content mounted after init");
		assert(
			h.innerSkeletonMounted(),
			false,
			"inner skeleton not mounted initially",
		);

		const os = h.outerScopeId;
		const ibs = h.innerBoundaryScopeId;
		const ics = h.innerContentScopeId;
		const iss = h.innerSkeletonScopeId;
		const oss = h.outerSkeletonScopeId;
		assert(os >= 0, true, "outer scope ID non-negative");
		assert(ibs >= 0, true, "inner boundary scope ID non-negative");
		assert(ics >= 0, true, "inner content scope ID non-negative");
		assert(iss >= 0, true, "inner skeleton scope ID non-negative");
		assert(oss >= 0, true, "outer skeleton scope ID non-negative");

		assert(h.scopeCount(), 5, "scope count = 5");
		assert(h.outerLoadHandler >= 0, true, "outer load handler valid");
		assert(h.innerLoadHandler >= 0, true, "inner load handler valid");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: sn_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — sn_rebuild produces mutations");
	{
		const h = createSN(fns);

		const text = rootText(h);
		assert(text.includes("Nested Suspense"), true, "h1 text present");
		assert(text.includes("Outer Load"), true, "outer load button present");
		assert(text.includes("Outer: ready"), true, "outer content text present");
		assert(text.includes("Inner Load"), true, "inner load button present");
		assert(text.includes("Inner: (none)"), true, "inner content text present");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — DOM structure initial");
	{
		const h = createSN(fns);

		const text = rootText(h);
		assert(text.includes("Nested Suspense"), true, "h1 present");
		assert(text.includes("Outer Load"), true, "outer button present");
		assert(text.includes("Outer: ready"), true, "outer content shows ready");
		assert(text.includes("Inner Load"), true, "inner button present");
		assert(text.includes("Inner: (none)"), true, "inner content shows (none)");
		assert(text.includes("Outer loading..."), false, "outer skeleton hidden");
		assert(text.includes("Inner loading..."), false, "inner skeleton hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: inner load — DOM shows inner skeleton
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — inner load — DOM shows inner skeleton");
	{
		const h = createSN(fns);

		h.innerLoad();

		const text = rootText(h);
		assert(
			text.includes("Inner loading..."),
			true,
			"inner skeleton text visible",
		);
		assert(h.innerSkeletonMounted(), true, "inner skeleton mounted");
		assert(h.innerContentMounted(), false, "inner content hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: inner load — outer content unaffected
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — inner load — outer content unaffected");
	{
		const h = createSN(fns);

		h.innerLoad();

		const text = rootText(h);
		assert(text.includes("Outer: ready"), true, "outer content still visible");
		assert(h.outerContentMounted(), true, "outer content still mounted");
		assert(h.isOuterPending(), false, "outer not pending");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: inner resolve — DOM shows inner data
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — inner resolve — DOM shows inner data");
	{
		const h = createSN(fns);

		h.innerLoad();
		h.innerResolve("InnerData");

		const text = rootText(h);
		assert(
			text.includes("Inner: InnerData"),
			true,
			"inner content shows resolved data",
		);
		assert(h.innerContentMounted(), true, "inner content mounted");
		assert(h.innerSkeletonMounted(), false, "inner skeleton hidden");
		assert(h.isInnerPending(), false, "inner not pending after resolve");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: outer load — DOM shows outer skeleton
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — outer load — DOM shows outer skeleton");
	{
		const h = createSN(fns);

		h.outerLoad();

		const text = rootText(h);
		assert(
			text.includes("Outer loading..."),
			true,
			"outer skeleton text visible",
		);
		assert(h.outerSkeletonMounted(), true, "outer skeleton mounted");
		assert(h.outerContentMounted(), false, "outer content hidden");
		assert(h.innerContentMounted(), false, "inner content hidden");
		assert(h.innerSkeletonMounted(), false, "inner skeleton hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: outer resolve — DOM restored with inner
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — outer resolve — DOM restored with inner");
	{
		const h = createSN(fns);

		h.outerLoad();
		h.outerResolve("OuterData");

		const text = rootText(h);
		assert(
			text.includes("Outer: OuterData"),
			true,
			"outer content shows resolved data",
		);
		assert(h.outerContentMounted(), true, "outer content mounted");
		assert(h.outerSkeletonMounted(), false, "outer skeleton hidden");
		assert(h.innerContentMounted(), true, "inner content restored");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: inner then outer load
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — inner then outer load");
	{
		const h = createSN(fns);

		h.innerLoad();
		h.outerLoad();

		assert(h.isOuterPending(), true, "outer pending");
		assert(h.isInnerPending(), true, "inner still pending");
		assert(h.outerSkeletonMounted(), true, "outer skeleton shown");
		assert(h.outerContentMounted(), false, "outer content hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: outer resolve reveals inner skeleton
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — outer resolve reveals inner skeleton");
	{
		const h = createSN(fns);

		h.innerLoad();
		h.outerLoad();
		h.outerResolve("OuterOK");

		assert(h.isOuterPending(), false, "outer resolved");
		assert(h.outerContentMounted(), true, "outer content restored");
		assert(h.outerSkeletonMounted(), false, "outer skeleton hidden");
		assert(h.isInnerPending(), true, "inner still pending");
		assert(h.innerSkeletonMounted(), true, "inner skeleton visible");
		assert(h.innerContentMounted(), false, "inner content hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: inner resolve after outer resolve — full recovery
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — inner resolve after outer resolve — full recovery");
	{
		const h = createSN(fns);

		h.innerLoad();
		h.outerLoad();
		h.outerResolve("OuterDone");
		h.innerResolve("InnerDone");

		const text = rootText(h);
		assert(h.isOuterPending(), false, "outer not pending");
		assert(h.isInnerPending(), false, "inner not pending");
		assert(h.outerContentMounted(), true, "outer content mounted");
		assert(h.innerContentMounted(), true, "inner content mounted");
		assert(h.outerSkeletonMounted(), false, "outer skeleton hidden");
		assert(h.innerSkeletonMounted(), false, "inner skeleton hidden");
		assert(text.includes("Outer: OuterDone"), true, "outer data in DOM");
		assert(text.includes("Inner: InnerDone"), true, "inner data in DOM");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: data text correct
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — data text correct");
	{
		const h = createSN(fns);

		assert(h.getOuterData(), "ready", "outer data initially ready");
		assert(h.getInnerData(), "(none)", "inner data initially (none)");

		h.innerLoad();
		h.innerResolve("ABC");
		assert(h.getInnerData(), "ABC", "inner data after resolve");

		h.outerLoad();
		h.outerResolve("XYZ");
		assert(h.getOuterData(), "XYZ", "outer data after resolve");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: scope IDs all distinct
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — scope IDs all distinct");
	{
		const h = createSN(fns);

		const ids = [
			h.outerScopeId,
			h.innerBoundaryScopeId,
			h.innerContentScopeId,
			h.innerSkeletonScopeId,
			h.outerSkeletonScopeId,
		];
		const unique = new Set(ids);
		assert(unique.size, 5, "all 5 scope IDs distinct");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: handler IDs all distinct
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — handler IDs all distinct");
	{
		const h = createSN(fns);

		const ids = [h.outerLoadHandler, h.innerLoadHandler];
		const unique = new Set(ids);
		assert(unique.size, 2, "2 unique handler IDs");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — flush returns 0 when clean");
	{
		const h = createSN(fns);

		h.flushAndApply();
		assert(h.hasDirty(), false, "no dirty scopes after clean flush");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: inner load flush produces minimal mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — inner load flush produces minimal mutations");
	{
		const h = createSN(fns);

		h.innerLoad();

		// After inner load + flush: only inner slots should have changed
		assert(h.outerContentMounted(), true, "outer content still mounted");
		assert(h.innerSkeletonMounted(), true, "inner skeleton now mounted");
		assert(h.innerContentMounted(), false, "inner content now hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: outer load flush produces minimal mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — outer load flush produces minimal mutations");
	{
		const h = createSN(fns);

		h.outerLoad();

		// After outer load + flush: only outer slots should have changed
		assert(h.outerSkeletonMounted(), true, "outer skeleton now mounted");
		assert(h.outerContentMounted(), false, "outer content now hidden");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: 5 inner load/resolve cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — 5 inner load/resolve cycles");
	{
		const h = createSN(fns);

		for (let i = 0; i < 5; i++) {
			h.innerLoad();
			assert(h.isInnerPending(), true, `inner pending cycle ${i}`);
			assert(
				h.innerSkeletonMounted(),
				true,
				`inner skeleton mounted cycle ${i}`,
			);
			assert(h.innerContentMounted(), false, `inner content hidden cycle ${i}`);
			assert(
				rootText(h).includes("Inner loading..."),
				true,
				`inner skeleton text cycle ${i}`,
			);

			h.innerResolve(`InnerData-${i}`);
			assert(
				h.isInnerPending(),
				false,
				`inner not pending after resolve cycle ${i}`,
			);
			assert(
				h.innerContentMounted(),
				true,
				`inner content mounted after resolve cycle ${i}`,
			);
			assert(
				h.innerSkeletonMounted(),
				false,
				`inner skeleton hidden after resolve cycle ${i}`,
			);
			assert(
				rootText(h).includes(`Inner: InnerData-${i}`),
				true,
				`inner data correct cycle ${i}`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: 5 outer load/resolve cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — 5 outer load/resolve cycles");
	{
		const h = createSN(fns);

		for (let i = 0; i < 5; i++) {
			h.outerLoad();
			assert(h.isOuterPending(), true, `outer pending cycle ${i}`);
			assert(
				h.outerSkeletonMounted(),
				true,
				`outer skeleton mounted cycle ${i}`,
			);
			assert(h.outerContentMounted(), false, `outer content hidden cycle ${i}`);
			assert(
				rootText(h).includes("Outer loading..."),
				true,
				`outer skeleton text cycle ${i}`,
			);

			h.outerResolve(`OuterData-${i}`);
			assert(
				h.isOuterPending(),
				false,
				`outer not pending after resolve cycle ${i}`,
			);
			assert(
				h.outerContentMounted(),
				true,
				`outer content mounted after resolve cycle ${i}`,
			);
			assert(
				h.outerSkeletonMounted(),
				false,
				`outer skeleton hidden after resolve cycle ${i}`,
			);
			assert(
				rootText(h).includes(`Outer: OuterData-${i}`),
				true,
				`outer data correct cycle ${i}`,
			);
			assert(
				h.innerContentMounted(),
				true,
				`inner content restored cycle ${i}`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — destroy does not crash");
	{
		const h = createSN(fns);
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — double destroy safe");
	{
		const h = createSN(fns);
		h.destroy();
		h.destroy(); // should not crash
		assert(h.destroyed, true, "destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 22: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — multiple independent instances");
	{
		const h1 = createSN(fns);
		const h2 = createSN(fns);

		// Inner load on h1, h2 should be unaffected
		h1.innerLoad();
		assert(h1.isInnerPending(), true, "h1 inner pending after load");
		assert(h2.isInnerPending(), false, "h2 inner not pending (independent)");
		assert(h2.innerContentMounted(), true, "h2 inner content still mounted");

		h1.innerResolve("h1-data");
		assert(h1.getInnerData(), "h1-data", "h1 has its own inner data");
		assert(h2.getInnerData(), "(none)", "h2 inner data unchanged");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 23: rapid alternating loads
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — rapid alternating loads");
	{
		const h = createSN(fns);

		for (let i = 0; i < 10; i++) {
			if (i % 2 === 0) {
				h.innerLoad();
				h.innerResolve(`inner-${i}`);
			} else {
				h.outerLoad();
				h.outerResolve(`outer-${i}`);
			}
		}

		assert(h.isOuterPending(), false, "outer not pending after rapid");
		assert(h.isInnerPending(), false, "inner not pending after rapid");
		assert(h.outerContentMounted(), true, "outer content mounted after rapid");
		assert(h.innerContentMounted(), true, "inner content mounted after rapid");
		assert(
			h.outerSkeletonMounted(),
			false,
			"outer skeleton hidden after rapid",
		);
		assert(
			h.innerSkeletonMounted(),
			false,
			"inner skeleton hidden after rapid",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 24: heapStats bounded across load cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — heapStats bounded across load cycles");
	{
		const h = createSN(fns);

		// Warm up
		h.innerLoad();
		h.innerResolve("warmup-inner");
		h.outerLoad();
		h.outerResolve("warmup-outer");

		const before = heapStats();

		for (let i = 0; i < 20; i++) {
			h.innerLoad();
			h.innerResolve(`inner-cycle-${i}`);
			h.outerLoad();
			h.outerResolve(`outer-cycle-${i}`);
		}

		const after = heapStats();

		const growth = Number(after.heapPointer - before.heapPointer);
		assert(growth < 524288, true, `heap growth bounded (${growth} bytes)`);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 25: destroy with active pending
	// ═════════════════════════════════════════════════════════════════════

	suite("SuspenseNest — destroy with active pending");
	{
		const h = createSN(fns);

		h.innerLoad();
		h.outerLoad();
		assert(h.isOuterPending(), true, "outer pending before destroy");
		assert(h.isInnerPending(), true, "inner pending before destroy");
		h.destroy();
		assert(h.destroyed, true, "destroyed with active pending");
	}
}
