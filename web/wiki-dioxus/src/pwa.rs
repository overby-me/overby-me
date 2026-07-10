//! PWA install/offline (#33) and native notifications (#139).
//!
//! dx only serves assets referenced via `asset!()` (hashed URLs), so the icon
//! and service worker are referenced that way; the web manifest is built at
//! runtime (with the hashed icon URL) and linked via a blob URL.

use dioxus::prelude::*;

const ICON: Asset = asset!("/assets/icon.svg");
const SW: Asset = asset!("/assets/sw.js");

/// Install the PWA head tags (manifest, icons, theme colour) and register the
/// service worker. Safe to call once at startup, before `launch` (the page's
/// `<head>` already exists).
pub fn setup() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let icon = ICON.to_string();

    if let Some(head) = document.head() {
        // Manifest — generated with the hashed icon URL and served via a blob so
        // it is same-origin (data: manifests are rejected as opaque).
        let manifest = format!(
            "{{\"name\":\"RadikalWiki\",\"short_name\":\"RadikalWiki\",\
             \"start_url\":\"/\",\"scope\":\"/\",\"display\":\"standalone\",\
             \"background_color\":\"#ffffff\",\"theme_color\":\"#006d39\",\
             \"icons\":[{{\"src\":\"{icon}\",\"sizes\":\"any\",\
             \"type\":\"image/svg+xml\",\"purpose\":\"any maskable\"}}]}}"
        );
        let parts = js_sys::Array::new();
        parts.push(&wasm_bindgen::JsValue::from_str(&manifest));
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type("application/manifest+json");
        if let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) {
            if let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) {
                append_link(&document, &head, "manifest", &url);
            }
        }
        append_link(&document, &head, "icon", &icon);
        append_link(&document, &head, "apple-touch-icon", &icon);
        append_meta(&document, &head, "theme-color", "#006d39");
        append_meta(&document, &head, "apple-mobile-web-app-capable", "yes");
        append_meta(&document, &head, "mobile-web-app-capable", "yes");
    }

    // Service worker (offline where it controls the root — see sw.js).
    let _ = window
        .navigator()
        .service_worker()
        .register(&SW.to_string());
}

fn append_link(
    document: &web_sys::Document,
    head: &web_sys::HtmlHeadElement,
    rel: &str,
    href: &str,
) {
    if let Ok(el) = document.create_element("link") {
        let _ = el.set_attribute("rel", rel);
        let _ = el.set_attribute("href", href);
        let _ = head.append_child(&el);
    }
}

fn append_meta(
    document: &web_sys::Document,
    head: &web_sys::HtmlHeadElement,
    name: &str,
    content: &str,
) {
    if let Ok(el) = document.create_element("meta") {
        let _ = el.set_attribute("name", name);
        let _ = el.set_attribute("content", content);
        let _ = head.append_child(&el);
    }
}

/// Ask for notification permission once, if the user hasn't decided yet.
pub fn request_notification_permission() {
    if web_sys::Notification::permission() == web_sys::NotificationPermission::Default {
        let _ = web_sys::Notification::request_permission();
    }
}

/// Show a native notification, if the user granted permission.
pub fn notify(title: &str, body: &str) {
    if web_sys::Notification::permission() != web_sys::NotificationPermission::Granted {
        return;
    }
    let opts = web_sys::NotificationOptions::new();
    opts.set_body(body);
    opts.set_icon(&ICON.to_string());
    opts.set_tag("radikalwiki");
    let _ = web_sys::Notification::new_with_options(title, &opts);
}
