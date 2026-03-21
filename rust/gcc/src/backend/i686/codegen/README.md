# i686 Backend -- 32-bit x86 Code Generator

## Overview

The i686 backend targets 32-bit x86 (IA-32) processors, emitting AT&T-syntax
assembly. It implements the `ArchCodegen` trait that the shared code
generation framework dispatches to, producing one `.s` file per translation
unit. The backend includes a builtin 32-bit assembler and linker that reuse
the x86-64 AT&T parser with a 32-bit encoder, producing ELFCLASS32 executables.

The default calling convention is **cdecl** (System V i386 ABI): all arguments
are passed on the stack, pushed right-to-left, and the caller cleans up.
Return values are placed in `%eax` (32-bit scalars), `%eax:%edx` (64-bit
integers), or `st(0)` (float, double, long double). Two alternative calling
conventions are also supported: **`-mregparm=N`** (first 1--3 integer
arguments in `%eax`, `%edx`, `%ecx`) and **`__attribute__((fastcall))`**
(first two DWORD-or-smaller arguments in `%ecx`, `%edx`, callee cleans
the stack).

The backend operates as an *accumulator machine*: intermediate results flow
through `%eax` (and `%edx` for the upper half of 64-bit values), with a
lightweight register allocator that promotes hot IR values into callee-saved
registers. A post-emission peephole optimizer cleans up the redundant
store/load traffic this style produces.

---

## Table of Contents

