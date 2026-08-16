# Phase 36.1 — Recursive Memo Propagation Tests.
#
# Validates that `write_signal` correctly propagates dirtiness through
# memo → memo chains to arbitrary depth using the worklist-based
# approach introduced in Phase 36.
#
# Tests cover:
#   - 2/3/4-level chains
#   - Scope and effect subscribers at the end of chains
#   - Diamond dependency patterns
#   - Already-dirty skip (cycle guard)
#   - Recompute clears dirty / recompute order matters
#   - Independent signal writes
#   - Re-subscription after recompute
#   - Destroyed memo in chain
#   - Mixed-type chains (I32 → Bool → String)
#   - String and Bool memos in various positions
#   - No-subscriber memo (no crash)
#   - Memo + scope and memo + effect mixed subscribers
#   - Regression: single-memo (no chain) still works
#
# Run with:
#   mojo test test/test_memo_propagation.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    args_ptr_i32_ptr,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    return Int(w[].call_i64("runtime_create", no_args()))


fn _destroy_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_destroy", args_ptr(rt))


fn _scope_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    height: Int32,
    parent: Int32,
) raises -> Int:
    return Int(
        w[].call_i32("scope_create", args_ptr_i32_i32(rt, height, parent))
    )


fn _scope_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, id: Int
) raises:
    w[].call_void("scope_destroy", args_ptr_i32(rt, Int32(id)))


fn _scope_begin_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, scope_id: Int
) raises -> Int:
    return Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, Int32(scope_id)))
    )


fn _scope_end_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, prev: Int
) raises:
    w[].call_void("scope_end_render", args_ptr_i32(rt, Int32(prev)))


fn _signal_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, initial: Int32
) raises -> Int:
    return Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, initial)))


fn _signal_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    return Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, Int32(key))))


fn _signal_write(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    key: Int,
    value: Int32,
) raises:
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, Int32(key), value))


fn _signal_subscriber_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    return Int(
        w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, Int32(key)))
    )


fn _memo_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: Int32,
) raises -> Int:
    return Int(
        w[].call_i32(
            "memo_create_i32",
            args_ptr_i32_i32(rt, Int32(scope_id), initial),
        )
    )


fn _memo_begin_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_begin_compute", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_end_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Int32,
) raises:
    w[].call_void(
        "memo_end_compute_i32",
        args_ptr_i32_i32(rt, Int32(memo_id), value),
    )


fn _memo_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    return Int(w[].call_i32("memo_read_i32", args_ptr_i32(rt, Int32(memo_id))))


fn _memo_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    return w[].call_i32("memo_is_dirty", args_ptr_i32(rt, Int32(memo_id))) != 0


fn _memo_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_destroy", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    return Int(w[].call_i32("memo_count", args_ptr(rt)))


fn _memo_output_key(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    return Int(
        w[].call_i32("memo_output_key", args_ptr_i32(rt, Int32(memo_id)))
    )


fn _memo_context_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    return Int(
        w[].call_i32("memo_context_id", args_ptr_i32(rt, Int32(memo_id)))
    )


fn _memo_bool_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: Bool,
) raises -> Int:
    var init_i32: Int32
    if initial:
        init_i32 = Int32(1)
    else:
        init_i32 = Int32(0)
    return Int(
        w[].call_i32(
            "memo_bool_create",
            args_ptr_i32_i32(rt, Int32(scope_id), init_i32),
        )
    )


fn _memo_bool_begin_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_bool_begin_compute", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_bool_end_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Bool,
) raises:
    var val_i32: Int32
    if value:
        val_i32 = Int32(1)
    else:
        val_i32 = Int32(0)
    w[].call_void(
        "memo_bool_end_compute",
        args_ptr_i32_i32(rt, Int32(memo_id), val_i32),
    )


fn _memo_bool_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    return w[].call_i32("memo_bool_read", args_ptr_i32(rt, Int32(memo_id))) != 0


fn _memo_bool_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    return (
        w[].call_i32("memo_bool_is_dirty", args_ptr_i32(rt, Int32(memo_id)))
        != 0
    )


