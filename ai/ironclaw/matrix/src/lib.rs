//! Matrix channel for IronClaw.
//!
//! This WASM component implements the IronClaw channel interface for the
//! Matrix protocol via the Client-Server API (v1.12+).
//!
//! # Features
//!
//! - Polling-based message receiving via `/sync`
//! - Room message sending with Markdown formatting
//! - Typing indicator support
//! - Thread/reply support
//! - DM access control (pairing, allowlist, open)
//! - Media attachment upload/download via content repository
//!
//! # Security
//!
//! - Access token is injected by the host during HTTP requests
//! - WASM never sees raw credentials

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
// Matrix Client-Server API Types
// ============================================================================

/// Response from `/sync`.
#[derive(Debug, Deserialize)]
struct SyncResponse {
    next_batch: String,
    #[serde(default)]
    rooms: Option<SyncRooms>,
}

#[derive(Debug, Deserialize)]
struct SyncRooms {
    #[serde(default)]
    join: Option<std::collections::HashMap<String, JoinedRoom>>,
}

#[derive(Debug, Deserialize)]
struct JoinedRoom {
    #[serde(default)]
    timeline: Option<Timeline>,
}

#[derive(Debug, Deserialize)]
struct Timeline {
    #[serde(default)]
    events: Vec<TimelineEvent>,
}

#[derive(Debug, Deserialize)]
struct TimelineEvent {
    #[serde(rename = "type")]
    event_type: String,
    event_id: Option<String>,
    sender: Option<String>,
    #[serde(default)]
    content: serde_json::Value,
    #[serde(default)]
    unsigned: Option<serde_json::Value>,
}

/// Response from `/user/{userId}/filter` (POST).
#[derive(Debug, Deserialize)]
struct FilterResponse {
    filter_id: String,
}

/// Response from content upload.
#[derive(Debug, Deserialize)]
struct UploadResponse {
    content_uri: String,
}

/// Response from `/whoami`.
#[derive(Debug, Deserialize)]
struct WhoAmIResponse {
    user_id: String,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const SYNC_TOKEN_PATH: &str = "state/sync_token";
const BOT_USER_ID_PATH: &str = "state/bot_user_id";
const DM_POLICY_PATH: &str = "state/dm_policy";
const ALLOW_FROM_PATH: &str = "state/allow_from";
const FILTER_ID_PATH: &str = "state/filter_id";

const CHANNEL_NAME: &str = "matrix";

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct MatrixMessageMetadata {
    homeserver: String,
    room_id: String,
    event_id: String,
    sender: String,
    is_direct: bool,
    #[serde(default)]
    bot_user_id: Option<String>,
}

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct MatrixConfig {
    /// Matrix homeserver base URL (e.g. "https://matrix.org").
    homeserver: String,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender IDs from config.
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Room IDs to listen in. If empty, listens in all joined rooms.
    #[serde(default)]
    room_ids: Vec<String>,

    /// Whether the bot should only respond when mentioned.
    #[serde(default)]
    require_mention: bool,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct MatrixChannel;

__export_sandboxed_channel_impl!(MatrixChannel);

impl Guest for MatrixChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Matrix channel config: {}", config_json),
        );

