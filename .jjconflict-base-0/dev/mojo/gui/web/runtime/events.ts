// EventBridge — DOM event delegation to WASM event handlers.
//
// The EventBridge installs a single set of event listeners on the root
// element (event delegation pattern).  When a DOM event fires, the bridge:
//
//   1. Walks up from the event target to find the nearest element with
//      a `data-eid` attribute (the ElementId assigned during rendering).
//   2. Looks up the handler ID for that element + event type combination
//      in its local handler map.
//   3. Calls the WASM `dispatch_event` export, which executes the
//      handler's action (e.g. signal write) and marks scopes dirty.
//
// The bridge maintains a mapping of (elementId, eventName) → handlerId
// that is populated when the Interpreter processes NewEventListener
// mutations and cleared when RemoveEventListener mutations arrive.
//
// For input/change events (Phase 20, M20.2), the bridge extracts the
// string value from `event.target.value` and dispatches via the
// `dispatchWithStringFn` callback (→ WASM `dispatch_event_with_string`).
// This enables Dioxus-style `oninput → SignalString` two-way binding.
// If the string dispatch is not handled (returns 0), the bridge falls
// back to the numeric dispatch path and then the default no-payload path.
//
// Event type tags (must match Mojo's EVT_* constants):

import { scratchFreeAll } from "./memory.ts";
import { writeStringStruct } from "./strings.ts";

export const EventType = {
	Click: 0,
	Input: 1,
	KeyDown: 2,
	KeyUp: 3,
	MouseMove: 4,
	Focus: 5,
	Blur: 6,
	Submit: 7,
	Change: 8,
	MouseDown: 9,
	MouseUp: 10,
	MouseEnter: 11,
	MouseLeave: 12,
	Custom: 255,
} as const;

export type EventTypeName = keyof typeof EventType;

/** Map from DOM event name string to our EventType numeric tag. */
const EVENT_NAME_TO_TYPE: Record<string, number> = {
	click: EventType.Click,
	input: EventType.Input,
	keydown: EventType.KeyDown,
	keyup: EventType.KeyUp,
	mousemove: EventType.MouseMove,
	focus: EventType.Focus,
	blur: EventType.Blur,
	submit: EventType.Submit,
	change: EventType.Change,
	mousedown: EventType.MouseDown,
	mouseup: EventType.MouseUp,
	mouseenter: EventType.MouseEnter,
	mouseleave: EventType.MouseLeave,
};

/** The set of event names we delegate from the root element. */
const DELEGATED_EVENTS = [
	"click",
	"input",
	"keydown",
	"keyup",
	"mousemove",
	"focus",
	"blur",
	"submit",
	"change",
	"mousedown",
	"mouseup",
	"mouseenter",
	"mouseleave",
] as const;

/**
 * Dispatch function signature — matches the WASM export.
 *
 * @param handlerId - The handler ID to invoke.
 * @param eventType - The EventType numeric tag.
 * @returns 1 if an action was executed, 0 otherwise.
 */
export type DispatchFn = (handlerId: number, eventType: number) => number;

/**
 * Dispatch-with-value function signature — matches the WASM export.
 *
 * @param handlerId - The handler ID to invoke.
 * @param eventType - The EventType numeric tag.
 * @param value     - An Int32 payload (e.g. parsed input value).
 * @returns 1 if an action was executed, 0 otherwise.
 */
export type DispatchWithValueFn = (
	handlerId: number,
	eventType: number,
	value: number,
) => number;

/**
 * Dispatch-with-string function signature — matches the WASM export
 * `dispatch_event_with_string`.
 *
 * Phase 20 (M20.2): The string value is written to WASM linear memory
 * as a Mojo String struct (via `writeStringStruct()`) before calling
 * this function.  The `stringPtr` is a pointer to the 24-byte struct.
 *
 * @param handlerId - The handler ID to invoke.
 * @param eventType - The EventType numeric tag.
 * @param stringPtr - Pointer to a Mojo String struct in WASM memory.
 * @returns 1 if an action was executed, 0 otherwise.
 */