fn _memo_string_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: String,
) raises -> Int:
    var in_ptr = w[].write_string_struct(initial)
    return Int(
        w[].call_i32(
            "memo_string_create",
            args_ptr_i32_ptr(rt, Int32(scope_id), in_ptr),
        )
    )


fn _memo_string_begin_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_string_begin_compute", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_string_end_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: String,
) raises:
    var str_ptr = w[].write_string_struct(value)
    w[].call_void(
        "memo_string_end_compute",
        args_ptr_i32_ptr(rt, Int32(memo_id), str_ptr),
    )


fn _memo_string_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "memo_string_read", args_ptr_i32_ptr(rt, Int32(memo_id), out_ptr)
    )
    return w[].read_string_struct(out_ptr)


fn _memo_string_peek(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "memo_string_peek", args_ptr_i32_ptr(rt, Int32(memo_id), out_ptr)
    )
    return w[].read_string_struct(out_ptr)


fn _memo_string_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    return (
        w[].call_i32("memo_string_is_dirty", args_ptr_i32(rt, Int32(memo_id)))
        != 0
    )


fn _drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    return Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))


fn _effect_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
) raises -> Int:
    return Int(w[].call_i32("effect_create", args_ptr_i32(rt, Int32(scope_id))))


fn _effect_begin_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    w[].call_void("effect_begin_run", args_ptr_i32(rt, Int32(effect_id)))


fn _effect_end_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    w[].call_void("effect_end_run", args_ptr_i32(rt, Int32(effect_id)))


fn _effect_is_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises -> Bool:
    return (
        w[].call_i32("effect_is_pending", args_ptr_i32(rt, Int32(effect_id)))
        != 0
    )


fn _effect_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    w[].call_void("effect_destroy", args_ptr_i32(rt, Int32(effect_id)))


# ── Chain setup helpers ──────────────────────────────────────────────────────
# These helpers create memo chains and perform initial computation to
# establish subscriptions.  After setup, all memos are clean.


fn _setup_chain_2(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    signal_key: Int,
) raises -> Tuple[Int, Int]:
    """Create signal → memoA → memoB chain.  Returns (memoA, memoB).

    After setup both memos are clean with subscriptions established.
    memoA = signal * 2, memoB = memoA + 10.
    """
    var mA = _memo_create(w, rt, scope_id, 0)
    var mB = _memo_create(w, rt, scope_id, 0)

    # Compute memoA: reads signal → subscribes A's context to signal
    _memo_begin_compute(w, rt, mA)
    var v = _signal_read(w, rt, signal_key)
    _memo_end_compute(w, rt, mA, Int32(v * 2))

    # Compute memoB: reads memoA → subscribes B's context to A's output
    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, Int32(va + 10))

    # Drain any dirty scopes from initial computation
    _ = _drain_dirty(w, rt)

    return Tuple(mA, mB)


fn _setup_chain_3(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    signal_key: Int,
) raises -> Tuple[Int, Int, Int]:
    """Create signal → A → B → C chain.  Returns (A, B, C).

    A = signal * 2, B = A + 10, C = B + 100.
    """
    var mA = _memo_create(w, rt, scope_id, 0)
    var mB = _memo_create(w, rt, scope_id, 0)
    var mC = _memo_create(w, rt, scope_id, 0)

    _memo_begin_compute(w, rt, mA)
    var v = _signal_read(w, rt, signal_key)
    _memo_end_compute(w, rt, mA, Int32(v * 2))

    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, Int32(va + 10))

    _memo_begin_compute(w, rt, mC)
    var vb = _memo_read(w, rt, mB)
    _memo_end_compute(w, rt, mC, Int32(vb + 100))

    _ = _drain_dirty(w, rt)

    return Tuple(mA, mB, mC)


# ── 1. test_chain_2_level ────────────────────────────────────────────────────


