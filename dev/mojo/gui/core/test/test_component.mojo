from memory import UnsafePointer

# Tests for Phase 10.4 — AppShell (component abstraction).
#
# Validates:
#   - AppShell create/destroy lifecycle
#   - is_alive tracking
#   - Scope creation (root and child) via shell
#   - Signal creation, read, peek, write via shell
#   - begin_render / end_render scope lifecycle
#   - has_dirty / collect_dirty / next_dirty scheduler integration
#   - dispatch_event via shell
#   - mount() produces valid mutations
#   - diff() produces correct mutations on state change
#   - Pointer accessors (rt_ptr, store_ptr, eid_ptr)
#   - Double destroy safety
#   - Shell memo helpers (M13.5)
#   - Counter memo demo (M13.6)

from testing import assert_equal, assert_true, assert_false
from wasm_harness import (
    WasmInstance,
    no_args,
    args_i32,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_ptr,
    args_ptr_i32_ptr,
    args_ptr_ptr_i32,
    args_ptr_ptr_i32_i32,
    args_ptr_ptr_i32_i32_i32,
    args_ptr_i32_i32_i32_ptr,
)


fn _load() raises -> WasmInstance:
    return WasmInstance("build/out.wasm")


# ── AppShell lifecycle ───────────────────────────────────────────────────────


def test_shell_create_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    assert_true(shell != 0, "shell pointer should be non-zero")
    assert_equal(w[].call_i32("shell_is_alive", args_ptr(shell)), 1)
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_is_alive_after_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    assert_equal(w[].call_i32("shell_is_alive", args_ptr(shell)), 1)
    w[].call_void("shell_destroy", args_ptr(shell))


# ── Pointer accessors ────────────────────────────────────────────────────────


def test_shell_rt_ptr_non_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    assert_true(rt != 0, "runtime pointer should be non-zero")
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_store_ptr_non_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var store = Int(w[].call_i64("shell_store_ptr", args_ptr(shell)))
    assert_true(store != 0, "store pointer should be non-zero")
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_eid_ptr_non_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var eid = Int(w[].call_i64("shell_eid_ptr", args_ptr(shell)))
    assert_true(eid != 0, "eid_alloc pointer should be non-zero")
    w[].call_void("shell_destroy", args_ptr(shell))


# ── Scope creation ───────────────────────────────────────────────────────────


def test_shell_create_root_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var s0 = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    assert_true(s0 >= 0, "root scope id should be non-negative")

    # Verify the scope exists in the underlying runtime
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    assert_equal(w[].call_i32("scope_contains", args_ptr_i32(rt, s0)), 1)
    assert_equal(w[].call_i32("scope_height", args_ptr_i32(rt, s0)), 0)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_create_child_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var parent = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var child = w[].call_i32(
        "shell_create_child_scope", args_ptr_i32(shell, parent)
    )
    assert_true(child != parent, "child id should differ from parent")

    # Verify parent/child relationship in the runtime
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    assert_equal(w[].call_i32("scope_height", args_ptr_i32(rt, child)), 1)
    assert_equal(w[].call_i32("scope_parent", args_ptr_i32(rt, child)), parent)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_create_multiple_root_scopes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var s0 = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var s1 = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var s2 = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    assert_true(s0 != s1, "scopes should have unique ids")
    assert_true(s1 != s2, "scopes should have unique ids")
    assert_true(s0 != s2, "scopes should have unique ids")
    w[].call_void("shell_destroy", args_ptr(shell))


# ── Signal operations ────────────────────────────────────────────────────────


def test_shell_create_signal_i32(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var key = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 42))
    assert_true(key >= 0, "signal key should be non-negative")

    # Peek should return the initial value
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, key)), 42
    )
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_write_and_peek_signal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var key = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, key, 99))
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, key)), 99
    )

    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, key, -7))
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, key)), -7
    )

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_read_signal_with_context(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """read_signal subscribes the current scope to the signal."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 10))

    # Begin render to activate the scope as reactive context
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    var val = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    assert_equal(val, 10)
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Writing should make the scope dirty (subscribed via read)
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 20))
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 1)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_peek_does_not_subscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """peek_signal does NOT subscribe the scope."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 5))

    # Begin render but only peek (not read)
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    var val = w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig))
    assert_equal(val, 5)
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Writing should NOT make the scope dirty (not subscribed)
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 99))
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 0)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_multiple_signals(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var sig_a = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 1))
    var sig_b = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 2))
    var sig_c = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 3))

    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig_a)), 1
    )
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig_b)), 2
    )
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig_c)), 3
    )

    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig_b, 200))
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig_b)), 200
    )
    # Others unchanged
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig_a)), 1
    )
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig_c)), 3
    )

    w[].call_void("shell_destroy", args_ptr(shell))


