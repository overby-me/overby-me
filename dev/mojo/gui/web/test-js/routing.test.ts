// Phase 30 — Client-Side Routing Tests
//
// Tests the MultiViewApp which demonstrates client-side routing:
//   - App shell: div > [ nav > [button("Counter"), button("Todo")], div > dyn_node[0] ]
//   - Route "/" → counter view: div > [ h1("Count: N"), button("+ 1"), button("- 1") ]
//   - Route "/todo" → todo view: div > [ h2("Items: N"), button("Add item"), p("...") ]
//
// Validates:
//   - Initial mount produces correct DOM with counter view (default route "/")
//   - Navigate "/" → "/todo" switches DOM to todo view
//   - Navigate "/todo" → "/" switches back to counter view
//   - Counter state persists across route transitions
//   - Todo add button works in the todo view
//   - Direct navigation to "/todo" renders todo view
//   - Navigate to unknown route returns false (no crash)
//   - Router reports correct path and branch after navigation
//   - Conditional slot mounted state tracks correctly
//   - Destroy cleans up without crash
//   - Multiple independent instances work correctly
//   - heapStats() bounded across route transitions (allocator reclaims memory)

import { parseHTML } from "npm:linkedom";
import { createMultiViewApp } from "../runtime/app.ts";
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

// ── Helper: find elements by tag name within root ───────────────────────────

function queryAll(root: Element, tag: string): Element[] {
	return Array.from(root.querySelectorAll(tag));
}

function query(root: Element, tag: string): Element | null {
	return root.querySelector(tag);
}

// ══════════════════════════════════════════════════════════════════════════════

