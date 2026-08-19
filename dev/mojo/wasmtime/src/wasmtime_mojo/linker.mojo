"""High-level wrapper for wasmtime_linker_t.

NOTE on FFI output parameters:
    Wasmtime C API functions write results through pointers (e.g. the
    instance handle from wasmtime_linker_instantiate).  For Mojo types
    that are TrivialRegisterPassable, the compiler may keep local
    variables in registers.  If we take the address of such a local with
    UnsafePointer(to=var) and pass it to FFI, the write goes to a stack
    spill slot — but the compiler may never reload the register from that
    slot, so the local still has its old (zero) value.

    The fix: heap-allocate output buffers with alloc(), let FFI write
    there, then read back with ptr[].  Heap pointers are opaque to the
    optimizer so reads always hit memory.

A Linker is used to define host-provided imports (functions, globals, memories,
tables) before instantiating a WebAssembly module.  It resolves import names
to concrete definitions so the module can be linked and run.

Usage:
    var engine = Engine()
    var store  = Store(engine.ptr())
    var linker = Linker(engine.ptr())

    # Define a host function import:
    linker.define_func(
        "env",
        "my_import",
        List[UInt8](WASM_I32, WASM_I32),  # param kinds
        List[UInt8](WASM_I64),             # result kinds
        my_callback,                       # WasmtimeCallback
        env_ptr,                           # user data
    )

    # Instantiate:
    var instance = linker.instantiate(store.context(), module.ptr())
"""

from std.memory import UnsafePointer, unsafe_memcpy, alloc

from ._types import (
    EnginePtr,
    ContextPtr,
    ModulePtr,
    LinkerPtr,
    ErrorPtr,
    TrapPtr,
    FuncTypePtr,
    WasmtimeInstance,
    WasmtimeCallback,
)
from ._lib import (
    _as_ext,
    wasmtime_linker_new,
    wasmtime_linker_delete,
    wasmtime_linker_define_func,
    wasmtime_linker_instantiate,
    make_functype,
    wasm_functype_delete,
    error_message,
    trap_message,
)


struct Linker(Movable):
    """RAII wrapper around wasmtime_linker_t.

    Owns the underlying linker pointer and deletes it on destruction.
    Used to define host imports and instantiate modules.
    """

    var _ptr: LinkerPtr

    def __init__(out self, engine_ptr: EnginePtr) raises:
        """Create a new Linker for the given engine.

        Args:
            engine_ptr: Raw pointer to the wasm_engine_t.  The engine must
                outlive this linker.
        """
        self._ptr = wasmtime_linker_new(engine_ptr)

    def __deinit__(deinit self):
        """Delete the linker, freeing all associated definitions."""
        try:
            wasmtime_linker_delete(self._ptr)
        except:
            pass

    def __init__(out self, *, deinit move: Self):
        """Move constructor — transfers ownership of the linker pointer."""
        self._ptr = move._ptr

    def ptr(self) -> LinkerPtr:
        """Return the raw linker pointer for FFI calls.

        The returned pointer carries MutUntrackedOrigin, which tells the
        compiler it does NOT borrow self. A Linker held in a local is therefore
        destroyed at its last use, so

            var e = Engine()
            var l = Linker(e.ptr())   # e dies here; wasmtime reads freed memory

        segfaults inside wasmtime. Keep the owner alive for at least as long as
        anything derived from it - a struct field, or a later use of the owner
        in the same scope.
        """
        return self._ptr

    # ------------------------------------------------------------------
    # Define a host function import
    # ------------------------------------------------------------------

    def define_func(
        self,
        module_name: String,
        func_name: String,
        param_kinds: List[UInt8],
        result_kinds: List[UInt8],
        callback: WasmtimeCallback,
        env: Optional[UnsafePointer[NoneType, MutUntrackedOrigin]] = None,
        finalizer: Optional[UnsafePointer[NoneType, MutUntrackedOrigin]] = None,
    ) raises:
        """Define a host function to satisfy a WASM import.

        The function type is built from the supplied parameter and result
        kind lists (use WASM_I32, WASM_I64, WASM_F32, WASM_F64 constants).

        Args:
            module_name: The import module name (e.g. ``"env"``).
            func_name: The import function name (e.g. ``"my_func"``).
            param_kinds: List of ``wasm_valkind_t`` values for parameters.
            result_kinds: List of ``wasm_valkind_t`` values for results.
            callback: The host callback matching ``WasmtimeCallback`` signature.
            env: Optional user-data pointer passed to the callback as its
                first argument.  Defaults to null.
            finalizer: Optional finalizer called with *env* when the linker
                definition is dropped.  Defaults to null (no finalizer).

        Raises:
            Error: If the linker rejects the definition (e.g. duplicate name).
        """
        var ft = make_functype(param_kinds, result_kinds)

        # Convert module_name to raw bytes
        var mod_bytes = module_name.as_bytes()
        var mod_ptr = _as_ext(mod_bytes.unsafe_ptr())
        var mod_len = module_name.byte_length()

        # Convert func_name to raw bytes
        var fn_bytes = func_name.as_bytes()
        var fn_ptr = _as_ext(fn_bytes.unsafe_ptr())
        var fn_len = func_name.byte_length()

        var err = wasmtime_linker_define_func(
            self._ptr,
            mod_ptr,
            mod_len,
            fn_ptr,
            fn_len,
            ft,
            callback,
            env,
            finalizer,
        )

        # wasmtime_linker_define_func takes ownership of the func type,
        # so we must NOT call wasm_functype_delete here.

        if err:
            var msg = error_message(err)
            raise Error(
                "Failed to define '"
                + module_name
                + "."
                + func_name
                + "': "
                + msg
            )

    # ------------------------------------------------------------------
    # Instantiate a module
    # ------------------------------------------------------------------

    def instantiate(
        self,
        context: ContextPtr,
        module_ptr: ModulePtr,
    ) raises -> WasmtimeInstance:
        """Instantiate a module, resolving all imports via this linker.

        Args:
            context: The store context to instantiate into.
            module_ptr: Raw pointer to the compiled wasmtime_module_t.

        Returns:
            A WasmtimeInstance value that can be used to access exports.

        Raises:
            Error: If instantiation fails (e.g. unresolved imports).
        """
        # Heap-allocate output buffers so FFI writes are visible
        # (see module docstring on register-passable aliasing).
        var instance_buf = alloc[WasmtimeInstance](1)
        instance_buf[] = WasmtimeInstance()
        var trap_buf = alloc[TrapPtr](1)
        trap_buf[] = TrapPtr()

        try:
            var err = wasmtime_linker_instantiate(
                self._ptr,
                context,
                module_ptr,
                _as_ext(instance_buf),
                _as_ext(trap_buf),
            )

            var trap = trap_buf[]

            if err:
                var msg = error_message(err)
                if trap:
                    # Also consume the trap if both are set
                    _ = trap_message(trap)
                raise Error("Instantiation failed: " + msg)

            if trap:
                raise Error("Instantiation trapped: " + trap_message(trap))

            return instance_buf[]
        finally:
            trap_buf.free()
            instance_buf.free()
