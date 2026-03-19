//! Configuration loading for `tangled-spindle-nix`.
//!
//! Parses the same `SPINDLE_SERVER_*` and `SPINDLE_ENGINE_*` environment variables
//! as the upstream Go spindle, plus the new `SPINDLE_ENGINE` variable for engine
//! selection. Also supports `SPINDLE_NIXERY_PIPELINES_*` for compatibility.
//!
//! # Environment Variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `SPINDLE_SERVER_HOSTNAME` | *required* | Public hostname of this spindle instance |
//! | `SPINDLE_SERVER_OWNER` | *required* | DID of the spindle owner |
//! | `SPINDLE_SERVER_TOKEN` | *required* | Authentication token (or path via `SPINDLE_SERVER_TOKEN_FILE`) |
//! | `SPINDLE_SERVER_TOKEN_FILE` | — | Path to file containing the auth token |
//! | `SPINDLE_SERVER_LISTEN_ADDR` | `127.0.0.1:6555` | Address the HTTP server binds to |
//! | `SPINDLE_SERVER_JETSTREAM_ENDPOINT` | `wss://jetstream1.us-west.bsky.network/subscribe` | Jetstream WebSocket URL |
//! | `SPINDLE_SERVER_PLC_URL` | `https://plc.directory` | PLC directory URL for DID resolution |
//! | `SPINDLE_SERVER_DB_PATH` | `spindle.db` | Path to the SQLite database file |
//! | `SPINDLE_SERVER_LOG_DIR` | `logs` | Directory for workflow log files |
//! | `SPINDLE_SERVER_DEV` | `false` | Enable dev mode (HTTP instead of HTTPS, etc.) |
//! | `SPINDLE_ENGINE` | `nix` | Engine to use (`nix` or `nixery` for compat) |
//! | `SPINDLE_ENGINE_MAX_JOBS` | `2` | Max concurrent workflow executions |
//! | `SPINDLE_ENGINE_QUEUE_SIZE` | `100` | Max pending jobs in queue |
//! | `SPINDLE_ENGINE_WORKFLOW_TIMEOUT` | `5m` | Default workflow timeout |
//! | `SPINDLE_ENGINE_EXTRA_NIX_FLAGS` | — | Comma-separated extra flags for `nix build` |
//! | `SPINDLE_NIXERY_PIPELINES_URL` | `nixery.tangled.sh` | Nixery URL (compat / fallback) |
//! | `SPINDLE_SERVER_SECRETS_PROVIDER` | `sqlite` | Secrets backend (`sqlite` or `openbao`) |
//! | `SPINDLE_SERVER_SECRETS_OPENBAO_PROXY_ADDR` | `http://127.0.0.1:8200` | OpenBao proxy address |
//! | `SPINDLE_SERVER_SECRETS_OPENBAO_MOUNT` | `spindle` | OpenBao KV v2 mount path |

use std::env;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Errors that can occur during configuration loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required environment variable is missing.
    #[error("missing required environment variable: {0}")]
    MissingEnvVar(String),

    /// An environment variable has an invalid value.
    #[error("invalid value for {key}: {message}")]
    InvalidValue { key: String, message: String },

    /// Failed to read a file (e.g. token file).
    #[error("failed to read {path}: {source}")]
    FileRead {
        path: String,
        source: std::io::Error,
    },
}

/// The secrets backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretsProvider {
    /// Encrypted secrets stored in SQLite.
    Sqlite,
    /// Proxy to an OpenBao (Vault-compatible) server.
    OpenBao,
}

impl SecretsProvider {
    /// Parse from a string value.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "sqlite" => Ok(SecretsProvider::Sqlite),
            "openbao" | "vault" => Ok(SecretsProvider::OpenBao),
            other => Err(format!(
                "unknown secrets provider: {other:?} (expected \"sqlite\" or \"openbao\")"
            )),
        }
    }
}

