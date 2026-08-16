# HandlerRegistry and event dispatch exercised through the real WASM binary
# via mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that the event handler registry and dispatch system work
# correctly when compiled to WASM and executed via the Wasmtime runtime.
#
# Note: Tests for handlers_for_scope, event_name, get (copy), HandlerEntry
# default/copy constructors, and event/action type constants are not covered
# here since those specific APIs lack WASM exports.
#
# Run with:
#   mojo test test/test_events.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    args_ptr_i32_ptr,
    args_ptr_i32_i32_ptr,
    args_ptr_i32_i32_i32_ptr,
    args_ptr_ptr,
    args_i64,
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


# ── Registry — initial state ─────────────────────────────────────────────────


fn test_registry_initial_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        0,
        "new registry has 0 handlers",
    )

    _destroy_runtime(w, rt)


# ── Registry — register and query ────────────────────────────────────────────


fn test_register_single_handler(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 1, w[].write_string_struct("click")
            ),
        )
    )

    assert_equal(id, 0, "first handler gets id 0")
    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        1,
        "count is 1 after register",
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id))),
        1,
        "registry contains the handler",
    )

    _destroy_runtime(w, rt)


fn test_register_multiple_handlers(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s0 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var s1 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig0 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var sig1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var sig2 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id0 = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s0, sig0, 42, w[].write_string_struct("click")
            ),
        )
    )
    var id1 = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s0, sig1, 1, w[].write_string_struct("click")
            ),
        )
    )
    var id2 = Int(
        w[].call_i32(
            "handler_register_signal_sub",
            args_ptr_i32_i32_i32_ptr(
                rt, s1, sig2, 5, w[].write_string_struct("input")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        3,
        "count is 3 after 3 registers",
    )
    assert_true(id0 != id1, "id0 != id1")
    assert_true(id1 != id2, "id1 != id2")
    assert_true(id0 != id2, "id0 != id2")
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id0))),
        1,
        "contains id0",
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id1))),
        1,
        "contains id1",
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id2))),
        1,
        "contains id2",
    )

    _destroy_runtime(w, rt)


# ── Registry — query fields ──────────────────────────────────────────────────


fn test_query_signal_add_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # ACTION_SIGNAL_ADD_I32 = 2
    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 10, w[].write_string_struct("click")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_scope_id", args_ptr_i32(rt, id))),
        s,
        "scope_id matches",
    )
    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        2,
        "action is ADD (2)",
    )
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id))),
        sig,
        "signal_key matches",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        10,
        "operand is 10",
    )

    _destroy_runtime(w, rt)


fn test_query_signal_set_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # ACTION_SIGNAL_SET_I32 = 1
    var id = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 99, w[].write_string_struct("submit")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        1,
        "action is SET (1)",
    )
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id))),
        sig,
        "signal_key matches",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        99,
        "operand is 99",
    )

    _destroy_runtime(w, rt)


fn test_query_signal_sub_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # ACTION_SIGNAL_SUB_I32 = 3
    var id = Int(
        w[].call_i32(
            "handler_register_signal_sub",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 7, w[].write_string_struct("click")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        3,
        "action is SUB (3)",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        7,
        "operand (delta) is 7",
    )

    _destroy_runtime(w, rt)


fn test_query_signal_toggle_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # ACTION_SIGNAL_TOGGLE = 4
    var id = Int(
        w[].call_i32(
            "handler_register_signal_toggle",
            args_ptr_i32_i32_ptr(rt, s, sig, w[].write_string_struct("click")),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        4,
        "action is TOGGLE (4)",
    )
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id))),
        sig,
        "signal_key matches",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        0,
        "operand is 0 for toggle",
    )

    _destroy_runtime(w, rt)


fn test_query_signal_set_input_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # ACTION_SIGNAL_SET_INPUT = 5
    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_input",
            args_ptr_i32_i32_ptr(rt, s, sig, w[].write_string_struct("input")),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        5,
        "action is SET_INPUT (5)",
    )
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id))),
        sig,
        "signal_key matches",
    )

    _destroy_runtime(w, rt)


