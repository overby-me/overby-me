// Phase 33.2 — DataLoaderApp Tests
//
// Tests the DataLoaderApp (dl_*) WASM exports which exercise suspense
// with load/resolve lifecycle:
//
// Validates:
//   - dl_init state validation (scope IDs, handler IDs, initial values)
//   - dl_rebuild produces mutations (RegisterTemplate, mount, child create)
//   - DOM structure: initial, pending (skeleton), resolved (content)
//   - load sets pending, shows skeleton, hides content
//   - resolve clears pending, shows content with data, hides skeleton
//   - reload cycle (load → resolve → load → resolve)
//   - resolve with different data each time
//   - 5 load/resolve cycles
//   - flush returns 0 when clean
//   - destroy does not crash (normal, double, while pending)
//   - multiple independent instances
//   - rapid load/resolve cycles
//   - heapStats bounded across load/resolve

import { parseHTML } from "npm:linkedom";
import {
	createDataLoaderApp,
	type DataLoaderAppHandle,
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

// ── Helper: create a mounted DataLoaderApp ──────────────────────────────────

function createDL(fns: Fns): DataLoaderAppHandle {
	const { document, root } = createDOM();
	return createDataLoaderApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: DataLoaderAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ══════════════════════════════════════════════════════════════════════════════

export function testDataLoader(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: dl_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — dl_init state validation");
	{
		const h = createDL(fns);

		assert(h.isPending(), false, "not pending initially");
		assert(h.getDataText(), "(none)", 'data text initially "(none)"');
		assert(h.isContentMounted(), true, "content mounted after init");
		assert(h.isSkeletonMounted(), false, "skeleton not mounted initially");

		const ps = h.parentScopeId;
		const cs = h.contentScopeId;
		const ss = h.skeletonScopeId;
		assert(ps >= 0, true, "parent scope ID non-negative");
		assert(cs >= 0, true, "content scope ID non-negative");
		assert(ss >= 0, true, "skeleton scope ID non-negative");
		assert(ps !== cs, true, "parent != content scope");
		assert(ps !== ss, true, "parent != skeleton scope");
		assert(cs !== ss, true, "content != skeleton scope");

		assert(h.scopeCount(), 3, "scope count = 3");
		assert(h.loadHandler >= 0, true, "load handler valid");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: dl_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — dl_rebuild produces mutations");
	{
		const h = createDL(fns);

		// After rebuild, the root should contain the app's initial DOM
		const text = rootText(h);
		assert(text.includes("Data Loader"), true, "h1 text present");
		assert(text.includes("Load"), true, "button text present");
		assert(text.includes("Data: (none)"), true, "initial content text present");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — DOM structure initial");
	{
		const h = createDL(fns);

		const text = rootText(h);
		assert(text.includes("Data Loader"), true, "h1 present");
		assert(text.includes("Load"), true, "button present");
		assert(text.includes("Data: (none)"), true, "content shows (none)");
		assert(text.includes("Loading..."), false, "skeleton not visible");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: load sets pending
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — load sets pending");
	{
		const h = createDL(fns);

		h.load();

		assert(h.isPending(), true, "isPending true after load");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: flush after load shows skeleton
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — flush after load shows skeleton");
	{
		const h = createDL(fns);

		h.load();

		const text = rootText(h);
		assert(text.includes("Loading..."), true, "skeleton text visible");
		assert(h.isSkeletonMounted(), true, "skeleton mounted after load");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: content hidden after load
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — content hidden after load");
	{
		const h = createDL(fns);

		h.load();

		assert(h.isContentMounted(), false, "content not mounted after load");
		const text = rootText(h);
		assert(text.includes("Data: (none)"), false, "content text not visible");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: resolve clears pending
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — resolve clears pending");
	{
		const h = createDL(fns);

		h.load();
		h.resolve("Hello");

		assert(h.isPending(), false, "isPending false after resolve");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: flush after resolve shows content
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — flush after resolve shows content");
	{
		const h = createDL(fns);

		h.load();
		h.resolve("Hello");

		const text = rootText(h);
		assert(text.includes("Data: Hello"), true, "content shows loaded data");
		assert(h.isContentMounted(), true, "content mounted after resolve");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: skeleton hidden after resolve
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — skeleton hidden after resolve");
	{
		const h = createDL(fns);

		h.load();
		h.resolve("Hello");

		assert(h.isSkeletonMounted(), false, "skeleton not mounted after resolve");
		const text = rootText(h);
		assert(text.includes("Loading..."), false, "skeleton text not visible");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: content visible after resolve
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — content visible after resolve");
	{
		const h = createDL(fns);

		h.load();
		h.resolve("Hello");

		assert(h.isContentMounted(), true, "content mounted");
		assert(h.isSkeletonMounted(), false, "skeleton not mounted");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: DOM structure after resolve
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — DOM structure after resolve");
	{
		const h = createDL(fns);

		h.load();
		h.resolve("Hello");

		const text = rootText(h);
		assert(text.includes("Data Loader"), true, "h1 still present");
		assert(text.includes("Load"), true, "button still present");
		assert(text.includes("Data: Hello"), true, "content shows resolved data");
		assert(text.includes("Loading..."), false, "skeleton gone");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: reload cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — reload cycle");
	{
		const h = createDL(fns);

		// First load/resolve
		h.load();
		assert(h.isPending(), true, "pending after first load");
		assert(h.isSkeletonMounted(), true, "skeleton after first load");
		assert(h.isContentMounted(), false, "content hidden after first load");

		h.resolve("First");
		assert(h.isPending(), false, "not pending after first resolve");
		assert(h.isContentMounted(), true, "content after first resolve");
		assert(h.isSkeletonMounted(), false, "skeleton hidden after first resolve");
		assert(rootText(h).includes("Data: First"), true, "first data in DOM");

		// Second load/resolve
		h.load();
		assert(h.isPending(), true, "pending after second load");
		assert(h.isSkeletonMounted(), true, "skeleton after second load");

		h.resolve("Second");
		assert(h.isPending(), false, "not pending after second resolve");
		assert(h.isContentMounted(), true, "content after second resolve");
		assert(rootText(h).includes("Data: Second"), true, "second data in DOM");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: resolve with different data
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — resolve with different data");
	{
		const h = createDL(fns);

		h.load();
		h.resolve("Alpha");
		assert(rootText(h).includes("Data: Alpha"), true, "first resolve data");
		assert(h.getDataText(), "Alpha", "getDataText returns Alpha");

		h.load();
		h.resolve("Beta");
		assert(rootText(h).includes("Data: Beta"), true, "second resolve data");
		assert(h.getDataText(), "Beta", "getDataText returns Beta");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: 5 load/resolve cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — 5 load/resolve cycles");
	{
		const h = createDL(fns);

		for (let i = 0; i < 5; i++) {
			h.load();
			assert(h.isPending(), true, `pending cycle ${i}`);
			assert(h.isSkeletonMounted(), true, `skeleton mounted cycle ${i}`);
			assert(h.isContentMounted(), false, `content hidden cycle ${i}`);
			assert(
				rootText(h).includes("Loading..."),
				true,
				`skeleton text cycle ${i}`,
			);

			h.resolve(`Data-${i}`);
			assert(h.isPending(), false, `not pending after resolve cycle ${i}`);
			assert(
				h.isContentMounted(),
				true,
				`content mounted after resolve cycle ${i}`,
			);
			assert(
				h.isSkeletonMounted(),
				false,
				`skeleton hidden after resolve cycle ${i}`,
			);
			assert(
				rootText(h).includes(`Data: Data-${i}`),
				true,
				`data text correct cycle ${i}`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — flush returns 0 when clean");
	{
		const h = createDL(fns);

		// Initial state is clean after rebuild — flush should produce nothing
		h.flushAndApply();
		assert(h.hasDirty(), false, "no dirty scopes after clean flush");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — destroy does not crash");
	{
		const h = createDL(fns);
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — double destroy safe");
	{
		const h = createDL(fns);
		h.destroy();
		h.destroy(); // should not crash
		assert(h.destroyed, true, "destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: destroy while pending
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — destroy while pending");
	{
		const h = createDL(fns);
		h.load();
		assert(h.isPending(), true, "pending before destroy");
		h.destroy();
		assert(h.destroyed, true, "destroyed while pending");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — multiple independent instances");
	{
		const h1 = createDL(fns);
		const h2 = createDL(fns);

		// Load on h1, h2 should be unaffected
		h1.load();
		assert(h1.isPending(), true, "h1 pending after load");
		assert(h2.isPending(), false, "h2 not pending (independent)");
		assert(h2.isContentMounted(), true, "h2 content still mounted");

		h1.resolve("h1-data");
		assert(h1.getDataText(), "h1-data", "h1 has its own data");
		assert(h2.getDataText(), "(none)", "h2 data unchanged");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: rapid load/resolve cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — rapid load/resolve cycles");
	{
		const h = createDL(fns);

		for (let i = 0; i < 10; i++) {
			h.load();
			h.resolve(`rapid-${i}`);
		}

		assert(h.isPending(), false, "not pending after rapid cycles");
		assert(h.isContentMounted(), true, "content mounted after rapid cycles");
		assert(h.isSkeletonMounted(), false, "skeleton hidden after rapid cycles");
		assert(
			rootText(h).includes("Data: rapid-9"),
			true,
			"last data visible after rapid cycles",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: scope IDs all distinct
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — scope IDs all distinct");
	{
		const h = createDL(fns);

		const ps = h.parentScopeId;
		const cs = h.contentScopeId;
		const ss = h.skeletonScopeId;

		assert(ps !== cs, true, "parent != content");
		assert(ps !== ss, true, "parent != skeleton");
		assert(cs !== ss, true, "content != skeleton");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 22: heapStats bounded across load/resolve
	// ═════════════════════════════════════════════════════════════════════

	suite("DataLoader — heapStats bounded across load/resolve");
	{
		const h = createDL(fns);

		// Warm up
		h.load();
		h.resolve("warmup");

		const before = heapStats();

		for (let i = 0; i < 20; i++) {
			h.load();
			h.resolve(`cycle-${i}`);
		}

		const after = heapStats();

		// Memory growth should be bounded (not growing linearly with cycles)
		const growth = Number(after.heapPointer - before.heapPointer);
		// Allow generous headroom — the point is it shouldn't grow unbounded
		assert(growth < 524288, true, `heap growth bounded (${growth} bytes)`);

		h.destroy();
	}
}
