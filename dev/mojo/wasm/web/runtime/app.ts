// App Shell — Orchestrates WASM runtime, DOM interpreter, and event bridge.
//
// Phase 26: destroy() now frees the mutation buffer, clears the root DOM,
// and sets a `destroyed` flag to guard against double-destroy.
//
// Provides a generic `createApp()` factory that wires together:
//
//   1. WASM app exports (init, rebuild, handle_event, flush, destroy)
//   2. TemplateCache (templates auto-registered from WASM via RegisterTemplate mutations)
//   3. Interpreter (applies binary mutation buffers to DOM)
//   4. EventBridge (delegates DOM events → WASM dispatch → flush → apply)
//
// The mutation buffer is allocated once in WASM linear memory and reused
// for every rebuild/flush cycle.
//
// Templates are automatically registered from WASM via RegisterTemplate
// mutations prepended to the mount buffer by AppShell.emit_templates().
// No manual DOM template construction is needed.
//
// App-specific factories (`createCounterApp`, etc.) are thin wrappers
// that pass the correct WASM export names and add app-specific helpers
// (e.g. `getCount`, `increment`, `addItem`).

import { EventBridge, EventType } from "./events.ts";
import { Interpreter } from "./interpreter.ts";
import { alignedAlloc, alignedFree, getMemory } from "./memory.ts";
import {
	allocStringStruct,
	readStringStruct,
	writeStringStruct,
} from "./strings.ts";
import { TemplateCache } from "./templates.ts";
import type { WasmExports } from "./types.ts";

// ── Constants ───────────────────────────────────────────────────────────────

/** Default mutation buffer size (16 KiB). */
const DEFAULT_BUF_CAPACITY = 16384;

// ── Generic App types ───────────────────────────────────────────────────────

/**
 * Configuration for the generic `createApp()` factory.
 *
 * Each field wraps a WASM export call so that `createApp` doesn't need
 * to know the specific export names (counter_init vs todo_init, etc.).
 */
export interface AppConfig {
	/** Instantiated WASM exports. */
	fns: WasmExports;

	/** The mount-point DOM element. */
	root: Element;

	/** The Document to use for DOM operations (for headless testing). */
	doc?: Document;

	/** Mutation buffer capacity in bytes (default: 16384). */
	bufCapacity?: number;

	/** Whether to install DOM event delegation (default: false). */
	install?: boolean;

	/** Initialize the WASM-side app.  Returns the app pointer. */
	init: (fns: WasmExports) => bigint;

	/** Initial render (mount).  Returns byte length of mutation data. */
	rebuild: (
		fns: WasmExports,
		appPtr: bigint,
		bufPtr: bigint,
		capacity: number,
	) => number;

	/** Flush pending updates.  Returns byte length of mutation data (0 = nothing dirty). */
	flush: (
		fns: WasmExports,
		appPtr: bigint,
		bufPtr: bigint,
		capacity: number,
	) => number;

	/** Dispatch an event to the WASM app.  Returns 1 if handled, 0 otherwise. */
	handleEvent: (
		fns: WasmExports,
		appPtr: bigint,
		handlerId: number,
		eventType: number,
	) => number;

	/**
	 * Dispatch an event with a String payload (Phase 20, M20.2).
	 *
	 * Optional — if provided, the EventBridge will call this for
	 * input/change events with the string value written to WASM memory
	 * via `writeStringStruct()`.  Handles ACTION_SIGNAL_SET_STRING
	 * handlers; falls back to normal dispatch for other action types.
	 *
	 * @returns 1 if handled, 0 otherwise.
	 */
	handleEventWithString?: (
		fns: WasmExports,
		appPtr: bigint,
		handlerId: number,
		eventType: number,
		stringPtr: bigint,
	) => number;

	/** Destroy the WASM-side app and free resources. */
	destroy: (fns: WasmExports, appPtr: bigint) => void;
}

/**
 * A fully wired app instance returned by `createApp()`.
 *
 * Provides the common lifecycle methods (dispatch, flush, destroy) and
 * exposes the underlying subsystems for inspection and extension.
 */
export interface AppHandle {
	/** The WASM exports object. */
	fns: WasmExports;

	/** The DOM interpreter. */
	interpreter: Interpreter;

	/** The event bridge. */
	events: EventBridge;

	/** The template cache. */
	templates: TemplateCache;

	/** The root DOM element. */
	root: Element;

	/** The WASM-side app pointer. */
	appPtr: bigint;

	/** Pointer to the mutation buffer in WASM memory. */
	bufPtr: bigint;

	/** Mutation buffer capacity in bytes. */
	bufCapacity: number;

	/** Dispatch an event by handler ID and flush + apply any mutations. */
	dispatchAndFlush(handlerId: number, eventType?: number): void;

	/** Flush pending updates and apply mutations to the DOM. */
	flushAndApply(): void;

	/** Whether the app has been destroyed. */
	destroyed: boolean;

	/** Destroy the app and free WASM resources. Idempotent. */
	destroy(): void;
}

// ── Generic App factory ─────────────────────────────────────────────────────

/**
 * Create a WASM app wired to a DOM root element.
 *
 * This is the generic, headless-compatible version (no browser required).
 * Pass a `Document` from linkedom/deno-dom for testing.
 *
 * Templates are automatically registered from WASM via RegisterTemplate
 * mutations — no manual DOM template construction needed.  Handler IDs
 * flow through the mutation protocol via the `handlerId` parameter on
 * `onNewListener` — no manual handler ordering needed.
 */
export function createApp(config: AppConfig): AppHandle {
	const {
		fns,
		root,
		init,
		rebuild,
		flush,
		handleEvent,
		handleEventWithString,
		destroy: destroyApp,
		install = false,
		bufCapacity = DEFAULT_BUF_CAPACITY,
	} = config;
	const document = config.doc ?? root.ownerDocument!;

	// 1. Initialize WASM-side app
	const appPtr = init(fns);

	// 2. Create empty template cache — templates come from WASM via
	//    RegisterTemplate mutations prepended to the mount buffer.
	const templates = new TemplateCache(document);

	// 3. Create interpreter
	const interpreter = new Interpreter(root, templates, document);

	// 4. Allocate a mutation buffer in WASM memory
	const bufPtr = alignedAlloc(8n, BigInt(bufCapacity));

	// 5. Create event bridge and wire onNewListener BEFORE mount so that
	//    NewEventListener mutations in the mount buffer are captured.
	const nodeMap: Map<number, Node> = new Map();
	const events = new EventBridge(root, nodeMap);

	// Handler IDs come from the mutation protocol — the Interpreter's
	// onNewListener callback receives (elementId, eventName, handlerId)
	// directly from the NewEventListener mutation.  No manual ordering needed.
	interpreter.onNewListener = (
		elementId: number,
		eventName: string,
		handlerId: number,
	): EventListener => {
		events.addHandler(elementId, eventName, handlerId);
		// Return a no-op listener (the EventBridge handles delegation)
		return () => {};
	};

	// Set up dispatch functions on the bridge
	events.setDispatch(
		(handlerId: number, eventType: number) => {
			return handleEvent(fns, appPtr, handlerId, eventType);
		},
		(handlerId: number, eventType: number, _value: number) => {
			return handleEvent(fns, appPtr, handlerId, eventType);
		},
		// Phase 20 (M20.2): Wire string dispatch if the app supports it
		handleEventWithString
			? (handlerId: number, eventType: number, stringPtr: bigint) => {
					return handleEventWithString(
						fns,
						appPtr,
						handlerId,
						eventType,
						stringPtr,
					);
				}
			: undefined,
	);

	// After-dispatch callback: flush and apply mutations
	events.onAfterDispatch = () => {
		flushAndApply();
	};

	// 6. Initial mount — rebuild (RegisterTemplate + LoadTemplate + events in one pass)
	const mountLen = rebuild(fns, appPtr, bufPtr, bufCapacity);
	if (mountLen > 0) {
		const mem = getMemory();
		interpreter.applyMutations(mem.buffer, Number(bufPtr), mountLen);
	}

	// 7. Optionally install DOM event delegation
	if (install) {
		events.install();
	}

	// ── Helpers ───────────────────────────────────────────────────────

	function flushAndApply(): void {
		const len = flush(fns, appPtr, bufPtr, bufCapacity);
		if (len > 0) {
			const mem = getMemory();
			interpreter.applyMutations(mem.buffer, Number(bufPtr), len);
		}
	}

	function dispatchAndFlush(
		handlerId: number,
		eventType: number = EventType.Click,
	): void {
		handleEvent(fns, appPtr, handlerId, eventType);
		flushAndApply();
	}

	// ── Public handle ─────────────────────────────────────────────────

	const handle: AppHandle = {
		fns,
		interpreter,
		events,
		templates,
		root,
		appPtr,
		bufPtr,
		bufCapacity,
		destroyed: false,
		dispatchAndFlush,
		flushAndApply,

		destroy(): void {
			if (handle.destroyed) return;
			handle.destroyed = true;

			// Remove DOM event delegation
			events.uninstall();

			// Free the mutation buffer in the JS-side allocator
			alignedFree(bufPtr);

			// Destroy the WASM-side app state
			destroyApp(fns, appPtr);

			// Clear rendered DOM (removes child elements and their listeners)
			root.replaceChildren();

			// Null out fields to prevent use-after-destroy
			handle.appPtr = 0n;
			handle.bufPtr = 0n;
		},
	};

	return handle;
}

