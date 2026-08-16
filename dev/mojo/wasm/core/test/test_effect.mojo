# Effect (reactive side effect) operations exercised through the real WASM
# binary via mojo-wasmtime (pure Mojo FFI bindings — no Python interop).
#
# These tests verify that the effect store, pending tracking, dependency
# auto-tracking, and propagation chains work correctly when compiled to
# WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_effect.mojo

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


fn _begin_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, scope_id: Int
) raises -> Int:
    """Begin rendering a scope. Returns previous scope ID."""
    return Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, Int32(scope_id)))
    )


fn _end_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, prev: Int
) raises:
    """End rendering a scope."""
    w[].call_void("scope_end_render", args_ptr_i32(rt, Int32(prev)))


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


fn _signal_subscriber_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    """Return subscriber count for a signal."""
    return Int(
        w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, Int32(key)))
    )


fn _signal_contains(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Bool:
    """Check whether a signal key is live."""
    return w[].call_i32("signal_contains", args_ptr_i32(rt, Int32(key))) != 0


fn _effect_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, scope_id: Int
) raises -> Int:
    """Create an effect and return its ID as Int."""
    return Int(w[].call_i32("effect_create", args_ptr_i32(rt, Int32(scope_id))))


fn _effect_begin_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    """Begin effect execution."""
    w[].call_void("effect_begin_run", args_ptr_i32(rt, Int32(effect_id)))


fn _effect_end_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    """End effect execution."""
    w[].call_void("effect_end_run", args_ptr_i32(rt, Int32(effect_id)))


fn _effect_is_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises -> Bool:
    """Check whether the effect is pending."""
    return (
        w[].call_i32("effect_is_pending", args_ptr_i32(rt, Int32(effect_id)))
        != 0
    )


fn _effect_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    """Destroy an effect."""
    w[].call_void("effect_destroy", args_ptr_i32(rt, Int32(effect_id)))


fn _effect_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Return the number of live effects."""
    return Int(w[].call_i32("effect_count", args_ptr(rt)))


fn _effect_context_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises -> Int:
    """Return the reactive context ID of the effect."""
    return Int(
        w[].call_i32("effect_context_id", args_ptr_i32(rt, Int32(effect_id)))
    )


fn _effect_drain_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Return the number of currently pending effects."""
    return Int(w[].call_i32("effect_drain_pending", args_ptr(rt)))


fn _effect_pending_at(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, index: Int
) raises -> Int:
    """Return the effect ID at the given index in the pending list."""
    return Int(
        w[].call_i32("effect_pending_at", args_ptr_i32(rt, Int32(index)))
    )


fn _dirty_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Return number of dirty scopes."""
    return Int(w[].call_i32("runtime_dirty_count", args_ptr(rt)))


fn _hook_use_effect(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    """Hook: create or retrieve an effect for the current scope."""
    return Int(w[].call_i32("hook_use_effect", args_ptr(rt)))


fn _hook_use_signal_i32(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, initial: Int32
) raises -> Int:
    """Hook: create or retrieve an Int32 signal for the current scope."""
    return Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, initial)))


fn _hook_use_memo_i32(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, initial: Int32
) raises -> Int:
    """Hook: create or retrieve an Int32 memo for the current scope."""
    return Int(w[].call_i32("hook_use_memo_i32", args_ptr_i32(rt, initial)))


# Memo helpers for testing effect→memo chain
fn _memo_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    initial: Int32,
) raises -> Int:
    """Create a memo and return its ID."""
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


fn _memo_end_compute_i32(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Int32,
) raises:
    w[].call_void(
        "memo_end_compute_i32",
        args_ptr_i32_i32(rt, Int32(memo_id), value),
    )


fn _memo_read_i32(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    return Int(w[].call_i32("memo_read_i32", args_ptr_i32(rt, Int32(memo_id))))


fn _memo_output_key(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    return Int(
        w[].call_i32("memo_output_key", args_ptr_i32(rt, Int32(memo_id)))
    )


# ═══════════════════════════════════════════════════════════════════════════════
# Effect Store — create, destroy, pending, running
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_create_returns_valid_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Creating an effect returns ID 0 and count becomes 1."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var eid = _effect_create(w, rt, scope)
    assert_equal(eid, 0, "first effect should be ID 0")
    assert_equal(_effect_count(w, rt), 1, "count should be 1")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_create_returns_valid_id")


def test_effect_starts_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """A newly created effect starts pending (needs first run)."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var eid = _effect_create(w, rt, scope)
    assert_true(_effect_is_pending(w, rt, eid), "effect should start pending")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_starts_pending")


