//! Mail to an invited address: the claim link an owner would otherwise have to
//! hand over themselves.
//!
//! One kind of mail only, to an address an owner of a context put on its roster,
//! carrying that seat's claim link. Off unless `APPVIEW_SMTP_URL` and
//! `APPVIEW_MAIL_FROM` are both set, and then an invitation is mailed as it is
//! made. Sent beside the request, never in it: a roster is two thousand rows,
//! and an SMTP server takes its time.

use crate::AppState;
use crate::config::Config;
use crate::live::Topic;
use crate::session::Caller;
use crate::xrpc::{err, forbidden, invalid, write_failed};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lettre::message::header::{ContentType, Header, HeaderName, HeaderValue};
use lettre::message::{Mailbox, Message};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use serde::Deserialize;
use std::time::Duration;

/// Between two mails of one roster: a provider's rate limit is met by waiting,
/// and an invitation is not urgent to the second.
const PACE: Duration = Duration::from_millis(120);

/// How soon an owner may have the same seat mailed again.
const AGAIN_AFTER_SECS: u64 = 10 * 60;

/// `Auto-Submitted: auto-generated` (RFC 3834), so that an out-of-office reply
/// is not sent to an address nobody reads.
#[derive(Clone)]
struct AutoSubmitted;

impl Header for AutoSubmitted {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("Auto-Submitted")
    }

    fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(AutoSubmitted)
    }

    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), "auto-generated".to_string())
    }
}

pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl Mailer {
    /// `None` when mail is not configured. Half a configuration is an error: a
    /// site that believes it mails its invitations and does not is worse off
    /// than one that knows it does not.
    pub fn from_config(config: &Config) -> Result<Option<Mailer>, String> {
        let (url, from) = (config.smtp_url.expose(), config.mail_from.trim());
        match (url.is_empty(), from.is_empty()) {
            (true, true) => return Ok(None),
            (false, false) => {}
            _ => return Err("APPVIEW_SMTP_URL and APPVIEW_MAIL_FROM go together".to_string()),
        }
        if config.app_origin.is_empty() {
            return Err("mail needs APPVIEW_APP_ORIGIN, which a claim link points into".into());
        }
        let from = from
            .parse::<Mailbox>()
            .map_err(|e| format!("APPVIEW_MAIL_FROM is not an address: {e}"))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|e| format!("APPVIEW_SMTP_URL is not an SMTP url: {e}"))?
            .build();
        Ok(Some(Mailer { transport, from }))
    }

    async fn send(&self, to: &str, subject: &str, text: String) -> Result<(), String> {
        let to = to.parse::<Mailbox>().map_err(|e| e.to_string())?;
        let mail = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .header(AutoSubmitted)
            .body(text)
            .map_err(|e| e.to_string())?;
        self.transport
            .send(mail)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// A seat to be mailed its claim link.
#[derive(Debug, Clone)]
pub struct Seat {
    pub member_id: String,
    pub email: String,
    pub claim_token: String,
}

/// The subject and the text of an invitation. In both of the site's languages,
/// since nothing is known of whoever is about to read it.
pub fn invitation(site: &str, context: &str, link: &str) -> (String, String) {
    let text = format!(
        "Du er inviteret til {context} på {site}.\n\n\
         Tag din plads her:\n{link}\n\n\
         Linket er personligt. Log ind med din konto, så er pladsen din.\n\
         Havde du ikke ventet denne mail, kan du se bort fra den.\n\n\
         ---\n\n\
         You have been invited to {context} on {site}.\n\n\
         Take your seat here:\n{link}\n\n\
         The link is personal. Sign in with your account and the seat is yours.\n\
         If you were not expecting this mail, you can ignore it.\n"
    );
    (format!("Invitation: {context}"), text)
}

fn claim_link(config: &Config, claim_token: &str) -> String {
    format!("{}/?claim={claim_token}", config.app_origin)
}

async fn context_name(state: &AppState, context_id: &str) -> String {
    let named = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query("SELECT name FROM context WHERE id = ?1", [context_id])
            .await?;
        Ok::<_, crate::db::DbError>(match rows.next().await? {
            Some(row) => row.get::<String>(0)?,
            None => String::new(),
        })
    };
    named.await.unwrap_or_default()
}

