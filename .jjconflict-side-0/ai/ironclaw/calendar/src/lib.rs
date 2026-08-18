//! Calendar channel for IronClaw via CalDAV.
//!
//! This WASM component implements the IronClaw channel interface for
//! calendar management using the CalDAV protocol (RFC 4791).
//!
//! # Features
//!
//! - Polling-based calendar event monitoring
//! - Reports new/modified events to the agent
//! - Agent can create/update/delete events via responses
//! - iCalendar (RFC 5545) parsing and generation
//!
//! # Protocol
//!
//! CalDAV is an HTTP-based protocol for calendar access. It uses
//! WebDAV PROPFIND/REPORT methods with XML bodies and iCalendar data.

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
// CalDAV Types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct CalendarConfig {
    /// CalDAV server URL (e.g. "http://localhost:8080").
    caldav_url: String,

    /// Calendar name to monitor (default: "default").
    #[serde(default)]
    calendar_name: Option<String>,

    /// Poll interval in milliseconds.
    #[serde(default)]
    poll_interval_ms: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CalendarEventMetadata {
    caldav_url: String,
    principal_url: String,
    calendar_href: String,
    event_href: String,
    event_uid: String,
    etag: Option<String>,
}

#[derive(Debug, Clone)]
struct CalendarEvent {
    href: String,
    etag: Option<String>,
    uid: String,
    summary: String,
    dtstart: Option<String>,
    dtend: Option<String>,
    description: Option<String>,
    location: Option<String>,
    _ical_data: String,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const CHANNEL_NAME: &str = "calendar";

// ============================================================================
// Channel Implementation
// ============================================================================

struct CalendarChannel;

__export_sandboxed_channel_impl!(CalendarChannel);

impl Guest for CalendarChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Calendar channel config: {}", config_json),
        );

        let config: CalendarConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Calendar channel starting for CalDAV server: {}",
                config.caldav_url
            ),
        );

        // Store config
        let _ = channel_host::workspace_write("state/caldav_url", &config.caldav_url);

        let calendar_name = config.calendar_name.as_deref().unwrap_or("default");
        let _ = channel_host::workspace_write("state/calendar_name", calendar_name);

        // Discover principal URL
        let principal_url = discover_principal(&config.caldav_url)?;
        let _ = channel_host::workspace_write("state/principal_url", &principal_url);

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("CalDAV principal: {}", principal_url),
        );

        // Discover calendar home and find target calendar
        let calendar_home = discover_calendar_home(&config.caldav_url, &principal_url)?;
        let _ = channel_host::workspace_write("state/calendar_home", &calendar_home);

        let calendar_href = find_calendar(&config.caldav_url, &calendar_home, calendar_name)?;
        let _ = channel_host::workspace_write("state/calendar_href", &calendar_href);

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Using calendar: {}", calendar_href),
        );

        // Store initial sync token
        if let Some(sync_token) = get_sync_token(&config.caldav_url, &calendar_href) {
            let _ = channel_host::workspace_write("state/sync_token", &sync_token);
        }

        let poll_interval = config.poll_interval_ms.unwrap_or(60_000);

        Ok(ChannelConfig {
            display_name: "Calendar".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/calendar".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: Some(PollConfig {
                interval_ms: poll_interval,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        let caldav_url = match channel_host::workspace_read("state/caldav_url") {
            Some(url) => url,
            None => return,
        };
        let principal_url = match channel_host::workspace_read("state/principal_url") {
            Some(url) => url,
            None => return,
        };
        let calendar_href = match channel_host::workspace_read("state/calendar_href") {
            Some(href) => href,
            None => return,
        };

        let last_ctag = channel_host::workspace_read("state/ctag");

        // Check CTag to see if calendar changed
        let current_ctag = get_ctag(&caldav_url, &calendar_href);

        if let (Some(ref last), Some(ref current)) = (&last_ctag, &current_ctag) {
            if last == current {
                return; // No changes
            }
        }

        // Fetch events modified since last poll
        let events = fetch_events(&caldav_url, &calendar_href);

        let known_etags_json =
            channel_host::workspace_read("state/known_etags").unwrap_or_else(|| "{}".to_string());
        let mut known_etags: std::collections::HashMap<String, String> =
            serde_json::from_str(&known_etags_json).unwrap_or_default();

        for event in &events {
            // Check if this event is new or modified
            let is_new_or_modified = match known_etags.get(&event.href) {
                Some(old_etag) => event.etag.as_ref().is_none_or(|e| e != old_etag),
                None => true,
            };

            if !is_new_or_modified {
                continue;
            }

            // Update known etag
            if let Some(ref etag) = event.etag {
                known_etags.insert(event.href.clone(), etag.clone());
            }

            let metadata = CalendarEventMetadata {
                caldav_url: caldav_url.clone(),
                principal_url: principal_url.clone(),
                calendar_href: calendar_href.clone(),
                event_href: event.href.clone(),
                event_uid: event.uid.clone(),
                etag: event.etag.clone(),
            };

            let content = format_event_summary(event);

            channel_host::emit_message(&EmittedMessage {
                user_id: CHANNEL_NAME.to_string(),
                user_name: Some("Calendar".to_string()),
                content,
                thread_id: Some(event.uid.clone()),
                metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
                attachments: vec![],
            });
        }

        // Persist updated state
        if let Ok(etags_json) = serde_json::to_string(&known_etags) {
            let _ = channel_host::workspace_write("state/known_etags", &etags_json);
        }
        if let Some(ref ctag) = current_ctag {
            let _ = channel_host::workspace_write("state/ctag", ctag);
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: CalendarEventMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // The agent's response content is expected to be an iCalendar event
        // or a JSON instruction for calendar operations.
        // Try to parse as JSON instruction first.
        if let Ok(instruction) = serde_json::from_str::<serde_json::Value>(&response.content) {
            match instruction
                .get("action")
                .and_then(|a| a.as_str())
                .unwrap_or("")
            {
                "create" => {
                    let ical = instruction
                        .get("ical")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'ical' field for create action")?;
                    let uid = instruction
                        .get("uid")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&metadata.event_uid);
                    let href = format!("{}{}.ics", metadata.calendar_href, uid);
                    put_event(&metadata.caldav_url, &href, ical, None)?;
                    Ok(())
                }
                "update" => {
                    let ical = instruction
                        .get("ical")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'ical' field for update action")?;
                    put_event(
                        &metadata.caldav_url,
                        &metadata.event_href,
                        ical,
                        metadata.etag.as_deref(),
                    )?;
                    Ok(())
                }
                "delete" => {
                    delete_event(
                        &metadata.caldav_url,
                        &metadata.event_href,
                        metadata.etag.as_deref(),
                    )?;
                    Ok(())
                }
                _ => {
                    // Treat as a text reply — no calendar action needed
                    Ok(())
                }
            }
        } else {
            // Plain text response — no calendar action
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
            "Calendar channel shutting down",
        );
    }
}

