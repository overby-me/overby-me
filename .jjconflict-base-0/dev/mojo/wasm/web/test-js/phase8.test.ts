// Phase 8.3–8.5 Tests: Context, Error Boundaries, Suspense
//
// Tests the advanced scope features:
//   8.3 — Context (Dependency Injection): provide/consume key→value pairs
//   8.4 — Error Boundaries: catch errors from child scopes
//   8.5 — Suspense: show fallback while async children are pending
//
// All tests operate on the reactive runtime's scope system via WASM exports.

import { writeStringStruct } from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, CallableFunction>;

export function testPhase8(fns: Fns): void {
	// ══════════════════════════════════════════════════════════════════
	// 8.3 — Context (Dependency Injection)
	// ══════════════════════════════════════════════════════════════════

	suite("Context — provide and consume at same scope");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		// Provide context key=42, value=100
		fns.ctx_provide(rt, scope, 42, 100);

		const found = fns.ctx_consume_found(rt, scope, 42);
		assert(found, 1, "context found at providing scope");

		const value = fns.ctx_consume(rt, scope, 42);
		assert(value, 100, "context value is 100");

		const count = fns.ctx_count(rt, scope);
		assert(count, 1, "scope has 1 context entry");

		fns.runtime_destroy(rt);
	}

	suite("Context — consume walks up parent chain");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		const child = fns.scope_create_child(rt, root);
		const grandchild = fns.scope_create_child(rt, child);

		// Provide at root
		fns.ctx_provide(rt, root, 1, 42);

		// Consume from grandchild — should walk up to root
		const found = fns.ctx_consume_found(rt, grandchild, 1);
		assert(found, 1, "grandchild finds root context");

		const value = fns.ctx_consume(rt, grandchild, 1);
		assert(value, 42, "grandchild gets root context value 42");

		// Grandchild does not provide it locally
		const local = fns.ctx_has_local(rt, grandchild, 1);
		assert(local, 0, "grandchild does not have local context");

		fns.runtime_destroy(rt);
	}

	suite("Context — closer ancestor shadows farther ancestor");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		const child = fns.scope_create_child(rt, root);
		const grandchild = fns.scope_create_child(rt, child);

		// Root provides key=5 → 10
		fns.ctx_provide(rt, root, 5, 10);
		// Child provides key=5 → 20 (shadows root)
		fns.ctx_provide(rt, child, 5, 20);

		const fromGrandchild = fns.ctx_consume(rt, grandchild, 5);
		assert(fromGrandchild, 20, "grandchild gets child's shadowed value 20");

		const fromChild = fns.ctx_consume(rt, child, 5);
		assert(fromChild, 20, "child gets its own value 20");

		const fromRoot = fns.ctx_consume(rt, root, 5);
		assert(fromRoot, 10, "root gets its own value 10");

		fns.runtime_destroy(rt);
	}

	suite("Context — missing key returns not-found");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		const found = fns.ctx_consume_found(rt, scope, 999);
		assert(found, 0, "missing key not found");

		const value = fns.ctx_consume(rt, scope, 999);
		assert(value, 0, "missing key returns 0");

		fns.runtime_destroy(rt);
	}

	suite("Context — update provided value");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		fns.ctx_provide(rt, scope, 7, 100);
		assert(fns.ctx_consume(rt, scope, 7), 100, "initial value is 100");

		fns.ctx_provide(rt, scope, 7, 200);
		assert(fns.ctx_consume(rt, scope, 7), 200, "updated value is 200");

		// Still only 1 entry (updated, not duplicated)
		assert(fns.ctx_count(rt, scope), 1, "still 1 context entry after update");

		fns.runtime_destroy(rt);
	}

	suite("Context — multiple keys at same scope");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		fns.ctx_provide(rt, scope, 1, 10);
		fns.ctx_provide(rt, scope, 2, 20);
		fns.ctx_provide(rt, scope, 3, 30);

		assert(fns.ctx_count(rt, scope), 3, "scope has 3 context entries");
		assert(fns.ctx_consume(rt, scope, 1), 10, "key 1 = 10");
		assert(fns.ctx_consume(rt, scope, 2), 20, "key 2 = 20");
		assert(fns.ctx_consume(rt, scope, 3), 30, "key 3 = 30");

		fns.runtime_destroy(rt);
	}

	suite("Context — remove context entry");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		fns.ctx_provide(rt, scope, 10, 99);
		assert(fns.ctx_count(rt, scope), 1, "1 entry before remove");

		const removed = fns.ctx_remove(rt, scope, 10);
		assert(removed, 1, "remove returns 1 (found)");
		assert(fns.ctx_count(rt, scope), 0, "0 entries after remove");
		assert(fns.ctx_consume_found(rt, scope, 10), 0, "removed key not found");

		// Remove non-existent key
		const removedAgain = fns.ctx_remove(rt, scope, 10);
		assert(removedAgain, 0, "remove of absent key returns 0");

		fns.runtime_destroy(rt);
	}

	suite("Context — deeply nested tree (5 levels)");
	{
		const rt = fns.runtime_create();
		const s0 = fns.scope_create(rt, 0, -1);
		const s1 = fns.scope_create_child(rt, s0);
		const s2 = fns.scope_create_child(rt, s1);
		const s3 = fns.scope_create_child(rt, s2);
		const s4 = fns.scope_create_child(rt, s3);

		fns.ctx_provide(rt, s0, 100, 1);
		fns.ctx_provide(rt, s2, 200, 2);

		assert(fns.ctx_consume(rt, s4, 100), 1, "s4 finds s0's context");
		assert(fns.ctx_consume(rt, s4, 200), 2, "s4 finds s2's context");
		assert(
			fns.ctx_consume_found(rt, s1, 200),
			0,
			"s1 does not find s2's context",
		);

		fns.runtime_destroy(rt);
	}

	// ══════════════════════════════════════════════════════════════════
	// 8.4 — Error Boundaries
	// ══════════════════════════════════════════════════════════════════

	suite("Error Boundary — mark scope as boundary");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		assert(fns.err_is_boundary(rt, scope), 0, "not a boundary initially");

		fns.err_set_boundary(rt, scope, 1);
		assert(fns.err_is_boundary(rt, scope), 1, "is a boundary after marking");

		fns.err_set_boundary(rt, scope, 0);
		assert(fns.err_is_boundary(rt, scope), 0, "not a boundary after unmarking");

		fns.runtime_destroy(rt);
	}

	suite("Error Boundary — set and clear error");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);
		fns.err_set_boundary(rt, scope, 1);

		assert(fns.err_has_error(rt, scope), 0, "no error initially");

		const msg = writeStringStruct("Something went wrong");
		fns.err_set_error(rt, scope, msg);

		assert(fns.err_has_error(rt, scope), 1, "has error after set");

		fns.err_clear(rt, scope);
		assert(fns.err_has_error(rt, scope), 0, "no error after clear");

		fns.runtime_destroy(rt);
	}

	suite("Error Boundary — find nearest boundary ancestor");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		const boundary = fns.scope_create_child(rt, root);
		const child = fns.scope_create_child(rt, boundary);
		const grandchild = fns.scope_create_child(rt, child);

		fns.err_set_boundary(rt, boundary, 1);

		const fromGrandchild = fns.err_find_boundary(rt, grandchild);
		assert(fromGrandchild, boundary, "grandchild finds boundary");

		const fromChild = fns.err_find_boundary(rt, child);
		assert(fromChild, boundary, "child finds boundary");

		// boundary itself looks at ancestors, not itself
		const fromBoundary = fns.err_find_boundary(rt, boundary);
		assert(fromBoundary, -1, "boundary does not find itself");

		// root has no boundary above it
		const fromRoot = fns.err_find_boundary(rt, root);
		assert(fromRoot, -1, "root has no boundary");

		fns.runtime_destroy(rt);
	}

	suite("Error Boundary — propagate error to boundary");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		fns.err_set_boundary(rt, root, 1);

		const child = fns.scope_create_child(rt, root);
		const grandchild = fns.scope_create_child(rt, child);

		const msg = writeStringStruct("render failed");
		const boundaryId = fns.err_propagate(rt, grandchild, msg);
		assert(boundaryId, root, "error propagated to root boundary");
		assert(fns.err_has_error(rt, root), 1, "root boundary has error");
		assert(
			fns.err_has_error(rt, grandchild),
			0,
			"grandchild does not have error",
		);

		fns.runtime_destroy(rt);
	}

	suite("Error Boundary — propagate with no boundary returns -1");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		const child = fns.scope_create_child(rt, root);

		const msg = writeStringStruct("unhandled error");
		const result = fns.err_propagate(rt, child, msg);
		assert(result, -1, "propagate returns -1 when no boundary");

		fns.runtime_destroy(rt);
	}

	suite("Error Boundary — nested boundaries: innermost catches");
	{
		const rt = fns.runtime_create();
		const outer = fns.scope_create(rt, 0, -1);
		fns.err_set_boundary(rt, outer, 1);

		const inner = fns.scope_create_child(rt, outer);
		fns.err_set_boundary(rt, inner, 1);

		const child = fns.scope_create_child(rt, inner);

		const msg = writeStringStruct("inner error");
		const caught = fns.err_propagate(rt, child, msg);
		assert(caught, inner, "innermost boundary catches error");
		assert(fns.err_has_error(rt, inner), 1, "inner has error");
		assert(fns.err_has_error(rt, outer), 0, "outer does not have error");

		fns.runtime_destroy(rt);
	}

	suite("Error Boundary — recovery: clear error and re-render");
	{
		const rt = fns.runtime_create();
		const boundary = fns.scope_create(rt, 0, -1);
		fns.err_set_boundary(rt, boundary, 1);

		const child = fns.scope_create_child(rt, boundary);

		// Error occurs
		const msg = writeStringStruct("crash");
		fns.err_propagate(rt, child, msg);
		assert(fns.err_has_error(rt, boundary), 1, "boundary has error");

		// Recovery
		fns.err_clear(rt, boundary);
		assert(fns.err_has_error(rt, boundary), 0, "error cleared for recovery");

		fns.runtime_destroy(rt);
	}

	// ══════════════════════════════════════════════════════════════════
	// 8.5 — Suspense
	// ══════════════════════════════════════════════════════════════════

	suite("Suspense — mark scope as boundary");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		assert(
			fns.suspense_is_boundary(rt, scope),
			0,
			"not a suspense boundary initially",
		);

		fns.suspense_set_boundary(rt, scope, 1);
		assert(
			fns.suspense_is_boundary(rt, scope),
			1,
			"is a suspense boundary after marking",
		);

		fns.suspense_set_boundary(rt, scope, 0);
		assert(
			fns.suspense_is_boundary(rt, scope),
			0,
			"not a suspense boundary after unmarking",
		);

		fns.runtime_destroy(rt);
	}

	suite("Suspense — set and clear pending");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		assert(fns.suspense_is_pending(rt, scope), 0, "not pending initially");

		fns.suspense_set_pending(rt, scope, 1);
		assert(fns.suspense_is_pending(rt, scope), 1, "pending after set");

		fns.suspense_set_pending(rt, scope, 0);
		assert(fns.suspense_is_pending(rt, scope), 0, "not pending after clear");

		fns.runtime_destroy(rt);
	}

	suite("Suspense — find nearest suspense boundary");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		const boundary = fns.scope_create_child(rt, root);
		const child = fns.scope_create_child(rt, boundary);
		const grandchild = fns.scope_create_child(rt, child);

		fns.suspense_set_boundary(rt, boundary, 1);

		assert(
			fns.suspense_find_boundary(rt, grandchild),
			boundary,
			"grandchild finds suspense boundary",
		);
		assert(
			fns.suspense_find_boundary(rt, child),
			boundary,
			"child finds suspense boundary",
		);
		assert(
			fns.suspense_find_boundary(rt, boundary),
			-1,
			"boundary does not find itself",
		);
		assert(
			fns.suspense_find_boundary(rt, root),
			-1,
			"root has no suspense boundary",
		);

		fns.runtime_destroy(rt);
	}

	suite("Suspense — has_pending_descendant detects pending child");
	{
		const rt = fns.runtime_create();
		const boundary = fns.scope_create(rt, 0, -1);
		fns.suspense_set_boundary(rt, boundary, 1);

		const child = fns.scope_create_child(rt, boundary);
		const grandchild = fns.scope_create_child(rt, child);

		assert(
			fns.suspense_has_pending(rt, boundary),
			0,
			"no pending descendants initially",
		);

		fns.suspense_set_pending(rt, grandchild, 1);
		assert(
			fns.suspense_has_pending(rt, boundary),
			1,
			"boundary detects pending grandchild",
		);

		fns.suspense_set_pending(rt, grandchild, 0);
		assert(
			fns.suspense_has_pending(rt, boundary),
			0,
			"no pending after grandchild resolves",
		);

		fns.runtime_destroy(rt);
	}

	suite("Suspense — resolve returns suspense boundary");
	{
		const rt = fns.runtime_create();
		const boundary = fns.scope_create(rt, 0, -1);
		fns.suspense_set_boundary(rt, boundary, 1);

		const child = fns.scope_create_child(rt, boundary);
		fns.suspense_set_pending(rt, child, 1);

		const result = fns.suspense_resolve(rt, child);
		assert(result, boundary, "resolve returns the suspense boundary");
		assert(
			fns.suspense_is_pending(rt, child),
			0,
			"child no longer pending after resolve",
		);

		fns.runtime_destroy(rt);
	}

	suite("Suspense — resolve with no boundary returns -1");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		const child = fns.scope_create_child(rt, root);
		fns.suspense_set_pending(rt, child, 1);

		const result = fns.suspense_resolve(rt, child);
		assert(result, -1, "resolve returns -1 when no suspense boundary");
		assert(
			fns.suspense_is_pending(rt, child),
			0,
			"pending cleared even without boundary",
		);

		fns.runtime_destroy(rt);
	}

	suite("Suspense — multiple pending children");
	{
		const rt = fns.runtime_create();
		const boundary = fns.scope_create(rt, 0, -1);
		fns.suspense_set_boundary(rt, boundary, 1);

		const childA = fns.scope_create_child(rt, boundary);
		const childB = fns.scope_create_child(rt, boundary);

		fns.suspense_set_pending(rt, childA, 1);
		fns.suspense_set_pending(rt, childB, 1);

		assert(
			fns.suspense_has_pending(rt, boundary),
			1,
			"has pending with two children",
		);

		// Resolve one
		fns.suspense_resolve(rt, childA);
		assert(
			fns.suspense_has_pending(rt, boundary),
			1,
			"still has pending after one resolves",
		);

		// Resolve the other
		fns.suspense_resolve(rt, childB);
		assert(
			fns.suspense_has_pending(rt, boundary),
			0,
			"no pending after both resolve",
		);

		fns.runtime_destroy(rt);
	}

	suite("Suspense — nested boundaries: innermost catches");
	{
		const rt = fns.runtime_create();
		const outer = fns.scope_create(rt, 0, -1);
		fns.suspense_set_boundary(rt, outer, 1);

		const inner = fns.scope_create_child(rt, outer);
		fns.suspense_set_boundary(rt, inner, 1);

		const child = fns.scope_create_child(rt, inner);
		fns.suspense_set_pending(rt, child, 1);

		assert(
			fns.suspense_find_boundary(rt, child),
			inner,
			"child finds inner boundary",
		);

		// Inner has the pending descendant
		assert(
			fns.suspense_has_pending(rt, inner),
			1,
			"inner boundary has pending descendant",
		);

		// Outer also has pending descendant (child is nested under it)
		assert(
			fns.suspense_has_pending(rt, outer),
			1,
			"outer boundary also has pending descendant",
		);

		// Resolve
		fns.suspense_resolve(rt, child);
		assert(fns.suspense_has_pending(rt, inner), 0, "inner clear after resolve");
		assert(fns.suspense_has_pending(rt, outer), 0, "outer clear after resolve");

		fns.runtime_destroy(rt);
	}

	// ══════════════════════════════════════════════════════════════════
	// Combined scenarios
	// ══════════════════════════════════════════════════════════════════

	suite("Combined — context + error boundary on same scope");
	{
		const rt = fns.runtime_create();
		const scope = fns.scope_create(rt, 0, -1);

		// Both features coexist
		fns.err_set_boundary(rt, scope, 1);
		fns.ctx_provide(rt, scope, 1, 42);

		assert(fns.err_is_boundary(rt, scope), 1, "is error boundary");
		assert(fns.ctx_consume(rt, scope, 1), 42, "context value intact");

		fns.runtime_destroy(rt);
	}

	suite("Combined — all three features on a scope tree");
	{
		const rt = fns.runtime_create();
		const root = fns.scope_create(rt, 0, -1);
		fns.ctx_provide(rt, root, 1, 100);
		fns.err_set_boundary(rt, root, 1);
		fns.suspense_set_boundary(rt, root, 1);

		const child = fns.scope_create_child(rt, root);
		const grandchild = fns.scope_create_child(rt, child);

		// Context works
		assert(fns.ctx_consume(rt, grandchild, 1), 100, "grandchild reads context");

		// Error boundary works
		const errMsg = writeStringStruct("oops");
		const boundary = fns.err_propagate(rt, grandchild, errMsg);
		assert(boundary, root, "error propagates to root");
		assert(fns.err_has_error(rt, root), 1, "root caught error");

		// Suspense works
		fns.suspense_set_pending(rt, grandchild, 1);
		assert(
			fns.suspense_has_pending(rt, root),
			1,
			"root sees pending grandchild",
		);

		fns.suspense_resolve(rt, grandchild);
		assert(fns.suspense_has_pending(rt, root), 0, "root no longer has pending");

		// Clean up error
		fns.err_clear(rt, root);
		assert(fns.err_has_error(rt, root), 0, "error cleared");

		fns.runtime_destroy(rt);
	}
}
