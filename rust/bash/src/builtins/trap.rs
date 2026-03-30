use super::*;

pub(super) fn builtin_trap(shell: &mut Shell, args: &[String]) -> i32 {
    fn signal_number(s: &str) -> i32 {
        let upper = s.to_uppercase();
        let name = upper.strip_prefix("SIG").unwrap_or(&upper);
        match name {
            "EXIT" | "0" => 0,
            "HUP" | "1" => 1,
            "INT" | "2" => 2,
            "QUIT" | "3" => 3,
            "ILL" | "4" => 4,
            "TRAP" | "5" => 5,
            "ABRT" | "6" => 6,
            "BUS" | "7" => 7,
            "FPE" | "8" => 8,
            "KILL" | "9" => 9,
            "USR1" | "10" => 10,
            "SEGV" | "11" => 11,
            "USR2" | "12" => 12,
            "PIPE" | "13" => 13,
            "ALRM" | "14" => 14,
            "TERM" | "15" => 15,
            "CHLD" | "17" => 17,
            "CONT" | "18" => 18,
            "STOP" | "19" => 19,
            "TSTP" | "20" => 20,
            "DEBUG" => 100,
            "ERR" => 101,
            "RETURN" => 102,
            _ => 999,
        }
    }

    // Normalize signal name for display
    fn normalize_signal_name(s: &str) -> String {
        match s {
            "0" | "EXIT" | "exit" => "EXIT".to_string(),
            "ERR" | "err" => "ERR".to_string(),
            "DEBUG" | "debug" => "DEBUG".to_string(),
            "RETURN" | "return" => "RETURN".to_string(),
            _ => {
                let upper = s.to_uppercase();
                let name = upper.strip_prefix("SIG").unwrap_or(&upper);
                format!("SIG{}", name)
            }
        }
    }

    if args.is_empty() {
        // Print current traps in signal number order
        let mut sorted: Vec<_> = shell.traps.iter().collect();
        sorted.sort_by_key(|(sig, _)| signal_number(sig));
        for (signal, handler) in sorted {
            println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
        }
        return 0;
    }

    // Check for invalid options
    for arg in args {
        if arg.starts_with('-')
            && arg.len() > 1
            && !matches!(arg.as_str(), "-p" | "-P" | "-l" | "-L" | "--")
        {
            eprintln!("{}: trap: {}: invalid option", shell.error_prefix(), arg);
            eprintln!("trap: usage: trap [-Plp] [[action] signal_spec ...]");
            return 2;
        }
        if arg == "--" {
            break;
        }
    }

    if args.len() == 1 {
        // trap '' or trap - : list traps or reset
        if args[0] == "-l" || args[0] == "-L" {
            // List signal names (same format as kill -l)
            let signals = list_all_signals();
            for (i, (num, name)) in signals.iter().enumerate() {
                print!("{:2}) {}", num, name);
                if (i + 1) % 5 == 0 || i == signals.len() - 1 {
                    println!();
                } else {
                    print!("\t");
                }
            }
            return 0;
        }
        if args[0] == "-p" {
            let mut sorted: Vec<_> = shell.traps.iter().collect();
            sorted.sort_by_key(|(sig, _)| signal_number(sig));
            for (signal, handler) in sorted {
                println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
            }
            return 0;
        }
    }

    // Handle -p with signal arguments: trap -p SIG1 SIG2 ...
    if args.first().map(|s| s.as_str()) == Some("-p") && args.len() >= 2 {
        // Check for conflicting -P
        if args.iter().any(|a| a == "-P") {
            eprintln!(
                "{}: trap: cannot specify both -p and -P",
                shell.error_prefix()
            );
            return 2;
        }
        let mut status = 0;
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            // Validate signal name
            if signal_number(&norm) == 999 {
                eprintln!(
                    "{}: trap: {}: invalid signal specification",
                    shell.error_prefix(),
                    sig_arg
                );
                status = 1;
                continue;
            }
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("trap -- '{}' {}", handler, norm);
            }
        }
        return status;
    }

    // trap [-p|-P] 'handler' signal [signal...]
    let handler_idx = 0;
    let sig_start = 1;

    // Check for conflicting -p and -P
    if args.contains(&"-p".to_string()) && args.contains(&"-P".to_string()) {
        eprintln!(
            "{}: trap: cannot specify both -p and -P",
            shell.error_prefix()
        );
        return 2;
    }

    // Handle -P flag — print just the handler command for specified signals
    if args.first().map(|s| s.as_str()) == Some("-P") {
        if args.len() < 2 {
            eprintln!(
                "{}: trap: -P requires at least one signal name",
                shell.error_prefix()
            );
            return 1;
        }
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("{}", handler);
            }
        }
        return 0;
    }

    // Handle -p flag — print traps for specified signals
    if args.first().map(|s| s.as_str()) == Some("-p") {
        if args.len() < 2 {
            let mut sorted: Vec<_> = shell.traps.iter().collect();
            sorted.sort_by_key(|(sig, _)| signal_number(sig));
            for (signal, handler) in sorted {
                println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
            }
            return 0;
        }
        // trap -p SIG1 SIG2 ... — print traps for specific signals
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            // Traps are stored without SIG prefix, so strip it for lookup
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("trap -- '{}' {}", handler, norm);
            }
        }
        return 0;
    }

    if args.len() < sig_start + 1 {
        // Handler with no signals specified
        if handler_idx == 0 && args.len() == 1 {
            // Single arg that looks like a signal name → reset it
            let norm = normalize_signal_name(&args[0]);
            if signal_number(&norm) != 999 {
                // It's a signal name — reset it
                let signal = norm.strip_prefix("SIG").unwrap_or(&norm).to_string();
                if !shell.original_ignored_signals.contains(&signal) {
                    shell.traps.remove(&signal);
                    // Also try the original arg form (e.g., "0" stored as "0" not "EXIT")
                    let raw_signal = args[0]
                        .to_uppercase()
                        .strip_prefix("SIG")
                        .unwrap_or(&args[0].to_uppercase())
                        .to_string();
                    shell.traps.remove(&raw_signal);
                }
                return 0;
            }
            // Not a signal name — error (e.g., trap "" with no signal)
            eprintln!("trap: usage: trap [-Plp] [[action] signal_spec ...]");
            return 2;
        }
    }

    let handler = &args[handler_idx];

    let mut status = 0;
    for sig in &args[sig_start..] {
        let signal = sig.to_uppercase();
        let signal = signal.strip_prefix("SIG").unwrap_or(&signal).to_string();

        // Validate signal name/number
        let valid = matches!(
            signal.as_str(),
            "EXIT"
                | "0"
                | "HUP"
                | "INT"
                | "QUIT"
                | "ILL"
                | "TRAP"
                | "ABRT"
                | "BUS"
                | "FPE"
                | "KILL"
                | "USR1"
                | "SEGV"
                | "USR2"
                | "PIPE"
                | "ALRM"
                | "TERM"
                | "STKFLT"
                | "CHLD"
                | "CONT"
                | "STOP"
                | "TSTP"
                | "TTIN"
                | "TTOU"
                | "URG"
                | "XCPU"
                | "XFSZ"
                | "VTALRM"
                | "PROF"
                | "WINCH"
                | "IO"
                | "PWR"
                | "SYS"
                | "DEBUG"
                | "ERR"
                | "RETURN"
        ) || signal.parse::<u32>().is_ok_and(|n| n <= 64);

        if !valid {
            eprintln!(
                "{}: trap: {}: invalid signal specification",
                shell.error_prefix(),
                sig
            );
            status = 1;
            continue;
        }

        // Signals that were ignored at startup cannot be trapped or reset
        if shell.original_ignored_signals.contains(&signal) {
            // Silently ignore the request (bash behavior)
            continue;
        }
        if handler == "-" {
            // Reset trap to default
            shell.traps.remove(&signal);
            // Reset Unix signal disposition to default
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                unsafe {
                    libc::signal(signum, libc::SIG_DFL);
                }
            }
        } else if handler.is_empty() {
            // Empty handler = ignore signal
            shell.traps.insert(signal.clone(), handler.clone());
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                unsafe {
                    libc::signal(signum, libc::SIG_IGN);
                }
            }
        } else {
            // Set trap handler and install signal catcher
            shell.traps.insert(signal.clone(), handler.clone());
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                crate::interpreter::install_signal_handler(signum);
            }
        }
    }
    status
}

