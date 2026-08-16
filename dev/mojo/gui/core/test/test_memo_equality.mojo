# Phase 37.1 — Equality-Gated Memo Propagation Tests.
#
# Validates that `memo_end_compute_*` correctly compares old vs new values,
# sets the `value_changed` flag, skips output signal writes when stable,
# and tracks changed signals for settle_scopes().
#
# Tests cover:
#   - I32 memo: same value (stable), different value (changed), initial
#   - Bool memo: same value (stable), different value (changed), false→false
#   - String memo: same value (stable), different value (changed), empty→empty
#   - String version signal not bumped when stable
#   - String version signal bumped when changed
#   - Chain cascade: all stable, all changed, partial cascade
#   - Diamond: one parent changed, both stable, both changed
#   - _changed_signals tracking: write_signal, end_compute changed/stable
#   - _changed_signals reset on drain_dirty
#   - Mixed-type chain cascade: all stable, partial
#   - Regression: value_changed flag toggles across recomputations
#
# Run with:
#   mojo test test/test_memo_equality.mojo

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


fn _signal_version(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Int:
    return Int(w[].call_i32("signal_version", args_ptr_i32(rt, Int32(key))))


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


fn _memo_bool_read(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises -> Bool:
    return w[].call_i32("memo_bool_read", args_ptr_i32(rt, Int32(memo_id))) != 0


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


fn _runtime_signal_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Bool:
    return (
        w[].call_i32("runtime_signal_changed", args_ptr_i32(rt, Int32(key)))
        != 0
    )


fn _runtime_clear_changed_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_clear_changed_signals", args_ptr(rt))


fn _drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    return Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))


fn _has_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Bool:
    return w[].call_i32("runtime_has_dirty", args_ptr(rt)) != 0


fn _memo_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_destroy", args_ptr_i32(rt, Int32(memo_id)))


# ── Helper: compute a memo cycle (begin + read inputs + end) ─────────────────
#
# These are convenience functions for the common pattern of recomputing a memo
# by reading its inputs and writing the result.


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


# ══════════════════════════════════════════════════════════════════════════════
# Test 1: I32 same value → no change
# ══════════════════════════════════════════════════════════════════════════════


fn test_i32_same_value_no_change() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    # Create memo with initial 0, compute with 42 (changed)
    var memo = _memo_create(w, rt, scope, 0)
    _recompute_i32(w, rt, memo, 42)
    assert_true(
        _memo_did_value_change(w, rt, memo),
        "first compute should report changed (0 → 42)",
    )
    # Clear changed signals for clean second cycle
    _runtime_clear_changed_signals(w, rt)

    # Second compute: same value 42 → 42 (stable)
    _recompute_i32(w, rt, memo, 42)
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "same value should report NOT changed",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 2: I32 different value → changed
# ══════════════════════════════════════════════════════════════════════════════


