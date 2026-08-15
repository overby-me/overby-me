use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dirs::config_dir;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RootConfig {
    #[serde(default)]
    pub default: DefaultSection,
    #[serde(default)]
    pub auth: AuthSection,
    #[serde(default)]
    pub knots: KnotsSection,
    #[serde(default)]
    pub ui: UiSection,
    #[serde(default)]
    pub profiles: ProfilesSection,
}

/// Which accounts this CLI knows about, and which one is in use.
///
/// The sessions themselves live in the keyring, one entry per profile, and a
/// keyring cannot be enumerated: without this list `auth list` would have
/// nothing to list.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfilesSection {
    /// Profile used when --profile is not given.
    pub active: Option<String>,
    /// Every profile that has been logged into.
    #[serde(default)]
    pub known: Vec<String>,
}

impl ProfilesSection {
    pub fn remember(&mut self, name: &str) {
        if !self.known.iter().any(|k| k == name) {
            self.known.push(name.to_string());
        }
        self.active = Some(name.to_string());
    }

    pub fn forget(&mut self, name: &str) {
        self.known.retain(|k| k != name);
        if self.active.as_deref() == Some(name) {
            self.active = self.known.first().cloned();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DefaultSection {
    pub knot: Option<String>,
    pub editor: Option<String>,
    pub pager: Option<String>,
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "table".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthSection {
    pub handle: Option<String>,
    pub did: Option<String>,
    pub pds_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnotsSection {
    pub default: Option<String>,
    #[serde(default)]
    pub custom: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiSection {
    #[serde(default)]
    pub color: bool,
    #[serde(default)]
    pub progress_bar: bool,
    #[serde(default)]
    pub confirm_destructive: bool,
}

pub fn default_config_path() -> Result<PathBuf> {
    let base = config_dir().context("Could not determine platform config directory")?;
    Ok(base.join("tangled").join("config.toml"))
}

pub fn load_config(path: Option<&Path>) -> Result<Option<RootConfig>> {
    let path = path
        .map(|p| p.to_path_buf())
        .unwrap_or(default_config_path()?);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed reading config file: {}", path.display()))?;
    let cfg: RootConfig = toml::from_str(&content).context("Invalid TOML in config")?;
    Ok(Some(cfg))
}

pub fn save_config(cfg: &RootConfig, path: Option<&Path>) -> Result<()> {
    let path = path
        .map(|p| p.to_path_buf())
        .unwrap_or(default_config_path()?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml = toml::to_string_pretty(cfg)?;
    fs::write(&path, toml)
        .with_context(|| format!("Failed writing config file: {}", path.display()))?;
    Ok(())
}