# ── Render lifecycle ─────────────────────────────────────────────────────────


def test_shell_begin_end_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    # begin_render returns previous scope (-1 for first render)
    var prev = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    assert_equal(prev, -1, "no previous scope on first render")

    w[].call_void("shell_end_render", args_ptr_i32(shell, prev))
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_nested_render(w: UnsafePointer[WasmInstance, MutExternalOrigin]):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var parent_scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var child_scope = w[].call_i32(
        "shell_create_child_scope", args_ptr_i32(shell, parent_scope)
    )

    # Render parent
    var prev1 = w[].call_i32(
        "shell_begin_render", args_ptr_i32(shell, parent_scope)
    )
    assert_equal(prev1, -1)

    # Nested render child
    var prev2 = w[].call_i32(
        "shell_begin_render", args_ptr_i32(shell, child_scope)
    )

    # End child render
    w[].call_void("shell_end_render", args_ptr_i32(shell, prev2))
    # End parent render
    w[].call_void("shell_end_render", args_ptr_i32(shell, prev1))

    w[].call_void("shell_destroy", args_ptr(shell))


# ── Dirty / Scheduler integration ───────────────────────────────────────────


def test_shell_initially_not_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 0)
    assert_equal(w[].call_i32("shell_scheduler_empty", args_ptr(shell)), 1)
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_dirty_after_signal_write(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    # Subscribe scope to signal
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Write → dirty
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 1))
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 1)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_collect_and_drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """collect_dirty + next_dirty yields dirty scopes in order."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    # Subscribe and write
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 42))

    # Collect dirty into scheduler
    w[].call_void("shell_collect_dirty", args_ptr(shell))
    assert_equal(w[].call_i32("shell_scheduler_empty", args_ptr(shell)), 0)

    # Runtime dirty queue should be drained
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 0)

    # Drain from scheduler
    var dirty_scope = w[].call_i32("shell_next_dirty", args_ptr(shell))
    assert_equal(dirty_scope, scope)
    assert_equal(w[].call_i32("shell_scheduler_empty", args_ptr(shell)), 1)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_dirty_height_ordering(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Scheduler yields parent before child when both are dirty."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var parent = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var child = w[].call_i32(
        "shell_create_child_scope", args_ptr_i32(shell, parent)
    )
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    # Subscribe both scopes to the same signal
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, parent))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, child))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Write → both dirty
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 99))

    # Collect and verify ordering
    w[].call_void("shell_collect_dirty", args_ptr(shell))
    var first = w[].call_i32("shell_next_dirty", args_ptr(shell))
    assert_equal(first, parent, "parent (height 0) should come first")
    var second = w[].call_i32("shell_next_dirty", args_ptr(shell))
    assert_equal(second, child, "child (height 1) should come second")

    w[].call_void("shell_destroy", args_ptr(shell))


# ── Event dispatch ───────────────────────────────────────────────────────────


def test_shell_dispatch_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """dispatch_event routes to the runtime's handler registry."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    # Register a handler that adds 5 to the signal
    var click_str = w[].write_string_struct("click")
    var handler = w[].call_i32(
        "handler_register_signal_add",
        args_ptr_i32_i32_i32_ptr(rt, scope, sig, 5, click_str),
    )

    # Subscribe scope to signal for dirty tracking
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Dispatch
    var executed = w[].call_i32(
        "shell_dispatch_event", args_ptr_i32_i32(shell, handler, 0)
    )
    assert_equal(executed, 1, "handler should execute")

    # Signal should be updated
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell, sig)), 5
    )

    # Scope should be dirty
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 1)

    w[].call_void("shell_destroy", args_ptr(shell))


# ── Mount ────────────────────────────────────────────────────────────────────


