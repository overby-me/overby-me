"""Phase 31.4 — ThemeCounterApp Mojo Tests.

Validates ThemeCounterApp via the tc_* WASM exports which exercise
shared context across multiple child components with upward communication:

  - init creates app with 3 distinct scopes (parent, counter, summary)
  - count starts at 0, theme starts light
  - increment updates count
  - both children consume same count signal
  - theme toggle updates theme signal
  - callback signal starts at 0
  - child reset writes to callback signal
  - parent flush detects reset callback and clears count
  - rebuild produces mutations and mounts both children
  - flush returns 0 when clean
  - mixed: increment + toggle theme + flush
  - reset callback: count returns to 0
  - multiple increment → reset → increment cycle
  - destroy cleans up all 3 scopes
  - destroy with dirty state does not crash
  - destroy + recreate cycle
  - 10 create/destroy cycles bounded memory
  - rapid 20 increments + flush each
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


fn _load() raises -> WasmInstance:
    return WasmInstance("build/out.wasm")


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_tc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a ThemeCounterApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("tc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("tc_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_tc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a ThemeCounterApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("tc_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush the app and return mutation byte length."""
    return w[].call_i32("tc_flush", args_ptr_ptr_i32(app, buf, 8192))


# ── Test: init creates 3 distinct scopes ─────────────────────────────────────


def test_tc_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Init returns a non-zero app pointer with 3 distinct scopes."""
    var app = Int(w[].call_i64("tc_init", no_args()))
    assert_true(app != 0, "app pointer should be non-zero")
    var parent_scope = w[].call_i32("tc_parent_scope_id", args_ptr(app))
    var counter_scope = w[].call_i32("tc_counter_scope_id", args_ptr(app))
    var summary_scope = w[].call_i32("tc_summary_scope_id", args_ptr(app))
    assert_true(parent_scope >= 0, "parent scope ID non-negative")
    assert_true(counter_scope >= 0, "counter scope ID non-negative")
    assert_true(summary_scope >= 0, "summary scope ID non-negative")
    assert_true(parent_scope != counter_scope, "parent != counter scope")
    assert_true(parent_scope != summary_scope, "parent != summary scope")
    assert_true(counter_scope != summary_scope, "counter != summary scope")
    w[].call_void("tc_destroy", args_ptr(app))


# ── Test: scope count = 3 ───────────────────────────────────────────────────


def test_tc_scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """App has exactly 3 live scopes (parent + counter + summary)."""
    var app = Int(w[].call_i64("tc_init", no_args()))
    var count = w[].call_i32("tc_scope_count", args_ptr(app))
    assert_equal(count, 3)
    w[].call_void("tc_destroy", args_ptr(app))


# ── Test: initial values ─────────────────────────────────────────────────────


def test_tc_initial_values(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Initial count is 0, theme is light, on_reset is 0."""
    var app = Int(w[].call_i64("tc_init", no_args()))
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 0)
    assert_equal(w[].call_i32("tc_theme_is_dark", args_ptr(app)), 0)
    assert_equal(w[].call_i32("tc_on_reset_value", args_ptr(app)), 0)
    w[].call_void("tc_destroy", args_ptr(app))


# ── Test: handler count ──────────────────────────────────────────────────────


def test_tc_handler_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """At least 3 handlers registered (toggle theme, increment, reset)."""
    var app = Int(w[].call_i64("tc_init", no_args()))
    var hcount = w[].call_i32("tc_handler_count", args_ptr(app))
    assert_true(hcount >= 3, "at least 3 handlers")
    w[].call_void("tc_destroy", args_ptr(app))


# ── Test: increment updates count ────────────────────────────────────────────


def test_tc_increment_updates_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Dispatching increment handler increases count to 1."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    assert_true(incr_h >= 0, "incr handler valid")
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 1)
    _destroy_tc(w, app, buf)


# ── Test: theme toggle ───────────────────────────────────────────────────────


