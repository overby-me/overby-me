# Port of test/properties.test.ts — algebraic property tests (commutativity,
# associativity, distributivity, identity elements, annihilators, self-inverse,
# De Morgan's laws, comparison duality) exercised through the real WASM binary
# via mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# Run with:
#   mojo test test/test_properties.mojo

from memory import UnsafePointer
from testing import assert_true, assert_equal

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_i32_i32,
    args_i64_i64,
    args_f64,
    args_f64_f64,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ---------------------------------------------------------------------------
# Commutativity — add
# ---------------------------------------------------------------------------


fn test_add_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0, 1, -7, 100, 2147483647, 12345]
    var bs: List[Int] = [0, 2, 13, -100, -2147483648, 67890]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("add_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("add_int32", args_i32_i32(b, a))),
            String("add_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_add_int64_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0, 1, -999, 9223372036854775807]
    var bs: List[Int] = [0, 2, 999, -1]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i64("add_int64", args_i64_i64(a, b))),
            Int(w[].call_i64("add_int64", args_i64_i64(b, a))),
            String("add_int64(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_add_float64_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Float64] = [0.0, 1.5, -3.14, 1e10]
    var bs: List[Float64] = [0.0, 2.5, 3.14, 1e-10]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_true(
            w[].call_f64("add_float64", args_f64_f64(a, b))
            == w[].call_f64("add_float64", args_f64_f64(b, a)),
            String("add_float64(")
            + String(a)
            + ", "
            + String(b)
            + ") commutes",
        )


# ---------------------------------------------------------------------------
# Commutativity — mul
# ---------------------------------------------------------------------------


fn test_mul_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0, 3, -5, -4, 2147483647, 1000]
    var bs: List[Int] = [1, 7, 11, -6, 2, 1000]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("mul_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("mul_int32", args_i32_i32(b, a))),
            String("mul_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_mul_int64_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0, 3, -100]
    var bs: List[Int] = [1, 7, 200]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i64("mul_int64", args_i64_i64(a, b))),
            Int(w[].call_i64("mul_int64", args_i64_i64(b, a))),
            String("mul_int64(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_mul_float64_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Float64] = [2.5, -1.5, 0.0]
    var bs: List[Float64] = [4.0, 3.0, 999.0]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_true(
            w[].call_f64("mul_float64", args_f64_f64(a, b))
            == w[].call_f64("mul_float64", args_f64_f64(b, a)),
            String("mul_float64(")
            + String(a)
            + ", "
            + String(b)
            + ") commutes",
        )


# ---------------------------------------------------------------------------
# Commutativity — min / max
# ---------------------------------------------------------------------------


fn test_min_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [3, -5, 0, 2147483647]
    var bs: List[Int] = [7, 5, 0, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("min_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("min_int32", args_i32_i32(b, a))),
            String("min_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_max_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [3, -5, 0, 2147483647]
    var bs: List[Int] = [7, 5, 0, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("max_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("max_int32", args_i32_i32(b, a))),
            String("max_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


# ---------------------------------------------------------------------------
# Commutativity — GCD
# ---------------------------------------------------------------------------


fn test_gcd_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [12, 7, 100, 0, 1071]
    var bs: List[Int] = [8, 13, 75, 5, 462]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("gcd_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("gcd_int32", args_i32_i32(b, a))),
            String("gcd_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


# ---------------------------------------------------------------------------
# Commutativity — bitwise and / or / xor
# ---------------------------------------------------------------------------


fn test_bitand_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0, 2147483647]
    var bs: List[Int] = [0b1010, 0x0F, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("bitand_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("bitand_int32", args_i32_i32(b, a))),
            String("bitand_int32(")
            + String(a)
            + ", "
            + String(b)
            + ") commutes",
        )


fn test_bitor_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0, 2147483647]
    var bs: List[Int] = [0b1010, 0x0F, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("bitor_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("bitor_int32", args_i32_i32(b, a))),
            String("bitor_int32(")
            + String(a)
            + ", "
            + String(b)
            + ") commutes",
        )


fn test_bitxor_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0, 2147483647]
    var bs: List[Int] = [0b1010, 0x0F, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("bitxor_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("bitxor_int32", args_i32_i32(b, a))),
            String("bitxor_int32(")
            + String(a)
            + ", "
            + String(b)
            + ") commutes",
        )


# ---------------------------------------------------------------------------
# Commutativity — boolean
# ---------------------------------------------------------------------------


fn test_bool_and_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    for a in range(2):
        for b in range(2):
            assert_equal(
                Int(w[].call_i32("bool_and", args_i32_i32(a, b))),
                Int(w[].call_i32("bool_and", args_i32_i32(b, a))),
                String("bool_and(")
                + String(a)
                + ", "
                + String(b)
                + ") commutes",
            )


fn test_bool_or_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    for a in range(2):
        for b in range(2):
            assert_equal(
                Int(w[].call_i32("bool_or", args_i32_i32(a, b))),
                Int(w[].call_i32("bool_or", args_i32_i32(b, a))),
                String("bool_or(")
                + String(a)
                + ", "
                + String(b)
                + ") commutes",
            )


# ---------------------------------------------------------------------------
# Commutativity — eq / ne
# ---------------------------------------------------------------------------


fn test_eq_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0, 5, -1, 2147483647]
    var bs: List[Int] = [0, 6, 1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("eq_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("eq_int32", args_i32_i32(b, a))),
            String("eq_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


fn test_ne_int32_commutes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0, 5, -1, 2147483647]
    var bs: List[Int] = [0, 6, 1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("ne_int32", args_i32_i32(a, b))),
            Int(w[].call_i32("ne_int32", args_i32_i32(b, a))),
            String("ne_int32(") + String(a) + ", " + String(b) + ") commutes",
        )


# ---------------------------------------------------------------------------
# Associativity — add
# ---------------------------------------------------------------------------


fn test_add_int32_associative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [1, -5, 100, 0, 2147483647]
    var bs: List[Int] = [2, 10, 200, 0, 1]
    var cs: List[Int] = [3, -3, 300, 0, -1]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "add_int32",
                args_i32_i32(
                    Int(w[].call_i32("add_int32", args_i32_i32(a, b))), c
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "add_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("add_int32", args_i32_i32(b, c)))
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("add_int32 associative: (")
            + String(a)
            + "+"
            + String(b)
            + ")+"
            + String(c),
        )


fn test_add_float64_associative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Float64] = [1.0, -1.0, 100.0]
    var bs: List[Float64] = [2.0, 1.0, 200.0]
    var cs: List[Float64] = [4.0, 0.0, 300.0]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        assert_true(
            w[].call_f64(
                "add_float64",
                args_f64_f64(
                    w[].call_f64("add_float64", args_f64_f64(a, b)), c
                ),
            )
            == w[].call_f64(
                "add_float64",
                args_f64_f64(
                    a, w[].call_f64("add_float64", args_f64_f64(b, c))
                ),
            ),
            String("add_float64 associative: (")
            + String(a)
            + "+"
            + String(b)
            + ")+"
            + String(c),
        )


# ---------------------------------------------------------------------------
# Associativity — mul
# ---------------------------------------------------------------------------


fn test_mul_int32_associative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [2, -1, 1, 0, 10]
    var bs: List[Int] = [3, 5, 1, 999, 10]
    var cs: List[Int] = [4, 7, 1, 123, 10]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "mul_int32",
                args_i32_i32(
                    Int(w[].call_i32("mul_int32", args_i32_i32(a, b))), c
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "mul_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("mul_int32", args_i32_i32(b, c)))
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("mul_int32 associative: (")
            + String(a)
            + "*"
            + String(b)
            + ")*"
            + String(c),
        )


# ---------------------------------------------------------------------------
# Associativity — bitwise and / or / xor
# ---------------------------------------------------------------------------


fn test_bitand_int32_associative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0]
    var bs: List[Int] = [0b1010, 0x0F, -1]
    var cs: List[Int] = [0b0110, 0xAA, 42]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "bitand_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitand_int32", args_i32_i32(a, b))), c
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitand_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("bitand_int32", args_i32_i32(b, c)))
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("bitand_int32 associative: (")
            + String(a)
            + "&"
            + String(b)
            + ")&"
            + String(c),
        )


fn test_bitor_int32_associative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0]
    var bs: List[Int] = [0b1010, 0x0F, -1]
    var cs: List[Int] = [0b0110, 0xAA, 42]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "bitor_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitor_int32", args_i32_i32(a, b))), c
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitor_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("bitor_int32", args_i32_i32(b, c)))
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("bitor_int32 associative: (")
            + String(a)
            + "|"
            + String(b)
            + ")|"
            + String(c),
        )


