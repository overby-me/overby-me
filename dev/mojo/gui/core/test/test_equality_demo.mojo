"""Phase 37.3 — EqualityDemoApp Mojo Tests.

Validates EqualityDemoApp via the eq_* WASM exports which exercise
an equality-gated memo chain: SignalI32 → MemoI32(clamp) → MemoString(label).

  - init creates app with non-zero pointer
  - initial state: input=0, clamped=0, label="low"
  - increment within range: clamped changes, label stable
  - increment across threshold (5→6): label "low"→"high" (changed)
  - increment at max (10→11): clamped stable, label stable
  - increment above max (15→16): clamped stable, label stable
  - decrement within range: clamped changes, label stable
  - decrement across threshold (6→5): label "high"→"low" (changed)
  - decrement at min (0→-1): clamped stable
  - full cycle: 0→12→0 round-trip
  - label NOT dirty after clamped stabilizes
  - scope settled when all stable (flush returns 0)
  - scope dirty when label changed (flush returns >0)
  - flush returns 0 when stable chain
  - flush returns nonzero when changed
  - handle_event marks dirty
  - memo count is 2
  - destroy does not crash
  - initial compute: both memos report value_changed correctly
  - consecutive stable flushes all return 0

Run with:
  mojo test test/test_equality_demo.mojo
"""

from memory import UnsafePointer

from testing import assert_equal, assert_true, assert_false
from wasm_harness import (
    WasmInstance,
    get_instance,
    no_args,
    args_i32,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_ptr,
    args_ptr_ptr_i32,
)


