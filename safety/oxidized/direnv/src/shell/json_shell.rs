use super::{Shell, ShellExport};
use crate::env::Env;
use std::collections::BTreeMap;

pub struct JsonShell;

impl Shell for JsonShell {
    fn hook(&self, _self_path: &str) -> Result<String, String> {
        Err("this feature is not supported".to_string())
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let map: BTreeMap<&str, Option<&str>> = e
            .vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_deref()))
            .collect();
        serde_json::to_string_pretty(&map).map_err(|e| e.to_string())
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        serde_json::to_string_pretty(env).map_err(|e| e.to_string())
    }
}