1. [ILP32 Type Model](#ilp32-type-model)
2. [File Inventory](#file-inventory)
3. [Calling Convention](#calling-convention)
4. [Register Allocation](#register-allocation)
5. [Stack Frame Layout](#stack-frame-layout)
6. [64-bit Operation Splitting](#64-bit-operation-splitting)
7. [F128 / Long Double via x87 FPU](#f128--long-double-via-x87-fpu)
8. [Inline Assembly Support](#inline-assembly-support)
9. [Intrinsics](#intrinsics)
10. [Peephole Optimizer](#peephole-optimizer)
11. [Codegen Options](#codegen-options)
12. [Key Design Decisions and Challenges](#key-design-decisions-and-challenges)

---

## ILP32 Type Model

The i686 target uses the ILP32 data model, which differs from LP64 (x86-64)
in several important ways:

| Type | i686 (ILP32) | x86-64 (LP64) |
|------|:------------:|:--------------:|
| `char` | 1 | 1 |
| `short` | 2 | 2 |
| `int` | 4 | 4 |
| `long` | **4** | 8 |
| `long long` | 8 | 8 |
| pointer | **4** | 8 |
| `size_t` | **4** | 8 |
| `float` | 4 | 4 |
| `double` | 8 | 8 |
| `long double` | **12** (80-bit x87) | 16 (80-bit x87, padded) |

The key consequences for code generation:

- **Pointers are 4 bytes.** Address arithmetic, GEP offsets, and pointer
  loads/stores all use `movl` and 32-bit registers. The assembler pointer
  directive is `.long` (not `.quad`).
- **`long` is 4 bytes**, so `long` and `int` are identical in size.
  This means `long` function parameters need no special treatment relative
  to `int`.
- **`long long` (64-bit) does not fit in a single register** and must be
  split across `%eax:%edx` pairs, creating the register-pair splitting
  described below.
- **`long double` is native 80-bit x87 extended precision**, stored in
  12-byte stack slots (10 bytes of data, 2 bytes padding). It is loaded
  and stored with `fldt`/`fstpt` directly, not via software emulation.

---

## File Inventory

All code generation logic lives under `src/backend/i686/codegen/`:

| File | Responsibility |
|------|---------------|
| `emit.rs` | `I686Codegen` struct and `ArchCodegen` trait impl. Core accumulator helpers (`operand_to_eax`, `operand_to_ecx`, `store_eax_to`), x87 FPU load/store helpers (`emit_f128_load_to_x87`, `emit_f64_load_to_x87`, `emit_f64_store_from_x87`), wide (64-bit) atomic operations via `lock cmpxchg8b`, runtime stubs (`__x86.get_pc_thunk.bx`, `__divdi3`/`__udivdi3`/`__moddi3`/`__umoddi3`), fastcall call emission, segment-override load/store (`%fs:`/`%gs:`), 64-bit bit manipulation (clz, ctz, bswap, popcount), and utility functions. |
| `prologue.rs` | Stack frame setup: `calculate_stack_space`, `emit_prologue`/`emit_epilogue`/`emit_epilogue_and_ret`, parameter storage from stack/registers to slots, register allocator integration, frame pointer omission logic, `aligned_frame_size` computation, and `emit_param_ref` for parameter re-reads. |
| `calls.rs` | Call ABI: stack argument layout, `regparm` register argument emission (reverse-order to avoid clobbering `%eax`), call instruction emission (direct/indirect/PLT), result retrieval (`%eax`, `%eax:%edx`, `st(0)` for float/double/F128). |
| `memory.rs` | Load/store for all type widths, 64-bit and F128 split load/store via `%eax:%edx` and x87, constant-offset load/store with offset folding, GEP address computation (direct, indirect, over-aligned), dynamic alloca support, memcpy emission via `rep movsb`, and over-aligned alloca handling via runtime `leal`+`andl` alignment. |
| `alu.rs` | Integer ALU: `add`/`sub`/`mul`/`and`/`or`/`xor`/`shl`/`shr`/`sar`, signed and unsigned division (`idivl`/`divl`), LEA strength reduction for multiply by 3/5/9, immediate-operand fast paths, integer negation (`negl`), bitwise NOT (`notl`), CLZ (`lzcntl`), CTZ (`tzcntl`), bswap, popcount, and F32 negation (SSE `xorps` with sign-bit mask).  (F64 negation uses x87 `fchs` in `emit.rs`; F128 negation is in `float_ops.rs`.) |
| `i128_ops.rs` | 64-bit register-pair operations (called "i128" in the shared trait): `add`/`adc`, `sub`/`sbb`, `mul` (schoolbook cross-product), `shld`/`shrd` shifts with 32-bit boundary handling, constant shift specializations, comparisons (`cmpl`+`sete`+`andb` for equality, high-first branching for ordered), `__divdi3`/`__udivdi3` calls for division, float conversions via x87 `fildq`/`fisttpq` with unsigned 2^63 correction. |
| `comparison.rs` | Float comparisons (SSE `ucomiss` for F32, x87 `fucomip` for F64/F128), integer comparisons (`cmpl` + `setCC` for all 10 comparison operators), fused compare-and-branch (`cmpl` + `jCC`), and `select` via conditional branching (test condition, branch to true/false label, copy appropriate value). |
| `casts.rs` | Type conversions: integer widening (`movsbl`/`movzbl`/`movswl`/`movzwl`) and narrowing, float-to-int and int-to-float via x87 (`fildl`/`fildq`/`fisttpl`/`fisttpq`), F128 conversions via `fldt`/`fstpt`, unsigned-to-float fixup for values with the sign bit set (2^64 / 2^63 correction paths), SSE scalar F32 casts (`cvtsi2ssl`/`cvttss2si`), and I64 widening/narrowing (sign-extension via `cltd` and half-word extraction). |
| `returns.rs` | Return value placement: 64-bit in `%eax:%edx` (loaded via `emit_load_acc_pair`), F32 returned in `st(0)` (pushed from `%eax` bit pattern via `flds`), F64 returned in `st(0)` (loaded from `%eax:%edx` 8-byte pair via `fldl`), F128 returned in `st(0)` (loaded via `fldt`), 32-bit scalars in `%eax` (no-op). Second return value accessors for F32/F64/F128 multi-register returns. |
| `float_ops.rs` | F128 negation: loads value onto x87 via `fldt`, applies `fchs`, stores back via `fstpt`. |
| `globals.rs` | Global/label address loading: absolute mode (`movl $name`), PIC mode (`@GOT(%ebx)`/`@GOTOFF(%ebx)` relative to GOT base), TLS access (`@GOTNTPOFF(%ebx)` in PIC, `@NTPOFF` in non-PIC, both reading `%gs:0` for the thread pointer base). |
| `variadic.rs` | `va_start` (compute stack pointer to first unnamed argument via `leal`), `va_arg` (load from `va_list` pointer, advance by argument size with 4-byte minimum; special handling for I64/U64/F64 8-byte types, F128 12-byte types via `fldt`, and I128 16-byte quad-word copy), `va_copy` (copy 4-byte pointer). On i686, `va_list` is a simple pointer into the stack frame. |
| `atomics.rs` | 32-bit atomic operations: `lock xadd` for add, `xchg` for exchange, `lock cmpxchg` loops for sub/and/or/xor/nand, `xchgb` for test-and-set, `lock cmpxchg` for CAS, atomic load/store via plain `mov` (with `mfence` for SeqCst), and `mfence` for fences. 64-bit atomics are in `emit.rs` via `lock cmpxchg8b`. |
| `intrinsics.rs` | SSE packed 128-bit operations (arithmetic, compare, logical, shuffle, shift, insert/extract), AES-NI (`aesenc`/`aesenclast`/`aesdec`/`aesdeclast`/`aesimc`/`aeskeygenassist`), CLMUL (`pclmulqdq`), CRC32 (`crc32b`/`crc32l`; 64-bit CRC32 emulated via two 32-bit ops), memory fences (`lfence`/`mfence`/`sfence`/`pause`/`clflush`), non-temporal stores (`movnti`/`movntdq`/`movntpd`), x87 FPU math (`fsqrt`/`fabs` for F32/F64), frame/return address intrinsics, and thread pointer (`%gs:0`). |
| `inline_asm.rs` | Inline assembly template substitution (delegates to shared x86 parser) and operand formatting with size modifiers (`%b` for 8-bit low, `%h` for 8-bit high, `%w` for 16-bit, `%k` for 32-bit). |
| `asm_emitter.rs` | `InlineAsmEmitter` trait impl: GCC constraint classification (`r`/`q` for GP, `a`/`b`/`c`/`d`/`S`/`D` for specific registers, `m` for memory, `i` for immediate, `t`/`u` for x87 `st(0)`/`st(1)`, `{regname}` for explicit registers, `=@cc` for condition codes, digit-tied operands), scratch register allocation from 6 GP registers (`ecx`/`edx`/`esi`/`edi`/`eax`/`ebx`) and 8 XMM registers (`xmm0`-`xmm7`), operand loading/storing for GP, XMM, x87 FPU stack, 64-bit register pairs, and condition code outputs (`=@cc` via `setCC`/`movzbl`), memory operand resolution (ebp-relative, over-aligned, indirect), and memory fallback when GP registers are exhausted. |
| `peephole.rs` | Post-emission assembly optimizer (see dedicated section below). |
| `mod.rs` | Module declarations and visibility. |

---

## Calling Convention

### cdecl (Default)

The standard System V i386 ABI:

```text
   Caller's frame
   ┌──────────────────────┐  higher addresses
   │  arg N               │  ← pushed first (right-to-left)
   │  ...                 │
   │  arg 1               │
   │  arg 0               │
   │  return address       │  ← pushed by CALL
   ├──────────────────────┤
   │  saved %ebp          │  ← pushed by prologue (unless -fomit-frame-pointer)
   │  saved callee-saved   │  ← %ebx, %esi, %edi (as needed)
   │  local variables      │
   │  spill slots          │
   └──────────────────────┘  ← %esp (16-byte aligned at call sites)
```

- All arguments are on the stack. The caller adjusts `%esp` after the call
  to remove them.
- The stack is aligned to 16 bytes at the `call` instruction (modern i386 ABI
  requirement).
- `%eax`, `%ecx`, `%edx` are caller-saved (scratch).
- `%ebx`, `%esi`, `%edi`, `%ebp` are callee-saved.

### `-mregparm=N` (N = 1, 2, or 3)

Passes the first N integer/pointer arguments in registers instead of on the
stack. The register order is `%eax`, `%edx`, `%ecx`. This is used
extensively by the Linux kernel. The `CallAbiConfig` sets `max_int_regs` to N,
and `emit_call_reg_args` loads the arguments into the appropriate registers in
reverse order to avoid clobbering `%eax` (the accumulator) prematurely.

### `__attribute__((fastcall))`

Passes the first two DWORD-or-smaller integer/pointer arguments in `%ecx` and
`%edx`. The callee pops the *stack* arguments on return (callee-cleanup) via
`ret $N`. Implemented via `is_fastcall`, `fastcall_reg_param_count`, and
`fastcall_stack_cleanup` fields on the codegen struct.

The prologue handles fastcall parameter storage by storing from `%ecx`/`%edx`
to the appropriate stack slots, with sub-integer types (I8, U8, I16, U16)
properly sign/zero-extended before storing. The epilogue emits `ret $N`
where N accounts for the stack bytes the callee must clean up.

### ABI Configuration

The `CallAbiConfig` for i686 is (from `calls.rs`):

```text
max_int_regs: regparm (0-3)                    // 0 for cdecl, 1-3 for -mregparm=N
max_float_regs: 0                               // all floats go on the stack
align_i128_pairs: false                          // no even-register alignment for i128
f128_in_fp_regs: false                           // long double passed on the stack, not FP regs
f128_in_gp_pairs: false                          // F128 not split into GP register pairs
variadic_floats_in_gp: false                     // not needed (no FP reg args on i686)
large_struct_by_ref: false                       // large structs pushed by value on the stack
use_sysv_struct_classification: false            // no per-eightbyte classification (x86-64 only)
use_riscv_float_struct_classification: false      // not applicable
allow_struct_split_reg_stack: false              // no partial reg/stack split
align_struct_pairs: false                        // no struct pair alignment
```

### Return Values

| Type | Location |
|------|----------|
| `int`, `long`, pointer | `%eax` |
| `long long` / 64-bit | `%eax` (low), `%edx` (high) |
| `float` | `st(0)` (pushed from `%eax` bit pattern via `flds`) |
| `double` | `st(0)` (loaded from `%eax:%edx` 8-byte pair via `fldl`) |
| `long double` (F128) | `st(0)` (loaded via `fldt`) |

---

## Register Allocation

The i686 backend has only **6 usable general-purpose registers** in total
(excluding `%esp`), of which three are caller-saved scratch:

| Register | Role |
|----------|------|
| `%eax` | Accumulator -- all intermediate results flow through here |
| `%ecx` | Secondary operand register (shift counts, RHS of binary ops) |
| `%edx` | Upper half of 64-bit results; `idivl`/`divl` remainder |
| `%ebx` | Callee-saved, allocatable (PhysReg 0); GOT base in PIC mode |
| `%esi` | Callee-saved, allocatable (PhysReg 1) |
| `%edi` | Callee-saved, allocatable (PhysReg 2) |
| `%ebp` | Frame pointer (callee-saved; allocatable as PhysReg 3 only with `-fomit-frame-pointer`) |

No caller-saved registers are available for general allocation
(`I686_CALLER_SAVED` is empty), because `%eax`, `%ecx`, and `%edx` are
consumed by the accumulator-based codegen as implicit scratch registers.

The register allocator runs before stack space computation and assigns
frequently-used IR values to the callee-saved registers `%ebx`, `%esi`,
`%edi` (and `%ebp` when available). Values assigned to physical registers are
loaded/stored with `movl %reg, ...` instead of going through stack slots,
eliminating memory traffic for the hottest values.

In **PIC mode**, `%ebx` (PhysReg 0) is reserved as the GOT base pointer
(loaded via `__x86.get_pc_thunk.bx` + `_GLOBAL_OFFSET_TABLE_`) and is excluded
from the allocatable set. It is still saved/restored as a callee-saved
register.

Inline assembly clobber lists are integrated into allocation: if an `asm`
block clobbers `%esi`, the allocator will not place values in `%esi` across
that block. Generic constraints (`r`, `q`, `g`) conservatively mark all
callee-saved registers as clobbered, since the scratch allocator might pick
any of them.

### Accumulator Register Cache

The codegen tracks the current contents of `%eax` with a small tag cache
(`reg_cache`). When `operand_to_eax` is called for a value that is already
in `%eax`, the load is skipped. The cache is invalidated on calls, inline
assembly, and any instruction that implicitly clobbers `%eax`. This simple
one-entry cache eliminates a significant fraction of redundant loads without
the complexity of a full register allocator.

---

## Stack Frame Layout

### With Frame Pointer (default)

```text
   higher addresses
   ┌──────────────────────┐
   │  arg 1               │  12(%ebp)
   │  arg 0               │   8(%ebp)
   │  return address       │   4(%ebp)
   ├──────────────────────┤
   │  saved %ebp          │   0(%ebp)  ← %ebp points here
   │  saved %ebx          │  -4(%ebp)
   │  saved %esi          │  -8(%ebp)
   │  ...                 │
   │  local slot 0         │  -N(%ebp)
   │  local slot 1         │  -(N+4)(%ebp)
   │  ...                 │
   └──────────────────────┘  ← %esp (16-byte aligned)
```

All local slots are referenced as negative offsets from `%ebp`. The total
frame size (the `subl $N, %esp` in the prologue) is rounded up so that
`%esp` is 16-byte aligned, accounting for the saved `%ebp`, return address,
and callee-saved register pushes.

Stack slots are 4-byte granularity by default. 64-bit values get 8-byte
slots; F128 (long double) gets 12-byte slots; 128-bit integers get 16-byte
slots. Over-aligned allocas (e.g., `__attribute__((aligned(16)))`) get
extra space and are dynamically aligned at access time with
`leal`/`addl`/`andl` sequences.

### Frame Alignment

The `aligned_frame_size` function ensures `%esp` is 16-byte aligned after
the prologue completes. It accounts for the fixed overhead on the stack
(callee-saved register pushes + return address + saved `%ebp` if present):

```text
fixed_overhead = callee_saved_bytes + 8  (with FP: saved ebp + return addr)
fixed_overhead = callee_saved_bytes + 4  (without FP: return addr only)
raw_locals     = raw_space - callee_saved_bytes
needed         = raw_locals + fixed_overhead
aligned        = (needed + 15) & !15
frame_size     = aligned - fixed_overhead
```

This rounds up the total stack usage (locals + overhead) to the next
16-byte boundary, then subtracts the fixed overhead to get the `subl`
operand for the prologue.

The alignment bias (8 with FP, 12 without FP) also appears in the
per-alloca slot allocation logic, where it ensures that allocas requesting
16-byte or greater alignment land on properly aligned addresses at runtime.

### Without Frame Pointer (`-fomit-frame-pointer`)

When the frame pointer is omitted, `%ebp` is freed as a fourth callee-saved
register (PhysReg 3). All stack references use `%esp`-relative addressing
instead. The `slot_ref` helper converts the EBP-relative offsets stored in
`StackSlot` values to ESP-relative offsets by adding `frame_base_offset +
esp_adjust`:

- `frame_base_offset` = `callee_saved_bytes + frame_size` (set once in the
  prologue)
- `esp_adjust` tracks temporary ESP changes during code generation (e.g.,
  `subl $N, %esp` for call arguments, `pushl` for temporaries)

This bookkeeping is critical for correctness: every `subl`/`pushl` that
modifies `%esp` increments `esp_adjust`, and every `addl`/`popl` decrements
it, keeping slot references accurate throughout the function body.

Parameter references require a small correction: without the pushed `%ebp`,
parameters are 4 bytes closer to the current stack frame. The `param_ref`
helper subtracts 4 from the EBP-relative offset before adding the
ESP-relative base.

Dynamic allocas (`alloca` / VLAs) force the frame pointer to remain enabled,
since ESP changes by runtime-computed amounts that cannot be statically
tracked.

---

## 64-bit Operation Splitting

Because every general-purpose register is 32 bits wide, 64-bit values
(`long long`, `double` bit patterns, `uint64_t`) must be represented as
register pairs or 8-byte stack slots.

### Register Pair Convention

The canonical register pair is `%eax:%edx` (low:high). For 64-bit
arithmetic:

| Operation | Instruction sequence |
|-----------|---------------------|
| Add | `addl` low, `adcl` high |
| Subtract | `subl` low, `sbbl` high |
| Multiply | Cross-multiply with `mull` + `imull`, accumulate partial products into `%edx` |
| Left shift | `shldl %cl, %eax, %edx` / `shll %cl, %eax` with branch on `%cl >= 32` |
| Logical right shift | `shrdl %cl, %edx, %eax` / `shrl %cl, %edx` with branch on `%cl >= 32` |
| Arithmetic right shift | `shrdl %cl, %edx, %eax` / `sarl %cl, %edx` with sign-extend fixup |
| Bitwise ops | Pair of `andl`/`orl`/`xorl` on both halves |
| Negate | `notl` both halves, `addl $1` low, `adcl $0` high |
| Bitwise NOT | `notl` both halves |
| Compare (eq/ne) | `cmpl` + `sete` on each half, `andb` the results (for ne: `xorb $1`) |
| Compare (ordered) | Compare high halves first; if equal, compare low halves (unsigned for low half, signed/unsigned for high depending on the comparison) |

The right-hand operand is pushed onto the stack before the operation and
popped afterward, since all scratch registers are occupied by the result pair.

Constant shifts are specialized inline without branches, using different
sequences for amounts < 32, == 32, and > 32.

### 64-bit Division and Modulo

Hardware `divl`/`idivl` only supports 32-bit divisors. For 64-bit
division, the backend calls runtime helper functions (`__divdi3`,
`__udivdi3`, `__moddi3`, `__umoddi3`) following the cdecl convention -- both
the dividend and divisor are pushed as 8-byte pairs. The compiler emits
`.weak` implementations of these helpers (based on compiler-rt's algorithms)
so that standalone builds without libgcc can link successfully, while builds
that do link libgcc naturally use its versions instead.

The division helper stubs use normalized-divisor estimation and are only
emitted when 64-bit division is actually used (`needs_divdi3_helpers` flag on
the codegen state).

### 64-bit Float Conversions

Conversions between 64-bit integers and floating-point use the x87 FPU:

- **Signed i64 to float**: Push the 64-bit value onto the stack, `fildq`
  to load as a signed integer onto the x87 stack, then `fstps`/`fstpl` to
  convert to the target float type.
- **Unsigned u64 to float**: Same as signed, but with a 2^63 correction
  path: if the high bit is set, the value is halved (right shift by 1),
  converted via `fildq`, then doubled via `fadd %st(0), %st(0)`.
- **Float to signed i64**: Load the float onto x87 (`flds`/`fldl`), then
  `fisttpq` to truncate and store as a 64-bit integer.

---

## F128 / Long Double via x87 FPU

On i686, `long double` maps to the x87 80-bit extended precision format
(10 bytes of data, stored in 12-byte aligned slots). Unlike x86-64, where
F128 is often software-emulated via `__float128` library calls, the i686
backend uses the x87 FPU natively:

- **Load:** `fldt offset(%ebp)` pushes the 80-bit value onto `st(0)`.
- **Store:** `fstpt offset(%ebp)` pops `st(0)` and writes 10 bytes.
- **Arithmetic:** `faddp`, `fsubp`, `fmulp`, `fdivp` operate on the x87
  stack.
- **Negation:** `fchs` negates `st(0)`.
- **Comparison:** Two values are loaded onto the x87 stack; `fucomip`
  compares `st(0)` with `st(1)` and sets EFLAGS directly (P6+ feature),
  followed by `fstp %st(0)` to pop the remaining operand.
- **Conversions:** Integer-to-F128 uses `fildl`/`fildq` (load integer from
  memory to x87); F128-to-integer uses `fisttpq` (truncate and store).
  Float/double to F128 uses `flds`/`fldl`; F128 to float/double uses
  `fstps`/`fstpl`.

Constants are materialized by constructing the 80-bit x87 byte representation
on the stack with `movl`/`movw` and then loading with `fldt`. The
`f128_bytes_to_x87_bytes` helper converts from IEEE binary128 to x87
extended format.

Tracking which values are "directly" in F128 slots (vs. loaded through a
pointer) is maintained via the `f128_direct_slots` set in `CodegenState`.

---

## Inline Assembly Support

### Template Substitution

The backend delegates to the shared x86 inline assembly parser for template
substitution. Positional (`%0`, `%1`), named (`%[name]`), and modified
operand references are all supported.

### Register Size Modifiers

| Modifier | Effect | Example (`eax`) |
|----------|--------|-----------------|
| (none) | Default based on operand type (I8/U8→`%al`, I16/U16→`%ax`, else→`%eax`) | depends on IR type |
| `%k0` | 32-bit | `%eax` |
| `%w0` | 16-bit | `%ax` |
| `%b0` | 8-bit low | `%al` |
| `%h0` | 8-bit high | `%ah` |
| `%c0` / `%P0` | Bare constant (no `$` prefix) | `42` |
| `%n0` | Negated constant | `-42` |
| `%a0` | Address reference | `symbol` (memory operand form) |

### Constraint Classification

The backend recognizes the GCC/Clang i386 constraint vocabulary:

| Constraint | Meaning |
|------------|---------|
| `r`, `q`, `R`, `l` | General-purpose register |
| `Q` | Byte-addressable register (al/bl/cl/dl) |
| `x`, `v`, `Y` | SSE/XMM register |
| `m`, `o`, `V`, `p` | Memory operand |
| `i`, `I`, `n`, `N`, `e`, `E`, `K`, `M`, `G`, `H`, `J`, `L`, `O` | Immediate |
| `g` | General (register, memory, or immediate) |
| `a` / `b` / `c` / `d` / `S` / `D` | Specific registers (`eax`/`ebx`/`ecx`/`edx`/`esi`/`edi`) |
| `t` / `u` | x87 `st(0)` / `st(1)` |
| `{regname}` | Explicit register |
| `=@cc<cond>` | Condition code output (emits `set{cc}` + `movzbl`) |
| `0`-`9` | Tied to operand N |

### Scratch Register Pools

Inline assembly allocates scratch registers from:

- **GP**: `ecx`, `edx`, `esi`, `edi`, `eax`, `ebx` (6 registers)
- **XMM**: `xmm0` through `xmm7` (8 registers)

When all GP registers are exhausted by operand assignments, the allocator
falls back to memory operands (stack slot references).

### x87 FPU Stack Operands

The `t` and `u` constraints map to `st(0)` and `st(1)` respectively. For
input operands, values are loaded onto the x87 stack via `fldt`/`fldl`/`flds`.
For output operands, the x87 stack top is stored back via `fstpt`/`fstpl`/
`fstps`. This supports inline assembly that operates on x87 registers
directly (common in math library code and legacy FPU routines).

### 64-bit Register Pairs

For I64/U64 types, the inline assembly emitter handles register pairs
automatically. Each 64-bit operand receives two GP registers (a low and
high half), and template substitution produces the appropriate register
for the requested half.

---

## Intrinsics

The backend directly emits machine instructions for architecture-specific
intrinsics, avoiding function call overhead.

### Memory Fences and Hints

`lfence`, `mfence`, `sfence`, `pause`, `clflush`

### Non-Temporal Stores

`movnti` (32-bit), `movntdq` (128-bit integer), `movntpd` (128-bit double)

### SSE/SSE2 Packed 128-bit Operations

- **Arithmetic**: `paddw`, `psubw`, `paddd`, `psubd`, `pmulhw`, `pmaddwd`
- **Compare**: `pcmpeqb`, `pcmpeqd`, `pcmpgtw`, `pcmpgtb`
- **Logical**: `pand`, `por`, `pxor`, `psubusb`, `psubsb`
- **Shuffle / Pack**: `pshufd`, `pshuflw`, `pshufhw`, `packssdw`,
  `packsswb`, `packuswb`, `punpcklbw`, `punpckhbw`, `punpcklwd`,
  `punpckhwd`
- **Shift**: `pslldq`, `psrldq`, `psllq`, `psrlq`, `psllw`, `psrlw`,
  `psraw`, `psrad`, `pslld`, `psrld` (with immediate)
- **Move mask**: `pmovmskb`
- **Broadcast**: `set_epi8` (byte splat), `set_epi16` (word splat),
  `set_epi32` (dword splat)
- **Load / Store**: `loaddqu`, `storedqu`, `loadldi128` (low 64-bit load),
  `storeldi128` (low 64-bit store)
- **Insert / Extract**: `pinsrw`, `pextrw`, `pinsrb`, `pextrb`, `pinsrd`,
  `pextrd` (SSE4.1 byte/dword variants)

### Scalar Float Math

x87 `fsqrt` (square root for F32/F64), x87 `fabs` (absolute value for F32/F64)

### AES-NI

`aesenc`, `aesenclast`, `aesdec`, `aesdeclast`, `aesimc`, `aeskeygenassist`

### CLMUL

`pclmulqdq` (carry-less multiplication with immediate selector)

### CRC32

`crc32b`, `crc32w`, `crc32l` (hardware CRC32C). The 64-bit `crc32q` variant
is emulated on i686 via two 32-bit CRC32 operations on each half.

### Builtins

- `__builtin_frame_address(0)` -- reads `%ebp`
- `__builtin_return_address(0)` -- reads `4(%ebp)`
- `__builtin_thread_pointer()` -- reads `%gs:0`

---

## Peephole Optimizer

After all assembly text is emitted, the entire function is processed by a
multi-pass peephole optimizer (`peephole.rs`) that eliminates redundancies
inherent in the accumulator-based code generation style.

### Pass Structure

1. **Local passes** (iterative, up to 8 rounds):
   - **Store/load elimination:** A `movl %eax, -8(%ebp)` immediately
     followed by `movl -8(%ebp), %eax` -- the load is removed. If the
     load is into a different register, it is converted to a reg-reg move.
   - **Self-move elimination:** `movl %eax, %eax` is deleted.
   - **Strength reduction:** `addl $1` → `incl`, `subl $1` → `decl`,
     `movl $0, %reg` → `xorl %reg, %reg`, with carry-flag safety checks
     to avoid breaking sequences that depend on CF.
   - **Redundant sign/zero-extension elimination:** A `movsbl ..., %eax`
     followed by `movsbl %al, %eax` -- the second is removed.
   - **Redundant jump elimination:** An unconditional `jmp` to the
     immediately following label is removed.
   - **Branch inversion:** A conditional jump over an unconditional jump is
     inverted to eliminate the unconditional jump.
   - **Reverse move elimination:** A `movl %ecx, %eax` followed by
     `movl %eax, %ecx` -- the second is removed.

2. **Global passes** (single pass):
   - **Dead register move elimination:** A `movl %eax, %ecx` where `%ecx`
     is never read before being overwritten is removed.
   - **Dead store elimination:** A `movl %eax, -8(%ebp)` where the slot is
     written again before being read is removed.
   - **Compare+branch fusion:** Detects patterns where a comparison result
     is stored, reloaded, and tested (`cmpl + setCC + movzbl + testl %eax
     + jne/je`), fusing them into a single`cmpl` + `jCC`.
   - **Memory operand folding:** Replaces a load-from-slot + ALU-with-register
     sequence (`movl -N(%ebp), %ecx; addl %ecx, %eax`) with a single
     ALU-with-memory-operand instruction (`addl -N(%ebp), %eax`).

3. **Local cleanup** (up to 4 rounds): Re-runs local and global passes to
   clean up opportunities exposed by the previous round.

4. **Never-read store elimination:** A global analysis collects all loaded
   stack offsets, then removes stores to offsets that are never loaded anywhere
   in the function. Conservatively bails if any `leal` address-of or indirect
   memory access exists (which could create aliased slot references).

### Line Classification

Every assembly line is classified into a `LineKind` enum (`StoreEbp`,
`LoadEbp`, `Move`, `SelfMove`, `Label`, `Jmp`, `JmpIndirect`, `CondJmp`,
`Call`, `Ret`, `Push`, `Pop`, `SetCC`, `Cmp`, `Directive`, `Nop`, `Empty`,
`Other`) for efficient pattern matching. Register operands are mapped to
family IDs (0--7 for `%eax` through `%edi`) so that sub-register aliases
(`%al`, `%ax`, `%eax`) are treated as the same physical register.

Implicit register uses are tracked for instructions like `cltd` (reads `%eax`,
writes `%edx`), `idivl`/`divl` (reads `%eax:%edx`, writes `%eax:%edx`),
`rep movsb` (uses `%esi`/`%edi`/`%ecx`), and `mull`/`imull` (writes
`%eax:%edx`). This ensures dead-register elimination does not remove moves
that are consumed by implicit register operands.

---

## Codegen Options

These options are applied via `I686Codegen::apply_options()`:

| Option | CLI Flag | Effect |
|--------|----------|--------|
| `pic` | `-fPIC` | Position-independent code: use `@GOT(%ebx)` for external globals, `@GOTOFF(%ebx)` for local globals, `@PLT` for external calls; reserves `%ebx` as GOT base |
| `regparm` | `-mregparm=N` | Pass first N (1--3) integer/pointer arguments in `%eax`, `%edx`, `%ecx` instead of on the stack |
| `omit_frame_pointer` | `-fomit-frame-pointer` | Skip `%ebp` frame pointer setup; use `%esp`-relative addressing; free `%ebp` as a 4th callee-saved register |
| `no_jump_tables` | `-fno-jump-tables` | Force all switch statements to use compare-and-branch chains instead of jump tables |
| `emit_cfi` | (internal) | Emit `.cfi_startproc`/`.cfi_endproc` CFI directives for DWARF unwinding |

The `-m16` flag sets `code16gcc` mode, which prepends `.code16gcc` to the
assembly output. This GNU assembler directive causes all subsequent 32-bit
instructions to be emitted with operand-size and address-size override
prefixes, allowing the code to execute in 16-bit real mode while being
written in 32-bit syntax. Used by the Linux kernel's early boot code.
The `.code16gcc` directive is prepended in the backend dispatcher
(`src/backend/mod.rs`) after peephole optimization completes.

---

## Key Design Decisions and Challenges

### The Accumulator Bottleneck

With only 6 GPRs total (3 scratch, 3 callee-saved), the i686 backend cannot
use a general-purpose register allocator the way x86-64 can with its 15
GPRs. Instead, it uses `%eax` as a universal accumulator: every expression
evaluation flows through `%eax`, with `%ecx` as the secondary operand
register for binary operations and `%edx` as the implicit upper-half
register for multiply/divide/64-bit pairs.

This design is simple and correct, but produces excessive memory traffic
(store to stack, reload from stack). The register allocator mitigates this
by assigning the most frequently used values to `%ebx`, `%esi`, `%edi`
(and `%ebp` when available), and the peephole optimizer eliminates the
remaining redundant store/load pairs.

The accumulator register cache (`reg_cache`) tracks what IR value is currently
in `%eax`, allowing `operand_to_eax` to skip the load when the value is
already present. This simple one-entry cache eliminates a significant
fraction of redundant loads without the complexity of a full register
allocator.

### 64-bit Values on a 32-bit Machine

Every 64-bit operation requires careful orchestration of register pairs.
The difficulty is compounded by the scarcity of registers: with `%eax:%edx`
holding the result and `%ecx` needed for shift counts, there are no scratch
registers left for the second operand. The backend resolves this by pushing
the RHS onto the stack and operating against `(%esp)`.

64-bit comparisons are particularly tricky: ordered comparisons must first
check the high halves, then branch to check the low halves only if the high
halves are equal. This requires careful label management and different
condition codes for the high (signed) and low (unsigned) halves.

### ESP Tracking for Frame Pointer Omission

Without `%ebp` as a stable reference point, every temporary ESP adjustment
(pushing call arguments, pushing temporaries for x87 conversions, etc.)
shifts all stack slot addresses. The `esp_adjust` field is meticulously
incremented and decremented around every `pushl`/`subl` and `popl`/`addl`
that modifies `%esp`, and `slot_ref` adds it to every stack access. A
single missed update would silently corrupt all subsequent memory references.

### PIC Mode and `%ebx` Reservation

Position-independent code on i686 requires a GOT base register. The
backend reserves `%ebx` for this purpose, loading it in the prologue via
`call __x86.get_pc_thunk.bx` / `addl $_GLOBAL_OFFSET_TABLE_, %ebx`.
Global address references use `@GOT(%ebx)` for external symbols and
`@GOTOFF(%ebx)` for local symbols. The `__x86.get_pc_thunk.bx` helper is
emitted as a COMDAT section so that the linker deduplicates it across
translation units.

### Standalone 64-bit Division Runtime

Programs that link without libgcc (e.g., musl libc) need compiler-provided
implementations of `__divdi3`, `__udivdi3`, `__moddi3`, and `__umoddi3`.
The backend emits these as `.weak` symbols in the `.text` section, based on
the compiler-rt i386 division algorithms using normalized-divisor estimation.
If libgcc is linked, its strong symbols take precedence. The stubs are only
emitted when 64-bit division is actually used (`needs_divdi3_helpers` flag).

### 64-bit Atomic Operations

The i686 ISA has no 64-bit atomic load/store instructions. The backend uses
`lock cmpxchg8b` loops for all 64-bit atomic operations (RMW, cmpxchg,
load, store). This requires `%ebx` and `%ecx` for the desired value and
`%eax:%edx` for the expected/old value, consuming all scratch registers
and `%ebx`. The backend saves `%ebx` and `%esi` on the stack and uses
`%esi` as the pointer register for the duration of the atomic operation.

F64 values are treated as "atomic wide" alongside I64/U64, since they
require the same 8-byte atomic semantics.

### Segment-Override Load/Store

The backend supports `%fs:` and `%gs:` segment-override prefixed memory
accesses, used for thread-local storage and kernel per-CPU data. Both
pointer-based (`%gs:(%ecx)`) and symbol-based (`%gs:symbol`) addressing
forms are supported, with proper type-width register selection (`%al`/`%ax`/
`%eax` for byte/word/dword operands).

### Division-by-Constant Optimization (Disabled)

The IR-level `div_by_const` pass, which replaces integer division by
compile-time constants with multiply-and-shift sequences, is **disabled for
the i686 target**. The replacement sequences use `MulHigh` (upper-half
multiply) operations that the IR expresses as 64-bit arithmetic. The i686
backend truncates 64-bit operations to 32 bits in its accumulator, producing
incorrect results for these sequences.

Until a 32-bit-aware variant is implemented (using single-operand `imull`
for the upper-half multiply), the backend falls back to hardware
`idivl`/`divl` instructions for all division and modulo operations. The
guard is `!target.is_32bit()` in the optimization pipeline.
