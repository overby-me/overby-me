# Algorithm tests (fib, factorial, gcd) exercised through the real WASM binary
# via mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that fibonacci, factorial, and GCD algorithms work correctly
# when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_algorithms.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
    args_i64,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Fibonacci — int32 ────────────────────────────────────────────────────────


fn test_fib_int32_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(0))), 0, "fib_int32(0) === 0"
    )


fn test_fib_int32_one(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(1))), 1, "fib_int32(1) === 1"
    )


fn test_fib_int32_two(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(2))), 1, "fib_int32(2) === 1"
    )


fn test_fib_int32_three(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(3))), 2, "fib_int32(3) === 2"
    )


fn test_fib_int32_four(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(4))), 3, "fib_int32(4) === 3"
    )


fn test_fib_int32_five(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(5))), 5, "fib_int32(5) === 5"
    )


fn test_fib_int32_six(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(6))), 8, "fib_int32(6) === 8"
    )


fn test_fib_int32_seven(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(7))),
        13,
        "fib_int32(7) === 13",
    )


fn test_fib_int32_ten(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(10))),
        55,
        "fib_int32(10) === 55",
    )


fn test_fib_int32_twenty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(20))),
        6765,
        "fib_int32(20) === 6765",
    )


# ── Fibonacci — int64 ────────────────────────────────────────────────────────


fn test_fib_int64_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("fib_int64", args_i64(0))), 0, "fib_int64(0) === 0"
    )


fn test_fib_int64_one(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("fib_int64", args_i64(1))), 1, "fib_int64(1) === 1"
    )


fn test_fib_int64_ten(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("fib_int64", args_i64(10))),
        55,
        "fib_int64(10) === 55",
    )


fn test_fib_int64_twenty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("fib_int64", args_i64(20))),
        6765,
        "fib_int64(20) === 6765",
    )


fn test_fib_int64_fifty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("fib_int64", args_i64(50))),
        12586269025,
        "fib_int64(50) === 12586269025",
    )


# ── Factorial — int32 ────────────────────────────────────────────────────────


fn test_factorial_int32_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(0))),
        1,
        "factorial_int32(0) === 1",
    )


fn test_factorial_int32_one(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(1))),
        1,
        "factorial_int32(1) === 1",
    )


fn test_factorial_int32_two(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(2))),
        2,
        "factorial_int32(2) === 2",
    )


fn test_factorial_int32_three(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(3))),
        6,
        "factorial_int32(3) === 6",
    )


fn test_factorial_int32_four(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(4))),
        24,
        "factorial_int32(4) === 24",
    )


fn test_factorial_int32_five(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(5))),
        120,
        "factorial_int32(5) === 120",
    )


fn test_factorial_int32_ten(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(10))),
        3628800,
        "factorial_int32(10) === 3628800",
    )


# ── Factorial — int64 ────────────────────────────────────────────────────────


fn test_factorial_int64_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(0))),
        1,
        "factorial_int64(0) === 1",
    )


fn test_factorial_int64_one(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(1))),
        1,
        "factorial_int64(1) === 1",
    )


fn test_factorial_int64_five(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(5))),
        120,
        "factorial_int64(5) === 120",
    )


fn test_factorial_int64_ten(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(10))),
        3628800,
        "factorial_int64(10) === 3628800",
    )


fn test_factorial_int64_twenty(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(20))),
        2432902008176640000,
        "factorial_int64(20) === 2432902008176640000",
    )


# ── GCD (Euclidean algorithm) ────────────────────────────────────────────────


fn test_gcd_12_8(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(12, 8))),
        4,
        "gcd_int32(12, 8) === 4",
    )


fn test_gcd_commutative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(8, 12))),
        4,
        "gcd_int32(8, 12) === 4 (commutative)",
    )


fn test_gcd_coprime(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(7, 13))),
        1,
        "gcd_int32(7, 13) === 1 (coprime)",
    )


fn test_gcd_100_75(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(100, 75))),
        25,
        "gcd_int32(100, 75) === 25",
    )


fn test_gcd_zero_first(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(0, 5))),
        5,
        "gcd_int32(0, 5) === 5",
    )


fn test_gcd_zero_second(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(5, 0))),
        5,
        "gcd_int32(5, 0) === 5",
    )


fn test_gcd_same(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(7, 7))),
        7,
        "gcd_int32(7, 7) === 7",
    )


fn test_gcd_one(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(1, 100))),
        1,
        "gcd_int32(1, 100) === 1",
    )


fn test_gcd_negative_first(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(-12, 8))),
        4,
        "gcd_int32(-12, 8) === 4 (negative input)",
    )


