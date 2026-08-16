//! Bluesky/AT Protocol channel for IronClaw.
//!
//! This WASM component implements the IronClaw channel interface for
//! the AT Protocol (Bluesky) via XRPC endpoints.
//!
//! # Features
//!
//! - Polling-based message receiving via `listNotifications`
//! - Post creation with reply threading
//! - DM support via `chat.bsky.convo` namespace
//! - Session management with automatic token refresh
//! - Media attachment upload via `uploadBlob`
//! - DM access control (pairing, allowlist, open)
//!
//! # Security
//!
//! - App password is used to create sessions; raw credentials are
//!   injected by the host via HTTP header placeholders and never
//!   exposed to WASM code directly. The channel stores session
//!   tokens in workspace state for subsequent API calls.

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "wit/channel.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

// ============================================================================
// AT Protocol / Bluesky API Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct SessionResponse {
    #[serde(rename = "accessJwt")]
    access_jwt: String,
    #[serde(rename = "refreshJwt")]
    refresh_jwt: String,
    did: String,
    handle: String,
}

#[derive(Debug, Deserialize)]
struct ListNotificationsResponse {
    notifications: Vec<Notification>,
}

#[derive(Debug, Deserialize)]
struct Notification {
    uri: String,
    cid: String,
    author: NotificationAuthor,
    reason: String,
    #[serde(default)]
    record: serde_json::Value,
    #[serde(rename = "isRead")]
    #[serde(default)]
    is_read: bool,
    #[serde(rename = "indexedAt")]
    indexed_at: String,
}

#[derive(Debug, Deserialize)]
struct NotificationAuthor {
    did: String,
    handle: String,
    #[serde(rename = "displayName")]
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateRecordResponse {
    uri: String,
    cid: String,
}

#[derive(Debug, Deserialize)]
struct UploadBlobResponse {
    blob: BlobRef,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct BlobRef {
    #[serde(rename = "$type")]
    blob_type: String,
    #[serde(rename = "ref")]
    blob_ref: BlobLink,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct BlobLink {
    #[serde(rename = "$link")]
    link: String,
}

#[derive(Debug, Deserialize)]
struct ListConvosResponse {
    convos: Vec<Convo>,
}

#[derive(Debug, Deserialize)]
struct Convo {
    id: String,
    members: Vec<ConvoMember>,
}

#[derive(Debug, Deserialize)]
struct ConvoMember {
    did: String,
    handle: String,
}

#[derive(Debug, Deserialize)]
struct GetMessagesResponse {
    messages: Vec<ConvoMessage>,
}

#[derive(Debug, Deserialize)]
struct ConvoMessage {
    id: String,
    #[serde(default)]
    text: Option<String>,
    sender: ConvoMessageSender,
}

#[derive(Debug, Deserialize)]
struct ConvoMessageSender {
    did: String,
}

#[derive(Debug, Deserialize)]
struct GetConvoForMembersResponse {
    convo: Convo,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const ACCESS_JWT_PATH: &str = "state/access_jwt";
const REFRESH_JWT_PATH: &str = "state/refresh_jwt";
const BOT_DID_PATH: &str = "state/bot_did";
const BOT_HANDLE_PATH: &str = "state/bot_handle";
const DM_POLICY_PATH: &str = "state/dm_policy";
const ALLOW_FROM_PATH: &str = "state/allow_from";
const LAST_SEEN_PATH: &str = "state/last_seen";
const PDS_URL_PATH: &str = "state/pds_url";
const DM_CURSOR_PREFIX: &str = "state/dm_cursor/";

const CHANNEL_NAME: &str = "bluesky";

// ============================================================================
// Channel Metadata
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct BlueskyMessageMetadata {
    /// "post" or "dm"
    kind: String,
    /// Author DID
    author_did: String,
    /// For posts: the AT URI of the post being replied to
    #[serde(default)]
    post_uri: Option<String>,
    /// For posts: the CID of the post being replied to
    #[serde(default)]
    post_cid: Option<String>,
    /// For posts: the root post URI (thread root)
    #[serde(default)]
    root_uri: Option<String>,
    /// For posts: the root post CID
    #[serde(default)]
    root_cid: Option<String>,
    /// For DMs: the conversation ID
    #[serde(default)]
    convo_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BlueskyConfig {
    /// PDS URL (default: "https://bsky.social")
    #[serde(default = "default_pds_url")]
    pds_url: String,

    /// DM policy: "pairing" (default), "allowlist", or "open"
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender DIDs or handles from config
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Whether to respond to mentions in addition to DMs
    #[serde(default = "default_true")]
    respond_to_mentions: bool,
}

fn default_pds_url() -> String {
    "https://bsky.social".to_string()
}

fn default_true() -> bool {
    true
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct BlueskyChannel;

__export_sandboxed_channel_impl!(BlueskyChannel);

impl Guest for BlueskyChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Bluesky channel config: {}", config_json),
        );

        let config: BlueskyConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Bluesky channel starting for PDS: {}", config.pds_url),
        );

        // Persist config
        let _ = channel_host::workspace_write(PDS_URL_PATH, &config.pds_url);
        let _ = channel_host::workspace_write(
            "state/respond_to_mentions",
            if config.respond_to_mentions {
                "true"
            } else {
                "false"
            },
        );

        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing");
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Create session using app password
        create_session(&config.pds_url)?;

        Ok(ChannelConfig {
            display_name: "Bluesky".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/bluesky".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: Some(PollConfig {
                interval_ms: 30000,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Bluesky doesn't use webhooks; we rely on notification polling.
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        let pds_url = match channel_host::workspace_read(PDS_URL_PATH) {
            Some(url) => url,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No PDS URL in workspace state",
                );
                return;
            }
        };

        let bot_did = match channel_host::workspace_read(BOT_DID_PATH) {
            Some(did) => did,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No bot DID in workspace state",
                );
                return;
            }
        };

