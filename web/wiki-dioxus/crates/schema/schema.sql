-- The ENTITY SUBSET of the target schema (user, context, document, post,
-- member, comment), extracted verbatim from docs/atproto-domain-model.md's
-- "DB realisation" section and made executable, plus a legacy_id column per
-- imported table (the interim Postgres uuid, so the big-bang importer is
-- idempotent and the field-gap report can join back).
--
-- PROVISIONAL dialect-validation artifact and dev-harness seed: the canonical
-- DDL is to be re-derived from the Rust domain types per the stack decision
-- (atproto-stack-decisions.md, "derive the DB DDL from the Rust types"); this
-- file exists to prove the SQLite-dialect claim, constraint enforcement, and
-- the JSON-column choices before any migration code is written. The voting
-- tables (poll, eligibility, delegation, token_issued, board_entry) join once
-- their shapes settle with the ballot spec.
--
-- Run with PRAGMA foreign_keys=ON: SQLite leaves FK enforcement OFF per
-- connection by default, and the tests assert both behaviours.

-- Identity: the DID IS the person (primary key = the DID).
CREATE TABLE user (
  did          TEXT PRIMARY KEY,
  handle       TEXT,
  display_name TEXT,
  avatar_url   TEXT,
  legacy_id    TEXT UNIQUE                               -- interim users.id (uuid)
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
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id     TEXT UNIQUE                              -- interim nodes.id (uuid)
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
  author_did    TEXT REFERENCES user(did),
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,                                    -- the at-uri, once published
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id     TEXT UNIQUE                              -- interim nodes.id (uuid)
);
CREATE INDEX document_context ON document(context_id, parent_id);

-- Feed posts: the social unit. visibility='public' -> mirrored to a repo.
CREATE TABLE post (
  id            TEXT PRIMARY KEY,
  author_did    TEXT NOT NULL REFERENCES user(did),
  group_id      TEXT REFERENCES context(id),            -- posted in a group/event
  reply_to      TEXT REFERENCES post(id),               -- thread parent
  text          TEXT NOT NULL,
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id     TEXT UNIQUE
);
CREATE INDEX post_feed ON post(group_id, created_at);   -- feed by group + time

-- Membership as a join table (user <-> context). Surrogate key plus partial
-- uniques: 83 percent of members import as email-only pending invites with
-- user_did NULL, and NULL PK parts are each distinct in SQLite, so a
-- (user_did, context_id) PK would silently unenforce dedup for exactly the
-- dominant case. See atproto-domain-model.md for the full rationale.
CREATE TABLE member (
  id          TEXT PRIMARY KEY,
  user_did    TEXT REFERENCES user(did),                 -- NULL until the invite is claimed
  context_id  TEXT NOT NULL REFERENCES context(id),
  role        TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member','owner')),
  active      INTEGER NOT NULL DEFAULT 1,
  email       TEXT,                                      -- the invite (roster) address
  claim_token TEXT UNIQUE,                               -- secret for mismatched-email claims
  legacy_id   TEXT UNIQUE                                -- interim members.id (uuid)
);
CREATE UNIQUE INDEX member_bound   ON member(context_id, user_did) WHERE user_did IS NOT NULL;
CREATE UNIQUE INDEX member_pending ON member(context_id, email)    WHERE user_did IS NULL;
CREATE INDEX member_by_context ON member(context_id, active);

-- Comments: internal discussion, threaded via on_id.
CREATE TABLE comment (
  id         TEXT PRIMARY KEY,
  on_id      TEXT NOT NULL,                              -- document/comment it replies to
  context_id TEXT NOT NULL REFERENCES context(id),
  author_did TEXT NOT NULL REFERENCES user(did),
  text       TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id  TEXT UNIQUE
);
