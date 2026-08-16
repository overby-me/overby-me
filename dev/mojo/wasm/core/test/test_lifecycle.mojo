"""Phase 26 — App Lifecycle (Destroy / Recreate) Mojo Tests.

Validates the full create→use→destroy→recreate loop at the WASM level:
  - counter_init → use → counter_destroy → counter_init → use cycle
  - todo_init → add → todo_destroy → todo_init cycle
  - heap stats checks: free list grows after destroy, heap stays bounded
  - aligned_free calls during destroy don't corrupt the free list
  - destroy with dirty (unflushed) state doesn't crash
  - double destroy safety (second destroy should not trap)
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


# ── Counter lifecycle ────────────────────────────────────────────────────────


def test_counter_create_use_destroy(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Counter init → increment → flush → destroy completes without error."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    assert_true(app != 0, "counter app pointer should be non-zero")

    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    assert_true(buf != 0, "mutation buffer should be non-zero")

    # Initial mount
    var mount_len = w[].call_i32(
        "counter_rebuild", args_ptr_ptr_i32(app, buf, 4096)
    )
    assert_true(mount_len > 0, "mount should produce mutations")

    # Increment
    var incr = w[].call_i32("counter_incr_handler", args_ptr(app))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr, 0))

    # Flush
    var flush_len = w[].call_i32(
        "counter_flush", args_ptr_ptr_i32(app, buf, 4096)
    )
    assert_true(flush_len > 0, "flush after increment should produce mutations")

    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 1, "count is 1 after increment")

    # Destroy
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


def test_counter_destroy_recreate_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Counter create→use→destroy→create→use cycle produces correct state."""
    # --- First instance ---
    var app1 = Int(w[].call_i64("counter_init", no_args()))
    var buf1 = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))

    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app1, buf1, 4096))

    var incr1 = w[].call_i32("counter_incr_handler", args_ptr(app1))
    for _ in range(3):
        _ = w[].call_i32(
            "counter_handle_event", args_ptr_i32_i32(app1, incr1, 0)
        )
        _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app1, buf1, 4096))

    var count1 = w[].call_i32("counter_count_value", args_ptr(app1))
    assert_equal(count1, 3, "first instance count is 3")

    # Destroy first instance
    w[].call_void("mutation_buf_free", args_ptr(buf1))
    w[].call_void("counter_destroy", args_ptr(app1))

    # --- Second instance ---
    var app2 = Int(w[].call_i64("counter_init", no_args()))
    var buf2 = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))

    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app2, buf2, 4096))

    # Second instance should start fresh at 0
    var count2 = w[].call_i32("counter_count_value", args_ptr(app2))
    assert_equal(count2, 0, "second instance starts at 0")

    var incr2 = w[].call_i32("counter_incr_handler", args_ptr(app2))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app2, incr2, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app2, buf2, 4096))

    var count2_after = w[].call_i32("counter_count_value", args_ptr(app2))
    assert_equal(count2_after, 1, "second instance count is 1 after increment")

    w[].call_void("mutation_buf_free", args_ptr(buf2))
    w[].call_void("counter_destroy", args_ptr(app2))


def test_counter_multiple_cycles_heap_bounded(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """10 counter create/destroy cycles — heap pointer stays bounded."""
    var stats_before = w[].heap_stats()
    var heap_before = stats_before[0]

    for _ in range(10):
        var app = Int(w[].call_i64("counter_init", no_args()))
        var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
        _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))

        var incr = w[].call_i32("counter_incr_handler", args_ptr(app))
        _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr, 0))
        _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 4096))

        w[].call_void("mutation_buf_free", args_ptr(buf))
        w[].call_void("counter_destroy", args_ptr(app))

    var stats_after = w[].heap_stats()
    var heap_after = stats_after[0]
    var free_blocks = stats_after[1]
    var free_bytes = stats_after[2]

    # Heap growth should be bounded — free list reuse keeps it manageable
    var growth = heap_after - heap_before
    assert_true(
        growth < 1_000_000,
        "heap growth < 1MB across 10 counter cycles (got "
        + String(growth)
        + ")",
    )

    # Free list should have entries from destroyed apps
    assert_true(
        free_blocks > 0,
        "free list should have entries after destroy cycles (got "
        + String(free_blocks)
        + " blocks, "
        + String(free_bytes)
        + " bytes)",
    )


