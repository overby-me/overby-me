//! Email channel for IronClaw via JMAP (RFC 8621).
//!
//! This WASM component implements the IronClaw channel interface for
//! email using the JMAP protocol (JSON Mail Access Protocol).
//!
//! # Features
//!
//! - Polling-based email receiving via JMAP `Email/query` + `Email/get`
//! - Reply sending via JMAP `Email/set` + `EmailSubmission/set`
//! - DM access control (allowlist, open)
//! - Attachment support (download inbound, upload outbound)
//!
//! # Protocol
//!
//! JMAP is a modern, HTTP+JSON based email protocol (RFC 8621) that
//! replaces IMAP. It works well from WASM since it only needs HTTP.
//!
//! # Security
//!
//! - Auth token is injected by the host during HTTP requests
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
// JMAP Types
// ============================================================================

/// JMAP Session resource (RFC 8620 §2).
#[derive(Debug, Deserialize)]
struct JmapSession {
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(rename = "downloadUrl")]
    download_url: Option<String>,
    #[serde(rename = "uploadUrl")]
    upload_url: Option<String>,
    accounts: std::collections::HashMap<String, JmapAccount>,
    #[serde(rename = "primaryAccounts")]
    primary_accounts: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct JmapAccount {
    name: String,
    #[serde(rename = "isPersonal")]
    _is_personal: Option<bool>,
}

/// Generic JMAP response envelope.
#[derive(Debug, Deserialize)]
struct JmapResponse {
    #[serde(rename = "methodResponses")]
    method_responses: Vec<(String, serde_json::Value, String)>,
    #[serde(rename = "sessionState", default)]
    _session_state: Option<String>,
}

/// JMAP Email object (subset of properties we need).
#[derive(Debug, Deserialize)]
struct JmapEmail {
    id: String,
    #[serde(rename = "threadId", default)]
    thread_id: Option<String>,
    #[serde(default)]
    from: Option<Vec<JmapAddress>>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(rename = "receivedAt", default)]
    received_at: Option<String>,
    #[serde(rename = "textBody", default)]
    text_body: Option<Vec<JmapBodyPart>>,
    #[serde(rename = "htmlBody", default)]
    html_body: Option<Vec<JmapBodyPart>>,
    #[serde(default)]
    attachments: Option<Vec<JmapBodyPart>>,
    #[serde(rename = "bodyValues", default)]
    body_values: Option<std::collections::HashMap<String, JmapBodyValue>>,
    #[serde(rename = "messageId", default)]
    message_id: Option<Vec<String>>,
    #[serde(rename = "inReplyTo", default)]
    _in_reply_to: Option<Vec<String>>,
    #[serde(default)]
    references: Option<Vec<String>>,
    #[serde(rename = "mailboxIds", default)]
    _mailbox_ids: Option<std::collections::HashMap<String, bool>>,
}

