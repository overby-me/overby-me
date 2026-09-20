//! AppView entrypoint: open the Turso datastore, build the router, and serve on
//! `$PORT` as a long-running process (NOT scale-to-zero serverless).
//!
//! `appview import <extraction.json>` loads a migrated wiki instead, and exits;
//! `appview import-files <extraction.json> <dir>` then files its files.

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
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (None, _, _) => {}
        (Some("import"), Some(path), None) => import(&db, &path).await,
        (Some("import-files"), Some(path), Some(dir)) => {
            import_files(AppState::new(db, config), &path, &dir).await
        }
        _ => {
            eprintln!(
                "usage: appview [import <extraction.json> | import-files <extraction.json> <dir>]"
            );
            std::process::exit(2);
        }
    }
    if let Err(e) = appview::context::ensure_home(&db, &config).await {
        tracing::error!("failed to make sure the site has a home: {e}");
        std::process::exit(1);
    }
    // The atproto OAuth client (durable SQLite stores). A build failure here is
    // fatal: identity is load-bearing, so the process must not serve `/callback`
    // silently misconfigured.
    let oauth = match WikiOAuth::new(db.clone(), &config) {
        Ok(o) => Arc::new(o),
        Err(e) => {
            tracing::error!("failed to build the OAuth client: {e}");
            std::process::exit(1);
        }
    };
    appview::blob::sweep_incoming(&config).await;
    // In the background: a search in the first seconds finds what is indexed so
    // far, which beats nothing answering until it all is.
    let to_index = db.clone();
    tokio::spawn(async move {
        match appview::search::rebuild(&to_index).await {
            Ok(nodes) => tracing::info!("search index rebuilt over {nodes} nodes"),
            Err(e) => tracing::error!("search index rebuild failed: {e}"),
        }
    });
    // Fatal, like the OAuth client: a board that is configured to be replicated
    // and is not is the integrity control silently off.
    let replica = match appview::ballot::open_replica(&config.ballot_replica_log) {
        Ok(replica) => replica,
        Err(e) => {
            tracing::error!(
                "failed to open the ballot replica log at {}: {e}",
                config.ballot_replica_log
            );
            std::process::exit(1);
        }
    };
    // Fatal, as the two above: a site set up to mail its invitations that
    // cannot is one whose members wait for a mail that is not coming.
    let mailer = match appview::mail::Mailer::from_config(&config) {
        Ok(mailer) => mailer.map(Arc::new),
        Err(e) => {
            tracing::error!("mail is misconfigured: {e}");
            std::process::exit(1);
        }
    };
    let addr = format!("0.0.0.0:{}", config.port);
    let mut state = AppState::new(db, config).with_oauth(oauth);
    state.replica = replica;
    state.mailer = mailer;

    // The firehose consumer runs for the life of the process, materializing
    // public records into the view and broadcasting deltas to /ws clients. It
    // reconnects on its own, so a failed connection never blocks serving.
    tokio::spawn(appview::firehose::run(state.clone()));

    let app = router(state);

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

async fn import(db: &Db, path: &str) -> ! {
    match appview::import::import_file(db, path).await {
        Ok(stats) => {
            let loaded = &stats.entities;
            println!(
                "loaded: {} users ({} to be recognized by address), {} contexts, {} documents \
                 ({} author rows), {} members, {} comments, {} reactions, {} polls, \
                 {} canvases ({} cells), {} reports",
                loaded.users,
                stats.accounts,
                loaded.contexts,
                loaded.documents,
                loaded.document_authors,
                loaded.members,
                loaded.comments,
                loaded.reactions,
                stats.polls,
                stats.canvases,
                stats.cells,
                stats.feedback
            );
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("import failed, nothing was loaded: {e}");
            std::process::exit(1);
        }
    }
}

async fn import_files(state: AppState, path: &str, dir: &str) -> ! {
    let copied = async {
        let raw = tokio::fs::read(path).await?;
        let ex = serde_json::from_slice(&raw)?;
        Ok::<_, Box<dyn std::error::Error>>(
            appview::import::import_files(&state, &ex, std::path::Path::new(dir)).await?,
        )
    };
    match copied.await {
        Ok(stats) => {
            println!(
                "files: {} copied, {} here already, {} not copied, {} that nothing points at",
                stats.copied,
                stats.already,
                stats.failed.len(),
                stats.unreferenced.len()
            );
            for (id, why) in &stats.failed {
                println!("not copied: {id}: {why}");
            }
            for name in &stats.unreferenced {
                println!("nothing points at: {name}");
            }
            std::process::exit(i32::from(!stats.failed.is_empty()));
        }
        Err(e) => {
            eprintln!("import-files failed: {e}");
            std::process::exit(1);
        }
    }
}
