# Mem2Reg -- SSA Promotion

## Overview

The mem2reg subsystem promotes stack-allocated local variables (`alloca` instructions) into
SSA virtual registers. Without this pass, every C local variable lives in memory: each
assignment is a `Store` and each read is a `Load`, with the alloca pointer threading through
them. After mem2reg, those memory operations disappear and the values flow directly between
instructions via SSA values and phi nodes. This is the single most important enabling
transformation in the optimizer -- constant folding, copy propagation, GVN, and LICM all
depend on the SSA form that mem2reg produces.

The subsystem has two distinct passes that bracket the optimization pipeline:

| Pass | Entry point | When it runs |
|------|-------------|--------------|
| **SSA construction** | `promote_allocas` | After IR lowering, before inlining (skips parameter allocas) |
| **SSA construction** | `promote_allocas_with_params` | After inlining (promotes all allocas including parameters) |
| **Phi elimination** | `eliminate_phis` | After all SSA optimizations, immediately before codegen |

SSA construction converts memory-form IR into SSA-form IR by removing loads and stores and
inserting phi nodes. Phi elimination later converts the SSA phi nodes back into explicit
`Copy` instructions that the backend can lower to register moves.

---

## SSA Construction Algorithm

The implementation follows the classic textbook algorithm from Cytron et al., using the
Cooper-Harvey-Kennedy dominator algorithm as a modern, efficient foundation. The pass is
structured as six steps applied to each function independently.

### Step 1: Identify Promotable Allocas

An alloca is promotable if and only if **all three** conditions hold:

1. **Scalar size.** Both the alloca's allocation size and its IR type size must be at most
   `MAX_PROMOTABLE_ALLOCA_SIZE` (8 bytes). This corresponds to the width of a
   general-purpose register on a 64-bit target. Larger allocas represent arrays or structs
   that cannot live in a single register.

2. **Not address-taken.** The alloca pointer must appear only as the `ptr` operand of `Load`
   and `Store` instructions (and as the output pointer of non-memory-constraint `InlineAsm`
   instructions). If the alloca's `Value` appears anywhere else -- as a function call
   argument, as the *value* operand of a `Store`, as a `GEP` base, as an `InlineAsm` input,
   or in a terminator -- its address has escaped and the alloca must remain in memory.

3. **Not volatile.** Allocas marked `volatile: true` (corresponding to C `volatile` locals)
   are never promoted. Volatile locals must reside in memory so their values survive
   `setjmp`/`longjmp` and are not cached in registers that `longjmp` would restore to stale
   values.

**Inline assembly special cases.** Register-constraint outputs (`"=r"`, `"=a"`, etc.) are
treated as definitions of the alloca and are compatible with promotion. Memory-constraint
outputs (`"=m"`, `"=o"`, `"=V"`, `"=p"`) are *not* compatible: the inline asm template writes
directly to the alloca's stack address, so the alloca must keep its stack slot. Input operands
that reference an alloca's `Value` (as opposed to a loaded value) indicate that the alloca's
*address* is being passed to the assembly, which disqualifies it.

**Entry-block vs. non-entry-block allocas.** The C front-end places all local variable allocas
in the entry block. After inlining, however, the inlined function's entry block (containing
its own allocas) becomes a non-entry block in the caller. The pass scans all blocks for
allocas so that inlined locals are also promoted.

### Step 2: Build CFG

The function's control-flow graph is constructed from block terminators and `InlineAsm`
goto edges, producing predecessor and successor adjacency lists. These adjacency lists are
the foundation for dominator computation and phi placement in the subsequent steps.

### Step 3: Compute Dominator Tree

The immediate dominator of every block is computed using the Cooper-Harvey-Kennedy algorithm
("A Simple, Fast Dominance Algorithm", 2001). This iterative dataflow algorithm processes
blocks in reverse postorder, converging in two passes for reducible CFGs and gracefully
handling irreducible ones.

### Step 4: Compute Dominance Frontiers

For each block *b*, the dominance frontier DF(*b*) is the set of blocks where *b*'s dominance
"ends" -- join points reachable from *b* where another path exists that *b* does not dominate.
These are exactly the points where phi nodes may be needed. The implementation walks
predecessors of each join point upward through the dominator tree until reaching the join
point's immediate dominator, accumulating frontier entries along the way.

