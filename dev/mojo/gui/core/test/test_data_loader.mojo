"""Phase 33.2 — DataLoaderApp Mojo Tests.

Validates DataLoaderApp via the dl_* WASM exports which exercise
suspense with load/resolve lifecycle:

  - init creates app with non-zero pointer
  - not pending initially
  - data text initially "(none)"
  - content mounted after rebuild
  - skeleton not mounted initially
  - load sets pending
  - flush after load hides content
  - flush after load shows skeleton
  - resolve clears pending
  - resolve stores data
  - flush after resolve shows content
  - flush after resolve hides skeleton
  - content shows resolved data
  - reload cycle (load → resolve → load → resolve)
  - multiple load/resolve cycles (5 cycles)
  - resolve with different data
  - flush returns 0 when clean
  - destroy does not crash
  - destroy while pending
  - scope IDs distinct
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


fn _create_dl(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a DataLoaderApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("dl_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("dl_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_dl(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a DataLoaderApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("dl_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("dl_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("dl_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _resolve(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    data: String,
) raises:
    """Resolve the pending state with a data string."""
    var data_ptr = w[].write_string_struct(data)
    w[].call_void("dl_resolve", args_ptr_ptr(app, data_ptr))


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_dl_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("dl_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("dl_destroy", args_ptr(app))


# ── Test: not pending initially ──────────────────────────────────────────────


fn test_dl_not_pending_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """App is not in pending state after init + rebuild."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("dl_is_pending", args_ptr(app)), 0)
    _destroy_dl(w, app, buf)


# ── Test: data text initially "(none)" ───────────────────────────────────────


