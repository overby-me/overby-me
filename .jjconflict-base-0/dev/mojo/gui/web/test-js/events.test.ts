// EventBridge String Dispatch Tests — Phase 20 (M20.2)
//
// Tests the JS EventBridge's string event dispatch infrastructure:
//   - DispatchWithStringFn callback invocation for input/change events
//   - String value extraction from event.target.value
//   - Fallback to numeric dispatch when string dispatch returns 0
//   - Fallback to default dispatch for non-input events
//   - Integration with WASM runtime: signal_set_string handler end-to-end
//
// Uses linkedom for headless DOM simulation and mock dispatch functions
// for unit-level EventBridge tests.

import { parseHTML } from "npm:linkedom";
import { EventBridge, EventType } from "../runtime/events.ts";
import {
	allocStringStruct,
	readStringStruct,
	writeStringStruct,
} from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, pass, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, CallableFunction>;

// ── DOM helpers ─────────────────────────────────────────────────────────────

function createDOM() {
	const { document, window } = parseHTML(
		'<!DOCTYPE html><html><body><div id="root"></div></body></html>',
	);
	const root = document.getElementById("root")!;
	return { document, window, root };
}

/**
 * Create a minimal DOM tree with an input element inside the root,
 * and an EventBridge wired to mock dispatch functions.
 *
 * Returns the bridge, the input element, and call logs for each
 * dispatch function.
 */
function createBridgeFixture() {
	const { document, root } = createDOM();

	// Build a DOM: root > div > input
	const div = document.createElement("div");
	const input = document.createElement("input");
	input.setAttribute("type", "text");
	div.appendChild(input);
	root.appendChild(div);

	// Node map: assign element IDs
	const nodeMap = new Map<number, Node>();
	nodeMap.set(1, div);
	nodeMap.set(2, input);

	const bridge = new EventBridge(root, nodeMap);

	// Call logs
	const dispatchLog: Array<{ hid: number; evt: number }> = [];
	const dispatchValueLog: Array<{
		hid: number;
		evt: number;
		value: number;
	}> = [];
	const dispatchStringLog: Array<{
		hid: number;
		evt: number;
		stringPtr: bigint;
	}> = [];

	// Track return values for mocks
	let stringDispatchReturn = 1;

	bridge.setDispatch(
		// Default dispatch
		(hid: number, evt: number) => {
			dispatchLog.push({ hid, evt });
			return 1;
		},
		// Dispatch with value (numeric)
		(hid: number, evt: number, value: number) => {
			dispatchValueLog.push({ hid, evt, value });
			return 1;
		},
		// Dispatch with string (Phase 20)
		(hid: number, evt: number, stringPtr: bigint) => {
			dispatchStringLog.push({ hid, evt, stringPtr });
			return stringDispatchReturn;
		},
	);

	return {
		document,
		root,
		div,
		input,
		bridge,
		nodeMap,
		dispatchLog,
		dispatchValueLog,
		dispatchStringLog,
		setStringDispatchReturn(v: number) {
			stringDispatchReturn = v;
		},
	};
}

// ── Exported test function ──────────────────────────────────────────────────