fn test_chain_2_level(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Chain 2-level: signal → memoA → memoB.  Write signal, both dirty."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_2(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]

    assert_false(_memo_is_dirty(w, rt, mA), "A clean before write")
    assert_false(_memo_is_dirty(w, rt, mB), "B clean before write")

    _signal_write(w, rt, sig, 5)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty after write")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty after write (propagated)")

    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 2. test_chain_3_level ────────────────────────────────────────────────────


fn test_chain_3_level(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Chain 3-level: signal → A → B → C.  Write signal, all three dirty."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_3(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]
    var mC = chain[2]

    _signal_write(w, rt, sig, 3)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty (propagated)")
    assert_true(_memo_is_dirty(w, rt, mC), "C dirty (propagated)")

    _memo_destroy(w, rt, mC)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 3. test_chain_4_level ────────────────────────────────────────────────────


fn test_chain_4_level(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Chain 4-level: signal → A → B → C → D.  Write signal, all four dirty."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)
    var mC = _memo_create(w, rt, s0, 0)
    var mD = _memo_create(w, rt, s0, 0)

    _memo_begin_compute(w, rt, mA)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v * 2))

    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, Int32(va + 1))

    _memo_begin_compute(w, rt, mC)
    var vb = _memo_read(w, rt, mB)
    _memo_end_compute(w, rt, mC, Int32(vb + 1))

    _memo_begin_compute(w, rt, mD)
    var vc = _memo_read(w, rt, mC)
    _memo_end_compute(w, rt, mD, Int32(vc + 1))

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 10)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty")
    assert_true(_memo_is_dirty(w, rt, mC), "C dirty")
    assert_true(_memo_is_dirty(w, rt, mD), "D dirty")

    _memo_destroy(w, rt, mD)
    _memo_destroy(w, rt, mC)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 4. test_chain_scope_at_end ───────────────────────────────────────────────


fn test_chain_scope_at_end(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Scope at end of chain: signal → A → B, scope subscribes to B's output.

    Write signal: A dirty, B dirty, scope in dirty_scopes.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_2(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]

    # Subscribe a scope to memoB's output by doing a scope render
    # that reads memoB.
    var s1 = _scope_create(w, rt, 1, Int32(s0))
    var prev = _scope_begin_render(w, rt, s1)
    _ = _memo_read(w, rt, mB)
    _scope_end_render(w, rt, prev)

    _signal_write(w, rt, sig, 7)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty")
    var n_dirty = _drain_dirty(w, rt)
    assert_true(n_dirty > 0, "scope(s) are dirty")

    _scope_destroy(w, rt, s1)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 5. test_chain_effect_at_end ──────────────────────────────────────────────


fn test_chain_effect_at_end(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Effect at end of chain: signal → A → B, effect subscribes to B's output.

    Write signal: A dirty, B dirty, effect pending.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_2(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]

    # Create effect and subscribe to memoB's output
    var eff = _effect_create(w, rt, s0)
    # Run the effect once to establish subscription: begin_run, read memoB, end_run
    _effect_begin_run(w, rt, eff)
    _ = _memo_read(w, rt, mB)
    _effect_end_run(w, rt, eff)

    assert_false(
        _effect_is_pending(w, rt, eff), "effect clean after initial run"
    )

    _signal_write(w, rt, sig, 9)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty")
    assert_true(_effect_is_pending(w, rt, eff), "effect pending (propagated)")

    _effect_destroy(w, rt, eff)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 6. test_diamond_2_inputs ─────────────────────────────────────────────────


fn test_diamond_2_inputs(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Diamond 2 inputs: signal → A, signal → B, C subscribes to both A and B outputs.

    Write signal: all three dirty.  C marked dirty only once.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)
    var mC = _memo_create(w, rt, s0, 0)

    # A reads signal
    _memo_begin_compute(w, rt, mA)
    var v1 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v1 + 1))

    # B reads signal
    _memo_begin_compute(w, rt, mB)
    var v2 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mB, Int32(v2 + 2))

    # C reads both A and B
    _memo_begin_compute(w, rt, mC)
    var va = _memo_read(w, rt, mA)
    var vb = _memo_read(w, rt, mB)
    _memo_end_compute(w, rt, mC, Int32(va + vb))

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 5)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty")
    assert_true(_memo_is_dirty(w, rt, mC), "C dirty (diamond)")

    _memo_destroy(w, rt, mC)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 7. test_diamond_deep ─────────────────────────────────────────────────────


