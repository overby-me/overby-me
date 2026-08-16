// Phase 35.2 — MemoFormApp Tests
//
// Tests the MemoFormApp (mf_*) WASM exports which exercise
// MemoBool + MemoString in a form-validation scenario:
//
// Validates:
//   - mf_init state validation (input="", valid=false, status="✗ Empty")
//   - mf_rebuild produces mutations (templates, text nodes)
//   - DOM structure initial (h1 + input + 2 paragraphs)
//   - DOM text initial ("Valid: false", "Status: ✗ Empty")
//   - setInput and flush ("hi" → "Valid: true", "Status: ✓ Valid: hi")
//   - clear input reverts DOM ("" → "Valid: false", "Status: ✗ Empty")
//   - multiple inputs ("a" → "ab" → "abc", all DOM texts correct)
//   - memos dirty after setInput (before flush)
//   - memos clean after flush
//   - flush returns 0 when clean
//   - derived state consistent (valid iff input non-empty)
//   - status matches validation ("✓" when valid, "✗" when invalid)
//   - destroy does not crash
//   - double destroy safe
//   - multiple independent instances
//   - rapid 20 inputs
//   - heapStats bounded across inputs
//   - DOM updates minimal (only changed text nodes get SetText)
//   - input element has value attribute (bind_value works)
//   - memo count is 2

import { parseHTML } from "npm:linkedom";
import { createMemoFormApp, type MemoFormAppHandle } from "../runtime/app.ts";
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

// ── Helper: create a mounted MemoFormApp ─────────────────────────────────────

function createMF(fns: Fns): MemoFormAppHandle {
	const { document, root } = createDOM();
	return createMemoFormApp(fns, root, document);
}

// ── Helper: get text content of root ────────────────────────────────────────

function rootText(h: MemoFormAppHandle): string {
	return (h.root as unknown as { textContent: string }).textContent ?? "";
}

// We need writeStringStruct for raw calls (set input without auto-flush).
import { writeStringStruct } from "../runtime/strings.ts";

// ── Helper: set input without auto-flush (raw WASM call) ────────────────────

function setInputNoFlush(fns: Fns, appPtr: bigint, value: string): void {
	const strPtr = writeStringStruct(value);
	fns.mf_set_input(appPtr, strPtr);
}

// ══════════════════════════════════════════════════════════════════════════════