fn test_query_custom_handler_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # ACTION_CUSTOM = 255
    var id = Int(
        w[].call_i32(
            "handler_register_custom",
            args_ptr_i32_ptr(rt, s, w[].write_string_struct("custom-event")),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        255,
        "action is CUSTOM (255)",
    )
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id))),
        0,
        "signal_key is 0 for custom",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        0,
        "operand is 0 for custom",
    )

    _destroy_runtime(w, rt)


fn test_query_noop_handler_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # ACTION_NONE = 0
    var id = Int(
        w[].call_i32(
            "handler_register_noop",
            args_ptr_i32_ptr(rt, s, w[].write_string_struct("blur")),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        0,
        "action is NONE (0)",
    )
    assert_equal(
        Int(w[].call_i32("handler_scope_id", args_ptr_i32(rt, id))),
        s,
        "scope_id matches",
    )

    _destroy_runtime(w, rt)


# ── Registry — remove ────────────────────────────────────────────────────────


fn test_remove_handler(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig0 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var sig1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id0 = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig0, 1, w[].write_string_struct("click")
            ),
        )
    )
    var id1 = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig1, 42, w[].write_string_struct("click")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        2,
        "2 handlers before remove",
    )

    w[].call_void("handler_remove", args_ptr_i32(rt, id0))

    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        1,
        "1 handler after remove",
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id0))),
        0,
        "removed handler not found",
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id1))),
        1,
        "other handler still exists",
    )

    _destroy_runtime(w, rt)


fn test_remove_nonexistent_is_noop(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    _ = w[].call_i32(
        "handler_register_noop",
        args_ptr_i32_ptr(rt, s, w[].write_string_struct("click")),
    )
    assert_equal(Int(w[].call_i32("handler_count", args_ptr(rt))), 1)

    # Remove an ID that was never registered
    w[].call_void("handler_remove", args_ptr_i32(rt, 99))
    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        1,
        "count unchanged after removing nonexistent",
    )

    _destroy_runtime(w, rt)


fn test_double_remove_is_noop(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var id = Int(
        w[].call_i32(
            "handler_register_noop",
            args_ptr_i32_ptr(rt, s, w[].write_string_struct("click")),
        )
    )
    w[].call_void("handler_remove", args_ptr_i32(rt, id))
    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        0,
        "count is 0 after remove",
    )

    w[].call_void("handler_remove", args_ptr_i32(rt, id))  # double remove
    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        0,
        "count still 0 after double remove",
    )

    _destroy_runtime(w, rt)


# ── Registry — slot reuse after remove ───────────────────────────────────────


fn test_slot_reuse_after_remove(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s0 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var s1 = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig0 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var sig1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var sig3 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id0 = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s0, sig0, 1, w[].write_string_struct("click")
            ),
        )
    )
    var id1 = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s0, sig1, 2, w[].write_string_struct("click")
            ),
        )
    )

    w[].call_void("handler_remove", args_ptr_i32(rt, id0))

    # New registration should reuse the freed slot
    var id2 = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s1, sig3, 99, w[].write_string_struct("input")
            ),
        )
    )

    assert_equal(id2, id0, "new handler reuses freed slot")
    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))), 2, "count is 2"
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id2))),
        1,
        "reused slot is alive",
    )

    # Verify the new handler's data
    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id2))),
        2,
        "action is ADD (2)",
    )
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id2))), sig3
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id2))), 99
    )

    # Original id1 should be unchanged
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id1))),
        1,
        "id1 still exists",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id1))),
        2,
        "id1 operand unchanged",
    )

    _destroy_runtime(w, rt)


