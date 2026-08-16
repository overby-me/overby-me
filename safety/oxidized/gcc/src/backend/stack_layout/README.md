# stack_layout/

Stack frame layout: determines which IR values get stack slots and where.

## Why this exists

Every backend needs to assign stack slots for local variables (allocas) and
SSA temporaries that don't fit in registers. A naive approach gives every
value its own 8-byte slot, but this wastes hundreds of bytes per function
on kernel/PostgreSQL code where macro expansion creates thousands of
short-lived intermediates.

This module implements a three-tier allocation scheme that typically reduces
stack usage by 40-60% compared to naive allocation.

## Three-tier allocation

- **Tier 1 (Permanent)**: Allocas get dedicated, non-shared slots. They're
  addressable memory -- the program may take their address.

- **Tier 2 (Liveness-packed)**: Multi-block SSA temporaries share slots based
  on live interval overlap. Uses greedy interval coloring with a min-heap.

- **Tier 3 (Block-local)**: Single-block values use intra-block greedy slot
  reuse. Values whose lifetimes don't overlap within a block share a slot.
  Different blocks' pools overlap since only one block executes at a time.

## Module layout

| File | Purpose |
|------|---------|
| `analysis.rs` | Value use-block maps, used-value collection, dead param alloca detection |
| `alloca_coalescing.rs` | Escape analysis: determines which allocas can share slots |
| `copy_coalescing.rs` | Copy alias map and immediately-consumed value optimization |
| `slot_assignment.rs` | The core phases: instruction classification, tier assignment, slot packing |
| `inline_asm.rs` | Inline assembly clobber register scanning |
| `regalloc_helpers.rs` | Register allocator integration and callee-saved register filtering |

## Key design decisions

- **Escape analysis for allocas**: An alloca whose address never escapes
  (isn't stored, passed to calls, or used in pointer arithmetic beyond GEP)
  can be demoted to Tier 3 block-local sharing. This is the biggest win for
  kernel code where many allocas are single-block temporaries.

- **Copy coalescing**: When a `Copy` instruction is the sole use of its source,
  the destination shares the source's stack slot. This eliminates redundant
  slots created by phi elimination.

- **Immediately-consumed optimization**: When value V is defined at instruction I
  and consumed at instruction I+1 as the first operand, the accumulator register
  cache keeps it alive without needing a stack slot at all.

- **Architecture independence**: The `assign_slot` closure abstracts over
  stack growth direction (x86 grows down, ARM/RISC-V grow up). All tier
  logic is shared across backends.

## Pipeline (7 phases)

1. **Build context**: Use-block maps, def-block tracking, copy aliases,
   alloca coalescability, immediately-consumed analysis
2. **Classify instructions**: Sort each value into Tier 1/2/3
3. **Tier 3 assignment**: Block-local greedy slot reuse
4. **Tier 2 assignment**: Liveness-based interval packing
5. **Finalize deferred**: Convert block-local offsets to absolute offsets
6. **Copy alias resolution**: Propagate slots from roots to aliases
7. **Wide value propagation**: 32-bit target multi-word copy tracking
