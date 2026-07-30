//! What people say about content: comments, the reactions on them, and the
//! feedback reports that come in from the app itself.

use super::*;

// --- Comments (vote/comment child nodes, nested) ---

#[derive(cynic::QueryVariables, Debug)]
pub struct CommentsVariables {
    pub where_clause: NodesBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "CommentsVariables"
)]
pub struct CommentsQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<ChildNodeFields>,
}

pub async fn query_comments(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<Vec<model::ChildNodeFields>, String> {
    use cynic::QueryBuilder;
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            is_null: None,
        }),
        mime_id: Some(StringComparisonExp {
            eq: Some("vote/comment".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let op = CommentsQuery::build(CommentsVariables { where_clause });
    let mut nodes = execute(access_token, op).await?.nodes;
    nodes.sort_by(|a, b| {
        let at = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        let bt = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        at.cmp(bt)
    });
    Ok(nodes.into_iter().map(Into::into).collect())
}

/// Post a comment (a `vote/comment` node) under `parent_id` (a post or another
/// comment for a nested reply). `author` is stored as the node name and `text`
/// as `data.text`, matching the reference comment shape.
#[allow(clippy::too_many_arguments)]
pub async fn insert_comment(
    access_token: Option<&str>,
    parent_id: &str,
    context_id: Option<&str>,
    key: &str,
    author: &str,
    text: &str,
) -> Result<bool, String> {
    let input = model::NodesInsertInput {
        name: Some(author.to_string()),
        key: Some(key.to_string()),
        mime_id: Some("vote/comment".to_string()),
        parent_id: Some(model::Uuid(parent_id.to_string())),
        context_id: context_id.map(|c| model::Uuid(c.to_string())),
        data: Some(model::Jsonb(serde_json::json!({ "text": text }))),
        mutable: Some(false),
        index: None,
        created_at: None,
    };
    insert_node(access_token, input)
        .await
        .map(|inserted| inserted.is_some())
}

/// The emoji reactions (`vote/reaction` children) on a node, in insertion order.
/// Each carries `owner_id` (who reacted) and `data.emoji`, so the UI can group
/// by emoji, count, and mark the caller's own reactions. Reuses the comment
/// query shape (both are `ChildNodeFields` children filtered by mime).
pub async fn query_reactions(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<Vec<model::ChildNodeFields>, String> {
    use cynic::QueryBuilder;
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            is_null: None,
        }),
        mime_id: Some(StringComparisonExp {
            eq: Some("vote/reaction".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let op = CommentsQuery::build(CommentsVariables { where_clause });
    let nodes = execute(access_token, op).await?.nodes;
    Ok(nodes.into_iter().map(Into::into).collect())
}

/// Add an emoji reaction (`vote/reaction` node) under `parent_id` (a comment),
/// storing the emoji in `data.emoji` and as the node name. Older contexts predate
/// the `vote/reaction` permission, so — like `create_speaker_list` — this grants
/// it for the context (scoped, member role) if missing before inserting.
pub async fn insert_reaction(
    access_token: Option<&str>,
    parent_id: &str,
    context_id: Option<&str>,
    emoji: &str,
) -> Result<bool, String> {
    if let Some(ctx) = context_id {
        let allowed = node_insert_mimes(access_token, parent_id)
            .await
            .iter()
            .any(|m| m == "vote/reaction");
        if !allowed {
            // Best-effort: seed the context's vote/reaction permission if it is
            // missing. Only an actor allowed to write the permissions table (an
            // owner) will succeed here; for a plain member in an OLD context the
            // seed is a no-op and the insert below is the real gate (new contexts
            // get the rule from the creation template). Never fatal on its own.
            let _ = execute_raw_vars(
                access_token,
                "mutation($objs: [permissions_insert_input!]!) { insertPermissions(objects: $objs) { affected_rows } }",
                serde_json::json!({ "objs": [{
                    "contextId": ctx,
                    "nodeId": ctx,
                    "mimeId": "vote/reaction",
                    "role": "member",
                    "parents": REACTION_PARENTS,
                    "active": true,
                    "insert": true,
                    "select": true,
                    "update": true,
                    "delete": true,
                }] }),
            )
            .await;
        }
    }
    let key = format!(
        "reaction-{}-{}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * 1e9) as u64
    );
    let input = model::NodesInsertInput {
        name: Some(emoji.to_string()),
        key: Some(key),
        mime_id: Some("vote/reaction".to_string()),
        parent_id: Some(model::Uuid(parent_id.to_string())),
        context_id: context_id.map(|c| model::Uuid(c.to_string())),
        data: Some(model::Jsonb(serde_json::json!({ "emoji": emoji }))),
        mutable: Some(false),
        index: None,
        created_at: None,
    };
    insert_node(access_token, input)
        .await
        .map(|inserted| inserted.is_some())
}

/// A feedback submission — a `wiki/feedback` node under the root. Which of these
/// a caller receives is gated SERVER-SIDE (the `nodes` select rule): a home-
/// context owner sees all; a plain member sees only their own.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedbackItem {
    pub id: String,
    pub kind: String,
    pub message: String,
    /// The screenshot file id (`data.image`), if one was attached.
    pub image: Option<String>,
    pub path: String,
    /// The build it was sent from. Empty on anything submitted before builds
    /// started recording it.
    pub commit: String,
    /// How many times this crash has been reported. Repeats are folded into one
    /// node by the backend, so this is the only record of how often it happens:
    /// the log sink keeps three days, this keeps everything. 1 (or 0, on a typed
    /// report, which is never folded) means it has been seen once.
    pub seen: u64,
    /// Everyone who has hit it, as user ids, accumulated by the backend. The
    /// literal string `anonymous` stands in for reporters with no account, and
    /// appears at most once.
    pub reporters: Vec<String>,
    /// When it was last seen — the node's `updatedAt`, which a database trigger
    /// moves on every fold.
    pub last_seen: String,
    pub created_at: String,
    pub owner_id: Option<String>,
    pub owner_name: String,
    pub owner_avatar: String,
}

/// Submit feedback: create a `wiki/feedback` node under the root node (its own
/// context), carrying the kind, message, optional screenshot file id, and the
/// originating path / app version / user agent. The submitter is stamped as the
/// node owner server-side; members may insert here via a root-context permission.
pub async fn insert_feedback(
    access_token: Option<&str>,
    kind: &str,
    message: &str,
    image_file_id: Option<&str>,
    path: &str,
    app_version: &str,
    user_agent: &str,
) -> Result<(), String> {
    let root_id = query_root_id(access_token)
        .await?
        .ok_or("root node not found")?;
    let mut data = serde_json::json!({
        "kind": kind,
        "message": message,
        "path": path,
        "appVersion": app_version,
        // The build this was sent from. `appVersion` is the crate version, which
        // is the same string for every build ever made; this is what actually
        // says which code the reporter was looking at.
        "commit": crate::build_info::COMMIT,
        "userAgent": user_agent,
    });
    if let Some(img) = image_file_id.filter(|i| !i.is_empty()) {
        data["image"] = serde_json::Value::from(img);
    }
    let name: String = message.trim().chars().take(80).collect();
    let name = if name.is_empty() {
        kind.to_string()
    } else {
        name
    };
    let key = format!(
        "feedback-{}-{}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * 1e9) as u64
    );
    let input = model::NodesInsertInput {
        name: Some(name),
        key: Some(key),
        mime_id: Some("wiki/feedback".to_string()),
        parent_id: Some(model::Uuid(root_id.clone())),
        context_id: Some(model::Uuid(root_id)),
        data: Some(model::Jsonb(data)),
        mutable: Some(false),
        index: None,
        created_at: None,
    };
    insert_node(access_token, input).await.map(|_| ())
}

/// The feedback the caller may see (owner: all, member: own), newest first.
pub async fn query_feedback(access_token: Option<&str>) -> Result<Vec<FeedbackItem>, String> {
    let root_id = query_root_id(access_token)
        .await?
        .ok_or("root node not found")?;
    let root_id = gql_escape(&root_id);
    let query = format!(
        "query {{ nodes(where: {{ parentId: {{ _eq: \"{root_id}\" }}, \
         mimeId: {{ _eq: \"wiki/feedback\" }} }}) \
         {{ id data createdAt updatedAt ownerId owner {{ displayName avatarUrl }} }} }}"
    );
    let data = execute_raw(access_token, &query).await?;
    let mut items: Vec<FeedbackItem> = data
        .get("nodes")
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .map(|n| {
                    let d = n.get("data");
                    let field = |k: &str| {
                        d.and_then(|d| d.get(k))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string()
                    };
                    let image = field("image");
                    let kind = field("kind");
                    FeedbackItem {
                        id: n
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        kind: if kind.is_empty() {
                            "other".to_string()
                        } else {
                            kind
                        },
                        message: field("message"),
                        image: if image.is_empty() { None } else { Some(image) },
                        path: field("path"),
                        commit: field("commit"),
                        seen: d
                            .and_then(|d| d.get("seen"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        reporters: d
                            .and_then(|d| d.get("reporters"))
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        last_seen: n
                            .get("updatedAt")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        created_at: n
                            .get("createdAt")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        owner_id: n
                            .get("ownerId")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        owner_name: n
                            .get("owner")
                            .and_then(|o| o.get("displayName"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        owner_avatar: n
                            .get("owner")
                            .and_then(|o| o.get("avatarUrl"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // Newest first (client-side; feedback volume is low).
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}

/// Resolve the path (list of keys from the root's child down to the node) for a
/// node id by walking up the parent chain. Mirrors the React `fromId` helper:
/// the root node contributes no segment.
/// The node whose page hosts `id`'s comment thread.
///
/// Neither a comment nor a reaction has a page of its own — both render inside
/// the thread on the content they hang under — so anything linking to one has to
/// link there instead. A reaction sits on a comment, and replies are comments on
/// comments (all three shapes exist in the data), so this climbs until the
/// ancestor is something that renders, bounded like [`path_from_id`]. Anything
/// else is returned unchanged, so callers may pass any node id.
pub async fn thread_host_id(access_token: Option<&str>, id: &str) -> String {
    use cynic::QueryBuilder;
    const PASS_THROUGH: [&str; 2] = ["vote/comment", "vote/reaction"];
    let mut current = id.to_string();
    for _ in 0..16 {
        let op = NodeByIdQuery::build(NodeByIdVariables {
            id: Uuid(current.clone()),
        });
        let Ok(data) = execute(access_token, op).await else {
            break;
        };
        let Some(node) = data.node else { break };
        if !node
            .mime_id
            .as_deref()
            .is_some_and(|m| PASS_THROUGH.contains(&m))
        {
            break;
        }
        match node.parent_id {
            Some(parent) => current = parent.0,
            None => break,
        }
    }
    current
}