        let config: MatrixConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Matrix channel starting for homeserver: {}",
                config.homeserver
            ),
        );

        // Persist homeserver URL for use in other callbacks
        let _ = channel_host::workspace_write("state/homeserver", &config.homeserver);

        // Persist room filter
        let rooms_json =
            serde_json::to_string(&config.room_ids).unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write("state/room_ids", &rooms_json);
        let _ = channel_host::workspace_write(
            "state/require_mention",
            if config.require_mention {
                "true"
            } else {
                "false"
            },
        );

        // Persist DM policy
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing");
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Discover bot's own user ID via /whoami
        let whoami_url = format!("{}/_matrix/client/v3/account/whoami", config.homeserver);
        match matrix_get(&whoami_url, None) {
            Ok(resp) if resp.status == 200 => {
                if let Ok(whoami) = serde_json::from_slice::<WhoAmIResponse>(&resp.body) {
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        &format!("Bot user ID: {}", whoami.user_id),
                    );
                    let _ = channel_host::workspace_write(BOT_USER_ID_PATH, &whoami.user_id);
                }
            }
            Ok(resp) => {
                let body = String::from_utf8_lossy(&resp.body);
                return Err(format!(
                    "Failed to verify access token (/whoami returned {}): {}",
                    resp.status, body
                ));
            }
            Err(e) => return Err(format!("Failed to call /whoami: {}", e)),
        }

        // Create a server-side filter to only receive message events
        let bot_user_id = channel_host::workspace_read(BOT_USER_ID_PATH).unwrap_or_default();
        let filter = create_sync_filter(&config.homeserver, &bot_user_id, &config.room_ids);
        if let Some(fid) = filter {
            let _ = channel_host::workspace_write(FILTER_ID_PATH, &fid);
        }

        Ok(ChannelConfig {
            display_name: "Matrix".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/matrix".to_string(),
                methods: vec!["PUT".to_string()],
                require_secret: false,
            }],
            poll: Some(PollConfig {
                interval_ms: 5000,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Matrix doesn't use webhooks; we rely on /sync polling.
        // This endpoint exists for potential Application Service (appservice) use.
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        let homeserver = match channel_host::workspace_read("state/homeserver") {
            Some(hs) => hs,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No homeserver URL in workspace state",
                );
                return;
            }
        };

        let since = channel_host::workspace_read(SYNC_TOKEN_PATH);
        let filter_id = channel_host::workspace_read(FILTER_ID_PATH);
        let bot_user_id = channel_host::workspace_read(BOT_USER_ID_PATH).unwrap_or_default();

        // Build /sync URL
        let mut sync_url = format!("{}/_matrix/client/v3/sync?timeout=5000", homeserver);
        if let Some(ref token) = since {
            sync_url.push_str(&format!("&since={}", url_encode(token)));
        }
        if let Some(ref fid) = filter_id {
            sync_url.push_str(&format!("&filter={}", url_encode(fid)));
        }
        // On first sync, don't fetch full history
        if since.is_none() {
            sync_url.push_str("&full_state=false");
        }

        let result = matrix_get(&sync_url, Some(10_000));

        match result {
            Ok(response) => {
                if response.status != 200 {
                    let body_str = String::from_utf8_lossy(&response.body);
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("/sync returned {}: {}", response.status, body_str),
                    );
                    return;
                }

                let sync: SyncResponse = match serde_json::from_slice(&response.body) {
                    Ok(s) => s,
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Failed to parse /sync response: {}", e),
                        );
                        return;
                    }
                };

                // Save next_batch for incremental sync
                if let Err(e) = channel_host::workspace_write(SYNC_TOKEN_PATH, &sync.next_batch) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to save sync token: {}", e),
                    );
                }

                // On first sync (no since token), skip processing events to avoid
                // replaying old messages.
                if since.is_none() {
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        "Initial sync complete, skipping historical events",
                    );
                    return;
                }

                // Process room events
                let rooms = match sync.rooms.and_then(|r| r.join) {
                    Some(r) => r,
                    None => return,
                };

                let room_filter = read_room_filter();
                let require_mention = channel_host::workspace_read("state/require_mention")
                    .map(|v| v == "true")
                    .unwrap_or(false);

                for (room_id, room) in rooms {
                    // If room filter is set, skip rooms not in the list
                    if !room_filter.is_empty() && !room_filter.contains(&room_id) {
                        continue;
                    }

                    let events = match room.timeline.map(|t| t.events) {
                        Some(e) => e,
                        None => continue,
                    };

                    for event in events {
                        if event.event_type != "m.room.message" {
                            continue;
                        }

                        // Skip our own messages
                        let sender = match &event.sender {
                            Some(s) if s != &bot_user_id => s.clone(),
                            _ => continue,
                        };

                        // Skip echo/redacted messages
                        if event
                            .unsigned
                            .as_ref()
                            .and_then(|u| u.get("transaction_id"))
                            .is_some()
                        {
                            continue;
                        }

                        let event_id = match &event.event_id {
                            Some(id) => id.clone(),
                            None => continue,
                        };

                        // Extract message body
                        let body = match event.content.get("body").and_then(|b| b.as_str()) {
                            Some(b) => b.to_string(),
                            None => continue,
                        };

                        // Check mention requirement
                        if require_mention && !message_mentions_bot(&body, &bot_user_id) {
                            continue;
                        }

                        // Strip bot mention from message
                        let content = strip_mention(&body, &bot_user_id);

                        // DM access control
                        let is_direct = is_direct_message(&room_id);
                        if is_direct && !check_dm_access(&sender) {
                            continue;
                        }

                        let metadata = MatrixMessageMetadata {
                            homeserver: homeserver.clone(),
                            room_id: room_id.clone(),
                            event_id: event_id.clone(),
                            sender: sender.clone(),
                            is_direct,
                            bot_user_id: Some(bot_user_id.clone()),
                        };

                        // Collect attachments
                        let attachments = extract_attachments(&event.content, &homeserver);

                        let user_name = sender
                            .strip_prefix('@')
                            .and_then(|s| s.split(':').next())
                            .map(|s| s.to_string());

                        channel_host::emit_message(&EmittedMessage {
                            user_id: sender,
                            user_name,
                            content,
                            thread_id: event
                                .content
                                .get("m.relates_to")
                                .and_then(|r| r.get("event_id"))
                                .and_then(|e| e.as_str())
                                .map(|s| s.to_string()),
                            metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
                            attachments,
                        });
                    }
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("/sync request failed: {}", e),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: MatrixMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        send_room_message(
            &metadata.homeserver,
            &metadata.room_id,
            &response.content,
            Some(&metadata.event_id),
            &response.attachments,
        )
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let homeserver =
            channel_host::workspace_read("state/homeserver").ok_or("No homeserver URL")?;
        // user_id is expected to be a room_id for Matrix
        send_room_message(
            &homeserver,
            &user_id,
            &response.content,
            None,
            &response.attachments,
        )
    }

    fn on_status(update: StatusUpdate) {
        let metadata: MatrixMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(m) => m,
            Err(_) => return,
        };

        let bot_user_id = metadata.bot_user_id.as_deref().unwrap_or("");

        match update.status {
            StatusType::Thinking => {
                send_typing(&metadata.homeserver, &metadata.room_id, bot_user_id, true);
            }
            StatusType::Done | StatusType::Interrupted => {
                send_typing(&metadata.homeserver, &metadata.room_id, bot_user_id, false);
            }
            StatusType::ApprovalNeeded
            | StatusType::JobStarted
            | StatusType::AuthRequired
            | StatusType::AuthCompleted => {
                let msg = update.message.trim();
                if !msg.is_empty() {
                    let _ =
                        send_room_message(&metadata.homeserver, &metadata.room_id, msg, None, &[]);
                }
            }
            _ => {}
        }
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Matrix channel shutting down");
    }
}

