//! The data layer on the atproto AppView (roadmap M9).
//!
//! A second implementation of what `crate::graphql` is to the components: the
//! same questions, answered by `crates/appview` through its generated client
//! (`appview_client`) and handed over as the same `crate::model` types. Only
//! built under the `appview` feature; what ships is unchanged until the cutover.

// Until the switch: the layer is written a part at a time, and nothing calls a
// part before the whole stands in for `crate::graphql`. Goes with the switch.
#![allow(dead_code)]

pub mod api;
mod ballot;
mod bin;
mod canvas;
pub mod map;
mod nodes;
mod people;
mod reports;
mod screen;
mod seen;
mod speak;
mod talk;
mod vote;

#[allow(unused_imports)]
pub use bin::*;
#[allow(unused_imports)]
pub use canvas::*;
#[allow(unused_imports)]
pub use nodes::*;
#[allow(unused_imports)]
pub use people::*;
#[allow(unused_imports)]
pub use reports::*;
#[allow(unused_imports)]
pub use screen::*;
#[allow(unused_imports)]
pub use speak::*;
#[allow(unused_imports)]
pub use talk::*;
#[allow(unused_imports)]
pub use vote::*;

use appview_client::{Client, Error};

/// Where the AppView is. Set at build time, as the interim's endpoints are
/// (`WIKI_APPVIEW_URL`); unset, a dev instance on this machine.
pub const APPVIEW_URL: &str = match option_env!("WIKI_APPVIEW_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:8080",
};

pub fn appview_url() -> String {
    #[cfg(test)]
    if let Some(url) = tests::URL.with(|url| url.borrow().clone()) {
        return url;
    }
    APPVIEW_URL.trim_end_matches('/').to_string()
}

/// How long to wait before asking a read again that got no answer. A venue's
/// wifi drops for a moment far more often than for a minute.
pub(crate) const RETRY_DELAYS_MS: &[u32] = &[300, 900];

/// Where signing in as `handle` starts. The browser comes back to the page it
/// left, with `#code=<code>` for `createSession`.
pub fn login_url(handle: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    let here = web_sys::window()
        .and_then(|w| w.location().href().ok())
        .unwrap_or_default();
    #[cfg(not(target_arch = "wasm32"))]
    let here = String::new();
    let here = here.split_once('#').map_or(here.as_str(), |(page, _)| page);
    client(None).login_url(handle, here)
}

/// Remember the answer to a read, for the tunnel. In a browser only: under
/// `cargo test` there is no storage to remember it in.
pub(crate) fn remember<T: serde::Serialize>(key: &str, value: &T) {
    #[cfg(target_arch = "wasm32")]
    crate::offline::put(key, value);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (key, serde_json::to_string(value));
}

/// The AppView, as whoever holds `access_token`, or as nobody.
pub(crate) fn client(access_token: Option<&str>) -> Client {
    let client = Client::new(&appview_url());
    match access_token.filter(|token| !token.is_empty()) {
        Some(token) => client.with_session(token),
        None => client,
    }
}

