# ScopeArena and scope/hook lifecycle exercised through the real WASM binary
# via mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that the scope arena, hook system, and rendering lifecycle
# work correctly when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_scopes.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    """Create a heap-allocated Runtime via WASM."""
    return Int(w[].call_i64("runtime_create", no_args()))


fn _destroy_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    """Destroy a heap-allocated Runtime via WASM."""
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── Scope lifecycle ──────────────────────────────────────────────────────────


fn test_scope_create_and_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        0,
        "new runtime has 0 scopes",
    )

    var s0 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        1,
        "1 scope after create",
    )
    assert_equal(
        Int(w[].call_i32("scope_contains", args_ptr_i32(rt, s0))),
        1,
        "scope exists",
    )

    w[].call_void("scope_destroy", args_ptr_i32(rt, s0))
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        0,
        "0 scopes after destroy",
    )
    assert_equal(
        Int(w[].call_i32("scope_contains", args_ptr_i32(rt, s0))),
        0,
        "scope no longer exists",
    )

    _destroy_runtime(w, rt)


fn test_scope_sequential_ids(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s0 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var s1 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var s2 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    assert_equal(s0, 0, "first scope gets ID 0")
    assert_equal(s1, 1, "second scope gets ID 1")
    assert_equal(s2, 2, "third scope gets ID 2")
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        3,
        "3 scopes created",
    )

    _destroy_runtime(w, rt)


fn test_scope_slot_reuse_after_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s0 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    _ = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    w[].call_void("scope_destroy", args_ptr_i32(rt, s0))

    var s2 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(s2, s0, "new scope reuses destroyed slot")
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        2,
        "2 scopes after reuse",
    )

    _destroy_runtime(w, rt)


fn test_scope_double_destroy_is_noop(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s0 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    w[].call_void("scope_destroy", args_ptr_i32(rt, s0))
    w[].call_void("scope_destroy", args_ptr_i32(rt, s0))  # should not crash
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        0,
        "still 0 scopes after double destroy",
    )

    _destroy_runtime(w, rt)


# ── Height and parent tracking ───────────────────────────────────────────────


fn test_scope_height_and_parent_tracking(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var root = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("scope_height", args_ptr_i32(rt, root))),
        0,
        "root height is 0",
    )
    assert_equal(
        Int(w[].call_i32("scope_parent", args_ptr_i32(rt, root))),
        -1,
        "root has no parent (-1)",
    )

    var child = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 1, root)))
    assert_equal(
        Int(w[].call_i32("scope_height", args_ptr_i32(rt, child))),
        1,
        "child height is 1",
    )
    assert_equal(
        Int(w[].call_i32("scope_parent", args_ptr_i32(rt, child))),
        root,
        "child parent is root",
    )

    var grandchild = Int(
        w[].call_i32("scope_create", args_ptr_i32_i32(rt, 2, child))
    )
    assert_equal(
        Int(w[].call_i32("scope_height", args_ptr_i32(rt, grandchild))),
        2,
        "grandchild height is 2",
    )
    assert_equal(
        Int(w[].call_i32("scope_parent", args_ptr_i32(rt, grandchild))),
        child,
        "grandchild parent is child",
    )

    _destroy_runtime(w, rt)


fn test_scope_create_child_auto_computes_height(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var root = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var child = Int(w[].call_i32("scope_create_child", args_ptr_i32(rt, root)))
    var grandchild = Int(
        w[].call_i32("scope_create_child", args_ptr_i32(rt, child))
    )

    assert_equal(
        Int(w[].call_i32("scope_height", args_ptr_i32(rt, child))),
        1,
        "child height auto-computed to 1",
    )
    assert_equal(
        Int(w[].call_i32("scope_parent", args_ptr_i32(rt, child))),
        root,
        "child parent is root",
    )
    assert_equal(
        Int(w[].call_i32("scope_height", args_ptr_i32(rt, grandchild))),
        2,
        "grandchild height auto-computed to 2",
    )
    assert_equal(
        Int(w[].call_i32("scope_parent", args_ptr_i32(rt, grandchild))),
        child,
        "grandchild parent is child",
    )

    _destroy_runtime(w, rt)


# ── Dirty flag ───────────────────────────────────────────────────────────────


fn test_scope_dirty_flag(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("scope_is_dirty", args_ptr_i32(rt, s))),
        0,
        "not dirty initially",
    )

    w[].call_void("scope_set_dirty", args_ptr_i32_i32(rt, s, 1))
    assert_equal(
        Int(w[].call_i32("scope_is_dirty", args_ptr_i32(rt, s))),
        1,
        "dirty after set_dirty(True)",
    )

    w[].call_void("scope_set_dirty", args_ptr_i32_i32(rt, s, 0))
    assert_equal(
        Int(w[].call_i32("scope_is_dirty", args_ptr_i32(rt, s))),
        0,
        "clean after set_dirty(False)",
    )

    _destroy_runtime(w, rt)


