//! Configuration loading from direnv.toml.

use crate::env::{self, DIRENV_BASH, DIRENV_CONFIG, DIRENV_FILE, Env};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Config represents the direnv configuration and state.
pub struct Config {
    pub env: Env,
    pub work_dir: PathBuf,
    pub conf_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub self_path: PathBuf,
    pub bash_path: PathBuf,
    #[allow(dead_code)]
    pub rc_file: Option<String>,
    pub toml_path: Option<PathBuf>,
    pub hide_env_diff: bool,
    pub disable_stdin: bool,
    pub strict_env: bool,
    pub load_dotenv: bool,
    pub log_format: String,
    pub log_color: bool,
    pub warn_timeout: Duration,
    pub whitelist_prefix: Vec<String>,
    pub whitelist_exact: HashSet<String>,
}

#[derive(Deserialize, Default)]
struct TomlConfig {
    #[serde(flatten)]
    global: TomlGlobal,
    #[serde(default)]
    whitelist: TomlWhitelist,
}

#[derive(Deserialize, Default)]
struct TomlGlobal {
    bash_path: Option<String>,
    disable_stdin: Option<bool>,
    strict_env: Option<bool>,
    #[allow(dead_code)]
    skip_dotenv: Option<bool>,
    load_dotenv: Option<bool>,
    warn_timeout: Option<String>,
    hide_env_diff: Option<bool>,
    log_format: Option<String>,
}

#[derive(Deserialize, Default)]
struct TomlWhitelist {
    #[serde(default)]
    prefix: Vec<String>,
    #[serde(default)]
    exact: Vec<String>,
}

pub const DEFAULT_LOG_FORMAT: &str = "direnv: %s";

fn xdg_config_dir(env: &Env, app: &str) -> PathBuf {
    if let Some(dir) = env.get("XDG_CONFIG_HOME") {
        PathBuf::from(dir).join(app)
    } else if let Some(home) = env.get("HOME") {
        PathBuf::from(home).join(".config").join(app)
    } else {
        PathBuf::from(".config").join(app)
    }
}

fn xdg_cache_dir(env: &Env, app: &str) -> PathBuf {
    if let Some(dir) = env.get("XDG_CACHE_HOME") {
        PathBuf::from(dir).join(app)
    } else if let Some(home) = env.get("HOME") {
        PathBuf::from(home).join(".cache").join(app)
    } else {
        PathBuf::from(".cache").join(app)
    }
}

fn xdg_data_dir(env: &Env, app: &str) -> PathBuf {
    if let Some(dir) = env.get("XDG_DATA_HOME") {
        PathBuf::from(dir).join(app)
    } else if let Some(home) = env.get("HOME") {
        PathBuf::from(home).join(".local").join("share").join(app)
    } else {
        PathBuf::from(".local").join("share").join(app)
    }
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}

