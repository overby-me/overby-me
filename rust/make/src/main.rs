mod ast;
mod engine;
mod expand;
mod parser;

use engine::Engine;

// === Async-signal-safe child PID tracking ===
//
// When `make` itself receives a fatal signal (SIGTERM, SIGINT, SIGHUP,
// SIGQUIT) we propagate it to the currently-running recipe child so the
// child's wait() returns with the signal in its status. The recipe error
// path then prints the standard `make: *** [...] Terminated` diagnostic
// and exits. Without this, a SIGTERM to make would silently kill us
// mid-recipe leaving the child (e.g. `sleep 10`) orphaned.
pub static CURRENT_CHILD_PID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[allow(non_camel_case_types)]
type c_int = i32;

unsafe extern "C" {
    fn signal(signum: c_int, handler: usize) -> usize;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    pub fn fork() -> c_int;
    pub fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    pub fn _exit(status: c_int) -> !;
    pub fn getpid() -> c_int;
}

const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGQUIT: c_int = 3;
const SIGTERM: c_int = 15;

extern "C" fn fatal_signal_handler(sig: c_int) {
    let pid = CURRENT_CHILD_PID.load(std::sync::atomic::Ordering::SeqCst);
    if pid > 0 {
        // Forward the signal to the child so its wait() returns with
        // status indicating death-by-signal. Our regular recipe error
        // path then prints the diagnostic.
        unsafe {
            kill(pid, sig);
        }
        // Don't exit here — let the recipe path handle the diagnostic.
    } else {
        // No child running: re-raise default handler by calling _exit.
        // 128 + signal number is the conventional shell exit status.
        std::process::exit(128 + sig);
    }
}

