use super::{Shell, ShellExport};
use crate::env::Env;
use crate::escape::tcsh_escape;

pub struct Tcsh;

impl Shell for Tcsh {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(format!("alias precmd 'eval `{self_path} export tcsh`'"))
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in &e.vars {
            match value {
                Some(v) => {
                    if key == "PATH" {
                        out.push_str("set path = (");
                        for path in v.split(':') {
                            out.push(' ');
                            out.push_str(&tcsh_escape(path));
                        }
                        out.push_str(" );");
                    } else {
                        out.push_str("setenv ");
                        out.push_str(&tcsh_escape(key));
                        out.push(' ');
                        out.push_str(&tcsh_escape(v));
                        out.push_str(" ;");
                    }
                }
                None => {
                    out.push_str("unsetenv ");
                    out.push_str(&tcsh_escape(key));
                    out.push_str(" ;");
                }
            }
        }
        Ok(out)
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in env {
            if key == "PATH" {
                out.push_str("set path = (");
                for path in value.split(':') {
                    out.push(' ');
                    out.push_str(&tcsh_escape(path));
                }
                out.push_str(" );");
            } else {
                out.push_str("setenv ");
                out.push_str(&tcsh_escape(key));
                out.push(' ');
                out.push_str(&tcsh_escape(value));
                out.push_str(" ;");
            }
        }
        Ok(out)
    }
}
