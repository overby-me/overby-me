//! An AppView to develop against: an empty in-memory wiki with a home, served on
//! a local port, with a session already made for every DID named on the command
//! line. Signing in takes a PDS and a browser; this takes neither, which is what
//! lets the frontend's data layer be tested from `cargo test`, and the frontend
//! be run on a laptop.
//!
//!   appview-dev [--port N] <did> [<did> ...]
//!
//! The first DID runs the site (owns the home). One line of JSON on stdout says
//! where it listens and which session is whose: `{"url": .., "sessions": {..}}`.
//! Never deployed: it mints sessions for anyone it is asked to.

use appview::{AppState, Config, Db, router};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut port = 0u16;
    if args.peek().map(String::as_str) == Some("--port") {
        args.next();
        port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    }
    let dids: Vec<String> = args.collect();
    if dids.is_empty() || dids.iter().any(|did| !did.starts_with("did:")) {
        eprintln!("usage: appview-dev [--port N] <did> [<did> ...]");
        std::process::exit(2);
    }

    // Bound first: the links the AppView hands out name where it listens.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("a port");
    let url = format!("http://{}", listener.local_addr().expect("an address"));

    let db = Db::open(":memory:").await.expect("an in-memory datastore");
    db.init_schema().await.expect("the schema");
    let config = Config {
        public_url: url.clone(),
        site_name: "Dev Wiki".to_string(),
        site_owner: dids.first().cloned(),
        // So that a frontend served from elsewhere on this machine may call it
        // (`APPVIEW_FRONTEND_ORIGINS`, as the real one reads it).
        frontend_origins: Config::from_env().frontend_origins,
        ..Config::default()
    };
    appview::context::ensure_home(&db, &config)
        .await
        .expect("a home");
    let state = AppState::new(db.clone(), config);

    let mut sessions = serde_json::Map::new();
    for did in &dids {
        appview::Store::new(db.clone())
            .upsert_user_min(did)
            .await
            .expect("a user");
        let session = appview::session::Sessions::new(db.clone())
            .create(did)
            .await
            .expect("a session");
        sessions.insert(did.clone(), serde_json::Value::String(session));
    }

    println!(
        "{}",
        serde_json::json!({ "url": url, "sessions": sessions })
    );
    axum::serve(listener, router(state)).await.expect("serving");
}
