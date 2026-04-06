use super::*;

pub(super) fn builtin_set(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print all variables with proper quoting (like bash)
        let mut vars: Vec<_> = shell.vars.iter().collect();
        vars.sort_by_key(|(k, _)| (*k).clone());
        for (key, value) in vars {
            println!("{}={}", key, quote_value_for_set(value));
        }
        return 0;
    }

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Set positional parameters
            let new_positional: Vec<String> = args[i + 1..].to_vec();
            let prog = shell.positional.first().cloned().unwrap_or_default();
            shell.positional = vec![prog];
            shell.positional.extend(new_positional);
            shell.source_set_params = true;
            return 0;
        }
        if arg.starts_with('-') || arg.starts_with('+') {
            let enable = arg.starts_with('-');
            let flags = &arg[1..];

            if flags == "o" {
                // set -o option / set +o option
                // If next arg starts with - or +, it's a separate flag, not an option name
                if i + 1 < args.len()
                    && !args[i + 1].starts_with('-')
                    && !args[i + 1].starts_with('+')
                {
                    i += 1;
                    let option = &args[i];
                    match option.as_str() {
                        "allexport" => shell.opt_allexport = enable,
                        "pipefail" => shell.opt_pipefail = enable,
                        "errexit" => shell.opt_errexit = enable,
                        "nounset" => shell.opt_nounset = enable,
                        "xtrace" => shell.opt_xtrace = enable,
                        "noclobber" => shell.opt_noclobber = enable,
                        "keyword" => shell.opt_keyword = enable,
                        "noglob" => shell.opt_noglob = enable,
                        "noexec" => shell.opt_noexec = enable,
                        "posix" => {
                            shell.opt_posix = enable;
                            // POSIX mode enables alias expansion (bash behavior)
                            if enable {
                                shell.shopt_expand_aliases = true;
                            }
                        }
                        "hashall" => shell.opt_hashall = enable,
                        "braceexpand"
                        | "errtrace"
                        | "functrace"
                        | "histexpand"
                        | "history"
                        | "interactive-comments" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                        }
                        "ignoreeof" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                            if enable {
                                shell.set_var("IGNOREEOF", "10".to_string());
                            } else {
                                shell.vars.remove("IGNOREEOF");
                                shell.exports.remove("IGNOREEOF");
                            }
                        }
                        "monitor" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                            shell.opt_monitor = enable;
                        }
                        "physical" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                            shell.opt_physical = enable;
                        }
                        "notify" | "onecmd" | "privileged" | "verbose" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                        }
                        _ => {
                            eprintln!(
                                "{}: set: {}: invalid option name",
                                shell.error_prefix(),
                                option
                            );
                            return 2;
                        }
                    }
                } else {
                    let so = |name: &str, default: bool| -> bool {
                        shell.shopt_options.get(name).copied().unwrap_or(default)
                    };
                    let options: Vec<(&str, bool)> = vec![
                        ("allexport", shell.opt_allexport),
                        ("braceexpand", so("braceexpand", true)),
                        ("errexit", shell.opt_errexit),
                        ("errtrace", so("errtrace", false)),
                        ("functrace", so("functrace", false)),
                        ("hashall", shell.opt_hashall),
                        ("histexpand", so("histexpand", false)),
                        ("history", so("history", false)),
                        ("ignoreeof", so("ignoreeof", false)),
                        ("interactive-comments", so("interactive-comments", true)),
                        ("keyword", shell.opt_keyword),
                        ("monitor", so("monitor", false)),
                        ("noclobber", shell.opt_noclobber),
                        ("noexec", shell.opt_noexec),
                        ("noglob", shell.opt_noglob),
                        ("nolog", so("nolog", false)),
                        ("notify", so("notify", false)),
                        ("nounset", shell.opt_nounset),
                        ("onecmd", so("onecmd", false)),
                        ("physical", so("physical", false)),
                        ("pipefail", shell.opt_pipefail),
                        ("posix", shell.opt_posix),
                        ("privileged", so("privileged", false)),
                        ("verbose", so("verbose", false)),
                        ("xtrace", shell.opt_xtrace),
                    ];
                    if enable {
                        // set -o: human-readable option listing
                        for (name, val) in &options {
                            println!("{:<15}\t{}", name, if *val { "on" } else { "off" });
                        }
                    } else {
                        // set +o: reusable format
                        for (name, val) in &options {
                            println!("set {}o {}", if *val { "-" } else { "+" }, name);
                        }
                    }
                }
            } else {
                for flag in flags.chars() {
                    match flag {
                        'e' => shell.opt_errexit = enable,
                        'u' => shell.opt_nounset = enable,
                        'x' => shell.opt_xtrace = enable,
                        'f' => shell.opt_noglob = enable,
                        'k' => shell.opt_keyword = enable,
                        'C' => shell.opt_noclobber = enable,
                        'n' => shell.opt_noexec = enable,
                        'h' => shell.opt_hashall = enable,
                        'm' => shell.opt_monitor = enable,
                        'P' => {
                            shell.opt_physical = enable;
                            shell.shopt_options.insert("physical".to_string(), enable);
                        }
                        'a' => shell.opt_allexport = enable,
                        'b' | 'p' | 't' | 'v' | 'B' | 'E' | 'H' | 'T' => {
                            // Known but not fully implemented flags — accept silently
                        }
                        _ => {
                            eprintln!("{}: set: -{}: invalid option", shell.error_prefix(), flag);
                            eprintln!(
                                "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                            );
                            return 2;
                        }
                    }
                }
            }
        } else {
            // Set positional parameters
            let new_positional: Vec<String> = args[i..].to_vec();
            let prog = shell.positional.first().cloned().unwrap_or_default();
            shell.positional = vec![prog];
            shell.positional.extend(new_positional);
            shell.source_set_params = true;
            return 0;
        }
        i += 1;
    }
    shell.update_shellopts();
    0
}

