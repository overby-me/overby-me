# Integer boundary and overflow tests exercised through the real WASM binary
# via mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests are especially critical to run through the actual WASM binary,
# because integer overflow/wrapping behavior could differ between native Mojo
# and the WASM target (e.g. 2's complement wrapping, truncation semantics).
#
# Run with:
#   mojo test test/test_boundaries.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
    args_i32_i32_i32,
    args_i64,
    args_i64_i64,
    args_f64_f64_f64,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Constants ────────────────────────────────────────────────────────────────

comptime INT32_MAX = 2147483647
comptime INT32_MIN = -2147483648
comptime INT64_MAX = 9223372036854775807
comptime INT64_MIN = -9223372036854775808


# ── Int32 boundary values — identity ─────────────────────────────────────────


fn test_identity_int32_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("identity_int32", args_i32(INT32_MAX))),
        INT32_MAX,
        "identity_int32(INT32_MAX)",
    )


fn test_identity_int32_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("identity_int32", args_i32(INT32_MIN))),
        INT32_MIN,
        "identity_int32(INT32_MIN)",
    )


fn test_identity_int32_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("identity_int32", args_i32(0))),
        0,
        "identity_int32(0)",
    )


# ── Int64 boundary values — identity ─────────────────────────────────────────


fn test_identity_int64_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("identity_int64", args_i64(INT64_MAX))),
        INT64_MAX,
        "identity_int64(INT64_MAX)",
    )


fn test_identity_int64_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("identity_int64", args_i64(INT64_MIN))),
        INT64_MIN,
        "identity_int64(INT64_MIN)",
    )


# ── Int32 overflow — addition ────────────────────────────────────────────────


fn test_add_int32_max_plus_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(INT32_MAX, 1))),
        INT32_MIN,
        "add_int32(INT32_MAX, 1) wraps to INT32_MIN",
    )


fn test_add_int32_min_minus_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(INT32_MIN, -1))),
        INT32_MAX,
        "add_int32(INT32_MIN, -1) wraps to INT32_MAX",
    )


fn test_add_int32_max_plus_max_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(INT32_MAX, INT32_MAX))),
        -2,
        "add_int32(INT32_MAX, INT32_MAX) wraps to -2",
    )


# ── Int64 overflow — addition ────────────────────────────────────────────────


fn test_add_int64_max_plus_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("add_int64", args_i64_i64(INT64_MAX, 1))),
        INT64_MIN,
        "add_int64(INT64_MAX, 1) wraps to INT64_MIN",
    )


fn test_add_int64_min_minus_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("add_int64", args_i64_i64(INT64_MIN, -1))),
        INT64_MAX,
        "add_int64(INT64_MIN, -1) wraps to INT64_MAX",
    )


# ── Int32 overflow — subtraction ─────────────────────────────────────────────


fn test_sub_int32_min_minus_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(INT32_MIN, 1))),
        INT32_MAX,
        "sub_int32(INT32_MIN, 1) wraps to INT32_MAX",
    )


fn test_sub_int32_max_minus_neg_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(INT32_MAX, -1))),
        INT32_MIN,
        "sub_int32(INT32_MAX, -1) wraps to INT32_MIN",
    )


# ── Int32 overflow — multiplication ──────────────────────────────────────────


fn test_mul_int32_max_times_two_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(INT32_MAX, 2))),
        -2,
        "mul_int32(INT32_MAX, 2) wraps to -2",
    )


fn test_mul_int32_min_times_neg_one_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(INT32_MIN, -1))),
        INT32_MIN,
        "mul_int32(INT32_MIN, -1) wraps to INT32_MIN (no positive equivalent)",
    )


# ── Int32 overflow — negation ────────────────────────────────────────────────


fn test_neg_int32_max(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("neg_int32", args_i32(INT32_MAX))),
        -INT32_MAX,
        "neg_int32(INT32_MAX) === -INT32_MAX",
    )


fn test_neg_int32_min_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("neg_int32", args_i32(INT32_MIN))),
        INT32_MIN,
        "neg_int32(INT32_MIN) wraps to INT32_MIN (2's complement)",
    )


# ── Int64 overflow — negation ────────────────────────────────────────────────


fn test_neg_int64_max(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("neg_int64", args_i64(INT64_MAX))),
        -INT64_MAX,
        "neg_int64(INT64_MAX) === -INT64_MAX",
    )


fn test_neg_int64_min_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("neg_int64", args_i64(INT64_MIN))),
        INT64_MIN,
        "neg_int64(INT64_MIN) wraps to INT64_MIN (2's complement)",
    )


