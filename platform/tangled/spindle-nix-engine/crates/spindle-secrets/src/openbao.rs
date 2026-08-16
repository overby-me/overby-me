//! OpenBao (Vault-compatible) secrets backend via HTTP KV v2 API.
//!
//! Proxies secret operations to an OpenBao server via its HTTP API.
//! Secrets are stored in a KV v2 mount at a configurable path, scoped
//! per-repository using a sanitized path convention.
//!
//! # Secret Path Convention
//!
//! Secrets are stored at `{mount}/data/repos/{sanitized_repo_path}/{key}`,
//! where the repo path is sanitized by replacing `:` and `/` with `_`:
//! - `did:plc:alice/myrepo` → `did_plc_alice_myrepo`
//!
//! # Token Renewal
//!
//! The OpenBao backend optionally supports periodic token renewal via a
//! background task. When the manager is created with a renewable token,
//! a `tokio::spawn`'ed task periodically calls the `/auth/token/renew-self`
//! endpoint. The [`Stopper`] trait is implemented to cancel this task on
//! shutdown.
//!
//! Matches the upstream Go `OpenBaoManager` / `VaultManager` behavior.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use spindle_models::UnlockedSecret;
use tokio::sync::watch;
use tracing::{debug, instrument, warn};

use crate::traits::{Manager, Stopper};
use crate::{SecretsError, sanitize_repo_path};

/// Response wrapper for OpenBao KV v2 read operations.
///
/// The KV v2 API wraps secret data in a `data` envelope:
/// ```json
/// { "data": { "data": { "value": "..." }, "metadata": { ... } } }
/// ```
#[derive(Debug, Deserialize)]
struct KvV2ReadResponse {
    data: Option<KvV2ReadData>,
}

#[derive(Debug, Deserialize)]
struct KvV2ReadData {
    data: Option<serde_json::Value>,
}

/// Response wrapper for OpenBao KV v2 list operations.
///
/// ```json
/// { "data": { "keys": ["key1", "key2"] } }
/// ```
#[derive(Debug, Deserialize)]
struct KvV2ListResponse {
    data: Option<KvV2ListData>,
}

#[derive(Debug, Deserialize)]
struct KvV2ListData {
    keys: Option<Vec<String>>,
}

/// Request body for OpenBao KV v2 write operations.
#[derive(Debug, Serialize)]
struct KvV2WriteRequest {
    data: serde_json::Value,
}

/// OpenBao-backed secrets manager using the KV v2 HTTP API.
///
/// Suitable for multi-node or enterprise deployments where secrets
/// are centrally managed by an OpenBao (or Vault-compatible) server.
#[derive(Clone)]
pub struct OpenBaoManager {
    inner: Arc<OpenBaoManagerInner>,
}

struct OpenBaoManagerInner {
    client: Client,
    /// Base URL of the OpenBao server (e.g. `http://127.0.0.1:8200`).
    addr: String,
    /// KV v2 mount path (e.g. `spindle`).
    mount: String,
    /// OpenBao token for authentication.
    token: String,
    /// Sender half of the stop signal for token renewal.
    /// When dropped or sent `true`, the renewal task exits.
    stop_tx: watch::Sender<bool>,
}

impl OpenBaoManager {
    /// Create a new OpenBao secrets manager.
    ///
    /// Connects to the OpenBao server at `addr` using the given `token`
    /// for authentication. Secrets are stored under the KV v2 `mount` path.
    ///
    /// # Arguments
    /// * `addr` — Base URL of the OpenBao server (e.g. `"http://127.0.0.1:8200"`).
    /// * `mount` — KV v2 mount path (e.g. `"spindle"`).
    /// * `token` — OpenBao authentication token.
    pub fn new(
        addr: impl Into<String>,
        mount: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        let (stop_tx, _stop_rx) = watch::channel(false);

        Self {
            inner: Arc::new(OpenBaoManagerInner {
                client: Client::new(),
                addr: addr.into().trim_end_matches('/').to_owned(),
                mount: mount.into(),
                token: token.into(),
                stop_tx,
            }),
        }
    }

