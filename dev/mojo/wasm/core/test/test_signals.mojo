# Runtime / Signals exercised through the real WASM binary via mojo-wasmtime
# (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that the reactive runtime's signal system works correctly
# when compiled to WASM and executed via the Wasmtime runtime.
#
# Note: SignalStore direct tests (subscribe, unsubscribe, get_subscribers,
# read_tracked, contains_out_of_bounds) are not covered here since there are
# no WASM exports for direct SignalStore operations.
#
# Run with:
#   mojo test test/test_signals.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
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


# ── Runtime lifecycle ────────────────────────────────────────────────────────


fn test_runtime_create_returns_non_null(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    assert_true(rt != 0, "runtime_create returns non-null pointer")
    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        0,
        "new runtime has 0 signals",
    )

    _destroy_runtime(w, rt)


# ── Signal create and read ───────────────────────────────────────────────────


fn test_signal_create_and_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 42)))
    assert_equal(key, 0, "first signal gets key 0")
    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        1,
        "signal_count is 1",
    )
    assert_equal(
        Int(w[].call_i32("signal_contains", args_ptr_i32(rt, key))),
        1,
        "signal exists",
    )

    var val = Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key)))
    assert_equal(val, 42, "read returns initial value 42")

    _destroy_runtime(w, rt)


# ── Signal write and read back ───────────────────────────────────────────────


fn test_signal_write_and_read_back(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        0,
        "initial value is 0",
    )

    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 99))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        99,
        "read after write returns 99",
    )

    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, -42))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        -42,
        "read after write returns -42",
    )

    _destroy_runtime(w, rt)


# ── Signal peek (no subscription) ────────────────────────────────────────────


fn test_signal_peek(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 77)))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))),
        77,
        "peek returns 77",
    )

    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 88))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))),
        88,
        "peek after write returns 88",
    )

    _destroy_runtime(w, rt)


# ── Signal version tracking ──────────────────────────────────────────────────


fn test_signal_version_tracking(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    assert_equal(
        Int(w[].call_i32("signal_version", args_ptr_i32(rt, key))),
        0,
        "initial version is 0",
    )

    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 1))
    assert_equal(
        Int(w[].call_i32("signal_version", args_ptr_i32(rt, key))),
        1,
        "version after 1 write is 1",
    )

    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 2))
    assert_equal(
        Int(w[].call_i32("signal_version", args_ptr_i32(rt, key))),
        2,
        "version after 2 writes is 2",
    )

    # Peek and read don't change version
    _ = w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_version", args_ptr_i32(rt, key))),
        2,
        "read/peek don't bump version",
    )

    _destroy_runtime(w, rt)


# ── Multiple independent signals ─────────────────────────────────────────────


fn test_multiple_independent_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var k1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 10)))
    var k2 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 20)))
    var k3 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 30)))

    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        3,
        "3 signals created",
    )
    assert_true(k1 != k2 and k2 != k3 and k1 != k3, "all keys distinct")

    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k1))),
        10,
        "signal 1 reads 10",
    )
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k2))),
        20,
        "signal 2 reads 20",
    )
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k3))),
        30,
        "signal 3 reads 30",
    )

    # Write to one doesn't affect others
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, k2, 200))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k1))),
        10,
        "signal 1 unchanged",
    )
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k2))),
        200,
        "signal 2 updated to 200",
    )
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k3))),
        30,
        "signal 3 unchanged",
    )

    _destroy_runtime(w, rt)


# ── Signal destroy ───────────────────────────────────────────────────────────


fn test_signal_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var k1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 10)))
    var k2 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 20)))
    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        2,
        "2 signals before destroy",
    )

    w[].call_void("signal_destroy", args_ptr_i32(rt, k1))
    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        1,
        "1 signal after destroy",
    )
    assert_equal(
        Int(w[].call_i32("signal_contains", args_ptr_i32(rt, k1))),
        0,
        "destroyed signal not found",
    )
    assert_equal(
        Int(w[].call_i32("signal_contains", args_ptr_i32(rt, k2))),
        1,
        "other signal still exists",
    )

    _destroy_runtime(w, rt)


# ── Signal slot reuse after destroy ──────────────────────────────────────────


fn test_signal_slot_reuse_after_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var k1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 10)))
    w[].call_void("signal_destroy", args_ptr_i32(rt, k1))

    var k2 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 99)))
    assert_equal(k2, k1, "new signal reuses destroyed slot")
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, k2))),
        99,
        "reused slot has new value",
    )

    _destroy_runtime(w, rt)


# ── Signal iadd (+=) via WASM export ─────────────────────────────────────────


