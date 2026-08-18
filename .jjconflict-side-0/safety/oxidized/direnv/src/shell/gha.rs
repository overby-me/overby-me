use super::{Shell, ShellExport};
use crate::env::Env;
use std::fmt::Write;
use std::io::Write as IoWrite;

pub struct Gha;

fn is_valid_env_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let bytes = key.as_bytes();
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

fn generate_delimiter() -> String {
    // Use random bytes to create a unique delimiter
    let mut buf = [0u8; 16];
    if getrandom(&mut buf).is_ok() {
        let hex: String = buf.iter().fold(String::new(), |mut s, b| {
            write!(s, "{b:02x}").unwrap();
            s
        });
        format!("ghadelimiter_{hex}")
    } else {
        // Fallback to timestamp-based
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("ghadelimiter_{ts:x}")
    }
}

fn getrandom(buf: &mut [u8]) -> Result<(), String> {
    // Read from /dev/urandom on Unix
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    f.read_exact(buf).map_err(|e| e.to_string())
}

fn gha_export(key: &str, value: &str) -> Result<String, String> {
    let delimiter = generate_delimiter();

    // Check for delimiter collision
    if key.contains(&delimiter) || value.contains(&delimiter) {
        // Retry once
        let delimiter2 = generate_delimiter();
        if key.contains(&delimiter2) || value.contains(&delimiter2) {
            return Err("delimiter collision persisted after retry".to_string());
        }
        return Ok(format!("{key}<<{delimiter2}\n{value}\n{delimiter2}\n"));
    }

    Ok(format!("{key}<<{delimiter}\n{value}\n{delimiter}\n"))
}

impl Shell for Gha {
    fn hook(&self, _self_path: &str) -> Result<String, String> {
        Err("this feature is not supported".to_string())
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let github_env =
            std::env::var("GITHUB_ENV").map_err(|_| "GITHUB_ENV not set".to_string())?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&github_env)
            .map_err(|e| format!("opening GITHUB_ENV: {e}"))?;

        for (key, value) in &e.vars {
            if !is_valid_env_key(key) {
                continue;
            }
            if let Some(v) = value {
                let entry = gha_export(key, v)?;
                file.write_all(entry.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
            // GHA doesn't support unsetting
        }

        Ok(String::new())
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let github_env =
            std::env::var("GITHUB_ENV").map_err(|_| "GITHUB_ENV not set".to_string())?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&github_env)
            .map_err(|e| format!("opening GITHUB_ENV: {e}"))?;

        for (key, value) in env {
            if !is_valid_env_key(key) {
                continue;
            }
            let entry = gha_export(key, value)?;
            file.write_all(entry.as_bytes())
                .map_err(|e| e.to_string())?;
        }

        Ok(String::new())
    }
}