def test_effect_has_context_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """An effect has a valid reactive context ID (a signal key)."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var eid = _effect_create(w, rt, scope)
    var ctx = _effect_context_id(w, rt, eid)
    # Context ID is a signal key — should be valid
    assert_true(_signal_contains(w, rt, ctx), "context signal should exist")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_has_context_id")


def test_effect_two_effects_distinct_ids(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Creating two effects returns distinct IDs."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var e0 = _effect_create(w, rt, scope)
    var e1 = _effect_create(w, rt, scope)
    assert_true(e0 != e1, "effect IDs should be distinct")
    assert_equal(_effect_count(w, rt), 2, "count should be 2")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_two_effects_distinct_ids")


def test_effect_destroy_reduces_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Destroying an effect reduces the count."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var e0 = _effect_create(w, rt, scope)
    var e1 = _effect_create(w, rt, scope)
    assert_equal(_effect_count(w, rt), 2, "count should be 2")

    _effect_destroy(w, rt, e0)
    assert_equal(_effect_count(w, rt), 1, "count should be 1 after destroy")

    _effect_destroy(w, rt, e1)
    assert_equal(
        _effect_count(w, rt), 0, "count should be 0 after both destroyed"
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_destroy_reduces_count")


def test_effect_id_reuse_after_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Destroyed effect IDs are reused by the next create."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var e0 = _effect_create(w, rt, scope)
    _effect_destroy(w, rt, e0)

    var e1 = _effect_create(w, rt, scope)
    assert_equal(e1, e0, "destroyed slot should be reused")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_id_reuse_after_destroy")


def test_effect_destroy_cleans_up_context_signal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroying an effect also destroys its context signal."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var eid = _effect_create(w, rt, scope)
    var ctx = _effect_context_id(w, rt, eid)
    assert_true(
        _signal_contains(w, rt, ctx),
        "context signal should exist before destroy",
    )

    _effect_destroy(w, rt, eid)
    assert_false(
        _signal_contains(w, rt, ctx),
        "context signal should be destroyed",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_destroy_cleans_up_context_signal")


# ═══════════════════════════════════════════════════════════════════════════════
# Effect execution — begin_run / end_run
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_begin_end_run_clears_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Running an effect (begin + end) clears the pending flag."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var eid = _effect_create(w, rt, scope)
    assert_true(_effect_is_pending(w, rt, eid), "should start pending")

    _effect_begin_run(w, rt, eid)
    _effect_end_run(w, rt, eid)

    assert_false(
        _effect_is_pending(w, rt, eid), "should not be pending after run"
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_begin_end_run_clears_pending")


def test_effect_signal_read_during_run_subscribes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Reading a signal during an effect run subscribes the effect's context."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(42))

    var eid = _effect_create(w, rt, scope)
    var ctx = _effect_context_id(w, rt, eid)

    # Before run: signal has no subscribers from the effect
    var subs_before = _signal_subscriber_count(w, rt, sig)

    # Run the effect and read the signal
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)  # should auto-subscribe
    _effect_end_run(w, rt, eid)

    var subs_after = _signal_subscriber_count(w, rt, sig)
    assert_equal(
        subs_after,
        subs_before + 1,
        "signal should have one more subscriber after effect reads it",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_signal_read_during_run_subscribes")


# ═══════════════════════════════════════════════════════════════════════════════
# Propagation — signal write → effect pending
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_signal_write_marks_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Writing to a signal that the effect depends on marks it pending."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(10))

    var eid = _effect_create(w, rt, scope)

    # Run the effect, reading the signal to establish subscription
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)

    assert_false(_effect_is_pending(w, rt, eid), "not pending after run")

    # Write to the signal — should mark effect pending
    _signal_write(w, rt, sig, Int32(20))

    assert_true(
        _effect_is_pending(w, rt, eid),
        "effect should be pending after signal write",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_signal_write_marks_pending")


def test_effect_signal_write_does_not_dirty_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Writing to a signal that ONLY an effect subscribes to does NOT dirty any scope.
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var eid = _effect_create(w, rt, scope)

    # Run effect to subscribe to the signal
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)

    # Confirm no dirty scopes before write
    assert_equal(_dirty_count(w, rt), 0, "no dirty scopes before")

    # Write to signal — effect becomes pending, but no scope should be dirty
    _signal_write(w, rt, sig, Int32(1))

    assert_true(_effect_is_pending(w, rt, eid), "effect should be pending")
    # The signal only has the effect's context as subscriber, not a scope,
    # so dirty_scopes should remain 0
    assert_equal(
        _dirty_count(w, rt),
        0,
        "no scopes should be dirty (only effect subscribed)",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_signal_write_does_not_dirty_scope")


def test_effect_two_effects_same_signal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Two effects reading the same signal both become pending on write."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var e0 = _effect_create(w, rt, scope)
    var e1 = _effect_create(w, rt, scope)

    # Run both effects, reading the same signal
    _effect_begin_run(w, rt, e0)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e0)

    _effect_begin_run(w, rt, e1)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e1)

    assert_false(_effect_is_pending(w, rt, e0), "e0 not pending after run")
    assert_false(_effect_is_pending(w, rt, e1), "e1 not pending after run")

    # Write to signal
    _signal_write(w, rt, sig, Int32(99))

    assert_true(
        _effect_is_pending(w, rt, e0), "e0 should be pending after write"
    )
    assert_true(
        _effect_is_pending(w, rt, e1), "e1 should be pending after write"
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_two_effects_same_signal")


def test_effect_scope_and_effect_both_react(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """A signal with both a scope subscriber and an effect subscriber:
    writing marks the scope dirty AND the effect pending."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    # Subscribe the scope to the signal via a render
    var prev = _begin_render(w, rt, scope)
    _ = _signal_read(w, rt, sig)
    _end_render(w, rt, prev)

    # Create an effect and subscribe it too
    var eid = _effect_create(w, rt, scope)
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)

    # Write to signal
    _signal_write(w, rt, sig, Int32(42))

    assert_true(_effect_is_pending(w, rt, eid), "effect should be pending")
    assert_true(_dirty_count(w, rt) > 0, "scope should be dirty")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_scope_and_effect_both_react")


