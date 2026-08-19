# Bitwise operations exercised through the real WASM binary via
# mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that bitand, bitor, bitxor, bitnot, shl, and shr operations
# work correctly when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_bitwise.mojo

from std.memory import Pointer
from std.testing import assert_equal

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
)


def _get_wasm() raises -> Pointer[WasmInstance, MutUntrackedOrigin]:
    return get_instance()


# ── Bitwise AND ──────────────────────────────────────────────────────────────


def test_bitand_basic(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitand_int32", args_i32_i32(0b1100, 0b1010))),
        0b1000,
        "bitand_int32(0b1100, 0b1010) === 0b1000",
    )


def test_bitand_mask(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitand_int32", args_i32_i32(0xFF, 0x0F))),
        0x0F,
        "bitand_int32(0xFF, 0x0F) === 0x0F",
    )


def test_bitand_zero(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitand_int32", args_i32_i32(0, 0xFFFF))),
        0,
        "bitand_int32(0, 0xFFFF) === 0",
    )


# ── Bitwise OR ───────────────────────────────────────────────────────────────


def test_bitor_basic(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitor_int32", args_i32_i32(0b1100, 0b1010))),
        0b1110,
        "bitor_int32(0b1100, 0b1010) === 0b1110",
    )


def test_bitor_zero(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitor_int32", args_i32_i32(0, 0))),
        0,
        "bitor_int32(0, 0) === 0",
    )


# ── Bitwise XOR ──────────────────────────────────────────────────────────────


def test_bitxor_basic(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitxor_int32", args_i32_i32(0b1100, 0b1010))),
        0b0110,
        "bitxor_int32(0b1100, 0b1010) === 0b0110",
    )


def test_bitxor_self_is_zero(
    w: Pointer[WasmInstance, MutUntrackedOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitxor_int32", args_i32_i32(42, 42))),
        0,
        "bitxor_int32(42, 42) === 0",
    )


def test_bitxor_with_zero_is_identity(
    w: Pointer[WasmInstance, MutUntrackedOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("bitxor_int32", args_i32_i32(42, 0))),
        42,
        "bitxor_int32(42, 0) === 42",
    )


# ── Bitwise NOT ──────────────────────────────────────────────────────────────


def test_bitnot_zero(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitnot_int32", args_i32(0))),
        Int(~Int32(0)),
        "bitnot_int32(0) === ~0",
    )


def test_bitnot_one(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("bitnot_int32", args_i32(1))),
        Int(~Int32(1)),
        "bitnot_int32(1) === ~1",
    )


# ── Shifts ───────────────────────────────────────────────────────────────────


def test_shl_by_zero(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("shl_int32", args_i32_i32(1, 0))),
        1,
        "shl_int32(1, 0) === 1",
    )


def test_shl_by_one(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("shl_int32", args_i32_i32(1, 1))),
        2,
        "shl_int32(1, 1) === 2",
    )


def test_shl_by_four(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("shl_int32", args_i32_i32(1, 4))),
        16,
        "shl_int32(1, 4) === 16",
    )


def test_shl_three_by_three(
    w: Pointer[WasmInstance, MutUntrackedOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("shl_int32", args_i32_i32(3, 3))),
        24,
        "shl_int32(3, 3) === 24",
    )


def test_shr_sixteen_by_four(
    w: Pointer[WasmInstance, MutUntrackedOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("shr_int32", args_i32_i32(16, 4))),
        1,
        "shr_int32(16, 4) === 1",
    )


def test_shr_twentyfour_by_three(
    w: Pointer[WasmInstance, MutUntrackedOrigin]
) raises:
    assert_equal(
        Int(w[].call_i32("shr_int32", args_i32_i32(24, 3))),
        3,
        "shr_int32(24, 3) === 3",
    )


def test_shr_255_by_one(w: Pointer[WasmInstance, MutUntrackedOrigin]) raises:
    assert_equal(
        Int(w[].call_i32("shr_int32", args_i32_i32(255, 1))),
        127,
        "shr_int32(255, 1) === 127",
    )


def main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_bitand_basic(w)
    test_bitand_mask(w)
    test_bitand_zero(w)
    test_bitor_basic(w)
    test_bitor_zero(w)
    test_bitxor_basic(w)
    test_bitxor_self_is_zero(w)
    test_bitxor_with_zero_is_identity(w)
    test_bitnot_zero(w)
    test_bitnot_one(w)
    test_shl_by_zero(w)
    test_shl_by_one(w)
    test_shl_by_four(w)
    test_shl_three_by_three(w)
    test_shr_sixteen_by_four(w)
    test_shr_twentyfour_by_three(w)
    test_shr_255_by_one(w)
    print("bitwise: 17/17 passed")
