//! Reports: what people send about the app, and the crashes it files itself.

use super::seen::{saw, Seen};
use super::{api, ask, client, map};
use appview_client::submit_feedback;

/// One report. Whoever runs the site is served everyone's, anyone else their
/// own: the AppView decides which.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FeedbackItem {
    pub id: String,
    pub kind: String,
    pub message: String,
    /// A screenshot's file id.
    pub image: Option<String>,
    pub path: String,
    pub commit: String,
    pub user_agent: String,
    /// How often a crash folded into this row has been reported. The log sink
    /// keeps three days, so this is the only lasting record of how common it is.
    pub seen: u64,
    /// Everyone who has hit it, and `anonymous` at most once for all those
    /// with no account.
    pub reporters: Vec<String>,
    pub last_seen: String,
    pub created_at: String,
    pub owner_id: Option<String>,
    pub owner_name: String,
    pub owner_avatar: String,
}

pub async fn insert_feedback(
    access_token: Option<&str>,
    kind: &str,
    message: &str,
    image_file_id: Option<&str>,
    path: &str,
    _app_version: &str,
    user_agent: &str,
) -> Result<(), String> {
    let mut input = submit_feedback::Input {
        kind: Some(kind.to_string()),
        message: message.to_string(),
        image: image_file_id
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        path: Some(path.to_string()),
        ..Default::default()
    };
    api::sent_from(&mut input);
    input.ua = Some(user_agent.to_string());
    let client = client(access_token);
    ask("submitFeedback", false, || client.submit_feedback(&input))
        .await
        .map(|_| ())
}

/// The reports the caller may see, newest first.
pub async fn query_feedback(access_token: Option<&str>) -> Result<Vec<FeedbackItem>, String> {
    let client = client(access_token);
    let reports = ask("listFeedback", true, || client.list_feedback()).await?;
    let mut items: Vec<FeedbackItem> = reports
        .feedback
        .into_iter()
        .map(|report| {
            saw(&report.id, Seen::Feedback);
            let owner = report
                .owner_did
                .as_deref()
                .map(|did| map::user_ref(&reports.profiles, did));
            FeedbackItem {
                id: report.id,
                kind: report.kind,
                message: report.message,
                image: report.image.filter(|id| !id.is_empty()),
                path: report.path,
                commit: report.commit,
                user_agent: report.user_agent,
                seen: u64::try_from(report.seen).unwrap_or(0),
                reporters: report.reporters,
                last_seen: report.updated_at,
                created_at: report.created_at,
                owner_id: report.owner_did,
                owner_name: owner
                    .as_ref()
                    .map(|o| o.display_name.clone())
                    .unwrap_or_default(),
                owner_avatar: owner.map(|o| o.avatar_url).unwrap_or_default(),
            }
        })
        .collect();
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}
