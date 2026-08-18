# Backend Subsystem Design

The backend transforms SSA IR produced by the compiler's middle-end into
target-specific assembly text, then assembles and links it into ELF
executables. Four architectures are supported: x86-64, i686, AArch64, and
RISC-V 64. Each architecture has a builtin assembler (instruction encoder +
ELF object file writer) and a builtin linker (symbol resolution + relocation
application + ELF executable writer), enabling fully self-contained
compilation with no external toolchain. An external GCC toolchain can be used
as a fallback.

---

## Code Generation Pipeline

```text
                         +------------------+
                         |     IR Module     |
                         | (SSA, per-func)   |
                         +--------+---------+
                                  |
                     +------------v-----------+
                     |   Pre-scan Analysis     |
                     |  - GEP fold map         |
                     |  - Cmp-branch fusion    |
                     |  - GlobalAddr folding    |
                     |  - Use counts            |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     | Stack Layout / RegAlloc |
                     |  - Three-tier slot      |
                     |    allocation           |
                     |  - Liveness analysis    |
                     |  - Linear scan regalloc |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     |  Instruction Selection  |
                     |  (ArchCodegen trait)     |
                     |  - Per-instruction      |
                     |    dispatch             |
                     |  - Prologue / epilogue  |
                     |  - ABI parameter stores |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     |   Raw Assembly Text     |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     |   Peephole Optimizer    |
                     |  - Local passes (x8)    |
                     |  - Global passes (x1)   |
                     |  - Local cleanup (x4)   |
                     |  - Tail call, callee-   |
                     |    save, frame compact   |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     |  Optimized Assembly     |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     |   Builtin Assembler     |
                     |  parse asm text         |
                     |  encode instructions    |
                     |  write ELF .o           |
                     +------------+-----------+
                                  |
                     +------------v-----------+
                     |   Builtin Linker        |
                     |  read .o + CRT + libs   |
                     |  resolve symbols        |
                     |  apply relocations      |
                     |  write ELF executable   |
                     +------------+-----------+
                                  |
                         +--------v---------+
                         |   ELF Executable  |
                         +------------------+
```

The pipeline is driven from `Target::generate_assembly_with_opts_and_debug` in
`mod.rs`. For each target, it:

1. Instantiates the architecture-specific codegen struct (e.g., `X86Codegen`).
2. Applies CLI-driven `CodegenOptions` (PIC, retpoline, CET, patchable entry,
   etc.).
3. Calls `generation::generate_module_with_debug`, which emits data sections,
   iterates over functions, and dispatches each IR instruction through the
   `ArchCodegen` trait.
4. Passes the resulting assembly text through the architecture's peephole
   optimizer.
5. Returns the final assembly string, which is then assembled (via the
   builtin assembler or an external toolchain) and linked (via the builtin
   linker or an external toolchain) into an ELF executable.

---

## Directory Layout

The backend is split into shared modules at the top level (some as directory
modules with submodules) and 4 architecture-specific subdirectories:

```text
src/backend/
  mod.rs              Target enum, CodegenOptions, top-level dispatch
  elf/                Shared ELF constants, StringTable, read/write helpers, archive parsing
    mod.rs            Core ELF types, struct definitions, re-exports
    constants.rs      ELF format constants (section types, flags, relocations)
    string_table.rs   StringTable builder for ELF string sections
    section_flags.rs  Section flag parsing from GAS directives
    archive.rs        AR archive (.a) parsing
    io.rs             Binary read/write helpers (little-endian, big-endian)
    parse_string.rs   GAS string literal parsing with escape sequences
    linker_symbols.rs Linker-generated symbols (_GLOBAL_OFFSET_TABLE_, etc.)
    symbol_table.rs   ELF symbol table emission
    numeric_labels.rs GAS numeric label (1:, 2f, 3b) support
    object_writer.rs  High-level ELF object file writer
    writer_base.rs    Low-level ELF writer (headers, sections, relocations)
  linker_common/      Shared linker infrastructure (17 files, see linker_common/README.md)
    mod.rs            Re-exports, linker-defined symbol detection
    types.rs          Core ELF64 types: Elf64Section, Elf64Symbol, Elf64Object, DynSymbol
    parse_object.rs   Parse ELF64 relocatable objects (.o)
    parse_shared.rs   Extract dynamic symbols and SONAME from shared libraries (.so)
    symbols.rs        GlobalSymbolOps trait, InputSection/OutputSection
    merge.rs          Merge input sections into output sections, COMMON symbols
    dynamic.rs        Match undefined globals against shared library exports
    archive.rs        Load archives (.a, thin archives), iterative resolution
    resolve_lib.rs    Resolve -l library names to filesystem paths
    args.rs           Parse -Wl, linker flags into structured LinkerArgs
    check.rs          Post-link undefined symbol validation
    section_map.rs    Section ordering and address assignment
    write.rs          Shared ELF executable writing helpers
    dynstr.rs         Dynamic string table builder
    hash.rs           GNU hash table and SysV hash table generation
    eh_frame.rs       .eh_frame FDE counting and .eh_frame_hdr builder
    gc_sections.rs    --gc-sections BFS reachability analysis
  peephole_common.rs  Shared peephole optimizer utilities: word matching, register replacement, LineStore
  asm_expr.rs         Shared integer expression evaluator (all 4 assembler parsers)
  asm_preprocess.rs   Shared GAS preprocessing: comments, macros, .rept, .if/.elseif/.else/.endif
  elf_writer_common.rs Shared generic ELF object writer for x86-64 and i686 assemblers
  traits.rs           ArchCodegen trait (~185 methods, ~64 default impls)
  generation.rs       Module/function/instruction dispatch (arch-independent)
  state.rs            CodegenState, StackSlot, SlotAddr, RegCache
  stack_layout/       Three-tier stack slot allocation (7 files, see stack_layout/README.md)
    mod.rs            Entry point, calculate_stack_space_common driver
    analysis.rs       Use counting, immediately-consumed detection, block analysis
    alloca_coalescing.rs  Escape analysis for non-escaping single-block allocas
    copy_coalescing.rs    Copy alias tracking (share source slot)
    slot_assignment.rs    Tier 2 liveness packing, Tier 3 block-local reuse
    inline_asm.rs     Inline asm callee-saved register scanning
    regalloc_helpers.rs Register allocation setup and parameter alloca lookup
  call_abi.rs         Unified ABI classification (CallArgClass, ParamClass)
  cast.rs             CastKind classification, FloatOp classification
  liveness.rs         Live interval computation (backward dataflow)
  regalloc.rs         Linear scan register allocator
  common.rs           Assembler/linker invocation, data emission
  f128_softfloat.rs   Shared F128 soft-float orchestration (ARM + RISC-V)
  inline_asm.rs       InlineAsmEmitter trait and 4-phase framework
  x86_common.rs       Shared x86/i686 register names, condition codes

  x86/                x86-64 backend (SysV AMD64 ABI)
    codegen/          Code generation (18 files) + peephole optimizer subdirectory
    assembler/        Builtin assembler (parser, encoder, ELF writer)
    linker/           Builtin linker (dynamic linking, PLT/GOT, TLS)
  i686/               i686 backend (cdecl, ILP32)
    codegen/          Code generation (18 files) + peephole optimizer
    assembler/        Builtin assembler (reuses x86 parser, 32-bit encoder)
    linker/           Builtin linker (32-bit ELF, R_386 relocations)
  arm/                AArch64 backend (AAPCS64)
    codegen/          Code generation (19 files) + peephole optimizer
    assembler/        Builtin assembler (parser, encoder, ELF writer)
    linker/           Builtin linker (dynamic linking, IFUNC/TLS)
  riscv/              RISC-V 64 backend (LP64D)
    codegen/          Code generation (19 files) + peephole optimizer
    assembler/        Builtin assembler (parser, encoder, RV64C compress)
    linker/           Builtin linker (dynamic linking)
```

