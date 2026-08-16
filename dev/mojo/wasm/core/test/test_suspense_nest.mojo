"""Phase 33.3 — SuspenseNestApp Mojo Tests.

Validates SuspenseNestApp via the sn_* WASM exports which exercise
nested suspense boundaries with independent load/resolve lifecycles:

  - init creates app with non-zero pointer
  - no pending initially (both boundaries)
  - all content mounted after rebuild (outer + inner)
  - no skeletons initially
  - inner load sets inner pending
  - inner load preserves outer (outer not pending)
  - flush after inner load shows inner skeleton, hides inner content
  - inner resolve clears inner pending
  - flush after inner resolve restores inner content with data
  - outer load sets outer pending
  - flush after outer load shows outer skeleton, hides outer content
  - outer resolve restores outer content + inner boundary
  - inner load then outer load (outer skeleton takes precedence)
  - outer resolve reveals inner pending (inner skeleton shown)
  - inner resolve after outer resolve (full resolution)
  - multiple inner load/resolve cycles (5 cycles)
  - multiple outer load/resolve cycles (5 cycles)
  - mixed load/resolve sequence
  - resolve with different data
  - destroy does not crash
  - destroy while pending
  - scope IDs all distinct
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


fn _create_sn(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a SuspenseNestApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("sn_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("sn_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _destroy_sn(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a SuspenseNestApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("sn_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("sn_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("sn_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _outer_resolve(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    data: String,
) raises:
    """Resolve the outer pending state with a data string."""
    var data_ptr = w[].write_string_struct(data)
    w[].call_void("sn_outer_resolve", args_ptr_ptr(app, data_ptr))


fn _inner_resolve(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    data: String,
) raises:
    """Resolve the inner pending state with a data string."""
    var data_ptr = w[].write_string_struct(data)
    w[].call_void("sn_inner_resolve", args_ptr_ptr(app, data_ptr))


fn _outer_load(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Press the outer load button and flush."""
    var hid = w[].call_i32("sn_outer_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)


fn _inner_load(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Press the inner load button and flush."""
    var hid = w[].call_i32("sn_inner_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    _ = _flush(w, app, buf)


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_sn_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("sn_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("sn_destroy", args_ptr(app))


# ── Test: no pending initially ───────────────────────────────────────────────


fn test_sn_no_pending_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Both boundaries are not pending after init + rebuild."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: all content mounted after rebuild ──────────────────────────────────


fn test_sn_all_content_mounted_after_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Outer + inner content are mounted after initial rebuild."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: no skeletons initially ─────────────────────────────────────────────


fn test_sn_no_skeletons_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Both skeletons are hidden after initial rebuild."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: inner load sets inner pending ──────────────────────────────────────


fn test_sn_inner_load_sets_inner_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner load button sets inner pending to true."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("sn_inner_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: inner load preserves outer ─────────────────────────────────────────


fn test_sn_inner_load_preserves_outer(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner load does not affect outer pending state."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _inner_load(w, app, buf)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: flush after inner load ─────────────────────────────────────────────


fn test_sn_flush_after_inner_load(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After inner load + flush: inner skeleton shown, inner content hidden,
    outer content still mounted."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _inner_load(w, app, buf)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: inner resolve clears inner pending ─────────────────────────────────


fn test_sn_inner_resolve_clears_inner_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner resolve clears inner pending state."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _inner_load(w, app, buf)
    _inner_resolve(w, app, String("Hello"))
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: flush after inner resolve ──────────────────────────────────────────


fn test_sn_flush_after_inner_resolve(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After inner load + flush + inner resolve + flush: inner content restored.
    """
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _inner_load(w, app, buf)
    _inner_resolve(w, app, String("InnerData"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 0)
    # Verify inner data stored
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("sn_inner_data", args_ptr_ptr(app, out_ptr))
    assert_equal(w[].read_string_struct(out_ptr), String("InnerData"))
    _destroy_sn(w, app, buf)


# ── Test: outer load sets outer pending ──────────────────────────────────────


fn test_sn_outer_load_sets_outer_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Outer load button sets outer pending to true."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    var hid = w[].call_i32("sn_outer_load_handler", args_ptr(app))
    _ = _handle_event(w, app, hid)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: flush after outer load ─────────────────────────────────────────────


fn test_sn_flush_after_outer_load(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After outer load + flush: outer skeleton shown, outer content hidden,
    inner boundary + children also hidden."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _outer_load(w, app, buf)
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: outer resolve restores outer content ───────────────────────────────


fn test_sn_outer_resolve_restores_outer_content(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After outer load + resolve + flush: outer content + inner boundary visible again.
    """
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _outer_load(w, app, buf)
    _outer_resolve(w, app, String("OuterData"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
    # Verify outer data stored
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("sn_outer_data", args_ptr_ptr(app, out_ptr))
    assert_equal(w[].read_string_struct(out_ptr), String("OuterData"))
    _destroy_sn(w, app, buf)


# ── Test: inner load then outer load ─────────────────────────────────────────


fn test_sn_inner_load_then_outer_load(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Inner load then outer load — outer skeleton takes visual precedence."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    # Inner load first
    _inner_load(w, app, buf)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
    # Now outer load
    _outer_load(w, app, buf)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 1)
    # Outer skeleton shown, everything else hidden
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: outer resolve reveals inner pending ────────────────────────────────


fn test_sn_outer_resolve_reveals_inner_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After inner load + outer load + outer resolve: inner still pending,
    inner skeleton shown."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    # Inner load, then outer load
    _inner_load(w, app, buf)
    _outer_load(w, app, buf)
    # Resolve outer only
    _outer_resolve(w, app, String("OuterOK"))
    _ = _flush(w, app, buf)
    # Outer content restored
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 0)
    # Inner still pending — inner skeleton should be shown
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: inner resolve after outer resolve ──────────────────────────────────


fn test_sn_inner_resolve_after_outer_resolve(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Full resolution: inner load → outer load → outer resolve → inner resolve.
    """
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _inner_load(w, app, buf)
    _outer_load(w, app, buf)
    _outer_resolve(w, app, String("OuterDone"))
    _ = _flush(w, app, buf)
    _inner_resolve(w, app, String("InnerDone"))
    _ = _flush(w, app, buf)
    # Everything resolved
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 0)
    _destroy_sn(w, app, buf)


# ── Test: multiple inner load/resolve cycles ─────────────────────────────────


fn test_sn_multiple_inner_load_resolve_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five inner load/resolve cycles all succeed."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    for i in range(5):
        # Inner load
        _inner_load(w, app, buf)
        assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
        assert_equal(
            w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 1
        )
        assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 0)
        # Inner resolve
        _inner_resolve(w, app, String("Inner ") + String(i))
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 0)
        assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
        assert_equal(
            w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 0
        )
    _destroy_sn(w, app, buf)


# ── Test: multiple outer load/resolve cycles ─────────────────────────────────


fn test_sn_multiple_outer_load_resolve_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Five outer load/resolve cycles all succeed."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    for i in range(5):
        # Outer load
        _outer_load(w, app, buf)
        assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 1)
        assert_equal(
            w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 1
        )
        assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 0)
        # Outer resolve
        _outer_resolve(w, app, String("Outer ") + String(i))
        _ = _flush(w, app, buf)
        assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 0)
        assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
        assert_equal(
            w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 0
        )
        assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: mixed load/resolve sequence ────────────────────────────────────────


