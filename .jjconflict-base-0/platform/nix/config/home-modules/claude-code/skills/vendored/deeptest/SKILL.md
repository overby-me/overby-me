---
name: deeptest
description: Audit tests by reading them as documentation and challenging whether each one has a clear purpose, a name that reflects it, and an API-level focus. Targets test suites by default, or a specific file/class chosen by the user. Detects vague names, low-level setup noise, over-mocking, and tests that fail to show how the API is meant to be used.
triggers:
  - phrase: "/deeptest"
  - phrase: "review the tests"
  - phrase: "audit the tests"
  - phrase: "simplify the tests"
  - phrase: "challenge the tests"
---

# Deep Test

Test suite audit.

## Purpose

Audit tests in the target code and challenge whether each one reads like documentation for the API under test. Focus on **test suites exercising public APIs by default**, or narrow to a specific file, class, or layer when the user specifies one. Read each test as if encountering the API for the first time, then ask hard questions:

- **Purpose**: does each test have one clear reason to exist?
- **Naming**: does the test name state the behavior being verified?
- **Abstraction**: does the test exercise the API at the level users call it?
- **Clarity**: can a reader learn how to use the API by reading the test?
- **State transitions**: for stateful subjects, are the actions performed and the resulting state changes made visible through the public API?

## When to Use

1. **Manual invocation**: user types `/deeptest <folder|file|class>` to audit tests around a target, or `/deeptest <target> --scope unit|integration|all` to choose a test layer
2. **Pre-release**: user asks to "review the tests" or "audit the tests"
3. **Proactive**: user says "simplify the tests" or "challenge the tests"
4. **Scoped**: user asks to review specific tests, e.g., "audit the tests for OrderProcessor" or "review tests in cache.spec.ts"

## What It Produces

A structured audit report output directly in the conversation:

```text
## Deep Test: <target>

### Tests (<scope>)
<table of every test in scope with name and the API surface it exercises>

### Purpose Analysis
<per-test: what behavior is being verified and whether the name reflects it>

### Findings
<numbered list of vague names, low-level noise, over-mocking, duplication, and documentation gaps>

### Recommendations
<table: action | before | after | why>
```

## Workflow

### Step 1: Inventory Tests

- Determine the scope: public API tests by default, or the scope the user specified (unit, integration, specific file/class)
- Scan the target for all test cases matching the scope
- List every test: name, file, the API entry point it exercises, setup shape
- Record which tests cluster around the same interface

### Step 2: Map Tests to API Surface

For each test:

- Identify which public method or endpoint is the subject under test
- Record what inputs the test feeds and what outcome it asserts
- Note whether the assertion speaks in domain terms or implementation terms
- Identify tests that share a subject and compare how each phrases its intent

### Step 3: Challenge Purpose

Read each test and ask:

- **One reason to exist?**: does the test verify a single observable behavior, or is it a grab-bag of assertions across unrelated concerns?
- **Named after behavior?**: does the name describe what the API does (e.g. `rejects_orders_with_expired_coupon`) rather than the method called (`test_submit`)?
- **Name matches body?**: does the body actually verify what the name claims, with no surprise assertions?
- **Redundant with a sibling?**: does another test already cover this exact behavior with different inputs?

Flag every test whose purpose is unclear, mislabeled, or duplicated.

### Step 4: Challenge Abstraction Level

Compare each test to how real callers use the API and ask:

- **Tests the API, not the internals?**: does the test call a public entry point, or does it poke at private state, patched internals, or field assignments?
- **Uses the public API the way users would?**: does the arrange/act sequence mirror a realistic caller, or does it set up artificial fixtures that only this test uses?
- **Mock at the seam, not the subject?**: are collaborators mocked at trust boundaries (network, clock, DB) rather than the very class under test?
- **Asserts outcomes, not call logs?**: does the test check observable results or does it verify that internal methods were called in a specific order?

Flag every test that exercises implementation detail instead of behavior.

### Step 5: Challenge State Transitions

For tests against stateful systems (stores, caches, state machines, aggregates, long-lived services), ask:

- **Actions visible?**: does the test spell out each action performed on the subject, in order, so the reader can replay the scenario?
- **State observable through the API?**: is the resulting state read back through a public query rather than by peeking at private fields or the database?
- **Before and after asserted?**: does the test establish the starting state, then assert the state after the action, so the transition is *shown* rather than implied?
- **One transition per test?**: does each test focus on a single action's effect, so a failure points to the exact transition that broke?
- **Setup uses the same API?**: is the starting state built by calling real actions (e.g. `deposit`, `register`) rather than hand-constructing an internal object that bypasses invariants?
- **Illegal transitions covered?**: is there a test showing what happens when an action is invoked in a state that disallows it, and that the state did not silently change?

Flag every stateful test where the reader cannot see which actions were performed or how the state moved as a result.

### Step 6: Challenge Clarity

Read each test as documentation for a new reader and ask:

- **Setup noise?**: is the arrange block dominated by variable assignments that obscure what actually matters? Could a factory, builder, or fixture hide that complexity?
- **Magic values?**: do literals like `42`, `"foo"`, or `true` carry meaning that should be named (`EXPIRED_COUPON`, `ADMIN_USER`)?
- **Act step buried?**: is the single line that calls the API hard to find among setup and assertion code?
- **Assertions tell a story?**: does the assert block read as a clear statement about the API's contract, or is it a pile of low-level field checks?
- **Reads top-to-bottom?**: can the reader follow given/when/then without scrolling, jumping to helpers, or decoding clever DSLs?

Flag every test that a new reader would struggle to learn from.

### Step 7: Challenge Coverage Shape

Look at the full test map for the target and ask:

- **Missing behaviors?**: are there public operations with no test that demonstrates intended usage?
- **Happy-path only?**: are error cases, edge conditions, and boundary values tested at the same interface level as the happy path?
- **Overlapping cases?**: do multiple tests exercise the same branch with trivially different inputs, adding noise but no coverage?
- **Wrong layer?**: is the behavior tested deep in a unit test when it really belongs as an integration or contract test (or vice versa)?

Propose concrete merges, deletions, renames, or additions.

### Step 8: Report

- List all tests in scope in a table with subject and intent
- Summarize each test's purpose in one line
- Number every finding with a severity (purpose / naming / abstraction / clarity / coverage)
- Present recommendations as a table: action, before, after, why

## Principles

1. **Tests are documentation**: a reader should learn how to use the API by reading the tests, in the order a caller would encounter them
2. **Name states the behavior**: the test name is the first line of that documentation; it must describe what is verified, not what is called
3. **One test, one reason to fail**: a test asserts a single behavior so a failure points to one cause
4. **Use the API like a user**: the arrange/act/assert shape must mirror how real callers hold the API, not how the implementation is wired
5. **Hide setup, highlight intent**: push setup into factories, builders, and fixtures so the test body reads as a short story about behavior
6. **Assert outcomes, not mechanics**: verify observable effects, not which internal methods were called in which order
7. **State changes must be shown**: for stateful subjects, tests must make each action explicit and read the resulting state back through the public API; never peek at internals to prove a transition
8. **Evidence-based**: every finding references a concrete test by file and name, not theory

## Examples

**Input**: `/deeptest src/orders`

**Output**:

```text
## Deep Test: src/orders

### Tests (public API)

| Test                                        | Subject             | Intent                              |
|---------------------------------------------|---------------------|-------------------------------------|
| test_submit_1                               | OrderService.submit | unclear, multiple assertions       |
| test_submit_works                           | OrderService.submit | happy path, name says nothing       |
| it_charges_customer_and_reserves_stock      | OrderService.submit | combined behavior, two reasons      |
| rejects_submit_when_cart_is_empty           | OrderService.submit | empty cart guard                    |
| test_internal_validate_called_once          | OrderService.submit | asserts internal call, not outcome  |

### Purpose Analysis

- test_submit_1 / test_submit_works: names hide the behavior; bodies
  assert a mix of totals, stock, and notifications
- it_charges_customer_and_reserves_stock: two behaviors in one test,
  a failure will not tell you which contract broke
- rejects_submit_when_cart_is_empty: clear name, single assertion
- test_internal_validate_called_once: verifies private collaboration,
  would break on any refactor that preserves behavior

### Findings

1. **Naming**: test_submit_1, test_submit_works give no signal about
   the behavior under test; a failing CI log tells the reader nothing
2. **Purpose**: it_charges_customer_and_reserves_stock bundles two
   independent outcomes; split so each failure has one cause
3. **Abstraction**: test_internal_validate_called_once reaches into
   the implementation; replace with an outcome-based assertion on the
   public result
4. **Clarity**: every submit test rebuilds Order and Customer inline
   with 8+ fields; extract an anOrder() builder so the body shows
   only what is specific to each case
5. **Coverage**: no test demonstrates submit with an expired coupon,
   a documented supported input

### Recommendations

| Action  | Before                                         | After                                              | Why                                           |
|---------|------------------------------------------------|----------------------------------------------------|-----------------------------------------------|
| Rename  | test_submit_1, test_submit_works               | submits_order_and_returns_confirmation             | Name states the behavior verified             |
| Split   | it_charges_customer_and_reserves_stock         | charges_customer_on_submit + reserves_stock_on_submit | One reason to fail per test                |
| Replace | test_internal_validate_called_once             | rejects_submit_with_invalid_line_item              | Assert outcome, not internal call             |
| Extract | Inline Order/Customer construction in 5 tests  | anOrder().withItems(...).build() fixture           | Hide setup, highlight what differs per test   |
| Add     | (no coverage)                                  | applies_discount_when_coupon_is_valid + rejects_expired_coupon | Cover documented behavior    |
```

**Input**: `/deeptest lib/cache.spec.ts`

**Output**:

```text
## Deep Test: lib/cache.spec.ts

### Tests (unit)

| Test                                      | Subject                 | Intent                              |
|-------------------------------------------|-------------------------|-------------------------------------|
| cache get returns value                   | Cache.get               | round-trip get/set                  |
| cache set works                           | Cache.set               | duplicate of above                  |
| ttl expires                               | Cache.get after ttl     | expiry behavior                     |
| should call internal _evict on overflow   | Cache internals         | asserts private method call         |
| test getOrSet                             | Cache.getOrSet          | unclear intent                      |

### Purpose Analysis

- cache get returns value / cache set works: both assert the same
  round-trip; one is enough
- ttl expires: clear intent but uses sleep(1100), flaky and slow
- should call internal _evict on overflow: verifies private behavior
  instead of the observable effect (oldest entry gone)
- test getOrSet: name reveals nothing about when to use getOrSet

### Findings

1. **Purpose**: cache get returns value and cache set works verify
   the same behavior with different phrasing; merge
2. **Naming**: test getOrSet does not tell the reader what the
   method guarantees; the test is supposed to be the docs for it
3. **Abstraction**: should call internal _evict on overflow couples
   the test to implementation; the user-visible contract is "oldest
   entry is evicted when capacity is exceeded"
4. **Clarity**: ttl expires uses a real sleep; inject a clock to
   make the test fast, deterministic, and self-documenting about
   how ttl is measured
5. **Coverage**: no test demonstrates getOrSet's cache-aside
   contract (compute on miss, return cached on hit)

### Recommendations

| Action  | Before                                       | After                                                           | Why                                           |
|---------|----------------------------------------------|-----------------------------------------------------------------|-----------------------------------------------|
| Merge   | cache get returns value + cache set works    | returns_value_previously_set                                    | Single round-trip behavior, one test          |
| Rename  | test getOrSet                                | computes_value_on_miss + returns_cached_value_on_hit            | Names document the cache-aside contract       |
| Replace | should call internal _evict on overflow      | evicts_oldest_entry_when_capacity_exceeded                      | Assert the observable effect, not the call    |
| Refactor| ttl expires using real sleep(1100)           | expires_entry_after_ttl_elapses using injected clock            | Deterministic, fast, names the seam           |
| Add     | (no coverage)                                | returns_null_for_unknown_key + overwrites_existing_value        | Document the remaining public contract        |
```

**Input**: `/deeptest src/account` (stateful subject: `Account` aggregate)

**Output**:

```text
## Deep Test: src/account

### Tests (behavior)

| Test                                       | Subject             | Intent                              |
|--------------------------------------------|---------------------|-------------------------------------|
| test_balance                               | Account             | unclear, inspects private field    |
| deposit_and_withdraw                       | Account             | two actions, balance only at end    |
| overdraft_throws                           | Account.withdraw    | rejection, no state check after     |
| test_close                                 | Account.close       | closes but does not verify frozen   |

### Purpose Analysis

- test_balance: sets account._balance = 100 directly, then reads
  account.balance; the transition is never exercised through the API
- deposit_and_withdraw: performs two actions and asserts the final
  balance only; a reader cannot tell which action produced which
  effect, and a regression in deposit could be masked by withdraw
- overdraft_throws: asserts the exception but does not verify that
  the balance is unchanged afterwards, because a silent corruption would
  still pass
- test_close: calls close() but never attempts a follow-up action
  to prove the account actually rejects further operations

### Findings

1. **State visibility**: test_balance bypasses the API by writing
   to _balance directly; the test proves nothing about deposit
2. **Actions visible**: deposit_and_withdraw collapses two
   transitions into one assertion; split so each action's effect
   on balance is shown
3. **Before/after**: overdraft_throws asserts the error but not
   that balance is preserved; the invariant is the whole point
4. **Setup uses API**: tests that need a funded account construct
   it by field assignment instead of calling deposit(), bypassing
   invariants the real code relies on
5. **Illegal transitions**: test_close does not verify that
   deposit/withdraw on a closed account are rejected and leave
   state untouched

### Recommendations

| Action   | Before                                                 | After                                                                                         | Why                                              |
|----------|--------------------------------------------------------|-----------------------------------------------------------------------------------------------|--------------------------------------------------|
| Replace  | test_balance (writes _balance directly)                | deposit_increases_balance: given empty, deposit(100), assert balance == 100                   | Exercise the transition through the public API   |
| Split    | deposit_and_withdraw                                   | deposit_increases_balance + withdraw_decreases_balance                                        | One action per test, each transition visible     |
| Extend   | overdraft_throws                                       | rejects_withdraw_exceeding_balance_and_leaves_balance_unchanged                               | Assert both the rejection and state preservation |
| Refactor | Tests setting _balance directly                        | anAccount().funded(100) builder that calls deposit()                                          | Starting state built through real actions        |
| Add      | (no coverage)                                          | rejects_deposit_on_closed_account + rejects_withdraw_on_closed_account                        | Cover illegal transitions and state invariance   |
```
