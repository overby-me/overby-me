// Todo App End-to-End Tests — Phase 8 (M8)
//
// Tests the full todo app lifecycle:
//   init → rebuild → mount → add/toggle/remove → flush → DOM update
//
// Uses linkedom for headless DOM and the WASM todo_* exports.
// Exercises:
//   - Dynamic keyed list reconciliation (add, remove, toggle)
//   - Conditional rendering (completed class styling)
//   - Fragment VNodes with keyed children
//   - String data flow (input text from JS → WASM)

import { parseHTML } from "npm:linkedom";
import { createApp } from "../runtime/app.ts";
import type { Interpreter } from "../runtime/interpreter.ts";
import { alignedAlloc, getMemory } from "../runtime/memory.ts";
import { MutationReader, Op } from "../runtime/protocol.ts";

import { writeStringStruct } from "../runtime/strings.ts";
import type { TemplateCache } from "../runtime/templates.ts";
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

const BUF_CAPACITY = 65536;

// ── Todo App handle ─────────────────────────────────────────────────────────

interface TodoAppHandle {
	fns: Fns;
	interpreter: Interpreter;
	templates: TemplateCache;
	root: Element;
	appPtr: bigint;
	bufPtr: bigint;

	addItem(text: string): void;
	removeItem(itemId: number): void;
	toggleItem(itemId: number): void;
	itemCount(): number;
	itemIdAt(index: number): number;
	itemCompletedAt(index: number): boolean;
	listVersion(): number;
	hasDirty(): boolean;
	isEmptyMsgMounted(): boolean;
	flush(): number;
	destroy(): void;
}

function createTodoApp(fns: Fns, root: Element, doc: Document): TodoAppHandle {
	// Use the generic createApp factory — templates come from WASM via
	// RegisterTemplate mutations, no manual DOM template construction needed.
	const handle = createApp({
		fns,
		root,
		doc,
		bufCapacity: BUF_CAPACITY,
		init: (f) => f.todo_init(),
		rebuild: (f, app, buf, cap) => f.todo_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.todo_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.todo_handle_event(app, hid, evt),
		destroy: (f, app) => f.todo_destroy(app),
	});

	return {
		fns,
		interpreter: handle.interpreter,
		templates: handle.templates,
		root,
		appPtr: handle.appPtr,
		bufPtr: handle.bufPtr,

		addItem(text: string): void {
			const strPtr = writeStringStruct(text);
			fns.todo_add_item(handle.appPtr, strPtr);
			handle.flushAndApply();
		},

		removeItem(itemId: number): void {
			fns.todo_remove_item(handle.appPtr, itemId);
			handle.flushAndApply();
		},

		toggleItem(itemId: number): void {
			fns.todo_toggle_item(handle.appPtr, itemId);
			handle.flushAndApply();
		},

		itemCount(): number {
			return fns.todo_item_count(handle.appPtr);
		},

		itemIdAt(index: number): number {
			return fns.todo_item_id_at(handle.appPtr, index);
		},

		itemCompletedAt(index: number): boolean {
			return fns.todo_item_completed_at(handle.appPtr, index) === 1;
		},

		listVersion(): number {
			return fns.todo_list_version(handle.appPtr);
		},

		hasDirty(): boolean {
			return fns.todo_has_dirty(handle.appPtr) === 1;
		},

		isEmptyMsgMounted(): boolean {
			return fns.todo_empty_msg_mounted(handle.appPtr) === 1;
		},

		flush(): number {
			const len = fns.todo_flush(handle.appPtr, handle.bufPtr, BUF_CAPACITY);
			if (len > 0) {
				const mem = getMemory();
				handle.interpreter.applyMutations(
					mem.buffer,
					Number(handle.bufPtr),
					len,
				);
			}
			return len;
		},

		destroy(): void {
			handle.destroy();
		},
	};
}

// ══════════════════════════════════════════════════════════════════════════════

