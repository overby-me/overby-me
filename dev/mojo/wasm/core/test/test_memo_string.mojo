# MemoString (computed/derived String signal) operations exercised through the
# real WASM binary via mojo-wasmtime (pure Mojo FFI bindings — no Python).
#
# These tests verify that MemoString creation, dirty tracking, computation,
# dependency auto-tracking, scope cleanup, string lifecycle, and propagation
# chains work correctly when compiled to WASM and executed via the Wasmtime
# runtime.
#
# Run with:
#   mojo test test/test_memo_string.mojo

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
    args_ptr_ptr,
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


fn _memo_string_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: String,
) raises -> Int:
    """Create a String memo and return its ID as Int."""
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
    """Begin String memo computation."""
    w[].call_void("memo_string_begin_compute", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_string_end_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: String,
) raises:
    """End String memo computation and store the result."""
    var str_ptr = w[].write_string_struct(value)
    w[].call_void(
        "memo_string_end_compute",
        args_ptr_i32_ptr(rt, Int32(memo_id), str_ptr),
    )


fn _memo_string_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> String:
    """Read the String memo's cached value (with context tracking)."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "memo_string_read", args_ptr_i32_ptr(rt, Int32(memo_id), out_ptr)
    )
    return w[].read_string_struct(out_ptr)


fn _memo_string_peek(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> String:
    """Read the String memo's cached value without subscribing."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "memo_string_peek", args_ptr_i32_ptr(rt, Int32(memo_id), out_ptr)
    )
    return w[].read_string_struct(out_ptr)


fn _memo_string_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    """Check whether the String memo needs recomputation."""
    return (
        w[].call_i32("memo_string_is_dirty", args_ptr_i32(rt, Int32(memo_id)))
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
    """Return the memo's output signal key (version signal for strings)."""
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


fn _memo_string_key(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    """Return the memo's StringStore key."""
    return Int(
        w[].call_i32("memo_string_key", args_ptr_i32(rt, Int32(memo_id)))
    )


fn _signal_string_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Return the number of live string signals in the StringStore."""
    return Int(w[].call_i32("signal_string_count", args_ptr(rt)))


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


# ── 1. ms_create_returns_valid_id ────────────────────────────────────────────


fn test_ms_create_returns_valid_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """memo ID is valid after creation."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    assert_equal(_memo_count(w, rt), 0, "no memos initially")

    var m0 = _memo_string_create(w, rt, s0, String(""))
    assert_true(m0 >= 0, "memo ID is non-negative")
    assert_equal(_memo_count(w, rt), 1, "memo count is 1 after create")

    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "memo count is 0 after destroy")

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 2. ms_starts_dirty ───────────────────────────────────────────────────────


fn test_ms_starts_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """initial dirty flag is True."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String("hello"))
    assert_true(
        _memo_string_is_dirty(w, rt, m0), "memo is dirty after creation"
    )

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 3. ms_initial_value ──────────────────────────────────────────────────────


fn test_ms_initial_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """peek returns initial string."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String("initial"))
    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("initial"),
        "peek returns initial string",
    )

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 4. ms_compute_stores_value ───────────────────────────────────────────────


fn test_ms_compute_stores_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """begin/end compute stores string."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String(""))
    assert_equal(
        _memo_string_peek(w, rt, m0), String(""), "initial value is empty"
    )

    _memo_string_begin_compute(w, rt, m0)
    _memo_string_end_compute(w, rt, m0, String("computed"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("computed"),
        "cached value is 'computed' after compute",
    )

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 5. ms_compute_clears_dirty ───────────────────────────────────────────────


fn test_ms_compute_clears_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """dirty cleared after compute."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String(""))
    assert_true(_memo_string_is_dirty(w, rt, m0), "dirty before first compute")

    _memo_string_begin_compute(w, rt, m0)
    _memo_string_end_compute(w, rt, m0, String("done"))

    assert_false(_memo_string_is_dirty(w, rt, m0), "clean after compute")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 6. ms_signal_write_marks_dirty ───────────────────────────────────────────


