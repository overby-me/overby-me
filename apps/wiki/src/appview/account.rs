//! What the app asks of `crate::nhost`, asked of the AppView: who is signed
//! in, and the files they keep.
//!
//! An account here is an atproto one. Signing in happens at the account's own
//! PDS, which sends the browser back with a one-time code (`#code=`); the code
//! buys a session that lasts a month, with nothing to refresh. So of the
//! interim's auth calls only those the session and the uploads need are here,
//! under its names and in its shapes. There is no password to set or reset.

use super::{api, ask_quiet, client};
use appview_client::{create_session, defs, Error};

/// How long the app takes a session to be good for before asking again whether
/// it still is. The AppView ends one after a month, or when it is signed out.
const ASK_AGAIN_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone)]
pub struct NhostError {
    pub status: Option<u16>,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl std::fmt::Display for NhostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let said = self.message.as_deref().or(self.error.as_deref());
        write!(f, "{}", said.unwrap_or("Unknown error"))
    }
}

impl From<Error> for NhostError {
    fn from(error: Error) -> Self {
        match error {
            Error::Api {
                status,
                error,
                message,
            } => NhostError {
                status: Some(status),
                error: Some(error),
                message: Some(message),
            },
            // No status: the network, which must never read as a dead session.
            other => NhostError {
                status: None,
                error: Some("network_error".to_string()),
                message: Some(other.to_string()),
            },
        }
    }
}

/// Whether the AppView said the session is over, as against not answering.
pub fn is_auth_error(err: &NhostError) -> bool {
    matches!(err.status, Some(401 | 403))
}

pub struct NhostUser {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

// No `Debug`: both tokens are the session.
pub struct AuthSession {
    pub access_token: String,
    /// The same as `access_token`. The interim renews with a second token; here
    /// "renewing" is asking the AppView whether the session still stands.
    pub refresh_token: String,
    pub access_token_expires_in: Option<i64>,
    pub user: Option<NhostUser>,
}

fn session_of(token: &str, me: defs::UserView) -> AuthSession {
    AuthSession {
        access_token: token.to_string(),
        refresh_token: token.to_string(),
        access_token_expires_in: Some(ASK_AGAIN_SECS),
        user: Some(NhostUser {
            display_name: me.display_name.clone().or(me.handle.clone()),
            avatar_url: me.avatar_url,
            // What the app shows under a name as the account it is signed in
            // with. Here that is the handle: the address is the PDS's to know.
            email: me.handle.map(|handle| format!("@{handle}")),
            id: me.did,
        }),
    }
}

/// The session a token is for, with its holder as the AppView knows them now.
pub async fn refresh_session(refresh_token: &str) -> Result<AuthSession, NhostError> {
    let client = client(Some(refresh_token));
    let me = ask_quiet(true, || client.get_session()).await?;
    Ok(session_of(refresh_token, me))
}

/// Spend the one-time code a sign-in came back with.
pub async fn sign_in_with_code(code: &str) -> Result<AuthSession, NhostError> {
    let input = create_session::Input {
        code: code.to_string(),
    };
    let made = client(None).create_session(&input).await?;
    refresh_session(&made.session).await
}

/// End the session at the AppView too, not only in this browser: a bearer that
/// lasts a month should not outlive a sign-out on a shared machine.
pub fn sign_out() {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(token) = crate::session::current_token() {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = client(Some(&token)).delete_session().await;
            });
        }
        crate::offline::clear();
    }
    // A ballot stub says how its owner voted: it goes with the session.
    super::ballot::forget_all();
}

#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub id: String,
    pub mime_type: Option<String>,
}

/// Store a file in `context_id`, whose readers are then the file's. `None` for
/// what belongs to no page, which goes to a context of the uploader's.
pub async fn upload_file(
    access_token: Option<&str>,
    context_id: Option<&str>,
    bytes: Vec<u8>,
    file_name: &str,
    content_type: &str,
) -> Result<UploadedFile, NhostError> {
    let kept = api::upload(access_token, context_id, bytes, file_name, content_type).await?;
    Ok(UploadedFile {
        id: kept.id,
        mime_type: Some(kept.mime),
    })
}