// ============================================================================
// HTTP Helpers
// ============================================================================

fn matrix_get(url: &str, timeout_ms: Option<u32>) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {MATRIX_ACCESS_TOKEN}"
    });
    channel_host::http_request("GET", url, &headers.to_string(), None, timeout_ms)
}

fn matrix_put(url: &str, body: &serde_json::Value) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {MATRIX_ACCESS_TOKEN}",
        "Content-Type": "application/json"
    });
    let body_bytes =
        serde_json::to_vec(body).map_err(|e| format!("Failed to serialize body: {}", e))?;
    channel_host::http_request("PUT", url, &headers.to_string(), Some(&body_bytes), None)
}

fn matrix_post(
    url: &str,
    body: &serde_json::Value,
    timeout_ms: Option<u32>,
) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {MATRIX_ACCESS_TOKEN}",
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

fn json_response(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({"Content-Type": "application/json"}).to_string(),
        body: body_bytes,
    }
}

// ============================================================================
// Matrix API Helpers
// ============================================================================

fn create_sync_filter(homeserver: &str, user_id: &str, room_ids: &[String]) -> Option<String> {
    let mut room_filter = serde_json::json!({
        "timeline": {
            "types": ["m.room.message"],
            "limit": 50
        },
        "state": {
            "types": ["m.room.member"],
            "lazy_load_members": true
        },
        "ephemeral": {
            "types": []
        }
    });

    if !room_ids.is_empty() {
        room_filter["rooms"] = serde_json::json!(room_ids);
    }

    let filter = serde_json::json!({
        "room": room_filter,
        "presence": {
            "types": []
        },
        "account_data": {
            "types": []
        }
    });

    let url = format!(
        "{}/_matrix/client/v3/user/{}/filter",
        homeserver,
        url_encode(user_id)
    );

    match matrix_post(&url, &filter, None) {
        Ok(resp) if resp.status == 200 => serde_json::from_slice::<FilterResponse>(&resp.body)
            .ok()
            .map(|f| f.filter_id),
        Ok(resp) => {
            let body = String::from_utf8_lossy(&resp.body);
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to create sync filter ({}): {}", resp.status, body),
            );
            None
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to create sync filter: {}", e),
            );
            None
        }
    }
}

