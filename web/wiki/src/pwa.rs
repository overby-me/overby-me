//! PWA install/offline (#33) and native notifications (#139).
//!
//! dx only serves assets referenced via `asset!()` (hashed URLs), so the icon
//! and service worker are referenced that way; the web manifest is built at
//! runtime (with the hashed icon URL) and linked via a blob URL.

use dioxus::prelude::*;

const ICON: Asset = asset!("/assets/icon.svg");
/// The service worker is served from the site ROOT (`/sw.js`) — copied there by
/// the `wiki-frontend` Nix package's install phase — rather than via
/// `asset!()` (which hashes it under `/assets/`, limiting its scope to
/// `/assets/`). A worker registered at `/sw.js` gets scope `/` by default, so it
/// controls the whole app (`/`, `/wasm/*`) for offline use.
const SW_URL: &str = "/sw.js";

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

    // Service worker at the site root, so its scope is `/` and it can serve the
    // whole app offline after a first online visit (see sw.js). In `dx serve` dev
    // `/sw.js` isn't present, so registration simply no-ops there.
    let _ = window.navigator().service_worker().register(SW_URL);
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

// --- background Web Push (#128) ---------------------------------------------

/// The server's VAPID public key (raw uncompressed P-256 point) as the
/// `applicationServerKey` for `pushManager.subscribe`. Its private half lives only
/// in the backend (Scaleway secret `VAPID_PRIVATE_KEY`).
const VAPID_PUBLIC_BYTES: [u8; 65] = [
    4, 75, 232, 81, 110, 46, 34, 121, 174, 49, 219, 87, 161, 174, 66, 111, 159, 96, 36, 159, 232,
    42, 226, 49, 55, 49, 205, 165, 103, 68, 214, 72, 180, 139, 52, 120, 27, 64, 95, 203, 236, 250,
    146, 23, 33, 4, 45, 31, 154, 254, 222, 178, 83, 233, 115, 21, 12, 134, 96, 254, 20, 49, 195,
    128, 137,
];

fn js_err(e: wasm_bindgen::JsValue) -> String {
    e.as_string().unwrap_or_else(|| "js error".to_string())
}

/// Whether this browser exposes the Push API at all (absent on e.g. non-installed
/// iOS Safari), so the UI can hide the notifications toggle where it can't work.
pub fn push_supported() -> bool {
    web_sys::window()
        .map(|w| js_sys::Reflect::has(&w, &"PushManager".into()).unwrap_or(false))
        .unwrap_or(false)
}

fn reflect_str(obj: &wasm_bindgen::JsValue, key: &str) -> Result<String, String> {
    js_sys::Reflect::get(obj, &key.into())
        .ok()
        .and_then(|v| v.as_string())
        .ok_or_else(|| format!("missing {key}"))
}

/// `PushSubscription.toJSON()` (endpoint + base64url `keys`), invoked dynamically
/// since web-sys 0.3.98 doesn't bind the method directly.
fn subscription_json(sub: &web_sys::PushSubscription) -> Result<wasm_bindgen::JsValue, String> {
    use wasm_bindgen::JsCast;
    let obj: &wasm_bindgen::JsValue = sub.as_ref();
    let func = js_sys::Reflect::get(obj, &"toJSON".into())
        .map_err(js_err)?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "toJSON not callable".to_string())?;
    func.call0(obj).map_err(js_err)
}

/// The active service worker registration (resolves once the worker is ready).
async fn registration() -> Result<web_sys::ServiceWorkerRegistration, String> {
    use wasm_bindgen::JsCast;
    let win = web_sys::window().ok_or("no window")?;
    let ready = win.navigator().service_worker().ready().map_err(js_err)?;
    wasm_bindgen_futures::JsFuture::from(ready)
        .await
        .map_err(js_err)?
        .dyn_into::<web_sys::ServiceWorkerRegistration>()
        .map_err(|_| "no service worker registration".to_string())
}

/// Whether this browser currently holds a push subscription. Used to render the
/// notifications toggle in the right state.
pub async fn push_subscribed() -> bool {
    use wasm_bindgen::JsCast;
    let Ok(reg) = registration().await else {
        return false;
    };
    let Ok(pm) = reg.push_manager() else {
        return false;
    };
    let Ok(promise) = pm.get_subscription() else {
        return false;
    };
    match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(v) => v.dyn_into::<web_sys::PushSubscription>().is_ok(),
        Err(_) => false,
    }
}

/// Subscribe this browser to background Web Push and register the subscription
/// with the backend (keyed by the caller's `token`). Requires (and requests)
/// notification permission.
pub async fn subscribe_push(token: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    // A push subscription with `userVisibleOnly` needs notification permission.
    if web_sys::Notification::permission() != web_sys::NotificationPermission::Granted {
        let win = web_sys::window().ok_or("no window")?;
        let perm = web_sys::Notification::request_permission().map_err(js_err)?;
        let _ = wasm_bindgen_futures::JsFuture::from(perm).await;
        let _ = win; // keep the window alive across the await
        if web_sys::Notification::permission() != web_sys::NotificationPermission::Granted {
            return Err("notifications not permitted".into());
        }
    }

    let reg = registration().await?;
    let pm = reg.push_manager().map_err(js_err)?;

    let opts = web_sys::PushSubscriptionOptionsInit::new();
    opts.set_user_visible_only(true);
    let key = js_sys::Uint8Array::from(&VAPID_PUBLIC_BYTES[..]);
    opts.set_application_server_key(key.as_ref());

    let promise = pm.subscribe_with_options(&opts).map_err(js_err)?;
    let sub = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_err)?
        .dyn_into::<web_sys::PushSubscription>()
        .map_err(|_| "no subscription".to_string())?;

    // `toJSON()` gives the endpoint + base64url keys the backend expects.
    let json = subscription_json(&sub)?;
    let endpoint = reflect_str(&json, "endpoint")?;
    let keys = js_sys::Reflect::get(&json, &"keys".into()).map_err(js_err)?;
    let p256dh = reflect_str(&keys, "p256dh")?;
    let auth = reflect_str(&keys, "auth")?;

    crate::backend_api::push_subscribe(token, &endpoint, &p256dh, &auth).await
}

/// Remove this browser's push subscription (both locally and at the backend).
pub async fn unsubscribe_push(token: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let reg = registration().await?;
    let pm = reg.push_manager().map_err(js_err)?;
    let promise = pm.get_subscription().map_err(js_err)?;
    let value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_err)?;
    let Ok(sub) = value.dyn_into::<web_sys::PushSubscription>() else {
        return Ok(()); // nothing to remove
    };
    let endpoint = reflect_str(&subscription_json(&sub)?, "endpoint")?;
    if let Ok(p) = sub.unsubscribe() {
        let _ = wasm_bindgen_futures::JsFuture::from(p).await;
    }
    crate::backend_api::push_unsubscribe(token, &endpoint).await
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
