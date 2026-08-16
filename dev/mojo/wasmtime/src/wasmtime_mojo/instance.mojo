"""High-level wrapper for wasmtime_instance_t export access.

An Instance represents a live WebAssembly module that has been instantiated
inside a Store.  It provides typed helpers to look up exported functions,
memories, and globals by name.

The raw `WasmtimeInstance` is a 16-byte value type produced by the Linker
during instantiation.  This module provides free-standing helper functions
that operate on (context, instance) pairs, keeping the design lightweight
and compatible with Mojo's value semantics.

Usage:
    var inst = linker.instantiate(store.context(), module.ptr())
    var func = instance_get_func(store.context(), inst, "add_int32")
    var mem  = instance_get_memory(store.context(), inst, "memory")
    var glob = instance_get_global(store.context(), inst, "__heap_base")
"""

from memory import UnsafePointer, alloc, memcpy, free

from ._types import (
    ContextPtr,
    ErrorPtr,
    TrapPtr,
    WasmtimeInstance,
    WasmtimeExtern,
    WasmtimeFunc,
    WasmtimeGlobal,
    WasmtimeMemory,
    WasmtimeVal,
    WASMTIME_EXTERN_FUNC,
    WASMTIME_EXTERN_GLOBAL,
    WASMTIME_EXTERN_MEMORY,
    WASMTIME_I32,
    WASMTIME_I64,
    WASMTIME_F32,
    WASMTIME_F64,
)
from ._lib import (
    _as_ext,
    wasmtime_instance_export_get,
    wasmtime_func_call,
    wasmtime_global_get,
    wasmtime_memory_data,
    wasmtime_memory_data_size,
    error_message,
    trap_message,
)


# ═══════════════════════════════════════════════════════════════════════════
# Export lookup helpers
# ═══════════════════════════════════════════════════════════════════════════


fn instance_get_export(
    context: ContextPtr,
    instance: WasmtimeInstance,
    name: String,
) raises -> WasmtimeExtern:
    """Look up an export by name and return the raw WasmtimeExtern.

    Args:
        context: The store context the instance lives in.
        instance: The instantiated module.
        name: The export name to look up.

    Returns:
        The WasmtimeExtern tagged union describing the export.

    Raises:
        Error: If the export is not found.
    """
    # Heap-allocate both the input instance and output extern so
    # FFI reads/writes go through opaque heap pointers that the
    # optimizer cannot keep in registers.
    var inst_buf = alloc[WasmtimeInstance](1)
    inst_buf[] = instance
    var ext_buf = alloc[WasmtimeExtern](1)
    ext_buf[] = WasmtimeExtern()

    var name_bytes = name.as_bytes()
    var name_ptr = _as_ext(name_bytes.unsafe_ptr())
    var name_len = len(name)

    var found = wasmtime_instance_export_get(
        context, _as_ext(inst_buf), name_ptr, name_len, _as_ext(ext_buf)
    )

    inst_buf.free()
    if not found:
        ext_buf.free()
        raise Error("Export not found: '" + name + "'")

    var ext = ext_buf[]
    ext_buf.free()
    return ext


fn instance_get_func(
    context: ContextPtr,
    instance: WasmtimeInstance,
    name: String,
) raises -> WasmtimeFunc:
    """Look up an exported function by name.

    Args:
        context: The store context the instance lives in.
        instance: The instantiated module.
        name: The function export name.

    Returns:
        A WasmtimeFunc handle that can be used with call helpers.

    Raises:
        Error: If the export is not found or is not a function.
    """
    var ext = instance_get_export(context, instance, name)
    if ext.get_kind() != WASMTIME_EXTERN_FUNC:
        raise Error(
            "Export '"
            + name
            + "' is not a function (kind="
            + String(Int(ext.get_kind()))
            + ")"
        )
    return ext.get_func()


fn instance_get_memory(
    context: ContextPtr,
    instance: WasmtimeInstance,
    name: String,
) raises -> WasmtimeMemory:
    """Look up an exported memory by name.

    Args:
        context: The store context the instance lives in.
        instance: The instantiated module.
        name: The memory export name (typically ``"memory"``).

    Returns:
        A WasmtimeMemory handle for data access.

    Raises:
        Error: If the export is not found or is not a memory.
    """
    var ext = instance_get_export(context, instance, name)
    if ext.get_kind() != WASMTIME_EXTERN_MEMORY:
        raise Error(
            "Export '"
            + name
            + "' is not a memory (kind="
            + String(Int(ext.get_kind()))
            + ")"
        )
    return ext.get_memory()


