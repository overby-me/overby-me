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
/// actually exist. Its `did` is the interim user id, which no login produces:
/// the account holds its seats and cannot sign in, until the person whose
/// address it was registered under takes it over ([`LegacyAccount`]).
#[derive(Debug, Clone, Deserialize)]
pub struct InterimUser {
    pub id: String,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(rename = "avatarUrl", default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(rename = "emailVerified", default)]
    pub email_verified: Option<bool>,
}

/// The address an interim account was registered under, which is how its
/// person is recognized when they sign in with a DID. Carried only where the
/// interim had VERIFIED it: an unverified address is one somebody typed, and
/// whoever typed it would otherwise inherit the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyAccount {
    pub id: String,
    /// Trimmed and lowercased, as member addresses are.
    pub email: String,
}

/// An interim `permissions` row, as far as it says who may READ a context. A
/// context is open to everyone when it has an ACTIVE row for the `public` role
/// that grants `select`: that row is the setting (`src/graphql/public.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct InterimPermission {
    #[serde(rename = "contextId", default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub select: Option<bool>,
    #[serde(default)]
    pub active: Option<bool>,
}

const CONTEXT_MIMES: &[&str] = &["wiki/group", "wiki/event", "wiki/site"];
/// The root every path starts under. A context like any other in the interim:
/// its members are who runs the site, and its content is the welcome page.
const HOME_MIME: &str = "wiki/home";
/// Nodes that become a document for their place in the tree, and a row of their
/// own for what they are.
const POLL_MIME: &str = "vote/poll";
const CANVAS_MIME: &str = "canvas/canvas";
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
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FieldGapReport {
    /// Source `table.column` or `mime.data-key` seen but not mapped, with a
    /// count and a one-line disposition note.
    pub unmapped_source: BTreeMap<String, GapEntry>,
    /// mimeIds with no target kind (legacy one-offs, junk), with counts.
    pub unmapped_mimes: BTreeMap<String, u64>,
    /// Required target fields that had no source value, with counts (a nonzero
    /// count means the import would violate a NOT NULL or drop meaning).
    pub unfilled_required: BTreeMap<String, u64>,
    /// Rows left behind ON PURPOSE, with counts. Not gaps: each is a decision,
    /// listed so that none of them is a silent one.
    #[serde(default)]
    pub left_behind: BTreeMap<String, u64>,
    /// Source values that arrive in another shape than they had, with what
    /// became of them. Decisions like `left_behind`, and as little a gap.
    #[serde(default)]
    pub reshaped: BTreeMap<String, GapEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GapEntry {
    pub count: u64,
    pub note: String,
}

impl FieldGapReport {
    fn note_source(&mut self, key: &str, note: &str) {
        Self::count_into(&mut self.unmapped_source, key, note);
    }
    fn note_reshaped(&mut self, key: &str, note: &str) {
        Self::count_into(&mut self.reshaped, key, note);
    }
    fn count_into(bucket: &mut BTreeMap<String, GapEntry>, key: &str, note: &str) {
        let e = bucket.entry(key.to_string()).or_insert(GapEntry {
            count: 0,
            note: note.to_string(),
        });
        e.count += 1;
    }
    fn note_mime(&mut self, mime: &str) {
        *self.unmapped_mimes.entry(mime.to_string()).or_insert(0) += 1;
    }
    fn note_left_behind(&mut self, what: &str) {
        *self.left_behind.entry(what.to_string()).or_insert(0) += 1;
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
    /// Absent from a dump made before visibility was carried, in which case
    /// every context comes out closed, and the report says so.
    #[serde(default)]
    pub permissions: Option<Vec<InterimPermission>>,
}

/// The extracted domain rows plus the gap report.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Extraction {
    pub users: Vec<User>,
    pub contexts: Vec<Context>,
    pub documents: Vec<Document>,
    pub members: Vec<Member>,
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub accounts: Vec<LegacyAccount>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    /// Each poll's rules and RESULT. Its ballots are counted here and not
    /// carried; its place in the tree is among `documents`.
    #[serde(default)]
    pub polls: Vec<Poll>,
    #[serde(default)]
    pub canvases: Vec<Canvas>,
    #[serde(default)]
    pub feedback: Vec<Feedback>,
    #[serde(default)]
    pub report: FieldGapReport,
}

/// [`extract`] over a whole snapshot, with each context opened to the public or
/// not as its permission rows say.
pub fn extract_snapshot(snap: &Snapshot) -> Extraction {
    let mut out = extract(&snap.nodes, &snap.members, &snap.users);
    match &snap.permissions {
        Some(permissions) => {
            let open: BTreeSet<&str> = permissions
                .iter()
                .filter(|p| {
                    p.role.as_deref() == Some("public")
                        && p.select == Some(true)
                        && p.active == Some(true)
                })
                .filter_map(|p| p.context_id.as_deref())
                .collect();
            for context in &mut out.contexts {
                if open.contains(context.id.as_str()) {
                    context.visibility = Visibility::Public;
                }
            }
        }
        None => out.report.note_source(
            "permissions",
            "not in this dump: every context is extracted closed, the public ones too",
        ),
    }
    out
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
    for u in users {
        let email = u.email.as_deref().map(normalized).filter(|e| !e.is_empty());
        match (email, u.email_verified) {
            (Some(email), Some(true)) => out.accounts.push(LegacyAccount {
                id: u.id.clone(),
                email,
            }),
            // Their seats are handed over one by one, by claim link.
            _ => out.report.note_left_behind("users.email, not verified"),
        }
    }

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
    // The one home: a root by mime AND by having nothing above it. Any other
    // node of that mime is reported with the unknown kinds.
    let home = nodes
        .iter()
        .find(|n| n.mime_id.as_deref() == Some(HOME_MIME) && n.parent_id.is_none())
        .map(|n| n.id.as_str());
    let mut context_ids = mimes_of(CONTEXT_MIMES);
    context_ids.extend(home);
    let tree: BTreeMap<&str, &InterimNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let orphaned = orphans(nodes, &tree, home);
    context_ids.retain(|id| !orphaned.contains(id));
    let mut authors_by_node: BTreeMap<String, Vec<Author>> = BTreeMap::new();

    // `members.nodeId` is tied to nothing in the interim, so an account can be
    // deleted from under its rows. Only a dump that carries accounts can tell.
    let accounts: BTreeSet<&str> = users.iter().map(|u| u.id.as_str()).collect();
    let gone = |id: &str| !accounts.is_empty() && !accounts.contains(id) && !tree.contains_key(id);
    let mut seats_of_the_gone = Vec::new();

    for m in members {
        let parent = m.parent_id.as_deref().unwrap_or_default();
        if orphaned.contains(parent) {
            out.report.note_left_behind("members, on an orphaned node");
            continue;
        }
        let named = m.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        if content_ids.contains(parent) {
            let author = match &m.node_id {
                Some(id) if gone(id) => match named {
                    Some(name) => {
                        out.report.note_reshaped(
                            "members(author).nodeId, account gone",
                            "an author whose account was deleted is carried by name",
                        );
                        Author::FreeText {
                            display: name.to_string(),
                        }
                    }
                    None => {
                        out.report
                            .note_left_behind("members(author), account gone and unnamed");
                        continue;
                    }
                },
                // A chip points at a node, and a group is a node too: a branch
                // that put a motion forward is not a person with that id.
                Some(id) if context_ids.contains(id.as_str()) => Author::Context {
                    context_id: id.clone(),
                    name: None,
                    path: None,
                },
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
        if m.node_id.as_deref().is_some_and(gone) {
            seats_of_the_gone.push((m, parent));
            continue;
        }
        // Normalize the email (census: 11 case/space variant clusters).
        let email = m.email.as_deref().map(normalized).filter(|e| !e.is_empty());
        // A claim link is spent once its seat is taken. Carried along, it would
        // open that seat again to whoever still has the old invitation.
        let claim_token = match &m.node_id {
            Some(_) => {
                if m.claim_token.is_some() {
                    out.report.note_left_behind("members.claim_token, spent");
                }
                None
            }
            None => m.claim_token.clone(),
        };
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
            claim_token,
            legacy_id: Some(m.id.clone()),
        });
    }
    keep_the_seats_of_the_gone(&seats_of_the_gone, &mut out);
    realize_context_owners(nodes, &context_ids, &mut out);

    let parents: BTreeSet<&str> = nodes
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    // The ids that become a `context` or a `document`: the rows a `parent_id`
    // can still point at after the move.
    let migrated: BTreeSet<&str> = nodes
        .iter()
        .filter(|n| !orphaned.contains(n.id.as_str()))
        .filter(|n| {
            let mime = n.mime_id.as_deref().unwrap_or("");
            CONTEXT_MIMES.contains(&mime)
                || CONTENT_MIMES.contains(&mime)
                || [POLL_MIME, CANVAS_MIME].contains(&mime)
        })
        .map(|n| n.id.as_str())
        .chain(home)
        .collect();

    for n in nodes {
        let mime = n.mime_id.as_deref().unwrap_or("");
        if orphaned.contains(n.id.as_str()) {
            out.report.note_left_behind(&format!("{mime}, orphaned"));
            continue;
        }
        let is_home = home == Some(n.id.as_str());
        if CONTEXT_MIMES.contains(&mime) || is_home {
            let name = match &n.name {
                Some(name) => name.clone(),
                None => {
                    out.report.note_unfilled("Context.name");
                    String::new()
                }
            };
            let (content, data) = content_and_rest(n);
            let mut place = place_of(n, &tree, &migrated, &mut out.report);
            if is_home {
                // Whatever key the interim gave its root, it is in no path.
                (place.slug, place.path) = (String::new(), String::new());
            }
            out.contexts.push(Context {
                id: n.id.clone(),
                kind: match mime {
                    HOME_MIME => ContextKind::Home,
                    "wiki/event" => ContextKind::Event,
                    "wiki/site" => ContextKind::Site,
                    _ => ContextKind::Group,
                },
                name,
                place,
                content,
                data,
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
            let comment = comment_of(n, &tree, &migrated, &mut out.report);
            out.comments.push(comment);
        } else if ["wiki/feedback", "vote/reaction"].contains(&mime)
            && deleted_alone(n, &tree, &migrated)
        {
            out.report.note_left_behind(&format!("{mime}, deleted"));
        } else if mime == POLL_MIME || mime == CANVAS_MIME {
            // Its place in the tree. What it IS follows below.
            out.documents.push(Document {
                id: n.id.clone(),
                context_id: n.context_id.clone().unwrap_or_default(),
                kind: if mime == POLL_MIME {
                    DocumentKind::Poll
                } else {
                    DocumentKind::Canvas
                },
                title: n.name.clone().unwrap_or_default(),
                place: place_of(n, &tree, &migrated, &mut out.report),
                mutable: false,
                content: None,
                data: None,
                authors: Vec::new(),
                visibility: Visibility::Private,
                published_uri: None,
                legacy_id: Some(n.id.clone()),
            });
            if mime == POLL_MIME {
                let poll = poll_of(n, nodes, &mut out.report);
                out.polls.push(poll);
            } else {
                let canvas = canvas_of(n, nodes);
                out.canvases.push(canvas);
            }
        } else if mime == "wiki/feedback" {
            out.feedback.push(feedback_of(n));
        } else if mime == "vote/reaction" {
            match reaction_of(n) {
                Some(reaction) => out.reactions.push(reaction),
                None => out.report.note_source(
                    "nodes(vote/reaction)",
                    "a reaction with no emoji, or on nothing",
                ),
            }
        } else if !mime.is_empty() {
            match mime {
                // Counted into their poll's result by `poll_of`, and canvas cells
                // into their canvas by `canvas_of`.
                "vote/vote" | "canvas/pixel" => {}
                // What a projector showed while a meeting ran, and nothing after.
                "speak/list" | "speak/speak" => out.report.note_left_behind(mime),
                // A legacy one-off with nothing in it and nothing under it loses
                // a name. One that HOLDS anything stays a gap to triage, and so
                // does a second home, which is no legacy mime but a broken tree.
                other if other != HOME_MIME && is_empty_shell(n, &parents) => {
                    out.report.note_left_behind(&format!("{other}, empty"));
                }
                other => out.report.note_mime(other),
            }
        }
    }
    // One reaction per person per emoji per subject, which the table insists on
    // and the interim did not.
    let mut seen = BTreeSet::new();
    out.reactions.retain(|r| {
        seen.insert((
            r.subject_uri.clone(),
            r.reactor_did.clone(),
            r.emoji.clone(),
        ))
    });

    out.feedback = folded_by_digest(std::mem::take(&mut out.feedback));

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

/// Whether `n` went into the bin along with `root`, something that is carried
/// and has a bin where it is going: a node of the tree, or a comment.
fn binned_with(
    n: &InterimNode,
    tree: &BTreeMap<&str, &InterimNode>,
    migrated: &BTreeSet<&str>,
) -> bool {
    n.deleted_root.as_deref().is_some_and(|root| {
        let a_comment = tree
            .get(root)
            .is_some_and(|r| r.mime_id.as_deref() == Some(COMMENT_MIME));
        root != n.id && (migrated.contains(root) || a_comment)
    })
}

/// The ids that hang off nothing: an ancestor's row is gone (deleted outright,
/// before the interim had a bin), so no URL reaches them and no page lists them.
/// Carried, they would come back at the top of their group. Empty for a dump
/// with no home, whose top level hangs off nothing by design.
fn orphans<'a>(
    nodes: &'a [InterimNode],
    tree: &BTreeMap<&str, &InterimNode>,
    home: Option<&str>,
) -> BTreeSet<&'a str> {
    if home.is_none() {
        return BTreeSet::new();
    }
    nodes
        .iter()
        .filter(|n| !reaches_a_root(n, tree))
        .map(|n| n.id.as_str())
        .collect()
}

/// Bounded, so a parent cycle in bad data ends, as an orphan.
fn reaches_a_root(n: &InterimNode, tree: &BTreeMap<&str, &InterimNode>) -> bool {
    let mut at = n;
    for _ in 0..64 {
        let Some(parent) = at.parent_id.as_deref() else {
            return true;
        };
        let Some(above) = tree.get(parent) else {
            return false;
        };
        at = above;
    }
    false
}

/// No data of its own, and no row under it (binned ones included: a restore
/// would look for this parent).
fn is_empty_shell(n: &InterimNode, parents: &BTreeSet<&str>) -> bool {
    let bare = match &n.data {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::Object(o)) => o.is_empty(),
        Some(serde_json::Value::Array(a)) => a.is_empty(),
        Some(serde_json::Value::String(t)) => t.trim().is_empty(),
        Some(_) => false,
    };
    bare && !parents.contains(n.id.as_str())
}

/// In the bin on its own account. Reactions and reports have no bin where they
/// are going, so carrying one of these would bring back what somebody deleted.
/// One binned WITH its comment or document stays: it is hidden with that, and
/// has to be there when that is restored.
fn deleted_alone(
    n: &InterimNode,
    tree: &BTreeMap<&str, &InterimNode>,
    migrated: &BTreeSet<&str>,
) -> bool {
    n.deleted_at.is_some() && !binned_with(n, tree, migrated)
}

/// A comment, with the document its thread hangs on and its place in the bin.
///
/// One binned along with its DOCUMENT comes across live: where it is going a
/// document's comments are not stamped, the document hides them. One deleted on
/// its own, or with the thread it was in, comes across in the bin, under the
/// comment whose deletion took it there.
fn comment_of(
    n: &InterimNode,
    tree: &BTreeMap<&str, &InterimNode>,
    migrated: &BTreeSet<&str>,
    report: &mut FieldGapReport,
) -> Comment {
    let mut root = n.parent_id.clone().unwrap_or_default();
    // Bounded, so a cycle in the dump cannot hang the extraction.
    for _ in 0..tree.len() {
        match tree.get(root.as_str()) {
            Some(up) if up.mime_id.as_deref() == Some(COMMENT_MIME) => {
                root = up.parent_id.clone().unwrap_or_default();
            }
            _ => break,
        }
    }
    let with_its_document = n
        .deleted_root
        .as_deref()
        .is_some_and(|root| migrated.contains(root));
    let binned = n.deleted_at.is_some() && !with_its_document;
    // Emptied in place because replies hang on it. The interim could not null
    // the account on the row, so the scrub is finished here.
    let tombstone = data_of(n, "deleted").and_then(|v| v.as_bool()) == Some(true);
    Comment {
        id: n.id.clone(),
        on_id: n.parent_id.clone().unwrap_or_default(),
        root_id: root,
        context_id: n.context_id.clone().unwrap_or_default(),
        author: match &n.owner_id {
            Some(uid) if !tombstone => Author::User { did: uid.clone() },
            _ => Author::FreeText {
                display: n.name.clone().filter(|_| !tombstone).unwrap_or_default(),
            },
        },
        text: if tombstone {
            String::new()
        } else {
            comment_text(n, report)
        },
        image: data_of(n, "image")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty() && !tombstone)
            .map(str::to_string),
        tombstone,
        created_at: n.created_at.clone(),
        deleted_at: n.deleted_at.clone().filter(|_| binned),
        deleted_root: n
            .deleted_root
            .clone()
            .or_else(|| Some(n.id.clone()))
            .filter(|_| binned),
        legacy_id: Some(n.id.clone()),
    }
}

