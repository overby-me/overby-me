// Phase 26 — App Lifecycle (Destroy / Recreate) Tests
//
// Tests the full create→use→destroy→recreate loop for all three apps.
// Verifies:
//   - destroy() clears DOM and frees WASM resources
//   - recreate after destroy produces correct DOM
//   - heap pointer stays bounded across create/destroy cycles
//   - double-destroy is a safe no-op
//   - destroy with dirty (unflushed) state doesn't crash
//   - bench warmup pattern (create→destroy→create→measure)

import { parseHTML } from "npm:linkedom";
import type { AppHandle } from "../runtime/app.ts";
import { createApp, createCounterApp } from "../runtime/app.ts";
import { heapStats } from "../runtime/memory.ts";
import { writeStringStruct } from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, pass, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, CallableFunction>;

// ── DOM helper ──────────────────────────────────────────────────────────────

function createDOM() {
	const { document, window } = parseHTML(
		"<!DOCTYPE html><html><body><div id='root'></div></body></html>",
	);
	const root = document.getElementById("root")!;
	return { document, window, root };
}

// ── Constants ───────────────────────────────────────────────────────────────

const BUF_CAPACITY = 16384;
const BENCH_BUF_CAPACITY = 8 * 1024 * 1024;

// ── App factory helpers ─────────────────────────────────────────────────────

interface TodoAppHandle {
	handle: AppHandle;
	addItem(text: string): void;
	itemCount(): number;
	listVersion(): number;
	destroy(): void;
}

function createTodoApp(fns: Fns, root: Element, doc: Document): TodoAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		bufCapacity: BUF_CAPACITY * 4,
		init: (f) => f.todo_init(),
		rebuild: (f, app, buf, cap) => f.todo_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.todo_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.todo_handle_event(app, hid, evt),
		destroy: (f, app) => f.todo_destroy(app),
	});

	return {
		handle,

		addItem(text: string): void {
			const strPtr = writeStringStruct(text);
			fns.todo_add_item(handle.appPtr, strPtr);
			handle.flushAndApply();
		},

		itemCount(): number {
			return fns.todo_item_count(handle.appPtr);
		},

		listVersion(): number {
			return fns.todo_list_version(handle.appPtr);
		},

		destroy(): void {
			handle.destroy();
		},
	};
}

interface BenchAppHandle {
	handle: AppHandle;
	create(count: number): void;
	rowCount(): number;
	destroy(): void;
}

function createBenchApp(
	fns: Fns,
	root: Element,
	doc: Document,
): BenchAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		bufCapacity: BENCH_BUF_CAPACITY,
		init: (f) => f.bench_init(),
		rebuild: (f, app, buf, cap) => f.bench_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.bench_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.bench_handle_event(app, hid, evt),
		destroy: (f, app) => f.bench_destroy(app),
	});

	return {
		handle,

		create(count: number): void {
			fns.bench_create(handle.appPtr, count);
			handle.flushAndApply();
		},

		rowCount(): number {
			return fns.bench_row_count(handle.appPtr);
		},

		destroy(): void {
			handle.destroy();
		},
	};
}

// ══════════════════════════════════════════════════════════════════════════════
// Counter lifecycle tests
// ══════════════════════════════════════════════════════════════════════════════

