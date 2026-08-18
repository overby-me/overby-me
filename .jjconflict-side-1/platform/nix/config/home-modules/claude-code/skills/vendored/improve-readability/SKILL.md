---
name: improve-readability
description: Flatten nested code, reorder by importance, extract inline complexity, and clarify names. Reduces deep indentation, puts main logic first, moves complex inline structures into named variables, and renames unclear identifiers.
triggers:
  - phrase: "/improve-readability"
  - phrase: "improve readability"
  - phrase: "make this more readable"
  - phrase: "clean up this code"
  - phrase: "flatten nesting"
  - phrase: "refactor for clarity"
---

# Improve Readability

Code clarity framework.

## Purpose

Improve the **readability** of the code specified by `$ARGUMENTS`. Apply six targeted transformations that make code easier to scan, understand, and maintain:

- **Flatten nesting**: guard clauses and early exits first, main logic at one indent level
- **Extract inline complexity**: move complex inline data structures and closures into named variables or functions
- **Rename for clarity**: names must be understandable in their surrounding context
- **Group by cohesion**: cluster related lines together, separate groups with blank lines
- **Comment only the why**: only when the reason cannot be expressed as code
- **Order deliberately**: arrange code by what the reader needs to understand first

## When to Use

1. **Manual invocation**: user types `/improve-readability <file|code-target>`
2. **Post-implementation**: user asks to "clean up" or "make this more readable"
3. **Proactive**: user says "flatten nesting", "refactor for clarity", or "improve readability"

## What It Produces

In-place edits that preserve identical behavior:

| Before | After |
|---|---|
| brief description of how the code was | what changed |

## Workflow

### Step 1: Flatten Nesting

- Find any branch whose body exits the surrounding scope (`return`, `continue`, `break`, `throw`, `panic`) regardless of the construct it lives in (`if`, `match`/`switch` arm, `case`, `try/catch`, destructure-with-fallback)
- Lift each terminating branch to a flat guard clause at the base indent of its scope, even when it means inverting a `match` or `switch` into an `if let` / early-exit form
- Test: if you delete the main body after the branch, does the branch still make sense on its own? If yes, it's a guard. Lift it
- Keep main logic at a single indent level

### Step 2: Extract Inline Complexity

- Find complex inline data structures, closures, and deeply nested object literals passed as arguments
- Pull them into named variables or functions above the call
- Prefer a named local over a single-use helper. Promote to a function only when there's a second call site, the name needs to appear in stack traces, or inlining would re-nest the caller
- Make the call site scannable at a glance

### Step 3: Rename for Clarity

- Find single-letter, abbreviated, or ambiguous names
- Rename to be understandable in context
- Match the abstraction level of the surrounding code

### Step 4: Group by Cohesion

- Cluster related lines together
- Separate groups with blank lines
- Add a short comment above a group only if its purpose is not obvious from the code

### Step 5: Comment Only the Why

- Remove comments that restate the code
- Add comments only when the reason cannot be expressed as code
- Never add comments that reference this conversation or the changes being made

### Step 6: Order Deliberately

Apply as the final step after all other changes:

- **Critical configuration and variables**: at the top
- **Core purpose**: the most interesting or important part next, not boilerplate or helpers
- **Supporting details**: utilities and edge cases last
- **Cohesion groups**: in logical reading order

## Principles

1. **Never change logic**: only flatten, extract, rename, group, comment, and reorder
2. **Verify identical behavior**: mentally confirm nothing changes functionally
3. **No conversation references**: never add comments about the refactoring itself
4. **Why over what**: explain design decisions, not code mechanics

## Examples

### Flatten nesting

Bad:

```text
process(order)
  if order exists
    if order.items not empty
      if order.status is pending
        total = calculateTotal(order.items)
        charge(order.customer, total)
        return order
      else
        throw 'already processed'
    else
      throw 'no items'
  else
    throw 'no order'
```

Good:

```text
process(order)
  if not order          -> throw 'no order'
  if items empty        -> throw 'no items'
  if status not pending -> throw 'already processed'

  total = calculateTotal(order.items)
  charge(order.customer, total)
  return order
```