export function testEvents(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: EventBridge unit tests with mock dispatch functions
	// ═════════════════════════════════════════════════════════════════════

	suite("EventBridge — input event calls dispatchWithStringFn");
	{
		const f = createBridgeFixture();

		// Register handler for input event on element 2 (the <input>)
		f.bridge.addHandler(2, "input", 42);
		f.bridge.install();

		// Simulate input event
		f.input.value = "hello world";
		const evt = new f.document.defaultView!.Event("input", {
			bubbles: true,
		});
		f.input.dispatchEvent(evt);

		assert(f.dispatchStringLog.length, 1, "dispatchWithStringFn called once");
		assert(f.dispatchStringLog[0]?.hid, 42, "handler ID is 42");
		assert(f.dispatchStringLog[0]?.evt, EventType.Input, "event type is Input");
		// Numeric and default dispatch should NOT be called
		assert(f.dispatchValueLog.length, 0, "dispatchWithValueFn not called");
		assert(f.dispatchLog.length, 0, "default dispatchFn not called");

		f.bridge.uninstall();
	}

	suite("EventBridge — change event calls dispatchWithStringFn");
	{
		const f = createBridgeFixture();

		f.bridge.addHandler(2, "change", 99);
		f.bridge.install();

		f.input.value = "changed value";
		const evt = new f.document.defaultView!.Event("change", {
			bubbles: true,
		});
		f.input.dispatchEvent(evt);

		assert(
			f.dispatchStringLog.length,
			1,
			"dispatchWithStringFn called for change",
		);
		assert(f.dispatchStringLog[0]?.hid, 99, "handler ID is 99");
		assert(
			f.dispatchStringLog[0]?.evt,
			EventType.Change,
			"event type is Change",
		);
		assert(
			f.dispatchValueLog.length,
			0,
			"dispatchWithValueFn not called for change",
		);
		assert(f.dispatchLog.length, 0, "default dispatchFn not called for change");

		f.bridge.uninstall();
	}

	suite("EventBridge — string dispatch returns 0 falls back to numeric");
	{
		const f = createBridgeFixture();

		// Make string dispatch return 0 (not handled)
		f.setStringDispatchReturn(0);

		f.bridge.addHandler(2, "input", 50);
		f.bridge.install();

		// Value is numeric — should fall back to dispatchWithValueFn
		f.input.value = "123";
		const evt = new f.document.defaultView!.Event("input", {
			bubbles: true,
		});
		f.input.dispatchEvent(evt);

		assert(
			f.dispatchStringLog.length,
			1,
			"dispatchWithStringFn was tried first",
		);
		assert(
			f.dispatchValueLog.length,
			1,
			"dispatchWithValueFn called as fallback",
		);
		assert(f.dispatchValueLog[0]?.value, 123, "numeric value is 123");
		assert(f.dispatchLog.length, 0, "default dispatchFn not called");

		f.bridge.uninstall();
	}

	suite(
		"EventBridge — string dispatch returns 0, non-numeric falls to default",
	);
	{
		const f = createBridgeFixture();

		f.setStringDispatchReturn(0);

		f.bridge.addHandler(2, "input", 50);
		f.bridge.install();

		// Value is non-numeric — should fall through to default dispatch
		f.input.value = "not a number";
		const evt = new f.document.defaultView!.Event("input", {
			bubbles: true,
		});
		f.input.dispatchEvent(evt);

		assert(
			f.dispatchStringLog.length,
			1,
			"dispatchWithStringFn was tried first",
		);
		assert(
			f.dispatchValueLog.length,
			0,
			"dispatchWithValueFn not called (non-numeric)",
		);
		assert(
			f.dispatchLog.length,
			1,
			"default dispatchFn called as final fallback",
		);

		f.bridge.uninstall();
	}

	suite("EventBridge — click event does NOT call dispatchWithStringFn");
	{
		const f = createBridgeFixture();

		f.bridge.addHandler(1, "click", 10);
		f.bridge.install();

		const evt = new f.document.defaultView!.Event("click", {
			bubbles: true,
		});
		f.div.dispatchEvent(evt);

		assert(
			f.dispatchStringLog.length,
			0,
			"dispatchWithStringFn not called for click",
		);
		assert(
			f.dispatchValueLog.length,
			0,
			"dispatchWithValueFn not called for click",
		);
		assert(f.dispatchLog.length, 1, "default dispatchFn called for click");
		assert(f.dispatchLog[0]?.hid, 10, "click handler ID is 10");

		f.bridge.uninstall();
	}

	suite("EventBridge — empty string input dispatches via string path");
	{
		const f = createBridgeFixture();

		f.bridge.addHandler(2, "input", 77);
		f.bridge.install();

		f.input.value = "";
		const evt = new f.document.defaultView!.Event("input", {
			bubbles: true,
		});
		f.input.dispatchEvent(evt);

		assert(
			f.dispatchStringLog.length,
			1,
			"dispatchWithStringFn called for empty string",
		);
		assert(f.dispatchStringLog[0]?.hid, 77, "handler ID is 77");

		f.bridge.uninstall();
	}

	suite("EventBridge — no string dispatch fn falls back to numeric");
	{
		const { document, root } = createDOM();

		const div = document.createElement("div");
		const input = document.createElement("input");
		input.setAttribute("type", "text");
		div.appendChild(input);
		root.appendChild(div);

		const nodeMap = new Map<number, Node>();
		nodeMap.set(1, div);
		nodeMap.set(2, input);

		const bridge = new EventBridge(root, nodeMap);

		const valueLog: number[] = [];
		const defaultLog: number[] = [];

		// Set dispatch WITHOUT string dispatch function
		bridge.setDispatch(
			(hid: number, _evt: number) => {
				defaultLog.push(hid);
				return 1;
			},
			(_hid: number, _evt: number, value: number) => {
				valueLog.push(value);
				return 1;
			},
			// No string dispatch
		);

		bridge.addHandler(2, "input", 33);
		bridge.install();

		input.value = "456";
		const evt = new document.defaultView!.Event("input", { bubbles: true });
		input.dispatchEvent(evt);

		assert(valueLog.length, 1, "dispatchWithValueFn called when no string fn");
		assert(valueLog[0], 456, "numeric value is 456");
		assert(defaultLog.length, 0, "default not called when numeric handled");

		bridge.uninstall();
	}

	suite("EventBridge — onAfterDispatch fires after string dispatch");
	{
		const f = createBridgeFixture();
		let afterCount = 0;
		f.bridge.onAfterDispatch = () => {
			afterCount++;
		};

		f.bridge.addHandler(2, "input", 88);
		f.bridge.install();

		f.input.value = "test";
		const evt = new f.document.defaultView!.Event("input", {
			bubbles: true,
		});
		f.input.dispatchEvent(evt);

		assert(afterCount, 1, "onAfterDispatch called once after string dispatch");

		f.bridge.uninstall();
	}

	suite("EventBridge — multiple input events dispatch correctly in sequence");
	{
		const f = createBridgeFixture();

		f.bridge.addHandler(2, "input", 55);
		f.bridge.install();

		const values = ["first", "second", "third"];
		for (const v of values) {
			f.input.value = v;
			const evt = new f.document.defaultView!.Event("input", {
				bubbles: true,
			});
			f.input.dispatchEvent(evt);
		}

		assert(
			f.dispatchStringLog.length,
			3,
			"dispatchWithStringFn called 3 times",
		);
		// All should target handler 55
		for (let i = 0; i < 3; i++) {
			assert(
				f.dispatchStringLog[i]?.hid,
				55,
				`call ${i + 1} targets handler 55`,
			);
		}

		f.bridge.uninstall();
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: WASM integration — string dispatch end-to-end
	// ═════════════════════════════════════════════════════════════════════

	suite("EventBridge WASM — string dispatch writes to SignalString");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		// Create a string signal with initial value "initial"
		const packed = fns.signal_create_string(rt, writeStringStruct("initial"));
		const stringKey = fns.signal_string_key(packed);
		const versionKey = fns.signal_version_key(packed);

		// Register a signal_set_string handler for "input" events
		const handlerId = fns.handler_register_signal_set_string(
			rt,
			scope,
			stringKey,
			versionKey,
			writeStringStruct("input"),
		);

		// Verify the handler was created (IDs are 0-based)
		assert(handlerId >= 0, true, "handler ID is non-negative");

		// Dispatch with string value "hello from bridge"
		const strPtr = writeStringStruct("hello from bridge");
		const handled = fns.dispatch_event_with_string(
			rt,
			handlerId,
			EventType.Input,
			strPtr,
		);
		assert(handled, 1, "dispatch returned 1 (handled)");

		// Read back the signal value
		const outPtr = allocStringStruct();
		fns.signal_peek_string(rt, stringKey, outPtr);
		const result = readStringStruct(outPtr);
		assert(result, "hello from bridge", "SignalString updated correctly");

		fns.runtime_destroy(rt);
	}

	suite("EventBridge WASM — string dispatch empty string writes correctly");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		const packed = fns.signal_create_string(rt, writeStringStruct("non-empty"));
		const stringKey = fns.signal_string_key(packed);
		const versionKey = fns.signal_version_key(packed);

		const handlerId = fns.handler_register_signal_set_string(
			rt,
			scope,
			stringKey,
			versionKey,
			writeStringStruct("input"),
		);

		// Dispatch empty string
		const handled = fns.dispatch_event_with_string(
			rt,
			handlerId,
			EventType.Input,
			writeStringStruct(""),
		);
		assert(handled, 1, "empty string dispatch handled");

		const outPtr = allocStringStruct();
		fns.signal_peek_string(rt, stringKey, outPtr);
		const result = readStringStruct(outPtr);
		assert(result, "", "SignalString is now empty");

		fns.runtime_destroy(rt);
	}

	suite("EventBridge WASM — string dispatch bumps version signal");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		const packed = fns.signal_create_string(rt, writeStringStruct("v0"));
		const stringKey = fns.signal_string_key(packed);
		const versionKey = fns.signal_version_key(packed);

		// Read initial version
		const v0 = fns.signal_read_i32(rt, versionKey);
		assert(v0, 0, "initial version is 0");

		const handlerId = fns.handler_register_signal_set_string(
			rt,
			scope,
			stringKey,
			versionKey,
			writeStringStruct("input"),
		);

		// Dispatch twice
		fns.dispatch_event_with_string(
			rt,
			handlerId,
			EventType.Input,
			writeStringStruct("v1"),
		);
		const v1 = fns.signal_read_i32(rt, versionKey);
		assert(v1, 1, "version is 1 after first dispatch");

		fns.dispatch_event_with_string(
			rt,
			handlerId,
			EventType.Input,
			writeStringStruct("v2"),
		);
		const v2 = fns.signal_read_i32(rt, versionKey);
		assert(v2, 2, "version is 2 after second dispatch");

		fns.runtime_destroy(rt);
	}

	suite("EventBridge WASM — string dispatch marks subscriber scope dirty");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		// Begin render context so signal reads subscribe the scope
		const prevScope = fns.scope_begin_render(rt, scope);

		const packed = fns.signal_create_string(rt, writeStringStruct("start"));
		const stringKey = fns.signal_string_key(packed);
		const versionKey = fns.signal_version_key(packed);

		// Read the version signal inside render context → subscribes scope
		fns.signal_read_i32(rt, versionKey);

		// End render context
		fns.scope_end_render(rt, prevScope);

		const handlerId = fns.handler_register_signal_set_string(
			rt,
			scope,
			stringKey,
			versionKey,
			writeStringStruct("input"),
		);

		// Dispatch — should mark scope dirty via version signal subscription
		fns.dispatch_event_with_string(
			rt,
			handlerId,
			EventType.Input,
			writeStringStruct("updated"),
		);

		const dirtyCount = fns.runtime_drain_dirty(rt);
		assert(dirtyCount >= 1, true, "at least one dirty scope after dispatch");

		fns.runtime_destroy(rt);
	}

	suite(
		"EventBridge WASM — non-string handler falls back via dispatch_event_with_string",
	);
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		// Register a signal_add handler (not signal_set_string)
		const sigKey = fns.signal_create_i32(rt, 10);
		const handlerId = fns.handler_register_signal_add(
			rt,
			scope,
			sigKey,
			5,
			writeStringStruct("click"),
		);

		// Dispatch via string path — should fall back to dispatch_event
		// which handles ACTION_SIGNAL_ADD_I32
		const handled = fns.dispatch_event_with_string(
			rt,
			handlerId,
			EventType.Click,
			writeStringStruct("ignored"),
		);
		assert(handled, 1, "fallback dispatch handled the signal_add action");

		// Verify signal was updated (10 + 5 = 15)
		const val = fns.signal_read_i32(rt, sigKey);
		assert(val, 15, "signal value is 15 after add fallback");

		fns.runtime_destroy(rt);
	}

	suite("EventBridge WASM — writeStringStruct round-trip");
	{
		// Verify that writeStringStruct + readStringStruct is a faithful
		// round-trip for various string values.
		const testStrings = [
			"",
			"a",
			"hello",
			"hello world with spaces",
			"emoji: 🔥🎉",
			"日本語テスト",
			"a".repeat(100),
		];

		for (const s of testStrings) {
			const ptr = writeStringStruct(s);
			const result = readStringStruct(ptr);
			assert(result, s, `round-trip: "${s.slice(0, 30)}..."`);
		}
		pass(0); // All assertions above already counted
	}
}