export function testMemoForm(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: mf_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — mf_init state validation");
	{
		const h = createMF(fns);

		assert(h.getInput(), "", 'input = "" initially');
		assert(h.isValid(), false, "valid = false initially");
		assert(h.getStatus(), "✗ Empty", 'status = "✗ Empty" initially');
		assert(h.hasDirty(), false, "no dirty scopes after mount");
		assert(h.scopeCount(), 1, "scope count = 1 (single root scope)");
		assert(h.inputHandler >= 0, true, "input handler valid");
		assert(h.getMemoCount(), 2, "memo count = 2");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: mf_rebuild produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — mf_rebuild produces mutations");
	{
		const h = createMF(fns);

		const text = rootText(h);
		assert(text.includes("Form Validation"), true, "h1 text present");
		assert(text.includes("Valid: false"), true, "valid text present");
		assert(text.includes("Status: ✗ Empty"), true, "status text present");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: DOM structure initial
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — DOM structure initial");
	{
		const h = createMF(fns);

		const rootEl = h.root;
		const div = rootEl.firstElementChild;
		assert(div !== null, true, "root has a child div");
		assert((div as Element).tagName.toLowerCase(), "div", "child is a div");

		const children = (div as Element).children;
		assert(children.length, 4, "div has 4 children (h1 + input + 2 p)");
		assert(children[0].tagName.toLowerCase(), "h1", "first child is h1");
		assert(children[1].tagName.toLowerCase(), "input", "second child is input");
		assert(children[2].tagName.toLowerCase(), "p", "third child is p");
		assert(children[3].tagName.toLowerCase(), "p", "fourth child is p");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: DOM text initial
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — DOM text initial");
	{
		const h = createMF(fns);

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(
			children[2].textContent,
			"Valid: false",
			'p[0] text = "Valid: false"',
		);
		assert(
			children[3].textContent,
			"Status: ✗ Empty",
			'p[1] text = "Status: ✗ Empty"',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: setInput and flush
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — setInput and flush");
	{
		const h = createMF(fns);

		h.setInput("hi");

		assert(h.getInput(), "hi", 'input = "hi" after setInput');
		assert(h.isValid(), true, "valid = true for non-empty input");
		assert(h.getStatus(), "✓ Valid: hi", 'status = "✓ Valid: hi"');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Valid: true", "DOM valid text updated");
		assert(
			children[3].textContent,
			"Status: ✓ Valid: hi",
			"DOM status text updated",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: clear input reverts DOM
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — clear input reverts DOM");
	{
		const h = createMF(fns);

		h.setInput("hi");
		assert(h.isValid(), true, "valid after input");

		h.setInput("");
		assert(h.isValid(), false, "valid = false after clear");
		assert(h.getStatus(), "✗ Empty", 'status = "✗ Empty" after clear');

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Valid: false", "DOM valid reverted");
		assert(children[3].textContent, "Status: ✗ Empty", "DOM status reverted");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: multiple inputs
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — multiple inputs");
	{
		const h = createMF(fns);

		const inputs = ["a", "ab", "abc"];
		for (const input of inputs) {
			h.setInput(input);

			const div = h.root.firstElementChild!;
			const children = div.children;
			assert(
				children[2].textContent,
				"Valid: true",
				`DOM valid = true for "${input}"`,
			);
			assert(
				children[3].textContent,
				`Status: ✓ Valid: ${input}`,
				`DOM status correct for "${input}"`,
			);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: memos dirty after setInput (raw, no flush)
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — memos dirty after setInput");
	{
		const h = createMF(fns);

		// Set input without flush
		setInputNoFlush(fns, h.appPtr, "test");

		assert(h.isValidDirty(), true, "is_valid dirty after raw setInput");
		assert(h.isStatusDirty(), true, "status dirty after raw setInput");

		// Clean up by flushing
		h.flushAndApply();
		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: memos clean after flush
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — memos clean after flush");
	{
		const h = createMF(fns);

		h.setInput("hello");

		assert(h.isValidDirty(), false, "is_valid clean after setInput+flush");
		assert(h.isStatusDirty(), false, "status clean after setInput+flush");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — flush returns 0 when clean");
	{
		const h = createMF(fns);

		const len = fns.mf_flush(h.appPtr, h.bufPtr, h.bufCapacity) as number;
		assert(len, 0, "flush returns 0 when nothing dirty");

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: derived state consistent
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — derived state consistent");
	{
		const h = createMF(fns);

		const testCases = ["", "a", "", "hello", "x", ""];
		for (const input of testCases) {
			h.setInput(input);
			const valid = h.isValid();
			const expectedValid = input.length > 0;
			assert(valid, expectedValid, `valid=${valid} for input="${input}"`);
		}

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: status matches validation
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — status matches validation");
	{
		const h = createMF(fns);

		h.setInput("test");
		const validStatus = h.getStatus();
		assert(
			validStatus.startsWith("✓"),
			true,
			'status starts with "✓" when valid',
		);
		assert(validStatus.includes("test"), true, "status includes input text");

		h.setInput("");
		const invalidStatus = h.getStatus();
		assert(
			invalidStatus.startsWith("✗"),
			true,
			'status starts with "✗" when invalid',
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — destroy does not crash");
	{
		const h = createMF(fns);
		h.setInput("hello");
		h.destroy();
		assert(h.destroyed, true, "destroyed flag set");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — double destroy safe");
	{
		const h = createMF(fns);
		h.destroy();
		h.destroy(); // should not crash
		assert(h.destroyed, true, "still destroyed after double destroy");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — multiple independent instances");
	{
		const h1 = createMF(fns);
		const h2 = createMF(fns);

		h1.setInput("hello");
		h2.setInput("world");

		assert(h1.getInput(), "hello", 'h1 input = "hello"');
		assert(h2.getInput(), "world", 'h2 input = "world"');
		assert(h1.isValid(), true, "h1 valid");
		assert(h2.isValid(), true, "h2 valid");
		assert(h1.getStatus(), "✓ Valid: hello", "h1 status independent");
		assert(h2.getStatus(), "✓ Valid: world", "h2 status independent");

		h1.destroy();
		h2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: rapid 20 inputs
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — rapid 20 inputs");
	{
		const h = createMF(fns);

		for (let i = 1; i <= 20; i++) {
			const input = "x".repeat(i);
			h.setInput(input);

			assert(h.isValid(), true, `valid for length ${i}`);
			assert(
				h.getStatus(),
				`✓ Valid: ${input}`,
				`status correct for length ${i}`,
			);
		}

		const div = h.root.firstElementChild!;
		const children = div.children;
		assert(children[2].textContent, "Valid: true", "final DOM valid = true");
		assert(
			children[3].textContent,
			`Status: ✓ Valid: ${"x".repeat(20)}`,
			"final DOM status correct",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: heapStats bounded across inputs
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — heapStats bounded across inputs");
	{
		const h = createMF(fns);

		// Warm up
		for (let i = 0; i < 5; i++) {
			h.setInput(`warm${i}`);
		}
		const before = heapStats();

		for (let i = 0; i < 50; i++) {
			h.setInput(`test${i}`);
		}
		const after = heapStats();

		// Heap should not grow unboundedly
		const growth = Number(after.heapPointer - before.heapPointer);
		assert(growth < 524288, true, `heap growth bounded (${growth} bytes)`);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: DOM updates minimal
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — DOM updates minimal");
	{
		const h = createMF(fns);

		// Get initial text
		const div = h.root.firstElementChild!;
		const children = div.children;
		const h1Text = children[0].textContent;

		h.setInput("minimal");

		// h1 should NOT have changed
		assert(children[0].textContent, h1Text, "h1 text unchanged after input");
		// p elements should have changed
		assert(children[2].textContent, "Valid: true", "valid text changed");
		assert(
			children[3].textContent,
			"Status: ✓ Valid: minimal",
			"status text changed",
		);

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: input element has value attribute
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — input element has value attribute");
	{
		const h = createMF(fns);

		const div = h.root.firstElementChild!;
		const inputEl = div.children[1];
		assert(inputEl.tagName.toLowerCase(), "input", "element is input");

		// After setInput, the bind_value should update the value attribute
		h.setInput("bound");

		// bind_value sets the "value" attribute on the element
		const valueAttr = inputEl.getAttribute("value");
		assert(valueAttr, "bound", 'input value attribute = "bound"');

		h.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: memo count is 2
	// ═════════════════════════════════════════════════════════════════════

	suite("MemoForm — memo count is 2");
	{
		const h = createMF(fns);

		assert(h.getMemoCount(), 2, "memo count = 2 (is_valid + status)");

		h.destroy();
	}
}
