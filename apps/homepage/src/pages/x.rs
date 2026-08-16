use dioxus::prelude::*;

#[component]
pub fn X(url: String) -> Element {
    use_effect(move || {
        let url = url.clone();
        if let Some(path) = extract_path(&url)
            && let Some(window) = web_sys::window()
        {
            let redirect = format!("https://xcancel.com{path}");
            let _ = window.location().set_href(&redirect);
        }
    });
    rsx! {}
}

fn extract_path(url: &str) -> Option<String> {
    // Match x.com or twitter.com and extract the path after the domain
    let url_lower = url.to_lowercase();
    for domain in &["x.com", "twitter.com"] {
        if let Some(idx) = url_lower.find(domain) {
            let after_domain = &url[idx + domain.len()..];
            return Some(after_domain.to_string());
        }
    }
    None
}
