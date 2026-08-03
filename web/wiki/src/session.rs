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

/// A pending membership claim token from a `?claim=<token>` link, stashed at
/// startup so it survives a login redirect; consumed once the user is signed in
/// (see `App`). `None` when there is nothing to claim.
pub static PENDING_CLAIM: GlobalSignal<Option<String>> = Signal::global(|| None);

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
///
/// It also opens with the answer this read gave last time, so a view that has
/// just been mounted draws immediately instead of holding a spinner for a round
/// trip (see [`crate::query_cache`], and the dependency note below). The value
/// is a `Signal<Option<T>>` rather than a `Resource<T>`, which reads the same
/// way: `.read()` and `.peek()` both give `Option<T>`.
///
/// **Dependencies are part of the cache key**, including the access token, so no
/// two identities ever share an answer. A dependency that is only a refresh
/// counter is in there too, which is harmless but worth knowing: after a
/// mutation bumps one, the next fresh mount opens on the answer from before the
/// mutation and corrects it a round trip later.
#[macro_export]
macro_rules! use_data_resource {
    (|($($dep:ident),* $(,)?)| $body:expr) => {{
        let __data_version = $crate::session::DATA_VERSION();
        let __site = concat!(file!(), ":", line!());
        let __key = $crate::query_cache::key(__site, &format!("{:?}", ($(&$dep,)*)));
        let __res = use_resource(use_reactive!(|($($dep,)* __data_version)| {
            let _ = __data_version;
            // Stamped with the key it was started under. The resource keeps its
            // previous value while re-running, so without this a dependency
            // change would file the outgoing answer under the incoming key.
            let __stamp = $crate::query_cache::key(__site, &format!("{:?}", ($(&$dep,)*)));
            // Built HERE, not inside the block below: the call site's own
            // clones have to happen in the closure, or its captures would be
            // moved out of an FnMut.
            let __fut = $body;
            async move { (__stamp, __fut.await) }
        }));
        $crate::query_cache::use_cached(__key, __res)
    }};
    // NOT cached, and cannot be: this form takes its dependencies from inside
    // the closure, so there is nothing here to key an answer by. Keyed only by
    // call site, one profile would open on another's data and a search on the
    // previous query's results. Use the dependency form above for a read that
    // should open on its last answer.
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
///
/// This is per-TAB: it is a static in one wasm instance, and every tab has its
/// own. Tabs share the token through localStorage but not this flag, so two of
/// them can still present the same token at once — see [`ROTATION_GRACE_MS`].
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// How long a rejected refresh waits for another tab to publish the token it
/// rotated, before accepting the session is really over.
///
/// NHost rotates on use: of two tabs presenting the same token, the first is
/// renewed and the second is REJECTED. The loser recovers by adopting whatever
/// the winner stored — but only if the winner has stored it yet. Both requests
/// went out together, so both responses land together, and which handler runs
/// first is a coin toss. Losing it signed the reader out of a session that was
/// perfectly alive, and cleared the storage the winner was about to write to.
///
/// Generous next to the gap it covers (two responses and a synchronous write),
/// and it costs nothing except delaying a sign-out that is genuinely due.
const ROTATION_GRACE_MS: u32 = 1200;
const ROTATION_POLL_MS: u32 = 50;
/// Looking more than ONCE is the whole fix — a single check is what signed
/// people out, because it could run before the winner had written anything.
const ROTATION_ATTEMPTS: u32 = ROTATION_GRACE_MS / ROTATION_POLL_MS;

/// Whether storage now holds a DIFFERENT refresh token than the one just
/// rejected — i.e. another tab rotated it and this one should adopt rather than
/// sign out.
///
/// No stored token is not a rotation: another tab signing out leaves nothing to
/// adopt, and this one should follow it out rather than resurrect the session.
fn is_rotation(tried: &str, stored: Option<&str>) -> bool {
    stored.is_some_and(|t| t != tried)
}

/// Set by the `visibilitychange` listener when the tab becomes visible again,
/// consumed by the refresh loop to force an immediate renewal.
static VISIBILITY_NUDGE: AtomicBool = AtomicBool::new(false);

/// Milliseconds since the Unix epoch (browser clock).
fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Whether the page is backgrounded.
fn page_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// Absolute expiry (ms epoch) for a token that lasts `expires_in` seconds.
pub fn expires_at_from(expires_in: Option<i64>) -> Option<f64> {
    expires_in.map(|secs| now_ms() + secs as f64 * 1000.0)
}

/// The client-to-server clock offset in ms (how far the server clock is ahead of
/// this device's). Derived for free from the access token: the JWT's server-issued
/// `exp` claim minus the client-computed `access_token_expires_at` (both captured at
/// token receipt) cancels the token lifetime and leaves the clock skew. Used to
/// align the speaker-list countdown across devices (the countdown is computed from a
/// DB timestamp, so a device with a wrong clock would otherwise drift). Returns 0
/// when the token or expiry is missing or unparsable, i.e. the previous
/// device-clock behaviour.
pub fn server_clock_offset_ms() -> f64 {
    let s = SESSION.read();
    let (Some(token), Some(expires_at)) = (s.access_token.as_ref(), s.access_token_expires_at)
    else {
        return 0.0;
    };
    match jwt_exp_ms(token) {
        Some(exp_ms) => exp_ms - expires_at,
        None => 0.0,
    }
}

/// The `exp` claim (ms epoch) from a JWT's payload segment, or None if it cannot be
/// decoded. Only reads the standard `exp` number; the signature is not verified (the
/// server does that), this is purely to read the server's notion of time.
fn jwt_exp_ms(token: &str) -> Option<f64> {
    let payload = token.split('.').nth(1)?;
    let bytes = b64url_decode(payload)?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(|e| e.as_f64()).map(|s| s * 1000.0)
}

/// Minimal base64url decoder (no padding required), enough to read a JWT payload
/// without pulling in a base64 crate.
fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let mut buf = 0u32;
    let mut bits = 0u8;
    let mut out = Vec::new();
    for c in input.bytes() {
        if c == b'=' {
            break;
        }
        buf = (buf << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
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
    /// Renewed (a fresh access token was stored).
    Renewed,
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
            RefreshOutcome::Renewed
        }
        Err(err) if nhost::is_auth_error(&err) => {
            // Another tab may have rotated the refresh token out from under us
            // (NHost invalidates the old token on refresh, and localStorage is
            // shared). If the stored token now differs from the one we tried,
            // adopt it and retry rather than logging the user out everywhere.
            //
            // Checked NOW and then again for a moment: the winning tab may not
            // have written its new token yet, and a rejection that merely
            // arrived first is not evidence of anything. Storage is re-read each
            // pass, so the winner's write is seen as soon as it lands. The
            // single-flight flag stays held throughout, which is what we want —
            // this tab must not start another refresh with the dead token.
            for _ in 0..ROTATION_ATTEMPTS {
                let stored = stored_session();
                if is_rotation(
                    &refresh_token,
                    stored.as_ref().and_then(|s| s.refresh_token.as_deref()),
                ) {
                    log::info!("adopting refresh token rotated by another tab");
                    if let Some(stored) = stored {
                        *SESSION.write() = stored;
                    }
                    return RefreshOutcome::Transient;
                }
                gloo_timers::future::TimeoutFuture::new(ROTATION_POLL_MS).await;
            }
            // Nobody rotated it in all that time, so the refresh token itself is
            // dead: clear the session and let the UI fall back to the login
            // screen instead of looping on a bad token.
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
        RefreshOutcome::Renewed | RefreshOutcome::Transient => SESSION.peek().access_token.clone(),
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
        // Never start one on the way into the background. NHost rotates the
        // token server-side before it answers, so a request iOS kills in flight
        // leaves this tab holding a dead one, and the next attempt signs a
        // perfectly good session out. The nudge covers coming back, and
        // `ensure_fresh_token` covers anything that needs a token before then.
        if !nudged && page_hidden() {
            continue;
        }
        if nudged || due_to_refresh() {
            refresh_access_token().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{b64url_decode, is_rotation, jwt_exp_ms, ROTATION_ATTEMPTS, ROTATION_GRACE_MS};

    #[test]
    fn b64url_decodes_without_padding() {
        assert_eq!(b64url_decode("aGVsbG8").as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn jwt_exp_reads_expiry_in_ms() {
        // Payload {"exp":1700000000,"sub":"x"}; signature is irrelevant here.
        // The base64url payload is split into short fragments so this dummy
        // token never looks like a credential to git secret scanners.
        let token = concat!(
            "aaa.",
            "eyJleHAiOjE3M",
            "DAwMDAwMDAsIn",
            "N1YiI6IngifQ",
            ".sig"
        );
        assert_eq!(jwt_exp_ms(token), Some(1_700_000_000_000.0));
    }

    #[test]
    fn jwt_exp_is_none_for_garbage() {
        assert_eq!(jwt_exp_ms("not-a-jwt"), None);
        assert_eq!(jwt_exp_ms("a.b.c"), None);
    }

    /// The three ways a refresh can be rejected, and what each means.
    #[test]
    fn a_rejection_is_only_a_rotation_when_storage_moved_on() {
        // Another tab refreshed first and published the token it got back. The
        // session is alive; adopt it instead of signing the reader out.
        assert!(is_rotation("old-token", Some("new-token")));

        // Storage still holds the very token that was just refused, so nobody
        // rotated anything and it really is dead.
        assert!(!is_rotation("dead-token", Some("dead-token")));

        // Another tab signed out and cleared storage. There is nothing to adopt,
        // and resurrecting a session it just ended would be wrong.
        assert!(!is_rotation("dead-token", None));
    }

    /// The predicate above was always right; what signed people out was asking
    /// it once, before the winning tab had written anything. The waiting itself
    /// needs a browser to exercise, so this pins the part that can be checked
    /// here: that there IS a retry, over a window wide enough to cover two
    /// responses landing together.
    #[test]
    fn a_rejected_refresh_looks_more_than_once() {
        assert!(
            ROTATION_ATTEMPTS > 1,
            "a single look is the bug, not the fix"
        );
        assert!(ROTATION_GRACE_MS >= 1000, "too tight to cover the race");
    }
}
