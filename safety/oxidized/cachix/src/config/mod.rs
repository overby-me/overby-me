use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Cachix configuration file (JSON format, replacing Dhall).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default = "default_hostname")]
    pub hostname: String,
    #[serde(default)]
    pub binary_caches: Vec<BinaryCacheConfig>,
}

fn default_hostname() -> String {
    "https://cachix.org".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryCacheConfig {
    pub name: String,
    pub secret_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auth_token: None,
            hostname: default_hostname(),
            binary_caches: Vec::new(),
        }
    }
}

impl Config {
    /// Get the default config file path.
    pub fn default_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir().context("could not determine config directory")?;
        Ok(config_dir.join("cachix").join("cachix.json"))
    }

    /// Load config from the given path, or create a default if it doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config from {}", path.display()))
    }

    /// Save config to the given path.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }
        let contents = serde_json::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(path, contents)
            .with_context(|| format!("failed to write config to {}", path.display()))?;
        Ok(())
    }

    /// Set the auth token.
    pub fn set_auth_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    /// Get the secret key for a given cache name.
    pub fn get_secret_key(&self, cache_name: &str) -> Option<&str> {
        self.binary_caches
            .iter()
            .find(|c| c.name == cache_name)
            .map(|c| c.secret_key.as_str())
    }

    /// Set or update the secret key for a given cache name.
    pub fn set_secret_key(&mut self, cache_name: &str, secret_key: &str) {
        if let Some(cache) = self.binary_caches.iter_mut().find(|c| c.name == cache_name) {
            cache.secret_key = secret_key.to_string();
        } else {
            self.binary_caches.push(BinaryCacheConfig {
                name: cache_name.to_string(),
                secret_key: secret_key.to_string(),
            });
        }
    }
}

/// Resolve push credentials from environment and config.
#[derive(Debug, Clone)]
pub enum PushCredential {
    /// Auth token + signing key (signs NARs locally).
    SigningKey {
        auth_token: Option<String>,
        signing_key: String,
    },
    /// Auth token only (server-side signing).
    Token(String),
}

impl PushCredential {
    pub fn auth_token(&self) -> Option<&str> {
        match self {
            PushCredential::SigningKey { auth_token, .. } => auth_token.as_deref(),
            PushCredential::Token(t) => Some(t),
        }
    }

    pub fn signing_key(&self) -> Option<&str> {
        match self {
            PushCredential::SigningKey { signing_key, .. } => Some(signing_key),
            PushCredential::Token(_) => None,
        }
    }
}

/// Resolve push credentials from env vars and config.
pub fn resolve_push_credential(config: &Config, cache_name: &str) -> Result<PushCredential> {
    // Check env var first
    let env_signing_key = std::env::var("CACHIX_SIGNING_KEY").ok();
    let env_auth_token = std::env::var("CACHIX_AUTH_TOKEN").ok();

    let signing_key =
        env_signing_key.or_else(|| config.get_secret_key(cache_name).map(String::from));
    let auth_token = env_auth_token.or_else(|| config.auth_token.clone());

    if let Some(sk) = signing_key {
        Ok(PushCredential::SigningKey {
            auth_token,
            signing_key: sk,
        })
    } else if let Some(token) = auth_token {
        Ok(PushCredential::Token(token))
    } else {
        bail!(
            "No credentials found for cache '{cache_name}'. \
             Set CACHIX_AUTH_TOKEN or CACHIX_SIGNING_KEY, \
             or run 'cachix authtoken' to configure."
        )
    }
}
