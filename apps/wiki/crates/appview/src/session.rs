//! Browser sessions.
//!
//! An atproto OAuth login leaves DPoP-bound PDS tokens in `oauth_session`. They
//! are bound to a key this process holds, so a browser cannot use them: it gets
//! an opaque bearer token instead. Only the token's SHA-256 is stored, so a
//! leaked database cannot be replayed as logins.

use crate::AppState;
use crate::db::{Db, DbError};
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use sha2::{Digest, Sha256};
use turso::Value;

pub const TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// Long enough for a redirect and one POST, short enough that a code read out
/// of browser history later is already dead.
pub const CODE_TTL_SECS: u64 = 60;

#[derive(Clone)]
pub struct Sessions {
    db: Db,
}

impl Sessions {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Start a session for `did` and return its bearer token. The token is not
    /// recoverable afterwards.
    pub async fn create(&self, did: &str) -> Result<String, DbError> {
        self.create_at(did, crate::util::now_secs()).await
    }

    async fn create_at(&self, did: &str, now: u64) -> Result<String, DbError> {
        let token = crate::util::random_token(32);
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO session (token_hash, did, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            vec![
                Value::Text(token_hash(&token)),
                Value::Text(did.to_string()),
                secs(now),
                secs(now.saturating_add(TTL_SECS)),
            ],
        )
        .await?;
        Ok(token)
    }

    /// The DID a token belongs to, or `None` if it is unknown or has expired.
    pub async fn resolve(&self, token: &str) -> Result<Option<String>, DbError> {
        self.resolve_at(token, crate::util::now_secs()).await
    }

    async fn resolve_at(&self, token: &str, now: u64) -> Result<Option<String>, DbError> {
        let hash = token_hash(token);
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT did, expires_at FROM session WHERE token_hash = ?1",
                [hash.as_str()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let did = row.get::<String>(0)?;
        let expires_at = row.get::<i64>(1)?;
        drop(rows);
        if expires_at <= secs_i64(now) {
            conn.execute("DELETE FROM session WHERE token_hash = ?1", [hash.as_str()])
                .await?;
            return Ok(None);
        }
        Ok(Some(did))
    }

    pub async fn revoke(&self, token: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute(
            "DELETE FROM session WHERE token_hash = ?1",
            [token_hash(token).as_str()],
        )
        .await?;
        Ok(())
    }

    /// Drop every expired session and login code. `resolve` and `redeem_code`
    /// already drop the ones they meet; this clears those nobody presents again.
    pub async fn purge_expired(&self) -> Result<u64, DbError> {
        let conn = self.db.acquire().await?;
        let now = secs(crate::util::now_secs());
        let sessions = conn
            .execute(
                "DELETE FROM session WHERE expires_at <= ?1",
                vec![now.clone()],
            )
            .await?;
        let codes = conn
            .execute("DELETE FROM login_code WHERE expires_at <= ?1", vec![now])
            .await?;
        Ok(sessions + codes)
    }

    /// A one-time code a completed login can be redeemed with, once, within
    /// [`CODE_TTL_SECS`].
    pub async fn issue_code(&self, did: &str) -> Result<String, DbError> {
        self.issue_code_at(did, crate::util::now_secs()).await
    }

    async fn issue_code_at(&self, did: &str, now: u64) -> Result<String, DbError> {
        let code = crate::util::random_token(32);
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO login_code (code_hash, did, expires_at) VALUES (?1, ?2, ?3)",
            vec![
                Value::Text(token_hash(&code)),
                Value::Text(did.to_string()),
                secs(now.saturating_add(CODE_TTL_SECS)),
            ],
        )
        .await?;
        Ok(code)
    }

    /// Spend a login code: the DID it was issued for, or `None` if it is unknown,
    /// expired, or already spent.
    pub async fn redeem_code(&self, code: &str) -> Result<Option<String>, DbError> {
        self.redeem_code_at(code, crate::util::now_secs()).await
    }