// ============================================================================
// CalDAV Discovery
// ============================================================================

fn discover_principal(caldav_url: &str) -> Result<String, String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:current-user-principal/>
  </d:prop>
</d:propfind>"#;

    let resp = caldav_request("PROPFIND", &format!("{}/", caldav_url), body, Some("0"))?;

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

fn discover_calendar_home(caldav_url: &str, principal_url: &str) -> Result<String, String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <c:calendar-home-set/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(caldav_url, principal_url);
    let resp = caldav_request("PROPFIND", &url, body, Some("0"))?;

    if resp.status != 207 {
        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "PROPFIND for calendar-home failed ({}): {}",
            resp.status, body_str
        ));
    }

    let body_str = String::from_utf8_lossy(&resp.body);
    extract_xml_href(&body_str, "calendar-home-set")
        .ok_or_else(|| "Could not find calendar-home-set".to_string())
}

fn find_calendar(
    caldav_url: &str,
    calendar_home: &str,
    calendar_name: &str,
) -> Result<String, String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <d:displayname/>
    <d:resourcetype/>
    <cs:getctag/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(caldav_url, calendar_home);
    let resp = caldav_request("PROPFIND", &url, body, Some("1"))?;

    if resp.status != 207 {
        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "PROPFIND for calendars failed ({}): {}",
            resp.status, body_str
        ));
    }

    let body_str = String::from_utf8_lossy(&resp.body);

    // Parse multistatus response to find matching calendar
    let mut best_match: Option<String> = None;
    let mut default_calendar: Option<String> = None;

    for response_block in body_str.split("<d:response>").skip(1) {
        let href = match extract_tag_content(response_block, "d:href") {
            Some(h) => h,
            None => continue,
        };

        // Skip the calendar home itself
        if href.trim_end_matches('/') == calendar_home.trim_end_matches('/') {
            continue;
        }

        // Check if this is a calendar resource
        if !response_block.contains("calendar") {
            continue;
        }

        let display_name = extract_tag_content(response_block, "d:displayname");

        if let Some(ref name) = display_name {
            if name.eq_ignore_ascii_case(calendar_name) {
                best_match = Some(href.clone());
                break;
            }
        }

        if default_calendar.is_none() {
            default_calendar = Some(href);
        }
    }

    // Also check with lowercase tag variants (some servers use different prefixes)
    if best_match.is_none() && default_calendar.is_none() {
        for response_block in body_str.split("<D:response>").skip(1) {
            let href = match extract_tag_content(response_block, "D:href") {
                Some(h) => h,
                None => continue,
            };

            if response_block.contains("calendar") {
                let display_name = extract_tag_content(response_block, "D:displayname");

                if let Some(ref name) = display_name {
                    if name.eq_ignore_ascii_case(calendar_name) {
                        best_match = Some(href.clone());
                        break;
                    }
                }

                if default_calendar.is_none() {
                    default_calendar = Some(href);
                }
            }
        }
    }

    best_match
        .or(default_calendar)
        .ok_or_else(|| format!("No calendar found matching '{}'", calendar_name))
}