fn test_multiple_slot_reuse(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var scopes = List[Int]()
    for i in range(5):
        scopes.append(
            Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
        )

    var ids = List[Int]()
    for i in range(5):
        ids.append(
            Int(
                w[].call_i32(
                    "handler_register_noop",
                    args_ptr_i32_ptr(
                        rt, scopes[i], w[].write_string_struct("click")
                    ),
                )
            )
        )
    assert_equal(Int(w[].call_i32("handler_count", args_ptr(rt))), 5)

    # Remove all even-indexed
    w[].call_void("handler_remove", args_ptr_i32(rt, ids[0]))
    w[].call_void("handler_remove", args_ptr_i32(rt, ids[2]))
    w[].call_void("handler_remove", args_ptr_i32(rt, ids[4]))
    assert_equal(Int(w[].call_i32("handler_count", args_ptr(rt))), 2)

    # Re-register 3 more — should reuse freed slots
    var new_ids = List[Int]()
    for i in range(3):
        var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
        new_ids.append(
            Int(
                w[].call_i32(
                    "handler_register_signal_set",
                    args_ptr_i32_i32_i32_ptr(
                        rt,
                        scopes[0],
                        sig,
                        i * 10,
                        w[].write_string_struct("input"),
                    ),
                )
            )
        )
    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        5,
        "back to 5 after reuse",
    )

    # All new IDs should be from the set {ids[0], ids[2], ids[4]}
    for i in range(3):
        var nid = new_ids[i]
        assert_true(
            nid == ids[0] or nid == ids[2] or nid == ids[4],
            "new id " + String(nid) + " should be a reused slot",
        )

    _destroy_runtime(w, rt)


# ── Registry — contains with out-of-bounds ID ────────────────────────────────


fn test_contains_out_of_bounds(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, 0))),
        0,
        "empty registry: contains(0) is false",
    )
    assert_equal(
        Int(w[].call_i32("handler_contains", args_ptr_i32(rt, 100))),
        0,
        "empty registry: contains(100) is false",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — signal_add ────────────────────────────────────────────────────


fn test_dispatch_signal_add(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 10)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 5, w[].write_string_struct("click")
            ),
        )
    )

    # EVT_CLICK = 0
    var result = Int(
        w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    )
    assert_equal(result, 1, "dispatch returns 1 (action executed)")

    # Signal should have been incremented by 5
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        15,
        "signal is 15 after adding 5 to 10",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — signal_sub ────────────────────────────────────────────────────


fn test_dispatch_signal_sub(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 100)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_sub",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 30, w[].write_string_struct("click")
            ),
        )
    )

    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        70,
        "signal is 70 after subtracting 30 from 100",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — signal_set ────────────────────────────────────────────────────


fn test_dispatch_signal_set(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 42, w[].write_string_struct("click")
            ),
        )
    )

    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        42,
        "signal is 42 after set",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — signal_toggle ─────────────────────────────────────────────────


fn test_dispatch_signal_toggle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_toggle",
            args_ptr_i32_i32_ptr(rt, s, sig, w[].write_string_struct("click")),
        )
    )

    # Toggle 0 → 1
    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        1,
        "signal toggled from 0 to 1",
    )

    # Toggle 1 → 0
    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        0,
        "signal toggled from 1 to 0",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — signal_set_input (with i32 payload) ───────────────────────────


fn test_dispatch_signal_set_input(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_input",
            args_ptr_i32_i32_ptr(rt, s, sig, w[].write_string_struct("input")),
        )
    )

    # EVT_INPUT = 1, payload = 77
    var result = Int(
        w[].call_i32(
            "dispatch_event_with_i32", args_ptr_i32_i32_i32(rt, id, 1, 77)
        )
    )
    assert_equal(result, 1, "dispatch returns 1")
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        77,
        "signal set to input value 77",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — marks scope dirty ─────────────────────────────────────────────


fn test_dispatch_marks_scope_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # Subscribe the scope to the signal (via render + read)
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        0,
        "no dirty scopes initially",
    )

    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 1, w[].write_string_struct("click")
            ),
        )
    )

    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "scope is dirty after dispatch",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — multiple dispatches accumulate ────────────────────────────────


