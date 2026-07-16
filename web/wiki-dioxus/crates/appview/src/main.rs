//! AppView entrypoint: open the Turso datastore, build the router, and serve on
//! `$PORT` as a long-running process (NOT scale-to-zero serverless).

use appview::{AppState, Config, Db, router};

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
    let addr = format!("0.0.0.0:{}", config.port);
    let app = router(AppState::new(db, config));

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