impl fmt::Display for SecretsProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretsProvider::Sqlite => f.write_str("sqlite"),
            SecretsProvider::OpenBao => f.write_str("openbao"),
        }
    }
}

/// The execution engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// Native Nix engine: `nix build` + child process execution.
    Nix,
    /// Legacy Nixery engine (for compatibility reference; not implemented in this project).
    Nixery,
}

impl EngineKind {
    /// Parse from a string value.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "nix" => Ok(EngineKind::Nix),
            "nixery" => Ok(EngineKind::Nixery),
            other => Err(format!(
                "unknown engine: {other:?} (expected \"nix\" or \"nixery\")"
            )),
        }
    }
}

impl fmt::Display for EngineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineKind::Nix => f.write_str("nix"),
            EngineKind::Nixery => f.write_str("nixery"),
        }
    }
}

/// OpenBao-specific secrets configuration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OpenBaoConfig {
    /// Address of the OpenBao proxy (e.g. `http://127.0.0.1:8200`).
    pub proxy_addr: String,
    /// KV v2 mount path (e.g. `spindle`).
    pub mount: String,
}

/// Secrets manager configuration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SecretsConfig {
    /// Which secrets backend to use.
    pub provider: SecretsProvider,
    /// OpenBao configuration (only relevant when `provider == OpenBao`).
    pub openbao: OpenBaoConfig,
}

/// Per-workflow resource limits applied via systemd scopes.
#[derive(Debug, Clone, Default)]
pub struct WorkflowLimits {
    /// Hard memory limit per workflow (e.g. `"4G"`).
    pub memory_max: Option<String>,
    /// Maximum tasks (processes/threads) per workflow.
    pub tasks_max: Option<u32>,
}

/// Engine configuration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Which engine to use.
    pub kind: EngineKind,
    /// Maximum number of concurrent workflow executions.
    pub max_jobs: usize,
    /// Maximum number of pending jobs in the queue.
    pub queue_size: usize,
    /// Default workflow timeout.
    pub workflow_timeout: Duration,
    /// Nixery URL (for compatibility / fallback).
    pub nixery_url: String,
    /// Extra flags to pass to `nix build`.
    pub extra_nix_flags: Vec<String>,
    /// Per-workflow resource limits.
    pub workflow_limits: WorkflowLimits,
}

