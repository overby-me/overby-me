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

## The headline realisation

Running the split across every entity produces one clear conclusion:

> **This is a DID-authenticated *private* app with a thin, optional public
> publishing layer — not an atproto-native app.**

Because atproto records are public and the app is internal political
deliberation + voting, almost everything is private org-authoritative state.
atproto's real jobs are narrow: **(1) identity (DID)** and **(2) optional public
publishing** (a member or the org choosing to publish a statement/resolution).
That's not a downgrade — it's the honest shape, and it means the backend DB is
the primary substrate and the lexicon surface is deliberately tiny.

## Source-of-truth split, per entity

| Current (`mimeId`) | What it is | Public atproto record? | Private DB (org-authoritative)? |
|---|---|---|---|
| user | a person | DID = identity; public profile optional | membership/roles **private** |
| `wiki/group`, `wiki/event` | org chapter / meeting | no | **yes** (container; content private) |
| `wiki/folder` | organising container | no | **yes** |
| `wiki/document`, `vote/policy`, `vote/position`, `vote/candidate`, `vote/change` | wiki content / proposals | **optional** publication | **yes** (draft/internal by default) |
| `wiki/file` | attachment (blob) | no | **yes** (private storage) |
| `vote/poll` | official ballot | no | **yes** (authoritative: options, open/closed, tally) |
| `vote/vote` | a cast ballot | **no** (secret; can't be public/DID-signed) | **yes** (anonymised) |
| `has_voted` | one-vote dedup | no | **yes** (anonymity mechanism) |
| `vote/comment` | discussion | optional public reply | **yes** (internal) |
| `speak/list`, `speak/speak` | speaker queue | no | **yes** (ephemeral coordination) |
| members | roster + roles | no (public membership would *out* members) | **yes** (org signs it, not self-asserted) |
| relations (`active`, `screenComments`) | projector state | no | **yes** (ephemeral) |

Everything operational is private. The only genuine public records are
publications (below).

## Lexicons (the public surface)

Two record types, on two kinds of identity:

- A **member DID** publishes a personal **statement**.
- The **org's own DID** (the service has its own repo) publishes an official
  **resolution** — the *outcome* of a vote (title + tally), which is
  publicly verifiable while individual ballots stay private.

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

Publishing writes the record to a repo via `atrium`; the AppView also ingests it
back from Jetstream to link it to the internal artifact (`document.published_uri`).

## Domain model (private, org-authoritative — SurrealQL)

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
DEFINE FIELD published_uri ON document TYPE option<string>;      -- set if published to a repo
DEFINE FIELD created_at    ON document TYPE datetime DEFAULT time::now();

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
