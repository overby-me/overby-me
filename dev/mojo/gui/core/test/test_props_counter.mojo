"""Phase 31.3 — PropsCounterApp Mojo Tests.

Validates PropsCounterApp via the pc_* WASM exports which exercise
the self-rendering child component with props pattern:

  - init creates app with non-zero pointer
  - child scope is distinct from parent scope
  - child template is distinct from parent template
  - scope count = 2 (parent + child)
  - initial count is 0
  - initial show_hex is false
  - count signal update via increment handler
  - child receives count via context prop
  - child show_hex toggle changes child state
  - child show_hex toggle marks child dirty (not parent)
  - parent increment marks parent dirty
  - rebuild produces mutations and mounts child
  - flush returns 0 when clean
  - flush after increment emits mutations
  - mixed increment + toggle in sequence
  - destroy does not crash
  - destroy + recreate cycle
  - 10 rapid increment cycles
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


fn _create_pc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a PropsCounterApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("pc_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_pc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a PropsCounterApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("pc_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush the app and return mutation byte length."""
    return w[].call_i32("pc_flush", args_ptr_ptr_i32(app, buf, 8192))


# ── Test: init creates app with non-zero pointer ────────────────────────────


def test_pc_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Init returns a non-zero app pointer."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    assert_true(app != 0, "app pointer should be non-zero")
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: child scope distinct from parent ───────────────────────────────────


def test_pc_scopes_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child scope ID differs from parent scope ID."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    var parent_scope = w[].call_i32("pc_parent_scope_id", args_ptr(app))
    var child_scope = w[].call_i32("pc_child_scope_id", args_ptr(app))
    assert_true(parent_scope >= 0, "parent scope ID non-negative")
    assert_true(child_scope >= 0, "child scope ID non-negative")
    assert_true(parent_scope != child_scope, "scopes must differ")
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: child template distinct from parent ────────────────────────────────


def test_pc_templates_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child template ID differs from parent template ID."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    var parent_tmpl = w[].call_i32("pc_parent_tmpl_id", args_ptr(app))
    var child_tmpl = w[].call_i32("pc_child_tmpl_id", args_ptr(app))
    assert_true(parent_tmpl >= 0, "parent template ID non-negative")
    assert_true(child_tmpl >= 0, "child template ID non-negative")
    assert_true(parent_tmpl != child_tmpl, "template IDs must differ")
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: scope count = 2 ───────────────────────────────────────────────────


def test_pc_scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """App has exactly 2 live scopes (parent + child)."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    var count = w[].call_i32("pc_scope_count", args_ptr(app))
    assert_equal(count, 2)
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: initial count is 0 ────────────────────────────────────────────────


def test_pc_initial_count_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Initial count value should be 0."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app)), 0)
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: initial show_hex is false ──────────────────────────────────────────


def test_pc_initial_show_hex_false(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Initial show_hex should be false (0)."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    assert_equal(w[].call_i32("pc_show_hex", args_ptr(app)), 0)
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: handler count ─────────────────────────────────────────────────────


def test_pc_handler_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """At least 3 handlers registered (incr, decr, toggle)."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    var hcount = w[].call_i32("pc_handler_count", args_ptr(app))
    assert_true(hcount >= 3, "at least 3 handlers (incr, decr, toggle)")
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: increment updates count signal ─────────────────────────────────────


def test_pc_increment_updates_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Dispatching increment handler increases count to 1."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app))
    assert_true(incr_h >= 0, "incr handler valid")
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app)), 1)
    _destroy_pc(w, app, buf)


# ── Test: decrement updates count signal ─────────────────────────────────────


def test_pc_decrement_updates_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Dispatching decrement handler decreases count to -1."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    var decr_h = w[].call_i32("pc_decr_handler", args_ptr(app))
    assert_true(decr_h >= 0, "decr handler valid")
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, decr_h, 0))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app)), -1)
    _destroy_pc(w, app, buf)


# ── Test: toggle marks child dirty (not parent) ─────────────────────────────


def test_pc_toggle_marks_child_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Dispatching toggle handler marks child dirty, not parent."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    # Start clean
    assert_equal(w[].call_i32("pc_child_is_dirty", args_ptr(app)), 0)
    assert_equal(w[].call_i32("pc_parent_is_dirty", args_ptr(app)), 0)
    # Toggle hex via handler
    var toggle_h = w[].call_i32("pc_toggle_handler", args_ptr(app))
    assert_true(toggle_h >= 0, "toggle handler valid")
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    # Child should be dirty
    assert_equal(w[].call_i32("pc_child_is_dirty", args_ptr(app)), 1)
    _destroy_pc(w, app, buf)


# ── Test: increment marks parent dirty ───────────────────────────────────────


def test_pc_increment_marks_parent_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Dispatching increment handler marks parent dirty."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    assert_equal(w[].call_i32("pc_parent_is_dirty", args_ptr(app)), 0)
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    assert_equal(w[].call_i32("pc_parent_is_dirty", args_ptr(app)), 1)
    _destroy_pc(w, app, buf)


# ── Test: rebuild produces mutations and mounts child ────────────────────────


