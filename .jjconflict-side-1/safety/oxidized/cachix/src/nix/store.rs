use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

/// Query the transitive closure of store paths using nix-store.
pub async fn query_closure(store_paths: &[String]) -> Result<Vec<String>> {
    if store_paths.is_empty() {
        return Ok(Vec::new());
    }

    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--requisites")
        .args(store_paths)
        .output()
        .await
        .context("failed to run nix-store --query --requisites")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nix-store --query --requisites failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Query the deriver of a store path.
pub async fn query_deriver(store_path: &str) -> Result<Option<String>> {
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--deriver")
        .arg(store_path)
        .output()
        .await
        .context("failed to run nix-store --query --deriver")?;

    if !output.status.success() {
        return Ok(None);
    }

    let deriver = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if deriver == "unknown-deriver" || deriver.is_empty() {
        Ok(None)
    } else {
        Ok(Some(deriver))
    }
}

/// Query references (direct dependencies) of a store path.
pub async fn query_references(store_path: &str) -> Result<Vec<String>> {
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--references")
        .arg(store_path)
        .output()
        .await
        .context("failed to run nix-store --query --references")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nix-store --query --references failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Dump a NAR from a store path.
pub async fn dump_nar(store_path: &str) -> Result<tokio::process::Child> {
    let child = Command::new("nix-store")
        .arg("--dump")
        .arg(store_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn nix-store --dump")?;
    Ok(child)
}

/// Query path info (hash, size) for a store path using nix path-info.
pub async fn query_path_info(store_path: &str) -> Result<PathInfo> {
    let output = Command::new("nix")
        .arg("path-info")
        .arg("--json")
        .arg(store_path)
        .output()
        .await
        .context("failed to run nix path-info --json")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nix path-info failed: {stderr}");
    }

    // nix path-info --json returns either a map {path: info} (Nix 2.x)
    // or an array [{path, ...}] (older versions). Handle both.
    let entry: PathInfoJson = {
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("failed to parse nix path-info output")?;
        if let Some(obj) = value.as_object() {
            // Map format: {"<store-path>": {narHash, narSize, ...}}
            let (path, info) = obj.into_iter().next().context("empty path-info response")?;
            let mut info = info.clone();
            if info.get("path").is_none() {
                info.as_object_mut()
                    .unwrap()
                    .insert("path".to_string(), serde_json::Value::String(path.clone()));
            }
            serde_json::from_value(info).context("failed to parse path-info entry")?
        } else {
            // Array format: [{path, narHash, narSize, ...}]
            let arr: Vec<PathInfoJson> =
                serde_json::from_value(value).context("failed to parse path-info array")?;
            arr.into_iter().next().context("no path info returned")?
        }
    };

    Ok(PathInfo {
        path: entry.path,
        nar_hash: entry.nar_hash,
        nar_size: entry.nar_size,
        references: entry.references,
        deriver: entry.deriver,
    })
}

#[derive(Debug, Clone)]
pub struct PathInfo {
    pub path: String,
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
    pub deriver: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathInfoJson {
    path: String,
    nar_hash: String,
    nar_size: u64,
    #[serde(default)]
    references: Vec<String>,
    deriver: Option<String>,
}

/// Realise (download/build) a store path.
pub async fn realise(store_path: &str) -> Result<()> {
    let output = Command::new("nix-store")
        .arg("--realise")
        .arg(store_path)
        .output()
        .await
        .context("failed to run nix-store --realise")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nix-store --realise failed: {stderr}");
    }
    Ok(())
}

/// Check if a store path is valid (exists in the store).
pub async fn is_valid_path(store_path: &str) -> Result<bool> {
    let output = Command::new("nix-store")
        .arg("--check-validity")
        .arg(store_path)
        .output()
        .await
        .context("failed to run nix-store --check-validity")?;
    Ok(output.status.success())
}

/// Watch the nix store for new paths. Returns paths as they appear.
pub fn watch_store() -> Result<tokio::sync::mpsc::Receiver<String>> {
    let (tx, rx) = tokio::sync::mpsc::channel(1024);

    tokio::spawn(async move {
        // Use inotifywait to watch /nix/store for new directories
        let mut child = match Command::new("inotifywait")
            .arg("-m")
            .arg("-e")
            .arg("create")
            .arg("--format")
            .arg("%f")
            .arg("/nix/store")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                tracing::error!("failed to spawn inotifywait: {e}");
                return;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let path = format!("/nix/store/{line}");
            if tx.send(path).await.is_err() {
                break;
            }
        }
    });

    Ok(rx)
}
