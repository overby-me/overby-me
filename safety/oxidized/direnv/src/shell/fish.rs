use super::{Shell, ShellExport};
use crate::env::Env;
use crate::escape::fish_escape;

pub struct Fish;

const FISH_HOOK: &str = r#"
    function __direnv_export_eval --on-event fish_prompt;
        "{{.SelfPath}}" export fish | source;

        if test "$direnv_fish_mode" != "disable_arrow";
            function __direnv_cd_hook --on-variable PWD;
                if test "$direnv_fish_mode" = "eval_after_arrow";
                    set -g __direnv_export_again 0;
                else;
                    "{{.SelfPath}}" export fish | source;
                end;
            end;
        end;
    end;

    function __direnv_export_eval_2 --on-event fish_preexec;
        if set -q __direnv_export_again;
            set -e __direnv_export_again;
            "{{.SelfPath}}" export fish | source;
            echo;
        end;

        functions --erase __direnv_cd_hook;
    end;
"#;

impl Shell for Fish {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(FISH_HOOK.replace("{{.SelfPath}}", self_path))
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in &e.vars {
            match value {
                Some(v) => {
                    if key == "PATH" {
                        out.push_str("set -x -g PATH");
                        for path in v.split(':') {
                            out.push(' ');
                            out.push_str(&fish_escape(path));
                        }
                        out.push(';');
                    } else {
                        out.push_str("set -x -g ");
                        out.push_str(&fish_escape(key));
                        out.push(' ');
                        out.push_str(&fish_escape(v));
                        out.push(';');
                    }
                }
                None => {
                    out.push_str("set -e -g ");
                    out.push_str(&fish_escape(key));
                    out.push(';');
                }
            }
        }
        Ok(out)
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in env {
            if key == "PATH" {
                out.push_str("set -x -g PATH");
                for path in value.split(':') {
                    out.push(' ');
                    out.push_str(&fish_escape(path));
                }
                out.push(';');
            } else {
                out.push_str("set -x -g ");
                out.push_str(&fish_escape(key));
                out.push(' ');
                out.push_str(&fish_escape(value));
                out.push(';');
            }
        }
        Ok(out)
    }
}