fn _load() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_eq(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create an EqualityDemoApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("eq_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("eq_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _create_eq_no_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create an EqualityDemoApp without mounting.  Returns (app_ptr, buf_ptr).
    """
    var app = Int(w[].call_i64("eq_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    return Tuple(app, buf)


fn _destroy_eq(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy an EqualityDemoApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("eq_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("eq_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("eq_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _incr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises:
    """Increment input via the button handler."""
    var hid = w[].call_i32("eq_incr_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)


fn _decr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises:
    """Decrement input via the button handler."""
    var hid = w[].call_i32("eq_decr_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)


fn _input(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("eq_input_value", args_ptr(app))


fn _clamped(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("eq_clamped_value", args_ptr(app))


fn _label_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("eq_label_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _clamped_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("eq_clamped_dirty", args_ptr(app)) != 0


fn _label_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("eq_label_dirty", args_ptr(app)) != 0


fn _clamped_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("eq_clamped_changed", args_ptr(app)) != 0


fn _label_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("eq_label_changed", args_ptr(app)) != 0


fn _has_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("eq_has_dirty", args_ptr(app)) != 0


fn _scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("eq_scope_count", args_ptr(app))


fn _memo_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("eq_memo_count", args_ptr(app))


# ── Helper: increment N times and flush ───────────────────────────────────────


fn _incr_and_flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
    n: Int,
) raises -> Int32:
    """Increment N times and flush once.  Returns flush byte count."""
    for _ in range(n):
        _incr(w, app)
    return _flush(w, app, buf)


fn _decr_and_flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
    n: Int,
) raises -> Int32:
    """Decrement N times and flush once.  Returns flush byte count."""
    for _ in range(n):
        _decr(w, app)
    return _flush(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 1: Initial state
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_initial_state() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    assert_equal(_input(w, app), Int32(0), "initial input should be 0")
    assert_equal(_clamped(w, app), Int32(0), "initial clamped should be 0")
    assert_equal(
        _label_text(w, app), String("low"), "initial label should be 'low'"
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 2: Increment within range — clamped changes, label stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_incr_within_range() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment 0→1: clamped 0→1 (changed), label "low"→"low" (stable)
    _ = _incr_and_flush(w, app, buf, 1)

    assert_equal(_input(w, app), Int32(1), "input should be 1")
    assert_equal(_clamped(w, app), Int32(1), "clamped should be 1")
    assert_equal(
        _label_text(w, app), String("low"), "label should still be 'low'"
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 3: Increment across threshold — label changes
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_incr_across_threshold() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 5 → clamped=5, label="low" (5 > 5 is false)
    _ = _incr_and_flush(w, app, buf, 5)
    assert_equal(_clamped(w, app), Int32(5), "clamped should be 5")
    assert_equal(
        _label_text(w, app), String("low"), "label at 5 should be 'low'"
    )

    # Increment to 6 → clamped=6, label="high" (6 > 5 is true)
    _ = _incr_and_flush(w, app, buf, 1)
    assert_equal(_input(w, app), Int32(6), "input should be 6")
    assert_equal(_clamped(w, app), Int32(6), "clamped should be 6")
    assert_equal(
        _label_text(w, app), String("high"), "label should change to 'high'"
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 4: Increment at max — clamped stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_incr_at_max() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 10 → clamped=10
    _ = _incr_and_flush(w, app, buf, 10)
    assert_equal(_clamped(w, app), Int32(10), "clamped at 10")

    # Increment to 11 → input=11, clamped=10 (stable!)
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(_input(w, app), Int32(11), "input should be 11")
    assert_equal(_clamped(w, app), Int32(10), "clamped should stay 10")
    assert_equal(
        _label_text(w, app), String("high"), "label should stay 'high'"
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 5: Increment above max — chain fully stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_incr_above_max() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 15 → clamped=10
    _ = _incr_and_flush(w, app, buf, 15)
    assert_equal(_clamped(w, app), Int32(10), "clamped at 15 input")

    # Increment to 16 → clamped still 10
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(_input(w, app), Int32(16), "input should be 16")
    assert_equal(_clamped(w, app), Int32(10), "clamped should stay 10")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 6: Decrement within range — clamped changes, label stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_decr_within_range() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 5 → clamped=5, label="low"
    _ = _incr_and_flush(w, app, buf, 5)

    # Decrement to 4 → clamped=4, label="low" (stable)
    _ = _decr_and_flush(w, app, buf, 1)
    assert_equal(_input(w, app), Int32(4), "input should be 4")
    assert_equal(_clamped(w, app), Int32(4), "clamped should be 4")
    assert_equal(_label_text(w, app), String("low"), "label should stay 'low'")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 7: Decrement across threshold — label changes
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_decr_across_threshold() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 6 → clamped=6, label="high"
    _ = _incr_and_flush(w, app, buf, 6)
    assert_equal(
        _label_text(w, app), String("high"), "label at 6 should be 'high'"
    )

    # Decrement to 5 → clamped=5, label="low" (5 > 5 is false → changed!)
    _ = _decr_and_flush(w, app, buf, 1)
    assert_equal(_input(w, app), Int32(5), "input should be 5")
    assert_equal(_clamped(w, app), Int32(5), "clamped should be 5")
    assert_equal(
        _label_text(w, app), String("low"), "label should change to 'low'"
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 8: Decrement at min — clamped stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_decr_at_min() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Input is 0. Decrement to -1 → clamped stays 0
    _decr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(_input(w, app), Int32(-1), "input should be -1")
    assert_equal(_clamped(w, app), Int32(0), "clamped should stay 0")
    assert_equal(_label_text(w, app), String("low"), "label should stay 'low'")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 9: Decrement below min — chain stable
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_decr_below_min() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Decrement to -5
    _ = _decr_and_flush(w, app, buf, 5)
    assert_equal(_input(w, app), Int32(-5), "input should be -5")
    assert_equal(_clamped(w, app), Int32(0), "clamped should stay 0")

    # Decrement to -6 → clamped still 0
    _decr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(_input(w, app), Int32(-6), "input should be -6")
    assert_equal(_clamped(w, app), Int32(0), "clamped should stay 0")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 10: Full cycle round-trip
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_full_cycle() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment from 0 to 12
    for i in range(12):
        _incr(w, app)
        _ = _flush(w, app, buf)

    assert_equal(_input(w, app), Int32(12), "input should be 12")
    assert_equal(_clamped(w, app), Int32(10), "clamped should be 10")
    assert_equal(_label_text(w, app), String("high"), "label should be 'high'")

    # Decrement from 12 back to 0
    for i in range(12):
        _decr(w, app)
        _ = _flush(w, app, buf)

    assert_equal(_input(w, app), Int32(0), "input should be 0")
    assert_equal(_clamped(w, app), Int32(0), "clamped should be 0")
    assert_equal(_label_text(w, app), String("low"), "label should be 'low'")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 11: Label NOT dirty after clamped stabilizes
#
# When clamped is stable, it doesn't write its output signal, so label
# should not be marked dirty on the NEXT flush (after memos are run).
# But note: Phase 36 eager propagation marks everything dirty at
# write_signal time.  The equality cascade means label recomputes to
# the same value (stable), but it IS still marked dirty by Phase 36.
# The key behaviour we test: after run_memos + settle, label should
# report value_changed = False.
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_label_stable_after_clamped_stable() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 10 → clamped=10, label="high"
    _ = _incr_and_flush(w, app, buf, 10)
    assert_equal(_label_text(w, app), String("high"), "label should be 'high'")

    # Increment to 11 → clamped stable (10→10), label stable ("high"→"high")
    _incr(w, app)
    _ = _flush(w, app, buf)

    # After flush, check that the clamped and label memos were value-stable
    assert_false(
        _clamped_changed(w, app),
        "clamped should NOT have changed (10→10)",
    )
    assert_false(
        _label_changed(w, app),
        "label should NOT have changed (high→high)",
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 12: Scope settled when all stable — flush returns 0
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_scope_settled_when_all_stable() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 11 (above max) → clamped stable, label stable
    _ = _incr_and_flush(w, app, buf, 11)

    # Now increment to 12 — clamped stays 10, label stays "high"
    _incr(w, app)
    var bytes = _flush(w, app, buf)

    # flush should return 0 because settle_scopes removed the dirty scope
    assert_equal(
        bytes,
        Int32(0),
        "flush should return 0 when entire chain is value-stable",
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 13: Scope dirty when label changed — flush returns >0
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_scope_dirty_when_label_changed() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 5 → clamped=5, label="low"
    _ = _incr_and_flush(w, app, buf, 5)

    # Increment to 6 → clamped=6 (changed), label="high" (changed)
    _incr(w, app)
    var bytes = _flush(w, app, buf)

    assert_true(bytes > 0, "flush should return >0 when label changed")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 14: Flush returns 0 when no events
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_flush_returns_zero_when_clean() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # No events dispatched — flush should be a no-op
    var bytes = _flush(w, app, buf)
    assert_equal(bytes, Int32(0), "flush should return 0 when clean")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 15: Flush returns nonzero when changed within range
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_flush_returns_nonzero_when_changed() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment within range → values actually change → mutations emitted
    _incr(w, app)
    var bytes = _flush(w, app, buf)
    assert_true(bytes > 0, "flush should return >0 when values changed")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 16: handle_event marks dirty
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_handle_event_marks_dirty() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    assert_false(_has_dirty(w, app), "should not be dirty after rebuild")

    _incr(w, app)
    assert_true(_has_dirty(w, app), "should be dirty after increment")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 17: Memo count is 2
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_memo_count() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    assert_equal(_memo_count(w, app), Int32(2), "should have 2 memos")

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 18: Destroy does not crash
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_destroy_clean() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Just destroy — should not crash
    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 19: Initial compute — clamped_changed and label_changed correct
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_initial_compute_values() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # After rebuild, clamped computed 0 (initial was 0) → stable.
    # Label computed "low" (initial was "low") → stable.
    # Both should report NOT changed because initial == computed.
    assert_false(
        _clamped_changed(w, app),
        "initial clamped compute: 0→0 should be stable",
    )
    assert_false(
        _label_changed(w, app),
        "initial label compute: low→low should be stable",
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Test 20: Consecutive stable flushes all return 0
# ══════════════════════════════════════════════════════════════════════════════


fn test_eq_consecutive_stable_flushes() raises:
    var w = _load()
    var t = _create_eq(w)
    var app = t[0]
    var buf = t[1]

    # Increment to 11 (above max) → clamped=10, label="high"
    _ = _incr_and_flush(w, app, buf, 11)

    # 5 more increments above max — each flush should return 0
    for i in range(5):
        _incr(w, app)
        var bytes = _flush(w, app, buf)
        assert_equal(
            bytes,
            Int32(0),
            "stable flush " + String(i) + " should return 0",
        )

    # Verify state is still correct
    assert_equal(_input(w, app), Int32(16), "input after 16 increments")
    assert_equal(_clamped(w, app), Int32(10), "clamped should stay 10")
    assert_equal(
        _label_text(w, app), String("high"), "label should stay 'high'"
    )

    _destroy_eq(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Main — run all tests
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var passed = 0
    var failed = 0
    var total = 20

    # ── Test 1 ──
    try:
        test_eq_initial_state()
        passed += 1
    except e:
        print("FAIL test_eq_initial_state:", e)
        failed += 1

    # ── Test 2 ──
    try:
        test_eq_incr_within_range()
        passed += 1
    except e:
        print("FAIL test_eq_incr_within_range:", e)
        failed += 1

    # ── Test 3 ──
    try:
        test_eq_incr_across_threshold()
        passed += 1
    except e:
        print("FAIL test_eq_incr_across_threshold:", e)
        failed += 1

    # ── Test 4 ──
    try:
        test_eq_incr_at_max()
        passed += 1
    except e:
        print("FAIL test_eq_incr_at_max:", e)
        failed += 1

    # ── Test 5 ──
    try:
        test_eq_incr_above_max()
        passed += 1
    except e:
        print("FAIL test_eq_incr_above_max:", e)
        failed += 1

    # ── Test 6 ──
    try:
        test_eq_decr_within_range()
        passed += 1
    except e:
        print("FAIL test_eq_decr_within_range:", e)
        failed += 1

    # ── Test 7 ──
    try:
        test_eq_decr_across_threshold()
        passed += 1
    except e:
        print("FAIL test_eq_decr_across_threshold:", e)
        failed += 1

    # ── Test 8 ──
    try:
        test_eq_decr_at_min()
        passed += 1
    except e:
        print("FAIL test_eq_decr_at_min:", e)
        failed += 1

    # ── Test 9 ──
    try:
        test_eq_decr_below_min()
        passed += 1
    except e:
        print("FAIL test_eq_decr_below_min:", e)
        failed += 1

    # ── Test 10 ──
    try:
        test_eq_full_cycle()
        passed += 1
    except e:
        print("FAIL test_eq_full_cycle:", e)
        failed += 1

    # ── Test 11 ──
    try:
        test_eq_label_stable_after_clamped_stable()
        passed += 1
    except e:
        print("FAIL test_eq_label_stable_after_clamped_stable:", e)
        failed += 1

    # ── Test 12 ──
    try:
        test_eq_scope_settled_when_all_stable()
        passed += 1
    except e:
        print("FAIL test_eq_scope_settled_when_all_stable:", e)
        failed += 1

    # ── Test 13 ──
    try:
        test_eq_scope_dirty_when_label_changed()
        passed += 1
    except e:
        print("FAIL test_eq_scope_dirty_when_label_changed:", e)
        failed += 1

    # ── Test 14 ──
    try:
        test_eq_flush_returns_zero_when_clean()
        passed += 1
    except e:
        print("FAIL test_eq_flush_returns_zero_when_clean:", e)
        failed += 1

    # ── Test 15 ──
    try:
        test_eq_flush_returns_nonzero_when_changed()
        passed += 1
    except e:
        print("FAIL test_eq_flush_returns_nonzero_when_changed:", e)
        failed += 1

    # ── Test 16 ──
    try:
        test_eq_handle_event_marks_dirty()
        passed += 1
    except e:
        print("FAIL test_eq_handle_event_marks_dirty:", e)
        failed += 1

    # ── Test 17 ──
    try:
        test_eq_memo_count()
        passed += 1
    except e:
        print("FAIL test_eq_memo_count:", e)
        failed += 1

    # ── Test 18 ──
    try:
        test_eq_destroy_clean()
        passed += 1
    except e:
        print("FAIL test_eq_destroy_clean:", e)
        failed += 1

    # ── Test 19 ──
    try:
        test_eq_initial_compute_values()
        passed += 1
    except e:
        print("FAIL test_eq_initial_compute_values:", e)
        failed += 1

    # ── Test 20 ──
    try:
        test_eq_consecutive_stable_flushes()
        passed += 1
    except e:
        print("FAIL test_eq_consecutive_stable_flushes:", e)
        failed += 1

    # ── Summary ──
    print(
        "  ✓ test_equality_demo — equality_demo:",
        String(passed) + "/" + String(total),
        "passed",
    )
    if failed > 0:
        raise Error(String(failed) + " of " + String(total) + " tests FAILED")
