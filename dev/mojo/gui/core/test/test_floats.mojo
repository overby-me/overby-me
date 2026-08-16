# Port of test/floats.test.ts — float NaN, Infinity, negative zero, subnormal,
# and precision edge-case tests exercised through the real WASM binary via
# mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# Run with:
#   mojo test test/test_floats.mojo

from math import nan as _get_nan, inf as _get_inf, isnan, copysign
from memory import UnsafePointer
from testing import assert_true, assert_equal

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_f32,
    args_f32_f32,
    args_f64,
    args_f64_f64,
    args_f64_f64_f64,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

comptime NaN = _get_nan[DType.float64]()
comptime Inf = _get_inf[DType.float64]()
comptime NegInf = -_get_inf[DType.float64]()
comptime NaN32 = _get_nan[DType.float32]()
comptime Inf32 = _get_inf[DType.float32]()
comptime NegInf32 = -_get_inf[DType.float32]()


fn _assert_nan_f64(result: Float64, label: String) raises:
    assert_true(isnan(result), label + ": expected NaN")


fn _assert_nan_f32(result: Float32, label: String) raises:
    assert_true(isnan(result), label + ": expected NaN")


fn _is_negative_zero(x: Float64) -> Bool:
    return x == 0.0 and copysign(Float64(1.0), x) < 0


fn _assert_eq_f64(actual: Float64, expected: Float64, label: String) raises:
    """Strict equality including sign of zero."""
    # Check for expected -0.0
    if expected == 0.0 and copysign(Float64(1.0), expected) < 0:
        assert_true(
            _is_negative_zero(actual),
            label + ": expected -0.0",
        )
    # Check for expected +0.0
    elif expected == 0.0 and copysign(Float64(1.0), expected) > 0:
        assert_true(
            actual == 0.0 and not _is_negative_zero(actual),
            label + ": expected +0.0",
        )
    # Check for expected Inf / -Inf
    elif expected == Inf:
        assert_true(
            actual == Inf,
            label + ": expected Inf",
        )
    elif expected == NegInf:
        assert_true(
            actual == NegInf,
            label + ": expected -Inf",
        )
    else:
        assert_true(
            actual == expected,
            label,
        )


# ---------------------------------------------------------------------------
# NaN propagation — arithmetic
# ---------------------------------------------------------------------------


fn test_add_float64_nan_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("add_float64", args_f64_f64(NaN, 1.0)),
        "add_float64(NaN, 1.0)",
    )


fn test_add_float64_1_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("add_float64", args_f64_f64(1.0, NaN)),
        "add_float64(1.0, NaN)",
    )


fn test_add_float64_nan_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("add_float64", args_f64_f64(NaN, NaN)),
        "add_float64(NaN, NaN)",
    )


fn test_sub_float64_nan_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("sub_float64", args_f64_f64(NaN, 1.0)),
        "sub_float64(NaN, 1.0)",
    )


fn test_sub_float64_1_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("sub_float64", args_f64_f64(1.0, NaN)),
        "sub_float64(1.0, NaN)",
    )


fn test_mul_float64_nan_2(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("mul_float64", args_f64_f64(NaN, 2.0)),
        "mul_float64(NaN, 2.0)",
    )


fn test_mul_float64_2_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("mul_float64", args_f64_f64(2.0, NaN)),
        "mul_float64(2.0, NaN)",
    )


fn test_div_float64_nan_2(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("div_float64", args_f64_f64(NaN, 2.0)),
        "div_float64(NaN, 2.0)",
    )


fn test_div_float64_2_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("div_float64", args_f64_f64(2.0, NaN)),
        "div_float64(2.0, NaN)",
    )


# ---------------------------------------------------------------------------
# NaN propagation — float32
# ---------------------------------------------------------------------------


fn test_add_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("add_float32", args_f32_f32(NaN32, Float32(1.0))),
        "add_float32(NaN, 1.0)",
    )


fn test_sub_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("sub_float32", args_f32_f32(NaN32, Float32(1.0))),
        "sub_float32(NaN, 1.0)",
    )