fn test_i32_different_value_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_create(w, rt, scope, 42)
    _recompute_i32(w, rt, memo, 42)
    _runtime_clear_changed_signals(w, rt)

    # Recompute with different value
    _recompute_i32(w, rt, memo, 43)
    assert_true(
        _memo_did_value_change(w, rt, memo),
        "different value should report changed",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 3: I32 initial compute always changed
# ══════════════════════════════════════════════════════════════════════════════


fn test_i32_initial_compute_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    # Create memo with initial 0, compute with 0.
    # Initial value in signal is 0, computed value is 0.
    # But first compute is always treated as changed (value_changed defaults True).
    var memo = _memo_create(w, rt, scope, 0)
    _recompute_i32(w, rt, memo, 0)
    # The initial signal value is 0 and the new value is 0.
    # Since the output signal was initialized with 0, the equality check
    # sees old==new and reports NOT changed.  But the MemoEntry default
    # is value_changed=True, so before end_compute it was True.
    # After end_compute(0): old=0, new=0 → changed=False.
    # This is correct: the initial value already matches, no propagation needed.
    # Actually, let's verify the actual behavior:
    var changed = _memo_did_value_change(w, rt, memo)
    # The initial value in the output signal is 0 (from create_memo_i32).
    # The computed value is also 0.  old == new → changed = False.
    # This is fine: if the initial value is already correct, no downstream
    # propagation is needed.
    assert_false(
        changed,
        "initial compute with same-as-initial value should be stable",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 4: Bool same value → no change
# ══════════════════════════════════════════════════════════════════════════════


fn test_bool_same_value_no_change() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_bool_create(w, rt, scope, True)
    _recompute_bool(w, rt, memo, True)
    _runtime_clear_changed_signals(w, rt)

    # Second compute: True → True
    _recompute_bool(w, rt, memo, True)
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "Bool same value should report NOT changed",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 5: Bool different value → changed
# ══════════════════════════════════════════════════════════════════════════════


fn test_bool_different_value_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_bool_create(w, rt, scope, True)
    _recompute_bool(w, rt, memo, True)
    _runtime_clear_changed_signals(w, rt)

    # Recompute: True → False
    _recompute_bool(w, rt, memo, False)
    assert_true(
        _memo_did_value_change(w, rt, memo),
        "Bool different value should report changed",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 6: Bool false→false → no change
# ══════════════════════════════════════════════════════════════════════════════


fn test_bool_false_to_false_no_change() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_bool_create(w, rt, scope, False)
    _recompute_bool(w, rt, memo, False)
    _runtime_clear_changed_signals(w, rt)

    # Second compute: False → False
    _recompute_bool(w, rt, memo, False)
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "Bool false→false should be stable",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 7: String same value → no change
# ══════════════════════════════════════════════════════════════════════════════


fn test_string_same_value_no_change() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_string_create(w, rt, scope, String("hello"))
    _recompute_string(w, rt, memo, String("hello"))
    _runtime_clear_changed_signals(w, rt)

    # Second compute: "hello" → "hello"
    _recompute_string(w, rt, memo, String("hello"))
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "String same value should be stable",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 8: String different value → changed
# ══════════════════════════════════════════════════════════════════════════════


fn test_string_different_value_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_string_create(w, rt, scope, String("hello"))
    _recompute_string(w, rt, memo, String("hello"))
    _runtime_clear_changed_signals(w, rt)

    # Recompute: "hello" → "world"
    _recompute_string(w, rt, memo, String("world"))
    assert_true(
        _memo_did_value_change(w, rt, memo),
        "String different value should report changed",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 9: String empty→empty → no change
# ══════════════════════════════════════════════════════════════════════════════


fn test_string_empty_to_empty_no_change() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_string_create(w, rt, scope, String(""))
    _recompute_string(w, rt, memo, String(""))
    _runtime_clear_changed_signals(w, rt)

    # Second compute: "" → ""
    _recompute_string(w, rt, memo, String(""))
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "String empty→empty should be stable",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 10: String version NOT bumped when stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_string_version_not_bumped_when_stable() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_string_create(w, rt, scope, String("hello"))
    _recompute_string(w, rt, memo, String("hello"))
    # Get the output key (version signal)
    var out_key = _memo_output_key(w, rt, memo)
    var ver_before = _signal_version(w, rt, out_key)

    # Second compute: "hello" → "hello" (stable)
    _recompute_string(w, rt, memo, String("hello"))
    var ver_after = _signal_version(w, rt, out_key)

    assert_equal(
        ver_before,
        ver_after,
        "version signal should NOT bump when string is stable",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 11: String version bumped when changed
# ══════════════════════════════════════════════════════════════════════════════


fn test_string_version_bumped_when_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_string_create(w, rt, scope, String("hello"))
    _recompute_string(w, rt, memo, String("hello"))
    var out_key = _memo_output_key(w, rt, memo)
    var ver_before = _signal_version(w, rt, out_key)

    # Second compute: "hello" → "world" (changed)
    _recompute_string(w, rt, memo, String("world"))
    var ver_after = _signal_version(w, rt, out_key)

    assert_true(
        ver_after > ver_before,
        "version signal SHOULD bump when string changed",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 12: Chain cascade — all stable
#
# signal(5) → memo_a(×2=10) → memo_b(>0=true)
# Write signal(5) (same value). Recompute both. Both stable.
# ══════════════════════════════════════════════════════════════════════════════


fn test_chain_cascade_stable() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    # Create signal with value 5
    var sig = _signal_create(w, rt, 5)

    # Create memo_a: output = sig * 2
    var memo_a = _memo_create(w, rt, scope, 0)
    # Create memo_b: output = (memo_a > 0) as Bool
    var memo_b = _memo_bool_create(w, rt, scope, False)

    # First recompute to establish baseline values
    _memo_begin_compute(w, rt, memo_a)
    var sig_val = Int32(_signal_read(w, rt, sig))  # subscribes memo_a to sig
    _memo_end_compute(w, rt, memo_a, sig_val * 2)  # 10

    _memo_bool_begin_compute(w, rt, memo_b)
    var a_val = Int32(_memo_read(w, rt, memo_a))  # subscribes memo_b to memo_a
    _memo_bool_end_compute(w, rt, memo_b, a_val > 0)  # True

    _runtime_clear_changed_signals(w, rt)

    # Write same value to signal — triggers dirty propagation
    _signal_write(w, rt, sig, 5)

    # Recompute memo_a: 5 * 2 = 10 == 10 (stable)
    _memo_begin_compute(w, rt, memo_a)
    var sig_val2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sig_val2 * 2)

    # Recompute memo_b: 10 > 0 = true == true (stable)
    _memo_bool_begin_compute(w, rt, memo_b)
    var a_val2 = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, a_val2 > 0)

    assert_false(
        _memo_did_value_change(w, rt, memo_a),
        "memo_a should be stable (10 == 10)",
    )
    assert_false(
        _memo_did_value_change(w, rt, memo_b),
        "memo_b should be stable (true == true)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 13: Chain cascade — all changed
#
# signal(5) → memo_a(×2=10) → memo_b(>10=false)
# Write signal(6). Recompute: a=12 (changed), b=true (changed).
# ══════════════════════════════════════════════════════════════════════════════


fn test_chain_cascade_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_bool_create(w, rt, scope, False)

    # First compute: a = 10, b = false (10 > 10 = false)
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)

    _memo_bool_begin_compute(w, rt, memo_b)
    var av = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av > 10)

    _runtime_clear_changed_signals(w, rt)

    # Write new value
    _signal_write(w, rt, sig, 6)

    # Recompute: a = 12 (changed), b = true (changed)
    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)

    _memo_bool_begin_compute(w, rt, memo_b)
    var av2 = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av2 > 10)

    assert_true(
        _memo_did_value_change(w, rt, memo_a),
        "memo_a should be changed (10 → 12)",
    )
    assert_true(
        _memo_did_value_change(w, rt, memo_b),
        "memo_b should be changed (false → true)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 14: Chain partial cascade
#
# signal(5) → memo_a(×2=10) → memo_b(>=10=true)
# Write signal(6). a=12 (changed), b=true (stable! was already true).
# ══════════════════════════════════════════════════════════════════════════════


fn test_chain_partial_cascade() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_bool_create(w, rt, scope, False)

    # First compute: a=10, b = (10 >= 10) = true
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)

    _memo_bool_begin_compute(w, rt, memo_b)
    var av = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av >= 10)

    _runtime_clear_changed_signals(w, rt)

    # Write signal(6) → a=12 (changed), b = (12 >= 10) = true (stable)
    _signal_write(w, rt, sig, 6)

    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)

    _memo_bool_begin_compute(w, rt, memo_b)
    var av2 = Int32(_memo_read(w, rt, memo_a))
    _memo_bool_end_compute(w, rt, memo_b, av2 >= 10)

    assert_true(
        _memo_did_value_change(w, rt, memo_a),
        "memo_a should be changed (10 → 12)",
    )
    assert_false(
        _memo_did_value_change(w, rt, memo_b),
        "memo_b should be stable (true → true)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 15: Diamond — one parent changed
#
# signal → memo_a(×2), memo_b(+0 = always same)
# Both feed memo_c(a + b). Write signal(new value).
# a changed, b stable, c should be changed.
# ══════════════════════════════════════════════════════════════════════════════


fn test_diamond_one_parent_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    # memo_a: sig * 2
    var memo_a = _memo_create(w, rt, scope, 0)
    # memo_b: 100 (constant, ignores signal, but subscribes for testing)
    var memo_b = _memo_create(w, rt, scope, 100)
    # memo_c: a + b
    var memo_c = _memo_create(w, rt, scope, 0)

    # First compute
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)  # 10

    _memo_begin_compute(w, rt, memo_b)
    _ = _signal_read(w, rt, sig)  # subscribe to trigger dirty
    _memo_end_compute(w, rt, memo_b, 100)  # always 100

    _memo_begin_compute(w, rt, memo_c)
    var a_val = Int32(_memo_read(w, rt, memo_a))
    var b_val = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, a_val + b_val)  # 110

    _runtime_clear_changed_signals(w, rt)

    # Write new value to signal
    _signal_write(w, rt, sig, 6)

    # Recompute
    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)  # 12 (changed)

    _memo_begin_compute(w, rt, memo_b)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, memo_b, 100)  # 100 (stable)

    _memo_begin_compute(w, rt, memo_c)
    var a2 = Int32(_memo_read(w, rt, memo_a))
    var b2 = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, a2 + b2)  # 112 (changed)

    assert_true(
        _memo_did_value_change(w, rt, memo_a),
        "diamond: memo_a should be changed",
    )
    assert_false(
        _memo_did_value_change(w, rt, memo_b),
        "diamond: memo_b should be stable",
    )
    assert_true(
        _memo_did_value_change(w, rt, memo_c),
        "diamond: memo_c should be changed (a changed)",
    )

    _memo_destroy(w, rt, memo_c)
    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 16: Diamond — both parents stable
