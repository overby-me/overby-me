# Arithmetic operations exercised through the real WASM binary via
# mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that add, sub, mul, div, mod, and pow operations work
# correctly when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_arithmetic.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
    args_i64,
    args_i64_i64,
    args_f32,
    args_f32_f32,
    args_f64,
    args_f64_f64,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Add ──────────────────────────────────────────────────────────────────────


fn test_add_int32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(2, 3))),
        5,
        "add_int32(2, 3) === 5",
    )


fn test_add_int64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("add_int64", args_i64_i64(2, 3))),
        5,
        "add_int64(2, 3) === 5",
    )


fn test_add_float32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Float64(w[].call_f32("add_float32", args_f32_f32(2.2, 3.3)))
    # Float32 precision: compute expected in f32
    var expected = Float64(Float32(2.2) + Float32(3.3))
    assert_equal(result, expected, "add_float32(2.2, 3.3)")


fn test_add_float64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        w[].call_f64("add_float64", args_f64_f64(2.2, 3.3)),
        2.2 + 3.3,
        "add_float64(2.2, 3.3)",
    )


fn test_add_int32_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(0, 0))),
        0,
        "add_int32(0, 0) === 0",
    )
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(-5, 5))),
        0,
        "add_int32(-5, 5) === 0",
    )
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(-3, -7))),
        -10,
        "add_int32(-3, -7) === -10",
    )
    assert_equal(
        Int(w[].call_i32("add_int32", args_i32_i32(1, 0))),
        1,
        "add_int32(1, 0) === 1 (identity)",
    )


fn test_add_int64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("add_int64", args_i64_i64(0, 0))),
        0,
        "add_int64(0, 0) === 0",
    )
    assert_equal(
        Int(w[].call_i64("add_int64", args_i64_i64(-100, 100))),
        0,
        "add_int64(-100, 100) === 0",
    )
    assert_equal(
        Int(w[].call_i64("add_int64", args_i64_i64(1, 0))),
        1,
        "add_int64(1, 0) === 1 (identity)",
    )


fn test_add_float64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("add_float64", args_f64_f64(0.0, 0.0)),
        0.0,
        "add_float64(0, 0) === 0",
    )
    assert_equal(
        w[].call_f64("add_float64", args_f64_f64(-1.5, 1.5)),
        0.0,
        "add_float64(-1.5, 1.5) === 0",
    )


# ── Subtract ─────────────────────────────────────────────────────────────────


fn test_sub_int32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(10, 3))),
        7,
        "sub_int32(10, 3) === 7",
    )


fn test_sub_int64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("sub_int64", args_i64_i64(10, 3))),
        7,
        "sub_int64(10, 3) === 7",
    )


fn test_sub_float32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Float64(w[].call_f32("sub_float32", args_f32_f32(5.5, 2.2)))
    var expected = Float64(Float32(5.5) - Float32(2.2))
    assert_equal(result, expected, "sub_float32(5.5, 2.2)")


fn test_sub_float64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        w[].call_f64("sub_float64", args_f64_f64(5.5, 2.2)),
        5.5 - 2.2,
        "sub_float64(5.5, 2.2)",
    )


fn test_sub_int32_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(0, 0))),
        0,
        "sub_int32(0, 0) === 0",
    )
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(5, 5))),
        0,
        "sub_int32(5, 5) === 0",
    )
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(3, 7))),
        -4,
        "sub_int32(3, 7) === -4",
    )
    assert_equal(
        Int(w[].call_i32("sub_int32", args_i32_i32(-3, -7))),
        4,
        "sub_int32(-3, -7) === 4",
    )


fn test_sub_int64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("sub_int64", args_i64_i64(0, 0))),
        0,
        "sub_int64(0, 0) === 0",
    )
    assert_equal(
        Int(w[].call_i64("sub_int64", args_i64_i64(-50, -50))),
        0,
        "sub_int64(-50, -50) === 0",
    )


# ── Multiply ─────────────────────────────────────────────────────────────────


fn test_mul_int32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(4, 5))),
        20,
        "mul_int32(4, 5) === 20",
    )