        // Ensure we have a valid session
        if ensure_session(&pds_url).is_err() {
            return;
        }

        let respond_to_mentions = channel_host::workspace_read("state/respond_to_mentions")
            .map(|v| v == "true")
            .unwrap_or(true);

        // Poll notifications (mentions and replies)
        if respond_to_mentions {
            poll_notifications(&pds_url, &bot_did);
        }

        // Poll DMs
        poll_dms(&pds_url, &bot_did);
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: BlueskyMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        let pds_url = channel_host::workspace_read(PDS_URL_PATH).ok_or("No PDS URL")?;

        if ensure_session(&pds_url).is_err() {
            return Err("Failed to ensure session".to_string());
        }

        match metadata.kind.as_str() {
            "dm" => {
                let convo_id = metadata.convo_id.ok_or("No convo_id in DM metadata")?;
                send_dm(&pds_url, &convo_id, &response.content)?;
            }
            _ => {
                send_reply_post(
                    &pds_url,
                    &response.content,
                    metadata.post_uri.as_deref(),
                    metadata.post_cid.as_deref(),
                    metadata.root_uri.as_deref(),
                    metadata.root_cid.as_deref(),
                    &response.attachments,
                )?;
            }
        }

        Ok(())
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let pds_url = channel_host::workspace_read(PDS_URL_PATH).ok_or("No PDS URL")?;

        if ensure_session(&pds_url).is_err() {
            return Err("Failed to ensure session".to_string());
        }

        // user_id is expected to be a DID; send a DM
        let convo = get_or_create_convo(&pds_url, &user_id)?;
        send_dm(&pds_url, &convo.id, &response.content)
    }

