---
name: change-summary
description: Generate a concise change summary from the conversation context. Focuses on features and APIs, not implementation details. Produces a copy-pasteable markdown summary.
triggers:
  - phrase: "/change-summary"
  - phrase: "change summary"
  - phrase: "summarize changes"
  - phrase: "merge request summary"
  - phrase: "PR description"
---

# Change Summary

Change description generator.

## Purpose

Generate a change summary that states **what changed and why** in the words the project already uses. Describe features and interfaces, not implementation.

The reader is a reviewer who has not seen the conversation and has not read the diff. Every bullet must stand on its own.

## When to Use

1. **Manual invocation**: user types `/change-summary` before opening a merge request
2. **Post-feature**: user asks to "summarize changes" or "write a PR description"
3. **Proactive**: user says "I need a summary for this branch"

## What It Produces

A markdown summary wrapped in a 4-backtick code block so inner 3-backtick fences for each example render correctly when pasted:

`````text
````markdown
## <title: max 60 chars, imperative mood>

### Purpose

- <3-5 bullets: the problem, what the change enables, why it matters>

### What changed

- <one bullet per feature or interface change, in project vocabulary>

### Examples

```<lang>
<example 1: short snippet showing usage of the new or changed interface>
```

```<lang>
<example 2: another snippet, different scenario>
```
````
`````

Each inner example MUST be wrapped in its own triple-backtick fence with a language tag (e.g. `http`, `json`, `bash`, `ts`, `py`). Do not paste raw snippets without fences, because they will not render as code.

## Workflow

### Step 1: Collect the Project's Vocabulary

Before writing a single bullet, gather the terms the project already uses. A reviewer who searches the codebase for your wording must find it.

- Read the diff for the names the change introduces or touches: types, functions, endpoints, config keys, CLI flags, error messages
- Read the README and any docs headings for the domain nouns
- Read `git log --oneline -30` for how past changes were phrased
- IMPORTANT: If a concept already has a name in the project, use that name everywhere in the summary. Never introduce a synonym, and never rename a concept mid-summary

### Step 2: Analyze Context

- Review the conversation to identify features implemented and problems solved
- Check git log and diffs to understand the full scope of changes
- Identify public interface changes, new endpoints, or user-facing behavior

### Step 3: Draft Summary

- Write a title that names the outcome, max 60 characters, imperative mood
- Write 3-5 purpose bullets: the problem, what the change enables, why it matters
- List each feature or interface change as one bullet
- Write 2-3 snippets showing usage of new or changed interfaces

### Step 4: Format Output

- Wrap the entire summary in a 4-backtick fence tagged `markdown` (` ````markdown `)
- Wrap each example snippet in its own 3-backtick fence with the appropriate language tag (`http`, `json`, `bash`, `ts`, `py`, etc.)
- Never leave example code as bare text. Every snippet must be inside a fenced block
- Verify the summary is under 30 lines
- Ensure examples are copy-pasteable and show the public interface

### Step 5: Check Every Bullet

Reject and rewrite any bullet that fails one of these:

1. States exactly one fact a reviewer can verify against the diff
2. Names a project term, marked in backticks when it is an identifier
3. Runs 14 words or fewer
4. Contains no word from the Plain style's banned vocabulary
5. Carries no adjective that cannot be checked, such as "fast", "robust", "clean", "simple"

## Bullet Form

- No trailing periods
- Purpose bullets state a problem or a gained capability, never the work performed

DO NOT list files, internal method names, or refactoring notes. The reviewer reads the diff for those, and the summary loses its purpose as a standalone description.

## Bad and Good Bullets

Bad (rejected):

```text
- Refactored the auth stuff to be more robust
```

(Names no interface, no verifiable fact, two unverifiable adjectives.)

Good:

```text
- `/auth/refresh` invalidates the old refresh token when it issues a new one
```

Bad (rejected):

```text
- Wired up pagination and made the list endpoints way faster
```

(Two facts in one bullet, informal wording, unmeasured claim.)

Good:

```text
- Every list endpoint accepts `cursor` and `limit`
- `limit` defaults to 25 and is capped at 100
```

## Principles

1. **Outcome over work**: the title names what the reader gets, not what was done
2. **Project vocabulary**: every term in the summary appears in the code or docs
3. **One fact per bullet**: verifiable against the diff, in 14 words or fewer
4. **No internals**: no file lists, no refactoring notes, no internal method names
5. **Copy-pasteable examples**: show the public interface, not implementation
6. **Brevity**: entire summary under 30 lines

## Examples

**Input**: "Summarize the auth feature I just built"

**Output**:

`````markdown
````markdown
## Add JWT authentication with refresh token rotation

### Purpose

- API clients need secure access to protected endpoints
- Server-side sessions do not scale across multiple instances
- Stateless tokens remove the need for shared session storage
- Rotation limits how long a stolen refresh token stays valid

### What changed

- `POST /auth/login` returns an access token and a refresh token
- `POST /auth/refresh` issues a new pair and invalidates the old refresh token
- Protected routes require an `Authorization: Bearer` header
- Requests without a valid access token receive `401`

### Examples

```http
POST /auth/login
{ "email": "a@b.com", "password": "..." }
```

```json
{ "access_token": "...", "refresh_token": "..." }
```

```http
GET /api/profile
Authorization: Bearer <access_token>
```
````
`````

**Input**: "PR description for the pagination changes"

**Output**:

`````markdown
````markdown
## Add cursor-based pagination to list endpoints

### Purpose

- Offset pagination skips rows, so response time grows with the page number
- Cursor pagination keeps query time constant on large tables
- Clients keep a stable position when rows are inserted during paging
- A capped `limit` prevents oversized responses

### What changed

- Every list endpoint accepts `cursor` and `limit`
- Responses carry `next_cursor`; an absent value marks the last page
- `limit` defaults to 25 and is capped at 100

### Examples

```http
GET /api/users?limit=10
```

```json
{ "data": [...], "next_cursor": "abc123" }
```

```http
GET /api/users?limit=10&cursor=abc123
```

```json
{ "data": [...], "next_cursor": "def456" }
```
````
`````