# ── Int32 boundary — abs ─────────────────────────────────────────────────────


fn test_abs_int32_max(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("abs_int32", args_i32(INT32_MAX))),
        INT32_MAX,
        "abs_int32(INT32_MAX) === INT32_MAX",
    )


fn test_abs_int32_min_wraps(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("abs_int32", args_i32(INT32_MIN))),
        INT32_MIN,
        "abs_int32(INT32_MIN) wraps to INT32_MIN (no positive equivalent)",
    )


fn test_abs_int32_min_plus_one(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("abs_int32", args_i32(INT32_MIN + 1))),
        INT32_MAX,
        "abs_int32(INT32_MIN + 1) === INT32_MAX",
    )


# ── Int32 boundary — min / max ───────────────────────────────────────────────


fn test_min_int32_boundaries(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("min_int32", args_i32_i32(INT32_MIN, INT32_MAX))),
        INT32_MIN,
        "min_int32(INT32_MIN, INT32_MAX) === INT32_MIN",
    )


fn test_max_int32_boundaries(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("max_int32", args_i32_i32(INT32_MIN, INT32_MAX))),
        INT32_MAX,
        "max_int32(INT32_MIN, INT32_MAX) === INT32_MAX",
    )


fn test_min_int32_same_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("min_int32", args_i32_i32(INT32_MIN, INT32_MIN))),
        INT32_MIN,
        "min_int32(INT32_MIN, INT32_MIN) === INT32_MIN",
    )


fn test_max_int32_same_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("max_int32", args_i32_i32(INT32_MAX, INT32_MAX))),
        INT32_MAX,
        "max_int32(INT32_MAX, INT32_MAX) === INT32_MAX",
    )


# ── Int32 boundary — comparison ──────────────────────────────────────────────


fn test_lt_int32_min_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("lt_int32", args_i32_i32(INT32_MIN, INT32_MAX))),
        1,
        "INT32_MIN < INT32_MAX",
    )


fn test_gt_int32_max_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gt_int32", args_i32_i32(INT32_MAX, INT32_MIN))),
        1,
        "INT32_MAX > INT32_MIN",
    )


fn test_eq_int32_max_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("eq_int32", args_i32_i32(INT32_MAX, INT32_MAX))),
        1,
        "INT32_MAX === INT32_MAX",
    )


fn test_eq_int32_min_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("eq_int32", args_i32_i32(INT32_MIN, INT32_MIN))),
        1,
        "INT32_MIN === INT32_MIN",
    )


fn test_ne_int32_min_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("ne_int32", args_i32_i32(INT32_MIN, INT32_MAX))),
        1,
        "INT32_MIN !== INT32_MAX",
    )


# ── Int32 boundary — clamp ───────────────────────────────────────────────────


fn test_clamp_int32_min_to_range(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("clamp_int32", args_i32_i32_i32(INT32_MIN, 0, 100))),
        0,
        "clamp_int32(INT32_MIN, 0, 100) === 0",
    )


fn test_clamp_int32_max_to_range(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("clamp_int32", args_i32_i32_i32(INT32_MAX, 0, 100))),
        100,
        "clamp_int32(INT32_MAX, 0, 100) === 100",
    )


fn test_clamp_int32_within_full_range(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(
            w[].call_i32(
                "clamp_int32", args_i32_i32_i32(50, INT32_MIN, INT32_MAX)
            )
        ),
        50,
        "clamp_int32(50, INT32_MIN, INT32_MAX) === 50",
    )


# ── Int32 boundary — bitwise ─────────────────────────────────────────────────


fn test_bitnot_int32_max(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitnot_int32", args_i32(INT32_MAX))),
        INT32_MIN,
        "bitnot_int32(INT32_MAX) === INT32_MIN",
    )


fn test_bitnot_int32_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitnot_int32", args_i32(INT32_MIN))),
        INT32_MAX,
        "bitnot_int32(INT32_MIN) === INT32_MAX",
    )


fn test_bitand_int32_max_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitand_int32", args_i32_i32(INT32_MAX, INT32_MIN))),
        0,
        "bitand_int32(INT32_MAX, INT32_MIN) === 0",
    )


fn test_bitor_int32_max_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitor_int32", args_i32_i32(INT32_MAX, INT32_MIN))),
        -1,
        "bitor_int32(INT32_MAX, INT32_MIN) === -1 (all bits set)",
    )


