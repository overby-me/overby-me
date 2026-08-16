"""mojo-wasmtime: Mojo FFI bindings for the Wasmtime WebAssembly runtime.

This package provides high-level Mojo wrappers around the Wasmtime C API,
enabling direct execution of WebAssembly modules from Mojo without requiring
Python interop.

Quick start:

    from wasmtime_mojo import Engine, Store, Module, Linker
    from wasmtime_mojo.instance import (
        instance_get_func,
        instance_get_memory,
        instance_get_global,
        func_call,
        func_call_i32,
        func_call_i64,
        func_call_f32,
        func_call_f64,
        memory_read_bytes,
        memory_write_bytes,
        memory_read_i64_le,
        memory_write_i64_le,
        global_get_i32,
        global_get_i64,
    )

    var engine = Engine()
    var store  = Store(engine.ptr())
    var linker = Linker(engine.ptr())
    # ... define imports, load module, instantiate, call exports ...
"""

# ── Core resource types ──────────────────────────────────────────────────
from .engine import Engine
from .store import Store
from .module import Module
from .linker import Linker

# ── Low-level C struct types ─────────────────────────────────────────────
from ._types import (
    # Opaque pointer aliases
    EnginePtr,
    StorePtr,
    ContextPtr,
    ModulePtr,
    LinkerPtr,
    ErrorPtr,
    TrapPtr,
    FuncTypePtr,
    ValTypePtr,
    CallerPtr,
    # Config pointer (re-exported from _lib)
    # Value / extern structs
    WasmtimeVal,
    WasmtimeFunc,
    WasmtimeInstance,
    WasmtimeGlobal,
    WasmtimeMemory,
    WasmtimeExtern,
    WasmByteVec,
    WasmValtypeVec,
    # Callback types
    WasmtimeCallback,
    FinalizerCallback,
    # Val kind constants
    WASMTIME_I32,
    WASMTIME_I64,
    WASMTIME_F32,
    WASMTIME_F64,
    WASMTIME_V128,
    WASMTIME_FUNCREF,
    WASMTIME_EXTERNREF,
    # WASM val kind constants (for wasm_valtype_new)
    WASM_I32,
    WASM_I64,
    WASM_F32,
    WASM_F64,
    WASM_EXTERNREF,
    WASM_FUNCREF,
    # Extern kind constants
    WASMTIME_EXTERN_FUNC,
    WASMTIME_EXTERN_GLOBAL,
    WASMTIME_EXTERN_TABLE,
    WASMTIME_EXTERN_MEMORY,
    WASMTIME_EXTERN_SHAREDMEMORY,
)

# ── Instance / export / call / memory helpers ────────────────────────────
from .instance import (
    instance_get_export,
    instance_get_func,
    instance_get_memory,
    instance_get_global,
    global_get_i32,
    global_get_i64,
    func_call,
    func_call_0,
    func_call_i32,
    func_call_i64,
    func_call_f32,
    func_call_f64,
    memory_data_ptr,
    memory_data_size,
    memory_read_bytes,
    memory_write_bytes,
    memory_read_i32_le,
    memory_write_i32_le,
    memory_read_i64_le,
    memory_write_i64_le,
    memory_read_u64_le,
)

# ── Raw FFI helpers (advanced use) ───────────────────────────────────────
from ._lib import (
    get_lib,
    make_functype,
    error_message,
    trap_message,
    ConfigPtr,
    wasm_config_new,
    wasm_config_delete,
    wasm_engine_new_with_config,
    wasmtime_config_cache_config_load,
    wasmtime_module_serialize,
    wasmtime_module_deserialize_file,
)