fn test_sn_mixed_load_resolve_sequence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """inner→outer→outer_resolve→inner_resolve sequence."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]

    # Step 1: inner load
    _inner_load(w, app, buf)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 0)

    # Step 2: outer load (hides everything including inner skeleton)
    _outer_load(w, app, buf)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_outer_skeleton_mounted", args_ptr(app)), 1)

    # Step 3: outer resolve → reveals inner pending
    _outer_resolve(w, app, String("MixedOuter"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_outer_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 1)

    # Step 4: inner resolve → fully resolved
    _inner_resolve(w, app, String("MixedInner"))
    _ = _flush(w, app, buf)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 0)
    assert_equal(w[].call_i32("sn_inner_content_mounted", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_inner_skeleton_mounted", args_ptr(app)), 0)

    _destroy_sn(w, app, buf)


# ── Test: resolve with different data ────────────────────────────────────────


fn test_sn_resolve_with_different_data(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Each resolve shows new data text."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]

    # Inner cycle 1
    _inner_load(w, app, buf)
    _inner_resolve(w, app, String("Alpha"))
    _ = _flush(w, app, buf)
    var out1 = w[].alloc_string_struct()
    w[].call_void("sn_inner_data", args_ptr_ptr(app, out1))
    assert_equal(w[].read_string_struct(out1), String("Alpha"))

    # Inner cycle 2
    _inner_load(w, app, buf)
    _inner_resolve(w, app, String("Beta"))
    _ = _flush(w, app, buf)
    var out2 = w[].alloc_string_struct()
    w[].call_void("sn_inner_data", args_ptr_ptr(app, out2))
    assert_equal(w[].read_string_struct(out2), String("Beta"))

    # Outer cycle 1
    _outer_load(w, app, buf)
    _outer_resolve(w, app, String("Gamma"))
    _ = _flush(w, app, buf)
    var out3 = w[].alloc_string_struct()
    w[].call_void("sn_outer_data", args_ptr_ptr(app, out3))
    assert_equal(w[].read_string_struct(out3), String("Gamma"))

    # Outer cycle 2
    _outer_load(w, app, buf)
    _outer_resolve(w, app, String("Delta"))
    _ = _flush(w, app, buf)
    var out4 = w[].alloc_string_struct()
    w[].call_void("sn_outer_data", args_ptr_ptr(app, out4))
    assert_equal(w[].read_string_struct(out4), String("Delta"))

    _destroy_sn(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_sn_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal lifecycle does not crash."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _destroy_sn(w, app, buf)


# ── Test: destroy while pending ──────────────────────────────────────────────


fn test_sn_destroy_while_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy while both boundaries are pending does not crash."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    _inner_load(w, app, buf)
    _outer_load(w, app, buf)
    assert_equal(w[].call_i32("sn_is_outer_pending", args_ptr(app)), 1)
    assert_equal(w[].call_i32("sn_is_inner_pending", args_ptr(app)), 1)
    _destroy_sn(w, app, buf)


# ── Test: scope IDs all distinct ─────────────────────────────────────────────


fn test_sn_scope_ids_all_distinct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """All five scope IDs (outer, inner boundary, inner content, inner skeleton,
    outer skeleton) are distinct."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    var outer = w[].call_i32("sn_outer_scope_id", args_ptr(app))
    var ib = w[].call_i32("sn_inner_boundary_scope_id", args_ptr(app))
    var ic = w[].call_i32("sn_inner_content_scope_id", args_ptr(app))
    var is_ = w[].call_i32("sn_inner_skeleton_scope_id", args_ptr(app))
    var os = w[].call_i32("sn_outer_skeleton_scope_id", args_ptr(app))

    # All pairs distinct
    assert_true(outer != ib, msg="outer != inner boundary")
    assert_true(outer != ic, msg="outer != inner content")
    assert_true(outer != is_, msg="outer != inner skeleton")
    assert_true(outer != os, msg="outer != outer skeleton")
    assert_true(ib != ic, msg="inner boundary != inner content")
    assert_true(ib != is_, msg="inner boundary != inner skeleton")
    assert_true(ib != os, msg="inner boundary != outer skeleton")
    assert_true(ic != is_, msg="inner content != inner skeleton")
    assert_true(ic != os, msg="inner content != outer skeleton")
    assert_true(is_ != os, msg="inner skeleton != outer skeleton")

    # Scope count = 5
    var count = w[].call_i32("sn_scope_count", args_ptr(app))
    assert_equal(count, 5)

    _destroy_sn(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_sn_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush without any state change returns 0."""
    var t = _create_sn(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, 0)
    _destroy_sn(w, app, buf)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point — runs all tests via a shared WASM instance
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var wp = _load()

    print(
        "test_suspense_nest — SuspenseNestApp nested suspense demo (Phase"
        " 33.3):"
    )

    test_sn_init_creates_app(wp)
    print("  ✓ init creates app")

    test_sn_no_pending_initially(wp)
    print("  ✓ no pending initially")

    test_sn_all_content_mounted_after_rebuild(wp)
    print("  ✓ all content mounted after rebuild")

    test_sn_no_skeletons_initially(wp)
    print("  ✓ no skeletons initially")

    test_sn_inner_load_sets_inner_pending(wp)
    print("  ✓ inner load sets inner pending")

    test_sn_inner_load_preserves_outer(wp)
    print("  ✓ inner load preserves outer")

    test_sn_flush_after_inner_load(wp)
    print("  ✓ flush after inner load")

    test_sn_inner_resolve_clears_inner_pending(wp)
    print("  ✓ inner resolve clears inner pending")

    test_sn_flush_after_inner_resolve(wp)
    print("  ✓ flush after inner resolve")

    test_sn_outer_load_sets_outer_pending(wp)
    print("  ✓ outer load sets outer pending")

    test_sn_flush_after_outer_load(wp)
    print("  ✓ flush after outer load")

    test_sn_outer_resolve_restores_outer_content(wp)
    print("  ✓ outer resolve restores outer content")

    test_sn_inner_load_then_outer_load(wp)
    print("  ✓ inner load then outer load")

    test_sn_outer_resolve_reveals_inner_pending(wp)
    print("  ✓ outer resolve reveals inner pending")

    test_sn_inner_resolve_after_outer_resolve(wp)
    print("  ✓ inner resolve after outer resolve")

    test_sn_multiple_inner_load_resolve_cycles(wp)
    print("  ✓ multiple inner load/resolve cycles")

    test_sn_multiple_outer_load_resolve_cycles(wp)
    print("  ✓ multiple outer load/resolve cycles")

    test_sn_mixed_load_resolve_sequence(wp)
    print("  ✓ mixed load/resolve sequence")

    test_sn_resolve_with_different_data(wp)
    print("  ✓ resolve with different data")

    test_sn_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_sn_destroy_while_pending(wp)
    print("  ✓ destroy while pending")

    test_sn_scope_ids_all_distinct(wp)
    print("  ✓ scope IDs all distinct")

    test_sn_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    print("\nAll 22 SuspenseNestApp tests passed.")
