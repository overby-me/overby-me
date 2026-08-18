//! Contacts channel for IronClaw via CardDAV.
//!
//! This WASM component implements the IronClaw channel interface for
//! contacts management using the CardDAV protocol (RFC 6352).
//!
//! # Features
//!
//! - Polling-based address book monitoring
//! - Reports new/modified contacts to the agent
//! - Agent can create/update/delete contacts via responses
//! - vCard (RFC 6350) parsing and generation
//!
//! # Protocol
//!
//! CardDAV is an HTTP-based protocol for contacts access. It uses
//! WebDAV PROPFIND/REPORT methods with XML bodies and vCard data.

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "wit/channel.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// CardDAV Types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct ContactsConfig {
    /// CardDAV server URL (e.g. "http://localhost:8080").
    carddav_url: String,

    /// Address book name to monitor (default: "default").
    #[serde(default)]
    addressbook_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ContactMetadata {
    carddav_url: String,
    principal_url: String,
    addressbook_href: String,
    contact_href: String,
    contact_uid: String,
    etag: Option<String>,
}

#[derive(Debug, Clone)]
struct Contact {
    href: String,
    etag: Option<String>,
    uid: String,
    full_name: Option<String>,
    emails: Vec<String>,
    phones: Vec<String>,
    org: Option<String>,
    title: Option<String>,
    _vcard_data: String,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const CHANNEL_NAME: &str = "contacts";

// ============================================================================
// Channel Implementation
// ============================================================================

struct ContactsChannel;

__export_sandboxed_channel_impl!(ContactsChannel);

impl Guest for ContactsChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Contacts channel config: {}", config_json),
        );

        let config: ContactsConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Contacts channel starting for CardDAV server: {}",
                config.carddav_url
            ),
        );

        // Store config
        let _ = channel_host::workspace_write("state/carddav_url", &config.carddav_url);

        let addressbook_name = config.addressbook_name.as_deref().unwrap_or("default");
        let _ = channel_host::workspace_write("state/addressbook_name", addressbook_name);

        // Discover principal URL
        let principal_url = discover_principal(&config.carddav_url)?;
        let _ = channel_host::workspace_write("state/principal_url", &principal_url);

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("CardDAV principal: {}", principal_url),
        );

        // Discover addressbook home and find target addressbook
        let addressbook_home = discover_addressbook_home(&config.carddav_url, &principal_url)?;
        let _ = channel_host::workspace_write("state/addressbook_home", &addressbook_home);

        let addressbook_href =
            find_addressbook(&config.carddav_url, &addressbook_home, addressbook_name)?;
        let _ = channel_host::workspace_write("state/addressbook_href", &addressbook_href);

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Using address book: {}", addressbook_href),
        );

        Ok(ChannelConfig {
            display_name: "Contacts".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/contacts".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: Some(PollConfig {
                interval_ms: 60_000,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        let carddav_url = match channel_host::workspace_read("state/carddav_url") {
            Some(url) => url,
            None => return,
        };
        let principal_url = match channel_host::workspace_read("state/principal_url") {
            Some(url) => url,
            None => return,
        };
        let addressbook_href = match channel_host::workspace_read("state/addressbook_href") {
            Some(href) => href,
            None => return,
        };

        let last_ctag = channel_host::workspace_read("state/ctag");

        // Check CTag to see if addressbook changed
        let current_ctag = get_ctag(&carddav_url, &addressbook_href);

        if let (Some(ref last), Some(ref current)) = (&last_ctag, &current_ctag) {
            if last == current {
                return; // No changes
            }
        }

        // Fetch contacts
        let contacts = fetch_contacts(&carddav_url, &addressbook_href);

        let known_etags_json =
            channel_host::workspace_read("state/known_etags").unwrap_or_else(|| "{}".to_string());
        let mut known_etags: std::collections::HashMap<String, String> =
            serde_json::from_str(&known_etags_json).unwrap_or_default();

        for contact in &contacts {
            let is_new_or_modified = match known_etags.get(&contact.href) {
                Some(old_etag) => contact.etag.as_ref().is_none_or(|e| e != old_etag),
                None => true,
            };

            if !is_new_or_modified {
                continue;
            }

            if let Some(ref etag) = contact.etag {
                known_etags.insert(contact.href.clone(), etag.clone());
            }

            let metadata = ContactMetadata {
                carddav_url: carddav_url.clone(),
                principal_url: principal_url.clone(),
                addressbook_href: addressbook_href.clone(),
                contact_href: contact.href.clone(),
                contact_uid: contact.uid.clone(),
                etag: contact.etag.clone(),
            };

            let content = format_contact_summary(contact);

            channel_host::emit_message(&EmittedMessage {
                user_id: CHANNEL_NAME.to_string(),
                user_name: Some("Contacts".to_string()),
                content,
                thread_id: Some(contact.uid.clone()),
                metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
                attachments: vec![],
            });
        }

        if let Ok(etags_json) = serde_json::to_string(&known_etags) {
            let _ = channel_host::workspace_write("state/known_etags", &etags_json);
        }
        if let Some(ref ctag) = current_ctag {
            let _ = channel_host::workspace_write("state/ctag", ctag);
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: ContactMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        if let Ok(instruction) = serde_json::from_str::<serde_json::Value>(&response.content) {
            match instruction
                .get("action")
                .and_then(|a| a.as_str())
                .unwrap_or("")
            {
                "create" => {
                    let vcard = instruction
                        .get("vcard")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'vcard' field for create action")?;
                    let uid = instruction
                        .get("uid")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&metadata.contact_uid);
                    let href = format!("{}{}.vcf", metadata.addressbook_href, uid);
                    put_contact(&metadata.carddav_url, &href, vcard, None)?;
                    Ok(())
                }
                "update" => {
                    let vcard = instruction
                        .get("vcard")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'vcard' field for update action")?;
                    put_contact(
                        &metadata.carddav_url,
                        &metadata.contact_href,
                        vcard,
                        metadata.etag.as_deref(),
                    )?;
                    Ok(())
                }
                "delete" => {
                    delete_contact(
                        &metadata.carddav_url,
                        &metadata.contact_href,
                        metadata.etag.as_deref(),
                    )?;
                    Ok(())
                }
                _ => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Ok(())
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Contacts channel shutting down",
        );
    }
}

// ============================================================================
// CardDAV Discovery
// ============================================================================

fn discover_principal(carddav_url: &str) -> Result<String, String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:current-user-principal/>
  </d:prop>
</d:propfind>"#;

    let resp = carddav_request("PROPFIND", &format!("{}/", carddav_url), body, Some("0"))?;

    if resp.status != 207 {
        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "PROPFIND for principal failed ({}): {}",
            resp.status, body_str
        ));
    }

    let body_str = String::from_utf8_lossy(&resp.body);
    extract_xml_href(&body_str, "current-user-principal")
        .ok_or_else(|| "Could not find current-user-principal in PROPFIND response".to_string())
}