    fn on_status(update: StatusUpdate) {
        // Bluesky doesn't have typing indicators.
        // For important status updates, we can post/DM them.
        match update.status {
            StatusType::ApprovalNeeded | StatusType::AuthRequired | StatusType::AuthCompleted => {
                let msg = update.message.trim();
                if !msg.is_empty() {
                    if let Ok(metadata) =
                        serde_json::from_str::<BlueskyMessageMetadata>(&update.metadata_json)
                    {
                        let pds_url = match channel_host::workspace_read(PDS_URL_PATH) {
                            Some(url) => url,
                            None => return,
                        };
                        if ensure_session(&pds_url).is_err() {
                            return;
                        }
                        match metadata.kind.as_str() {
                            "dm" => {
                                if let Some(convo_id) = &metadata.convo_id {
                                    let _ = send_dm(&pds_url, convo_id, msg);
                                }
                            }
                            _ => {
                                let _ = send_reply_post(
                                    &pds_url,
                                    msg,
                                    metadata.post_uri.as_deref(),
                                    metadata.post_cid.as_deref(),
                                    metadata.root_uri.as_deref(),
                                    metadata.root_cid.as_deref(),
                                    &[],
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Bluesky channel shutting down",
        );
    }
}

// ============================================================================
// Session Management
// ============================================================================

fn create_session(pds_url: &str) -> Result<(), String> {
    let url = format!("{}/xrpc/com.atproto.server.createSession", pds_url);

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let body = serde_json::json!({
        "identifier": "{BLUESKY_IDENTIFIER}",
        "password": "{BLUESKY_APP_PASSWORD}"
    });

    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let resp =
        channel_host::http_request("POST", &url, &headers.to_string(), Some(&body_bytes), None)?;

    if resp.status != 200 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("createSession failed ({}): {}", resp.status, body));
    }

    let session: SessionResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse session response: {}", e))?;

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!("Session created for {} ({})", session.handle, session.did),
    );

    let _ = channel_host::workspace_write(ACCESS_JWT_PATH, &session.access_jwt);
    let _ = channel_host::workspace_write(REFRESH_JWT_PATH, &session.refresh_jwt);
    let _ = channel_host::workspace_write(BOT_DID_PATH, &session.did);
    let _ = channel_host::workspace_write(BOT_HANDLE_PATH, &session.handle);

    Ok(())
}

fn refresh_session(pds_url: &str) -> Result<(), String> {
    let refresh_jwt =
        channel_host::workspace_read(REFRESH_JWT_PATH).ok_or("No refresh token available")?;

    let url = format!("{}/xrpc/com.atproto.server.refreshSession", pds_url);

    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", refresh_jwt)
    });

    let resp = channel_host::http_request("POST", &url, &headers.to_string(), None, None)?;

    if resp.status != 200 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("refreshSession failed ({}): {}", resp.status, body));
    }

    let session: SessionResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    let _ = channel_host::workspace_write(ACCESS_JWT_PATH, &session.access_jwt);
    let _ = channel_host::workspace_write(REFRESH_JWT_PATH, &session.refresh_jwt);
    let _ = channel_host::workspace_write(BOT_DID_PATH, &session.did);
    let _ = channel_host::workspace_write(BOT_HANDLE_PATH, &session.handle);

    channel_host::log(
        channel_host::LogLevel::Debug,
        "Session refreshed successfully",
    );

    Ok(())
}

/// Ensure we have a valid access token. Try to use existing, refresh if 401.
fn ensure_session(pds_url: &str) -> Result<(), String> {
    if channel_host::workspace_read(ACCESS_JWT_PATH).is_none() {
        return create_session(pds_url);
    }
    Ok(())
}

// ============================================================================
// Notification Polling
// ============================================================================

