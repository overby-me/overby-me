# Domain model & lexicons (atproto rewrite)

A first cut of the data model for the custom backend, derived from the current
`mimeId` taxonomy and the source-of-truth split in
[`atproto-port.md`](./atproto-port.md). Two layers:

- **Lexicons** — the (small) surface of *public* atproto records in user repos.
- **Domain model** — the (large) *private, org-authoritative* store in the
  backend DB (SurrealDB, per the port doc). Written in SurrealQL as a concrete
  starting point.

> Draft, not committed. NSID `wiki.radikal.*` is a placeholder — use a domain the
> org controls.

## The headline: visibility is per-item — a public/private hybrid

The scope may grow into a general social platform (feed, groups, events) where
those things can **optionally be public**. So visibility is a *per-item property*,
not a global stance — and that makes atproto a first-class substrate for the
public half:

> Public content (posts, groups, events, statements, resolutions) lives as
> user- or org-owned **atproto records** — a real, federatable social platform.
> Private content and the always-private machinery (secret ballots, the roster,
> authoritative state) live in the **backend DB**. One queryable store unifies
> the two via materialisation.

Two constraints shape the whole design, and both matter:

- **atproto has no private records.** Anything private is simply DB-only, never a
  record. "Optionally public" therefore means "optionally *becomes* a record" —
  publish creates it, un-publish tombstones it.
- **A few things are never public regardless of the toggle.** Secret ballots
  (obviously). And for a political org, **membership/affiliation** probably —
  outing who belongs is a real harm, so being a *member* stays private even in a
  public group (posting *in* it can be public; belonging to it is not, unless the
  member opts in).

So it's not "private app" vs "atproto app" — it's both, split per item. The
lexicon surface is consequently **substantial** (posts, groups, events, …), not
tiny, and the backend DB is still the source of truth (public items are mirrored
out as records; see the visibility model below).

## Lexicons as the canonical model (not just the public surface)

atproto earns its place on two counts, independent of how public the app is:

1. **Identity.** The DID is the primary identity; atproto OAuth is the login —
   portable, password-less, and it de-risks the hardest migration (member → DID).
   True even for a mostly-private app.
2. **Lexicons model ALL the data — public *and* private.** A lexicon is a
   schema/IDL. Define one per entity (user, post, group, event, document, poll,
   ballot, comment, membership…) and it is the single canonical contract:
   `atrium` codegens Rust types from it, so *one* type model drives both the
   record wire-format (public items) AND the DB rows (private items). Visibility
   decides only **publication**, not schema:
   - **public** instance → published as a record in a repo (governed by its lexicon);
   - **private** instance → the *same* lexicon shape, kept in the DB, validated but
     never broadcast.

Precision worth keeping: atproto-the-network is public-by-default, so for the
private half this is **"lexicons as the schema language"** (one canonical model +
validation + codegen), not "private atproto records" (which don't exist on the
public network). Private instances live in your store; the lexicon is the shared
shape. **SurrealDB then *realises* these entities** — adding the relational/graph
structure (membership edges, references), indexes, and the always-private tables —
while the lexicons stay the source of truth for entity *shape*.

## Source-of-truth split, per entity

Visibility dispositions: **optional** (public → an atproto record; private →
DB-only), **always-private** (never a record), **public** (always a record).
The DB is the source of truth throughout; public items are *mirrored out* as
records.

| Entity | What it is | Visibility | Where the record lives (when public) |
|---|---|---|---|
| user / profile | a person (DID) | public profile | member repo (or reuse `app.bsky.actor.profile`) |
| **post** *(new — the feed unit)* | a feed item | **optional** | author's repo |
| `wiki/group` | group | **optional** | org or owner repo |
| `wiki/event` | event | **optional** | org or owner repo |
| `wiki/document`, `vote/policy`/`position`/`candidate`/`change` | content / proposals | **optional** | author's repo |
| `vote/comment` | reply / discussion | **optional** (public iff on public content) | author's repo |
| `wiki/file` | attachment (blob) | follows its parent | blob ref in the parent record |
| resolution | official vote **outcome** | **public** | **org** repo (org-signed) |
| statement | personal public statement | **public** | member repo |
| `wiki/folder` | organising structure | private | — (DB) |
| `vote/poll` | a poll | optional (a *public* poll may be announced) | org repo (announcement only) |
| `vote/vote` ballot | a cast ballot | **always-private** | — (never a record; anonymised in DB) |
| `has_voted` | one-vote dedup | **always-private** | — |
| members | roster + roles | **private** (opt-in "member of public group X" only) | member repo (only if the member opts in) |
| `speak/*`, relations (`active`, `screenComments`) | ephemeral coordination | private | — (DB) |

