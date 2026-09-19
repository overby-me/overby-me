//! Reading the tree: a node by its path or id with what a screen draws around
//! it, its children, the way down to it.

use super::seen::{saw_node, seen, Seen};
use super::{ask, ask_quiet, client, is_absent, map, offline_copy, reported};
use crate::model::{
    ChildNodeFields, Crumb, DrawerChildFields, InsertedNode, NodeWithChildren, NodesInsertInput,
    NodesSetInput, Uuid,
};
use appview_client::{
    close_poll, create_context, create_document, delete_comment, delete_document, get_node,
    join_speaker_list, list_children, list_members, move_document, set_canvas_open, update_context,
    update_document,
};

/// One `getNode`, as the node the components know. `None` for a node that is
/// not there, or not there for this reader: the AppView says the same of both.
async fn read_node(
    access_token: Option<&str>,
    params: get_node::Params,
    user_id: &str,
) -> Result<Option<NodeWithChildren>, String> {
    let client = client(access_token);
    // Quiet only so that "not there" is an answer and no fault.
    let read = match ask_quiet(true, || client.get_node(&params)).await {
        Ok(read) => read,
        Err(e) if is_absent(&e) => return Ok(None),
        Err(e) => return Err(reported("getNode", &e)),
    };
    let viewer = Some(user_id).filter(|id| !id.is_empty());
    let mut node = map::node_with_children(&read, viewer);
    match &read.node {
        get_node::OutputNode::Context(c) => saw_node(&c.id, "context", &c.kind),
        get_node::OutputNode::Document(d) => saw_node(&d.id, "document", &d.kind),
    }
    for child in &read.children {
        saw_node(&child.id, &child.node, &child.kind);
    }
    // A poll is a node to the components: what is asked in its `data`, open
    // while `mutable`. So are the polls among a page's children.
    if node.mime_id.as_deref() == Some("vote/poll") {
        if let Some(poll) = super::vote::read_poll(access_token, &node.id.0).await {
            node.data = Some(crate::model::Jsonb(super::vote::poll_data(&poll)));
            node.mutable = poll.open;
        }
    }
    if node
        .children
        .iter()
        .any(|c| c.mime_id.as_deref() == Some("vote/poll"))
    {
        let polls = super::vote::polls_on(access_token, &node.id.0).await;
        for child in &mut node.children {
            if let Some(poll) = polls.iter().find(|p| p.id == child.id.0) {
                child.data = Some(crate::model::Jsonb(super::vote::poll_data(poll)));
                child.mutable = poll.open;
            }
        }
    }
    // The home lists who runs the site, which no other page does of its members.
    if matches!(&read.node, get_node::OutputNode::Context(c) if c.kind == "home") {
        let members = list_members::Params {
            context: node.id.0.clone(),
            ..Default::default()
        };
        if let Ok(roster) = client.list_members(&members).await {
            node.members = roster.members.iter().map(map::member).collect();
        }
    }
    Ok(Some(node))
}

pub async fn query_node_by_id(
    access_token: Option<&str>,
    id: &str,
    user_id: &str,
) -> Result<Option<NodeWithChildren>, String> {
    let params = get_node::Params {
        id: Some(id.to_string()),
        ..Default::default()
    };
    read_node(access_token, params, user_id).await
}

/// The home: the one context everything else is under, at the empty path.
pub async fn query_root_node(
    access_token: Option<&str>,
    user_id: &str,
) -> Result<Option<NodeWithChildren>, String> {
    let params = get_node::Params {
        path: Some(String::new()),
        ..Default::default()
    };
    read_node(access_token, params, user_id).await
}

/// The node at a path. The page a reader opened before the tunnel is the page
/// they meant to read in it: anything that answered, "no such node" included,
/// replaces the remembered copy, and only an unreachable server falls back to it.
pub async fn resolve_path(
    access_token: Option<&str>,
    segments: &[String],
    user_id: &str,
) -> Result<Option<NodeWithChildren>, String> {
    if segments.is_empty() {
        return Ok(None);
    }
    let path = segments.join("/");
    let key = format!("node:{user_id}:{path}");
    let params = get_node::Params {
        path: Some(path),
        ..Default::default()
    };
    match read_node(access_token, params, user_id).await {
        Ok(Some(node)) => {
            super::remember(&key, &node);
            Ok(Some(node))
        }
        Ok(None) => Ok(None),
        Err(e) => offline_copy::<NodeWithChildren>(&key, &e)
            .map(Some)
            .ok_or(e),
    }
}

