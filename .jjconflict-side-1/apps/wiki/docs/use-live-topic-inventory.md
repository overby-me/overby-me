# Live-subscription inventory (`use_live`) — input for the AppView realtime design

Decision-forcing input, NOT a frozen API. The AppView's realtime layer is still
open: `docs/atproto-stack-decisions.md` decides only ONE multiplexed `/ws` and
leaves the channel/topic mechanism and the subscription restrictor undesigned.
This is the code-grounded list of what the shipped frontend actually subscribes
to, so that design starts from facts instead of a guess. It deliberately does
NOT propose a frozen `(change-kind, scope-id)` taxonomy: freezing one from the
frontend side, ahead of the restrictor, is the exact guessing the pre-rewrite
plan deferred.

## What transfers unchanged, and what repoints

`use_live(query, refresh)` (`src/subscription.rs:23`) ignores the payload: every
push just bumps a `refresh` counter that a `use_resource` is keyed on, so the
view refetches. The **public signature and the whole `SubState` machine**
(connection_init, single subscribe, reconnect/backoff, and the focus-refresh
recovery, `subscription.rs`) lift wholesale to the AppView. The ONLY thing that
repoints is the transport: today each call passes a Hasura `subscription { ...
where: {...} }` string; against the AppView it passes a topic selector over the
one multiplexed `/ws`. So the migration is "swap the query string for a topic,"
and this table is the set of topics the app must be able to express.

## The 11 call sites

Every site subscribes to a set of rows and refetches when any of them changes.
The "scope key" is the entity id the filter is anchored on; the "discriminator"
is the extra predicate (a `mimeId`, or a relation `name`) that narrows it.

| # | Call site | Table + WHERE | Scope key | Discriminator |
|---|-----------|---------------|-----------|---------------|
| 1 | `layout/home_list.rs:37` | `members` where `nodeId _eq {uid}` | user id (`uid`) | — (membership rows) |
| 2 | `vote/mod.rs:53` | `relations` where `parentId _eq {ctx}` + `name _eq "active"` | context id | relation `active` |
| 3 | `vote/poll.rs:178` | `nodes` where `parentId _eq {poll}` + `mimeId _eq "vote/vote"` | poll node id | mime `vote/vote` |
| 4 | `vote/poll.rs:188` | `nodes` where `id _eq {poll}` (selects `mutable`) | poll node id | poll open/closed state |
| 5 | `screen.rs:28` | `relations` where `parentId _eq {ctx}` + `name _in ["active","screenComments"]` | context id | relations `active`/`screenComments` |
| 6 | `screen.rs:65` | `relations` where `parentId _eq {ctx}` + `name _like "focus:%"` | context id | relation `focus:*` |
| 7 | `screen.rs:159` | `relations` where `parentId _eq {ctx}` + `name _eq "active"` | context id | relation `active` |
| 8 | `speak.rs:223` | `nodes` where `parentId _eq {list}` + `mimeId _eq "speak/speak"` | speak-list node id | mime `speak/speak` |
| 9 | `admin.rs:47` | `relations` where `parentId _eq {ctx}` + `name _eq "active"` | context id | relation `active` |
| 10 | `comments.rs:118` | `nodes` where (`contextId _eq {ctx}` \| `parentId _eq {node}`) + `mimeId _eq "vote/comment"` | context id, else node id | mime `vote/comment` |
| 11 | `folder.rs:148` | `nodes` where `parentId _eq {node}` | node id | — (any child change) |

## What the raw list already tells the topic design

Stated as observations, not a decision:

- **Three scope keys recur**: a node id (`parentId`/`id`), a context id
  (`contextId`), and the user id (`nodeId` on `members`). Any topic scheme must
  express at least these three anchors.
- **The discriminator is either a `mimeId` or a relation `name`**: `vote/vote`,
  `vote/comment`, `speak/speak` on `nodes`; `active`, `screenComments`,
  `focus:*` on `relations`. A topic scheme narrower than "all children of X"
  needs to carry this discriminator; a coarser one (fire on any child change,
  as site 11 already does) does not.
- **One site is user-scoped, not content-scoped** (site 1, membership): the
  restrictor must authorize a member subscribing to their OWN membership feed,
  which is a different check from context-membership authorization.
- **Relations vs nodes**: 5 of 11 subscribe to `relations` (the join/edge rows),
  6 to `nodes`. The AppView's model splits `nodes` into
  context/document/comment and drops the generic `relations` table, so sites
  2/5/6/7/9 (relation `name` predicates) do NOT have a mechanical 1:1 topic and
  are the ones whose redesign needs the most thought — they are the input the
  restrictor design must resolve, not something to pre-freeze here.

## Explicitly out of scope

- No frozen topic taxonomy or broadcast API is proposed (see the header).
- No read/write contract table: `src/model.rs` already IS the code-derived
  read/write surface; duplicating it here would be redundant.
- The actual code swap onto topics waits on the AppView multiplexed-`/ws` topic
  API, which does not exist yet.
