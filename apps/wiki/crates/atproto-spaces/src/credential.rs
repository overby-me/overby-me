//! A space credential, and the key it is bound to: what an application reads a
//! whole space with, for two hours (proposal 0016, "Credential flow").

use crate::client::{Auth, Host};
use crate::dpop::DpopKey;
use crate::{Error, jwt};

pub struct Credential {
    token: String,
    key: DpopKey,
    /// Unix seconds, read from the token.
    pub expires_at: u64,
}

impl Credential {
    /// Through one user's session: a delegation token from THEIR PDS, spent at
    /// the space's host for a credential bound to a fresh key. Any one user of
    /// a space will do for an application that serves many.
    pub async fn obtain(
        users_pds: &Host,
        session: &str,
        space_host: &Host,
        space: &str,
        attestation: Option<&str>,
    ) -> Result<Credential, Error> {
        let delegation = users_pds.delegation_token(session, space).await?;
        let key = DpopKey::generate();
        let token = space_host
            .space_credential(&delegation, space, &key, attestation)
            .await?;
        let read = jwt::decode(&token)?;
        // Not verified here: the hosts it is shown to do that. What matters to
        // its holder is that it is for this space, bound to this key, and when
        // it runs out.
        if read.claim("sub") != Some(space)
            || read.claims["cnf"]["jkt"].as_str() != Some(&key.thumbprint())
        {
            return Err(Error::Unverified(
                "a credential for another space or another key".into(),
            ));
        }
        Ok(Credential {
            expires_at: read.claims["exp"].as_u64().unwrap_or(0),
            token,
            key,
        })
    }

    pub fn auth(&self) -> Auth<'_> {
        Auth::Space(&self.token, &self.key)
    }

    /// Whether it should be replaced before `margin` seconds from now.
    pub fn runs_out_within(&self, margin: u64) -> bool {
        jwt::now() + margin >= self.expires_at
    }
}