/// A crumb for every segment, the ones the reader may not read as their slug.
pub async fn path_crumbs(
    access_token: Option<&str>,
    segments: &[String],
) -> Result<Vec<Crumb>, String> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }
    let key = format!("crumbs:{}", segments.join("/"));
    let client = client(access_token);
    let params = get_node::Params {
        path: Some(segments.join("/")),
        ..Default::default()
    };
    match ask_quiet(true, || client.get_node(&params)).await {
        Ok(read) => {
            let mut crumbs: Vec<Crumb> = read.crumbs.iter().map(map::crumb).collect();
            // The node's own number is the last crumb's: the A of a motion.
            if let (Some(last), Some(ordinal)) = (crumbs.last_mut(), read.ordinal) {
                last.ordinal = usize::try_from(ordinal - 1).ok();
            }
            super::remember(&key, &crumbs);
            Ok(crumbs)
        }
        // A trail to somewhere that is not there is still a trail of its slugs.
        Err(e) if is_absent(&e) => Ok(segments
            .iter()
            .map(|segment| Crumb {
                key: segment.clone(),
                name: segment.clone(),
                mime_id: None,
                ordinal: None,
                data: None,
            })
            .collect()),
        Err(e) => {
            let message = reported("getNode", &e);
            offline_copy::<Vec<Crumb>>(&key, &message).ok_or(message)
        }
    }
}

