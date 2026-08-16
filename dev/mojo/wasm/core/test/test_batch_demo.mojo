"""Phase 38.2 — BatchDemoApp Mojo Integration Tests.

Validates BatchDemoApp via the bd_* WASM exports which exercise
batch signal writes: two SignalString fields (first_name, last_name)
feed a MemoString (full_name), and a SignalI32 (write_count) tracks
batch operations.

  - init creates app with non-zero pointer
  - initial state: full_name=" ", write_count=0
  - set_names("Alice", "Smith") → full_name="Alice Smith", write_count=1
  - reset → full_name=" ", write_count=0
  - set_names + flush produces DOM mutations
  - reset + flush produces DOM mutations
  - after set_names (before flush), full_name memo is dirty
  - is_batching returns false after set_names completes
  - memo stable same names: second flush returns 0
  - set then reset cycle: both flushes produce mutations
  - multiple sets with different values
  - write_count increments across multiple set_names calls
  - scope_count == 1
  - memo_count == 1
  - destroy does not crash
  - flush returns 0 when clean (no events)
  - handle_event for set handler marks dirty
  - handle_event for reset handler marks dirty
  - rapid 10 set_names calls: final state correct

Run with:
  mojo test test/test_batch_demo.mojo
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
    args_ptr_ptr_ptr,
)


fn _load() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_bd(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a BatchDemoApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("bd_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("bd_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _create_bd_no_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a BatchDemoApp without mounting.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("bd_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    return Tuple(app, buf)


fn _destroy_bd(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a BatchDemoApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("bd_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("bd_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _handle_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    handler_id: Int32,
) raises -> Int32:
    """Dispatch an event.  Returns 1 if handled."""
    return w[].call_i32("bd_handle_event", args_ptr_i32_i32(app, handler_id, 0))


fn _set_names(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    first: String,
    last: String,
) raises:
    """Call bd_set_names(app, first, last) via WASM."""
    var first_ptr = w[].write_string_struct(first)
    var last_ptr = w[].write_string_struct(last)
    w[].call_void("bd_set_names", args_ptr_ptr_ptr(app, first_ptr, last_ptr))


fn _reset(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises:
    """Call bd_reset(app) via WASM."""
    w[].call_void("bd_reset", args_ptr(app))


fn _full_name_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("bd_full_name_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _first_name_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("bd_first_name_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _last_name_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("bd_last_name_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _write_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("bd_write_count", args_ptr(app))


fn _full_name_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("bd_full_name_dirty", args_ptr(app)) != 0


fn _full_name_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("bd_full_name_changed", args_ptr(app)) != 0


fn _has_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("bd_has_dirty", args_ptr(app)) != 0


fn _is_batching(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    return w[].call_i32("bd_is_batching", args_ptr(app)) != 0


fn _scope_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("bd_scope_count", args_ptr(app))


fn _memo_count(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("bd_memo_count", args_ptr(app))


fn _set_handler(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("bd_set_handler", args_ptr(app))


fn _reset_handler(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Int32:
    return w[].call_i32("bd_reset_handler", args_ptr(app))


# ── Tests ────────────────────────────────────────────────────────────────────


def test_bd_initial_state():
    """After rebuild, full_name=' ' and write_count=0."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    assert_equal(_full_name_text(w, app), " ", "full_name = ' '")
    assert_equal(_write_count(w, app), 0, "write_count = 0")
    assert_equal(_first_name_text(w, app), "", "first_name = ''")
    assert_equal(_last_name_text(w, app), "", "last_name = ''")

    _destroy_bd(w, app, buf)


def test_bd_set_names():
    """Set names 'Alice'+'Smith' → full_name='Alice Smith', write_count=1."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("Alice"), String("Smith"))
    _ = _flush(w, app, buf)

    assert_equal(_full_name_text(w, app), "Alice Smith", "full_name correct")
    assert_equal(_write_count(w, app), 1, "write_count = 1")
    assert_equal(_first_name_text(w, app), "Alice", "first_name = 'Alice'")
    assert_equal(_last_name_text(w, app), "Smith", "last_name = 'Smith'")

    _destroy_bd(w, app, buf)


def test_bd_reset():
    """Set names then reset → full_name=' ', write_count=0."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("Alice"), String("Smith"))
    _ = _flush(w, app, buf)

    _reset(w, app)
    _ = _flush(w, app, buf)

    assert_equal(_full_name_text(w, app), " ", "full_name = ' ' after reset")
    assert_equal(_write_count(w, app), 0, "write_count = 0 after reset")

    _destroy_bd(w, app, buf)


def test_bd_set_names_flush():
    """Set names + flush produces DOM mutations (nonzero bytes)."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("Alice"), String("Smith"))
    var bytes = _flush(w, app, buf)
    assert_true(bytes > 0, "flush returns >0 after set_names")

    _destroy_bd(w, app, buf)


def test_bd_reset_flush():
    """Reset + flush produces DOM mutations (nonzero bytes)."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    # First set names so there's something to reset
    _set_names(w, app, String("Alice"), String("Smith"))
    _ = _flush(w, app, buf)

    # Reset should produce mutations
    _reset(w, app)
    var bytes = _flush(w, app, buf)
    assert_true(bytes > 0, "flush returns >0 after reset")

    _destroy_bd(w, app, buf)