fn test_diamond_deep(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Diamond deep: signal → A → B, signal → C → B (B has two parents).

    Write signal: A, B, C all dirty.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mC = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # A reads signal
    _memo_begin_compute(w, rt, mA)
    var v1 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v1 * 2))

    # C reads signal
    _memo_begin_compute(w, rt, mC)
    var v2 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mC, Int32(v2 * 3))

    # B reads both A and C
    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    var vc = _memo_read(w, rt, mC)
    _memo_end_compute(w, rt, mB, Int32(va + vc))

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 4)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mC), "C dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty (reached via both A and C)")

    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mC)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 8. test_chain_already_dirty_skip ─────────────────────────────────────────


fn test_chain_already_dirty_skip(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Already-dirty skip: signal → A → B.  B already dirty before write.

    Write signal: A dirty, B still dirty (no double processing).
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # Initial compute to establish subscriptions
    _memo_begin_compute(w, rt, mA)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v * 2))

    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, Int32(va + 10))

    _ = _drain_dirty(w, rt)

    # Now write signal to make A and B dirty
    _signal_write(w, rt, sig, 1)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty after first write")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty after first write")

    # Recompute A only (B stays dirty)
    _memo_begin_compute(w, rt, mA)
    var v2 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v2 * 2))

    assert_false(_memo_is_dirty(w, rt, mA), "A clean after recompute")
    assert_true(_memo_is_dirty(w, rt, mB), "B still dirty (not recomputed)")

    # Write signal again — A should be dirty, B already dirty (skipped)
    _signal_write(w, rt, sig, 2)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty after second write")
    assert_true(
        _memo_is_dirty(w, rt, mB), "B still dirty (already dirty, skipped)"
    )

    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 9. test_chain_recompute_clears_dirty ─────────────────────────────────────


fn test_chain_recompute_clears_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Recompute clears dirty: signal → A → B.  Write, recompute A then B.

    Assert both clean afterwards.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_2(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]

    _signal_write(w, rt, sig, 5)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty")

    # Recompute A
    _memo_begin_compute(w, rt, mA)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v * 2))

    assert_false(_memo_is_dirty(w, rt, mA), "A clean after recompute")
    assert_true(_memo_is_dirty(w, rt, mB), "B still dirty")

    # Recompute B
    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, Int32(va + 10))

    assert_false(_memo_is_dirty(w, rt, mB), "B clean after recompute")

    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 10. test_chain_recompute_order_matters ───────────────────────────────────


fn test_chain_recompute_order_matters(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Recompute order matters: signal → A → B.  Write, recompute A then B.

    B's value should reflect A's new output.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_2(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]

    # Initial: sig=0, A=0*2=0, B=0+10=10
    assert_equal(_memo_read(w, rt, mA), 0, "A initial value")
    assert_equal(_memo_read(w, rt, mB), 10, "B initial value")

    _signal_write(w, rt, sig, 7)

    # Recompute A: 7*2 = 14
    _memo_begin_compute(w, rt, mA)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(v * 2))

    assert_equal(_memo_read(w, rt, mA), 14, "A new value")

    # Recompute B: 14+10 = 24
    _memo_begin_compute(w, rt, mB)
    var va = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, Int32(va + 10))

    assert_equal(_memo_read(w, rt, mB), 24, "B reflects A's new output")

    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 11. test_chain_independent_write ─────────────────────────────────────────


