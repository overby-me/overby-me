# Rate limiting, as part of the permission system

Design, not yet built. Written 2026-07-31 from a design conversation, with the
measurements that decided each choice, so the reasoning survives the decision.

The wiki already answers "who may create this?" in one table. It cannot answer
"how often?", and every feature that needs an answer has so far had to invent
one. The pixel canvas (issue #12) is the first feature that cannot exist without
it, which makes it a good vehicle: build the general mechanism, and let the
canvas be its first consumer.

## The mechanism

One nullable column on the table that already governs creation:

```sql
alter table permissions add column rate_limit interval;   -- null = no limit
```

Read as: **the shortest time allowed between two of these, by the same person, in
this context**. `'60 seconds'` is a pixel cooldown. `'500 milliseconds'` is a
guard against a double-tapped submit button. `'5 minutes'` is a canvas that wants
to last all day.

### Why one column and not "N per window"

"5 per minute" is more expressive and worse in every way that matters here.

- It is **cheaper to enforce**. A minimum interval is one indexed lookup of the
  actor's most recent node. A count over a window scans the window on every
  attempt, by everyone, at exactly the moment everyone is attempting.
- It is **unambiguous**. "5 per minute" raises the question of whether the window
  slides or tumbles, and the two differ precisely when someone is hammering.
- It stays **additive**. If burst semantics are ever genuinely needed, a
  `rate_limit_count` defaulting to 1 slots in without changing what existing rows
  mean.

Every case the wiki actually has (a pixel cooldown, comment spam, repeated
feedback) is a minimum interval.

### Why `interval` and not an integer

Postgres has a duration type. An integer with the unit baked into the column name
is what you do in a language that lacks one, and it invites the 300-means-what
question at every call site.

The performance intuition points the other way, so it was measured: comparing
1,000,000 durations on this database,

| | per comparison |
|-|-|
| `now() - last < rate_limit` (interval) | **0.78 µs** |
| `extract(epoch from …) * 1000 < ms` (integer) | **1.25 µs** |

The integer form is *slower*, because reaching an integer means converting the
interval to a double and multiplying. Both are noise against a measured **170 µs**
node insert, so the check is at most 0.5% of the write it guards. Pick the type
that says what it means.

## Why a trigger can trust who is acting

The load-bearing fact, verified in this deployment's metadata rather than assumed.
The insert permission for role `user` carries:

```json
"column_presets": {"owner_id": "x-hasura-User-Id"}
```

`owner_id` is set by Hasura from the JWT. A client cannot write it, so it cannot
create a node as someone else to dodge its own cooldown, and the trigger needs no
session variables:

```sql
create trigger nodes_rate_limit before insert or update on nodes
  for each row execute function enforce_rate_limit();
```

which looks up the applicable `permissions` row for `(context_id, mime_id, role)`
and, when `rate_limit` is set, raises if the actor's most recent node of that mime
in that context is newer than the limit.

It needs one index, or it scans:

```sql
create index on nodes (owner_id, mime_id, context_id, created_at desc);
```

## The refusal has to be legible

This is the part to get right, and the reason is in `docs/` already: a failure the
user cannot act on is the defect. A rate limit is the one failure where the app
knows exactly what to say.

- Raise with a distinct, parseable shape carrying **`retry_after_ms`**, so the
  client stays in the unit the rest of the code speaks.
- Add `Failure::RateLimited` to `src/errors.rs` beside Offline, Refused and
  Broken, so `classify` cannot mistake it for a fault. It is neither a bug nor a
  refusal: it is a "not yet".
- The UI says *you can paint again in 42 seconds*, and never "something went
  wrong".

## First consumer: the pixel canvas

Two mimes, mirroring pairs the app already has (`vote/poll` + `vote/vote`,
`speak/list` + `speak/speak`):

| mime | what it is | hidden |
|-|-|-|
| `pixel/canvas` | the canvas node: size, palette and cooldown in `data`, open/closed via `mutable` | no |
| `pixel/pixel` | one painted pixel, `key = "p_<x>_<y>"`, colour index in `data` | **yes** |

What staying inside `nodes` buys, all of it already built:

- **Permissions**, including who may paint, as a `permissions` row keyed by
  context, mime and role. The rate limit above is then just another column on
  that same row.
- **Invisibility.** `mime_not_hidden()` already filters hidden mimes out of folder
  listings, search and sort, which is exactly how `vote/vote` stays out of view.
- **Rendering** is one arm in the `match mime_id` at `loader.rs:236`. No `?app=`
  entry needed: a canvas is a node type, so there can be many of them anywhere in
  the tree, and last year's can simply stay where it is.
- **Upsert for free** from the unique index on `(parent_id, key) where deleted_at
  is null`: repainting a pixel is an update of that row, so the canvas stays at
  its pixel count rather than growing without bound.

### Liveness: stream it, do not refetch it

Verified working on this deployment against the `user` role: subscribing to
`nodes_stream` with a cursor and then inserting a row delivered **exactly one
row**, not the result set.

```graphql
subscription {
  nodes_stream(batch_size: 50,
               cursor: {initial_value: {updatedAt: $since}, ordering: ASC},
               where: {parentId: {_eq: $canvas}, mimeId: {_eq: "pixel/pixel"}}) {
    key data updatedAt
  }
}
```

That removes the whole refetch-storm class described in `docs/assembly-load.md`
rather than merely taming it: the placement IS the payload. Note that a stream
pushes only what is newer than the cursor, so there is no initial payload; the
canvas loads once by query, then streams.

Scoping by `parentId` means people watching one canvas never receive another's
traffic, which falls out of the parent relationship instead of having to be
arranged.

### Measured, so the shape is not a guess

A full 64×64 canvas as nodes, against production (created and deleted):

| | |
|-|-|
| 4,096 pixel nodes inserted, path and ancestors triggers firing | 681 ms, **0.17 ms each** |
| table growth for the whole canvas | 26 MB → 28 MB |

An earlier draft of this design rejected one-node-per-pixel as obviously too
expensive. That was wrong, and measuring it is what showed the node model is
comfortably affordable at this size.

## Open questions before building

- Does the feed's `mime_not_hidden` filter really exclude them? Otherwise the feed
  becomes "someone painted a pixel", four thousand times.
- The drawer's `children_aggregate` will count pixels as children of the canvas.
- `ChildIdsQuery` is deliberately uncapped, so deleting a canvas enumerates every
  pixel in it.
- Repainting as an update keeps the row count fixed but discards history, so
  there is no timelapse. An append-log would give replay at the cost of unbounded
  growth. Worth deciding deliberately rather than by default.

## Effort

Roughly two to three days, almost all of it the canvas component: rendering into
a single `<canvas>` with `putImageData` (four thousand Dioxus elements is not
viable), pan and zoom on touch, and the palette. The rate limiting itself is a
migration, a trigger, an index and a `Failure` variant.

Not before the assembly. It puts a new write profile on the database that runs
the voting, and the voting is the thing that must not fail.
