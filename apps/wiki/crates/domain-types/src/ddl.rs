//! The canonical entity-subset DDL, derived from the domain types in this crate
//! (the Rust types are the source of truth per `docs/atproto-stack-decisions.md`;
//! this is the generated artifact, replacing the previously hand-authored
//! `crates/schema/schema.sql`, so the two can no longer drift).
//!
//! Reconciled to the census + extractor reality:
//! - authorship is DID-OR-free-text everywhere (42% of author chips are
//!   free-text with only a third name-recoverable), so `author_did` is nullable
//!   and paired with `author_text`, guarded by a CHECK that one is present;
//! - a document has MANY authors (up to 8), so authorship is a
//!   `document_author` join table, not a scalar `document.author_did`;
//! - every table carries `legacy_id UNIQUE` for idempotent big-bang import;
//! - `context` and `document` are the two spines of ONE tree (`Place`). The
//!   frontend reaches every node by the path in its URL, so both carry a stored
//!   `path`, unique among live rows, and `parent_id` is a plain column on both
//!   because a parent may be either kind.
//!
//! Voting entities (poll, eligibility, delegation, token_issued, board_entry)
//! are intentionally NOT here: they settle with the ballot spec and are added by
//! the ballot service, not the content/membership migration.

/// The reconciled entity-subset schema, in the SQLite dialect (Turso's primary
/// frontend and the bridge target). Its foreign keys bind only on a connection
/// that has set `PRAGMA foreign_keys=ON`; both engines default it off.
pub const DDL: &str = r#"
-- Identity: the DID IS the person.
CREATE TABLE user (
  did          TEXT PRIMARY KEY,
  handle       TEXT,
  display_name TEXT,
  avatar_url   TEXT,
  legacy_id    TEXT UNIQUE
);

-- Contexts: groups, events and sites (the org's structures). parent_id names a
-- context OR a document (a group can sit in a folder), so it is not a foreign
-- key; the write path keeps it honest.
CREATE TABLE context (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('group','event','site')),
  name          TEXT NOT NULL,
  slug          TEXT NOT NULL,
  path          TEXT NOT NULL,
  parent_id     TEXT,
  idx           INTEGER NOT NULL DEFAULT 0,
  attachable    INTEGER NOT NULL DEFAULT 1,
  owner_did     TEXT REFERENCES user(did),
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
  deleted_at    TEXT,
  deleted_root  TEXT,
  legacy_id     TEXT UNIQUE
);
-- Live rows only: a node in the bin does not hold its URL hostage.
CREATE UNIQUE INDEX context_path_live ON context(path) WHERE deleted_at IS NULL;
CREATE INDEX context_children ON context(parent_id, idx);

-- Content: documents / folders / files / proposals (kind-tagged). Authorship is
-- in the document_author join table below, not a scalar column here.
CREATE TABLE document (
  id            TEXT PRIMARY KEY,
  context_id    TEXT NOT NULL REFERENCES context(id),
  kind          TEXT NOT NULL,
  title         TEXT NOT NULL,
  slug          TEXT NOT NULL,
  path          TEXT NOT NULL,
  parent_id     TEXT,
  idx           INTEGER NOT NULL DEFAULT 0,
  mutable       INTEGER NOT NULL DEFAULT 1,
  attachable    INTEGER NOT NULL DEFAULT 1,
  owner_did     TEXT REFERENCES user(did),
  content       TEXT,
  data          TEXT,
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
  deleted_at    TEXT,
  deleted_root  TEXT,
  legacy_id     TEXT UNIQUE
);
CREATE UNIQUE INDEX document_path_live ON document(path) WHERE deleted_at IS NULL;
CREATE INDEX document_children ON document(parent_id, idx);
CREATE INDEX document_context ON document(context_id);

-- A document's authors: many per document, each a DID (an account) OR a
-- free-text display name (no account), never a single scalar author_did.
CREATE TABLE document_author (
  document_id TEXT NOT NULL REFERENCES document(id),
  author_did  TEXT REFERENCES user(did),
  author_text TEXT,
  ord         INTEGER NOT NULL DEFAULT 0,
  CHECK (author_did IS NOT NULL OR author_text IS NOT NULL)
);
CREATE INDEX document_author_by_doc ON document_author(document_id);

-- Feed posts: the social unit. Authorship is free-text-capable (a migrated post
-- may have a free-text author with no account).
CREATE TABLE post (
  id            TEXT PRIMARY KEY,
  author_did    TEXT REFERENCES user(did),
  author_text   TEXT,
  group_id      TEXT REFERENCES context(id),
  reply_to      TEXT REFERENCES post(id),
  text          TEXT NOT NULL,
  visibility    TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  published_uri TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id     TEXT UNIQUE,
  CHECK (author_did IS NOT NULL OR author_text IS NOT NULL)
);
CREATE INDEX post_feed ON post(group_id, created_at);

-- Membership as a join table. Surrogate id + partial uniques: 83% of members
-- import as email-only pending invites with user_did NULL, and NULL PK parts are
-- each distinct in SQLite, so a (user_did, context_id) PK would silently
-- unenforce dedup for exactly the dominant case.
CREATE TABLE member (
  id          TEXT PRIMARY KEY,
  user_did    TEXT REFERENCES user(did),
  context_id  TEXT NOT NULL REFERENCES context(id),
  role        TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member','owner')),
  active      INTEGER NOT NULL DEFAULT 1,
  email       TEXT,
  claim_token TEXT UNIQUE,
  legacy_id   TEXT UNIQUE
);
CREATE UNIQUE INDEX member_bound   ON member(context_id, user_did) WHERE user_did IS NOT NULL;
CREATE UNIQUE INDEX member_pending ON member(context_id, email)    WHERE user_did IS NULL;
CREATE INDEX member_by_context ON member(context_id, active);

-- Comments: internal discussion, threaded via on_id. Free-text-capable author.
CREATE TABLE comment (
  id          TEXT PRIMARY KEY,
  on_id       TEXT NOT NULL,
  context_id  TEXT NOT NULL REFERENCES context(id),
  author_did  TEXT REFERENCES user(did),
  author_text TEXT,
  text        TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id   TEXT UNIQUE,
  CHECK (author_did IS NOT NULL OR author_text IS NOT NULL)
);

-- Emoji reactions on content (a member's react to a comment/post/resolution),
-- addressed by the subject's at-uri. NET-NEW (no interim source rows): the old
-- wiki had no reactions, so nothing migrates here; rows arrive from the public
-- reaction records via the firehose. One reaction per (subject, reactor, emoji);
-- deleting the record removes the row (toggle).
CREATE TABLE reaction (
  id          TEXT PRIMARY KEY,                    -- the reaction record's at-uri
  subject_uri TEXT NOT NULL,                       -- the reacted-to record's at-uri
  reactor_did TEXT REFERENCES user(did),
  emoji       TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  legacy_id   TEXT UNIQUE
);
CREATE INDEX reaction_by_subject ON reaction(subject_uri);
CREATE UNIQUE INDEX reaction_once ON reaction(subject_uri, reactor_did, emoji);
"#;
