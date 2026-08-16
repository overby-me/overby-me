"""Phase 29 — Component Composition Mojo Tests.

Validates ChildComponent via the child-counter app (cc_*) WASM exports:

  - cc_init creates app with distinct parent/child scope and template IDs
  - cc_rebuild produces mutations (mount)
  - cc_handle_event increments/decrements the count signal
  - cc_flush emits SetText after increment
  - cc_flush returns 0 when nothing dirty
  - child scope ID differs from parent scope ID
  - child template ID differs from parent template ID
  - child has no event bindings (display only)
  - child is mounted after rebuild
  - destroy does not crash
  - destroy → recreate cycle works correctly
  - 10 create/destroy cycles with state verification
  - mixed increment/decrement produces correct count
  - rapid 50 increments produce correct final count
  - destroy with dirty (unflushed) state does not crash
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


fn _load() raises -> WasmInstance:
    return WasmInstance("build/out.wasm")


# ── Helper: create + mount a child-counter app ──────────────────────────────


fn _create_cc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a child-counter app and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("cc_rebuild", args_ptr_ptr_i32(app, buf, 4096))
    return Tuple(app, buf)


fn _destroy_cc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a child-counter app and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("cc_destroy", args_ptr(app))


# ── cc_init creates app with correct state ───────────────────────────────────


def test_cc_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """cc_init returns a non-zero app pointer."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    assert_true(app != 0, "app pointer should be non-zero")
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_init_count_starts_at_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Initial count value should be 0."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var count = w[].call_i32("cc_count_value", args_ptr(app))
    assert_equal(count, 0)
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_parent_child_scope_differ(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child scope ID should differ from parent scope ID."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var parent_scope = w[].call_i32("cc_parent_scope_id", args_ptr(app))
    var child_scope = w[].call_i32("cc_child_scope_id", args_ptr(app))
    assert_true(parent_scope >= 0, "parent scope ID is non-negative")
    assert_true(child_scope >= 0, "child scope ID is non-negative")
    assert_true(parent_scope != child_scope, "scopes must differ")
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_parent_child_tmpl_differ(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child template ID should differ from parent template ID."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var parent_tmpl = w[].call_i32("cc_parent_tmpl_id", args_ptr(app))
    var child_tmpl = w[].call_i32("cc_child_tmpl_id", args_ptr(app))
    assert_true(parent_tmpl >= 0, "parent template ID is non-negative")
    assert_true(child_tmpl >= 0, "child template ID is non-negative")
    assert_true(parent_tmpl != child_tmpl, "template IDs must differ")
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_child_no_events(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child component has no event bindings (display only)."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var child_events = w[].call_i32("cc_child_event_count", args_ptr(app))
    assert_equal(child_events, 0)
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_handler_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """At least 2 handlers registered (increment + decrement)."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var count = w[].call_i32("cc_handler_count", args_ptr(app))
    assert_true(count >= 2, "at least 2 handlers expected")
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_incr_decr_handlers_valid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Increment and decrement handlers are valid and distinct."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
    var decr_h = w[].call_i32("cc_decr_handler", args_ptr(app))
    assert_true(incr_h >= 0, "incr handler is non-negative")
    assert_true(decr_h >= 0, "decr handler is non-negative")
    assert_true(incr_h != decr_h, "handlers must differ")
    w[].call_void("cc_destroy", args_ptr(app))


# ── cc_rebuild produces mutations ────────────────────────────────────────────


def test_cc_rebuild_produces_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """cc_rebuild produces a non-zero mutation buffer."""
    var app = Int(w[].call_i64("cc_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    var offset = w[].call_i32("cc_rebuild", args_ptr_ptr_i32(app, buf, 4096))
    assert_true(offset > 0, "rebuild should produce mutations")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("cc_destroy", args_ptr(app))


def test_cc_child_mounted_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child should be mounted in the DOM after rebuild."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]
    var mounted = w[].call_i32("cc_child_is_mounted", args_ptr(app))
    assert_equal(mounted, 1)
    _destroy_cc(w, app, buf)


def test_cc_child_has_rendered_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child should have rendered at least once after rebuild."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]
    var rendered = w[].call_i32("cc_child_has_rendered", args_ptr(app))
    assert_equal(rendered, 1)
    _destroy_cc(w, app, buf)


# ── cc_handle_event increments/decrements ────────────────────────────────────


def test_cc_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Handle event increment updates count signal."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
    var r = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    assert_equal(r, 1)
    assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 1)

    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 2)

    _destroy_cc(w, app, buf)


def test_cc_decrement(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Handle event decrement updates count signal."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var decr_h = w[].call_i32("cc_decr_handler", args_ptr(app))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, decr_h, 0))
    assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), -1)

    _destroy_cc(w, app, buf)


def test_cc_mixed_incr_decr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Mixed increment/decrement produces correct count."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
    var decr_h = w[].call_i32("cc_decr_handler", args_ptr(app))

    # +1, +1, +1, -1, -1 → 1
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, decr_h, 0))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, decr_h, 0))
    assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 1)

    _destroy_cc(w, app, buf)


# ── cc_flush ─────────────────────────────────────────────────────────────────


def test_cc_flush_after_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """cc_flush produces mutations after increment."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))

    var flush_len = w[].call_i32("cc_flush", args_ptr_ptr_i32(app, buf, 4096))
    assert_true(flush_len > 0, "flush should produce mutations")

    _destroy_cc(w, app, buf)


def test_cc_flush_returns_zero_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """cc_flush returns 0 when nothing is dirty."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var flush_len = w[].call_i32("cc_flush", args_ptr_ptr_i32(app, buf, 4096))
    assert_equal(flush_len, 0)

    _destroy_cc(w, app, buf)


def test_cc_multiple_flush_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Multiple flush cycles produce mutations each time."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))

    for i in range(5):
        _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        var flush_len = w[].call_i32(
            "cc_flush", args_ptr_ptr_i32(app, buf, 4096)
        )
        assert_true(flush_len > 0, "flush cycle should produce mutations")

    assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 5)

    _destroy_cc(w, app, buf)


# ── Destroy lifecycle ────────────────────────────────────────────────────────


def test_cc_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy after use does not crash."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    _ = w[].call_i32("cc_flush", args_ptr_ptr_i32(app, buf, 4096))

    _destroy_cc(w, app, buf)
    assert_true(True, "destroy did not crash")


def test_cc_destroy_with_dirty_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy with unflushed dirty state does not crash."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    # Do NOT flush — destroy with dirty state
    _destroy_cc(w, app, buf)
    assert_true(True, "destroy with dirty state did not crash")


def test_cc_destroy_recreate_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy → recreate cycle works correctly (clean state)."""
    # First instance: create, increment, destroy
    var tup1 = _create_cc(w)
    var app1 = tup1[0]
    var buf1 = tup1[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app1))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app1, incr_h, 0))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app1, incr_h, 0))
    _ = w[].call_i32("cc_flush", args_ptr_ptr_i32(app1, buf1, 4096))
    assert_equal(w[].call_i32("cc_count_value", args_ptr(app1)), 2)

    _destroy_cc(w, app1, buf1)

    # Second instance: should start fresh
    var tup2 = _create_cc(w)
    var app2 = tup2[0]
    var buf2 = tup2[1]

    assert_equal(w[].call_i32("cc_count_value", args_ptr(app2)), 0)

    var incr_h2 = w[].call_i32("cc_incr_handler", args_ptr(app2))
    _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app2, incr_h2, 0))
    _ = w[].call_i32("cc_flush", args_ptr_ptr_i32(app2, buf2, 4096))
    assert_equal(w[].call_i32("cc_count_value", args_ptr(app2)), 1)

    # Child should still be mounted after recreate
    var mounted = w[].call_i32("cc_child_is_mounted", args_ptr(app2))
    assert_equal(mounted, 1)

    _destroy_cc(w, app2, buf2)


