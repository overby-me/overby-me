use super::{Shell, ShellExport};
use crate::env::Env;
use std::collections::BTreeMap;

pub struct Elvish;

const ELVISH_HOOK: &str = r#"## hook for direnv
set @edit:before-readline = $@edit:before-readline {
	try {
		var m = [("{{.SelfPath}}" export elvish | from-json)]
		if (> (count $m) 0) {
			set m = (all $m)
			keys $m | each { |k|
				if $m[$k] {
					set-env $k $m[$k]
				} else {
					unset-env $k
				}
			}
		}
	} catch e {
		echo $e
	}
}
"#;

impl Shell for Elvish {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(ELVISH_HOOK.replace("{{.SelfPath}}", self_path))
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        // Elvish uses JSON format, same as the JSON shell but with Option<String> values
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