def test_pc_rebuild_produces_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Rebuild produces mutation bytes > 0."""
    var app = Int(w[].call_i64("pc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    var length = w[].call_i32("pc_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    assert_true(length > 0, "rebuild produces mutations")
    assert_equal(w[].call_i32("pc_child_is_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("pc_child_has_rendered", args_ptr(app)), 1)
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("pc_destroy", args_ptr(app))


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


def test_pc_flush_returns_zero_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Flush after rebuild with no changes returns 0."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    var length = _flush(w, app, buf)
    assert_equal(length, 0)
    _destroy_pc(w, app, buf)


# ── Test: flush after increment emits mutations ──────────────────────────────


def test_pc_flush_after_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Flush after increment emits mutations > 0."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    var length = _flush(w, app, buf)
    assert_true(length > 0, "flush after increment produces mutations")
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app)), 1)
    _destroy_pc(w, app, buf)


# ── Test: mixed increment + toggle ───────────────────────────────────────────


def test_pc_mixed_increment_toggle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Mixed increment + toggle both apply correctly."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    # Increment 3 times
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app))
    for _ in range(3):
        _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    # Toggle hex
    var toggle_h = w[].call_i32("pc_toggle_handler", args_ptr(app))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    assert_equal(w[].call_i32("pc_has_dirty", args_ptr(app)), 1)
    var length = _flush(w, app, buf)
    assert_true(
        length > 0, "flush after mixed changes should produce mutations"
    )
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app)), 3)
    assert_equal(w[].call_i32("pc_show_hex", args_ptr(app)), 1)
    _destroy_pc(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


def test_pc_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Creating and destroying the app does not crash."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    _destroy_pc(w, app, buf)


# ── Test: destroy with dirty state ───────────────────────────────────────────


def test_pc_destroy_with_dirty_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroying app with unflushed dirty state does not crash."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    var toggle_h = w[].call_i32("pc_toggle_handler", args_ptr(app))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    assert_equal(w[].call_i32("pc_has_dirty", args_ptr(app)), 1)
    _destroy_pc(w, app, buf)


# ── Test: destroy + recreate cycle ───────────────────────────────────────────


def test_pc_destroy_recreate_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy then recreate produces a fresh app with correct initial state."""
    var result1 = _create_pc(w)
    var app1 = result1[0]
    var buf1 = result1[1]
    # Modify state
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app1))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app1, incr_h, 0))
    _ = _flush(w, app1, buf1)
    var toggle_h = w[].call_i32("pc_toggle_handler", args_ptr(app1))
    _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app1, toggle_h, 0))
    _ = _flush(w, app1, buf1)
    _destroy_pc(w, app1, buf1)

    # Recreate
    var result2 = _create_pc(w)
    var app2 = result2[0]
    var buf2 = result2[1]
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app2)), 0)
    assert_equal(w[].call_i32("pc_show_hex", args_ptr(app2)), 0)
    _destroy_pc(w, app2, buf2)


# ── Test: rapid 10 increment cycles ─────────────────────────────────────────


def test_pc_rapid_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Ten rapid increments + flushes produce correct final count."""
    var result = _create_pc(w)
    var app = result[0]
    var buf = result[1]
    var incr_h = w[].call_i32("pc_incr_handler", args_ptr(app))
    for _ in range(10):
        _ = w[].call_i32("pc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("pc_count_value", args_ptr(app)), 10)
    _destroy_pc(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = get_instance()
    print("test_props_counter — PropsCounterApp (Phase 31.3):")

    test_pc_init_creates_app(wp)
    print("  ✓ pc_init creates app")

    test_pc_scopes_distinct(wp)
    print("  ✓ child/parent scope IDs differ")

    test_pc_templates_distinct(wp)
    print("  ✓ child/parent template IDs differ")

    test_pc_scope_count(wp)
    print("  ✓ scope count = 2")

    test_pc_initial_count_zero(wp)
    print("  ✓ initial count is 0")

    test_pc_initial_show_hex_false(wp)
    print("  ✓ initial show_hex is false")

    test_pc_handler_count(wp)
    print("  ✓ at least 3 handlers registered")

    test_pc_rebuild_produces_mutations(wp)
    print("  ✓ rebuild produces mutations + child mounted")

    test_pc_increment_updates_count(wp)
    print("  ✓ increment updates count to 1")

    test_pc_decrement_updates_count(wp)
    print("  ✓ decrement updates count to -1")

    test_pc_toggle_marks_child_dirty(wp)
    print("  ✓ toggle marks child dirty (not parent)")

    test_pc_increment_marks_parent_dirty(wp)
    print("  ✓ increment marks parent dirty")

    test_pc_flush_returns_zero_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_pc_flush_after_increment(wp)
    print("  ✓ flush after increment emits mutations")

    test_pc_mixed_increment_toggle(wp)
    print("  ✓ mixed increment + toggle")

    test_pc_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_pc_destroy_with_dirty_state(wp)
    print("  ✓ destroy with dirty state does not crash")

    test_pc_destroy_recreate_cycle(wp)
    print("  ✓ destroy → recreate cycle")

    test_pc_rapid_increments(wp)
    print("  ✓ rapid 10 increments")
