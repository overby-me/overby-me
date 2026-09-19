//! Reading the tree: a node by its path or id with what a screen draws around
//! it, its children, the way down to it.

use super::{ask, ask_quiet, client, is_absent, map, offline_copy, reported};
use crate::model::{ChildNodeFields, Crumb, DrawerChildFields, NodeWithChildren};
use appview_client::{get_node, list_children, list_members};

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
            crate::offline::put(&key, &node);
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
            crate::offline::put(&key, &crumbs);
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
    ask("listChildren", true, || client.list_children(&params)).await
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