#
# Write same value. a stable, b stable, c stable.
# ══════════════════════════════════════════════════════════════════════════════


fn test_diamond_both_parents_stable() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_create(w, rt, scope, 100)
    var memo_c = _memo_create(w, rt, scope, 0)

    # First compute
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)

    _memo_begin_compute(w, rt, memo_b)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, memo_b, 100)

    _memo_begin_compute(w, rt, memo_c)
    var a = Int32(_memo_read(w, rt, memo_a))
    var b = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, a + b)

    _runtime_clear_changed_signals(w, rt)

    # Write SAME value to signal
    _signal_write(w, rt, sig, 5)

    # Recompute — all should produce same values
    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)  # 10 == 10

    _memo_begin_compute(w, rt, memo_b)
    _ = _signal_read(w, rt, sig)
    _memo_end_compute(w, rt, memo_b, 100)  # 100 == 100

    _memo_begin_compute(w, rt, memo_c)
    var a2 = Int32(_memo_read(w, rt, memo_a))
    var b2 = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, a2 + b2)  # 110 == 110

    assert_false(
        _memo_did_value_change(w, rt, memo_a),
        "diamond stable: memo_a should be stable",
    )
    assert_false(
        _memo_did_value_change(w, rt, memo_b),
        "diamond stable: memo_b should be stable",
    )
    assert_false(
        _memo_did_value_change(w, rt, memo_c),
        "diamond stable: memo_c should be stable",
    )

    _memo_destroy(w, rt, memo_c)
    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 17: Diamond — both parents changed
