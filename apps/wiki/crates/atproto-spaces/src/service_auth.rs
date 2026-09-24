//! The token a PDS calls the AppView with: to ask whether a user may read or
//! write a space (`checkUserAccess`), and to say that a repo advanced
//! (`notifyWrite`). Signed by the calling account's `#atproto` key, addressed
//! to one service and good for one method.

use crate::directory::Directory;
use crate::{Error, jwt};

/// The DID that called, once everything about its token holds.
pub async fn caller(
    directory: &Directory,
    authorization: Option<&str>,
    audience: &str,
    method: &str,
) -> Result<String, Error> {
    let refuse = |why: &str| Error::Unverified(format!("service token: {why}"));
    let token = authorization
        .and_then(|header| header.strip_prefix("Bearer "))
        .ok_or_else(|| refuse("none"))?;
    let read = jwt::decode(token)?;
    let now = jwt::now();
    if read.claim("aud") != Some(audience) {
        return Err(refuse("addressed to someone else"));
    }
    if read.claim("lxm") != Some(method) {
        return Err(refuse("good for another method"));
    }
    if read.claims["exp"].as_u64().is_none_or(|exp| exp <= now) {
        return Err(refuse("run out"));
    }
    // A minute of clock skew, and no token from the future beyond it.
    if read.claims["iat"]
        .as_u64()
        .is_some_and(|iat| iat > now + 60)
    {
        return Err(refuse("not issued yet"));
    }
    let issuer = read.claim("iss").ok_or_else(|| refuse("no issuer"))?;
    // An issuer may name a key of its document by fragment; the account's own
    // is the only one a PDS signs these with.
    let did = issuer.split('#').next().unwrap_or(issuer);
    let identity = directory.resolve(did).await?;
    match read.signed_by(&identity.signing_key) {
        true => Ok(did.to_string()),
        false => Err(refuse("not signed by its issuer")),
    }
}
