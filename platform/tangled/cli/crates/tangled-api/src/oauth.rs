use std::sync::Arc;

use anyhow::{Context, Result};
use atrium_api::agent::SessionManager as _;
use atrium_common::store::Store;
use atrium_identity::{
    did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL},
    handle::{AtprotoHandleResolver, AtprotoHandleResolverConfig, DnsTxtResolver},
};
use atrium_oauth::{
    store::{
        session::{MemorySessionStore, Session as OAuthSession},
        state::MemoryStateStore,
    },
    AtprotoLocalhostClientMetadata, AuthorizeOptions, CallbackParams, DefaultHttpClient,
    KnownScope, OAuthClientConfig, OAuthResolverConfig, Scope,
};
use atrium_xrpc::HttpClient;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use url::Url;

/// DNS TXT resolver using Cloudflare's DNS-over-HTTPS JSON API.
struct DohJsonDnsTxtResolver {
    client: reqwest::Client,
}

impl Default for DohJsonDnsTxtResolver {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[derive(serde::Deserialize)]
struct DohAnswer {
    data: Option<String>,
}

#[derive(serde::Deserialize)]
struct DohResponse {
    #[serde(rename = "Answer")]
    answer: Option<Vec<DohAnswer>>,
}

impl DnsTxtResolver for DohJsonDnsTxtResolver {
    async fn resolve(
        &self,
        query: &str,
    ) -> core::result::Result<Vec<String>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let resp: DohResponse = self
            .client
            .get("https://cloudflare-dns.com/dns-query")
            .query(&[("name", query), ("type", "TXT")])
            .header("Accept", "application/dns-json")
            .send()
            .await?
            .json()
            .await?;

        Ok(resp
            .answer
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| a.data.map(|d| d.trim_matches('"').to_string()))
            .collect())
    }
}

/// Persisted OAuth session data (serializable to keychain).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistedOAuthSession {
    pub did: String,
    pub handle: String,
    pub pds: Option<String>,
    pub oauth_session: OAuthSession,
    /// The client_id this session was issued to.
    ///
    /// A loopback client's id encodes its redirect_uri, and login uses a
    /// random port, so the id cannot be reconstructed later. Refreshing the
    /// token requires presenting the same one, so it has to be kept. Sessions
    /// stored before this field existed have None and cannot be refreshed.
    #[serde(default)]
    pub client_id: Option<String>,
}

/// Result of a successful browser-based OAuth login.
pub struct OAuthLoginResult {
    pub did: String,
    pub handle: String,
    pub pds: Option<String>,
    pub persisted: PersistedOAuthSession,
}

/// Resolve PDS endpoint from a DID via the PLC directory.
struct DidInfo {
    pds: Option<String>,
    handle: Option<String>,
}

async fn resolve_did_info(did: &str) -> Result<DidInfo> {
    #[derive(serde::Deserialize)]
    struct Service {
        #[serde(rename = "type")]
        service_type: String,
        #[serde(rename = "serviceEndpoint")]
        service_endpoint: String,
    }
    #[derive(serde::Deserialize)]
    struct DidDocument {
        #[serde(default, rename = "alsoKnownAs")]
        also_known_as: Vec<String>,
        service: Option<Vec<Service>>,
    }

    let base = DEFAULT_PLC_DIRECTORY_URL.trim_end_matches('/');
    let url = format!("{base}/{did}");
    let doc: DidDocument = reqwest::get(&url)
        .await
        .context("failed to fetch DID document")?
        .json()
        .await
        .context("failed to parse DID document")?;

    let pds = doc
        .service
        .unwrap_or_default()
        .into_iter()
        .find(|s| s.service_type == "AtprotoPersonalDataServer")
        .map(|s| s.service_endpoint);

    let handle = doc
        .also_known_as
        .into_iter()
        .find(|a| a.starts_with("at://"))
        .and_then(|a| a.strip_prefix("at://").map(String::from));

    Ok(DidInfo { pds, handle })
}

macro_rules! oauth_client_config {
    ($http_client:expr, $session_store:expr, $redirect_uris:expr) => {
        OAuthClientConfig {
            client_metadata: AtprotoLocalhostClientMetadata {
                redirect_uris: $redirect_uris,
                scopes: Some(vec![
                    Scope::Known(KnownScope::Atproto),
                    Scope::Known(KnownScope::TransitionGeneric),
                ]),
            },
            keys: None,
            resolver: OAuthResolverConfig {
                did_resolver: CommonDidResolver::new(CommonDidResolverConfig {
                    plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_string(),
                    http_client: Arc::clone(&$http_client),
                }),
                handle_resolver: AtprotoHandleResolver::new(AtprotoHandleResolverConfig {
                    dns_txt_resolver: DohJsonDnsTxtResolver::default(),
                    http_client: Arc::clone(&$http_client),
                }),
                authorization_server_metadata: Default::default(),
                protected_resource_metadata: Default::default(),
            },
            state_store: MemoryStateStore::default(),
            session_store: $session_store,
        }
    };
}

