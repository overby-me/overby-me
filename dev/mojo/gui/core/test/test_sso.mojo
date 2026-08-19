# Port of test/sso.test.ts — String SSO (Small String Optimization) boundary
# tests exercised through the real WASM binary via mojo-wasmtime (pure Mojo
# FFI bindings — no Python interop required).
#
# Mojo's Small String Optimization stores strings inline in the 24-byte struct
# when they fit (<=23 bytes). At 24+ bytes the data is heap-allocated. These
# tests exercise the boundary to verify that string operations work correctly
# across the SSO/heap transition.
#
# Run with:
#   mojo test test/test_sso.mojo

from std.memory import UnsafePointer
from std.testing import assert_true, assert_equal

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_ptr,
    args_ptr_ptr_ptr,
    args_ptr_i32_ptr,
    no_args,
)


def _get_wasm() raises -> UnsafePointer[WasmInstance, MutUntrackedOrigin]:
    return get_instance()


# ---------------------------------------------------------------------------
# Helper — build a repeated-char string of length n
# ---------------------------------------------------------------------------


def _repeat_char(ch: String, n: Int) -> String:
    var result = String("")
    for _ in range(n):
        result += ch
    return result


# ---------------------------------------------------------------------------
# SSO roundtrip via return_input_string
# ---------------------------------------------------------------------------


def test_roundtrip_22_bytes_sso(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """22 bytes: comfortably within SSO."""
    var s = _repeat_char("a", 22)
    var in_ptr = w[].write_string_struct(s)
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
    assert_equal(
        w[].read_string_struct(out_ptr),
        s,
        "return_input_string 22-byte string (SSO)",
    )


def test_roundtrip_23_bytes_sso_max(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """23 bytes: max SSO capacity."""
    var s = _repeat_char("b", 23)
    var in_ptr = w[].write_string_struct(s)
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
    assert_equal(
        w[].read_string_struct(out_ptr),
        s,
        "return_input_string 23-byte string (SSO max)",
    )


def test_roundtrip_24_bytes_heap(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """24 bytes: first heap-allocated size."""
    var s = _repeat_char("c", 24)
    var in_ptr = w[].write_string_struct(s)
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
    assert_equal(
        w[].read_string_struct(out_ptr),
        s,
        "return_input_string 24-byte string (heap)",
    )


def test_roundtrip_25_bytes_heap(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """25 bytes: safely past the boundary."""
    var s = _repeat_char("d", 25)
    var in_ptr = w[].write_string_struct(s)
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
    assert_equal(
        w[].read_string_struct(out_ptr),
        s,
        "return_input_string 25-byte string (heap)",
    )


# ---------------------------------------------------------------------------
# SSO boundary — string_length
# ---------------------------------------------------------------------------


def test_length_22_sso(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var ptr = w[].write_string_struct(_repeat_char("x", 22))
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(ptr))),
        22,
        "string_length 22-byte (SSO)",
    )


def test_length_23_sso_max(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var ptr = w[].write_string_struct(_repeat_char("x", 23))
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(ptr))),
        23,
        "string_length 23-byte (SSO max)",
    )


def test_length_24_heap(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var ptr = w[].write_string_struct(_repeat_char("x", 24))
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(ptr))),
        24,
        "string_length 24-byte (heap)",
    )


# ---------------------------------------------------------------------------
# SSO boundary — string_eq
# ---------------------------------------------------------------------------