// ============================================================================
// CalDAV Operations
// ============================================================================

fn get_ctag(caldav_url: &str, calendar_href: &str) -> Option<String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <cs:getctag/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(caldav_url, calendar_href);
    let resp = caldav_request("PROPFIND", &url, body, Some("0")).ok()?;
    if resp.status != 207 {
        return None;
    }
    let body_str = String::from_utf8_lossy(&resp.body);
    extract_tag_content(&body_str, "cs:getctag")
        .or_else(|| extract_tag_content(&body_str, "getctag"))
}

fn get_sync_token(caldav_url: &str, calendar_href: &str) -> Option<String> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:sync-token/>
  </d:prop>
</d:propfind>"#;

    let url = resolve_url(caldav_url, calendar_href);
    let resp = caldav_request("PROPFIND", &url, body, Some("0")).ok()?;
    if resp.status != 207 {
        return None;
    }
    let body_str = String::from_utf8_lossy(&resp.body);
    extract_tag_content(&body_str, "d:sync-token")
        .or_else(|| extract_tag_content(&body_str, "sync-token"))
}

fn fetch_events(caldav_url: &str, calendar_href: &str) -> Vec<CalendarEvent> {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:getetag/>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT"/>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#;

    let url = resolve_url(caldav_url, calendar_href);
    let resp = match caldav_report(&url, body) {
        Ok(r) => r,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("CalDAV REPORT failed: {}", e),
            );
            return vec![];
        }
    };

    if resp.status != 207 {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("CalDAV REPORT returned {}", resp.status),
        );
        return vec![];
    }

    let body_str = String::from_utf8_lossy(&resp.body);
    parse_calendar_multistatus(&body_str)
}

fn put_event(
    caldav_url: &str,
    event_href: &str,
    ical_data: &str,
    etag: Option<&str>,
) -> Result<(), String> {
    let url = resolve_url(caldav_url, event_href);

    let mut headers = serde_json::json!({
        "Authorization": "Basic {CALENDAR_AUTH_TOKEN}",
        "Content-Type": "text/calendar; charset=utf-8",
    });

    if let Some(etag) = etag {
        headers["If-Match"] = serde_json::json!(etag);
    }

    let resp = channel_host::http_request(
        "PUT",
        &url,
        &headers.to_string(),
        Some(ical_data.as_bytes()),
        None,
    )?;

    if resp.status >= 200 && resp.status < 300 {
        Ok(())
    } else {
        let body = String::from_utf8_lossy(&resp.body);
        Err(format!("PUT event failed ({}): {}", resp.status, body))
    }
}

fn delete_event(caldav_url: &str, event_href: &str, etag: Option<&str>) -> Result<(), String> {
    let url = resolve_url(caldav_url, event_href);

    let mut headers = serde_json::json!({
        "Authorization": "Basic {CALENDAR_AUTH_TOKEN}",
    });

    if let Some(etag) = etag {
        headers["If-Match"] = serde_json::json!(etag);
    }

    let resp = channel_host::http_request("DELETE", &url, &headers.to_string(), None, None)?;

    if resp.status >= 200 && resp.status < 300 {
        Ok(())
    } else {
        let body = String::from_utf8_lossy(&resp.body);
        Err(format!("DELETE event failed ({}): {}", resp.status, body))
    }
}