/// Full server configuration.
///
/// Loaded from environment variables. The `did_web` field is derived from
/// `hostname` (as `did:web:{hostname}`), matching the upstream Go spindle.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Config {
    /// Public hostname of this spindle instance (e.g. `spindle1.example.com`).
    pub hostname: String,
    /// The `did:web:{hostname}` identity of this spindle.
    pub did_web: String,
    /// DID of the spindle owner (e.g. `did:plc:abc123`).
    pub owner: String,
    /// Authentication token.
    pub token: String,
    /// Address the HTTP server binds to (e.g. `127.0.0.1:6555`).
    pub listen_addr: String,
    /// Jetstream WebSocket endpoint URL.
    pub jetstream_endpoint: String,
    /// PLC directory URL for DID resolution.
    pub plc_url: String,
    /// Path to the SQLite database file.
    pub db_path: PathBuf,
    /// Directory for workflow log files.
    pub log_dir: PathBuf,
    /// Whether dev mode is enabled.
    pub dev: bool,
    /// Engine configuration.
    pub engine: EngineConfig,
    /// Secrets configuration.
    pub secrets: SecretsConfig,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// # Required Variables
    ///
    /// - `SPINDLE_SERVER_HOSTNAME`
    /// - `SPINDLE_SERVER_OWNER`
    /// - `SPINDLE_SERVER_TOKEN` or `SPINDLE_SERVER_TOKEN_FILE`
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if required variables are missing or values are invalid.
    pub fn from_env() -> Result<Self, ConfigError> {
        let hostname = require_env("SPINDLE_SERVER_HOSTNAME")?;
        let did_web = format!("did:web:{hostname}");
        let owner = require_env("SPINDLE_SERVER_OWNER")?;
        let token = load_token()?;

        let listen_addr = env_or_default("SPINDLE_SERVER_LISTEN_ADDR", "127.0.0.1:6555");
        let jetstream_endpoint = env_or_default(
            "SPINDLE_SERVER_JETSTREAM_ENDPOINT",
            "wss://jetstream1.us-west.bsky.network/subscribe",
        );
        let plc_url = env_or_default("SPINDLE_SERVER_PLC_URL", "https://plc.directory");
        let db_path = PathBuf::from(env_or_default("SPINDLE_SERVER_DB_PATH", "spindle.db"));
        let log_dir = PathBuf::from(env_or_default("SPINDLE_SERVER_LOG_DIR", "logs"));
        let dev = parse_bool_env("SPINDLE_SERVER_DEV", false)?;

        let engine = load_engine_config()?;
        let secrets = load_secrets_config()?;

        Ok(Config {
            hostname,
            did_web,
            owner,
            token,
            listen_addr,
            jetstream_endpoint,
            plc_url,
            db_path,
            log_dir,
            dev,
            engine,
            secrets,
        })
    }

    /// Convert this configuration into a map of environment variables.
    ///
    /// Useful for passing configuration to child processes or for debugging.
    #[allow(dead_code)]
    pub fn to_env_map(&self) -> Vec<(String, String)> {
        let mut env = vec![
            ("SPINDLE_SERVER_HOSTNAME".into(), self.hostname.clone()),
            ("SPINDLE_SERVER_OWNER".into(), self.owner.clone()),
            (
                "SPINDLE_SERVER_LISTEN_ADDR".into(),
                self.listen_addr.clone(),
            ),
            (
                "SPINDLE_SERVER_JETSTREAM_ENDPOINT".into(),
                self.jetstream_endpoint.clone(),
            ),
            ("SPINDLE_SERVER_PLC_URL".into(), self.plc_url.clone()),
            (
                "SPINDLE_SERVER_DB_PATH".into(),
                self.db_path.to_string_lossy().into_owned(),
            ),
            (
                "SPINDLE_SERVER_LOG_DIR".into(),
                self.log_dir.to_string_lossy().into_owned(),
            ),
            ("SPINDLE_SERVER_DEV".into(), self.dev.to_string()),
            ("SPINDLE_ENGINE".into(), self.engine.kind.to_string()),
            (
                "SPINDLE_ENGINE_MAX_JOBS".into(),
                self.engine.max_jobs.to_string(),
            ),
            (
                "SPINDLE_ENGINE_QUEUE_SIZE".into(),
                self.engine.queue_size.to_string(),
            ),
            (
                "SPINDLE_ENGINE_WORKFLOW_TIMEOUT".into(),
                format_duration(self.engine.workflow_timeout),
            ),
            (
                "SPINDLE_NIXERY_PIPELINES_URL".into(),
                self.engine.nixery_url.clone(),
            ),
            (
                "SPINDLE_SERVER_SECRETS_PROVIDER".into(),
                self.secrets.provider.to_string(),
            ),
            (
                "SPINDLE_SERVER_SECRETS_OPENBAO_PROXY_ADDR".into(),
                self.secrets.openbao.proxy_addr.clone(),
            ),
            (
                "SPINDLE_SERVER_SECRETS_OPENBAO_MOUNT".into(),
                self.secrets.openbao.mount.clone(),
            ),
        ];

        if !self.engine.extra_nix_flags.is_empty() {
            env.push((
                "SPINDLE_ENGINE_EXTRA_NIX_FLAGS".into(),
                self.engine.extra_nix_flags.join(","),
            ));
        }

        env
    }
}