fn test_bitxor_int32_max_min(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitxor_int32", args_i32_i32(INT32_MAX, INT32_MIN))),
        -1,
        "bitxor_int32(INT32_MAX, INT32_MIN) === -1",
    )


# ── Int32 boundary — GCD ─────────────────────────────────────────────────────


fn test_gcd_int32_max_with_one(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(INT32_MAX, 1))),
        1,
        "gcd_int32(INT32_MAX, 1) === 1",
    )


fn test_gcd_int32_max_with_self(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("gcd_int32", args_i32_i32(INT32_MAX, INT32_MAX))),
        INT32_MAX,
        "gcd_int32(INT32_MAX, INT32_MAX) === INT32_MAX",
    )


# ── Int32 overflow — factorial ───────────────────────────────────────────────


fn test_factorial_int32_12_fits(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # 12! = 479001600, fits in Int32
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(12))),
        479001600,
        "factorial_int32(12) === 479001600",
    )


fn test_factorial_int32_13_overflows(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # 13! = 6227020800, overflows Int32 — verify it wraps
    assert_equal(
        Int(w[].call_i32("factorial_int32", args_i32(13))),
        1932053504,
        "factorial_int32(13) wraps (6227020800 truncated to i32)",
    )


# ── Int64 boundary — factorial ───────────────────────────────────────────────


fn test_factorial_int64_20_fits(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # 20! = 2432902008176640000, fits in Int64
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(20))),
        2432902008176640000,
        "factorial_int64(20) === 2432902008176640000",
    )


fn test_factorial_int64_21_overflows(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # 21! = 51090942171709440000, overflows Int64 — verify it wraps
    assert_equal(
        Int(w[].call_i64("factorial_int64", args_i64(21))),
        -4249290049419214848,
        "factorial_int64(21) wraps (overflow)",
    )


# ── Int32 overflow — fibonacci ───────────────────────────────────────────────


fn test_fib_int32_46_fits(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # fib(46) = 1836311903, fits in Int32
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(46))),
        1836311903,
        "fib_int32(46) === 1836311903",
    )


fn test_fib_int32_47_overflows(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # fib(47) = 2971215073, overflows Int32 — verify wrapping
    assert_equal(
        Int(w[].call_i32("fib_int32", args_i32(47))),
        -1323752223,
        "fib_int32(47) wraps (2971215073 truncated to i32)",
    )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_identity_int32_max(w)
    test_identity_int32_min(w)
    test_identity_int32_zero(w)
    test_identity_int64_max(w)
    test_identity_int64_min(w)
    test_add_int32_max_plus_one_wraps(w)
    test_add_int32_min_minus_one_wraps(w)
    test_add_int32_max_plus_max_wraps(w)
    test_add_int64_max_plus_one_wraps(w)
    test_add_int64_min_minus_one_wraps(w)
    test_sub_int32_min_minus_one_wraps(w)
    test_sub_int32_max_minus_neg_one_wraps(w)
    test_mul_int32_max_times_two_wraps(w)
    test_mul_int32_min_times_neg_one_wraps(w)
    test_neg_int32_max(w)
    test_neg_int32_min_wraps(w)
    test_neg_int64_max(w)
    test_neg_int64_min_wraps(w)
    test_abs_int32_max(w)
    test_abs_int32_min_wraps(w)
    test_abs_int32_min_plus_one(w)
    test_min_int32_boundaries(w)
    test_max_int32_boundaries(w)
    test_min_int32_same_min(w)
    test_max_int32_same_max(w)
    test_lt_int32_min_max(w)
    test_gt_int32_max_min(w)
    test_eq_int32_max_max(w)
    test_eq_int32_min_min(w)
    test_ne_int32_min_max(w)
    test_clamp_int32_min_to_range(w)
    test_clamp_int32_max_to_range(w)
    test_clamp_int32_within_full_range(w)
    test_bitnot_int32_max(w)
    test_bitnot_int32_min(w)
    test_bitand_int32_max_min(w)
    test_bitor_int32_max_min(w)
    test_bitxor_int32_max_min(w)
    test_gcd_int32_max_with_one(w)
    test_gcd_int32_max_with_self(w)
    test_factorial_int32_12_fits(w)
    test_factorial_int32_13_overflows(w)
    test_factorial_int64_20_fits(w)
    test_factorial_int64_21_overflows(w)
    test_fib_int32_46_fits(w)
    test_fib_int32_47_overflows(w)
    print("boundaries: 46/46 passed")
