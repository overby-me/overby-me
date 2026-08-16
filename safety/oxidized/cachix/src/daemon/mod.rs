use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::api::client::ApiClient;
use crate::api::types::*;
use crate::config::PushCredential;
use crate::push::PushOptions;

/// Daemon configuration options.
#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub allow_remote_stop: bool,
    pub keep_alive_interval_secs: u64,
    pub keep_alive_timeout_secs: u64,
    pub narinfo_batch_size: usize,
    pub narinfo_batch_timeout_secs: f64,
    pub narinfo_cache_ttl_secs: u64,
    pub narinfo_max_cache_size: usize,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/tmp/cachix-daemon.sock"),
            allow_remote_stop: true,
            keep_alive_interval_secs: 30,
            keep_alive_timeout_secs: 180,
            narinfo_batch_size: 100,
            narinfo_batch_timeout_secs: 0.5,
            narinfo_cache_ttl_secs: 300,
            narinfo_max_cache_size: 0,
        }
    }
}

/// Run the daemon server.
pub async fn run_daemon(
    api: ApiClient,
    cache_name: String,
    credential: PushCredential,
    push_opts: PushOptions,
    daemon_opts: DaemonOptions,
) -> Result<()> {
    // Clean up existing socket
    if daemon_opts.socket_path.exists() {
        std::fs::remove_file(&daemon_opts.socket_path)?;
    }

    if let Some(parent) = daemon_opts.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&daemon_opts.socket_path)
        .with_context(|| format!("failed to bind to {}", daemon_opts.socket_path.display()))?;

    tracing::info!("daemon listening on {}", daemon_opts.socket_path.display());

    let running = Arc::new(tokio::sync::watch::Sender::new(true));

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, _) = accept.context("accept failed")?;
                let api = api.clone();
                let cache_name = cache_name.clone();
                let credential = credential.clone();
                let push_opts = push_opts.clone();
                let allow_stop = daemon_opts.allow_remote_stop;
                let running = running.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, &api, &cache_name, &credential, &push_opts, allow_stop, &running).await {
                        tracing::error!("client handler error: {e:#}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down daemon");
                break;
            }
        }

        if !*running.borrow() {
            tracing::info!("daemon stopped by remote request");
            break;
        }
    }

    // Cleanup socket
    let _ = std::fs::remove_file(&daemon_opts.socket_path);
    Ok(())
}

async fn handle_client(
    stream: UnixStream,
    api: &ApiClient,
    cache_name: &str,
    credential: &PushCredential,
    push_opts: &PushOptions,
    allow_stop: bool,
    running: &tokio::sync::watch::Sender<bool>,
) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let reader = BufReader::new(reader);
    let writer = Arc::new(Mutex::new(writer));
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let msg: DaemonClientMessage = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(e) => {
                send_message(
                    &writer,
                    &DaemonServerMessage::DaemonError(DaemonError {
                        message: format!("invalid message: {e}"),
                    }),
                )
                .await?;
                continue;
            }
        };

        match msg {
            DaemonClientMessage::ClientPing => {
                send_message(&writer, &DaemonServerMessage::DaemonPong).await?;
            }
            DaemonClientMessage::ClientStop => {
                if allow_stop {
                    send_message(
                        &writer,
                        &DaemonServerMessage::DaemonExit(DaemonExitInfo {
                            exit_code: 0,
                            exit_message: "stopped by client".to_string(),
                        }),
                    )
                    .await?;
                    running.send(false).ok();
                    return Ok(());
                } else {
                    send_message(
                        &writer,
                        &DaemonServerMessage::DaemonError(DaemonError {
                            message: "remote stop is disabled".to_string(),
                        }),
                    )
                    .await?;
                }
            }
            DaemonClientMessage::ClientPushRequest(req) => {
                let api = api.clone();
                let cache_name = cache_name.to_string();
                let credential = credential.clone();
                let push_opts = push_opts.clone();
                let writer = writer.clone();
                let subscribe = req.subscribe_to_updates;

                tokio::spawn(async move {
                    for path in &req.store_paths {
                        if subscribe {
                            let _ = send_message(
                                &writer,
                                &DaemonServerMessage::DaemonPushEvent(PushEvent::PushStarted {
                                    path: path.clone(),
                                }),
                            )
                            .await;
                        }

                        match crate::push::push_store_path(
                            &api,
                            &cache_name,
                            path,
                            &credential,
                            &push_opts,
                        )
                        .await
                        {
                            Ok(()) => {
                                if subscribe {
                                    let _ = send_message(
                                        &writer,
                                        &DaemonServerMessage::DaemonPushEvent(
                                            PushEvent::PushCompleted { path: path.clone() },
                                        ),
                                    )
                                    .await;
                                }
                            }
                            Err(e) => {
                                if subscribe {
                                    let _ = send_message(
                                        &writer,
                                        &DaemonServerMessage::DaemonPushEvent(
                                            PushEvent::PushFailed {
                                                path: path.clone(),
                                                error: format!("{e:#}"),
                                            },
                                        ),
                                    )
                                    .await;
                                }
                            }
                        }
                    }

                    if subscribe {
                        let _ = send_message(
                            &writer,
                            &DaemonServerMessage::DaemonPushEvent(PushEvent::AllComplete),
                        )
                        .await;
                    }
                });
            }
            DaemonClientMessage::ClientDiagnosticsRequest => {
                // Basic health check: try to reach the API
                let healthy = api.get_cache(cache_name).await.is_ok();
                send_message(
                    &writer,
                    &DaemonServerMessage::DaemonDiagnosticsResult(DaemonDiagnostics {
                        is_healthy: healthy,
                        message: if healthy {
                            "daemon is healthy".to_string()
                        } else {
                            "failed to reach cachix API".to_string()
                        },
                    }),
                )
                .await?;
            }
        }
    }

    Ok(())
}

