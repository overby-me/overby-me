"""High-level wrapper for wasmtime_store_t and wasmtime_context_t.

A Store is the unit of isolation in Wasmtime — it holds all runtime state
for a set of instances (memories, globals, tables, etc.).  Every Store is
tied to an Engine that controls compilation settings.

The Context is the short-lived handle used for most store operations.
It is derived from the Store and remains valid as long as the Store lives.

Usage:
    var engine = Engine()
    var store = Store(engine)
    var ctx = store.context()
    # ... pass ctx to instantiation, function calls, memory access ...
"""

from memory import UnsafePointer

from ._types import EnginePtr, StorePtr, ContextPtr
from ._lib import (
    wasmtime_store_new,
    wasmtime_store_delete,
    wasmtime_store_context,
)


struct Store:
    """RAII wrapper around wasmtime_store_t.

    Owns the underlying store pointer and deletes it on destruction.
    Provides access to the wasmtime_context_t needed for most runtime
    operations.
    """

    var _ptr: StorePtr
    var _context: ContextPtr

    fn __init__(out self, engine_ptr: EnginePtr) raises:
        """Create a new Store backed by the given engine.

        Args:
            engine_ptr: Raw pointer to the wasm_engine_t. The engine must
                outlive this store.
        """
        self._ptr = wasmtime_store_new(
            engine_ptr,
            UnsafePointer[NoneType, MutExternalOrigin](),
            UnsafePointer[NoneType, MutExternalOrigin](),
        )
        self._context = wasmtime_store_context(self._ptr)

    fn __del__(deinit self):
        """Delete the store, freeing all runtime state it holds."""
        if self._ptr:
            try:
                wasmtime_store_delete(self._ptr)
            except:
                pass

    fn __moveinit__(out self, deinit other: Self):
        """Move constructor — transfers ownership of the store pointer."""
        self._ptr = other._ptr
        self._context = other._context

    fn context(self) -> ContextPtr:
        """Return the wasmtime_context_t for this store.

        The context is valid as long as the store is alive and is used
        for instantiation, function calls, memory access, and global reads.
        """
        return self._context

    fn ptr(self) -> StorePtr:
        """Return the raw store pointer for FFI calls."""
        return self._ptr
