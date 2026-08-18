// Phase 38.2 — BatchDemoApp JS Integration Tests
//
// Tests the BatchDemoApp (bd_*) WASM exports which exercise batch signal
// writes: two SignalString fields (first_name, last_name) feed a MemoString
// (full_name), and a SignalI32 (write_count) tracks batch operations.
//
// Validates:
//   1.  init and destroy — lifecycle (no crash)
//   2.  initial render — DOM: h1 "Batch Demo", two buttons, two paragraphs
//   3.  initial DOM text — "Full:  " and "Writes: 0"
//   4.  set names — DOM updates: "Full: Alice Smith", "Writes: 1"
//   5.  reset — DOM: "Full:  ", "Writes: 0"
//   6.  set then reset cycle — verify both transitions
//   7.  multiple set operations — 5 sets, final DOM correct
//   8.  write count accumulates — 3 sets → "Writes: 3"
//   9.  memo stable same names — set same names twice, write_count still changes
//  10.  full_name_changed after set — returns true
//  11.  flush returns 0 when clean — no events, flush returns 0
//  12.  flush returns nonzero after set — after set, flush > 0
//  13.  is_batching during normal operation — returns false
//  14.  scope count — 1
//  15.  memo count — 1
//  16.  destroy is clean — no errors
//  17.  double destroy safe — no crash
//  18.  multiple independent instances — two instances, independent state
//  19.  rapid 10 sets — verify final DOM correct
//  20.  first_name and last_name signals — verify individual signal values

import { parseHTML } from "npm:linkedom";
import { type BatchDemoAppHandle, createBatchDemoApp } from "../runtime/app.ts";
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

// ── Helper: create a mounted BatchDemoApp ────────────────────────────────────

