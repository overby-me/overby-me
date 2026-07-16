//! Canonical backend-side domain types for the CONTENT and MEMBERSHIP half of
//! the model (round-2 item 17, deferrals c and d graduated). Hand-authored
//! Rust serde types, NOT bound to interim Hasura JSON: the extractor maps the
//! throwaway Postgres shapes INTO these, and the AppView reads/writes these.
//!
//! The public trio (post, and later statement/resolution) carry a lexicon and
//! their DB-side types are marked provisional (they get re-derived from
//! atrium-lex codegen at the rewrite); the private types (context, document,
//! member, comment) are canonical here per the boundary-only lexicon decision.
//!
//! DELIBERATELY EXCLUDED (no settled schema until the ballot spec and the
//! voting-SQL reconciliation land): poll, voted, ballot, eligibility,
//! delegation. See docs/pre-rewrite-plan.md Round 2.

use serde::{Deserialize, Serialize};

mod ddl;
pub use ddl::DDL;

/// A DID (the durable identity), or an unbound author fallback. The census
/// found 42 percent of author chips are free-text with only a third
/// name-recoverable, so an author is NOT always a DID: model that honestly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    /// A resolved account: the author IS a user (their DID once migrated, the
    /// interim node_id in the meantime).
    User { did: String },
    /// A free-text author name with no account (the 42 percent). Kept as a
    /// display string so authorship is not silently dropped at import.
    FreeText { display: String },
}

impl Author {
    /// The author's DID, if they are a resolved account (maps to `author_did`).
    pub fn did(&self) -> Option<&str> {
        match self {
            Author::User { did } => Some(did),
            Author::FreeText { .. } => None,
        }
    }

    /// The author's free-text display, if they have no account (maps to
    /// `author_text`). Exactly one of `did()`/`text()` is `Some`, matching the
    /// `author_did IS NOT NULL OR author_text IS NOT NULL` CHECK.
    pub fn text(&self) -> Option<&str> {
        match self {
            Author::FreeText { display } => Some(display),
            Author::User { .. } => None,
        }
    }
}

/// A person. `did` is the primary identity post-migration; during import it
/// holds the interim user id until the DID binding runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub did: String,
    pub handle: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// The interim `users.id` this row came from (import provenance).
    pub legacy_id: Option<String>,
}

/// A group or event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Group,
    Event,
}

/// Visibility: public content is mirrored to a repo; private stays DB-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Public,
}

/// A context (group/event), the org's organizing structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub id: String,
    pub kind: ContextKind,
    pub name: String,
    pub slug: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub visibility: Visibility,
    pub published_uri: Option<String>,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}

/// The content sub-kinds carried by the interim `mimeId` taxonomy, mapped onto
/// one `Document` with a kind tag (the domain model's "start with the tag"
/// decision).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Document,
    Folder,
    File,
    Policy,
    Position,
    Candidate,
    Change,
    Question,
}

/// A content node: document / folder / file / proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub context_id: String,
    pub parent_id: Option<String>,
    pub kind: DocumentKind,
    pub title: String,
    /// Slate JSON, carried over verbatim (parsed only at the publish seam).
    pub content: Option<serde_json::Value>,
    /// Authors: possibly several (the census found up to 8 per node), each a
    /// DID or a free-text fallback. Replaces the single nullable author_did the
    /// census showed to be insufficient.
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub visibility: Visibility,
    pub published_uri: Option<String>,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}

/// A feed post (the social unit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub id: String,
    pub author: Author,
    pub group_id: Option<String>,
    pub reply_to: Option<String>,
    pub text: String,
    #[serde(default)]
    pub visibility: Visibility,
    pub published_uri: Option<String>,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}

/// Membership role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Member,
    Owner,
}

/// A membership edge. `user_did` is None for the 83 percent pending email
/// invites; `claim_token` binds a mismatched-email invite. Matches the item-4
/// member DDL (surrogate id, partial uniques enforced at the DB).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub user_did: Option<String>,
    pub context_id: String,
    pub role: Role,
    pub active: bool,
    /// The invite address, normalized (lowercased, trimmed) at import: the
    /// census found 11 case/whitespace variant clusters.
    pub email: Option<String>,
    pub claim_token: Option<String>,
    pub legacy_id: Option<String>,
}

/// The pending-invite vs bound-member distinction the partial uniques encode.
impl Member {
    pub fn is_pending_invite(&self) -> bool {
        self.user_did.is_none()
    }
}

/// A comment (internal threaded discussion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    /// The document or comment it replies to.
    pub on_id: String,
    pub context_id: String,
    pub author: Author,
    pub text: String,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}

/// A poll (the question + options + open/secret state). The ONLY voting entity
/// that migrates: a `vote/poll` node carries reconstructable metadata, whereas a
/// cast ballot (`vote/vote`) is unmigratable (no eligibility/tokens survive). The
/// per-poll issuer key is minted fresh at open, not carried, so it is absent here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub id: String,
    pub context_id: String,
    pub question: String,
    pub options: Vec<String>,
    pub open: bool,
    pub secret: bool,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}

/// An emoji reaction to a content item, addressed by the subject's at-uri.
/// Net-new (no interim source); one per `(subject_uri, reactor_did, emoji)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    /// The reaction record's at-uri (the primary key).
    pub id: String,
    /// The at-uri of the record being reacted to.
    pub subject_uri: String,
    /// The reactor's DID (`None` only for a not-yet-realized user).
    pub reactor_did: Option<String>,
    /// A single emoji grapheme.
    pub emoji: String,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}