fn instance_get_global(
    context: ContextPtr,
    instance: WasmtimeInstance,
    name: String,
) raises -> WasmtimeGlobal:
    """Look up an exported global by name.

    Args:
        context: The store context the instance lives in.
        instance: The instantiated module.
        name: The global export name (e.g. ``"__heap_base"``).

    Returns:
        A WasmtimeGlobal handle for value access.

    Raises:
        Error: If the export is not found or is not a global.
    """
    var ext = instance_get_export(context, instance, name)
    if ext.get_kind() != WASMTIME_EXTERN_GLOBAL:
        raise Error(
            "Export '"
            + name
            + "' is not a global (kind="
            + String(Int(ext.get_kind()))
            + ")"
        )
    return ext.get_global()


# ═══════════════════════════════════════════════════════════════════════════
# Global value access
# ═══════════════════════════════════════════════════════════════════════════


fn global_get_i32(
    context: ContextPtr, `global`: WasmtimeGlobal
) raises -> Int32:
    """Read an i32 value from a WASM global."""
    var g_buf = alloc[WasmtimeGlobal](1)
    g_buf[] = `global`
    var val_buf = alloc[WasmtimeVal](1)
    val_buf[] = WasmtimeVal()
    wasmtime_global_get(context, _as_ext(g_buf), _as_ext(val_buf))
    g_buf.free()
    var val = val_buf[]
    val_buf.free()
    return val.get_i32()


fn global_get_i64(
    context: ContextPtr, `global`: WasmtimeGlobal
) raises -> Int64:
    """Read an i64 value from a WASM global."""
    var g_buf = alloc[WasmtimeGlobal](1)
    g_buf[] = `global`
    var val_buf = alloc[WasmtimeVal](1)
    val_buf[] = WasmtimeVal()
    wasmtime_global_get(context, _as_ext(g_buf), _as_ext(val_buf))
    g_buf.free()
    var val = val_buf[]
    val_buf.free()
    return val.get_i64()


# ═══════════════════════════════════════════════════════════════════════════
# Function call helpers
# ═══════════════════════════════════════════════════════════════════════════


fn func_call(
    context: ContextPtr,
    func: WasmtimeFunc,
    args: List[WasmtimeVal],
    nresults: Int = 1,
) raises -> List[WasmtimeVal]:
    """Call a WASM function with the given arguments.

    Args:
        context: The store context.
        func: The function handle obtained from ``instance_get_func``.
        args: List of WasmtimeVal arguments matching the function signature.
        nresults: Expected number of return values (default 1).

    Returns:
        A List of WasmtimeVal results.

    Raises:
        Error: If the call fails or traps.
    """
    var f_buf = alloc[WasmtimeFunc](1)
    f_buf[] = func

    # Prepare args buffer
    var nargs = len(args)
    var args_buf = alloc[WasmtimeVal](max(nargs, 1))
    for i in range(nargs):
        args_buf[i] = args[i]

    # Prepare results buffer
    var results_buf = alloc[WasmtimeVal](max(nresults, 1))
    for i in range(nresults):
        results_buf[i] = WasmtimeVal()

    # Heap-allocate trap output so FFI write is visible.
    var trap_buf = alloc[UnsafePointer[NoneType, MutExternalOrigin]](1)
    trap_buf[] = UnsafePointer[NoneType, MutExternalOrigin]()

    var err = wasmtime_func_call(
        context,
        _as_ext(f_buf),
        args_buf,
        nargs,
        results_buf,
        nresults,
        _as_ext(trap_buf),
    )
    f_buf.free()

    var trap = trap_buf[]
    trap_buf.free()

    if err:
        var msg = error_message(err)
        if trap:
            _ = trap_message(trap)
        args_buf.free()
        results_buf.free()
        raise Error("Function call failed: " + msg)

    if trap:
        var msg = trap_message(trap)
        args_buf.free()
        results_buf.free()
        raise Error("Function call trapped: " + msg)

    var results = List[WasmtimeVal]()
    for i in range(nresults):
        results.append(results_buf[i])

    args_buf.free()
    results_buf.free()
    return results^


fn func_call_0(
    context: ContextPtr,
    func: WasmtimeFunc,
    args: List[WasmtimeVal],
) raises:
    """Call a WASM function that returns no values.

    Args:
        context: The store context.
        func: The function handle.
        args: List of WasmtimeVal arguments.

    Raises:
        Error: If the call fails or traps.
    """
    _ = func_call(context, func, args, nresults=0)


fn func_call_i32(
    context: ContextPtr,
    func: WasmtimeFunc,
    args: List[WasmtimeVal],
) raises -> Int32:
    """Call a WASM function and return a single i32 result.

    Args:
        context: The store context.
        func: The function handle.
        args: List of WasmtimeVal arguments.

    Returns:
        The i32 result value.

    Raises:
        Error: If the call fails or traps.
    """
    var results = func_call(context, func, args, nresults=1)
    return results[0].get_i32()


fn func_call_i64(
    context: ContextPtr,
    func: WasmtimeFunc,
    args: List[WasmtimeVal],
) raises -> Int64:
    """Call a WASM function and return a single i64 result.

    Args:
        context: The store context.
        func: The function handle.
        args: List of WasmtimeVal arguments.

    Returns:
        The i64 result value.

    Raises:
        Error: If the call fails or traps.
    """
    var results = func_call(context, func, args, nresults=1)
    return results[0].get_i64()