The always-private set is small and specific — secret ballots, the dedup marker,
the roster/roles, and ephemeral coordination. Almost everything *content* is
optionally public, which is what makes the public half a genuine atproto social
platform.

## Publishable lexicons (the ones that reach public repos)

Every entity has a lexicon (see above); this is the subset that, when an instance
is *public*, is published to a repo. User-authored things (**post**, statement,
comment, document) go to the **author's** repo; org-owned things (public group /
event, official **resolution**) go to the **org's** repo (the service holds its
own DID). Core sketches below — group, event and document follow the same shape.

The `post` is the feed unit — the atproto-native heart of the "public half":

```json
{
  "lexicon": 1,
  "id": "wiki.radikal.post",
  "defs": {
    "main": {
      "type": "record",
      "key": "tid",
      "record": {
        "type": "object",
        "required": ["text", "createdAt"],
        "properties": {
          "text":      { "type": "string", "maxGraphemes": 3000 },
          "createdAt": { "type": "string", "format": "datetime" },
          "group":     { "type": "string", "format": "at-uri",
                         "description": "the public group/event this was posted in, if any" },
          "reply":     { "type": "string", "format": "at-uri",
                         "description": "parent post, for threads" },
          "embed":     { "type": "union", "refs": ["#image", "#link"] }
        }
      }
    }
  }
}
```

```json
{
  "lexicon": 1,
  "id": "wiki.radikal.statement",
  "defs": {
    "main": {
      "type": "record",
      "key": "tid",
      "record": {
        "type": "object",
        "required": ["text", "createdAt"],
        "properties": {
          "text":        { "type": "string", "maxGraphemes": 3000 },
          "topic":       { "type": "string", "maxLength": 200 },
          "createdAt":   { "type": "string", "format": "datetime" },
          "canonicalUri":{ "type": "string", "format": "uri",
                           "description": "optional link to the internal artifact this was published from" }
        }
      }
    }
  }
}
```

```json
{
  "lexicon": 1,
  "id": "wiki.radikal.resolution",
  "defs": {
    "main": {
      "type": "record",
      "key": "tid",
      "record": {
        "type": "object",
        "required": ["title", "status", "decidedAt"],
        "properties": {
          "title":       { "type": "string", "maxGraphemes": 300 },
          "body":        { "type": "string", "maxGraphemes": 10000 },
          "status":      { "type": "string", "knownValues": ["passed", "rejected"] },
          "tallyFor":    { "type": "integer" },
          "tallyAgainst":{ "type": "integer" },
          "assembly":    { "type": "string", "description": "which meeting decided it" },
          "decidedAt":   { "type": "string", "format": "datetime" }
        }
      }
    }
  }
}
```

## Visibility model (how "optionally public" works)

The DB is always the source of truth. Visibility is a per-item field:

- **private** → DB only; never leaves the backend.
- **public** → DB **plus** a mirrored atproto record. On publish, the backend
  writes the record — to the **author's** repo for user content (via their OAuth
  session) or the **org's** repo for org content — via `atrium`. On un-publish it
  tombstones the record.
- The AppView also **ingests public records from Jetstream** (your users' and,
  optionally, external ones), materialising them into the same DB, so a single
  query serves a feed that mixes private-DB items and public-record items.

Design consequences:

- **Public is (potentially) forever.** Toggling public→private issues an atproto
  delete, but you can't recall it from caches/AppViews you don't control. Treat
  "make public" as "this becomes world-readable, possibly permanently."
- **User-owned when in the user's repo** — that's the atproto payoff (data
  ownership, federation, visible to other AppViews/clients). Org-owned public
  content (groups, resolutions) is owned by the org's DID instead.
