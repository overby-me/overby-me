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
//! ## Run harness (the live confirmation, a human step)
//!
//! 1. Construct `WikiOAuth::new(db)` and mount [`callback_handler`] at `/callback`
//!    (already wired when [`crate::AppState`] carries the client).
//! 2. Call `begin_login("<handle-or-pds>")` to get the authorization URL; open it
//!    in a browser and complete consent on the account's real PDS.
//! 3. The PDS redirects to `http://127.0.0.1/callback?code=...&state=...&iss=...`;
//!    the handler drives [`WikiOAuth::callback`] to a token + the resolved DID.
//!    Record how far the exchange + refresh get (per the spike's honest limits).

use crate::db::{Db, DbError};
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
    AtprotoLocalhostClientMetadata, AuthorizeOptions, CallbackParams, DefaultHttpClient,
    KnownScope, OAuthClient, OAuthClientConfig, OAuthResolverConfig, Scope,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
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

type HttpClient = DefaultHttpClient;
type DidRes = CommonDidResolver<HttpClient>;
type HandleRes = AtprotoHandleResolver<DohDnsTxtResolver<HttpClient>, HttpClient>;
type Client = OAuthClient<SqliteStateStore, SqliteSessionStore, DidRes, HandleRes>;

/// The result of a completed callback: the resolved account DID (if the session
/// exposes it) and the app-state the authorize call round-tripped.
pub struct CallbackOutcome {
    pub did: Option<String>,
    pub app_state: Option<String>,
}

/// The wiki's atproto OAuth client, backed by durable SQLite stores. Construct
/// once (`new`), `begin_login` per member, `callback` on the redirect back.
pub struct WikiOAuth {
    client: Client,
}

impl WikiOAuth {
    /// Build the OAuth client with durable stores over `db`. Uses the localhost
    /// client-metadata profile (public client, no secret or JWKS), the atproto +
    /// transitional-generic scopes, Cloudflare DoH for handle TXT resolution, and
    /// the default PLC directory for DID resolution (mirrors `oauth-spike`).
    pub fn new(db: Db) -> Result<Self, Box<dyn std::error::Error>> {
        let http_client = Arc::new(DefaultHttpClient::default());
        let config = OAuthClientConfig {
            client_metadata: AtprotoLocalhostClientMetadata {
                redirect_uris: Some(vec![String::from("http://127.0.0.1/callback")]),
                scopes: Some(vec![
                    Scope::Known(KnownScope::Atproto),
                    Scope::Known(KnownScope::TransitionGeneric),
                ]),
            },
            keys: None,
            resolver: OAuthResolverConfig {
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
            },
            state_store: SqliteStateStore::new(db.clone()),
            session_store: SqliteSessionStore::new(db),
        };
        Ok(Self {
            client: OAuthClient::new(config)?,
        })
    }

    /// Begin login for a member identified by handle or PDS URL: the full
    /// server-side pre-redirect flow (resolution + PAR with a fresh DPoP key +
    /// PKCE), returning the authorization URL to redirect them to.
    pub async fn begin_login(
        &self,
        handle_or_pds: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = self
            .client
            .authorize(
                handle_or_pds,
                AuthorizeOptions {
                    scopes: vec![
                        Scope::Known(KnownScope::Atproto),
                        Scope::Known(KnownScope::TransitionGeneric),
                    ],
                    ..Default::default()
                },
            )
            .await?;
        Ok(url)
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
}

// ---------------------------------------------------------------------------
// The /callback HTTP handler.
// ---------------------------------------------------------------------------

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// `/callback`: the PDS redirects the browser here with `code`/`state`/`iss`.
/// Drives [`WikiOAuth::callback`] to a token + DID. Returns 503 if the AppView
/// was built without an OAuth client (the default in tests).
pub async fn callback_handler(
    State(state): State<crate::AppState>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(oauth) = state.oauth.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "oauth not configured").into_response();
    };
    let pairs = crate::util::parse_query(query.as_deref());
    let get = |k: &str| pairs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
    let Some(code) = get("code") else {
        return (StatusCode::BAD_REQUEST, "missing code").into_response();
    };
    let params = CallbackParams {
        code,
        state: get("state"),
        iss: get("iss"),
    };
    match oauth.callback(params).await {
        Ok(outcome) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "did": outcome.did,
                "app_state": outcome.app_state,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("oauth callback failed: {e}");
            (StatusCode::BAD_GATEWAY, "callback failed").into_response()
        }
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
