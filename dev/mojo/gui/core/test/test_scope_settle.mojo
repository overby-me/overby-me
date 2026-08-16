# Phase 37.6 — Scope Settle Tests.
#
# Dedicated unit tests for `settle_scopes()` at the runtime level.
# These test the runtime directly via WASM exports, without going
# through any app layer.
#
# Tests cover:
#   1.  settle removes scope when memo is value-stable
#   2.  settle keeps scope when memo value changed
#   3.  settle with mixed scopes (one stable, one changed)
#   4.  settle keeps scope that subscribes directly to source signal
#   5.  settle keeps scope subscribing to both stable memo and changed signal
#   6.  settle with no dirty scopes (no crash)
#   7.  settle when all scopes are stable (both removed)
#   8.  settle with no changed signals (all scopes removed)
#   9.  settle with 3-level chain cascade (all stable)
#  10.  settle with chain partial (A changed, B stable → scope removed)
#  11.  settle with chain fully changed (scope kept)
#  12.  settle with diamond dependency (one parent changed → scope kept)
#  13.  settle with direct signal subscription (scope kept)
#  14.  settle does not affect pending effects
#  15.  settle is idempotent (calling twice is safe)
#  16.  settle after no memos (signals and scopes only)
#
# Run with:
#   mojo test test/test_scope_settle.mojo

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
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
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


fn _signal_peek(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    return Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, Int32(key))))


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


fn _memo_output_key(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Int:
    return Int(
        w[].call_i32("memo_output_key", args_ptr_i32(rt, Int32(memo_id)))
    )


fn _memo_did_value_change(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    return (
        w[].call_i32("memo_did_value_change", args_ptr_i32(rt, Int32(memo_id)))
        != 0
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


fn _memo_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_destroy", args_ptr_i32(rt, Int32(memo_id)))


fn _settle_scopes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_settle_scopes", args_ptr(rt))


fn _clear_changed_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_clear_changed_signals", args_ptr(rt))


fn _signal_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Bool:
    return (
        w[].call_i32("runtime_signal_changed", args_ptr_i32(rt, Int32(key)))
        != 0
    )


fn _has_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Bool:
    return w[].call_i32("runtime_has_dirty", args_ptr(rt)) != 0


fn _drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    return Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))


fn _effect_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, scope_id: Int
) raises -> Int:
    return Int(w[].call_i32("effect_create", args_ptr_i32(rt, Int32(scope_id))))


fn _effect_is_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises -> Bool:
    return (
        w[].call_i32("effect_is_pending", args_ptr_i32(rt, Int32(effect_id)))
        != 0
    )


fn _effect_begin_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    w[].call_void("effect_begin_run", args_ptr_i32(rt, Int32(effect_id)))


fn _effect_end_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, effect_id: Int
) raises:
    w[].call_void("effect_end_run", args_ptr_i32(rt, Int32(effect_id)))


# ── Helper: recompute a memo (begin + end) ───────────────────────────────────


fn _recompute_i32(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Int32,
) raises:
    """Begin + end compute a memo with a pre-calculated value."""
    _memo_begin_compute(w, rt, memo_id)
    _memo_end_compute(w, rt, memo_id, value)


fn _recompute_bool(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: Bool,
) raises:
    """Begin + end compute a bool memo with a pre-calculated value."""
    _memo_bool_begin_compute(w, rt, memo_id)
    _memo_bool_end_compute(w, rt, memo_id, value)


fn _recompute_string(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    memo_id: Int,
    value: String,
) raises:
    """Begin + end compute a string memo with a pre-calculated value."""
    _memo_string_begin_compute(w, rt, memo_id)
    _memo_string_end_compute(w, rt, memo_id, value)


# ── Helper: subscribe scope to a signal by rendering ─────────────────────────


fn _subscribe_scope_to_signal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    scope_id: Int,
    signal_key: Int,
) raises:
    """Subscribe a scope to a signal by reading during a scope render pass."""
    var prev = _scope_begin_render(w, rt, scope_id)
    _ = _signal_read(w, rt, signal_key)
    _scope_end_render(w, rt, prev)


