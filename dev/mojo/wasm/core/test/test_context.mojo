"""Phase 31.1 — Context (Dependency Injection) Surface Tests.

Validates ComponentContext.provide_context(), consume_context(), has_context(),
and the typed signal-sharing helpers (provide_signal_i32, consume_signal_i32,
provide_signal_bool, consume_signal_bool, provide_signal_string,
consume_signal_string) via the ContextTestApp (cta_*) WASM exports.

Tests:
  - provide_context stores value at root scope
  - consume_context retrieves value from same scope
  - consume_context walks up parent chain (provide at root, consume at child)
  - consume_context returns 0 for missing key
  - has_context returns True/False correctly
  - provide_context overwrites existing key
  - provide_signal_i32 round-trips through consume
  - consumed signal handle reads correct value
  - consumed signal handle writes propagate to parent
  - writing consumed signal marks parent scope dirty
  - multiple context keys coexist
  - context survives across flush cycles (signal writes)
  - context cleaned up on destroy
  - child and parent scopes are distinct
  - 10 create/destroy cycles bounded memory
"""

from std.memory import Pointer

from std.testing import assert_equal, assert_true, assert_false
from wasm_harness import (
    WasmInstance,
    get_instance,
    no_args,
    args_i32,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
)


def _load() raises -> WasmInstance:
    return WasmInstance("build/out.wasm")


# ── Helpers ──────────────────────────────────────────────────────────────────


def _create_cta(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises -> Int:
    """Create a context test app.  Returns app_ptr."""
    return Int(w[].call_i64("cta_init", no_args()))


def _destroy_cta(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
    app: Int,
) raises:
    """Destroy a context test app."""
    w[].call_void("cta_destroy", args_ptr(app))


# ── Test: provide_context stores value at root scope ─────────────────────────


def test_provide_context_stores_value(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Provide_context stores a value that can be consumed from the same scope.
    """
    var app = _create_cta(w)
    # Provide key=42, value=123 at root scope
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 42, 123))
    # Consume from root scope
    var val = w[].call_i32("cta_consume_context", args_ptr_i32(app, 42))
    assert_equal(val, 123)
    _destroy_cta(w, app)


# ── Test: consume_context returns 0 for missing key ──────────────────────────


def test_consume_context_missing_key(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Consume_context returns 0 when the key is not found."""
    var app = _create_cta(w)
    var val = w[].call_i32("cta_consume_context", args_ptr_i32(app, 999))
    assert_equal(val, 0)
    _destroy_cta(w, app)


# ── Test: has_context returns True/False ─────────────────────────────────────


def test_has_context_true_false(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Has_context returns 1 for existing key and 0 for missing key."""
    var app = _create_cta(w)
    # Before provide: not found
    var before = w[].call_i32("cta_has_context", args_ptr_i32(app, 10))
    assert_equal(before, 0)
    # Provide key=10
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 10, 77))
    # After provide: found
    var after = w[].call_i32("cta_has_context", args_ptr_i32(app, 10))
    assert_equal(after, 1)
    # Different key: still not found
    var other = w[].call_i32("cta_has_context", args_ptr_i32(app, 11))
    assert_equal(other, 0)
    _destroy_cta(w, app)


# ── Test: consume_context walks up parent chain ──────────────────────────────


def test_consume_from_child_walks_up(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Child scope can consume context provided at the parent scope."""
    var app = _create_cta(w)
    # Provide at root
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 50, 200))
    # Consume from child — should walk up to root
    var val = w[].call_i32("cta_consume_from_child", args_ptr_i32(app, 50))
    assert_equal(val, 200)
    _destroy_cta(w, app)


# ── Test: consume_from_child returns 0 for missing ──────────────────────────


def test_consume_from_child_missing(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Child scope consume returns 0 when key is not provided anywhere."""
    var app = _create_cta(w)
    var found = w[].call_i32(
        "cta_consume_found_from_child", args_ptr_i32(app, 777)
    )
    assert_equal(found, 0)
    _destroy_cta(w, app)


# ── Test: provide_context overwrites existing key ────────────────────────────


def test_provide_context_overwrites(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Providing the same key again overwrites the old value."""
    var app = _create_cta(w)
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 5, 100))
    var val1 = w[].call_i32("cta_consume_context", args_ptr_i32(app, 5))
    assert_equal(val1, 100)
    # Overwrite
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 5, 200))
    var val2 = w[].call_i32("cta_consume_context", args_ptr_i32(app, 5))
    assert_equal(val2, 200)
    _destroy_cta(w, app)


# ── Test: provide_signal_i32 round-trips through consume ─────────────────────


