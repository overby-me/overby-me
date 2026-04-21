mod ast;
mod engine;
mod expand;
mod parser;

use engine::Engine;

fn main() {
    // Exit gracefully on stdout write errors (e.g. /dev/full, broken pipe)
    // instead of panicking with exit code 101.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        if msg.contains("failed printing to") {
            eprintln!("make: write error");
            std::process::exit(1);
        }
        default_hook(info);
    }));

    let code = run();

    // Flush stdout; exit 1 on write error (e.g. /dev/full).
    use std::io::Write;
    if std::io::stdout().flush().is_err() {
        eprintln!("make: write error");
        std::process::exit(1);
    }
    std::process::exit(code);
}

fn run() -> i32 {
    // GNU make prepends GNUMAKEFLAGS and MAKEFLAGS env to argv so options
    // and cmdline vars from them are applied before explicit command-line
    // args. Unset GNUMAKEFLAGS afterward so recursive makes don't
    // re-apply them; MAKEFLAGS is rebuilt below and propagated via env.
    let mut args: Vec<String> = std::env::args().collect();
    let mut prepend: Vec<String> = Vec::new();
    if let Ok(flags) = std::env::var("GNUMAKEFLAGS") {
        prepend.extend(flags.split_whitespace().map(|s| s.to_string()));
        unsafe {
            std::env::set_var("GNUMAKEFLAGS", "");
        }
    }
    if let Ok(flags) = std::env::var("MAKEFLAGS") {
        // MAKEFLAGS entries without a leading `-` are either short-flag
        // clusters (e.g. `erR`) or `VAR=value` assignments. The token
        // `--` is a separator GNU make inserts between flags and
        // command-line variable assignments — skip it.
        // Use split_makeflags to handle `\ ` (backslash-space) escaping
        // in --eval values.
        for tok in split_makeflags(&flags) {
            if tok == "--" {
                continue;
            }
            // Decode --eval values from MAKEFLAGS encoding
            if let Some(encoded) = tok.strip_prefix("--eval=") {
                let decoded = decode_makeflags_eval(encoded);
                prepend.push(format!("--eval={decoded}"));
                continue;
            }
            if tok.starts_with("-") || tok.contains('=') {
                prepend.push(tok);
            } else {
                prepend.push(format!("-{tok}"));
            }
        }
    }
    let prepend_count = prepend.len();
    if !prepend.is_empty() {
        let mut new_args = Vec::with_capacity(args.len() + prepend.len());
        new_args.push(args.remove(0));
        new_args.extend(prepend);
        new_args.extend(args);
        args = new_args;
    }
    // Index where real command-line args start (after argv[0] and prepended MAKEFLAGS entries).
    let cmdline_start: usize = 1 + prepend_count;
    let mut engine = Engine::new();

    // Set $(MAKE) to argv[0] so recursive make and tests can locate this
    // binary. GNU make does the same.
    if let Some(arg0) = args.first() {
        engine.set_var_with_origin(
            "MAKE",
            arg0,
            engine::VarFlavor::Simple,
            engine::VarOrigin::Default,
        );
    }

    // Track MAKEFLAGS as we parse options. Short flags without arguments
    // concatenate into a single cluster (e.g. `-erR` → "erR"). Long flags
    // and flags-with-args accumulate as separate tokens prefixed with `-`
    // (e.g. `--trace`).
    let mut mflags_short = String::new();
    let mut mflags_long: Vec<String> = Vec::new();
    // `--eval=TEXT` strings, evaluated before the primary makefile.
    let mut eval_strings: Vec<String> = Vec::new();
    // Track command-line variable overrides for MAKEOVERRIDES.
    // Only real command-line args (not inherited from MAKEFLAGS) go here.
    let mut makeoverrides: Vec<String> = Vec::new();

    let mut targets = Vec::new();
    let mut makefiles: Vec<String> = Vec::new();
    let mut directory: Option<String> = None;
    let mut debug_mode = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-f" | "--file" | "--makefile" => {
                i += 1;
                if i < args.len() {
                    makefiles.push(args[i].clone());
                } else {
                    eprintln!("make: the '{}' option requires an argument", args[i - 1]);
                    return 2;
                }
            }
            "-C" | "--directory" => {
                i += 1;
                if i < args.len() {
                    directory = Some(args[i].clone());
                } else {
                    eprintln!("make: the '{}' option requires an argument", args[i - 1]);
                    return 2;
                }
            }
            "-I" | "--include-dir" => {
                i += 1;
                if i < args.len() {
                    // `-I -` (with the literal `-` as argument) clears
                    // any include dirs accumulated so far — matching
                    // GNU make's convention for removing the defaults.
                    if args[i] == "-" {
                        engine.include_dirs.borrow_mut().clear();
                        mflags_long.push("-I-".to_string());
                    } else {
                        engine.include_dirs.borrow_mut().push(args[i].clone());
                        mflags_long.push(format!("-I{}", args[i]));
                    }
                } else {
                    eprintln!("make: the '{}' option requires an argument", args[i - 1]);
                    return 2;
                }
            }
            arg if arg.starts_with("-I") && arg.len() > 2 => {
                let rest = &arg[2..];
                if rest == "-" {
                    engine.include_dirs.borrow_mut().clear();
                    mflags_long.push("-I-".to_string());
                } else {
                    engine.include_dirs.borrow_mut().push(rest.to_string());
                    mflags_long.push(format!("-I{rest}"));
                }
            }
            arg if arg.starts_with("--include-dir=") => {
                let rest = &arg["--include-dir=".len()..];
                if rest == "-" {
                    engine.include_dirs.borrow_mut().clear();
                    mflags_long.push("-I-".to_string());
                } else {
                    engine.include_dirs.borrow_mut().push(rest.to_string());
                    mflags_long.push(format!("-I{rest}"));
                }
            }
            "-W" | "--what-if" | "--new-file" | "--assume-new" => {
                i += 1;
                if i < args.len() {
                    engine
                        .assume_new
                        .borrow_mut()
                        .insert(normalize_path(&args[i]).to_string());
                }
            }
            arg if arg.starts_with("-W") && arg.len() > 2 => {
                engine
                    .assume_new
                    .borrow_mut()
                    .insert(normalize_path(&arg[2..]).to_string());
            }
            arg if arg.starts_with("--what-if=") => {
                engine
                    .assume_new
                    .borrow_mut()
                    .insert(normalize_path(&arg["--what-if=".len()..]).to_string());
            }
            arg if arg.starts_with("--new-file=") => {
                engine
                    .assume_new
                    .borrow_mut()
                    .insert(normalize_path(&arg["--new-file=".len()..]).to_string());
            }
            arg if arg.starts_with("--assume-new=") => {
                engine
                    .assume_new
                    .borrow_mut()
                    .insert(normalize_path(&arg["--assume-new=".len()..]).to_string());
            }
            "-o" | "--old-file" | "--assume-old" => {
                // Opposite of -W: assume file is old, never rebuild.
                i += 1;
            }
            "-j" | "--jobs" => {
                // Only consume the next arg if it parses as a positive
                // integer. Otherwise `-j` alone means "unlimited" and the
                // next arg is a target.
                if let Some(next) = args.get(i + 1)
                    && let Ok(n) = next.parse::<usize>()
                {
                    engine.jobs = n;
                    i += 1;
                } else {
                    engine.jobs = 0; // unlimited
                }
            }
            "-l" | "--load-average" | "--max-load" => {
                // Accept -l with a float argument. TODO: implement load limiting.
                i += 1;
            }
            "-n" | "--just-print" | "--dry-run" | "--recon" => {
                engine.dry_run = true;
                mflags_short.push('n');
            }
            "-s" | "--silent" | "--quiet" => {
                engine.silent = true;
                mflags_short.push('s');
                mflags_long.retain(|s| s != "--no-silent");
            }
            "--no-silent" => {
                engine.silent = false;
                mflags_short.retain(|c| c != 's');
                mflags_long.push("--no-silent".to_string());
            }
            "-k" | "--keep-going" => {
                engine.keep_going = true;
                mflags_short.push('k');
            }
            "--no-keep-going" | "--stop" => {
                engine.keep_going = false;
                mflags_short.retain(|c| c != 'k');
            }
            "-t" | "--touch" => {
                engine.touch = true;
                mflags_short.push('t');
            }
            "-q" | "--question" => {
                engine.question = true;
                mflags_short.push('q');
            }
            "-B" | "--always-make" => {
                engine.always_make.set(true);
                mflags_short.push('B');
            }
            "-i" | "--ignore-errors" => {
                engine.ignore_errors = true;
                mflags_short.push('i');
            }
            "-e" | "--environment-overrides" => {
                engine.env_overrides = true;
                mflags_short.push('e');
            }
            "-r" | "--no-builtin-rules" => {
                engine.disable_builtin_rules();
                mflags_short.push('r');
            }
            "-R" | "--no-builtin-variables" => {
                engine.disable_builtin_rules();
                engine.disable_builtin_vars();
                mflags_short.push('R');
            }
            "-w" | "--print-directory" => {
                engine.print_directory_opt = Some(true);
                mflags_long.retain(|s| s != "--no-print-directory");
                mflags_long.push("--print-directory".to_string());
            }
            "--no-print-directory" => {
                engine.print_directory_opt = Some(false);
                mflags_long.retain(|s| s != "--print-directory");
                mflags_long.push("--no-print-directory".to_string());
            }
            "--trace" => {
                mflags_long.push("--trace".to_string());
                engine.trace = true;
            }
            "--warn-undefined-variables" => {
                engine.warn_undefined_variables.set(true);
                mflags_long.push("--warn-undefined-variables".to_string());
            }
            "-d" | "--debug" | "--debug=a" | "--debug=b" | "--debug=basic" | "--debug=v"
            | "--debug=verbose" | "--debug=i" | "--debug=implicit" | "--debug=j"
            | "--debug=jobs" | "--debug=m" | "--debug=makefile" | "--debug=n" | "--debug=none" => {
                debug_mode = true;
            }
            arg if arg.starts_with("--debug=") => {
                debug_mode = true;
            }
            arg if arg.starts_with("--eval=") => {
                eval_strings.push(arg["--eval=".len()..].to_string());
            }
            "-E" | "--eval" => {
                i += 1;
                if i < args.len() {
                    eval_strings.push(args[i].clone());
                }
            }
            "-p" | "--print-data-base" => {
                // TODO: print database
            }
            "-v" | "--version" => {
                // Mimic GNU make's version format so the upstream test
                // driver accepts us as a compatible `make`.
                println!("GNU Make 4.4.1 (rust-make {})", env!("CARGO_PKG_VERSION"));
                println!("Built for x86_64-pc-linux-gnu");
                println!("Copyright (C) 1988-2023 Free Software Foundation, Inc.");
                println!(
                    "License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>"
                );
                println!("This is free software: you are free to change and redistribute it.");
                println!("There is NO WARRANTY, to the extent permitted by law.");
                return 0;
            }
            "-h" | "--help" => {
                println!("Usage: make [options] [target] ...");
                println!("Options:");
                println!("  -f FILE  Read FILE as a makefile");
                println!("  -C DIR   Change to DIR before doing anything");
                println!("  -j N     Allow N jobs at once");
                println!("  -n       Dry run (print commands without executing)");
                println!("  -s       Silent mode");
                println!("  -k       Keep going on errors");
                println!("  -t       Touch targets instead of building");
                println!("  -q       Question mode (exit 1 if not up to date)");
                println!("  -B       Always make all targets");
                println!("\nThis program built for x86_64-pc-linux-gnu");
                return 0;
            }
            arg if arg.starts_with("-j") => match arg[2..].parse::<usize>() {
                Ok(n) => engine.jobs = n,
                Err(_) => {
                    eprintln!("make: invalid integer argument '{}' for '-j'", &arg[2..]);
                    return 2;
                }
            },
            arg if arg.starts_with("-l") && arg.len() > 2 => {
                // -l0.0001 form — accept and ignore.
            }
            arg if arg.starts_with("--load-average=") || arg.starts_with("--max-load=") => {
                // Accept and ignore.
            }
            arg if arg.starts_with("-f") => {
                makefiles.push(arg[2..].to_string());
            }
            arg if arg.starts_with("--file=") => {
                makefiles.push(arg["--file=".len()..].to_string());
            }
            arg if arg.starts_with("--makefile=") => {
                makefiles.push(arg["--makefile=".len()..].to_string());
            }
            arg if arg.starts_with("-C") => {
                directory = Some(arg[2..].to_string());
            }
            "--shuffle" => {
                *engine.shuffle_mode.borrow_mut() = Some("random".to_string());
            }
            arg if arg.starts_with("--shuffle=") => {
                *engine.shuffle_mode.borrow_mut() = Some(arg["--shuffle=".len()..].to_string());
            }
            arg if arg.contains('=') => {
                // Command-line variable assignment. Distinguish the
                // operator so flavor matches GNU make:
                //   VAR=val  → recursive
                //   VAR:=val / VAR::=val → simple
                //   VAR?=val → conditional (skip if already defined)
                //   VAR+=val → append
                let is_real_cmdline = i >= cmdline_start;
                // Track the original separator so MAKEOVERRIDES preserves it
                // (`:=` stays as `:=` in MAKEFLAGS, GNU make convention).
                let mut orig_sep: &str = "=";
                let (name, op, value) = if let Some(idx) = arg.find("::=") {
                    orig_sep = "::=";
                    (&arg[..idx], engine::VarFlavor::Simple, &arg[idx + 3..])
                } else if let Some(idx) = arg.find(":=") {
                    orig_sep = ":=";
                    (&arg[..idx], engine::VarFlavor::Simple, &arg[idx + 2..])
                } else if let Some(idx) = arg.find("+=") {
                    // Append — combine with any existing value.
                    let name = &arg[..idx];
                    let new_val = &arg[idx + 2..];
                    let existing = engine.lookup_var(name);
                    let combined = if existing.is_empty() {
                        new_val.to_string()
                    } else {
                        format!("{existing} {new_val}")
                    };
                    engine.set_var_with_origin(
                        name,
                        &combined,
                        engine::VarFlavor::Recursive,
                        engine::VarOrigin::CommandLine,
                    );
                    if is_real_cmdline {
                        let escaped = combined.replace('$', "$$");
                        makeoverrides.push(format!("{name}={escaped}"));
                    }
                    i += 1;
                    continue;
                } else if let Some(idx) = arg.find("?=") {
                    if !engine.lookup_var(&arg[..idx]).is_empty() {
                        i += 1;
                        continue;
                    }
                    (&arg[..idx], engine::VarFlavor::Recursive, &arg[idx + 2..])
                } else if let Some(idx) = arg.find('=') {
                    (&arg[..idx], engine::VarFlavor::Recursive, &arg[idx + 1..])
                } else {
                    i += 1;
                    continue;
                };
                engine.set_var_with_origin(name, value, op, engine::VarOrigin::CommandLine);
                if is_real_cmdline {
                    // Escape $ as $$ so MAKEOVERRIDES expansion doesn't
                    // trigger unterminated variable reference errors for
                    // values like x=$(other.
                    let escaped = value.replace('$', "$$");
                    makeoverrides.push(format!("{name}{orig_sep}{escaped}"));
                }
            }
            arg if arg.starts_with("--") => {
                // Unknown long option: GNU make prints an error and
                // a usage banner that includes the "built for" line.
                eprintln!("make: unrecognized option '{arg}'");
                eprintln!("Usage: make [options] [target] ...");
                eprintln!();
                eprintln!("This program built for x86_64-pc-linux-gnu");
                return 2;
            }
            arg if arg.starts_with('-') => {
                // Combined short flags. Some flags (-f, -C, -I, -j, -W, -l, -o)
                // take an argument; when they appear in a combined cluster they
                // consume the rest of the arg, or the next argv entry if none.
                let flags: Vec<char> = arg[1..].chars().collect();
                let mut idx = 0;
                let mut unknown = false;
                while idx < flags.len() {
                    let flag = flags[idx];
                    let take_arg = |flags: &[char], idx: usize, i: &mut usize| -> Option<String> {
                        let rest: String = flags[idx + 1..].iter().collect();
                        if !rest.is_empty() {
                            Some(rest)
                        } else {
                            *i += 1;
                            if *i < args.len() {
                                Some(args[*i].clone())
                            } else {
                                None
                            }
                        }
                    };
                    match flag {
                        'n' => engine.dry_run = true,
                        's' => engine.silent = true,
                        'k' => engine.keep_going = true,
                        't' => engine.touch = true,
                        'q' => engine.question = true,
                        'B' => engine.always_make.set(true),
                        'i' => engine.ignore_errors = true,
                        'e' => engine.env_overrides = true,
                        'w' => {}
                        'r' => engine.disable_builtin_rules(),
                        'R' => {
                            engine.disable_builtin_rules();
                            engine.disable_builtin_vars();
                        }
                        'f' => {
                            if let Some(v) = take_arg(&flags, idx, &mut i) {
                                makefiles.push(v);
                            }
                            break;
                        }
                        'C' => {
                            if let Some(v) = take_arg(&flags, idx, &mut i) {
                                directory = Some(v);
                            }
                            break;
                        }
                        'j' => {
                            if let Some(v) = take_arg(&flags, idx, &mut i) {
                                engine.jobs = v.parse().unwrap_or(1);
                            }
                            break;
                        }
                        'I' => {
                            if let Some(v) = take_arg(&flags, idx, &mut i) {
                                if v == "-" {
                                    engine.include_dirs.borrow_mut().clear();
                                    mflags_long.push("-I-".to_string());
                                } else {
                                    engine.include_dirs.borrow_mut().push(v.clone());
                                    mflags_long.push(format!("-I{v}"));
                                }
                            }
                            break;
                        }
                        'W' => {
                            // Consume the filename argument.
                            let _ = take_arg(&flags, idx, &mut i);
                            break;
                        }
                        'l' => {
                            // -l takes a float argument (rest of cluster or next arg)
                            let _ = take_arg(&flags, idx, &mut i);
                            break;
                        }
                        'o' => {
                            let _ = take_arg(&flags, idx, &mut i);
                            break;
                        }
                        _ => {
                            eprintln!("make: Unknown option '-{flag}'");
                            unknown = true;
                        }
                    }
                    idx += 1;
                }
                if unknown {}
            }
            _ => {
                targets.push(args[i].clone());
            }
        }
        i += 1;
    }

    if debug_mode {
        eprintln!("GNU Make 4.4.1 (rust-make)");
    }

    // Change directory if requested
    if let Some(dir) = directory
        && let Err(e) = std::env::set_current_dir(&dir)
    {
        eprintln!("make: *** {dir}: {e}.  Stop.");
        return 2;
    }

    // Find and load makefiles. If -f was given (potentially multiple
    // times), load each. Otherwise pick the first default that exists.
    let makefile_paths: Vec<String> = if !makefiles.is_empty() {
        makefiles
    } else if std::path::Path::new("GNUmakefile").exists() {
        vec!["GNUmakefile".to_string()]
    } else if std::path::Path::new("makefile").exists() {
        vec!["makefile".to_string()]
    } else if std::path::Path::new("Makefile").exists() {
        vec!["Makefile".to_string()]
    } else if eval_strings.is_empty() {
        eprintln!("make: *** No makefile found.  Stop.");
        return 2;
    } else {
        // -E/--eval provided but no default makefile — will try
        // them as -include later (after eval strings are processed).
        vec![]
    };

    // Populate MAKECMDGOALS before loading the makefile so user
    // conditionals can check it.
    engine.set_var_with_origin(
        "MAKECMDGOALS",
        &targets.join(" "),
        engine::VarFlavor::Simple,
        engine::VarOrigin::Default,
    );

    // Populate MAKEOVERRIDES from real command-line variable assignments.
    // GNU make stores these so sub-makes can inherit command-line overrides.
    engine.set_var_with_origin(
        "MAKEOVERRIDES",
        &makeoverrides.join(" "),
        engine::VarFlavor::Recursive,
        engine::VarOrigin::Default,
    );

    // Build MAKEFLAGS: short flags concatenated (no leading '-'), then
    // long flags as separate `--opt` tokens. GNU make includes
    // $(MAKEOVERRIDES) after a `--` separator so sub-makes inherit
    // command-line variable assignments. MAKEFLAGS is recursive so
    // changes to MAKEOVERRIDES (even from inside a makefile) are
    // reflected when MAKEFLAGS is exported to child processes.
    let mut mflags = mflags_short.clone();
    // Sort long flags in GNU make's switches-table order.
    // Add --eval strings to mflags_long for MAKEFLAGS propagation.
    // Encode: $ -> $$$$ (becomes $$ after one make expansion), space -> \\ (backslash-space).
    for text in &eval_strings {
        let encoded = encode_eval_for_makeflags(text);
        mflags_long.push(format!("--eval={encoded}"));
    }

    mflags_long.sort_by_key(|f| {
        let key = f.strip_prefix("--").unwrap_or(f);
        match key {
            _ if f.starts_with("-I") => 0,   // -I (include-dir)
            "trace" => 1,                    // CHAR_MAX+1
            "print-directory" => 2,          // -w
            "no-print-directory" => 3,       // CHAR_MAX+2
            "warn-undefined-variables" => 4, // CHAR_MAX+3
            "no-silent" => 5,                // CHAR_MAX+4
            _ => 6,
        }
    });
    if mflags_short.is_empty() && !mflags_long.is_empty() {
        mflags.push(' ');
    }
    for (idx, long) in mflags_long.iter().enumerate() {
        if idx > 0 || !mflags_short.is_empty() {
            mflags.push(' ');
        }
        mflags.push_str(long);
    }
    // Use $(MAKEOVERRIDES) so the value is resolved dynamically -- if
    // the makefile appends to MAKEOVERRIDES, MAKEFLAGS picks it up.
    // Only include the " -- " separator when MAKEOVERRIDES is non-empty,
    // otherwise MAKEFLAGS would always end with " --".
    mflags.push_str("$(if $(MAKEOVERRIDES), -- $(MAKEOVERRIDES))");
    engine.set_var_with_origin(
        "MAKEFLAGS",
        &mflags,
        engine::VarFlavor::Recursive,
        engine::VarOrigin::Default,
    );

    // Auto-include any makefiles listed in the MAKEFILES variable before
    // loading the primary makefile (GNU make behavior). MAKEFILES can be
    // set via env or command-line assignment; by the time we get here
    // both have been absorbed into engine state, so reading the engine
    // var covers both.
    // Print "Entering directory" early (before load_file) so $(info) in
    // makefiles appears after it. For sub-makes (MAKELEVEL > 0) this
    // is the default; for top-level make only if -w is explicit.
    let cwd_display = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let restarts: i32 = std::env::var("MAKE_RESTARTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let makelevel: i32 = engine.lookup_var_or("MAKELEVEL", "0").parse().unwrap_or(0);
    let should_print_dir = match engine.print_directory_opt {
        Some(choice) => choice,
        None => makelevel > 0,
    };
    let printed_entering_early = should_print_dir && restarts == 0;
    if printed_entering_early {
        let make_tag = if makelevel > 0 {
            format!("make[{makelevel}]")
        } else {
            "make".to_string()
        };
        println!("{make_tag}: Entering directory '{cwd_display}'");
    }
    engine.printed_entering.set(printed_entering_early);

    // Evaluate any `--eval=TEXT` strings before the primary makefile
    // and before MAKEFILES env loading (GNU make processes -E first).
    for text in &eval_strings {
        engine.load_string(text);
    }

    // Auto-include any makefiles listed in the MAKEFILES variable.
    let makefiles_list = engine.lookup_var("MAKEFILES");
    if !makefiles_list.trim().is_empty() {
        *engine.suppress_default_goal.borrow_mut() = true;
        for path in makefiles_list.split_whitespace() {
            if !engine.load_file_with_loc(path, true, None) {
                engine.pending_includes.borrow_mut().push((
                    path.to_string(),
                    true,
                    String::new(),
                    0,
                ));
            }
        }
        *engine.suppress_default_goal.borrow_mut() = false;
    }

    // When no explicit makefile is given (-E only mode), the default
    // makefile names are subject to include remaking.
    if makefile_paths.is_empty() && !eval_strings.is_empty() {
        for name in &["GNUmakefile", "makefile", "Makefile"] {
            engine
                .pending_includes
                .borrow_mut()
                .push((name.to_string(), true, String::new(), 0));
        }
    }

    let mut stdin_read = false;
    for path in &makefile_paths {
        if path == "-" {
            if stdin_read {
                eprintln!("make: *** Makefile from standard input specified twice.  Stop.");
                return 2;
            }
            stdin_read = true;
            use std::io::Read;
            let mut content = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut content) {
                eprintln!("make: stdin: {e}");
                return 2;
            }
            // GNU make writes stdin to a temp file in $TMPDIR (or /tmp).
            // Replicate the error when that directory is not writable.
            {
                let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
                let tmp_path = std::path::Path::new(&tmpdir).join("make-stdin-XXXXXX");
                if std::fs::write(&tmp_path, &content).is_err() {
                    eprintln!(
                        "make: *** cannot store makefile from stdin to a temporary file.  Stop."
                    );
                    return 2;
                }
                let _ = std::fs::remove_file(&tmp_path);
            }
            engine.load_string(&content);
        } else {
            engine.load_file(path, false);
        }
    }

    // After loading all makefiles, re-check MAKEFLAGS for -r / -R flags
    // that were set inside the makefile (e.g. `MAKEFLAGS += -r`). GNU
    // make honours these the same way as command-line flags.
    {
        let mflags_post = engine.lookup_var("MAKEFLAGS");
        // Split on whitespace; stop at "--" (separator before var
        // assignments). Each token that starts with "-" is a flag
        // cluster or long option; bare tokens (no leading dash) are
        // also short-flag clusters in MAKEFLAGS format.
        let mut has_r = false;
        let mut has_big_r = false;
        for tok in mflags_post.split_whitespace() {
            // Don't break at "--" — flags appended via
            // `MAKEFLAGS += -r` may appear after the separator.
            // Skip variable assignments (contain '=') — they are not flags.
            if tok.contains('=') {
                continue;
            }
            if tok == "--no-builtin-rules" {
                has_r = true;
                continue;
            }
            if tok == "--no-builtin-variables" {
                has_big_r = true;
                continue;
            }
            // Short-flag cluster: may or may not start with '-'
            let chars: &str = if let Some(rest) = tok.strip_prefix('-') {
                rest
            } else {
                tok
            };
            for ch in chars.chars() {
                match ch {
                    'r' => has_r = true,
                    'R' => {
                        has_r = true;
                        has_big_r = true;
                    }
                    _ => {}
                }
            }
        }
        if has_r {
            engine.disable_builtin_rules();
        }
        if has_big_r {
            engine.disable_builtin_vars();
        }
    }

    let rc = engine.build(&targets);

    // Sentinel -1 means an included makefile was remade and we need to
    // re-exec the process (GNU make "restart" semantics).
    if rc == -1 {
        // Increment MAKE_RESTARTS in the environment before re-exec.
        let restarts: u32 = std::env::var("MAKE_RESTARTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // Safety guard: prevent infinite re-exec loops.
        if restarts >= 10 {
            eprintln!(
                "make: *** Makefile re-exec loop detected (MAKE_RESTARTS={restarts}).  Stop."
            );
            return 2;
        }
        unsafe {
            std::env::set_var("MAKE_RESTARTS", (restarts + 1).to_string());
        }

        // Flush stdout/stderr before exec() replaces the process;
        // otherwise any buffered output (e.g. from $(info ...)) is lost.
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();

        // Re-exec the same binary with the same arguments.
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&args[0]).args(&args[1..]).exec();
        // exec() only returns on error. Format the message like GNU make,
        // which prints `make: <path>: <reason>` on EACCES / ENOENT.
        let msg = match err.kind() {
            std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
            std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
            _ => err.to_string(),
        };
        eprintln!("make: {}: {}", args[0], msg);
        return 127;
    }

    rc
}

/// Split a MAKEFLAGS string on whitespace, treating `\ ` (backslash-space)
/// as an escaped space that does NOT split tokens.
fn split_makeflags(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&' ') {
            chars.next(); // consume space
            current.push('\\');
            current.push(' ');
        } else if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Decode a MAKEFLAGS-encoded --eval value: `$$` -> `$`, `\ ` -> ` `.
fn decode_makeflags_eval(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'$') {
            chars.next();
            result.push('$');
        } else if c == '\\' && chars.peek() == Some(&' ') {
            chars.next();
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

/// Encode an --eval text for MAKEFLAGS variable value.
/// `$` -> `$$$$` (so after one make expansion it becomes `$$`),
/// ` ` -> `\ ` (not affected by make expansion).
fn encode_eval_for_makeflags(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '$' => result.push_str("$$$$"),
            ' ' => {
                result.push('\\');
                result.push(' ');
            }
            _ => result.push(c),
        }
    }
    result
}

/// Normalize a file path by stripping a leading `./` prefix.
fn normalize_path(s: &str) -> &str {
    s.strip_prefix("./").unwrap_or(s)
}