Each architecture subdirectory contains 18-19 codegen files
(including `mod.rs`) that implement the `ArchCodegen` trait methods. The
x86 backend's peephole optimizer is a subdirectory (`peephole/`) rather
than a single file, containing its own module structure for the multi-stage
pass pipeline:

| File | Responsibility |
|------|---------------|
| `emit.rs` | Struct definition, `ArchCodegen` impl, `delegate_to_impl!` |
| `alu.rs` | Integer arithmetic and bitwise operations |
| `atomics.rs` | Atomic load/store/RMW/cmpxchg |
| `calls.rs` | Function call emission, argument marshalling |
| `cast_ops.rs` (`casts.rs` on i686) | Type casts (int widening/narrowing, int-float conversions) |
| `comparison.rs` | Comparison and fused compare-and-branch |
| `f128.rs` | F128 (long double) operations (absent on i686, which uses x87) |
| `float_ops.rs` | Floating-point arithmetic |
| `globals.rs` | Global address materialization, TLS access |
| `i128_ops.rs` | 128-bit integer operations |
| `inline_asm.rs` | Architecture-specific inline assembly emission |
| `intrinsics.rs` | Compiler builtins (popcount, bswap, clz, etc.) |
| `memory.rs` | Load, store, memcpy, GEP |
| `peephole.rs` or `peephole/` | Post-generation assembly optimization (x86 uses a subdirectory) |
| `prologue.rs` | Function prologue and epilogue |
| `returns.rs` | Return value emission |
| `variadic.rs` | Variadic function support (va_start, va_arg, va_copy) |
| `asm_emitter.rs` | `InlineAsmEmitter` trait implementation |

For architecture-specific details, see:

| Architecture | Overview | Code Generation | Assembler | Linker |
|-------------|----------|----------------|-----------|--------|
| x86-64 | [`x86/README.md`](x86/README.md) | [`x86/codegen/README.md`](x86/codegen/README.md) | [`x86/assembler/README.md`](x86/assembler/README.md) | [`x86/linker/README.md`](x86/linker/README.md) |
| i686 | [`i686/README.md`](i686/README.md) | [`i686/codegen/README.md`](i686/codegen/README.md) | [`i686/assembler/README.md`](i686/assembler/README.md) | [`i686/linker/README.md`](i686/linker/README.md) |
| AArch64 | [`arm/README.md`](arm/README.md) | [`arm/codegen/README.md`](arm/codegen/README.md) | [`arm/assembler/README.md`](arm/assembler/README.md) | [`arm/linker/README.md`](arm/linker/README.md) |
| RISC-V 64 | [`riscv/README.md`](riscv/README.md) | [`riscv/codegen/README.md`](riscv/codegen/README.md) | [`riscv/assembler/README.md`](riscv/assembler/README.md) | [`riscv/linker/README.md`](riscv/linker/README.md) |

The x86-64 peephole optimizer has its own detailed documentation: [`x86/codegen/peephole/README.md`](x86/codegen/peephole/README.md).

---

## The ArchCodegen Trait

The `ArchCodegen` trait (defined in `traits.rs`) is the central abstraction
that decouples the shared code generation framework from architecture-specific
instruction emission. It defines approximately 185 methods organized into
several categories:

- **State access**: `state()` and `state_ref()` provide mutable and immutable
  access to the shared `CodegenState`.
- **Prologue/epilogue**: `emit_prologue`, `emit_epilogue`,
  `calculate_stack_space`, `aligned_frame_size`.
- **Operand handling**: `emit_load_operand`, `emit_store_result`,
  `emit_copy_value`.
- **Memory operations**: `emit_store`, `emit_load`,
  `emit_load_with_const_offset`, `emit_store_with_const_offset`,
  `emit_seg_load`, `emit_seg_store`, `emit_global_load_rip_rel`,
  `emit_global_store_rip_rel`.
- **Arithmetic and logic**: `emit_binop`, `emit_unaryop`,
  `emit_float_binop`.
- **Comparisons**: `emit_cmp`, `emit_fused_cmp_branch_blocks`.
- **Casts**: `emit_cast`, `emit_cast_instrs`.
- **Control flow**: `emit_branch`, `emit_cond_branch_blocks`, `emit_switch`,
  `emit_indirect_branch`.
- **Function calls**: `emit_call` (handles both direct and indirect calls
  via `direct_name: Option<&str>` and `func_ptr: Option<&Operand>`), plus
  the 8-phase hook methods (`emit_call_compute_stack_space`,
  `emit_call_f128_pre_convert`, `emit_call_spill_fptr`,
  `emit_call_stack_args`, `emit_call_sret_setup`, `emit_call_reg_args`,
  `emit_call_instruction`, `emit_call_cleanup`, `emit_call_store_result`).
- **Atomics**: `emit_atomic_load`, `emit_atomic_store`, `emit_atomic_rmw`,
  `emit_atomic_cmpxchg`.
- **128-bit**: `emit_i128_binop`, `emit_i128_cmp`,
  `emit_i128_store_result`, `emit_store_acc_pair`,
  `emit_load_acc_pair`.
- **Register allocation**: `get_phys_reg_for_value`, `emit_reg_to_reg_move`,
  `emit_acc_to_phys_reg`.

### Default Implementations and Primitive Composition

Approximately 64 methods have default implementations that capture shared
codegen patterns. These defaults are built from small "primitive" methods that
each backend overrides with 1--4 line architecture-specific implementations.
The design lets the shared framework express an algorithm once while backends
only provide instruction-level differences.

Key default implementations include:

- **`emit_store_default` / `emit_load_default`**: Resolve a value's `SlotAddr`
  (Direct, Indirect, or OverAligned) and dispatch to the appropriate primitive:
  `emit_typed_store_to_slot`, `emit_typed_store_indirect`, or
  `emit_alloca_aligned_addr` plus `emit_typed_store_indirect`. Each primitive
  is a backend-supplied one-liner.
- **`emit_copy_value`**: Checks whether source and destination have physical
  register assignments (from the register allocator) and emits direct
  register-to-register moves when possible, falling back to the accumulator
  load/store path otherwise. Backends needing special handling (e.g., x86 F128
  x87 copies) override this for the special case and delegate the rest to the
  default.
- **`emit_binop`**: Classifies the operation as i128, float, or integer and
  delegates to the corresponding specialized method.
- **`emit_call`**: Orchestrates an 8-phase call sequence (Phase 0: classify
  arguments and compute stack space, Phase 1: F128 pre-conversion, Phase 2:
  spill function pointer, Phase 3: push stack arguments, Phase 3.5: sret
  pointer setup, Phase 4: load register arguments, Phase 5: emit the call
  instruction, Phase 6: clean up the stack and store the result). Each phase
  is a backend-supplied hook method.
- **`emit_load_with_const_offset` / `emit_store_with_const_offset`**: Handle
  GEP-folded memory accesses by dispatching on `SlotAddr` and folding the
  constant offset into the appropriate addressing mode.
- **`build_jump_table`**: Shared jump table construction. All 64-bit backends
  use relative 32-bit offsets (`.long target - table_base`) to avoid
  unresolved `R_*_ABS64` relocations; i686 uses absolute 4-byte entries.