fn test_ms_signal_write_marks_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """writing subscribed signal dirties memo."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 10)
    var m0 = _memo_string_create(w, rt, s0, String(""))

    # Compute: read signal inside bracket -> auto-subscribe
    _memo_string_begin_compute(w, rt, m0)
    var val = _signal_read(w, rt, sig)
    _memo_string_end_compute(w, rt, m0, String("val=") + String(val))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("val=10"),
        "computed from signal=10",
    )
    assert_false(_memo_string_is_dirty(w, rt, m0), "clean after compute")

    # Write to input signal
    _signal_write(w, rt, sig, 20)
    assert_true(_memo_string_is_dirty(w, rt, m0), "dirty after signal write")

    # Recompute
    _memo_string_begin_compute(w, rt, m0)
    var val2 = _signal_read(w, rt, sig)
    _memo_string_end_compute(w, rt, m0, String("val=") + String(val2))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("val=20"),
        "recomputed from signal=20",
    )
    assert_false(_memo_string_is_dirty(w, rt, m0), "clean after recompute")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 7. ms_read_subscribes_context ────────────────────────────────────────────


fn test_ms_read_subscribes_context(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """reading in context subscribes via version signal."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var m0 = _memo_string_create(w, rt, s0, String(""))

    # Compute: subscribe memo to signal
    _memo_string_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    _memo_string_end_compute(w, rt, m0, String("v=") + String(v))

    # Subscribe scope to memo's output (simulate render read)
    var prev = _scope_begin_render(w, rt, s0)
    _ = _memo_string_read(w, rt, m0)
    _scope_end_render(w, rt, prev)

    # Drain any existing dirty
    _ = _drain_dirty(w, rt)

    # Write to input -> should propagate to scope
    _signal_write(w, rt, sig, -1)
    assert_true(_memo_string_is_dirty(w, rt, m0), "memo dirty after sig write")

    var dirty_count = _drain_dirty(w, rt)
    assert_true(dirty_count >= 1, "scope dirty after propagation")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 8. ms_recompute_from_convenience ─────────────────────────────────────────


