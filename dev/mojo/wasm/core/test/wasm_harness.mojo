"""WASM test harness using mojo-wasmtime (pure Mojo FFI bindings).

Provides a WasmInstance for loading the Mojo WASM binary and interacting with
its exported functions, including string struct read/write operations
that mirror the TypeScript runtime/strings.ts and runtime/memory.ts.

This module is designed to be imported from Mojo test files:

    from wasm_harness import WasmInstance, get_instance

Import signatures are derived from `wasm-objdump -j Import -x build/out.wasm`:

  Import[15]:
   - func[0]  sig=0  (i64, i64) -> i64      KGEN_CompilerRT_AlignedAlloc
   - func[1]  sig=1  (i64) -> nil            KGEN_CompilerRT_AlignedFree
   - func[2]  sig=2  (f32, f32, f32) -> f32  fmaf
   - func[3]  sig=3  (f32, f32) -> f32       fminf
   - func[4]  sig=3  (f32, f32) -> f32       fmaxf
   - func[5]  sig=4  (f64, f64, f64) -> f64  fma
   - func[6]  sig=5  (f64, f64) -> f64       fmin
   - func[7]  sig=5  (f64, f64) -> f64       fmax
   - func[8]  sig=6  (i64, i64, i64) -> i64  write
   - func[9]  sig=7  (i32) -> i32            dup
   - func[10] sig=8  (i32, i64) -> i64       fdopen
   - func[11] sig=9  (i64) -> i32            fflush
   - func[12] sig=9  (i64) -> i32            fclose
   - func[13] sig=10 (i64, i64, i64) -> i32  KGEN_CompilerRT_fprintf
   - func[14] sig=11 () -> f64               performance_now
"""

from collections import Dict
from memory import UnsafePointer, memcpy, memset_zero, alloc
from pathlib import Path
from sys.ffi import OwnedDLHandle

from wasmtime_mojo import (
    Engine,
    Store,
    Module,
    Linker,
    WasmtimeVal,
    WasmtimeFunc,
    WasmtimeInstance,
    WasmtimeGlobal,
    WasmtimeMemory,
    WasmtimeExtern,
    WasmtimeCallback,
    ContextPtr,
    WASM_I32,
    WASM_I64,
    WASM_F32,
    WASM_F64,
    WASMTIME_I32,
    WASMTIME_I64,
    WASMTIME_F32,
    WASMTIME_F64,
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
    memory_read_i64_le,
    memory_write_i64_le,
    memory_read_u64_le,
)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

comptime STRING_STRUCT_SIZE: Int = 24
comptime STRING_STRUCT_ALIGN: Int = 8

comptime SSO_FLAG: UInt64 = 0x8000_0000_0000_0000
comptime SSO_LEN_MASK: UInt64 = 0x1F00_0000_0000_0000

comptime WASM_PATH = "build/out.wasm"
comptime CWASM_PATH = "build/out.cwasm"


# ---------------------------------------------------------------------------
# SharedState — heap-allocated state shared by all import callbacks
# ---------------------------------------------------------------------------