def test_eq_23_identical_sso(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """Both SSO."""
    var s = _repeat_char("y", 23)
    var a_ptr = w[].write_string_struct(s)
    var b_ptr = w[].write_string_struct(s)
    assert_equal(
        Int(w[].call_i32("string_eq", args_ptr_ptr(a_ptr, b_ptr))),
        1,
        "string_eq 23-byte identical (SSO === SSO)",
    )


def test_eq_23_vs_24_different_length(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """SSO vs heap: different lengths should not match."""
    var a_ptr = w[].write_string_struct(_repeat_char("z", 23))
    var b_ptr = w[].write_string_struct(_repeat_char("z", 24))
    assert_equal(
        Int(w[].call_i32("string_eq", args_ptr_ptr(a_ptr, b_ptr))),
        0,
        "string_eq 23-byte vs 24-byte (SSO !== heap, different length)",
    )


def test_eq_24_identical_heap(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """Both heap."""
    var s = _repeat_char("w", 24)
    var a_ptr = w[].write_string_struct(s)
    var b_ptr = w[].write_string_struct(s)
    assert_equal(
        Int(w[].call_i32("string_eq", args_ptr_ptr(a_ptr, b_ptr))),
        1,
        "string_eq 24-byte identical (heap === heap)",
    )


def test_eq_23_differ_last_byte_sso(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """Same length at boundary, different content."""
    var a_ptr = w[].write_string_struct(_repeat_char("a", 23))
    var b_ptr = w[].write_string_struct(_repeat_char("a", 22) + "b")
    assert_equal(
        Int(w[].call_i32("string_eq", args_ptr_ptr(a_ptr, b_ptr))),
        0,
        "string_eq 23-byte differ in last byte (SSO)",
    )


# ---------------------------------------------------------------------------
# SSO boundary — string_concat crossing the boundary
# ---------------------------------------------------------------------------


def test_concat_11_plus_12_eq_23_sso(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """Two small strings that concat to exactly 23 bytes (SSO)."""
    var a_ptr = w[].write_string_struct(_repeat_char("a", 11))
    var b_ptr = w[].write_string_struct(_repeat_char("b", 12))
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("string_concat", args_ptr_ptr_ptr(a_ptr, b_ptr, out_ptr))
    var result = w[].read_string_struct(out_ptr)
    assert_equal(
        result,
        _repeat_char("a", 11) + _repeat_char("b", 12),
        "string_concat 11+12=23 bytes (result at SSO max)",
    )
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(out_ptr))),
        23,
        "string_concat result length === 23",
    )


def test_concat_12_plus_12_eq_24_heap(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """Two small strings that concat to exactly 24 bytes (crosses to heap)."""
    var a_ptr = w[].write_string_struct(_repeat_char("a", 12))
    var b_ptr = w[].write_string_struct(_repeat_char("b", 12))
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("string_concat", args_ptr_ptr_ptr(a_ptr, b_ptr, out_ptr))
    var result = w[].read_string_struct(out_ptr)
    assert_equal(
        result,
        _repeat_char("a", 12) + _repeat_char("b", 12),
        "string_concat 12+12=24 bytes (result crosses to heap)",
    )
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(out_ptr))),
        24,
        "string_concat result length === 24",
    )


# ---------------------------------------------------------------------------
# SSO boundary — string_repeat crossing the boundary
# ---------------------------------------------------------------------------


def test_repeat_8x3_eq_24_heap(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """8 * 3 = 24 bytes -> heap."""
    var ptr = w[].write_string_struct(_repeat_char("a", 8))
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("string_repeat", args_ptr_i32_ptr(ptr, 3, out_ptr))
    var result = w[].read_string_struct(out_ptr)
    assert_equal(
        result,
        _repeat_char("a", 24),
        "string_repeat 8-byte * 3 = 24 bytes (crosses to heap)",
    )


def test_repeat_23x1_stays_sso(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    """23 * 1 = 23 bytes -> stays SSO."""
    var ptr = w[].write_string_struct(_repeat_char("q", 23))
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("string_repeat", args_ptr_i32_ptr(ptr, 1, out_ptr))
    var result = w[].read_string_struct(out_ptr)
    assert_equal(
        result,
        _repeat_char("q", 23),
        "string_repeat 23-byte * 1 = 23 bytes (stays SSO)",
    )


# ---------------------------------------------------------------------------
# Larger heap strings (well past SSO)
# ---------------------------------------------------------------------------


def test_roundtrip_150_bytes(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var s = _repeat_char("abc", 50)  # 150 bytes
    var in_ptr = w[].write_string_struct(s)
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
    assert_equal(
        w[].read_string_struct(out_ptr),
        s,
        "return_input_string 150-byte string (well past SSO)",
    )


def test_length_256_bytes(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var s = _repeat_char("x", 256)
    var ptr = w[].write_string_struct(s)
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(ptr))),
        256,
        "string_length 256-byte (heap)",
    )


def main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_roundtrip_22_bytes_sso(w)
    test_roundtrip_23_bytes_sso_max(w)
    test_roundtrip_24_bytes_heap(w)
    test_roundtrip_25_bytes_heap(w)
    test_length_22_sso(w)
    test_length_23_sso_max(w)
    test_length_24_heap(w)
    test_eq_23_identical_sso(w)
    test_eq_23_vs_24_different_length(w)
    test_eq_24_identical_heap(w)
    test_eq_23_differ_last_byte_sso(w)
    test_concat_11_plus_12_eq_23_sso(w)
    test_concat_12_plus_12_eq_24_heap(w)
    test_repeat_8x3_eq_24_heap(w)
    test_repeat_23x1_stays_sso(w)
    test_roundtrip_150_bytes(w)
    test_length_256_bytes(w)
    print("sso: 17/17 passed")