fn test_mul_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("mul_float32", args_f32_f32(NaN32, Float32(2.0))),
        "mul_float32(NaN, 2.0)",
    )


fn test_div_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("div_float32", args_f32_f32(NaN32, Float32(2.0))),
        "div_float32(NaN, 2.0)",
    )


# ---------------------------------------------------------------------------
# NaN-producing operations
# ---------------------------------------------------------------------------


fn test_add_inf_neg_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("add_float64", args_f64_f64(Inf, NegInf)),
        "add_float64(Inf, -Inf)",
    )


fn test_sub_inf_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_nan_f64(
        w[].call_f64("sub_float64", args_f64_f64(Inf, Inf)),
        "sub_float64(Inf, Inf)",
    )


fn test_mul_0_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_nan_f64(
        w[].call_f64("mul_float64", args_f64_f64(0.0, Inf)),
        "mul_float64(0, Inf)",
    )


fn test_div_0_0(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_nan_f64(
        w[].call_f64("div_float64", args_f64_f64(0.0, 0.0)),
        "div_float64(0, 0)",
    )


# ---------------------------------------------------------------------------
# NaN propagation — unary
# ---------------------------------------------------------------------------


fn test_neg_float64_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("neg_float64", args_f64(NaN)),
        "neg_float64(NaN)",
    )


fn test_neg_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("neg_float32", args_f32(NaN32)),
        "neg_float32(NaN)",
    )


fn test_abs_float64_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("abs_float64", args_f64(NaN)),
        "abs_float64(NaN)",
    )


fn test_abs_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("abs_float32", args_f32(NaN32)),
        "abs_float32(NaN)",
    )


# ---------------------------------------------------------------------------
# NaN propagation — identity
# ---------------------------------------------------------------------------


fn test_identity_float64_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("identity_float64", args_f64(NaN)),
        "identity_float64(NaN)",
    )


fn test_identity_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("identity_float32", args_f32(NaN32)),
        "identity_float32(NaN)",
    )


# ---------------------------------------------------------------------------
# NaN — min/max (comparison quirk)
#
# Mojo uses `if x < y: return x; return y` — NaN comparisons always return
# false, so the "else" branch wins.
# ---------------------------------------------------------------------------


fn test_min_nan_5(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("min_float64", args_f64_f64(NaN, 5.0)),
        5.0,
        "min_float64(NaN, 5.0) === 5.0",
    )