fn test_dispatch_multiple_accumulate(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 1, w[].write_string_struct("click")
            ),
        )
    )

    # Dispatch 10 times
    for _ in range(10):
        _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))

    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        10,
        "signal is 10 after 10 dispatches adding 1",
    )

    _destroy_runtime(w, rt)


# ── Dispatch — drain dirty ───────────────────────────────────────────────────


fn test_dispatch_and_drain_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # Subscribe
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 42, w[].write_string_struct("click")
            ),
        )
    )

    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("runtime_dirty_count", args_ptr(rt))),
        1,
        "1 dirty scope",
    )

    var drained = Int(w[].call_i32("runtime_drain_dirty", args_ptr(rt)))
    assert_equal(drained, 1, "drained 1 dirty scope")
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        0,
        "no dirty scopes after drain",
    )

    _destroy_runtime(w, rt)


# ── Edge case — negative operand ─────────────────────────────────────────────


fn test_negative_operand(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, -100, w[].write_string_struct("click")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        -100,
        "negative operand preserved",
    )

    # Dispatch — should add -100
    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, id, 0))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        -100,
        "signal is -100 after adding -100 to 0",
    )

    _destroy_runtime(w, rt)


fn test_int32_min_max_operand(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig0 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
    var sig1 = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    var id_min = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig0, -2147483648, w[].write_string_struct("a")
            ),
        )
    )
    var id_max = Int(
        w[].call_i32(
            "handler_register_signal_set",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig1, 2147483647, w[].write_string_struct("b")
            ),
        )
    )

    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id_min))),
        -2147483648,
        "INT32_MIN operand",
    )
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id_max))),
        2147483647,
        "INT32_MAX operand",
    )

    _destroy_runtime(w, rt)


# ── Stress — many handlers ───────────────────────────────────────────────────


fn test_stress_100_handlers(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var scopes = List[Int]()
    for i in range(10):
        scopes.append(
            Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
        )

    var ids = List[Int]()
    for i in range(100):
        var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))
        ids.append(
            Int(
                w[].call_i32(
                    "handler_register_signal_set",
                    args_ptr_i32_i32_i32_ptr(
                        rt,
                        scopes[i % 10],
                        sig,
                        i * 10,
                        w[].write_string_struct("click"),
                    ),
                )
            )
        )

    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        100,
        "100 handlers registered",
    )

    # Verify all are alive and have correct data
    for i in range(100):
        assert_equal(
            Int(w[].call_i32("handler_contains", args_ptr_i32(rt, ids[i]))),
            1,
            "handler " + String(i) + " is alive",
        )
        assert_equal(
            Int(w[].call_i32("handler_operand", args_ptr_i32(rt, ids[i]))),
            i * 10,
            "handler " + String(i) + " operand correct",
        )

    _destroy_runtime(w, rt)


