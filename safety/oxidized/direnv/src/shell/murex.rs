use super::{Shell, ShellExport};
use crate::env::Env;
use std::collections::BTreeMap;

pub struct Murex;

const MUREX_HOOK: &str = r#"event: onPrompt direnv_hook=before {
	"{{.SelfPath}}" export murex -> set exports
	if { $exports != "" } {
		$exports -> :json: formap key value {
			if { is-null value } then {
				!export "$key"
			} else {
				$value -> export "$key"
			}
		}
	}
}"#;

impl Shell for Murex {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(MUREX_HOOK.replace("{{.SelfPath}}", self_path))
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let map: BTreeMap<&str, Option<&str>> = e
            .vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_deref()))
            .collect();
        serde_json::to_string(&map).map_err(|e| e.to_string())
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        serde_json::to_string(env).map_err(|e| e.to_string())
    }
}