# ══════════════════════════════════════════════════════════════════════════════
# Test 1: settle removes scope when memo is value-stable
#
# signal(5) → memo(×2=10), scope subscribes to memo's output.
# Write same value (5). Recompute memo (10==10 → stable).
# settle_scopes() should remove the scope from dirty_scopes.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_removes_scope_when_no_change() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute: subscribe memo to signal, value = 10
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)

    # Subscribe scope to memo's output signal
    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)

    # Clear tracking from setup
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value → triggers dirty propagation
    _signal_write(w, rt, sig, 5)
    assert_true(_has_dirty(w, rt), "scope should be dirty after write")

    # Recompute memo: 5 × 2 = 10 == 10 → stable
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)

    assert_false(
        _memo_did_value_change(w, rt, memo),
        "memo should be stable (10 == 10)",
    )

    # Settle: scope subscribes to stable memo output → removed
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "dirty_scope_count should be 0 after settle (stable memo)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 2: settle keeps scope when memo value changed
#
# signal(5) → memo(×2=10), scope subscribes to memo's output.
# Write new value (6). Recompute memo (12 != 10 → changed).
# settle_scopes() should keep the scope in dirty_scopes.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_keeps_scope_when_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)  # 10

    # Subscribe scope to memo's output
    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write new value
    _signal_write(w, rt, sig, 6)
    assert_true(_has_dirty(w, rt), "scope should be dirty after write")

    # Recompute memo: 6 × 2 = 12 != 10 → changed
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)

    assert_true(
        _memo_did_value_change(w, rt, memo),
        "memo should be changed (10 → 12)",
    )

    # Settle: scope subscribes to changed memo output → kept
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should still be dirty after settle (changed memo)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 3: settle with mixed scopes — one stable, one changed
#
# signal(5) → memo_a(×2=10), signal(3) → memo_b(×2=6).
# scope_a subscribes to memo_a output, scope_b subscribes to memo_b output.
# Write same value to sig_a (stable), new value to sig_b (changed).
# After settle: scope_a removed, scope_b kept.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_mixed_scopes() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope_a = _scope_create(w, rt, 0, -1)
    var scope_b = _scope_create(w, rt, 0, -1)

    var sig_a = _signal_create(w, rt, 5)
    var sig_b = _signal_create(w, rt, 3)
    var memo_a = _memo_create(w, rt, scope_a, 0)
    var memo_b = _memo_create(w, rt, scope_b, 0)

    # Initial compute for both memos
    _memo_begin_compute(w, rt, memo_a)
    var sva = Int32(_signal_read(w, rt, sig_a))
    _memo_end_compute(w, rt, memo_a, sva * 2)  # 10

    _memo_begin_compute(w, rt, memo_b)
    var svb = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo_b, svb * 2)  # 6

    # Subscribe scopes to their respective memo outputs
    var out_a = _memo_output_key(w, rt, memo_a)
    var out_b = _memo_output_key(w, rt, memo_b)
    _subscribe_scope_to_signal(w, rt, scope_a, out_a)
    _subscribe_scope_to_signal(w, rt, scope_b, out_b)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value to sig_a (memo_a stable), new value to sig_b (memo_b changed)
    _signal_write(w, rt, sig_a, 5)
    _signal_write(w, rt, sig_b, 7)

    # Recompute both memos
    _memo_begin_compute(w, rt, memo_a)
    var sva2 = Int32(_signal_read(w, rt, sig_a))
    _memo_end_compute(w, rt, memo_a, sva2 * 2)  # 10 == 10 → stable

    _memo_begin_compute(w, rt, memo_b)
    var svb2 = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo_b, svb2 * 2)  # 14 != 6 → changed

    assert_false(
        _memo_did_value_change(w, rt, memo_a),
        "memo_a should be stable",
    )
    assert_true(
        _memo_did_value_change(w, rt, memo_b),
        "memo_b should be changed",
    )

    # Settle: scope_a removed (stable), scope_b kept (changed)
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "at least one scope should remain dirty (scope_b)",
    )
    # Drain and verify exactly 1 scope is dirty
    var drained = _drain_dirty(w, rt)
    assert_equal(drained, 1, "exactly 1 dirty scope should remain after settle")

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 4: settle keeps scope that subscribes directly to source signal
#
# Scope subscribes to a source signal (no memo). Write signal. Settle.
# Source signal writes always add to _changed_signals, so scope stays dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_scope_subscribes_to_signal() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 42)

    # Subscribe scope to signal directly
    _subscribe_scope_to_signal(w, rt, scope, sig)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write signal (source signals always count as changed)
    _signal_write(w, rt, sig, 99)
    assert_true(_has_dirty(w, rt), "scope should be dirty after write")

    # Settle: source signal is in _changed_signals → scope kept
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should remain dirty (subscribes to changed source signal)",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 5: settle keeps scope subscribing to stable memo AND changed signal
#
# Scope subscribes to a stable memo output AND a changed source signal.
# The changed source signal keeps the scope dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_scope_subscribes_to_both() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig_src = _signal_create(w, rt, 10)  # source signal
    var sig_memo_in = _signal_create(w, rt, 5)  # memo input signal
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute for memo
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig_memo_in))
    _memo_end_compute(w, rt, memo, sv * 2)  # 10

    # Subscribe scope to both the memo output and the source signal
    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)
    # Second render pass to also subscribe to source signal
    var prev = _scope_begin_render(w, rt, scope)
    _ = _signal_read(w, rt, out_key)
    _ = _signal_read(w, rt, sig_src)
    _scope_end_render(w, rt, prev)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value to memo input (memo stable), write source signal (changed)
    _signal_write(w, rt, sig_memo_in, 5)
    _signal_write(w, rt, sig_src, 99)

    # Recompute memo: 5 × 2 = 10 == 10 → stable
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig_memo_in))
    _memo_end_compute(w, rt, memo, sv2 * 2)

    assert_false(
        _memo_did_value_change(w, rt, memo),
        "memo should be stable",
    )

    # Settle: source signal is changed → scope kept
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should remain dirty (source signal changed)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 6: settle with no dirty scopes — no crash, still 0
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_no_dirty_scopes() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)

    # No scopes, no signals, no memos — just settle
    assert_false(_has_dirty(w, rt), "no dirty scopes initially")
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "still no dirty scopes after settle (no crash)",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 7: settle when all scopes are stable — both removed
#
# Two scopes, both subscribe to stable memo outputs. Both are removed.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_all_stable() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope_a = _scope_create(w, rt, 0, -1)
    var scope_b = _scope_create(w, rt, 0, -1)

    var sig_a = _signal_create(w, rt, 5)
    var sig_b = _signal_create(w, rt, 3)
    var memo_a = _memo_create(w, rt, scope_a, 0)
    var memo_b = _memo_create(w, rt, scope_b, 0)

    # Initial compute
    _memo_begin_compute(w, rt, memo_a)
    var sva = Int32(_signal_read(w, rt, sig_a))
    _memo_end_compute(w, rt, memo_a, sva * 2)  # 10

    _memo_begin_compute(w, rt, memo_b)
    var svb = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo_b, svb * 2)  # 6

    # Subscribe scopes to memo outputs
    var out_a = _memo_output_key(w, rt, memo_a)
    var out_b = _memo_output_key(w, rt, memo_b)
    _subscribe_scope_to_signal(w, rt, scope_a, out_a)
    _subscribe_scope_to_signal(w, rt, scope_b, out_b)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same values to both signals
    _signal_write(w, rt, sig_a, 5)
    _signal_write(w, rt, sig_b, 3)
    assert_true(_has_dirty(w, rt), "scopes should be dirty after writes")

    # Recompute both memos — both stable
    _memo_begin_compute(w, rt, memo_a)
    var sva2 = Int32(_signal_read(w, rt, sig_a))
    _memo_end_compute(w, rt, memo_a, sva2 * 2)  # 10 == 10

    _memo_begin_compute(w, rt, memo_b)
    var svb2 = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo_b, svb2 * 2)  # 6 == 6

    # Settle: both stable → both removed
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "no dirty scopes should remain (both memos stable)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 8: settle with no changed signals — all scopes removed
#
# Dirty scopes exist but _changed_signals is empty (cleared manually).
# settle should remove all dirty scopes because no signal changed.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_no_changed_signals() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute + subscribe
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)

    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)

    # Clear, then write to make scope dirty
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _signal_write(w, rt, sig, 6)  # scope dirtied
    assert_true(_has_dirty(w, rt), "scope should be dirty")

    # Recompute memo (changed, but we'll clear the tracking)
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)

    # Manually clear _changed_signals to simulate empty state
    _clear_changed_signals(w, rt)

    # Settle with empty _changed_signals → all scopes removed
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "all scopes removed when _changed_signals is empty",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 9: settle with 3-level chain cascade — all stable
#
# signal(5) → A(×2=10) → B(+1=11) → C(×3=33)
# Scope subscribes to C's output. Write same value (5). Recompute all.
# All stable → scope removed.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_chain_cascade_all_stable() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_create(w, rt, scope, 0)
    var memo_c = _memo_create(w, rt, scope, 0)

    # Initial compute chain: A reads sig, B reads A, C reads B
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)  # 10

    _memo_begin_compute(w, rt, memo_b)
    var av = Int32(_memo_read(w, rt, memo_a))
    _memo_end_compute(w, rt, memo_b, av + 1)  # 11

    _memo_begin_compute(w, rt, memo_c)
    var bv = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, bv * 3)  # 33

    # Subscribe scope to C's output
    var out_c = _memo_output_key(w, rt, memo_c)
    _subscribe_scope_to_signal(w, rt, scope, out_c)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value
    _signal_write(w, rt, sig, 5)

    # Recompute chain with same values → all stable
    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)  # 10 == 10

    _memo_begin_compute(w, rt, memo_b)
    var av2 = Int32(_memo_read(w, rt, memo_a))
    _memo_end_compute(w, rt, memo_b, av2 + 1)  # 11 == 11

    _memo_begin_compute(w, rt, memo_c)
    var bv2 = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, bv2 * 3)  # 33 == 33

    assert_false(_memo_did_value_change(w, rt, memo_a), "A stable")
    assert_false(_memo_did_value_change(w, rt, memo_b), "B stable")
    assert_false(_memo_did_value_change(w, rt, memo_c), "C stable")

    # Settle: scope removed
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "scope should be removed (3-level chain all stable)",
    )

    _memo_destroy(w, rt, memo_c)
    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 10: settle with chain partial — A changed, B stable
