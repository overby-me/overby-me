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
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<String>,
    /// Slash-joined keys from the root, kept by a trigger. Absent in an older
    /// dump, in which case it is rebuilt from the keys.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub mutable: Option<bool>,
    #[serde(default)]
    pub attachable: Option<bool>,
    /// Set while the node is in the bin.
    #[serde(default)]
    pub deleted_at: Option<String>,
    /// The node whose deletion took this one along.
    #[serde(default)]
    pub deleted_root: Option<String>,
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
    #[serde(default)]
    pub hidden: bool,
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

const CONTEXT_MIMES: &[&str] = &["wiki/group", "wiki/event", "wiki/site"];
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

/// The interim snapshot the extractor consumes: `{ nodes, members, users }` as
/// produced by the read-only `scripts/dump-interim-snapshot.nu` dump (a separate,
/// owner-approved step; this crate never touches the live DB). Each list defaults
/// to empty so a partial dump still parses.
#[derive(Debug, Default, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub nodes: Vec<InterimNode>,
    #[serde(default)]
    pub members: Vec<InterimMember>,
    #[serde(default)]
    pub users: Vec<InterimUser>,
}

/// The extracted domain rows plus the gap report.
#[derive(Debug, Default, Serialize)]
pub struct Extraction {
    pub users: Vec<User>,
    pub contexts: Vec<Context>,
    pub documents: Vec<Document>,
    pub members: Vec<Member>,
    pub comments: Vec<Comment>,
    /// The migratable voting entity: poll metadata. Cast ballots (`vote/vote`) are
    /// unmigratable and only reported.
    pub polls: Vec<Poll>,
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

    // A member row is one of two things, told apart by what it hangs on. On a
    // CONTENT node it is an author chip, and becomes one of that document's
    // authors. On a CONTEXT it is a roster membership. On anything else it has
    // no home, and is reported rather than loaded against a context that is not
    // one, which the foreign key would refuse.
    let mimes_of = |wanted: &[&str]| -> BTreeSet<&str> {
        nodes
            .iter()
            .filter(|n| n.mime_id.as_deref().is_some_and(|m| wanted.contains(&m)))
            .map(|n| n.id.as_str())
            .collect()
    };
    let content_ids = mimes_of(CONTENT_MIMES);
    let context_ids = mimes_of(CONTEXT_MIMES);
    let mut authors_by_node: BTreeMap<String, Vec<Author>> = BTreeMap::new();