/// One row per crash, which the table insists on. The interim looks a crash up
/// and then files it, in two steps, so two people crashing at once file two.
fn folded_by_digest(reports: Vec<Feedback>) -> Vec<Feedback> {
    let mut at: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: Vec<Feedback> = Vec::new();
    for report in reports {
        let kept = report.digest.as_ref().and_then(|d| at.get(d)).copied();
        match kept {
            Some(i) => {
                let kept = &mut out[i];
                kept.seen += report.seen;
                kept.updated_at = kept.updated_at.take().max(report.updated_at);
                for reporter in report.reporters {
                    if !kept.reporters.contains(&reporter) {
                        kept.reporters.push(reporter);
                    }
                }
            }
            None => {
                if let Some(digest) = &report.digest {
                    at.insert(digest.clone(), out.len());
                }
                out.push(report);
            }
        }
    }
    out
}

fn normalized(email: &str) -> String {
    email.trim().to_lowercase()
}

fn data_of<'a>(n: &'a InterimNode, key: &str) -> Option<&'a serde_json::Value> {
    n.data.as_ref().and_then(|d| d.get(key))
}

fn live_children<'a>(
    nodes: &'a [InterimNode],
    parent: &'a str,
    mime: &'a str,
) -> impl Iterator<Item = &'a InterimNode> {
    nodes.iter().filter(move |c| {
        c.parent_id.as_deref() == Some(parent)
            && c.mime_id.as_deref() == Some(mime)
            && c.deleted_at.is_none()
    })
}

