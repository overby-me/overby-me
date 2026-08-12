---
name: code-comments
description: Rules for writing (and not writing) code comments and doc comments. Use whenever adding or editing comments in source files, writing doc comments on functions, types or columns, annotating SQL migrations and schema files, writing TODOs, justifying magic numbers or constants, recording why an obvious alternative was rejected, or when asked to tighten, trim or reduce comments.
---

# Code comments

Two different things, two different audiences:

- **Documentation** (`///`, docstrings) is written for *consumers* of the code.
- **Comments** (`//`, `#`) are written for *collaborators* in the code.

This skill is mostly about the latter.

## The test

Default to **no comment**. Then ask the one question that matters:

> Does this comment carry information that is **not recoverable from the code**?

If the answer is no, delete it. Information that was never written down cannot
be recovered by reading harder, but information already in the code does not
need restating.

Better names, smaller functions and richer types clarify the *how*. They do not
replace comments, because they cannot convey the *why*. Do both.

## What earns a comment

**TODOs.** Leave enough for a stranger to act on. A few keywords that made sense
while it was fresh will still be a to-do in six months. Carry the author's name
(`git blame` is not a substitute) and a ticket link if there is one. Multi-line
TODOs and partial steps are fine. The only fixed part of the format is that the
letters `TODO` appear.

**References.** Code copied from elsewhere, or implementing an algorithm from a
paper, post or book, links its source: a *permalink* for code (press `y` on
GitHub first), title plus author plus chapter for print. Document and motivate
any divergence from the reference.

**Correctness arguments.** Code shows the steps, tests show the outcome, and
neither shows *why* those steps reliably produce that outcome. Write the proof
down, and pair it with assertions (`unreachable!`, `expect`, `debug_assert!`)
where the language allows. Never commit a correctness argument abandoned
half-way: that is worse than none.

**Hard-learned lessons.** If it took more than ~30 minutes to land on some
unintuitive incantation, comment it. You did not know it 30 minutes ago, and
nobody else will either. If you never found out *why* it works, say how you
arrived at it and what breaks without it, so the next person can continue from
there.

**Rationale for constants.** For any magic number: what it represents, how it
was chosen, and what changing it costs. `const HEARTBEAT_INTERVAL: usize = 5;`
tells the reader nothing about whether 5 is load-bearing. "Picked arbitrarily,
never tuned" is a perfectly good and genuinely useful answer.

**Load-bearing choices.** If correctness depends on a seemingly innocuous detail
elsewhere, flag it *at that detail*: "must collect into a `BTreeSet`; the code
below assumes ordered iteration", "this type must never be constructible outside
this module or the `unsafe` block below is UB". Prefer encoding the invariant in
types or assertions; a comment is the fallback, not the first resort.

**Algorithm outlines.** When a simple algorithm gets lost in the syntax
implementing it, sketch the high-level steps up top, or mark the sections inline
so readers can locate themselves.

**"Why not"s.** The corollary to hard-learned lessons: not "why is this line
needed?" but "why didn't you do it the obvious way?". Deliberate deviations from
convention, or avoidance of an available helper, need their reasoning recorded
or it will be re-derived (or undone) by someone else.