fn test_stress_register_remove_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Register and remove handlers in a tight loop to exercise free list."""
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    for _ in range(500):
        var id = Int(
            w[].call_i32(
                "handler_register_noop",
                args_ptr_i32_ptr(rt, s, w[].write_string_struct("click")),
            )
        )
        assert_equal(
            Int(w[].call_i32("handler_contains", args_ptr_i32(rt, id))), 1
        )
        w[].call_void("handler_remove", args_ptr_i32(rt, id))

    assert_equal(
        Int(w[].call_i32("handler_count", args_ptr(rt))),
        0,
        "count is 0 after 500 register/remove cycles",
    )

    # The next alloc should reuse slot 0
    var id = Int(
        w[].call_i32(
            "handler_register_noop",
            args_ptr_i32_ptr(rt, s, w[].write_string_struct("click")),
        )
    )
    assert_equal(id, 0, "first slot reused after cycle")
    assert_equal(Int(w[].call_i32("handler_count", args_ptr(rt))), 1)

    _destroy_runtime(w, rt)


# ── Dispatch — full counter scenario ─────────────────────────────────────────


fn test_dispatch_counter_scenario(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Simulate a counter component: scope with signal + click handler."""
    var rt = _create_runtime(w)

    # Create scope and signal
    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var count = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0)))

    # Render — subscribe scope to signal
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, key))
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Register increment handler
    var inc_handler = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, key, 1, w[].write_string_struct("click")
            ),
        )
    )
    # Register decrement handler
    var dec_handler = Int(
        w[].call_i32(
            "handler_register_signal_sub",
            args_ptr_i32_i32_i32_ptr(
                rt, s, key, 1, w[].write_string_struct("click")
            ),
        )
    )

    # Click + 3 times
    for _ in range(3):
        _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, inc_handler, 0))

    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))),
        3,
        "count is 3 after 3 increments",
    )

    # Click - 1 time
    _ = w[].call_i32("dispatch_event", args_ptr_i32_i32(rt, dec_handler, 0))
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, key))),
        2,
        "count is 2 after decrement",
    )

    # Verify scope was dirtied
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "scope is dirty",
    )

    # Drain dirty and re-render
    _ = w[].call_i32("runtime_drain_dirty", args_ptr(rt))
    prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))
    var key2 = Int(w[].call_i32("hook_use_signal_i32", args_ptr_i32(rt, 0)))
    assert_equal(key2, key, "same signal key on re-render")
    assert_equal(
        Int(w[].call_i32("signal_read_i32", args_ptr_i32(rt, key2))),
        2,
        "count is 2 on re-render",
    )
    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    _destroy_runtime(w, rt)


# ── Phase 20 — dispatch_event_with_string (SignalString) ─────────────────────


fn test_query_signal_set_string_fields(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Verify handler fields for ACTION_SIGNAL_SET_STRING (action=6)."""
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # Create a string signal → returns packed i64 (low=string_key, high=version_key)
    var packed = w[].call_i64(
        "signal_create_string",
        args_ptr_ptr(rt, w[].write_string_struct("")),
    )
    var string_key = Int(w[].call_i32("signal_string_key", args_i64(packed)))
    var version_key = Int(w[].call_i32("signal_version_key", args_i64(packed)))

    # Register a set_string handler
    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_string",
            args_ptr_i32_i32_i32_ptr(
                rt,
                s,
                string_key,
                version_key,
                w[].write_string_struct("input"),
            ),
        )
    )

    # ACTION_SIGNAL_SET_STRING = 6
    assert_equal(
        Int(w[].call_i32("handler_action", args_ptr_i32(rt, id))),
        6,
        "action is SET_STRING (6)",
    )
    # signal_key stores the string_key
    assert_equal(
        Int(w[].call_i32("handler_signal_key", args_ptr_i32(rt, id))),
        string_key,
        "signal_key holds string_key",
    )
    # operand stores the version_key
    assert_equal(
        Int(w[].call_i32("handler_operand", args_ptr_i32(rt, id))),
        version_key,
        "operand holds version_key",
    )

    _destroy_runtime(w, rt)


fn test_dispatch_signal_set_string(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Dispatch_event_with_string writes the string to the SignalString."""
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # Create a string signal with initial value "initial"
    var packed = w[].call_i64(
        "signal_create_string",
        args_ptr_ptr(rt, w[].write_string_struct("initial")),
    )
    var string_key = Int(w[].call_i32("signal_string_key", args_i64(packed)))
    var version_key = Int(w[].call_i32("signal_version_key", args_i64(packed)))

    # Register a set_string handler
    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_string",
            args_ptr_i32_i32_i32_ptr(
                rt,
                s,
                string_key,
                version_key,
                w[].write_string_struct("input"),
            ),
        )
    )

    # Dispatch with string payload "hello world"
    var result = Int(
        w[].call_i32(
            "dispatch_event_with_string",
            args_ptr_i32_i32_ptr(
                rt, id, 1, w[].write_string_struct("hello world")
            ),
        )
    )
    assert_equal(result, 1, "dispatch returns 1 (action executed)")

    # Read back the string signal value
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "signal_peek_string", args_ptr_i32_ptr(rt, string_key, out_ptr)
    )
    assert_equal(
        w[].read_string_struct(out_ptr),
        String("hello world"),
        "string signal set to dispatched value",
    )

    _destroy_runtime(w, rt)


