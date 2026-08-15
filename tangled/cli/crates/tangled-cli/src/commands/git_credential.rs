//! A git credential helper for pushing to a knot over HTTP.
//!
//! A knot authorises an HTTP push with an atproto service-auth JWT, presented
//! either as a bearer token or as the password of a basic credential whose
//! user is `x-tangled-token`. The knot caps such a token's lifetime at 300
//! seconds, which is far too short to paste by hand, so the only comfortable
//! way to use HTTP push is to mint one per invocation. That is exactly what a
//! credential helper is for.
//!
//! Configure it for every knot you push to. The empty value first is not
//! decoration: it resets the inherited helper list for that URL. Without it a
//! global helper such as `store` runs first, writes the minted token to
//! ~/.git-credentials in plaintext, and then replays it after it has expired,
//! so every later push fails with `token expired at ...`.
//!
//! ```text
//! git config --global credential.https://knot1.tangled.sh.helper ''
//! git config --global --add credential.https://knot1.tangled.sh.helper \
//!     '!tangled-cli git-credential'
//! ```

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::io::Read;

use crate::cli::{Cli, GitCredentialArgs};

/// The lexicon method a knot requires a push token to be bound to.
const PUSH_LXM: &str = "sh.tangled.repo.push";
/// The username a knot expects when the token travels as a basic credential.
const TOKEN_USER: &str = "x-tangled-token";
/// Comfortably inside the knot's 300s ceiling, with room for a slow push.
const TOKEN_LIFETIME_SECS: i64 = 240;

pub async fn run(_cli: &Cli, args: GitCredentialArgs) -> Result<()> {
    // git only ever wants a credential from `get`. `store` and `erase` must
    // succeed silently: these tokens expire on their own and there is nothing
    // cached to forget.
    if args.operation != "get" {
        return Ok(());
    }

    let input = read_request()?;
    // A token's audience is the knot that will verify it. That is usually the
    // host git asked about, but not when the push goes through something else
    // (a local josh-proxy on localhost), so --knot overrides it.
    let host = match args.knot.clone().or_else(|| input.get("host").cloned()) {
        Some(h) if !h.is_empty() => h,
        // No host means nothing to mint against. Staying quiet lets git fall
        // through to another helper rather than failing the push.
        _ => return Ok(()),
    };

    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);

    // The host in the request is the knot; a token is bound to that audience.
    let token = client
        .knot_push_token(
            &pds,
            &session.access_jwt,
            host_without_port(&host),
            PUSH_LXM,
            TOKEN_LIFETIME_SECS,
        )
        .await
        .map_err(|e| anyhow!("could not mint a push token for {host}: {e}"))?;

    println!("username={TOKEN_USER}");
    println!("password={token}");
    Ok(())
}

/// git writes `key=value` lines then a blank line.
fn read_request() -> Result<HashMap<String, String>> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    Ok(raw
        .lines()
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

/// A service-auth audience is `did:web:<host>`, and a port would have to be
/// percent-encoded to belong in a DID. Knots are reached on 443 in practice.
fn host_without_port(host: &str) -> &str {
    host.split_once(':').map_or(host, |(h, _)| h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_port_from_the_audience_host() {
        assert_eq!(host_without_port("knot1.tangled.sh"), "knot1.tangled.sh");
        assert_eq!(host_without_port("localhost:5555"), "localhost");
    }
}
