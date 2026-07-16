//! Read-only migration extractor (round-2 item 17): maps the interim
//! Hasura/Postgres node+member shapes into the canonical `wiki-domain-types`
//! for the CONTENT and MEMBERSHIP half, and emits a FIELD-GAP REPORT (every
//! source column or JSONB key with no target slot, every required target with
//! no source). The mapping is the front half of the real importer the AppView
//! runs; the report is keeper knowledge even if re-run at cutover.
//!
//! This crate is PURE and hermetic: it operates on already-dumped interim rows
//! (fed as JSON), so the tests use synthetic fixtures and NOTHING here touches
//! the live DB. A live run is a separate, owner-approved step (dump the rows
//! with the census-style read-only script, pipe them into the `extract`
//! binary); no live PII is embedded in the crate.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wiki_domain_types::*;

/// An interim `nodes` row (the fields the census read), as dumped from Hasura.
#[derive(Debug, Clone, Deserialize)]
pub struct InterimNode {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(rename = "mimeId", default)]
    pub mime_id: Option<String>,
    #[serde(rename = "parentId", default)]
    pub parent_id: Option<String>,
    #[serde(rename = "contextId", default)]
    pub context_id: Option<String>,
    #[serde(rename = "ownerId", default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

/// An interim `members` row.
#[derive(Debug, Clone, Deserialize)]
pub struct InterimMember {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(rename = "nodeId", default)]
    pub node_id: Option<String>,
    #[serde(rename = "parentId", default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub owner: bool,
    #[serde(rename = "claimToken", default)]
    pub claim_token: Option<String>,
}

/// An interim `users` row (the account behind a `node_id`). Realized into a
/// domain `User` so the FK targets every author/member/comment references
/// actually exist. During migration `did` holds the interim user id until the
/// atproto DID binding runs (0 DIDs are linked today).
#[derive(Debug, Clone, Deserialize)]
pub struct InterimUser {
    pub id: String,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(rename = "avatarUrl", default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub handle: Option<String>,
}

const CONTEXT_MIMES: &[&str] = &["wiki/group", "wiki/event"];
const CONTENT_MIMES: &[&str] = &[
    "wiki/document",
    "vote/policy",
    "vote/change",
    "vote/position",
    "vote/candidate",
    "vote/question",
    "wiki/file",
    "wiki/folder",
];
const COMMENT_MIME: &str = "vote/comment";

/// A field-gap: something in the source that the mapping did NOT carry into a
/// target type, or a required target field that had no source. Each becomes an
/// interim-admin junk sweep, an extractor mapping rule, or a schema amendment.
#[derive(Debug, Default, Serialize)]
pub struct FieldGapReport {
    /// Source `table.column` or `mime.data-key` seen but not mapped, with a
    /// count and a one-line disposition note.
    pub unmapped_source: BTreeMap<String, GapEntry>,
    /// mimeIds with no target kind (legacy one-offs, junk), with counts.
    pub unmapped_mimes: BTreeMap<String, u64>,
    /// Required target fields that had no source value, with counts (a nonzero
    /// count means the import would violate a NOT NULL or drop meaning).
    pub unfilled_required: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub struct GapEntry {
    pub count: u64,
    pub note: String,
}

impl FieldGapReport {
    fn note_source(&mut self, key: &str, note: &str) {
        let e = self
            .unmapped_source
            .entry(key.to_string())
            .or_insert(GapEntry {
                count: 0,
                note: note.to_string(),
            });
        e.count += 1;
    }
    fn note_mime(&mut self, mime: &str) {
        *self.unmapped_mimes.entry(mime.to_string()).or_insert(0) += 1;
    }
    fn note_unfilled(&mut self, target: &str) {
        *self
            .unfilled_required
            .entry(target.to_string())
            .or_insert(0) += 1;
    }
}

/// The extracted domain rows plus the gap report.
#[derive(Debug, Default, Serialize)]
pub struct Extraction {
    pub users: Vec<User>,
    pub contexts: Vec<Context>,
    pub documents: Vec<Document>,
    pub members: Vec<Member>,
    pub comments: Vec<Comment>,
    pub report: FieldGapReport,
}

/// Map interim rows into the canonical content/membership domain types. `users`
/// realizes the accounts every author/member/comment DID references, so the
/// loader's FK targets exist (during migration `User.did` holds the interim user
/// id until the DID binding runs).
pub fn extract(
    nodes: &[InterimNode],
    members: &[InterimMember],
    users: &[InterimUser],
) -> Extraction {
    let realized_users = users
        .iter()
        .map(|u| User {
            did: u.id.clone(),
            handle: u.handle.clone(),
            display_name: u.display_name.clone(),
            avatar_url: u.avatar_url.clone(),
            legacy_id: Some(u.id.clone()),
        })
        .collect();
    let mut out = Extraction {
        users: realized_users,
        ..Default::default()
    };

    // Author chips: member rows hung on CONTENT nodes become that document's
    // authors, keyed by the content node id. Roster members hang on contexts.
    let content_ids: BTreeSet<&str> = nodes
        .iter()
        .filter(|n| {
            n.mime_id
                .as_deref()
                .is_some_and(|m| CONTENT_MIMES.contains(&m))
        })
        .map(|n| n.id.as_str())
        .collect();
    let mut authors_by_node: BTreeMap<String, Vec<Author>> = BTreeMap::new();

    for m in members {
        let on_content = m
            .parent_id
            .as_deref()
            .is_some_and(|p| content_ids.contains(p));
        if on_content {
            // An author chip (not a roster membership).
            let author = match &m.node_id {
                Some(uid) => Author::User { did: uid.clone() },
                None => Author::FreeText {
                    display: m.name.clone().unwrap_or_default(),
                },
            };
            authors_by_node
                .entry(m.parent_id.clone().unwrap_or_default())
                .or_default()
                .push(author);
            continue;
        }
        // A roster membership row. Normalize the email (census: 11 case/space
        // variant clusters); track the drop of accepted/owner-into-role.
        let email = m
            .email
            .as_ref()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty());
        if m.accepted {
            out.report.note_source(
                "members.accepted",
                "folded into active/bind state, not a target column",
            );
        }
        out.members.push(Member {
            id: m.id.clone(),
            user_did: m.node_id.clone(),
            context_id: m.parent_id.clone().unwrap_or_default(),
            role: if m.owner { Role::Owner } else { Role::Member },
            active: m.active,
            email,
            claim_token: m.claim_token.clone(),
            legacy_id: Some(m.id.clone()),
        });
    }

