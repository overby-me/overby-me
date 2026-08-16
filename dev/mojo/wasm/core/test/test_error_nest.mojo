"""Phase 32.3 — ErrorNestApp Mojo Tests.

Validates ErrorNestApp via the en_* WASM exports which exercise
nested error boundaries with independent crash/retry lifecycles:

  - init creates app with non-zero pointer
  - no errors initially (both boundaries clean)
  - all normal children mounted after rebuild
  - no fallbacks initially
  - inner crash sets inner error
  - inner crash preserves outer (outer still clean)
  - flush after inner crash: inner fallback shown, inner normal hidden,
    outer normal still mounted
  - inner retry clears inner error
  - flush after inner retry: inner normal restored
  - outer crash sets outer error
  - flush after outer crash: outer fallback shown, outer normal hidden
    (inner boundary + children also hidden)
  - outer retry restores outer normal + inner boundary visible again
  - inner crash then outer crash: both errors set, outer fallback
    takes precedence visually
  - outer retry reveals inner error (inner fallback shown)
  - inner retry after outer retry: full recovery
  - multiple inner crash/retry cycles
  - multiple outer crash/retry cycles
  - mixed crash/retry sequence
  - destroy does not crash
  - destroy with active error
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
    args_ptr_ptr_i32,
)


fn _load() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_en(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create an ErrorNestApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("en_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("en_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_en(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy an ErrorNestApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("en_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("en_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("en_handle_event", args_ptr_i32_i32(app, handler_id, 0))


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_en_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("en_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("en_destroy", args_ptr(app))


# ── Test: no errors initially ────────────────────────────────────────────────


fn test_en_no_errors_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Both boundaries have no error after init."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: all normal mounted after rebuild ───────────────────────────────────


fn test_en_all_normal_mounted_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Outer normal + inner normal are mounted after rebuild."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 1)
    _destroy_en(w, app, buf)


# ── Test: no fallbacks initially ─────────────────────────────────────────────


fn test_en_no_fallbacks_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Both fallbacks are hidden after initial rebuild."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: inner crash sets inner error ───────────────────────────────────────


fn test_en_inner_crash_sets_inner_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner crash handler sets inner has_error to true."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
    _destroy_en(w, app, buf)


# ── Test: inner crash preserves outer ────────────────────────────────────────


fn test_en_inner_crash_preserves_outer(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner crash does not set outer error."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: flush after inner crash ────────────────────────────────────────────


fn test_en_flush_after_inner_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After inner crash + flush: inner fallback shown, inner normal hidden,
    outer normal still mounted."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: inner retry clears inner error ─────────────────────────────────────


fn test_en_inner_retry_clears_inner_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner retry handler clears the inner error state."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    # Crash
    var crash_hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Retry
    var retry_hid = w[].call_i32("en_inner_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: flush after inner retry ────────────────────────────────────────────


fn test_en_flush_after_inner_retry(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After inner crash + flush + inner retry + flush: inner normal restored.
    """
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    # Crash + flush
    var crash_hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Retry + flush
    var retry_hid = w[].call_i32("en_inner_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: outer crash sets outer error ───────────────────────────────────────


fn test_en_outer_crash_sets_outer_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Outer crash handler sets outer has_error to true."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 1)
    _destroy_en(w, app, buf)


# ── Test: flush after outer crash ────────────────────────────────────────────


fn test_en_flush_after_outer_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After outer crash + flush: outer fallback shown, outer normal hidden
    (inner boundary + children also hidden)."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: outer retry restores outer normal ──────────────────────────────────


fn test_en_outer_retry_restores_outer_normal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After outer crash + flush + outer retry + flush: outer normal +
    inner boundary visible again."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    # Crash + flush
    var crash_hid = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Retry + flush
    var retry_hid = w[].call_i32("en_outer_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 1)
    _destroy_en(w, app, buf)


# ── Test: inner crash then outer crash ───────────────────────────────────────


fn test_en_inner_crash_then_outer_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Both errors set; outer fallback takes precedence visually."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var inner_hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var outer_hid = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    # Inner crash + flush
    _ = _handle_event(w, app, inner_hid)
    _ = _flush(w, app, buf)
    # Outer crash + flush
    _ = _handle_event(w, app, outer_hid)
    _ = _flush(w, app, buf)
    # Both errors set
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
    # Outer fallback shown, everything else hidden
    assert_equal(w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: outer retry reveals inner error ────────────────────────────────────


fn test_en_outer_retry_reveals_inner_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After outer retry, inner still in error (inner fallback shown)."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var inner_crash = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var outer_crash = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    var outer_retry = w[].call_i32("en_outer_retry_handler", args_ptr(app))
    # Inner crash + flush
    _ = _handle_event(w, app, inner_crash)
    _ = _flush(w, app, buf)
    # Outer crash + flush
    _ = _handle_event(w, app, outer_crash)
    _ = _flush(w, app, buf)
    # Outer retry + flush
    _ = _handle_event(w, app, outer_retry)
    _ = _flush(w, app, buf)
    # Outer restored, inner still in error
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: inner retry after outer retry — full recovery ──────────────────────


fn test_en_inner_retry_after_outer_retry(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Full recovery: inner crash → outer crash → outer retry → inner retry."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var inner_crash = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var outer_crash = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    var outer_retry = w[].call_i32("en_outer_retry_handler", args_ptr(app))
    var inner_retry = w[].call_i32("en_inner_retry_handler", args_ptr(app))
    # Inner crash + flush
    _ = _handle_event(w, app, inner_crash)
    _ = _flush(w, app, buf)
    # Outer crash + flush
    _ = _handle_event(w, app, outer_crash)
    _ = _flush(w, app, buf)
    # Outer retry + flush
    _ = _handle_event(w, app, outer_retry)
    _ = _flush(w, app, buf)
    # Inner retry + flush
    _ = _handle_event(w, app, inner_retry)
    _ = _flush(w, app, buf)
    # Fully recovered
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 0)
    _destroy_en(w, app, buf)


# ── Test: multiple inner crash/retry cycles ──────────────────────────────────


fn test_en_multiple_inner_crash_retry_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five inner crash/retry cycles all succeed."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var crash_hid = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var retry_hid = w[].call_i32("en_inner_retry_handler", args_ptr(app))
    for _ in range(5):
        # Crash
        _ = _handle_event(w, app, crash_hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
        assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 0)
        assert_equal(
            w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 1
        )
        # Retry
        _ = _handle_event(w, app, retry_hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 0)
        assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 1)
        assert_equal(
            w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 0
        )
    _destroy_en(w, app, buf)


# ── Test: multiple outer crash/retry cycles ──────────────────────────────────


fn test_en_multiple_outer_crash_retry_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five outer crash/retry cycles all succeed."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var crash_hid = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    var retry_hid = w[].call_i32("en_outer_retry_handler", args_ptr(app))
    for _ in range(5):
        # Crash
        _ = _handle_event(w, app, crash_hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 1)
        assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 0)
        assert_equal(
            w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 1
        )
        # Retry
        _ = _handle_event(w, app, retry_hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
        assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
        assert_equal(
            w[].call_i32("en_outer_fallback_mounted", args_ptr(app)), 0
        )
    _destroy_en(w, app, buf)


# ── Test: mixed crash/retry sequence ─────────────────────────────────────────


fn test_en_mixed_crash_retry_sequence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Mixed: inner→outer→outer_retry→inner_retry — full recovery."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var inner_crash = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var outer_crash = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    var inner_retry = w[].call_i32("en_inner_retry_handler", args_ptr(app))
    var outer_retry = w[].call_i32("en_outer_retry_handler", args_ptr(app))
    # Inner crash
    _ = _handle_event(w, app, inner_crash)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    # Outer crash
    _ = _handle_event(w, app, outer_crash)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
    # Outer retry
    _ = _handle_event(w, app, outer_retry)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_fallback_mounted", args_ptr(app)), 1)
    # Inner retry
    _ = _handle_event(w, app, inner_retry)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("en_has_outer_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_has_inner_error", args_ptr(app)), 0)
    assert_equal(w[].call_i32("en_outer_normal_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("en_inner_normal_mounted", args_ptr(app)), 1)
    _destroy_en(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_en_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal lifecycle does not crash."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    _destroy_en(w, app, buf)


# ── Test: destroy with active error ──────────────────────────────────────────


fn test_en_destroy_with_active_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy while both errors are active does not crash."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var inner_crash = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var outer_crash = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, inner_crash)
    _ = _flush(w, app, buf)
    _ = _handle_event(w, app, outer_crash)
    _ = _flush(w, app, buf)
    _destroy_en(w, app, buf)


# ── Test: scope count ────────────────────────────────────────────────────────


fn test_en_scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five scopes: root + outer_normal + inner_normal + inner_fallback +
    outer_fallback."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("en_scope_count", args_ptr(app))
    assert_equal(count, 5)
    _destroy_en(w, app, buf)


# ── Test: scope IDs are distinct ─────────────────────────────────────────────


fn test_en_scope_ids_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """All five scope IDs are distinct."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var outer = w[].call_i32("en_outer_scope_id", args_ptr(app))
    var inner_b = w[].call_i32("en_inner_boundary_scope_id", args_ptr(app))
    var inner_n = w[].call_i32("en_inner_normal_scope_id", args_ptr(app))
    var inner_f = w[].call_i32("en_inner_fallback_scope_id", args_ptr(app))
    var outer_f = w[].call_i32("en_outer_fallback_scope_id", args_ptr(app))
    assert_true(outer != inner_b, msg="outer != inner_boundary")
    assert_true(outer != inner_n, msg="outer != inner_normal")
    assert_true(outer != inner_f, msg="outer != inner_fallback")
    assert_true(outer != outer_f, msg="outer != outer_fallback")
    assert_true(inner_b != inner_n, msg="inner_boundary != inner_normal")
    assert_true(inner_b != inner_f, msg="inner_boundary != inner_fallback")
    assert_true(inner_b != outer_f, msg="inner_boundary != outer_fallback")
    assert_true(inner_n != inner_f, msg="inner_normal != inner_fallback")
    assert_true(inner_n != outer_f, msg="inner_normal != outer_fallback")
    assert_true(inner_f != outer_f, msg="inner_fallback != outer_fallback")
    _destroy_en(w, app, buf)


# ── Test: handler IDs are valid and distinct ─────────────────────────────────


fn test_en_handler_ids_valid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Four handler IDs are all distinct and non-negative."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var oc = w[].call_i32("en_outer_crash_handler", args_ptr(app))
    var ic = w[].call_i32("en_inner_crash_handler", args_ptr(app))
    var or_ = w[].call_i32("en_outer_retry_handler", args_ptr(app))
    var ir = w[].call_i32("en_inner_retry_handler", args_ptr(app))
    assert_true(oc >= 0, msg="outer crash handler >= 0")
    assert_true(ic >= 0, msg="inner crash handler >= 0")
    assert_true(or_ >= 0, msg="outer retry handler >= 0")
    assert_true(ir >= 0, msg="inner retry handler >= 0")
    assert_true(oc != ic, msg="outer_crash != inner_crash")
    assert_true(oc != or_, msg="outer_crash != outer_retry")
    assert_true(oc != ir, msg="outer_crash != inner_retry")
    assert_true(ic != or_, msg="inner_crash != outer_retry")
    assert_true(ic != ir, msg="inner_crash != inner_retry")
    assert_true(or_ != ir, msg="outer_retry != inner_retry")
    _destroy_en(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_en_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush without any state change returns 0."""
    var t = _create_en(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, 0)
    _destroy_en(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point — runs all tests via a shared WASM instance
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = _load()

    print(
        "test_error_nest — ErrorNestApp nested error boundaries (Phase 32.3):"
    )

    test_en_init_creates_app(wp)
    print("  ✓ init creates app")

    test_en_no_errors_initially(wp)
    print("  ✓ no errors initially")

    test_en_all_normal_mounted_after_rebuild(wp)
    print("  ✓ all normal mounted after rebuild")

    test_en_no_fallbacks_initially(wp)
    print("  ✓ no fallbacks initially")

    test_en_inner_crash_sets_inner_error(wp)
    print("  ✓ inner crash sets inner error")

    test_en_inner_crash_preserves_outer(wp)
    print("  ✓ inner crash preserves outer")

    test_en_flush_after_inner_crash(wp)
    print("  ✓ flush after inner crash")

    test_en_inner_retry_clears_inner_error(wp)
    print("  ✓ inner retry clears inner error")

    test_en_flush_after_inner_retry(wp)
    print("  ✓ flush after inner retry")

    test_en_outer_crash_sets_outer_error(wp)
    print("  ✓ outer crash sets outer error")

    test_en_flush_after_outer_crash(wp)
    print("  ✓ flush after outer crash")

    test_en_outer_retry_restores_outer_normal(wp)
    print("  ✓ outer retry restores outer normal")

    test_en_inner_crash_then_outer_crash(wp)
    print("  ✓ inner crash then outer crash")

    test_en_outer_retry_reveals_inner_error(wp)
    print("  ✓ outer retry reveals inner error")

    test_en_inner_retry_after_outer_retry(wp)
    print("  ✓ inner retry after outer retry — full recovery")

    test_en_multiple_inner_crash_retry_cycles(wp)
    print("  ✓ multiple inner crash/retry cycles")

    test_en_multiple_outer_crash_retry_cycles(wp)
    print("  ✓ multiple outer crash/retry cycles")

    test_en_mixed_crash_retry_sequence(wp)
    print("  ✓ mixed crash/retry sequence")

    test_en_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_en_destroy_with_active_error(wp)
    print("  ✓ destroy with active error")

    test_en_scope_count(wp)
    print("  ✓ scope count")

    test_en_scope_ids_distinct(wp)
    print("  ✓ scope IDs distinct")

    test_en_handler_ids_valid(wp)
    print("  ✓ handler IDs valid")

    test_en_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    print("  ✓ test_error_nest — error_nest: 24/24 passed")
