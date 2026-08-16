# Port of test/stress.test.ts — allocator stress tests exercised through the
# real WASM binary via mojo-wasmtime (pure Mojo FFI bindings — no Python
# interop required).
#
# Tests cover:
# - Sequential string allocations and readback
# - String roundtrip pipeline (return_input_string)
# - Repeated concat operations
# - Interleaved numeric and string operations
# - Empty struct allocations (non-overlapping)
# - Mixed-size string allocations
# - Fibonacci sequence consistency
# - string_eq reflexivity
#
# Run with:
#   mojo test test/test_stress.mojo

from memory import UnsafePointer
from testing import assert_true, assert_equal

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
    args_ptr,
    args_ptr_ptr,
    args_ptr_ptr_ptr,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ---------------------------------------------------------------------------
# Helper — pure-Mojo GCD for reference
# ---------------------------------------------------------------------------


fn _gcd(var a: Int, var b: Int) -> Int:
    if a < 0:
        a = -a
    if b < 0:
        b = -b
    while b != 0:
        var t = b
        b = a % b
        a = t
    return a


fn _i32_wrap(x: Int) -> Int:
    """Wrap to signed 32-bit integer (matching WASM i32 semantics)."""
    var v = x & 0xFFFFFFFF
    if v >= 0x80000000:
        v -= 0x100000000
    return v


fn _repeat_char(ch: String, n: Int) -> String:
    var result = String("")
    for _ in range(n):
        result += ch
    return result


# ---------------------------------------------------------------------------
# Allocator stress — many sequential string allocations
# ---------------------------------------------------------------------------


fn test_200_sequential_string_allocations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Write 200 distinct strings and verify they all read back correctly."""
    var ptrs = List[Int]()
    var strings = List[String]()
    for i in range(200):
        var s = "string-" + String(i) + "-" + _repeat_char("x", i % 30)
        strings.append(s)
        ptrs.append(w[].write_string_struct(s))

    # Read them all back — earlier allocations must not be corrupted
    for i in range(200):
        var result = w[].read_string_struct(ptrs[i])
        assert_equal(
            result,
            strings[i],
            String("readback string #") + String(i) + " after 200 allocs",
        )


# ---------------------------------------------------------------------------
# Allocator stress — many alloc + return_input_string roundtrips
# ---------------------------------------------------------------------------


fn test_100_return_input_string_roundtrips(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    for i in range(100):
        var s = "roundtrip-" + String(i)
        var in_ptr = w[].write_string_struct(s)
        var out_ptr = w[].alloc_string_struct()
        w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
        var result = w[].read_string_struct(out_ptr)
        assert_equal(
            result,
            s,
            String("roundtrip #") + String(i) + " failed",
        )


# ---------------------------------------------------------------------------
# Allocator stress — many concat operations
# ---------------------------------------------------------------------------


fn test_50_sequential_concats(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Build a string by concatenating 'ab' 50 times through WASM."""
    var current_ptr = w[].write_string_struct("")
    for _ in range(50):
        var append_ptr = w[].write_string_struct("ab")
        var out_ptr = w[].alloc_string_struct()
        w[].call_void(
            "string_concat", args_ptr_ptr_ptr(current_ptr, append_ptr, out_ptr)
        )
        current_ptr = out_ptr

    var result = w[].read_string_struct(current_ptr)
    var expected = _repeat_char("ab", 50)
    assert_equal(
        result,
        expected,
        "50 sequential concats produce correct result",
    )
    assert_equal(
        Int(w[].call_i64("string_length", args_ptr(current_ptr))),
        100,
        "50 sequential concats produce 100-byte string",
    )


# ---------------------------------------------------------------------------
# Allocator stress — interleaved numeric and string operations
# ---------------------------------------------------------------------------