fn test_bitxor_int32_associative(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0]
    var bs: List[Int] = [0b1010, 0x0F, -1]
    var cs: List[Int] = [0b0110, 0xAA, 42]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "bitxor_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitxor_int32", args_i32_i32(a, b))), c
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitxor_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("bitxor_int32", args_i32_i32(b, c)))
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("bitxor_int32 associative: (")
            + String(a)
            + "^"
            + String(b)
            + ")^"
            + String(c),
        )


# ---------------------------------------------------------------------------
# Distributivity — mul over add
# ---------------------------------------------------------------------------


fn test_mul_distributes_over_add(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [2, -3, 0, 1, 10, 7]
    var bs: List[Int] = [3, 5, 100, -1, 10, 0]
    var cs: List[Int] = [4, 7, 200, 1, 10, 0]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "mul_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("add_int32", args_i32_i32(b, c)))
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "add_int32",
                args_i32_i32(
                    Int(w[].call_i32("mul_int32", args_i32_i32(a, b))),
                    Int(w[].call_i32("mul_int32", args_i32_i32(a, c))),
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("mul_int32 distributes: ")
            + String(a)
            + "*("
            + String(b)
            + "+"
            + String(c)
            + ")",
        )


# ---------------------------------------------------------------------------
# Distributivity — bitwise and over or
# ---------------------------------------------------------------------------


fn test_bitand_distributes_over_bitor(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, -1, 0]
    var bs: List[Int] = [0b1010, 0x0F, 42, 0xFFFF]
    var cs: List[Int] = [0b0110, 0xF0, 99, 0xFF00]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "bitand_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("bitor_int32", args_i32_i32(b, c)))
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitor_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitand_int32", args_i32_i32(a, b))),
                    Int(w[].call_i32("bitand_int32", args_i32_i32(a, c))),
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("bitand distributes over bitor: ")
            + String(a)
            + "&("
            + String(b)
            + "|"
            + String(c)
            + ")",
        )


