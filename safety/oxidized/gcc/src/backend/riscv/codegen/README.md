# RISC-V 64-bit Backend

Code generator targeting **RV64GC** (the general-purpose profile: RV64IMAFDC)
with the **LP64D** calling convention. The backend includes a native assembler
(enabled by default) with RV64C compressed instruction support, producing ELF
object files directly without requiring an external assembler.

---

## Table of Contents

1. [Overview](#overview)
2. [File Inventory](#file-inventory)
3. [Calling Convention (LP64D)](#calling-convention-lp64d)
4. [Hardware Float Classification](#hardware-float-classification)
5. [Register Allocation](#register-allocation)
6. [Stack Frame Layout](#stack-frame-layout)
7. [Addressing Modes](#addressing-modes)
8. [Type Conversions and Casts](#type-conversions-and-casts)
9. [F128 Quad-Precision Handling](#f128-quad-precision-handling)
10. [128-bit Integer Operations](#128-bit-integer-operations)
11. [Atomic Operations](#atomic-operations)
12. [Software SIMD Emulation](#software-simd-emulation)
13. [Software Bit-Manipulation Builtins](#software-bit-manipulation-builtins)
14. [Inline Assembly Support](#inline-assembly-support)
15. [Peephole Optimizer](#peephole-optimizer)
16. [RISC-V Codegen Options](#risc-v-codegen-options)
17. [Key Design Decisions](#key-design-decisions)

---

## Overview

The RISC-V backend follows a **stack-slot-based intermediate strategy** with a
register allocator layered on top. IR values are assigned either to callee-saved
registers or to stack slots indexed by negative offsets from the frame pointer
(`s0`). A dedicated accumulator register `t0` (and `t1` for 128-bit values)
shuttles values between stack slots and instructions. A peephole optimizer runs
over the emitted assembly text to eliminate redundant store/load pairs and
propagate register copies.

The backend implements the `ArchCodegen` trait from the shared backend
framework. Architecture-specific logic (instruction selection, calling
convention details, prologue/epilogue generation) lives in the files below;
common orchestration (IR traversal, basic-block ordering, shared ABI
classification) is reused from `backend/traits.rs` and `backend/call_abi.rs`.

---

## File Inventory

All files live under `src/backend/riscv/codegen/`.

| File | Purpose |
|------|---------|
| `emit.rs` | Core `RiscvCodegen` struct, register constants, operand load/store helpers (`operand_to_t0`, `store_t0_to`), 12-bit immediate check, large-offset helpers, `ArchCodegen` trait implementation, switch/jump-table emission. |
| `prologue.rs` | Stack space calculation, register allocator invocation, prologue/epilogue emission (small-frame, large-frame, stack-probing paths), parameter classification and storage, variadic register save area setup. |
| `calls.rs` | Outgoing call ABI: `CallAbiConfig` for LP64D, stack argument marshalling, GP/FP register argument staging (three-phase strategy using t3/t4/t5), F128 and i128 register-pair argument loading, call instruction emission, result retrieval. |
| `memory.rs` | Load, store, and memcpy operations. Slot addressing (direct s0-relative, indirect through pointer, over-aligned alloca). GEP (get-element-pointer) with constant offsets (variable-offset GEP uses the shared trait default in `traits.rs`). Dynamic stack allocation helpers (`alloca` round-up, sp manipulation). |
| `alu.rs` | Integer arithmetic: binary operations (`add`/`addw`, `sub`/`subw`, `mul`/`mulw`, `div`/`divw`/`divu`/`divuw`, `rem`/`remw`/`remu`/`remuw`, shifts, bitwise), unary negation, bitwise NOT, float negation (`fneg.{s,d}`), i128 copy. Selects 32-bit `*w` instruction variants for I32/U32 types. |
| `float_ops.rs` | Floating-point binary operations (`fadd`, `fsub`, `fmul`, `fdiv`) for F32 and F64, using `fmv.w.x`/`fmv.d.x` (GP-to-FP) and `fmv.x.w`/`fmv.x.d` (FP-to-GP) to move between register files. F128 binary ops dispatch to soft-float libcalls. F128 negation (sign-bit flip). |
| `comparison.rs` | Integer comparisons (via `slt`/`sltu`/`seqz`/`snez`), float comparisons (via `feq`/`flt`/`fle`), fused compare-and-branch (inverted branch + jump), select (`beqz`-based conditional move), F128 comparisons via libcalls. |
| `cast_ops.rs` | Type conversions: integer widening/narrowing via `slli`/`srai`/`srli`/`andi`, float-to-int via `fcvt.l.d`/`fcvt.lu.d`, int-to-float via `fcvt.d.l`/`fcvt.s.l`, float-to-float via `fcvt.d.s`/`fcvt.s.d`, U32-to-I32 sign extension. |
| `i128_ops.rs` | 128-bit integer arithmetic (add with carry, subtract with borrow, multiply, shifts with boundary cases), 128-bit comparisons (high-word-first cascade), div/rem via `__divti3`/`__modti3` libcalls, i128-to-float and float-to-i128 conversions via compiler-rt libcalls. |
| `f128.rs` | `F128SoftFloat` trait implementation for RISC-V. Provides the arch-specific primitives (GP register pair `a0:a1` representation, `fmv.d.x`/`fmv.x.d` move instructions, s0-relative addressing) consumed by the shared F128 orchestration in `backend/f128_softfloat.rs`. |
| `atomics.rs` | Atomic load, store, RMW, and CAS. Word/doubleword atomics via AMO instructions (`amoadd`, `amoswap`, `amoand`, etc.) and LR/SC loops (for NAND). Sub-word (8/16-bit) atomics via word-aligned LR.W/SC.W with bit masking. Software CLZ, CTZ, BSWAP, POPCOUNT. |
| `intrinsics.rs` | Hardware intrinsics (`fsqrt.{s,d}`, `fabs.{s,d}`, `fence`, `fence.tso`), software CRC32C, `__builtin_frame_address`, `__builtin_return_address`, thread pointer (`tp`). Software SSE-equivalent 128-bit SIMD: byte compare, dword compare, unsigned and signed saturating subtract, bitwise OR/AND/XOR, pmovmskb, byte/dword splat. |
| `inline_asm.rs` | Constraint classification (`RvConstraintKind`), operand formatting for `%0`/`%[name]`/`%z` substitution, `%lo`/`%hi` modifier pass-through, template line substitution. |
| `asm_emitter.rs` | `InlineAsmEmitter` trait implementation: scratch register allocation from t0-t6/a2-a7 (GP) and ft0-ft7 (FP), operand loading/storing with register-allocation awareness, memory operand resolution, RISC-V-specific immediate constraint validation (`I` = 12-bit signed, `K` = 5-bit CSR). |
| `globals.rs` | Global symbol address loading: `lla` (PC-relative `auipc`+`addi`) for local symbols, `la` (GOT-indirect `auipc`+`ld`) for PIC externals, TLS Local Exec model (`lui`/`add tp`/`addi` with `%tprel_hi`/`%tprel_add`/`%tprel_lo`). |
| `variadic.rs` | `va_arg` (load next 8-byte arg from va_list pointer, with 16-byte-aligned F128 handling), `va_start` (initialize va_list to point past named register args), `va_copy` (pointer copy). |
| `returns.rs` | Return value marshalling: integer via `a0`, F32 via `fa0` (`fmv.w.x`), F64 via `fa0` (`fmv.d.x`), i128 via `a0:a1`, F128 via `a0:a1` (full 128-bit loaded directly; the shared trait path uses `__extenddftf2`), struct second-field returns via `fa1`. |
| `peephole.rs` | Multi-pass peephole optimizer operating on assembly text lines. |
| `mod.rs` | Module declarations linking all codegen submodules. |

---

## Calling Convention (LP64D)

The backend follows the RISC-V LP64D ABI, which is the standard ABI for
RV64GC hardware with double-precision float support.

### Register Usage

| Registers | Role | Caller/Callee Saved |
|-----------|------|---------------------|
| `a0`-`a7` | Integer argument/return registers | Caller-saved |
| `fa0`-`fa7` | FP argument/return registers | Caller-saved |
| `t0`-`t6` | Temporaries | Caller-saved |
| `ft0`-`ft7` | FP temporaries | Caller-saved |
| `s0` | Frame pointer | Callee-saved |
| `s1`-`s11` | Saved registers | Callee-saved |
| `ra` | Return address | Callee-saved |
| `sp` | Stack pointer | Callee-saved |
| `tp` | Thread pointer | Reserved |
| `gp` | Global pointer | Reserved |
| `zero` | Hardwired zero | N/A |

### Argument Passing Rules

- **Up to 8 integer arguments** in `a0`-`a7`; excess spills to the stack.
- **Up to 8 FP arguments** in `fa0`-`fa7` (non-variadic functions only).
- **Variadic functions** route *all* float arguments through GP registers
  (`a0`-`a7`), matching the psABI requirement. The `variadic_floats_in_gp`
  flag in `CallAbiConfig` controls this.
- **i128** values are passed in aligned GP register pairs (`a0:a1`, `a2:a3`,
  etc.) with the low 64 bits in the even register.
- **F128** (quad-precision `long double`) values are passed in GP register
  pairs, identical to i128 layout. The `f128_in_gp_pairs` flag enables this.
- **Structs <= 16 bytes** are passed by value in up to two GP registers.
  Structs with float fields may use FP registers (see
  [Hardware Float Classification](#hardware-float-classification)).
- **Structs > 16 bytes** are passed by reference: the caller allocates a
  copy and passes a pointer in a GP register.
- **Struct split across register/stack boundary**: the first 8 bytes go in
  the last available GP register; the remainder spills to the stack.
  The `allow_struct_split_reg_stack` flag enables this.

### Return Values

- Scalars in `a0` (integer) or `fa0` (float/double).
- i128 in `a0:a1`.
- F128 in `a0:a1` (GP register pair, full 128-bit precision loaded directly).
- F32 return via `fmv.w.x fa0, t0`; F64 via `fmv.d.x fa0, t0`.
- Struct returns with two float fields may use `fa0`:`fa1`.

### CallAbiConfig Summary

```text
CallAbiConfig {
    max_int_regs: 8,
    max_float_regs: 8,
    align_i128_pairs: true,
    f128_in_fp_regs: false,
    f128_in_gp_pairs: true,
    variadic_floats_in_gp: true,
    large_struct_by_ref: true,
    use_sysv_struct_classification: false,
    use_riscv_float_struct_classification: true,
    allow_struct_split_reg_stack: true,
    align_struct_pairs: true,
}
```

---

## Hardware Float Classification

The RISC-V psABI defines special rules for passing small structs that contain
floating-point fields. The `RiscvFloatClass` enum captures these cases:

```text
RiscvFloatClass
  OneFloat   { is_double }                             -- e.g., struct { float x; }
  TwoFloats  { lo_is_double, hi_is_double }            -- e.g., struct { float x; double y; }
  FloatAndInt { float_is_double, float_offset,          -- e.g., struct { double x; int y; }
                int_offset, int_size }
  IntAndFloat { float_is_double, int_offset,            -- e.g., struct { int x; float y; }
                int_size, float_offset }
```

**Classification rules** (from the psABI):

- A struct with exactly one float or double member and no other data members
  is passed in a single FP register.
- A struct with exactly two float/double members is passed in two FP
  registers.
- A struct with one float/double and one integer (in either order) is passed
  with the float in an FP register and the integer in a GP register.

When FP registers are exhausted, or the struct does not match any of these
patterns, the struct falls back to GP-register or stack passing.

The classification is computed by the frontend and attached to IR parameter
metadata via the `riscv_float_class` field. The backend reads this during
both callee-side parameter storage (`emit_store_params`) and caller-side
argument loading (`emit_call_reg_args`), using `flw`/`fld`/`fsw`/`fsd`
instructions with the correct field offsets derived from the classification.

---

## Register Allocation

The backend runs a graph-coloring register allocator before stack layout
computation, assigning hot IR values to callee-saved registers. This avoids
stack traffic for frequently-used values.

### Available Registers

The allocator draws from two pools:

| Pool | Registers | Count | Notes |
|------|-----------|-------|-------|
| Primary | `s1`, `s7`-`s11` | 6 | Never used by any codegen helper |
| Secondary | `s2`-`s6` | 5 | Freed by three-phase call staging using caller-saved `t3`/`t4`/`t5` |

All **11 callee-saved registers** are unconditionally added to the allocation
pool. The earlier design reserved `s2`-`s6` for call argument staging; the
current three-phase staging strategy (see [Call Argument Staging](#three-phase-call-argument-staging))
freed them for the allocator, significantly reducing stack frame sizes. The
only reason a register may be excluded from the pool is if a function's inline
assembly clobbers it (see [Inline Assembly Interaction](#inline-assembly-interaction)).

### Inline Assembly Interaction

Functions containing inline assembly may clobber callee-saved registers.
The allocator handles this by:

1. Scanning all inline asm blocks for explicit register constraints (e.g.,
   `{s3}`) and clobber lists.
2. Removing clobbered registers from the available pool via
   `filter_available_regs`.
3. Allocating from the remaining registers.

This allows functions with inline asm (common in kernel code for spin locks
and CSR manipulation) to still benefit from register allocation using the
non-clobbered registers, rather than disabling allocation entirely. Without
this, kernel functions with inlined spin_lock/spin_unlock would get no
register allocation and enormous stack frames (4KB+), causing stack overflows.

### Value Selection

Values assigned to registers skip stack slot allocation entirely. The
`operand_to_t0` function first checks `reg_cache` (to skip the load if `t0`
already holds the value), then checks `reg_assignments` and emits a simple
`mv t0, sN` when the value lives in a register, instead of a memory load.
Similarly, `store_t0_to` writes back to the callee-saved register with
`mv sN, t0`. Callee-saved registers used by the allocator are saved in the
prologue and restored in the epilogue at the bottom of the stack frame
(highest negative offsets from `s0`).

**Eligible instructions**: BinOp, UnaryOp, Cmp, Cast, Load, GEP, Copy, Call,
CallIndirect, Select, GlobalAddr, LabelAddr, AtomicLoad, AtomicRmw,
AtomicCmpxchg. Float, i128, and long-double results are excluded.

---

## Stack Frame Layout

### Standard (Non-variadic) Frame

```text
Higher addresses
                    +-----------------------+
                    |  Caller's stack args  |     s0 + 0, s0 + 8, ...
                    +-----------------------+
          s0 --->  |  (frame pointer here)  |
                    +-----------------------+
          s0 - 8   |  saved ra             |
          s0 - 16  |  saved s0 (old FP)    |
                    +-----------------------+
          s0 - 24  |  local variable slots  |     negative offsets from s0
          ...      |  (8-byte aligned)      |
                    +-----------------------+
   s0 - frame_size |  callee-saved regs    |     saved s1, s7-s11, etc.
                    +-----------------------+
          sp --->  |  (16-byte aligned)     |
                    +-----------------------+
Lower addresses
```

### Variadic Frame

For variadic functions, a **64-byte register save area** for `a0`-`a7` is
placed at *positive* offsets from `s0`, contiguous with the caller's stack
arguments. This ensures `va_start` can produce a single pointer that walks
through register-saved args and then seamlessly into stack-passed args:

```text
Higher addresses
                    +-----------------------+
                    |  Caller's stack args  |     s0 + 64, s0 + 72, ...
                    +-----------------------+
          s0 + 56  |  a7 save              |
          s0 + 48  |  a6 save              |
          ...      |  ...                  |
          s0 + 0   |  a0 save              |
                    +-----------------------+
          s0 --->  |  (frame pointer here)  |
                    +-----------------------+
          s0 - 8   |  saved ra             |
          s0 - 16  |  saved s0             |
                    +-----------------------+
          ...      |  local variable slots  |
                    +-----------------------+
Lower addresses
```

The total stack allocation is `frame_size + 64` for variadic functions.
During `va_start`, the va_list pointer is initialized to `s0 + (named_gp_count * 8)`
when named parameters fit in registers, or `s0 + 64 + named_stack_bytes`
when they overflow.

### 16-byte Alignment

All frame sizes are rounded up to 16-byte alignment via
`(raw_space + 15) & !15`. The RISC-V psABI requires the stack pointer to be
16-byte aligned at all times.

### Stack Probing

Frames larger than 4096 bytes use a stack probing loop that touches each page
to ensure the kernel grows the stack mapping. Without probing, a single large
`sub sp, sp, N` can skip guard pages and cause a segfault:

```asm
    li t1, <total_alloc>
    li t2, 4096
.Lstack_probe:
    sub sp, sp, t2
    sd zero, 0(sp)       # touch the page
    sub t1, t1, t2
    bgt t1, t2, .Lstack_probe
    sub sp, sp, t1       # allocate remaining bytes
    sd zero, 0(sp)
```

### Prologue Variants

The prologue has three code paths depending on frame size:

1. **Small frame** (total allocation fits in 12-bit signed immediate, i.e.
   <= 2047): single `addi sp, sp, -N`, then `sd ra`/`sd s0`/`addi s0` all
   using immediate offsets.
2. **Large frame** (2048 through 4096 inclusive): `li t0, N; sub sp, sp, t0`,
   then compute `s0 = sp + frame_size` via `li`/`add`.
3. **Very large frame** (over 4096): stack probing loop followed by the
   large-frame setup.

The epilogue mirrors the prologue. When dynamic alloca is used
(`has_dyn_alloca` flag), the epilogue always restores from s0-relative
offsets rather than sp-relative, since sp may have been modified at runtime.

### Over-aligned Allocas

Allocas with alignment greater than 16 bytes get extra padding in their stack
slot. The address is dynamically aligned at use sites:

```asm
    addi t5, s0, <offset>
    li   t6, <align - 1>
    add  t5, t5, t6
    li   t6, -<align>
    and  t5, t5, t6         # t5 = (s0 + offset + align-1) & -align
```

A variant (`emit_alloca_aligned_addr_to_acc`) computes the result into `t0`
instead of `t5`, for use when the aligned address is needed in the
accumulator.

---

## Addressing Modes

### Global Symbols

- **Non-PIC local symbols**: `lla t0, <symbol>` (expands to `auipc` +
  `addi` with `R_RISCV_PCREL_HI20` relocation).
- **PIC external symbols**: `la t0, <symbol>` (expands to `auipc` + `ld`
  with `R_RISCV_GOT_HI20` relocation, GOT-indirect load).
- **TLS (Local Exec)**: three-instruction sequence using `tp`:

  ```asm
  lui  t0, %tprel_hi(sym)
  add  t0, t0, tp, %tprel_add(sym)
  addi t0, t0, %tprel_lo(sym)
  ```

- **Label addresses**: always use `lla` (labels are local symbols, avoiding
  unnecessary GOT indirection in PIC mode).

### Large Offsets from s0

RISC-V 12-bit signed immediates cover offsets in [-2048, 2047]. For
stack frames exceeding this range, all s0-relative load/store helpers
transparently fall back to a `li t6, <offset>; add t6, s0, t6` sequence:

```asm
# Small offset (fits in 12 bits):
    ld t0, -128(s0)

# Large offset:
    li t6, -8192
    add t6, s0, t6
    ld t0, 0(t6)
```

The same pattern applies to sp-relative addressing used during call setup.
Register `t6` is reserved as the large-offset scratch register throughout
the backend.

### Slot Addressing Modes

Memory operations resolve value addresses through three `SlotAddr` variants:

| Variant | Meaning | Access pattern |
|---------|---------|----------------|
| `Direct(slot)` | Value lives at a known s0-relative offset | `ld/sd t0, offset(s0)` |
| `Indirect(slot)` | Slot holds a pointer; must dereference | `ld t5, offset(s0); ld/sd t0, 0(t5)` |
| `OverAligned(slot, id)` | Alloca with > 16-byte alignment | Dynamic alignment then `ld/sd t0, 0(t5)` |

For loads/stores with constant offsets (e.g., struct field access), the
offset is folded into the slot offset for Direct addressing, or added to
the dereferenced pointer for Indirect and OverAligned.

---

## Type Conversions and Casts

All type conversions are handled in `cast_ops.rs` through a `CastKind` enum
that classifies every source-to-destination type pair.

- **Integer widening**: zero-extend unsigned types with `andi`/`slli`+`srli`;
  sign-extend signed types with `slli`+`srai` or `sext.w` for I32.
- **Integer narrowing**: mask or sign-extend to the target width.
- **Float-to-int**: `fmv.{d,w}.x` to FP register, then `fcvt.l.{d,s}` (signed)
  or `fcvt.lu.{d,s}` (unsigned) with `rtz` (round-toward-zero) rounding mode.
- **Int-to-float**: optional sign-extension, then `fcvt.{d,s}.l` (signed) or
  `fcvt.{d,s}.lu`/`fcvt.{d,s}.wu` (unsigned), then `fmv.x.{d,w}` back to GP.
- **Float-to-float**: `fcvt.d.s` (widen F32 to F64) or `fcvt.s.d` (narrow
  F64 to F32).
- **U32-to-I32**: `sext.w` to ensure proper sign-extension in 64-bit
  registers, matching the ABI's sign-extended I32 convention.
- **F128 casts**: dispatched to the shared F128 soft-float framework
  (`__extenddftf2`, `__trunctfdf2`, etc.).

---

## F128 Quad-Precision Handling

RISC-V LP64D has no hardware quad-precision float instructions. The backend
uses a **dual-representation strategy**:

1. **Full 128-bit representation** in stack slots (16 bytes at the alloca).
2. **Truncated F64 "working copy"** in GP registers and the accumulator,
   sufficient for comparisons and control flow.

### Arithmetic and Comparison

All F128 arithmetic is performed through compiler-rt / libgcc soft-float
library calls:

| Operation | Libcall |
|-----------|---------|
| Add | `__addtf3` |
| Sub | `__subtf3` |
| Mul | `__multf3` |
| Div | `__divtf3` |
| Compare | `__letf2`, `__getf2`, `__eqtf2`, `__netf2` |
| F64-to-F128 | `__extenddftf2` |
| F128-to-F64 | `__trunctfdf2` |
| F128 negation | Sign-bit flip: `xor a1, a1, (1 << 63)` |

Arguments are passed in GP register pairs `a0:a1` (first operand) and
`a2:a3` (second operand). Results are returned in `a0:a1`.

### Parameter Passing

When a function receives F128 parameters alongside other register arguments,
the backend saves all 16 argument registers (`a0`-`a7` and `fa0`-`fa7`) to a
128-byte temporary stack area before calling `__trunctfdf2` for the working
copy. This prevents the conversion libcall from clobbering other not-yet-saved
parameters.

### F128SoftFloat Trait

The `f128.rs` file implements the `F128SoftFloat` trait, providing RISC-V
specific primitives:

- Load/store 128-bit values via GP register pair `a0:a1`
- Move between argument pairs: `a0:a1` to `a2:a3`
- Temporary stack allocation via `addi sp, sp, -16` / `addi sp, sp, 16`
- Sign-bit manipulation (bit 63 of `a1` = bit 127 of IEEE f128)
- Comparison result to boolean via `seqz`/`snez`/`slti`/`slt`

The shared orchestration logic in `backend/f128_softfloat.rs` calls these
primitives to implement all F128 operations uniformly across architectures.

---

## 128-bit Integer Operations

The backend implements 128-bit integers using pairs of 64-bit registers.
The convention is `t0` (low 64 bits) : `t1` (high 64 bits) for the
accumulator pair, and `t3:t4` / `t5:t6` for binary operation operands.

### Inline Arithmetic

| Operation | Implementation |
|-----------|----------------|
| Add | `add` low halves, `sltu` for carry, `add` high halves + carry |
| Sub | `sltu` for borrow, `sub` both halves, subtract borrow from high |
| Mul | `mul` + `mulhu` for low*low, then `mul` for cross products added to high |
| And/Or/Xor | Parallel operations on both halves |
| Neg | `not` both halves, `addi t0, t0, 1`, propagate carry with `seqz`+`add` |
| Not | `not` both halves |
| Shl/Lshr/Ashr | Branch on shift amount vs 64: sub-64 uses double-width shift pattern, >=64 shifts the other half |

### Constant Shifts

Shift amounts known at compile time (masked to 0..127) are expanded to
straight-line code without branches, handling four cases:

- **amount = 0**: simple move.
- **amount = 64**: move one half to the other, zero/sign-extend.
- **amount > 64** (65..127): shift the single relevant half by `amount - 64`.
- **amount 1..63**: `slli`/`srli`/`srai` with complementary shift and
  `or` to combine bits crossing the 64-bit boundary.

### Division and Conversion Libcalls

| Operation | Libcall |
|-----------|---------|
| Signed div | `__divti3` |
| Signed rem | `__modti3` |
| Unsigned div | `__udivti3` |
| Unsigned rem | `__umodti3` |
| i128 to F64 (signed) | `__floattidf` |
| i128 to F32 (signed) | `__floattisf` |
| i128 to F64 (unsigned) | `__floatuntidf` |
| i128 to F32 (unsigned) | `__floatuntisf` |
| F64 to i128 (signed) | `__fixdfti` |
| F32 to i128 (signed) | `__fixsfti` |
| F64 to i128 (unsigned) | `__fixunsdfti` |
| F32 to i128 (unsigned) | `__fixunssfti` |

All libcalls pass both halves in `a0:a1` and `a2:a3`, with results returned
in `a0:a1`.

### Comparisons

- **Equality** (`eq`/`ne`): XOR both halves, OR the results, then
  `seqz`/`snez`.
- **Ordered comparisons**: high-word-first cascade. Compare high halves with
  `slt`/`sltu`; if equal, compare low halves with unsigned `sltu` (since the
  low half represents unsigned magnitude). For `<=` and `>=`, the comparison
  is inverted via `xori t0, t0, 1`.

---

## Atomic Operations

### Word and Doubleword Atomics

For 32-bit and 64-bit types, the backend uses RISC-V AMO (Atomic Memory
Operation) instructions directly:

| Operation | Instruction |
|-----------|-------------|
| Add | `amoadd.{w,d}` (Sub negates the value first) |
| And | `amoand.{w,d}` |
| Or | `amoor.{w,d}` |
| Xor | `amoxor.{w,d}` |
| Exchange | `amoswap.{w,d}` |
| TestAndSet | `amoswap.{w,d}` with value 1 |
| Nand | LR/SC loop (`lr.{w,d}`, `and`, `not`, `sc.{w,d}`) |

Ordering suffixes map to: `.aq` (acquire), `.rl` (release), `.aqrl`
(acquire-release and sequential consistency). Relaxed ordering uses no suffix.

### Compare-and-swap (CAS)

Word/doubleword CAS uses an LR/SC loop: load-reserved the current value,
compare against expected, store-conditional the desired value if they match.
On failure (value mismatch or SC retry), the old value is returned. A boolean
variant returns 1 on success and 0 on failure.

### Sub-word Atomics (8-bit and 16-bit)

RISC-V provides no byte or halfword atomic instructions. The backend
synthesizes them using word-aligned `lr.w`/`sc.w` loops with bit masking:

```text
Inputs: t1 = byte/halfword address, t2 = value

a2 = t1 & ~3          # word-aligned address
a3 = (t1 & 3) << 3    # bit shift within word
a4 = mask << a3        # e.g., 0xFF shifted to byte position
a5 = ~a4               # inverted mask

.Lloop:
    lr.w t0, (a2)      # load-reserved the containing word
    <compute new word>  # operation-specific (see below)
    sc.w t5, t4, (a2)  # store-conditional (t5 != t4 per spec)
    bnez t5, .Lloop     # retry on SC failure

    srlw t0, t0, a3     # extract old sub-word value
    andi t0, t0, 0xff   # mask to field width
```

Each RMW operation has operation-specific "compute new word" logic:

- **Xchg**: `(old & ~mask) | (new_val & mask)`
- **Add/Sub**: extract field, add/subtract, mask back, reinsert
- **And**: `old & (val_shifted | ~mask)`
- **Or**: `old | (val_shifted & mask)`
- **Xor**: `old ^ (val_shifted & mask)`
- **Nand**: extract, AND, NOT, mask, reinsert

Register `t5` is used as the SC destination register, which must differ from
the SC source register (`t4`) per the RISC-V specification.

### Fences

| Intrinsic | Instruction | Purpose |
|-----------|-------------|---------|
| `lfence`, `mfence` | `fence iorw, iorw` | Full memory barrier |
| `sfence` | `fence ow, ow` | Store barrier |
| `pause` | `fence.tso` | Spin-wait hint |
| `clflush` | `fence iorw, iorw` | Best-effort approximation (no direct equivalent) |

Additionally, `atomics.rs` provides ordering-based fences for atomic
operations (distinct from the x86-named intrinsics above):

| Ordering | Instruction |
|----------|-------------|
| Relaxed | (none) |
| Acquire | `fence r, rw` |
| Release | `fence rw, w` |
| AcqRel / SeqCst | `fence rw, rw` |

---

## Software SIMD Emulation

The backend provides software emulation of SSE-equivalent 128-bit SIMD
intrinsics using scalar RISC-V instructions. These are used when C code
includes `<emmintrin.h>` or similar headers in cross-compilation scenarios.
All 128-bit vectors are represented as pairs of 64-bit values in memory.

### Implemented Operations

| SSE Intrinsic | Implementation |
|---------------|----------------|
| `_mm_or_si128` / `_mm_and_si128` / `_mm_xor_si128` | Load two 64-bit halves, apply `or`/`and`/`xor`, store back. |
| `_mm_cmpeq_epi8` | XOR corresponding halves, detect zero bytes using SWAR formula `(x - 0x0101...) & ~x & 0x8080...`, expand 0x80 to 0xFF via shift-or cascade. |
| `_mm_cmpeq_epi32` | Extract and compare each 32-bit lane independently using `sub`/`snez`/`neg`/`not`, producing 0xFFFFFFFF (equal) or 0x00000000 (not equal). |
| `_mm_subs_epu8` | Per-byte unsigned saturating subtract: extract each byte with `srli`/`andi`, compare with `bltu`, subtract, accumulate with `slli`/`or`. |
| `_mm_subs_epi8` | Per-byte signed saturating subtract: extract each byte with sign-extension, subtract, clamp to [-128, 127], mask to 8 bits, accumulate. |
| `_mm_movemask_epi8` | Extract bit 7 of each of 16 bytes into a 16-bit integer mask using shift-and-OR accumulation. |
| `_mm_set1_epi8` | Byte splat: `val & 0xFF` multiplied by `0x0101010101010101`, stored to both halves. |
| `_mm_set1_epi32` | Dword splat: `(val << 32) | val`, stored to both halves. |
| `_mm_loadu_si128` / `_mm_storeu_si128` | Two `ld`/`sd` pairs (unaligned is handled identically to aligned on RISC-V). |
| Non-temporal stores (`movnti`, `movntdq`, `movntpd`) | Plain stores (no non-temporal hint available on RISC-V). |

### Unimplemented x86-only Intrinsics

AES-NI, PCLMUL, and many SSE2/SSE4 lane-manipulation intrinsics (shuffles,
pack/unpack, multiply-add, etc.) emit zeroed output when encountered. These
are expected to be dead-code eliminated in cross-compiled code behind
`#ifdef __x86_64__` guards.

---

## Software Bit-Manipulation Builtins

The backend provides software implementations of GCC/Clang builtins that
have no corresponding RISC-V instructions in the base ISA (absent the Zbb
extension). These implementations live in `atomics.rs` alongside the atomic
operations, while CRC32C lives in `intrinsics.rs`:

### CLZ (Count Leading Zeros)

Scans from the MSB using a mask (`1 << (bits-1)`) that shifts right each
iteration. Returns the bit width for zero input. Operates on 32-bit (with
zero-extension) or 64-bit values.

### CTZ (Count Trailing Zeros)

Checks bit 0 via `andi t3, t0, 1` and shifts right each iteration until a
set bit is found or all bits are checked.

### BSWAP (Byte Swap)

Type-specific straight-line code:

- **16-bit**: extract and swap 2 bytes with `andi`/`slli`/`srli`/`or`.
- **32-bit**: extract 4 bytes with shift/mask, reassemble in reversed order,
  zero-extend to clear upper 32 bits.
- **64-bit**: unrolled 8-iteration sequence extracting each byte with
  `srli`/`andi` and placing it at the mirrored position with `slli`/`or`.

### POPCOUNT (Population Count)

Brian Kernighan's algorithm: repeatedly clear the lowest set bit with
`n & (n - 1)` and count iterations. 32-bit values are zero-extended first.

### CRC32C

Bit-by-bit CRC32C (Castagnoli polynomial `0x82F63B78`) computation. Processes
8, 16, 32, or 64 bits depending on the variant. Implemented as a loop that
checks the LSB, shifts right, and conditionally XORs with the polynomial.
The result is zero-extended to 32 bits.

---

## Inline Assembly Support

### Constraint Types

| Constraint | Kind | Description |
|------------|------|-------------|
| `r` | `GpReg` | Any general-purpose register |
| `f` | `FpReg` | Any floating-point register |
| `m` | `Memory` | Memory operand, formatted as `offset(s0)` or `0(reg)` |
| `A` | `Address` | Address for AMO/LR/SC, formatted as `(reg)` |
| `I` | `Immediate` | 12-bit signed immediate (-2048..2047) |
| `i`, `n` | `Immediate` | Any integer constant |
| `K` | `GpReg` | 5-bit unsigned CSR immediate (0..31); classified as GpReg in `classify_rv_constraint`, with the 5-bit range check in `constant_fits_immediate` |
| `J`, `rJ` | `ZeroOrReg` | `zero` register if value is 0, else GP register |
| `{s3}`, `a0`, etc. | `Specific` | Named register (brace syntax `{reg}` is parsed in `asm_emitter.rs`; bare register names like `a0` are handled in `inline_asm.rs`) |
| `0`, `1`, etc. | `Tied` | Tied to output operand N |

### Template Substitution

The template engine handles:

- **`%0`, `%1`**: positional operand substitution (GCC numbering, mapped
  through a `gcc_to_internal` index table).
- **`%[name]`**: named operand substitution.
- **`%z0`, `%z[name]`**: zero-register modifier -- emits `zero` if the
  operand value is 0, otherwise emits the register name.
- **`%lo(...)`, `%hi(...)`**: passed through verbatim to the assembler
  (RISC-V assembler directives for address splitting).
- **`%%`**: literal percent sign.
- **`%l[name]`**: goto label reference (for `asm goto`).

### Scratch Register Allocation

Input/output operands that need a register are assigned from a scratch pool:

- **GP**: `t0`, `t1`, `t2`, `t3`, `t4`, `t5`, `t6`, `a2`, `a3`, `a4`, `a5`,
  `a6`, `a7`, `a0`, `a1` (15 registers). The `a0`/`a1` entries are placed
  last as a fallback for inline asm blocks with many operands (e.g., 8
  outputs + 8 inputs) that would exhaust the first 13 entries.
- **FP**: `ft0`-`ft7` (8 registers).

Each operand gets a unique register. Excluded registers (from clobber lists
or explicit constraints) are skipped. For memory operands with large
s0-relative offsets (outside the 12-bit signed immediate range), the address
is pre-computed into a scratch register with the emitted `offset(s0)`
replaced by `0(reg)`.

### Register-Allocation Awareness

The inline asm emitter integrates with the register allocator:

- When loading inputs, it checks `reg_assignments` first and uses
  `mv reg, sN` from the callee-saved register rather than loading from
  the stack. This is critical for correctness when inline asm changes page
  tables (e.g., `csrw satp`) -- any stack access between CSR writes would
  fault because the new page table may not map the stack.
- When storing outputs, it writes through the callee-saved register's
  pointer if the value is register-allocated.

---

## Peephole Optimizer

The peephole optimizer operates on the final assembly text, applying pattern
matching to eliminate redundant instructions generated by the stack-based
codegen model.

### Pass Structure

1. **Local passes** (iterative, up to 8 rounds): applied within basic blocks
   (between labels/branches).
2. **Global passes** (single pass): cross-basic-block optimizations.
3. **Local cleanup** (up to 4 rounds): re-runs local passes to clean up
   opportunities exposed by global passes.

### Line Classification

Each assembly line is pre-parsed into a `LineKind` enum for efficient
pattern matching using integer/enum comparisons instead of string parsing:

```text
LineKind
  Nop                                    -- deleted / blank
  StoreS0 { reg, offset, is_word }       -- sd/sw reg, offset(s0)
  LoadS0  { reg, offset, is_word }       -- ld/lw reg, offset(s0)
  Move    { dst, src }                   -- mv rdst, rsrc
  LoadImm { dst }                        -- li rdst, imm
  Jump                                   -- jump / j
  Branch                                 -- beq, bne, bgeu, etc.
  Label                                  -- .LBBx: or func_name:
  Ret                                    -- ret
  Call                                   -- call / jal ra
  Directive                              -- lines starting with .
  Alu                                    -- add, sub, and, or, etc.
  SextW   { dst, src }                   -- sext.w
  LoadAddr { dst }                       -- lla / la (protected from copy propagation)
  Other                                  -- any other instruction
```

### Optimization Patterns

| # | Pattern | Description |
|---|---------|-------------|
| 1 | **Adjacent store/load elimination** | `sd t0, off(s0)` followed by `ld t0, off(s0)` at the same offset -- the load is redundant and deleted. |
| 2 | **Redundant jump elimination** | `jump .LBBN, t6` (or `j .LBBN`) when `.LBBN:` is the immediately following non-empty line -- the jump falls through naturally. |
| 3 | **Self-move elimination** | `mv tX, tX` is a no-op and can be deleted. |
| 4 | **Move chain optimization** | `mv A, B; mv C, A` is rewritten to `mv C, B`, potentially making the first move dead. |
| 5 | **Li-to-move chain** | `li rX, imm; mv rY, rX` is collapsed to `li rY, imm` when rX is a temp register and the move is the immediate next instruction. |
| 6 | **Global store forwarding** | Tracks slot-to-register mappings within basic blocks. When a load reads a slot whose value is known to be in a register, the load is replaced with a register move (or eliminated if same register). Mappings are invalidated at calls, labels, and instructions that clobber the tracked register. |
| 7 | **Register copy propagation** | After store forwarding creates moves, propagates `mv dst, src` into the next instruction's source operand fields. `lla`/`la` instructions are protected from propagation since symbol names may contain register-like substrings (e.g., `main.s1.0`). |
| 8 | **Global dead store elimination** | Removes stores to stack slots that are never loaded anywhere in the function. Scans all lines for load instructions and builds a set of "live" offsets; stores to other offsets are deleted. |
| 9 | **Dead register move elimination** | Removes `mv sN, t0` moves where the destination callee-saved register is never read before being overwritten. Applied in global and cleanup passes. |

---

## RISC-V Codegen Options

### `-mno-relax`

When enabled (via `set_no_relax(true)` or the `--mno-relax` CLI flag), emits
`.option norelax` at the beginning of the assembly output. This prevents the
linker from performing relaxation optimizations (e.g., converting `auipc` +
`addi` pairs to shorter sequences). This is necessary when linking with
linkers that do not support RISC-V relaxation, or when precise code layout
is required.

### `-fPIC` / `-fpie`

Enables position-independent code generation. In PIC mode, external global
symbols are accessed through the GOT (`la` pseudo-instruction with
`R_RISCV_GOT_HI20` relocation) instead of direct PC-relative addressing.
Local symbols continue to use `lla` regardless of PIC mode.

### `-fno-jump-tables`

Disables jump table emission for switch statements, forcing all cases to use
linear compare-and-branch chains. Jump tables normally use `.word` entries
with PC-relative offsets stored in a `.rodata` section.

---

## Key Design Decisions

### Accumulator-based Codegen Model

The backend uses `t0` as a universal accumulator: every value passes through
`t0` on its way between stack slots, registers, and instruction operands. This
keeps the instruction selection simple and uniform at the cost of extra moves,
which the peephole optimizer cleans up. For 128-bit values, the pair `t0:t1`
serves as the accumulator.

A lightweight `reg_cache` tracks the current contents of `t0` to avoid
redundant reloads when the same value is used consecutively.

### Three-phase Call Argument Staging

Outgoing call arguments are loaded in three phases to avoid conflicts between
source reads and destination writes, while keeping `s2`-`s6` free for the
register allocator:

1. **Phase 1**: Load the first 3 GP arguments into caller-saved staging
   registers `t3`, `t4`, `t5`. Float arguments are loaded directly into
   `fa0`-`fa7` (they use separate register files, so no conflict).
2. **Phase 2**: Move staged values from `t3`/`t4`/`t5` to their target
   `a`-registers.
3. **Phase 3**: Load remaining GP arguments (4th and beyond) directly into
   their target `a`-registers. This is safe because `operand_to_t0` reads
   only from s0-relative stack slots or callee-saved registers, never from
   `a`-registers.

This design eliminated the earlier approach of reserving `s2`-`s6` for call
staging, recovering 5 additional callee-saved registers for allocation.

### t6 as Large-offset Scratch

Register `t6` is reserved as the scratch register for all large-offset
address computations (load/store helpers, sp adjustment). This convention
is consistent across the entire backend, preventing accidental clobbering.

### Float Values in GP Registers

The backend carries float values in their IEEE 754 bit representation within
GP registers (`t0`, callee-saved `sN` registers, stack slots). Float
operations begin with `fmv.{w,d}.x` to transfer bits to an FP register,
perform the operation, and `fmv.x.{w,d}` to transfer back. This avoids
needing separate FP register allocation or FP stack slot management, at the
cost of the transfer instructions (which are single-cycle on most
implementations).

### Unified Sub-word Extension Strategy

Sub-word integer types (I8, U8, I16, U16) are sign- or zero-extended in GP
registers using shift pairs (`slli`/`srai` for signed, `slli`/`srli` or
`andi` for unsigned). This is preferred over the Zbb `sext.b`/`zext.h`
instructions for baseline RV64GC compatibility. The `sext.w` pseudo-instruction
(which is `addiw rd, rs, 0`) is used for I32 sign-extension as it is part of
the base ISA.

### 32-bit Instruction Variants

For I32/U32 types, the backend selects the `*w` instruction variants
(`addw`, `subw`, `mulw`, `divw`, `divuw`, `remw`, `remuw`, `sllw`, `srlw`,
`sraw`) that operate on the lower 32 bits and sign-extend the result to 64
bits. This matches the RISC-V convention that I32 values are always held
sign-extended in 64-bit registers. Bitwise operations (`and`, `or`, `xor`)
have no `*w` variants in the ISA and operate on the full 64 bits.

### Jump Tables with PC-relative Offsets

Switch statements with dense case ranges use jump tables stored in `.rodata`.
Each entry is a 32-bit signed offset from the table base (`table_label`). The
dispatch sequence:

```asm
    lla t1, <table_label>     # load table base address
    slli t0, t0, 2            # index * 4 (each entry is .word)
    add t1, t1, t0            # entry address
    lw t0, 0(t1)              # load PC-relative offset
    lla t1, <table_label>     # reload base
    add t1, t1, t0            # compute target address
    jr t1                     # jump
```

This produces position-independent tables without requiring relocations per
entry.

### Dynamic Stack Allocation (DynAlloca)

When a function uses `alloca` with a runtime size, the stack pointer is
modified dynamically. The epilogue detects this (`has_dyn_alloca` flag) and
restores `sp` from `s0` (frame pointer) rather than using sp-relative
offsets, ensuring correct cleanup regardless of how much dynamic stack was
allocated.
