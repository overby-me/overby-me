# MemoBool (computed/derived Bool signal) operations exercised through the
# real WASM binary via mojo-wasmtime (pure Mojo FFI bindings — no Python).
#
# These tests verify that MemoBool creation, dirty tracking, computation,
# dependency auto-tracking, scope cleanup, and propagation chains work
# correctly when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_memo_bool.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    """Create a heap-allocated Runtime via WASM."""
    return Int(w[].call_i64("runtime_create", no_args()))


fn _destroy_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    """Destroy a heap-allocated Runtime via WASM."""
    w[].call_void("runtime_destroy", args_ptr(rt))


fn _scope_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    height: Int32,
    parent: Int32,
) raises -> Int:
    """Create a scope and return its ID as Int."""
    return Int(
        w[].call_i32("scope_create", args_ptr_i32_i32(rt, height, parent))
    )


fn _scope_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, id: Int
) raises:
    """Destroy a scope."""
    w[].call_void("scope_destroy", args_ptr_i32(rt, Int32(id)))


fn _signal_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, initial: Int32
) raises -> Int:
    """Create an Int32 signal and return its key as Int."""
    return Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, initial)))


fn _signal_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    """Read an Int32 signal (with context tracking)."""
    return Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, Int32(key))))


fn _signal_write(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    key: Int,
    value: Int32,
) raises:
    """Write to an Int32 signal."""
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, Int32(key), value))


fn _signal_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises:
    """Destroy a signal."""
    w[].call_void("signal_destroy", args_ptr_i32(rt, Int32(key)))


fn _signal_contains(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Bool:
    """Check whether a signal key is live."""
    return w[].call_i32("signal_contains", args_ptr_i32(rt, Int32(key))) != 0


fn _memo_bool_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: Bool,
) raises -> Int:
    """Create a Bool memo and return its ID as Int."""
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
    """Begin Bool memo computation."""
    w[].call_void("memo_bool_begin_compute", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_bool_end_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Bool,
) raises:
    """End Bool memo computation and store the result."""
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
    """Read the Bool memo's cached value."""
    return w[].call_i32("memo_bool_read", args_ptr_i32(rt, Int32(memo_id))) != 0


fn _memo_bool_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    """Check whether the Bool memo needs recomputation."""
    return (
        w[].call_i32("memo_bool_is_dirty", args_ptr_i32(rt, Int32(memo_id)))
        != 0
    )


fn _memo_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    """Check whether any memo needs recomputation (type-agnostic)."""
    return w[].call_i32("memo_is_dirty", args_ptr_i32(rt, Int32(memo_id))) != 0


fn _memo_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    """Destroy a memo."""
    w[].call_void("memo_destroy", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Return the number of live memos."""
    return Int(w[].call_i32("memo_count", args_ptr(rt)))


fn _memo_output_key(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    """Return the memo's output signal key."""
    return Int(
        w[].call_i32("memo_output_key", args_ptr_i32(rt, Int32(memo_id)))
    )


fn _memo_context_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    """Return the memo's reactive context ID."""
    return Int(
        w[].call_i32("memo_context_id", args_ptr_i32(rt, Int32(memo_id)))
    )


fn _scope_begin_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, scope_id: Int
) raises -> Int:
    """Begin scope render and return prev scope."""
    return Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, Int32(scope_id)))
    )


fn _scope_end_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, prev: Int
) raises:
    """End scope render."""
    w[].call_void("scope_end_render", args_ptr_i32(rt, Int32(prev)))


fn _drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Drain the dirty scope queue and return the count."""
    return Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))


# ── 1. mb_create_returns_valid_id ────────────────────────────────────────────


fn test_mb_create_returns_valid_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """memo ID is valid after creation."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    assert_equal(_memo_count(w, rt), 0, "no memos initially")

    var m0 = _memo_bool_create(w, rt, s0, False)
    assert_true(m0 >= 0, "memo ID is non-negative")
    assert_equal(_memo_count(w, rt), 1, "memo count is 1 after create")

    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "memo count is 0 after destroy")

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 2. mb_starts_dirty ───────────────────────────────────────────────────────


fn test_mb_starts_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """initial dirty flag is True."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, False)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "memo is dirty after creation")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 3. mb_initial_value ──────────────────────────────────────────────────────


fn test_mb_initial_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """peek returns initial value."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m_false = _memo_bool_create(w, rt, s0, False)
    assert_false(
        _memo_bool_read(w, rt, m_false), "initial False value readable"
    )

    var m_true = _memo_bool_create(w, rt, s0, True)
    assert_true(_memo_bool_read(w, rt, m_true), "initial True value readable")

    _memo_destroy(w, rt, m_false)
    _memo_destroy(w, rt, m_true)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 4. mb_compute_stores_value ───────────────────────────────────────────────


