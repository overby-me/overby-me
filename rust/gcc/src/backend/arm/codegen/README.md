# AArch64 (ARM64) Backend

Code generation targeting the AArch64 architecture with the AAPCS64 (Arm Architecture
Procedure Call Standard for 64-bit) calling convention. The backend translates the
compiler's intermediate representation into AArch64 assembly text. A post-codegen
peephole optimizer cleans up redundant patterns inherent to the stack-based code
generation strategy. The backend includes a builtin assembler and linker
(enabled by default) supporting both static and dynamic linking with
IFUNC/IPLT and TLS support, producing ELF executables and shared
libraries directly without requiring an external toolchain.

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [File Inventory](#file-inventory)
3. [Calling Convention (AAPCS64)](#calling-convention-aapcs64)
4. [Register Allocation](#register-allocation)
5. [Stack Frame Layout](#stack-frame-layout)
6. [Addressing Modes](#addressing-modes)
7. [F128 Quad-Precision Handling](#f128-quad-precision-handling)
8. [128-bit Integer Operations](#128-bit-integer-operations)
9. [Atomic Operations](#atomic-operations)
10. [NEON/SIMD Intrinsics](#neonsimd-intrinsics)
11. [Inline Assembly Support](#inline-assembly-support)
12. [Peephole Optimizer](#peephole-optimizer)
13. [Codegen Options](#codegen-options)
14. [Key Design Decisions](#key-design-decisions)

---

## Architecture Overview

The AArch64 backend is structured as an implementation of the `ArchCodegen` trait,
which defines the interface between the shared codegen framework and architecture-specific
emission. The central type is `ArmCodegen`, which carries all per-function state:
the current frame size, register assignments, variadic parameter metadata, and a
reference to the shared `CodegenState` that accumulates emitted assembly lines.

The codegen follows a **stack-based accumulator model**: most values pass through the
`x0` register (the "accumulator") and are spilled to stack slots as needed. A
lightweight register allocator assigns frequently-used values to callee-saved physical
registers (`x20`-`x28`) and a small set of caller-saved registers (`x13`, `x14`) to
reduce stack traffic. The peephole optimizer runs as a final pass over the emitted
assembly text to eliminate redundant store/load pairs, dead moves, and other patterns
that the accumulator model produces.

---

## File Inventory

All codegen source files reside under `src/backend/arm/codegen/`.

| File | Responsibility |
|------|---------------|
| `emit.rs` | Core `ArmCodegen` struct, register pool definitions (`ARM_CALLEE_SAVED`, `ARM_CALLER_SAVED`), ALU mnemonic mapping, condition code tables, immediate encoding helpers (`const_as_imm12`, `const_as_power_of_2`), prescan of inline asm for callee-saved register discovery, and integer comparison emission with optimized reg-vs-imm12 and reg-vs-reg paths. |
| `mod.rs` | Module declarations for the codegen submodules. |
| `prologue.rs` | Stack space calculation, prologue/epilogue emission (frame pointer/link register pair save, callee-saved register save/restore via `STP`/`LDP`), parameter store dispatch, and variadic register save area layout. |
| `calls.rs` | Call ABI configuration (`CallAbiConfig` with 8 integer regs, 8 float regs, I128 pair alignment), stack argument marshalling (scalars, I128, F128, structs by value), GP-to-temp staging, FP register argument loading, indirect call via `BLR x17`, and post-call stack cleanup. |
| `memory.rs` | Load/store emission with F128 specialization, typed slot access, pointer indirection, GEP (get-element-pointer) address computation, `memcpy` helpers, dynamic alloca support (`sub sp` / `mov sp`), and alignment rounding. |
| `alu.rs` | Integer ALU operations: binary ops (add, sub, mul, div, rem, and, or, xor, shifts) with strength-reduction for power-of-2 division/remainder, register-direct fast paths for callee-saved operands, and unary ops (neg, not, clz, ctz, bswap, popcount via NEON `CNT`/`UADDLV`). |
| `comparison.rs` | Integer comparison (`CMP` + `CSET`), floating-point comparison (`FCMP` + `CSET`), F128 comparison via soft-float libcalls, fused compare-and-branch emission, and `CSEL`-based select. |
| `float_ops.rs` | Floating-point binary operations (`FADD`, `FSUB`, `FMUL`, `FDIV`) for F32/F64 with `FMOV` transfers between GP and FP registers, and F128 negation. |
| `cast_ops.rs` | Type casts: float-to-int (`FCVTZS`/`FCVTZU`), int-to-float (`SCVTF`/`UCVTF`), float-to-float (`FCVT`), sign/zero extension, and truncation with appropriate `SXTB`/`SXTH`/`SXTW`/`AND` masking. |
| `f128.rs` | IEEE 754 binary128 (quad-precision) support implementing the `F128SoftFloat` trait. Provides AArch64-specific primitives for Q-register loads/stores, constant materialization into `v0`, and calls to compiler-rt/libgcc soft-float routines. |
| `i128_ops.rs` | 128-bit integer arithmetic: add/sub via `ADDS`/`ADC`/`SUBS`/`SBC`, multiplication via `MUL`/`UMULH`/`MADD`, bitwise ops, shifts (with >64 and ==64 special cases), division/remainder via `__divti3`/`__modti3` libcalls, float-to-i128 and i128-to-float conversions, and comparisons. |
| `atomics.rs` | Atomic operations: `LDXR`/`STXR` exclusive load/store loops for RMW (exchange, add, sub, and, or, xor, nand, test-and-set), compare-and-exchange with `CLREX` on failure, atomic loads (`LDAR`/`LDARB`/`LDARH`), atomic stores (`STLR`/`STLRB`/`STLRH`), and fences (`DMB ISH`/`ISHLD`/`ISHST`). |
| `intrinsics.rs` | NEON/SIMD intrinsic emission (SSE-equivalent operations), hardware builtins (CRC32, `fsqrt`, `fabs`), memory barriers (`DMB`), non-temporal stores, cache maintenance (`DC CIVAC`), `__builtin_frame_address`, `__builtin_return_address`, and `__builtin_thread_pointer`. |
| `globals.rs` | Global symbol addressing: `ADRP`+`ADD :lo12:` for direct access (used for all regular symbols, including in PIC/PIE mode), `ADRP`+`LDR :got:` for GOT-indirect access (weak extern symbols only), and TLS access via `MRS TPIDR_EL0` + `tprel_hi12`/`tprel_lo12_nc`. |
| `returns.rs` | Return value handling: integer returns in `x0`, F32 in `s0`, F64 in `d0`, F128 in `q0` (with `__extenddftf2` promotion), I128 in `x0:x1`, struct return via `x8` (sret pointer), and second-slot float returns in `d1`/`s1`. |
| `variadic.rs` | Variadic function support: `va_arg` implementation (GP register save area at offset 24, FP register save area at offset 28), `va_start` initialization of the `va_list` struct (stack pointer, `gr_top`, `vr_top`, `gr_offs`, `vr_offs`), and `va_copy`. |
| `inline_asm.rs` | Inline assembly template substitution: `%0`/`%[name]` positional and named operand references, `%w`/`%x`/`%s`/`%d`/`%q`/`%c`/`%a` register modifiers, GCC `r0`-`r30` alias normalization, and `%%` literal percent. |
| `asm_emitter.rs` | `InlineAsmEmitter` trait implementation: constraint classification (`r`=GP, `w`=FP, `m`/`Q`=memory, `i`/`n`=immediate, `{reg}`=specific), scratch register allocation from `ARM_GP_SCRATCH` and `ARM_FP_SCRATCH` pools, operand loading/storing, memory operand resolution, AArch64 logical immediate validation (`K`/`L` constraints), and output register writeback. |
| `peephole.rs` | Multi-phase post-codegen peephole optimizer operating on assembly text lines. |

---

## Calling Convention (AAPCS64)

The backend implements the standard AAPCS64 calling convention:

### Parameter Passing

| Category | Registers | Notes |
|----------|-----------|-------|
| Integer arguments | `x0`-`x7` | Up to 8 GP registers |
| Floating-point arguments | `d0`-`d7` (or `s0`-`s7` for F32) | Up to 8 FP/SIMD registers |
| F128 arguments | `q0`-`q7` | Passed in NEON Q registers |
| I128 arguments | Aligned register pair (e.g., `x0:x1`, `x2:x3`) | Must start on even-numbered register |
| Indirect result (sret) | `x8` | Pointer to caller-allocated return buffer |
| Stack arguments | `[sp, #0]`, `[sp, #8]`, ... | 8-byte aligned, 16-byte for I128/F128 |

Because sret uses the dedicated `x8` register (not `x0`), it does not consume a GP
argument slot. The initial classification assigns sret to IntReg(0) like other targets,
but both caller (`emit_call` in `traits.rs`) and callee (`classify_params_full` in
`call_abi.rs`) then shift GP indices down by 1 and promote the first stack-overflow GP
argument to `x7`. The callee uses `sret_shift=1` in `emit_store_gp_params` to map the
promoted reg_idx back to physical registers.

### Return Values

| Type | Location |
|------|----------|
| Integer (up to 64-bit) | `x0` |
| I128 | `x0` (low), `x1` (high) |
| F32 | `s0` |
| F64 | `d0` |
| F128 | `q0` |
| Struct (small) | `x0`/`x0:x1` |
| Struct (large, >16 bytes) | Via `x8` sret pointer |

### Special Registers

| Register | Role |
|----------|------|
| `x29` | Frame pointer (FP) |
| `x30` | Link register (LR, return address) |
| `x8` | Indirect result location (sret) |
| `x9` | Primary address/scratch register |
| `x10` | memcpy source register, secondary scratch |
| `x11` | memcpy loop counter |
| `x12` | memcpy byte transfer |
| `x15` | F128 large-offset scratch |
| `x16`/`x17` | Intra-procedure-call scratch (IP0/IP1); `x17` used for indirect calls (`BLR x17`) |
| `x18` | Platform-reserved (not used) |
| `sp` | Stack pointer (must remain 16-byte aligned) |

### Variadic Functions

For variadic (varargs) functions, the prologue saves the remaining argument registers
to dedicated save areas on the stack:

- **GP save area**: `x0`-`x7` saved to a 64-byte region (8 registers x 8 bytes)
- **FP save area**: `q0`-`q7` saved to a 128-byte region (8 registers x 16 bytes)

The `va_list` struct is initialized by `va_start` with five fields:

```text
struct va_list {
    __stack: *void,       // offset 0:  pointer to next stack argument
    __gr_top: *void,      // offset 8:  top of GP register save area
    __vr_top: *void,      // offset 16: top of FP register save area
    __gr_offs: i32,       // offset 24: negative offset from gr_top to next GP reg
    __vr_offs: i32,       // offset 28: negative offset from vr_top to next FP reg
}
```

Named (non-variadic) parameters that consume GP or FP registers are accounted for by
adjusting `__gr_offs` and `__vr_offs` so that `va_arg` skips them. When
`-mgeneral-regs-only` is active, the FP save area is skipped entirely and `__vr_offs`
is set to zero.

---

## Register Allocation

The backend uses a lightweight register allocator that runs before code emission. It
assigns IR values to physical registers to reduce stack traffic. Values that remain
unassigned use the accumulator (`x0`) with stack spill/reload.

### Callee-Saved Pool

Nine callee-saved registers are available for allocation:

```text
x20, x21, x22, x23, x24, x25, x26, x27, x28
```

These survive across function calls, so values assigned to them do not need spilling
around call sites. `x19` is excluded (reserved by some ABIs). `x29` is the frame
pointer and `x30` is the link register.

### Caller-Saved Pool

Two caller-saved registers are available for allocation:

```text
x13, x14
```

These are a subset of the AAPCS64 "corruptible" registers (`x9`-`x15`). The remaining
corruptible registers are excluded because they have dedicated scratch uses in the
codegen:

| Register | Hardcoded Use |
|----------|---------------|
| `x9` | Primary address register |
| `x10` | memcpy source, secondary scratch |
| `x11` | memcpy loop counter |
| `x12` | memcpy byte transfer |
| `x15` | F128 large-offset scratch |

Caller-saved allocation assigns values whose live ranges do **not** span any function
call. Additionally, functions containing inline assembly have the caller-saved pool
disabled entirely (inline asm uses `x13`/`x14` as part of `ARM_GP_SCRATCH`).

### F128 Interaction

When a function contains F128 (quad-precision) operations, the caller-saved register
pool is cleared. This is because F128 arithmetic requires soft-float library calls
(e.g., `__addtf3`), which clobber caller-saved registers unpredictably across the
F128 operation's live range.

### Callee-Saved Register Save/Restore

Used callee-saved registers are saved in the prologue and restored in the epilogue.
Pairs of registers are saved/restored with `STP`/`LDP` instructions for efficiency;
an odd register uses a single `STR`/`LDR`.

---

## Stack Frame Layout

Every function prologue establishes a standard AArch64 stack frame. The stack pointer
must remain 16-byte aligned at all times.

### Prologue Sequence

The prologue uses one of three code paths depending on frame size:

**Small frame (≤504 bytes):**

```asm
    stp x29, x30, [sp, #-FRAME_SIZE]!   // save FP and LR, allocate frame
    mov x29, sp                           // establish frame pointer
```

**Medium frame (505–4096 bytes):**

```asm
    sub sp, sp, #FRAME_SIZE              // allocate frame (via emit_sub_sp)
    stp x29, x30, [sp]                   // save FP and LR at bottom of frame
    mov x29, sp                           // establish frame pointer
```

**Large frame (>4096 bytes) — with stack probing:**

```asm
    mov x17, #FRAME_SIZE                 // materialize frame size
.Lstack_probe_N:
    sub sp, sp, #4096                    // step down one page
    str xzr, [sp]                        // probe (touch page to grow stack)
    sub x17, x17, #4096
    cmp x17, #4096
    b.hi .Lstack_probe_N                 // repeat for remaining pages
    sub sp, sp, x17                      // allocate residual bytes
    str xzr, [sp]                        // probe final page
    stp x29, x30, [sp]                   // save FP and LR
    mov x29, sp                           // establish frame pointer
```

Stack probing ensures the kernel can grow the stack mapping page-by-page. Without
probing, a single large `sub sp` can skip guard pages and cause a segfault.

For variadic functions, the prologue additionally saves `x0`-`x7` (and optionally
`q0`-`q7` unless `-mgeneral-regs-only`) to the register save areas.

### Frame Organization

```text
High addresses (caller's frame)
  +----------------------------------+
  | Caller's stack arguments         |  [x29 + frame_size + ...]
  +----------------------------------+  <-- previous sp (before prologue)
  | Callee-saved register save area  |  [sp + callee_save_offset]
  |   (x20-x28 as needed, via STP)   |
  +----------------------------------+
  | Variadic FP save area (128 bytes)|  [sp + va_fp_save_offset]  (if variadic)
  | Variadic GP save area (64 bytes) |  [sp + va_gp_save_offset]  (if variadic)
  +----------------------------------+
  | Local variables / spill slots    |  [sp + 16...]
  |   (8-byte minimum alignment,     |
  |    respecting alloca alignment)   |
  +----------------------------------+
  | Saved x30 (LR)                   |  [sp + 8] = [x29 + 8]
  | Saved x29 (FP)                   |  [sp + 0] = [x29]
  +----------------------------------+  <-- sp = x29 (frame pointer)
Low addresses
```

### Frame Size Calculation

The raw frame size is computed by summing:

1. Space for all local variables and alloca slots (8-byte granularity, respecting
   alignment requirements up to the alloca's specified alignment)
2. Variadic register save areas (64 bytes GP + 128 bytes FP, if applicable)
3. Callee-saved register save slots (8 bytes per register)

The total is then rounded up to a 16-byte boundary (`(raw + 15) & !15`) to maintain
the AArch64 stack alignment requirement.

---

## Addressing Modes

### Global Symbols

The backend supports three addressing modes for global symbols:

**Direct (PC-relative, used for all regular symbols including PIC/PIE):**

```asm
    adrp x0, symbol           // load 4KB-aligned page address
    add  x0, x0, :lo12:symbol // add page offset
```

**GOT-indirect (weak extern symbols only):**

```asm
    adrp x0, :got:symbol           // load page of GOT entry
    ldr  x0, [x0, :got_lo12:symbol] // load address from GOT
```

On AArch64, GOT-indirect addressing is only used for weak extern symbols.
Regular extern symbols use direct PC-relative ADRP+ADD even in PIC/PIE mode,
since this is inherently position-independent and works correctly for
statically-linked executables and early boot code (pre-MMU).

**Thread-Local Storage (TLS):**

```asm
    mrs  x0, tpidr_el0              // read thread pointer
    add  x0, x0, :tprel_hi12:sym   // add high 12 bits of TLS offset
    add  x0, x0, :tprel_lo12_nc:sym // add low 12 bits of TLS offset
```

### Stack-Relative Addressing

Local variables are accessed via SP-relative offsets:

```asm
    ldr  x0, [sp, #offset]    // load from stack slot
    str  x0, [sp, #offset]    // store to stack slot
```

When offsets exceed the immediate encoding range (which varies by instruction), a
large-immediate helper materializes the offset into a scratch register:

```asm
    movz x17, #imm16                      // low 16 bits
    movk x17, #imm16, lsl #16             // (if needed) bits 16-31
    movk x17, #imm16, lsl #32             // (if needed) bits 32-47
    add  x17, sp, x17
    ldr  x0, [x17]
```

### Immediate Encoding

The backend recognizes and exploits AArch64 immediate encoding constraints:

- **imm12** (0-4095): used by `ADD`/`SUB`/`CMP` instructions. The `const_as_imm12`
  helper detects operands that fit, avoiding a register load.
- **Logical immediates**: bitmask patterns encodable in the 13-bit `N:immr:imms` field
  used by `AND`/`ORR`/`EOR`/`TST`. Validated by `is_valid_aarch64_logical_immediate`.
- **Power-of-2 strength reduction**: `UDiv` by a power of 2 is lowered to `LSR`,
  and `URem` by a power of 2 is lowered to `AND` with a bitmask.

---

## F128 Quad-Precision Handling

On AArch64, `long double` is IEEE 754 binary128 (16 bytes). The hardware has no
quad-precision floating-point instructions, so all F128 arithmetic uses soft-float
library calls from compiler-rt or libgcc.

### Storage

F128 values are stored in 16-byte stack slots or NEON Q registers (`q0`-`q7`). The
backend tracks the "source" of each F128 value (which alloca/slot and offset it was
loaded from) to enable full-precision reloads for comparisons and conversions, avoiding
the lossy round-trip through a double-precision truncation.

### Library Calls

| Operation | Library Function |
|-----------|-----------------|
| Addition | `__addtf3` |
| Subtraction | `__subtf3` |
| Multiplication | `__multf3` |
| Division | `__divtf3` |
| Equality comparison | `__eqtf2` |
| Less-than | `__lttf2` |
| Less-or-equal | `__letf2` |
| Greater-than | `__gttf2` |
| Greater-or-equal | `__getf2` |
| F64 to F128 | `__extenddftf2` |
| F128 to F64 | `__trunctfdf2` |

### ABI

F128 values are passed and returned in Q registers per AAPCS64. Operands are loaded
into `q0` (and `q1` for binary operations) before the library call. The result is
returned in `q0`. For the compiler's internal accumulator representation (which is
64-bit), F128 results are truncated to `double` via `__trunctfdf2` after the operation,
with the full-precision value retained in its stack slot for subsequent F128 operations.

### NEON Register Transfers

Moving 128-bit values between Q registers uses the NEON bytewise move:

```asm
    mov v1.16b, v0.16b    // q0 -> q1
```

Loading F128 constants involves materializing the two 64-bit halves:

```asm
    fmov d0, x0           // load low 64 bits into d0 (lower half of q0)
    mov  v0.d[1], x1      // insert high 64 bits into upper half of q0
```

---

## 128-bit Integer Operations

I128 values are represented as register pairs (`x0:x1` where `x0` = low, `x1` = high)
or as two adjacent 8-byte stack slots.

### Inline Arithmetic

| Operation | Implementation |
|-----------|---------------|
| Add | `adds x0, x2, x4` / `adc x1, x3, x5` |
| Sub | `subs x0, x2, x4` / `sbc x1, x3, x5` |
| Mul | `mul x0, x2, x4` / `umulh x1, x2, x4` / `madd x1, x3, x4, x1` / `madd x1, x2, x5, x1` |
| Neg | `mvn` + `adds` + `adc` (two's complement) |
| Not | `mvn x0, x0` / `mvn x1, x1` |
| And/Or/Xor | Parallel per-half operations |

### Shifts

128-bit shifts use branching sequences that handle three cases: shift amount is zero,
less than 64, and 64 or greater. Constant shift amounts use branchless sequences with
`LSL`/`LSR`/`ASR` and `ORR` combined-shift operands.

### Division and Remainder

128-bit division and remainder call compiler-rt/libgcc library functions:

| Operation | Library Function |
|-----------|-----------------|
| Signed division | `__divti3` |
| Unsigned division | `__udivti3` |
| Signed remainder | `__modti3` |
| Unsigned remainder | `__umodti3` |

### Float Conversions

| Conversion | Library Function |
|-----------|-----------------|
| F64 to I128 (signed) | `__fixdfti` |
| F32 to I128 (signed) | `__fixsfti` |
| F64 to I128 (unsigned) | `__fixunsdfti` |
| F32 to I128 (unsigned) | `__fixunssfti` |
| I128 (signed) to F64 | `__floattidf` |
| I128 (signed) to F32 | `__floattisf` |
| I128 (unsigned) to F64 | `__floatuntidf` |
| I128 (unsigned) to F32 | `__floatuntisf` |

### Comparisons

Equality/inequality uses XOR-and-OR reduction:

```asm
    eor x0, x2, x4    // XOR low halves
    eor x1, x3, x5    // XOR high halves
    orr x0, x0, x1    // combine
    cmp x0, #0
    cset x0, eq/ne
```

Ordered comparisons compare high halves first, then low halves on equality.

---

## Atomic Operations

The backend implements atomics using the ARMv8 exclusive monitor mechanism
(load-exclusive / store-exclusive pairs).

### Instruction Selection

The exclusive instruction variant is selected based on type size and memory ordering:

| Size | Load-Exclusive | Store-Exclusive |
|------|---------------|----------------|
| 8-bit | `ldxrb` / `ldaxrb` | `stxrb` / `stlxrb` |
| 16-bit | `ldxrh` / `ldaxrh` | `stxrh` / `stlxrh` |
| 32-bit | `ldxr` / `ldaxr` (w-reg) | `stxr` / `stlxr` (w-reg) |
| 64-bit | `ldxr` / `ldaxr` (x-reg) | `stxr` / `stlxr` (x-reg) |

Acquire semantics use `LDAXR`; release semantics use `STLXR`.

### Atomic RMW Pattern

All atomic read-modify-write operations follow the LL/SC (load-linked / store-conditional)
loop pattern:

```asm
.Latomic_N:
    ldxr  x0, [x1]          // load-exclusive old value
    <op>  x3, x0, x2        // compute new value (add, sub, and, or, xor, nand)
    stxr  w4, x3, [x1]      // store-exclusive new value
    cbnz  w4, .Latomic_N    // retry if exclusive monitor lost
```

### Compare-and-Exchange

```asm
.Lcas_loop_N:
    ldxr  x0, [x1]          // load current value
    cmp   x0, x2            // compare with expected
    b.ne  .Lcas_fail_N      // mismatch: fail
    stxr  w4, x3, [x1]      // try to store desired
    cbnz  w4, .Lcas_loop_N  // retry if monitor lost
    b     .Lcas_done_N
.Lcas_fail_N:
    clrex                    // clear exclusive monitor
.Lcas_done_N:
```

### Atomic Loads and Stores

Atomic loads use `LDAR` (acquire) or plain `LDR` (relaxed). Atomic stores use `STLR`
(release) or plain `STR` (relaxed). Byte and halfword variants (`LDARB`/`LDARH`/
`STLRB`/`STLRH`) are used for sub-word types.

### Fences

| Ordering | Instruction |
|----------|------------|
| Acquire | `dmb ishld` |
| Release | `dmb ishst` |
| AcqRel / SeqCst | `dmb ish` |
| Relaxed | (none) |

---

## NEON/SIMD Intrinsics

The backend provides NEON equivalents for common SSE intrinsics, operating on 128-bit
vectors through Q registers.

### Binary Vector Operations

128-bit binary operations follow a common pattern:

```asm
    ldr q0, [x0]                    // load first operand
    ldr q1, [x1]                    // load second operand
    <neon_op> v0.16b, v0.16b, v1.16b // apply operation bytewise
    str q0, [x0]                    // store result
```

Supported operations include:

| Intrinsic | NEON Instruction | Description |
|-----------|-----------------|-------------|
| `pcmpeqb` | `cmeq v.16b` | Byte-wise equality comparison |
| `pcmpeqd` | `cmeq v.4s` | 32-bit lane equality |
| `psubusb` | `uqsub v.16b` | Unsigned saturating byte subtract |
| `psubsb` | `sqsub v.16b` | Signed saturating byte subtract |
| `por` | `orr v.16b` | Bitwise OR |
| `pand` | `and v.16b` | Bitwise AND |
| `pxor` | `eor v.16b` | Bitwise XOR |
| `loaddqu` | `ldr q` | 128-bit unaligned load |
| `storedqu` | `str q` | 128-bit unaligned store |

### Scalar Operations

| Intrinsic | NEON Instruction |
|-----------|-----------------|
| `sqrt(f64)` | `fsqrt d0, d0` |
| `sqrt(f32)` | `fsqrt s0, s0` |
| `fabs(f64)` | `fabs d0, d0` |
| `fabs(f32)` | `fabs s0, s0` |

### Special Intrinsics

- **`pmovmskb`**: Emulated via `USHR` + multiply-by-bit-position + `ADDV` reduction
  (no direct NEON equivalent)
- **`set_epi8`/`set_epi32`**: Vector broadcast via `DUP v.16b`/`DUP v.4s`
- **`crc32c`**: Hardware CRC32C via `CRC32CB`/`CRC32CH`/`CRC32CW`/`CRC32CX`
- **Barriers**: `DMB ISH` (full), `DMB ISHST` (store), `YIELD` (pause hint)
- **Cache**: `DC CIVAC` (clean and invalidate by VA to Point of Coherency)
- **Non-temporal stores**: Mapped to regular `STR` (ARM has no direct NT store hint
  for general-purpose data)
- **x86-only intrinsics** (AES-NI, CLMUL, `pslldq`, `pshufd`, `paddw`, `pmulhw`,
  etc.): Stubbed to zero their output since no NEON equivalents are implemented

### Popcount

Integer popcount uses NEON byte-count instructions:

```asm
    fmov d0, x0           // move integer to FP register
    cnt  v0.8b, v0.8b     // count set bits per byte
    uaddlv h0, v0.8b      // sum all byte counts
    fmov w0, s0            // move result back to GP register
```

---

## Inline Assembly Support

The backend implements the GCC-compatible inline assembly interface with AArch64-specific
constraint handling and register formatting.

### Constraint Classification

| Constraint | Kind | Description |
|-----------|------|-------------|
| `r` | GP register | General-purpose register (`x0`-`x30`) |
| `w` | FP register | Floating-point/SIMD register (`v0`-`v31`) |
| `m`, `Q` | Memory | Memory operand (stack slot or indirect pointer) |
| `i`, `n` | Immediate | Compile-time constant |
| `I` | Immediate | Unsigned 12-bit (0-4095) for `ADD`/`SUB` |
| `K` | Immediate | 64-bit logical immediate (bitmask pattern) |
| `L` | Immediate | 32-bit logical immediate |
| `g` | General | GP register, memory, or immediate |
| `{reg}` | Specific | Named register (e.g., `{x0}`, `{d5}`) |
| `0`-`9` | Tied | Tied to operand N |

### Register Modifiers

Template operands support modifiers that select the register view:

| Modifier | Effect | Example |
|----------|--------|---------|
| `%w` | 32-bit GP | `%w0` produces `w20` |
| `%x` | 64-bit GP | `%x0` produces `x20` |
| `%s` | 32-bit FP scalar | `%s0` produces `s16` |
| `%d` | 64-bit FP scalar | `%d0` produces `d16` |
| `%q` | 128-bit FP vector | `%q0` produces `q16` |
| `%h` | 16-bit FP half | `%h0` produces `h16` (half-precision view) |
| `%b` | 8-bit FP byte | `%b0` produces `b16` (byte-width view) |
| `%c` | Raw constant | No `#` prefix (for data directives) |
| `%a` | Address reference | `[reg]` form for prefetch |

### Scratch Register Pools

The inline asm emitter allocates scratch registers from two pools:

- **GP scratch**: `x9, x10, x11, x12, x13, x14, x15, x19, x20, x21`
  (caller-saved first, then callee-saved as overflow)
- **FP scratch**: `v16, v17, v18, v19, v20, v21, v22, v23, v24, v25`
  (caller-saved NEON registers; `d8`-`d15` are callee-saved and avoided)

When inline asm clobbers enough caller-saved registers to force allocation into
callee-saved registers (`x19`-`x21`), the prologue prescan (`prescan_inline_asm_callee_saved`)
detects this ahead of time so the prologue can save/restore them.

### GCC Register Aliases

The backend normalizes GCC's `r0`-`r30` aliases to `x0`-`x30` for AArch64, matching
the convention used extensively in Linux kernel inline assembly (e.g., `register
unsigned long r0 asm("r0")` in arm-smccc.h).

---

## Peephole Optimizer

The peephole optimizer runs as a post-processing pass over the emitted assembly text.
It pre-parses every line into a `LineKind` enum for efficient pattern matching using
integer/enum comparisons rather than repeated string parsing.

### Pass Structure

The optimizer runs in three phases:

1. **Iterative local passes** (up to 8 rounds): all core pattern elimination passes
   (store/load elimination, redundant branches, self-moves, move chains,
   branch-over-branch fusion)
2. **Global passes**: register copy propagation and dead store elimination
3. **Local cleanup** (up to 4 rounds): a subset of the local passes (same as Phase 1
   but without branch-over-branch fusion) to mop up patterns exposed by global passes

### Optimization Catalog

**1. Adjacent Store/Load Elimination**

When a `str xN, [sp, #off]` is immediately followed by `ldr xN, [sp, #off]` (same
register, same offset), the load is redundant and eliminated. If the registers differ
(`str xN` then `ldr xM`), the load is replaced with `mov xM, xN`.

Also handles `str wN, [sp, #off]` followed by `ldrsw xN, [sp, #off]` (sign-extending
load after 32-bit store).

**2. Redundant Branch Elimination**

An unconditional `b .label` where `.label:` is the immediately next non-empty line
is a no-op (natural fall-through) and is eliminated.

**3. Self-Move Elimination**

`mov xN, xN` (64-bit) is a no-op and removed. Importantly, `mov wN, wN` (32-bit) is
**not** eliminated because it zeros the upper 32 bits of the 64-bit register, which
is a meaningful operation.

**4. Move Chain Optimization**

The sequence `mov A, B; mov C, A` is transformed to `mov C, B`, which enables the
first `mov` to become dead if `A` has no other uses.

**5. Branch-Over-Branch Fusion**

The pattern:

```asm
    b.cc .Lskip
    b .target
.Lskip:
```

is fused into a single inverted conditional branch:

```asm
    b.!cc .target
```

**6. Move-Immediate Chain**

The sequence `mov xN, #imm; mov xM, xN` where `xN` is a scratch register is collapsed
to `mov xM, #imm` when safe.

**7. Register Copy Propagation** (global)

Propagates register copies across basic blocks, replacing uses of the copy destination
with the original source when the source is still live.

**8. Dead Store Elimination** (global)

Removes `str` instructions to stack slots that are overwritten before being read.

---

## Codegen Options

The backend supports several command-line-driven options:

| Option | Effect |
|--------|--------|
| `-fPIC` / `-fpie` | Enable position-independent code generation. On AArch64, regular extern symbols use direct PC-relative addressing (`ADRP` + `ADD`), which works correctly for both PIC executables and early boot code (pre-MMU). Only weak extern symbols use GOT-indirect addressing (`ADRP` + `LDR :got:`). |
| `-mgeneral-regs-only` | Disable FP/SIMD register use in variadic prologues. The FP register save area is skipped, and `__vr_offs` is set to 0. |
| `-fno-jump-tables` | Disable jump table emission for `switch` statements. |
| Patchable function entry | Reserved NOP sled at function entry for runtime patching. |

---

## Key Design Decisions

### Accumulator-Based Code Generation

The backend uses a stack-based accumulator model where `x0` is the primary working
register. This simplifies instruction selection (every operation follows a uniform
load-operate-store pattern) at the cost of generating redundant moves and stack
traffic. The peephole optimizer is specifically designed to recover much of this cost
by eliminating the redundant patterns in a post-processing pass.

### Register Allocation Strategy

Rather than implementing a full graph-coloring register allocator, the backend uses a
simple but effective two-pool approach: callee-saved registers for values that live
across calls, and a small set of caller-saved registers for call-free live ranges.
This keeps the allocator simple while providing meaningful speedups for hot variables.
The allocator is integrated with inline asm handling: the prescan mechanism ensures
callee-saved registers used by inline asm scratch allocation are properly saved in
the prologue, even though the prologue is emitted before inline asm codegen runs.

### F128 Dual-Track Representation

F128 values carry both a full-precision 16-byte representation (in stack slots and Q
registers) and a truncated `double` approximation in the accumulator. This
dual-track approach lets most of the codegen infrastructure (which assumes 64-bit
accumulator values) work unchanged, while precision-sensitive operations (comparisons,
stores, conversions) reload the full-precision value from its tracked source slot.

### Text-Based Peephole Optimization

The peephole optimizer operates on assembly text rather than a structured IR. This is
a pragmatic choice: it runs after all codegen is complete, so it can catch redundancies
introduced by any part of the pipeline (including inline asm, intrinsics, and library
call sequences). The pre-parsed `LineKind` classification ensures the text-based
approach does not become a performance bottleneck.

### Conservative Scratch Register Partitioning

The corruptible registers `x9`-`x15` are partitioned into dedicated roles (address
computation, memcpy, F128 scratch) rather than being treated as a general pool. This
eliminates the need for tracking register liveness within a single instruction's
emission sequence, which would add significant complexity. Only `x13` and `x14` have
no hardcoded uses and are offered to the register allocator as caller-saved registers.

### Atomic Loop Structure

Atomic operations use the LL/SC (LDXR/STXR) pattern rather than the newer LSE
(Large System Extension) atomics (`LDADD`, `SWPAL`, etc.). This ensures compatibility
with all ARMv8.0 implementations. The retry loop with `CBNZ` handles spurious
exclusive monitor failures that can occur on multiprocessor systems.