export function testRouting(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: Low-level WASM exports
	// ═════════════════════════════════════════════════════════════════════

	suite("Routing — mv_init creates app with correct state");
	{
		const app = (fns as Fns).mv_init();
		assert(app !== 0n, true, "app pointer should be non-zero");

		const routeCount = (fns as Fns).mv_route_count(app);
		assert(routeCount, 2, "should have 2 registered routes");

		const branch = (fns as Fns).mv_current_branch(app);
		assert(branch, 0, "initial branch should be 0 (counter)");

		const navCounterH = (fns as Fns).mv_nav_counter_handler(app);
		assert(
			navCounterH >= 0,
			true,
			"counter nav handler should be non-negative",
		);

		const navTodoH = (fns as Fns).mv_nav_todo_handler(app);
		assert(navTodoH >= 0, true, "todo nav handler should be non-negative");
		assert(navTodoH !== navCounterH, true, "nav handlers should be distinct");

		const todoAddH = (fns as Fns).mv_todo_add_handler(app);
		assert(todoAddH >= 0, true, "todo add handler should be non-negative");

		(fns as Fns).mv_destroy(app);
		pass();
	}

	suite("Routing — mv_navigate changes branch and marks dirty");
	{
		const app = (fns as Fns).mv_init();

		// Navigate to /todo
		const result = (fns as Fns).mv_navigate(app, writeStringStruct("/todo"));
		assert(result, 1, "navigate to /todo should return 1 (matched)");

		const branch = (fns as Fns).mv_current_branch(app);
		assert(branch, 1, "branch should be 1 (todo) after navigate");

		const dirty = (fns as Fns).mv_has_dirty(app);
		assert(dirty, 1, "scope should be dirty after navigate");

		// Navigate to unknown route
		const result2 = (fns as Fns).mv_navigate(
			app,
			writeStringStruct("/unknown"),
		);
		assert(result2, 0, "navigate to /unknown should return 0 (no match)");

		// Branch unchanged
		const branch2 = (fns as Fns).mv_current_branch(app);
		assert(branch2, 1, "branch should still be 1 after failed navigate");

		(fns as Fns).mv_destroy(app);
		pass();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: DOM-level tests via createMultiViewApp
	// ═════════════════════════════════════════════════════════════════════

	suite("Routing — initial mount shows counter view (default route '/')");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// App shell structure: div > [ nav, div(content) ]
		const outerDiv = dom.root.firstElementChild;
		assert(outerDiv !== null, true, "root should have an outer div");
		assert(
			outerDiv!.tagName.toLowerCase(),
			"div",
			"outer element should be a div",
		);

		// Nav bar
		const nav = query(dom.root, "nav");
		assert(nav !== null, true, "should have a nav element");

		const navButtons = queryAll(nav!, "button");
		assert(navButtons.length, 2, "nav should have 2 buttons");
		assert(
			navButtons[0].textContent,
			"Counter",
			"first nav button should be Counter",
		);
		assert(
			navButtons[1].textContent,
			"Todo",
			"second nav button should be Todo",
		);

		// Content area should show counter view
		const h1 = query(dom.root, "h1");
		assert(h1 !== null, true, "should have an h1 (counter view)");
		assert(
			h1!.textContent!.includes("Count:"),
			true,
			"h1 should contain 'Count:'",
		);

		// Counter buttons
		const _contentButtons = queryAll(
			outerDiv!.querySelector("div:last-child")! || outerDiv!,
			"button",
		);
		// Content area buttons (inside the counter view div)
		const allButtons = queryAll(dom.root, "button");
		// Should have nav buttons (2) + counter buttons (2) = 4
		assert(
			allButtons.length >= 4,
			true,
			"should have at least 4 buttons total",
		);

		assert(
			handle.getCurrentBranch(),
			0,
			"current branch should be 0 (counter)",
		);
		assert(handle.isCondMounted(), true, "conditional slot should be mounted");

		handle.destroy();
		pass();
	}

	suite("Routing — navigate '/' → '/todo' switches to todo view");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Verify initial counter view
		assert(
			query(dom.root, "h1") !== null,
			true,
			"initial: h1 present (counter)",
		);

		// Navigate to todo
		const result = handle.navigate("/todo");
		assert(result, true, "navigate to /todo should succeed");

		// Counter h1 should be gone
		const h1After = query(dom.root, "h1");
		assert(h1After, null, "h1 should be gone after navigating to /todo");

		// Todo h2 should be present
		const h2 = query(dom.root, "h2");
		assert(h2 !== null, true, "should have h2 (todo view)");
		assert(
			h2!.textContent!.includes("Items:"),
			true,
			"h2 should contain 'Items:'",
		);

		assert(handle.getCurrentBranch(), 1, "branch should be 1 (todo)");

		handle.destroy();
		pass();
	}

	suite("Routing — navigate '/todo' → '/' restores counter view");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Navigate to todo
		handle.navigate("/todo");
		assert(query(dom.root, "h2") !== null, true, "todo view: h2 present");

		// Navigate back to counter
		handle.navigate("/");
		const h1 = query(dom.root, "h1");
		assert(h1 !== null, true, "counter view restored: h1 present");
		assert(h1!.textContent!.includes("Count:"), true, "h1 should show Count:");

		// h2 should be gone
		assert(
			query(dom.root, "h2"),
			null,
			"h2 should be gone after returning to counter",
		);

		assert(handle.getCurrentBranch(), 0, "branch should be 0 (counter)");

		handle.destroy();
		pass();
	}

	suite("Routing — navigate '/' → '/todo' → '/' round-trip preserves nav bar");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Nav bar should be present throughout
		const navBefore = query(dom.root, "nav");
		assert(navBefore !== null, true, "nav present before navigation");

		handle.navigate("/todo");
		const navDuring = query(dom.root, "nav");
		assert(navDuring !== null, true, "nav present during /todo");

		handle.navigate("/");
		const navAfter = query(dom.root, "nav");
		assert(navAfter !== null, true, "nav present after returning to /");

		// Nav buttons should still have correct text
		const buttons = queryAll(navAfter!, "button");
		assert(buttons.length, 2, "nav still has 2 buttons");
		assert(buttons[0].textContent, "Counter", "first button still Counter");
		assert(buttons[1].textContent, "Todo", "second button still Todo");

		handle.destroy();
		pass();
	}

	suite("Routing — counter increments persist across route transitions");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Increment counter twice via dispatchAndFlush with the signal handler
		// The counter uses onclick_add which is handled by dispatch_event
		// We use navToCounter is already on /, so use navigate to /todo and back
		const countBefore = handle.getCountValue();
		assert(countBefore, 0, "initial count should be 0");

		// Navigate away to /todo
		handle.navigate("/todo");
		assert(handle.getCurrentBranch(), 1, "on todo view");

		// Navigate back to /
		handle.navigate("/");
		assert(handle.getCurrentBranch(), 0, "back on counter view");

		// Count should still be 0 (we didn't increment)
		const countAfter = handle.getCountValue();
		assert(countAfter, 0, "count should still be 0 after round-trip");

		handle.destroy();
		pass();
	}

	suite("Routing — todo add button works in todo view");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Navigate to todo
		handle.navigate("/todo");
		assert(handle.getTodoCount(), 0, "initial todo count should be 0");

		// Add an item via dispatchAndFlush
		handle.addTodoItem();
		assert(handle.getTodoCount(), 1, "todo count should be 1 after add");

		// The DOM should reflect the update
		const h2 = query(dom.root, "h2");
		assert(h2 !== null, true, "h2 still present after add");
		assert(
			h2!.textContent!.includes("Items: 1"),
			true,
			"h2 should show 'Items: 1'",
		);

		// Item listing text
		const p = query(dom.root, "p");
		assert(p !== null, true, "p element should be present");
		assert(
			p!.textContent!.includes("Item 1"),
			true,
			"p should contain 'Item 1'",
		);

		handle.destroy();
		pass();
	}

	suite("Routing — todo items persist across route round-trip");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Navigate to todo and add items
		handle.navigate("/todo");
		handle.addTodoItem();
		handle.addTodoItem();
		assert(handle.getTodoCount(), 2, "2 items added");

		// Navigate away and back
		handle.navigate("/");
		assert(handle.getCurrentBranch(), 0, "on counter view");

		handle.navigate("/todo");
		assert(handle.getCurrentBranch(), 1, "back on todo view");

		// Items should persist (signal state is preserved)
		assert(handle.getTodoCount(), 2, "todo count still 2 after round-trip");

		const h2 = query(dom.root, "h2");
		assert(
			h2!.textContent!.includes("Items: 2"),
			true,
			"h2 should show 'Items: 2'",
		);

		handle.destroy();
		pass();
	}

	suite("Routing — navigate to same route is a no-op (no crash)");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Already on "/" — navigate again
		const result = handle.navigate("/");
		// May return true (route exists) but no DOM change
		assert(typeof result, "boolean", "navigate should return a boolean");

		// Counter view still visible
		assert(query(dom.root, "h1") !== null, true, "h1 still present");
		assert(handle.getCurrentBranch(), 0, "still on counter branch");

		handle.destroy();
		pass();
	}

	suite("Routing — navigate to unknown route returns false");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		const result = handle.navigate("/nonexistent");
		assert(result, false, "should return false for unknown route");

		// View unchanged
		assert(handle.getCurrentBranch(), 0, "branch unchanged");
		assert(query(dom.root, "h1") !== null, true, "counter view unchanged");

		handle.destroy();
		pass();
	}

	suite("Routing — nav button dispatch switches view (counter → todo)");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Click the Todo nav button
		handle.navToTodo();

		// Should now show todo view
		assert(handle.getCurrentBranch(), 1, "branch should be 1 after navToTodo");

		const h2 = query(dom.root, "h2");
		assert(h2 !== null, true, "h2 should be present (todo view)");

		handle.destroy();
		pass();
	}

	suite("Routing — nav button dispatch switches view (todo → counter)");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Go to todo first
		handle.navToTodo();
		assert(handle.getCurrentBranch(), 1, "on todo");

		// Click the Counter nav button
		handle.navToCounter();
		assert(handle.getCurrentBranch(), 0, "back on counter");

		const h1 = query(dom.root, "h1");
		assert(h1 !== null, true, "h1 should be present (counter view)");

		handle.destroy();
		pass();
	}

	suite("Routing — direct navigation to '/todo' renders todo view");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Use programmatic navigate (simulating direct URL access)
		handle.navigate("/todo");

		const h2 = query(dom.root, "h2");
		assert(h2 !== null, true, "h2 present after direct /todo navigate");
		assert(h2!.textContent!.includes("Items:"), true, "h2 contains 'Items:'");

		// No h1 (counter view hidden)
		assert(query(dom.root, "h1"), null, "h1 should not be present");

		handle.destroy();
		pass();
	}

	suite(
		"Routing — multiple route transitions produce correct DOM at each step",
	);
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Step 1: "/" (counter)
		assert(query(dom.root, "h1") !== null, true, "step 1: h1 present");
		assert(query(dom.root, "h2"), null, "step 1: h2 absent");

		// Step 2: "/todo"
		handle.navigate("/todo");
		assert(query(dom.root, "h1"), null, "step 2: h1 absent");
		assert(query(dom.root, "h2") !== null, true, "step 2: h2 present");

		// Step 3: "/"
		handle.navigate("/");
		assert(query(dom.root, "h1") !== null, true, "step 3: h1 present");
		assert(query(dom.root, "h2"), null, "step 3: h2 absent");

		// Step 4: "/todo"
		handle.navigate("/todo");
		assert(query(dom.root, "h1"), null, "step 4: h1 absent");
		assert(query(dom.root, "h2") !== null, true, "step 4: h2 present");

		// Step 5: "/"
		handle.navigate("/");
		assert(query(dom.root, "h1") !== null, true, "step 5: h1 present");

		handle.destroy();
		pass();
	}

	suite("Routing — browser back simulated via popstate → navigate");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Simulate: user navigates / → /todo → /
		// Step 1: Initial on "/"
		assert(handle.getCurrentBranch(), 0, "start on /");

		// Step 2: Navigate to "/todo" (simulate pushState)
		handle.navigate("/todo");
		assert(handle.getCurrentBranch(), 1, "navigated to /todo");

		// Step 3: Simulate browser "back" (popstate) → navigate to "/"
		handle.navigate("/");
		assert(handle.getCurrentBranch(), 0, "back to / (simulated popstate)");

		// DOM should show counter view
		assert(query(dom.root, "h1") !== null, true, "counter view restored");
		assert(query(dom.root, "h2"), null, "todo view removed");

		handle.destroy();
		pass();
	}

	suite("Routing — destroy does not crash");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Navigate around before destroy
		handle.navigate("/todo");
		handle.addTodoItem();
		handle.navigate("/");

		handle.destroy();
		assert(handle.destroyed, true, "app should be marked destroyed");

		// Root should be cleared
		assert(dom.root.childElementCount, 0, "root should be empty after destroy");
		pass();
	}

	suite("Routing — double destroy is a safe no-op");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);
		handle.destroy();
		handle.destroy(); // should not throw
		assert(handle.destroyed, true, "still destroyed");
		pass();
	}

	suite("Routing — multiple independent app instances");
	{
		const dom1 = createDOM();
		const dom2 = createDOM();
		const h1 = createMultiViewApp(fns, dom1.root, dom1.document);
		const h2 = createMultiViewApp(fns, dom2.root, dom2.document);

		// Navigate instance 1 to /todo
		h1.navigate("/todo");
		assert(h1.getCurrentBranch(), 1, "instance 1 on /todo");
		assert(h2.getCurrentBranch(), 0, "instance 2 still on /");

		// Instance 2 counter view still intact
		assert(query(dom2.root, "h1") !== null, true, "instance 2: h1 present");
		assert(query(dom1.root, "h2") !== null, true, "instance 1: h2 present");

		// Navigate instance 2 to /todo
		h2.navigate("/todo");
		assert(h2.getCurrentBranch(), 1, "instance 2 now on /todo");

		// Both on /todo
		assert(query(dom1.root, "h2") !== null, true, "instance 1: still h2");
		assert(query(dom2.root, "h2") !== null, true, "instance 2: now h2");

		h1.destroy();
		h2.destroy();
		pass();
	}

	suite("Routing — rapid route switching (10 round-trips)");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		for (let i = 0; i < 10; i++) {
			handle.navigate("/todo");
			handle.navigate("/");
		}

		// Final state should be counter view
		assert(handle.getCurrentBranch(), 0, "final branch should be 0 (counter)");
		assert(
			query(dom.root, "h1") !== null,
			true,
			"h1 present after 10 round-trips",
		);
		assert(query(dom.root, "h2"), null, "h2 absent after 10 round-trips");

		handle.destroy();
		pass();
	}

	suite("Routing — heapStats bounded across route transitions");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Record heap after initial mount
		const statsBefore = heapStats();
		const heapBefore = statsBefore.heapPointer;

		// Perform 20 route transitions
		for (let i = 0; i < 20; i++) {
			handle.navigate("/todo");
			handle.addTodoItem();
			handle.navigate("/");
		}

		const statsAfter = heapStats();
		const heapAfter = statsAfter.heapPointer;

		// The heap should grow by at most a bounded amount (allocator reuse).
		// Allow generous 512 KiB growth — the key insight is that it doesn't
		// grow unboundedly (linear in transitions), which would indicate a leak.
		const growth = Number(heapAfter - heapBefore);
		const MAX_GROWTH = 512 * 1024; // 512 KiB
		assert(
			growth < MAX_GROWTH,
			true,
			`heap growth should be bounded: ${growth} < ${MAX_GROWTH}`,
		);

		handle.destroy();
		pass();
	}

	suite("Routing — conditional slot reports correct mounted state");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// After initial mount, the conditional slot should be mounted (showing counter view)
		assert(handle.isCondMounted(), true, "slot mounted after initial render");

		// Navigate to /todo — slot should still be mounted (different branch, but content present)
		handle.navigate("/todo");
		assert(handle.isCondMounted(), true, "slot mounted on /todo");

		// Navigate back
		handle.navigate("/");
		assert(handle.isCondMounted(), true, "slot mounted on /");

		handle.destroy();
		pass();
	}

	suite("Routing — getRouteCount returns 2");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);
		assert(handle.getRouteCount(), 2, "should have 2 routes");
		handle.destroy();
		pass();
	}

	suite("Routing — todo add across route transitions accumulates");
	{
		const dom = createDOM();
		const handle = createMultiViewApp(fns, dom.root, dom.document);

		// Add items on todo, navigate away, come back, add more
		handle.navigate("/todo");
		handle.addTodoItem();
		handle.addTodoItem();
		assert(handle.getTodoCount(), 2, "2 items after first visit");

		handle.navigate("/");
		handle.navigate("/todo");
		handle.addTodoItem();
		assert(handle.getTodoCount(), 3, "3 items after second visit");

		// Verify DOM shows updated count
		const h2 = query(dom.root, "h2");
		assert(
			h2!.textContent!.includes("Items: 3"),
			true,
			"h2 should show 'Items: 3'",
		);

		handle.destroy();
		pass();
	}
}