def test_shell_mount_text_vnode(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """shell_mount produces valid mutation bytes for a text VNode."""
    var shell = Int(w[].call_i64("shell_create", no_args()))

    # Create a text VNode in the store
    var store = Int(w[].call_i64("shell_store_ptr", args_ptr(shell)))
    var text_ptr = w[].write_string_struct("hello world")
    var vn = w[].call_i32("vnode_push_text", args_ptr_ptr(store, text_ptr))

    # Allocate mutation buffer
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))

    # Mount
    var byte_len = w[].call_i32(
        "shell_mount", args_ptr_ptr_i32_i32(shell, buf, 4096, vn)
    )
    assert_true(byte_len > 0, "mount should produce mutations")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_mount_template_ref(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """shell_mount produces mutations for a TemplateRef VNode."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    var store = Int(w[].call_i64("shell_store_ptr", args_ptr(shell)))

    # Build and register a simple template: div > text("hi")
    var tmpl_name_ptr = w[].write_string_struct("test-tmpl")
    var builder = Int(
        w[].call_i64("tmpl_builder_create", args_ptr(tmpl_name_ptr))
    )
    var div_idx = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(builder, 0, -1)
    )
    var hi_ptr = w[].write_string_struct("hi")
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(builder, hi_ptr, div_idx),
    )
    var tmpl_id = w[].call_i32(
        "tmpl_builder_register", args_ptr_ptr(rt, builder)
    )
    w[].call_void("tmpl_builder_destroy", args_ptr(builder))

    # Create a TemplateRef VNode
    var vn = w[].call_i32(
        "vnode_push_template_ref", args_ptr_i32(store, tmpl_id)
    )

    # Mount
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    var byte_len = w[].call_i32(
        "shell_mount", args_ptr_ptr_i32_i32(shell, buf, 4096, vn)
    )
    assert_true(byte_len > 0, "mount should produce mutations")

    # VNode should be mounted
    assert_equal(w[].call_i32("vnode_is_mounted", args_ptr_i32(store, vn)), 1)

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("shell_destroy", args_ptr(shell))


# ── Diff ─────────────────────────────────────────────────────────────────────


def test_shell_diff_same_text_zero_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Diffing identical text VNodes produces 0 mutation bytes (just End)."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var store = Int(w[].call_i64("shell_store_ptr", args_ptr(shell)))

    # Create old text VNode and mount it
    var same_ptr1 = w[].write_string_struct("same")
    var old_vn = w[].call_i32("vnode_push_text", args_ptr_ptr(store, same_ptr1))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32(
        "shell_mount", args_ptr_ptr_i32_i32(shell, buf, 4096, old_vn)
    )

    # Create new text VNode with same content
    var same_ptr2 = w[].write_string_struct("same")
    var new_vn = w[].call_i32("vnode_push_text", args_ptr_ptr(store, same_ptr2))

    # Diff
    var diff_len = w[].call_i32(
        "shell_diff",
        args_ptr_ptr_i32_i32_i32(shell, buf, 4096, old_vn, new_vn),
    )
    # Only the End sentinel (1 byte) should be written
    assert_equal(diff_len, 1, "same text → only End sentinel")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_diff_text_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Diffing different text VNodes produces SetText mutations."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var store = Int(w[].call_i64("shell_store_ptr", args_ptr(shell)))

    # Create and mount old text VNode
    var before_ptr = w[].write_string_struct("before")
    var old_vn = w[].call_i32(
        "vnode_push_text", args_ptr_ptr(store, before_ptr)
    )
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32(
        "shell_mount", args_ptr_ptr_i32_i32(shell, buf, 4096, old_vn)
    )

    # Create new text VNode with different content
    var after_ptr = w[].write_string_struct("after")
    var new_vn = w[].call_i32("vnode_push_text", args_ptr_ptr(store, after_ptr))

    # Diff
    var diff_len = w[].call_i32(
        "shell_diff",
        args_ptr_ptr_i32_i32_i32(shell, buf, 4096, old_vn, new_vn),
    )
    # Should produce SetText + End = more than 1 byte
    assert_true(diff_len > 1, "text change should produce SetText mutation")

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("shell_destroy", args_ptr(shell))


# ── Full mount → update cycle ────────────────────────────────────────────────


def test_shell_full_mount_update_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """End-to-end: create shell, mount, write signal, collect dirty, diff."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    var store = Int(w[].call_i64("shell_store_ptr", args_ptr(shell)))

    # 1. Create scope and signal
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    # 2. Register template: div > dynamic_text[0]
    var tmpl_name_ptr = w[].write_string_struct("cycle-tmpl")
    var builder = Int(
        w[].call_i64("tmpl_builder_create", args_ptr(tmpl_name_ptr))
    )
    var div_idx = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(builder, 0, -1)
    )
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text",
        args_ptr_i32_i32(builder, 0, div_idx),
    )
    var tmpl_id = w[].call_i32(
        "tmpl_builder_register", args_ptr_ptr(rt, builder)
    )
    w[].call_void("tmpl_builder_destroy", args_ptr(builder))

    # 3. Subscribe scope
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # 4. Build initial VNode and mount
    var v0 = w[].call_i32(
        "vnode_push_template_ref", args_ptr_i32(store, tmpl_id)
    )
    var count0_ptr = w[].write_string_struct("Count: 0")
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(store, v0, count0_ptr),
    )

    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    var mount_len = w[].call_i32(
        "shell_mount", args_ptr_ptr_i32_i32(shell, buf, 8192, v0)
    )
    assert_true(mount_len > 0, "mount should produce mutations")

    # 5. Write signal → scope dirty
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 1))
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 1)

    # 6. Collect dirty into scheduler
    w[].call_void("shell_collect_dirty", args_ptr(shell))
    assert_equal(w[].call_i32("shell_scheduler_empty", args_ptr(shell)), 0)

    # 7. Drain scheduler
    var dirty = w[].call_i32("shell_next_dirty", args_ptr(shell))
    assert_equal(dirty, scope)

    # 8. Build new VNode with updated text
    var v1 = w[].call_i32(
        "vnode_push_template_ref", args_ptr_i32(store, tmpl_id)
    )
    var count1_ptr = w[].write_string_struct("Count: 1")
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(store, v1, count1_ptr),
    )

    # 9. Diff old → new
    var diff_len = w[].call_i32(
        "shell_diff", args_ptr_ptr_i32_i32_i32(shell, buf, 8192, v0, v1)
    )
    assert_true(diff_len > 1, "diff should produce SetText mutation")

    # 10. No more dirty
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 0)
    assert_equal(w[].call_i32("shell_scheduler_empty", args_ptr(shell)), 1)

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("shell_destroy", args_ptr(shell))


