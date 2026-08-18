"""Wasmtime shared library loader and raw FFI function wrappers.

This module loads libwasmtime.so via OwnedDLHandle and provides thin typed
wrappers around the C API functions needed by mojo-wasmtime.

The OwnedDLHandle is loaded lazily on first use via `get_lib()`.
"""

from os import getenv
from std.ffi import OwnedDLHandle
from memory import UnsafePointer, alloc

from ._types import (
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
    WasmtimeVal,
    WasmtimeFunc,
    WasmtimeInstance,
    WasmtimeGlobal,
    WasmtimeMemory,
    WasmtimeExtern,
    WasmByteVec,
    WasmValtypeVec,
    WasmtimeCallback,
    FinalizerCallback,
)

# ---------------------------------------------------------------------------
# Origin cast helper
# ---------------------------------------------------------------------------


@always_inline
fn _as_ext[
    T: AnyType, origin: Origin
](ptr: UnsafePointer[T, origin]) -> UnsafePointer[T, MutExternalOrigin]:
    """Cast any UnsafePointer to MutExternalOrigin for FFI calls."""
    return UnsafePointer[T, MutExternalOrigin](unsafe_from_address=Int(ptr))


# ---------------------------------------------------------------------------
# Library loading
# ---------------------------------------------------------------------------


fn _find_lib_in_nix_ldflags() raises -> OwnedDLHandle:
    """Search NIX_LDFLAGS for a -L directory containing libwasmtime.so."""
    var flags = getenv("NIX_LDFLAGS", "")
    if not flags:
        raise Error("NIX_LDFLAGS not set")
    var parts = flags.split(" ")
    for i in range(len(parts)):
        var part = parts[i]
        if part.startswith("-L") and "wasmtime" in part:
            var dir_path = part.removeprefix("-L")
            var full = dir_path + "/libwasmtime.so"
            try:
                return OwnedDLHandle(full)
            except:
                pass
    raise Error("libwasmtime.so not found in NIX_LDFLAGS")


fn _open_lib() raises -> OwnedDLHandle:
    """Open libwasmtime.so, falling back to NIX_LDFLAGS."""
    try:
        return OwnedDLHandle("libwasmtime.so")
    except:
        return _find_lib_in_nix_ldflags()


fn _pin_lib() raises:
    """Pin the wasmtime library by leaking one OwnedDLHandle on the heap.

    This prevents dlclose from ever unloading the library, which would
    invalidate internal function pointers held by wasmtime objects
    (Engine, Store, Module, etc.) that outlive individual FFI calls.

    Safe to call repeatedly — dlopen is reference-counted so each leaked
    handle only costs ~16 bytes of heap, which is negligible.
    """
    var p = alloc[OwnedDLHandle](1)
    p.init_pointee_move(_open_lib())
    # Intentionally leak `p` — never free, never dlclose.


@always_inline
fn get_lib() raises -> OwnedDLHandle:
    """Return a handle to the wasmtime shared library.

    Pins the library in memory on each call so that it is never unloaded
    by dlclose.  This is necessary because wasmtime objects (engines,
    stores, modules) hold internal function pointers into the library
    that must remain valid for the lifetime of the process.

    Falls back to searching NIX_LDFLAGS (Nix environments) when the
    library is not on the default search path.
    """
    _pin_lib()
    return _open_lib()


# ═══════════════════════════════════════════════════════════════════════════
# Engine
# ═══════════════════════════════════════════════════════════════════════════


fn wasm_engine_new() raises -> EnginePtr:
    """Create a new wasm engine."""
    var lib = get_lib()
    var f = lib.get_function[fn() -> EnginePtr]("wasm_engine_new")
    return f()


fn wasm_engine_delete(engine: EnginePtr) raises:
    """Delete a wasm engine."""
    var lib = get_lib()
    var f = lib.get_function[fn(EnginePtr) -> None]("wasm_engine_delete")
    f(engine)


