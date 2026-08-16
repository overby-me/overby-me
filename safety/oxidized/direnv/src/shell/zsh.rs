use super::{Shell, ShellExport};
use crate::env::Env;
use crate::escape::bash_escape;

pub struct Zsh;

const ZSH_HOOK: &str = r#"
_direnv_hook() {
  vars="$("{{.SelfPath}}" export zsh)"
  trap -- '' SIGINT
  eval "$vars"
  trap - SIGINT
}
typeset -ag precmd_functions
if (( ! ${precmd_functions[(I)_direnv_hook]} )); then
  precmd_functions=(_direnv_hook $precmd_functions)
fi
typeset -ag chpwd_functions
if (( ! ${chpwd_functions[(I)_direnv_hook]} )); then
  chpwd_functions=(_direnv_hook $chpwd_functions)
fi
"#;

impl Shell for Zsh {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(ZSH_HOOK.replace("{{.SelfPath}}", self_path))
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