fn send_room_message(
    homeserver: &str,
    room_id: &str,
    text: &str,
    reply_to_event_id: Option<&str>,
    attachments: &[exports::near::agent::channel::Attachment],
) -> Result<(), String> {
    // Upload and send any attachments first
    for attachment in attachments {
        upload_and_send_attachment(homeserver, room_id, attachment, reply_to_event_id)?;
    }

    // Send text message
    if text.is_empty() && !attachments.is_empty() {
        return Ok(());
    }

    let txn_id = generate_txn_id();
    let url = format!(
        "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
        homeserver,
        url_encode(room_id),
        url_encode(&txn_id)
    );

    let mut body = serde_json::json!({
        "msgtype": "m.text",
        "body": text,
        "format": "org.matrix.custom.html",
        "formatted_body": markdown_to_html(text)
    });

    if let Some(event_id) = reply_to_event_id {
        body["m.relates_to"] = serde_json::json!({
            "m.in_reply_to": {
                "event_id": event_id
            }
        });
    }

    match matrix_put(&url, &body) {
        Ok(resp) if resp.status == 200 => Ok(()),
        Ok(resp) => {
            let err_body = String::from_utf8_lossy(&resp.body);
            // Retry without HTML formatting if the server rejects it
            if resp.status == 400 {
                let plain_body = serde_json::json!({
                    "msgtype": "m.text",
                    "body": text
                });
                match matrix_put(&url, &plain_body) {
                    Ok(r) if r.status == 200 => Ok(()),
                    Ok(r) => Err(format!(
                        "sendMessage returned {}: {}",
                        r.status,
                        String::from_utf8_lossy(&r.body)
                    )),
                    Err(e) => Err(e),
                }
            } else {
                Err(format!(
                    "sendMessage returned {}: {}",
                    resp.status, err_body
                ))
            }
        }
        Err(e) => Err(format!("sendMessage failed: {}", e)),
    }
}