export type DispatchWithStringFn = (
	handlerId: number,
	eventType: number,
	stringPtr: bigint,
) => number;

/** Composite key for the handler map: "elementId:eventName" */
function handlerKey(elementId: number, eventName: string): string {
	return `${elementId}:${eventName}`;
}

/**
 * EventBridge — Delegates DOM events to WASM handlers.
 *
 * Usage:
 *   const bridge = new EventBridge(rootElement);
 *   bridge.setDispatch(dispatchFn, dispatchWithValueFn, dispatchWithStringFn);
 *   bridge.install();
 *
 *   // When Interpreter processes NewEventListener:
 *   bridge.addHandler(elementId, "click", handlerId);
 *
 *   // When Interpreter processes RemoveEventListener:
 *   bridge.removeHandler(elementId, "click");
 *
 *   // After dispatch, check for dirty scopes and flush:
 *   bridge.onAfterDispatch = () => { flushUpdates(); };
 */
export class EventBridge {
	/** Root element for event delegation. */
	private root: Element;

	/** Map from "elementId:eventName" → handlerId. */
	private handlerMap: Map<string, number> = new Map();

	/** Map from elementId → DOM Element (populated by Interpreter). */
	private nodes: Map<number, Node>;

	/** WASM dispatch function (set after WASM init). */
	private dispatchFn: DispatchFn | null = null;

	/** WASM dispatch-with-value function (set after WASM init). */
	private dispatchWithValueFn: DispatchWithValueFn | null = null;

	/** WASM dispatch-with-string function (set after WASM init). Phase 20. */
	private dispatchWithStringFn: DispatchWithStringFn | null = null;

	/** Installed AbortController for cleanup. */
	private abortController: AbortController | null = null;

	/**
	 * Callback invoked after every successful dispatch.
	 * The host should use this to flush dirty scopes and apply mutations.
	 */
	onAfterDispatch: (() => void) | null = null;

	constructor(root: Element, nodes: Map<number, Node>) {
		this.root = root;
		this.nodes = nodes;
	}

	// ── Configuration ────────────────────────────────────────────────

	/**
	 * Set the WASM dispatch functions.
	 * Must be called before events can be dispatched.
	 *
	 * @param dispatch           - Default dispatch (no payload).
	 * @param dispatchWithValue  - Dispatch with Int32 payload (numeric inputs).
	 * @param dispatchWithString - Dispatch with String payload (Phase 20).
	 */
	setDispatch(
		dispatch: DispatchFn,
		dispatchWithValue?: DispatchWithValueFn,
		dispatchWithString?: DispatchWithStringFn,
	): void {
		this.dispatchFn = dispatch;
		this.dispatchWithValueFn = dispatchWithValue ?? null;
		this.dispatchWithStringFn = dispatchWithString ?? null;
	}

	// ── Handler management ───────────────────────────────────────────

	/**
	 * Register a handler mapping: when `eventName` fires on the element
	 * with `elementId`, dispatch to `handlerId` on the WASM side.
	 */
	addHandler(elementId: number, eventName: string, handlerId: number): void {
		this.handlerMap.set(handlerKey(elementId, eventName), handlerId);
	}

	/**
	 * Remove a handler mapping for the given element and event name.
	 */
	removeHandler(elementId: number, eventName: string): void {
		this.handlerMap.delete(handlerKey(elementId, eventName));
	}

	/**
	 * Remove all handler mappings for the given element.
	 */
	removeAllHandlers(elementId: number): void {
		const prefix = `${elementId}:`;
		for (const key of this.handlerMap.keys()) {
			if (key.startsWith(prefix)) {
				this.handlerMap.delete(key);
			}
		}
	}

	/**
	 * Look up the handler ID for a given element + event name.
	 * Returns undefined if no handler is registered.
	 */
	getHandlerId(elementId: number, eventName: string): number | undefined {
		return this.handlerMap.get(handlerKey(elementId, eventName));
	}

