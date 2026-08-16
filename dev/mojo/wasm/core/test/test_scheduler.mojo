from memory import UnsafePointer

# Tests for Phase 10.4 — Scheduler (height-ordered dirty scope queue).
#
# Validates:
#   - Scheduler create/destroy lifecycle
#   - collect() drains runtime dirty queue into scheduler
#   - collect_one() adds individual scopes
#   - next() returns scopes in height-first order (shallowest first)
#   - Deduplication: same scope ID not queued twice
#   - clear() discards all pending scopes
#   - has_scope() checks membership
#   - Empty scheduler returns is_empty = 1

from testing import assert_equal, assert_true, assert_false
from wasm_harness import (
    WasmInstance,
    no_args,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_ptr,
    args_ptr_ptr_i32,
)


fn _load() raises -> WasmInstance:
    return WasmInstance("build/out.wasm")


# ── Scheduler lifecycle ──────────────────────────────────────────────────────


def test_scheduler_create_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var sched = Int(w[].call_i64("scheduler_create", no_args()))
    assert_true(sched != 0, "scheduler pointer should be non-zero")
    w[].call_void("scheduler_destroy", args_ptr(sched))


def test_scheduler_initially_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var sched = Int(w[].call_i64("scheduler_create", no_args()))
    assert_equal(w[].call_i32("scheduler_is_empty", args_ptr(sched)), 1)
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 0)
    w[].call_void("scheduler_destroy", args_ptr(sched))


# ── collect_one and next ─────────────────────────────────────────────────────


def test_scheduler_collect_one_and_next(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    # Create a scope at height 0
    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    # Manually add it
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    assert_equal(w[].call_i32("scheduler_is_empty", args_ptr(sched)), 0)
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 1)
    assert_equal(
        w[].call_i32("scheduler_has_scope", args_ptr_i32(sched, s0)), 1
    )

    # Pop it
    var got = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(got, s0)
    assert_equal(w[].call_i32("scheduler_is_empty", args_ptr(sched)), 1)

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── Height-ordered processing ────────────────────────────────────────────────


def test_scheduler_height_ordering(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Scopes are yielded shallowest (lowest height) first."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    # Create scopes at different heights
    var root = w[].call_i32(
        "scope_create", args_ptr_i32_i32(rt, 0, -1)
    )  # height 0
    var child = w[].call_i32(
        "scope_create_child", args_ptr_i32(rt, root)
    )  # height 1
    var grandchild = w[].call_i32(
        "scope_create_child", args_ptr_i32(rt, child)
    )  # height 2

    # Add in REVERSE order (grandchild first)
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, grandchild)
    )
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, child))
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, root))
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 3)

    # Should come out in height order: root, child, grandchild
    var first = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(first, root, "first should be root (height 0)")

    var second = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(second, child, "second should be child (height 1)")

    var third = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(third, grandchild, "third should be grandchild (height 2)")

    assert_equal(w[].call_i32("scheduler_is_empty", args_ptr(sched)), 1)

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


def test_scheduler_same_height_preserves_order(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Scopes at the same height are yielded in insertion order (stable sort).
    """
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    # Create 3 scopes all at height 0
    var s1 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    var s2 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    var s3 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s1))
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s2))
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s3))

    # Same height → insertion order preserved
    assert_equal(w[].call_i32("scheduler_next", args_ptr(sched)), s1)
    assert_equal(w[].call_i32("scheduler_next", args_ptr(sched)), s2)
    assert_equal(w[].call_i32("scheduler_next", args_ptr(sched)), s3)

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── Deduplication ────────────────────────────────────────────────────────────


def test_scheduler_deduplicates(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Adding the same scope twice does not create duplicates."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0)
    )  # duplicate
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0)
    )  # duplicate

    assert_equal(
        w[].call_i32("scheduler_count", args_ptr(sched)),
        1,
        "should be 1 (deduped)",
    )

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── collect() from runtime dirty queue ───────────────────────────────────────


def test_scheduler_collect_from_dirty_queue(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """collect() drains the runtime's dirty scopes into the scheduler."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    # Create scope and signal, subscribe scope to signal
    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    _ = w[].call_i32("scope_begin_render", args_ptr_i32(rt, s0))
    var sig = w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))  # subscribe
    w[].call_void("scope_end_render", args_ptr_i32(rt, -1))

    # Write to signal → scope becomes dirty in runtime
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, sig, 42))
    assert_equal(w[].call_i32("runtime_has_dirty", args_ptr(rt)), 1)

    # Collect into scheduler
    w[].call_void("scheduler_collect", args_ptr_ptr(sched, rt))

    # Runtime dirty queue should be drained
    assert_equal(w[].call_i32("runtime_has_dirty", args_ptr(rt)), 0)

    # Scheduler should have the scope
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 1)
    assert_equal(
        w[].call_i32("scheduler_has_scope", args_ptr_i32(sched, s0)), 1
    )

    var got = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(got, s0)

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