# ── Render count ─────────────────────────────────────────────────────────────


fn test_scope_render_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("scope_render_count", args_ptr_i32(rt, s))),
        0,
        "render_count starts at 0",
    )

    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(
        Int(w[].call_i32("scope_render_count", args_ptr_i32(rt, s))),
        1,
        "render_count is 1 after first begin_render",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(
        Int(w[].call_i32("scope_render_count", args_ptr_i32(rt, s))),
        2,
        "render_count is 2 after second begin_render",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))

    _destroy_runtime(w, rt)


# ── Begin render clears dirty ────────────────────────────────────────────────


fn test_scope_begin_render_clears_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    w[].call_void("scope_set_dirty", args_ptr_i32_i32(rt, s, 1))
    assert_equal(
        Int(w[].call_i32("scope_is_dirty", args_ptr_i32(rt, s))),
        1,
        "dirty before render",
    )

    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(
        Int(w[].call_i32("scope_is_dirty", args_ptr_i32(rt, s))),
        0,
        "clean after begin_render",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    _destroy_runtime(w, rt)


# ── Begin/end render manages current scope ───────────────────────────────────


fn test_scope_begin_end_render_manages_current(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    assert_equal(
        Int(w[].call_i32("scope_has_scope", args_ptr(rt))),
        0,
        "no scope initially",
    )
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        -1,
        "current scope is -1 initially",
    )

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(prev, -1, "previous scope is -1 (was no scope)")
    assert_equal(
        Int(w[].call_i32("scope_has_scope", args_ptr(rt))),
        1,
        "scope active during render",
    )
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        s,
        "current scope is the rendering scope",
    )

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))
    assert_equal(
        Int(w[].call_i32("scope_has_scope", args_ptr(rt))),
        0,
        "no scope after end_render",
    )
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        -1,
        "current scope is -1 after end_render",
    )

    _destroy_runtime(w, rt)


# ── Begin render sets reactive context ───────────────────────────────────────


fn test_scope_begin_render_sets_reactive_context(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("runtime_has_context", args_ptr(rt))),
        0,
        "no context initially",
    )

    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(
        Int(w[].call_i32("runtime_has_context", args_ptr(rt))),
        1,
        "context active during render",
    )

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))
    assert_equal(
        Int(w[].call_i32("runtime_has_context", args_ptr(rt))),
        0,
        "context cleared after end_render",
    )

    _destroy_runtime(w, rt)


# ── Nested scope rendering ───────────────────────────────────────────────────


fn test_scope_nested_rendering(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var root = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var child = Int(w[].call_i32("scope_create_child", args_ptr_i32(rt, root)))

    # Begin rendering root
    var prev1 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, root)))
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        root,
        "current scope is root",
    )

    # Nest: begin rendering child
    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, child)))
    assert_equal(prev2, root, "previous scope was root")
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        child,
        "current scope is child",
    )

    # End child rendering
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        root,
        "current scope restored to root",
    )

    # End root rendering
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev1))
    assert_equal(
        Int(w[].call_i32("scope_get_current", args_ptr(rt))),
        -1,
        "current scope cleared",
    )

    _destroy_runtime(w, rt)


# ── is_first_render ──────────────────────────────────────────────────────────


fn test_scope_is_first_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("scope_is_first_render", args_ptr_i32(rt, s))),
        1,
        "first render before any rendering",
    )

    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(
        Int(w[].call_i32("scope_is_first_render", args_ptr_i32(rt, s))),
        1,
        "first render during first render pass",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    assert_equal(
        Int(w[].call_i32("scope_is_first_render", args_ptr_i32(rt, s))),
        0,
        "not first render on second pass",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))

    _destroy_runtime(w, rt)


# ── Hooks start empty ────────────────────────────────────────────────────────


fn test_scope_hooks_start_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        0,
        "no hooks initially",
    )

    _destroy_runtime(w, rt)


# ── Hook: use_signal creates signal on first render ──────────────────────────


