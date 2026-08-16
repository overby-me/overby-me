use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Represents a parsed nix.conf file.
#[derive(Debug, Clone)]
pub struct NixConf {
    pub path: PathBuf,
    lines: Vec<String>,
}

impl NixConf {
    /// Read and parse a nix.conf file.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = if path.exists() {
            std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?
        } else {
            String::new()
        };

        Ok(Self {
            path: path.to_path_buf(),
            lines: contents.lines().map(String::from).collect(),
        })
    }

    /// Write the nix.conf file.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = self.lines.join("\n") + "\n";
        std::fs::write(&self.path, contents)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }

    /// Get the value of a setting.
    pub fn get(&self, key: &str) -> Option<String> {
        for line in self.lines.iter().rev() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim_start();
                if let Some(value) = rest.strip_prefix('=') {
                    return Some(value.trim().to_string());
                }
            }
        }
        None
    }

    /// Add a value to a space-separated list setting.
    pub fn add_to_list(&mut self, key: &str, value: &str) {
        let existing = self.get(key).unwrap_or_default();
        let values: Vec<&str> = existing.split_whitespace().collect();
        if values.contains(&value) {
            return;
        }
        let new_value = if existing.is_empty() {
            value.to_string()
        } else {
            format!("{existing} {value}")
        };
        self.set(key, &new_value);
    }

    /// Remove a value from a space-separated list setting.
    pub fn remove_from_list(&mut self, key: &str, value: &str) {
        if let Some(existing) = self.get(key) {
            let values: Vec<&str> = existing
                .split_whitespace()
                .filter(|v| *v != value)
                .collect();
            if values.is_empty() {
                self.remove_key(key);
            } else {
                self.set(key, &values.join(" "));
            }
        }
    }

    /// Set a key to a value. Updates existing line or appends.
    pub fn set(&mut self, key: &str, value: &str) {
        for line in &mut self.lines {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(key)
                && rest.trim_start().starts_with('=')
            {
                *line = format!("{key} = {value}");
                return;
            }
        }
        self.lines.push(format!("{key} = {value}"));
    }

    /// Remove a key entirely.
    pub fn remove_key(&mut self, key: &str) {
        self.lines.retain(|line| {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(key) {
                !rest.trim_start().starts_with('=')
            } else {
                true
            }
        });
    }

    /// Check if an include directive exists for a file.
    pub fn has_include(&self, include_path: &str) -> bool {
        self.lines.iter().any(|line| {
            let trimmed = line.trim();
            trimmed == format!("include {include_path}")
                || trimmed == format!("!include {include_path}")
        })
    }

    /// Add an include directive.
    pub fn add_include(&mut self, include_path: &str, optional: bool) {
        if self.has_include(include_path) {
            return;
        }
        let prefix = if optional { "!include" } else { "include" };
        self.lines.push(format!("{prefix} {include_path}"));
    }
}

/// Installation mode for `cachix use`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMode {
    NixOS,
    RootNixConf,
    UserNixConf,
}

/// Configure a binary cache in nix.conf.
pub fn use_cache(
    cache_name: &str,
    cache_uri: &str,
    public_keys: &[String],
    mode: InstallMode,
    nixos_folder: &Path,
    output_dir: Option<&Path>,
) -> Result<()> {
    match mode {
        InstallMode::NixOS => {
            use_cache_nixos(cache_name, nixos_folder)?;
        }
        InstallMode::RootNixConf | InstallMode::UserNixConf => {
            let conf_path = match mode {
                InstallMode::RootNixConf => PathBuf::from("/etc/nix/nix.conf"),
                InstallMode::UserNixConf => {
                    let config_dir =
                        dirs::config_dir().context("could not determine config directory")?;
                    config_dir.join("nix").join("nix.conf")
                }
                _ => unreachable!(),
            };

            let conf_path = if let Some(dir) = output_dir {
                dir.join("nix.conf")
            } else {
                conf_path
            };

            let mut conf = NixConf::load(&conf_path)?;
            conf.add_to_list("substituters", cache_uri);
            for key in public_keys {
                conf.add_to_list("trusted-public-keys", key);
            }
            conf.save()?;

            tracing::info!("configured {cache_name} in {}", conf_path.display());
        }
    }
    Ok(())
}

/// Generate a NixOS module that configures the cache.
fn use_cache_nixos(cache_name: &str, nixos_folder: &Path) -> Result<()> {
    let filename = format!("cachix/{cache_name}.nix");
    let filepath = nixos_folder.join(&filename);

    if let Some(parent) = filepath.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = format!(
        r#"# WARNING: this file is generated by cachix. Do not edit.
{{
  nix.settings = {{
    substituters = [
      "https://{cache_name}.cachix.org"
    ];
    trusted-public-keys = [
      "{cache_name}.cachix.org-1:<KEY>"
    ];
  }};
}}
"#
    );

    std::fs::write(&filepath, content)?;
    tracing::info!("created NixOS configuration at {}", filepath.display());
    tracing::info!(
        "don't forget to add ./cachix/{cache_name}.nix to your imports in configuration.nix"
    );

    Ok(())
}

/// Remove a binary cache from nix.conf.
pub fn remove_cache(
    cache_name: &str,
    cache_uri: &str,
    public_keys: &[String],
    mode: InstallMode,
    nixos_folder: &Path,
) -> Result<()> {
    match mode {
        InstallMode::NixOS => {
            let filepath = nixos_folder.join(format!("cachix/{cache_name}.nix"));
            if filepath.exists() {
                std::fs::remove_file(&filepath)?;
                tracing::info!("removed {}", filepath.display());
            }
        }
        InstallMode::RootNixConf | InstallMode::UserNixConf => {
            let conf_path = match mode {
                InstallMode::RootNixConf => PathBuf::from("/etc/nix/nix.conf"),
                InstallMode::UserNixConf => {
                    let config_dir =
                        dirs::config_dir().context("could not determine config directory")?;
                    config_dir.join("nix").join("nix.conf")
                }
                _ => unreachable!(),
            };

            let mut conf = NixConf::load(&conf_path)?;
            conf.remove_from_list("substituters", cache_uri);
            for key in public_keys {
                conf.remove_from_list("trusted-public-keys", key);
            }
            conf.save()?;

            tracing::info!("removed {cache_name} from {}", conf_path.display());
        }
    }
    Ok(())
}

/// Write netrc entry for private cache access.
pub fn write_netrc(hostname: &str, auth_token: &str, output_dir: Option<&Path>) -> Result<PathBuf> {
    let netrc_path = if let Some(dir) = output_dir {
        dir.join("netrc")
    } else {
        let config_dir = dirs::config_dir().context("could not determine config directory")?;
        config_dir.join("nix").join("netrc")
    };

    if let Some(parent) = netrc_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Parse hostname to get just the domain
    let domain = hostname
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let entry = format!("machine {domain} login authtoken password {auth_token}\n");

    // Read existing, update or append
    let mut content = if netrc_path.exists() {
        std::fs::read_to_string(&netrc_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Remove existing entry for this machine
    let lines: Vec<&str> = content.lines().collect();
    let filtered: Vec<&str> = lines
        .into_iter()
        .filter(|l| !l.contains(&format!("machine {domain}")))
        .collect();
    content = filtered.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&entry);

    std::fs::write(&netrc_path, &content)?;

    // Set permissions to 600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&netrc_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(netrc_path)
}