/// Mail one seat, and note that it was. Returns what went wrong, if it did.
async fn mail_seat(
    state: &AppState,
    mailer: &Mailer,
    context: &str,
    seat: &Seat,
) -> Result<(), String> {
    let link = claim_link(&state.config, &seat.claim_token);
    let (subject, text) = invitation(&state.config.site_name, context, &link);
    mailer.send(&seat.email, &subject, text).await?;
    let noted = async {
        let conn = state.db.acquire().await?;
        conn.execute(
            "UPDATE member SET mailed_at = ?1 WHERE id = ?2",
            [crate::util::now_stamp().as_str(), seat.member_id.as_str()],
        )
        .await
        .map_err(crate::db::DbError::from)
    };
    noted.await.map(|_| ()).map_err(|e| e.to_string())
}

/// Mail every seat of a roster that was just put in, one after another. The
/// address is not logged: a log is read by more people than a roster is.
pub async fn invite_all(state: AppState, context_id: String, seats: Vec<Seat>) {
    let Some(mailer) = state.mailer.clone() else {
        return;
    };
    let context = context_name(&state, &context_id).await;
    let mut failed = 0usize;
    for seat in &seats {
        if let Err(e) = mail_seat(&state, &mailer, &context, seat).await {
            failed += 1;
            tracing::warn!(
                "an invitation to seat {} was not mailed: {e}",
                seat.member_id
            );
        }
        tokio::time::sleep(PACE).await;
    }
    tracing::info!(
        "mailed {} of {} invitations to {context_id}",
        seats.len() - failed,
        seats.len()
    );
    state.publish(Topic::Context(context_id.clone()), "member", &context_id);
}

#[derive(Debug, Deserialize)]
pub struct SendInvitationBody {
    /// The member id.
    pub member: String,
}