fn test_hook_use_signal_creates_on_first_render(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))

    var key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 42)))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))),
        42,
        "signal created with initial value 42",
    )
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        1,
        "1 hook after use_signal",
    )
    assert_equal(
        Int(w[].call_i32("scope_hook_value_at", args_ptr_i32_i32(rt, s, 0))),
        key,
        "hook[0] stores the signal key",
    )
    # HOOK_SIGNAL tag is 0
    assert_equal(
        Int(w[].call_i32("scope_hook_tag_at", args_ptr_i32_i32(rt, s, 0))),
        0,
        "hook[0] tag is HOOK_SIGNAL (0)",
    )

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))
    _destroy_runtime(w, rt)


# ── Hook: use_signal returns same signal on re-render ────────────────────────


fn test_hook_use_signal_same_on_rerender(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # First render: create signal
    var prev1 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key1 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 100)))
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key1))),
        100,
        "first render: signal value is 100",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev1))

    # Modify signal between renders
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key1, 200))

    # Second render: retrieve same signal (initial value ignored)
    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key2 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 999)))
    assert_equal(key2, key1, "re-render returns same signal key")
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key2))),
        200,
        "signal retains modified value, not initial",
    )
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        1,
        "still 1 hook (no new hook created)",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))

    _destroy_runtime(w, rt)


# ── Hook: multiple signals in same scope ─────────────────────────────────────


fn test_hook_multiple_signals_same_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # First render: create 3 signals
    var prev1 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var k1 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 10)))
    var k2 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 20)))
    var k3 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 30)))
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        3,
        "3 hooks after first render",
    )
    assert_true(k1 != k2 and k2 != k3, "all signal keys distinct")
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev1))

    # Second render: same order returns same keys
    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var k1b = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    var k2b = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    var k3b = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    assert_equal(k1b, k1, "re-render hook 0 returns same key")
    assert_equal(k2b, k2, "re-render hook 1 returns same key")
    assert_equal(k3b, k3, "re-render hook 2 returns same key")
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        3,
        "still 3 hooks",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))

    # Values are independent
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k1))),
        10,
        "signal 1 has value 10",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k2))),
        20,
        "signal 2 has value 20",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k3))),
        30,
        "signal 3 has value 30",
    )

    _destroy_runtime(w, rt)


# ── Hook: signals in different scopes are independent ────────────────────────


fn test_hook_signals_in_different_scopes_independent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var s1 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var s2 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # Render scope 1
    var prev1 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s1)))
    var k1 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 100)))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev1))

    # Render scope 2
    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s2)))
    var k2 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 200)))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))

    assert_true(k1 != k2, "different scopes get different signal keys")
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k1))),
        100,
        "scope 1 signal is 100",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k2))),
        200,
        "scope 2 signal is 200",
    )

    # Modify one, other unchanged
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, k1, 999))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k1))),
        999,
        "scope 1 signal updated",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, k2))),
        200,
        "scope 2 signal unchanged",
    )

    _destroy_runtime(w, rt)


# ── Hook: signal read during render subscribes scope ─────────────────────────


fn test_hook_signal_read_subscribes_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # First render
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))

    # Read the signal during render — should subscribe this scope
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        1,
        "scope subscribed after read during render",
    )

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Write should mark scope dirty
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, 42))
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "dirty after signal write",
    )
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        1,
        "1 dirty scope",
    )

    _destroy_runtime(w, rt)


# ── Hook: peek during render does NOT subscribe ──────────────────────────────


fn test_hook_peek_does_not_subscribe(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))

    # Peek should NOT subscribe
    _ = w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))
    assert_equal(
        Int(w[].call_i32("signal_subscriber_count", args_ptr_i32(rt, key))),
        0,
        "peek does not subscribe",
    )

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    _destroy_runtime(w, rt)


# ── Nested rendering: child signals subscribe child scope ────────────────────


