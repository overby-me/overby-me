use super::{Shell, ShellExport};
use crate::env::Env;
use crate::gzenv;
use std::collections::BTreeMap;

pub struct GzEnvShell;

impl Shell for GzEnvShell {
    fn hook(&self, _self_path: &str) -> Result<String, String> {
        Err("this feature is not supported".to_string())
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let map: BTreeMap<&str, Option<&str>> = e
            .vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_deref()))
            .collect();
        Ok(gzenv::marshal(&map))
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        Ok(gzenv::marshal(env))
    }
}