# ── Subsystem isolation ──────────────────────────────────────────────────────


def test_shell_independent_instances(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Two AppShells are fully independent (no shared state)."""
    var shell_a = Int(w[].call_i64("shell_create", no_args()))
    var shell_b = Int(w[].call_i64("shell_create", no_args()))

    # Create signals in each
    var sig_a = w[].call_i32(
        "shell_create_signal_i32", args_ptr_i32(shell_a, 100)
    )
    var sig_b = w[].call_i32(
        "shell_create_signal_i32", args_ptr_i32(shell_b, 200)
    )

    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell_a, sig_a)), 100
    )
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell_b, sig_b)), 200
    )

    # Writing to one does not affect the other
    w[].call_void(
        "shell_write_signal_i32", args_ptr_i32_i32(shell_a, sig_a, 999)
    )
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell_a, sig_a)), 999
    )
    assert_equal(
        w[].call_i32("shell_peek_signal_i32", args_ptr_i32(shell_b, sig_b)), 200
    )

    w[].call_void("shell_destroy", args_ptr(shell_a))
    w[].call_void("shell_destroy", args_ptr(shell_b))


# ── Shell memo helpers (M13.5) ───────────────────────────────────────────────


def test_shell_memo_create_returns_valid_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """shell_memo_create_i32 returns a non-negative memo ID."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var memo = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 42)
    )
    assert_true(memo >= 0, "memo ID should be non-negative")
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_memo_initial_value_readable(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Memo's initial cached value is readable via shell helper."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var memo = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 77)
    )
    var val = w[].call_i32("shell_memo_read_i32", args_ptr_i32(shell, memo))
    assert_equal(val, 77)
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_memo_starts_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Newly created memo starts dirty (needs first computation)."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var memo = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 0)
    )
    assert_equal(
        w[].call_i32("shell_memo_is_dirty", args_ptr_i32(shell, memo)), 1
    )
    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_memo_compute_clears_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """begin_compute + end_compute clears the dirty flag."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var memo = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 0)
    )

    # Starts dirty
    assert_equal(
        w[].call_i32("shell_memo_is_dirty", args_ptr_i32(shell, memo)), 1
    )

    # Compute: begin, read inputs, end with result
    w[].call_void("shell_memo_begin_compute", args_ptr_i32(shell, memo))
    w[].call_void(
        "shell_memo_end_compute_i32", args_ptr_i32_i32(shell, memo, 99)
    )

    # Now clean
    assert_equal(
        w[].call_i32("shell_memo_is_dirty", args_ptr_i32(shell, memo)), 0
    )

    # Value updated
    var val = w[].call_i32("shell_memo_read_i32", args_ptr_i32(shell, memo))
    assert_equal(val, 99)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_memo_signal_write_marks_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Writing an input signal marks the memo dirty and propagates to scope."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 10))
    var memo = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 0)
    )

    # First compute: read the signal to establish dependency
    w[].call_void("shell_memo_begin_compute", args_ptr_i32(shell, memo))
    # Read the signal inside the memo's reactive context to subscribe
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))
    w[].call_void(
        "shell_memo_end_compute_i32", args_ptr_i32_i32(shell, memo, 10)
    )

    # Memo is now clean
    assert_equal(
        w[].call_i32("shell_memo_is_dirty", args_ptr_i32(shell, memo)), 0
    )

    # Subscribe the scope to the memo's output signal by reading memo inside scope context
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    _ = w[].call_i32("shell_memo_read_i32", args_ptr_i32(shell, memo))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Write to the input signal — should mark memo dirty + scope dirty
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 20))

    # Memo should be dirty
    assert_equal(
        w[].call_i32("shell_memo_is_dirty", args_ptr_i32(shell, memo)), 1
    )

    # Shell should have dirty scopes
    assert_equal(w[].call_i32("shell_has_dirty", args_ptr(shell)), 1)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_use_memo_hook(w: UnsafePointer[WasmInstance, MutExternalOrigin]):
    """shell_use_memo_i32 creates on first render, returns same ID on re-render.
    """
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    # First render — creates memo
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    var m0 = w[].call_i32("shell_use_memo_i32", args_ptr_i32(shell, 0))
    assert_true(m0 >= 0, "memo ID should be non-negative")
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    # Re-render — returns the same ID
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    var m1 = w[].call_i32("shell_use_memo_i32", args_ptr_i32(shell, 999))
    assert_equal(m1, m0, "same memo ID on re-render")
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_memo_parity_with_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Shell memo helpers produce same results as raw Runtime methods."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    # Create memo via shell
    var shell_memo = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 50)
    )

    # Read via shell and via raw runtime — should match
    var shell_val = w[].call_i32(
        "shell_memo_read_i32", args_ptr_i32(shell, shell_memo)
    )
    var rt_val = w[].call_i32("memo_read_i32", args_ptr_i32(rt, shell_memo))
    assert_equal(shell_val, rt_val, "shell read == runtime read")
    assert_equal(shell_val, 50)

    # Dirty check via shell and runtime — should match
    var shell_dirty = w[].call_i32(
        "shell_memo_is_dirty", args_ptr_i32(shell, shell_memo)
    )
    var rt_dirty = w[].call_i32("memo_is_dirty", args_ptr_i32(rt, shell_memo))
    assert_equal(shell_dirty, rt_dirty, "shell dirty == runtime dirty")

    # Compute via shell, then verify via runtime
    w[].call_void("shell_memo_begin_compute", args_ptr_i32(shell, shell_memo))
    w[].call_void(
        "shell_memo_end_compute_i32",
        args_ptr_i32_i32(shell, shell_memo, 100),
    )

    var shell_val2 = w[].call_i32(
        "shell_memo_read_i32", args_ptr_i32(shell, shell_memo)
    )
    var rt_val2 = w[].call_i32("memo_read_i32", args_ptr_i32(rt, shell_memo))
    assert_equal(shell_val2, 100)
    assert_equal(rt_val2, 100)
    assert_equal(shell_val2, rt_val2, "post-compute: shell == runtime")

    # Clean after compute
    assert_equal(
        w[].call_i32("shell_memo_is_dirty", args_ptr_i32(shell, shell_memo)),
        0,
    )
    assert_equal(w[].call_i32("memo_is_dirty", args_ptr_i32(rt, shell_memo)), 0)

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_memo_multiple_memos(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Multiple memos via shell have distinct IDs and independent values."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    var m0 = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 10)
    )
    var m1 = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 20)
    )
    var m2 = w[].call_i32(
        "shell_memo_create_i32", args_ptr_i32_i32(shell, scope, 30)
    )

    # Distinct IDs
    assert_true(m0 != m1, "m0 != m1")
    assert_true(m1 != m2, "m1 != m2")
    assert_true(m0 != m2, "m0 != m2")

    # Independent values
    assert_equal(
        w[].call_i32("shell_memo_read_i32", args_ptr_i32(shell, m0)), 10
    )
    assert_equal(
        w[].call_i32("shell_memo_read_i32", args_ptr_i32(shell, m1)), 20
    )
    assert_equal(
        w[].call_i32("shell_memo_read_i32", args_ptr_i32(shell, m2)), 30
    )

    w[].call_void("shell_destroy", args_ptr(shell))


# ── Shell effect helpers (M14.4) ────────────────────────────────────────────


def test_shell_effect_create_returns_valid_id(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Creating an effect via shell returns a valid ID."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    var eid = w[].call_i32("shell_effect_create", args_ptr_i32(shell, scope))
    assert_true(eid >= 0, "effect ID should be non-negative")

    # Effect starts pending
    assert_equal(
        w[].call_i32("shell_effect_is_pending", args_ptr_i32(shell, eid)),
        1,
        "effect should start pending",
    )

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_effect_begin_end_run_clears_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Running an effect via shell clears the pending flag."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    var eid = w[].call_i32("shell_effect_create", args_ptr_i32(shell, scope))
    assert_equal(
        w[].call_i32("shell_effect_is_pending", args_ptr_i32(shell, eid)),
        1,
        "pending before run",
    )

    w[].call_void("shell_effect_begin_run", args_ptr_i32(shell, eid))
    w[].call_void("shell_effect_end_run", args_ptr_i32(shell, eid))

    assert_equal(
        w[].call_i32("shell_effect_is_pending", args_ptr_i32(shell, eid)),
        0,
        "not pending after run",
    )

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_effect_signal_write_marks_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Writing a signal that an effect reads via shell marks it pending."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    var eid = w[].call_i32("shell_effect_create", args_ptr_i32(shell, scope))

    # Run effect, reading the signal to establish subscription
    w[].call_void("shell_effect_begin_run", args_ptr_i32(shell, eid))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_effect_end_run", args_ptr_i32(shell, eid))

    assert_equal(
        w[].call_i32("shell_effect_is_pending", args_ptr_i32(shell, eid)),
        0,
        "not pending after run",
    )

    # Write to signal
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 42))

    assert_equal(
        w[].call_i32("shell_effect_is_pending", args_ptr_i32(shell, eid)),
        1,
        "pending after signal write",
    )

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_use_effect_hook(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Use_effect hook via shell creates on first render, returns same on re-render.
    """
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    # First render
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    var eid1 = w[].call_i32("shell_use_effect", args_ptr(shell))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    assert_true(eid1 >= 0, "effect ID should be non-negative")

    # Re-render
    _ = w[].call_i32("shell_begin_render", args_ptr_i32(shell, scope))
    var eid2 = w[].call_i32("shell_use_effect", args_ptr(shell))
    w[].call_void("shell_end_render", args_ptr_i32(shell, -1))

    assert_equal(eid1, eid2, "same effect ID on re-render")

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_effect_parity_with_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Shell effect helpers produce the same result as raw Runtime methods."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var rt = Int(w[].call_i64("shell_rt_ptr", args_ptr(shell)))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))

    # Create effect via shell
    var eid = w[].call_i32("shell_effect_create", args_ptr_i32(shell, scope))

    # Verify via raw runtime
    assert_equal(
        w[].call_i32("effect_count", args_ptr(rt)),
        1,
        "runtime sees 1 effect",
    )
    assert_equal(
        w[].call_i32("effect_is_pending", args_ptr_i32(rt, eid)),
        1,
        "runtime sees effect pending",
    )

    # Run via shell, verify via runtime
    w[].call_void("shell_effect_begin_run", args_ptr_i32(shell, eid))
    w[].call_void("shell_effect_end_run", args_ptr_i32(shell, eid))

    assert_equal(
        w[].call_i32("effect_is_pending", args_ptr_i32(rt, eid)),
        0,
        "runtime sees effect not pending after shell run",
    )

    w[].call_void("shell_destroy", args_ptr(shell))