fn test_50_interleaved_numeric_string_ops(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    for i in range(50):
        # Do some numeric work
        var x = Int(w[].call_i32("add_int32", args_i32_i32(i, i)))
        var y = Int(w[].call_i32("mul_int32", args_i32_i32(i, 3)))
        var g = Int(w[].call_i32("gcd_int32", args_i32_i32(x, y)))

        # Do a string roundtrip
        var s = "iter-" + String(i) + "-gcd-" + String(g)
        var in_ptr = w[].write_string_struct(s)
        var out_ptr = w[].alloc_string_struct()
        w[].call_void("return_input_string", args_ptr_ptr(in_ptr, out_ptr))
        var result = w[].read_string_struct(out_ptr)

        # Verify numeric result
        var expected_gcd = _gcd(Int(i) + Int(i), Int(i) * 3)
        assert_equal(
            g,
            expected_gcd,
            String("gcd at iteration ") + String(i),
        )

        # Verify string result
        assert_equal(
            result,
            s,
            String("string roundtrip at iteration ") + String(i),
        )


# ---------------------------------------------------------------------------
# Allocator stress — many small allocStringStruct calls
# ---------------------------------------------------------------------------


fn test_300_alloc_string_struct_non_overlapping(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Verify no two struct pointers overlap (each struct is 24 bytes)."""
    var ptrs = List[Int]()
    for _ in range(300):
        ptrs.append(w[].alloc_string_struct())

    for i in range(1, 300):
        var gap = ptrs[i] - ptrs[i - 1]
        assert_true(
            gap >= 24,
            String("struct #")
            + String(i)
            + " overlaps with #"
            + String(i - 1)
            + " (gap="
            + String(gap)
            + ")",
        )


# ---------------------------------------------------------------------------
# Allocator stress — mixed-size string allocations
# ---------------------------------------------------------------------------


fn test_mixed_size_strings_report_correct_length(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var sizes: List[Int] = [0, 1, 5, 22, 23, 24, 25, 50, 100, 255, 1, 0, 23, 24]
    var ptrs = List[Int]()

    for i in range(len(sizes)):
        var size = sizes[i]
        var s = _repeat_char("m", size)
        ptrs.append(w[].write_string_struct(s))

    for i in range(len(sizes)):
        var size = sizes[i]
        var length = Int(w[].call_i64("string_length", args_ptr(ptrs[i])))
        assert_equal(
            length,
            size,
            String("length of ") + String(size) + "-byte string",
        )


# ---------------------------------------------------------------------------
# Computation stress — fibonacci consistency across many values
# ---------------------------------------------------------------------------


fn test_fib_recurrence_2_to_40(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Verify fib(n) = fib(n-1) + fib(n-2) for n = 2..40."""
    for n in range(2, 41):
        var fn0 = Int(w[].call_i32("fib_int32", args_i32(n)))
        var fn1 = Int(w[].call_i32("fib_int32", args_i32(n - 1)))
        var fn2 = Int(w[].call_i32("fib_int32", args_i32(n - 2)))
        # Use i32 wrapping addition for the check (matches WASM semantics)
        var expected = _i32_wrap(fn1 + fn2)
        assert_equal(
            fn0,
            expected,
            String("fib(")
            + String(n)
            + ") === fib("
            + String(n - 1)
            + ") + fib("
            + String(n - 2)
            + ")",
        )


# ---------------------------------------------------------------------------
# Computation stress — string_eq reflexivity for many strings
# ---------------------------------------------------------------------------


fn test_string_eq_reflexive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var test_strings: List[String] = [
        "",
        "a",
        "hello",
        _repeat_char("x", 23),
        _repeat_char("y", 24),
        String("café"),
        String("Hello, World! "),
        _repeat_char("z", 100),
    ]
    for i in range(len(test_strings)):
        var s = test_strings[i]
        var a_ptr = w[].write_string_struct(s)
        var b_ptr = w[].write_string_struct(s)
        var eq = Int(w[].call_i32("string_eq", args_ptr_ptr(a_ptr, b_ptr)))
        assert_equal(
            eq,
            1,
            String("string_eq reflexive for string #") + String(i),
        )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_200_sequential_string_allocations(w)
    test_100_return_input_string_roundtrips(w)
    test_50_sequential_concats(w)
    test_50_interleaved_numeric_string_ops(w)
    test_300_alloc_string_struct_non_overlapping(w)
    test_mixed_size_strings_report_correct_length(w)
    test_fib_recurrence_2_to_40(w)
    test_string_eq_reflexive(w)
    print("stress: 8/8 passed")