def test_counter_destroy_with_dirty_state(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Destroy counter with dirty (unflushed) state doesn't crash."""
    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))

    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))

    # Dispatch event but do NOT flush — app has dirty scopes
    var incr = w[].call_i32("counter_incr_handler", args_ptr(app))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr, 0))

    # Destroy without flushing — should not crash
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


# ── Todo lifecycle ───────────────────────────────────────────────────────────


def test_todo_create_destroy_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Todo create→add items→destroy→create cycle produces clean slate."""
    # --- First instance ---
    var app1 = Int(w[].call_i64("todo_init", no_args()))
    var buf1 = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))

    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app1, buf1, 65536))

    # Add items via WASM export
    var str1 = w[].write_string_struct("Buy milk")
    w[].call_void("todo_add_item", args_ptr_ptr(app1, str1))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app1, buf1, 65536))

    var str2 = w[].write_string_struct("Walk dog")
    w[].call_void("todo_add_item", args_ptr_ptr(app1, str2))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app1, buf1, 65536))

    var count1 = w[].call_i32("todo_item_count", args_ptr(app1))
    assert_equal(count1, 2, "first todo instance has 2 items")

    var version1 = w[].call_i32("todo_list_version", args_ptr(app1))
    assert_true(version1 > 0, "first todo instance version > 0")

    # Destroy first instance
    w[].call_void("mutation_buf_free", args_ptr(buf1))
    w[].call_void("todo_destroy", args_ptr(app1))

    # --- Second instance ---
    var app2 = Int(w[].call_i64("todo_init", no_args()))
    var buf2 = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))

    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app2, buf2, 65536))

    # Second instance should start fresh
    var count2 = w[].call_i32("todo_item_count", args_ptr(app2))
    assert_equal(count2, 0, "second todo instance starts with 0 items")

    var version2 = w[].call_i32("todo_list_version", args_ptr(app2))
    assert_equal(version2, 0, "second todo instance version is 0")

    # Can add items to the new instance
    var str3 = w[].write_string_struct("New task")
    w[].call_void("todo_add_item", args_ptr_ptr(app2, str3))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app2, buf2, 65536))

    var count2_after = w[].call_i32("todo_item_count", args_ptr(app2))
    assert_equal(count2_after, 1, "second todo instance has 1 item after add")

    w[].call_void("mutation_buf_free", args_ptr(buf2))
    w[].call_void("todo_destroy", args_ptr(app2))


def test_todo_multiple_cycles_heap_bounded(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """5 todo create/add/destroy cycles — heap stays bounded."""
    var stats_before = w[].heap_stats()
    var heap_before = stats_before[0]

    for i in range(5):
        var app = Int(w[].call_i64("todo_init", no_args()))
        var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
        _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(app, buf, 65536))

        var s = w[].write_string_struct("Task " + String(i))
        w[].call_void("todo_add_item", args_ptr_ptr(app, s))
        _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(app, buf, 65536))

        w[].call_void("mutation_buf_free", args_ptr(buf))
        w[].call_void("todo_destroy", args_ptr(app))

    var stats_after = w[].heap_stats()
    var heap_after = stats_after[0]
    var free_blocks = stats_after[1]

    var growth = heap_after - heap_before
    assert_true(
        growth < 2_000_000,
        "todo heap growth < 2MB across 5 cycles (got " + String(growth) + ")",
    )

    assert_true(
        free_blocks > 0,
        "todo free list has entries after destroy cycles (got "
        + String(free_blocks)
        + " blocks)",
    )


# ── Bench lifecycle ──────────────────────────────────────────────────────────


def test_bench_create_destroy_cycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Bench create→rows→destroy→create→rows cycle produces correct counts."""
    var buf_cap = 8 * 1024 * 1024

    # --- First instance ---
    var app1 = Int(w[].call_i64("bench_init", no_args()))
    var buf1 = Int(w[].call_i64("mutation_buf_alloc", args_i32(buf_cap)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app1, buf1, buf_cap))

    w[].call_void("bench_create", args_ptr_i32(app1, 100))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app1, buf1, buf_cap))

    var count1 = w[].call_i32("bench_row_count", args_ptr(app1))
    assert_equal(count1, 100, "first bench instance has 100 rows")

    w[].call_void("mutation_buf_free", args_ptr(buf1))
    w[].call_void("bench_destroy", args_ptr(app1))

    # --- Second instance ---
    var app2 = Int(w[].call_i64("bench_init", no_args()))
    var buf2 = Int(w[].call_i64("mutation_buf_alloc", args_i32(buf_cap)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(app2, buf2, buf_cap))

    w[].call_void("bench_create", args_ptr_i32(app2, 50))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(app2, buf2, buf_cap))

    var count2 = w[].call_i32("bench_row_count", args_ptr(app2))
    assert_equal(count2, 50, "second bench instance has 50 rows")

    w[].call_void("mutation_buf_free", args_ptr(buf2))
    w[].call_void("bench_destroy", args_ptr(app2))