/// The client_id a loopback client gets for a given callback port.
/// Built by hand: atrium does not export the trait that computes it.
fn client_id_for(port: u16) -> Option<String> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("redirect_uri", &format!("http://127.0.0.1:{port}/callback"))
        .append_pair("scope", "atproto transition:generic")
        .finish();
    Some(format!("http://localhost?{query}"))
}

/// Whether a session's access token has expired.
pub fn is_expired(persisted: &PersistedOAuthSession) -> bool {
    match &persisted.oauth_session.token_set.expires_at {
        Some(expires_at) => {
            expires_at.as_str() <= atrium_api::types::string::Datetime::now().as_str()
        }
        // No expiry recorded: assume it is still good and let the server say.
        None => false,
    }
}

/// Exchange the refresh token for a new access token, and persist it.
///
/// atrium only refreshes from inside its own XRPC client, on a 401, so a CLI
/// making raw requests never reaches it. This does the exchange directly
/// against the authorization server, using atrium's DpopClient so the proof
/// and nonce handling stay its problem rather than ours.
pub async fn refresh(persisted: &PersistedOAuthSession) -> Result<PersistedOAuthSession> {
    let token_set = &persisted.oauth_session.token_set;
    let refresh_token = token_set
        .refresh_token
        .as_deref()
        .context("this session has no refresh token; log in again")?;
    let client_id = persisted
        .client_id
        .as_deref()
        .context("this session predates client_id being stored; log in again")?;

    #[derive(serde::Deserialize)]
    struct ServerMetadata {
        token_endpoint: String,
    }
    let metadata: ServerMetadata = reqwest::Client::new()
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            token_set.iss.trim_end_matches('/')
        ))
        .send()
        .await?
        .json()
        .await
        .context("read authorization server metadata")?;

    let body = serde_urlencoded::to_string([
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])?;
    let request = atrium_xrpc::http::Request::builder()
        .method("POST")
        .uri(&metadata.token_endpoint)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.into_bytes())
        .context("build refresh request")?;

    let dpop_client = make_dpop_client(persisted)?;
    let response = dpop_client
        .send_http(request)
        .await
        .map_err(|e| anyhow::anyhow!("refresh request failed: {e}"))?;
    let status = response.status();
    let bytes = response.into_body();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "refresh rejected ({status}): {}",
            String::from_utf8_lossy(&bytes)
        ));
    }

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<i64>,
    }
    let fresh: TokenResponse = serde_json::from_slice(&bytes).context("decode refreshed token")?;

    let mut updated = persisted.clone();
    updated.oauth_session.token_set.access_token = fresh.access_token;
    if let Some(rt) = fresh.refresh_token {
        updated.oauth_session.token_set.refresh_token = Some(rt);
    }
    if let Some(secs) = fresh.expires_in {
        let expiry = chrono::Utc::now() + chrono::Duration::seconds(secs);
        updated.oauth_session.token_set.expires_at =
            Some(atrium_api::types::string::Datetime::new(expiry.into()));
    }
    Ok(updated)
}