def test_scheduler_collect_multiple_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """collect() handles multiple dirty scopes and orders by height."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    # Create parent (height 0) and child (height 1)
    var parent = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    var child = w[].call_i32("scope_create_child", args_ptr_i32(rt, parent))

    # Create a signal and subscribe both scopes
    _ = w[].call_i32("scope_begin_render", args_ptr_i32(rt, parent))
    var sig = w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))
    w[].call_void("scope_end_render", args_ptr_i32(rt, -1))

    _ = w[].call_i32("scope_begin_render", args_ptr_i32(rt, child))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))
    w[].call_void("scope_end_render", args_ptr_i32(rt, -1))

    # Write → both scopes dirty
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, sig, 99))

    # Collect
    w[].call_void("scheduler_collect", args_ptr_ptr(sched, rt))
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 2)

    # Should come out parent first (height 0), then child (height 1)
    var first = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(first, parent, "parent (height 0) should be first")

    var second = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(second, child, "child (height 1) should be second")

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── clear() ──────────────────────────────────────────────────────────────────


def test_scheduler_clear(w: UnsafePointer[WasmInstance, MutExternalOrigin]):
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    var s1 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s1))
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 2)

    w[].call_void("scheduler_clear", args_ptr(sched))
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 0)
    assert_equal(w[].call_i32("scheduler_is_empty", args_ptr(sched)), 1)

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── has_scope() ──────────────────────────────────────────────────────────────


def test_scheduler_has_scope(w: UnsafePointer[WasmInstance, MutExternalOrigin]):
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    var s1 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    # s1 not added
    assert_equal(
        w[].call_i32("scheduler_has_scope", args_ptr_i32(sched, s1)), 0
    )

    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    assert_equal(
        w[].call_i32("scheduler_has_scope", args_ptr_i32(sched, s0)), 1
    )
    assert_equal(
        w[].call_i32("scheduler_has_scope", args_ptr_i32(sched, s1)), 0
    )

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── Multiple collect cycles ──────────────────────────────────────────────────


def test_scheduler_multiple_collect_cycles(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Scheduler can be reused across multiple collect/drain cycles."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    # First cycle
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    var got1 = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(got1, s0)
    assert_equal(w[].call_i32("scheduler_is_empty", args_ptr(sched)), 1)

    # Second cycle — same scope can be added again after being drained
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 1)
    var got2 = w[].call_i32("scheduler_next", args_ptr(sched))
    assert_equal(got2, s0)

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── Deep hierarchy ───────────────────────────────────────────────────────────


def test_scheduler_deep_hierarchy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
):
    """Height ordering works with deeper scope trees."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    # Build a chain: root → child → grandchild → great-grandchild
    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))
    var s1 = w[].call_i32("scope_create_child", args_ptr_i32(rt, s0))
    var s2 = w[].call_i32("scope_create_child", args_ptr_i32(rt, s1))
    var s3 = w[].call_i32("scope_create_child", args_ptr_i32(rt, s2))

    # Add in scrambled order
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s2)
    )  # height 2
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0)
    )  # height 0
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s3)
    )  # height 3
    w[].call_void(
        "scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s1)
    )  # height 1

    assert_equal(
        w[].call_i32("scheduler_next", args_ptr(sched)), s0, "height 0 first"
    )
    assert_equal(
        w[].call_i32("scheduler_next", args_ptr(sched)), s1, "height 1 second"
    )
    assert_equal(
        w[].call_i32("scheduler_next", args_ptr(sched)), s2, "height 2 third"
    )
    assert_equal(
        w[].call_i32("scheduler_next", args_ptr(sched)), s3, "height 3 fourth"
    )

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


# ── Collect deduplicates against existing entries ────────────────────────────


def test_scheduler_collect_deduplicates_against_existing(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """collect() does not add scopes already in the queue."""
    var rt = Int(w[].call_i64("runtime_create", no_args()))
    var sched = Int(w[].call_i64("scheduler_create", no_args()))

    var s0 = w[].call_i32("scope_create", args_ptr_i32_i32(rt, 0, -1))

    # Add s0 manually first
    w[].call_void("scheduler_collect_one", args_ptr_ptr_i32(sched, rt, s0))
    assert_equal(w[].call_i32("scheduler_count", args_ptr(sched)), 1)

    # Now subscribe s0 to a signal and make it dirty
    _ = w[].call_i32("scope_begin_render", args_ptr_i32(rt, s0))
    var sig = w[].call_i32("signal_create_i32", args_ptr_i32(rt, 0))
    _ = w[].call_i32("signal_read_i32", args_ptr_i32(rt, sig))
    w[].call_void("scope_end_render", args_ptr_i32(rt, -1))
    w[].call_void("signal_write_i32", args_ptr_i32_i32(rt, sig, 1))

    # Collect from runtime — s0 is already in queue, should be deduped
    w[].call_void("scheduler_collect", args_ptr_ptr(sched, rt))
    assert_equal(
        w[].call_i32("scheduler_count", args_ptr(sched)), 1, "still 1 (deduped)"
    )

    w[].call_void("scheduler_destroy", args_ptr(sched))
    w[].call_void("runtime_destroy", args_ptr(rt))


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_scheduler_create_destroy(w)
    test_scheduler_initially_empty(w)
    test_scheduler_collect_one_and_next(w)
    test_scheduler_height_ordering(w)
    test_scheduler_same_height_preserves_order(w)
    test_scheduler_deduplicates(w)
    test_scheduler_collect_from_dirty_queue(w)
    test_scheduler_collect_multiple_dirty(w)
    test_scheduler_clear(w)
    test_scheduler_has_scope(w)
    test_scheduler_multiple_collect_cycles(w)
    test_scheduler_deep_hierarchy(w)
    test_scheduler_collect_deduplicates_against_existing(w)
    print("scheduler: 13/13 passed")