fn test_hook_nested_rendering_subscribes_correct_scope(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var root = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var child = Int(w[].call_i32("scope_create_child", args_ptr_i32(rt, root)))

    # Begin root render
    var prev_root = Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, root))
    )
    var root_signal = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 10))
    )
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, root_signal))

    # Begin child render (nested)
    var prev_child = Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, child))
    )
    var child_signal = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 20))
    )
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, child_signal))

    # Child signal should have child as subscriber, not root
    assert_equal(
        Int(
            w[].call_i32(
                "signal_subscriber_count", args_ptr_i32(rt, child_signal)
            )
        ),
        1,
        "child signal has 1 subscriber",
    )

    # End child render
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev_child))

    # Root signal should still have root subscribed
    assert_equal(
        Int(
            w[].call_i32(
                "signal_subscriber_count", args_ptr_i32(rt, root_signal)
            )
        ),
        1,
        "root signal has 1 subscriber",
    )

    # End root render
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev_root))

    # Write to child signal should only mark child dirty
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, child_signal, 99))
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        1,
        "only 1 dirty scope from child signal write",
    )

    _destroy_runtime(w, rt)


# ── Stress: 100 scopes ──────────────────────────────────────────────────────


fn test_scope_stress_100_scopes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var ids = List[Int]()
    for i in range(100):
        ids.append(
            Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
        )
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        100,
        "100 scopes created",
    )

    # Destroy half (even indices)
    for i in range(0, 100, 2):
        w[].call_void("scope_destroy", args_ptr_i32(rt, ids[i]))
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        50,
        "50 scopes after destroying half",
    )

    # Create 50 more (reuse freed slots)
    var new_ids = List[Int]()
    for i in range(50):
        new_ids.append(
            Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
        )
    assert_equal(
        Int(w[].call_i32("scope_count", args_ptr(rt))),
        100,
        "100 scopes after refill",
    )

    # Verify all odd-indexed original scopes still exist
    var all_exist = True
    for i in range(1, 100, 2):
        if Int(w[].call_i32("scope_contains", args_ptr_i32(rt, ids[i]))) != 1:
            all_exist = False
            break
    assert_true(all_exist, "all odd-indexed original scopes still exist")

    _destroy_runtime(w, rt)


# ── Hook: signal stable across many re-renders ──────────────────────────────


fn test_hook_signal_stable_across_many_rerenders(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # First render
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Increment signal and re-render 50 times
    for i in range(1, 51):
        w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, key, i))

        prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
        var k = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 999)))
        assert_equal(k, key, "re-render: same key")
        w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))),
        50,
        "signal holds value 50 after 50 writes",
    )
    assert_equal(
        Int(w[].call_i32("scope_render_count", args_ptr_i32(rt, s))),
        51,
        "render_count is 51 after 1 + 50 re-renders",
    )
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        1,
        "still just 1 hook",
    )

    _destroy_runtime(w, rt)


# ── Simulated counter component ──────────────────────────────────────────────


fn test_hook_simulated_counter_component(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)
    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # First render
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var count_key = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0))
    )
    var count_val = Int(
        w[].call_i32("signal_read_i32", args_ptr_i32(rt, count_key))
    )
    assert_equal(count_val, 0, "initial count is 0")
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Simulate click: count += 1
    var current = Int(
        w[].call_i32("signal_peek_i32", args_ptr_i32(rt, count_key))
    )
    w[].call_void(
        "signal_write_i32", args_ptr_i32_i32(rt, count_key, current + 1)
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, count_key))),
        1,
        "count is 1 after increment",
    )
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "scope marked dirty after signal write",
    )

    # Re-render (triggered by dirty)
    prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var count_key2 = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0))
    )
    assert_equal(count_key2, count_key, "same signal key on re-render")
    var count_val2 = Int(
        w[].call_i32("signal_read_i32", args_ptr_i32(rt, count_key2))
    )
    assert_equal(count_val2, 1, "count reads 1 on re-render")
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Another click
    current = Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, count_key)))
    w[].call_void(
        "signal_write_i32", args_ptr_i32_i32(rt, count_key, current + 1)
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, count_key))),
        2,
        "count is 2 after second increment",
    )

    _destroy_runtime(w, rt)


# ── Simulated multi-state component ──────────────────────────────────────────


fn test_hook_simulated_multi_state_component(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)
    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # First render: 3 signals (name as i32=0, age=0, submitted=0)
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var name_key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    var age_key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    var submitted_key = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0))
    )
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        3,
        "3 hooks for 3 signals",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Simulate user interaction
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, name_key, 42))
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, age_key, 25))

    # Re-render
    prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var name_key2 = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0))
    )
    var age_key2 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    var submitted_key2 = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0))
    )
    assert_equal(name_key2, name_key, "name signal stable")
    assert_equal(age_key2, age_key, "age signal stable")
    assert_equal(submitted_key2, submitted_key, "submitted signal stable")
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, name_key2))),
        42,
        "name retains value",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, age_key2))),
        25,
        "age retains value",
    )
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, submitted_key2))),
        0,
        "submitted still false",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    _destroy_runtime(w, rt)


