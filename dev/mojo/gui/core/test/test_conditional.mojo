"""Phase 28 — Conditional Rendering Mojo Tests.

Validates ConditionalSlot and flush_conditional/flush_conditional_empty
at the WASM level via the counter app's show/hide detail feature:

  - counter_toggle_handler returns a valid handler ID
  - counter_show_detail starts as 0 (hidden)
  - toggle on → show_detail=1, cond_mounted=1
  - toggle off → show_detail=0, cond_mounted=0
  - toggle on→off→on cycle preserves correct state
  - increment while detail visible → flush produces mutations
  - increment while detail hidden → detail shows updated content on toggle
  - decrement with detail visible works (negative values)
  - rapid toggle cycles don't corrupt state
  - detail template is registered (counter_detail_tmpl_id)
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


# ── Counter conditional rendering ────────────────────────────────────────────


def test_toggle_handler_valid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Counter_toggle_handler returns a valid non-negative handler ID."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))
    assert_true(toggle_h >= 0, "toggle handler ID is non-negative")

    var incr_h = w[].call_i32("counter_incr_handler", args_ptr(app))
    var decr_h = w[].call_i32("counter_decr_handler", args_ptr(app))
    assert_true(toggle_h != incr_h, "toggle handler differs from incr")
    assert_true(toggle_h != decr_h, "toggle handler differs from decr")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_show_detail_starts_false(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Show_detail starts as 0 (false) and cond_mounted starts as 0."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))

    var show = w[].call_i32("counter_show_detail", args_ptr(app))
    assert_equal(show, 0, "show_detail is 0 initially")

    var mounted = w[].call_i32("counter_cond_mounted", args_ptr(app))
    assert_equal(mounted, 0, "cond_mounted is 0 initially")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_toggle_on_sets_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Toggle detail ON → show_detail=1, cond_mounted=1, flush produces mutations.
    """
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))

    # Dispatch toggle event
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))

    # show_detail should be 1 after dispatch (signal updated before flush)
    var show = w[].call_i32("counter_show_detail", args_ptr(app))
    assert_equal(show, 1, "show_detail is 1 after toggle dispatch")

    # Flush — should produce mutations (create detail VNode, ReplaceWith)
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(flush_len > 0, "flush produces mutations after toggle on")

    var mounted = w[].call_i32("counter_cond_mounted", args_ptr(app))
    assert_equal(mounted, 1, "cond_mounted is 1 after flush")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_toggle_off_clears_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Toggle ON then OFF → show_detail=0, cond_mounted=0."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))

    # Toggle ON
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    var mounted_on = w[].call_i32("counter_cond_mounted", args_ptr(app))
    assert_equal(mounted_on, 1, "mounted after toggle on")

    # Toggle OFF
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(flush_len > 0, "flush produces mutations after toggle off")

    var show = w[].call_i32("counter_show_detail", args_ptr(app))
    assert_equal(show, 0, "show_detail is 0 after toggle off")

    var mounted_off = w[].call_i32("counter_cond_mounted", args_ptr(app))
    assert_equal(mounted_off, 0, "cond_mounted is 0 after toggle off")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_toggle_on_off_on_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Toggle ON → OFF → ON restores mounted state correctly."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))

    # Toggle ON
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "mounted after first ON",
    )

    # Toggle OFF
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        0,
        "unmounted after OFF",
    )

    # Toggle ON again
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(flush_len > 0, "flush produces mutations on re-toggle ON")
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "re-mounted after second ON",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_increment_with_detail_visible(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Incrementing while detail is visible → flush produces mutations (diff).
    """
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))
    var incr_h = w[].call_i32("counter_incr_handler", args_ptr(app))

    # Toggle ON
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    # Increment (count 0 → 1)
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr_h, 0))
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(
        flush_len > 0,
        "flush produces mutations after increment with detail visible",
    )

    # Detail should still be mounted
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "detail still mounted after increment",
    )

    # Count should be 1
    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 1, "count is 1 after increment")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_increment_hidden_then_show(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Increment while hidden → toggle ON shows updated content."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))
    var incr_h = w[].call_i32("counter_incr_handler", args_ptr(app))

    # Increment 3 times while detail is hidden
    for _ in range(3):
        _ = w[].call_i32(
            "counter_handle_event", args_ptr_i32_i32(app, incr_h, 0)
        )
        _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 3, "count is 3 after 3 increments")

    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        0,
        "detail still hidden",
    )

    # Now toggle ON — detail should mount with count=3 content
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(flush_len > 0, "flush produces mutations on toggle on")
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "detail mounted after toggle on",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_decrement_with_detail_visible(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Decrement with detail visible works (negative values)."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))
    var decr_h = w[].call_i32("counter_decr_handler", args_ptr(app))

    # Toggle ON
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    # Decrement (count 0 → -1)
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, decr_h, 0))
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(flush_len > 0, "flush after decrement with detail visible")

    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, -1, "count is -1 after decrement")

    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "detail still mounted after decrement",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_rapid_toggle_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """10 rapid ON/OFF toggle cycles don't corrupt state."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))

    for i in range(10):
        # Toggle ON
        _ = w[].call_i32(
            "counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0)
        )
        var len_on = w[].call_i32(
            "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
        )
        assert_true(len_on > 0, "flush on toggle ON cycle " + String(i))
        assert_equal(
            w[].call_i32("counter_cond_mounted", args_ptr(app)),
            1,
            "mounted after ON in cycle " + String(i),
        )

        # Toggle OFF
        _ = w[].call_i32(
            "counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0)
        )
        var len_off = w[].call_i32(
            "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
        )
        assert_true(len_off > 0, "flush on toggle OFF cycle " + String(i))
        assert_equal(
            w[].call_i32("counter_cond_mounted", args_ptr(app)),
            0,
            "unmounted after OFF in cycle " + String(i),
        )

    # Final state: detail hidden
    assert_equal(
        w[].call_i32("counter_show_detail", args_ptr(app)),
        0,
        "show_detail is 0 after 10 cycles",
    )
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        0,
        "cond_mounted is 0 after 10 cycles",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_detail_template_registered(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """The detail template is registered with a valid ID."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))

    var detail_tmpl = w[].call_i32("counter_detail_tmpl_id", args_ptr(app))
    assert_true(detail_tmpl >= 0, "detail template ID is non-negative")

    var main_tmpl = w[].call_i32("counter_tmpl_id", args_ptr(app))
    assert_true(
        detail_tmpl != main_tmpl,
        "detail template differs from main template",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_mixed_increment_toggle_sequence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Mixed increment + toggle sequence: toggle ON, incr 3x, toggle OFF, incr 2x, toggle ON.
    """
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))
    var incr_h = w[].call_i32("counter_incr_handler", args_ptr(app))

    # Toggle ON
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "mounted after toggle on",
    )

    # Increment 3 times with detail visible
    for _ in range(3):
        _ = w[].call_i32(
            "counter_handle_event", args_ptr_i32_i32(app, incr_h, 0)
        )
        _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("counter_count_value", args_ptr(app)),
        3,
        "count is 3 after 3 increments",
    )
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "detail still mounted after increments",
    )

    # Toggle OFF
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        0,
        "unmounted after toggle off",
    )

    # Increment 2 more times while hidden
    for _ in range(2):
        _ = w[].call_i32(
            "counter_handle_event", args_ptr_i32_i32(app, incr_h, 0)
        )
        _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("counter_count_value", args_ptr(app)),
        5,
        "count is 5 after 2 more increments",
    )

    # Toggle ON again — should mount with count=5 content
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 8192)
    )
    assert_true(flush_len > 0, "flush produces mutations on re-toggle")
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "re-mounted with count=5",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_destroy_with_detail_mounted(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroying the app while detail is mounted doesn't crash."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var toggle_h = w[].call_i32("counter_toggle_handler", args_ptr(app))

    # Toggle ON
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, toggle_h, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app)),
        1,
        "detail mounted before destroy",
    )

    # Destroy — should not crash even with detail mounted
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_destroy_recreate_with_conditional(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy with detail mounted → recreate → detail starts hidden."""
    # First instance: mount detail
    var app1 = Int(w[].call_i64("counter_init", no_args()))
    var buf1 = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app1, buf1, 8192))

    var toggle1 = w[].call_i32("counter_toggle_handler", args_ptr(app1))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app1, toggle1, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app1, buf1, 8192))

    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app1)),
        1,
        "first instance: detail mounted",
    )

    # Destroy first instance
    w[].call_void("mutation_buf_free", args_ptr(buf1))
    w[].call_void("counter_destroy", args_ptr(app1))

    # Second instance: should start with detail hidden
    var app2 = Int(w[].call_i64("counter_init", no_args()))
    var buf2 = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app2, buf2, 8192))

    assert_equal(
        w[].call_i32("counter_show_detail", args_ptr(app2)),
        0,
        "second instance: show_detail is 0",
    )
    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app2)),
        0,
        "second instance: detail not mounted",
    )

    # Toggle ON in second instance should work
    var toggle2 = w[].call_i32("counter_toggle_handler", args_ptr(app2))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app2, toggle2, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app2, buf2, 8192))

    assert_equal(
        w[].call_i32("counter_cond_mounted", args_ptr(app2)),
        1,
        "second instance: detail mounted after toggle",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf2))
    w[].call_void("counter_destroy", args_ptr(app2))


# ── Todo empty state tests (P28.4) ───────────────────────────────────────────


def test_todo_empty_msg_on_initial_mount(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Empty state message is mounted on initial mount (0 items)."""
    var app = Int(w[].call_i64("todo_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    var count = w[].call_i32("todo_item_count", args_ptr(app))
    assert_equal(count, 0, "0 items initially")

    var mounted = w[].call_i32("todo_empty_msg_mounted", args_ptr(app))
    assert_equal(mounted, 1, "empty msg mounted on initial mount")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("todo_destroy", args_ptr(app))


def test_todo_empty_msg_hidden_after_add(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Empty state message hides after adding an item."""
    var app = Int(w[].call_i64("todo_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        1,
        "msg mounted before add",
    )

    # Add an item
    var str_ptr = w[].write_string_struct("Hello")
    w[].call_void("todo_add_item", args_ptr_ptr(app, str_ptr))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("todo_item_count", args_ptr(app)),
        1,
        "1 item after add",
    )
    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        0,
        "msg hidden after add",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("todo_destroy", args_ptr(app))


def test_todo_empty_msg_returns_after_remove_all(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Empty state message returns after removing all items."""
    var app = Int(w[].call_i64("todo_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    # Add two items
    var str1 = w[].write_string_struct("A")
    w[].call_void("todo_add_item", args_ptr_ptr(app, str1))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))

    var str2 = w[].write_string_struct("B")
    w[].call_void("todo_add_item", args_ptr_ptr(app, str2))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("todo_item_count", args_ptr(app)),
        2,
        "2 items after adds",
    )
    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        0,
        "msg hidden with 2 items",
    )

    # Remove both items
    var id1 = w[].call_i32("todo_item_id_at", args_ptr_i32(app, 0))
    w[].call_void("todo_remove_item", args_ptr_i32(app, id1))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))

    var id2 = w[].call_i32("todo_item_id_at", args_ptr_i32(app, 0))
    w[].call_void("todo_remove_item", args_ptr_i32(app, id2))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("todo_item_count", args_ptr(app)),
        0,
        "0 items after removing all",
    )
    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        1,
        "msg re-mounted after removing all",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("todo_destroy", args_ptr(app))