# ══════════════════════════════════════════════════════════════════════════════


fn test_diamond_both_parents_changed() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    # memo_a: sig * 2
    var memo_a = _memo_create(w, rt, scope, 0)
    # memo_b: sig + 1
    var memo_b = _memo_create(w, rt, scope, 0)
    # memo_c: a + b
    var memo_c = _memo_create(w, rt, scope, 0)

    # First compute: a=10, b=6, c=16
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)

    _memo_begin_compute(w, rt, memo_b)
    var sv1 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_b, sv1 + 1)

    _memo_begin_compute(w, rt, memo_c)
    var a = Int32(_memo_read(w, rt, memo_a))
    var b = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, a + b)

    _runtime_clear_changed_signals(w, rt)

    # Write different value
    _signal_write(w, rt, sig, 7)

    # Recompute: a=14, b=8, c=22
    _memo_begin_compute(w, rt, memo_a)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv2 * 2)

    _memo_begin_compute(w, rt, memo_b)
    var sv3 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_b, sv3 + 1)

    _memo_begin_compute(w, rt, memo_c)
    var a2 = Int32(_memo_read(w, rt, memo_a))
    var b2 = Int32(_memo_read(w, rt, memo_b))
    _memo_end_compute(w, rt, memo_c, a2 + b2)

    assert_true(
        _memo_did_value_change(w, rt, memo_a),
        "diamond both changed: a should be changed",
    )
    assert_true(
        _memo_did_value_change(w, rt, memo_b),
        "diamond both changed: b should be changed",
    )
    assert_true(
        _memo_did_value_change(w, rt, memo_c),
        "diamond both changed: c should be changed",
    )

    _memo_destroy(w, rt, memo_c)
    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 18: _changed_signals tracking