### Step 5: Insert Phi Nodes with Cost Limiting

For each promotable alloca, the pass computes the **iterated dominance frontier** of its
defining blocks (blocks containing stores to the alloca). Starting from the set of defining
blocks, it iteratively adds dominance frontier blocks to a worklist until a fixed point is
reached. A phi node is placed at every block in this iterated frontier.

**Phi cost limiting.** Each phi node with *P* predecessors generates approximately *P* copy
instructions during phi elimination. For functions with large multi-way branches (e.g., a
`switch` with 84 cases in a VM dispatch loop), promoting many allocas can produce
O(cases x allocas) copies. The pass estimates the total copy cost and, if it exceeds the
`MAX_PHI_COPY_COST` threshold (50,000), excludes the most expensive allocas from promotion,
leaving them as stack variables. This keeps the generated code within reasonable bounds
(roughly 400 KB of copy instructions at 8 bytes per slot).

### Step 6: Rename Variables via Dominator-Tree DFS

The final step rewrites the IR in a single depth-first traversal of the dominator tree. Each
alloca maintains a **definition stack** -- a stack of SSA `Operand` values representing the
most recent definition visible at the current point in the traversal.

At each block, the renamer:

1. **Pushes phi destinations.** For each phi inserted in this block, the phi's destination
   `Value` is pushed onto the corresponding alloca's definition stack.

2. **Rewrites instructions.** `Load` instructions from promoted allocas are replaced with
   `Copy` instructions that read the current top of the definition stack. `Store` instructions
   push the stored value onto the stack and are then removed. `InlineAsm` register-constraint
   outputs receive fresh SSA values that are pushed onto the stack.

3. **Fills successor phis.** For each successor block, the renamer appends the current
   definition (top of stack) as an incoming value to each phi in that successor.

4. **Recurses into dominator-tree children.** After processing a block, the renamer visits
   all blocks that it immediately dominates.

5. **Pops on exit.** When returning from a block, all definition stack entries pushed during
   that block's processing are popped, restoring the state for the parent's next child.

**Uninitialized variables.** If a load occurs before any store (reading an uninitialized
variable), the definition stack's initial entry provides a zero constant of the appropriate
type. Reading an uninitialized local is undefined behavior in C, but using zero is a
practical choice that provides predictable behavior and avoids exposing stale stack data.

**Constant narrowing.** The IR front-end produces `I64` constants for all integer literals.
When storing to a narrower alloca (e.g., `I32`), the renamer narrows the constant to match
the alloca's type. This prevents phi copies from using 64-bit operations for 32-bit values,
which would leave high bits uninitialized on some paths.