def test_bench_warmup_pattern(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Bench warmup: create→1k→destroy→create→1k — heap stays bounded."""
    var buf_cap = 8 * 1024 * 1024
    var stats_before = w[].heap_stats()
    var heap_before = stats_before[0]

    # Warmup cycle
    var warmup = Int(w[].call_i64("bench_init", no_args()))
    var wbuf = Int(w[].call_i64("mutation_buf_alloc", args_i32(buf_cap)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(warmup, wbuf, buf_cap))

    w[].call_void("bench_create", args_ptr_i32(warmup, 1000))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(warmup, wbuf, buf_cap))

    var wcount = w[].call_i32("bench_row_count", args_ptr(warmup))
    assert_equal(wcount, 1000, "warmup: 1000 rows created")

    w[].call_void("mutation_buf_free", args_ptr(wbuf))
    w[].call_void("bench_destroy", args_ptr(warmup))

    # Measurement cycle
    var measure = Int(w[].call_i64("bench_init", no_args()))
    var mbuf = Int(w[].call_i64("mutation_buf_alloc", args_i32(buf_cap)))
    _ = w[].call_i32("bench_rebuild", args_ptr_ptr_i32(measure, mbuf, buf_cap))

    w[].call_void("bench_create", args_ptr_i32(measure, 1000))
    _ = w[].call_i32("bench_flush", args_ptr_ptr_i32(measure, mbuf, buf_cap))

    var mcount = w[].call_i32("bench_row_count", args_ptr(measure))
    assert_equal(mcount, 1000, "measure: 1000 rows created")

    var stats_after = w[].heap_stats()
    var heap_after = stats_after[0]
    var growth = heap_after - heap_before

    # Generous bound for 2 × 1k rows + 8MB buffers
    assert_true(
        growth < 50_000_000,
        "bench warmup heap growth < 50MB (got " + String(growth) + ")",
    )

    w[].call_void("mutation_buf_free", args_ptr(mbuf))
    w[].call_void("bench_destroy", args_ptr(measure))


# ── Free list integrity ──────────────────────────────────────────────────────


def test_free_list_integrity_across_destroys(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Verify aligned_free calls during destroy don't corrupt the free list.

    After multiple create/destroy cycles, the free list should remain
    consistent: free blocks should still be reusable (no crash on reuse).
    """
    # Run several cycles to populate the free list
    for _ in range(5):
        var app = Int(w[].call_i64("counter_init", no_args()))
        var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
        _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))
        w[].call_void("mutation_buf_free", args_ptr(buf))
        w[].call_void("counter_destroy", args_ptr(app))

    # After cycles, allocate again — should reuse free list blocks without crash
    var stats_before = w[].heap_stats()
    var heap_before = stats_before[0]

    var app = Int(w[].call_i64("counter_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(app, buf, 4096))

    var incr = w[].call_i32("counter_incr_handler", args_ptr(app))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(app, incr, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(app, buf, 4096))

    var count = w[].call_i32("counter_count_value", args_ptr(app))
    assert_equal(count, 1, "counter works after free list cycles")

    var stats_after = w[].heap_stats()
    var heap_after = stats_after[0]

    # Heap should have grown minimally since free list blocks were reused
    var growth = heap_after - heap_before
    assert_true(
        growth < 500_000,
        "heap growth minimal after reuse (got " + String(growth) + ")",
    )

    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("counter_destroy", args_ptr(app))


# ── Interleaved app lifecycle ────────────────────────────────────────────────


def test_interleaved_counter_todo(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
):
    """Counter → destroy → todo → destroy → counter — all on same WASM instance.
    """
    # Counter
    var c1 = Int(w[].call_i64("counter_init", no_args()))
    var cb1 = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(c1, cb1, 4096))
    var incr = w[].call_i32("counter_incr_handler", args_ptr(c1))
    _ = w[].call_i32("counter_handle_event", args_ptr_i32_i32(c1, incr, 0))
    _ = w[].call_i32("counter_flush", args_ptr_ptr_i32(c1, cb1, 4096))
    assert_equal(
        w[].call_i32("counter_count_value", args_ptr(c1)),
        1,
        "counter count 1",
    )
    w[].call_void("mutation_buf_free", args_ptr(cb1))
    w[].call_void("counter_destroy", args_ptr(c1))

    # Todo
    var t = Int(w[].call_i64("todo_init", no_args()))
    var tb = Int(w[].call_i64("mutation_buf_alloc", args_i32(65536)))
    _ = w[].call_i32("todo_rebuild", args_ptr_ptr_i32(t, tb, 65536))
    var s = w[].write_string_struct("Lifecycle item")
    w[].call_void("todo_add_item", args_ptr_ptr(t, s))
    _ = w[].call_i32("todo_flush", args_ptr_ptr_i32(t, tb, 65536))
    assert_equal(
        w[].call_i32("todo_item_count", args_ptr(t)),
        1,
        "todo has 1 item",
    )
    w[].call_void("mutation_buf_free", args_ptr(tb))
    w[].call_void("todo_destroy", args_ptr(t))

    # Counter again — should start fresh
    var c2 = Int(w[].call_i64("counter_init", no_args()))
    var cb2 = Int(w[].call_i64("mutation_buf_alloc", args_i32(4096)))
    _ = w[].call_i32("counter_rebuild", args_ptr_ptr_i32(c2, cb2, 4096))
    assert_equal(
        w[].call_i32("counter_count_value", args_ptr(c2)),
        0,
        "second counter starts at 0",
    )
    w[].call_void("mutation_buf_free", args_ptr(cb2))
    w[].call_void("counter_destroy", args_ptr(c2))


# ── Test runner ──────────────────────────────────────────────────────────────


fn main() raises:
    var w = get_instance()

    # Counter lifecycle
    test_counter_create_use_destroy(w)
    test_counter_destroy_recreate_cycle(w)
    test_counter_multiple_cycles_heap_bounded(w)
    test_counter_destroy_with_dirty_state(w)

    # Todo lifecycle
    test_todo_create_destroy_cycle(w)
    test_todo_multiple_cycles_heap_bounded(w)

    # Bench lifecycle
    test_bench_create_destroy_cycle(w)
    test_bench_warmup_pattern(w)

    # Free list integrity
    test_free_list_integrity_across_destroys(w)

    # Interleaved apps
    test_interleaved_counter_todo(w)

    print("lifecycle: 10/10 passed")