fn test_dispatch_signal_set_string_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Dispatch_event_with_string handles empty string payload."""
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    var packed = w[].call_i64(
        "signal_create_string",
        args_ptr_ptr(rt, w[].write_string_struct("not empty")),
    )
    var string_key = Int(w[].call_i32("signal_string_key", args_i64(packed)))
    var version_key = Int(w[].call_i32("signal_version_key", args_i64(packed)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_string",
            args_ptr_i32_i32_i32_ptr(
                rt,
                s,
                string_key,
                version_key,
                w[].write_string_struct("input"),
            ),
        )
    )

    # Dispatch with empty string
    var result = Int(
        w[].call_i32(
            "dispatch_event_with_string",
            args_ptr_i32_i32_ptr(rt, id, 1, w[].write_string_struct("")),
        )
    )
    assert_equal(result, 1, "dispatch returns 1")

    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "signal_peek_string", args_ptr_i32_ptr(rt, string_key, out_ptr)
    )
    assert_equal(
        w[].read_string_struct(out_ptr),
        String(""),
        "string signal set to empty string",
    )

    _destroy_runtime(w, rt)


fn test_dispatch_signal_set_string_overwrite(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Multiple dispatches overwrite the SignalString value each time."""
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    var packed = w[].call_i64(
        "signal_create_string",
        args_ptr_ptr(rt, w[].write_string_struct("v0")),
    )
    var string_key = Int(w[].call_i32("signal_string_key", args_i64(packed)))
    var version_key = Int(w[].call_i32("signal_version_key", args_i64(packed)))

    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_string",
            args_ptr_i32_i32_i32_ptr(
                rt,
                s,
                string_key,
                version_key,
                w[].write_string_struct("input"),
            ),
        )
    )

    # Record initial version
    var v0 = Int(w[].call_i32("signal_version", args_ptr_i32(rt, version_key)))

    # Dispatch first value
    _ = w[].call_i32(
        "dispatch_event_with_string",
        args_ptr_i32_i32_ptr(rt, id, 1, w[].write_string_struct("first")),
    )
    var v1 = Int(w[].call_i32("signal_version", args_ptr_i32(rt, version_key)))
    assert_true(v1 > v0, "version bumped after first dispatch")

    # Dispatch second value
    _ = w[].call_i32(
        "dispatch_event_with_string",
        args_ptr_i32_i32_ptr(rt, id, 1, w[].write_string_struct("second")),
    )
    var v2 = Int(w[].call_i32("signal_version", args_ptr_i32(rt, version_key)))
    assert_true(v2 > v1, "version bumped after second dispatch")

    # Final value should be "second"
    var out_ptr = w[].alloc_string_struct()
    w[].call_void(
        "signal_peek_string", args_ptr_i32_ptr(rt, string_key, out_ptr)
    )
    assert_equal(
        w[].read_string_struct(out_ptr),
        String("second"),
        "string signal has last dispatched value",
    )

    _destroy_runtime(w, rt)