// ============================================================================
// iCalendar Parsing
// ============================================================================

fn parse_calendar_multistatus(xml: &str) -> Vec<CalendarEvent> {
    let mut events = Vec::new();

    // Split on response elements (handle both d: and D: prefixes)
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
        let ical = extract_tag_content(block, "c:calendar-data")
            .or_else(|| extract_tag_content(block, "C:calendar-data"))
            .or_else(|| extract_tag_content(block, "cal:calendar-data"));

        let href = match href {
            Some(h) => h,
            None => continue,
        };
        let ical = match ical {
            Some(i) => i,
            None => continue,
        };

        let uid = extract_ical_property(&ical, "UID").unwrap_or_default();
        if uid.is_empty() {
            continue;
        }

        let summary =
            extract_ical_property(&ical, "SUMMARY").unwrap_or_else(|| "(no title)".to_string());
        let dtstart = extract_ical_property(&ical, "DTSTART");
        let dtend = extract_ical_property(&ical, "DTEND");
        let description = extract_ical_property(&ical, "DESCRIPTION");
        let location = extract_ical_property(&ical, "LOCATION");

        events.push(CalendarEvent {
            href,
            etag,
            uid,
            summary,
            dtstart,
            dtend,
            description,
            location,
            _ical_data: ical,
        });
    }

    events
}

fn extract_ical_property(ical: &str, property: &str) -> Option<String> {
    for line in ical.lines() {
        let trimmed = line.trim();
        // Handle properties with parameters like DTSTART;VALUE=DATE:20240101
        if let Some(rest) = trimmed.strip_prefix(property) {
            if let Some(value) = rest.strip_prefix(':') {
                return Some(value.to_string());
            } else if rest.starts_with(';') {
                // Has parameters — find the colon
                if let Some(colon_pos) = rest.find(':') {
                    return Some(rest[colon_pos + 1..].to_string());
                }
            }
        }
    }
    None
}

fn format_event_summary(event: &CalendarEvent) -> String {
    let mut parts = vec![format!("Calendar Event: {}", event.summary)];

    if let Some(ref start) = event.dtstart {
        parts.push(format!("Start: {}", format_ical_datetime(start)));
    }
    if let Some(ref end) = event.dtend {
        parts.push(format!("End: {}", format_ical_datetime(end)));
    }
    if let Some(ref location) = event.location {
        if !location.is_empty() {
            parts.push(format!("Location: {}", location));
        }
    }
    if let Some(ref description) = event.description {
        if !description.is_empty() {
            parts.push(format!("\n{}", description));
        }
    }

    parts.join("\n")
}

fn format_ical_datetime(dt: &str) -> String {
    // Convert 20240315T140000Z -> 2024-03-15 14:00:00 UTC
    if dt.len() >= 15 {
        let formatted = format!(
            "{}-{}-{} {}:{}:{}",
            &dt[0..4],
            &dt[4..6],
            &dt[6..8],
            &dt[9..11],
            &dt[11..13],
            &dt[13..15],
        );
        if dt.ends_with('Z') {
            format!("{} UTC", formatted)
        } else {
            formatted
        }
    } else if dt.len() == 8 {
        // Date only: 20240315 -> 2024-03-15
        format!("{}-{}-{}", &dt[0..4], &dt[4..6], &dt[6..8])
    } else {
        dt.to_string()
    }
}

// ============================================================================
// HTTP/XML Helpers
// ============================================================================

fn caldav_request(
    method: &str,
    url: &str,
    body: &str,
    depth: Option<&str>,
) -> Result<channel_host::HttpResponse, String> {
    let mut headers = serde_json::json!({
        "Authorization": "Basic {CALENDAR_AUTH_TOKEN}",
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

fn caldav_report(url: &str, body: &str) -> Result<channel_host::HttpResponse, String> {
    let headers = serde_json::json!({
        "Authorization": "Basic {CALENDAR_AUTH_TOKEN}",
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
    // Look for <parent_tag><d:href>...</d:href></parent_tag>
    // or <parent_tag><href>...</href></parent_tag>
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