    for n in nodes {
        let mime = n.mime_id.as_deref().unwrap_or("");
        if CONTEXT_MIMES.contains(&mime) {
            let name = match &n.name {
                Some(name) => name.clone(),
                None => {
                    out.report.note_unfilled("Context.name");
                    String::new()
                }
            };
            out.contexts.push(Context {
                id: n.id.clone(),
                kind: if mime == "wiki/event" {
                    ContextKind::Event
                } else {
                    ContextKind::Group
                },
                name,
                slug: n.key.clone().unwrap_or_default(),
                parent_id: n.parent_id.clone(),
                visibility: Visibility::Private,
                published_uri: None,
                created_at: n.created_at.clone(),
                legacy_id: Some(n.id.clone()),
            });
        } else if CONTENT_MIMES.contains(&mime) {
            let (content, kind) = map_content(n, &mut out.report);
            out.documents.push(Document {
                id: n.id.clone(),
                context_id: n.context_id.clone().unwrap_or_default(),
                parent_id: n.parent_id.clone(),
                kind,
                title: n.name.clone().unwrap_or_default(),
                content,
                authors: authors_by_node.remove(&n.id).unwrap_or_default(),
                visibility: Visibility::Private,
                published_uri: None,
                created_at: n.created_at.clone(),
                legacy_id: Some(n.id.clone()),
            });
        } else if mime == COMMENT_MIME {
            out.comments.push(Comment {
                id: n.id.clone(),
                on_id: n.parent_id.clone().unwrap_or_default(),
                context_id: n.context_id.clone().unwrap_or_default(),
                author: match &n.owner_id {
                    Some(uid) => Author::User { did: uid.clone() },
                    None => Author::FreeText {
                        display: n.name.clone().unwrap_or_default(),
                    },
                },
                text: comment_text(n, &mut out.report),
                created_at: n.created_at.clone(),
                legacy_id: Some(n.id.clone()),
            });
        } else if !mime.is_empty() {
            // Poll/vote/speak nodes are voting/ephemeral (excluded this phase);
            // anything else is an unknown mime for the report.
            match mime {
                "vote/poll" | "vote/vote" | "speak/list" | "speak/speak" => {}
                other => out.report.note_mime(other),
            }
        }
    }

    // Author chips whose content node was excluded (e.g. hung on a poll):
    // record so none are silently dropped.
    for (node_id, authors) in authors_by_node {
        out.report.note_source(
            "members(author-chip on excluded node)",
            &format!(
                "{} author rows on non-content node {node_id}",
                authors.len()
            ),
        );
    }

    out
}

/// Map a content node's `data` JSONB into `content`, reporting unmapped keys.
fn map_content(
    n: &InterimNode,
    report: &mut FieldGapReport,
) -> (Option<serde_json::Value>, DocumentKind) {
    let kind = match n.mime_id.as_deref().unwrap_or("") {
        "wiki/document" => DocumentKind::Document,
        "wiki/folder" => DocumentKind::Folder,
        "wiki/file" => DocumentKind::File,
        "vote/policy" => DocumentKind::Policy,
        "vote/position" => DocumentKind::Position,
        "vote/candidate" => DocumentKind::Candidate,
        "vote/change" => DocumentKind::Change,
        "vote/question" => DocumentKind::Question,
        _ => DocumentKind::Document,
    };
    let Some(serde_json::Value::Object(map)) = &n.data else {
        return (None, kind);
    };
    // `content` is the Slate body; other keys (image, fileId, type, text) are
    // carried in report notes so the importer decides where each lands.
    let content = map.get("content").cloned();
    for key in map.keys() {
        if key != "content" {
            report.note_source(
                &format!("{}.data.{key}", n.mime_id.as_deref().unwrap_or("?")),
                "non-content data key: importer must map (image/fileId/type/etc.)",
            );
        }
    }
    (content, kind)
}

/// A comment node's text lives in `data.text` (census: vote/comment shape).
fn comment_text(n: &InterimNode, report: &mut FieldGapReport) -> String {
    match &n.data {
        Some(serde_json::Value::Object(m)) => m
            .get("text")
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .unwrap_or_default(),
        _ => {
            report.note_unfilled("Comment.text");
            String::new()
        }
    }
}
