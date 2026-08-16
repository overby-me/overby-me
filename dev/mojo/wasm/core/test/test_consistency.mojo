# Port of test/consistency.test.ts — cross-function consistency checks exercised
# through the real WASM binary via mojo-wasmtime (pure Mojo FFI bindings —
# no Python interop required).
#
# These tests verify that different WASM-exported functions compose correctly:
# add/sub inverses, mul/div inverses, neg/abs relationships, min/max ordering,
# bitwise identities, shift roundtrips, De Morgan's law, GCD scaling,
# Fibonacci/factorial recurrence, string concat length, and clamp equivalence.
#
# Run with:
#   mojo test test/test_consistency.mojo

from memory import UnsafePointer
from testing import assert_true, assert_equal

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
    args_i32_i32_i32,
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


# ---------------------------------------------------------------------------
# Cross-function consistency checks
# ---------------------------------------------------------------------------


fn test_add_sub_inverse(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """sub(add(x, y), y) === x (add/sub inverse)."""
    var x = 17
    var y = 9
    var s = Int(w[].call_i32("add_int32", args_i32_i32(x, y)))
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(s, y))),
        x,
        "sub(add(x, y), y) === x",
    )


fn test_mul_div_inverse(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """div(mul(x, y), y) === x (mul/div inverse for exact division)."""
    var x = 6
    var y = 3
    var product = Int(w[].call_i32("mul_int32", args_i32_i32(x, y)))
    assert_equal(
        Int(w[].call_i32("div_int32", args_i32_i32(product, y))),
        x,
        "div(mul(x, y), y) === x",
    )


fn test_neg_neg_identity(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """neg(neg(x)) === x."""
    var inner = Int(w[].call_i32("neg_int32", args_i32(42)))
    assert_equal(
        Int(w[].call_i32("neg_int32", args_i32(inner))),
        42,
        "neg(neg(42)) === 42",
    )


fn test_abs_neg_eq_abs(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """abs(neg(x)) === abs(x) for positive x."""
    var neg7 = Int(w[].call_i32("neg_int32", args_i32(7)))
    var abs_neg = Int(w[].call_i32("abs_int32", args_i32(neg7)))
    var abs_pos = Int(w[].call_i32("abs_int32", args_i32(7)))
    assert_equal(abs_neg, abs_pos, "abs(neg(7)) === abs(7)")


fn test_min_le_max(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    """min(x, y) <= max(x, y)."""
    var a = 3
    var b = 7
    var lo = Int(w[].call_i32("min_int32", args_i32_i32(a, b)))
    var hi = Int(w[].call_i32("max_int32", args_i32_i32(a, b)))
    assert_equal(
        Int(w[].call_i32("le_int32", args_i32_i32(lo, hi))),
        1,
        "min(x,y) <= max(x,y)",
    )


fn test_bitwise_identity_and_or_xor(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """(x & y) | (x ^ y) === x | y."""
    var x = 0b1100
    var y = 0b1010
    var band = Int(w[].call_i32("bitand_int32", args_i32_i32(x, y)))
    var bxor = Int(w[].call_i32("bitxor_int32", args_i32_i32(x, y)))
    var lhs = Int(w[].call_i32("bitor_int32", args_i32_i32(band, bxor)))
    var rhs = Int(w[].call_i32("bitor_int32", args_i32_i32(x, y)))
    assert_equal(lhs, rhs, "(x & y) | (x ^ y) === x | y")


fn test_shl_shr_roundtrip(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """shr(shl(x, 4), 4) === x."""
    var x = 5
    var shifted = Int(w[].call_i32("shl_int32", args_i32_i32(x, 4)))
    assert_equal(
        Int(w[].call_i32("shr_int32", args_i32_i32(shifted, 4))),
        x,
        "shr(shl(x, 4), 4) === x",
    )


fn test_de_morgan(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    """De Morgan: not(and(a,b)) === or(not(a), not(b))."""
    var a = 1
    var b = 0
    var and_ab = Int(w[].call_i32("bool_and", args_i32_i32(a, b)))
    var lhs = Int(w[].call_i32("bool_not", args_i32(and_ab)))
    var not_a = Int(w[].call_i32("bool_not", args_i32(a)))
    var not_b = Int(w[].call_i32("bool_not", args_i32(b)))
    var rhs = Int(w[].call_i32("bool_or", args_i32_i32(not_a, not_b)))
    assert_equal(lhs, rhs, "De Morgan: not(and(a,b)) === or(not(a), not(b))")


fn test_gcd_scaling(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    """gcd(a*k, b*k) === k * gcd(a, b)."""
    var a = 6
    var b = 4
    var k = 5
    var ak = Int(w[].call_i32("mul_int32", args_i32_i32(a, k)))
    var bk = Int(w[].call_i32("mul_int32", args_i32_i32(b, k)))
    var lhs = Int(w[].call_i32("gcd_int32", args_i32_i32(ak, bk)))
    var gcd_ab = Int(w[].call_i32("gcd_int32", args_i32_i32(a, b)))
    var rhs = Int(w[].call_i32("mul_int32", args_i32_i32(k, gcd_ab)))
    assert_equal(lhs, rhs, "gcd(a*k, b*k) === k * gcd(a, b)")


fn test_fibonacci_recurrence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """fib(n) === fib(n-1) + fib(n-2) for several values of n."""
    var ns: List[Int] = [5, 8, 12, 15]
    for i in range(len(ns)):
        var n = ns[i]
        var fn2 = Int(w[].call_i32("fib_int32", args_i32(n - 2)))
        var fn1 = Int(w[].call_i32("fib_int32", args_i32(n - 1)))
        var fn0 = Int(w[].call_i32("fib_int32", args_i32(n)))
        assert_equal(
            fn0,
            fn1 + fn2,
            String("fib(") + String(n) + ") === fib(n-1) + fib(n-2)",
        )


fn test_factorial_recurrence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """factorial(n) === n * factorial(n-1) for n = 2..7."""
    var ns: List[Int] = [2, 3, 4, 5, 6, 7]
    for i in range(len(ns)):
        var n = ns[i]
        var fn0 = Int(w[].call_i32("factorial_int32", args_i32(n)))
        var fn1 = Int(w[].call_i32("factorial_int32", args_i32(n - 1)))
        assert_equal(
            fn0,
            n * fn1,
            String("factorial(") + String(n) + ") === n * factorial(n-1)",
        )


fn test_string_concat_length(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """len(concat(a, b)) === len(a) + len(b)."""
    var a_ptr = w[].write_string_struct("foo")
    var b_ptr = w[].write_string_struct("barbaz")
    var out_ptr = w[].alloc_string_struct()
    w[].call_void("string_concat", args_ptr_ptr_ptr(a_ptr, b_ptr, out_ptr))
    var concat_len = Int(w[].call_i64("string_length", args_ptr(out_ptr)))
    var a_len = Int(w[].call_i64("string_length", args_ptr(a_ptr)))
    var b_len = Int(w[].call_i64("string_length", args_ptr(b_ptr)))
    assert_equal(
        concat_len, a_len + b_len, "len(concat(a,b)) === len(a) + len(b)"
    )


fn test_clamp_eq_max_lo_min_hi_x(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """clamp(x, lo, hi) === max(lo, min(hi, x)) for several values of x."""
    var lo = 0
    var hi = 10
    var xs: List[Int] = [-5, 0, 5, 10, 15]
    for i in range(len(xs)):
        var x = xs[i]
        var lhs = Int(w[].call_i32("clamp_int32", args_i32_i32_i32(x, lo, hi)))
        var min_hi_x = Int(w[].call_i32("min_int32", args_i32_i32(hi, x)))
        var rhs = Int(w[].call_i32("max_int32", args_i32_i32(lo, min_hi_x)))
        assert_equal(
            lhs,
            rhs,
            String("clamp(")
            + String(x)
            + ", "
            + String(lo)
            + ", "
            + String(hi)
            + ") === max(lo, min(hi, x))",
        )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_add_sub_inverse(w)
    test_mul_div_inverse(w)
    test_neg_neg_identity(w)
    test_abs_neg_eq_abs(w)
    test_min_le_max(w)
    test_bitwise_identity_and_or_xor(w)
    test_shl_shr_roundtrip(w)
    test_de_morgan(w)
    test_gcd_scaling(w)
    test_fibonacci_recurrence(w)
    test_factorial_recurrence(w)
    test_string_concat_length(w)
    test_clamp_eq_max_lo_min_hi_x(w)
    print("consistency: 13/13 passed")
