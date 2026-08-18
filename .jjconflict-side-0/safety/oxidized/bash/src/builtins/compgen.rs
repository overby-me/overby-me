use super::*;

pub(super) fn builtin_complete(_shell: &mut Shell, _args: &[String]) -> i32 {
    0 // No-op
}

pub(super) fn builtin_compgen(shell: &mut Shell, args: &[String]) -> i32 {
    // Parse -A action and optional prefix
    let mut action = None;
    let mut prefix = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-A" => {
                i += 1;
                if i < args.len() {
                    action = Some(args[i].clone());
                }
            }
            a if !a.starts_with('-') => prefix = a.to_string(),
            _ => {}
        }
        i += 1;
    }

    match action.as_deref() {
        Some("function") => {
            let mut names: Vec<&String> = shell.functions.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("variable") => {
            let mut names: Vec<&String> = shell.vars.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("alias") => {
            let mut names: Vec<&String> = shell.aliases.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("builtin") => {
            let builtins = builtins();
            let mut names: Vec<&&str> = builtins.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("shopt") => {
            // List all shopt option names
            let all_shopts = [
                "array_expand_once",
                "assoc_expand_once",
                "autocd",
                "bash_source_fullpath",
                "cdable_vars",
                "cdspell",
                "checkhash",
                "checkjobs",
                "checkwinsize",
                "cmdhist",
                "compat31",
                "compat32",
                "compat40",
                "compat41",
                "compat42",
                "compat43",
                "compat44",
                "complete_fullquote",
                "direxpand",
                "dirspell",
                "dotglob",
                "execfail",
                "expand_aliases",
                "extdebug",
                "extglob",
                "extquote",
                "failglob",
                "force_fignore",
                "globasciiranges",
                "globskipdots",
                "globstar",
                "gnu_errfmt",
                "histappend",
                "histreedit",
                "histverify",
                "hostcomplete",
                "huponexit",
                "inherit_errexit",
                "interactive_comments",
                "lastpipe",
                "lithist",
                "localvar_inherit",
                "localvar_unset",
                "login_shell",
                "mailwarn",
                "no_empty_cmd_completion",
                "nocaseglob",
                "nocasematch",
                "noexpand_translation",
                "nullglob",
                "patsub_replacement",
                "progcomp",
                "progcomp_alias",
                "promptvars",
                "restricted_shell",
                "shift_verbose",
                "sourcepath",
                "varredir_close",
                "xpg_echo",
            ];
            for name in &all_shopts {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        _ => {}
    }
    0
}
