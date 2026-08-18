---
name: reduce
description: Deeply reduce complexity by identifying the abstract recipe underneath the code, the minimal data and state it needs, the right abstraction level for each step, and then challenging every step for necessity, overlap, and placement.
triggers:
  - phrase: "/reduce"
  - phrase: "reduce complexity"
  - phrase: "reduce this"
  - phrase: "minimize this code"
  - phrase: "challenge this code"
---

# Reduce

Complexity reduction framework.

## Purpose

Deeply think and reduce the complexity of the code specified by `$ARGUMENTS` (a file, snippet, or reference from context). This goes past a cleanup pass. It questions whether each step, each piece of state, and each location is justified at all.

Related but distinct:

- **`/improve-readability`** rearranges code for clarity without changing what it does.
- **`/simplify`** cleans up the diff for reuse and efficiency.
- **`/reduce`** asks the harder question: *should this code exist at all, in this shape, in this place?*

## When to Use

1. **Manual invocation**: user types `/reduce <file|code-target>`
2. **Pre-refactor**: before a larger restructuring, to see what can be removed first
3. **Proactive**: user says "reduce complexity", "minimize this", or "challenge this code"

## Workflow

Walk the four analysis steps before proposing any change. Skipping ahead produces cosmetic edits, not reductions.

### Step 1: Identify the Abstract Recipe

Strip the code down to the underlying **core steps** in plain words. Ignore syntax, framework, and naming.

- What is the input?
- What is the output?
- What is the minimal sequence of operations connecting them?
- Write the recipe as 3-7 bullet points. If it takes more, the code is doing more than one thing.

### Step 2: Identify the Minimal Data, State, and Access

For each step in the recipe, list the **minimum** it needs:

- Which fields of which inputs?
- Which pieces of state must persist across steps?
- Which external resources (DB, network, filesystem, globals) does it touch?

Anything carried but unused is a candidate for removal. Anything passed through layers untouched is a candidate for relocation.

### Step 3: Check the Abstraction Level

For each piece of data, state, or access, ask:

- Is it handled at the level where its meaning lives, or one layer too high or too low?
- Does this module own the concept, or is it borrowing it from a neighbor?
- Would the concept be clearer if defined where it is used, or where it originates?

Misplaced abstractions are the #1 source of code that "feels" complex even when each line is simple.

### Step 4: Separate the Steps

Lay the steps out as if they were independent. Make dependencies visible:

- Which steps share state? (a real dependency)
- Which steps share code paths only because they were written together? (a false dependency)
- Which steps could swap order with no behavior change? (no dependency at all)

A step that depends on nothing belongs somewhere else, or nowhere.

### Step 5: Challenge Every Step

For each step from the recipe, ask the four questions in order:

1. **Is this step needed?** What breaks if it's removed? If nothing breaks, delete it.
2. **Can it be simplified with the minimal data?** Strip the step to operate only on what Step 2 said it needs.
3. **Does it overlap another step?** Merge or extract the common core. Two steps doing 80% of the same work usually want to be one.
4. **Is it in the right place?** A step that operates at a different abstraction level than its neighbors belongs in the layer that matches.

### Step 6: Propose Changes

Present the reductions as a short, ordered list. For each:

- The step (or piece of state) involved
- Which question it failed (necessity / minimal data / overlap / placement)
- The concrete change

Do **not** apply changes silently. Propose them, then wait for the user to choose what to apply. A reduction the user disagrees with is worse than no reduction at all.

## Principles

1. **Recipe first, code second.** Never propose a change before the abstract recipe is on the table.
2. **Pass only what the step needs.** Most overcomplication comes from passing more than that.
3. **Wrong-level beats wrong-name.** A misplaced abstraction won't be fixed by renaming.
4. **Deletion is the strongest reduction.** Prefer removing a step to simplifying it.
5. **Propose, don't apply.** The user owns the call on whether the reduction is worth it.

## Output Shape

```markdown
## Recipe

1. <core step>
2. <core step>
3. <core step>

## Minimal data per step

- Step 1 needs: <fields / state / access>
- Step 2 needs: <fields / state / access>
- ...

## Misplacements

- <data/state> handled in <here> but conceptually belongs in <there> because <reason>

## Proposed reductions

1. **<Step>** fails <necessity|minimal-data|overlap|placement>. Change: <concrete>.
2. **<Step>** fails <…>. Change: <concrete>.

## Recommendation

<which 1-2 reductions to apply first and why>
```

## Example

**Input**: `/reduce src/orders/checkout.ts`

**Output**:

````markdown
## Recipe

1. Receive a cart
2. Validate the cart's items are in stock
3. Compute the total
4. Charge the customer
5. Persist an order record

## Minimal data per step

- Validate: item IDs and quantities, not prices, not customer
- Total: item prices and quantities, not stock state
- Charge: customer payment token and total, not items
- Persist: item IDs, total, customer ID, payment receipt

## Misplacements

- `taxRate` is read from cart-builder config but used only in `computeTotal`. It belongs in the pricing module.
- `validateStock` calls `inventoryService.reserve()`, which both checks AND mutates. The mutation belongs in a separate step after charge succeeds, not during validation.

## Proposed reductions

1. **Step 2 (validate)** fails *placement*. The reserve-on-validate behavior couples stock mutation to an early step. Move reservation to a new step after step 4 so a failed charge doesn't orphan reservations.
2. **Step 3 (total)** fails *minimal data*. It currently receives the full cart object. Pass only `items[]`. That removes the need for the function to know about the cart wrapper.
3. **`logCheckoutAttempt` call inside step 2** fails *necessity*. The request middleware already logs it. Delete the duplicate.
4. **`formatLineItems` and `serializeForReceipt`** fail *overlap*. Both walk the same items list and produce 80% the same fields. Merge into one `presentLineItems(items, audience)` taking an audience flag.

## Recommendation

Apply (1) first, because it is the only one with a correctness implication (orphaned reservations on failed charges). Then (4), which removes the most code. (2) and (3) are small cleanups once the bigger shape settles.
````