fn test_chain_independent_write(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Independent signals: signal1 → A → C, signal2 → B.

    Write signal1: A and C dirty, B clean.
    Write signal2: B dirty.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig1 = _signal_create(w, rt, 0)
    var sig2 = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mC = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # A reads sig1
    _memo_begin_compute(w, rt, mA)
    _ = _signal_read(w, rt, sig1)
    _memo_end_compute(w, rt, mA, 0)

    # C reads A
    _memo_begin_compute(w, rt, mC)
    _ = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mC, 0)

    # B reads sig2
    _memo_begin_compute(w, rt, mB)
    _ = _signal_read(w, rt, sig2)
    _memo_end_compute(w, rt, mB, 0)

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig1, 1)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty (sig1)")
    assert_true(_memo_is_dirty(w, rt, mC), "C dirty (propagated from A)")
    assert_false(_memo_is_dirty(w, rt, mB), "B clean (different signal)")

    _signal_write(w, rt, sig2, 1)
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty (sig2)")

    _memo_destroy(w, rt, mC)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 12. test_chain_propagation_after_resubscribe ─────────────────────────────


fn test_chain_propagation_after_resubscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Re-subscription: signal1 → A → B.  Recompute A reading signal2.

    Write signal1: A NOT dirty (unsubscribed).
    Write signal2: A dirty, B dirty.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig1 = _signal_create(w, rt, 0)
    var sig2 = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # Initial: A reads sig1
    _memo_begin_compute(w, rt, mA)
    _ = _signal_read(w, rt, sig1)
    _memo_end_compute(w, rt, mA, 0)

    # B reads A
    _memo_begin_compute(w, rt, mB)
    _ = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, 0)

    _ = _drain_dirty(w, rt)

    # Recompute A reading sig2 instead (re-subscribe)
    _signal_write(w, rt, sig1, 1)  # makes A dirty
    _ = _drain_dirty(w, rt)

    _memo_begin_compute(w, rt, mA)
    var v2 = _signal_read(w, rt, sig2)  # now A subscribes to sig2
    _memo_end_compute(w, rt, mA, Int32(v2))

    # Recompute B to re-establish subscription to A's output
    _memo_begin_compute(w, rt, mB)
    _ = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, 0)

    _ = _drain_dirty(w, rt)

    # Write sig1 — A should NOT be dirty (unsubscribed from sig1)
    _signal_write(w, rt, sig1, 99)
    assert_false(
        _memo_is_dirty(w, rt, mA), "A not dirty (unsubscribed from sig1)"
    )
    assert_false(_memo_is_dirty(w, rt, mB), "B not dirty (A not dirty)")

    # Write sig2 — A should be dirty, B should propagate
    _signal_write(w, rt, sig2, 42)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty (subscribed to sig2)")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty (propagated from A)")

    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 13. test_chain_with_destroyed_memo ───────────────────────────────────────


