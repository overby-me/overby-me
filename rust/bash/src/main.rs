mod ast;
mod builtins;
mod expand;
mod interpreter;
mod lexer;
mod parser;

use interpreter::Shell;
use std::io::{self, BufRead, Write};

fn main() {
    // Ignore SIGPIPE so builtins can handle write errors gracefully.
    // External commands reset SIGPIPE to SIG_DFL before exec.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let code = run();
    // Flush stdout before exiting - std::process::exit() doesn't flush
    std::io::stdout().flush().ok();
    std::io::stderr().flush().ok();
    std::process::exit(code);
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let mut shell = Shell::new();

    // Record the SIGPIPE ignore in the trap table
    #[cfg(unix)]
    {
        shell.traps.insert("PIPE".to_string(), String::new());
        // Check which signals were already ignored (SIG_IGN) at startup.
        // These cannot be trapped by the shell (POSIX requirement).
        let signals_to_check: &[(i32, &str)] = &[
            (libc::SIGHUP, "HUP"),
            (libc::SIGINT, "INT"),
            (libc::SIGQUIT, "QUIT"),
            (libc::SIGUSR1, "USR1"),
            (libc::SIGUSR2, "USR2"),
            (libc::SIGTERM, "TERM"),
            (libc::SIGCHLD, "CHLD"),
        ];
        for &(signum, name) in signals_to_check {
            unsafe {
                let prev = libc::signal(signum, libc::SIG_DFL);
                if prev == libc::SIG_IGN {
                    // Signal was ignored — restore and record
                    libc::signal(signum, libc::SIG_IGN);
                    shell.original_ignored_signals.insert(name.to_string());
                    // Also set the trap to empty (representing the inherited ignore)
                    shell.traps.insert(name.to_string(), String::new());
                } else {
                    // Restore original handler
                    libc::signal(signum, prev);
                }
            }
        }
    }

    // Detect if invoked as "sh" (posix mode, like bash's act_like_sh)
    if let Some(argv0) = args.first() {
        let base = argv0.rsplit('/').next().unwrap_or(argv0);
        if base == "sh" || base == "sh.exe" {
            shell.opt_posix = true;
            shell.shopt_expand_aliases = true;
            shell
                .shopt_options
                .insert("expand_aliases".to_string(), true);
        }
    }

    let mut command_string: Option<String> = None;
    let mut script_file: Option<String> = None;
    let mut force_interactive = false;
    let mut read_stdin = false;
    let mut positional_start = None;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("bash (rust-bash) {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            "-c" => {
                i += 1;
                if i < args.len() {
                    command_string = Some(args[i].clone());
                    // Remaining args become positional parameters
                    if i + 1 < args.len() {
                        positional_start = Some(i + 1);
                    }
                    break;
                } else {
                    eprintln!("bash: -c: option requires an argument");
                    return 2;
                }
            }
            "-i" => force_interactive = true,
            "-s" => read_stdin = true,
            "-l" | "--login" => {}         // Accepted but ignored
            "--norc" | "--noprofile" => {} // Accepted but ignored
            "--" => {
                i += 1;
                if i < args.len() && script_file.is_none() && command_string.is_none() {
                    script_file = Some(args[i].clone());
                    if i + 1 < args.len() {
                        positional_start = Some(i + 1);
                    }
                }
                break;
            }
            arg if arg.starts_with('-') || arg.starts_with('+') => {
                // Parse set-style options
                let enable = arg.starts_with('-');
                let flags = &arg[1..];
                if flags == "o" {
                    i += 1;
                    if i < args.len() {
                        match args[i].as_str() {
                            "pipefail" => shell.opt_pipefail = enable,
                            "errexit" => shell.opt_errexit = enable,
                            "nounset" => shell.opt_nounset = enable,
                            "xtrace" => shell.opt_xtrace = enable,
                            "posix" => {
                                shell.opt_posix = enable;
                                if enable {
                                    shell.shopt_expand_aliases = true;
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    let mut has_c = false;
                    for flag in flags.chars() {
                        match flag {
                            'c' => has_c = true,
                            'e' => shell.opt_errexit = enable,
                            'u' => shell.opt_nounset = enable,
                            'x' => shell.opt_xtrace = enable,
                            'f' => shell.opt_noglob = enable,
                            'C' => shell.opt_noclobber = enable,
                            'n' => shell.opt_noexec = enable,
                            _ => {}
                        }
                    }
                    if has_c {
                        i += 1;
                        if i < args.len() {
                            command_string = Some(args[i].clone());
                            if i + 1 < args.len() {
                                positional_start = Some(i + 1);
                            }
                            break;
                        }
                    }
                }
            }
            _ => {
                // First non-option argument is the script file
                script_file = Some(args[i].clone());
                if i + 1 < args.len() {
                    positional_start = Some(i + 1);
                }
                break;
            }
        }
        i += 1;
    }

    // Set positional parameters
    if let Some(start) = positional_start {
        if command_string.is_some() {
            // For -c: bash -c 'cmd' arg0 arg1 ... → $0=arg0, $1=arg1, ...
            shell.positional = vec![
                args.get(start)
                    .cloned()
                    .unwrap_or_else(|| "bash".to_string()),
            ];
            if start + 1 < args.len() {
                shell.positional.extend(args[start + 1..].to_vec());
            }
        } else {
            // For scripts: bash script.sh arg1 ... → $0=script.sh, $1=arg1, ...
            shell.positional = vec![
                args.get(start - 1)
                    .cloned()
                    .unwrap_or_else(|| "bash".to_string()),
            ];
            shell.positional.extend(args[start..].to_vec());
        }
    } else if let Some(ref file) = script_file {
        shell.positional = vec![file.clone()];
    }

    // Set script_name for error messages
    if let Some(ref file) = script_file {
        shell.script_name = file.clone();
    } else if command_string.is_some() {
        shell.script_name = shell
            .positional
            .first()
            .cloned()
            .unwrap_or_else(|| "bash".to_string());
    }
    // Store in a special internal variable for error reporting
    shell
        .vars
        .insert("_BASH_SOURCE_FILE".to_string(), shell.script_name.clone());
    crate::expand::set_script_name(&shell.script_name);

    // Set BASH_SOURCE array
    if !shell.script_name.is_empty() {
        shell
            .arrays
            .insert("BASH_SOURCE".to_string(), vec![shell.script_name.clone()]);
    }

    // Execute based on mode
    if let Some(cmd) = command_string {
        shell.dash_c_mode = true;
        shell.vars.insert("_BASH_C_STRING".to_string(), cmd.clone());
        let status = shell.run_string(&cmd);
        shell.run_exit_trap();
        return status;
    }

    if let Some(file) = script_file
        && !read_stdin
    {
        match std::fs::read_to_string(&file) {
            Ok(content) => {
                // Open script on fd 0 so exec 0< can redirect subsequent reading
                #[cfg(unix)]
                {
                    use std::os::unix::io::IntoRawFd;
                    if let Ok(f) = std::fs::File::open(&file) {
                        let raw = f.into_raw_fd();
                        let script_dup =
                            nix::fcntl::fcntl(raw, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(100)).ok();
                        nix::unistd::dup2(raw, 0).ok();
                        nix::unistd::close(raw).ok();
                        nix::unistd::lseek(0, content.len() as i64, nix::unistd::Whence::SeekSet)
                            .ok();
                        shell.script_fd = script_dup;
                    }
                }
                let status = shell.run_string(&content);
                #[cfg(unix)]
                if let Some(fd) = shell.script_fd.take() {
                    nix::unistd::close(fd).ok();
                }
                shell.run_exit_trap();
                return status;
            }
            Err(e) => {
                let argv0 = std::env::args()
                    .next()
                    .unwrap_or_else(|| "bash".to_string());
                let msg = match e.kind() {
                    std::io::ErrorKind::NotFound => "No such file or directory",
                    std::io::ErrorKind::PermissionDenied => "Permission denied",
                    _ => "No such file or directory",
                };
                eprintln!("{}: {}: {}", argv0, file, msg);
                return 127;
            }
        }
    }

    // Interactive mode or reading from stdin
    let is_tty = atty_is_tty();
    let interactive = force_interactive || (is_tty && command_string.is_none());

    if interactive {
        run_interactive(&mut shell);
        shell.last_status
    } else {
        // Read all of stdin and execute
        let mut input = String::new();
        io::stdin().read_to_string(&mut input).ok();
        let status = shell.run_string(&input);
        shell.run_exit_trap();
        status
    }
}

fn run_interactive(shell: &mut Shell) {
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        // Print prompt
        let ps1 = shell
            .vars
            .get("PS1")
            .cloned()
            .unwrap_or_else(|| "$ ".to_string());
        eprint!("{}", ps1);
        io::stderr().flush().ok();

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Err(_) => break,
            _ => {}
        }

        if line.trim().is_empty() {
            continue;
        }

        // Handle multi-line input (check for unclosed quotes, etc.)
        // For now, just execute line by line
        shell.run_string(&line);
    }
}

fn atty_is_tty() -> bool {
    #[cfg(unix)]
    {
        nix::unistd::isatty(0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

use std::io::Read;