fn test_gcd_negative_second(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(12, -8))),
        4,
        "gcd_int32(12, -8) === 4 (negative input)",
    )


fn test_gcd_both_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(-12, -8))),
        4,
        "gcd_int32(-12, -8) === 4 (both negative)",
    )


fn test_gcd_48_18(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(48, 18))),
        6,
        "gcd_int32(48, 18) === 6",
    )


fn test_gcd_classic_euclid(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(1071, 462))),
        21,
        "gcd_int32(1071, 462) === 21 (classic Euclid example)",
    )


# ── Fibonacci recurrence property ────────────────────────────────────────────
# fib(n) === fib(n-1) + fib(n-2) for n >= 2


fn test_fib_recurrence_property(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    for n in range(2, 21):
        var fn0 = Int(w[].call_i32("fib_int32", args_i32(Int32(n))))
        var fn1 = Int(w[].call_i32("fib_int32", args_i32(Int32(n - 1))))
        var fn2 = Int(w[].call_i32("fib_int32", args_i32(Int32(n - 2))))
        assert_equal(
            fn0,
            fn1 + fn2,
            String("fib(")
            + String(n)
            + ") === fib("
            + String(n - 1)
            + ") + fib("
            + String(n - 2)
            + ")",
        )


# ── Factorial recurrence property ────────────────────────────────────────────
# n! === n * (n-1)! for n >= 2


fn test_factorial_recurrence_property(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    for n in range(2, 11):
        var fn0 = Int(w[].call_i32("factorial_int32", args_i32(Int32(n))))
        var fn1 = Int(w[].call_i32("factorial_int32", args_i32(Int32(n - 1))))
        assert_equal(
            fn0,
            n * fn1,
            String("factorial(")
            + String(n)
            + ") === "
            + String(n)
            + " * factorial("
            + String(n - 1)
            + ")",
        )


# ── GCD properties ───────────────────────────────────────────────────────────


fn test_gcd_commutative_property(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """gcd(a, b) === gcd(b, a) for several pairs."""
    var as_: List[Int] = [12, 7, 100, 0, 1071]
    var bs: List[Int] = [8, 13, 75, 5, 462]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("gcd_int32", args_i32_i32(Int32(a), Int32(b)))),
            Int(w[].call_i32("gcd_int32", args_i32_i32(Int32(b), Int32(a)))),
            String("gcd(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_gcd_idempotent(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """gcd(a, a) === a for positive values."""
    for v in range(1, 20):
        assert_equal(
            Int(w[].call_i32("gcd_int32", args_i32_i32(Int32(v), Int32(v)))),
            v,
            String("gcd(")
            + String(v)
            + ", "
            + String(v)
            + ") === "
            + String(v),
        )


fn test_gcd_with_one(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    """gcd(1, n) === 1 for any positive n."""
    for v in range(1, 20):
        assert_equal(
            Int(w[].call_i32("gcd_int32", args_i32_i32(1, Int32(v)))),
            1,
            String("gcd(1, ") + String(v) + ") === 1",
        )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_fib_int32_zero(w)
    test_fib_int32_one(w)
    test_fib_int32_two(w)
    test_fib_int32_three(w)
    test_fib_int32_four(w)
    test_fib_int32_five(w)
    test_fib_int32_six(w)
    test_fib_int32_seven(w)
    test_fib_int32_ten(w)
    test_fib_int32_twenty(w)
    test_fib_int64_zero(w)
    test_fib_int64_one(w)
    test_fib_int64_ten(w)
    test_fib_int64_twenty(w)
    test_fib_int64_fifty(w)
    test_factorial_int32_zero(w)
    test_factorial_int32_one(w)
    test_factorial_int32_two(w)
    test_factorial_int32_three(w)
    test_factorial_int32_four(w)
    test_factorial_int32_five(w)
    test_factorial_int32_ten(w)
    test_factorial_int64_zero(w)
    test_factorial_int64_one(w)
    test_factorial_int64_five(w)
    test_factorial_int64_ten(w)
    test_factorial_int64_twenty(w)
    test_gcd_12_8(w)
    test_gcd_commutative(w)
    test_gcd_coprime(w)
    test_gcd_100_75(w)
    test_gcd_zero_first(w)
    test_gcd_zero_second(w)
    test_gcd_same(w)
    test_gcd_one(w)
    test_gcd_negative_first(w)
    test_gcd_negative_second(w)
    test_gcd_both_negative(w)
    test_gcd_48_18(w)
    test_gcd_classic_euclid(w)
    test_fib_recurrence_property(w)
    test_factorial_recurrence_property(w)
    test_gcd_commutative_property(w)
    test_gcd_idempotent(w)
    test_gcd_with_one(w)
    print("algorithms: 45/45 passed")
