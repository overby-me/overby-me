//! Speaker lists: who is waiting for the floor, and whose turn it is.
//!
//! To its component a list is a hidden child of its context, open while
//! `mutable`, with the turn's limit and the moment it began in its `data`, and
//! everyone waiting as a child row under it whose `data` is the kind of speech
//! and whose `index` is the chair's say over the order. The AppView keeps lists
//! and queues in tables of their own and serves a queue already in order, so a
//! list is read there and dressed as that node, each place in the queue
//! carrying its position as its index.

use super::seen::{saw, Seen};
use super::{ask, ask_quiet, client, map};
use crate::model::{ChildNodeFields, InsertedNode, Jsonb, NodeWithChildren, Timestamptz, Uuid};
use appview_client::{create_speaker_list, list_speaker_lists, move_speaker, update_speaker_list};

/// A context's lists, remembered as lists of that context.
pub(crate) async fn lists_of(
    access_token: Option<&str>,
    context_id: &str,
) -> Option<list_speaker_lists::Output> {
    let client = client(access_token);
    let params = list_speaker_lists::Params {
        context: context_id.to_string(),
    };
    // Quiet: whoever may not read the context has no lists to be shown.
    let listed = ask_quiet(true, || client.list_speaker_lists(&params))
        .await
        .ok()?;
    for list in &listed.lists {
        let context = context_id.to_string();
        saw(&list.id, Seen::SpeakerList { context });
        for entry in &list.queue {
            saw(&entry.id, Seen::SpeakerEntry);
        }
    }
    Some(listed)
}

/// A list as the hidden child its context's page carries.
pub(crate) fn list_child(list: &list_speaker_lists::SpeakerListView) -> ChildNodeFields {
    ChildNodeFields {
        id: Uuid(list.id.clone()),
        name: list.name.clone(),
        key: list.id.clone(),
        mime_id: Some("speak/list".to_string()),
        mutable: list.open,
        index: 0,
        created_at: None,
        owner_id: None,
        data: Some(Jsonb(list_data(list))),
        mime: None,
        is_owner: None,
        is_context_owner: None,
        owner: None,
        author_name: None,
        author_avatar: None,
        parent: None,
    }
}

/// The turn's limit and the moment it began, where the component reads them.
fn list_data(list: &list_speaker_lists::SpeakerListView) -> serde_json::Value {
    serde_json::json!({
        "time": list.turn_secs,
        "updatedAt": list.turn_started_at.clone().unwrap_or_default(),
    })
}

/// One list as the node its component loads: the queue as its children, in the
/// order the AppView serves it.
pub(crate) async fn list_node(
    access_token: Option<&str>,
    list_id: &str,
    context_id: &str,
    viewer: &str,
) -> Option<NodeWithChildren> {
    let listed = lists_of(access_token, context_id).await?;
    let list = listed.lists.iter().find(|l| l.id == list_id)?;
    let children = list
        .queue
        .iter()
        .enumerate()
        .map(|(place, entry)| {
            let owner = map::user_ref(&listed.profiles, &entry.speaker_did);
            ChildNodeFields {
                id: Uuid(entry.id.clone()),
                name: owner.display_name.clone(),
                key: super::seen::key_of(&entry.id),
                mime_id: Some("speak/speak".to_string()),
                mutable: false,
                index: i32::try_from(place).unwrap_or(i32::MAX),
                created_at: Some(Timestamptz(entry.created_at.clone())),
                owner_id: Some(Uuid(entry.speaker_did.clone())),
                // The kind of speech, as the string the interim keeps it as.
                data: Some(Jsonb(serde_json::Value::String(entry.kind.to_string()))),
                mime: None,
                is_owner: Some(entry.speaker_did == viewer),
                is_context_owner: None,
                author_name: Some(owner.display_name.clone()),
                author_avatar: Some(owner.avatar_url.clone()),
                owner: Some(owner),
                parent: None,
            }
        })
        .collect();
    Some(NodeWithChildren {
        id: Uuid(list.id.clone()),
        name: list.name.clone(),
        key: list.id.clone(),
        path: None,
        mime_id: Some("speak/list".to_string()),
        parent_id: Some(Uuid(context_id.to_string())),
        context_id: Some(Uuid(context_id.to_string())),
        owner_id: None,
        mutable: list.open,
        index: 0,
        get_index: None,
        data: Some(Jsonb(list_data(list))),
        mime: None,
        parent: None,
        children,
        members: Vec::new(),
        is_owner: None,
        is_context_owner: None,
        attachable: list.open,
        created_at: None,
        owner: None,
        author_name: None,
        author_avatar: None,
    })
}

pub async fn create_speaker_list(
    access_token: Option<&str>,
    context_id: &str,
    name: &str,
) -> Result<InsertedNode, String> {
    let client = client(access_token);
    let list = create_speaker_list::Input {
        context_id: context_id.to_string(),
        name: name.to_string(),
    };
    let made = ask("createSpeakerList", false, || {
        client.create_speaker_list(&list)
    })
    .await?;
    let context = context_id.to_string();
    saw(&made.id, Seen::SpeakerList { context });
    Ok(InsertedNode {
        key: made.id.clone(),
        id: Uuid(made.id),
    })
}

/// The chair's changes to a list: open or closed, its name, and the limit on a
/// turn, which starts the clock afresh, as setting it does in the interim.
pub(crate) async fn update_list(
    access_token: Option<&str>,
    list_id: &str,
    set: &crate::model::NodesSetInput,
) -> Result<(), String> {
    let client = client(access_token);
    let turn_secs = set
        .data
        .as_ref()
        .and_then(|d| d.0.get("time"))
        .and_then(serde_json::Value::as_f64)
        .map(|secs| secs as i64);
    let change = update_speaker_list::Input {
        id: list_id.to_string(),
        name: set.name.clone().filter(|n| !n.trim().is_empty()),
        open: set.mutable,
        turn_secs,
    };
    ask("updateSpeakerList", false, || {
        client.update_speaker_list(&change)
    })
    .await?;
    Ok(())
}

/// The chair's say over the order. The component asks for an index below the
/// lowest or above the highest; the AppView is told the front or the back.
pub(crate) async fn move_entry(
    access_token: Option<&str>,
    entry_id: &str,
    index: i32,
) -> Result<(), String> {
    let client = client(access_token);
    let to = move_speaker::Input {
        entry_id: entry_id.to_string(),
        to: if index < 0 { "front" } else { "back" }.to_string(),
    };
    ask("moveSpeaker", false, || client.move_speaker(&to)).await?;
    Ok(())
}