def test_provide_signal_i32_roundtrip(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Provide_signal_i32 stores the signal key; child can consume and read value.
    """
    var app = _create_cta(w)
    # Provide the count signal at context key 1
    w[].call_void("cta_provide_signal_i32", args_ptr_i32(app, 1))
    # Consume from child — should get the signal's current value (0)
    var val = w[].call_i32(
        "cta_consume_signal_i32_from_child", args_ptr_i32(app, 1)
    )
    assert_equal(val, 0)
    _destroy_cta(w, app)


# ── Test: consumed signal reads correct value ────────────────────────────────


def test_consumed_signal_reads_value(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """After the parent writes the signal, the child's consumed handle reads the new value.
    """
    var app = _create_cta(w)
    # Provide signal at context key 1
    w[].call_void("cta_provide_signal_i32", args_ptr_i32(app, 1))
    # Write directly to the count signal via cta_write_signal_via_child
    # First, consume_signal_i32 from child context and write 42
    w[].call_void("cta_write_signal_via_child", args_ptr_i32_i32(app, 1, 42))
    # Now read the count value from the parent's signal
    var parent_val = w[].call_i32("cta_count_value", args_ptr(app))
    assert_equal(parent_val, 42)
    # And read via child's consumed handle
    var child_val = w[].call_i32(
        "cta_consume_signal_i32_from_child", args_ptr_i32(app, 1)
    )
    assert_equal(child_val, 42)
    _destroy_cta(w, app)


# ── Test: writing consumed signal marks parent scope dirty ───────────────────


def test_consumed_signal_write_marks_dirty(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Writing a consumed signal (parent-owned) marks the parent scope dirty."""
    var app = _create_cta(w)
    # Should start clean
    var dirty_before = w[].call_i32("cta_has_dirty", args_ptr(app))
    assert_equal(dirty_before, 0)
    # Provide and consume signal
    w[].call_void("cta_provide_signal_i32", args_ptr_i32(app, 1))
    # Write via child handle — marks parent dirty
    w[].call_void("cta_write_signal_via_child", args_ptr_i32_i32(app, 1, 10))
    var dirty_after = w[].call_i32("cta_has_dirty", args_ptr(app))
    assert_equal(dirty_after, 1)
    _destroy_cta(w, app)


# ── Test: multiple context keys coexist ──────────────────────────────────────


def test_multiple_context_keys(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Multiple context keys can coexist without interference."""
    var app = _create_cta(w)
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 1, 10))
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 2, 20))
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 3, 30))
    var v1 = w[].call_i32("cta_consume_context", args_ptr_i32(app, 1))
    var v2 = w[].call_i32("cta_consume_context", args_ptr_i32(app, 2))
    var v3 = w[].call_i32("cta_consume_context", args_ptr_i32(app, 3))
    assert_equal(v1, 10)
    assert_equal(v2, 20)
    assert_equal(v3, 30)
    _destroy_cta(w, app)


# ── Test: child and parent scopes are distinct ───────────────────────────────


def test_child_parent_scopes_distinct(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Root scope and child scope have different IDs."""
    var app = _create_cta(w)
    var root_id = w[].call_i32("cta_root_scope_id", args_ptr(app))
    var child_id = w[].call_i32("cta_child_scope_id", args_ptr(app))
    assert_true(root_id >= 0, "root scope ID should be non-negative")
    assert_true(child_id >= 0, "child scope ID should be non-negative")
    assert_true(root_id != child_id, "root and child scope IDs must differ")
    _destroy_cta(w, app)


# ── Test: context from child walks up (found check) ─────────────────────────


def test_consume_found_from_child(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Consume_found_from_child returns 1 when parent provides the key."""
    var app = _create_cta(w)
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 88, 999))
    var found = w[].call_i32(
        "cta_consume_found_from_child", args_ptr_i32(app, 88)
    )
    assert_equal(found, 1)
    _destroy_cta(w, app)


# ── Test: context survives across signal writes ──────────────────────────────


def test_context_survives_signal_writes(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Context values survive after signal writes and dirty cycling."""
    var app = _create_cta(w)
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 7, 777))
    # Provide signal and write to it a few times
    w[].call_void("cta_provide_signal_i32", args_ptr_i32(app, 1))
    w[].call_void("cta_write_signal_via_child", args_ptr_i32_i32(app, 1, 5))
    w[].call_void("cta_write_signal_via_child", args_ptr_i32_i32(app, 1, 10))
    # Context value should still be intact
    var val = w[].call_i32("cta_consume_from_child", args_ptr_i32(app, 7))
    assert_equal(val, 777)
    # Signal value should be updated
    var sig_val = w[].call_i32("cta_count_value", args_ptr(app))
    assert_equal(sig_val, 10)
    _destroy_cta(w, app)


# ── Test: destroy does not crash ─────────────────────────────────────────────


def test_destroy_does_not_crash(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Creating and destroying a context test app does not crash."""
    var app = _create_cta(w)
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 1, 42))
    w[].call_void("cta_provide_signal_i32", args_ptr_i32(app, 2))
    _destroy_cta(w, app)
    # If we got here, no crash


# ── Test: destroy + recreate cycle ───────────────────────────────────────────


def test_destroy_recreate_cycle(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Destroy then recreate cycle produces a fresh app with independent state.
    """
    var app1 = _create_cta(w)
    w[].call_void("cta_provide_context", args_ptr_i32_i32(app1, 1, 100))
    _destroy_cta(w, app1)

    var app2 = _create_cta(w)
    # New app should not see the old context
    var val = w[].call_i32("cta_consume_context", args_ptr_i32(app2, 1))
    assert_equal(val, 0)
    _destroy_cta(w, app2)


# ── Test: 10 create/destroy cycles bounded memory ───────────────────────────


def test_ten_create_destroy_cycles(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """10 create/destroy cycles work correctly (no leaks, no crashes)."""
    for i in range(10):
        var app = _create_cta(w)
        w[].call_void("cta_provide_context", args_ptr_i32_i32(app, 1, Int32(i)))
        var val = w[].call_i32("cta_consume_context", args_ptr_i32(app, 1))
        assert_equal(val, Int32(i))
        _destroy_cta(w, app)


# ── Test: scope count reflects 2 scopes ─────────────────────────────────────


def test_scope_count(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """The app has exactly 2 live scopes (root + child)."""
    var app = _create_cta(w)
    var count = w[].call_i32("cta_scope_count", args_ptr(app))
    assert_equal(count, 2)
    _destroy_cta(w, app)


# ── Test: consume_signal returns sentinel for missing context key ────────────


def test_consume_signal_missing_key_sentinel(
    w: Pointer[WasmInstance, MutUntrackedOrigin],
) raises:
    """Consume_signal_i32_from_child returns sentinel (-9999) for missing key.
    """
    var app = _create_cta(w)
    var val = w[].call_i32(
        "cta_consume_signal_i32_from_child", args_ptr_i32(app, 999)
    )
    assert_equal(val, -9999)
    _destroy_cta(w, app)


# ══════════════════════════════════════════════════════════════════════════════
# Entry point
# ══════════════════════════════════════════════════════════════════════════════


def main() raises:
    var wp = get_instance()
    print(
        "test_context — ComponentContext provide/consume + signal helpers"
        " (Phase 31.1):"
    )

    test_provide_context_stores_value(wp)
    print("  ✓ provide_context stores value at root scope")

    test_consume_context_missing_key(wp)
    print("  ✓ consume_context returns 0 for missing key")

    test_has_context_true_false(wp)
    print("  ✓ has_context returns True/False correctly")

    test_consume_from_child_walks_up(wp)
    print("  ✓ consume_context walks up parent chain")

    test_consume_from_child_missing(wp)
    print("  ✓ consume from child returns 0 for missing key")

    test_provide_context_overwrites(wp)
    print("  ✓ provide_context overwrites existing key")

    test_provide_signal_i32_roundtrip(wp)
    print("  ✓ provide_signal_i32 round-trips through consume")

    test_consumed_signal_reads_value(wp)
    print("  ✓ consumed signal handle reads correct value")

    test_consumed_signal_write_marks_dirty(wp)
    print("  ✓ writing consumed signal marks parent scope dirty")

    test_multiple_context_keys(wp)
    print("  ✓ multiple context keys coexist")

    test_child_parent_scopes_distinct(wp)
    print("  ✓ child and parent scopes are distinct")

    test_consume_found_from_child(wp)
    print("  ✓ consume_found_from_child returns 1 for parent-provided key")

    test_context_survives_signal_writes(wp)
    print("  ✓ context survives across signal writes")

    test_destroy_does_not_crash(wp)
    print("  ✓ destroy does not crash")

    test_destroy_recreate_cycle(wp)
    print("  ✓ destroy → recreate cycle")

    test_ten_create_destroy_cycles(wp)
    print("  ✓ 10 create/destroy cycles")

    test_scope_count(wp)
    print("  ✓ scope count reflects 2 scopes")

    test_consume_signal_missing_key_sentinel(wp)
    print("  ✓ consume_signal returns sentinel for missing context key")

    print("\n  18/18 tests passed ✓")
