//! HTTP server startup and subsystem wiring.
//!
//! The [`run_server`] function initializes all subsystems (secrets, engine,
//! job queue, knot consumer, Jetstream client, orchestrator) and runs them
//! concurrently with graceful shutdown support.

use std::collections::HashSet;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::{Config, SecretsProvider};
use crate::notifier::Notifier;
use crate::orchestrator::{self, OrchestratorContext};
use crate::router::{AppState, build_router};

/// Errors that can occur during server startup or operation.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("secrets manager initialization failed: {0}")]
    Secrets(String),

    #[error("http server error: {0}")]
    Http(#[from] std::io::Error),
}

/// Start all subsystems and run until shutdown.
///
/// Initializes and runs concurrently:
/// 1. HTTP server (axum)
/// 2. Jetstream WebSocket consumer (AT Protocol events)
/// 3. Jetstream event ingestion processor
/// 4. Pipeline event processor (orchestrator)
/// 5. Job queue (bounded worker pool, started implicitly)
///
/// Shuts down gracefully on SIGTERM or SIGINT (Ctrl-C).
pub async fn run_server(
    cfg: Config,
    db: Arc<spindle_db::Database>,
    rbac: spindle_rbac::SpindleEnforcer,
) -> Result<(), ServerError> {
    let shutdown = CancellationToken::new();

    // Initialize secrets manager.
    let secrets: Arc<dyn spindle_secrets::Manager + Send + Sync> = match cfg.secrets.provider {
        SecretsProvider::Sqlite => {
            let key_material = ring::digest::digest(&ring::digest::SHA256, cfg.token.as_bytes());
            let master_key = key_material.as_ref();
            let db_path = cfg.db_path.with_extension("secrets.db");
            match spindle_secrets::SqliteManager::new(&db_path, master_key) {
                Ok(mgr) => Arc::new(mgr),
                Err(e) => return Err(ServerError::Secrets(e.to_string())),
            }
        }
        SecretsProvider::OpenBao => Arc::new(spindle_secrets::OpenBaoManager::new(
            &cfg.secrets.openbao.proxy_addr,
            &cfg.secrets.openbao.mount,
            &cfg.token,
        )),
    };

    // Create event notifier (broadcast channel).
    let notifier = Arc::new(Notifier::new(1024));

    // Create Nix engine.
    // Derive workspace and cache dirs from the state directory (db_path parent),
    // which is writable under systemd's StateDirectory sandbox.
    let state_dir = cfg
        .db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let workspace_root = state_dir.join("workspaces");
    let cache_dir = state_dir.join("cache");
    let engine = Arc::new(spindle_engine::NixEngine::new(
        workspace_root,
        cache_dir,
        cfg.engine.workflow_timeout,
        cfg.engine.extra_nix_flags.clone(),
        cfg.dev,
        spindle_engine::nix_engine::WorkflowLimits {
            memory_max: cfg.engine.workflow_limits.memory_max.clone(),
            tasks_max: cfg.engine.workflow_limits.tasks_max,
        },
    ));

    // Create job queue.
    let queue = Arc::new(spindle_queue::JobQueue::new(
        cfg.engine.max_jobs,
        cfg.engine.queue_size,
        shutdown.clone(),
    ));

    // Create pipeline event channel (knot consumer → orchestrator).
    let (pipeline_tx, mut pipeline_rx) = tokio::sync::mpsc::channel(256);

    // Create knot consumer.
    let knot_consumer = Arc::new(spindle_knot::KnotConsumer::new(
        Arc::clone(&db),
        pipeline_tx,
        shutdown.clone(),
    ));

    // Restore knot subscriptions from previous session.
    if let Err(e) = knot_consumer.restore_subscriptions().await {
        warn!(%e, "failed to restore knot subscriptions");
    }

    // Create Jetstream event channel.
    let (jetstream_tx, mut jetstream_rx) =
        tokio::sync::mpsc::channel::<spindle_jetstream::ParsedEvent>(256);

    // Load initial watched DIDs from database.
    let initial_dids: HashSet<String> = db.get_all_dids().unwrap_or_default().into_iter().collect();

    // Create Jetstream client.
    let jetstream_client = Arc::new(spindle_jetstream::JetstreamClient::new(
        &cfg.jetstream_endpoint,
        initial_dids,
        jetstream_tx,
    ));

    // Create ingestion context with knot subscriber adapter.
    let knot_adapter = Arc::new(orchestrator::KnotSubscriberAdapter(Arc::clone(
        &knot_consumer,
    )));
    let ingestion_ctx = Arc::new(spindle_jetstream::ingester::IngestionContext {
        hostname: cfg.hostname.clone(),
        db: Arc::clone(&db),
        rbac: Arc::new(rbac.clone()),
        did_web: cfg.did_web.clone(),
        knot_subscriber: knot_adapter,
    });

    // Create XRPC context.
    let xrpc = Arc::new(spindle_xrpc::XrpcContext {
        db: Arc::clone(&db),
        rbac,
        secrets: Arc::clone(&secrets),
        did_web: cfg.did_web.clone(),
        owner: cfg.owner.clone(),
        token: cfg.token.clone(),
        plc_url: cfg.plc_url.clone(),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client"),
        dev: cfg.dev,
    });

    // Create orchestrator context.
    let orch_ctx = Arc::new(OrchestratorContext {
        db: Arc::clone(&db),
        engine,
        secrets,
        queue,
        notifier: Arc::clone(&notifier),
        hostname: cfg.hostname.clone(),
        did_web: cfg.did_web.clone(),
        log_dir: cfg.log_dir.clone(),
        dev: cfg.dev,
    });

    // Build application state for the HTTP router.
    let state = Arc::new(AppState {
        db: Arc::clone(&db),
        notifier,
        xrpc,
        log_dir: cfg.log_dir.clone(),
        hostname: cfg.hostname.clone(),
    });

    let app = build_router(state);

    // Bind HTTP listener.
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await?;
    let local_addr = listener.local_addr()?;

    info!(
        listen_addr = %local_addr,
        hostname = %cfg.hostname,
        "HTTP server listening"
    );

    // -----------------------------------------------------------------------
    // Spawn concurrent subsystems
    // -----------------------------------------------------------------------

    // 1. HTTP server
    let http_shutdown = shutdown.clone();
    let http_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(http_shutdown.cancelled_owned())
            .await
            .map_err(|e| error!(%e, "HTTP server error"))
            .ok();
        info!("HTTP server shut down");
    });

    // 2. Jetstream consumer
    let js_client = Arc::clone(&jetstream_client);
    let js_db = Arc::clone(&db);
    let js_shutdown = shutdown.clone();
    let js_handle = tokio::spawn(async move {
        let cursor = js_db.get_last_time_us().unwrap_or(0);
        info!(cursor, "starting Jetstream consumer");

        let db_for_cursor = js_db;
        js_client
            .run(
                cursor,
                move |time_us| {
                    db_for_cursor
                        .save_last_time_us(time_us)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                },
                js_shutdown,
            )
            .await;

        info!("Jetstream consumer shut down");
    });

    // 3. Jetstream event ingestion processor
    let ingest_ctx = Arc::clone(&ingestion_ctx);
    let ingest_shutdown = shutdown.clone();
    let ingest_handle = tokio::spawn(async move {
        info!("Jetstream ingestion processor started");
        loop {
            tokio::select! {
                event = jetstream_rx.recv() => {
                    match event {
                        Some(parsed) => {
                            if let Err(e) = spindle_jetstream::ingester::ingest_event(&ingest_ctx, parsed).await {
                                warn!(%e, "failed to ingest Jetstream event");
                            }
                        }
                        None => {
                            info!("Jetstream event channel closed");
                            break;
                        }
                    }
                }
                () = ingest_shutdown.cancelled() => {
                    info!("Jetstream ingestion processor shutting down");
                    break;
                }
            }
        }
    });

    // 4. Pipeline event processor (orchestrator)
    let orch = Arc::clone(&orch_ctx);
    let pipeline_shutdown = shutdown.clone();
    let pipeline_handle = tokio::spawn(async move {
        info!("pipeline event processor started");
        loop {
            tokio::select! {
                event = pipeline_rx.recv() => {
                    match event {
                        Some(pipeline_event) => {
                            let ctx = Arc::clone(&orch);
                            orchestrator::process_pipeline_event(ctx, pipeline_event);
                        }
                        None => {
                            info!("pipeline event channel closed");
                            break;
                        }
                    }
                }
                () = pipeline_shutdown.cancelled() => {
                    info!("pipeline event processor shutting down");
                    break;
                }
            }
        }
    });

    // -----------------------------------------------------------------------
    // Wait for shutdown signal, then cancel all subsystems
    // -----------------------------------------------------------------------

    shutdown_signal().await;
    info!("shutdown signal received, stopping subsystems");
    shutdown.cancel();

    // Wait for all tasks to complete.
    let _ = tokio::join!(http_handle, js_handle, ingest_handle, pipeline_handle);

    info!("all subsystems shut down gracefully");
    Ok(())
}

/// Wait for a shutdown signal (SIGTERM or Ctrl-C).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("received Ctrl-C, shutting down");
        }
        () = terminate => {
            info!("received SIGTERM, shutting down");
        }
    }
}