def test_todo_empty_msg_add_remove_add_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Add → remove all → add again: message toggles correctly."""
    var app = Int(w[].call_i64("todo_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    # Add → msg hides
    var str1 = w[].write_string_struct("X")
    w[].call_void("todo_add_item", args_ptr_ptr(app, str1))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        0,
        "msg hidden after first add",
    )

    # Remove → msg shows
    var id1 = w[].call_i32("todo_item_id_at", args_ptr_i32(app, 0))
    w[].call_void("todo_remove_item", args_ptr_i32(app, id1))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        1,
        "msg shown after remove",
    )

    # Add again → msg hides again
    var str2 = w[].write_string_struct("Y")
    w[].call_void("todo_add_item", args_ptr_ptr(app, str2))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 8192))
    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        0,
        "msg hidden after second add",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("todo_destroy", args_ptr(app))


def test_todo_destroy_with_empty_msg_mounted(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroying the todo app with empty msg mounted doesn't crash."""
    var app = Int(w[].call_i64("todo_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app, buf, 8192))

    assert_equal(
        w[].call_i32("todo_empty_msg_mounted", args_ptr(app)),
        1,
        "msg mounted before destroy",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("todo_destroy", args_ptr(app))


# ── Main ─────────────────────────────────────────────────────────────────────


fn main() raises:
    var ptr = get_instance()

    print("test_conditional — conditional rendering (Phase 28):")

    # Counter conditional detail (P28.3)
    test_toggle_handler_valid(ptr)
    print("  ✓ toggle_handler_valid")

    test_show_detail_starts_false(ptr)
    print("  ✓ show_detail_starts_false")

    test_toggle_on_sets_state(ptr)
    print("  ✓ toggle_on_sets_state")

    test_toggle_off_clears_state(ptr)
    print("  ✓ toggle_off_clears_state")

    test_toggle_on_off_on_cycle(ptr)
    print("  ✓ toggle_on_off_on_cycle")

    test_increment_with_detail_visible(ptr)
    print("  ✓ increment_with_detail_visible")

    test_increment_hidden_then_show(ptr)
    print("  ✓ increment_hidden_then_show")

    test_decrement_with_detail_visible(ptr)
    print("  ✓ decrement_with_detail_visible")

    test_rapid_toggle_cycles(ptr)
    print("  ✓ rapid_toggle_cycles")

    test_detail_template_registered(ptr)
    print("  ✓ detail_template_registered")

    test_mixed_increment_toggle_sequence(ptr)
    print("  ✓ mixed_increment_toggle_sequence")

    test_destroy_with_detail_mounted(ptr)
    print("  ✓ destroy_with_detail_mounted")

    test_destroy_recreate_with_conditional(ptr)
    print("  ✓ destroy_recreate_with_conditional")

    # Todo empty state (P28.4)
    test_todo_empty_msg_on_initial_mount(ptr)
    print("  ✓ todo_empty_msg_on_initial_mount")

    test_todo_empty_msg_hidden_after_add(ptr)
    print("  ✓ todo_empty_msg_hidden_after_add")

    test_todo_empty_msg_returns_after_remove_all(ptr)
    print("  ✓ todo_empty_msg_returns_after_remove_all")

    test_todo_empty_msg_add_remove_add_cycle(ptr)
    print("  ✓ todo_empty_msg_add_remove_add_cycle")

    test_todo_destroy_with_empty_msg_mounted(ptr)
    print("  ✓ todo_destroy_with_empty_msg_mounted")

    print("test_conditional: 18/18 passed")