fn test_min_5_nan(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_nan_f64(
        w[].call_f64("min_float64", args_f64_f64(5.0, NaN)),
        "min_float64(5.0, NaN) === NaN",
    )


fn test_max_nan_5(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("max_float64", args_f64_f64(NaN, 5.0)),
        5.0,
        "max_float64(NaN, 5.0) === 5.0",
    )


fn test_max_5_nan(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_nan_f64(
        w[].call_f64("max_float64", args_f64_f64(5.0, NaN)),
        "max_float64(5.0, NaN) === NaN",
    )


# ---------------------------------------------------------------------------
# NaN — power
# ---------------------------------------------------------------------------


fn test_pow_float64_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f64(
        w[].call_f64("pow_float64", args_f64(NaN)),
        "pow_float64(NaN)",
    )


fn test_pow_float32_nan(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_nan_f32(
        w[].call_f32("pow_float32", args_f32(NaN32)),
        "pow_float32(NaN)",
    )


# ---------------------------------------------------------------------------
# Infinity — arithmetic
# ---------------------------------------------------------------------------


fn test_add_inf_1(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("add_float64", args_f64_f64(Inf, 1.0)),
        Inf,
        "add_float64(Inf, 1) === Inf",
    )


fn test_add_neg_inf_neg1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_eq_f64(
        w[].call_f64("add_float64", args_f64_f64(NegInf, -1.0)),
        NegInf,
        "add_float64(-Inf, -1) === -Inf",
    )


fn test_sub_inf_1(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("sub_float64", args_f64_f64(Inf, 1.0)),
        Inf,
        "sub_float64(Inf, 1) === Inf",
    )


fn test_mul_inf_2(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("mul_float64", args_f64_f64(Inf, 2.0)),
        Inf,
        "mul_float64(Inf, 2) === Inf",
    )


fn test_mul_inf_neg2(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("mul_float64", args_f64_f64(Inf, -2.0)),
        NegInf,
        "mul_float64(Inf, -2) === -Inf",
    )


fn test_div_1_0(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("div_float64", args_f64_f64(1.0, 0.0)),
        Inf,
        "div_float64(1, 0) === Inf",
    )


fn test_div_neg1_0(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("div_float64", args_f64_f64(-1.0, 0.0)),
        NegInf,
        "div_float64(-1, 0) === -Inf",
    )


fn test_div_1_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("div_float64", args_f64_f64(1.0, Inf)),
        0.0,
        "div_float64(1, Inf) === 0",
    )


# ---------------------------------------------------------------------------
# Infinity — float32
# ---------------------------------------------------------------------------


fn test_add_float32_inf_1(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Float64(
        w[].call_f32("add_float32", args_f32_f32(Inf32, Float32(1.0)))
    )
    assert_true(
        result == Float64(Inf32),
        "add_float32(Inf, 1) === Inf",
    )


fn test_div_float32_1_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Float64(
        w[].call_f32("div_float32", args_f32_f32(Float32(1.0), Float32(0.0)))
    )
    assert_true(
        result == Float64(Inf32),
        "div_float32(1, 0) === Inf",
    )


fn test_div_float32_neg1_0(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Float64(
        w[].call_f32("div_float32", args_f32_f32(Float32(-1.0), Float32(0.0)))
    )
    assert_true(
        result == Float64(NegInf32),
        "div_float32(-1, 0) === -Inf",
    )


# ---------------------------------------------------------------------------
# Infinity — unary
# ---------------------------------------------------------------------------


fn test_neg_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("neg_float64", args_f64(Inf)),
        NegInf,
        "neg_float64(Inf) === -Inf",
    )


fn test_neg_neg_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("neg_float64", args_f64(NegInf)),
        Inf,
        "neg_float64(-Inf) === Inf",
    )


fn test_abs_neg_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("abs_float64", args_f64(NegInf)),
        Inf,
        "abs_float64(-Inf) === Inf",
    )


fn test_abs_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("abs_float64", args_f64(Inf)),
        Inf,
        "abs_float64(Inf) === Inf",
    )


# ---------------------------------------------------------------------------
# Infinity — identity
# ---------------------------------------------------------------------------


fn test_identity_float64_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_eq_f64(
        w[].call_f64("identity_float64", args_f64(Inf)),
        Inf,
        "identity_float64(Inf) === Inf",
    )


fn test_identity_float64_neg_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_eq_f64(
        w[].call_f64("identity_float64", args_f64(NegInf)),
        NegInf,
        "identity_float64(-Inf) === -Inf",
    )


fn test_identity_float32_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Float64(w[].call_f32("identity_float32", args_f32(Inf32)))
    assert_true(
        result == Float64(Inf32),
        "identity_float32(Inf) === Inf",
    )


# ---------------------------------------------------------------------------
# Infinity — min/max
# ---------------------------------------------------------------------------


fn test_min_neg_inf_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_eq_f64(
        w[].call_f64("min_float64", args_f64_f64(NegInf, Inf)),
        NegInf,
        "min_float64(-Inf, Inf) === -Inf",
    )


fn test_max_neg_inf_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_eq_f64(
        w[].call_f64("max_float64", args_f64_f64(NegInf, Inf)),
        Inf,
        "max_float64(-Inf, Inf) === Inf",
    )


fn test_min_42_neg_inf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    _assert_eq_f64(
        w[].call_f64("min_float64", args_f64_f64(42.0, NegInf)),
        NegInf,
        "min_float64(42, -Inf) === -Inf",
    )


