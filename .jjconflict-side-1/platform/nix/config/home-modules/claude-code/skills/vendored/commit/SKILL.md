---
name: commit
description: Detect the repo's existing commit style (tense, casing, prefixes, body shape), then hand off to the commit-name skill to write a matching message and commit. Never adds Claude as co-author.
triggers:
  - phrase: "/commit"
  - phrase: "commit this"
  - phrase: "commit staged"
---

# Commit

Style-matched commit. Delegates to `commit-name`.

## Purpose

`commit-name` writes good commit messages in a fixed house style. This skill adds one step in front: **read the repo's own history**, summarise the convention, then invoke `commit-name` with that style as a constraint so the resulting message fits in.

## When to Use

- User types `/commit` after staging changes
- User says "commit this", "commit staged", or similar

## Workflow

### Step 1: Detect the Style

Run:

```sh
git log -n 20 --pretty=format:'%s%n%b%n---'
```

Identify the dominant pattern across the most recent ~5 commits:

- **Tense**: imperative / past / present
- **Casing**: lowercase, Sentence case, Title Case
- **Prefixes**: Conventional Commits (`feat:`, `fix:`), ticket IDs, scopes, or none
- **Subject length**: typical character count
- **Body**: absent, bullets, prose, or trailers (the *shape*, not the length)
- **Punctuation**: trailing period or not

Write this down as a one-paragraph style brief.

**Match style, not verbosity.** Detection covers tense, casing, prefixes, and body shape. It does **not** license long messages. Even if the history has sprawling multi-line bullets, the new message stays crisp, at most 5 single-line bullets, as enforced by `commit-name`.

### Step 2: Hand Off to commit-name

Invoke the `commit-name` skill and pass the style brief as an override, e.g.:

> Use the commit-name skill. Match this repo's existing style instead of the default: <style brief>. Do not add Claude as co-author.

`commit-name` then composes the message and runs `git commit`.

### Step 3: Verify

After the commit, run `git log -1` and confirm the new subject visually matches its neighbours.

## Principles

1. **Mimic, don't impose.** The repo's style wins over the default, but only for tense, casing, prefixes and shape, never for length.
2. **Crisp beats conforming.** Match the house style without inheriting its verbosity. Bullets stay short even if history's don't.
3. **Delegate.** Composing and committing stays in `commit-name`. This skill only contributes the style detection.
4. **No AI attribution.** Never add `Co-Authored-By: Claude` or similar.