fn test_signal_iadd(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 10)))
    w[].call_void("signal_iadd_i32", args_ptr_i32_i32(rt, key, 5))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        15,
        "10 += 5 => 15",
    )

    w[].call_void("signal_iadd_i32", args_ptr_i32_i32(rt, key, -3))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        12,
        "15 += (-3) => 12",
    )

    w[].call_void("signal_iadd_i32", args_ptr_i32_i32(rt, key, 0))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        12,
        "12 += 0 => 12",
    )

    _destroy_runtime(w, rt)


# ── Signal isub (-=) via WASM export ─────────────────────────────────────────


fn test_signal_isub(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 100)))
    w[].call_void("signal_isub_i32", args_ptr_i32_i32(rt, key, 30))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        70,
        "100 -= 30 => 70",
    )

    w[].call_void("signal_isub_i32", args_ptr_i32_i32(rt, key, -10))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        80,
        "70 -= (-10) => 80",
    )

    _destroy_runtime(w, rt)


# ── Context: no context by default ───────────────────────────────────────────


fn test_no_context_by_default(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    assert_equal(
        Int(w[].call_i32("runtime_has_context", args_ptr(rt))),
        0,
        "no context initially",
    )

    _destroy_runtime(w, rt)


# ── Context: set and clear ───────────────────────────────────────────────────


fn test_context_set_and_clear(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    w[].call_void("runtime_set_context", args_ptr_i32(rt, 42))
    assert_equal(
        Int(w[].call_i32("runtime_has_context", args_ptr(rt))),
        1,
        "context active after set",
    )

    w[].call_void("runtime_clear_context", args_ptr(rt))
    assert_equal(
        Int(w[].call_i32("runtime_has_context", args_ptr(rt))),
        0,
        "no context after clear",
    )

    _destroy_runtime(w, rt)


# ── Subscription: read with context subscribes ───────────────────────────────


fn test_read_with_context_subscribes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        0,
        "0 subscribers initially",
    )

    # Read without context — no subscription
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        0,
        "still 0 subscribers after read without context",
    )

    # Read with context — subscribes
    w[].call_void("runtime_set_context", args_ptr_i32(rt, 100))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        1,
        "1 subscriber after read with context",
    )

    # Reading again with same context is idempotent
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        1,
        "still 1 subscriber (idempotent)",
    )

    w[].call_void("runtime_clear_context", args_ptr(rt))
    _destroy_runtime(w, rt)


# ── Subscription: peek does NOT subscribe ────────────────────────────────────


fn test_peek_does_not_subscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    w[].call_void("runtime_set_context", args_ptr_i32(rt, 200))

    _ = w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        0,
        "peek does not subscribe",
    )

    w[].call_void("runtime_clear_context", args_ptr(rt))
    _destroy_runtime(w, rt)


# ── Subscription: multiple contexts subscribe ────────────────────────────────


fn test_multiple_contexts_subscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    w[].call_void("runtime_set_context", args_ptr_i32(rt, 10))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))

    w[].call_void("runtime_set_context", args_ptr_i32(rt, 20))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))

    w[].call_void("runtime_set_context", args_ptr_i32(rt, 30))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))

    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        3,
        "3 different contexts subscribed",
    )

    w[].call_void("runtime_clear_context", args_ptr(rt))
    _destroy_runtime(w, rt)


# ── Dirty scopes: write with subscribers produces dirty ──────────────────────


fn test_write_marks_subscribers_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        0,
        "no dirty scopes initially",
    )

    # Subscribe context 1
    w[].call_void("runtime_set_context", args_ptr_i32(rt, 1))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))

    # Subscribe context 2
    w[].call_void("runtime_set_context", args_ptr_i32(rt, 2))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))

    w[].call_void("runtime_clear_context", args_ptr(rt))

    # Write — should mark both contexts dirty
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 42))
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "has dirty after write",
    )
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        2,
        "2 dirty scopes",
    )

    _destroy_runtime(w, rt)


# ── Dirty scopes: write without subscribers ──────────────────────────────────


fn test_write_without_subscribers_is_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 99))

    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        0,
        "no dirty without subscribers",
    )
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        0,
        "dirty count is 0",
    )

    _destroy_runtime(w, rt)


# ── Dirty scopes: iadd marks dirty ───────────────────────────────────────────


fn test_iadd_marks_subscribers_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    w[].call_void("runtime_set_context", args_ptr_i32(rt, 50))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    w[].call_void("runtime_clear_context", args_ptr(rt))

    # iadd via WASM export
    w[].call_void("signal_iadd_i32", args_ptr_i32_i32(rt, key, 1))
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "iadd marks subscriber dirty",
    )
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        1,
        "1 dirty scope from iadd",
    )

    _destroy_runtime(w, rt)