fn test_dl_data_text_initially_none(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Data text is '(none)' after init."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("dl_data_text", args_ptr_ptr(app, out_ptr))
    var result = w[].read_string_struct(out_ptr)
    assert_equal(result, String("(none)"))
    _destroy_dl(w, app, buf)


# ── Test: content mounted after rebuild ──────────────────────────────────────


fn test_dl_content_mounted_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Content child is mounted after initial rebuild."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 1)
    _destroy_dl(w, app, buf)


# ── Test: skeleton not mounted initially ─────────────────────────────────────


fn test_dl_skeleton_not_mounted_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Skeleton child is hidden after initial rebuild."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 0)
    _destroy_dl(w, app, buf)


# ── Test: load sets pending ──────────────────────────────────────────────────


fn test_dl_load_sets_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Load button handler sets pending to true."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    assert_equal(w[].call_i32("dl_is_pending", args_ptr(app)), 1)
    _destroy_dl(w, app, buf)


# ── Test: flush after load hides content ─────────────────────────────────────


fn test_dl_flush_after_load_hides_content(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After load + flush: content is hidden."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 0)
    _destroy_dl(w, app, buf)


# ── Test: flush after load shows skeleton ────────────────────────────────────


fn test_dl_flush_after_load_shows_skeleton(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After load + flush: skeleton is shown."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 1)
    _destroy_dl(w, app, buf)


# ── Test: resolve clears pending ─────────────────────────────────────────────


fn test_dl_resolve_clears_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Resolve clears the pending state."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    _resolve(w, app, String("Hello"))
    assert_equal(w[].call_i32("dl_is_pending", args_ptr(app)), 0)
    _destroy_dl(w, app, buf)


# ── Test: resolve stores data ────────────────────────────────────────────────


fn test_dl_resolve_stores_data(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Resolve stores the data string."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    _resolve(w, app, String("Hello World"))
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("dl_data_text", args_ptr_ptr(app, out_ptr))
    var result = w[].read_string_struct(out_ptr)
    assert_equal(result, String("Hello World"))
    _destroy_dl(w, app, buf)


# ── Test: flush after resolve shows content ──────────────────────────────────


fn test_dl_flush_after_resolve_shows_content(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After load + flush + resolve + flush: content is remounted."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    _resolve(w, app, String("Hello"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 1)
    _destroy_dl(w, app, buf)


# ── Test: flush after resolve hides skeleton ─────────────────────────────────


fn test_dl_flush_after_resolve_hides_skeleton(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After load + flush + resolve + flush: skeleton is hidden."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    _resolve(w, app, String("Hello"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 0)
    _destroy_dl(w, app, buf)


# ── Test: reload cycle ───────────────────────────────────────────────────────


fn test_dl_reload_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Load → resolve → load → resolve works correctly."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))

    # First load/resolve
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 0)
    _resolve(w, app, String("First"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 0)

    # Second load/resolve
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 0)
    _resolve(w, app, String("Second"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 0)

    _destroy_dl(w, app, buf)


# ── Test: multiple load/resolve cycles ───────────────────────────────────────


fn test_dl_multiple_load_resolve_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five load/resolve cycles all succeed."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    for i in range(5):
        # Load
        _ = _handle_event(w, app, hid)
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("dl_is_pending", args_ptr(app)), 1)
        assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 1)
        assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 0)
        # Resolve
        _resolve(w, app, String("Data ") + String(i))
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("dl_is_pending", args_ptr(app)), 0)
        assert_equal(w[].call_i32("dl_content_mounted", args_ptr(app)), 1)
        assert_equal(w[].call_i32("dl_skeleton_mounted", args_ptr(app)), 0)
    _destroy_dl(w, app, buf)


# ── Test: resolve with different data ────────────────────────────────────────


fn test_dl_resolve_with_different_data(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Each resolve shows the new data string."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))

    # First cycle
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    _resolve(w, app, String("Alpha"))
    _ = _flush(w, app, buf)
    var out1 = w[].alloc_string_struct()
    w[].call_void("dl_data_text", args_ptr_ptr(app, out1))
    assert_equal(w[].read_string_struct(out1), String("Alpha"))

    # Second cycle
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    _resolve(w, app, String("Beta"))
    _ = _flush(w, app, buf)
    var out2 = w[].alloc_string_struct()
    w[].call_void("dl_data_text", args_ptr_ptr(app, out2))
    assert_equal(w[].read_string_struct(out2), String("Beta"))

    _destroy_dl(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_dl_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush without any state change returns 0."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, 0)
    _destroy_dl(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_dl_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal lifecycle does not crash."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    _destroy_dl(w, app, buf)


# ── Test: destroy while pending ──────────────────────────────────────────────


fn test_dl_destroy_while_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy while in pending state does not crash."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("dl_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("dl_is_pending", args_ptr(app)), 1)
    _destroy_dl(w, app, buf)


# ── Test: scope IDs distinct ────────────────────────────────────────────────


fn test_dl_scope_ids_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """All three scope IDs (parent, content, skeleton) are distinct."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var parent = w[].call_i32("dl_parent_scope_id", args_ptr(app))
    var content = w[].call_i32("dl_content_scope_id", args_ptr(app))
    var skeleton = w[].call_i32("dl_skeleton_scope_id", args_ptr(app))
    assert_true(parent != content, msg="parent != content")
    assert_true(parent != skeleton, msg="parent != skeleton")
    assert_true(content != skeleton, msg="content != skeleton")
    _destroy_dl(w, app, buf)


# ── Test: scope count ────────────────────────────────────────────────────────


fn test_dl_scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Three scopes: root + content + skeleton."""
    var t = _create_dl(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("dl_scope_count", args_ptr(app))
    assert_equal(count, 3)
    _destroy_dl(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point — runs all tests via a shared WASM instance
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = _load()

    print("test_data_loader — DataLoaderApp suspense demo (Phase 33.2):")

    test_dl_init_creates_app(wp)
    print("  ✓ init creates app")

    test_dl_not_pending_initially(wp)
    print("  ✓ not pending initially")

    test_dl_data_text_initially_none(wp)
    print("  ✓ data text initially (none)")

    test_dl_content_mounted_after_rebuild(wp)
    print("  ✓ content mounted after rebuild")

    test_dl_skeleton_not_mounted_initially(wp)
    print("  ✓ skeleton not mounted initially")

    test_dl_load_sets_pending(wp)
    print("  ✓ load sets pending")

    test_dl_flush_after_load_hides_content(wp)
    print("  ✓ flush after load hides content")

    test_dl_flush_after_load_shows_skeleton(wp)
    print("  ✓ flush after load shows skeleton")

    test_dl_resolve_clears_pending(wp)
    print("  ✓ resolve clears pending")

    test_dl_resolve_stores_data(wp)
    print("  ✓ resolve stores data")

    test_dl_flush_after_resolve_shows_content(wp)
    print("  ✓ flush after resolve shows content")

    test_dl_flush_after_resolve_hides_skeleton(wp)
    print("  ✓ flush after resolve hides skeleton")

    test_dl_reload_cycle(wp)
    print("  ✓ reload cycle")

    test_dl_multiple_load_resolve_cycles(wp)
    print("  ✓ multiple load/resolve cycles")

    test_dl_resolve_with_different_data(wp)
    print("  ✓ resolve with different data")

    test_dl_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_dl_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_dl_destroy_while_pending(wp)
    print("  ✓ destroy while pending")

    test_dl_scope_ids_distinct(wp)
    print("  ✓ scope IDs distinct")

    test_dl_scope_count(wp)
    print("  ✓ scope count")

    print("  ✓ test_data_loader — data_loader: 20/20 passed")
