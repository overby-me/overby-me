---
name: propose
description: Present 1-3 design options as concrete code so the user can feel the choice before any implementation begins. Works for code APIs and architectural designs. For naming requests, scans project conventions and returns a ranked list of 3-10 candidate names with a pick recommendation.
triggers:
  - phrase: "/propose"
  - phrase: "propose options"
  - phrase: "design proposal"
  - phrase: "what are the options"
  - phrase: "show me a few options"
  - phrase: "propose a name"
  - phrase: "propose an architecture"
---

# Propose

Concrete design options.

## Purpose

Before committing to a direction, present 1-3 options as **code**. Let the snippet carry the design; strip the prose. The reader should be able to pick by reading the artifacts alone.

## Modes

The artifact changes by what's being proposed; the structure does not.

- **Code / API** (default): usage snippets showing how the caller would write it
- **Architecture**: a structural sketch: an ASCII diagram or file tree. Triggered by "propose an architecture", "how should we structure…", "service boundaries", "data flow"
- **Naming**: a ranked list of candidate names. Triggered by "propose a name", "what should we call…", "rename…", "name ideas", "suggest a name"

If the phrasing is ambiguous, pick the matching mode; don't ask unless the modes diverge wildly.

## When to Use

1. **Manual invocation**: user types `/propose <design question>`
2. **Pre-implementation**: "what's the best way to..." or "how should I design..."
3. **Naming**: "what should we call this" or "propose a name for…"
4. **Architecture**: "how should this be structured" or "where should X live"
5. **Proactive**: when there are multiple viable shapes and you'd otherwise pick one silently

Skip if there's only one reasonable option or the design is trivial.

## What It Produces

**Code / Architecture**: 1-3 options. Each option is:

- A numbered **label** that names its character (`Option N: <what it is>`, not just "Option N")
- A **code artifact**: the usage snippet (Code) or sketch (Architecture). This is the whole option.

No tradeoff sentences. No supporting paragraphs. If a cost matters, put it in a one-line `// comment` inside the snippet, never a prose block.

Close with **Pick:**, one line giving the option and the deciding factor. Nothing more.

**Naming**: a plain bullet list of 3-10 candidates, no notes:

```text
## Names for <thing>

- `candidateName`
- `anotherName`
- `yetAnotherName`

**Pick:** `recommendedName`, deciding factor.
```

## Workflow

### Step 1: Frame

One line naming what's being designed, only if it isn't obvious from the artifacts. If the question is ambiguous, ask before drafting.

### Step 2: Anchor in Stack Idioms

Identify the language, framework, and libraries in play. Each option must be **idiomatic to that stack**.

- React → hooks and composition before class wrappers
- Go → small interfaces, errors as values, no exceptions
- Rust → traits and ownership before runtime checks
- TypeScript → discriminated unions before string enums
- Python → context managers, dataclasses, async if already async

Check the repo for existing patterns first. If the codebase already solves similar problems a certain way, match it unless there's a real reason to diverge.

### Step 3: Generate Distinct Shapes

Options must differ on a meaningful axis, not cosmetics.

*Code:* positional args vs options object vs builder · sync vs streaming vs callback · free function vs class vs module · explicit config vs convention vs inference · one general interface vs several specific ones

*Architecture:* monolith vs split services · sync call vs queue vs event stream · shared database vs per-service storage · library vs sidecar vs service

*Naming:* grep for casing, prefix/suffix families (`xFor`, `xBy`, `handle*`, `on*`), abbreviation policy. Match existing patterns unless asked to diverge. Generate 3–10 candidates on a real axis (verb-first vs noun-first, domain vs generic, suffix family vs bare name).

Cap at 3 for code/architecture. If more come to mind, drop the weakest.

### Step 4: Show the Code

Lead each option with its code artifact: the caller's snippet (Code) or the diagram/tree (Architecture). This is the option. No framing prose around it.

**Naming mode** ends here: a plain bullet list of candidates, then the **Pick** line.

