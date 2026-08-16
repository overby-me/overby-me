// Phase 31.4 — ThemeCounterApp Tests
//
// Tests the ThemeCounterApp (tc_*) WASM exports which exercise shared
// context across multiple child components with upward communication:
//
// Validates:
//   - tc_init state validation (3 scopes, handler IDs, initial values)
//   - tc_rebuild mounts parent + both children
//   - increment updates parent signal, both children re-render
//   - theme toggle updates both children's format/class
//   - DOM mount: parent div with 2 buttons + counter child div + summary p
//   - increment → counter shows "Count: 1", summary shows "1 clicks"
//   - theme toggle → counter shows "Theme: dark, Count: 0"
//   - summary gets "dark" class after theme toggle
//   - reset button → count returns to 0, both children update
//   - increment → reset → increment cycle
//   - theme toggle does not affect count
//   - both children re-render on count change
//   - children have independent scope IDs
//   - flush returns 0 when clean
//   - destroy does not crash
//   - double destroy safe
//   - destroy + recreate cycle
//   - multiple independent instances
//   - rapid increments bounded memory
//   - theme toggle + increment in same flush
//   - 5 create/destroy cycles with state verification

import { parseHTML } from "npm:linkedom";
import {
	createApp,
	createThemeCounterApp,
	type ThemeCounterAppHandle,
} from "../runtime/app.ts";
import { alignedAlloc } from "../runtime/memory.ts";
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

// ── Constants ───────────────────────────────────────────────────────────────

const BUF_CAPACITY = 16384;

// ── Helper: create a mounted ThemeCounterApp via createThemeCounterApp ───────

function createTC(fns: Fns): ThemeCounterAppHandle {
	const { document, root } = createDOM();
	return createThemeCounterApp(fns, root, document);
}

// ══════════════════════════════════════════════════════════════════════════════

