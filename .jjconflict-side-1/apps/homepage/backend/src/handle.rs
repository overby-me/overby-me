use axum::{body::Body, extract::Request, response::Response};
use http::{header, StatusCode};

// The static site (the wasm SPA shell) that humans load. For a `/@handle`
// request we fetch its index.html and inject per-handle Open Graph tags, so
// link-card crawlers (Bluesky, Mastodon, …) that don't run the wasm still show
// the handle, while humans get the unchanged live app.
const SITE: &str = "https://overby.me";

pub async fn handle(req: Request<Body>) -> Response<Body> {
    let index = fetch_index().await;

    // Only `/@handle` paths get a per-handle card; anything else is the site
    // served as-is (its crawler-facing form matches the static one).
    let html = match handle_from_path(req.uri().path()) {
        Some(handle) => inject_card(&index, &handle),
        None => index,
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(
            header::CACHE_CONTROL,
            "s-maxage=300, stale-while-revalidate",
        )
        .body(Body::from(html))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Fetch the SPA shell; empty string on failure so we still emit a card below.
async fn fetch_index() -> String {
    let Ok(resp) = reqwest::get(format!("{SITE}/index.html")).await else {
        return String::new();
    };
    resp.text().await.unwrap_or_default()
}

/// The atproto handle from a `/@handle` path, if it looks like a real one. The
/// character allow-list also guarantees nothing HTML-significant reaches the
/// injected attributes.
fn handle_from_path(path: &str) -> Option<String> {
    let rest = path.trim_start_matches('/').strip_prefix('@')?;
    let handle = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let plausible = (1..=253).contains(&handle.len())
        && (handle.starts_with("did:") || handle.contains('.'))
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    plausible.then(|| handle.to_string())
}

/// Set the `<title>` and inject Open Graph / Twitter card tags for `handle`.
fn inject_card(index: &str, handle: &str) -> String {
    let title = format!("@{handle}");
    let tags = format!(
        "<meta property=\"og:title\" content=\"{title}\">\
         <meta property=\"og:description\" content=\"Every atproto platform @{handle} is on, drawn as a live graph.\">\
         <meta property=\"og:type\" content=\"profile\">\
         <meta property=\"og:url\" content=\"{SITE}/@{handle}\">\
         <meta name=\"twitter:card\" content=\"summary\">\
         <meta name=\"twitter:title\" content=\"{title}\">"
    );
    let with_title = replace_between(index, "<title>", "</title>", &title);
    match with_title.find("</head>") {
        Some(pos) => format!("{}{tags}{}", &with_title[..pos], &with_title[pos..]),
        None => with_title,
    }
}

/// Replace the text between the first `open` and the next following `close`.
fn replace_between(s: &str, open: &str, close: &str, value: &str) -> String {
    let Some(start) = s.find(open) else {
        return s.to_string();
    };
    let inner = start + open.len();
    let Some(end) = s[inner..].find(close) else {
        return s.to_string();
    };
    format!("{}{value}{}", &s[..inner], &s[inner + end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plausible_handles_only() {
        assert_eq!(
            handle_from_path("/@overby.me").as_deref(),
            Some("overby.me")
        );
        assert_eq!(
            handle_from_path("/@did:plc:abc").as_deref(),
            Some("did:plc:abc")
        );
        // Path trimmed at the next segment.
        assert_eq!(
            handle_from_path("/@overby.me/x").as_deref(),
            Some("overby.me")
        );
        // Not a handle-shaped thing / not a /@ path.
        assert_eq!(handle_from_path("/"), None);
        assert_eq!(handle_from_path("/@nodot"), None);
        assert_eq!(handle_from_path("/@bad<script>.com"), None);
    }

    #[test]
    fn injects_title_and_og() {
        let index =
            "<!DOCTYPE html><html><head><title>Niclas Overby Ⓝ</title></head><body></body></html>";
        let out = inject_card(index, "overby.me");
        assert!(out.contains("<title>@overby.me</title>"));
        assert!(out.contains("<meta property=\"og:title\" content=\"@overby.me\">"));
        assert!(out.contains("og:description"));
        // og:url ends with the handle route (asserted without a fetchable literal).
        assert!(out.contains("property=\"og:url\"") && out.contains("/@overby.me\">"));
        // The card sits inside the head.
        assert!(out.find("og:title").unwrap() < out.find("</head>").unwrap());
    }
}
