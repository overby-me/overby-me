//! The atproto OAuth slice in the AppView: the thin `atrium-oauth` wrapper
//! (proven PDS-agnostic in `crates/oauth-spike`), now completed with the two
//! pieces the spike left open (`docs/atproto-stack-decisions.md`): DURABLE
//! SQLite-backed state/session stores (replacing atrium's in-memory `Memory*`),
//! and the interactive `/callback` that drives the token exchange.
//!
//! Identity is the top migration risk (0 DIDs linked), so this is the untested
//! half of the foundation the big-bang cutover assumes. What is agent-completable
//! and lives here: the stores, the wrapper's `callback`, and the HTTP handler,
//! all compiling and unit-covered. The one step that needs a human is a real
//! browser redirect against an independent PDS; see the run harness note below.
//!
//! ## The login, end to end
//!
//! 1. The frontend sends the browser to `/login?handle=<handle>&return=<url>`.
//!    [`login_handler`] runs the pre-redirect flow, binds the login to this
//!    browser with a cookie, and redirects to the account's own PDS.
//! 2. The PDS redirects back to `/callback?code=...&state=...&iss=...`.
//!    [`callback_handler`] checks the cookie, drives the token exchange, and
//!    returns to `<url>#code=<one-time code>`.
//! 3. The frontend POSTs the code to `com.example.wiki.createSession` and holds
//!    the bearer token it gets back (`crate::session`).
//!
//! Step 2 against a real PDS needs a human in a browser; everything around the
//! exchange is unit-covered here.