def test_shell_effect_drain_pending(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Shell drain_pending returns correct count of pending effects."""
    var shell = Int(w[].call_i64("shell_create", no_args()))
    var scope = w[].call_i32("shell_create_root_scope", args_ptr(shell))
    var sig = w[].call_i32("shell_create_signal_i32", args_ptr_i32(shell, 0))

    var e0 = w[].call_i32("shell_effect_create", args_ptr_i32(shell, scope))
    var e1 = w[].call_i32("shell_effect_create", args_ptr_i32(shell, scope))

    # Both start pending
    assert_equal(
        w[].call_i32("shell_effect_drain_pending", args_ptr(shell)),
        2,
        "both effects start pending",
    )

    # Run both, subscribing to signal
    w[].call_void("shell_effect_begin_run", args_ptr_i32(shell, e0))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_effect_end_run", args_ptr_i32(shell, e0))
    w[].call_void("shell_effect_begin_run", args_ptr_i32(shell, e1))
    _ = w[].call_i32("shell_read_signal_i32", args_ptr_i32(shell, sig))
    w[].call_void("shell_effect_end_run", args_ptr_i32(shell, e1))

    assert_equal(
        w[].call_i32("shell_effect_drain_pending", args_ptr(shell)),
        0,
        "no pending after running both",
    )

    # Write → both pending
    w[].call_void("shell_write_signal_i32", args_ptr_i32_i32(shell, sig, 1))
    assert_equal(
        w[].call_i32("shell_effect_drain_pending", args_ptr(shell)),
        2,
        "both pending after write",
    )

    w[].call_void("shell_destroy", args_ptr(shell))


# ── Counter doubled demo (inline computation, no memo) ──────────────────────


def test_counter_doubled_initial(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Counter doubled value is 0 initially (computed inline as count * 2)."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    # counter_doubled_memo returns -1 (memo removed in ergonomic rewrite)
    var memo_id = w[].call_i32("counter_doubled_memo", args_ptr(app))
    assert_equal(memo_id, -1, "doubled memo removed (returns -1)")
    # counter_doubled_value computes count * 2 inline
    var doubled = w[].call_i32("counter_doubled_value", args_ptr(app))
    assert_equal(doubled, 0, "doubled starts at 0")
    w[].call_void("counter_destroy", args_ptr(app))


def test_counter_doubled_after_first_build(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After first rebuild, doubled = count * 2 = 0."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))
    var doubled = w[].call_i32("counter_doubled_value", args_ptr(app))
    assert_equal(doubled, 0, "doubled is 0 after first build")
    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 0, "count is 0")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_counter_doubled_after_increment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After increment + flush, doubled = count * 2 = 2."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    # Initial mount
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))
    # Increment
    var incr = w[].call_i32("counter_incr_handler", args_ptr(app))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr, 0))
    # Flush
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 4096))
    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 1, "count is 1 after increment")
    var doubled = w[].call_i32("counter_doubled_value", args_ptr(app))
    assert_equal(doubled, 2, "doubled is 2 after increment")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_counter_doubled_multiple_increments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After 5 increments + flush, doubled = 10."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    # Initial mount
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))
    var incr = w[].call_i32("counter_incr_handler", args_ptr(app))
    for _ in range(5):
        _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr, 0))
        _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 4096))
    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 5, "count is 5 after 5 increments")
    var doubled = w[].call_i32("counter_doubled_value", args_ptr(app))
    assert_equal(doubled, 10, "doubled is 10 after 5 increments")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_counter_doubled_decrement(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After decrement, doubled = -2."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    # Initial mount
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))
    var decr = w[].call_i32("counter_decr_handler", args_ptr(app))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, decr, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 4096))
    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, -1, "count is -1 after decrement")
    var doubled = w[].call_i32("counter_doubled_value", args_ptr(app))
    assert_equal(doubled, -2, "doubled is -2 after decrement")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


# ── Phase 17 — ItemBuilder + HandlerAction on KeyedList ──────────────────────


def test_todo_handler_map_empty_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After init (before any flush), handler map count is 0."""
    var app = Int(w[].call_i64("todo_init", no_args()))
    var count = w[].call_i32("todo_handler_map_count", args_ptr(app))
    assert_equal(count, 0, "handler map empty before first rebuild")
    w[].call_void("todo_destroy", args_ptr(app))


def test_bench_handler_map_empty_initially(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After init (before any create), bench handler map count is 0."""
    var app = Int(w[].call_i64("bench_init", no_args()))
    var count = w[].call_i32("bench_handler_map_count", args_ptr(app))
    assert_equal(count, 0, "bench handler map empty before rows created")
    w[].call_void("bench_destroy", args_ptr(app))


def test_bench_handler_map_after_create(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After creating 10 rows and flushing, handler map has 2*10=20 entries."""
    var app = Int(w[].call_i64("bench_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
    # Initial mount (empty)
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app, buf, 65536))
    # Create 10 rows
    w[].call_void("bench_create", args_ptr_i32(app, 10))
    # Flush to trigger rebuild (which populates handler map)
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    var map_count = w[].call_i32("bench_handler_map_count", args_ptr(app))
    assert_equal(map_count, 20, "2 handlers per row × 10 rows = 20")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("bench_destroy", args_ptr(app))


def test_bench_handler_map_cleared_on_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Handler map is cleared on each begin_rebuild (via flush)."""
    var app = Int(w[].call_i64("bench_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app, buf, 65536))
    # Create 10 rows, flush
    w[].call_void("bench_create", args_ptr_i32(app, 10))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    assert_equal(
        w[].call_i32("bench_handler_map_count", args_ptr(app)),
        20,
        "20 mappings after 10 rows",
    )
    # Create 5 rows (replaces), flush — map should reset to 2*5=10
    w[].call_void("bench_create", args_ptr_i32(app, 5))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    assert_equal(
        w[].call_i32("bench_handler_map_count", args_ptr(app)),
        10,
        "10 mappings after replacing with 5 rows",
    )
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("bench_destroy", args_ptr(app))


def test_bench_handler_map_clear_rows(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """After clearing all rows, handler map has 0 entries."""
    var app = Int(w[].call_i64("bench_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app, buf, 65536))
    # Create 10 rows, flush
    w[].call_void("bench_create", args_ptr_i32(app, 10))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    assert_equal(
        w[].call_i32("bench_handler_map_count", args_ptr(app)),
        20,
        "20 mappings after 10 rows",
    )
    # Clear all rows, flush — map should be 0
    w[].call_void("bench_clear", args_ptr(app))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    assert_equal(
        w[].call_i32("bench_handler_map_count", args_ptr(app)),
        0,
        "0 mappings after clearing rows",
    )
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("bench_destroy", args_ptr(app))


def test_bench_handler_map_append_rows(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Append adds to row count; handler map reflects total."""
    var app = Int(w[].call_i64("bench_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app, buf, 65536))
    # Create 5, flush
    w[].call_void("bench_create", args_ptr_i32(app, 5))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    assert_equal(
        w[].call_i32("bench_handler_map_count", args_ptr(app)),
        10,
        "10 after 5 rows",
    )
    # Append 3 more, flush — total 8 rows → 16 mappings
    w[].call_void("bench_append", args_ptr_i32(app, 3))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    assert_equal(
        w[].call_i32("bench_handler_map_count", args_ptr(app)),
        16,
        "16 after appending 3 to 5 rows",
    )
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("bench_destroy", args_ptr(app))


def test_bench_row_count_matches_handler_map(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Handler map count is always 2 × row count after flush."""
    var app = Int(w[].call_i64("bench_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app, buf, 65536))
    # Create 20 rows
    w[].call_void("bench_create", args_ptr_i32(app, 20))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    var rows = w[].call_i32("bench_row_count", args_ptr(app))
    var maps = w[].call_i32("bench_handler_map_count", args_ptr(app))
    assert_equal(maps, rows * 2, "handler map = 2 × row count")
    # Remove a row
    var first_id = w[].call_i32("bench_row_id_at", args_ptr_i32(app, 0))
    w[].call_void("bench_remove", args_ptr_i32(app, first_id))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app, buf, 65536))
    rows = w[].call_i32("bench_row_count", args_ptr(app))
    maps = w[].call_i32("bench_handler_map_count", args_ptr(app))
    assert_equal(maps, rows * 2, "handler map = 2 × row count after remove")
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("bench_destroy", args_ptr(app))


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_shell_create_destroy(w)
    test_shell_is_alive_after_create(w)
    test_shell_rt_ptr_non_zero(w)
    test_shell_store_ptr_non_zero(w)
    test_shell_eid_ptr_non_zero(w)
    test_shell_create_root_scope(w)
    test_shell_create_child_scope(w)
    test_shell_create_multiple_root_scopes(w)
    test_shell_create_signal_i32(w)
    test_shell_write_and_peek_signal(w)
    test_shell_read_signal_with_context(w)
    test_shell_peek_does_not_subscribe(w)
    test_shell_multiple_signals(w)
    test_shell_begin_end_render(w)
    test_shell_nested_render(w)
    test_shell_initially_not_dirty(w)
    test_shell_dirty_after_signal_write(w)
    test_shell_collect_and_drain_dirty(w)
    test_shell_dirty_height_ordering(w)
    test_shell_dispatch_event(w)
    test_shell_mount_text_vnode(w)
    test_shell_mount_template_ref(w)
    test_shell_diff_same_text_zero_mutations(w)
    test_shell_diff_text_changed(w)
    test_shell_full_mount_update_cycle(w)
    test_shell_independent_instances(w)
    # Shell memo helpers (M13.5)
    test_shell_memo_create_returns_valid_id(w)
    test_shell_memo_initial_value_readable(w)
    test_shell_memo_starts_dirty(w)
    test_shell_memo_compute_clears_dirty(w)
    test_shell_memo_signal_write_marks_dirty(w)
    test_shell_use_memo_hook(w)
    test_shell_memo_parity_with_runtime(w)
    test_shell_memo_multiple_memos(w)
    # Counter doubled demo (inline computation, no memo)
    test_counter_doubled_initial(w)
    test_counter_doubled_after_first_build(w)
    test_counter_doubled_after_increment(w)
    test_counter_doubled_multiple_increments(w)
    test_counter_doubled_decrement(w)
    # Shell effect helpers (M14.4)
    test_shell_effect_create_returns_valid_id(w)
    test_shell_effect_begin_end_run_clears_pending(w)
    test_shell_effect_signal_write_marks_pending(w)
    test_shell_use_effect_hook(w)
    test_shell_effect_parity_with_runtime(w)
    test_shell_effect_drain_pending(w)
    # Phase 17 — ItemBuilder + HandlerAction on KeyedList
    test_todo_handler_map_empty_initially(w)
    test_bench_handler_map_empty_initially(w)
    test_bench_handler_map_after_create(w)
    test_bench_handler_map_cleared_on_rebuild(w)
    test_bench_handler_map_clear_rows(w)
    test_bench_handler_map_append_rows(w)
    test_bench_row_count_matches_handler_map(w)
    print("component: 52/52 passed")
