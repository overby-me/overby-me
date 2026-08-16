# Memo (computed/derived signal) operations exercised through the real WASM
# binary via mojo-wasmtime (pure Mojo FFI bindings — no Python interop).
#
# These tests verify that the memo store, dirty tracking, dependency
# auto-tracking, and propagation chains work correctly when compiled to
# WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_memo.mojo

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


fn _signal_subscriber_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    """Return subscriber count for a signal."""
    return Int(
        w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, Int32(key)))
    )


fn _signal_version(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    """Return the write-version counter of a signal."""
    return Int(w[].call_i32("signal_version", args_ptr_i32(rt, Int32(key))))


fn _signal_contains(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Bool:
    """Check whether a signal key is live."""
    return w[].call_i32("signal_contains", args_ptr_i32(rt, Int32(key))) != 0


fn _memo_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: Int32,
) raises -> Int:
    """Create a memo and return its ID as Int."""
    return Int(
        w[].call_i32(
            "memo_create_i32",
            args_ptr_i32_i32(rt, Int32(scope_id), initial),
        )
    )


fn _memo_begin_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    """Begin memo computation."""
    w[].call_void("memo_begin_compute", args_ptr_i32(rt, Int32(memo_id)))


fn _memo_end_compute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Int32,
) raises:
    """End memo computation and store the result."""
    w[].call_void(
        "memo_end_compute_i32",
        args_ptr_i32_i32(rt, Int32(memo_id), value),
    )


fn _memo_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    """Read the memo's cached value."""
    return Int(w[].call_i32("memo_read_i32", args_ptr_i32(rt, Int32(memo_id))))


fn _memo_is_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    """Check whether the memo needs recomputation."""
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