/// Load the authentication token from `SPINDLE_SERVER_TOKEN` or `SPINDLE_SERVER_TOKEN_FILE`.
fn load_token() -> Result<String, ConfigError> {
    // Try direct token first
    if let Ok(token) = env::var("SPINDLE_SERVER_TOKEN")
        && !token.is_empty()
    {
        return Ok(token);
    }

    // Fall back to token file
    if let Ok(path) = env::var("SPINDLE_SERVER_TOKEN_FILE")
        && !path.is_empty()
    {
        let contents = fs::read_to_string(&path).map_err(|e| ConfigError::FileRead {
            path: path.clone(),
            source: e,
        })?;
        let token = contents.trim().to_owned();
        if token.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "SPINDLE_SERVER_TOKEN_FILE".into(),
                message: format!("token file {path:?} is empty"),
            });
        }
        return Ok(token);
    }

    Err(ConfigError::MissingEnvVar(
        "SPINDLE_SERVER_TOKEN or SPINDLE_SERVER_TOKEN_FILE".into(),
    ))
}

/// Load engine configuration from environment variables.
fn load_engine_config() -> Result<EngineConfig, ConfigError> {
    let kind_str = env_or_default("SPINDLE_ENGINE", "nix");
    let kind = EngineKind::parse(&kind_str).map_err(|message| ConfigError::InvalidValue {
        key: "SPINDLE_ENGINE".into(),
        message,
    })?;

    let max_jobs = parse_usize_env("SPINDLE_ENGINE_MAX_JOBS", 2)?;
    let queue_size = parse_usize_env("SPINDLE_ENGINE_QUEUE_SIZE", 100)?;

    let timeout_str = env_or_default("SPINDLE_ENGINE_WORKFLOW_TIMEOUT", "5m");
    let workflow_timeout =
        parse_duration(&timeout_str).map_err(|message| ConfigError::InvalidValue {
            key: "SPINDLE_ENGINE_WORKFLOW_TIMEOUT".into(),
            message,
        })?;

    let nixery_url = env_or_default("SPINDLE_NIXERY_PIPELINES_URL", "nixery.tangled.sh");

    let extra_nix_flags = match env::var("SPINDLE_ENGINE_EXTRA_NIX_FLAGS") {
        Ok(val) if !val.is_empty() => val.split(',').map(|s| s.trim().to_owned()).collect(),
        _ => Vec::new(),
    };

    let workflow_limits = WorkflowLimits {
        memory_max: env::var("SPINDLE_ENGINE_WORKFLOW_MEMORY_MAX")
            .ok()
            .filter(|v| !v.is_empty()),
        tasks_max: env::var("SPINDLE_ENGINE_WORKFLOW_TASKS_MAX")
            .ok()
            .filter(|v| !v.is_empty())
            .and_then(|v| v.parse().ok()),
    };

    Ok(EngineConfig {
        kind,
        max_jobs,
        queue_size,
        workflow_timeout,
        nixery_url,
        extra_nix_flags,
        workflow_limits,
    })
}

