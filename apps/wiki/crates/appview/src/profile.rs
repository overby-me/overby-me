//! What a member's own PDS says about them, read once they have signed in: the
//! handle, name and picture a page shows, and the address their invitations
//! were sent to.
//!
//! The address is what makes the roster work. It was imported as names and
//! email addresses, 83 percent of it with nothing else, and a member has to be
//! found in it somehow. An invitation to a CONFIRMED address is handed to
//! whoever signs in with the account holding it.
//!
//! That takes a PDS's word for "confirmed", and anyone can run a PDS. One that
//! lies would walk its owner into another person's invitations, voting rights
//! included. So the word is taken only from the hosts configured as trusted
//! (`Config::trusted_email_pds`), and everyone else uses a claim link.

use crate::AppState;
use crate::config::Config;
use crate::db::DbError;
use crate::live::Topic;
use turso::Value;

const MAX_NAME_CHARS: usize = 200;

#[derive(Debug, Default, PartialEq)]
pub struct PdsAccount {
    /// The PDS that answered, which the login bound to the DID.
    pub pds: String,
    pub handle: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// The account's address, only if its PDS says it is confirmed.
    pub confirmed_email: Option<String>,
}

/// Read an account out of what its PDS answered: `getSession`, and the
/// `app.bsky.actor.profile` record if there is one.
pub fn account_from(
    did: &str,
    pds: &str,
    session: &serde_json::Value,
    profile: Option<&serde_json::Value>,
) -> PdsAccount {
    let pds = pds.trim_end_matches('/').to_string();
    let string = |value: Option<&serde_json::Value>| {
        value
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // A blob is `{"$type":"blob","ref":{"$link":<cid>}}`, or before 2023
    // `{"cid":<cid>}`. It is served by the PDS that holds the repo.
    let avatar = profile.and_then(|p| p.get("avatar"));
    let cid = string(avatar.and_then(|a| a.pointer("/ref/$link")))
        .or_else(|| string(avatar.and_then(|a| a.get("cid"))))
        .filter(|cid| cid.chars().all(|c| c.is_ascii_alphanumeric()));
    let served = pds.starts_with("https://") || pds.starts_with("http://");
    PdsAccount {
        // A PDS says `handle.invalid` for a handle that no longer resolves.
        handle: string(session.get("handle")).filter(|h| h != "handle.invalid"),
        display_name: string(profile.and_then(|p| p.get("displayName")))
            .map(|name| name.chars().take(MAX_NAME_CHARS).collect()),
        avatar_url: cid
            .filter(|_| served)
            .map(|cid| format!("{pds}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}")),
        confirmed_email: (session.get("emailConfirmed") == Some(&serde_json::Value::Bool(true)))
            .then(|| crate::store::normalized_email(session.get("email").and_then(|e| e.as_str())))
            .flatten(),
        pds,
    }
}

/// Whether `pds` is one whose word on a confirmed address is taken.
pub fn trusts_email_of(config: &Config, pds: &str) -> bool {
    let Some(rest) = pds.strip_prefix("https://") else {
        return false;
    };
    let host = rest
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    config
        .trusted_email_pds
        .iter()
        .any(|trusted| match trusted.strip_prefix('.') {
            Some(_) => host.ends_with(trusted.as_str()),
            None => host == *trusted,
        })
}

/// Record what the PDS said, and hand `did` the invitations sent to their
/// address. Returns how many invitations that was.
pub async fn apply(state: &AppState, did: &str, account: &PdsAccount) -> Result<u64, DbError> {
    let conn = state.db.acquire().await?;
    let or_keep = |value: &Option<String>| value.clone().map_or(Value::Null, Value::Text);
    // What the PDS did not say is left as it was: an account with no profile
    // record keeps the name its roster row gave it.
    conn.execute(
        "UPDATE user SET handle = coalesce(?2, handle), \
           display_name = coalesce(?3, display_name), avatar_url = coalesce(?4, avatar_url) \
         WHERE did = ?1",
        vec![
            Value::Text(did.to_string()),
            or_keep(&account.handle),
            or_keep(&account.display_name),
            or_keep(&account.avatar_url),
        ],
    )
    .await?;
    let Some(email) = account
        .confirmed_email
        .as_deref()
        .filter(|_| trusts_email_of(&state.config, &account.pds))
    else {
        return Ok(0);
    };
    // Not where they already have a seat: a context holds a person once.
    let bound = conn
        .execute(
            "UPDATE member SET user_did = ?1 \
             WHERE user_did IS NULL AND email = ?2 \
               AND NOT EXISTS (SELECT 1 FROM member seated \
                               WHERE seated.context_id = member.context_id \
                                 AND seated.user_did = ?1)",
            [did, email],
        )
        .await?;
    if bound > 0 {
        state.publish(Topic::User(did.to_string()), "invitation", did);
    }
    Ok(bound)
}

/// Ask `did`'s PDS who they are, and act on it. Run beside a login and never in
/// its way, so every failure ends here, in the log.
pub async fn hydrate(state: AppState, did: String) {
    let Some(oauth) = state.oauth.clone() else {
        return;
    };
    let account = match oauth.account(&did).await {
        Ok(account) => account,
        Err(e) => {
            tracing::warn!("could not read the account of {did} from its PDS: {e}");
            return;
        }
    };
    match apply(&state, &did, &account).await {
        Ok(0) => {}
        Ok(bound) => tracing::info!("{did} signed in and found {bound} invitations waiting"),
        Err(e) => tracing::error!("could not record the account of {did}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xrpc::tests::{seeded_state, token_for};
    use serde_json::json;

    const DID: &str = "did:plc:carol";

    fn pds() -> String {
        format!("https://{}", "morel.us-east.host.bsky.network")
    }

    fn session(confirmed: bool) -> serde_json::Value {
        json!({
            "did": DID, "handle": "carol.example", "email": " Carol@Example.ORG ",
            "emailConfirmed": confirmed, "active": true
        })
    }

    #[test]
    fn an_account_is_read_out_of_what_its_pds_answered() {
        let profile = json!({
            "$type": "app.bsky.actor.profile", "displayName": "  Carol Ørsted ",
            "avatar": {"$type": "blob", "ref": {"$link": "bafkreiabc123"}, "mimeType": "image/jpeg", "size": 1}
        });
        let account = account_from(DID, &format!("{}/", pds()), &session(true), Some(&profile));
        assert_eq!(account.handle.as_deref(), Some("carol.example"));
        assert_eq!(account.display_name.as_deref(), Some("Carol Ørsted"));
        assert_eq!(
            account.avatar_url,
            Some(format!(
                "{}/xrpc/com.atproto.sync.getBlob?did={DID}&cid=bafkreiabc123",
                pds()
            ))
        );
        assert_eq!(
            account.confirmed_email.as_deref(),
            Some("carol@example.org")
        );

        let bare = account_from(DID, &pds(), &session(false), None);
        assert_eq!(bare.display_name, None);
        assert_eq!(bare.avatar_url, None);
        assert_eq!(
            bare.confirmed_email, None,
            "an unconfirmed address is nobody's yet"
        );

        let old = json!({"avatar": {"cid": "bafyold", "mimeType": "image/png"}});
        assert!(
            account_from(DID, &pds(), &session(true), Some(&old))
                .avatar_url
                .is_some()
        );
        let odd = json!({"avatar": {"ref": {"$link": "x&did=did:plc:someone-else"}}});
        assert_eq!(
            account_from(DID, &pds(), &session(true), Some(&odd)).avatar_url,
            None
        );
        let gone = json!({"handle": "handle.invalid"});
        assert_eq!(account_from(DID, &pds(), &gone, None).handle, None);
    }

    #[test]
    fn only_a_configured_pds_is_believed_about_an_address() {
        let config = Config::default();
        let at = |host: &str| format!("https://{host}");
        assert!(trusts_email_of(&config, &at("bsky.social")));
        assert!(trusts_email_of(&config, &pds()));
        assert!(trusts_email_of(
            &config,
            &at("Morel.US-East.host.bsky.network:443/")
        ));
        for not in [
            at("pds.example"),
            at("evilbsky.social"),
            at("host.bsky.network.evil.example"),
            at("bsky.social.evil.example"),
            format!("http://{}", "bsky.social"),
        ] {
            assert!(!trusts_email_of(&config, &not), "{not}");
        }
    }

    async fn invited() -> AppState {
        let state = seeded_state().await;
        token_for(&state, DID).await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO member (id, context_id, email, name, active) \
               VALUES ('inv-9', 'c9', 'carol@example.org', 'Carol', 1);
             INSERT INTO member (id, context_id, email, name) \
               VALUES ('inv-1', 'c1', 'carol@example.org', 'Carol');
             INSERT INTO member (id, context_id, email) VALUES ('inv-x', 'c10', 'someone@else.example');
             INSERT INTO member (id, context_id, user_did) VALUES ('seat-1', 'c1', 'did:plc:carol');",
        )
        .await
        .expect("seed");
        state
    }

    async fn bound_rows(state: &AppState) -> Vec<String> {
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query(
                "SELECT id FROM member WHERE user_did = ?1 ORDER BY id",
                [DID],
            )
            .await
            .expect("q");
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await.expect("next") {
            ids.push(row.get::<String>(0).expect("id"));
        }
        ids
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_confirmed_address_finds_its_invitations_and_a_profile_is_kept() {
        let state = invited().await;
        let mut changes = state.changes.subscribe();
        let account = account_from(
            DID,
            &pds(),
            &session(true),
            Some(&json!({"displayName": "Carol"})),
        );
        assert_eq!(apply(&state, DID, &account).await.expect("apply"), 1);
        assert_eq!(
            bound_rows(&state).await,
            ["inv-9", "seat-1"],
            "c1 already seats her, so its invitation is left for an owner to clear"
        );
        assert_eq!(changes.try_recv().expect("told").kind, "invitation");

        let user = crate::Store::new(state.db.clone())
            .read_user(DID)
            .await
            .expect("read")
            .expect("user");
        assert_eq!(user.handle.as_deref(), Some("carol.example"));
        assert_eq!(user.display_name.as_deref(), Some("Carol"));

        // Signing in again, from an account with no profile, forgets nothing.
        let later = account_from(DID, &pds(), &session(true), None);
        assert_eq!(apply(&state, DID, &later).await.expect("apply"), 0);
        let user = crate::Store::new(state.db.clone())
            .read_user(DID)
            .await
            .expect("read")
            .expect("user");
        assert_eq!(user.display_name.as_deref(), Some("Carol"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_address_no_trusted_pds_vouches_for_binds_nothing() {
        let state = invited().await;
        let unconfirmed = account_from(DID, &pds(), &session(false), None);
        assert_eq!(apply(&state, DID, &unconfirmed).await.expect("apply"), 0);
        // Anyone can run a PDS, and this one says what suits its owner.
        let own = format!("https://{}", "pds.mallory.example");
        let claimed = account_from(DID, &own, &session(true), None);
        assert_eq!(apply(&state, DID, &claimed).await.expect("apply"), 0);
        assert_eq!(
            bound_rows(&state).await,
            ["seat-1"],
            "an invitation was walked into"
        );
    }
}