fn upload_and_send_attachment(
    homeserver: &str,
    room_id: &str,
    attachment: &exports::near::agent::channel::Attachment,
    reply_to_event_id: Option<&str>,
) -> Result<(), String> {
    // Upload to content repository
    let upload_url = format!(
        "{}/_matrix/media/v3/upload?filename={}",
        homeserver,
        url_encode(&attachment.filename)
    );

    let headers = serde_json::json!({
        "Authorization": "Bearer {MATRIX_ACCESS_TOKEN}",
        "Content-Type": attachment.mime_type
    });

    let resp = channel_host::http_request(
        "POST",
        &upload_url,
        &headers.to_string(),
        Some(&attachment.data),
        None,
    )?;

    if resp.status != 200 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Upload failed ({}): {}", resp.status, body));
    }

    let upload: UploadResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse upload response: {}", e))?;

    // Determine message type from MIME
    let msgtype = if attachment.mime_type.starts_with("image/") {
        "m.image"
    } else if attachment.mime_type.starts_with("audio/") {
        "m.audio"
    } else if attachment.mime_type.starts_with("video/") {
        "m.video"
    } else {
        "m.file"
    };

    let txn_id = generate_txn_id();
    let url = format!(
        "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
        homeserver,
        url_encode(room_id),
        url_encode(&txn_id)
    );

    let mut body = serde_json::json!({
        "msgtype": msgtype,
        "body": attachment.filename,
        "url": upload.content_uri,
        "info": {
            "mimetype": attachment.mime_type,
            "size": attachment.data.len()
        }
    });

    if let Some(event_id) = reply_to_event_id {
        body["m.relates_to"] = serde_json::json!({
            "m.in_reply_to": {
                "event_id": event_id
            }
        });
    }

    match matrix_put(&url, &body) {
        Ok(resp) if resp.status == 200 => Ok(()),
        Ok(resp) => {
            let err = String::from_utf8_lossy(&resp.body);
            Err(format!("Send attachment failed ({}): {}", resp.status, err))
        }
        Err(e) => Err(e),
    }
}

fn send_typing(homeserver: &str, room_id: &str, bot_user_id: &str, typing: bool) {
    if bot_user_id.is_empty() {
        return;
    }

    let url = format!(
        "{}/_matrix/client/v3/rooms/{}/typing/{}",
        homeserver,
        url_encode(room_id),
        url_encode(bot_user_id)
    );

    let body = serde_json::json!({
        "typing": typing,
        "timeout": 30000
    });

    if let Err(e) = matrix_put(&url, &body) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("typing notification failed: {}", e),
        );
    }
}

// ============================================================================
// Message Processing Helpers
// ============================================================================

