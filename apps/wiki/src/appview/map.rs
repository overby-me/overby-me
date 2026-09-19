//! From what the AppView says to what the components read.
//!
//! The components were written against the interim's one `nodes` table, where a
//! group, a page, a comment and a ballot are all rows told apart by a mime, and
//! where a page's text sits inside its `data`. The AppView has a table per
//! thing. This is where the one is dressed as the other, so that no component
//! has to know which backend answered.

use crate::model::{
    ChildNodeFields, ContextNodeFields, Crumb, DrawerChildFields, Jsonb, MemberFields,
    MemberNodeRef, MimeFields, NodeFields, NodeWithChildren, ParentNodeFields, Timestamptz,
    UserRef, Uuid,
};
use appview_client::{defs, get_node};

/// The interim's mime for an AppView kind, and back. A kind this table does not
/// know is passed through as `wiki/<kind>`, which no component matches: it draws
/// as an unknown node rather than as the wrong one.
const MIMES: &[(&str, &str)] = &[
    ("home", "wiki/home"),
    ("group", "wiki/group"),
    ("event", "wiki/event"),
    ("site", "wiki/site"),
    ("document", "wiki/document"),
    ("folder", "wiki/folder"),
    ("file", "wiki/file"),
    ("policy", "vote/policy"),
    ("change", "vote/change"),
    ("position", "vote/position"),
    ("candidate", "vote/candidate"),
    ("question", "vote/question"),
    ("poll", "vote/poll"),
    ("canvas", "canvas/canvas"),
    ("comment", "vote/comment"),
    ("reaction", "vote/reaction"),
];

/// Kinds a listing leaves out in the interim (`mimes.hidden`): each has an app
/// of its own to be found through.
const HIDDEN: &[&str] = &["poll", "canvas", "comment", "reaction"];

const CONTEXTS: &[&str] = &["home", "group", "event", "site"];

/// Kinds that are documents a member makes with `createDocument`.
const CONTENT: &[&str] = &[
    "document",
    "folder",
    "file",
    "policy",
    "change",
    "position",
    "candidate",
    "question",
];

pub fn is_content(kind: &str) -> bool {
    CONTENT.contains(&kind)
}

/// Whether a listing leaves this kind out, as the interim's `mimes.hidden` does.
pub fn is_hidden(kind: &str) -> bool {
    HIDDEN.contains(&kind)
}

pub fn mime_of(kind: &str) -> String {
    MIMES
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or_else(|| format!("wiki/{kind}"), |(_, mime)| mime.to_string())
}

/// The AppView kind a mime names, if it names one.
pub fn kind_of(mime: &str) -> Option<&'static str> {
    MIMES
        .iter()
        .find(|(_, m)| *m == mime)
        .map(|(kind, _)| *kind)
}

fn mime_fields(kind: &str) -> MimeFields {
    MimeFields {
        id: mime_of(kind),
        // The icon comes from the mime id (`components::loader`); the table's
        // own column was never read.
        icon: String::new(),
        hidden: HIDDEN.contains(&kind),
        context: CONTEXTS.contains(&kind) && kind != "home",
    }
}