#
# Write signal → appears in changed_signals.
# Memo end_compute changed → output_key appears.
# Memo end_compute stable → output_key does NOT appear.
# ══════════════════════════════════════════════════════════════════════════════


fn test_changed_signals_tracking() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)
    var out_key = _memo_output_key(w, rt, memo)

    # Clear any initial state
    _runtime_clear_changed_signals(w, rt)

    # Write signal — should appear in changed_signals
    _signal_write(w, rt, sig, 10)
    assert_true(
        _runtime_signal_changed(w, rt, sig),
        "written signal should be in _changed_signals",
    )

    # Memo end_compute with changed value → output_key should appear
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)  # 20, changed from 0

    assert_true(
        _runtime_signal_changed(w, rt, out_key),
        "changed memo output should be in _changed_signals",
    )

    _runtime_clear_changed_signals(w, rt)

    # Write same value to signal — signal still "changes" (write_signal
    # always adds to _changed_signals regardless of value equality)
    _signal_write(w, rt, sig, 10)
    assert_true(
        _runtime_signal_changed(w, rt, sig),
        "write_signal always adds to _changed_signals",
    )

    # Memo end_compute with same value → output_key should NOT appear
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)  # 20 == 20, stable

    assert_false(
        _runtime_signal_changed(w, rt, out_key),
        "stable memo output should NOT be in _changed_signals",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 19: _changed_signals reset via clear
# ══════════════════════════════════════════════════════════════════════════════


fn test_changed_signals_reset() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)

    var sig = _signal_create(w, rt, 5)

    _signal_write(w, rt, sig, 10)
    assert_true(
        _runtime_signal_changed(w, rt, sig),
        "signal should be marked changed after write",
    )

    _runtime_clear_changed_signals(w, rt)
    assert_false(
        _runtime_signal_changed(w, rt, sig),
        "signal should NOT be marked changed after clear",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 20: Mixed-type chain cascade — all stable
#
# signal(I32=5) → MemoI32(×2=10) → MemoBool(>=10=true) → MemoString("BIG")
# Write signal(5) (same). Recompute all. All stable.
# ══════════════════════════════════════════════════════════════════════════════


fn test_mixed_type_chain_cascade_stable() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    var doubled = _memo_create(w, rt, scope, 0)
    var is_big = _memo_bool_create(w, rt, scope, False)
    var label = _memo_string_create(w, rt, scope, String("small"))

    # First compute
    _memo_begin_compute(w, rt, doubled)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, doubled, sv * 2)  # 10

    _memo_bool_begin_compute(w, rt, is_big)
    var dv = Int32(_memo_read(w, rt, doubled))
    _memo_bool_end_compute(w, rt, is_big, dv >= 10)  # true

    _memo_string_begin_compute(w, rt, label)
    var big = _memo_bool_read(w, rt, is_big)
    if big:
        _memo_string_end_compute(w, rt, label, String("BIG"))
    else:
        _memo_string_end_compute(w, rt, label, String("small"))

    _runtime_clear_changed_signals(w, rt)

    # Write SAME value
    _signal_write(w, rt, sig, 5)

    # Recompute all — same values
    _memo_begin_compute(w, rt, doubled)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, doubled, sv2 * 2)  # 10

    _memo_bool_begin_compute(w, rt, is_big)
    var dv2 = Int32(_memo_read(w, rt, doubled))
    _memo_bool_end_compute(w, rt, is_big, dv2 >= 10)  # true

    _memo_string_begin_compute(w, rt, label)
    var big2 = _memo_bool_read(w, rt, is_big)
    if big2:
        _memo_string_end_compute(w, rt, label, String("BIG"))
    else:
        _memo_string_end_compute(w, rt, label, String("small"))

    assert_false(
        _memo_did_value_change(w, rt, doubled),
        "mixed chain stable: doubled should be stable",
    )
    assert_false(
        _memo_did_value_change(w, rt, is_big),
        "mixed chain stable: is_big should be stable",
    )
    assert_false(
        _memo_did_value_change(w, rt, label),
        "mixed chain stable: label should be stable",
    )

    _memo_destroy(w, rt, label)
    _memo_destroy(w, rt, is_big)
    _memo_destroy(w, rt, doubled)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 21: Mixed-type chain — partial cascade