### Flatten nesting: terminating match/switch arms

Bad: the failure arm is a mini state machine buried inside a value-binding match; the success arm is trivial. The reader has to parse both arms to see the control flow:

```text
validated = match result:
  success(value) -> value
  failure(detail) ->
    state.retries += 1
    if state.retries > maxRetries
      throw 'retries exhausted'
    state.messages.push(retryPrompt(detail))
    emit(turnEnd)
    continue
```

Good: the terminating arm becomes a flat guard; the happy path falls through at base indent:

```text
if result is failure(detail)
  state.retries += 1
  if state.retries > maxRetries -> throw 'retries exhausted'

  state.messages.push(retryPrompt(detail))
  emit(turnEnd)
  continue

validated = result.value
```

The tell for this transform: one arm is trivial (just binds the value), the other is substantive with its own branching and exits. The match is disguised control flow. Lift the substantive arm to a guard.

### Rename for clarity

Bad:

```text
calc(d, r, t)
  v = d.filter(x => x.a > t)
  return v.reduce((s, x) => s + x.a * r, 0)
```

Good:

```text
discountedTotal(orders, rate, minAmount)
  eligible = orders.filter(o => o.amount > minAmount)
  return eligible.reduce((sum, o) => sum + o.amount * rate, 0)
```

### Extract inline complexity: inline literal arguments

Bad: the inline literal pushes the call apart vertically; the reader has to scan 7 lines to see what `emit` is being called with:

```text
emit(
  runtime,
  spec,
  {
    kind: 'agentEnd',
    turns: state.turn,
    status: 'completed',
  },
)
```

Good: name the constructed value, then pass the name. The call collapses to one scannable line; the literal's meaning is carried by the variable name:

```text
agentEnd = {
  kind: 'agentEnd',
  turns: state.turn,
  status: 'completed',
}

emit(runtime, spec, agentEnd)
```

Applies whenever a function call's arguments span multiple lines purely because of an inline literal, even a small one. The indented block inside a call is an anonymous value; giving it a name splits "what is this value" from "what do we do with it."

### Extract inline complexity

Bad: data structure and closure inline obscure the call:

```text
schedule({
  task: 'sync',
  retries: 3,
  backoff: { initial: 100, max: 5000, factor: 2 },
  onFailure: (err, ctx) => {
    log(err)
    if ctx.attempt < 3
      ctx.retry()
    else
      alert(ctx.task, err)
  }
})
```

Good: named variable and function make the call scannable:

```text
retryPolicy = { initial: 100, max: 5000, factor: 2 }

syncJob = {
  task: 'sync',
  retries: 3,
  backoff: retryPolicy,
  onFailure: retryOrAlert
}

schedule(syncJob)

retryOrAlert(err, ctx)
  log(err)
  if ctx.attempt < 3 -> ctx.retry()
  else               -> alert(ctx.task, err)
```

### Extract deeply nested object literals

Bad: nested JSON built inline inside a function call:

```text
createClient(
  buildConfig({
    auth: {
      provider: 'oauth',
      credentials: {
        clientId: env.CLIENT_ID,
        clientSecret: env.CLIENT_SECRET,
        scopes: ['read', 'write', 'admin']
      },
      refresh: { enabled: true, interval: 3600 }
    },
    retry: { attempts: 3, backoff: { base: 100, max: 5000 } }
  })
)
```

Good: each concern extracted into a named variable:

```text
scopes = ['read', 'write', 'admin']

credentials = {
  clientId: env.CLIENT_ID,
  clientSecret: env.CLIENT_SECRET,
  scopes: scopes
}

authConfig = {
  provider: 'oauth',
  credentials: credentials,
  refresh: { enabled: true, interval: 3600 }
}

retryPolicy = { attempts: 3, backoff: { base: 100, max: 5000 } }

config = buildConfig({ auth: authConfig, retry: retryPolicy })
createClient(config)
```

### Group by cohesion

Bad: dense wall of loosely related lines:

