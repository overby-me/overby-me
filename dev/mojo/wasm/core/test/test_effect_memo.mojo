"""Phase 34.2 — EffectMemoApp Mojo Tests.

Validates EffectMemoApp via the em_* WASM exports which exercise
the signal → memo → effect → signal reactive chain:

  - init creates app with non-zero pointer
  - input starts at 0
  - tripled starts at 0 (memo initial value)
  - label starts at "small" (0 < 10)
  - increment updates input
  - flush updates tripled (tripled = 3)
  - flush updates label ("small" since 3 < 10)
  - 3 increments: tripled=9, label="small"
  - 4 increments: tripled=12, label="big"
  - threshold boundary (3→"small", 4→"big")
  - memo and effect both run on flush
  - effect reads memo not input
  - 10 increments: input=10, tripled=30, label="big"
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


fn _create_em(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create an EffectMemoApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("em_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("em_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_em(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy an EffectMemoApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("em_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("em_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("em_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _incr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises:
    """Increment input via the button handler."""
    var hid = w[].call_i32("em_incr_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)


fn _label_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    """Read the label text from the app."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("em_label_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_em_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("em_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("em_destroy", args_ptr(app))


# ── Test: input starts at 0 ─────────────────────────────────────────────────


fn test_em_input_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Input signal is 0 after init + rebuild."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("em_input_value", args_ptr(app)), 0)
    _destroy_em(w, app, buf)


# ── Test: tripled starts at 0 ───────────────────────────────────────────────


fn test_em_tripled_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Tripled memo is 0 after init + rebuild (memo recomputed on mount)."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 0)
    _destroy_em(w, app, buf)


# ── Test: label starts at "small" ────────────────────────────────────────────


fn test_em_label_starts_at_small(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Label text is 'small' after init + rebuild (0 < 10)."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_em(w, app, buf)


# ── Test: increment updates input ────────────────────────────────────────────


fn test_em_increment_updates_input(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment, input = 1."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    assert_equal(w[].call_i32("em_input_value", args_ptr(app)), 1)
    _destroy_em(w, app, buf)


# ── Test: flush updates tripled ──────────────────────────────────────────────


fn test_em_flush_updates_tripled(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment + flush, tripled = 3."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 3)
    _destroy_em(w, app, buf)


# ── Test: flush updates label ────────────────────────────────────────────────


fn test_em_flush_updates_label(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After increment + flush, label = 'small' (3 < 10)."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_em(w, app, buf)


# ── Test: 3 increments tripled=9, label="small" ─────────────────────────────


fn test_em_3_increments_tripled_9(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 3 increments, input=3, tripled=9, label='small'."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(3):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("em_input_value", args_ptr(app)), 3)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 9)
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    _destroy_em(w, app, buf)


# ── Test: 4 increments tripled=12, label="big" ──────────────────────────────


fn test_em_4_increments_tripled_12(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 4 increments, input=4, tripled=12, label='big'."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(4):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("em_input_value", args_ptr(app)), 4)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 12)
    var label = _label_text(w, app)
    assert_equal(label, String("big"))
    _destroy_em(w, app, buf)


# ── Test: threshold boundary ─────────────────────────────────────────────────


fn test_em_threshold_boundary(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Threshold: input=3→tripled=9→'small', input=4→tripled=12→'big'."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    # Get to input=3
    for _ in range(3):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 9)
    var label3 = _label_text(w, app)
    assert_equal(label3, String("small"))
    # One more → input=4
    _incr(w, app)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 12)
    var label4 = _label_text(w, app)
    assert_equal(label4, String("big"))
    _destroy_em(w, app, buf)


# ── Test: memo and effect both run ───────────────────────────────────────────


fn test_em_memo_and_effect_both_run(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After flush, both memo and effect have executed (tripled updated, label updated).
    """
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    # Memo ran: tripled = 3
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 3)
    # Effect ran: label = "small" (3 < 10)
    var label = _label_text(w, app)
    assert_equal(label, String("small"))
    # Effect should not be pending
    assert_equal(w[].call_i32("em_effect_is_pending", args_ptr(app)), 0)
    _destroy_em(w, app, buf)


# ── Test: effect reads memo not input ────────────────────────────────────────


fn test_em_effect_reads_memo_not_input(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Effect depends on tripled (memo output), not input directly.

    Validates: the effect threshold is on tripled value (10), not input.
    input=3 → tripled=9 → "small"
    input=4 → tripled=12 → "big"
    If effect read input instead of tripled, threshold would be at input=10.
    """
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    # input=4 → tripled=12 → "big" (effect reads tripled, not input)
    for _ in range(4):
        _incr(w, app)
        _ = _flush(w, app, buf)
    var label = _label_text(w, app)
    assert_equal(label, String("big"))
    # If the effect read input (4), it would still be "small" since 4 < 10
    # But since it reads tripled (12), it's "big"
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 12)
    _destroy_em(w, app, buf)


# ── Test: 10 increments ─────────────────────────────────────────────────────


fn test_em_10_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After 10 increments, input=10, tripled=30, label='big'."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    for _ in range(10):
        _incr(w, app)
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("em_input_value", args_ptr(app)), 10)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 30)
    var label = _label_text(w, app)
    assert_equal(label, String("big"))
    _destroy_em(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_em_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal use does not crash."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    _incr(w, app)
    _ = _flush(w, app, buf)
    _destroy_em(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_em_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush returns 0 when no state changes have occurred."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, Int32(0))
    _destroy_em(w, app, buf)


# ── Test: rapid 20 increments ───────────────────────────────────────────────


fn test_em_rapid_20_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """20 increments with flush after each — all derived state correct."""
    var t = _create_em(w)
    var app = t[0]
    var buf = t[1]
    for i in range(20):
        _incr(w, app)
        _ = _flush(w, app, buf)
        var input_val = Int(w[].call_i32("em_input_value", args_ptr(app)))
        var tripled_val = Int(w[].call_i32("em_tripled_value", args_ptr(app)))
        assert_equal(input_val, i + 1)
        assert_equal(tripled_val, (i + 1) * 3)
    # Final state
    assert_equal(w[].call_i32("em_input_value", args_ptr(app)), 20)
    assert_equal(w[].call_i32("em_tripled_value", args_ptr(app)), 60)
    var label = _label_text(w, app)
    assert_equal(label, String("big"))
    _destroy_em(w, app, buf)


# ── Test runner ──────────────────────────────────────────────────────────────


fn main() raises:
    var wp = _load()

    print("test_effect_memo — EffectMemoApp effect+memo chain (Phase 34.2):")

    test_em_init_creates_app(wp)
    print("  ✓ init creates app")

    test_em_input_starts_at_0(wp)
    print("  ✓ input starts at 0")

    test_em_tripled_starts_at_0(wp)
    print("  ✓ tripled starts at 0")

    test_em_label_starts_at_small(wp)
    print("  ✓ label starts at small")

    test_em_increment_updates_input(wp)
    print("  ✓ increment updates input")

    test_em_flush_updates_tripled(wp)
    print("  ✓ flush updates tripled")

    test_em_flush_updates_label(wp)
    print("  ✓ flush updates label")

    test_em_3_increments_tripled_9(wp)
    print("  ✓ 3 increments — tripled=9, label=small")

    test_em_4_increments_tripled_12(wp)
    print("  ✓ 4 increments — tripled=12, label=big")

    test_em_threshold_boundary(wp)
    print("  ✓ threshold boundary")

    test_em_memo_and_effect_both_run(wp)
    print("  ✓ memo and effect both run")

    test_em_effect_reads_memo_not_input(wp)
    print("  ✓ effect reads memo not input")

    test_em_10_increments(wp)
    print("  ✓ 10 increments")

    test_em_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_em_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_em_rapid_20_increments(wp)
    print("  ✓ rapid 20 increments")

    print("  ✓ test_effect_memo — effect_memo: 16/16 passed")