    /// Create a new OpenBao secrets manager with periodic token renewal.
    ///
    /// Spawns a background task that renews the token at the given interval.
    /// Use [`Stopper::stop`] to cancel the renewal task on shutdown.
    ///
    /// # Arguments
    /// * `addr` — Base URL of the OpenBao server.
    /// * `mount` — KV v2 mount path.
    /// * `token` — OpenBao authentication token.
    /// * `renewal_interval` — How often to renew the token.
    pub fn with_token_renewal(
        addr: impl Into<String>,
        mount: impl Into<String>,
        token: impl Into<String>,
        renewal_interval: std::time::Duration,
    ) -> Self {
        let manager = Self::new(addr, mount, token);

        let inner = manager.inner.clone();
        let mut stop_rx = inner.stop_tx.subscribe();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(renewal_interval);
            // Skip the first tick (fires immediately).
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = renew_token(&inner.client, &inner.addr, &inner.token).await {
                            warn!(error = %e, "failed to renew OpenBao token");
                        } else {
                            debug!("renewed OpenBao token");
                        }
                    }
                    _ = stop_rx.changed() => {
                        debug!("stopping OpenBao token renewal task");
                        break;
                    }
                }
            }
        });

        manager
    }

    /// Build the KV v2 data path for a secret.
    ///
    /// Format: `/v1/{mount}/data/repos/{sanitized_repo}/{key}`
    fn data_path(&self, sanitized_repo: &str, key: &str) -> String {
        format!(
            "{}/v1/{}/data/repos/{}/{}",
            self.inner.addr, self.inner.mount, sanitized_repo, key
        )
    }

    /// Build the KV v2 metadata path for listing secrets in a repo.
    ///
    /// Format: `/v1/{mount}/metadata/repos/{sanitized_repo}`
    fn metadata_path(&self, sanitized_repo: &str) -> String {
        format!(
            "{}/v1/{}/metadata/repos/{}",
            self.inner.addr, self.inner.mount, sanitized_repo
        )
    }

    /// Build the KV v2 delete path for a specific secret's metadata.
    ///
    /// For permanent deletion: `/v1/{mount}/metadata/repos/{sanitized_repo}/{key}`
    fn metadata_key_path(&self, sanitized_repo: &str, key: &str) -> String {
        format!(
            "{}/v1/{}/metadata/repos/{}/{}",
            self.inner.addr, self.inner.mount, sanitized_repo, key
        )
    }

    /// Make an authenticated request to the OpenBao API.
    fn auth_header(&self) -> (&str, &str) {
        ("X-Vault-Token", &self.inner.token)
    }
}

/// Renew the current token via the OpenBao API.
async fn renew_token(client: &Client, addr: &str, token: &str) -> Result<(), SecretsError> {
    let url = format!("{addr}/v1/auth/token/renew-self");
    let resp = client
        .post(&url)
        .header("X-Vault-Token", token)
        .send()
        .await
        .map_err(|e| SecretsError::OpenBao(format!("token renewal request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(SecretsError::OpenBao(format!(
            "token renewal failed with status {status}: {body}"
        )));
    }

    Ok(())
}

#[async_trait]
impl Manager for OpenBaoManager {
    #[instrument(skip(self), fields(repo = %repo))]
    async fn get_secrets_unlocked(&self, repo: &str) -> Result<Vec<UnlockedSecret>, SecretsError> {
        let sanitized = sanitize_repo_path(repo);

        // First, list all secret keys for this repo.
        let keys = self.list_secrets(repo).await?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        // Then, read each secret individually.
        let mut secrets = Vec::with_capacity(keys.len());
        for key in &keys {
            let url = self.data_path(&sanitized, key);
            let (header_name, header_value) = self.auth_header();

            let resp = self
                .inner
                .client
                .get(&url)
                .header(header_name, header_value)
                .send()
                .await
                .map_err(|e| SecretsError::OpenBao(format!("read request failed: {e}")))?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                // Secret was deleted between list and read; skip it.
                debug!(key = %key, "secret not found during read, skipping");
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SecretsError::OpenBao(format!(
                    "read secret {key} failed with status {status}: {body}"
                )));
            }

