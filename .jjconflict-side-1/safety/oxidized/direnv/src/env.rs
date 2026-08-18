//! Environment variable types and operations.

use crate::gzenv;
use std::collections::BTreeMap;

/// Env is a map representation of environment variables.
pub type Env = BTreeMap<String, String>;

/// Get the current process environment as an Env map.
pub fn get_env() -> Env {
    std::env::vars().collect()
}

/// Convert an Env map to a Vec of "KEY=VALUE" strings.
#[allow(dead_code)]
pub fn to_go_env(env: &Env) -> Vec<String> {
    env.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

/// Serialize an Env to gzenv format.
pub fn serialize_env(env: &Env) -> String {
    gzenv::marshal(env)
}

/// Deserialize an Env from gzenv format.
pub fn load_env(gzenv_str: &str) -> Result<Env, String> {
    gzenv::unmarshal(gzenv_str)
}

/// Deserialize an Env from JSON bytes.
pub fn load_env_json(json_bytes: &[u8]) -> Result<Env, String> {
    serde_json::from_slice(json_bytes).map_err(|e| format!("json parsing: {e}"))
}

/// Clean all direnv-related context variables from an Env.
pub fn clean_context(env: &mut Env) {
    env.remove(DIRENV_DIFF);
    env.remove(DIRENV_DIR);
    env.remove(DIRENV_FILE);
    env.remove(DIRENV_DUMP_FILE_PATH);
    env.remove(DIRENV_WATCHES);
}

// Direnv environment variable names
pub const DIRENV_CONFIG: &str = "DIRENV_CONFIG";
pub const DIRENV_BASH: &str = "DIRENV_BASH";
pub const DIRENV_DIR: &str = "DIRENV_DIR";
pub const DIRENV_FILE: &str = "DIRENV_FILE";
pub const DIRENV_DIFF: &str = "DIRENV_DIFF";
pub const DIRENV_WATCHES: &str = "DIRENV_WATCHES";
pub const DIRENV_DUMP_FILE_PATH: &str = "DIRENV_DUMP_FILE_PATH";
#[allow(dead_code)]
pub const DIRENV_IN_ENVRC: &str = "DIRENV_IN_ENVRC";
pub const DIRENV_REQUIRED: &str = "DIRENV_REQUIRED";