The `traits.rs` module also provides free functions (`emit_store_default`,
`emit_load_default`, `emit_cast_default`, `emit_unaryop_default`,
`emit_return_default`) that backends overriding a trait method for special
types (e.g., x86 F128) can call for the non-special cases, avoiding code
duplication.

### The `delegate_to_impl!` Macro

Most `impl ArchCodegen for XxxCodegen` blocks consist of one-liner
delegations to `_impl` methods defined on the codegen struct itself. The
`delegate_to_impl!` macro eliminates this boilerplate:

```rust
delegate_to_impl! {
    fn calculate_stack_space(&mut self, func: &IrFunction) -> i64
        => calculate_stack_space_impl;
    fn emit_prologue(&mut self, func: &IrFunction, frame_size: i64)
        => emit_prologue_impl;
    fn store_instr_for_type(&self, ty: IrType) -> &'static str
        => store_instr_for_type_impl;
}
```

Each line maps a trait method to its corresponding `_impl` counterpart. The
macro handles `&self`/`&mut self` receivers and optional return types,
generating the forwarding body automatically.

---

## Code Generation Dispatch (generation.rs)

The `generation.rs` module contains the arch-independent driver that
orchestrates code generation through the `ArchCodegen` trait. Its entry
points are:

### `generate_module`

1. Pre-sizes the output buffer based on total IR instruction count (each
   instruction generates roughly 40 bytes of assembly text; buffer is
   clamped to 256 KB--64 MB).
2. Collects symbol sets (local, TLS, weak extern) for PIC and GOT decisions.
3. Builds and emits the DWARF file table (`.file` directives) when debug info
   is enabled.
4. Emits data sections (`.data`, `.bss`, `.rodata`, string literals) via
   `common::emit_data_sections`.
5. Emits top-level `asm("...")` directives verbatim.
6. Emits extern visibility directives for referenced symbols.
7. Iterates over functions, calling `generate_function` for each.
8. Emits aliases, symbol attributes, `.init_array`/`.fini_array` entries.
9. Emits architecture-specific runtime helper stubs (e.g., i686 `__divdi3`).
10. Emits `.note.GNU-stack` section for non-executable stack.

### `generate_function`

1. Resets per-function state and emits linkage directives (`.globl`,
   `.local`), visibility, and type directives.
2. Emits patchable function entry NOP padding when configured
   (`-fpatchable-function-entry`), along with the
   `__patchable_function_entries` section pointer for ftrace. Inline
   functions are excluded to avoid overwhelming the kernel's ftrace
   initialization.
3. For naked functions, emits only inline asm blocks with no
   prologue/epilogue.
4. Pre-scans for `DynAlloca`/`StackRestore` (triggers frame-pointer-based SP
   restore in the epilogue).
5. Calls `calculate_stack_space` -- which internally runs the three-tier
   allocator and register allocator -- then aligns the frame and emits the
   prologue.
6. Emits parameter stores from argument registers to stack slots.
7. Builds several pre-scan maps used during instruction dispatch:
   - **Value use counts**: for compare-branch fusion eligibility and GEP
     fold analysis.
   - **GEP fold map**: identifies GEPs with constant offsets foldable into
     Load/Store (uses value use counts to verify single-use).
   - **Global address map**: maps GlobalAddr values to symbol names for
     RIP-relative folding.
   - **Global address pointer set**: distinguishes pointer vs. integer uses
     of GlobalAddr in kernel code model.
   - **Foldable GlobalAddr set**: GlobalAddr values whose `leaq` can be
     skipped entirely (all uses are foldable Load/Store pointers).
8. Iterates over basic blocks. At each block boundary, invalidates the
   register cache. For each instruction, calls `generate_instruction`; for
   the terminator, either emits a fused compare-and-branch or calls
   `generate_terminator`.
9. Emits `.size` directive.

### `generate_instruction`

A large `match` statement dispatches each IR instruction variant to the
appropriate `ArchCodegen` method. It also manages the register value cache:
instructions that follow the load-compute-store pattern leave the accumulator
holding the destination value, so subsequent instructions can skip reloading.
Instructions with unpredictable clobbers (calls, inline asm, atomics, complex
operations) invalidate the cache.

---

## Three-Tier Stack Slot Allocation (stack_layout/)

All four backends share a unified stack layout algorithm that assigns stack
slots to IR values. The algorithm is implemented in
`calculate_stack_space_common` and uses three tiers to minimize frame size.

### Tier 1: Alloca Slots

Allocas represent addressable local variables. They receive permanent,
non-shared stack slots because their addresses may escape (be passed to
other functions, stored in pointers, etc.). The exception is
**non-escaping single-block allocas**: an escape analysis pass
(`compute_coalescable_allocas`) identifies allocas whose addresses never
leave their defining block (no address taken in calls, no address stored to
memory, no address passed through phi nodes or across block boundaries), and
these are demoted to Tier 3 for block-local sharing.

Dead non-parameter allocas (those with no uses at all) are detected and
skipped entirely -- no stack slot is allocated.

### Tier 2: Multi-Block SSA Temporaries

Non-alloca SSA values that are live across multiple basic blocks use
**liveness-based packing**. The liveness analysis (from `liveness.rs`)
computes live intervals for each value. A greedy interval coloring
algorithm uses a min-heap to assign values with non-overlapping live
intervals to the same stack slot. This is particularly effective for
switch-heavy code (dispatch tables, state machines) where many values are
defined in different case handlers that never execute simultaneously.

Multi-definition values (from phi elimination, which creates Copy
instructions in multiple predecessor blocks) are always routed to Tier 2,
since their definition spans multiple blocks and they cannot safely share
block-local pools.

### Tier 3: Single-Block Values

Non-alloca values that are defined and used entirely within a single basic
block use **block-local slot reuse**. Each block maintains its own slot
pool; pools from different blocks overlap in the frame since only one block
executes at a time. Within a block, slots are reused greedily: once a
value's last use has been passed, its slot becomes available for the next
value of the same size.

Non-escaping single-block allocas (identified by the escape analysis in
Tier 1) are included in the Tier 3 pools, sharing slots with regular
single-block values.

### Additional Optimizations

- **Copy alias tracking**: `Copy` instructions that simply move a value
  between SSA names share the same stack slot as their source, avoiding
  redundant allocation and copies.
- **Immediately-consumed value elimination**: Values that are produced and
  immediately consumed by the very next instruction as the first operand do
  not need a stack slot at all -- the accumulator register cache keeps them
  alive. The `immediately_consumed` set in `StackLayoutContext` tracks
  these.
- **Dead parameter alloca elimination**: Unused parameter allocas are
  detected and skipped entirely (no stack slot needed).
- **Small slot tracking**: The `small_slot_values` set in `CodegenState`
  tracks values eligible for 4-byte slots (I32, U32, F32, and smaller on
  64-bit targets), but slot allocation currently always uses 8 bytes. Using
  4-byte movl store/load is unsafe because it zero-extends on reload, losing
  sign information when 32-bit values are widened to 64 bits (see
  `ideas/reduce_stack_frame_size_for_postgres.txt`).
- **Deferred slot finalization**: Block-local slots use `DeferredSlot`
  entries whose final frame offset is computed only after all tiers have
  determined their space requirements, so Tier 3 slots are placed after the
  Tier 1 and Tier 2 regions.

---

## Linear Scan Register Allocator (regalloc.rs)

