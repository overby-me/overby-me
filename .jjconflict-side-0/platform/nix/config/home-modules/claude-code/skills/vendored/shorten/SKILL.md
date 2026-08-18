---
name: shorten
description: "Compress text by deciding what the reader actually needs: extract the core claims, rank them, drop what is derivable or inert, then rewrite the survivors crisp. Three levels: tight (nothing lost), hard (default, lossy by design), core (ruthless). Applies to a file (rewritten in place), pasted text, or prior conversation context."
triggers:
  - phrase: "/shorten"
  - phrase: "shorten this"
  - phrase: "make this shorter"
  - phrase: "condense this"
  - phrase: "tighten this text"
  - phrase: "cut the filler"
  - phrase: "what can be cut here"
---

# Shorten

## Purpose

Two things make text long: words that carry no meaning, and meaning the reader does not need. Cutting only the first buys little. Most bloated prose is not bloated because of the word "basically". It is bloated because it explains what the reader can already derive, repeats a pattern five times when one instance established it, narrates how the current state came to be, and buries its conclusion under the derivation that produced it.

So compress content first, prose second. Content decisions lose information on purpose; that is the point, and the reason the levels below exist.

One-shot on a target the user points at, not a persistent output mode. After the rewrite, answer normally.

## Levels

| Level | Invoke | Runs | Result |
|---|---|---|---|
| **tight** | `/shorten tight` | Step 4 only | Nothing is lost. Every claim, every section, every fact survives. |
| **hard** | `/shorten` (default) | Steps 0–4 | Lossy. Derivable and inert content is dropped, order is rebuilt. Full sentences and byte-exact headings stay. |
| **core** | `/shorten core` | Steps 0–4, ruthless | Very lossy. Only claims that change a decision survive, in whatever form is densest: fragments, bullets, tables, merged or dropped headings. |

`tight` is the level for text that must stay complete: a spec, a contract, a checklist someone works through line by line.

## Target

Resolve in this order:

1. **File path(s) given** → rewrite in place.
2. **Text pasted or quoted in the message** → return the shortened version in chat, nothing else.
3. **No target** → the most recent substantial prose in the conversation: the last summary, doc, plan, or explanation. If two candidates are equally plausible, ask instead of guessing.

Before overwriting a file at `hard` or `core`: if it is git-tracked and committed, rewrite in place, because git is the undo. If it is untracked, or its current state is uncommitted, show the result and confirm first. A lossy rewrite with no recovery path is not a rewrite the user can accept afterwards.

## Step 0: Fix the reader

Name who reads this before cutting anything. "Derivable" has no meaning without a reader. What a maintainer reconstructs instantly, a new contributor cannot.

Infer the audience from the document: a README addresses someone deciding whether to use the thing, a runbook addresses someone under time pressure at 3am, a design doc addresses a reviewer who will argue with it. When it is genuinely unclear, assume *a competent peer who knows the domain but has not read this document*, and say so in the report.

## Step 1: Extract the core

Read the whole target, then list every distinct claim it makes, one line each. Working state. Never output it.

Do this before touching a sentence. Editing linearly cannot find redundancy that spans sections, and spanning redundancy is where the savings are.

## Step 2: Order

Rank the claims by decision impact: what would change someone's action, first.

- Conclusion before derivation.
- Constraint before rationale.
- What to do before why it works.

Ordering is itself compression. Once ranked, claims that restate each other land next to each other and become visible, and the bottom of the ranking is the cut list.

## Step 3: Omit

The governing rule: **cut what the reader can predict, keep what would surprise them.** Predictable text carries no information; the reader supplies it for free. Surprising text cannot be reconstructed from anything else, which is exactly why it is in the document.

Cut a claim when it is:

- **Derivable**: reconstructible from what remains plus domain knowledge.
- **Inert**: nobody acts, decides, or believes differently without it.
- **The Nth instance**: a pattern an earlier example already established. Keep one, cut the rest.
- **Predictable rationale**: the "why" behind something no reader would question.

Concrete targets: roadmap preambles ("This document describes…"), closing recaps of the body, origin stories and process history, alternatives considered and rejected, definitions of terms the audience knows, enumerated cases a single rule subsumes, qualifications for situations the audience will never hit, transition sentences, future work with no owner.

Keep, at every level:

- **Sole carrier**: the only place a number, unit, version, path, identifier, or constraint appears.
- **Negation, exception, precondition**: cutting "don't do X" reads as permission to do X.
- **Ordering constraints** in a procedure.
- **The counter-intuitive**: anything contradicting what the reader would otherwise assume. Highest-value content in any document, and the easiest to mistake for a digression because it interrupts the flow.
- **Provenance** where credibility rests on it: citations, measured numbers, sources.

