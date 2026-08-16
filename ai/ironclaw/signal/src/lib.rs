//! Signal channel for IronClaw.
//!
//! This WASM component implements the IronClaw channel interface for
//! Signal via the signal-cli REST API.
//!
//! # Features
//!
//! - Polling-based message receiving via `/v1/receive`
//! - Message sending to individuals and groups
//! - Typing indicator support
//! - Attachment upload/download
//! - DM access control (pairing, allowlist, open)
//!
//! # Security
//!
//! - API token (if configured) is injected by the host via HTTP headers
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
// signal-cli REST API Types
// ============================================================================

/// Envelope from `/v1/receive`.
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    source: Option<String>,
    #[serde(rename = "sourceNumber", default)]
    source_number: Option<String>,
    #[serde(rename = "sourceName", default)]
    source_name: Option<String>,
    #[serde(rename = "dataMessage", default)]
    data_message: Option<DataMessage>,
}

#[derive(Debug, Deserialize)]
struct DataMessage {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    timestamp: Option<u64>,
    #[serde(rename = "groupInfo", default)]
    group_info: Option<GroupInfo>,
    #[serde(default)]
    attachments: Option<Vec<SignalAttachment>>,
    #[serde(default)]
    quote: Option<Quote>,
}

#[derive(Debug, Deserialize)]
struct GroupInfo {
    #[serde(rename = "groupId")]
    group_id: String,
}

#[derive(Debug, Deserialize)]
struct SignalAttachment {
    #[serde(rename = "contentType", default)]
    content_type: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Quote {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    _author: Option<String>,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const DM_POLICY_PATH: &str = "state/dm_policy";
const ALLOW_FROM_PATH: &str = "state/allow_from";
const CHANNEL_NAME: &str = "signal";

// ============================================================================
// Channel Metadata
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct SignalMessageMetadata {
    api_url: String,
    phone_number: String,
    sender: String,
    #[serde(default)]
    sender_name: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    timestamp: Option<u64>,
    is_group: bool,
}

#[derive(Debug, Deserialize)]
struct SignalConfig {
    /// signal-cli REST API base URL (e.g. "http://localhost:8080").
    api_url: String,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender phone numbers from config.
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Group IDs to listen in. If empty, listens in all groups.
    #[serde(default)]
    group_ids: Vec<String>,

    /// Whether the bot should only respond when mentioned in groups.
    #[serde(default)]
    require_mention: bool,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct SignalChannel;

__export_sandboxed_channel_impl!(SignalChannel);

impl Guest for SignalChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Signal channel config: {}", config_json),
        );

        let config: SignalConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Signal channel starting with API at: {}", config.api_url),
        );

        // Persist config for use in other callbacks
        let _ = channel_host::workspace_write("state/api_url", &config.api_url);

        // Read phone number from secrets
        let phone_number = channel_host::workspace_read("state/phone_number")
            .or_else(|| {
                // Try to get from secret
                if channel_host::secret_exists("signal_phone_number") {
                    // Phone number is passed as config, not a secret we can read directly.
                    // It should be set via the config or workspace state.
                    None
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if phone_number.is_empty() {
            // Check if phone_number is in config_json directly
            if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&config_json) {
                if let Some(phone_num) = raw.get("phone_number").and_then(|v| v.as_str()) {
                    let _ = channel_host::workspace_write("state/phone_number", phone_num);
                } else {
                    return Err(
                        "No phone number configured. Set 'phone_number' in channel config or workspace state.".to_string(),
                    );
                }
            }
        }

        // Persist group filter
        let groups_json =
            serde_json::to_string(&config.group_ids).unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write("state/group_ids", &groups_json);
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

        // Verify connectivity
        let about_url = format!("{}/v1/about", config.api_url);
        match signal_get(&about_url, None) {
            Ok(resp) if resp.status == 200 => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "signal-cli REST API is reachable",
                );
            }
            Ok(resp) => {
                let body = String::from_utf8_lossy(&resp.body);
                return Err(format!(
                    "signal-cli REST API returned {}: {}",
                    resp.status, body
                ));
            }
            Err(e) => return Err(format!("Failed to reach signal-cli REST API: {}", e)),
        }

