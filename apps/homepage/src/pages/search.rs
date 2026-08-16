use dioxus::prelude::*;

#[component]
pub fn Search(url: String) -> Element {
    use_effect(move || {
        let url = url.clone();
        let redirect = if let Some(caps) = extract_query(&url) {
            format!("https://startpage.com/search?q={caps}")
        } else {
            "https://startpage.com".to_string()
        };
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href(&redirect);
        }
    });
    rsx! {}
}

fn extract_query(url: &str) -> Option<String> {
    let idx = url.find("q=")?;
    let rest = &url[idx + 2..];
    let end = rest.find('&').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}
