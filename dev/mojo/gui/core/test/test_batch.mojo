# Phase 38.1 — Batch Signal Writes Tests.
#
# Dedicated unit tests for `begin_batch()` / `end_batch()` at the runtime
# level.  These test the runtime directly via WASM exports, without going
# through any app layer.
#
# Tests cover:
#   1.  batch single signal — propagation happens on end_batch
#   2.  batch multi signal same memo — memo dirty once
#   3.  batch multi signal different memos — both dirty
#   4.  batch defers propagation — memo NOT dirty during batch
#   5.  batch scope dirty after end — scope NOT dirty during batch
#   6.  batch read sees new value — peek returns updated value
#   7.  batch read_signal subscribes — subscription tracked during batch
#   8.  batch empty noop — begin+end with no writes is safe
#   9.  batch nested — inner end_batch does NOT propagate
#  10.  batch nested depth 3 — only outermost propagates
#  11.  batch changed_signals populated — after end_batch
#  12.  batch string signal — version bumped, propagation deferred
#  13.  batch mixed types — I32 + string signals in one batch
#  14.  batch dedup keys — same key written twice, tracked once
#  15.  batch effect pending after end — effect NOT pending during batch
#  16.  batch memo worklist shared — diamond into one memo
#  17.  batch chain propagation — signal → memo_a → memo_b → scope
#  18.  batch settle after batch — stable memo, settle removes scope
#  19.  batch non-batch still works — regression guard
#  20.  batch end without begin — no crash
#  21.  batch is_batching flag — true during, false outside
#  22.  batch large batch — 20 signals in one batch
#
# Run with:
#   mojo test test/test_batch.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    args_ptr_i32_i32_ptr,
    args_ptr_i32_ptr,
    args_ptr_ptr,
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


fn _memo_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, memo_id: Int
) raises:
    w[].call_void("memo_destroy", args_ptr_i32(rt, Int32(memo_id)))


fn _has_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Bool:
    return w[].call_i32("runtime_has_dirty", args_ptr(rt)) != 0


fn _drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Int:
    return Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))


fn _begin_batch(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_begin_batch", args_ptr(rt))


fn _end_batch(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_end_batch", args_ptr(rt))


fn _is_batching(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises -> Bool:
    return w[].call_i32("runtime_is_batching", args_ptr(rt)) != 0


fn _signal_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int, key: Int
) raises -> Bool:
    return (
        w[].call_i32("runtime_signal_changed", args_ptr_i32(rt, Int32(key)))
        != 0
    )


fn _clear_changed_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_clear_changed_signals", args_ptr(rt))


fn _settle_scopes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_settle_scopes", args_ptr(rt))


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


fn _create_signal_string(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    initial: String,
) raises -> Int:
    """Create a string signal.  Returns packed i64: low 32 = string_key, high 32 = version_key.
    """
    var in_ptr = w[].write_string_struct(initial)
    return Int(w[].call_i64("signal_create_string", args_ptr_ptr(rt, in_ptr)))


fn _signal_string_key(packed: Int) raises -> Int:
    """Extract string_key (low 32 bits) from packed signal pair."""
    return packed & 0xFFFFFFFF


fn _signal_version_key(packed: Int) raises -> Int:
    """Extract version_key (high 32 bits) from packed signal pair."""
    return (packed >> 32) & 0xFFFFFFFF


fn _write_signal_string(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    string_key: Int,
    version_key: Int,
    value: String,
) raises:
    var str_ptr = w[].write_string_struct(value)
    w[].call_void(
        "signal_write_string",
        args_ptr_i32_i32_ptr(
            rt, Int32(string_key), Int32(version_key), str_ptr
        ),
    )


fn _peek_signal_string(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    rt: Int,
    string_key: Int,
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "signal_peek_string", args_ptr_i32_ptr(rt, Int32(string_key), out_ptr)
    )
    return w[].read_string_struct(out_ptr)


# ── Helper: subscribe scope/memo to a signal by reading during render ────────


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
# Test 1: batch single signal — propagation happens on end_batch
#
# Create signal → memo. begin_batch, write signal, end_batch.
# Assert memo is dirty after end_batch.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_single_signal() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute: subscribe memo to signal
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)  # 10

    # Clear tracking from setup
    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write
    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 7)
    _end_batch(w, rt)

    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty after end_batch",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 2: batch multi signal same memo — memo dirty once