# ── Simulated parent-child component tree ────────────────────────────────────


fn test_hook_simulated_parent_child_tree(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var parent = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var child1 = Int(
        w[].call_i32("scope_create_child", args_ptr_i32(rt, parent))
    )
    var child2 = Int(
        w[].call_i32("scope_create_child", args_ptr_i32(rt, parent))
    )

    # Render parent
    var prev_p = Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, parent))
    )
    var parent_count = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0))
    )
    _ = w[].call_i32(
        "signal_read_i32", args_ptr_i32(rt, parent_count)
    )  # subscribe parent

    # Render child1 (nested)
    var prev_c1 = Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, child1))
    )
    var child1_local = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 10))
    )
    _ = w[].call_i32(
        "signal_read_i32", args_ptr_i32(rt, child1_local)
    )  # subscribe child1
    # Also read parent's signal from child1
    _ = w[].call_i32(
        "signal_read_i32", args_ptr_i32(rt, parent_count)
    )  # child1 subscribes to parent signal
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev_c1))

    # Render child2 (nested)
    var prev_c2 = Int(
        w[].call_i32("scope_begin_render", args_ptr_i32(rt, child2))
    )
    var child2_local = Int(
        w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 20))
    )
    _ = w[].call_i32(
        "signal_read_i32", args_ptr_i32(rt, child2_local)
    )  # subscribe child2
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev_c2))

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev_p))

    # parentCount has 2 subscribers: parent + child1
    assert_equal(
        Int(
            w[].call_i32(
                "signal_subscriber_count", args_ptr_i32(rt, parent_count)
            )
        ),
        2,
        "parent signal has 2 subscribers (parent + child1)",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "signal_subscriber_count", args_ptr_i32(rt, child1_local)
            )
        ),
        1,
        "child1 signal has 1 subscriber",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "signal_subscriber_count", args_ptr_i32(rt, child2_local)
            )
        ),
        1,
        "child2 signal has 1 subscriber",
    )

    # Write to parent signal → parent and child1 dirty
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, parent_count, 5))
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        2,
        "2 dirty scopes from parent signal write",
    )

    _destroy_runtime(w, rt)


# ── Render scope with no hooks ───────────────────────────────────────────────


fn test_scope_render_with_no_hooks(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # Render with no hook calls (static component)
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    # No hook calls — just render static content
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    assert_equal(
        Int(w[].call_i32("scope_render_count", args_ptr_i32(rt, s))),
        1,
        "render_count is 1",
    )
    assert_equal(
        Int(w[].call_i32("scope_hook_count", args_ptr_i32(rt, s))),
        0,
        "0 hooks for static component",
    )

    # Re-render
    var prev2 = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev2))

    assert_equal(
        Int(w[].call_i32("scope_render_count", args_ptr_i32(rt, s))),
        2,
        "render_count is 2",
    )

    _destroy_runtime(w, rt)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_scope_create_and_destroy(w)
    test_scope_sequential_ids(w)
    test_scope_slot_reuse_after_destroy(w)
    test_scope_double_destroy_is_noop(w)
    test_scope_height_and_parent_tracking(w)
    test_scope_create_child_auto_computes_height(w)
    test_scope_dirty_flag(w)
    test_scope_render_count(w)
    test_scope_begin_render_clears_dirty(w)
    test_scope_begin_end_render_manages_current(w)
    test_scope_begin_render_sets_reactive_context(w)
    test_scope_nested_rendering(w)
    test_scope_is_first_render(w)
    test_scope_hooks_start_empty(w)
    test_hook_use_signal_creates_on_first_render(w)
    test_hook_use_signal_same_on_rerender(w)
    test_hook_multiple_signals_same_scope(w)
    test_hook_signals_in_different_scopes_independent(w)
    test_hook_signal_read_subscribes_scope(w)
    test_hook_peek_does_not_subscribe(w)
    test_hook_nested_rendering_subscribes_correct_scope(w)
    test_scope_stress_100_scopes(w)
    test_hook_signal_stable_across_many_rerenders(w)
    test_hook_simulated_counter_component(w)
    test_hook_simulated_multi_state_component(w)
    test_hook_simulated_parent_child_tree(w)
    test_scope_render_with_no_hooks(w)
    print("scopes: 27/27 passed")