async fn listed(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<list_children::Output, String> {
    let client = client(access_token);
    let params = list_children::Params {
        parent: parent_id.to_string(),
    };
    let listed = ask("listChildren", true, || client.list_children(&params)).await?;
    for child in &listed.children {
        saw_node(&child.id, &child.node, &child.kind);
    }
    Ok(listed)
}

/// A node's children as a page draws them: every child of either kind, and for
/// a document what it says too, since a resolution shows its amendments' text.
pub async fn query_children(
    access_token: Option<&str>,
    parent_id: &str,
    user_id: &str,
) -> Result<Vec<ChildNodeFields>, String> {
    let listed = listed(access_token, parent_id).await?;
    Ok(listed
        .children
        .iter()
        .map(|row| {
            let mut child = map::child(row, &listed.profiles);
            child.is_owner = Some(!user_id.is_empty() && row.owner_did.as_deref() == Some(user_id));
            if let Some(whole) = listed.documents.iter().find(|d| d.id == row.id) {
                child.data = map::data_with_content(whole.data.as_ref(), whole.content.as_ref());
            }
            child
        })
        .collect())
}

/// One level of the drawer's tree. Without what a listing leaves out in the
/// interim too: a poll and a canvas are found through apps of their own.
pub async fn query_drawer_children(
    access_token: Option<&str>,
    parent_id: &str,
    _user_id: &str,
) -> Result<Vec<DrawerChildFields>, String> {
    let listed = listed(access_token, parent_id).await?;
    Ok(listed
        .children
        .iter()
        .filter(|row| !map::is_hidden(&row.kind))
        .map(map::drawer_child)
        .collect())
}

/// The path of a node as its segments, or none for one that cannot be read.
pub async fn path_from_id(access_token: Option<&str>, id: &str) -> Result<Vec<String>, String> {
    let node = query_node_by_id(access_token, id, "").await?;
    Ok(node
        .and_then(|n| n.path)
        .map(|path| {
            path.split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

pub async fn node_path(access_token: Option<&str>, node_id: &str) -> Vec<String> {
    path_from_id(access_token, node_id)
        .await
        .unwrap_or_default()
}

/// The mimes the caller may create under a node, from the server's own rule
/// (`viewer.can_create`), so a screen offers what will be accepted.
pub async fn node_insert_mimes(access_token: Option<&str>, node_id: &str) -> Vec<String> {
    let client = client(access_token);
    let params = get_node::Params {
        id: Some(node_id.to_string()),
        ..Default::default()
    };
    match ask_quiet(true, || client.get_node(&params)).await {
        Ok(read) => read
            .viewer
            .can_create
            .iter()
            .map(|kind| map::mime_of(kind))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Whether `target` is `ancestor` or somewhere under it, by their paths.
pub async fn is_descendant_of(access_token: Option<&str>, target: &str, ancestor: &str) -> bool {
    if target == ancestor {
        return true;
    }
    let (under, over) = (
        node_path(access_token, target).await,
        node_path(access_token, ancestor).await,
    );
    !over.is_empty() && under.len() > over.len() && under[..over.len()] == over[..]
}

// --- Writing the tree ---

/// What kind of thing `id` is: what a read said of it, or for an id no read has
/// seen, what the AppView says now. A document, where even that says nothing,
/// since that is what most writes are to.
async fn kind_of(access_token: Option<&str>, id: &str) -> Seen {
    if let Some(known) = seen(id) {
        return known;
    }
    let _ = query_node_by_id(access_token, id, "").await;
    seen(id).unwrap_or_else(|| Seen::Document("document".to_string()))
}

/// Make a node. The AppView picks the key from the name and says which it took,
/// so the interim's search for a free one has nothing left to do.
pub async fn insert_node(
    access_token: Option<&str>,
    input: NodesInsertInput,
) -> Result<Option<InsertedNode>, String> {
    let mime = input.mime_id.clone().unwrap_or_default();
    let parent = input.parent_id.clone().map(|p| p.0).unwrap_or_default();
    let client = client(access_token);
    if mime == "speak/speak" {
        // The interim keeps the kind of speech as a number in a string.
        let kind = match &input.data {
            Some(data) => data.0.as_str().and_then(|k| k.parse::<i64>().ok()),
            None => None,
        };
        let join = join_speaker_list::Input {
            list_id: parent,
            kind,
        };
        let joined = ask("joinSpeakerList", false, || client.join_speaker_list(&join)).await?;
        super::seen::saw(&joined.id, Seen::SpeakerEntry);
        return Ok(Some(InsertedNode {
            id: Uuid(joined.id),
            key: input.key.unwrap_or_default(),
        }));
    }
    let Some(kind) = map::kind_of(&mime).filter(|kind| map::is_content(kind)) else {
        return Err(format!("{mime} is not made as a node here"));
    };
    let (content, data) = match &input.data {
        Some(data) => map::content_and_data(&data.0),
        None => (None, serde_json::json!({})),
    };
    let context_id = match input.context_id.clone() {
        Some(context) => context.0,
        // A context is its own context, so a node made straight under one and
        // told nothing else is in its parent.
        None => parent.clone(),
    };
    let create = create_document::Input {
        context_id,
        parent_id: Some(parent),
        kind: kind.to_string(),
        title: input.name.clone().unwrap_or_default(),
        content,
        data: data
            .as_object()
            .is_some_and(|d| !d.is_empty())
            .then_some(data),
    };
    let made = ask("createDocument", false, || client.create_document(&create)).await?;
    saw_node(&made.id, "document", kind);
    if input.mutable == Some(false) {
        let submit = update_document::Input {
            id: made.id.clone(),
            mutable: Some(false),
            ..Default::default()
        };
        ask("updateDocument", false, || client.update_document(&submit)).await?;
    }
    Ok(Some(InsertedNode {
        id: Uuid(made.id),
        key: made.slug,
    }))
}

pub async fn insert_node_named(
    access_token: Option<&str>,
    input: NodesInsertInput,
    _name: &str,
) -> Result<Option<InsertedNode>, String> {
    insert_node(access_token, input).await
}

/// Change a node. One call to the interim; here, the method that belongs to the
/// kind of thing it is and to what is being changed about it.
pub async fn update_node(
    access_token: Option<&str>,
    id: &str,
    set: NodesSetInput,
) -> Result<bool, String> {
    let client = client(access_token);
    let kind = kind_of(access_token, id).await;
    // Emptying a comment in place is how the interim deletes one that has been
    // answered. The AppView decides that itself, from whether it has been.
    let emptied = set
        .data
        .as_ref()
        .is_some_and(|d| d.0.get("deleted") == Some(&serde_json::Value::Bool(true)));
    if kind == Seen::Comment && emptied {
        let gone = delete_comment::Input { id: id.to_string() };
        ask("deleteComment", false, || client.delete_comment(&gone)).await?;
        return Ok(true);
    }
    if let Some(parent) = &set.parent_id {
        let to = move_document::Input {
            id: id.to_string(),
            parent_id: parent.0.clone(),
        };
        ask("moveDocument", false, || client.move_document(&to)).await?;
    }
    let (content, data) = match &set.data {
        Some(data) => {
            let (content, rest) = map::content_and_data(&data.0);
            (content, Some(rest))
        }
        None => (None, None),
    };
    let created_at = set.created_at.as_ref().map(|at| at.0.clone());
    match kind {
        Seen::Context => {
            let change = update_context::Input {
                id: id.to_string(),
                name: set.name.clone(),
                attachable: set.attachable,
                content,
                data,
                created_at,
                ..Default::default()
            };
            ask("updateContext", false, || client.update_context(&change)).await?;
        }
        Seen::Document(kind) if kind == "poll" && set.mutable == Some(false) => {
            let close = close_poll::Input { id: id.to_string() };
            ask("closePoll", false, || client.close_poll(&close)).await?;
        }
        Seen::Document(kind) if kind == "canvas" && set.mutable.is_some() => {
            let open = set_canvas_open::Input {
                id: id.to_string(),
                open: set.mutable.unwrap_or(true),
            };
            ask("setCanvasOpen", false, || client.set_canvas_open(&open)).await?;
        }
        _ => {
            let change = update_document::Input {
                id: id.to_string(),
                title: set.name.clone(),
                content,
                data,
                mutable: set.mutable,
                attachable: set.attachable,
                idx: set.index.map(i64::from),
                created_at,
            };
            ask("updateDocument", false, || client.update_document(&change)).await?;
        }
    }
    Ok(true)
}

/// Whether members may add to a context directly (the lock on the whole place).
pub async fn set_context_attachable(
    access_token: Option<&str>,
    context_id: &str,
    attachable: bool,
) -> Result<u64, String> {
    let client = client(access_token);
    let change = update_context::Input {
        id: context_id.to_string(),
        attachable: Some(attachable),
        ..Default::default()
    };
    ask("updateContext", false, || client.update_context(&change)).await?;
    Ok(1)
}

/// Make a group, an event or a site, with its maker as its first owner. One
/// call, where the interim makes four writes from the browser.
pub async fn create_context(
    access_token: Option<&str>,
    parent_id: &str,
    _parent_context_id: &str,
    mime_id: &str,
    name: &str,
    _creator: Option<&crate::session::User>,
) -> Result<InsertedNode, String> {
    let Some(kind) = map::kind_of(mime_id) else {
        return Err(format!("{mime_id} is not a place"));
    };
    let client = client(access_token);
    let make = create_context::Input {
        kind: kind.to_string(),
        name: name.to_string(),
        parent_id: parent_id.to_string(),
    };
    let made = ask("createContext", false, || client.create_context(&make)).await?;
    saw_node(&made.id, "context", kind);
    Ok(InsertedNode {
        id: Uuid(made.id),
        key: made.path.rsplit('/').next().unwrap_or_default().to_string(),
    })
}

/// Put one node in the bin. Here the interim deletes a row outright, which it
/// does to what has no bin (a reaction, a report, a place in a queue); a page
/// goes to the bin, from where it can still be purged.
pub async fn delete_node(access_token: Option<&str>, id: &str) -> Result<bool, String> {
    use appview_client::{
        delete_feedback, delete_speaker_list, leave_speaker_list, remove_reaction,
    };
    let client = client(access_token);
    match kind_of(access_token, id).await {
        Seen::Reaction { subject, emoji } => {
            let taken_back = remove_reaction::Input { subject, emoji };
            ask("removeReaction", false, || {
                client.remove_reaction(&taken_back)
            })
            .await?;
        }
        Seen::Feedback => {
            let gone = delete_feedback::Input { id: id.to_string() };
            ask("deleteFeedback", false, || client.delete_feedback(&gone)).await?;
        }
        Seen::SpeakerEntry => {
            let left = leave_speaker_list::Input {
                entry_id: id.to_string(),
            };
            ask("leaveSpeakerList", false, || {
                client.leave_speaker_list(&left)
            })
            .await?;
        }
        Seen::SpeakerList => {
            let gone = delete_speaker_list::Input {
                list_id: id.to_string(),
            };
            ask("deleteSpeakerList", false, || {
                client.delete_speaker_list(&gone)
            })
            .await?;
        }
        Seen::Comment => {
            let gone = delete_comment::Input { id: id.to_string() };
            ask("deleteComment", false, || client.delete_comment(&gone)).await?;
        }
        Seen::Context => {
            return super::bin_node(access_token, id, None, None)
                .await
                .map(|n| n > 0)
        }
        Seen::Document(_) => {
            let gone = delete_document::Input { id: id.to_string() };
            ask("deleteDocument", false, || client.delete_document(&gone)).await?;
        }
    }
    Ok(true)
}

/// Clear away a node that has lost its parent, and what is under it. The one
/// caller is the missing-parent view, which is whoever runs the site.
pub fn delete_node_deep(
    access_token: Option<String>,
    id: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        let client = client(access_token.as_deref());
        let gone = appview_client::purge_orphan::Input { id };
        ask("purgeOrphan", false, || client.purge_orphan(&gone)).await?;
        Ok(())
    })
}