#
# Two signals feed the same memo. Batch write both. Assert memo dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_multi_signal_same_memo() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig_a = _signal_create(w, rt, 3)
    var sig_b = _signal_create(w, rt, 4)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute: subscribe memo to both signals
    _memo_begin_compute(w, rt, memo)
    var va = Int32(_signal_read(w, rt, sig_a))
    var vb = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo, va + vb)  # 7

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write both signals
    _begin_batch(w, rt)
    _signal_write(w, rt, sig_a, 10)
    _signal_write(w, rt, sig_b, 20)
    _end_batch(w, rt)

    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty after batch write of both inputs",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 3: batch multi signal different memos — both dirty
#
# signal_a → memo_a, signal_b → memo_b. Batch write both.
# Assert both memos dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_multi_signal_different_memos() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig_a = _signal_create(w, rt, 1)
    var sig_b = _signal_create(w, rt, 2)
    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_create(w, rt, scope, 0)

    # Subscribe memo_a to sig_a
    _memo_begin_compute(w, rt, memo_a)
    var va = Int32(_signal_read(w, rt, sig_a))
    _memo_end_compute(w, rt, memo_a, va)

    # Subscribe memo_b to sig_b
    _memo_begin_compute(w, rt, memo_b)
    var vb = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo_b, vb)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write both
    _begin_batch(w, rt)
    _signal_write(w, rt, sig_a, 10)
    _signal_write(w, rt, sig_b, 20)
    _end_batch(w, rt)

    assert_true(
        _memo_is_dirty(w, rt, memo_a),
        "memo_a should be dirty after batch",
    )
    assert_true(
        _memo_is_dirty(w, rt, memo_b),
        "memo_b should be dirty after batch",
    )

    _memo_destroy(w, rt, memo_a)
    _memo_destroy(w, rt, memo_b)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 4: batch defers propagation — memo NOT dirty during batch
#
# begin_batch, write signal. Assert memo is NOT dirty yet.
# end_batch. Assert memo dirty now.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_defers_propagation() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 99)

    # During batch: memo should NOT be dirty (propagation deferred)
    assert_false(
        _memo_is_dirty(w, rt, memo),
        "memo should NOT be dirty during batch (propagation deferred)",
    )

    _end_batch(w, rt)

    # After end_batch: memo should be dirty
    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty after end_batch",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 5: batch scope dirty after end — scope NOT dirty during batch
#
# signal → scope (direct subscription). Batch write signal.
# Assert scope NOT dirty during batch. After end_batch, scope dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_scope_dirty_after_end() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 0)
    _subscribe_scope_to_signal(w, rt, scope, sig)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 42)

    assert_false(
        _has_dirty(w, rt),
        "scope should NOT be dirty during batch",
    )

    _end_batch(w, rt)

    assert_true(
        _has_dirty(w, rt),
        "scope should be dirty after end_batch",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 6: batch read sees new value — peek returns updated value
#
# begin_batch, write signal(42). peek returns 42 immediately.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_read_sees_new_value() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)

    var sig = _signal_create(w, rt, 0)

    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 42)

    var val = _signal_peek(w, rt, sig)
    assert_equal(val, 42, "peek should return 42 during batch")

    _end_batch(w, rt)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 7: batch read_signal subscribes — subscription tracked during batch