/// Load secrets configuration from environment variables.
fn load_secrets_config() -> Result<SecretsConfig, ConfigError> {
    let provider_str = env_or_default("SPINDLE_SERVER_SECRETS_PROVIDER", "sqlite");
    let provider =
        SecretsProvider::parse(&provider_str).map_err(|message| ConfigError::InvalidValue {
            key: "SPINDLE_SERVER_SECRETS_PROVIDER".into(),
            message,
        })?;

    let openbao = OpenBaoConfig {
        proxy_addr: env_or_default(
            "SPINDLE_SERVER_SECRETS_OPENBAO_PROXY_ADDR",
            "http://127.0.0.1:8200",
        ),
        mount: env_or_default("SPINDLE_SERVER_SECRETS_OPENBAO_MOUNT", "spindle"),
    };

    Ok(SecretsConfig { provider, openbao })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a required environment variable, returning an error if missing or empty.
fn require_env(key: &str) -> Result<String, ConfigError> {
    match env::var(key) {
        Ok(val) if !val.is_empty() => Ok(val),
        _ => Err(ConfigError::MissingEnvVar(key.into())),
    }
}

/// Read an environment variable with a default fallback.
fn env_or_default(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// Parse a boolean environment variable (`true`/`false`/`1`/`0`).
fn parse_bool_env(key: &str, default: bool) -> Result<bool, ConfigError> {
    match env::var(key) {
        Ok(val) if !val.is_empty() => match val.to_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(ConfigError::InvalidValue {
                key: key.into(),
                message: format!("expected bool, got {val:?}"),
            }),
        },
        _ => Ok(default),
    }
}

/// Parse a `usize` environment variable.
fn parse_usize_env(key: &str, default: usize) -> Result<usize, ConfigError> {
    match env::var(key) {
        Ok(val) if !val.is_empty() => val.parse::<usize>().map_err(|_| ConfigError::InvalidValue {
            key: key.into(),
            message: format!("expected positive integer, got {val:?}"),
        }),
        _ => Ok(default),
    }
}

/// Parse a human-readable duration string.
///
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours).
/// Plain integers are treated as seconds.
///
/// Examples: `"5m"`, `"300s"`, `"1h"`, `"300"`.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".into());
    }

    if let Some(num) = s.strip_suffix('h') {
        let n: u64 = num
            .parse()
            .map_err(|_| format!("invalid hours value: {num:?}"))?;
        Ok(Duration::from_secs(n * 3600))
    } else if let Some(num) = s.strip_suffix('m') {
        let n: u64 = num
            .parse()
            .map_err(|_| format!("invalid minutes value: {num:?}"))?;
        Ok(Duration::from_secs(n * 60))
    } else if let Some(num) = s.strip_suffix('s') {
        let n: u64 = num
            .parse()
            .map_err(|_| format!("invalid seconds value: {num:?}"))?;
        Ok(Duration::from_secs(n))
    } else {
        // Bare number → seconds
        let n: u64 = s.parse().map_err(|_| {
            format!("invalid duration: {s:?} (expected e.g. \"5m\", \"300s\", \"1h\")")
        })?;
        Ok(Duration::from_secs(n))
    }
}

/// Format a [`Duration`] as a human-readable string (e.g. `"5m"`, `"300s"`).
#[allow(dead_code)]
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs == 0 {
        return "0s".into();
    }
    if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
    }

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration("300s").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("0s").unwrap(), Duration::from_secs(0));
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn parse_duration_bare_number() {
        assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5x").is_err());
    }

    #[test]
    fn format_duration_roundtrip() {
        assert_eq!(format_duration(Duration::from_secs(300)), "5m");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(90)), "90s");
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn engine_kind_parse() {
        assert_eq!(EngineKind::parse("nix").unwrap(), EngineKind::Nix);
        assert_eq!(EngineKind::parse("Nix").unwrap(), EngineKind::Nix);
        assert_eq!(EngineKind::parse("nixery").unwrap(), EngineKind::Nixery);
        assert!(EngineKind::parse("docker").is_err());
    }

    #[test]
    fn engine_kind_display() {
        assert_eq!(EngineKind::Nix.to_string(), "nix");
        assert_eq!(EngineKind::Nixery.to_string(), "nixery");
    }

    #[test]
    fn secrets_provider_parse() {
        assert_eq!(
            SecretsProvider::parse("sqlite").unwrap(),
            SecretsProvider::Sqlite
        );
        assert_eq!(
            SecretsProvider::parse("openbao").unwrap(),
            SecretsProvider::OpenBao
        );
        assert_eq!(
            SecretsProvider::parse("vault").unwrap(),
            SecretsProvider::OpenBao
        );
        assert!(SecretsProvider::parse("redis").is_err());
    }

    #[test]
    fn secrets_provider_display() {
        assert_eq!(SecretsProvider::Sqlite.to_string(), "sqlite");
        assert_eq!(SecretsProvider::OpenBao.to_string(), "openbao");
    }

    #[test]
    fn did_web_derivation() {
        // Simulate just the did:web derivation logic
        let hostname = "spindle1.example.com";
        let did_web = format!("did:web:{hostname}");
        assert_eq!(did_web, "did:web:spindle1.example.com");
    }
}