/// `wiki.radikal.sendInvitation` (procedure): an owner has a seat's claim
/// link mailed to the address on it, again or for the first time.
pub async fn send_invitation(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<SendInvitationBody>,
) -> Response {
    let what = "sendInvitation";
    let Some(mailer) = state.mailer.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "MailNotConfigured",
            "this site sends no mail: hand the claim link over yourself",
        );
    };
    let found = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT context_id, email, claim_token, user_did, mailed_at FROM member WHERE id = ?1",
                [body.member.as_str()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok::<_, crate::db::DbError>(None);
        };
        let text = |i: usize| match row.get_value(i) {
            Ok(turso::Value::Text(s)) => Some(s),
            _ => None,
        };
        Ok(Some((
            row.get::<String>(0)?,
            text(1),
            text(2),
            text(3),
            text(4),
        )))
    };
    // The same answer to a stranger as for a seat that is not there, so that
    // this is no oracle for member ids (as `getMemberClaimLink` is none).
    let refused = || forbidden("not an owner of that member's context");
    let (context_id, email, claim_token, holder, mailed_at) = match found.await {
        Ok(Some(seat)) => seat,
        Ok(None) => return refused(),
        Err(e) => return write_failed(what, e),
    };
    match crate::authz::Authz::new(state.db.clone())
        .is_active_owner(&context_id, &did)
        .await
    {
        Ok(true) => {}
        Ok(false) => return refused(),
        Err(e) => return write_failed(what, e),
    }
    // A seat an interim account holds is still to be claimed by a person.
    if holder
        .as_deref()
        .is_some_and(|id| !crate::legacy::is_carried(id))
    {
        return invalid("that seat is taken already, so there is nothing to claim");
    }
    let Some(email) = email else {
        return invalid("that seat has no address to mail");
    };
    let claim_token = match claim_token {
        Some(token) => token,
        None => match crate::Store::new(state.db.clone())
            .mint_claim_token(&body.member)
            .await
        {
            Ok(token) => token,
            Err(e) => return write_failed(what, e),
        },
    };
    let recently =
        crate::util::rfc3339_utc(crate::util::now_secs().saturating_sub(AGAIN_AFTER_SECS));
    if mailed_at.is_some_and(|at| at > recently) {
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "TooSoon",
            "that seat was mailed a moment ago",
        );
    }
    let seat = Seat {
        member_id: body.member.clone(),
        email,
        claim_token,
    };
    let context = context_name(&state, &context_id).await;
    match mail_seat(&state, &mailer, &context, &seat).await {
        Ok(()) => {
            state.publish(Topic::Context(context_id), "member", &body.member);
            (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Err(e) => {
            tracing::warn!("an invitation to seat {} was not mailed: {e}", body.member);
            err(
                StatusCode::BAD_GATEWAY,
                "MailFailed",
                "the mail server did not take it",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{get_as, post, seeded_state, token_for};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// (the recipient, the mail as it came over the wire)
    type Inbox = Arc<Mutex<Vec<(String, String)>>>;

    /// An SMTP server that takes anything and keeps it.
    async fn smtp_server() -> (u16, Inbox) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let inbox = Inbox::default();
        let kept = inbox.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(session(socket, kept.clone()));
            }
        });
        (port, inbox)
    }

    async fn session(socket: tokio::net::TcpStream, inbox: Inbox) {
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        let _ = write.write_all(b"220 fake ESMTP\r\n").await;
        let mut to = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let command = line.to_ascii_uppercase();
            let answer: &[u8] = if command.starts_with("EHLO") {
                b"250-fake\r\n250 8BITMIME\r\n"
            } else if command.starts_with("RCPT TO:") {
                to = line[8..].trim().to_string();
                b"250 ok\r\n"
            } else if command == "DATA" {
                let _ = write.write_all(b"354 go on\r\n").await;
                let mut data = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line == "." {
                        break;
                    }
                    data.push_str(&line);
                    data.push('\n');
                }
                inbox.lock().expect("inbox").push((to.clone(), data));
                b"250 queued\r\n"
            } else if command.starts_with("QUIT") {
                let _ = write.write_all(b"221 bye\r\n").await;
                return;
            } else {
                b"250 ok\r\n"
            };
            let _ = write.write_all(answer).await;
        }
    }

    /// The text of a mail, whichever of the two encodings lettre chose for it.
    fn text_of(mail: &str) -> String {
        let (headers, body) = mail.split_once("\n\n").unwrap_or(("", mail));
        if !headers.to_ascii_lowercase().contains("quoted-printable") {
            return body.to_string();
        }
        let joined = body.replace("=\n", "");
        let mut bytes = Vec::new();
        let mut rest = joined.as_bytes();
        while let Some((&b, tail)) = rest.split_first() {
            let hex = (b == b'=' && tail.len() >= 2)
                .then(|| std::str::from_utf8(&tail[..2]).ok())
                .flatten()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            match hex {
                Some(byte) => {
                    bytes.push(byte);
                    rest = &tail[2..];
                }
                None => {
                    bytes.push(b);
                    rest = tail;
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn mailing_state(port: u16) -> AppState {
        let mut state = seeded_state().await;
        state.config.smtp_url = crate::config::Secret::new(format!("smtp://127.0.0.1:{port}"));
        state.config.mail_from = "Wiki <wiki@wiki.example>".to_string();
        state.config.app_origin = "https://wiki.example".to_string();
        state.mailer = Mailer::from_config(&state.config)
            .expect("a mailer")
            .map(Arc::new);
        state
    }

    async fn arrived(inbox: &Inbox, count: usize) -> Vec<(String, String)> {
        for _ in 0..100 {
            if inbox.lock().expect("inbox").len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        inbox.lock().expect("inbox").clone()
    }

    const INVITE: &str = "/xrpc/wiki.radikal.inviteMembers";
    const SEND: &str = "/xrpc/wiki.radikal.sendInvitation";

    #[test]
    fn an_invitation_says_where_to_and_how_in_both_languages() {
        let (subject, text) = invitation(
            "RadikalWiki",
            "Hovedbestyrelsen",
            "https://w.example/?claim=t",
        );
        assert_eq!(subject, "Invitation: Hovedbestyrelsen");
        assert_eq!(text.matches("https://w.example/?claim=t").count(), 2);
        assert!(text.contains("Du er inviteret til Hovedbestyrelsen på RadikalWiki"));
        assert!(text.contains("You have been invited to Hovedbestyrelsen on RadikalWiki"));
    }

    #[test]
    fn half_a_mail_configuration_is_refused() {
        let mut config = Config::default();
        assert!(Mailer::from_config(&config).expect("unset").is_none());
        config.mail_from = "Wiki <wiki@wiki.example>".to_string();
        assert!(
            Mailer::from_config(&config).is_err(),
            "a sender and no server"
        );
        config.smtp_url = crate::config::Secret::new("smtp://127.0.0.1:2525");
        assert!(
            Mailer::from_config(&config).is_err(),
            "no origin for the link"
        );
        config.app_origin = "https://wiki.example".to_string();
        assert!(Mailer::from_config(&config).expect("whole").is_some());
        config.mail_from = "not an address".to_string();
        assert!(Mailer::from_config(&config).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_invited_address_is_mailed_its_claim_link_and_an_account_is_not() {
        let (port, inbox) = smtp_server().await;
        let state = mailing_state(port).await;
        let alice = token_for(&state, "did:plc:alice").await;
        let roster = serde_json::json!({"context_id": "c9", "invites": [
            {"name": "Bo", "email": "Bo@Wiki.example"},
            {"did": "did:plc:zoe"},
            {"name": "Uden adresse"},
        ]});
        let (status, v) = post(router(state.clone()), INVITE, Some(&alice), roster).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            (&v["inserted"], &v["mailing"]),
            (&3.into(), &1.into()),
            "{v}"
        );

        let mails = arrived(&inbox, 1).await;
        assert_eq!(mails.len(), 1, "one seat has an address and no account");
        assert_eq!(mails[0].0, "<bo@wiki.example>");
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query(
                "SELECT id, claim_token, mailed_at FROM member WHERE email = 'bo@wiki.example'",
                (),
            )
            .await
            .expect("query");
        let row = rows.next().await.expect("rows").expect("the seat");
        let (seat, token): (String, String) = (row.get(0).expect("id"), row.get(1).expect("token"));
        drop(rows);
        let text = text_of(&mails[0].1);
        assert!(
            text.contains(&format!("https://wiki.example/?claim={token}")),
            "{text}"
        );
        assert!(mails[0].1.contains("Auto-Submitted: auto-generated"));

        // The roster says when, to an owner and to nobody else.
        let bob = token_for(&state, "did:plc:bob").await;
        let list = "/xrpc/wiki.radikal.listMembers?context=c9&q=Bo";
        let (_, owners) = get_as(router(state.clone()), list, &alice).await;
        assert!(owners["members"][0]["mailed_at"].is_string(), "{owners}");
        let (_, members) = get_as(router(state.clone()), list, &bob).await;
        assert!(
            members["members"][0].get("mailed_at").is_none(),
            "{members}"
        );

        // Again, for a mail that was lost: an owner's to ask, and not at once.
        let again = serde_json::json!({ "member": seat });
        let (status, _) = post(router(state.clone()), SEND, Some(&bob), again.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, v) = post(router(state.clone()), SEND, Some(&alice), again.clone()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
        conn.execute(
            "UPDATE member SET mailed_at = '2026-01-01T00:00:00.000Z' WHERE id = ?1",
            [seat.as_str()],
        )
        .await
        .expect("an hour passes");
        let (status, v) = post(router(state.clone()), SEND, Some(&alice), again).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(arrived(&inbox, 2).await.len(), 2);

        let mut rows = conn
            .query(
                "SELECT id FROM member WHERE user_did = 'did:plc:zoe' AND context_id = 'c9'",
                (),
            )
            .await
            .expect("query");
        let zoe: String = rows
            .next()
            .await
            .expect("rows")
            .expect("zoe")
            .get(0)
            .expect("id");
        let taken = serde_json::json!({ "member": zoe });
        let (status, _) = post(router(state.clone()), SEND, Some(&alice), taken).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "an account has nothing to claim"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_site_that_sends_no_mail_says_so() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let roster = serde_json::json!({"context_id": "c9", "invites": [
            {"name": "Bo", "email": "bo@wiki.example"},
        ]});
        let (_, v) = post(router(state.clone()), INVITE, Some(&alice), roster).await;
        assert_eq!(v["mailing"], 0, "{v}");
        let anyone = serde_json::json!({ "member": "m-x" });
        let (status, v) = post(router(state.clone()), SEND, Some(&alice), anyone).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "MailNotConfigured");
    }
}