# ---------------------------------------------------------------------------
# Distributivity — bitwise or over and
# ---------------------------------------------------------------------------


fn test_bitor_distributes_over_bitand(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [0b1100, 0xFF, 0]
    var bs: List[Int] = [0b1010, 0x0F, 42]
    var cs: List[Int] = [0b0110, 0xF0, 99]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var c = cs[i]
        var lhs = Int(
            w[].call_i32(
                "bitor_int32",
                args_i32_i32(
                    a, Int(w[].call_i32("bitand_int32", args_i32_i32(b, c)))
                ),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitand_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitor_int32", args_i32_i32(a, b))),
                    Int(w[].call_i32("bitor_int32", args_i32_i32(a, c))),
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("bitor distributes over bitand: ")
            + String(a)
            + "|("
            + String(b)
            + "&"
            + String(c)
            + ")",
        )


# ---------------------------------------------------------------------------
# Identity elements
# ---------------------------------------------------------------------------


fn test_add_identity(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("add_int32", args_i32_i32(x, 0))),
            x,
            String("add_int32(") + String(x) + ", 0) === " + String(x),
        )


fn test_mul_identity(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("mul_int32", args_i32_i32(x, 1))),
            x,
            String("mul_int32(") + String(x) + ", 1) === " + String(x),
        )


fn test_bitand_identity(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("bitand_int32", args_i32_i32(x, -1))),
            x,
            String("bitand_int32(") + String(x) + ", -1) === " + String(x),
        )


fn test_bitor_identity(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("bitor_int32", args_i32_i32(x, 0))),
            x,
            String("bitor_int32(") + String(x) + ", 0) === " + String(x),
        )


fn test_bitxor_identity(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("bitxor_int32", args_i32_i32(x, 0))),
            x,
            String("bitxor_int32(") + String(x) + ", 0) === " + String(x),
        )


# ---------------------------------------------------------------------------
# Annihilators / zero elements
# ---------------------------------------------------------------------------


fn test_mul_zero(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("mul_int32", args_i32_i32(x, 0))),
            0,
            String("mul_int32(") + String(x) + ", 0) === 0",
        )


fn test_bitand_zero(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("bitand_int32", args_i32_i32(x, 0))),
            0,
            String("bitand_int32(") + String(x) + ", 0) === 0",
        )


