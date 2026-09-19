//! What the components ask of `crate::backend_api`, asked of the AppView.
//!
//! The sidecar's endpoints became XRPC methods. The account link did not come
//! along: here someone signs in WITH their atproto account, so whoever is
//! signed in is linked, and there is nothing to link or unlink afterwards.

use super::{ask_quiet, client, said};
use appview_client::{
    claim_membership, defs, get_blob_link, get_member_claim_link, list_contexts, notify_context,
    notify_reply, share_to_bluesky, submit_feedback, subscribe_push, unsubscribe_push, upload_blob,
    Error,
};
use serde::Deserialize;

#[allow(unused_imports)]
pub use super::ballot::{vote_cast_secret, vote_status};
#[allow(unused_imports)]
pub use crate::bsky::search_bsky_actors;

/// Where the log sink and the crash reporter post to: the AppView serves both.
pub const BACKEND_URL: &str = super::APPVIEW_URL;

/// Signing in is what links an account here, so this is where that starts.
pub fn atproto_start_url(handle: &str, _token: &str) -> String {
    super::login_url(handle)
}

/// For fetches, which can send the session. An element cannot, and takes
/// [`presigned_file_url`] instead.
pub fn file_url(file_id: &str) -> String {
    client(None).blob_url(file_id)
}

pub async fn file_bytes(file_id: &str, token: &str) -> Result<Vec<u8>, String> {
    if file_id.is_empty() {
        return Err("no file".into());
    }
    let client = client(Some(token));
    ask_quiet(true, || client.get_blob(file_id))
        .await
        .map(|file| file.bytes)
        .map_err(|error| match error {
            // The AppView answers 404 for a file the reader may not read, as for
            // one that is not there, so say both rather than "missing".
            Error::Api { status: 404, .. } => "no such file, or not ours to read".to_string(),
            other => said(&other),
        })
}

/// A Windows metafile drawn as SVG or PNG, with which of the two it is.
pub async fn render_metafile(bytes: &[u8], token: &str) -> Result<(Vec<u8>, String), String> {
    let client = client(Some(token));
    let picture = client
        .render_metafile(bytes.to_vec(), "application/octet-stream")
        .await
        .map_err(|e| said(&e))?;
    let mime = picture.content_type.split(';').next().unwrap_or_default();
    let mime = match mime.trim() {
        "" => "image/png",
        mime => mime,
    };
    Ok((picture.bytes, mime.to_string()))
}

async fn blob_link(file_id: &str, token: &str) -> Option<String> {
    let client = client(Some(token));
    let params = get_blob_link::Params {
        id: file_id.to_string(),
    };
    ask_quiet(true, || client.get_blob_link(&params))
        .await
        .ok()
        .map(|link| link.url)
}

/// A link Microsoft's viewer can fetch the document by. Whoever holds it reads
/// that document until it expires, and using the viewer sends it to Microsoft.
pub async fn office_embed_url(file_id: &str, token: &str) -> Option<String> {
    blob_link(file_id, token).await
}

/// A link that carries its own authority, for an element `src`, which cannot
/// send a header. It lasts two hours, so ask again for a file still on screen.
pub async fn presigned_file_url(file_id: &str, token: &str) -> Option<String> {
    blob_link(file_id, token).await
}

#[derive(Deserialize, Clone, PartialEq, Debug, Default)]
pub struct AtprotoLink {
    #[serde(default)]
    pub linked: bool,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub did: String,
}

/// Who the caller is on atproto. "Not linked" when nobody is signed in, or the
/// AppView did not answer.
pub async fn atproto_status(token: &str) -> AtprotoLink {
    let client = client(Some(token));
    match ask_quiet(true, || client.get_session()).await {
        Ok(me) => AtprotoLink {
            linked: true,
            handle: me.handle.unwrap_or_default(),
            did: me.did,
        },
        Err(_) => AtprotoLink::default(),
    }
}

/// The account is the identity here, so it cannot be taken off.
pub async fn atproto_unlink(_token: &str) -> bool {
    false
}

pub async fn atproto_post(token: &str, text: &str, link: &str, title: &str) -> Result<(), String> {
    let given = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let client = client(Some(token));
    let input = share_to_bluesky::Input {
        text: text.to_string(),
        title: given(title),
        url: given(link),
    };
    client
        .share_to_bluesky(&input)
        .await
        .map(|_| ())
        .map_err(|e| said(&e))
}

