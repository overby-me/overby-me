"""C struct type definitions for the wasmtime C API.

These structs mirror the exact memory layout of their C counterparts
so they can be passed directly through FFI boundaries.

Layout reference (64-bit Linux):
  wasmtime_val_t:    32 bytes (kind:1 + pad:7 + union:24)
  wasmtime_extern_t: 32 bytes (kind:1 + pad:7 + union:24)
  wasmtime_func_t:   16 bytes (store_id:8 + __private:8)
  wasmtime_instance_t: 16 bytes (store_id:8 + __private:8)
  wasmtime_global_t: 24 bytes (store_id:8 + 3×u32:12 + pad:4)
  wasmtime_memory_t: 24 bytes (store_id:8 + u32:4 + pad:4 + u32:4 + pad:4)
  wasm_valtype_vec_t: 16 bytes (size:8 + data:8)
  wasm_byte_vec_t:   16 bytes (size:8 + data:8)

IMPORTANT: All struct storage uses UInt64 fields instead of SIMD vectors.
SIMD[DType.uint8, N] has N-byte alignment (e.g. 32-byte for N=32), but
the C wasmtime structs are only 8-byte aligned.  When wasmtime passes
callback args/results as pointers to its own stack-allocated arrays,
the 8-byte-aligned addresses cause SIGSEGV on SIMD-aligned loads/stores.
Using UInt64 fields gives correct 8-byte alignment matching C.
"""

from memory import UnsafePointer, memset_zero, memcpy
from sys.info import sizeof

# ---------------------------------------------------------------------------
# Wasmtime val kind constants (wasmtime_valkind_t)
# ---------------------------------------------------------------------------

comptime WASMTIME_I32: UInt8 = 0
comptime WASMTIME_I64: UInt8 = 1
comptime WASMTIME_F32: UInt8 = 2
comptime WASMTIME_F64: UInt8 = 3
comptime WASMTIME_V128: UInt8 = 4
comptime WASMTIME_FUNCREF: UInt8 = 5
comptime WASMTIME_EXTERNREF: UInt8 = 6

# ---------------------------------------------------------------------------
# WASM val kind constants (wasm_valkind_t) — used by wasm_valtype_new
# ---------------------------------------------------------------------------

comptime WASM_I32: UInt8 = 0
comptime WASM_I64: UInt8 = 1
comptime WASM_F32: UInt8 = 2
comptime WASM_F64: UInt8 = 3
comptime WASM_EXTERNREF: UInt8 = 128
comptime WASM_FUNCREF: UInt8 = 129

# ---------------------------------------------------------------------------
# Wasmtime extern kind constants (wasmtime_extern_kind_t)
# ---------------------------------------------------------------------------

comptime WASMTIME_EXTERN_FUNC: UInt8 = 0
comptime WASMTIME_EXTERN_GLOBAL: UInt8 = 1
comptime WASMTIME_EXTERN_TABLE: UInt8 = 2
comptime WASMTIME_EXTERN_MEMORY: UInt8 = 3
comptime WASMTIME_EXTERN_SHAREDMEMORY: UInt8 = 4

# ---------------------------------------------------------------------------
# Sizes of C structs (bytes)
# ---------------------------------------------------------------------------

comptime WASMTIME_VAL_SIZE = 32
comptime WASMTIME_EXTERN_SIZE = 32
comptime WASMTIME_FUNC_SIZE = 16
comptime WASMTIME_INSTANCE_SIZE = 16
comptime WASMTIME_GLOBAL_SIZE = 24
comptime WASMTIME_MEMORY_SIZE = 24
comptime WASMTIME_TABLE_SIZE = 24
comptime WASM_VALTYPE_VEC_SIZE = 16
comptime WASM_BYTE_VEC_SIZE = 16


# ---------------------------------------------------------------------------
# Opaque pointer aliases — these wrap C types we never inspect directly
# ---------------------------------------------------------------------------