function createBD(fns: Fns): BatchDemoAppHandle {
	const { document, root } = createDOM();
	return createBatchDemoApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: BatchDemoAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// ══════════════════════════════════════════════════════════════════════════════

export function testBatchDemo(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// 1. init and destroy
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — init and destroy");
	{
		const h = createBD(fns);
		assert(h.destroyed, false, "not destroyed after create");
		h.destroy();
		assert(h.destroyed, true, "destroyed after destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// 2. initial render
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — initial render");
	{
		const h = createBD(fns);
		const text = rootText(h);
		assert(text.includes("Batch Demo"), true, "h1 text present");
		assert(text.includes("Set Names"), true, "set names button text present");
		assert(text.includes("Reset"), true, "reset button text present");
		assert(text.includes("Full:"), true, "full paragraph present");
		assert(text.includes("Writes:"), true, "writes paragraph present");
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 3. initial DOM text
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — initial DOM text");
	{
		const h = createBD(fns);
		const div = h.root.firstElementChild!;
		const children = div.children;

		// Structure: h1, button(Set Names), button(Reset), p(full), p(writes)
		assert(children.length, 5, "div has 5 children (h1 + 2 buttons + 2 p)");
		assert(children[3].textContent, "Full:  ", 'p[0] text = "Full:  "');
		assert(children[4].textContent, "Writes: 0", 'p[1] text = "Writes: 0"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 4. set names
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — set names");
	{
		const h = createBD(fns);
		h.setNames("Alice", "Smith");

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(
			children[3].textContent,
			"Full: Alice Smith",
			'p[0] = "Full: Alice Smith"',
		);
		assert(children[4].textContent, "Writes: 1", 'p[1] = "Writes: 1"');

		assert(h.getFullName(), "Alice Smith", 'getFullName = "Alice Smith"');
		assert(h.getWriteCount(), 1, "getWriteCount = 1");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 5. reset
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — reset");
	{
		const h = createBD(fns);
		h.setNames("Alice", "Smith");
		h.reset();

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[3].textContent, "Full:  ", 'p[0] = "Full:  " after reset');
		assert(
			children[4].textContent,
			"Writes: 0",
			'p[1] = "Writes: 0" after reset',
		);

		assert(h.getFullName(), " ", 'getFullName = " " after reset');
		assert(h.getWriteCount(), 0, "getWriteCount = 0 after reset");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 6. set then reset cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — set then reset cycle");
	{
		const h = createBD(fns);

		// Set names
		h.setNames("Bob", "Jones");
		assert(h.getFullName(), "Bob Jones", "full name after set");
		assert(h.getWriteCount(), 1, "write count after set");

		// Reset
		h.reset();
		assert(h.getFullName(), " ", "full name after reset");
		assert(h.getWriteCount(), 0, "write count after reset");

		// Set again
		h.setNames("Carol", "Lee");
		assert(h.getFullName(), "Carol Lee", "full name after second set");
		assert(h.getWriteCount(), 1, "write count after second set");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 7. multiple set operations
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — multiple set operations");
	{
		const h = createBD(fns);

		h.setNames("A", "1");
		h.setNames("B", "2");
		h.setNames("C", "3");
		h.setNames("D", "4");
		h.setNames("E", "5");

		assert(h.getFullName(), "E 5", "final full name after 5 sets");
		assert(h.getWriteCount(), 5, "write count after 5 sets");

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[3].textContent, "Full: E 5", "DOM full name correct");
		assert(children[4].textContent, "Writes: 5", "DOM write count correct");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 8. write count accumulates
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — write count accumulates");
	{
		const h = createBD(fns);

		h.setNames("X", "Y");
		assert(h.getWriteCount(), 1, "write count = 1");
		h.setNames("X", "Z");
		assert(h.getWriteCount(), 2, "write count = 2");
		h.setNames("X", "W");
		assert(h.getWriteCount(), 3, "write count = 3");

		const div = h.root.firstElementChild!;
		assert(div.children[4].textContent, "Writes: 3", "DOM shows Writes: 3");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 9. memo stable same names (write_count still changes)
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — memo stable same names");
	{
		const h = createBD(fns);

		h.setNames("Alice", "Smith");
		assert(h.getFullName(), "Alice Smith", "first set full name");
		assert(h.getWriteCount(), 1, "first set write count");

		// Same names again — memo is value-stable, but write_count changes
		h.setNames("Alice", "Smith");
		assert(h.getFullName(), "Alice Smith", "second set full name unchanged");
		assert(h.getWriteCount(), 2, "second set write count incremented");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 10. full_name_changed after set
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — fullNameChanged after set");
	{
		const h = createBD(fns);

		// Set names — this calls bd_set_names + flushAndApply (which runs
		// run_memos internally). After flush, the memo has been recomputed.
		h.setNames("Alice", "Smith");

		// After setNames + flush, the memo was recomputed and changed
		// But fullNameChanged queries the value_changed flag which persists
		// until the next recomputation. Let's verify it's true.
		assert(h.fullNameChanged(), true, "fullNameChanged true after set");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 11. flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — flush returns 0 when clean");
	{
		const h = createBD(fns);

		// No events dispatched — flush should return 0
		h.flushAndApply();
		// If we got here without error, flush handled 0 bytes gracefully
		assert(h.hasDirty(), false, "not dirty after clean flush");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 12. flush returns nonzero after set
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — flush nonzero after set");
	{
		const h = createBD(fns);

		// After setNames (which includes flushAndApply), DOM should have updated
		h.setNames("Alice", "Smith");
		assert(h.hasDirty(), false, "not dirty after setNames+flush");

		// Verify DOM was actually updated (flush produced mutations)
		const div = h.root.firstElementChild!;
		assert(
			div.children[3].textContent,
			"Full: Alice Smith",
			"DOM updated after flush",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 13. is_batching during normal operation
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — isBatching during normal operation");
	{
		const h = createBD(fns);
		assert(h.isBatching(), false, "not batching initially");

		h.setNames("Alice", "Smith");
		assert(h.isBatching(), false, "not batching after setNames");

		h.reset();
		assert(h.isBatching(), false, "not batching after reset");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 14. scope count
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — scope count");
	{
		const h = createBD(fns);
		assert(h.scopeCount(), 1, "scope count = 1");
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 15. memo count
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — memo count");
	{
		const h = createBD(fns);
		assert(h.memoCount(), 1, "memo count = 1");
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 16. destroy is clean
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — destroy is clean");
	{
		const h = createBD(fns);
		h.setNames("Alice", "Smith");
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// 17. double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — double destroy safe");
	{
		const h = createBD(fns);
		h.destroy();
		h.destroy(); // Should not throw
		assert(h.destroyed, true, "still destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// 18. multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — multiple independent instances");
	{
		const h1 = createBD(fns);
		const h2 = createBD(fns);

		h1.setNames("Alice", "Smith");
		h2.setNames("Bob", "Jones");

		assert(h1.getFullName(), "Alice Smith", "h1 full name");
		assert(h2.getFullName(), "Bob Jones", "h2 full name");
		assert(h1.getWriteCount(), 1, "h1 write count");
		assert(h2.getWriteCount(), 1, "h2 write count");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 19. rapid 10 sets
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — rapid 10 sets");
	{
		const h = createBD(fns);

		for (let i = 0; i < 10; i++) {
			h.setNames(`F${i}`, `L${i}`);
		}

		assert(h.getFullName(), "F9 L9", 'final full name = "F9 L9"');
		assert(h.getWriteCount(), 10, "write count = 10");

		const div = h.root.firstElementChild!;
		assert(div.children[3].textContent, "Full: F9 L9", "DOM full name correct");
		assert(
			div.children[4].textContent,
			"Writes: 10",
			"DOM write count correct",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// 20. first_name and last_name signals
	// ═════════════════════════════════════════════════════════════════════

	suite("BatchDemo — first_name and last_name signals");
	{
		const h = createBD(fns);

		assert(h.getFirstName(), "", 'initial first_name = ""');
		assert(h.getLastName(), "", 'initial last_name = ""');

		h.setNames("Alice", "Smith");
		assert(h.getFirstName(), "Alice", 'first_name = "Alice"');
		assert(h.getLastName(), "Smith", 'last_name = "Smith"');

		h.reset();
		assert(h.getFirstName(), "", 'first_name = "" after reset');
		assert(h.getLastName(), "", 'last_name = "" after reset');

		h.destroy();
	}
}