/// Run the full browser-based OAuth login flow.
pub async fn login_browser(handle: &str) -> Result<OAuthLoginResult> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind local server")?;
    let port = listener.local_addr()?.port();

    let http_client = Arc::new(DefaultHttpClient::default());
    let session_store = MemorySessionStore::default();

    let config = oauth_client_config!(
        http_client,
        session_store.clone(),
        Some(vec![format!("http://127.0.0.1:{port}/callback")])
    );
    let client = atrium_oauth::OAuthClient::new(config).context("failed to create OAuth client")?;

    let auth_url = client
        .authorize(
            handle,
            AuthorizeOptions {
                scopes: vec![
                    Scope::Known(KnownScope::Atproto),
                    Scope::Known(KnownScope::TransitionGeneric),
                ],
                ..Default::default()
            },
        )
        .await
        .context("failed to get authorization URL")?;
    open::that(&auth_url).context("failed to open browser")?;

    // Wait for the OAuth callback redirect
    let (stream, _) = listener
        .accept()
        .await
        .context("failed to accept callback connection")?;
    let mut stream = BufReader::new(stream);

    let mut request_line = String::new();
    stream
        .read_line(&mut request_line)
        .await
        .context("failed to read request")?;

    let path = request_line
        .split_whitespace()
        .nth(1)
        .context("malformed HTTP request")?;

    let url = Url::parse(&format!("http://127.0.0.1:{port}{path}"))
        .context("failed to parse callback URL")?;

    let params: std::collections::HashMap<String, String> =
        url.query_pairs().into_owned().collect();

    let callback_params = CallbackParams {
        code: params
            .get("code")
            .context("missing 'code' parameter")?
            .clone(),
        state: params.get("state").cloned(),
        iss: params.get("iss").cloned(),
    };

    // Send response to the browser
    let body = "<html><body><h1>Login successful!</h1><p>You can close this tab.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.get_mut().write_all(response.as_bytes()).await.ok();
    stream.get_mut().shutdown().await.ok();

    // Exchange code for tokens
    let (session, _) = client
        .callback(callback_params)
        .await
        .context("OAuth callback failed")?;

    let did = session.did().await.context("no DID in OAuth session")?;

    // Read the OAuth session data from the memory store for persistence
    let oauth_session = session_store
        .get(&did)
        .await
        .map_err(|e| anyhow::anyhow!("failed to read session store: {e}"))?
        .context("OAuth session not in store")?;

    let did_str = did.to_string();
    let did_info = resolve_did_info(&did_str).await.unwrap_or(DidInfo {
        pds: None,
        handle: None,
    });
    let resolved_handle = did_info.handle.unwrap_or_else(|| handle.to_string());

    let persisted = PersistedOAuthSession {
        client_id: client_id_for(port),
        did: did_str.clone(),
        handle: resolved_handle.clone(),
        pds: did_info.pds.clone(),
        oauth_session,
    };

    Ok(OAuthLoginResult {
        did: did_str,
        handle: resolved_handle,
        pds: did_info.pds,
        persisted,
    })
}

/// Create a DPoP HTTP client from persisted session data.
fn make_dpop_client(
    persisted: &PersistedOAuthSession,
) -> Result<atrium_oauth::DpopClient<DefaultHttpClient>> {
    let http_client = Arc::new(DefaultHttpClient::default());
    atrium_oauth::DpopClient::new(
        persisted.oauth_session.dpop_key.clone(),
        http_client,
        false,
        &None,
    )
    .map_err(|e| anyhow::anyhow!("failed to create DPoP client: {e}"))
}

/// Make an authenticated XRPC GET request using a persisted OAuth session.
pub async fn oauth_get(persisted: &PersistedOAuthSession, url: &str) -> Result<Vec<u8>> {
    let dpop_client = make_dpop_client(persisted)?;
    let auth_value = format!("DPoP {}", persisted.oauth_session.token_set.access_token);
    let request = atrium_xrpc::http::Request::builder()
        .method("GET")
        .uri(url)
        .header("Authorization", &auth_value)
        .body(Vec::new())
        .context("failed to build request")?;
    let response = dpop_client
        .send_http(request)
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = response.status();
    let body = response.into_body();
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        return Err(anyhow::anyhow!("{status}: {text}"));
    }
    Ok(body)
}

/// Make an authenticated XRPC POST request using a persisted OAuth session.
pub async fn oauth_post(
    persisted: &PersistedOAuthSession,
    url: &str,
    json_body: &[u8],
) -> Result<Vec<u8>> {
    let dpop_client = make_dpop_client(persisted)?;
    let auth_value = format!("DPoP {}", persisted.oauth_session.token_set.access_token);
    let request = atrium_xrpc::http::Request::builder()
        .method("POST")
        .uri(url)
        .header("content-type", "application/json")
        .header("Authorization", &auth_value)
        .body(json_body.to_vec())
        .context("failed to build request")?;
    let response = dpop_client
        .send_http(request)
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = response.status();
    let body = response.into_body();
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        return Err(anyhow::anyhow!("{status}: {text}"));
    }
    Ok(body)
}

/// Make an authenticated XRPC POST request with arbitrary content type.
pub async fn oauth_post_raw(
    persisted: &PersistedOAuthSession,
    url: &str,
    raw_body: &[u8],
    content_type: &str,
) -> Result<Vec<u8>> {
    let dpop_client = make_dpop_client(persisted)?;
    let auth_value = format!("DPoP {}", persisted.oauth_session.token_set.access_token);
    let request = atrium_xrpc::http::Request::builder()
        .method("POST")
        .uri(url)
        .header("content-type", content_type)
        .header("Authorization", &auth_value)
        .body(raw_body.to_vec())
        .context("failed to build request")?;
    let response = dpop_client
        .send_http(request)
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = response.status();
    let body = response.into_body();
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        return Err(anyhow::anyhow!("{status}: {text}"));
    }
    Ok(body)
}
