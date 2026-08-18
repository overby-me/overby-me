//! Network integration test for the PDS-agnostic login flow. `#[ignore]` by
//! default (it hits the live network and third-party PDSes, so it never runs
//! in the offline `cargo test`); run explicitly with:
//!
//!   cargo test -p oauth-spike -- --ignored --nocapture
//!
//! It asserts what the spike proved on 2026-07-16: `begin_login` completes the
//! full server-side pre-redirect flow (handle -> DID -> PDS resolution, PAR,
//! DPoP, PKCE) and issues an authorization URL against an INDEPENDENT,
//! non-Bluesky PDS, not just bsky.social.

use oauth_spike::WikiOAuth;

#[tokio::test]
#[ignore = "hits the live network and third-party PDSes"]
async fn login_works_against_bluesky_and_an_independent_pds() {
    let oauth = WikiOAuth::new().expect("client builds");

    // A Bluesky-hosted identity: the mainstream path.
    let bsky = oauth
        .begin_login("bsky.app")
        .await
        .expect("bsky.social identity resolves and PARs");
    assert!(bsky.starts_with("https://"), "authorize URL: {bsky}");

    // A self-hosted independent PDS acting as its own authorization server:
    // this is the load-bearing PDS-agnostic assertion.
    let indie = oauth
        .begin_login("https://pds.witchcraft.systems")
        .await
        .expect("an independent PDS entryway resolves and PARs");
    assert!(
        indie.contains("pds.witchcraft.systems"),
        "independent auth server: {indie}"
    );
}