fn test_ms_recompute_from_convenience(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """single-call recompute via begin+end."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String(""))
    assert_true(_memo_string_is_dirty(w, rt, m0), "dirty initially")

    # Simulate recompute_from: begin_compute + end_compute
    _memo_string_begin_compute(w, rt, m0)
    _memo_string_end_compute(w, rt, m0, String("recomputed"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("recomputed"),
        "value is 'recomputed' after recompute",
    )
    assert_false(_memo_string_is_dirty(w, rt, m0), "clean after recompute")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 9. ms_peek_does_not_subscribe ────────────────────────────────────────────


fn test_ms_peek_does_not_subscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """peek has no subscription side effects."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 1)
    var m0 = _memo_string_create(w, rt, s0, String(""))

    # Compute: subscribe memo to signal
    _memo_string_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    _memo_string_end_compute(w, rt, m0, String("v=") + String(v))

    # Peek inside a scope render — should NOT subscribe scope
    var prev = _scope_begin_render(w, rt, s0)
    var peeked = _memo_string_peek(w, rt, m0)
    assert_equal(peeked, String("v=1"), "peek returns cached value")
    _scope_end_render(w, rt, prev)

    # Drain existing dirty
    _ = _drain_dirty(w, rt)

    # Write to input signal — memo gets dirty
    _signal_write(w, rt, sig, 0)
    assert_true(_memo_string_is_dirty(w, rt, m0), "memo dirty after write")

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


# ── 10. ms_is_empty_when_empty ───────────────────────────────────────────────


fn test_ms_is_empty_when_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """is_empty returns True for empty string."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String(""))
    var val = _memo_string_peek(w, rt, m0)
    assert_equal(len(val), 0, "peek returns empty string")
    assert_true(len(val) == 0, "is_empty for empty string")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 11. ms_is_empty_when_not_empty ───────────────────────────────────────────


fn test_ms_is_empty_when_not_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """is_empty returns False for non-empty string."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String("hello"))
    var val = _memo_string_peek(w, rt, m0)
    assert_equal(val, String("hello"), "peek returns 'hello'")
    assert_false(len(val) == 0, "is_empty False for 'hello'")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 12. ms_destroy_cleans_up ─────────────────────────────────────────────────


fn test_ms_destroy_cleans_up(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """memo count decremented, string freed, signals destroyed."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var str_count_before = _signal_string_count(w, rt)
    var m0 = _memo_string_create(w, rt, s0, String("test"))
    assert_equal(_memo_count(w, rt), 1, "1 memo after create")

    var str_count_after = _signal_string_count(w, rt)
    assert_equal(
        str_count_after,
        str_count_before + 1,
        "StringStore count increased by 1",
    )

    var out_key = _memo_output_key(w, rt, m0)
    var ctx_id = _memo_context_id(w, rt, m0)
    var str_key = _memo_string_key(w, rt, m0)
    assert_true(
        str_key != Int(0xFFFFFFFF), "string_key is not MEMO_NO_STRING sentinel"
    )
    assert_true(_signal_contains(w, rt, out_key), "version signal exists")
    assert_true(_signal_contains(w, rt, ctx_id), "context signal exists")

    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "0 memos after destroy")
    assert_false(_signal_contains(w, rt, out_key), "version signal destroyed")
    assert_false(_signal_contains(w, rt, ctx_id), "context signal destroyed")

    var str_count_final = _signal_string_count(w, rt)
    assert_equal(
        str_count_final,
        str_count_before,
        "StringStore count restored after destroy",
    )

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 13. ms_id_reuse_after_destroy ────────────────────────────────────────────


fn test_ms_id_reuse_after_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """freed memo ID is reused by the slab allocator."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String("first"))
    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "0 memos after destroy")

    var m1 = _memo_string_create(w, rt, s0, String("second"))
    assert_equal(m1, m0, "freed memo ID is reused")
    assert_equal(
        _memo_string_peek(w, rt, m1),
        String("second"),
        "reused slot has new value 'second'",
    )

    _memo_destroy(w, rt, m1)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 14. ms_multiple_memos_independent ────────────────────────────────────────


fn test_ms_multiple_memos_independent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """two string memos don't interfere."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String("alpha"))
    var m1 = _memo_string_create(w, rt, s0, String("beta"))
    assert_true(m0 != m1, "distinct IDs")
    assert_equal(_memo_count(w, rt), 2, "2 memos created")

    assert_equal(
        _memo_string_peek(w, rt, m0), String("alpha"), "m0 initial = alpha"
    )
    assert_equal(
        _memo_string_peek(w, rt, m1), String("beta"), "m1 initial = beta"
    )

    # Compute m0 to "gamma"
    _memo_string_begin_compute(w, rt, m0)
    _memo_string_end_compute(w, rt, m0, String("gamma"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("gamma"),
        "m0 = gamma after compute",
    )
    assert_equal(
        _memo_string_peek(w, rt, m1),
        String("beta"),
        "m1 still beta (unchanged)",
    )

    # Compute m1 to "delta"
    _memo_string_begin_compute(w, rt, m1)
    _memo_string_end_compute(w, rt, m1, String("delta"))

    assert_equal(
        _memo_string_peek(w, rt, m0), String("gamma"), "m0 still gamma"
    )
    assert_equal(
        _memo_string_peek(w, rt, m1),
        String("delta"),
        "m1 = delta after compute",
    )

    _memo_destroy(w, rt, m0)
    _memo_destroy(w, rt, m1)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 15. ms_dirty_propagates_through_chain ────────────────────────────────────


fn test_ms_dirty_propagates_through_chain(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """signal -> memo_string chain: writing signal dirties memo."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 1)
    var m0 = _memo_string_create(w, rt, s0, String(""))

    # Compute: label from signal value
    _memo_string_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    if v > 0:
        _memo_string_end_compute(w, rt, m0, String("positive"))
    else:
        _memo_string_end_compute(w, rt, m0, String("non-positive"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("positive"),
        "1 -> 'positive'",
    )
    assert_false(_memo_string_is_dirty(w, rt, m0), "clean after compute")

    # Write 0 -> should dirty memo
    _signal_write(w, rt, sig, 0)
    assert_true(_memo_string_is_dirty(w, rt, m0), "dirty after write(0)")

    # Recompute
    _memo_string_begin_compute(w, rt, m0)
    var v2 = _signal_read(w, rt, sig)
    if v2 > 0:
        _memo_string_end_compute(w, rt, m0, String("positive"))
    else:
        _memo_string_end_compute(w, rt, m0, String("non-positive"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("non-positive"),
        "0 -> 'non-positive'",
    )
    assert_false(_memo_string_is_dirty(w, rt, m0), "clean after recompute")

    # Write 5 -> dirty again
    _signal_write(w, rt, sig, 5)
    assert_true(_memo_string_is_dirty(w, rt, m0), "dirty after write(5)")

    _memo_string_begin_compute(w, rt, m0)
    var v3 = _signal_read(w, rt, sig)
    if v3 > 0:
        _memo_string_end_compute(w, rt, m0, String("positive"))
    else:
        _memo_string_end_compute(w, rt, m0, String("non-positive"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("positive"),
        "5 -> 'positive'",
    )

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── 16. ms_str_conversion ────────────────────────────────────────────────────


fn test_ms_str_conversion(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """peek returns the cached string (verifies __str__ roundtrip)."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_string_create(w, rt, s0, String("hello world"))
    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("hello world"),
        "initial peek = 'hello world'",
    )

    _memo_string_begin_compute(w, rt, m0)
    _memo_string_end_compute(w, rt, m0, String("goodbye"))

    assert_equal(
        _memo_string_peek(w, rt, m0),
        String("goodbye"),
        "after compute peek = 'goodbye'",
    )

    # Verify read (with context tracking) also returns correct value
    var read_val = _memo_string_read(w, rt, m0)
    assert_equal(read_val, String("goodbye"), "read = 'goodbye'")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Test runner ──────────────────────────────────────────────────────────────


fn test_all(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    test_ms_create_returns_valid_id(w)
    test_ms_starts_dirty(w)
    test_ms_initial_value(w)
    test_ms_compute_stores_value(w)
    test_ms_compute_clears_dirty(w)
    test_ms_signal_write_marks_dirty(w)
    test_ms_read_subscribes_context(w)
    test_ms_recompute_from_convenience(w)
    test_ms_peek_does_not_subscribe(w)
    test_ms_is_empty_when_empty(w)
    test_ms_is_empty_when_not_empty(w)
    test_ms_destroy_cleans_up(w)
    test_ms_id_reuse_after_destroy(w)
    test_ms_multiple_memos_independent(w)
    test_ms_dirty_propagates_through_chain(w)
    test_ms_str_conversion(w)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_all(w)
    print("memo_string: 16/16 passed")