fn poll_notifications(pds_url: &str, bot_did: &str) {
    let last_seen = channel_host::workspace_read(LAST_SEEN_PATH);

    let mut url = format!(
        "{}/xrpc/app.bsky.notification.listNotifications?limit=50",
        pds_url
    );
    if let Some(ref seen) = last_seen {
        url.push_str(&format!("&seenAt={}", url_encode(seen)));
    }

    let result = bsky_get(&url, None);

    match result {
        Ok(resp) => {
            if resp.status == 401 {
                // Token expired, try refresh
                if refresh_session(pds_url).is_ok() {
                    // Retry once
                    poll_notifications(pds_url, bot_did);
                }
                return;
            }
            if resp.status != 200 {
                let body = String::from_utf8_lossy(&resp.body);
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("listNotifications returned {}: {}", resp.status, body),
                );
                return;
            }

            let notifs: ListNotificationsResponse = match serde_json::from_slice(&resp.body) {
                Ok(n) => n,
                Err(e) => {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to parse notifications: {}", e),
                    );
                    return;
                }
            };

            // Track latest timestamp
            let mut latest_indexed_at: Option<String> = None;

            for notif in &notifs.notifications {
                // Only process mentions and replies
                if notif.reason != "mention" && notif.reason != "reply" {
                    continue;
                }

                // Skip already-read notifications on first poll
                if last_seen.is_none() && notif.is_read {
                    continue;
                }

                // Skip if we've already seen this
                if let Some(ref seen) = last_seen {
                    if notif.indexed_at <= *seen {
                        continue;
                    }
                }

                // Skip own posts
                if notif.author.did == bot_did {
                    continue;
                }

                // Extract post text
                let text = match notif.record.get("text").and_then(|t| t.as_str()) {
                    Some(t) => t.to_string(),
                    None => continue,
                };

                // Strip bot mention from text
                let bot_handle = channel_host::workspace_read(BOT_HANDLE_PATH).unwrap_or_default();
                let content = strip_mention(&text, &bot_handle);

                // Determine thread root
                let (root_uri, root_cid) = extract_reply_root(&notif.record)
                    .unwrap_or_else(|| (notif.uri.clone(), notif.cid.clone()));

                let metadata = BlueskyMessageMetadata {
                    kind: "post".to_string(),
                    author_did: notif.author.did.clone(),
                    post_uri: Some(notif.uri.clone()),
                    post_cid: Some(notif.cid.clone()),
                    root_uri: Some(root_uri),
                    root_cid: Some(root_cid),
                    convo_id: None,
                };

                let user_name = notif
                    .author
                    .display_name
                    .clone()
                    .unwrap_or_else(|| notif.author.handle.clone());

                channel_host::emit_message(&EmittedMessage {
                    user_id: notif.author.did.clone(),
                    user_name: Some(user_name),
                    content,
                    thread_id: metadata.root_uri.clone(),
                    metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
                    attachments: extract_post_attachments(&notif.record),
                });

                // Track latest
                if latest_indexed_at
                    .as_ref()
                    .map(|l| notif.indexed_at > *l)
                    .unwrap_or(true)
                {
                    latest_indexed_at = Some(notif.indexed_at.clone());
                }
            }

            // Update last seen timestamp
            if let Some(ts) = latest_indexed_at {
                let _ = channel_host::workspace_write(LAST_SEEN_PATH, &ts);
            } else if last_seen.is_none() {
                // First poll: mark current time so we don't re-process
                let now = format!("{}Z", channel_host::now_millis());
                // Use the latest notification timestamp or current time
                if let Some(first) = notifs.notifications.first() {
                    let _ = channel_host::workspace_write(LAST_SEEN_PATH, &first.indexed_at);
                } else {
                    let _ = channel_host::workspace_write(LAST_SEEN_PATH, &now);
                }
            }

            // Mark notifications as seen
            if !notifs.notifications.is_empty() {
                let seen_at = notifs
                    .notifications
                    .first()
                    .map(|n| n.indexed_at.clone())
                    .unwrap_or_default();
                let _ = update_seen(pds_url, &seen_at);
            }
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("listNotifications request failed: {}", e),
            );
        }
    }
}

fn update_seen(pds_url: &str, seen_at: &str) -> Result<(), String> {
    let url = format!("{}/xrpc/app.bsky.notification.updateSeen", pds_url);
    let body = serde_json::json!({"seenAt": seen_at});
    bsky_post(&url, &body, None)?;
    Ok(())
}

// ============================================================================
// DM Polling
// ============================================================================