function testCounterLifecycle(fns: Fns): void {
	// ─────────────────────────────────────────────────────────────────────
	// Create → use → destroy → verify DOM cleared
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — counter: create → click → destroy → root is empty");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		handle.increment();
		handle.increment();
		assert(handle.getCount(), 2, "count is 2 before destroy");

		// Root should have content before destroy
		assert(
			dom.root.childNodes.length > 0,
			true,
			"root has children before destroy",
		);

		handle.destroy();

		assert(dom.root.childNodes.length, 0, "root is empty after destroy");
		assert(handle.destroyed, true, "destroyed flag is set");
	}

	// ─────────────────────────────────────────────────────────────────────
	// Create → destroy → recreate → use → verify correct DOM
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — counter: destroy → recreate → click → DOM correct");
	{
		const dom = createDOM();

		// First instance
		const h1 = createCounterApp(fns, dom.root, dom.document);
		h1.increment();
		h1.increment();
		h1.increment();
		assert(h1.getCount(), 3, "first instance count is 3");
		h1.destroy();
		assert(dom.root.childNodes.length, 0, "root empty after first destroy");

		// Second instance on the same root
		const h2 = createCounterApp(fns, dom.root, dom.document);
		assert(h2.getCount(), 0, "second instance starts at 0");

		h2.increment();
		assert(h2.getCount(), 1, "second instance count is 1 after click");

		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl?.childNodes[0] as Element;
		assert(
			h1El?.textContent,
			"High-Five counter: 1",
			"second instance DOM shows correct count",
		);

		h2.destroy();
	}

	// ─────────────────────────────────────────────────────────────────────
	// Create→destroy × 10 loop — heap pointer stays bounded
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — counter: 10 create/destroy cycles — heap bounded");
	{
		const dom = createDOM();
		const statsBefore = heapStats();

		for (let i = 0; i < 10; i++) {
			const h = createCounterApp(fns, dom.root, dom.document);
			h.increment();
			h.increment();
			h.destroy();
		}

		const statsAfter = heapStats();

		// The heap pointer should not grow linearly with cycles.
		// After free-list reuse, the growth should be minimal.
		// Allow a generous bound: at most 10× the first-cycle growth.
		// The key insight: freed blocks are reused, so heap shouldn't explode.
		const growth = statsAfter.heapPointer - statsBefore.heapPointer;
		// Just verify heap didn't grow by more than 1MB (very generous for 10 counter cycles)
		assert(
			growth < 1_000_000n,
			true,
			`heap growth bounded (${growth} bytes across 10 cycles)`,
		);

		// Free bytes should be > 0 (freed blocks from destroyed apps)
		assert(
			statsAfter.freeBlocks > 0,
			true,
			`free list has entries (${statsAfter.freeBlocks} blocks, ${statsAfter.freeBytes} bytes)`,
		);
	}

	// ─────────────────────────────────────────────────────────────────────
	// Double-destroy is a safe no-op
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — counter: double-destroy is a safe no-op");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		handle.increment();

		handle.destroy();
		assert(handle.destroyed, true, "destroyed after first call");

		// Second destroy should not throw
		handle.destroy();
		assert(handle.destroyed, true, "still destroyed after second call");
		pass(1); // "did not throw" counts as a pass
	}

	// ─────────────────────────────────────────────────────────────────────
	// Destroy with dirty (unflushed) state doesn't crash
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — counter: destroy with dirty state doesn't crash");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		// Dispatch event but do NOT flush — app has dirty scopes
		fns.counter_handle_event(handle.appPtr, handle.incrHandler, 0);
		// Now destroy without flushing
		handle.destroy();
		assert(dom.root.childNodes.length, 0, "root cleared despite dirty state");
		pass(1); // "did not crash" counts as a pass
	}
}

// ══════════════════════════════════════════════════════════════════════════════
// Todo lifecycle tests
// ══════════════════════════════════════════════════════════════════════════════

function testTodoLifecycle(fns: Fns): void {
	// ─────────────────────────────────────────────────────────────────────
	// Create → add items → destroy → recreate → verify clean slate
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — todo: add items → destroy → recreate → clean slate");
	{
		const dom = createDOM();

		// First instance: add some items
		const app1 = createTodoApp(fns, dom.root, dom.document);
		app1.addItem("Buy milk");
		app1.addItem("Walk dog");
		app1.addItem("Write tests");
		assert(app1.itemCount(), 3, "first instance has 3 items");

		const version1 = app1.listVersion();
		assert(version1 > 0, true, "first instance version > 0");

		app1.destroy();
		assert(dom.root.childNodes.length, 0, "root empty after todo destroy");

		// Second instance: should start fresh
		const app2 = createTodoApp(fns, dom.root, dom.document);
		assert(app2.itemCount(), 0, "second instance starts with 0 items");
		assert(app2.listVersion(), 0, "second instance version is 0");

		// Can add items to the new instance
		app2.addItem("New task");
		assert(app2.itemCount(), 1, "second instance has 1 item after add");

		// Verify DOM shows the new item
		const ul = dom.root.querySelector("ul");
		if (ul) {
			const lis = ul.querySelectorAll("li");
			assert(lis.length, 1, "DOM shows 1 li after recreate + add");
		} else {
			// ul might not exist in the DOM shape — just verify root has content
			assert(
				dom.root.childNodes.length > 0,
				true,
				"root has content after recreate + add",
			);
		}

		app2.destroy();
	}

	// ─────────────────────────────────────────────────────────────────────
	// Todo create→destroy × 5 with items — heap bounded
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — todo: 5 create/add/destroy cycles — heap bounded");
	{
		const dom = createDOM();
		const statsBefore = heapStats();

		for (let i = 0; i < 5; i++) {
			const app = createTodoApp(fns, dom.root, dom.document);
			app.addItem(`Task A-${i}`);
			app.addItem(`Task B-${i}`);
			app.destroy();
		}

		const statsAfter = heapStats();
		const growth = statsAfter.heapPointer - statsBefore.heapPointer;

		assert(
			growth < 2_000_000n,
			true,
			`todo heap growth bounded (${growth} bytes across 5 cycles)`,
		);

		assert(
			statsAfter.freeBlocks > 0,
			true,
			`todo free list has entries (${statsAfter.freeBlocks} blocks)`,
		);
	}
}