def test_tc_theme_toggle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Toggle theme switches from light to dark."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("tc_theme_is_dark", args_ptr(app)), 0)
    var toggle_h = w[].call_i32("tc_toggle_theme_handler", args_ptr(app))
    assert_true(toggle_h >= 0, "toggle handler valid")
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_theme_is_dark", args_ptr(app)), 1)
    _destroy_tc(w, app, buf)


# ── Test: rebuild produces mutations and mounts children ─────────────────────


def test_tc_rebuild_produces_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Rebuild produces mutation bytes > 0 and mounts both children."""
    var app = Int(w[].call_i64("tc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    var length = w[].call_i32("tc_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    assert_true(length > 0, "rebuild produces mutations")
    assert_equal(w[].call_i32("tc_counter_is_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("tc_summary_is_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("tc_counter_has_rendered", args_ptr(app)), 1)
    assert_equal(w[].call_i32("tc_summary_has_rendered", args_ptr(app)), 1)
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("tc_destroy", args_ptr(app))


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


def test_tc_flush_returns_zero_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Flush after rebuild with no changes returns 0."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var length = _flush(w, app, buf)
    assert_equal(length, 0)
    _destroy_tc(w, app, buf)


# ── Test: callback signal starts at 0 ───────────────────────────────────────


def test_tc_callback_starts_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """On_reset callback signal is 0 after init."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("tc_on_reset_value", args_ptr(app)), 0)
    _destroy_tc(w, app, buf)


# ── Test: reset handler writes callback signal ───────────────────────────────


def test_tc_reset_writes_callback(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Dispatching reset handler sets on_reset to 1."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var reset_h = w[].call_i32("tc_reset_handler", args_ptr(app))
    assert_true(reset_h >= 0, "reset handler valid")
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, reset_h, 0))
    assert_equal(w[].call_i32("tc_on_reset_value", args_ptr(app)), 1)
    _destroy_tc(w, app, buf)


# ── Test: parent flush detects reset and clears count ────────────────────────


def test_tc_reset_clears_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """After increment + reset + flush, count returns to 0."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    # Increment to 3
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    for _ in range(3):
        _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 3)
    # Reset
    var reset_h = w[].call_i32("tc_reset_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, reset_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 0)
    assert_equal(w[].call_i32("tc_on_reset_value", args_ptr(app)), 0)
    _destroy_tc(w, app, buf)


# ── Test: mixed increment + theme toggle ─────────────────────────────────────


def test_tc_mixed_increment_toggle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Mixed increment + theme toggle both apply correctly."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    # Increment twice
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    for _ in range(2):
        _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    # Toggle theme
    var toggle_h = w[].call_i32("tc_toggle_theme_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    var length = _flush(w, app, buf)
    assert_true(length > 0, "flush after mixed changes produces mutations")
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 2)
    assert_equal(w[].call_i32("tc_theme_is_dark", args_ptr(app)), 1)
    _destroy_tc(w, app, buf)


# ── Test: increment → reset → increment cycle ───────────────────────────────


def test_tc_increment_reset_increment_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Increment → reset → increment cycle works correctly."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    var reset_h = w[].call_i32("tc_reset_handler", args_ptr(app))
    # Increment to 5
    for _ in range(5):
        _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 5)
    # Reset
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, reset_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 0)
    # Increment to 3
    for _ in range(3):
        _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 3)
    _destroy_tc(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


def test_tc_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Creating and destroying the app does not crash."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    _destroy_tc(w, app, buf)


# ── Test: destroy with dirty state ───────────────────────────────────────────


def test_tc_destroy_with_dirty_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroying app with unflushed dirty state does not crash."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    var toggle_h = w[].call_i32("tc_toggle_theme_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    var reset_h = w[].call_i32("tc_reset_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, reset_h, 0))
    assert_equal(w[].call_i32("tc_has_dirty", args_ptr(app)), 1)
    _destroy_tc(w, app, buf)


# ── Test: destroy + recreate cycle ───────────────────────────────────────────


def test_tc_destroy_recreate_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy then recreate produces a fresh app with correct initial state."""
    var result1 = _create_tc(w)
    var app1 = result1[0]
    var buf1 = result1[1]
    # Modify state
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app1))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app1, incr_h, 0))
    _ = _flush(w, app1, buf1)
    var toggle_h = w[].call_i32("tc_toggle_theme_handler", args_ptr(app1))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app1, toggle_h, 0))
    _ = _flush(w, app1, buf1)
    _destroy_tc(w, app1, buf1)

    # Recreate
    var result2 = _create_tc(w)
    var app2 = result2[0]
    var buf2 = result2[1]
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app2)), 0)
    assert_equal(w[].call_i32("tc_theme_is_dark", args_ptr(app2)), 0)
    assert_equal(w[].call_i32("tc_on_reset_value", args_ptr(app2)), 0)
    _destroy_tc(w, app2, buf2)


