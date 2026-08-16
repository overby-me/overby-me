# x86-64 Backend

Code generation targeting x86-64 (AMD64) with the System V AMD64 ABI. This backend
translates the compiler's IR into AT&T-syntax assembly, handling everything from
register allocation and calling conventions to 128-bit arithmetic, x87 long double,
inline assembly, and hardware intrinsics. The backend includes a builtin assembler
and dynamic linker (enabled by default) with PLT/GOT, TLS, and
copy relocation support, producing ELF executables directly.

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [File Inventory](#file-inventory)
3. [Calling Convention (System V AMD64 ABI)](#calling-convention-system-v-amd64-abi)
4. [Register Allocation](#register-allocation)
5. [Stack Frame Layout](#stack-frame-layout)
6. [Addressing Modes](#addressing-modes)
7. [F128 / Long Double Handling](#f128--long-double-handling)
8. [128-bit Integer Operations](#128-bit-integer-operations)
9. [Inline Assembly Support](#inline-assembly-support)
10. [Intrinsics](#intrinsics)
11. [Peephole Optimizer](#peephole-optimizer)
12. [Codegen Options](#codegen-options)
13. [Key Design Decisions](#key-design-decisions)

---

## Architecture Overview

The backend is structured around a single entry type, `X86Codegen`, which implements
the shared `ArchCodegen` trait. The shared codegen framework walks the IR function and
dispatches to trait methods; each method is implemented in a focused module within
this directory. The overall pipeline for a single function is:

```text
IR Function
   |
   v
calculate_stack_space   -- register allocation + stack layout
   |
   v
emit_prologue           -- push rbp, sub rsp, save callee-saved, variadic save area
   |
   v
emit_store_params       -- store incoming register/stack parameters to slots
   |
   v
emit per-block code     -- instructions dispatched via ArchCodegen trait methods
   |
   v
emit_epilogue + ret     -- restore callee-saved, pop rbp, ret (or retpoline thunk)
   |
   v
peephole_optimize       -- post-pass assembly text optimizer
```

All output is AT&T syntax x86-64 assembly, emitted line-by-line into a string buffer
that is later written to the `.s` file.

---

## File Inventory

The codegen is split into focused modules, all implementing or supporting `ArchCodegen`:

| File | Responsibility |
|------|---------------|
| `emit.rs` | Core `X86Codegen` struct, `ArchCodegen` trait implementation, register constants, PhysReg-to-name mapping, operand loading (`operand_to_rax`, `operand_to_rcx`), result storage (`store_rax_to`), accumulator register cache integration, switch/jump-table emission, and codegen option setters. |
| `prologue.rs` | Function prologue and epilogue. Stack space calculation, register allocation invocation, callee-saved save/restore, stack probing for large frames (>4096 bytes), variadic register save area, and parameter storage from ABI registers to stack slots. |
| `calls.rs` | Function call emission. ABI configuration (`CallAbiConfig`), stack argument pushing (with 16-byte alignment padding), register argument loading (integer via `rdi`..`r9`, float via `xmm0`..`xmm7`), `%al` float count for variadic calls, direct/indirect call instruction emission (including retpoline thunks, `@PLT` in PIC mode), call cleanup, and result extraction with per-eightbyte SSE/INTEGER classification. |
| `alu.rs` | Integer and float unary/binary arithmetic. Accumulator-based fallback path (load to `%rax`/`%rcx`, operate, store result) and register-direct fast path for values assigned to callee-saved registers. Covers add, sub, mul, imul, div, idiv, rem, and/or/xor, shl/shr/sar, neg, not, clz (`lzcnt`), ctz (`tzcnt`), bswap, and popcount (`popcnt`). |
| `comparison.rs` | Integer and float comparisons, `SETcc`/`CMOVcc` emission, fused compare-and-branch for conditional jumps, conditional select via `cmovneq`, and F128 comparisons via x87 `fucomip`. |
| `memory.rs` | Load and store operations. Type-specific move instruction selection (`movb`/`movw`/`movl`/`movq`), stack slot access via `rbp`-relative addressing, GEP constant-offset folding, over-aligned alloca handling, and indirect pointer slot dereferencing. |
| `globals.rs` | Global symbol address computation. RIP-relative `leaq` for local symbols, `GOTPCREL` for external symbols (always, for PIE compatibility) and all non-local symbols in PIC mode, TLS access via `GOTTPOFF`/`TPOFF`, absolute address loads for kernel code model, and label address (`&&label`) via RIP-relative `leaq`. |
| `f128.rs` | F128 (long double) support via the x87 FPU. `fldt`/`fstpt` for 80-bit extended precision load/store, f64-to-x87 promotion, x87-to-integer conversion via `fisttpq` (with unsigned 64-bit special case), raw 10-byte constant materialization, and SlotAddr resolution for Direct/OverAligned/Indirect memory. |
| `float_ops.rs` | Floating-point binary operations (SSE `addss`/`addsd`/`subss`/`subsd`/`mulss`/`mulsd`/`divss`/`divsd`) and F128 binary operations via x87 (`faddp`/`fsubrp`/`fmulp`/`fdivrp`). Float negation for F32/F64 via SSE `xorps`/`xorpd` sign-bit flip, and F128 negation via x87 `fchs`. |
| `cast_ops.rs` | Type cast operations. Intercepts casts to/from F128 for full x87 precision: int-to-F128 via `fildq`, float-to-F128 via `fldl`/`flds`, F128-to-float via `fstpl`/`fstps`, F128-to-int via `fisttpq`. Unsigned-to-F128 uses a 2^63 correction path. All other casts fall through to the shared default. |
| `i128_ops.rs` | 128-bit integer arithmetic using `rax:rdx` pairs. Add/sub with carry (`adcq`/`sbbq`), multiply via `mulq` + cross products, bitwise ops on both halves, shifts via `shldq`/`shrdq` with 64-bit boundary handling, division/remainder via `__divti3`/`__udivti3`/`__modti3`/`__umodti3` libcalls, i128-to-float and float-to-i128 via compiler-rt helpers, and 128-bit comparisons. |
| `returns.rs` | Return value handling. F32/F64 returns via `movd`/`movq` to `xmm0`, F128 returns via x87 `fldt` to `st(0)`, i128 returns via `rax:rdx` with per-eightbyte SSE classification (INTEGER+SSE, SSE+INTEGER, SSE+SSE), struct returns with mixed register classes, and second-return-value accessors for multi-register returns. |
| `variadic.rs` | Variadic function support. `va_arg` implementation with register save area lookup (GP via `gp_offset < 48`, FP via `fp_offset < 176`) and overflow area fallback, `va_arg` for structs (qword-by-qword copy from overflow area), `va_start` initialization of the `va_list` struct, and `va_copy` as memory copy. |
| `inline_asm.rs` | x86 inline assembly template substitution. AT&T syntax operand formatting with size modifiers (`%k0` = 32-bit, `%w0` = 16-bit, `%b0` = 8-bit, `%h0` = 8-bit high, `%q0` = 64-bit), `%a` for RIP-relative addressing, `%c`/`%P` for bare constants, `%n` for negated constants, `%l` for goto labels, and named operand `%[name]` references. |
| `asm_emitter.rs` | `InlineAsmEmitter` trait implementation. Constraint classification (multi-alternative parsing of `r`, `m`, `i`, `x`, `a`/`b`/`c`/`d`/`S`/`D`, `g`, `Q`, `t`/`u` for x87, `{regname}` for explicit registers, `@cc` for condition codes, tied operands), GP and XMM scratch register allocation, memory operand resolution (rbp-relative, over-aligned, indirect, symbol-based RIP-relative), and operand load/store for input/output operands. |
| `atomics.rs` | Atomic operations. `lock xadd` for atomic add, `xchg` for atomic exchange, `xchgb`-based test-and-set, `lock cmpxchg` for CAS (with `sete` for boolean result), cmpxchg-loop for sub/and/or/xor/nand, atomic load/store with `mfence` for SeqCst stores, and `mfence`-based fences. |
| `intrinsics.rs` | Architecture-specific intrinsic emission. SSE/SSE2 packed operations, AES-NI, CLMUL, CRC32, non-temporal stores, memory fences, scalar float math (`sqrtsd`/`sqrtss`/`fabs`), and builtins (`__builtin_frame_address`, `__builtin_return_address`, `__builtin_thread_pointer`). |
| `peephole/` | Post-codegen peephole optimizer. See [`peephole/README.md`](peephole/README.md) for full details. |

---

## Calling Convention (System V AMD64 ABI)

### Integer / Pointer Arguments

The first six integer or pointer arguments are passed in registers, in order:

| Argument | Register |
|----------|----------|
| 1st | `rdi` |
| 2nd | `rsi` |
| 3rd | `rdx` |
| 4th | `rcx` |
| 5th | `r8` |
| 6th | `r9` |

Additional integer arguments are pushed onto the stack right-to-left (highest
address first) with 8-byte alignment.

### Floating-Point Arguments

The first eight float/double arguments are passed in `xmm0` through `xmm7`.
Additional floating-point arguments go on the stack.

### Variadic Calls

Before calling a variadic function, `%al` must contain the number of SSE
registers used for floating-point arguments (0-8). The caller emits either
`movb $N, %al` or `xorl %eax, %eax` accordingly.

### Return Values

| Type | Return register(s) |
|------|--------------------|
| Integer / pointer | `rax` |
| F32 / F64 | `xmm0` |
| F128 (long double) | x87 `st(0)` |
| i128 / u128 | `rax` (low), `rdx` (high) |
| Small structs (<=16 bytes) | Up to two registers, classified per eightbyte |
| Second FP return | `xmm1` |

### Struct Classification (Per-Eightbyte)

Structs of 16 bytes or smaller are classified eightbyte-by-eightbyte into
`INTEGER` or `SSE` classes. Each eightbyte is passed (or returned) in the
corresponding register type:

- **INTEGER eightbyte**: passed in the next available GP register (`rdi`..`r9`)
  or returned in `rax`/`rdx`.
- **SSE eightbyte**: passed in the next available `xmm` register or returned
  in `xmm0`/`xmm1`.
- **Mixed structs** (e.g., `{ int; double; }`) use one GP and one SSE register.

The backend supports all four return combinations: INTEGER+INTEGER,
INTEGER+SSE, SSE+INTEGER, and SSE+SSE.

Structs larger than 16 bytes are passed by pushing their contents onto the
stack (pushed as a sequence of 8-byte words, highest address first).

### ABI Configuration

The `CallAbiConfig` for x86-64 is:

```text
max_int_regs: 6                          // rdi, rsi, rdx, rcx, r8, r9
max_float_regs: 8                        // xmm0 - xmm7
use_sysv_struct_classification: true      // per-eightbyte INTEGER/SSE
f128_in_fp_regs: false                   // long double is on the x87 stack
large_struct_by_ref: false               // large structs pushed by value
allow_struct_split_reg_stack: false       // no partial reg/stack split
```

---

## Register Allocation

The backend uses a shared linear-scan register allocator that runs before stack
space computation. Values assigned to physical registers bypass stack slots
entirely, reducing memory traffic.

### Callee-Saved Registers (5)

These survive across function calls and are saved/restored in the prologue/epilogue:

| PhysReg ID | Register |
|------------|----------|
| 1 | `rbx` |
| 2 | `r12` |
| 3 | `r13` |
| 4 | `r14` |
| 5 | `r15` |

`rbp` is reserved as the frame pointer and is never allocatable.

### Caller-Saved Registers (6)

These are available for values whose live ranges do not span function calls:

| PhysReg ID | Register | Notes |
|------------|----------|-------|
| 10 | `r11` | General-purpose scratch |
| 11 | `r10` | Excluded if function has indirect calls (used as `call *%r10` trampoline) |
| 12 | `r8` | Excluded if function has i128 ops or atomic RMW |
| 13 | `r9` | Excluded if function has i128 ops |
| 14 | `rdi` | Free after prologue stores incoming params; excluded if function has i128 ops |
| 15 | `rsi` | Free after prologue stores incoming params; excluded if function has i128 ops |

The allocator dynamically filters caller-saved registers based on function
characteristics: `r10` is excluded when indirect calls are present (it serves
as the trampoline register for `call *%r10`), `r8`/`r9`/`rdi`/`rsi` are
excluded when 128-bit integer operations appear (they serve as scratch for
multi-register arithmetic), and `r8` is excluded when atomic RMW operations
are present.

### Inline Assembly Interactions

If inline assembly clobbers or uses callee-saved registers (detected by
scanning `{rbx}` constraints or clobber lists), those registers are excluded
from general allocation but are still saved/restored in the prologue/epilogue.

### Accumulator Register Cache

The codegen tracks the current contents of `%rax` with a small tag cache
(`reg_cache`). When `operand_to_rax` is called for a value that is already in
`%rax`, the load is skipped. The cache is invalidated on calls, inline
assembly, and any instruction that implicitly clobbers `%rax`.

---

## Stack Frame Layout

All functions use `rbp`-based frames with the following layout:

```text
        +---------------------------+  (high addresses)
        | caller's stack frame      |
        +---------------------------+
        | return address            |  [rbp + 8]
        +---------------------------+
        | saved rbp                 |  [rbp]
        +---------------------------+
        | callee-saved registers    |  [rbp - frame_size .. rbp - frame_size + N*8]
        |   (rbx, r12-r15 as used) |
        +---------------------------+
        | local variable slots      |  [rbp - X]  (negative offsets, 8-byte aligned)
        |   (IR values, allocas)    |
        +---------------------------+
        | register save area        |  [rbp - Y]  (variadic functions only)
        |   (6 GP + 8 XMM = 176B)  |
        +---------------------------+  <-- rsp (16-byte aligned)
        (low addresses)
```

### Frame Construction

1. **`pushq %rbp`** + **`movq %rsp, %rbp`** -- establish frame pointer.
2. **`subq $frame_size, %rsp`** -- reserve space for locals + callee-saved + variadic area.
3. **Callee-saved save**: each used callee-saved register is stored at a known
   offset within the frame (`movq %reg, offset(%rbp)`).
4. **Variadic save area** (if function is variadic): all six GP argument
   registers and all eight XMM registers are spilled into the register save
   area at the bottom of the frame.

### Stack Probing

Frames larger than 4096 bytes use a probing loop that touches each page:

```asm
    movq $frame_size, %r11
.Lstack_probe_N:
    subq $4096, %rsp
    orl  $0, (%rsp)        # probe (touch the guard page)
    subq $4096, %r11
    cmpq $4096, %r11
    ja   .Lstack_probe_N
    subq %r11, %rsp
    orl  $0, (%rsp)
```

### Frame Alignment

The frame size is rounded up to a 16-byte boundary (`(raw_space + 15) & !15`),
ensuring `%rsp` is always 16-byte aligned after the prologue. Stack argument
pushes for function calls insert an 8-byte padding push when the total stack
argument space is not a multiple of 16.

### Alloca Handling

- **Normal allocas**: assigned a direct `rbp`-relative slot.
- **Over-aligned allocas** (e.g., `__attribute__((aligned(32)))`): extra padding
  is reserved, and the runtime-aligned address is computed as
  `(slot_addr + align-1) & ~(align-1)`.

---

## Addressing Modes

### RIP-Relative (Default for Globals)

Local (non-external) global variables and functions use PC-relative addressing:

```asm
    leaq symbol(%rip), %rax        # address of symbol
    movq symbol(%rip), %rax        # load from symbol
    movq %rax, symbol(%rip)        # store to symbol
```

### RBP-Relative (Local Variables)

All local values and allocas are accessed through the frame pointer:

```asm
    movq -24(%rbp), %rax           # load local
    movq %rax, -32(%rbp)           # store local
    leaq -48(%rbp), %rax           # address of alloca
```

GEP constant offsets are folded into the `rbp` displacement at codegen time.

### GOT / PLT (Position-Independent Code)

External symbol address-of always uses GOTPCREL (even in non-PIC mode) for
PIE compatibility. In PIC mode, all non-local symbol addresses use GOTPCREL:

```asm
    movq symbol@GOTPCREL(%rip), %rax    # load address via GOT
    call function@PLT                    # call via PLT (PIC mode only)
```

### TLS (Thread-Local Storage)

Two modes depending on PIC:

```asm
# PIC mode (initial-exec):
    movq symbol@GOTTPOFF(%rip), %rax
    addq %fs:0, %rax

# Non-PIC mode:
    movq %fs:0, %rax
    leaq symbol@TPOFF(%rax), %rax
```

### Kernel Code Model

With `-mcmodel=kernel`, all symbols are assumed to reside in the negative 2GB
of the virtual address space. Absolute addressing is used (`movq $symbol, %rax`
with `R_X86_64_32S` sign-extended 32-bit relocations) instead of RIP-relative
or GOT-based access.

---

## F128 / Long Double Handling

C `long double` on x86-64 is 80-bit extended precision, stored in 16 bytes
(10 bytes of data + 6 bytes of padding). The backend uses the x87 FPU for all
long double operations.

### Storage Model

F128 values occupy 16-byte stack slots. The backend maintains a set of
"direct slots" (`f128_direct_slots`) tracking which IR values have their
full 80-bit x87 representation stored in their stack slot. This avoids
precision loss from round-tripping through the 64-bit accumulator.

### Load / Store

```asm
    fldt  offset(%rbp)       # load 80-bit extended to x87 ST(0)
    fstpt offset(%rbp)       # store x87 ST(0) as 80-bit extended
```

When the full-precision slot is not available, the fallback path pushes the
f64 value to the stack and converts via `fldl`:

```asm
    pushq %rax               # push f64 bit pattern
    fldl  (%rsp)             # load as x87 double -> extended
    addq  $8, %rsp
    fstpt offset(%rbp)       # store full 80-bit
```

### Arithmetic

F128 binary operations load both operands onto the x87 stack and use x87
instructions:

| C Operation | x87 Instruction |
|-------------|-----------------|
| `a + b` | `faddp %st, %st(1)` |
| `a - b` | `fsubrp %st, %st(1)` |
| `a * b` | `fmulp %st, %st(1)` |
| `a / b` | `fdivrp %st, %st(1)` |
| `-a` | `fchs` |

### Comparisons

F128 comparisons use `fucomip %st(1), %st` followed by `fstp %st(0)` to
pop the second operand, then read condition codes via `seta`/`setae`/
`setnp`/`setp` etc., following the same operand-swapping strategy as SSE
float comparisons.

### Conversions

- **int to F128**: `fildq` (signed) or `fildq` + 2^63 correction (unsigned)
- **F128 to int**: `fisttpq` (signed) or comparison + subtraction for unsigned
- **float/double to F128**: `flds`/`fldl`
- **F128 to float/double**: `fstps`/`fstpl`

### Return Convention

F128 values are returned in x87 `st(0)` per the SysV ABI. The return
sequence loads the value via `fldt` into the FPU stack register before
executing the epilogue.

---

## 128-bit Integer Operations

128-bit integers (`i128`/`u128`) are represented as register pairs `rax:rdx`
(low:high).

### Arithmetic

| Operation | Implementation |
|-----------|---------------|
| Add | `addq %rcx, %rax` / `adcq %rsi, %rdx` |
| Sub | `subq %rcx, %rax` / `sbbq %rsi, %rdx` |
| Multiply | Cross-product via `mulq` + two `imulq` partial products |
| And/Or/Xor | Parallel operation on both halves |
| Shift left | `shldq %cl, %rax, %rdx` / `shlq %cl, %rax` + 64-bit boundary check |
| Shift right (logical) | `shrdq %cl, %rdx, %rax` / `shrq %cl, %rdx` + boundary check |
| Shift right (arithmetic) | `shrdq %cl, %rdx, %rax` / `sarq %cl, %rdx` + sign extension at boundary |
| Div / Rem | Libcall to `__divti3` / `__udivti3` / `__modti3` / `__umodti3` via PLT |

Constant shifts are specialized with direct `shldq`/`shrdq` with immediate
operands and handle the `amount == 64` and `amount > 64` cases explicitly.

### Comparisons

Equality (`==`, `!=`) uses XOR + OR reduction:

```asm
    xorq %rcx, %rax    # diff_lo
    xorq %rsi, %rdx    # diff_hi
    orq  %rdx, %rax    # combined
    sete %al            # (or setne)
```

Ordered comparisons compare the high halves first, then the low halves if
the high halves are equal.

### Float Conversions

i128-to-float and float-to-i128 conversions call compiler-rt helpers:
`__floattidf`, `__floattisf`, `__floatuntidf`, `__floatuntisf`,
`__fixdfti`, `__fixsfti`, `__fixunsdfti`, `__fixunssfti`.

---

## Inline Assembly Support

### Syntax

The backend emits and processes AT&T syntax inline assembly. Template
substitution handles positional (`%0`, `%1`), named (`%[name]`), and
modified operand references.

### Register Size Modifiers

| Modifier | Effect | Example (`rax`) |
|----------|--------|-----------------|
| (none) | Default based on operand type | depends on IR type |
| `%q0` | 64-bit | `%rax` |
| `%k0` / `%l0` | 32-bit | `%eax` |
| `%w0` | 16-bit | `%ax` |
| `%b0` | 8-bit low | `%al` |
| `%h0` | 8-bit high | `%ah` |
| `%c0` / `%P0` | Bare constant (no `$` prefix) | `42` |
| `%n0` | Negated constant | `-42` |
| `%a0` | Address (RIP-relative for symbols) | `symbol(%rip)` |

### Constraint Classification

The backend recognizes the full GCC/Clang x86 constraint vocabulary:

| Constraint | Meaning |
|------------|---------|
| `r`, `q`, `R`, `l` | General-purpose register |
| `Q` | Byte-addressable register (al/bl/cl/dl) |
| `x`, `v`, `Y` | SSE/XMM register |
| `m`, `o`, `V`, `p` | Memory operand |
| `i`, `I`, `n`, `N`, `e`, `E`, `K`, `M`, `G`, `H`, `J`, `L`, `O` | Immediate |
| `g` | General (register, memory, or immediate) |
| `a` / `b` / `c` / `d` / `S` / `D` / `A` | Specific registers (`rax`/`rbx`/`rcx`/`rdx`/`rsi`/`rdi`/`rax`) |
| `t` / `u` | x87 `st(0)` / `st(1)` |
| `{regname}` | Explicit register |
| `=@cc<cond>` | Condition code output |
| `0`-`9` | Tied to operand N |

Multi-alternative constraints (e.g., `"rm"`, `"Ir"`, `"qm"`) are parsed
character by character with priority: specific register > GP register > FP
register > memory > immediate.

### Scratch Register Pools

Inline assembly allocates scratch registers from:

- **GP**: `rcx`, `rdx`, `rsi`, `rdi`, `r8`, `r9`, `r10`, `r11`
- **XMM**: `xmm0` through `xmm15`

---

## Intrinsics

The backend directly emits machine instructions for architecture-specific
intrinsics, avoiding function call overhead.

### Memory Fences and Hints

`lfence`, `mfence`, `sfence`, `pause`, `clflush`

### Non-Temporal Stores

`movnti` (32/64-bit), `movntdq` (128-bit integer), `movntpd` (128-bit double)

### SSE/SSE2 Packed 128-bit Operations

- **Arithmetic**: `paddw`, `psubw`, `paddd`, `psubd`, `pmulhw`, `pmaddwd`
- **Compare**: `pcmpeqb`, `pcmpeqd`, `pcmpgtw`, `pcmpgtb`
- **Logical**: `pand`, `por`, `pxor`, `psubusb`
- **Shuffle / Pack**: `pshufd`, `packssdw`, `packuswb`, `punpcklbw`,
  `punpckhbw`, `punpcklwd`, `punpckhwd`
- **Shift**: `pslldq`, `psrldq`, `psllq`, `psrlq`, `psllw`, `psrlw`,
  `psraw`, `psrad`, `pslld`, `psrld` (with immediate)
- **Move mask**: `pmovmskb`
- **Broadcast**: `set_epi8` (byte splat), `set_epi32` (dword splat)
- **Load / Store**: `loaddqu`, `storedqu`, `loadldi128` (low 64-bit load)

### Scalar Float Math

`sqrtsd` / `sqrtss` (square root), `fabsF64` / `fabsF32` (absolute value
via `andpd`/`andps` sign-bit mask)

### AES-NI

`aesenc`, `aesenclast`, `aesdec`, `aesdeclast`, `aesimc`, `aeskeygenassist`

### CLMUL

`pclmulqdq` (carry-less multiplication with immediate selector)

### CRC32

`crc32b`, `crc32w`, `crc32l`, `crc32q` (hardware CRC32C)

### Builtins

- `__builtin_frame_address(0)` -- reads `%rbp`
- `__builtin_return_address(0)` -- reads `8(%rbp)`
- `__builtin_thread_pointer()` -- reads `%fs:0`

---

## Peephole Optimizer

The post-codegen peephole optimizer operates on the generated AT&T assembly
text. It is a substantial subsystem with its own documentation at
[`peephole/README.md`](peephole/README.md).

In brief, the optimizer runs in eight phases:

1. **Local passes** (iterative, up to 8 rounds): eliminates adjacent
   redundancies -- self-moves, store-then-load of the same slot, reverse
   move pairs, redundant jumps, branch inversions, push/pop elimination,
   redundant sign/zero extensions, and binary-op push/pop rewriting.

2. **Global passes** (single pass): store forwarding across fallthrough
   labels, register copy propagation, dead register move elimination, dead
   store elimination, compare-and-branch fusion, and memory operand folding.

3. **Post-global cleanup** (up to 4 rounds): re-runs local passes to clean
   up opportunities exposed by global passes.

4. **Loop trampoline elimination** (single pass): coalesces SSA loop
   back-edge trampoline blocks, followed by a cleanup round.

5. **Tail call optimization** (single pass): converts `call` + epilogue +
   `ret` sequences into epilogue + `jmp` when safe.

6. **Never-read store elimination** (single pass): whole-function analysis
   removing stores to stack slots that are never subsequently loaded.

7. **Callee-save elimination** (single pass): removes save/restore of
   callee-saved registers that are never referenced in the function body.

8. **Frame compaction** (single pass): repacks surviving callee-saved saves
   and shrinks the stack frame allocation after earlier phases create gaps.

Assembly lines are pre-parsed into compact `LineInfo` structs with integer
enum tags, so all pattern matching in the hot loop uses integer comparisons
rather than string parsing.

---

## Codegen Options

These options are set via `X86Codegen::apply_options()` and control various
code generation behaviors:

| Option | CLI Flag | Effect |
|--------|----------|--------|
| `pic` | `-fPIC` | Position-independent code: use `@GOTPCREL` for all non-local addresses (external symbols always use GOTPCREL even without PIC), `@PLT` for external calls |
| `function_return_thunk` | `-mfunction-return=thunk-extern` | Replace `ret` with `jmp __x86_return_thunk` (Spectre v2 mitigation) |
| `indirect_branch_thunk` | `-mindirect-branch=thunk-extern` | Replace `call *%r10` / `jmp *%rax` with calls to `__x86_indirect_thunk_r10` / `__x86_indirect_thunk_rax` (retpoline) |
| `patchable_function_entry` | `-fpatchable-function-entry=N,M` | Emit N NOPs at function entry (M before the label, N-M after), with `__patchable_function_entries` section pointer for ftrace |
| `cf_protection_branch` | `-fcf-protection=branch` | Emit `endbr64` at function entry for Intel CET / Indirect Branch Tracking |
| `code_model_kernel` | `-mcmodel=kernel` | Assume all symbols in negative 2GB; use absolute `movq $symbol` addressing with `R_X86_64_32S` relocations |
| `no_jump_tables` | `-fno-jump-tables` | Force all switch statements to use compare-and-branch chains instead of jump tables |
| `no_sse` | `-mno-sse` | Disable all SSE/XMM instructions; variadic prologues skip XMM saves, `va_start` sets `fp_offset` to overflow immediately |

---

## Key Design Decisions

### Accumulator-Based Codegen with Register Allocation

The code generator uses an accumulator-based model: most operations load
operands into `%rax` (and `%rcx` for binary ops), perform the operation, and
store the result back. This simple model is augmented by a linear-scan
register allocator that assigns frequently-used values to callee-saved
registers. Register-allocated values use a "register-direct" fast path that
operates directly on the callee-saved register, bypassing `%rax` entirely
for simple ALU operations.

### Register-Only Storage for Allocated Values

Values assigned to physical registers skip stack slot allocation entirely.
`store_rax_to` writes only to the register (not to memory), and
`operand_to_rax` reads from the register. This eliminates redundant memory
traffic for hot values.

### F128 Dual-Track Precision

Long double values maintain two representations simultaneously: the full
80-bit x87 precision in the stack slot (tracked via `f128_direct_slots`)
and a truncated f64 copy in `%rax` for the accumulator-based codegen paths.
This lets the common accumulator infrastructure work unchanged while
preserving full precision for operations that need it.

### Caller-Saved Register Filtering

The set of available caller-saved registers is dynamically narrowed based on
function content. Functions with indirect calls lose `r10` (used as the
call target register). Functions with i128 operations lose `r8`, `r9`,
`rdi`, and `rsi` (used as scratch by multi-register arithmetic). This
avoids interference between the register allocator and instruction-specific
register requirements.

### Jump Table vs. Compare Chain

Switch statements emit either a jump table (relative offsets in `.rodata`,
indexed via `leaq`/`movslq`/`addq`/`jmp *%rdx`) or a linear
compare-and-branch chain. Jump tables are used when the case range is dense
enough to justify the table, and can be disabled entirely with
`-fno-jump-tables`.

### Stack Probing for Large Frames

Frames larger than one page (4096 bytes) use a probing loop that touches
each page sequentially. This ensures the OS guard page is hit before any
large skip, preventing the stack from silently growing past the guard into
other memory regions. This is important for security and correctness on
Linux with its lazy stack page allocation.

### Retpoline / CET Support

The backend supports both Spectre v2 mitigations (retpoline via
`__x86_indirect_thunk_*`) and Intel CET forward-edge control flow integrity
(via `endbr64`). These are independently toggleable and compose correctly:
`endbr64` is emitted at function entry, while retpoline thunks replace
indirect branches throughout the function body.
