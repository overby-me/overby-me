# Domain model & lexicons (atproto rewrite)

A first cut of the data model for the custom backend, derived from the current
`mimeId` taxonomy and the source-of-truth split in
[`atproto-port.md`](./atproto-port.md). Two layers:

- **Lexicons** — the (small) surface of *public* atproto records in user repos.
- **Domain model** — the (large) *private, org-authoritative* store in the
  backend DB (Turso, per the tech-stack decision). Written as concrete SQL (SQLite
  dialect) as a starting point.

> Draft, not committed. NSID `com.example.wiki.*` is a deliberate RFC 2606 placeholder:
> the authority domain is not decided yet (an Open entry in `atproto-open-decisions.md`);
> the rebrand procedure is documented in `lexicons/README.md`.

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

## Lexicon scope: boundary-only (decided 2026-07-16, closes OPEN-1)

atproto earns its place on two counts, independent of how public the app is:

1. **Identity.** The DID is the primary identity; atproto OAuth is the login:
   portable, password-less, and it de-risks the hardest migration (member to DID).
   True even for a mostly-private app.
2. **Lexicons are canonical at the federation boundary ONLY.** The public subset
   (post, statement, resolution, public group/event/document, comment) is governed
   by `com.example.wiki.*` lexicons: `atrium` codegens the Rust record types, and the
   lexicon is the published, versioned contract every federated record must obey.
   The always-private entities (ballot, eligibility/delegation, voted-dedup,
   membership-as-affiliation, projector/speaker state) get NO lexicon: hand-authored
   Rust serde types are their canonical schema, and the DB DDL derives from those
   types. Genuinely dual-form entities carry a public lexicon plus a separate
   private Rust type, mapped at an explicit publish/materialize seam.

This supersedes the "lexicons model ALL the data" stance this section previously
documented. The losing rationale, recorded: one uniform codegen pipeline was
attractive, but Lexicon cannot express the private half's uniqueness, cross-field,
and anonymity invariants, has only closed string enums (no algebraic sum types),
and would publish schema contracts for records that never federate, taxing
private-side iteration with versioning ceremony. **Turso then realises the private
entities** as SQL tables (join tables, foreign keys, indexes) derived from the Rust
types, while the lexicons stay the source of truth for the public record shapes.

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
| `vote/vote` ballot | a cast ballot | **split** | the org-side row (eligibility, issuance) is always-private; the ANONYMISED board entry (token + choices, no voter) IS a public record (custody: pending owner call) |
| token issuance / dedup | one-issuance-per-voter marker | **always-private** | — (dedup itself is public token uniqueness on the board) |
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
  "id": "com.example.wiki.post",
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
  "id": "com.example.wiki.statement",
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
  "id": "com.example.wiki.resolution",
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

## DB realisation (SQL, on Turso)

