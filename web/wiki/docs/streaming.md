# Where streaming subscriptions belong

2026-08-01. Hasura's streaming subscriptions deliver only rows newer than a
cursor, rather than re-sending a result set whenever it changes. Verified working
on this deployment as an ordinary member, including that a stream delivers an
UPDATE (a poll being closed arrived as a row carrying the new `mutable`, twice,
on the change and on the restore).

This is the audit of all fourteen live subscriptions in the app: which can
stream, which should, and which should not.

## The hard constraint: only `nodes` has a cursor

A stream needs a column that advances on every change. Checked against the
database:

| table | `updated_at` | `created_at` |
|-|-|-|
| `nodes` | **yes** (trigger-maintained) | yes |
| `relations` | no | no |
| `members` | no | no |
| `permissions` | no | no |

So six of the fourteen subscriptions — the projector's active node, its comment
and feed toggles, its focus anchor, the vote app's active relation, the admin
app's, and the two invitation watches — **cannot stream at all** without a schema
change.

### Would adding a column to those tables be worth it?

No, and the reason is worth stating because the answer looks like yes.

Streaming saves bytes in proportion to the size of the result set it replaces.
Those six subscriptions return **one to three rows**: which node the room is
looking at, whether comments are shown, a person's memberships. There is nothing
to save. `created_at` alone would not even work, since these rows are UPSERTED —
the chair changing the active node is an update, which a creation timestamp does
not move. It would have to be `updated_at` with a trigger, for no gain.

What that inspection did find is a real bug, and a schema change would not have
fixed it: both invitation subscriptions selected `{ id }` alone. A subscription
fires when its RESULT changes, and accepting an invitation is an update to
`accepted` — invisible in a list of ids. They select `accepted` and `active` now.
The fix is one line and needed no new column at all.

## The eight on `nodes`

### Converted

**The poll's open/closed state** (`poll.rs`). Was: a change token, then a whole
`query_node_by_id` — which fetches the node WITH its children and members — to
read one boolean. Now: the streamed row carries `mutable` itself, and there is no
refetch. The initial value comes from the prop the component already has, so the
stream is pure delta.

**Reactions** (`comments.rs`). Every reaction bar shares one context-wide
subscription, so a tap anywhere woke all of them and each refetched its own
comment's reactions: forty comments on a motion meant forty queries for one
emoji. The stream carries the rows, so a bar can see whether the reaction was on
ITS comment and ignore the rest. The list is still re-fetched rather than merged
— reactions are a handful of rows, and a refetch cannot drift out of step with
the server the way hand-applied deltas can.

That distinction is the general lesson: **a stream is worth having for what it
tells you changed, not only for what it saves you fetching.** Every conversion
below keeps the refetch and uses the stream to decide WHETHER to run it, or WHICH
rows to ask for — so the server stays the authority on contents and order, and
nothing drifts.

**Comments** (`comments.rs`). The same shape, and the biggest win of the three: a
post's comment list and EVERY open thread's replies shared one counter, so a
comment anywhere in the context refetched all of them on every device. Each now
wakes only for its own children — the section for comments on the post, a thread
for replies to itself — through one shared subscription, because identical
queries fold together in the hub.

**The feed** (`feed.rs`). This one deserved converting most and I nearly talked
myself out of it. The feed already keeps its own accumulated list and a queue of
staged arrivals, so the "hand-maintained ordered state" objection did not apply:
that state already existed. What the change token cost was a whole page of rich
rows fetched on every push, diffed against what was on screen, to discover the
one row that was new. The stream names the ids; those rows are fetched by id
through the same query the first page came from, so the row shape stays defined
in one place and a push costs the rows that actually arrived.

The pattern worth reusing is that last one: **stream the ids, fetch the rows with
the typed query.** It avoids spelling a rich selection out again inside a
subscription string, where it would drift from the real one.

### Deliberately not converted

**The vote tally.** Correctness outranks efficiency here. A streamed count is
maintained by the client, so a gap — a reconnect, a missed batch, a tab asleep —
leaves it quietly wrong, and a wrong number on a ballot is worse than a slow one.
The tally is already one aggregate of 0.15 KB and is self-correcting by
construction.

**Folder children and the speaker queue.** Both are small lists whose refetch is
already cheap, and both would need order maintained by hand (index changes,
renames, soft deletes arriving as updates). The win does not pay for the risk.

## If you do convert one later

- The cursor starts at mount and the initial state comes from a query; a stream
  pushes nothing until something changes, so there is no first payload to wait
  for. (An early probe of mine hung waiting for one.)
- Soft deletes arrive as updates, because `deleted_at` is a column. Whatever
  applies deltas has to drop rows that arrive already deleted.
- `batch_size` bounds a single push, not the subscription. A backlog arrives in
  several frames, in cursor order.
- Scope by `parentId` where you can. Two canvases, or two documents' comments,
  then never carry each other's traffic, and that falls out of the relationship
  rather than having to be arranged.