pub(super) fn signal_name_to_number(name: &str) -> Option<i32> {
    match name {
        "HUP" => Some(libc::SIGHUP),
        "INT" => Some(libc::SIGINT),
        "QUIT" => Some(libc::SIGQUIT),
        "ILL" => Some(libc::SIGILL),
        "TRAP" => Some(libc::SIGTRAP),
        "ABRT" => Some(libc::SIGABRT),
        "BUS" => Some(libc::SIGBUS),
        "FPE" => Some(libc::SIGFPE),
        "KILL" => Some(libc::SIGKILL),
        "USR1" => Some(libc::SIGUSR1),
        "SEGV" => Some(libc::SIGSEGV),
        "USR2" => Some(libc::SIGUSR2),
        "PIPE" => Some(libc::SIGPIPE),
        "ALRM" => Some(libc::SIGALRM),
        "TERM" => Some(libc::SIGTERM),
        "CHLD" => Some(libc::SIGCHLD),
        "CONT" => Some(libc::SIGCONT),
        "STOP" => Some(libc::SIGSTOP),
        "TSTP" => Some(libc::SIGTSTP),
        _ => None,
    }
}

pub(super) fn builtin_wait(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::wait::{WaitStatus, waitpid};
        use nix::unistd::Pid;

        // Handle -n flag (wait for any single job)
        if args.first().map(|s| s.as_str()) == Some("-n") {
            match waitpid(Pid::from_raw(-1), None) {
                Ok(WaitStatus::Exited(_, code)) => {
                    shell.last_status = code;
                    return code;
                }
                Ok(WaitStatus::Signaled(_, sig, _)) => {
                    let code = 128 + sig as i32;
                    shell.last_status = code;
                    return code;
                }
                _ => return shell.last_status,
            }
        }

        if args.is_empty() {
            // Wait for all background children
            // Use blocking wait with SIGCHLD trap support
            let has_chld_trap = shell.traps.contains_key("CHLD");
            loop {
                match waitpid(Pid::from_raw(-1), None) {
                    Ok(WaitStatus::Exited(_, code)) => {
                        shell.last_status = code;
                        if has_chld_trap {
                            // Clear any pending SIGCHLD from the signal handler
                            // to avoid double-firing
                            crate::interpreter::take_pending_signal(libc::SIGCHLD);
                            if let Some(handler) = shell.traps.get("CHLD").cloned()
                                && !handler.is_empty()
                            {
                                shell.in_trap_handler += 1;
                                shell.run_string(&handler);
                                shell.in_trap_handler -= 1;
                            }
                        }
                    }
                    Ok(WaitStatus::Signaled(_, sig, _)) => {
                        shell.last_status = 128 + sig as i32;
                        if has_chld_trap {
                            crate::interpreter::take_pending_signal(libc::SIGCHLD);
                            if let Some(handler) = shell.traps.get("CHLD").cloned()
                                && !handler.is_empty()
                            {
                                shell.in_trap_handler += 1;
                                shell.run_string(&handler);
                                shell.in_trap_handler -= 1;
                            }
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            // wait with no arguments returns 0 per POSIX
            return 0;
        } else {
            // Wait for specific PIDs
            for arg in args {
                if let Ok(pid) = arg.parse::<i32>() {
                    match waitpid(Pid::from_raw(pid), None) {
                        Ok(WaitStatus::Exited(_, code)) => {
                            shell.last_status = code;
                        }
                        Ok(WaitStatus::Signaled(_, sig, _)) => {
                            shell.last_status = 128 + sig as i32;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    shell.last_status
}

pub(super) fn builtin_kill(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        if args.is_empty() {
            eprintln!(
                "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            );
            return 2;
        }

        // Handle kill -l [signum]
        if args.first().map(|s| s.as_str()) == Some("-l")
            || args.first().map(|s| s.as_str()) == Some("-L")
        {
            if args.len() > 1 {
                let sig_names: &[(&str, i32)] = &[
                    ("HUP", 1),
                    ("INT", 2),
                    ("QUIT", 3),
                    ("ILL", 4),
                    ("TRAP", 5),
                    ("ABRT", 6),
                    ("BUS", 7),
                    ("FPE", 8),
                    ("KILL", 9),
                    ("USR1", 10),
                    ("SEGV", 11),
                    ("USR2", 12),
                    ("PIPE", 13),
                    ("ALRM", 14),
                    ("TERM", 15),
                    ("STKFLT", 16),
                    ("CHLD", 17),
                    ("CONT", 18),
                    ("STOP", 19),
                    ("TSTP", 20),
                    ("TTIN", 21),
                    ("TTOU", 22),
                    ("URG", 23),
                    ("XCPU", 24),
                    ("XFSZ", 25),
                    ("VTALRM", 26),
                    ("PROF", 27),
                    ("WINCH", 28),
                    ("IO", 29),
                    ("PWR", 30),
                    ("SYS", 31),
                ];
                for arg in &args[1..] {
                    if let Ok(num) = arg.parse::<i32>() {
                        // kill -l <signum> — print signal name
                        let num = if num > 128 { num - 128 } else { num };
                        if let Some((name, _)) = sig_names.iter().find(|(_, n)| *n == num) {
                            println!("{}", name);
                        } else {
                            eprintln!(
                                "{}: kill: {}: invalid signal specification",
                                shell.error_prefix(),
                                arg
                            );
                            return 1;
                        }
                    } else {
                        // kill -l <name> — print signal number
                        let upper = arg.to_uppercase();
                        let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
                        if let Some((_, num)) = sig_names.iter().find(|(n, _)| *n == upper) {
                            println!("{}", num);
                        } else {
                            eprintln!(
                                "{}: kill: {}: invalid signal specification",
                                shell.error_prefix(),
                                arg
                            );
                            return 1;
                        }
                    }
                }
            } else {
                // kill -l — list all signals
                // Use same signal list as trap -l
                let signals = list_all_signals();
                for (i, (num, name)) in signals.iter().enumerate() {
                    print!("{:2}) {}", num, name);
                    if (i + 1) % 5 == 0 || i == signals.len() - 1 {
                        println!();
                    } else {
                        print!("\t");
                    }
                }
            }
            return 0;
        }

        let mut signal = Signal::SIGTERM;
        let mut pids = Vec::new();

        let parse_signal = |name: &str| -> Option<Signal> {
            let upper = name.to_uppercase();
            let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
            match upper {
                "HUP" => Some(Signal::SIGHUP),
                "INT" => Some(Signal::SIGINT),
                "QUIT" => Some(Signal::SIGQUIT),
                "ILL" => Some(Signal::SIGILL),
                "TRAP" => Some(Signal::SIGTRAP),
                "ABRT" => Some(Signal::SIGABRT),
                "BUS" => Some(Signal::SIGBUS),
                "FPE" => Some(Signal::SIGFPE),
                "KILL" => Some(Signal::SIGKILL),
                "USR1" => Some(Signal::SIGUSR1),
                "SEGV" => Some(Signal::SIGSEGV),
                "USR2" => Some(Signal::SIGUSR2),
                "PIPE" => Some(Signal::SIGPIPE),
                "ALRM" => Some(Signal::SIGALRM),
                "TERM" => Some(Signal::SIGTERM),
                "CHLD" => Some(Signal::SIGCHLD),
                "CONT" => Some(Signal::SIGCONT),
                "STOP" => Some(Signal::SIGSTOP),
                "TSTP" => Some(Signal::SIGTSTP),
                "TTIN" => Some(Signal::SIGTTIN),
                "TTOU" => Some(Signal::SIGTTOU),
                _ => None,
            }
        };

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-s" || arg == "-n" {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "{}: kill: {}: option requires an argument",
                        shell.error_prefix(),
                        arg
                    );
                    return 2;
                }
                let sig_arg = &args[i];
                if let Ok(n) = sig_arg.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else if let Some(sig) = parse_signal(sig_arg) {
                    signal = sig;
                } else {
                    eprintln!(
                        "{}: kill: {}: invalid signal specification",
                        shell.error_prefix(),
                        sig_arg
                    );
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let sig_name = &arg[1..];
                if let Ok(n) = sig_name.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else if let Some(sig) = parse_signal(sig_name) {
                    signal = sig;
                } else {
                    eprintln!(
                        "{}: kill: {}: invalid signal specification",
                        shell.error_prefix(),
                        sig_name
                    );
                    return 1;
                }
            } else if arg.is_empty() {
                eprintln!(
                    "{}: kill: `': not a pid or valid job spec",
                    shell.error_prefix()
                );
                return 1;
            } else if let Ok(pid) = arg.parse::<i32>() {
                pids.push(pid);
            } else {
                eprintln!(
                    "{}: kill: `{}': not a pid or valid job spec",
                    shell.error_prefix(),
                    arg
                );
                return 1;
            }
            i += 1;
        }

        if pids.is_empty() {
            eprintln!(
                "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            );
            return 2;
        }
        let mut status = 0;
        for pid in pids {
            if signal::kill(Pid::from_raw(pid), signal).is_err() {
                eprintln!(
                    "{}: kill: ({}) - No such process",
                    shell.error_prefix(),
                    pid
                );
                status = 1;
            }
        }
        status
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!(
            "{}: kill: not supported on this platform",
            shell.error_prefix()
        );
        1
    }
}

pub(super) fn builtin_enable(shell: &mut Shell, args: &[String]) -> i32 {
    let builtin_map = builtins();

    // POSIX special builtins
    let special_builtins: std::collections::HashSet<&str> = [
        ".", ":", "break", "continue", "eval", "exec", "exit", "export", "readonly", "return",
        "set", "shift", "source", "times", "trap", "unset",
    ]
    .iter()
    .copied()
    .collect();

    let mut flag_n = false; // disable
    let mut _flag_p = false; // print
    let mut flag_s = false; // special builtins only
    let mut flag_a = false; // all (include disabled)
    let mut flag_d = false; // delete dynamically loaded
    let mut names: Vec<String> = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'n' => flag_n = true,
                    'p' => _flag_p = true,
                    's' => flag_s = true,
                    'a' => flag_a = true,
                    'd' => flag_d = true,
                    _ => {
                        eprintln!("{}: enable: -{}: invalid option", shell.error_prefix(), ch);
                        return 2;
                    }
                }
            }
        } else {
            names.push(arg.clone());
        }
    }

    // enable -d NAME: attempt to unload dynamically loaded builtin (we don't support dynamic loading)
    if flag_d {
        for name in &names {
            if builtin_map.contains_key(name.as_str()) {
                eprintln!(
                    "{}: enable: {}: not dynamically loaded",
                    shell.error_prefix(),
                    name
                );
            } else {
                eprintln!(
                    "{}: enable: {}: not a shell builtin",
                    shell.error_prefix(),
                    name
                );
            }
        }
        return if names.is_empty() { 0 } else { 1 };
    }

    // If no names given, list builtins
    if names.is_empty() {
        let mut all_names: Vec<&&str> = builtin_map.keys().collect();
        all_names.sort();

        if flag_n && !flag_a {
            // enable -n: list disabled builtins
            let mut disabled: Vec<&String> = shell.disabled_builtins.iter().collect();
            disabled.sort();
            for name in disabled {
                if flag_s && !special_builtins.contains(name.as_str()) {
                    continue;
                }
                println!("enable -n {}", name);
            }
        } else {
            // List enabled (and optionally disabled) builtins
            for name in &all_names {
                let is_disabled = shell.disabled_builtins.contains(**name);
                if flag_s && !special_builtins.contains(**name) {
                    continue;
                }
                if flag_a {
                    // Show all
                    if is_disabled {
                        println!("enable -n {}", name);
                    } else {
                        println!("enable {}", name);
                    }
                } else if !is_disabled {
                    // Only show enabled
                    println!("enable {}", name);
                }
            }
        }
        return 0;
    }

    // Process named builtins
    let mut status = 0;
    for name in &names {
        if !builtin_map.contains_key(name.as_str()) {
            eprintln!(
                "{}: enable: {}: not a shell builtin",
                shell.error_prefix(),
                name
            );
            status = 1;
            continue;
        }
        if flag_n {
            // Disable the builtin
            shell.disabled_builtins.insert(name.clone());
        } else {
            // Re-enable the builtin
            shell.disabled_builtins.remove(name.as_str());
        }
    }
    status
}

pub(super) fn builtin_suspend(_shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("bash: suspend: cannot suspend");
    1
}

pub(super) fn builtin_times(_shell: &mut Shell, _args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // Get resource usage for this process
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        println!(
            "{}m{}.{:03}s {}m{}.{:03}s",
            usage.ru_utime.tv_sec / 60,
            usage.ru_utime.tv_sec % 60,
            usage.ru_utime.tv_usec / 1000,
            usage.ru_stime.tv_sec / 60,
            usage.ru_stime.tv_sec % 60,
            usage.ru_stime.tv_usec / 1000,
        );
        // Get resource usage for children
        let mut child_usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut child_usage) };
        println!(
            "{}m{}.{:03}s {}m{}.{:03}s",
            child_usage.ru_utime.tv_sec / 60,
            child_usage.ru_utime.tv_sec % 60,
            child_usage.ru_utime.tv_usec / 1000,
            child_usage.ru_stime.tv_sec / 60,
            child_usage.ru_stime.tv_sec % 60,
            child_usage.ru_stime.tv_usec / 1000,
        );
        0
    }
    #[cfg(not(unix))]
    {
        println!("0m0.000s 0m0.000s");
        println!("0m0.000s 0m0.000s");
        0
    }
}