struct SharedState(Movable):
    """Mutable state shared across all WASM import callbacks.

    Allocated on the heap so a stable pointer can be passed as the
    callback `env` parameter.
    """

    var bump_ptr: Int
    var context: ContextPtr
    var memory: WasmtimeMemory
    var captured_stdout: List[String]
    var has_memory: Bool
    var mock_time: Float64  # P24.3: deterministic clock for performance_now

    # --- Free-list allocator state (P25.3) ---
    # Mirrors the JS-side size-class map allocator from runtime/memory.ts.
    # Reuse is enabled by default — double-free protection in aligned_free
    # ensures safe reuse even with WASM code that frees the same pointer twice.
    var ptr_size: Dict[Int, Int]  # pointer → allocation size
    var free_map: Dict[Int, List[Int]]  # size → LIFO stack of freed pointers
    var reuse_enabled: Bool

    fn __init__(out self):
        self.bump_ptr = 0
        self.context = ContextPtr()
        self.memory = WasmtimeMemory()
        self.captured_stdout = List[String]()
        self.has_memory = False
        self.mock_time = 0.0
        self.ptr_size = Dict[Int, Int]()
        self.free_map = Dict[Int, List[Int]]()
        self.reuse_enabled = True

    fn __moveinit__(out self, deinit other: Self):
        self.bump_ptr = other.bump_ptr
        self.context = other.context
        self.memory = other.memory
        self.captured_stdout = other.captured_stdout^
        self.has_memory = other.has_memory
        self.mock_time = other.mock_time
        self.ptr_size = other.ptr_size^
        self.free_map = other.free_map^
        self.reuse_enabled = other.reuse_enabled

    fn aligned_alloc(mut self, align: Int, size: Int) -> Int:
        """Allocate *size* bytes with the given alignment.

        If reuse is enabled, checks the size-class free list for an exact
        match (O(1) pop).  Otherwise falls back to bump allocation.
        Tracks ptr→size in a Dict for later free.
        """
        var actual = size if size >= 1 else 1

        # --- Size-class map path (O(1) pop) ---
        if self.reuse_enabled:
            var bucket = self.free_map.get(actual)
            if bucket:
                var b = bucket.value().copy()
                if len(b) > 0:
                    var ptr = b.pop()
                    self.free_map[actual] = b^
                    # Re-register in ptr_size so a future free can look up the size.
                    self.ptr_size[ptr] = actual
                    return ptr

        # --- Bump-allocator fallback ---
        var remainder = self.bump_ptr % align
        if remainder != 0:
            self.bump_ptr += align - remainder
        var ptr = self.bump_ptr
        self.bump_ptr += actual

        # Track the size so aligned_free can recover it later.
        self.ptr_size[ptr] = actual

        return ptr

    fn aligned_free(mut self, ptr: Int):
        """Free a previously allocated block.

        Looks up the block size from ptr_size and pushes the pointer onto
        the size-class free list.  If reuse is disabled the block is still
        tracked (visible in heap_stats) but won't be handed out.
        """
        if ptr == 0:
            return

        var size_opt = self.ptr_size.get(ptr)
        if not size_opt:
            return  # unknown or already-freed pointer — ignore

        var size = size_opt.value()

        # Remove from ptr_size so that a second free of the same pointer
        # (double-free from WASM) is detected as "unknown" and ignored.
        try:
            _ = self.ptr_size.pop(ptr)
        except:
            pass

        # Push onto the size-class bucket (O(1)).
        var bucket = self.free_map.get(size)
        if bucket:
            var b = bucket.value().copy()
            b.append(ptr)
            self.free_map[size] = b^
        else:
            var b = List[Int]()
            b.append(ptr)
            self.free_map[size] = b^

    fn heap_stats(self) raises -> Tuple[Int, Int, Int]:
        """Return (heap_pointer, free_blocks, free_bytes)."""
        var blocks = 0
        var bytes = 0
        # Collect keys into a list to avoid dict iterator subscript issues.
        var keys = List[Int]()
        for key_ref in self.free_map.keys():
            keys.append(key_ref)
        for i in range(len(keys)):
            var size = keys[i]
            var bucket = self.free_map.get(size)
            if bucket:
                var b = bucket.value().copy()
                blocks += len(b)
                bytes += size * len(b)
        return Tuple(self.bump_ptr, blocks, bytes)


# ---------------------------------------------------------------------------
# Import callbacks
#
# Each callback has the wasmtime_func_callback_t signature:
#   fn(env, caller, args, nargs, results, nresults) -> trap_ptr
#
# Return null pointer (UnsafePointer[NoneType, MutExternalOrigin]()) on success.
# ---------------------------------------------------------------------------


# Helper to get the SharedState from the env pointer.
@always_inline
fn _state(
    env: UnsafePointer[NoneType, MutExternalOrigin],
) -> UnsafePointer[SharedState, MutExternalOrigin]:
    return env.bitcast[SharedState]()


# -- func[0] KGEN_CompilerRT_AlignedAlloc: (i64, i64) -> i64 --------------