#
# signal(5) → A(×2=10) → B(>= 10 → true)
# Write signal(6) → A = 12 (changed). B = (12 >= 10) = true (stable!).
# Scope subscribes to B's output → scope removed.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_chain_partial() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_bool_create(w, rt, scope, False)

    # Initial compute: A = 10, B = (10 >= 10) = true
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)  # 10

    _memo_bool_begin_compute(w, rt, memo_b)
    var av = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av >= 10)  # true

    # Subscribe scope to B's output
    var out_b = _memo_output_key(w, rt, memo_b)
    _subscribe_scope_to_signal(w, rt, scope, out_b)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write new value → A changes, but B stays true
    _signal_write(w, rt, sig, 6)

    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)  # 12 != 10 → changed

    _memo_bool_begin_compute(w, rt, memo_b)
    var av2 = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av2 >= 10)  # true == true → stable

    assert_true(
        _memo_did_value_change(w, rt, memo_a),
        "A should be changed (10 → 12)",
    )
    assert_false(
        _memo_did_value_change(w, rt, memo_b),
        "B should be stable (true == true)",
    )

    # Settle: scope subscribes to B's output which is stable → removed
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "scope should be removed (B is stable, scope only subscribes to B)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 11: settle with chain fully changed — scope kept
#
# signal(5) → A(×2=10) → B(> 10 → false)
# Write signal(6) → A = 12 (changed), B = (12 > 10) = true (changed).
# Scope subscribes to B's output → scope kept.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_chain_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_bool_create(w, rt, scope, False)

    # Initial compute: A = 10, B = (10 > 10) = false
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)  # 10

    _memo_bool_begin_compute(w, rt, memo_b)
    var av = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av > 10)  # false

    # Subscribe scope to B's output
    var out_b = _memo_output_key(w, rt, memo_b)
    _subscribe_scope_to_signal(w, rt, scope, out_b)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write new value → A changes, B changes
    _signal_write(w, rt, sig, 6)

    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)  # 12 != 10

    _memo_bool_begin_compute(w, rt, memo_b)
    var av2 = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av2 > 10)  # true != false

    assert_true(
        _memo_did_value_change(w, rt, memo_a),
        "A should be changed (10 → 12)",
    )
    assert_true(
        _memo_did_value_change(w, rt, memo_b),
        "B should be changed (false → true)",
    )

    # Settle: B's output changed → scope kept
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should remain dirty (B changed)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 12: settle with diamond dependency — one parent changed
#
# sig → A (×2), sig → B (×3).  C reads A and B.
# Write same value → A stable, B stable → C stable → scope removed.
# Then write new value → A changed, B changed → C changed → scope kept.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_diamond() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_create(w, rt, scope, 0)
    var memo_c = _memo_create(w, rt, scope, 0)

    # Initial compute: A = 10, B = 15, C = A + B = 25
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)  # 10

    _memo_begin_compute(w, rt, memo_b)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_b, sv2 * 3)  # 15

    _memo_begin_compute(w, rt, memo_c)
    var av = Int32(_memo_read(w, rt, memo_a))
    var bv = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, av + bv)  # 25

    # Subscribe scope to C's output
    var out_c = _memo_output_key(w, rt, memo_c)
    _subscribe_scope_to_signal(w, rt, scope, out_c)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value → all stable
    _signal_write(w, rt, sig, 5)

    _memo_begin_compute(w, rt, memo_a)
    var sv3 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv3 * 2)  # 10 == 10

    _memo_begin_compute(w, rt, memo_b)
    var sv4 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_b, sv4 * 3)  # 15 == 15

    _memo_begin_compute(w, rt, memo_c)
    var av2 = Int32(_memo_read(w, rt, memo_a))
    var bv2 = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, av2 + bv2)  # 25 == 25

    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "scope should be removed (diamond all stable)",
    )

    # Now write new value → all changed
    _signal_write(w, rt, sig, 7)

    _memo_begin_compute(w, rt, memo_a)
    var sv5 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv5 * 2)  # 14 != 10

    _memo_begin_compute(w, rt, memo_b)
    var sv6 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_b, sv6 * 3)  # 21 != 15

    _memo_begin_compute(w, rt, memo_c)
    var av3 = Int32(_memo_read(w, rt, memo_a))
    var bv3 = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, av3 + bv3)  # 35 != 25

    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should be dirty (diamond all changed)",
    )

    _memo_destroy(w, rt, memo_c)
    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 13: settle with direct signal subscription — scope kept
