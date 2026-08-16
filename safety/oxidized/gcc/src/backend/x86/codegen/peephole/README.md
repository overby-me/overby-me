# x86-64 Peephole Optimizer

Post-codegen assembly-text optimizer that eliminates redundant patterns from the
stack-based code generator. Operates on AT&T-syntax x86-64 assembly, transforming
it through a multi-phase pipeline of local and global optimization passes.

The peephole optimizer is a critical component of the backend: because the code
generator uses an accumulator-based strategy (load from stack, operate, store
back), the generated assembly contains many redundant moves, dead stores, and
unnecessary extensions that this optimizer systematically eliminates.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Pass Pipeline](#pass-pipeline)
3. [Pre-Classification System](#pre-classification-system)
4. [Line Storage](#line-storage)
5. [Phase 1: Iterative Local Passes](#phase-1-iterative-local-passes)
6. [Phase 2: Global Passes](#phase-2-global-passes)
7. [Phase 3: Post-Global Cleanup](#phase-3-post-global-cleanup)
8. [Phase 4: Loop Trampoline Elimination](#phase-4-loop-trampoline-elimination)
9. [Phase 5: Tail Call Optimization](#phase-5-tail-call-optimization)
10. [Phase 5b: Never-Read Store Elimination](#phase-5b-never-read-store-elimination)
11. [Phase 6: Callee-Save Elimination](#phase-6-callee-save-elimination)
12. [Phase 7: Frame Compaction](#phase-7-frame-compaction)
13. [Design Decisions and Tradeoffs](#design-decisions-and-tradeoffs)
14. [Files](#files)

---

## Architecture Overview

The optimizer has two architectural layers:

1. **Pre-classification** (`types.rs`): Every assembly line is classified once
   into a compact `LineInfo` struct with pre-computed metadata -- line kind,
   register references (as a bitmask), stack frame offsets, and extension type.
   All subsequent passes use integer/enum comparisons on these structs, never
   re-parsing the assembly text.

2. **Optimization passes** (`passes/`): A pipeline of 15 distinct pass functions
   organized into 7 phases. Passes range from simple single-line pattern matching (self-move
   elimination) to whole-function analysis (never-read store elimination and
   loop trampoline coalescing).

The core invariant is the **NOP-marking discipline**: eliminated instructions are
marked as `LineKind::Nop` in their `LineInfo` rather than being removed from the
array. This avoids O(n^2) array shifting and preserves index stability across all
passes. The final output is reconstructed by `LineStore::build_result`, which
skips NOP-marked lines.

---

## Pass Pipeline

The entry point is `peephole_optimize(asm: String) -> String`. The full pipeline:

```text
Phase 1: Iterative Local Passes (up to 8 rounds)
    combined_local_pass         (7 merged pattern matchers)
    fuse_movq_ext_truncation    (movq + extension -> single instruction)
    eliminate_push_pop_pairs    (only if local patterns changed or first round)
    eliminate_binop_push_pop    (only if local patterns changed or first round)
        |
        v
Phase 2: Global Passes (exactly once)
    global_store_forwarding     (slot -> register tracking)
    propagate_register_copies   (register copy propagation)
    eliminate_dead_reg_moves    (windowed dead move analysis)
    eliminate_dead_stores       (windowed dead store analysis)
    fuse_compare_and_branch     (cmp+setCC+test+jCC -> jCC)
    fold_memory_operands        (fold stack loads into ALU instructions)
        |
        v
Phase 3: Post-Global Cleanup (up to 4 rounds, only if Phase 2 changed)
    combined_local_pass
    fuse_movq_ext_truncation
    eliminate_dead_reg_moves
    eliminate_dead_stores
    fold_memory_operands
        |
        v
Phase 4: Loop Trampoline Elimination (once)
    eliminate_loop_trampolines
    + Post-trampoline cleanup (up to 4 rounds, same as Phase 3)
        |
        v
Phase 5: Tail Call Optimization (once)
    optimize_tail_calls (call+epilogue+ret -> epilogue+jmp)
        |
        v
Phase 5b: Never-Read Store Elimination (once)
    eliminate_never_read_stores (whole-function analysis)
        |
        v
Phase 6: Callee-Save Elimination (once)
    eliminate_unused_callee_saves
        |
        v
Phase 7: Frame Compaction (once)
    compact_frame (repack callee-saves, shrink subq $N, %rsp)
```

Push/pop elimination is conditioned on local patterns having changed (or being
the first iteration) because push/pop patterns are typically only exposed after
local pattern elimination removes intervening dead code.

Phase 3 runs more passes per iteration than Phase 1 because global passes create
opportunities for dead code elimination and memory folding that Phase 1 does not
need. Post-trampoline cleanup (Phase 4b) uses the same pass set as Phase 3.

---

## Pre-Classification System

### `LineInfo`

Every assembly line is classified into a compact `LineInfo` struct:

```text
LineInfo {
    kind: LineKind,           // What kind of assembly line
    ext_kind: ExtKind,        // Extension elimination classification
    trim_start: u16,          // Byte offset of first non-space character
    has_indirect_mem: bool,   // Has non-%rbp/%rsp/%rip memory references
    rbp_offset: i32,          // Pre-parsed %rbp offset (sentinel if none)
    reg_refs: u16,            // Bitmask of register families referenced
}
```

`LineInfo` is `Copy + Clone` so it can sit in a dense `Vec<LineInfo>` for
cache-friendly iteration across all passes.

### `LineKind`

The 17-variant enum classifies assembly lines into the categories the optimizer
needs:

| Variant | Carries | Description |
|---------|---------|-------------|
| `Nop` | -- | Deleted line (NOP marker) |
| `Empty` | -- | Blank line |
| `StoreRbp` | reg, offset, size | `movX %reg, offset(%rbp)` |
| `LoadRbp` | reg, offset, size | `movX offset(%rbp), %reg` or `movslq` |
| `SelfMove` | -- | `movq %reg, %reg` (identity move) |
| `Label` | -- | `name:` |
| `Jmp` | -- | `jmp label` |
| `JmpIndirect` | -- | `jmpq *%reg` or indirect thunk call |
| `CondJmp` | -- | `je`/`jne`/`jl`/... label |
| `Call` | -- | `call ...` |
| `Ret` | -- | `ret` |
| `Push` | reg | `pushq %reg` |
| `Pop` | reg | `popq %reg` |
| `SetCC` | reg | `setCC %reg` (byte register) |
| `Cmp` | -- | `cmpX`/`testX`/`ucomis*` |
| `Directive` | -- | Lines starting with `.` |
| `Other` | dest_reg | Everything else, with pre-parsed destination |

`StoreRbp` and `LoadRbp` only match general-purpose registers (families 0-15).
XMM/MMX register loads and stores fall through to `Other`, since the
optimizer focuses on scalar GP register operations.

### `ExtKind`

An 18-variant enum supporting the redundant extension elimination pass (1 `None`
default + 6 consumers + 11 producers). Variants are split into two categories:

**Consumer patterns** (extensions that may be redundant):
`MovzbqAlRax`, `MovzwqAxRax`, `MovsbqAlRax`, `MovslqEaxRax`, `MovlEaxEax`,
`Cltq`

**Producer patterns** (instructions that already produce extended results):
`ProducerMovzbqToRax`, `ProducerMovzwqToRax`, `ProducerMovsbqToRax`,
`ProducerMovslqToRax`, `ProducerMovqConstRax`, `ProducerArith32` (32-bit ALU
ops that implicitly zero-extend to 64 bits), `ProducerMovlToEax`,
`ProducerMovzbToEax`, `ProducerMovzwToEax`, `ProducerDiv32`,
`ProducerMovqRegToRax`

### `classify_line`

The single most important function in the module. Called once per assembly line,
it produces a complete `LineInfo` with all metadata needed by every pass:

1. Compute `trim_start` (cached whitespace offset for O(1) trimming).
2. Check for empty lines, labels (trailing `:`), and directives (leading `.`).
3. Move instructions: try `StoreRbp` parse, then `LoadRbp` parse, then
   self-move check, then extension classification.
4. Control flow: classify `jmp`, `jmpq`, conditional jumps, `call`, `ret`.
5. Comparisons: `cmpX`, `testX`, `ucomisd`/`ucomiss`.
6. `push`/`pop`: extract register family.
7. `setCC`: parse condition code and destination register.
8. Default: classify arithmetic extension producers, parse destination
   register (including implicit writers like `cltq`, `cqto`, `div`, `idiv`,
   `mul`, single-operand `inc`/`dec`/`not`/`neg`/`bswap`), compute indirect
   memory flag, rbp offset, and register reference bitmask.

### Register Identification

Registers are identified by family ID (`RegId = u8`), with families 0-15
covering all GP registers across all sizes:

| Family | 64-bit | 32-bit | 16-bit | 8-bit |
|--------|--------|--------|--------|-------|
| 0 | rax | eax | ax | al |
| 1 | rcx | ecx | cx | cl |
| 2 | rdx | edx | dx | dl |
| 3 | rbx | ebx | bx | bl |
| 4 | rsp | esp | sp | spl |
| 5 | rbp | ebp | bp | bpl |
| 6 | rsi | esi | si | sil |
| 7 | rdi | edi | di | dil |
| 8-15 | r8-r15 | r8d-r15d | r8w-r15w | r8b-r15b |

Families 16-23 are MMX (mm0-mm7) and 24-39 are XMM (xmm0-xmm15). These are
recognized by `register_family_fast` but are not tracked by store forwarding
or copy propagation.

The `reg_refs` bitmask uses the 16 least significant bits, one per GP register
family. Computed once by `scan_register_refs` (a single O(n) scan of the line
bytes), it enables O(1) "does this instruction reference register R?" checks
via `reg_refs & (1 << R) != 0`.

---

## Line Storage

### `LineStore`

A zero-allocation line storage design for performance. Instead of splitting
assembly into N individually heap-allocated strings, `LineStore` keeps:

- `original: String` -- the entire assembly text as one contiguous buffer.
- `entries: Vec<LineEntry>` -- one 8-byte entry per line with `(start: u32,
  len: u32)` pointing into `original`.
- `replacements: Vec<String>` -- side buffer for replaced lines (typically
  <1% of total lines).

When a line is replaced, `LineEntry.len` is set to `u32::MAX` as a sentinel,
and `start` indexes into `replacements`. This means most `get(idx)` calls
return a zero-copy slice into the original string, and only modified lines
require allocation.

Construction (`LineStore::new`) runs a single pass over the input, estimating
~20 bytes/line for initial `Vec` capacity. Output reconstruction
(`build_result`) walks all entries, skipping NOP-marked lines, and writes into
a pre-sized output string.

---

## Phase 1: Iterative Local Passes

Phase 1 runs up to 8 iterations of cheap O(n) single-scan passes until
convergence (no pass reports changes).

### Combined Local Pass (`local_patterns.rs`)

A single forward scan merging 7 pattern matchers:

**1. Self-move elimination.** Lines pre-classified as `SelfMove` during
`classify_line` are simply marked as NOP. No string parsing needed.

**2. Reverse-move elimination.** Detects `movq %A, %B` followed by
`movq %B, %A` within 8 lines (skipping NOPs and `StoreRbp`). The second
move is redundant because the value is already in both registers. Any
non-NOP/non-StoreRbp instruction stops the forward search.

**3. Redundant jump to next label.** If `jmp .Label` is followed (skipping
NOPs and empty lines) by `.Label:`, the jump is eliminated -- execution would
fall through to the same place.

**4. Conditional branch inversion.** Detects the pattern:

```text
    jCC .Ltrue
    jmp .Lfalse
.Ltrue:
```

Transforms to `j!CC .Lfalse` followed by `.Ltrue:`, eliminating the
unconditional jump by inverting the condition. Requires the label to match
the conditional branch target exactly.

**5. Adjacent store/load at same offset.** When `StoreRbp{reg=R, offset=N}`
is immediately followed by `LoadRbp{reg=R, offset=N}` with the same register
and size, the load is redundant. (Different-register forwarding is deferred to
global store forwarding.)

**6. Redundant zero/sign extension elimination.** Uses pre-classified `ExtKind`
for O(1) producer/consumer matching. Scans forward up to 10 lines (skipping
NOPs and stores) to find the next real instruction. When a producer instruction
already establishes the value extension that a subsequent consumer instruction
would create, the consumer is eliminated. Examples:

- `movzbl ..., %eax` followed by `movzbq %al, %rax` -- the zero-extension is
  already done by the 32-bit move (x86-64 implicitly zero-extends 32-bit results).
- `addl ..., %eax` followed by `movl %eax, %eax` -- 32-bit arithmetic already
  zero-extends.
- Extended backward scan for `cltq`: if an upcoming `cltq` instruction can be
  proven redundant by a prior sign-extension producer (within 6 lines backward),
  it is eliminated.

**7. Redundant `xorl %eax, %eax` elimination.** Tracks whether `%rax` is known
to be zero (set by a previous `xorl %eax, %eax`). When a second `xorl %eax, %eax`
is encountered while `%rax` is still known-zero, the duplicate is NOP-ed. The
zero-tracking is invalidated by any instruction that writes to `%rax`, crosses
a basic block boundary, or performs a call.

### Movq/Extension Fusion (`local_patterns.rs`)

Fuses `movq %REG, %rax` followed by an extension/truncation into a single
instruction. Six fusion patterns:

| Before | After |
|--------|-------|
| `movq %REG, %rax` + `movl %eax, %eax` | `movl %REGd, %eax` |
| `movq %REG, %rax` + `movslq %eax, %rax` | `movslq %REGd, %rax` |
| `movq %REG, %rax` + `cltq` | `movslq %REGd, %rax` |
| `movq %REG, %rax` + `movzbq %al, %rax` | `movzbl %REGb, %eax` |
| `movq %REG, %rax` + `movzwq %ax, %rax` | `movzwl %REGw, %eax` |
| `movq %REG, %rax` + `movsbq %al, %rax` | `movsbq %REGb, %rax` |

Only NOPs are skipped between the two instructions. REG must not be rax itself.

### Push/Pop Elimination (`push_pop.rs`)

**Push/pop pair elimination.** Finds `pushq %reg` followed within 4 lines by
`popq %reg` (same register). Verifies no intervening instruction modifies the
register. `Call` instructions invalidate caller-saved registers (rax, rcx, rdx,
rsi, rdi, r8-r11) but not callee-saved ones (rbx, r12-r15).

**Binop/push/pop pattern.** Eliminates the 4-instruction pattern:

```text
    pushq %reg
    <load into %reg>          (e.g., movq -N(%rbp), %reg)
    movq %reg, %other
    popq %reg
```

Transformed by redirecting the load's destination from `%reg` to `%other`,
then removing the push, mov, and pop. The `xorq` zero idiom is handled
specially (both operands must be rewritten when the destination changes).

---

## Phase 2: Global Passes

Phase 2 runs each pass exactly once. These passes track state across basic
block boundaries.

### Global Store Forwarding (`store_forwarding.rs`)

Tracks a mapping of `stack_offset -> (register, size)` across the function.
When a `LoadRbp` is encountered, checks if the stack slot already has a known
value in a register.

**Data structures:**

- `SlotEntry` array capped at 64 entries, cleaned when exceeded.
- `SmallVec` (inline 4-element vector, heap-overflows at 5+) for per-register
  offset tracking, avoiding allocation in the common case.
- Pre-computed jump target analysis: if any `JmpIndirect` exists, ALL labels
  are conservatively marked as jump targets.

**Algorithm:**

1. Pre-pass: collect jump targets by scanning all instructions.
2. Forward scan, maintaining slot-to-register mappings:
   - **Label**: If it is a jump target OR preceded by an unconditional jump,
     invalidate all mappings. Otherwise (fallthrough-only label), preserve them.
   - **StoreRbp**: Invalidate any mapping at overlapping offsets, then record
     the new mapping.
   - **LoadRbp**: Look up the mapping. If found with matching size: same-register
     loads are NOP-ed (redundant); different-register loads become reg-to-reg
     moves. Epilogue callee-save restores are excluded to preserve ABI semantics.
   - **Call**: Invalidate caller-saved registers (rax, rcx, rdx, rsi, rdi,
     r8-r11). Callee-saved registers (rbx, r12-r15) survive.
   - **Indirect memory, div/idiv, mul**: Invalidate conservatively.

### Register Copy Propagation (`copy_propagation.rs`)

Maintains a `copy_src[dst] = src` table (16 entries, one per GP register).
For each `movq %src, %dst`, resolves transitive copy chains and propagates
into subsequent instructions.

**Key behaviors:**

- Self-moves detected through transitivity (A copies B, B copies A) are
  NOP-ed.
- Propagation into instructions checks safety: shifts and rotates are excluded
  when propagating into %rcx (implicit operand), and instructions with
  implicit register usage (div/idiv/mul/cltq/cqto/cdq/cbw/rep/repne/cpuid/
  syscall/rdtsc/rdmsr/wrmsr/xchg/cmpxchg/lock) block all propagation.
- Registers rsp (4) and rbp (5) are excluded from copy tracking.
- At basic block barriers, all copies are invalidated.

### Dead Register Move Elimination (`dead_code.rs`)

For each `movq %src, %dst` (reg-to-reg), scans forward within a 24-instruction
window in the same basic block. If `dst` is overwritten before being read, the
move is dead and is NOP-ed.

Distinguishes write-only instructions (mov, lea, set) from read-modify-write
instructions (add, sub, cmov) using `is_read_modify_write`. As a defense-in-depth
measure, after the initial classification check, additionally verifies that the
destination register name does not appear anywhere in the source operand of the
overwriting instruction.

### Dead Store Elimination (`dead_code.rs`)

For each `StoreRbp`, scans forward within a 16-instruction window. If the same
stack slot is fully overwritten by a subsequent store with no intervening read,
the original store is dead.

Byte-range overlap checking handles stores of different sizes: a 4-byte store
at offset -8 is killed by a subsequent 8-byte store at offset -8, but not by
a 1-byte store at offset -8 (which only covers one byte). Multi-byte stores
check sub-byte offsets for intervening references.

Stack slot pattern matching uses `write_rbp_pattern` -- a function that formats
`N(%rbp)` into a stack buffer without `format!()` overhead, measured at ~2.45%
of total compilation time savings.

### Compare-and-Branch Fusion (`compare_branch.rs`)

Fuses the multi-instruction comparison pattern that the code generator emits
for boolean expressions:

```text
    cmpq %rcx, %rax          ; compare
    setl %al                  ; set byte to 0/1
    movzbq %al, %rax         ; zero-extend to 64-bit
    [store/load to/from stack] ; optional spill
    testq %rax, %rax         ; test the boolean
    jne .Label                ; branch on result
```

Transformed to:

```text
    cmpq %rcx, %rax          ; compare
    jl .Label                 ; direct conditional branch
```

The pass handles up to 4 intervening store/load pairs (boolean spills) between
the `setCC` and the `testq`. Each store must have a matching load nearby;
unmatched stores indicate the boolean is read elsewhere and the setCC cannot
be eliminated. The `jne`/`je` at the end determines whether the condition
code is used directly or inverted.

### Memory Operand Folding (`memory_fold.rs`)

Folds `movq -N(%rbp), %scratch; op %scratch, %dst` into `op -N(%rbp), %dst`:

```text
    movq -16(%rbp), %rax     ; load from stack
    addq %rax, %rcx          ; add to destination
```

Becomes:

```text
    addq -16(%rbp), %rcx     ; fold load into ALU
```

**Constraints:**

- Only scratch registers (rax, rcx, rdx) are eligible.
- Only `movq` and `movl` loads (not sign-extending `movslq`).
- The loaded register must be the source operand (first in AT&T syntax).
- No intervening store to the same stack offset.
- Supported ALU operations: `add`, `sub`, `and`, `or`, `xor`, `cmp`, `test`
  (with q/l/w/b suffixes).

---

## Phase 3: Post-Global Cleanup

If Phase 2 made any changes, Phase 3 runs up to 4 iterations of:
`combined_local_pass`, `fuse_movq_ext_truncation`, `eliminate_dead_reg_moves`,
`eliminate_dead_stores`, and `fold_memory_operands`. This cleans up
opportunities that global passes exposed (e.g., store forwarding may create
dead stores; copy propagation may create self-moves).

---

## Phase 4: Loop Trampoline Elimination

The most complex pass (`loop_trampoline.rs`). Handles SSA codegen artifacts
where phi elimination creates "trampoline" blocks to shuffle register values
on loop back-edges.

### The Problem

After phi elimination, a loop like:

```c
for (int i = 0; i < n; i++) sum += arr[i];
```

generates a back-edge trampoline:

```asm
.LOOP:
    movq %r9, %r14        ; copy loop var to temp
    addq $8, %r14         ; modify temp
    jne .TRAMPOLINE
.TRAMPOLINE:
    movq %r14, %r9        ; shuffle temp back to loop var
    jmp .LOOP
```

The trampoline exists because phi elimination cannot modify the loop variable
in-place (the old value is still needed until the back-edge). This creates
unnecessary register copies and an extra jump on every iteration.

### The Solution

The pass detects single-predecessor trampoline blocks (labels with exactly one
branch reference), identifies the register shuffle pattern, and coalesces by:

1. Finding the initial copy (`movq %dst, %src`) in the loop body where the
   loop variable was copied to the temporary.
2. Rewriting all modifications between the copy and the branch to operate on
   the original register instead of the temporary.
3. Removing the trampoline block entirely and redirecting the branch to the
   loop header.

The result operates on the loop variable directly:

```asm
.LOOP:
    addq $8, %r9          ; modify loop var in-place
    jne .LOOP             ; branch directly to loop header
```

**Safety verification:** Before coalescing, the pass checks that neither
the source nor destination register is read between the branch and the next
write (following up to 2 jumps on the fall-through path). SetCC instructions
(partial byte-register writes) cause the pass to bail out entirely. Stack-load
patterns in trampolines (`movq -N(%rbp), %rax; movq %rax, %dst`) are detected
but not currently coalesced; only register-to-register shuffle moves are
rewritten.

After trampoline elimination, another round of Phase 3-style cleanup runs to
clean up any dead code exposed.

---

## Phase 5: Tail Call Optimization

Converts `call TARGET; <epilogue>; ret` sequences into `<epilogue>; jmp TARGET`
(`tail_call.rs`). This eliminates the overhead of creating a new stack frame for
functions whose last action is calling another function and returning its result.

**Algorithm:**

1. Track function boundaries via global labels and `.cfi_startproc` directives.
2. For each function, scan for patterns that make tail calls unsafe:
   - `leaq offset(%rbp), %reg` or `leaq offset(%rsp), %reg` (address-of-local)
   - `subq %reg, %rsp` (dynamic stack allocation from `__builtin_alloca`)
3. For each `call` instruction, check if the subsequent instructions form a pure
   epilogue sequence: callee-save restores (`LoadRbp`), frame teardown
   (`movq %rbp, %rsp`), `popq %rbp`, and `ret`. No instruction between the call
   and ret may write to `%rax` (which carries the return value).
4. If the pattern matches and the function has no unsafe stack usage:
   NOP the `call` and replace the `ret` with `jmp TARGET`.

**Safety:** The pass must NOT apply when:

- The called function might receive a pointer to a local variable. After frame
  teardown, such pointers become dangling. Detected by checking for `leaq` with
  `(%rbp)` or `(%rsp)` memory operands.
- The function uses dynamic stack allocation (`__builtin_alloca`). Alloca'd memory
  lives below `%rsp`; after frame teardown (`movq %rbp, %rsp`), the tail-called
  function's stack frame may overlap and clobber it. Detected by checking for
  `subq %reg, %rsp`.
If either pattern is found, all tail call optimization is suppressed for that function.

This optimization is critical for threaded interpreters (like wasm3) that use
indirect tail calls (`call *%r10` -> `jmp *%r10`) to dispatch between opcode
handlers without overflowing the stack.

---

## Phase 5b: Never-Read Store Elimination

A whole-function analysis pass (`dead_code.rs`) that removes stores to stack
slots that no instruction ever reads.

**Algorithm:**

1. Find function boundaries via prologue pattern (`pushq %rbp; movq %rsp, %rbp;
   subq $N, %rsp`) and `.size` directive.
2. Collect all "read" byte ranges from the function body: `LoadRbp` offsets and
   sizes, plus `Other` instructions with rbp-offset references (using a
   conservative 32-byte size for unclassified instructions).
3. If any `leaq` takes the address of a stack slot, or any unclassified rbp
   reference exists, bail out of the entire function (the address has escaped
   and reads cannot be tracked).
4. For each `StoreRbp` in the function body, check if its byte range overlaps
   with any collected read range. If not, the store is dead and is NOP-ed.

---

## Phase 6: Callee-Save Elimination

Removes unused callee-saved register saves and restores
(`callee_saves.rs`).

**Algorithm:**

1. Find the function prologue and collect callee-saved register saves
   (moves to negative rbp offsets) for rbx, r12, r13, r14, r15.
2. For each saved register, scan the function body for any reference using
   the pre-computed `reg_refs` bitmask (O(1) per line).
3. Find corresponding epilogue restores near `ret` or `jmp` terminators.
4. If no body reference exists and the restore is found: NOP both the save
   and all restores.

**Safety note:** This pass does not shrink the stack frame (`subq $N, %rsp`)
or relocate the remaining callee-save offsets. It only NOPs saves and restores
for registers that are never referenced. The subsequent Phase 7 (frame
compaction) handles repacking the surviving saves and reducing the frame size.

---

## Phase 7: Frame Compaction

Repacks the stack frame after dead store and callee-save elimination create
gaps (`frame_compact.rs`). Earlier phases may NOP callee-saved saves or dead
stores, leaving unused holes in the frame layout. This pass reclaims that
space by packing the surviving callee-saved saves tightly and shrinking the
frame allocation.

**Algorithm:**

1. Find the function prologue (`pushq %rbp; movq %rsp, %rbp; subq $N, %rsp`)
   and collect the callee-saved register saves immediately after.
2. Scan the function body to find the deepest negative `%rbp` offset that is
   actually **read** (loads, memory operands in ALU instructions). Store-only
   offsets (never read) do not need frame space.
3. Pack the surviving callee-saved saves tightly below the body area. The first
   save goes at `-new_frame_size`, the next at `-new_frame_size + 8`, etc.
4. Rewrite `subq $N, %rsp` to the new smaller size (16-byte aligned).
5. Rewrite all callee-saved save offsets in the prologue and restore offsets in
   epilogues to their new positions.
6. NOP dead stores whose byte ranges fall entirely below the body read area,
   since after relocation those stores would clobber the relocated callee-save
   slots.

**Safety constraints:**

- Bails out if any `leaq ... (%rbp)` takes the address of a stack slot, since
  address arithmetic would break if offsets change.
- Bails out if any unrecognized `%rbp` reference exists in the function body.
- Only relocates callee-saved save/restore offsets; body-area offsets are
  unchanged since the body occupies the same `[%rbp - body_size, %rbp)` region
  before and after compaction.

---

## Design Decisions and Tradeoffs

### Text-Based Optimization

The optimizer works on assembly text rather than a structured IR. This is
deliberate: it catches patterns from the accumulator-based code generator that
would be harder to eliminate at the IR level. The code generator uses %rax as
a primary accumulator with stack-slot temporary storage, producing sequences
like `load -> operate -> store -> load -> operate -> store` that the peephole
optimizer compresses into register-to-register operations.

### Pre-Classification for Performance

`classify_line` runs once per line and extracts ALL metadata needed by every
pass. The result is a compact `LineInfo` in a dense vector. Hot pass loops
use integer comparisons on `LineKind` variants and bitmask checks on
`reg_refs`, never touching the assembly text. This design makes the per-line
cost of each pass nearly zero for lines that don't match the pass's pattern.

### Zero-Allocation Line Storage

`LineStore` keeps the original assembly as one contiguous `String` and uses
8-byte `(offset, length)` entries to reference line boundaries. Only replaced
lines (typically <1% of total) allocate new strings. This reduces per-line
heap allocation from thousands of malloc/free calls to near zero, improving
cache locality and reducing allocation overhead.

### Register Reference Bitmask

The `reg_refs: u16` field is computed once during classification via
`scan_register_refs` (a single O(n) byte scan). It encodes which of the 16 GP
register families appear anywhere in the instruction. This enables O(1)
`line_references_reg_fast(info, reg)` checks that replace what would otherwise
be O(n * patterns) `str::contains` calls across all passes.

### Conservative Safety

The optimizer consistently errs on the side of correctness:

- Indirect memory accesses (`has_indirect_mem`) invalidate all tracked state
  in store forwarding and copy propagation.
- Inline assembly (detected by semicolons in the line), `rep` prefixes, and
  privileged instructions (`rdmsr`, `cpuid`, `rdtsc`, `syscall`) trigger full
  invalidation.
- Indirect jumps cause ALL labels to be marked as jump targets.
- Non-numeric labels (rare, from certain codegen patterns) trigger conservative
  jump target handling.
- Division/modulo instructions are special-cased to invalidate rdx (implicit
  high result register).

### Performance-Critical Formatting

`write_rbp_pattern` formats `N(%rbp)` strings directly into a stack buffer
without using Rust's `format!()` macro. This was measured at approximately
2.45% of total compilation time savings, since dead store elimination calls
this function in a tight inner loop for every candidate store.

### SmallVec for Store Forwarding

The store forwarding pass uses a custom `SmallVec` (inline 4-element storage,
heap overflow at 5+) to track which stack offsets each register maps to. The
common case is 1-2 offsets per register, so this avoids heap allocation on the
hot path.

---

## Files

| File | Lines | Description |
|------|------:|-------------|
| `mod.rs` | ~25 | Module root; re-exports `peephole_optimize` |
| `types.rs` | ~1275 | `LineInfo`, `LineKind`, `ExtKind`, `MoveSize`, `LineStore`, `classify_line`, register tables, utility functions |
| `passes/mod.rs` | ~1200 | Pass pipeline orchestrator (`peephole_optimize` entry point) + unit tests |
| `passes/helpers.rs` | ~290 | Shared utilities: register rewriting, label parsing, epilogue detection, instruction analysis |
| `passes/local_patterns.rs` | ~490 | Phase 1: combined local pass (7 merged patterns) + movq/extension fusion |
| `passes/push_pop.rs` | ~175 | Phase 1: push/pop pair elimination + binop/push/pop pattern |
| `passes/store_forwarding.rs` | ~390 | Phase 2: global store-to-load forwarding across fallthrough labels |
| `passes/copy_propagation.rs` | ~230 | Phase 2: register copy propagation across basic blocks |
| `passes/dead_code.rs` | ~400 | Phase 2+5b: dead register moves, dead stores (windowed), never-read stores (whole-function) |
| `passes/compare_branch.rs` | ~170 | Phase 2: compare-and-branch fusion |
| `passes/memory_fold.rs` | ~150 | Phase 2: fold stack loads into ALU memory operands |
| `passes/loop_trampoline.rs` | ~520 | Phase 4: SSA loop backedge trampoline coalescing |
| `passes/tail_call.rs` | ~420 | Phase 5: tail call optimization (call+epilogue+ret -> epilogue+jmp) |
| `passes/callee_saves.rs` | ~160 | Phase 6: unused callee-saved register save/restore elimination |
| `passes/frame_compact.rs` | ~320 | Phase 7: stack frame compaction after dead store/callee-save elimination |