    for m in members {
        let parent = m.parent_id.as_deref().unwrap_or_default();
        if content_ids.contains(parent) {
            let author = match &m.node_id {
                Some(uid) => Author::User { did: uid.clone() },
                None => Author::FreeText {
                    display: m.name.clone().unwrap_or_default(),
                },
            };
            authors_by_node
                .entry(parent.to_string())
                .or_default()
                .push(author);
            continue;
        }
        if !context_ids.contains(parent) {
            let on = nodes
                .iter()
                .find(|n| n.id == parent)
                .and_then(|n| n.mime_id.as_deref())
                .unwrap_or("a node that is not in the dump");
            out.report.note_source(
                &format!("members(on {on})"),
                "a member row on neither a context nor content: no home",
            );
            continue;
        }
        // Normalize the email (census: 11 case/space variant clusters).
        let email = m
            .email
            .as_ref()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty());
        out.members.push(Member {
            id: m.id.clone(),
            user_did: m.node_id.clone(),
            context_id: parent.to_string(),
            role: if m.owner { Role::Owner } else { Role::Member },
            active: m.active,
            name: m.name.clone().filter(|n| !n.trim().is_empty()),
            hidden: m.hidden,
            accepted: m.accepted,
            email,
            claim_token: m.claim_token.clone(),
            legacy_id: Some(m.id.clone()),
        });
    }
    realize_context_owners(nodes, &context_ids, &mut out);

    let tree: BTreeMap<&str, &InterimNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    // The ids that become a `context` or a `document`: the rows a `parent_id`
    // can still point at after the move.
    let migrated: BTreeSet<&str> = nodes
        .iter()
        .filter(|n| {
            let mime = n.mime_id.as_deref().unwrap_or("");
            CONTEXT_MIMES.contains(&mime) || CONTENT_MIMES.contains(&mime)
        })
        .map(|n| n.id.as_str())
        .collect();

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
                kind: match mime {
                    "wiki/event" => ContextKind::Event,
                    "wiki/site" => ContextKind::Site,
                    _ => ContextKind::Group,
                },
                name,
                place: place_of(n, &tree, &migrated, &mut out.report),
                visibility: Visibility::Private,
                published_uri: None,
                legacy_id: Some(n.id.clone()),
            });
        } else if CONTENT_MIMES.contains(&mime) {
            let (content, data, kind) = map_content(n);
            out.documents.push(Document {
                id: n.id.clone(),
                context_id: n.context_id.clone().unwrap_or_default(),
                kind,
                title: n.name.clone().unwrap_or_default(),
                place: place_of(n, &tree, &migrated, &mut out.report),
                mutable: n.mutable.unwrap_or(true),
                content,
                data,
                authors: authors_by_node.remove(&n.id).unwrap_or_default(),
                visibility: Visibility::Private,
                published_uri: None,
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
        } else if mime == "vote/poll" {
            // The one migratable voting entity: the poll's question/options/state.
            let obj = n.data.as_ref().and_then(|d| d.as_object());
            let str_key = |k: &str| {
                obj.and_then(|m| m.get(k))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let bool_key = |k: &str| obj.and_then(|m| m.get(k)).and_then(|v| v.as_bool());
            let options = obj
                .and_then(|m| m.get("options"))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.polls.push(Poll {
                id: n.id.clone(),
                context_id: n.context_id.clone().unwrap_or_default(),
                question: str_key("question").unwrap_or_default(),
                options,
                open: bool_key("open").unwrap_or(false),
                secret: bool_key("secret").unwrap_or(false),
                created_at: n.created_at.clone(),
                legacy_id: Some(n.id.clone()),
            });
        } else if !mime.is_empty() {
            match mime {
                // A cast ballot is unmigratable: the eligibility/tokens that gave
                // it weight and anonymity do not survive, so it is reported, never
                // carried.
                "vote/vote" => out.report.note_source(
                    "nodes(vote/vote)",
                    "historical cast ballot: unmigratable (no eligibility/tokens survive)",
                ),
                // Speaker lists/entries are ephemeral projector state, dropped.
                "speak/list" | "speak/speak" => {}
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

/// Whoever made a context owns it, whether or not a member row says so: the
/// interim's read rules and its `is_active_owner` both count `nodes.owner_id`.
/// The new model knows owners only as members, so without this the general
/// secretary who owns Landsmøde 2026 and holds no row in it
/// (`docs/read-permissions.md`) would lose the meeting at cutover.
///
/// A missing row is added hidden, because they were never on the member list;
/// an existing one is raised to owner. Both are counted in the report.
fn realize_context_owners(
    nodes: &[InterimNode],
    context_ids: &BTreeSet<&str>,
    out: &mut Extraction,
) {
    for n in nodes.iter().filter(|n| context_ids.contains(n.id.as_str())) {
        let Some(owner) = n.owner_id.as_deref() else {
            continue;
        };
        let held = out
            .members
            .iter_mut()
            .find(|m| m.context_id == n.id && m.user_did.as_deref() == Some(owner));
        match held {
            Some(member) if member.role == Role::Owner => {}
            Some(member) => {
                member.role = Role::Owner;
                out.report.note_source(
                    "nodes.ownerId (context)",
                    "the owner of a context, realized as an owner membership",
                );
            }
            None => {
                out.members.push(Member {
                    id: format!("owner-of-{}", n.id),
                    user_did: Some(owner.to_string()),
                    context_id: n.id.clone(),
                    role: Role::Owner,
                    active: true,
                    name: None,
                    hidden: true,
                    accepted: true,
                    email: None,
                    claim_token: None,
                    legacy_id: None,
                });
                out.report.note_source(
                    "nodes.ownerId (context)",
                    "the owner of a context, realized as an owner membership",
                );
            }
        }
    }
}

/// Where a node sits: its key, its path and its parent.
///
/// A parent that is not itself migrated cannot be kept, or the row would hang
/// off something the new tree does not have. The interim root (`wiki/home`) is
/// the expected case, and makes its children roots; any other is reported,
/// because that subtree is about to come loose.
fn place_of(
    n: &InterimNode,
    tree: &BTreeMap<&str, &InterimNode>,
    migrated: &BTreeSet<&str>,
    report: &mut FieldGapReport,
) -> Place {
    let parent_id = n
        .parent_id
        .as_deref()
        .filter(|p| migrated.contains(p))
        .map(str::to_string);
    if parent_id.is_none()
        && let Some(parent) = n.parent_id.as_deref()
        && let Some(parent_mime) = tree.get(parent).map(|p| p.mime_id.as_deref().unwrap_or(""))
        && parent_mime != "wiki/home"
    {
        report.note_source(
            &format!("nodes.parentId -> {parent_mime}"),
            "parent kind is not migrated: the node is re-rooted",
        );
    }
    Place {
        slug: n.key.clone().unwrap_or_default(),
        path: n.path.clone().unwrap_or_else(|| path_from_keys(n, tree)),
        parent_id,
        idx: n.index.unwrap_or(0),
        attachable: n.attachable.unwrap_or(true),
        owner_did: n.owner_id.clone(),
        created_at: n.created_at.clone(),
        updated_at: n.updated_at.clone(),
        deleted_at: n.deleted_at.clone(),
        deleted_root: n.deleted_root.clone(),
    }
}

/// The path the interim trigger would have stored: the keys from the root down,
/// the root's own key excluded. Bounded, so a parent cycle in bad data ends.
fn path_from_keys(n: &InterimNode, tree: &BTreeMap<&str, &InterimNode>) -> String {
    let mut keys = Vec::new();
    let mut at = Some(n);
    for _ in 0..64 {
        let Some(node) = at else { break };
        if node.parent_id.is_none() {
            break;
        }
        keys.push(node.key.as_deref().unwrap_or_default());
        at = node.parent_id.as_deref().and_then(|p| tree.get(p).copied());
    }
    keys.reverse();
    keys.join("/")
}

/// Split a content node's `data` JSONB into the Slate `content` and everything
/// else (a file's id and type, a cover image), which is carried as it is.
fn map_content(
    n: &InterimNode,
) -> (
    Option<serde_json::Value>,
    Option<serde_json::Value>,
    DocumentKind,
) {
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
        return (None, None, kind);
    };
    let mut rest = map.clone();
    let content = rest.remove("content");
    let data = (!rest.is_empty()).then_some(serde_json::Value::Object(rest));
    (content, data, kind)
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
