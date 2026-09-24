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
    /// A group or an event named as the author, which is how a local branch
    /// puts a motion forward. `name` and `path` are filled in when it is read,
    /// and ignored when it is written.
    Context {
        context_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

impl Author {
    /// The author's DID, if they are a resolved account (maps to `author_did`).
    pub fn did(&self) -> Option<&str> {
        match self {
            Author::User { did } => Some(did),
            _ => None,
        }
    }

    /// The author's free-text display, if they have no account (maps to
    /// `author_text`). Exactly one of `did()`, `text()` and `context()` is
    /// `Some`, which is what the table's CHECK asks for.
    pub fn text(&self) -> Option<&str> {
        match self {
            Author::FreeText { display } => Some(display),
            _ => None,
        }
    }

    /// The group or event, if that is who the author is (`author_context`).
    pub fn context(&self) -> Option<&str> {
        match self {
            Author::Context { context_id, .. } => Some(context_id),
            _ => None,
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

/// A place with its own members: a standing body, a meeting, or a site that
/// publishes and may have no members at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    /// The one place everything else is under: its owners run the site, and its
    /// path is the empty one.
    Home,
    Group,
    Event,
    Site,
}

/// Visibility: public content is mirrored to a repo; private stays DB-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Public,
}

/// Where a node sits in the tree. `context` and `document` both carry one: the
/// frontend knows a single tree and reaches every node by the path in its URL.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Place {
    /// The URL segment, unique among live siblings. The interim `key`.
    pub slug: String,
    /// Slash-joined slugs from the root. Stored, and rewritten on a rename or a
    /// move, because nothing can walk the tree in one query.
    pub path: String,
    /// A context or a document: a group can sit in a folder.
    pub parent_id: Option<String>,
    /// Manual order among siblings.
    #[serde(default)]
    pub idx: i64,
    /// Whether children may be added (the folder lock).
    #[serde(default = "yes")]
    pub attachable: bool,
    /// Who created it. Distinct from authorship, which a document lists.
    pub owner_did: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Set while the node is in the bin.
    pub deleted_at: Option<String>,
    /// The node whose deletion took this one to the bin: itself, or an ancestor.
    /// A restore brings back exactly the rows that share one, so it never digs
    /// up what was deleted earlier from inside the same folder.
    #[serde(default)]
    pub deleted_root: Option<String>,
}

fn yes() -> bool {
    true
}

/// A context (group/event/site), the org's organizing structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub id: String,
    pub kind: ContextKind,
    pub name: String,
    #[serde(flatten)]
    pub place: Place,
    /// What the place says about itself (Slate JSON). Left out of listings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    /// What it holds beside that: a cover image's file id, a redirect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub visibility: Visibility,
    pub published_uri: Option<String>,
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
    /// A poll's place in the tree. What is voted on, and how it went, is its
    /// `poll` row (`ballot-store`), which shares this document's id.
    Poll,
    /// A shared pixel canvas's place in the tree. Its size and its cells are the
    /// AppView's `canvas` tables.
    Canvas,
}

/// A content node: document / folder / file / proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub context_id: String,
    pub kind: DocumentKind,
    pub title: String,
    #[serde(flatten)]
    pub place: Place,
    /// Whether it may still be edited. Submitting a motion clears it.
    #[serde(default = "yes")]
    pub mutable: bool,
    /// Slate JSON, carried over verbatim (parsed only at the publish seam).
    pub content: Option<serde_json::Value>,
    /// What the interim `data` blob held beside `content`: a file's id and type,
    /// a cover image. An object, or absent.
    pub data: Option<serde_json::Value>,
    /// Authors: possibly several (the census found up to 8 per node), each a
    /// DID or a free-text fallback. Replaces the single nullable author_did the
    /// census showed to be insufficient.
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub visibility: Visibility,
    pub published_uri: Option<String>,
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
    /// Voting rights, which an owner grants. Not membership: see `accepted`.
    pub active: bool,
    /// The roster's name for them. For a pending invitation it is the only
    /// label there is, since no account stands behind the row yet.
    #[serde(default)]
    pub name: Option<String>,
    /// Kept off the member list everyone sees.
    #[serde(default)]
    pub hidden: bool,
    /// Whether they have said yes. An invitation by account is bound from the
    /// start, so being bound does not say it.
    #[serde(default)]
    pub accepted: bool,
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
    /// The document its whole thread hangs on: `on_id`, or for a reply its
    /// parent's.
    #[serde(default)]
    pub root_id: String,
    pub context_id: String,
    pub author: Author,
    pub text: String,
    /// An attached picture's file id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Emptied by its author or an owner, and kept because replies hang on it:
    /// no text, no author, no picture.
    #[serde(default)]
    pub tombstone: bool,
    pub created_at: Option<String>,
    /// Set while it is in the bin, along with the comment whose deletion took
    /// it there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_root: Option<String>,
    pub legacy_id: Option<String>,
}

/// A poll as it is carried over: what was asked, under which rules, and how it
/// went. Its ballots are not carried (the tokens that made them anonymous do not
/// survive), so its `counts` are: the outcome of every past vote is a record the
/// organisation keeps. A poll also has a place in the tree, which is a
/// [`Document`] of kind [`DocumentKind::Poll`] with the same id.
///
/// Always migrated CLOSED. One that was open at the dump cannot take another
/// ballot in the new scheme, and its issuer key is minted at opening, not carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub id: String,
    pub context_id: String,
    pub question: String,
    pub options: Vec<String>,
    pub min: u32,
    pub max: u32,
    /// The last option is the abstention, which the interim always appends.
    pub blank: bool,
    pub secret: bool,
    /// Counts are for the context's owners (the interim's `hidden`).
    pub hide_tally: bool,
    /// One count per option.
    pub counts: Vec<u64>,
    pub ballots: u64,
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub legacy_id: Option<String>,
}

/// A shared pixel canvas and what was painted on it. Its place in the tree is a
/// [`Document`] of kind [`DocumentKind::Canvas`] with the same id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    pub id: String,
    pub width: u32,
    pub height: u32,
    /// Seconds between one person's placements.
    pub cooldown: u32,
    pub open: bool,
    pub cells: Vec<CanvasCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasCell {
    pub x: u32,
    pub y: u32,
    /// An index into the client's palette.
    pub colour: u8,
    pub painter_did: Option<String>,
    pub painted_at: Option<String>,
}

/// A report: a person's account of a bug or a wish, or a crash folded from every
/// time it was seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feedback {
    pub id: String,
    pub kind: String,
    pub message: String,
    pub path: String,
    pub app_version: String,
    pub commit: String,
    pub user_agent: String,
    /// A screenshot's file id.
    pub image: Option<String>,
    /// What a crash is folded by.
    pub digest: Option<String>,
    pub seen: u64,
    /// Everyone who has hit a folded crash: DIDs, and `anonymous` at most once.
    pub reporters: Vec<String>,
    pub owner_did: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// An emoji reaction, one per `(subject_uri, reactor_did, emoji)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub id: String,
    /// The id of the comment or document reacted to, or the at-uri of a public
    /// record.
    pub subject_uri: String,
    /// `None` for a reactor whose account did not come across.
    pub reactor_did: Option<String>,
    /// A single emoji grapheme.
    pub emoji: String,
    pub created_at: Option<String>,
    pub legacy_id: Option<String>,
}