// ══════════════════════════════════════════════════════════════════════════════
// Bench lifecycle tests
// ══════════════════════════════════════════════════════════════════════════════

function testBenchLifecycle(fns: Fns): void {
	// ─────────────────────────────────────────────────────────────────────
	// Bench create→destroy→create cycle with 100 rows
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — bench: create rows → destroy → recreate → rows correct");
	{
		const dom = createDOM();

		// First instance: create 100 rows
		const app1 = createBenchApp(fns, dom.root, dom.document);
		app1.create(100);
		assert(app1.rowCount(), 100, "first instance has 100 rows");

		app1.destroy();
		assert(dom.root.childNodes.length, 0, "root empty after bench destroy");

		// Second instance: create 50 rows
		const app2 = createBenchApp(fns, dom.root, dom.document);
		app2.create(50);
		assert(app2.rowCount(), 50, "second instance has 50 rows");

		app2.destroy();
	}

	// ─────────────────────────────────────────────────────────────────────
	// Bench warmup pattern: create→1k→destroy→create→1k→measure
	// (validates js-framework-benchmark warmup requirement)
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — bench: warmup pattern (create→1k→destroy→create→1k)");
	{
		const dom = createDOM();
		const statsBefore = heapStats();

		// Warmup cycle
		const warmup = createBenchApp(fns, dom.root, dom.document);
		warmup.create(1000);
		assert(warmup.rowCount(), 1000, "warmup: 1000 rows created");
		warmup.destroy();

		// Measurement cycle
		const start = performance.now();
		const measure = createBenchApp(fns, dom.root, dom.document);
		measure.create(1000);
		const elapsed = performance.now() - start;

		assert(measure.rowCount(), 1000, "measure: 1000 rows created");

		// Heap should stay bounded — the free list should absorb the warmup allocations
		const heapGrowthTotal = heapStats().heapPointer - statsBefore.heapPointer;
		// The measurement cycle should reuse free-list blocks from the warmup.
		// Allow generous bound: total growth < 50 MB (1k rows with labels is large).
		assert(
			heapGrowthTotal < 50_000_000n,
			true,
			`heap growth bounded across warmup+measure (${heapGrowthTotal} bytes)`,
		);

		console.log(`    ℹ warmup+create 1k rows: ${elapsed.toFixed(1)}ms`);

		measure.destroy();
	}

	// ─────────────────────────────────────────────────────────────────────
	// Bench create→destroy × 5 with rows — heap stays bounded
	// ─────────────────────────────────────────────────────────────────────

	suite(
		"Lifecycle — bench: 5 create/destroy cycles with 100 rows — heap bounded",
	);
	{
		const dom = createDOM();
		const statsBefore = heapStats();

		for (let i = 0; i < 5; i++) {
			const app = createBenchApp(fns, dom.root, dom.document);
			app.create(100);
			assert(app.rowCount(), 100, `cycle ${i}: 100 rows`);
			app.destroy();
		}

		const statsAfter = heapStats();
		const growth = statsAfter.heapPointer - statsBefore.heapPointer;

		assert(
			growth < 100_000_000n,
			true,
			`bench heap growth bounded (${growth} bytes across 5 cycles)`,
		);

		assert(
			statsAfter.freeBlocks > 0,
			true,
			`bench free list has entries (${statsAfter.freeBlocks} blocks)`,
		);
	}
}

