//! What people say: comments and reactions, the feed they show up in, what a
//! person or a group has put forward, and finding things by their words.

use super::seen::{placed, saw, saw_node, Seen};
use super::{ask, client, map};
use crate::model::{ChildNodeFields, ContextNodeFields, Jsonb, NodeFields, Timestamptz, Uuid};
use appview_client::{
    add_reaction, defs, get_comments, get_reactions, list_contributions, list_recent, post_comment,
    search,
};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// The page a comment's thread is on, and what it is called, as the reads
    /// that mention the comment said it. A comment has no page of its own, so a
    /// link to one goes here.
    static HOSTS: RefCell<HashMap<String, (String, Option<String>)>> = RefCell::new(HashMap::new());
}

fn hosted(comment: &str, root: &str, name: Option<&str>) {
    HOSTS.with(|hosts| {
        let mut hosts = hosts.borrow_mut();
        let known = hosts.get(comment).and_then(|(_, name)| name.clone());
        let name = name.map(str::to_string).or(known);
        hosts.insert(comment.to_string(), (root.to_string(), name));
    });
}

/// A comment's stored `data`, as the interim shapes it and the components read it.
pub(crate) fn comment_data(text: &str, image: Option<&str>) -> serde_json::Value {
    match image.filter(|i| !i.is_empty()) {
        Some(id) => serde_json::json!({ "text": text, "image": id }),
        None => serde_json::json!({ "text": text }),
    }
}

fn comment_node(
    comment: &defs::CommentView,
    profiles: &serde_json::Value,
    viewer: &get_comments::OutputViewer,
) -> ChildNodeFields {
    let owner = comment
        .author
        .did
        .as_deref()
        .map(|did| map::user_ref(profiles, did));
    let emptied = comment.tombstone.unwrap_or(false);
    let data = match emptied {
        // As the interim leaves one it has emptied, which is what the component
        // looks for to draw it as deleted.
        true => serde_json::json!({ "deleted": true }),
        false => comment_data(&comment.text, comment.image.as_deref()),
    };
    let name = owner
        .as_ref()
        .map(|o| o.display_name.clone())
        .or_else(|| comment.author.display.clone())
        .unwrap_or_default();
    ChildNodeFields {
        id: Uuid(comment.id.clone()),
        name: name.clone(),
        key: comment.id.clone(),
        mime_id: Some("vote/comment".to_string()),
        mutable: false,
        index: 0,
        created_at: comment.created_at.clone().map(Timestamptz),
        owner_id: comment.author.did.clone().map(Uuid),
        data: Some(Jsonb(data)),
        mime: None,
        is_owner: Some(viewer.did.is_some() && comment.author.did == viewer.did),
        is_context_owner: Some(viewer.is_context_owner),
        author_name: Some(name).filter(|n| !n.is_empty()),
        author_avatar: owner.as_ref().map(|o| o.avatar_url.clone()),
        owner,
        parent: None,
    }
}

