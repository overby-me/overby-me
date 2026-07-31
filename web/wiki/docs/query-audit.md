# Every list query, and what it costs

2026-07-31, after the author-picker work. The question asked was whether other
queries share the two faults found there: no `limit`, and fetching a whole
document to read one key. Every row count below is from production, not
estimated.

## The headline

Missing `limit` turned out to be mostly a **non-problem**, and payload shape the
real one. A list the user must see in full cannot be capped without pagination —
a cap would silently hide children, comments or votes, and on a vote tally it
would silently produce a WRONG NUMBER. So the fixes worth making were about what
each row carries, not how many rows come back.

Three changes came out of it, all measured:

| what | before | after |
|-|-|-|
| poll turnout (`count_active_members`) | 46.0 KB | 0.1 KB |
| drawer expansion, widest folder | 54.0 KB | 11.4 KB |
| home context list | 3.2 KB | 1.9 KB |

## The queries that return lists

`limit` column: whether the operation sends one. `worst` is the largest result
this deployment can actually produce today.

| query | limit | worst | verdict |
|-|-|-|-|
| NodesWhereQuery | ✓ | — | |
| NodePickerQuery | ✓ 10 | — | author picker |
| NodesSearchQuery | ✓ 30 | — | search bar |
| MembersExistQuery | ✓ 1 | — | existence probe |
| UsersSearchQuery | ✓ 10 | — | |
| RecentNodesQuery | ✓ | — | |
| ChildrenQuery | — | 48 children | **must not cap** — a folder shows all its children |
| DrawerChildrenQuery | — | 48 children | **must not cap** — same list, in the tree |
| ChildIdsQuery | — | 239 per context | correct: a subtree walk that is wrong if truncated; ids only |
| VotesWhereQuery | — | 10 votes | **must not cap** — this is the tally |
| CommentsQuery | — | 7 comments | must not cap — a truncated discussion is a lie |
| ContextsWhereQuery | — | 8 groups | bounded by the number of contexts |
| PollsWhereQuery | — | 5 per context | bounded |
| PermissionsQuery | — | 45 per context | bounded by mime × role |
| RelationsQuery | — | 2 per node | bounded |
| InvitationsQuery | — | 23 per person | bounded, and per signed-in user |
| DeletedNodesQuery | — | 2 in the bin | bounded today; see below |
| MembersCountQuery | n/a | 1001 → **1 integer** | fixed, below |

### The one that was genuinely wrong: counting by fetching

`count_active_members` selected every member id of a context and took `.len()`.
On the largest context here — 1001 members — that is 46 KB over the wire to
learn one integer, and it runs for every poll, because turnout is drawn on each
one. The comment above it said the schema exposes no `members_aggregate`. That
was simply false: `membersAggregate` is there and the `user` role may call it.
Verified as an ordinary member against production: `{"count": 1001}`, 0.1 KB.

### The two that carried documents to draw an icon

`DrawerChildFields` and `ContextNodeFields` both select `data`, and both read
exactly one thing out of it: a file's content `type`, for the glyph. Everything
else in that jsonb — the whole Slate document of every sibling — was travelling
for nothing. Hasura selects inside a jsonb column, so they now ask for
`data(path: "type")`, as the search bar already does.

Measured on the widest folder in production, 48 children: **54.0 KB → 11.4 KB**.
The drawer expands a folder every time you open one, so this is not a one-off.

### The ones deliberately left alone

- `ChildNodeFields.data` stays whole. A folder row renders rich content inline
  (`SlateRenderer`) and the feed reads a comment's text out of it. It is used,
  not carried.
- `PollSummaryFields.data` stays whole — the options are the point.
- `NodeFields.data` is the document itself.

## Worth revisiting when the data grows

Nothing here is urgent at today's sizes, and each would be a real fix later:

- **The bin has 2 rows today.** At a few thousand it needs `order_by deletedAt
  desc` plus a limit and a "show older" step — a cap without an order would hide
  arbitrary items, which is why it has none now.
- **A folder with hundreds of children** needs pagination, and then
  `ChildrenQuery`/`DrawerChildrenQuery` get their limit for free. The
  congress-scale folder to watch is the one at 48.
- **A poll with thousands of votes** should be tallied server-side by an
  aggregate rather than by shipping ballots to the browser to count. Same shape
  of fix as the member count above, and the same reason: ask for the number.