# ── Test: 10 create/destroy cycles ───────────────────────────────────────────


def test_tc_ten_create_destroy_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Ten create/destroy cycles work without leaks or crashes."""
    for i in range(10):
        var result = _create_tc(w)
        var app = result[0]
        var buf = result[1]
        var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
        _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 1)
        _destroy_tc(w, app, buf)


# ── Test: rapid 20 increments ────────────────────────────────────────────────


def test_tc_rapid_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Twenty rapid increments + flushes produce correct final count."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    for _ in range(20):
        _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 20)
    _destroy_tc(w, app, buf)


# ── Test: flush after increment emits mutations ──────────────────────────────


def test_tc_flush_after_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Flush after increment emits mutations > 0."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    var length = _flush(w, app, buf)
    assert_true(length > 0, "flush after increment produces mutations")
    _destroy_tc(w, app, buf)


# ── Test: theme toggle does not affect count ─────────────────────────────────


def test_tc_theme_does_not_affect_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Theme toggle does not change count value."""
    var result = _create_tc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("tc_increment_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 1)
    var toggle_h = w[].call_i32("tc_toggle_theme_handler", args_ptr(app))
    _ = w[].call_i32("tc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("tc_count_value", args_ptr(app)), 1)
    assert_equal(w[].call_i32("tc_theme_is_dark", args_ptr(app)), 1)
    _destroy_tc(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = get_instance()
    print("test_theme_counter — ThemeCounterApp (Phase 31.4):")

    test_tc_init_creates_app(wp)
    print("  ✓ tc_init creates app with 3 distinct scopes")

    test_tc_scope_count(wp)
    print("  ✓ scope count = 3")

    test_tc_initial_values(wp)
    print("  ✓ initial values (count=0, light theme, on_reset=0)")

    test_tc_handler_count(wp)
    print("  ✓ at least 3 handlers registered")

    test_tc_rebuild_produces_mutations(wp)
    print("  ✓ rebuild produces mutations + children mounted")

    test_tc_increment_updates_count(wp)
    print("  ✓ increment updates count to 1")

    test_tc_theme_toggle(wp)
    print("  ✓ theme toggle switches to dark")

    test_tc_flush_returns_zero_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_tc_callback_starts_zero(wp)
    print("  ✓ callback signal starts at 0")

    test_tc_reset_writes_callback(wp)
    print("  ✓ reset handler writes callback signal")

    test_tc_reset_clears_count(wp)
    print("  ✓ reset clears count to 0")

    test_tc_mixed_increment_toggle(wp)
    print("  ✓ mixed increment + theme toggle")

    test_tc_increment_reset_increment_cycle(wp)
    print("  ✓ increment → reset → increment cycle")

    test_tc_flush_after_increment(wp)
    print("  ✓ flush after increment emits mutations")

    test_tc_theme_does_not_affect_count(wp)
    print("  ✓ theme toggle does not affect count")

    test_tc_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_tc_destroy_with_dirty_state(wp)
    print("  ✓ destroy with dirty state does not crash")

    test_tc_destroy_recreate_cycle(wp)
    print("  ✓ destroy → recreate cycle")

    test_tc_ten_create_destroy_cycles(wp)
    print("  ✓ 10 create/destroy cycles")

    test_tc_rapid_increments(wp)
    print("  ✓ rapid 20 increments")
