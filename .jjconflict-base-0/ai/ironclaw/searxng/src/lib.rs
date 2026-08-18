//! SearXNG Web Search WASM Tool for IronClaw.
//!
//! Searches the web using a SearXNG instance and returns structured results.
//! SearXNG is a privacy-respecting, self-hostable meta search engine.
//!
//! No API key required — SearXNG instances are typically open.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "wit/tool.wit",
});

use serde::Deserialize;

const MAX_COUNT: u32 = 20;
const DEFAULT_COUNT: u32 = 5;
const MAX_RETRIES: u32 = 3;

struct SearxngTool;

impl exports::near::agent::tool::Guest for SearxngTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Search the web using SearXNG, a privacy-respecting meta search engine. \
         Returns titles, URLs, and descriptions for matching web pages. \
         Supports filtering by category, language, and time range. \
         Requires the 'instance_url' parameter pointing to a SearXNG instance."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    instance_url: String,
    query: String,
    count: Option<u32>,
    categories: Option<String>,
    language: Option<String>,
    time_range: Option<String>,
    safesearch: Option<u8>,
    pageno: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    engine: Option<String>,
    #[serde(default)]
    engines: Vec<String>,
    #[serde(rename = "publishedDate")]
    published_date: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: SearchParams =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if params.query.is_empty() {
        return Err("'query' must not be empty".into());
    }
    if params.query.len() > 2000 {
        return Err("'query' exceeds maximum length of 2000 characters".into());
    }
    if params.instance_url.is_empty() {
        return Err("'instance_url' must not be empty".into());
    }

    // Validate time_range
    if let Some(ref tr) = params.time_range {
        if !matches!(tr.as_str(), "day" | "week" | "month" | "year") {
            return Err(format!(
                "Invalid 'time_range': expected 'day', 'week', 'month', or 'year', got '{tr}'"
            ));
        }
    }

    // Validate safesearch
    if let Some(ss) = params.safesearch {
        if ss > 2 {
            return Err(format!(
                "Invalid 'safesearch': expected 0, 1, or 2, got '{ss}'"
            ));
        }
    }

    let count = params.count.unwrap_or(DEFAULT_COUNT).clamp(1, MAX_COUNT);
    let url = build_search_url(&params);

    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-SearXNG-Tool/0.1"
    });

    // Retry loop for transient errors
    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let resp =
                near::agent::host::http_request("GET", &url, &headers.to_string(), None, None)
                    .map_err(|e| format!("HTTP request failed: {e}"))?;

            if resp.status >= 200 && resp.status < 300 {
                break resp;
            }

            if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!(
                        "SearXNG error {} (attempt {}/{}). Retrying...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body = String::from_utf8_lossy(&resp.body);
            return Err(format!("SearXNG error (HTTP {}): {}", resp.status, body));
        }
    };

    let body =
        String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let searxng_response: SearxngResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse SearXNG response: {e}"))?;

    let results: Vec<serde_json::Value> = searxng_response
        .results
        .into_iter()
        .take(count as usize)
        .filter_map(|r| {
            let title = r.title?;
            let url = r.url?;
            let description = r.content.unwrap_or_default();

            let mut entry = serde_json::json!({
                "title": title,
                "url": url,
                "description": description,
            });
            if let Some(date) = r.published_date {
                entry["published"] = serde_json::json!(date);
            }
            if !r.engines.is_empty() {
                entry["engines"] = serde_json::json!(r.engines);
            } else if let Some(engine) = r.engine {
                entry["engines"] = serde_json::json!([engine]);
            }
            if let Some(host) = extract_hostname(&url) {
                entry["site_name"] = serde_json::json!(host);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "query": params.query,
        "result_count": results.len(),
        "results": results,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn build_search_url(params: &SearchParams) -> String {
    let base = params.instance_url.trim_end_matches('/');
    let mut url = format!(
        "{}/search?q={}&format=json",
        base,
        url_encode(&params.query),
    );

    if let Some(ref categories) = params.categories {
        url.push_str(&format!("&categories={}", url_encode(categories)));
    }
    if let Some(ref language) = params.language {
        url.push_str(&format!("&language={}", url_encode(language)));
    }
    if let Some(ref time_range) = params.time_range {
        url.push_str(&format!("&time_range={}", url_encode(time_range)));
    }
    if let Some(safesearch) = params.safesearch {
        url.push_str(&format!("&safesearch={}", safesearch));
    }
    if let Some(pageno) = params.pageno {
        url.push_str(&format!("&pageno={}", pageno));
    }

    url
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(char::from(HEX_CHARS[(b >> 4) as usize]));
                out.push(char::from(HEX_CHARS[(b & 0xf) as usize]));
            }
        }
    }
    out
}

const HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";

fn extract_hostname(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = after_scheme.split('/').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "instance_url": {
            "type": "string",
            "description": "The SearXNG instance URL (e.g. 'https://search.example.com')"
        },
        "query": {
            "type": "string",
            "description": "The search query to look up on the web"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-20, default 5)",
            "minimum": 1,
            "maximum": 20,
            "default": 5
        },
        "categories": {
            "type": "string",
            "description": "Comma-separated search categories (e.g. 'general', 'images', 'news', 'science', 'it')"
        },
        "language": {
            "type": "string",
            "description": "Language code for search results (e.g. 'en', 'de', 'fr')"
        },
        "time_range": {
            "type": "string",
            "description": "Filter by time: 'day', 'week', 'month', or 'year'"
        },
        "safesearch": {
            "type": "integer",
            "description": "Safe search filter level: 0 (off), 1 (moderate), 2 (strict)",
            "minimum": 0,
            "maximum": 2
        },
        "pageno": {
            "type": "integer",
            "description": "Page number for pagination (default 1)",
            "minimum": 1
        }
    },
    "required": ["instance_url", "query"],
    "additionalProperties": false
}"#;

export!(SearxngTool);