def test_effect_unsubscribed_signal_no_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Writing to a signal that the effect does NOT read does not trigger pending.
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig_a = _signal_create(w, rt, Int32(0))
    var sig_b = _signal_create(w, rt, Int32(0))

    var eid = _effect_create(w, rt, scope)

    # Run effect reading only sig_a
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig_a)
    _effect_end_run(w, rt, eid)

    # Write to sig_b (not subscribed by effect)
    _signal_write(w, rt, sig_b, Int32(99))

    assert_false(
        _effect_is_pending(w, rt, eid),
        "effect should not be pending — sig_b not subscribed",
    )

    # Write to sig_a (subscribed by effect)
    _signal_write(w, rt, sig_a, Int32(99))
    assert_true(
        _effect_is_pending(w, rt, eid),
        "effect should be pending — sig_a is subscribed",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_unsubscribed_signal_no_pending")


# ═══════════════════════════════════════════════════════════════════════════════
# Dependency re-tracking
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_dependency_retracking(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Effect dependencies are re-tracked on each run.

    First run reads sig_a → subscribed to sig_a.
    Second run reads sig_b (not sig_a) → now only subscribed to sig_b.
    Writing sig_a should NOT trigger pending; writing sig_b should.
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig_a = _signal_create(w, rt, Int32(0))
    var sig_b = _signal_create(w, rt, Int32(0))

    var eid = _effect_create(w, rt, scope)

    # First run: read sig_a
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig_a)
    _effect_end_run(w, rt, eid)

    # Verify: writing sig_a triggers pending
    _signal_write(w, rt, sig_a, Int32(1))
    assert_true(_effect_is_pending(w, rt, eid), "pending after sig_a write")

    # Second run: read sig_b (NOT sig_a) — old subscriptions cleared
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig_b)
    _effect_end_run(w, rt, eid)

    # Now writing sig_a should NOT trigger pending
    _signal_write(w, rt, sig_a, Int32(2))
    assert_false(
        _effect_is_pending(w, rt, eid),
        "should NOT be pending — sig_a no longer tracked",
    )

    # Writing sig_b SHOULD trigger pending
    _signal_write(w, rt, sig_b, Int32(1))
    assert_true(
        _effect_is_pending(w, rt, eid),
        "should be pending — sig_b is now tracked",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_dependency_retracking")


# ═══════════════════════════════════════════════════════════════════════════════
# Drain pending effects
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_drain_pending_returns_pending_effects(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Drain_pending returns the correct count and IDs of pending effects."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var e0 = _effect_create(w, rt, scope)
    var e1 = _effect_create(w, rt, scope)

    # Both start pending — drain should return 2
    assert_equal(_effect_drain_pending(w, rt), 2, "both effects start pending")

    # Run both to clear pending
    _effect_begin_run(w, rt, e0)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e0)

    _effect_begin_run(w, rt, e1)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e1)

    assert_equal(
        _effect_drain_pending(w, rt), 0, "no pending after running both"
    )

    # Write to signal — both should become pending again
    _signal_write(w, rt, sig, Int32(99))
    var pending_count = _effect_drain_pending(w, rt)
    assert_equal(pending_count, 2, "both pending after write")

    # Verify the pending IDs
    var p0 = _effect_pending_at(w, rt, 0)
    var p1 = _effect_pending_at(w, rt, 1)
    # Both effect IDs should be in the pending list (order may vary)
    assert_true(
        (p0 == e0 and p1 == e1) or (p0 == e1 and p1 == e0),
        "pending list should contain both effect IDs",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_drain_pending_returns_pending_effects")


def test_effect_drain_pending_after_partial_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After running one of two pending effects, drain returns only the remaining one.
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var e0 = _effect_create(w, rt, scope)
    var e1 = _effect_create(w, rt, scope)

    # Run both to clear initial pending, subscribing to sig
    _effect_begin_run(w, rt, e0)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e0)
    _effect_begin_run(w, rt, e1)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e1)

    # Write to signal — both pending
    _signal_write(w, rt, sig, Int32(1))
    assert_equal(_effect_drain_pending(w, rt), 2, "both pending")

    # Run only e0
    _effect_begin_run(w, rt, e0)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, e0)

    assert_equal(
        _effect_drain_pending(w, rt),
        1,
        "only e1 should be pending",
    )
    assert_equal(
        _effect_pending_at(w, rt, 0), e1, "e1 is the remaining pending effect"
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_drain_pending_after_partial_run")


# ═══════════════════════════════════════════════════════════════════════════════
# Effect + Memo chain
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_reads_memo_output(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """An effect that reads a memo's output becomes pending when the memo changes.

    Chain: signal → memo (dirty → recompute) → memo output signal → effect pending.
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(5))

    # Create a memo that doubles the signal
    var memo = _memo_create(w, rt, scope, Int32(0))
    _memo_begin_compute(w, rt, memo)
    var val = _signal_read(w, rt, sig)
    _memo_end_compute_i32(w, rt, memo, Int32(val * 2))

    # Get the memo's output key
    var out_key = _memo_output_key(w, rt, memo)

    # Create an effect that reads the memo's output
    var eid = _effect_create(w, rt, scope)
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, out_key)  # reads memo output, subscribes
    _effect_end_run(w, rt, eid)

    assert_false(_effect_is_pending(w, rt, eid), "not pending after run")

    # Write to the base signal — this should:
    # 1. Mark memo dirty
    # 2. Propagate to memo output's subscribers (effect context)
    # 3. Mark effect pending
    _signal_write(w, rt, sig, Int32(10))

    assert_true(
        _effect_is_pending(w, rt, eid),
        "effect should be pending after memo's input signal changes",
    )

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_reads_memo_output")


# ═══════════════════════════════════════════════════════════════════════════════
# Effect destroy while pending — no crash
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_destroy_while_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Destroying a pending effect is safe and cleans up properly."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var eid = _effect_create(w, rt, scope)
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)

    # Make it pending
    _signal_write(w, rt, sig, Int32(1))
    assert_true(_effect_is_pending(w, rt, eid), "pending before destroy")

    # Destroy while pending
    _effect_destroy(w, rt, eid)
    assert_equal(_effect_count(w, rt), 0, "count should be 0")

    # Writing to the signal should not cause any issues
    _signal_write(w, rt, sig, Int32(2))

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_destroy_while_pending")


def test_effect_destroy_nonexistent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Destroying a non-existent effect is a no-op."""
    var rt = _create_runtime(w)

    # Destroy ID 99 which doesn't exist — should not crash
    _effect_destroy(w, rt, 99)
    assert_equal(_effect_count(w, rt), 0, "count remains 0")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_destroy_nonexistent")


# ═══════════════════════════════════════════════════════════════════════════════
# use_effect hook
# ═══════════════════════════════════════════════════════════════════════════════


def test_hook_use_effect_creates_on_first_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Use_effect on first render creates an effect and returns its ID."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var prev = _begin_render(w, rt, scope)
    var eid = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    assert_equal(eid, 0, "first effect should be ID 0")
    assert_equal(_effect_count(w, rt), 1, "one effect should exist")
    assert_true(_effect_is_pending(w, rt, eid), "new effect starts pending")

    _destroy_runtime(w, rt)
    print("  ✓ test_hook_use_effect_creates_on_first_render")


def test_hook_use_effect_returns_same_id_on_rerender(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Use_effect on re-render returns the same effect ID."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    # First render
    var prev = _begin_render(w, rt, scope)
    var eid1 = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    # Re-render
    prev = _begin_render(w, rt, scope)
    var eid2 = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    assert_equal(eid1, eid2, "same effect ID on re-render")
    assert_equal(_effect_count(w, rt), 1, "still only one effect")

    _destroy_runtime(w, rt)
    print("  ✓ test_hook_use_effect_returns_same_id_on_rerender")


def test_hook_use_effect_multiple_distinct_ids(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Multiple use_effect calls in one render create distinct effects."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var prev = _begin_render(w, rt, scope)
    var e0 = _hook_use_effect(w, rt)
    var e1 = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    assert_true(e0 != e1, "effect IDs should be distinct")
    assert_equal(_effect_count(w, rt), 2, "two effects should exist")

    # Re-render — same IDs
    prev = _begin_render(w, rt, scope)
    var e0b = _hook_use_effect(w, rt)
    var e1b = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    assert_equal(e0b, e0, "first effect stable on re-render")
    assert_equal(e1b, e1, "second effect stable on re-render")

    _destroy_runtime(w, rt)
    print("  ✓ test_hook_use_effect_multiple_distinct_ids")


def test_hook_use_effect_interleaved_with_signal_and_memo(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Hooks use_effect interleaved with use_signal and use_memo all remain stable.
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    # First render: signal, effect, memo, effect
    var prev = _begin_render(w, rt, scope)
    var sig = _hook_use_signal_i32(w, rt, Int32(0))
    var eff0 = _hook_use_effect(w, rt)
    var mem = _hook_use_memo_i32(w, rt, Int32(0))
    var eff1 = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    # Re-render: same order, same IDs
    prev = _begin_render(w, rt, scope)
    var sig2 = _hook_use_signal_i32(w, rt, Int32(0))
    var eff0b = _hook_use_effect(w, rt)
    var mem2 = _hook_use_memo_i32(w, rt, Int32(0))
    var eff1b = _hook_use_effect(w, rt)
    _end_render(w, rt, prev)

    assert_equal(sig2, sig, "signal hook stable")
    assert_equal(eff0b, eff0, "first effect hook stable")
    assert_equal(mem2, mem, "memo hook stable")
    assert_equal(eff1b, eff1, "second effect hook stable")

    _destroy_runtime(w, rt)
    print("  ✓ test_hook_use_effect_interleaved_with_signal_and_memo")


# ═══════════════════════════════════════════════════════════════════════════════
# Edge cases
# ═══════════════════════════════════════════════════════════════════════════════


def test_effect_multiple_writes_single_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Multiple writes to the same signal only produce one pending state (idempotent).
    """
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var eid = _effect_create(w, rt, scope)
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)

    # Multiple writes
    _signal_write(w, rt, sig, Int32(1))
    _signal_write(w, rt, sig, Int32(2))
    _signal_write(w, rt, sig, Int32(3))

    assert_true(_effect_is_pending(w, rt, eid), "effect is pending")
    assert_equal(_effect_drain_pending(w, rt), 1, "only one pending entry")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_multiple_writes_single_pending")


def test_effect_run_resubscribe_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Running an effect multiple times correctly re-subscribes each time."""
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)
    var sig = _signal_create(w, rt, Int32(0))

    var eid = _effect_create(w, rt, scope)

    # Run 1: subscribe
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)

    # Trigger pending
    _signal_write(w, rt, sig, Int32(1))
    assert_true(_effect_is_pending(w, rt, eid), "pending after write 1")

    # Run 2: re-subscribe
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)
    assert_false(_effect_is_pending(w, rt, eid), "not pending after run 2")

    # Trigger pending again
    _signal_write(w, rt, sig, Int32(2))
    assert_true(_effect_is_pending(w, rt, eid), "pending after write 2")

    # Run 3: still works
    _effect_begin_run(w, rt, eid)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, eid)
    assert_false(_effect_is_pending(w, rt, eid), "not pending after run 3")

    _destroy_runtime(w, rt)
    print("  ✓ test_effect_run_resubscribe_cycle")


# ═══════════════════════════════════════════════════════════════════════════════
# Test runner
# ═══════════════════════════════════════════════════════════════════════════════


fn test_all(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    # Store basics
    test_effect_create_returns_valid_id(w)
    test_effect_starts_pending(w)
    test_effect_has_context_id(w)
    test_effect_two_effects_distinct_ids(w)
    test_effect_destroy_reduces_count(w)
    test_effect_id_reuse_after_destroy(w)
    test_effect_destroy_cleans_up_context_signal(w)
    # Execution
    test_effect_begin_end_run_clears_pending(w)
    test_effect_signal_read_during_run_subscribes(w)
    # Propagation
    test_effect_signal_write_marks_pending(w)
    test_effect_signal_write_does_not_dirty_scope(w)
    test_effect_two_effects_same_signal(w)
    test_effect_scope_and_effect_both_react(w)
    test_effect_unsubscribed_signal_no_pending(w)
    # Dependency re-tracking
    test_effect_dependency_retracking(w)
    # Drain pending
    test_effect_drain_pending_returns_pending_effects(w)
    test_effect_drain_pending_after_partial_run(w)
    # Effect + Memo chain
    test_effect_reads_memo_output(w)
    # Destroy edge cases
    test_effect_destroy_while_pending(w)
    test_effect_destroy_nonexistent(w)
    # Hook
    test_hook_use_effect_creates_on_first_render(w)
    test_hook_use_effect_returns_same_id_on_rerender(w)
    test_hook_use_effect_multiple_distinct_ids(w)
    test_hook_use_effect_interleaved_with_signal_and_memo(w)
    # Edge cases
    test_effect_multiple_writes_single_pending(w)
    test_effect_run_resubscribe_cycle(w)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_all(w)
    print("effect: 63/63 passed")