pub(super) fn builtin_shopt(shell: &mut Shell, args: &[String]) -> i32 {
    let mut set = false;
    let mut unset = false;
    let mut query = false;
    let mut print_mode = false;
    let mut set_o = false;
    let mut opts = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            let mut valid = true;
            for ch in arg[1..].chars() {
                match ch {
                    's' => set = true,
                    'u' => unset = true,
                    'q' => query = true,
                    'p' => print_mode = true,
                    'o' => set_o = true,
                    _ => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                eprintln!("{}: shopt: {}: invalid option", shell.error_prefix(), arg);
                eprintln!("shopt: usage: shopt [-pqsu] [-o] [optname ...]");
                return 2;
            }
        } else {
            opts.push(arg.as_str());
        }
    }

    // Cannot set and unset simultaneously
    if set && unset {
        eprintln!(
            "{}: shopt: cannot set and unset shell options simultaneously",
            shell.error_prefix()
        );
        return 1;
    }

    // Handle -o (set -o options) separately — delegates to set -o options
    if set_o {
        let set_options: Vec<(&str, bool)> = vec![
            ("allexport", shell.opt_allexport),
            (
                "braceexpand",
                shell
                    .shopt_options
                    .get("braceexpand")
                    .copied()
                    .unwrap_or(true),
            ),
            (
                "emacs",
                shell.shopt_options.get("emacs").copied().unwrap_or(false),
            ),
            ("errexit", shell.opt_errexit),
            (
                "errtrace",
                shell
                    .shopt_options
                    .get("errtrace")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "functrace",
                shell
                    .shopt_options
                    .get("functrace")
                    .copied()
                    .unwrap_or(false),
            ),
            ("hashall", shell.opt_hashall),
            (
                "histexpand",
                shell
                    .shopt_options
                    .get("histexpand")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "history",
                shell.shopt_options.get("history").copied().unwrap_or(false),
            ),
            (
                "ignoreeof",
                shell
                    .shopt_options
                    .get("ignoreeof")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "interactive-comments",
                shell
                    .shopt_options
                    .get("interactive-comments")
                    .copied()
                    .unwrap_or(true),
            ),
            ("keyword", shell.opt_keyword),
            (
                "monitor",
                shell.shopt_options.get("monitor").copied().unwrap_or(false),
            ),
            ("noclobber", shell.opt_noclobber),
            ("noexec", shell.opt_noexec),
            ("noglob", shell.opt_noglob),
            ("nolog", false),
            (
                "notify",
                shell.shopt_options.get("notify").copied().unwrap_or(false),
            ),
            ("nounset", shell.opt_nounset),
            (
                "onecmd",
                shell.shopt_options.get("onecmd").copied().unwrap_or(false),
            ),
            (
                "physical",
                shell
                    .shopt_options
                    .get("physical")
                    .copied()
                    .unwrap_or(false),
            ),
            ("pipefail", shell.opt_pipefail),
            ("posix", shell.opt_posix),
            (
                "privileged",
                shell
                    .shopt_options
                    .get("privileged")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "verbose",
                shell.shopt_options.get("verbose").copied().unwrap_or(false),
            ),
            (
                "vi",
                shell.shopt_options.get("vi").copied().unwrap_or(false),
            ),
            ("xtrace", shell.opt_xtrace),
        ];

        if opts.is_empty() {
            // List all set -o options
            if !query {
                for (name, val) in &set_options {
                    if print_mode && set {
                        // shopt -s -p -o: only print ON options
                        if *val {
                            println!("set -o {}", name);
                        }
                    } else if print_mode && unset {
                        // shopt -u -p -o: only print OFF options
                        if !*val {
                            println!("set +o {}", name);
                        }
                    } else if print_mode {
                        // shopt -p -o: print all in set format
                        println!("set {}o {}", if *val { "-" } else { "+" }, name);
                    } else if set {
                        // shopt -s -o: list only ON options (human-readable)
                        if *val {
                            println!("{:<15}\ton", name);
                        }
                    } else if unset {
                        // shopt -u -o: list only OFF options (human-readable)
                        if !*val {
                            println!("{:<15}\toff", name);
                        }
                    } else {
                        println!("{:<15}\t{}", name, if *val { "on" } else { "off" });
                    }
                }
            }
            return 0;
        }

        // Handle specific set -o options
        let mut status = 0;
        for opt in &opts {
            if let Some((_, val)) = set_options.iter().find(|(n, _)| n == opt) {
                if set {
                    // set the option via builtin_set logic
                    match *opt {
                        "allexport" => shell.opt_allexport = true,
                        "errexit" => shell.opt_errexit = true,
                        "nounset" => shell.opt_nounset = true,
                        "xtrace" => shell.opt_xtrace = true,
                        "noclobber" => shell.opt_noclobber = true,
                        "noglob" => shell.opt_noglob = true,
                        "noexec" => shell.opt_noexec = true,
                        "posix" => shell.opt_posix = true,
                        "pipefail" => shell.opt_pipefail = true,
                        "hashall" => shell.opt_hashall = true,
                        "keyword" => shell.opt_keyword = true,
                        "physical" => {
                            shell.opt_physical = true;
                            shell.shopt_options.insert(opt.to_string(), true);
                        }
                        "monitor" => {
                            shell.opt_monitor = true;
                            shell.shopt_options.insert(opt.to_string(), true);
                        }
                        "ignoreeof" => {
                            shell.shopt_options.insert(opt.to_string(), true);
                            shell.set_var("IGNOREEOF", "10".to_string());
                        }
                        _ => {
                            shell.shopt_options.insert(opt.to_string(), true);
                        }
                    }
                } else if unset {
                    match *opt {
                        "allexport" => shell.opt_allexport = false,
                        "errexit" => shell.opt_errexit = false,
                        "nounset" => shell.opt_nounset = false,
                        "xtrace" => shell.opt_xtrace = false,
                        "noclobber" => shell.opt_noclobber = false,
                        "noglob" => shell.opt_noglob = false,
                        "noexec" => shell.opt_noexec = false,
                        "posix" => shell.opt_posix = false,
                        "pipefail" => shell.opt_pipefail = false,
                        "hashall" => shell.opt_hashall = false,
                        "keyword" => shell.opt_keyword = false,
                        "physical" => {
                            shell.opt_physical = false;
                            shell.shopt_options.insert(opt.to_string(), false);
                        }
                        "monitor" => {
                            shell.opt_monitor = false;
                            shell.shopt_options.insert(opt.to_string(), false);
                        }
                        "ignoreeof" => {
                            shell.shopt_options.insert(opt.to_string(), false);
                            shell.vars.remove("IGNOREEOF");
                            shell.exports.remove("IGNOREEOF");
                        }
                        _ => {
                            shell.shopt_options.insert(opt.to_string(), false);
                        }
                    }
                } else if !query {
                    if print_mode {
                        println!("set {}o {}", if *val { "-" } else { "+" }, opt);
                    } else {
                        println!("{:<15}\t{}", opt, if *val { "on" } else { "off" });
                    }
                } else if !*val {
                    status = 1;
                }
            } else {
                eprintln!(
                    "{}: shopt: {}: invalid option name",
                    shell.error_prefix(),
                    opt
                );
                status = 1;
            }
        }
        shell.update_shellopts();
        return status;
    }

    // All known shopt option names (accept silently even if not fully implemented)
    let all_known_opts = [
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

    // Options we actually track for listing
    let _shopt_options: Vec<(&str, bool)> = vec![
        ("expand_aliases", shell.shopt_expand_aliases),
        ("extglob", shell.shopt_extglob),
        ("globstar", false),
        ("inherit_errexit", shell.shopt_inherit_errexit),
        ("lastpipe", shell.shopt_lastpipe),
        ("nocasematch", shell.shopt_nocasematch),
        ("nullglob", shell.shopt_nullglob),
        ("xpg_echo", false),
    ];

    // Build the full options table with current values
    // Helper: get shopt option value, checking per-field bools first, then generic map
    let get_opt = |name: &str| -> bool {
        match name {
            "expand_aliases" => shell.shopt_expand_aliases,
            "extglob" => shell.shopt_extglob,
            "globstar" => shell.shopt_globstar,
            "inherit_errexit" => shell.shopt_inherit_errexit,
            "lastpipe" => shell.shopt_lastpipe,
            "nocasematch" => shell.shopt_nocasematch,
            "nullglob" => shell.shopt_nullglob,
            _ => *shell.shopt_options.get(name).unwrap_or(&false),
        }
    };
    // Default values for options (used when not explicitly set)
    let defaults: &[(&str, bool)] = &[
        ("checkwinsize", false),
        ("cmdhist", true),
        ("complete_fullquote", true),
        ("extquote", true),
        ("force_fignore", true),
        ("globasciiranges", true),
        ("globskipdots", true),
        ("hostcomplete", true),
        ("interactive_comments", true),
        ("patsub_replacement", true),
        ("progcomp", true),
        ("promptvars", true),
        ("sourcepath", true),
    ];
    let option_names: &[&str] = &[
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
    let all_options: Vec<(&str, bool)> = option_names
        .iter()
        .map(|&name| {
            let val = if shell.shopt_options.contains_key(name) {
                get_opt(name)
            } else {
                // Check defaults
                defaults
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| *v)
                    .unwrap_or_else(|| get_opt(name))
            };
            (name, val)
        })
        .collect();

    if opts.is_empty() && !set && !unset {
        // List all shopt options
        if !query {
            for (name, val) in &all_options {
                if print_mode {
                    println!("shopt {} {}", if *val { "-s" } else { "-u" }, name);
                } else {
                    {
                        let status_str = if *val { "on" } else { "off" };
                        let line = format!("{:<20}\t{}\n", name, status_str);
                        #[cfg(unix)]
                        nix::unistd::write(std::io::stdout(), line.as_bytes()).ok();
                        #[cfg(not(unix))]
                        print!("{}", line);
                    }
                }
            }
        }
        return 0;
    }

    if opts.is_empty() && (set || unset) {
        // List options that are set (-s) or unset (-u)
        if !query {
            for (name, val) in &all_options {
                if (set && *val) || (unset && !*val) {
                    if print_mode {
                        println!("shopt {} {}", if *val { "-s" } else { "-u" }, name);
                    } else {
                        {
                            let status_str = if *val { "on" } else { "off" };
                            let line = format!("{:<20}\t{}\n", name, status_str);
                            #[cfg(unix)]
                            nix::unistd::write(std::io::stdout(), line.as_bytes()).ok();
                            #[cfg(not(unix))]
                            print!("{}", line);
                        }
                    }
                }
            }
        }
        return 0;
    }

    let mut status = 0;
    for opt in &opts {
        match *opt {
            "nullglob" => {
                if set {
                    shell.shopt_nullglob = true;
                } else if unset {
                    shell.shopt_nullglob = false;
                } else if !query {
                    println!(
                        "{:<20}\t{}",
                        "nullglob",
                        if shell.shopt_nullglob { "on" } else { "off" }
                    );
                } else if !shell.shopt_nullglob {
                    status = 1;
                }
            }
            "globstar" => {
                if set {
                    shell.shopt_globstar = true;
                } else if unset {
                    shell.shopt_globstar = false;
                } else if !query {
                    println!(
                        "{:<20}\t{}",
                        "globstar",
                        if shell.shopt_globstar { "on" } else { "off" }
                    );
                } else if !shell.shopt_globstar {
                    status = 1;
                }
            }
            "extglob" => {
                if set {
                    shell.shopt_extglob = true;
                } else if unset {
                    shell.shopt_extglob = false;
                } else if !query {
                    println!(
                        "{:<20}\t{}",
                        "extglob",
                        if shell.shopt_extglob { "on" } else { "off" }
                    );
                } else if !shell.shopt_extglob {
                    status = 1;
                }
            }
            "inherit_errexit" => {
                if set {
                    shell.shopt_inherit_errexit = true;
                } else if unset {
                    shell.shopt_inherit_errexit = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} inherit_errexit",
                            if shell.shopt_inherit_errexit {
                                "-s"
                            } else {
                                "-u"
                            }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "inherit_errexit",
                            if shell.shopt_inherit_errexit {
                                "on"
                            } else {
                                "off"
                            }
                        );
                    }
                } else if !shell.shopt_inherit_errexit {
                    status = 1;
                }
            }
            "nocasematch" => {
                if set {
                    shell.shopt_nocasematch = true;
                } else if unset {
                    shell.shopt_nocasematch = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} nocasematch",
                            if shell.shopt_nocasematch { "-s" } else { "-u" }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "nocasematch",
                            if shell.shopt_nocasematch { "on" } else { "off" }
                        );
                    }
                } else if !shell.shopt_nocasematch {
                    status = 1;
                }
            }
            "lastpipe" => {
                if set {
                    shell.shopt_lastpipe = true;
                } else if unset {
                    shell.shopt_lastpipe = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} lastpipe",
                            if shell.shopt_lastpipe { "-s" } else { "-u" }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "lastpipe",
                            if shell.shopt_lastpipe { "on" } else { "off" }
                        );
                    }
                } else if !shell.shopt_lastpipe {
                    status = 1;
                }
            }
            "expand_aliases" => {
                if set {
                    shell.shopt_expand_aliases = true;
                } else if unset {
                    shell.shopt_expand_aliases = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} expand_aliases",
                            if shell.shopt_expand_aliases {
                                "-s"
                            } else {
                                "-u"
                            }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "expand_aliases",
                            if shell.shopt_expand_aliases {
                                "on"
                            } else {
                                "off"
                            }
                        );
                    }
                } else if !shell.shopt_expand_aliases {
                    status = 1;
                }
            }
            _ if all_known_opts.contains(opt) => {
                if set {
                    shell.shopt_options.insert(opt.to_string(), true);
                } else if unset {
                    shell.shopt_options.insert(opt.to_string(), false);
                } else if let Some((_, val)) = all_options.iter().find(|(n, _)| n == opt) {
                    if !query {
                        if print_mode {
                            println!("shopt {} {}", if *val { "-s" } else { "-u" }, opt);
                        } else {
                            println!("{:<20}\t{}", opt, if *val { "on" } else { "off" });
                        }
                    } else if !*val {
                        status = 1;
                    }
                }
            }
            _ => {
                if !query {
                    eprintln!(
                        "{}: shopt: {}: invalid shell option name",
                        shell.error_prefix(),
                        opt
                    );
                }
                status = 1;
            }
        }
    }
    status
}