fn test_mb_compute_stores_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """begin/end compute stores True."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, False)
    assert_false(_memo_bool_read(w, rt, m0), "initial value is False")

    _memo_bool_begin_compute(w, rt, m0)
    _memo_bool_end_compute(w, rt, m0, True)

    assert_true(
        _memo_bool_read(w, rt, m0), "cached value is True after compute"
    )

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 5. mb_compute_clears_dirty ───────────────────────────────────────────────


fn test_mb_compute_clears_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """dirty cleared after compute."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, False)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "dirty before first compute")

    _memo_bool_begin_compute(w, rt, m0)
    _memo_bool_end_compute(w, rt, m0, True)

    assert_false(_memo_bool_is_dirty(w, rt, m0), "clean after compute")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 6. mb_signal_write_marks_dirty ───────────────────────────────────────────


fn test_mb_signal_write_marks_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """writing subscribed signal dirties memo."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 10)
    var m0 = _memo_bool_create(w, rt, s0, False)

    # Compute: read signal inside bracket → auto-subscribe
    _memo_bool_begin_compute(w, rt, m0)
    var val = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, val > 0)

    assert_true(_memo_bool_read(w, rt, m0), "computed True (10 > 0)")
    assert_false(_memo_bool_is_dirty(w, rt, m0), "clean after compute")

    # Write to input signal
    _signal_write(w, rt, sig, 0)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "dirty after signal write")

    # Recompute
    _memo_bool_begin_compute(w, rt, m0)
    var val2 = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, val2 > 0)

    assert_false(_memo_bool_read(w, rt, m0), "recomputed False (0 > 0)")
    assert_false(_memo_bool_is_dirty(w, rt, m0), "clean after recompute")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 7. mb_read_subscribes_context ────────────────────────────────────────────


fn test_mb_read_subscribes_context(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """reading in context subscribes scope to memo output."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var m0 = _memo_bool_create(w, rt, s0, False)

    # Compute: subscribe memo to signal
    _memo_bool_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, v > 0)

    # Subscribe scope to memo's output (simulate render read)
    var prev = _scope_begin_render(w, rt, s0)
    _ = _memo_bool_read(w, rt, m0)
    _scope_end_render(w, rt, prev)

    # Drain any existing dirty
    _ = _drain_dirty(w, rt)

    # Write to input → should propagate to scope
    _signal_write(w, rt, sig, -1)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "memo dirty after sig write")

    var dirty_count = _drain_dirty(w, rt)
    assert_true(dirty_count >= 1, "scope dirty after propagation")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 8. mb_recompute_from_convenience ─────────────────────────────────────────


fn test_mb_recompute_from_convenience(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """single-call recompute via begin+end."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, False)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "dirty initially")

    # Simulate recompute_from: begin_compute + end_compute(True)
    _memo_bool_begin_compute(w, rt, m0)
    _memo_bool_end_compute(w, rt, m0, True)

    assert_true(_memo_bool_read(w, rt, m0), "value is True after recompute")
    assert_false(_memo_bool_is_dirty(w, rt, m0), "clean after recompute")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 9. mb_peek_does_not_subscribe ────────────────────────────────────────────


fn test_mb_peek_does_not_subscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """peek has no subscription side effects.

    We verify that reading via the output_key (peek) inside a scope
    render does NOT cause the scope to become dirty when the memo
    recomputes.  We do this by peeking the output signal directly
    (same as what MemoBool.peek() does) instead of using memo_bool_read.
    """
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 1)
    var m0 = _memo_bool_create(w, rt, s0, False)

    # Compute: subscribe memo to signal
    _memo_bool_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, v > 0)

    # "Peek" inside a scope render — read the output signal directly
    # WITHOUT using memo_bool_read (which subscribes)
    var prev = _scope_begin_render(w, rt, s0)
    var out_key = _memo_output_key(w, rt, m0)
    # We read the output signal via signal_peek (no context tracking)
    # — just verify the value is readable
    var peeked = w[].call_i32(
        "signal_peek_i32", args_ptr_i32(rt, Int32(out_key))
    )
    assert_equal(Int(peeked), 1, "peek output signal reads 1 (True)")
    _scope_end_render(w, rt, prev)

    # Drain existing dirty
    _ = _drain_dirty(w, rt)

    # Write to input signal — memo gets dirty
    _signal_write(w, rt, sig, 0)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "memo dirty after write")

    # But scope should NOT be dirty (we only peeked, not subscribed)
    var dirty_count = _drain_dirty(w, rt)
    assert_equal(
        dirty_count,
        0,
        "scope NOT dirty (peek doesn't subscribe)",
    )

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 10. mb_destroy_cleans_up ─────────────────────────────────────────────────


fn test_mb_destroy_cleans_up(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """memo count decremented and signals destroyed."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, True)
    assert_equal(_memo_count(w, rt), 1, "1 memo after create")

    var out_key = _memo_output_key(w, rt, m0)
    var ctx_id = _memo_context_id(w, rt, m0)
    assert_true(_signal_contains(w, rt, out_key), "output signal exists")
    assert_true(_signal_contains(w, rt, ctx_id), "context signal exists")

    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "0 memos after destroy")
    assert_false(_signal_contains(w, rt, out_key), "output signal destroyed")
    assert_false(_signal_contains(w, rt, ctx_id), "context signal destroyed")

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 11. mb_id_reuse_after_destroy ────────────────────────────────────────────


