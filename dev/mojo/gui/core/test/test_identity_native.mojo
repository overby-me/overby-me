# Identity/passthrough operations exercised through the real WASM binary via
# mojo-wasmtime (pure Mojo FFI bindings).
#
# These tests verify that identity functions correctly pass through values
# when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_identity_native.mojo

from std.math import copysign
from std.memory import UnsafePointer
from std.testing import assert_equal, assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i64,
    args_f32,
    args_f64,
    no_args,
)


def _get_wasm() raises -> UnsafePointer[WasmInstance, MutUntrackedOrigin]:
    return get_instance()


# ── Identity — int32 ─────────────────────────────────────────────────────────


def test_identity_int32_zero(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = Int(w[].call_i32("identity_int32", args_i32(0)))
    assert_equal(result, 0, "identity_int32(0) === 0")


def test_identity_int32_positive(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = Int(w[].call_i32("identity_int32", args_i32(42)))
    assert_equal(result, 42, "identity_int32(42) === 42")


def test_identity_int32_negative(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = Int(w[].call_i32("identity_int32", args_i32(-42)))
    assert_equal(result, -42, "identity_int32(-42) === -42")


# ── Identity — int64 ─────────────────────────────────────────────────────────


def test_identity_int64_zero(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = Int(w[].call_i64("identity_int64", args_i64(0)))
    assert_equal(result, 0, "identity_int64(0) === 0")


def test_identity_int64_positive(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = Int(w[].call_i64("identity_int64", args_i64(999)))
    assert_equal(result, 999, "identity_int64(999) === 999")


def test_identity_int64_negative(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = Int(w[].call_i64("identity_int64", args_i64(-999)))
    assert_equal(result, -999, "identity_int64(-999) === -999")


# ── Identity — float32 ───────────────────────────────────────────────────────


def test_identity_float32_pi(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var input = Float32(3.14)
    var result = w[].call_f32("identity_float32", args_f32(input))
    assert_equal(Float64(result), Float64(input), "identity_float32(3.14)")


def test_identity_float32_zero(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = w[].call_f32("identity_float32", args_f32(0.0))
    assert_equal(Float64(result), 0.0, "identity_float32(0) === 0")


# ── Identity — float64 ───────────────────────────────────────────────────────


def test_identity_float64_pi(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var pi = 3.141592653589793
    var result = w[].call_f64("identity_float64", args_f64(pi))
    assert_equal(result, pi, "identity_float64(pi)")


def test_identity_float64_zero(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var result = w[].call_f64("identity_float64", args_f64(0.0))
    assert_equal(result, 0.0, "identity_float64(0) === 0")


def test_identity_float64_negative_zero(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    # -0.0 should roundtrip through identity. IEEE 754 says -0.0 == 0.0,
    # but we can verify the sign bit is preserved via copysign.
    var result = w[].call_f64("identity_float64", args_f64(-0.0))
    # copysign(1.0, -0.0) == -1.0, so if sign is preserved the copysign is negative
    assert_true(
        copysign(Float64(1.0), result) < 0,
        "identity_float64(-0) === -0 (sign bit preserved)",
    )


def main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_identity_int32_zero(w)
    test_identity_int32_positive(w)
    test_identity_int32_negative(w)
    test_identity_int64_zero(w)
    test_identity_int64_positive(w)
    test_identity_int64_negative(w)
    test_identity_float32_pi(w)
    test_identity_float32_zero(w)
    test_identity_float64_pi(w)
    test_identity_float64_zero(w)
    test_identity_float64_negative_zero(w)
    print("identity_native: 11/11 passed")
