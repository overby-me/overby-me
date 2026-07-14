use std::sync::atomic::{AtomicBool, Ordering};

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::nhost;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    /// The user's avatar URL (e.g. their linked Bluesky picture); empty if none.
    #[serde(default)]
    pub avatar_url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub user: Option<User>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub node_id: Option<String>,
    /// When the access token expires, in ms since the Unix epoch. `None` for
    /// sessions persisted before this field existed (treated as "refresh now").
    #[serde(default)]
    pub access_token_expires_at: Option<f64>,
}

impl Session {
    pub fn is_authenticated(&self) -> bool {
        self.user.is_some() && self.access_token.is_some()
    }
}

pub static SESSION: GlobalSignal<Session> = Signal::global(Session::default);

/// Bumped after a mutation so cached data queries refetch instead of serving a
/// stale result. `use_resource` only re-runs when its dependencies change, so a
/// query that reads this in its `use_reactive` deps refetches on every bump.
///
/// This is one app-wide counter: a bump refetches EVERY mounted `use_data_resource!`
/// at once (coarse but correct). Scoping it per-context is deliberately deferred —
/// see `docs/data-version-invalidation.md` for the mechanism, the load-bearing
/// cross-view-consistency constraint, and a low-regression scoping design.
pub static DATA_VERSION: GlobalSignal<u32> = Signal::global(|| 0);

/// Invalidate cached data queries; call after a successful mutation so views
/// showing the changed node refetch instead of staying stale until a reload.
pub fn bump_data_version() {
    *DATA_VERSION.write() += 1;
}

/// A `use_resource` that also refetches whenever the global data version bumps
/// (any mutation via [`bump_data_version`], or a pull-to-refresh), so one refresh
/// updates the whole view. It mirrors the two `use_resource` idioms, folding the
/// data version in for you either way, so call sites never repeat it:
///
/// - `use_data_resource!(|(a, b)| async move { … })` — explicit reactive
///   dependencies, exactly like `use_reactive!`.
/// - `use_data_resource!(move || { …; async move { … } })` — a plain closure that
///   captures its own dependencies / reads signals inside.
///
/// Use it for every read-side data resource so a refresh works everywhere.
#[macro_export]
macro_rules! use_data_resource {
    (|($($dep:ident),* $(,)?)| $body:expr) => {{
        let __data_version = $crate::session::DATA_VERSION();
        use_resource(use_reactive!(|($($dep,)* __data_version)| {
            let _ = __data_version;
            $body
        }))
    }};
    (move || $body:expr) => {
        use_resource(move || {
            // Subscribe this resource to the data version so it refetches on a
            // global refresh, alongside the closure's own dependencies.
            let _ = $crate::session::DATA_VERSION();
            $body
        })
    };
}

pub fn use_session() -> Signal<Session> {
    SESSION.signal()
}

/// Load session from localStorage on startup
pub fn load_session() {
    if let Ok(Some(json)) = web_sys_storage() {
        if let Ok(session) = serde_json::from_str::<Session>(&json) {
            *SESSION.write() = session;
        }
    }
}

/// Save session to localStorage
pub fn save_session(session: &Session) {
    if let Ok(json) = serde_json::to_string(session) {
        let _ = set_web_sys_storage(&json);
    }
}

/// Establish a session from a bare refresh token — the one embedded in an NHost
/// password-reset email link (`/?type=passwordReset&refreshToken=...`). Populates
/// and persists `SESSION` so the set-password form has a valid access token.
/// Returns whether the exchange succeeded.
pub async fn establish_from_refresh_token(refresh_token: &str) -> bool {
    match nhost::refresh_session(refresh_token).await {
        Ok(new) => {
            let snapshot = {
                let mut session = SESSION.write();
                session.access_token = Some(new.access_token);
                session.refresh_token = Some(new.refresh_token);
                session.access_token_expires_at = expires_at_from(new.access_token_expires_in);
                if let Some(user) = new.user {
                    session.user = Some(User {
                        id: user.id,
                        email: user.email.unwrap_or_default(),
                        display_name: user.display_name.unwrap_or_default(),
                        avatar_url: user.avatar_url.unwrap_or_default(),
                    });
                }
                session.clone()
            };
            save_session(&snapshot);
            true
        }
        Err(err) => {
            log::warn!("password-reset token exchange failed: {err}");
            false
        }
    }
}

