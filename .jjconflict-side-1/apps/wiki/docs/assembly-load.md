# A 500-person assembly

2026-07-31, ahead of a general assembly of up to 500 people. Every number here is
measured against production — including a probe poll carrying 500 real vote rows,
created for this and deleted afterwards (the database is back at its exact prior
row count).

## The finding

**The database was never the problem. The request count was.**

Postgres answers the heaviest query on that 500-vote poll in **0.7 ms**, using an
index, with the full six-way permission filter applied. The wiki's shape at scale
was the risk: what every device does when *anybody* votes.

A poll pushes to every subscribed device on every vote. Each push re-ran three
queries per device. So one vote by one delegate cost 500 pushes and 1,500
queries, and a 500-ballot vote cost:

|  | as it was |
|-|-|
| queries during one vote | **750,000** |
| sustained rate over a 2-minute vote | **~6,250 req/s** |
| measured capacity before latency doubles | **~60 req/s** |
| data pushed + fetched | **~13 GB** over venue wifi |

That is roughly a hundred times what the endpoint serves, and the bandwidth alone
would not fit through a hall's access points. It would not have degraded
gracefully; it would have stopped working during a vote.

## What each piece cost, measured at 500 votes

| what a device did per vote cast | time | payload |
|-|-|-|
| tally: fetch every ballot and count in the browser | 241 ms | 28.3 KB |
| "have I voted" | 186 ms | 0.1 KB |
| "how many may vote" | 183 ms | 0.1 KB |
| subscription push carrying every vote id | — | 23.0 KB |

Baseline round trip to Hasura is 176 ms, so almost all of that is network, not
work: **the fix is to make fewer, smaller requests, not faster ones.**

## What changed

**1. A burst of pushes is now one refresh, and devices refresh apart.**
Pushes inside a 1.5 s window fold into a single refresh, and each refresh is
spread over a random second. Jitter matters as much as folding: 500 devices
refreshing 1.5 s later *together* is still a burst of 500. A voter now sees the
tally move up to ~2.5 s late, which is imperceptible on a number that is
climbing anyway.

**2. Subscriptions carry a change token, not the rows.**
`count` + `max(updatedAt)` moves on exactly the events row selection did — an
insert changes the count, an edit the timestamp, and a delete here is an edit
that does both. Verified over a real websocket: **23,011 bytes → 101 bytes**, and
confirmed to still push on a live insert.

Only for `nodes`. `relations` and `members` expose no timestamp to aggregate, so
a row edited in place — the chair moving what the room is looking at — would
leave count and max(id) unchanged and never reach the projector. Those stay as
they are.

**3. The tally is counted by the server.**
One aliased aggregate per option instead of shipping every ballot:
**28.3 KB → 0.15 KB, and no slower** (205 ms against 248 ms). Checked against the
real poll in production — the aggregate counts match the ballots exactly. A poll
whose results are hidden now asks only for the turnout total, and no delegate's
ballot leaves the database to be counted in a browser.

**4. The electorate is no longer re-counted per vote.** It changes when someone
joins or leaves, not when someone votes.

**5. Having voted is remembered.** The check stops running once it is true —
which matters most on a *secret* poll, where it calls our own axum backend rather
than Hasura.

**6. Identical subscriptions share one server-side subscription.** Each reaction
bar used to watch its own comment, so forty comments meant forty live queries per
device — 20,000 across a hall. They now watch the context, which makes every bar
ask the same question, and the hub collapses them into one.

### Where that leaves it

| | before | after |
|-|-|-|
| queries during a 500-ballot vote | 750,000 | **~30,000** |
| sustained rate | ~6,250 req/s | **~250 req/s** |
| bytes moved | ~13 GB | **~40 MB** |

Still above the ~60 req/s at which latency began to climb in my probe, but that
probe was one client machine and is a floor, not a ceiling. The remaining
traffic is small requests, and the DB work behind each is under a millisecond.

## What is NOT fixed, in the order I would worry about it

1. **The cold load.** 4.4 MB of assets, but the server compresses: a device
   actually downloads **~1.7 MB** on a first visit, so 500 devices at the door is
   ~860 MB through the venue uplink rather than the 2.2 GB an earlier draft of
   this file claimed. It is served by the CDN, not by Hasura, so nothing breaks —
   but it will feel slow, and it is the first impression. **Ask people to open
   radikal.wiki once before they arrive**, or on mobile data; the service worker
   then serves it from cache. `docs/bundle-size.md` measures where those bytes go
   and what can be cut (down to ~0.95 MB).
2. **The login storm.** 500 sign-ins in the same ten minutes is untested. Same
   mitigation: get people signed in before the session starts.
3. **Postgres allows 100 connections.** Hasura pools, so 500 devices do not mean
   500 connections — but the axum backend shares the budget. Worth watching if
   anything else is deployed against the same database that week.
4. **The chair's results view still fetches every ballot** (`query_poll_votes`).
   That is one device, and it may legitimately want the detail, so I left it.
5. **Anonymous visitors appear not to get live updates.** Seen before any of
   these changes, so it is pre-existing and not a regression. Not worth chasing
   for the assembly, where everyone is signed in.

## Before the day

- **Rehearse one poll with real phones** — ten people, one poll, watch the tally
  move. The client wiring of the coalescer is unit-tested, and the server half is
  verified over a live websocket, but a rehearsal is what proves the two together
  on real devices.
- **Watch Better Stack during the first vote.** Client errors ship there, so a
  failing query shows up as a spike rather than as a rumour in the hall.
- **Keep polls small in options.** The tally costs one aggregate per option.
  Twenty options is twenty aggregates; three is three.

## How to re-measure

The probe is reproducible: insert a context, a poll, 500 vote rows with
`generate_series`, add one member row so the permission path matches a real
delegate, measure, then delete. Row counts before and after must match exactly.
The scripts are not checked in — they touch production and should be written
deliberately each time, not run by habit.