- **Always-private items get no visibility toggle** — ballots, the dedup marker,
  the roster. The toggle exists only where publishing is meaningful and safe.

## DB realisation (SurrealQL)

The SurrealDB tables *realise* the lexicon-defined entities — same shapes,
codegen-shared with the record types — and add what lexicons don't express: the
relational/graph structure (membership edges, references), indexes, and the
always-private tables (ballots, dedup, roster). `visibility` marks which rows also
exist as published records.

```surql
-- Identity: the DID *is* the person (record id = the DID).
DEFINE TABLE user SCHEMAFULL;
DEFINE FIELD did          ON user TYPE string;
DEFINE FIELD handle       ON user TYPE option<string>;
DEFINE FIELD display_name ON user TYPE option<string>;
DEFINE FIELD avatar_url   ON user TYPE option<string>;

-- Contexts: groups & events (the org's structures). Hierarchy via `parent`.
DEFINE TABLE context SCHEMAFULL;
DEFINE FIELD kind       ON context TYPE string ASSERT $value IN ['group', 'event'];
DEFINE FIELD name       ON context TYPE string;
DEFINE FIELD slug       ON context TYPE string;
DEFINE FIELD parent     ON context TYPE option<record<context>>;
DEFINE FIELD visibility  ON context TYPE string DEFAULT 'private' ASSERT $value IN ['private', 'public'];
DEFINE FIELD published_uri ON context TYPE option<string>;   -- the at-uri, if the group/event is public
DEFINE FIELD created_at ON context TYPE datetime DEFAULT time::now();
DEFINE INDEX context_slug ON context FIELDS parent, slug UNIQUE;

-- Content: documents / folders / files / proposals (kind-tagged).
DEFINE TABLE document SCHEMAFULL;
DEFINE FIELD context       ON document TYPE record<context>;
DEFINE FIELD parent        ON document TYPE option<record>;      -- folder or context
DEFINE FIELD kind          ON document TYPE string;              -- document|folder|file|policy|position|candidate|change
DEFINE FIELD title         ON document TYPE string;
DEFINE FIELD content       ON document FLEXIBLE TYPE option<object>;  -- Slate JSON (carries over)
DEFINE FIELD author        ON document TYPE option<record<user>>;
DEFINE FIELD visibility    ON document TYPE string DEFAULT 'private' ASSERT $value IN ['private', 'public'];
DEFINE FIELD published_uri ON document TYPE option<string>;      -- the at-uri, once published
DEFINE FIELD created_at    ON document TYPE datetime DEFAULT time::now();

-- Feed posts: the social unit. `visibility=public` -> mirrored to a repo.
DEFINE TABLE post SCHEMAFULL;
DEFINE FIELD author        ON post TYPE record<user>;
DEFINE FIELD group         ON post TYPE option<record<context>>;  -- posted in a group/event
DEFINE FIELD reply         ON post TYPE option<record<post>>;     -- thread parent
DEFINE FIELD text          ON post TYPE string;
DEFINE FIELD visibility    ON post TYPE string DEFAULT 'private' ASSERT $value IN ['private', 'public'];
DEFINE FIELD published_uri ON post TYPE option<string>;
DEFINE FIELD created_at    ON post TYPE datetime DEFAULT time::now();
DEFINE INDEX post_feed     ON post FIELDS group, created_at;      -- feed by group + time

-- Membership as a GRAPH edge (user -> context) — this is where SurrealDB beats
-- Hasura's per-row RLS subqueries.
DEFINE TABLE member TYPE RELATION FROM user TO context SCHEMAFULL;
DEFINE FIELD role   ON member TYPE string DEFAULT 'member' ASSERT $value IN ['member', 'owner'];
DEFINE FIELD active ON member TYPE bool DEFAULT true;
DEFINE FIELD email  ON member TYPE option<string>;   -- the invite (roster) address
DEFINE INDEX member_unique ON member FIELDS in, out UNIQUE;

-- Comments: internal discussion, threaded via `parent`.
DEFINE TABLE comment SCHEMAFULL;
DEFINE FIELD on         ON comment TYPE record;             -- document/comment it replies to
DEFINE FIELD context    ON comment TYPE record<context>;
DEFINE FIELD author     ON comment TYPE record<user>;
DEFINE FIELD text       ON comment TYPE string;
DEFINE FIELD created_at ON comment TYPE datetime DEFAULT time::now();

-- === Voting (carries over the interim anonymity hardening as first-class schema) ===
DEFINE TABLE poll SCHEMAFULL;
DEFINE FIELD context    ON poll TYPE record<context>;
DEFINE FIELD question   ON poll TYPE string;
DEFINE FIELD options    ON poll TYPE array<string>;
DEFINE FIELD open       ON poll TYPE bool DEFAULT true;
DEFINE FIELD secret     ON poll TYPE bool DEFAULT false;
DEFINE FIELD created_at ON poll TYPE datetime DEFAULT time::now();

-- WHO voted (dedup) — no choice stored. One row per (poll, voter).
DEFINE TABLE voted SCHEMAFULL;
DEFINE FIELD poll  ON voted TYPE record<poll>;
DEFINE FIELD voter ON voted TYPE record<user>;
DEFINE INDEX voted_once ON voted FIELDS poll, voter UNIQUE;

-- WHAT was chosen — no voter, append-only, coarse timestamp (poll's, not cast time).
-- Nothing links a `ballot` to a `voted` row: same anonymity design as the interim
-- fix, now enforced by the schema itself.
DEFINE TABLE ballot SCHEMAFULL;
DEFINE FIELD poll        ON ballot TYPE record<poll>;
DEFINE FIELD choices     ON ballot TYPE array<int>;
DEFINE FIELD cast_bucket ON ballot TYPE datetime;   -- = poll.created_at, NOT the real cast time
```