        Ok(ChannelConfig {
            display_name: "Signal".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/signal".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: Some(PollConfig {
                interval_ms: 5000,
                enabled: true,
            }),
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Handle webhook callbacks from signal-cli-rest-api (if configured)
        if req.method == "POST" {
            if let Ok(envelopes) = serde_json::from_slice::<Vec<serde_json::Value>>(&req.body) {
                let api_url = channel_host::workspace_read("state/api_url").unwrap_or_default();
                let phone_number =
                    channel_host::workspace_read("state/phone_number").unwrap_or_default();

                for envelope_val in envelopes {
                    if let Ok(envelope) = serde_json::from_value::<Envelope>(envelope_val) {
                        process_envelope(&envelope, &api_url, &phone_number);
                    }
                }
            }
        }

        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        let api_url = match channel_host::workspace_read("state/api_url") {
            Some(url) => url,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No API URL in workspace state",
                );
                return;
            }
        };

        let phone_number = match channel_host::workspace_read("state/phone_number") {
            Some(phone) => phone,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No phone number in workspace state",
                );
                return;
            }
        };

        // Receive pending messages
        let receive_url = format!("{}/v1/receive/{}", api_url, url_encode(&phone_number));

        let result = signal_get(&receive_url, Some(10_000));

        match result {
            Ok(response) => {
                if response.status != 200 {
                    let body_str = String::from_utf8_lossy(&response.body);
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("/v1/receive returned {}: {}", response.status, body_str),
                    );
                    return;
                }

                // Response is an array of envelopes
                let envelopes: Vec<Envelope> = match serde_json::from_slice(&response.body) {
                    Ok(e) => e,
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Failed to parse /v1/receive response: {}", e),
                        );
                        return;
                    }
                };

                for envelope in &envelopes {
                    process_envelope(envelope, &api_url, &phone_number);
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("/v1/receive request failed: {}", e),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: SignalMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        send_message(
            &metadata.api_url,
            &metadata.phone_number,
            metadata.group_id.as_deref(),
            Some(&metadata.sender),
            &response.content,
            metadata.timestamp,
            &response.attachments,
        )
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let api_url = channel_host::workspace_read("state/api_url").ok_or("No API URL")?;
        let phone_number =
            channel_host::workspace_read("state/phone_number").ok_or("No phone number")?;

        // user_id is the recipient phone number or group ID
        let is_group = !user_id.starts_with('+');
        send_message(
            &api_url,
            &phone_number,
            if is_group { Some(&user_id) } else { None },
            if is_group { None } else { Some(&user_id) },
            &response.content,
            None,
            &response.attachments,
        )
    }

    fn on_status(update: StatusUpdate) {
        let metadata: SignalMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(m) => m,
            Err(_) => return,
        };

        match update.status {
            StatusType::Thinking => {
                send_typing(&metadata);
            }
            StatusType::ApprovalNeeded
            | StatusType::JobStarted
            | StatusType::AuthRequired
            | StatusType::AuthCompleted => {
                let msg = update.message.trim();
                if !msg.is_empty() {
                    let _ = send_message(
                        &metadata.api_url,
                        &metadata.phone_number,
                        metadata.group_id.as_deref(),
                        Some(&metadata.sender),
                        msg,
                        None,
                        &[],
                    );
                }
            }
            _ => {}
        }
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Signal channel shutting down");
    }
}

// ============================================================================
// Message Processing
// ============================================================================

fn process_envelope(envelope: &Envelope, api_url: &str, phone_number: &str) {
    let data_message = match &envelope.data_message {
        Some(dm) => dm,
        None => return,
    };

    let message = match &data_message.message {
        Some(m) if !m.is_empty() => m.clone(),
        _ => return,
    };

    let sender = envelope
        .source_number
        .as_deref()
        .or(envelope.source.as_deref())
        .unwrap_or("")
        .to_string();

    if sender.is_empty() || sender == phone_number {
        return;
    }

    let is_group = data_message.group_info.is_some();
    let group_id = data_message.group_info.as_ref().map(|g| g.group_id.clone());

    // Group filter
    if is_group {
        let group_filter = read_group_filter();
        if let Some(ref gid) = group_id {
            if !group_filter.is_empty() && !group_filter.contains(gid) {
                return;
            }
        }

        // Check mention requirement for groups
        let require_mention = channel_host::workspace_read("state/require_mention")
            .map(|v| v == "true")
            .unwrap_or(false);
        if require_mention && !message_mentions_bot(&message, phone_number) {
            return;
        }
    }

    // DM access control
    if !is_group && !check_dm_access(&sender) {
        return;
    }

    let content = strip_mention(&message, phone_number);

    let metadata = SignalMessageMetadata {
        api_url: api_url.to_string(),
        phone_number: phone_number.to_string(),
        sender: sender.clone(),
        sender_name: envelope.source_name.clone(),
        group_id,
        timestamp: data_message.timestamp,
        is_group,
    };

    // Thread ID: use quote timestamp if replying, otherwise message timestamp
    let thread_id = data_message
        .quote
        .as_ref()
        .and_then(|q| q.id)
        .or(data_message.timestamp)
        .map(|t| t.to_string());

    // Collect attachments
    let attachments = extract_attachments(data_message, api_url, phone_number);

    let user_name = envelope
        .source_name
        .clone()
        .or_else(|| sender.strip_prefix('+').map(|s| s.to_string()));

    channel_host::emit_message(&EmittedMessage {
        user_id: sender,
        user_name,
        content,
        thread_id,
        metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
        attachments,
    });
}

// ============================================================================
// HTTP Helpers
// ============================================================================

fn signal_get(url: &str, timeout_ms: Option<u32>) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {SIGNAL_API_TOKEN}"
    });
    channel_host::http_request("GET", url, &headers.to_string(), None, timeout_ms)
}