// ── CounterApp handle ───────────────────────────────────────────────────────

/**
 * A fully wired counter app instance.
 *
 * Returned by `createCounterApp`.  Extends `AppHandle` with
 * counter-specific methods for programmatic interaction (useful in tests).
 */
export interface CounterAppHandle extends AppHandle {
	/** Increment handler ID (for programmatic dispatch). */
	incrHandler: number;

	/** Decrement handler ID (for programmatic dispatch). */
	decrHandler: number;

	/** Toggle detail handler ID (for programmatic dispatch). */
	toggleHandler: number;

	/** Read the current count value from WASM. */
	getCount(): number;

	/** Read the current doubled memo value from WASM. */
	getDoubled(): number;

	/** Read the show_detail signal value (true = detail visible). */
	getShowDetail(): boolean;

	/** Check whether the conditional detail slot is mounted in the DOM. */
	isDetailMounted(): boolean;

	/** Simulate an increment click (dispatch + flush + apply). */
	increment(): void;

	/** Simulate a decrement click (dispatch + flush + apply). */
	decrement(): void;

	/** Simulate a toggle-detail click (dispatch + flush + apply). */
	toggleDetail(): void;
}

// ── Counter App factory ─────────────────────────────────────────────────────

/**
 * Create a counter app wired to a DOM root element.
 *
 * Thin wrapper around `createApp()` that adds counter-specific helpers:
 * `getCount()`, `increment()`, `decrement()`, and handler ID accessors.
 *
 * This is the headless-compatible version (no browser required).
 * Pass a `Document` from linkedom/deno-dom for testing.
 *
 * @param fns     - Instantiated WASM exports.
 * @param root    - The mount-point DOM element.
 * @param doc     - The Document to use for DOM operations.
 * @param install - Whether to install DOM event delegation (default: false).
 */