fn poll_dms(pds_url: &str, bot_did: &str) {
    // List conversations
    let url = format!("{}/xrpc/chat.bsky.convo.listConvos?limit=50", pds_url);

    let result = bsky_get_chat(&url, None);

    let resp = match result {
        Ok(r) => r,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("listConvos failed: {}", e),
            );
            return;
        }
    };

    if resp.status == 401 {
        if refresh_session(pds_url).is_ok() {
            poll_dms(pds_url, bot_did);
        }
        return;
    }

    if resp.status != 200 {
        let body = String::from_utf8_lossy(&resp.body);
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("listConvos returned {}: {}", resp.status, body),
        );
        return;
    }

    let convos: ListConvosResponse = match serde_json::from_slice(&resp.body) {
        Ok(c) => c,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse listConvos: {}", e),
            );
            return;
        }
    };

    for convo in &convos.convos {
        poll_convo_messages(pds_url, bot_did, convo);
    }
}

fn poll_convo_messages(pds_url: &str, bot_did: &str, convo: &Convo) {
    let cursor_path = format!("{}{}", DM_CURSOR_PREFIX, convo.id);
    let cursor = channel_host::workspace_read(&cursor_path);

    let mut url = format!(
        "{}/xrpc/chat.bsky.convo.getMessages?convoId={}&limit=50",
        pds_url,
        url_encode(&convo.id)
    );
    if let Some(ref c) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(c)));
    }

    let resp = match bsky_get_chat(&url, None) {
        Ok(r) if r.status == 200 => r,
        Ok(r) => {
            let body = String::from_utf8_lossy(&r.body);
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "getMessages for {} returned {}: {}",
                    convo.id, r.status, body
                ),
            );
            return;
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("getMessages for {} failed: {}", convo.id, e),
            );
            return;
        }
    };

    let messages: GetMessagesResponse = match serde_json::from_slice(&resp.body) {
        Ok(m) => m,
        Err(_) => return,
    };

    // On first poll (no cursor), skip existing messages
    if cursor.is_none() {
        // Just save cursor to latest
        if let Some(last) = messages.messages.first() {
            let _ = channel_host::workspace_write(&cursor_path, &last.id);
        }
        return;
    }

    let other_member = convo.members.iter().find(|m| m.did != bot_did);

    for msg in &messages.messages {
        // Skip our own messages
        if msg.sender.did == bot_did {
            continue;
        }

        let text = match &msg.text {
            Some(t) if !t.is_empty() => t.clone(),
            _ => continue,
        };

        // DM access control
        if !check_dm_access(&msg.sender.did) {
            continue;
        }

        let metadata = BlueskyMessageMetadata {
            kind: "dm".to_string(),
            author_did: msg.sender.did.clone(),
            post_uri: None,
            post_cid: None,
            root_uri: None,
            root_cid: None,
            convo_id: Some(convo.id.clone()),
        };

        let user_name = other_member
            .filter(|m| m.did == msg.sender.did)
            .map(|m| m.handle.clone());

        channel_host::emit_message(&EmittedMessage {
            user_id: msg.sender.did.clone(),
            user_name,
            content: text,
            thread_id: Some(convo.id.clone()),
            metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
            attachments: vec![],
        });
    }

    // Save cursor to latest message
    if let Some(last) = messages.messages.first() {
        let _ = channel_host::workspace_write(&cursor_path, &last.id);
    }
}

// ============================================================================
// Sending Messages
// ============================================================================