fn web_sys_storage() -> Result<Option<String>, ()> {
    let window = web_sys::window().ok_or(())?;
    let storage = window.local_storage().map_err(|_| ())?.ok_or(())?;
    storage.get_item("wiki_session").map_err(|_| ())
}

fn set_web_sys_storage(value: &str) -> Result<(), ()> {
    let window = web_sys::window().ok_or(())?;
    let storage = window.local_storage().map_err(|_| ())?.ok_or(())?;
    storage.set_item("wiki_session", value).map_err(|_| ())
}

/// Read and parse the persisted session straight from localStorage (the value
/// shared across tabs), bypassing the in-memory `SESSION`.
fn stored_session() -> Option<Session> {
    web_sys_storage()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<Session>(&json).ok())
}

// ---------------------------------------------------------------------------
// Access-token refresh
//
// The NHost access token (JWT) is short-lived (~15 min). Nothing used to renew
// it, so the session silently died after the token's lifetime and after the tab
// had been backgrounded past expiry. `run_token_refresh` (driven by a
// `use_future` in `App`) keeps it alive: it refreshes once on startup (swapping
// out a possibly-stale stored token), then again shortly before each expiry, and
// immediately when the tab regains visibility.
// ---------------------------------------------------------------------------

/// Guards against two overlapping refreshes racing on the same (rotating)
/// refresh token, which NHost would reject.
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Set by the `visibilitychange` listener when the tab becomes visible again,
/// consumed by the refresh loop to force an immediate renewal.
static VISIBILITY_NUDGE: AtomicBool = AtomicBool::new(false);

/// Milliseconds since the Unix epoch (browser clock).
fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Absolute expiry (ms epoch) for a token that lasts `expires_in` seconds.
pub fn expires_at_from(expires_in: Option<i64>) -> Option<f64> {
    expires_in.map(|secs| now_ms() + secs as f64 * 1000.0)
}

/// Signal from the visibility listener that the tab just became visible.
pub fn nudge_refresh() {
    VISIBILITY_NUDGE.store(true, Ordering::SeqCst);
}

fn take_visibility_nudge() -> bool {
    VISIBILITY_NUDGE.swap(false, Ordering::SeqCst)
}

/// Result of a single refresh attempt.
enum RefreshOutcome {
    /// Renewed; carries the new token lifetime in seconds.
    Renewed(i64),
    /// No stored refresh token (not logged in).
    NoSession,
    /// Another refresh was already running; nothing to do.
    InFlight,
    /// Refresh token was rejected; the session has been cleared.
    Expired,
    /// Transient failure (network); safe to retry later.
    Transient,
}

