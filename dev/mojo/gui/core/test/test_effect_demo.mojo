"""Phase 34.1 — EffectDemoApp Mojo Tests.

Validates EffectDemoApp via the ed_* WASM exports which exercise
effects in the flush cycle with derived state (doubled, parity):

  - init creates app with non-zero pointer
  - count starts at 0
  - doubled starts at 0
  - parity starts at "even"
  - effect starts pending (initial run needed)
  - rebuild runs effect (doubled=0, parity="even", effect cleared)
  - increment updates count
  - increment marks effect pending
  - flush after increment updates doubled
  - flush after increment updates parity
  - effect not pending after flush
  - two increments: doubled=4
  - two increments: parity="even"
  - 10 increments: count=10, doubled=20, parity="even"
  - effect resubscribes each run
  - destroy does not crash
  - flush returns 0 when clean
  - rapid 20 increments
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


fn _create_ed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create an EffectDemoApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("ed_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("ed_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_ed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy an EffectDemoApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("ed_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("ed_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("ed_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _incr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises:
    """Increment count via the button handler."""
    var hid = w[].call_i32("ed_incr_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)


fn _parity_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    """Read the parity text from the app."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("ed_parity_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_ed_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("ed_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("ed_destroy", args_ptr(app))


# ── Test: count starts at 0 ─────────────────────────────────────────────────


fn test_ed_count_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Count signal is 0 after init + rebuild."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("ed_count_value", args_ptr(app)), 0)
    _destroy_ed(w, app, buf)


# ── Test: doubled starts at 0 ───────────────────────────────────────────────


fn test_ed_doubled_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Doubled signal is 0 after init + rebuild (effect ran on mount)."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 0)
    _destroy_ed(w, app, buf)


# ── Test: parity starts at "even" ───────────────────────────────────────────


fn test_ed_parity_starts_at_even(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Parity text is 'even' after init + rebuild (effect ran on mount)."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    var parity = _parity_text(w, app)
    assert_equal(parity, String("even"))
    _destroy_ed(w, app, buf)


# ── Test: effect starts pending ──────────────────────────────────────────────


fn test_ed_effect_starts_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Effect is pending after init (before rebuild runs it)."""
    var app = Int(w[].call_i64("ed_init", no_args()))
    # Before rebuild, the effect should be pending (initial run needed)
    assert_equal(w[].call_i32("ed_effect_is_pending", args_ptr(app)), 1)
    w[].call_void("ed_destroy", args_ptr(app))


# ── Test: rebuild runs effect ────────────────────────────────────────────────


fn test_ed_rebuild_runs_effect(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After rebuild, effect has been run (not pending), doubled=0, parity='even'.
    """
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("ed_effect_is_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 0)
    var parity = _parity_text(w, app)
    assert_equal(parity, String("even"))
    _destroy_ed(w, app, buf)


# ── Test: increment updates count ────────────────────────────────────────────


fn test_ed_increment_updates_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment, count = 1."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    assert_equal(w[].call_i32("ed_count_value", args_ptr(app)), 1)
    _destroy_ed(w, app, buf)


# ── Test: increment marks effect pending ─────────────────────────────────────


fn test_ed_increment_marks_effect_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment (before flush), effect is pending."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    assert_equal(w[].call_i32("ed_effect_is_pending", args_ptr(app)), 1)
    _destroy_ed(w, app, buf)


# ── Test: flush after increment updates doubled ─────────────────────────────


fn test_ed_flush_after_increment_doubled(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment + flush, doubled = 2."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 2)
    _destroy_ed(w, app, buf)


# ── Test: flush after increment updates parity ──────────────────────────────


fn test_ed_flush_after_increment_parity(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment + flush, parity = 'odd'."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    var parity = _parity_text(w, app)
    assert_equal(parity, String("odd"))
    _destroy_ed(w, app, buf)


# ── Test: effect not pending after flush ─────────────────────────────────────


fn test_ed_effect_not_pending_after_flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment + flush, effect is no longer pending."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_effect_is_pending", args_ptr(app)), 0)
    _destroy_ed(w, app, buf)


# ── Test: two increments doubled=4 ──────────────────────────────────────────


fn test_ed_two_increments_doubled_4(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 2 increments + flush, doubled = 4."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_count_value", args_ptr(app)), 2)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 4)
    _destroy_ed(w, app, buf)


# ── Test: two increments parity="even" ──────────────────────────────────────


fn test_ed_two_increments_parity_even(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 2 increments + flush, parity = 'even'."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    _incr(w, app)
    _ = _flush(w, app, buf)
    var parity = _parity_text(w, app)
    assert_equal(parity, String("even"))
    _destroy_ed(w, app, buf)


# ── Test: 10 increments ─────────────────────────────────────────────────────


fn test_ed_10_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 10 increments, count=10, doubled=20, parity='even'."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(10):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_count_value", args_ptr(app)), 10)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 20)
    var parity = _parity_text(w, app)
    assert_equal(parity, String("even"))
    _destroy_ed(w, app, buf)