use crate::db::{Db, DbError};
use crate::http::{Reach, RustlsHttpClient};
use atrium_api::agent::SessionManager;
use atrium_api::types::string::Did;
use atrium_common::store::Store;
use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};
use atrium_identity::handle::{
    AtprotoHandleResolver, AtprotoHandleResolverConfig, DohDnsTxtResolver, DohDnsTxtResolverConfig,
};
use atrium_oauth::store::session::{Session, SessionStore};
use atrium_oauth::store::state::{InternalStateData, StateStore};
use atrium_oauth::{
    AtprotoClientMetadata, AtprotoLocalhostClientMetadata, AuthMethod, AuthorizeOptions,
    CallbackParams, GrantType, KnownScope, OAuthClient, OAuthClientConfig, OAuthResolverConfig,
    Scope,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Durable stores: atrium's `Store<K, V>` over a SQLite (Turso) JSON-KV table.
// ---------------------------------------------------------------------------

/// A store failure: a datastore error or a (de)serialization error. atrium's
/// `Store` trait requires the error to be `std::error::Error`.
#[derive(Debug)]
pub enum StoreError {
    Db(DbError),
    Turso(turso::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "store db error: {e}"),
            StoreError::Turso(e) => write!(f, "store query error: {e}"),
            StoreError::Json(e) => write!(f, "store json error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<DbError> for StoreError {
    fn from(e: DbError) -> Self {
        StoreError::Db(e)
    }
}
impl From<turso::Error> for StoreError {
    fn from(e: turso::Error) -> Self {
        StoreError::Turso(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

/// A durable string-keyed JSON key-value table backing the atrium OAuth stores.
/// The value column always holds `serde_json` of the atrium value type. `table`
/// and `key_col` are `'static` from this crate (never user input), so their
/// interpolation into SQL is safe.
#[derive(Clone)]
struct JsonKv {
    db: Db,
    table: &'static str,
    key_col: &'static str,
}

impl JsonKv {
    async fn get_json<V: DeserializeOwned>(&self, key: &str) -> Result<Option<V>, StoreError> {
        let conn = self.db.acquire().await?;
        let sql = format!(
            "SELECT value FROM {} WHERE {} = ?1",
            self.table, self.key_col
        );
        let mut rows = conn.query(&sql, [key]).await?;
        match rows.next().await? {
            Some(row) => {
                let json: String = row.get(0)?;
                Ok(Some(serde_json::from_str(&json)?))
            }
            None => Ok(None),
        }
    }

    async fn set_json<V: Serialize>(&self, key: &str, value: &V) -> Result<(), StoreError> {
        let json = serde_json::to_string(value)?;
        let conn = self.db.acquire().await?;
        // turso 0.2.2 has no upsert (ON CONFLICT/OR REPLACE unsupported), so
        // UPDATE-then-INSERT (same as the push seam in `store.rs`).
        let updated = conn
            .execute(
                &format!(
                    "UPDATE {} SET value = ?1 WHERE {} = ?2",
                    self.table, self.key_col
                ),
                [json.clone(), key.to_string()],
            )
            .await?;
        if updated == 0 {
            conn.execute(
                &format!(
                    "INSERT INTO {} ({}, value) VALUES (?1, ?2)",
                    self.table, self.key_col
                ),
                [key.to_string(), json],
            )
            .await?;
        }
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<(), StoreError> {
        let conn = self.db.acquire().await?;
        conn.execute(
            &format!("DELETE FROM {} WHERE {} = ?1", self.table, self.key_col),
            [key],
        )
        .await?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), StoreError> {
        let conn = self.db.acquire().await?;
        conn.execute(&format!("DELETE FROM {}", self.table), ())
            .await?;
        Ok(())
    }
}

/// Durable atrium `StateStore`: the pre-redirect PKCE/DPoP/issuer context keyed
/// by the OAuth `state` nonce, persisted in `oauth_state`.
#[derive(Clone)]
pub struct SqliteStateStore {
    kv: JsonKv,
}

impl SqliteStateStore {
    pub fn new(db: Db) -> Self {
        Self {
            kv: JsonKv {
                db,
                table: "oauth_state",
                key_col: "key",
            },
        }
    }
}

impl Store<String, InternalStateData> for SqliteStateStore {
    type Error = StoreError;

    async fn get(&self, key: &String) -> Result<Option<InternalStateData>, Self::Error> {
        self.kv.get_json(key).await
    }
    async fn set(&self, key: String, value: InternalStateData) -> Result<(), Self::Error> {
        self.kv.set_json(&key, &value).await
    }
    async fn del(&self, key: &String) -> Result<(), Self::Error> {
        self.kv.del(key).await
    }
    async fn clear(&self) -> Result<(), Self::Error> {
        self.kv.clear().await
    }
}

impl StateStore for SqliteStateStore {}

/// Durable atrium `SessionStore`: the post-exchange DPoP key + token set keyed
/// by the account DID, persisted in `oauth_session`.
#[derive(Clone)]
pub struct SqliteSessionStore {
    kv: JsonKv,
}

impl SqliteSessionStore {
    pub fn new(db: Db) -> Self {
        Self {
            kv: JsonKv {
                db,
                table: "oauth_session",
                key_col: "did",
            },
        }
    }
}

impl Store<Did, Session> for SqliteSessionStore {
    type Error = StoreError;

    async fn get(&self, key: &Did) -> Result<Option<Session>, Self::Error> {
        self.kv.get_json(key.as_str()).await
    }
    async fn set(&self, key: Did, value: Session) -> Result<(), Self::Error> {
        self.kv.set_json(key.as_str(), &value).await
    }
    async fn del(&self, key: &Did) -> Result<(), Self::Error> {
        self.kv.del(key.as_str()).await
    }
    async fn clear(&self) -> Result<(), Self::Error> {
        self.kv.clear().await
    }
}

impl SessionStore for SqliteSessionStore {}

// ---------------------------------------------------------------------------
// The OAuth client wrapper.
// ---------------------------------------------------------------------------

type HttpClient = RustlsHttpClient;
type DidRes = CommonDidResolver<HttpClient>;
type HandleRes = AtprotoHandleResolver<DohDnsTxtResolver<HttpClient>, HttpClient>;
type Client = OAuthClient<SqliteStateStore, SqliteSessionStore, DidRes, HandleRes, HttpClient>;

/// The result of a completed callback: the resolved account DID (if the session
/// exposes it) and the app-state the authorize call round-tripped.
pub struct CallbackOutcome {
    pub did: Option<String>,
    pub app_state: Option<String>,
}

fn scopes() -> Vec<Scope> {
    vec![
        Scope::Known(KnownScope::Atproto),
        Scope::Known(KnownScope::TransitionGeneric),
    ]
}

/// The wiki's atproto OAuth client, backed by durable SQLite stores. Construct
/// once (`new`), `begin_login` per member, `callback` on the redirect back.
pub struct WikiOAuth {
    client: Client,
    /// A second handle on the state store the client owns, so a callback can be
    /// checked against its login before the token exchange is spent on it.
    states: SqliteStateStore,
}

impl WikiOAuth {
    /// Build the OAuth client with durable stores over `db`. A configured
    /// `public_url` selects the production profile, whose `client_id` is the
    /// metadata document [`client_metadata_handler`] serves; without one this is
    /// a loopback dev client. Both are public clients (no secret, no JWKS).
    pub fn new(db: Db, config: &crate::Config) -> Result<Self, Box<dyn std::error::Error>> {
        // A dev instance has no public URL, and may need a PDS on localhost.
        let reach = if config.public_url.is_empty() {
            Reach::Any
        } else {
            Reach::PublicOnly
        };
        let http = RustlsHttpClient::new(reach)?;
        let http_client = Arc::new(http.clone());
        let resolver = OAuthResolverConfig {
            did_resolver: CommonDidResolver::new(CommonDidResolverConfig {
                plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_string(),
                http_client: Arc::clone(&http_client),
            }),
            handle_resolver: AtprotoHandleResolver::new(AtprotoHandleResolverConfig {
                dns_txt_resolver: DohDnsTxtResolver::new(DohDnsTxtResolverConfig {
                    service_url: String::from("https://cloudflare-dns.com/dns-query"),
                    http_client: Arc::clone(&http_client),
                }),
                http_client: Arc::clone(&http_client),
            }),
            authorization_server_metadata: Default::default(),
            protected_resource_metadata: Default::default(),
        };
        let states = SqliteStateStore::new(db.clone());
        let state_store = states.clone();
        let session_store = SqliteSessionStore::new(db);
        let client = if config.public_url.is_empty() {
            OAuthClient::new(OAuthClientConfig {
                client_metadata: AtprotoLocalhostClientMetadata {
                    // The port is part of the declared URI because atrium matches
                    // redirect URIs exactly; the PDS itself ignores a loopback port.
                    redirect_uris: Some(vec![format!("http://127.0.0.1:{}/callback", config.port)]),
                    scopes: Some(scopes()),
                },
                keys: None,
                resolver,
                state_store,
                session_store,
                http_client: http,
            })?
        } else {
            let base = &config.public_url;
            OAuthClient::new(OAuthClientConfig {
                client_metadata: AtprotoClientMetadata {
                    client_id: format!("{base}{CLIENT_METADATA_PATH}"),
                    client_uri: Some(base.clone()),
                    redirect_uris: vec![format!("{base}/callback")],
                    token_endpoint_auth_method: AuthMethod::None,
                    grant_types: vec![GrantType::AuthorizationCode, GrantType::RefreshToken],
                    scopes: scopes(),
                    jwks_uri: None,
                    token_endpoint_auth_signing_alg: None,
                },
                keys: None,
                resolver,
                state_store,
                session_store,
                http_client: http,
            })?
        };
        Ok(Self { client, states })
    }

    /// Begin login for a member identified by handle or PDS URL: the full
    /// server-side pre-redirect flow (resolution + PAR with a fresh DPoP key +
    /// PKCE), returning the authorization URL to redirect them to. `app_state`
    /// comes back from [`WikiOAuth::callback`].
    pub async fn begin_login(
        &self,
        handle_or_pds: &str,
        app_state: Option<String>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = self
            .client
            .authorize(
                handle_or_pds,
                AuthorizeOptions {
                    scopes: scopes(),
                    state: app_state,
                    ..Default::default()
                },
            )
            .await?;
        Ok(url)
    }

    /// The app-state of a login that has begun and not yet come back, by the
    /// OAuth `state` its callback carries.
    pub async fn pending_app_state(&self, state: &str) -> Option<String> {
        match Store::get(&self.states, &state.to_string()).await {
            Ok(data) => data.and_then(|d| d.app_state),
            Err(e) => {
                tracing::error!("oauth state lookup failed: {e}");
                None
            }
        }
    }

    /// Complete login from the callback query: drive the token exchange, persist
    /// the session (via the durable `SessionStore`), and report the resolved DID.
    pub async fn callback(
        &self,
        params: CallbackParams,
    ) -> Result<CallbackOutcome, Box<dyn std::error::Error>> {
        let (session, app_state) = self.client.callback(params).await?;
        let did = session.did().await.map(|d| d.as_str().to_string());
        Ok(CallbackOutcome { did, app_state })
    }

    /// The client metadata document a PDS fetches from the `client_id` URL.
    /// atrium's struct omits two fields the atproto profile lists as required.
    pub fn client_metadata(&self) -> serde_json::Value {
        let mut doc = serde_json::to_value(&self.client.client_metadata)
            .unwrap_or_else(|_| serde_json::json!({}));
        if let Some(map) = doc.as_object_mut() {
            map.insert("application_type".into(), "web".into());
            map.insert("response_types".into(), serde_json::json!(["code"]));
        }
        doc
    }
}

// ---------------------------------------------------------------------------
// The HTTP handlers.
// ---------------------------------------------------------------------------

use crate::session::Sessions;
use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::header::{COOKIE, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

pub const CLIENT_METADATA_PATH: &str = "/client-metadata.json";

const LOGIN_COOKIE: &str = "wiki_login";
const LOGIN_COOKIE_SECS: u64 = 600;

/// What a login carries across the PDS redirect, as atrium's app-state. It stays
/// server-side in `oauth_state`; the browser holds only `binder`, in a cookie.
#[derive(Serialize, Deserialize)]
struct LoginState {
    binder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    return_to: Option<String>,
}

fn bad_request(message: &str) -> Response {
    crate::xrpc::err(StatusCode::BAD_REQUEST, "InvalidRequest", message)
}

fn login_cookie(value: &str, max_age: u64, config: &crate::Config) -> String {
    // The AppView sits behind a TLS-terminating edge, so the public URL is the
    // only place it can learn that the browser is on https.
    let secure = if config.public_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{LOGIN_COOKIE}={value}; Max-Age={max_age}; Path=/callback; HttpOnly; SameSite=Lax{secure}"
    )
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

fn redirect(location: &str, set_cookie: &str) -> Response {
    let (Ok(location), Ok(set_cookie)) = (
        HeaderValue::from_str(location),
        HeaderValue::from_str(set_cookie),
    ) else {
        return bad_request("unusable redirect target");
    };
    let mut resp = StatusCode::SEE_OTHER.into_response();
    resp.headers_mut().insert(LOCATION, location);
    resp.headers_mut().insert(SET_COOKIE, set_cookie);
    resp
}

/// `GET /login?handle=<handle-or-pds>&return=<frontend url>`: start a login and
/// send the browser to the account's own PDS.
pub async fn login_handler(
    State(state): State<crate::AppState>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(oauth) = state.oauth.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "oauth not configured").into_response();
    };
    let pairs = crate::util::parse_query(query.as_deref());
    let get = |k: &str| pairs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
    let Some(handle) = get("handle").filter(|h| !h.is_empty()) else {
        return bad_request("missing handle");
    };
    let return_to = get("return");
    if let Some(url) = &return_to
        && !state.config.allows_return(url)
    {
        return bad_request("return is not an allowed frontend origin");
    }
    let binder = crate::util::random_token(16);
    let login = LoginState {
        binder: binder.clone(),
        return_to,
    };
    let Ok(app_state) = serde_json::to_string(&login) else {
        return bad_request("unusable login state");
    };
    match oauth.begin_login(&handle, Some(app_state)).await {
        Ok(url) => redirect(
            &url,
            &login_cookie(&binder, LOGIN_COOKIE_SECS, &state.config),
        ),
        Err(e) => {
            // Almost always a handle that does not resolve, which is the
            // caller's to fix, so this is not a 5xx.
            tracing::warn!("login could not start for {handle}: {e}");
            bad_request("could not start a login for that handle")
        }
    }
}

/// `GET /callback`: the PDS redirects the browser here with `code`/`state`/`iss`.
///
/// The cookie check comes before the exchange on purpose. Without it, a link to
/// an attacker's own finished consent would sign the victim's browser in as the
/// attacker (login CSRF), and everything they then wrote would land in the
/// attacker's account.
pub async fn callback_handler(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(oauth) = state.oauth.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "oauth not configured").into_response();
    };
    let pairs = crate::util::parse_query(query.as_deref());
    let get = |k: &str| pairs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
    let Some(code) = get("code") else {
        return bad_request("missing code");
    };
    let Some(oauth_state) = get("state") else {
        return bad_request("missing state");
    };
    let login = oauth
        .pending_app_state(&oauth_state)
        .await
        .and_then(|json| serde_json::from_str::<LoginState>(&json).ok());
    let Some(login) = login else {
        return bad_request("unknown or expired login");
    };
    if cookie(&headers, LOGIN_COOKIE) != Some(login.binder.as_str()) {
        return bad_request("this login was not started in this browser");
    }

    let params = CallbackParams {
        code,
        state: Some(oauth_state),
        iss: get("iss"),
    };
    let did = match oauth.callback(params).await {
        Ok(CallbackOutcome { did: Some(did), .. }) => did,
        Ok(_) => {
            tracing::error!("oauth callback resolved no DID");
            return (StatusCode::BAD_GATEWAY, "callback failed").into_response();
        }
        Err(e) => {
            tracing::error!("oauth callback failed: {e}");
            return (StatusCode::BAD_GATEWAY, "callback failed").into_response();
        }
    };
    finish_login(&state, &did, login.return_to.as_deref()).await
}

/// Record a completed login and hand the browser a one-time code for it.
async fn finish_login(state: &crate::AppState, did: &str, return_to: Option<&str>) -> Response {
    let issued = async {
        crate::Store::new(state.db.clone())
            .upsert_user_min(did)
            .await?;
        Sessions::new(state.db.clone()).issue_code(did).await
    };
    let code = match issued.await {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("could not record the login of {did}: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "login failed").into_response();
        }
    };
    let clear = login_cookie("", 0, &state.config);
    // Re-checked, not trusted: the allowlist may have shrunk since /login.
    match return_to.filter(|url| state.config.allows_return(url)) {
        Some(url) => {
            let base = url.split_once('#').map_or(url, |(base, _)| base);
            redirect(&format!("{base}#code={code}"), &clear)
        }
        None => {
            let mut resp = Json(serde_json::json!({ "did": did, "code": code })).into_response();
            if let Ok(clear) = HeaderValue::from_str(&clear) {
                resp.headers_mut().insert(SET_COOKIE, clear);
            }
            resp
        }
    }
}

/// `GET /client-metadata.json`: the document the production `client_id` names.
pub async fn client_metadata_handler(State(state): State<crate::AppState>) -> Response {
    match &state.oauth {
        Some(oauth) => Json(oauth.client_metadata()).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, "oauth not configured").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug, Clone)]
    struct Probe {
        a: String,
        n: u32,
    }

    async fn kv() -> JsonKv {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        JsonKv {
            db,
            table: "oauth_state",
            key_col: "key",
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn json_kv_roundtrips_upserts_and_clears() {
        let kv = kv().await;
        // Absent key -> None.
        assert!(kv.get_json::<Probe>("k").await.expect("get").is_none());
        // Set then get.
        let v = Probe {
            a: "x".into(),
            n: 1,
        };
        kv.set_json("k", &v).await.expect("set");
        assert_eq!(kv.get_json::<Probe>("k").await.expect("get").unwrap(), v);
        // Upsert overwrites (no ON CONFLICT: UPDATE path).
        kv.set_json(
            "k",
            &Probe {
                a: "y".into(),
                n: 2,
            },
        )
        .await
        .expect("upsert");
        assert_eq!(kv.get_json::<Probe>("k").await.expect("get").unwrap().n, 2);
        // Delete.
        kv.del("k").await.expect("del");
        assert!(kv.get_json::<Probe>("k").await.expect("get").is_none());
        // Clear.
        kv.set_json("a", &v).await.expect("set a");
        kv.set_json("b", &v).await.expect("set b");
        kv.clear().await.expect("clear");
        assert!(kv.get_json::<Probe>("a").await.expect("get").is_none());
    }

    // -- The login handlers. Nothing here touches the network: building the
    //    client is offline, and every case stops before the PDS is contacted. --

    use crate::{AppState, Config, router};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    const FRONTEND: &str = "https://wiki.example";

    async fn login_state(public_url: &str) -> AppState {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let config = Config {
            public_url: public_url.to_string(),
            frontend_origins: vec![FRONTEND.to_string()],
            ..Config::default()
        };
        let oauth = WikiOAuth::new(db.clone(), &config).expect("oauth client");
        AppState::new(db, config).with_oauth(Arc::new(oauth))
    }

    async fn send(state: &AppState, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
        let mut req = Request::builder().uri(uri);
        if let Some(c) = cookie {
            req = req.header("cookie", c);
        }
        let resp = router(state.clone())
            .oneshot(req.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A login that has begun and not come back, as `authorize()` leaves it. The
    /// key is the RFC 7517 appendix A.2 example; nothing signs with it here.
    async fn pending_login(state: &AppState, oauth_state: &str, login: &LoginState) {
        let data = serde_json::json!({
            "iss": "https://pds.example",
            "dpop_key": {
                "kty": "EC",
                "crv": "P-256",
                "x": "MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4",
                "y": "4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFyM",
                "d": "870MB6gfuTJ4HtUnUvYMyJpr5eUZNP4Bk43bVdj3eAE"
            },
            "verifier": "v",
            "app_state": serde_json::to_string(login).expect("json"),
        });
        let data: InternalStateData = serde_json::from_value(data).expect("state data");
        Store::set(
            &SqliteStateStore::new(state.db.clone()),
            oauth_state.to_string(),
            data,
        )
        .await
        .expect("set");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn login_refuses_a_missing_handle_and_a_foreign_return() {
        let state = login_state("").await;
        let (status, body) = send(&state, "/login", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let (status, body) = send(
            &state,
            "/login?handle=alice.test&return=https%3A%2F%2Fevil.example%2F",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("allowed frontend origin"), "{body}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_callback_for_no_known_login_is_refused() {
        let state = login_state("").await;
        let (status, body) = send(&state, "/callback?code=c&state=nope", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unknown or expired login"), "{body}");
    }

    /// Login CSRF: the attacker finishes consent for their OWN account and gets
    /// the victim's browser to open the callback. That browser never started the
    /// login, so it has no binder cookie, or has the binder of another login.
    #[tokio::test(flavor = "current_thread")]
    async fn a_callback_from_a_browser_that_did_not_start_the_login_is_refused() {
        let state = login_state("").await;
        let login = LoginState {
            binder: "attackers-binder".into(),
            return_to: None,
        };
        pending_login(&state, "st1", &login).await;
        for cookie in [None, Some("wiki_login=some-other-login")] {
            let (status, body) = send(&state, "/callback?code=c&state=st1", cookie).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "cookie {cookie:?}");
            assert!(body.contains("not started in this browser"), "{body}");
        }
        let oauth = state.oauth.as_ref().expect("oauth");
        assert!(
            oauth.pending_app_state("st1").await.is_some(),
            "a refused callback must not spend the login it was aimed at"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_finished_login_returns_a_redeemable_code_to_the_frontend() {
        let state = login_state("https://api.wiki.example").await;
        let resp = finish_login(
            &state,
            "did:plc:alice",
            Some("https://wiki.example/a/b?app=vote#stale"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp.headers()[LOCATION].to_str().expect("location");
        let (base, code) = location.split_once("#code=").expect("a code fragment");
        assert_eq!(base, "https://wiki.example/a/b?app=vote");
        let cleared = resp.headers()[SET_COOKIE].to_str().expect("cookie");
        assert!(cleared.contains("Max-Age=0"), "{cleared}");
        assert!(cleared.contains("; Secure"), "{cleared}");

        let did = Sessions::new(state.db.clone())
            .redeem_code(code)
            .await
            .expect("redeem");
        assert_eq!(did.as_deref(), Some("did:plc:alice"));
        let user = crate::Store::new(state.db.clone())
            .read_user("did:plc:alice")
            .await
            .expect("read");
        assert!(user.is_some(), "a login must leave a user row behind");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_finished_login_never_redirects_off_the_allowlist() {
        let state = login_state("").await;
        for return_to in [None, Some("https://evil.example/")] {
            let resp = finish_login(&state, "did:plc:alice", return_to).await;
            assert_eq!(resp.status(), StatusCode::OK, "return_to {return_to:?}");
            assert!(resp.headers().get(LOCATION).is_none());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_production_profile_serves_its_own_client_id() {
        let state = login_state("https://api.wiki.example").await;
        let (status, body) = send(&state, CLIENT_METADATA_PATH, None).await;
        assert_eq!(status, StatusCode::OK);
        let doc: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            doc["client_id"],
            "https://api.wiki.example/client-metadata.json"
        );
        assert_eq!(
            doc["redirect_uris"],
            serde_json::json!(["https://api.wiki.example/callback"])
        );
        assert_eq!(doc["dpop_bound_access_tokens"], true);
        assert_eq!(doc["token_endpoint_auth_method"], "none");
        assert_eq!(doc["response_types"], serde_json::json!(["code"]));
        assert_eq!(doc["application_type"], "web");
        assert_eq!(doc["scope"], "atproto transition:generic");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_loopback_profile_redirects_to_the_port_it_listens_on() {
        let state = login_state("").await;
        let oauth = state.oauth.as_ref().expect("oauth");
        let doc = oauth.client_metadata();
        assert_eq!(
            doc["redirect_uris"],
            serde_json::json!(["http://127.0.0.1:8080/callback"])
        );
        let client_id = doc["client_id"].as_str().expect("client_id");
        assert!(client_id.starts_with("http://localhost?"), "{client_id}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn atrium_stores_implement_the_trait() {
        // Exercises the atrium `Store` trait impls end-to-end on empty tables
        // (no value construction needed): a get on an absent key deserializes to
        // None through the real `InternalStateData`/`Session` types, and
        // del/clear round-trip. This is the compile-and-behaviour proof that the
        // durable stores satisfy `StateStore`/`SessionStore`.
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");

        let ss = SqliteStateStore::new(db.clone());
        assert!(
            Store::get(&ss, &"nonce".to_string())
                .await
                .expect("state get")
                .is_none()
        );
        Store::del(&ss, &"nonce".to_string())
            .await
            .expect("state del");
        Store::clear(&ss).await.expect("state clear");

        let sess = SqliteSessionStore::new(db);
        let did = Did::new("did:plc:z72i7hdynmk6r22z27h6tvur".to_string()).expect("valid did");
        assert!(
            Store::get(&sess, &did)
                .await
                .expect("session get")
                .is_none()
        );
        Store::del(&sess, &did).await.expect("session del");
        Store::clear(&sess).await.expect("session clear");
    }
}