fn send_reply_post(
    pds_url: &str,
    text: &str,
    parent_uri: Option<&str>,
    parent_cid: Option<&str>,
    root_uri: Option<&str>,
    root_cid: Option<&str>,
    attachments: &[exports::near::agent::channel::Attachment],
) -> Result<(), String> {
    let bot_did = channel_host::workspace_read(BOT_DID_PATH).ok_or("No bot DID")?;

    // Split long text into multiple posts if needed (300 char limit)
    let chunks = split_post_text(text, 300);

    let mut prev_uri: Option<String> = parent_uri.map(|s| s.to_string());
    let mut prev_cid: Option<String> = parent_cid.map(|s| s.to_string());
    let actual_root_uri = root_uri.map(|s| s.to_string());
    let actual_root_cid = root_cid.map(|s| s.to_string());

    for (i, chunk) in chunks.iter().enumerate() {
        let mut record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": chunk,
            "createdAt": iso_now(),
            "langs": ["en"]
        });

        // Add reply reference
        if let (Some(ref p_uri), Some(ref p_cid)) = (&prev_uri, &prev_cid) {
            let r_uri = actual_root_uri.as_deref().unwrap_or(p_uri);
            let r_cid = actual_root_cid.as_deref().unwrap_or(p_cid);
            record["reply"] = serde_json::json!({
                "root": {"uri": r_uri, "cid": r_cid},
                "parent": {"uri": p_uri, "cid": p_cid}
            });
        }

        // Add image embeds to the first chunk only
        if i == 0 && !attachments.is_empty() {
            if let Some(embed) = upload_attachments(pds_url, attachments)? {
                record["embed"] = embed;
            }
        }

        let body = serde_json::json!({
            "repo": bot_did,
            "collection": "app.bsky.feed.post",
            "record": record
        });

        let url = format!("{}/xrpc/com.atproto.repo.createRecord", pds_url);
        let resp = bsky_post(&url, &body, None)?;

        if resp.status == 401 {
            refresh_session(pds_url)?;
            let resp = bsky_post(&url, &body, None)?;
            if resp.status != 200 {
                let err = String::from_utf8_lossy(&resp.body);
                return Err(format!("createRecord failed ({}): {}", resp.status, err));
            }
            if let Ok(created) = serde_json::from_slice::<CreateRecordResponse>(&resp.body) {
                prev_uri = Some(created.uri);
                prev_cid = Some(created.cid);
            }
        } else if resp.status != 200 {
            let err = String::from_utf8_lossy(&resp.body);
            return Err(format!("createRecord failed ({}): {}", resp.status, err));
        } else if let Ok(created) = serde_json::from_slice::<CreateRecordResponse>(&resp.body) {
            prev_uri = Some(created.uri);
            prev_cid = Some(created.cid);
        }
    }

    Ok(())
}

fn send_dm(pds_url: &str, convo_id: &str, text: &str) -> Result<(), String> {
    let url = format!("{}/xrpc/chat.bsky.convo.sendMessage", pds_url);

    let body = serde_json::json!({
        "convoId": convo_id,
        "message": {
            "text": text
        }
    });

    let resp = bsky_post_chat(&url, &body, None)?;

    if resp.status == 401 {
        refresh_session(pds_url)?;
        let resp = bsky_post_chat(&url, &body, None)?;
        if resp.status != 200 {
            let err = String::from_utf8_lossy(&resp.body);
            return Err(format!("sendMessage failed ({}): {}", resp.status, err));
        }
    } else if resp.status != 200 {
        let err = String::from_utf8_lossy(&resp.body);
        return Err(format!("sendMessage failed ({}): {}", resp.status, err));
    }

    Ok(())
}

fn get_or_create_convo(pds_url: &str, member_did: &str) -> Result<Convo, String> {
    let url = format!(
        "{}/xrpc/chat.bsky.convo.getConvoForMembers?members={}",
        pds_url,
        url_encode(member_did)
    );

    let resp = bsky_get_chat(&url, None)?;

    if resp.status != 200 {
        let err = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "getConvoForMembers failed ({}): {}",
            resp.status, err
        ));
    }

    let result: GetConvoForMembersResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse getConvoForMembers: {}", e))?;

    Ok(result.convo)
}

// ============================================================================
// Attachment Handling
// ============================================================================

fn upload_attachments(
    pds_url: &str,
    attachments: &[exports::near::agent::channel::Attachment],
) -> Result<Option<serde_json::Value>, String> {
    let mut images = Vec::new();

    for attachment in attachments.iter().take(4) {
        let url = format!("{}/xrpc/com.atproto.repo.uploadBlob", pds_url);

        let access_jwt = channel_host::workspace_read(ACCESS_JWT_PATH).ok_or("No access token")?;

        let headers = serde_json::json!({
            "Authorization": format!("Bearer {}", access_jwt),
            "Content-Type": &attachment.mime_type
        });

        let resp = channel_host::http_request(
            "POST",
            &url,
            &headers.to_string(),
            Some(&attachment.data),
            None,
        )?;

        if resp.status != 200 {
            let err = String::from_utf8_lossy(&resp.body);
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("uploadBlob failed ({}): {}", resp.status, err),
            );
            continue;
        }

        let upload: UploadBlobResponse = serde_json::from_slice(&resp.body)
            .map_err(|e| format!("Failed to parse uploadBlob response: {}", e))?;

        if attachment.mime_type.starts_with("image/") {
            images.push(serde_json::json!({
                "alt": attachment.filename,
                "image": upload.blob
            }));
        }
    }

    if images.is_empty() {
        return Ok(None);
    }

    Ok(Some(serde_json::json!({
        "$type": "app.bsky.embed.images",
        "images": images
    })))
}

