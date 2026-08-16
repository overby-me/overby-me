"""Phase 35.2 — MemoFormApp Mojo Tests.

Validates MemoFormApp via the mf_* WASM exports which exercise
MemoBool + MemoString in a form-validation scenario:

  - init creates app with non-zero pointer
  - input starts empty
  - is_valid starts false
  - status starts "✗ Empty"
  - memos start dirty before first flush
  - rebuild settles memos (both clean)
  - rebuild is_valid = False
  - rebuild status = "✗ Empty"
  - set_input marks both memos dirty
  - flush after set_input "hello" → is_valid = True
  - flush after set_input "hello" → status = "✓ Valid: hello"
  - clear input reverts (is_valid=False, status="✗ Empty")
  - memo recomputation order (is_valid before status)
  - multiple inputs correct ("a" → "ab" → "abc")
  - flush returns 0 when clean
  - memo count is 2
  - destroy does not crash
  - scope count is 1
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


fn _create_mf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a MemoFormApp and mount it.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("mf_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    _ = w[].call_i32("mf_rebuild", args_ptr_ptr_i32(app, buf, 8192))
    return Tuple(app, buf)


fn _create_mf_no_rebuild(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises -> Tuple[Int, Int]:
    """Create a MemoFormApp without mounting.  Returns (app_ptr, buf_ptr)."""
    var app = Int(w[].call_i64("mf_init", no_args()))
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))
    return Tuple(app, buf)


fn _destroy_mf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises:
    """Destroy a MemoFormApp and free the buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))
    w[].call_void("mf_destroy", args_ptr(app))


fn _flush(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    buf: Int,
) raises -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    return w[].call_i32("mf_flush", args_ptr_ptr_i32(app, buf, 8192))


fn _set_input(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
    value: String,
) raises:
    """Set the input signal directly (test helper)."""
    var str_ptr = w[].write_string_struct(value)
    w[].call_void("mf_set_input", args_ptr_ptr(app, str_ptr))


fn _input_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    """Read the input signal text."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("mf_input_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _status_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> String:
    """Read the status memo text."""
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("mf_status_text", args_ptr_ptr(app, out_ptr))
    return w[].read_string_struct(out_ptr)


fn _is_valid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    """Read the is_valid memo value."""
    return w[].call_i32("mf_is_valid", args_ptr(app)) != 0


fn _is_valid_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    """Check if is_valid memo is dirty."""
    return w[].call_i32("mf_is_valid_dirty", args_ptr(app)) != 0


fn _status_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
    app: Int,
) raises -> Bool:
    """Check if status memo is dirty."""
    return w[].call_i32("mf_status_dirty", args_ptr(app)) != 0


# ── Test: init creates app ───────────────────────────────────────────────────


fn test_mf_init_creates_app(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Init returns a non-zero pointer."""
    var app = Int(w[].call_i64("mf_init", no_args()))
    assert_true(app != 0, msg="app pointer should be non-zero")
    w[].call_void("mf_destroy", args_ptr(app))


# ── Test: input starts empty ─────────────────────────────────────────────────


fn test_mf_input_starts_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Input signal is empty after init + rebuild."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    var txt = _input_text(w, app)
    assert_equal(txt, String(""))
    _destroy_mf(w, app, buf)


# ── Test: is_valid starts false ──────────────────────────────────────────────