# ═══════════════════════════════════════════════════════════════════════════
# Config (for cache support)
# ═══════════════════════════════════════════════════════════════════════════

comptime ConfigPtr = UnsafePointer[NoneType, MutExternalOrigin]


fn wasm_config_new() raises -> ConfigPtr:
    """Create a new wasm config."""
    var lib = get_lib()
    var f = lib.get_function[fn() -> ConfigPtr]("wasm_config_new")
    return f()


fn wasm_config_delete(config: ConfigPtr) raises:
    """Delete a wasm config."""
    var lib = get_lib()
    var f = lib.get_function[fn(ConfigPtr) -> None]("wasm_config_delete")
    f(config)


fn wasmtime_config_cache_config_load(
    config: ConfigPtr,
    path: UnsafePointer[UInt8, MutExternalOrigin],
) raises -> ErrorPtr:
    """Load cache configuration.

    Pass a null pointer for *path* to use the default cache location
    (~/.cache/wasmtime or equivalent).
    """
    var lib = get_lib()
    var f = lib.get_function[
        fn(ConfigPtr, UnsafePointer[UInt8, MutExternalOrigin]) -> ErrorPtr
    ]("wasmtime_config_cache_config_load")
    return f(config, path)


fn wasm_engine_new_with_config(config: ConfigPtr) raises -> EnginePtr:
    """Create a new wasm engine with the given configuration.

    Note: this takes ownership of the config — do NOT delete it after
    calling this function.
    """
    var lib = get_lib()
    var f = lib.get_function[fn(ConfigPtr) -> EnginePtr](
        "wasm_engine_new_with_config"
    )
    return f(config)


# ═══════════════════════════════════════════════════════════════════════════
# Store / Context
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_store_new(
    engine: EnginePtr,
    data: UnsafePointer[NoneType, MutExternalOrigin],
    finalizer: UnsafePointer[NoneType, MutExternalOrigin],
) raises -> StorePtr:
    """Create a new wasmtime store."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            EnginePtr,
            UnsafePointer[NoneType, MutExternalOrigin],
            UnsafePointer[NoneType, MutExternalOrigin],
        ) -> StorePtr
    ]("wasmtime_store_new")
    return f(engine, data, finalizer)


fn wasmtime_store_delete(store: StorePtr) raises:
    """Delete a wasmtime store."""
    var lib = get_lib()
    var f = lib.get_function[fn(StorePtr) -> None]("wasmtime_store_delete")
    f(store)


fn wasmtime_store_context(store: StorePtr) raises -> ContextPtr:
    """Get the context from a wasmtime store."""
    var lib = get_lib()
    var f = lib.get_function[fn(StorePtr) -> ContextPtr](
        "wasmtime_store_context"
    )
    return f(store)


# ═══════════════════════════════════════════════════════════════════════════
# Module
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_module_new(
    engine: EnginePtr,
    wasm: UnsafePointer[UInt8, MutExternalOrigin],
    wasm_len: Int,
    ret: UnsafePointer[ModulePtr, MutExternalOrigin],
) raises -> ErrorPtr:
    """Compile a WASM binary into a module."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            EnginePtr,
            UnsafePointer[UInt8, MutExternalOrigin],
            Int,
            UnsafePointer[ModulePtr, MutExternalOrigin],
        ) -> ErrorPtr
    ]("wasmtime_module_new")
    return f(engine, wasm, wasm_len, ret)


fn wasmtime_module_delete(module: ModulePtr) raises:
    """Delete a compiled module."""
    var lib = get_lib()
    var f = lib.get_function[fn(ModulePtr) -> None]("wasmtime_module_delete")
    f(module)


fn wasmtime_module_serialize(
    module: ModulePtr,
    ret: UnsafePointer[WasmByteVec, MutExternalOrigin],
) raises -> ErrorPtr:
    """Serialize a compiled module to bytes.

    The caller must free *ret* with wasm_byte_vec_delete when done.
    """
    var lib = get_lib()
    var f = lib.get_function[
        fn(ModulePtr, UnsafePointer[WasmByteVec, MutExternalOrigin]) -> ErrorPtr
    ]("wasmtime_module_serialize")
    return f(module, ret)