comptime EnginePtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime StorePtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime ContextPtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime ModulePtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime LinkerPtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime ErrorPtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime TrapPtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime FuncTypePtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime ValTypePtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime CallerPtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime GlobalTypePtr = UnsafePointer[NoneType, MutExternalOrigin]
comptime ExternTypePtr = UnsafePointer[NoneType, MutExternalOrigin]


# ---------------------------------------------------------------------------
# wasmtime_val_t — 32-byte tagged union for WASM values
#
# C layout:
#   struct wasmtime_val_t {
#       uint8_t kind;          // offset 0
#       // 7 bytes padding
#       wasmtime_valunion_t of; // offset 8, 24 bytes
#   };
#
# The union's largest members are anyref/externref at 24 bytes.
# For our purposes we only need i32/i64/f32/f64 access.
#
# Storage: 4 × UInt64 = 32 bytes, 8-byte aligned (matches C).
# ---------------------------------------------------------------------------


struct WasmtimeVal(TrivialRegisterPassable):
    """Mirrors wasmtime_val_t (32 bytes, 8-byte aligned)."""

    var _w0: UInt64  # kind (byte 0) + 7 bytes padding
    var _w1: UInt64  # value union bytes 0-7 (offset 8)
    var _w2: UInt64  # value union bytes 8-15 (offset 16)
    var _w3: UInt64  # value union bytes 16-23 (offset 24)

    @always_inline
    fn __init__(out self):
        self._w0 = 0
        self._w1 = 0
        self._w2 = 0
        self._w3 = 0

    # -- Kind accessor (offset 0, lowest byte of _w0) --

    @always_inline
    fn get_kind(self) -> UInt8:
        return UInt8(self._w0 & 0xFF)

    @always_inline
    fn set_kind(mut self, kind: UInt8):
        self._w0 = (self._w0 & ~UInt64(0xFF)) | UInt64(kind)

    # -- i32 accessor (offset 8, stored in _w1 low 32 bits) --

    @always_inline
    fn get_i32(self) -> Int32:
        return Int32(Int(self._w1) & 0xFFFFFFFF)

    @always_inline
    fn set_i32(mut self, value: Int32):
        self._w1 = UInt64(Int64(value) & 0xFFFFFFFF)

    # -- i64 accessor (offset 8, stored in _w1) --

    @always_inline
    fn get_i64(self) -> Int64:
        return Int64(self._w1)

    @always_inline
    fn set_i64(mut self, value: Int64):
        self._w1 = UInt64(value)

    # -- f32 accessor (offset 8, stored in _w1 low 32 bits) --

    @always_inline
    fn get_f32(self) -> Float32:
        var bits = Int32(Int(self._w1) & 0xFFFFFFFF)
        return UnsafePointer(to=bits).bitcast[Float32]()[]

    @always_inline
    fn set_f32(mut self, value: Float32):
        var bits = UnsafePointer(to=value).bitcast[Int32]()[]
        self._w1 = UInt64(Int64(bits) & 0xFFFFFFFF)

    # -- f64 accessor (offset 8, stored in _w1) --

    @always_inline
    fn get_f64(self) -> Float64:
        var w = self._w1
        return UnsafePointer(to=w).bitcast[Float64]()[]

    @always_inline
    fn set_f64(mut self, value: Float64):
        self._w1 = UnsafePointer(to=value).bitcast[UInt64]()[]

    # -- Convenience constructors --

    @staticmethod
    fn from_i32(value: Int32) -> WasmtimeVal:
        var v = WasmtimeVal()
        v.set_kind(WASMTIME_I32)
        v.set_i32(value)
        return v

    @staticmethod
    fn from_i64(value: Int64) -> WasmtimeVal:
        var v = WasmtimeVal()
        v.set_kind(WASMTIME_I64)
        v.set_i64(value)
        return v

    @staticmethod
    fn from_f32(value: Float32) -> WasmtimeVal:
        var v = WasmtimeVal()
        v.set_kind(WASMTIME_F32)
        v.set_f32(value)
        return v

    @staticmethod
    fn from_f64(value: Float64) -> WasmtimeVal:
        var v = WasmtimeVal()
        v.set_kind(WASMTIME_F64)
        v.set_f64(value)
        return v


