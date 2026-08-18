import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

// ── Helpers ─────────────────────────────────────────────────────────────────

function createRt(fns: WasmExports): bigint {
	return fns.runtime_create();
}

function destroyRt(fns: WasmExports, rt: bigint): void {
	fns.runtime_destroy(rt);
}

// ── Tests ───────────────────────────────────────────────────────────────────

export function testEffect(fns: WasmExports): void {
	// ── Create / Destroy ────────────────────────────────────────────
	suite("Effect — create and destroy");
	{
		const rt = createRt(fns);

		const scopeId = fns.scope_create(rt, 0, -1);
		assert(fns.effect_count(rt), 0, "no effects initially");

		const e0 = fns.effect_create(rt, scopeId);
		assert(fns.effect_count(rt), 1, "effect count is 1 after create");
		assert(e0 >= 0, true, "effect ID is non-negative");

		// Effect starts pending (needs first run)
		assert(
			fns.effect_is_pending(rt, e0),
			1,
			"effect is pending after creation",
		);

		// Context ID is a valid signal key
		const ctxId = fns.effect_context_id(rt, e0);
		assert(fns.signal_contains(rt, ctxId), 1, "context signal exists");

		// Destroy
		fns.effect_destroy(rt, e0);
		assert(fns.effect_count(rt), 0, "effect count is 0 after destroy");
		// Context signal should be cleaned up
		assert(
			fns.signal_contains(rt, ctxId),
			0,
			"context signal destroyed after effect destroy",
		);

		destroyRt(fns, rt);
	}

	// ── Two effects, distinct IDs ───────────────────────────────────
	suite("Effect — two effects with distinct IDs");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const e0 = fns.effect_create(rt, scopeId);
		const e1 = fns.effect_create(rt, scopeId);
		assert(e0 !== e1, true, "effect IDs are distinct");
		assert(fns.effect_count(rt), 2, "count is 2");

		fns.effect_destroy(rt, e0);
		fns.effect_destroy(rt, e1);
		assert(fns.effect_count(rt), 0, "count is 0 after destroying both");

		destroyRt(fns, rt);
	}

	// ── Begin/end run clears pending ────────────────────────────────
	suite("Effect — begin/end run clears pending");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const e0 = fns.effect_create(rt, scopeId);
		assert(fns.effect_is_pending(rt, e0), 1, "pending before run");

		fns.effect_begin_run(rt, e0);
		fns.effect_end_run(rt, e0);

		assert(fns.effect_is_pending(rt, e0), 0, "not pending after run");

		destroyRt(fns, rt);
	}

	// ── Auto-tracking: signal read during run subscribes ────────────
	suite("Effect — auto-tracking during run");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 10);

		const e0 = fns.effect_create(rt, scopeId);
		const subsBefore = fns.signal_subscriber_count(rt, sig);

		// Run effect reading the signal
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		const subsAfter = fns.signal_subscriber_count(rt, sig);
		assert(
			subsAfter,
			subsBefore + 1,
			"signal gains one subscriber from effect",
		);

		destroyRt(fns, rt);
	}

	// ── Signal write → effect pending ───────────────────────────────
	suite("Effect — signal write marks effect pending");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);

		// Run to establish subscription
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);
		assert(fns.effect_is_pending(rt, e0), 0, "not pending after run");

		// Write to signal
		fns.signal_write_i32(rt, sig, 42);
		assert(fns.effect_is_pending(rt, e0), 1, "pending after signal write");

		destroyRt(fns, rt);
	}

	// ── Signal write does NOT dirty scope (only effect) ─────────────
	suite("Effect — signal write only marks effect pending, not scope dirty");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);

		// Run effect to subscribe to signal
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		assert(fns.runtime_dirty_count(rt), 0, "no dirty scopes before write");

		// Write — only effect context is subscribed
		fns.signal_write_i32(rt, sig, 1);
		assert(fns.effect_is_pending(rt, e0), 1, "effect is pending");
		assert(
			fns.runtime_dirty_count(rt),
			0,
			"no dirty scopes (only effect subscribed)",
		);

		destroyRt(fns, rt);
	}

	// ── Two effects on same signal both become pending ───────────────
	suite("Effect — two effects on same signal");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);
		const e1 = fns.effect_create(rt, scopeId);

		// Run both, reading the same signal
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		fns.effect_begin_run(rt, e1);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e1);

		// Write to signal
		fns.signal_write_i32(rt, sig, 99);
		assert(fns.effect_is_pending(rt, e0), 1, "e0 pending after write");
		assert(fns.effect_is_pending(rt, e1), 1, "e1 pending after write");

		destroyRt(fns, rt);
	}

	// ── Scope and effect both react ─────────────────────────────────
	suite("Effect — scope dirty AND effect pending from same signal");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		// Subscribe scope via render
		const prev = fns.scope_begin_render(rt, scopeId);
		fns.signal_read_i32(rt, sig);
		fns.scope_end_render(rt, prev);

		// Subscribe effect
		const e0 = fns.effect_create(rt, scopeId);
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		// Write
		fns.signal_write_i32(rt, sig, 42);
		assert(fns.effect_is_pending(rt, e0), 1, "effect is pending");
		assert(fns.runtime_dirty_count(rt) > 0, true, "scope is dirty");

		destroyRt(fns, rt);
	}

	// ── Dependency re-tracking ──────────────────────────────────────
	suite("Effect — dependency re-tracking on second run");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sigA = fns.signal_create_i32(rt, 0);
		const sigB = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);

		// First run: read sigA
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sigA);
		fns.effect_end_run(rt, e0);

		// Verify sigA triggers pending
		fns.signal_write_i32(rt, sigA, 1);
		assert(fns.effect_is_pending(rt, e0), 1, "pending after sigA write");

		// Second run: read sigB (NOT sigA)
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sigB);
		fns.effect_end_run(rt, e0);

		// sigA should NOT trigger pending anymore
		fns.signal_write_i32(rt, sigA, 2);
		assert(
			fns.effect_is_pending(rt, e0),
			0,
			"sigA no longer tracked after re-tracking",
		);

		// sigB SHOULD trigger pending
		fns.signal_write_i32(rt, sigB, 1);
		assert(fns.effect_is_pending(rt, e0), 1, "sigB now triggers pending");

		destroyRt(fns, rt);
	}

	// ── Drain pending effects ───────────────────────────────────────
	suite("Effect — drain pending effects");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);
		const e1 = fns.effect_create(rt, scopeId);

		// Both start pending
		assert(fns.effect_drain_pending(rt), 2, "both start pending");

		// Run both to clear pending, subscribing to sig
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		fns.effect_begin_run(rt, e1);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e1);

		assert(fns.effect_drain_pending(rt), 0, "none pending after running both");

		// Write to signal — both become pending
		fns.signal_write_i32(rt, sig, 99);
		assert(fns.effect_drain_pending(rt), 2, "both pending after write");

		// Run only e0
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		assert(fns.effect_drain_pending(rt), 1, "only e1 pending after running e0");
		assert(
			fns.effect_pending_at(rt, 0),
			e1,
			"e1 is the remaining pending effect",
		);

		destroyRt(fns, rt);
	}

	// ── Effect reads memo output → pending on memo input change ─────
	suite("Effect — reads memo output, pending on memo input change");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 5);

		// Create a memo that doubles the signal
		const m0 = fns.memo_create_i32(rt, scopeId, 0);
		fns.memo_begin_compute(rt, m0);
		const val = fns.signal_read_i32(rt, sig);
		fns.memo_end_compute_i32(rt, m0, val * 2);

		// Get memo output key
		const outKey = fns.memo_output_key(rt, m0);

		// Create effect that reads the memo's output signal
		const e0 = fns.effect_create(rt, scopeId);
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, outKey);
		fns.effect_end_run(rt, e0);

		assert(fns.effect_is_pending(rt, e0), 0, "not pending after run");

		// Write to base signal → memo dirty → propagate to memo output subs → effect pending
		fns.signal_write_i32(rt, sig, 10);
		assert(
			fns.effect_is_pending(rt, e0),
			1,
			"effect pending after memo input signal changes",
		);

		destroyRt(fns, rt);
	}

	// ── Hook: use_effect creates on first render ────────────────────
	suite("Effect — hook use_effect creates on first render");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const prev = fns.scope_begin_render(rt, scopeId);
		const eid = fns.hook_use_effect(rt);
		fns.scope_end_render(rt, prev);

		assert(eid >= 0, true, "effect ID is non-negative");
		assert(fns.effect_count(rt), 1, "one effect exists");
		assert(fns.effect_is_pending(rt, eid), 1, "hook effect starts pending");

		destroyRt(fns, rt);
	}

	// ── Hook: same ID on re-render ──────────────────────────────────
	suite("Effect — hook use_effect same ID on re-render");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		// First render
		let prev = fns.scope_begin_render(rt, scopeId);
		const eid1 = fns.hook_use_effect(rt);
		fns.scope_end_render(rt, prev);

		// Re-render
		prev = fns.scope_begin_render(rt, scopeId);
		const eid2 = fns.hook_use_effect(rt);
		fns.scope_end_render(rt, prev);

		assert(eid1, eid2, "same effect ID on re-render");
		assert(fns.effect_count(rt), 1, "still only one effect");

		destroyRt(fns, rt);
	}

	// ── Hook: interleaved with signal and memo hooks ────────────────
	suite("Effect — hook interleaved with signal and memo");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		// First render: signal, effect, memo, effect
		let prev = fns.scope_begin_render(rt, scopeId);
		const sig = fns.hook_use_signal_i32(rt, 0);
		const eff0 = fns.hook_use_effect(rt);
		const mem = fns.hook_use_memo_i32(rt, 0);
		const eff1 = fns.hook_use_effect(rt);
		fns.scope_end_render(rt, prev);

		// Re-render
		prev = fns.scope_begin_render(rt, scopeId);
		const sig2 = fns.hook_use_signal_i32(rt, 0);
		const eff0b = fns.hook_use_effect(rt);
		const mem2 = fns.hook_use_memo_i32(rt, 0);
		const eff1b = fns.hook_use_effect(rt);
		fns.scope_end_render(rt, prev);

		assert(sig2, sig, "signal hook stable");
		assert(eff0b, eff0, "first effect hook stable");
		assert(mem2, mem, "memo hook stable");
		assert(eff1b, eff1, "second effect hook stable");

		destroyRt(fns, rt);
	}

	// ── Destroy while pending — no crash ────────────────────────────
	suite("Effect — destroy while pending is safe");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		fns.signal_write_i32(rt, sig, 1);
		assert(fns.effect_is_pending(rt, e0), 1, "pending before destroy");

		// Destroy while pending — should not crash
		fns.effect_destroy(rt, e0);
		assert(fns.effect_count(rt), 0, "count is 0 after destroy");

		// Writing to signal should not crash
		fns.signal_write_i32(rt, sig, 2);

		destroyRt(fns, rt);
	}

	// ── Multiple writes produce single pending ──────────────────────
	suite("Effect — multiple writes single pending");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);
		fns.effect_begin_run(rt, e0);
		fns.signal_read_i32(rt, sig);
		fns.effect_end_run(rt, e0);

		fns.signal_write_i32(rt, sig, 1);
		fns.signal_write_i32(rt, sig, 2);
		fns.signal_write_i32(rt, sig, 3);

		assert(fns.effect_is_pending(rt, e0), 1, "effect is pending");
		assert(fns.effect_drain_pending(rt), 1, "only one pending entry");

		destroyRt(fns, rt);
	}

	// ── Run / re-subscribe cycle ────────────────────────────────────
	suite("Effect — run/re-subscribe cycle works repeatedly");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);
		const sig = fns.signal_create_i32(rt, 0);

		const e0 = fns.effect_create(rt, scopeId);

		for (let i = 1; i <= 5; i++) {
			// Run (subscribe)
			fns.effect_begin_run(rt, e0);
			fns.signal_read_i32(rt, sig);
			fns.effect_end_run(rt, e0);
			assert(fns.effect_is_pending(rt, e0), 0, `not pending after run #${i}`);

			// Write (trigger pending)
			fns.signal_write_i32(rt, sig, i);
			assert(fns.effect_is_pending(rt, e0), 1, `pending after write #${i}`);
		}

		destroyRt(fns, rt);
	}

	// ── Shell Effect Helpers ────────────────────────────────────────

	suite("Effect — shell create and pending");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);

		const e0 = fns.shell_effect_create(shell, scopeId);
		assert(e0 >= 0, true, "shell effect ID is non-negative");
		assert(
			fns.shell_effect_is_pending(shell, e0),
			1,
			"shell effect starts pending",
		);

		fns.shell_effect_begin_run(shell, e0);
		fns.shell_effect_end_run(shell, e0);
		assert(
			fns.shell_effect_is_pending(shell, e0),
			0,
			"shell effect not pending after run",
		);

		fns.shell_destroy(shell);
	}

	suite("Effect — shell signal write marks effect pending");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);
		const sig = fns.shell_create_signal_i32(shell, 0);

		const e0 = fns.shell_effect_create(shell, scopeId);

		// Run effect reading signal to subscribe
		fns.shell_effect_begin_run(shell, e0);
		fns.shell_read_signal_i32(shell, sig);
		fns.shell_effect_end_run(shell, e0);
		assert(fns.shell_effect_is_pending(shell, e0), 0, "not pending after run");

		// Write to signal
		fns.shell_write_signal_i32(shell, sig, 99);
		assert(
			fns.shell_effect_is_pending(shell, e0),
			1,
			"pending after shell signal write",
		);

		fns.shell_destroy(shell);
	}

	suite("Effect — shell use_effect hook lifecycle");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);

		// First render
		let prev = fns.shell_begin_render(shell, scopeId);
		const eid1 = fns.shell_use_effect(shell);
		fns.shell_end_render(shell, prev);

		assert(eid1 >= 0, true, "shell hook effect ID non-negative");
		assert(
			fns.shell_effect_is_pending(shell, eid1),
			1,
			"shell hook effect starts pending",
		);

		// Re-render
		prev = fns.shell_begin_render(shell, scopeId);
		const eid2 = fns.shell_use_effect(shell);
		fns.shell_end_render(shell, prev);

		assert(eid1, eid2, "shell hook returns same effect ID on re-render");

		fns.shell_destroy(shell);
	}

	suite("Effect — shell drain pending effects");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);
		const sig = fns.shell_create_signal_i32(shell, 0);

		const e0 = fns.shell_effect_create(shell, scopeId);
		const e1 = fns.shell_effect_create(shell, scopeId);

		// Both start pending
		assert(
			fns.shell_effect_drain_pending(shell),
			2,
			"both shell effects start pending",
		);

		// Run both subscribing to signal
		fns.shell_effect_begin_run(shell, e0);
		fns.shell_read_signal_i32(shell, sig);
		fns.shell_effect_end_run(shell, e0);
		fns.shell_effect_begin_run(shell, e1);
		fns.shell_read_signal_i32(shell, sig);
		fns.shell_effect_end_run(shell, e1);

		assert(
			fns.shell_effect_drain_pending(shell),
			0,
			"no shell effects pending after running both",
		);

		// Write → both pending
		fns.shell_write_signal_i32(shell, sig, 1);
		assert(
			fns.shell_effect_drain_pending(shell),
			2,
			"both shell effects pending after write",
		);

		// Verify pending_at
		const p0 = fns.shell_effect_pending_at(shell, 0);
		const p1 = fns.shell_effect_pending_at(shell, 1);
		assert(
			(p0 === e0 && p1 === e1) || (p0 === e1 && p1 === e0),
			true,
			"shell pending list contains both effect IDs",
		);

		fns.shell_destroy(shell);
	}

	suite("Effect — shell parity with raw runtime");
	{
		const shell = fns.shell_create();
		const rt = fns.shell_rt_ptr(shell);
		const scopeId = fns.shell_create_root_scope(shell);

		// Create via shell
		const eid = fns.shell_effect_create(shell, scopeId);

		// Verify via raw runtime
		assert(fns.effect_count(rt), 1, "runtime sees 1 effect");
		assert(fns.effect_is_pending(rt, eid), 1, "runtime sees effect pending");

		// Run via shell
		fns.shell_effect_begin_run(shell, eid);
		fns.shell_effect_end_run(shell, eid);

		// Verify via runtime
		assert(
			fns.effect_is_pending(rt, eid),
			0,
			"runtime sees not pending after shell run",
		);

		fns.shell_destroy(shell);
	}
}
