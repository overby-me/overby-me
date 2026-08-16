# Unary operations (neg, abs) exercised through the real WASM binary via
# mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that negation and absolute value operations work correctly
# when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_unary.mojo

from math import copysign
from memory import UnsafePointer
from testing import assert_equal, assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i64,
    args_f32,
    args_f64,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Negate — int32 ───────────────────────────────────────────────────────────


fn test_neg_int32_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("neg_int32", args_i32(5))),
        -5,
        "neg_int32(5) === -5",
    )


fn test_neg_int32_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("neg_int32", args_i32(-5))),
        5,
        "neg_int32(-5) === 5",
    )


fn test_neg_int32_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("neg_int32", args_i32(0))),
        0,
        "neg_int32(0) === 0",
    )


# ── Negate — int64 ───────────────────────────────────────────────────────────


fn test_neg_int64_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("neg_int64", args_i64(42))),
        -42,
        "neg_int64(42) === -42",
    )


fn test_neg_int64_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("neg_int64", args_i64(-42))),
        42,
        "neg_int64(-42) === 42",
    )


fn test_neg_int64_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("neg_int64", args_i64(0))),
        0,
        "neg_int64(0) === 0",
    )


# ── Negate — float32 ────────────────────────────────────────────────────────


fn test_neg_float32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Float64(w[].call_f32("neg_float32", args_f32(3.14)))
    var expected = Float64(-Float32(3.14))
    assert_equal(result, expected, "neg_float32(3.14)")


# ── Negate — float64 ────────────────────────────────────────────────────────


fn test_neg_float64_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("neg_float64", args_f64(3.14)),
        -3.14,
        "neg_float64(3.14) === -3.14",
    )


fn test_neg_float64_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("neg_float64", args_f64(-3.14)),
        3.14,
        "neg_float64(-3.14) === 3.14",
    )


fn test_neg_float64_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # neg_float64(0.0) produces -0.0
    var result = w[].call_f64("neg_float64", args_f64(0.0))
    # copysign(1.0, -0.0) == -1.0, so if sign is negative we got -0.0
    assert_true(
        copysign(Float64(1.0), result) < 0,
        "neg_float64(0) === -0 (negative zero)",
    )


# ── Absolute value — int32 ──────────────────────────────────────────────────


fn test_abs_int32_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("abs_int32", args_i32(5))),
        5,
        "abs_int32(5) === 5",
    )


fn test_abs_int32_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("abs_int32", args_i32(-5))),
        5,
        "abs_int32(-5) === 5",
    )


fn test_abs_int32_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("abs_int32", args_i32(0))),
        0,
        "abs_int32(0) === 0",
    )


# ── Absolute value — int64 ──────────────────────────────────────────────────


fn test_abs_int64_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("abs_int64", args_i64(99))),
        99,
        "abs_int64(99) === 99",
    )


fn test_abs_int64_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("abs_int64", args_i64(-99))),
        99,
        "abs_int64(-99) === 99",
    )


fn test_abs_int64_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("abs_int64", args_i64(0))),
        0,
        "abs_int64(0) === 0",
    )


# ── Absolute value — float32 ────────────────────────────────────────────────


fn test_abs_float32_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Float64(w[].call_f32("abs_float32", args_f32(2.5)))
    var expected = Float64(Float32(2.5))
    assert_equal(result, expected, "abs_float32(2.5)")


fn test_abs_float32_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Float64(w[].call_f32("abs_float32", args_f32(-2.5)))
    var expected = Float64(Float32(2.5))
    assert_equal(result, expected, "abs_float32(-2.5)")


# ── Absolute value — float64 ────────────────────────────────────────────────


fn test_abs_float64_positive(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("abs_float64", args_f64(3.14)),
        3.14,
        "abs_float64(3.14) === 3.14",
    )


fn test_abs_float64_negative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("abs_float64", args_f64(-3.14)),
        3.14,
        "abs_float64(-3.14) === 3.14",
    )


fn test_abs_float64_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("abs_float64", args_f64(0.0)),
        0.0,
        "abs_float64(0) === 0",
    )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_neg_int32_positive(w)
    test_neg_int32_negative(w)
    test_neg_int32_zero(w)
    test_neg_int64_positive(w)
    test_neg_int64_negative(w)
    test_neg_int64_zero(w)
    test_neg_float32(w)
    test_neg_float64_positive(w)
    test_neg_float64_negative(w)
    test_neg_float64_zero(w)
    test_abs_int32_positive(w)
    test_abs_int32_negative(w)
    test_abs_int32_zero(w)
    test_abs_int64_positive(w)
    test_abs_int64_negative(w)
    test_abs_int64_zero(w)
    test_abs_float32_positive(w)
    test_abs_float32_negative(w)
    test_abs_float64_positive(w)
    test_abs_float64_negative(w)
    test_abs_float64_zero(w)
    print("unary: 21/21 passed")