def test_cc_ten_create_destroy_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """10 create/destroy cycles with state verification."""
    for cycle in range(10):
        var tup = _create_cc(w)
        var app = tup[0]
        var buf = tup[1]

        # Verify fresh state
        assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 0)

        # Increment a few times
        var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))
        _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = w[].call_i32("cc_flush", args_ptr_ptr_i32(app, buf, 4096))
        assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 2)

        # Child should be mounted
        assert_equal(w[].call_i32("cc_child_is_mounted", args_ptr(app)), 1)

        _destroy_cc(w, app, buf)

    assert_true(True, "10 create/destroy cycles completed")


def test_cc_rapid_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Rapid 50 increments produce correct final count."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))

    for i in range(50):
        _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = w[].call_i32("cc_flush", args_ptr_ptr_i32(app, buf, 4096))

    assert_equal(w[].call_i32("cc_count_value", args_ptr(app)), 50)
    assert_equal(w[].call_i32("cc_child_is_mounted", args_ptr(app)), 1)

    _destroy_cc(w, app, buf)


def test_cc_child_scope_survives_flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Child scope and template IDs remain valid across multiple flushes."""
    var tup = _create_cc(w)
    var app = tup[0]
    var buf = tup[1]

    var child_scope_before = w[].call_i32("cc_child_scope_id", args_ptr(app))
    var child_tmpl_before = w[].call_i32("cc_child_tmpl_id", args_ptr(app))

    var incr_h = w[].call_i32("cc_incr_handler", args_ptr(app))

    for i in range(5):
        _ = w[].call_i32("cc_handle_event", args_ptr_i32_i32(app, incr_h, 0))
        _ = w[].call_i32("cc_flush", args_ptr_ptr_i32(app, buf, 4096))

    var child_scope_after = w[].call_i32("cc_child_scope_id", args_ptr(app))
    var child_tmpl_after = w[].call_i32("cc_child_tmpl_id", args_ptr(app))

    assert_equal(child_scope_before, child_scope_after)
    assert_equal(child_tmpl_before, child_tmpl_after)

    _destroy_cc(w, app, buf)


# ── Test runner ──────────────────────────────────────────────────────────────


fn main() raises:
    var wp = get_instance()
    print("test_child_component — component composition (Phase 29):")

    test_cc_init_creates_app(wp)
    print("  ✓ cc_init creates app")

    test_cc_init_count_starts_at_zero(wp)
    print("  ✓ initial count is 0")

    test_cc_parent_child_scope_differ(wp)
    print("  ✓ parent/child scope IDs differ")

    test_cc_parent_child_tmpl_differ(wp)
    print("  ✓ parent/child template IDs differ")

    test_cc_child_no_events(wp)
    print("  ✓ child has no event bindings")

    test_cc_handler_count(wp)
    print("  ✓ at least 2 handlers registered")

    test_cc_incr_decr_handlers_valid(wp)
    print("  ✓ incr/decr handlers valid and distinct")

    test_cc_rebuild_produces_mutations(wp)
    print("  ✓ rebuild produces mutations")

    test_cc_child_mounted_after_rebuild(wp)
    print("  ✓ child mounted after rebuild")

    test_cc_child_has_rendered_after_rebuild(wp)
    print("  ✓ child has rendered after rebuild")

    test_cc_increment(wp)
    print("  ✓ increment updates count")

    test_cc_decrement(wp)
    print("  ✓ decrement updates count")

    test_cc_mixed_incr_decr(wp)
    print("  ✓ mixed increment/decrement")

    test_cc_flush_after_increment(wp)
    print("  ✓ flush after increment produces mutations")

    test_cc_flush_returns_zero_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_cc_multiple_flush_cycles(wp)
    print("  ✓ multiple flush cycles")

    test_cc_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_cc_destroy_with_dirty_state(wp)
    print("  ✓ destroy with dirty state")

    test_cc_destroy_recreate_cycle(wp)
    print("  ✓ destroy → recreate cycle")

    test_cc_ten_create_destroy_cycles(wp)
    print("  ✓ 10 create/destroy cycles")

    test_cc_rapid_increments(wp)
    print("  ✓ rapid 50 increments")

    test_cc_child_scope_survives_flush(wp)
    print("  ✓ child scope survives flush")

    print("\n  22/22 tests passed ✓")
