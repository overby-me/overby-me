---
name: commit-name
description: Generate a short, crisp commit message and commit staged files. Summarizes what changed in imperative mood with a concise subject and brief body.
triggers:
  - phrase: "/commit-name"
  - phrase: "write a commit message"
  - phrase: "commit message"
  - phrase: "commit changes"
---

# Commit Name

Git commit generator.

## Purpose

Generate a commit message from the conversation context and commit staged files. Produce a subject line and a minimal body. The body gives the reader the **why**. The diff already shows the what.

## When to Use

1. **Manual invocation**: user types `/commit-name` after staging changes
2. **Post-task**: user asks to "commit changes" or "write a commit message"
3. **Proactive**: user says "commit this" or "save my work"

## What It Produces

A git commit with the following message format:

```text
<subject>

- <change 1>
- <change 2>
- <change 3>
```

- **Subject**: one short line, max 50 characters, lowercase, imperative mood, no period
- **Body**: max 5 bullets, each a **single line** ≤80 characters, only what is not obvious from the diff. No wrapping, no sub-bullets, no multi-clause sentences. One crisp change per bullet. If a bullet needs a second line, it is doing too much. Split it or cut it.

## Workflow

### Step 1: Read Context

- Review the conversation to identify what was done and why
- Check `git diff --staged` to see what will be committed

### Step 2: Compose Message

- Write a subject line that describes the outcome
- Write up to 3 body bullets summarizing non-obvious changes
- Keep it brief. If the diff speaks for itself, use fewer bullets

### Step 3: Commit

- Run `git commit` with the composed message
- Do **not** add a co-author line

## Principles

1. **Imperative mood**: "add feature", not "added feature"
2. **Outcome over activity**: describe what changed, not what you did
3. **Brevity**: only include what the diff does not already say. Each bullet stays one short line, max 5 total
4. **No decoration**: no scope prefixes, no emojis, no trailing period
5. **No co-author**: do not add coding agent attribution

## Examples

**Input**: User added input validation to a form component

**Output**:

```text
add input validation to signup form

- reject emails without domain
- show inline error on blur
```

**Input**: User fixed a race condition in the cache layer

**Output**:

```text
fix race condition in cache invalidation

- guard concurrent writes with mutex
- add staleness check before serve
```