fn discover_addressbook_home(carddav_url: &str, principal_url: &str) -> Result<String, String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop>
    <c:addressbook-home-set/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(carddav_url, principal_url);
    let resp = carddav_request("PROPFIND", &url, body, Some("0"))?;

    if resp.status != 207 {
        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "PROPFIND for addressbook-home failed ({}): {}",
            resp.status, body_str
        ));
    }

    let body_str = String::from_utf8_lossy(&resp.body);
    extract_xml_href(&body_str, "addressbook-home-set")
        .ok_or_else(|| "Could not find addressbook-home-set".to_string())
}

fn find_addressbook(
    carddav_url: &str,
    addressbook_home: &str,
    addressbook_name: &str,
) -> Result<String, String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <d:displayname/>
    <d:resourcetype/>
    <cs:getctag/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(carddav_url, addressbook_home);
    let resp = carddav_request("PROPFIND", &url, body, Some("1"))?;

    if resp.status != 207 {
        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "PROPFIND for addressbooks failed ({}): {}",
            resp.status, body_str
        ));
    }

    let body_str = String::from_utf8_lossy(&resp.body);

    let mut best_match: Option<String> = None;
    let mut default_book: Option<String> = None;

    for response_block in body_str.split("<d:response>").skip(1) {
        let href = match extract_tag_content(response_block, "d:href") {
            Some(h) => h,
            None => continue,
        };

        if href.trim_end_matches('/') == addressbook_home.trim_end_matches('/') {
            continue;
        }

        if !response_block.contains("addressbook") {
            continue;
        }

        let display_name = extract_tag_content(response_block, "d:displayname");

        if let Some(ref name) = display_name {
            if name.eq_ignore_ascii_case(addressbook_name) {
                best_match = Some(href.clone());
                break;
            }
        }

        if default_book.is_none() {
            default_book = Some(href);
        }
    }

    // Also check D: prefix variant
    if best_match.is_none() && default_book.is_none() {
        for response_block in body_str.split("<D:response>").skip(1) {
            let href = match extract_tag_content(response_block, "D:href") {
                Some(h) => h,
                None => continue,
            };

            if response_block.contains("addressbook") {
                let display_name = extract_tag_content(response_block, "D:displayname");

                if let Some(ref name) = display_name {
                    if name.eq_ignore_ascii_case(addressbook_name) {
                        best_match = Some(href.clone());
                        break;
                    }
                }

                if default_book.is_none() {
                    default_book = Some(href);
                }
            }
        }
    }

    best_match
        .or(default_book)
        .ok_or_else(|| format!("No address book found matching '{}'", addressbook_name))
}