**Casting a secret ballot** (atomic; robust regardless of isolation guarantees —
the de-risking from the port doc):

```surql
BEGIN;
  CREATE voted SET poll = $poll, voter = $me;   -- UNIQUE index rejects a second vote
  CREATE ballot SET poll = $poll, choices = $choices, cast_bucket = $poll.created_at;
COMMIT;
```

**Tally** — always recomputed by aggregation, never a mutable counter:

```surql
SELECT choices, count() AS n FROM ballot WHERE poll = $poll GROUP BY choices;
```

**Membership checks via graph** (replaces Hasura's `is_context_owner` subqueries):

```surql
-- active members of a context:
SELECT in.* FROM member WHERE out = $ctx AND active = true;
-- is $me an active owner of $ctx?
SELECT * FROM member WHERE in = $me AND out = $ctx AND role = 'owner' AND active = true;
```

Ephemeral coordination (the `active` projector node, `screenComments`, speaker
lists) is small mutable state — a `projector` / `speaker_entry` table or even
per-context fields; not modelled here since it's transient.

## Identity binding (email invite → DID)

The roster constraint is unchanged (Excel, keyed by email). Flow:

1. Import roster → `member` edges with `email` set, `active=false`, no `user`.
2. A person authenticates with their **DID** (atproto OAuth).
3. Bind: match the pending `member` by email (or a claim-token for
   mismatched-email cases, as today), set its `in = user:<did>`, `active=true`.

`member.email` stays the invite address; the DID is the durable identity. This is
the current `members.node_id` pattern, re-pointed at DIDs.

## AppView / materialisation

- Consume **Jetstream**, filtered to `wiki.radikal.*` + relevant `app.bsky.*`.
- On a `wiki.radikal.statement` / `resolution` record → upsert a row and link it
  (`document.published_uri`); on delete → unlink.
- **Publishing** (internal → public) writes the record to the repo via `atrium`,
  then the firehose echoes it back for materialisation.
- Client realtime is SurrealDB **LIVE queries** (validated) with
  **refetch-on-reconnect**.

## Open questions

- Do `document` sub-kinds (policy/position/candidate/change) deserve their own
  tables, or is a `kind` tag enough? (Start with the tag; split if queries get ugly.)
- Ephemeral state (projector/speaker) — SurrealDB LIVE queries or a lighter
  channel? LIVE is probably fine and keeps it in one store.
- Org DID custody — who holds the org's signing key that publishes resolutions?
  (Ties to the run-your-own-PDS decision in `atproto-port.md §7.2`.)