#
# begin_batch, read_signal in memo context. Assert subscription is tracked.
# After end_batch + write, memo is dirty (subscription was recorded).
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_read_signal_subscribes() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Subscribe memo to signal during a batch
    _begin_batch(w, rt)
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv)
    _end_batch(w, rt)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Write signal normally — memo should be dirty if subscription was tracked
    _signal_write(w, rt, sig, 99)
    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty (subscription was tracked during batch)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 8: batch empty noop — begin+end with no writes is safe
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_empty_noop() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    _begin_batch(w, rt)
    _end_batch(w, rt)

    assert_false(
        _has_dirty(w, rt),
        "no dirty scopes after empty batch",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 9: batch nested — inner end_batch does NOT propagate
#
# begin_batch, begin_batch (nested), write signal, end_batch (inner),
# assert memo NOT dirty. end_batch (outer). Assert memo dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_nested() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _begin_batch(w, rt)  # depth = 1
    _begin_batch(w, rt)  # depth = 2
    _signal_write(w, rt, sig, 99)
    _end_batch(w, rt)  # depth = 1 — inner, no propagation

    assert_false(
        _memo_is_dirty(w, rt, memo),
        "memo should NOT be dirty after inner end_batch",
    )

    _end_batch(w, rt)  # depth = 0 — outer, propagation runs

    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty after outer end_batch",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 10: batch nested depth 3 — only outermost propagates
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_nested_depth3() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 0)
    _subscribe_scope_to_signal(w, rt, scope, sig)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _begin_batch(w, rt)  # depth 1
    _begin_batch(w, rt)  # depth 2
    _begin_batch(w, rt)  # depth 3
    _signal_write(w, rt, sig, 42)
    _end_batch(w, rt)  # depth 2
    assert_false(_has_dirty(w, rt), "not dirty at depth 2")
    _end_batch(w, rt)  # depth 1
    assert_false(_has_dirty(w, rt), "not dirty at depth 1")
    _end_batch(w, rt)  # depth 0 — propagate

    assert_true(
        _has_dirty(w, rt),
        "scope should be dirty after outermost end_batch",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 11: batch changed_signals populated — after end_batch
#
# Batch write signal. After end_batch, signal_changed_this_cycle returns true.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_changed_signals_populated() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)

    var sig = _signal_create(w, rt, 0)
    _clear_changed_signals(w, rt)

    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 99)

    # During batch: changed_signals should NOT contain the key
    assert_false(
        _signal_changed(w, rt, sig),
        "signal should NOT be in _changed_signals during batch",
    )

    _end_batch(w, rt)

    assert_true(
        _signal_changed(w, rt, sig),
        "signal should be in _changed_signals after end_batch",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 12: batch string signal — version bumped, propagation deferred
#
# Create a string signal. Batch write. Assert version bumped during batch
# (reads see new version), but propagation deferred until end_batch.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_string_signal() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var packed = _create_signal_string(w, rt, String("hello"))
    var str_key = _signal_string_key(packed)
    var ver_key = _signal_version_key(packed)

    # Subscribe scope to the version signal
    _subscribe_scope_to_signal(w, rt, scope, ver_key)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    var ver_before = _signal_peek(w, rt, ver_key)

    _begin_batch(w, rt)
    _write_signal_string(w, rt, str_key, ver_key, String("world"))

    # Version signal is bumped immediately
    var ver_during = _signal_peek(w, rt, ver_key)
    assert_true(
        ver_during > ver_before,
        "version should be bumped during batch",
    )

    # But scope is NOT dirty (propagation deferred)
    assert_false(
        _has_dirty(w, rt),
        "scope should NOT be dirty during string batch",
    )

    _end_batch(w, rt)

    # Now scope is dirty
    assert_true(
        _has_dirty(w, rt),
        "scope should be dirty after string batch end",
    )

    # String value is accessible
    var val = _peek_signal_string(w, rt, str_key)
    assert_equal(val, String("world"), "string value should be 'world'")

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 13: batch mixed types — I32 + string signals in one batch
#
# Batch write an Int32 signal and a string signal together.
# Both should propagate on end_batch.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_mixed_types() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig_i32 = _signal_create(w, rt, 0)
    var packed = _create_signal_string(w, rt, String(""))
    var str_key = _signal_string_key(packed)
    var ver_key = _signal_version_key(packed)

    # Subscribe scope to both
    var prev = _scope_begin_render(w, rt, scope)
    _ = _signal_read(w, rt, sig_i32)
    _ = _signal_read(w, rt, ver_key)
    _scope_end_render(w, rt, prev)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write both
    _begin_batch(w, rt)
    _signal_write(w, rt, sig_i32, 42)
    _write_signal_string(w, rt, str_key, ver_key, String("test"))
    _end_batch(w, rt)

    assert_true(
        _has_dirty(w, rt),
        "scope should be dirty after mixed-type batch",
    )
    assert_equal(_signal_peek(w, rt, sig_i32), 42, "i32 value = 42")
    var s = _peek_signal_string(w, rt, str_key)
    assert_equal(s, String("test"), "string value = 'test'")

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 14: batch dedup keys — same key written twice, tracked once
#
# Write the same signal key twice in a batch. After end_batch,
# propagation should work correctly (key not duplicated).
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_dedup_keys() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 0)
    _subscribe_scope_to_signal(w, rt, scope, sig)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 10)
    _signal_write(w, rt, sig, 20)  # second write to same key
    _end_batch(w, rt)

    # Final value should be the last write
    assert_equal(
        _signal_peek(w, rt, sig), 20, "value should be last write (20)"
    )

    # Scope should be dirty exactly once
    assert_true(_has_dirty(w, rt), "scope dirty after dedup batch")
    var drained = _drain_dirty(w, rt)
    assert_equal(drained, 1, "exactly 1 dirty scope (not duplicated)")

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 15: batch effect pending after end — effect NOT pending during batch
#
# signal → effect. Batch write. Assert effect NOT pending during batch.
# end_batch. Assert effect pending.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_effect_pending_after_end() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 0)
    var effect = _effect_create(w, rt, scope)

    # Subscribe effect to signal
    _effect_begin_run(w, rt, effect)
    _ = _signal_read(w, rt, sig)
    _effect_end_run(w, rt, effect)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 42)

    assert_false(
        _effect_is_pending(w, rt, effect),
        "effect should NOT be pending during batch",
    )

    _end_batch(w, rt)

    assert_true(
        _effect_is_pending(w, rt, effect),
        "effect should be pending after end_batch",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 16: batch memo worklist shared — diamond into one memo
#
# signal_a and signal_b both feed memo (A+B). Batch write both.
# end_batch. Assert memo dirty (added to worklist once, not twice).
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_memo_worklist_shared() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig_a = _signal_create(w, rt, 1)
    var sig_b = _signal_create(w, rt, 2)
    var memo = _memo_create(w, rt, scope, 0)

    # Subscribe memo to both signals
    _memo_begin_compute(w, rt, memo)
    var va = Int32(_signal_read(w, rt, sig_a))
    var vb = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo, va + vb)  # 3

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write both
    _begin_batch(w, rt)
    _signal_write(w, rt, sig_a, 10)
    _signal_write(w, rt, sig_b, 20)
    _end_batch(w, rt)

    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty (shared worklist)",
    )

    # Recompute memo
    _memo_begin_compute(w, rt, memo)
    var va2 = Int32(_signal_read(w, rt, sig_a))
    var vb2 = Int32(_signal_read(w, rt, sig_b))
    _memo_end_compute(w, rt, memo, va2 + vb2)

    # Value should be 30
    var result = Int32(_memo_read(w, rt, memo))
    assert_equal(result, Int32(30), "memo value should be 30 (10+20)")

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 17: batch chain propagation — signal → memo_a → memo_b → scope
#
# Full chain: Batch write signal. end_batch. Assert memo_a dirty,
# memo_b dirty (via worklist), scope dirty.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_chain_propagation() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo_a = _memo_create(w, rt, scope, 0)
    var memo_b = _memo_create(w, rt, scope, 0)

    # Chain: sig → memo_a(×2) → memo_b(+1)
    _memo_begin_compute(w, rt, memo_a)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo_a, sv * 2)  # 10

    _memo_begin_compute(w, rt, memo_b)
    var av = Int32(_memo_read(w, rt, memo_a))
    _memo_end_compute(w, rt, memo_b, av + 1)  # 11

    # Subscribe scope to memo_b's output
    var out_b = _memo_output_key(w, rt, memo_b)
    _subscribe_scope_to_signal(w, rt, scope, out_b)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write signal
    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 99)
    _end_batch(w, rt)

    assert_true(
        _memo_is_dirty(w, rt, memo_a),
        "memo_a should be dirty (direct subscriber)",
    )
    assert_true(
        _memo_is_dirty(w, rt, memo_b),
        "memo_b should be dirty (worklist propagation)",
    )
    assert_true(
        _has_dirty(w, rt),
        "scope should be dirty (end of chain)",
    )

    _memo_destroy(w, rt, memo_b)
    _memo_destroy(w, rt, memo_a)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 18: batch settle after batch — stable memo, settle removes scope