export function createCounterApp(
	fns: WasmExports,
	root: Element,
	doc?: Document,
	install = false,
): CounterAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		install,
		init: (f) => f.counter_init(),
		rebuild: (f, app, buf, cap) => f.counter_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.counter_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.counter_handle_event(app, hid, evt),
		destroy: (f, app) => f.counter_destroy(app),
	});

	const incrHandler = fns.counter_incr_handler(handle.appPtr);
	const decrHandler = fns.counter_decr_handler(handle.appPtr);
	const toggleHandler = fns.counter_toggle_handler(handle.appPtr);

	const counterHandle: CounterAppHandle = {
		...handle,
		incrHandler,
		decrHandler,
		toggleHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getCount(): number {
			return fns.counter_count_value(handle.appPtr);
		},

		getDoubled(): number {
			return fns.counter_doubled_value(handle.appPtr);
		},

		getShowDetail(): boolean {
			return fns.counter_show_detail(handle.appPtr) !== 0;
		},

		isDetailMounted(): boolean {
			return fns.counter_cond_mounted(handle.appPtr) !== 0;
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},

		decrement(): void {
			handle.dispatchAndFlush(decrHandler);
		},

		toggleDetail(): void {
			handle.dispatchAndFlush(toggleHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return counterHandle;
}

// ── ChildCounterApp handle ──────────────────────────────────────────────────

/**
 * A fully wired child-counter app instance (Phase 29).
 *
 * Returned by `createChildCounterApp`.  Demonstrates component composition:
 * the parent owns buttons and a dyn_node slot; a ChildComponent renders
 * the display `<p>Count: N</p>` with its own scope.
 */
export interface ChildCounterAppHandle extends AppHandle {
	/** Increment handler ID (parent view_events[0]). */
	incrHandler: number;

	/** Decrement handler ID (parent view_events[1]). */
	decrHandler: number;

	/** Read the current count value from WASM. */
	getCount(): number;

	/** Simulate an increment click (dispatch + flush + apply). */
	increment(): void;

	/** Simulate a decrement click (dispatch + flush + apply). */
	decrement(): void;

	/** Return the child component's scope ID. */
	childScopeId: number;

	/** Return the child component's template ID. */
	childTmplId: number;

	/** Return the parent root scope ID. */
	parentScopeId: number;

	/** Return the parent template ID. */
	parentTmplId: number;

	/** Return the number of event bindings on the child. */
	childEventCount: number;

	/** Check whether the child has been rendered at least once. */
	childHasRendered(): boolean;

	/** Check whether the child is mounted in the DOM. */
	childIsMounted(): boolean;

	/** Return the total number of registered handlers. */
	handlerCount(): number;
}

// ── ChildCounter App factory ────────────────────────────────────────────────

/**
 * Create a child-counter app wired to a DOM root element (Phase 29).
 *
 * Thin wrapper around `createApp()` that adds child-counter-specific helpers.
 * Demonstrates component composition: parent with buttons + child display.
 *
 * @param fns     - Instantiated WASM exports.
 * @param root    - The mount-point DOM element.
 * @param doc     - The Document to use for DOM operations.
 * @param install - Whether to install DOM event delegation (default: false).
 */
export function createChildCounterApp(
	fns: WasmExports,
	root: Element,
	doc?: Document,
	install = false,
): ChildCounterAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		install,
		init: (f) => f.cc_init(),
		rebuild: (f, app, buf, cap) => f.cc_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.cc_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.cc_handle_event(app, hid, evt),
		destroy: (f, app) => f.cc_destroy(app),
	});

	const incrHandler = fns.cc_incr_handler(handle.appPtr);
	const decrHandler = fns.cc_decr_handler(handle.appPtr);

	const ccHandle: ChildCounterAppHandle = {
		...handle,
		incrHandler,
		decrHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get childScopeId(): number {
			return fns.cc_child_scope_id(handle.appPtr);
		},

		get childTmplId(): number {
			return fns.cc_child_tmpl_id(handle.appPtr);
		},

		get parentScopeId(): number {
			return fns.cc_parent_scope_id(handle.appPtr);
		},

		get parentTmplId(): number {
			return fns.cc_parent_tmpl_id(handle.appPtr);
		},

		get childEventCount(): number {
			return fns.cc_child_event_count(handle.appPtr);
		},

		getCount(): number {
			return fns.cc_count_value(handle.appPtr);
		},

		childHasRendered(): boolean {
			return fns.cc_child_has_rendered(handle.appPtr) !== 0;
		},

		childIsMounted(): boolean {
			return fns.cc_child_is_mounted(handle.appPtr) !== 0;
		},

		handlerCount(): number {
			return fns.cc_handler_count(handle.appPtr);
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},

		decrement(): void {
			handle.dispatchAndFlush(decrHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return ccHandle;
}

// ── MultiViewApp handle (Phase 30) ──────────────────────────────────────────

/**
 * A fully wired multi-view app instance with client-side routing.
 *
 * Returned by `createMultiViewApp`.  Extends `AppHandle` with
 * routing-specific methods for programmatic navigation and view queries.
 */
export interface MultiViewAppHandle extends AppHandle {
	/** Counter nav button handler ID. */
	navCounterHandler: number;

	/** Todo nav button handler ID. */
	navTodoHandler: number;

	/** Todo Add button handler ID. */
	todoAddHandler: number;

	/** Navigate to a URL path.  Returns true if route matched. */
	navigate(path: string): boolean;

	/** Get the currently active URL path. */
	getCurrentPath(): string;

	/** Get the currently active branch tag (0=counter, 1=todo, 255=none). */
	getCurrentBranch(): number;

	/** Get the number of registered routes. */
	getRouteCount(): number;

	/** Read the counter view's count value from WASM. */
	getCountValue(): number;

	/** Read the todo view's item count from WASM. */
	getTodoCount(): number;

	/** Check whether the router's conditional slot is mounted. */
	isCondMounted(): boolean;

	/** Check whether the router has a pending route change. */
	isRouterDirty(): boolean;

	/** Simulate a Counter nav click (dispatch + flush + apply). */
	navToCounter(): void;

	/** Simulate a Todo nav click (dispatch + flush + apply). */
	navToTodo(): void;

	/** Simulate a Todo Add click (dispatch + flush + apply). */
	addTodoItem(): void;
}

// ── MultiViewApp factory ────────────────────────────────────────────────────

/**
 * Create a multi-view app wired to a DOM root element (Phase 30).
 *
 * Thin wrapper around `createApp()` that adds routing-specific helpers:
 * `navigate()`, `getCurrentPath()`, `navToCounter()`, `navToTodo()`, etc.
 *
 * This is the headless-compatible version (no browser required).
 * Pass a `Document` from linkedom/deno-dom for testing.
 *
 * @param fns     - Instantiated WASM exports.
 * @param root    - The mount-point DOM element.
 * @param doc     - The Document to use for DOM operations.
 * @param install - Whether to install DOM event delegation (default: false).
 */
export function createMultiViewApp(
	fns: WasmExports,
	root: Element,
	doc?: Document,
	install = false,
): MultiViewAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		install,
		init: (f) => f.mv_init(),
		rebuild: (f, app, buf, cap) => f.mv_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.mv_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.mv_handle_event(app, hid, evt),
		destroy: (f, app) => f.mv_destroy(app),
	});

	const navCounterHandler = fns.mv_nav_counter_handler(handle.appPtr);
	const navTodoHandler = fns.mv_nav_todo_handler(handle.appPtr);
	const todoAddHandler = fns.mv_todo_add_handler(handle.appPtr);

	const mvHandle: MultiViewAppHandle = {
		...handle,
		navCounterHandler,
		navTodoHandler,
		todoAddHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		navigate(path: string): boolean {
			const strPtr = writeStringStruct(path);
			const result = fns.mv_navigate(handle.appPtr, strPtr);
			if (result) {
				handle.flushAndApply();
			}
			return result !== 0;
		},

		getCurrentPath(): string {
			const outPtr = allocStringStruct();
			fns.mv_current_path(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},

		getCurrentBranch(): number {
			return fns.mv_current_branch(handle.appPtr);
		},

		getRouteCount(): number {
			return fns.mv_route_count(handle.appPtr);
		},

		getCountValue(): number {
			return fns.mv_count_value(handle.appPtr);
		},

		getTodoCount(): number {
			return fns.mv_todo_count(handle.appPtr);
		},

		isCondMounted(): boolean {
			return fns.mv_cond_mounted(handle.appPtr) !== 0;
		},

		isRouterDirty(): boolean {
			return fns.mv_router_dirty(handle.appPtr) !== 0;
		},

		navToCounter(): void {
			handle.dispatchAndFlush(navCounterHandler);
		},

		navToTodo(): void {
			handle.dispatchAndFlush(navTodoHandler);
		},

		addTodoItem(): void {
			handle.dispatchAndFlush(todoAddHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return mvHandle;
}

// ── PropsCounterApp handle (Phase 31.3) ─────────────────────────────────────

/**
 * A fully wired props-counter app instance demonstrating self-rendering
 * child components with props via context.
 *
 * Parent: div > h1("Props Counter") + button("+1") + button("-1") + dyn_node[0]
 * Child (CounterDisplay): div > p(dyn_text) + button("Toggle hex")
 *
 * The parent provides the count signal via context (prop).
 * The child consumes it and also owns a local show_hex toggle.
 */
export interface PropsCounterAppHandle extends AppHandle {
	/** Increment handler ID (parent view_events[0]). */
	incrHandler: number;

	/** Decrement handler ID (parent view_events[1]). */
	decrHandler: number;

	/** Toggle hex handler ID (child event_bindings[0]). */
	toggleHandler: number;

	/** Read the current count value from WASM. */
	getCount(): number;

	/** Read the current show_hex flag (true/false). */
	getShowHex(): boolean;

	/** Simulate an increment click (dispatch + flush + apply). */
	increment(): void;

	/** Simulate a decrement click (dispatch + flush + apply). */
	decrement(): void;

	/** Simulate a toggle-hex click (dispatch + flush + apply). */
	toggleHex(): void;

	/** Return the child component's scope ID. */
	childScopeId: number;

	/** Return the child component's template ID. */
	childTmplId: number;

	/** Return the parent root scope ID. */
	parentScopeId: number;

	/** Return the parent template ID. */
	parentTmplId: number;

	/** Check whether the child is mounted in the DOM. */
	isChildMounted(): boolean;

	/** Check whether the child scope is dirty. */
	isChildDirty(): boolean;

	/** Check whether the parent scope is dirty. */
	isParentDirty(): boolean;

	/** Check whether any scope is dirty. */
	hasDirty(): boolean;

	/** Check whether the child has been rendered at least once. */
	childHasRendered(): boolean;

	/** Return the total number of registered handlers. */
	handlerCount(): number;

	/** Return the number of live scopes. */
	scopeCount(): number;
}

// ── PropsCounter App factory ────────────────────────────────────────────────

/**
 * Create a props-counter app wired to a DOM root element (Phase 31.3).
 *
 * Thin wrapper around `createApp()` that adds props-counter-specific helpers.
 * Demonstrates self-rendering child components with props via context DI.
 *
 * @param fns     - Instantiated WASM exports.
 * @param root    - The mount-point DOM element.
 * @param doc     - The Document to use for DOM operations.
 * @param install - Whether to install DOM event delegation (default: false).
 */
export function createPropsCounterApp(
	fns: WasmExports,
	root: Element,
	doc?: Document,
	install = false,
): PropsCounterAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		install,
		init: (f) => f.pc_init(),
		rebuild: (f, app, buf, cap) => f.pc_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.pc_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.pc_handle_event(app, hid, evt),
		destroy: (f, app) => f.pc_destroy(app),
	});

	const incrHandler = fns.pc_incr_handler(handle.appPtr);
	const decrHandler = fns.pc_decr_handler(handle.appPtr);
	const toggleHandler = fns.pc_toggle_handler(handle.appPtr);

	const pcHandle: PropsCounterAppHandle = {
		...handle,
		incrHandler,
		decrHandler,
		toggleHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get childScopeId(): number {
			return fns.pc_child_scope_id(handle.appPtr);
		},

		get childTmplId(): number {
			return fns.pc_child_tmpl_id(handle.appPtr);
		},

		get parentScopeId(): number {
			return fns.pc_parent_scope_id(handle.appPtr);
		},

		get parentTmplId(): number {
			return fns.pc_parent_tmpl_id(handle.appPtr);
		},

		getCount(): number {
			return fns.pc_count_value(handle.appPtr);
		},

		getShowHex(): boolean {
			return fns.pc_show_hex(handle.appPtr) !== 0;
		},

		isChildMounted(): boolean {
			return fns.pc_child_is_mounted(handle.appPtr) !== 0;
		},

		isChildDirty(): boolean {
			return fns.pc_child_is_dirty(handle.appPtr) !== 0;
		},

		isParentDirty(): boolean {
			return fns.pc_parent_is_dirty(handle.appPtr) !== 0;
		},

		hasDirty(): boolean {
			return fns.pc_has_dirty(handle.appPtr) !== 0;
		},

		childHasRendered(): boolean {
			return fns.pc_child_has_rendered(handle.appPtr) !== 0;
		},

		handlerCount(): number {
			return fns.pc_handler_count(handle.appPtr);
		},

		scopeCount(): number {
			return fns.pc_scope_count(handle.appPtr);
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},

		decrement(): void {
			handle.dispatchAndFlush(decrHandler);
		},

		toggleHex(): void {
			handle.dispatchAndFlush(toggleHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return pcHandle;
}

// ── ThemeCounterApp handle (Phase 31.4) ─────────────────────────────────────

/**
 * A fully wired theme-counter app instance demonstrating shared context
 * across multiple child components with upward communication.
 *
 * Parent: div > button("Toggle theme") + button("Increment") + dyn_node[0] + dyn_node[1]
 * CounterChild: div > p(dyn_text) + button("Reset")
 * SummaryChild: p(dyn_text, dyn_attr[0])
 *
 * Both children consume theme and count from parent context.
 * CounterChild has a Reset button that writes to a callback signal
 * consumed by the parent to reset the count.
 */
export interface ThemeCounterAppHandle extends AppHandle {
	/** Toggle-theme handler ID (parent view_events[0]). */
	toggleThemeHandler: number;

	/** Increment handler ID (parent view_events[1]). */
	incrementHandler: number;

	/** Reset handler ID (counter child event_bindings[0]). */
	resetHandler: number;

	/** Read the current count value from WASM. */
	getCountValue(): number;

	/** Read whether the theme is dark (true) or light (false). */
	isDarkTheme(): boolean;

	/** Read the on_reset callback signal value. */
	getOnResetValue(): number;

	/** Simulate a theme toggle click (dispatch + flush + apply). */
	toggleTheme(): void;

	/** Simulate an increment click (dispatch + flush + apply). */
	increment(): void;

	/** Simulate a reset click from the counter child (dispatch + flush + apply). */
	resetViaChild(): void;

	/** Return the parent scope ID. */
	parentScopeId: number;

	/** Return the counter child scope ID. */
	counterScopeId: number;

	/** Return the summary child scope ID. */
	summaryScopeId: number;

	/** Check whether the counter child is mounted. */
	isCounterMounted(): boolean;

	/** Check whether the summary child is mounted. */
	isSummaryMounted(): boolean;

	/** Check whether the counter child scope is dirty. */
	isCounterDirty(): boolean;

	/** Check whether the summary child scope is dirty. */
	isSummaryDirty(): boolean;

	/** Check whether any scope is dirty. */
	hasDirty(): boolean;

	/** Check whether the counter child has rendered. */
	counterHasRendered(): boolean;

	/** Check whether the summary child has rendered. */
	summaryHasRendered(): boolean;

	/** Return the total number of registered handlers. */
	handlerCount(): number;

	/** Return the number of live scopes. */
	scopeCount(): number;
}

// ── ThemeCounter App factory ────────────────────────────────────────────────

/**
 * Create a theme-counter app wired to a DOM root element (Phase 31.4).
 *
 * Thin wrapper around `createApp()` that adds theme-counter-specific helpers.
 * Demonstrates shared context across multiple children and upward communication
 * via callback signals.
 *
 * @param fns     - Instantiated WASM exports.
 * @param root    - The mount-point DOM element.
 * @param doc     - The Document to use for DOM operations.
 * @param install - Whether to install DOM event delegation (default: false).
 */
export function createThemeCounterApp(
	fns: WasmExports,
	root: Element,
	doc?: Document,
	install = false,
): ThemeCounterAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		install,
		init: (f) => f.tc_init(),
		rebuild: (f, app, buf, cap) => f.tc_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.tc_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.tc_handle_event(app, hid, evt),
		destroy: (f, app) => f.tc_destroy(app),
	});

	const toggleThemeHandler = fns.tc_toggle_theme_handler(handle.appPtr);
	const incrementHandler = fns.tc_increment_handler(handle.appPtr);
	const resetHandler = fns.tc_reset_handler(handle.appPtr);

	const tcHandle: ThemeCounterAppHandle = {
		...handle,
		toggleThemeHandler,
		incrementHandler,
		resetHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get parentScopeId(): number {
			return fns.tc_parent_scope_id(handle.appPtr);
		},

		get counterScopeId(): number {
			return fns.tc_counter_scope_id(handle.appPtr);
		},

		get summaryScopeId(): number {
			return fns.tc_summary_scope_id(handle.appPtr);
		},

		getCountValue(): number {
			return fns.tc_count_value(handle.appPtr);
		},

		isDarkTheme(): boolean {
			return fns.tc_theme_is_dark(handle.appPtr) !== 0;
		},

		getOnResetValue(): number {
			return fns.tc_on_reset_value(handle.appPtr);
		},

		isCounterMounted(): boolean {
			return fns.tc_counter_is_mounted(handle.appPtr) !== 0;
		},

		isSummaryMounted(): boolean {
			return fns.tc_summary_is_mounted(handle.appPtr) !== 0;
		},

		isCounterDirty(): boolean {
			return fns.tc_counter_is_dirty(handle.appPtr) !== 0;
		},

		isSummaryDirty(): boolean {
			return fns.tc_summary_is_dirty(handle.appPtr) !== 0;
		},

		hasDirty(): boolean {
			return fns.tc_has_dirty(handle.appPtr) !== 0;
		},

		counterHasRendered(): boolean {
			return fns.tc_counter_has_rendered(handle.appPtr) !== 0;
		},

		summaryHasRendered(): boolean {
			return fns.tc_summary_has_rendered(handle.appPtr) !== 0;
		},

		handlerCount(): number {
			return fns.tc_handler_count(handle.appPtr);
		},

		scopeCount(): number {
			return fns.tc_scope_count(handle.appPtr);
		},

		toggleTheme(): void {
			handle.dispatchAndFlush(toggleThemeHandler);
		},

		increment(): void {
			handle.dispatchAndFlush(incrementHandler);
		},

		resetViaChild(): void {
			handle.dispatchAndFlush(resetHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return tcHandle;
}

// ── SafeCounterApp (Phase 32 — Error Boundaries) ───────────────────────────

/**
 * SafeCounterApp handle — counter with error boundary, crash/retry lifecycle.
 *
 * Parent has an error boundary, count signal, Crash button, and two child
 * components (normal display + error fallback).  Crash triggers report_error()
 * → fallback shown.  Retry calls clear_error() → normal child restored.
 */
export interface SafeCounterAppHandle extends AppHandle {
	/** Handler IDs for programmatic dispatch. */
	incrHandler: number;
	crashHandler: number;
	retryHandler: number;

	/** Scope IDs. */
	readonly parentScopeId: number;
	readonly normalScopeId: number;
	readonly fallbackScopeId: number;

	/** Current count value. */
	getCount(): number;

	/** Whether the error boundary has captured an error. */
	hasError(): boolean;

	/** The captured error message (read from WASM String struct). */
	getErrorMessage(): string;

	/** Whether the normal child is mounted in the DOM. */
	isNormalMounted(): boolean;

	/** Whether the fallback child is mounted in the DOM. */
	isFallbackMounted(): boolean;

	/** Whether the normal child has rendered at least once. */
	normalHasRendered(): boolean;

	/** Whether the fallback child has rendered at least once. */
	fallbackHasRendered(): boolean;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Total number of registered handlers. */
	handlerCount(): number;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Dispatch increment + flush. */
	increment(): void;

	/** Dispatch crash + flush. */
	crash(): void;

	/** Dispatch retry + flush. */
	retry(): void;
}

/**
 * Create a SafeCounterApp wired to a DOM root element.
 */
export function createSafeCounterApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): SafeCounterAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.sc_init(),
		rebuild: (f, app, buf, cap) => f.sc_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.sc_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.sc_handle_event(app, hid, evt),
		destroy: (f, app) => f.sc_destroy(app),
	});

	const incrHandler = fns.sc_incr_handler(handle.appPtr) as number;
	const crashHandler = fns.sc_crash_handler(handle.appPtr) as number;
	const retryHandler = fns.sc_retry_handler(handle.appPtr) as number;

	const scHandle: SafeCounterAppHandle = {
		...handle,

		incrHandler,
		crashHandler,
		retryHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get parentScopeId(): number {
			return fns.sc_parent_scope_id(handle.appPtr) as number;
		},
		get normalScopeId(): number {
			return fns.sc_normal_scope_id(handle.appPtr) as number;
		},
		get fallbackScopeId(): number {
			return fns.sc_fallback_scope_id(handle.appPtr) as number;
		},

		getCount(): number {
			return fns.sc_count_value(handle.appPtr) as number;
		},
		hasError(): boolean {
			return (fns.sc_has_error(handle.appPtr) as number) !== 0;
		},
		getErrorMessage(): string {
			const outPtr = allocStringStruct();
			fns.sc_error_message(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isNormalMounted(): boolean {
			return (fns.sc_normal_mounted(handle.appPtr) as number) !== 0;
		},
		isFallbackMounted(): boolean {
			return (fns.sc_fallback_mounted(handle.appPtr) as number) !== 0;
		},
		normalHasRendered(): boolean {
			return (fns.sc_normal_has_rendered(handle.appPtr) as number) !== 0;
		},
		fallbackHasRendered(): boolean {
			return (fns.sc_fallback_has_rendered(handle.appPtr) as number) !== 0;
		},
		hasDirty(): boolean {
			return (fns.sc_has_dirty(handle.appPtr) as number) !== 0;
		},
		handlerCount(): number {
			return fns.sc_handler_count(handle.appPtr) as number;
		},
		scopeCount(): number {
			return fns.sc_scope_count(handle.appPtr) as number;
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},
		crash(): void {
			handle.dispatchAndFlush(crashHandler);
		},
		retry(): void {
			handle.dispatchAndFlush(retryHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return scHandle;
}

// ── ErrorNestApp (nested error boundaries) ──────────────────────────────────

export interface ErrorNestAppHandle extends AppHandle {
	/** Whether the outer boundary has captured an error. */
	hasOuterError(): boolean;

	/** Whether the inner boundary has captured an error. */
	hasInnerError(): boolean;

	/** The outer boundary's error message. */
	getOuterErrorMessage(): string;

	/** The inner boundary's error message. */
	getInnerErrorMessage(): string;

	/** Whether the outer normal child is mounted. */
	outerNormalMounted(): boolean;

	/** Whether the outer fallback child is mounted. */
	outerFallbackMounted(): boolean;

	/** Whether the inner normal child is mounted. */
	innerNormalMounted(): boolean;

	/** Whether the inner fallback child is mounted. */
	innerFallbackMounted(): boolean;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Total number of registered handlers. */
	handlerCount(): number;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Scope IDs. */
	readonly outerScopeId: number;
	readonly innerBoundaryScopeId: number;
	readonly innerNormalScopeId: number;
	readonly innerFallbackScopeId: number;
	readonly outerFallbackScopeId: number;

	/** Handler IDs. */
	outerCrashHandler: number;
	innerCrashHandler: number;
	outerRetryHandler: number;
	innerRetryHandler: number;

	/** Dispatch outer crash + flush. */
	outerCrash(): void;

	/** Dispatch inner crash + flush. */
	innerCrash(): void;

	/** Dispatch outer retry + flush. */
	outerRetry(): void;

	/** Dispatch inner retry + flush. */
	innerRetry(): void;
}

/**
 * Create an ErrorNestApp wired to a DOM root element.
 */
export function createErrorNestApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): ErrorNestAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.en_init(),
		rebuild: (f, app, buf, cap) => f.en_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.en_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.en_handle_event(app, hid, evt),
		destroy: (f, app) => f.en_destroy(app),
	});

	const outerCrashHandler = fns.en_outer_crash_handler(handle.appPtr) as number;
	const innerCrashHandler = fns.en_inner_crash_handler(handle.appPtr) as number;
	const outerRetryHandler = fns.en_outer_retry_handler(handle.appPtr) as number;
	const innerRetryHandler = fns.en_inner_retry_handler(handle.appPtr) as number;

	const enHandle: ErrorNestAppHandle = {
		...handle,

		outerCrashHandler,
		innerCrashHandler,
		outerRetryHandler,
		innerRetryHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get outerScopeId(): number {
			return fns.en_outer_scope_id(handle.appPtr) as number;
		},
		get innerBoundaryScopeId(): number {
			return fns.en_inner_boundary_scope_id(handle.appPtr) as number;
		},
		get innerNormalScopeId(): number {
			return fns.en_inner_normal_scope_id(handle.appPtr) as number;
		},
		get innerFallbackScopeId(): number {
			return fns.en_inner_fallback_scope_id(handle.appPtr) as number;
		},
		get outerFallbackScopeId(): number {
			return fns.en_outer_fallback_scope_id(handle.appPtr) as number;
		},

		hasOuterError(): boolean {
			return (fns.en_has_outer_error(handle.appPtr) as number) !== 0;
		},
		hasInnerError(): boolean {
			return (fns.en_has_inner_error(handle.appPtr) as number) !== 0;
		},
		getOuterErrorMessage(): string {
			const outPtr = allocStringStruct();
			fns.en_outer_error_message(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		getInnerErrorMessage(): string {
			const outPtr = allocStringStruct();
			fns.en_inner_error_message(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},

		outerNormalMounted(): boolean {
			return (fns.en_outer_normal_mounted(handle.appPtr) as number) !== 0;
		},
		outerFallbackMounted(): boolean {
			return (fns.en_outer_fallback_mounted(handle.appPtr) as number) !== 0;
		},
		innerNormalMounted(): boolean {
			return (fns.en_inner_normal_mounted(handle.appPtr) as number) !== 0;
		},
		innerFallbackMounted(): boolean {
			return (fns.en_inner_fallback_mounted(handle.appPtr) as number) !== 0;
		},

		hasDirty(): boolean {
			return (fns.en_has_dirty(handle.appPtr) as number) !== 0;
		},
		handlerCount(): number {
			return fns.en_handler_count(handle.appPtr) as number;
		},
		scopeCount(): number {
			return fns.en_scope_count(handle.appPtr) as number;
		},

		outerCrash(): void {
			handle.dispatchAndFlush(outerCrashHandler);
		},
		innerCrash(): void {
			handle.dispatchAndFlush(innerCrashHandler);
		},
		outerRetry(): void {
			handle.dispatchAndFlush(outerRetryHandler);
		},
		innerRetry(): void {
			handle.dispatchAndFlush(innerRetryHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return enHandle;
}

// ── DataLoaderApp (Phase 33.2 — Suspense demo) ─────────────────────────────

export interface DataLoaderAppHandle extends AppHandle {
	/** Whether the app is in pending (loading) state. */
	isPending(): boolean;

	/** The current data text string. */
	getDataText(): string;

	/** Whether the content child is mounted in the DOM. */
	isContentMounted(): boolean;

	/** Whether the skeleton child is mounted in the DOM. */
	isSkeletonMounted(): boolean;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Scope IDs. */
	readonly parentScopeId: number;
	readonly contentScopeId: number;
	readonly skeletonScopeId: number;

	/** Load button handler ID. */
	loadHandler: number;

	/** Dispatch load button + flush (enters pending state). */
	load(): void;

	/** Resolve pending state with data string + flush. */
	resolve(data: string): void;
}

/**
 * Create a DataLoaderApp wired to a DOM root element.
 */
export function createDataLoaderApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): DataLoaderAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.dl_init(),
		rebuild: (f, app, buf, cap) => f.dl_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.dl_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.dl_handle_event(app, hid, evt),
		destroy: (f, app) => f.dl_destroy(app),
	});

	const loadHandler = fns.dl_load_handler(handle.appPtr) as number;

	const dlHandle: DataLoaderAppHandle = {
		...handle,

		loadHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get parentScopeId(): number {
			return fns.dl_parent_scope_id(handle.appPtr) as number;
		},
		get contentScopeId(): number {
			return fns.dl_content_scope_id(handle.appPtr) as number;
		},
		get skeletonScopeId(): number {
			return fns.dl_skeleton_scope_id(handle.appPtr) as number;
		},

		isPending(): boolean {
			return (fns.dl_is_pending(handle.appPtr) as number) !== 0;
		},
		getDataText(): string {
			const outPtr = allocStringStruct();
			fns.dl_data_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isContentMounted(): boolean {
			return (fns.dl_content_mounted(handle.appPtr) as number) !== 0;
		},
		isSkeletonMounted(): boolean {
			return (fns.dl_skeleton_mounted(handle.appPtr) as number) !== 0;
		},
		hasDirty(): boolean {
			return (fns.dl_has_dirty(handle.appPtr) as number) !== 0;
		},
		scopeCount(): number {
			return fns.dl_scope_count(handle.appPtr) as number;
		},

		load(): void {
			handle.dispatchAndFlush(loadHandler);
		},
		resolve(data: string): void {
			const strPtr = writeStringStruct(data);
			fns.dl_resolve(handle.appPtr, strPtr);
			handle.flushAndApply();
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return dlHandle;
}

// ── SuspenseNestApp (Phase 33.3 — Nested suspense boundaries) ──────────────

export interface SuspenseNestAppHandle extends AppHandle {
	/** Whether the outer boundary is in pending state. */
	isOuterPending(): boolean;

	/** Whether the inner boundary is in pending state. */
	isInnerPending(): boolean;

	/** The current outer data text string. */
	getOuterData(): string;

	/** The current inner data text string. */
	getInnerData(): string;

	/** Whether the outer content child is mounted. */
	outerContentMounted(): boolean;

	/** Whether the outer skeleton child is mounted. */
	outerSkeletonMounted(): boolean;

	/** Whether the inner content child is mounted. */
	innerContentMounted(): boolean;

	/** Whether the inner skeleton child is mounted. */
	innerSkeletonMounted(): boolean;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Scope IDs. */
	readonly outerScopeId: number;
	readonly innerBoundaryScopeId: number;
	readonly innerContentScopeId: number;
	readonly innerSkeletonScopeId: number;
	readonly outerSkeletonScopeId: number;

	/** Handler IDs. */
	outerLoadHandler: number;
	innerLoadHandler: number;

	/** Dispatch outer load button + flush (enters outer pending state). */
	outerLoad(): void;

	/** Dispatch inner load button + flush (enters inner pending state). */
	innerLoad(): void;

	/** Resolve outer pending state with data string + flush. */
	outerResolve(data: string): void;

	/** Resolve inner pending state with data string + flush. */
	innerResolve(data: string): void;
}

/**
 * Create a SuspenseNestApp wired to a DOM root element.
 */
export function createSuspenseNestApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): SuspenseNestAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.sn_init(),
		rebuild: (f, app, buf, cap) => f.sn_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.sn_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.sn_handle_event(app, hid, evt),
		destroy: (f, app) => f.sn_destroy(app),
	});

	const outerLoadHandler = fns.sn_outer_load_handler(handle.appPtr) as number;
	const innerLoadHandler = fns.sn_inner_load_handler(handle.appPtr) as number;

	const snHandle: SuspenseNestAppHandle = {
		...handle,

		outerLoadHandler,
		innerLoadHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		get outerScopeId(): number {
			return fns.sn_outer_scope_id(handle.appPtr) as number;
		},
		get innerBoundaryScopeId(): number {
			return fns.sn_inner_boundary_scope_id(handle.appPtr) as number;
		},
		get innerContentScopeId(): number {
			return fns.sn_inner_content_scope_id(handle.appPtr) as number;
		},
		get innerSkeletonScopeId(): number {
			return fns.sn_inner_skeleton_scope_id(handle.appPtr) as number;
		},
		get outerSkeletonScopeId(): number {
			return fns.sn_outer_skeleton_scope_id(handle.appPtr) as number;
		},

		isOuterPending(): boolean {
			return (fns.sn_is_outer_pending(handle.appPtr) as number) !== 0;
		},
		isInnerPending(): boolean {
			return (fns.sn_is_inner_pending(handle.appPtr) as number) !== 0;
		},
		getOuterData(): string {
			const outPtr = allocStringStruct();
			fns.sn_outer_data(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		getInnerData(): string {
			const outPtr = allocStringStruct();
			fns.sn_inner_data(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},

		outerContentMounted(): boolean {
			return (fns.sn_outer_content_mounted(handle.appPtr) as number) !== 0;
		},
		outerSkeletonMounted(): boolean {
			return (fns.sn_outer_skeleton_mounted(handle.appPtr) as number) !== 0;
		},
		innerContentMounted(): boolean {
			return (fns.sn_inner_content_mounted(handle.appPtr) as number) !== 0;
		},
		innerSkeletonMounted(): boolean {
			return (fns.sn_inner_skeleton_mounted(handle.appPtr) as number) !== 0;
		},

		hasDirty(): boolean {
			return (fns.sn_has_dirty(handle.appPtr) as number) !== 0;
		},
		scopeCount(): number {
			return fns.sn_scope_count(handle.appPtr) as number;
		},

		outerLoad(): void {
			handle.dispatchAndFlush(outerLoadHandler);
		},
		innerLoad(): void {
			handle.dispatchAndFlush(innerLoadHandler);
		},
		outerResolve(data: string): void {
			const strPtr = writeStringStruct(data);
			fns.sn_outer_resolve(handle.appPtr, strPtr);
			handle.flushAndApply();
		},
		innerResolve(data: string): void {
			const strPtr = writeStringStruct(data);
			fns.sn_inner_resolve(handle.appPtr, strPtr);
			handle.flushAndApply();
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return snHandle;
}

// ── Phase 34.1 — EffectDemoApp ──────────────────────────────────────────────

export interface EffectDemoAppHandle extends AppHandle {
	/** Current count signal value. */
	getCount(): number;

	/** Current doubled signal value (derived by effect). */
	getDoubled(): number;

	/** Current parity text ("even" or "odd", derived by effect). */
	getParity(): string;

	/** Whether the count effect is pending. */
	isEffectPending(): boolean;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Increment button handler ID. */
	incrHandler: number;

	/** Dispatch increment button + flush. */
	increment(): void;
}

/**
 * Create an EffectDemoApp wired to a DOM root element.
 */
export function createEffectDemoApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): EffectDemoAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.ed_init(),
		rebuild: (f, app, buf, cap) => f.ed_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.ed_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.ed_handle_event(app, hid, evt),
		destroy: (f, app) => f.ed_destroy(app),
	});

	const incrHandler = fns.ed_incr_handler(handle.appPtr) as number;

	const edHandle: EffectDemoAppHandle = {
		...handle,

		incrHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getCount(): number {
			return fns.ed_count_value(handle.appPtr) as number;
		},
		getDoubled(): number {
			return fns.ed_doubled_value(handle.appPtr) as number;
		},
		getParity(): string {
			const outPtr = allocStringStruct();
			fns.ed_parity_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isEffectPending(): boolean {
			return (fns.ed_effect_is_pending(handle.appPtr) as number) !== 0;
		},
		hasDirty(): boolean {
			return (fns.ed_has_dirty(handle.appPtr) as number) !== 0;
		},
		scopeCount(): number {
			return fns.ed_scope_count(handle.appPtr) as number;
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return edHandle;
}

// ── Phase 34.2 — EffectMemoApp ──────────────────────────────────────────────

export interface EffectMemoAppHandle extends AppHandle {
	/** Current input signal value. */
	getInput(): number;

	/** Current tripled memo value (input * 3). */
	getTripled(): number;

	/** Current label text ("small" or "big", derived by effect from tripled). */
	getLabel(): string;

	/** Whether the label effect is pending. */
	isEffectPending(): boolean;

	/** Raw memo output value (same as tripled). */
	getMemoValue(): number;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Increment button handler ID. */
	incrHandler: number;

	/** Dispatch increment button + flush. */
	increment(): void;
}

/**
 * Create an EffectMemoApp wired to a DOM root element.
 */
export function createEffectMemoApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): EffectMemoAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.em_init(),
		rebuild: (f, app, buf, cap) => f.em_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.em_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.em_handle_event(app, hid, evt),
		destroy: (f, app) => f.em_destroy(app),
	});

	const incrHandler = fns.em_incr_handler(handle.appPtr) as number;

	const emHandle: EffectMemoAppHandle = {
		...handle,

		incrHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getInput(): number {
			return fns.em_input_value(handle.appPtr) as number;
		},
		getTripled(): number {
			return fns.em_tripled_value(handle.appPtr) as number;
		},
		getLabel(): string {
			const outPtr = allocStringStruct();
			fns.em_label_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isEffectPending(): boolean {
			return (fns.em_effect_is_pending(handle.appPtr) as number) !== 0;
		},
		getMemoValue(): number {
			return fns.em_memo_value(handle.appPtr) as number;
		},
		hasDirty(): boolean {
			return (fns.em_has_dirty(handle.appPtr) as number) !== 0;
		},
		scopeCount(): number {
			return fns.em_scope_count(handle.appPtr) as number;
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return emHandle;
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 35.2 — MemoFormApp (MemoBool + MemoString in a form)
// ══════════════════════════════════════════════════════════════════════════════

export interface MemoFormAppHandle extends AppHandle {
	/** Current input signal text. */
	getInput(): string;

	/** Whether the is_valid memo is True (input non-empty). */
	isValid(): boolean;

	/** Current status memo text ("✓ Valid: ..." or "✗ Empty"). */
	getStatus(): string;

	/** Whether the is_valid memo needs recomputation. */
	isValidDirty(): boolean;

	/** Whether the status memo needs recomputation. */
	isStatusDirty(): boolean;

	/** Set the input signal directly (test helper). */
	setInput(value: string): void;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live memos. */
	getMemoCount(): number;

	/** Number of live scopes. */
	scopeCount(): number;

	/** The oninput_set_string handler ID. */
	inputHandler: number;
}

/**
 * Create a MemoFormApp wired to a DOM root element.
 */
export function createMemoFormApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): MemoFormAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.mf_init(),
		rebuild: (f, app, buf, cap) => f.mf_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.mf_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.mf_handle_event(app, hid, evt),
		handleEventWithString: (f, app, hid, evt, strPtr) =>
			f.mf_handle_event_string(app, hid, evt, strPtr),
		destroy: (f, app) => f.mf_destroy(app),
	});

	const inputHandler = fns.mf_input_handler(handle.appPtr) as number;

	const mfHandle: MemoFormAppHandle = {
		...handle,

		inputHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getInput(): string {
			const outPtr = allocStringStruct();
			fns.mf_input_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isValid(): boolean {
			return (fns.mf_is_valid(handle.appPtr) as number) !== 0;
		},
		getStatus(): string {
			const outPtr = allocStringStruct();
			fns.mf_status_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isValidDirty(): boolean {
			return (fns.mf_is_valid_dirty(handle.appPtr) as number) !== 0;
		},
		isStatusDirty(): boolean {
			return (fns.mf_status_dirty(handle.appPtr) as number) !== 0;
		},
		setInput(value: string): void {
			const strPtr = writeStringStruct(value);
			fns.mf_set_input(handle.appPtr, strPtr);
			handle.flushAndApply();
		},
		hasDirty(): boolean {
			return (fns.mf_has_dirty(handle.appPtr) as number) !== 0;
		},
		getMemoCount(): number {
			return fns.mf_memo_count(handle.appPtr) as number;
		},
		scopeCount(): number {
			return fns.mf_scope_count(handle.appPtr) as number;
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return mfHandle;
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 35.3 — MemoChainApp (mixed-type memo chain)
// ══════════════════════════════════════════════════════════════════════════════

export interface MemoChainAppHandle extends AppHandle {
	/** Current input signal value. */
	getInput(): number;

	/** Current doubled memo value (input * 2). */
	getDoubled(): number;

	/** Whether the is_big memo is True (doubled >= 10). */
	isBig(): boolean;

	/** Current label memo text ("small" or "BIG"). */
	getLabel(): string;

	/** Whether the doubled memo needs recomputation. */
	isDoubledDirty(): boolean;

	/** Whether the is_big memo needs recomputation. */
	isBigDirty(): boolean;

	/** Whether the label memo needs recomputation. */
	isLabelDirty(): boolean;

	/** Dispatch increment button + flush. */
	increment(): void;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live memos. */
	getMemoCount(): number;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Increment button handler ID. */
	incrHandler: number;
}

/**
 * Create a MemoChainApp wired to a DOM root element.
 */
export function createMemoChainApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): MemoChainAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.mc_init(),
		rebuild: (f, app, buf, cap) => f.mc_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.mc_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.mc_handle_event(app, hid, evt),
		destroy: (f, app) => f.mc_destroy(app),
	});

	const incrHandler = fns.mc_incr_handler(handle.appPtr) as number;

	const mcHandle: MemoChainAppHandle = {
		...handle,

		incrHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getInput(): number {
			return fns.mc_input_value(handle.appPtr) as number;
		},
		getDoubled(): number {
			return fns.mc_doubled_value(handle.appPtr) as number;
		},
		isBig(): boolean {
			return (fns.mc_is_big(handle.appPtr) as number) !== 0;
		},
		getLabel(): string {
			const outPtr = allocStringStruct();
			fns.mc_label_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isDoubledDirty(): boolean {
			return (fns.mc_doubled_dirty(handle.appPtr) as number) !== 0;
		},
		isBigDirty(): boolean {
			return (fns.mc_is_big_dirty(handle.appPtr) as number) !== 0;
		},
		isLabelDirty(): boolean {
			return (fns.mc_label_dirty(handle.appPtr) as number) !== 0;
		},
		hasDirty(): boolean {
			return (fns.mc_has_dirty(handle.appPtr) as number) !== 0;
		},
		getMemoCount(): number {
			return fns.mc_memo_count(handle.appPtr) as number;
		},
		scopeCount(): number {
			return fns.mc_scope_count(handle.appPtr) as number;
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return mcHandle;
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 37.7 — EqualityDemoApp (equality-gated memo chain)
// ══════════════════════════════════════════════════════════════════════════════

export interface EqualityDemoAppHandle extends AppHandle {
	/** Current input signal value. */
	getInput(): number;

	/** Current clamped memo value (clamp(input, 0, 10)). */
	getClamped(): number;

	/** Current label memo text ("low" or "high"). */
	getLabel(): string;

	/** Whether the clamped memo needs recomputation. */
	isClampedDirty(): boolean;

	/** Whether the label memo needs recomputation. */
	isLabelDirty(): boolean;

	/** Whether the clamped memo's last recompute changed its value. */
	clampedChanged(): boolean;

	/** Whether the label memo's last recompute changed its value. */
	labelChanged(): boolean;

	/** Increment button handler ID. */
	incrHandler: number;

	/** Decrement button handler ID. */
	decrHandler: number;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Number of live memos. */
	memoCount(): number;

	/** Dispatch increment and flush. */
	increment(): void;

	/** Dispatch decrement and flush. */
	decrement(): void;
}

export function createEqualityDemoApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): EqualityDemoAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.eq_init(),
		rebuild: (f, app, buf, cap) => f.eq_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.eq_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.eq_handle_event(app, hid, evt),
		destroy: (f, app) => f.eq_destroy(app),
	});

	const incrHandler = fns.eq_incr_handler(handle.appPtr) as number;
	const decrHandler = fns.eq_decr_handler(handle.appPtr) as number;

	const eqHandle: EqualityDemoAppHandle = {
		...handle,

		incrHandler,
		decrHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getInput(): number {
			return fns.eq_input_value(handle.appPtr) as number;
		},
		getClamped(): number {
			return fns.eq_clamped_value(handle.appPtr) as number;
		},
		getLabel(): string {
			const outPtr = allocStringStruct();
			fns.eq_label_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isClampedDirty(): boolean {
			return (fns.eq_clamped_dirty(handle.appPtr) as number) !== 0;
		},
		isLabelDirty(): boolean {
			return (fns.eq_label_dirty(handle.appPtr) as number) !== 0;
		},
		clampedChanged(): boolean {
			return (fns.eq_clamped_changed(handle.appPtr) as number) !== 0;
		},
		labelChanged(): boolean {
			return (fns.eq_label_changed(handle.appPtr) as number) !== 0;
		},
		hasDirty(): boolean {
			return (fns.eq_has_dirty(handle.appPtr) as number) !== 0;
		},
		scopeCount(): number {
			return fns.eq_scope_count(handle.appPtr) as number;
		},
		memoCount(): number {
			return fns.eq_memo_count(handle.appPtr) as number;
		},

		increment(): void {
			handle.dispatchAndFlush(incrHandler);
		},
		decrement(): void {
			handle.dispatchAndFlush(decrHandler);
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return eqHandle;
}

// ── Phase 38.2 — BatchDemoApp ───────────────────────────────────────────────

export interface BatchDemoAppHandle extends AppHandle {
	/** Current full_name memo text. */
	getFullName(): string;

	/** Current write_count signal value. */
	getWriteCount(): number;

	/** Current first_name signal text. */
	getFirstName(): string;

	/** Current last_name signal text. */
	getLastName(): string;

	/** Whether the full_name memo needs recomputation. */
	isFullNameDirty(): boolean;

	/** Whether the full_name memo's last recompute changed its value. */
	fullNameChanged(): boolean;

	/** Whether any scope is dirty. */
	hasDirty(): boolean;

	/** Whether the runtime is in batch mode. */
	isBatching(): boolean;

	/** Number of live scopes. */
	scopeCount(): number;

	/** Number of live memos. */
	memoCount(): number;

	/** Set-names button handler ID. */
	setHandler: number;

	/** Reset button handler ID. */
	resetHandler: number;

	/** Set both names in a batch and flush. */
	setNames(first: string, last: string): void;

	/** Reset all state in a batch and flush. */
	reset(): void;
}

export function createBatchDemoApp(
	fns: WasmExports & Record<string, CallableFunction>,
	root: Element,
	doc?: Document,
): BatchDemoAppHandle {
	const handle = createApp({
		fns,
		root,
		doc,
		init: (f) => f.bd_init(),
		rebuild: (f, app, buf, cap) => f.bd_rebuild(app, buf, cap),
		flush: (f, app, buf, cap) => f.bd_flush(app, buf, cap),
		handleEvent: (f, app, hid, evt) => f.bd_handle_event(app, hid, evt),
		destroy: (f, app) => f.bd_destroy(app),
	});

	const setHandler = fns.bd_set_handler(handle.appPtr) as number;
	const resetHandler = fns.bd_reset_handler(handle.appPtr) as number;

	const bdHandle: BatchDemoAppHandle = {
		...handle,

		setHandler,
		resetHandler,

		get destroyed(): boolean {
			return handle.destroyed;
		},
		set destroyed(v: boolean) {
			handle.destroyed = v;
		},

		get appPtr(): bigint {
			return handle.appPtr;
		},
		set appPtr(v: bigint) {
			handle.appPtr = v;
		},

		get bufPtr(): bigint {
			return handle.bufPtr;
		},
		set bufPtr(v: bigint) {
			handle.bufPtr = v;
		},

		getFullName(): string {
			const outPtr = allocStringStruct();
			fns.bd_full_name_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		getWriteCount(): number {
			return fns.bd_write_count(handle.appPtr) as number;
		},
		getFirstName(): string {
			const outPtr = allocStringStruct();
			fns.bd_first_name_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		getLastName(): string {
			const outPtr = allocStringStruct();
			fns.bd_last_name_text(handle.appPtr, outPtr);
			return readStringStruct(outPtr);
		},
		isFullNameDirty(): boolean {
			return (fns.bd_full_name_dirty(handle.appPtr) as number) !== 0;
		},
		fullNameChanged(): boolean {
			return (fns.bd_full_name_changed(handle.appPtr) as number) !== 0;
		},
		hasDirty(): boolean {
			return (fns.bd_has_dirty(handle.appPtr) as number) !== 0;
		},
		isBatching(): boolean {
			return (fns.bd_is_batching(handle.appPtr) as number) !== 0;
		},
		scopeCount(): number {
			return fns.bd_scope_count(handle.appPtr) as number;
		},
		memoCount(): number {
			return fns.bd_memo_count(handle.appPtr) as number;
		},

		setNames(first: string, last: string): void {
			const firstPtr = writeStringStruct(first);
			const lastPtr = writeStringStruct(last);
			fns.bd_set_names(handle.appPtr, firstPtr, lastPtr);
			handle.flushAndApply();
		},
		reset(): void {
			fns.bd_reset(handle.appPtr);
			handle.flushAndApply();
		},

		destroy(): void {
			handle.destroy();
		},
	};

	return bdHandle;
}