/// The (name, email) rows of an .xlsx roster. Empty on any failure.
pub async fn parse_roster(token: Option<&str>, bytes: Vec<u8>) -> Vec<(String, String)> {
    const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    match client(token).parse_roster(bytes, XLSX).await {
        Ok(roster) => roster
            .entries
            .into_iter()
            .map(|row| (row.name, row.email))
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub async fn push_subscribe(
    token: &str,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> Result<(), String> {
    let input = subscribe_push::Input {
        endpoint: endpoint.to_string(),
        p256dh: p256dh.to_string(),
        auth: auth.to_string(),
    };
    client(Some(token))
        .subscribe_push(&input)
        .await
        .map(|_| ())
        .map_err(|e| format!("subscribe failed: {}", said(&e)))
}

pub async fn push_unsubscribe(token: &str, endpoint: &str) -> Result<(), String> {
    let input = unsubscribe_push::Input {
        endpoint: endpoint.to_string(),
    };
    client(Some(token))
        .unsubscribe_push(&input)
        .await
        .map(|_| ())
        .map_err(|e| said(&e))
}

fn sent_to(recipients: i64, sent: i64) -> (u64, u64) {
    let count = |n: i64| u64::try_from(n).unwrap_or(0);
    (count(recipients), count(sent))
}

/// Push to a context's active members, as one of its owners: (recipients, sent).
pub async fn push_notify(
    token: &str,
    context: &str,
    title: &str,
    body: &str,
    link: &str,
) -> Result<(u64, u64), String> {
    let input = notify_context::Input {
        context_id: context.to_string(),
        title: Some(title.to_string()),
        body: Some(body.to_string()),
        url: Some(link.to_string()),
    };
    client(Some(token))
        .notify_context(&input)
        .await
        .map(|told| sent_to(told.recipients, told.sent))
        .map_err(|e| said(&e))
}

/// Push "someone answered you" to whoever wrote `parent`: (recipients, sent).
pub async fn push_reply(
    token: &str,
    parent: &str,
    title: &str,
    body: &str,
    link: &str,
) -> Result<(u64, u64), String> {
    let input = notify_reply::Input {
        parent: parent.to_string(),
        title: Some(title.to_string()),
        body: Some(body.to_string()),
        url: Some(link.to_string()),
    };
    client(Some(token))
        .notify_reply(&input)
        .await
        .map(|told| sent_to(told.recipients, told.sent))
        .map_err(|e| said(&e))
}

/// Take the seat a `?claim=` link is for. Answers the context it is in.
pub async fn claim_membership(token: &str, claim_token: &str) -> Result<String, String> {
    let input = claim_membership::Input {
        token: claim_token.to_string(),
    };
    client(Some(token))
        .claim_membership(&input)
        .await
        .map(|seat| seat.context_id)
        .map_err(|e| said(&e))
}

/// An owner's: the secret of a member's `?claim=` link.
pub async fn member_claim_link(token: &str, member_id: &str) -> Result<String, String> {
    let params = get_member_claim_link::Params {
        member: member_id.to_string(),
    };
    client(Some(token))
        .get_member_claim_link(&params)
        .await
        .map(|link| link.token)
        .map_err(|e| said(&e))
}

/// The app, the build and the browser a report is sent from.
pub(crate) fn sent_from(input: &mut submit_feedback::Input) {
    input.app = Some(env!("CARGO_PKG_VERSION").to_string());
    input.commit = Some(crate::build_info::COMMIT.to_string());
    #[cfg(target_arch = "wasm32")]
    {
        input.ua = web_sys::window().and_then(|w| w.navigator().user_agent().ok());
    }
}

thread_local! {
    /// Reports already filed by this tab, so a failure that repeats on every
    /// render files once.
    static REPORTED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The most automatic reports one tab may file, however many things go wrong:
/// a build broken in twenty ways must not have every device narrate all twenty.
const MAX_AUTO_REPORTS: usize = 5;

/// File a failure the reader was only told was "something went wrong". Quiet
/// about its own failure: a report that reports failing to report is a loop.
pub async fn report_error(access_token: Option<&str>, message: &str, path: &str) {
    let fresh = REPORTED.with(|seen| {
        let mut seen = seen.borrow_mut();
        seen.len() < MAX_AUTO_REPORTS && seen.insert(message.to_string())
    });
    if !fresh {
        return;
    }
    let mut input = submit_feedback::Input {
        kind: Some("error".to_string()),
        message: message.to_string(),
        path: Some(path.to_string()),
        ..Default::default()
    };
    sent_from(&mut input);
    let _ = client(access_token).submit_feedback(&input).await;
}

/// A context of the caller's to file something under that belongs to no page:
/// a report's screenshot. Only its members and whoever reads the reports see it.
async fn a_context_of_mine(client: &appview_client::Client) -> Result<String, Error> {
    let params = list_contexts::Params {
        scope: Some("mine".to_string()),
        ..Default::default()
    };
    let mine = ask_quiet(true, || client.list_contexts(&params)).await?;
    mine.contexts
        .first()
        .map(|context| context.id.clone())
        .ok_or_else(|| Error::Api {
            status: 403,
            error: "Forbidden".to_string(),
            message: "a file is kept in a group, and you are in none".to_string(),
        })
}

/// Store a file in `context_id`, whose readers are then the file's.
pub(crate) async fn upload(
    access_token: Option<&str>,
    context_id: Option<&str>,
    bytes: Vec<u8>,
    file_name: &str,
    content_type: &str,
) -> Result<defs::BlobView, Error> {
    let client = client(access_token);
    let context = match context_id.filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => a_context_of_mine(&client).await?,
    };
    let params = upload_blob::Params {
        context,
        name: Some(file_name.to_string()).filter(|name| !name.is_empty()),
    };
    // An empty browser `File.type`: let the server call it what it is.
    let content_type = match content_type.trim() {
        "" => "application/octet-stream",
        given => given,
    };
    client.upload_blob(&params, bytes, content_type).await
}