fn install_signal_handlers() {
    let h = fatal_signal_handler as usize;
    unsafe {
        signal(SIGTERM, h);
        signal(SIGINT, h);
        signal(SIGHUP, h);
        signal(SIGQUIT, h);
    }
}

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
    install_signal_handlers();
    // If a previous re-exec left a stdin temp file, record its path so
    // we can clean it up on exit. Detect by checking -f args for paths
    // matching the make-stdin-<pid> pattern in TMPDIR.
    let mut inherited_stdin_temp: Option<String> = None;
    let mut cmdline_set_jobs: bool = false;
    let mut cmdline_set_load: bool = false;

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

    // Detect inherited stdin temp files from a previous re-exec.
    // These have the pattern make-stdin-<PID> in the filename.
    // Check all -f / --file / --makefile arguments for this pattern.
    {
        let is_stdin_temp = |p: &str| -> bool {
            std::path::Path::new(p)
                .file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.starts_with("make-stdin-"))
        };
        // Scan all args (including those prepended from MAKEFLAGS).
        for i in 1..args.len() {
            let arg = &args[i];
            if let Some(path) = arg.strip_prefix("-f")
                && !path.is_empty()
                && is_stdin_temp(path)
            {
                inherited_stdin_temp = Some(path.to_string());
                break;
            }
            if (arg == "-f" || arg == "--makefile" || arg == "--file")
                && i + 1 < args.len()
                && is_stdin_temp(&args[i + 1])
            {
                inherited_stdin_temp = Some(args[i + 1].clone());
                break;
            }
            if let Some(path) = arg
                .strip_prefix("--makefile=")
                .or_else(|| arg.strip_prefix("--file="))
                && is_stdin_temp(path)
            {
                inherited_stdin_temp = Some(path.to_string());
                break;
            }
            // Handle combined flags like `-Rf<path>` or `-Rf <path>`.
            if arg.starts_with('-')
                && !arg.starts_with("--")
                && arg.len() > 2
                && let Some(pos) = arg.find('f')
            {
                let after_f = &arg[pos + 1..];
                if !after_f.is_empty() && is_stdin_temp(after_f) {
                    inherited_stdin_temp = Some(after_f.to_string());
                    break;
                } else if after_f.is_empty() && i + 1 < args.len() && is_stdin_temp(&args[i + 1]) {
                    inherited_stdin_temp = Some(args[i + 1].clone());
                    break;
                }
            }
            // Also check if the arg itself is a stdin temp path (could be
            // after a `-f` that was already consumed by option parsing).
            if is_stdin_temp(arg) {
                inherited_stdin_temp = Some(arg.clone());
                break;
            }
            if let Some(path) = arg.strip_prefix("--temp-stdin=") {
                inherited_stdin_temp = Some(path.to_string());
                break;
            }
        }
    }

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
    // Track env-inherited variable overrides (from MAKEFLAGS env).
    // These appear after cmdline overrides in MAKEFLAGS output.
    let mut env_overrides_list: Vec<String> = Vec::new();

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
                cmdline_set_jobs = true;
            }
            "-l" | "--load-average" | "--max-load" => {
                // Accept -l with a float argument. TODO: implement load limiting.
                i += 1;
                if i < args.len() {
                    if let Ok(v) = args[i].parse::<f64>() {
                        engine.load_limit = v;
                    }
                    mflags_long.push(format!("-l{}", args[i]));
                    cmdline_set_load = true;
                }
            }
            "-O" | "--output-sync" => {
                // Accept and ignore. May have an optional next argument
                // (line, target, none, recurse). Peek at next arg.
                if let Some(next) = args.get(i + 1) {
                    match next.as_str() {
                        "none" | "line" | "target" | "recurse" => {
                            mflags_long.push(format!("-O{next}"));
                            i += 1;
                        }
                        _ => {
                            mflags_long.push("-O".to_string());
                        }
                    }
                } else {
                    mflags_long.push("-O".to_string());
                }
            }
            arg if arg.starts_with("-O") && arg.len() > 2 => {
                // -Oline, -Otarget, etc.
                mflags_long.push(arg.to_string());
            }
            arg if arg.starts_with("--output-sync=") => {
                // --output-sync=target etc.
                let val = &arg["--output-sync=".len()..];
                mflags_long.push(format!("-O{val}"));
            }
            "-n" | "--just-print" | "--dry-run" | "--recon" => {
                engine.dry_run = true;
                if !mflags_short.contains('n') {
                    mflags_short.push('n');
                }
            }
            "-s" | "--silent" | "--quiet" => {
                engine.silent = true;
                if !mflags_short.contains('s') {
                    mflags_short.push('s');
                }
                mflags_long.retain(|s| s != "--no-silent");
            }
            "--no-silent" => {
                engine.silent = false;
                mflags_short.retain(|c| c != 's');
                mflags_long.push("--no-silent".to_string());
            }
            "-k" | "--keep-going" => {
                engine.keep_going = true;
                mflags_short.retain(|c| c != 'S');
                if !mflags_short.contains('k') {
                    mflags_short.push('k');
                }
            }
            "-S" | "--no-keep-going" | "--stop" => {
                engine.keep_going = false;
                mflags_short.retain(|c| c != 'k');
                if !mflags_short.contains('S') {
                    mflags_short.push('S');
                }
            }
            "-t" | "--touch" => {
                engine.touch = true;
                if !mflags_short.contains('t') {
                    mflags_short.push('t');
                }
            }
            "-q" | "--question" => {
                engine.question = true;
                if !mflags_short.contains('q') {
                    mflags_short.push('q');
                }
            }
            "-B" | "--always-make" => {
                engine.always_make.set(true);
                if !mflags_short.contains('B') {
                    mflags_short.push('B');
                }
            }
            "-i" | "--ignore-errors" => {
                engine.ignore_errors = true;
                if !mflags_short.contains('i') {
                    mflags_short.push('i');
                }
            }
            "-e" | "--environment-overrides" => {
                engine.env_overrides = true;
                if !mflags_short.contains('e') {
                    mflags_short.push('e');
                }
            }
            "-r" | "--no-builtin-rules" => {
                engine.disable_builtin_rules();
                if !mflags_short.contains('r') {
                    mflags_short.push('r');
                }
            }
            "-R" | "--no-builtin-variables" => {
                engine.disable_builtin_rules();
                engine.disable_builtin_vars();
                if !mflags_short.contains('R') {
                    mflags_short.push('R');
                }
            }
            "-w" | "--print-directory" => {
                engine.print_directory_opt = Some(true);
                mflags_long.retain(|s| s != "--no-print-directory");
                if !mflags_short.contains('w') {
                    mflags_short.push('w');
                }
            }
            "--no-print-directory" => {
                engine.print_directory_opt = Some(false);
                mflags_short.retain(|c| c != 'w');
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
            "-d" | "--debug" | "--debug=a" => {
                debug_mode = true;
                engine.set_debug_flag(b'a');
                if !mflags_short.contains('d') {
                    mflags_short.push('d');
                }
            }
            "--debug=b" | "--debug=basic" => {
                debug_mode = true;
                engine.set_debug_flag(b'b');
                mflags_long.push("--debug=b".to_string());
            }
            "--debug=v" | "--debug=verbose" => {
                debug_mode = true;
                engine.set_debug_flag(b'v');
                mflags_long.push("--debug=v".to_string());
            }
            "--debug=i" | "--debug=implicit" => {
                debug_mode = true;
                engine.set_debug_flag(b'i');
                mflags_long.push("--debug=i".to_string());
            }
            "--debug=j" | "--debug=jobs" => {
                debug_mode = true;
                engine.set_debug_flag(b'j');
                mflags_long.push("--debug=j".to_string());
            }
            "--debug=m" | "--debug=makefile" => {
                debug_mode = true;
                engine.set_debug_flag(b'm');
                mflags_long.push("--debug=m".to_string());
            }
            "--debug=n" | "--debug=none" => {
                debug_mode = true;
                mflags_long.push("--debug=n".to_string());
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
                Ok(n) => {
                    engine.jobs = n;
                    cmdline_set_jobs = true;
                }
                Err(_) => {
                    eprintln!("make: invalid integer argument '{}' for '-j'", &arg[2..]);
                    return 2;
                }
            },
            arg if arg.starts_with("-l") && arg.len() > 2 => {
                // -l0.0001 form
                if let Ok(v) = arg[2..].parse::<f64>() {
                    engine.load_limit = v;
                    cmdline_set_load = true;
                }
                mflags_long.push(arg.to_string());
            }
            arg if arg.starts_with("--load-average=") => {
                let val = &arg["--load-average=".len()..];
                mflags_long.push(format!("-l{val}"));
            }
            arg if arg.starts_with("--max-load=") => {
                let val = &arg["--max-load=".len()..];
                mflags_long.push(format!("-l{val}"));
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
                    } else {
                        let escaped = combined.replace('$', "$$");
                        env_overrides_list.push(format!("{name}={escaped}"));
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
                } else {
                    let escaped = value.replace('$', "$$");
                    env_overrides_list.push(format!("{name}{orig_sep}{escaped}"));
                }
            }
            arg if arg.starts_with("--temp-stdin=") => {
                // Recognized but no-op: stdin contents are passed via -f<path>;
                // this flag exists for the re-exec banner under --debug=b and
                // for cleanup of the temp file on exit.
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
                        'n' => {
                            engine.dry_run = true;
                            if !mflags_short.contains('n') {
                                mflags_short.push('n');
                            }
                        }
                        's' => {
                            engine.silent = true;
                            if !mflags_short.contains('s') {
                                mflags_short.push('s');
                            }
                            mflags_long.retain(|s| s != "--no-silent");
                        }
                        'k' => {
                            engine.keep_going = true;
                            mflags_short.retain(|c| c != 'S');
                            if !mflags_short.contains('k') {
                                mflags_short.push('k');
                            }
                        }
                        'S' => {
                            engine.keep_going = false;
                            mflags_short.retain(|c| c != 'k');
                            if !mflags_short.contains('S') {
                                mflags_short.push('S');
                            }
                        }
                        't' => {
                            engine.touch = true;
                            if !mflags_short.contains('t') {
                                mflags_short.push('t');
                            }
                        }
                        'q' => {
                            engine.question = true;
                            if !mflags_short.contains('q') {
                                mflags_short.push('q');
                            }
                        }
                        'B' => {
                            engine.always_make.set(true);
                            if !mflags_short.contains('B') {
                                mflags_short.push('B');
                            }
                        }
                        'i' => {
                            engine.ignore_errors = true;
                            if !mflags_short.contains('i') {
                                mflags_short.push('i');
                            }
                        }
                        'e' => {
                            engine.env_overrides = true;
                            if !mflags_short.contains('e') {
                                mflags_short.push('e');
                            }
                        }
                        'w' => {
                            engine.print_directory_opt = Some(true);
                            mflags_long.retain(|s| s != "--no-print-directory");
                            if !mflags_short.contains('w') {
                                mflags_short.push('w');
                            }
                        }
                        'r' => {
                            engine.disable_builtin_rules();
                            if !mflags_short.contains('r') {
                                mflags_short.push('r');
                            }
                        }
                        'R' => {
                            engine.disable_builtin_rules();
                            engine.disable_builtin_vars();
                            if !mflags_short.contains('R') {
                                mflags_short.push('R');
                            }
                        }
                        'd' => {
                            debug_mode = true;
                            engine.set_debug_flag(b'a');
                            if !mflags_short.contains('d') {
                                mflags_short.push('d');
                            }
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
                            if let Some(v) = take_arg(&flags, idx, &mut i) {
                                mflags_long.push(format!("-l{v}"));
                            }
                            break;
                        }
                        'O' => {
                            // -O takes an optional argument (rest of cluster)
                            if let Some(v) = take_arg(&flags, idx, &mut i) {
                                mflags_long.push(format!("-O{v}"));
                            } else {
                                mflags_long.push("-O".to_string());
                            }
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
    // GNU make orders `:=` / `::=` (simple) assignments before `=` (recursive)
    // assignments in MAKEOVERRIDES, preserving relative order within each group.
    // Combine: cmdline overrides first, then env-inherited overrides.
    // Deduplicate: if a variable name appears in both cmdline and env,
    // cmdline wins (don't include the env-inherited one).
    makeoverrides.sort_by_key(|s| if s.contains(":=") { 0 } else { 1 });
    env_overrides_list.sort_by_key(|s| if s.contains(":=") { 0 } else { 1 });
    let cmdline_var_names: std::collections::HashSet<String> = makeoverrides
        .iter()
        .filter_map(|s| {
            s.find(":=")
                .or_else(|| s.find('='))
                .map(|idx| s[..idx].to_string())
        })
        .collect();
    let mut all_overrides = makeoverrides.clone();
    for ov in &env_overrides_list {
        let var_name = ov.find(":=").or_else(|| ov.find('=')).map(|idx| &ov[..idx]);
        if let Some(name) = var_name {
            if !cmdline_var_names.contains(name) {
                all_overrides.push(ov.clone());
            }
        } else {
            all_overrides.push(ov.clone());
        }
    }
    engine.set_var_with_origin(
        "MAKEOVERRIDES",
        &all_overrides.join(" "),
        engine::VarFlavor::Recursive,
        engine::VarOrigin::Default,
    );

    // Build MAKEFLAGS: short flags concatenated (no leading '-'), then
    // long flags as separate `--opt` tokens. GNU make includes
    // $(MAKEOVERRIDES) after a `--` separator so sub-makes inherit
    // command-line variable assignments. MAKEFLAGS is recursive so
    // changes to MAKEOVERRIDES (even from inside a makefile) are
    // reflected when MAKEFLAGS is exported to child processes.
    // Sort short flags in GNU make's canonical order: case-insensitive,
    // lowercase before uppercase for the same letter (e.g. r before R, s before S).
    let mut mflags_chars: Vec<char> = mflags_short.chars().collect();
    mflags_chars.sort_by(|a, b| {
        let la = a.to_ascii_lowercase();
        let lb = b.to_ascii_lowercase();
        la.cmp(&lb).then(b.cmp(a))
    });
    let mflags_short_sorted: String = mflags_chars.into_iter().collect();
    let mut mflags = mflags_short_sorted.clone();
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
    // When there were originally overrides (cmdline or env), always
    // include the " -- " separator even if MAKEOVERRIDES becomes empty
    // later (e.g. via `MAKEOVERRIDES=` in the makefile). GNU make
    // keeps the separator once overrides existed.
    if !all_overrides.is_empty() {
        mflags.push_str(" -- $(MAKEOVERRIDES)");
    } else {
        mflags.push_str("$(if $(MAKEOVERRIDES), -- $(MAKEOVERRIDES))");
    }
    // Use CommandLine origin when cmdline flags or overrides are present
    // so that file-level `MAKEFLAGS +=` is blocked (matching GNU make).
    // The MAKEFLAGS merge in set_var_with_origin still intercepts plain
    // `MAKEFLAGS=X` assignments from makefiles.
    let mflags_origin =
        if !mflags_short.is_empty() || !mflags_long.is_empty() || !makeoverrides.is_empty() {
            engine::VarOrigin::CommandLine
        } else {
            engine::VarOrigin::Default
        };
    engine.set_var_with_origin(
        "MAKEFLAGS",
        &mflags,
        engine::VarFlavor::Recursive,
        mflags_origin,
    );
    // Store command-line flags so MAKEFLAGS merge logic in set_var_with_origin
    // can preserve them when a makefile assigns to MAKEFLAGS.
    engine.cmdline_mflags.borrow_mut().clone_from(&mflags_short);
    *engine.cmdline_mflags_long.borrow_mut() = mflags_long.clone();

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
            engine
                .include_mentioned
                .borrow_mut()
                .insert(path.to_string());
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
            engine
                .include_mentioned
                .borrow_mut()
                .insert(name.to_string());
        }
    }

    let mut stdin_read = false;
    // Saved stdin content for re-exec (written to temp file only if needed).
    let mut stdin_content_for_reexec: Option<String> = None;
    // Primary `-f` makefiles that failed to load (treated as goals on the
    // main build; their failure also suppresses re-exec).
    let mut failed_primary_makefiles: Vec<String> = Vec::new();
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
            // Verify we CAN write there (error if not), but don't persist
            // the file yet — only write it on re-exec when actually needed.
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
            stdin_content_for_reexec = Some(content.clone());
            engine.load_string(&content);
        } else {
            let loaded = engine.load_file_with_loc(path, false, None);
            if loaded {
                // Register primary makefile for auto-rebuild checking in
                // finalize_includes (GNU make rebuilds the primary makefile
                // too, not just included files).
                engine.included_files.borrow_mut().push(path.clone());
            } else {
                // Primary makefile couldn't be loaded. Register it so
                // finalize_includes attempts to rebuild it; if no rule
                // ever produces it, treat its name as a goal so the
                // main build phase emits "No rule to make target 'X'".
                engine.included_files.borrow_mut().push(path.clone());
                failed_primary_makefiles.push(path.clone());
            }
        }
    }
    // Suppress re-exec when any primary makefile failed to load AND
    // wasn't rebuilt — matches GNU make, which doesn't restart in that
    // case (the missing makefile is reported as a goal-failure instead).
    let had_failed_primary = !failed_primary_makefiles.is_empty();

    // After loading all makefiles, re-check MAKEFLAGS for flags
    // that were set inside the makefile (e.g. `MAKEFLAGS += -r`). GNU
    // make honours these the same way as command-line flags.
    {
        let mflags_post = engine.lookup_var("MAKEFLAGS");
        let mut has_r = false;
        let mut has_big_r = false;
        let mut has_s = false;
        let mut has_k = false;
        let mut has_i = false;
        let mut has_n = false;
        let mut has_e = false;
        let mut has_w = false;
        let mut has_no_print_dir = false;
        let mut has_no_silent = false;
        let mut has_trace = false;
        let mut past_separator = false;
        for tok in mflags_post.split_whitespace() {
            if tok == "--" {
                past_separator = true;
                continue;
            }
            if past_separator {
                continue;
            }
            // Variable assignments contain '=', but options like --debug=b also do.
            // Skip plain assignments (NAME=val) but keep --opt=val.
            if tok.contains('=') && !tok.starts_with("--") {
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
            if tok == "--no-print-directory" {
                has_no_print_dir = true;
                continue;
            }
            if tok == "--no-silent" {
                has_no_silent = true;
                continue;
            }
            if tok == "--trace" {
                has_trace = true;
                continue;
            }
            if let Some(arg) = tok.strip_prefix("--debug=") {
                for c in arg.chars() {
                    match c {
                        'a' => engine.set_debug_flag(b'a'),
                        'b' => engine.set_debug_flag(b'b'),
                        'v' => engine.set_debug_flag(b'v'),
                        'i' => engine.set_debug_flag(b'i'),
                        'j' => engine.set_debug_flag(b'j'),
                        'm' => engine.set_debug_flag(b'm'),
                        'n' => engine.set_debug_flag(b'n'),
                        _ => {}
                    }
                }
                if matches!(
                    arg,
                    "basic" | "verbose" | "implicit" | "jobs" | "makefile" | "all" | "none"
                ) {
                    let f = match arg {
                        "basic" => b'b',
                        "verbose" => b'v',
                        "implicit" => b'i',
                        "jobs" => b'j',
                        "makefile" => b'm',
                        "all" => b'a',
                        _ => b'n',
                    };
                    engine.set_debug_flag(f);
                }
                continue;
            }
            if tok == "--debug" {
                engine.set_debug_flag(b'a');
                continue;
            }
            // Post-load -jN / --jobs=N: update engine.jobs unless cmdline set it.
            if let Some(n_str) = tok.strip_prefix("-j") {
                if !cmdline_set_jobs {
                    if n_str.is_empty() {
                        engine.jobs = 0; // unlimited
                    } else if let Ok(n) = n_str.parse::<usize>() {
                        engine.jobs = n;
                    }
                }
                continue;
            }
            if let Some(n_str) = tok.strip_prefix("--jobs=") {
                if !cmdline_set_jobs && let Ok(n) = n_str.parse::<usize>() {
                    engine.jobs = n;
                }
                continue;
            }
            // Post-load -lN / --load-average=N / --max-load=N: update engine.load_limit unless cmdline set it.
            if let Some(n_str) = tok.strip_prefix("-l") {
                if !cmdline_set_load
                    && !n_str.is_empty()
                    && let Ok(v) = n_str.parse::<f64>()
                {
                    engine.load_limit = v;
                }
                continue;
            }
            if let Some(n_str) = tok
                .strip_prefix("--load-average=")
                .or_else(|| tok.strip_prefix("--max-load="))
            {
                if !cmdline_set_load && let Ok(v) = n_str.parse::<f64>() {
                    engine.load_limit = v;
                }
                continue;
            }
            // Short-flag cluster: may or may not start with '-'.
            // Skip long options (--foo) — only single-dash or bare tokens.
            if tok.starts_with("--") {
                continue;
            }
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
                    's' => has_s = true,
                    'k' => has_k = true,
                    'i' => has_i = true,
                    'n' => has_n = true,
                    'e' => has_e = true,
                    'w' => has_w = true,
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
        if has_s && !has_no_silent {
            engine.silent = true;
        }
        if has_no_silent {
            engine.silent = false;
        }
        if has_k {
            engine.keep_going = true;
        }
        if has_i {
            engine.ignore_errors = true;
        }
        if has_n {
            engine.dry_run = true;
        }
        if has_e {
            engine.env_overrides = true;
        }
        if has_trace {
            engine.trace = true;
        }
        // -w / --no-print-directory: only override if not already set by cmdline
        if has_no_print_dir {
            engine.print_directory_opt = Some(false);
        } else if has_w && engine.print_directory_opt.is_none() {
            engine.print_directory_opt = Some(true);
        }

        // -R (no-builtin-variables) implies -r (no-builtin-rules) in GNU make.
        // Check the raw MAKEFLAGS value for whether 'r' is already present;
        // if not, insert it before 'R'. We check the raw value because the
        // scanner above sets has_r=true whenever it sees 'R'.
        if has_big_r {
            let vars = engine.vars.borrow();
            let raw = vars.get("MAKEFLAGS").map(|v| (v.value.clone(), v.origin));
            drop(vars);
            if let Some((raw_val, origin)) = raw {
                // Check if 'r' is already in the short-flag cluster (before
                // any space, '$', or '-'). Only look at leading flag chars.
                let flags_end = raw_val.find([' ', '$', '-']).unwrap_or(raw_val.len());
                let flag_cluster = &raw_val[..flags_end];
                if !flag_cluster.contains('r')
                    && let Some(pos) = raw_val.find('R')
                {
                    let mut new_val = raw_val[..pos].to_string();
                    new_val.push('r');
                    new_val.push_str(&raw_val[pos..]);
                    engine.vars.borrow_mut().insert(
                        "MAKEFLAGS".to_string(),
                        engine::Variable {
                            value: new_val,
                            flavor: engine::VarFlavor::Recursive,
                            origin,
                        },
                    );
                }
            }
        }

        // GNU make reconstructs MAKEFLAGS before running recipes.
        // If MAKEOVERRIDES is now empty, switch from the unconditional
        // " -- $(MAKEOVERRIDES)" suffix to the conditional form so the
        // " -- " separator disappears when there are no overrides.
        {
            let mo = engine.lookup_var("MAKEOVERRIDES");
            if mo.trim().is_empty() {
                let vars = engine.vars.borrow();
                let raw = vars.get("MAKEFLAGS").map(|v| (v.value.clone(), v.origin));
                drop(vars);
                if let Some((raw_val, origin)) = raw
                    && let Some(pos) = raw_val.find(" -- $(MAKEOVERRIDES)")
                {
                    let mut new_val = raw_val[..pos].to_string();
                    new_val.push_str("$(if $(MAKEOVERRIDES), -- $(MAKEOVERRIDES))");
                    new_val.push_str(&raw_val[pos + " -- $(MAKEOVERRIDES)".len()..]);
                    engine.vars.borrow_mut().insert(
                        "MAKEFLAGS".to_string(),
                        engine::Variable {
                            value: new_val,
                            flavor: engine::VarFlavor::Recursive,
                            origin,
                        },
                    );
                }
            }
        }
    }

    // GNU make adds unloadable primary `-f` makefiles to the goal list
    // so the main build phase reports "No rule to make target 'X'".
    let mut effective_targets = targets.clone();
    for fp in &failed_primary_makefiles {
        if !effective_targets.contains(fp) {
            effective_targets.insert(0, fp.clone());
        }
    }
    // Suppress re-exec when any primary `-f` makefile failed to load:
    // GNU make does not restart in that case, even if other includes
    // were rebuilt.
    if had_failed_primary {
        engine.suppress_reexec.set(true);
    }

    let rc = engine.build(&effective_targets);

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

        // Re-exec the same binary with the same arguments. If stdin was
        // read (-f-), write the content to a temp file and replace that
        // arg with the temp file path so the re-exec'd process can read it.
        let mut reexec_args: Vec<String> = args[1..].to_vec();
        let mut just_created_stdin_temp: Option<String> = None;
        if let Some(ref content) = stdin_content_for_reexec {
            let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
            let temp_path =
                std::path::Path::new(&tmpdir).join(format!("make-stdin-{}", std::process::id()));
            if let Err(e) = std::fs::write(&temp_path, content) {
                eprintln!("make: *** cannot write stdin temp file for re-exec: {e}.  Stop.");
                return 2;
            }
            let temp_str = temp_path.to_string_lossy().to_string();
            just_created_stdin_temp = Some(temp_str.clone());
            // Also append --temp-stdin=<path> to mirror GNU make 4.4+ semantics.
            // (The -f<path> arg is what actually loads the makefile; --temp-stdin
            // exists primarily for the re-exec banner under --debug=b.)
            reexec_args.push(format!("--temp-stdin={temp_str}"));
            // Replace `-f-`, `-f -`, `--makefile -`, `--makefile=-` with
            // the temp file path.
            let mut i = 0;
            while i < reexec_args.len() {
                if reexec_args[i] == "-f-" {
                    reexec_args[i] = format!("-f{temp_str}");
                } else if reexec_args[i] == "--makefile=-" || reexec_args[i] == "--file=-" {
                    reexec_args[i] = format!("--file={temp_str}");
                } else if (reexec_args[i] == "-f"
                    || reexec_args[i] == "--makefile"
                    || reexec_args[i] == "--file")
                    && i + 1 < reexec_args.len()
                    && reexec_args[i + 1] == "-"
                {
                    reexec_args[i + 1] = temp_str.clone();
                } else if reexec_args[i].starts_with('-')
                    && !reexec_args[i].starts_with("--")
                    && reexec_args[i].len() > 2
                {
                    // Handle combined short flags like `-Rf-`, `-Rf -`.
                    // If the arg contains `f-` at the end, it's `-<flags>f-`.
                    // If it ends with `f`, the next arg is the filename.
                    let arg = &reexec_args[i];
                    if arg.ends_with("f-") {
                        // e.g. `-Rf-` → `-Rf<temp_path>`
                        let prefix = &arg[..arg.len() - 1]; // strip trailing `-`
                        reexec_args[i] = format!("{prefix}{temp_str}");
                    } else if arg.ends_with('f')
                        && i + 1 < reexec_args.len()
                        && reexec_args[i + 1] == "-"
                    {
                        // e.g. `-Rf -` → `-Rf <temp_path>`
                        reexec_args[i + 1] = temp_str.clone();
                    }
                }
                i += 1;
            }
        }
        if engine.debug_basic() {
            let mut buf = String::from(&args[0][..]);
            for a in &reexec_args {
                buf.push(' ');
                buf.push_str(a);
            }
            eprintln!("Re-executing[{}]: {}", restarts + 1, buf);
        }
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&args[0])
            .args(&reexec_args)
            .exec();
        // exec() only returns on error. Format the message like GNU make,
        // which prints `make: <path>: <reason>` on EACCES / ENOENT.
        // exec failed — clean up the stdin temp file we just created.
        if let Some(ref tmp) = just_created_stdin_temp {
            let _ = std::fs::remove_file(tmp);
        }
        let msg = match err.kind() {
            std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
            std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
            _ => err.to_string(),
        };
        eprintln!("make: {}: {}", args[0], msg);
        return 127;
    }

    // Clean up stdin temp file from a previous re-exec (or from a
    // re-exec we just triggered — exec() replaces the process so this
    // line is only reached on normal exit, not re-exec).
    if let Some(ref temp_path) = inherited_stdin_temp {
        let _ = std::fs::remove_file(temp_path);
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