/// A page's text is `data.content` to a component, and its own field to the
/// AppView. `None` where there is neither.
pub fn data_with_content(
    data: Option<&serde_json::Value>,
    content: Option<&serde_json::Value>,
) -> Option<Jsonb> {
    let mut merged = match data {
        Some(serde_json::Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    if let Some(content) = content {
        merged.insert("content".to_string(), content.clone());
    }
    (!merged.is_empty()).then(|| Jsonb(serde_json::Value::Object(merged)))
}

/// The reverse, for a write: `data.content` lifted out into its own field.
pub fn content_and_data(
    data: &serde_json::Value,
) -> (Option<serde_json::Value>, serde_json::Value) {
    let mut rest = match data {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    let content = rest.remove("content");
    (content, serde_json::Value::Object(rest))
}

/// The name and picture behind a DID, from the `profiles` object a read carries.
pub fn user_ref(profiles: &serde_json::Value, did: &str) -> UserRef {
    let text = |key: &str| profiles[did][key].as_str().unwrap_or_default().to_string();
    UserRef {
        id: Uuid(did.to_string()),
        display_name: match text("display_name") {
            name if name.is_empty() => text("handle"),
            name => name,
        },
        avatar_url: text("avatar_url"),
    }
}

fn stamp(at: &Option<String>) -> Option<Timestamptz> {
    at.clone().map(Timestamptz)
}

/// An author chip as the member row the components draw chips from.
fn author_chip(
    author: &defs::AuthorView,
    ord: usize,
    profiles: &serde_json::Value,
) -> MemberFields {
    let user = author.did.as_deref().map(|did| user_ref(profiles, did));
    MemberFields {
        // A chip has no id of its own here; its place in the list is what a
        // component keys it by.
        id: Uuid(format!("author-{ord}")),
        name: author.display.clone().or_else(|| author.name.clone()),
        email: None,
        accepted: true,
        active: true,
        owner: false,
        hidden: false,
        node_id: author
            .did
            .clone()
            .or_else(|| author.context_id.clone())
            .map(Uuid),
        user,
        node: author.context_id.as_ref().map(|_| MemberNodeRef {
            mime_id: Some("wiki/group".to_string()),
        }),
    }
}

/// A seat on a roster as the member row the components know.
pub fn member(seat: &defs::MemberView) -> MemberFields {
    MemberFields {
        id: Uuid(seat.id.clone()),
        name: seat.name.clone(),
        email: seat.email.clone(),
        accepted: seat.accepted,
        active: seat.active,
        owner: seat.role == "owner",
        hidden: seat.hidden,
        node_id: seat.user_did.clone().map(Uuid),
        user: seat.user_did.as_ref().map(|did| UserRef {
            id: Uuid(did.clone()),
            display_name: seat
                .display_name
                .clone()
                .or_else(|| seat.handle.clone())
                .unwrap_or_default(),
            avatar_url: seat.avatar_url.clone().unwrap_or_default(),
        }),
        node: None,
    }
}

pub fn child(row: &defs::ChildView, profiles: &serde_json::Value) -> ChildNodeFields {
    let owner = row.owner_did.as_deref().map(|did| user_ref(profiles, did));
    ChildNodeFields {
        id: Uuid(row.id.clone()),
        name: row.name.clone(),
        key: row.slug.clone(),
        mime_id: Some(mime_of(&row.kind)),
        mutable: row.mutable,
        index: i32::try_from(row.idx).unwrap_or(0),
        created_at: stamp(&row.created_at),
        owner_id: row.owner_did.clone().map(Uuid),
        data: row.data.clone().map(Jsonb),
        mime: Some(mime_fields(&row.kind)),
        is_owner: None,
        is_context_owner: None,
        author_name: owner.as_ref().map(|o| o.display_name.clone()),
        author_avatar: owner.as_ref().map(|o| o.avatar_url.clone()),
        owner,
        parent: None,
    }
}

pub fn drawer_child(row: &defs::ChildView) -> DrawerChildFields {
    DrawerChildFields {
        id: Uuid(row.id.clone()),
        name: row.name.clone(),
        key: row.slug.clone(),
        mime_id: Some(mime_of(&row.kind)),
        mutable: row.mutable,
        data: row.data.clone().map(Jsonb),
        child_count: i32::try_from(row.child_count).unwrap_or(i32::MAX),
    }
}

pub fn crumb(row: &get_node::CrumbView) -> Crumb {
    Crumb {
        key: row.slug.clone(),
        // A place the reader may not read is named by its slug alone, which
        // they already hold: it is in the address.
        name: row.name.clone().unwrap_or_else(|| row.slug.clone()),
        mime_id: row.kind.as_deref().map(mime_of),
        ordinal: None,
        data: None,
    }
}

/// A node as a screen reads it, from one `getNode`.
pub fn node_with_children(read: &get_node::Output, viewer_did: Option<&str>) -> NodeWithChildren {
    let profiles = &read.profiles;
    let children = read
        .children
        .iter()
        .map(|row| {
            let mut child = child(row, profiles);
            child.is_owner = Some(viewer_did.is_some() && row.owner_did.as_deref() == viewer_did);
            child.is_context_owner = Some(read.viewer.is_context_owner);
            child
        })
        .collect();
    let base = match &read.node {
        get_node::OutputNode::Context(c) => Base {
            id: &c.id,
            name: &c.name,
            slug: &c.slug,
            path: &c.path,
            kind: &c.kind,
            parent_id: &c.parent_id,
            context_id: &c.id,
            owner_did: &c.owner_did,
            // A context is locked from the day it is made.
            mutable: false,
            idx: c.idx,
            attachable: c.attachable,
            created_at: &c.created_at,
            data: data_with_content(c.data.as_ref(), c.content.as_ref()),
            authors: &[],
        },
        get_node::OutputNode::Document(d) => Base {
            id: &d.id,
            name: &d.title,
            slug: &d.slug,
            path: &d.path,
            kind: &d.kind,
            parent_id: &d.parent_id,
            context_id: &d.context_id,
            owner_did: &d.owner_did,
            mutable: d.mutable.unwrap_or(true),
            idx: d.idx,
            attachable: d.attachable,
            created_at: &d.created_at,
            data: data_with_content(d.data.as_ref(), d.content.as_ref()),
            authors: &d.authors,
        },
    };
    let owner = base.owner_did.as_deref().map(|did| user_ref(profiles, did));
    NodeWithChildren {
        id: Uuid(base.id.clone()),
        name: base.name.clone(),
        key: base.slug.clone(),
        path: Some(base.path.clone()),
        mime_id: Some(mime_of(base.kind)),
        parent_id: base.parent_id.clone().map(Uuid),
        context_id: Some(Uuid(base.context_id.clone())),
        owner_id: base.owner_did.clone().map(Uuid),
        mutable: base.mutable,
        index: i32::try_from(base.idx.unwrap_or(0)).unwrap_or(0),
        get_index: read.ordinal.and_then(|n| i32::try_from(n).ok()),
        data: base.data,
        mime: Some(mime_fields(base.kind)),
        parent: None,
        children,
        members: base
            .authors
            .iter()
            .enumerate()
            .map(|(ord, author)| author_chip(author, ord, profiles))
            .collect(),
        is_owner: Some(read.viewer.is_owner),
        is_context_owner: Some(read.viewer.is_context_owner),
        attachable: base.attachable.unwrap_or(true),
        created_at: stamp(base.created_at),
        author_name: owner.as_ref().map(|o| o.display_name.clone()),
        author_avatar: owner.as_ref().map(|o| o.avatar_url.clone()),
        owner,
    }
}

/// What a context and a document have in common, as `node_with_children` needs it.
struct Base<'a> {
    id: &'a String,
    name: &'a String,
    slug: &'a String,
    path: &'a String,
    kind: &'a String,
    parent_id: &'a Option<String>,
    context_id: &'a String,
    owner_did: &'a Option<String>,
    mutable: bool,
    idx: Option<i64>,
    attachable: Option<bool>,
    created_at: &'a Option<String>,
    data: Option<Jsonb>,
    authors: &'a [defs::AuthorView],
}

pub fn context_node(c: &defs::ContextView) -> ContextNodeFields {
    ContextNodeFields {
        id: Uuid(c.id.clone()),
        name: c.name.clone(),
        key: c.slug.clone(),
        mime_id: Some(mime_of(&c.kind)),
        parent_id: c.parent_id.clone().map(Uuid),
        created_at: stamp(&c.created_at),
        data: c.data.clone().map(Jsonb),
    }
}

pub fn parent_ref(node: &defs::NodeRef) -> ParentNodeFields {
    ParentNodeFields {
        id: Uuid(node.id.clone()),
        name: node.title.clone(),
        key: node.path.rsplit('/').next().unwrap_or_default().to_string(),
        mime_id: Some(mime_of(&node.kind)),
        data: None,
        author_avatar: None,
        parent: None,
    }
}

pub fn search_hit(hit: &defs::HitView) -> NodeFields {
    NodeFields {
        id: Uuid(hit.id.clone()),
        name: hit.title.clone(),
        key: hit.path.rsplit('/').next().unwrap_or_default().to_string(),
        path: Some(hit.path.clone()),
        mime_id: Some(mime_of(&hit.kind)),
        parent_id: hit.parent.as_ref().map(|p| Uuid(p.id.clone())),
        context_id: Some(Uuid(hit.context_id.clone())),
        owner_id: None,
        mutable: false,
        index: 0,
        get_index: None,
        data: None,
        mime: Some(mime_fields(&hit.kind)),
        is_owner: None,
        is_context_owner: None,
        created_at: Some(Timestamptz(hit.created_at.clone())),
        parent: hit.parent.as_ref().map(parent_ref),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_kind_and_its_mime_are_each_others() {
        for (kind, mime) in MIMES {
            assert_eq!(mime_of(kind), *mime);
            assert_eq!(kind_of(mime), Some(*kind));
        }
        assert_eq!(mime_of("resolution"), "wiki/resolution");
        assert_eq!(kind_of("speak/list"), None);
    }

    #[test]
    fn a_pages_text_goes_into_its_data_and_comes_back_out() {
        let content = json!([{"children": [{"text": "Hej"}]}]);
        let data = json!({"image": "f1"});
        let merged = data_with_content(Some(&data), Some(&content)).expect("data");
        assert_eq!(merged.0, json!({"image": "f1", "content": content}));
        assert_eq!(content_and_data(&merged.0), (Some(content), data));
        assert_eq!(data_with_content(None, None), None);
        assert_eq!(content_and_data(&json!(null)), (None, json!({})));
    }

    /// What `getNode` says of a page, as the AppView's own tests have it say it.
    fn a_page() -> get_node::Output {
        serde_json::from_value(json!({
            "node": {
                "node": "document", "id": "d1", "context_id": "c1", "kind": "policy",
                "title": "Forslag 1", "slug": "forslag_1", "path": "hb/forslag_1",
                "parent_id": "c1", "idx": 2, "mutable": false, "attachable": true,
                "owner_did": "did:plc:alice", "visibility": "private",
                "content": [{"children": [{"text": "Vi foreslår"}]}],
                "data": {"image": "cover"},
                "created_at": "2026-05-01T00:00:00.000Z",
                "authors": [
                    {"kind": "user", "did": "did:plc:alice"},
                    {"kind": "free_text", "display": "Sekretariatet"},
                    {"kind": "context", "context_id": "c9", "name": "Aarhus", "path": "aarhus"}
                ]
            },
            "children": [{
                "node": "document", "id": "p1", "kind": "poll", "name": "Forslag 1",
                "slug": "afstemning", "path": "hb/forslag_1/afstemning", "idx": 0,
                "mutable": false, "attachable": true, "child_count": 0,
                "owner_did": "did:plc:bob"
            }],
            "crumbs": [
                {"slug": "hb", "path": "hb"},
                {"slug": "forslag_1", "path": "hb/forslag_1", "id": "d1", "kind": "policy",
                 "name": "Forslag 1", "node": "document"}
            ],
            "ordinal": 1,
            "profiles": {"did:plc:alice": {"display_name": "Alice", "avatar_url": "a.png"},
                         "did:plc:bob": {"handle": "bob.example"}},
            "viewer": {"can_create": ["change", "comment"], "is_owner": true, "is_member": true,
                       "is_context_owner": false, "can_vote": true}
        }))
        .expect("a getNode answer")
    }

    #[test]
    fn a_page_reads_as_the_node_the_components_know() {
        let node = node_with_children(&a_page(), Some("did:plc:alice"));
        assert_eq!(node.mime_id.as_deref(), Some("vote/policy"));
        assert_eq!(
            (node.key.as_str(), node.index, node.get_index),
            ("forslag_1", 2, Some(1))
        );
        assert!(!node.mutable, "a submitted motion");
        let data = node.data.expect("data").0;
        assert_eq!(data["content"][0]["children"][0]["text"], "Vi foreslår");
        assert_eq!(data["image"], "cover");
        assert_eq!(node.owner.expect("owner").display_name, "Alice");
        assert_eq!(
            (node.is_owner, node.is_context_owner),
            (Some(true), Some(false))
        );

        let chips: Vec<String> = node.members.iter().map(MemberFields::label).collect();
        assert_eq!(chips, ["Alice", "Sekretariatet", "Aarhus"]);
        assert_eq!(
            node.members[2].node_id,
            Some(Uuid("c9".into())),
            "a group is named by its id"
        );

        let poll = &node.children[0];
        assert_eq!(poll.mime_id.as_deref(), Some("vote/poll"));
        assert!(
            poll.mime.as_ref().expect("mime").hidden,
            "a poll is found through its own app"
        );
        assert_eq!(poll.is_owner, Some(false), "bob opened it");
        assert_eq!(
            poll.author_name.as_deref(),
            Some("bob.example"),
            "a handle, for want of a name"
        );
    }

    #[test]
    fn a_crumb_the_reader_may_not_read_is_its_slug() {
        let crumbs: Vec<Crumb> = a_page().crumbs.iter().map(crumb).collect();
        assert_eq!(
            (crumbs[0].name.as_str(), crumbs[0].mime_id.as_deref()),
            ("hb", None)
        );
        assert_eq!(crumbs[1].mime_id.as_deref(), Some("vote/policy"));
    }
}