	/**
	 * Return the number of registered handler mappings.
	 */
	get handlerCount(): number {
		return this.handlerMap.size;
	}

	/**
	 * Clear all handler mappings.
	 */
	clear(): void {
		this.handlerMap.clear();
	}

	// ── Event delegation ─────────────────────────────────────────────

	/**
	 * Install delegated event listeners on the root element.
	 * Call this once after the root is attached to the DOM.
	 */
	install(): void {
		if (this.abortController) {
			// Already installed — tear down first
			this.uninstall();
		}

		this.abortController = new AbortController();
		const signal = this.abortController.signal;

		for (const eventName of DELEGATED_EVENTS) {
			this.root.addEventListener(
				eventName,
				(e: Event) => this.handleEvent(e, eventName),
				{ capture: true, signal },
			);
		}
	}

	/**
	 * Remove all delegated event listeners.
	 */
	uninstall(): void {
		if (this.abortController) {
			this.abortController.abort();
			this.abortController = null;
		}
	}

	/**
	 * Core event handler: find the element ID, look up the handler,
	 * and dispatch to WASM.
	 *
	 * For input/change events (Phase 20, M20.2):
	 *   1. Try string dispatch first (if `dispatchWithStringFn` is set).
	 *      Extracts `event.target.value` as a string, writes it to WASM
	 *      memory via `writeStringStruct()`, and calls the string dispatch.
	 *      If handled (returns 1), done.
	 *   2. Fall back to numeric dispatch (if `dispatchWithValueFn` is set
	 *      and the value parses as an integer).
	 *   3. Fall back to default no-payload dispatch.
	 */
	private handleEvent(e: Event, eventName: string): void {
		if (!this.dispatchFn) return;

		const elementId = this.findElementId(e.target);
		if (elementId === null) return;

		const hid = this.handlerMap.get(handlerKey(elementId, eventName));
		if (hid === undefined) return;

		const eventType = EVENT_NAME_TO_TYPE[eventName];
		if (eventType === undefined) return;

		// For input/change events, extract the value and try dispatch paths
		if (eventName === "input" || eventName === "change") {
			const target = e.target as
				| HTMLInputElement
				| HTMLTextAreaElement
				| HTMLSelectElement;
			if (target && "value" in target) {
				const strValue = String(target.value);

				// Phase 20 (M20.2): Try string dispatch first.
				// This handles ACTION_SIGNAL_SET_STRING handlers directly.
				// For other action types, the WASM side falls back to
				// dispatch_event internally (without the string payload).
				if (this.dispatchWithStringFn) {
					const stringPtr = writeStringStruct(strValue);
					const handled = this.dispatchWithStringFn(hid, eventType, stringPtr);
					scratchFreeAll();
					if (handled) {
						this.onAfterDispatch?.();
						return;
					}
				}

				// Numeric fallback: for ACTION_SIGNAL_SET_INPUT handlers
				// that expect an Int32 payload.
				if (this.dispatchWithValueFn) {
					const numValue = parseInt(strValue, 10);
					if (!Number.isNaN(numValue)) {
						this.dispatchWithValueFn(hid, eventType, numValue);
						this.onAfterDispatch?.();
						return;
					}
				}
			}
		}

		// Default dispatch (no payload)
		this.dispatchFn(hid, eventType);
		this.onAfterDispatch?.();
	}

	/**
	 * Walk up from the event target to find the nearest element with
	 * a known ElementId in our node map.
	 *
	 * The Interpreter stores DOM nodes by ElementId.  We reverse-lookup
	 * by walking up the DOM tree and checking if each node is in the map.
	 */
	private findElementId(target: EventTarget | null): number | null {
		let node = target as Node | null;

		while (node && node !== this.root) {
			// Check if this node is registered in our node map
			for (const [eid, registeredNode] of this.nodes) {
				if (registeredNode === node) {
					return eid;
				}
			}
			node = node.parentNode;
		}

		return null;
	}
}