The register allocator assigns physical registers to IR values based on
their live intervals, prioritizing values with the most uses and longest
lifetimes. Values that do not receive a register remain on the stack and
are accessed through the accumulator load/store path.

### Three-Phase Allocation

**Phase 1 -- Callee-saved registers for call-spanning values.**
Values whose live ranges span function calls are assigned callee-saved
registers (x86: `rbx`, `r12`--`r15`; ARM: `x20`--`x28`; RISC-V: `s1`,
`s7`--`s11`). These registers are preserved across calls by the ABI, so no
per-call save/restore is needed -- only the prologue and epilogue must save
and restore them. The linear scan walks sorted candidate intervals and
assigns each to the callee-saved register with the earliest free time.

**Phase 2 -- Caller-saved registers for non-call-spanning values.**
Values whose live ranges do not cross any call are assigned caller-saved
registers (x86: `r11`, `r10`, `r8`, `r9`; ARM: `x13`, `x14`). Since
these values are not live across calls, the registers do not need to be
saved or restored at all -- neither at call sites nor in the
prologue/epilogue.

**Phase 3 -- Callee-saved spillover.**
After Phases 1 and 2, any remaining callee-saved registers are assigned to
the highest-priority non-call-spanning values that did not fit in the
caller-saved pool. This is critical for call-free hot loops (hash
functions, matrix multiply, sorting kernels) where all values compete for
only a few caller-saved registers. The one-time prologue/epilogue
save/restore cost is amortized over many loop iterations.

### Priority Scoring

Candidates are sorted by a priority score that combines live range length
and use count. Uses inside loops are weighted exponentially by nesting
depth: a use at loop depth D contributes 10^D to the weighted count
(depth 1 = 10x, depth 2 = 100x, depth 3 = 1000x, capped at 10,000 for
very deep nesting). This ensures inner-loop temporaries receive registers
ahead of straight-line code values, which is critical for compute-heavy
loops like zlib's `deflate_slow`, `longest_match`, and `slide_hash`.

### Eligibility Filtering

The allocator uses a whitelist approach: only values produced by simple,
well-understood instructions are eligible. Specifically:

- **Eligible**: `BinOp`, `UnaryOp`, `Cmp`, `Cast` (GPR types only),
  `Load`, `GetElementPtr`, `Copy`, `Call`/`CallIndirect` (integer result),
  `Select`, `GlobalAddr`, `LabelAddr`, `AtomicLoad`, `AtomicRmw`,
  `AtomicCmpxchg`.
- **Excluded**: Alloca values (they represent stack addresses, not data);
  float and F128 values (they use dedicated FP register paths); i128
  values (they require register pairs); I64/U64 on 32-bit targets (they
  require `eax:edx` pairs); values used only once immediately after
  definition (no benefit from a register); values used as memory pointers
  in instructions whose codegen paths access `resolve_slot_addr` directly.

The allocator does not split live intervals: a value either gets a register
for its entire lifetime or remains on the stack.

### Integration with Stack Layout

The register allocator runs during `calculate_stack_space` as part of the
`run_regalloc_and_merge_clobbers` helper. Its liveness analysis result is
cached in `RegAllocResult::liveness` so the Tier 2 liveness-based slot
packing can reuse it, avoiding a redundant dataflow computation. Inline
assembly clobber registers are collected separately
(`collect_inline_asm_callee_saved`) and merged with the register allocator's
used-register set to produce the final set of callee-saved registers
requiring prologue/epilogue save/restore.

---

## Liveness Analysis (liveness.rs)

The liveness module computes live intervals for each IR value. A live
interval `[start, end]` represents the program point range where a value
must be preserved (either in a register or a stack slot).

### Backward Dataflow

The analysis proceeds in three steps:

1. **Numbering**: Assign sequential program points to all instructions and
   terminators across all basic blocks.
2. **Dataflow**: Run backward dataflow iteration to compute `live_in` and
   `live_out` sets for each block. This correctly handles values live across
   loop back-edges by iterating until a fixed point is reached. The dataflow
   uses compact bitsets (packed `u64` words) instead of hash sets, with
   value IDs remapped to a dense `[0..N)` range. Operations are
   word-level: union = bitwise OR, difference = AND-NOT, equality = `==`.
   This eliminates per-iteration heap allocation and replaces hash-table
   operations with fast word-level bitwise ops.
3. **Interval construction**: Build intervals by taking the union of
   definition/use points and live-through blocks.

### Auxiliary Results

The liveness result includes:

- **Call points** (`call_points`): program points corresponding to
  `Call`/`CallIndirect` instructions, used by the register allocator to
  identify which values span calls and therefore need callee-saved
  registers.
- **Loop nesting depth** (`block_loop_depth`): per-block depth computed via
  DFS-based back-edge detection. Depth 0 = not in any loop, depth 1 = one
  loop, etc. Used for priority weighting in the register allocator.

### Canonical Operand Iterators

The module also provides the canonical instruction/terminator operand
iterators (`for_each_operand_in_instruction`,
`for_each_value_use_in_instruction`, `for_each_operand_in_terminator`)
used by code generation, register allocation, stack layout, and liveness
analysis itself. This ensures that new IR instruction variants only need
operand traversal updates in one place.

---

## Call ABI Classification (call_abi.rs)

The call ABI module provides a unified classification system for function
call arguments and callee-side parameters. The core insight is that callers
and callees must agree exactly on where each argument lives (which register
or which stack offset), so the classification algorithm is implemented once
in `classify_args_core` and wrapped by two thin entry points:

- **`classify_call_args`**: caller-side classification (returns
  `CallArgClass`), used by call emission to place arguments into registers
  and stack slots.
- **`classify_params_full`**: callee-side classification (returns
  `ParamClass` with concrete stack offsets), used by `emit_store_params` to
  load incoming parameters from their ABI-defined locations.

### Classification Categories

The classifier walks the argument list and assigns each to one of these
categories, consuming GP and FP register slots in order:

| Category | Description |
|----------|-------------|
| `IntReg` | Integer/pointer in a GP register |
| `FloatReg` | Float/double in an FP register |
| `I128RegPair` | 128-bit integer in an aligned GP register pair |
| `F128Reg` / `F128GpPair` / `F128AlwaysStack` | Long double (arch-specific) |
| `StructByValReg` | Small struct (<=16 bytes) in 1--2 GP registers |
| `StructSseReg` | Small struct with all-float eightbytes in XMM registers |
| `StructMixedIntSseReg` | Mixed struct: INTEGER first, SSE second |
| `StructMixedSseIntReg` | Mixed struct: SSE first, INTEGER second |
| `StructSplitRegStack` | Struct split across last GP register and stack |
| `LargeStructStack` / `LargeStructByRefReg` / `LargeStructByRefStack` | Large struct (>16 bytes) |
| `Stack` / `F128Stack` / `I128Stack` / `StructByValStack` | Overflow to stack |
| `ZeroSizeSkip` | Zero-size struct, consumes nothing |

### Architecture Parameterization via `CallAbiConfig`

The classification is parameterized by a `CallAbiConfig` struct that each
backend provides via its `call_abi_config()` method. This captures ABI
differences without duplicating the core algorithm:

- Number of available GP and FP argument registers (x86: 6 GP + 8 FP;
  ARM: 8 GP + 8 FP; RISC-V: 8 GP + 8 FP; i686: 0 GP by default, up to 3
  with `-mregparm`).
- Whether variadic float arguments are promoted to GP registers (ARM,
  RISC-V) or remain in FP registers (x86).