fn test_mb_id_reuse_after_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """freed memo ID is reused by the slab allocator."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, True)
    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "0 memos after destroy")

    var m1 = _memo_bool_create(w, rt, s0, False)
    assert_equal(m1, m0, "freed memo ID is reused")
    assert_false(_memo_bool_read(w, rt, m1), "reused slot has new value False")

    _memo_destroy(w, rt, m1)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 12. mb_multiple_memos_independent ────────────────────────────────────────


fn test_mb_multiple_memos_independent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """two memos don't interfere."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, False)
    var m1 = _memo_bool_create(w, rt, s0, True)
    assert_true(m0 != m1, "distinct IDs")
    assert_equal(_memo_count(w, rt), 2, "2 memos created")

    assert_false(_memo_bool_read(w, rt, m0), "m0 initial = False")
    assert_true(_memo_bool_read(w, rt, m1), "m1 initial = True")

    # Compute m0 to True
    _memo_bool_begin_compute(w, rt, m0)
    _memo_bool_end_compute(w, rt, m0, True)

    assert_true(_memo_bool_read(w, rt, m0), "m0 = True after compute")
    assert_true(_memo_bool_read(w, rt, m1), "m1 still True (unchanged)")

    # Compute m1 to False
    _memo_bool_begin_compute(w, rt, m1)
    _memo_bool_end_compute(w, rt, m1, False)

    assert_true(_memo_bool_read(w, rt, m0), "m0 still True")
    assert_false(_memo_bool_read(w, rt, m1), "m1 = False after compute")

    _memo_destroy(w, rt, m0)
    _memo_destroy(w, rt, m1)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 13. mb_dirty_propagates_through_chain ────────────────────────────────────


fn test_mb_dirty_propagates_through_chain(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """signal → memo_bool chain: writing signal dirties memo."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 100)
    var m0 = _memo_bool_create(w, rt, s0, False)

    # Compute: is sig > 50?
    _memo_bool_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, v > 50)

    assert_true(_memo_bool_read(w, rt, m0), "100 > 50 = True")
    assert_false(_memo_bool_is_dirty(w, rt, m0), "clean after compute")

    # Write 30 → should dirty memo
    _signal_write(w, rt, sig, 30)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "dirty after write(30)")

    # Recompute
    _memo_bool_begin_compute(w, rt, m0)
    var v2 = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, v2 > 50)

    assert_false(_memo_bool_read(w, rt, m0), "30 > 50 = False")
    assert_false(_memo_bool_is_dirty(w, rt, m0), "clean after recompute")

    # Write 60 → dirty again
    _signal_write(w, rt, sig, 60)
    assert_true(_memo_bool_is_dirty(w, rt, m0), "dirty after write(60)")

    _memo_bool_begin_compute(w, rt, m0)
    var v3 = _signal_read(w, rt, sig)
    _memo_bool_end_compute(w, rt, m0, v3 > 50)

    assert_true(_memo_bool_read(w, rt, m0), "60 > 50 = True")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 14. mb_str_conversion ────────────────────────────────────────────────────


fn test_mb_str_conversion(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Bool memo values read as 1/0 (True/False at WASM level)."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_bool_create(w, rt, s0, True)

    # Read raw i32 to verify the storage representation
    var out_key = _memo_output_key(w, rt, m0)
    var raw = Int(
        w[].call_i32("signal_peek_i32", args_ptr_i32(rt, Int32(out_key)))
    )
    assert_equal(raw, 1, "True stored as 1")

    _memo_bool_begin_compute(w, rt, m0)
    _memo_bool_end_compute(w, rt, m0, False)

    var raw2 = Int(
        w[].call_i32("signal_peek_i32", args_ptr_i32(rt, Int32(out_key)))
    )
    assert_equal(raw2, 0, "False stored as 0")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Test runner ──────────────────────────────────────────────────────────────


fn test_all(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    test_mb_create_returns_valid_id(w)
    test_mb_starts_dirty(w)
    test_mb_initial_value(w)
    test_mb_compute_stores_value(w)
    test_mb_compute_clears_dirty(w)
    test_mb_signal_write_marks_dirty(w)
    test_mb_read_subscribes_context(w)
    test_mb_recompute_from_convenience(w)
    test_mb_peek_does_not_subscribe(w)
    test_mb_destroy_cleans_up(w)
    test_mb_id_reuse_after_destroy(w)
    test_mb_multiple_memos_independent(w)
    test_mb_dirty_propagates_through_chain(w)
    test_mb_str_conversion(w)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_all(w)
    print("memo_bool: 14/14 passed")