# ── Test: effect resubscribes each run ───────────────────────────────────────


fn test_ed_effect_resubscribes_each_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Effect dependency tracking works across multiple runs.

    After rebuild, increment, flush, increment again — effect still
    fires and updates derived state correctly.
    """
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    # First increment + flush
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 2)
    # Second increment + flush — effect must still track count
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 4)
    # Third increment + flush
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 6)
    _destroy_ed(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_ed_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal use does not crash."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    _destroy_ed(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_ed_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush returns 0 when no state changes have occurred."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, Int32(0))
    _destroy_ed(w, app, buf)


# ── Test: rapid 20 increments ───────────────────────────────────────────────


fn test_ed_rapid_20_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """20 increments with flush after each — all derived state correct."""
    var t = _create_ed(w)
    var app = t[0]
    var buf = t[1]
    for i in range(20):
        _incr(w, app)
        _ = _flush(w, app, buf)
        var count = Int(w[].call_i32("ed_count_value", args_ptr(app)))
        var doubled = Int(w[].call_i32("ed_doubled_value", args_ptr(app)))
        assert_equal(count, i + 1)
        assert_equal(doubled, (i + 1) * 2)
    # Final state
    assert_equal(w[].call_i32("ed_count_value", args_ptr(app)), 20)
    assert_equal(w[].call_i32("ed_doubled_value", args_ptr(app)), 40)
    var parity = _parity_text(w, app)
    assert_equal(parity, String("even"))
    _destroy_ed(w, app, buf)


# ── Test runner ──────────────────────────────────────────────────────────────


fn main() raises:
    var wp = _load()

    print("test_effect_demo — EffectDemoApp effect-in-flush (Phase 34.1):")

    test_ed_init_creates_app(wp)
    print("  ✓ init creates app")

    test_ed_count_starts_at_0(wp)
    print("  ✓ count starts at 0")

    test_ed_doubled_starts_at_0(wp)
    print("  ✓ doubled starts at 0")

    test_ed_parity_starts_at_even(wp)
    print("  ✓ parity starts at even")

    test_ed_effect_starts_pending(wp)
    print("  ✓ effect starts pending")

    test_ed_rebuild_runs_effect(wp)
    print("  ✓ rebuild runs effect")

    test_ed_increment_updates_count(wp)
    print("  ✓ increment updates count")

    test_ed_increment_marks_effect_pending(wp)
    print("  ✓ increment marks effect pending")

    test_ed_flush_after_increment_doubled(wp)
    print("  ✓ flush after increment — doubled")

    test_ed_flush_after_increment_parity(wp)
    print("  ✓ flush after increment — parity")

    test_ed_effect_not_pending_after_flush(wp)
    print("  ✓ effect not pending after flush")

    test_ed_two_increments_doubled_4(wp)
    print("  ✓ two increments — doubled=4")

    test_ed_two_increments_parity_even(wp)
    print("  ✓ two increments — parity=even")

    test_ed_10_increments(wp)
    print("  ✓ 10 increments")

    test_ed_effect_resubscribes_each_run(wp)
    print("  ✓ effect resubscribes each run")

    test_ed_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_ed_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_ed_rapid_20_increments(wp)
    print("  ✓ rapid 20 increments")

    print("  ✓ test_effect_demo — effect_demo: 18/18 passed")