Rejected alternatives are a judgment call, not an automatic cut. Drop them when the document is instructions; keep them when someone will otherwise re-propose the rejected thing.

## Step 4: Reformulate

Write the survivors crisp. At `tight` this is the entire skill.

Cut the filler, hedging, and unverifiable adjectives the Plain style's `Cut` and `Vocabulary` sections list.

Keep:

- Articles and complete grammatical sentences at `tight` and `hard`. No fragments, no telegraphic style. Only `core` may drop them.
- Lists as lists. Do not fuse a list into a paragraph to save lines.
- Markdown frontmatter and links.
- Heading text byte-exact, emoji included, at `tight` and `hard`, because a heading is its own anchor and rewording one breaks every inbound link. `core` may merge, rewrite, or drop headings; when it does, say so in the report.
- The source language. Never translate; compress the style, not the language.

At `core` only, comparatives may become operators: "I'd rather review three small pull requests than one large one" → "3 small PRs > 1 large".

## Never touch: byte-exact

Code blocks and inline code, shell commands, file paths, identifiers, API and function names, error strings, quotes and citations, numbers and units, legal and license text.

Never drop a negation: not, never, no, only, except. A flipped meaning costs far more than any word saved.

Never invent abbreviations (cfg, impl, req, fn) and never substitute arrows for words. Both tokenize the same as what they replace, so there is zero saving, and the reader still has to decode them.

## Do not compress

Leave these at full length and shorten around them. At `hard` and `core` they are also ineligible for omission, whatever the tests say:

- Security warnings.
- Irreversible-action confirmations.
- Ordered multi-step instructions where dropping connectives makes the order ambiguous.
- Any passage whose compressed form is open to a second reading.
- Text where completeness is the content: specifications, API contracts, legal and license text, changelogs, audit trails.

## Report

Close with a report, and nothing else. Do not narrate tool calls while working.

At `tight`: before/after word count, plus one line naming anything deliberately left uncompressed.

At `hard` and `core`, add what was dropped. A word count alone is not an honest report for a lossy transform:

```text
412 → 138 words (−66%). Reader assumed: an operator running this service.
Dropped: origin story, rejected alternatives (read replicas, larger instance), definition of Redis.
Left uncompressed: the data-loss warning under "Rollback".
Rerun with /shorten tight to keep everything.
```

Name a dropped claim individually whenever a reasonable reader might want it back. End every lossy report with the `tight` rerun line, because that is when the user discovers they wanted it.

## Examples

**`hard`, content omission.** Before:

> ## Caching
>
> This section describes the caching layer and how it is configured. Some background first: when we built the service in 2023, every request hit Postgres directly, which was fine at the traffic we had then. As traffic grew we saw p99 latency creep up, and after investigation we determined the database was the bottleneck. We considered read replicas and a larger instance, but ultimately went with Redis.
>
> The cache is a Redis instance. Redis is an in-memory key-value store, which means reads are very fast. We set the TTL to 300 seconds. It's worth noting that the TTL is configurable via the `CACHE_TTL` env var if you need to change it. One thing to be careful about: the cache is not invalidated on write, so a stale read is possible for up to the TTL window.

After:

> ## Caching
>
> Reads go through Redis with a 300 second TTL, set by `CACHE_TTL`. The cache is not invalidated on write, so a read can be stale for up to one TTL window.

The origin story, the rejected alternatives, and the definition of Redis all went. The stale-read behavior led, because it is the one thing in the passage a reader would not have assumed.

**`core`, same input:**

> Redis cache, 300 s TTL (`CACHE_TTL`). No write invalidation, so reads stale up to 300 s.

**`tight`, prose only, nothing lost:**

> Sure! So the way that connection pooling works is that it essentially makes use of a set of already-open connections rather than performing the creation of a brand new connection for every single request that comes in.

→

> Connection pooling reuses open connections instead of creating a new one per request.

**Never compressed, at any level:**

> **Warning:** This deletes every row in the `users` table and cannot be undone. Verify a backup exists before running it.

The warning stays as written. Compressing it trades a real safety margin for four words.

## Boundaries

`/shorten` changes the named target only. It does not change how you talk afterwards, and it does not rewrite code, tests, or config. Prose only, including prose inside comments and docstrings when a file is the target.

Style adapted from [Caveman](https://github.com/JuliusBrussee/caveman) (MIT), whose `lite` level is roughly this skill's `tight`. The content-level steps are not from it.