fn test_mf_is_valid_starts_false(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Validate is_valid memo is False after init + rebuild."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    assert_false(_is_valid(w, app), msg="is_valid should be False initially")
    _destroy_mf(w, app, buf)


# ── Test: status starts "✗ Empty" ────────────────────────────────────────────


fn test_mf_status_starts_empty_marker(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Status memo is '✗ Empty' after init + rebuild."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    var status = _status_text(w, app)
    assert_equal(status, String("✗ Empty"))
    _destroy_mf(w, app, buf)


# ── Test: memos start dirty ──────────────────────────────────────────────────


fn test_mf_memos_start_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Both memos are dirty before first rebuild."""
    var t = _create_mf_no_rebuild(w)
    var app = t[0]
    var buf = t[1]
    assert_true(_is_valid_dirty(w, app), msg="is_valid should be dirty")
    assert_true(_status_dirty(w, app), msg="status should be dirty")
    _destroy_mf(w, app, buf)


# ── Test: rebuild settles memos ──────────────────────────────────────────────


fn test_mf_rebuild_settles_memos(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After rebuild, both memos are clean."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    assert_false(
        _is_valid_dirty(w, app), msg="is_valid should be clean after rebuild"
    )
    assert_false(
        _status_dirty(w, app), msg="status should be clean after rebuild"
    )
    _destroy_mf(w, app, buf)


# ── Test: rebuild is_valid = False ───────────────────────────────────────────


fn test_mf_rebuild_is_valid_false(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Validate is_valid = False after rebuild (empty input)."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    assert_false(_is_valid(w, app), msg="is_valid should be False")
    _destroy_mf(w, app, buf)


# ── Test: rebuild status = "✗ Empty" ─────────────────────────────────────────


fn test_mf_rebuild_status_empty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Status memo = '✗ Empty' after rebuild (empty input)."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    var status = _status_text(w, app)
    assert_equal(status, String("✗ Empty"))
    _destroy_mf(w, app, buf)


# ── Test: set_input marks dirty ──────────────────────────────────────────────


fn test_mf_set_input_marks_dirty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Setting input dirties both memos."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    _set_input(w, app, String("hello"))
    assert_true(
        _is_valid_dirty(w, app), msg="is_valid should be dirty after set_input"
    )
    assert_true(
        _status_dirty(w, app), msg="status should be dirty after set_input"
    )
    _destroy_mf(w, app, buf)


# ── Test: flush after set_input → is_valid True ─────────────────────────────


fn test_mf_flush_after_set_input_valid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After set_input("hello") + flush, is_valid = True."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    _set_input(w, app, String("hello"))
    _ = _flush(w, app, buf)
    assert_true(_is_valid(w, app), msg="is_valid should be True for 'hello'")
    _destroy_mf(w, app, buf)


# ── Test: flush after set_input → status ─────────────────────────────────────


fn test_mf_flush_after_set_input_status(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """After set_input("hello") + flush, status = '✓ Valid: hello'."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    _set_input(w, app, String("hello"))
    _ = _flush(w, app, buf)
    var status = _status_text(w, app)
    assert_equal(status, String("✓ Valid: hello"))
    _destroy_mf(w, app, buf)


# ── Test: clear input reverts ────────────────────────────────────────────────


fn test_mf_clear_input_reverts(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Setting input to '' reverts is_valid=False, status='✗ Empty'."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    # First set non-empty
    _set_input(w, app, String("hello"))
    _ = _flush(w, app, buf)
    assert_true(_is_valid(w, app), msg="is_valid should be True")
    # Clear
    _set_input(w, app, String(""))
    _ = _flush(w, app, buf)
    assert_false(_is_valid(w, app), msg="is_valid should be False after clear")
    var status = _status_text(w, app)
    assert_equal(status, String("✗ Empty"))
    _destroy_mf(w, app, buf)


# ── Test: memo recomputation order ───────────────────────────────────────────


fn test_mf_memo_recomputation_order(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Validate is_valid recomputed before status — status sees updated is_valid.

    Validates order: set input → flush → status reads is_valid=True →
    produces '✓ Valid: ...' (not '✗ Empty' from stale is_valid).
    """
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    _set_input(w, app, String("test"))
    _ = _flush(w, app, buf)
    # If status ran before is_valid, it would see is_valid=False and
    # produce "✗ Empty" even though input is non-empty.
    var status = _status_text(w, app)
    assert_equal(status, String("✓ Valid: test"))
    assert_true(_is_valid(w, app), msg="is_valid should be True")
    _destroy_mf(w, app, buf)


# ── Test: multiple inputs correct ────────────────────────────────────────────


fn test_mf_multiple_inputs_correct(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """'a' → 'ab' → 'abc' all produce correct derived state."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]

    _set_input(w, app, String("a"))
    _ = _flush(w, app, buf)
    assert_true(_is_valid(w, app), msg="is_valid True for 'a'")
    var s1 = _status_text(w, app)
    assert_equal(s1, String("✓ Valid: a"))

    _set_input(w, app, String("ab"))
    _ = _flush(w, app, buf)
    assert_true(_is_valid(w, app), msg="is_valid True for 'ab'")
    var s2 = _status_text(w, app)
    assert_equal(s2, String("✓ Valid: ab"))

    _set_input(w, app, String("abc"))
    _ = _flush(w, app, buf)
    assert_true(_is_valid(w, app), msg="is_valid True for 'abc'")
    var s3 = _status_text(w, app)
    assert_equal(s3, String("✓ Valid: abc"))

    _destroy_mf(w, app, buf)


# ── Test: flush returns 0 when clean ─────────────────────────────────────────


fn test_mf_flush_returns_0_when_clean(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Flush returns 0 when no state changes have occurred."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    var result = _flush(w, app, buf)
    assert_equal(result, Int32(0))
    _destroy_mf(w, app, buf)


# ── Test: memo count is 2 ───────────────────────────────────────────────────


fn test_mf_memo_count_is_2(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Two live memos (is_valid + status)."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("mf_memo_count", args_ptr(app))
    assert_equal(count, Int32(2))
    _destroy_mf(w, app, buf)


# ── Test: destroy does not crash ─────────────────────────────────────────────


fn test_mf_destroy_does_not_crash(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Destroy after normal use does not crash."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    _set_input(w, app, String("hello"))
    _ = _flush(w, app, buf)
    _destroy_mf(w, app, buf)


# ── Test: scope count is 1 ──────────────────────────────────────────────────


fn test_mf_scope_count_is_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Single root scope."""
    var t = _create_mf(w)
    var app = t[0]
    var buf = t[1]
    var count = w[].call_i32("mf_scope_count", args_ptr(app))
    assert_equal(count, Int32(1))
    _destroy_mf(w, app, buf)


# ── Test runner ──────────────────────────────────────────────────────────────


fn main() raises:
    var wp = _load()

    print("test_memo_form — MemoFormApp form validation (Phase 35.2):")

    test_mf_init_creates_app(wp)
    print("  ✓ init creates app")

    test_mf_input_starts_empty(wp)
    print("  ✓ input starts empty")

    test_mf_is_valid_starts_false(wp)
    print("  ✓ is_valid starts false")

    test_mf_status_starts_empty_marker(wp)
    print("  ✓ status starts '✗ Empty'")

    test_mf_memos_start_dirty(wp)
    print("  ✓ memos start dirty")

    test_mf_rebuild_settles_memos(wp)
    print("  ✓ rebuild settles memos")

    test_mf_rebuild_is_valid_false(wp)
    print("  ✓ rebuild is_valid = False")

    test_mf_rebuild_status_empty(wp)
    print("  ✓ rebuild status = '✗ Empty'")

    test_mf_set_input_marks_dirty(wp)
    print("  ✓ set_input marks dirty")

    test_mf_flush_after_set_input_valid(wp)
    print("  ✓ flush after set_input — is_valid = True")

    test_mf_flush_after_set_input_status(wp)
    print("  ✓ flush after set_input — status = '✓ Valid: hello'")

    test_mf_clear_input_reverts(wp)
    print("  ✓ clear input reverts")

    test_mf_memo_recomputation_order(wp)
    print("  ✓ memo recomputation order")

    test_mf_multiple_inputs_correct(wp)
    print("  ✓ multiple inputs correct")

    test_mf_flush_returns_0_when_clean(wp)
    print("  ✓ flush returns 0 when clean")

    test_mf_memo_count_is_2(wp)
    print("  ✓ memo count is 2")

    test_mf_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_mf_scope_count_is_1(wp)
    print("  ✓ scope count is 1")

    print("  ✓ test_memo_form — memo_form: 18/18 passed")