fn check_dm_access(sender: &str) -> bool {
    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

    match dm_policy.as_str() {
        "open" => true,
        "allowlist" => is_sender_allowed(sender),
        _ => {
            if is_sender_allowed(sender) {
                return true;
            }
            // Check pairing store
            match channel_host::pairing_is_allowed(
                CHANNEL_NAME,
                sender,
                extract_localpart(sender).as_deref(),
            ) {
                Ok(true) => true,
                Ok(false) => {
                    // Upsert a pairing request
                    let meta = serde_json::json!({"user_id": sender}).to_string();
                    match channel_host::pairing_upsert_request(CHANNEL_NAME, sender, &meta) {
                        Ok(result) if result.created => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request created for {}: code {}",
                                    sender, result.code
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

fn is_sender_allowed(sender: &str) -> bool {
    // Check config allowlist
    if let Some(allow_from_json) = channel_host::workspace_read(ALLOW_FROM_PATH) {
        if let Ok(allow_from) = serde_json::from_str::<Vec<String>>(&allow_from_json) {
            if allow_from.iter().any(|a| a == sender) {
                return true;
            }
            // Check localpart match
            if let Some(localpart) = extract_localpart(sender) {
                if allow_from.contains(&localpart) {
                    return true;
                }
            }
        }
    }

    // Check pairing-approved store
    if let Ok(approved) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        if approved.iter().any(|a| a == sender) {
            return true;
        }
    }

    false
}

fn extract_localpart(user_id: &str) -> Option<String> {
    user_id
        .strip_prefix('@')
        .and_then(|s| s.split(':').next())
        .map(|s| s.to_string())
}

fn message_mentions_bot(body: &str, bot_user_id: &str) -> bool {
    body.contains(bot_user_id)
        || extract_localpart(bot_user_id)
            .map(|lp| body.to_lowercase().contains(&lp.to_lowercase()))
            .unwrap_or(false)
}

fn strip_mention(body: &str, bot_user_id: &str) -> String {
    let mut result = body.replace(bot_user_id, "");
    if let Some(localpart) = extract_localpart(bot_user_id) {
        // Strip @localpart and localpart: style mentions
        result = result.replace(&format!("@{}", localpart), "");
        result = result.replace(&format!("{}:", localpart), "");
    }
    result.trim().to_string()
}

fn is_direct_message(room_id: &str) -> bool {
    // Heuristic: check room member count via state.
    // For simplicity, assume rooms with "!" prefix and known DM patterns.
    // A more robust approach would check m.direct account data,
    // but that requires additional API calls.
    let homeserver = match channel_host::workspace_read("state/homeserver") {
        Some(hs) => hs,
        None => return false,
    };

    let url = format!(
        "{}/_matrix/client/v3/rooms/{}/joined_members",
        homeserver,
        url_encode(room_id)
    );

    match matrix_get(&url, None) {
        Ok(resp) if resp.status == 200 => {
            // Count members - DM if exactly 2
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                if let Some(joined) = val.get("joined").and_then(|j| j.as_object()) {
                    return joined.len() == 2;
                }
            }
            false
        }
        _ => false,
    }
}

fn extract_attachments(content: &serde_json::Value, homeserver: &str) -> Vec<InboundAttachment> {
    let msgtype = match content.get("msgtype").and_then(|m| m.as_str()) {
        Some(t) => t,
        None => return vec![],
    };

    let mxc_url = match content.get("url").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => return vec![],
    };

    // Only process media message types
    if !matches!(msgtype, "m.image" | "m.audio" | "m.video" | "m.file") {
        return vec![];
    }

    let info = content.get("info");
    let mime_type = info
        .and_then(|i| i.get("mimetype"))
        .and_then(|m| m.as_str())
        .unwrap_or("application/octet-stream");
    let size = info.and_then(|i| i.get("size")).and_then(|s| s.as_u64());
    let filename = content
        .get("filename")
        .or_else(|| content.get("body"))
        .and_then(|f| f.as_str())
        .map(|s| s.to_string());

    // Convert mxc:// URL to HTTP download URL
    let download_url = mxc_to_http(mxc_url, homeserver);

    vec![InboundAttachment {
        id: mxc_url.to_string(),
        mime_type: mime_type.to_string(),
        filename,
        size_bytes: size,
        source_url: download_url,
        storage_key: None,
        extracted_text: None,
        extras_json: "{}".to_string(),
    }]
}

fn mxc_to_http(mxc_url: &str, homeserver: &str) -> Option<String> {
    // mxc://server/media_id -> /_matrix/media/v3/download/server/media_id
    mxc_url
        .strip_prefix("mxc://")
        .map(|path| format!("{}/_matrix/media/v3/download/{}", homeserver, path))
}

fn read_room_filter() -> Vec<String> {
    channel_host::workspace_read("state/room_ids")
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
}

// ============================================================================
// Utility Functions
// ============================================================================

fn generate_txn_id() -> String {
    let millis = channel_host::now_millis();
    format!("ic_{}", millis)
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

/// Minimal Markdown to HTML conversion.
fn markdown_to_html(text: &str) -> String {
    let mut html = String::with_capacity(text.len() * 2);
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>", escape_html(heading)));
        } else if let Some(heading) = line.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>", escape_html(heading)));
        } else if let Some(heading) = line.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>", escape_html(heading)));
        } else {
            html.push_str(&inline_markdown(line));
            html.push_str("<br/>");
        }
    }
    html
}

fn inline_markdown(text: &str) -> String {
    let escaped = escape_html(text);
    // Bold
    let result = escaped
        .replace("**", "\x01")
        .split('\x01')
        .enumerate()
        .map(|(i, s)| {
            if i % 2 == 1 {
                format!("<strong>{}</strong>", s)
            } else {
                s.to_string()
            }
        })
        .collect::<String>();
    // Inline code
    result
        .replace('`', "\x02")
        .split('\x02')
        .enumerate()
        .map(|(i, s)| {
            if i % 2 == 1 {
                format!("<code>{}</code>", s)
            } else {
                s.to_string()
            }
        })
        .collect()
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