#
# Scope subscribes to raw source signal (no memo in between).
# Signal is written. settle_scopes() runs. Scope stays dirty because
# write_signal adds source signal to _changed_signals.
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_with_direct_signal_sub() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 0)

    # Subscribe scope to raw signal
    _subscribe_scope_to_signal(w, rt, scope, sig)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write new value
    _signal_write(w, rt, sig, 1)
    assert_true(_has_dirty(w, rt), "scope dirty after signal write")

    # Settle: write_signal adds source to _changed_signals → scope kept
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should stay dirty (direct signal subscription, changed)",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 14: settle does not affect pending effects
#
# An effect is pending, memo was stable. settle_scopes() runs.
# Effect should still be pending (settle only affects scopes).
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_effect_not_affected() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    # Create an effect and subscribe it to the signal
    var effect = _effect_create(w, rt, scope)
    # Initial run: subscribe effect to signal
    _effect_begin_run(w, rt, effect)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, effect)

    # Create a memo to have something to settle
    var memo = _memo_create(w, rt, scope, 0)
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)  # 10

    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)

    # Clear tracking
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value → memo stable but effect gets dirtied (re-pending)
    _signal_write(w, rt, sig, 5)

    # Recompute memo (stable)
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)  # 10 == 10

    assert_false(
        _memo_did_value_change(w, rt, memo),
        "memo should be stable",
    )

    # Effect should be pending (signal was written, effect subscribed)
    assert_true(
        _effect_is_pending(w, rt, effect),
        "effect should be pending before settle",
    )

    # Settle scopes: scope removed (stable memo), but effect unaffected
    _settle_scopes(w, rt)

    assert_true(
        _effect_is_pending(w, rt, effect),
        (
            "effect should still be pending after settle (settle only affects"
            " scopes)"
        ),
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 15: settle is idempotent — calling twice is safe
#
# Call settle_scopes() twice in a row. Assert same result on second call
# (no crash, no double-removal, _changed_signals cleared by first call).
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_idempotent() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)

    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)

    # Clear
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write same value → stable
    _signal_write(w, rt, sig, 5)

    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)  # stable

    # First settle
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "scope removed after first settle",
    )

    # Second settle — should be no-op, no crash
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "still no dirty scopes after second settle (idempotent)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 16: settle after no memos — signals and scopes only
#
# App has signals and scopes but no memos. Write signal. Settle.
# Scope stays dirty (signal change is tracked directly by write_signal).
# ══════════════════════════════════════════════════════════════════════════════