/// A poll's rules, and its result counted from the ballots under it.
///
/// The interim has no `question`: a poll is named after what it is on, and that
/// name is what was voted on. It has no `open` either: a poll is open while its
/// node is `mutable`. One still open at the dump comes across closed, since what
/// it had taken is all it will ever take here, and the report says so.
fn poll_of(n: &InterimNode, nodes: &[InterimNode], report: &mut FieldGapReport) -> Poll {
    let options: Vec<String> = data_of(n, "options")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|o| o.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let number = |key: &str| {
        data_of(n, key)
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1)
    };
    let flag = |key: &str| data_of(n, key).and_then(|v| v.as_bool()).unwrap_or(false);
    if n.mutable == Some(true) {
        report.note_reshaped(
            "nodes(vote/poll).mutable",
            "a poll open at the dump is migrated closed, with what it had taken",
        );
    }
    let mut counts = vec![0u64; options.len()];
    let mut ballots = 0;
    for vote in live_children(nodes, &n.id, "vote/vote") {
        ballots += 1;
        let chosen = vote.data.as_ref().and_then(|d| d.as_array());
        for index in chosen.into_iter().flatten().filter_map(|i| i.as_u64()) {
            match counts.get_mut(index as usize) {
                Some(count) => *count += 1,
                None => report.note_source(
                    "nodes(vote/vote).data",
                    "a ballot for an option its poll does not have",
                ),
            }
        }
    }
    Poll {
        id: n.id.clone(),
        context_id: n.context_id.clone().unwrap_or_default(),
        question: n.name.clone().unwrap_or_default(),
        // The interim always appends the abstention, and knows it by position.
        blank: options.len() > 1,
        options,
        min: number("minVote"),
        max: number("maxVote"),
        secret: flag("secret"),
        hide_tally: flag("hidden"),
        counts,
        ballots,
        created_at: n.created_at.clone(),
        closed_at: n.updated_at.clone(),
        legacy_id: Some(n.id.clone()),
    }
}