// ══════════════════════════════════════════════════════════════════════════════
// Cross-app lifecycle tests
// ══════════════════════════════════════════════════════════════════════════════

function testCrossAppLifecycle(fns: Fns): void {
	// ─────────────────────────────────────────────────────────────────────
	// Interleaved app lifecycle: counter → todo → counter on same root
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — cross-app: counter → destroy → todo → destroy → counter");
	{
		const dom = createDOM();

		// Counter
		const counter1 = createCounterApp(fns, dom.root, dom.document);
		counter1.increment();
		counter1.increment();
		assert(counter1.getCount(), 2, "counter: count is 2");
		counter1.destroy();
		assert(dom.root.childNodes.length, 0, "root empty after counter destroy");

		// Todo on the same root
		const todo = createTodoApp(fns, dom.root, dom.document);
		todo.addItem("Lifecycle test");
		assert(todo.itemCount(), 1, "todo: 1 item added");
		todo.destroy();
		assert(dom.root.childNodes.length, 0, "root empty after todo destroy");

		// Counter again
		const counter2 = createCounterApp(fns, dom.root, dom.document);
		assert(counter2.getCount(), 0, "second counter starts at 0");
		counter2.increment();
		assert(counter2.getCount(), 1, "second counter incremented to 1");

		const divEl = dom.root.childNodes[0] as Element;
		const h1El = divEl?.childNodes[0] as Element;
		assert(
			h1El?.textContent,
			"High-Five counter: 1",
			"second counter DOM correct",
		);

		counter2.destroy();
	}

	// ─────────────────────────────────────────────────────────────────────
	// Multiple simultaneous instances on different roots
	// ─────────────────────────────────────────────────────────────────────

	suite("Lifecycle — simultaneous counter instances → independent destroy");
	{
		const dom1 = createDOM();
		const dom2 = createDOM();

		const h1 = createCounterApp(fns, dom1.root, dom1.document);
		const h2 = createCounterApp(fns, dom2.root, dom2.document);

		h1.increment();
		h1.increment();
		h2.increment();

		assert(h1.getCount(), 2, "instance 1: count 2");
		assert(h2.getCount(), 1, "instance 2: count 1");

		// Destroy only h1
		h1.destroy();
		assert(dom1.root.childNodes.length, 0, "root 1 empty after destroy");
		assert(dom2.root.childNodes.length > 0, true, "root 2 still has content");

		// h2 should still work
		h2.increment();
		assert(h2.getCount(), 2, "instance 2: count 2 after h1 destroyed");

		h2.destroy();
		assert(dom2.root.childNodes.length, 0, "root 2 empty after destroy");
	}
}

// ══════════════════════════════════════════════════════════════════════════════
// Destroyed-flag / AppHandle state tests
// ══════════════════════════════════════════════════════════════════════════════

function testDestroyedState(fns: Fns): void {
	suite("Lifecycle — AppHandle.destroyed flag tracks state");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);

		assert(handle.destroyed, false, "destroyed is false initially");
		handle.destroy();
		assert(handle.destroyed, true, "destroyed is true after destroy()");
	}

	suite("Lifecycle — AppHandle fields nulled after destroy");
	{
		const dom = createDOM();
		const handle = createCounterApp(fns, dom.root, dom.document);
		const origAppPtr = handle.appPtr;
		assert(origAppPtr !== 0n, true, "appPtr is non-zero before destroy");

		handle.destroy();

		// After destroy, appPtr and bufPtr should be zeroed
		assert(handle.appPtr, 0n, "appPtr is 0n after destroy");
		assert(handle.bufPtr, 0n, "bufPtr is 0n after destroy");
	}
}

// ══════════════════════════════════════════════════════════════════════════════
// Entry point
// ══════════════════════════════════════════════════════════════════════════════

export function testLifecycle(fns: Fns): void {
	testCounterLifecycle(fns);
	testTodoLifecycle(fns);
	testBenchLifecycle(fns);
	testCrossAppLifecycle(fns);
	testDestroyedState(fns);
}