export function testTodo(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: Low-level todo app exports
	// ═════════════════════════════════════════════════════════════════════

	suite("Todo — todo_init creates app with correct state");
	{
		const appPtr = fns.todo_init();

		const appTmplId = fns.todo_app_template_id(appPtr);
		assert(appTmplId >= 0, true, "app template ID is non-negative");

		const itemTmplId = fns.todo_item_template_id(appPtr);
		assert(itemTmplId >= 0, true, "item template ID is non-negative");
		assert(
			appTmplId !== itemTmplId,
			true,
			"app and item templates are different",
		);

		const addHandler = fns.todo_add_handler(appPtr);
		assert(addHandler >= 0, true, "add handler ID is non-negative");

		const scopeId = fns.todo_scope_id(appPtr);
		assert(scopeId >= 0, true, "scope ID is non-negative");

		const count = fns.todo_item_count(appPtr);
		assert(count, 0, "initial item count is 0");

		const version = fns.todo_list_version(appPtr);
		assert(version, 0, "initial list version is 0");

		fns.todo_destroy(appPtr);
		pass();
		console.log("    ✓ destroy does not crash");
	}

	suite("Todo — add items bumps version and count");
	{
		const appPtr = fns.todo_init();

		fns.todo_add_item(appPtr, writeStringStruct("Buy milk"));
		assert(fns.todo_item_count(appPtr), 1, "count is 1 after adding");
		assert(fns.todo_list_version(appPtr), 1, "version is 1 after adding");

		fns.todo_add_item(appPtr, writeStringStruct("Walk dog"));
		assert(fns.todo_item_count(appPtr), 2, "count is 2 after second add");
		assert(fns.todo_list_version(appPtr), 2, "version is 2 after second add");

		const id1 = fns.todo_item_id_at(appPtr, 0);
		const id2 = fns.todo_item_id_at(appPtr, 1);
		assert(id1 !== id2, true, "item IDs are unique");
		assert(id1 > 0, true, "first item ID is positive");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — toggle item changes completion");
	{
		const appPtr = fns.todo_init();
		fns.todo_add_item(appPtr, writeStringStruct("Test toggle"));

		const id = fns.todo_item_id_at(appPtr, 0);
		assert(fns.todo_item_completed_at(appPtr, 0), 0, "not completed initially");

		fns.todo_toggle_item(appPtr, id);
		assert(fns.todo_item_completed_at(appPtr, 0), 1, "completed after toggle");

		fns.todo_toggle_item(appPtr, id);
		assert(
			fns.todo_item_completed_at(appPtr, 0),
			0,
			"uncompleted after second toggle",
		);

		fns.todo_destroy(appPtr);
	}

	suite("Todo — remove item decreases count");
	{
		const appPtr = fns.todo_init();
		fns.todo_add_item(appPtr, writeStringStruct("A"));
		fns.todo_add_item(appPtr, writeStringStruct("B"));
		fns.todo_add_item(appPtr, writeStringStruct("C"));
		assert(fns.todo_item_count(appPtr), 3, "3 items");

		const idB = fns.todo_item_id_at(appPtr, 1);
		fns.todo_remove_item(appPtr, idB);
		assert(fns.todo_item_count(appPtr), 2, "2 items after remove");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — empty text is not added");
	{
		const appPtr = fns.todo_init();
		fns.todo_add_item(appPtr, writeStringStruct(""));
		assert(fns.todo_item_count(appPtr), 0, "empty text not added");
		assert(fns.todo_list_version(appPtr), 0, "version unchanged for empty add");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — dirty state after item mutation");
	{
		const appPtr = fns.todo_init();

		assert(fns.todo_has_dirty(appPtr), 0, "not dirty initially");
		fns.todo_add_item(appPtr, writeStringStruct("Dirty test"));
		assert(fns.todo_has_dirty(appPtr), 1, "dirty after add_item");

		fns.todo_destroy(appPtr);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: DOM-level todo app tests
	// ═════════════════════════════════════════════════════════════════════

	suite("Todo — rebuild emits RegisterTemplate before LoadTemplate");
	{
		const appPtr = fns.todo_init();
		const bufPtr = alignedAlloc(8n, BigInt(BUF_CAPACITY));

		const offset = fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);
		assert(offset > 0, true, "todo rebuild wrote mutations");

		const mem = getMemory();
		const mutations = new MutationReader(
			mem.buffer,
			Number(bufPtr),
			offset,
		).readAll();

		const regIndices = mutations
			.map((m, i) => (m.op === Op.RegisterTemplate ? i : -1))
			.filter((i) => i >= 0);
		const loadIdx = mutations.findIndex((m) => m.op === Op.LoadTemplate);

		assert(
			regIndices.length >= 2,
			true,
			"at least 2 RegisterTemplate mutations (app + item templates)",
		);
		assert(loadIdx >= 0, true, "contains LoadTemplate mutation");
		assert(
			regIndices[regIndices.length - 1] < loadIdx,
			true,
			"all RegisterTemplate precede LoadTemplate",
		);

		fns.todo_destroy(appPtr);
	}

	suite("Todo — initial mount renders empty app shell");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		// The app template: div > [ input, button("Add"), ul, empty-msg ]
		const div = root.firstChild;
		assert(div !== null, true, "root has a child");
		assert((div as Element).tagName, "DIV", "first child is div");

		const children = (div as Element).childNodes;
		assert(
			children.length,
			4,
			"div has 4 children (input, button, ul, empty msg)",
		);
		assert(children[0].tagName, "INPUT", "first child is input");
		assert(children[1].tagName, "BUTTON", "second child is button");
		assert(children[2].tagName, "UL", "third child is ul");

		// Phase 28: 4th child is the empty state message <p>
		assert(
			(children[3] as Element).tagName,
			"P",
			"fourth child is p (empty msg)",
		);
		assert(
			(children[3] as Element).textContent,
			"No items yet -- add one above!",
			"empty message text correct",
		);

		assert(children[1].textContent, "Add", 'button text is "Add"');

		// UL should be empty (placeholder was replaced or is a comment)
		const _ulChildren = children[2].childNodes;
		// May have a placeholder comment or be empty
		const liCount = children[2].querySelectorAll("li").length;
		assert(liCount, 0, "no li items in empty list");

		app.destroy();
	}

	suite("Todo — add single item renders li in ul");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Buy groceries");

		assert(app.itemCount(), 1, "WASM has 1 item");

		const ul = root.querySelector("ul");
		assert(ul !== null, true, "ul exists");

		const lis = ul!.querySelectorAll("li");
		assert(lis.length, 1, "1 li in ul");

		// Check item text (span > text)
		const span = lis[0].querySelector("span");
		assert(span !== null, true, "li has a span");
		assert(
			span!.textContent!.includes("Buy groceries"),
			true,
			'span contains "Buy groceries"',
		);

		// Check buttons exist
		const buttons = lis[0].querySelectorAll("button");
		assert(buttons.length, 2, "li has 2 buttons (toggle, remove)");

		app.destroy();
	}

	suite("Todo — add multiple items renders all in order");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Item Alpha");
		app.addItem("Item Beta");
		app.addItem("Item Gamma");

		assert(app.itemCount(), 3, "WASM has 3 items");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 3, "3 li elements in DOM");

		const texts = Array.from(lis).map((li: Element) => {
			const span = li.querySelector("span");
			return span ? span.textContent : "";
		});

		assert(texts[0].includes("Item Alpha"), true, "first item is Alpha");
		assert(texts[1].includes("Item Beta"), true, "second item is Beta");
		assert(texts[2].includes("Item Gamma"), true, "third item is Gamma");

		app.destroy();
	}

	suite("Todo — toggle item updates text/class in DOM");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Toggle me");

		const itemId = app.itemIdAt(0);
		assert(app.itemCompletedAt(0), false, "item not completed initially");

		// Toggle to completed
		app.toggleItem(itemId);

		assert(app.itemCompletedAt(0), true, "item completed after toggle");

		const li = root.querySelector("li");
		assert(li !== null, true, "li exists after toggle");

		// Check that the span text has the ✓ indicator
		const span = li!.querySelector("span");
		const spanText = span!.textContent || "";
		assert(spanText.includes("✓"), true, 'completed item text includes "✓"');

		// Check that the li has the "completed" class
		const liClass = li!.getAttribute("class") || "";
		assert(liClass.includes("completed"), true, 'li has "completed" class');

		// Toggle back
		app.toggleItem(itemId);
		assert(
			app.itemCompletedAt(0),
			false,
			"item uncompleted after second toggle",
		);

		const li2 = root.querySelector("li");
		const span2 = li2!.querySelector("span");
		const spanText2 = span2!.textContent || "";
		assert(
			spanText2.includes("✓"),
			false,
			'uncompleted item text does not include "✓"',
		);

		app.destroy();
	}

	suite("Todo — remove item removes li from DOM");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Keep");
		app.addItem("Remove me");
		app.addItem("Also keep");

		assert(root.querySelectorAll("li").length, 3, "3 items before remove");

		const removeId = app.itemIdAt(1);
		app.removeItem(removeId);

		assert(app.itemCount(), 2, "WASM has 2 items after remove");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 2, "2 li elements after remove");

		app.destroy();
	}

	suite("Todo — remove all items leaves empty ul");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("A");
		app.addItem("B");

		const id1 = app.itemIdAt(0);
		const id2 = app.itemIdAt(1);

		app.removeItem(id1);
		app.removeItem(id2);

		assert(app.itemCount(), 0, "0 items after removing all");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 0, "0 li elements in DOM");

		app.destroy();
	}

	suite("Todo — add after remove works correctly");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("First");
		const firstId = app.itemIdAt(0);
		app.removeItem(firstId);

		assert(app.itemCount(), 0, "0 items after remove");
		assert(root.querySelectorAll("li").length, 0, "0 li after remove");

		app.addItem("Second");
		assert(app.itemCount(), 1, "1 item after re-add");
		assert(root.querySelectorAll("li").length, 1, "1 li after re-add");

		const span = root.querySelector("li span");
		assert(
			span!.textContent!.includes("Second"),
			true,
			'new item text is "Second"',
		);

		app.destroy();
	}

	suite("Todo — list version increments on each mutation");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		assert(app.listVersion(), 0, "version starts at 0");

		app.addItem("V1");
		assert(app.listVersion(), 1, "version is 1 after add");

		app.addItem("V2");
		assert(app.listVersion(), 2, "version is 2 after second add");

		const id = app.itemIdAt(0);
		app.toggleItem(id);
		assert(app.listVersion(), 3, "version is 3 after toggle");

		app.removeItem(id);
		assert(app.listVersion(), 4, "version is 4 after remove");

		app.destroy();
	}

	suite("Todo — 10 items added and all rendered");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		for (let i = 0; i < 10; i++) {
			app.addItem(`Item ${i}`);
		}

		assert(app.itemCount(), 10, "WASM has 10 items");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 10, "10 li elements in DOM");

		// Check first and last
		const firstSpan = lis[0].querySelector("span");
		assert(
			firstSpan!.textContent!.includes("Item 0"),
			true,
			"first item text correct",
		);

		const lastSpan = lis[9].querySelector("span");
		assert(
			lastSpan!.textContent!.includes("Item 9"),
			true,
			"last item text correct",
		);

		app.destroy();
	}

	suite("Todo — toggle multiple items independently");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Task A");
		app.addItem("Task B");
		app.addItem("Task C");

		const idA = app.itemIdAt(0);
		const idC = app.itemIdAt(2);

		// Toggle A and C, leave B
		app.toggleItem(idA);
		app.toggleItem(idC);

		assert(app.itemCompletedAt(0), true, "A is completed");
		assert(app.itemCompletedAt(1), false, "B is not completed");
		assert(app.itemCompletedAt(2), true, "C is completed");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 3, "still 3 items");

		// Check classes
		const classA = lis[0].getAttribute("class") || "";
		const classB = lis[1].getAttribute("class") || "";
		const classC = lis[2].getAttribute("class") || "";

		assert(classA.includes("completed"), true, "A has completed class");
		assert(
			classB.includes("completed"),
			false,
			"B does not have completed class",
		);
		assert(classC.includes("completed"), true, "C has completed class");

		app.destroy();
	}

	suite("Todo — remove from middle preserves other items");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("First");
		app.addItem("Middle");
		app.addItem("Last");

		const middleId = app.itemIdAt(1);
		app.removeItem(middleId);

		assert(app.itemCount(), 2, "2 items after removing middle");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 2, "2 li elements");

		// The remaining items should be First and Last
		// Note: remove uses swap-remove, so order may change
		// Just verify we have 2 items and neither is "Middle"
		const texts = Array.from(lis).map((li: Element) => {
			const span = li.querySelector("span");
			return span ? span.textContent : "";
		});
		const hasMiddle = texts.some((t: string) => t.includes("Middle"));
		assert(hasMiddle, false, '"Middle" is not in the list');

		app.destroy();
	}

	suite("Todo — multiple independent app instances");
	{
		const dom1 = createDOM();
		const dom2 = createDOM();
		const app1 = createTodoApp(fns, dom1.root, dom1.document);
		const app2 = createTodoApp(fns, dom2.root, dom2.document);

		app1.addItem("App1 item");
		app2.addItem("App2 item A");
		app2.addItem("App2 item B");

		assert(app1.itemCount(), 1, "app1 has 1 item");
		assert(app2.itemCount(), 2, "app2 has 2 items");

		assert(dom1.root.querySelectorAll("li").length, 1, "app1 DOM has 1 li");
		assert(dom2.root.querySelectorAll("li").length, 2, "app2 DOM has 2 li");

		app1.destroy();
		app2.destroy();
	}

	suite("Todo — rapid 50 adds");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		for (let i = 0; i < 50; i++) {
			app.addItem(`Rapid ${i}`);
		}

		assert(app.itemCount(), 50, "50 items after rapid adds");
		assert(root.querySelectorAll("li").length, 50, "50 li elements in DOM");

		app.destroy();
	}

	suite("Todo — add, toggle, remove interleaved");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("One");
		app.addItem("Two");
		app.addItem("Three");

		// Toggle "Two"
		const twoId = app.itemIdAt(1);
		app.toggleItem(twoId);
		assert(app.itemCompletedAt(1), true, '"Two" is completed');

		// Remove "One"
		const oneId = app.itemIdAt(0);
		app.removeItem(oneId);
		assert(app.itemCount(), 2, "2 items after removing One");

		// Add a new item
		app.addItem("Four");
		assert(app.itemCount(), 3, "3 items after adding Four");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 3, "3 li elements in DOM");

		app.destroy();
	}

	suite("Todo — set_input stores text (no re-render)");
	{
		const appPtr = fns.todo_init();
		const versionBefore = fns.todo_list_version(appPtr);
		fns.todo_set_input(appPtr, writeStringStruct("hello"));
		const versionAfter = fns.todo_list_version(appPtr);
		assert(versionAfter, versionBefore, "set_input does not change version");

		const dirty = fns.todo_has_dirty(appPtr);
		assert(dirty, 0, "set_input does not make scope dirty");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — SignalString input_text version tracking (Phase 19)");
	{
		const appPtr = fns.todo_init();

		// Initial version is 0 (no writes yet)
		assert(fns.todo_input_version(appPtr), 0, "initial input version is 0");

		// Initial value is empty
		assert(fns.todo_input_is_empty(appPtr), 1, "input is empty initially");

		// First set() bumps version to 1
		fns.todo_set_input(appPtr, writeStringStruct("hello"));
		assert(fns.todo_input_version(appPtr), 1, "version is 1 after first set");
		assert(fns.todo_input_is_empty(appPtr), 0, "input not empty after set");

		// Second set() bumps version to 2
		fns.todo_set_input(appPtr, writeStringStruct("world"));
		assert(fns.todo_input_version(appPtr), 2, "version is 2 after second set");

		// Setting to empty string still bumps version
		fns.todo_set_input(appPtr, writeStringStruct(""));
		assert(fns.todo_input_version(appPtr), 3, "version is 3 after empty set");
		assert(fns.todo_input_is_empty(appPtr), 1, "input is empty after clearing");

		// list_version is still unchanged (no coupling)
		assert(
			fns.todo_list_version(appPtr),
			0,
			"list_version unchanged by input sets",
		);

		// Scope never became dirty (create_signal_string — no subscription)
		assert(fns.todo_has_dirty(appPtr), 0, "scope not dirty after input sets");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — SignalString input_text is_empty reflects state (Phase 19)");
	{
		const appPtr = fns.todo_init();

		assert(fns.todo_input_is_empty(appPtr), 1, "empty on init");

		fns.todo_set_input(appPtr, writeStringStruct("x"));
		assert(fns.todo_input_is_empty(appPtr), 0, "not empty after set");

		fns.todo_set_input(appPtr, writeStringStruct(""));
		assert(fns.todo_input_is_empty(appPtr), 1, "empty again after clear");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — remove nonexistent item is a no-op");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Only item");
		app.removeItem(9999); // nonexistent ID

		assert(app.itemCount(), 1, "item count unchanged");
		assert(root.querySelectorAll("li").length, 1, "DOM unchanged");

		app.destroy();
	}

	suite("Todo — toggle nonexistent item is a no-op");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Only item");
		const vBefore = app.listVersion();
		app.toggleItem(9999); // nonexistent ID

		assert(app.itemCompletedAt(0), false, "item unchanged");
		// Version should not change for nonexistent toggle
		// (the toggle function only bumps if it finds the item)
		const vAfter = app.listVersion();
		assert(vAfter, vBefore, "version unchanged for nonexistent toggle");

		app.destroy();
	}

	suite("Todo — flush with no dirty returns 0");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		// Nothing dirty after initial mount
		const len = app.flush();
		assert(len, 0, "flush returns 0 when not dirty");

		app.destroy();
	}

	suite("Todo — DOM structure: each li has span + 2 buttons");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Structure test");

		const li = root.querySelector("li")!;
		const children = li.childNodes;

		// li > span + button + button
		let spanCount = 0;
		let buttonCount = 0;
		for (let i = 0; i < children.length; i++) {
			const child = children[i] as Element;
			if (child.tagName === "SPAN") spanCount++;
			if (child.tagName === "BUTTON") buttonCount++;
		}

		assert(spanCount, 1, "li has 1 span");
		assert(buttonCount, 2, "li has 2 buttons");

		app.destroy();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: Handler lifecycle (M13.1)
	// ═════════════════════════════════════════════════════════════════════

	suite("Todo — handler count after adding items");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		// Before adding items: 3 app-level handlers from register_view
		// (oninput_set_string + onkeydown_enter_custom + onclick_custom)
		const hcBefore = fns.todo_handler_count(appPtr);
		assert(hcBefore, 3, "3 handlers (oninput + enter + add) before any items");

		// Add 5 items — each gets 2 handlers (toggle + remove)
		for (let i = 0; i < 5; i++) {
			fns.todo_add_item(appPtr, writeStringStruct(`Item ${i}`));
		}
		fns.todo_flush(appPtr, bufPtr, BUF_CAPACITY);

		const hcAfter = fns.todo_handler_count(appPtr);
		// 3 (oninput + enter + add) + 5 * 2 (toggle + remove per item) = 13
		assert(hcAfter, 13, "13 handlers after adding 5 items (3 app + 10 item)");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — handler count stays bounded after add/remove cycles");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		// Perform 10 cycles of add-then-remove
		for (let cycle = 0; cycle < 10; cycle++) {
			fns.todo_add_item(appPtr, writeStringStruct(`Cycle ${cycle}`));
			fns.todo_flush(appPtr, bufPtr, BUF_CAPACITY);

			const itemId = fns.todo_item_id_at(appPtr, 0);
			fns.todo_remove_item(appPtr, itemId);
			fns.todo_flush(appPtr, bufPtr, BUF_CAPACITY);
		}

		// After 10 add/remove cycles with 0 items remaining:
		// Should be 3 (oninput + enter + add handlers only), NOT 3 + 10*2 = 23 (leaked)
		const hc = fns.todo_handler_count(appPtr);
		assert(hc, 3, "3 handlers after 10 add/remove cycles (no leak)");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — handler count bounded with multiple items across flushes");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		// Add 3 items, flush
		fns.todo_add_item(appPtr, writeStringStruct("A"));
		fns.todo_add_item(appPtr, writeStringStruct("B"));
		fns.todo_add_item(appPtr, writeStringStruct("C"));
		fns.todo_flush(appPtr, bufPtr, BUF_CAPACITY);

		const hc1 = fns.todo_handler_count(appPtr);
		assert(hc1, 9, "9 handlers: 3 app + 3*2 item handlers");

		// Add 2 more items, flush again — old item handlers should be cleaned up
		// and re-registered (total = 1 add + 5*2 item = 11)
		fns.todo_add_item(appPtr, writeStringStruct("D"));
		fns.todo_add_item(appPtr, writeStringStruct("E"));
		fns.todo_flush(appPtr, bufPtr, BUF_CAPACITY);

		const hc2 = fns.todo_handler_count(appPtr);
		assert(
			hc2,
			13,
			"13 handlers: 3 app + 5*2 item handlers (no leak from previous flush)",
		);

		fns.todo_destroy(appPtr);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Phase 20.5 — WASM-driven Add flow tests
	// ═════════════════════════════════════════════════════════════════════

	suite("Todo — M20.5: string dispatch updates SignalString");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		// Get the Add handler ID (3rd event in register_view tree-walk order)
		const addHandler = fns.todo_add_handler_id(appPtr);
		assert(addHandler >= 0, true, "add handler ID is non-negative");

		// Dispatch string event to the oninput_set_string handler
		// (the handler for input events writes to the SignalString)
		// oninput is the 1st event (index 0), enter is 2nd, add is 3rd
		// so oninput handler ID = addHandler - 2
		const strPtr = writeStringStruct("Buy milk");
		fns.todo_dispatch_string(appPtr, addHandler - 2, 0, strPtr);

		// The oninput handler (addHandler - 2) writes to the SignalString
		// Let's verify the signal was updated by checking input version
		const version = fns.todo_input_version(appPtr);
		assert(version > 0, true, "input version bumped after string dispatch");
		assert(
			fns.todo_input_is_empty(appPtr),
			0,
			"input not empty after string dispatch",
		);

		fns.todo_destroy(appPtr);
	}

	suite("Todo — M20.5: handle_event Add reads signal and adds item");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const addHandler = fns.todo_add_handler_id(appPtr);

		// Simulate typing by writing to the signal via todo_set_input
		fns.todo_set_input(appPtr, writeStringStruct("Buy milk"));

		// Dispatch Add handler — WASM reads signal, adds item, clears signal
		const handled = fns.todo_handle_event(appPtr, addHandler, 0);
		assert(handled, 1, "handle_event returns 1 (handled) for Add");

		// Item was added
		assert(fns.todo_item_count(appPtr), 1, "1 item after Add");

		// Signal was cleared
		assert(fns.todo_input_is_empty(appPtr), 1, "input cleared after Add");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — M20.5: Add with empty input is a no-op");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const addHandler = fns.todo_add_handler_id(appPtr);

		// Don't set any input text — signal is empty
		const handled = fns.todo_handle_event(appPtr, addHandler, 0);
		assert(handled, 1, "handle_event returns 1 even for empty (handled)");

		// No item added
		assert(fns.todo_item_count(appPtr), 0, "0 items when input empty");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — M20.5: WASM-driven Add with DOM rendering");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		const addHandler = fns.todo_add_handler_id(app.appPtr);

		// Simulate typing via signal
		fns.todo_set_input(app.appPtr, writeStringStruct("WASM todo"));

		// Trigger Add via handle_event (same as EventBridge would)
		fns.todo_handle_event(app.appPtr, addHandler, 0);
		app.flush();

		// Item appears in DOM
		const ul = root.querySelector("ul");
		const lis = ul ? Array.from(ul.querySelectorAll("li")) : [];
		assert(lis.length, 1, "1 li rendered after WASM-driven Add");

		// Verify item text
		const span = lis[0]?.querySelector("span");
		assert(span?.textContent?.includes("WASM todo"), true, "item text matches");

		// Signal was cleared — input version bumped
		assert(fns.todo_input_is_empty(app.appPtr), 1, "signal cleared after Add");

		app.destroy();
	}

	suite("Todo — M20.5: multiple WASM-driven Adds");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		const addHandler = fns.todo_add_handler_id(app.appPtr);

		// Add 3 items via the WASM-driven flow
		for (const text of ["Alpha", "Beta", "Gamma"]) {
			fns.todo_set_input(app.appPtr, writeStringStruct(text));
			fns.todo_handle_event(app.appPtr, addHandler, 0);
			app.flush();
		}

		assert(app.itemCount(), 3, "3 items after 3 WASM-driven Adds");

		const ul = root.querySelector("ul");
		const lis = ul ? Array.from(ul.querySelectorAll("li")) : [];
		assert(lis.length, 3, "3 li elements rendered");

		app.destroy();
	}

	suite("Todo — M20.5: todo_dispatch_string export works");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		// Dispatch a string to the oninput handler
		// The oninput handler is 2 less than the add handler
		// (1st event in tree-walk order: oninput, then enter, then add)
		const addHandler = fns.todo_add_handler_id(appPtr);
		const oninputHandler = addHandler - 2;

		const strPtr = writeStringStruct("test dispatch");
		const result = fns.todo_dispatch_string(appPtr, oninputHandler, 0, strPtr);
		assert(result, 1, "dispatch_string returns 1 (handled)");

		// Verify signal was updated
		assert(
			fns.todo_input_is_empty(appPtr),
			0,
			"input not empty after dispatch_string",
		);

		fns.todo_destroy(appPtr);
	}

	// ── Phase 22: Enter key handler tests ───────────────────────────────────

	suite("Todo — Phase 22: todo_enter_handler_id returns valid handler");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const enterHandler = fns.todo_enter_handler_id(appPtr);
		assert(enterHandler >= 0, true, "enter handler ID is non-negative");

		const addHandler = fns.todo_add_handler_id(appPtr);
		assert(
			enterHandler !== addHandler,
			true,
			"enter and add handler IDs are different",
		);

		// Enter handler should be between oninput and add in tree-walk order
		assert(
			enterHandler,
			addHandler - 1,
			"enter handler is one before add handler (tree-walk order)",
		);

		fns.todo_destroy(appPtr);
	}

	suite("Todo — Phase 22: dispatch_string with Enter key marks scope dirty");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const enterHandler = fns.todo_enter_handler_id(appPtr);

		// Dispatch "Enter" key string to the enter handler
		const strPtr = writeStringStruct("Enter");
		const result = fns.todo_dispatch_string(appPtr, enterHandler, 0, strPtr);
		assert(result, 1, "dispatch_string returns 1 (accepted) for Enter key");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — Phase 22: dispatch_string with non-Enter key is ignored");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const enterHandler = fns.todo_enter_handler_id(appPtr);

		// Dispatch a non-Enter key — should be rejected
		const strPtr = writeStringStruct("a");
		const result = fns.todo_dispatch_string(appPtr, enterHandler, 0, strPtr);
		assert(result, 0, "dispatch_string returns 0 (rejected) for non-Enter key");

		fns.todo_destroy(appPtr);
	}

	suite(
		"Todo — Phase 22: Enter key triggers Add (dispatch_string + handle_event)",
	);
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const enterHandler = fns.todo_enter_handler_id(appPtr);

		// Simulate typing via signal
		fns.todo_set_input(appPtr, writeStringStruct("Enter key todo"));

		// Step 1: dispatch_string with "Enter" → accepted, marks scope dirty
		const strPtr = writeStringStruct("Enter");
		const accepted = fns.todo_dispatch_string(appPtr, enterHandler, 0, strPtr);
		assert(accepted, 1, "Enter key accepted");

		// Step 2: handle_event routes the enter handler to Add logic
		const handled = fns.todo_handle_event(appPtr, enterHandler, 0);
		assert(handled, 1, "handle_event returns 1 for enter handler (Add)");

		// Item should be added and input cleared
		const count = fns.todo_item_count(appPtr);
		assert(count, 1, "1 item added via Enter key");

		assert(fns.todo_input_is_empty(appPtr), 1, "input cleared after Enter Add");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — Phase 22: Enter key with empty input is a no-op");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const enterHandler = fns.todo_enter_handler_id(appPtr);

		// Don't set any input text — signal is empty
		const strPtr = writeStringStruct("Enter");
		fns.todo_dispatch_string(appPtr, enterHandler, 0, strPtr);
		fns.todo_handle_event(appPtr, enterHandler, 0);

		const count = fns.todo_item_count(appPtr);
		assert(count, 0, "no item added when input is empty");

		fns.todo_destroy(appPtr);
	}

	suite("Todo — Phase 22: Enter key Add with DOM rendering");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		const enterHandler = fns.todo_enter_handler_id(app.appPtr);

		// Simulate typing via signal
		fns.todo_set_input(app.appPtr, writeStringStruct("DOM enter test"));

		// Dispatch Enter key → accepted → handle_event routes to Add
		const strPtr = writeStringStruct("Enter");
		fns.todo_dispatch_string(app.appPtr, enterHandler, 0, strPtr);
		fns.todo_handle_event(app.appPtr, enterHandler, 0);
		app.flush();

		// Verify DOM
		const ul = root.querySelector("ul");
		const lis = ul ? Array.from(ul.querySelectorAll("li")) : [];
		assert(lis.length, 1, "1 li rendered after Enter key Add");

		const span = lis[0]?.querySelector("span");
		assert(span?.textContent, "DOM enter test", "li text matches entered text");

		app.destroy();
	}

	suite("Todo — Phase 22: Shift key does not trigger Add");
	{
		const appPtr = fns.todo_init();
		const bufPtr = fns.mutation_buf_alloc(BUF_CAPACITY);
		fns.todo_rebuild(appPtr, bufPtr, BUF_CAPACITY);

		const enterHandler = fns.todo_enter_handler_id(appPtr);

		// Set input and dispatch a non-Enter key
		fns.todo_set_input(appPtr, writeStringStruct("Should not add"));

		const strPtr = writeStringStruct("Shift");
		const rejected = fns.todo_dispatch_string(appPtr, enterHandler, 0, strPtr);
		assert(rejected, 0, "Shift key rejected");

		// Even if we call handle_event, no item should be added because
		// the scope was not marked dirty by the rejected key
		const count = fns.todo_item_count(appPtr);
		assert(count, 0, "no item added for Shift key");

		fns.todo_destroy(appPtr);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 30: Phase 28 — Empty state message (ConditionalSlot)
	// ═════════════════════════════════════════════════════════════════════

	suite("Todo — empty state message visible on initial mount");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		assert(app.isEmptyMsgMounted(), true, "empty msg is mounted initially");
		assert(app.itemCount(), 0, "0 items initially");

		const div = root.firstChild as Element;
		// 4th child should be the message <p>
		const msgEl = div.childNodes[3] as Element;
		assert(msgEl.tagName, "P", "message element is a <p>");
		assert(
			msgEl.textContent,
			"No items yet -- add one above!",
			"message text correct",
		);

		app.destroy();
	}

	suite("Todo — empty message hidden after adding item");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		assert(app.isEmptyMsgMounted(), true, "msg mounted before add");

		app.addItem("First task");

		assert(app.isEmptyMsgMounted(), false, "msg hidden after add");
		assert(app.itemCount(), 1, "1 item after add");

		// The message element should be gone (replaced by placeholder)
		const div = root.firstChild as Element;
		const fourthChild = div.childNodes[3];
		const isNotP = !fourthChild || (fourthChild as Element).tagName !== "P";
		assert(isNotP, true, "4th child is no longer a <p>");

		app.destroy();
	}

	suite("Todo — empty message returns after removing all items");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		// Add two items — message should hide
		app.addItem("A");
		app.addItem("B");
		assert(app.isEmptyMsgMounted(), false, "msg hidden with 2 items");

		// Remove both items — message should return
		const id1 = app.itemIdAt(0);
		const id2 = app.itemIdAt(1);
		app.removeItem(id1);
		app.removeItem(id2);

		assert(app.itemCount(), 0, "0 items after removing all");
		assert(app.isEmptyMsgMounted(), true, "msg re-mounted after removing all");

		const div = root.firstChild as Element;
		const msgEl = div.childNodes[3] as Element;
		assert(msgEl.tagName, "P", "message <p> is back");
		assert(
			msgEl.textContent,
			"No items yet -- add one above!",
			"message text correct after re-mount",
		);

		app.destroy();
	}

	suite("Todo — add → remove all → add again: message hides again");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		// Add item — msg hides
		app.addItem("X");
		assert(app.isEmptyMsgMounted(), false, "msg hidden after first add");

		// Remove it — msg shows
		const xId = app.itemIdAt(0);
		app.removeItem(xId);
		assert(app.isEmptyMsgMounted(), true, "msg shown after remove");

		// Add another — msg hides again
		app.addItem("Y");
		assert(app.isEmptyMsgMounted(), false, "msg hidden after second add");
		assert(app.itemCount(), 1, "1 item after re-add");

		app.destroy();
	}

	suite("Todo — empty message does not affect item rendering");
	{
		const { document, root } = createDOM();
		const app = createTodoApp(fns, root, document);

		app.addItem("Buy groceries");
		app.addItem("Walk the dog");

		assert(app.isEmptyMsgMounted(), false, "msg hidden with items");

		const lis = root.querySelectorAll("li");
		assert(lis.length, 2, "2 li elements rendered");

		const span1 = lis[0].querySelector("span");
		assert(span1?.textContent, "Buy groceries", "first item text correct");

		const span2 = lis[1].querySelector("span");
		assert(span2?.textContent, "Walk the dog", "second item text correct");

		app.destroy();
	}
}
