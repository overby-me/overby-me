use anyhow::{anyhow, Result};
use tangled_api::oauth::PersistedOAuthSession;
use tangled_config::session::{Session, SessionManager};

/// Load session and automatically refresh if expired
pub async fn load_session() -> Result<Session> {
    let mgr = SessionManager::default();
    let session = mgr
        .load()?
        .ok_or_else(|| anyhow!("Please login first: tangled auth login"))?;

    Ok(session)
}

/// Load the persisted OAuth session from keychain, if available.
pub fn load_oauth_session() -> Option<PersistedOAuthSession> {
    let keychain = tangled_config::keychain::Keychain::new("tangled-cli-oauth", "default");
    let json = keychain.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

/// The PDS a session belongs to. Every command needs this and each one was
/// spelling out the same three-step fallback.
pub fn pds_of(session: &Session) -> String {
    session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into())
}

/// Create a TangledClient for the given base URL, with OAuth if available.
pub fn make_client(base_url: &str) -> tangled_api::TangledClient {
    let client = tangled_api::TangledClient::new(base_url);
    match load_oauth_session() {
        Some(oauth) => client.with_oauth(oauth),
        None => client,
    }
}

/// Create a default TangledClient (tngl.sh), with OAuth if available.
pub fn make_default_client() -> tangled_api::TangledClient {
    let client = tangled_api::TangledClient::default();
    match load_oauth_session() {
        Some(oauth) => client.with_oauth(oauth),
        None => client,
    }
}

/// Refresh the session using the refresh token
pub async fn refresh_session(session: &Session) -> Result<Session> {
    let pds = session
        .pds
        .clone()
        .unwrap_or_else(|| "https://bsky.social".to_string());

    let client = tangled_api::TangledClient::new(&pds);
    let mut new_session = client.refresh_session(&session.refresh_jwt).await?;

    // Preserve PDS from old session
    new_session.pds = session.pds.clone();

    // Save the refreshed session
    let mgr = SessionManager::default();
    mgr.save(&new_session)?;

    Ok(new_session)
}

/// Load session with automatic refresh on ExpiredToken
pub async fn load_session_with_refresh() -> Result<Session> {
    let session = load_session().await?;

    // With an OAuth session the JWTs in the stored session are dead weight:
    // `login-browser` writes an empty pair, and a password login that happened
    // earlier leaves a stale one behind. Callers pass `session.access_jwt` as
    // the bearer, and a non-empty bearer makes the client skip OAuth
    // (`should_use_oauth`), so a stale JWT silently shadows a working OAuth
    // session and every request comes back 401. Blank it and let DPoP run.
    if load_oauth_session().is_some() {
        return Ok(Session {
            access_jwt: String::new(),
            refresh_jwt: String::new(),
            ..session
        });
    }

    // Check if session is older than 30 minutes - if so, proactively refresh
    let age = chrono::Utc::now()
        .signed_duration_since(session.created_at)
        .num_minutes();

    if age > 30 {
        // Session is old, proactively refresh. A failure here used to fall
        // through and send the stale token anyway, so every command reported
        // the server's opaque `"exp" claim timestamp check failed` instead of
        // the actual reason the refresh did not happen.
        match refresh_session(&session).await {
            Ok(new_session) => return Ok(new_session),
            Err(e) => {
                return Err(anyhow!(
                    "stored session is expired and refreshing it failed: {e}\n\
                     Run: tangled auth login --handle <handle> --pds <pds>"
                ));
            }
        }
    }

    Ok(session)
}