async fn send_message(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    msg: &DaemonServerMessage,
) -> Result<()> {
    let mut data = serde_json::to_string(msg)?;
    data.push('\n');
    let mut writer = writer.lock().await;
    writer.write_all(data.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Send a push request to a running daemon.
pub async fn daemon_push(socket_path: &Path, store_paths: Vec<String>, wait: bool) -> Result<()> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to daemon at {}", socket_path.display()))?;

    let (reader, mut writer) = stream.into_split();

    let msg = DaemonClientMessage::ClientPushRequest(PushRequest {
        store_paths,
        subscribe_to_updates: wait,
    });

    let mut data = serde_json::to_string(&msg)?;
    data.push('\n');
    writer.write_all(data.as_bytes()).await?;
    writer.flush().await?;

    if wait {
        let reader = BufReader::new(reader);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            let msg: DaemonServerMessage = serde_json::from_str(&line)?;
            match msg {
                DaemonServerMessage::DaemonPushEvent(event) => match event {
                    PushEvent::PushStarted { path } => {
                        tracing::info!("pushing {path}");
                    }
                    PushEvent::PushCompleted { path } => {
                        tracing::info!("completed {path}");
                    }
                    PushEvent::PushFailed { path, error } => {
                        tracing::error!("failed {path}: {error}");
                    }
                    PushEvent::AllComplete => {
                        tracing::info!("all pushes complete");
                        break;
                    }
                },
                DaemonServerMessage::DaemonError(err) => {
                    bail!("daemon error: {}", err.message);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Send a stop request to the daemon.
pub async fn daemon_stop(socket_path: &Path) -> Result<()> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to daemon at {}", socket_path.display()))?;

    let (reader, mut writer) = stream.into_split();

    let msg = DaemonClientMessage::ClientStop;
    let mut data = serde_json::to_string(&msg)?;
    data.push('\n');
    writer.write_all(data.as_bytes()).await?;
    writer.flush().await?;

    let reader = BufReader::new(reader);
    let mut lines = reader.lines();

    if let Some(line) = lines.next_line().await? {
        let msg: DaemonServerMessage = serde_json::from_str(&line)?;
        match msg {
            DaemonServerMessage::DaemonExit(info) => {
                tracing::info!("daemon stopped: {}", info.exit_message);
            }
            DaemonServerMessage::DaemonError(err) => {
                bail!("daemon refused to stop: {}", err.message);
            }
            _ => {}
        }
    }

    Ok(())
}

/// Run daemon diagnostics.
pub async fn daemon_doctor(socket_path: &Path) -> Result<()> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to daemon at {}", socket_path.display()))?;

    let (reader, mut writer) = stream.into_split();

    let msg = DaemonClientMessage::ClientDiagnosticsRequest;
    let mut data = serde_json::to_string(&msg)?;
    data.push('\n');
    writer.write_all(data.as_bytes()).await?;
    writer.flush().await?;

    let reader = BufReader::new(reader);
    let mut lines = reader.lines();

    if let Some(line) = lines.next_line().await? {
        let msg: DaemonServerMessage = serde_json::from_str(&line)?;
        if let DaemonServerMessage::DaemonDiagnosticsResult(diag) = msg {
            if diag.is_healthy {
                println!("Daemon is healthy: {}", diag.message);
            } else {
                println!("Daemon is unhealthy: {}", diag.message);
            }
        }
    }

    Ok(())
}