# ---------------------------------------------------------------------------
# wasmtime_func_t — 16 bytes
#
# C layout:
#   struct wasmtime_func_t {
#       uint64_t store_id;   // offset 0
#       size_t   __private;  // offset 8
#   };
#
# Storage: 2 × UInt64 = 16 bytes, 8-byte aligned.
# ---------------------------------------------------------------------------


struct WasmtimeFunc(TrivialRegisterPassable):
    """Mirrors wasmtime_func_t (16 bytes, 8-byte aligned)."""

    var _w0: UInt64
    var _w1: UInt64

    @always_inline
    fn __init__(out self):
        self._w0 = 0
        self._w1 = 0


# ---------------------------------------------------------------------------
# wasmtime_instance_t — 16 bytes
#
# C layout:
#   struct wasmtime_instance_t {
#       uint64_t store_id;   // offset 0
#       size_t   __private;  // offset 8
#   };
#
# Storage: 2 × UInt64 = 16 bytes, 8-byte aligned.
# ---------------------------------------------------------------------------


struct WasmtimeInstance(TrivialRegisterPassable):
    """Mirrors wasmtime_instance_t (16 bytes, 8-byte aligned)."""

    var _w0: UInt64
    var _w1: UInt64

    @always_inline
    fn __init__(out self):
        self._w0 = 0
        self._w1 = 0


# ---------------------------------------------------------------------------
# wasmtime_global_t — 24 bytes
#
# C layout:
#   struct wasmtime_global_t {
#       uint64_t store_id;      // offset 0
#       uint32_t __private1;    // offset 8
#       uint32_t __private2;    // offset 12
#       uint32_t __private3;    // offset 16
#       // 4 bytes padding to align to 8
#   };
#
# Storage: 3 × UInt64 = 24 bytes, 8-byte aligned.
# ---------------------------------------------------------------------------


struct WasmtimeGlobal(TrivialRegisterPassable):
    """Mirrors wasmtime_global_t (24 bytes, 8-byte aligned)."""

    var _w0: UInt64
    var _w1: UInt64
    var _w2: UInt64

    @always_inline
    fn __init__(out self):
        self._w0 = 0
        self._w1 = 0
        self._w2 = 0


# ---------------------------------------------------------------------------
# wasmtime_memory_t — 24 bytes
#
# C layout (from Python):
#   struct _anon { uint64_t store_id; uint32_t __private1; };
#   struct wasmtime_memory_t { _anon; uint32_t __private2; };
#
# Total: 8 + 4 + (4 pad) + 4 + (4 pad) = 24 bytes
#
# Storage: 3 × UInt64 = 24 bytes, 8-byte aligned.
# ---------------------------------------------------------------------------


struct WasmtimeMemory(TrivialRegisterPassable):
    """Mirrors wasmtime_memory_t (24 bytes, 8-byte aligned)."""

    var _w0: UInt64
    var _w1: UInt64
    var _w2: UInt64

    @always_inline
    fn __init__(out self):
        self._w0 = 0
        self._w1 = 0
        self._w2 = 0


# ---------------------------------------------------------------------------
# wasmtime_extern_t — 32-byte tagged union for exported items
#
# C layout:
#   struct wasmtime_extern_t {
#       uint8_t kind;               // offset 0
#       // 7 bytes padding
#       wasmtime_extern_union_t of; // offset 8, 24 bytes
#   };
#
# The union holds func/global/table/memory/sharedmemory.
#
# Storage: 4 × UInt64 = 32 bytes, 8-byte aligned.
# ---------------------------------------------------------------------------


