//! Drives the WikiOAuth wrapper against two DIFFERENT PDS hosts to prove the
//! flow is PDS-agnostic. Prints how far each identity's login gets (the
//! authorization URL, or the error), then stops: completing the code exchange
//! needs a real browser redirect. Needs network.
//!
//! Usage: oauth-spike <handle-or-pds> [<handle-or-pds> ...]
//! Defaults probe bsky.social and a non-Bluesky host.

use oauth_spike::WikiOAuth;

#[tokio::main]
async fn main() {
    let targets: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            vec!["bsky.social".into(), "bsky.app".into()]
        } else {
            args
        }
    };
    let oauth = match WikiOAuth::new() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("client build FAILED: {e}");
            std::process::exit(1);
        }
    };
    for t in targets {
        match oauth.begin_login(&t).await {
            Ok(url) => {
                let host = url.split('/').nth(2).unwrap_or("?");
                println!("OK   {t}: authorize URL issued (authorization server: {host})");
            }
            Err(e) => println!("FAIL {t}: {e}"),
        }
    }
}