            let body: KvV2ReadResponse = resp.json().await.map_err(|e| {
                SecretsError::OpenBao(format!("failed to parse read response: {e}"))
            })?;

            let value = body
                .data
                .and_then(|d| d.data)
                .and_then(|data| data.get("value").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_default();

            secrets.push(UnlockedSecret::new(key.clone(), value));
        }

        debug!(
            repo = sanitized,
            count = secrets.len(),
            "retrieved secrets from OpenBao"
        );
        Ok(secrets)
    }

    #[instrument(skip(self, value), fields(repo = %repo, key = %key))]
    async fn put_secret(&self, repo: &str, key: &str, value: &str) -> Result<(), SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let url = self.data_path(&sanitized, key);
        let (header_name, header_value) = self.auth_header();

        let body = KvV2WriteRequest {
            data: serde_json::json!({ "value": value }),
        };

        let resp = self
            .inner
            .client
            .post(&url)
            .header(header_name, header_value)
            .json(&body)
            .send()
            .await
            .map_err(|e| SecretsError::OpenBao(format!("write request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::OpenBao(format!(
                "write secret {key} failed with status {status}: {resp_body}"
            )));
        }

        debug!(repo = sanitized, key = key, "stored secret in OpenBao");
        Ok(())
    }

    #[instrument(skip(self), fields(repo = %repo, key = %key))]
    async fn delete_secret(&self, repo: &str, key: &str) -> Result<(), SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let url = self.metadata_key_path(&sanitized, key);
        let (header_name, header_value) = self.auth_header();

        let resp = self
            .inner
            .client
            .delete(&url)
            .header(header_name, header_value)
            .send()
            .await
            .map_err(|e| SecretsError::OpenBao(format!("delete request failed: {e}")))?;

        // 404 is acceptable (idempotent delete).
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::OpenBao(format!(
                "delete secret {key} failed with status {status}: {body}"
            )));
        }

        debug!(repo = sanitized, key = key, "deleted secret from OpenBao");
        Ok(())
    }

    #[instrument(skip(self), fields(repo = %repo))]
    async fn list_secrets(&self, repo: &str) -> Result<Vec<String>, SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let url = self.metadata_path(&sanitized);
        let (header_name, header_value) = self.auth_header();

        // OpenBao LIST is done via the LIST HTTP method or GET with `?list=true`.
        // The reqwest crate doesn't have a `.list()` method, so we use the
        // `LIST` custom method or add `?list=true` to a GET request.
        let resp = self
            .inner
            .client
            .request(reqwest::Method::from_bytes(b"LIST").unwrap(), &url)
            .header(header_name, header_value)
            .send()
            .await
            .map_err(|e| SecretsError::OpenBao(format!("list request failed: {e}")))?;

        // 404 means no secrets exist for this repo (empty list).
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::OpenBao(format!(
                "list secrets failed with status {status}: {body}"
            )));
        }

        let body: KvV2ListResponse = resp
            .json()
            .await
            .map_err(|e| SecretsError::OpenBao(format!("failed to parse list response: {e}")))?;

        let mut keys = body.data.and_then(|d| d.keys).unwrap_or_default();

        // Filter out directory-like entries (trailing `/`), which are sub-paths.
        keys.retain(|k| !k.ends_with('/'));
        keys.sort();

        debug!(
            repo = sanitized,
            count = keys.len(),
            "listed secrets from OpenBao"
        );
        Ok(keys)
    }
}

