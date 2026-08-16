// Phase 31.1 — Context (Dependency Injection) Surface Tests
//
// Tests the ContextTestApp (cta_*) WASM exports which exercise
// ComponentContext.provide_context(), consume_context(), has_context(),
// and the typed signal-sharing helpers.
//
// Validates:
//   - provide + consume round-trip via WASM exports
//   - missing key returns 0
//   - signal sharing via context (provide, consume, read value)
//   - signal write via consumed handle marks dirty
//   - overwrite context key
//   - multiple keys
//   - child scope consumes parent context
//   - context across signal write cycles
//   - destroy cleanup
//   - independent instances don't share context

import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, CallableFunction>;

export function testContext(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: provide + consume round-trip
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — provide + consume round-trip");
	{
		const app = fns.cta_init();

		// Provide key=42, value=123
		fns.cta_provide_context(app, 42, 123);
		const val = fns.cta_consume_context(app, 42);
		assert(val, 123, "consume returns provided value");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: missing key returns 0
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — missing key returns 0");
	{
		const app = fns.cta_init();

		const val = fns.cta_consume_context(app, 999);
		assert(val, 0, "consume of missing key returns 0");

		const found = fns.cta_has_context(app, 999);
		assert(found, 0, "has_context returns 0 for missing key");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: signal sharing via context
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — signal sharing via context");
	{
		const app = fns.cta_init();

		// Provide the count signal at context key 1
		fns.cta_provide_signal_i32(app, 1);

		// Consume from child — should read the signal's initial value (0)
		const val = fns.cta_consume_signal_i32_from_child(app, 1);
		assert(val, 0, "consumed signal reads initial value 0");

		// Verify the count signal key was stored in context
		const signalKey = fns.cta_count_signal_key(app);
		const ctxVal = fns.cta_consume_from_child(app, 1);
		assert(ctxVal, signalKey, "context stores the signal key");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: signal write via consumed handle marks dirty
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — signal write via consumed handle marks dirty");
	{
		const app = fns.cta_init();

		// Should start clean
		assert(fns.cta_has_dirty(app), 0, "starts clean");

		// Provide and write via child
		fns.cta_provide_signal_i32(app, 1);
		fns.cta_write_signal_via_child(app, 1, 42);

		assert(fns.cta_has_dirty(app), 1, "dirty after signal write");

		// Count should be updated
		assert(fns.cta_count_value(app), 42, "count updated to 42");

		// Child can also read the updated value
		const childVal = fns.cta_consume_signal_i32_from_child(app, 1);
		assert(childVal, 42, "child reads updated value 42");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: overwrite context key
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — overwrite context key");
	{
		const app = fns.cta_init();

		fns.cta_provide_context(app, 5, 100);
		assert(fns.cta_consume_context(app, 5), 100, "first value is 100");

		fns.cta_provide_context(app, 5, 200);
		assert(fns.cta_consume_context(app, 5), 200, "overwritten value is 200");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: multiple keys
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — multiple keys coexist");
	{
		const app = fns.cta_init();

		fns.cta_provide_context(app, 1, 10);
		fns.cta_provide_context(app, 2, 20);
		fns.cta_provide_context(app, 3, 30);

		assert(fns.cta_consume_context(app, 1), 10, "key 1 = 10");
		assert(fns.cta_consume_context(app, 2), 20, "key 2 = 20");
		assert(fns.cta_consume_context(app, 3), 30, "key 3 = 30");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: child scope consumes parent context
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — child scope consumes parent context");
	{
		const app = fns.cta_init();

		// Verify distinct scopes
		const rootScope = fns.cta_root_scope_id(app);
		const childScope = fns.cta_child_scope_id(app);
		assert(rootScope !== childScope, true, "root and child scopes differ");

		// Provide at root, consume from child
		fns.cta_provide_context(app, 50, 500);
		const fromChild = fns.cta_consume_from_child(app, 50);
		assert(fromChild, 500, "child consumes parent's context value");

		const foundFromChild = fns.cta_consume_found_from_child(app, 50);
		assert(foundFromChild, 1, "consume_found returns 1 from child");

		// Missing key from child
		const missingFromChild = fns.cta_consume_found_from_child(app, 777);
		assert(missingFromChild, 0, "consume_found returns 0 for missing key");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: context across signal write cycles
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — context survives across signal write cycles");
	{
		const app = fns.cta_init();

		fns.cta_provide_context(app, 7, 777);
		fns.cta_provide_signal_i32(app, 1);

		// Multiple writes
		fns.cta_write_signal_via_child(app, 1, 5);
		fns.cta_write_signal_via_child(app, 1, 10);
		fns.cta_write_signal_via_child(app, 1, 15);

		// Context value still intact
		const ctxVal = fns.cta_consume_from_child(app, 7);
		assert(ctxVal, 777, "context value survives signal writes");

		// Signal value updated
		assert(fns.cta_count_value(app), 15, "signal value is 15");

		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: destroy cleanup
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — destroy cleanup");
	{
		const app = fns.cta_init();
		fns.cta_provide_context(app, 1, 42);
		fns.cta_provide_signal_i32(app, 2);
		fns.cta_write_signal_via_child(app, 2, 99);
		// Destroy should not crash
		fns.cta_destroy(app);
		// If we got here, no crash
		assert(true, true, "destroy after context + signal use does not crash");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 10: independent instances don't share context
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — independent instances don't share context");
	{
		const app1 = fns.cta_init();
		const app2 = fns.cta_init();

		fns.cta_provide_context(app1, 1, 111);
		fns.cta_provide_context(app2, 1, 222);

		assert(fns.cta_consume_context(app1, 1), 111, "app1 sees its own value");
		assert(fns.cta_consume_context(app2, 1), 222, "app2 sees its own value");

		// Key 2 only in app1
		fns.cta_provide_context(app1, 2, 333);
		assert(fns.cta_has_context(app2, 2), 0, "app2 does not see app1's key 2");

		fns.cta_destroy(app1);
		fns.cta_destroy(app2);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 11: scope count
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — scope count reflects 2 scopes");
	{
		const app = fns.cta_init();
		assert(fns.cta_scope_count(app), 2, "2 live scopes (root + child)");
		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 12: consume_signal missing key returns sentinel
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — consume_signal missing key returns sentinel");
	{
		const app = fns.cta_init();
		const val = fns.cta_consume_signal_i32_from_child(app, 999);
		assert(val, -9999, "sentinel -9999 for missing context key");
		fns.cta_destroy(app);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 13: destroy + recreate cycle
	// ═════════════════════════════════════════════════════════════════════

	suite("Context — destroy + recreate cycle");
	{
		const app1 = fns.cta_init();
		fns.cta_provide_context(app1, 1, 100);
		fns.cta_destroy(app1);

		const app2 = fns.cta_init();
		// New app should not see old context
		assert(
			fns.cta_consume_context(app2, 1),
			0,
			"recreated app has fresh context",
		);
		fns.cta_destroy(app2);
	}
}