**Intentional trade-offs.** Decisions get re-litigated every few years
otherwise. A concise
[Y-Statement](https://www.infoq.com/articles/sustainable-architectural-design-decisions/)
in a comment next to the code it justifies stays discoverable and gets updated
with the code. Roughly: *"In the context of \<use case\>, facing \<concern\>, we
decided for \<option\> to achieve \<quality\>, accepting \<downside\>."* Each
field is a concise fragment, none may be skipped; add free-form paragraphs after
if needed. Prefer this over a `docs/adrs/` entry for decisions localised to one
place in the code, since a separate document drifts and is easy to miss; reserve
ADRs for cross-cutting ones.

## What gets deleted

- Restating the code in English.
- Explaining what a well-named function already says.
- Reassurance ("this is safe because...", "note that this correctly...").
- Narrating alternatives tried during the session that left no trace in the
  code. A *rejected* alternative that a future reader would plausibly reach for
  is a "why not" and stays. The five things fumbled on the way do not.
- Anything that only makes sense to someone who watched you write it.
- Comments describing a previous iteration of the code. A stale comment is worse
  than no comment: it makes the reader doubt their own correct understanding.

## Length

Length is set by the information, not by a line budget.

Noise is too long at one line. A correctness argument or a constant's testing
methodology can be worth several paragraphs. Bytes are cheap; your
undercaffeinated self in a year is not. Do not be terse at the cost of being
understood.

The old heuristic "never longer than the code it describes" is wrong in exactly
the cases that matter most: three lines of subtle concurrency can need fifteen
lines of proof. Use it only as a smell test on comments that carry no new
information.

## Comments are technical writing

**Remember the reader.** They lack your context. "Obviously", "of course",
"trivially" and "just" are tells that you assumed otherwise. Re-read while
suppressing what you know.

**Precision matters.** Typos, missing words, sloppy grammar, and stale
references to renamed variables or functions actively mislead. Read it once more
and check it says what you meant.

**Full sentences, real punctuation.** Avoid all but the most obvious
abbreviations.

## Doc comments

Written for consumers. State what it does, the contract, and any surprising
precondition. Not a design document: the reasoning behind the implementation
belongs in `//` comments inside it.

```rust
// Too much:
/// Begin a transaction, fetch its ID, and log it.
///
/// `author` is the acting user's `users.rl`, or `None` for unauthenticated
/// paths such as the annotation webhook. It is published to the transaction
/// via `set_config(..., true)` so database triggers can attribute the change.
///
/// **Attribution only reaches writes made through the returned transaction.**
/// A bare `sqlx::query!(...).execute(&pool)` runs as its own implicit
/// transaction with the setting unset, so any table carrying an event-log
/// trigger must be written through `begin_tx`.

// Enough:
/// Begin a transaction and publish `author` as `cmss.author_rl` for the
/// event-log triggers. Writes made outside this transaction are unattributed.
```

## SQL schema and migrations

Column comments are one line, table comments one or two. These are descriptions,
not arguments, so they stay short.

A migration comment explains why the migration is shaped the way it is when that
is not obvious: a backfill ordering constraint, a lock-avoidance trick, a column
left nullable pending a later migration. Not the options weighed.

## Working with agents

Agents arrive with no memory of the design discussion, the incident, or which
choices were deliberate versus arbitrary. They are precisely the audience for
"why" comments, and comments land directly in the context window, unlike a
ticket or PR description they may never find.

- When writing comments during an agent session, use the *why* uncovered during
  that session. That context is exactly what the code cannot express.
- **If you find yourself repeatedly correcting an agent on the same point, fix
  it with a strategically placed comment at the code in question, not another
  stanza in `AGENTS.md` or `CLAUDE.md`.**
- Agents are good at auditing whether existing comments still match their code
  and whether the stated reasoning still holds. Use them for that.

## When you catch yourself

If a comment is growing and you are unsure, classify it:

1. **Carries information the code cannot.** Keep it, at whatever length it
   honestly needs.
2. **A reviewer needs this now.** Put it in the PR description.
3. **A decision that outlives this PR.** Y-Statement by the code, or an ADR.
4. **Nobody needs this.** Delete it.

There is no correct comment-to-code ratio, and not all code should be commented.
Good commenting is spotting information gaps and filling them: judgement,
empathy and foresight, not a mechanical pass. If it feels mechanical, you are
probably writing noise.

Finally, do not burn the review on comment bikeshedding. Landing good code with
adequate comments and improving them as a follow-up is legitimate, and someone
other than the author often writes them better, being free of the author's
implicit knowledge.