    async fn redeem_code_at(&self, code: &str, now: u64) -> Result<Option<String>, DbError> {
        let hash = token_hash(code);
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT did, expires_at FROM login_code WHERE code_hash = ?1",
                [hash.as_str()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let did = row.get::<String>(0)?;
        let expires_at = row.get::<i64>(1)?;
        drop(rows);
        // The DELETE is what spends the code: of two racing redeemers only one
        // deletes a row, and only that one is given the DID.
        let spent = conn
            .execute(
                "DELETE FROM login_code WHERE code_hash = ?1",
                [hash.as_str()],
            )
            .await?;
        if spent == 0 || expires_at <= secs_i64(now) {
            return Ok(None);
        }
        Ok(Some(did))
    }
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn secs_i64(t: u64) -> i64 {
    i64::try_from(t).unwrap_or(i64::MAX)
}

fn secs(t: u64) -> Value {
    Value::Integer(secs_i64(t))
}

/// The `Authorization: Bearer` credential, if the request carries one.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// The authenticated caller. Extracting it rejects the request with 401 unless
/// it carries a live session.
pub struct Caller {
    pub did: String,
}

/// The caller when there is one. No credential is anonymous; a credential that
/// does not resolve is still a 401, so an expired session is never quietly
/// served the signed-out view.
pub struct MaybeCaller(pub Option<Caller>);

impl MaybeCaller {
    pub fn did(&self) -> Option<&str> {
        self.0.as_ref().map(|caller| caller.did.as_str())
    }
}

impl FromRequestParts<AppState> for MaybeCaller {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let Some(token) = bearer(&parts.headers) else {
            return Ok(MaybeCaller(None));
        };
        match Sessions::new(state.db.clone()).resolve(token).await {
            Ok(Some(did)) => Ok(MaybeCaller(Some(Caller { did }))),
            Ok(None) => Err(crate::xrpc::err(
                StatusCode::UNAUTHORIZED,
                "InvalidToken",
                "the session is unknown or has expired",
            )),
            Err(e) => {
                tracing::error!("session lookup failed: {e}");
                Err(crate::xrpc::err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError",
                    "session lookup failed",
                ))
            }
        }
    }
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        match MaybeCaller::from_request_parts(parts, state).await?.0 {
            Some(caller) => Ok(caller),
            None => Err(crate::xrpc::err(
                StatusCode::UNAUTHORIZED,
                "AuthRequired",
                "this method needs a session",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn sessions() -> Sessions {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        Sessions::new(db)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_token_resolves_to_its_did_until_revoked() {
        let s = sessions().await;
        let token = s.create("did:plc:alice").await.expect("create");
        assert_eq!(
            s.resolve(&token).await.expect("resolve").as_deref(),
            Some("did:plc:alice")
        );
        assert!(
            s.resolve("not-a-token").await.expect("resolve").is_none(),
            "an unknown token must not resolve"
        );
        s.revoke(&token).await.expect("revoke");
        assert!(
            s.resolve(&token).await.expect("resolve").is_none(),
            "a revoked token must not resolve"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_token_itself_is_never_stored() {
        let s = sessions().await;
        let token = s.create("did:plc:alice").await.expect("create");
        let conn = s.db.acquire().await.expect("conn");
        let mut rows = conn
            .query("SELECT token_hash FROM session", ())
            .await
            .expect("query");
        let row = rows.next().await.expect("next").expect("one row");
        let stored = row.get::<String>(0).expect("text");
        assert_ne!(stored, token, "the bearer token was stored in the clear");
        assert_eq!(stored, token_hash(&token));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_session_ends_at_its_expiry_and_is_dropped() {
        let s = sessions().await;
        let token = s.create_at("did:plc:alice", 1_000).await.expect("create");
        let last = 1_000 + TTL_SECS - 1;
        assert!(
            s.resolve_at(&token, last).await.expect("resolve").is_some(),
            "a session must last its whole lifetime"
        );
        assert!(
            s.resolve_at(&token, last + 1)
                .await
                .expect("resolve")
                .is_none(),
            "a session must end at its expiry"
        );
        assert!(
            s.resolve_at(&token, 1_000)
                .await
                .expect("resolve")
                .is_none(),
            "an expired session must be deleted, not merely hidden"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn purge_drops_only_the_expired() {
        let s = sessions().await;
        let dead = s.create_at("did:plc:old", 0).await.expect("create");
        let live = s.create("did:plc:new").await.expect("create");
        assert_eq!(s.purge_expired().await.expect("purge"), 1);
        assert!(s.resolve(&dead).await.expect("resolve").is_none());
        assert!(s.resolve(&live).await.expect("resolve").is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_login_code_is_spent_by_its_first_use() {
        let s = sessions().await;
        let code = s.issue_code("did:plc:alice").await.expect("issue");
        assert_eq!(
            s.redeem_code(&code).await.expect("redeem").as_deref(),
            Some("did:plc:alice")
        );
        assert!(
            s.redeem_code(&code).await.expect("redeem").is_none(),
            "a login code must not be redeemable twice"
        );
        assert!(
            s.redeem_code("not-a-code").await.expect("redeem").is_none(),
            "an unknown code must not redeem"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_login_code_dies_with_its_minute_and_is_still_spent() {
        let s = sessions().await;
        let code = s
            .issue_code_at("did:plc:alice", 1_000)
            .await
            .expect("issue");
        assert!(
            s.redeem_code_at(&code, 1_000 + CODE_TTL_SECS)
                .await
                .expect("redeem")
                .is_none(),
            "an expired code must not redeem"
        );
        assert!(
            s.redeem_code_at(&code, 1_000)
                .await
                .expect("redeem")
                .is_none(),
            "a late attempt must still spend the code"
        );
    }

    #[test]
    fn bearer_reads_only_a_bearer_credential() {
        let with = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(AUTHORIZATION, v.parse().expect("header"));
            h
        };
        assert_eq!(bearer(&with("Bearer abc")), Some("abc"));
        assert_eq!(bearer(&with("Bearer   abc  ")), Some("abc"));
        assert_eq!(bearer(&with("Basic abc")), None);
        assert_eq!(bearer(&with("Bearer ")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }
}