/// The comments on a node (a page, or another comment), oldest first.
pub async fn query_comments(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<Vec<ChildNodeFields>, String> {
    let client = client(access_token);
    let params = get_comments::Params {
        on: parent_id.to_string(),
    };
    let thread = ask("getComments", true, || client.get_comments(&params)).await?;
    Ok(thread
        .comments
        .iter()
        .map(|comment| {
            saw(&comment.id, Seen::Comment);
            placed(&comment.id, &comment.context_id);
            if let Some(root) = &comment.root_id {
                hosted(&comment.id, root, None);
            }
            comment_node(comment, &thread.profiles, &thread.viewer)
        })
        .collect())
}

/// Post a comment under `parent_id`. The AppView knows who is writing and which
/// context the thread is in, so the name, the key and the context are not sent.
pub async fn insert_comment(
    access_token: Option<&str>,
    parent_id: &str,
    _context_id: Option<&str>,
    _key: &str,
    _author: &str,
    text: &str,
    image: Option<&str>,
) -> Result<bool, String> {
    let client = client(access_token);
    let say = post_comment::Input {
        on_id: parent_id.to_string(),
        text: text.to_string(),
        image: image.filter(|i| !i.is_empty()).map(str::to_string),
        ..Default::default()
    };
    let said = ask("postComment", false, || client.post_comment(&say)).await?;
    saw(&said.id, Seen::Comment);
    Ok(true)
}

/// The reactions on a node, oldest first, each as the row the component groups
/// by emoji and marks the caller's own in.
pub async fn query_reactions(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<Vec<ChildNodeFields>, String> {
    let client = client(access_token);
    let params = get_reactions::Params {
        subject: parent_id.to_string(),
    };
    let listed = ask("getReactions", true, || client.get_reactions(&params)).await?;
    Ok(listed
        .reactions
        .iter()
        .map(|reaction| {
            saw(
                &reaction.id,
                Seen::Reaction {
                    subject: reaction.subject_uri.clone(),
                    emoji: reaction.emoji.clone(),
                },
            );
            ChildNodeFields {
                id: Uuid(reaction.id.clone()),
                name: reaction.emoji.clone(),
                key: reaction.id.clone(),
                mime_id: Some("vote/reaction".to_string()),
                mutable: false,
                index: 0,
                created_at: reaction.created_at.clone().map(Timestamptz),
                owner_id: reaction.reactor_did.clone().map(Uuid),
                data: Some(Jsonb(serde_json::json!({ "emoji": reaction.emoji }))),
                mime: None,
                is_owner: None,
                is_context_owner: None,
                owner: None,
                author_name: None,
                author_avatar: None,
                parent: None,
            }
        })
        .collect())
}

pub async fn insert_reaction(
    access_token: Option<&str>,
    parent_id: &str,
    _context_id: Option<&str>,
    emoji: &str,
) -> Result<bool, String> {
    let client = client(access_token);
    let react = add_reaction::Input {
        subject: parent_id.to_string(),
        emoji: emoji.to_string(),
    };
    let made = ask("addReaction", false, || client.add_reaction(&react)).await?;
    saw(
        &made.id,
        Seen::Reaction {
            subject: parent_id.to_string(),
            emoji: emoji.to_string(),
        },
    );
    Ok(true)
}

/// The page hosting `id`'s thread, and what it is called where that is known.
/// Anything that is no comment is its own host.
pub async fn thread_host(_access_token: Option<&str>, id: &str) -> (String, Option<String>) {
    HOSTS
        .with(|hosts| hosts.borrow().get(id).cloned())
        .unwrap_or_else(|| (id.to_string(), None))
}

pub async fn thread_host_id(access_token: Option<&str>, id: &str) -> String {
    thread_host(access_token, id).await.0
}

/// A feed row as the node the feed's component draws.
///
/// The interim hands it the whole node and lets it dig: a page's opening out of
/// its text, a reply's quote out of its parent, where that happened out of the
/// grandparent. The AppView says each of those outright, and they are put back
/// where the component looks for them.
fn feed_node(item: &defs::FeedItemView, profiles: &serde_json::Value) -> ChildNodeFields {
    let owner = item
        .by_did
        .as_deref()
        .map(|did| map::user_ref(profiles, did));
    let about = item.about.as_ref().map(map::parent_ref);
    let (name, data) = match item.node.as_str() {
        "comment" => (
            owner
                .as_ref()
                .map(|o| o.display_name.clone())
                .or_else(|| item.by_text.clone())
                .unwrap_or_default(),
            comment_data(&item.text, item.image.as_deref()),
        ),
        "reaction" => (item.text.clone(), serde_json::json!({ "emoji": item.text })),
        _ => {
            let mut data = serde_json::Map::new();
            if let Some(excerpt) = &item.excerpt {
                data.insert(
                    "content".to_string(),
                    serde_json::json!([{ "children": [{ "text": excerpt }] }]),
                );
            }
            if let Some(image) = &item.image {
                data.insert("image".to_string(), serde_json::json!(image));
            }
            (item.text.clone(), serde_json::Value::Object(data))
        }
    };
    // What a reply or a reaction is to, with the page it happened on above it.
    let quoted = item.quote.as_ref().map(|quote| {
        let by = quote
            .by_did
            .as_deref()
            .map(|did| map::user_ref(profiles, did));
        crate::model::ParentNodeFields {
            id: Uuid(String::new()),
            name: by
                .as_ref()
                .map(|u| u.display_name.clone())
                .or_else(|| quote.by_text.clone())
                .unwrap_or_default(),
            key: String::new(),
            mime_id: Some("vote/comment".to_string()),
            data: Some(Jsonb(serde_json::json!({ "text": quote.text }))),
            author_avatar: by.map(|u| u.avatar_url),
            parent: about.clone().map(Box::new),
        }
    });
    if let Some(about) = &item.about {
        if item.node != "document" {
            hosted(&item.id, &about.id, Some(&about.title));
        }
    }
    ChildNodeFields {
        id: Uuid(item.id.clone()),
        name,
        key: item
            .path
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or(&item.id)
            .to_string(),
        mime_id: Some(map::mime_of(&item.kind)),
        mutable: false,
        index: 0,
        created_at: Some(Timestamptz(item.created_at.clone())),
        owner_id: item.by_did.clone().map(Uuid),
        data: Some(Jsonb(data)),
        mime: None,
        is_owner: None,
        is_context_owner: None,
        author_name: owner
            .as_ref()
            .map(|o| o.display_name.clone())
            .or_else(|| item.by_text.clone()),
        author_avatar: owner.as_ref().map(|o| o.avatar_url.clone()),
        owner,
        parent: quoted.or(about),
    }
}

/// The feed: what was submitted, said and reacted to where the caller belongs,
/// or in one context, newest first.
pub async fn query_recent_nodes(
    access_token: Option<&str>,
    limit: i32,
    offset: i32,
    _user_id: &str,
    context_id: Option<&str>,
) -> Vec<ChildNodeFields> {
    let client = client(access_token);
    let params = list_recent::Params {
        context: context_id.map(str::to_string),
        limit: Some(i64::from(limit)),
        offset: Some(i64::from(offset)),
    };
    match ask("listRecent", true, || client.list_recent(&params)).await {
        Ok(feed) => feed
            .items
            .iter()
            .map(|item| feed_node(item, &feed.profiles))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// How much of the feed's head is read to find what just landed in it.
const ARRIVALS_PAGE: i32 = 30;

/// The feed rows for `ids`, which a live push named as having just landed. They
/// are at the head of the feed, so that is where they are looked for: an id that
/// is not there was an edit to something older, or is not the reader's to see.
pub async fn query_nodes_by_ids(
    access_token: Option<&str>,
    ids: &[String],
    user_id: &str,
    context_id: Option<&str>,
) -> Vec<ChildNodeFields> {
    if ids.is_empty() {
        return Vec::new();
    }
    query_recent_nodes(access_token, ARRIVALS_PAGE, 0, user_id, context_id)
        .await
        .into_iter()
        .filter(|row| ids.contains(&row.id.0))
        .collect()
}

async fn contributions(
    access_token: Option<&str>,
    params: list_contributions::Params,
) -> Vec<ChildNodeFields> {
    let client = client(access_token);
    match ask("listContributions", true, || {
        client.list_contributions(&params)
    })
    .await
    {
        Ok(theirs) => theirs
            .items
            .iter()
            .map(|item| feed_node(item, &theirs.profiles))
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub async fn query_user_contributions(
    access_token: Option<&str>,
    user_id: &str,
    limit: i32,
) -> Vec<ChildNodeFields> {
    let params = list_contributions::Params {
        did: Some(user_id.to_string()),
        limit: Some(i64::from(limit)),
        ..Default::default()
    };
    contributions(access_token, params).await
}

pub async fn query_group_contributions(
    access_token: Option<&str>,
    group_id: &str,
    limit: i32,
) -> Vec<ChildNodeFields> {
    let params = list_contributions::Params {
        context: Some(group_id.to_string()),
        limit: Some(i64::from(limit)),
        ..Default::default()
    };
    contributions(access_token, params).await
}

/// Pages and places by what they are called and what they say.
pub async fn search_nodes(
    access_token: Option<&str>,
    query: &str,
    context_id: Option<&str>,
) -> Result<Vec<NodeFields>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let client = client(access_token);
    let params = search::Params {
        q: query.to_string(),
        context: context_id.map(str::to_string),
    };
    let found = ask("search", true, || client.search(&params)).await?;
    Ok(found
        .hits
        .iter()
        .map(|hit| {
            saw_node(&hit.id, &hit.node, &hit.kind);
            map::search_hit(hit)
        })
        .collect())
}

/// What has lost its parent, for whoever runs the site.
pub async fn query_orphans(access_token: Option<&str>) -> Result<Vec<ContextNodeFields>, String> {
    let client = client(access_token);
    let astray = ask("listOrphans", true, || client.list_orphans()).await?;
    Ok(astray
        .orphans
        .iter()
        .map(|orphan| {
            match orphan.node.as_str() {
                "comment" => saw(&orphan.id, Seen::Comment),
                node => saw_node(&orphan.id, node, &orphan.kind),
            }
            ContextNodeFields {
                id: Uuid(orphan.id.clone()),
                name: orphan.text.clone(),
                key: orphan.id.clone(),
                mime_id: Some(map::mime_of(&orphan.kind)),
                // The parent that is not there, which is what makes it one.
                parent_id: Some(Uuid(orphan.parent_id.clone())),
                created_at: None,
                data: None,
            }
        })
        .collect())
}
