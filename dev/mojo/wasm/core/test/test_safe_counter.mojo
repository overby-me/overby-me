"""Phase 32.2 — SafeCounterApp Mojo Tests.

Validates SafeCounterApp via the sc_* WASM exports which exercise
the error boundary pattern with crash/retry lifecycle:

  - init creates app with non-zero pointer
  - count starts at 0
  - has_error initially false
  - normal child mounted after rebuild
  - fallback child not mounted initially
  - increment updates count
  - crash sets error state
  - flush after crash hides normal child
  - flush after crash shows fallback child
  - retry clears error state
  - flush after retry shows normal child
  - flush after retry hides fallback child
  - count preserved after crash/recovery
  - multiple crash/retry cycles
  - destroy does not crash
  - rapid increments after recovery
  - scope count = 3
  - scope IDs distinct
  - handler IDs valid and distinct
  - flush returns 0 when clean
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


fn _create_sc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a SafeCounterApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("sc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("sc_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_sc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a SafeCounterApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("sc_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("sc_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("sc_handle_event", args_ptr_i32_i32(app, handler_id, 0))


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_sc_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("sc_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("sc_destroy", args_ptr(app))


# ── Test: count starts at 0 ─────────────────────────────────────────────────


fn test_sc_count_starts_at_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Initial count value is 0."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("sc_count_value", args_ptr(app))
    assert_equal(count, 0)
    _destroy_sc(w, app, buf)


# ── Test: has_error initially false ──────────────────────────────────────────


fn test_sc_has_error_initially_false(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Boundary has no error after init."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var has_err = w[].call_i32("sc_has_error", args_ptr(app))
    assert_equal(has_err, 0)
    _destroy_sc(w, app, buf)


# ── Test: normal mounted after rebuild ───────────────────────────────────────


fn test_sc_normal_mounted_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Normal child is mounted after rebuild."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var mounted = w[].call_i32("sc_normal_mounted", args_ptr(app))
    assert_equal(mounted, 1)
    _destroy_sc(w, app, buf)


# ── Test: fallback not mounted initially ─────────────────────────────────────


fn test_sc_fallback_not_mounted_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Fallback child is NOT mounted after initial rebuild."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var mounted = w[].call_i32("sc_fallback_mounted", args_ptr(app))
    assert_equal(mounted, 0)
    _destroy_sc(w, app, buf)


# ── Test: increment updates count ────────────────────────────────────────────


fn test_sc_increment_updates_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Incrementing via the incr handler updates the count signal."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("sc_incr_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    var count = w[].call_i32("sc_count_value", args_ptr(app))
    assert_equal(count, 1)
    _destroy_sc(w, app, buf)


# ── Test: crash sets error ───────────────────────────────────────────────────


fn test_sc_crash_sets_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Dispatching the crash handler sets has_error to true."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    var has_err = w[].call_i32("sc_has_error", args_ptr(app))
    assert_equal(has_err, 1)
    _destroy_sc(w, app, buf)


# ── Test: flush after crash hides normal ─────────────────────────────────────


fn test_sc_flush_after_crash_hides_normal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After crash + flush, normal child is no longer mounted."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    var mounted = w[].call_i32("sc_normal_mounted", args_ptr(app))
    assert_equal(mounted, 0)
    _destroy_sc(w, app, buf)


# ── Test: flush after crash shows fallback ───────────────────────────────────


fn test_sc_flush_after_crash_shows_fallback(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After crash + flush, fallback child is mounted."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    var mounted = w[].call_i32("sc_fallback_mounted", args_ptr(app))
    assert_equal(mounted, 1)
    _destroy_sc(w, app, buf)


# ── Test: retry clears error ─────────────────────────────────────────────────


fn test_sc_retry_clears_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Retry handler clears the error state."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    # Crash
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Retry
    var retry_hid = w[].call_i32("sc_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    var has_err = w[].call_i32("sc_has_error", args_ptr(app))
    assert_equal(has_err, 0)
    _destroy_sc(w, app, buf)


# ── Test: flush after retry shows normal ─────────────────────────────────────


fn test_sc_flush_after_retry_shows_normal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After crash + flush + retry + flush, normal child is re-mounted."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    # Crash + flush
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Retry + flush
    var retry_hid = w[].call_i32("sc_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    _ = _flush(w, app, buf)
    var mounted = w[].call_i32("sc_normal_mounted", args_ptr(app))
    assert_equal(mounted, 1)
    _destroy_sc(w, app, buf)


# ── Test: flush after retry hides fallback ───────────────────────────────────


fn test_sc_flush_after_retry_hides_fallback(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After crash + flush + retry + flush, fallback child is hidden."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    # Crash + flush
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Retry + flush
    var retry_hid = w[].call_i32("sc_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    _ = _flush(w, app, buf)
    var mounted = w[].call_i32("sc_fallback_mounted", args_ptr(app))
    assert_equal(mounted, 0)
    _destroy_sc(w, app, buf)


# ── Test: count preserved after crash/recovery ───────────────────────────────


fn test_sc_count_preserved_after_crash_recovery(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Count signal persists across crash/recovery cycles."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var incr_hid = w[].call_i32("sc_incr_handler", args_ptr(app))
    # Increment 3 times
    for _ in range(3):
        _ = _handle_event(w, app, incr_hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sc_count_value", args_ptr(app)), 3)
    # Crash + flush
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Count should still be 3
    assert_equal(w[].call_i32("sc_count_value", args_ptr(app)), 3)
    # Retry + flush
    var retry_hid = w[].call_i32("sc_retry_handler", args_ptr(app))
    _ = _handle_event(w, app, retry_hid)
    _ = _flush(w, app, buf)
    # Count should still be 3
    assert_equal(w[].call_i32("sc_count_value", args_ptr(app)), 3)
    _destroy_sc(w, app, buf)


# ── Test: multiple crash/retry cycles ────────────────────────────────────────


fn test_sc_multiple_crash_retry_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five crash/retry cycles all succeed without errors."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    var retry_hid = w[].call_i32("sc_retry_handler", args_ptr(app))
    for _ in range(5):
        # Crash
        _ = _handle_event(w, app, crash_hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("sc_has_error", args_ptr(app)), 1)
        assert_equal(w[].call_i32("sc_normal_mounted", args_ptr(app)), 0)
        assert_equal(w[].call_i32("sc_fallback_mounted", args_ptr(app)), 1)
        # Retry
        _ = _handle_event(w, app, retry_hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("sc_has_error", args_ptr(app)), 0)
        assert_equal(w[].call_i32("sc_normal_mounted", args_ptr(app)), 1)
        assert_equal(w[].call_i32("sc_fallback_mounted", args_ptr(app)), 0)
    _destroy_sc(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_sc_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal lifecycle does not crash."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    _destroy_sc(w, app, buf)


# ── Test: rapid increments after recovery ────────────────────────────────────


fn test_sc_rapid_increments_after_recovery(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Twenty increments after crash/recovery produce correct count."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var incr_hid = w[].call_i32("sc_incr_handler", args_ptr(app))
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    var retry_hid = w[].call_i32("sc_retry_handler", args_ptr(app))
    # Increment 5 times
    for _ in range(5):
        _ = _handle_event(w, app, incr_hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sc_count_value", args_ptr(app)), 5)
    # Crash + flush + retry + flush
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    _ = _handle_event(w, app, retry_hid)
    _ = _flush(w, app, buf)
    # Increment 20 more times
    for _ in range(20):
        _ = _handle_event(w, app, incr_hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sc_count_value", args_ptr(app)), 25)
    _destroy_sc(w, app, buf)


# ── Test: scope count ────────────────────────────────────────────────────────


fn test_sc_scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Three scopes: parent + normal child + fallback child."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("sc_scope_count", args_ptr(app))
    assert_equal(count, 3)
    _destroy_sc(w, app, buf)


# ── Test: scope IDs are distinct ─────────────────────────────────────────────


fn test_sc_scope_ids_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Parent, normal child, and fallback child have distinct scope IDs."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var parent = w[].call_i32("sc_parent_scope_id", args_ptr(app))
    var normal = w[].call_i32("sc_normal_scope_id", args_ptr(app))
    var fallback = w[].call_i32("sc_fallback_scope_id", args_ptr(app))
    assert_true(parent != normal, msg="parent != normal scope")
    assert_true(parent != fallback, msg="parent != fallback scope")
    assert_true(normal != fallback, msg="normal != fallback scope")
    _destroy_sc(w, app, buf)


# ── Test: handler IDs are valid ──────────────────────────────────────────────


fn test_sc_handler_ids_valid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Crash, retry, and increment handler IDs are all distinct and non-negative.
    """
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var crash = w[].call_i32("sc_crash_handler", args_ptr(app))
    var retry = w[].call_i32("sc_retry_handler", args_ptr(app))
    var incr = w[].call_i32("sc_incr_handler", args_ptr(app))
    assert_true(crash >= 0, msg="crash handler ID >= 0")
    assert_true(retry >= 0, msg="retry handler ID >= 0")
    assert_true(incr >= 0, msg="incr handler ID >= 0")
    assert_true(crash != retry, msg="crash != retry")
    assert_true(crash != incr, msg="crash != incr")
    assert_true(retry != incr, msg="retry != incr")
    _destroy_sc(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_sc_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush without any state change returns 0."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, 0)
    _destroy_sc(w, app, buf)


# ── Test: destroy with active error ──────────────────────────────────────────


fn test_sc_destroy_with_active_error(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy while error is active does not crash."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, crash_hid)
    _ = _flush(w, app, buf)
    # Destroy while in error state — should not crash
    _destroy_sc(w, app, buf)


# ── Test: increment after crash without flush ────────────────────────────────


fn test_sc_increment_after_crash_no_flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Increment after crash (before flush) still updates count."""
    var t = _create_sc(w)
    var app = t[0]
    var buf = t[1]
    var incr_hid = w[].call_i32("sc_incr_handler", args_ptr(app))
    var crash_hid = w[].call_i32("sc_crash_handler", args_ptr(app))
    _ = _handle_event(w, app, incr_hid)
    _ = _handle_event(w, app, crash_hid)
    # Count should be 1 even though we crashed before flushing
    assert_equal(w[].call_i32("sc_count_value", args_ptr(app)), 1)
    _destroy_sc(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point — runs all tests via a shared WASM instance
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = _load()

    print("test_safe_counter — SafeCounterApp error boundary (Phase 32.2):")

    test_sc_init_creates_app(wp)
    print("  ✓ init creates app")

    test_sc_count_starts_at_0(wp)
    print("  ✓ count starts at 0")

    test_sc_has_error_initially_false(wp)
    print("  ✓ has_error initially false")

    test_sc_normal_mounted_after_rebuild(wp)
    print("  ✓ normal mounted after rebuild")

    test_sc_fallback_not_mounted_initially(wp)
    print("  ✓ fallback not mounted initially")

    test_sc_increment_updates_count(wp)
    print("  ✓ increment updates count")

    test_sc_crash_sets_error(wp)
    print("  ✓ crash sets error")

    test_sc_flush_after_crash_hides_normal(wp)
    print("  ✓ flush after crash hides normal")

    test_sc_flush_after_crash_shows_fallback(wp)
    print("  ✓ flush after crash shows fallback")

    test_sc_retry_clears_error(wp)
    print("  ✓ retry clears error")

    test_sc_flush_after_retry_shows_normal(wp)
    print("  ✓ flush after retry shows normal")

    test_sc_flush_after_retry_hides_fallback(wp)
    print("  ✓ flush after retry hides fallback")

    test_sc_count_preserved_after_crash_recovery(wp)
    print("  ✓ count preserved after crash/recovery")

    test_sc_multiple_crash_retry_cycles(wp)
    print("  ✓ multiple crash/retry cycles")

    test_sc_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_sc_rapid_increments_after_recovery(wp)
    print("  ✓ rapid increments after recovery")

    test_sc_scope_count(wp)
    print("  ✓ scope count")

    test_sc_scope_ids_distinct(wp)
    print("  ✓ scope IDs distinct")

    test_sc_handler_ids_valid(wp)
    print("  ✓ handler IDs valid")

    test_sc_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_sc_destroy_with_active_error(wp)
    print("  ✓ destroy with active error")

    test_sc_increment_after_crash_no_flush(wp)
    print("  ✓ increment after crash without flush")

    print("  ✓ test_safe_counter — safe_counter: 22/22 passed")