- Whether SysV struct classification (per-eightbyte GP/SSE analysis) is
  used (x86-64 only, via `classify_sysv_struct`).
- Whether i128 arguments require even-aligned register pairs (ARM, RISC-V).
- Whether large structs are passed by hidden reference in a GP register
  (AAPCS64) or by value on the stack.
- F128 handling: x87 stack convention (x86), Q-register (ARM), or GP pair
  (RISC-V).
- Whether sret uses a dedicated register (ARM: x8) or consumes a regular GP
  slot (x86: rdi, RISC-V: a0). When a dedicated register is used, the callee
  classification promotes the first stack-overflow GP arg to the freed register
  slot so that caller and callee agree on argument locations.

---

## Cast Classification (cast.rs)

The `cast.rs` module provides a shared `CastKind` enum and
`classify_cast_with_f128` function that all four backends use to determine
what conversion to emit. After pointer normalization (`Ptr` treated as
`U64`) and F128 reduction (on x86, `F128` is approximated as `F64` for
x87), the classifier returns one of approximately 20 `CastKind` variants:

- `Noop` -- no conversion needed (same type, or Ptr to I64/U64).
- `FloatToSigned` / `FloatToUnsigned` -- FP to integer.
- `SignedToFloat` / `UnsignedToFloat` -- integer to FP.
- `FloatToFloat` -- F32 to F64 or vice versa.
- `IntWiden` / `IntNarrow` -- integer widening (sign/zero-extend) or
  truncation.
- `SignedToUnsignedSameSize` / `UnsignedToSignedSameSize` -- same-width
  reinterpretation (the RISC-V case requires sign-extension for U32 to I32
  due to the ABI requiring sign-extended 32-bit values in 64-bit
  registers).
- `SignedToF128` / `UnsignedToF128` / `F128ToSigned` / `F128ToUnsigned` /
  `FloatToF128` / `F128ToFloat` -- IEEE binary128 softfloat conversions on
  ARM/RISC-V.

The `f128_is_native` parameter distinguishes ARM/RISC-V (IEEE binary128,
requiring `__floatsitf`, `__fixtfdi`, `__extendsftf2`, etc.) from x86
(x87 80-bit, approximated as F64 for computation).

The module also provides `FloatOp` classification and `classify_float_binop`
to map IR binary operations to their floating-point categories, and
`F128CmpKind` / `f128_cmp_libcall` for F128 comparison library call
mapping.

---

## Peephole Optimizer

Each backend has a text-based peephole optimizer that operates on the
generated assembly string after instruction selection and before the
external assembler. The optimizer works on the full assembly output of a
module, processing it line by line.

### Line Classification

Each line of assembly text is pre-parsed into a compact enum/struct
representation. On x86-64, a `LineInfo` struct captures the `LineKind`
(store-to-rbp, load-from-rbp, self-move, label, jump, call, push, pop,
etc.), destination register, stack offset, extension kind, whether the line
contains indirect memory access, and a bitmask of referenced register
families. On ARM, a `LineKind` enum captures stores/loads to sp, register
moves, branches, and ALU instructions. This pre-parsing avoids repeated
string comparisons in hot optimization loops.

### NOP Marking Strategy

Lines to be eliminated are marked as `Nop` (their kind set to the Nop
variant and their text replaced with an empty string) rather than removed
from the line array. This preserves array indices for multi-line pattern
matching and adjacency checks. A final compaction pass filters out all
Nop-marked lines before returning the optimized text.

### Iterative Convergence

Local passes run iteratively (up to 8 rounds on all four architectures,
up to 4 rounds for cleanup) until no further changes are made. Each round makes a
single pass over all lines applying multiple pattern matchers
simultaneously. This handles cascading opportunities where one optimization
(e.g., removing a redundant load) exposes another (e.g., making a store
dead).

### x86-64 Pass Structure (Seven Phases)

The x86-64 peephole (in `x86/codegen/peephole/`) is the most
comprehensive, organized into seven phases:

**Phase 1 -- Local passes** (iterative, up to 8 rounds):
`combined_local_pass` merges several single-scan patterns into one:

1. **Adjacent store/load elimination**: `movq %rax, -8(%rbp)` followed by
   `movq -8(%rbp), %rax` -- the load is redundant since the value is
   already in the register.
2. **Redundant jump elimination**: `jmp .LBB0_1` where `.LBB0_1:` is the
   immediately next non-empty line.
3. **Self-move elimination**: `movq %rax, %rax` is a no-op.
4. **Redundant `cltq` elimination**: sign-extension when the value is
   already sign-extended.
5. **Redundant zero/sign extension elimination**: eliminates unnecessary
   `movzbl`, `movsbl`, etc.

Additionally: push/pop elimination and binary-op push/pop rewriting
(replacing push-op-pop sequences with direct register operations).

**Phase 2 -- Global passes** (single execution):

- **Global store forwarding** across fallthrough labels.
- **Register copy propagation**.
- **Dead register move elimination**.
- **Dead store elimination** (stores to stack slots never subsequently
  loaded).
- **Compare-and-branch fusion** at the assembly level.
- **Memory operand folding** (combining separate load + operation into a
  single memory-operand instruction).

**Phase 3 -- Post-global local cleanup** (up to 4 rounds):
Re-runs local passes to clean up new opportunities exposed by global
passes (e.g., a global pass may make a store dead, which makes a preceding
load dead).

**Phase 4 -- Loop trampoline elimination**: Removes unnecessary jump
trampolines created during code generation, followed by additional local
cleanup if changes were made.

**Phase 5 -- Tail call optimization + never-read store elimination**:
Converts `call` + `ret` sequences into `jmp` (tail calls), then performs
whole-function analysis removing stores to stack slots that are never
subsequently loaded.

**Phase 6 -- Unused callee-save elimination**: Removes prologue
push/epilogue pop pairs for callee-saved registers that are never actually
referenced in the function body.

**Phase 7 -- Frame compaction**: Reassigns stack slot offsets to eliminate
gaps left by eliminated stores, reducing total frame size.

### Other Architectures

- **AArch64**: Three-phase structure (8 rounds local, global passes once,
  4 rounds cleanup). Local passes cover store/load elimination on
  `[sp, #off]` pairs, redundant branch elimination, self-move elimination
  (64-bit `mov xN, xN` only -- 32-bit `mov wN, wN` zeros upper bits and
  is not safe to eliminate), move chain optimization (`mov A, B; mov C, A`
  becomes `mov C, B`), branch-over-branch fusion (`b.cc .Lskip; b .target;
  .Lskip:` becomes `b.!cc .target`), and move-immediate chain optimization.
  Global passes include register copy propagation and dead store
  elimination.
- **RISC-V**: Follows the same three-phase structure (8/1/4 rounds)
  adapted to its instruction set.
- **i686**: Four-phase structure (8/1/4 rounds plus never-read store
  elimination as a final phase) adapted to the 32-bit x86 instruction set.

---

## GEP Constant Offset Folding

A pre-scan pass in `generation.rs` (`build_gep_fold_map`) identifies
`GetElementPtr` instructions with constant offsets that can be folded
directly into subsequent `Load`/`Store` addressing modes. This eliminates
the GEP instruction entirely and avoids materializing the computed pointer
to a stack slot.

### Eligibility Criteria

A GEP is foldable when all three conditions hold:

1. Its offset is a compile-time constant (`Operand::Const`).
2. The constant fits in a 32-bit signed displacement (the safe common limit
   across x86 disp32, ARM signed 9-bit unscaled / 12-bit scaled, and
   RISC-V signed 12-bit). Unsigned constants that exceed `i32::MAX` but fit
   in `u32` are sign-narrowed.
3. The GEP result is used *only* as the pointer operand of `Load`/`Store`
   instructions -- not as a value operand, call argument, terminator
   operand, or the base of another GEP.

Phase 1 of the map construction collects all GEPs with constant offsets.
Phase 2 verifies that each candidate's result is used exclusively in
foldable positions by scanning all instructions and terminators for
non-pointer uses. GEPs with any non-pointer use, or whose Load/Store uses
involve i128 types or segment overrides, are removed from the map.

### Folding Mechanism

When a GEP is foldable, `generate_function` skips it during instruction
iteration. Each `Load`/`Store` that references the GEP result receives the
`(base, offset)` pair through the `GepFoldInfo` struct and calls
`emit_load_with_const_offset` or `emit_store_with_const_offset`. These
trait methods handle all three `SlotAddr` variants:

- **Direct** (alloca base): the offset is folded directly into the
  frame-pointer-relative slot address. For example, an alloca at `rbp-24`
  with a GEP offset of `+8` becomes a single `movl -16(%rbp), %eax`,
  avoiding a separate `leaq` instruction.
- **OverAligned** (runtime-aligned alloca): the aligned address is computed
  first, then the offset is added to the address register before the
  load/store.
- **Indirect** (non-alloca pointer base): the base pointer is loaded from
  its stack slot into the address register, the constant offset is added
  (if non-zero), and the load/store uses the resulting address.

This optimization yields approximately 5% assembly size reduction on
real-world code like zlib.

### Global Address Folding (x86-64)

A related optimization on x86-64 folds `GlobalAddr` instructions whose only
uses are as `Load`/`Store` pointers into direct `symbol(%rip)` memory
accesses:

- `build_global_addr_map` maps GlobalAddr values (and GEPs derived from
  them) to symbol name strings (e.g., `"myvar"`, `"myvar+8"`).
- `build_foldable_global_addr_set` identifies GlobalAddr values where ALL
  uses are foldable Load/Store pointers, allowing the `leaq symbol(%rip),
  %rax` to be eliminated entirely.
- In kernel code model (`-mcmodel=kernel`), `build_global_addr_ptr_set`
  distinguishes pointer uses (which need RIP-relative addressing) from
  integer uses (which need absolute `R_X86_64_32S` addressing for the
  linked virtual address).

---

## Compare-and-Branch Fusion

When the last instruction in a basic block is a `Cmp` whose boolean result
is used only by the block's `CondBranch` terminator (exactly one use,
detected via `count_value_uses`), the comparison and conditional branch are
fused into a single instruction sequence: `cmp` + `jCC` (x86) / `cmp` +
`b.cc` (ARM) / `bCC` (RISC-V). This avoids materializing the boolean
result to a register or stack slot and then re-testing it.

The fusion is conservative: it excludes i128 and float comparisons (which
have multi-instruction codegen paths) and, on 32-bit targets, I64/U64
comparisons (which require two-word comparison sequences that the fused
path does not support).

---

## SlotAddr: Three-Way Memory Access Dispatch

The `SlotAddr` enum (defined in `state.rs`) captures the three distinct
memory access patterns that appear throughout the codegen:

```rust
pub enum SlotAddr {
    OverAligned(StackSlot, u32),  // Runtime-aligned alloca (alignment > 16)
    Direct(StackSlot),            // Normal alloca: slot IS the data
    Indirect(StackSlot),          // Non-alloca: slot holds a pointer
}
```

`CodegenState::resolve_slot_addr` classifies a value by checking whether it
is an alloca, whether it has over-alignment, and whether it is
register-assigned. Every load, store, GEP, memcpy, and address computation
dispatches on this enum to emit the correct instruction sequence, ensuring
the three patterns are handled uniformly and consistently across the entire
codebase. This eliminates the risk of one codegen path handling allocas
correctly while another forgets the OverAligned case.

---

## Register Value Cache (state.rs)

The `RegCache` tracks which IR value is currently known to reside in the
primary accumulator register (x86: `%rax`, ARM: `x0`, RISC-V: `t0`). When
an instruction produces a result via the accumulator (the common
load-compute-store pattern), the cache records the value ID. If the next
instruction needs that same value as its first operand, `emit_load_operand`
can skip the redundant stack load entirely.

The cache follows a conservative invalidation policy:

- **Invalidated after**: function calls, inline asm, stores through
  pointers, atomic operations, complex multi-register operations, and any
  instruction that might clobber the accumulator in ways the cache cannot
  track.
- **Invalidated at**: basic block boundaries, since a value in a register
  from a predecessor's fall-through is not guaranteed valid when control
  arrives from a different predecessor.
- **Safety property**: a stale entry causes only a redundant load (the same
  behavior as without the cache), while a missing invalidation would produce
  incorrect code by skipping a needed load.

The architecture mapping is:

- x86: accumulator = `%rax`
- ARM64: accumulator = `x0`
- RISC-V: accumulator = `t0`

---

## F128 Soft-Float Framework (f128_softfloat.rs)

ARM and RISC-V lack hardware quad-precision floating point. All F128
operations on these targets go through compiler-rt/libgcc soft-float library
calls (`__addtf3`, `__multf3`, `__fixtfsi`, `__extendsftf2`, etc.). The
orchestration logic -- loading operands to argument positions, calling the
library function, and storing the result -- is identical between the two
architectures; only the register names, instruction mnemonics, and F128
register representation differ.

The `F128SoftFloat` trait captures approximately 48 architecture-specific
primitive methods (each 1--5 instructions), organized into categories:

- **State access**: `f128_get_slot`, `f128_get_source`,
  `f128_resolve_slot_addr`, `f128_is_alloca`, `f128_track_self`,
  `f128_set_acc_cache`, `f128_set_dyn_alloca`.
- **Loading constants**: `f128_load_const_to_arg1`.
- **Loading from memory**: `f128_load_16b_from_addr_reg_to_arg1`,
  `f128_load_from_frame_offset_to_arg1`, `f128_load_operand_and_extend`,
  `f128_load_operand_to_acc`, `f128_load_indirect_ptr_to_addr_reg`,
  `f128_load_from_addr_reg_to_acc`, `f128_load_from_direct_slot_to_acc`.
- **Address computation**: `f128_load_ptr_to_addr_reg`,
  `f128_add_offset_to_addr_reg`, `f128_alloca_aligned_addr`,
  `f128_move_callee_reg_to_addr_reg`, `f128_move_aligned_to_addr_reg`.
- **Storing**: `f128_store_const_halves_to_slot`, `f128_store_arg1_to_slot`,
  `f128_copy_slot_to_slot`, `f128_copy_addr_reg_to_slot`,
  `f128_store_const_halves_to_addr`, `f128_store_acc_to_dest`,
  `f128_store_result_and_truncate`.
- **Argument marshalling**: `f128_move_arg1_to_arg2`,
  `f128_save_arg1_to_sp`, `f128_reload_arg1_from_sp`,
  `f128_move_acc_to_arg0`, `f128_move_arg0_to_acc`.
- **Conversions and comparison**: `f128_truncate_result_to_acc`,
  `f128_cmp_result_to_bool`, `f128_sign_extend_acc`,
  `f128_zero_extend_acc`, `f128_narrow_acc`,
  `f128_extend_float_to_f128`, `f128_truncate_to_float_acc`.