fn test_chain_with_destroyed_memo(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroyed memo: signal → A → B.  Destroy B.  Write signal.  No crash."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var chain = _setup_chain_2(w, rt, s0, sig)
    var mA = chain[0]
    var mB = chain[1]

    _memo_destroy(w, rt, mB)

    # Write signal — A should be dirty, no crash from destroyed B
    _signal_write(w, rt, sig, 3)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty after write")

    # mB is destroyed — no assertions on it, just no crash
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 14. test_chain_mixed_types ───────────────────────────────────────────────


fn test_chain_mixed_types(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Mixed types: signal(Int32) → MemoI32 → MemoBool → MemoString.

    Mirrors MemoChainApp topology.  Write signal, all three dirty.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mI32 = _memo_create(w, rt, s0, 0)
    var mBool = _memo_bool_create(w, rt, s0, False)
    var mStr = _memo_string_create(w, rt, s0, String("init"))

    # MemoI32: reads signal
    _memo_begin_compute(w, rt, mI32)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mI32, Int32(v * 2))

    # MemoBool: reads MemoI32
    _memo_bool_begin_compute(w, rt, mBool)
    var vi32 = _memo_read(w, rt, mI32)
    _memo_bool_end_compute(w, rt, mBool, vi32 >= 10)

    # MemoString: reads MemoBool
    _memo_string_begin_compute(w, rt, mStr)
    var b = _memo_bool_read(w, rt, mBool)
    if b:
        _memo_string_end_compute(w, rt, mStr, String("BIG"))
    else:
        _memo_string_end_compute(w, rt, mStr, String("small"))

    _ = _drain_dirty(w, rt)

    assert_false(_memo_is_dirty(w, rt, mI32), "I32 clean before write")
    assert_false(_memo_bool_is_dirty(w, rt, mBool), "Bool clean before write")
    assert_false(
        _memo_string_is_dirty(w, rt, mStr), "String clean before write"
    )

    _signal_write(w, rt, sig, 5)

    assert_true(_memo_is_dirty(w, rt, mI32), "I32 dirty")
    assert_true(_memo_bool_is_dirty(w, rt, mBool), "Bool dirty (propagated)")
    assert_true(_memo_string_is_dirty(w, rt, mStr), "String dirty (propagated)")

    _memo_destroy(w, rt, mStr)
    _memo_destroy(w, rt, mBool)
    _memo_destroy(w, rt, mI32)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 15. test_chain_string_memo_at_end ────────────────────────────────────────


fn test_chain_string_memo_at_end(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """String memo at end: signal → MemoI32 → MemoString.

    Write signal, both dirty.  Recompute both, MemoString correct.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mI32 = _memo_create(w, rt, s0, 0)
    var mStr = _memo_string_create(w, rt, s0, String("zero"))

    _memo_begin_compute(w, rt, mI32)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mI32, Int32(v))

    _memo_string_begin_compute(w, rt, mStr)
    var vi = _memo_read(w, rt, mI32)
    _memo_string_end_compute(w, rt, mStr, String("val:") + String(vi))

    _ = _drain_dirty(w, rt)

    assert_equal(
        _memo_string_peek(w, rt, mStr), String("val:0"), "initial string"
    )

    _signal_write(w, rt, sig, 42)

    assert_true(_memo_is_dirty(w, rt, mI32), "I32 dirty")
    assert_true(_memo_string_is_dirty(w, rt, mStr), "String dirty")

    # Recompute both in order
    _memo_begin_compute(w, rt, mI32)
    var v2 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mI32, Int32(v2))

    _memo_string_begin_compute(w, rt, mStr)
    var vi2 = _memo_read(w, rt, mI32)
    _memo_string_end_compute(w, rt, mStr, String("val:") + String(vi2))

    assert_equal(
        _memo_string_peek(w, rt, mStr), String("val:42"), "updated string"
    )

    _memo_destroy(w, rt, mStr)
    _memo_destroy(w, rt, mI32)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 16. test_chain_bool_memo_in_middle ───────────────────────────────────────


fn test_chain_bool_memo_in_middle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Bool memo in middle: signal → MemoBool → MemoI32.

    Write signal, both dirty.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mBool = _memo_bool_create(w, rt, s0, False)
    var mI32 = _memo_create(w, rt, s0, 0)

    # MemoBool reads signal (> 5 → True)
    _memo_bool_begin_compute(w, rt, mBool)
    var v = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, mBool, v > 5)

    # MemoI32 reads MemoBool (True → 1, False → 0)
    _memo_begin_compute(w, rt, mI32)
    var b = _memo_bool_read(w, rt, mBool)
    if b:
        _memo_end_compute(w, rt, mI32, 1)
    else:
        _memo_end_compute(w, rt, mI32, 0)

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 10)

    assert_true(_memo_bool_is_dirty(w, rt, mBool), "Bool dirty")
    assert_true(_memo_is_dirty(w, rt, mI32), "I32 dirty (propagated)")

    _memo_destroy(w, rt, mI32)
    _memo_destroy(w, rt, mBool)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 17. test_chain_no_subscribers ────────────────────────────────────────────


fn test_chain_no_subscribers(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """No subscribers: signal → memoA (no subscribers on A's output).

    Write signal: A dirty, no crash (worklist processes A but finds
    no output subscribers).
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)

    _memo_begin_compute(w, rt, mA)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, 0)

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 1)
    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")

    # No crash from worklist processing A with 0 output subscribers
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 18. test_chain_memo_to_memo_and_scope ────────────────────────────────────