fn extract_post_attachments(record: &serde_json::Value) -> Vec<InboundAttachment> {
    let mut attachments = Vec::new();

    let embed = match record.get("embed") {
        Some(e) => e,
        None => return attachments,
    };

    let embed_type = embed.get("$type").and_then(|t| t.as_str()).unwrap_or("");

    if embed_type == "app.bsky.embed.images" {
        if let Some(images) = embed.get("images").and_then(|i| i.as_array()) {
            for image in images {
                let alt = image.get("alt").and_then(|a| a.as_str()).unwrap_or("image");
                let mime = image
                    .get("image")
                    .and_then(|i| i.get("mimeType"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("image/jpeg");
                let size = image
                    .get("image")
                    .and_then(|i| i.get("size"))
                    .and_then(|s| s.as_u64());
                let link = image
                    .get("image")
                    .and_then(|i| i.get("ref"))
                    .and_then(|r| r.get("$link"))
                    .and_then(|l| l.as_str());

                if let Some(cid) = link {
                    let bot_did = channel_host::workspace_read(BOT_DID_PATH).unwrap_or_default();
                    let pds_url = channel_host::workspace_read(PDS_URL_PATH).unwrap_or_default();

                    attachments.push(InboundAttachment {
                        id: cid.to_string(),
                        mime_type: mime.to_string(),
                        filename: Some(alt.to_string()),
                        size_bytes: size,
                        source_url: Some(format!(
                            "{}/xrpc/com.atproto.sync.getBlob?did={}&cid={}",
                            pds_url,
                            url_encode(&bot_did),
                            url_encode(cid)
                        )),
                        storage_key: None,
                        extracted_text: None,
                        extras_json: "{}".to_string(),
                    });
                }
            }
        }
    }

    attachments
}

// ============================================================================
// HTTP Helpers
// ============================================================================

fn bsky_get(url: &str, timeout_ms: Option<u32>) -> Result<channel_host::HttpResponse, String> {
    let access_jwt = channel_host::workspace_read(ACCESS_JWT_PATH).unwrap_or_default();
    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", access_jwt)
    });
    channel_host::http_request("GET", url, &headers.to_string(), None, timeout_ms)
}

fn bsky_post(
    url: &str,
    body: &serde_json::Value,
    timeout_ms: Option<u32>,
) -> Result<channel_host::HttpResponse, String> {
    let access_jwt = channel_host::workspace_read(ACCESS_JWT_PATH).unwrap_or_default();
    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", access_jwt),
        "Content-Type": "application/json"
    });
    let body_bytes =
        serde_json::to_vec(body).map_err(|e| format!("Failed to serialize body: {}", e))?;
    channel_host::http_request(
        "POST",
        url,
        &headers.to_string(),
        Some(&body_bytes),
        timeout_ms,
    )
}

fn bsky_get_chat(url: &str, timeout_ms: Option<u32>) -> Result<channel_host::HttpResponse, String> {
    let access_jwt = channel_host::workspace_read(ACCESS_JWT_PATH).unwrap_or_default();
    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", access_jwt),
        "atproto-proxy": "did:web:api.bsky.chat#bsky_chat"
    });
    channel_host::http_request("GET", url, &headers.to_string(), None, timeout_ms)
}

