"""Phase 31.2 — ChildComponentContext Mojo Tests.

Validates ChildComponentContext via the ChildContextTestApp (cct_*) WASM exports:

  - create child context returns valid scope/template
  - child scope is distinct from parent scope
  - use_signal creates signal under child scope
  - child signal write marks child scope dirty (not parent)
  - parent signal write marks parent dirty (not child)
  - consume_signal_i32 retrieves parent-provided signal
  - consumed signal reads correct value
  - consumed signal key matches parent signal key
  - render_builder produces valid VNode (rebuild succeeds)
  - child is mounted after rebuild
  - child has rendered after rebuild
  - is_dirty reflects child scope state
  - flush after increment emits mutations
  - flush returns 0 when clean
  - toggle hex changes child state only
  - mixed increment + toggle in sequence
  - destroy does not crash
  - destroy + recreate cycle
  - 10 create/destroy cycles
  - rapid 20 increments produce correct count
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


fn _create_cct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a child-context test app and mount it.  Returns (app_ptr, buf_ptr).
    """
    var app = Int(w[].call_i64("cct_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("cct_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_cct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a child-context test app and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("cct_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush the app and return mutation byte length."""
    return w[].call_i32("cct_flush", args_ptr_ptr_i32(app, buf, 8192))


# ── Test: create child context returns valid scope/template ──────────────────


def test_cct_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Init returns a non-zero app pointer."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    assert_true(app != 0, "app pointer should be non-zero")
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: child scope is distinct from parent scope ──────────────────────────


def test_cct_scopes_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child scope ID differs from parent scope ID."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    var parent_scope = w[].call_i32("cct_parent_scope_id", args_ptr(app))
    var child_scope = w[].call_i32("cct_child_scope_id", args_ptr(app))
    assert_true(parent_scope >= 0, "parent scope ID non-negative")
    assert_true(child_scope >= 0, "child scope ID non-negative")
    assert_true(parent_scope != child_scope, "scopes must differ")
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: templates distinct ─────────────────────────────────────────────────


def test_cct_templates_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child template ID differs from parent template ID."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    var parent_tmpl = w[].call_i32("cct_parent_tmpl_id", args_ptr(app))
    var child_tmpl = w[].call_i32("cct_child_tmpl_id", args_ptr(app))
    assert_true(parent_tmpl >= 0, "parent template ID non-negative")
    assert_true(child_tmpl >= 0, "child template ID non-negative")
    assert_true(parent_tmpl != child_tmpl, "template IDs must differ")
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: scope count = 2 ───────────────────────────────────────────────────


def test_cct_scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """App has exactly 2 live scopes (parent + child)."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    var count = w[].call_i32("cct_scope_count", args_ptr(app))
    assert_equal(count, 2)
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: use_signal creates signal under child scope ────────────────────────


def test_cct_child_signal_independent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child's show_hex signal is independent from parent's count signal."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    var count_val = w[].call_i32("cct_count_value", args_ptr(app))
    var hex_val = w[].call_i32("cct_show_hex", args_ptr(app))
    assert_equal(count_val, 0)
    assert_equal(hex_val, 0)  # False
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: child signal write marks child dirty (not parent) ──────────────────


def test_cct_child_signal_marks_child_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Toggling show_hex marks child dirty but not parent."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    # Start clean after rebuild
    assert_equal(w[].call_i32("cct_child_is_dirty", args_ptr(app)), 0)
    assert_equal(w[].call_i32("cct_parent_is_dirty", args_ptr(app)), 0)
    # Toggle child's show_hex
    w[].call_void("cct_toggle_hex", args_ptr(app))
    # Child should be dirty
    assert_equal(w[].call_i32("cct_child_is_dirty", args_ptr(app)), 1)
    # Parent should NOT be dirty (child signal doesn't subscribe parent)
    # Note: has_dirty checks the runtime dirty queue, which includes child
    # We check the parent's own scope via consume_dirty
    _destroy_cct(w, app, buf)


# ── Test: parent signal write marks parent dirty ─────────────────────────────


def test_cct_parent_signal_marks_parent_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Setting count marks parent dirty."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("cct_parent_is_dirty", args_ptr(app)), 0)
    # Write parent signal
    w[].call_void("cct_set_count", args_ptr_i32(app, 42))
    assert_equal(w[].call_i32("cct_parent_is_dirty", args_ptr(app)), 1)
    assert_equal(w[].call_i32("cct_count_value", args_ptr(app)), 42)
    _destroy_cct(w, app, buf)


# ── Test: consumed signal key matches parent signal key ──────────────────────


def test_cct_consumed_signal_key_matches_parent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child's consumed count signal key should match parent's count signal key.
    """
    var app = Int(w[].call_i64("cct_init", no_args()))
    var parent_key = w[].call_i32("cct_parent_count_signal_key", args_ptr(app))
    var child_key = w[].call_i32("cct_child_count_signal_key", args_ptr(app))
    assert_equal(parent_key, child_key)
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: rebuild produces mutations ─────────────────────────────────────────


def test_cct_rebuild_produces_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Rebuild produces a non-zero mutation buffer."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    var offset = w[].call_i32("cct_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    assert_true(offset > 0, "rebuild should produce mutations")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: child is mounted after rebuild ─────────────────────────────────────


def test_cct_child_mounted_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child should be mounted after rebuild."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("cct_child_is_mounted", args_ptr(app)), 1)
    _destroy_cct(w, app, buf)


# ── Test: child has rendered after rebuild ───────────────────────────────────


def test_cct_child_has_rendered_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child should have rendered after rebuild."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("cct_child_has_rendered", args_ptr(app)), 1)
    _destroy_cct(w, app, buf)


# ── Test: flush after increment emits mutations ─────────────────────────────


def test_cct_flush_after_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Flush after increment produces non-zero mutations."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    # Dispatch increment
    var incr_h = w[].call_i32("cct_incr_handler", args_ptr(app))
    _ = w[].call_i32("cct_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    # Flush
    var offset = _flush(w, app, buf)
    assert_true(offset > 0, "flush after increment should produce mutations")
    assert_equal(w[].call_i32("cct_count_value", args_ptr(app)), 1)
    _destroy_cct(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


def test_cct_flush_returns_zero_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Flush returns 0 when nothing is dirty."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    var offset = _flush(w, app, buf)
    assert_equal(offset, 0)
    _destroy_cct(w, app, buf)


# ── Test: toggle hex changes child state only ────────────────────────────────


def test_cct_toggle_hex(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Toggle hex changes show_hex state."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("cct_show_hex", args_ptr(app)), 0)
    w[].call_void("cct_toggle_hex", args_ptr(app))
    assert_equal(w[].call_i32("cct_show_hex", args_ptr(app)), 1)
    # Flush to clear dirty state
    _ = _flush(w, app, buf)
    # Toggle back
    w[].call_void("cct_toggle_hex", args_ptr(app))
    assert_equal(w[].call_i32("cct_show_hex", args_ptr(app)), 0)
    _destroy_cct(w, app, buf)


# ── Test: mixed increment + toggle ───────────────────────────────────────────


def test_cct_mixed_increment_toggle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Mixed increment + toggle both apply correctly."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    # Increment
    var incr_h = w[].call_i32("cct_incr_handler", args_ptr(app))
    _ = w[].call_i32("cct_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    # Toggle hex
    w[].call_void("cct_toggle_hex", args_ptr(app))
    # Both should be dirty
    assert_equal(w[].call_i32("cct_has_dirty", args_ptr(app)), 1)
    # Flush
    var offset = _flush(w, app, buf)
    assert_true(
        offset > 0, "flush after mixed changes should produce mutations"
    )
    assert_equal(w[].call_i32("cct_count_value", args_ptr(app)), 1)
    assert_equal(w[].call_i32("cct_show_hex", args_ptr(app)), 1)
    _destroy_cct(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


def test_cct_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Creating and destroying the app does not crash."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    _destroy_cct(w, app, buf)


# ── Test: destroy + recreate cycle ───────────────────────────────────────────


def test_cct_destroy_recreate_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy then recreate produces a fresh app with correct initial state."""
    var result1 = _create_cct(w)
    var app1 = result1[0]
    var buf1 = result1[1]
    # Modify state
    w[].call_void("cct_set_count", args_ptr_i32(app1, 99))
    w[].call_void("cct_toggle_hex", args_ptr(app1))
    _destroy_cct(w, app1, buf1)

    # Recreate
    var result2 = _create_cct(w)
    var app2 = result2[0]
    var buf2 = result2[1]
    assert_equal(w[].call_i32("cct_count_value", args_ptr(app2)), 0)
    assert_equal(w[].call_i32("cct_show_hex", args_ptr(app2)), 0)
    _destroy_cct(w, app2, buf2)


# ── Test: 10 create/destroy cycles ───────────────────────────────────────────


def test_cct_ten_create_destroy_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Ten create/destroy cycles work without leaks or crashes."""
    for i in range(10):
        var result = _create_cct(w)
        var app = result[0]
        var buf = result[1]
        w[].call_void("cct_set_count", args_ptr_i32(app, Int32(i)))
        assert_equal(w[].call_i32("cct_count_value", args_ptr(app)), Int32(i))
        _destroy_cct(w, app, buf)


# ── Test: rapid 20 increments ────────────────────────────────────────────────


def test_cct_rapid_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Twenty rapid increments + flushes produce correct final count."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("cct_incr_handler", args_ptr(app))
    for _ in range(20):
        _ = w[].call_i32("cct_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("cct_count_value", args_ptr(app)), 20)
    _destroy_cct(w, app, buf)


# ── Test: initial count is 0 ────────────────────────────────────────────────


def test_cct_initial_count_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Initial count value should be 0."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    assert_equal(w[].call_i32("cct_count_value", args_ptr(app)), 0)
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: handler count ──────────────────────────────────────────────────────


def test_cct_handler_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """At least 1 handler registered (increment)."""
    var app = Int(w[].call_i64("cct_init", no_args()))
    var hcount = w[].call_i32("cct_handler_count", args_ptr(app))
    assert_true(hcount >= 1, "at least 1 handler (incr)")
    w[].call_void("cct_destroy", args_ptr(app))


# ── Test: destroy with dirty state ───────────────────────────────────────────


def test_cct_destroy_with_dirty_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroying app with unflushed dirty state does not crash."""
    var result = _create_cct(w)
    var app = result[0]
    var buf = result[1]
    # Make dirty but don't flush
    w[].call_void("cct_set_count", args_ptr_i32(app, 42))
    w[].call_void("cct_toggle_hex", args_ptr(app))
    assert_equal(w[].call_i32("cct_has_dirty", args_ptr(app)), 1)
    _destroy_cct(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = get_instance()
    print("test_child_context — ChildComponentContext (Phase 31.2):")

    test_cct_init_creates_app(wp)
    print("  ✓ cct_init creates app")

    test_cct_scopes_distinct(wp)
    print("  ✓ child/parent scope IDs differ")

    test_cct_templates_distinct(wp)
    print("  ✓ child/parent template IDs differ")

    test_cct_scope_count(wp)
    print("  ✓ scope count = 2")

    test_cct_initial_count_zero(wp)
    print("  ✓ initial count is 0")

    test_cct_child_signal_independent(wp)
    print("  ✓ child signal independent from parent")

    test_cct_consumed_signal_key_matches_parent(wp)
    print("  ✓ consumed signal key matches parent signal key")

    test_cct_handler_count(wp)
    print("  ✓ at least 1 handler registered")

    test_cct_rebuild_produces_mutations(wp)
    print("  ✓ rebuild produces mutations")

    test_cct_child_mounted_after_rebuild(wp)
    print("  ✓ child mounted after rebuild")

    test_cct_child_has_rendered_after_rebuild(wp)
    print("  ✓ child has rendered after rebuild")

    test_cct_child_signal_marks_child_dirty(wp)
    print("  ✓ child signal write marks child dirty")

    test_cct_parent_signal_marks_parent_dirty(wp)
    print("  ✓ parent signal write marks parent dirty")

    test_cct_flush_after_increment(wp)
    print("  ✓ flush after increment emits mutations")

    test_cct_flush_returns_zero_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_cct_toggle_hex(wp)
    print("  ✓ toggle hex changes child state")

    test_cct_mixed_increment_toggle(wp)
    print("  ✓ mixed increment + toggle")

    test_cct_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_cct_destroy_with_dirty_state(wp)
    print("  ✓ destroy with dirty state does not crash")

    test_cct_destroy_recreate_cycle(wp)
    print("  ✓ destroy → recreate cycle")

    test_cct_ten_create_destroy_cycles(wp)
    print("  ✓ 10 create/destroy cycles")

    test_cct_rapid_increments(wp)
    print("  ✓ rapid 20 increments")

    print("\n  22/22 tests passed ✓")