struct WasmtimeExtern(TrivialRegisterPassable):
    """Mirrors wasmtime_extern_t (32 bytes, 8-byte aligned)."""

    var _w0: UInt64  # kind (byte 0) + 7 bytes padding
    var _w1: UInt64  # union bytes 0-7 (offset 8)
    var _w2: UInt64  # union bytes 8-15 (offset 16)
    var _w3: UInt64  # union bytes 16-23 (offset 24)

    @always_inline
    fn __init__(out self):
        self._w0 = 0
        self._w1 = 0
        self._w2 = 0
        self._w3 = 0

    @always_inline
    fn get_kind(self) -> UInt8:
        return UInt8(self._w0 & 0xFF)

    @always_inline
    fn get_func(self) -> WasmtimeFunc:
        """Extract the func from the union (valid when kind == WASMTIME_EXTERN_FUNC).
        """
        var f = WasmtimeFunc()
        # Union starts at offset 8, func is 16 bytes (2 × UInt64)
        f._w0 = self._w1
        f._w1 = self._w2
        return f

    @always_inline
    fn get_global(self) -> WasmtimeGlobal:
        """Extract the global from the union (valid when kind == WASMTIME_EXTERN_GLOBAL).
        """
        var g = WasmtimeGlobal()
        # Union starts at offset 8, global is 24 bytes (3 × UInt64)
        g._w0 = self._w1
        g._w1 = self._w2
        g._w2 = self._w3
        return g

    @always_inline
    fn get_memory(self) -> WasmtimeMemory:
        """Extract the memory from the union (valid when kind == WASMTIME_EXTERN_MEMORY).
        """
        var m = WasmtimeMemory()
        # Union starts at offset 8, memory is 24 bytes (3 × UInt64)
        m._w0 = self._w1
        m._w1 = self._w2
        m._w2 = self._w3
        return m


# ---------------------------------------------------------------------------
# wasm_byte_vec_t — 16 bytes
#
# C layout:
#   struct wasm_byte_vec_t {
#       size_t size;
#       wasm_byte_t *data;  // uint8_t*
#   };
# ---------------------------------------------------------------------------


struct WasmByteVec(TrivialRegisterPassable):
    """Mirrors wasm_byte_vec_t (16 bytes)."""

    var size: Int
    var data: UnsafePointer[UInt8, MutExternalOrigin]

    @always_inline
    fn __init__(out self):
        self.size = 0
        self.data = UnsafePointer[UInt8, MutExternalOrigin]()


# ---------------------------------------------------------------------------
# wasm_valtype_vec_t — 16 bytes
#
# C layout:
#   struct wasm_valtype_vec_t {
#       size_t size;
#       wasm_valtype_t **data;  // array of pointers
#   };
# ---------------------------------------------------------------------------


struct WasmValtypeVec(TrivialRegisterPassable):
    """Mirrors wasm_valtype_vec_t (16 bytes)."""

    var size: Int
    var data: UnsafePointer[ValTypePtr, MutExternalOrigin]

    @always_inline
    fn __init__(out self):
        self.size = 0
        self.data = UnsafePointer[ValTypePtr, MutExternalOrigin]()


# ---------------------------------------------------------------------------
# Callback type aliases
#
# wasmtime_func_callback_t:
#   wasm_trap_t* (*)(
#       void *env,
#       wasmtime_caller_t *caller,
#       const wasmtime_val_t *args,
#       size_t nargs,
#       wasmtime_val_t *results,
#       size_t nresults
#   );
#
# We represent this as a raw function pointer type. The return value is
# a wasm_trap_t* where NULL means success.
# ---------------------------------------------------------------------------

comptime WasmtimeCallback = fn(
    UnsafePointer[NoneType, MutExternalOrigin],  # env
    UnsafePointer[NoneType, MutExternalOrigin],  # caller
    UnsafePointer[WasmtimeVal, MutExternalOrigin],  # args
    Int,  # nargs
    UnsafePointer[WasmtimeVal, MutExternalOrigin],  # results
    Int,  # nresults
) -> UnsafePointer[NoneType, MutExternalOrigin]

# Finalizer callback: void (*)(void*)
comptime FinalizerCallback = fn(
    UnsafePointer[NoneType, MutExternalOrigin]
) -> None