fn bsky_post_chat(
    url: &str,
    body: &serde_json::Value,
    timeout_ms: Option<u32>,
) -> Result<channel_host::HttpResponse, String> {
    let access_jwt = channel_host::workspace_read(ACCESS_JWT_PATH).unwrap_or_default();
    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", access_jwt),
        "Content-Type": "application/json",
        "atproto-proxy": "did:web:api.bsky.chat#bsky_chat"
    });
    let body_bytes =
        serde_json::to_vec(body).map_err(|e| format!("Failed to serialize body: {}", e))?;
    channel_host::http_request(
        "POST",
        url,
        &headers.to_string(),
        Some(&body_bytes),
        timeout_ms,
    )
}

fn json_response(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({"Content-Type": "application/json"}).to_string(),
        body: body_bytes,
    }
}

// ============================================================================
// Message Processing Helpers
// ============================================================================

fn check_dm_access(sender_did: &str) -> bool {
    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

    match dm_policy.as_str() {
        "open" => true,
        "allowlist" => is_sender_allowed(sender_did),
        _ => {
            if is_sender_allowed(sender_did) {
                return true;
            }
            match channel_host::pairing_is_allowed(CHANNEL_NAME, sender_did, None) {
                Ok(true) => true,
                Ok(false) => {
                    let meta = serde_json::json!({"did": sender_did}).to_string();
                    match channel_host::pairing_upsert_request(CHANNEL_NAME, sender_did, &meta) {
                        Ok(result) if result.created => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request created for {}: code {}",
                                    sender_did, result.code
                                ),
                            );
                        }
                        _ => {}
                    }
                    false
                }
                Err(_) => false,
            }
        }
    }
}

fn is_sender_allowed(sender_did: &str) -> bool {
    if let Some(allow_from_json) = channel_host::workspace_read(ALLOW_FROM_PATH) {
        if let Ok(allow_from) = serde_json::from_str::<Vec<String>>(&allow_from_json) {
            if allow_from.iter().any(|a| a == sender_did) {
                return true;
            }
        }
    }

    if let Ok(approved) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        if approved.iter().any(|a| a == sender_did) {
            return true;
        }
    }

    false
}

fn strip_mention(text: &str, bot_handle: &str) -> String {
    let mut result = text.to_string();
    // Strip @handle mention
    result = result.replace(&format!("@{}", bot_handle), "");
    result.trim().to_string()
}

fn extract_reply_root(record: &serde_json::Value) -> Option<(String, String)> {
    let reply = record.get("reply")?;
    let root = reply.get("root")?;
    let uri = root.get("uri")?.as_str()?;
    let cid = root.get("cid")?.as_str()?;
    Some((uri.to_string(), cid.to_string()))
}

// ============================================================================
// Post Text Splitting
// ============================================================================

/// Split text into chunks that fit within Bluesky's character limit.
/// Tries to split at paragraph or sentence boundaries.
fn split_post_text(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at paragraph boundary
        let split_at = find_split_point(remaining, max_len);
        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.trim_end().to_string());
        remaining = rest.trim_start();
    }

    chunks
}

fn find_split_point(text: &str, max_len: usize) -> usize {
    let search = &text[..max_len];

    // Try paragraph break
    if let Some(pos) = search.rfind("\n\n") {
        return pos + 1;
    }
    // Try line break
    if let Some(pos) = search.rfind('\n') {
        return pos + 1;
    }
    // Try sentence end
    if let Some(pos) = search.rfind(". ") {
        return pos + 2;
    }
    // Try space
    if let Some(pos) = search.rfind(' ') {
        return pos + 1;
    }
    // Hard split
    max_len
}

// ============================================================================
// Utility Functions
// ============================================================================

fn iso_now() -> String {
    let millis = channel_host::now_millis();
    let secs = millis / 1000;
    let ms = millis % 1000;
    // Simple ISO 8601 formatting from epoch millis
    // This is approximate but sufficient for AT Protocol timestamps
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_date(days_since_epoch);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, ms
    )
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push(char::from(HEX_CHARS[(byte >> 4) as usize]));
                result.push(char::from(HEX_CHARS[(byte & 0x0f) as usize]));
            }
        }
    }
    result
}

const HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";