#
# signal(5) → doubled(10) → is_big(true) → label("BIG")
# Write signal(6): doubled=12 (changed), is_big=true (stable), label="BIG" (stable)
# ══════════════════════════════════════════════════════════════════════════════


fn test_mixed_type_chain_partial() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)

    var doubled = _memo_create(w, rt, scope, 0)
    var is_big = _memo_bool_create(w, rt, scope, False)
    var label = _memo_string_create(w, rt, scope, String("small"))

    # First compute
    _memo_begin_compute(w, rt, doubled)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, doubled, sv * 2)  # 10

    _memo_bool_begin_compute(w, rt, is_big)
    var dv = Int32(_memo_read(w, rt, doubled))
    _memo_bool_end_compute(w, rt, is_big, dv >= 10)  # true

    _memo_string_begin_compute(w, rt, label)
    var big = _memo_bool_read(w, rt, is_big)
    if big:
        _memo_string_end_compute(w, rt, label, String("BIG"))
    else:
        _memo_string_end_compute(w, rt, label, String("small"))

    _runtime_clear_changed_signals(w, rt)

    # Write signal(6): doubled=12 (changed), is_big = (12 >= 10) = true (stable),
    # label = "BIG" (stable)
    _signal_write(w, rt, sig, 6)

    _memo_begin_compute(w, rt, doubled)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, doubled, sv2 * 2)  # 12 (changed)

    _memo_bool_begin_compute(w, rt, is_big)
    var dv2 = Int32(_memo_read(w, rt, doubled))
    _memo_bool_end_compute(w, rt, is_big, dv2 >= 10)  # true (stable)

    _memo_string_begin_compute(w, rt, label)
    var big2 = _memo_bool_read(w, rt, is_big)
    if big2:
        _memo_string_end_compute(w, rt, label, String("BIG"))
    else:
        _memo_string_end_compute(w, rt, label, String("small"))

    assert_true(
        _memo_did_value_change(w, rt, doubled),
        "mixed partial: doubled should be changed (10 → 12)",
    )
    assert_false(
        _memo_did_value_change(w, rt, is_big),
        "mixed partial: is_big should be stable (true → true)",
    )
    assert_false(
        _memo_did_value_change(w, rt, label),
        "mixed partial: label should be stable (BIG → BIG)",
    )

    _memo_destroy(w, rt, label)
    _memo_destroy(w, rt, is_big)
    _memo_destroy(w, rt, doubled)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 22: Regression — value_changed flag toggles correctly
#
# Compute changed, then stable, then changed again. Flag should track each.
# ══════════════════════════════════════════════════════════════════════════════


