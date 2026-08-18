---
name: deepcomment
description: Challenge every comment in the target and keep only the ones the code cannot say for itself. Deletes comments that restate the code, rewrites "what" into "why", tightens the survivors, and preserves warnings, contracts, and provenance. Governing rule: a comment must not outweigh the code it describes. Edits in place; defaults to the current unstaged changes.
triggers:
  - phrase: "/deepcomment"
  - phrase: "review the comments"
  - phrase: "audit the comments"
  - phrase: "challenge the comments"
  - phrase: "clean up the comments"
  - phrase: "are these comments useful"
---

# Deep Comment

Comment audit.

## Purpose

A comment earns its place only when it says something the code cannot. A comment that restates the code is worse than no comment: it is extra to read, and it drifts out of sync the moment the code changes, at which point it actively lies.

So challenge every comment against one test. *Could a competent reader derive this from the code itself?* If yes, cut it. If no, it is carrying real information; keep it, and tighten it to just that. The result is code where the few remaining comments are all signal.

Governing rule, above all others: **a comment must not outweigh the code it describes.** A three-line comment over a one-line statement is a defect regardless of how well written it is.

One-shot on a target the user points at. Edit in place, report what changed, then answer normally.

## Target

Resolve in this order:

1. **File / class / function given** → challenge the comments there, edit in place.
2. **Code pasted or quoted in the message** → return the cleaned version in chat, nothing else.
3. **No target** → the comments in the current unstaged/uncommitted changes (`git diff`). The default is to review the comments the user just wrote.

Comment-only edits are low-risk and git holds the recovery path, so edit in place and report every deletion for review via `git diff`. For a wholly untracked file with no committed baseline, show the result and confirm before overwriting.

## The Test: Does This Comment Earn Its Place?

Run every comment through this classifier.

**Keep, because the code cannot say it.** Preserve these, byte-exact where you can:

- **Why / intent**: the reason behind a choice the code cannot express: why this algorithm over the obvious one, why this order, why this workaround exists.
- **Warning / constraint / gotcha**: a non-obvious precondition, easy misuse, invariant, thread-safety note, or ordering requirement. Cutting one reads as permission to break it.
- **Contract / API doc**: a public docstring documenting params, returns, and errors for callers who will never read the body.
- **Provenance**: a link to a ticket/spec/RFC, the source of a magic number, a citation. The credibility lives in the reference.

**Cut or convert, because the reader can derive it:**

- **Restates the code** → delete. The line below already says it.
- **Describes "what"** → rewrite as *why*, or make the code self-documenting (rename, extract) and drop the comment entirely.
- **Outweighs its code** → tighten to the one thing the reader cannot derive.
- **Drifted / obsolete** → contradicts the code → delete. A wrong comment costs more than a missing one.
- **Dead commented-out code** → delete. Git holds the history.
- **Noise** → decorative section-dividers, changelog and attribution lines, ownerless TODOs, comments referencing this conversation or the edit being made.

## Workflow

### Step 1: Inventory

Read the target and list every comment: its location, its text, and the code it annotates. Include inline comments, block comments, docstrings, doc-comments, and file/module headers.

### Step 2: Classify

Run each comment through the test above. For each, decide: keep, tighten, convert to why, replace with better code, or delete. Say why.

### Step 3: Challenge the "What"

Delete pure restatements. For a comment that names *what* the code does, ask whether the reader needs *why* instead. If so, rewrite it. If the "what" was only unclear because of a poor name, fix the name or extract a well-named piece and drop the comment. The best comment is often no comment.

### Step 4: Challenge Length

Apply the governing rule to every survivor: **a comment must not outweigh the code it describes.** Cut a survivor down to the single fact the reader cannot reconstruct. If a comment is longer than the code it sits over, either the comment is padded or the code needs the explanation broken into the design itself.

### Step 5: Preserve What the Code Cannot Say

Warnings, contracts, provenance, and negations stay. Never trade a safety note or a precondition for brevity. When in doubt about a warning, keep it verbatim.

### Step 6: Apply and Report

Edit in place. Close with the report below. Do not narrate tool calls while working.

## Principles

1. **Comment the why, never the what.** The code already shows what it does.
2. **Cut what the reader can derive; keep what would surprise them.** Predictable text carries no information.
3. **A comment must not outweigh the code it describes.** Length is a defect on its own.
4. **The best comment is often better code.** A rename or an extraction beats a paragraph.
5. **A drifted comment is worse than none.** Delete on contradiction.
6. **Never delete warnings, contracts, provenance, or negations.** A flipped or missing constraint costs far more than any line saved.
7. **Edit in place; git is the undo.** Report every change.
8. **Evidence-based.** Every cut points at a concrete comment at its location.

## Report

Close with a compact report, and nothing else:

```text
Comments: 14 → 5. Reviewed src/orders/checkout.ts.
Deleted: 8 restatements, 1 commented-out block.
Tightened: 3 down to the why.
Kept untouched: the overdraft precondition on withdraw(), the CVE link on sanitize().
```

Name every comment you preserved, so the user can see the safety-critical ones survived.

## Examples

**Delete restatement.** The comment says nothing the line below doesn't:

```text
// increment the counter
counter++
```

→

```text
counter++
```

**Convert what into why.** The "what" is obvious; the "why" is not:

```text
// retry three times
for (let i = 0; i < 3; i++) { ... }
```

→

```text
// the gateway drops the first connection after an idle period, so retry covers the reconnect
for (let i = 0; i < 3; i++) { ... }
```

**Tighten an overweight comment.** Four lines of prose over one statement:

```text
// We use a Set here instead of an array because we need to check
// membership frequently and arrays would be O(n) for each lookup,
// which would make the whole loop O(n^2) and too slow for large
// inputs, so a Set gives us O(1) membership checks instead.
const seen = new Set()
```

→

```text
// Set, not array: membership is checked per-iteration, so keep it O(1)
const seen = new Set()
```

**Keep the warning verbatim.** No brevity is worth blurring a precondition. This survives untouched:

```text
// Must run AFTER migrate(): reads columns that migrate() adds.
// Reordering silently corrupts existing rows.
backfill(db)
```

**Delete commented-out code.** Git holds the history:

```text
// const legacyRate = 0.05
// return amount * legacyRate
return amount * taxRate(region)
```

→

```text
return amount * taxRate(region)
```

**Replace the comment with self-documenting code.** The comment exists only to decode a bad name:

```text
// d is the number of days until the subscription expires
if (d < 7) notify(user)
```

→

```text
if (daysUntilExpiry < 7) notify(user)
```