fn test_settle_after_no_memos() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 42)

    # Subscribe scope to signal directly (no memo involved)
    _subscribe_scope_to_signal(w, rt, scope, sig)

    # Clear
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write signal
    _signal_write(w, rt, sig, 43)
    assert_true(_has_dirty(w, rt), "scope dirty after write")

    # Settle: signal is in _changed_signals → scope kept
    _settle_scopes(w, rt)
    assert_true(
        _has_dirty(w, rt),
        "scope should remain dirty (no memos, signal change tracked directly)",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Main — run all tests
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var total = 16
    var passed = 0
    var failed = 0

    # ── Test 1 ──
    try:
        test_settle_removes_scope_when_no_change()
        passed += 1
    except e:
        print("FAIL test_settle_removes_scope_when_no_change:", e)
        failed += 1

    # ── Test 2 ──
    try:
        test_settle_keeps_scope_when_changed()
        passed += 1
    except e:
        print("FAIL test_settle_keeps_scope_when_changed:", e)
        failed += 1

    # ── Test 3 ──
    try:
        test_settle_mixed_scopes()
        passed += 1
    except e:
        print("FAIL test_settle_mixed_scopes:", e)
        failed += 1

    # ── Test 4 ──
    try:
        test_settle_scope_subscribes_to_signal()
        passed += 1
    except e:
        print("FAIL test_settle_scope_subscribes_to_signal:", e)
        failed += 1

    # ── Test 5 ──
    try:
        test_settle_scope_subscribes_to_both()
        passed += 1
    except e:
        print("FAIL test_settle_scope_subscribes_to_both:", e)
        failed += 1

    # ── Test 6 ──
    try:
        test_settle_no_dirty_scopes()
        passed += 1
    except e:
        print("FAIL test_settle_no_dirty_scopes:", e)
        failed += 1

    # ── Test 7 ──
    try:
        test_settle_all_stable()
        passed += 1
    except e:
        print("FAIL test_settle_all_stable:", e)
        failed += 1

    # ── Test 8 ──
    try:
        test_settle_no_changed_signals()
        passed += 1
    except e:
        print("FAIL test_settle_no_changed_signals:", e)
        failed += 1

    # ── Test 9 ──
    try:
        test_settle_chain_cascade_all_stable()
        passed += 1
    except e:
        print("FAIL test_settle_chain_cascade_all_stable:", e)
        failed += 1

    # ── Test 10 ──
    try:
        test_settle_chain_partial()
        passed += 1
    except e:
        print("FAIL test_settle_chain_partial:", e)
        failed += 1

    # ── Test 11 ──
    try:
        test_settle_chain_changed()
        passed += 1
    except e:
        print("FAIL test_settle_chain_changed:", e)
        failed += 1

    # ── Test 12 ──
    try:
        test_settle_diamond()
        passed += 1
    except e:
        print("FAIL test_settle_diamond:", e)
        failed += 1

    # ── Test 13 ──
    try:
        test_settle_with_direct_signal_sub()
        passed += 1
    except e:
        print("FAIL test_settle_with_direct_signal_sub:", e)
        failed += 1

    # ── Test 14 ──
    try:
        test_settle_effect_not_affected()
        passed += 1
    except e:
        print("FAIL test_settle_effect_not_affected:", e)
        failed += 1

    # ── Test 15 ──
    try:
        test_settle_idempotent()
        passed += 1
    except e:
        print("FAIL test_settle_idempotent:", e)
        failed += 1

    # ── Test 16 ──
    try:
        test_settle_after_no_memos()
        passed += 1
    except e:
        print("FAIL test_settle_after_no_memos:", e)
        failed += 1

    # ── Summary ──
    print("scope_settle:", String(passed) + "/" + String(total), "passed")
    if failed > 0:
        raise Error(String(failed) + " of " + String(total) + " tests FAILED")
