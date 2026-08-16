# mojo-wasmtime

Mojo FFI bindings for the [Wasmtime](https://wasmtime.dev/) WebAssembly runtime.

Provides high-level RAII wrappers around the [Wasmtime C API](https://docs.wasmtime.dev/c-api/), enabling direct execution of WebAssembly modules from Mojo without requiring Python interop. The library loads `libwasmtime.so` dynamically at runtime via Mojo's `DLHandle`.

## How it works

The package mirrors Wasmtime's C struct layouts in Mojo using `UInt64` fields for correct 8-byte alignment (avoiding SIMD alignment issues), then exposes typed wrappers on top:

```txt
libwasmtime.so (C API)
  └─ _lib.mojo      — DLHandle loader + raw FFI function wrappers
  └─ _types.mojo    — C struct definitions (WasmtimeVal, WasmtimeExtern, …)
      └─ engine.mojo — RAII Engine  (compilation settings, module caching)
      └─ store.mojo  — RAII Store   (runtime state isolation) + Context
      └─ module.mojo — RAII Module  (compile, serialize, deserialize)
      └─ linker.mojo — RAII Linker  (host imports, instantiation)
      └─ instance.mojo — Free-standing helpers (export lookup, calls, memory, globals)
```

The shared library is discovered automatically from `NIX_LDFLAGS` (set by the Nix dev shell), making it work seamlessly in Nix-based environments.

## Project structure

```txt
mojo-wasmtime/
└── src/
    └── wasmtime_mojo/
        ├── __init__.mojo      # Package root — re-exports all public API
        ├── engine.mojo        # Engine: compilation settings, optional file-based module cache
        ├── store.mojo         # Store + Context: runtime state isolation
        ├── module.mojo        # Module: compile from WASM bytes, serialize/deserialize
        ├── linker.mojo        # Linker: define host function imports, instantiate modules
        ├── instance.mojo      # Export lookup, function calls, memory & global access helpers
        ├── _types.mojo        # C struct type definitions matching Wasmtime memory layout
        └── _lib.mojo          # DLHandle loader + thin typed wrappers over the C API
```

## API overview

### Core types

| Type | Description |
|---|---|
| `Engine` | Top-level compilation environment. Supports optional file-based module caching (`Engine(cache=True)`) for fast reloads across processes. |
| `Store` | Unit of isolation — holds all runtime state (memories, globals, tables). Provides a `context()` handle for runtime operations. |
| `Module` | A compiled WebAssembly module. Created from raw `.wasm` bytes. Supports `serialize()`/`deserialize_file()` for pre-compiled `.cwasm` files. |
| `Linker` | Defines host-provided imports and instantiates modules. Use `define_func()` to register host callbacks, then `instantiate()` to produce an instance. |

### Instance helpers

| Function | Description |
|---|---|
| `instance_get_func` | Look up an exported function by name |
| `instance_get_memory` | Look up an exported memory by name |
| `instance_get_global` | Look up an exported global by name |
| `func_call` | Call a WASM function with `List[WasmtimeVal]` args and results |
| `func_call_i32` / `func_call_i64` / `func_call_f32` / `func_call_f64` | Typed single-return-value call helpers |
| `func_call_0` | Call a WASM function that returns no values |
| `global_get_i32` / `global_get_i64` | Read typed values from WASM globals |
| `memory_read_bytes` / `memory_write_bytes` | Read/write byte slices in linear memory |
| `memory_read_i32_le` / `memory_write_i32_le` | Little-endian i32 memory access |
| `memory_read_i64_le` / `memory_write_i64_le` | Little-endian i64 memory access |

### Value constructors

`WasmtimeVal` provides static constructors for building typed WASM values:

- `WasmtimeVal.from_i32(value)` / `WasmtimeVal.from_i64(value)`
- `WasmtimeVal.from_f32(value)` / `WasmtimeVal.from_f64(value)`

## Usage

```mojo
from wasmtime_mojo import Engine, Store, Module, Linker
from wasmtime_mojo import instance_get_func, func_call_i32, WasmtimeVal

# Set up the runtime
var engine = Engine(cache=True)
var store = Store(engine.ptr())
var linker = Linker(engine.ptr())

# Compile a WASM module from bytes
var wasm_bytes = read_wasm_file("module.wasm")
var module = Module(engine.ptr(), wasm_bytes)

# Instantiate (linker resolves imports)
var instance = linker.instantiate(store.context(), module.ptr())

# Call an exported function
var add = instance_get_func(store.context(), instance, "add_int32")
var result = func_call_i32(
    store.context(),
    add,
    List[WasmtimeVal](WasmtimeVal.from_i32(40), WasmtimeVal.from_i32(2)),
)
print(result)  # 42
```

To define host function imports before instantiation:

```mojo
from wasmtime_mojo import WASM_I32, WASM_I64

fn my_host_func(
    env: UnsafePointer[NoneType],
    caller: UnsafePointer[NoneType],
    args: UnsafePointer[WasmtimeVal],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal],
    nresults: Int,
) -> UnsafePointer[NoneType]:
    results[] = WasmtimeVal.from_i64(args[].get_i32().cast[DType.int64]())
    return UnsafePointer[NoneType]()  # null = success (no trap)

linker.define_func(
    "env",
    "widen_i32",
    List[UInt8](WASM_I32),   # parameter types
    List[UInt8](WASM_I64),   # result types
    my_host_func,
)
```

## Prerequisites

Enter the dev shell (requires [Nix](https://nixos.org/)):

```sh
nix develop .#mojo-wasm
```

This provides `mojo` and `libwasmtime` with the shared library path exported in `NIX_LDFLAGS`.