pub(super) fn builtin_ulimit(_shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        // Handle basic -n (open files) case
        let mut resource = libc::RLIMIT_FSIZE;
        let mut set_value: Option<u64> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-n" => resource = libc::RLIMIT_NOFILE,
                "-c" => resource = libc::RLIMIT_CORE,
                "-d" => resource = libc::RLIMIT_DATA,
                "-f" => resource = libc::RLIMIT_FSIZE,
                "-l" => resource = libc::RLIMIT_MEMLOCK,
                "-m" => resource = libc::RLIMIT_RSS,
                "-s" => resource = libc::RLIMIT_STACK,
                "-t" => resource = libc::RLIMIT_CPU,
                "-v" => resource = libc::RLIMIT_AS,
                "-S" | "-H" => {} // soft/hard limit flags
                "unlimited" => set_value = Some(libc::RLIM_INFINITY),
                val => {
                    if let Ok(n) = val.parse::<u64>() {
                        set_value = Some(n);
                    }
                }
            }
            i += 1;
        }

        if let Some(val) = set_value {
            let rlim = libc::rlimit {
                rlim_cur: val,
                rlim_max: val,
            };
            unsafe { libc::setrlimit(resource, &rlim) };
        } else {
            let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
            unsafe { libc::getrlimit(resource, &mut rlim) };
            if rlim.rlim_cur == libc::RLIM_INFINITY {
                println!("unlimited");
            } else {
                println!("{}", rlim.rlim_cur);
            }
        }
    }
    0
}