fn wasmtime_module_deserialize_file(
    engine: EnginePtr,
    path: UnsafePointer[UInt8, MutExternalOrigin],
    ret: UnsafePointer[ModulePtr, MutExternalOrigin],
) raises -> ErrorPtr:
    """Deserialize a pre-compiled module directly from a file.

    This can mmap the file for very fast loading.  The file must have been
    produced by wasmtime_module_serialize with a compatible engine.
    """
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            EnginePtr,
            UnsafePointer[UInt8, MutExternalOrigin],
            UnsafePointer[ModulePtr, MutExternalOrigin],
        ) -> ErrorPtr
    ]("wasmtime_module_deserialize_file")
    return f(engine, path, ret)


# ═══════════════════════════════════════════════════════════════════════════
# Linker
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_linker_new(engine: EnginePtr) raises -> LinkerPtr:
    """Create a new linker for the given engine."""
    var lib = get_lib()
    var f = lib.get_function[fn(EnginePtr) -> LinkerPtr]("wasmtime_linker_new")
    return f(engine)


fn wasmtime_linker_delete(linker: LinkerPtr) raises:
    """Delete a linker."""
    var lib = get_lib()
    var f = lib.get_function[fn(LinkerPtr) -> None]("wasmtime_linker_delete")
    f(linker)


fn wasmtime_linker_define_func(
    linker: LinkerPtr,
    module_name: UnsafePointer[UInt8, MutExternalOrigin],
    module_name_len: Int,
    func_name: UnsafePointer[UInt8, MutExternalOrigin],
    func_name_len: Int,
    func_type: FuncTypePtr,
    callback: WasmtimeCallback,
    env: UnsafePointer[NoneType, MutExternalOrigin],
    finalizer: UnsafePointer[NoneType, MutExternalOrigin],
) raises -> ErrorPtr:
    """Define a host function in the linker."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            LinkerPtr,
            UnsafePointer[UInt8, MutExternalOrigin],
            Int,
            UnsafePointer[UInt8, MutExternalOrigin],
            Int,
            FuncTypePtr,
            WasmtimeCallback,
            UnsafePointer[NoneType, MutExternalOrigin],
            UnsafePointer[NoneType, MutExternalOrigin],
        ) -> ErrorPtr
    ]("wasmtime_linker_define_func")
    return f(
        linker,
        module_name,
        module_name_len,
        func_name,
        func_name_len,
        func_type,
        callback,
        env,
        finalizer,
    )


fn wasmtime_linker_instantiate(
    linker: LinkerPtr,
    context: ContextPtr,
    module: ModulePtr,
    instance: UnsafePointer[WasmtimeInstance, MutExternalOrigin],
    trap: UnsafePointer[TrapPtr, MutExternalOrigin],
) raises -> ErrorPtr:
    """Instantiate a module using the linker definitions."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            LinkerPtr,
            ContextPtr,
            ModulePtr,
            UnsafePointer[WasmtimeInstance, MutExternalOrigin],
            UnsafePointer[TrapPtr, MutExternalOrigin],
        ) -> ErrorPtr
    ]("wasmtime_linker_instantiate")
    return f(linker, context, module, instance, trap)