fn test_max_42_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("max_float64", args_f64_f64(42.0, Inf)),
        Inf,
        "max_float64(42, Inf) === Inf",
    )


# ---------------------------------------------------------------------------
# Infinity — clamp
# ---------------------------------------------------------------------------


fn test_clamp_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("clamp_float64", args_f64_f64_f64(Inf, 0.0, 10.0)),
        10.0,
        "clamp_float64(Inf, 0, 10) === 10",
    )


fn test_clamp_neg_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("clamp_float64", args_f64_f64_f64(NegInf, 0.0, 10.0)),
        0.0,
        "clamp_float64(-Inf, 0, 10) === 0",
    )


# ---------------------------------------------------------------------------
# Negative zero (-0.0)
# ---------------------------------------------------------------------------


fn test_identity_neg_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var neg_zero = -0.0
    _assert_eq_f64(
        w[].call_f64("identity_float64", args_f64(neg_zero)),
        neg_zero,
        "identity_float64(-0) === -0",
    )


fn test_neg_zero(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("neg_float64", args_f64(0.0)),
        -0.0,
        "neg_float64(0) === -0",
    )


fn test_neg_neg_zero(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var neg_zero = -0.0
    _assert_eq_f64(
        w[].call_f64("neg_float64", args_f64(neg_zero)),
        0.0,
        "neg_float64(-0) === 0",
    )


fn test_add_neg_zero_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var neg_zero = -0.0
    _assert_eq_f64(
        w[].call_f64("add_float64", args_f64_f64(neg_zero, 0.0)),
        0.0,
        "add_float64(-0, 0) === 0",
    )


fn test_mul_neg1_zero(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("mul_float64", args_f64_f64(-1.0, 0.0)),
        -0.0,
        "mul_float64(-1, 0) === -0",
    )


fn test_mul_neg_zero_neg_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var neg_zero = -0.0
    _assert_eq_f64(
        w[].call_f64("mul_float64", args_f64_f64(neg_zero, neg_zero)),
        0.0,
        "mul_float64(-0, -0) === 0",
    )


fn test_div_1_neg_inf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    _assert_eq_f64(
        w[].call_f64("div_float64", args_f64_f64(1.0, NegInf)),
        -0.0,
        "div_float64(1, -Inf) === -0",
    )


# ---------------------------------------------------------------------------
# Subnormal / denormalized numbers
# ---------------------------------------------------------------------------


fn test_identity_subnormal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var subnormal = 5e-324
    _assert_eq_f64(
        w[].call_f64("identity_float64", args_f64(subnormal)),
        subnormal,
        "identity_float64(5e-324) roundtrips",
    )


fn test_add_subnormal_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var subnormal = 5e-324
    _assert_eq_f64(
        w[].call_f64("add_float64", args_f64_f64(subnormal, 0.0)),
        subnormal,
        "add_float64(subnormal, 0) === subnormal",
    )


fn test_neg_subnormal(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var subnormal = 5e-324
    var neg_subnormal = -5e-324
    _assert_eq_f64(
        w[].call_f64("neg_float64", args_f64(subnormal)),
        neg_subnormal,
        "neg_float64(subnormal) === -subnormal",
    )


fn test_abs_neg_subnormal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var subnormal = 5e-324
    var neg_subnormal = -5e-324
    _assert_eq_f64(
        w[].call_f64("abs_float64", args_f64(neg_subnormal)),
        subnormal,
        "abs_float64(-subnormal) === subnormal",
    )


fn test_mul_subnormal_2(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var subnormal = 5e-324
    var expected = subnormal * 2.0
    _assert_eq_f64(
        w[].call_f64("mul_float64", args_f64_f64(subnormal, 2.0)),
        expected,
        "mul_float64(subnormal, 2) === subnormal * 2",
    )


# ---------------------------------------------------------------------------
# Float precision edge cases
# ---------------------------------------------------------------------------


fn test_0_1_plus_0_2(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    """Classic IEEE 754: 0.1 + 0.2 matches native Mojo result."""
    # Use separate variables to prevent the compiler from constant-folding
    # 0.1 + 0.2 to exactly 0.3 (which loses the IEEE 754 precision bit).
    var a: Float64 = 0.1
    var b: Float64 = 0.2
    var expected = a + b
    _assert_eq_f64(
        w[].call_f64("add_float64", args_f64_f64(0.1, 0.2)),
        expected,
        "add_float64(0.1, 0.2) matches Mojo 0.1+0.2",
    )


fn test_0_1_plus_0_2_not_0_3(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """The WASM result also differs from 0.3 (IEEE 754 precision)."""
    var result = w[].call_f64("add_float64", args_f64_f64(0.1, 0.2))
    assert_true(
        result != 0.3,
        "add_float64(0.1, 0.2) !== 0.3 (IEEE 754 precision)",
    )


fn test_large_plus_small(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var expected = 1e16 + 1.0
    _assert_eq_f64(
        w[].call_f64("add_float64", args_f64_f64(1e16, 1.0)),
        expected,
        "add_float64(1e16, 1) matches native precision",
    )


fn test_catastrophic_cancellation(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var a = 1e16 + 2.0
    var b = 1e16
    var expected = a - b
    _assert_eq_f64(
        w[].call_f64("sub_float64", args_f64_f64(a, b)),
        expected,
        "sub_float64(1e16+2, 1e16) matches native precision",
    )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_add_float64_nan_1(w)
    test_add_float64_1_nan(w)
    test_add_float64_nan_nan(w)
    test_sub_float64_nan_1(w)
    test_sub_float64_1_nan(w)
    test_mul_float64_nan_2(w)
    test_mul_float64_2_nan(w)
    test_div_float64_nan_2(w)
    test_div_float64_2_nan(w)
    test_add_float32_nan(w)
    test_sub_float32_nan(w)
    test_mul_float32_nan(w)
    test_div_float32_nan(w)
    test_add_inf_neg_inf(w)
    test_sub_inf_inf(w)
    test_mul_0_inf(w)
    test_div_0_0(w)
    test_neg_float64_nan(w)
    test_neg_float32_nan(w)
    test_abs_float64_nan(w)
    test_abs_float32_nan(w)
    test_identity_float64_nan(w)
    test_identity_float32_nan(w)
    test_min_nan_5(w)
    test_min_5_nan(w)
    test_max_nan_5(w)
    test_max_5_nan(w)
    test_pow_float64_nan(w)
    test_pow_float32_nan(w)
    test_add_inf_1(w)
    test_add_neg_inf_neg1(w)
    test_sub_inf_1(w)
    test_mul_inf_2(w)
    test_mul_inf_neg2(w)
    test_div_1_0(w)
    test_div_neg1_0(w)
    test_div_1_inf(w)
    test_add_float32_inf_1(w)
    test_div_float32_1_0(w)
    test_div_float32_neg1_0(w)
    test_neg_inf(w)
    test_neg_neg_inf(w)
    test_abs_neg_inf(w)
    test_abs_inf(w)
    test_identity_float64_inf(w)
    test_identity_float64_neg_inf(w)
    test_identity_float32_inf(w)
    test_min_neg_inf_inf(w)
    test_max_neg_inf_inf(w)
    test_min_42_neg_inf(w)
    test_max_42_inf(w)
    test_clamp_inf(w)
    test_clamp_neg_inf(w)
    test_identity_neg_zero(w)
    test_neg_zero(w)
    test_neg_neg_zero(w)
    test_add_neg_zero_zero(w)
    test_mul_neg1_zero(w)
    test_mul_neg_zero_neg_zero(w)
    test_div_1_neg_inf(w)
    test_identity_subnormal(w)
    test_add_subnormal_zero(w)
    test_neg_subnormal(w)
    test_abs_neg_subnormal(w)
    test_mul_subnormal_2(w)
    test_0_1_plus_0_2(w)
    test_0_1_plus_0_2_not_0_3(w)
    test_large_plus_small(w)
    test_catastrophic_cancellation(w)
    print("floats: 69/69 passed")