fn test_mul_int64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("mul_int64", args_i64_i64(4, 5))),
        20,
        "mul_int64(4, 5) === 20",
    )


fn test_mul_float32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Float64(w[].call_f32("mul_float32", args_f32_f32(2.0, 3.0)))
    var expected = Float64(Float32(2.0) * Float32(3.0))
    assert_equal(result, expected, "mul_float32(2.0, 3.0)")


fn test_mul_float64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        w[].call_f64("mul_float64", args_f64_f64(2.5, 4.0)),
        10.0,
        "mul_float64(2.5, 4.0) === 10.0",
    )


fn test_mul_int32_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(0, 100))),
        0,
        "mul_int32(0, 100) === 0",
    )
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(1, 42))),
        42,
        "mul_int32(1, 42) === 42 (identity)",
    )
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(-1, 42))),
        -42,
        "mul_int32(-1, 42) === -42",
    )
    assert_equal(
        Int(w[].call_i32("mul_int32", args_i32_i32(-3, -4))),
        12,
        "mul_int32(-3, -4) === 12",
    )


fn test_mul_int64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("mul_int64", args_i64_i64(0, 999))),
        0,
        "mul_int64(0, 999) === 0",
    )
    assert_equal(
        Int(w[].call_i64("mul_int64", args_i64_i64(1, 999))),
        999,
        "mul_int64(1, 999) === 999 (identity)",
    )


fn test_mul_float64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("mul_float64", args_f64_f64(0.0, 123.456)),
        0.0,
        "mul_float64(0, 123.456) === 0",
    )


# ── Division ─────────────────────────────────────────────────────────────────


fn test_div_int32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("div_int32", args_i32_i32(20, 4))),
        5,
        "div_int32(20, 4) === 5",
    )


fn test_div_int64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("div_int64", args_i64_i64(20, 4))),
        5,
        "div_int64(20, 4) === 5",
    )


fn test_div_float32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Float64(w[].call_f32("div_float32", args_f32_f32(10.0, 4.0)))
    var expected = Float64(Float32(10.0) / Float32(4.0))
    assert_equal(result, expected, "div_float32(10.0, 4.0) === 2.5")


fn test_div_float64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        w[].call_f64("div_float64", args_f64_f64(10.0, 4.0)),
        2.5,
        "div_float64(10.0, 4.0) === 2.5",
    )


fn test_div_int32_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("div_int32", args_i32_i32(7, 2))),
        3,
        "div_int32(7, 2) === 3 (floor division)",
    )
    assert_equal(
        Int(w[].call_i32("div_int32", args_i32_i32(0, 5))),
        0,
        "div_int32(0, 5) === 0",
    )
    assert_equal(
        Int(w[].call_i32("div_int32", args_i32_i32(1, 1))),
        1,
        "div_int32(1, 1) === 1",
    )
    # Mojo floor division for negative numbers:
    # -7 // 2 = -4 in Python/Mojo (floor toward -inf)
    assert_equal(
        Int(w[].call_i32("div_int32", args_i32_i32(-7, 2))),
        -4,
        "div_int32(-7, 2) === -4 (floor division)",
    )


fn test_div_int64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        Int(w[].call_i64("div_int64", args_i64_i64(7, 2))),
        3,
        "div_int64(7, 2) === 3 (floor division)",
    )


fn test_div_float64_edge_cases(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    assert_equal(
        w[].call_f64("div_float64", args_f64_f64(1.0, 3.0)),
        1.0 / 3.0,
        "div_float64(1, 3)",
    )
    assert_equal(
        w[].call_f64("div_float64", args_f64_f64(0.0, 1.0)),
        0.0,
        "div_float64(0, 1) === 0",
    )


# ── Modulo ───────────────────────────────────────────────────────────────────


fn test_mod_int32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("mod_int32", args_i32_i32(10, 3))),
        1,
        "mod_int32(10, 3) === 1",
    )
    assert_equal(
        Int(w[].call_i32("mod_int32", args_i32_i32(15, 5))),
        0,
        "mod_int32(15, 5) === 0",
    )
    assert_equal(
        Int(w[].call_i32("mod_int32", args_i32_i32(0, 7))),
        0,
        "mod_int32(0, 7) === 0",
    )
    assert_equal(
        Int(w[].call_i32("mod_int32", args_i32_i32(7, 1))),
        0,
        "mod_int32(7, 1) === 0",
    )


