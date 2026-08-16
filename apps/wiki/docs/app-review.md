# Deep review of the wiki

A whole-application review, 2026-07-30: architecture, data model, scale,
correctness, security, testing and operations. The Material 3 conformance layer
is separate, in [`m3-audit.md`](./m3-audit.md).

Measurements are from the tree at `a61b0997`. Where something could not be
verified from here (anything needing a signed-in session or production load), it
says so rather than guessing.

**Shape of the thing.** 31,800 lines of Rust in the frontend, 4,900 in the
backend, 7,100 lines of CSS, 8,700 lines across the parallel atproto crates. A
3.7 MB wasm bundle plus 430 KB of JS, CSS and fonts. 79 unit tests. One
database, shared with the retired React app.

---

## 1. Scale is the real risk, and it is concentrated in three places

This app's hardest hour is a congress: several hundred delegates in one hall, on
one wifi, all voting and joining speaker lists at once. Everything below is fine
at ten users and questionable at four hundred.

### 1.1 One WebSocket per subscription

`subscription.rs` opens a socket per `use_live` call, and says so deliberately:
*"Per-subscription connection state ... NO shared connection manager: the
rewrite's single multiplexed WebSocket lifts this state machine wholesale."*

There are 13 `use_live` sites. A delegate reading a motion with an open poll
mounts the comments' two, the poll's two, the vote view's one: **four to six
sockets per device**. Four hundred delegates is on the order of **two thousand
concurrent subscriptions** against one Hasura instance, each with its own
reconnect-with-backoff loop when the venue wifi blips, and blips are correlated
across the room.

Nobody has load-tested this. It is the single most likely thing to fail on the
day, and the failure mode is not graceful: a dropped socket means a projector or
a ballot silently stops updating until its backoff succeeds.

**What to do.** Multiplex onto one socket per client. The state machine is
already written and isolated in one file, so this is contained work rather than
a rewrite, and it turns 2000 sockets into 400. Do it before the next congress,
and rehearse it against a copy of the database.

### 1.2 One mutation refetches everything

`use_data_resource!` subscribes every resource to a single global `DATA_VERSION`
(`session.rs:71`), and `bump_data_version()` increments it after any mutation.
There are **55 resource sites**. Every one currently mounted refetches on every
mutation, whether or not it could possibly have changed.

So a delegate casting a vote re-runs their agenda query, their crumb query, their
member list, their poll tallies. It is correct and it is simple, which is why it
was built this way; it is also a burst of a dozen queries where one was needed,
multiplied by everyone in the room mutating at once.

**What to do.** Key invalidation by topic rather than one global counter: a
version per context, or per mime, so a comment invalidates comments. A smaller
first step is to exempt the expensive resources (crumbs, agenda) which almost
never change as a result of the mutations that fire this.

### 1.3 Nothing is cached for offline

The service worker (`assets/sw.js`) is cache-first for hashed assets and
stale-while-revalidate for navigations, falling back to the cached shell. **No
GraphQL response is cached at all**, so the moment the network wobbles every page
is empty even though the app itself loads fine.

For a congress this is the difference between "the wiki is slow" and "the wiki is
down". Caching the last response per query, marked stale, would keep the agenda
and the motion under discussion readable through a blip.

---

## 2. Data model

### 2.1 The tree has no ancestor chain (partly addressed)

`contextId` names a node's *nearest* group or event, and a context is its own
context, so it cannot express "everything under X". The `path` column added in
`migrations/0001` fixes the navigation half: URL resolution and breadcrumbs went
from one query per segment to one query flat.

What it does not yet fix: subtree queries still walk. `delete_node_deep`
(`graphql.rs:2550`) deletes a subtree one request per node, depth-capped at 32,
and `node_path` (`graphql.rs:3048`) climbs one request per level, which the code
honestly documents as *"One request per level, which is why the caller should do
this on CLICK rather than while rendering."* Both become single statements once
`ancestors` lands beside `path`.

### 2.2 There is no foreign key on `parent_id`

Deleting a node leaves its children pointing at an id that no longer resolves.
**279 orphan parents and 527 unreachable nodes** exist right now, measured during
the migration. The app has a whole view (`?app=parent`) built to find and adopt
them, which is a feature that exists because of a missing constraint.

The bin work (soft delete) is the right way out, and its columns are already
live. Adding the foreign key without the bin would make accidental deletion
unrecoverable, so the order matters: bin first, constraint second.

### 2.3 Permissions are data, and only partly understood from the client

Insert rights come from a `permissions` table consulted through a computed field
(`nodes.inserts`). That is a good design — the UI cannot drift from the rules —
but it means the client cannot explain *why* something is not allowed. Every
"you can't do that" is silent: the button simply is not there. When someone asks
why they cannot add an event, the answer requires a database query, as it did
today.

---

## 3. Correctness: the failure mode is silence

There are only **14 `unwrap()`/`expect()` calls** in the whole frontend and most
are in tests, so the app does not panic. What it does instead is swallow:

- **188** `unwrap_or_default()`
- **111** `.ok()`
- **151** `let _ = …`

Almost every one turns a failed query into an empty list. A permissions error, a
network failure and a genuinely empty folder all render identically as "no
content". This is why the two bugs found today were invisible for so long: the
projector's stale speaker list, and wasm-opt aborting on every single build while
the build reported success.

