//! AppView entrypoint: open the Turso datastore, build the router, and serve on
//! `$PORT` as a long-running process (NOT scale-to-zero serverless).

use appview::oauth::WikiOAuth;
use appview::{AppState, Config, Db, router};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Structured JSON logs to stdout (the deploy unit ships them to BetterStack).
    // Server-side tracing, not the browser-only `src/logging.rs`.
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    let db = match Db::open(&config.db_path).await {
        Ok(db) => db,
        Err(e) => {
            tracing::error!("failed to open datastore at {}: {e}", config.db_path);
            std::process::exit(1);
        }
    };
    // Create the entity + runtime schema on a fresh db (idempotent on an
    // already-initialized persistent file).
    if let Err(e) = db.init_schema().await {
        tracing::error!("failed to initialize schema: {e}");
        std::process::exit(1);
    }
    // The atproto OAuth client (durable SQLite stores). A build failure here is
    // fatal: identity is load-bearing, so the process must not serve `/callback`
    // silently misconfigured.
    let oauth = match WikiOAuth::new(db.clone()) {
        Ok(o) => Arc::new(o),
        Err(e) => {
            tracing::error!("failed to build the OAuth client: {e}");
            std::process::exit(1);
        }
    };
    let addr = format!("0.0.0.0:{}", config.port);
    let app = router(AppState::new(db, config).with_oauth(oauth));

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("appview listening on {addr}");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("server error: {e}");
        std::process::exit(1);
    }
}
