//! AppView-owned RUNTIME tables that are NOT part of the migrated entity subset
//! (`wiki_schema::ENTITY_SCHEMA`, the content/membership half the big-bang import
//! carries). These hold operational state the AppView itself creates and
//! discards at runtime: Web Push device subscriptions (a member re-subscribes a
//! device) and the durable atrium-oauth state/session stores (an OAuth login
//! round-trip). Nothing here is migrated from the interim backend, so it lives
//! beside the entity schema rather than in the generated DDL.

/// The runtime infra DDL, in the SQLite dialect (Turso's primary frontend).
/// Applied after `wiki_schema::ENTITY_SCHEMA` by [`crate::db::Db::init_schema`].
/// Idempotent (`IF NOT EXISTS`) so a persistent-file process can re-run it on
/// restart without erroring.
pub const RUNTIME_DDL: &str = r#"
-- Durable atrium-oauth StateStore: the pre-redirect PKCE verifier + DPoP key +
-- issuer, keyed by the OAuth `state` nonce; value is JSON of atrium's
-- InternalStateData. Replaces the in-memory MemoryStateStore so a login survives
-- a process restart between authorize and callback.
CREATE TABLE IF NOT EXISTS oauth_state (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Durable atrium-oauth SessionStore: the post-exchange DPoP key + token set,
-- keyed by the account DID; value is JSON of atrium's Session.
CREATE TABLE IF NOT EXISTS oauth_session (
  did   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Browser sessions (src/session.rs). token_hash is SHA-256 of the bearer token,
-- never the token; the two timestamps are Unix seconds.
CREATE TABLE IF NOT EXISTS session (
  token_hash TEXT PRIMARY KEY,
  did        TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS session_by_did ON session(did);

-- One-time login codes: what /callback hands the browser in place of a session
-- token, so no credential ever sits in a URL. Hashed like the session tokens.
CREATE TABLE IF NOT EXISTS login_code (
  code_hash  TEXT PRIMARY KEY,
  did        TEXT NOT NULL,
  expires_at INTEGER NOT NULL
);
"#;