impl Stopper for OpenBaoManager {
    fn stop(&self) {
        // Signal the token renewal task to exit (if running).
        // Ignore the error if no receivers are listening (task already exited).
        let _ = self.inner.stop_tx.send(true);
    }
}

impl Drop for OpenBaoManagerInner {
    fn drop(&mut self) {
        // Best-effort stop signal on drop.
        let _ = self.stop_tx.send(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_path_construction() {
        let mgr = OpenBaoManager::new("http://127.0.0.1:8200", "spindle", "test-token");
        let path = mgr.data_path("did_plc_alice_myrepo", "API_KEY");
        assert_eq!(
            path,
            "http://127.0.0.1:8200/v1/spindle/data/repos/did_plc_alice_myrepo/API_KEY"
        );
    }

    #[test]
    fn data_path_strips_trailing_slash() {
        let mgr = OpenBaoManager::new("http://127.0.0.1:8200/", "spindle", "test-token");
        let path = mgr.data_path("did_plc_alice_myrepo", "KEY");
        assert_eq!(
            path,
            "http://127.0.0.1:8200/v1/spindle/data/repos/did_plc_alice_myrepo/KEY"
        );
    }

    #[test]
    fn metadata_path_construction() {
        let mgr = OpenBaoManager::new("http://127.0.0.1:8200", "spindle", "test-token");
        let path = mgr.metadata_path("did_plc_alice_myrepo");
        assert_eq!(
            path,
            "http://127.0.0.1:8200/v1/spindle/metadata/repos/did_plc_alice_myrepo"
        );
    }

    #[test]
    fn metadata_key_path_construction() {
        let mgr = OpenBaoManager::new("http://127.0.0.1:8200", "spindle", "test-token");
        let path = mgr.metadata_key_path("did_plc_alice_myrepo", "API_KEY");
        assert_eq!(
            path,
            "http://127.0.0.1:8200/v1/spindle/metadata/repos/did_plc_alice_myrepo/API_KEY"
        );
    }

    #[test]
    fn stopper_is_idempotent() {
        let mgr = OpenBaoManager::new("http://127.0.0.1:8200", "spindle", "test-token");
        mgr.stop();
        mgr.stop(); // Should not panic.
    }

    #[test]
    fn deserialize_kv_v2_read_response() {
        let json = r#"{
            "data": {
                "data": { "value": "my-secret-value" },
                "metadata": { "version": 1 }
            }
        }"#;
        let resp: KvV2ReadResponse = serde_json::from_str(json).unwrap();
        let data = resp.data.unwrap().data.unwrap();
        let value = data.get("value").unwrap().as_str().unwrap();
        assert_eq!(value, "my-secret-value");
    }

    #[test]
    fn deserialize_kv_v2_read_response_missing_data() {
        let json = r#"{ "data": null }"#;
        let resp: KvV2ReadResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.is_none());
    }

    #[test]
    fn deserialize_kv_v2_list_response() {
        let json = r#"{
            "data": {
                "keys": ["API_KEY", "DB_PASS", "subdir/"]
            }
        }"#;
        let resp: KvV2ListResponse = serde_json::from_str(json).unwrap();
        let mut keys = resp.data.unwrap().keys.unwrap();
        keys.retain(|k| !k.ends_with('/'));
        assert_eq!(keys, vec!["API_KEY", "DB_PASS"]);
    }

    #[test]
    fn deserialize_kv_v2_list_response_empty() {
        let json = r#"{ "data": { "keys": [] } }"#;
        let resp: KvV2ListResponse = serde_json::from_str(json).unwrap();
        let keys = resp.data.unwrap().keys.unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn serialize_kv_v2_write_request() {
        let req = KvV2WriteRequest {
            data: serde_json::json!({ "value": "secret-value" }),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["data"]["value"], "secret-value");
    }

    // NOTE: Full integration tests against a real (or mock) OpenBao server
    // are in Phase 8. The unit tests here verify serialization, path
    // construction, and basic behavior without network access.
}