This is a design choice, not an accident, and for a read view it is often the
right one. But it should be possible to tell the three apart. The cheapest
improvement with the biggest payoff: distinguish *empty* from *failed* in the
resource type, and let the empty states say which. The error card exists already;
it is simply unreachable from most views.

---

## 4. Security

Nothing alarming, and two things worth knowing.

- **The backend is the trusted edge and behaves like it.** Every route takes the
  caller's bearer token and resolves a uid from it (`backend/src/auth.rs`);
  the admin secret is used only server-side, to mint short-lived signed links for
  the Office viewer. That is the right shape.
- **Rendering is safe by construction.** Content is Slate JSON rendered into
  elements, not HTML; the one `set_inner_html` is the editor seeding its own
  serialized content, and pasted HTML goes through a tag allow-list. Link hrefs
  pass `safe_href`, which permits only http, https, mailto and app-relative URLs.
- **The Better Stack ingest token is compiled into the bundle**, because
  `option_env!` bakes it at build time. Anyone can read it out of the wasm and
  post junk to your log stream. That is inherent to client-side telemetry and not
  worth panicking about, but it should be a rotatable, write-only source token,
  and it should not be reused anywhere else.

---

## 5. Testing: the tests cannot see the app

79 tests, all pure logic: GraphQL where-clause serialization, i18n parity, path
helpers, ODT export, JWT parsing. Nothing renders a component. The browser smoke
test (`test-browser.nu`) exists but needs `WIKI_EMAIL`/`WIKI_PASSWORD`, which are
not set, so **no test exercises a signed-in path** — which is to say, no test
exercises the app as anyone actually uses it.

Every UI change in this session was verified by rendering screenshots and looking
at them. That works, and it does not survive into CI: the next person changing
the drawer has nothing to tell them they broke the place picker.

A seeded test account plus five or six headless flows (log in, open an event,
project an item, join a speaker list, comment, vote) would cover the paths that
actually matter, and would have caught both of today's bugs.

There is also no CI for the frontend at all: `.tangled/workflows/` builds the Nix
flake, nothing builds or tests the wiki.

---

## 6. Operations

- **Deploys are manual and unrehearsed.** `just build` in the devshell, then a
  zip upload to statichost. `dev.radikal.wiki` exists but is far behind, so
  production is the first place a bundle ever runs. That is how wasm-opt could
  abort on every build for months without anyone noticing.
- **The commit is the only deploy record.** `version.json` carries it, and the
  git bookmark `wiki-prod` is moved by hand. It is now accurate; it was
  months stale before today.
- **There is no rollback story.** Redeploying an older bundle means rebuilding it
  from an older commit. Keeping the last two builds' output would make rollback a
  re-upload.

---

## 7. Architecture

The structure is sound and the comments are unusually good: nearly every
non-obvious decision has its reasoning recorded next to it, which is why this
review could be evidence-based rather than speculative.

Two strains worth naming.

**`graphql.rs` is 4,400 lines** and holds every query, mutation, input type and
conversion in the app. It is the file every feature touches, and the file most
likely to conflict. Splitting it by domain (nodes, members, votes, speak,
relations) is mechanical and would make the next year cheaper.

**There are two futures in the tree.** `crates/` holds 8,700 lines of atproto
port work — appview, ballot spec and store, migration extractor and loader — next
to a Hasura app under active development. `docs/pre-rewrite-plan.md` says the
intent is to contain the backend swap behind a repository boundary, but the
components still name `graphql.rs` types directly. Every feature added now is
added twice: once here, once in whatever the port becomes. That is a strategic
cost worth being deliberate about rather than drifting into.

---

## 8. Product gaps

Ordered by what they cost a user, not by effort.

1. **An invitation only arrives if a human sends it.** Delivery is the inviter
   opening their mail client via `mailto:`. Someone who never signs in never
   learns they were invited, which is the single biggest barrier to anyone
   getting in at all.
2. **The badge is not live.** `PENDING_INVITES` is fetched on mount and on
   mutations, so an invitation arriving while the tab is open stays invisible
   until reload. `HomeList` already subscribes to the right rows; pointing the
   badge at the same subscription is small.
3. **A group's feed cannot include its events**, per §2.1. Wanted the moment
   anyone opens a group expecting to see what happened at its meetings.
4. **No bin yet.** The columns are live, the UI is not, so deletion is still
   permanent everywhere except a comment with replies.
5. **The emoji picker's category labels are untranslated** — Quick, Smileys,
   Gestures, Hearts, Celebration, Symbols, hardcoded in `comments.rs` — so a
   Danish reader gets six English headings inside an otherwise Danish app. They
   are the only user-facing strings that bypass i18n; the other literals are
   class names, mime types and a hidden debug page. Worth knowing that the i18n
   test cannot catch this class at all: it checks that every key *used* exists,
   and a bare literal uses no key. (`auth.rs` documents having already been bitten
   by exactly this, when an unmapped service error reached a Danish screen as
   "Password is too short".)

---

## Where I would start

1. **Multiplex the WebSocket** (§1.1). Highest risk, contained fix, and the
   deadline is the next congress rather than "someday".
2. **Seeded browser tests** (§5). Everything else gets safer to change once
   these exist, including the multiplexing.
3. **Cache query responses for offline reads** (§1.3), same deadline as 1.
4. **Distinguish empty from failed** (§3). Cheap, and it makes the next bug
   visible instead of silent.
5. **Server-sent invitations** (§8.1), the biggest product gap.
6. **Scope invalidation** (§1.2) and **split `graphql.rs`** (§7) as the tree
   grows.
