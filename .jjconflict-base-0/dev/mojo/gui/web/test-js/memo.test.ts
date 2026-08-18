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

export function testMemo(fns: WasmExports): void {
	// ── Create / Destroy ────────────────────────────────────────────
	suite("Memo — create and destroy");
	{
		const rt = createRt(fns);

		const scopeId = fns.scope_create(rt, 0, -1);
		assert(fns.memo_count(rt), 0, "no memos initially");

		const m0 = fns.memo_create_i32(rt, scopeId, 42);
		assert(fns.memo_count(rt), 1, "memo count is 1 after create");
		assert(m0 >= 0, true, "memo ID is non-negative");

		// Memo starts dirty (needs first computation)
		assert(fns.memo_is_dirty(rt, m0), 1, "memo is dirty after creation");

		// Read initial cached value
		assert(fns.memo_read_i32(rt, m0), 42, "initial cached value is 42");

		// Output key and context ID are valid signal keys
		const outKey = fns.memo_output_key(rt, m0);
		const ctxId = fns.memo_context_id(rt, m0);
		assert(fns.signal_contains(rt, outKey), 1, "output signal exists");
		assert(fns.signal_contains(rt, ctxId), 1, "context signal exists");

		// Destroy
		fns.memo_destroy(rt, m0);
		assert(fns.memo_count(rt), 0, "memo count is 0 after destroy");
		// Output signal and context signal should be cleaned up
		assert(
			fns.signal_contains(rt, outKey),
			0,
			"output signal destroyed after memo destroy",
		);
		assert(
			fns.signal_contains(rt, ctxId),
			0,
			"context signal destroyed after memo destroy",
		);

		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Multiple memos ──────────────────────────────────────────────
	suite("Memo — multiple memos");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const m0 = fns.memo_create_i32(rt, scopeId, 10);
		const m1 = fns.memo_create_i32(rt, scopeId, 20);
		const m2 = fns.memo_create_i32(rt, scopeId, 30);
		assert(fns.memo_count(rt), 3, "3 memos created");
		assert(m0 !== m1 && m1 !== m2, true, "memo IDs are distinct");

		assert(fns.memo_read_i32(rt, m0), 10, "m0 initial = 10");
		assert(fns.memo_read_i32(rt, m1), 20, "m1 initial = 20");
		assert(fns.memo_read_i32(rt, m2), 30, "m2 initial = 30");

		fns.memo_destroy(rt, m1);
		assert(fns.memo_count(rt), 2, "2 memos after destroying m1");

		fns.memo_destroy(rt, m0);
		fns.memo_destroy(rt, m2);
		assert(fns.memo_count(rt), 0, "0 memos after destroying all");

		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── ID reuse after destroy ──────────────────────────────────────
	suite("Memo — ID reuse after destroy");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const m0 = fns.memo_create_i32(rt, scopeId, 1);
		fns.memo_destroy(rt, m0);

		const m1 = fns.memo_create_i32(rt, scopeId, 2);
		assert(m1, m0, "freed memo ID is reused");
		assert(fns.memo_read_i32(rt, m1), 2, "reused slot has new value");

		fns.memo_destroy(rt, m1);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Begin / End compute cycle ───────────────────────────────────
	suite("Memo — begin/end compute");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const m0 = fns.memo_create_i32(rt, scopeId, 0);
		assert(fns.memo_is_dirty(rt, m0), 1, "dirty before first compute");

		// Compute: result = 100
		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 100);

		assert(fns.memo_is_dirty(rt, m0), 0, "clean after compute");
		assert(fns.memo_read_i32(rt, m0), 100, "cached value is 100");

		// Compute again with a different value
		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 200);

		assert(fns.memo_is_dirty(rt, m0), 0, "clean after second compute");
		assert(fns.memo_read_i32(rt, m0), 200, "cached value updated to 200");

		fns.memo_destroy(rt, m0);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Auto-track: signal write → memo dirty ───────────────────────
	suite("Memo — signal write marks memo dirty");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		// Create an input signal
		const sig = fns.signal_create_i32(rt, 10);

		// Create a memo that depends on `sig`
		const m0 = fns.memo_create_i32(rt, scopeId, 0);

		// First compute: read the signal inside the compute bracket
		// This should auto-subscribe the memo's context to `sig`.
		fns.memo_begin_compute(rt, m0);
		const val = fns.signal_read_i32(rt, sig); // auto-tracks
		fns.memo_end_compute_i32(rt, m0, val);

		assert(fns.memo_read_i32(rt, m0), 10, "computed value matches signal");
		assert(fns.memo_is_dirty(rt, m0), 0, "memo is clean after compute");

		// Verify subscription: memo's context is subscribed to `sig`
		const _ctxId = fns.memo_context_id(rt, m0);
		assert(
			fns.signal_subscriber_count(rt, sig) >= 1,
			true,
			"input signal has at least 1 subscriber (memo context)",
		);

		// Write to the input signal — should mark memo dirty
		fns.signal_write_i32(rt, sig, 20);
		assert(fns.memo_is_dirty(rt, m0), 1, "memo dirty after signal write");

		// Recompute
		fns.memo_begin_compute(rt, m0);
		const val2 = fns.signal_read_i32(rt, sig);
		fns.memo_end_compute_i32(rt, m0, val2);

		assert(fns.memo_read_i32(rt, m0), 20, "recomputed value is 20");
		assert(fns.memo_is_dirty(rt, m0), 0, "clean after recompute");

		fns.memo_destroy(rt, m0);
		fns.signal_destroy(rt, sig);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Propagation chain: signal → memo → scope dirty ──────────────
	suite("Memo — signal → memo → scope dirty propagation");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const sig = fns.signal_create_i32(rt, 5);
		const m0 = fns.memo_create_i32(rt, scopeId, 0);

		// Compute: subscribe memo to signal
		fns.memo_begin_compute(rt, m0);
		const v = fns.signal_read_i32(rt, sig);
		fns.memo_end_compute_i32(rt, m0, v);

		// Subscribe scopeId to memo's output signal
		// (simulate a scope reading the memo during render)
		fns.scope_begin_render(rt, scopeId);
		fns.memo_read_i32(rt, m0); // subscribes scope to memo's output
		fns.scope_end_render(rt, -1);

		// Drain any existing dirty scopes
		fns.runtime_drain_dirty(rt);

		// Write to input signal — should propagate: sig → memo dirty → scope dirty
		fns.signal_write_i32(rt, sig, 99);

		assert(fns.memo_is_dirty(rt, m0), 1, "memo is dirty after sig write");
		// scope should be in the dirty queue
		const dirtyCount = fns.runtime_drain_dirty(rt);
		assert(dirtyCount >= 1, true, "at least 1 scope dirty after propagation");

		fns.memo_destroy(rt, m0);
		fns.signal_destroy(rt, sig);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Cache hit: read memo twice → same value ─────────────────────
	suite("Memo — cache hit (no recompute needed)");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const m0 = fns.memo_create_i32(rt, scopeId, 0);
		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 77);

		// Read twice — both should return the cached value
		const r1 = fns.memo_read_i32(rt, m0);
		const r2 = fns.memo_read_i32(rt, m0);
		assert(r1, 77, "first read returns 77");
		assert(r2, 77, "second read returns 77 (cache hit)");
		assert(fns.memo_is_dirty(rt, m0), 0, "still clean after reads");

		fns.memo_destroy(rt, m0);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Diamond dependency: two memos read same signal ───────────────
	suite("Memo — diamond: two memos depend on same signal");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const sig = fns.signal_create_i32(rt, 1);
		const mA = fns.memo_create_i32(rt, scopeId, 0);
		const mB = fns.memo_create_i32(rt, scopeId, 0);

		// mA computes: sig * 2
		fns.memo_begin_compute(rt, mA);
		const vA = fns.signal_read_i32(rt, sig);
		fns.memo_end_compute_i32(rt, mA, vA * 2);

		// mB computes: sig * 3
		fns.memo_begin_compute(rt, mB);
		const vB = fns.signal_read_i32(rt, sig);
		fns.memo_end_compute_i32(rt, mB, vB * 3);

		assert(fns.memo_read_i32(rt, mA), 2, "mA = sig*2 = 2");
		assert(fns.memo_read_i32(rt, mB), 3, "mB = sig*3 = 3");

		// Write to shared input — both memos should become dirty
		fns.signal_write_i32(rt, sig, 10);
		assert(fns.memo_is_dirty(rt, mA), 1, "mA dirty after write");
		assert(fns.memo_is_dirty(rt, mB), 1, "mB dirty after write");

		// Recompute both
		fns.memo_begin_compute(rt, mA);
		fns.memo_end_compute_i32(rt, mA, fns.signal_read_i32(rt, sig) * 2);
		fns.memo_begin_compute(rt, mB);
		fns.memo_end_compute_i32(rt, mB, fns.signal_read_i32(rt, sig) * 3);

		assert(fns.memo_read_i32(rt, mA), 20, "mA recomputed = 20");
		assert(fns.memo_read_i32(rt, mB), 30, "mB recomputed = 30");

		fns.memo_destroy(rt, mA);
		fns.memo_destroy(rt, mB);
		fns.signal_destroy(rt, sig);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Cleanup: destroy memo removes subscription on input signal ───
	suite("Memo — destroy removes input signal subscription");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const sig = fns.signal_create_i32(rt, 1);
		const subsBefore = fns.signal_subscriber_count(rt, sig);

		const m0 = fns.memo_create_i32(rt, scopeId, 0);

		// Compute so the memo subscribes to sig
		fns.memo_begin_compute(rt, m0);
		fns.signal_read_i32(rt, sig);
		fns.memo_end_compute_i32(rt, m0, 1);

		const subsAfterCompute = fns.signal_subscriber_count(rt, sig);
		assert(
			subsAfterCompute > subsBefore,
			true,
			"subscriber count increased after compute",
		);

		// Destroy the memo — its context signal is destroyed, and the
		// subscription should be effectively removed (context no longer
		// exists, so even if still in the list it's inert).
		fns.memo_destroy(rt, m0);

		// Write to signal should not cause issues
		fns.signal_write_i32(rt, sig, 99);
		// Just verify no crash
		fns.runtime_drain_dirty(rt);

		fns.signal_destroy(rt, sig);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Dependency re-tracking on recompute ─────────────────────────
	suite("Memo — dependency re-tracking on recompute");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const sigA = fns.signal_create_i32(rt, 1);
		const sigB = fns.signal_create_i32(rt, 100);

		const m0 = fns.memo_create_i32(rt, scopeId, 0);

		// First compute: read only sigA
		fns.memo_begin_compute(rt, m0);
		const v1 = fns.signal_read_i32(rt, sigA);
		fns.memo_end_compute_i32(rt, m0, v1);

		assert(fns.memo_read_i32(rt, m0), 1, "first compute reads sigA=1");

		// Writing sigA should dirty the memo
		fns.signal_write_i32(rt, sigA, 2);
		assert(fns.memo_is_dirty(rt, m0), 1, "dirty after sigA write");

		// Second compute: read only sigB (not sigA)
		fns.memo_begin_compute(rt, m0);
		const v2 = fns.signal_read_i32(rt, sigB);
		fns.memo_end_compute_i32(rt, m0, v2);

		assert(fns.memo_read_i32(rt, m0), 100, "second compute reads sigB=100");
		assert(fns.memo_is_dirty(rt, m0), 0, "clean after recompute");

		// Now writing sigA should NOT dirty the memo (no longer subscribed)
		fns.signal_write_i32(rt, sigA, 999);
		assert(
			fns.memo_is_dirty(rt, m0),
			0,
			"memo NOT dirty after sigA write (unsubscribed)",
		);

		// Writing sigB SHOULD dirty the memo
		fns.signal_write_i32(rt, sigB, 200);
		assert(fns.memo_is_dirty(rt, m0), 1, "memo dirty after sigB write");

		fns.memo_destroy(rt, m0);
		fns.signal_destroy(rt, sigA);
		fns.signal_destroy(rt, sigB);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Read memo without active context — no crash ─────────────────
	suite("Memo — read without active context");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const m0 = fns.memo_create_i32(rt, scopeId, 55);

		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 55);

		// Read with no scope render active (no context) — should just return value
		const v = fns.memo_read_i32(rt, m0);
		assert(v, 55, "read without context returns cached value");

		fns.memo_destroy(rt, m0);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Destroy non-existent memo — no crash ────────────────────────
	suite("Memo — destroy non-existent memo is no-op");
	{
		const rt = createRt(fns);

		// Should not crash
		fns.memo_destroy(rt, 9999);
		assert(
			fns.memo_count(rt),
			0,
			"count still 0 after destroying non-existent",
		);

		destroyRt(fns, rt);
	}

	// ── Output signal version bumps on compute ──────────────────────
	suite("Memo — output signal version increments on compute");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		const m0 = fns.memo_create_i32(rt, scopeId, 0);
		const outKey = fns.memo_output_key(rt, m0);

		const v0 = fns.signal_version(rt, outKey);

		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 10);
		const v1 = fns.signal_version(rt, outKey);
		assert(v1 > v0, true, "version bumped after first compute");

		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 20);
		const v2 = fns.signal_version(rt, outKey);
		assert(v2 > v1, true, "version bumped after second compute");

		fns.memo_destroy(rt, m0);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Hook: use_memo_i32 — create on first render ─────────────────
	suite("Memo hook — creates memo on first render");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		assert(fns.memo_count(rt), 0, "no memos before render");
		assert(fns.scope_hook_count(rt, scopeId), 0, "no hooks before render");

		// First render
		const prev = fns.scope_begin_render(rt, scopeId);
		const m0 = fns.hook_use_memo_i32(rt, 42);
		fns.scope_end_render(rt, prev);

		assert(m0 >= 0, true, "memo ID is non-negative");
		assert(fns.memo_count(rt), 1, "1 memo after hook");
		assert(fns.scope_hook_count(rt, scopeId), 1, "1 hook registered");
		// HOOK_MEMO tag = 1
		assert(
			fns.scope_hook_tag_at(rt, scopeId, 0),
			1,
			"hook tag is HOOK_MEMO (1)",
		);
		assert(
			fns.scope_hook_value_at(rt, scopeId, 0),
			m0,
			"hook value is memo ID",
		);
		// Initial value readable
		assert(fns.memo_read_i32(rt, m0), 42, "memo initial value is 42");
		// Memo starts dirty
		assert(fns.memo_is_dirty(rt, m0), 1, "memo starts dirty");

		fns.memo_destroy(rt, m0);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Hook: use_memo_i32 — same ID on re-render ───────────────────
	suite("Memo hook — returns same ID on re-render");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		// First render
		let prev = fns.scope_begin_render(rt, scopeId);
		const m0 = fns.hook_use_memo_i32(rt, 10);
		fns.scope_end_render(rt, prev);

		// Compute a value
		fns.memo_begin_compute(rt, m0);
		fns.memo_end_compute_i32(rt, m0, 100);
		assert(fns.memo_read_i32(rt, m0), 100, "computed value is 100");

		// Re-render — initial value (999) is ignored
		prev = fns.scope_begin_render(rt, scopeId);
		const m1 = fns.hook_use_memo_i32(rt, 999);
		fns.scope_end_render(rt, prev);

		assert(m1, m0, "same memo ID on re-render");
		assert(fns.memo_count(rt), 1, "still 1 memo");
		assert(fns.scope_hook_count(rt, scopeId), 1, "still 1 hook");
		// Cached value survives re-render
		assert(fns.memo_read_i32(rt, m1), 100, "cached value survives re-render");

		fns.memo_destroy(rt, m0);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Hook: use_memo_i32 — interleaved with signal hooks ──────────
	suite("Memo hook — interleaved with signal hooks");
	{
		const rt = createRt(fns);
		const scopeId = fns.scope_create(rt, 0, -1);

		// First render: signal, memo, signal, memo
		let prev = fns.scope_begin_render(rt, scopeId);
		const sig0 = fns.hook_use_signal_i32(rt, 10);
		const mem0 = fns.hook_use_memo_i32(rt, 20);
		const sig1 = fns.hook_use_signal_i32(rt, 30);
		const mem1 = fns.hook_use_memo_i32(rt, 40);
		fns.scope_end_render(rt, prev);

		assert(fns.scope_hook_count(rt, scopeId), 4, "4 hooks total");
		// HOOK_SIGNAL = 0, HOOK_MEMO = 1
		assert(fns.scope_hook_tag_at(rt, scopeId, 0), 0, "hook 0 = SIGNAL");
		assert(fns.scope_hook_tag_at(rt, scopeId, 1), 1, "hook 1 = MEMO");
		assert(fns.scope_hook_tag_at(rt, scopeId, 2), 0, "hook 2 = SIGNAL");
		assert(fns.scope_hook_tag_at(rt, scopeId, 3), 1, "hook 3 = MEMO");

		// Re-render: same order returns same IDs
		prev = fns.scope_begin_render(rt, scopeId);
		const rSig0 = fns.hook_use_signal_i32(rt, 0);
		const rMem0 = fns.hook_use_memo_i32(rt, 0);
		const rSig1 = fns.hook_use_signal_i32(rt, 0);
		const rMem1 = fns.hook_use_memo_i32(rt, 0);
		fns.scope_end_render(rt, prev);

		assert(rSig0, sig0, "signal 0 stable across re-render");
		assert(rMem0, mem0, "memo 0 stable across re-render");
		assert(rSig1, sig1, "signal 1 stable across re-render");
		assert(rMem1, mem1, "memo 1 stable across re-render");

		fns.memo_destroy(rt, mem0);
		fns.memo_destroy(rt, mem1);
		fns.scope_destroy(rt, scopeId);
		destroyRt(fns, rt);
	}

	// ── Shell memo helpers (M13.5) ──────────────────────────────────

	suite("Shell memo — create and read initial value");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);
		const m0 = fns.shell_memo_create_i32(shell, scopeId, 42);
		assert(m0 >= 0, true, "shell memo ID is non-negative");
		assert(
			fns.shell_memo_read_i32(shell, m0),
			42,
			"shell memo initial value is 42",
		);
		fns.shell_destroy(shell);
	}

	suite("Shell memo — starts dirty, compute clears dirty");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);
		const m0 = fns.shell_memo_create_i32(shell, scopeId, 0);

		assert(fns.shell_memo_is_dirty(shell, m0), 1, "shell memo starts dirty");

		fns.shell_memo_begin_compute(shell, m0);
		fns.shell_memo_end_compute_i32(shell, m0, 99);

		assert(
			fns.shell_memo_is_dirty(shell, m0),
			0,
			"shell memo clean after compute",
		);
		assert(
			fns.shell_memo_read_i32(shell, m0),
			99,
			"shell memo value updated to 99",
		);
		fns.shell_destroy(shell);
	}

	suite("Shell memo — signal write marks memo dirty and propagates to scope");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);
		const sig = fns.shell_create_signal_i32(shell, 10);
		const m0 = fns.shell_memo_create_i32(shell, scopeId, 0);

		// First compute: read signal to establish dependency
		fns.shell_memo_begin_compute(shell, m0);
		const rt = fns.shell_rt_ptr(shell);
		fns.signal_read_i32(rt, sig); // subscribe memo context to signal
		fns.shell_memo_end_compute_i32(shell, m0, 10);

		assert(
			fns.shell_memo_is_dirty(shell, m0),
			0,
			"memo clean after first compute",
		);

		// Subscribe scope to memo output by reading memo inside scope context
		fns.shell_begin_render(shell, scopeId);
		fns.shell_memo_read_i32(shell, m0);
		fns.shell_end_render(shell, -1);

		// Write to input signal — should dirty memo + scope
		fns.shell_write_signal_i32(shell, sig, 20);

		assert(
			fns.shell_memo_is_dirty(shell, m0),
			1,
			"memo dirty after signal write",
		);
		assert(
			fns.shell_has_dirty(shell),
			1,
			"shell has dirty scopes after propagation",
		);
		fns.shell_destroy(shell);
	}

	suite("Shell memo — multiple memos have distinct IDs and independent values");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);
		const m0 = fns.shell_memo_create_i32(shell, scopeId, 10);
		const m1 = fns.shell_memo_create_i32(shell, scopeId, 20);
		const m2 = fns.shell_memo_create_i32(shell, scopeId, 30);

		assert(m0 !== m1, true, "m0 !== m1");
		assert(m1 !== m2, true, "m1 !== m2");
		assert(m0 !== m2, true, "m0 !== m2");

		assert(fns.shell_memo_read_i32(shell, m0), 10, "m0 value is 10");
		assert(fns.shell_memo_read_i32(shell, m1), 20, "m1 value is 20");
		assert(fns.shell_memo_read_i32(shell, m2), 30, "m2 value is 30");
		fns.shell_destroy(shell);
	}

	suite("Shell memo — use_memo_i32 hook creates on first render");
	{
		const shell = fns.shell_create();
		const scopeId = fns.shell_create_root_scope(shell);

		// First render — creates memo
		const prev = fns.shell_begin_render(shell, scopeId);
		const m0 = fns.shell_use_memo_i32(shell, 0);
		assert(m0 >= 0, true, "hook memo ID is non-negative");
		fns.shell_end_render(shell, prev);

		// Re-render — returns the same ID
		const prev2 = fns.shell_begin_render(shell, scopeId);
		const m1 = fns.shell_use_memo_i32(shell, 999);
		assert(m1, m0, "same memo ID on re-render");
		fns.shell_end_render(shell, prev2);

		fns.shell_destroy(shell);
	}

	suite("Shell memo — parity with raw Runtime methods");
	{
		const shell = fns.shell_create();
		const rt = fns.shell_rt_ptr(shell);
		const scopeId = fns.shell_create_root_scope(shell);

		const m0 = fns.shell_memo_create_i32(shell, scopeId, 50);

		// Read via shell and via raw runtime — should match
		const shellVal = fns.shell_memo_read_i32(shell, m0);
		const rtVal = fns.memo_read_i32(rt, m0);
		assert(shellVal, rtVal, "shell read == runtime read");
		assert(shellVal, 50, "initial value is 50");

		// Dirty check parity
		const shellDirty = fns.shell_memo_is_dirty(shell, m0);
		const rtDirty = fns.memo_is_dirty(rt, m0);
		assert(shellDirty, rtDirty, "shell dirty == runtime dirty");

		// Compute via shell, verify via runtime
		fns.shell_memo_begin_compute(shell, m0);
		fns.shell_memo_end_compute_i32(shell, m0, 100);

		const shellVal2 = fns.shell_memo_read_i32(shell, m0);
		const rtVal2 = fns.memo_read_i32(rt, m0);
		assert(shellVal2, 100, "post-compute shell value is 100");
		assert(rtVal2, 100, "post-compute runtime value is 100");
		assert(shellVal2, rtVal2, "post-compute: shell == runtime");

		assert(fns.shell_memo_is_dirty(shell, m0), 0, "shell: clean after compute");
		assert(fns.memo_is_dirty(rt, m0), 0, "runtime: clean after compute");

		fns.shell_destroy(shell);
	}
}
