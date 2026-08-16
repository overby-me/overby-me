//! Environment diffing and patching.

use crate::env::Env;
use crate::gzenv;
use crate::shell::ShellExport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Keys that are always ignored in environment diffs.
const IGNORED_KEYS: &[&str] = &[
    "DIRENV_CONFIG",
    "DIRENV_BASH",
    "DIRENV_IN_ENVRC",
    "COMP_WORDBREAKS",
    "PS1",
    "OLDPWD",
    "PWD",
    "SHELL",
    "SHELLOPTS",
    "SHLVL",
    "_",
];

/// Check if a key should be ignored in environment diffs.
pub fn ignored_env(key: &str) -> bool {
    if key.starts_with("__fish") || key.starts_with("BASH_FUNC_") {
        return true;
    }
    IGNORED_KEYS.contains(&key)
}

/// EnvDiff represents the diff between two environments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvDiff {
    #[serde(rename = "p")]
    pub prev: BTreeMap<String, String>,
    #[serde(rename = "n")]
    pub next: BTreeMap<String, String>,
}

impl EnvDiff {
    /// Create a new empty EnvDiff.
    pub fn new() -> Self {
        Self {
            prev: BTreeMap::new(),
            next: BTreeMap::new(),
        }
    }

    /// Build an EnvDiff by comparing two environments.
    pub fn build(e1: &Env, e2: &Env) -> Self {
        let mut diff = Self::new();

        for (key, val1) in e1 {
            if ignored_env(key) {
                continue;
            }
            let val2 = e2.get(key);
            if val2 != Some(val1) {
                diff.prev.insert(key.clone(), val1.clone());
            }
        }

        for (key, val2) in e2 {
            if ignored_env(key) {
                continue;
            }
            let val1 = e1.get(key);
            if val1 != Some(val2) {
                diff.next.insert(key.clone(), val2.clone());
            }
        }

        diff
    }

    /// Check if the diff contains any changes.
    pub fn any(&self) -> bool {
        !self.prev.is_empty() || !self.next.is_empty()
    }

    /// Convert the diff to a ShellExport.
    pub fn to_shell_export(&self) -> ShellExport {
        let mut export = ShellExport::new();

        for key in self.prev.keys() {
            if !self.next.contains_key(key) {
                export.remove(key);
            }
        }

        for (key, value) in &self.next {
            export.add(key, value);
        }

        export
    }

    /// Patch applies the diff to the given env.
    pub fn patch(&self, env: &Env) -> Env {
        let mut new_env = env.clone();

        for key in self.prev.keys() {
            new_env.remove(key);
        }

        for (key, value) in &self.next {
            new_env.insert(key.clone(), value.clone());
        }

        new_env
    }

    /// Reverse flips the diff.
    pub fn reverse(&self) -> Self {
        Self {
            prev: self.next.clone(),
            next: self.prev.clone(),
        }
    }

    /// Serialize to gzenv format.
    pub fn serialize(&self) -> String {
        gzenv::marshal(self)
    }

    /// Deserialize from gzenv format.
    pub fn load(gzenv_str: &str) -> Result<Self, String> {
        gzenv::unmarshal(gzenv_str)
    }
}
