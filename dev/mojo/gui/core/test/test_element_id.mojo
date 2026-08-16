# ElementIdAllocator exercised through the real WASM binary via mojo-wasmtime
# (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that the ElementIdAllocator works correctly when compiled
# to WASM and executed via the Wasmtime runtime.
#
# Note: ElementId basic struct tests (is_root, is_valid, equality, from_int,
# ROOT_ELEMENT_ID) and next_id, clear, capacity constructor tests are not
# covered here since those specific APIs lack WASM exports.
#
# Run with:
#   mojo test test/test_element_id.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_alloc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    """Create a heap-allocated ElementIdAllocator via WASM."""
    return Int(w[].call_i64("eid_alloc_create", no_args()))


fn _destroy_alloc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], alloc_ptr: Int
) raises:
    """Destroy a heap-allocated ElementIdAllocator via WASM."""
    w[].call_void("eid_alloc_destroy", args_ptr(alloc_ptr))


# ── Allocator — initial state ────────────────────────────────────────────────


fn test_allocator_initial_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    # Root is pre-reserved, so count=1, user_count=0
    assert_equal(
        Int(w[].call_i32("eid_count", args_ptr(a))),
        1,
        "initial count should be 1 (root)",
    )
    assert_equal(
        Int(w[].call_i32("eid_user_count", args_ptr(a))),
        0,
        "initial user_count should be 0",
    )

    _destroy_alloc(w, a)


# ── Allocator — first alloc returns 1 ────────────────────────────────────────


fn test_allocator_first_alloc_returns_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var id = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    assert_equal(id, 1, "first alloc should return ID 1")
    assert_equal(Int(w[].call_i32("eid_count", args_ptr(a))), 2)
    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 1)

    _destroy_alloc(w, a)


# ── Allocator — sequential alloc ─────────────────────────────────────────────


fn test_allocator_sequential_alloc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var id1 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    var id2 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    var id3 = Int(w[].call_i32("eid_alloc", args_ptr(a)))

    assert_equal(id1, 1)
    assert_equal(id2, 2)
    assert_equal(id3, 3)
    assert_equal(Int(w[].call_i32("eid_count", args_ptr(a))), 4)
    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 3)

    _destroy_alloc(w, a)


# ── Allocator — is_alive ─────────────────────────────────────────────────────


fn test_allocator_is_alive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var id1 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    var id2 = Int(w[].call_i32("eid_alloc", args_ptr(a)))

    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id1))),
        1,
        "id1 should be alive",
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id2))),
        1,
        "id2 should be alive",
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, 0))),
        1,
        "root should be alive",
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, 99))),
        0,
        "unallocated ID should not be alive",
    )

    _destroy_alloc(w, a)


# ── Allocator — free decrements count ────────────────────────────────────────


fn test_allocator_free_decrements_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var id1 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    var id2 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 2)

    w[].call_void("eid_free", args_ptr_i32(a, id1))
    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 1)
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id1))),
        0,
        "freed ID should not be alive",
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id2))),
        1,
        "other ID should still be alive",
    )

    _destroy_alloc(w, a)


# ── Allocator — free root is noop ────────────────────────────────────────────


fn test_allocator_free_root_is_noop(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    _ = w[].call_i32("eid_alloc", args_ptr(a))
    var count_before = Int(w[].call_i32("eid_count", args_ptr(a)))

    w[].call_void("eid_free", args_ptr_i32(a, 0))
    assert_equal(
        Int(w[].call_i32("eid_count", args_ptr(a))),
        count_before,
        "freeing root should not change count",
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, 0))),
        1,
        "root should still be alive after free attempt",
    )

    _destroy_alloc(w, a)


# ── Allocator — double free is noop ──────────────────────────────────────────


fn test_allocator_double_free_is_noop(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var id = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    w[].call_void("eid_free", args_ptr_i32(a, id))
    var count_after_first_free = Int(w[].call_i32("eid_count", args_ptr(a)))

    w[].call_void("eid_free", args_ptr_i32(a, id))  # double free
    assert_equal(
        Int(w[].call_i32("eid_count", args_ptr(a))),
        count_after_first_free,
        "double free should not change count",
    )

    _destroy_alloc(w, a)


# ── Allocator — reuse freed slot ─────────────────────────────────────────────


fn test_allocator_reuse_freed_slot(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var id1 = Int(w[].call_i32("eid_alloc", args_ptr(a)))  # gets 1
    var id2 = Int(w[].call_i32("eid_alloc", args_ptr(a)))  # gets 2

    w[].call_void("eid_free", args_ptr_i32(a, id1))  # free slot 1

    var id3 = Int(w[].call_i32("eid_alloc", args_ptr(a)))  # should reuse slot 1
    assert_equal(id3, id1, "new alloc should reuse freed slot")
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id3))),
        1,
        "reused ID should be alive",
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id2))),
        1,
        "other ID should still be alive",
    )

    _destroy_alloc(w, a)


# ── Allocator — reuse multiple freed slots ───────────────────────────────────