fn func_call_f32(
    context: ContextPtr,
    func: WasmtimeFunc,
    args: List[WasmtimeVal],
) raises -> Float32:
    """Call a WASM function and return a single f32 result.

    Args:
        context: The store context.
        func: The function handle.
        args: List of WasmtimeVal arguments.

    Returns:
        The f32 result value.

    Raises:
        Error: If the call fails or traps.
    """
    var results = func_call(context, func, args, nresults=1)
    return results[0].get_f32()


fn func_call_f64(
    context: ContextPtr,
    func: WasmtimeFunc,
    args: List[WasmtimeVal],
) raises -> Float64:
    """Call a WASM function and return a single f64 result.

    Args:
        context: The store context.
        func: The function handle.
        args: List of WasmtimeVal arguments.

    Returns:
        The f64 result value.

    Raises:
        Error: If the call fails or traps.
    """
    var results = func_call(context, func, args, nresults=1)
    return results[0].get_f64()


# ═══════════════════════════════════════════════════════════════════════════
# Memory access helpers
# ═══════════════════════════════════════════════════════════════════════════


fn memory_data_ptr(
    context: ContextPtr, memory: WasmtimeMemory
) raises -> UnsafePointer[UInt8, MutExternalOrigin]:
    """Return a raw pointer to the start of WASM linear memory.

    The pointer is only valid until the next memory-growing operation.

    Args:
        context: The store context.
        memory: The memory handle.

    Returns:
        Pointer to the first byte of linear memory.
    """
    var m_buf = alloc[WasmtimeMemory](1)
    m_buf[] = memory
    var result = wasmtime_memory_data(context, _as_ext(m_buf))
    m_buf.free()
    return result


fn memory_data_size(context: ContextPtr, memory: WasmtimeMemory) raises -> Int:
    """Return the current size of WASM linear memory in bytes.

    Args:
        context: The store context.
        memory: The memory handle.

    Returns:
        The size in bytes.
    """
    var m_buf = alloc[WasmtimeMemory](1)
    m_buf[] = memory
    var result = wasmtime_memory_data_size(context, _as_ext(m_buf))
    m_buf.free()
    return result


fn memory_read_bytes(
    context: ContextPtr,
    memory: WasmtimeMemory,
    offset: Int,
    length: Int,
) raises -> List[UInt8]:
    """Read a slice of bytes from WASM linear memory.

    Args:
        context: The store context.
        memory: The memory handle.
        offset: Byte offset into linear memory.
        length: Number of bytes to read.

    Returns:
        A List[UInt8] containing the bytes.
    """
    var base = memory_data_ptr(context, memory)
    var result = List[UInt8](capacity=length)
    for i in range(length):
        result.append((base + offset + i)[])
    return result^


fn memory_write_bytes(
    context: ContextPtr,
    memory: WasmtimeMemory,
    offset: Int,
    data: List[UInt8],
) raises:
    """Write bytes into WASM linear memory.

    Args:
        context: The store context.
        memory: The memory handle.
        offset: Byte offset into linear memory.
        data: The bytes to write.
    """
    var base = memory_data_ptr(context, memory)
    for i in range(len(data)):
        (base + offset + i)[] = data[i]


fn memory_read_i32_le(
    context: ContextPtr, memory: WasmtimeMemory, offset: Int
) raises -> Int32:
    """Read a little-endian i32 from WASM memory at the given byte offset."""
    var base = memory_data_ptr(context, memory)
    return (base + offset).bitcast[Int32]()[]


fn memory_write_i32_le(
    context: ContextPtr, memory: WasmtimeMemory, offset: Int, value: Int32
) raises:
    """Write a little-endian i32 into WASM memory at the given byte offset."""
    var base = memory_data_ptr(context, memory)
    (base + offset).bitcast[Int32]()[] = value


fn memory_read_i64_le(
    context: ContextPtr, memory: WasmtimeMemory, offset: Int
) raises -> Int64:
    """Read a little-endian i64 from WASM memory at the given byte offset."""
    var base = memory_data_ptr(context, memory)
    return (base + offset).bitcast[Int64]()[]


fn memory_write_i64_le(
    context: ContextPtr, memory: WasmtimeMemory, offset: Int, value: Int64
) raises:
    """Write a little-endian i64 into WASM memory at the given byte offset."""
    var base = memory_data_ptr(context, memory)
    (base + offset).bitcast[Int64]()[] = value


fn memory_read_u64_le(
    context: ContextPtr, memory: WasmtimeMemory, offset: Int
) raises -> UInt64:
    """Read a little-endian u64 from WASM memory at the given byte offset."""
    var base = memory_data_ptr(context, memory)
    return (base + offset).bitcast[UInt64]()[]
