use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::http::Request;

use crate::api::client::ApiClient;
use crate::api::types::*;

/// Run the deploy agent, connecting via WebSocket and handling deployments.
pub async fn run_agent(
    api: &ApiClient,
    agent_name: &str,
    agent_token: &str,
    profile: Option<&str>,
    bootstrap: bool,
) -> Result<()> {
    let ws_url = format!(
        "{}/ws",
        api.base_url()
            .replace("https://", "wss://")
            .replace("http://", "ws://")
    );

    tracing::info!("connecting to {ws_url} as agent '{agent_name}'");

    let request = Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {agent_token}"))
        .header("X-Agent-Name", agent_name)
        .header("X-Agent-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Agent-System", std::env::consts::OS)
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Host",
            api.base_url()
                .trim_start_matches("https://")
                .trim_start_matches("http://"),
        )
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .body(())
        .context("failed to build WebSocket request")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("WebSocket connection failed")?;

    let (mut write, mut read) = ws_stream.split();

    tracing::info!("connected to WebSocket");

    while let Some(msg) = read.next().await {
        let msg = msg.context("WebSocket read error")?;

        match msg {
            Message::Text(text) => {
                let backend_msg: BackendMessage = serde_json::from_str(&text)
                    .with_context(|| format!("failed to parse backend message: {text}"))?;

                match backend_msg {
                    BackendMessage::AgentRegistered(info) => {
                        tracing::info!("agent registered with id {}", info.id);
                        if let Some(cache) = &info.cache {
                            tracing::info!("cache: {}", cache.name);
                        }
                    }
                    BackendMessage::Deployment(details) => {
                        tracing::info!(
                            "received deployment {} for store path {}",
                            details.id,
                            details.store_path
                        );

                        // Send deployment started
                        let started = AgentMessage::DeploymentStarted {
                            id: details.id,
                            time: Utc::now(),
                            closure_size: None,
                        };
                        write
                            .send(Message::Text(serde_json::to_string(&started)?.into()))
                            .await?;

                        // Execute deployment
                        let success = execute_deployment(&details, profile).await;

                        let finished = AgentMessage::DeploymentFinished {
                            id: details.id,
                            time: Utc::now(),
                            has_succeeded: success,
                        };
                        write
                            .send(Message::Text(serde_json::to_string(&finished)?.into()))
                            .await?;

                        if bootstrap {
                            tracing::info!("bootstrap mode: exiting after first deployment");
                            break;
                        }
                    }
                }
            }
            Message::Ping(data) => {
                write.send(Message::Pong(data)).await?;
            }
            Message::Close(_) => {
                tracing::info!("WebSocket closed by server");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Execute a deployment: download the store path and activate it.
async fn execute_deployment(details: &DeploymentDetails, profile: Option<&str>) -> bool {
    // Realise the store path
    tracing::info!("realising {}", details.store_path);
    if let Err(e) = crate::nix::store::realise(&details.store_path).await {
        tracing::error!("failed to realise store path: {e:#}");
        return false;
    }

    // Check for .cachix-deployment script
    let deployment_script = format!("{}/.cachix-deployment", details.store_path);
    if Path::new(&deployment_script).exists() {
        tracing::info!("running deployment script: {deployment_script}");
        let payload = serde_json::json!({
            "storePath": details.store_path,
            "deploymentId": details.id.to_string(),
            "index": details.index,
            "rollbackScript": details.rollback_script,
            "profile": profile,
        });

        let result = tokio::process::Command::new(&deployment_script)
            .env("CACHIX_DEPLOYMENT_PAYLOAD", payload.to_string())
            .status()
            .await;

        match result {
            Ok(status) if status.success() => {
                tracing::info!("deployment script succeeded");
                true
            }
            Ok(status) => {
                tracing::error!("deployment script failed with status {status}");
                false
            }
            Err(e) => {
                tracing::error!("failed to run deployment script: {e}");
                false
            }
        }
    } else {
        // Default activation: try NixOS switch-to-configuration, then nix-env --set
        let profile_path = profile.unwrap_or("/nix/var/nix/profiles/system");
        activate_profile(&details.store_path, profile_path).await
    }
}

/// Activate a profile by setting it and running switch-to-configuration if available.
async fn activate_profile(store_path: &str, profile_path: &str) -> bool {
    // Set profile
    let result = tokio::process::Command::new("nix-env")
        .arg("--profile")
        .arg(profile_path)
        .arg("--set")
        .arg(store_path)
        .status()
        .await;

    match result {
        Ok(status) if !status.success() => {
            tracing::error!("nix-env --set failed");
            return false;
        }
        Err(e) => {
            tracing::error!("failed to run nix-env: {e}");
            return false;
        }
        _ => {}
    }

    // Try switch-to-configuration
    let switch_script = format!("{store_path}/bin/switch-to-configuration");
    if Path::new(&switch_script).exists() {
        let result = tokio::process::Command::new(&switch_script)
            .arg("switch")
            .status()
            .await;

        match result {
            Ok(status) if status.success() => {
                tracing::info!("switch-to-configuration succeeded");
            }
            Ok(status) => {
                tracing::error!("switch-to-configuration failed with {status}");
                return false;
            }
            Err(e) => {
                tracing::error!("failed to run switch-to-configuration: {e}");
                return false;
            }
        }
    }

    true
}

/// Activate a deployment by sending the spec to the API.
pub async fn activate(
    api: &ApiClient,
    spec_path: &Path,
    agents: &[String],
    async_mode: bool,
) -> Result<()> {
    let spec_content = std::fs::read_to_string(spec_path)
        .with_context(|| format!("failed to read deploy spec from {}", spec_path.display()))?;

    let mut spec: DeploySpec =
        serde_json::from_str(&spec_content).context("failed to parse deploy spec")?;

    // Filter to specific agents if requested
    if !agents.is_empty() {
        let agent_set: std::collections::HashSet<&str> =
            agents.iter().map(|s| s.as_str()).collect();
        spec.agents
            .retain(|name, _| agent_set.contains(name.as_str()));
        if spec.agents.is_empty() {
            bail!("no matching agents found in deploy spec");
        }
    }

    tracing::info!("activating deployment for {} agents", spec.agents.len());

    let response = api.deploy_activate(&spec).await?;

    for agent in &response.agents {
        tracing::info!(
            "agent '{}': deployment {} queued",
            agent.name,
            agent.deployment_id
        );
    }

    if async_mode {
        println!("deployment activated (async mode, not waiting for completion)");
        return Ok(());
    }

    // Poll for deployment status
    for agent in &response.agents {
        tracing::info!(
            "waiting for deployment {} (agent: {})",
            agent.deployment_id,
            agent.name
        );
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let deployment = api.get_deployment(&agent.deployment_id).await?;
            match deployment.status {
                DeploymentStatus::Succeeded => {
                    tracing::info!("deployment {} succeeded", agent.deployment_id);
                    break;
                }
                DeploymentStatus::Failed => {
                    bail!(
                        "deployment {} for agent '{}' failed",
                        agent.deployment_id,
                        agent.name
                    );
                }
                DeploymentStatus::Cancelled => {
                    bail!(
                        "deployment {} for agent '{}' was cancelled",
                        agent.deployment_id,
                        agent.name
                    );
                }
                _ => {
                    // Still in progress
                }
            }
        }
    }

    println!("all deployments completed successfully");
    Ok(())
}