fn _drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Drain the dirty scope queue and return the count."""
    return Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))


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


fn _hook_use_memo(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, initial: Int32
) raises -> Int:
    """Hook: create or retrieve an Int32 memo for the current scope."""
    return Int(w[].call_i32("hook_use_memo_i32", args_ptr_i32(rt, initial)))


fn _hook_use_signal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, initial: Int32
) raises -> Int:
    """Hook: create or retrieve an Int32 signal for the current scope."""
    return Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, initial)))


fn _scope_hook_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, scope_id: Int
) raises -> Int:
    """Return the number of hooks in a scope."""
    return Int(
        w[].call_i32("scope_hook_count", args_ptr_i32(rt, Int32(scope_id)))
    )


fn _scope_hook_tag_at(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    index: Int,
) raises -> Int:
    """Return the hook tag at the given index."""
    return Int(
        w[].call_i32(
            "scope_hook_tag_at",
            args_ptr_i32_i32(rt, Int32(scope_id), Int32(index)),
        )
    )


fn _scope_hook_value_at(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    index: Int,
) raises -> Int:
    """Return the hook value at the given index."""
    return Int(
        w[].call_i32(
            "scope_hook_value_at",
            args_ptr_i32_i32(rt, Int32(scope_id), Int32(index)),
        )
    )


# ── Create / Destroy ─────────────────────────────────────────────────────────


fn test_memo_create_returns_valid_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    assert_equal(_memo_count(w, rt), 0, "no memos initially")

    var m0 = _memo_create(w, rt, s0, 42)
    assert_true(m0 >= 0, "memo ID is non-negative")
    assert_equal(_memo_count(w, rt), 1, "memo count is 1 after create")

    _memo_destroy(w, rt, m0)
    assert_equal(_memo_count(w, rt), 0, "memo count is 0 after destroy")

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_memo_initial_value_readable(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 99)
    assert_equal(_memo_read(w, rt, m0), 99, "initial cached value is 99")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_memo_starts_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 0)
    assert_true(_memo_is_dirty(w, rt, m0), "memo is dirty after creation")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_memo_allocates_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 0)
    var out_key = _memo_output_key(w, rt, m0)
    var ctx_id = _memo_context_id(w, rt, m0)

    assert_true(_signal_contains(w, rt, out_key), "output signal exists")
    assert_true(_signal_contains(w, rt, ctx_id), "context signal exists")

    _memo_destroy(w, rt, m0)
    assert_false(
        _signal_contains(w, rt, out_key),
        "output signal destroyed with memo",
    )
    assert_false(
        _signal_contains(w, rt, ctx_id),
        "context signal destroyed with memo",
    )

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Multiple memos ───────────────────────────────────────────────────────────


fn test_memo_multiple_create_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 10)
    var m1 = _memo_create(w, rt, s0, 20)
    var m2 = _memo_create(w, rt, s0, 30)
    assert_equal(_memo_count(w, rt), 3, "3 memos created")
    assert_true(m0 != m1, "m0 != m1")
    assert_true(m1 != m2, "m1 != m2")

    assert_equal(_memo_read(w, rt, m0), 10, "m0 initial = 10")
    assert_equal(_memo_read(w, rt, m1), 20, "m1 initial = 20")
    assert_equal(_memo_read(w, rt, m2), 30, "m2 initial = 30")

    _memo_destroy(w, rt, m1)
    assert_equal(_memo_count(w, rt), 2, "2 memos after destroying m1")

    _memo_destroy(w, rt, m0)
    _memo_destroy(w, rt, m2)
    assert_equal(_memo_count(w, rt), 0, "0 memos after destroying all")

    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── ID reuse ─────────────────────────────────────────────────────────────────


fn test_memo_id_reuse(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 1)
    _memo_destroy(w, rt, m0)

    var m1 = _memo_create(w, rt, s0, 2)
    assert_equal(m1, m0, "freed memo ID is reused")
    assert_equal(_memo_read(w, rt, m1), 2, "reused slot has new value")

    _memo_destroy(w, rt, m1)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Begin / End compute ──────────────────────────────────────────────────────


fn test_memo_compute_clears_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 0)
    assert_true(_memo_is_dirty(w, rt, m0), "dirty before first compute")

    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 100)

    assert_false(_memo_is_dirty(w, rt, m0), "clean after compute")
    assert_equal(_memo_read(w, rt, m0), 100, "cached value is 100")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_memo_recompute_updates_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 0)

    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 50)
    assert_equal(_memo_read(w, rt, m0), 50, "first compute = 50")

    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 75)
    assert_equal(_memo_read(w, rt, m0), 75, "second compute = 75")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Auto-track: signal write → memo dirty ────────────────────────────────────


fn test_memo_signal_write_marks_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 10)
    var m0 = _memo_create(w, rt, s0, 0)

    # Compute: read signal inside bracket → auto-subscribe
    _memo_begin_compute(w, rt, m0)
    var val = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, m0, Int32(val))

    assert_equal(_memo_read(w, rt, m0), 10, "computed from signal=10")
    assert_false(_memo_is_dirty(w, rt, m0), "clean after compute")

    # Write to input signal
    _signal_write(w, rt, sig, 20)
    assert_true(_memo_is_dirty(w, rt, m0), "dirty after signal write")

    # Recompute
    _memo_begin_compute(w, rt, m0)
    var val2 = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, m0, Int32(val2))

    assert_equal(_memo_read(w, rt, m0), 20, "recomputed = 20")
    assert_false(_memo_is_dirty(w, rt, m0), "clean after recompute")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Propagation: signal → memo → scope dirty ────────────────────────────────


fn test_memo_propagation_to_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var m0 = _memo_create(w, rt, s0, 0)

    # Compute: subscribe memo to signal
    _memo_begin_compute(w, rt, m0)
    var v = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, m0, Int32(v))

    # Subscribe scope to memo's output (simulate render read)
    var prev = _scope_begin_render(w, rt, s0)
    _ = _memo_read(w, rt, m0)
    _scope_end_render(w, rt, prev)

    # Drain any existing dirty
    _ = _drain_dirty(w, rt)

    # Write to input → should propagate to scope
    _signal_write(w, rt, sig, 99)
    assert_true(_memo_is_dirty(w, rt, m0), "memo dirty after sig write")

    var dirty_count = _drain_dirty(w, rt)
    assert_true(dirty_count >= 1, "scope dirty after propagation")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Diamond: two memos depend on same signal ─────────────────────────────────


fn test_memo_diamond_dependency(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 1)
    var mA = _memo_create(w, rt, s0, 0)
    var mB = _memo_create(w, rt, s0, 0)

    # mA computes sig * 2
    _memo_begin_compute(w, rt, mA)
    var vA = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mA, Int32(vA * 2))

    # mB computes sig * 3
    _memo_begin_compute(w, rt, mB)
    var vB = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, mB, Int32(vB * 3))

    assert_equal(_memo_read(w, rt, mA), 2, "mA = sig*2 = 2")
    assert_equal(_memo_read(w, rt, mB), 3, "mB = sig*3 = 3")

    # Write to shared input — both dirty
    _signal_write(w, rt, sig, 10)
    assert_true(_memo_is_dirty(w, rt, mA), "mA dirty after write")
    assert_true(_memo_is_dirty(w, rt, mB), "mB dirty after write")

    _memo_destroy(w, rt, mA)
    _memo_destroy(w, rt, mB)
    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Dependency re-tracking on recompute ──────────────────────────────────────


fn test_memo_dependency_retracking(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sigA = _signal_create(w, rt, 1)
    var sigB = _signal_create(w, rt, 100)
    var m0 = _memo_create(w, rt, s0, 0)

    # First compute: read only sigA
    _memo_begin_compute(w, rt, m0)
    var v1 = _signal_read(w, rt, sigA)
    _memo_end_compute(w, rt, m0, Int32(v1))
    assert_equal(_memo_read(w, rt, m0), 1, "first compute reads sigA=1")

    # sigA write → dirty
    _signal_write(w, rt, sigA, 2)
    assert_true(_memo_is_dirty(w, rt, m0), "dirty after sigA write")

    # Second compute: read only sigB (not sigA)
    _memo_begin_compute(w, rt, m0)
    var v2 = _signal_read(w, rt, sigB)
    _memo_end_compute(w, rt, m0, Int32(v2))
    assert_equal(_memo_read(w, rt, m0), 100, "second compute reads sigB=100")

    # sigA write → NOT dirty (no longer subscribed)
    _signal_write(w, rt, sigA, 999)
    assert_false(
        _memo_is_dirty(w, rt, m0),
        "memo NOT dirty after sigA write (unsubscribed)",
    )

    # sigB write → dirty
    _signal_write(w, rt, sigB, 200)
    assert_true(_memo_is_dirty(w, rt, m0), "memo dirty after sigB write")

    _memo_destroy(w, rt, m0)
    _signal_destroy(w, rt, sigA)
    _signal_destroy(w, rt, sigB)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Output signal version bumps ──────────────────────────────────────────────


fn test_memo_output_version_bumps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 0)
    var out_key = _memo_output_key(w, rt, m0)

    var v0 = _signal_version(w, rt, out_key)

    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 10)
    var v1 = _signal_version(w, rt, out_key)
    assert_true(v1 > v0, "version bumped after first compute")

    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 20)
    var v2 = _signal_version(w, rt, out_key)
    assert_true(v2 > v1, "version bumped after second compute")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Destroy non-existent memo — no crash ─────────────────────────────────────


fn test_memo_destroy_nonexistent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    # Should not crash
    _memo_destroy(w, rt, 9999)
    assert_equal(_memo_count(w, rt), 0, "count 0 after destroying nonexistent")

    _destroy_runtime(w, rt)


# ── Cache hit: read without recompute ────────────────────────────────────────


fn test_memo_cache_hit(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 0)
    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 77)

    var r1 = _memo_read(w, rt, m0)
    var r2 = _memo_read(w, rt, m0)
    assert_equal(r1, 77, "first read = 77")
    assert_equal(r2, 77, "second read = 77 (cache hit)")
    assert_false(_memo_is_dirty(w, rt, m0), "still clean after reads")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Read without active context — no crash ───────────────────────────────────


fn test_memo_read_no_context(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var m0 = _memo_create(w, rt, s0, 55)
    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 55)

    # Read with no scope render active (no context)
    var v = _memo_read(w, rt, m0)
    assert_equal(v, 55, "read without context returns cached value")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Destroy cleans up input subscriptions ────────────────────────────────────


fn test_memo_destroy_cleans_up(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 1)
    var m0 = _memo_create(w, rt, s0, 0)

    # Compute to subscribe memo to sig
    _memo_begin_compute(w, rt, m0)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, m0, 1)

    var subs_before = _signal_subscriber_count(w, rt, sig)
    assert_true(subs_before >= 1, "sig has subscriber after compute")

    # Destroy memo
    _memo_destroy(w, rt, m0)

    # Write should not crash even though subscriber context is gone
    _signal_write(w, rt, sig, 99)
    _ = _drain_dirty(w, rt)

    _signal_destroy(w, rt, sig)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Test runner ──────────────────────────────────────────────────────────────


# ── Hook: use_memo_i32 ──────────────────────────────────────────────────────


fn test_hook_memo_creates_on_first_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Hook creates memo on first render with HOOK_MEMO tag."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    assert_equal(_memo_count(w, rt), 0, "no memos before render")
    assert_equal(_scope_hook_count(w, rt, s0), 0, "no hooks before render")

    # First render
    var prev = _scope_begin_render(w, rt, s0)
    var m0 = _hook_use_memo(w, rt, 42)
    _scope_end_render(w, rt, prev)

    assert_true(m0 >= 0, "memo ID is non-negative")
    assert_equal(_memo_count(w, rt), 1, "1 memo after hook")
    assert_equal(_scope_hook_count(w, rt, s0), 1, "1 hook registered")
    # HOOK_MEMO tag = 1
    assert_equal(
        _scope_hook_tag_at(w, rt, s0, 0), 1, "hook tag is HOOK_MEMO (1)"
    )
    assert_equal(
        _scope_hook_value_at(w, rt, s0, 0), m0, "hook value is memo ID"
    )
    # Initial value readable
    assert_equal(_memo_read(w, rt, m0), 42, "memo initial value is 42")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_hook_memo_returns_same_id_on_rerender(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Hook returns same memo ID on re-render (initial ignored)."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    # First render
    var prev = _scope_begin_render(w, rt, s0)
    var m0 = _hook_use_memo(w, rt, 10)
    _scope_end_render(w, rt, prev)

    # Compute a value
    _memo_begin_compute(w, rt, m0)
    _memo_end_compute(w, rt, m0, 100)
    assert_equal(_memo_read(w, rt, m0), 100, "computed value is 100")

    # Re-render — initial value (999) is ignored
    prev = _scope_begin_render(w, rt, s0)
    var m1 = _hook_use_memo(w, rt, 999)
    _scope_end_render(w, rt, prev)

    assert_equal(m1, m0, "same memo ID on re-render")
    assert_equal(_memo_count(w, rt), 1, "still 1 memo")
    assert_equal(_scope_hook_count(w, rt, s0), 1, "still 1 hook")
    # Cached value survives re-render
    assert_equal(_memo_read(w, rt, m1), 100, "cached value survives re-render")

    _memo_destroy(w, rt, m0)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_hook_memo_multiple_distinct_ids(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Multiple memos in same scope get distinct IDs."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    # First render — create 3 memos
    var prev = _scope_begin_render(w, rt, s0)
    var mA = _hook_use_memo(w, rt, 1)
    var mB = _hook_use_memo(w, rt, 2)
    var mC = _hook_use_memo(w, rt, 3)
    _scope_end_render(w, rt, prev)

    assert_true(mA != mB, "mA != mB")
    assert_true(mB != mC, "mB != mC")
    assert_true(mA != mC, "mA != mC")
    assert_equal(_memo_count(w, rt), 3, "3 memos created")
    assert_equal(_scope_hook_count(w, rt, s0), 3, "3 hooks registered")

    # All tags are HOOK_MEMO (1)
    assert_equal(_scope_hook_tag_at(w, rt, s0, 0), 1, "hook 0 tag = MEMO")
    assert_equal(_scope_hook_tag_at(w, rt, s0, 1), 1, "hook 1 tag = MEMO")
    assert_equal(_scope_hook_tag_at(w, rt, s0, 2), 1, "hook 2 tag = MEMO")

    # Re-render returns same IDs in order
    prev = _scope_begin_render(w, rt, s0)
    var rA = _hook_use_memo(w, rt, 0)
    var rB = _hook_use_memo(w, rt, 0)
    var rC = _hook_use_memo(w, rt, 0)
    _scope_end_render(w, rt, prev)

    assert_equal(rA, mA, "re-render: mA stable")
    assert_equal(rB, mB, "re-render: mB stable")
    assert_equal(rC, mC, "re-render: mC stable")

    _memo_destroy(w, rt, mA)
    _memo_destroy(w, rt, mB)
    _memo_destroy(w, rt, mC)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


fn test_hook_memo_interleaved_with_signal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Hook cursor advances correctly when memos and signals are interleaved."""
    var rt = _create_runtime(w)
    var s0 = _scope_create(w, rt, 0, -1)

    # First render: signal, memo, signal, memo
    var prev = _scope_begin_render(w, rt, s0)
    var sig0 = _hook_use_signal(w, rt, 10)
    var mem0 = _hook_use_memo(w, rt, 20)
    var sig1 = _hook_use_signal(w, rt, 30)
    var mem1 = _hook_use_memo(w, rt, 40)
    _scope_end_render(w, rt, prev)

    assert_equal(_scope_hook_count(w, rt, s0), 4, "4 hooks total")
    # HOOK_SIGNAL = 0, HOOK_MEMO = 1
    assert_equal(_scope_hook_tag_at(w, rt, s0, 0), 0, "hook 0 = SIGNAL")
    assert_equal(_scope_hook_tag_at(w, rt, s0, 1), 1, "hook 1 = MEMO")
    assert_equal(_scope_hook_tag_at(w, rt, s0, 2), 0, "hook 2 = SIGNAL")
    assert_equal(_scope_hook_tag_at(w, rt, s0, 3), 1, "hook 3 = MEMO")

    # Re-render: same order
    prev = _scope_begin_render(w, rt, s0)
    var rSig0 = _hook_use_signal(w, rt, 0)
    var rMem0 = _hook_use_memo(w, rt, 0)
    var rSig1 = _hook_use_signal(w, rt, 0)
    var rMem1 = _hook_use_memo(w, rt, 0)
    _scope_end_render(w, rt, prev)

    assert_equal(rSig0, sig0, "signal 0 stable")
    assert_equal(rMem0, mem0, "memo 0 stable")
    assert_equal(rSig1, sig1, "signal 1 stable")
    assert_equal(rMem1, mem1, "memo 1 stable")

    _memo_destroy(w, rt, mem0)
    _memo_destroy(w, rt, mem1)
    _scope_destroy(w, rt, s0)
    _destroy_runtime(w, rt)


# ── Test runner ──────────────────────────────────────────────────────────────


fn test_all(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    test_memo_create_returns_valid_id(w)
    test_memo_initial_value_readable(w)
    test_memo_starts_dirty(w)
    test_memo_allocates_signals(w)
    test_memo_multiple_create_destroy(w)
    test_memo_id_reuse(w)
    test_memo_compute_clears_dirty(w)
    test_memo_recompute_updates_value(w)
    test_memo_signal_write_marks_dirty(w)
    test_memo_propagation_to_scope(w)
    test_memo_diamond_dependency(w)
    test_memo_dependency_retracking(w)
    test_memo_output_version_bumps(w)
    test_memo_destroy_nonexistent(w)
    test_memo_cache_hit(w)
    test_memo_read_no_context(w)
    test_memo_destroy_cleans_up(w)
    test_hook_memo_creates_on_first_render(w)
    test_hook_memo_returns_same_id_on_rerender(w)
    test_hook_memo_multiple_distinct_ids(w)
    test_hook_memo_interleaved_with_signal(w)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_all(w)
    print("memo: 83/83 passed")
