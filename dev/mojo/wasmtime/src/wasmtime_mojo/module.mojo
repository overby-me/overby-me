"""High-level wrapper for wasmtime_module_t.

A Module represents a compiled WebAssembly module. It is created from
raw WASM bytes (binary format) using an Engine for compilation settings.

Modules are immutable once compiled and can be instantiated multiple times
with different Stores.

Usage:
    var engine = Engine()
    var wasm_bytes = read_wasm_file("module.wasm")
    var module = Module(engine.ptr(), wasm_bytes)
    # ... use module with a Linker to create instances ...

    # Serialize a compiled module to disk for fast reloading:
    module.serialize("module.cwasm")

    # Deserialize a pre-compiled module (very fast — can mmap the file):
    var fast_module = Module.deserialize_file(engine.ptr(), "module.cwasm")
"""

from memory import UnsafePointer, alloc, free
from pathlib import Path

from ._types import EnginePtr, ModulePtr, ErrorPtr, WasmByteVec
from ._lib import (
    _as_ext,
    wasmtime_module_new,
    wasmtime_module_delete,
    wasmtime_module_serialize,
    wasmtime_module_deserialize_file,
    wasm_byte_vec_delete,
    error_message,
)


struct Module:
    """RAII wrapper around wasmtime_module_t.

    Owns the underlying module pointer and deletes it on destruction.
    A Module is created by compiling WASM bytes with an Engine.
    """

    var _ptr: ModulePtr

    fn __init__(
        out self, engine_ptr: EnginePtr, wasm_bytes: List[UInt8]
    ) raises:
        """Compile a WASM binary into a Module.

        Args:
            engine_ptr: Raw pointer to the wasm_engine_t used for compilation.
                The engine must outlive this module.
            wasm_bytes: The raw WASM binary bytes (.wasm format).

        Raises:
            Error: If compilation fails (e.g. invalid WASM binary).
        """
        var module_out = alloc[ModulePtr](1)
        module_out[] = ModulePtr()

        var data_ptr = _as_ext(wasm_bytes.unsafe_ptr())
        var err = wasmtime_module_new(
            engine_ptr,
            data_ptr,
            len(wasm_bytes),
            module_out,
        )

        if err:
            var msg = error_message(err)
            module_out.free()
            raise Error("Failed to compile WASM module: " + msg)

        self._ptr = module_out[]
        module_out.free()

    fn __init__(out self, *, var _ptr: ModulePtr):
        """Create a Module from a raw pointer (takes ownership).

        Args:
            _ptr: Raw pointer to an already-compiled wasmtime_module_t.
                This Module takes ownership and will delete it on destruction.
        """
        self._ptr = _ptr

    fn __del__(deinit self):
        """Delete the module, freeing compilation artifacts."""
        if self._ptr:
            try:
                wasmtime_module_delete(self._ptr)
            except:
                pass

    fn __moveinit__(out self, deinit other: Self):
        """Move constructor — transfers ownership of the module pointer."""
        self._ptr = other._ptr

    fn ptr(self) -> ModulePtr:
        """Return the raw module pointer for FFI calls."""
        return self._ptr

    # ------------------------------------------------------------------
    # Serialization
    # ------------------------------------------------------------------

    fn serialize(self, path: String) raises:
        """Serialize the compiled module to a file.

        The resulting file can be loaded later with `Module.deserialize_file`
        for very fast instantiation (skips compilation entirely).

        The file is only valid for the same version of wasmtime and the
        same engine configuration that produced it.

        Args:
            path: Filesystem path to write the serialized module to.

        Raises:
            Error: If serialization fails.
        """
        # Heap-allocate the output buffer so the FFI write is visible.
        # (WasmByteVec is @register_passable("trivial"); a stack local
        # may stay in registers after a write through a cast pointer.)
        var bv_buf = alloc[WasmByteVec](1)
        bv_buf[] = WasmByteVec()
        var bv_ext = _as_ext(bv_buf)
        var err = wasmtime_module_serialize(self._ptr, bv_ext)
        if err:
            bv_buf.free()
            var msg = error_message(err)
            raise Error("Failed to serialize module: " + msg)

        # Read back the filled-in byte vec from the heap buffer
        var byte_vec = bv_buf[]
        bv_buf.free()

        # Write the serialized bytes to a file
        var data = List[UInt8](capacity=byte_vec.size)
        for i in range(byte_vec.size):
            data.append(byte_vec.data[i])
        wasm_byte_vec_delete(bv_ext)

        Path(path).write_bytes(data)

    @staticmethod
    fn deserialize_file(engine_ptr: EnginePtr, path: String) raises -> Module:
        """Deserialize a pre-compiled module from a file.

        This is much faster than compiling from WASM bytes because the
        runtime can mmap the file directly.  The file must have been
        produced by `Module.serialize` with a compatible engine.

        Args:
            engine_ptr: Raw pointer to the wasm_engine_t. Must use the
                same configuration as the engine that serialized the module.
            path: Filesystem path to the serialized module (.cwasm).

        Returns:
            A Module wrapping the deserialized compiled code.

        Raises:
            Error: If the file cannot be read or deserialization fails
                (e.g. engine version / config mismatch).
        """
        # Build a null-terminated C string for the path
        var path_bytes = path.as_bytes()
        var c_path = alloc[UInt8](len(path_bytes) + 1)
        for i in range(len(path_bytes)):
            c_path[i] = path_bytes[i]
        c_path[len(path_bytes)] = 0  # null terminator

        var module_out = alloc[ModulePtr](1)
        module_out[] = ModulePtr()

        var err = wasmtime_module_deserialize_file(
            engine_ptr, c_path, module_out
        )
        c_path.free()

        if err:
            var msg = error_message(err)
            module_out.free()
            raise Error(
                "Failed to deserialize module from '" + path + "': " + msg
            )

        var ptr = module_out[]
        module_out.free()
        return Module(_ptr=ptr)
