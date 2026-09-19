//! The bin: what a delete put there, how it is listed, and the two ways out of
//! it, back into the tree or gone for good.

use super::seen::{saw, saw_node, seen, Seen};
use super::{ask, client};
use crate::model::{Timestamptz, Uuid};
use appview_client::{
    delete_comment, delete_context, delete_document, list_deleted, purge_comment, purge_document,
    restore_comment, restore_context, restore_document,
};

/// One thing somebody deleted, as the bin lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeletedNodeFields {
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub key: Option<String>,
    pub path: Option<String>,
    pub mime_id: Option<String>,
    pub deleted_at: Option<Timestamptz>,
}

/// A context's bin: one row per delete someone asked for, newest first. Who
/// sees which rows is the AppView's to say: an owner all of it, anyone else
/// what they made themselves.
pub async fn query_deleted(
    access_token: Option<&str>,
    context_id: &str,
    _node_id: &str,
) -> Result<Vec<DeletedNodeFields>, String> {
    let client = client(access_token);
    let params = list_deleted::Params {
        context: context_id.to_string(),
    };
    let bin = ask("listDeleted", true, || client.list_deleted(&params)).await?;
    Ok(bin
        .deleted
        .iter()
        .map(|entry| {
            match entry.node.as_str() {
                "comment" => saw(&entry.id, Seen::Comment),
                node => saw_node(&entry.id, node, &entry.kind),
            }
            DeletedNodeFields {
                id: Some(Uuid(entry.id.clone())),
                name: Some(entry.title.clone()),
                key: entry.path.rsplit('/').next().map(str::to_string),
                path: Some(entry.path.clone()),
                mime_id: Some(super::map::mime_of(&entry.kind)),
                deleted_at: Some(Timestamptz(entry.deleted_at.clone())),
            }
        })
        .collect())
}

/// Put a node, and everything under it, in the bin. Returns how much went.
pub async fn bin_node(
    access_token: Option<&str>,
    node_id: &str,
    _path: Option<&str>,
    _actor: Option<&str>,
) -> Result<u32, String> {
    let client = client(access_token);
    let id = node_id.to_string();
    let binned = match seen(node_id) {
        Some(Seen::Comment) => {
            let gone = delete_comment::Input { id };
            ask("deleteComment", false, || client.delete_comment(&gone)).await?;
            1
        }
        Some(Seen::Context) => {
            let gone = delete_context::Input { id };
            ask("deleteContext", false, || client.delete_context(&gone))
                .await?
                .binned
        }
        _ => {
            let gone = delete_document::Input { id };
            ask("deleteDocument", false, || client.delete_document(&gone))
                .await?
                .binned
        }
    };
    Ok(u32::try_from(binned).unwrap_or(u32::MAX))
}

/// Bring back what one bin entry holds.
pub async fn restore_node(access_token: Option<&str>, root_id: &str) -> Result<u32, String> {
    let client = client(access_token);
    let id = root_id.to_string();
    let restored = match seen(root_id) {
        Some(Seen::Comment) => {
            let back = restore_comment::Input { id };
            ask("restoreComment", false, || client.restore_comment(&back))
                .await?
                .restored
        }
        Some(Seen::Context) => {
            let back = restore_context::Input { id };
            ask("restoreContext", false, || client.restore_context(&back))
                .await?
                .restored
        }
        _ => {
            let back = restore_document::Input { id };
            ask("restoreDocument", false, || client.restore_document(&back))
                .await?
                .restored
        }
    };
    Ok(u32::try_from(restored).unwrap_or(u32::MAX))
}

/// Delete for good what one bin entry holds. A place is nobody's to purge, and
/// the AppView says so.
pub async fn purge_node(access_token: Option<&str>, root_id: &str) -> Result<u32, String> {
    let client = client(access_token);
    let id = root_id.to_string();
    let purged = match seen(root_id) {
        Some(Seen::Comment) => {
            let gone = purge_comment::Input { id };
            ask("purgeComment", false, || client.purge_comment(&gone))
                .await?
                .purged
        }
        _ => {
            let gone = purge_document::Input { id };
            ask("purgeDocument", false, || client.purge_document(&gone))
                .await?
                .purged
        }
    };
    Ok(u32::try_from(purged).unwrap_or(u32::MAX))
}