/// A canvas and the cells painted on it: each a hidden child keyed `p_<x>_<y>`
/// whose `data.c` is the colour and whose owner is the last painter.
fn canvas_of(n: &InterimNode, nodes: &[InterimNode]) -> Canvas {
    let side = |key: &str, otherwise: u32| {
        data_of(n, key)
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(otherwise)
    };
    let cells = live_children(nodes, &n.id, "canvas/pixel")
        .filter_map(|cell| {
            let (x, y) = cell
                .key
                .as_deref()?
                .strip_prefix("p_")?
                .split_once('_')
                .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))?;
            Some(CanvasCell {
                x,
                y,
                colour: u8::try_from(data_of(cell, "c")?.as_u64()?).ok()?,
                painter_did: cell.owner_id.clone(),
                painted_at: cell.updated_at.clone().or_else(|| cell.created_at.clone()),
            })
        })
        .collect();
    Canvas {
        id: n.id.clone(),
        width: side("w", 32).clamp(1, 128),
        height: side("h", 32).clamp(1, 128),
        cooldown: side("cooldown", 60),
        open: n.mutable.unwrap_or(true),
        cells,
    }
}

fn feedback_of(n: &InterimNode) -> Feedback {
    let text = |key: &str| {
        data_of(n, key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let kind = text("kind");
    Feedback {
        id: n.id.clone(),
        kind: match kind.as_str() {
            "bug" | "feature" | "crash" | "error" => kind,
            _ => "other".to_string(),
        },
        message: Some(text("message"))
            .filter(|m| !m.is_empty())
            .or_else(|| n.name.clone())
            .unwrap_or_default(),
        path: text("path"),
        app_version: text("appVersion"),
        commit: text("commit"),
        user_agent: text("userAgent"),
        image: Some(text("image")).filter(|i| !i.is_empty()),
        digest: Some(text("crashDigest")).filter(|d| !d.is_empty()),
        seen: data_of(n, "seen").and_then(|v| v.as_u64()).unwrap_or(1),
        reporters: data_of(n, "reporters")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        owner_did: n.owner_id.clone(),
        created_at: n.created_at.clone(),
        updated_at: n.updated_at.clone(),
    }
}

/// A reaction is a node under what it reacts to, holding its emoji in
/// `data.emoji` and as its name.
fn reaction_of(n: &InterimNode) -> Option<Reaction> {
    let emoji = data_of(n, "emoji")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| n.name.clone())
        .filter(|e| !e.is_empty())?;
    Some(Reaction {
        id: n.id.clone(),
        subject_uri: n.parent_id.clone()?,
        reactor_did: n.owner_id.clone(),
        emoji,
        created_at: n.created_at.clone(),
        legacy_id: Some(n.id.clone()),
    })
}

/// A roster says who belongs, account or none. A seat whose account was deleted
/// goes back to waiting for its address, as it was before anyone took it, and
/// after the rest: one address holds one waiting seat in a context, and the
/// row that was always waiting is the one to keep.
fn keep_the_seats_of_the_gone(seats: &[(&InterimMember, &str)], out: &mut Extraction) {
    let mut waiting: BTreeSet<(String, String)> = out
        .members
        .iter()
        .filter(|m| m.user_did.is_none())
        .filter_map(|m| Some((m.context_id.clone(), m.email.clone()?)))
        .collect();
    for (m, context) in seats {
        let email = m.email.as_deref().map(normalized).filter(|e| !e.is_empty());
        let Some(email) = email else {
            out.report
                .note_left_behind("members, account gone and no address");
            continue;
        };
        if !waiting.insert((context.to_string(), email.clone())) {
            out.report
                .note_left_behind("members, account gone and its address already waits");
            continue;
        }
        out.report.note_reshaped(
            "members.nodeId, account gone",
            "a seat whose account was deleted waits for its address again",
        );
        out.members.push(Member {
            id: m.id.clone(),
            user_did: None,
            context_id: context.to_string(),
            role: if m.owner { Role::Owner } else { Role::Member },
            active: m.active,
            name: m.name.clone().filter(|n| !n.trim().is_empty()),
            hidden: m.hidden,
            accepted: false,
            email: Some(email),
            claim_token: None,
            legacy_id: Some(m.id.clone()),
        });
    }
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
                out.report.note_reshaped(
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
                out.report.note_reshaped(
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
/// off something the new tree does not have. It is reported, because that
/// subtree is about to come loose. A dump with no home is the one exception:
/// there the top level hangs off nothing, as it did.
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
    let (content, data) = content_and_rest(n);
    (content, data, kind)
}

/// A node's Slate `content`, and whatever else its `data` holds, carried as it
/// is: a file's id and type, a cover image, a redirect.
fn content_and_rest(n: &InterimNode) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    let Some(serde_json::Value::Object(map)) = &n.data else {
        return (None, None);
    };
    let mut rest = map.clone();
    let content = rest.remove("content");
    (
        content,
        (!rest.is_empty()).then_some(serde_json::Value::Object(rest)),
    )
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