# ═══════════════════════════════════════════════════════════════════════════
# Instance exports
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_instance_export_get(
    context: ContextPtr,
    instance: UnsafePointer[WasmtimeInstance, MutExternalOrigin],
    name: UnsafePointer[UInt8, MutExternalOrigin],
    name_len: Int,
    item: UnsafePointer[WasmtimeExtern, MutExternalOrigin],
) raises -> Bool:
    """Look up an export from an instance by name."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            ContextPtr,
            UnsafePointer[WasmtimeInstance, MutExternalOrigin],
            UnsafePointer[UInt8, MutExternalOrigin],
            Int,
            UnsafePointer[WasmtimeExtern, MutExternalOrigin],
        ) -> Bool
    ]("wasmtime_instance_export_get")
    return f(context, instance, name, name_len, item)


# ═══════════════════════════════════════════════════════════════════════════
# Function calls
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_func_call(
    context: ContextPtr,
    func: UnsafePointer[WasmtimeFunc, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
    trap: UnsafePointer[TrapPtr, MutExternalOrigin],
) raises -> ErrorPtr:
    """Call a WASM function."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            ContextPtr,
            UnsafePointer[WasmtimeFunc, MutExternalOrigin],
            UnsafePointer[WasmtimeVal, MutExternalOrigin],
            Int,
            UnsafePointer[WasmtimeVal, MutExternalOrigin],
            Int,
            UnsafePointer[TrapPtr, MutExternalOrigin],
        ) -> ErrorPtr
    ]("wasmtime_func_call")
    return f(context, func, args, nargs, results, nresults, trap)


# ═══════════════════════════════════════════════════════════════════════════
# Global access
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_global_get(
    context: ContextPtr,
    global_: UnsafePointer[WasmtimeGlobal, MutExternalOrigin],
    result: UnsafePointer[WasmtimeVal, MutExternalOrigin],
) raises:
    """Read the value of a WASM global."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            ContextPtr,
            UnsafePointer[WasmtimeGlobal, MutExternalOrigin],
            UnsafePointer[WasmtimeVal, MutExternalOrigin],
        ) -> None
    ]("wasmtime_global_get")
    f(context, global_, result)


# ═══════════════════════════════════════════════════════════════════════════
# Memory access
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_memory_data(
    context: ContextPtr,
    memory: UnsafePointer[WasmtimeMemory, MutExternalOrigin],
) raises -> UnsafePointer[UInt8, MutExternalOrigin]:
    """Get a pointer to WASM linear memory data."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            ContextPtr, UnsafePointer[WasmtimeMemory, MutExternalOrigin]
        ) -> UnsafePointer[UInt8, MutExternalOrigin]
    ]("wasmtime_memory_data")
    return f(context, memory)


fn wasmtime_memory_data_size(
    context: ContextPtr,
    memory: UnsafePointer[WasmtimeMemory, MutExternalOrigin],
) raises -> Int:
    """Get the current size of WASM linear memory in bytes."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(ContextPtr, UnsafePointer[WasmtimeMemory, MutExternalOrigin]) -> Int
    ]("wasmtime_memory_data_size")
    return f(context, memory)


# ═══════════════════════════════════════════════════════════════════════════
# Val types and func types — needed for defining import signatures
# ═══════════════════════════════════════════════════════════════════════════


fn wasm_valtype_new(kind: UInt8) raises -> ValTypePtr:
    """Create a new WASM value type from a kind constant."""
    var lib = get_lib()
    var f = lib.get_function[fn(UInt8) -> ValTypePtr]("wasm_valtype_new")
    return f(kind)


fn wasm_valtype_delete(vt: ValTypePtr) raises:
    """Delete a WASM value type."""
    var lib = get_lib()
    var f = lib.get_function[fn(ValTypePtr) -> None]("wasm_valtype_delete")
    f(vt)


fn wasm_functype_new(
    params: UnsafePointer[WasmValtypeVec, MutExternalOrigin],
    results: UnsafePointer[WasmValtypeVec, MutExternalOrigin],
) raises -> FuncTypePtr:
    """Create a function type from param and result type vecs.

    Note: takes ownership of both vecs and the valtypes within them.
    Do NOT delete them after calling this.
    """
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            UnsafePointer[WasmValtypeVec, MutExternalOrigin],
            UnsafePointer[WasmValtypeVec, MutExternalOrigin],
        ) -> FuncTypePtr
    ]("wasm_functype_new")
    return f(params, results)


fn wasm_functype_delete(ft: FuncTypePtr) raises:
    """Delete a function type."""
    var lib = get_lib()
    var f = lib.get_function[fn(FuncTypePtr) -> None]("wasm_functype_delete")
    f(ft)


fn wasm_valtype_vec_new(
    result: UnsafePointer[WasmValtypeVec, MutExternalOrigin],
    size: Int,
    data: UnsafePointer[ValTypePtr, MutExternalOrigin],
) raises:
    """Create a new valtype vec from an array of valtype pointers."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(
            UnsafePointer[WasmValtypeVec, MutExternalOrigin],
            Int,
            UnsafePointer[ValTypePtr, MutExternalOrigin],
        ) -> None
    ]("wasm_valtype_vec_new")
    f(result, size, data)