The Turso tables *realise* the lexicon-defined entities (same shapes, codegen-shared with the record types)
and add what lexicons do not express: the relational structure (membership and references as join tables and
foreign keys), indexes, and the always-private tables (ballots, dedup, roster). `visibility` marks which rows
also exist as published records. SQL is shown in the SQLite dialect (Turso's primary frontend); arrays and
Slate content are JSON columns and timestamps are text.

The **canonical** entity-subset DDL is now *generated* from the Rust domain types
(`crates/domain-types/src/ddl.rs`, re-exported as `wiki_schema::ENTITY_SCHEMA`) and validated on both
engines by `crates/schema/tests/roundtrip.rs`, so schema and types can no longer drift. The SQL below is the
illustrative narrative of that artifact; where it differs, the generated DDL wins. Two reconciliations the
census forced are reflected here: authorship is **DID-or-free-text** everywhere (42 percent of author chips
are free-text), and a document has **many** authors (up to 8), so authorship is a `document_author` join
table, not a scalar `document.author_did`. Every entity table also carries `legacy_id TEXT UNIQUE` for the
idempotent big-bang import (elided below for readability).

```sql
-- Identity: the DID IS the person (primary key = the DID).
CREATE TABLE user (
  did          TEXT PRIMARY KEY,
  handle       TEXT,
  display_name TEXT,
  avatar_url   TEXT
);

-- Contexts: groups & events (the org's structures). Hierarchy via parent_id.
CREATE TABLE context (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('group','event')),
  name          TEXT NOT NULL,
  slug          TEXT NOT NULL,
  parent_id     TEXT REFERENCES context(id),
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,                                    -- the at-uri, if the group/event is public
  created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX context_slug ON context(parent_id, slug);

-- Content: documents / folders / files / proposals (kind-tagged).
CREATE TABLE document (
  id            TEXT PRIMARY KEY,
  context_id    TEXT NOT NULL REFERENCES context(id),
  parent_id     TEXT,                                    -- folder or context
  kind          TEXT NOT NULL,                           -- document|folder|file|policy|position|candidate|change
  title         TEXT NOT NULL,
  content       TEXT,                                    -- Slate JSON (carries over)
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,                                    -- the at-uri, once published
  created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX document_context ON document(context_id, parent_id);

-- A document's authors: many per document (census: up to 8), each a DID (an
-- account) OR a free-text display name (no account), never a scalar author_did.
CREATE TABLE document_author (
  document_id TEXT NOT NULL REFERENCES document(id),
  author_did  TEXT REFERENCES user(did),
  author_text TEXT,                                      -- free-text name (no account)
  ord         INTEGER NOT NULL DEFAULT 0,
  CHECK (author_did IS NOT NULL OR author_text IS NOT NULL)
);
CREATE INDEX document_author_by_doc ON document_author(document_id);

-- Feed posts: the social unit. visibility='public' -> mirrored to a repo.
CREATE TABLE post (
  id            TEXT PRIMARY KEY,
  author_did    TEXT REFERENCES user(did),              -- DID-or-free-text (a migrated
  author_text   TEXT,                                   --   post may have no account)
  group_id      TEXT REFERENCES context(id),            -- posted in a group/event
  reply_to      TEXT REFERENCES post(id),               -- thread parent
  text          TEXT NOT NULL,
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  CHECK (author_did IS NOT NULL OR author_text IS NOT NULL)
);
CREATE INDEX post_feed ON post(group_id, created_at);   -- feed by group + time

-- Membership as a join table (user <-> context), indexed both ways. Replaces
-- Hasura's per-row RLS subqueries with a plain indexed join.
--
-- Keyed by a surrogate id, NOT (user_did, context_id): the DID audit found 83
-- percent of members are email-only roster rows that import with user_did NULL,
-- and NULL primary-key parts are each distinct in SQLite (dedup silently
-- unenforced) and rejected outright in stricter dialects. Partial unique
-- indexes enforce each state's real invariant instead: one bound row per
-- (context, DID), one PENDING invite per (context, email). Once a row binds a
-- DID it leaves the email index's scope BY DESIGN; re-inviting that email then
-- creates a fresh pending row, and the application (not the schema) must first
-- check "already an active member?" to avoid duplicate memberships. That check
-- is the same is_active_member predicate the AppView authz core already needs.
CREATE TABLE member (
  id          TEXT PRIMARY KEY,
  user_did    TEXT REFERENCES user(did),                  -- NULL until the invite is claimed
  context_id  TEXT NOT NULL REFERENCES context(id),
  role        TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member','owner')),
  active      INTEGER NOT NULL DEFAULT 1,
  email       TEXT,                                       -- the invite (roster) address
  claim_token TEXT UNIQUE                                 -- secret for mismatched-email claims
);
CREATE UNIQUE INDEX member_bound   ON member(context_id, user_did) WHERE user_did IS NOT NULL;
CREATE UNIQUE INDEX member_pending ON member(context_id, email)    WHERE user_did IS NULL;
CREATE INDEX member_by_context ON member(context_id, active);

-- Comments: internal discussion, threaded via on_id.
CREATE TABLE comment (
  id          TEXT PRIMARY KEY,
  on_id       TEXT NOT NULL,                             -- document/comment it replies to
  context_id  TEXT NOT NULL REFERENCES context(id),
  author_did  TEXT REFERENCES user(did),                -- DID-or-free-text (a migrated
  author_text TEXT,                                     --   comment may have no account)
  text        TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  CHECK (author_did IS NOT NULL OR author_text IS NOT NULL)
);

-- === Voting: the decided E2E-V scheme (blind-signature UNIT tokens, PER-POLL
-- issuer keys, a public bulletin board; see atproto-open-decisions.md). This
-- replaces the interim voter_did dedup + cast_bucket design: dedup is now
-- token-uniqueness on the board, and nothing in the store links a ballot to a
-- DID. Crypto field encodings (token bytes, signature format) are PROVISIONAL
-- until the ballot-math spec crate pins the message format, and the board's
-- custody (voter-published vs org-published records) is a pending owner call.
CREATE TABLE poll (
  id            TEXT PRIMARY KEY,
  context_id    TEXT NOT NULL REFERENCES context(id),
  question      TEXT NOT NULL,
  options       TEXT NOT NULL,                           -- JSON array of strings
  open          INTEGER NOT NULL DEFAULT 1,
  secret        INTEGER NOT NULL DEFAULT 0,
  -- Per-poll RSA issuer keypair: the pubkey is published to the board BEFORE
  -- the poll opens (verifiability); the private key is dropped at close.
  issuer_pubkey TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- WHO MAY vote, org-authoritative and always-private. base_weight is the
-- voter's own weight; resolved_weight = base plus incoming delegations, FROZEN
-- when the poll opens (delegation changes after open do not move weight).
CREATE TABLE eligibility (
  poll_id         TEXT NOT NULL REFERENCES poll(id),
  did             TEXT NOT NULL REFERENCES user(did),
  base_weight     INTEGER NOT NULL DEFAULT 1,
  resolved_weight INTEGER,                               -- set at open; NULL until frozen
  PRIMARY KEY (poll_id, did)
);

-- Delegations: a signed assignment moving a voter's weight to a delegate,
-- resolved into eligibility.resolved_weight BEFORE the poll opens. Visible to
-- the org, never on the public board (the delegation-vs-anonymity resolution).
CREATE TABLE delegation (
  poll_id        TEXT NOT NULL REFERENCES poll(id),
  from_did       TEXT NOT NULL REFERENCES user(did),
  to_did         TEXT NOT NULL REFERENCES user(did),
  assignment_sig TEXT NOT NULL,                          -- the delegator's signature
  PRIMARY KEY (poll_id, from_did)                        -- one outgoing delegation per voter
);

-- THAT a voter was issued their tokens (never the tokens themselves: storing a
-- token would link its later spend back to the DID and break unlinkability).
-- A voter with resolved_weight N is blind-issued N identical unit tokens.
CREATE TABLE token_issued (
  poll_id   TEXT NOT NULL REFERENCES poll(id),
  did       TEXT NOT NULL REFERENCES user(did),
  issued_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (poll_id, did)                             -- issuance happens once per voter
);

-- The org-side MIRROR of the public bulletin board, for tally and serving.
-- Append-only; one row per spent unit token; carries NO voter identity. The
-- UNIQUE token IS the double-vote rejection (a reused token collides here and
-- publicly on the board). Every entry weighs exactly 1 (unit tokens), so the
-- tally is a plain count and no weight column exists to shrink the anonymity
-- set. entry_ref points at the public board record once published.
CREATE TABLE board_entry (
  poll_id   TEXT NOT NULL REFERENCES poll(id),
  token     TEXT NOT NULL UNIQUE,                        -- the unblinded unit token
  token_sig TEXT NOT NULL,                               -- RSA-PSS sig under poll.issuer_pubkey
  choices   TEXT NOT NULL,                               -- JSON array of option indices
  entry_ref TEXT                                         -- at-uri/CID of the public record
);
CREATE INDEX board_by_poll ON board_entry(poll_id);
```

**Casting** (atomic; robust regardless of isolation guarantees). `BEGIN IMMEDIATE` takes the write lock up
front; the UNIQUE token constraint rejects a double spend in the same statement that records the ballot:

```sql
BEGIN IMMEDIATE;
  -- fails on a reused token: that IS the one-vote-per-token rule
  INSERT INTO board_entry (poll_id, token, token_sig, choices)
    VALUES (:poll, :token, :sig, :choices);
COMMIT;
```

(The signature is verified against `poll.issuer_pubkey` before the insert; the transaction shape, a
unique-constrained dedup insert plus an append-only ballot insert under `BEGIN IMMEDIATE`, is the same one
the Turso crash harness exercises.)

**Tally** — always recomputed by aggregation, never a mutable counter; unit tokens make it a plain count,
and anyone can recompute the same count from the public board (universal verifiability):

```sql
SELECT choices, count(*) AS n FROM board_entry WHERE poll_id = :poll GROUP BY choices;
```

**Membership checks** (replace Hasura's `is_context_owner` subqueries with an indexed join):

```sql
-- active members of a context:
SELECT u.* FROM member m JOIN user u ON u.did = m.user_did
  WHERE m.context_id = :ctx AND m.active = 1;
-- is :me an active owner of :ctx?
SELECT 1 FROM member
  WHERE user_did = :me AND context_id = :ctx AND role = 'owner' AND active = 1;
```

Ephemeral coordination (the `active` projector node, `screenComments`, speaker lists) is small mutable state,
a `projector` / `speaker_entry` table pushed over the in-process broadcast channel; not modelled here since it
is transient.

## Identity binding (email invite → DID)

The roster constraint is unchanged (Excel, keyed by email). Flow:

1. Import roster → `member` rows with `email` and a fresh `claim_token` set,
   `active=0`, `user_did` NULL (the pending-invite state the partial unique
   `member_pending` deduplicates).
2. A person authenticates with their **DID** (atproto OAuth).
3. Bind: match the pending `member` by email (or by `claim_token` for
   mismatched-email cases, as today), set `user_did = :did`, `active=1` (the row
   moves from the `member_pending` unique into `member_bound`).

`member.email` stays the invite address; the DID is the durable identity. This is
the current `members.node_id` pattern, re-pointed at DIDs.

## AppView / materialisation

- Consume **Jetstream**, filtered to `com.example.wiki.*` + relevant `app.bsky.*`.
- On a `com.example.wiki.statement` / `resolution` record → upsert a row and link it
  (`document.published_uri`); on delete → unlink.
- **Publishing** (internal → public) writes the record to the repo via `atrium`,
  then the firehose echoes it back for materialisation.
- Client realtime is an **in-process broadcast channel** (`tokio::sync::broadcast`)
  fed by the Turso write path, pushed over one axum WebSocket, with
  **refetch-on-reconnect**.

## Open questions

- Do `document` sub-kinds (policy/position/candidate/change) deserve their own
  tables, or is a `kind` tag enough? (Start with the tag; split if queries get ugly.)
- Ephemeral state (projector/speaker): a small mutable table pushed over the
  in-process broadcast, or an even lighter transient channel? The broadcast keeps it
  in one process alongside the store.
- Org DID custody — who holds the org's signing key that publishes resolutions?
  (Ties to the run-your-own-PDS decision in `atproto-port.md §7.2`.)