#[derive(Debug, Deserialize, Clone)]
struct JmapAddress {
    name: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JmapBodyPart {
    #[serde(rename = "partId", default)]
    part_id: Option<String>,
    #[serde(rename = "blobId", default)]
    blob_id: Option<String>,
    name: Option<String>,
    #[serde(rename = "type", default)]
    content_type: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct JmapBodyValue {
    value: String,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const DM_POLICY_PATH: &str = "state/dm_policy";
const ALLOW_FROM_PATH: &str = "state/allow_from";
const CHANNEL_NAME: &str = "mail";

// ============================================================================
// Channel Metadata
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct MailMessageMetadata {
    api_url: String,
    account_id: String,
    email_id: String,
    sender_email: String,
    sender_name: Option<String>,
    subject: String,
    message_id: Option<String>,
    references: Vec<String>,
    thread_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MailConfig {
    /// JMAP server URL (e.g. "https://api.fastmail.com").
    jmap_url: String,

    /// DM policy: "allowlist" (default) or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender email addresses.
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Mailbox name to monitor (default: "Inbox").
    #[serde(default)]
    mailbox_name: Option<String>,

    /// Display name for outgoing emails.
    #[serde(default)]
    send_from_name: Option<String>,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct MailChannel;

__export_sandboxed_channel_impl!(MailChannel);

impl Guest for MailChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Mail channel config: {}", config_json),
        );

        let config: MailConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Mail channel starting for JMAP server: {}", config.jmap_url),
        );

        // Discover JMAP session
        let session_url = format!("{}/.well-known/jmap", config.jmap_url);
        let session = match jmap_get(&session_url, None) {
            Ok(resp) if resp.status == 200 => serde_json::from_slice::<JmapSession>(&resp.body)
                .map_err(|e| format!("Failed to parse JMAP session: {}", e))?,
            Ok(resp) => {
                let body = String::from_utf8_lossy(&resp.body);
                return Err(format!(
                    "JMAP session discovery failed ({}): {}",
                    resp.status, body
                ));
            }
            Err(e) => return Err(format!("Failed to reach JMAP server: {}", e)),
        };

        // Find primary mail account
        let account_id = session
            .primary_accounts
            .as_ref()
            .and_then(|pa| pa.get("urn:ietf:params:jmap:mail"))
            .cloned()
            .or_else(|| session.accounts.keys().next().cloned())
            .ok_or("No JMAP mail account found")?;

        let account_name = session
            .accounts
            .get(&account_id)
            .map(|a| a.name.clone())
            .unwrap_or_default();

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Using JMAP account: {} ({})", account_id, account_name),
        );

        // Persist state
        let _ = channel_host::workspace_write("state/jmap_url", &config.jmap_url);
        let _ = channel_host::workspace_write("state/api_url", &session.api_url);
        let _ = channel_host::workspace_write("state/account_id", &account_id);

        if let Some(ref dl) = session.download_url {
            let _ = channel_host::workspace_write("state/download_url", dl);
        }
        if let Some(ref ul) = session.upload_url {
            let _ = channel_host::workspace_write("state/upload_url", ul);
        }

        let send_name = config.send_from_name.as_deref().unwrap_or("IronClaw");
        let _ = channel_host::workspace_write("state/send_from_name", send_name);

        // Resolve mailbox ID for the target mailbox
        let mailbox_name = config.mailbox_name.as_deref().unwrap_or("Inbox");
        let _ = channel_host::workspace_write("state/mailbox_name", mailbox_name);

        if let Some(mailbox_id) = resolve_mailbox_id(&session.api_url, &account_id, mailbox_name) {
            let _ = channel_host::workspace_write("state/mailbox_id", &mailbox_id);
        } else {
            return Err(format!("Mailbox '{}' not found", mailbox_name));
        }

        // Persist DM policy
        let dm_policy = config.dm_policy.as_deref().unwrap_or("allowlist");
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Do an initial query to get the latest email state so we don't
        // replay old emails on the first real poll.
        if let Some(latest_received_at) = get_latest_received_at(&session.api_url, &account_id) {
            let _ = channel_host::workspace_write("state/last_seen_at", &latest_received_at);
        }

        Ok(ChannelConfig {
            display_name: "Email".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/mail".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: Some(PollConfig {
                interval_ms: 30_000,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // JMAP doesn't use webhooks in our setup; we use polling.
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

        let account_id = match channel_host::workspace_read("state/account_id") {
            Some(id) => id,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No account ID in workspace state",
                );
                return;
            }
        };

        let mailbox_id = match channel_host::workspace_read("state/mailbox_id") {
            Some(id) => id,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "No mailbox ID in workspace state",
                );
                return;
            }
        };

        let last_seen_at = channel_host::workspace_read("state/last_seen_at");

        // Query for new emails since last_seen_at
        let mut filter = serde_json::json!({
            "inMailbox": mailbox_id,
            "isUnread": true,
        });

        if let Some(ref since) = last_seen_at {
            filter["after"] = serde_json::json!(since);
        }

        let query_call = serde_json::json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"],
            "methodCalls": [
                ["Email/query", {
                    "accountId": account_id,
                    "filter": filter,
                    "sort": [{"property": "receivedAt", "isAscending": true}],
                    "limit": 20,
                }, "q0"],
                ["Email/get", {
                    "accountId": account_id,
                    "#ids": {
                        "resultOf": "q0",
                        "name": "Email/query",
                        "path": "/ids"
                    },
                    "properties": [
                        "id", "threadId", "from", "to", "subject", "receivedAt",
                        "textBody", "htmlBody", "attachments", "bodyValues",
                        "messageId", "inReplyTo", "references", "mailboxIds"
                    ],
                    "fetchTextBodyValues": true,
                    "maxBodyValueBytes": 65536,
                }, "g0"]
            ]
        });

        let result = jmap_post(&api_url, &query_call, Some(30_000));

        match result {
            Ok(response) => {
                if response.status != 200 {
                    let body_str = String::from_utf8_lossy(&response.body);
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("JMAP request returned {}: {}", response.status, body_str),
                    );
                    return;
                }

                let jmap_resp: JmapResponse = match serde_json::from_slice(&response.body) {
                    Ok(r) => r,
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Failed to parse JMAP response: {}", e),
                        );
                        return;
                    }
                };

                // Find Email/get response
                let emails = extract_emails_from_response(&jmap_resp);

                let mut latest_received_at = last_seen_at.clone();

                for email in &emails {
                    // Track latest timestamp
                    if let Some(ref received) = email.received_at {
                        if latest_received_at.as_ref().is_none_or(|l| received > l) {
                            latest_received_at = Some(received.clone());
                        }
                    }

                    process_email(email, &api_url, &account_id);
                }

                // Update last_seen_at
                if let Some(ref ts) = latest_received_at {
                    let _ = channel_host::workspace_write("state/last_seen_at", ts);
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("JMAP poll request failed: {}", e),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: MailMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        send_reply(&metadata, &response.content, &response.attachments)
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let api_url = channel_host::workspace_read("state/api_url").ok_or("No API URL")?;
        let account_id = channel_host::workspace_read("state/account_id").ok_or("No account ID")?;

        // user_id is expected to be an email address for broadcast
        let metadata = MailMessageMetadata {
            api_url,
            account_id,
            email_id: String::new(),
            sender_email: user_id.clone(),
            sender_name: None,
            subject: "Message from IronClaw".to_string(),
            message_id: None,
            references: vec![],
            thread_id: None,
        };

        send_reply(&metadata, &response.content, &response.attachments)
    }

    fn on_status(update: StatusUpdate) {
        // Email doesn't support typing indicators.
        // For approval/auth status, send an email.
        match update.status {
            StatusType::ApprovalNeeded | StatusType::AuthRequired | StatusType::AuthCompleted => {
                let msg = update.message.trim();
                if !msg.is_empty() {
                    if let Ok(metadata) =
                        serde_json::from_str::<MailMessageMetadata>(&update.metadata_json)
                    {
                        let _ = send_reply(&metadata, msg, &[]);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Mail channel shutting down");
    }
}

// ============================================================================
// Email Processing
// ============================================================================

fn process_email(email: &JmapEmail, api_url: &str, account_id: &str) {
    let sender_addr = match email
        .from
        .as_ref()
        .and_then(|f| f.first())
        .and_then(|a| a.email.as_ref())
    {
        Some(addr) => addr.clone(),
        None => return,
    };

    let sender_name = email
        .from
        .as_ref()
        .and_then(|f| f.first())
        .and_then(|a| a.name.clone());

    // DM access control
    if !check_dm_access(&sender_addr) {
        return;
    }

    // Extract text body
    let body = extract_body(email);
    if body.is_empty() {
        return;
    }

    let subject = email
        .subject
        .as_deref()
        .unwrap_or("(no subject)")
        .to_string();

    // Build references chain for threading
    let mut refs = email.references.clone().unwrap_or_default();
    if let Some(ref msg_ids) = email.message_id {
        if let Some(msg_id) = msg_ids.first() {
            if !refs.contains(msg_id) {
                refs.push(msg_id.clone());
            }
        }
    }

    let metadata = MailMessageMetadata {
        api_url: api_url.to_string(),
        account_id: account_id.to_string(),
        email_id: email.id.clone(),
        sender_email: sender_addr.clone(),
        sender_name: sender_name.clone(),
        subject: subject.clone(),
        message_id: email
            .message_id
            .as_ref()
            .and_then(|ids| ids.first().cloned()),
        references: refs,
        thread_id: email.thread_id.clone(),
    };

    // Format content with subject context
    let content = format!("Subject: {}\n\n{}", subject, body);

    // Collect attachments
    let attachments = extract_attachments(email, api_url, account_id);

    channel_host::emit_message(&EmittedMessage {
        user_id: sender_addr,
        user_name: sender_name,
        content,
        thread_id: email.thread_id.clone(),
        metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
        attachments,
    });

    // Mark the email as read
    mark_as_read(api_url, account_id, &email.id);
}

fn extract_body(email: &JmapEmail) -> String {
    let body_values = match &email.body_values {
        Some(bv) => bv,
        None => return String::new(),
    };

    // Prefer text/plain body
    if let Some(ref text_parts) = email.text_body {
        for part in text_parts {
            if let Some(ref part_id) = part.part_id {
                if let Some(bv) = body_values.get(part_id) {
                    let text = bv.value.trim().to_string();
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
        }
    }

    // Fall back to HTML body (strip tags)
    if let Some(ref html_parts) = email.html_body {
        for part in html_parts {
            if let Some(ref part_id) = part.part_id {
                if let Some(bv) = body_values.get(part_id) {
                    let stripped = strip_html_tags(&bv.value);
                    let text = stripped.trim().to_string();
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
        }
    }

    String::new()
}

fn extract_attachments(
    email: &JmapEmail,
    api_url: &str,
    account_id: &str,
) -> Vec<InboundAttachment> {
    let atts = match &email.attachments {
        Some(a) if !a.is_empty() => a,
        _ => return vec![],
    };

    let download_url_template =
        channel_host::workspace_read("state/download_url").unwrap_or_else(|| {
            format!(
                "{}/jmap/download/{{accountId}}/{{blobId}}/{{name}}",
                api_url
            )
        });

    atts.iter()
        .filter_map(|att| {
            let blob_id = att.blob_id.as_ref()?;
            let mime = att
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream");
            let filename = att.name.clone();
            let att_id = format!("mail-{}-{}", email.id, blob_id);

            let download_url = download_url_template
                .replace("{accountId}", &url_encode(account_id))
                .replace("{blobId}", &url_encode(blob_id))
                .replace(
                    "{name}",
                    &url_encode(filename.as_deref().unwrap_or("attachment")),
                );

            Some(InboundAttachment {
                id: att_id,
                mime_type: mime.to_string(),
                filename,
                size_bytes: att.size,
                source_url: Some(download_url),
                storage_key: None,
                extracted_text: None,
                extras_json: "{}".to_string(),
            })
        })
        .collect()
}

// ============================================================================
// JMAP Helpers
// ============================================================================

fn resolve_mailbox_id(api_url: &str, account_id: &str, mailbox_name: &str) -> Option<String> {
    let call = serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
        "methodCalls": [
            ["Mailbox/get", {
                "accountId": account_id,
                "properties": ["id", "name", "role"]
            }, "m0"]
        ]
    });

    let resp = jmap_post(api_url, &call, None).ok()?;
    if resp.status != 200 {
        return None;
    }

    let jmap_resp: JmapResponse = serde_json::from_slice(&resp.body).ok()?;

    for (method, data, _) in &jmap_resp.method_responses {
        if method == "Mailbox/get" {
            if let Some(list) = data.get("list").and_then(|l| l.as_array()) {
                // First try matching by name
                for mb in list {
                    let name = mb.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name.eq_ignore_ascii_case(mailbox_name) {
                        return mb
                            .get("id")
                            .and_then(|id| id.as_str())
                            .map(|s| s.to_string());
                    }
                }
                // Fall back to matching by role for "Inbox"
                if mailbox_name.eq_ignore_ascii_case("inbox") {
                    for mb in list {
                        let role = mb.get("role").and_then(|r| r.as_str()).unwrap_or("");
                        if role == "inbox" {
                            return mb
                                .get("id")
                                .and_then(|id| id.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

fn get_latest_received_at(api_url: &str, account_id: &str) -> Option<String> {
    let mailbox_id = channel_host::workspace_read("state/mailbox_id")?;

    let call = serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
        "methodCalls": [
            ["Email/query", {
                "accountId": account_id,
                "filter": {"inMailbox": mailbox_id},
                "sort": [{"property": "receivedAt", "isAscending": false}],
                "limit": 1,
            }, "q0"],
            ["Email/get", {
                "accountId": account_id,
                "#ids": {
                    "resultOf": "q0",
                    "name": "Email/query",
                    "path": "/ids"
                },
                "properties": ["receivedAt"],
            }, "g0"]
        ]
    });

    let resp = jmap_post(api_url, &call, None).ok()?;
    if resp.status != 200 {
        return None;
    }

    let jmap_resp: JmapResponse = serde_json::from_slice(&resp.body).ok()?;

    for (method, data, _) in &jmap_resp.method_responses {
        if method == "Email/get" {
            if let Some(list) = data.get("list").and_then(|l| l.as_array()) {
                if let Some(email) = list.first() {
                    return email
                        .get("receivedAt")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string());
                }
            }
        }
    }

    None
}

fn mark_as_read(api_url: &str, account_id: &str, email_id: &str) {
    let call = serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
        "methodCalls": [
            ["Email/set", {
                "accountId": account_id,
                "update": {
                    email_id: {
                        "keywords/$seen": true,
                    }
                }
            }, "s0"]
        ]
    });

    if let Err(e) = jmap_post(api_url, &call, None) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Failed to mark email as read: {}", e),
        );
    }
}

fn send_reply(
    metadata: &MailMessageMetadata,
    text: &str,
    attachments: &[exports::near::agent::channel::Attachment],
) -> Result<(), String> {
    let send_name = channel_host::workspace_read("state/send_from_name")
        .unwrap_or_else(|| "IronClaw".to_string());

    // We need our own email address — get it from the account identity
    let from_email = get_own_email(&metadata.api_url, &metadata.account_id)?;

    // Build subject line
    let subject = if metadata.subject.starts_with("Re: ") {
        metadata.subject.clone()
    } else {
        format!("Re: {}", metadata.subject)
    };

    // Build references header for threading
    let references = if metadata.references.is_empty() {
        None
    } else {
        Some(metadata.references.clone())
    };

    let in_reply_to = metadata.message_id.as_ref().map(|id| vec![id.clone()]);

    // Upload attachments and collect blob IDs
    let mut att_parts = Vec::new();
    for attachment in attachments {
        if let Ok(blob_id) = upload_blob(&metadata.api_url, &metadata.account_id, attachment) {
            att_parts.push(serde_json::json!({
                "blobId": blob_id,
                "type": attachment.mime_type,
                "name": attachment.filename,
                "disposition": "attachment",
            }));
        }
    }

    // Create draft and submit in one request
    let draft_id = format!("draft-{}", channel_host::now_millis());

    let mut email_obj = serde_json::json!({
        "from": [{"name": send_name, "email": from_email}],
        "to": [{"email": metadata.sender_email}],
        "subject": subject,
        "textBody": [{
            "partId": "body",
            "type": "text/plain",
        }],
        "bodyValues": {
            "body": {"value": text, "isEncodingProblem": false, "isTruncated": false}
        },
    });

    if let Some(ref refs) = references {
        email_obj["references"] = serde_json::json!(refs);
    }
    if let Some(ref irt) = in_reply_to {
        email_obj["inReplyTo"] = serde_json::json!(irt);
    }
    if !att_parts.is_empty() {
        email_obj["attachments"] = serde_json::json!(att_parts);
    }

    let call = serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"],
        "methodCalls": [
            ["Email/set", {
                "accountId": metadata.account_id,
                "create": {
                    &draft_id: email_obj,
                }
            }, "e0"],
            ["EmailSubmission/set", {
                "accountId": metadata.account_id,
                "create": {
                    "sub0": {
                        "emailId": format!("#{}", draft_id),
                        "identityId": get_identity_id(&metadata.api_url, &metadata.account_id).unwrap_or_default(),
                    }
                },
                "onSuccessDestroyEmail": ["#sub0"],
            }, "s0"]
        ]
    });

    match jmap_post(&metadata.api_url, &call, None) {
        Ok(resp) if resp.status == 200 => {
            // Check for method-level errors
            if let Ok(jmap_resp) = serde_json::from_slice::<JmapResponse>(&resp.body) {
                for (method, data, _) in &jmap_resp.method_responses {
                    if method == "error" {
                        let err_type = data
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown");
                        return Err(format!("JMAP error: {}", err_type));
                    }
                    if (method == "Email/set" || method == "EmailSubmission/set")
                        && data.get("notCreated").is_some()
                    {
                        let not_created = data.get("notCreated").unwrap();
                        if let Some(obj) = not_created.as_object() {
                            if !obj.is_empty() {
                                return Err(format!("Failed to create email: {:?}", not_created));
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        Ok(resp) => {
            let err_body = String::from_utf8_lossy(&resp.body);
            Err(format!("JMAP send returned {}: {}", resp.status, err_body))
        }
        Err(e) => Err(format!("JMAP send failed: {}", e)),
    }
}

fn get_own_email(api_url: &str, account_id: &str) -> Result<String, String> {
    // Check cached value first
    if let Some(email) = channel_host::workspace_read("state/own_email") {
        return Ok(email);
    }

    let call = serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:submission"],
        "methodCalls": [
            ["Identity/get", {
                "accountId": account_id,
                "properties": ["id", "email", "name"]
            }, "i0"]
        ]
    });

    let resp = jmap_post(api_url, &call, None)
        .map_err(|e| format!("Failed to fetch identities: {}", e))?;

    if resp.status != 200 {
        return Err(format!("Identity/get returned {}", resp.status));
    }

    let jmap_resp: JmapResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse identity response: {}", e))?;

    for (method, data, _) in &jmap_resp.method_responses {
        if method == "Identity/get" {
            if let Some(list) = data.get("list").and_then(|l| l.as_array()) {
                if let Some(identity) = list.first() {
                    if let Some(email) = identity.get("email").and_then(|e| e.as_str()) {
                        let _ = channel_host::workspace_write("state/own_email", email);
                        // Also cache identity ID
                        if let Some(id) = identity.get("id").and_then(|i| i.as_str()) {
                            let _ = channel_host::workspace_write("state/identity_id", id);
                        }
                        return Ok(email.to_string());
                    }
                }
            }
        }
    }

    Err("No identity found".to_string())
}

fn get_identity_id(api_url: &str, account_id: &str) -> Option<String> {
    if let Some(id) = channel_host::workspace_read("state/identity_id") {
        return Some(id);
    }

    // Fetching own email also caches identity_id
    let _ = get_own_email(api_url, account_id);
    channel_host::workspace_read("state/identity_id")
}

fn upload_blob(
    api_url: &str,
    account_id: &str,
    attachment: &exports::near::agent::channel::Attachment,
) -> Result<String, String> {
    let upload_url = channel_host::workspace_read("state/upload_url")
        .unwrap_or_else(|| format!("{}/jmap/upload/{{accountId}}/", api_url));

    let url = upload_url.replace("{accountId}", &url_encode(account_id));

    let headers = serde_json::json!({
        "Authorization": "Bearer {MAIL_AUTH_TOKEN}",
        "Content-Type": attachment.mime_type,
    });

    let resp = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&attachment.data),
        None,
    )?;

    if resp.status != 200 && resp.status != 201 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Upload failed ({}): {}", resp.status, body));
    }

    let upload_resp: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse upload response: {}", e))?;

    upload_resp
        .get("blobId")
        .and_then(|b| b.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No blobId in upload response".to_string())
}

fn extract_emails_from_response(jmap_resp: &JmapResponse) -> Vec<JmapEmail> {
    for (method, data, _) in &jmap_resp.method_responses {
        if method == "Email/get" {
            if let Some(list) = data.get("list") {
                if let Ok(emails) = serde_json::from_value::<Vec<JmapEmail>>(list.clone()) {
                    return emails;
                }
            }
        }
    }
    vec![]
}

// ============================================================================
// HTTP Helpers
// ============================================================================

fn jmap_get(url: &str, timeout_ms: Option<u32>) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {MAIL_AUTH_TOKEN}",
    });
    channel_host::http_request("GET", url, &headers.to_string(), None, timeout_ms)
}

fn jmap_post(
    url: &str,
    body: &serde_json::Value,
    timeout_ms: Option<u32>,
) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Bearer {MAIL_AUTH_TOKEN}",
        "Content-Type": "application/json",
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
// Access Control
// ============================================================================

fn check_dm_access(sender: &str) -> bool {
    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "allowlist".to_string());

    match dm_policy.as_str() {
        "open" => true,
        _ => is_sender_allowed(sender),
    }
}

fn is_sender_allowed(sender: &str) -> bool {
    if let Some(allow_from_json) = channel_host::workspace_read(ALLOW_FROM_PATH) {
        if let Ok(allow_from) = serde_json::from_str::<Vec<String>>(&allow_from_json) {
            if allow_from.iter().any(|a| a.eq_ignore_ascii_case(sender)) {
                return true;
            }
        }
    }

    if let Ok(approved) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        if approved.iter().any(|a| a.eq_ignore_ascii_case(sender)) {
            return true;
        }
    }

    false
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

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
}