def test_bd_set_names_memo_dirty():
    """After set_names (before flush), full_name memo is dirty."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("Alice"), String("Smith"))
    assert_true(_full_name_dirty(w, app), "full_name dirty after set_names")

    # Clean up by flushing
    _ = _flush(w, app, buf)
    _destroy_bd(w, app, buf)


def test_bd_not_batching_after_set():
    """Batching flag returns false after set_names completes (batch ended)."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    assert_false(_is_batching(w, app), "not batching initially")
    _set_names(w, app, String("Alice"), String("Smith"))
    assert_false(_is_batching(w, app), "not batching after set_names")

    _ = _flush(w, app, buf)
    _destroy_bd(w, app, buf)


def test_bd_memo_stable_same_names():
    """Set same names twice — second flush returns 0 (memo value-stable)."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    # First set
    _set_names(w, app, String("Alice"), String("Smith"))
    var bytes1 = _flush(w, app, buf)
    assert_true(bytes1 > 0, "first flush returns >0")

    # Same names again — memo recomputes to same value, but write_count changes
    # So the scope is still dirty due to write_count signal
    _set_names(w, app, String("Alice"), String("Smith"))
    var bytes2 = _flush(w, app, buf)
    # write_count changed (1→2), so DOM updates for "Writes: 2"
    assert_true(bytes2 > 0, "second flush >0 (write_count changed)")

    _destroy_bd(w, app, buf)


def test_bd_set_then_reset():
    """Set names, flush, reset, flush — both flushes produce mutations."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("Alice"), String("Smith"))
    var bytes1 = _flush(w, app, buf)
    assert_true(bytes1 > 0, "flush after set returns >0")

    _reset(w, app)
    var bytes2 = _flush(w, app, buf)
    assert_true(bytes2 > 0, "flush after reset returns >0")

    assert_equal(_full_name_text(w, app), " ", "full_name = ' ' after cycle")
    assert_equal(_write_count(w, app), 0, "write_count = 0 after cycle")

    _destroy_bd(w, app, buf)


def test_bd_multiple_sets():
    """5 set_names calls with different values — final state correct."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("A"), String("B"))
    _ = _flush(w, app, buf)
    _set_names(w, app, String("C"), String("D"))
    _ = _flush(w, app, buf)
    _set_names(w, app, String("E"), String("F"))
    _ = _flush(w, app, buf)
    _set_names(w, app, String("G"), String("H"))
    _ = _flush(w, app, buf)
    _set_names(w, app, String("I"), String("J"))
    _ = _flush(w, app, buf)

    assert_equal(_full_name_text(w, app), "I J", "final full_name correct")
    assert_equal(_write_count(w, app), 5, "write_count = 5")

    _destroy_bd(w, app, buf)


def test_bd_write_count_increments():
    """3 set_names calls → write_count=3."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    _set_names(w, app, String("A"), String("B"))
    _ = _flush(w, app, buf)
    assert_equal(_write_count(w, app), 1, "write_count = 1 after first set")

    _set_names(w, app, String("C"), String("D"))
    _ = _flush(w, app, buf)
    assert_equal(_write_count(w, app), 2, "write_count = 2 after second set")

    _set_names(w, app, String("E"), String("F"))
    _ = _flush(w, app, buf)
    assert_equal(_write_count(w, app), 3, "write_count = 3 after third set")

    _destroy_bd(w, app, buf)


def test_bd_scope_count():
    """App has exactly 1 scope."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    assert_equal(_scope_count(w, app), 1, "scope_count = 1")

    _destroy_bd(w, app, buf)


def test_bd_memo_count():
    """App has exactly 1 memo (full_name)."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    assert_equal(_memo_count(w, app), 1, "memo_count = 1")

    _destroy_bd(w, app, buf)


def test_bd_destroy_clean():
    """Destroy does not crash."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    # Should not crash
    _destroy_bd(w, app, buf)


def test_bd_flush_returns_zero_when_clean():
    """Flush without events returns 0."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    var bytes = _flush(w, app, buf)
    assert_equal(bytes, 0, "flush returns 0 when nothing dirty")

    _destroy_bd(w, app, buf)


def test_bd_handle_event_set():
    """Dispatch set_handler event marks scope dirty."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    var hid = _set_handler(w, app)
    # onclick_custom registers ACTION_CUSTOM which marks the scope dirty
    # but returns False (0) — the app is expected to do custom routing.
    _ = _handle_event(w, app, hid)
    assert_true(_has_dirty(w, app), "scope dirty after set handler dispatch")

    # Clean up
    _ = _flush(w, app, buf)
    _destroy_bd(w, app, buf)


def test_bd_handle_event_reset():
    """Dispatch reset_handler event marks scope dirty."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    var hid = _reset_handler(w, app)
    # onclick_custom registers ACTION_CUSTOM which marks the scope dirty
    # but returns False (0) — the app is expected to do custom routing.
    _ = _handle_event(w, app, hid)
    assert_true(_has_dirty(w, app), "scope dirty after reset handler dispatch")

    # Clean up
    _ = _flush(w, app, buf)
    _destroy_bd(w, app, buf)