### Step 5: Pick

One line. The option and the deciding factor. Leaving the choice open pushes the work back to the user.

## Principles

1. **Code, not prose**: the snippet is the proposal; everything else is noise
2. **Costs go in comments**: if a tradeoff matters, it's a one-line `//` inside the snippet, never a paragraph
3. **Idiomatic to the stack**: every option looks native; match repo patterns before importing outside ones
4. **Distinct shapes**: options differ on a real axis, not cosmetics
5. **At most three**: more options dilute the decision (naming: 3–10 candidates)
6. **Always pick**: one line naming the option and the deciding factor

## Examples

**Input**: `/propose how should the new HTTP client handle retries?`

**Output**:

````markdown
## HTTP retry handling

### Option 1: Inline per call

```ts
const user = await client.get('/users/1', {
  retry: { attempts: 3, backoff: 'exponential' }, // repeated at every call site
})
```

### Option 2: Configured at construction

```ts
const client = new HttpClient({
  retry: { attempts: 3, backoff: 'exponential' }, // one policy per client
})

const user = await client.get('/users/1')
```

### Option 3: Default with per-call override

```ts
const client = new HttpClient({ retry: { attempts: 3 } })

const user = await client.get('/users/1')
const report = await client.get('/reports/heavy', { retry: { attempts: 0 } })
```

**Pick:** Default with override. Most calls stay terse; outliers opt out without a second client.
````

**Input**: `/propose API for feature flag checks in TypeScript`

**Output**:

````markdown
## Feature flag API

### Option 1: Single function, string key

```ts
if (isEnabled('checkout.v2', { userId })) { // typos silent until runtime
  renderCheckoutV2()
}
```

### Option 2: Generated flag constants

```ts
import { Flags } from './flags.gen' // needs a codegen step in the build

if (Flags.CheckoutV2.isEnabled({ userId })) {
  renderCheckoutV2()
}
```

**Pick:** Generated constants if codegen already runs; otherwise string key with a lint rule pinning valid names.
````

**Input**: `/propose a name for the function that resolves a user's effective permissions across roles and overrides`

**Output**:

````markdown
## Names for permission resolution function

- `effectivePermissionsFor`
- `resolvePermissions`
- `computeEffectivePermissions`
- `permissionsFor`
- `mergedPermissionsFor`

**Pick:** `effectivePermissionsFor`, which names the concept and fits the `rolesFor` / `overridesFor` suffix family.
````

**Input**: `/propose an architecture for the new ingestion pipeline`

**Output**:

````markdown
## Ingestion pipeline architecture

### Option 1: Single service, in-process stages

```text
[ HTTP intake ] → [ validate → enrich → persist ]  (one service)
                                     ↓
                                 Postgres
// slow enrichment blocks intake; no failure isolation
```

### Option 2: Queue between intake and processing

```text
[ HTTP intake ] → SQS → [ worker: validate → enrich → persist ] → Postgres
// intake acks fast, workers scale; a queue to operate
```

### Option 3: Stage-per-service with a queue between each

```text
[ intake ] → Q1 → [ validate ] → Q2 → [ enrich ] → Q3 → [ persist ] → Postgres
// each stage scales/fails alone; three services + three queues to own
```

**Pick:** Queue between intake and processing. It decouples intake from variable enrichment cost without the tax of stage-per-service; split further later if one stage bottlenecks.
````

**Input**: `/propose how to expose pagination on the list endpoints`

**Output**:

````markdown
## Pagination shape

### Option 1: Offset + limit

```http
GET /api/users?offset=40&limit=20
```

```json
{ "data": [], "total": 1284 }
```

### Option 2: Cursor

```http
GET /api/users?limit=20&cursor=eyJpZCI6NDB9
```

```json
{ "data": [], "next_cursor": "eyJpZCI6NjB9" }
```

**Pick:** Cursor. The list endpoints are append-mostly feeds; nobody needs random-access paging and the perf curve stays flat.
````