```text
initApp()
  db = connectDB(config.db)
  cache = createCache(config.cache)
  logger = setupLogger(config.log)
  router = createRouter()
  router.use(authMiddleware)
  router.use(rateLimiter)
  router.mount('/users', userRoutes)
  router.mount('/orders', orderRoutes)
  server = createServer(router)
  server.onError(logger.error)
  server.listen(config.port)
  logger.info('started on ' + config.port)
```

Good: grouped by concern with blank lines between:

```text
initApp()
  db = connectDB(config.db)
  cache = createCache(config.cache)
  logger = setupLogger(config.log)

  router = createRouter()
  router.use(authMiddleware)
  router.use(rateLimiter)
  router.mount('/users', userRoutes)
  router.mount('/orders', orderRoutes)

  server = createServer(router)
  server.onError(logger.error)
  server.listen(config.port)
  logger.info('started on ' + config.port)
```

### Comments

Bad: restating the code:

```text
// set status to active
user.status = 'active'
```

Good: explaining a non-obvious reason:

```text
// prefer reactivation over new account to retain history
user.status = 'active'
```

### Order deliberately

Bad: file level: config buried, helpers before purpose, related pieces scattered:

```text
delay(ms) -> ...

class ProfileCache
  get(id) -> ...
  set(id, data) -> ...
  invalidate(id) -> ...

buildHeaders(token) -> ...

RETRIES = 3
BACKOFF = 100
BASE_URL = '/api/v2'

export fetchProfile(id, token) -> ...
export updateProfile(id, data, token) -> ...

formatProfile(raw) -> ...
```

Good: file level: config at top, then the interesting part, then supporting pieces grouped by what they support:

```text
RETRIES = 3
BACKOFF = 100
BASE_URL = '/api/v2'

export fetchProfile(id, token) -> ...
export updateProfile(id, data, token) -> ...

class ProfileCache
  get(id) -> ...
  set(id, data) -> ...
  invalidate(id) -> ...

formatProfile(raw) -> ...
buildHeaders(token) -> ...
delay(ms) -> ...
```

Bad: class level: internals before the interface, construction mixed with behavior:

```text
class OrderProcessor
  _validateItem(item) -> ...
  _applyDiscount(item, code) -> ...

  constructor(gateway, inventory)
    this.gateway = gateway
    this.inventory = inventory

  _reserveStock(items) -> ...

  submit(order) -> ...
  cancel(orderId) -> ...
```

Good: class level: construction, then public interface, then internals:

```text
class OrderProcessor
  constructor(gateway, inventory)
    this.gateway = gateway
    this.inventory = inventory

  submit(order) -> ...
  cancel(orderId) -> ...

  _validateItem(item) -> ...
  _applyDiscount(item, code) -> ...
  _reserveStock(items) -> ...
```

Bad: function level: concerns scattered, reader can't tell what matters:

```text
deployService(service, env)
  logDir = env.logDir or '/var/log'
  ensureDir(logDir)
  pidFile = logDir + '/' + service.name + '.pid'
  if fileExists(pidFile) -> throw 'already running'
  artifact = buildArtifact(service.source)
  checksum = hash(artifact)
  previous = getRunningChecksum(service.name)
  if checksum == previous -> return 'no changes'
  healthCheck = service.healthCheck or '/health'
  timeout = env.timeout or 30
  stopExisting(service.name)
  instance = startInstance(artifact, env)
  waitUntilHealthy(instance, healthCheck, timeout)
  writePid(pidFile, instance.pid)
  notifyTeam(service.name, checksum)
  return instance
```

Good: function level: config, then guard rails, then the interesting part (build, deploy, verify):

```text
deployService(service, env)
  logDir = env.logDir or '/var/log'
  healthCheck = service.healthCheck or '/health'
  timeout = env.timeout or 30

  pidFile = logDir + '/' + service.name + '.pid'
  if fileExists(pidFile) -> throw 'already running'

  artifact = buildArtifact(service.source)
  checksum = hash(artifact)
  if checksum == getRunningChecksum(service.name) -> return 'no changes'

  stopExisting(service.name)
  instance = startInstance(artifact, env)
  waitUntilHealthy(instance, healthCheck, timeout)

  writePid(pidFile, instance.pid)
  notifyTeam(service.name, checksum)
  return instance
```