Shared orchestration functions build on these primitives:

- `f128_operand_to_arg1`: load an F128 operand (value or constant) to the
  first argument position, handling all SlotAddr variants.
- `f128_emit_store` / `f128_emit_load`: SlotAddr 4-way dispatch for F128
  store/load.
- `f128_emit_cast`: integer-to-F128 and float-to-F128 casts via libcalls.
- `f128_emit_binop`: F128 arithmetic via libcalls (`__addtf3`, etc.).
- `f128_cmp`: F128 comparison via libcalls.
- `f128_neg`: sign bit flip.

The key architecture difference:

- **ARM**: F128 lives in a single NEON Q register (`q0`/`q1`). Moving
  between argument positions is `mov v1.16b, v0.16b`. Sign bit flip uses
  `mov` + `eor` + `mov` on the high lane.
- **RISC-V**: F128 lives in a GP register pair (`a0:a1` / `a2:a3`). Moving
  between argument positions is `mv a2, a0; mv a3, a1`. Sign bit flip uses
  `li` + `slli` + `xor` on the high register.

On x86, F128 corresponds to x87 80-bit extended precision and is handled
differently: values are loaded/stored via `fldt`/`fstpt`, and arithmetic
uses x87 instructions directly rather than soft-float library calls. The
`CodegenState` tracks F128 load sources (`f128_load_sources`) to enable
full-precision reloading from the original memory location, and
`f128_direct_slots` to identify slots containing full x87 80-bit data.

---

## Inline Assembly Framework (inline_asm.rs)

All four backends share a common 4-phase inline assembly processing
pipeline, orchestrated by `emit_inline_asm_common`:

1. **Classify constraints**: Parse GCC-style constraint strings and assign
   register or memory operands. Specific register constraints (e.g., `"a"`
   for `%rax`, RISC-V specific register names) are assigned first, then
   general constraints (`"r"`) are assigned from a scratch register pool.
   Tied operands (`"0"`, `"1"`) inherit their tied partner's register.
2. **Load inputs**: Load input values from stack slots into their assigned
   registers. Read-write outputs (`"+r"`) are pre-loaded as well.
3. **Template substitution**: Replace `%0`, `%1`, `%[name]` operand
   references in the template string with the assigned register names.
   GCC modifiers are handled: `%b` (byte register), `%w` (word), `%h`
   (high byte), `%P` (raw symbol name), `%c` (constant). Dialect
   alternatives (`{att|intel}`) select the AT&T variant. The x86_common
   module provides `resolve_dialect_alternatives` for this.
4. **Store outputs**: Store output registers back to their destination
   stack slots.

Each backend implements the `InlineAsmEmitter` trait to provide
architecture-specific register classification, constraint-to-register
mapping, sized register naming, and store/load logic.

The `AsmOperandKind` enum covers: `GpReg`, `FpReg`, `Memory`,
`Specific(name)`, `Tied(index)`, `Immediate`, `Address`, `ZeroOrReg`,
`ConditionCode(suffix)`, `X87St0`, `X87St1`, and `QReg` (x86 registers
with accessible high-byte forms).

---

## Codegen Options

The `CodegenOptions` struct (in `mod.rs`) captures all CLI-driven flags
that affect code generation. These are propagated to the backends via
`apply_options` before code generation begins.

| Option | Flag | Description |
|--------|------|-------------|
| `pic` | `-fPIC` | Position-independent code |
| `function_return_thunk` | `-mfunction-return=thunk-extern` | Replace `ret` with retpoline thunk (Spectre v2 / retbleed) |
| `indirect_branch_thunk` | `-mindirect-branch=thunk-extern` | Retpoline indirect calls/jumps |
| `patchable_function_entry` | `-fpatchable-function-entry=N[,M]` | NOP padding + `__patchable_function_entries` for ftrace |
| `cf_protection_branch` | `-fcf-protection=branch` | Intel CET/IBT `endbr64` emission |
| `no_sse` | `-mno-sse` | Avoid SSE in variadic prologues (Linux kernel) |
| `general_regs_only` | `-mgeneral-regs-only` | Avoid FP/SIMD registers (Linux kernel, AArch64) |
| `code_model_kernel` | `-mcmodel=kernel` | Kernel code model, `R_X86_64_32S` relocations |
| `no_jump_tables` | `-fno-jump-tables` | Force compare-and-branch for all switches |
| `no_relax` | `-mno-relax` | Suppress RISC-V linker relaxation |
| `debug_info` | `-g` | Emit DWARF `.file`/`.loc` directives |
| `function_sections` | `-ffunction-sections` | Each function in its own `.text.name` section |
| `data_sections` | `-fdata-sections` | Each global in its own data section |
| `code16gcc` | `-m16` | Prepend `.code16gcc` for 16-bit real mode (Linux boot) |
| `regparm` | `-mregparm=N` | Integer args in registers (i686, 0--3) |
| `omit_frame_pointer` | `-fomit-frame-pointer` | Free EBP as general register (i686) |
| `emit_cfi` | `-f[no-]asynchronous-unwind-tables` | Emit `.cfi_*` directives for `.eh_frame` unwind tables (default: on) |

---

## Builtin Assembler

Each architecture has a native assembler that parses the generated assembly
text into instructions, encodes them into machine code, and writes ELF
object files directly. The builtin assembler is the default. When the
`gcc_assembler` Cargo feature is enabled at compile time, GCC is used
instead.

### Architecture

Each assembler follows a three-stage pipeline:

```text
  Assembly text (String)
       |
       |  Parser (parser.rs)
       v
  Vec<AsmStatement>  (parsed instructions, directives, labels)
       |
       |  Encoder (encoder.rs)
       v
  Vec<u8>  (encoded machine code bytes + relocation entries)
       |
       |  ELF Writer (elf_writer.rs)
       v
  ELF object file (.o)
```

**Parser**: Tokenizes and parses the assembly text into structured
`AsmStatement` items. Each statement is either an instruction (with
opcode, operands, and optional size suffix), a directive (`.section`,
`.globl`, `.byte`, `.long`, `.ascii`, `.align`, etc.), or a label
definition. The x86 and i686 backends share the same AT&T syntax parser
(i686 reuses `x86::assembler::parser`). AArch64 and RISC-V have
architecture-specific parsers.

**Encoder**: Translates parsed instructions into machine code bytes. For
x86-64, this involves REX prefix generation, ModR/M and SIB byte
construction, and displacement/immediate encoding. For i686, REX prefixes
are omitted and the default operand size is 32-bit. For AArch64,
instructions are encoded as fixed-width 32-bit words. For RISC-V,
instructions are 32-bit words with an optional compression pass
(`compress.rs`) that converts eligible instructions to 16-bit RV64C
compact form.

**ELF Writer**: Collects encoded sections (`.text`, `.data`, `.rodata`,
`.bss`, etc.), builds the symbol table and relocation entries, and writes
a complete ELF object file. x86-64 produces ELFCLASS64 with `EM_X86_64`;
i686 produces ELFCLASS32 with `EM_386` using `Elf32_Sym` and `Elf32_Rel`;
AArch64 produces ELFCLASS64 with `EM_AARCH64`; RISC-V produces ELFCLASS64
with `EM_RISCV`.

### Per-Architecture Assembler Details