thread_local! {
    /// Whose each session is, as the AppView said when first asked.
    static HOLDERS: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The DID a session is for. Asked once per session and remembered: it does
/// not change, and a session says nothing about its holder by itself.
pub(crate) async fn whoami(access_token: &str) -> Option<String> {
    if let Some(did) = HOLDERS.with(|holders| holders.borrow().get(access_token).cloned()) {
        return Some(did);
    }
    let client = client(Some(access_token));
    let me = ask_quiet(true, || client.get_session()).await.ok()?;
    HOLDERS.with(|holders| {
        holders
            .borrow_mut()
            .insert(access_token.to_string(), me.did.clone());
    });
    Some(me.did)
}

/// A failure in the words `crate::errors::classify` sorts by. "No" and "not
/// there" are one answer here, as they are from the AppView, which tells a
/// stranger that what they may not read does not exist.
pub(crate) fn said(error: &Error) -> String {
    match error {
        Error::Transport(e) => format!("error sending request: {e}"),
        Error::Api {
            status,
            error,
            message,
        } => match status {
            401 | 403 | 404 => format!("not allowed: {error}: {message}"),
            408 | 429 | 500..=599 => format!("http {status} instead of an answer: {error}"),
            _ => format!("{error}: {message}"),
        },
        Error::Decode(e) => format!("not what the lexicon says: {e}"),
    }
}

/// Whether the AppView's answer was that there is no such thing, or none for
/// this reader: an empty result to a read, and no fault.
pub(crate) fn is_absent(error: &Error) -> bool {
    matches!(error, Error::Api { status: 404, .. })
}

/// The remembered answer to a read, if the failure was the kind a copy answers.
/// A refusal must not fall back: serving what someone could read yesterday would
/// be the app overriding a permission change made since.
pub(crate) fn offline_copy<T: serde::de::DeserializeOwned>(key: &str, error: &str) -> Option<T> {
    if crate::errors::classify(error) != crate::errors::Failure::Offline {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let copy = crate::offline::get::<T>(key)?;
        crate::errors::report_offline_copy();
        Some(copy)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

/// Run one call, and let its failure be known as `graphql::execute` does: noted
/// for the record, classified, logged at the level its class deserves, and told
/// to the reader only if it is theirs to care about. A read that never got an
/// answer is asked again; a write is not, since no answer is not "not done".
pub(crate) async fn ask<T, F, Fut>(what: &'static str, read: bool, call: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
{
    ask_quiet(read, call)
        .await
        .map_err(|error| reported(what, &error))
}

/// [`ask`] for a refusal the CALLER expects and handles, which is then neither
/// shown to the reader nor logged as a fault. The error comes back whole, so
/// the caller can tell which refusal it was, and pass any other to [`reported`].
pub(crate) async fn ask_quiet<T, F, Fut>(read: bool, mut call: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
{
    let mut result = call().await;
    if read {
        for delay in RETRY_DELAYS_MS {
            // No answer, or a gateway's instead of one: the same to a reader.
            let unanswered = result.as_ref().is_err_and(|error| {
                crate::errors::classify(&said(error)) == crate::errors::Failure::Offline
            });
            if !unanswered {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(*delay).await;
            result = call().await;
        }
    }
    result
}

/// Make a failure known, and hand back the words for it.
pub(crate) fn reported(what: &'static str, error: &Error) -> String {
    let message = said(error);
    let failure = crate::errors::classify(&message);
    match failure {
        crate::errors::Failure::Broken => log::error!("appview error [{what}]: {message}"),
        _ => log::info!("appview {} [{what}]: {message}", failure.label()),
    }
    // The record and the toast are the browser's: a clock and a screen.
    #[cfg(target_arch = "wasm32")]
    {
        crate::errors::note_failure(format!("[{what}] {message}"));
        crate::errors::report(failure);
    }
    message
}

#[cfg(test)]
mod live;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{classify, Failure};

    thread_local! {
        /// Where the AppView is, for the test running on this thread
        /// (`live.rs` starts one each).
        pub(crate) static URL: std::cell::RefCell<Option<String>> =
            const { std::cell::RefCell::new(None) };
    }

    #[test]
    fn a_failure_is_said_in_the_words_it_is_sorted_by() {
        let api = |status: u16, error: &str| Error::Api {
            status,
            error: error.to_string(),
            message: "why".to_string(),
        };
        for (error, class) in [
            (Error::Transport("dns".into()), Failure::Offline),
            (api(404, "NotFound"), Failure::Refused),
            (api(403, "Forbidden"), Failure::Refused),
            (api(401, "AuthRequired"), Failure::Refused),
            (api(502, "InternalError"), Failure::Offline),
            (api(429, "TooSoon"), Failure::Offline),
            (api(409, "PathTaken"), Failure::Broken),
            (Error::Decode("missing field".into()), Failure::Broken),
        ] {
            assert_eq!(classify(&said(&error)), class, "{error}");
        }
    }
}