fn test_mod_int64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("mod_int64", args_i64_i64(10, 3))),
        1,
        "mod_int64(10, 3) === 1",
    )
    assert_equal(
        Int(w[].call_i64("mod_int64", args_i64_i64(100, 7))),
        2,
        "mod_int64(100, 7) === 2",
    )


# ── Power ────────────────────────────────────────────────────────────────────


fn test_pow_int32(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("pow_int32", args_i32(3))),
        27,
        "pow_int32(3) === 27 (3^3)",
    )
    assert_equal(
        Int(w[].call_i32("pow_int32", args_i32(1))),
        1,
        "pow_int32(1) === 1 (1^1)",
    )
    assert_equal(
        Int(w[].call_i32("pow_int32", args_i32(2))),
        4,
        "pow_int32(2) === 4 (2^2)",
    )


fn test_pow_int64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    assert_equal(
        Int(w[].call_i64("pow_int64", args_i64(3))),
        27,
        "pow_int64(3) === 27 (3^3)",
    )
    assert_equal(
        Int(w[].call_i64("pow_int64", args_i64(1))),
        1,
        "pow_int64(1) === 1 (1^1)",
    )
    assert_equal(
        Int(w[].call_i64("pow_int64", args_i64(2))),
        4,
        "pow_int64(2) === 4 (2^2)",
    )


fn test_pow_float64(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    # pow_float64(3.3) = 3.3^3.3 ≈ 51.41572944937184
    var result = w[].call_f64("pow_float64", args_f64(3.3))
    assert_true(
        result > 51.415 and result < 51.416,
        "pow_float64(3.3) ≈ 51.4157",
    )
    # pow_float64(1.0) = 1.0^1.0 = 1.0
    var r1 = w[].call_f64("pow_float64", args_f64(1.0))
    assert_true(
        r1 > 0.999999 and r1 < 1.000001,
        "pow_float64(1.0) ≈ 1.0",
    )
    # pow_float64(2.0) = 2.0^2.0 = 4.0
    var r2 = w[].call_f64("pow_float64", args_f64(2.0))
    assert_true(
        r2 > 3.999999 and r2 < 4.000001,
        "pow_float64(2.0) ≈ 4.0",
    )


fn test_pow_float32_stable(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    # Verify pow_float32 is at least stable (same input → same output)
    var a = Float64(w[].call_f32("pow_float32", args_f32(3.3)))
    var b = Float64(w[].call_f32("pow_float32", args_f32(3.3)))
    assert_equal(a, b, "pow_float32(3.3) is stable")


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_add_int32(w)
    test_add_int64(w)
    test_add_float32(w)
    test_add_float64(w)
    test_add_int32_edge_cases(w)
    test_add_int64_edge_cases(w)
    test_add_float64_edge_cases(w)
    test_sub_int32(w)
    test_sub_int64(w)
    test_sub_float32(w)
    test_sub_float64(w)
    test_sub_int32_edge_cases(w)
    test_sub_int64_edge_cases(w)
    test_mul_int32(w)
    test_mul_int64(w)
    test_mul_float32(w)
    test_mul_float64(w)
    test_mul_int32_edge_cases(w)
    test_mul_int64_edge_cases(w)
    test_mul_float64_edge_cases(w)
    test_div_int32(w)
    test_div_int64(w)
    test_div_float32(w)
    test_div_float64(w)
    test_div_int32_edge_cases(w)
    test_div_int64_edge_cases(w)
    test_div_float64_edge_cases(w)
    test_mod_int32(w)
    test_mod_int64(w)
    test_pow_int32(w)
    test_pow_int64(w)
    test_pow_float64(w)
    test_pow_float32_stable(w)
    print("arithmetic: 33/33 passed")