# ── Multiple writes deduplicate dirty scopes ─────────────────────────────────


fn test_multiple_writes_deduplicate_dirty_scopes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    w[].call_void("runtime_set_context", args_ptr_i32(rt, 1))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    w[].call_void("runtime_clear_context", args_ptr(rt))

    # Write twice — same subscriber should not be double-queued
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 10))
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 20))

    # The dirty count should still be 1 since it's the same context
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        1,
        "same subscriber not double-queued",
    )

    _destroy_runtime(w, rt)


# ── Read after write in same turn returns new value ──────────────────────────


fn test_read_after_write_returns_new_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 123))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        123,
        "immediate read gets 123",
    )

    # iadd
    w[].call_void("signal_iadd_i32", args_ptr_i32_i32(rt, key, 1))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        124,
        "iadd then read gets 124",
    )

    _destroy_runtime(w, rt)


# ── Stress: create 100 signals, verify independence ──────────────────────────


fn test_stress_100_independent_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var keys = List[Int]()
    for i in range(100):
        keys.append(
            Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, i * 10)))
        )

    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        100,
        "100 signals created",
    )

    # Verify each holds its own value
    for i in range(100):
        assert_equal(
            Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, keys[i]))),
            i * 10,
            "signal " + String(i) + " holds correct value",
        )

    # Write to every other one
    for i in range(0, 100, 2):
        w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, keys[i], 999))

    # Verify written ones changed and others didn't
    for i in range(100):
        var expected = 999 if i % 2 == 0 else i * 10
        assert_equal(
            Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, keys[i]))),
            expected,
            "signal " + String(i) + " has expected value after selective write",
        )

    _destroy_runtime(w, rt)


# ── Stress: create/destroy cycle reuses slots ────────────────────────────────


fn test_stress_create_destroy_reuse_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    # Create 50 signals
    var keys = List[Int]()
    for i in range(50):
        keys.append(Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, i))))

    # Destroy all even-indexed
    for i in range(0, 50, 2):
        w[].call_void("signal_destroy", args_ptr_i32(rt, keys[i]))

    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        25,
        "25 signals after destroying 25",
    )

    # Create 25 more — should reuse freed slots
    var new_keys = List[Int]()
    for i in range(25):
        new_keys.append(
            Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 1000 + i)))
        )

    assert_equal(
        Int(w[].call_i32("signal_count", args_ptr(rt))),
        50,
        "back to 50 signals",
    )

    # Verify the new signals have correct values
    for i in range(25):
        assert_equal(
            Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, new_keys[i]))),
            1000 + i,
            "reused slot " + String(i) + " has correct new value",
        )

    # Verify odd-indexed originals still intact
    for i in range(1, 50, 2):
        assert_equal(
            Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, keys[i]))),
            i,
            "original odd-indexed signal " + String(i) + " still intact",
        )

    _destroy_runtime(w, rt)


# ── Edge case: negative values ───────────────────────────────────────────────


fn test_signal_negative_values(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(
        w[].call_i32("signal_create_i32", args_ptr_i32(rt, -2147483648))
    )  # INT32_MIN
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        -2147483648,
        "can store INT32_MIN",
    )

    w[].call_void(
        "signal_write_i32", args_ptr_i32_i32(rt, key, 2147483647)
    )  # INT32_MAX
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        2147483647,
        "can store INT32_MAX",
    )

    _destroy_runtime(w, rt)


# ── Edge case: zero initial value ────────────────────────────────────────────


fn test_signal_zero_initial_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var key = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        0,
        "zero initial value",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))),
        0,
        "zero peek value",
    )

    _destroy_runtime(w, rt)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_runtime_create_returns_non_null(w)
    test_signal_create_and_read(w)
    test_signal_write_and_read_back(w)
    test_signal_peek(w)
    test_signal_version_tracking(w)
    test_multiple_independent_signals(w)
    test_signal_destroy(w)
    test_signal_slot_reuse_after_destroy(w)
    test_signal_iadd(w)
    test_signal_isub(w)
    test_no_context_by_default(w)
    test_context_set_and_clear(w)
    test_read_with_context_subscribes(w)
    test_peek_does_not_subscribe(w)
    test_multiple_contexts_subscribe(w)
    test_write_marks_subscribers_dirty(w)
    test_write_without_subscribers_is_clean(w)
    test_iadd_marks_subscribers_dirty(w)
    test_multiple_writes_deduplicate_dirty_scopes(w)
    test_read_after_write_returns_new_value(w)
    test_stress_100_independent_signals(w)
    test_stress_create_destroy_reuse_cycle(w)
    test_signal_negative_values(w)
    test_signal_zero_initial_value(w)
    print("signals: 24/24 passed")