fn test_chain_memo_to_memo_and_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Memo + scope: signal → A; both scope AND memoB subscribe to A's output.

    Write signal: A dirty, B dirty, scope dirty.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # A reads signal
    _memo_begin_compute(w, rt, mA)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, 0)

    # B reads A
    _memo_begin_compute(w, rt, mB)
    _ = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, 0)

    # A scope also reads A's output
    var s1 = _scope_create(w, rt, 1, Int32(s0))
    var prev = _scope_begin_render(w, rt, s1)
    _ = _memo_read(w, rt, mA)
    _scope_end_render(w, rt, prev)

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 1)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty (memo subscriber)")
    var n_dirty = _drain_dirty(w, rt)
    assert_true(n_dirty > 0, "scope dirty (scope subscriber on A's output)")

    _scope_destroy(w, rt, s1)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 19. test_chain_memo_to_memo_and_effect ───────────────────────────────────


fn test_chain_memo_to_memo_and_effect(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Memo + effect: signal → A; both effect AND memoB subscribe to A's output.

    Write signal: A dirty, B dirty, effect pending.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # A reads signal
    _memo_begin_compute(w, rt, mA)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, 0)

    # B reads A
    _memo_begin_compute(w, rt, mB)
    _ = _memo_read(w, rt, mA)
    _memo_end_compute(w, rt, mB, 0)

    # Effect reads A
    var eff = _effect_create(w, rt, s0)
    _effect_begin_run(w, rt, eff)
    _ = _memo_read(w, rt, mA)
    _effect_end_run(w, rt, eff)

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 1)

    assert_true(_memo_is_dirty(w, rt, mA), "A dirty")
    assert_true(_memo_is_dirty(w, rt, mB), "B dirty (memo subscriber)")
    assert_true(
        _effect_is_pending(w, rt, eff), "effect pending (effect subscriber)"
    )

    _effect_destroy(w, rt, eff)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 20. test_regression_single_memo ──────────────────────────────────────────


fn test_regression_single_memo(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Regression single memo: signal → memo (no chain), scope on output.

    Write signal: memo dirty, scope dirty.
    Verifies refactored code doesn't break single-level case.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, 0)

    var mA = _memo_create(w, rt, s0, 0)

    # Compute A to establish subscription to signal
    _memo_begin_compute(w, rt, mA)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, 0)

    # A scope reads A's output
    var s1 = _scope_create(w, rt, 1, Int32(s0))
    var prev = _scope_begin_render(w, rt, s1)
    _ = _memo_read(w, rt, mA)
    _scope_end_render(w, rt, prev)

    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 5)

    assert_true(_memo_is_dirty(w, rt, mA), "memo dirty")
    var n_dirty = _drain_dirty(w, rt)
    assert_true(n_dirty > 0, "scope dirty (via memo output)")

    _scope_destroy(w, rt, s1)
    _memo_destroy(w, rt, mA)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Runner ───────────────────────────────────────────────────────────────────


fn test_all(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    test_chain_2_level(w)
    test_chain_3_level(w)
    test_chain_4_level(w)
    test_chain_scope_at_end(w)
    test_chain_effect_at_end(w)
    test_diamond_2_inputs(w)
    test_diamond_deep(w)
    test_chain_already_dirty_skip(w)
    test_chain_recompute_clears_dirty(w)
    test_chain_recompute_order_matters(w)
    test_chain_independent_write(w)
    test_chain_propagation_after_resubscribe(w)
    test_chain_with_destroyed_memo(w)
    test_chain_mixed_types(w)
    test_chain_string_memo_at_end(w)
    test_chain_bool_memo_in_middle(w)
    test_chain_no_subscribers(w)
    test_chain_memo_to_memo_and_scope(w)
    test_chain_memo_to_memo_and_effect(w)
    test_regression_single_memo(w)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_all(w)
    print("memo_propagation: 20/20 passed")