fn test_bitor_all_ones(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var xs: List[Int] = [-42, 0, 1, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(w[].call_i32("bitor_int32", args_i32_i32(x, -1))),
            -1,
            String("bitor_int32(") + String(x) + ", -1) === -1",
        )


# ---------------------------------------------------------------------------
# Self-inverse / involution
# ---------------------------------------------------------------------------


fn test_neg_neg(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var xs: List[Int] = [-42, 0, 1, 99, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(
                w[].call_i32(
                    "neg_int32",
                    args_i32(Int(w[].call_i32("neg_int32", args_i32(x)))),
                )
            ),
            x,
            String("neg_int32(neg_int32(") + String(x) + ")) === " + String(x),
        )


fn test_bitnot_bitnot(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var xs: List[Int] = [-42, 0, 1, 99, 2147483647, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        assert_equal(
            Int(
                w[].call_i32(
                    "bitnot_int32",
                    args_i32(Int(w[].call_i32("bitnot_int32", args_i32(x)))),
                )
            ),
            x,
            String("bitnot_int32(bitnot_int32(")
            + String(x)
            + ")) === "
            + String(x),
        )


fn test_bool_not_not(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    for x in range(2):
        assert_equal(
            Int(
                w[].call_i32(
                    "bool_not",
                    args_i32(Int(w[].call_i32("bool_not", args_i32(x)))),
                )
            ),
            x,
            String("bool_not(bool_not(") + String(x) + ")) === " + String(x),
        )


fn test_bitxor_self_inverse(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var xs: List[Int] = [42, 0, 2147483647]
    var ys: List[Int] = [99, -1, -2147483648]
    for i in range(len(xs)):
        var x = xs[i]
        var y = ys[i]
        assert_equal(
            Int(
                w[].call_i32(
                    "bitxor_int32",
                    args_i32_i32(
                        Int(w[].call_i32("bitxor_int32", args_i32_i32(x, y))),
                        y,
                    ),
                )
            ),
            x,
            String("bitxor(bitxor(")
            + String(x)
            + ", "
            + String(y)
            + "), "
            + String(y)
            + ") === "
            + String(x),
        )


# ---------------------------------------------------------------------------
# De Morgan's laws — boolean
# ---------------------------------------------------------------------------


fn test_de_morgan_not_and_eq_or_not(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """not(a and b) === not(a) or not(b)."""
    for a in range(2):
        for b in range(2):
            var lhs = Int(
                w[].call_i32(
                    "bool_not",
                    args_i32(Int(w[].call_i32("bool_and", args_i32_i32(a, b)))),
                )
            )
            var rhs = Int(
                w[].call_i32(
                    "bool_or",
                    args_i32_i32(
                        Int(w[].call_i32("bool_not", args_i32(a))),
                        Int(w[].call_i32("bool_not", args_i32(b))),
                    ),
                )
            )
            assert_equal(
                lhs,
                rhs,
                String("not(")
                + String(a)
                + " and "
                + String(b)
                + ") === not("
                + String(a)
                + ") or not("
                + String(b)
                + ")",
            )


fn test_de_morgan_not_or_eq_and_not(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """not(a or b) === not(a) and not(b)."""
    for a in range(2):
        for b in range(2):
            var lhs = Int(
                w[].call_i32(
                    "bool_not",
                    args_i32(Int(w[].call_i32("bool_or", args_i32_i32(a, b)))),
                )
            )
            var rhs = Int(
                w[].call_i32(
                    "bool_and",
                    args_i32_i32(
                        Int(w[].call_i32("bool_not", args_i32(a))),
                        Int(w[].call_i32("bool_not", args_i32(b))),
                    ),
                )
            )
            assert_equal(
                lhs,
                rhs,
                String("not(")
                + String(a)
                + " or "
                + String(b)
                + ") === not("
                + String(a)
                + ") and not("
                + String(b)
                + ")",
            )


# ---------------------------------------------------------------------------
# De Morgan's laws — bitwise
# ---------------------------------------------------------------------------


fn test_bitnot_and_eq_or_bitnot(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """~(a & b) === ~a | ~b."""
    var as_: List[Int] = [0b1100, 0xFF, 0, 2147483647]
    var bs: List[Int] = [0b1010, 0x0F, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var lhs = Int(
            w[].call_i32(
                "bitnot_int32",
                args_i32(Int(w[].call_i32("bitand_int32", args_i32_i32(a, b)))),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitor_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitnot_int32", args_i32(a))),
                    Int(w[].call_i32("bitnot_int32", args_i32(b))),
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("~(")
            + String(a)
            + " & "
            + String(b)
            + ") === ~"
            + String(a)
            + " | ~"
            + String(b),
        )


fn test_bitnot_or_eq_and_bitnot(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """~(a | b) === ~a & ~b."""
    var as_: List[Int] = [0b1100, 0xFF, 0, 2147483647]
    var bs: List[Int] = [0b1010, 0x0F, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        var lhs = Int(
            w[].call_i32(
                "bitnot_int32",
                args_i32(Int(w[].call_i32("bitor_int32", args_i32_i32(a, b)))),
            )
        )
        var rhs = Int(
            w[].call_i32(
                "bitand_int32",
                args_i32_i32(
                    Int(w[].call_i32("bitnot_int32", args_i32(a))),
                    Int(w[].call_i32("bitnot_int32", args_i32(b))),
                ),
            )
        )
        assert_equal(
            lhs,
            rhs,
            String("~(")
            + String(a)
            + " | "
            + String(b)
            + ") === ~"
            + String(a)
            + " & ~"
            + String(b),
        )


# ---------------------------------------------------------------------------
# Comparison duality — lt vs ge, le vs gt
# ---------------------------------------------------------------------------


fn test_lt_eq_not_ge(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var as_: List[Int] = [3, 5, 7, -1, 0, 2147483647]
    var bs: List[Int] = [5, 5, 5, 0, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("lt_int32", args_i32_i32(a, b))),
            Int(
                w[].call_i32(
                    "bool_not",
                    args_i32(Int(w[].call_i32("ge_int32", args_i32_i32(a, b)))),
                )
            ),
            String("lt(")
            + String(a)
            + ", "
            + String(b)
            + ") === not(ge("
            + String(a)
            + ", "
            + String(b)
            + "))",
        )


fn test_le_eq_not_gt(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var as_: List[Int] = [3, 5, 7, -1, 0, 2147483647]
    var bs: List[Int] = [5, 5, 5, 0, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("le_int32", args_i32_i32(a, b))),
            Int(
                w[].call_i32(
                    "bool_not",
                    args_i32(Int(w[].call_i32("gt_int32", args_i32_i32(a, b)))),
                )
            ),
            String("le(")
            + String(a)
            + ", "
            + String(b)
            + ") === not(gt("
            + String(a)
            + ", "
            + String(b)
            + "))",
        )


fn test_eq_iff_le_and_ge(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var as_: List[Int] = [3, 5, 7, -1, 0, 2147483647]
    var bs: List[Int] = [5, 5, 5, 0, -1, -2147483648]
    for i in range(len(as_)):
        var a = as_[i]
        var b = bs[i]
        assert_equal(
            Int(w[].call_i32("eq_int32", args_i32_i32(a, b))),
            Int(
                w[].call_i32(
                    "bool_and",
                    args_i32_i32(
                        Int(w[].call_i32("le_int32", args_i32_i32(a, b))),
                        Int(w[].call_i32("ge_int32", args_i32_i32(a, b))),
                    ),
                )
            ),
            String("eq(")
            + String(a)
            + ", "
            + String(b)
            + ") === le("
            + String(a)
            + ", "
            + String(b)
            + ") and ge("
            + String(a)
            + ", "
            + String(b)
            + ")",
        )


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_add_int32_commutes(w)
    test_add_int64_commutes(w)
    test_add_float64_commutes(w)
    test_mul_int32_commutes(w)
    test_mul_int64_commutes(w)
    test_mul_float64_commutes(w)
    test_min_int32_commutes(w)
    test_max_int32_commutes(w)
    test_gcd_int32_commutes(w)
    test_bitand_int32_commutes(w)
    test_bitor_int32_commutes(w)
    test_bitxor_int32_commutes(w)
    test_bool_and_commutes(w)
    test_bool_or_commutes(w)
    test_eq_int32_commutes(w)
    test_ne_int32_commutes(w)
    test_add_int32_associative(w)
    test_add_float64_associative(w)
    test_mul_int32_associative(w)
    test_bitand_int32_associative(w)
    test_bitor_int32_associative(w)
    test_bitxor_int32_associative(w)
    test_mul_distributes_over_add(w)
    test_bitand_distributes_over_bitor(w)
    test_bitor_distributes_over_bitand(w)
    test_add_identity(w)
    test_mul_identity(w)
    test_bitand_identity(w)
    test_bitor_identity(w)
    test_bitxor_identity(w)
    test_mul_zero(w)
    test_bitand_zero(w)
    test_bitor_all_ones(w)
    test_neg_neg(w)
    test_bitnot_bitnot(w)
    test_bool_not_not(w)
    test_bitxor_self_inverse(w)
    test_de_morgan_not_and_eq_or_not(w)
    test_de_morgan_not_or_eq_and_not(w)
    test_bitnot_and_eq_or_bitnot(w)
    test_bitnot_or_eq_and_bitnot(w)
    test_lt_eq_not_ge(w)
    test_le_eq_not_gt(w)
    test_eq_iff_le_and_ge(w)
    print("properties: 44/44 passed")
