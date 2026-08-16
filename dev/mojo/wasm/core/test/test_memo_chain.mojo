"""Phase 35.3 — MemoChainApp Mojo Tests.

Validates MemoChainApp via the mc_* WASM exports which exercise
a mixed-type memo chain: SignalI32 → MemoI32 → MemoBool → MemoString.

  - init creates app with non-zero pointer
  - input starts at 0
  - doubled starts at 0
  - is_big starts false
  - label starts "small"
  - all memos start dirty before first flush
  - rebuild settles all memos (all clean)
  - rebuild values correct (doubled=0, is_big=false, label="small")
  - increment to 1 (doubled=2, is_big=false, label="small")
  - increment to 4 (doubled=8, is_big=false, label="small")
  - increment to 5 crosses threshold (doubled=10, is_big=true, label="BIG")
  - increment to 6 stays big (doubled=12, is_big=true, label="BIG")
  - chain propagation order (doubled before is_big before label)
  - 10 increments all correct
  - flush returns 0 when clean
  - memo count is 3
  - scope count is 1
  - destroy does not crash
  - rapid 20 increments
  - threshold boundary exact (input=5 is the boundary)
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


fn _create_mc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a MemoChainApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("mc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("mc_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _create_mc_no_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a MemoChainApp without mounting.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("mc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    return Tuple(app, buf)


fn _destroy_mc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a MemoChainApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("mc_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("mc_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("mc_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _incr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises:
    """Increment input via the button handler."""
    var hid = w[].call_i32("mc_incr_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)


fn _label_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    """Read the label text from the app."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("mc_label_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _is_big(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    """Read the is_big memo value."""
    return w[].call_i32("mc_is_big", args_ptr(app)) != 0


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_mc_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("mc_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("mc_destroy", args_ptr(app))


# ── Test: input starts at 0 ─────────────────────────────────────────────────


fn test_mc_input_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Input signal is 0 after init + rebuild."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 0)
    _destroy_mc(w, app, buf)


# ── Test: doubled starts at 0 ───────────────────────────────────────────────


fn test_mc_doubled_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Doubled memo is 0 after init + rebuild."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 0)
    _destroy_mc(w, app, buf)


# ── Test: is_big starts false ────────────────────────────────────────────────


fn test_mc_is_big_starts_false(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """The is_big memo is False after init + rebuild."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    assert_false(_is_big(w, app), msg="is_big should be False initially")
    _destroy_mc(w, app, buf)


# ── Test: label starts "small" ───────────────────────────────────────────────


fn test_mc_label_starts_small(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Label memo is 'small' after init + rebuild."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_mc(w, app, buf)


# ── Test: all memos start dirty ──────────────────────────────────────────────


fn test_mc_all_memos_start_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """All three memos are dirty before first rebuild."""
    var t = _create_mc_no_rebuild(w)
    var app = t[0]
    var buf = t[1]
    assert_true(
        w[].call_i32("mc_doubled_dirty", args_ptr(app)) != 0,
        msg="doubled should be dirty",
    )
    assert_true(
        w[].call_i32("mc_is_big_dirty", args_ptr(app)) != 0,
        msg="is_big should be dirty",
    )
    assert_true(
        w[].call_i32("mc_label_dirty", args_ptr(app)) != 0,
        msg="label should be dirty",
    )
    _destroy_mc(w, app, buf)


# ── Test: rebuild settles all memos ──────────────────────────────────────────


fn test_mc_rebuild_settles_all(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After rebuild, all three memos are clean."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    assert_false(
        w[].call_i32("mc_doubled_dirty", args_ptr(app)) != 0,
        msg="doubled should be clean after rebuild",
    )
    assert_false(
        w[].call_i32("mc_is_big_dirty", args_ptr(app)) != 0,
        msg="is_big should be clean after rebuild",
    )
    assert_false(
        w[].call_i32("mc_label_dirty", args_ptr(app)) != 0,
        msg="label should be clean after rebuild",
    )
    _destroy_mc(w, app, buf)


# ── Test: rebuild values correct ─────────────────────────────────────────────


fn test_mc_rebuild_values_correct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After rebuild: doubled=0, is_big=false, label='small'."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 0)
    assert_false(_is_big(w, app), msg="is_big should be False")
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_mc(w, app, buf)


# ── Test: increment to 1 ────────────────────────────────────────────────────


fn test_mc_increment_to_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 1 increment: input=1, doubled=2, is_big=false, label='small'."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 1)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 2)
    assert_false(_is_big(w, app), msg="is_big should be False (2 < 10)")
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_mc(w, app, buf)


# ── Test: increment to 4 ────────────────────────────────────────────────────


fn test_mc_increment_to_4(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 4 increments: input=4, doubled=8, is_big=false, label='small'."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(4):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 4)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 8)
    assert_false(_is_big(w, app), msg="is_big should be False (8 < 10)")
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_mc(w, app, buf)


# ── Test: increment to 5 crosses threshold ───────────────────────────────────


fn test_mc_increment_to_5_crosses_threshold(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 5 increments: input=5, doubled=10, is_big=true, label='BIG'."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(5):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 5)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 10)
    assert_true(_is_big(w, app), msg="is_big should be True (10 >= 10)")
    var label = _label_text(w, app)
    assert_equal(label, String("BIG"))
    _destroy_mc(w, app, buf)


# ── Test: increment to 6 stays big ──────────────────────────────────────────


fn test_mc_increment_to_6_stays_big(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 6 increments: input=6, doubled=12, is_big=true, label='BIG'."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(6):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 6)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 12)
    assert_true(_is_big(w, app), msg="is_big should be True (12 >= 10)")
    var label = _label_text(w, app)
    assert_equal(label, String("BIG"))
    _destroy_mc(w, app, buf)


# ── Test: chain propagation order ────────────────────────────────────────────


fn test_mc_chain_propagation_order(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Chain order: doubled before is_big before label.

    Validates by incrementing to 5 (threshold) — if is_big ran before
    doubled, it would see stale doubled=8 and produce False.  If label
    ran before is_big, it would see stale is_big=False and produce "small".
    """
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(5):
        _incr(w, app)
        _ = _flush(w, app, buf)
    # All chain values must be consistent
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 10)
    assert_true(_is_big(w, app), msg="is_big should be True")
    var label = _label_text(w, app)
    assert_equal(label, String("BIG"))
    _destroy_mc(w, app, buf)


# ── Test: 10 increments all correct ─────────────────────────────────────────


fn test_mc_10_increments_all_correct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 10 increments: input=10, doubled=20, is_big=true, label='BIG'."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for i in range(10):
        _incr(w, app)
        _ = _flush(w, app, buf)
        var input_val = Int(w[].call_i32("mc_input_value", args_ptr(app)))
        var doubled_val = Int(w[].call_i32("mc_doubled_value", args_ptr(app)))
        assert_equal(input_val, i + 1)
        assert_equal(doubled_val, (i + 1) * 2)
    # Final state
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 10)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 20)
    assert_true(_is_big(w, app), msg="is_big should be True")
    var label = _label_text(w, app)
    assert_equal(label, String("BIG"))
    _destroy_mc(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_mc_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush returns 0 when no state changes have occurred."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, Int32(0))
    _destroy_mc(w, app, buf)


# ── Test: memo count is 3 ───────────────────────────────────────────────────


fn test_mc_memo_count_is_3(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Three live memos (doubled + is_big + label)."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("mc_memo_count", args_ptr(app))
    assert_equal(count, Int32(3))
    _destroy_mc(w, app, buf)


# ── Test: scope count is 1 ──────────────────────────────────────────────────


fn test_mc_scope_count_is_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Single root scope."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("mc_scope_count", args_ptr(app))
    assert_equal(count, Int32(1))
    _destroy_mc(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_mc_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal use does not crash."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(5):
        _incr(w, app)
        _ = _flush(w, app, buf)
    _destroy_mc(w, app, buf)


# ── Test: rapid 20 increments ───────────────────────────────────────────────


fn test_mc_rapid_20_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """20 increments with flush after each — all derived state correct."""
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    for i in range(20):
        _incr(w, app)
        _ = _flush(w, app, buf)
        var input_val = Int(w[].call_i32("mc_input_value", args_ptr(app)))
        var doubled_val = Int(w[].call_i32("mc_doubled_value", args_ptr(app)))
        assert_equal(input_val, i + 1)
        assert_equal(doubled_val, (i + 1) * 2)
        var expected_big = (i + 1) * 2 >= 10
        var actual_big = _is_big(w, app)
        assert_equal(actual_big, expected_big)
    # Final state
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 20)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 40)
    assert_true(_is_big(w, app), msg="is_big should be True")
    var label = _label_text(w, app)
    assert_equal(label, String("BIG"))
    _destroy_mc(w, app, buf)


# ── Test: threshold boundary exact ───────────────────────────────────────────


fn test_mc_threshold_boundary_exact(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Threshold: input=4→doubled=8→small, input=5→doubled=10→BIG.

    Validates that input=5 (doubled=10) is the exact boundary where
    is_big flips from False to True.
    """
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    # Get to input=4
    for _ in range(4):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 8)
    assert_false(_is_big(w, app), msg="is_big False at doubled=8")
    var label4 = _label_text(w, app)
    assert_equal(label4, String("small"))
    # One more → input=5
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 10)
    assert_true(_is_big(w, app), msg="is_big True at doubled=10")
    var label5 = _label_text(w, app)
    assert_equal(label5, String("BIG"))
    _destroy_mc(w, app, buf)


# ── Test: all memos dirty after increment (Phase 36) ────────────────────────


fn test_mc_all_memos_dirty_after_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment, all three memos are independently dirty.

    Phase 36 recursive propagation marks downstream memos dirty
    through memo → memo chains, so doubled, is_big, AND label
    should all be dirty after a single signal write.
    """
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    # After rebuild, all memos are clean
    assert_equal(
        w[].call_i32("mc_doubled_dirty", args_ptr(app)),
        0,
        msg="doubled clean after rebuild",
    )
    assert_equal(
        w[].call_i32("mc_is_big_dirty", args_ptr(app)),
        0,
        msg="is_big clean after rebuild",
    )
    assert_equal(
        w[].call_i32("mc_label_dirty", args_ptr(app)),
        0,
        msg="label clean after rebuild",
    )
    # Increment — signal write should dirty ALL memos via propagation
    _incr(w, app)
    assert_equal(
        w[].call_i32("mc_doubled_dirty", args_ptr(app)),
        1,
        msg="doubled dirty after increment",
    )
    assert_equal(
        w[].call_i32("mc_is_big_dirty", args_ptr(app)),
        1,
        msg="is_big dirty after increment (propagated)",
    )
    assert_equal(
        w[].call_i32("mc_label_dirty", args_ptr(app)),
        1,
        msg="label dirty after increment (propagated)",
    )
    _destroy_mc(w, app, buf)


# ── Test: partial recompute (Phase 36) ──────────────────────────────────────


fn test_mc_partial_recompute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment, flush recomputes all independently-dirty memos.

    Verifies that flush settles the full chain: doubled clean,
    is_big clean, label clean — each was dirty independently and
    each was recomputed independently by run_memos().
    """
    var t = _create_mc(w)
    var app = t[0]
    var buf = t[1]
    # Increment to trigger dirtiness
    _incr(w, app)
    # All three dirty before flush
    assert_equal(
        w[].call_i32("mc_doubled_dirty", args_ptr(app)),
        1,
        msg="doubled dirty before flush",
    )
    assert_equal(
        w[].call_i32("mc_is_big_dirty", args_ptr(app)),
        1,
        msg="is_big dirty before flush",
    )
    assert_equal(
        w[].call_i32("mc_label_dirty", args_ptr(app)),
        1,
        msg="label dirty before flush",
    )
    # Flush — run_memos recomputes each independently
    _ = _flush(w, app, buf)
    # All three clean after flush
    assert_equal(
        w[].call_i32("mc_doubled_dirty", args_ptr(app)),
        0,
        msg="doubled clean after flush",
    )
    assert_equal(
        w[].call_i32("mc_is_big_dirty", args_ptr(app)),
        0,
        msg="is_big clean after flush",
    )
    assert_equal(
        w[].call_i32("mc_label_dirty", args_ptr(app)),
        0,
        msg="label clean after flush",
    )
    # Values are correct
    assert_equal(w[].call_i32("mc_input_value", args_ptr(app)), 1)
    assert_equal(w[].call_i32("mc_doubled_value", args_ptr(app)), 2)
    assert_false(_is_big(w, app), msg="is_big False at doubled=2")
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_mc(w, app, buf)


# ── Test runner ──────────────────────────────────────────────────────────────


fn main() raises:
    var wp = _load()

    print("test_memo_chain — MemoChainApp mixed-type memo chain (Phase 35.3):")

    test_mc_init_creates_app(wp)
    print("  ✓ init creates app")

    test_mc_input_starts_at_0(wp)
    print("  ✓ input starts at 0")

    test_mc_doubled_starts_at_0(wp)
    print("  ✓ doubled starts at 0")

    test_mc_is_big_starts_false(wp)
    print("  ✓ is_big starts false")

    test_mc_label_starts_small(wp)
    print("  ✓ label starts 'small'")

    test_mc_all_memos_start_dirty(wp)
    print("  ✓ all memos start dirty")

    test_mc_rebuild_settles_all(wp)
    print("  ✓ rebuild settles all memos")

    test_mc_rebuild_values_correct(wp)
    print("  ✓ rebuild values correct")

    test_mc_increment_to_1(wp)
    print("  ✓ increment to 1")

    test_mc_increment_to_4(wp)
    print("  ✓ increment to 4")

    test_mc_increment_to_5_crosses_threshold(wp)
    print("  ✓ increment to 5 crosses threshold")

    test_mc_increment_to_6_stays_big(wp)
    print("  ✓ increment to 6 stays big")

    test_mc_chain_propagation_order(wp)
    print("  ✓ chain propagation order")

    test_mc_10_increments_all_correct(wp)
    print("  ✓ 10 increments all correct")

    test_mc_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_mc_memo_count_is_3(wp)
    print("  ✓ memo count is 3")

    test_mc_scope_count_is_1(wp)
    print("  ✓ scope count is 1")

    test_mc_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_mc_rapid_20_increments(wp)
    print("  ✓ rapid 20 increments")

    test_mc_threshold_boundary_exact(wp)
    print("  ✓ threshold boundary exact")

    test_mc_all_memos_dirty_after_increment(wp)
    print("  ✓ all memos dirty after increment (Phase 36)")

    test_mc_partial_recompute(wp)
    print("  ✓ partial recompute (Phase 36)")

    print("  ✓ test_memo_chain — memo_chain: 22/22 passed")