fn wasm_valtype_vec_new_empty(
    result: UnsafePointer[WasmValtypeVec, MutExternalOrigin]
) raises:
    """Create a new empty valtype vec."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(UnsafePointer[WasmValtypeVec, MutExternalOrigin]) -> None
    ]("wasm_valtype_vec_new_empty")
    f(result)


fn wasm_valtype_vec_delete(
    vec: UnsafePointer[WasmValtypeVec, MutExternalOrigin]
) raises:
    """Delete a valtype vec."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(UnsafePointer[WasmValtypeVec, MutExternalOrigin]) -> None
    ]("wasm_valtype_vec_delete")
    f(vec)


# ═══════════════════════════════════════════════════════════════════════════
# Error handling
# ═══════════════════════════════════════════════════════════════════════════


fn wasmtime_error_message(
    error: ErrorPtr,
    message: UnsafePointer[WasmByteVec, MutExternalOrigin],
) raises:
    """Extract the error message from a wasmtime error."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(ErrorPtr, UnsafePointer[WasmByteVec, MutExternalOrigin]) -> None
    ]("wasmtime_error_message")
    f(error, message)


fn wasmtime_error_delete(error: ErrorPtr) raises:
    """Delete a wasmtime error."""
    var lib = get_lib()
    var f = lib.get_function[fn(ErrorPtr) -> None]("wasmtime_error_delete")
    f(error)


fn wasm_byte_vec_delete(
    vec: UnsafePointer[WasmByteVec, MutExternalOrigin]
) raises:
    """Delete a byte vec."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(UnsafePointer[WasmByteVec, MutExternalOrigin]) -> None
    ]("wasm_byte_vec_delete")
    f(vec)


# ═══════════════════════════════════════════════════════════════════════════
# Trap handling
# ═══════════════════════════════════════════════════════════════════════════


fn wasm_trap_message(
    trap: TrapPtr,
    message: UnsafePointer[WasmByteVec, MutExternalOrigin],
) raises:
    """Extract the message from a trap."""
    var lib = get_lib()
    var f = lib.get_function[
        fn(TrapPtr, UnsafePointer[WasmByteVec, MutExternalOrigin]) -> None
    ]("wasm_trap_message")
    f(trap, message)


fn wasm_trap_delete(trap: TrapPtr) raises:
    """Delete a trap."""
    var lib = get_lib()
    var f = lib.get_function[fn(TrapPtr) -> None]("wasm_trap_delete")
    f(trap)


# ═══════════════════════════════════════════════════════════════════════════
# Helper: build a wasm_functype_t from lists of WASM val kinds
# ═══════════════════════════════════════════════════════════════════════════