fn _cb_aligned_alloc(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var state = _state(env)
    var align = Int(args[0].get_i64())
    var size = Int(args[1].get_i64())
    var ptr = state[].aligned_alloc(align, size)
    results[0] = WasmtimeVal.from_i64(Int64(ptr))
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[1] KGEN_CompilerRT_AlignedFree: (i64) -> nil --------------------


fn _cb_aligned_free(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var state = _state(env)
    var ptr = Int(args[0].get_i64())
    state[].aligned_free(ptr)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[2] fmaf: (f32, f32, f32) -> f32 ---------------------------------


fn _cb_fmaf(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var x = args[0].get_f32()
    var y = args[1].get_f32()
    var z = args[2].get_f32()
    # fused multiply-add (truncated to f32 precision)
    var r = x * y + z
    results[0] = WasmtimeVal.from_f32(r)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[3] fminf: (f32, f32) -> f32 -------------------------------------


fn _cb_fminf(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var x = args[0].get_f32()
    var y = args[1].get_f32()
    var r = y if x > y else x
    results[0] = WasmtimeVal.from_f32(r)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[4] fmaxf: (f32, f32) -> f32 -------------------------------------


fn _cb_fmaxf(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var x = args[0].get_f32()
    var y = args[1].get_f32()
    var r = x if x > y else y
    results[0] = WasmtimeVal.from_f32(r)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[5] fma: (f64, f64, f64) -> f64 ----------------------------------


fn _cb_fma(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var x = args[0].get_f64()
    var y = args[1].get_f64()
    var z = args[2].get_f64()
    var r = x * y + z
    results[0] = WasmtimeVal.from_f64(r)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[6] fmin: (f64, f64) -> f64 --------------------------------------


fn _cb_fmin(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var x = args[0].get_f64()
    var y = args[1].get_f64()
    var r = y if x > y else x
    results[0] = WasmtimeVal.from_f64(r)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[7] fmax: (f64, f64) -> f64 --------------------------------------


fn _cb_fmax(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var x = args[0].get_f64()
    var y = args[1].get_f64()
    var r = x if x > y else y
    results[0] = WasmtimeVal.from_f64(r)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[8] write: (i64, i64, i64) -> i64 --------------------------------
# (moved up; was func[15] with i32 return in older Mojo)


# -- func[9] dup: (i32) -> i32 --------------------------------------------


fn _cb_dup(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    results[0] = WasmtimeVal.from_i32(1)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[11] fdopen: (i32, i64) -> i64 -----------------------------------


fn _cb_fdopen(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    results[0] = WasmtimeVal.from_i64(1)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[12] fflush: (i64) -> i32 ----------------------------------------


fn _cb_fflush(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    results[0] = WasmtimeVal.from_i32(1)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[13] fclose: (i64) -> i32 ----------------------------------------


fn _cb_fclose(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    results[0] = WasmtimeVal.from_i32(1)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[14] KGEN_CompilerRT_fprintf: (i64, i64, i64) -> i32 -------------


fn _cb_fprintf(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    results[0] = WasmtimeVal.from_i32(0)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[8] write: (i64, i64, i64) -> i64 --------------------------------


fn _cb_write(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var state = _state(env)
    var fd = Int(args[0].get_i64())
    var ptr = Int(args[1].get_i64())
    var length = Int(args[2].get_i64())

    if length == 0:
        results[0] = WasmtimeVal.from_i64(0)
        return UnsafePointer[NoneType, MutExternalOrigin]()

    if not state[].has_memory:
        results[0] = WasmtimeVal.from_i64(-1)
        return UnsafePointer[NoneType, MutExternalOrigin]()

    if fd == 1:
        # stdout — capture the written text
        try:
            var bytes = memory_read_bytes(
                state[].context, state[].memory, ptr, length
            )
            var buf = List[UInt8](capacity=length)
            for i in range(length):
                buf.append(bytes[i])
            var text = String(unsafe_from_utf8=buf)
            state[].captured_stdout.append(text)
            results[0] = WasmtimeVal.from_i64(Int64(length))
        except:
            results[0] = WasmtimeVal.from_i64(-1)
    elif fd == 2:
        # stderr — just report length, don't capture
        results[0] = WasmtimeVal.from_i64(Int64(length))
    else:
        results[0] = WasmtimeVal.from_i64(-1)

    return UnsafePointer[NoneType, MutExternalOrigin]()


# -- func[16] performance_now: () -> f64 -----------------------------------
# P24.3: Deterministic mock clock for benchmark timing tests.
# Each call returns the current mock_time and advances it by 1.0,
# so before/after pairs always yield a predictable delta.


fn _cb_performance_now(
    env: UnsafePointer[NoneType, MutExternalOrigin],
    caller: UnsafePointer[NoneType, MutExternalOrigin],
    args: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nargs: Int,
    results: UnsafePointer[WasmtimeVal, MutExternalOrigin],
    nresults: Int,
) -> UnsafePointer[NoneType, MutExternalOrigin]:
    var state = _state(env)
    var t = state[].mock_time
    state[].mock_time += 1.0
    results[0] = WasmtimeVal.from_f64(t)
    return UnsafePointer[NoneType, MutExternalOrigin]()


# ---------------------------------------------------------------------------
# WasmInstance — high-level harness wrapping engine, store, linker, instance
# ---------------------------------------------------------------------------


struct WasmInstance(Movable):
    """Wraps a Wasmtime instance with helper methods mirroring the Python harness.

    Provides:
    - `call(name, args)` to invoke exported WASM functions
    - `write_string_struct(s)` / `read_string_struct(ptr)` for Mojo String structs
    - `alloc_string_struct()` for allocating empty string struct slots
    - Typed call shortcuts (call_i32, call_i64, call_f32, call_f64)
    """

    var _engine: Engine
    var _store: Store
    var _module: Module
    var _linker: Linker
    var _instance: WasmtimeInstance
    var _memory: WasmtimeMemory
    var _state_ptr: UnsafePointer[SharedState, MutExternalOrigin]

    fn __init__(out self, wasm_path: String) raises:
        """Create a WasmInstance by loading and instantiating the WASM binary.

        Args:
            wasm_path: Path to the .wasm binary file.
        """
        # Read the WASM binary
        var wasm_bytes = Path(wasm_path).read_bytes()

        # Allocate shared state on the heap
        self._state_ptr = alloc[SharedState](1)
        self._state_ptr.init_pointee_move(SharedState())
        var env = self._state_ptr.bitcast[NoneType]()
        var no_fin = UnsafePointer[NoneType, MutExternalOrigin]()

        # Create engine with module caching enabled.
        # Each mojo-test function runs in a separate process, so in-memory
        # sharing is impossible.  Wasmtime's file-based module cache
        # persists the compiled machine code to disk (~/.cache/wasmtime),
        # letting subsequent processes skip the expensive WASM compilation.
        self._engine = Engine(cache=True)
        self._store = Store(self._engine.ptr())
        self._linker = Linker(self._engine.ptr())

        # ──────────────────────────────────────────────────────────────
        # Define all 16 imports
        # ──────────────────────────────────────────────────────────────

        # func[0] KGEN_CompilerRT_AlignedAlloc: (i64, i64) -> i64
        self._linker.define_func(
            "env",
            "KGEN_CompilerRT_AlignedAlloc",
            [WASM_I64, WASM_I64],
            [WASM_I64],
            _cb_aligned_alloc,
            env,
        )

        # func[1] KGEN_CompilerRT_AlignedFree: (i64) -> nil
        self._linker.define_func(
            "env",
            "KGEN_CompilerRT_AlignedFree",
            [WASM_I64],
            List[UInt8](),
            _cb_aligned_free,
            env,
        )

        # func[2] fmaf: (f32, f32, f32) -> f32
        self._linker.define_func(
            "env",
            "fmaf",
            [WASM_F32, WASM_F32, WASM_F32],
            [WASM_F32],
            _cb_fmaf,
            env,
        )

        # func[3] fminf: (f32, f32) -> f32
        self._linker.define_func(
            "env",
            "fminf",
            [WASM_F32, WASM_F32],
            [WASM_F32],
            _cb_fminf,
            env,
        )

        # func[4] fmaxf: (f32, f32) -> f32
        self._linker.define_func(
            "env",
            "fmaxf",
            [WASM_F32, WASM_F32],
            [WASM_F32],
            _cb_fmaxf,
            env,
        )

        # func[5] fma: (f64, f64, f64) -> f64
        self._linker.define_func(
            "env",
            "fma",
            [WASM_F64, WASM_F64, WASM_F64],
            [WASM_F64],
            _cb_fma,
            env,
        )

        # func[6] fmin: (f64, f64) -> f64
        self._linker.define_func(
            "env",
            "fmin",
            [WASM_F64, WASM_F64],
            [WASM_F64],
            _cb_fmin,
            env,
        )

        # func[7] fmax: (f64, f64) -> f64
        self._linker.define_func(
            "env",
            "fmax",
            [WASM_F64, WASM_F64],
            [WASM_F64],
            _cb_fmax,
            env,
        )

        # func[9] dup: (i32) -> i32
        self._linker.define_func(
            "env",
            "dup",
            [WASM_I32],
            [WASM_I32],
            _cb_dup,
            env,
        )

        # func[10] fdopen: (i32, i64) -> i64
        self._linker.define_func(
            "env",
            "fdopen",
            [WASM_I32, WASM_I64],
            [WASM_I64],
            _cb_fdopen,
            env,
        )

        # func[11] fflush: (i64) -> i32
        self._linker.define_func(
            "env",
            "fflush",
            [WASM_I64],
            [WASM_I32],
            _cb_fflush,
            env,
        )

        # func[12] fclose: (i64) -> i32
        self._linker.define_func(
            "env",
            "fclose",
            [WASM_I64],
            [WASM_I32],
            _cb_fclose,
            env,
        )

        # func[13] KGEN_CompilerRT_fprintf: (i64, i64, i64) -> i32
        self._linker.define_func(
            "env",
            "KGEN_CompilerRT_fprintf",
            [WASM_I64, WASM_I64, WASM_I64],
            [WASM_I32],
            _cb_fprintf,
            env,
        )

        # func[8] write: (i64, i64, i64) -> i64
        self._linker.define_func(
            "env",
            "write",
            [WASM_I64, WASM_I64, WASM_I64],
            [WASM_I64],
            _cb_write,
            env,
        )

        # func[14] performance_now: () -> f64 (P24.3)
        self._linker.define_func(
            "env",
            "performance_now",
            List[UInt8](),
            [WASM_F64],
            _cb_performance_now,
            env,
        )

        # ──────────────────────────────────────────────────────────────
        # Load the module: prefer pre-compiled .cwasm (very fast mmap),
        # fall back to compiling from .wasm bytes (cached by engine).
        # ──────────────────────────────────────────────────────────────

        var cwasm_path = wasm_path.removesuffix(".wasm") + ".cwasm"
        if Path(cwasm_path).exists():
            self._module = Module.deserialize_file(
                self._engine.ptr(), cwasm_path
            )
        else:
            self._module = Module(self._engine.ptr(), wasm_bytes)
        self._instance = self._linker.instantiate(
            self._store.context(), self._module.ptr()
        )

        # Obtain the memory export
        self._memory = instance_get_memory(
            self._store.context(), self._instance, "memory"
        )

        # Read __heap_base global to initialise the bump allocator
        var heap_base_global = instance_get_global(
            self._store.context(), self._instance, "__heap_base"
        )
        var heap_base = global_get_i64(self._store.context(), heap_base_global)

        # Update shared state with memory and context info
        self._state_ptr[].bump_ptr = Int(heap_base)
        self._state_ptr[].context = self._store.context()
        self._state_ptr[].memory = self._memory
        self._state_ptr[].has_memory = True

    fn __del__(deinit self):
        """Clean up: free the heap-allocated shared state."""
        if self._state_ptr:
            self._state_ptr.destroy_pointee()
            self._state_ptr.free()

    fn __moveinit__(out self, deinit other: Self):
        """Move constructor."""
        self._engine = other._engine^
        self._store = other._store^
        self._module = other._module^
        self._linker = other._linker^
        self._instance = other._instance
        self._memory = other._memory
        self._state_ptr = other._state_ptr

    # ------------------------------------------------------------------
    # Raw memory helpers
    # ------------------------------------------------------------------

    fn read_bytes(self, ptr: Int, length: Int) raises -> List[UInt8]:
        """Read *length* bytes from WASM memory at *ptr*."""
        return memory_read_bytes(
            self._store.context(), self._memory, ptr, length
        )

    fn write_bytes(self, ptr: Int, data: List[UInt8]) raises:
        """Write *data* bytes into WASM memory at *ptr*."""
        memory_write_bytes(self._store.context(), self._memory, ptr, data)

    fn read_i64_le(self, ptr: Int) raises -> Int64:
        """Read a little-endian i64 from WASM memory."""
        return memory_read_i64_le(self._store.context(), self._memory, ptr)

    fn read_u64_le(self, ptr: Int) raises -> UInt64:
        """Read a little-endian u64 from WASM memory."""
        return memory_read_u64_le(self._store.context(), self._memory, ptr)

    fn write_i64_le(self, ptr: Int, value: Int64) raises:
        """Write a little-endian i64 into WASM memory."""
        memory_write_i64_le(self._store.context(), self._memory, ptr, value)

    # ------------------------------------------------------------------
    # Bump allocator wrappers
    # ------------------------------------------------------------------

    fn aligned_alloc(self, align: Int, size: Int) -> Int:
        """Bump-allocate *size* bytes with the given alignment."""
        return self._state_ptr[].aligned_alloc(align, size)

    fn heap_stats(self) raises -> Tuple[Int, Int, Int]:
        """Return (heap_pointer, free_blocks, free_bytes) from the JS-side allocator.
        """
        return self._state_ptr[].heap_stats()

    # ------------------------------------------------------------------
    # String struct operations (mirrors runtime/strings.ts)
    # ------------------------------------------------------------------

    fn write_string_struct(self, s: String) raises -> Int:
        """Allocate a Mojo String struct in WASM memory, populated with *s*.

        The struct is 24 bytes:
          - offset  0: data_ptr (i64) — pointer to UTF-8 bytes
          - offset  8: len      (i64) — byte length (no null terminator)
          - offset 16: capacity (i64) — buffer capacity (len + 1)

        Returns the WASM pointer to the struct.
        """
        var encoded = s.as_bytes()
        var data_len = len(s)

        # Allocate data buffer (with null terminator)
        var data_ptr = self.aligned_alloc(1, data_len + 1)
        var data_bytes = List[UInt8](capacity=data_len + 1)
        for i in range(len(encoded)):
            data_bytes.append(encoded[i])
        # Ensure null terminator
        if len(data_bytes) == 0 or data_bytes[len(data_bytes) - 1] != 0:
            data_bytes.append(0)
        self.write_bytes(data_ptr, data_bytes)

        # Allocate 24-byte String struct
        var struct_ptr = self.aligned_alloc(
            STRING_STRUCT_ALIGN, STRING_STRUCT_SIZE
        )
        self.write_i64_le(struct_ptr, Int64(data_ptr))
        self.write_i64_le(struct_ptr + 8, Int64(data_len))
        self.write_i64_le(struct_ptr + 16, Int64(data_len + 1))

        return struct_ptr

    fn alloc_string_struct(self) raises -> Int:
        """Allocate a zero-initialized 24-byte Mojo String struct.

        Returns the WASM pointer to the struct.
        """
        var struct_ptr = self.aligned_alloc(
            STRING_STRUCT_ALIGN, STRING_STRUCT_SIZE
        )
        var zeros = List[UInt8](capacity=STRING_STRUCT_SIZE)
        for _ in range(STRING_STRUCT_SIZE):
            zeros.append(0)
        self.write_bytes(struct_ptr, zeros)
        return struct_ptr

    fn read_string_struct(self, struct_ptr: Int) raises -> String:
        """Read a Mojo String struct back into a Mojo String.

        Handles both heap-allocated strings and SSO (small string optimization)
        strings.
        """
        var capacity = self.read_u64_le(struct_ptr + 16)

        var data_ptr: Int
        var length: Int

        if capacity & SSO_FLAG:
            # SSO: data inline at struct_ptr, length encoded in capacity
            data_ptr = struct_ptr
            length = Int((capacity & SSO_LEN_MASK) >> 56)
        else:
            data_ptr = Int(self.read_i64_le(struct_ptr))
            length = Int(self.read_i64_le(struct_ptr + 8))

        if length <= 0:
            return String("")

        var raw = self.read_bytes(data_ptr, length)
        var buf = List[UInt8](capacity=length)
        for i in range(length):
            buf.append(raw[i])
        return String(unsafe_from_utf8=buf)

    # ------------------------------------------------------------------
    # Captured stdout access
    # ------------------------------------------------------------------

    fn get_captured_stdout(self) raises -> List[String]:
        """Return the list of strings captured from WASM stdout writes."""
        return self._state_ptr[].captured_stdout.copy()

    fn clear_captured_stdout(self) raises:
        """Clear the captured stdout buffer."""
        self._state_ptr[].captured_stdout = List[String]()

    # ------------------------------------------------------------------
    # WASM function calling
    # ------------------------------------------------------------------

    fn get_func(self, name: String) raises -> WasmtimeFunc:
        """Look up an exported function by name.

        Args:
            name: The export function name.

        Returns:
            A WasmtimeFunc handle for the exported function.

        Raises:
            Error: If the export is not found or is not a function.
        """
        return instance_get_func(self._store.context(), self._instance, name)

    fn call(
        self,
        name: String,
        args: List[WasmtimeVal],
        nresults: Int = 1,
    ) raises -> List[WasmtimeVal]:
        """Call an exported WASM function by name.

        Args:
            name: The export function name.
            args: Arguments as WasmtimeVal values.
            nresults: Expected number of results (default 1).

        Returns:
            A List of WasmtimeVal results.

        Raises:
            Error: If the function is not found or the call fails/traps.
        """
        var func = self.get_func(name)
        return func_call(self._store.context(), func, args, nresults)

    fn call_void(self, name: String, args: List[WasmtimeVal]) raises:
        """Call an exported WASM function that returns no values.

        Args:
            name: The export function name.
            args: Arguments as WasmtimeVal values.

        Raises:
            Error: If the function is not found or the call fails/traps.
        """
        var func = self.get_func(name)
        func_call_0(self._store.context(), func, args)

    fn call_i32(self, name: String, args: List[WasmtimeVal]) raises -> Int32:
        """Call an exported WASM function and return a single i32 result.

        Args:
            name: The export function name.
            args: Arguments as WasmtimeVal values.

        Returns:
            The i32 result.

        Raises:
            Error: If the function is not found or the call fails/traps.
        """
        var func = self.get_func(name)
        return func_call_i32(self._store.context(), func, args)

    fn call_i64(self, name: String, args: List[WasmtimeVal]) raises -> Int64:
        """Call an exported WASM function and return a single i64 result.

        Args:
            name: The export function name.
            args: Arguments as WasmtimeVal values.

        Returns:
            The i64 result.

        Raises:
            Error: If the function is not found or the call fails/traps.
        """
        var func = self.get_func(name)
        return func_call_i64(self._store.context(), func, args)

    fn call_f32(self, name: String, args: List[WasmtimeVal]) raises -> Float32:
        """Call an exported WASM function and return a single f32 result.

        Args:
            name: The export function name.
            args: Arguments as WasmtimeVal values.

        Returns:
            The f32 result.

        Raises:
            Error: If the function is not found or the call fails/traps.
        """
        var func = self.get_func(name)
        return func_call_f32(self._store.context(), func, args)

    fn call_f64(self, name: String, args: List[WasmtimeVal]) raises -> Float64:
        """Call an exported WASM function and return a single f64 result.

        Args:
            name: The export function name.
            args: Arguments as WasmtimeVal values.

        Returns:
            The f64 result.

        Raises:
            Error: If the function is not found or the call fails/traps.
        """
        var func = self.get_func(name)
        return func_call_f64(self._store.context(), func, args)


# ---------------------------------------------------------------------------
# Convenience: build argument lists
# ---------------------------------------------------------------------------


fn args_i32(a: Int32) -> List[WasmtimeVal]:
    """Build a single-i32 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i32(a))
    return v^


fn args_i32_i32(a: Int32, b: Int32) -> List[WasmtimeVal]:
    """Build a two-i32 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    return v^


fn args_i64(a: Int64) -> List[WasmtimeVal]:
    """Build a single-i64 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(a))
    return v^


fn args_i64_i64(a: Int64, b: Int64) -> List[WasmtimeVal]:
    """Build a two-i64 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(a))
    v.append(WasmtimeVal.from_i64(b))
    return v^


fn args_f32(a: Float32) -> List[WasmtimeVal]:
    """Build a single-f32 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_f32(a))
    return v^


fn args_f32_f32(a: Float32, b: Float32) -> List[WasmtimeVal]:
    """Build a two-f32 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_f32(a))
    v.append(WasmtimeVal.from_f32(b))
    return v^


fn args_f64(a: Float64) -> List[WasmtimeVal]:
    """Build a single-f64 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_f64(a))
    return v^


fn args_f64_f64(a: Float64, b: Float64) -> List[WasmtimeVal]:
    """Build a two-f64 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_f64(a))
    v.append(WasmtimeVal.from_f64(b))
    return v^


fn args_i32_i32_i32(a: Int32, b: Int32, c: Int32) -> List[WasmtimeVal]:
    """Build a three-i32 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i32(c))
    return v^


fn args_f64_f64_f64(a: Float64, b: Float64, c: Float64) -> List[WasmtimeVal]:
    """Build a three-f64 argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_f64(a))
    v.append(WasmtimeVal.from_f64(b))
    v.append(WasmtimeVal.from_f64(c))
    return v^


fn args_ptr(ptr: Int) -> List[WasmtimeVal]:
    """Build a single-pointer (i64) argument list from an Int address."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    return v^


fn args_ptr_ptr(a: Int, b: Int) -> List[WasmtimeVal]:
    """Build a two-pointer (i64, i64) argument list from Int addresses."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    return v^


fn args_ptr_i32(ptr: Int, val: Int32) -> List[WasmtimeVal]:
    """Build a (ptr, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(val))
    return v^


fn args_ptr_i32_i32(ptr: Int, a: Int32, b: Int32) -> List[WasmtimeVal]:
    """Build a (ptr, i32, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    return v^


fn args_ptr_i32_i32_i32(
    ptr: Int, a: Int32, b: Int32, c: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, i32, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i32(c))
    return v^


fn args_ptr_i32_i32_i32_ptr(
    ptr: Int, a: Int32, b: Int32, c: Int32, ptr2: Int
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, i32, i32, ptr) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i32(c))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    return v^


fn args_ptr_i32_ptr(ptr: Int, val: Int32, ptr2: Int) -> List[WasmtimeVal]:
    """Build a (ptr, i32, ptr) argument list — i64, i32, i64."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(val))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    return v^


fn args_ptr_i64_ptr(a: Int, b: Int64, c: Int) -> List[WasmtimeVal]:
    """Build a (ptr, i64, ptr) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(b))
    v.append(WasmtimeVal.from_i64(Int64(c)))
    return v^


fn args_ptr_ptr_ptr(a: Int, b: Int, c: Int) -> List[WasmtimeVal]:
    """Build a three-pointer (i64, i64, i64) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i64(Int64(c)))
    return v^


fn args_ptr_ptr_i32(a: Int, b: Int, c: Int32) -> List[WasmtimeVal]:
    """Build a (ptr, ptr, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i32(c))
    return v^


fn args_ptr_ptr_i32_i32(
    a: Int, b: Int, c: Int32, d: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, ptr, i32, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i32(c))
    v.append(WasmtimeVal.from_i32(d))
    return v^


fn args_ptr_ptr_i32_i32_i32(
    a: Int, b: Int, c: Int32, d: Int32, e: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, ptr, i32, i32, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i32(c))
    v.append(WasmtimeVal.from_i32(d))
    v.append(WasmtimeVal.from_i32(e))
    return v^


fn args_ptr_ptr_ptr_ptr(a: Int, b: Int, c: Int, d: Int) -> List[WasmtimeVal]:
    """Build a four-pointer (i64, i64, i64, i64) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i64(Int64(c)))
    v.append(WasmtimeVal.from_i64(Int64(d)))
    return v^


fn args_ptr_ptr_ptr_ptr_i32(
    a: Int, b: Int, c: Int, d: Int, e: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, ptr, ptr, ptr, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i64(Int64(c)))
    v.append(WasmtimeVal.from_i64(Int64(d)))
    v.append(WasmtimeVal.from_i32(e))
    return v^


fn args_ptr_ptr_ptr_ptr_i32_i32(
    a: Int, b: Int, c: Int, d: Int, e: Int32, f: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, ptr, ptr, ptr, i32, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(a)))
    v.append(WasmtimeVal.from_i64(Int64(b)))
    v.append(WasmtimeVal.from_i64(Int64(c)))
    v.append(WasmtimeVal.from_i64(Int64(d)))
    v.append(WasmtimeVal.from_i32(e))
    v.append(WasmtimeVal.from_i32(f))
    return v^


fn args_ptr_i32_i32_ptr(
    ptr: Int, a: Int32, b: Int32, ptr2: Int
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, i32, ptr) argument list — i64, i32, i32, i64."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    return v^


fn args_ptr_i32_i32_i32_i32(
    ptr: Int, a: Int32, b: Int32, c: Int32, d: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, i32, i32, i32) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i32(c))
    v.append(WasmtimeVal.from_i32(d))
    return v^


fn args_ptr_i32_i32_i32_ptr_ptr(
    ptr: Int, a: Int32, b: Int32, c: Int32, ptr2: Int, ptr3: Int
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, i32, i32, ptr, ptr) argument list."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i32(c))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    v.append(WasmtimeVal.from_i64(Int64(ptr3)))
    return v^


fn args_ptr_i32_ptr_ptr(
    ptr: Int, val: Int32, ptr2: Int, ptr3: Int
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, ptr, ptr) argument list — i64, i32, i64, i64."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(val))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    v.append(WasmtimeVal.from_i64(Int64(ptr3)))
    return v^


fn args_ptr_i32_ptr_i32(
    ptr: Int, a: Int32, ptr2: Int, b: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, ptr, i32) argument list — i64, i32, i64, i32."""
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    v.append(WasmtimeVal.from_i32(b))
    return v^


fn args_ptr_i32_ptr_i32_i32(
    ptr: Int, a: Int32, ptr2: Int, b: Int32, c: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, ptr, i32, i32) argument list — i64, i32, i64, i32, i32.
    """
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    v.append(WasmtimeVal.from_i32(b))
    v.append(WasmtimeVal.from_i32(c))
    return v^


fn args_ptr_i32_ptr_ptr_i32(
    ptr: Int, a: Int32, ptr2: Int, ptr3: Int, b: Int32
) -> List[WasmtimeVal]:
    """Build a (ptr, i32, ptr, ptr, i32) argument list — i64, i32, i64, i64, i32.
    """
    var v = List[WasmtimeVal]()
    v.append(WasmtimeVal.from_i64(Int64(ptr)))
    v.append(WasmtimeVal.from_i32(a))
    v.append(WasmtimeVal.from_i64(Int64(ptr2)))
    v.append(WasmtimeVal.from_i64(Int64(ptr3)))
    v.append(WasmtimeVal.from_i32(b))
    return v^


fn no_args() -> List[WasmtimeVal]:
    """Build an empty argument list."""
    return List[WasmtimeVal]()


# ---------------------------------------------------------------------------
# Instance factory — create a fresh instance for each caller.
#
# Module-level `var` is not supported in Mojo 0.25.6 and `mojo test`
# runs every test function in a separate process, so in-memory singleton
# caching is impossible.
#
# Instead we rely on two layers of on-disk caching to avoid recompiling
# the WASM module from scratch on every test invocation:
#
#   1. Pre-compiled `.cwasm` — `just build` serializes the compiled
#      module to `build/out.cwasm`.  The harness loads it via
#      `Module.deserialize_file` (essentially an mmap, very fast).
#
#   2. Wasmtime engine cache — `Engine(cache=True)` enables wasmtime's
#      built-in file-based cache (~/.cache/wasmtime) as a fallback when
#      no `.cwasm` file is present.
# ---------------------------------------------------------------------------


fn get_instance() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    """Create a new WasmInstance and return a heap pointer to it.

    Returns:
        UnsafePointer to a freshly allocated WasmInstance.

    Raises:
        Error: If the WASM binary cannot be loaded or instantiated.
    """
    var ptr = alloc[WasmInstance](1)
    ptr.init_pointee_move(WasmInstance(WASM_PATH))
    return ptr