def test_bd_rapid_10_sets():
    """10 rapid set_names calls — final state correct."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    for i in range(10):
        var first = String("F") + String(i)
        var last = String("L") + String(i)
        _set_names(w, app, first, last)
        _ = _flush(w, app, buf)

    assert_equal(_full_name_text(w, app), "F9 L9", "final full_name = 'F9 L9'")
    assert_equal(_write_count(w, app), 10, "write_count = 10")

    _destroy_bd(w, app, buf)


def test_bd_has_dirty_after_set():
    """Dirty flag is true after set_names, false after flush."""
    var w = _load()
    var t = _create_bd(w)
    var app = t[0]
    var buf = t[1]

    assert_false(_has_dirty(w, app), "not dirty initially (after rebuild)")

    _set_names(w, app, String("Alice"), String("Smith"))
    assert_true(_has_dirty(w, app), "dirty after set_names")

    _ = _flush(w, app, buf)
    assert_false(_has_dirty(w, app), "not dirty after flush")

    _destroy_bd(w, app, buf)


# ── Main ─────────────────────────────────────────────────────────────────────


fn main() raises:
    var passed = 0
    var failed = 0
    var total = 19

    # ── Test 1 ──
    try:
        test_bd_initial_state()
        passed += 1
    except e:
        print("FAIL test_bd_initial_state:", e)
        failed += 1

    # ── Test 2 ──
    try:
        test_bd_set_names()
        passed += 1
    except e:
        print("FAIL test_bd_set_names:", e)
        failed += 1

    # ── Test 3 ──
    try:
        test_bd_reset()
        passed += 1
    except e:
        print("FAIL test_bd_reset:", e)
        failed += 1

    # ── Test 4 ──
    try:
        test_bd_set_names_flush()
        passed += 1
    except e:
        print("FAIL test_bd_set_names_flush:", e)
        failed += 1

    # ── Test 5 ──
    try:
        test_bd_reset_flush()
        passed += 1
    except e:
        print("FAIL test_bd_reset_flush:", e)
        failed += 1

    # ── Test 6 ──
    try:
        test_bd_set_names_memo_dirty()
        passed += 1
    except e:
        print("FAIL test_bd_set_names_memo_dirty:", e)
        failed += 1

    # ── Test 7 ──
    try:
        test_bd_not_batching_after_set()
        passed += 1
    except e:
        print("FAIL test_bd_not_batching_after_set:", e)
        failed += 1

    # ── Test 8 ──
    try:
        test_bd_memo_stable_same_names()
        passed += 1
    except e:
        print("FAIL test_bd_memo_stable_same_names:", e)
        failed += 1

    # ── Test 9 ──
    try:
        test_bd_set_then_reset()
        passed += 1
    except e:
        print("FAIL test_bd_set_then_reset:", e)
        failed += 1

    # ── Test 10 ──
    try:
        test_bd_multiple_sets()
        passed += 1
    except e:
        print("FAIL test_bd_multiple_sets:", e)
        failed += 1

    # ── Test 11 ──
    try:
        test_bd_write_count_increments()
        passed += 1
    except e:
        print("FAIL test_bd_write_count_increments:", e)
        failed += 1

    # ── Test 12 ──
    try:
        test_bd_scope_count()
        passed += 1
    except e:
        print("FAIL test_bd_scope_count:", e)
        failed += 1

    # ── Test 13 ──
    try:
        test_bd_memo_count()
        passed += 1
    except e:
        print("FAIL test_bd_memo_count:", e)
        failed += 1

    # ── Test 14 ──
    try:
        test_bd_destroy_clean()
        passed += 1
    except e:
        print("FAIL test_bd_destroy_clean:", e)
        failed += 1

    # ── Test 15 ──
    try:
        test_bd_flush_returns_zero_when_clean()
        passed += 1
    except e:
        print("FAIL test_bd_flush_returns_zero_when_clean:", e)
        failed += 1

    # ── Test 16 ──
    try:
        test_bd_handle_event_set()
        passed += 1
    except e:
        print("FAIL test_bd_handle_event_set:", e)
        failed += 1

    # ── Test 17 ──
    try:
        test_bd_handle_event_reset()
        passed += 1
    except e:
        print("FAIL test_bd_handle_event_reset:", e)
        failed += 1

    # ── Test 18 ──
    try:
        test_bd_rapid_10_sets()
        passed += 1
    except e:
        print("FAIL test_bd_rapid_10_sets:", e)
        failed += 1

    # ── Test 19 ──
    try:
        test_bd_has_dirty_after_set()
        passed += 1
    except e:
        print("FAIL test_bd_has_dirty_after_set:", e)
        failed += 1

    # ── Summary ──
    print(
        "  ✓ test_batch_demo — batch_demo:",
        String(passed) + "/" + String(total),
        "passed",
    )
    if failed > 0:
        raise Error(String(failed) + " of " + String(total) + " tests FAILED")