| Architecture | Parser | Encoder | Extra Features |
|-------------|--------|---------|---------------|
| x86-64 | AT&T syntax, shared | REX prefixes, ModR/M, SIB | SSE/AES-NI encoding |
| i686 | Reuses x86 parser | No REX, 32-bit operands | ELFCLASS32, Elf32_Rel |
| AArch64 | ARM assembly syntax | Fixed 32-bit encoding | imm12 auto-shift |
| RISC-V | RV assembly syntax | Fixed 32-bit encoding | RV64C compression |

### Assembler Files

Each backend's `assembler/` directory contains:

| File | Purpose |
|------|---------|
| `mod.rs` | Entry point: `assemble(asm_text, output_path)` |
| `parser.rs` | Tokenize + parse assembly text (absent in i686; reuses x86) |
| `encoder/` | Instruction-to-bytes encoding (directory with 7--8 submodules per arch) |
| `elf_writer.rs` | ELF object file generation |
| `compress.rs` | RV64C instruction compression (RISC-V only) |

The `encoder/` directory splits encoding by instruction category (e.g., GP
integer, SSE/NEON, FP, system, atomics) to keep each file focused. See
each architecture's assembler README for details.

---

## Builtin Linker

Each architecture has a native ELF linker that reads object files and
static archives, resolves symbols, applies relocations, and writes a
complete ELF executable. The builtin linker is the default. When the
`gcc_linker` Cargo feature is enabled at compile time, GCC is used instead.

### Architecture

The linker pipeline processes input files in this order:

```text
  Input .o files + CRT objects + static archives (.a)
       |
       |  Read ELF headers and sections
       v
  Collected sections, symbols, and relocations
       |
       |  Symbol resolution (strong/weak, local/global)
       v
  Resolved symbol table
       |
       |  Apply relocations (per-architecture relocation types)
       v
  Linked sections with resolved addresses
       |
       |  Write ELF executable (program headers, dynamic section if needed)
       v
  ELF executable
```

### Common Linker Responsibilities

All four linker implementations handle:

- **CRT object discovery**: Locates and includes `crt1.o`, `crti.o`,
  `crtbegin.o`, `crtend.o`, and `crtn.o` from standard system paths
  (probing both native and cross-compilation directories).
- **Static archive processing** (`.a` files): Reads ar-format archives and
  selectively includes object files that define needed symbols.
- **Symbol resolution**: Handles strong vs. weak symbols, local vs. global
  visibility, COMMON symbols, and undefined symbol diagnostics.
- **Section merging**: Merges `.text`, `.data`, `.rodata`, `.bss`, and
  custom sections from multiple input objects.
- **Relocation application**: Applies all architecture-specific relocation
  types to produce position-correct machine code.

### Per-Architecture Linker Details

| Architecture | Link Mode | Key Relocations | Special Features |
|-------------|-----------|-----------------|-----------------|
| x86-64 | Dynamic | R_X86_64_64, PC32, PLT32, GOTPCREL, GOTTPOFF, TPOFF32 | PLT/GOT, TLS (IE-to-LE relaxation), copy relocations |
| i686 | Dynamic | R_386_32, PC32, PLT32, GOTPC, GOTOFF, GOT32X, GOT32 | 32-bit ELF, `.rel` (not `.rela`) |
| AArch64 | Dynamic | ADR_PREL_PG_HI21, ADD_ABS_LO12_NC, CALL26, JUMP26, LDST*, ADR_GOT_PAGE | PLT/GOT, shared library output, IFUNC/IPLT, GLOB_DAT, copy relocations, TLS |
| RISC-V | Dynamic | HI20, LO12_I, LO12_S, CALL, PCREL_HI20, GOT_HI20, BRANCH | Linker relaxation markers |

### Linker Files

Each backend's `linker/` directory has a similar structure. Common files
include `mod.rs` (entry point), `link.rs` (main link driver), and
`input.rs` (input object/archive loading). See each architecture's
linker README for file-level details.

- **x86-64** (8 files): `mod.rs`, `link.rs`, `input.rs`, `types.rs`,
  `elf.rs` (ELF helpers), `plt_got.rs` (PLT/GOT construction),
  `emit_exec.rs`, `emit_shared.rs`
- **i686** (12 files): `mod.rs`, `link.rs`, `input.rs`, `parse.rs`,
  `types.rs`, `reloc.rs`, `emit.rs`, `sections.rs`, `symbols.rs`,
  `shared.rs`, `dynsym.rs`, `gnu_hash.rs`
- **AArch64** (10 files): `mod.rs`, `link.rs`, `input.rs`, `types.rs`,
  `elf.rs`, `reloc.rs`, `plt_got.rs`, `emit_dynamic.rs`,
  `emit_shared.rs`, `emit_static.rs`
- **RISC-V** (10 files): `mod.rs`, `link.rs`, `input.rs`, `elf_read.rs`,
  `relocations.rs`, `reloc.rs`, `sections.rs`, `symbols.rs`,
  `emit_exec.rs`, `emit_shared.rs`

---

## Assembler and Linker Selection (common.rs + mod.rs)

Assembler and linker selection is controlled at **compile time** via Cargo
features, not by environment variables:

| Feature | Effect |
|---------|--------|
| (default, no features) | **Builtin assembler and linker** for all architectures |
| `gcc_assembler` | Use GCC as the assembler instead of the builtin |
| `gcc_linker` | Use GCC as the linker instead of the builtin |
| `gcc_m16` | Use GCC for `-m16` (16-bit real mode boot code) |

When using the GCC fallback, each target provides an `AssemblerConfig`
and `LinkerConfig` that specify the toolchain command, static flags,
and expected ELF `e_machine` value for input validation.

The `CCC_KEEP_ASM` environment variable preserves the intermediate `.s`
file next to the output for debugging.

### Usage Examples

```bash
# Default build: builtin assembler and linker (fully self-contained)
cargo build --release
./target/release/ccc -o output input.c

# Build with GCC assembler and linker fallback
cargo build --release --features gcc_assembler,gcc_linker

# Static linking with builtin linker
ccc -static file.c -o file
```

---

## Switch Statement Compilation

Switch statements use a density-based heuristic (defined in `traits.rs`)
to choose between jump tables and compare-and-branch chains:

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `MIN_JUMP_TABLE_CASES` | 4 | Below this, a linear chain is always faster |
| `MAX_JUMP_TABLE_RANGE` | 4,096 | Larger tables waste memory for sparse switches |
| `MIN_JUMP_TABLE_DENSITY_PERCENT` | 40% | Below this, the table is mostly empty default entries |

When `-fno-jump-tables` is set (required by the Linux kernel with retpoline
to avoid indirect jumps that objtool would reject), all switches use
compare-and-branch chains regardless of density.

---

## Data Section Emission (common.rs)

Global variables are classified into sections via `classify_global` which
returns a `GlobalSection` enum: `Extern`, `Custom`, `Rodata`, `Tdata`,
`Data`, `Common`, `Tbss`, or `Bss`. Each section group is emitted by
internal helpers:

- `emit_section_group`: groups globals by section and emits section
  directives.
- `emit_init_data`: recursively emits all `GlobalInit` variants (integers,
  floats, strings, global addresses, address offsets, label differences,
  compound initializers, zero-fill).
- `emit_symbol_directives`: emits linkage (`.globl`/`.local`) and
  visibility (`.hidden`/`.protected`/`.internal`) directives.
- `emit_zero_global`: emits the zero-init BSS pattern (`.zero N`).

The 64-bit data directive varies by architecture: `.quad` on x86/i686,
`.xword` on AArch64, `.dword` on RISC-V. This is parameterized through the
`PtrDirective` type returned by each backend's `ptr_directive()` method.