fn test_dispatch_signal_set_string_marks_scope_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Dispatch_event_with_string marks subscribing scopes dirty via version signal.
    """
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))

    # Begin render so we can subscribe
    var prev = Int(w[].call_i32("scope_begin_render", args_ptr_i32(rt, s)))

    var packed = w[].call_i64(
        "signal_create_string",
        args_ptr_ptr(rt, w[].write_string_struct("")),
    )
    var string_key = Int(w[].call_i32("signal_string_key", args_i64(packed)))
    var version_key = Int(w[].call_i32("signal_version_key", args_i64(packed)))

    # Subscribe scope by reading the version signal
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, version_key))

    w[].call_void("scope_end_render", args_ptr_i32(rt, prev))

    # Not dirty yet
    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        0,
        "no dirty scopes before dispatch",
    )

    # Register handler (outside render bracket — no subscription needed for handler)
    var id = Int(
        w[].call_i32(
            "handler_register_signal_set_string",
            args_ptr_i32_i32_i32_ptr(
                rt,
                s,
                string_key,
                version_key,
                w[].write_string_struct("input"),
            ),
        )
    )

    # Dispatch — should write string + bump version → dirty scope
    _ = w[].call_i32(
        "dispatch_event_with_string",
        args_ptr_i32_i32_ptr(rt, id, 1, w[].write_string_struct("typed text")),
    )

    assert_equal(
        Int(w[].call_i32("runtime_has_dirty", args_ptr(rt))),
        1,
        "scope is dirty after string dispatch",
    )

    _destroy_runtime(w, rt)


fn test_dispatch_signal_set_string_fallback(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Dispatch_event_with_string falls back to normal dispatch for non-string actions.
    """
    var rt = _create_runtime(w)

    var s = Int(w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1)))
    var sig = Int(w[].call_i32("signal_create_i32", args_ptr_i32(rt, 10)))

    # Register an ADD handler (not a string handler)
    var id = Int(
        w[].call_i32(
            "handler_register_signal_add",
            args_ptr_i32_i32_i32_ptr(
                rt, s, sig, 5, w[].write_string_struct("click")
            ),
        )
    )

    # Call dispatch_event_with_string on a non-string handler → should fall back
    var result = Int(
        w[].call_i32(
            "dispatch_event_with_string",
            args_ptr_i32_i32_ptr(rt, id, 0, w[].write_string_struct("ignored")),
        )
    )
    assert_equal(result, 1, "fallback dispatch returns 1 (ADD executed)")

    # Signal should have been incremented by 5 (normal ADD action)
    assert_equal(
        Int(w[].call_i32("signal_peek_i32", args_ptr_i32(rt, sig))),
        15,
        "signal is 15 after fallback ADD (10 + 5)",
    )

    _destroy_runtime(w, rt)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_registry_initial_state(w)
    test_register_single_handler(w)
    test_register_multiple_handlers(w)
    test_query_signal_add_fields(w)
    test_query_signal_set_fields(w)
    test_query_signal_sub_fields(w)
    test_query_signal_toggle_fields(w)
    test_query_signal_set_input_fields(w)
    test_query_custom_handler_fields(w)
    test_query_noop_handler_fields(w)
    test_remove_handler(w)
    test_remove_nonexistent_is_noop(w)
    test_double_remove_is_noop(w)
    test_slot_reuse_after_remove(w)
    test_multiple_slot_reuse(w)
    test_contains_out_of_bounds(w)
    test_dispatch_signal_add(w)
    test_dispatch_signal_sub(w)
    test_dispatch_signal_set(w)
    test_dispatch_signal_toggle(w)
    test_dispatch_signal_set_input(w)
    test_dispatch_marks_scope_dirty(w)
    test_dispatch_multiple_accumulate(w)
    test_dispatch_and_drain_dirty(w)
    test_negative_operand(w)
    test_int32_min_max_operand(w)
    test_stress_100_handlers(w)
    test_stress_register_remove_cycle(w)
    test_dispatch_counter_scenario(w)
    # Phase 20 — dispatch_event_with_string
    test_query_signal_set_string_fields(w)
    test_dispatch_signal_set_string(w)
    test_dispatch_signal_set_string_empty(w)
    test_dispatch_signal_set_string_overwrite(w)
    test_dispatch_signal_set_string_marks_scope_dirty(w)
    test_dispatch_signal_set_string_fallback(w)
    print("events: 35/35 passed")
