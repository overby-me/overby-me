use super::{Shell, ShellExport};
use crate::env::Env;
use crate::escape::bash_escape;

pub struct Bash;

const BASH_HOOK: &str = r#"
_direnv_hook() {
  local previous_exit_status=$?;
  vars="$("{{.SelfPath}}" export bash)";
  trap -- '' SIGINT;
  eval "$vars";
  trap - SIGINT;
  return $previous_exit_status;
};
if [[ ";${PROMPT_COMMAND[*]:-};" != *";_direnv_hook;"* ]]; then
  if [[ "$(declare -p PROMPT_COMMAND 2>&1)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(_direnv_hook "${PROMPT_COMMAND[@]}")
  else
    PROMPT_COMMAND="_direnv_hook${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  fi
fi
"#;

impl Shell for Bash {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(BASH_HOOK.replace("{{.SelfPath}}", self_path))
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in &e.vars {
            match value {
                Some(v) => {
                    out.push_str("export ");
                    out.push_str(&bash_escape(key));
                    out.push('=');
                    out.push_str(&bash_escape(v));
                    out.push(';');
                }
                None => {
                    out.push_str("unset ");
                    out.push_str(&bash_escape(key));
                    out.push(';');
                }
            }
        }
        Ok(out)
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in env {
            out.push_str("export ");
            out.push_str(&bash_escape(key));
            out.push('=');
            out.push_str(&bash_escape(value));
            out.push(';');
        }
        Ok(out)
    }
}