fn make_functype(
    param_kinds: List[UInt8], result_kinds: List[UInt8]
) raises -> FuncTypePtr:
    """Create a wasm_functype_t from parameter and result kind lists.

    Each element should be one of WASM_I32, WASM_I64, WASM_F32, WASM_F64.
    The returned FuncTypePtr must be freed with wasm_functype_delete
    (though wasmtime_linker_define_func takes ownership).
    """
    # Build params vec
    var params = WasmValtypeVec()
    var params_ptr = _as_ext(UnsafePointer(to=params))
    if len(param_kinds) == 0:
        wasm_valtype_vec_new_empty(params_ptr)
    else:
        var ptypes = alloc[ValTypePtr](len(param_kinds))
        try:
            for i in range(len(param_kinds)):
                ptypes[i] = wasm_valtype_new(param_kinds[i])
            wasm_valtype_vec_new(params_ptr, len(param_kinds), ptypes)
        finally:
            ptypes.free()

    # Build results vec
    var results = WasmValtypeVec()
    var results_ptr = _as_ext(UnsafePointer(to=results))
    if len(result_kinds) == 0:
        wasm_valtype_vec_new_empty(results_ptr)
    else:
        var rtypes = alloc[ValTypePtr](len(result_kinds))
        try:
            for i in range(len(result_kinds)):
                rtypes[i] = wasm_valtype_new(result_kinds[i])
            wasm_valtype_vec_new(results_ptr, len(result_kinds), rtypes)
        finally:
            rtypes.free()

    # wasm_functype_new takes ownership of both vecs
    return wasm_functype_new(params_ptr, results_ptr)


# ═══════════════════════════════════════════════════════════════════════════
# Helper: extract error/trap message as String
# ═══════════════════════════════════════════════════════════════════════════


fn error_message(error: ErrorPtr) raises -> String:
    """Extract the message from a wasmtime_error_t, delete it, and return
    the message as a Mojo String."""
    # Heap-allocate the output buffer so the FFI write is visible.
    # (WasmByteVec is TrivialRegisterPassable; a stack local may stay in
    # registers after a write through a cast pointer.)
    var msg_buf = alloc[WasmByteVec](1)
    msg_buf[] = WasmByteVec()
    # _as_ext is an address cast, so msg_ext aliases msg_buf: the vec has to be
    # deleted through it while the buffer is still live, and the buffer freed
    # only on the way out.
    var msg_ext = _as_ext(msg_buf)
    try:
        wasmtime_error_message(error, msg_ext)
        var msg = msg_buf[]
        var result = String("")
        if msg.size > 0 and msg.data:
            var buf = List[UInt8](capacity=msg.size + 1)
            for i in range(msg.size):
                buf.append(msg.data[i])
            buf.append(0)  # null-terminate
            result = String(unsafe_from_utf8=buf)
        wasm_byte_vec_delete(msg_ext)
        wasmtime_error_delete(error)
        return result
    finally:
        msg_buf.free()


fn trap_message(trap: TrapPtr) raises -> String:
    """Extract the message from a wasm_trap_t, delete it, and return
    the message as a Mojo String."""
    # Heap-allocate the output buffer so the FFI write is visible. As in
    # error_message, msg_ext aliases msg_buf, so the vec is deleted through it
    # before the buffer goes.
    var msg_buf = alloc[WasmByteVec](1)
    msg_buf[] = WasmByteVec()
    var msg_ext = _as_ext(msg_buf)
    try:
        wasm_trap_message(trap, msg_ext)
        var msg = msg_buf[]
        var result = String("")
        if msg.size > 0 and msg.data:
            var buf = List[UInt8](capacity=msg.size + 1)
            for i in range(msg.size):
                buf.append(msg.data[i])
            buf.append(0)  # null-terminate
            result = String(unsafe_from_utf8=buf)
        wasm_byte_vec_delete(msg_ext)
        wasm_trap_delete(trap)
        return result
    finally:
        msg_buf.free()


# ═══════════════════════════════════════════════════════════════════════════
# Helper: check error/trap and raise if present
# ═══════════════════════════════════════════════════════════════════════════


fn check_error(
    error: ErrorPtr, trap: UnsafePointer[TrapPtr, MutExternalOrigin]
) raises:
    """Check the error and trap pointers returned from a wasmtime API call.
    If either is non-null, raises with the appropriate message."""
    if error:
        raise Error("wasmtime error: " + error_message(error))
    if trap[]:
        raise Error("wasmtime trap: " + trap_message(trap[]))


fn check_error_only(error: ErrorPtr) raises:
    """Check the error pointer returned from a wasmtime API call.
    If non-null, raises with the message."""
    if error:
        raise Error("wasmtime error: " + error_message(error))
