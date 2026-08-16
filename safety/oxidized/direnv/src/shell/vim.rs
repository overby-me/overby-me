use super::{Shell, ShellExport};
use crate::env::Env;

pub struct Vim;

fn vim_escape_value(s: &str) -> String {
    let escaped = s.replace('\n', "\\n").replace('\'', "''");
    format!("'{escaped}'")
}

impl Shell for Vim {
    fn hook(&self, _self_path: &str) -> Result<String, String> {
        Err("this feature is not supported. Install the direnv.vim plugin instead".to_string())
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in &e.vars {
            match value {
                Some(v) => {
                    out.push_str(&format!(
                        "call setenv({},{})\n",
                        vim_escape_value(key),
                        vim_escape_value(v)
                    ));
                }
                None => {
                    out.push_str(&format!("call setenv({},v:null)\n", vim_escape_value(key)));
                }
            }
        }
        Ok(out)
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in env {
            out.push_str(&format!(
                "call setenv({},{})\n",
                vim_escape_value(key),
                vim_escape_value(value)
            ));
        }
        Ok(out)
    }
}