**Asm-goto snapshots.** An `InlineAsm` with `goto_labels` creates implicit control-flow edges
from the asm instruction to its label targets. Any definitions produced *after* the asm goto
in the same block (e.g., the asm's own output values) must not be visible along the goto
edge. The renamer snapshots the definition stacks before processing asm goto outputs and uses
those snapshots when filling in phi incoming values for goto targets and when recursing into
dominator-tree children that are goto targets.

**Cleanup.** After renaming completes, the pass removes all promoted `Alloca`, `Store`, and
`Load` instructions from the IR. Parameter allocas in the entry block are retained even if
promoted, because the backend's `find_param_alloca` relies on their positional indexing.

---

## MAX_PROMOTABLE_ALLOCA_SIZE

```rust
const MAX_PROMOTABLE_ALLOCA_SIZE: usize = 8;
```

This constant is the maximum byte size for an alloca to be eligible for promotion. It
corresponds to the width of a general-purpose register: 8 bytes on 64-bit targets, which is
the primary target architecture. An alloca that fits within this size is a scalar (integer,
pointer, float, or double) and can reside in a single register. Allocas larger than this
represent aggregates (arrays, structs passed by value) that require memory and cannot be
expressed as a single SSA value.

Both the alloca's allocation size *and* the IR type size are checked. An alloca may have a
larger allocation than its type (e.g., a 4-byte `I32` padded to 8 bytes for alignment), and
both must fit within the register width.

---

## Two Invocations in the Pipeline

### Invocation 1: Before Inlining (`promote_allocas`)

Called in the driver immediately after IR lowering, before any optimization passes run. This
invocation **skips parameter allocas** (the first *N* allocas in the entry block, where *N* is
the number of function parameters). Parameter allocas are left in place because the inliner
assumes they exist: it maps caller arguments to callee parameter allocas via `Store`
instructions when splicing the inlined body.

This early promotion converts most local variables to SSA form, enabling the optimizer to
work on SSA values from the start.

### Invocation 2: After Inlining (`promote_allocas_with_params`)

Called in `run_inline_phase` immediately after inlining completes. This invocation **includes
parameter allocas**, which are now safe to promote because:

- Inlining has already completed and no longer needs the parameter allocas.
- The IR lowering emits explicit `ParamRef` + `Store` instruction sequences that make
  parameter initial values visible as stores, so the renamer can track them.
- Inlined function bodies introduce new allocas in non-entry blocks that need promotion for
  the inlined parameters to flow as SSA values into constant folding and other passes.

Parameter allocas that have no IR-visible stores (e.g., `sret` return-value pointers or
struct parameters whose values are populated by backend-level `emit_store_params`) are
excluded from this promotion, since their definitions are not represented in the IR.

---

## Phi Elimination Pass

After all SSA optimizations have run and before backend code generation, the phi elimination
pass (`eliminate_phis`) converts every `Phi` instruction into explicit `Copy` instructions
placed in predecessor blocks. The backend does not understand phi nodes; it needs sequential
copy instructions it can lower to register moves.

### Non-Conflicting Phis (Common Case)

When a block has a single phi, or when multiple phis have no interference between their
sources and destinations on a given predecessor edge, the elimination is straightforward.
Each phi becomes a direct `Copy` placed at the end of the predecessor block, before its
terminator:

```text
pred_block:
    ... existing instructions ...
    %dest1 = copy src1
    %dest2 = copy src2
    <terminator to target_block>
```

Self-copies (where the source equals the destination) are elided entirely.

### Conflicting Phis: The Lost-Copy Problem

When a block has multiple phis and, on some predecessor edge, one phi's source is another
phi's destination, a naive copy sequence would produce incorrect results. The classic example
is a swap pattern:

```text
target_block:
    %a = phi [%b, pred], ...
    %b = phi [%a, pred], ...
```

Emitting `%a = copy %b; %b = copy %a` in `pred` would read the already-overwritten `%b` for
the second copy. This is the *lost-copy problem*.

**Conflict detection.** For each predecessor edge, the pass builds a copy graph: `(dest_i,
src_i)` for each phi. A phi needs a temporary if its source is the destination of another phi
on the same edge (and it is not a self-copy). A conservative safety net also marks phis whose
destinations are read by a conflicting phi, ensuring correctness for chains and multi-way
cycles like `a = b, b = c, c = a`.

**Two-phase copy protocol.** Conflicting phis use shared temporary variables and a two-phase
sequence:

- **Phase 1 (predecessor block):** Save conflicting sources into temporaries.

  ```text
  pred_block:
      %tmp_a = copy %b    // save before overwrite
      %tmp_b = copy %a    // save before overwrite
      <terminator>
  ```

- **Phase 2 (target block, prepended):** Restore from temporaries into final destinations.

  ```text
  target_block:
      %a = copy %tmp_a
      %b = copy %tmp_b
      ... rest of block ...
  ```

Non-conflicting phis on the same edge still emit direct copies in Phase 1, avoiding
unnecessary temporaries. Temporaries are **globally allocated** -- if a phi is conflicting on
*any* predecessor edge, it uses a temporary on *all* edges for that phi, so the Phase 2
restore copies in the target block are always valid.

### Critical Edge Splitting

A **critical edge** is an edge from a block with multiple successors to a block with multiple
predecessors. If the predecessor has a `CondBranch` to two targets and we place copies at the
end of the predecessor, those copies execute on *all* outgoing paths, corrupting values
intended for only one successor.

The pass resolves this by inserting a **trampoline block**: a new basic block containing only
the phi copies and an unconditional `Branch` to the original target. The predecessor's
terminator is retargeted to the trampoline instead:

```text
Before:                          After:
  pred (multi-succ)                pred (multi-succ)
    |         \                      |         \
    v          v                     v          v
  target    other              trampoline     other
                                   |
                                   | (copies here)
                                   v
                                 target
```

Trampoline block labels are allocated from a module-wide counter to avoid collisions across
functions. Trampoline creation is memoized per `(predecessor, target)` pair so that multiple
phis targeting the same block through the same critical edge share a single trampoline.

**Exception: `IndirectBranch`.** Indirect branches (computed gotos) cannot have their targets
retargeted because the target is a runtime value. For these edges, copies are placed directly
in the predecessor block. This is safe in practice because C computed gotos (`goto *ptr`)
typically branch to a single logical target at a time.

---

## File Inventory

| File | Purpose |
|------|---------|
| `mod.rs` | Module declaration and public re-exports of `promote_allocas`, `promote_allocas_with_params`, and `eliminate_phis`. |
| `promote.rs` | SSA construction: alloca identification, phi insertion via iterated dominance frontiers, and variable renaming via dominator-tree DFS. |
| `phi_eliminate.rs` | Phi elimination: conflict detection, two-phase copy emission, critical edge splitting via trampoline blocks, and final IR rewriting. |

The SSA construction pass depends on shared infrastructure in `ir::analysis` for CFG
construction, dominator computation (Cooper-Harvey-Kennedy), and dominance frontier
calculation. The phi elimination pass is self-contained, performing its own successor
analysis and conflict detection.

---

## Design Decisions and Tradeoffs

**Iterated dominance frontier vs. on-the-fly SSA.** The implementation uses the classic
Cytron-style iterated dominance frontier algorithm rather than Braun et al.'s on-the-fly
approach. The IDF algorithm is a good fit because the IR is fully constructed before mem2reg
runs, and the dominator tree / dominance frontiers are straightforward to compute with the
Cooper-Harvey-Kennedy algorithm. The IDF approach also naturally supports the phi cost
limiting heuristic, which examines all phi placement sites before committing.

**Phi cost limiting.** Rather than unconditionally promoting every eligible alloca, the pass
estimates total copy cost and drops the most expensive allocas when the threshold is exceeded.
This is a pragmatic defense against pathological cases (e.g., large `switch` dispatch loops
with many promoted variables) that would otherwise produce an explosion of copy instructions
during phi elimination. The threshold of 50,000 copies balances optimization opportunity
against code size.

**Shared temporaries across edges.** The phi elimination pass allocates temporaries globally
(per-phi, not per-edge). If a phi is conflicting on any predecessor edge, it gets a temporary
on all edges. This slightly over-approximates -- some edges may not actually need the
temporary -- but it simplifies the implementation considerably: the Phase 2 restore copies in
the target block can be emitted once, unconditionally, rather than requiring per-edge
dispatch logic. The cost is a few extra copy instructions on non-conflicting edges, which
downstream copy coalescing in the backend typically eliminates.

**No copy propagation after phi elimination.** After phi elimination the IR is no longer in
SSA form. The `Copy` instructions from phi elimination represent moves at specific program
points, and propagating through them could change semantics (e.g., reading a value before it
is defined in a loop iteration). Instead, copy coalescing is deferred to the backend's
register allocator.

**Parameter alloca two-phase promotion.** Splitting promotion into pre-inlining (skip params)
and post-inlining (include params) is a deliberate coordination point with the inliner. The
inliner maps caller arguments to callee allocas via stores to parameter slots; if those slots
were already promoted to SSA, the inliner would have no target for its argument-passing
stores. After inlining, the parameter allocas are just ordinary allocas with explicit stores
and can be promoted normally.

**Conservative conflict detection.** The conflict analysis in `find_conflicting_phis` uses a
safety net that marks additional phis beyond those strictly involved in cycles. Specifically,
if phi *i* is conflicting and its source is phi *j*'s destination, then *j* is also marked.
This over-approximation ensures correctness for chain patterns (`a = b, b = c`) and complex
multi-way rotations at the cost of a few extra temporaries. Correctness is prioritized over
minimal temporary count.

**Trampoline blocks vs. conventional edge splitting.** Rather than splitting critical edges as
a separate CFG transformation, trampolines are created on demand during phi elimination. This
avoids modifying the CFG for blocks that have no phis and keeps the transformation local and
self-contained. Each trampoline is a minimal block (copies + unconditional branch) that the
backend can easily fall through or fold.
