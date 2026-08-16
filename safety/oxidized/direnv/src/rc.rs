//! RC file management - finding, loading, allowing, and denying .envrc/.env files.

use crate::config::Config;
use crate::env::{DIRENV_DIFF, DIRENV_DIR, DIRENV_FILE, DIRENV_WATCHES, Env};
use crate::env_diff::EnvDiff;
use crate::escape::bash_escape;
use crate::file_times::FileTimes;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// AllowStatus represents the permission status of an RC file.
#[derive(Debug, PartialEq)]
pub enum AllowStatus {
    Allowed,
    NotAllowed,
    Denied,
}

/// RC represents the .envrc or .env file.
pub struct RC {
    pub path: String,
    pub allow_path: String,
    pub deny_path: String,
    pub times: FileTimes,
}

impl RC {
    /// Find an RC file (.envrc or .env) starting from the given directory.
    #[allow(dead_code)]
    pub fn find(wd: &str, config: &Config) -> Result<Option<Self>, String> {
        let rc_path = find_env_up(wd, config.load_dotenv);
        match rc_path {
            Some(path) => Self::from_path(&path, config).map(Some),
            None => Ok(None),
        }
    }

    /// Initialize an RC from a given path.
    pub fn from_path(path: &str, config: &Config) -> Result<Self, String> {
        let file_hash = file_hash(path)?;
        let allow_path = config
            .allow_dir()
            .join(&file_hash)
            .to_string_lossy()
            .to_string();

        let path_hash = path_hash(path)?;
        let deny_path = config
            .deny_dir()
            .join(&path_hash)
            .to_string_lossy()
            .to_string();

        let mut times = FileTimes::new();
        let _ = times.update(path);
        let _ = times.update(&allow_path);
        let _ = times.update(&deny_path);

        Ok(Self {
            path: path.to_string(),
            allow_path,
            deny_path,
            times,
        })
    }

    /// Initialize an RC from the environment.
    pub fn from_env(path: &str, marshalled_times: &str, config: &Config) -> Option<Self> {
        let file_hash = file_hash(path).ok()?;
        let allow_path = config
            .allow_dir()
            .join(&file_hash)
            .to_string_lossy()
            .to_string();

        let times = FileTimes::unmarshal(marshalled_times).ok()?;

        let path_hash = path_hash(path).ok()?;
        let deny_path = config
            .deny_dir()
            .join(&path_hash)
            .to_string_lossy()
            .to_string();

        Some(Self {
            path: path.to_string(),
            allow_path,
            deny_path,
            times,
        })
    }

    /// Allow grants the RC as allowed to load.
    pub fn allow(&mut self) -> Result<(), String> {
        if self.allow_path.is_empty() {
            return Err("cannot allow empty path".to_string());
        }

        if let Some(parent) = Path::new(&self.allow_path).parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        fs::write(&self.allow_path, format!("{}\n", self.path))
            .map_err(|e| format!("allow write: {e}"))?;

        self.times
            .update(&self.allow_path)
            .map_err(|e| format!("update times: {e}"))?;

        // Remove deny file if it exists
        if Path::new(&self.deny_path).exists() {
            let _ = fs::remove_file(&self.deny_path);
        }

        Ok(())
    }

    /// Deny revokes the permission of the RC file to load.
    pub fn deny(&self) -> Result<(), String> {
        if let Some(parent) = Path::new(&self.deny_path).parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        fs::write(&self.deny_path, format!("{}\n", self.path))
            .map_err(|e| format!("deny write: {e}"))?;

        // Remove allow file if it exists
        if Path::new(&self.allow_path).exists() {
            let _ = fs::remove_file(&self.allow_path);
        }

        Ok(())
    }

    /// Check if the RC file has been granted loading.
    pub fn allowed(&self, config: &Config) -> AllowStatus {
        // Check deny first
        if Path::new(&self.deny_path).exists() {
            return AllowStatus::Denied;
        }

        // Check explicit allow
        if Path::new(&self.allow_path).exists() {
            return AllowStatus::Allowed;
        }

        // Check whitelist exact
        if let Ok(abs_path) = fs::canonicalize(&self.path) {
            let abs_str = abs_path.to_string_lossy().to_string();
            if config.whitelist_exact.contains(&abs_str) {
                return AllowStatus::Allowed;
            }

            // Check whitelist prefix
            for prefix in &config.whitelist_prefix {
                if abs_str.starts_with(prefix) {
                    return AllowStatus::Allowed;
                }
            }
        }

        AllowStatus::NotAllowed
    }

    /// Touch updates the mtime of the RC file.
    #[allow(dead_code)]
    pub fn touch(&self) -> Result<(), String> {
        let now = filetime::FileTime::now();
        filetime::set_file_mtime(&self.path, now).map_err(|e| e.to_string())
    }