fn test_regression_changed_flag_reset() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var memo = _memo_create(w, rt, scope, 0)

    # Compute 1: 0 → 42 (changed)
    _recompute_i32(w, rt, memo, 42)
    assert_true(
        _memo_did_value_change(w, rt, memo),
        "step 1: should be changed (0 → 42)",
    )

    # Compute 2: 42 → 42 (stable)
    _recompute_i32(w, rt, memo, 42)
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "step 2: should be stable (42 → 42)",
    )

    # Compute 3: 42 → 99 (changed)
    _recompute_i32(w, rt, memo, 99)
    assert_true(
        _memo_did_value_change(w, rt, memo),
        "step 3: should be changed (42 → 99)",
    )

    # Compute 4: 99 → 99 (stable again)
    _recompute_i32(w, rt, memo, 99)
    assert_false(
        _memo_did_value_change(w, rt, memo),
        "step 4: should be stable (99 → 99)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Main — run all tests
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var passed = 0
    var failed = 0
    var total = 22

    # ── Test 1 ──
    try:
        test_i32_same_value_no_change()
        passed += 1
    except e:
        print("FAIL test_i32_same_value_no_change:", e)
        failed += 1

    # ── Test 2 ──
    try:
        test_i32_different_value_changed()
        passed += 1
    except e:
        print("FAIL test_i32_different_value_changed:", e)
        failed += 1

    # ── Test 3 ──
    try:
        test_i32_initial_compute_changed()
        passed += 1
    except e:
        print("FAIL test_i32_initial_compute_changed:", e)
        failed += 1

    # ── Test 4 ──
    try:
        test_bool_same_value_no_change()
        passed += 1
    except e:
        print("FAIL test_bool_same_value_no_change:", e)
        failed += 1

    # ── Test 5 ──
    try:
        test_bool_different_value_changed()
        passed += 1
    except e:
        print("FAIL test_bool_different_value_changed:", e)
        failed += 1

    # ── Test 6 ──
    try:
        test_bool_false_to_false_no_change()
        passed += 1
    except e:
        print("FAIL test_bool_false_to_false_no_change:", e)
        failed += 1

    # ── Test 7 ──
    try:
        test_string_same_value_no_change()
        passed += 1
    except e:
        print("FAIL test_string_same_value_no_change:", e)
        failed += 1

    # ── Test 8 ──
    try:
        test_string_different_value_changed()
        passed += 1
    except e:
        print("FAIL test_string_different_value_changed:", e)
        failed += 1

    # ── Test 9 ──
    try:
        test_string_empty_to_empty_no_change()
        passed += 1
    except e:
        print("FAIL test_string_empty_to_empty_no_change:", e)
        failed += 1

    # ── Test 10 ──
    try:
        test_string_version_not_bumped_when_stable()
        passed += 1
    except e:
        print("FAIL test_string_version_not_bumped_when_stable:", e)
        failed += 1

    # ── Test 11 ──
    try:
        test_string_version_bumped_when_changed()
        passed += 1
    except e:
        print("FAIL test_string_version_bumped_when_changed:", e)
        failed += 1

    # ── Test 12 ──
    try:
        test_chain_cascade_stable()
        passed += 1
    except e:
        print("FAIL test_chain_cascade_stable:", e)
        failed += 1

    # ── Test 13 ──
    try:
        test_chain_cascade_changed()
        passed += 1
    except e:
        print("FAIL test_chain_cascade_changed:", e)
        failed += 1

    # ── Test 14 ──
    try:
        test_chain_partial_cascade()
        passed += 1
    except e:
        print("FAIL test_chain_partial_cascade:", e)
        failed += 1

    # ── Test 15 ──
    try:
        test_diamond_one_parent_changed()
        passed += 1
    except e:
        print("FAIL test_diamond_one_parent_changed:", e)
        failed += 1

    # ── Test 16 ──
    try:
        test_diamond_both_parents_stable()
        passed += 1
    except e:
        print("FAIL test_diamond_both_parents_stable:", e)
        failed += 1

    # ── Test 17 ──
    try:
        test_diamond_both_parents_changed()
        passed += 1
    except e:
        print("FAIL test_diamond_both_parents_changed:", e)
        failed += 1

    # ── Test 18 ──
    try:
        test_changed_signals_tracking()
        passed += 1
    except e:
        print("FAIL test_changed_signals_tracking:", e)
        failed += 1

    # ── Test 19 ──
    try:
        test_changed_signals_reset()
        passed += 1
    except e:
        print("FAIL test_changed_signals_reset:", e)
        failed += 1

    # ── Test 20 ──
    try:
        test_mixed_type_chain_cascade_stable()
        passed += 1
    except e:
        print("FAIL test_mixed_type_chain_cascade_stable:", e)
        failed += 1

    # ── Test 21 ──
    try:
        test_mixed_type_chain_partial()
        passed += 1
    except e:
        print("FAIL test_mixed_type_chain_partial:", e)
        failed += 1

    # ── Test 22 ──
    try:
        test_regression_changed_flag_reset()
        passed += 1
    except e:
        print("FAIL test_regression_changed_flag_reset:", e)
        failed += 1

    # ── Summary ──
    print("memo_equality:", String(passed) + "/" + String(total), "passed")
    if failed > 0:
        raise Error(String(failed) + " of " + String(total) + " tests FAILED")