fn signal_post(
    url: &str,
    body: &serde_json::Value,
    timeout_ms: Option<u32>,
) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {SIGNAL_API_TOKEN}",
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
// Signal API Helpers
// ============================================================================

fn send_message(
    api_url: &str,
    phone_number: &str,
    group_id: Option<&str>,
    recipient: Option<&str>,
    text: &str,
    quote_timestamp: Option<u64>,
    attachments: &[exports::near::agent::channel::Attachment],
) -> Result<(), String> {
    // Upload attachments first
    let mut attachment_ids = Vec::new();
    for attachment in attachments {
        if let Ok(att_id) = upload_attachment(api_url, phone_number, attachment) {
            attachment_ids.push(att_id);
        }
    }

    let url = format!("{}/v2/send", api_url);

    let mut body = serde_json::json!({
        "message": text,
        "number": phone_number,
        "text_mode": "normal",
    });

    if let Some(gid) = group_id {
        body["recipients"] = serde_json::json!([]);
        body["group_id"] = serde_json::json!(gid);
    } else if let Some(recip) = recipient {
        body["recipients"] = serde_json::json!([recip]);
    }

    if let Some(ts) = quote_timestamp {
        body["quote_timestamp"] = serde_json::json!(ts);
    }

    if !attachment_ids.is_empty() {
        body["base64_attachments"] = serde_json::json!(attachment_ids);
    }

    match signal_post(&url, &body, None) {
        Ok(resp) if resp.status == 200 || resp.status == 201 => Ok(()),
        Ok(resp) => {
            let err_body = String::from_utf8_lossy(&resp.body);
            Err(format!(
                "send message returned {}: {}",
                resp.status, err_body
            ))
        }
        Err(e) => Err(format!("send message failed: {}", e)),
    }
}

fn upload_attachment(
    api_url: &str,
    _phone_number: &str,
    attachment: &exports::near::agent::channel::Attachment,
) -> Result<String, String> {
    // signal-cli-rest-api v2 accepts base64-encoded attachments inline
    let b64 = base64_encode(&attachment.data);
    let data_uri = format!(
        "data:{};filename={};base64,{}",
        attachment.mime_type, attachment.filename, b64
    );

    // For v2/send API, we return the data URI to be included in base64_attachments
    let _ = api_url; // api_url used by caller
    Ok(data_uri)
}

fn send_typing(metadata: &SignalMessageMetadata) {
    let url = format!(
        "{}/v1/typing-indicator/{}",
        metadata.api_url,
        url_encode(&metadata.phone_number)
    );

    let mut body = serde_json::json!({
        "recipient": metadata.sender,
    });

    if let Some(ref gid) = metadata.group_id {
        body["group_id"] = serde_json::json!(gid);
    }

    if let Err(e) = signal_post(&url, &body, None) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("typing indicator failed: {}", e),
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
            match channel_host::pairing_is_allowed(CHANNEL_NAME, sender, None) {
                Ok(true) => true,
                Ok(false) => {
                    let meta = serde_json::json!({"phone_number": sender}).to_string();
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
    if let Some(allow_from_json) = channel_host::workspace_read(ALLOW_FROM_PATH) {
        if let Ok(allow_from) = serde_json::from_str::<Vec<String>>(&allow_from_json) {
            if allow_from.iter().any(|a| a == sender) {
                return true;
            }
        }
    }

    if let Ok(approved) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        if approved.iter().any(|a| a == sender) {
            return true;
        }
    }

    false
}

fn message_mentions_bot(body: &str, phone_number: &str) -> bool {
    body.contains(phone_number)
}

fn strip_mention(body: &str, phone_number: &str) -> String {
    body.replace(phone_number, "").trim().to_string()
}

fn extract_attachments(
    data_message: &DataMessage,
    api_url: &str,
    phone_number: &str,
) -> Vec<InboundAttachment> {
    let attachments = match &data_message.attachments {
        Some(atts) if !atts.is_empty() => atts,
        _ => return vec![],
    };

    attachments
        .iter()
        .map(|att| {
            let id = att.id.as_deref().unwrap_or("unknown");
            let mime_type = att
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream");

            let download_url = format!("{}/v1/attachments/{}", api_url, url_encode(id));
            let att_id = format!("signal-{}-{}", phone_number, id);

            // Try to download and store attachment data
            if let Ok(resp) = signal_get(&download_url, None) {
                if resp.status == 200 {
                    if let Err(e) = channel_host::store_attachment_data(&att_id, &resp.body) {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!("Failed to store attachment {}: {}", att_id, e),
                        );
                    }
                }
            }

            InboundAttachment {
                id: att_id,
                mime_type: mime_type.to_string(),
                filename: att.filename.clone(),
                size_bytes: att.size,
                source_url: Some(download_url),
                storage_key: None,
                extracted_text: None,
                extras_json: "{}".to_string(),
            }
        })
        .collect()
}

fn read_group_filter() -> Vec<String> {
    channel_host::workspace_read("state/group_ids")
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
}

// ============================================================================
// Utility Functions
// ============================================================================

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

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}