#
# Batch write same value. end_batch. Recompute memo (stable).
# settle_scopes. Assert scope removed.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_settle_after_batch() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 5)
    var memo = _memo_create(w, rt, scope, 0)

    # Initial compute
    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv * 2)  # 10

    var out_key = _memo_output_key(w, rt, memo)
    _subscribe_scope_to_signal(w, rt, scope, out_key)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write same value
    _begin_batch(w, rt)
    _signal_write(w, rt, sig, 5)
    _end_batch(w, rt)

    assert_true(_has_dirty(w, rt), "scope dirty after batch")

    # Recompute memo: 5×2=10 == 10 → stable
    _memo_begin_compute(w, rt, memo)
    var sv2 = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv2 * 2)

    # Settle: scope subscribes to stable memo output → removed
    _settle_scopes(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "scope should be removed after settle (memo stable)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 19: batch non-batch still works — regression guard
#
# Write signal outside of any batch. Assert immediate propagation.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_non_batch_still_works() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    var sig = _signal_create(w, rt, 0)
    var memo = _memo_create(w, rt, scope, 0)

    _memo_begin_compute(w, rt, memo)
    var sv = Int32(_signal_read(w, rt, sig))
    _memo_end_compute(w, rt, memo, sv)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Normal write (no batch) — immediate propagation
    _signal_write(w, rt, sig, 42)
    assert_true(
        _memo_is_dirty(w, rt, memo),
        "memo should be dirty immediately (no batch, regression guard)",
    )

    _memo_destroy(w, rt, memo)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 20: batch end without begin — no crash
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_end_without_begin() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)

    # end_batch without begin_batch — should be a safe no-op
    _end_batch(w, rt)
    assert_false(
        _has_dirty(w, rt),
        "no dirty scopes after orphan end_batch (no crash)",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 21: batch is_batching flag
#
# is_batching returns false initially, true after begin_batch,
# false after end_batch.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_is_batching_flag() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)

    assert_false(
        _is_batching(w, rt),
        "is_batching should be false initially",
    )

    _begin_batch(w, rt)
    assert_true(
        _is_batching(w, rt),
        "is_batching should be true after begin_batch",
    )

    _begin_batch(w, rt)  # nested
    assert_true(
        _is_batching(w, rt),
        "is_batching should be true when nested",
    )

    _end_batch(w, rt)  # still depth 1
    assert_true(
        _is_batching(w, rt),
        "is_batching should be true at depth 1",
    )

    _end_batch(w, rt)  # depth 0
    assert_false(
        _is_batching(w, rt),
        "is_batching should be false after outermost end_batch",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Test 22: batch large batch — 20 signals in one batch
#
# Write 20 signals in one batch. Assert all propagated correctly.
# ══════════════════════════════════════════════════════════════════════════════


fn test_batch_large_batch() raises:
    var w = _get_wasm()
    var rt = _create_runtime(w)
    var scope = _scope_create(w, rt, 0, -1)

    # Create 20 signals, one memo per signal
    var sigs = List[Int]()
    var memos = List[Int]()
    for i in range(20):
        var s = _signal_create(w, rt, Int32(i))
        sigs.append(s)
        var m = _memo_create(w, rt, scope, 0)
        # Subscribe memo to its signal
        _memo_begin_compute(w, rt, m)
        var sv = Int32(_signal_read(w, rt, s))
        _memo_end_compute(w, rt, m, sv)
        memos.append(m)

    _clear_changed_signals(w, rt)
    _ = _drain_dirty(w, rt)

    # Batch write all 20
    _begin_batch(w, rt)
    for i in range(20):
        _signal_write(w, rt, sigs[i], Int32(i + 100))
    _end_batch(w, rt)

    # All 20 memos should be dirty
    var all_dirty = True
    for i in range(20):
        if not _memo_is_dirty(w, rt, memos[i]):
            all_dirty = False
            break

    assert_true(all_dirty, "all 20 memos should be dirty after large batch")

    # Verify signal values
    for i in range(20):
        var val = _signal_peek(w, rt, sigs[i])
        assert_equal(
            val,
            Int(i + 100),
            "signal " + String(i) + " should be " + String(i + 100),
        )

    # Cleanup
    for i in range(20):
        _memo_destroy(w, rt, memos[i])
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Main — run all tests
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var total = 22
    var passed = 0
    var failed = 0

    # ── Test 1 ──
    try:
        test_batch_single_signal()
        passed += 1
    except e:
        print("FAIL test_batch_single_signal:", e)
        failed += 1

    # ── Test 2 ──
    try:
        test_batch_multi_signal_same_memo()
        passed += 1
    except e:
        print("FAIL test_batch_multi_signal_same_memo:", e)
        failed += 1

    # ── Test 3 ──
    try:
        test_batch_multi_signal_different_memos()
        passed += 1
    except e:
        print("FAIL test_batch_multi_signal_different_memos:", e)
        failed += 1

    # ── Test 4 ──
    try:
        test_batch_defers_propagation()
        passed += 1
    except e:
        print("FAIL test_batch_defers_propagation:", e)
        failed += 1

    # ── Test 5 ──
    try:
        test_batch_scope_dirty_after_end()
        passed += 1
    except e:
        print("FAIL test_batch_scope_dirty_after_end:", e)
        failed += 1

    # ── Test 6 ──
    try:
        test_batch_read_sees_new_value()
        passed += 1
    except e:
        print("FAIL test_batch_read_sees_new_value:", e)
        failed += 1

    # ── Test 7 ──
    try:
        test_batch_read_signal_subscribes()
        passed += 1
    except e:
        print("FAIL test_batch_read_signal_subscribes:", e)
        failed += 1

    # ── Test 8 ──
    try:
        test_batch_empty_noop()
        passed += 1
    except e:
        print("FAIL test_batch_empty_noop:", e)
        failed += 1

    # ── Test 9 ──
    try:
        test_batch_nested()
        passed += 1
    except e:
        print("FAIL test_batch_nested:", e)
        failed += 1

    # ── Test 10 ──
    try:
        test_batch_nested_depth3()
        passed += 1
    except e:
        print("FAIL test_batch_nested_depth3:", e)
        failed += 1

    # ── Test 11 ──
    try:
        test_batch_changed_signals_populated()
        passed += 1
    except e:
        print("FAIL test_batch_changed_signals_populated:", e)
        failed += 1

    # ── Test 12 ──
    try:
        test_batch_string_signal()
        passed += 1
    except e:
        print("FAIL test_batch_string_signal:", e)
        failed += 1

    # ── Test 13 ──
    try:
        test_batch_mixed_types()
        passed += 1
    except e:
        print("FAIL test_batch_mixed_types:", e)
        failed += 1

    # ── Test 14 ──
    try:
        test_batch_dedup_keys()
        passed += 1
    except e:
        print("FAIL test_batch_dedup_keys:", e)
        failed += 1

    # ── Test 15 ──
    try:
        test_batch_effect_pending_after_end()
        passed += 1
    except e:
        print("FAIL test_batch_effect_pending_after_end:", e)
        failed += 1

    # ── Test 16 ──
    try:
        test_batch_memo_worklist_shared()
        passed += 1
    except e:
        print("FAIL test_batch_memo_worklist_shared:", e)
        failed += 1

    # ── Test 17 ──
    try:
        test_batch_chain_propagation()
        passed += 1
    except e:
        print("FAIL test_batch_chain_propagation:", e)
        failed += 1

    # ── Test 18 ──
    try:
        test_batch_settle_after_batch()
        passed += 1
    except e:
        print("FAIL test_batch_settle_after_batch:", e)
        failed += 1

    # ── Test 19 ──
    try:
        test_batch_non_batch_still_works()
        passed += 1
    except e:
        print("FAIL test_batch_non_batch_still_works:", e)
        failed += 1

    # ── Test 20 ──
    try:
        test_batch_end_without_begin()
        passed += 1
    except e:
        print("FAIL test_batch_end_without_begin:", e)
        failed += 1

    # ── Test 21 ──
    try:
        test_batch_is_batching_flag()
        passed += 1
    except e:
        print("FAIL test_batch_is_batching_flag:", e)
        failed += 1

    # ── Test 22 ──
    try:
        test_batch_large_batch()
        passed += 1
    except e:
        print("FAIL test_batch_large_batch:", e)
        failed += 1

    # ── Summary ──
    print("batch:", String(passed) + "/" + String(total), "passed")
    if failed > 0:
        raise Error(String(failed) + " of " + String(total) + " tests FAILED")