impl Config {
    /// Load config from the environment.
    pub fn load(current_env: &Env) -> Result<Self, String> {
        let conf_dir = if let Some(dir) = current_env.get(DIRENV_CONFIG) {
            PathBuf::from(dir)
        } else {
            xdg_config_dir(current_env, "direnv")
        };

        let self_path = std::env::current_exe().map_err(|e| format!("os.Executable: {e}"))?;
        let work_dir = std::env::current_dir().unwrap_or_default();

        let mut config = Config {
            env: current_env.clone(),
            work_dir,
            conf_dir: conf_dir.clone(),
            cache_dir: xdg_cache_dir(current_env, "direnv"),
            data_dir: xdg_data_dir(current_env, "direnv"),
            self_path,
            bash_path: PathBuf::from("bash"),
            rc_file: current_env.get(DIRENV_FILE).cloned(),
            toml_path: None,
            hide_env_diff: false,
            disable_stdin: false,
            strict_env: false,
            load_dotenv: false,
            log_format: DEFAULT_LOG_FORMAT.to_string(),
            log_color: current_env.get("TERM").is_none_or(|t| t != "dumb"),
            warn_timeout: Duration::from_secs(5),
            whitelist_prefix: Vec::new(),
            whitelist_exact: HashSet::new(),
        };

        // Look for toml config
        let toml_path = conf_dir.join("direnv.toml");
        let toml_path = if toml_path.exists() {
            Some(toml_path)
        } else {
            let alt = conf_dir.join("config.toml");
            if alt.exists() { Some(alt) } else { None }
        };

        if let Some(ref tp) = toml_path {
            config.toml_path = Some(tp.clone());
            let contents = std::fs::read_to_string(tp)
                .map_err(|e| format!("failed to read {}: {e}", tp.display()))?;
            let toml_conf: TomlConfig = toml::from_str(&contents)
                .map_err(|e| format!("failed to parse {}: {e}", tp.display()))?;

            if let Some(fmt) = current_env.get("DIRENV_LOG_FORMAT") {
                config.log_format = fmt.clone();
            } else if let Some(fmt) = toml_conf.global.log_format {
                config.log_format = if fmt == "-" { String::new() } else { fmt };
            }

            config.hide_env_diff = toml_conf.global.hide_env_diff.unwrap_or(false);

            for path in &toml_conf.whitelist.prefix {
                config.whitelist_prefix.push(expand_tilde(path));
            }

            for path in &toml_conf.whitelist.exact {
                let mut p = expand_tilde(path);
                if !p.ends_with("/.envrc") && !p.ends_with("/.env") {
                    p = format!("{p}/.envrc");
                }
                config.whitelist_exact.insert(p);
            }

            if let Some(bp) = toml_conf.global.bash_path {
                config.bash_path = PathBuf::from(bp);
            }
            config.disable_stdin = toml_conf.global.disable_stdin.unwrap_or(false);
            config.load_dotenv = toml_conf.global.load_dotenv.unwrap_or(false);
            config.strict_env = toml_conf.global.strict_env.unwrap_or(false);

            if let Some(wt) = toml_conf.global.warn_timeout
                && let Ok(d) = parse_duration(&wt)
            {
                config.warn_timeout = d;
            }
        }

        // Override warn timeout from env
        if let Some(ts) = current_env.get("DIRENV_WARN_TIMEOUT")
            && let Ok(d) = parse_duration(ts)
        {
            config.warn_timeout = d;
        }

        // Resolve bash path
        if let Some(bp) = current_env.get(DIRENV_BASH) {
            config.bash_path = PathBuf::from(bp);
        } else if config.bash_path == Path::new("bash") {
            // Try to find bash in PATH
            if let Ok(path) = which::which("bash") {
                config.bash_path = path;
            }
        }

        Ok(config)
    }

    /// AllowDir returns the folder where all the "allow" files are stored.
    pub fn allow_dir(&self) -> PathBuf {
        self.data_dir.join("allow")
    }

    /// DenyDir returns the folder where all the "deny" files are stored.
    pub fn deny_dir(&self) -> PathBuf {
        self.data_dir.join("deny")
    }

    /// Revert undoes the recorded changes to the supplied environment.
    pub fn revert(&self, env: &Env) -> Result<Env, String> {
        if let Some(diff_str) = self.env.get(env::DIRENV_DIFF)
            && !diff_str.is_empty()
        {
            let diff = crate::env_diff::EnvDiff::load(diff_str)?;
            return Ok(diff.reverse().patch(env));
        }
        Ok(env.clone())
    }
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    // Support formats like "5s", "500ms", "1m", "1h"
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("ms") {
        rest.parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|e| e.to_string())
    } else if let Some(rest) = s.strip_suffix('s') {
        rest.parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| e.to_string())
    } else if let Some(rest) = s.strip_suffix('m') {
        rest.parse::<u64>()
            .map(|m| Duration::from_secs(m * 60))
            .map_err(|e| e.to_string())
    } else if let Some(rest) = s.strip_suffix('h') {
        rest.parse::<u64>()
            .map(|h| Duration::from_secs(h * 3600))
            .map_err(|e| e.to_string())
    } else {
        // Try as seconds
        s.parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| e.to_string())
    }
}