    /// Load evaluates the RC file and returns the new Env.
    pub fn load(&self, previous_env: &Env, config: &Config) -> Result<Env, String> {
        let mut new_env = previous_env.clone();
        new_env.insert(DIRENV_WATCHES.to_string(), self.times.marshal());

        // Check allow status
        match self.allowed(config) {
            AllowStatus::NotAllowed => {
                // Still set context vars even on error
                let dir = Path::new(&self.path)
                    .parent()
                    .map(|p| format!("-{}", p.to_string_lossy()))
                    .unwrap_or_default();
                new_env.insert(DIRENV_DIR.to_string(), dir);
                new_env.insert(DIRENV_FILE.to_string(), self.path.clone());
                let diff = EnvDiff::build(previous_env, &new_env);
                new_env.insert(DIRENV_DIFF.to_string(), diff.serialize());
                return Err(format!(
                    "{} is blocked. Run `direnv allow` to approve its content",
                    self.path
                ));
            }
            AllowStatus::Denied => {
                let dir = Path::new(&self.path)
                    .parent()
                    .map(|p| format!("-{}", p.to_string_lossy()))
                    .unwrap_or_default();
                new_env.insert(DIRENV_DIR.to_string(), dir);
                new_env.insert(DIRENV_FILE.to_string(), self.path.clone());
                let diff = EnvDiff::build(previous_env, &new_env);
                new_env.insert(DIRENV_DIFF.to_string(), diff.serialize());
                return Ok(new_env);
            }
            AllowStatus::Allowed => {}
        }

        // Determine if .env or .envrc
        let filename = Path::new(&self.path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let fn_name = if filename == ".env" {
            "dotenv"
        } else {
            "source_env"
        };

        let self_path = config.self_path.to_string_lossy();
        let prelude = if config.strict_env {
            "set -euo pipefail && "
        } else {
            ""
        };

        let escaped_path = bash_escape(&self.path.replace('\\', "/"));
        let arg = format!(
            r#"{prelude}eval "$("{self_path}" stdlib)" && __main__ {fn_name} {escaped_path}"#,
        );

        let mut cmd = Command::new(&config.bash_path);
        cmd.arg("-c").arg(&arg);
        cmd.current_dir(&config.work_dir);
        cmd.envs(new_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd.stderr(std::process::Stdio::inherit());

        if config.disable_stdin {
            cmd.stdin(std::process::Stdio::null());
        } else {
            cmd.stdin(std::process::Stdio::inherit());
        }

        // Start a warn timeout thread
        let warn_timeout = config.warn_timeout;
        let rc_path = self.path.clone();
        let warn_handle = std::thread::spawn(move || {
            std::thread::sleep(warn_timeout);
            eprintln!(
                "direnv: ({}) is taking a while to execute. Use `direnv status` to debug.",
                rc_path
            );
        });

        let output = cmd
            .output()
            .map_err(|e| format!("failed to run bash: {e}"))?;

        // Drop the warn handle - if the thread hasn't fired yet it will just end
        drop(warn_handle);

        if !output.stdout.is_empty() {
            match crate::env::load_env_json(&output.stdout) {
                Ok(env2) => {
                    new_env = env2;
                }
                Err(e) => {
                    // Set context vars
                    let dir = Path::new(&self.path)
                        .parent()
                        .map(|p| format!("-{}", p.to_string_lossy()))
                        .unwrap_or_default();
                    new_env.insert(DIRENV_DIR.to_string(), dir);
                    new_env.insert(DIRENV_FILE.to_string(), self.path.clone());
                    let diff = EnvDiff::build(previous_env, &new_env);
                    new_env.insert(DIRENV_DIFF.to_string(), diff.serialize());
                    return Err(format!("failed to parse env output: {e}"));
                }
            }
        }

        // Always set context vars
        let dir = Path::new(&self.path)
            .parent()
            .map(|p| format!("-{}", p.to_string_lossy()))
            .unwrap_or_default();
        new_env.insert(DIRENV_DIR.to_string(), dir);
        new_env.insert(DIRENV_FILE.to_string(), self.path.clone());
        let diff = EnvDiff::build(previous_env, &new_env);
        new_env.insert(DIRENV_DIFF.to_string(), diff.serialize());

        Ok(new_env)
    }
}

/// Find .envrc or .env walking up the directory tree.
pub fn find_env_up(search_dir: &str, load_dotenv: bool) -> Option<String> {
    if load_dotenv {
        find_up(search_dir, &[".envrc", ".env"])
    } else {
        find_up(search_dir, &[".envrc"])
    }
}

fn find_up(search_dir: &str, file_names: &[&str]) -> Option<String> {
    if search_dir.is_empty() {
        return None;
    }

    let mut dir = PathBuf::from(search_dir);
    loop {
        for name in file_names {
            let path = dir.join(name);
            if file_exists(&path) {
                return Some(path.to_string_lossy().to_string());
            }
        }

        if !dir.pop() {
            return None;
        }
    }
}

fn file_exists(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(_) => false,
    }
}

fn file_hash(path: &str) -> Result<String, String> {
    let abs = fs::canonicalize(path)
        .or_else(|_| {
            let p = PathBuf::from(path);
            if p.is_absolute() {
                Ok(p)
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(p))
                    .map_err(|e| e.to_string())
            }
        })
        .map_err(|e| format!("abs path: {e}"))?;
    let abs_str = abs.to_string_lossy();

    let content = fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;

    let mut hasher = Sha256::new();
    hasher.update(format!("{abs_str}\n").as_bytes());
    hasher.update(&content);

    Ok(format!("{:x}", hasher.finalize()))
}

fn path_hash(path: &str) -> Result<String, String> {
    let abs = fs::canonicalize(path)
        .or_else(|_| {
            let p = PathBuf::from(path);
            if p.is_absolute() {
                Ok(p)
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(p))
                    .map_err(|e| e.to_string())
            }
        })
        .map_err(|e| format!("abs path: {e}"))?;
    let abs_str = abs.to_string_lossy();

    let mut hasher = Sha256::new();
    hasher.update(format!("{abs_str}\n").as_bytes());

    Ok(format!("{:x}", hasher.finalize()))
}