// ============================================================================
// CardDAV Operations
// ============================================================================

fn get_ctag(carddav_url: &str, addressbook_href: &str) -> Option<String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <cs:getctag/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(carddav_url, addressbook_href);
    let resp = carddav_request("PROPFIND", &url, body, Some("0")).ok()?;
    if resp.status != 207 {
        return None;
    }
    let body_str = String::from_utf8_lossy(&resp.body);
    extract_tag_content(&body_str, "cs:getctag")
        .or_else(|| extract_tag_content(&body_str, "getctag"))
}

fn fetch_contacts(carddav_url: &str, addressbook_href: &str) -> Vec<Contact> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop>
    <d:getetag/>
    <c:address-data/>
  </d:prop>
</c:addressbook-query>"#;

    let url = resolve_url(carddav_url, addressbook_href);
    let resp = match carddav_report(&url, body) {
        Ok(r) => r,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("CardDAV REPORT failed: {}", e),
            );
            return vec![];
        }
    };

    if resp.status != 207 {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("CardDAV REPORT returned {}", resp.status),
        );
        return vec![];
    }

    let body_str = String::from_utf8_lossy(&resp.body);
    parse_contacts_multistatus(&body_str)
}

fn put_contact(
    carddav_url: &str,
    contact_href: &str,
    vcard_data: &str,
    etag: Option<&str>,
) -> Result<(), String> {
    let url = resolve_url(carddav_url, contact_href);

    let mut headers = serde_json::json!({
        "Authorization": "Basic {CONTACTS_AUTH_TOKEN}",
        "Content-Type": "text/vcard; charset=utf-8",
    });

    if let Some(etag) = etag {
        headers["If-Match"] = serde_json::json!(etag);
    }

    let resp = channel_host::http_request(
        "PUT",
        &url,
        &headers.to_string(),
        Some(vcard_data.as_bytes()),
        None,
    )?;

    if resp.status >= 200 && resp.status < 300 {
        Ok(())
    } else {
        let body = String::from_utf8_lossy(&resp.body);
        Err(format!("PUT contact failed ({}): {}", resp.status, body))
    }
}

fn delete_contact(carddav_url: &str, contact_href: &str, etag: Option<&str>) -> Result<(), String> {
    let url = resolve_url(carddav_url, contact_href);

    let mut headers = serde_json::json!({
        "Authorization": "Basic {CONTACTS_AUTH_TOKEN}",
    });

    if let Some(etag) = etag {
        headers["If-Match"] = serde_json::json!(etag);
    }

    let resp = channel_host::http_request("DELETE", &url, &headers.to_string(), None, None)?;

    if resp.status >= 200 && resp.status < 300 {
        Ok(())
    } else {
        let body = String::from_utf8_lossy(&resp.body);
        Err(format!("DELETE contact failed ({}): {}", resp.status, body))
    }
}

// ============================================================================
// vCard Parsing
// ============================================================================

fn parse_contacts_multistatus(xml: &str) -> Vec<Contact> {
    let mut contacts = Vec::new();

    let response_blocks: Vec<&str> = xml
        .split("<d:response>")
        .chain(xml.split("<D:response>"))
        .skip(1)
        .collect();

    for block in response_blocks {
        let href =
            extract_tag_content(block, "d:href").or_else(|| extract_tag_content(block, "D:href"));
        let etag = extract_tag_content(block, "d:getetag")
            .or_else(|| extract_tag_content(block, "D:getetag"));
        let vcard = extract_tag_content(block, "c:address-data")
            .or_else(|| extract_tag_content(block, "C:address-data"))
            .or_else(|| extract_tag_content(block, "card:address-data"));

        let href = match href {
            Some(h) => h,
            None => continue,
        };
        let vcard = match vcard {
            Some(v) => v,
            None => continue,
        };

        let uid = extract_vcard_property(&vcard, "UID").unwrap_or_default();
        if uid.is_empty() {
            continue;
        }

        let full_name = extract_vcard_property(&vcard, "FN");
        let emails = extract_vcard_properties(&vcard, "EMAIL");
        let phones = extract_vcard_properties(&vcard, "TEL");
        let org = extract_vcard_property(&vcard, "ORG");
        let title = extract_vcard_property(&vcard, "TITLE");

        contacts.push(Contact {
            href,
            etag,
            uid,
            full_name,
            emails,
            phones,
            org,
            title,
            _vcard_data: vcard,
        });
    }

    contacts
}

