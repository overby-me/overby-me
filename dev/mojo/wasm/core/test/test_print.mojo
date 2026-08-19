# Port of test/print.test.ts — verifies print functions execute without error
# through the real WASM binary via mojo-wasmtime (pure Mojo FFI bindings —
# no Python interop required).
#
# The TypeScript version just calls each print function and checks that they
# don't throw.  We do the same here, plus verify that print_input_string
# handles a string struct correctly.
#
# Run with:
#   mojo test test/test_print.mojo

from std.memory import UnsafePointer
from std.testing import assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    no_args,
)


def _get_wasm() raises -> UnsafePointer[WasmInstance, MutUntrackedOrigin]:
    return get_instance()


# ---------------------------------------------------------------------------
# Print (static values) — just verify no crash
# ---------------------------------------------------------------------------


def test_print_static_string(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    w[].call_void("print_static_string", no_args())


def test_print_int32(w: UnsafePointer[WasmInstance, MutUntrackedOrigin]) raises:
    w[].call_void("print_int32", no_args())


def test_print_int64(w: UnsafePointer[WasmInstance, MutUntrackedOrigin]) raises:
    w[].call_void("print_int64", no_args())


def test_print_float32(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    w[].call_void("print_float32", no_args())


def test_print_float64(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    w[].call_void("print_float64", no_args())


# ---------------------------------------------------------------------------
# Print input string
# ---------------------------------------------------------------------------


def test_print_input_string(
    w: UnsafePointer[WasmInstance, MutUntrackedOrigin]
) raises:
    var struct_ptr = w[].write_string_struct("print-input-string")
    w[].call_void("print_input_string", args_ptr(struct_ptr))


def main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_print_static_string(w)
    test_print_int32(w)
    test_print_int64(w)
    test_print_float32(w)
    test_print_float64(w)
    test_print_input_string(w)
    print("print: 6/6 passed")