fn test_allocator_reuse_multiple_freed_slots(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var a = _create_alloc(w)

    var id1 = Int(w[].call_i32("eid_alloc", args_ptr(a)))  # 1
    _ = w[].call_i32("eid_alloc", args_ptr(a))  # 2
    var id3 = Int(w[].call_i32("eid_alloc", args_ptr(a)))  # 3

    w[].call_void("eid_free", args_ptr_i32(a, id1))
    w[].call_void("eid_free", args_ptr_i32(a, id3))
    assert_equal(
        Int(w[].call_i32("eid_user_count", args_ptr(a))),
        1,
        "only id2 should remain",
    )

    # Allocate two more — should reuse freed slots (LIFO order from free list)
    var id4 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    var id5 = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id4))),
        1,
    )
    assert_equal(
        Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id5))),
        1,
    )
    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 3)

    # The reused IDs should be from {1, 3}
    assert_true(
        (id4 == 1 and id5 == 3) or (id4 == 3 and id5 == 1),
        "should reuse freed slots 1 and 3",
    )

    _destroy_alloc(w, a)


# ── Stress — many allocations ────────────────────────────────────────────────


fn test_allocator_stress_100(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = _create_alloc(w)

    var ids = List[Int]()
    for i in range(100):
        ids.append(Int(w[].call_i32("eid_alloc", args_ptr(a))))

    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 100)

    # All IDs should be unique and alive
    for i in range(100):
        assert_equal(
            Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, ids[i]))),
            1,
            "id " + String(i) + " is alive",
        )
        assert_equal(ids[i], i + 1, "id " + String(i) + " == " + String(i + 1))

    _destroy_alloc(w, a)


# ── Stress — free even and realloc ───────────────────────────────────────────


fn test_allocator_stress_free_even_realloc(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var a = _create_alloc(w)

    var ids = List[Int]()
    # Allocate 50
    for i in range(50):
        ids.append(Int(w[].call_i32("eid_alloc", args_ptr(a))))

    # Free all even-indexed (0, 2, 4, ...)
    for i in range(0, 50, 2):
        w[].call_void("eid_free", args_ptr_i32(a, ids[i]))

    assert_equal(
        Int(w[].call_i32("eid_user_count", args_ptr(a))),
        25,
        "25 should remain after freeing 25",
    )

    # Allocate 25 more — should reuse freed slots
    var new_ids = List[Int]()
    for i in range(25):
        new_ids.append(Int(w[].call_i32("eid_alloc", args_ptr(a))))

    assert_equal(
        Int(w[].call_i32("eid_user_count", args_ptr(a))),
        50,
        "back to 50 after reallocating",
    )

    # All new IDs should be alive
    for i in range(25):
        assert_equal(
            Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, new_ids[i]))),
            1,
            "new id " + String(i) + " is alive",
        )

    # Original odd-indexed should still be alive with correct values
    for i in range(1, 50, 2):
        assert_equal(
            Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, ids[i]))),
            1,
            "original odd id " + String(i) + " still alive",
        )
        assert_equal(ids[i], i + 1)

    _destroy_alloc(w, a)


# ── Stress — alloc/free cycle ────────────────────────────────────────────────


fn test_allocator_stress_alloc_free_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Allocate and free in a tight loop to exercise slot reuse."""
    var a = _create_alloc(w)

    for _ in range(1000):
        var id = Int(w[].call_i32("eid_alloc", args_ptr(a)))
        assert_equal(
            Int(w[].call_i32("eid_is_alive", args_ptr_i32(a, id))),
            1,
        )
        w[].call_void("eid_free", args_ptr_i32(a, id))

    # After all cycles, should be back to just root
    assert_equal(Int(w[].call_i32("eid_user_count", args_ptr(a))), 0)
    # But one slot was created and is on the free list
    assert_equal(Int(w[].call_i32("eid_count", args_ptr(a))), 1)

    # Next alloc should reuse that slot
    var id = Int(w[].call_i32("eid_alloc", args_ptr(a)))
    assert_equal(id, 1, "first alloc after cycle should reuse slot 1")

    _destroy_alloc(w, a)


# ── Debug — capacity query ───────────────────────────────────────────────────


fn test_debug_eid_alloc_capacity(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """debug_eid_alloc_capacity returns the total number of slots."""
    var a = _create_alloc(w)

    # Initial capacity: at least 1 slot (root is pre-reserved)
    var cap0 = Int(w[].call_i32("debug_eid_alloc_capacity", args_ptr(a)))
    assert_true(cap0 >= 1, "initial capacity should be >= 1")

    # Allocate several IDs — capacity should grow to accommodate them
    for _ in range(10):
        _ = w[].call_i32("eid_alloc", args_ptr(a))

    var cap1 = Int(w[].call_i32("debug_eid_alloc_capacity", args_ptr(a)))
    assert_true(
        cap1 >= 11,
        "capacity should be >= 11 after 10 allocs (root + 10)",
    )
    assert_true(cap1 >= cap0, "capacity should not shrink")

    _destroy_alloc(w, a)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_allocator_initial_state(w)
    test_allocator_first_alloc_returns_1(w)
    test_allocator_sequential_alloc(w)
    test_allocator_is_alive(w)
    test_allocator_free_decrements_count(w)
    test_allocator_free_root_is_noop(w)
    test_allocator_double_free_is_noop(w)
    test_allocator_reuse_freed_slot(w)
    test_allocator_reuse_multiple_freed_slots(w)
    test_allocator_stress_100(w)
    test_allocator_stress_free_even_realloc(w)
    test_allocator_stress_alloc_free_cycle(w)
    test_debug_eid_alloc_capacity(w)
    print("element_id: 13/13 passed")