fn extract_vcard_property(vcard: &str, property: &str) -> Option<String> {
    for line in vcard.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(property) {
            if let Some(value) = rest.strip_prefix(':') {
                return Some(value.to_string());
            } else if rest.starts_with(';') {
                if let Some(colon_pos) = rest.find(':') {
                    return Some(rest[colon_pos + 1..].to_string());
                }
            }
        }
    }
    None
}

fn extract_vcard_properties(vcard: &str, property: &str) -> Vec<String> {
    let mut values = Vec::new();
    for line in vcard.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(property) {
            if let Some(value) = rest.strip_prefix(':') {
                values.push(value.to_string());
            } else if rest.starts_with(';') {
                if let Some(colon_pos) = rest.find(':') {
                    values.push(rest[colon_pos + 1..].to_string());
                }
            }
        }
    }
    values
}

fn format_contact_summary(contact: &Contact) -> String {
    let mut parts = Vec::new();

    let name = contact.full_name.as_deref().unwrap_or("(unnamed contact)");
    parts.push(format!("Contact: {}", name));

    if !contact.emails.is_empty() {
        parts.push(format!("Email: {}", contact.emails.join(", ")));
    }
    if !contact.phones.is_empty() {
        parts.push(format!("Phone: {}", contact.phones.join(", ")));
    }
    if let Some(ref org) = contact.org {
        if !org.is_empty() {
            parts.push(format!("Organization: {}", org));
        }
    }
    if let Some(ref title) = contact.title {
        if !title.is_empty() {
            parts.push(format!("Title: {}", title));
        }
    }

    parts.join("\n")
}

// ============================================================================
// HTTP/XML Helpers
// ============================================================================

fn carddav_request(
    method: &str,
    url: &str,
    body: &str,
    depth: Option<&str>,
) -> Result<channel_host::HttpResponse, String> {
    let mut headers = serde_json::json!({
        "Authorization": "Basic {CONTACTS_AUTH_TOKEN}",
        "Content-Type": "application/xml; charset=utf-8",
    });

    if let Some(d) = depth {
        headers["Depth"] = serde_json::json!(d);
    }

    channel_host::http_request(
        method,
        url,
        &headers.to_string(),
        Some(body.as_bytes()),
        Some(30_000),
    )
}

fn carddav_report(url: &str, body: &str) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Basic {CONTACTS_AUTH_TOKEN}",
        "Content-Type": "application/xml; charset=utf-8",
        "Depth": "1",
    });

    channel_host::http_request(
        "REPORT",
        url,
        &headers.to_string(),
        Some(body.as_bytes()),
        Some(60_000),
    )
}

fn resolve_url(base_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", base_url.trim_end_matches('/'), path)
    }
}

fn extract_xml_href(xml: &str, parent_tag: &str) -> Option<String> {
    let parent_start = format!("<d:{}", parent_tag);
    let parent_start_alt = format!("<D:{}", parent_tag);
    let parent_start_no_prefix = format!("<{}", parent_tag);

    let start_pos = xml
        .find(&parent_start)
        .or_else(|| xml.find(&parent_start_alt))
        .or_else(|| xml.find(&parent_start_no_prefix))?;

    let remaining = &xml[start_pos..];
    extract_tag_content(remaining, "d:href")
        .or_else(|| extract_tag_content(remaining, "D:href"))
        .or_else(|| extract_tag_content(remaining, "href"))
}

fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let open_with_attrs = format!("<{} ", tag);
    let close = format!("</{}>", tag);

    let start = xml.find(&open).map(|pos| pos + open.len()).or_else(|| {
        xml.find(&open_with_attrs).and_then(|pos| {
            let rest = &xml[pos..];
            rest.find('>').map(|end| pos + end + 1)
        })
    })?;

    let end = xml[start..].find(&close).map(|pos| start + pos)?;

    Some(xml[start..end].trim().to_string())
}

fn json_response(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({"Content-Type": "application/json"}).to_string(),
        body: body_bytes,
    }
}