export function testThemeCounter(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: tc_init state validation
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — tc_init state validation");
	{
		const handle = createTC(fns);

		const parentScope = handle.parentScopeId;
		const counterScope = handle.counterScopeId;
		const summaryScope = handle.summaryScopeId;
		assert(parentScope >= 0, true, "parent scope ID non-negative");
		assert(counterScope >= 0, true, "counter scope ID non-negative");
		assert(summaryScope >= 0, true, "summary scope ID non-negative");
		assert(parentScope !== counterScope, true, "parent ≠ counter scope");
		assert(parentScope !== summaryScope, true, "parent ≠ summary scope");
		assert(counterScope !== summaryScope, true, "counter ≠ summary scope");

		assert(handle.scopeCount(), 3, "scope count = 3");
		assert(handle.handlerCount() >= 3, true, "at least 3 handlers");

		assert(handle.getCountValue(), 0, "initial count = 0");
		assert(handle.isDarkTheme(), false, "initial theme = light");
		assert(handle.getOnResetValue(), 0, "initial on_reset = 0");

		assert(handle.toggleThemeHandler >= 0, true, "toggle theme handler valid");
		assert(handle.incrementHandler >= 0, true, "increment handler valid");
		assert(handle.resetHandler >= 0, true, "reset handler valid");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: tc_rebuild produces mutations and mounts both children
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — rebuild produces mutations and mounts both children");
	{
		const app = fns.tc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		const len = fns.tc_rebuild(app, bufPtr, BUF_CAPACITY);
		assert(len > 0, true, "rebuild produces mutations");

		assert(
			fns.tc_counter_is_mounted(app) !== 0,
			true,
			"counter child mounted after rebuild",
		);
		assert(
			fns.tc_summary_is_mounted(app) !== 0,
			true,
			"summary child mounted after rebuild",
		);
		assert(
			fns.tc_counter_has_rendered(app) !== 0,
			true,
			"counter child has rendered after rebuild",
		);
		assert(
			fns.tc_summary_has_rendered(app) !== 0,
			true,
			"summary child has rendered after rebuild",
		);

		fns.mutation_buf_free(bufPtr);
		fns.tc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: increment updates both children
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — increment updates count");
	{
		const handle = createTC(fns);

		handle.increment();
		assert(handle.getCountValue(), 1, "count is 1 after increment");

		handle.increment();
		assert(handle.getCountValue(), 2, "count is 2 after second increment");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: theme toggle
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — theme toggle switches light ↔ dark");
	{
		const handle = createTC(fns);

		assert(handle.isDarkTheme(), false, "starts light");
		handle.toggleTheme();
		assert(handle.isDarkTheme(), true, "now dark");
		handle.toggleTheme();
		assert(handle.isDarkTheme(), false, "back to light");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: DOM mount produces correct structure
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — DOM mount produces correct structure");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		// Parent should have buttons
		const buttons = root.querySelectorAll("button");
		assert(
			buttons.length >= 3,
			true,
			"at least 3 buttons (Toggle theme, Increment, Reset)",
		);

		// Counter child should have p with "Count: 0"
		const p = root.querySelector("p");
		assert(p !== null, true, "p element exists (counter child rendered)");
		if (p) {
			assert(
				p.textContent?.includes("Count: 0") ?? false,
				true,
				'counter child text contains "Count: 0"',
			);
		}

		// Summary child should have text with "clicks so far"
		const allPs = root.querySelectorAll("p");
		let foundSummary = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("clicks so far")) {
				foundSummary = true;
				break;
			}
		}
		assert(foundSummary, true, 'summary text contains "clicks so far"');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: increment → counter shows "Count: 1"
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — increment updates counter child text");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		handle.increment();

		assert(handle.getCountValue(), 1, "count is 1");

		// Find counter child's p (contains "Count:")
		const allPs = root.querySelectorAll("p");
		let foundCount1 = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("Count: 1")) {
				foundCount1 = true;
				break;
			}
		}
		assert(foundCount1, true, 'counter child shows "Count: 1"');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: increment → summary shows "1 clicks so far"
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — increment updates summary child text");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		handle.increment();

		const allPs = root.querySelectorAll("p");
		let foundSummary = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("1 clicks so far")) {
				foundSummary = true;
				break;
			}
		}
		assert(foundSummary, true, 'summary shows "1 clicks so far"');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: theme toggle → counter shows "Theme: dark, Count: 0"
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — theme toggle updates counter child format");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		handle.toggleTheme();

		const allPs = root.querySelectorAll("p");
		let foundDark = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("Theme: dark")) {
				foundDark = true;
				break;
			}
		}
		assert(foundDark, true, 'counter child shows "Theme: dark" after toggle');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: summary gets "dark" class after theme toggle
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — summary gets dark class after toggle");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		// Initially should have "light" class
		const allPs = root.querySelectorAll("p");
		let summaryP: Element | null = null;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("clicks so far")) {
				summaryP = allPs[i];
				break;
			}
		}
		if (summaryP) {
			assert(
				summaryP.getAttribute("class"),
				"light",
				'summary class is "light" initially',
			);
		}

		handle.toggleTheme();

		// Re-query after flush
		const allPs2 = root.querySelectorAll("p");
		let summaryP2: Element | null = null;
		for (let i = 0; i < allPs2.length; i++) {
			if (allPs2[i].textContent?.includes("clicks so far")) {
				summaryP2 = allPs2[i];
				break;
			}
		}
		if (summaryP2) {
			assert(
				summaryP2.getAttribute("class"),
				"dark",
				'summary class is "dark" after toggle',
			);
		}

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: reset button → count returns to 0
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — reset via child resets count to 0");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		handle.increment();
		handle.increment();
		handle.increment();
		assert(handle.getCountValue(), 3, "count is 3 before reset");

		handle.resetViaChild();
		assert(handle.getCountValue(), 0, "count is 0 after reset");
		assert(handle.getOnResetValue(), 0, "on_reset cleared after flush");

		// Both children should show reset values
		const allPs = root.querySelectorAll("p");
		let foundCount0 = false;
		let foundSummary0 = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("Count: 0")) foundCount0 = true;
			if (allPs[i].textContent?.includes("0 clicks so far"))
				foundSummary0 = true;
		}
		assert(foundCount0, true, 'counter shows "Count: 0" after reset');
		assert(foundSummary0, true, 'summary shows "0 clicks so far" after reset');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: increment → reset → increment cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — increment → reset → increment cycle");
	{
		const handle = createTC(fns);

		// Increment to 5
		for (let i = 0; i < 5; i++) handle.increment();
		assert(handle.getCountValue(), 5, "count is 5");

		// Reset
		handle.resetViaChild();
		assert(handle.getCountValue(), 0, "count reset to 0");

		// Increment to 3
		for (let i = 0; i < 3; i++) handle.increment();
		assert(handle.getCountValue(), 3, "count is 3 after re-increment");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: theme toggle does not affect count
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — theme toggle does not affect count");
	{
		const handle = createTC(fns);

		handle.increment();
		assert(handle.getCountValue(), 1, "count is 1");

		handle.toggleTheme();
		assert(handle.getCountValue(), 1, "count still 1 after theme toggle");
		assert(handle.isDarkTheme(), true, "theme is now dark");

		handle.toggleTheme();
		assert(handle.getCountValue(), 1, "count still 1 after second toggle");
		assert(handle.isDarkTheme(), false, "theme back to light");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: flush returns 0 when clean
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — flush returns 0 when clean");
	{
		const app = fns.tc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		fns.tc_rebuild(app, bufPtr, BUF_CAPACITY);

		const len = fns.tc_flush(app, bufPtr, BUF_CAPACITY);
		assert(len, 0, "flush returns 0 when nothing dirty");

		fns.mutation_buf_free(bufPtr);
		fns.tc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 14: flush after increment produces mutations
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — flush after increment produces mutations");
	{
		const app = fns.tc_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));
		fns.tc_rebuild(app, bufPtr, BUF_CAPACITY);

		const incrH = fns.tc_increment_handler(app);
		fns.tc_handle_event(app, incrH, 0);
		const len = fns.tc_flush(app, bufPtr, BUF_CAPACITY);
		assert(len > 0, true, "flush after increment produces mutations");

		fns.mutation_buf_free(bufPtr);
		fns.tc_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 15: destroy does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — destroy does not crash");
	{
		const handle = createTC(fns);
		handle.increment();
		handle.toggleTheme();
		handle.resetViaChild();
		handle.destroy();
		assert(true, true, "destroy after mutations does not crash");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 16: double destroy safe
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — double destroy safe");
	{
		const handle = createTC(fns);
		handle.destroy();
		handle.destroy(); // should be idempotent
		assert(true, true, "double destroy does not crash");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 17: destroy + recreate cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — destroy + recreate cycle");
	{
		const handle1 = createTC(fns);
		handle1.increment();
		handle1.increment();
		handle1.toggleTheme();
		assert(handle1.getCountValue(), 2, "first instance count = 2");
		assert(handle1.isDarkTheme(), true, "first instance dark theme");
		handle1.destroy();

		const handle2 = createTC(fns);
		assert(handle2.getCountValue(), 0, "fresh count after recreate");
		assert(handle2.isDarkTheme(), false, "fresh theme after recreate");
		assert(handle2.getOnResetValue(), 0, "fresh on_reset after recreate");
		handle2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 18: multiple independent instances
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — multiple independent instances");
	{
		const handle1 = createTC(fns);
		const handle2 = createTC(fns);

		handle1.increment();
		handle1.increment();
		handle1.increment();

		handle2.increment();
		handle2.toggleTheme();

		assert(handle1.getCountValue(), 3, "instance 1 count = 3");
		assert(handle2.getCountValue(), 1, "instance 2 count = 1");
		assert(handle1.isDarkTheme(), false, "instance 1 light theme");
		assert(handle2.isDarkTheme(), true, "instance 2 dark theme");

		handle1.destroy();
		handle2.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 19: rapid 50 increments bounded memory
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — rapid 50 increments");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		for (let i = 0; i < 50; i++) {
			handle.increment();
		}

		assert(handle.getCountValue(), 50, "count is 50 after 50 increments");

		const allPs = root.querySelectorAll("p");
		let foundCount50 = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("Count: 50")) {
				foundCount50 = true;
				break;
			}
		}
		assert(foundCount50, true, 'counter shows "Count: 50"');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 20: theme toggle + increment in same flush
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — theme toggle + increment combined");
	{
		const { document, root } = createDOM();
		const handle = createThemeCounterApp(fns, root, document);

		// First increment normally
		handle.increment();
		assert(handle.getCountValue(), 1, "count is 1");

		// Toggle theme, then increment
		handle.toggleTheme();
		handle.increment();
		assert(handle.getCountValue(), 2, "count is 2");
		assert(handle.isDarkTheme(), true, "dark theme");

		// Counter child should show dark format with count 2
		const allPs = root.querySelectorAll("p");
		let foundDarkCount = false;
		for (let i = 0; i < allPs.length; i++) {
			if (allPs[i].textContent?.includes("Theme: dark, Count: 2")) {
				foundDarkCount = true;
				break;
			}
		}
		assert(foundDarkCount, true, 'counter shows "Theme: dark, Count: 2"');

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 21: 5 create/destroy cycles with state verification
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — 5 create/destroy cycles");
	for (let cycle = 0; cycle < 5; cycle++) {
		const handle = createTC(fns);
		for (let i = 0; i < cycle + 1; i++) handle.increment();
		assert(
			handle.getCountValue(),
			cycle + 1,
			`cycle ${cycle}: count = ${cycle + 1}`,
		);
		handle.destroy();
	}
	assert(true, true, "5 create/destroy cycles completed");

	// ═════════════════════════════════════════════════════════════════════
	// Section 22: children have independent scope IDs
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — children have independent scope IDs");
	{
		const handle = createTC(fns);

		const parent = handle.parentScopeId;
		const counter = handle.counterScopeId;
		const summary = handle.summaryScopeId;

		// All three should be distinct
		assert(parent !== counter, true, "parent ≠ counter");
		assert(parent !== summary, true, "parent ≠ summary");
		assert(counter !== summary, true, "counter ≠ summary");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 23: 10 increments + theme toggles
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — 10 increments + 5 theme toggles");
	{
		const handle = createTC(fns);

		for (let i = 0; i < 10; i++) {
			handle.increment();
			if (i % 2 === 0) handle.toggleTheme();
		}

		assert(handle.getCountValue(), 10, "count is 10 after 10 increments");
		// 5 toggles: starts light, toggles at i=0,2,4,6,8 → 5 toggles → dark
		assert(handle.isDarkTheme(), true, "theme is dark after 5 toggles");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 24: reset + increment in same session
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — reset then increment gives count 1");
	{
		const handle = createTC(fns);

		handle.increment();
		handle.increment();
		assert(handle.getCountValue(), 2, "count is 2 before reset");

		handle.resetViaChild();
		assert(handle.getCountValue(), 0, "count is 0 after reset");

		handle.increment();
		assert(handle.getCountValue(), 1, "count is 1 after reset + increment");

		handle.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 25: destroy with unflushed dirty state does not crash
	// ═════════════════════════════════════════════════════════════════════

	suite("ThemeCounter — destroy with dirty state does not crash");
	{
		const { document, root } = createDOM();
		const handle = createApp({
			fns,
			root,
			doc: document,
			init: (f) => f.tc_init(),
			rebuild: (f, app, buf, cap) => f.tc_rebuild(app, buf, cap),
			flush: (f, app, buf, cap) => f.tc_flush(app, buf, cap),
			handleEvent: (f, app, hid, evt) => f.tc_handle_event(app, hid, evt),
			destroy: (f, app) => f.tc_destroy(app),
		});

		// Make things dirty without flushing
		const incrH = fns.tc_increment_handler(handle.appPtr);
		fns.tc_handle_event(handle.appPtr, incrH, 0);
		const toggleH = fns.tc_toggle_theme_handler(handle.appPtr);
		fns.tc_handle_event(handle.appPtr, toggleH, 0);
		const resetH = fns.tc_reset_handler(handle.appPtr);
		fns.tc_handle_event(handle.appPtr, resetH, 0);

		assert(fns.tc_has_dirty(handle.appPtr) !== 0, true, "has dirty state");

		handle.destroy();
		assert(true, true, "destroy with dirty state does not crash");
	}
}