/// Exchange the stored refresh token for a fresh access token and persist it.
async fn refresh_access_token() -> RefreshOutcome {
    // `peek` (not `read`): this runs in a background loop that must never
    // subscribe to SESSION, or a refresh's own write could restart the loop.
    let Some(refresh_token) = SESSION.peek().refresh_token.clone() else {
        return RefreshOutcome::NoSession;
    };

    // Single-flight: bail if a refresh is already running. The guard resets the
    // flag even if this future is dropped or panics mid-flight.
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return RefreshOutcome::InFlight;
    }
    struct ResetOnDrop;
    impl Drop for ResetOnDrop {
        fn drop(&mut self) {
            REFRESHING.store(false, Ordering::SeqCst);
        }
    }
    let _reset = ResetOnDrop;

    match nhost::refresh_session(&refresh_token).await {
        Ok(new) => {
            let snapshot = {
                let mut session = SESSION.write();
                session.access_token = Some(new.access_token);
                session.refresh_token = Some(new.refresh_token);
                session.access_token_expires_at = expires_at_from(new.access_token_expires_in);
                if let Some(user) = new.user {
                    session.user = Some(User {
                        id: user.id,
                        email: user.email.unwrap_or_default(),
                        display_name: user.display_name.unwrap_or_default(),
                        avatar_url: user.avatar_url.unwrap_or_default(),
                    });
                }
                session.clone()
            };
            save_session(&snapshot);
            RefreshOutcome::Renewed(new.access_token_expires_in.unwrap_or(900))
        }
        Err(err) if nhost::is_auth_error(&err) => {
            // Another tab may have rotated the refresh token out from under us
            // (NHost invalidates the old token on refresh, and localStorage is
            // shared). If the stored token now differs from the one we tried,
            // adopt it and retry rather than logging the user out everywhere.
            if let Some(stored) = stored_session() {
                if stored
                    .refresh_token
                    .as_deref()
                    .is_some_and(|t| t != refresh_token)
                {
                    log::info!("adopting refresh token rotated by another tab");
                    *SESSION.write() = stored;
                    return RefreshOutcome::Transient;
                }
            }
            // The refresh token itself is dead, so clear the session and let the
            // UI fall back to the login screen instead of looping on a bad token.
            log::warn!("session refresh rejected, signing out: {err}");
            *SESSION.write() = Session::default();
            save_session(&Session::default());
            RefreshOutcome::Expired
        }
        Err(err) => {
            log::warn!("session refresh failed (will retry): {err}");
            RefreshOutcome::Transient
        }
    }
}

/// Ensure a fresh access token for retrying a request that failed with an expired
/// JWT (e.g. a tab returning after its token lapsed while backgrounded). Refreshes
/// now, or waits for a refresh already in flight, then returns the current access
/// token. `None` when signed out or the refresh token itself is dead.
pub async fn ensure_fresh_token() -> Option<String> {
    use gloo_timers::future::TimeoutFuture;
    match refresh_access_token().await {
        RefreshOutcome::Renewed(_) | RefreshOutcome::Transient => {
            SESSION.peek().access_token.clone()
        }
        RefreshOutcome::InFlight => {
            // The background loop (or a sibling query) is already refreshing; wait
            // for it to land rather than starting a second, racing refresh.
            for _ in 0..50 {
                TimeoutFuture::new(100).await;
                if !REFRESHING.load(Ordering::SeqCst) {
                    break;
                }
            }
            SESSION.peek().access_token.clone()
        }
        RefreshOutcome::NoSession | RefreshOutcome::Expired => None,
    }
}

/// Long-running refresh loop. Never returns; intended to be owned by a
/// `use_future` in the root component so its `SESSION` writes run inside the
/// Dioxus runtime.
pub async fn run_token_refresh() {
    use gloo_timers::future::TimeoutFuture;

    // Renew this long before the token expires, and re-check often enough that a
    // refocused (previously throttled) tab reacts promptly. `SESSION`'s stored
    // expiry is the single source of truth, so refreshes and tokens adopted from
    // other tabs are reflected automatically.
    const BUFFER_MS: f64 = 120_000.0;
    const CHUNK_MS: u32 = 45_000;

    let due_to_refresh = || {
        let session = SESSION.peek();
        session.refresh_token.is_some()
            && session
                .access_token_expires_at
                .is_none_or(|exp| now_ms() >= exp - BUFFER_MS)
    };

    // On startup, refresh only if the stored token is missing/expired. A token
    // still comfortably valid is used as-is, avoiding a redundant refresh (and
    // the double data-fetch it would trigger via access-token-keyed queries).
    if due_to_refresh() {
        refresh_access_token().await;
    }

    loop {
        TimeoutFuture::new(CHUNK_MS).await;

        let nudged = take_visibility_nudge();
        if SESSION.peek().refresh_token.is_none() {
            continue; // Signed out; idle until a session appears again.
        }
        if nudged || due_to_refresh() {
            refresh_access_token().await;
        }
    }
}
