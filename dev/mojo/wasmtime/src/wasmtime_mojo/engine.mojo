"""High-level wrapper for wasm_engine_t.

The Engine is the top-level compilation environment for WebAssembly modules.
It manages global compilation settings and caches.

Usage:
    var engine = Engine()
    # ... use engine to create stores, modules, linkers ...
    # engine is automatically cleaned up when it goes out of scope

    # With module caching enabled (persists compiled modules to disk):
    var cached_engine = Engine(cache=True)
"""

from memory import UnsafePointer

from ._types import EnginePtr
from ._lib import (
    wasm_engine_new,
    wasm_engine_delete,
    wasm_config_new,
    wasm_config_delete,
    wasm_engine_new_with_config,
    wasmtime_config_cache_config_load,
    ConfigPtr,
    error_message,
)


struct Engine:
    """RAII wrapper around wasm_engine_t.

    Owns the underlying engine pointer and deletes it on destruction.
    An Engine is required to create Stores, Modules, and Linkers.
    """

    var _ptr: EnginePtr

    fn __init__(out self) raises:
        """Create a new Wasmtime engine with default settings."""
        self._ptr = wasm_engine_new()

    fn __init__(out self, *, cache: Bool) raises:
        """Create a new Wasmtime engine with optional module caching.

        When *cache* is True, compiled WASM modules are persisted to disk
        (default location: ~/.cache/wasmtime) so subsequent loads of the
        same module skip the expensive compilation step — even across
        separate processes.

        Args:
            cache: If True, enable the default file-based module cache.
        """
        if not cache:
            self._ptr = wasm_engine_new()
            return

        var config = wasm_config_new()
        # Pass null path → use default cache directory
        var err = wasmtime_config_cache_config_load(
            config, UnsafePointer[UInt8, MutExternalOrigin]()
        )
        if err:
            var msg = error_message(err)
            # Config is NOT yet consumed if cache load fails; clean it up.
            wasm_config_delete(config)
            raise Error("Failed to enable wasmtime cache: " + msg)
        # wasm_engine_new_with_config takes ownership of config
        self._ptr = wasm_engine_new_with_config(config)

    fn __del__(deinit self):
        """Delete the engine, freeing all associated resources."""
        if self._ptr:
            try:
                wasm_engine_delete(self._ptr)
            except:
                pass

    fn __moveinit__(out self, deinit other: Self):
        """Move constructor — transfers ownership of the engine pointer."""
        self._ptr = other._ptr

    fn ptr(self) -> EnginePtr:
        """Return the raw engine pointer for FFI calls."""
        return self._ptr
